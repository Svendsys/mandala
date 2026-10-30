#!/usr/bin/env bash
# Run one coverage-guided fuzz target for a time budget (design D29,
# V1-B8 #278; TEST_CONVENTIONS §T14).
#
# Usage:
#   scripts/fuzz.sh <target> <seconds> [libFuzzer options...]
#   scripts/fuzz.sh --list
#
# Builds `fuzz/` on the nightly its own `rust-toolchain.toml` pins (never
# the workspace's stable one, and never from `./test.sh`), then runs the
# target with `cargo fuzz run` over:
#
#   fuzz/corpus/<target>   what the run finds (written; not committed)
#   fuzz/seeds/<target>    the committed seed corpus
#   the surface's own inputs, read where they live, never copied:
#     load_*               lib/baumhard/src/document/format/fixtures
#                          lib/baumhard/src/document/tests/fuzz_findings
#                          maps/*.mandala.json
#     glyph_mutation_raw   lib/baumhard/src/gfx_structs/tests/fuzz_findings
#
# Each input is held to a time and a memory budget: `-timeout` (seconds
# per input) and `-rss_limit_mb`, and a length ceiling `-max_len` per
# surface. A crash, a timeout or the memory limit writes the input to
# fuzz/artifacts/<target>/ and exits non-zero. Turn such an input into a
# regression test before fixing it (TEST_CONVENTIONS §T14): minimize it
# with `cargo fuzz tmin <target> <artifact>` (from `fuzz/`), commit it
# beside its surface's other findings, and add the test that replays it.
#
# Needs: the nightly in fuzz/rust-toolchain.toml (rustup installs it on
# first use) and cargo-fuzz (`cargo install cargo-fuzz --version 0.13.2
# --locked`). Exit codes: 0 the budget ran out with no finding, 1 a
# finding (or a failed build), 2 a bad command line.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGETS=(load_raw load_structured flow_raw flow_structured glyph_mutation_raw glyph_mutation_structured)

usage() {
  sed -n '2,32p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
}

if [ "${1:-}" = "--list" ]; then
  printf '%s\n' "${TARGETS[@]}"
  exit 0
fi
if [ "$#" -lt 2 ]; then
  usage >&2
  exit 2
fi

TARGET="$1"
SECONDS_BUDGET="$2"
shift 2
case " ${TARGETS[*]} " in
  *" $TARGET "*) ;;
  *) echo "fuzz.sh: unknown target '$TARGET'; one of: ${TARGETS[*]}" >&2; exit 2 ;;
esac
if ! [[ "$SECONDS_BUDGET" =~ ^[0-9]+$ ]] || [ "$SECONDS_BUDGET" -eq 0 ]; then
  echo "fuzz.sh: the budget is a positive number of seconds, not '$SECONDS_BUDGET'" >&2
  exit 2
fi

# The longest input each surface is fed. The raw loader's must hold the
# largest committed seed (a deep chain with a link on every node);
# a structured input is choices, four bytes each.
case "$TARGET" in
  load_raw) MAX_LEN=65536 ;;
  load_structured) MAX_LEN=65536 ;;
  flow_raw) MAX_LEN=4096 ;;
  flow_structured) MAX_LEN=8192 ;;
  glyph_mutation_raw) MAX_LEN=16384 ;;
  glyph_mutation_structured) MAX_LEN=8192 ;;
esac

EXTRA=()
if [ "$TARGET" = load_raw ]; then
  # The format's keys and spellings, spliced whole by the mutator.
  EXTRA+=("-dict=$ROOT/fuzz/dicts/mandala_json.dict")
fi

CORPORA=("fuzz/corpus/$TARGET" "fuzz/seeds/$TARGET")
case "$TARGET" in
  load_*)
    CORPORA+=("lib/baumhard/src/document/format/fixtures" "lib/baumhard/src/document/tests/fuzz_findings")
    # The repository's maps in this format, once they exist (V1-C3):
    # read in place through a scratch directory of links, so nothing is
    # copied and a map added later is a seed without editing this file.
    MAPS="$(mktemp -d)"
    trap 'rm -rf "$MAPS"' EXIT
    for map in "$ROOT"/maps/*.mandala.json; do
      [ -e "$map" ] && ln -s "$map" "$MAPS/"
    done
    CORPORA+=("$MAPS")
    ;;
  glyph_mutation_raw)
    CORPORA+=("lib/baumhard/src/gfx_structs/tests/fuzz_findings")
    ;;
esac

cd "$ROOT"
mkdir -p "fuzz/corpus/$TARGET" "fuzz/artifacts/$TARGET"
# Paths for cargo fuzz, which runs from fuzz/.
ARGS=()
for corpus in "${CORPORA[@]}"; do
  case "$corpus" in
    /*) ARGS+=("$corpus") ;;
    *) ARGS+=("$ROOT/$corpus") ;;
  esac
done

cd "$ROOT/fuzz"
export CARGO_INCREMENTAL=0
cargo fuzz run "$TARGET" "${ARGS[@]}" -- \
  -max_total_time="$SECONDS_BUDGET" \
  -max_len="$MAX_LEN" \
  -timeout=20 \
  -rss_limit_mb=2048 \
  -artifact_prefix="$ROOT/fuzz/artifacts/$TARGET/" \
  -print_final_stats=1 \
  ${EXTRA[@]+"${EXTRA[@]}"} \
  "$@"
