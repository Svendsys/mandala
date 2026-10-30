// SPDX-License-Identifier: MPL-2.0

//! The `.mandala.json` loader's fuzz properties (design D29, §16;
//! TEST_CONVENTIONS §T14): what every input must satisfy, as functions
//! the `fuzz/` targets call on what libFuzzer hands them and the suite
//! calls on its own sweeps and on every finding.
//!
//! Two entry points, one per fuzz target:
//!
//! - [`check_load_bytes`](crate::document::tests::format_fuzz::check_load_bytes)
//!   takes any bytes (the load_raw target);
//! - [`check_structured`](crate::document::tests::format_fuzz::check_structured)
//!   reads its bytes as choices
//!   ([`Choices`](crate::util::tests::choices::Choices)) and builds a map
//!   whose structure an independent model judges (load_structured).
//!
//! Each returns `Err` naming the property an input broke; a fuzz target
//! panics on it, so libFuzzer records the input. A panic in the code
//! under test is a finding the same way. The properties:
//!
//! - **A refusal is well-formed.** Its `Display` holds no raw control,
//!   line separator or bidi control; its excerpt is at most
//!   [`EXCERPT_BYTES`](crate::document::format::context::EXCERPT_BYTES) bytes of the node's text; its path resolves in the
//!   input (`SiblingsOutOfOrder` included) and names `nodes[i]` exactly
//!   when it carries the node context of node `i`; its line is inside
//!   that node.
//! - **What loads is valid.** Every element passes `validate_shape`,
//!   every number is inside its bound in
//!   [`bounds`](crate::document::bounds) (read here from
//!   `format/schema.md`'s table, not from the loader's calls), the
//!   document is savable, and the one-node check agrees with the full
//!   build.
//! - **What loads round-trips.** `save`, `load`, `save` reproduces the
//!   document and the text, byte for byte, and the text holds no raw
//!   control (C0 but the line feeds between keys, DEL, C1), line
//!   separator or bidi control (R23).
//! - **Structured only:** the loader accepts exactly what the
//!   differential oracle (a port of the B2 review's `chain_ref.py`)
//!   accepts, refuses with the error the reference model expects, and a
//!   single-fault input is refused with the same kind at the same path
//!   whatever the order of its keys (C.9.14, which holds for one fault
//!   only: with two, the first in text order wins).

use crate::document::bounds::{self, BoundError};
use crate::document::element::{
    DocElement, FlowStyle, FrameStyle, LabelStyle, NodeStyle, PathData, PortalEndStyle, RingStyle, TextStyle,
};
use crate::document::format::context::EXCERPT_BYTES;
use crate::document::format::{
    check_savable_by_building, check_savable_under, load, load_from_bytes, save, Limits, LoadError,
    LoadErrorKind, LIMITS,
};
use crate::document::ids::Document;
use crate::document::tests::format_support::same_document;
use crate::document::values::{FitMode, FontSize, Shape, ZoomWindow};

/// Every property of a load of `bytes`, whatever they are: the
/// load_raw target's whole body.
pub fn check_load_bytes(bytes: &[u8]) -> Result<(), String> {
    match load_from_bytes(bytes) {
        Ok(doc) => check_loaded(&doc),
        Err(error) => check_refusal(&error, bytes),
    }
}

/// The properties of a document a load accepted: it is valid, savable
/// by both checks, and round-trips byte for byte through clean text.
pub fn check_loaded(doc: &Document) -> Result<(), String> {
    for id in doc.tree().descendants() {
        doc.validate_shape(id)
            .map_err(|error| format!("a loaded element breaks the shape rules: {error:?}"))?;
    }
    bounds_hold(doc)?;
    savable_checks_agree(doc, LIMITS)?;
    round_trips(doc)
}

/// `check_savable` and the full build it stands in for give the same
/// answer for `doc` under `limits`' element bound, the same refusal
/// included. (The byte bound is `check_savable`'s alone, counted after
/// either check passes, so it is left out of the comparison.)
pub(crate) fn savable_checks_agree(doc: &Document, limits: Limits) -> Result<(), String> {
    let elements_only = Limits {
        bytes: None,
        ..limits
    };
    let quick = check_savable_under(doc, elements_only);
    let full = check_savable_by_building(doc, limits.elements);
    // Compared as written out: a refusal can carry a `NaN`, which is
    // not equal to itself.
    if format!("{quick:?}") != format!("{full:?}") {
        return Err(format!(
            "check_savable and the full build disagree: {quick:?} against {full:?}"
        ));
    }
    Ok(())
}

/// `doc` saves to clean text that loads to the same document, which
/// saves to the same text.
pub fn round_trips(doc: &Document) -> Result<(), String> {
    let first = save(doc).map_err(|error| format!("a loaded document does not save: {error}"))?;
    saved_text_is_clean(&first)?;
    let again = load(&first).map_err(|error| format!("saved text does not load: {error}\n{first}"))?;
    same_document(doc, &again).map_err(|why| format!("save, then load, changed the document: {why}"))?;
    let second = save(&again).map_err(|error| format!("a reloaded document does not save: {error}"))?;
    if second != first {
        return Err(format!(
            "a second save differs from the first:\n{first}\n---\n{second}"
        ));
    }
    Ok(())
}

