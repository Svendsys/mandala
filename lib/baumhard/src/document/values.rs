// SPDX-License-Identifier: MPL-2.0

//! Typed values: every color, size, shape, anchor and mode a document
//! element stores, in the form it is stored in.
//!
//! A format could carry these as strings (`"#30b082"`,
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
use std::borrow::Cow;

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

/// Which of a palette group's three colors a
/// [`ColorRef::Palette`] names.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PaletteField {
    /// The node's frame text.
    Frame,
    /// Section text.
    Text,
    /// The text of the node's first section.
    Title,
}

/// A color a palette group or the map's background may hold: a
/// literal or a theme variable. A palette never refers to another
/// palette, and the background belongs to no node a palette could be
/// bound to, so this has no `Palette` arm. Serialized as [`ColorRef`]'s
/// two arms are, `{"rgba": [r, g, b, a]}` and `{"var": n}`, for the
/// predicate field that tests the background
/// ([`DocField::Background`](crate::document::inspect::DocField::Background)).
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
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

/// A node's outline, which its frame runs around (D11).
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
/// of each junction by the clearance R33.1 keeps. In between, each end
/// of the line moves by arc length from its natural place toward its
/// place at whole, in proportion. A `repeat` ring ignores it. In range
/// by construction: [`LineSpan::new`] refuses anything outside
/// `0.0..=1.0` (and a number that is not one).
#[derive(Copy, Clone, Debug, PartialEq)]
pub struct LineSpan(f32);

impl LineSpan {
    /// The line at its natural width: `"natural"`.
    pub const NATURAL: LineSpan = LineSpan(0.0);
    /// The whole half: `"whole"`, the built-in.
    pub const WHOLE: LineSpan = LineSpan(1.0);

