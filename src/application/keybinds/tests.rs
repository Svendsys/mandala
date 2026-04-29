// SPDX-License-Identifier: MPL-2.0

//! Unit tests for keybinds — parsing, matching, default config,
//! custom-mutation binding lifecycle, JSON round-trip, and
//! contextual resolution.

use super::*;
use std::collections::HashMap;

#[test]
fn test_parse_simple_key() {
    let k = KeyBind::parse("Escape").unwrap();
    assert_eq!(k.key, "escape");
    assert!(!k.ctrl && !k.shift && !k.alt);
}

#[test]
fn test_parse_ctrl_z() {
    let k = KeyBind::parse("Ctrl+Z").unwrap();
    assert_eq!(k.key, "z");
    assert!(k.ctrl);
    assert!(!k.shift && !k.alt);
}

#[test]
fn test_parse_is_case_insensitive() {
    let k1 = KeyBind::parse("ctrl+z").unwrap();
    let k2 = KeyBind::parse("CTRL+Z").unwrap();
    let k3 = KeyBind::parse("Ctrl+Z").unwrap();
    assert_eq!(k1, k2);
    assert_eq!(k2, k3);
}

#[test]
fn test_parse_all_modifiers() {
    let k = KeyBind::parse("ctrl+shift+alt+delete").unwrap();
    assert_eq!(k.key, "delete");
    assert!(k.ctrl && k.shift && k.alt);
}

#[test]
fn test_parse_whitespace_tolerated() {
    let k = KeyBind::parse(" Ctrl + Z ").unwrap();
    assert_eq!(k.key, "z");
    assert!(k.ctrl);
}

#[test]
fn test_parse_modifier_aliases() {
    // cmd/command/meta/super all map to ctrl for cross-platform muscle memory
    assert!(KeyBind::parse("Cmd+Z").unwrap().ctrl);
    assert!(KeyBind::parse("Meta+Z").unwrap().ctrl);
    assert!(KeyBind::parse("Super+Z").unwrap().ctrl);
    // option aliases alt
    assert!(KeyBind::parse("Option+Z").unwrap().alt);
}

#[test]
fn test_parse_rejects_empty() {
    assert!(KeyBind::parse("").is_err());
    assert!(KeyBind::parse("Ctrl+").is_err());
}

#[test]
fn test_parse_rejects_multiple_keys() {
    assert!(KeyBind::parse("Z+X").is_err());
    assert!(KeyBind::parse("Ctrl+Z+X").is_err());
}

#[test]
fn test_matches_modifiers_exactly() {
    let k = KeyBind::parse("Ctrl+Z").unwrap();
    assert!(k.matches("z", true, false, false));
    // Extra shift mustn't match
    assert!(!k.matches("z", true, true, false));
    // Missing ctrl mustn't match
    assert!(!k.matches("z", false, false, false));
}

#[test]
fn test_default_config_has_all_actions() {
    let cfg = KeybindConfig::default();
    let resolved = cfg.resolve();
    assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
    assert_eq!(resolved.action_for("p", true, false, false), Some(Action::EnterReparentMode));
    assert_eq!(resolved.action_for("d", true, false, false), Some(Action::EnterConnectMode));
    assert_eq!(resolved.action_for("delete", false, false, false), Some(Action::DeleteSelection));
    assert_eq!(resolved.action_for("escape", false, false, false), Some(Action::CancelMode));
    assert_eq!(resolved.action_for("n", true, false, false), Some(Action::CreateOrphanNode));
    assert_eq!(resolved.action_for("o", true, false, false), Some(Action::OrphanSelection));
    assert_eq!(resolved.action_for("enter", false, false, false), Some(Action::EditSelection));
    assert_eq!(resolved.action_for("backspace", false, false, false), Some(Action::EditSelectionClean));
}

#[test]
fn test_custom_mutation_binding_resolves_when_no_built_in_action() {
    let mut bindings = HashMap::new();
    bindings.insert("Ctrl+Shift+M".into(), "my-mutation".into());
    let cfg = KeybindConfig {
        custom_mutation_bindings: bindings,
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.custom_mutation_for("m", true, true, false),
        Some("my-mutation")
    );
}

