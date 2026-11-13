// SPDX-License-Identifier: MPL-2.0

//! `Action` — the abstract user-action vocabulary the event loop
//! dispatches on. New keyboard interactions go here; the config side
//! is one row in the `keybind_surface!` table in
//! [`super::config`], and the build fails until you write it.

use serde::{Deserialize, Serialize};
use strum_macros::{EnumDiscriminants, EnumIter, EnumString, IntoStaticStr};

use super::context::InputContext;

// ── Action payload enums ─────────────────────────────────────────
//
// Typed discriminators for the parametric Action variants that fan
// across a small fixed set of axes. Promoting these into the variant
// payload (instead of one Action variant per axis) keeps a dispatch
// match exhaustive without a "fan-out missed inner-match" guard, and
// keeps one row per Action in `KeybindConfig`, `is_destructive`,
// `wasm_compatibility` and `context`. Each serializes as the kv key a
// keybind or macro spells it with.

/// Which color a [`Action::SetColor`] sets: the `text|border|ring` kv
/// key of the `color` console verb's grammar. The strum derives
/// generate the bidirectional kv-key conversion: `Text.into() ==
/// "text"` (via `IntoStaticStr`) and `"text".parse::<ColorAxis>() ==
/// Ok(Text)` (via `EnumString`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ColorAxis {
    /// The text: a node's, a section's, a label's; a path's, a cap's
    /// and a portal end's one color.
    Text,
    /// A node's frame, which its rings inherit; a path's, a cap's and a
    /// portal end's one color, as `text`.
    Border,
    /// Each ring of a node's frame, its own.
    Ring,
}

/// Which frame a [`Action::SetFramePreview`] previews: the selected
/// nodes' frames, or the map's default frame (`Defaults.frame`), which
/// every frame that sets nothing of its own inherits — `main`'s
/// `canvas-border`. A keybind spells it in kebab case. Sections have no
/// frame (D8), so `main`'s section targets have no counterpart.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr, EnumString)]
#[serde(rename_all = "kebab-case")]
#[strum(serialize_all = "kebab-case")]
pub enum FramePreviewTarget {
    /// The selected nodes' frames.
    Node,
    /// The map's default frame.
    Defaults,
}

/// Which text field a [`Action::SetFont`] sets: the `size|weight|italic`
/// kv key of the `font` console verb's grammar. The family is its own
/// Action, [`Action::SetFontFamily`], since its value is a name.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum FontSlot {
    /// The size in points.
    Size,
    /// `normal`, `bold` or a weight from 100 to 900.
    Weight,
    /// `true` or `false`.
    Italic,
}

/// Which zoom bound a [`Action::SetZoom`] targets: the `min|max` kv key
/// of the `zoom` console verb's grammar. Clearing both at once is its
/// own Action, [`Action::ClearZoom`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, IntoStaticStr, EnumString)]
#[serde(rename_all = "snake_case")]
#[strum(serialize_all = "snake_case")]
pub enum ZoomBound {
    Min,
    Max,
}

