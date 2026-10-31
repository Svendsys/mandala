// SPDX-License-Identifier: MPL-2.0

//! Typed values: every color, size, shape, anchor and mode a document
//! element stores, in the form it is stored in.
//!
//! A `.mindmap.json` used to carry these as strings (`"#30b082"`,
//! `"var(--accent)"`, `"rectangle"`, `"auto"`) that every projection
//! pass parsed again. Here a value is parsed **once**, at load, into
//! one of these types, and nothing after load parses a string
//! (design §2 rule 3, §5.3). Each type is a small `Copy` value except
//! where it carries text
//! ([`FitMode::Once`](crate::document::values::FitMode::Once)'s filler
//! and [`LinkTarget::Url`](crate::document::values::LinkTarget::Url)),
//! which are two of the few `String`s a document element may hold.
//!
//! Some of these types derive serde because the document's predicate
//! surface, [`DocField`](crate::document::inspect::DocField), carries
//! them as reference values. That is a predicate spelling, not the
//! file format: the format (V1-B2) decides how a map file spells a
//! color or a shape.

use crate::document::bounds::{self, BoundError};
use serde::{Deserialize, Serialize};

/// A color as four bytes, red, green, blue and alpha. Final: it
/// resolves to itself.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba(pub [u8; 4]);

impl Rgba {
    /// The color with these four channels. O(1).
    pub const fn new(r: u8, g: u8, b: u8, a: u8) -> Self {
        Rgba([r, g, b, a])
    }

    /// The opaque color with these three channels. O(1).
    pub const fn opaque(r: u8, g: u8, b: u8) -> Self {
        Rgba([r, g, b, 255])
    }
}

/// A theme variable: an index into
/// [`ThemeData::variables`](crate::document::element::ThemeData::variables),
/// and into each variant's `values` at the same position.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct VarId(pub u16);

/// A palette: an index among the map's
/// [`Palette`](crate::document::element::DocKind::Palette) elements,
/// in tree order.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PaletteId(pub u16);

/// Which of a palette group's four colors a
/// [`ColorRef::Palette`] names.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaletteField {
    /// The node's background fill.
    Fill,
    /// The node's frame text.
    Frame,
    /// Section text.
    Text,
    /// The text of the node's first section.
    Title,
}

/// A color a palette group may hold: a literal or a theme variable.
/// A palette never refers to another palette, so this has no
/// `Palette` arm.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum PlainColor {
    /// A literal color.
    Rgba(Rgba),
    /// The theme variable's value in the active variant.
    Var(VarId),
}

/// A color as an element stores it.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorRef {
    /// A literal color.
    Rgba(Rgba),
    /// The theme variable's value in the active variant
    /// ([`resolve`](crate::document::resolve) reads it, and records
    /// that it read the theme).
    Var(VarId),
    /// A color of the palette group the element's owning node is bound
    /// to at its depth (design §5.4).
    Palette(PaletteField),
}

/// A font size.
#[derive(Copy, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FontSize {
    /// Points.
    Pt(f32),
    /// A fraction of the owning node's shorter side, in `(0.0, 1.0]`
    /// ([`bounds::check_rel_size`]). Frames only: a ring's size is
    /// always `Rel`, so the text and the outline it runs around scale
    /// together (D10). Converted to points by the projection, from the
    /// node's drawn size.
    Rel(f32),
}

/// A percentage, `0..=100`. The constructor clamps, so a `Percent` is
/// in range by construction.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "u8")]
pub struct Percent(u8);

impl Percent {
    /// `value`, clamped to [`bounds::MAX_PERCENT`]. O(1).
    pub const fn new(value: u8) -> Self {
        if value > bounds::MAX_PERCENT {
            Percent(bounds::MAX_PERCENT)
        } else {
            Percent(value)
        }
    }

    /// The percentage, `0..=100`. O(1).
    pub const fn get(self) -> u8 {
        self.0
    }
}

/// Clamps, as [`Percent::new`] does, so a predicate's reference value
/// cannot hold a percentage no element can.
impl From<u8> for Percent {
    fn from(value: u8) -> Self {
        Percent::new(value)
    }
}

/// A node's outline, which its fill and frame share (D11).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Shape {
    /// A rectangle whose corner radius is `radius` percent of half the
    /// node's shorter side: 0 is sharp, 100 a circle on a square node
    /// and a pill on a wide one.
    Rectangle {
        /// The corner radius, as a percentage of half the shorter side.
        radius: Percent,
    },
    /// The ellipse inscribed in the node's rectangle.
    Ellipse,
}

/// Where a path attaches to a node.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Anchor {
    /// The side facing the other end, chosen by the projection.
    Auto,
    /// The top side's midpoint.
    Top,
    /// The right side's midpoint.
    Right,
    /// The bottom side's midpoint.
    Bottom,
    /// The left side's midpoint.
    Left,
}

/// What a [`Link`](crate::document::element::DocKind::Link) draws.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LinkKind {
    /// A path from its node to the target, like a connector.
    Line,
    /// Two portal ends, one beside each node, and no path.
    Portal,
}

