// SPDX-License-Identifier: MPL-2.0

//! The ornament fallback (V1-B11, #297): the compiled ornament faces hold
//! exactly the glyphs they are subset to, the regeneration script and the
//! built-in styles agree with that list, and a span whose face lacks one
//! of them draws it in its compiled face though a host face that holds it
//! comes first in cosmic-text's own fallback order.
//!
//! The faces' character maps are read with `skrifa`, from the compiled
//! bytes, not through the shaper whose fallback is under test.

use cosmic_text::skrifa::bitmap::BitmapStrikes;
use cosmic_text::skrifa::instance::{LocationRef, Size};
use cosmic_text::skrifa::{FontRef, MetadataProvider};

use crate::document::defaults::BUILT_IN;
use crate::font::fallback::{FALLBACK_WEIGHTS, ORNAMENT_FALLBACK};
use crate::font::fonts::{self, acquire_font_system_write, AppFont, FONT_SOURCES};

/// What `Mandala Ornaments` is subset to, and so holds: the ornaments of
/// the demo maps (`✦ ❦ ♪ ∙`, the connectors' `· ~`, a ring's `+`), the
/// built-in connector, link and portal-end glyphs (`·`, `◈`), and the
/// space an ornament's copy is parted by. `scripts/ornament_fonts.sh`
/// subsets the face to exactly these.
pub const ORNAMENT_CODE_POINTS: [char; 9] = [' ', '+', '~', '·', '∙', '◈', '♪', '✦', '❦'];

/// What `Mandala Emoji` is subset to: the frames demo's `🌸`.
pub const EMOJI_CODE_POINTS: [char; 1] = ['🌸'];

/// The compiled bytes of `font`'s face.
fn face_data(font: AppFont) -> &'static [u8] {
    match FONT_SOURCES.get(&font) {
        Some(cosmic_text::fontdb::Source::Binary(data)) => {
            let data: &'static [u8] = (**data).as_ref();
            data
        }
        _ => panic!("precondition: {font:?} is a compiled face held in memory"),
    }
}

/// Every code point `data`'s character map maps to a glyph, sorted.
fn mapped(data: &[u8]) -> Vec<char> {
    let font = FontRef::new(data).expect("a compiled face parses");
    let mut chars: Vec<char> = font
        .charmap()
        .mappings()
        .filter(|(_, glyph)| glyph.to_u32() != 0)
        .filter_map(|(code_point, _)| char::from_u32(code_point))
        .collect();
    chars.sort_unstable();
    chars.dedup();
    chars
}

/// `chars`, sorted.
fn sorted(chars: &[char]) -> Vec<char> {
    let mut chars = chars.to_vec();
    chars.sort_unstable();
    chars
}

#[test]
fn test_each_ornament_face_holds_exactly_its_listed_code_points() {
    do_each_ornament_face_holds_exactly_its_listed_code_points();
}

/// Each listed code point maps in its compiled face, to a glyph that
/// draws something — an outline in `Mandala Ornaments` (but the space), a
/// color bitmap in `Mandala Emoji` — and the faces map nothing else, so a
/// code point added to a list without regenerating the face, or a face
/// regenerated with another, is red here. The reader is held to a face
/// that lacks the ornaments first.
pub fn do_each_ornament_face_holds_exactly_its_listed_code_points() {
    let galatia = mapped(face_data(AppFont::GalatiaSIL));
    assert!(
        galatia.contains(&'a') && !galatia.contains(&'✦'),
        "control: the reader finds a text face's letters and not an ornament it lacks"
    );
    let ornaments = face_data(AppFont::MandalaOrnaments);
    assert_eq!(
        mapped(ornaments),
        sorted(&ORNAMENT_CODE_POINTS),
        "Mandala Ornaments' character map"
    );
    let font = FontRef::new(ornaments).expect("a compiled face parses");
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let inked = |c: char| {
        let glyph = font.charmap().map(c).expect("mapped");
        metrics
            .bounds(glyph)
            .is_some_and(|b| b.x_max > b.x_min && b.y_max > b.y_min)
    };
    assert!(!inked(' '), "control: the space has no ink");
    for c in ORNAMENT_CODE_POINTS.iter().filter(|c| **c != ' ') {
        assert!(inked(*c), "Mandala Ornaments draws no outline for {c:?}");
    }
    let emoji = face_data(AppFont::MandalaEmoji);
    assert_eq!(
        mapped(emoji),
        sorted(&EMOJI_CODE_POINTS),
        "Mandala Emoji's character map"
    );
    let font = FontRef::new(emoji).expect("a compiled face parses");
    let strikes = BitmapStrikes::new(&font);
    for c in EMOJI_CODE_POINTS {
        let glyph = font.charmap().map(c).expect("mapped");
        assert!(
            strikes.iter().any(|strike| strike.get(glyph).is_some()),
            "Mandala Emoji holds no bitmap for {c:?}"
        );
    }
}

