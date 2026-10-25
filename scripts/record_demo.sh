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
#   --beat-ms N      the settled "before" and "after" hold, in ms, 1 to 1600000
#                    (default 500; a virtual clock's step cap)
#   --debug          dev-profile build            --no-build  use the binaries as built
#   --overwrite      replace an existing <out> and its <out stem>.frames/ — only
#                    once the new recording has succeeded; a failed run leaves
#                    the old one untouched
#   --allow-in-repo  let <out> land inside this checkout (refused by default:
#                    recordings are review artifacts, never repo content)
#
# After the three arguments, `-- <k=v>...` passes options on to `mandalactl
# record assemble` (e.g. `-- max_frames=100 fps=12`).
#
# The script file, one command per line; blank lines and `#` comments ignored
# (a `#` that starts a word ends the line, as in a shell):
#   <mandalactl arguments>   run as `mandalactl <arguments>`. Words split the
#                            way a shell's quoting splits them — 'single',
#                            "double", backslash — and nothing more: no
#                            `$var`, no `$(...)`, no globs, no `;` or `|`;
#                            they are ordinary characters, so a script is
#                            data and running someone else's runs only
#                            mandalactl. (`raw -` has no stdin to read.)
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
case "$FPS" in '' | *[!0-9]*) die 3 "--fps is a whole number, 1 to 60" ;; esac
{ [ "$FPS" -ge 1 ] && [ "$FPS" -le 60 ]; } || die 3 "--fps is 1 to 60"
case "$ASM_WIDTH" in '' | *[!0-9]*) die 3 "--width is a whole number of pixels" ;; esac
case "$BEAT_MS" in '' | *[!0-9]*) die 3 "--beat-ms is a whole number of milliseconds, 1 to 1600000" ;; esac
# The app's own clock.step bound (STEP_MS_RANGE): a beat under a virtual
# clock is one step, so a value it would refuse is refused here, before the
# build and the launch.
{ [ "$BEAT_MS" -ge 1 ] && [ "$BEAT_MS" -le 1600000 ]; } || die 3 "--beat-ms is 1 to 1600000"
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

