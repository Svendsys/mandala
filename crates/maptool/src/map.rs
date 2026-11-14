// SPDX-License-Identifier: MPL-2.0

//! How the verbs read a document: its nodes in the file's order, the
//! elements of a node that hold text, and the text they hold.
//!
//! Every verb reads the same [`Document`] the app does, through its read
//! API alone: [`Nodes`] walks the nodes in depth-first pre-order — the
//! order the file lists them — with each node's depth, and [`walk`]
//! visits every text one node holds — each element's spans and every
//! string field it draws as glyphs — named in the spelling a load
//! error's path uses (`sections[0]`, `links[1].ends[0].label`,
//! `frame.rings[0].fit.once.filler`), so what `grep` and `verify` print
//! points at the place in the file.

use std::fmt;

use baumhard::arena::NodeId;
use baumhard::document::element::{
    CapData, ConnectorData, DirectionalGlyphs, DocElement, DocKind, FlowStyle, FrameData, FrameStyle,
    LabelData, LabelStyle, LinkData, NodeData, NodeStyle, PathData, PortalEndData, PortalEndStyle, RingData,
    RingLayoutStyle, RingStyle, SectionData, SpanData, TextStyle, Triggers,
};
use baumhard::document::ids::Document;
use baumhard::document::values::{FitMode, LinkKind, LinkTarget, Sides};

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

/// An element of a node that holds text, named as a load error's path
/// names it below the node.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Place {
    /// `sections[n]`.
    Section(usize),
    /// `frame`: the frame itself, which holds strings but no spans.
    Frame,
    /// `frame.rings[n]`.
    Ring(usize),
    /// A connector or a line link.
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
            Place::Frame => f.write_str("frame"),
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

/// A string field of an element, drawn as glyphs, named as the saved map
/// spells its key below the element.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Field {
    /// `.content`: what a path or portal end draws with no spans.
    Content,
    /// `.fit.once.filler`: what a `once` fit repeats after the text.
    Filler,
    /// `.sides`, or `.sides.content` when the ornament has a color of its
    /// own. Named as the saved map spells it: the saver writes an
    /// ornament with no color as the bare string, whatever the file it
    /// was read from said (the document keeps no spelling).
    Sides {
        /// Whether it has a color, and so is saved as an object.
        colored: bool,
    },
    /// `.vertical`: a repeating ring's text where its outline runs steep.
    Vertical,
    /// `.corners[n]`: a repeating ring's text at a corner.
    Corner(usize),
}

impl fmt::Display for Field {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Field::Content => f.write_str(".content"),
            Field::Filler => f.write_str(".fit.once.filler"),
            Field::Sides { colored: false } => f.write_str(".sides"),
            Field::Sides { colored: true } => f.write_str(".sides.content"),
            Field::Vertical => f.write_str(".vertical"),
            Field::Corner(n) => write!(f, ".corners[{n}]"),
        }
    }
}

/// What the walk found at a [`Place`].
#[derive(Copy, Clone, Debug)]
pub enum Found<'d> {
    /// The element's spans — its text, concatenated — held by `owner`.
    Spans(NodeId),
    /// One of its own string fields, and the string.
    Field(Field, &'d str),
}

