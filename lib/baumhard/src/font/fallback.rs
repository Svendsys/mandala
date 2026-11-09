// SPDX-License-Identifier: MPL-2.0

//! The default face and the fallback (#228, #297): which compiled face
//! draws text that pins no family, and which faces a glyph a span's own
//! face lacks is looked for in, in what order — the same on every target.
//!
//! **The default face** is
//! [`DEFAULT_FACE`](crate::font::fallback::DEFAULT_FACE), Inter 4.1's
//! variable face, compiled in. cosmic-text names a face for each generic
//! family (`Family::SansSerif`, which `Attrs::new()` asks for,
//! `Family::Monospace` and the rest), and `new_font_system` names this
//! one for all of them.
//! So a span with no family — an `AppFont::Any` span, an edge label, a
//! connector's `·` — is drawn in it on every machine and in the browser.
//! Until #228 those names were cosmic-text's own ("Open Sans", "Noto Sans
//! Mono"), which no compiled face bears: on a Linux desktop the text
//! reached the host's DejaVu Sans through the fallback, and in the
//! browser, or on a host with no fonts, a decorative face or an empty box.
//!
//! **The fallback** is one list,
//! [`fallback_families`](crate::font::fallback::fallback_families),
//! which cosmic-text tries after a span's own face: the default face, then
//! the two ornament faces
//! ([`ORNAMENT_FALLBACK`](crate::font::fallback::ORNAMENT_FALLBACK)),
//! then the compiled faces of the scripts the default face does not write
//! ([`SCRIPT_FALLBACK`](crate::font::fallback::SCRIPT_FALLBACK)). It is
//! the common list of a [`Fallback`](cosmic_text::Fallback) whose script
//! lists are empty, so a run of any script, on any target, tries the same
//! faces in the same order;
//! cosmic-text's platform lists, which differ between Linux, Windows,
//! macOS and the browser and name host families, are not consulted. A
//! glyph none of those faces holds is looked for in every other face, as
//! cosmic-text does after its lists, in the order of its `FontMatchKey`:
//! emoji faces first, then nearest in weight, in stretch and in slant,
//! then lighter and narrower, and only among faces equal in all of those
//! in the order they were loaded — the compiled faces in the order the
//! build lists them, then, on native, the host's (#228). So a host face
//! draws a glyph no compiled face holds; and a glyph only an unlisted
//! (decorative) compiled face holds can be drawn first by a host emoji
//! face, by a host face of the span's weight where the compiled one has
//! another, or, for an italic span, by a host italic face. Every glyph the
//! app, its built-in styles and the shipped maps draw is held by a listed
//! face, so none reaches that pass.
//!
//! The ornament faces hold no code point the default face holds
//! (`scripts/ornament_fonts.sh`, held by `font::tests::ornament_tests`),
//! so a letter, digit, space or punctuation mark is never drawn by an
//! ornament face, whatever face a span pins (#228's choice (a)); and a
//! span's own face still comes first, so a pinned family that holds an
//! ornament draws it in its own design.
//!
//! cosmic-text matches a fallback family only at the exact weight a span
//! asks for, or within a variable face's `wght` axis. The default face is
//! variable from 100 to 900; each static fallback face is registered at
//! every CSS weight instead (`register_every_weight`): the same outlines
//! under each weight, no weight synthesized, as a single-weight family
//! draws every weight at its own (ruling R30). The slant needs no
//! registering, since the matcher does not filter on it: an italic span's
//! glyph is drawn by an upright face, which cosmic-text skews by 14°
//! (`CacheKeyFlags::FAKE_ITALIC`).

use cosmic_text::fontdb::{Database, Weight, ID};
use cosmic_text::{Fallback, FontSystem};
use tinyvec::TinyVec;
use unicode_script::Script;

use crate::font::fonts::AppFont;

/// The face text that pins no family is drawn in, and the first face of
/// the fallback, with the family name it stores: Inter 4.1's variable
/// face (SIL Open Font License 1.1), `wght` 100 to 900. It holds Latin,
/// Greek and Cyrillic and the punctuation, arrows and geometric shapes
/// the maps and the app's own text draw; the box-drawing characters it
/// lacks are in `Mandala Ornaments`. `format/fonts.md` records why this
/// face.
pub const DEFAULT_FACE: (AppFont, &str) = (AppFont::InterVariable, "Inter Variable");

/// The ornament faces, in the order they are tried, after the default
/// face, with the family name each face stores.
///
/// - **Mandala Ornaments**: DejaVu Sans 2.37 subset to `∙ ◈ ✦ ❦` and the
///   box-drawing `─ ━ │ ┃ ╭ ╮ ╯ ╰` (Bitstream Vera and Arev licenses).
/// - **Mandala Emoji**: Noto Color Emoji 2.047 subset to `🌸` (OFL 1.1).
///
/// Two faces, because cosmic-text tries one face of a fallback family: a
/// second face under the same name would be reached only after every
/// listed family.
pub const ORNAMENT_FALLBACK: &[(AppFont, &str)] = &[
    (AppFont::MandalaOrnaments, "Mandala Ornaments"),
    (AppFont::MandalaEmoji, "Mandala Emoji"),
];

