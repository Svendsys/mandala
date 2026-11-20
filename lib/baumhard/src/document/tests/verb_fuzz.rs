// SPDX-License-Identifier: MPL-2.0

//! The verb fuzz properties (design D29, #252 comment 6428789421;
//! TEST_CONVENTIONS §T14): a map's verbs, triggers and links are the
//! most security-relevant untrusted-input surface a map has, so any
//! bytes read as a map's `verbs`, and verbs built from choices, hold to:
//!
//! - **no panic**, in the reader, an invocation, a frame at any clock
//!   reading, or a release;
//! - **the sandbox**: a step is one of the five kinds or the map does not
//!   load — a step with any other key, or with none of the five, is
//!   refused, at its path under `verbs[i]`;
//! - **the load decides as the format says**: a map whose verbs a
//!   reference model accepts — every name the map's own, every wait
//!   within a minute, every node id one a node can carry, the calls
//!   acyclic and at most [`MAX_VERB_DEPTH`](crate::document::verb::MAX_VERB_DEPTH) verbs deep, and at most
//!   [`MAX_DEFINITION_PARTS`](crate::document::bounds::MAX_DEFINITION_PARTS) parts once the calls are expanded — loads,
//!   and one it refuses does not; the model shares no code with the
//!   loader;
//! - **bounded recursion and time**: an invocation runs at most as many
//!   steps as its verb has parts after expansion, however its calls fan
//!   out, and every run is over once the waits it can make have passed —
//!   [`MAX_VERB_DEPTH`](crate::document::verb::MAX_VERB_DEPTH) × the parts × a minute at the very most, which
//!   one frame far enough on reaches;
//! - **a missing node is skipped, not a crash (D27)**: a target naming a
//!   node the map lacks reaches the performer as no node, while the
//!   targets the map has are acted on;
//! - **links open only `http`, `https` and `mailto`, and no `mailto`
//!   that attaches a file** (ruling D3.2 item 2): a span link of any
//!   other scheme, or a `mailto` with an `attach` or `attachment`
//!   parameter, is refused at load at its `url`, and a `set_link` to
//!   one is refused at apply, the map left as it was;
//! - **a loaded map saves and reloads to the same bytes**, its verbs
//!   with it.
//!
//! The fuzz targets (`fuzz/fuzz_targets/verbs_*.rs`) are thin wrappers
//! over [`check_verbs_json`](crate::document::tests::verb_fuzz::check_verbs_json) and [`check_verbs_structured`](crate::document::tests::verb_fuzz::check_verbs_structured); the sweep in
//! `verb_fuzz_tests` runs the generator on stable, in `./test.sh`.

use crate::arena::NodeId;
use crate::document::bounds::{MAX_ANIMATE_MS, MAX_DEFINITION_PARTS, MAX_NODE_IDENT};
use crate::document::element::{DocElement, DocKind};
use crate::document::format::{load, LoadErrorKind};
use crate::document::ids::{Document, NodeIdent};
use crate::document::mutation::compose;
use crate::document::mutation::ops::{DocMutation, SpanOp};
use crate::document::presentation::{InstanceId, PresentError};
use crate::document::tests::edit_fuzz::Boxes;
use crate::document::values::{EditId, LinkTarget, PresentationId};
use crate::document::verb::{Performer, Runs, MAX_VERB_DEPTH};
use crate::document::writer::{ChangeSource, EditRequest};
use crate::edit_handlers::registry;
use crate::util::tests::choices::Choices;

/// The map verbs are read into and run on: five nodes — 1 (children 2
/// and 3), 2 (child 4), and 5 — two presentations, `p` (`while`) and
/// `h` (`hold`), and one edit, `e`. Its `verbs` value is where an
/// input goes.
pub const MAP_BEFORE_VERBS: &str = r##"{"format":1,"name":"verbs",
"edits":[{"id":"e","name":"E","reach":"self","target":"node","mutations":[{"node":{"nudge":[10.0,0.0]}}]}],
"presentations":[
 {"id":"p","name":"P","play":"while","target":"node","mutations":[{"node":{"set_opacity":0.5}}],"animate":{"ms":100}},
 {"id":"h","name":"H","play":"hold","target":"node","mutations":[{"node":{"set_hidden":true}}],"return":{"ms":50}}
],
"verbs":"##;

