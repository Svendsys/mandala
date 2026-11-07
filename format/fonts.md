# Fonts

Mandala bundles every font it ships with, so the binary renders any
saved map without external assets. The intent is that what's compiled
in is *all* that is available, on every target: identical glyphs on
native and in the browser (CODE_CONVENTIONS §4), and a map that looks
the same wherever it is opened.

**The native build does not yet honor that.**
`baumhard::font::fonts`' `FONT_SYSTEM` is
`RwLock::new(FontSystem::new())`, and cosmic-text's `FontSystem::new`
indexes the host's fontconfig — so on native, `font list`,
`font set <family>` and the completion popup all offer whatever the
machine happens to have installed, attributed to `AppFont::Any`,
alongside the compiled-in faces. The browser build has no fontconfig
to index, so it really does see only what is compiled in. A map
authored on a Linux desktop against a host family therefore renders
in the monospace fallback in a browser, with a `warn!`, and nothing
about the file says why.

A `.mandala.json` map cannot take part in that accident: the loader
refuses a family no compiled-in face provides, so a map names only
portable families and loads, or does not load, the same on every
target ([`schema.md`](./schema.md)). What the host's fonts still reach
is the native console's font listing and completion.

Closing the gap means constructing the `FontSystem` without the system
sources. That is deliberately **not** done here: it changes what the
console offers on native, so it is a decision of its own, not a line
change. Until it is taken, read the paragraph above as: the
compiled-in set is the *portable* set, and anything outside it is a
native-only accident. New surfaces should assume the portable set; the
load-failure placard (`MindMapDocument::load_failure_placard`) is the
worked example: it pins no family at all rather than naming one that
exists on one target.

## Where a map names a font

A span's `font` names a family ([`schema.md`](./schema.md)); a span
without one inherits its element's, and so on up to the map's defaults.
Each name resolves through `baumhard::font::fonts::app_font_by_family`
to the build-time `AppFont` enum, and the loader refuses a name that
resolves to no compiled-in face (`LoadErrorKind::UnknownFont`) — the
fallback face has no name a file could write, so the document could not
save it back.

## Weight within a pinned family

A pinned family is drawn only in its compiled faces, at the nearest
weight they have (ruling R30 on #278). A request for a weight the
family's compiled faces lack — `"bold"` on a single-weight face, `300`
on SilhousTwo — is matched within the family as CSS Fonts 4 §5.2
matches `font-weight`, and nothing is synthesized: a weight the family
has is kept; for one from 400 to 500, the lightest heavier weight up
to 500, then the heaviest lighter one, then the lightest above 500; for
one below 400, the heaviest lighter weight, then the lightest heavier;
for one above 500, the lightest heavier, then the heaviest lighter. A
family whose only face is 400 draws every weight at 400.

The reason is the shaper. cosmic-text keeps a pinned family only
through a face of exactly the weight asked for, and otherwise draws in
whatever host face has that exact weight — on the CI runner,
`DejaVuSans-ExtraLight` for SilhousTwo at 200 — so the host's fonts, not
the map, would decide the face on native, where the browser has only
the compiled set. Every shaping path resolves the weight before
cosmic-text sees it: a TextFlow span's style weight
(`flow::span_attrs`, and the tab check that measures with it), and the
regular weight the measuring and region paths ask for
(`font::fonts::PinnedFamily::weight_for`). The slant needs no
resolving: the matcher keeps a pinned face whatever its slant. A
character no compiled face of the family holds is still drawn in a
fallback face, as it must be.

**Compiled families are authoritative by name too** (ruling R30.1).
cosmic-text matches a pin by family name and weight alone, and
`FontSystem::new` indexes the host's fonts before the compiled set is
added, so a host face bearing a compiled family's name — the Noto Sans
Hebrew, Noto Sans Devanagari, Noto Serif Tibetan or Noto Sans Egyptian
Hieroglyphs a Debian or Ubuntu desktop installs — would be met first and
drawn in place of the compiled one, and a map naming that family would
resolve to the host's face (`AppFont::Any`). So once the compiled faces
are registered, every other face bearing a compiled family's name is
removed from the font system (`remove_shadowing_faces`). A pin then
names the compiled faces on every machine; the host's other families
stay, for fallback and for `font list`.

## Ornament fallback

A character a span's face lacks is drawn in a fallback face, and the
first faces tried are two compiled ones (#297,
`baumhard::font::fallback::ORNAMENT_FALLBACK`):

| Family | Holds | Source | License |
|--------|-------|--------|---------|
| `Mandala Ornaments` | the space and `+ ~ · ∙ ◈ ♪ ✦ ❦` | DejaVu Sans 2.37 | Bitstream Vera and Arev; DejaVu's changes public domain |
| `Mandala Emoji` | `🌸` | Noto Color Emoji 2.047 (its color bitmap) | SIL Open Font License 1.1 |

Those are the ornaments the demo maps draw — sides, `repeat` rings,
connectors — and the glyphs the built-in styles draw a connector, a link
and a portal end with (`·`, `◈`). Without the fallback, cosmic-text
tries the families its platform lists (on Linux `Noto Sans`,
`DejaVu Sans`, …, `Noto Color Emoji`) before any other face, so on
native these were drawn in whatever host face held them, and in the
browser, which has only the compiled faces, in whichever compiled face
came first, or as an empty box. Now they are the same glyph everywhere.

How it holds:

- **Ahead of the platform's lists.** `FONT_SYSTEM` is built with a
  cosmic-text `Fallback` that puts the two families first in the common
  fallback list and in every script's list, which a run of that script
  tries before the common one.
- **At every weight.** cosmic-text matches a fallback family only at the
  exact weight a span asks for, so each face is registered at every CSS
  weight from 100 to 900: the same outlines under each, no weight
  synthesized, as a single-weight family draws every weight at its own
  (R30). A bold span's ornament is drawn in the compiled face, not in
  the host's bold. The slant needs no registering, since the matcher
  does not filter on it: an italic span's ornament is drawn by the
  upright face, which cosmic-text skews by 14° (`FAKE_ITALIC`), as it
  skewed the host's upright DejaVu Sans before.
- **By name** (R30.1). A host face that took one of the two names would
  be removed like any other face bearing a compiled family's name.

What it does not change: a span's own face comes first, so a pinned
family that holds an ornament draws it in its own design. And a span
that pins no compiled family asks for cosmic-text's default family,
which is a host face where the host has one; a glyph that face holds is
drawn in it. That is the default family's gap, the paragraph this file
opens with and #228, not the fallback's.

**Extending it.** Add the code point to `ORNAMENTS` (or `EMOJI`) in
`scripts/ornament_fonts.sh`, run it (it checks its sources' SHA-256 and
the pinned fontTools version, and writes the faces byte for byte), and
add it to the list `font/tests/ornament_tests.rs` holds the faces, the
script and the built-in styles to. The two licenses cannot share one
font file, which is why there are two faces. Each face's `info.txt` and
`misc/` record its source and license.

## Console

The `font` console verb is rebuilt on the document's one writer by
V1-C5 (#246); until then it refuses, naming its owner. Its family
completion is live: when the cursor sits in the family slot of
`font set ...`, the popup shows every family whose name starts with the
typed prefix (case-insensitive), each candidate row pre-shaped in that
very face.
