// SPDX-License-Identifier: MPL-2.0

//! Unit tests for the [`crate::application::macros`] registry +
//! step-iteration privilege gate. Exercises:
//!
//! - `MacroRegistry` insert / get / replace semantics, and the
//!   shadow stack: a higher tier shadows a lower one without
//!   displacing it.
//! - `SourceTier::allows_action` / `allows_console_line` —
//!   the privilege gate the dispatcher reads before running
//!   each step. Includes the fail-closed contract: a rejected
//!   step aborts the rest of the macro.
//! - JSON serde round-trips for `MacroStep` (the load-bearing
//!   on-disk shape `~/.config/mandala/macros.json` uses).

#![cfg(test)]

use super::*;

#[test]
fn macro_registry_insert_and_get() {
    let mut reg = MacroRegistry::new();
    let m = Macro {
        id: "test".into(),
        name: "Test".into(),
        description: String::new(),
        steps: vec![MacroStep::Action { action: Action::Undo }],
    };
    assert!(reg.is_empty());
    reg.insert(m.clone(), SourceTier::User);
    assert_eq!(reg.len(), 1);
    assert!(reg.contains("test"));
    let got = reg.get("test").unwrap();
    assert_eq!(got.id, "test");
    assert_eq!(got.steps.len(), 1);
    let (_, src) = reg.get_with_source("test").unwrap();
    assert_eq!(src, SourceTier::User);
}

#[test]
fn macro_registry_insert_replaces_by_id() {
    let mut reg = MacroRegistry::new();
    let m1 = Macro {
        id: "x".into(),
        name: "first".into(),
        description: String::new(),
        steps: vec![],
    };
    let m2 = Macro {
        id: "x".into(),
        name: "second".into(),
        description: String::new(),
        steps: vec![MacroStep::ConsoleLine {
            line: "fps on".into(),
        }],
    };
    reg.insert(m1, SourceTier::User);
    let prev = reg.insert(m2, SourceTier::User);
    assert!(prev.is_some());
    assert_eq!(reg.get("x").unwrap().name, "second");
}

#[test]
fn macro_source_console_line_gating() {
    assert!(SourceTier::User.allows_console_line());
    assert!(!SourceTier::App.allows_console_line());
}

#[test]
fn macro_source_allows_action_gates_destructive_actions_for_non_user() {
    // User passes everything.
    for a in [
        Action::SaveDocument,
        Action::DeleteSelection,
        Action::Cut,
        Action::Paste,
        Action::Copy,
        Action::OrphanSelection,
        Action::CreateOrphanNode,
        Action::CreateOrphanNodeAndEdit,
        Action::NewDocument,
        // Parametric filesystem variants — User tier passes
        // them; non-User tiers reject (asserted in the next
        // block).
        Action::OpenDocument("/tmp/x.mandala.json".into()),
        Action::SaveDocumentAs("/tmp/x.mandala.json".into()),
        Action::NewDocumentAt("/tmp/x.mandala.json".into()),
        // §4.6 destructive section variants — User tier passes;
        // non-User tier rejects (next block).
        Action::SetSectionText { text: "x".into() },
        Action::AddSection {
            at: "".into(),
            text: "x".into(),
        },
        Action::DeleteSection,
        Action::SplitSection {
            at_grapheme: "".into(),
        },
        Action::FastResizeStart,
    ] {
        assert!(
            SourceTier::User.allows_action(&a),
            "User tier should allow {:?}",
            a
        );
    }
    // Non-User tiers reject all of the above.
    for tier in [SourceTier::App] {
        for a in [
            Action::SaveDocument,
            Action::DeleteSelection,
            Action::Cut,
            Action::Paste,
            Action::Copy,
            Action::OrphanSelection,
            Action::CreateOrphanNode,
            Action::CreateOrphanNodeAndEdit,
            // Mixed-branch destructive Actions — their dispatch
            // arms reach editor modals that mutate model state
            // on commit.
            Action::DoubleClickActivate,
            Action::EditSelection,
            Action::EditSelectionClean,
            // Opens an editor as `EditSelection` does — a hostile
            // macro firing this with a label selected forces the
            // user into an inline editor over selected content.
            Action::LabelEditOnSelection,
            Action::NewDocument,
            // Parametric filesystem variants — denylisted for
            // non-User tiers. A macro the user did not write must not be
            // able to overwrite arbitrary files, replace the
            // active document with attacker content, or write
            // out a blank doc on top of an existing path.
            Action::OpenDocument("/tmp/x.mandala.json".into()),
            Action::SaveDocumentAs("/tmp/x.mandala.json".into()),
            Action::NewDocumentAt("/tmp/x.mandala.json".into()),
            // §4.6 destructive section variants — pin the gate
            // value-explicitly per Test Quality #2 + Security
            // M-2. Structural exhaustiveness via `ActionKind::iter()`
            // covers them transitively, but a load-bearing
            // explicit pin catches a future refactor that
            // bypassed `is_destructive()` for any reason.
            Action::SetSectionText { text: "x".into() },
            Action::AddSection {
                at: "".into(),
                text: "x".into(),
            },
            Action::DeleteSection,
            Action::SplitSection {
                at_grapheme: "".into(),
            },
            // Destructive on commit — pin explicitly so the resize
            // its right-button release commits stays gated.
            Action::FastResizeStart,
        ] {
            assert!(!tier.allows_action(&a), "{:?} tier should reject {:?}", tier, a);
        }
    }
}

