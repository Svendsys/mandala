// SPDX-License-Identifier: MPL-2.0

//! Identity, and the [`Document`](crate::document::ids::Document)
//! that owns the tree.
//!
//! **In memory, an element is its arena `NodeId`** (D3). Selection,
//! journals, links and projection all hold `NodeId`s, and nothing
//! hashes a string to find an element.
//!
//! **From outside, a node is its
//! [`NodeIdent`](crate::document::ids::NodeIdent)**: a small integer the
//! file stores so links and verbs can name a node, and the console, IPC
//! and `mandalactl` can too. One index maps it to the `NodeId`, and
//! only
//! [`Document::node_by_ident`](crate::document::ids::Document::node_by_ident)
//! reads that index — it is for
//! callers outside the document (the loader, the console, IPC), never
//! for a document-internal path, which already holds `NodeId`s.
//!
//! **The id rule** (design §5.2): when a document is built the counter
//! starts one past the highest ident in it; while it is open the
//! counter only increases, so an ident is never issued twice. The
//! counter is not saved, so after a save and a reload an ident deleted
//! at the top of the range may be issued again. Ident `0` is reserved
//! as "not yet assigned": a node a mutation inserts carries it until
//! the writer (V1-B4) assigns the next ident.
//!
//! There is no `&mut` access to a `Document` here: the document changes
//! only through its writer (V1-B4), and is built only by the loader
//! (V1-B2) and, for tests,
//! `DocumentBuilder`, crate-private in `document::tests::test_support`.

use crate::document::bounds::{self, BoundError};
use crate::document::element::{DefaultsData, DocElement, DocKind, PaletteData, ThemeData};
use crate::document::shape::{validate_shape, ShapeError};
use crate::document::values::PaletteId;
use crate::engine::tree::Tree;
use indextree::NodeId;
use rustc_hash::FxHashMap;

/// A node's external identity: unique within its document, never
/// reused while the document is open. `0` is reserved for "not yet
/// assigned" ([`NodeIdent::UNASSIGNED`]).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct NodeIdent(pub u32);

impl NodeIdent {
    /// The ident of a node no ident has been assigned to yet.
    pub const UNASSIGNED: NodeIdent = NodeIdent(0);
}

/// An edit the map defines (§10.1). Only its identity for now; V1-D1
/// completes it.
#[derive(Clone, Debug, PartialEq)]
pub struct EditDef {
    /// The id the file and the verbs name it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
}

/// A presentation the map defines (§10.2). Only its identity for now;
/// V1-D2 completes it.
#[derive(Clone, Debug, PartialEq)]
pub struct PresentationDef {
    /// The id the file and the verbs name it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
}

/// A verb the map defines (§11.1). Only its identity for now; V1-D3
/// completes it.
#[derive(Clone, Debug, PartialEq)]
pub struct VerbDef {
    /// The id the file and the triggers name it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
}

/// What a document carries beside its tree: definitions that are
/// loaded, validated, saved and invoked, but not changed by any V1 code
/// path (§5.7), so they are not tree elements.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Definitions {
    /// The edits. An [`EditId`](crate::document::values::EditId) indexes
    /// this list.
    pub edits: Vec<EditDef>,
    /// The presentations. A
    /// [`PresentationId`](crate::document::values::PresentationId)
    /// indexes this list.
    pub presentations: Vec<PresentationDef>,
    /// The verbs. A [`VerbId`](crate::document::values::VerbId) indexes
    /// this list.
    pub verbs: Vec<VerbDef>,
}

/// Why a tree could not become a [`Document`].
#[derive(Clone, Debug, PartialEq)]
pub enum DocumentError {
    /// The map row breaks the shape rules — the root is not a map, or
    /// it lacks its theme or defaults.
    Shape(ShapeError),
    /// Two nodes carry the same ident.
    DuplicateIdent(NodeIdent),
    /// A node carries [`NodeIdent::UNASSIGNED`].
    UnassignedIdent(NodeId),
    /// The highest ident is
    /// [`MAX_NODE_IDENT`](crate::document::bounds::MAX_NODE_IDENT): the
    /// counter has no ident left to issue, and issuing one would reuse
    /// an ident.
    IdentSpaceExhausted,
    /// A number is out of its bound: a node ident, or a count an index
    /// type cannot address — palettes, theme variables or variants,
    /// verbs, edits or presentations.
    Bound {
        /// What was counted or read.
        what: &'static str,
        /// The bound it broke.
        error: BoundError,
    },
}

impl std::fmt::Display for DocumentError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DocumentError::Shape(error) => write!(f, "{error}"),
            DocumentError::DuplicateIdent(ident) => write!(f, "two nodes carry the id {}", ident.0),
            DocumentError::UnassignedIdent(node) => write!(f, "node {node} has no id"),
            DocumentError::IdentSpaceExhausted => f.write_str("no node id is left to issue"),
            DocumentError::Bound { what, error } => write!(f, "{what}: {error}"),
        }
    }
}

impl std::error::Error for DocumentError {}

