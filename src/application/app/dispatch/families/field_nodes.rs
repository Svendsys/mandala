// SPDX-License-Identifier: MPL-2.0

//! Field editing I — nodes, sections, frames and rings, the theme, the
//! defaults, and style across the selection — rebuilt on the one writer
//! by V1-C5 (#246).

use crate::application::keybinds::ActionKind;

use super::Family;

/// The Actions V1-C5 rebuilds.
pub(in crate::application::app) const FAMILY: Family = Family {
    kinds: &[
        ActionKind::SetColor,
        ActionKind::SetFont,
        ActionKind::SetFontFamily,
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
    ],
    owner: "V1-C5 (#246)",
};