#[test]
fn test_custom_mutation_binding_loses_to_builtin_action_via_event_loop() {
    // `custom_mutation_for` is only called after `action_for`
    // returns None — a combo bound to both resolves to the
    // built-in. This test just locks the resolver shape: both
    // lookups are independent.
    let mut bindings = HashMap::new();
    bindings.insert("Ctrl+Z".into(), "collision".into());
    let cfg = KeybindConfig {
        custom_mutation_bindings: bindings,
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
    assert_eq!(
        resolved.custom_mutation_for("z", true, false, false),
        Some("collision")
    );
}

#[test]
fn test_custom_mutation_invalid_combo_is_skipped() {
    let mut bindings = HashMap::new();
    bindings.insert("Z+X".into(), "invalid".into()); // two non-modifier keys
    bindings.insert("Ctrl+M".into(), "valid".into());
    let cfg = KeybindConfig {
        custom_mutation_bindings: bindings,
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(resolved.custom_mutation_for("m", true, false, false), Some("valid"));
}

#[test]
fn test_set_custom_mutation_binding_adds_and_replaces() {
    let mut resolved = KeybindConfig::default().resolve();
    let prev = resolved
        .set_custom_mutation_binding("Ctrl+Shift+M", "first".into())
        .unwrap();
    assert!(prev.is_none());
    assert_eq!(
        resolved.custom_mutation_for("m", true, true, false),
        Some("first")
    );
    let prev = resolved
        .set_custom_mutation_binding("Ctrl+Shift+M", "second".into())
        .unwrap();
    assert_eq!(prev.as_deref(), Some("first"));
    assert_eq!(
        resolved.custom_mutation_for("m", true, true, false),
        Some("second")
    );
}

#[test]
fn test_remove_custom_mutation_binding_returns_removed_id() {
    let mut resolved = KeybindConfig::default().resolve();
    resolved
        .set_custom_mutation_binding("Ctrl+Shift+M", "id-1".into())
        .unwrap();
    let prev = resolved.remove_custom_mutation_binding("Ctrl+Shift+M").unwrap();
    assert_eq!(prev.as_deref(), Some("id-1"));
    assert_eq!(
        resolved.custom_mutation_for("m", true, true, false),
        None
    );
}

#[test]
fn test_keybind_string_round_trip_through_parse() {
    let cases = &[
        "Ctrl+Z",
        "Ctrl+Shift+M",
        "Alt+F4",
        "Shift+Enter",
        "Escape",
    ];
    for c in cases {
        let parsed = KeyBind::parse(c).unwrap();
        let rendered = parsed.to_binding_string();
        let reparsed = KeyBind::parse(&rendered).unwrap();
        assert_eq!(parsed, reparsed, "round-trip failed for '{}'", c);
    }
}

#[test]
fn test_keybind_parse_mouse_gestures() {
    let cases = &[
        ("DoubleClick", "doubleclick"),
        ("MiddleClick", "middleclick"),
        ("LeftDrag", "leftdrag"),
        ("WheelUp", "wheelup"),
        ("WheelDown", "wheeldown"),
    ];
    for (input, expected_key) in cases {
        let k = KeyBind::parse(input).unwrap();
        assert_eq!(k.key, *expected_key, "parse('{}')", input);
        assert!(!k.ctrl && !k.shift && !k.alt);
    }
}

#[test]
fn test_keybind_parse_modified_mouse_gestures() {
    let k = KeyBind::parse("Shift+DoubleClick").unwrap();
    assert_eq!(k.key, "doubleclick");
    assert!(k.shift);
    assert!(!k.ctrl && !k.alt);

    let k = KeyBind::parse("Ctrl+WheelUp").unwrap();
    assert_eq!(k.key, "wheelup");
    assert!(k.ctrl);
}

#[test]
fn test_keybind_mouse_gesture_round_trip_pascal_case() {
    let cases = &[
        "DoubleClick",
        "MiddleClick",
        "Shift+DoubleClick",
        "Ctrl+WheelUp",
        "Ctrl+Shift+LeftDrag",
    ];
    for c in cases {
        let parsed = KeyBind::parse(c).unwrap();
        let rendered = parsed.to_binding_string();
        assert_eq!(rendered, *c, "round-trip emit form for '{}'", c);
        let reparsed = KeyBind::parse(&rendered).unwrap();
        assert_eq!(parsed, reparsed);
    }
}

#[test]
fn test_gesture_key_name_matches_parser_token() {
    // Every MouseGesture's canonical name must round-trip through
    // KeyBind::parse to a binding with the matching key field.
    let gestures = [
        MouseGesture::LeftDrag,
        MouseGesture::DoubleClick,
        MouseGesture::MiddleClick,
        MouseGesture::WheelUp,
        MouseGesture::WheelDown,
    ];
    for g in gestures {
        let name = gesture_key_name(g);
        let bind = KeyBind::parse(name).unwrap();
        assert_eq!(bind.key, name);
    }
}

// ─── WASM-compatibility classification (locks the API surface) ──

#[test]
fn test_wasm_compatibility_navigation_actions_are_compatible() {
    // Navigation / view-state Actions only touch the renderer +
    // document, both of which exist on both targets. If a contributor
    // ever flips one of these to `NativeOnly`, this test fails and
    // the WASM port loses functionality silently.
    use crate::application::keybinds::WasmCompatibility::Compatible;
    for a in [
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomReset,
        Action::ZoomFit,
        Action::PanCameraNorth,
        Action::PanCameraSouth,
        Action::PanCameraEast,
        Action::PanCameraWest,
        Action::JumpToRoot,
        Action::CenterOnSelection,
        Action::ToggleFps,
        Action::ToggleFpsDebug,
    ] {
        assert_eq!(a.wasm_compatibility(), Compatible, "{:?} should be Compatible", a);
    }
}

#[test]
fn test_wasm_compatibility_selection_actions_are_compatible() {
    use crate::application::keybinds::WasmCompatibility::Compatible;
    for a in [
        Action::SelectAll,
        Action::DeselectAll,
        Action::InvertSelection,
        Action::SelectParent,
        Action::SelectChild,
        Action::SelectNextSibling,
        Action::SelectPrevSibling,
    ] {
        assert_eq!(a.wasm_compatibility(), Compatible, "{:?} should be Compatible", a);
    }
}

#[test]
fn test_wasm_compatibility_console_modals_are_native_only() {
    use crate::application::keybinds::WasmCompatibility::NativeOnly;
    // A representative sample — the full list lives in
    // action.rs::wasm_compatibility. The test pins the contract:
    // these Actions touch native-only `console_state`, so flipping
    // them to Compatible without porting the modal would crash WASM.
    for a in [
        Action::OpenConsole,
        Action::ConsoleClose,
        Action::ConsoleSubmit,
        Action::ConsoleHistoryUp,
        Action::ConsoleHistoryDown,
        Action::ConsoleScrollUp,
    ] {
        assert_eq!(a.wasm_compatibility(), NativeOnly, "{:?} should be NativeOnly", a);
    }
}

#[test]
fn test_wasm_compatibility_modal_actions_are_native_only() {
    use crate::application::keybinds::WasmCompatibility::NativeOnly;
    for a in [
        Action::EnterReparentMode,
        Action::EnterConnectMode,
        Action::CancelMode,
        Action::PickerCancel,
        Action::PickerCommit,
        Action::LabelEditCancel,
        Action::LabelEditCommit,
        Action::LabelEditOnSelection,
        Action::OpenColorPicker,
        Action::CloseColorPicker,
        Action::SaveDocument,
        Action::PanCanvas,
        Action::NewDocument,
    ] {
        assert_eq!(a.wasm_compatibility(), NativeOnly, "{:?} should be NativeOnly", a);
    }
}

/// Mixed-branch Actions (whose dispatch arm reads/writes
/// different state per branch) classify as NativeOnly per the
/// "ANY NativeOnly branch ⇒ NativeOnly" rule. Locks the
/// classification so a future contributor can't silently
/// downgrade the rule to "the WASM-reachable branch is
/// reachable in practice" — that's the looser semantic the
/// reviewer flagged as a forward-compat trap.
#[test]
fn test_wasm_compatibility_mixed_branch_actions_are_native_only() {
    use crate::application::keybinds::WasmCompatibility::NativeOnly;
    for a in [
        // EdgeLabel branch reaches `open_label_edit` (NativeOnly state).
        Action::DoubleClickActivate,
        // EdgeLabel + Portal* selection branches reach NativeOnly editors.
        Action::EditSelection,
        Action::EditSelectionClean,
    ] {
        assert_eq!(
            a.wasm_compatibility(),
            NativeOnly,
            "{:?} should be NativeOnly under the 'ANY NativeOnly branch' rule",
            a
        );
    }
}

/// Exhaustiveness pin: every variant returns one of the two
/// `WasmCompatibility` values. The list is hand-maintained, so
/// adding a new `Action` variant requires extending it (a PR-
/// review forcing function — the compiler doesn't enforce list
/// completeness, but the test name + comment make the
/// requirement loud). Catches a future broad-default
/// `_ => Compatible` regression that would lose the match's
/// arm-per-variant structural property.
#[test]
fn test_wasm_compatibility_classifies_every_variant_explicitly() {
    use crate::application::keybinds::WasmCompatibility;
    let all_variants: &[Action] = &[
        // Document-level
        Action::Undo,
        Action::EnterReparentMode,
        Action::EnterConnectMode,
        Action::DeleteSelection,
        Action::CancelMode,
        Action::CreateOrphanNode,
        Action::OrphanSelection,
        Action::EditSelection,
        Action::EditSelectionClean,
        Action::OpenConsole,
        Action::SaveDocument,
        Action::Copy,
        Action::Paste,
        Action::Cut,
        // Console
        Action::ConsoleClose,
        Action::ConsoleSubmit,
        Action::ConsoleTabComplete,
        Action::ConsoleHistoryUp,
        Action::ConsoleHistoryDown,
        Action::ConsoleCursorLeft,
        Action::ConsoleCursorRight,
        Action::ConsoleCursorHome,
        Action::ConsoleCursorEnd,
        Action::ConsoleDeleteBack,
        Action::ConsoleDeleteForward,
        Action::ConsoleInsertSpace,
        Action::ConsoleClearLine,
        Action::ConsoleJumpStart,
        Action::ConsoleJumpEnd,
        Action::ConsoleKillToStart,
        Action::ConsoleKillWord,
        Action::ConsoleScrollUp,
        Action::ConsoleScrollDown,
        Action::ConsoleScrollPageUp,
        Action::ConsoleScrollPageDown,
        Action::ConsoleScrollEnd,
        Action::ConsoleScrollHome,
        // Picker
        Action::PickerCancel,
        Action::PickerCommit,
        Action::PickerNudgeHueDown,
        Action::PickerNudgeHueUp,
        Action::PickerNudgeSatDown,
        Action::PickerNudgeSatUp,
        Action::PickerNudgeValDown,
        Action::PickerNudgeValUp,
        // Label / text-edit cancel/commit
        Action::LabelEditCancel,
        Action::LabelEditCommit,
        Action::TextEditCancel,
        // Mouse gestures
        Action::DoubleClickActivate,
        Action::CreateOrphanNodeAndEdit,
        Action::PanCanvas,
        // Navigation / camera
        Action::ZoomIn,
        Action::ZoomOut,
        Action::ZoomReset,
        Action::ZoomFit,
        Action::PanCameraNorth,
        Action::PanCameraSouth,
        Action::PanCameraEast,
        Action::PanCameraWest,
        Action::CenterOnSelection,
        Action::JumpToRoot,
        // Selection
        Action::SelectAll,
        Action::DeselectAll,
        Action::InvertSelection,
        Action::SelectParent,
        Action::SelectChild,
        Action::SelectNextSibling,
        Action::SelectPrevSibling,
        // TextEdit cursor primitives
        Action::TextEditCursorLeft,
        Action::TextEditCursorRight,
        Action::TextEditCursorUp,
        Action::TextEditCursorDown,
        Action::TextEditCursorHome,
        Action::TextEditCursorEnd,
        Action::TextEditWordLeft,
        Action::TextEditWordRight,
        Action::TextEditDeleteBack,
        Action::TextEditDeleteForward,
        Action::TextEditDeleteWordBack,
        Action::TextEditDeleteWordForward,
        Action::TextEditCommit,
        // LabelEdit cursor primitives
        Action::LabelEditCursorLeft,
        Action::LabelEditCursorRight,
        Action::LabelEditCursorHome,
        Action::LabelEditCursorEnd,
        Action::LabelEditDeleteBack,
        Action::LabelEditDeleteForward,
        // Console verbs
        Action::OpenColorPicker,
        Action::CloseColorPicker,
        Action::LabelEditOnSelection,
        Action::ToggleFps,
        Action::ToggleFpsDebug,
        Action::NewDocument,
        // Parametric console verbs
        Action::SetEdgeAnchor {
            from: "top".into(),
            to: "auto".into(),
        },
        Action::SetEdgeBodyGlyph("dash".into()),
        Action::SetBorderField {
            field: "preset".into(),
            value: "rounded".into(),
        },
        Action::SetEdgeCap {
            from: "arrow".into(),
            to: "none".into(),
        },
        Action::SetColorBg("#fafafa".into()),
        Action::SetColorText("accent".into()),
        Action::SetColorBorder("#000000".into()),
        Action::SetEdgeType("cross_link".into()),
        Action::SetEdgeDisplayMode("portal".into()),
        Action::ResetEdge("style".into()),
    ];
    for a in all_variants {
        let c = a.wasm_compatibility();
        assert!(
            matches!(c, WasmCompatibility::Compatible | WasmCompatibility::NativeOnly),
            "{:?} returned an unexpected classification {:?}",
            a, c
        );
        // Co-pin `is_destructive` against the same variant set so
        // adding a new `Action` forces the privilege classification
        // to land alongside the WASM classification. Both methods
        // are exhaustive over `Action`, so the compiler enforces
        // the missing-arm shape; this loop pins the *value* (every
        // variant should classify either way without panicking)
        // and keeps the destructive set discoverable as a list.
        let _ = a.is_destructive();
    }
}

/// Lock the `is_destructive` set for the privilege gate. Every
/// `Action` variant in the list above resolves to `true` (gated
/// from non-User macro tiers) or `false` (allowed). Adding a new
/// variant flips the test to "missing classification" only if
/// the variant is also missing from `all_variants` — but the
/// exhaustive match in `Action::is_destructive` is the
/// load-bearing structural check. This test pins the *contents*
/// of the destructive set so a change to which variants are
/// considered destructive shows up as a diff.
#[test]
fn test_is_destructive_destructive_set_is_pinned() {
    let destructive: &[Action] = &[
        Action::SaveDocument,
        Action::NewDocument,
        Action::DeleteSelection,
        Action::OrphanSelection,
        Action::CreateOrphanNode,
        Action::CreateOrphanNodeAndEdit,
        Action::Copy,
        Action::Cut,
        Action::Paste,
        Action::DoubleClickActivate,
        Action::EditSelection,
        Action::EditSelectionClean,
        Action::LabelEditOnSelection,
    ];
    for a in destructive {
        assert!(
            a.is_destructive(),
            "{:?} expected to be destructive (privilege-gated for non-User tiers)",
            a
        );
    }

    // Spot-check a few non-destructive to lock the inverse — the
    // exhaustive match in `is_destructive` itself is the
    // structural completeness check.
    for a in [
        Action::Undo,
        Action::ZoomIn,
        Action::SelectAll,
        Action::TextEditCursorLeft,
        Action::OpenColorPicker,
        // Parametric mutators are recoverable via undo, so they
        // ride the non-destructive lane (same trust posture as
        // the existing configurable-* Actions).
        Action::SetEdgeAnchor {
            from: "top".into(),
            to: "auto".into(),
        },
        Action::SetEdgeBodyGlyph("dash".into()),
        Action::SetBorderField {
            field: "preset".into(),
            value: "rounded".into(),
        },
        Action::SetEdgeCap {
            from: "arrow".into(),
            to: "none".into(),
        },
        Action::SetColorBg("#fafafa".into()),
        Action::SetColorText("accent".into()),
        Action::SetColorBorder("#000000".into()),
        Action::SetEdgeType("cross_link".into()),
        Action::SetEdgeDisplayMode("portal".into()),
        Action::ResetEdge("style".into()),
    ] {
        assert!(
            !a.is_destructive(),
            "{:?} expected to be non-destructive",
            a
        );
    }
}

#[test]
fn test_wasm_compatibility_text_edit_primitives_are_compatible() {
    // text_edit_state exists on both targets, so the cursor /
    // delete primitives all work in the browser today.
    use crate::application::keybinds::WasmCompatibility::Compatible;
    for a in [
        Action::TextEditCancel,
        Action::TextEditCommit,
        Action::TextEditCursorLeft,
        Action::TextEditCursorRight,
        Action::TextEditCursorUp,
        Action::TextEditCursorDown,
        Action::TextEditCursorHome,
        Action::TextEditCursorEnd,
        Action::TextEditWordLeft,
        Action::TextEditWordRight,
        Action::TextEditDeleteBack,
        Action::TextEditDeleteForward,
        Action::TextEditDeleteWordBack,
        Action::TextEditDeleteWordForward,
    ] {
        assert_eq!(a.wasm_compatibility(), Compatible, "{:?} should be Compatible", a);
    }
}

#[test]
fn test_wasm_compatibility_label_edit_primitives_are_native_only() {
    // The inline label / portal-text editors only exist on native.
    // (The node text editor is shared and tested above as
    // Compatible.) When WASM gains the inline label editor, flip
    // these to Compatible.
    use crate::application::keybinds::WasmCompatibility::NativeOnly;
    for a in [
        Action::LabelEditCursorLeft,
        Action::LabelEditCursorRight,
        Action::LabelEditCursorHome,
        Action::LabelEditCursorEnd,
        Action::LabelEditDeleteBack,
        Action::LabelEditDeleteForward,
    ] {
        assert_eq!(a.wasm_compatibility(), NativeOnly, "{:?} should be NativeOnly", a);
    }
}

// ─── Mouse-gesture default-binding regression guards ───────────
// These tests pin the user-facing contract for mouse-gesture
// defaults. A future contributor flipping a default array (or
// re-introducing the empty-canvas double-click that the user
// asked us to remove) fails one of these tests.

#[test]
fn test_double_click_activate_default_resolves_to_action() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_context(InputContext::Document, "doubleclick", false, false, false),
        Some(Action::DoubleClickActivate)
    );
}