/// A map: a `Tree<DocElement>` whose root is the
/// [`Map`](DocKind::Map), its ident index and counter, and the
/// definitions it carries.
#[derive(Clone, Debug)]
pub struct Document {
    tree: Tree<DocElement>,
    ident_index: FxHashMap<NodeIdent, NodeId>,
    next_ident: u32,
    definitions: Definitions,
}

impl Document {
    /// Make `tree` a document: check the map row
    /// ([`validate_shape`] at the root, so the theme and the defaults
    /// are there), check every index bound (`bounds::MAX_PALETTES` and
    /// its siblings), index every node's ident, and start the ident
    /// counter one past the highest ident (at least `1`). A tree whose
    /// counter would have no ident left to issue is refused
    /// ([`DocumentError::IdentSpaceExhausted`]), so the counter never
    /// issues an ident twice.
    ///
    /// Only the root row is checked; whoever built the tree checks the
    /// rest — the loader calls [`validate_shape`] for every element as it
    /// builds. Crate-private: a document is built by the loader or the
    /// test builder, never assembled by the app.
    ///
    /// Cost: one pass over the tree, one hash insert per node.
    pub(crate) fn from_tree(tree: Tree<DocElement>, definitions: Definitions) -> Result<Self, DocumentError> {
        validate_shape(&tree, tree.root()).map_err(DocumentError::Shape)?;
        check_indexes(&tree, &definitions)?;
        let mut ident_index = FxHashMap::default();
        let mut highest = 0;
        for id in tree.descendants() {
            let Some(node) = tree.arena()[id].get().as_node() else {
                continue;
            };
            if node.ident == NodeIdent::UNASSIGNED {
                return Err(DocumentError::UnassignedIdent(id));
            }
            bounds::check_node_ident(node.ident.0).map_err(|error| DocumentError::Bound {
                what: "node id",
                error,
            })?;
            if ident_index.insert(node.ident, id).is_some() {
                return Err(DocumentError::DuplicateIdent(node.ident));
            }
            highest = highest.max(node.ident.0);
        }
        // `highest <= MAX_NODE_IDENT < u32::MAX`, so this cannot wrap.
        let next_ident = highest + 1;
        if next_ident > bounds::MAX_NODE_IDENT {
            return Err(DocumentError::IdentSpaceExhausted);
        }
        Ok(Document {
            tree,
            ident_index,
            next_ident,
            definitions,
        })
    }

    /// The tree, read-only. The resolvers and the shape checks work
    /// from it. O(1).
    pub fn tree(&self) -> &Tree<DocElement> {
        &self.tree
    }

    /// The root: the [`Map`](DocKind::Map). O(1).
    pub fn root(&self) -> NodeId {
        self.tree.root()
    }

    /// The node an external caller names by `ident` — the loader
    /// resolving a link, the console, IPC. Not for document-internal
    /// code, which holds `NodeId`s. O(1).
    pub fn node_by_ident(&self, ident: NodeIdent) -> Option<NodeId> {
        self.ident_index.get(&ident).copied()
    }

    /// The ident the next inserted node will be given. O(1).
    pub fn next_ident(&self) -> NodeIdent {
        NodeIdent(self.next_ident)
    }

    /// The element at `id`, or `None` if `id` is not in the tree. O(1).
    pub fn get(&self, id: NodeId) -> Option<&DocElement> {
        get(&self.tree, id)
    }

    /// `id`'s children, in sibling order (kind, then channel). Empty for
    /// an id not in the tree. No allocation.
    pub fn children(&self, id: NodeId) -> impl Iterator<Item = NodeId> + '_ {
        let id = self.tree.get(id).map(|_| id);
        id.into_iter().flat_map(|id| id.children(self.tree.arena()))
    }

    /// `id`'s parent; `None` for the root and for an id not in the tree.
    /// O(1).
    pub fn parent(&self, id: NodeId) -> Option<NodeId> {
        self.tree.get(id)?.parent()
    }

    /// How many [`Node`](DocKind::Node) elements are above `id`: `0`
    /// for a root node and for everything outside the nodes, `1` for a
    /// root node's children, and so on. O(depth).
    pub fn depth(&self, id: NodeId) -> usize {
        depth(&self.tree, id)
    }

    /// The nearest [`Node`](DocKind::Node) at or above `id`: the node
    /// itself, or the node a section, span, frame, path or end belongs
    /// to. `None` for the map, theme, palettes and defaults. O(depth
    /// within the node).
    pub fn owning_node(&self, id: NodeId) -> Option<NodeId> {
        owning_node(&self.tree, id)
    }

    /// The theme. O(1).
    pub fn theme(&self) -> &ThemeData {
        // `from_tree` checked the map row, so the theme is there.
        theme(&self.tree).map_or(&EMPTY_THEME, |(_, theme)| theme)
    }

    /// The map's defaults. O(palettes): the defaults follow the
    /// palettes in the map row.
    pub fn defaults(&self) -> &DefaultsData {
        // `from_tree` checked the map row, so the defaults are there.
        defaults(&self.tree).map_or(&EMPTY_DEFAULTS, |(_, defaults)| defaults)
    }

    /// The palettes, in tree order: the `n`th is `PaletteId(n)`. No
    /// allocation.
    pub fn palettes(&self) -> impl Iterator<Item = (PaletteId, &PaletteData)> + '_ {
        palettes(&self.tree)
    }

    /// The definitions the map carries. O(1).
    pub fn definitions(&self) -> &Definitions {
        &self.definitions
    }

    /// [`validate_shape`] for the element at `id`: its own row, and a
    /// node's connector rule. O(children of `id`).
    pub fn validate_shape(&self, id: NodeId) -> Result<(), ShapeError> {
        validate_shape(&self.tree, id)
    }
}