/// The code points of the `NAME=U+XXXX,…` line of `script`, sorted.
fn script_list(script: &str, name: &str) -> Vec<char> {
    let prefix = format!("{name}=");
    let line = script
        .lines()
        .find(|line| line.starts_with(&prefix))
        .unwrap_or_else(|| panic!("the script assigns {name}"));
    let mut chars: Vec<char> = line[prefix.len()..]
        .split(',')
        .map(|item| {
            let hex = item.trim().strip_prefix("U+").expect("a U+XXXX code point");
            char::from_u32(u32::from_str_radix(hex, 16).expect("hexadecimal")).expect("a scalar value")
        })
        .collect();
    chars.sort_unstable();
    chars
}

#[test]
fn test_the_regeneration_script_subsets_exactly_the_listed_code_points() {
    do_the_regeneration_script_subsets_exactly_the_listed_code_points();
}

/// `scripts/ornament_fonts.sh` subsets each face to the list this module
/// holds it to, so regenerating a face keeps every glyph the list (and so
/// the demo maps and the built-in styles) needs. The parser is held to a
/// planted line first.
pub fn do_the_regeneration_script_subsets_exactly_the_listed_code_points() {
    assert_eq!(
        script_list("X=1\nPLANTED=U+0041,U+1F338\n", "PLANTED"),
        vec!['A', '🌸'],
        "control: a planted list is read"
    );
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../scripts/ornament_fonts.sh");
    let script = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"));
    assert_eq!(script_list(&script, "ORNAMENTS"), sorted(&ORNAMENT_CODE_POINTS));
    assert_eq!(script_list(&script, "EMOJI"), sorted(&EMOJI_CODE_POINTS));
}

#[test]
fn test_every_built_in_ornament_is_an_ornament_face_glyph() {
    do_every_built_in_ornament_is_an_ornament_face_glyph();
}

/// The glyphs the built-in styles draw a connector, a link and a portal
/// end with are in `Mandala Ornaments`, so a map that sets none of them
/// draws them in a compiled face on every install. A built-in changed to
/// a glyph the face lacks is red until the face is regenerated with it.
pub fn do_every_built_in_ornament_is_an_ornament_face_glyph() {
    let held = mapped(face_data(AppFont::MandalaOrnaments));
    let built_in = [
        ("connector", BUILT_IN.connector.content),
        ("link", BUILT_IN.link.content),
        ("portal end", BUILT_IN.portal_end.content),
    ];
    for (what, content) in built_in {
        assert!(
            !content.is_empty(),
            "precondition: the built-in {what} draws a glyph"
        );
        for c in content.chars() {
            assert!(
                held.contains(&c),
                "the built-in {what}'s {c:?} is not in Mandala Ornaments"
            );
        }
    }
}

#[test]
fn test_each_ornament_fallback_names_its_faces_family_at_every_weight() {
    do_each_ornament_fallback_names_its_faces_family_at_every_weight();
}

