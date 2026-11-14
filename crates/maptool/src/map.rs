// SPDX-License-Identifier: MPL-2.0

//! How the verbs read a document: its nodes in the file's order, the
//! elements of a node that hold text, and the text they hold.
//!
//! Every verb reads the same [`Document`] the app does, through its read
//! API alone: [`Nodes`] walks the nodes in depth-first pre-order — the
//! order the file lists them — with each node's depth, and
//! [`text_owners`] names every element of one node that holds spans, in
//! the spelling a load error's path uses (`sections[0]`,
//! `links[1].ends[0].label`), so what `grep` and `verify` print points at
//! the place in the file.

use std::fmt;

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind, LinkData};
use baumhard::document::ids::Document;
use baumhard::document::values::LinkKind;

/// The nodes of a document in depth-first pre-order — the order the file
/// lists them — each with its depth: `0` for a top-level node.
///
/// Iterative, so a tree of any depth is walked in constant stack; the
/// pending nodes are held in one `Vec` that grows to the widest frontier
/// and is never reallocated per node.
pub struct Nodes<'d> {
    doc: &'d Document,
    pending: Vec<(NodeId, usize)>,
}

impl<'d> Nodes<'d> {
    /// The walk over every node of `doc`. Cost: the root's children
    /// queued; each step queues the children of the node it yields.
    pub fn new(doc: &'d Document) -> Self {
        let mut nodes = Nodes {
            doc,
            pending: Vec::new(),
        };
        push_children(doc, doc.root(), 0, &mut nodes.pending);
        nodes
    }
}

/// Queue `parent`'s child nodes on `pending`, each with `depth`, in
/// reverse, so that popping yields them in sibling order. No allocation
/// beyond `pending`'s growth.
pub fn push_children(doc: &Document, parent: NodeId, depth: usize, pending: &mut Vec<(NodeId, usize)>) {
    let start = pending.len();
    pending.extend(
        doc.children(parent)
            .filter(|&id| is(doc, id, DocKind::Node))
            .map(|id| (id, depth)),
    );
    pending[start..].reverse();
}

impl Iterator for Nodes<'_> {
    type Item = (NodeId, usize);

    fn next(&mut self) -> Option<Self::Item> {
        let (id, depth) = self.pending.pop()?;
        push_children(self.doc, id, depth + 1, &mut self.pending);
        Some((id, depth))
    }
}

/// Whether `id` is an element of `kind`. O(1).
pub fn is(doc: &Document, id: NodeId, kind: DocKind) -> bool {
    doc.get(id).map(DocElement::kind) == Some(kind)
}

/// The id the file gives the node `id`, or `0` for an element that is
/// not a node (which no caller passes). O(1).
pub fn ident(doc: &Document, id: NodeId) -> u32 {
    doc.get(id)
        .and_then(DocElement::as_node)
        .map_or(0, |node| node.ident.0)
}

/// The link `id`, if it is one. O(1).
pub fn link(doc: &Document, id: NodeId) -> Option<&LinkData> {
    match doc.get(id) {
        Some(DocElement::Link(link)) => Some(link),
        _ => None,
    }
}

/// Replace `out` with the text of `owner`: its spans' text, concatenated
/// in order. Cost: O(its children and their text); reuses `out`'s
/// allocation.
pub fn text_into(doc: &Document, owner: NodeId, out: &mut String) {
    out.clear();
    for id in doc.children(owner) {
        if let Some(DocElement::Span(span)) = doc.get(id) {
            out.push_str(&span.text);
        }
    }
}

/// The lines `text` is matched as: [`str::lines`], except that a text
/// with nothing in it is one empty line, so `''` and `^$` find an empty
/// section. No allocation.
pub fn lines(text: &str) -> impl Iterator<Item = &str> {
    let empty = text.is_empty().then_some("");
    text.lines().chain(empty)
}

/// The `content` of its own that `owner` — a connector, a line link or a
/// portal end — sets: the text drawn when it has no spans. `None` for an
/// element that sets none, and for every other element. O(1).
pub fn own_content(doc: &Document, owner: NodeId) -> Option<&str> {
    match doc.get(owner) {
        Some(DocElement::PortalEnd(end)) => end.style.content.as_deref(),
        Some(element) => element.as_path().and_then(|path| path.flow.content.as_deref()),
        None => None,
    }
}

/// A node's sections, in their order in the file. No allocation.
pub fn sections(doc: &Document, node: NodeId) -> impl Iterator<Item = NodeId> + '_ {
    doc.children(node)
        .filter(move |&id| is(doc, id, DocKind::Section))
}

/// Which path of a node: its connector, or its `n`th link.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PathAt {
    /// The line to the node's parent.
    Connector,
    /// `links[n]`.
    Link(usize),
}

impl fmt::Display for PathAt {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathAt::Connector => f.write_str("connector"),
            PathAt::Link(n) => write!(f, "links[{n}]"),
        }
    }
}

