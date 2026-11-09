# Fonts

Mandala bundles every font it ships with, so the binary renders any
saved map without external assets, and what is compiled in is *all*
that is available, on every target: the same faces on native and in
the browser (CODE_CONVENTIONS §4), and a map that looks the same
wherever it is opened.

The font system (`baumhard::font::fonts`' `FONT_SYSTEM`) is built from
the compiled faces and nothing else (#228). It reads nothing of the
host — not its installed fonts, not its fontconfig, not its locale — so
a machine with a hundred families installed, a container with none, and
the browser all shape a map's text in the same faces. `font list`,
`font set <family>` and the completion popup offer the compiled
families, and only those.

## The default face

Text that pins no family — an `AppFont::Any` span, a section that sets
no `font`, every edge label, the connector's `·` — is drawn in the
**default face**: **Inter** 4.1, the variable face (`Inter Variable`,
`lib/baumhard/src/font/fonts/inter-font/`, `fallback::DEFAULT_FACE`).
cosmic-text resolves an unpinned span through its generic families
(`Family::SansSerif` for `Attrs::new()`, `Family::Monospace` for the
`AttrsList` bridge), and the font system names the default face for all
of them.

Until #228 those generic names were cosmic-text's own, "Open Sans" and
"Noto Sans Mono", which no compiled face bears. On a Linux desktop the
text reached the host's DejaVu Sans through the fallback; in the
browser, or on a host with no fonts, it was drawn in a decorative face
(`212Keyboard`, `Skullphabet`) or as `.notdef` boxes, and the
connector's `·` drew nothing.

Why Inter:

| | |
|---|---|
| License | SIL Open Font License 1.1, no Reserved Font Name (`misc/LICENSE.txt`) — the license of most compiled faces |
| Size | 879,708 bytes, one file, unmodified from the 4.1 release (`info.txt` records its SHA-256) |
| Weights | a `wght` axis from 100 to 900: every weight a map can ask for is drawn at that weight, none snapped or synthesized |
| Italic | none compiled in: an italic span is the upright face skewed 14° by cosmic-text, as it was in DejaVu Sans on a desktop that had no oblique face |
| Coverage | Latin with its extensions, Greek, Cyrillic, and the punctuation, arrows (`→`), math (`≥ ×`) and geometric shapes (`◆ ◇`) the maps and the app's own text draw |

The alternatives weighed: DejaVu Sans covers everything above and is
what a Linux desktop drew before, but it ships as static faces (a
regular and a bold, 1.47 MB together) under the Bitstream Vera license;
Noto Sans's variable face is 2.0 MB and lacks the arrows and geometric
shapes; Open Sans's spans only 300 to 800 and lacks them too; Source
Sans 3 lacks `◇`. None of them draws the box-drawing characters but
DejaVu Sans, so those are in `Mandala Ornaments` (below).

A map may also pin `Inter Variable` by name, like any compiled family.

## Where a map names a font

A span's `font` names a family ([`schema.md`](./schema.md)); a span
without one inherits its element's, and so on up to the map's defaults.
Each name resolves through `baumhard::font::fonts::app_font_by_family`
to the build-time `AppFont` enum, and the loader refuses a name that
resolves to no compiled-in face (`LoadErrorKind::UnknownFont`), so a map
names only families that load, and draw, the same on every target. A
span that names none is drawn in the default face, which the file need
not name.

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
family whose only face is 400 draws every weight at 400. A variable
face has every weight its `wght` axis spans, so `Inter Variable` keeps
the weight asked for.

The reason is the shaper. cosmic-text keeps a pinned family only
through a face of exactly the weight asked for, or a variable face
whose axis spans it, and otherwise draws in another face that has that
weight — before #228 the CI runner's `DejaVuSans-ExtraLight` for
SilhousTwo at 200, now the default face, whose axis spans every weight.
Every shaping path resolves the weight before cosmic-text sees it: a
TextFlow span's style weight (`flow::span_attrs`, and the tab check
that measures with it), and the regular weight the measuring and region
paths ask for (`font::fonts::PinnedFamily::weight_for`). The slant needs
no resolving: the matcher keeps a pinned face whatever its slant. A
character no compiled face of the family holds is still drawn in a
fallback face, as it must be.

Ruling R30.1 removed from the font system every host face that bore a
compiled family's name — the Noto Sans Hebrew a Debian desktop installs,
met before the compiled one. With no host face in the font system there
is nothing left to remove, and a pin names the compiled faces on every
machine by construction.

## Fallback

A character a span's face lacks is drawn in a fallback face. The font
system's fallback is one list, the same on every target
(`baumhard::font::fallback::fallback_families`), tried in this order:

| Family | Holds | Source | License |
|--------|-------|--------|---------|
| `Inter Variable` | the default face (above) | Inter 4.1 | SIL Open Font License 1.1 |
| `Mandala Ornaments` | `∙ ◈ ✦ ❦`, and the box-drawing `─ ━ │ ┃ ╭ ╮ ╯ ╰` | DejaVu Sans 2.37 | Bitstream Vera and Arev; DejaVu's changes public domain |
| `Mandala Emoji` | `🌸` | Noto Color Emoji 2.047 (its color bitmap) | SIL Open Font License 1.1 |
| `Noto Sans Hebrew`, `Noto Sans Devanagari`, `Noto Serif Tibetan`, `Noto Sans Egyptian Hieroglyphs` | their scripts | Noto | SIL Open Font License 1.1 |

The two ornament faces hold the ornaments the demo maps draw (sides,
`repeat` rings), the built-in portal end's `◈`, and the box-drawing
characters of the console's frame and of testament's custom sides —
**and no code point the default face holds** (#228). So a letter, digit,
space or punctuation mark is drawn by a text face, never by an ornament
face, whatever face a span pins: a decorative face that lacks a space or
a `·` gets the default face's. Until #228 `Mandala Ornaments` also held
the space and `+ ~ · ♪`, and the spaces of default text on a desktop
were drawn in it: at the bold weight, 0.03 em narrower than the bold
text face's own (#323).

How it holds:

- **One list, ahead of everything else.** `FONT_SYSTEM` is built with a
  cosmic-text `Fallback` whose common list is the table above and whose
  script lists are empty, so a run of any script, on any target, tries
  the same faces in the same order. cosmic-text's platform lists, which
  differ between Linux, Windows, macOS and the browser and name host
  families, are not consulted. A character none of the listed faces
  holds is looked for in every other compiled face, in the order the
  build lists them; one no compiled face holds is drawn as the span
  face's `.notdef` box.
- **At every weight.** cosmic-text matches a fallback family only at the
  exact weight a span asks for, or within a variable face's axis. The
  default face's axis spans every weight; each static fallback face is
  registered at every CSS weight from 100 to 900 instead: the same
  outlines under each, no weight synthesized, as a single-weight family
  draws every weight at its own (R30). The slant needs no registering,
  since the matcher does not filter on it: an italic span's glyph is
  drawn by the upright face, which cosmic-text skews by 14°
  (`FAKE_ITALIC`).
- **A span's own face first.** A pinned family that holds an ornament
  draws it in its own design, and text that pins no family draws what
  the default face holds — `♪`, say — in the default face.

**Extending it.** Add the code point to `ORNAMENTS` (or `EMOJI`) in
`scripts/ornament_fonts.sh`, run it (it checks its sources' SHA-256 and
the pinned fontTools version, and writes the faces byte for byte), and
add it to the list `font/tests/ornament_tests.rs` holds the faces, the
script and the built-in styles to. A code point the default face holds
is not added: the default face draws it, and the test that holds the
ornament faces apart from it refuses it. The two licenses cannot share
one font file, which is why there are two ornament faces. Each face's
`info.txt` and `misc/` record its source and license.

## What no compiled face draws

A character outside every compiled face is drawn as a `.notdef` box, on
every target. Since #228 that includes native: the scripts no compiled
face writes — Arabic, Armenian, Georgian, Thai and the other scripts of
South and Southeast Asia but Devanagari and Tibetan, Chinese, Japanese
and Korean — and every emoji but `🌸`, which a desktop used to draw in
its installed fonts. The browser never could. A script or symbol is
supported by compiling a face for it in, which then draws it the same
everywhere.

## Console

The `font` console verb is rebuilt on the document's one writer by
V1-C5 (#246); until then it refuses, naming its owner. Its family
completion is live: when the cursor sits in the family slot of
`font set ...`, the popup shows every compiled family whose name starts
with the typed prefix (case-insensitive), each candidate row pre-shaped
in that very face. A `console_font` that names no compiled family
falls back to the default face with a `warn!`.
