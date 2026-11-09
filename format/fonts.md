# Fonts

Mandala bundles every font it ships with, so the binary renders any
saved map without external assets: the same faces on native and in the
browser (CODE_CONVENTIONS §4), and a map that looks the same wherever
it is opened.

The font system (`baumhard::font::fonts`' `FONT_SYSTEM`) is built from
the compiled faces first. On native it then loads the host's installed
fonts, after every compiled face, and drops any that bears a compiled
family's name (below); the browser has none to load. The host's locale
and cosmic-text's per-platform fallback lists are not read. So text in
the default family, and text a map pins to a compiled family, are drawn
in the same faces on a machine with a hundred families installed, in a
container with none, and in the browser, wherever the compiled faces
hold the characters (#228). The host's fonts serve what no compiled
face holds, and are offered by `font list` and the console's
completion; a map naming one is #329.

## The default face

Text that pins no family — an `AppFont::Any` span, a section that sets
no `font`, every edge label, the connector's `·` — is drawn in the
**default face**: **Inter** 4.1, the variable face (`Inter Variable`,
`lib/baumhard/src/font/fonts/inter-font/`, `fallback::DEFAULT_FACE`).
cosmic-text resolves an unpinned span through its generic families
(`Family::SansSerif` for `Attrs::new()`, which every path asks for), and
the font system names the default face for all of them, so a host's
fonts never answer for the default family.

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
names only families that load, and draw, the same on every target. An
installed family resolves to the fallback face `AppFont::Any`, which has
no name a file could write back, and is refused the same way until #329
lets a map name one, and tells its reader what to install or pick in its
place when the family is missing. A span that names none is drawn in the
default face, which the file need not name.

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

**Compiled families are authoritative by name too** (ruling R30.1).
cosmic-text resolves a family by name, and among every face of that
name fontdb picks the best weight and slant: a host's Inter Variable
Italic would take every italic span of default text from the compiled
upright face, and the Noto Sans Hebrew a Debian desktop installs would
answer for the compiled one. So once the host's faces are loaded,
every one that bears a compiled family's name is removed
(`remove_shadowing_faces`). A pin, and the default family, then name the
compiled faces on every machine. The host's other families stay.

## Fallback

A character a span's face lacks is drawn in a fallback face. The font
system's fallback is one list, the same on every target
(`baumhard::font::fallback::fallback_families`), tried in this order:

| Family | Holds | Source | License |
|--------|-------|--------|---------|
| `Inter Variable` | the default face (above) | Inter 4.1 | SIL Open Font License 1.1 |
| `Mandala Ornaments` | `∙ ◈ ✦ ❦`; the box-drawing `─ ━ │ ┃ ┄ ┆ ═ ╭ ╮ ╯ ╰`; the chrome's `❯ ▌ ▏ ▸ ∼ ⋯` (with `❮ ∽`, their mirrored forms) | DejaVu Sans 2.37 | Bitstream Vera and Arev; DejaVu's changes public domain |
| `Mandala Emoji` | `🌸` | Noto Color Emoji 2.047 (its color bitmap) | SIL Open Font License 1.1 |
| `Noto Sans Hebrew`, `Noto Sans Devanagari`, `Noto Serif Tibetan`, `Noto Sans Egyptian Hieroglyphs` | their scripts | Noto | SIL Open Font License 1.1 |

Each script face is listed, so it is registered at every weight and met
at the span's own weight, before the last pass (below), where another
face that holds the script could come first: a host face at the span's
weight — FreeSerif Bold for bold Devanagari on a Debian desktop — or
Public Pixel, which holds Hebrew letters.

The two ornament faces hold the ornaments the demo maps draw (sides,
`repeat` rings), the built-in portal end's `◈`, the box-drawing
characters of testament's custom sides, and the glyphs of the app's own
chrome: the console's frame, prompt, cursor, gutter and completion
marker, the rubber band's corners and dashes, and the `body` presets —
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
  holds is looked for in every other face, in the order of
  cosmic-text's `FontMatchKey`: emoji faces first; then nearest the
  span in weight, in stretch and in slant; then lighter, then narrower;
  and only among faces equal in all of those, in the order they were
  loaded — the compiled faces in the order the build lists them, then,
  on native, the host's. So a host face draws a character no compiled
  face holds. A character only an unlisted (decorative) compiled face
  holds can be drawn first by a host face in three ways: by a host
  emoji face, at any weight and slant (`♠ ☺` in Noto Color Emoji on a
  Debian desktop); by a host face of the span's weight where the
  compiled one has another; and, for an italic span, by a host italic
  face (`┌ ░` in Liberation Serif Italic). Every glyph the app, its
  built-in styles and the shipped maps draw is held by a face its span
  names or a listed face, and tests hold all three to that, so none of
  them reaches this pass. One no face holds
  is drawn as the span face's `.notdef` box. Every path asks for one of
  the generic families the default face is named for, never
  `Family::Monospace`, on which cosmic-text tries every monospaced face
  before the list.
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
script and the built-in styles to. A glyph of the app's own chrome
belongs in a named constant, which
`renderer::tests::test_every_chrome_glyph_is_drawn_by_the_default_or_an_ornament_face`
holds to the default face or `Mandala Ornaments`. A code point the
default face holds is not added: the default face draws it, and the
test that holds the ornament faces apart from it refuses it. The two
licenses cannot share one font file, which is why there are two ornament faces. Each face's
`info.txt` and `misc/` record its source and license.

## What no compiled face draws

A character outside every compiled face is drawn on native in a host
face that holds it — the scripts no compiled face writes (Arabic,
Armenian, Georgian, Thai and the other scripts of South and Southeast
Asia but Devanagari and Tibetan, Chinese, Japanese and Korean) and every
emoji but `🌸` — and as a `.notdef` box where none does, and in the
browser, which has no host faces. A script or symbol is drawn the same
everywhere by compiling a face for it in.

## Console

`font family=<name>` sets the face of every selected element, on the
document's one writer, and takes exactly the names a map's `font` key
takes: a compiled family. An installed host font cannot be named by a
map yet (#329), so `family=` refuses it as the loader does. When the
cursor sits on the value of `font family=`, the popup shows every
family a map can name whose name starts with the typed prefix
(case-insensitive), each candidate row pre-shaped in that very face;
`font list` prints every loaded family, the host's included on native. A
`console_font` that names no compiled family is drawn in the default
face, with a `warn!` when it names no loaded family at all.