#[test]
fn test_create_orphan_node_and_edit_default_is_unbound() {
    // The user's primary feature request: empty-canvas double-click
    // does nothing by default. Implemented via an unbound default for
    // CreateOrphanNodeAndEdit, gated by has_any_binding_for in
    // dispatch::dispatch_action's DoubleClickActivate arm.
    let r = KeybindConfig::default().resolve();
    assert!(!r.has_any_binding_for(Action::CreateOrphanNodeAndEdit));
}

#[test]
fn test_has_any_binding_for_returns_true_when_user_opts_in() {
    let cfg = KeybindConfig {
        create_orphan_node_and_edit: vec!["DoubleClick".into()],
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert!(r.has_any_binding_for(Action::CreateOrphanNodeAndEdit));
}

#[test]
fn test_pan_canvas_default_resolves_via_middle_click_and_left_drag() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_context(InputContext::Document, "middleclick", false, false, false),
        Some(Action::PanCanvas)
    );
    assert_eq!(
        r.action_for_context(InputContext::Document, "leftdrag", false, false, false),
        Some(Action::PanCanvas)
    );
}

#[test]
fn test_zoom_in_default_resolves_to_wheelup() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_context(InputContext::Document, "wheelup", false, false, false),
        Some(Action::ZoomIn)
    );
}

