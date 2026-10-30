// SPDX-License-Identifier: MPL-2.0

//! [`MutationError`](crate::document::mutation::error::MutationError):
//! why a document mutator tree, a step or a composite builder was
//! refused. Each refusal names where: the mutator's path from its root
//! for what [`validate`](crate::document::mutation::validate::validate)
//! finds before a walk, the element for what
//! [`validate_applied`](crate::document::mutation::validate::validate_applied)
//! finds after one.

use crate::document::bounds::BoundError;
use crate::document::element::DocKind;
use crate::document::ids::NodeIdent;
use crate::document::shape::ShapeError;
use crate::document::values::{FontSize, LinkKind};
use crate::engine::tree::TreeError;
use indextree::NodeId;
use std::fmt;

/// Where in a mutator tree a refusal was found: the position of each
/// child taken from the mutator's root down to the offending mutator
/// (empty for the root), and, for an element inside an
/// [`Insert`](crate::engine::structure::StructOp::Insert)'s subtree, the
/// same path within the subtree.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct MutatorPath {
    /// From the mutator tree's root to the mutator.
    pub mutator: Vec<usize>,
    /// From an inserted subtree's root to the element, when the refusal
    /// is inside one.
    pub subtree: Option<Vec<usize>>,
}

impl fmt::Display for MutatorPath {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "mutator {:?}", self.mutator)?;
        if let Some(subtree) = &self.subtree {
            write!(f, ", inserted element {subtree:?}")?;
        }
        Ok(())
    }
}

/// A structural command the document does not allow.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum StructureRule {
    /// A `Move` whose target is not a node.
    MoveNotANode,
    /// A `Move` whose destination is neither a node nor the map.
    MoveDestination,
    /// A `Split` or `Merge` whose target is not a span.
    SplitMergeNotASpan,
    /// A command on the map row's theme, palettes or defaults: an insert
    /// under the map of anything but a node, or a remove or move keyed
    /// to one of those kinds. Their indices are what colors and
    /// palettes refer to, so V1 never adds, removes or reorders them.
    MapRow,
    /// A command keyed to the map, which is the root and cannot be
    /// removed, moved, split or merged — or an insert of a map.
    Map,
}

impl fmt::Display for StructureRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            StructureRule::MoveNotANode => "only a node moves",
            StructureRule::MoveDestination => "a node moves under a node or the map",
            StructureRule::SplitMergeNotASpan => "only a span splits or merges",
            StructureRule::MapRow => "the theme, palettes and defaults are not inserted, removed or moved",
            StructureRule::Map => "the map is the root: it is not inserted, removed, moved, split or merged",
        })
    }
}

/// Why a document mutation was refused. Nothing was applied, or what
/// was applied is undone (see where each is returned).
#[derive(Clone, Debug, PartialEq)]
pub enum MutationError {
    /// A payload whose variant cannot apply to its mutator's kind.
    PayloadKind {
        /// Where.
        at: MutatorPath,
        /// The mutator's key kind.
        kind: DocKind,
        /// The payload's variant.
        payload: &'static str,
    },
    /// A literal value out of its bound.
    Bound {
        /// Where.
        at: MutatorPath,
        /// The field, as `kind.field`.
        field: &'static str,
        /// The bound it broke.
        error: BoundError,
    },
    /// A size in the unit its place does not take: points inside a frame,
    /// or a relative size outside one.
    Unit {
        /// Where.
        at: MutatorPath,
        /// The field.
        field: &'static str,
        /// The size given.
        size: FontSize,
    },
    /// A grapheme range whose start is past its end: a malformed
    /// request, not an empty one.
    InvertedRange {
        /// Where.
        at: MutatorPath,
        /// The first cluster named.
        start: usize,
        /// The cluster after the last one named.
        end: usize,
    },
    /// A face that names no compiled-in font: `AppFont::Any` is the
    /// shaper's fallback, which a map never stores (ruling R11.2 on #236).
    AnyFont {
        /// Where.
        at: MutatorPath,
    },
    /// A name an author chose — a theme variant or variable, a palette —
    /// that a file cannot hold: empty, or with a control or bidi
    /// character ([`name_fault`](crate::document::place::name_fault),
    /// the rule the loader holds a file's names to).
    Name {
        /// Where.
        at: MutatorPath,
        /// What the name names: `"theme variant"`, `"theme variable"`,
        /// `"palette"`.
        what: &'static str,
        /// Why it may not.
        fault: crate::document::place::NameFault,
    },
    /// A span inside a ring with a size of its own, which a ring never
    /// reads (one size per ring, #235 amendment 3) and a load refuses
    /// (ruling R11.2 on #236).
    RingSpanSize {
        /// Where.
        at: MutatorPath,
    },
    /// A reference to something the document does not have.
    Missing {
        /// Where.
        at: MutatorPath,
        /// What is referred to: `"verb"`, `"palette"`, `"theme
        /// variable"`, `"theme variant"`, `"palette group"`.
        what: &'static str,
        /// Its index.
        index: usize,
    },
    /// A link whose target is not a node of the document.
    LinkTarget {
        /// Where.
        at: MutatorPath,
        /// The target.
        to: NodeId,
    },
    /// An instruction the document's walks do not take: `MapChildren`
    /// pairs by position and would carry a payload to another kind, and
    /// `SpatialDescend` needs a spatial element type.
    Instruction {
        /// Where.
        at: MutatorPath,
    },
    /// An inserted subtree breaks the shape rules.
    Subtree {
        /// The insert.
        at: MutatorPath,
        /// What is wrong.
        error: ShapeError,
    },
    /// An inserted node that carries an ident: the writer (V1-B4)
    /// assigns idents, so an inserted node carries
    /// [`NodeIdent::UNASSIGNED`].
    AssignedIdent {
        /// Where.
        at: MutatorPath,
        /// The ident it carries.
        ident: NodeIdent,
    },
    /// A structural command the document does not allow.
    Structure {
        /// Where.
        at: MutatorPath,
        /// Which rule.
        rule: StructureRule,
    },
    /// A `LinkOp::SetKind` without the commands that remove the
    /// children the new kind does not hold: a bare kind switch would
    /// leave the link's row broken.
    BareKindSwitch {
        /// Where.
        at: MutatorPath,
        /// The kind switched to.
        kind: LinkKind,
    },
    /// A structural command in a presentation, which changes fields only
    /// (D17).
    PresentationStructure {
        /// Where.
        at: MutatorPath,
    },
    /// After the steps: an element whose row breaks the shape rules.
    Shape(ShapeError),
    /// After the steps: a span inside a ring that sets a size of its
    /// own (see [`MutationError::RingSpanSize`]).
    PlacedRingSpanSize {
        /// The span.
        element: NodeId,
    },
    /// After the steps: a link that leads to its own node.
    SelfLink {
        /// The link.
        link: NodeId,
    },
    /// After the steps: a link whose target is not a node in the tree.
    DanglingLink {
        /// The link.
        link: NodeId,
        /// Its target.
        to: NodeId,
    },
    /// After the steps: a text size in the unit its place does not
    /// take — what a load would refuse.
    PlacedUnit {
        /// The element.
        element: NodeId,
        /// Its size.
        size: FontSize,
    },
    /// A walk the engine refused: a stale anchor, a failing structural
    /// command.
    Tree(TreeError),
    /// A composite builder was given an element that is not in the
    /// document, or not of the kind it works on.
    NotA {
        /// The element.
        element: NodeId,
        /// What it should have been.
        expected: &'static str,
    },
}