/// High-level user actions that can be bound to keys. Add a new
/// variant here when a new keyboard interaction is introduced, give
/// it a row in the `keybind_surface!` table (which generates its
/// `KeybindConfig` field, its default and its resolve row from that
/// one row — or records why it has no key surface), and handle the
/// variant in the event loop. Skipping the table row is a compile
/// error, not a silently dead binding: see [`super::surface`].
///
/// `Serialize` / `Deserialize` are derived so macros can carry actions
/// in their JSON payload — see `crate::application::macros::MacroStep`.
///
/// `PayloadFieldNames` publishes each payload variant's field names,
/// in the order written below, into `action_payload_fields`. The
/// `keybind_surface!` table restates those names to define the
/// positional `args` contract of a `keybinds.json` binding, and the
/// two lists are compared at compile time — so the field order in
/// this declaration is load-bearing, and reordering a variant's
/// fields without reordering its table row is `error[E0080]` rather
/// than a user's arguments arriving in the wrong slots.
///
/// `#[non_exhaustive]` because new variants need to be reviewed
/// against the macro privilege gate
/// (`SourceTier::allows_action`) before they ship — the gate uses
/// a denylist of destructive Actions, and a new I/O / clipboard /
/// document-lifecycle variant added without updating the denylist
/// would silently bypass the gate from non-User macro tiers. The
/// `#[non_exhaustive]` is the structural signal that "review the
/// gate when extending."
///
/// **The Actions that edit the map are owned by the issues that
/// rebuild them on the one writer** (`app::dispatch::families`: V1-C4
/// to V1-C8, D1 and D3); V1-C4's structure Actions have landed. Until
/// its owner lands, each other one answers
/// `unhandled` with a `debug!` naming the owner; the variants stay so a
/// user's keybinds and macros naming them still load. Their docs say
/// what each is for and name the owner.
///
/// Some variants carry payload (e.g. `String` paths, kv-shaped
/// `(field, value)` tuples) so `Copy` is impossible — payload-bearing
/// variants are cloned at lookup time in
/// [`super::resolved::ResolvedKeybinds::action_for_context`]. Each
/// keypress allocates one short string per `String`-payload variant
/// (two for `SetPathAnchor` / `SetPathCap` / `SetFrameField`). For
/// the typical interactive cadence (≤10 keypresses/sec) the cost is
/// inconsequential; for a synthetic load of macros firing thousands
/// of `Action`s/sec, switch the lookup to return `&Action` and clone
/// at the dispatch boundary, or wrap payload strings in `Arc<str>`.
#[derive(
    Debug,
    Clone,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    EnumDiscriminants,
    mandala_derive::ActionClassify,
    mandala_derive::PayloadFieldNames,
)]
#[strum_discriminants(
    name(ActionKind),
    derive(Hash, EnumIter),
    doc = "Variant-kind mirror of [`Action`] without payloads. Generated by \
           strum's `EnumDiscriminants`. Used by the classifier methods \
           ([`is_destructive`](Self::is_destructive), \
           [`context`](Self::context), \
           [`wasm_compatibility`](Self::wasm_compatibility)) so the \
           classification matches need not destructure payloads, and by \
           tests that need to iterate every variant via `ActionKind::iter()`. \
           `&Action` and `Action` both `into()` an `ActionKind` (auto-derived)."
)]
#[non_exhaustive]
pub enum Action {
    // ── Document-level (global) ──────────────────────────────────
    /// Undo the newest history entry (`Document::undo`).
    #[action(context = Document, wasm = Compatible)]
    Undo,
    /// Redo the history entry undone last (`Document::redo`).
    #[action(context = Document, wasm = Compatible)]
    Redo,
    /// Enter reparent mode for the currently selected nodes: the
    /// pointer's valid target is tinted, and the next click ends it
    /// ([`Self::ReparentToTarget`]). `unhandled` with no node selected.
    #[action(context = Document, wasm = NativeOnly)]
    EnterReparentMode,
    /// Enter connect mode for the currently selected nodes: the next
    /// click ends it ([`Self::ConnectToTarget`]). `unhandled` with no
    /// node selected.
    #[action(context = Document, wasm = NativeOnly)]
    EnterConnectMode,
    /// Confirm a reparent onto a target node, whose integer id the
    /// payload carries (`None`: an empty-canvas click, which promotes the
    /// sources to roots), and leave the mode. A target that is a source
    /// or below one is refused, on the status line. Destructive, so the
    /// macro privilege gate (`SourceTier::allows_action`) denies it to
    /// every tier but the user's.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    ReparentToTarget(Option<u32>),
    /// Confirm a connect onto a target node, whose integer id the
    /// payload carries, and leave the mode: each source gets a line link
    /// to it, unless it is the target or already has one (`None`: an
    /// empty-canvas click, which leaves connect mode with no link made).
    /// Destructive, as [`Self::ReparentToTarget`] is.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    ConnectToTarget(Option<u32>),
    /// Delete the current selection: nodes with their subtrees, sections,
    /// links and labels. A connector alone is not deleted, and the
    /// status line says so.
    #[action(context = Document, wasm = Compatible, destructive)]
    DeleteSelection,
    /// Leave the active interaction mode for `Default`, and dismiss a
    /// refusal's status line. Both targets.
    #[action(context = Document, wasm = Compatible)]
    ExitMode,
    /// Create a new unattached (orphan) node where a pointer dispatch
    /// landed, or at the viewport's center from a key, and select it.
    #[action(context = Document, wasm = Compatible, destructive)]
    CreateOrphanNode,
    /// Detach every currently selected node from its parent, keeping it
    /// where it is on the canvas.
    #[action(context = Document, wasm = Compatible, destructive)]
    OrphanSelection,
    /// Edit the selection's text with its existing text, the cursor at
    /// the end: a node's spans, or a label's or a ring's single line.
    /// Answers `unhandled` until V1-C8 (#249) rebuilds it.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    EditSelection,
    /// [`Self::EditSelection`], starting from an empty buffer.
    /// Answers `unhandled` until V1-C8 (#249) rebuilds it.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    EditSelectionClean,
    /// Enter node-edit mode on the selected node, where a section is
    /// picked to edit; a node of one section opens its editor at once.
    /// Destructive, since the editor it may open can clear text on the
    /// spot.
    /// Answers `unhandled` until V1-C8 (#249) rebuilds it.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    EnterNodeEdit,
    /// [`Self::EnterNodeEdit`], with the editor it may open starting
    /// from an empty buffer.
    /// Answers `unhandled` until V1-C8 (#249) rebuilds it.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    EnterNodeEditClean,
    /// Open the editor on the picked section while in node-edit mode;
    /// closing it returns to node-edit mode. In
    /// [`InputContext::NodeEdit`], so a key bound to it there does not
    /// shadow the same key at the document level.
    /// Answers `unhandled` until V1-C8 (#249) rebuilds it.
    #[action(context = NodeEdit, wasm = NativeOnly, destructive)]
    EnterSectionEdit,
    /// Open (or toggle) the CLI console.
    #[action(context = Document, wasm = NativeOnly)]
    OpenConsole,
    /// Save the currently-open mindmap document to its bound file path.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    SaveDocument,
    /// Quit the app, gracefully: the exit closing the window is, and the
    /// only one an app with no window has (`--headless`, where an agent
    /// sends it as `act.action Quit`). The IPC client hears `shutdown`, a
    /// recording in progress is finished, the artifacts dir and the
    /// socket are removed (`app::pump::shut_down`).
    ///
    /// **Unbound by default.** Quitting does not ask about unsaved
    /// changes, any more than closing the window does, so a key that
    /// does it is the user's to choose rather than one a stray press
    /// reaches. Destructive for the same reason: a map's or a plugin's
    /// macro cannot end the user's session.
    ///
    /// **WASM: NativeOnly** — a page is closed by its browser, which a
    /// page cannot do on the user's behalf.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    Quit,
    /// Copy the selection's clipboard representation. On the browser
    /// the OS clipboard is a stub (`clipboard::write_clipboard`;
    /// `CLAUDE.md` "Dual-target status"), and the app's own clipboard
    /// holds the copy (`application::clipboard`).
    #[action(context = Document, wasm = Compatible, destructive)]
    Copy,
    /// Paste the system clipboard's text into the selection. On the
    /// browser `clipboard::read_clipboard` returns `None`, and the app's
    /// own clipboard is pasted.
    #[action(context = Document, wasm = Compatible, destructive)]
    Paste,
    /// Cut: copy, then clear, the selection's clipboard representation,
    /// as one undo entry.
    #[action(context = Document, wasm = Compatible, destructive)]
    Cut,
    /// Enter resize mode on the selected node or section: its handles
    /// are drawn on the chrome, and dragging one resizes it in a
    /// transaction. `ExitMode` returns to `Default`. Not destructive:
    /// the mode change edits nothing, and the drag's commit is gated
    /// on its own. The touch route to a resize, as a long-press, which
    /// is why [`Self::FastResizeStart`] needs no touch binding.
    /// NativeOnly while the browser has no drag-state machine.
    /// Answers `unhandled` until V1-C7 (#248) rebuilds it.
    #[action(context = Document, wasm = NativeOnly)]
    EnterResizeMode,
    /// Start a corner-anchored resize from a held mouse button's drag:
    /// the drag's threshold cross dispatches it with a hit built from
    /// the press (the body hit, the canvas position and the button,
    /// #194), and the corner nearest the press is the one that moves.
    /// The gesture starts a destructive flow, so the macro privilege
    /// gate (`SourceTier::allows_action`) denies it to every tier but
    /// the user's. A mouse accelerator by design (#160): touch reaches
    /// the same resize through [`Self::EnterResizeMode`]. NativeOnly
    /// while the browser has no drag-state machine.
    /// Answers `unhandled` until V1-C7 (#248) rebuilds it.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    FastResizeStart,

