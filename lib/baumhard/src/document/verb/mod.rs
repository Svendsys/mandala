// SPDX-License-Identifier: MPL-2.0

//! **Verbs**: the map's public interface (design §11, D7; V1-D3, #252).
//!
//! A [`VerbDef`](crate::document::verb::VerbDef) is a named sequence of
//! [steps](crate::document::verb::Step): start and stop the map's
//! presentations, apply the map's edits, wait on the app clock, and run
//! another of the map's verbs inline. That is all a verb can do — no
//! step reaches a console line, an Action, a file or the process (§11.3)
//! — and a node's click and hover, a span's link, the console, IPC and
//! the user's and the app's macros invoke them.
//!
//! A step's [targets](crate::document::verb::On) are resolved when it
//! runs, against the document as it then stands
//! ([`On::nodes`](crate::document::verb::On::nodes)); a node named by id
//! that the map no longer has is skipped with a warning (D27).
//!
//! What a definition may be is held at load (`verb::load`); how a verb runs, on the app
//! clock, with several runs at once and the held runs a hover makes, is
//! [`run`](crate::document::verb::run).

pub(crate) mod load;
pub mod run;

pub use run::{Performer, RunId, Runs, VerbError};

use crate::arena::NodeId;
use crate::document::edit::reach::elements;
use crate::document::edit::{Reach, Target};
use crate::document::element::DocKind;
use crate::document::ids::{Definitions, Document, NodeIdent};
use crate::document::values::{PresentationId, VerbId};

/// The deepest a chain of verb calls may go: a verb that calls a verb
/// that calls a verb … at most this many verbs long (#252 §1). A verb
/// that calls none is one deep.
pub const MAX_VERB_DEPTH: usize = 8;

/// A verb the map defines (design §11.1). See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct VerbDef {
    /// The id triggers, links, the console and IPC name it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
    /// What it does.
    pub description: String,
    /// What it does, in order.
    pub steps: Vec<Step>,
}

/// One step of a verb: one of five kinds, and no other (the sandbox,
/// design §11.3).
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// Start one of the map's presentations on the targets.
    Present {
        /// The presentation: an index into the map's list.
        presentation: PresentationId,
        /// Where.
        on: On,
    },
    /// Stop one of the map's presentations on the targets: each
    /// instance there plays its way out.
    Stop {
        /// The presentation: an index into the map's list.
        presentation: PresentationId,
        /// Where.
        on: On,
    },
    /// Apply one of the map's edits at the targets: one history entry,
    /// recorded with `ChangeSource::Verb`.
    Edit {
        /// The edit's id, found among the map's edits when the step
        /// runs.
        edit: String,
        /// Where.
        on: On,
    },
    /// Wait on the app clock: the run is suspended until the clock
    /// reaches the time.
    Wait {
        /// How long, in milliseconds.
        ms: u32,
    },
    /// Run another of the map's verbs inline, in the same run.
    Verb {
        /// The verb.
        verb: VerbId,
    },
}

/// Which nodes a step acts on, resolved when it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum On {
    /// The node the verb was invoked on.
    SelfNode,
    /// What is selected when the step runs.
    Selection,
    /// These nodes, by id. One the map does not have when the step runs
    /// is skipped with a warning (D27).
    Nodes(Vec<NodeIdent>),
    /// The nodes on a channel.
    Channel {
        /// The channel.
        channel: usize,
        /// Where they are looked for.
        within: Within,
    },
    /// The child nodes of the node the verb was invoked on.
    Children,
    /// Every node below the node the verb was invoked on.
    Descendants,
}

/// Where a [`On::Channel`] target looks.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Within {
    /// Every node below the node the verb was invoked on.
    SelfNode,
    /// Every node of the map.
    Map,
}

impl On {
    /// Whether resolving this target needs the node the verb was invoked
    /// on. O(1).
    pub fn needs_self(&self) -> bool {
        match self {
            On::SelfNode
            | On::Children
            | On::Descendants
            | On::Channel {
                within: Within::SelfNode,
                ..
            } => true,
            On::Selection
            | On::Nodes(_)
            | On::Channel {
                within: Within::Map, ..
            } => false,
        }
    }

