// SPDX-License-Identifier: MPL-2.0

//! `dispatch_action` — the single entry point that runs `Action`
//! bodies on native. Mouse handlers and the keyboard handler funnel
//! through here. WASM has its own dispatch path today; the
//! convergence track is documented in `work_plans/WASM_CONVERGENCE.md`.
//! Adding a new behavior
//! is variant + default + arm, in that order; never inline a body in
//! a handler.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::document::{ChangeSource, EdgeRef, SelectionState, UndoAction};
use crate::application::keybinds::Action;

use super::super::click::rebuild_all_with_mode;
use super::super::color_picker_flow::{
    apply_picker_nudge, cancel_color_picker, close_color_picker_standalone, commit_color_picker,
    commit_color_picker_to_selection, open_color_picker_standalone, picker_decline_reason, picker_op_for,
    PickerOp,
};
use super::super::console_input::{
    rebuild_console_overlay, save_console_history, save_document_to_bound_path,
};
use super::super::fire_custom_mutation::fire_custom_mutation;
use super::super::input_context::InputHandlerContext;
use super::super::scene_rebuild::rebuild_all;
use super::super::single_line_edit::{
    close_single_line_edit, open_single_line_edit, resolve_single_line_target, SingleLineEditTarget,
};
use super::super::{DragState, HeldInput, InteractionMode, PanOwner};
use crate::application::console::ConsoleState;

// `DispatchHit` lives in `cross_dispatch::pointer` — both targets'
// mouse handlers populate one now, so the payload is cross-platform.
// Re-exported through `dispatch/mod.rs` so the `super::dispatch::
// DispatchHit` import shape at the call sites is unchanged.
pub(in crate::application::app) use super::cross_dispatch::DispatchHit;

// `DispatchOutcome` lives in `cross_dispatch`; the dispatch arms
// here name it via `super::DispatchOutcome` (re-exported in
// `dispatch/mod.rs`).
use super::DispatchOutcome;

/// Quote a free-form string (typically a filesystem path) so the
/// console parser sees it as a single token. Wraps with `"..."`
/// unconditionally and escapes both `\` (→ `\\`) and `"` (→ `\"`)
/// so Windows-style paths and embedded quotes round-trip cleanly
/// through `parser::tokenize`'s quoted-string handling. Order
/// matters: backslash MUST be escaped before quote, otherwise a
/// path ending in `\` produces an unterminated quoted token.
/// Used by the parametric filesystem Action arms.
fn quote_console_arg(s: &str) -> String {
    let mut escaped = String::with_capacity(s.len() + 2);
    escaped.push('"');
    for ch in s.chars() {
        if ch == '\\' || ch == '"' {
            escaped.push('\\');
        }
        escaped.push(ch);
    }
    escaped.push('"');
    escaped
}

/// What [`Action::PanCanvas`] does with the drag state it finds.
///
/// Pure so the arm is pinnable at `DragState` level — the arm itself
/// takes an `InputHandlerContext`, i.e. a live wgpu device, which
/// only the suite's GPU-backed tier stands up (TEST_CONVENTIONS §T8).
/// Same shape, and for the same reason, as
/// `event_mouse_click::route_middle_button`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PanCanvasRoute {
    /// Nothing is in flight that a pan would destroy — arm a
    /// [`PanOwner::Toggled`] pan. A held input that dispatched this
    /// claims it next ([`dispatch_held_action`]); anything else leaves
    /// it toggled on.
    Arm,
    /// A toggled pan is running: this dispatch is the toggle, and
    /// ends it.
    ToggleOff,
    /// A pan some input is holding is running. It ends on that
    /// input's release, so leave it exactly as it is — this includes
    /// a key's own auto-repeat, which re-dispatches `PanCanvas` while
    /// the key that holds the pan is down.
    Held,
    /// Another gesture owns the drag state and owes the model a
    /// commit. Leave it exactly as it is, and do nothing else.
    Refuse,
}

/// Route one `Action::PanCanvas` dispatch against the drag state it
/// finds.
///
/// The guard belongs on the Action rather than on any one of the
/// gestures that reach it (CODE_CONVENTIONS §3), because `PanCanvas`
/// has three entry points and only one of them was ever guarded:
/// `MouseGesture::MiddleClick` (routed through
/// `event_mouse_click::route_middle_button`),
/// `MouseGesture::LeftDrag` (which only runs from
/// `DragState::Pending`), and **any keyboard binding or macro step
/// naming `pan_canvas`** — `keybinds::config` declares it
/// user-bindable, `event_keyboard` dispatches with no drag-state
/// check, and `SourceTier::allows_action` does not gate it, so every
/// tier reaches it. Pre-fix that third route overwrote
/// `DragState::Throttled(..)` with `Panning` exactly the way the
/// middle button used to: the abandoned drag's
/// `commit_on_release_core` never ran, so the tree kept the dragged
/// offsets until the next model rebuild snapped them back, with no
/// undo entry to recover from. Same class as #37 item 5, one
/// dispatch surface over.
///
/// `Pending` is not in the refused class and must not be: the
/// `LeftDrag` threshold cross dispatches `PanCanvas` *from*
/// `Pending`, so refusing there would leave the left button unable
/// to pan at all.
///
/// **Toggle or hold is the input's, not the arm's** (#153,
/// [`PanOwner`]). The arm arms one shape; whether that pan is
/// momentary is decided by who dispatched it. That is why a running
/// pan answers two different ways here: a toggled one was armed by an
/// input with no release, so the next `pan_canvas` — from a key, a
/// macro, the console or IPC — is the only dispatch that can end it,
/// while a held one already has its end coming and a second dispatch
/// must not pre-empt it.
fn route_pan_canvas(drag_state: &DragState) -> PanCanvasRoute {
    match drag_state {
        DragState::Panning {
            owner: PanOwner::Toggled,
        } => PanCanvasRoute::ToggleOff,
        DragState::Panning {
            owner: PanOwner::Held(_),
        } => PanCanvasRoute::Held,
        state if state.would_abandon_gesture() => PanCanvasRoute::Refuse,
        _ => PanCanvasRoute::Arm,
    }
}

/// End a rubber band that is live when a target-picker mode
/// (`Reparent` / `Connect`) takes the pointer.
///
/// The two modes swallow both halves of the left button —
/// `handle_mouse_input` consumes the release as "choose target" and
/// never reaches the `mem::replace` that ends a `SelectingRect`, and
/// `handle_cursor_moved` returns before the drag-state ladder — so a
/// band entered mid-gesture would otherwise sit frozen on the canvas
/// for the whole picker session, and its covered set would be painted
/// by every hover rebuild the mode runs.
///
/// Only `SelectingRect` is ended. The other states a picker mode can
/// interrupt are not this function's call: `Throttled(..)` owes the
/// model a commit that its own release still performs, and dropping
/// it here would be the silent loss #37 item 5 is about.
///
/// The shell around [`take_rubber_band_for_target_picker`], which is
/// the whole of it except dropping the overlay rectangle —
/// `&mut Renderer` is a live wgpu device, which only the suite's
/// GPU-backed tier stands up (TEST_CONVENTIONS §T8).
fn end_target_picker_rubber_band(
    drag_state: &mut DragState,
    document: &mut Option<crate::application::document::MindMapDocument>,
    renderer: &mut crate::application::renderer::Renderer,
) {
    if take_rubber_band_for_target_picker(drag_state, document) {
        renderer.clear_selection_rect_overlay();
    }
}

/// The renderer-free half of [`end_target_picker_rubber_band`]:
/// drop the drag state and the covered set it authorized, and report
/// whether there was one.
///
/// The per-frame drain would end the covered set on its own next
/// frame — that is where the invariant lives, not here — but a mode
/// entry rebuilds immediately, and that rebuild would paint the
/// abandoned set once on its way out.
fn take_rubber_band_for_target_picker(
    drag_state: &mut DragState,
    document: &mut Option<crate::application::document::MindMapDocument>,
) -> bool {
    if !matches!(drag_state, DragState::SelectingRect { .. }) {
        return false;
    }
    *drag_state = DragState::None;
    document
        .as_mut()
        .and_then(crate::application::document::MindMapDocument::take_rect_select_preview);
    true
}

