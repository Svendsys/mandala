// SPDX-License-Identifier: MPL-2.0

//! `ActionKind::wasm_compatibility` — the WASM porting status
//! classifier. Lives on [`super::ActionKind`] so the match doesn't
//! need to destructure payloads; [`super::Action::wasm_compatibility`]
//! is a thin delegate. See `WASM_CONVERGENCE.md` for the porting
//! path each `NativeOnly` arm follows on its way to `Compatible`.

use super::{ActionKind, WasmCompatibility};

impl ActionKind {
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
    pub fn wasm_compatibility(self) -> WasmCompatibility {
        match self {
            // ── Document-only — works on both targets ─────────
            // Copy/Cut/Paste are Compatible because the
            // `crate::application::clipboard` module has cfg-gated
            // WASM stubs that log+no-op rather than panic. They
            // function as silent no-ops on WASM until the async
            // web-clipboard integration lands; `clipboard.rs`
            // documents the stub behaviour.
            ActionKind::Undo
            | ActionKind::DeleteSelection
            | ActionKind::OrphanSelection
            | ActionKind::CreateOrphanNode
            | ActionKind::CreateOrphanNodeAndEdit
            | ActionKind::Copy
            | ActionKind::Cut
            | ActionKind::Paste
            | ActionKind::SelectAll
            | ActionKind::DeselectAll
            | ActionKind::InvertSelection
            | ActionKind::SelectParent
            | ActionKind::SelectChild
            | ActionKind::SelectNextSibling
            | ActionKind::SelectPrevSibling
            | ActionKind::JumpToRoot
            | ActionKind::CenterOnSelection
            // Parametric mutators that touch only `MindMapDocument`
            // setters — Compatible by classification rule.
            | ActionKind::SetEdgeAnchor
            | ActionKind::SetEdgeBodyGlyph
            | ActionKind::SetBorderField
            | ActionKind::SetEdgeCap
            | ActionKind::SetColor
            | ActionKind::SetEdgeType
            | ActionKind::SetEdgeDisplayMode
            | ActionKind::ResetEdge
            | ActionKind::SetFontFamily
            | ActionKind::SetFont
            | ActionKind::SetEdgeLabelText
            | ActionKind::SetEdgeLabelPosition
            | ActionKind::SetSpacing
            | ActionKind::SetZoom
            | ActionKind::ClearZoom => WasmCompatibility::Compatible,

            // ── Renderer-only — works on both targets ─────────
            ActionKind::ZoomIn
            | ActionKind::ZoomOut
            | ActionKind::ZoomReset
            | ActionKind::ZoomFit
            | ActionKind::PanCameraNorth
            | ActionKind::PanCameraSouth
            | ActionKind::PanCameraEast
            | ActionKind::PanCameraWest
            | ActionKind::ToggleFps
            | ActionKind::ToggleFpsDebug => WasmCompatibility::Compatible,

            // ── TextEdit cursor primitives — text_edit_state
            //    exists on both targets ──────────────────────
            ActionKind::TextEditCancel
            | ActionKind::TextEditCommit
            | ActionKind::TextEditCursorLeft
            | ActionKind::TextEditCursorRight
            | ActionKind::TextEditCursorUp
            | ActionKind::TextEditCursorDown
            | ActionKind::TextEditCursorHome
            | ActionKind::TextEditCursorEnd
            | ActionKind::TextEditWordLeft
            | ActionKind::TextEditWordRight
            | ActionKind::TextEditDeleteBack
            | ActionKind::TextEditDeleteForward
            | ActionKind::TextEditDeleteWordBack
            | ActionKind::TextEditDeleteWordForward => WasmCompatibility::Compatible,

            // ── Native-only: AppMode (Reparent / Connect) ────
            ActionKind::EnterReparentMode
            | ActionKind::EnterConnectMode
            | ActionKind::ReparentToTarget
            | ActionKind::ConnectToTarget
            | ActionKind::CancelMode => WasmCompatibility::NativeOnly,

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
            ActionKind::DoubleClickActivate
            | ActionKind::EditSelection
            | ActionKind::EditSelectionClean => WasmCompatibility::NativeOnly,

            // ── Native-only: console modal ────────────────────
            ActionKind::OpenConsole
            | ActionKind::ConsoleClose
            | ActionKind::ConsoleSubmit
            | ActionKind::ConsoleTabComplete
            | ActionKind::ConsoleHistoryUp
            | ActionKind::ConsoleHistoryDown
            | ActionKind::ConsoleCursorLeft
            | ActionKind::ConsoleCursorRight
            | ActionKind::ConsoleCursorHome
            | ActionKind::ConsoleCursorEnd
            | ActionKind::ConsoleDeleteBack
            | ActionKind::ConsoleDeleteForward
            | ActionKind::ConsoleInsertSpace
            | ActionKind::ConsoleClearLine
            | ActionKind::ConsoleJumpStart
            | ActionKind::ConsoleJumpEnd
            | ActionKind::ConsoleKillToStart
            | ActionKind::ConsoleKillWord
            | ActionKind::ConsoleScrollUp
            | ActionKind::ConsoleScrollDown
            | ActionKind::ConsoleScrollPageUp
            | ActionKind::ConsoleScrollPageDown
            | ActionKind::ConsoleScrollEnd
            | ActionKind::ConsoleScrollHome => WasmCompatibility::NativeOnly,

            // ── Native-only: glyph-wheel color picker modal ──
            ActionKind::PickerCancel
            | ActionKind::PickerCommit
            | ActionKind::PickerNudgeHueDown
            | ActionKind::PickerNudgeHueUp
            | ActionKind::PickerNudgeSatDown
            | ActionKind::PickerNudgeSatUp
            | ActionKind::PickerNudgeValDown
            | ActionKind::PickerNudgeValUp => WasmCompatibility::NativeOnly,

            // ── Native-only: inline label / portal-text editors
            //    These modals exist only on native. The shared
            //    `text_edit_state` (node text editor) is on both. ─
            ActionKind::LabelEditCancel
            | ActionKind::LabelEditCommit
            | ActionKind::LabelEditCursorLeft
            | ActionKind::LabelEditCursorRight
            | ActionKind::LabelEditCursorHome
            | ActionKind::LabelEditCursorEnd
            | ActionKind::LabelEditDeleteBack
            | ActionKind::LabelEditDeleteForward
            | ActionKind::LabelEditOnSelection => WasmCompatibility::NativeOnly,

            // ── Native-only: console-verb Actions that open
            //    modals that don't exist on WASM. ──────────────
            ActionKind::OpenColorPicker
            | ActionKind::CloseColorPicker => WasmCompatibility::NativeOnly,

            // ── Native-only: filesystem / drag state ─────────
            ActionKind::SaveDocument => WasmCompatibility::NativeOnly,
            ActionKind::PanCanvas => WasmCompatibility::NativeOnly,
            ActionKind::NewDocument => WasmCompatibility::NativeOnly,
            // Parametric filesystem variants — same NativeOnly
            // posture as their unit-variant siblings; the dispatch
            // arms are cfg-gated and the body uses
            // `execute_console_line` which on WASM is itself
            // NativeOnly.
            ActionKind::OpenDocument => WasmCompatibility::NativeOnly,
            ActionKind::SaveDocumentAs => WasmCompatibility::NativeOnly,
            ActionKind::NewDocumentAt => WasmCompatibility::NativeOnly,
        }
    }
}
