// SPDX-License-Identifier: MPL-2.0

//! Field editing I (V1-C5, #246): the fields of nodes, sections, frames
//! and their rings, the theme and the defaults, and the style every
//! selected element takes whatever its kind — on the one writer
//! (design §8).
//!
//! **One user action is one request.** Each function here reads the
//! document and the selection and returns the ordered steps of one
//! change ([`Steps`]); [`apply`] hands them to
//! [`MindMapDocument::apply`] as one [`EditRequest`], so a key, a
//! console line or an IPC command is one undo entry however many
//! elements it reaches. The keybinds' Actions
//! (`app::dispatch::families::field_nodes`) and the console verbs
//! (`node`, `section`, `frame`, `color`, `font`, `zoom`, `range`,
//! `theme`, `defaults`) are thin readers of their arguments over these
//! bodies, so the two surfaces cannot disagree about what an edit
//! does.
//!
//! **Elements are grouped by what they receive.** Every element that
//! takes the same mutations is an anchor of one walk
//! ([`compose::set`]), so a color on fifty nodes is one walk, and an
//! edit whose value depends on the element — one zoom bound kept, the
//! next frame glyph — is one walk per distinct value.
//!
//! **Values are read as the file reads them** where the file has a
//! spelling ([`values`]): a color through `format::color`, a font
//! through `format::font`. Bounds are the writer's: a size, a radius or
//! a channel out of bounds is refused by the request's validation,
//! which names the field.

pub(crate) mod frame;
pub(crate) mod map;
pub(crate) mod node;
pub(crate) mod section;
pub(crate) mod style;
pub(crate) mod values;

use std::hash::Hash;

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::ids::Document;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::DocMutation;
use baumhard::document::mutation::step::DocStep;
use baumhard::document::writer::error::ApplyError;
use baumhard::document::writer::{ChangeSource, EditRequest};
use rustc_hash::FxHashMap;

use crate::application::document::selection::kind_of;
use crate::application::document::{MindMapDocument, TRANSACTION_OPEN_STATUS};

/// The steps of one change, or why there is nothing to apply: a value
/// that does not read, a selection with nothing the edit reaches.
pub(crate) type Steps = Result<Vec<DocStep>, String>;

/// Why an edit did not apply.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Refused {
    /// An interactive transaction holds the document (D28): the status
    /// line says so ([`TRANSACTION_OPEN_STATUS`]).
    Busy,
    /// The edit could not be built, or the writer refused it: the
    /// message names what and why.
    Because(String),
}

impl Refused {
    /// The message a console line answers with.
    pub(crate) fn message(&self) -> &str {
        match self {
            Refused::Busy => TRANSACTION_OPEN_STATUS,
            Refused::Because(why) => why,
        }
    }
}

/// Apply `steps` as one request called `label`, declared by `source`.
/// `Ok(false)` when there was nothing to apply; a refusal of the writer
/// is reported on the status line by [`MindMapDocument::apply`] as well.
pub(crate) fn apply(
    doc: &mut MindMapDocument,
    steps: Steps,
    label: &str,
    source: ChangeSource,
) -> Result<bool, Refused> {
    let steps = steps.map_err(Refused::Because)?;
    if steps.is_empty() {
        return Ok(false);
    }
    match doc.apply(EditRequest::new(steps, label), source) {
        Ok(_) => Ok(true),
        Err(ApplyError::TransactionOpen) => Err(Refused::Busy),
        Err(error) => Err(Refused::Because(format!("{label}: {error}"))),
    }
}

/// The selected elements of `kind`, in selection order.
pub(crate) fn selected(doc: &MindMapDocument, kind: DocKind) -> Vec<NodeId> {
    let d = doc.doc();
    doc.selection
        .iter()
        .filter(|&id| kind_of(d, id) == Some(kind))
        .collect()
}

/// Elements grouped by what they receive: one group per element kind
/// and key, each with the mutations that key stands for, in the order
/// each group was first met. The caller keys an edit whose mutations
/// depend only on the element's kind with `()`, and one whose value
/// depends on the element — a zoom window, a toggled flag — with that
/// value, so equal keys of one kind always mean equal mutations.
pub(crate) struct Groups<K> {
    index: FxHashMap<(DocKind, K), usize>,
    groups: Vec<(DocKind, K, Vec<DocMutation>, Vec<NodeId>)>,
}

impl<K> Default for Groups<K> {
    fn default() -> Self {
        Groups {
            index: FxHashMap::default(),
            groups: Vec::new(),
        }
    }
}

impl<K: Hash + Eq + Clone> Groups<K> {
    /// Give `id`, an element of `doc`, the mutations `key` stands for:
    /// `mutations` makes them, once, for the first element of its group.
    /// Nothing for an id the document does not hold. O(1) amortized, so a
    /// selection is grouped in time linear in its size.
    pub(crate) fn add(
        &mut self,
        doc: &Document,
        id: NodeId,
        key: K,
        mutations: impl FnOnce() -> Vec<DocMutation>,
    ) {
        let Some(kind) = kind_of(doc, id) else { return };
        let groups = &mut self.groups;
        let at = *self.index.entry((kind, key.clone())).or_insert_with(|| {
            groups.push((kind, key, mutations(), Vec::new()));
            groups.len() - 1
        });
        self.groups[at].3.push(id);
    }

    /// Whether no element was added.
    pub(crate) fn is_empty(&self) -> bool {
        self.groups
            .iter()
            .all(|(_, _, mutations, _)| mutations.is_empty())
    }

    /// Each group's key and elements, in the order each was first met,
    /// for an edit that is not a [`compose::set`].
    pub(crate) fn members(self) -> impl Iterator<Item = (K, Vec<NodeId>)> {
        self.groups.into_iter().map(|(_, key, _, ids)| (key, ids))
    }

    /// One walk per group with mutations ([`compose::set`]).
    pub(crate) fn steps(self, doc: &Document) -> Steps {
        let mut steps = Vec::new();
        for (kind, _, mutations, ids) in self.groups {
            steps.extend(compose::set(doc, kind, mutations, &ids).map_err(|e| e.to_string())?);
        }
        Ok(steps)
    }
}