    // ── Console ──────────────────────────────────────────────────
    /// Close the console (two-tier: dismiss popup first, then close).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleClose,
    /// Submit the current console input line for execution.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleSubmit,
    /// Cycle tab completions.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleTabComplete,
    /// Walk history backward / navigate completion popup upward.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleHistoryUp,
    /// Walk history forward / navigate completion popup downward.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleHistoryDown,
    /// Move cursor one grapheme left.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleCursorLeft,
    /// Move cursor one grapheme right.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleCursorRight,
    /// Move cursor to start of input.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleCursorHome,
    /// Move cursor to end of input.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleCursorEnd,
    /// Delete grapheme before cursor.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleDeleteBack,
    /// Delete grapheme after cursor.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleDeleteForward,
    /// Insert a literal space (winit delivers Space as Named, not Character).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleInsertSpace,
    /// Clear the current input line (shell Ctrl+C muscle-memory).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleClearLine,
    /// Jump cursor to start of line (shell Ctrl+A).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleJumpStart,
    /// Jump cursor to end of line (shell Ctrl+E).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleJumpEnd,
    /// Kill from cursor to start of line (shell Ctrl+U).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleKillToStart,
    /// Kill the word before cursor (shell Ctrl+W).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleKillWord,
    /// Scroll the scrollback window up by one line (Shift+Up).
    /// Plain Up still walks command history.
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollUp,
    /// Scroll the scrollback window down by one line (Shift+Down).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollDown,
    /// Scroll the scrollback window up by one page
    /// (`MAX_CONSOLE_SCROLLBACK_ROWS` lines, PgUp).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollPageUp,
    /// Scroll the scrollback window down by one page (PgDn).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollPageDown,
    /// Pin the scrollback window at the bottom (newest line trailing,
    /// Shift+End).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollEnd,
    /// Pin the scrollback window at the top (oldest reachable line,
    /// Shift+Home).
    #[action(context = Console, wasm = NativeOnly)]
    ConsoleScrollHome,

