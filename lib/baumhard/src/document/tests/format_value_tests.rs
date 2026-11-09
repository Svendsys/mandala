// SPDX-License-Identifier: MPL-2.0

//! `format::start`, `edge`, `direction`, `align`, `shape`, `weight`,
//! `line_span` and `fit`: the file's spellings of single values, read
//! from typed text (`format/read.rs`). Each is held to the loader
//! itself: a spelling reads to the value a map carrying it at that key
//! loads with, and one the loader refuses reads to nothing.

use serde_json::Value;

use crate::document::element::DocElement;
use crate::document::format::{self, load};
use crate::document::ids::Document;

/// A one-node map with `value` at `key` of `place` — the node, its
/// section, its span or its frame — and every other key left out.
fn map_with(place: &str, key: &str, value: Value) -> Result<Document, format::LoadError> {
    let mut node = serde_json::json!({
        "id": 1, "position": [0.0, 0.0],
        "sections": [{"spans": [{"text": "x"}]}],
        "frame": {"rings": []}
    });
    match place {
        "node" => node[key] = value,
        "section" => node["sections"][0][key] = value,
        "span" => node["sections"][0]["spans"][0][key] = value,
        _ => node["frame"][key] = value,
    }
    load(&serde_json::json!({"format": 1, "name": "v", "nodes": [node]}).to_string())
}

/// What a person types, as the JSON a file holds for it: JSON as it is,
/// anything else as a string.
fn as_json(text: &str) -> Value {
    serde_json::from_str(text).unwrap_or_else(|_| Value::String(text.to_string()))
}

/// The first element of `doc` that `pick` reads a value from.
fn first<T>(doc: &Document, pick: impl Fn(&DocElement) -> Option<T>) -> Option<T> {
    doc.tree()
        .descendants()
        .find_map(|id| doc.get(id).and_then(&pick))
}

/// The frame's style field `pick` reads, as a [`hold`] reads an element.
fn frame<T>(
    pick: fn(&crate::document::element::FrameStyle) -> Option<T>,
) -> impl Fn(&DocElement) -> Option<T> {
    move |e: &DocElement| match e {
        DocElement::Frame(frame) => pick(&frame.style),
        _ => None,
    }
}

/// One reader against the loader: every text in `texts` reads to what a
/// map holding it at `place`.`key` loads with, and to `None` exactly
/// when the loader refuses it. Returns how many each way, for the
/// controls.
fn hold<T: PartialEq + std::fmt::Debug>(
    place: &str,
    key: &str,
    texts: &[&str],
    read: impl Fn(&str) -> Option<T>,
    loaded: impl Fn(&DocElement) -> Option<T>,
) -> (usize, usize) {
    let (mut accepted, mut refused) = (0, 0);
    for text in texts {
        match map_with(place, key, as_json(text)) {
            Ok(doc) => {
                assert_eq!(
                    read(text),
                    first(&doc, &loaded),
                    "{key} {text:?} reads as it loads"
                );
                assert!(read(text).is_some(), "{key} {text:?} loads, so it reads");
                accepted += 1;
            }
            Err(_) => {
                assert!(read(text).is_none(), "{key} {text:?} is refused by the loader");
                refused += 1;
            }
        }
    }
    (accepted, refused)
}

/// **Each reader reads what the loader reads at its key, and refuses
/// what it refuses**: names, numbers and objects, bounds included — a
/// start's `at` past 1, a weight off the hundreds, a radius past 100, a
/// fit range upside down. Control: every reader meets spellings of both
/// kinds, so neither half passes by finding none.
#[test]
fn test_format_value_readers_read_what_the_loader_reads() {
    let counts = [
        hold(
            "frame",
            "start",
            &[
                "top-left",
                "bottom",
                r#"{"edge": "left", "at": 0.25}"#,
                r#"{"edge": "left", "at": 2}"#,
                "center",
                "Top",
            ],
            format::start,
            frame(|s| s.start),
        ),
        hold(
            "frame",
            "direction",
            &["clockwise", "counterclockwise", "cw", "anticlockwise"],
            format::direction,
            frame(|s| s.direction),
        ),
        hold(
            "frame",
            "fit",
            &[
                "repeat",
                "fit",
                r#"{"once": {"filler": "·"}}"#,
                r#"{"fit": {"min": 0.5, "max": 1.5}}"#,
                r#"{"fit": {"min": 2, "max": 1}}"#,
                "grow",
            ],
            format::fit,
            frame(|s| s.fit.clone()),
        ),
        hold(
            "frame",
            "span",
            &["natural", "whole", "0.5", "1.5", "most"],
            format::line_span,
            frame(|s| s.ring_layout.span),
        ),
        hold(
            "section",
            "align",
            &["left", "center", "right", "middle"],
            format::align,
            |e| match e {
                DocElement::Section(s) => s.align,
                _ => None,
            },
        ),
        hold(
            "node",
            "shape",
            &[
                "ellipse",
                r#"{"rectangle": {"radius": 25}}"#,
                r#"{"rectangle": {"radius": 101}}"#,
                "rectangle",
                "circle",
            ],
            format::shape,
            |e| match e {
                DocElement::Node(n) => n.style.shape,
                _ => None,
            },
        ),
        hold(
            "span",
            "weight",
            &["normal", "bold", "300", "350", "heavy"],
            format::weight,
            |e| match e {
                DocElement::Span(s) => s.style.weight,
                _ => None,
            },
        ),
    ];
    for (accepted, refused) in counts {
        assert!(
            accepted > 1 && refused > 0,
            "control: both kinds met ({accepted}, {refused})"
        );
    }
    let edge = |text| format::start(&format!(r#"{{"edge": "{text}", "at": 0}}"#)).map(|s| s.edge);
    for text in ["top", "right", "bottom", "left", "middle"] {
        assert_eq!(
            format::edge(text),
            edge(text),
            "an edge reads as a start's edge does: {text}"
        );
    }
    assert!(format::edge("top").is_some() && format::edge("middle").is_none());
}
