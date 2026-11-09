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
#     flow_raw, flow_structured, flow_structured_every_face
#                          lib/baumhard/src/flow/tests/fuzz_findings
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
# finding (or a failed build, or a fuzz/Cargo.lock that would have to
# change), 2 a bad command line.

set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGETS=(load_raw load_structured flow_raw flow_structured flow_structured_every_face glyph_mutation_raw glyph_mutation_structured projection_edits edits_raw edits_structured)

usage() {
  sed -n '2,35p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//'
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

# The longest input each surface is fed. A loader's must hold the
# largest committed seed, which since V1-C3 is a repository map read in
# place (`maps/testament.mandala.json`, 113 KiB); a structured input is
# otherwise choices, four bytes each. A flow target's length
# does not bound its work (flow_fuzz.rs; rulings R44.2, R45-R47): a
# frame's budget is max(400,000, 3,000 × glyphs) steps of all its work,
# overshot by one build's shaping and inking and a lay of each line, at
# most 32.3 µs a step under ASan (ASAN_STEP_NANOS), and an input's
# frames together may take 15 s, three quarters of -timeout=20: 464,396
# steps. Two frames at the floor are 25.8 s, so a flow target lays one
# frame an input, its text cut until that frame's work fits: at most
# MAX_FRAME_GLYPHS (150) glyphs in one face, 462,949 steps, 14.95 s.
# A Repeat ring round an outline is budgeted as a frame is (#310), and
# laid as an input's one frame (ring_work_steps), at most MAX_RING_GLYPHS
# glyphs (rulings B17.2a, B17.2b).
# Every input this script feeds a target
# fits its ceiling, which `test_every_fuzz_input_fits_its_targets_max_len`
# holds.
case "$TARGET" in
  load_raw) MAX_LEN=131072 ;;
  load_structured) MAX_LEN=131072 ;;
  flow_raw) MAX_LEN=4096 ;;
  flow_structured) MAX_LEN=8192 ;;
  flow_structured_every_face) MAX_LEN=8192 ;;
  glyph_mutation_raw) MAX_LEN=16384 ;;
  glyph_mutation_structured) MAX_LEN=8192 ;;
  projection_edits) MAX_LEN=4096 ;;
  # An edit's own seeds include the definition parts bound, 65,536
  # mutations and one past it (owner, format Q4 on #250): 1.6 MB of text.
  edits_raw) MAX_LEN=2097152 ;;
  edits_structured) MAX_LEN=4096 ;;
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
  flow_raw)
    CORPORA+=("lib/baumhard/src/flow/tests/fuzz_findings")
    ;;
  flow_structured)
    CORPORA+=("lib/baumhard/src/flow/tests/fuzz_findings")
    ;;
  flow_structured_every_face)
    # The flow findings as seeds, read over this target's own list of
    # faces (#297): inputs to start from, not findings replayed.
    CORPORA+=("lib/baumhard/src/flow/tests/fuzz_findings")
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
# `cargo fuzz` has no `--locked`: a package fuzz/Cargo.lock lacks would be
# resolved afresh, at the newest compatible version, and the lockfile
# rewritten without a word. Refuse to run instead whenever it would have
# to change; ./test.sh holds it to the workspace's lockfile
# (test_the_fuzz_lockfile_locks_what_the_workspace_locks).
if ! cargo metadata --locked --format-version 1 >/dev/null; then
  echo "fuzz.sh: fuzz/Cargo.lock is out of date; see test_the_fuzz_lockfile_locks_what_the_workspace_locks for how to refresh it" >&2
  exit 1
fi
cargo fuzz run "$TARGET" "${ARGS[@]}" -- \
  -max_total_time="$SECONDS_BUDGET" \
  -max_len="$MAX_LEN" \
  -timeout=20 \
  -rss_limit_mb=2048 \
  -artifact_prefix="$ROOT/fuzz/artifacts/$TARGET/" \
  -print_final_stats=1 \
  ${EXTRA[@]+"${EXTRA[@]}"} \
  "$@"
