// SPDX-License-Identifier: MPL-2.0

//! `Action` — the abstract user-action vocabulary the event loop
//! dispatches on. New keyboard interactions go here, then add a
//! matching `KeybindConfig` field + default + binding-string list.

use serde::{Deserialize, Serialize};

use super::context::InputContext;

/// High-level user actions that can be bound to keys. Add a new variant
/// here when a new keyboard interaction is introduced, extend
/// `KeybindConfig` with a matching field + default, and handle the variant
/// in the event loop.
///
/// `Serialize` / `Deserialize` are derived so macros can carry actions
/// in their JSON payload — see `crate::application::macros::MacroStep`.
///
/// `#[non_exhaustive]` because new variants need to be reviewed
/// against the macro privilege gate
/// (`MacroSource::allows_action`) before they ship — the gate uses
/// a denylist of destructive Actions, and a new I/O / clipboard /
/// document-lifecycle variant added without updating the denylist
/// would silently bypass the gate from non-User macro tiers. The
/// `#[non_exhaustive]` is the structural signal that "review the
/// gate when extending."
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub enum Action {
    // ── Document-level (global) ──────────────────────────────────
    /// Undo the last action on the document.
    Undo,
    /// Enter reparent mode for the currently selected nodes.
    EnterReparentMode,
    /// Enter connect mode for the currently selected node.
    EnterConnectMode,
    /// Delete the current selection (currently: selected edge).
    DeleteSelection,
    /// Cancel the current mode (reparent / connect).
    CancelMode,
    /// Create a new unattached (orphan) node at the cursor position.
    CreateOrphanNode,
    /// Detach every currently selected node from its parent.
    OrphanSelection,
    /// Open the inline text editor on the currently selected single node
    /// with the node's existing text, cursor at end.
    EditSelection,
    /// Same as `EditSelection` but opens the editor with an empty buffer.
    EditSelectionClean,
    /// Open (or toggle) the CLI console.
    OpenConsole,
    /// Save the currently-open mindmap document to its bound file path.
    SaveDocument,
    /// Copy the focused component's clipboard representation.
    /// **WASM:** the underlying `clipboard::write_clipboard` is a
    /// log-and-no-op stub today (async-clipboard integration
    /// pending). The Action is classified `Compatible` because it
    /// doesn't crash, but the user-visible behaviour is "nothing
    /// happens." Tracked in `WASM_CONVERGENCE.md`.
    Copy,
    /// Paste the system clipboard's text content into the focused component.
    /// **WASM:** same stub posture as `Copy` — `read_clipboard`
    /// returns `None`.
    Paste,
    /// Cut: copy then clear the focused component's clipboard representation.
    /// **WASM:** same stub posture as `Copy`.
    Cut,

    // ── Console ──────────────────────────────────────────────────
    /// Close the console (two-tier: dismiss popup first, then close).
    ConsoleClose,
    /// Submit the current console input line for execution.
    ConsoleSubmit,
    /// Cycle tab completions.
    ConsoleTabComplete,
    /// Walk history backward / navigate completion popup upward.
    ConsoleHistoryUp,
    /// Walk history forward / navigate completion popup downward.
    ConsoleHistoryDown,
    /// Move cursor one grapheme left.
    ConsoleCursorLeft,
    /// Move cursor one grapheme right.
    ConsoleCursorRight,
    /// Move cursor to start of input.
    ConsoleCursorHome,
    /// Move cursor to end of input.
    ConsoleCursorEnd,
    /// Delete grapheme before cursor.
    ConsoleDeleteBack,
    /// Delete grapheme after cursor.
    ConsoleDeleteForward,
    /// Insert a literal space (winit delivers Space as Named, not Character).
    ConsoleInsertSpace,
    /// Clear the current input line (shell Ctrl+C muscle-memory).
    ConsoleClearLine,
    /// Jump cursor to start of line (shell Ctrl+A).
    ConsoleJumpStart,
    /// Jump cursor to end of line (shell Ctrl+E).
    ConsoleJumpEnd,
    /// Kill from cursor to start of line (shell Ctrl+U).
    ConsoleKillToStart,
    /// Kill the word before cursor (shell Ctrl+W).
    ConsoleKillWord,
    /// Scroll the scrollback window up by one line (Shift+Up).
    /// Plain Up still walks command history.
    ConsoleScrollUp,
    /// Scroll the scrollback window down by one line (Shift+Down).
    ConsoleScrollDown,
    /// Scroll the scrollback window up by one page
    /// (`MAX_CONSOLE_SCROLLBACK_ROWS` lines, PgUp).
    ConsoleScrollPageUp,
    /// Scroll the scrollback window down by one page (PgDn).
    ConsoleScrollPageDown,
    /// Pin the scrollback window at the bottom (newest line trailing,
    /// Shift+End).
    ConsoleScrollEnd,
    /// Pin the scrollback window at the top (oldest reachable line,
    /// Shift+Home).
    ConsoleScrollHome,

    // ── Color Picker ─────────────────────────────────────────────
    /// Cancel the color picker (contextual mode only; ignored in standalone).
    PickerCancel,
    /// Commit the current color (contextual: close; standalone: apply to selection).
    PickerCommit,
    /// Nudge hue −15°.
    PickerNudgeHueDown,
    /// Nudge hue +15°.
    PickerNudgeHueUp,
    /// Nudge saturation −0.1.
    PickerNudgeSatDown,
    /// Nudge saturation +0.1.
    PickerNudgeSatUp,
    /// Nudge value −0.1.
    PickerNudgeValDown,
    /// Nudge value +0.1.
    PickerNudgeValUp,

    // ── Label Editor ─────────────────────────────────────────────
    /// Cancel the inline label editor (discard changes).
    LabelEditCancel,
    /// Commit the inline label editor.
    LabelEditCommit,

    // ── Text Editor ──────────────────────────────────────────────
    /// Cancel the inline text editor (discard changes).
    TextEditCancel,

    // ─────────────────────────────────────────────────────────────
    // ── Mouse-gesture Actions (Document context) ────────────────
    // The mouse handler synthesizes the gesture's canonical key
    // name (see `bind::gesture_key_name`) and feeds it through the
    // same `action_for_context` lookup as keyboard input. Every
    // gesture below can be bound to a key, and every keyboard
    // binding below can be bound to a gesture.
    // ─────────────────────────────────────────────────────────────
    /// Default-bound to `DoubleClick`. Dispatches by what the click hit:
    /// `Node` → open text editor, `PortalMarker`/`PortalText` → pan to
    /// partner endpoint, `EdgeLabel` → open inline label editor,
    /// `Empty` → fire `CreateOrphanNodeAndEdit` if it's bound (default
    /// off — the gesture is intentionally unbound for empty-canvas
    /// double-clicks).
    DoubleClickActivate,
    /// Create an unattached node at the cursor and immediately open
    /// its text editor with an empty buffer. Sibling of `CreateOrphanNode`
    /// which only creates and selects (no editor). Default unbound.
    CreateOrphanNodeAndEdit,
    /// Continuous left-button drag on empty canvas → camera pan.
    /// Default-bound to `LeftDrag` and `MiddleClick`. The dispatcher
    /// enters `DragState::Panning` on press and exits on release.
    PanCanvas,
    /// Click outside an open editor → commit the editor's buffer.
    /// Mouse handler dispatches when the release lands outside the
    /// edited target's AABB.
    ///
    /// **Scaffolded — no dispatch arm yet.** The variant exists so
    /// `KeybindConfig` and macros can refer to it stably, but
    /// `dispatch.rs` does not currently match on it. Pressing a
    /// key bound to `CommitOrCloseEditor` falls through the
    /// dispatcher's catch-all (silent no-op + debug log). Wiring
    /// the body — folding the existing inline click-outside-commit
    /// paths in `event_mouse_click.rs:425-563` into a dispatch arm
    /// — is tracked as a follow-up in `TODO.md`.
    CommitOrCloseEditor,

    // ── Navigation / camera (Document context) ──────────────────
    /// Zoom the camera in by one step. Default-bound to `WheelUp`.
    ZoomIn,
    /// Zoom the camera out by one step. Default-bound to `WheelDown`.
    ZoomOut,
    /// Reset the camera zoom to 1.0.
    ZoomReset,
    /// Fit the entire mindmap tree to the viewport.
    ZoomFit,
    /// Pan the camera north (up) by one step.
    PanCameraNorth,
    /// Pan the camera south (down) by one step.
    PanCameraSouth,
    /// Pan the camera east (right) by one step.
    PanCameraEast,
    /// Pan the camera west (left) by one step.
    PanCameraWest,
    /// Center the camera on the centroid of the current selection.
    CenterOnSelection,
    /// Jump the camera + selection to the document's root node.
    JumpToRoot,

    // ── Selection (Document context) ────────────────────────────
    /// Select every node in the document.
    SelectAll,
    /// Clear the current selection.
    DeselectAll,
    /// Invert the current selection (selected ↔ unselected).
    InvertSelection,
    /// Select the parent of the currently-selected single node.
    SelectParent,
    /// Select the first child of the currently-selected single node.
    SelectChild,
    /// Select the next sibling of the currently-selected single node.
    SelectNextSibling,
    /// Select the previous sibling of the currently-selected single node.
    SelectPrevSibling,

    // ── TextEdit cursor primitives (TextEdit context) ───────────
    /// Move cursor one grapheme left.
    TextEditCursorLeft,
    /// Move cursor one grapheme right.
    TextEditCursorRight,
    /// Move cursor one visual line up.
    TextEditCursorUp,
    /// Move cursor one visual line down.
    TextEditCursorDown,
    /// Jump cursor to the start of the current line.
    TextEditCursorHome,
    /// Jump cursor to the end of the current line.
    TextEditCursorEnd,
    /// Move cursor one word left.
    TextEditWordLeft,
    /// Move cursor one word right.
    TextEditWordRight,
    /// Delete the grapheme before the cursor.
    TextEditDeleteBack,
    /// Delete the grapheme at / after the cursor.
    TextEditDeleteForward,
    /// Delete from cursor back to the start of the current word.
    TextEditDeleteWordBack,
    /// Delete from cursor forward through the current word.
    TextEditDeleteWordForward,
    /// Commit the editor's buffer to the model and close. Default unbound
    /// (Enter is literal in the multi-line node editor).
    TextEditCommit,

    // ── LabelEdit cursor primitives (LabelEdit context) ─────────
    /// Move cursor one grapheme left in the label/portal-text editor.
    LabelEditCursorLeft,
    /// Move cursor one grapheme right.
    LabelEditCursorRight,
    /// Jump cursor to the start of the buffer.
    LabelEditCursorHome,
    /// Jump cursor to the end of the buffer.
    LabelEditCursorEnd,
    /// Delete the grapheme before the cursor.
    LabelEditDeleteBack,
    /// Delete the grapheme at / after the cursor.
    LabelEditDeleteForward,

    // ── Console-verb Actions (Document context) ─────────────────
    /// Open the glyph-wheel color picker as a standalone palette.
    /// Mirrors `color picker on`.
    OpenColorPicker,
    /// Close the glyph-wheel color picker. Mirrors `color picker off`.
    CloseColorPicker,
    /// Open the inline label editor on the currently-selected edge.
    /// Mirrors `label edit`.
    LabelEditOnSelection,
    /// Toggle the FPS overlay on/off. Mirrors `fps on` ↔ `fps off`.
    ToggleFps,
    /// Toggle the FPS overlay's debug variant. Mirrors `fps debug` ↔
    /// `fps off`.
    ToggleFpsDebug,
    /// Replace the current document with a fresh blank one. Mirrors
    /// `new` (no path).
    NewDocument,
}

