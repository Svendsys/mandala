// SPDX-License-Identifier: MPL-2.0

//! **Running verbs** (#252 §2; design §11.1).
//!
//! An invocation makes a run: the verb, the node it was invoked on (its
//! `self`, if any), where it is in its steps, the time it has reached on
//! the app clock, and the presentation instances it started. Several runs
//! may be active at once ([`Runs`]).
//!
//! - **`present`, `stop`, `edit` and `verb` steps run at once, in order;
//!   `wait` suspends the run.** A `verb` step runs the callee's steps
//!   inline, in the same run, and comes back to the step after it.
//! - **Targets are resolved when the step runs**, against the document
//!   as it then stands ([`On::nodes`]).
//! - **A run keeps time by its own steps, not by the frames that resume
//!   it.** A `wait` moves the run's time on by its milliseconds, and the
//!   steps after it run at that time — a presentation starts there, an
//!   animated edit starts there — whenever the frame that resumes it
//!   comes. So on a virtual clock (headless, IPC `clock.step`) a run is
//!   deterministic, and on a real one it does not drift with the frame
//!   rate.
//! - **A held run** — a hover's — is kept after its last step until it is
//!   [released](Runs::release): its remaining steps are skipped, and
//!   every `while` instance it started is stopped, so each plays its way
//!   out. A run that is not held is dropped after its last step.
//!
//! What a step does to the app — start or stop a presentation, apply an
//! edit through the app's one funnel, read the selection — is the
//! [`Performer`]'s, which the app implements over its open map. The
//! runner never writes a document itself.
//!
//! **What a run's work is bounded by**: the load's checks (`load`) — the
//! call graph is acyclic and at most [`MAX_VERB_DEPTH`] verbs deep, and a
//! verb is at most
//! [`MAX_DEFINITION_PARTS`](crate::document::bounds::MAX_DEFINITION_PARTS)
//! parts once the verbs it runs are expanded — so one invocation runs at
//! most that many steps, however its calls fan out; and every `wait` is
//! at most a minute.

use crate::arena::NodeId;
use crate::document::format::context::shown;
use crate::document::ids::Document;
use crate::document::presentation::{InstanceId, Play, PresentError};
use crate::document::values::{EditId, PresentationId, VerbId};
use crate::document::verb::{On, Step, MAX_VERB_DEPTH};
use std::fmt;

/// What a run needs from the app it runs in: the document to resolve
/// targets against, the selection, and the four things a step does to
/// the map. The app implements it over its open map
/// (`MindMapDocument`), where an edit goes through the funnel every
/// route takes; the fuzz properties over a bare [`Document`].
pub trait Performer {
    /// The document, as it stands.
    fn document(&self) -> &Document;

    /// What is selected now.
    fn selection(&self) -> Vec<NodeId>;

    /// Start the map's presentation `id` on `nodes` at the clock reading
    /// `at_ms`: the instances it now plays as. The answer is the start's
    /// (`Document::start_defined`), and is not where a refusal of the
    /// platform's budget arrives when the layer is set aside — that one
    /// arrives later, on the instance — so nothing here relies on it
    /// beyond the instances it names. A run asks only for a presentation
    /// the map defines.
    fn start(
        &mut self,
        id: PresentationId,
        nodes: &[NodeId],
        at_ms: f64,
    ) -> Result<Vec<InstanceId>, PresentError>;

    /// Stop the map's presentation `id` on `nodes` at `at_ms`. Returns
    /// how many instances it stopped.
    fn stop(&mut self, id: PresentationId, nodes: &[NodeId], at_ms: f64) -> usize;

    /// Stop the instances `ids` that still play, at `at_ms`. Returns how
    /// many it stopped.
    fn stop_instances(&mut self, ids: &[InstanceId], at_ms: f64) -> usize;

    /// Apply the map's edit `id` at `nodes`, at `at_ms`, recorded with
    /// `ChangeSource::Verb`; or why it did not.
    fn edit(&mut self, id: EditId, nodes: &[NodeId], at_ms: f64) -> Result<(), String>;
}

/// A run, as [`Runs::invoke`] names it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RunId(pub u64);

/// Why a verb could not be invoked. Nothing ran.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum VerbError {
    /// The map defines no verb by that id.
    Unknown {
        /// The id asked for.
        verb: String,
    },
    /// The verb acts on the node it is invoked on, and it was invoked on
    /// none.
    NeedsNode {
        /// The verb's id.
        verb: String,
    },
    /// The node it was invoked on is not a node of the map.
    NotANode {
        /// The verb's id.
        verb: String,
    },
}

/// Every part that may hold what a map wrote is shown as file text is.
impl fmt::Display for VerbError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VerbError::Unknown { verb } => write!(f, "the map defines no verb `{}`", shown(verb)),
            VerbError::NeedsNode { verb } => write!(
                f,
                "verb `{}` acts on the node it is run on: name a node to run it on",
                shown(verb)
            ),
            VerbError::NotANode { verb } => {
                write!(
                    f,
                    "verb `{}`: what it was run on is not a node of the map",
                    shown(verb)
                )
            }
        }
    }
}

