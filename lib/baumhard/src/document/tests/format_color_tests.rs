// SPDX-License-Identifier: MPL-2.0

//! `format::color` and `format::hex`: the file's color reader and hex
//! spelling, for callers outside a load (the app's paste and copy). Each
//! is held to the loader itself: a spelling reads to the color a map
//! carrying it loads with, and one the loader refuses reads to nothing.

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