/// What follows the `verbs` value: the nodes.
pub const MAP_AFTER_VERBS: &str = r#",
"nodes":[
 {"id":1,"position":[0,0],"sections":[{"spans":[{"text":"one"}]}]},
 {"id":2,"parent":1,"position":[300,0],"sections":[{"spans":[{"text":"two"}]}]},
 {"id":4,"parent":2,"channel":1,"position":[600,0],"sections":[{"spans":[{"text":"four"}]}]},
 {"id":3,"parent":1,"channel":1,"position":[300,200],"sections":[{"spans":[{"text":"three"}]}]},
 {"id":5,"channel":1,"position":[900,0],"sections":[{"spans":[{"text":"five"}]}]}
]}"#;

/// The map with `verbs` as its `verbs` value.
pub fn map_with_verbs(verbs: &str) -> String {
    format!("{MAP_BEFORE_VERBS}{verbs}{MAP_AFTER_VERBS}")
}

/// A [`Performer`] over a bare document: what a run does, done on the
/// document directly — an edit through `EditDef::request` and
/// `Document::apply`, instantly — and every call counted, for the
/// properties to hold the run to.
pub struct OnTheDocument<'d> {
    /// The document.
    pub(in crate::document) doc: &'d mut Document,
    /// What is selected.
    pub(in crate::document) selection: Vec<NodeId>,
    /// Every call, as `(what, the presentation's or the edit's id, the
    /// nodes, the time)`.
    pub(in crate::document) calls: Vec<(&'static str, String, Vec<NodeId>, f64)>,
}

impl<'d> OnTheDocument<'d> {
    /// A performer over `doc`, nothing selected.
    pub fn new(doc: &'d mut Document) -> Self {
        OnTheDocument {
            doc,
            selection: Vec::new(),
            calls: Vec::new(),
        }
    }
}

impl OnTheDocument<'_> {
    /// The id of the map's presentation `id`, for the record of a call.
    fn presentation(&self, id: PresentationId) -> String {
        self.doc
            .definitions()
            .presentations
            .get(usize::from(id.0))
            .map_or_else(String::new, |def| def.id.clone())
    }
}

impl Performer for OnTheDocument<'_> {
    fn document(&self) -> &Document {
        self.doc
    }

    fn selection(&self) -> Vec<NodeId> {
        self.selection.clone()
    }

    fn start(
        &mut self,
        id: PresentationId,
        nodes: &[NodeId],
        at_ms: f64,
    ) -> Result<Vec<InstanceId>, PresentError> {
        self.calls
            .push(("start", self.presentation(id), nodes.to_vec(), at_ms));
        self.doc
            .start_defined(id, nodes, at_ms)
            .unwrap_or_else(|| panic!("a run starts only a presentation the map defines, not {id:?}"))
    }

    fn stop(&mut self, id: PresentationId, nodes: &[NodeId], at_ms: f64) -> usize {
        self.calls
            .push(("stop", self.presentation(id), nodes.to_vec(), at_ms));
        self.doc.stop_defined(id, nodes, at_ms)
    }

    fn stop_instances(&mut self, ids: &[InstanceId], at_ms: f64) -> usize {
        self.calls.push(("release", String::new(), Vec::new(), at_ms));
        self.doc.stop_instances(ids, at_ms)
    }

    fn edit(&mut self, id: EditId, nodes: &[NodeId], at_ms: f64) -> Result<(), String> {
        let def = self
            .doc
            .definitions()
            .edits
            .get(usize::from(id.0))
            .cloned()
            .ok_or_else(|| format!("no edit at {id:?}"))?;
        self.calls.push(("edit", def.id.clone(), nodes.to_vec(), at_ms));
        let request = def
            .request(self.doc, &Boxes::of(self.doc), &registry(), nodes)
            .map_err(|error| error.to_string())?;
        self.doc
            .apply(request, ChangeSource::Verb)
            .map(drop)
            .map_err(|error| error.to_string())
    }
}

