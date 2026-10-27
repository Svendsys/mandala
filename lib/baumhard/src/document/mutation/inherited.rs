// SPDX-License-Identifier: MPL-2.0

//! What a document walk carries down:
//! [`DocInherited`](crate::document::mutation::inherited::DocInherited),
//! the state a mutation reads when the value it acts on is inherited
//! rather than stored (design §4.2, ruling R10 on #237).
//!
//! "Grow the font by 2 pt" on a span with no size of its own must grow
//! the size the span **displays**, which
//! [`resolve_text`](crate::document::resolve::resolve_text) computes
//! through the span's owners, the map's `Defaults` and `BUILT_IN`. A
//! mutation sees one element and the state its parent handed down, so
//! that state carries what the resolution needs:
//!
//! - **The tails.** For every kind of text and every flow it can sit
//!   in, the size the `Defaults` and `BUILT_IN` tiers give it. They are
//!   read once, by the seed
//!   ([`inherited_seed`](crate::document::mutation::inherited::inherited_seed)), and handed down
//!   unchanged: nothing on the walk reads the tree.
//! - **The nearest size the owners set**, within the handing element's
//!   own flow: a section's or its node's for the section's spans; a
//!   ring's or its frame's; a cap's or its path's; a label's; a path's;
//!   a portal end's. It resets at every flow boundary, which is what
//!   keeps a frame's size from ever being the node's points (amendment 2
//!   on #237) and a connector's, cap's or label's size from ever being
//!   the node's text (R4 on #235). A child node never takes its parent
//!   node's size at all.
//! - **Which element handed it**, so a mutation can tell which of those
//!   applies to the kind of element it is acting on — a node hands one
//!   state to its sections, its frame and its child nodes, whose
//!   displayed sizes come from different chains.
//!
//! [`DocInherited::text_size_for`](crate::document::mutation::inherited::DocInherited::text_size_for) answers, for an element of a given
//! kind under the handing element, exactly what `resolve_text` would
//! give that element's size if its own were unset. The state is a
//! small `Copy` value on the walker's stack: nothing is allocated per
//! visited element.

use crate::document::defaults::BUILT_IN;
use crate::document::element::{DefaultsData, DocElement, DocKind};
use crate::document::ids::defaults;
use crate::document::values::FontSize;
use crate::engine::tree::Tree;
use indextree::NodeId;

/// What the element that handed a [`DocInherited`] down is, as far as
/// its children's text sizes care.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Handing {
    /// Nothing: the state a walk anchored at the map starts from.
    Outside,
    /// The map, whose text-bearing children are root nodes.
    Map,
    /// A node: its sections, frame and child nodes.
    Node,
    /// A section: its spans.
    Section,
    /// A frame: its rings.
    Frame,
    /// A ring: its spans.
    Ring,
    /// A connector or a link (the kind): its spans, caps and label.
    Path(DocKind),
    /// A cap of a connector or a link (the path's kind): its spans.
    Cap(DocKind),
    /// A label on a connector, a link or a portal end (the host's
    /// kind): its spans.
    Label(DocKind),
    /// A portal end: its spans and its label.
    PortalEnd,
    /// An element whose children hold no text of their own: a span, the
    /// theme, a palette, the defaults.
    Leaf,
}

/// The size each kind of text displays when nothing between it and the
/// `Defaults` tier sets one: the `Defaults` tiers in the order
/// [`resolve_text`](crate::document::resolve::resolve_text) reads them,
/// then `BUILT_IN`. Read once per seed.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Tails {
    /// A node: `Defaults.node.text`.
    node: FontSize,
    /// A section: `Defaults.section`, `Defaults.node.text`.
    section: FontSize,
    /// A span in a section: `Defaults.span`, then as a section.
    section_span: FontSize,
    /// A frame: `Defaults.frame`.
    frame: FontSize,
    /// A ring, and a span in one: `Defaults.ring`, `Defaults.frame`.
    ring: FontSize,
    /// A connector's spans and caps: `Defaults.connector`.
    connector: FontSize,
    /// A line link's spans and caps: `Defaults.link`.
    link: FontSize,
    /// A label on a connector: `Defaults.label`.
    label_on_connector: FontSize,
    /// A label on a line link: `Defaults.label`.
    label_on_link: FontSize,
    /// A label beside a portal end: `Defaults.label`.
    label_on_portal_end: FontSize,
    /// A portal end's spans: `Defaults.portal_end`.
    portal_end: FontSize,
}

