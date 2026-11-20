// SPDX-License-Identifier: MPL-2.0

//! Reading a map's verbs (`format/verbs.md`; design §11.1, #252 §1),
//! after its edits and its presentations, against which they resolve:
//!
//! - **a step is one of five kinds**, with exactly one of the keys
//!   `present`, `stop`, `edit`, `wait` and `verb`, and `on` beside the
//!   first three and only there — the sandbox (§11.3): no other step
//!   kind has a spelling;
//! - **every name resolves in the map itself** (amendment 2 on #252,
//!   §11.3): a `present` and a `stop` name one of the map's
//!   presentations, an `edit` one of the map's edits, a `verb` one of
//!   the map's verbs. A node a target names by id is **not** checked: one
//!   the map lacks is skipped when the step runs (D27), and a deleted node
//!   a verb still names leaves the map loadable;
//! - **the numbers are in bounds**: a node id is one a node can carry, a
//!   channel one a node can be on, and a `wait` no longer than
//!   [`MAX_ANIMATE_MS`](crate::document::bounds::MAX_ANIMATE_MS), the
//!   longest an edit or a presentation animates;
//! - **the call graph is acyclic, and no deeper than
//!   [`MAX_VERB_DEPTH`]** verbs, so that running a verb — its calls run
//!   inline — ends;
//! - **a verb has at most
//!   [`MAX_DEFINITION_PARTS`]
//!   parts, counted after the expansion of the verbs it runs** — its
//!   steps and the node ids its targets name, and those of every verb it
//!   runs, as often as it runs them (owner, format Q4: parts are counted
//!   after any expansion the format allows) — so that a run's work is
//!   bounded whatever the calls fan out to.
//!
//! The checks run in that order, the steps of every verb in file order
//! first, so a map with several faults is refused at the first one a
//! reader of the file meets.

use crate::document::bounds::{self, check_animate_ms, BoundError, MAX_DEFINITION_PARTS};
use crate::document::edit::EditDef;
use crate::document::format::error::{LoadErrorKind, Seg};
use crate::document::format::verbs::{StepDto, StepTargetDto, VerbDto, WithinDto};
use crate::document::ids::{Definitions, NodeIdent};
use crate::document::keyed::NameIndex;
use crate::document::presentation::PresentationDef;
use crate::document::values::{EditId, PresentationId, VerbId};
use crate::document::verb::{On, Step, VerbDef, Within, MAX_VERB_DEPTH};

/// A refusal: the path below `verbs`, and what is wrong there.
pub(crate) type Refusal = (Vec<Seg>, LoadErrorKind);

/// The map's own names a step resolves against, each list indexed once:
/// a step's lookup is one probe of a [`NameIndex`], never a scan of a
/// list a file chose the length of (D29).
struct Names<'a> {
    /// Every verb's id, to the `VerbId` it has: the builder's own index.
    verbs: &'a NameIndex<VerbId>,
    /// Every presentation's id, to its place in the list.
    presentations: NameIndex<PresentationId>,
    /// Every edit's id, to its place in the list.
    edits: NameIndex<EditId>,
}

impl Names<'_> {
    fn verb(&self, id: &str) -> Option<VerbId> {
        self.verbs.get(id)
    }

    fn presentation(&self, id: &str) -> Option<PresentationId> {
        self.presentations.get(id)
    }

    fn edit(&self, id: &str) -> Option<EditId> {
        self.edits.get(id)
    }
}

