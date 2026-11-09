// SPDX-License-Identifier: MPL-2.0

//! `format::color`, `format::hex` and `format::font`: the file's color
//! reader, hex spelling and face reader, for callers outside a load (the
//! app's paste and copy, the console). Each is held to the loader
//! itself: a spelling reads to the value a map carrying it loads with,
//! and one the loader refuses reads to nothing.

use crate::document::element::DocElement;
use crate::document::format::{self, load};
use crate::document::ids::Document;
use crate::document::values::{ColorRef, Rgba};

/// A map whose one span is colored `color`, as the file spells it: the
/// loaded document, or the loader's refusal.
fn map_colored(color: &str) -> Result<Document, format::LoadError> {
    load(&format!(
        r##"{{"format":1,"name":"c","theme":{{"active":"d","variants":{{"d":{{"accent":"#d4a24c","fg":"#e0e0e0"}}}}}},"nodes":[{{"id":1,"position":[0.0,0.0],"sections":[{{"spans":[{{"text":"x","color":"{color}"}}]}}]}}]}}"##
    ))
}

/// The color the loaded map's span carries.
fn loaded_color(doc: &Document) -> Option<ColorRef> {
    doc.tree().descendants().find_map(|id| match doc.get(id) {
        Some(DocElement::Span(span)) => span.style.color,
        _ => None,
    })
}

/// **`format::color` reads what the loader reads, and refuses what it
/// refuses.** For each spelling, the color it reads is the one a map
/// spelling its span's color so loads with, and a spelling the loader
/// refuses reads to `None` — so the two cannot drift. Controls: both
/// lists are non-empty, every accepted spelling does load, and every
/// refused one really is refused by the loader.
#[test]
fn test_format_color_reads_what_the_loader_reads() {
    let accepted = [
        "#102030",
        "#10203080",
        "var(accent)",
        "var(fg)",
        "palette(frame)",
        "palette(title)",
    ];
    let refused = [
        "#FFFFFF",
        "#abc",
        "#102030ff",
        "red",
        "var(nope)",
        "palette(nope)",
        "",
        "102030",
    ];
    for text in accepted {
        let doc = map_colored(text).unwrap_or_else(|e| panic!("control: {text:?} loads: {e}"));
        let read = format::color(&doc, text);
        assert!(read.is_some(), "{text:?} reads");
        assert_eq!(read, loaded_color(&doc), "{text:?} reads as it loads");
    }
    let plain = map_colored("#000000").expect("a plain map loads");
    for text in refused {
        assert!(map_colored(text).is_err(), "control: the loader refuses {text:?}");
        assert_eq!(format::color(&plain, text), None, "{text:?} is not a color");
    }
}

/// **`format::hex` writes the file's spelling**: lowercase, six digits
/// for an opaque color and eight otherwise, which loads back to the same
/// color. Control: the alpha digits appear exactly when the color is not
/// opaque.
#[test]
fn test_format_hex_writes_the_spelling_the_loader_reads_back() {
    for rgba in [
        Rgba::new(0xAB, 0x10, 0xFF, 0xFF),
        Rgba::new(0x01, 0x02, 0x03, 0x80),
    ] {
        let text = format::hex(rgba);
        assert_eq!(text, text.to_lowercase());
        assert_eq!(text.len(), if rgba.0[3] == 0xFF { 7 } else { 9 }, "{text}");
        let doc = map_colored(&text).unwrap_or_else(|e| panic!("{text:?} loads: {e}"));
        assert_eq!(
            loaded_color(&doc),
            Some(ColorRef::Rgba(rgba)),
            "{text:?} loads back"
        );
    }
}

/// A map whose one span names the face `family`: the loaded document,
/// or the loader's refusal.
fn map_in_font(family: &str) -> Result<Document, format::LoadError> {
    let family = serde_json::to_string(family).expect("a string serializes");
    load(&format!(
        r#"{{"format":1,"name":"f","nodes":[{{"id":1,"position":[0.0,0.0],"sections":[{{"spans":[{{"text":"x","font":{family}}}]}}]}}]}}"#
    ))
}

/// **`format::font` reads what the loader reads, and refuses what it
/// refuses**, for every family this process has loaded — the compiled
/// ones, and on native the host's, which no file may name (#329) — and
/// for names no face has. Each accepted name reads to the face a map
/// naming it loads with. Controls: some family is accepted, so the loop
/// is not vacuous, and the refused names really are refused by the
/// loader.
#[test]
fn test_format_font_reads_what_the_loader_reads() {
    let mut accepted = 0;
    let families: Vec<String> = crate::font::fonts::loaded_families_iter()
        .map(str::to_string)
        .chain(["", "Nope Sans", "inter"].map(String::from))
        .collect();
    for family in &families {
        let read = format::font(family);
        match map_in_font(family) {
            Ok(doc) => {
                accepted += 1;
                let loaded = doc.tree().descendants().find_map(|id| match doc.get(id) {
                    Some(DocElement::Span(span)) => span.style.font,
                    _ => None,
                });
                assert_eq!(read, loaded, "{family:?} reads as it loads");
                assert!(read.is_some(), "{family:?} loads, so it reads");
                // The loader shares `format::font`, so agreeing with it
                // proves nothing alone: what is read must also save back.
                assert!(format::save(&doc).is_ok(), "{family:?}: a face read saves");
            }
            Err(_) => assert_eq!(read, None, "{family:?} is refused by the loader"),
        }
    }
    assert!(accepted > 0, "control: a compiled family loads");
    assert!(format::font("Nope Sans").is_none() && map_in_font("Nope Sans").is_err());
}