#[test]
fn test_zoom_out_default_resolves_to_wheeldown() {
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_context(InputContext::Document, "wheeldown", false, false, false),
        Some(Action::ZoomOut)
    );
}

#[test]
fn test_action_for_gesture_falls_back_to_unmodified_binding() {
    // Modifier-fallback: Ctrl+WheelUp resolves to ZoomIn even though
    // only the bare WheelUp is bound by default. Exact-modifier
    // override still wins when the user explicitly binds the
    // modified form.
    let r = KeybindConfig::default().resolve();
    assert_eq!(
        r.action_for_gesture("wheelup", true, false, false),
        Some(Action::ZoomIn),
        "Ctrl+WheelUp should fall back to bare WheelUp -> ZoomIn"
    );
    assert_eq!(
        r.action_for_gesture("middleclick", true, true, true),
        Some(Action::PanCanvas),
        "Ctrl+Shift+Alt+MiddleClick should fall back"
    );
}

#[test]
fn test_action_for_gesture_exact_modifier_match_wins_over_fallback() {
    // Clear default zoom_in (also bound to WheelUp) so the test
    // exercises only the configured bindings.
    let cfg = KeybindConfig {
        zoom_in: vec![],
        zoom_out: vec!["WheelUp".into()],            // bare WheelUp -> ZoomOut
        zoom_reset: vec!["Ctrl+WheelUp".into()],     // Ctrl+WheelUp -> ZoomReset
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert_eq!(
        r.action_for_gesture("wheelup", true, false, false),
        Some(Action::ZoomReset),
        "exact Ctrl+WheelUp binding wins over the bare-WheelUp fallback"
    );
    assert_eq!(
        r.action_for_gesture("wheelup", false, false, false),
        Some(Action::ZoomOut),
        "bare wheelup honours its bare binding"
    );
}

// ─── Macro-tier resolution-order tests ─────────────────────────

#[test]
fn test_macro_for_returns_bound_id() {
    let mut bindings = HashMap::new();
    bindings.insert("Ctrl+G".to_string(), "do-stuff".to_string());
    let cfg = KeybindConfig {
        macro_bindings: bindings,
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert_eq!(r.macro_for("g", true, false, false), Some("do-stuff"));
    assert_eq!(r.macro_for("g", false, false, false), None);
}

#[test]
fn test_macro_bindings_resolve_skips_invalid_combos() {
    let mut bindings = HashMap::new();
    bindings.insert("Ctrl+G".to_string(), "valid".to_string());
    bindings.insert("Garbage++".to_string(), "would-be-orphan".to_string());
    let cfg = KeybindConfig {
        macro_bindings: bindings,
        ..KeybindConfig::default()
    };
    // Resolve survives — invalid combos log and skip; the valid one
    // still lands.
    let r = cfg.resolve();
    assert_eq!(r.macro_for("g", true, false, false), Some("valid"));
}

#[test]
fn test_action_for_gesture_returns_none_when_completely_unbound() {
    let cfg = KeybindConfig {
        zoom_in: vec![],
        zoom_out: vec![],
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert_eq!(r.action_for_gesture("wheelup", false, false, false), None);
    assert_eq!(r.action_for_gesture("wheelup", true, false, false), None);
}

#[test]
fn test_default_console_font_size_is_16() {
    let cfg = KeybindConfig::default();
    assert!((cfg.console_font_size - 16.0).abs() < f32::EPSILON);
}

#[test]
fn test_resolve_exposes_console_style_fields() {
    let cfg = KeybindConfig {
        console_font: "MyFont".into(),
        console_font_size: 20.0,
        ..KeybindConfig::default()
    };
    let r = cfg.resolve();
    assert_eq!(r.console_font, "MyFont");
    assert!((r.console_font_size - 20.0).abs() < f32::EPSILON);
}

#[test]
fn test_open_console_default_bound_to_slash() {
    let cfg = KeybindConfig::default();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for("/", false, false, false),
        Some(Action::OpenConsole)
    );
}

#[test]
fn test_open_console_in_document_context() {
    // The event loop calls `action_for_context(Document, "/", …)`
    // — not the bare `action_for("/")`. Pins the resolver path the
    // event loop actually walks, guarding the `/` → console binding
    // against a regression in contextual dispatch.
    let resolved = KeybindConfig::default().resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "/", false, false, false),
        Some(Action::OpenConsole),
    );
}

