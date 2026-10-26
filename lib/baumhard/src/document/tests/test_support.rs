// SPDX-License-Identifier: MPL-2.0

//! [`DocumentBuilder`]: how a test or a bench makes a document, since
//! nothing else here can — the loader is V1-B2's, and a `Document` has
//! no `&mut` API.
//!
//! The builder starts from the smallest valid map — a map, an empty
//! theme and empty defaults — and adds elements in key order, so every
//! row stays sorted. [`DocumentBuilder::build`] checks every element's
//! shape before it hands the document out; a test that needs a
//! document that breaks the shape rules builds it with
//! [`DocumentBuilder::build_unchecked`], which checks only the map row.

use crate::document::element::{
    ConnectorData, DefaultsData, DocElement, DocKind, LinkData, MapData, NodeData, NodeStyle, PaletteData,
    PathData, PortalEndData, SectionData, SpanData, ThemeData, Triggers,
};
use crate::document::ids::{Definitions, Document, DocumentError, NodeIdent};
use crate::document::shape::{validate_shape, ShapeError};
use crate::document::values::{Anchor, LinkKind, PaletteId};
use crate::engine::tree::{Placement, Tree};
use glam::Vec2;
use indextree::NodeId;

/// A node at the origin with nothing set: every style field inherits.
pub fn node_data(ident: u32) -> NodeData {
    NodeData {
        ident: NodeIdent(ident),
        channel: 0,
        position: Vec2::ZERO,
        min_size: None,
        style: NodeStyle::default(),
        palette: None,
        zoom: None,
        folded: false,
        hidden: false,
        opacity: crate::document::defaults::DEFAULT_OPACITY,
        on: Triggers::default(),
    }
}

/// A path with nothing set, attached automatically at both ends.
pub fn path_data() -> PathData {
    PathData {
        flow: Default::default(),
        anchors: (Anchor::Auto, Anchor::Auto),
        control_points: Vec::new(),
        visible: true,
        zoom: None,
    }
}

/// A span of `text` with no style of its own.
pub fn span(text: &str) -> DocElement {
    DocElement::Span(SpanData {
        text: text.to_string(),
        ..SpanData::default()
    })
}

/// Why [`DocumentBuilder::build`] refused.
#[derive(Clone, Debug, PartialEq)]
pub enum BuildFailure {
    /// An element breaks the shape rules.
    Shape(ShapeError),
    /// The tree cannot be a document.
    Document(DocumentError),
}

/// Builds a document element by element. See the module docs.
pub struct DocumentBuilder {
    tree: Tree<DocElement>,
    theme: NodeId,
    defaults: NodeId,
    palettes: u16,
    next_ident: u32,
}

impl Default for DocumentBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl DocumentBuilder {
    /// A map named `"test"` with an empty theme and empty defaults.
    pub fn new() -> Self {
        let mut tree = Tree::new_non_indexed_with(DocElement::Map(MapData {
            name: "test".to_string(),
        }));
        let root = tree.root();
        let theme = tree
            .insert_child(root, DocElement::Theme(ThemeData::default()), Placement::Last)
            .unwrap();
        let defaults = tree
            .insert_child(root, DocElement::Defaults(Box::default()), Placement::Last)
            .unwrap();
        DocumentBuilder {
            tree,
            theme,
            defaults,
            palettes: 0,
            next_ident: 1,
        }
    }

    /// The map.
    pub fn root(&self) -> NodeId {
        self.tree.root()
    }

    /// The theme element.
    pub fn theme_id(&self) -> NodeId {
        self.theme
    }

    /// The defaults element.
    pub fn defaults_id(&self) -> NodeId {
        self.defaults
    }

    /// Replace the theme's data.
    pub fn theme(&mut self, theme: ThemeData) -> &mut Self {
        self.set(self.theme, DocElement::Theme(theme));
        self
    }

    /// Replace the defaults' data.
    pub fn defaults(&mut self, defaults: DefaultsData) -> &mut Self {
        self.set(self.defaults, DocElement::Defaults(Box::new(defaults)));
        self
    }

    /// Add a palette after the others; returns its id.
    pub fn palette(&mut self, palette: PaletteData) -> PaletteId {
        let root = self.root();
        self.add(root, DocElement::Palette(palette));
        let id = PaletteId(self.palettes);
        self.palettes += 1;
        id
    }

    /// Add `element` under `parent`, at the end of the run of its key.
    pub fn add(&mut self, parent: NodeId, element: DocElement) -> NodeId {
        self.tree.insert_child(parent, element, Placement::Last).unwrap()
    }

    /// Replace the element at `id`.
    pub fn set(&mut self, id: NodeId, element: DocElement) {
        self.tree.update(id, |e| *e = element).unwrap();
    }

    /// Change the element at `id` with `f`.
    pub fn edit(&mut self, id: NodeId, f: impl FnOnce(&mut DocElement)) -> &mut Self {
        self.tree.update(id, f).unwrap();
        self
    }