    // ── Color Picker ─────────────────────────────────────────────
    // Owned by V1-C7 (#248), with the picker's transactions; until it
    // lands each answers `unhandled`.
    /// Cancel the color picker (contextual mode only; ignored in standalone).
    /// Answers `unhandled` until V1-C7 (#248) rebuilds it.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerCancel,
    /// Commit the current color (contextual: close; standalone: apply to selection).
    /// Answers `unhandled` until V1-C7 (#248) rebuilds it.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerCommit,
    /// Nudge hue −15°.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeHueDown,
    /// Nudge hue +15°.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeHueUp,
    /// Nudge saturation −0.1.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeSatDown,
    /// Nudge saturation +0.1.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeSatUp,
    /// Nudge value −0.1.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeValDown,
    /// Nudge value +0.1.
    #[action(context = ColorPicker, wasm = NativeOnly)]
    PickerNudgeValUp,

    // ── Label Editor ─────────────────────────────────────────────
    // This section's Actions, and every `TextEdit*` and `LabelEdit*`
    // below, are owned by V1-C8 (#249), with text editing on spans;
    // until it lands each answers `unhandled`.
    /// Cancel the inline label editor (discard changes).
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCancel,
    /// Commit the inline label editor.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCommit,

    // ── Text Editor ──────────────────────────────────────────────
    /// Cancel the inline text editor (discard changes).
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCancel,

    // ─────────────────────────────────────────────────────────────
    // ── Mouse-gesture Actions (Document context) ────────────────
    // The mouse handler synthesizes the gesture's canonical key
    // name (see `bind::gesture_key_name`) and feeds it through the
    // same `action_for_context` lookup as keyboard input. Every
    // gesture below can be bound to a key, and every keyboard
    // binding below can be bound to a gesture.
    // ─────────────────────────────────────────────────────────────
    /// What a double-click does to what it hit: edit a node's text or a
    /// label's line, or follow a link to its other end. Default-bound
    /// to `DoubleClick`; both targets resolve the gesture through this
    /// table (`cross_dispatch::pointer`).
    /// Answers `unhandled` until V1-C8 (#249) rebuilds it.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    DoubleClickActivate,
    /// Create an unattached node at the cursor and immediately open
    /// its text editor with an empty buffer. Sibling of `CreateOrphanNode`
    /// which only creates and selects (no editor). Default unbound.
    /// Answers `unhandled` until V1-C8 (#249) rebuilds it.
    #[action(context = Document, wasm = Compatible, destructive)]
    CreateOrphanNodeAndEdit,
    /// Camera pan: while it runs, the canvas follows the pointer.
    /// Default-bound to `LeftDrag` and `MiddleClick`.
    ///
    /// A pan belongs to the input that armed it (#153). From a held
    /// input — a mouse gesture, or a key — it is momentary: it ends on
    /// that button's or that key's release, and no other release ends
    /// it. From an input with no release — a macro step, a console
    /// line, IPC `act.action`, a touch long-press — it is modal: it
    /// toggles, and the next `pan_canvas` ends it. Losing window focus
    /// ends either kind, and a left press re-arms over either.
    #[action(context = Document, wasm = NativeOnly)]
    PanCanvas,