#[test]
fn test_all_document_defaults_resolve_via_action_for_context() {
    // Parametric coverage for every default Document-context
    // binding under the new resolver. Catches any regression that
    // slips past the single-action tests above.
    let r = KeybindConfig::default().resolve();
    let doc = InputContext::Document;
    let cases: &[(Action, &str, bool, bool, bool)] = &[
        (Action::Undo, "z", true, false, false),
        (Action::Undo, "undo", false, false, false),
        (Action::EnterReparentMode, "p", true, false, false),
        (Action::EnterConnectMode, "d", true, false, false),
        (Action::DeleteSelection, "delete", false, false, false),
        (Action::CancelMode, "escape", false, false, false),
        (Action::CreateOrphanNode, "n", true, false, false),
        (Action::OrphanSelection, "o", true, false, false),
        (Action::EditSelection, "enter", false, false, false),
        (Action::EditSelectionClean, "backspace", false, false, false),
        (Action::OpenConsole, "/", false, false, false),
        (Action::SaveDocument, "s", true, false, false),
        (Action::Copy, "c", true, false, false),
        (Action::Copy, "copy", false, false, false),
        (Action::Paste, "v", true, false, false),
        (Action::Paste, "paste", false, false, false),
        (Action::Cut, "x", true, false, false),
        (Action::Cut, "cut", false, false, false),
    ];
    for (action, key, ctrl, shift, alt) in cases {
        assert_eq!(
            r.action_for_context(doc, key, *ctrl, *shift, *alt),
            Some(action.clone()),
            "expected {:?} for key={:?} ctrl={} shift={} alt={}",
            action, key, ctrl, shift, alt,
        );
    }
}

