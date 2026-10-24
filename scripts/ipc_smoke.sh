#!/usr/bin/env bash
# End-to-end smoke test of the --ipc drive-and-see loop: launch the app
# (under its own Xvfb when there is no display), ping it, check the map
# loaded, take a window-sized screenshot and a region screenshot around
# one node, check the PNGs and their geometry sidecars, and tear it all
# down. The procedure is the one `.claude/skills/mandala-drive/SKILL.md`
# teaches; this script is that skill's proof that it still works.
#
# Deliberately outside ./test.sh: it needs a live GPU adapter, and
# TEST_CONVENTIONS §T8/§T10 keep live wgpu out of the suite.
#
# Usage:
#   ./scripts/ipc_smoke.sh                   # testament, release build
#   ./scripts/ipc_smoke.sh --map <path>      # another map
#   ./scripts/ipc_smoke.sh --debug           # dev-profile build (logs info!/debug!)
#   ./scripts/ipc_smoke.sh --xvfb            # own Xvfb even when $DISPLAY is set
#   ./scripts/ipc_smoke.sh --keep            # keep the work dir on success too
#
# Exit codes: 0 pass; 1 a check failed (the work dir with the app's log
# is kept and named); 2 the environment cannot run the app (what to
# install is printed); 3 bad command line.

set -euo pipefail

MAP="maps/testament.mindmap.json"
PROFILE="release"
FORCE_XVFB=0
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
        --keep) KEEP=1; shift ;;
        -h | --help) usage; exit 0 ;;
        *) echo "error: unknown argument: $1" >&2; usage; exit 3 ;;
    esac
done

cd "$(dirname "$0")/.."

[ -f "$MAP" ] || { echo "error: map file not found: $MAP" >&2; exit 3; }

step() { printf '== %s\n' "$*"; }
fail() {
    printf 'FAIL: %s\n' "$*" >&2
    FAILED=1
    exit 1
}

# ---- Environment preflight --------------------------------------------