impl fmt::Display for MutationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MutationError::PayloadKind { at, kind, payload } => {
                write!(f, "{at}: a {payload} payload cannot apply to a {kind:?}")
            }
            MutationError::Bound { at, field, error } => write!(f, "{at}: {field}: {error}"),
            MutationError::Unit { at, field, size } => {
                write!(f, "{at}: {field}: {size:?} is not the unit this place takes")
            }
            MutationError::InvertedRange { at, start, end } => {
                write!(f, "{at}: the range {start}..{end} ends before it starts")
            }
            MutationError::AnyFont { at } => {
                write!(
                    f,
                    "{at}: a font must name a face; the fallback face is never stored"
                )
            }
            MutationError::Name { at, what, fault } => match fault {
                crate::document::place::NameFault::Empty => write!(f, "{at}: a {what} name is empty"),
                crate::document::place::NameFault::NotPrintable(c) => write!(
                    f,
                    "{at}: a {what} name holds U+{:04X}, a control or bidi character a file cannot hold",
                    u32::from(*c)
                ),
            },
            MutationError::RingSpanSize { at } => {
                write!(
                    f,
                    "{at}: a span inside a ring takes the ring's size and sets none"
                )
            }
            MutationError::Missing { at, what, index } => {
                write!(f, "{at}: the document has no {what} {index}")
            }
            MutationError::LinkTarget { at, to } => write!(f, "{at}: link target {to} is not a node"),
            MutationError::Instruction { at } => {
                write!(
                    f,
                    "{at}: the document walks neither MapChildren nor SpatialDescend"
                )
            }
            MutationError::Subtree { at, error } => write!(f, "{at}: {error}"),
            MutationError::AssignedIdent { at, ident } => write!(
                f,
                "{at}: an inserted node carries id {}; the writer assigns ids",
                ident.0
            ),
            MutationError::Structure { at, rule } => write!(f, "{at}: {rule}"),
            MutationError::BareKindSwitch { at, kind } => write!(
                f,
                "{at}: switching a link to {kind:?} needs the commands that remove what it no longer holds"
            ),
            MutationError::PresentationStructure { at } => {
                write!(f, "{at}: a presentation changes fields only, never structure")
            }
            MutationError::Shape(error) => write!(f, "{error}"),
            MutationError::PlacedRingSpanSize { element } => {
                write!(
                    f,
                    "{element}: a span inside a ring takes the ring's size and sets none"
                )
            }
            MutationError::SelfLink { link } => write!(f, "link {link} leads to its own node"),
            MutationError::DanglingLink { link, to } => {
                write!(f, "link {link} leads to {to}, which is not a node in the map")
            }
            MutationError::PlacedUnit { element, size } => {
                write!(f, "{element}: size {size:?} is not the unit its place takes")
            }
            MutationError::Tree(error) => write!(f, "{error}"),
            MutationError::NotA { element, expected } => {
                write!(f, "{element} is not {expected} in the document")
            }
        }
    }
}

impl std::error::Error for MutationError {}

impl From<TreeError> for MutationError {
    fn from(error: TreeError) -> Self {
        MutationError::Tree(error)
    }
}