/// Whether `c` may never stand raw in a saved file (R23): a control —
/// C0, DEL or C1 — a line or paragraph separator, or a bidi control.
/// The line feeds of the layout are the caller's to allow.
pub fn never_raw_in_a_saved_file(c: char) -> bool {
    c.is_control() || matches!(c, '\u{2028}' | '\u{2029}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

/// A saved text holds no character [`never_raw_in_a_saved_file`] names
/// but the line feeds that end its lines, never a carriage return, and
/// ends with exactly one line feed.
pub fn saved_text_is_clean(text: &str) -> Result<(), String> {
    if let Some((at, c)) = text
        .char_indices()
        .find(|(_, c)| *c != '\n' && never_raw_in_a_saved_file(*c))
    {
        return Err(format!(
            "the saved text holds U+{:04X} raw at byte {at}: {:?}",
            u32::from(c),
            &text[at.saturating_sub(40)..(at + 40).min(text.len())]
        ));
    }
    if !text.ends_with('\n') || text.ends_with("\n\n") {
        return Err("the saved text does not end with exactly one line feed".to_string());
    }
    Ok(())
}

/// A message's characters that a terminal, a dialog or an IPC line
/// could act on: a control, a line or paragraph separator, a bidi
/// control. `Some` names the first.
pub fn raw_in_a_message(message: &str) -> Option<char> {
    message.chars().find(|c| never_raw_in_a_saved_file(*c))
}

/// The properties of a refusal of `bytes`: a clean message, a bounded
/// excerpt, a node context that agrees with the path, a path that
/// resolves in the input and a line inside the node it names.
pub fn check_refusal(error: &LoadError, bytes: &[u8]) -> Result<(), String> {
    let shown = error.to_string();
    if let Some(c) = raw_in_a_message(&shown) {
        return Err(format!("a refusal shows U+{:04X} raw: {shown:?}", u32::from(c)));
    }
    if let Some(excerpt) = error.node.as_ref().and_then(|node| node.excerpt.as_deref()) {
        let length =
            excerpt_length(excerpt).ok_or_else(|| format!("an excerpt is not escaped text: {excerpt:?}"))?;
        if length > EXCERPT_BYTES {
            return Err(format!("an excerpt holds {length} bytes of text: {excerpt:?}"));
        }
    }
    let in_node = node_of_path(&error.path);
    if in_node != error.node.as_ref().map(|node| node.index) {
        return Err(format!(
            "the node context {:?} does not agree with the path `{}`: {:?}",
            error.node, error.path, error.kind
        ));
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Ok(());
    };
    // A text serde_json cannot read as a value (a syntax error, a number
    // out of `f64`'s range, nesting past its limit) has no tree to
    // resolve a path in; the loader's own index is what reads it.
    let Ok(value) = serde_json::from_str::<Spelled>(text) else {
        return Ok(());
    };
    if !path_resolves(&value, &error.path) {
        return Err(format!("the path of `{error}` does not resolve in the input"));
    }
    if let (Some(node), Some(position)) = (&error.node, error.position) {
        if let Some(&(first, last)) = node_lines(text).as_ref().and_then(|lines| lines.get(node.index)) {
            if position.line < first || position.line > last {
                return Err(format!(
                    "`{error}` is placed on line {}, outside its node's lines {first}..={last}",
                    position.line
                ));
            }
        }
    }
    Ok(())
}

/// How many bytes of the node's text an excerpt holds, its escapes read
/// back and the `…` that marks a cut left out; `None` when it is not the
/// content of a JSON string.
pub fn excerpt_length(excerpt: &str) -> Option<usize> {
    let text: String = serde_json::from_str(&format!("\"{excerpt}\"")).ok()?;
    Some(text.strip_suffix('…').unwrap_or(&text).len())
}

/// `i` when `path` starts with `nodes[i]`.
pub fn node_of_path(path: &str) -> Option<usize> {
    let rest = path.strip_prefix("nodes[")?;
    let end = rest.find(']')?;
    rest[..end].parse().ok()
}

/// One step of a JSON path: a key or a position.
#[derive(Clone, Debug, PartialEq)]
pub enum Step {
    /// A key of an object.
    Key(String),
    /// A position in an array.
    Index(usize),
}

/// `path` read as the loader writes one: a plain key after a `.` (none
/// before the first), a quoted key or a position in brackets. `None`
/// when it is not one.
pub fn parse_path(path: &str) -> Option<Vec<Step>> {
    let mut steps = Vec::new();
    let mut rest = path;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix('.') {
            if steps.is_empty() {
                return None;
            }
            let end = after.find(['.', '[']).unwrap_or(after.len());
            if end == 0 {
                return None;
            }
            steps.push(Step::Key(after[..end].to_string()));
            rest = &after[end..];
        } else if let Some(after) = rest.strip_prefix('[') {
            if after.starts_with('"') {
                // A quoted key: a JSON string, then `]`.
                let mut reader = serde_json::Deserializer::from_str(after).into_iter::<String>();
                let key = reader.next()?.ok()?;
                rest = after[reader.byte_offset()..].strip_prefix(']')?;
                steps.push(Step::Key(key));
            } else {
                let end = after.find(']')?;
                steps.push(Step::Index(after[..end].parse().ok()?));
                rest = &after[end + 1..];
            }
        } else {
            if !steps.is_empty() {
                return None;
            }
            let end = rest.find(['.', '[']).unwrap_or(rest.len());
            steps.push(Step::Key(rest[..end].to_string()));
            rest = &rest[end..];
        }
    }
    Some(steps)
}

/// Whether `path` names a value `value` holds; the empty path names the
/// whole of it.
///
/// A key written twice in one object is followed into both: a refusal
/// may name a place inside the first, which the reader met before it met
/// the second (a `serde_json::Value` would hold the last alone, and the
/// first finding of the load_raw target was this harness saying such a
/// path did not resolve).
pub fn path_resolves(value: &Spelled, path: &str) -> bool {
    let Some(steps) = parse_path(path) else {
        return false;
    };
    let mut frontier = vec![value];
    for step in &steps {
        let mut next = Vec::new();
        for at in frontier {
            match (step, at) {
                (Step::Key(key), Spelled::Object(members)) => next.extend(
                    members
                        .iter()
                        .filter(|(member, _)| member == key)
                        .map(|(_, inner)| inner),
                ),
                (Step::Index(index), Spelled::Array(items)) => next.extend(items.get(*index)),
                _ => {}
            }
        }
        if next.is_empty() {
            return false;
        }
        frontier = next;
    }
    true
}

/// A JSON value as its text spells it: an object keeps every member in
/// order, a key written twice included, and a scalar is only that. What
/// [`path_resolves`] walks.
#[derive(Debug)]
pub enum Spelled {
    /// An object's members, in text order.
    Object(Vec<(String, Spelled)>),
    /// An array's items.
    Array(Vec<Spelled>),
    /// A string, number, boolean or `null`.
    Scalar,
}

impl<'de> serde::Deserialize<'de> for Spelled {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Reader;
        impl<'de> serde::de::Visitor<'de> for Reader {
            type Value = Spelled;

            fn expecting(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("a JSON value")
            }

            fn visit_bool<E>(self, _: bool) -> Result<Spelled, E> {
                Ok(Spelled::Scalar)
            }

            fn visit_i64<E>(self, _: i64) -> Result<Spelled, E> {
                Ok(Spelled::Scalar)
            }

            fn visit_u64<E>(self, _: u64) -> Result<Spelled, E> {
                Ok(Spelled::Scalar)
            }

            fn visit_f64<E>(self, _: f64) -> Result<Spelled, E> {
                Ok(Spelled::Scalar)
            }

            fn visit_str<E>(self, _: &str) -> Result<Spelled, E> {
                Ok(Spelled::Scalar)
            }

            fn visit_unit<E>(self) -> Result<Spelled, E> {
                Ok(Spelled::Scalar)
            }

            fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Spelled, A::Error> {
                let mut items = Vec::new();
                while let Some(item) = seq.next_element()? {
                    items.push(item);
                }
                Ok(Spelled::Array(items))
            }

            fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Spelled, A::Error> {
                let mut members = Vec::new();
                while let Some(member) = map.next_entry::<String, Spelled>()? {
                    members.push(member);
                }
                Ok(Spelled::Object(members))
            }
        }
        deserializer.deserialize_any(Reader)
    }
}