/// Every property of `bytes` read as a map's `verbs` value.
pub fn check_verbs_json(bytes: &[u8]) -> Result<(), String> {
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(());
    };
    check_verbs_text(text).map(|_| ())
}

/// What one input did, for the sweeps that hold the properties to
/// having been exercised.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Ran {
    /// Whether the map loaded.
    pub loaded: bool,
    /// The verbs invoked.
    pub invoked: usize,
    /// The steps that reached the performer.
    pub calls: usize,
    /// The waits a run made.
    pub waited: usize,
}

/// [`check_verbs_json`] for a text, with what it ran.
///
/// A text that is not one JSON value — trailing bytes after a list, a
/// list left open — is no `verbs` value: in the map it breaks the text
/// around it, so a refusal there may name the whole text. It is held to
/// not panicking alone (`verb_fuzz_tests::test_hostile_verbs_hold_every_property`).
pub fn check_verbs_text(verbs: &str) -> Result<Ran, String> {
    let map = map_with_verbs(verbs);
    let one_value = serde_json::from_str::<serde::de::IgnoredAny>(verbs).is_ok();
    let loaded = match load(&map) {
        Ok(doc) => doc,
        Err(error) => {
            if one_value && !error.path.starts_with("verbs") {
                return Err(format!(
                    "a refusal of the verbs is at `{}`, outside `verbs`: {error}",
                    error.path
                ));
            }
            return Ok(Ran::default());
        }
    };
    run_every_verb(loaded)
}

/// Save and reload `doc`, then invoke each of its verbs on each node and
/// on none, frame it through every wait, and release the held runs:
/// every property a loaded map is held to.
fn run_every_verb(mut doc: Document) -> Result<Ran, String> {
    let saved = doc
        .save_json()
        .map_err(|error| format!("a loaded map saves: {error}"))?;
    let mut again = load(&saved).map_err(|error| format!("a saved map loads: {error}\n{saved}"))?;
    let resaved = again
        .save_json()
        .map_err(|error| format!("a reloaded map saves: {error}"))?;
    if resaved != saved {
        return Err(format!(
            "a map saves to other bytes once reloaded:\n{saved}\n{resaved}"
        ));
    }
    let mut ran = Ran {
        loaded: true,
        ..Ran::default()
    };
    let nodes: Vec<NodeId> = doc
        .tree()
        .descendants()
        .filter(|&id| doc.get(id).is_some_and(|e| e.kind() == DocKind::Node))
        .collect();
    let model = Model::of(&doc);
    let verbs = doc.definitions().verbs.len();
    for verb in 0..verbs {
        let id = crate::document::values::VerbId(u16::try_from(verb).map_err(|_| "too many verbs")?);
        let bound = model.expanded(verb);
        for on in std::iter::once(None).chain(nodes.iter().take(2).copied().map(Some)) {
            let mut runs = Runs::default();
            let mut performer = OnTheDocument::new(&mut doc);
            performer.selection = nodes.iter().take(1).copied().collect();
            let held = on.is_some();
            let invoked = runs.invoke(id, on, held, 0.0, &mut performer);
            if invoked.is_err() {
                continue;
            }
            ran.invoked += 1;
            // Every wait a run can make, and a frame past them all.
            let mut now = 0.0;
            while let Some(due) = runs.next_due_ms() {
                if due < now || !due.is_finite() {
                    return Err(format!("a run resumes at {due}, before the frame that runs it"));
                }
                now = due;
                runs.tick(now, &mut performer);
                ran.waited += 1;
                if ran.waited > 4 * MAX_DEFINITION_PARTS {
                    return Err("a run waits on past what its parts allow".to_string());
                }
            }
            let horizon = (MAX_VERB_DEPTH * MAX_DEFINITION_PARTS) as f64 * f64::from(MAX_ANIMATE_MS) + 1.0;
            runs.tick(horizon, &mut performer);
            if runs.waiting() {
                return Err("a run still waits after every wait it can make".to_string());
            }
            if let Ok(run) = invoked {
                runs.release(run, horizon, &mut performer);
            }
            if runs.runs().next().is_some() {
                return Err("a run outlives its release".to_string());
            }
            let steps = performer
                .calls
                .iter()
                .filter(|(what, ..)| *what != "release")
                .count();
            if steps > bound {
                return Err(format!(
                    "verb {verb} reached the performer {steps} times, past its {bound} parts after expansion"
                ));
            }
            // D27: every node a step reaches the performer with is a
            // node of the map.
            for (what, _, reached, _) in &performer.calls {
                let not_a_node =
                    |id: &&NodeId| performer.doc.get(**id).map(DocElement::kind) != Some(DocKind::Node);
                if let Some(missing) = reached.iter().find(not_a_node) {
                    return Err(format!(
                        "a `{what}` step reached {missing}, which is not a node in the map"
                    ));
                }
            }
            ran.calls += steps;
            doc.tick_presentations(horizon);
        }
    }
    Ok(ran)
}

