#!/usr/bin/env bash
# Regenerate the two compiled ornament faces (V1-B11, #297) from their
# freely licensed sources, byte for byte.
#
# Usage:
#   scripts/ornament_fonts.sh [DejaVuSans.ttf] [NotoColorEmoji.ttf]
#
# The faces are what the shaper reaches first, after a span's own face,
# for the ornament glyphs the demo maps and the built-in styles draw
# (`baumhard::font::fonts::ORNAMENT_FALLBACK`, `format/fonts.md`
# "Ornament fallback"), so that every install, and the browser, draws
# the same glyph for them instead of whatever host face holds one:
#
#   mandala-ornaments-font/MandalaOrnaments-Regular.ttf
#     DejaVu Sans 2.37, subset to ORNAMENTS below. Bitstream Vera and
#     Arev licenses: a modified font may not be named "Bitstream",
#     "Vera", "Tavmjong Bah" or "Arev", hence "Mandala Ornaments".
#   mandala-emoji-font/MandalaEmoji-Regular.ttf
#     Noto Color Emoji 2.047, subset to EMOJI below (its CBDT bitmap at
#     109 ppem). SIL Open Font License 1.1, no Reserved Font Name.
#
# Two faces, not one: the two licenses cannot be merged into one font
# (the OFL asks a derivative to be distributed under the OFL alone), and
# cosmic-text reaches one face of a fallback family, so a second face
# under the same family name would be met only after every host face.
#
# To add a glyph, add its code point to ORNAMENTS (or EMOJI), run this
# script, and add it to `ORNAMENT_CODE_POINTS` (or `EMOJI_CODE_POINTS`)
# in `lib/baumhard/src/font/tests/ornament_tests.rs`, which holds this
# list, the faces' character maps and the demo maps to one another.
#
# The sources are pinned by SHA-256: Ubuntu 24.04's fonts-dejavu-core
# 2.37-8 and fonts-noto-color-emoji 2.047-0ubuntu0.24.04.1, at the paths
# below by default. The tool is pinned too, since another fontTools may
# write other bytes:
#
#   pip install fonttools==4.68.0
#
# Exit codes: 0 written, 1 a source or the tool is not the pinned one,
# 2 a bad command line.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FONTS="$ROOT/lib/baumhard/src/font/fonts"

DEJAVU="${1:-/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf}"
NOTO_EMOJI="${2:-/usr/share/fonts/truetype/noto/NotoColorEmoji.ttf}"
if [ $# -gt 2 ]; then
    echo "usage: $0 [DejaVuSans.ttf] [NotoColorEmoji.ttf]" >&2
    exit 2
fi

DEJAVU_SHA256=ae7b7855e115a5966d8b1b3f80f254ccc117ec86f9965e202ee2940453837280
NOTO_EMOJI_SHA256=93cdc4ee9aa40e2afceecc63da0ca05ec7aab4bec991ece51a6b52389f48a477
FONTTOOLS_VERSION=4.68.0

# space + ~ · ∙ ◈ ♪ ✦ ❦: the ornaments of the demo maps' sides, rings and
# connectors, and the built-in connector, link and portal-end glyphs
# (`document::defaults::BUILT_IN`), with the space an ornament's copy is
# parted by.
ORNAMENTS=U+0020,U+002B,U+007E,U+00B7,U+2219,U+25C8,U+266A,U+2726,U+2766
# 🌸, the emoji of the frames demo.
EMOJI=U+1F338

check_sha256() {
    local file="$1" want="$2" got
    got="$(sha256sum "$file" | cut -d' ' -f1)"
    if [ "$got" != "$want" ]; then
        echo "$file: SHA-256 $got, not the pinned $want" >&2
        exit 1
    fi
}

check_sha256 "$DEJAVU" "$DEJAVU_SHA256"
check_sha256 "$NOTO_EMOJI" "$NOTO_EMOJI_SHA256"
have="$(python3 -I -c 'import fontTools; print(fontTools.version)')"
if [ "$have" != "$FONTTOOLS_VERSION" ]; then
    echo "fontTools $have, not the pinned $FONTTOOLS_VERSION: pip install fonttools==$FONTTOOLS_VERSION" >&2
    exit 1
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Name IDs kept: 0-6, and 13 and 14, the license and its URL, which the
# sources' licenses ask every copy to carry. The source's timestamps are
# kept (pyftsubset recalculates none by default), so the output depends
# on the inputs alone.
pyftsubset "$DEJAVU" --unicodes="$ORNAMENTS" --name-IDs=0,1,2,3,4,5,6,13,14 \
    --drop-tables+=FFTM --output-file="$WORK/ornaments.ttf"
pyftsubset "$NOTO_EMOJI" --unicodes="$EMOJI" --name-IDs=0,1,2,3,4,5,6,13,14 \
    --output-file="$WORK/emoji.ttf"

# Rename each subset: family (1, and 4 its full name), unique id (3) and
# PostScript name (6). The PostScript name keeps "Emoji" for the emoji,
# which cosmic-text reads to sort a color face as one.
rename() {
    python3 -I - "$1" "$2" "$3" "$4" "$5" <<'PY'
import sys
from fontTools.ttLib import TTFont

src, dst, family, postscript, unique = sys.argv[1:]
font = TTFont(src, recalcTimestamp=False)
names = font["name"]
for record in list(names.names):
    if record.nameID in (1, 3, 4, 6, 16, 17):
        names.removeNames(nameID=record.nameID)
for name_id, text in ((1, family), (3, unique), (4, family), (6, postscript)):
    names.setName(text, name_id, 3, 1, 0x409)
font.save(dst)
PY
}

mkdir -p "$FONTS/mandala-ornaments-font" "$FONTS/mandala-emoji-font"
rename "$WORK/ornaments.ttf" "$FONTS/mandala-ornaments-font/MandalaOrnaments-Regular.ttf" \
    "Mandala Ornaments" "MandalaOrnaments-Regular" "Mandala Ornaments: a subset of DejaVu Sans 2.37"
rename "$WORK/emoji.ttf" "$FONTS/mandala-emoji-font/MandalaEmoji-Regular.ttf" \
    "Mandala Emoji" "MandalaEmoji-Regular" "Mandala Emoji: a subset of Noto Color Emoji 2.047"

for face in "$FONTS/mandala-ornaments-font/MandalaOrnaments-Regular.ttf" \
    "$FONTS/mandala-emoji-font/MandalaEmoji-Regular.ttf"; do
    echo "$(stat -c %s "$face") bytes  $(sha256sum "$face" | cut -d' ' -f1)  ${face#"$ROOT"/}"
done