impl Action {
    /// The input context this action belongs to. Used by the
    /// contextual resolver to filter which actions are eligible
    /// in a given modal state.
    pub fn context(&self) -> InputContext {
        match self {
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
            | Action::ConsoleScrollHome => InputContext::Console,

            Action::PickerCancel
            | Action::PickerCommit
            | Action::PickerNudgeHueDown
            | Action::PickerNudgeHueUp
            | Action::PickerNudgeSatDown
            | Action::PickerNudgeSatUp
            | Action::PickerNudgeValDown
            | Action::PickerNudgeValUp => InputContext::ColorPicker,

            Action::LabelEditCancel
            | Action::LabelEditCommit
            | Action::LabelEditCursorLeft
            | Action::LabelEditCursorRight
            | Action::LabelEditCursorHome
            | Action::LabelEditCursorEnd
            | Action::LabelEditDeleteBack
            | Action::LabelEditDeleteForward => InputContext::LabelEdit,

            Action::TextEditCancel
            | Action::TextEditCursorLeft
            | Action::TextEditCursorRight
            | Action::TextEditCursorUp
            | Action::TextEditCursorDown
            | Action::TextEditCursorHome
            | Action::TextEditCursorEnd
            | Action::TextEditWordLeft
            | Action::TextEditWordRight
            | Action::TextEditDeleteBack
            | Action::TextEditDeleteForward
            | Action::TextEditDeleteWordBack
            | Action::TextEditDeleteWordForward
            | Action::TextEditCommit => InputContext::TextEdit,

            // All Document-context Actions — built-ins, mouse gestures,
            // navigation, selection, and console-verb Actions — fall
            // through to the catch-all.
            _ => InputContext::Document,
        }
    }