/// A verb as the reference model reads it: its own parts and the verbs
/// it calls, by index; `None` for a call to a verb the map lacks.
struct ModelVerb {
    parts: usize,
    calls: Vec<Option<usize>>,
}

/// The reference model of a map's verbs, read from the loaded
/// definitions: what the expansion of each verb's calls comes to.
/// Computed by plain recursion with no memory, so that it shares
/// nothing with the loader's walk; used only on maps the loader
/// accepted, whose calls are acyclic and shallow.
struct Model {
    verbs: Vec<ModelVerb>,
}

impl Model {
    /// The model of `doc`'s verbs.
    fn of(doc: &Document) -> Self {
        use crate::document::verb::{On, Step};
        let verbs = doc
            .definitions()
            .verbs
            .iter()
            .map(|verb| ModelVerb {
                parts: verb
                    .steps
                    .iter()
                    .map(|step| match step {
                        Step::Present {
                            on: On::Nodes(ids), ..
                        }
                        | Step::Stop {
                            on: On::Nodes(ids), ..
                        }
                        | Step::Edit {
                            on: On::Nodes(ids), ..
                        } => 1 + ids.len(),
                        _ => 1,
                    })
                    .sum(),
                calls: verb
                    .steps
                    .iter()
                    .filter_map(|step| match step {
                        Step::Verb { verb } => Some(Some(usize::from(verb.0))),
                        _ => None,
                    })
                    .collect(),
            })
            .collect();
        Model { verbs }
    }

    /// `verb`'s parts with its calls expanded, saturating.
    fn expanded(&self, verb: usize) -> usize {
        let Some(model) = self.verbs.get(verb) else {
            return 0;
        };
        model.calls.iter().fold(model.parts, |sum, call| {
            sum.saturating_add(call.map_or(0, |callee| self.expanded(callee)))
        })
    }
}

/// Every property of the verbs `bytes` choose.
pub fn check_verbs_structured(bytes: &[u8]) -> Result<(), String> {
    check_verbs_generated(&mut Choices::new(bytes)).map(|_| ())
}

/// A generated verb: its id and its steps, each as the model sees it and
/// as JSON.
struct Generated {
    id: String,
    steps: Vec<(GenStep, String)>,
}

/// A generated step, as the reference model judges it.
#[derive(Clone, Debug)]
enum GenStep {
    /// A `present`, `stop` or `edit` naming something the map has or
    /// lacks, on a target whose node ids are each in bounds or not.
    Targeted { known: bool, ids_ok: bool, parts: usize },
    /// A `wait` of this many milliseconds.
    Wait(u64),
    /// A `verb` call to the verb with this index, or one the map lacks.
    Call(Option<usize>),
    /// A step that is no step: none of the five keys, two of them, an
    /// unknown key, or `on` where it is not taken.
    Malformed,
}

