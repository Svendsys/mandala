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
    Copy,
    /// Paste the system clipboard's text content into the focused component.
    Paste,
    /// Cut: copy then clear the focused component's clipboard representation.
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
}