impl Tails {
    /// The tails of a map whose defaults are `defaults` (none for a tree
    /// without them, which only `BUILT_IN` then serves). O(1).
    fn read(defaults: Option<&DefaultsData>) -> Tails {
        let d = defaults;
        let text = |pick: fn(&DefaultsData) -> Option<FontSize>| d.and_then(pick);
        let points = |pick: fn(&DefaultsData) -> Option<f32>| d.and_then(pick).map(FontSize::Pt);
        let node_text = text(|d| d.node.text.size);
        let section = text(|d| d.section.size).or(node_text);
        let frame = text(|d| d.frame.text.size);
        let label = text(|d| d.label.text.size);
        Tails {
            node: node_text.unwrap_or(BUILT_IN.section.size),
            section: section.unwrap_or(BUILT_IN.section.size),
            section_span: text(|d| d.span.size).or(section).unwrap_or(BUILT_IN.section.size),
            frame: frame.unwrap_or(BUILT_IN.ring.text.size),
            ring: text(|d| d.ring.text.size)
                .or(frame)
                .unwrap_or(BUILT_IN.ring.text.size),
            connector: points(|d| d.connector.size).unwrap_or(BUILT_IN.connector.text.size),
            link: points(|d| d.link.size).unwrap_or(BUILT_IN.link.text.size),
            label_on_connector: label.unwrap_or(BUILT_IN.label.on_connector.size),
            label_on_link: label.unwrap_or(BUILT_IN.label.on_link.size),
            label_on_portal_end: label.unwrap_or(BUILT_IN.label.on_portal_end.size),
            portal_end: points(|d| d.portal_end.size).unwrap_or(BUILT_IN.portal_end.text.size),
        }
    }

    /// The tail of a path of `kind`'s spans and caps.
    fn path(&self, kind: DocKind) -> FontSize {
        if kind == DocKind::Link {
            self.link
        } else {
            self.connector
        }
    }

    /// The tail of a label on a host of `kind`.
    fn label(&self, host: DocKind) -> FontSize {
        match host {
            DocKind::Link => self.label_on_link,
            DocKind::PortalEnd => self.label_on_portal_end,
            _ => self.label_on_connector,
        }
    }
}

/// The document's inherited state: what an element hands its children
/// on a walk. See the module docs.
///
/// A mutation receives its target's **parent's** state
/// ([`Mutation::apply_to`](crate::engine::mutation::Mutation::apply_to)).
/// Built only by [`inherited_seed`] and by
/// [`TreeElement::inherit`](crate::engine::element::TreeElement::inherit)
/// on the walk.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct DocInherited {
    /// Whether the element that handed this down is a frame or inside
    /// one: its text children's sizes are
    /// [`Rel`](FontSize::Rel), a fraction of the node's shorter side.
    pub in_frame: bool,
    /// What handed it.
    handing: Handing,
    /// The nearest size an owner within the handing element's flow sets
    /// (see the module docs); `None` when none does.
    near: Option<FontSize>,
    /// The `Defaults` / `BUILT_IN` sizes, read by the seed.
    tails: Tails,
}

impl DocInherited {
    /// The state outside the map: what a walk anchored at the map
    /// starts from. O(1) beyond reading the defaults' sizes.
    fn outside(defaults: Option<&DefaultsData>) -> DocInherited {
        DocInherited {
            in_frame: false,
            handing: Handing::Outside,
            near: None,
            tails: Tails::read(defaults),
        }
    }

    /// The size an element of `kind`, a child of the element that handed
    /// this state down, displays when its own size is unset: what
    /// [`resolve_text`](crate::document::resolve::resolve_text) gives it
    /// with its own `size` cleared. [`Rel`](FontSize::Rel) inside a
    /// frame, [`Pt`](FontSize::Pt) elsewhere, as the map stores them.
    ///
    /// A kind that holds no text of its own answers a section's size,
    /// which is what `resolve_text` falls back to for such an owner.
    /// O(1).
    pub fn text_size_for(&self, kind: DocKind) -> FontSize {
        let tails = &self.tails;
        let near_from = |handing: Handing| if self.handing == handing { self.near } else { None };
        match kind {
            DocKind::Node => tails.node,
            DocKind::Section => near_from(Handing::Node).unwrap_or(tails.section),
            DocKind::Frame => tails.frame,
            DocKind::Ring => near_from(Handing::Frame).unwrap_or(tails.ring),
            DocKind::CapStart | DocKind::CapEnd => match self.handing {
                Handing::Path(path) => self.near.unwrap_or(tails.path(path)),
                _ => tails.connector,
            },
            DocKind::Label => match self.handing {
                Handing::Path(DocKind::Link) => tails.label(DocKind::Link),
                Handing::PortalEnd => tails.label(DocKind::PortalEnd),
                _ => tails.label(DocKind::Connector),
            },
            DocKind::Span => match self.handing {
                Handing::Section => self.near.unwrap_or(tails.section_span),
                Handing::Ring => self.near.unwrap_or(tails.ring),
                Handing::Path(path) | Handing::Cap(path) => self.near.unwrap_or(tails.path(path)),
                Handing::Label(host) => self.near.unwrap_or(tails.label(host)),
                Handing::PortalEnd => self.near.unwrap_or(tails.portal_end),
                _ => tails.section_span,
            },
            _ => tails.section,
        }
    }