/// The map's verbs, read from `dtos` against the map's own
/// `presentations` and `edits`; `verbs` is every verb's id to the
/// `VerbId` it has, as the loader numbered them in the file's order. See
/// the [module docs](self) for what is refused.
///
/// Cost: O(the presentations and the edits, each indexed once), O(1) for
/// each step's name, O(the ids the steps name), and the call graph's
/// edges, each looked at once.
pub(crate) fn definitions(
    dtos: &[VerbDto],
    verbs: &NameIndex<VerbId>,
    presentations: &[PresentationDef],
    edits: &[EditDef],
) -> Result<Vec<VerbDef>, Refusal> {
    let mut by_id: NameIndex<PresentationId> = NameIndex::default();
    by_id.reserve(presentations.len());
    for (index, def) in presentations.iter().enumerate() {
        if let Ok(index) = u16::try_from(index) {
            by_id.insert_first(&def.id, PresentationId(index));
        }
    }
    let mut edit_ids: NameIndex<EditId> = NameIndex::default();
    edit_ids.reserve(edits.len());
    for (index, def) in edits.iter().enumerate() {
        if let Ok(index) = u16::try_from(index) {
            edit_ids.insert_first(&def.id, EditId(index));
        }
    }
    let names = Names {
        verbs,
        presentations: by_id,
        edits: edit_ids,
    };
    let mut defs = Vec::with_capacity(dtos.len());
    for (i, dto) in dtos.iter().enumerate() {
        let mut steps = Vec::with_capacity(dto.steps.len());
        for (j, step) in dto.steps.iter().enumerate() {
            let at = |mut segs: Vec<Seg>| {
                let mut path = vec![Seg::Index(i), Seg::Key("steps"), Seg::Index(j)];
                path.append(&mut segs);
                path
            };
            steps.push(self::step(step, &names).map_err(|(segs, kind)| (at(segs), kind))?);
        }
        defs.push(VerbDef {
            id: dto.id.clone(),
            name: dto.name.clone(),
            description: dto.description.clone(),
            steps,
        });
    }
    check_graph(&defs)?;
    Ok(defs)
}

/// One step, resolved; or the path inside it, and what is wrong there.
fn step(dto: &StepDto, names: &Names) -> Result<Step, Refusal> {
    let StepDto {
        present,
        stop,
        edit,
        wait,
        verb,
        on,
    } = dto;
    let kinds = [
        present.is_some(),
        stop.is_some(),
        edit.is_some(),
        wait.is_some(),
        verb.is_some(),
    ]
    .into_iter()
    .filter(|&set| set)
    .count();
    if kinds != 1 {
        return Err((
            Vec::new(),
            LoadErrorKind::InvalidVerb {
                message: format!(
                    "a step has exactly one of `present`, `stop`, `edit`, `wait` and `verb`, and this one has {}",
                    if kinds == 0 {
                        "none".to_string()
                    } else {
                        kinds.to_string()
                    }
                ),
            },
        ));
    }
    let targets = |key: &'static str| -> Result<On, Refusal> {
        match on {
            Some(on) => self::on(on).map_err(|(mut segs, kind)| {
                segs.insert(0, Seg::Key("on"));
                (segs, kind)
            }),
            None => Err((
                Vec::new(),
                LoadErrorKind::InvalidVerb {
                    message: format!("a `{key}` step names its targets in `on`"),
                },
            )),
        }
    };
    let untargeted = |key: &'static str| -> Result<(), Refusal> {
        match on {
            Some(_) => Err((
                vec![Seg::Key("on")],
                LoadErrorKind::KeyNotAllowed {
                    key: "on",
                    reason: if key == "wait" {
                        "a `wait` step acts on nothing"
                    } else {
                        "a `verb` step runs the verb on the node this one was invoked on"
                    },
                },
            )),
            None => Ok(()),
        }
    };
    let presentation = |key: &'static str, id: &str| -> Result<PresentationId, Refusal> {
        names.presentation(id).ok_or_else(|| {
            (
                vec![Seg::Key(key)],
                LoadErrorKind::UnknownPresentation { id: id.to_string() },
            )
        })
    };
    if let Some(id) = present {
        let presentation = presentation("present", id)?;
        return Ok(Step::Present {
            presentation,
            on: targets("present")?,
        });
    }
    if let Some(id) = stop {
        let presentation = presentation("stop", id)?;
        return Ok(Step::Stop {
            presentation,
            on: targets("stop")?,
        });
    }
    if let Some(id) = edit {
        let Some(edit) = names.edit(id) else {
            return Err((
                vec![Seg::Key("edit")],
                LoadErrorKind::UnknownEdit { id: id.clone() },
            ));
        };
        return Ok(Step::Edit {
            edit,
            on: targets("edit")?,
        });
    }
    if let Some(ms) = wait {
        untargeted("wait")?;
        let ms = check_animate_ms(*ms)
            .map_err(|error| (vec![Seg::Key("wait")], LoadErrorKind::Bound { error }))?;
        return Ok(Step::Wait { ms });
    }
    if let Some(id) = verb {
        untargeted("verb")?;
        let verb = names.verb(id).ok_or_else(|| {
            (
                vec![Seg::Key("verb")],
                LoadErrorKind::UnknownVerb { id: id.clone() },
            )
        })?;
        return Ok(Step::Verb { verb });
    }
    // `kinds == 1` above: one of the five was set.
    Err((
        Vec::new(),
        LoadErrorKind::Internal {
            message: "a step with one kind matched none".to_string(),
        },
    ))
}