    /// Change the node at `id` with `f`.
    pub fn edit_node(&mut self, id: NodeId, f: impl FnOnce(&mut NodeData)) -> &mut Self {
        self.edit(id, |e| match e {
            DocElement::Node(node) => f(node),
            other => panic!("edit_node on a {:?}", other.kind()),
        })
    }

    /// Add a node with the next ident under `parent`; under a node it
    /// gets a connector, as the shape rules require. Returns the node.
    pub fn node(&mut self, parent: NodeId) -> NodeId {
        let ident = self.next_ident;
        self.next_ident += 1;
        self.node_with(parent, node_data(ident))
    }

    /// Add `data` as a node under `parent`; under a node it gets a
    /// connector. Returns the node.
    pub fn node_with(&mut self, parent: NodeId, data: NodeData) -> NodeId {
        self.next_ident = self.next_ident.max(data.ident.0.saturating_add(1));
        let under_node = self.tree.arena()[parent].get().kind() == DocKind::Node;
        let node = self.add(parent, DocElement::Node(data));
        if under_node {
            self.connector(node);
        }
        node
    }

    /// Add a connector with nothing set under `node`. Returns it.
    pub fn connector(&mut self, node: NodeId) -> NodeId {
        self.add(node, DocElement::Connector(ConnectorData { path: path_data() }))
    }

    /// Add a section on `channel` with nothing set under `node`.
    pub fn section(&mut self, node: NodeId, channel: usize) -> NodeId {
        self.add(
            node,
            DocElement::Section(SectionData {
                channel,
                ..SectionData::default()
            }),
        )
    }

    /// Add a span of `text` under `parent`.
    pub fn span(&mut self, parent: NodeId, text: &str) -> NodeId {
        self.add(parent, span(text))
    }

    /// Add a link of `kind` from `node` to `to`; a portal link gets its
    /// two ends. Returns the link.
    pub fn link(&mut self, node: NodeId, to: NodeId, kind: LinkKind) -> NodeId {
        let link = self.add(
            node,
            DocElement::Link(LinkData {
                to,
                kind,
                path: path_data(),
            }),
        );
        if kind == LinkKind::Portal {
            for end in 0..2 {
                self.add(
                    link,
                    DocElement::PortalEnd(PortalEndData {
                        end,
                        ..PortalEndData::default()
                    }),
                );
            }
        }
        link
    }

    /// The tree so far.
    pub fn tree(&self) -> &Tree<DocElement> {
        &self.tree
    }

    /// The document, once every element passes [`validate_shape`].
    pub fn build(self) -> Result<Document, BuildFailure> {
        for id in self.tree.descendants() {
            validate_shape(&self.tree, id).map_err(BuildFailure::Shape)?;
        }
        Document::from_tree(self.tree, Definitions::default()).map_err(BuildFailure::Document)
    }

    /// The document, checking only the map row: for tests of the shape
    /// rules themselves.
    pub fn build_unchecked(self) -> Result<Document, DocumentError> {
        Document::from_tree(self.tree, Definitions::default())
    }
}

/// The ids of [`example`]'s elements, by role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Example {
    /// Root node, ident 1, bound to the palette.
    pub a: NodeId,
    /// `a`'s first section.
    pub a_title: NodeId,
    /// `a`'s second section.
    pub a_body: NodeId,
    /// The span in `a_title`.
    pub a_title_span: NodeId,
    /// The span in `a_body`.
    pub a_body_span: NodeId,
    /// `a`'s frame.
    pub frame: NodeId,
    /// The frame's ring.
    pub ring: NodeId,
    /// The span in the ring.
    pub ring_span: NodeId,
    /// A line link from `a` to `c`.
    pub line: NodeId,
    /// A portal link from `a` to `d`.
    pub portal: NodeId,
    /// The portal's source end.
    pub portal_source: NodeId,
    /// The portal's target end.
    pub portal_target: NodeId,
    /// `a`'s child, ident 2.
    pub b: NodeId,
    /// `b`'s section.
    pub b_section: NodeId,
    /// The span in `b_section`.
    pub b_span: NodeId,
    /// `b`'s connector.
    pub b_connector: NodeId,
    /// The span along `b`'s connector.
    pub b_connector_span: NodeId,
    /// The start cap on `b`'s connector.
    pub b_cap: NodeId,
    /// The label on `b`'s connector.
    pub b_label: NodeId,
    /// The label on the line link.
    pub line_label: NodeId,
    /// The label beside the portal's source end.
    pub portal_label: NodeId,
    /// `b`'s child, ident 3.
    pub c: NodeId,
    /// `c`'s connector, with no spans.
    pub c_connector: NodeId,
    /// A second root node, ident 4.
    pub d: NodeId,
}

