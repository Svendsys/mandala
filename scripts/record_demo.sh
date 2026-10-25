#!/usr/bin/env bash
# One-command demo recording: launch the app on a map, run a command script
# against it, record the part of the script you mark, assemble the frames
# into a GIF (or an MP4), check the result, and print the caption to fill in.
# The procedure is the one `.claude/skills/mandala-record/SKILL.md` teaches;
# the launch half is `.claude/skills/mandala-drive/SKILL.md`'s.
#
# Deliberately outside ./test.sh: it needs a live GPU adapter, and
# TEST_CONVENTIONS §T8/§T10 keep live wgpu out of the suite.
#
# Usage:
#   ./scripts/record_demo.sh <map> <script-file> <out.gif>
#   ./scripts/record_demo.sh --mp4 ... <out.mp4>   # H.264 through the host's ffmpeg
#
# Options (before the three arguments):
#   --headless | --xvfb | --window   what the app runs in (default: --headless
#                                    on Linux with no display, --window with one)
#   --size WxH       the app's frame size (default 800x600)
#   --fps N          recording rate, 1-60 (default 20); a script's own `@record fps=`
#                    wins
#   --width PX       scale the assembled file down to this many pixels wide
#                    (default 640; never scaled up)
#   --beat-ms N      the settled "before" and "after" hold, in ms (default 500)
#   --debug          dev-profile build            --no-build  use the binaries as built
#   --overwrite      replace an existing <out> and its <out stem>.frames/
#   --allow-in-repo  let <out> land inside this checkout (refused by default:
#                    recordings are review artifacts, never repo content)
#
# After the three arguments, `-- <k=v>...` passes options on to `mandalactl
# record assemble` (e.g. `-- max_frames=100 fps=12`).
#
# The script file, one command per line, blank lines and `#` comments ignored:
#   <mandalactl arguments>   run as `mandalactl <arguments>` — shell-quoted
#                            words, exactly as you would type them
#   @record [record_start params]   start recording here, then hold the "before" beat
#   @stop                           hold the "after" beat, then stop recording
# Exactly one @record ... @stop pair. Everything before @record is setup
# and is never in the picture. Under a virtual clock (a `--headless` app
# starts with one) frames are taken only while the clock steps, so put
# `clock.step ms=<n>` lines between the actions; the beats do their own.
#
# Output: <out>, plus <out stem>.frames/ (the PNG frames and manifest.json,
# for reviewing frames and for re-assembling; delete it when done).
#
# Exit codes: 0 made; 1 a step failed (the app's log is named); 2 the
# environment cannot run the app (what to install is printed); 3 bad
# command line.

set -euo pipefail

MODE=""
SIZE="800x600"
FPS=20
ASM_WIDTH=640
BEAT_MS=500
PROFILE="release"
BUILD=1
OVERWRITE=0
ALLOW_IN_REPO=0
FORMAT="gif"
ASM_EXTRA=()

usage() {
    sed -n '/^# Usage:/,/^# command line\./p' "$0" | sed 's/^# \{0,1\}//' >&2
}
die() { # <code> <message>
    local code="$1"
    shift
    echo "error: $*" >&2
    exit "$code"
}
need_arg() { [ "$2" -ge 2 ] || die 3 "$1 needs a value"; }

while [ $# -gt 0 ]; do
    case "$1" in
        --headless | --xvfb | --window)
            [ -z "$MODE" ] || [ "$MODE" = "${1#--}" ] || die 3 "--headless, --xvfb and --window each name what the app runs in; pick one"
            MODE="${1#--}"
            shift
            ;;
        --size) need_arg "$1" $#; SIZE="$2"; shift 2 ;;
        --fps) need_arg "$1" $#; FPS="$2"; shift 2 ;;
        --width) need_arg "$1" $#; ASM_WIDTH="$2"; shift 2 ;;
        --beat-ms) need_arg "$1" $#; BEAT_MS="$2"; shift 2 ;;
        --mp4) FORMAT="mp4"; shift ;;
        --debug) PROFILE="debug"; shift ;;
        --no-build) BUILD=0; shift ;;
        --overwrite) OVERWRITE=1; shift ;;
        --allow-in-repo) ALLOW_IN_REPO=1; shift ;;
        -h | --help) usage; exit 0 ;;
        -*) usage; die 3 "unknown option: $1" ;;
        *) break ;;
    esac