/// Run an `Action` against the live application context. The body of
/// every Document-level action lives here; handlers (`event_keyboard`,
/// `event_mouse_click`, the macro runtime via `dispatch_macro`)
/// construct an `InputHandlerContext` and call this.
///
/// `hit` carries mouse-event-only payload (what the click hit, where
/// the cursor was in canvas space). Keyboard / macro callers pass
/// `None`; mouse callers populate it before invoking the dispatcher.
///
/// **Two-stage dispatch.** Every call routes through the cross-
/// platform [`super::action_core::dispatch_compatible`] first.
/// On `Handled`, this returns immediately — that path covers every
/// Compatible-classified Action plus the cross-platform slice of
/// mixed-branch arms (`ExitMode`'s `last_click` clear and
///   Resize-mode reset,
/// `EditSelection*`-Single open). The native match below runs only
/// when the cross-platform dispatcher returns `Unhandled`, which
/// means one of:
///   - a NativeOnly Action whose body needs `NativeContextExt` fields
///     (console / picker / interaction_mode / drag — see
///     `Action::wasm_compatibility`'s NativeOnly classification),
///   - a mixed-branch arm's native residual (`ExitMode`'s mode
///     reset + rebuild; `EditSelection*` on EdgeLabel / Portal
///     selections),
///   - the edge-label branch of `DoubleClickActivate`. Not a
///     payload question — `dispatch_compatible` takes the same
///     `Option<&DispatchHit>` this function does, and both targets
///     populate it from their mouse handlers. What the branch needs
///     is `single_line_edit_state`, a `NativeContextExt` field the
///     browser has no counterpart for; the arm below is that one
///     step and nothing else. `CreateOrphanNodeAndEdit` no longer
///     appears here at all: `dispatch_create_orphan_and_edit` (gone)
///     went with it, and the mouse path reaches
///     `apply_create_orphan_node_and_edit` through
///     `DoubleClickRoute::CreateOrphanAndEdit`,
///   - a `DoubleClickActivate` dispatched with no `DispatchHit` at
///     all (a macro, say). That is a soft-skip: nothing ran. The arm
///     below finds no target, does nothing, and returns `Unhandled`
///     itself, so this function — which is what the *native* macro
///     loop reads — reports "did not run" for it. That is a behavior
///     change on native: the arm used to return `Handled`
///     unconditionally, so a native macro step
///     `[Action(DoubleClickActivate)]` bumped `any_ran` for a step
///     that touched nothing, and stopped falling through to the
///     custom-mutation tier. WASM was fixed first; this is the same
///     fix on the other side of the seam, so the two targets now
///     answer identically for identical input.
///
/// `work_plans/WASM_CONVERGENCE.md` Track C records the architecture; calling
/// `dispatch_compatible` from this fn is the seam.
pub(in crate::application::app) fn dispatch_action(
    action: Action,
    ctx: &mut InputHandlerContext<'_>,
    hit: Option<&DispatchHit>,
) -> DispatchOutcome {
    // The funnel the document revision's change source names
    // (`document/revision.rs`).
    driven_by(ctx, ChangeSource::Action, |ctx| {
        dispatch_action_arms(action, ctx, hit)
    })
}

/// [`dispatch_action`] on behalf of a *held* input — a mouse button
/// or a key that is down now and will come up — so a pan the
/// dispatch arms ends on `input`'s release (#153).
///
/// The mouse presses (middle press, the `LeftDrag` and `RightDrag`
/// threshold crosses) and the key press go through here. A dispatch
/// with no release behind it (a macro step, a console line, IPC
/// `act.action`) calls [`dispatch_action`] and leaves its pan
/// toggled, and so does the touch `LongPress`: [`HeldInput`] has no
/// finger, so nothing could claim its pan. The arm cannot tell the
/// two apart itself: the funnel hands it an `Action`, and the input
/// is gone by then.
pub(in crate::application::app) fn dispatch_held_action(
    action: Action,
    ctx: &mut InputHandlerContext<'_>,
    hit: Option<&DispatchHit>,
    input: HeldInput,
) -> DispatchOutcome {
    let was_panning = ctx.drag_state.is_panning();
    let outcome = dispatch_action(action, ctx, hit);
    ctx.drag_state.hand_armed_pan_to(was_panning, input);
    outcome
}

