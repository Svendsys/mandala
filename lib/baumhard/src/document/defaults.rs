// SPDX-License-Identifier: MPL-2.0

//! [`BUILT_IN`](crate::document::defaults::BUILT_IN): the last tier of
//! every resolution — the value a field takes when the element, its
//! owners, its palette and the map's `Defaults` all leave it unset.
//!
//! The values reproduce what the app draws today for a map that says
//! nothing, each taken from the constant that decides it now (the
//! sources are named beside each value), so a map converted to the
//! document draws as it did. Fields the document adds — the frame's
//! start, direction, fit, gap and inset, ring sizes,
//! `content` — take the defaults design §5 and #235 give them.
//!
//! `BUILT_IN` is fully resolved: no `Option`, no theme variable, no
//! palette reference. It is keyed by **where text is drawn**, not by
//! the kind of the element that holds it: a span in a section, in a
//! ring, on a connector, on a link, in a portal marker, and in a label
//! on each of the three label hosts falls back to its own text, because
//! today those draw at different sizes and in different colors.

use crate::document::resolve::{BorderPoint, ResolvedText};
use crate::document::values::{
    Direction, Edge, FitMode, FontSize, LineSpan, Percent, Rgba, Shape, Sides, StartPoint, Weight,
};
use crate::font::fonts::AppFont;

/// The map's background when the file does not give one: black
/// (V1-B12, "a single background color, default black").
/// `background` is not inherited, so it has no tier here; the loader
/// writes this, and the saver leaves a background equal to it out.
pub const DEFAULT_BACKGROUND: Rgba = Rgba::opaque(0, 0, 0);

/// A node's opacity when the file does not give one: fully opaque.
/// `opacity` is not inherited, so it has no tier here; the loader
/// writes this.
pub const DEFAULT_OPACITY: f32 = 1.0;

/// The smallest scale a fit mode `"fit"` allows when the file gives no
/// bounds: the lower end of [`BUILT_IN`]'s fit. `"fit"` alone, in a
/// file, means these bounds.
pub const FIT_MIN: f32 = 0.5;

/// The largest scale a fit mode `"fit"` allows when the file gives no
/// bounds: the upper end of [`BUILT_IN`]'s fit.
pub const FIT_MAX: f32 = 2.0;

/// The color of connector text: `#888888`, the old model's parent-child
/// edge default (`default_parent_child_edge` (gone)), kept so a converted
/// map draws as it did.
const CONNECTOR_COLOR: Rgba = Rgba::opaque(0x88, 0x88, 0x88);

/// The color of line-link and portal text: `#aa88cc`, the old model's
/// cross-link and portal edge defaults (`default_cross_link_edge` (gone),
/// `default_portal_edge` (gone)), kept so a converted map draws as it did.
const LINK_COLOR: Rgba = Rgba::opaque(0xaa, 0x88, 0xcc);

/// The size of a portal marker, in points (#235's table).
pub const PORTAL_MARKER_SIZE_PT: f32 = 50.0;

/// The size of connector and link text, in points: the old model's
/// connection default (`default_connection_font_size` (gone)), kept so a
/// converted map draws as it did.
pub const PATH_TEXT_SIZE_PT: f32 = 12.0;

/// How much larger a label's text is than its path's (#235's table).
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
    /// How much of its half each of a two-line ring's lines covers.
    pub span: LineSpan,
    /// The ornament in the breaks between a two-line ring's lines.
    pub sides: Sides,
    /// The space a `repeat` ring adds between every two glyph steps.
    pub spacing: f32,
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

/// [`BUILT_IN`]'s label fields. A label's text depends on what it
/// labels, as it does today: it reads like its host.
#[derive(Clone, Debug, PartialEq)]
pub struct BuiltInLabel {
    /// The position along the path.
    pub t: f32,
    /// The distance from the path.
    pub offset: f32,
    /// The text of a label on a connector.
    pub on_connector: ResolvedText,
    /// The text of a label on a line link.
    pub on_link: ResolvedText,
    /// The text of a label beside a portal end.
    pub on_portal_end: ResolvedText,
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
        fit: FitMode::Fit {
            min: FIT_MIN,
            max: FIT_MAX,
        },
        // New with rings; no counterpart today. Rings touch, and the
        // first sits on the outline.
        gap: 0.0,
        inset: 0.0,
        // #291 (ruling R32): each line over its whole half, no ornament
        // in the breaks, a ring's copies as the font sets them.
        span: LineSpan::WHOLE,
        sides: Sides::Off,
        spacing: 0.0,
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
        // #235's table.
        spacing: 0.0,
        // #235's table: the middle dot.
        content: "\u{00B7}",
        text: text(FontSize::Pt(PATH_TEXT_SIZE_PT), CONNECTOR_COLOR),
    },
    link: BuiltInPath {
        fit: FitMode::Repeat,
        spacing: 0.0,
        content: "\u{00B7}",
        text: text(FontSize::Pt(PATH_TEXT_SIZE_PT), LINK_COLOR),
    },
    label: BuiltInLabel {
        // The path's midpoint, on the path (#235's table).
        t: 0.5,
        offset: 0.0,
        // On a path: the path's size times `DEFAULT_LABEL_SIZE_FACTOR`
        // (`EdgeLabelConfig::effective_font_size_pt`), in the edge's
        // color (`EdgeLabelConfig`'s "edge color cascade").
        on_connector: text(
            FontSize::Pt(PATH_TEXT_SIZE_PT * LABEL_SIZE_FACTOR),
            CONNECTOR_COLOR,
        ),
        on_link: text(FontSize::Pt(PATH_TEXT_SIZE_PT * LABEL_SIZE_FACTOR), LINK_COLOR),
        // Beside a portal end: the marker's size and color at 1:1,
        // without the label factor, on purpose (#235's table).
        on_portal_end: text(FontSize::Pt(PORTAL_MARKER_SIZE_PT), LINK_COLOR),
    },
    portal_end: BuiltInPortalEnd {
        // `PortalEndpointState::border_t` `None`: face the other end.
        border_t: BorderPoint::Auto,
        // `PortalEndpointState::perpendicular_offset` `None`: flush.
        offset: 0.0,
        // #235's table: the first portal glyph.
        content: "\u{25C8}",
        text: text(FontSize::Pt(PORTAL_MARKER_SIZE_PT), LINK_COLOR),
    },
};