done
REPO="$(cd "$(dirname "$0")/.." && pwd)"
# shellcheck source=lib/mandala_app.sh
. "$REPO/scripts/lib/mandala_app.sh"
[ -n "$MODE" ] || MODE="$(mandala_default_mode)"
[ "$#" -ge 3 ] || { usage; die 3 "expected <map> <script-file> <out>"; }
MAP="$1"
SCRIPT="$2"
OUT="$3"
shift 3
if [ "$#" -gt 0 ]; then
    [ "$1" = "--" ] || die 3 "unexpected argument: $1 (options for \`record assemble\` go after --)"
    shift
    ASM_EXTRA=("$@")
fi

case "$SIZE" in
    [0-9]*x[0-9]*) ;;
    *) die 3 "--size is WxH, e.g. 800x600" ;;
esac
case "$FPS" in '' | *[!0-9]*) die 3 "--fps is a whole number" ;; esac
case "$ASM_WIDTH" in '' | *[!0-9]*) die 3 "--width is a whole number of pixels" ;; esac
case "$BEAT_MS" in '' | *[!0-9]*) die 3 "--beat-ms is a whole number of milliseconds" ;; esac
[ -f "$MAP" ] || die 3 "map file not found: $MAP"
[ -f "$SCRIPT" ] || die 3 "script file not found: $SCRIPT"
case "$OUT" in
    *.gif) [ "$FORMAT" = "gif" ] || die 3 "--mp4 wants an <out> ending .mp4, not $OUT" ;;
    *.mp4) [ "$FORMAT" = "mp4" ] || die 3 "<out> ends .mp4: pass --mp4 to say you want a video through ffmpeg" ;;
    *) die 3 "<out> must end .gif (or .mp4 with --mp4): $OUT" ;;
esac
if [ "$FORMAT" = "mp4" ]; then
    command -v ffmpeg >/dev/null 2>&1 || die 2 "--mp4 needs ffmpeg on PATH (the GIF needs nothing)"
fi

abs() { # <path>: absolute, its directory resolved; the file need not exist
    local dir
    dir="$(cd "$(dirname "$1")" 2>/dev/null && pwd)" || return 1
    printf '%s/%s' "$dir" "$(basename "$1")"
}
MAP="$(abs "$MAP")"
SCRIPT="$(abs "$SCRIPT")"
OUT="$(abs "$OUT")" || die 3 "the directory of <out> does not exist: $OUT"
FRAMES="${OUT%.*}.frames"

