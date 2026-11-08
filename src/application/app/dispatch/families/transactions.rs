// SPDX-License-Identifier: MPL-2.0

//! Live interaction on transactions — drags, resizes, the color picker
//! and the handle chrome — rebuilt on the one writer by V1-C7 (#248).

use crate::application::keybinds::ActionKind;

use super::Family;

/// The Actions V1-C7 rebuilds.
pub(in crate::application::app) const FAMILY: Family = Family {
    kinds: &[
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
    ],
    owner: "V1-C7 (#248)",
};
