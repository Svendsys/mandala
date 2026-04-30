// SPDX-License-Identifier: MPL-2.0

//! `ActionKind::is_destructive` — the privilege-gate classifier
//! consulted by `MacroSource::allows_action` so non-User macro
//! tiers (App / Map / Inline — including hostile `.mindmap.json`
//! content) cannot fire destructive Actions. Lives on
//! [`super::ActionKind`] so the match doesn't need to destructure
//! payloads; [`super::Action::is_destructive`] is a thin delegate.

use super::ActionKind;

impl ActionKind {
    /// Whether this action mutates persistent state (filesystem,
    /// document model that bypasses the undo stack, clipboard) or
    /// reaches an editor modal that can mutate model state on
    /// commit. Consulted by `MacroSource::allows_action` so non-
    /// User macro tiers (App / Map / Inline — including hostile
    /// `.mindmap.json` content) cannot fire destructive Actions.
    ///
    /// **The match is exhaustive**, and `Action` is
    /// `#[non_exhaustive]`. Adding a new variant forces the
    /// compiler to surface this method — that's the structural
    /// reminder that every new Action gets a privilege-gate
    /// review. A bare denylist `matches!(action, ...)` defaults
    /// to "allowed" and silently widens the attack surface; this
    /// classifier defaults to "you must decide."
    ///
    /// Classification rule:
    /// - **Destructive** (returns `true`): touches filesystem,
    ///   reaches an editor modal that mutates model state on
    ///   commit, touches the clipboard, or replaces the document.
    /// - **Non-destructive** (returns `false`): pure navigation /
    ///   selection / view-state / zoom / undo / fold. Recoverable
    ///   via undo or has no document side-effect at all.
    pub fn is_destructive(self) -> bool {
        match self {
            // Filesystem / document lifecycle.
            ActionKind::SaveDocument
            | ActionKind::NewDocument
            | ActionKind::OpenDocument
            | ActionKind::SaveDocumentAs
            | ActionKind::NewDocumentAt
            // Direct destructive mutators.
            | ActionKind::DeleteSelection
            | ActionKind::OrphanSelection
            | ActionKind::CreateOrphanNode
            | ActionKind::CreateOrphanNodeAndEdit
            // Reparent/Connect target confirmation: tree topology
            // mutations (subtree move; cross-link edge create) with
            // hand-rolled `UndoAction` entries. The `EnterReparentMode`
            // / `EnterConnectMode` siblings stay non-destructive
            // (just app-mode toggles); the `*ToTarget` variants are
            // the actual mutators.
            | ActionKind::ReparentToTarget
            | ActionKind::ConnectToTarget
            // Clipboard surface (Copy is read-only on the
            // document side, but reads private content into the
            // shared OS buffer — a surveillance vector for
            // hostile macros, so still gated).
            | ActionKind::Copy
            | ActionKind::Cut
            | ActionKind::Paste
            // Mixed-branch Actions whose dispatch arms reach
            // inline editor opens (`open_label_edit` /
            // `open_portal_text_edit` / `open_text_edit`); a
            // hostile macro firing one of these while a
            // sensitive selection is active forces the user
            // into an editor that may overwrite content on
            // commit.
            | ActionKind::DoubleClickActivate
            | ActionKind::EditSelection
            | ActionKind::EditSelectionClean
            | ActionKind::LabelEditOnSelection => true,

            // Pure navigation / selection / view-state / undo —
            // either has no document side-effect or is
            // round-tripable via the undo stack.
            ActionKind::Undo
            | ActionKind::EnterReparentMode
            | ActionKind::EnterConnectMode
            | ActionKind::CancelMode
            | ActionKind::OpenConsole
            | ActionKind::PanCanvas
            | ActionKind::ZoomIn
            | ActionKind::ZoomOut
            | ActionKind::ZoomReset
            | ActionKind::ZoomFit
            | ActionKind::PanCameraNorth
            | ActionKind::PanCameraSouth
            | ActionKind::PanCameraEast
            | ActionKind::PanCameraWest
            | ActionKind::CenterOnSelection
            | ActionKind::JumpToRoot
            | ActionKind::SelectAll
            | ActionKind::DeselectAll
            | ActionKind::InvertSelection
            | ActionKind::SelectParent
            | ActionKind::SelectChild
            | ActionKind::SelectNextSibling
            | ActionKind::SelectPrevSibling
            | ActionKind::OpenColorPicker
            | ActionKind::CloseColorPicker
            | ActionKind::ToggleFps
            | ActionKind::ToggleFpsDebug
            // Parametric console-verb mutators are recoverable via
            // undo (same trust posture as the existing
            // configurable-* Actions). Filesystem-touching parametric
            // variants will land in a later commit and DO classify as
            // destructive.
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
            | ActionKind::ClearZoom => false,

            // Modal-context Actions (Console / Picker / TextEdit /
            // LabelEdit / DoubleClickActivate's `Empty`-hit branch
            // gating). These either don't mutate model state
            // (cursor / cancel / scroll / commit-to-tree-only) or
            // are gated by the modal handler being open in the
            // first place — a non-User macro firing them outside
            // their modal is a no-op. Treated as non-destructive
            // for gate purposes.
            ActionKind::ConsoleClose
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
            | ActionKind::ConsoleScrollHome
            | ActionKind::PickerCancel
            | ActionKind::PickerCommit
            | ActionKind::PickerNudgeHueDown
            | ActionKind::PickerNudgeHueUp
            | ActionKind::PickerNudgeSatDown
            | ActionKind::PickerNudgeSatUp
            | ActionKind::PickerNudgeValDown
            | ActionKind::PickerNudgeValUp
            | ActionKind::LabelEditCancel
            | ActionKind::LabelEditCommit
            | ActionKind::LabelEditCursorLeft
            | ActionKind::LabelEditCursorRight
            | ActionKind::LabelEditCursorHome
            | ActionKind::LabelEditCursorEnd
            | ActionKind::LabelEditDeleteBack
            | ActionKind::LabelEditDeleteForward
            | ActionKind::TextEditCancel
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
            | ActionKind::TextEditDeleteWordForward
            | ActionKind::TextEditCommit => false,
        }
    }
}