if [ "$ALLOW_IN_REPO" -eq 0 ]; then
    case "$OUT" in
        "$REPO"/*) die 3 "$OUT is inside the repo: recordings are review artifacts, not repo content (write it under /tmp, or pass --allow-in-repo)" ;;
    esac
fi
if [ "$OVERWRITE" -eq 0 ]; then
    [ ! -e "$OUT" ] || die 3 "$OUT exists; pass --overwrite to replace it"
    [ ! -e "$FRAMES" ] || die 3 "$FRAMES exists; pass --overwrite to replace it"
fi

# The script's shape, checked before anything is launched: one @record, one
# @stop after it.
recs="$(grep -c '^[[:space:]]*@record\([[:space:]]\|$\)' "$SCRIPT" || true)"
stops="$(grep -c '^[[:space:]]*@stop[[:space:]]*$' "$SCRIPT" || true)"
[ "$recs" -eq 1 ] && [ "$stops" -eq 1 ] || die 3 "$SCRIPT needs exactly one @record and one @stop (found $recs and $stops)"
[ "$(grep -n '^[[:space:]]*@record\([[:space:]]\|$\)' "$SCRIPT" | cut -d: -f1)" -lt "$(grep -n '^[[:space:]]*@stop[[:space:]]*$' "$SCRIPT" | cut -d: -f1)" ] \
    || die 3 "$SCRIPT has @stop before @record"

cd "$REPO"

# ---- Environment, build, launch ----------------------------------------

if [ "$(uname -s)" != "Linux" ] && [ "$MODE" = "xvfb" ]; then
    die 3 "Xvfb is the Linux recipe; this is $(uname -s)"
fi
mandala_check_packages 0 "the $MODE recipe needs"
if [ "$BUILD" -eq 1 ]; then mandala_build "$PROFILE"; else mandala_locate "$PROFILE"; fi
mandala_workdir mandala-rec
trap mandala_teardown EXIT
[ "$MODE" = "xvfb" ] && mandala_start_xvfb
mandala_launch "$MAP" --size "$SIZE"
"$CTL" clock.wait settled >/dev/null || fail "the app did not settle after loading the map"

ctl() { # <label> <mandalactl arguments...>: run one, naming the script line on failure
    local label="$1" out
    shift
    out="$("$CTL" "$@" 2>&1)" || fail "$label: mandalactl $* -> ${out:-exit $?}"
    printf '%s\n' "$out"
}

# A "beat": time the picture holds still. Under a virtual clock the frames
# come from stepping it; under a real one, from waiting for it.
beat() {
    local mode
    mode="$(field "$("$CTL" clock.status)" mode)"
    if [ "$mode" = '"virtual"' ]; then
        ctl "beat" clock.step "ms=$BEAT_MS" >/dev/null
    else
        "$CTL" clock.wait settled >/dev/null || fail "beat: the app did not settle"
        sleep "$(awk -v ms="$BEAT_MS" 'BEGIN { printf "%.3f", ms / 1000 }')"
    fi
}

# ---- Run the script ----------------------------------------------------

# --overwrite (or nothing there) was settled above: clear the way, since the
# app only records into a directory that is new or empty.
rm -rf "$FRAMES"
rm -f "$OUT"

recording=0
lineno=0
STOP=""
step "script $SCRIPT"
while IFS= read -r line || [ -n "$line" ]; do
    lineno=$((lineno + 1))
    trimmed="${line#"${line%%[![:space:]]*}"}"
    case "$trimmed" in '' | '#'*) continue ;; esac
    label="$(basename "$SCRIPT"):$lineno"
    case "$trimmed" in
        @record | @record[[:space:]]*)
            eval "words=(${trimmed#@record})" || fail "$label: cannot parse: $trimmed"
            fps_given=0
            for w in ${words[@]+"${words[@]}"}; do
                case "$w" in fps=*) fps_given=1 ;; esac
            done
            [ "$fps_given" -eq 1 ] && fps_arg=() || fps_arg=("fps=$FPS")
            step "$label: record start"
            ctl "$label" capture.record_start "$FRAMES" ${fps_arg[@]+"${fps_arg[@]}"} ${words[@]+"${words[@]}"}
            recording=1
            beat
            ;;
        @stop)
            beat
            step "$label: record stop"
            STOP="$(ctl "$label" capture.record_stop)"
            recording=0
            ;;
        @*) fail "$label: unknown directive: $trimmed" ;;
        *)
            eval "words=($trimmed)" || fail "$label: cannot parse: $trimmed"
            echo "+ mandalactl ${words[*]}"
            ctl "$label" "${words[@]}"
            ;;
    esac
done <"$SCRIPT"
[ "$recording" -eq 0 ] || fail "the script ended while still recording"

frames="$(field "$STOP" frames)"
dropped="$(field "$STOP" dropped_frames)"
duration="$(awk -v ms="$(field "$STOP" duration_ms)" 'BEGIN { printf "%.0f", ms }')"
[ -n "$frames" ] && [ "$frames" -ge 2 ] 2>/dev/null || fail "the recording holds ${frames:-no} frame(s): a recording of one frame proves nothing a screenshot would not (under a virtual clock, did the script step the clock?)"
pacing="$(sed -n 's/.*"pacing": *"\([a-z]*\)".*/\1/p' "$FRAMES/manifest.json" | head -n 1)"
rec_w="$(field "$STOP" width)"

# ---- Graceful quit -----------------------------------------------------

step "act.action Quit"
"$CTL" act.action Quit >/dev/null || fail "act.action Quit was not answered"
for _ in $(seq 1 50); do
    kill -0 "$APP_PID" 2>/dev/null || break
    sleep 0.1
done
kill -0 "$APP_PID" 2>/dev/null && fail "the app was still running 5 s after Quit"
quit_rc=0
wait "$APP_PID" || quit_rc=$?
APP_PID=""
[ "$quit_rc" -eq 0 ] || fail "the app exited $quit_rc after Quit, not 0"

# ---- Assemble and check ------------------------------------------------