    // ── Navigation / camera (Document context) ──────────────────
    /// Zoom the camera in by one step. Default-bound to `WheelUp`.
    #[action(context = Document, wasm = Compatible)]
    ZoomIn,
    /// Zoom the camera out by one step. Default-bound to `WheelDown`.
    #[action(context = Document, wasm = Compatible)]
    ZoomOut,
    /// Reset the camera zoom to 1.0.
    #[action(context = Document, wasm = Compatible)]
    ZoomReset,
    /// Fit the entire mindmap tree to the viewport.
    #[action(context = Document, wasm = Compatible)]
    ZoomFit,
    /// Pan the camera north (up) by one step.
    #[action(context = Document, wasm = Compatible)]
    PanCameraNorth,
    /// Pan the camera south (down) by one step.
    #[action(context = Document, wasm = Compatible)]
    PanCameraSouth,
    /// Pan the camera east (right) by one step.
    #[action(context = Document, wasm = Compatible)]
    PanCameraEast,
    /// Pan the camera west (left) by one step.
    #[action(context = Document, wasm = Compatible)]
    PanCameraWest,
    /// Center the camera on the centroid of the current selection.
    #[action(context = Document, wasm = Compatible)]
    CenterOnSelection,
    /// Jump the camera + selection to the document's root node.
    #[action(context = Document, wasm = Compatible)]
    JumpToRoot,

    // ── Selection (Document context) ────────────────────────────
    /// Select every node in the document.
    #[action(context = Document, wasm = Compatible)]
    SelectAll,
    /// Clear the current selection.
    #[action(context = Document, wasm = Compatible)]
    DeselectAll,
    /// Invert the current selection (selected ↔ unselected).
    #[action(context = Document, wasm = Compatible)]
    InvertSelection,
    /// Select the parent of the currently-selected single node.
    #[action(context = Document, wasm = Compatible)]
    SelectParent,
    /// Select the first child of the currently-selected single node.
    #[action(context = Document, wasm = Compatible)]
    SelectChild,
    /// Select the next sibling of the currently-selected single node.
    #[action(context = Document, wasm = Compatible)]
    SelectNextSibling,
    /// Select the previous sibling of the currently-selected single node.
    #[action(context = Document, wasm = Compatible)]
    SelectPrevSibling,

    // ── TextEdit cursor primitives (TextEdit context) ───────────
    /// Move cursor one grapheme left.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorLeft,
    /// Move cursor one grapheme right.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorRight,
    /// Move cursor one visual line up.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorUp,
    /// Move cursor one visual line down.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorDown,
    /// Jump cursor to the start of the current line.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorHome,
    /// Jump cursor to the end of the current line.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorEnd,

    // ── TextEdit shift-select cursor primitives (TextEdit
    // context). Same as their non-`Select` siblings, but they extend
    // a selection from where it was anchored.
    /// Extend selection one grapheme left.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorLeftSelect,
    /// Extend selection one grapheme right.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorRightSelect,
    /// Extend selection one visual line up.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorUpSelect,
    /// Extend selection one visual line down.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorDownSelect,
    /// Extend selection to the start of the current line.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorHomeSelect,
    /// Extend selection to the end of the current line.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCursorEndSelect,

    /// Move cursor one word left.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditWordLeft,
    /// Move cursor one word right.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditWordRight,
    /// Delete the grapheme before the cursor.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditDeleteBack,
    /// Delete the grapheme at / after the cursor.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditDeleteForward,
    /// Delete from cursor back to the start of the current word.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditDeleteWordBack,
    /// Delete from cursor forward through the current word.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditDeleteWordForward,
    /// Insert a space at the cursor. winit delivers the space bar as
    /// `Key::Named(NamedKey::Space)`, not as a `" "` character, so it
    /// is a binding like the console's `ConsoleInsertSpace` rather
    /// than a literal-character keystroke.
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditInsertSpace,
    /// Commit the editor's buffer to the document and close. Default
    /// unbound (Enter is literal in the multi-line node editor).
    #[action(context = TextEdit, wasm = Compatible)]
    TextEditCommit,

