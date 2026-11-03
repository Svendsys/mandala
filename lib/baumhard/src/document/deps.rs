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
//! holds it to that**: each data struct, and each style struct that
//! mixes looks with placement, is destructured with every field named,
//! so a field added to an element does not compile until this module
//! says which input it belongs to. What is classified as no input is
//! what no resolver reads — a node's position and minimum size, a
//! section's offset, a path's anchors and control points, a span's
//! text. Moving or resizing a node therefore touches no input, and a
//! label or portal-marker drag touches only `Placement`, which no text
//! resolution reads: a drag re-places geometry and reshapes nothing.

use crate::document::element::{
    CapData, ConnectorData, DocElement, FrameData, FrameStyle, LabelData, LabelStyle, LinkData, MapData,
    NodeData, PathData, PortalEndData, PortalEndStyle, RingData, SectionData, SpanData,
};

/// One kind of resolution input a change of an element can touch. The
/// element-scoped kinds map onto the element's own
/// [`Input`](crate::document::resolve::Input); `ParentRow` onto its
/// parent's.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum InputKind {
    /// `Input::Style` of the element: a field it draws with changed.
    Style,
    /// `Input::Placement` of the element: a field that says where it
    /// sits changed — a label's `t` or `offset`, a portal end's
    /// `border_t` or `offset`, a frame's `start`, `direction`, `gap` or
    /// `inset`.
    Placement,
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

/// What one change touched.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
struct Touched {
    style: bool,
    placement: bool,
    binding: bool,
    row: bool,
    whole: Option<InputKind>,
}

impl Touched {
    /// Everything an element can be read for: a change of kind.
    const ALL: Touched = Touched {
        style: true,
        placement: true,
        binding: true,
        row: true,
        whole: None,
    };

    /// The kinds set, in a fixed order.
    fn kinds(self) -> impl Iterator<Item = InputKind> {
        [
            self.style.then_some(InputKind::Style),
            self.placement.then_some(InputKind::Placement),
            self.binding.then_some(InputKind::PaletteBinding),
            self.row.then_some(InputKind::ParentRow),
            self.whole,
        ]
        .into_iter()
        .flatten()
    }
}

/// The resolution inputs a change of one element from `before` to
/// `after` touches. See the module docs. O(1) per field; no allocation.
pub fn inputs_changed(before: &DocElement, after: &DocElement) -> impl Iterator<Item = InputKind> {
    classify(before, after).kinds()
}

/// A style-only change.
fn style(changed: bool) -> Touched {
    Touched {
        style: changed,
        ..Touched::default()
    }
}

/// A whole-element input.
fn whole(changed: bool, kind: InputKind) -> Touched {
    Touched {
        whole: changed.then_some(kind),
        ..Touched::default()
    }
}

/// What `before` → `after` touches. The match is over `before` alone and
/// names every kind, so a new kind does not compile until it is
/// classified; an `after` of another kind touches everything an element
/// can be read for.
fn classify(before: &DocElement, after: &DocElement) -> Touched {
    use DocElement as E;
    match before {
        E::Map(a) => {
            let E::Map(_) = after else { return Touched::ALL };
            let MapData { name: _ } = a;
            Touched::default()
        }
        E::Theme(a) => {
            let E::Theme(b) = after else { return Touched::ALL };
            whole(a != b, InputKind::Theme)
        }
        E::Palette(a) => {
            let E::Palette(b) = after else { return Touched::ALL };
            whole(a != b, InputKind::Palette)
        }
        E::Defaults(a) => {
            let E::Defaults(b) = after else { return Touched::ALL };
            whole(a != b, InputKind::Defaults)
        }
        E::Node(a) => {
            let E::Node(b) = after else { return Touched::ALL };
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
            Touched {
                style: style != &b.style,
                binding: palette != &b.palette,
                row: channel != &b.channel,
                ..Touched::default()
            }
        }
        E::Section(a) => {
            let E::Section(b) = after else { return Touched::ALL };
            let SectionData {
                channel,
                offset: _,
                min_size: _,
                align: _,
                text,
            } = a;
            Touched {
                style: text != &b.text,
                row: channel != &b.channel,
                ..Touched::default()
            }
        }
        E::Span(a) => {
            let E::Span(b) = after else { return Touched::ALL };
            let SpanData {
                text: _,
                style: span_style,
                link: _,
            } = a;
            style(span_style != &b.style)
        }
        E::Frame(a) => {
            let E::Frame(b) = after else { return Touched::ALL };
            let FrameData { style: a } = a;
            let FrameData { style: b } = b;
            let FrameStyle {
                visible,
                start,
                direction,
                fit,
                gap,
                inset,
                ring_layout,
                text,
            } = a;
            Touched {
                style: visible != &b.visible
                    || fit != &b.fit
                    || ring_layout != &b.ring_layout
                    || text != &b.text,
                placement: start != &b.start
                    || direction != &b.direction
                    || gap != &b.gap
                    || inset != &b.inset,
                ..Touched::default()
            }
        }
        E::Ring(a) => {
            let E::Ring(b) = after else { return Touched::ALL };
            let RingData {
                channel,
                style: ring_style,
            } = a;
            Touched {
                style: ring_style != &b.style,
                row: channel != &b.channel,
                ..Touched::default()
            }
        }
        E::Connector(a) => {
            let E::Connector(b) = after else { return Touched::ALL };
            let ConnectorData { path } = a;
            style(path_style_changed(path, &b.path))
        }
        E::Link(a) => {
            let E::Link(b) = after else { return Touched::ALL };
            let LinkData { to: _, kind: _, path } = a;
            style(path_style_changed(path, &b.path))
        }
        E::CapStart(a) => {
            let E::CapStart(b) = after else { return Touched::ALL };
            let CapData { text } = a;
            style(text != &b.text)
        }
        E::CapEnd(a) => {
            let E::CapEnd(b) = after else { return Touched::ALL };
            let CapData { text } = a;
            style(text != &b.text)
        }
        E::Label(a) => {
            let E::Label(b) = after else { return Touched::ALL };
            let LabelData { style: a } = a;
            let LabelData { style: b } = b;
            let LabelStyle { t, offset, text } = a;
            Touched {
                style: text != &b.text,
                placement: t != &b.t || offset != &b.offset,
                ..Touched::default()
            }
        }
        E::PortalEnd(a) => {
            let E::PortalEnd(b) = after else { return Touched::ALL };
            let PortalEndData { end, style: a } = a;
            let PortalEndData { end: b_end, style: b } = b;
            let PortalEndStyle {
                border_t,
                offset,
                color,
                size,
                content,
            } = a;
            Touched {
                style: color != &b.color || size != &b.size || content != &b.content,
                placement: border_t != &b.border_t || offset != &b.offset,
                row: end != b_end,
                ..Touched::default()
            }
        }
    }
}

/// Whether a path's style changed: its flow style is what resolution
/// reads; its anchors, control points, visibility and zoom window are
/// geometry and visibility, read by no resolver.
fn path_style_changed(a: &PathData, b: &PathData) -> bool {
    let PathData {
        flow,
        anchors: _,
        control_points: _,
        visible: _,
        zoom: _,
    } = a;
    flow != &b.flow
}