step "assemble $FORMAT"
asm=(record assemble "$FRAMES" "out=$OUT" "format=$FORMAT")
# Never scale up: width= asks for a smaller picture.
if [ -n "$rec_w" ] && [ "$ASM_WIDTH" -gt 0 ] && [ "$rec_w" -gt "$ASM_WIDTH" ]; then
    asm+=("width=$ASM_WIDTH")
fi
[ "$OVERWRITE" -eq 1 ] && asm+=("overwrite=true")
asm+=(${ASM_EXTRA[@]+"${ASM_EXTRA[@]}"})
ASM_OUT="$("$CTL" "${asm[@]}" 2>&1)" || fail "mandalactl ${asm[*]} -> $ASM_OUT"
printf '%s\n' "$ASM_OUT"

[ -s "$OUT" ] || fail "$OUT is missing or empty"
case "$FORMAT" in
    gif) [ "$(head -c 3 "$OUT")" = "GIF" ] || fail "$OUT is not a GIF" ;;
    mp4) [ "$(dd if="$OUT" bs=1 skip=4 count=4 2>/dev/null)" = "ftyp" ] || fail "$OUT is not an MP4" ;;
esac
bytes="$(wc -c <"$OUT" | tr -d ' ')"
frames_out="$(field "$ASM_OUT" frames_out)"
dur_out="$(awk -v ms="$(field "$ASM_OUT" duration_ms)" 'BEGIN { printf "%.0f", ms }')"

# ---- The caption to fill in --------------------------------------------

echo
echo "RECORDED: $OUT ($bytes bytes, ${frames_out:-$frames} frames, ${dur_out:-$duration} ms, $pacing pacing)"
if [ "$bytes" -gt $((10 << 20)) ]; then
    echo "WARNING: over GitHub's 10 MiB inline limit — shrink it: --width 480, -- fps=12, a tighter region, or a shorter script"
fi
if [ "${dropped:-0}" -gt 0 ]; then
    echo "NOTE: $dropped frame(s) were due and not taken (see gaps in $FRAMES/manifest.json); the GIF holds the frame before each gap"
fi
echo "Frames and manifest: $FRAMES/ — LOOK at it before you post (first, middle and last frame), then delete the directory."
echo
out_w="$(field "$ASM_OUT" width)"
out_h="$(field "$ASM_OUT" height)"
repo_rel() { # <path>: relative to the repo when inside it, else as given
    case "$1" in "$REPO"/*) printf '%s' "${1#"$REPO"/}" ;; *) printf '%s' "$1" ;; esac
}
# The command that re-makes it: every option that is not the default, then
# the paths, then the assemble options.
repro="scripts/record_demo.sh"
[ "$MODE" = "$(mandala_default_mode)" ] || repro+=" --$MODE"
[ "$SIZE" = "800x600" ] || repro+=" --size $SIZE"
[ "$FPS" -eq 20 ] || repro+=" --fps $FPS"
[ "$ASM_WIDTH" -eq 640 ] || repro+=" --width $ASM_WIDTH"
[ "$BEAT_MS" -eq 500 ] || repro+=" --beat-ms $BEAT_MS"
[ "$FORMAT" = "gif" ] || repro+=" --mp4"
[ "$PROFILE" = "release" ] || repro+=" --debug"
repro+=" $(printf '%q %q %q' "$(repo_rel "$MAP")" "$(repo_rel "$SCRIPT")" "$(basename "$OUT")")"
[ ${#ASM_EXTRA[@]} -eq 0 ] || repro+=" -- ${ASM_EXTRA[*]}"

echo "---- caption (fill in the <...>, then post with the recording) ----"
cat <<EOF
**<behavior in one line>** — \`$(basename "$OUT")\`

Look at: <what to watch, and when in the clip — the on-screen sign of the behavior>.
If it were broken: <what would look different>.
Proof (text, read after the clip): <the query.* lines that show the state changed>.
Pacing: $pacing — <why: stepped = smooth and deterministic, not real-time feel; realtime = the speed a user would see>.
Map: \`$(basename "$MAP")\`, app $SIZE ($MODE), assembled ${out_w:-?}x${out_h:-?}, ${frames_out:-$frames} frames, ${dur_out:-$duration} ms.

Reproduce (from the repo root; the script is below):
\`\`\`sh
$repro
\`\`\`
\`\`\`text
$(cat "$SCRIPT")
\`\`\`
EOF
echo "---- end caption ----"
echo "PASS"