    // ── LabelEdit cursor primitives (LabelEdit context) ─────────
    /// Move cursor one grapheme left in the label/portal-text editor.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCursorLeft,
    /// Move cursor one grapheme right.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCursorRight,
    /// Jump cursor to the start of the buffer.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCursorHome,
    /// Jump cursor to the end of the buffer.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditCursorEnd,
    /// Delete the grapheme before the cursor.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditDeleteBack,
    /// Delete the grapheme at / after the cursor.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditDeleteForward,
    /// Insert a space at the cursor; the label editor's peer of
    /// `TextEditInsertSpace`.
    #[action(context = LabelEdit, wasm = NativeOnly)]
    LabelEditInsertSpace,

    // ── Console-verb Actions (Document context) ─────────────────
    /// Open the glyph-wheel color picker as a standalone palette.
    /// Answers `unhandled` until V1-C7 (#248) rebuilds it.
    #[action(context = Document, wasm = NativeOnly)]
    OpenColorPicker,
    /// Close the glyph-wheel color picker.
    /// Answers `unhandled` until V1-C7 (#248) rebuilds it.
    #[action(context = Document, wasm = NativeOnly)]
    CloseColorPicker,
    /// Open the inline label editor on the selected connector.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    LabelEditOnSelection,
    /// Toggle the FPS overlay on/off. Mirrors `fps on` ↔ `fps off`.
    #[action(context = Document, wasm = Compatible)]
    ToggleFps,
    /// Toggle the FPS overlay's debug variant. Mirrors `fps debug` ↔
    /// `fps off`.
    #[action(context = Document, wasm = Compatible)]
    ToggleFpsDebug,
    /// Replace the current document with a fresh blank one. Mirrors
    /// `new` (no path).
    #[action(context = Document, wasm = NativeOnly, destructive)]
    NewDocument,

