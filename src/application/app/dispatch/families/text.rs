// SPDX-License-Identifier: MPL-2.0

//! Text editing on spans — the section editor, the single-line editors
//! for labels, rings and path content, and node-edit mode — rebuilt on
//! the one writer by V1-C8 (#249).

use crate::application::keybinds::ActionKind;

use super::Family;

/// The Actions V1-C8 rebuilds.
pub(in crate::application::app) const FAMILY: Family = Family {
    kinds: &[
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
    ],
    owner: "V1-C8 (#249)",
};