impl std::error::Error for VerbError {}

/// One run. See the [module docs](self).
#[derive(Clone, Debug)]
struct Run {
    id: RunId,
    /// The verb invoked: what a run is reported as.
    verb: VerbId,
    /// The node it was invoked on.
    self_node: Option<NodeId>,
    /// Whether a release ends it (a hover's).
    held: bool,
    /// The verbs it is inside, outermost first, each with the index of
    /// its next step. Empty once the last step has run.
    frames: Vec<(VerbId, usize)>,
    /// The time the run has reached: its invocation, moved on by each
    /// `wait` it has run.
    at_ms: f64,
    /// While suspended by a `wait`, when it resumes.
    waiting: bool,
    /// The `while` instances it started, which a release stops.
    held_instances: Vec<InstanceId>,
}

/// A run, as [`Runs::runs`] reports it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct RunInfo {
    /// Which run.
    pub id: RunId,
    /// The verb invoked.
    pub verb: VerbId,
    /// The node it was invoked on.
    pub self_node: Option<NodeId>,
    /// Whether a release ends it.
    pub held: bool,
    /// When a `wait` suspends it, the clock reading it resumes at.
    pub resumes_at_ms: Option<f64>,
}

/// The runs active on one document. See the [module docs](self).
#[derive(Clone, Debug, Default)]
pub struct Runs {
    /// In the order they were invoked.
    runs: Vec<Run>,
    next: u64,
}

/// What one step resolved to, owned, so that the document can be lent
/// out again to do it: the definitions it names by index, and the nodes
/// it acts on.
enum Act {
    /// Start a presentation that plays as `Play`.
    Start(PresentationId, Play, Vec<NodeId>),
    Stop(PresentationId, Vec<NodeId>),
    Edit(EditId, Vec<NodeId>),
    Wait(u32),
    Call(VerbId),
    Nothing,
}

impl Runs {
    /// Invoke `verb` on `self_node` — an element stands for the node it
    /// belongs to — at the clock reading `now_ms`: its steps run at once
    /// up to its first `wait`, or its end. A `held` run is kept until it
    /// is [released](Self::release), even once its last step has run;
    /// any other is dropped then.
    ///
    /// Errors, with nothing run: the map has no such verb; the verb, or
    /// one it runs, acts on the node it is invoked on and there is none
    /// ([`VerbError::NeedsNode`]), or `self_node` is not in a node of the
    /// map.
    ///
    /// Cost: the steps up to the first `wait`, each its target's
    /// resolution and what it does.
    pub fn invoke(
        &mut self,
        verb: VerbId,
        self_node: Option<NodeId>,
        held: bool,
        now_ms: f64,
        performer: &mut dyn Performer,
    ) -> Result<RunId, VerbError> {
        let doc = performer.document();
        let definitions = doc.definitions();
        let Some(def) = definitions.verbs.get(usize::from(verb.0)) else {
            return Err(VerbError::Unknown {
                verb: format!("#{}", verb.0),
            });
        };
        let self_node = match self_node {
            Some(given) => Some(doc.owning_node(given).ok_or_else(|| VerbError::NotANode {
                verb: def.id.to_string(),
            })?),
            None => None,
        };
        if self_node.is_none() && def.needs_self(definitions) {
            return Err(VerbError::NeedsNode {
                verb: def.id.to_string(),
            });
        }
        let id = RunId(self.next);
        self.next += 1;
        let mut run = Run {
            id,
            verb,
            self_node,
            held,
            frames: vec![(verb, 0)],
            at_ms: now_ms,
            waiting: false,
            held_instances: Vec::new(),
        };
        advance(&mut run, now_ms, performer);
        if held || run.waiting {
            self.runs.push(run);
        }
        Ok(id)
    }

    /// Resume every run whose `wait` is over at `now_ms`, each at the
    /// time its wait ends, and drop the runs that are not held once their
    /// last step has run. Returns whether any step ran.
    ///
    /// Cost: O(the runs), and the steps of the ones that resume.
    pub fn tick(&mut self, now_ms: f64, performer: &mut dyn Performer) -> bool {
        let mut ran = false;
        for run in &mut self.runs {
            if run.waiting && run.at_ms <= now_ms {
                run.waiting = false;
                advance(run, now_ms, performer);
                ran = true;
            }
        }
        self.runs.retain(|run| run.held || run.waiting);
        ran
    }