/// A target, its numbers held to their bounds.
fn on(dto: &StepTargetDto) -> Result<On, Refusal> {
    Ok(match dto {
        StepTargetDto::SelfNode => On::SelfNode,
        StepTargetDto::Selection => On::Selection,
        StepTargetDto::Children => On::Children,
        StepTargetDto::Descendants => On::Descendants,
        StepTargetDto::Nodes(ids) => On::Nodes(
            ids.iter()
                .enumerate()
                .map(|(k, &id)| {
                    bounds::check_node_ident(id).map(NodeIdent).map_err(|error| {
                        (
                            vec![Seg::Key("nodes"), Seg::Index(k)],
                            LoadErrorKind::Bound { error },
                        )
                    })
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
        StepTargetDto::Channel { channel, within } => On::Channel {
            channel: usize::try_from(*channel)
                .map_err(|_| BoundError::NotAChannel(*channel))
                .and_then(|value| bounds::check_channel(value).map(|()| value))
                .map_err(|error| (vec![Seg::Key("channel")], LoadErrorKind::Bound { error }))?,
            within: match within {
                WithinDto::SelfNode => Within::SelfNode,
                WithinDto::Map => Within::Map,
            },
        },
    })
}

/// Where a verb stands in the walk of the call graph.
#[derive(Copy, Clone)]
enum Mark {
    /// Not reached yet.
    Unseen,
    /// On the walk's stack: reaching it again is a cycle.
    Open,
    /// Done: how many verbs deep its calls go (itself counted), and its
    /// parts after expansion.
    Done { depth: usize, parts: usize },
}

/// Refuse a cycle in the call graph, a chain of calls more than
/// [`MAX_VERB_DEPTH`] verbs long, and a verb of more than
/// [`MAX_DEFINITION_PARTS`] parts once the verbs it runs are expanded.
/// The graph is walked from each verb in file order, iteratively, each
/// verb finished once.
fn check_graph(defs: &[VerbDef]) -> Result<(), Refusal> {
    let mut marks = vec![Mark::Unseen; defs.len()];
    for root in 0..defs.len() {
        if !matches!(marks[root], Mark::Unseen) {
            continue;
        }
        // The walk: each verb on it, and the next of its steps to look at.
        let mut stack: Vec<(usize, usize)> = vec![(root, 0)];
        marks[root] = Mark::Open;
        while let Some(&mut (verb, ref mut next)) = stack.last_mut() {
            let steps = &defs[verb].steps;
            let callee = steps[*next..]
                .iter()
                .enumerate()
                .find_map(|(k, step)| match step {
                    Step::Verb { verb } => Some((*next + k, usize::from(verb.0))),
                    _ => None,
                });
            let Some((at, callee)) = callee else {
                // Every step looked at: the verb is done.
                let mut depth = 1usize;
                let mut parts = 0usize;
                for step in steps {
                    parts = parts.saturating_add(own_parts(step));
                    if let Step::Verb { verb } = step {
                        if let Some(Mark::Done {
                            depth: below,
                            parts: theirs,
                        }) = marks.get(usize::from(verb.0))
                        {
                            depth = depth.max(below + 1);
                            parts = parts.saturating_add(*theirs);
                        }
                    }
                }
                if depth > MAX_VERB_DEPTH {
                    return Err((
                        vec![Seg::Index(verb)],
                        LoadErrorKind::VerbTooDeep { max: MAX_VERB_DEPTH },
                    ));
                }
                if parts > MAX_DEFINITION_PARTS {
                    return Err((
                        vec![Seg::Index(verb)],
                        LoadErrorKind::DefinitionTooLarge {
                            max: MAX_DEFINITION_PARTS,
                        },
                    ));
                }
                marks[verb] = Mark::Done { depth, parts };
                stack.pop();
                continue;
            };
            *next = at + 1;
            match marks.get(callee).copied() {
                Some(Mark::Unseen) => {
                    marks[callee] = Mark::Open;
                    stack.push((callee, 0));
                }
                Some(Mark::Open) => {
                    let from = stack.iter().position(|&(v, _)| v == callee).unwrap_or(0);
                    let mut cycle: Vec<String> =
                        stack[from..].iter().map(|&(v, _)| defs[v].id.clone()).collect();
                    cycle.push(defs[callee].id.clone());
                    return Err((
                        vec![
                            Seg::Index(verb),
                            Seg::Key("steps"),
                            Seg::Index(at),
                            Seg::Key("verb"),
                        ],
                        LoadErrorKind::VerbCycle { cycle },
                    ));
                }
                Some(Mark::Done { .. }) | None => {}
            }
        }
    }
    Ok(())
}

/// The parts a step is on its own: one, and one more for each node id
/// its target names.
fn own_parts(step: &Step) -> usize {
    match step {
        Step::Present { on, .. } | Step::Stop { on, .. } | Step::Edit { on, .. } => match on {
            On::Nodes(ids) => ids.len().saturating_add(1),
            _ => 1,
        },
        Step::Wait { .. } | Step::Verb { .. } => 1,
    }
}

/// `def` as a file writes it: what the saver writes back, every name
/// spelled from `definitions`. `None` for a step that names an index the
/// definitions do not have, which a load never makes.
pub(crate) fn to_dto(def: &VerbDef, definitions: &Definitions) -> Option<VerbDto> {
    let presentation = |id: PresentationId| {
        definitions
            .presentations
            .get(usize::from(id.0))
            .map(|def| def.id.clone())
    };
    let steps = def
        .steps
        .iter()
        .map(|step| {
            let mut dto = StepDto {
                present: None,
                stop: None,
                edit: None,
                wait: None,
                verb: None,
                on: None,
            };
            match step {
                Step::Present { presentation: id, on } => {
                    dto.present = Some(presentation(*id)?);
                    dto.on = Some(on_dto(on));
                }
                Step::Stop { presentation: id, on } => {
                    dto.stop = Some(presentation(*id)?);
                    dto.on = Some(on_dto(on));
                }
                Step::Edit { edit, on } => {
                    dto.edit = Some(definitions.edits.get(usize::from(edit.0))?.id.clone());
                    dto.on = Some(on_dto(on));
                }
                Step::Wait { ms } => dto.wait = Some(*ms),
                Step::Verb { verb } => {
                    dto.verb = Some(definitions.verbs.get(usize::from(verb.0))?.id.clone());
                }
            }
            Some(dto)
        })
        .collect::<Option<Vec<_>>>()?;
    Some(VerbDto {
        id: def.id.clone(),
        name: def.name.clone(),
        description: def.description.clone(),
        steps,
    })
}

/// A target as a file writes it.
fn on_dto(on: &On) -> StepTargetDto {
    match on {
        On::SelfNode => StepTargetDto::SelfNode,
        On::Selection => StepTargetDto::Selection,
        On::Children => StepTargetDto::Children,
        On::Descendants => StepTargetDto::Descendants,
        On::Nodes(idents) => StepTargetDto::Nodes(idents.iter().map(|ident| u64::from(ident.0)).collect()),
        On::Channel { channel, within } => StepTargetDto::Channel {
            channel: *channel as u64,
            within: match within {
                Within::SelfNode => WithinDto::SelfNode,
                Within::Map => WithinDto::Map,
            },
        },
    }
}
