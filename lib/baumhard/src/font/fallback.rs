// SPDX-License-Identifier: MPL-2.0

//! The ornament fallback (V1-B11, #297): the compiled faces the shaper
//! reaches first, after a span's own face, for a glyph that face lacks.
//!
//! The demo maps and the built-in styles draw ornaments — `✦`, `❦`, `♪`,
//! `∙`, the connector's `·`, the portal end's `◈`, the frames demo's `🌸`
//! — that no compiled text face holds. cosmic-text looks for such a glyph
//! in the fallback families its platform lists (on Linux `Noto Sans`,
//! `DejaVu Sans`, …, `Noto Color Emoji`), and only after them in every
//! other face. So on native they were drawn in whatever host face holds
//! them, and in the browser, which has only the compiled faces, in
//! whichever compiled face came first, or as an empty box.
//!
//! Two compiled faces hold them now, subset from freely licensed sources
//! by `scripts/ornament_fonts.sh` ([`ORNAMENT_FALLBACK`]), and the font
//! system is built with a [`Fallback`] that lists their families ahead of
//! the platform's, in the common list and in every script's
//! ([`new_font_system`]). A span's own face still comes first: a pinned
//! family that holds a glyph draws it. What changes is the face a glyph
//! falls back to, which is now a compiled one on every install.
//!
//! cosmic-text matches a fallback family only at the exact weight a span
//! asks for (or within a variable face's `wght` axis), so each fallback
//! face is registered at every CSS weight ([`register_every_weight`]): the
//! same outlines under each weight, nothing synthesized, as a single-weight
//! family draws every weight at its own (ruling R30).
//!
//! What this cannot reach is a span's own family when that is a host
//! face: a span that pins no compiled family asks for cosmic-text's
//! default, which on a host that has it is a host face, and a glyph it
//! holds is drawn in it. That is the default family's gap (#228), not a
//! fallback's.

use std::sync::OnceLock;

use cosmic_text::fontdb::{Weight, ID};
use cosmic_text::{Fallback, FontSystem, PlatformFallback};
use tinyvec::TinyVec;
use unicode_script::Script;

use crate::font::fonts::AppFont;

/// The compiled faces a glyph falls back to before any host face, in the
/// order they are tried, with the family name each face stores.
///
/// - **Mandala Ornaments**: DejaVu Sans 2.37 subset to the space and
///   `+ ~ · ∙ ◈ ♪ ✦ ❦` (Bitstream Vera and Arev licenses).
/// - **Mandala Emoji**: Noto Color Emoji 2.047 subset to `🌸` (OFL 1.1).
///
/// Two faces, because cosmic-text tries one face of a fallback family:
/// a second face under the same name would be reached only after every
/// host face. The names are spelled here, not read from the faces,
/// because the fallback lists are built with the font system, before the
/// compiled faces are registered in it;
/// `test_each_ornament_fallback_names_its_faces_family_at_every_weight`
/// holds each to its face.
pub const ORNAMENT_FALLBACK: &[(AppFont, &str)] = &[
    (AppFont::MandalaOrnaments, "Mandala Ornaments"),
    (AppFont::MandalaEmoji, "Mandala Emoji"),
];

/// The weights each [`ORNAMENT_FALLBACK`] face is registered at: every
/// CSS weight, so a span at any of them reaches it.
pub const FALLBACK_WEIGHTS: [u16; 9] = [100, 200, 300, 400, 500, 600, 700, 800, 900];

/// cosmic-text's platform fallback lists with [`ORNAMENT_FALLBACK`]'s
/// families first in each: the list every script's run falls back to
/// last (`common`), and each script's own list, which a run of that
/// script tries before it (`scripts`).
///
/// The trait hands out borrows of the lists, so each is kept here. A
/// script's is built the first time cosmic-text asks for it, in its slot
/// of `scripts`, indexed by the script's `u8` discriminant: Unicode's
/// scripts cannot be listed up front through `unicode_script` (a
/// `ScriptExtension` of every script iterates as `Common` alone).
struct OrnamentsFirst {
    /// The ornament families, then the platform's common list.
    common: Vec<&'static str>,
    /// For each script, once asked for, the ornament families, then the
    /// platform's list for it at `locale`.
    scripts: [OnceLock<Vec<&'static str>>; 256],
    /// The locale `scripts` is built for.
    locale: String,
    /// The platform's lists.
    platform: PlatformFallback,
}

/// [`ORNAMENT_FALLBACK`]'s family names, in order.
fn ornament_families() -> impl Iterator<Item = &'static str> {
    ORNAMENT_FALLBACK.iter().map(|(_, family)| *family)
}

impl OrnamentsFirst {
    /// The lists for a font system at `locale`. Cost: the common list; a
    /// script's costs one platform lookup and one small `Vec`, the first
    /// time it is asked for.
    fn new(locale: &str) -> Self {
        let platform = PlatformFallback;
        let common = ornament_families()
            .chain(platform.common_fallback().iter().copied())
            .collect();
        Self {
            common,
            scripts: std::array::from_fn(|_| OnceLock::new()),
            locale: locale.to_string(),
            platform,
        }
    }
}

impl Fallback for OrnamentsFirst {
    fn common_fallback(&self) -> &[&'static str] {
        &self.common
    }

    fn forbidden_fallback(&self) -> &[&'static str] {
        self.platform.forbidden_fallback()
    }

    fn script_fallback(&self, script: Script, locale: &str) -> &[&'static str] {
        // cosmic-text asks for its own locale, which is the one the lists
        // are built for; another gets the platform's list unchanged.
        if locale != self.locale {
            return self.platform.script_fallback(script, locale);
        }
        self.scripts[script as usize].get_or_init(|| {
            ornament_families()
                .chain(self.platform.script_fallback(script, locale).iter().copied())
                .collect()
        })
    }
}

/// The font system [`FONT_SYSTEM`](crate::font::fonts) is: cosmic-text's
/// own (`FontSystem::new`, the host's faces indexed on native, the locale
/// read from the system), with [`OrnamentsFirst`]'s fallback lists in
/// place of the platform's. Cost: `FontSystem::new`, then one more walk
/// over the database's faces, which cosmic-text makes to list the
/// monospaced ones.
pub(crate) fn new_font_system() -> FontSystem {
    let (locale, db) = FontSystem::new().into_locale_and_db();
    let fallback = OrnamentsFirst::new(&locale);
    FontSystem::new_with_locale_and_db_and_fallback(locale, db, fallback)
}

/// Register the face `ids` names at every one of [`FALLBACK_WEIGHTS`] it
/// is not already at, adding each new face's id to `ids`: the same
/// source, family and outlines under another weight. Cost: one database
/// insert per weight added.
pub(crate) fn register_every_weight(font_system: &mut FontSystem, ids: &mut TinyVec<[ID; 8]>) {
    let faces: Vec<_> = ids
        .iter()
        .filter_map(|id| font_system.db().face(*id).cloned())
        .collect();
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
        ids.push(font_system.db_mut().push_face_info(info));
    }
}