/// The first and last line of each node of `text`, in list order, or
/// `None` when its `nodes` cannot be read as a list of values.
pub fn node_lines(text: &str) -> Option<Vec<(u32, u32)>> {
    #[derive(serde::Deserialize)]
    struct Top<'a> {
        #[serde(borrow, default)]
        nodes: Vec<&'a serde_json::value::RawValue>,
    }
    let top: Top<'_> = serde_json::from_str(text).ok()?;
    // The nodes are in text order, so one pass over the text counts the
    // lines to every boundary: counting from the start for each node was
    // quadratic, and held the load_structured target to one input in
    // several seconds on a 4,096-node chain.
    let bytes = text.as_bytes();
    let mut line = 1u32;
    let mut counted = 0usize;
    let mut line_at = |offset: usize| {
        let offset = offset.min(bytes.len());
        if offset >= counted {
            let newlines = bytes[counted..offset].iter().filter(|&&b| b == b'\n').count();
            line = line.saturating_add(u32::try_from(newlines).unwrap_or(u32::MAX));
            counted = offset;
        }
        line
    };
    top.nodes
        .iter()
        .map(|raw| {
            let start = crate::document::format::context::offset_in(text, raw.get())?;
            let first = line_at(start);
            Some((first, line_at(start + raw.get().len())))
        })
        .collect()
}

/// Every number `doc` holds is inside its bound. Which bound a field
/// has is read from `format/schema.md`'s table of bounds, not from the
/// loader's calls, so a field the loader forgot to check is found
/// (TEST_CONVENTIONS §T13, shape 6); the bound itself is the
/// `bounds::check_*` predicate the table names. `Err` names the element
/// and the field.
pub fn bounds_hold(doc: &Document) -> Result<(), String> {
    let definitions = doc.definitions();
    for (count, max, what) in [
        (definitions.verbs.len(), bounds::MAX_VERBS, "verbs"),
        (definitions.edits.len(), bounds::MAX_EDITS, "edits"),
        (
            definitions.presentations.len(),
            bounds::MAX_PRESENTATIONS,
            "presentations",
        ),
        (doc.palettes().count(), bounds::MAX_PALETTES, "palettes"),
        (
            doc.theme().variables.len(),
            bounds::MAX_THEME_VARIABLES,
            "theme variables",
        ),
        (
            doc.theme().variants.len(),
            bounds::MAX_THEME_VARIANTS,
            "theme variants",
        ),
    ] {
        named(what, bounds::check_count(count, max))?;
    }
    for id in doc.tree().descendants() {
        let Some(element) = doc.get(id) else {
            continue;
        };
        element_bounds(element).map_err(|why| format!("{:?} element {id}: {why}", element.kind()))?;
    }
    Ok(())
}

/// The bounds of one element's own fields.
fn element_bounds(element: &DocElement) -> Result<(), String> {
    match element {
        DocElement::Map(_) | DocElement::Theme(_) | DocElement::Palette(_) => Ok(()),
        DocElement::Defaults(defaults) => {
            node_style(&defaults.node)?;
            text_style(&defaults.section)?;
            frame_style(&defaults.frame)?;
            ring_style(&defaults.ring)?;
            flow_style(&defaults.connector)?;
            flow_style(&defaults.link)?;
            label_style(&defaults.label)?;
            portal_end_style(&defaults.portal_end)?;
            text_style(&defaults.span)
        }
        DocElement::Node(node) => {
            named("id", bounds::check_node_ident(u64::from(node.ident.0)))?;
            named("channel", bounds::check_channel(node.channel))?;
            coordinates("position", &[node.position.x, node.position.y])?;
            if let Some(size) = node.min_size {
                extents("min_size", &[size.x, size.y])?;
            }
            node_style(&node.style)?;
            zoom(node.zoom)?;
            named("opacity", bounds::check_unit_interval(node.opacity))
        }
        DocElement::Section(section) => {
            named("channel", bounds::check_channel(section.channel))?;
            coordinates("offset", &[section.offset.x, section.offset.y])?;
            if let Some(size) = section.min_size {
                extents("min_size", &[size.x, size.y])?;
            }
            text_style(&section.text)
        }
        DocElement::Span(span) => text_style(&span.style),
        DocElement::Frame(frame) => frame_style(&frame.style),
        DocElement::Ring(ring) => {
            named("channel", bounds::check_channel(ring.channel))?;
            ring_style(&ring.style)
        }
        DocElement::Connector(connector) => path(&connector.path),
        DocElement::Link(link) => path(&link.path),
        DocElement::CapStart(cap) | DocElement::CapEnd(cap) => text_style(&cap.text),
        DocElement::Label(label) => label_style(&label.style),
        DocElement::PortalEnd(end) => portal_end_style(&end.style),
    }
}

/// `result`, its error named after the field.
fn named<T>(field: &str, result: Result<T, BoundError>) -> Result<(), String> {
    result.map(|_| ()).map_err(|error| format!("`{field}`: {error}"))
}

/// Canvas coordinates: finite, magnitude at most 1e9.
fn coordinates(field: &str, values: &[f32]) -> Result<(), String> {
    values
        .iter()
        .try_for_each(|value| named(field, bounds::check_canvas_coord(*value)))
}

/// Parts of a `min_size`: canvas extents.
fn extents(field: &str, values: &[f32]) -> Result<(), String> {
    values
        .iter()
        .try_for_each(|value| named(field, bounds::check_extent(*value)))
}

fn zoom(window: Option<ZoomWindow>) -> Result<(), String> {
    match window {
        Some(window) => named("zoom", bounds::check_zoom_window(window.min, window.max)),
        None => Ok(()),
    }
}

fn fit(mode: &Option<FitMode>) -> Result<(), String> {
    match mode {
        Some(FitMode::Fit { min, max }) => named("fit", bounds::check_fit(*min, *max)),
        _ => Ok(()),
    }
}

