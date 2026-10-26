// SPDX-License-Identifier: MPL-2.0

//! [`BUILT_IN`](crate::document::defaults::BUILT_IN): the last tier of
//! every resolution — the value a field takes when the element, its
//! owners, its palette and the map's `Defaults` all leave it unset.
//!
//! The values reproduce what the app draws today for a map that says
//! nothing, each taken from the constant that decides it now (the
//! sources are named beside each value), so a map converted to the
//! document draws as it did. Fields the document adds — `fill_fade`,
//! the frame's start, direction, fit, gap and inset, ring sizes,
//! `content` — take the defaults design §5 and #235 give them.
//!
//! `BUILT_IN` is fully resolved: no `Option`, no theme variable, no
//! palette reference. It is keyed by **where text is drawn**, not by
//! the kind of the element that holds it: a span in a section, in a
//! ring, on a connector, on a link, in a label and in a portal marker
//! each fall back to their own text, because today those six draw at
//! different sizes and in different colors.

use crate::document::resolve::{BorderPoint, ResolvedText};
use crate::document::values::{Direction, Edge, FitMode, FontSize, Percent, Rgba, Shape, StartPoint, Weight};
use crate::font::fonts::AppFont;

/// A node's opacity when the file does not give one: fully opaque.
/// `opacity` is not inherited, so it has no tier here; the loader
/// writes this.
pub const DEFAULT_OPACITY: f32 = 1.0;

/// The size of connector and link text, in points:
/// `default_connection_font_size()` in
/// `lib/baumhard/src/mindmap/model/edge.rs`.
pub const PATH_TEXT_SIZE_PT: f32 = 12.0;

/// How much larger a label's text is than its path's:
/// `DEFAULT_LABEL_SIZE_FACTOR` in
/// `lib/baumhard/src/mindmap/model/edge.rs`.
pub const LABEL_SIZE_FACTOR: f32 = 1.1;

/// The text every flow falls back to before its size and color: no
/// pinned face — today's run font, `"LiberationSans"`, names no
/// compiled-in face and resolves to none, so text draws in the
/// shaper's fallback face (`AppFont::Any`) — regular weight, upright.
const fn text(size: FontSize, color: Rgba) -> ResolvedText {
    ResolvedText {
        font: AppFont::Any,
        size,
        color,
        weight: Weight::NORMAL,
        italic: false,
    }
}

/// [`BUILT_IN`]'s node fields.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltInNode {
    /// The outline.
    pub shape: Shape,
    /// The fill.
    pub fill: Rgba,
    /// The fill's soft edge.
    pub fill_fade: f32,
}

/// [`BUILT_IN`]'s frame fields.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltInFrame {
    /// Whether a frame is drawn.
    pub visible: bool,
    /// Where its text starts.
    pub start: StartPoint,
    /// Which way its text runs.
    pub direction: Direction,
    /// What its rings do when text and outline differ in length.
    pub fit: FitMode,
    /// The space between rings.
    pub gap: f32,
    /// The first ring's distance from the outline.
    pub inset: f32,
}

/// [`BUILT_IN`]'s ring fields.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltInRing {
    /// Ring text. Its size is `Rel`, like every ring's.
    pub text: ResolvedText,
}

/// [`BUILT_IN`]'s fields for one kind of path.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltInPath {
    /// What its text does when text and path differ in length.
    pub fit: FitMode,
    /// The extra space between glyphs.
    pub spacing: f32,
    /// What it draws when it has no spans.
    pub content: &'static str,
    /// Its text, and its caps' text.
    pub text: ResolvedText,
}

/// [`BUILT_IN`]'s label fields.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltInLabel {
    /// The position along the path.
    pub t: f32,
    /// The distance from the path.
    pub offset: f32,
    /// Label text.
    pub text: ResolvedText,
}

/// [`BUILT_IN`]'s portal-end fields.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltInPortalEnd {
    /// The position around the outline.
    pub border_t: BorderPoint,
    /// The distance from the outline.
    pub offset: f32,
    /// The marker drawn when the end has no spans.
    pub content: &'static str,
    /// Marker text.
    pub text: ResolvedText,
}

/// Every built-in default, by where it applies.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltIn {
    /// Nodes.
    pub node: BuiltInNode,
    /// Section text, and the text of a node's own style.
    pub section: ResolvedText,
    /// Frames.
    pub frame: BuiltInFrame,
    /// Rings.
    pub ring: BuiltInRing,
    /// Connectors and their caps.
    pub connector: BuiltInPath,
    /// Line links and their caps.
    pub link: BuiltInPath,
    /// Labels.
    pub label: BuiltInLabel,
    /// Portal ends.
    pub portal_end: BuiltInPortalEnd,
}