    /// Whether an element of `kind` under the handing element sits in a
    /// frame — the frame itself included — so its own size is
    /// [`Rel`](FontSize::Rel). O(1).
    pub fn places_in_frame(&self, kind: DocKind) -> bool {
        self.in_frame || kind == DocKind::Frame
    }
}

/// The state `element` hands its children, given the state its own
/// parent handed it: [`TreeElement::inherit`](crate::engine::element::TreeElement::inherit)
/// for the document. O(1).
pub(crate) fn inherit(element: &DocElement, parent: &DocInherited) -> DocInherited {
    let mut state = *parent;
    let parent_near = |handing: Handing| {
        if parent.handing == handing {
            parent.near
        } else {
            None
        }
    };
    let (handing, near, in_frame) = match element {
        DocElement::Map(_) => (Handing::Map, None, false),
        DocElement::Theme(_) | DocElement::Palette(_) | DocElement::Defaults(_) | DocElement::Span(_) => {
            (Handing::Leaf, None, parent.in_frame)
        }
        DocElement::Node(node) => (Handing::Node, node.style.text.size, false),
        DocElement::Section(section) => (
            Handing::Section,
            section.text.size.or(parent_near(Handing::Node)),
            false,
        ),
        DocElement::Frame(frame) => (Handing::Frame, frame.style.text.size, true),
        DocElement::Ring(ring) => (
            Handing::Ring,
            ring.style.text.size.or(parent_near(Handing::Frame)),
            true,
        ),
        DocElement::Connector(connector) => (
            Handing::Path(DocKind::Connector),
            connector.path.flow.size.map(FontSize::Pt),
            false,
        ),
        DocElement::Link(link) => (
            Handing::Path(DocKind::Link),
            link.path.flow.size.map(FontSize::Pt),
            false,
        ),
        DocElement::CapStart(cap) | DocElement::CapEnd(cap) => match parent.handing {
            Handing::Path(path) => (Handing::Cap(path), cap.text.size.or(parent.near), false),
            _ => (Handing::Cap(DocKind::Connector), cap.text.size, false),
        },
        DocElement::Label(label) => {
            let host = match parent.handing {
                Handing::Path(DocKind::Link) => DocKind::Link,
                Handing::PortalEnd => DocKind::PortalEnd,
                _ => DocKind::Connector,
            };
            (Handing::Label(host), label.style.text.size, false)
        }
        DocElement::PortalEnd(end) => (Handing::PortalEnd, end.style.size.map(FontSize::Pt), false),
    };
    state.handing = handing;
    state.near = near;
    state.in_frame = in_frame;
    state
}

/// The inherited state a walk anchored at `node` starts from: the state
/// `node`'s **parent** hands down, as the walk would have computed it
/// had it come from the map — what an anchored walk passes as
/// `seed_for` (`MutatorTree::apply_at`).
///
/// Reads the map's `Defaults` once, then folds `inherit` down from the
/// nearest node at or above the parent — a node's state depends on
/// nothing above it but the defaults — or from the map. A `node` not
/// in the tree, and the map itself, start from outside the map.
///
/// A free function over the tree (amendment 1 on #237), so the writer
/// can pass it while it holds the tree mutably for the walk.
///
/// Cost: O(depth below the nearest node) to fold, which the shape rules
/// keep at five elements at most, plus the defaults look-up, O(palettes).
/// No allocation.
pub fn inherited_seed(tree: &Tree<DocElement>, node: NodeId) -> DocInherited {
    let outside = DocInherited::outside(defaults(tree).map(|(_, d)| d));
    match tree.get(node).and_then(|n| n.parent()) {
        Some(parent) => state_of(tree, parent, &outside),
        None => outside,
    }
}

/// The state `id` hands its children: `inherit` folded down from the
/// nearest node at or above it, or from the map. Recursion as deep as
/// `id` is below that node.
fn state_of(tree: &Tree<DocElement>, id: NodeId, outside: &DocInherited) -> DocInherited {
    let arena = tree.arena();
    let element = arena[id].get();
    let parent_state = match arena[id].parent() {
        Some(parent) if element.kind() != DocKind::Node => state_of(tree, parent, outside),
        _ => *outside,
    };
    inherit(element, &parent_state)
}