    /// `value` as a span, or `None` outside `0.0..=1.0`. O(1).
    pub fn new(value: f32) -> Option<LineSpan> {
        (0.0..=1.0).contains(&value).then_some(LineSpan(value))
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

/// The schemes a link's [`LinkTarget::Url`] may have (#252 §3): web
/// addresses and mail. Anything else — `file`, `javascript`, a custom
/// handler — is refused at load and at apply, so no map leads a click to
/// it.
pub const LINK_SCHEMES: [&str; 3] = ["http", "https", "mailto"];

/// The header names a `mailto` address may carry in its query: RFC
/// 6068's (ruling D3.3 item 1 on #252). Any other — `attach`,
/// `attachment`, `from`, `X-…`, however spelled — is refused, since some
/// mail handlers have honored one that attached a local file, and they
/// do not read a name alike: an allowlist closes every spelling at once,
/// where a denylist closes only the ones it thought of.
pub const MAIL_HEADERS: [&str; 6] = ["to", "cc", "bcc", "subject", "body", "in-reply-to"];

/// Why a link may not lead to an address ([`check_link_url`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkRefusal<'a> {
    /// Its scheme is not one of [`LINK_SCHEMES`]: the scheme it wrote,
    /// or the whole of it when it has no `:`.
    Scheme(&'a str),
    /// A `mailto` address that names a header outside [`MAIL_HEADERS`],
    /// in its query or its address part, or whose address part decodes
    /// to hold a `?`.
    MailHeader {
        /// The header's name as the address writes it (reading (a)), or
        /// as it reads once the query is decoded (reading (b)); or the
        /// address part, for one that holds a `?`.
        name: Cow<'a, str>,
    },
}

/// `url` if a link may lead there: its scheme — what it writes before its
/// first `:` — is one of [`LINK_SCHEMES`], in any case (a scheme is
/// case-insensitive), and, for `mailto`, every header it names, in its
/// query or its address part, is one of [`MAIL_HEADERS`] under both
/// readings of `mail_header`. Otherwise what is wrong, for the
/// refusal to name. O(the address's length).
pub fn check_link_url(url: &str) -> Result<&str, LinkRefusal<'_>> {
    let (scheme, rest) = url.split_once(':').unwrap_or((url, ""));
    if !(LINK_SCHEMES
        .iter()
        .any(|allowed| allowed.eq_ignore_ascii_case(scheme))
        && scheme.len() < url.len())
    {
        return Err(LinkRefusal::Scheme(scheme));
    }
    if scheme.eq_ignore_ascii_case("mailto") {
        if let Some(name) = mail_header(rest) {
            return Err(LinkRefusal::MailHeader { name });
        }
    }
    Ok(url)
}

/// What is wrong with the `mailto` address `rest` — what follows its
/// `mailto:` — if anything (ruling D3.3 item 1), checked under two
/// readings, both of which must pass, since mail handlers read a query
/// either way:
///
/// - **(a) split first**: the query — what follows the first `?` — is
///   split on every `?`, `&` and `;`; each segment's name, what it
///   writes before its first `=` (all of it, without one), is
///   percent-decoded once (`+` stays `+`) and its ASCII letters
///   lowercased, and every name that is not empty must be one of
///   [`MAIL_HEADERS`] exactly — nothing trimmed or dropped, so a name
///   with a byte more is refused;
/// - **(b) decode first**: the whole query is percent-decoded once, `+`
///   read as a space, then split the same way; every segment with an
///   `=` must name one of [`MAIL_HEADERS`] before it, lowercased. A
///   segment with none is text, not a header, so `body=Q%26A` passes;
///   `body=a%26b%3Dc` does not (it reads a header `b`) — over-refused,
///   on the safe side;
///
/// and the address part — what comes before the first raw `?` — is read
/// the way the query is (ruling D3.4 item 1): one that decodes to hold a
/// `?` is refused, since a handler may read the rest as a query; and
/// split on `?`, `&`, `;`, `,` and `#`, raw and decoded once, every part
/// after the first that holds an `=` must name one of [`MAIL_HEADERS`],
/// since a handler may split what follows `mailto:` without first
/// finding the `?`. A part with no `=` is an address, so `a@b,c@d`
/// passes; `a@b;c=d` does not — over-refused, and a `to` part writes
/// `=` encoded anyway (RFC 6068). Returns the name refused. O(the
/// address's length).
fn mail_header(rest: &str) -> Option<Cow<'_, str>> {
    let (address, query) = rest.split_once('?').unwrap_or((rest, ""));
    let allowed = |name: &[u8]| {
        MAIL_HEADERS
            .iter()
            .any(|header| name.to_ascii_lowercase() == header.as_bytes())
    };
    let decoded_address = percent_decoded(address.as_bytes(), false);
    if decoded_address.contains(&b'?') {
        return Some(Cow::Borrowed(address));
    }
    // The address part, read as the query is (ruling D3.4 item 1): split
    // on `?`, `&`, `;`, `,` and `#`, raw and decoded once, every part
    // after the first with an `=` must name one of the six.
    let address_split = |b: u8| matches!(b, b'?' | b'&' | b';' | b',' | b'#');
    for segment in address
        .split(|c: char| c.is_ascii() && address_split(c as u8))
        .skip(1)
    {
        if let Some((name, _)) = segment.split_once('=') {
            if !allowed(&percent_decoded(name.as_bytes(), false)) {
                return Some(Cow::Borrowed(name));
            }
        }
    }
    for segment in decoded_address.split(|&b| address_split(b)).skip(1) {
        if let Some(at) = segment.iter().position(|&b| b == b'=') {
            let name = &segment[..at];
            if !allowed(name) {
                return Some(Cow::Owned(String::from_utf8_lossy(name).into_owned()));
            }
        }
    }
    let split = |c: char| matches!(c, '?' | '&' | ';');
    for segment in query.split(split) {
        let name = segment.split_once('=').map_or(segment, |(name, _)| name);
        if !name.is_empty() && !allowed(&percent_decoded(name.as_bytes(), false)) {
            return Some(Cow::Borrowed(name));
        }
    }
    let decoded = percent_decoded(query.as_bytes(), true);
    for segment in decoded.split(|&b| matches!(b, b'?' | b'&' | b';')) {
        if let Some(at) = segment.iter().position(|&b| b == b'=') {
            let name = &segment[..at];
            if !allowed(name) {
                return Some(Cow::Owned(String::from_utf8_lossy(name).into_owned()));
            }
        }
    }
    None
}

/// `bytes` with each `%` and two hex digits after it decoded once, as a
/// mail client decodes a header; a `%` not followed by two hex digits is
/// kept as it is, and a `+` is read as a space when `plus_is_space`.
/// O(`bytes`' length).
fn percent_decoded(bytes: &[u8], plus_is_space: bool) -> Vec<u8> {
    let hex = |b: u8| char::from(b).to_digit(16).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if let (b'%', Some(high), Some(low)) = (
            bytes[i],
            bytes.get(i + 1).copied().and_then(hex),
            bytes.get(i + 2).copied().and_then(hex),
        ) {
            out.push(high * 16 + low);
            i += 3;
        } else {
            out.push(if plus_is_space && bytes[i] == b'+' {
                b' '
            } else {
                bytes[i]
            });
            i += 1;
        }
    }
    out
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