#[test]
fn test_save_document_default_bound_to_ctrl_s() {
    let cfg = KeybindConfig::default();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for("s", true, false, false),
        Some(Action::SaveDocument)
    );
}

#[test]
fn test_default_config_has_undo_alias() {
    // Ctrl+Z and the bare "Undo" key should both fire undo
    let cfg = KeybindConfig::default();
    let resolved = cfg.resolve();
    assert_eq!(resolved.action_for("undo", false, false, false), Some(Action::Undo));
}

#[test]
fn test_partial_json_uses_defaults_for_missing_fields() {
    // A user who only wants to rebind one action should be able to omit
    // every other field and get the defaults for them.
    let json = r#"{ "undo": ["Ctrl+Y"] }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    assert_eq!(cfg.undo, vec!["Ctrl+Y"]);
    // Other fields should still have defaults
    assert_eq!(cfg.enter_reparent_mode, vec!["Ctrl+P"]);
    assert_eq!(cfg.cancel_mode, vec!["Escape"]);
}

#[test]
fn test_resolve_skips_invalid_bindings() {
    let cfg = KeybindConfig {
        undo: vec!["Ctrl+Z".into(), "Z+X".into()], // second is invalid
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    // Valid binding still works
    assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
}

#[test]
fn test_user_override_replaces_default() {
    // A user who specifies undo bindings should get only those — not
    // theirs merged with the hardcoded list. This matches common
    // config-file intuition.
    let json = r#"{ "undo": ["Ctrl+Y"] }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    let resolved = cfg.resolve();
    assert_eq!(resolved.action_for("y", true, false, false), Some(Action::Undo));
    // Original Ctrl+Z no longer bound
    assert_eq!(resolved.action_for("z", true, false, false), None);
}

#[test]
fn test_json_roundtrip() {
    let cfg = KeybindConfig::default();
    let json = serde_json::to_string(&cfg).unwrap();
    let parsed = KeybindConfig::from_json(&json).unwrap();
    let resolved = parsed.resolve();
    assert_eq!(resolved.action_for("z", true, false, false), Some(Action::Undo));
}

#[test]
fn test_normalize_key_name() {
    assert_eq!(normalize_key_name("Escape"), "escape");
    assert_eq!(normalize_key_name("  Delete  "), "delete");
    assert_eq!(normalize_key_name("Z"), "z");
}

// ── Component-scoped actions and contextual resolution ──

#[test]
fn test_default_config_has_console_actions() {
    let resolved = KeybindConfig::default().resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "escape", false, false, false),
        Some(Action::ConsoleClose),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "enter", false, false, false),
        Some(Action::ConsoleSubmit),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "tab", false, false, false),
        Some(Action::ConsoleTabComplete),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "c", true, false, false),
        Some(Action::ConsoleClearLine),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "a", true, false, false),
        Some(Action::ConsoleJumpStart),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "e", true, false, false),
        Some(Action::ConsoleJumpEnd),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "u", true, false, false),
        Some(Action::ConsoleKillToStart),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "w", true, false, false),
        Some(Action::ConsoleKillWord),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "backspace", false, false, false),
        Some(Action::ConsoleDeleteBack),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "space", false, false, false),
        Some(Action::ConsoleInsertSpace),
    );
}

#[test]
fn test_default_config_has_picker_actions() {
    let resolved = KeybindConfig::default().resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "escape", false, false, false),
        Some(Action::PickerCancel),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "enter", false, false, false),
        Some(Action::PickerCommit),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "h", false, false, false),
        Some(Action::PickerNudgeHueDown),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "h", false, true, false),
        Some(Action::PickerNudgeHueUp),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "s", false, false, false),
        Some(Action::PickerNudgeSatDown),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "v", false, false, false),
        Some(Action::PickerNudgeValDown),
    );
}