/// An element of a node that holds spans, named as a load error's path
/// names it below the node.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// `sections[n]`.
    Section(usize),
    /// `frame.rings[n]`.
    Ring(usize),
    /// A connector's or a line link's own spans.
    Path(PathAt),
    /// `<path>.cap_start`.
    CapStart(PathAt),
    /// `<path>.cap_end`.
    CapEnd(PathAt),
    /// `<path>.label`.
    Label(PathAt),
    /// `links[link].ends[end]`.
    PortalEnd {
        /// The link's place among the node's links.
        link: usize,
        /// `0` for the source end, `1` for the target end.
        end: u8,
    },
    /// `links[link].ends[end].label`.
    PortalEndLabel {
        /// The link's place among the node's links.
        link: usize,
        /// `0` for the source end, `1` for the target end.
        end: u8,
    },
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Place::Section(n) => write!(f, "sections[{n}]"),
            Place::Ring(n) => write!(f, "frame.rings[{n}]"),
            Place::Path(path) => write!(f, "{path}"),
            Place::CapStart(path) => write!(f, "{path}.cap_start"),
            Place::CapEnd(path) => write!(f, "{path}.cap_end"),
            Place::Label(path) => write!(f, "{path}.label"),
            Place::PortalEnd { link, end } => write!(f, "links[{link}].ends[{end}]"),
            Place::PortalEndLabel { link, end } => write!(f, "links[{link}].ends[{end}].label"),
        }
    }
}

/// Call `f` with every element of `node` that holds spans, in the order
/// the file lists them: the sections, the frame's rings, the connector
/// and its caps and label, then each link — a line link's spans, caps and
/// label, or a portal link's two ends and their labels. The node's child
/// nodes are not visited. O(the node's own elements); no allocation.
pub fn text_owners(doc: &Document, node: NodeId, mut f: impl FnMut(Place, NodeId)) {
    let mut section = 0;
    let mut link = 0;
    for id in doc.children(node) {
        match doc.get(id).map(DocElement::kind) {
            Some(DocKind::Section) => {
                f(Place::Section(section), id);
                section += 1;
            }
            Some(DocKind::Frame) => {
                let rings = doc.children(id).filter(|&ring| is(doc, ring, DocKind::Ring));
                for (n, ring) in rings.enumerate() {
                    f(Place::Ring(n), ring);
                }
            }
            Some(DocKind::Connector) => path_owners(doc, PathAt::Connector, id, &mut f),
            Some(DocKind::Link) => {
                if link_kind(doc, id) == Some(LinkKind::Portal) {
                    portal_owners(doc, link, id, &mut f);
                } else {
                    path_owners(doc, PathAt::Link(link), id, &mut f);
                }
                link += 1;
            }
            // Every kind by name, so one a node may come to hold fails to
            // compile here rather than going unsearched. A child node is
            // walked by `Nodes`; the others never sit under a node.
            Some(
                DocKind::Node
                | DocKind::Map
                | DocKind::Theme
                | DocKind::Palette
                | DocKind::Defaults
                | DocKind::Span
                | DocKind::Ring
                | DocKind::CapStart
                | DocKind::CapEnd
                | DocKind::Label
                | DocKind::PortalEnd,
            )
            | None => {}
        }
    }
}

/// A path's own spans, then its caps and its label.
fn path_owners(doc: &Document, at: PathAt, path: NodeId, f: &mut impl FnMut(Place, NodeId)) {
    f(Place::Path(at), path);
    for id in doc.children(path) {
        match doc.get(id).map(DocElement::kind) {
            Some(DocKind::CapStart) => f(Place::CapStart(at), id),
            Some(DocKind::CapEnd) => f(Place::CapEnd(at), id),
            Some(DocKind::Label) => f(Place::Label(at), id),
            // The path's own spans were its `Place::Path`; every other
            // kind by name, so a new one fails to compile here.
            Some(
                DocKind::Span
                | DocKind::Node
                | DocKind::Map
                | DocKind::Theme
                | DocKind::Palette
                | DocKind::Defaults
                | DocKind::Section
                | DocKind::Frame
                | DocKind::Ring
                | DocKind::Connector
                | DocKind::Link
                | DocKind::PortalEnd,
            )
            | None => {}
        }
    }
}

/// A portal link's ends, each followed by its label.
fn portal_owners(doc: &Document, link: usize, portal: NodeId, f: &mut impl FnMut(Place, NodeId)) {
    for id in doc.children(portal) {
        let Some(DocElement::PortalEnd(end)) = doc.get(id) else {
            continue;
        };
        let end = end.end;
        f(Place::PortalEnd { link, end }, id);
        for label in doc.children(id).filter(|&l| is(doc, l, DocKind::Label)) {
            f(Place::PortalEndLabel { link, end }, label);
        }
    }
}

/// The kind of the link `id`, if it is one.
fn link_kind(doc: &Document, id: NodeId) -> Option<LinkKind> {
    link(doc, id).map(|link| link.kind)
}
