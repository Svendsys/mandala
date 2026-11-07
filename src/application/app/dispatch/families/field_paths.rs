// SPDX-License-Identifier: MPL-2.0

//! Field editing II — connectors, links, labels and portal ends —
//! rebuilt on the one writer by V1-C6 (#247), which also deletes
//! `SetEdgeType` (D22: a connector is always a line).

use crate::application::keybinds::ActionKind;

use super::Family;

/// The Actions V1-C6 rebuilds.
pub(in crate::application::app) const FAMILY: Family = Family {
    kinds: &[
        ActionKind::SetPathAnchor,
        ActionKind::SetPathContent,
        ActionKind::SetPathCap,
        ActionKind::SetEdgeType,
        ActionKind::SetLinkKind,
        ActionKind::ResetPath,
        ActionKind::SetLabelText,
        ActionKind::SetLabelPosition,
        ActionKind::SetPathSpacing,
    ],
    owner: "V1-C6 (#247)",
};