/// Each [`ORNAMENT_FALLBACK`] entry names the family its compiled face
/// stores, which is what cosmic-text matches a fallback family by, and
/// the face is registered at every one of [`FALLBACK_WEIGHTS`], which a
/// fallback family is matched at.
pub fn do_each_ornament_fallback_names_its_faces_family_at_every_weight() {
    assert_eq!(
        ORNAMENT_FALLBACK.len(),
        2,
        "precondition: the fallback lists the ornament and the emoji face"
    );
    fonts::init();
    // Read under the guard and assert once it is released, so that a
    // failure does not poison the lock for the tests beside it.
    let pins: Vec<Option<fonts::PinnedFamily>> = {
        let fs = acquire_font_system_write("ornament_tests::do_*");
        ORNAMENT_FALLBACK
            .iter()
            .map(|(font, _)| fonts::pinned_family(&fs, *font))
            .collect()
    };
    for ((font, family), pin) in ORNAMENT_FALLBACK.iter().zip(pins) {
        let pin = pin.expect("a compiled family");
        assert_eq!(pin.name, *family, "{font:?}'s family");
        assert_eq!(pin.weights, FALLBACK_WEIGHTS.to_vec(), "{family}'s weights");
        assert_eq!(
            pin.faces.len(),
            FALLBACK_WEIGHTS.len(),
            "{family}: one face a weight"
        );
    }
}

/// For an ornament face, a letter of the script whose fallback family the
/// test build gives a copy of that face (`fonts::ORNAMENT_DECOYS`): a run
/// of it tries that family before the common list. None for the emoji:
/// Unicode puts a line-break opportunity on each side of one (UAX #14
/// class ID), so cosmic-text shapes it as a word of its own, with no
/// script, which reaches the common list alone.
#[cfg(test)]
const SCRIPT_RUNS: [(AppFont, char); 1] = [(AppFont::MandalaOrnaments, 'ب')];

/// What `text`, shaped alone at `weight` in the family `family`, is
/// drawn in: each glyph's face, PostScript name and glyph id. Test builds
/// only, as its one caller is.
#[cfg(test)]
fn drawn(
    fs: &mut cosmic_text::FontSystem,
    text: &str,
    family: &str,
    weight: u16,
) -> Vec<(cosmic_text::fontdb::ID, String, u16)> {
    let attrs = cosmic_text::Attrs::new()
        .family(cosmic_text::Family::Name(family))
        .weight(cosmic_text::Weight(weight));
    let mut buffer = cosmic_text::Buffer::new(fs, cosmic_text::Metrics::new(16.0, 20.0));
    buffer.set_text(text, &attrs, cosmic_text::Shaping::Advanced, None);
    buffer.shape_until_scroll(fs, false);
    let glyphs: Vec<(cosmic_text::fontdb::ID, u16)> = buffer
        .layout_runs()
        .flat_map(|run| {
            run.glyphs
                .iter()
                .map(|g| (g.font_id, g.glyph_id))
                .collect::<Vec<_>>()
        })
        .collect();
    glyphs
        .into_iter()
        .map(|(id, glyph)| {
            let name = fs
                .db()
                .face(id)
                .map_or_else(String::new, |f| f.post_script_name.clone());
            (id, name, glyph)
        })
        .collect()
}