# Xvfb is the Linux headless recipe. Elsewhere (macOS) the desktop is not
# announced through $DISPLAY, so a missing one says nothing there.
USE_XVFB=0
if [ "$(uname -s)" = "Linux" ]; then
    if [ "$FORCE_XVFB" -eq 1 ] || { [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; }; then
        USE_XVFB=1
    fi
elif [ "$FORCE_XVFB" -eq 1 ]; then
    echo "error: --xvfb is the Linux headless recipe; this is $(uname -s)" >&2
    exit 3
fi

if [ "$USE_XVFB" -eq 1 ]; then
    missing=()
    # Read once: `ldconfig -p | grep -q` under pipefail fails whenever grep
    # exits early and ldconfig takes the SIGPIPE.
    ldcache="$(ldconfig -p 2>/dev/null || true)"
    command -v Xvfb >/dev/null 2>&1 || missing+=("xvfb")
    # winit's X11 backend dlopens libxkbcommon-x11 and panics without it.
    grep -q 'libxkbcommon-x11\.so' <<<"$ldcache" || missing+=("libxkbcommon-x11-0")
    # The adapter: Vulkan through Mesa's lavapipe (llvmpipe) ICD. wgpu's GL
    # backend reports the Xvfb window surface incompatible, so GL alone is
    # not enough.
    grep -q 'libvulkan\.so\.1' <<<"$ldcache" || missing+=("libvulkan1")
    if ! compgen -G '/usr/share/vulkan/icd.d/*.json' >/dev/null && ! compgen -G '/etc/vulkan/icd.d/*.json' >/dev/null; then
        missing+=("mesa-vulkan-drivers")
    fi
    if [ ${#missing[@]} -gt 0 ]; then
        echo "error: no display, and the headless recipe needs packages that are missing:" >&2
        sudo=""
        [ "$(id -u)" -eq 0 ] || sudo="sudo "
        echo "  ${sudo}apt-get update && ${sudo}apt-get install -y --no-install-recommends ${missing[*]}" >&2
        echo "(package names are Debian/Ubuntu's; see .claude/skills/mandala-drive/SKILL.md)" >&2
        exit 2
    fi
fi

# ---- Build -------------------------------------------------------------

step "build ($PROFILE)"
if [ "$PROFILE" = "release" ]; then
    cargo build --release -p mandala -p mandalactl
else
    cargo build -p mandala -p mandalactl
fi
APP="target/$PROFILE/mandala"
CTL="target/$PROFILE/mandalactl"

# ---- Work dir and teardown ---------------------------------------------

# Short and private: the socket path must stay under the 107-byte limit
# and every directory above it must be writable only by its owner.
WORK="$(mktemp -d "${TMPDIR:-/tmp}/mandala-smoke.XXXXXX")"
chmod 700 "$WORK"
SOCK="$WORK/m.sock"
FAILED=0
APP_PID=""
XVFB_PID=""

teardown() {
    local code=$?
    if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
        # No IPC command quits the app, and SIGTERM is not a graceful exit:
        # the socket and the artifacts dir are left behind, so both are
        # removed below.
        kill -TERM "$APP_PID" 2>/dev/null || true
        for _ in 1 2 3 4 5 6 7 8 9 10; do
            kill -0 "$APP_PID" 2>/dev/null || break
            sleep 0.2
        done
        kill -KILL "$APP_PID" 2>/dev/null || true
        wait "$APP_PID" 2>/dev/null || true
    fi
    if [ -n "$XVFB_PID" ]; then
        kill -TERM "$XVFB_PID" 2>/dev/null || true
        wait "$XVFB_PID" 2>/dev/null || true
    fi
    if [ -n "$APP_PID" ]; then
        rm -rf "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/mandala-$APP_PID-artifacts"*
    fi
    rm -f "$SOCK"
    if [ "$code" -ne 0 ] || [ "$FAILED" -ne 0 ]; then
        echo "work dir kept for inspection: $WORK (app log: $WORK/app.log)" >&2
        if [ -f "$WORK/app.log" ]; then
            echo "---- last lines of the app log ----" >&2
            tail -n 15 "$WORK/app.log" >&2
        fi
    elif [ "$KEEP" -eq 1 ]; then
        echo "work dir kept: $WORK"
    else
        rm -rf "$WORK"
    fi
}
trap teardown EXIT

# ---- Display -----------------------------------------------------------

if [ "$USE_XVFB" -eq 1 ]; then
    step "start Xvfb"
    # -displayfd picks a free display number and writes it to fd 3 once
    # the server is accepting connections, so there is no sleep-and-hope.
    Xvfb -displayfd 3 -screen 0 1280x800x24 -nolisten tcp 3>"$WORK/display" 2>"$WORK/xvfb.log" &
    XVFB_PID=$!
    for _ in $(seq 1 50); do
        [ -s "$WORK/display" ] && break
        kill -0 "$XVFB_PID" 2>/dev/null || fail "Xvfb exited: $(cat "$WORK/xvfb.log")"
        sleep 0.1
    done
    [ -s "$WORK/display" ] || fail "Xvfb did not report a display within 5 s"
    DISPLAY=":$(tr -d '[:space:]' <"$WORK/display")"
    export DISPLAY
    # A Wayland session in the environment would win over our X server.
    unset WAYLAND_DISPLAY
    echo "DISPLAY=$DISPLAY"
fi

# ---- Launch and liveness -----------------------------------------------

step "launch $APP $MAP --ipc $SOCK"
"$APP" "$MAP" --ipc "$SOCK" >"$WORK/app.log" 2>&1 &
APP_PID=$!
export MANDALA_IPC="$SOCK"

step "wait for ping"
alive=0
for _ in $(seq 1 60); do
    kill -0 "$APP_PID" 2>/dev/null || fail "the app exited during startup"
    if "$CTL" --timeout 2 ping >/dev/null 2>&1; then
        alive=1
        break
    fi
    sleep 0.5
done
[ "$alive" -eq 1 ] || fail "no ping answer within 30 s"
"$CTL" ping

# ---- The map loaded ----------------------------------------------------

step "query.document"
doc="$("$CTL" query.document)" || fail "query.document failed"
echo "$doc"
nodes="$(printf '%s\n' "$doc" | awk -F': ' '$1 == "counts.nodes" { print $2 }')"
if [ -z "$nodes" ] || [ "$nodes" -le 0 ]; then fail "the document reports no nodes"; fi
printf '%s\n' "$doc" | grep -qx "file_path: \"$MAP\"" || fail "file_path is not $MAP"

# ---- Helpers for the captures ------------------------------------------

field() { # <mandalactl output> <field>: the value of `field: value`
    printf '%s\n' "$1" | awk -F': ' -v k="$2" '$1 == k { print $2; exit }'
}

png_size() { # <png>: "W H" from the IHDR chunk
    od -An -tu1 -j16 -N8 "$1" | awk '{ print $1*16777216 + $2*65536 + $3*256 + $4, $5*16777216 + $6*65536 + $7*256 + $8 }'
}

check_png() { # <png> <expected width> <expected height>
    local png="$1" w="$2" h="$3"
    [ -s "$png" ] || fail "$png is missing or empty"
    [ "$(od -An -tx1 -N8 "$png" | tr -d ' \n')" = "89504e470d0a1a0a" ] || fail "$png is not a PNG"
    [ "$(png_size "$png")" = "$w $h" ] || fail "$png is $(png_size "$png"), the reply said $w $h"
    if command -v python3 >/dev/null 2>&1; then
        python3 -I - "$png" <<'PY' || fail "$png is a single flat color: nothing visible was drawn"
import struct, sys, zlib
data = open(sys.argv[1], "rb").read()
pos, idat, w, h = 8, b"", 0, 0
while pos < len(data):
    n, kind = struct.unpack(">I4s", data[pos:pos + 8])
    body = data[pos + 8:pos + 8 + n]
    if kind == b"IHDR":
        w, h, depth, ctype = struct.unpack(">IIBB", body[:10])
        assert depth == 8 and ctype in (2, 6), f"unexpected PNG format {depth}/{ctype}"
        bpp = 3 if ctype == 2 else 4
    elif kind == b"IDAT":
        idat += body
    pos += 12 + n
raw, stride = zlib.decompress(idat), w * bpp
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
    prev = line
print(f"   {len(colors)} distinct colors sampled")
sys.exit(0 if len(colors) > 1 else 1)
PY
    else
        echo "   (python3 not found: pixel-content check skipped; the sidecar check below still runs)"
    fi
    local sidecar="$png.geometry.json"
    [ -s "$sidecar" ] || fail "$sidecar is missing or empty"
    grep -q "\"surface\":{\"width\":$w,\"height\":$h," "$sidecar" || fail "$sidecar does not describe a ${w}x$h image"
    grep -q '"nodes":\[{"id":' "$sidecar" || fail "$sidecar lists no drawn node"
}

# ---- Window-sized screenshot -------------------------------------------

step "clock.wait settled"
"$CTL" clock.wait settled || fail "the app did not settle"

step "capture.screenshot (window size)"
shot="$("$CTL" capture.screenshot "$WORK/window.png")" || fail "capture.screenshot failed"
echo "$shot"
[ "$(field "$shot" rendered)" = "true" ] || fail "rendered is not true: skipped $(field "$shot" skipped)"
check_png "$WORK/window.png" "$(field "$shot" width)" "$(field "$shot" height)"

# ---- Region screenshot around one node ---------------------------------

node="$(grep -o '"nodes":\[{"id":"[^"]*"' "$WORK/window.png.geometry.json" | sed 's/.*"id":"//; s/"$//')"
step "scene.node_geometry $node"
geo="$("$CTL" scene.node_geometry "$node")" || fail "scene.node_geometry $node failed"
x="$(field "$geo" canvas.x)"; y="$(field "$geo" canvas.y)"
w="$(field "$geo" canvas.w)"; h="$(field "$geo" canvas.h)"
if [ -z "$x" ] || [ -z "$y" ] || [ -z "$w" ] || [ -z "$h" ]; then
    fail "scene.node_geometry gave no canvas rect for $node"
fi
# The node's canvas rect grown by its own size on every side.
region="$(awk -v x="$x" -v y="$y" -v w="$w" -v h="$h" \
    'BEGIN { printf "{\"x\":%s,\"y\":%s,\"w\":%s,\"h\":%s}", x - w, y - h, 3 * w, 3 * h }')"

camera_before="$("$CTL" query.camera)"
step "capture.screenshot region=$region"
shot="$("$CTL" capture.screenshot "$WORK/region.png" region:="$region")" || fail "region capture failed"
echo "$shot"
[ "$(field "$shot" rendered)" = "true" ] || fail "rendered is not true: skipped $(field "$shot" skipped)"
check_png "$WORK/region.png" "$(field "$shot" width)" "$(field "$shot" height)"
grep -q "{\"id\":\"$node\",\"rect\":{[^}]*},\"clipped\":false" "$WORK/region.png.geometry.json" \
    || fail "node $node is not wholly inside the region capture framed around it"
[ "$("$CTL" query.camera)" = "$camera_before" ] || fail "the capture moved the live camera"

echo "PASS"