/// A map with every kind in it, in a valid shape:
///
/// ```text
/// Map
/// ├─ Theme (two variables, variants "dark" and "light"), Palette "rainbow" (three groups), Defaults
/// ├─ Node a (1, palette 0)
/// │  ├─ Section 0 ─ Span        Section 1 ─ Span
/// │  ├─ Frame ─ Ring 0 ─ Span
/// │  ├─ Link line → c ─ Span, CapStart ─ Span, CapEnd ─ Span, Label ─ Span
/// │  ├─ Link portal → d ─ PortalEnd 0 ─ Span, Label ─ Span; PortalEnd 1
/// │  └─ Node b (2) ─ Section 0 ─ Span; Connector ─ Span, CapStart, CapEnd, Label ─ Span
/// │     └─ Node c (3) ─ Connector
/// └─ Node d (4)
/// ```
///
/// Nothing sets a style field. Returns the builder, so a test can set
/// some before building, and the ids. Every call builds the same tree
/// in the same order, and the arena hands out slots in insertion
/// order, so every call returns the same ids.
pub fn example() -> (DocumentBuilder, Example) {
    use crate::document::element::{CapData, LabelData, ThemeVariant};
    use crate::document::values::Rgba;

    let mut builder = DocumentBuilder::new();
    builder.theme(ThemeData {
        variables: vec!["accent".to_string(), "fg".to_string()],
        variants: vec![
            ThemeVariant {
                name: "dark".to_string(),
                values: vec![Rgba::opaque(0xd4, 0xa2, 0x4c), Rgba::opaque(0xe0, 0xe0, 0xe0)],
            },
            ThemeVariant {
                name: "light".to_string(),
                values: vec![Rgba::opaque(0x8a, 0x5a, 0x00), Rgba::opaque(0x20, 0x20, 0x24)],
            },
        ],
        active: 0,
    });
    let rainbow = builder.palette(PaletteData {
        name: "rainbow".to_string(),
        groups: vec![palette_group(10), palette_group(11), palette_group(12)],
    });
    let root = builder.root();

    let a = builder.node(root);
    builder.edit_node(a, |n| n.palette = Some(rainbow));
    let a_title = builder.section(a, 0);
    let a_title_span = builder.span(a_title, "title");
    let a_body = builder.section(a, 1);
    let a_body_span = builder.span(a_body, "body");
    let frame = builder.add(a, DocElement::Frame(Default::default()));
    let ring = builder.add(frame, DocElement::Ring(Default::default()));
    let ring_span = builder.span(ring, "and the earth was without form");

    let b = builder.node(a);
    let b_connector = connector_of(&builder, b);
    let b_section = builder.section(b, 0);
    let b_span = builder.span(b_section, "child");
    let b_connector_span = builder.span(b_connector, "\u{2500}");
    let b_cap = builder.add(b_connector, DocElement::CapStart(CapData::default()));
    builder.add(b_connector, DocElement::CapEnd(CapData::default()));
    let b_label = builder.add(b_connector, DocElement::Label(LabelData::default()));
    builder.span(b_label, "label");

    let c = builder.node(b);
    let c_connector = connector_of(&builder, c);
    let d = builder.node(root);

    let line = builder.link(a, c, LinkKind::Line);
    builder.span(line, "~");
    let cap = builder.add(line, DocElement::CapStart(CapData::default()));
    builder.span(cap, "<");
    let cap = builder.add(line, DocElement::CapEnd(CapData::default()));
    builder.span(cap, ">");
    let line_label = builder.add(line, DocElement::Label(LabelData::default()));
    builder.span(line_label, "see also");

    let portal = builder.link(a, d, LinkKind::Portal);
    let mut ends = portal.children(builder.tree().arena());
    let portal_source = ends.next().unwrap();
    let portal_target = ends.next().unwrap();
    builder.span(portal_source, "\u{25C6}");
    let portal_label = builder.add(portal_source, DocElement::Label(LabelData::default()));
    builder.span(portal_label, "to d");

    let ids = Example {
        a,
        a_title,
        a_body,
        a_title_span,
        a_body_span,
        frame,
        ring,
        ring_span,
        line,
        portal,
        portal_source,
        portal_target,
        b,
        b_section,
        b_span,
        b_connector,
        b_connector_span,
        b_cap,
        b_label,
        line_label,
        portal_label,
        c,
        c_connector,
        d,
    };
    (builder, ids)
}

/// The palette group of depth `n` in [`example`]'s palette: every
/// color distinct, and each one naming its group and field — fill
/// `(n, 0, 0)`, frame `(0, n, 0)`, text `(0, 0, n)`, title `(n, n, 0)`.
pub fn palette_group(n: u8) -> crate::document::element::PaletteGroup {
    use crate::document::values::{PlainColor, Rgba};
    crate::document::element::PaletteGroup {
        fill: PlainColor::Rgba(Rgba::opaque(n, 0, 0)),
        frame: PlainColor::Rgba(Rgba::opaque(0, n, 0)),
        text: PlainColor::Rgba(Rgba::opaque(0, 0, n)),
        title: Some(PlainColor::Rgba(Rgba::opaque(n, n, 0))),
    }
}

/// The connector of `node`.
pub fn connector_of(builder: &DocumentBuilder, node: NodeId) -> NodeId {
    node.children(builder.tree().arena())
        .find(|&c| builder.tree().arena()[c].get().kind() == DocKind::Connector)
        .unwrap()
}