fn text_style(style: &TextStyle) -> Result<(), String> {
    match style.size {
        Some(FontSize::Pt(points)) => named("size", bounds::check_font_size_pt(points))?,
        Some(FontSize::Rel(fraction)) => named("size", bounds::check_rel_size(fraction))?,
        None => {}
    }
    match style.weight {
        Some(weight) => named("weight", bounds::check_weight(weight.get())),
        None => Ok(()),
    }
}

fn node_style(style: &NodeStyle) -> Result<(), String> {
    if let Some(Shape::Rectangle { radius }) = style.shape {
        named("shape.radius", bounds::check_percent(radius.get()))?;
    }
    if let Some(fade) = style.fill_fade {
        named("fill_fade", bounds::check_non_negative(fade))?;
    }
    text_style(&style.text)
}

fn frame_style(style: &FrameStyle) -> Result<(), String> {
    if let Some(start) = style.start {
        named("start", bounds::check_unit_interval(start.at))?;
    }
    fit(&style.fit)?;
    for (field, value) in [("gap", style.gap), ("inset", style.inset)] {
        if let Some(value) = value {
            named(field, bounds::check_non_negative(value))?;
        }
    }
    text_style(&style.text)
}

fn ring_style(style: &RingStyle) -> Result<(), String> {
    fit(&style.fit)?;
    text_style(&style.text)
}

fn flow_style(style: &FlowStyle) -> Result<(), String> {
    fit(&style.fit)?;
    if let Some(spacing) = style.spacing {
        named("spacing", bounds::check_non_negative(spacing))?;
    }
    match style.size {
        Some(size) => named("size", bounds::check_font_size_pt(size)),
        None => Ok(()),
    }
}

fn path(path: &PathData) -> Result<(), String> {
    flow_style(&path.flow)?;
    named(
        "control_points",
        bounds::check_control_points(path.control_points.len()),
    )?;
    for point in &path.control_points {
        coordinates("control_points", &[point.x, point.y])?;
    }
    zoom(path.zoom)
}

fn label_style(style: &LabelStyle) -> Result<(), String> {
    if let Some(t) = style.t {
        named("t", bounds::check_unit_interval(t))?;
    }
    if let Some(offset) = style.offset {
        coordinates("offset", &[offset])?;
    }
    text_style(&style.text)
}

fn portal_end_style(style: &PortalEndStyle) -> Result<(), String> {
    if let Some(at) = style.border_t {
        named("border_t", bounds::check_unit_interval(at))?;
    }
    if let Some(offset) = style.offset {
        coordinates("offset", &[offset])?;
    }
    match style.size {
        Some(size) => named("size", bounds::check_font_size_pt(size)),
        None => Ok(()),
    }
}

// ── The structure-aware target ────────────────────────────────────

/// The largest id a node may have (`MAX_NODE_IDENT`), as the model's
/// integer.
const MAX_IDENT: i64 = bounds::MAX_NODE_IDENT as i64;

/// The largest channel (`MAX_CHANNEL`), as the model's integer.
const MAX_CHANNEL: i64 = u32::MAX as i64;

/// The deepest chain, the widest row and the most rows of one kind in a
/// dense node that the load_structured target asks for. Deep enough
/// for a walk that climbs to the root per node, or a link check that
/// walks the chain, to cost the quadratic time a fuzz timeout reports;
/// small enough that one input still runs in milliseconds. The suite's
/// seeded sweep asks for less, since it runs unoptimized.
pub const MAX_GENERATED_ROWS: u32 = 4096;

/// One node of a generated map, as the model sees it.
#[derive(Clone, Debug)]
pub struct NodeSpec {
    /// Its `id`.
    pub id: i64,
    /// Its `parent`, absent on a top-level node.
    pub parent: Option<i64>,
    /// Its `channel`; `0` is written by leaving the key out.
    pub channel: i64,
    /// The `to` of each of its links, in order.
    pub links: Vec<i64>,
    /// Its other keys, each a key and its value as JSON text: valid
    /// content that the model does not judge, but that the loader must
    /// read past (sections and spans of hostile text, frames, a
    /// connector on a node with a parent).
    pub content: Vec<(String, String)>,
    /// Each link's keys after `to` and `kind`, as JSON text that starts
    /// with a `,` or is empty: a label, a portal kind.
    pub link_tails: Vec<String>,
}

/// A generated map: its nodes, the one fault injected into them if any,
/// and the order its keys are written in.
#[derive(Clone, Debug)]
pub struct MapSpec {
    /// The nodes, in list order.
    pub nodes: Vec<NodeSpec>,
    /// What was done to make it wrong.
    pub fault: Fault,
}

/// What was injected into a generated map.
#[derive(Clone, Debug, PartialEq)]
pub enum Fault {
    /// Nothing: the map is valid unless the structure the generator drew
    /// happens not to be, which the model decides.
    None,
    /// The structure was corrupted (ids, parents, channels, the list
    /// order, links), one or two times. The reference model names the
    /// refusal.
    Structure,
    /// One fault the JSON reader or a row check finds inside node
    /// `node`, with the path and the kind it must be refused at.
    Content {
        /// The node it is in.
        node: usize,
        /// The path of the refusal.
        path: String,
        /// The kind's name (`LoadErrorKind`'s variant).
        kind: &'static str,
    },
}

/// Text a span may hold: plain, combining, emoji, every character the
/// saver escapes, quotes, backslashes and the paragraph separators.
const TEXTS: [&str; 12] = [
    "",
    "plain",
    "e\u{301}\u{302}",
    "\u{1F469}\u{200D}\u{1F4BB}",
    "\u{202E}gnp.exe",
    "a\u{2028}b\u{2029}c",
    "\u{85}\u{9B}[2J\u{7F}",
    "\"quoted\\\"",
    "\n\r\t\u{1}",
    "\u{1C}\u{1D}\u{1E}",
    "\u{5d0}\u{5d1} abc",
    "\u{fe0f}\u{20e3}",
];