#[test]
fn macro_source_allows_action_passes_navigation_for_non_user() {
    // Non-User tiers may still invoke navigation / view-state
    // Actions — they don't touch the filesystem or destroy data.
    for tier in [SourceTier::App] {
        for a in [
            Action::ZoomIn,
            Action::ZoomOut,
            Action::ZoomReset,
            Action::ZoomFit,
            Action::SelectAll,
            Action::DeselectAll,
            Action::CenterOnSelection,
            Action::JumpToRoot,
            Action::Undo,
        ] {
            assert!(
                tier.allows_action(&a),
                "{:?} tier should allow non-destructive {:?}",
                tier,
                a
            );
        }
    }
}

/// Pure-logic test for the fail-closed step-iteration contract
/// in `dispatch_macro`: walking a macro's steps stops at the
/// first rejected step. The dispatcher itself is renderer-
/// touching and can run only under the suite's GPU-backed tier
/// (TEST_CONVENTIONS §T8); this test simulates the per-step
/// policy decision so the load-bearing security claim
/// ("fail-closed: a rejected privileged step aborts the rest
/// of the macro") has a runtime regression check.
fn step_allowed(step: &MacroStep, src: SourceTier) -> bool {
    match step {
        MacroStep::Action { action } => src.allows_action(action),
        MacroStep::ConsoleLine { .. } => src.allows_console_line(),
        // An edit has no per-step privilege gate: it is one undoable
        // document change with no I/O, open to every tier.
        MacroStep::Edit { .. } => true,
    }
}

fn run_until_rejected(steps: &[MacroStep], src: SourceTier) -> &[MacroStep] {
    let mut executed = 0;
    for s in steps {
        if !step_allowed(s, src) {
            break;
        }
        executed += 1;
    }
    &steps[..executed]
}

#[test]
fn dispatch_macro_fail_closed_aborts_on_first_reject() {
    // App-tier macro: Undo (allowed) → ConsoleLine (rejected) →
    // SaveDocument (would also reject). Iteration must stop at
    // ConsoleLine; SaveDocument never gets a chance to run even
    // though it would have been rejected on its own.
    let steps = vec![
        MacroStep::Action { action: Action::Undo },
        MacroStep::ConsoleLine {
            line: "save /tmp/evil".into(),
        },
        MacroStep::Action {
            action: Action::SaveDocument,
        },
    ];
    let executed = run_until_rejected(&steps, SourceTier::App);
    assert_eq!(executed.len(), 1);
    assert!(matches!(
        &executed[0],
        MacroStep::Action { action } if matches!(action, Action::Undo)
    ));
}

#[test]
fn dispatch_macro_fail_closed_first_step_rejected() {
    // First step rejected → no steps execute.
    let steps = vec![
        MacroStep::Action {
            action: Action::SaveDocument,
        },
        MacroStep::Action { action: Action::Undo },
    ];
    let executed = run_until_rejected(&steps, SourceTier::App);
    assert!(executed.is_empty());
}