/// [`check_verbs_structured`] for choices from any source: verbs built
/// to cover every step kind, every target, names the map has and lacks,
/// waits at and past their bound, calls that cycle, chains past the
/// depth and fans past the parts — each held to the reference model's
/// verdict — then the properties of the map they load into; and a span
/// link of a scheme the choices draw, at load and at apply.
pub fn check_verbs_generated(choices: &mut Choices<'_>) -> Result<Ran, String> {
    let generated = generate(choices);
    let json = format!(
        "[{}]",
        generated
            .iter()
            .map(|verb| format!(
                r#"{{"id":"{}","name":"N","steps":[{}]}}"#,
                verb.id,
                verb.steps
                    .iter()
                    .map(|(_, json)| json.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ))
            .collect::<Vec<_>>()
            .join(",")
    );
    let accepts = model_accepts(&generated);
    let map = map_with_verbs(&json);
    let ran = match (load(&map), accepts) {
        (Ok(doc), true) => run_every_verb(doc)?,
        (Err(error), false) if error.path.starts_with("verbs") => Ran::default(),
        (Ok(_), false) => return Err(format!("the map loads, and the model refuses its verbs:\n{json}")),
        (Err(error), _) => {
            return Err(format!(
                "the map is refused ({error}), and the model accepts its verbs:\n{json}"
            ))
        }
    };
    check_link_scheme(choices)?;
    Ok(ran)
}

/// What a load says of a link's address.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Link {
    /// It loads.
    Opens,
    /// Refused for its scheme.
    Scheme,
    /// Refused as a `mailto` that attaches a file (ruling D3.2 item 2).
    Attaches,
}

/// The addresses a generated link draws from, each with what a load
/// says of it: the three schemes a link opens, in two cases, others, and
/// `mailto` addresses with and without a parameter that attaches a file
/// — spelled in any case, percent-encoded, after a `;` — or the word as
/// a value, which attaches nothing.
const SCHEMES: [(&str, Link); 16] = [
    ("https://example.org/a", Link::Opens),
    ("HTTP://example.org", Link::Opens),
    ("mailto:someone@example.org", Link::Opens),
    ("file:///etc/passwd", Link::Scheme),
    ("javascript:alert(1)", Link::Scheme),
    ("data:text/html,x", Link::Scheme),
    ("example.org", Link::Scheme),
    ("https", Link::Scheme),
    (" https://example.org", Link::Scheme),
    ("mailto:a@example.org?subject=attach&body=attachment", Link::Opens),
    ("https://example.org/?attach=x", Link::Opens),
    ("mailto:a@example.org?attach=/etc/passwd", Link::Attaches),
    (
        "MAILTO:a@example.org?subject=x&ATTACHMENT=%2Fetc%2Fpasswd",
        Link::Attaches,
    ),
    ("mailto:a@example.org?%61ttach=x", Link::Attaches),
    ("mailto:a@example.org?body=x;Attach=y", Link::Attaches),
    ("mailto:?%20attachment%20", Link::Attaches),
];