/// Run `body` with `source` declared on the document `ctx` holds, and
/// hand driving back afterwards. The one place a handler-context
/// funnel enters and leaves a source, so no early return in `body` can
/// skip the exit. A document `body` swaps out takes the restore too,
/// which only hands its driver back to what it was.
fn driven_by<R>(
    ctx: &mut InputHandlerContext<'_>,
    source: ChangeSource,
    body: impl FnOnce(&mut InputHandlerContext<'_>) -> R,
) -> R {
    let displaced = ctx.document.as_mut().map(|doc| doc.enter_source(source));
    let result = body(ctx);
    if let (Some(displaced), Some(doc)) = (displaced, ctx.document.as_mut()) {
        doc.leave_source(displaced);
    }
    result
}

/// The body of [`dispatch_action`], run with the document's change
/// source already declared.
fn dispatch_action_arms(
    action: Action,
    ctx: &mut InputHandlerContext<'_>,
    hit: Option<&DispatchHit>,
) -> DispatchOutcome {
    // Cross-platform stage. Bounded scope so `_` (the unused
    // `NativeContextExt` view returned by `split_borrow`) drops
    // before the outer match re-borrows `ctx`.
    let cross_outcome = {
        // `_` (not `_ext`) — the extension view is constructed by
        // `split_borrow` because it returns the pair, but the
        // cross-platform dispatcher takes only `core`. The native-
        // only arms below re-borrow from `ctx` directly after this
        // scope drops.
        let (mut core, _) = ctx.split_borrow();
        super::action_core::dispatch_compatible(&action, &mut core, hit)
    };
    if matches!(cross_outcome, DispatchOutcome::Handled) {
        return cross_outcome;
    }
    match action {
        // The loop ends at its next exit check (`app::pump::iterate`),
        // through the same shutdown a closed window takes.
        Action::Quit => {
            *ctx.quit_requested = true;
            DispatchOutcome::Handled
        }
        Action::OpenConsole => {
            if ctx.console_state.is_open() {
                save_console_history(ctx.console_history);
                *ctx.console_state = ConsoleState::Closed;
                ctx.renderer.rebuild_console_overlay_buffers(ctx.app_scene, None);
            } else {
                *ctx.console_state = ConsoleState::open(ctx.console_history.clone());
                if let Some(doc) = ctx.document.as_ref() {
                    rebuild_console_overlay(
                        ctx.console_state,
                        doc,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.keybinds,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        // ── Console keystroke Actions (NativeOnly) ──────────────
        // Every `Console*` variant funnels here; the fan-out body
        // lives in `console_input::dispatch::dispatch_console_action`
        // alongside the `edit::*` private helpers it reaches.
        // Or-pattern groups all 22 variants under one arm — they
        // share the delegation. CODE_CONVENTIONS §3: the funnel
        // requires each user-named effect to be an `Action` and
        // reach `dispatch_action`; it does not require one match
        // arm per variant.
        Action::ConsoleClose
        | Action::ConsoleSubmit
        | Action::ConsoleTabComplete
        | Action::ConsoleHistoryUp
        | Action::ConsoleHistoryDown
        | Action::ConsoleCursorLeft
        | Action::ConsoleCursorRight
        | Action::ConsoleCursorHome
        | Action::ConsoleCursorEnd
        | Action::ConsoleDeleteBack
        | Action::ConsoleDeleteForward
        | Action::ConsoleInsertSpace
        | Action::ConsoleClearLine
        | Action::ConsoleJumpStart
        | Action::ConsoleJumpEnd
        | Action::ConsoleKillToStart
        | Action::ConsoleKillWord
        | Action::ConsoleScrollUp
        | Action::ConsoleScrollDown
        | Action::ConsoleScrollPageUp
        | Action::ConsoleScrollPageDown
        | Action::ConsoleScrollEnd
        | Action::ConsoleScrollHome => {
            // Every Console* body edits the open modal. With the
            // console closed there is nothing for one to act on (the
            // keyboard only reaches here while it is open), so a macro
            // step or an IPC `act.action` sending one reports that
            // nothing ran, as the hitless `DoubleClickActivate` does,
            // rather than claiming a keystroke into a modal that is
            // not there.
            if !ctx.console_state.is_open() {
                return DispatchOutcome::Unhandled;
            }
            super::super::console_input::dispatch_console_action(&action, ctx);
            DispatchOutcome::Handled
        }
        Action::ExitMode => {
            // Cross-platform slice (mode reset + rebuild on Resize)
            // already ran in `dispatch_compatible`; that branch
            // returns `Handled` and we never reach here. We arrive
            // only when mode was target-picker (Reparent / Connect)
            // — those depend on `hovered_node` from `NativeContextExt`
            // and the `rebuild_all_with_mode` overlay path (orange /
            // green highlights), so the residual stays native.
            if ctx.interaction_mode.is_target_picker() {
                *ctx.interaction_mode = InteractionMode::Default;
                *ctx.hovered_node = None;
                if let Some(doc) = ctx.document.as_ref() {
                    rebuild_all_with_mode(
                        doc,
                        ctx.interaction_mode,
                        ctx.hovered_node.as_deref(),
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::EnterResizeMode => {
            // The body is cross-platform (mode flip + scene rebuild)
            // and lives in `apply_enter_resize_mode`, but the Action
            // is `wasm = NativeOnly` until WASM gains a resize
            // gesture pipeline (no `DragState`, no handle hit-test
            // on `run_wasm/event_mouse_click.rs` today). On native,
            // the helper does the resolve + flip + rebuild.
            let (mut core, _ext) = ctx.split_borrow();
            super::action_core::with_doc_rebuild(&mut core, super::cross_dispatch::apply_enter_resize_mode);
            DispatchOutcome::Handled
        }
        Action::FastResizeStart => {
            // Fast-resize gesture start: the press-time target, the
            // press-time anchor and the owning button all come off
            // `hit` (`fast_resize_drag`), and the drag drains and
            // commits through the existing resize plumbing on its
            // owning button's release. A refusal ran nothing — the
            // native macro loop reads this for `any_ran` — and says why,
            // which IPC `act.action` returns as its `reason`.
            match apply_fast_resize_start(ctx, hit) {
                Ok(()) => DispatchOutcome::Handled,
                Err(reason) => DispatchOutcome::Refused(reason),
            }
        }
        Action::EnterNodeEdit | Action::EnterNodeEditClean => {
            // NodeEdit-mode entry. Single-section nodes
            // short-circuit to the editor; multi-section nodes
            // stop at NodeEdit and the user selects a section.
            // `wasm = NativeOnly` because the editor depends on
            // `TextEditState` (modal-stealer cascade is native).
            let clean = matches!(action, Action::EnterNodeEditClean);
            let (mut core, _ext) = ctx.split_borrow();
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::RebuildContext {
                    document: doc,
                    mindmap_tree: core.mindmap_tree,
                    app_scene: core.app_scene,
                    renderer: core.renderer,
                    scene_cache: core.scene_cache,
                    interaction_mode: core.interaction_mode,
                };
                let _ = super::cross_dispatch::apply_enter_node_edit(clean, &mut rc, core.text_edit_state);
            }
            DispatchOutcome::Handled
        }
        Action::EnterSectionEdit => {
            // SectionEdit (open text editor) on the active section
            // while in NodeEdit. Same NativeOnly story as
            // EnterNodeEdit — the editor is native-only.
            let (mut core, _ext) = ctx.split_borrow();
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::cross_dispatch::RebuildContext {
                    document: doc,
                    mindmap_tree: core.mindmap_tree,
                    app_scene: core.app_scene,
                    renderer: core.renderer,
                    scene_cache: core.scene_cache,
                    interaction_mode: core.interaction_mode,
                };
                let _ = super::cross_dispatch::apply_enter_section_edit(false, &mut rc, core.text_edit_state);
            }
            DispatchOutcome::Handled
        }
        Action::EnterReparentMode => {
            // `sel` is collected out of the borrow rather than held
            // across the block: `end_target_picker_rubber_band` takes
            // `&mut Option<MindMapDocument>`, and the rebuild below
            // re-resolves the document afterwards.
            let sel: Vec<String> = ctx
                .document
                .as_ref()
                .map(|doc| {
                    doc.selection
                        .selected_ids()
                        .iter()
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();
            if !sel.is_empty() {
                *ctx.interaction_mode = InteractionMode::Reparent { sources: sel };
                *ctx.hovered_node = None;
                *ctx.last_click = None;
                end_target_picker_rubber_band(ctx.drag_state, ctx.document, ctx.renderer);
                if let Some(doc) = ctx.document.as_ref() {
                    rebuild_all_with_mode(
                        doc,
                        ctx.interaction_mode,
                        ctx.hovered_node.as_deref(),
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::EnterConnectMode => {
            let source = match ctx.document.as_ref().map(|doc| &doc.selection) {
                Some(SelectionState::Single(source)) => Some(source.clone()),
                _ => None,
            };
            if let Some(source) = source {
                *ctx.interaction_mode = InteractionMode::Connect { source };
                *ctx.hovered_node = None;
                *ctx.last_click = None;
                end_target_picker_rubber_band(ctx.drag_state, ctx.document, ctx.renderer);
                if let Some(doc) = ctx.document.as_ref() {
                    rebuild_all_with_mode(
                        doc,
                        ctx.interaction_mode,
                        ctx.hovered_node.as_deref(),
                        ctx.mindmap_tree,
                        ctx.app_scene,
                        ctx.renderer,
                        ctx.scene_cache,
                    );
                }
            }
            DispatchOutcome::Handled
        }
        Action::ReparentToTarget(ref target) => {
            // Mode-exit + mutation: extract sources from
            // `interaction_mode` atomically with the reset to
            // `Default`. A stale fire outside Reparent mode silently
            // no-ops; the `mem::replace` guards against re-entry
            // leaving the mode half-reset on early return.
            let sources = match std::mem::replace(ctx.interaction_mode, InteractionMode::Default) {
                InteractionMode::Reparent { sources } => sources,
                _ => {
                    return DispatchOutcome::Handled;
                }
            };
            *ctx.hovered_node = None;
            if let Some(doc) = ctx.document.as_mut() {
                // `target` of `Some(id)` reparents under that node;
                // `None` promotes sources to root (empty-canvas click).
                let undo_data = doc.apply_reparent(&sources, target.as_deref());
                if !undo_data.entries.is_empty() {
                    doc.undo_stack.push(UndoAction::ReparentNodes {
                        entries: undo_data.entries,
                        old_edges: undo_data.old_edges,
                    });
                    doc.mark_dirty();
                }
                // Full rebuild regardless: tree structure changed
                // (or even if no-op, mode-exit must clear orange/
                // green highlights).
                rebuild_all(
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::ConnectToTarget(ref target) => {
            // Mirror `ReparentToTarget`'s mode-exit pattern. Source
            // comes from `InteractionMode::Connect { source }`;
            // stale-fire outside Connect mode silently no-ops.
            // `target = None` is empty-canvas mode-exit (no edge to
            // create); the arm still runs the rebuild so orange /
            // green highlights clear.
            let source = match std::mem::replace(ctx.interaction_mode, InteractionMode::Default) {
                InteractionMode::Connect { source } => source,
                _ => {
                    return DispatchOutcome::Handled;
                }
            };
            *ctx.hovered_node = None;
            if let Some(doc) = ctx.document.as_mut() {
                if let Some(target_id) = target.as_deref() {
                    if let Some(idx) = doc.create_cross_link_edge(&source, target_id) {
                        doc.undo_stack.push(UndoAction::CreateEdge { index: idx });
                        // Snap selection to the new edge so the
                        // user gets immediate visual confirmation
                        // and can Delete or style it next.
                        doc.selection = SelectionState::Edge(EdgeRef::new(
                            source.clone(),
                            target_id.to_string(),
                            "cross_link",
                        ));
                        doc.mark_dirty();
                    }
                }
                rebuild_all(
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::EditSelection | Action::EditSelectionClean => {
            // The Single branch is owned by the cross-platform
            // dispatcher (`dispatch_compatible`'s mixed-branch slice).
            // This arm runs ONLY when that returned `Unhandled`,
            // which means selection was non-Single — so we go
            // straight to the EdgeLabel / Portal native-only
            // branches without re-checking Single.
            //
            // `clean` is threaded into the single-line editors so
            // `EditSelectionClean` keeps its empty-buffer contract
            // on edge-label / portal selections, not just on nodes.
            let clean = matches!(action, Action::EditSelectionClean);
            open_editor_for_edge_selection(clean, ctx);
            DispatchOutcome::Handled
        }
        Action::SaveDocument => match ctx.document.as_mut().map(save_document_to_bound_path) {
            Some(true) => DispatchOutcome::Handled,
            Some(false) | None => DispatchOutcome::Unhandled,
        },

        // ── Mouse-gesture Actions ──────────────────────────────
        Action::DoubleClickActivate => {
            // The cross-platform stage above ran the whole gesture
            // except one branch: an edge-label double-click commits
            // the selection there and hands back
            // `DoubleClickResidual::OpenEdgeLabelEditor`, which
            // surfaces here as `Unhandled`. The single-line editor is
            // the only piece that needs `NativeContextExt` state, so
            // it is the only piece left in this arm.
            //
            // `edge_label_target` is the same `EdgeKey` -> `EdgeRef`
            // conversion the route resolver used, so the editor
            // cannot open on a different edge than the one the
            // selection just committed to.
            let target = hit.and_then(|h| super::edge_label_target(&h.click_hit));
            match (target, ctx.document.as_mut()) {
                (Some(edge_ref), Some(doc)) => {
                    // Double-click on an edge label edits the existing
                    // text — not clean.
                    open_single_line_edit(
                        SingleLineEditTarget::EdgeLabel { edge_ref },
                        false,
                        doc,
                        ctx.single_line_edit_state,
                        ctx.app_scene,
                        ctx.renderer,
                    );
                    DispatchOutcome::Handled
                }
                // No edge-label target means no `DispatchHit` at all:
                // the cross-platform stage only hands this arm an
                // `Unhandled` for the edge-label residual (which
                // always has one) or for the hitless soft-skip. So
                // this is the soft-skip — nothing ran here and
                // nothing ran above — and it is now reported as such
                // on native too. It previously returned `Handled`,
                // which bumped the *native* macro loop's `any_ran`
                // for a step that did nothing, the mirror image of
                // the WASM misreport fixed in `c023ff9`. Both targets
                // now report the same thing for the same input.
                (None, _) => DispatchOutcome::Unhandled,
                // A label hit with no document loaded cannot reach
                // here — `apply_double_click_activate` returns `Done`
                // (and so `Handled`) when there is no document — but
                // "nothing ran" is the honest answer if it ever does.
                (Some(_), None) => DispatchOutcome::Unhandled,
            }
        }
        Action::PanCanvas => match route_pan_canvas(ctx.drag_state) {
            // Armed toggled; a held input claims it on the way out of
            // `dispatch_held_action`, and its release ends it there
            // (`DragState::end_pan_held_by`). This arm handles only
            // the two ends a dispatch can reach.
            PanCanvasRoute::Arm => {
                *ctx.drag_state = DragState::Panning {
                    owner: PanOwner::Toggled,
                };
                DispatchOutcome::Handled
            }
            PanCanvasRoute::ToggleOff => {
                *ctx.drag_state = DragState::None;
                DispatchOutcome::Handled
            }
            // Nothing ran, and the caller — the native macro loop
            // reads this for `any_ran` — is told so.
            PanCanvasRoute::Held => {
                log::debug!("dispatch: PanCanvas ignored (a held input owns the pan); state stays put");
                DispatchOutcome::Refused(
                    "a pan is already running, held by a button or key; its release ends it",
                )
            }
            PanCanvasRoute::Refuse => {
                log::debug!("dispatch: PanCanvas ignored (uncommitted gesture in flight); state stays put");
                DispatchOutcome::Refused(
                    "a drag is in flight and owes the model a commit; its release ends it",
                )
            }
        },
        // ── Console-verb Actions ───────────────────────────────
        Action::OpenColorPicker => {
            // Mirror `color picker on`: open the standalone palette.
            if let Some(doc) = ctx.document.as_mut() {
                open_color_picker_standalone(
                    doc,
                    ctx.color_picker_state,
                    ctx.interaction_mode,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::CloseColorPicker => {
            // Mirror `color picker off`.
            if let Some(doc) = ctx.document.as_mut() {
                close_color_picker_standalone(
                    ctx.color_picker_state,
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
            DispatchOutcome::Handled
        }
        Action::LabelEditOnSelection => {
            // Mirror `label edit`: open the inline editor on the
            // currently-selected edge / portal-endpoint, seeded
            // with the existing text (the console verb has no
            // "clean" spelling).
            open_editor_for_edge_selection(false, ctx);
            DispatchOutcome::Handled
        }

        // ── Modal commit / cancel ────────────────────────────
        // §3 funnel: modal handlers used to call `close_*` helpers
        // inline. Commit/cancel are user-named effects (Esc /
        // Enter / click-outside), NOT the §3 carve-out for literal
        // Key character insertion — so they belong in the funnel.
        // `TextEditCommit` / `TextEditCancel` are Compatible and
        // handled by `dispatch_compatible` (cross-platform); they
        // never reach this match. `LabelEdit*` are NativeOnly
        // (label_edit + portal_text_edit modules are cfg-gated to
        // native) — handled here. Both reuse the same Action
        // variants since the editors are mutually exclusive by
        // selection-state construction (a node selection opens the
        // text editor; an edge-label selection opens the label
        // editor; a portal-text selection opens the portal-text
        // editor — never two at once). Order is observationally
        // equivalent because at most one is_open(); checking
        // portal-text first picks the more specific selection.
        Action::LabelEditCancel | Action::LabelEditCommit => {
            if let Some(doc) = ctx.document.as_mut() {
                close_single_line_edit(
                    matches!(action, Action::LabelEditCommit),
                    ctx.single_line_edit_state,
                    &mut super::cross_dispatch::rebuild_ctx!(ctx, doc),
                );
            }
            DispatchOutcome::Handled
        }

        // ── LabelEdit cursor primitives ───────────────────────
        //
        // Three declared behavior changes live on this arm, all of
        // them consequences of routing the funnel through the same
        // `handle_input_core` a keystroke takes instead of mutating
        // the buffer directly. Macros, the console and IPC are the
        // callers that reach it without a keystroke.
        //
        // 1. The preview is refreshed after the caret moves; before,
        //    it wrote state and painted nothing.
        // 2. It reaches the portal caption at all; before, it wrote
        //    to the edge-label state only.
        // 3. It meets `still_editable` first — so one of these
        //    actions arriving while the portal caption editor is
        //    open on an edge that has since been deleted or left
        //    portal mode now closes the editor and **discards the
        //    buffer uncommitted**, where before the buffer survived
        //    until Enter or a click outside. That guard existed on
        //    `main` only on the keystroke path; unifying the entry
        //    points widened its reach here. It is the consistent
        //    behavior — one editor, one guard — and it is pinned by
        //    `single_line_edit::tests::oracle::
        //    test_oracle_funnel_action_on_an_invalidated_portal_caption_discards_the_buffer`
        //    so it stays a decision rather than a side effect.
        //
        // The `if let Some(doc)` gate also makes these actions
        // no-ops with no document loaded, where before they still
        // mutated the buffer. Unobservable — an open editor implies
        // a document — noted so the audit is complete.
        Action::LabelEditCursorLeft
        | Action::LabelEditCursorRight
        | Action::LabelEditCursorHome
        | Action::LabelEditCursorEnd
        | Action::LabelEditDeleteBack
        | Action::LabelEditDeleteForward
        | Action::LabelEditInsertSpace => {
            if let Some(doc) = ctx.document.as_mut() {
                super::super::single_line_edit::apply_single_line_edit_action(
                    action,
                    ctx.single_line_edit_state,
                    &mut super::cross_dispatch::rebuild_ctx!(ctx, doc),
                );
            }
            DispatchOutcome::Handled
        }

        // ── Filesystem variants (NativeOnly) ────────────────────
        // Dispatch arms route through `execute_console_line` so the
        // existing `replace_document` / `dirty` / `file_path`
        // plumbing on `ConsoleEffects` is reused. The whole module
        // is already `cfg(not(target_arch = "wasm32"))`, so no
        // additional cfg gate is needed.
        Action::OpenDocument(ref path)
        | Action::SaveDocumentAs(ref path)
        | Action::NewDocumentAt(ref path) => {
            let verb = match action {
                Action::OpenDocument(_) => "open",
                Action::SaveDocumentAs(_) => "save",
                Action::NewDocumentAt(_) => "new",
                _ => {
                    log::error!(
                        "dispatch: fs-variant fan-out missed inner-match variant: {:?}",
                        action,
                    );
                    return DispatchOutcome::Handled;
                }
            };
            let line = format!("{} {}", verb, quote_console_arg(path));
            // The Action applied only if its line did: a missing file
            // or an unwritable path is `Unhandled`, so a caller that
            // reads the outcome (a macro's `any_ran`, an IPC reply)
            // is not told the map was opened or saved when it was not.
            match crate::application::app::console_input::exec::execute_console_line(ctx, &line) {
                Some(run) => {
                    crate::application::app::console_input::exec::log_console_line(&line, &run.outcome);
                    if run.outcome.applied() {
                        DispatchOutcome::Handled
                    } else {
                        DispatchOutcome::Unhandled
                    }
                }
                None => {
                    log::warn!(
                        "dispatch: {}: no document loaded; skipping '{}'",
                        verb,
                        baumhard::util::log::console_line_excerpt(&line)
                    );
                    DispatchOutcome::Unhandled
                }
            }
        }

        // ── Color-picker modal Actions (NativeOnly) ─────────────
        // `PickerCancel` / `PickerCommit` / the six `PickerNudge*`.
        // These used to run entirely inside the picker's own key
        // handler, which made `MacroStep::Action { PickerCommit }`
        // a silent no-op while `TextEditCommit` from a macro
        // worked — the third modal was the odd one out. Same
        // rationale as the `LabelEdit*` arms above: commit /
        // cancel / nudge are user-named effects, not the §3
        // carve-out for literal Key payloads.
        //
        // The guard is `picker_op_for`, the picker module's single
        // source of truth for "the picker owns this Action". The
        // keyboard pre-filter (`event_keyboard.rs`) and the click
        // router (`color_picker_flow::click`) resolve through the
        // same fn, so nothing can be routed toward this funnel
        // without an arm here to receive it, and a ninth `Picker*`
        // variant is live on all three surfaces the moment it is
        // added there.
        ref a if picker_op_for(a).is_some() => {
            let Some(op) = picker_op_for(a) else {
                // The guard just proved `Some`; a mismatch means
                // `picker_op_for` is not a pure function any more.
                // Fail safe per CODE_CONVENTIONS §9.
                log::error!("dispatch: picker arm guard/body disagreement on {:?}", a);
                return DispatchOutcome::Unhandled;
            };
            dispatch_picker_op(op, ctx)
        }

        // Console / Picker / LabelEdit / TextEdit modal-context actions
        // not handled above (e.g. cancel/commit) are dispatched by their
        // respective modal handlers. Falling through to `Unhandled`
        // lets the keyboard handler's contextual resolution own them.
        _ => {
            log::debug!("dispatch: {:?} not handled at Document context", action);
            DispatchOutcome::Unhandled
        }
    }
}

/// Open the single-line editor the current selection names: the
/// edge-label editor for an `EdgeLabel` selection, the portal-text
/// editor for `PortalLabel` / `PortalText`. Any other selection
/// logs and no-ops.
///
/// One body for two arms. `Action::EditSelection` /
/// `EditSelectionClean` reach it as the native residual after
/// `dispatch_compatible` declines the node-scoped selections;
/// `Action::LabelEditOnSelection` (the `label edit` console verb's
/// Action mirror) reaches it directly. The two used to carry
/// byte-identical match bodies, and the `EditSelectionClean` half
/// computed a `clean` flag it then discarded — so the "empty
/// buffer" contract held on nodes and silently didn't on edge
/// labels and portal endpoints.
///
/// A selection whose target evaporated between the selection and
/// the dispatch leaves the editor closed; the `open_*` helpers own
/// that `log::warn!` themselves, so there is nothing for callers
/// here to branch on.
fn open_editor_for_edge_selection(clean: bool, ctx: &mut InputHandlerContext<'_>) {
    let Some(doc) = ctx.document.as_mut() else {
        return;
    };
    let Some(target) = resolve_single_line_target(&doc.selection) else {
        log::debug!("dispatch: edit-selection: selection is not an edge label / portal endpoint; no-op");
        return;
    };
    open_single_line_edit(
        target,
        clean,
        doc,
        ctx.single_line_edit_state,
        ctx.app_scene,
        ctx.renderer,
    );
}

/// Run a [`PickerOp`] against the live picker. Body of the
/// `dispatch_action` picker arm, lifted out so the arm stays a
/// two-liner and the mode branches read in one place.
///
/// Returns `Unhandled` whenever nothing ran — see
/// [`picker_decline_reason`] for the three cases and why each one
/// must not report `Handled`. `Handled` means the op reached its
/// effect, so a nudge that the picker state rejected also reports
/// `Unhandled` rather than claiming success.
fn dispatch_picker_op(op: PickerOp, ctx: &mut InputHandlerContext<'_>) -> DispatchOutcome {
    let standalone = ctx.color_picker_state.is_standalone();
    if let Some(reason) = picker_decline_reason(
        op,
        ctx.color_picker_state.is_open(),
        standalone,
        ctx.document.is_some(),
    ) {
        log::debug!("dispatch: picker action {:?} declined: {:?}", op, reason);
        return DispatchOutcome::Unhandled;
    }
    // `picker_decline_reason` just proved the document is present.
    let Some(doc) = ctx.document.as_mut() else {
        log::error!("dispatch: picker arm: decline check and document borrow disagree");
        return DispatchOutcome::Unhandled;
    };
    match op {
        PickerOp::Cancel => cancel_color_picker(
            ctx.color_picker_state,
            doc,
            ctx.interaction_mode,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        ),
        PickerOp::Commit => {
            if standalone {
                // Standalone: fan the wheel color across the
                // document selection and stay open.
                commit_color_picker_to_selection(
                    ctx.color_picker_state,
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            } else {
                // Contextual: write the bound handle and close.
                commit_color_picker(
                    ctx.color_picker_state,
                    doc,
                    ctx.interaction_mode,
                    ctx.mindmap_tree,
                    ctx.app_scene,
                    ctx.renderer,
                    ctx.scene_cache,
                );
            }
        }
        PickerOp::Nudge(nudge) => {
            // Renderer-free: the preview stamp marks
            // `picker_hover.dirty` and the per-frame drain rebuilds.
            // The helper's `false` means the picker state rejected
            // the nudge, so the op did not take effect — report
            // that rather than a blanket `Handled`.
            if !apply_picker_nudge(nudge, ctx.color_picker_state, doc, ctx.picker_hover) {
                log::debug!(
                    "dispatch: picker nudge {:?} did not apply; reporting Unhandled",
                    nudge
                );
                return DispatchOutcome::Unhandled;
            }
        }
    }
    DispatchOutcome::Handled
}

/// Apply a `LabelEdit*` cursor / delete primitive to a bare
/// `(buffer, cursor)` pair.
///
/// Generic over the carrier so it serves both the single-line
/// editor and any other buffer with grapheme-cursor semantics.
/// Returns `true` when state changed.
pub(in crate::application::app) fn apply_label_edit_action_to_buffer(
    action: Action,
    buffer: &mut String,
    cursor: &mut usize,
) -> bool {
    use super::super::text_edit::{delete_at_cursor, delete_before_cursor};
    use baumhard::util::grapheme_chad;
    let before = *cursor;
    let len_before = buffer.len();
    match action {
        Action::LabelEditCursorLeft => {
            if *cursor > 0 {
                *cursor -= 1;
            }
        }
        Action::LabelEditCursorRight => {
            if *cursor < grapheme_chad::count_grapheme_clusters(buffer) {
                *cursor += 1;
            }
        }
        Action::LabelEditCursorHome => {
            *cursor = 0;
        }
        Action::LabelEditCursorEnd => {
            *cursor = grapheme_chad::count_grapheme_clusters(buffer);
        }
        Action::LabelEditDeleteBack => {
            if *cursor > 0 {
                *cursor = delete_before_cursor(buffer, *cursor);
            }
        }
        Action::LabelEditDeleteForward if *cursor < grapheme_chad::count_grapheme_clusters(buffer) => {
            *cursor = delete_at_cursor(buffer, *cursor);
        }
        // At the end of the buffer delete-forward is a no-op, and it
        // says so in its own arm rather than falling through to the
        // catch-all, so an arm added above `_` later cannot receive it.
        // The guard above is clippy's `collapsible_match` form (the
        // nested `if` the siblings use trips it on the last arm only).
        Action::LabelEditDeleteForward => {}
        Action::LabelEditInsertSpace => {
            *cursor += grapheme_chad::insert_str_at_grapheme_counted(buffer, *cursor, " ");
        }
        _ => {}
    }
    *cursor != before || buffer.len() != len_before
}

// `sibling_id` lifted to `dispatch/cross_dispatch/selection/mod.rs`
// so the WASM dispatcher can reach the same fold-aware navigation
// logic.

/// Run a macro by id against the current `InputHandlerContext`.
/// Iterates the macro's steps in order, forwarding each through the
/// matching dispatch surface:
/// - `MacroStep::Action` → `dispatch_action`
/// - `MacroStep::CustomMutation` → `fire_custom_mutation`
///   (selection-fallback target resolution)
/// - `MacroStep::ConsoleLine` → `console_input::execute_console_line`
///
/// Steps are run sequentially; a step that fails (e.g. an unbound
/// custom-mutation id, or an Action that returns Unhandled) logs and
/// the next step still runs. This matches "best-effort macro" — if a
/// later step depends on an earlier one, the macro author can split
/// it into two macros.
///
/// Returns `true` if any step ran successfully.
///
/// `after_step`, when given, is called with the app as it stands
/// after each step that ran (see [`MacroStepObserver`]).
pub(in crate::application::app) fn dispatch_macro(
    macro_id: &str,
    ctx: &mut InputHandlerContext<'_>,
    after_step: Option<MacroStepObserver<'_>>,
) -> bool {
    // Body lifted to `dispatch_macro_core` (cross-platform); this
    // shim wraps `ctx` in a `NativeMacroDispatchTarget` so the
    // native dispatch funnel calls the same step loop the WASM
    // dispatcher uses. The privilege gate is single-sourced there.
    let mut target = NativeMacroDispatchTarget { ctx, after_step };
    super::macro_core::dispatch_macro(macro_id, &mut target)
}

/// Run an anonymous step sequence through the same gated step loop
/// [`dispatch_macro`] runs a registered macro through, as a macro
/// from tier `source`. `label` names the sequence in diagnostics.
/// `after_step` is as for [`dispatch_macro`].
pub(in crate::application::app) fn run_macro_steps(
    label: &str,
    steps: &[crate::application::macros::MacroStep],
    source: crate::application::source_tier::SourceTier,
    ctx: &mut InputHandlerContext<'_>,
    after_step: Option<MacroStepObserver<'_>>,
) -> bool {
    let mut target = NativeMacroDispatchTarget { ctx, after_step };
    super::macro_core::run_macro_steps(label, steps, source, &mut target)
}

/// An observer called between macro steps, handed the handler
/// context as it stands after the step. It reads the context and
/// changes nothing.
pub(in crate::application::app) type MacroStepObserver<'o> = &'o mut dyn FnMut(&InputHandlerContext<'_>);

/// Native impl of [`super::macro_core::MacroDispatchTarget`].
/// Wraps `&mut InputHandlerContext` and forwards each operation to
/// the existing native helpers (`dispatch_action`,
/// `fire_custom_mutation`, `execute_console_line`).
struct NativeMacroDispatchTarget<'a, 'b, 'o> {
    ctx: &'a mut InputHandlerContext<'b>,
    after_step: Option<MacroStepObserver<'o>>,
}

impl<'a, 'b, 'o> super::macro_core::MacroDispatchTarget for NativeMacroDispatchTarget<'a, 'b, 'o> {
    fn registry(&self) -> &crate::application::macros::MacroRegistry {
        self.ctx.macros
    }

    fn dispatch_action(&mut self, action: Action) -> DispatchOutcome {
        dispatch_action(action, self.ctx, None)
    }

    fn apply_custom_mutation(&mut self, id: &str, node_id: &str) -> bool {
        // Look the mutation up, fire it through the one body every
        // route lands through, rebuild. A macro step names a node,
        // not a section, so the target is whole-node.
        let cm = self
            .ctx
            .document
            .as_ref()
            .and_then(|d| d.mutation_registry.get(id).cloned());
        let Some(cm) = cm else {
            log::warn!("macros: CustomMutation step: unknown custom-mutation id '{}'", id);
            return false;
        };
        let Some(doc) = self.ctx.document.as_mut() else {
            return false;
        };
        let now = self.ctx.clock.now_whole_ms();
        fire_custom_mutation(
            doc,
            self.ctx.mindmap_tree,
            self.ctx.scene_cache,
            &cm,
            node_id,
            None,
            now,
        );
        rebuild_all(
            doc,
            self.ctx.interaction_mode,
            self.ctx.mindmap_tree,
            self.ctx.app_scene,
            self.ctx.renderer,
            self.ctx.scene_cache,
        );
        true
    }

    fn execute_console_line(&mut self, line: &str) -> bool {
        // Every verb runs against a document, so a macro fired before
        // one loads soft-skips. A line that ran but failed (an unknown
        // verb, a verb error) did not apply either. Neither bumps the
        // macro's `any_ran`, so a keystroke whose macro did nothing
        // still falls through to the custom-mutation tier.
        match crate::application::app::console_input::exec::execute_console_line(self.ctx, line) {
            Some(run) => {
                crate::application::app::console_input::exec::log_console_line(line, &run.outcome);
                run.outcome.applied()
            }
            None => {
                log::warn!(
                    "macros: ConsoleLine step: no document loaded; skipping '{}'",
                    baumhard::util::log::console_line_excerpt(line)
                );
                false
            }
        }
    }

    fn current_selection_node_id(&self) -> Option<String> {
        self.ctx.document.as_ref().and_then(|d| {
            if let SelectionState::Single(nid) = &d.selection {
                Some(nid.clone())
            } else {
                None
            }
        })
    }

    fn has_node(&self, node_id: &str) -> bool {
        self.ctx
            .document
            .as_ref()
            .map(|d| d.mindmap.nodes.contains_key(node_id))
            .unwrap_or(false)
    }

    fn step_finished(&mut self) {
        if let Some(observe) = self.after_step.as_mut() {
            observe(self.ctx);
        }
    }
}

/// Fast-resize gesture start (`Action::FastResizeStart`).
///
/// The shell around [`fast_resize_drag`], which decides everything:
/// this only clears the scene cache and installs the drag it hands
/// back. A refusal leaves the drag state as it found it — the
/// threshold cross that dispatched this has already taken its pending
/// press down (#194), so there is nothing stale to reset. `Err` names
/// why nothing started.
fn apply_fast_resize_start(
    ctx: &mut InputHandlerContext<'_>,
    hit: Option<&DispatchHit>,
) -> Result<(), &'static str> {
    let started = match (hit, ctx.document.as_ref()) {
        (None, _) => Err(FastResizeRefusal::NO_POINTER),
        (Some(_), None) => Err(FastResizeRefusal::NO_DOCUMENT),
        (Some(h), Some(doc)) => fast_resize_drag(doc, h),
    };
    match started {
        Ok(drag) => {
            ctx.scene_cache.clear();
            *ctx.drag_state = DragState::throttled(drag);
            Ok(())
        }
        // A fill-parent section is the one refusal a user's own
        // well-aimed drag meets, with nothing on screen to say why
        // the corner did not move, so it is logged at `info!`:
        // `RUST_LOG=info` shows it without the dispatch trace. The
        // rest are no target, or a caller that cannot finalize a
        // drag.
        Err(reason) if reason == FastResizeRefusal::FILL_PARENT => {
            log::info!("dispatch: FastResizeStart refused: {reason}");
            Err(reason)
        }
        Err(reason) => {
            log::debug!("dispatch: FastResizeStart refused: {reason}");
            Err(reason)
        }
    }
}

/// Why a fast-resize started nothing — one sentence each, the text IPC
/// `act.action` returns as `reason`. Named constants so the tests and
/// the smoke check hold the reply to the arm without copying a string.
pub(in crate::application::app) struct FastResizeRefusal;

impl FastResizeRefusal {
    pub(in crate::application::app) const NO_POINTER: &'static str =
        "FastResizeStart needs a pointer gesture to anchor at";
    pub(in crate::application::app) const NO_DOCUMENT: &'static str = "no document is loaded";
    pub(in crate::application::app) const NO_HOLDER: &'static str =
        "FastResizeStart needs a held mouse button to finalize it, and nothing holds this gesture";
    pub(in crate::application::app) const UNFINALIZING_BUTTON: &'static str =
        "only the left and right buttons finalize a drag";
    pub(in crate::application::app) const NOT_ON_A_NODE: &'static str = "the press did not land on a node";
    pub(in crate::application::app) const NODE_GONE: &'static str = "the pressed node no longer exists";
    pub(in crate::application::app) const SECTION_GONE: &'static str = "the pressed section no longer exists";
    pub(in crate::application::app) const FILL_PARENT: &'static str =
        "a fill-parent section has no size of its own to resize";
}

/// The resize a fast-resize gesture starts, or why it starts none.
///
/// Everything comes off `hit`, which the drag gesture's threshold
/// cross builds (`event_cursor_moved`; #194):
///
/// - **the target** is `hit.click_hit` — the press-time body hit, a
///   node or one of its sections;
/// - **the anchor** is inferred from `hit.canvas_pos`, which the cross
///   fills with the **press-time** canvas position, not the cursor's
///   at the cross: "Quadrant determined at press time, not
///   continuously" (`SECTIONS_BORDERS_RESIZE_PLAN.md` §6.3), so the
///   corner you grab is the one nearest where you pressed;
/// - **the owner** is `hit.held_by`, the button whose release
///   finalizes the drag. A hit no button holds — a synthetic IPC
///   pointer — is refused rather than start a drag no release can end.
///
/// Reading all three off the hit, not off `DragState`, is what lets
/// the cross take the pending press down before it dispatches, so a
/// `RightDrag`-bound `pan_canvas` can arm (#194).
///
/// Renderer-free, so the harness drives it (§T8).
fn fast_resize_drag(
    doc: &crate::application::document::MindMapDocument,
    hit: &DispatchHit,
) -> Result<super::super::throttled_interaction::ThrottledDrag, &'static str> {
    use baumhard::mindmap::tree_builder::infer_resize_anchor;
    use glam::Vec2;

    use super::super::throttled_interaction::{
        NodeResizeInteraction, SectionResizeInteraction, ThrottledDrag,
    };
    use crate::application::platform::input::MouseButton;

    let started_with_right = match hit.held_by {
        Some(MouseButton::Right) => true,
        Some(MouseButton::Left) => false,
        Some(_) => return Err(FastResizeRefusal::UNFINALIZING_BUTTON),
        None => return Err(FastResizeRefusal::NO_HOLDER),
    };
    let (node_id, section_idx) = match &hit.click_hit {
        super::super::ClickHit::Node(id, section_idx) => (id, *section_idx),
        _ => return Err(FastResizeRefusal::NOT_ON_A_NODE),
    };
    let node = doc
        .mindmap
        .nodes
        .get(node_id)
        .ok_or(FastResizeRefusal::NODE_GONE)?;

    // Two targets: a section of a multi-section node, or the node.
    if let Some(section_idx) = section_idx {
        let section = node
            .sections
            .get(section_idx)
            .ok_or(FastResizeRefusal::SECTION_GONE)?;
        let start_size = section.size.ok_or(FastResizeRefusal::FILL_PARENT)?;
        let start_offset = section.offset;
        let aabb_pos = Vec2::new(
            (node.position.x + start_offset.x) as f32,
            (node.position.y + start_offset.y) as f32,
        );
        let aabb_size = Vec2::new(start_size.width as f32, start_size.height as f32);
        let side = infer_resize_anchor(hit.canvas_pos, aabb_pos, aabb_size);
        Ok(ThrottledDrag::SectionResize(SectionResizeInteraction::new(
            node_id.clone(),
            section_idx,
            side,
            start_offset,
            start_size,
            started_with_right,
        )))
    } else {
        let aabb_pos = Vec2::new(node.position.x as f32, node.position.y as f32);
        let aabb_size = Vec2::new(node.size.width as f32, node.size.height as f32);
        let side = infer_resize_anchor(hit.canvas_pos, aabb_pos, aabb_size);
        Ok(ThrottledDrag::NodeResize(NodeResizeInteraction::new(
            node_id.clone(),
            side,
            node.position,
            node.size,
            started_with_right,
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::moving_node::MovingNodeInteraction;
    use crate::application::app::throttled_interaction::node_resize::NodeResizeInteraction;
    use crate::application::app::throttled_interaction::ThrottledDrag;
    use baumhard::mindmap::model::{Position, Size};
    use baumhard::mindmap::tree_builder::ResizeHandleSide;
    use std::collections::HashSet;

    // Most dispatch arms touch the renderer (wgpu), which only the
    // GPU-backed tier of `TEST_CONVENTIONS.md §T8` stands up —
    // `app/tests_gpu.rs` drives `DeleteSelection`, `Undo` and `ZoomIn`
    // through the real arms there. Arms whose bodies factor cleanly
    // into a pure helper are tested here through that helper —
    // `route_pan_canvas` below is the first; the keybind tests in
    // `keybinds/tests.rs` exercise the resolver, not the dispatch
    // bodies.

    /// A right-started fast-resize, mid-gesture: the model write and
    /// the undo entry it owes happen only in
    /// `commit_on_release_core`.
    fn throttled_fast_resize_state() -> DragState {
        DragState::throttled(ThrottledDrag::NodeResize(NodeResizeInteraction::new(
            "n0".to_string(),
            ResizeHandleSide::SE,
            Position { x: 0.0, y: 0.0 },
            Size {
                width: 100.0,
                height: 50.0,
            },
            true,
        )))
    }

    /// A left-started move-node drag, mid-gesture.
    fn moving_node_drag() -> DragState {
        DragState::throttled(ThrottledDrag::MovingNode(MovingNodeInteraction::new(
            vec!["n0".to_string()],
            false,
            HashSet::new(),
        )))
    }

    /// A pending right press on node 1, pressed at `press` (canvas).
    fn pending_right_on_node_1(press: glam::Vec2) -> DragState {
        DragState::PendingRight {
            start_pos: (0.0, 0.0),
            start_canvas: press,
            hit_node: Some("1".into()),
            hit_section_idx: None,
        }
    }

    /// Node 1's canvas AABB in the canonical fixture: (min, max).
    fn node_1_corners(doc: &crate::application::document::MindMapDocument) -> (glam::Vec2, glam::Vec2) {
        let node = doc.mindmap.nodes.get("1").expect("fixture carries node 1");
        let min = glam::Vec2::new(node.position.x as f32, node.position.y as f32);
        let size = glam::Vec2::new(node.size.width as f32, node.size.height as f32);
        (min, min + size)
    }

    /// The side a fast-resize armed from `hit` anchors at.
    fn fast_resize_side(
        doc: &crate::application::document::MindMapDocument,
        hit: &DispatchHit,
    ) -> ResizeHandleSide {
        match fast_resize_drag(doc, hit).expect("a node hit held by a button starts a resize") {
            ThrottledDrag::NodeResize(i) => i.side,
            _ => panic!("a whole-node hit starts a node resize"),
        }
    }

    /// **The `RightDrag` cross hands the Action the press, not the
    /// cross** (#194): its hit carries the pending press's canvas
    /// position and body hit, and the right button as holder. Read
    /// back through `fast_resize_drag`, a press near the SE corner
    /// anchors SE.
    ///
    /// The control is the same node with the hit's `canvas_pos` moved
    /// to near the NW corner — where a drag that started SE could have
    /// crossed the threshold — which anchors NW. So the anchor follows
    /// `canvas_pos`, and the cross filling it with the press position is
    /// what decides the corner. Fails on a cross that built its hit from
    /// the cursor at the cross.
    #[test]
    fn test_a_fast_resize_anchors_at_the_press_not_the_threshold_cross() {
        let doc = crate::application::document::tests_common::load_test_doc();
        let (min, max) = node_1_corners(&doc);
        let near_se = max - glam::Vec2::splat(4.0);
        let near_nw = min + glam::Vec2::splat(4.0);

        let hit = pending_right_on_node_1(near_se)
            .right_drag_hit()
            .expect("a pending right press builds the cross's hit");
        assert_eq!(hit.canvas_pos, near_se, "the hit carries the press position");
        assert!(matches!(hit.click_hit, crate::application::app::ClickHit::Node(ref id, None) if id == "1"));
        assert_eq!(
            hit.held_by,
            Some(crate::application::platform::input::MouseButton::Right)
        );
        assert_eq!(fast_resize_side(&doc, &hit), ResizeHandleSide::SE);

        let crossed_elsewhere = DispatchHit {
            canvas_pos: near_nw,
            ..hit
        };
        assert_eq!(
            fast_resize_side(&doc, &crossed_elsewhere),
            ResizeHandleSide::NW,
            "the anchor follows canvas_pos, so the press position is what must fill it"
        );
    }

    /// Only a pending right press builds a `RightDrag` hit; every other
    /// state has no right press to describe.
    #[test]
    fn test_only_a_pending_right_press_builds_a_right_drag_hit() {
        assert!(pending_right_on_node_1(glam::Vec2::ZERO)
            .right_drag_hit()
            .is_some());
        for state in [
            DragState::None,
            DragState::Panning {
                owner: PanOwner::Toggled,
            },
            throttled_fast_resize_state(),
        ] {
            assert!(state.right_drag_hit().is_none());
        }
    }

    /// **A fast-resize is owned by the button that holds its gesture,
    /// and refused when none does** (#194). Once the cross takes the
    /// pending press down, the hit is the only witness to the owner.
    ///
    /// The hit's button is the drag's owner, whichever it is. The
    /// refusal row is IPC `act.action FastResizeStart x= y=`, whose
    /// synthetic pointer holds nothing: the same hit with an owner
    /// starts a resize (the two rows above), so the refusal is the
    /// owner's absence and nothing else.
    #[test]
    fn test_a_fast_resize_takes_its_owner_from_the_hit_and_refuses_none() {
        use crate::application::platform::input::MouseButton;
        let doc = crate::application::document::tests_common::load_test_doc();
        let (_, max) = node_1_corners(&doc);
        let hit = |held_by| DispatchHit {
            click_hit: crate::application::app::ClickHit::Node("1".into(), None),
            canvas_pos: max,
            held_by,
        };
        let owner =
            |held_by| fast_resize_drag(&doc, &hit(held_by)).map(|drag| drag.as_dyn().started_with_right());
        assert_eq!(owner(Some(MouseButton::Right)), Ok(true));
        assert_eq!(owner(Some(MouseButton::Left)), Ok(false));
        assert_eq!(
            owner(None),
            Err(FastResizeRefusal::NO_HOLDER),
            "no button holds an IPC pointer's gesture, and the refusal says so by name"
        );
        assert_eq!(
            fast_resize_drag(
                &doc,
                &DispatchHit {
                    click_hit: crate::application::app::ClickHit::Empty,
                    ..hit(Some(MouseButton::Right))
                }
            )
            .map(|_| ()),
            Err(FastResizeRefusal::NOT_ON_A_NODE),
            "a press on empty canvas has no target to resize"
        );
    }

    /// **A `pan_canvas` dispatch mid-drag must not destroy the drag.**
    /// The middle-button route was guarded at the route in #37 item 5;
    /// this is the same silent loss reached through the *Action*,
    /// which every keyboard binding and every macro tier can name.
    ///
    /// Fails on the pre-fix arm, which wrote `DragState::Panning`
    /// unconditionally — i.e. on `fn route_pan_canvas(_) ->
    /// PanCanvasRoute { PanCanvasRoute::Arm }`.
    #[test]
    fn test_pan_canvas_mid_drag_leaves_the_throttled_drag_intact() {
        assert_eq!(
            route_pan_canvas(&throttled_fast_resize_state()),
            PanCanvasRoute::Refuse,
            "a keyboard-bound pan must not replace a right-started fast-resize: \
             its release-commit has not run, so the model write and the undo \
             entry go with it"
        );
        assert_eq!(
            route_pan_canvas(&moving_node_drag()),
            PanCanvasRoute::Refuse,
            "nor any other throttled drag"
        );
        assert_eq!(
            route_pan_canvas(&DragState::PendingRight {
                start_pos: (0.0, 0.0),
                start_canvas: glam::Vec2::ZERO,
                hit_node: None,
                hit_section_idx: None,
            }),
            PanCanvasRoute::Refuse,
            "nor a right press whose RightClick / FastResizeStart has not fired"
        );
    }

    /// The other half of the same guard — without these rows the fix
    /// could be "answer `Refuse` always", which would leave the canvas
    /// unable to pan at all.
    ///
    /// `Pending` is the load-bearing row: the `LeftDrag` threshold
    /// cross dispatches `PanCanvas` *from* `Pending`, so a guard as
    /// broad as the right-button press's `!matches!(.., None)` would
    /// break the default left-drag pan.
    #[test]
    fn test_pan_canvas_still_arms_from_every_state_that_owes_the_model_nothing() {
        for drag in [
            DragState::None,
            DragState::SelectingRect {
                start_canvas: glam::Vec2::ZERO,
                current_canvas: glam::Vec2::new(10.0, 10.0),
            },
            DragState::Pending(Box::new(crate::application::app::PendingPress {
                start_pos: (0.0, 0.0),
                hit_node: None,
                hit_section_idx: None,
                hit_edge_handle: None,
                hit_portal_label: None,
                hit_edge_label: None,
                hit_section_resize_handle: None,
                hit_node_resize_handle: None,
            })),
        ] {
            assert_eq!(
                route_pan_canvas(&drag),
                PanCanvasRoute::Arm,
                "PanCanvas must still arm from {:?}",
                std::mem::discriminant(&drag)
            );
        }
    }

    /// **A `pan_canvas` with no release behind it toggles** (#153). A
    /// macro step, a console line or IPC `act.action` has nothing
    /// that comes up, so the pan it arms is modal: the next
    /// `pan_canvas` that finds it ends it. The `Arm` row from `None`
    /// is the first half of the toggle; this is the second.
    ///
    /// Fails on the pre-#153 route, which answered `Arm` for any pan
    /// and so could never end one: a macro-armed pan ran until a
    /// button release happened to clear it.
    #[test]
    fn test_pan_canvas_ends_a_toggled_pan() {
        assert_eq!(
            route_pan_canvas(&DragState::Panning {
                owner: PanOwner::Toggled,
            }),
            PanCanvasRoute::ToggleOff
        );
    }

    /// **...but leaves a held pan to the input holding it.** The
    /// negative control for the toggle above: the same variant,
    /// owned by a held input, is *not* ended — its end is that
    /// input's release, which is still coming. The key row is the one
    /// that would bite: a key holding a pan auto-repeats, and each
    /// repeat re-dispatches `PanCanvas`, so a toggle here would flip
    /// the pan off and on for as long as the key is down.
    #[test]
    fn test_pan_canvas_leaves_a_held_pan_to_its_input() {
        for input in [
            HeldInput::Key("space".into()),
            HeldInput::Button(crate::application::platform::input::MouseButton::Middle),
            HeldInput::Button(crate::application::platform::input::MouseButton::Left),
        ] {
            assert_eq!(
                route_pan_canvas(&DragState::Panning {
                    owner: PanOwner::Held(input.clone()),
                }),
                PanCanvasRoute::Held,
                "a pan held by {input:?} ends on its release, not on a dispatch"
            );
        }
    }

    /// **A target-picker mode takes the pointer, so it has to take
    /// the rubber band with it.** `Reparent` / `Connect` swallow both
    /// halves of the left button and `handle_cursor_moved` returns
    /// before the drag-state ladder, so a band live at mode entry
    /// would sit frozen for the whole picker session — and its
    /// covered set would be painted by every hover rebuild the mode
    /// runs, because `highlight_entries_for` is what all of them
    /// read.
    ///
    /// Fails on the pre-fix shape, which is this function not
    /// existing: the mode entries wrote `interaction_mode` and
    /// rebuilt, and touched neither the drag state nor the set.
    #[test]
    fn test_entering_a_target_picker_mode_ends_a_live_rubber_band() {
        let mut doc = crate::application::document::tests_common::load_test_doc();
        doc.selection = SelectionState::Single("the-real-selection".to_string());
        doc.set_rect_select_preview(vec!["stale-preview".to_string()]);
        let mut document = Some(doc);
        let mut drag_state = DragState::SelectingRect {
            start_canvas: glam::Vec2::ZERO,
            current_canvas: glam::Vec2::new(10.0, 10.0),
        };

        assert!(take_rubber_band_for_target_picker(&mut drag_state, &mut document));

        assert!(
            matches!(drag_state, DragState::None),
            "the gesture the mode interrupted must not stay live"
        );
        let ids: Vec<&str> = crate::application::app::scene_rebuild::highlight_entries_for(
            document.as_ref().expect("fixture document"),
        )
        .into_iter()
        .map(|(id, _, _)| id)
        .collect();
        assert_eq!(
            ids,
            vec!["the-real-selection"],
            "the mode's own hover rebuilds must not paint the abandoned set"
        );
    }

    /// The complement: every other state is left alone. `Throttled`
    /// is the row that matters — it owes the model a write and an
    /// undo entry that only its own release performs, so ending it
    /// here would be the silent loss this issue opened with.
    #[test]
    fn test_entering_a_target_picker_mode_leaves_every_other_gesture_alone() {
        for mut drag in [
            throttled_fast_resize_state(),
            moving_node_drag(),
            DragState::Panning {
                owner: PanOwner::Toggled,
            },
            DragState::None,
        ] {
            let before = std::mem::discriminant(&drag);
            let mut document = None;
            assert!(!take_rubber_band_for_target_picker(&mut drag, &mut document));
            assert_eq!(
                std::mem::discriminant(&drag),
                before,
                "a picker mode must not end {:?}",
                before
            );
        }
    }

    #[test]
    fn test_quote_console_arg_wraps_plain_path_in_double_quotes() {
        assert_eq!(super::quote_console_arg("/tmp/x.json"), "\"/tmp/x.json\"");
    }

    #[test]
    fn test_quote_console_arg_handles_paths_with_spaces() {
        // Embedded whitespace is the whole reason quoting exists —
        // the tokenizer would otherwise split the path into multiple
        // positionals.
        assert_eq!(
            super::quote_console_arg("/tmp/some dir/x.json"),
            "\"/tmp/some dir/x.json\"",
        );
    }

    #[test]
    fn test_quote_console_arg_escapes_embedded_double_quotes() {
        // A literal `"` inside the path becomes `\"` so the
        // tokenizer doesn't terminate the quoted token early.
        assert_eq!(
            super::quote_console_arg(r#"/tmp/he said "hi"/x.json"#),
            r#""/tmp/he said \"hi\"/x.json""#,
        );
    }

    #[test]
    fn test_quote_console_arg_escapes_backslashes_for_windows_paths() {
        // Windows path: every `\` becomes `\\` so the tokenizer
        // doesn't consume the next char as part of an escape, and
        // a path ending in `\` doesn't unterminate the quote.
        assert_eq!(
            super::quote_console_arg(r"C:\Users\foo\map.json"),
            r#""C:\\Users\\foo\\map.json""#,
        );
    }

    #[test]
    fn test_quote_console_arg_handles_path_ending_in_backslash() {
        // Pre-fix this would produce `"C:\\foo\"` — an unterminated
        // quoted token. With the backslash escape it produces
        // `"C:\\foo\\"` which round-trips cleanly.
        assert_eq!(super::quote_console_arg(r"C:\foo\"), r#""C:\\foo\\""#);
    }
}
