// SPDX-License-Identifier: MPL-2.0

//! The app-side execution context a command handler drives the
//! application through (`work_plans/LLM_IPC.md` §D5 "Crossing the
//! `app` boundary").
//!
//! The funnels a command reaches are `dispatch_action`,
//! `dispatch_macro` and `execute_console_line`. They are private to
//! `crate::application::app`, and this module is that module's
//! sibling. So `app` implements [`CommandTarget`] over the event
//! loop's own state, building the same `InputHandlerContext` an input
//! handler receives, and passes it in as `&mut dyn`. `MacroDispatchTarget` uses the same indirection,
//! and it keeps both the funnels private and this module free of
//! `app` internals.
//!
//! **The trait adds no route around a gate.** Each method is one of
//! the existing funnels and nothing else. A future command that
//! needs a path past a privilege gate is a redesign of `format/ipc.md`
//! §Trust model, not a new method here (§D4's composition rule).

use serde_json::Value;

use baumhard::gfx_structs::camera::Camera2D;
use baumhard::mindmap::tree_builder::MindMapTree;

use crate::application::console::LineOutcome;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::Action;
use crate::application::macros::MacroStep;
use crate::application::scene_host::AppScene;

use super::injected::{Overloaded, Pacing, Sample};
use super::space::Point;

/// A part of the app shell's own state that a `query` command reads.
/// The state types behind it (`DragState`, the modal editors) are
/// private to `app`, so the app side projects each one to JSON
/// ([`CommandTarget::shell_state`]) and the `ipc` side never names
/// them. The match on it, in `app/ipc_target.rs`, has no wildcard arm:
/// a new part is a compile error there until it has a projection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShellState {
    /// The camera: center, zoom and surface size (`query.camera`).
    Camera,
    /// The interaction mode, drag state, open modal, hover and pointer
    /// (`query.mode`).
    Mode,
    /// The mode-status line the renderer shows (`query.status`).
    Status,
    /// Every modal editor's live state and the document's previews
    /// (`query.editors`).
    Editors,
}

/// What the canvas draws right now, read-only: the inputs the `scene`
/// family walks (`format/ipc.md` §`scene`). Every part is plain data
/// with no GPU handle in it — the trees the renderer shapes, the
/// identity indices their hit tests resolve through, and the camera
/// that culls them — so a test stands one up without a renderer.
pub(crate) struct SceneView<'a> {
    /// The loaded document: its model answers what the trees do not
    /// carry (a node's `folded` flag, the fold that hides a node).
    pub(crate) document: &'a MindMapDocument,
    /// The node tree, or `None` before the first scene build.
    pub(crate) mindmap_tree: Option<&'a MindMapTree>,
    /// The canvas role trees and their identity indices.
    pub(crate) app_scene: &'a AppScene,
    /// The camera the renderer culls and projects with.
    pub(crate) camera: &'a Camera2D,
}

/// What a command may ask of the running application.
///
/// Every method executes on the main thread, inside the IPC drain
/// that runs right after `drain_inputs`, with the access an input
/// handler has.
pub(crate) trait CommandTarget {
    /// Run `action` through `dispatch_action`, exactly as a key
    /// binding fires it. With `pointer`, the dispatch also carries the
    /// payload a mouse handler builds before it dispatches. The
    /// cursor stands at that point for the duration of the dispatch,
    /// and the `DispatchHit` holds what the canonical click hit test
    /// finds there, so pointer-driven arms (`DoubleClickActivate`, the
    /// orphan-creation and zoom arms) behave as they do under a real
    /// pointer. Afterwards the cursor is back where the user's real
    /// pointer is, so their next wheel or key does not act at a point
    /// the agent chose.
    ///
    /// Returns whether the funnel reported `Handled`.
    fn dispatch_action(&mut self, action: Action, pointer: Option<Point>) -> bool;

    /// Parse and run one console line through the console's own
    /// parse → execute → side-effect drain, without opening the
    /// console and without writing to its scrollback. `None` when no
    /// document is loaded: every verb runs against one.
    fn run_console_line(&mut self, line: &str) -> Option<LineOutcome>;