    /// Whether this action can fire on WASM today.
    ///
    /// **WASM convergence is a deferred work item** — see
    /// `WASM_CONVERGENCE.md` for the porting path. Until that work
    /// lands, the WASM target lacks several systems native has
    /// (`AppMode`, console, color picker, label/portal-text
    /// editors, `DragState`). Actions that depend on those systems
    /// classify as [`WasmCompatibility::NativeOnly`] and currently
    /// no-op on WASM (or are filtered out before dispatch entirely).
    ///
    /// **Forcing function for new variants.** This match is
    /// exhaustive on `Action`. Combined with `#[non_exhaustive]`,
    /// adding a new variant forces a developer to classify it
    /// here — the compile error is the structural reminder. When
    /// classifying, the rule is:
    ///
    /// - Reads/writes only `MindMapDocument`, `Renderer`, or
    ///   `text_edit_state` → [`WasmCompatibility::Compatible`]
    ///   (those exist on both targets).
    /// - Touches `console_state`, `color_picker_state`,
    ///   `label_edit_state`, `portal_text_edit_state`, `app_mode`,
    ///   `drag_state`, or filesystem → [`WasmCompatibility::NativeOnly`].
    /// - Mixed-branch Actions (where the dispatch arm reads /
    ///   writes different state per branch) classify as
    ///   `Compatible` ONLY when EVERY branch is Compatible. If
    ///   ANY branch reads or writes NativeOnly state — even a
    ///   branch unreachable from current callers — the variant
    ///   is `NativeOnly`. Future callers may reach previously-
    ///   unreachable branches; the classification is a
    ///   forward-compat contract, not a current-callers
    ///   snapshot.
    ///
    /// **`NativeOnly` does not preclude WASM-relevant side-effects.**
    /// A handler may still special-case a `NativeOnly` variant
    /// before the compatibility filter when the action has a
    /// meaningful WASM-side effect even without the native
    /// state — see `run_wasm.rs`'s `CancelMode` short-circuit,
    /// which clears `last_click` (relevant on both targets) even
    /// though the variant is `NativeOnly` because it primarily
    /// touches `app_mode`.
    pub fn wasm_compatibility(&self) -> WasmCompatibility {
        match self {
            // ── Document-only — works on both targets ─────────
            // Copy/Cut/Paste are Compatible because the
            // `crate::application::clipboard` module has cfg-gated
            // WASM stubs that log+no-op rather than panic. They
            // function as silent no-ops on WASM until the async
            // web-clipboard integration lands; `clipboard.rs`
            // documents the stub behaviour.
            Action::Undo
            | Action::DeleteSelection
            | Action::OrphanSelection
            | Action::CreateOrphanNode
            | Action::CreateOrphanNodeAndEdit
            | Action::Copy
            | Action::Cut
            | Action::Paste
            | Action::SelectAll
            | Action::DeselectAll
            | Action::InvertSelection
            | Action::SelectParent
            | Action::SelectChild
            | Action::SelectNextSibling
            | Action::SelectPrevSibling
            | Action::JumpToRoot
            | Action::CenterOnSelection => WasmCompatibility::Compatible,

            // ── Renderer-only — works on both targets ─────────
            Action::ZoomIn
            | Action::ZoomOut
            | Action::ZoomReset
            | Action::ZoomFit
            | Action::PanCameraNorth
            | Action::PanCameraSouth
            | Action::PanCameraEast
            | Action::PanCameraWest
            | Action::ToggleFps
            | Action::ToggleFpsDebug => WasmCompatibility::Compatible,

            // ── TextEdit cursor primitives — text_edit_state
            //    exists on both targets ──────────────────────
            Action::TextEditCancel
            | Action::TextEditCommit
            | Action::TextEditCursorLeft
            | Action::TextEditCursorRight
            | Action::TextEditCursorUp
            | Action::TextEditCursorDown
            | Action::TextEditCursorHome
            | Action::TextEditCursorEnd
            | Action::TextEditWordLeft
            | Action::TextEditWordRight
            | Action::TextEditDeleteBack
            | Action::TextEditDeleteForward
            | Action::TextEditDeleteWordBack
            | Action::TextEditDeleteWordForward => WasmCompatibility::Compatible,

            // ── Native-only: AppMode (Reparent / Connect) ────
            Action::EnterReparentMode
            | Action::EnterConnectMode
            | Action::CancelMode => WasmCompatibility::NativeOnly,

            // ── Mixed-branch Actions — NativeOnly per the
            //    "ANY NativeOnly branch ⇒ NativeOnly" rule. ──
            //
            // `EditSelection` / `EditSelectionClean` route through
            // `dispatch.rs:222-260` based on selection state:
            //   - `Single` → `open_text_edit` (Compatible)
            //   - `PortalLabel` / `PortalText` → `open_portal_text_edit`
            //     (touches `portal_text_edit_state`, NativeOnly)
            //   - `EdgeLabel` → `open_label_edit` (touches
            //     `label_edit_state`, NativeOnly)
            // Any user with a portal or edge-label selection
            // reaches the NativeOnly branches — not a future-only
            // concern. Classification flips when WASM gains the
            // inline portal-text + label editors.
            //
            // `DoubleClickActivate` (`dispatch.rs:319-432`) has
            // the same shape: Node / PortalMarker branches are
            // Compatible, but the EdgeLabel branch calls
            // `open_label_edit` (NativeOnly).
            //
            // `CommitOrCloseEditor` is Compatible-by-arm-body
            // (the variant has no dispatch arm yet — orphan,
            // see TODO.md). Classified `NativeOnly` defensively
            // until its arm lands; flipping is safe once the arm
            // body is verified Compatible.
            Action::DoubleClickActivate
            | Action::EditSelection
            | Action::EditSelectionClean
            | Action::CommitOrCloseEditor => WasmCompatibility::NativeOnly,

            // ── Native-only: console modal ────────────────────
            Action::OpenConsole
            | Action::ConsoleClose
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
            | Action::ConsoleScrollHome => WasmCompatibility::NativeOnly,

            // ── Native-only: glyph-wheel color picker modal ──
            Action::PickerCancel
            | Action::PickerCommit
            | Action::PickerNudgeHueDown
            | Action::PickerNudgeHueUp
            | Action::PickerNudgeSatDown
            | Action::PickerNudgeSatUp
            | Action::PickerNudgeValDown
            | Action::PickerNudgeValUp => WasmCompatibility::NativeOnly,

            // ── Native-only: inline label / portal-text editors
            //    These modals exist only on native. The shared
            //    `text_edit_state` (node text editor) is on both. ─
            Action::LabelEditCancel
            | Action::LabelEditCommit
            | Action::LabelEditCursorLeft
            | Action::LabelEditCursorRight
            | Action::LabelEditCursorHome
            | Action::LabelEditCursorEnd
            | Action::LabelEditDeleteBack
            | Action::LabelEditDeleteForward
            | Action::LabelEditOnSelection => WasmCompatibility::NativeOnly,

            // ── Native-only: console-verb Actions that open
            //    modals that don't exist on WASM. ──────────────
            Action::OpenColorPicker
            | Action::CloseColorPicker => WasmCompatibility::NativeOnly,

            // ── Native-only: filesystem / drag state ─────────
            Action::SaveDocument => WasmCompatibility::NativeOnly,
            Action::PanCanvas => WasmCompatibility::NativeOnly,
            Action::NewDocument => WasmCompatibility::NativeOnly,
        }
    }
}

