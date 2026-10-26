// SPDX-License-Identifier: MPL-2.0

//! Which resolution inputs a change of one element touches: the other
//! half of the [`Input`](crate::document::resolve::Input) contract.
//!
//! A resolver reports what it reads as narrowly as the read itself
//! ([`crate::document::resolve`]);
//! [`inputs_changed`](crate::document::deps::inputs_changed) says, for an
//! element before and after a change, which of those inputs the change
//! touched. V1-B7 joins the two: a journal entry's before and after go
//! through `inputs_changed`, each
//! [`InputKind`](crate::document::deps::InputKind) becomes the `Input`
//! of the touched element, and the projection re-resolves exactly the
//! readers of those inputs.
//!
//! **Every field of every kind is classified here, and the compiler
//! holds it to that**: each data struct is destructured with every
//! field named, so a field added to an element does not compile until
//! this module says which input it belongs to. What is classified as no
//! input is what no resolver reads — a node's position and minimum size,
//! a section's offset, a path's anchors and control points, a span's
//! text. Moving or resizing a node therefore touches no input, and
//! dirties no resolution: a drag re-places geometry and reshapes
//! nothing.

use crate::document::element::{
    CapData, ConnectorData, DocElement, FrameData, LabelData, LinkData, MapData, NodeData, PortalEndData,
    RingData, SectionData, SpanData,
};

/// One kind of resolution input a change of an element can touch. The
/// element-scoped kinds map onto the element's own
/// [`Input`](crate::document::resolve::Input); `ParentRow` onto its
/// parent's.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum InputKind {
    /// `Input::Style` of the element: a style field changed.
    Style,
    /// `Input::PaletteBinding` of the node: its `palette` changed.
    PaletteBinding,
    /// `Input::Row` of the element's **parent**: the element's key
    /// changed, so it moved within its parent's row — which section is
    /// first may have changed.
    ParentRow,
    /// `Input::Theme`: the theme changed.
    Theme,
    /// `Input::Palette` of this palette: its groups changed.
    Palette,
    /// `Input::Defaults`: the map's defaults changed.
    Defaults,
}

/// The element-scoped kinds that are set, at most three, in a fixed
/// order.
fn kinds(style: bool, binding: bool, row: bool) -> impl Iterator<Item = InputKind> {
    [
        style.then_some(InputKind::Style),
        binding.then_some(InputKind::PaletteBinding),
        row.then_some(InputKind::ParentRow),
    ]
    .into_iter()
    .flatten()
}

/// The resolution inputs a change of one element from `before` to
/// `after` touches. See the module docs. O(1) per field; no allocation.
pub fn inputs_changed(before: &DocElement, after: &DocElement) -> impl Iterator<Item = InputKind> {
    let (style, binding, row, whole) = classify(before, after);
    let whole = whole.into_iter();
    kinds(style, binding, row).chain(whole)
}

/// `(style, binding, row, whole-element input)` for `before` → `after`.
/// The match is over `before` alone and names every kind, so a new kind
/// does not compile until it is classified; an `after` of another kind
/// touches everything an element can be read for.
fn classify(before: &DocElement, after: &DocElement) -> (bool, bool, bool, Option<InputKind>) {
    use DocElement as E;
    const KIND_CHANGED: (bool, bool, bool, Option<InputKind>) = (true, true, true, None);
    match before {
        E::Map(a) => {
            let E::Map(_) = after else { return KIND_CHANGED };
            let MapData { name: _ } = a;
            (false, false, false, None)
        }
        E::Theme(a) => {
            let E::Theme(b) = after else { return KIND_CHANGED };
            (false, false, false, (a != b).then_some(InputKind::Theme))
        }
        E::Palette(a) => {
            let E::Palette(b) = after else { return KIND_CHANGED };
            (false, false, false, (a != b).then_some(InputKind::Palette))
        }
        E::Defaults(a) => {
            let E::Defaults(b) = after else { return KIND_CHANGED };
            (false, false, false, (a != b).then_some(InputKind::Defaults))
        }
        E::Node(a) => {
            let E::Node(b) = after else { return KIND_CHANGED };
            let NodeData {
                ident: _,
                channel,
                position: _,
                min_size: _,
                style,
                palette,
                zoom: _,
                folded: _,
                hidden: _,
                opacity: _,
                on: _,
            } = a;
            (
                style != &b.style,
                palette != &b.palette,
                channel != &b.channel,
                None,
            )
        }
        E::Section(a) => {
            let E::Section(b) = after else { return KIND_CHANGED };
            let SectionData {
                channel,
                offset: _,
                min_size: _,
                align: _,
                text,
            } = a;
            (text != &b.text, false, channel != &b.channel, None)
        }
        E::Span(a) => {
            let E::Span(b) = after else { return KIND_CHANGED };
            let SpanData {
                text: _,
                style,
                link: _,
            } = a;
            (style != &b.style, false, false, None)
        }
        E::Frame(a) => {
            let E::Frame(b) = after else { return KIND_CHANGED };
            let FrameData { style } = a;
            (style != &b.style, false, false, None)
        }
        E::Ring(a) => {
            let E::Ring(b) = after else { return KIND_CHANGED };
            let RingData { channel, style } = a;
            (style != &b.style, false, channel != &b.channel, None)
        }
        E::Connector(a) => {
            let E::Connector(b) = after else { return KIND_CHANGED };
            let ConnectorData { path } = a;
            (path_style_changed(path, &b.path), false, false, None)
        }
        E::Link(a) => {
            let E::Link(b) = after else { return KIND_CHANGED };
            let LinkData { to: _, kind: _, path } = a;
            (path_style_changed(path, &b.path), false, false, None)
        }
        E::CapStart(a) => {
            let E::CapStart(b) = after else { return KIND_CHANGED };
            let CapData { text } = a;
            (text != &b.text, false, false, None)
        }
        E::CapEnd(a) => {
            let E::CapEnd(b) = after else { return KIND_CHANGED };
            let CapData { text } = a;
            (text != &b.text, false, false, None)
        }
        E::Label(a) => {
            let E::Label(b) = after else { return KIND_CHANGED };
            let LabelData { style } = a;
            (style != &b.style, false, false, None)
        }
        E::PortalEnd(a) => {
            let E::PortalEnd(b) = after else { return KIND_CHANGED };
            let PortalEndData { end, style } = a;
            (style != &b.style, false, end != &b.end, None)
        }
    }
}

/// Whether a path's style changed: its flow style is what resolution
/// reads; its anchors, control points, visibility and zoom window are
/// geometry and visibility, read by no resolver.
fn path_style_changed(
    a: &crate::document::element::PathData,
    b: &crate::document::element::PathData,
) -> bool {
    let crate::document::element::PathData {
        flow,
        anchors: _,
        control_points: _,
        visible: _,
        zoom: _,
    } = a;
    flow != &b.flow
}
