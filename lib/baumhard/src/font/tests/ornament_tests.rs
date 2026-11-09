// SPDX-License-Identifier: MPL-2.0

//! The default face and the fallback (#228, #297): the compiled ornament
//! faces hold exactly the glyphs they are subset to and none the default
//! face holds; the regeneration script and the built-in styles agree with
//! that list; each fallback family names its face; a span whose face
//! lacks an ornament draws it in its ornament face, at every weight; and
//! no letter, digit, space or punctuation mark is ever drawn by an
//! ornament face, whether the span pins a face or not.
//!
//! The faces' character maps are read with `skrifa`, from the compiled
//! bytes, not through the shaper whose fallback is under test.

use cosmic_text::skrifa::bitmap::BitmapStrikes;
use cosmic_text::skrifa::instance::{LocationRef, Size};
use cosmic_text::skrifa::{FontRef, MetadataProvider};

use crate::document::defaults::BUILT_IN;
use crate::font::fallback::{fallback_families, DEFAULT_FACE, FALLBACK_WEIGHTS, ORNAMENT_FALLBACK};
use crate::font::fonts::{self, acquire_font_system_write, AppFont, FONT_SOURCES};

/// What `Mandala Ornaments` is subset to, and so holds: the ornaments of
/// the demo maps (`✦ ❦ ∙`), the built-in portal-end glyph (`◈`), the
/// box-drawing characters of the console's frame (`─ │ ╭ ╮ ╯ ╰`) and of
/// testament's custom sides (`━ ┃`), and the rest of the app's chrome:
/// the console's prompt, cursor, gutter and completion marker
/// (`❯ ▌ ▏ ▸`), the rubber band's dashes (`┄ ┆`) and the `body` presets'
/// `═ ∼ ⋯` — with `∽` and `❮`, the mirrored forms of `∼` and `❯`, which
/// the subsetter keeps for a right-to-left run. The default face holds
/// none of them. `scripts/ornament_fonts.sh` subsets the face to exactly
/// these.
pub const ORNAMENT_CODE_POINTS: [char; 23] = [
    '∙', '∼', '∽', '⋯', '─', '━', '│', '┃', '┄', '┆', '═', '╭', '╮', '╯', '╰', '▌', '▏', '▸', '◈', '✦', '❦',
    '❮', '❯',
];

/// What `Mandala Emoji` is subset to: the frames demo's `🌸`.
pub const EMOJI_CODE_POINTS: [char; 1] = ['🌸'];

/// Text the default face draws, of every kind #228's choice (a) keeps
/// from the ornament faces: letters, digits, the space, punctuation, and
/// the four code points `Mandala Ornaments` held until #228 (` + ~ ·`)
/// and the `♪` it held beside the default face's own.
pub const TEXT: &str = "Lord God 2:16, “free” — + ~ · ♪ (yes)!";

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

/// Whether `c` maps, in the face `data`, to a glyph whose outline has an
/// area.
fn inked(data: &[u8], c: char) -> bool {
    let font = FontRef::new(data).expect("a compiled face parses");
    let metrics = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    font.charmap().map(c).is_some_and(|glyph| {
        metrics
            .bounds(glyph)
            .is_some_and(|b| b.x_max > b.x_min && b.y_max > b.y_min)
    })
}

#[test]
fn test_each_ornament_face_holds_exactly_its_listed_code_points() {
    do_each_ornament_face_holds_exactly_its_listed_code_points();
}

