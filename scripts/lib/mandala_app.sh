# Shared by the scripts that launch the app and drive it over --ipc
# (scripts/ipc_smoke.sh, scripts/record_demo.sh); sourced, never run.
# What it holds is the procedure `.claude/skills/mandala-drive/SKILL.md`
# teaches — the environment preflight, the build, a private work dir, the
# display and the launch, and the teardown — so the recipe has one
# definition, which a script that drifts from it cannot silently fork.
#
# The caller sets `set -euo pipefail`, then sources this and:
#   1. picks MODE: `window` (the display that is there), `xvfb` (its own
#      Xvfb), or `headless` (no window) — `mandala_default_mode` is the
#      default;
#   2. calls `mandala_check_packages`, then `mandala_build` (or
#      `mandala_locate` to use binaries as built);
#   3. calls `mandala_workdir` and installs `trap mandala_teardown EXIT`;
#   4. calls `mandala_start_xvfb` (xvfb only), then `mandala_launch <map>`.
# It reads and sets these globals: MODE, APP, CTL, WORK, SOCK, APP_PID,
# XVFB_PID, FAILED, KEEP (set it to 1 to keep the work dir on success too).

FAILED=0
APP_PID=""
XVFB_PID=""
KEEP="${KEEP:-0}"

step() { printf '== %s\n' "$*"; }
fail() {
    printf 'FAIL: %s\n' "$*" >&2
    FAILED=1
    exit 1
}

field() { # <mandalactl output> <field>: the value of `field: value`
    awk -F': ' -v k="$2" '$1 == k { print substr($0, length(k) + 3); exit }' <<<"$1"
}

# What the app runs in when the caller does not say: no window with no
# display — the recipe for a container or CI — and a window elsewhere than
# Linux (macOS does not announce its desktop through $DISPLAY, so a missing
# one says nothing there).
mandala_default_mode() {
    if [ "$(uname -s)" = "Linux" ] && [ -z "${DISPLAY:-}" ] && [ -z "${WAYLAND_DISPLAY:-}" ]; then
        echo headless
    else
        echo window
    fi
}