/// One span, as JSON text.
fn span(choices: &mut crate::util::tests::choices::Choices<'_>, in_ring: bool) -> String {
    let text = serde_json::to_string(choices.pick(&TEXTS)).unwrap_or_default();
    match choices.below(4) {
        1 if !in_ring => format!(r#"{{"text": {text}, "size": 12.5}}"#),
        2 => format!(r#"{{"text": {text}, "weight": "bold", "italic": true}}"#),
        3 => format!(r#"{{"text": {text}, "link": {{"url": "https://example.com"}}}}"#),
        _ => format!(r#"{{"text": {text}}}"#),
    }
}

/// `count` spans, as a JSON array.
fn spans(choices: &mut crate::util::tests::choices::Choices<'_>, count: u32, in_ring: bool) -> String {
    let items: Vec<String> = (0..count).map(|_| span(choices, in_ring)).collect();
    format!("[{}]", items.join(", "))
}

/// A node's sections: `count` of them in ascending channel order, each
/// empty or holding spans.
fn sections(choices: &mut crate::util::tests::choices::Choices<'_>, count: u32) -> String {
    let mut channel = 0;
    let items: Vec<String> = (0..count)
        .map(|_| {
            channel += choices.below(3);
            let spans = match choices.below(3) {
                0 => String::new(),
                n => format!(r#""spans": {}"#, spans(choices, n, false)),
            };
            let key = if channel == 0 {
                String::new()
            } else {
                format!(r#""channel": {channel}"#)
            };
            let body: Vec<String> = [key, spans].into_iter().filter(|part| !part.is_empty()).collect();
            format!("{{{}}}", body.join(", "))
        })
        .collect();
    format!("[{}]", items.join(", "))
}

/// A tree of `n` nodes in pre-order: each node's parent an earlier one
/// or none, siblings' channels ascending, ids scrambled out of list
/// order. The structure the B2 review's `random_tree` drew.
fn random_tree(choices: &mut crate::util::tests::choices::Choices<'_>, n: usize) -> Vec<NodeSpec> {
    let mut kids: Vec<Vec<usize>> = vec![Vec::new(); n + 1];
    let mut roots = Vec::new();
    for k in 1..=n {
        if k > 1 && !choices.one_in(5) {
            kids[1 + choices.index(k - 1)].push(k);
        } else {
            roots.push(k);
        }
    }
    // Ids scrambled, so that they say nothing about the structure.
    let mut ids: Vec<i64> = (1..=n as i64).collect();
    for k in (1..ids.len()).rev() {
        let j = choices.index(k + 1);
        ids.swap(k, j);
    }
    let id_of = |k: usize| ids[k - 1] * 3 + 1;
    let channels = |choices: &mut crate::util::tests::choices::Choices<'_>, row: &[usize]| {
        let mut channel = 0;
        row.iter()
            .map(|_| {
                if choices.one_in(3) {
                    channel += i64::from(choices.below(3));
                }
                channel
            })
            .collect::<Vec<_>>()
    };
    let mut out = Vec::with_capacity(n);
    let mut stack: Vec<(usize, Option<usize>, i64)> = Vec::new();
    let root_channels = channels(choices, &roots);
    for (root, channel) in roots.iter().zip(root_channels).rev() {
        stack.push((*root, None, channel));
    }
    while let Some((k, parent, channel)) = stack.pop() {
        out.push(NodeSpec {
            id: id_of(k),
            parent: parent.map(id_of),
            channel,
            links: Vec::new(),
            content: Vec::new(),
            link_tails: Vec::new(),
        });
        let row_channels = channels(choices, &kids[k]);
        for (child, channel) in kids[k].iter().zip(row_channels).rev() {
            stack.push((*child, Some(k), channel));
        }
    }
    out
}

/// A plain node: `id`, under `parent`, no content.
fn plain(id: i64, parent: Option<i64>) -> NodeSpec {
    NodeSpec {
        id,
        parent,
        channel: 0,
        links: Vec::new(),
        content: Vec::new(),
        link_tails: Vec::new(),
    }
}

/// How many rows a dense shape has: up to 64, or — one time in eight,
/// so that most inputs stay fast — up to `most`. Encoded as two choices,
/// `7` then the count, for a large one.
fn rows(choices: &mut crate::util::tests::choices::Choices<'_>, most: u32) -> u32 {
    if choices.one_in(8) {
        choices.below(most)
    } else {
        choices.below(most.min(64))
    }
}

/// The map `choices` describe, no row of it longer than `most`. The
/// first choice is its shape:
///
/// 0. a random tree of up to 24 nodes, with random content and links;
/// 1. a chain as deep as the next choices say (`rows`), every node
///    but the root linking somewhere (the root, its parent, the bottom);
/// 2. a fan: one root and that many children;
/// 3. one node holding rows of empty sections, of empty rings and of
///    portal links to a second node, as many as the next choices say;
/// 4. a random tree as large as the next choices say (sized as `rows` sizes one, up
///    to 1024 nodes).
///
/// Then the fault: none, a structural corruption, or one content fault.
pub fn generate(choices: &mut crate::util::tests::choices::Choices<'_>, most: u32) -> MapSpec {
    let mut nodes = match choices.below(5) {
        1 => {
            let depth = 1 + rows(choices, most) as i64;
            let target = choices.below(3);
            (1..=depth)
                .map(|id| {
                    let mut node = plain(id, (id > 1).then_some(id - 1));
                    if id > 1 {
                        node.links.push(match target {
                            0 => 1,
                            1 => id - 1,
                            _ if id == depth => 1,
                            _ => depth,
                        });
                        node.link_tails.push(String::new());
                    }
                    node
                })
                .collect()
        }
        2 => {
            let width = 1 + rows(choices, most) as i64;
            let mut channel = 0;
            let mut out = vec![plain(1, None)];
            for id in 2..=width {
                channel += i64::from(choices.below(2));
                out.push(NodeSpec {
                    channel,
                    ..plain(id, Some(1))
                });
            }
            out
        }
        3 => {
            let mut dense = plain(1, None);
            let empty = |count: u32| vec!["{}"; count as usize].join(", ");
            let sections = rows(choices, most);
            let rings = rows(choices, most);
            let portals = rows(choices, most);
            dense
                .content
                .push(("sections".to_string(), format!("[{}]", empty(sections))));
            dense
                .content
                .push(("frame".to_string(), format!(r#"{{"rings": [{}]}}"#, empty(rings))));
            for _ in 0..portals {
                dense.links.push(2);
                dense.link_tails.push(r#", "kind": "portal""#.to_string());
            }
            vec![dense, plain(2, None)]
        }
        shape => {
            let n = 1 + if shape == 4 {
                rows(choices, most.min(1024)) as usize
            } else {
                choices.index(24)
            };
            let mut nodes = random_tree(choices, n);
            add_content(choices, &mut nodes);
            nodes
        }
    };
    let fault = match choices.below(4) {
        1 | 2 => {
            for _ in 0..1 + choices.below(2) {
                corrupt(choices, &mut nodes);
            }
            Fault::Structure
        }
        3 => inject(choices, &mut nodes),
        _ => Fault::None,
    };
    MapSpec { nodes, fault }
}

/// Random valid content for each node: sections, a frame of rings, a
/// connector on a node with a parent, links to other nodes.
fn add_content(choices: &mut crate::util::tests::choices::Choices<'_>, nodes: &mut [NodeSpec]) {
    let ids: Vec<i64> = nodes.iter().map(|node| node.id).collect();
    for node in nodes.iter_mut() {
        if choices.one_in(2) {
            let count = choices.below(4);
            node.content
                .push(("sections".to_string(), sections(choices, count)));
        }
        if choices.one_in(4) {
            let rings: Vec<String> = (0..choices.below(3))
                .map(|_| {
                    let count = choices.below(3);
                    format!(r#"{{"spans": {}}}"#, spans(choices, count, true))
                })
                .collect();
            node.content.push((
                "frame".to_string(),
                format!(r#"{{"rings": [{}]}}"#, rings.join(", ")),
            ));
        }
        if node.parent.is_some() && choices.one_in(4) {
            let count = choices.below(2);
            node.content.push((
                "connector".to_string(),
                format!(r#"{{"spans": {}}}"#, spans(choices, count, false)),
            ));
        }
        for _ in 0..choices.below(3) {
            let to = *choices.pick(&ids);
            if to == node.id {
                continue;
            }
            node.links.push(to);
            node.link_tails.push(match choices.below(3) {
                1 => r#", "kind": "portal""#.to_string(),
                2 => {
                    let count = 1 + choices.below(2);
                    format!(r#", "label": {{"spans": {}}}"#, spans(choices, count, false))
                }
                _ => String::new(),
            });
        }
    }
}

/// One structural corruption: the B2 review's `corrupt`, eleven ways.
fn corrupt(choices: &mut crate::util::tests::choices::Choices<'_>, nodes: &mut Vec<NodeSpec>) {
    let len = nodes.len();
    if len == 0 {
        return;
    }
    let any = |choices: &mut crate::util::tests::choices::Choices<'_>| choices.index(len);
    match choices.below(11) {
        0 => {
            let (a, b) = (any(choices), any(choices));
            nodes.swap(a, b);
        }
        1 => {
            let (k, p) = (any(choices), any(choices));
            nodes[k].parent = Some(nodes[p].id);
        }
        2 => {
            let (a, b) = (any(choices), any(choices));
            nodes[a].id = nodes[b].id;
        }
        3 => {
            let k = any(choices);
            nodes[k].channel = i64::from(choices.below(4));
        }
        4 if len > 1 => {
            let k = any(choices);
            nodes.remove(k);
        }
        5 => {
            let k = any(choices);
            nodes[k].parent = None;
        }
        6 => {
            let k = any(choices);
            nodes[k].parent = Some(*choices.pick(&[0, MAX_IDENT + 1, 1 << 32, 999_999]));
        }
        7 => {
            let k = any(choices);
            nodes[k].id = *choices.pick(&[0, MAX_IDENT, MAX_IDENT + 1]);
        }
        8 => {
            let k = any(choices);
            nodes[k].channel = *choices.pick(&[MAX_CHANNEL, MAX_CHANNEL + 1]);
        }
        9 => {
            let k = any(choices);
            let to = *choices.pick(&[0, 999_999, nodes[k].id, MAX_IDENT + 1]);
            nodes[k].links.push(to);
            nodes[k].link_tails.push(String::new());
        }
        _ => {
            if len > 1 {
                let node = nodes.remove(any(choices));
                let at = choices.index(nodes.len() + 1);
                nodes.insert(at, node);
            }
        }
    }
}

/// One fault inside one node, which the map's other content cannot
/// mask: the map is otherwise as generated, and a generated map's
/// structure is valid. The refusal's path and kind are known.
fn inject(choices: &mut crate::util::tests::choices::Choices<'_>, nodes: &mut [NodeSpec]) -> Fault {
    if nodes.is_empty() {
        return Fault::None;
    }
    let k = choices.index(nodes.len());
    let node = &mut nodes[k];
    let at = |rest: &str| format!("nodes[{k}]{rest}");
    match choices.below(5) {
        1 => {
            node.content
                .push(("position".to_string(), r#""left""#.to_string()));
            Fault::Content {
                node: k,
                path: at(".position"),
                kind: "Invalid",
            }
        }
        2 => {
            // Two sections, the second's channel below the first's.
            node.content.retain(|(key, _)| key != "sections");
            node.content
                .push(("sections".to_string(), r#"[{"channel": 2}, {}]"#.to_string()));
            Fault::Content {
                node: k,
                path: at(".sections[1]"),
                kind: "SiblingsOutOfOrder",
            }
        }
        3 => {
            node.content.retain(|(key, _)| key != "sections");
            node.content.push((
                "sections".to_string(),
                r#"[{"spans": [{"text": "a"}, {"text": "b", "hue": 3}]}]"#.to_string(),
            ));
            Fault::Content {
                node: k,
                path: at(".sections[0].spans[1].hue"),
                kind: "UnknownKey",
            }
        }
        4 => {
            node.content.push(("opacity".to_string(), "1.5".to_string()));
            Fault::Content {
                node: k,
                path: at(".opacity"),
                kind: "Bound",
            }
        }
        _ => {
            node.content
                .push(("bogus".to_string(), "[1, {\"x\": null}]".to_string()));
            Fault::Content {
                node: k,
                path: at(".bogus"),
                kind: "UnknownKey",
            }
        }
    }
}

/// `spec` as the text of a map, its keys in the order `order` names: `0`
/// is the schema's order, and any other value rotates and reverses the
/// keys of the map and of each node, differently per node. Key order is
/// never significant to the loader (§7.2).
pub fn render(spec: &MapSpec, order: u32) -> String {
    let nodes: Vec<String> = spec
        .nodes
        .iter()
        .enumerate()
        .map(|(i, node)| {
            let mut keys: Vec<String> = vec![format!(r#""id": {}"#, node.id)];
            if let Some(parent) = node.parent {
                keys.push(format!(r#""parent": {parent}"#));
            }
            if node.channel != 0 {
                keys.push(format!(r#""channel": {}"#, node.channel));
            }
            if !node.content.iter().any(|(key, _)| key == "position") {
                keys.push(r#""position": [0.0, 0.0]"#.to_string());
            }
            for (key, value) in &node.content {
                keys.push(format!(r#""{key}": {value}"#));
            }
            if !node.links.is_empty() {
                let links: Vec<String> = node
                    .links
                    .iter()
                    .zip(&node.link_tails)
                    .map(|(to, tail)| {
                        let kind = if tail.contains(r#""kind""#) {
                            ""
                        } else {
                            r#", "kind": "line""#
                        };
                        format!(r#"{{"to": {to}{kind}{tail}}}"#)
                    })
                    .collect();
                keys.push(format!(r#""links": [{}]"#, links.join(", ")));
            }
            permute(&mut keys, order, i);
            format!("{{{}}}", keys.join(", "))
        })
        .collect();
    let mut top = vec![
        r#""format": 1"#.to_string(),
        r#""name": "generated""#.to_string(),
        format!("\"nodes\": [\n{}\n]", nodes.join(",\n")),
    ];
    permute(&mut top, order, usize::MAX);
    format!("{{{}}}\n", top.join(",\n"))
}

/// `keys` in the order `order` names for the object at `place`.
fn permute(keys: &mut [String], order: u32, place: usize) {
    if order == 0 || keys.is_empty() {
        return;
    }
    let turn = (order as usize).wrapping_add(place) % keys.len();
    keys.rotate_left(turn);
    if order % 2 == 1 {
        keys.reverse();
    }
}

/// Whether `nodes` spell a valid map, judged without streaming: every
/// id unique and in range, every parent present, siblings in channel
/// order, the list the pre-order of the tree it describes (so no cycle,
/// no orphan), no connector on a top-level node, and every link to
/// another node. A port of the B2 review's `chain_ref.py` `oracle()`,
/// sharing no code with the loader.
pub fn oracle(nodes: &[NodeSpec]) -> bool {
    use std::collections::{HashMap, HashSet};
    let ids: Vec<i64> = nodes.iter().map(|node| node.id).collect();
    let unique: HashSet<i64> = ids.iter().copied().collect();
    if unique.len() != ids.len() || ids.iter().any(|id| !(1..=MAX_IDENT).contains(id)) {
        return false;
    }
    if nodes
        .iter()
        .any(|node| !(0..=MAX_CHANNEL).contains(&node.channel))
    {
        return false;
    }
    let place: HashMap<i64, usize> = ids.iter().enumerate().map(|(k, id)| (*id, k)).collect();
    let mut kids: HashMap<Option<i64>, Vec<i64>> = HashMap::new();
    for node in nodes {
        if node.parent.is_some_and(|parent| !place.contains_key(&parent)) {
            return false;
        }
        if node.parent.is_none() && node.content.iter().any(|(key, _)| key == "connector") {
            return false;
        }
        kids.entry(node.parent).or_default().push(node.id);
    }
    for row in kids.values() {
        let channels: Vec<i64> = row.iter().map(|id| nodes[place[id]].channel).collect();
        if channels.windows(2).any(|pair| pair[0] > pair[1]) {
            return false;
        }
    }
    let mut order = Vec::with_capacity(nodes.len());
    let mut stack: Vec<i64> = kids.get(&None).cloned().unwrap_or_default();
    stack.reverse();
    let mut seen = HashSet::new();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            return false;
        }
        order.push(id);
        let mut row = kids.get(&Some(id)).cloned().unwrap_or_default();
        row.reverse();
        stack.extend(row);
    }
    if order != ids {
        return false;
    }
    nodes.iter().all(|node| {
        node.links
            .iter()
            .all(|to| place.contains_key(to) && *to != node.id)
    })
}

/// The refusal the streaming check of design §7.1 must give for
/// `nodes`, or `None` when they load: the B2 review's port of
/// `chain_ref.py`'s `check()`. `(path, kind, the ids a
/// ParentNotOnChain offers)`.
pub fn reference(nodes: &[NodeSpec]) -> Option<(String, &'static str, Vec<u32>)> {
    use std::collections::HashMap;
    let mut idents: HashMap<i64, usize> = HashMap::new();
    // The chain: each level's id and channel.
    let mut chain: Vec<(i64, i64)> = Vec::new();
    for (i, node) in nodes.iter().enumerate() {
        let at = format!("nodes[{i}]");
        if !(1..=MAX_IDENT).contains(&node.id) {
            return Some((at + ".id", "Bound", vec![]));
        }
        if idents.contains_key(&node.id) {
            return Some((at + ".id", "DuplicateId", vec![]));
        }
        if !(0..=MAX_CHANNEL).contains(&node.channel) {
            return Some((at + ".channel", "Bound", vec![]));
        }
        let depth = match node.parent {
            None => 0,
            Some(parent) => {
                if !(1..=MAX_IDENT).contains(&parent) {
                    return Some((at + ".parent", "Bound", vec![]));
                }
                if parent == node.id {
                    return Some((at + ".parent", "ParentIsSelf", vec![]));
                }
                let Some(&level) = idents.get(&parent) else {
                    return Some((at + ".parent", "ParentNotEarlier", vec![]));
                };
                if chain.get(level).map(|(id, _)| *id) != Some(parent) {
                    let offered = chain.iter().rev().take(8).map(|(id, _)| *id as u32).collect();
                    return Some((at + ".parent", "ParentNotOnChain", offered));
                }
                level + 1
            }
        };
        if chain
            .get(depth)
            .is_some_and(|(_, channel)| node.channel < *channel)
        {
            // A channel left at its default is refused at the element
            // (R17.6): the key is not in the text.
            let path = if node.channel == 0 { at } else { at + ".channel" };
            return Some((path, "SiblingsOutOfOrder", vec![]));
        }
        chain.truncate(depth);
        if node.parent.is_none() && node.content.iter().any(|(key, _)| key == "connector") {
            return Some((at + ".connector", "ConnectorOnRoot", vec![]));
        }
        for (j, to) in node.links.iter().enumerate() {
            if !(1..=MAX_IDENT).contains(to) {
                return Some((format!("nodes[{i}].links[{j}].to"), "Bound", vec![]));
            }
        }
        chain.push((node.id, node.channel));
        idents.insert(node.id, depth);
    }
    for (i, node) in nodes.iter().enumerate() {
        for (j, to) in node.links.iter().enumerate() {
            let at = format!("nodes[{i}].links[{j}].to");
            if !idents.contains_key(to) {
                return Some((at, "DanglingLink", vec![]));
            }
            if *to == node.id {
                return Some((at, "SelfLink", vec![]));
            }
        }
    }
    None
}

/// The name of `kind`'s variant.
pub fn kind_name(kind: &LoadErrorKind) -> String {
    let debug = format!("{kind:?}");
    debug
        .split([' ', '{', '('])
        .next()
        .unwrap_or_default()
        .to_string()
}

/// Every property of the map `bytes` choose (the load_structured
/// target's whole body): those of [`check_load_bytes`] on its text; the
/// loader's verdict against [`oracle`], and its refusal against
/// [`reference()`] or the injected fault; the same refusal in a second key
/// order (C.9.14, single fault); and, when it loads, `check_savable`
/// against the full build on the document with up to two values a
/// buggy writer could leave, under this target's limits and a small
/// element bound.
pub fn check_structured(bytes: &[u8]) -> Result<(), String> {
    let mut choices = crate::util::tests::choices::Choices::new(bytes);
    check_generated(&mut choices, MAX_GENERATED_ROWS).map(|_| ())
}

/// [`check_structured`] for a map drawn from `choices`, whatever their
/// source: the fuzzer's bytes, or a seeded sweep in the suite. `Ok`
/// says what the input was and how it ended (`"none: loaded"`,
/// `"structure: DuplicateId"`, `"content: Bound"`), so that a sweep can
/// show it reached every case.
pub fn check_generated(
    choices: &mut crate::util::tests::choices::Choices<'_>,
    most: u32,
) -> Result<String, String> {
    let spec = generate(choices, most);
    let order = 1 + choices.below(u32::MAX - 1);
    let text = render(&spec, 0);
    let result = load(&text);
    let context = |why: String| format!("{why}\n{text}");
    match &result {
        Ok(doc) => check_loaded(doc).map_err(context)?,
        Err(error) => check_refusal(error, text.as_bytes()).map_err(context)?,
    }
    match &spec.fault {
        Fault::Content { path, kind, .. } => match &result {
            Err(error) if error.path == *path && kind_name(&error.kind) == *kind => {}
            other => {
                return Err(context(format!(
                    "the fault injected at `{path}` ({kind}) was answered {:?}",
                    other.as_ref().err()
                )))
            }
        },
        Fault::None | Fault::Structure => {
            let accepted = oracle(&spec.nodes);
            if result.is_ok() != accepted {
                return Err(context(format!(
                    "the loader {} what the oracle {}: {:?}",
                    if result.is_ok() { "accepts" } else { "refuses" },
                    if accepted { "accepts" } else { "refuses" },
                    result.as_ref().err()
                )));
            }
            match (&result, reference(&spec.nodes)) {
                (Ok(_), None) => {}
                (Err(error), Some((path, kind, offered))) => {
                    let same_offer = match &error.kind {
                        LoadErrorKind::ParentNotOnChain { expected, .. } => *expected == offered,
                        _ => true,
                    };
                    if error.path != path || kind_name(&error.kind) != kind || !same_offer {
                        return Err(context(format!(
                            "refused `{error}` ({:?}); the reference refuses at `{path}` ({kind}, {offered:?})",
                            error.kind
                        )));
                    }
                }
                (got, expected) => {
                    return Err(context(format!(
                        "the loader answered {:?}; the reference {expected:?}",
                        got.as_ref().err()
                    )))
                }
            }
        }
    }
    // C.9.14: one fault, any key order, the same refusal. Every input
    // here has at most one fault the reader finds in text order; the
    // structural checks run in a fixed order whatever the text's.
    let reordered = render(&spec, order);
    let again = load(&reordered);
    let verdict = |result: &Result<Document, LoadError>| {
        result
            .as_ref()
            .err()
            .map(|error| (error.path.clone(), kind_name(&error.kind)))
    };
    if verdict(&result) != verdict(&again) {
        return Err(context(format!(
            "reordering the keys changed the verdict from {:?} to {:?}:\n{reordered}",
            verdict(&result),
            verdict(&again)
        )));
    }
    let fault = match spec.fault {
        Fault::None => "none",
        Fault::Structure => "structure",
        Fault::Content { .. } => "content",
    };
    match result {
        Ok(doc) => {
            tampered_documents_agree(choices, doc)?;
            Ok(format!("{fault}: loaded"))
        }
        Err(error) => Ok(format!("{fault}: {}", kind_name(&error.kind))),
    }
}

/// `doc` with up to two values a buggy writer could leave — a second
/// node with one id, a fresh id, a position or opacity out of bounds, a
/// size in the wrong unit, a link to itself or to no node — each drawn
/// from `choices`; then `check_savable` must agree with the full build,
/// under this target's limits and under an element bound small enough
/// to be reached, and a document it passes must round-trip.
fn tampered_documents_agree(
    choices: &mut crate::util::tests::choices::Choices<'_>,
    doc: Document,
) -> Result<(), String> {
    use crate::document::ids::{get, IdentIndex, NodeIdent};
    let mut tree = doc.tree().clone();
    let ids: Vec<indextree::NodeId> = tree.descendants().collect();
    let nodes: Vec<indextree::NodeId> = ids
        .iter()
        .copied()
        .filter(|id| matches!(get(&tree, *id), Some(DocElement::Node(_))))
        .collect();
    if nodes.is_empty() {
        return Ok(());
    }
    for _ in 0..choices.below(3) {
        let target = *choices.pick(&ids);
        let other = *choices.pick(&nodes);
        let other_ident = match get(&tree, other) {
            Some(DocElement::Node(node)) => node.ident,
            _ => NodeIdent(1),
        };
        let fault = choices.below(7);
        let _ = tree.update(target, |element| match (fault, element) {
            (0, DocElement::Node(node)) => node.ident = other_ident,
            (1, DocElement::Node(node)) => node.ident = NodeIdent(bounds::MAX_NODE_IDENT),
            (2, DocElement::Node(node)) => node.position = glam::Vec2::new(f32::NAN, 0.0),
            (3, DocElement::Node(node)) => node.opacity = 1.5,
            (4, DocElement::Span(span)) => span.style.size = Some(FontSize::Rel(0.5)),
            (5, DocElement::Link(link)) => link.to = target,
            (6, DocElement::Link(link)) => link.to = other,
            (_, DocElement::Section(section)) => section.offset.y = 2.0e9,
            _ => {}
        });
    }
    // The index a loader would build from this tree: each node under its
    // ident, the later of two with one ident winning.
    let mut highest = 0;
    let index: IdentIndex = nodes
        .iter()
        .filter_map(|id| match get(&tree, *id) {
            Some(DocElement::Node(node)) => {
                highest = highest.max(node.ident.0);
                Some((node.ident, *id))
            }
            _ => None,
        })
        .collect();
    let Ok(tampered) = Document::from_loaded(tree, doc.definitions().clone(), index, highest) else {
        return Ok(());
    };
    savable_checks_agree(&tampered, LIMITS)?;
    let small = Limits {
        bytes: None,
        elements: Some(u64::from(choices.below(64))),
    };
    savable_checks_agree(&tampered, small)?;
    if check_savable_under(&tampered, LIMITS).is_ok() {
        round_trips(&tampered)?;
    }
    Ok(())
}