    // ── Parametric field Actions (Document context) ─────────────
    // Each sets one field of the selection from a keybind or a macro,
    // without a console line. Free-form `String` payloads are parsed
    // when dispatched. V1-C5 (#246) rebuilt the ones for nodes,
    // sections, frames and style across the selection
    // (`app::dispatch::families::field_nodes`); V1-C6 (#247) owns the
    // ones for connectors, links and labels, which answer `unhandled`
    // until it lands.
    /// Set the selected connector's anchor sides, `from` and `to`:
    /// `auto|top|right|bottom|left`.
    /// Answers `unhandled` until V1-C6 (#247) rebuilds it.
    #[action(context = Document, wasm = Compatible)]
    SetPathAnchor { from: String, to: String },
    /// Set the selected connector's path content: the glyph its ring
    /// draws (`dot|dash|double|wave|chain`).
    /// Answers `unhandled` until V1-C6 (#247) rebuilds it.
    #[action(context = Document, wasm = Compatible)]
    SetPathContent(String),
    /// Set one field of the selected nodes' frames, named by `field`
    /// as the `frame` console verb names it (`start`, `direction`,
    /// `fit`, `gap`, `inset`). The name is a string checked when
    /// dispatched, not at bind time; #227 tracks typed per-field
    /// variants that would catch a misspelling at load.
    #[action(context = Document, wasm = Compatible)]
    SetFrameField { field: String, value: String },
    /// Make the first ring of each selected node's frame the next box
    /// of light `─ │ ┌ ┐ ┘ └`, heavy `━ ┃ ┏ ┓ ┛ ┗` and double
    /// `═ ║ ╔ ╗ ╝ ╚`, wrapping: its spans, `vertical` and `corners`
    /// together (#338). Its fit is not touched: on a `repeat` ring the
    /// keys read as a box; a ring in another mode keeps them but draws
    /// its spans alone. A node with no frame gets one with one light
    /// ring. No payload, so one key cycles it.
    #[action(context = Document, wasm = Compatible)]
    CycleFrameContent,
    /// Show or hide each selected node's frame, each node on its own; a
    /// node with no frame gets one with one light `─ │ ┌ ┐ ┘ └` ring.
    /// No payload, so one key toggles it.
    #[action(context = Document, wasm = Compatible)]
    ToggleFrameVisible,
    /// Preview one field of the frame `target_kind` names
    /// ([`FramePreviewTarget`]) without making it an edit: an open
    /// interactive transaction (D28) whose stage is the preview, which
    /// [`Self::CommitFramePreview`] makes one undo entry and
    /// [`Self::CancelFramePreview`] reverts. A second preview replaces
    /// the first.
    #[action(context = Document, wasm = Compatible)]
    SetFramePreview {
        target_kind: FramePreviewTarget,
        field: String,
        value: String,
    },
    /// Commit the open frame preview. Nothing when none is open.
    #[action(context = Document, wasm = Compatible)]
    CommitFramePreview,
    /// Revert the open frame preview. Nothing when none is open.
    /// Default-bound to nothing: `ExitMode` reverts it first.
    #[action(context = Document, wasm = Compatible)]
    CancelFramePreview,
    /// Set the selected connector's end caps, `from` and `to`:
    /// `arrow|circle|diamond|none`.
    /// Answers `unhandled` until V1-C6 (#247) rebuilds it.
    #[action(context = Document, wasm = Compatible)]
    SetPathCap { from: String, to: String },
    /// Set one color of every selected element, whatever its kind:
    /// `axis` picks the text, the frame or the rings ([`ColorAxis`]),
    /// and `value` is a color as a map spells one (`#rrggbb[aa]`,
    /// `var(name)`, `palette(field)`), or `reset` to inherit.
    #[action(context = Document, wasm = Compatible)]
    SetColor { axis: ColorAxis, value: String },
    /// Set the selected path's kind: a connector between parent and
    /// child, or a cross link.
    /// Answers `unhandled` until V1-C6 (#247) rebuilds it.
    #[action(context = Document, wasm = Compatible)]
    SetEdgeType(String),
    /// Set how the selected link is drawn: a line, or a portal at each
    /// end.
    /// Answers `unhandled` until V1-C6 (#247) rebuilds it.
    #[action(context = Document, wasm = Compatible)]
    SetLinkKind(String),
    /// Reset one part of the selected path to its default:
    /// `straight|curve|style|position`.
    /// Answers `unhandled` until V1-C6 (#247) rebuilds it.
    #[action(context = Document, wasm = Compatible)]
    ResetPath(String),
    /// Set the selection's font family by name: a family a map may
    /// name, as the loader reads one.
    #[action(context = Document, wasm = Compatible)]
    SetFontFamily(String),
    /// Set one text field of the selection: `slot` picks the size in
    /// points, the weight or the slant ([`FontSlot`]).
    #[action(context = Document, wasm = Compatible)]
    SetFont { slot: FontSlot, value: String },
    /// Set the selected path's label text; an empty payload clears it.
    /// Answers `unhandled` until V1-C6 (#247) rebuilds it.
    #[action(context = Document, wasm = Compatible)]
    SetLabelText(String),
    /// Set where along the selected path its label sits:
    /// `start|middle|end`.
    /// Answers `unhandled` until V1-C6 (#247) rebuilds it.
    #[action(context = Document, wasm = Compatible)]
    SetLabelPosition(String),
    /// Set the spacing of the selected path's glyphs:
    /// `tight|normal|wide`, or a number.
    /// Answers `unhandled` until V1-C6 (#247) rebuilds it.
    #[action(context = Document, wasm = Compatible)]
    SetPathSpacing(String),
    /// Set one bound of the zoom range the selected nodes and paths
    /// draw in: `bound` picks `min|max`, and `value` is a positive zoom
    /// or `unset`.
    #[action(context = Document, wasm = Compatible)]
    SetZoom { bound: ZoomBound, value: String },
    /// Clear both bounds of the zoom range the selection draws in.
    #[action(context = Document, wasm = Compatible)]
    ClearZoom,
    /// Move the selected section by `(dx, dy)` canvas units.
    //
    // Stringly-typed payload convention: f64 / usize fields on
    // `Action` variants ride as `String` because the enum
    // derives Hash + Eq + Serialize / Deserialize across every
    // variant, and f64 doesn't `Hash`. Dispatch arms parse
    // them. This applies to every `Set/Add/Split/...Section*`
    // variant and to any future numeric-payload Action.
    #[action(context = Document, wasm = Compatible)]
    SetSectionOffsetDelta { dx: String, dy: String },
    /// Set the selected section's size to `(w, h)`: its minimum (D5).
    #[action(context = Document, wasm = Compatible)]
    SetSectionSizeAbs { w: String, h: String },
    /// Let the selected section fit its content again, with no size of
    /// its own.
    #[action(context = Document, wasm = Compatible)]
    SetSectionSizeFillParent,
    /// Place the selected section at `(x, y)`: the absolute form of
    /// [`Self::SetSectionOffsetDelta`].
    #[action(context = Document, wasm = Compatible)]
    SetSectionOffsetAbs { x: String, y: String },
    /// Replace the selected section's spans with one span holding
    /// `text`, which inherits the section's style. Destructive.
    #[action(context = Document, wasm = Compatible, destructive)]
    SetSectionText { text: String },
    /// Add a section to the selected node, at index `at` (empty:
    /// appended), holding `text`. Destructive.
    #[action(context = Document, wasm = Compatible, destructive)]
    AddSection { at: String, text: String },
    /// Delete the selected sections, as `DeleteSelection` deletes a
    /// section. Destructive.
    #[action(context = Document, wasm = Compatible, destructive)]
    DeleteSection,
    /// Split the selected section at grapheme `at_grapheme`, which is
    /// required: a split at the end would only add an empty section.
    /// Named apart from [`Self::AddSection`]'s `at`, a section index.
    /// Destructive.
    #[action(context = Document, wasm = Compatible, destructive)]
    SplitSection { at_grapheme: String },
    /// Mirror `open <path>` — replace the current document with the
    /// one loaded from `path`. **NativeOnly** + **destructive**:
    /// touches the filesystem. Denylisted for non-User macro tiers
    /// — a hostile mindmap must not be able to load arbitrary
    /// content as the active document.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    OpenDocument(String),
    /// Mirror `save <path>` — write the current document to `path`
    /// and rebind. **NativeOnly** + **destructive**: writes to the
    /// filesystem; a hostile macro could overwrite arbitrary files.
    /// Denylisted for non-User macro tiers.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    SaveDocumentAs(String),
    /// Mirror `new <path>` — start a fresh document and bind it to
    /// `path` (writes a blank file there immediately).
    /// **NativeOnly** + **destructive**: writes to the filesystem.
    /// Denylisted for non-User macro tiers.
    #[action(context = Document, wasm = NativeOnly, destructive)]
    NewDocumentAt(String),
}