/// A span link of an address `choices` draws is loaded exactly when a
/// link may open it, refused at its `url` otherwise, of the kind the
/// table says; and a `set_link` to it, applied to a map that loaded, is
/// accepted exactly then too, a refusal leaving the map as it was.
fn check_link_scheme(choices: &mut Choices<'_>) -> Result<(), String> {
    let &(url, verdict) = choices.pick(&SCHEMES);
    let allowed = verdict == Link::Opens;
    let escaped = serde_json::to_string(url).map_err(|error| error.to_string())?;
    let map = format!(
        r#"{{"format":1,"name":"l","nodes":[{{"id":1,"position":[0,0],"sections":[{{"spans":[{{"text":"x","link":{{"url":{escaped}}}}}]}}]}}]}}"#
    );
    match (load(&map), allowed) {
        (Ok(_), true) => {}
        (Err(error), false)
            if error.path == "nodes[0].sections[0].spans[0].link.url"
                && match verdict {
                    Link::Scheme => matches!(error.kind, LoadErrorKind::LinkScheme { .. }),
                    Link::Attaches => matches!(error.kind, LoadErrorKind::LinkAttachment { .. }),
                    Link::Opens => false,
                } => {}
        (read, _) => {
            return Err(format!(
                "the link `{url}` read as {read:?}, and it is allowed: {allowed}"
            ))
        }
    }
    let mut doc = load(&map_with_verbs("[]")).map_err(|error| format!("the plain map loads: {error}"))?;
    let span = doc
        .tree()
        .descendants()
        .find(|&id| doc.get(id).is_some_and(|e| e.kind() == DocKind::Span))
        .ok_or("the plain map has a span")?;
    let before = doc.save_json().map_err(|error| error.to_string())?;
    let steps = compose::set(
        &doc,
        DocKind::Span,
        vec![DocMutation::Span(SpanOp::SetLink(Some(LinkTarget::Url(
            url.to_string(),
        ))))],
        &[span],
    )
    .map_err(|error| format!("a set_link composes: {error}"))?;
    let applied = doc.apply(EditRequest::new(steps, "link".to_string()), ChangeSource::Edit);
    match (applied, allowed) {
        (Ok(_), true) => Ok(()),
        (Err(_), false) => {
            let after = doc.save_json().map_err(|error| error.to_string())?;
            if after == before {
                Ok(())
            } else {
                Err(format!("a refused set_link to `{url}` changed the map"))
            }
        }
        (applied, _) => Err(format!(
            "a set_link to `{url}` applied as {applied:?}, and it is allowed: {allowed}"
        )),
    }
}