/// Each listed code point maps in its compiled face, to a glyph that
/// draws something — an outline in `Mandala Ornaments`, a color bitmap in
/// `Mandala Emoji` — and the faces map nothing else, so a code point
/// added to a list without regenerating the face, or a face regenerated
/// with another, is red here. The readers are held to a text face first.
pub fn do_each_ornament_face_holds_exactly_its_listed_code_points() {
    let galatia = face_data(AppFont::GalatiaSIL);
    assert!(
        mapped(galatia).contains(&'a') && !mapped(galatia).contains(&'✦'),
        "control: the reader finds a text face's letters and not an ornament it lacks"
    );
    assert!(
        inked(galatia, 'a') && !inked(galatia, ' '),
        "control: a letter has ink and the space has none"
    );
    let ornaments = face_data(AppFont::MandalaOrnaments);
    assert_eq!(
        mapped(ornaments),
        sorted(&ORNAMENT_CODE_POINTS),
        "Mandala Ornaments' character map"
    );
    for c in ORNAMENT_CODE_POINTS {
        assert!(
            inked(ornaments, c),
            "Mandala Ornaments draws no outline for {c:?}"
        );
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

#[test]
fn test_the_ornament_faces_hold_no_code_point_the_default_face_holds() {
    do_the_ornament_faces_hold_no_code_point_the_default_face_holds();
}

/// #228's choice (a), held at the faces: an ornament face holds no code
/// point the default face holds, so whatever order the fallback tries
/// them in, and whatever face a span pins, a letter, digit, space or
/// punctuation mark the default face draws is never drawn by an ornament
/// face. The comparison is held first to two faces that share letters.
pub fn do_the_ornament_faces_hold_no_code_point_the_default_face_holds() {
    let shared =
        |a: &[char], b: &[char]| -> Vec<char> { a.iter().filter(|c| b.contains(c)).copied().collect() };
    let default = mapped(face_data(DEFAULT_FACE.0));
    assert!(
        shared(&mapped(face_data(AppFont::GalatiaSIL)), &default).contains(&'a'),
        "control: two text faces share a letter"
    );
    for c in TEXT.chars() {
        assert!(default.contains(&c), "precondition: the default face holds {c:?}");
    }
    for (font, family) in ORNAMENT_FALLBACK {
        assert_eq!(
            shared(&mapped(face_data(*font)), &default),
            Vec::<char>::new(),
            "{family} holds code points the default face holds"
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
fn test_every_built_in_glyph_is_held_by_a_compiled_fallback_face() {
    do_every_built_in_glyph_is_held_by_a_compiled_fallback_face();
}

/// The glyphs the built-in styles draw a connector, a link and a portal
/// end with are held by the default face (the connector's and the link's
/// `·`) or an ornament face (the portal end's `◈`), so a map that sets
/// none of them draws them in a compiled face on every target. A built-in
/// changed to a glyph neither holds is red until a face holds it.
pub fn do_every_built_in_glyph_is_held_by_a_compiled_fallback_face() {
    let default = mapped(face_data(DEFAULT_FACE.0));
    let ornaments = mapped(face_data(AppFont::MandalaOrnaments));
    let built_in = [
        ("connector", BUILT_IN.connector.content, &default),
        ("link", BUILT_IN.link.content, &default),
        ("portal end", BUILT_IN.portal_end.content, &ornaments),
    ];
    for (what, content, held) in built_in {
        assert!(
            !content.is_empty(),
            "precondition: the built-in {what} draws a glyph"
        );
        for c in content.chars() {
            assert!(held.contains(&c), "the built-in {what}'s {c:?} is not held");
        }
    }
}

#[test]
fn test_each_fallback_names_its_faces_family_at_every_weight() {
    do_each_fallback_names_its_faces_family_at_every_weight();
}

/// Each [`fallback_families`] entry names the family its compiled face
/// stores, which is what cosmic-text matches a fallback family by; each
/// static face is registered at every one of [`FALLBACK_WEIGHTS`], which
/// a fallback family is matched at, and the variable default face spans
/// them all with its axis.
pub fn do_each_fallback_names_its_faces_family_at_every_weight() {
    let families: Vec<_> = fallback_families().collect();
    assert!(
        families.len() >= 3,
        "precondition: the fallback lists the default face and the ornament faces"
    );
    fonts::init();
    // Read under the guard and assert once it is released, so that a
    // failure does not poison the lock for the tests beside it.
    let pins: Vec<Option<fonts::PinnedFamily>> = {
        let fs = acquire_font_system_write("ornament_tests::do_*");
        families
            .iter()
            .map(|(font, _)| fonts::pinned_family(&fs, *font))
            .collect()
    };
    for ((font, family), pin) in families.iter().zip(pins) {
        let pin = pin.expect("a compiled family");
        assert_eq!(pin.name, *family, "{font:?}'s family");
        if *font == DEFAULT_FACE.0 {
            assert_eq!(pin.axes, vec![(100, 900)], "{family}'s axis");
            assert_eq!(pin.faces.len(), 1, "{family}: one variable face");
            continue;
        }
        assert_eq!(pin.weights, FALLBACK_WEIGHTS.to_vec(), "{family}'s weights");
        assert_eq!(
            pin.faces.len(),
            FALLBACK_WEIGHTS.len(),
            "{family}: one face a weight"
        );
    }
}

/// What `text`, shaped alone at `weight` in the family `family`, is
/// drawn in: each glyph's face, PostScript name and glyph id.
fn drawn(
    fs: &mut cosmic_text::FontSystem,
    text: &str,
    family: &str,
    weight: u16,
) -> Vec<(cosmic_text::fontdb::ID, String, u16)> {
    let attrs = cosmic_text::Attrs::new()
        .family(cosmic_text::Family::Name(family))
        .weight(cosmic_text::Weight(weight));
    drawn_with(fs, text, &attrs)
}

/// What `text`, shaped alone with `attrs`, is drawn in, as [`drawn`]
/// reads it.
fn drawn_with(
    fs: &mut cosmic_text::FontSystem,
    text: &str,
    attrs: &cosmic_text::Attrs<'_>,
) -> Vec<(cosmic_text::fontdb::ID, String, u16)> {
    let mut buffer = cosmic_text::Buffer::new(fs, cosmic_text::Metrics::new(16.0, 20.0));
    buffer.set_text(text, attrs, cosmic_text::Shaping::Advanced, None);
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

/// A Hebrew letter: a run of it is a run of another script than Latin,
/// shaped through the same single fallback list (cosmic-text tries a
/// script's own list first, and every script's is empty).
const SCRIPT_LETTER: char = 'א';

#[test]
fn test_a_pinned_span_draws_each_ornament_in_its_compiled_face() {
    do_a_pinned_span_draws_each_ornament_in_its_compiled_face();
}

/// #297: a span pinned to a compiled family that lacks an ornament draws
/// it in its compiled ornament face, at the regular and the bold weight,
/// alone and between two Hebrew letters. Galatia SIL lacks every one of
/// them.
///
/// Controls (the PR lists what each drew): with the ornament families
/// left out of the fallback list, red on `∙` and the box-drawing
/// characters, drawn in `African`, a decorative face cosmic-text meets
/// first among the rest; with the faces at their own weight only (no
/// `register_every_weight`), red at 700 alone.
pub fn do_a_pinned_span_draws_each_ornament_in_its_compiled_face() {
    fonts::init();
    let galatia = mapped(face_data(AppFont::GalatiaSIL));
    let galatia_bold = mapped(face_data(AppFont::GalatiaSILBold));
    let cases: Vec<(char, AppFont)> = ORNAMENT_CODE_POINTS
        .iter()
        .map(|c| (*c, AppFont::MandalaOrnaments))
        .chain(EMOJI_CODE_POINTS.iter().map(|c| (*c, AppFont::MandalaEmoji)))
        .collect();
    for (c, _) in &cases {
        assert!(
            !galatia.contains(c) && !galatia_bold.contains(c),
            "precondition: Galatia SIL lacks {c:?}, so it falls back"
        );
    }
    assert!(
        !galatia.contains(&SCRIPT_LETTER) && !galatia_bold.contains(&SCRIPT_LETTER),
        "precondition: Galatia SIL lacks {SCRIPT_LETTER:?}, so its run falls back too"
    );
    // Read under the guard and assert once it is released, so that a
    // failure does not poison the lock for the tests beside it.
    let results = {
        let mut fs = acquire_font_system_write("ornament_tests::pinned");
        let pin = fonts::pinned_family(&fs, AppFont::GalatiaSIL).expect("a compiled family");
        let mut results = Vec::new();
        for (c, font) in &cases {
            let texts = [
                (c.to_string(), 0),
                (format!("{SCRIPT_LETTER}{c}{SCRIPT_LETTER}"), 1),
            ];
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
        results
    };
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

#[test]
fn test_an_unpinned_span_draws_each_ornament_in_its_compiled_face_at_every_weight() {
    do_an_unpinned_span_draws_each_ornament_in_its_compiled_face_at_every_weight();
}

/// #297 at every weight and both slants: a span that pins no family, as
/// `flow::span_attrs` shapes an `AppFont::Any` span (cosmic-text's
/// default attributes, the span's weight as written), draws `✦ ❦ ∙ ◈`,
/// two box-drawing characters and `🌸` in their compiled faces at each of
/// [`FALLBACK_WEIGHTS`], upright and italic. The span's own face is the
/// default face, which holds none of them (asserted), and which, being
/// variable, cosmic-text matches at every weight: so at a weight the
/// ornament faces were not registered at, the fallback would pass them
/// by. The italic half holds the matcher's not filtering on slant, which
/// the fallback relies on: the upright face draws an italic span's
/// ornament, skewed.
///
/// Control: with the faces at their own weight only (no
/// `register_every_weight`), red at the eight weights but 400, the faces'
/// own, upright and italic, on `∙`, drawn in `African`, the one compiled
/// face beside them that holds it at those weights.
pub fn do_an_unpinned_span_draws_each_ornament_in_its_compiled_face_at_every_weight() {
    use cosmic_text::fontdb::{Style, Weight};
    fonts::init();
    let candidates: [(char, AppFont); 7] = [
        ('✦', AppFont::MandalaOrnaments),
        ('❦', AppFont::MandalaOrnaments),
        ('∙', AppFont::MandalaOrnaments),
        ('◈', AppFont::MandalaOrnaments),
        ('━', AppFont::MandalaOrnaments),
        ('╭', AppFont::MandalaOrnaments),
        ('🌸', AppFont::MandalaEmoji),
    ];
    let default = mapped(face_data(DEFAULT_FACE.0));
    for (c, _) in candidates {
        assert!(
            !default.contains(&c),
            "precondition: the default face lacks {c:?}, so it falls back"
        );
    }
    let results = {
        let mut fs = acquire_font_system_write("ornament_tests::unpinned");
        let mut results = Vec::new();
        for weight in FALLBACK_WEIGHTS {
            for style in [Style::Normal, Style::Italic] {
                for (c, font) in candidates {
                    let attrs = cosmic_text::Attrs::new().weight(Weight(weight)).style(style);
                    let glyphs = drawn_with(&mut fs, &c.to_string(), &attrs);
                    results.push((c, font, weight, style, glyphs.first().cloned(), glyphs.len()));
                }
            }
        }
        results
    };
    assert_eq!(
        results.len(),
        candidates.len() * FALLBACK_WEIGHTS.len() * 2,
        "precondition: every weight and both slants are shaped"
    );
    let mut faults = Vec::new();
    for (c, font, weight, style, glyph, count) in results {
        let ids = &fonts::COMPILED_FONT_ID_MAP[&font];
        match glyph {
            Some((id, _, glyph)) if ids.contains(&id) && glyph != 0 => {}
            Some((_, name, glyph)) => faults.push(format!(
                "{c:?} at {weight} {style:?}: drawn in {name} (glyph {glyph})"
            )),
            None => faults.push(format!("{c:?} at {weight} {style:?}: {count} glyphs")),
        }
    }
    assert!(
        faults.is_empty(),
        "{} ornaments not drawn in their compiled face:\n{}",
        faults.len(),
        faults.join("\n")
    );
}

#[test]
fn test_no_text_glyph_is_drawn_by_an_ornament_face() {
    do_no_text_glyph_is_drawn_by_an_ornament_face();
}

/// #228's choice (a), held at the shaper: no glyph of [`TEXT`] — letters,
/// digits, spaces, punctuation, and the ` + ~ · ♪` `Mandala Ornaments`
/// held until #228 — is drawn by an ornament face. Unpinned, at every
/// weight and both slants, every glyph is the default face's (#228's
/// choice (c)). Pinned to each compiled face that lacks some of them,
/// those it lacks are drawn by the default face, the first face of the
/// fallback, and each by a glyph that is not a `.notdef`.
///
/// Two mechanisms hold it, each enough alone: the ornament faces hold no
/// code point the default face holds
/// (`do_the_ornament_faces_hold_no_code_point_the_default_face_holds`),
/// and the default face comes first in the fallback list. Controls (the
/// PR lists what each drew): with the pre-#228 `Mandala Ornaments` (the
/// space and `+ ~ · ♪` among its glyphs) put ahead of the default face in
/// the fallback list, red on the pinned faces' `+ ~ · ♪`, drawn in
/// `MandalaOrnaments-Regular`; with the default face left out of the
/// fallback list, red on the pinned faces' punctuation, drawn in
/// `African` and Noto Sans Devanagari; and with that and the generic
/// families left as cosmic-text's names, red on every unpinned glyph.
pub fn do_no_text_glyph_is_drawn_by_an_ornament_face() {
    use cosmic_text::fontdb::{Style, Weight};
    fonts::init();
    let default_ids = &fonts::COMPILED_FONT_ID_MAP[&DEFAULT_FACE.0];
    let ornament_ids: Vec<cosmic_text::fontdb::ID> = ORNAMENT_FALLBACK
        .iter()
        .flat_map(|(font, _)| fonts::COMPILED_FONT_ID_MAP[font].iter().copied())
        .collect();
    let lacking: Vec<(AppFont, Vec<char>)> = fonts::COMPILED_FONT_ID_MAP
        .keys()
        .filter(|font| !fallback_families().any(|(listed, _)| listed == *font))
        .map(|font| {
            let held = mapped(face_data(*font));
            let mut missing: Vec<char> = TEXT.chars().filter(|c| !held.contains(c)).collect();
            missing.sort_unstable();
            missing.dedup();
            (*font, missing)
        })
        .filter(|(_, missing)| !missing.is_empty())
        .collect();
    for c in [' ', '+', '~', '·'] {
        assert!(
            lacking.iter().any(|(_, missing)| missing.contains(&c)),
            "precondition: a compiled face lacks {c:?}, so a span pinned to it falls back for it"
        );
    }
    let (unpinned, pinned) = {
        let mut fs = acquire_font_system_write("ornament_tests::text");
        let mut unpinned = Vec::new();
        for weight in FALLBACK_WEIGHTS {
            for style in [Style::Normal, Style::Italic] {
                let attrs = cosmic_text::Attrs::new().weight(Weight(weight)).style(style);
                unpinned.push((weight, style, drawn_with(&mut fs, TEXT, &attrs)));
            }
        }
        let mut pinned = Vec::new();
        for (font, missing) in &lacking {
            // A face whose family name the font system cannot read
            // (Scrapbook Chinese) is one no span can be pinned to.
            let Some(pin) = fonts::pinned_family(&fs, *font) else {
                continue;
            };
            for c in missing {
                // Between two of its own letters, as in running text.
                let glyphs = drawn(&mut fs, &format!("a{c}a"), &pin.name, pin.weight_for(400));
                pinned.push((*font, *c, pin.faces.clone(), glyphs));
            }
        }
        (unpinned, pinned)
    };
    let mut faults = Vec::new();
    for (weight, style, glyphs) in &unpinned {
        assert_eq!(
            glyphs.len(),
            TEXT.chars().count(),
            "precondition: a glyph a character at {weight} {style:?}"
        );
        for ((id, name, glyph), c) in glyphs.iter().zip(TEXT.chars()) {
            if !default_ids.contains(id) || *glyph == 0 {
                faults.push(format!(
                    "unpinned {c:?} at {weight} {style:?}: {name} glyph {glyph}"
                ));
            }
        }
    }
    assert!(pinned.len() >= 4, "precondition: the pinned cases are shaped");
    for (font, c, own, glyphs) in &pinned {
        let fallen: Vec<_> = glyphs.iter().filter(|(id, _, _)| !own.contains(id)).collect();
        if fallen.is_empty() {
            faults.push(format!("{font:?}'s missing {c:?}: drawn in its own face"));
        }
        for (id, name, glyph) in fallen {
            if ornament_ids.contains(id) || !default_ids.contains(id) || *glyph == 0 {
                faults.push(format!("{font:?}'s missing {c:?}: {name} glyph {glyph}"));
            }
        }
    }
    assert!(
        faults.is_empty(),
        "{} text glyphs not drawn by the default face:\n{}",
        faults.len(),
        faults.join("\n")
    );
}

#[test]
fn test_a_hebrew_run_is_drawn_in_noto_sans_hebrew_not_public_pixel() {
    do_a_hebrew_run_is_drawn_in_noto_sans_hebrew_not_public_pixel();
}

/// [`SCRIPT_FALLBACK`](crate::font::fallback::SCRIPT_FALLBACK)'s one
/// entry: Public Pixel holds Hebrew letters too, so a run of Hebrew that
/// its span's face lacks — unpinned, or pinned to Galatia SIL — is drawn
/// in Noto Sans Hebrew because the fallback lists it, at the regular and
/// the bold weight, and not in whichever of the two cosmic-text's last
/// pass meets first.
///
/// In the font system itself the last pass would meet Noto Sans Hebrew
/// first anyway: the two are both 400, and between equal weights the
/// pass goes in build order, which sorts Noto before Public Pixel by
/// name. So the run is shaped twice: in the font system, and in one
/// built over the same fallback with Public Pixel loaded first, as a
/// compiled Hebrew face whose name sorts earlier would be. There only
/// the entry decides.
///
/// Control: with `SCRIPT_FALLBACK` empty, red on every letter of the
/// second shaping, drawn in `PublicPixel`; the first stays green.
pub fn do_a_hebrew_run_is_drawn_in_noto_sans_hebrew_not_public_pixel() {
    const WORD: &str = "שלום";
    let pixel = mapped(face_data(AppFont::PublicPixel));
    let galatia = mapped(face_data(AppFont::GalatiaSIL));
    let default = mapped(face_data(DEFAULT_FACE.0));
    for c in WORD.chars() {
        assert!(
            pixel.contains(&c),
            "precondition: Public Pixel holds {c:?}, so it competes"
        );
        assert!(
            !galatia.contains(&c) && !default.contains(&c),
            "precondition: neither span face holds {c:?}, so it falls back"
        );
    }
    // Shape `WORD` unpinned and pinned to Galatia SIL at 400 and 700 in
    // `fs`, naming each glyph that is not one of `hebrew` or is a
    // `.notdef`.
    let faults_in = |fs: &mut cosmic_text::FontSystem, hebrew: &[cosmic_text::fontdb::ID], what: &str| {
        let mut faults = Vec::new();
        for weight in [400, 700] {
            let unpinned = cosmic_text::Attrs::new().weight(cosmic_text::Weight(weight));
            let shaped = [
                ("unpinned", drawn_with(fs, WORD, &unpinned)),
                ("Galatia SIL", drawn(fs, WORD, "Galatia SIL", weight)),
            ];
            for (span, glyphs) in shaped {
                if glyphs.len() != WORD.chars().count() {
                    faults.push(format!("{what}, {span} at {weight}: {} glyphs", glyphs.len()));
                }
                for (id, name, glyph) in glyphs {
                    if !hebrew.contains(&id) || glyph == 0 {
                        faults.push(format!("{what}, {span} at {weight}: {name} glyph {glyph}"));
                    }
                }
            }
        }
        faults
    };
    fonts::init();
    let mut faults = {
        let mut fs = acquire_font_system_write("ornament_tests::hebrew");
        let hebrew = fonts::COMPILED_FONT_ID_MAP[&AppFont::NotoSansHebrewRegular].to_vec();
        faults_in(&mut fs, &hebrew, "the font system")
    };
    // Public Pixel first, then the span faces and the fallback's, each
    // static fallback face at every weight as the font system has it.
    let mut db = cosmic_text::fontdb::Database::new();
    let mut load = |font: AppFont| -> tinyvec::TinyVec<[cosmic_text::fontdb::ID; 8]> {
        db.load_font_source(FONT_SOURCES[&font].clone())
    };
    load(AppFont::PublicPixel);
    load(AppFont::GalatiaSIL);
    load(AppFont::GalatiaSILBold);
    load(DEFAULT_FACE.0);
    let mut statics: Vec<_> = ORNAMENT_FALLBACK.iter().map(|(font, _)| load(*font)).collect();
    let mut hebrew = load(AppFont::NotoSansHebrewRegular);
    for ids in statics.iter_mut().chain(std::iter::once(&mut hebrew)) {
        crate::font::fallback::register_every_weight(&mut db, ids);
    }
    let mut scratch = crate::font::fallback::new_font_system(db);
    faults.extend(faults_in(&mut scratch, &hebrew, "Public Pixel first"));
    assert!(
        faults.is_empty(),
        "Hebrew not drawn in Noto Sans Hebrew:\n{}",
        faults.join("\n")
    );
}