#[test]
fn dispatch_macro_user_tier_passes_destructive_steps() {
    // User-authored macros pass everything — User is trusted by
    // construction (the user wrote the file).
    let steps = vec![
        MacroStep::Action {
            action: Action::SaveDocument,
        },
        MacroStep::ConsoleLine { line: "save".into() },
        MacroStep::Action {
            action: Action::DeleteSelection,
        },
    ];
    let executed = run_until_rejected(&steps, SourceTier::User);
    assert_eq!(executed.len(), 3);
}

#[test]
fn dispatch_macro_app_tier_rejects_label_edit_on_selection() {
    // `LabelEditOnSelection` is `destructive`, and
    // `SourceTier::allows_action` admits a destructive Action only
    // from the User tier, so an App-tier macro carrying it is
    // rejected on the first step.
    let steps = vec![MacroStep::Action {
        action: Action::LabelEditOnSelection,
    }];
    let executed = run_until_rejected(&steps, SourceTier::App);
    assert!(executed.is_empty());
}

#[test]
fn macro_step_serde_round_trip() {
    let steps = vec![
        MacroStep::Action { action: Action::Undo },
        MacroStep::ConsoleLine {
            line: "fps on".into(),
        },
        MacroStep::Edit {
            id: "nudge-right".into(),
            target: MacroTarget::CurrentSelection,
        },
    ];
    let json = serde_json::to_string(&steps).unwrap();
    let parsed: Vec<MacroStep> = serde_json::from_str(&json).unwrap();
    assert_eq!(parsed.len(), 3);
    assert!(matches!(parsed[0], MacroStep::Action { action: Action::Undo }));
    if let MacroStep::ConsoleLine { line } = &parsed[1] {
        assert_eq!(line, "fps on");
    } else {
        panic!("step 1 should be ConsoleLine");
    }
}

