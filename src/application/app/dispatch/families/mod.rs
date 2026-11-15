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
//! | [`field_paths`] | connector, link, label and portal fields | V1-C6 (#247) |
//! | [`transactions`] | drags, resizes, the color picker, handle chrome | V1-C7 (#248) |
//! | [`text`] | text editing on spans, node editing | V1-C8 (#249) |
//! | [`edits`] | the edit funnel: keys bound to edits, the macro `Edit` step, the console | V1-D1 (#250), rebuilt |
//! | [`verbs`] | node click triggers, the macro verb step | V1-D3 (#252) |
//!
//! Presentations (V1-D2, #251) have no Action and no dispatch entry of
//! their own: verbs start them.
//!
//! [`structure`] and [`field_nodes`] have landed: their Actions run
//! ([`structure::dispatch`], [`field_nodes::dispatch`]), asked first on
//! both targets. Until its owner lands, each other family
//! answers its Actions
//! [`DispatchOutcome::Unhandled`] with a `debug!` naming the owner
//! ([`unhandled`]) — the only temporary scaffolding the cut-over allows,
//! which V1-E3 (#255) audits. The Action variants stay, so a user's
//! keybinds and macros naming them still load.

pub(in crate::application::app) mod edits;
pub(in crate::application::app) mod field_nodes;
pub(in crate::application::app) mod field_paths;
pub(in crate::application::app) mod structure;
pub(in crate::application::app) mod text;
pub(in crate::application::app) mod transactions;
pub(in crate::application::app) mod verbs;

use crate::application::keybinds::{Action, ActionKind};

use super::cross_dispatch::DispatchOutcome;

/// A family of Actions an owner issue rebuilds: the kinds it owns and
/// the owner it names.
pub(in crate::application::app) struct Family {
    /// The Actions it owns.
    pub(in crate::application::app) kinds: &'static [ActionKind],
    /// The issue that rebuilds them, as a log line names it.
    pub(in crate::application::app) owner: &'static str,
}

impl Family {
    /// The family's answer to `action`: [`unhandled`] when it owns it,
    /// `None` when it does not. O(the family's kinds).
    pub(in crate::application::app) fn dispatch(&self, action: &Action) -> Option<DispatchOutcome> {
        let kind = ActionKind::from(action);
        self.kinds.contains(&kind).then(|| unhandled(action, self.owner))
    }
}

/// Every family whose Actions still answer `Unhandled`, in the table's
/// order.
pub(in crate::application::app) const FAMILIES: [&Family; 3] =
    [&field_paths::FAMILY, &transactions::FAMILY, &text::FAMILY];

/// The answer a removed Action gets until `owner` rebuilds it: logged at
/// `debug!` with the owner named, and `Unhandled`, as an Action that does
/// not apply is answered.
pub(in crate::application::app) fn unhandled(action: &Action, owner: &str) -> DispatchOutcome {
    log::debug!("dispatch: {action:?} is rebuilt on the one writer by {owner}; unhandled until then");
    DispatchOutcome::Unhandled
}

/// The family that owns `action`, when a family does: its answer. Asked
/// before any other arm, on both targets, so no removed Action reaches a
/// body. `None` for every Action the app handles itself.
pub(in crate::application::app) fn dispatch(action: &Action) -> Option<DispatchOutcome> {
    FAMILIES.iter().find_map(|family| family.dispatch(action))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::keybinds::InputContext;
    use strum::IntoEnumIterator;

    /// The Actions the app still handles itself in the canvas's own
    /// context — everything no family owns. Spelled out, so moving an
    /// Action into or out of a family is a change this list has to
    /// agree with. The console's own keys (`InputContext::Console`) are
    /// the console modal's, and are not listed.
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
    ];

    /// Every Action is the app's, the console modal's, or owned by
    /// exactly one family.
    #[test]
    fn test_every_action_is_the_apps_or_exactly_one_familys() {
        for kind in ActionKind::iter() {
            let owners: Vec<&str> = FAMILIES
                .iter()
                .filter(|family| family.kinds.contains(&kind))
                .map(|family| family.owner)
                .collect();
            let apps = HANDLED_BY_THE_APP.contains(&kind) || kind.context() == InputContext::Console;
            match (apps, owners.len()) {
                (true, 0) | (false, 1) => {}
                (true, _) => panic!("{kind:?} is the app's, but {owners:?} claim it"),
                (false, _) => panic!("{kind:?} must be owned by exactly one family, not {owners:?}"),
            }
        }
    }

    /// A family's Action is answered `Unhandled`, and an Action of the
    /// app's — a rebuilt family's among them — is left to the app (the
    /// control: a family that claimed everything would answer `Undo`).
    #[test]
    fn test_a_familys_action_is_unhandled_and_the_apps_is_not_claimed() {
        assert_eq!(dispatch(&Action::DeleteSelection), None);
        assert_eq!(dispatch(&Action::CycleFrameContent), None);
        assert_eq!(
            dispatch(&Action::ResetPath("style".into())),
            Some(DispatchOutcome::Unhandled)
        );
        assert_eq!(
            dispatch(&Action::EnterResizeMode),
            Some(DispatchOutcome::Unhandled)
        );
        assert_eq!(
            dispatch(&Action::TextEditCommit),
            Some(DispatchOutcome::Unhandled)
        );
        assert_eq!(dispatch(&Action::Undo), None);
        assert_eq!(dispatch(&Action::SelectAll), None);
    }

    /// **No family's Action ships a binding** (V1-C3 removed each
    /// binding with its body): the default keybinds resolve no key, no
    /// gesture and no context to an Action a family owns, so nothing a
    /// user presses before a rebuild lands reaches an `Unhandled` arm by
    /// default. A user's own keybinds naming one still load.
    ///
    /// Control: the app's own Actions are bound, so a resolve that bound
    /// nothing at all fails here rather than passing the loop vacuously.
    #[test]
    fn test_no_familys_action_ships_a_default_binding() {
        let bound = crate::application::keybinds::KeybindConfig::default()
            .resolve()
            .bound_action_kinds();
        for family in FAMILIES {
            for kind in family.kinds {
                assert!(
                    !bound.contains(kind),
                    "{kind:?} ships a default binding, but {} has not rebuilt it",
                    family.owner
                );
            }
        }
        for kind in [ActionKind::Undo, ActionKind::Redo, ActionKind::OpenConsole] {
            assert!(bound.contains(&kind), "control: {kind:?} is bound by default");
        }
    }
}
