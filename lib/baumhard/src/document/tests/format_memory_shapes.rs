// SPDX-License-Identifier: MPL-2.0

//! The maps wasm32's memory limits are derived from, and the model they
//! are held to — shared by `tests/format_memory.rs`, which measures them
//! natively, and the wasm32 probe (`tests/wasm_font_lock/probe.rs`),
//! which measures them in a real WebAssembly engine, under the
//! allocator and the pointer width the browser has.
//!
//! **The model.** A map's peak while it loads, and while it is then
//! saved, is at most
//! `text × WASM32_BYTES_PER_TEXT_BYTE + elements × WASM32_BYTES_PER_ELEMENT`
//! ([`bound`]), where `text` is the longer of the text read and the text
//! written. Each shape here is the densest the format allows of one
//! kind of content, so a map of any mix is no denser per byte or per
//! element than the worst of them.
//!
//! **Measured at the worst point.** Every container doubles as it
//! grows, so what it holds per entry depends on where in its growth it
//! is. The tree's arena is the largest; each element shape is measured
//! with just past [`ARENA_POINT`]` - 1` elements, right after the arena
//! has doubled, where it holds twice what it needs. The text shapes are
//! measured just past [`TEXT_POINT`]` - 1` bytes, for the same reason
//! about the text being written.

use crate::document::format::{load, save, WASM32_BYTES_PER_ELEMENT, WASM32_BYTES_PER_TEXT_BYTE};
use crate::document::ids::Document;

/// The element shapes: each node of a fan holds one kind of row in
/// bulk, or nothing; and the definitions.
pub const ELEMENT_SHAPES: [&str; 10] = [
    // Every node under node 1: a node and a connector each.
    "fan", // Each node under the one before: the chain as long as the map.
    "chain",
    // Every node at the top: one element each, the most nodes per
    // element — the ident index's worst case.
    "tops",     // A short label on each: a section and a span.
    "labels",   // 200 empty sections `{}` on each: 3 bytes an element.
    "sections", // A frame of 200 empty rings `{}` on each.
    "rings",
    // 50 portal links on each: a link and two ends, three elements in
    // 25 bytes.
    "portals", // 50 line links on each.
    "lines",   // One section of 100 empty spans `{"text":""}` on each.
    "spans",
    // `MAX_VERBS` verbs, each an id and an empty name: a definition is
    // counted as an element is. Edits and presentations are the same
    // two strings.
    "verbs",
];

/// The text shapes: few elements, a long text, or a header that is all
/// text and no element.
pub const TEXT_SHAPES: [&str; 3] = [
    // One node, one span, one long text.
    "text",  // A theme of 4096 variables in as many variants as the length takes.
    "theme", // One palette of many groups.
    "palettes",
];

/// The element count each element shape is measured just past: one
/// more than a power of two, the arena's worst point.
pub const ARENA_POINT: u64 = (1 << 17) + 1;

/// The text length each text shape is measured just past.
pub const TEXT_POINT: usize = (1 << 23) + 1;

/// The bound the model puts on a map of `text` bytes and `elements`
/// elements.
pub fn bound(text: u64, elements: u64) -> u64 {
    text * WASM32_BYTES_PER_TEXT_BYTE + elements * WASM32_BYTES_PER_ELEMENT
}