/// Locks the on-disk JSON shape for every `MacroStep` variant —
/// hand-authored macro files in `~/.config/mandala/macros.json`
/// use these keys; a serde-derive rearrangement would silently
/// break user configs. Covers Action, ConsoleLine, and the two
/// Edit target shapes (default-omitted CurrentSelection and an
/// explicit `{node_id: <integer id>}`).
#[test]
fn macro_step_json_shape_round_trip() {
    let action = MacroStep::Action { action: Action::Undo };
    assert_eq!(
        serde_json::to_string(&action).unwrap(),
        r#"{"kind":"Action","action":"Undo"}"#,
    );

    let console = MacroStep::ConsoleLine { line: "save".into() };
    assert_eq!(
        serde_json::to_string(&console).unwrap(),
        r#"{"kind":"ConsoleLine","line":"save"}"#,
    );

    let omitted: MacroStep = serde_json::from_str(r#"{"kind":"Edit","id":"x"}"#).unwrap();
    let MacroStep::Edit { id, target } = omitted else {
        panic!("expected Edit");
    };
    assert_eq!(id, "x");
    assert!(matches!(target, MacroTarget::CurrentSelection));

    let node_target: MacroStep =
        serde_json::from_str(r#"{"kind":"Edit","id":"x","target":{"node_id":7}}"#).unwrap();
    let MacroStep::Edit {
        target: MacroTarget::NodeId(ident),
        ..
    } = node_target
    else {
        panic!("expected NodeId target");
    };
    assert_eq!(ident, 7);
    // The old step's name, and a node named by a path string, no longer
    // read (no alias).
    assert!(serde_json::from_str::<MacroStep>(r#"{"kind":"CustomMutation","id":"x"}"#).is_err());
    assert!(
        serde_json::from_str::<MacroStep>(r#"{"kind":"Edit","id":"x","target":{"node_id":"1.2"}}"#).is_err()
    );
}

/// A User macro SHADOWS an App macro with the same id — it does not
/// displace it: both slots stay filled, and lookup finds the higher.
#[test]
fn macro_registry_user_overrides_app_by_id() {
    let mut reg = MacroRegistry::new();
    let app_macro = Macro {
        id: "shared-id".into(),
        name: "App version".into(),
        description: String::new(),
        steps: vec![],
    };
    let user_macro = Macro {
        id: "shared-id".into(),
        name: "User version".into(),
        description: String::new(),
        steps: vec![MacroStep::Action { action: Action::Undo }],
    };
    // Insert order matches run_native_init::build: App first.
    reg.insert(app_macro, SourceTier::App);
    // First write to the User tier — no prior User entry to
    // return.
    let prev = reg.insert(user_macro, SourceTier::User);
    assert!(prev.is_none(), "no prior User-tier entry");
    // Lookup walks high-to-low → User wins.
    let (m, src) = reg.get_with_source("shared-id").unwrap();
    assert_eq!(m.name, "User version");
    assert_eq!(src, SourceTier::User);
    // Critical for the privilege gate: the tier upgrade is
    // honored (User can run ConsoleLine / privileged Actions
    // even if an App macro had the same id first).
    assert!(src.allows_console_line());
    assert!(
        reg.get_with_source("shared-id")
            .is_some_and(|(_, tier)| tier == SourceTier::User),
        "the shadowed App slot does not surface while the User slot is filled"
    );
}

/// Sentinel for `SourceTier::COUNT` ↔ variant-count drift. If a
/// future contributor adds a `SourceTier` variant without
/// extending `SourceTier::ALL`, `SourceTier::index()` would return
/// an out-of-bounds index for the new variant and every lookup
/// involving it would panic in `dispatch_macro`. Inserting at every
/// tier the ladder declares and asserting the count catches the
/// omission at test time rather than at runtime.
#[test]
fn macro_registry_every_tier_holds_an_entry() {
    let mut reg = MacroRegistry::new();
    let make = |id: &str| Macro {
        id: id.into(),
        name: String::new(),
        description: String::new(),
        steps: vec![],
    };
    for (i, src) in SourceTier::ALL.iter().enumerate() {
        reg.insert(make(&format!("id-{}", i)), *src);
        assert!(
            reg.get_with_source(&format!("id-{}", i)).is_some(),
            "tier {:?} (SourceTier::COUNT={}) failed to store an entry — \
             likely a slot-array / SourceTier drift",
            src,
            SourceTier::COUNT,
        );
    }
    assert_eq!(reg.len(), SourceTier::COUNT);
}

/// Within-tier id collisions follow last-writer-wins —
/// documented at `format/macros.md` "Within-tier and cross-
/// tier collision semantics". A user's `macros.json` with two
/// entries both `id: "x"` keeps only the second.
#[test]
fn macro_registry_within_tier_collision_is_last_writer() {
    let mut reg = MacroRegistry::new();
    let macros = vec![
        Macro {
            id: "x".into(),
            name: "first".into(),
            description: "".into(),
            steps: vec![],
        },
        Macro {
            id: "x".into(),
            name: "second".into(),
            description: "".into(),
            steps: vec![],
        },
    ];
    for m in macros {
        reg.insert(m, SourceTier::User);
    }
    assert_eq!(reg.len(), 1);
    let (m, _src) = reg.get_with_source("x").unwrap();
    assert_eq!(m.name, "second", "later writer wins within a tier");
}

/// `MacroRegistry::get_with_source` returns the loader-pinned
/// tier alongside the macro. This is the load-bearing accessor
/// the dispatcher uses to gate `ConsoleLine` and privileged
/// `Action` steps. Without it, a future caller that uses bare
/// `get` would silently bypass the gate.
#[test]
fn macro_registry_get_with_source_returns_pinned_tier() {
    let mut reg = MacroRegistry::new();
    let app_macro = Macro {
        id: "bundled".into(),
        name: "From the bundle".into(),
        description: String::new(),
        steps: vec![MacroStep::ConsoleLine {
            line: "save /tmp/evil".into(),
        }],
    };
    reg.insert(app_macro, SourceTier::App);
    let (m, src) = reg.get_with_source("bundled").unwrap();
    assert_eq!(src, SourceTier::App);
    // The dispatcher looks at this exact pair to decide gating.
    assert!(!src.allows_console_line());
    assert!(matches!(&m.steps[0], MacroStep::ConsoleLine { .. }));
}
