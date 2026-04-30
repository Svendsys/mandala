// SPDX-License-Identifier: MPL-2.0

//! `ActionKind::context` — maps each variant kind to the
//! [`InputContext`] modal where it's eligible. Lives on
//! [`super::ActionKind`] so the match doesn't need to destructure
//! payloads; [`super::Action::context`] is a thin delegate.

use super::ActionKind;
use crate::application::keybinds::context::InputContext;

impl ActionKind {
    /// The input context this action belongs to. Used by the
    /// contextual resolver to filter which actions are eligible
    /// in a given modal state.
    pub fn context(self) -> InputContext {
        match self {
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
            | ActionKind::ConsoleScrollHome => InputContext::Console,

            ActionKind::PickerCancel
            | ActionKind::PickerCommit
            | ActionKind::PickerNudgeHueDown
            | ActionKind::PickerNudgeHueUp
            | ActionKind::PickerNudgeSatDown
            | ActionKind::PickerNudgeSatUp
            | ActionKind::PickerNudgeValDown
            | ActionKind::PickerNudgeValUp => InputContext::ColorPicker,

            ActionKind::LabelEditCancel
            | ActionKind::LabelEditCommit
            | ActionKind::LabelEditCursorLeft
            | ActionKind::LabelEditCursorRight
            | ActionKind::LabelEditCursorHome
            | ActionKind::LabelEditCursorEnd
            | ActionKind::LabelEditDeleteBack
            | ActionKind::LabelEditDeleteForward => InputContext::LabelEdit,

            ActionKind::TextEditCancel
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
            | ActionKind::TextEditCommit => InputContext::TextEdit,

            // Document-context Actions — built-ins, mouse gestures,
            // navigation, selection, and console-verb Actions.
            // Spelled out exhaustively (no `_` catch-all) so that
            // adding a new variant to the `#[non_exhaustive]`
            // `Action` enum forces a compile error here, rather
            // than silently defaulting the new variant to Document
            // context. Forcing-function discipline matches what
            // `wasm_compatibility` and `is_destructive` use.
            ActionKind::Undo
            | ActionKind::EnterReparentMode
            | ActionKind::EnterConnectMode
            | ActionKind::ReparentToTarget
            | ActionKind::ConnectToTarget
            | ActionKind::DeleteSelection
            | ActionKind::CancelMode
            | ActionKind::CreateOrphanNode
            | ActionKind::OrphanSelection
            | ActionKind::EditSelection
            | ActionKind::EditSelectionClean
            | ActionKind::OpenConsole
            | ActionKind::SaveDocument
            | ActionKind::Copy
            | ActionKind::Paste
            | ActionKind::Cut
            | ActionKind::DoubleClickActivate
            | ActionKind::CreateOrphanNodeAndEdit
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
            | ActionKind::LabelEditOnSelection
            | ActionKind::ToggleFps
            | ActionKind::ToggleFpsDebug
            | ActionKind::NewDocument
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
            | ActionKind::ClearZoom
            | ActionKind::OpenDocument
            | ActionKind::SaveDocumentAs
            | ActionKind::NewDocumentAt => InputContext::Document,
        }
    }
}