    /// Whether a macro with this id is registered in any tier.
    fn has_macro(&self, macro_id: &str) -> bool;

    /// Run a registered macro through `dispatch_macro`, under the
    /// tier its loader pinned. Returns the dispatcher's
    /// any-step-executed flag.
    fn run_macro(&mut self, macro_id: &str) -> bool;

    /// Run an anonymous step sequence through the same gated step
    /// loop, at the `User` tier: the steps were authored by the IPC
    /// client, and IPC executes as `User` (`format/ipc.md` §Trust
    /// model). Returns the any-step-executed flag.
    fn run_macro_steps(&mut self, steps: &[MacroStep]) -> bool;

    /// Whether anything run through this target so far may have
    /// changed what the window draws. The executor reads it after the
    /// handler returns, and the event loop requests a redraw only when
    /// it is set. So a refused action, a failed or read-only console
    /// line, or a macro whose steps all skipped costs no frame. The
    /// implementation answers from each funnel's own outcome, the only
    /// place that knows.
    fn changed_view(&self) -> bool;

    /// The loaded document, **read-only**. `None` when the app has no
    /// document (the initial load failed). The read half of the
    /// `query` family goes through here, so what it can reach is what
    /// `&MindMapDocument` allows: nothing that flips `dirty`, pushes
    /// an undo entry or rebuilds a scene.
    fn document(&self) -> Option<&MindMapDocument>;

    /// The app-clock ms at which a tree-mutating drag suspended the
    /// animation tick, or `None` while ticks run. `query.animations`
    /// reads it: during the pause the animations are frozen, and a
    /// reply that kept advancing their clocks would promise a commit
    /// the next tick does not make.
    fn animation_pause(&self) -> Option<u64>;

    /// One part of the app shell's state, projected to JSON.
    /// Read-only, and it reports no view change
    /// ([`Self::changed_view`] is untouched), so a query polled in a
    /// loop costs no frame.
    fn shell_state(&self, which: ShellState) -> Value;

    /// The drawn scene, read-only. `None` when the app has no
    /// document (the initial load failed). Like
    /// [`Self::document`], what it reaches is what `&` allows: the
    /// `scene` family cannot rebuild a tree or move the camera.
    fn scene(&self) -> Option<SceneView<'_>>;

    /// What a click at `point` would target, through the canonical
    /// click hit test (`compute_click_hit`, the one mouse presses and
    /// touch taps run, never a fork of it) and its last rung, the
    /// connection path under the pointer: the whole ladder, as JSON
    /// (`format/ipc.md` §`scene.hit_test`). `None` when the app has no
    /// document.
    ///
    /// `&mut` only because the hit tests memoize subtree AABBs in the
    /// trees they descend. Nothing else is written: the document, the
    /// selection, the cursor and the camera are untouched, and
    /// [`Self::changed_view`] stays as it was, so a probe costs no
    /// frame.
    fn hit_test(&mut self, point: Point) -> Option<Value>;

    /// `point` in surface pixels, through the renderer's camera as it
    /// stands now: the position a synthetic pointer is placed at.
    fn surface_point(&self, point: Point) -> (f64, f64);

    /// Deliver raw input to the handler entry points winit events
    /// reach (`format/ipc.md` §`input`). A [`Pacing::Burst`] applies
    /// now when nothing is queued ahead of it, and the reply's
    /// `applied` says so; anything else joins the injected-input
    /// queue, which the heartbeat drains one sample per frame. A
    /// command that would overflow the queue enqueues nothing.
    ///
    /// Applying a sample marks the view changed exactly when the
    /// winit arm it stands in for would have requested a redraw.
    fn inject(&mut self, samples: Vec<Sample>, pacing: Pacing) -> Result<Injected, Overloaded>;

    /// Samples still waiting in the injected-input queue. Read-only.
    fn injected_pending(&self) -> usize;
}

/// What [`CommandTarget::inject`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Injected {
    /// The samples were applied before the command returned.
    pub(crate) applied: bool,
    /// Samples still waiting in the queue afterwards.
    pub(crate) pending: usize,
}