# <line>: split into WORDS the way a shell's quoting splits, and nothing
# more — 'single' quotes, "double" quotes (backslash escapes only \" and \\
# inside), a backslash outside quotes, whitespace (a CR included) between
# words, and an unquoted `#` at the start of a word ending the line. No
# expansion of any kind: `$`, backticks, `;`, `|` and `*` are ordinary
# characters, which is what makes a script safe to run unread. Fails on an
# unterminated quote or a trailing backslash.
split_words() {
    local s="$1" c word="" inword=0 q="" i=0 n=${#1}
    WORDS=()
    while [ "$i" -lt "$n" ]; do
        c="${s:i:1}"
        if [ "$q" = "'" ]; then
            if [ "$c" = "'" ]; then q=""; else word+="$c"; fi
        elif [ "$q" = '"' ]; then
            if [ "$c" = '"' ]; then
                q=""
            elif [ "$c" = '\' ] && [ $((i + 1)) -lt "$n" ] && { [ "${s:i+1:1}" = '"' ] || [ "${s:i+1:1}" = '\' ]; }; then
                i=$((i + 1))
                word+="${s:i:1}"
            else
                word+="$c"
            fi
        elif [ "$c" = "'" ] || [ "$c" = '"' ]; then
            q="$c"
            inword=1
        elif [ "$c" = '\' ]; then
            i=$((i + 1))
            [ "$i" -lt "$n" ] || return 1
            word+="${s:i:1}"
            inword=1
        elif [[ "$c" == [[:space:]] ]]; then
            if [ "$inword" -eq 1 ]; then WORDS+=("$word"); word=""; inword=0; fi
        elif [ "$c" = "#" ] && [ "$inword" -eq 0 ]; then
            break
        else
            word+="$c"
            inword=1
        fi
        i=$((i + 1))
    done
    [ -z "$q" ] || return 1
    if [ "$inword" -eq 1 ]; then WORDS+=("$word"); fi
    return 0
}

# The script's shape, checked before anything is launched: every line
# parses, and there is one @record and, after it, one @stop.
recs=0 stops=0 rec_line=0 stop_line=0 n=0
while IFS= read -r -u 3 line || [ -n "$line" ]; do
    n=$((n + 1))
    split_words "$line" || die 3 "$SCRIPT:$n: an unterminated quote or a trailing backslash: $line"
    [ ${#WORDS[@]} -gt 0 ] || continue
    case "${WORDS[0]}" in
        @record) recs=$((recs + 1)); rec_line=$n ;;
        @stop)
            [ ${#WORDS[@]} -eq 1 ] || die 3 "$SCRIPT:$n: @stop takes no arguments"
            stops=$((stops + 1)); stop_line=$n
            ;;
        @*) die 3 "$SCRIPT:$n: unknown directive: ${WORDS[0]}" ;;
    esac
done 3<"$SCRIPT"
[ "$recs" -eq 1 ] && [ "$stops" -eq 1 ] || die 3 "$SCRIPT needs exactly one @record and one @stop (found $recs and $stops)"
[ "$rec_line" -lt "$stop_line" ] || die 3 "$SCRIPT has @stop before @record"

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
    out="$("$CTL" "$@" 2>&1 </dev/null)" || fail "$label: mandalactl $* -> ${out:-exit $?}"
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

# The recording goes to a staging dir in the work dir and is published over
# <out> and <out stem>.frames/ only after everything has passed, so a failed
# run — a setup line that does not apply, a truncated recording — never
# costs the recording a --overwrite was replacing. (The app only records
# into a directory that is new or empty, which the staging dir is.)
STAGE_FRAMES="$WORK/frames"
STAGE_OUT="$WORK/out.$FORMAT"

recording=0
lineno=0
STOP=""
step "script $SCRIPT"
while IFS= read -r -u 3 line || [ -n "$line" ]; do
    lineno=$((lineno + 1))
    split_words "$line" || fail "$(basename "$SCRIPT"):$lineno: cannot parse: $line"
    [ ${#WORDS[@]} -gt 0 ] || continue
    label="$(basename "$SCRIPT"):$lineno"
    case "${WORDS[0]}" in
        @record)
            params=("${WORDS[@]:1}")
            fps_arg=("fps=$FPS")
            for w in ${params[@]+"${params[@]}"}; do
                case "$w" in fps=*) fps_arg=() ;; esac
            done
            step "$label: record start"
            ctl "$label" capture.record_start "$STAGE_FRAMES" ${fps_arg[@]+"${fps_arg[@]}"} ${params[@]+"${params[@]}"}
            recording=1
            beat
            ;;
        @stop)
            beat
            step "$label: record stop"
            STOP="$(ctl "$label" capture.record_stop)"
            recording=0
            ;;
        *)
            echo "+ mandalactl ${WORDS[*]}"
            ctl "$label" "${WORDS[@]}"
            ;;
    esac
done 3<"$SCRIPT"
[ "$recording" -eq 0 ] || fail "the script ended while still recording"

frames="$(field "$STOP" frames)"
dropped="$(field "$STOP" dropped_frames)"
stopped="$(field "$STOP" stopped)"
duration="$(awk -v ms="$(field "$STOP" duration_ms)" 'BEGIN { printf "%.0f", ms }')"
# A recording that ended itself (max_frames, the disk budget) is cut off
# before the behavior ended: evidence that stops early is worse than none.
[ "$stopped" = '"client"' ] || fail "the recording ended by itself ($stopped) before @stop, so it is cut short: raise @record max_frames= (up to 3600), lower fps=, or shorten the script"
[ -n "$frames" ] && [ "$frames" -ge 2 ] 2>/dev/null || fail "the recording holds ${frames:-no} frame(s): a recording of one frame proves nothing a screenshot would not (under a virtual clock, did the script step the clock?)"
pacing="$(sed -n 's/.*"pacing": *"\([a-z]*\)".*/\1/p' "$STAGE_FRAMES/manifest.json" | head -n 1)"
rec_w="$(field "$STOP" width)"

# ---- Graceful quit -----------------------------------------------------

mandala_quit

# ---- Assemble and check ------------------------------------------------

step "assemble $FORMAT"
asm=(record assemble "$STAGE_FRAMES" "out=$STAGE_OUT" "format=$FORMAT")
# Never scale up: width= asks for a smaller picture.
if [ -n "$rec_w" ] && [ "$ASM_WIDTH" -gt 0 ] && [ "$rec_w" -gt "$ASM_WIDTH" ]; then
    asm+=("width=$ASM_WIDTH")
fi
asm+=(${ASM_EXTRA[@]+"${ASM_EXTRA[@]}"})
ASM_OUT="$("$CTL" "${asm[@]}" 2>&1)" || fail "mandalactl ${asm[*]} -> $ASM_OUT"
[ -s "$STAGE_OUT" ] || fail "$STAGE_OUT is missing or empty"
case "$FORMAT" in
    gif) [ "$(head -c 3 "$STAGE_OUT")" = "GIF" ] || fail "$STAGE_OUT is not a GIF" ;;
    mp4) [ "$(dd if="$STAGE_OUT" bs=1 skip=4 count=4 2>/dev/null)" = "ftyp" ] || fail "$STAGE_OUT is not an MP4" ;;
esac
bytes="$(wc -c <"$STAGE_OUT" | tr -d ' ')"

# Everything passed: now, and only now, replace what was there.
step "publish"
rm -rf "$FRAMES"
mv "$STAGE_FRAMES" "$FRAMES"
mv -f "$STAGE_OUT" "$OUT"
printf '%s\n' "${ASM_OUT//"$STAGE_OUT"/"$OUT"}"
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
repro+=" --$MODE"
[ "$SIZE" = "800x600" ] || repro+=" --size $SIZE"
[ "$FPS" -eq 20 ] || repro+=" --fps $FPS"
[ "$ASM_WIDTH" -eq 640 ] || repro+=" --width $ASM_WIDTH"
[ "$BEAT_MS" -eq 500 ] || repro+=" --beat-ms $BEAT_MS"
[ "$FORMAT" = "gif" ] || repro+=" --mp4"
[ "$PROFILE" = "release" ] || repro+=" --debug"
repro+=" $(printf '%q %q %q' "$(repo_rel "$MAP")" "$(repo_rel "$SCRIPT")" "$OUT")"
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