    /// The nodes this target names on `doc` as it stands, for a verb
    /// invoked on `self_node`, with `selection` selected: in tree order
    /// for the targets that walk the tree, in the order given for `nodes`
    /// and the selection. A node named by id that `doc` does not have is
    /// skipped with a warning (D27); so is a `self_node` that is no longer
    /// in the map, which names nothing. `None` for a target that needs
    /// `self` when there is none.
    ///
    /// Cost: O(1) for `self`; the selection's length; one index lookup
    /// per named id; the child nodes of `self`, or the subtree below it,
    /// for `children`, `descendants` and a channel within `self`; every
    /// node of the map for a channel within the map. Iterative: a deep
    /// subtree costs its size, never stack.
    pub fn nodes(
        &self,
        doc: &Document,
        self_node: Option<NodeId>,
        selection: &[NodeId],
    ) -> Option<Vec<NodeId>> {
        let own = || {
            let node = self_node?;
            if doc
                .get(node)
                .is_some_and(|element| element.kind() == DocKind::Node)
            {
                Some(Some(node))
            } else {
                log::warn!(
                    "verb: the node it was invoked on is no longer in the map; the step acts on nothing"
                );
                Some(None)
            }
        };
        let below =
            |reach: Reach| own().map(|node| node.map_or_else(Vec::new, |n| reached(doc, &[n], reach)));
        Some(match self {
            On::SelfNode => own()?.into_iter().collect(),
            On::Selection => selection.to_vec(),
            On::Nodes(idents) => idents
                .iter()
                .filter_map(|&ident| {
                    let found = doc.node_by_ident(ident);
                    if found.is_none() {
                        log::warn!("verb: no node has the id {}; skipping it", ident.0);
                    }
                    found
                })
                .collect(),
            On::Children => below(Reach::Children)?,
            On::Descendants => below(Reach::Descendants)?,
            On::Channel { channel, within } => {
                let nodes = match within {
                    Within::SelfNode => below(Reach::Descendants)?,
                    Within::Map => {
                        let roots: Vec<NodeId> = doc
                            .children(doc.root())
                            .filter(|&child| doc.get(child).is_some_and(|e| e.kind() == DocKind::Node))
                            .collect();
                        reached(doc, &roots, Reach::Subtree)
                    }
                };
                nodes
                    .into_iter()
                    .filter(|&node| {
                        doc.get(node)
                            .and_then(|e| e.as_node())
                            .is_some_and(|n| n.channel == *channel)
                    })
                    .collect()
            }
        })
    }
}

/// The nodes `reach` names from `anchors`, in tree order.
fn reached(doc: &Document, anchors: &[NodeId], reach: Reach) -> Vec<NodeId> {
    elements(doc, anchors, reach, Target::Node, None)
}

impl VerbDef {
    /// Whether this verb has a step that changes the map: an `edit`
    /// step, its own or one of a verb it runs. What `verb list` and
    /// `describe` mark as "changes the map" (design §11.1); a verb
    /// without one is a performance. O(the verbs it reaches and their
    /// steps): each is looked at once.
    pub fn changes_the_map(&self, definitions: &Definitions) -> bool {
        any_reached(self, definitions, |step| matches!(step, Step::Edit { .. }))
    }

    /// Whether running this verb needs a node to run it on: a step of
    /// its own, or of a verb it runs, targets `self`, its children, its
    /// descendants or a channel within it. O(the verbs it reaches and
    /// their steps).
    pub fn needs_self(&self, definitions: &Definitions) -> bool {
        any_reached(self, definitions, |step| match step {
            Step::Present { on, .. } | Step::Stop { on, .. } | Step::Edit { on, .. } => on.needs_self(),
            Step::Wait { .. } | Step::Verb { .. } => false,
        })
    }
}

/// Whether `test` holds for a step of `verb` or of a verb it runs. Each
/// verb is looked at once, so a verb reached by many paths costs one
/// look.
fn any_reached(verb: &VerbDef, definitions: &Definitions, test: impl Fn(&Step) -> bool) -> bool {
    let mut seen = vec![false; definitions.verbs.len()];
    let mut pending: Vec<&VerbDef> = vec![verb];
    while let Some(next) = pending.pop() {
        for step in &next.steps {
            if test(step) {
                return true;
            }
            if let Step::Verb { verb } = step {
                let index = usize::from(verb.0);
                if let (Some(flag), Some(callee)) = (seen.get_mut(index), definitions.verbs.get(index)) {
                    if !*flag {
                        *flag = true;
                        pending.push(callee);
                    }
                }
            }
        }
    }
    false
}

/// Every node ident the verbs of `definitions` name in a `nodes` target:
/// what the ident counter starts above (D32). O(the steps).
pub(crate) fn named_idents(verbs: &[VerbDef]) -> impl Iterator<Item = u32> + '_ {
    verbs
        .iter()
        .flat_map(|verb| verb.steps.iter())
        .filter_map(|step| match step {
            Step::Present { on, .. } | Step::Stop { on, .. } | Step::Edit { on, .. } => Some(on),
            Step::Wait { .. } | Step::Verb { .. } => None,
        })
        .flat_map(|on| match on {
            On::Nodes(idents) => idents.as_slice(),
            _ => &[],
        })
        .map(|ident| ident.0)
}