/// The built-in defaults: the last tier of every resolution. See the
/// module docs; each value names its source.
pub const BUILT_IN: BuiltIn = BuiltIn {
    node: BuiltInNode {
        // #235's table. Today's `default_shape()` is "rectangle"; its
        // corner radius was `corner_radius_percent`, deleted by D13.
        shape: Shape::Rectangle {
            radius: Percent::new(0),
        },
        // `DEFAULT_NODE_BACKGROUND_COLOR`, src/application/document/defaults.rs.
        fill: Rgba::opaque(0x14, 0x14, 0x14),
        // #235's table: new with D12.
        fill_fade: 0.0,
    },
    // Size: `DEFAULT_RUN_SIZE_PT`; color: `DEFAULT_NODE_TEXT_COLOR`;
    // both src/application/document/defaults.rs. Upright regular:
    // `default_text_run` sets neither bold nor italic.
    section: text(FontSize::Pt(24.0), Rgba::opaque(0xff, 0xff, 0xff)),
    frame: BuiltInFrame {
        // D25 (amended #235): a Frame element is shown unless it says
        // otherwise; a node with no Frame element draws none.
        visible: true,
        // #235's table.
        start: StartPoint {
            edge: Edge::Top,
            at: 0.0,
        },
        direction: Direction::Clockwise,
        fit: FitMode::Fit { min: 0.5, max: 2.0 },
        // New with rings; no counterpart today. Rings touch, and the
        // first sits on the outline.
        gap: 0.0,
        inset: 0.0,
    },
    ring: BuiltInRing {
        // Size: #235's table (amended: a ring's size is its text size,
        // always `Rel`). Color: `DEFAULT_NODE_FRAME_COLOR`,
        // src/application/document/defaults.rs.
        text: text(FontSize::Rel(0.06), Rgba::opaque(0x30, 0xb0, 0x82)),
    },
    connector: BuiltInPath {
        // #235's table.
        fit: FitMode::Repeat,
        // `GlyphConnectionConfig::default().spacing`, mindmap/model/edge.rs.
        spacing: 0.0,
        // `default_connection_body()`, mindmap/model/edge.rs.
        content: "\u{00B7}",
        // Color: `default_parent_child_edge`'s `#888888`,
        // src/application/document/defaults.rs.
        text: text(FontSize::Pt(PATH_TEXT_SIZE_PT), Rgba::opaque(0x88, 0x88, 0x88)),
    },
    link: BuiltInPath {
        fit: FitMode::Repeat,
        spacing: 0.0,
        content: "\u{00B7}",
        // Color: `default_cross_link_edge`'s `#aa88cc`,
        // src/application/document/defaults.rs.
        text: text(FontSize::Pt(PATH_TEXT_SIZE_PT), Rgba::opaque(0xaa, 0x88, 0xcc)),
    },
    label: BuiltInLabel {
        // `EdgeLabelConfig::effective_position_t`'s midpoint and
        // `effective_perpendicular_offset`'s `0.0`, mindmap/model/edge.rs.
        t: 0.5,
        offset: 0.0,
        // Size: the path's size times `DEFAULT_LABEL_SIZE_FACTOR`
        // (`EdgeLabelConfig::effective_font_size_pt`). Color: a label
        // inherits its edge's color, `#888888` on a parent-child edge.
        text: text(
            FontSize::Pt(PATH_TEXT_SIZE_PT * LABEL_SIZE_FACTOR),
            Rgba::opaque(0x88, 0x88, 0x88),
        ),
    },
    portal_end: BuiltInPortalEnd {
        // `PortalEndpointState::border_t` `None`: face the other end.
        border_t: BorderPoint::Auto,
        // `PortalEndpointState::perpendicular_offset` `None`: flush.
        offset: 0.0,
        // `PORTAL_GLYPH_PRESETS[0]`, mindmap/model/edge.rs.
        content: "\u{25C8}",
        // Size: `DEFAULT_PORTAL_MARKER_FONT_SIZE_PT`,
        // mindmap/tree_builder/portal_style.rs. Color: the portal
        // edge's `#aa88cc` (`default_portal_edge`).
        text: text(FontSize::Pt(50.0), Rgba::opaque(0xaa, 0x88, 0xcc)),
    },
};