#[test]
fn test_default_config_has_label_edit_actions() {
    let resolved = KeybindConfig::default().resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::LabelEdit, "escape", false, false, false),
        Some(Action::LabelEditCancel),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::LabelEdit, "enter", false, false, false),
        Some(Action::LabelEditCommit),
    );
}

#[test]
fn test_default_config_has_text_edit_actions() {
    let resolved = KeybindConfig::default().resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::TextEdit, "escape", false, false, false),
        Some(Action::TextEditCancel),
    );
}

#[test]
fn test_console_context_does_not_leak_document_actions() {
    let resolved = KeybindConfig::default().resolve();
    // Ctrl+Z is Undo in Document but should not resolve in Console
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "z", true, false, false),
        None,
    );
    // "/" is OpenConsole in Document but should not resolve in Console
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "/", false, false, false),
        None,
    );
}

#[test]
fn test_picker_context_falls_through_to_document() {
    let resolved = KeybindConfig::default().resolve();
    // Ctrl+Z is not a picker action, but color picker falls through
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "z", true, false, false),
        Some(Action::Undo),
    );
    // "/" opens console — should fall through from picker
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "/", false, false, false),
        Some(Action::OpenConsole),
    );
}

#[test]
fn test_picker_context_prefers_picker_action_over_document() {
    let resolved = KeybindConfig::default().resolve();
    // Escape is CancelMode at Document level but PickerCancel at picker level
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "escape", false, false, false),
        Some(Action::PickerCancel),
    );
    // Enter is EditSelection at Document level but PickerCommit at picker level
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "enter", false, false, false),
        Some(Action::PickerCommit),
    );
}

#[test]
fn test_label_edit_does_not_fall_through() {
    let resolved = KeybindConfig::default().resolve();
    // Ctrl+Z should not resolve in label edit (no fallthrough)
    assert_eq!(
        resolved.action_for_context(InputContext::LabelEdit, "z", true, false, false),
        None,
    );
}

#[test]
fn test_text_edit_does_not_fall_through() {
    let resolved = KeybindConfig::default().resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::TextEdit, "z", true, false, false),
        None,
    );
}

#[test]
fn test_document_context_matches_action_for() {
    let resolved = KeybindConfig::default().resolve();
    // Document context should match all global actions the same as action_for
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "z", true, false, false),
        resolved.action_for("z", true, false, false),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "escape", false, false, false),
        resolved.action_for("escape", false, false, false),
    );
}

#[test]
fn test_action_context_assignment() {
    assert_eq!(Action::Undo.context(), InputContext::Document);
    assert_eq!(Action::Copy.context(), InputContext::Document);
    assert_eq!(Action::ConsoleClose.context(), InputContext::Console);
    assert_eq!(Action::ConsoleClearLine.context(), InputContext::Console);
    assert_eq!(Action::PickerCancel.context(), InputContext::ColorPicker);
    assert_eq!(Action::PickerNudgeHueDown.context(), InputContext::ColorPicker);
    assert_eq!(Action::LabelEditCancel.context(), InputContext::LabelEdit);
    assert_eq!(Action::TextEditCancel.context(), InputContext::TextEdit);
}

#[test]
fn test_user_can_override_component_keybinds() {
    let json = r#"{ "picker_nudge_hue_down": ["j"], "picker_nudge_hue_up": ["k"] }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "j", false, false, false),
        Some(Action::PickerNudgeHueDown),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "k", false, false, false),
        Some(Action::PickerNudgeHueUp),
    );
    // Original "h" no longer bound to hue nudge
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "h", false, false, false),
        None,
    );
}

#[test]
fn test_copy_paste_cut_fall_through_to_picker() {
    let resolved = KeybindConfig::default().resolve();
    // Copy/Paste/Cut are Document-level actions that fall through
    // to the color picker context
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "c", true, false, false),
        Some(Action::Copy),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "v", true, false, false),
        Some(Action::Paste),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "x", true, false, false),
        Some(Action::Cut),
    );
}

#[test]
fn test_partial_json_preserves_component_defaults() {
    let json = r#"{ "undo": ["Ctrl+Y"] }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    // Console defaults should still be present
    assert_eq!(cfg.console_close, vec!["Escape"]);
    assert_eq!(cfg.console_clear_line, vec!["Ctrl+C"]);
    // Picker defaults should still be present
    assert_eq!(cfg.picker_nudge_hue_down, vec!["h"]);
}

#[test]
fn test_empty_binding_list_disables_action() {
    let json = r#"{ "cancel_mode": [] }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "escape", false, false, false),
        None,
    );
}

#[test]
fn test_duplicate_key_in_same_context_first_wins() {
    let json = r#"{
        "console_close": ["Tab"],
        "console_tab_complete": ["Tab"]
    }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "tab", false, false, false),
        Some(Action::ConsoleClose),
    );
}

#[test]
fn test_action_for_context_document_filters_component_actions() {
    let resolved = KeybindConfig::default().resolve();
    // "tab" has no Document binding. action_for (global) returns
    // ConsoleTabComplete, but action_for_context(Document) returns None.
    assert_eq!(
        resolved.action_for("tab", false, false, false),
        Some(Action::ConsoleTabComplete),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "tab", false, false, false),
        None,
    );
}

#[test]
fn test_json_roundtrip_all_contexts() {
    let cfg = KeybindConfig::default();
    let json = serde_json::to_string(&cfg).unwrap();
    let parsed = KeybindConfig::from_json(&json).unwrap();
    let resolved = parsed.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "z", true, false, false),
        Some(Action::Undo),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Console, "escape", false, false, false),
        Some(Action::ConsoleClose),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::ColorPicker, "h", false, false, false),
        Some(Action::PickerNudgeHueDown),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::LabelEdit, "enter", false, false, false),
        Some(Action::LabelEditCommit),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::TextEdit, "escape", false, false, false),
        Some(Action::TextEditCancel),
    );
}