/// The compiled faces of the scripts the default face does not write,
/// tried after the ornament faces, with the family name each face stores.
/// Listed, each is registered at every weight and met at the span's own
/// weight before cosmic-text's last pass, where another face that holds
/// the script — a host face at the span's weight, or Public Pixel, which
/// holds Hebrew letters — could come first. A compiled script face is
/// added here when it is compiled in.
pub const SCRIPT_FALLBACK: &[(AppFont, &str)] = &[
    (AppFont::NotoSansHebrewRegular, "Noto Sans Hebrew"),
    (AppFont::NotoSansDevanagariRegular, "Noto Sans Devanagari"),
    (AppFont::NotoSerifTibetanRegular, "Noto Serif Tibetan"),
    (
        AppFont::NotoSansEgyptianHieroglyphsRegular,
        "Noto Sans Egyptian Hieroglyphs",
    ),
];

/// The weights each static fallback face is registered at: every CSS
/// weight, so a span at any of them reaches it.
pub const FALLBACK_WEIGHTS: [u16; 9] = [100, 200, 300, 400, 500, 600, 700, 800, 900];

/// Every face of the fallback, in the order cosmic-text tries them:
/// [`DEFAULT_FACE`], [`ORNAMENT_FALLBACK`], [`SCRIPT_FALLBACK`]. It is
/// the list a span of any family asks for: the `AttrsList` bridge asks
/// for no family either (`font::attrs`), so no path reaches
/// cosmic-text's `Family::Monospace` search, which tries every
/// monospaced face before the list.
pub fn fallback_families() -> impl Iterator<Item = &'static (AppFont, &'static str)> {
    std::iter::once(&DEFAULT_FACE)
        .chain(ORNAMENT_FALLBACK)
        .chain(SCRIPT_FALLBACK)
}

/// The locale the font system is built with. cosmic-text reads a locale
/// only to choose a script's fallback list, which [`CompiledFallback`]
/// leaves empty, so it decides nothing; it is fixed rather than read
/// from the host so that it cannot.
const LOCALE: &str = "en-US";

/// The fallback lists: [`fallback_families`] as the common list, and no
/// script's own list, on every target.
struct CompiledFallback {
    /// [`fallback_families`]' names, in order.
    common: Vec<&'static str>,
}

impl Fallback for CompiledFallback {
    fn common_fallback(&self) -> &[&'static str] {
        &self.common
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        &[]
    }

    fn script_fallback(&self, _script: Script, _locale: &str) -> &[&'static str] {
        &[]
    }
}

/// The font system over `db`, which holds the compiled faces first and,
/// on native, the host's after them: [`DEFAULT_FACE`]'s family named for
/// every generic family, and [`CompiledFallback`]'s lists. The host's
/// locale is not read, nor its platform's fallback lists. Cost: one walk
/// over the database's faces, which cosmic-text makes to list the
/// monospaced ones.
pub(crate) fn new_font_system(mut db: Database) -> FontSystem {
    let family = DEFAULT_FACE.1;
    db.set_sans_serif_family(family);
    db.set_serif_family(family);
    db.set_monospace_family(family);
    db.set_cursive_family(family);
    db.set_fantasy_family(family);
    let common = fallback_families().map(|(_, family)| *family).collect();
    FontSystem::new_with_locale_and_db_and_fallback(LOCALE.to_string(), db, CompiledFallback { common })
}

/// Whether the font `data` (a face of a collection at `index`) is
/// variable in weight: whether it has a `wght` axis, and the whole
/// weights it spans if so, its ends rounded inward. Cost: one parse of the font's table directory and its
/// `fvar` table.
pub(crate) fn weight_axis(data: &[u8], index: u32) -> Option<(u16, u16)> {
    use cosmic_text::skrifa::{FontRef, MetadataProvider, Tag};
    let font = FontRef::from_index(data, index).ok()?;
    let axis = font.axes().get_by_tag(Tag::new(b"wght"))?;
    // Inward, so every weight reported as spanned is one cosmic-text,
    // which compares the axis's own ends, matches the face at.
    Some((axis.min_value().ceil() as u16, axis.max_value().floor() as u16))
}

/// Register the face `ids` names at every one of [`FALLBACK_WEIGHTS`] it
/// is not already at, adding each new face's id to `ids`: the same
/// source, family and outlines under another weight. Cost: one database
/// insert per weight added.
pub(crate) fn register_every_weight(db: &mut Database, ids: &mut TinyVec<[ID; 8]>) {
    let faces: Vec<_> = ids.iter().filter_map(|id| db.face(*id).cloned()).collect();
    let have: Vec<u16> = faces.iter().map(|face| face.weight.0).collect();
    let Some(face) = faces.first() else {
        return;
    };
    for weight in FALLBACK_WEIGHTS
        .into_iter()
        .filter(|weight| !have.contains(weight))
    {
        let mut info = face.clone();
        info.id = ID::dummy();
        info.weight = Weight(weight);
        ids.push(db.push_face_info(info));
    }
}
