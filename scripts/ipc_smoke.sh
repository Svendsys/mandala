#!/usr/bin/env bash
# End-to-end smoke test of the --ipc drive-and-see loop: launch the app
# (with no window when there is no display, `--headless`), ping it, check
# the map loaded, take a window-sized screenshot and a region screenshot
# around one node, check the PNGs and their geometry sidecars, check that
# a one-shot `--screenshot` launch writes the same picture, and tear it
# all down. The procedure is the one `.claude/skills/mandala-drive/SKILL.md`
# teaches; this script is that skill's proof that it still works.
#
# Deliberately outside ./test.sh: it needs the built binaries, a socket
# and a process to drive, and it checks the launch, IPC and capture loop
# end to end. The suite's GPU-backed tests (TEST_CONVENTIONS §T8) build
# the app in-process on the same adapter; this script is the other half.
#
# Usage:
#   ./scripts/ipc_smoke.sh                   # testament, release build: a window when
#                                            # there is a display, --headless when not
#   ./scripts/ipc_smoke.sh --headless        # no window even when there is a display
#   ./scripts/ipc_smoke.sh --xvfb            # a real window under its own Xvfb
#   ./scripts/ipc_smoke.sh --compare-xvfb    # also: a headless one-shot and an Xvfb one
#                                            # of the same map must be byte-identical
#   ./scripts/ipc_smoke.sh --map <path>      # another map
#   ./scripts/ipc_smoke.sh --debug           # dev-profile build (logs info!/debug!)
#   ./scripts/ipc_smoke.sh --keep            # keep the work dir on success too
#
# Exit codes: 0 pass; 1 a check failed (the work dir with the app's log
# is kept and named); 2 the environment cannot run the app (what to
# install is printed); 3 bad command line.

set -euo pipefail

MAP=""
PROFILE="release"
FORCE_XVFB=0
FORCE_HEADLESS=0
COMPARE_XVFB=0
KEEP=0

usage() {
    sed -n '/^# Usage:/,/^# Exit codes/p' "$0" | sed 's/^# \{0,1\}//' >&2
}

while [ $# -gt 0 ]; do
    case "$1" in
        --map)
            [ $# -ge 2 ] || { echo "error: --map needs a path" >&2; exit 3; }
            MAP="$2"
            shift 2
            ;;
        --debug) PROFILE="debug"; shift ;;
        --xvfb) FORCE_XVFB=1; shift ;;
        --headless) FORCE_HEADLESS=1; shift ;;
        --compare-xvfb) COMPARE_XVFB=1; shift ;;
        --keep) KEEP=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) echo "error: unknown argument: $1" >&2; usage; exit 3 ;;
    esac
done