// `Action::is_destructive`, `::context`, `::wasm_compatibility` are
// generated by `mandala_derive::ActionClassify` (each forwards to
// `ActionKind::from(self).method()`); see the derive's emit body in
// `lib/mandala_derive/src/lib.rs`. The canonical classifier lives on
// the discriminant enum so its match doesn't need to destructure
// payloads; the delegates exist so callers reach for whichever shape
// is closer to hand.

/// Cross-platform compatibility classification for an [`Action`].
///
/// The Mandala application targets both native (winit + desktop wgpu)
/// and WASM (browser canvas + web wgpu). Today native has every
/// modal and state machine; WASM has a curated subset (document,
/// renderer, mouse, keyboard). The
/// [`Action::wasm_compatibility`] method classifies each variant
/// based on which target's machinery it requires.
///
/// **Where this matters.** `run_wasm/event_keyboard.rs` and
/// `run_wasm/event_mouse_click.rs` filter on this so a key
/// bound to e.g. `Action::OpenConsole` silently no-ops in the
/// browser instead of triggering a panic
/// at the dispatch site. As WASM gains its own version of the
/// missing modals (see `work_plans/WASM_CONVERGENCE.md`), variants migrate
/// from `NativeOnly` to `Compatible` one at a time.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum WasmCompatibility {
    /// Action works identically on native and WASM today. Its
    /// dispatch arm reads / writes only state both targets have:
    /// `MindMapDocument`, `Renderer`, mouse gestures, the macro
    /// registry. Safe to fire from the browser's `dispatch_compatible`
    /// path.
    Compatible,
    /// Action requires a native-only system not yet ported to
    /// WASM (console, color picker, inline label editors,
    /// `DragState`, filesystem `save`). Currently a no-op on WASM;
    /// the convergence path is to
    /// either port the underlying system or surface a WASM-
    /// specific equivalent and flip the classification to
    /// `Compatible`. Tracked in `work_plans/WASM_CONVERGENCE.md`.
    NativeOnly,
}