/// Call `f` with every text `node` holds, in the order the saved file
/// lists it: each element's string fields in key order and its spans,
/// for the sections, the frame and its rings, the connector and each
/// link with their caps and labels, and each portal end and its label.
/// The node's child nodes are not visited.
///
/// **Every string field is named here.** Each element's data, and each
/// style in it, is destructured without `..`, so a field the format
/// gains fails to compile in this walk until it is either visited or
/// listed as not drawn — which is what makes "every string a map draws
/// as glyphs" a fact `grep` can rely on (amendment E1.1, third
/// addendum §2). Strings not drawn: a span's `url` link. Strings of
/// the map's defaults are not the element's own and are not visited.
///
/// Cost: O(the node's own elements); no allocation.
pub fn walk<'d>(doc: &'d Document, node: NodeId, mut f: impl FnMut(Place, Found<'d>)) {
    if let Some(DocElement::Node(NodeData {
        ident: _,
        channel: _,
        position: _,
        min_size: _,
        style: NodeStyle { shape: _, text },
        palette: _,
        zoom: _,
        folded: _,
        hidden: _,
        opacity: _,
        on: Triggers { click: _, hover: _ },
    })) = doc.get(node)
    {
        no_strings(text);
    }
    let mut section = 0;
    let mut link = 0;
    for id in doc.children(node) {
        match doc.get(id) {
            Some(DocElement::Section(SectionData {
                channel: _,
                offset: _,
                min_size: _,
                align: _,
                text,
            })) => {
                no_strings(text);
                spans(doc, id, Place::Section(section), &mut f);
                section += 1;
            }
            Some(DocElement::Frame(FrameData {
                style:
                    FrameStyle {
                        visible: _,
                        start: _,
                        direction: _,
                        fit,
                        gap: _,
                        inset: _,
                        ring_layout,
                        text,
                    },
            })) => {
                no_strings(text);
                filler(fit, Place::Frame, &mut f);
                sides(ring_layout, Place::Frame, &mut f);
                let rings = doc.children(id).filter(|&ring| is(doc, ring, DocKind::Ring));
                for (n, ring) in rings.enumerate() {
                    if let Some(DocElement::Ring(data)) = doc.get(ring) {
                        walk_ring(doc, ring, data, n, &mut f);
                    }
                }
            }
            Some(DocElement::Connector(ConnectorData { path })) => {
                walk_path(doc, id, path, PathAt::Connector, &mut f);
            }
            Some(DocElement::Link(LinkData { to: _, kind, path })) => {
                match kind {
                    LinkKind::Line => walk_path(doc, id, path, PathAt::Link(link), &mut f),
                    // A portal link takes no path keys (the loader refuses
                    // them), so its `path` holds nothing drawn; its ends do.
                    LinkKind::Portal => walk_portal(doc, id, link, &mut f),
                }
                link += 1;
            }
            // A child node is walked by `Nodes`; the other kinds never sit
            // directly under a node. Named, so a new kind fails here.
            Some(
                DocElement::Node(_)
                | DocElement::Map(_)
                | DocElement::Theme(_)
                | DocElement::Palette(_)
                | DocElement::Defaults(_)
                | DocElement::Span(_)
                | DocElement::Ring(_)
                | DocElement::CapStart(_)
                | DocElement::CapEnd(_)
                | DocElement::Label(_)
                | DocElement::PortalEnd(_),
            )
            | None => {}
        }
    }
}

/// A text style holds no string: a face is a typed font, a color a typed
/// reference. Destructured so a string added to it fails to compile here.
fn no_strings(style: &TextStyle) {
    let TextStyle {
        font: _,
        size: _,
        color: _,
        weight: _,
        italic: _,
    } = style;
}

/// `owner`'s spans, and each span's fields: its text is drawn, through
/// its owner's concatenated text; its `url` link is not.
fn spans<'d>(doc: &'d Document, owner: NodeId, at: Place, f: &mut impl FnMut(Place, Found<'d>)) {
    for id in doc.children(owner) {
        if let Some(DocElement::Span(SpanData { text: _, style, link })) = doc.get(id) {
            no_strings(style);
            match link {
                Some(LinkTarget::Url(_)) | Some(LinkTarget::Verb(_)) | None => {}
            }
        }
    }
    f(at, Found::Spans(owner));
}

/// A `once` fit's filler, drawn after the text.
fn filler<'d>(fit: &'d Option<FitMode>, at: Place, f: &mut impl FnMut(Place, Found<'d>)) {
    match fit {
        Some(FitMode::Once { filler: Some(text) }) => f(at, Found::Field(Field::Filler, text)),
        Some(
            FitMode::Once { filler: None }
            | FitMode::Repeat
            | FitMode::Spread
            | FitMode::Fit { min: _, max: _ },
        )
        | None => {}
    }
}

/// A ring layout's `sides` ornament.
fn sides<'d>(layout: &'d RingLayoutStyle, at: Place, f: &mut impl FnMut(Place, Found<'d>)) {
    let RingLayoutStyle {
        span: _,
        sides,
        spacing: _,
    } = layout;
    match sides {
        Some(Sides::On { content, color }) => f(
            at,
            Found::Field(
                Field::Sides {
                    colored: color.is_some(),
                },
                content,
            ),
        ),
        Some(Sides::Off) | None => {}
    }
}