/// Cross-platform compatibility classification for an [`Action`].
///
/// The Mandala application targets both native (winit + desktop wgpu)
/// and WASM (browser canvas + web wgpu). Today native has every
/// modal and state machine; WASM has a curated subset (document,
/// renderer, node text editor, mouse, keyboard). The
/// [`Action::wasm_compatibility`] method classifies each variant
/// based on which target's machinery it requires.
///
/// **Where this matters.** `run_wasm.rs`'s keyboard / mouse paths
/// filter on this so a key bound to e.g. `Action::OpenConsole`
/// silently no-ops in the browser instead of triggering a panic
/// at the dispatch site. As WASM gains its own version of the
/// missing modals (see `WASM_CONVERGENCE.md`), variants migrate
/// from `NativeOnly` to `Compatible` one at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WasmCompatibility {
    /// Action works identically on native and WASM today. Its
    /// dispatch arm reads / writes only state both targets have:
    /// `MindMapDocument`, `Renderer`, `TextEditState`, mouse
    /// gestures, the macro registry. Safe to fire from a WASM
    /// `dispatch_action_for_wasm` once that path is built.
    Compatible,
    /// Action requires a native-only system not yet ported to
    /// WASM (`AppMode`, console, color picker, inline label /
    /// portal-text editors, `DragState`, filesystem `save`).
    /// Currently a no-op on WASM; the convergence path is to
    /// either port the underlying system or surface a WASM-
    /// specific equivalent and flip the classification to
    /// `Compatible`. Tracked in `WASM_CONVERGENCE.md`.
    NativeOnly,
}