/// #297's decoy test: a span pinned to a compiled family that lacks an
/// ornament draws it in its compiled ornament face, though
/// a host face that holds it is reached first in cosmic-text's own order.
///
/// The test build registers copies of the ornament faces as host faces
/// (`fonts::ORNAMENT_DECOYS`), in common fallback families of
/// cosmic-text's at 400 and 700 (the ornaments in `Noto Sans`, the emoji
/// in `Noto Color Emoji`), and the ornaments in the family a run of Arabic
/// tries first (`Noto Sans Arabic`). Galatia SIL lacks the five the issue
/// names, and each is shaped in it at 400 and at 700 (its bold face),
/// alone, and each ornament between two Arabic letters ([`SCRIPT_RUNS`]),
/// where the script's own fallback list comes before the common one.
/// Plain, not a `do_*()` pair: it reads the test build's decoys.
///
/// Controls (the PR lists what each drew): with `OrnamentsFirst` listing
/// the platform's families alone, red on every case; with the faces at
/// their own weight only (no `register_every_weight`), red at 700 alone;
/// with the families first in the common list only, red inside the
/// Arabic runs alone, in the `Noto Sans Arabic` decoy.
#[test]
fn test_a_pinned_span_draws_each_ornament_in_its_compiled_face_before_a_host_face() {
    fonts::init();
    let galatia = mapped(face_data(AppFont::GalatiaSIL));
    let galatia_bold = mapped(face_data(AppFont::GalatiaSILBold));
    let cases: Vec<(char, AppFont)> = ORNAMENT_CODE_POINTS
        .iter()
        .map(|c| (*c, AppFont::MandalaOrnaments))
        .chain(EMOJI_CODE_POINTS.iter().map(|c| (*c, AppFont::MandalaEmoji)))
        .filter(|(c, _)| !galatia.contains(c) && !galatia_bold.contains(c))
        .collect();
    for c in ['✦', '❦', '♪', '∙', '🌸'] {
        assert!(
            cases.iter().any(|(case, _)| *case == c),
            "precondition: Galatia SIL lacks {c:?}, so it falls back"
        );
    }
    for (_, letter) in SCRIPT_RUNS {
        assert!(
            !galatia.contains(&letter) && !galatia_bold.contains(&letter),
            "precondition: Galatia SIL lacks {letter:?}, so its run falls back through its script's list"
        );
    }
    // Read under the guard and assert once it is released, so that a
    // failure does not poison the lock for the tests beside it.
    let (decoys, results) = {
        let mut fs = acquire_font_system_write("ornament_tests::pinned");
        let pin = fonts::pinned_family(&fs, AppFont::GalatiaSIL).expect("a compiled family");
        let names: Vec<String> = fs.db().faces().map(|f| f.post_script_name.clone()).collect();
        let decoys: Vec<(String, bool)> = fonts::ORNAMENT_DECOYS
            .iter()
            .map(|(font, family, weight)| {
                let name = fonts::ornament_decoy_name(*font, family, *weight);
                let found = names.contains(&name);
                (name, found)
            })
            .collect();
        let mut results = Vec::new();
        for (c, font) in &cases {
            let mut texts = vec![(c.to_string(), 0)];
            for (_, letter) in SCRIPT_RUNS.iter().filter(|(face, _)| face == font) {
                texts.push((format!("{letter}{c}{letter}"), 1));
            }
            for weight in [400, 700] {
                let weight = pin.weight_for(weight);
                for (text, at) in &texts {
                    let glyphs = drawn(&mut fs, text, &pin.name, weight);
                    results.push((
                        *c,
                        *font,
                        weight,
                        text.clone(),
                        glyphs.get(*at).cloned(),
                        glyphs.len(),
                    ));
                }
            }
        }
        (decoys, results)
    };
    for (name, found) in &decoys {
        assert!(found, "precondition: the host decoy {name} is registered");
    }
    let mut faults = Vec::new();
    for (c, font, weight, text, glyph, count) in results {
        let ids = &fonts::COMPILED_FONT_ID_MAP[&font];
        match glyph {
            Some((id, _, glyph)) if ids.contains(&id) && glyph != 0 => {}
            Some((_, name, glyph)) => faults.push(format!(
                "{c:?} in {text:?} at {weight}: drawn in {name} (glyph {glyph})"
            )),
            None => faults.push(format!("{c:?} in {text:?} at {weight}: {count} glyphs")),
        }
    }
    assert!(
        faults.is_empty(),
        "{} ornaments not drawn in their compiled face:\n{}",
        faults.len(),
        faults.join("\n")
    );
}