/// The verbs `choices` build. Their ids are `v0`, `v1`, …; one verb in
/// four is a chain or a fan the choices size around the depth and the
/// parts bounds.
fn generate(choices: &mut Choices<'_>) -> Vec<Generated> {
    let count = 1 + choices.index(6);
    let mut verbs: Vec<Generated> = (0..count)
        .map(|n| Generated {
            id: format!("v{n}"),
            steps: Vec::new(),
        })
        .collect();
    // One input in two has one fault, at a step the choices pick; the
    // rest are well formed, their calls going forward so that they
    // cycle only where the fault makes them.
    let faulty = choices.one_in(2);
    let mut fault_at = if faulty {
        Some(choices.index(count * 6))
    } else {
        None
    };
    let mut at = 0;
    for (n, verb) in verbs.iter_mut().enumerate() {
        let steps = choices.index(6);
        for _ in 0..steps {
            let fault = fault_at.is_some_and(|f| f <= at);
            if fault {
                fault_at = None;
            }
            verb.steps.push(step(choices, n, count, fault));
            at += 1;
        }
    }
    // A chain: v0 → v1 → … around the depth bound, the rest of the list
    // made of links of it.
    if choices.one_in(4) {
        let length = MAX_VERB_DEPTH - 1 + choices.index(3);
        let base = verbs.len();
        for k in 0..length {
            let next = if k + 1 < length {
                format!(r#"{{"verb":"v{}"}}"#, base + k + 1)
            } else {
                r#"{"wait":0}"#.to_string()
            };
            let call = if k + 1 < length {
                GenStep::Call(Some(base + k + 1))
            } else {
                GenStep::Wait(0)
            };
            verbs.push(Generated {
                id: format!("v{}", base + k),
                steps: vec![(call, next)],
            });
        }
    }
    // A fan: each level calls the next one many times, around the parts
    // bound, so the expansion — not the text — is what is large.
    if choices.one_in(4) {
        let width = 15 + choices.index(4);
        let levels = 4;
        let base = verbs.len();
        for k in 0..levels {
            let (step, json) = if k + 1 < levels {
                (
                    GenStep::Call(Some(base + k + 1)),
                    format!(r#"{{"verb":"v{}"}}"#, base + k + 1),
                )
            } else {
                (
                    GenStep::Targeted {
                        known: true,
                        ids_ok: true,
                        parts: 1,
                    },
                    r#"{"stop":"h","on":"selection"}"#.to_string(),
                )
            };
            verbs.push(Generated {
                id: format!("v{}", base + k),
                steps: vec![(step, json); width],
            });
        }
    }
    verbs
}

/// One step `choices` build for the verb `n` of `count`: well formed,
/// its names the map's, its target's ids each one a node can carry (the
/// map's or not, D27), its wait within the bound and its call to a later
/// verb — or, when `fault`, one of the ways a step is refused: a name the
/// map lacks, an id no node can carry, a wait past the bound, a call to
/// a verb the map lacks or back to an earlier one (which may close a
/// cycle), or no step at all.
fn step(choices: &mut Choices<'_>, n: usize, count: usize, fault: bool) -> (GenStep, String) {
    if fault {
        return match choices.below(6) {
            0 => {
                let (kind, name) =
                    *choices.pick(&[("present", "no-such"), ("stop", "lift"), ("edit", "missing")]);
                let (on, _, parts) = target(choices, false);
                (
                    GenStep::Targeted {
                        known: false,
                        ids_ok: true,
                        parts,
                    },
                    format!(r#"{{"{kind}":"{name}","on":{on}}}"#),
                )
            }
            1 => {
                let (on, ids_ok, parts) = target(choices, true);
                (
                    GenStep::Targeted {
                        known: true,
                        ids_ok,
                        parts,
                    },
                    format!(r#"{{"present":"h","on":{on}}}"#),
                )
            }
            2 => {
                let ms = if choices.one_in(2) {
                    u64::from(MAX_ANIMATE_MS) + 1
                } else {
                    u64::from(choices.next_u32())
                };
                (GenStep::Wait(ms), format!(r#"{{"wait":{ms}}}"#))
            }
            3 => (GenStep::Call(None), r#"{"verb":"nobody"}"#.to_string()),
            4 => {
                let callee = choices.index(n + 1);
                (GenStep::Call(Some(callee)), format!(r#"{{"verb":"v{callee}"}}"#))
            }
            _ => {
                let json = *choices.pick(&[
                    r#"{}"#,
                    r#"{"present":"p","stop":"p","on":"self"}"#,
                    r#"{"console":"save"}"#,
                    r#"{"action":"Quit"}"#,
                    r#"{"wait":10,"on":"self"}"#,
                    r#"{"verb":"v0","on":"self"}"#,
                    r#"{"present":"p"}"#,
                    r#"{"present":"p","on":"everything"}"#,
                    r#"{"present":"p","on":{"nodes":[1],"channel":1,"within":"map"}}"#,
                    r#"{"present":"p","on":{"channel":1}}"#,
                    r#"{"present":"p","on":{"open":"file:///etc/passwd"}}"#,
                ]);
                (GenStep::Malformed, json.to_string())
            }
        };
    }
    match choices.below(7) {
        0..=3 => {
            let (kind, name) = *choices.pick(&[
                ("present", "p"),
                ("present", "h"),
                ("stop", "p"),
                ("stop", "h"),
                ("edit", "e"),
            ]);
            let (on, ids_ok, parts) = target(choices, false);
            (
                GenStep::Targeted {
                    known: true,
                    ids_ok,
                    parts,
                },
                format!(r#"{{"{kind}":"{name}","on":{on}}}"#),
            )
        }
        4 => {
            let ms = match choices.below(3) {
                0 => u64::from(choices.below(400)),
                1 => u64::from(MAX_ANIMATE_MS),
                _ => 0,
            };
            (GenStep::Wait(ms), format!(r#"{{"wait":{ms}}}"#))
        }
        _ if n + 1 < count => {
            let callee = n + 1 + choices.index(count - n - 1);
            (GenStep::Call(Some(callee)), format!(r#"{{"verb":"v{callee}"}}"#))
        }
        _ => (GenStep::Wait(1), r#"{"wait":1}"#.to_string()),
    }
}

/// A target `choices` build, whether its node ids are in bounds, and its
/// parts: one, and one for each id. Its ids are each one a node can
/// carry — the map's, or ones it lacks (D27) — unless `out_of_bounds`.
fn target(choices: &mut Choices<'_>, out_of_bounds: bool) -> (String, bool, usize) {
    if out_of_bounds {
        let id = if choices.one_in(2) {
            0
        } else {
            u64::from(MAX_NODE_IDENT) + 1
        };
        return (format!(r#"{{"nodes":[1,{id}]}}"#), false, 3);
    }
    match choices.below(8) {
        0 => ("\"self\"".to_string(), true, 1),
        1 => ("\"selection\"".to_string(), true, 1),
        2 => ("\"children\"".to_string(), true, 1),
        3 => ("\"descendants\"".to_string(), true, 1),
        4 => {
            let within = if choices.one_in(2) { "self" } else { "map" };
            (
                format!(r#"{{"channel":{},"within":"{within}"}}"#, choices.below(3)),
                true,
                1,
            )
        }
        _ => {
            // Ids the map has, and ones it lacks (D27: they load, and are
            // skipped when the step runs).
            let n = 1 + choices.index(4);
            let mut ok = true;
            let ids: Vec<String> = (0..n)
                .map(|_| {
                    let id: u64 = match choices.below(5) {
                        0 => u64::from(choices.below(1_000_000)) + 6,
                        1 => u64::from(MAX_NODE_IDENT),
                        _ => u64::from(1 + choices.below(5)),
                    };
                    ok &= (1..=u64::from(MAX_NODE_IDENT)).contains(&id);
                    id.to_string()
                })
                .collect();
            (format!(r#"{{"nodes":[{}]}}"#, ids.join(",")), ok, 1 + n)
        }
    }
}

/// Whether the format accepts `verbs`, by the reference model: every
/// step well formed with names the map has, ids in bounds and waits
/// within a minute; the calls acyclic and at most [`MAX_VERB_DEPTH`]
/// verbs deep; and every verb at most [`MAX_DEFINITION_PARTS`] parts
/// once its calls are expanded. Plain recursion over the generated
/// verbs, with an explicit path for the cycle check.
fn model_accepts(verbs: &[Generated]) -> bool {
    let steps_ok = verbs
        .iter()
        .flat_map(|verb| &verb.steps)
        .all(|(step, _)| match step {
            GenStep::Targeted { known, ids_ok, .. } => *known && *ids_ok,
            GenStep::Wait(ms) => *ms <= u64::from(MAX_ANIMATE_MS),
            GenStep::Call(callee) => callee.is_some(),
            GenStep::Malformed => false,
        });
    if !steps_ok {
        return false;
    }
    /// The depth of `verb`'s calls, or `None` on a cycle through `path`.
    fn depth(verbs: &[Generated], verb: usize, path: &mut Vec<usize>) -> Option<usize> {
        if path.contains(&verb) {
            return None;
        }
        if path.len() > MAX_VERB_DEPTH + 1 {
            // Deeper than the bound already: no need to walk on.
            return Some(path.len());
        }
        path.push(verb);
        let mut deepest = 1;
        for (step, _) in &verbs[verb].steps {
            if let GenStep::Call(Some(callee)) = step {
                deepest = deepest.max(1 + depth(verbs, *callee, path)?);
            }
        }
        path.pop();
        Some(deepest)
    }
    /// `verb`'s parts with its calls expanded, saturating.
    fn parts(verbs: &[Generated], verb: usize) -> usize {
        verbs[verb].steps.iter().fold(0usize, |sum, (step, _)| {
            sum.saturating_add(match step {
                GenStep::Targeted { parts, .. } => *parts,
                GenStep::Call(Some(callee)) => 1usize.saturating_add(parts(verbs, *callee)),
                _ => 1,
            })
        })
    }
    for verb in 0..verbs.len() {
        match depth(verbs, verb, &mut Vec::new()) {
            None => return false,
            Some(deep) if deep > MAX_VERB_DEPTH => return false,
            Some(_) => {}
        }
    }
    (0..verbs.len()).all(|verb| parts(verbs, verb) <= MAX_DEFINITION_PARTS)
}

/// The node with the ident `ident` in `doc`. For the tests.
pub fn node(doc: &Document, ident: u32) -> NodeId {
    doc.node_by_ident(NodeIdent(ident))
        .unwrap_or_else(|| panic!("the map has a node {ident}"))
}
