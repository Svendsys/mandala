// SPDX-License-Identifier: MPL-2.0

//! The Actions whose bodies edited the old model, by the issue that
//! rebuilds each family on the one writer (design §8; V1-C3 #244, its
//! "Ownership of every removed editing family").
//!
//! One file per family, so the fan-out issues fill disjoint files:
//!
//! | File | Family | Owner |
//! |---|---|---|
//! | [`structure`] | create, delete, reparent, orphan, connect, clipboard | V1-C4 (#245), rebuilt |
//! | [`field_nodes`] | node, section, frame, theme and defaults fields; selection-wide style | V1-C5 (#246), rebuilt |
//! | [`field_paths`] | connector, link, label and portal fields | V1-C6 (#247), rebuilt |
//! | `transactions` (native) | the resize mode, the fast resize, the color picker (the drags are not Actions: `app::live`) | V1-C7 (#248), rebuilt |
//! | [`text`] | text editing on spans, node editing | V1-C8 (#249), rebuilt |
//! | [`edits`] | the edit funnel: keys bound to edits, the macro `Edit` step, the console | V1-D1 (#250), rebuilt |
//! | [`verbs`] | map verbs: click, hover and link triggers, the console, IPC, the macro `Verb` step | V1-D3 (#252), landed |
//!
//! Presentations (V1-D2, #251) have no Action and no dispatch entry of
//! their own: the console's `present` verb starts and stops them
//! (`console_input::presentations`), and so do the map's verbs
//! ([`verbs`]). What they
//! have here is [`the_map`](super::the_map): every Action that reads the
//! map runs with them set aside.
//!
//! Every family has landed. [`structure`], [`field_nodes`],
//! [`field_paths`] and [`text`] are asked first on both targets
//! ([`structure::dispatch`], [`field_nodes::dispatch`],
//! [`field_paths::dispatch`], [`text::dispatch`]); `transactions`, whose
//! Actions are `NativeOnly`, and the single-line half of [`text`] are
//! asked by the native funnel after the cross-platform stage
//! (`transactions::dispatch`, `text::dispatch_native`). The cut-over's
//! one temporary scaffolding — a family that answered its Actions
//! `Unhandled` until its owner rebuilt them — is gone with the last
//! owner, V1-C8 (#249).

pub(in crate::application::app) mod edits;
pub(in crate::application::app) mod field_nodes;
pub(in crate::application::app) mod field_paths;
#[cfg(test)]
mod projected;
pub(in crate::application::app) mod structure;
pub(in crate::application::app) mod text;
pub(in crate::application::app) mod transactions;
pub(in crate::application::app) mod verbs;

#[cfg(test)]
mod tests {
    use crate::application::keybinds::{ActionKind, InputContext};
    use strum::IntoEnumIterator;