/// A ring: its filler and sides, its spans, then its directional texts.
fn walk_ring<'d>(
    doc: &'d Document,
    ring: NodeId,
    data: &'d RingData,
    n: usize,
    f: &mut impl FnMut(Place, Found<'d>),
) {
    let RingData {
        channel: _,
        style: RingStyle {
            fit,
            ring_layout,
            text,
        },
        directional,
    } = data;
    let at = Place::Ring(n);
    no_strings(text);
    filler(fit, at, f);
    sides(ring_layout, at, f);
    spans(doc, ring, at, f);
    if let Some(glyphs) = directional {
        let DirectionalGlyphs { vertical, corners } = &**glyphs;
        if let Some(vertical) = vertical {
            f(at, Found::Field(Field::Vertical, vertical));
        }
        for (i, corner) in corners.iter().flatten().enumerate() {
            f(at, Found::Field(Field::Corner(i), corner));
        }
    }
}

/// A connector or line link: its filler and content, its spans, then its
/// caps and label.
fn walk_path<'d>(
    doc: &'d Document,
    id: NodeId,
    path: &'d PathData,
    at: PathAt,
    f: &mut impl FnMut(Place, Found<'d>),
) {
    let PathData {
        flow:
            FlowStyle {
                fit,
                spacing: _,
                size: _,
                color: _,
                content,
            },
        anchors: _,
        control_points: _,
        straight: _,
        visible: _,
        zoom: _,
    } = path;
    filler(fit, Place::Path(at), f);
    if let Some(content) = content {
        f(Place::Path(at), Found::Field(Field::Content, content));
    }
    spans(doc, id, Place::Path(at), f);
    for child in doc.children(id) {
        match doc.get(child) {
            Some(DocElement::CapStart(CapData { text })) => {
                no_strings(text);
                spans(doc, child, Place::CapStart(at), f);
            }
            Some(DocElement::CapEnd(CapData { text })) => {
                no_strings(text);
                spans(doc, child, Place::CapEnd(at), f);
            }
            Some(DocElement::Label(LabelData {
                style:
                    LabelStyle {
                        t: _,
                        offset: _,
                        text,
                    },
            })) => {
                no_strings(text);
                spans(doc, child, Place::Label(at), f);
            }
            // The path's own spans were visited above; no other kind sits
            // under a path. Named, so a new kind fails here.
            Some(
                DocElement::Span(_)
                | DocElement::Node(_)
                | DocElement::Map(_)
                | DocElement::Theme(_)
                | DocElement::Palette(_)
                | DocElement::Defaults(_)
                | DocElement::Section(_)
                | DocElement::Frame(_)
                | DocElement::Ring(_)
                | DocElement::Connector(_)
                | DocElement::Link(_)
                | DocElement::PortalEnd(_),
            )
            | None => {}
        }
    }
}

/// A portal link's ends, each its content, its spans and its label.
fn walk_portal<'d>(doc: &'d Document, portal: NodeId, link: usize, f: &mut impl FnMut(Place, Found<'d>)) {
    for id in doc.children(portal) {
        let Some(DocElement::PortalEnd(PortalEndData {
            end,
            style:
                PortalEndStyle {
                    border_t: _,
                    offset: _,
                    color: _,
                    size: _,
                    content,
                },
        })) = doc.get(id)
        else {
            continue;
        };
        let end = *end;
        let at = Place::PortalEnd { link, end };
        if let Some(content) = content {
            f(at, Found::Field(Field::Content, content));
        }
        spans(doc, id, at, f);
        for label in doc.children(id) {
            if let Some(DocElement::Label(LabelData {
                style:
                    LabelStyle {
                        t: _,
                        offset: _,
                        text,
                    },
            })) = doc.get(label)
            {
                no_strings(text);
                spans(doc, label, Place::PortalEndLabel { link, end }, f);
            }
        }
    }
}

/// Call `f` with every element of `node` that holds spans, in the order
/// the file lists them ([`walk`]'s). O(the node's own elements); no
/// allocation.
pub fn text_owners(doc: &Document, node: NodeId, mut f: impl FnMut(Place, NodeId)) {
    walk(doc, node, |place, found| {
        if let Found::Spans(owner) = found {
            f(place, owner);
        }
    });
}
