---
name: mandala-record
description: >-
  Record a short, reviewable demo GIF of Mandala doing something — an
  animation, a zoom, a click or double-click, a drag, a mode change — so a
  human can watch it instead of taking your word for it. Use when asked to
  "demonstrate", "record", "show me" or "make a video/GIF of" a behavior, when
  a change is about how something moves or responds rather than how a frame
  looks, or when a PR should carry visual evidence. Covers the demo recipe
  (state before, shortest proving sequence, before/after beats, stepped vs
  realtime pacing), the one-command wrapper scripts/record_demo.sh, the
  caption and reproduction script every recording ships with, watching your
  own recording before you post it, where recordings go (never into the
  repo), size limits, and two worked examples. Builds on mandala-drive.
---
# mandala-record

A screenshot proves a state; a recording proves a *behavior* — an
animation's timing, what a double-click does, how a zoom ramps. This skill
is how to make one a human can review in thirty seconds: **set up → record
the shortest proof → watch it yourself → post it with a caption and the
script that made it.**

The mechanism is `capture.record_start` / `capture.record_stop` plus
`mandalactl record assemble` (`format/ipc.md` §"capture.record_start").
`scripts/record_demo.sh` is the wrapper that runs the whole loop in one
command; the launch recipes, `scene.*` queries, framing and teardown are
[`mandala-drive`](../mandala-drive/SKILL.md)'s — read it first if you have not
driven the app before. Every command and number below was run, in this
repo's cloud container.

## When to record, and when not to

- **Record** when the claim is about *change over time or in response to
  input*: "ZoomIn zooms toward the pointer", "double-click opens the editor",
  "the animated mutation eases in", "dragging a node moves its edges with it".
- **Screenshot instead** (mandala-drive) when the claim is about how one frame
  looks: layout, color, glyphs, overlap. A recording of a still scene is a
  screenshot with extra bytes.
- **Neither — use text** (`query.*`, `scene.*`) when the claim is about state:
  "the selection is node 3.2" is one line of `query.selection`, not a GIF.
  Put that line in the caption as well; the GIF shows it, the line proves it.

## One command

```sh
scripts/record_demo.sh maps/testament.mindmap.json \
    .claude/skills/mandala-record/examples/zoom-in.demo \
    /tmp/73-zoom-in-20261025.gif
```