    /// Every Action the app handles in the canvas's own contexts, by the
    /// arm or the family that runs it. Spelled out, so an Action added to
    /// the vocabulary is a change this list has to agree with: it is
    /// placed, never left to fall through. The console's own keys
    /// (`InputContext::Console`) are the console modal's, and are not
    /// listed.
    const HANDLED_BY_THE_APP: &[ActionKind] = &[
        ActionKind::Undo,
        ActionKind::Redo,
        ActionKind::ExitMode,
        ActionKind::OpenConsole,
        ActionKind::SaveDocument,
        ActionKind::Quit,
        ActionKind::PanCanvas,
        ActionKind::ZoomIn,
        ActionKind::ZoomOut,
        ActionKind::ZoomReset,
        ActionKind::ZoomFit,
        ActionKind::PanCameraNorth,
        ActionKind::PanCameraSouth,
        ActionKind::PanCameraEast,
        ActionKind::PanCameraWest,
        ActionKind::CenterOnSelection,
        ActionKind::JumpToRoot,
        ActionKind::SelectAll,
        ActionKind::DeselectAll,
        ActionKind::InvertSelection,
        ActionKind::SelectParent,
        ActionKind::SelectChild,
        ActionKind::SelectNextSibling,
        ActionKind::SelectPrevSibling,
        ActionKind::ToggleFps,
        ActionKind::ToggleFpsDebug,
        ActionKind::NewDocument,
        ActionKind::OpenDocument,
        ActionKind::SaveDocumentAs,
        ActionKind::NewDocumentAt,
        ActionKind::RescanFonts,
        // The structure family, rebuilt (`structure::dispatch`).
        ActionKind::DeleteSelection,
        ActionKind::CreateOrphanNode,
        ActionKind::OrphanSelection,
        ActionKind::EnterReparentMode,
        ActionKind::EnterConnectMode,
        ActionKind::ReparentToTarget,
        ActionKind::ConnectToTarget,
        ActionKind::Copy,
        ActionKind::Cut,
        ActionKind::Paste,
        // The field family, rebuilt (`field_nodes::dispatch`).
        ActionKind::SetColor,
        ActionKind::SetFont,
        ActionKind::SetFontFamily,
        ActionKind::ReplaceFontFamily,
        ActionKind::SetZoom,
        ActionKind::ClearZoom,
        ActionKind::SetFrameField,
        ActionKind::CycleFrameContent,
        ActionKind::ToggleFrameVisible,
        ActionKind::SetFramePreview,
        ActionKind::CommitFramePreview,
        ActionKind::CancelFramePreview,
        ActionKind::SetSectionOffsetDelta,
        ActionKind::SetSectionOffsetAbs,
        ActionKind::SetSectionSizeAbs,
        ActionKind::SetSectionSizeFillParent,
        ActionKind::SetSectionText,
        ActionKind::AddSection,
        ActionKind::DeleteSection,
        ActionKind::SplitSection,
        // The path family, rebuilt (`field_paths::dispatch`).
        ActionKind::SetPathAnchor,
        ActionKind::SetPathContent,
        ActionKind::SetPathCap,
        ActionKind::SetLinkKind,
        ActionKind::ResetPath,
        ActionKind::SetLabelText,
        ActionKind::SetLabelPosition,
        ActionKind::SetPathSpacing,
        // The transactions family, rebuilt (`transactions::dispatch`).
        ActionKind::EnterResizeMode,
        ActionKind::FastResizeStart,
        ActionKind::OpenColorPicker,
        ActionKind::CloseColorPicker,
        ActionKind::PickerCancel,
        ActionKind::PickerCommit,
        ActionKind::PickerNudgeHueDown,
        ActionKind::PickerNudgeHueUp,
        ActionKind::PickerNudgeSatDown,
        ActionKind::PickerNudgeSatUp,
        ActionKind::PickerNudgeValDown,
        ActionKind::PickerNudgeValUp,
        // The text family, rebuilt (`text::dispatch`,
        // `text::dispatch_native`).
        ActionKind::EditSelection,
        ActionKind::EditSelectionClean,
        ActionKind::EnterNodeEdit,
        ActionKind::EnterNodeEditClean,
        ActionKind::EnterSectionEdit,
        ActionKind::DoubleClickActivate,
        ActionKind::LabelEditOnSelection,
        ActionKind::CreateOrphanNodeAndEdit,
        ActionKind::TextEditCancel,
        ActionKind::TextEditCommit,
        ActionKind::TextEditCursorLeft,
        ActionKind::TextEditCursorRight,
        ActionKind::TextEditCursorUp,
        ActionKind::TextEditCursorDown,
        ActionKind::TextEditCursorHome,
        ActionKind::TextEditCursorEnd,
        ActionKind::TextEditCursorLeftSelect,
        ActionKind::TextEditCursorRightSelect,
        ActionKind::TextEditCursorUpSelect,
        ActionKind::TextEditCursorDownSelect,
        ActionKind::TextEditCursorHomeSelect,
        ActionKind::TextEditCursorEndSelect,
        ActionKind::TextEditWordLeft,
        ActionKind::TextEditWordRight,
        ActionKind::TextEditDeleteBack,
        ActionKind::TextEditDeleteForward,
        ActionKind::TextEditDeleteWordBack,
        ActionKind::TextEditDeleteWordForward,
        ActionKind::TextEditInsertSpace,
        ActionKind::LabelEditCancel,
        ActionKind::LabelEditCommit,
        ActionKind::LabelEditCursorLeft,
        ActionKind::LabelEditCursorRight,
        ActionKind::LabelEditCursorHome,
        ActionKind::LabelEditCursorEnd,
        ActionKind::LabelEditDeleteBack,
        ActionKind::LabelEditDeleteForward,
        ActionKind::LabelEditInsertSpace,
    ];

    /// **Every Action has a body**: each one is the app's — an arm or a
    /// family's — or the console modal's. Since V1-C8 (#249) no family
    /// answers an Action `Unhandled` to stand in for one an owner has not
    /// rebuilt.
    ///
    /// Control: the list is not every kind, so a test that passed it the
    /// whole vocabulary would not be this one — the console's keys are
    /// left out of it.
    #[test]
    fn test_every_action_is_the_apps_or_the_consoles() {
        for kind in ActionKind::iter() {
            let consoles = kind.context() == InputContext::Console;
            assert!(
                HANDLED_BY_THE_APP.contains(&kind) != consoles,
                "{kind:?} must be the app's or the console modal's, and only one of them"
            );
        }
        assert!(
            ActionKind::iter().any(|kind| kind.context() == InputContext::Console),
            "control: the console's keys are left out of the list"
        );
    }
}