/// What [`Document::theme`] returns if the theme were missing, which a
/// document's construction rules out. A `static` rather than a panic:
/// an accessor on an interactive path never panics.
static EMPTY_THEME: ThemeData = ThemeData {
    variables: Vec::new(),
    variants: Vec::new(),
    active: 0,
};

/// What [`Document::defaults`] returns if the defaults were missing,
/// which a document's construction rules out. See [`EMPTY_THEME`].
static EMPTY_DEFAULTS: std::sync::LazyLock<DefaultsData> = std::sync::LazyLock::new(DefaultsData::default);

/// The element at `id` in `tree`, or `None` if it is not there. O(1).
pub fn get(tree: &Tree<DocElement>, id: NodeId) -> Option<&DocElement> {
    tree.get(id).map(|node| node.get())
}

/// [`Document::depth`] over a tree. O(depth).
pub fn depth(tree: &Tree<DocElement>, id: NodeId) -> usize {
    if tree.get(id).is_none() {
        return 0;
    }
    id.ancestors(tree.arena())
        .skip(1)
        .filter(|&a| tree.arena()[a].get().kind() == DocKind::Node)
        .count()
}

/// [`Document::owning_node`] over a tree. O(depth within the node).
pub fn owning_node(tree: &Tree<DocElement>, id: NodeId) -> Option<NodeId> {
    tree.get(id)?;
    id.ancestors(tree.arena())
        .find(|&a| tree.arena()[a].get().kind() == DocKind::Node)
}

/// The theme element of `tree` and its data: the map's first child.
/// O(1).
pub fn theme(tree: &Tree<DocElement>) -> Option<(NodeId, &ThemeData)> {
    let first = tree.root().children(tree.arena()).next()?;
    match tree.arena()[first].get() {
        DocElement::Theme(theme) => Some((first, theme)),
        _ => None,
    }
}

/// The defaults element of `tree` and its data: the map's child after
/// the theme and the palettes. O(palettes).
pub fn defaults(tree: &Tree<DocElement>) -> Option<(NodeId, &DefaultsData)> {
    tree.root()
        .children(tree.arena())
        .take_while(|&c| tree.arena()[c].get().kind() <= DocKind::Defaults)
        .find_map(|c| match tree.arena()[c].get() {
            DocElement::Defaults(defaults) => Some((c, &**defaults)),
            _ => None,
        })
}

/// The palettes of `tree`, in tree order, numbered. Only the first
/// [`MAX_PALETTES`](crate::document::bounds::MAX_PALETTES) have an id:
/// past them the iteration ends rather than alias an id (a `Document`
/// refuses such a tree). No allocation.
pub fn palettes(tree: &Tree<DocElement>) -> impl Iterator<Item = (PaletteId, &PaletteData)> + '_ {
    tree.root()
        .children(tree.arena())
        .take_while(move |&c| tree.arena()[c].get().kind() <= DocKind::Palette)
        .filter_map(move |c| match tree.arena()[c].get() {
            DocElement::Palette(palette) => Some(palette),
            _ => None,
        })
        .enumerate()
        .map_while(|(n, palette)| Some((PaletteId(u16::try_from(n).ok()?), palette)))
}

/// Refuse a tree or definitions with more of something than its index
/// type addresses: past the bound an id would alias another.
fn check_indexes(tree: &Tree<DocElement>, definitions: &Definitions) -> Result<(), DocumentError> {
    let bound = |what: &'static str, count: usize, max: usize| {
        bounds::check_count(count, max).map_err(|error| DocumentError::Bound { what, error })
    };
    let palettes = tree
        .root()
        .children(tree.arena())
        .filter(|&c| tree.arena()[c].get().kind() == DocKind::Palette)
        .count();
    bound("palettes", palettes, bounds::MAX_PALETTES)?;
    if let Some((_, theme)) = theme(tree) {
        bound(
            "theme variables",
            theme.variables.len(),
            bounds::MAX_THEME_VARIABLES,
        )?;
        bound("theme variants", theme.variants.len(), bounds::MAX_THEME_VARIANTS)?;
    }
    bound("verbs", definitions.verbs.len(), bounds::MAX_VERBS)?;
    bound("edits", definitions.edits.len(), bounds::MAX_EDITS)?;
    bound(
        "presentations",
        definitions.presentations.len(),
        bounds::MAX_PRESENTATIONS,
    )?;
    Ok(())
}

/// The palette `id` names in `tree`. O(palettes).
pub fn palette(tree: &Tree<DocElement>, id: PaletteId) -> Option<&PaletteData> {
    palettes(tree).find(|(n, _)| *n == id).map(|(_, palette)| palette)
}
