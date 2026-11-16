#!/usr/bin/env bash
# Rebuild the compiled Scrapbook Chinese face (V1-B36, #355) from the face
# as published, byte for byte.
#
# Usage:
#   scripts/scrapbook_chinese_font.sh [ScrapbookChinese-5X6B.ttf]
#
# The published face (Helena Jole's Scrapbook Chinese, from
# https://www.fontspace.com/scrapbook-chinese-font-f4497) names its family,
# "Scrapbook Chinese", on both platforms it writes names for, but has no
# PostScript name (name ID 6). fontdb, which the font system is built on,
# refuses a face without one, with an error that speaks of the family
# ("font doesn't have a family name"): so the face was compiled in and
# never loaded, no span could be pinned to it, and every launch logged a
# WARN for it (#355).
#
# This script adds that name, "ScrapbookChinese-Regular", as one record on
# each of the face's two name platforms — Macintosh Roman, and Windows
# Symbol, the encoding of the face's character map, whose letters sit at
# U+F020..U+F0FF as a symbol font's do — and changes nothing else: every
# other table's contents are as published, and of `head` only the
# whole-file checksum moves. (fontTools writes the tables in its own order,
# at new offsets; their contents are what the face carries.)
# The family, full name and unique id, which the build's `AppFont` name is
# read from (`lib/baumhard/build.rs`), stay as published.
#
# The source is the published file, pinned by SHA-256. Without an
# argument it is read from git's object store, where the repository kept
# it, as committed, until this repair (blob SOURCE_BLOB below); a clone
# whose history no longer holds that object passes the file downloaded
# from the link above instead. The tool is pinned too, since another
# fontTools may write other bytes:
#
#   pip install fonttools==4.68.0
#
# Exit codes: 0 written, 1 the source is missing or the source or the tool
# is not the pinned one, 2 a bad command line.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
FACE="$ROOT/lib/baumhard/src/font/fonts/scrapbook-chinese-font/ScrapbookChinese-5X6B.ttf"

# It takes no options: an argument that begins with `-` is a bad command
# line (a file so named is passed as `./-name`).
if [ $# -gt 1 ] || [[ "${1:-}" == -* ]]; then
    echo "usage: $0 [ScrapbookChinese-5X6B.ttf]" >&2
    exit 2
fi

SOURCE_SHA256=1da927449e91ae29fb986cfece36ed1badc00eea188e371f2431fff9eb65451f
SOURCE_BLOB=f5713767c7f27edf0a7390bba6cb74376f1238e1
FONTTOOLS_VERSION=4.68.0
POSTSCRIPT_NAME=ScrapbookChinese-Regular

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

if [ $# -eq 1 ]; then
    SOURCE="$1"
    if [ ! -f "$SOURCE" ]; then
        echo "$SOURCE: no such file (the published ScrapbookChinese-5X6B.ttf)" >&2
        exit 1
    fi
else
    SOURCE="$WORK/source.ttf"
    if ! git -C "$ROOT" cat-file blob "$SOURCE_BLOB" > "$SOURCE" 2>/dev/null; then
        echo "git object $SOURCE_BLOB (the published face) is not in this clone:" >&2
        echo "download the face from https://www.fontspace.com/scrapbook-chinese-font-f4497 and pass its path" >&2
        exit 1
    fi
fi

# Read on stdin, so a path that begins with `-` is not taken for an option.
got="$(sha256sum < "$SOURCE" | cut -d' ' -f1)"
if [ "$got" != "$SOURCE_SHA256" ]; then
    echo "$SOURCE: SHA-256 $got, not the pinned $SOURCE_SHA256" >&2
    exit 1
fi
have="$(python3 -I -c 'import fontTools; print(fontTools.version)' 2>/dev/null || true)"
if [ "$have" != "$FONTTOOLS_VERSION" ]; then
    echo "fontTools ${have:-not installed}, not the pinned $FONTTOOLS_VERSION: pip install fonttools==$FONTTOOLS_VERSION" >&2
    exit 1
fi

# `recalcTimestamp` and `recalcBBoxes` off, so `head` and the glyph
# tables are not rewritten: only the `name` table is compiled anew.
python3 -I - "$SOURCE" "$WORK/face.ttf" "$POSTSCRIPT_NAME" <<'PY'
import sys
from fontTools.ttLib import TTFont

src, dst, postscript = sys.argv[1:]
font = TTFont(src, recalcTimestamp=False, recalcBBoxes=False)
names = font["name"]
for platform, encoding, language in ((1, 0, 0), (3, 0, 0x409)):
    names.setName(postscript, 6, platform, encoding, language)
font.save(dst)
PY

cp "$WORK/face.ttf" "$FACE"
echo "$(stat -c %s "$FACE") bytes  $(sha256sum "$FACE" | cut -d' ' -f1)  ${FACE#"$ROOT"/}"