/// A font weight, `100..=900` in steps of 100. Built only through
/// [`Weight::new`], so a `Weight` is always one of the nine.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Weight(u16);

impl Weight {
    /// 400, the regular weight.
    pub const NORMAL: Weight = Weight(400);
    /// 700, the bold weight.
    pub const BOLD: Weight = Weight(700);

    /// `value` as a weight, or why it is not one
    /// ([`bounds::check_weight`]). O(1).
    pub fn new(value: u16) -> Result<Self, BoundError> {
        bounds::check_weight(value).map(Weight)
    }

    /// The weight, `100..=900`. O(1).
    pub const fn get(self) -> u16 {
        self.0
    }
}

/// Horizontal alignment of a section's lines in its box.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Align {
    /// Flush left.
    Left,
    /// Centered.
    Center,
    /// Flush right.
    Right,
}

/// A side of a node's outline.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Edge {
    /// The top side.
    Top,
    /// The right side.
    Right,
    /// The bottom side.
    Bottom,
    /// The left side.
    Left,
}

/// Where a frame's text starts on the outline: a side, and a fraction
/// along it.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct StartPoint {
    /// The side the text starts on.
    pub edge: Edge,
    /// How far along that side, `0.0..=1.0`, measured clockwise.
    pub at: f32,
}

/// Which way a frame's text runs around the outline.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Direction {
    /// Clockwise.
    Clockwise,
    /// Counter-clockwise.
    CounterClockwise,
}

/// What a path flow does when its text is shorter or longer than its
/// path (design §6.4).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FitMode {
    /// The text once from the start; the rest of the path gets the
    /// filler repeated, or nothing.
    Once {
        /// The text repeated after the content, if any.
        filler: Option<String>,
    },
    /// The text repeated until the path is full, the last repetition
    /// cut at a grapheme boundary.
    Repeat,
    /// The text once, with extra spacing so it ends at the path's end.
    Spread,
    /// Font size and spacing scaled so the text fills the path exactly
    /// once, the scale clamped to `min..=max`
    /// ([`bounds::check_fit`]).
    Fit {
        /// The smallest scale, a multiple of the declared size.
        min: f32,
        /// The largest scale, a multiple of the declared size.
        max: f32,
    },
}

/// How much of its half each line of a frame's two-line ring covers
/// (#291, rulings R32 and R33): `0.0` its natural width — its font's own
/// letter gap, centered on its apex — up to `1.0` the whole half, short
/// of each junction by the clearance R33.1 keeps. In between, each end of the line moves by arc
/// length from its natural place toward its half's junction in
/// proportion. A `repeat` ring ignores it. In range by construction:
/// [`Span::new`] refuses anything outside `0.0..=1.0` (and a number that
/// is not one).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct Span(f32);

impl Span {
    /// The line at its natural width: `"natural"`.
    pub const NATURAL: Span = Span(0.0);
    /// The whole half: `"whole"`, the built-in.
    pub const WHOLE: Span = Span(1.0);

    /// `value` as a span, or `None` outside `0.0..=1.0`. O(1).
    pub fn new(value: f32) -> Option<Span> {
        (0.0..=1.0).contains(&value).then_some(Span(value))
    }

    /// The fraction, `0.0..=1.0`. O(1).
    pub fn get(self) -> f32 {
        self.0
    }
}

/// What closes the parts of a frame's outline its two lines leave
/// uncovered (#291, rulings R32 and R33.1): around each junction where
/// the lines meet, whole copies of an ornament, every gap and both joins
/// to the text equal, its ink no taller than the text's x-height.
#[derive(Clone, Debug, PartialEq)]
pub enum Sides {
    /// None: what a ring says to drop sides its frame or the defaults
    /// give it.
    Off,
    /// An ornament.
    On {
        /// The ornament's text, one copy (`"·"`, `"✦"`); never empty
        /// ([`check_ornament`](crate::document::bounds::check_ornament)).
        content: String,
        /// Its color; the ring's when `None`.
        color: Option<ColorRef>,
    },
}

/// The zoom range an element renders in. Either bound may be open.
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct ZoomWindow {
    /// The lowest camera zoom it renders at, inclusive.
    pub min: Option<f32>,
    /// The highest camera zoom it renders at, inclusive.
    pub max: Option<f32>,
}

/// Where a linked span leads.
#[derive(Clone, Debug, PartialEq)]
pub enum LinkTarget {
    /// A web address.
    Url(String),
    /// One of the map's verbs.
    Verb(VerbId),
}

/// A verb: an index into
/// [`Definitions::verbs`](crate::document::ids::Definitions::verbs).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct VerbId(pub u16);

/// An edit: an index into
/// [`Definitions::edits`](crate::document::ids::Definitions::edits).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct EditId(pub u16);

/// A presentation: an index into
/// [`Definitions::presentations`](crate::document::ids::Definitions::presentations).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub struct PresentationId(pub u16);