/// One node of `shape`, listed with `id`.
fn node(shape: &str, id: u32) -> String {
    let parent = match (shape, id) {
        (_, 1) | ("tops", _) => String::new(),
        ("chain", _) => format!(r#","parent":{}"#, id - 1),
        _ => r#","parent":1"#.to_string(),
    };
    // A link may not name its own node: node 1's name node 2.
    let other = if id == 1 { 2 } else { 1 };
    let content = match shape {
        "labels" => r#","sections":[{"spans":[{"text":"label"}]}]"#.to_string(),
        "sections" => format!(r#","sections":[{}]"#, vec!["{}"; 200].join(",")),
        "rings" => format!(r#","frame":{{"rings":[{}]}}"#, vec!["{}"; 200].join(",")),
        "portals" => format!(
            r#","links":[{}]"#,
            vec![format!(r#"{{"to":{other},"kind":"portal"}}"#); 50].join(",")
        ),
        "lines" => format!(
            r#","links":[{}]"#,
            vec![format!(r#"{{"to":{other},"kind":"line"}}"#); 50].join(",")
        ),
        "spans" => format!(
            r#","sections":[{{"spans":[{}]}}]"#,
            vec![r#"{"text":""}"#; 100].join(",")
        ),
        _ => String::new(),
    };
    format!(r#"{{"id":{id}{parent},"position":[0,0]{content}}}"#)
}

/// The text of an element shape with `n` nodes, written as densely as
/// the format allows (compact, no whitespace), in one allocation of the
/// size it needs.
pub fn element_text(shape: &str, n: u32) -> String {
    let size: usize = 64 + (1..=n).map(|id| node(shape, id).len() + 1).sum::<usize>();
    let mut text = String::with_capacity(size);
    text.push_str(r#"{"format":1,"name":"m","nodes":["#);
    for id in 1..=n {
        if id > 1 {
            text.push(',');
        }
        text.push_str(&node(shape, id));
    }
    text.push_str("]}");
    text
}

/// The text of a text shape about `length` bytes long.
pub fn text_text(shape: &str, length: usize) -> String {
    let one = r#"{"id":1,"position":[0,0]}"#;
    let mut text = String::with_capacity(length + 256);
    match shape {
        "text" => {
            text.push_str(
                r#"{"format":1,"name":"m","nodes":[{"id":1,"position":[0,0],"sections":[{"spans":[{"text":""#,
            );
            let fill = length.saturating_sub(text.len() + 8);
            text.extend(std::iter::repeat_n('a', fill));
            text.push_str(r#""}]}]}]}"#);
        }
        "theme" => {
            // Variables `v0`.. in every variant; variants `a0`, `a1`, ...
            // until the text is long enough.
            const VARIABLES: usize = 4096;
            text.push_str(r#"{"format":1,"name":"m","theme":{"active":"a0","variants":{"#);
            let mut variant = 0;
            while text.len() < length {
                if variant > 0 {
                    text.push(',');
                }
                text.push_str(&format!(r#""a{variant}":{{"#));
                for v in 0..VARIABLES {
                    if v > 0 {
                        text.push(',');
                    }
                    text.push_str(&format!(r##""v{v}":"#000000""##));
                }
                text.push('}');
                variant += 1;
            }
            text.push_str(&format!("}}}},\"nodes\":[{one}]}}"));
        }
        "verbs" => {
            text.push_str(r#"{"format":1,"name":"m","verbs":["#);
            let mut verb = 0;
            while text.len() < length && verb < crate::document::bounds::MAX_VERBS {
                if verb > 0 {
                    text.push(',');
                }
                text.push_str(&format!(r#"{{"id":"{verb}","name":""}}"#));
                verb += 1;
            }
            text.push_str(&format!("],\"nodes\":[{one}]}}"));
        }
        _ => {
            // "palettes": one palette of groups naming one variable.
            text.push_str(
                r##"{"format":1,"name":"m","theme":{"active":"a","variants":{"a":{"a":"#000000"}}},"palettes":{"p":["##,
            );
            let mut group = 0;
            while text.len() < length {
                if group > 0 {
                    text.push(',');
                }
                text.push_str(r#"{"fill":"var(a)","frame":"var(a)","text":"var(a)"}"#);
                group += 1;
            }
            text.push_str(&format!("]}},\"nodes\":[{one}]}}"));
        }
    }
    text
}

/// How many elements `doc` holds, its root included, and how many
/// definitions: what the builder counts against `MAX_ELEMENTS`.
pub fn elements_of(doc: &Document) -> u64 {
    let definitions = doc.definitions();
    (doc.tree().descendants().count()
        + definitions.edits.len()
        + definitions.presentations.len()
        + definitions.verbs.len()) as u64
}

/// The text of `shape` at its measuring point: an element shape with
/// just past [`ARENA_POINT`]` - 1` elements, a text shape just past
/// [`TEXT_POINT`]` - 1` bytes, and the most verbs a map may define.
pub fn measured_text(shape: &str) -> String {
    if TEXT_SHAPES.contains(&shape) {
        return text_text(shape, TEXT_POINT);
    }
    if shape == "verbs" {
        // As many as a map may define.
        return text_text(shape, 4 << 20);
    }
    // Elements grow linearly with nodes: two small loads give the line.
    let count = |n: u32| {
        load(&element_text(shape, n))
            .map(|doc| elements_of(&doc))
            .unwrap_or_else(|error| panic!("{shape}: {error}"))
    };
    let (four, eight) = (count(4), count(8));
    let per_node = (eight - four) / 4;
    let fixed = four - 4 * per_node;
    let n = (ARENA_POINT.saturating_sub(fixed)).div_ceil(per_node);
    element_text(shape, u32::try_from(n).unwrap_or(u32::MAX))
}

/// The granularity [`WASM32_BYTES_PER_ELEMENT`] is pinned at.
pub const ELEMENT_STEP: u64 = 16;

/// The two costs `measured` shows: per byte, the most any text shape
/// costs per byte of its longer text, all of its peak charged to its
/// text; per element, the most any shape costs per element beyond its
/// text at the pinned cost per byte.
pub fn derive(measured: &[Measured]) -> (f64, f64) {
    let per_byte = measured
        .iter()
        .filter(|m| TEXT_SHAPES.contains(&m.shape.as_str()))
        .map(|m| (m.load as f64 / m.text_in as f64).max(m.both as f64 / m.text() as f64))
        .fold(0.0, f64::max);
    let beyond = |peak: u64, text: u64| peak as f64 - (text * WASM32_BYTES_PER_TEXT_BYTE) as f64;
    let per_element = measured
        .iter()
        .map(|m| beyond(m.load, m.text_in).max(beyond(m.both, m.text())) / m.elements as f64)
        .fold(0.0, f64::max);
    (per_byte, per_element)
}

/// Whether the pinned costs are the costs `measured` shows, rounded up:
/// the cost per byte to a whole byte, the cost per element to
/// [`ELEMENT_STEP`]. Every shape must have been measured.
pub fn check_pins(measured: &[Measured]) -> Result<(), String> {
    for shape in ELEMENT_SHAPES.iter().chain(&TEXT_SHAPES) {
        if !measured.iter().any(|m| m.shape == *shape) {
            return Err(format!("{shape} was not measured"));
        }
    }
    let (per_byte, per_element) = derive(measured);
    let a = WASM32_BYTES_PER_TEXT_BYTE as f64;
    let b = WASM32_BYTES_PER_ELEMENT as f64;
    if !(per_byte <= a && a - per_byte < 1.0) {
        return Err(format!(
            "a map costs {per_byte:.2} bytes per byte of text; WASM32_BYTES_PER_TEXT_BYTE is {a}: pin it again"
        ));
    }
    if !(per_element <= b && b - per_element < ELEMENT_STEP as f64) {
        return Err(format!(
            "a map costs {per_element:.1} bytes per element beyond its text; WASM32_BYTES_PER_ELEMENT is {b}: pin it again"
        ));
    }
    Ok(())
}

/// What one measurement found.
#[derive(Clone, Debug)]
pub struct Measured {
    /// The shape.
    pub shape: String,
    /// The text read.
    pub text_in: u64,
    /// The text written.
    pub text_out: u64,
    /// The elements the document holds.
    pub elements: u64,
    /// The peak while it loads, the text included.
    pub load: u64,
    /// The peak while it loads and is then saved.
    pub both: u64,
}

impl Measured {
    /// The line a measuring process reports.
    pub fn line(&self) -> String {
        format!(
            "MEASURED {} {} {} {} {} {}",
            self.shape, self.text_in, self.text_out, self.elements, self.load, self.both
        )
    }

    /// The measurement a [`Self::line`] reports.
    pub fn parse(line: &str) -> Option<Measured> {
        let mut words = line.strip_prefix("MEASURED ")?.split(' ');
        let shape = words.next()?.to_string();
        let mut number = || words.next()?.parse::<u64>().ok();
        Some(Measured {
            shape,
            text_in: number()?,
            text_out: number()?,
            elements: number()?,
            load: number()?,
            both: number()?,
        })
    }

    /// The longer of the text read and the text written.
    pub fn text(&self) -> u64 {
        self.text_in.max(self.text_out)
    }

    /// Whether the measurement is within the model: the load's peak
    /// under the bound of its text, and the save's under the bound of
    /// the longer text.
    pub fn check(&self) -> Result<(), String> {
        let load_bound = bound(self.text_in, self.elements);
        let both_bound = bound(self.text(), self.elements);
        if self.load > load_bound || self.both > both_bound {
            return Err(format!(
                "{}: a peak of {} bytes loading and {} saving, past the model's {} and {} \
                 ({} bytes of text read, {} written, {} elements): WASM32_BYTES_PER_TEXT_BYTE \
                 and WASM32_BYTES_PER_ELEMENT no longer bound what a map costs",
                self.shape,
                self.load,
                self.both,
                load_bound,
                both_bound,
                self.text_in,
                self.text_out,
                self.elements
            ));
        }
        Ok(())
    }
}

/// Load and save `text`, reporting the peaks `peak` reads after each
/// against `base`, the heap before the text was made. `peak` reads a
/// high-water mark: the allocator's on native, the module's memory on
/// wasm32.
pub fn measure(shape: &str, text: String, base: u64, peak: impl Fn() -> u64) -> Measured {
    let text_in = text.len() as u64;
    let doc = load(&text).unwrap_or_else(|error| panic!("{shape}: {error}"));
    let load_peak = peak().saturating_sub(base);
    drop(text);
    let saved = save(&doc).unwrap_or_else(|error| panic!("{shape}: {error}"));
    let both = peak().saturating_sub(base);
    Measured {
        shape: shape.to_string(),
        text_in,
        text_out: saved.len() as u64,
        elements: elements_of(&doc),
        load: load_peak,
        both,
    }
}
