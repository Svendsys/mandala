// SPDX-License-Identifier: MPL-2.0

//! What a document costs, on the two axes wasm32 bounds a map by
//! ([`MAX_ELEMENTS`](crate::document::format::MAX_ELEMENTS) and
//! [`MAX_MAP_BYTES`](crate::document::format::MAX_MAP_BYTES)): how many
//! elements it holds, and how many bytes of text.
//!
//! A load refuses a map past either bound; the writer refuses an edit
//! that would grow a document past it, so a browser that opened a map
//! never traps on editing it. The element count is the loader's own
//! (`format`'s builder counts each element it appends, and each
//! definition it reads): the writer starts from that count and moves it
//! by the elements each journal puts into the tree or takes out of it.
//! The text is every string an element holds — the span texts, and the
//! names, URLs, contents and fillers beside them
//! ([`text_bytes`](crate::document::writer::budget::text_bytes)) — moved
//! the same way. Neither is ever recounted by a walk after load.

use crate::document::element::{
    DefaultsData, DirectionalGlyphs, DocElement, FlowStyle, FrameData, FrameStyle, MapData, PaletteData,
    PortalEndData, PortalEndStyle, RingData, RingLayoutStyle, RingStyle, SpanData, ThemeData, ThemeVariant,
};
use crate::document::values::{FitMode, LinkTarget, Sides};

/// A document's cost on the two axes.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Budget {
    /// The elements in the tree, the map included, and the definitions
    /// beside it — what the loader counts against `MAX_ELEMENTS`.
    pub elements: u64,
    /// The bytes of every string the elements in the tree hold.
    pub text: u64,
}

impl Budget {
    /// `element` entering the tree. O(its strings).
    pub(super) fn enter(&mut self, element: &DocElement) {
        self.elements = self.elements.saturating_add(1);
        self.text = self.text.saturating_add(text_bytes(element));
    }

    /// `element` leaving the tree. O(its strings).
    pub(super) fn leave(&mut self, element: &DocElement) {
        self.elements = self.elements.saturating_sub(1);
        self.text = self.text.saturating_sub(text_bytes(element));
    }
}

/// The bytes of every string `element` holds: a span's text and URL, the
/// map's name, the theme's variable and variant names, a palette's name,
/// a path's or a portal end's content, a fit mode's filler — on the
/// element and, for the defaults, in every style they carry. Every
/// struct that holds a string is taken apart field by field, with no
/// rest pattern, so a string field added to one is a compile error here.
///
/// O(the element's strings); reads lengths only.
pub fn text_bytes(element: &DocElement) -> u64 {
    match element {
        DocElement::Map(MapData { name, .. }) => len(name),
        DocElement::Theme(ThemeData {
            variables,
            variants,
            active: _,
        }) => {
            let variables: u64 = variables.iter().map(|name| len(name)).sum();
            let variants: u64 = variants
                .iter()
                .map(|ThemeVariant { name, values: _ }| len(name))
                .sum();
            variables + variants
        }
        DocElement::Palette(PaletteData { name, groups: _ }) => len(name),
        DocElement::Defaults(defaults) => {
            let DefaultsData {
                node: _,
                section: _,
                frame,
                ring,
                connector,
                link,
                label: _,
                portal_end,
                span: _,
            } = &**defaults;
            frame_style(frame)
                + ring_style(ring)
                + flow(connector)
                + flow(link)
                + portal_end_style(portal_end)
        }
        DocElement::Span(SpanData { text, style: _, link }) => {
            len(text)
                + match link {
                    Some(LinkTarget::Url(url)) => len(url),
                    Some(LinkTarget::Verb(_)) | None => 0,
                }
        }
        DocElement::Frame(FrameData { style }) => frame_style(style),
        DocElement::Ring(RingData {
            channel: _,
            style,
            directional,
        }) => ring_style(style) + directional.as_deref().map_or(0, directional_glyphs),
        DocElement::Connector(connector) => flow(&connector.path.flow),
        DocElement::Link(link) => flow(&link.path.flow),
        DocElement::PortalEnd(PortalEndData { end: _, style }) => portal_end_style(style),
        DocElement::Section(_)
        | DocElement::CapStart(_)
        | DocElement::CapEnd(_)
        | DocElement::Label(_)
        | DocElement::Node(_) => 0,
    }
}

/// A string's length in bytes.
fn len(text: &str) -> u64 {
    u64::try_from(text.len()).unwrap_or(u64::MAX)
}

/// A fit mode's filler.
fn fit(mode: &Option<FitMode>) -> u64 {
    match mode {
        Some(FitMode::Once { filler: Some(filler) }) => len(filler),
        Some(FitMode::Once { filler: None } | FitMode::Repeat | FitMode::Spread | FitMode::Fit { .. })
        | None => 0,
    }
}

fn flow(style: &FlowStyle) -> u64 {
    let FlowStyle {
        fit: mode,
        spacing: _,
        size: _,
        color: _,
        content,
    } = style;
    fit(mode) + content.as_deref().map_or(0, len)
}

fn frame_style(style: &FrameStyle) -> u64 {
    let FrameStyle {
        visible: _,
        start: _,
        direction: _,
        fit: mode,
        gap: _,
        inset: _,
        ring_layout,
        text: _,
    } = style;
    fit(mode) + ring_layout_style(ring_layout)
}

fn ring_style(style: &RingStyle) -> u64 {
    let RingStyle {
        fit: mode,
        ring_layout,
        text: _,
    } = style;
    fit(mode) + ring_layout_style(ring_layout)
}

/// A ring's glyphs by direction (#324): their texts.
fn directional_glyphs(glyphs: &DirectionalGlyphs) -> u64 {
    let DirectionalGlyphs { vertical, corners } = glyphs;
    vertical.as_deref().map_or(0, len) + corners.iter().flatten().map(|c| len(c)).sum::<u64>()
}

/// A ring's layout: its `sides` ornament's text (#291).
fn ring_layout_style(style: &RingLayoutStyle) -> u64 {
    let RingLayoutStyle {
        span: _,
        sides,
        spacing: _,
    } = style;
    match sides {
        Some(Sides::On { content, color: _ }) => len(content),
        Some(Sides::Off) | None => 0,
    }
}

fn portal_end_style(style: &PortalEndStyle) -> u64 {
    let PortalEndStyle {
        border_t: _,
        offset: _,
        color: _,
        size: _,
        content,
    } = style;
    content.as_deref().map_or(0, len)
}