# <want_xvfb 0|1> <what>: with MODE, check the packages the script stands
# the graphics stack up with — on Linux, where it knows their names — and
# exit 2 with the install line when one is missing. A plain window uses the
# desktop's own, so only an `xvfb`/`headless` MODE, or a script that also
# wants Xvfb (`want_xvfb`), is checked. `what` names who needs them, for the
# message.
mandala_check_packages() {
    local want_xvfb="$1" what="$2"
    [ "$(uname -s)" = "Linux" ] || return 0
    [ "$MODE" != "window" ] || [ "$want_xvfb" -eq 1 ] || return 0
    local missing=() ldcache="" ldc
    if [ "$MODE" = "xvfb" ] || [ "$want_xvfb" -eq 1 ]; then
        want_xvfb=1
        command -v Xvfb >/dev/null 2>&1 || missing+=("xvfb")
    fi
    # ldconfig lives in /sbin, which a non-root PATH often lacks. Read the
    # cache once: `ldconfig -p | grep -q` under pipefail fails whenever grep
    # exits early and ldconfig takes the SIGPIPE.
    for ldc in ldconfig /sbin/ldconfig /usr/sbin/ldconfig; do
        if ldcache="$("$ldc" -p 2>/dev/null)" && [ -n "$ldcache" ]; then
            break
        fi
        ldcache=""
    done
    if [ -n "$ldcache" ]; then
        # winit's X11 backend dlopens libxkbcommon-x11 and panics without
        # it; with no window no winit backend is loaded at all.
        if [ "$want_xvfb" -eq 1 ]; then
            grep -q 'libxkbcommon-x11\.so' <<<"$ldcache" || missing+=("libxkbcommon-x11-0")
        fi
        # The adapter: Vulkan through Mesa's lavapipe ICD, the one backend
        # that serves both: wgpu's GL backend reports the Xvfb window
        # surface incompatible, though it works with no window.
        grep -q 'libvulkan\.so\.1' <<<"$ldcache" || missing+=("libvulkan1")
    else
        echo "note: ldconfig cannot be run here; the library check is skipped (a missing library shows in the app log)"
    fi
    # Where the Vulkan loader looks for ICD manifests, unless told otherwise.
    local icd_found=0 dir
    if [ -n "${VK_DRIVER_FILES:-}${VK_ICD_FILENAMES:-}" ]; then
        icd_found=1
    else
        local data_dirs
        IFS=: read -r -a data_dirs <<<"${XDG_DATA_DIRS:-/usr/local/share:/usr/share}"
        for dir in /etc "${data_dirs[@]}" /usr/local/share /usr/share; do
            if compgen -G "$dir/vulkan/icd.d/*.json" >/dev/null; then
                icd_found=1
                break
            fi
        done
    fi
    [ "$icd_found" -eq 1 ] || missing+=("mesa-vulkan-drivers")
    if [ ${#missing[@]} -gt 0 ]; then
        echo "error: $what packages that are missing:" >&2
        local sudo=""
        [ "$(id -u)" -eq 0 ] || sudo="sudo "
        echo "  ${sudo}apt-get update && ${sudo}apt-get install -y --no-install-recommends ${missing[*]}" >&2
        echo "(package names are Debian/Ubuntu's; see .claude/skills/mandala-drive/SKILL.md)" >&2
        exit 2
    fi
}

# <profile>: where the binaries are. CARGO_TARGET_DIR or build.target-dir
# may put them anywhere; ask cargo rather than assume target/, which could
# hold a stale build.
mandala_locate() {
    local profile="$1" target_dir
    target_dir="$(cargo metadata --format-version 1 --no-deps | sed -n 's/.*"target_directory":"\([^"]*\)".*/\1/p')"
    [ -n "$target_dir" ] || fail "cargo metadata did not name a target directory"
    APP="$target_dir/$profile/mandala"
    CTL="$target_dir/$profile/mandalactl"
    if [ ! -x "$APP" ] || [ ! -x "$CTL" ]; then
        fail "the build did not leave $APP and $CTL"
    fi
}

# <profile>: build both binaries (cargo skips what is up to date), locate them.
mandala_build() {
    local profile="$1"
    step "build ($profile)"
    if [ "$profile" = "release" ]; then
        cargo build --release -p mandala -p mandalactl
    else
        cargo build -p mandala -p mandalactl
    fi
    mandala_locate "$profile"
}

# <prefix>: a short, private work dir and the socket in it. Short and
# private because the socket path must stay under the 107-byte limit and
# every directory above it must be writable only by its owner.
mandala_workdir() {
    WORK="$(mktemp -d "${TMPDIR:-/tmp}/$1.XXXXXX")"
    chmod 700 "$WORK"
    SOCK="$WORK/m.sock"
}

# The EXIT trap: stop what is still running, remove what the app leaves
# behind, and keep the work dir (naming it, with the app log's tail) when a
# check failed — or on success too with KEEP=1.
mandala_teardown() {
    local code=$?
    if [ -n "$APP_PID" ] && kill -0 "$APP_PID" 2>/dev/null; then
        # Still running: a check failed before the graceful quit.
        # SIGTERM is not a graceful exit — the socket and the artifacts
        # dir are left behind — so both are removed below.
        kill -TERM "$APP_PID" 2>/dev/null || true
        local _
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

# Start our own Xvfb and export DISPLAY for the launch.
mandala_start_xvfb() {
    step "start Xvfb"
    # -displayfd picks a free display number and writes it to fd 3 once
    # the server is accepting connections, so there is no sleep-and-hope.
    Xvfb -displayfd 3 -screen 0 1280x800x24 -nolisten tcp 3>"$WORK/display" 2>"$WORK/xvfb.log" &
    XVFB_PID=$!
    local _
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
}

# <map> [app args...]: launch the app on the map with --ipc, wait for it to
# answer. Sets MODE_FLAGS, the flag that makes a launch of this MODE's — no
# window for `headless`, none needed otherwise — for a caller's own launches
# of the same app (a one-shot `--screenshot`).
mandala_launch() {
    local map="$1"
    shift
    MODE_FLAGS=()
    [ "$MODE" = "headless" ] && MODE_FLAGS=(--headless)
    step "launch ($MODE) $APP ${MODE_FLAGS[*]-} $* $map --ipc $SOCK"
    "$APP" ${MODE_FLAGS[@]+"${MODE_FLAGS[@]}"} "$@" "$map" --ipc "$SOCK" >"$WORK/app.log" 2>&1 &
    APP_PID=$!
    export MANDALA_IPC="$SOCK"

    step "wait for ping"
    local alive=0 deadline=$((SECONDS + 30))
    while [ "$SECONDS" -lt "$deadline" ]; do
        kill -0 "$APP_PID" 2>/dev/null || fail "the app exited during startup"
        if "$CTL" --timeout 2 ping >/dev/null 2>&1; then
            alive=1
            break
        fi
        sleep 0.5
    done
    [ "$alive" -eq 1 ] || fail "no ping answer within 30 s"
}

# The directories the app made for path-less captures, which a graceful
# quit removes.
mandala_artifacts_dirs() {
    compgen -G "${XDG_RUNTIME_DIR:-${TMPDIR:-/tmp}}/mandala-$APP_PID-artifacts*" || true
}

# The graceful exit: `act.action Quit` is the one a closed window takes and
# the only one a headless app has. The reply arrives, the process exits 0,
# and the socket and the artifacts dir go with it — each checked here.
mandala_quit() {
    step "act.action Quit"
    local artifacts dir quit_rc=0 _
    artifacts="$(mandala_artifacts_dirs)"
    "$CTL" act.action Quit >/dev/null || fail "act.action Quit was not answered"
    for _ in $(seq 1 50); do
        kill -0 "$APP_PID" 2>/dev/null || break
        sleep 0.1
    done
    kill -0 "$APP_PID" 2>/dev/null && fail "the app was still running 5 s after Quit"
    wait "$APP_PID" || quit_rc=$?
    APP_PID=""
    [ "$quit_rc" -eq 0 ] || fail "the app exited $quit_rc after Quit, not 0"
    [ ! -e "$SOCK" ] || fail "the socket $SOCK is still there after Quit"
    for dir in $artifacts; do
        [ ! -e "$dir" ] || fail "the artifacts dir $dir is still there after Quit"
    done
    echo "   exited 0; socket and artifacts dir removed"
}
