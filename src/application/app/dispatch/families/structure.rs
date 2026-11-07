// SPDX-License-Identifier: MPL-2.0

//! Structure editing — create, delete, reparent, orphan, connect, and the
//! clipboard — rebuilt on the one writer by V1-C4 (#245).

use crate::application::keybinds::ActionKind;

use super::Family;

/// The Actions V1-C4 rebuilds.
pub(in crate::application::app) const FAMILY: Family = Family {
    kinds: &[
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
    ],
    owner: "V1-C4 (#245)",
};