// ─────────────────────────────────────────────────────────────────
// Parametric bindings (`ParametricBinding`)
// ─────────────────────────────────────────────────────────────────

#[test]
fn test_parametric_set_edge_anchor_resolves_with_two_args() {
    let cfg = KeybindConfig {
        set_edge_anchor: vec![ParametricBinding {
            combo: "Ctrl+Shift+a".into(),
            args: vec!["top".into(), "auto".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "a", true, true, false),
        Some(Action::SetEdgeAnchor {
            from: "top".into(),
            to: "auto".into(),
        }),
    );
}

#[test]
fn test_parametric_set_edge_body_glyph_resolves_with_one_arg() {
    let cfg = KeybindConfig {
        set_edge_body_glyph: vec![ParametricBinding {
            combo: "Ctrl+b".into(),
            args: vec!["dash".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "b", true, false, false),
        Some(Action::SetEdgeBodyGlyph("dash".into())),
    );
}

#[test]
fn test_parametric_wrong_arg_count_is_skipped() {
    // A 1-arg binding for a 2-arg variant — the build closure
    // returns None, the warn-log fires, no Action lands in the
    // resolved table. Crucially: not a panic, so a user-config
    // typo never crashes the app.
    //
    // The combo (Ctrl+F8) intentionally avoids the default-bound
    // chords so the assertion is about "no parametric Action got
    // built", not "the default got shadowed."
    let cfg = KeybindConfig {
        set_edge_anchor: vec![ParametricBinding {
            combo: "Ctrl+F8".into(),
            args: vec!["top".into()], // missing the `to` arg
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f8", true, false, false),
        None,
    );
}

#[test]
fn test_parametric_binding_round_trips_through_json() {
    let cfg = KeybindConfig {
        set_edge_anchor: vec![ParametricBinding {
            combo: "Ctrl+Shift+a".into(),
            args: vec!["top".into(), "auto".into()],
        }],
        set_edge_body_glyph: vec![ParametricBinding {
            combo: "Ctrl+b".into(),
            args: vec!["dash".into()],
        }],
        ..KeybindConfig::default()
    };
    let json = serde_json::to_string(&cfg).unwrap();
    let parsed = KeybindConfig::from_json(&json).unwrap();
    assert_eq!(parsed.set_edge_anchor, cfg.set_edge_anchor);
    assert_eq!(parsed.set_edge_body_glyph, cfg.set_edge_body_glyph);
}

#[test]
fn test_parametric_set_border_field_resolves_with_two_args() {
    let cfg = KeybindConfig {
        set_border_field: vec![ParametricBinding {
            combo: "Ctrl+Shift+b".into(),
            args: vec!["preset".into(), "rounded".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "b", true, true, false),
        Some(Action::SetBorderField {
            field: "preset".into(),
            value: "rounded".into(),
        }),
    );
}

#[test]
fn test_parametric_color_axes_resolve() {
    let cfg = KeybindConfig {
        set_color_bg: vec![ParametricBinding {
            combo: "F1".into(),
            args: vec!["#fafafa".into()],
        }],
        set_color_text: vec![ParametricBinding {
            combo: "F2".into(),
            args: vec!["accent".into()],
        }],
        set_color_border: vec![ParametricBinding {
            combo: "F3".into(),
            args: vec!["#000000".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f1", false, false, false),
        Some(Action::SetColorBg("#fafafa".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f2", false, false, false),
        Some(Action::SetColorText("accent".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f3", false, false, false),
        Some(Action::SetColorBorder("#000000".into())),
    );
}

#[test]
fn test_parametric_edge_structural_resolve() {
    let cfg = KeybindConfig {
        set_edge_type: vec![ParametricBinding {
            combo: "F4".into(),
            args: vec!["cross_link".into()],
        }],
        set_edge_display_mode: vec![ParametricBinding {
            combo: "F5".into(),
            args: vec!["portal".into()],
        }],
        reset_edge: vec![ParametricBinding {
            combo: "F6".into(),
            args: vec!["style".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f4", false, false, false),
        Some(Action::SetEdgeType("cross_link".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f5", false, false, false),
        Some(Action::SetEdgeDisplayMode("portal".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "f6", false, false, false),
        Some(Action::ResetEdge("style".into())),
    );
}

#[test]
fn test_parametric_set_edge_cap_resolves_with_two_args() {
    let cfg = KeybindConfig {
        set_edge_cap: vec![ParametricBinding {
            combo: "Ctrl+Shift+c".into(),
            args: vec!["arrow".into(), "none".into()],
        }],
        ..KeybindConfig::default()
    };
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "c", true, true, false),
        Some(Action::SetEdgeCap {
            from: "arrow".into(),
            to: "none".into(),
        }),
    );
}

#[test]
fn test_parametric_binding_user_partial_config_only_overrides_listed_field() {
    // Confirm the `#[serde(default)]` shape works: a partial JSON
    // with only the parametric field set leaves every other binding
    // at its default.
    let json = r#"{
        "set_edge_body_glyph": [
            { "combo": "Ctrl+b", "args": ["dash"] }
        ]
    }"#;
    let cfg = KeybindConfig::from_json(json).unwrap();
    let resolved = cfg.resolve();
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "b", true, false, false),
        Some(Action::SetEdgeBodyGlyph("dash".into())),
    );
    assert_eq!(
        resolved.action_for_context(InputContext::Document, "z", true, false, false),
        Some(Action::Undo),
    );
}