# A relative --map names a file relative to where the script was run from,
# so it is made absolute before the cd to the repo root below; the default
# is the repo's own testament.
[ -n "$MAP" ] || MAP="$(cd "$(dirname "$0")/.." && pwd)/maps/testament.mindmap.json"
case "$MAP" in
    /*) ;;
    *) MAP="$PWD/$MAP" ;;
esac
[ -f "$MAP" ] || { echo "error: map file not found: $MAP" >&2; exit 3; }

cd "$(dirname "$0")/.."
# shellcheck source=lib/mandala_app.sh
. scripts/lib/mandala_app.sh

# ---- Environment preflight --------------------------------------------

# What the app runs in: a window on the display that is there, a window on
# our own Xvfb, or no window at all (`--headless`). With no display the
# default is no window — the recipe for a container or CI. Elsewhere than
# Linux (macOS) the desktop is not announced through $DISPLAY, so a missing
# one says nothing there.
MODE=""
if [ "$FORCE_HEADLESS" -eq 1 ] && [ "$FORCE_XVFB" -eq 1 ]; then
    echo "error: --headless and --xvfb each name what the app runs in; pick one" >&2
    exit 3
elif [ "$FORCE_HEADLESS" -eq 1 ]; then
    MODE="headless"
elif [ "$FORCE_XVFB" -eq 1 ]; then
    MODE="xvfb"
else
    MODE="$(mandala_default_mode)"
fi
if [ "$(uname -s)" != "Linux" ] && { [ "$MODE" = "xvfb" ] || [ "$COMPARE_XVFB" -eq 1 ]; }; then
    echo "error: Xvfb is the Linux recipe; this is $(uname -s)" >&2
    exit 3
fi
USE_XVFB=0
[ "$MODE" = "xvfb" ] && USE_XVFB=1

# Packages are checked where the script stands up the graphics stack itself
# — no window, or its own X server.
what="the $MODE recipe needs"
[ "$COMPARE_XVFB" -eq 1 ] && what="the $MODE recipe and --compare-xvfb need"
mandala_check_packages "$COMPARE_XVFB" "$what"

# ---- Build -------------------------------------------------------------

mandala_build "$PROFILE"

# ---- Work dir and teardown ---------------------------------------------

mandala_workdir mandala-smoke
trap mandala_teardown EXIT

# ---- Display -----------------------------------------------------------

[ "$USE_XVFB" -eq 1 ] && mandala_start_xvfb

# ---- Launch and liveness -----------------------------------------------

mandala_launch "$MAP"
"$CTL" ping

# ---- The map loaded ----------------------------------------------------

step "query.document"
doc="$("$CTL" query.document)" || fail "query.document failed"
echo "$doc"
nodes="$(awk -F': ' '$1 == "counts.nodes" { print $2 }' <<<"$doc")"
if [ -z "$nodes" ] || [ "$nodes" -le 0 ]; then fail "the document reports no nodes"; fi

# ---- Helpers -----------------------------------------------------------

json_string() { # <text>: the text as a JSON string literal (backslash and quote escaped)
    printf '"%s"' "$(printf '%s' "$1" | sed 's/\\/\\\\/g; s/"/\\"/g')"
}

[ "$(field "$doc" file_path)" = "$(json_string "$MAP")" ] || fail "file_path is $(field "$doc" file_path), not $MAP"

png_size() { # <png>: "W H" from the IHDR chunk
    od -An -tu1 -j16 -N8 "$1" | awk '{ print $1*16777216 + $2*65536 + $3*256 + $4, $5*16777216 + $6*65536 + $7*256 + $8 }'
}

check_png() { # <png> <expected width> <expected height>
    local png="$1" w="$2" h="$3"
    [ -s "$png" ] || fail "$png is missing or empty"
    [ "$(od -An -tx1 -N8 "$png" | tr -d ' \n')" = "89504e470d0a1a0a" ] || fail "$png is not a PNG"
    [ "$(png_size "$png")" = "$w $h" ] || fail "$png is $(png_size "$png"), the reply said $w $h"
    if command -v python3 >/dev/null 2>&1; then
        local rc=0
        python3 -I - "$png" <<'PY' || rc=$?
import struct, sys, zlib

# Exit 0: more than one color. Exit 3: one flat color. Exit 2: not decodable.
try:
    data = open(sys.argv[1], "rb").read()
    pos, idat, w, h, bpp = 8, b"", 0, 0, 0
    while pos < len(data):
        n, kind = struct.unpack(">I4s", data[pos:pos + 8])
        body = data[pos + 8:pos + 8 + n]
        if kind == b"IHDR":
            w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
            if depth != 8 or ctype not in (2, 6):
                raise ValueError(f"unexpected PNG format: bit depth {depth}, color type {ctype}")
            bpp = 3 if ctype == 2 else 4
        elif kind == b"IDAT":
            idat += body
        pos += 12 + n
    raw, stride = zlib.decompress(idat), w * bpp
    if len(raw) != h * (stride + 1):
        raise ValueError(f"{len(raw)} bytes of pixel data, expected {h * (stride + 1)}")
except (OSError, ValueError, struct.error, zlib.error) as err:
    print(f"   cannot decode: {err}", file=sys.stderr)
    sys.exit(2)
# Rows are unfiltered in full (each depends on the one above), sampled every
# 7th pixel, and the scan stops at the second color.
prev, colors = bytearray(stride), set()
for y in range(h):
    f, line = raw[y * (stride + 1)], bytearray(raw[y * (stride + 1) + 1:(y + 1) * (stride + 1)])
    for i in range(stride):
        a = line[i - bpp] if i >= bpp else 0
        b, c = prev[i], prev[i - bpp] if i >= bpp else 0
        if f == 1: line[i] = (line[i] + a) & 255
        elif f == 2: line[i] = (line[i] + b) & 255
        elif f == 3: line[i] = (line[i] + (a + b) // 2) & 255
        elif f == 4:
            p = a + b - c
            pa, pb, pc = abs(p - a), abs(p - b), abs(p - c)
            line[i] = (line[i] + (a if pa <= pb and pa <= pc else b if pb <= pc else c)) & 255
    for x in range(0, stride, bpp * 7):
        colors.add(bytes(line[x:x + 3]))
    if len(colors) > 1:
        print(f"   second color found in row {y}")
        sys.exit(0)
    prev = line
sys.exit(3)
PY
        case "$rc" in
            0) ;;
            3) fail "$png is a single flat color: nothing visible was drawn" ;;
            *) fail "$png could not be decoded (see above)" ;;
        esac
    else
        echo "   (python3 not found: pixel-content check skipped; the sidecar check below still runs)"
    fi
    local sidecar="$png.geometry.json"
    [ -s "$sidecar" ] || fail "$sidecar is missing or empty"
    grep -q "\"surface\":{\"width\":$w,\"height\":$h," "$sidecar" || fail "$sidecar does not describe a ${w}x$h image"
    grep -q '"nodes":\[{"id":' "$sidecar" || fail "$sidecar lists no drawn node"
}

capture() { # <png> [params...]: capture.screenshot, retrying a contended font system
    local out tries=0 skipped
    while :; do
        out="$("$CTL" capture.screenshot "$@")" || fail "capture.screenshot $* failed"
        [ "$(field "$out" rendered)" = "true" ] && break
        skipped="$(field "$out" skipped)"
        tries=$((tries + 1))
        # The one transient skip: the font system was held for that frame.
        if [ "$skipped" = '"font_system_contended"' ] && [ "$tries" -lt 5 ]; then
            echo "   skipped: font_system_contended, retrying" >&2
            sleep 0.2
            continue
        fi
        fail "the frame was not rendered: skipped $skipped"
    done
    printf '%s\n' "$out"
}

# ---- Window-sized screenshot -------------------------------------------

step "clock.wait settled"
"$CTL" clock.wait settled || fail "the app did not settle"

step "capture.screenshot (window size)"
shot="$(capture "$WORK/window.png")" || exit 1
echo "$shot"
win_w="$(field "$shot" width)"; win_h="$(field "$shot" height)"
zoom="$(field "$shot" camera.zoom)"
check_png "$WORK/window.png" "$win_w" "$win_h"

# ---- Region screenshot around one node ---------------------------------

# A node the window shows whole, framed by a region centered on it at the
# window's own zoom: zoom-dependent visibility cannot hide it, and it fits.
# (No `| head`: under pipefail its early exit would SIGPIPE the grep.)
whole="$(grep -o '{"id":"[^"]*","rect":{[^}]*},"clipped":false' "$WORK/window.png.geometry.json" || true)"
node="${whole%%$'\n'*}"
node="${node#\{\"id\":\"}"
node="${node%%\"*}"
if [ -z "$node" ]; then
    echo "   no node is wholly inside the window capture; region check skipped"
else
    step "scene.node_geometry $node"
    geo="$("$CTL" scene.node_geometry "$node")" || fail "scene.node_geometry $node failed"
    x="$(field "$geo" canvas.x)"; y="$(field "$geo" canvas.y)"
    w="$(field "$geo" canvas.w)"; h="$(field "$geo" canvas.h)"
    if [ -z "$x" ] || [ -z "$y" ] || [ -z "$w" ] || [ -z "$h" ]; then
        fail "scene.node_geometry gave no canvas rect for $node"
    fi
    region="$(awk -v x="$x" -v y="$y" -v w="$w" -v h="$h" -v W="$win_w" -v H="$win_h" -v z="$zoom" \
        'BEGIN { rw = W / z; rh = H / z;
                 printf "{\"x\":%.4f,\"y\":%.4f,\"w\":%.4f,\"h\":%.4f}", x + w / 2 - rw / 2, y + h / 2 - rh / 2, rw, rh }')"

    camera_before="$("$CTL" query.camera)"
    step "capture.screenshot region=$region"
    shot="$(capture "$WORK/region.png" region:="$region")" || exit 1
    echo "$shot"
    check_png "$WORK/region.png" "$(field "$shot" width)" "$(field "$shot" height)"
    grep -q "{\"id\":\"$node\",\"rect\":{[^}]*},\"clipped\":false" "$WORK/region.png.geometry.json" \
        || fail "node $node is not wholly inside the region capture framed around it"
    [ "$("$CTL" query.camera)" = "$camera_before" ] || fail "the capture moved the live camera"
fi

# ---- A pan ends on its own button's release ---------------------------

# The left and right release arms end a pan behind a `mem::replace` with
# no pure route in front of them, so no renderer-free test can reach
# them (#153, TEST_CONVENTIONS §T8), and no GPU-backed test drives them
# through `input.*` yet; until one does, this is the one place they
# run. A middle press arms a pan the middle button holds; the left and
# right releases must leave it, and the middle release must end it.
pan_state() { # the drag state's kind and owner, as `query.mode` reports them
    local mode
    mode="$("$CTL" query.mode)" || fail "query.mode failed"
    printf '%s %s %s' "$(field "$mode" drag_state.kind)" "$(field "$mode" drag_state.owner)" \
        "$(field "$mode" drag_state.button)"
}
step "pan ownership"
"$CTL" input.mouse_button middle press --wait-settled >/dev/null || fail "middle press failed"
held='"Panning" "button" "middle"'
[ "$(pan_state)" = "$held" ] || fail "a middle press did not arm a middle-held pan: $(pan_state)"
for other in left right; do
    "$CTL" input.mouse_button "$other" release --wait-settled >/dev/null || fail "$other release failed"
    [ "$(pan_state)" = "$held" ] || fail "a $other release ended the middle-held pan: $(pan_state)"
done
"$CTL" input.mouse_button middle release --wait-settled >/dev/null || fail "middle release failed"
[ "$(field "$("$CTL" query.mode)" drag_state.kind)" = '"None"' ] || fail "the middle release did not end its pan"

# ---- Graceful quit -----------------------------------------------------

# A path-less capture first, so there is an artifacts dir for the graceful
# quit (`mandala_quit`) to see removed.
step "path-less capture"
"$CTL" capture.screenshot >/dev/null || fail "a path-less capture.screenshot failed"
[ -n "$(mandala_artifacts_dirs)" ] || fail "the path-less capture made no artifacts dir"
mandala_quit

# ---- One-shot launches -------------------------------------------------

one_shot() { # <png> [launch flags...]: a one-shot launch of the map, which must exit 0
    local png="$1" rc=0
    shift
    "$@" --screenshot "$png" "$MAP" >>"$WORK/oneshot.log" 2>&1 || rc=$?
    [ "$rc" -eq 0 ] || fail "the one-shot launch ${*} --screenshot $png exited $rc (log: $WORK/oneshot.log)"
}

same_capture() { # <png> <png>: the two PNGs and their sidecars are byte-identical
    cmp -s "$1" "$2" || fail "$1 and $2 differ"
    cmp -s "$1.geometry.json" "$2.geometry.json" || fail "the sidecars of $1 and $2 differ"
}

# The map as loaded, captured by a launch that exits on its own: the same
# picture the session's window capture took at load, in the same mode. A
# window on a real desktop is the exception — a window manager may size the
# second window differently — so there it is only checked as a picture.
step "one-shot --screenshot ($MODE)"
one_shot "$WORK/oneshot.png" "$APP" ${MODE_FLAGS[@]+"${MODE_FLAGS[@]}"}
read -r os_w os_h <<<"$(png_size "$WORK/oneshot.png")"
check_png "$WORK/oneshot.png" "$os_w" "$os_h"
if [ "$MODE" = "window" ]; then
    echo "   a window on this display: checked as a picture, not against the session's capture"
else
    same_capture "$WORK/window.png" "$WORK/oneshot.png"
    echo "   byte-identical to the session's window capture"
fi

# IPC-10's acceptance check: with no display and with a real window under
# Xvfb, the same machine draws the same bytes.
if [ "$COMPARE_XVFB" -eq 1 ]; then
    step "compare: headless one-shot against an Xvfb window's"
    one_shot "$WORK/compare-headless.png" "$APP" --headless
    if [ "$USE_XVFB" -eq 1 ]; then
        one_shot "$WORK/compare-xvfb.png" "$APP"
    else
        one_shot "$WORK/compare-xvfb.png" env -u WAYLAND_DISPLAY xvfb-run -a -s "-screen 0 1280x800x24" "$APP"
    fi
    same_capture "$WORK/compare-headless.png" "$WORK/compare-xvfb.png"
    echo "   byte-identical, PNG and sidecar"
fi

echo "PASS"