It builds (release; `--no-build` skips), launches the app (`--headless` when
there is no display, a window when there is; `--xvfb` for a real window under
Xvfb), runs the script, assembles the GIF, checks it, quits the app, and
prints a caption to fill in. Exit `0` made, `1` a step failed (the app's log
is named), `2` the environment is missing packages (the install line is
printed — the same as mandala-drive's matrix), `3` bad command line.
`scripts/record_demo.sh --help` lists every option.

Beside the GIF it leaves `<out stem>.frames/` — the PNG frames and
`manifest.json` — for you to **look at** (below) and then delete.

### The script file

One `mandalactl` invocation per line (the arguments, as you would type
them minus `mandalactl`), blank lines and `#` comments ignored — a `#` that
starts a word ends the line, so a trailing `# comment` is fine — plus two
directives:

| line | meaning |
|---|---|
| `@record [params]` | `capture.record_start` into the frames dir at `--fps` (default 20), then hold the "before" beat. `params` are `record_start`'s: `fps=`, `max_frames=`, `width=`/`height=`, `region:='{…}'`, `space=`, `fit=`, `scale=` |
| `@stop` | hold the "after" beat, then `capture.record_stop` |

Exactly one pair. **Everything before `@record` is setup and is not in the
picture**; everything after `@stop` runs unrecorded, which is where the proof
lines go (`query.camera`, `query.editors`). A failing command fails the run
and names its script line. Words split the way a shell's quoting splits
them (`'single'`, `"double"`, backslash) **and nothing else**: `$var`,
`$(…)`, backticks, `;`, `|` and `*` are ordinary characters, so no shell
code runs from a script. **That is not "safe to run unread":** `mandalactl`
itself can write files with your privileges — `act.console 'save <path>'`
overwrites any writable file with map JSON, `capture.* path=… overwrite=true`
with a PNG — so read a script (they are short) before re-running one
someone else posted. `raw -` has no stdin to read in a script; pass the
frame inline instead, `raw '<json>'`. Line endings
may be CRLF; the wrapper checks the whole file parses, and that it holds one
`@record` and one `@stop` after it, before it builds or launches anything.

The beats are the wrapper's: `--beat-ms` (default 500) of settled, unchanged
picture right after `@record` and right before `@stop`. They exist so a
reviewer's eye has a "before" to compare against and the end state to rest
on — without them a half-second recording is over before anyone has looked.

## The demo recipe

**1. Set up the state before recording — all of it.** The recording starts
on a map already open, framed and settled. A reviewer should see the
behavior, not you finding it. In the setup section:

- Pick the map (`maps/testament.mindmap.json` unless the behavior needs
  another) and the size (`--size`, default 800×600; bigger is only bigger
  bytes).
- **Frame the subject.** Nothing is framed for you: testament's initial view
  is the whole map at zoom 0.05, a few pixels per node, and the geometric
  center of a map is often empty canvas. The recipe that works for any node
  — it puts it dead center at a legible zoom:

  ```text
  act.action ZoomFit --wait-settled                  # a known camera
  input.click_node <id> --wait-settled               # select it (needs it ON SCREEN — see below)
  act.action CenterOnSelection --wait-settled        # camera centers on it
  act.action ZoomReset --wait-settled                # zoom 1.0 about the screen center — where it now is
  act.action DeselectAll --wait-settled              # no selection highlight in the "before" beat
  ```

  **`ZoomFit` does not always put the node on screen.** Testament does not
  fit 800×600: the fit stops at the minimum zoom 0.05 and nodes near the
  bottom land below the surface (`input.click_node` then fails `not_found`).
  Check with `scene.node_geometry <id>` (its `surface.*` rect against 800×600)
  and pan: `act.action PanCameraNorth --wait-settled` moves the view by 50
  screen pixels per step (the compass names are the camera's, not the
  picture's — confirm the direction with `query.camera` and repeat the
  line until the node is in). To click one *section* of a multi-section
  node, `input.click_node <id> section=<n>`; which nodes have several is
  `mandalactl --json scene.dump include=all | jq -r '.result.nodes[] |
  select(.sections|length>1) | .node_id'` (testament has one: `3.7`).

  Or hand `@record` a `region:='{"x":…,"y":…,"w":…,"h":…}'` with
  `width=`/`height=` of the same aspect (canvas units, from
  `scene.node_geometry`); then the frames are that crop whatever the camera
  does, which suits a behavior that moves the camera itself — **and legibility
  too**: the default assembled width is 640 px, so a full 800×600 frame is
  shrunk to 0.8× and small text goes soft. A crop around the subject at
  `width=640 height=<same aspect>` draws it larger than the live view
  (worked example 2: 1.25×) and drops the neighbors that crowd it.
- **Check the setup took** with a text query inside the setup (`query.camera`,
  `query.selection`, `query.mode`); its output is in the run's log. Do not
  find out from the GIF that the zoom is still 0.05.
- `clock.set_mode virtual` first, so the script means the same in every
  environment: a `--headless` app starts virtual, a window starts real
  (setting the mode it already has is a no-op).

**2. Record the shortest sequence that proves the behavior.** Target
2–10 seconds including both beats (both worked examples are a little
under or over 2 s — fine for a single instant change; if the whole clip
is shorter than the beats plus a second, it is too short to follow); most
are 2–3. One recording shows
**one behavior**. Two behaviors are two recordings — a reviewer who has to
work out which part you mean has not been shown anything.

**3. Choose the pacing, and say which in the caption.**

| | `stepped` | `realtime` |
|---|---|---|
| how | `clock.set_mode virtual`, then `clock.step ms=<n>` between actions: time moves only when you move it | `clock.set_mode real` in the setup; the app runs at wall-clock speed |
| frames | exactly one per `1000/fps` boundary of virtual time; no jitter, no gaps | whatever the machine managed; gaps are in the manifest and the GIF holds the frame before them |
| honest about | *what happens, in what order, at what intended speed* — smooth, deterministic, the same bytes headless and in a window | *what a user would see on this machine*, stalls included — and on llvmpipe that is slower than a GPU |
| use for | state changes, input responses, animations whose timing you control | a claim about real-time feel: "this eases in without a hitch" |

`stepped` is the default for a reason: a GIF of a software-rendered app in
real time is mostly a statement about llvmpipe. State the choice in the
caption — "stepped: eight ZoomIn steps 100 ms apart on the virtual clock; the
real-time feel is not what this shows" — so nobody mistakes smooth for fast.

**Under a virtual clock frames exist only while it steps.** An animation
(`timing.duration_ms > 0`) does not move until you `clock.step ms=<n>` past
it, and the beats step for you. A recording with no steps between `@record`
and `@stop` is two beats and a jump — correct for an *instant* change, wrong
for an animation. Check what the thing you demonstrate actually does before
you choose: **a single `ZoomIn` is instant** (one 1.1× jump, no animation —
`apply_zoom_step`), so the worked example below shows it as a ramp of steps.
An animated `CustomMutation` is the case for `clock.step` across its
duration (`query.animations` says when it is done).

**Two traps found while writing this.** `input.wheel 26` is one notch, not
twenty-six — it moved zoom 0.05 → 0.055; to repeat an action, repeat the
line. And `ZoomIn` at a point keeps *that point* fixed on screen, so zooming
at an arbitrary spot of a sparse map zooms into black; frame first.

## Structure for reviewability

Every recording ships three things together:

1. **The GIF** — one behavior, a settled "before" beat (~0.5 s), the
   behavior, a settled "after" beat.
2. **A caption**, one paragraph: *what to look at* (and when in the clip),
   *what would look different if it were broken*, and the pacing with the
   reason. The wrapper prints the template; fill every `<…>`. The "if it
   were broken" sentence is the one that makes it a test and not a trailer —
   "if ZoomIn were a no-op the node would stay 270 px wide; if it zoomed
   about the wrong point, the node would drift off center as it grew".
3. **The exact script** that produced it, in a code block, plus the
   `record_demo.sh` line — so a reviewer or a later agent re-runs it and
   gets the same bytes (stepped recordings are deterministic: the zoom
   example's headless and Xvfb GIFs are byte-identical).

Name the file `<issue>-<behavior>-<yyyymmdd>.gif`, e.g.
`73-double-click-section-editor-20261025.gif`; with no issue, drop the
number.

**Say what the picture cannot.** A subtle on-screen change needs its text
proof beside it: the selection tint and the editor tint in the double-click
example are the same cyan, and the only on-screen difference between "a
section is selected" and "a section is being edited" is one caret glyph. Put
the caret, and the `query.*` lines that prove the state, in the caption.
A read-only `query.*` line between `@record` and `@stop` is safe (it takes no
frame and moves no clock) and logs what an intermediate step did.

## Review your own recording first

**A demo the demonstrator has not watched is not evidence.** Before you post:

```sh
# The text proof: the script's trailing query.* lines printed it. Read them.
# The frames: first, a middle one, last. Your file-reading tool shows PNGs.
ls /tmp/73-zoom-in-20261025.frames | sed -n '1p;$p'      # frame_00000.png … frame_00036.png
# Where the picture changes: frames in runs of identical bytes. A stepped
# recording of an instant change is a few long runs, and the frames to look
# at are the first of each run.
md5sum /tmp/73-zoom-in-20261025.frames/frame_*.png | awk '{print $1}' | uniq -c
# The GIF itself, as a strip, to check what a reviewer actually gets (needs
# ffmpeg; frames at the beats and mid-action). Keep the tiles wide enough
# to read the subject's text — 400 px tiles hide exactly what you are checking:
ffmpeg -v error -y -i /tmp/73-zoom-in-20261025.gif \
  -vf "select='eq(n,0)+eq(n,12)+eq(n,24)+eq(n,36)',scale=640:-1,tile=2x2" -frames:v 1 /tmp/strip.png
```

Check, in order: the **first frame is the "before"** you meant (right node,
right zoom, nothing selected); the **last frame is the "after"**; the
**change happens between them** (for a ramp, a middle frame between the
first and last states; for an instant change, the frame where a run of
identical frames ends — not a middle frame that already shows the final
state); the **subject is legible** (look at a frame at full size, not the
strip: can you read the text the caption points at?); and
`manifest.json` has no surprising `gaps` (`dropped_frames` > 0 is printed as a
`NOTE`). If any of that is wrong, fix the script and re-run
(`--overwrite`) — do not post and apologize. When it is right, say what you
saw in your own words in the post, not just that you recorded it.

## Where the recording goes

Recordings are **ephemeral review artifacts, not repo content.** Do not
commit GIFs: they are binary, they never leave history, and §5's cleanliness
applies. `record_demo.sh` refuses an `<out>` inside this checkout
(`--allow-in-repo` is for the rare case you were told otherwise); write to
`/tmp`, and delete `<out stem>.frames/` when you are done.

- **Working conversationally** (a cloud or local session with a person
  watching): hand the GIF to the session's file-sending mechanism — in Claude
  Code, `SendUserFile` with `display: "render"` — with the caption as its
  `caption` or the message around it. The script goes in the message.
- **For a PR:** put the caption and the script in a PR comment (the GitHub
  MCP's `add_issue_comment`, or the PR description), because they are text
  and must be there. The GIF itself is the awkward part: GitHub has **no
  documented API to attach an image to a comment** — the drag-and-drop
  uploader is the web UI's. So either (a) hand the GIF to the person
  (above) and say in the comment "recording attached in the session; drop it
  here to inline it", or (b), only if the maintainer agreed to a binary in
  git at all, push it alone to a throwaway branch that never merges
  (`review-artifacts/<pr-number>`, deleted when the PR merges) and embed it
  as `![caption](https://github.com/<owner>/<repo>/blob/review-artifacts/<pr-number>/<file>.gif?raw=true)`.
  (b) commits a binary and strays from "do not commit GIFs"; it was not
  tried here, so check that it renders before relying on it. Say in the
  comment which you did.
- Either way the comment carries: the caption, the script block, and the
  `record_demo.sh` line.

## Size discipline

GitHub shows a GIF inline up to **10 MiB**; `record_demo.sh` and
`record assemble` both warn past it. What the wrapper does by default keeps
a typical demo far below that: frames scaled to **640 px wide** (never
scaled up), recorded at **20 fps**, GIF delays taken from the manifest so
speed is honest. Measured here: 37 frames / 1.85 s of a 640×480 zoom = 0.45
MB (0.24 MB/s); 44 frames / 2.2 s of the cropped double-click demo = 0.80
MB (0.37 MB/s); a busy whole-map view at 640 px cost 0.99 MB for 1.65 s
(0.6 MB/s). Plan for ≲ 0.6 MB per second of clip, and read the size the run
prints.

If it is too big, in this order: **shorten** (fewer steps, shorter beats —
`--beat-ms 300`); **crop** with `region:=` to the subject; **narrow**
(`--width 480`); **thin** the rate (`-- fps=12`, which keeps the same speed
with fewer frames, or `--fps 12` at record time). Do not raise `--width` or
`--fps` unless something in the picture is illegible: 20 fps and 640 px are
what the review needs, not what the app can do. A GIF frame shorter than
20 ms is shown as 100 ms by browsers, so `assemble` drops frames that close
together and says how many; that is why 60 fps is not "smoother".

**When a demo genuinely needs MP4** (a long recording, a gradient-heavy
scene a GIF's 256 colors band): `scripts/record_demo.sh --mp4 … out.mp4`
runs `mandalactl record assemble format=mp4` through the host's `ffmpeg`
(never required for the GIF; this repo's cloud container has it). GitHub will not
play a linked `.mp4` inline — a video renders in a comment only when
uploaded through the web UI (up to 10 MB on a free plan, 100 MB paid) — so
an MP4 is delivered the same two ways as above, and the web upload is the
person's. Prefer cutting the GIF down first.

## Environments

The launch half is mandala-drive's matrix, and the wrapper takes the same
three: `--headless` (no display, Mesa lavapipe; the default with no
`DISPLAY`), `--xvfb` (a real window under its own Xvfb), `--window` (the
display that is there). The packages are the same too —
`libvulkan1 mesa-vulkan-drivers`, plus `xvfb libxkbcommon-x11-0` for Xvfb —
and the wrapper prints the install line when one is missing.

**Both software environments draw the same bytes**: the zoom example's
`--headless` and `--xvfb` runs gave byte-identical GIFs, frame for frame. So
reach for Xvfb only when the behavior is the *window's* (a resize, a real
`RedrawRequested`); otherwise headless is the fast one. A window on a real
desktop runs the same script (the clock line makes it virtual) but was not
run here, and a window manager may size the window differently from
`--size`.

## Worked example 1: `ZoomIn`

The claim: *`ZoomIn` zooms toward the point it is given, one 1.1× step at a
time.* The first thing to learn about it is that a single `ZoomIn` is
instant, so "the animated ZoomIn" is really a **ramp** of eight steps. The
whole script, `examples/zoom-in.demo`:

```text
# Demo: ZoomIn zooms toward the point under it, one 1.1x step at a time.
# Map: maps/testament.mindmap.json, 800x600 (the default size).
#
# --- setup (never in the picture) ---------------------------------------
# Frame the node "Arpachshad" (1.2.0.0.0.0.0.0.0.0.0.2) dead center at
# zoom 1.0: ZoomFit puts it on screen, a click selects it, CenterOnSelection
# centers the camera on it, ZoomReset zooms to 1.0 about the screen center
# (where it now is), and DeselectAll takes the selection highlight out of
# the picture.
# The clock is virtual so the script means the same headless (which starts
# virtual) and in a window (which starts real); setting it again is a no-op.
clock.set_mode virtual
act.action ZoomFit --wait-settled
input.click_node 1.2.0.0.0.0.0.0.0.0.0.2 --wait-settled
act.action CenterOnSelection --wait-settled
act.action ZoomReset --wait-settled
act.action DeselectAll --wait-settled
clock.wait settled
query.camera
#
# --- the recording ------------------------------------------------------
# A single ZoomIn is instant (one 1.1x jump, no animation), so the
# behavior is shown as a ramp: eight steps at the screen center, 100 ms
# of virtual time apart (1.1^8 = 2.14x). Under the virtual clock a
# headless app starts with, frames exist only while it steps.
@record
act.action ZoomIn x=400 y=300 space=surface
clock.step ms=100
act.action ZoomIn x=400 y=300 space=surface
clock.step ms=100
act.action ZoomIn x=400 y=300 space=surface
clock.step ms=100
act.action ZoomIn x=400 y=300 space=surface
clock.step ms=100
act.action ZoomIn x=400 y=300 space=surface
clock.step ms=100
act.action ZoomIn x=400 y=300 space=surface
clock.step ms=100
act.action ZoomIn x=400 y=300 space=surface
clock.step ms=100
act.action ZoomIn x=400 y=300 space=surface
clock.step ms=100
@stop
# --- proof the state changed (not in the picture) -----------------------
query.camera
```

```sh
scripts/record_demo.sh maps/testament.mindmap.json \
    .claude/skills/mandala-record/examples/zoom-in.demo /tmp/73-zoom-in-20261025.gif
# RECORDED: /tmp/73-zoom-in-20261025.gif (449076 bytes, 37 frames, 1850 ms, stepped pacing)
# the run's query.camera lines: zoom: 1.0 before, zoom: 2.143589 after (1.1^8)
```

Watched, as a strip of frames 0, 12, 24, 36: the first two nearly the same
(the beat, then the first step), "Arpachshad / Uncertain etymology" growing
about the center through the third, and the last the largest with its
neighbor Shelah pushed off the right edge. The caption posted with it:

> **`ZoomIn` zooms about the given point, 1.1× per step** —
> `73-zoom-in-20261025.gif`
>
> Look at: the "Arpachshad" node in the middle. After the half-second hold
> it grows steadily, staying centered, for eight steps; by the end it fills
> about twice the width (zoom 1.0 → 2.14, read from `query.camera`).
> If it were broken: a no-op would leave it 270 px wide for the whole clip;
> zooming about the wrong point would make it slide off center as it grew.
> Pacing: stepped — eight `ZoomIn`s 100 ms of virtual time apart; this shows
> the order and size of the steps, not real-time feel.
> Map `testament.mindmap.json`, 800×600, headless. Script and command below.

## Worked example 2: double-click opens the section editor

The claim from the issue that defined this skill: *"double-click opens the
section editor."* Read literally, that needs a node with **more than one
section** — on a one-section node `text_edit.section_idx` is `0` whether the
editor is per-section or per-node, so the demo cannot tell them apart. (This
example first used a one-section node and showed only "double-click opens a
text editor"; a cold run by a fresh agent caught it.) Testament has one
multi-section node, `3.7`, three sections. The recording half of
`examples/double-click-section-editor.demo`:

```text
@record region:='{"x":330,"y":11800,"w":512,"h":320}' width=640 height=400
input.click_node 3.7 section=2 --wait-settled
query.selection        # kind: "Single", node 3.7 — the first click selects the node
query.mode
clock.step ms=150
input.click_node 3.7 section=2 --wait-settled
clock.step ms=1000
@stop
query.mode             # key_owner: "text_edit"   (was "none")
query.editors          # text_edit.node_id: "3.7", text_edit.section_idx: 2, the buffer is section 2's text
query.selection        # kind: "Section", section_idx: 2
```

The setup (the framing recipe, with two `PanCameraNorth` first because `3.7`
is below the screen at fit) is in the file. Three things carry it: the
**region** crops to the node at 640×400 so its text is drawn 1.25× instead
of shrunk and its neighbor's frame is cropped away; the **second section**
is clicked, so a wrong target (section 0, or the whole node) shows; and the
clicks are **150 ms of virtual time apart** — a frozen clock would make any
two clicks one double-click, so the gap is stated, not implied.

Watched (44 frames, 2.2 s, 0.8 MB, in three runs of identical frames): the
node, nothing selected, for the beat; after the first click all three
sections cyan (the node is selected); after the second only section 2 stays
cyan and a caret `|` appears after its text, held to the end. The caption
says the part the picture cannot: the selection tint and the editor tint are
the same cyan, the caret is the only on-screen sign of the editor, and the
`query.*` lines above are the proof. It also says what is *not* shown — the
editor opens in one frame, with no animation to judge.

## Troubleshooting

| symptom | meaning | fix |
|---|---|---|
| the GIF is two still beats and a jump | a virtual clock, and nothing stepped it between `@record` and `@stop` (an instant change shows as a jump; an animation does not move at all) | add `clock.step ms=<n>` lines; an animation needs steps past its whole `duration_ms`. (`FAIL: … holds 0 or 1 frame(s)` is a recording with nothing between its beats and nothing in them) |
| `FAIL: the recording ended by itself ("max_frames") before @stop` | the app stops a recording at `max_frames` (default 600 — 30 s at 20 fps) or its disk budget, so what was assembled would end before the behavior does; the wrapper refuses it rather than ship a clip that looks complete | shorten the script, lower `fps=`, or `@record max_frames=<n>` (up to 3600) |
| `error: --beat-ms is 1 to 1600000` | a beat under a virtual clock is one `clock.step`, which the app bounds to that range | pass a value in it |
| first frame is wrong (a different zoom, a selection highlight) | the setup did not finish before `@record`, or it left state | `--wait-settled` each setup line, `DeselectAll`, and read the setup's `query.*` output |
| the subject is tiny or off to one side | not framed | the framing recipe, or `@record region:=…` |
| frames look right but the GIF stutters / `NOTE: N frame(s) were due and not taken` | a realtime recording on a slow machine; the manifest's `gaps` name them | use stepped pacing; or accept and say so in the caption |
| `FAIL: <script>:<n>: mandalactl … -> error: not_found: node … is not on screen` | `input.click_node` needs the node on screen | frame first: `ZoomFit` does not always bring a node on screen (testament at 800×600 stops at zoom 0.05); check `scene.node_geometry` and `PanCameraNorth`/`South`/`East`/`West` it in |
| `error: … is inside the repo` | the guard working | write under `/tmp` |
| `error: … exists; pass --overwrite` | `<out>` or `<out stem>.frames/` is there | `--overwrite`: the old files are replaced only after the new run has passed, so a failed re-run leaves them as they were |
| `WARNING: over GitHub's 10 MiB inline limit` | too big to inline | **Size discipline** |
| exit 2 with an `apt-get install` line | no Vulkan ICD / Xvfb in this environment | run the line, or see mandala-drive's matrix |
| `clock.set_mode … refused while recording` | the mode was changed inside `@record … @stop` | set it in the setup |
| app log in the kept work dir | any `FAIL:` keeps `/tmp/mandala-rec.*` and prints its path and the log's tail | read it; mandala-drive's troubleshooting table covers the app's own messages |

Everything above that is a launch problem — the adapter, the socket, a
frozen loop — is `mandala-drive`'s troubleshooting table; this skill adds
only what recording adds. `scripts/ipc_smoke.sh` is that skill's end-to-end
check; `scripts/record_demo.sh` on the two examples is this one's, and like
the smoke script it is deliberately outside `./test.sh`: it needs the built
binaries, a socket and a process to record, and the suite's GPU-backed tests
assert on state, never on frames (TEST_CONVENTIONS §T8/§T10). Run both
example scripts after changing
anything on the recording path, and whenever this skill is edited.