    /// Release the held run `id` at `now_ms`: the steps it has not run
    /// are skipped, and every `while` instance it started that still
    /// plays is stopped, to play its way out. Returns whether there was
    /// such a run. Releasing a run that is not held does nothing: only a
    /// hold ends that way.
    ///
    /// Cost: O(the runs), and the instances it stops.
    pub fn release(&mut self, id: RunId, now_ms: f64, performer: &mut dyn Performer) -> bool {
        let Some(at) = self.runs.iter().position(|run| run.id == id && run.held) else {
            return false;
        };
        let run = self.runs.remove(at);
        if !run.held_instances.is_empty() {
            performer.stop_instances(&run.held_instances, now_ms);
        }
        true
    }

    /// Whether a run is suspended by a `wait`: one that a frame will
    /// resume, so the app's clock owes it frames. O(the runs).
    pub fn waiting(&self) -> bool {
        self.runs.iter().any(|run| run.waiting)
    }

    /// The earliest clock reading a suspended run resumes at, if any.
    /// O(the runs).
    pub fn next_due_ms(&self) -> Option<f64> {
        self.runs
            .iter()
            .filter(|run| run.waiting)
            .map(|run| run.at_ms)
            .reduce(f64::min)
    }

    /// The runs, in the order they were invoked. No allocation.
    pub fn runs(&self) -> impl Iterator<Item = RunInfo> + '_ {
        self.runs.iter().map(|run| RunInfo {
            id: run.id,
            verb: run.verb,
            self_node: run.self_node,
            held: run.held,
            resumes_at_ms: run.waiting.then_some(run.at_ms),
        })
    }
}

/// Run `run`'s steps from where it is until a `wait` suspends it past
/// `now_ms` or its last step has run.
fn advance(run: &mut Run, now_ms: f64, performer: &mut dyn Performer) {
    loop {
        let Some(&mut (verb, ref mut next)) = run.frames.last_mut() else {
            return;
        };
        let index = *next;
        *next += 1;
        let act = {
            let doc = performer.document();
            let Some(def) = doc.definitions().verbs.get(usize::from(verb.0)) else {
                run.frames.clear();
                return;
            };
            match def.steps.get(index) {
                None => {
                    run.frames.pop();
                    continue;
                }
                Some(step) => resolve(step, doc, run.self_node, performer, &def.id, index),
            }
        };
        match act {
            Act::Start(id, play, nodes) => match performer.start(id, &nodes, run.at_ms) {
                // The start's answer names the instances; whether the
                // layer can show them is the layer's to say, on each.
                Ok(ids) => {
                    if play == Play::While && run.held {
                        run.held_instances.extend(ids);
                    }
                }
                Err(error) => log::warn!("verb: {error}"),
            },
            Act::Stop(id, nodes) => {
                performer.stop(id, &nodes, run.at_ms);
            }
            Act::Edit(id, nodes) => {
                if let Err(message) = performer.edit(id, &nodes, run.at_ms) {
                    log::warn!("verb: {message}");
                }
            }
            Act::Wait(ms) => {
                run.at_ms += f64::from(ms);
                if run.at_ms > now_ms {
                    run.waiting = true;
                    return;
                }
            }
            Act::Call(callee) => {
                if run.frames.len() >= MAX_VERB_DEPTH {
                    // The load refuses a deeper chain; a run never meets
                    // one.
                    log::error!("verb: a call past the depth a load allows; skipping it");
                } else {
                    run.frames.push((callee, 0));
                }
            }
            Act::Nothing => {}
        }
    }
}

/// What `step` — step `index` of the verb `verb` — does on `doc` as it
/// stands, for a run on `self_node`, its targets resolved.
fn resolve(
    step: &Step,
    doc: &Document,
    self_node: Option<NodeId>,
    performer: &dyn Performer,
    verb: &str,
    index: usize,
) -> Act {
    let targets = |on: &On| {
        let selection = if *on == On::Selection {
            performer.selection()
        } else {
            Vec::new()
        };
        let nodes = on.nodes(doc, self_node, &selection);
        if nodes.is_none() {
            // The invocation checked this; a definition cannot change
            // while the map is open.
            log::error!(
                "verb: `{}`, step {index}, acts on the node it was run on, and there is none",
                shown(verb)
            );
        }
        nodes.unwrap_or_default()
    };
    let presentation = |id: PresentationId| doc.definitions().presentations.get(usize::from(id.0));
    match step {
        Step::Present { presentation: id, on } => match presentation(*id) {
            Some(def) => Act::Start(*id, def.play, targets(on)),
            None => Act::Nothing,
        },
        Step::Stop { presentation: id, on } => match presentation(*id) {
            Some(_) => Act::Stop(*id, targets(on)),
            None => Act::Nothing,
        },
        Step::Edit { edit, on } => match doc.definitions().edits.get(usize::from(edit.0)) {
            Some(_) => Act::Edit(*edit, targets(on)),
            None => Act::Nothing,
        },
        Step::Wait { ms } => Act::Wait(*ms),
        Step::Verb { verb } => Act::Call(*verb),
    }
}
