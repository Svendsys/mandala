// SPDX-License-Identifier: MPL-2.0

//! Integration-style tests for the `border` console verb against
//! the testament map. Mirrors `font.rs::tests`'s shape: load a
//! fresh document per test, drive the verb through `tokenize` +
//! `execute_border`, assert on `ExecResult` and the resulting
//! model fields.

use crate::application::console::tests::fixtures::{
    assert_exec_err_contains, assert_exec_ok, join_lines, run,
};
use crate::application::console::ExecResult;
use crate::application::document::tests_common::{
    first_testament_node_id as first_node_id,
    load_test_doc as fixture_doc,
};
use crate::application::document::{MindMapDocument, SelectionState};

#[test]
fn border_on_then_off_toggles_show_frame() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    // Testament fixture defaults to show_frame=false; turn on
    // first, then back off, and assert each leg actually moved.
    assert_exec_ok(run("border on", &mut doc));
    assert!(doc.mindmap.nodes.get(&id).unwrap().style.show_frame);
    assert_exec_ok(run("border off", &mut doc));
    assert!(!doc.mindmap.nodes.get(&id).unwrap().style.show_frame);
}

#[test]
fn border_preset_writes_field() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    assert_exec_ok(run("border preset=heavy", &mut doc));
    let cfg = doc
        .mindmap
        .nodes
        .get(&id)
        .unwrap()
        .style
        .border
        .as_ref()
        .expect("border config materialised");
    assert_eq!(cfg.preset, "heavy");
}

/// `border preset=custom` alone (no glyph fields) is the
/// vocabulary's most confusing surface: the data model accepts it
/// but the visual is identical to the `light` preset until at
/// least one of `top=` / `bottom=` / `left=` / `right=` / `tl=` /
/// `tr=` / `bl=` / `br=` is supplied. Surface a hint listing those
/// keys so the user knows what to type next.
#[test]
fn border_preset_custom_alone_emits_glyph_field_hint() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    let lines = match run("border preset=custom", &mut doc) {
        ExecResult::Lines(rows) => rows,
        other => panic!("expected Lines for the hint output, got {:?}", other),
    };
    let blob = join_lines(&lines);
    assert!(
        blob.contains("preset=custom"),
        "expected the readout to mention preset=custom; got: {}",
        blob
    );
    // The hint mentions the eight glyph keys so the user can copy
    // a starting pair without a doc dive.
    for key in &["top=", "bottom=", "left=", "right=", "tl=", "tr=", "bl=", "br="] {
        assert!(
            blob.contains(key),
            "hint missing '{}': {}",
            key,
            blob
        );
    }
}

/// `preset=custom` together with a glyph field is the productive
/// shape — no hint should fire then. The user has already supplied
/// a side / corner override so they clearly know what they want.
#[test]
fn border_preset_custom_with_glyph_field_skips_hint() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    let lines = match run("border preset=custom top=#", &mut doc) {
        ExecResult::Lines(rows) => rows,
        ExecResult::Ok(_) => return, // no hint at all is also fine
        other => panic!("expected Ok / Lines, got {:?}", other),
    };
    let blob = join_lines(&lines);
    // The hint string identifies itself via "preset=custom" plus a
    // catalogue of side / corner keys joined together. Confirm that
    // *catalogue text* doesn't appear when at least one glyph
    // field is set — the preset-was-promoted note can still fire,
    // but the orientation hint shouldn't.
    assert!(
        !blob.contains("hint: 'custom' is the preset"),
        "hint fired despite a glyph field being set: {}",
        blob
    );
}

#[test]
fn border_top_pattern_parse_error_surfaces_with_prefix() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    // `a)b` has an unmatched `)` — parser rejects it; the verb
    // surfaces the error verbatim with a `top:` prefix.
    match run("border top=a)b", &mut doc) {
        ExecResult::Err(s) => {
            assert!(s.contains("top:"), "missing prefix: {}", s);
            assert!(s.contains("unmatched ')'"), "missing parser msg: {}", s);
        }
        other => panic!("expected Err, got {:?}", other),
    }
}

#[test]
fn border_pattern_promotes_preset_to_custom() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    assert_exec_ok(run("border top=\"###(*)###\"", &mut doc));
    let cfg = doc
        .mindmap
        .nodes
        .get(&id)
        .unwrap()
        .style
        .border
        .as_ref()
        .expect("border config materialised");
    assert_eq!(
        cfg.preset, "custom",
        "setting a side pattern should auto-promote to 'custom'"
    );
    assert_eq!(
        cfg.glyphs.as_ref().unwrap().top,
        "###(*)###",
        "raw pattern string is preserved verbatim"
    );
}

/// Pick a palette name suitable for unquoted use in the console
/// kv form — i.e. no whitespace. The testament map has both
/// space-bearing names ("My Palette") and plain ones ("coral");
/// `keys().next()` is non-deterministic across HashMap orderings,
/// and the cached fixture loader makes that ordering stable, so
/// without filtering the test would flake on whichever name lands
/// first. Quoted-palette-name parsing is its own concern (covered
/// indirectly by the existing tokenizer tests); this helper keeps
/// the palette tests focused on the kv-pipeline contract.
fn parser_friendly_palette_name(doc: &MindMapDocument) -> String {
    doc.mindmap
        .palettes
        .keys()
        .find(|n| !n.chars().any(char::is_whitespace))
        .cloned()
        .expect("testament map has at least one whitespace-free palette name")
}

#[test]
fn border_palette_records_palette_name() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    let palette_name = parser_friendly_palette_name(&doc);
    let line = format!("border palette={}", palette_name);
    assert_exec_ok(run(&line, &mut doc));
    let id = first_node_id(&doc);
    let cfg = doc
        .mindmap
        .nodes
        .get(&id)
        .unwrap()
        .style
        .border
        .as_ref()
        .expect("border config materialised");
    assert_eq!(cfg.color_palette.as_deref(), Some(palette_name.as_str()));
}

#[test]
fn border_palette_off_clears_field() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    let palette_name = parser_friendly_palette_name(&doc);
    let _ = run(&format!("border palette={}", palette_name), &mut doc);
    let _ = run("border palette=off", &mut doc);
    let cfg = doc
        .mindmap
        .nodes
        .get(&id)
        .unwrap()
        .style
        .border
        .as_ref()
        .expect("border config materialised");
    assert!(cfg.color_palette.is_none());
}

#[test]
fn border_show_emits_lines() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    match run("border show", &mut doc) {
        ExecResult::Lines(rows) => {
            assert!(!rows.is_empty());
            // Every readout includes the visible / preset / size
            // header lines — sanity-check the labels.
            let joined = join_lines(&rows);
            assert!(joined.contains("preset:"));
            assert!(joined.contains("size:"));
            assert!(joined.contains("top:"));
        }
        other => panic!("expected Lines, got {:?}", other),
    }
}

#[test]
fn border_no_selection_errors() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::None;
    assert_exec_err_contains(run("border preset=heavy", &mut doc), "no selection");
}

#[test]
fn border_grows_node_to_fit_static_parts() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    // Testament fixture defaults to show_frame=false; turn it on
    // and shrink the box so the auto-resize floor becomes
    // observable. Without `show_frame=true`,
    // `grow_one_node_to_fit_border` short-circuits per its
    // contract — the resize is gated on the user opting in to a
    // visible border.
    {
        let n = doc.mindmap.nodes.get_mut(&id).unwrap();
        n.style.show_frame = true;
        n.size.width = 10.0;
    }
    doc.selection = SelectionState::Single(id.clone());
    // 10-cluster prefix and 10-cluster suffix → border floor is
    // wider than 10 px regardless of font size.
    assert_exec_ok(run("border top=\"##########(*)##########\"", &mut doc));
    let w = doc.mindmap.nodes.get(&id).unwrap().size.width;
    assert!(
        w > 10.0,
        "node should have grown to fit the border statics; w={}",
        w
    );
}

#[test]
fn border_unknown_key_errors_with_pointer() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    assert_exec_err_contains(run("border bogus=1", &mut doc), "unknown key");
}

#[test]
fn border_unknown_subverb_errors_clearly() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    assert_exec_err_contains(run("border frobnicate", &mut doc), "unknown subverb");
}

/// `border palette=My Palette` (unquoted multi-word value) tokenises
/// as `["palette=My", "Palette"]` because the parser splits on
/// whitespace. The verb sees a bare positional alongside a kv and
/// surfaces a quoting hint rather than the generic "unknown subverb"
/// message — the latter is technically correct but unhelpful when
/// the user obviously meant a single multi-word value.
#[test]
fn border_unquoted_multi_word_value_hints_at_quoting() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    match run("border palette=My Palette", &mut doc) {
        ExecResult::Err(s) => {
            assert!(
                s.contains("did you mean to quote"),
                "expected quoting hint, got: {}",
                s
            );
            // The hint should suggest the correct quoted form.
            assert!(
                s.contains("palette=\"Palette\""),
                "expected the hint to show the quoted form, got: {}",
                s
            );
        }
        other => panic!("expected Err, got {:?}", other),
    }
}

/// Multi-node selection fans the same edit across every selected
/// node — the loop in `apply_edits` must invoke
/// `set_node_border_config` once per id and report a multi-node
/// success message. Pre-fix the loop was untested through the
/// console layer (only `Single` selections had coverage).
#[test]
fn border_multi_node_selection_applies_to_all() {
    let mut doc = fixture_doc();
    let ids: Vec<String> = doc
        .mindmap
        .nodes
        .keys()
        .take(3)
        .cloned()
        .collect();
    assert_eq!(ids.len(), 3, "testament map must have ≥3 nodes");
    doc.selection = SelectionState::Multi(ids.clone());
    match run("border preset=heavy", &mut doc) {
        ExecResult::Ok(_) | ExecResult::Lines(_) => {}
        other => panic!("expected Ok / Lines, got {:?}", other),
    }
    for id in &ids {
        let cfg = doc
            .mindmap
            .nodes
            .get(id)
            .unwrap()
            .style
            .border
            .as_ref()
            .expect("border config materialised on every selected node");
        assert_eq!(cfg.preset, "heavy");
    }
}

/// `border show` must work on a node with no per-node border
/// override (`style.border = None`) — it falls back through the
/// canvas-default cascade and the readout reports the resolved
/// values without panicking. Pre-fix, the padding line was gated
/// on `cfg.is_some()` so a default-only node had no padding row.
#[test]
fn border_show_on_node_without_per_node_override() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    // Strip any per-node border override the testament fixture
    // happens to have on node 0, then inspect.
    doc.mindmap.nodes.get_mut(&id).unwrap().style.border = None;
    doc.selection = SelectionState::Single(id);
    match run("border show", &mut doc) {
        ExecResult::Lines(rows) => {
            let joined = join_lines(&rows);
            // Padding line is now unconditional — confirm it
            // surfaces with the hardcoded floor.
            assert!(
                joined.contains("padding:"),
                "expected padding row, got:\n{}",
                joined
            );
            // Preset reports "(default)" since nothing was overridden.
            assert!(
                joined.contains("(default)"),
                "expected preset '(default)' marker, got:\n{}",
                joined
            );
        }
        other => panic!("expected Lines, got {:?}", other),
    }
}

/// `border preset=heavy top="…"` auto-promotes the preset to
/// `"custom"` because side overrides only apply to the custom
/// preset. The console verb must surface that auto-promotion
/// in its success message so the user isn't left thinking their
/// `heavy` request landed unchanged.
#[test]
fn border_preset_auto_promotion_is_announced() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    match run(
        "border preset=heavy top=\"###(*)###\"",
        &mut doc,
    ) {
        ExecResult::Lines(rows) => {
            let joined = join_lines(&rows);
            assert!(
                joined.contains("auto-promoted") && joined.contains("'heavy'"),
                "expected auto-promotion note mentioning 'heavy', got:\n{}",
                joined
            );
        }
        other => panic!(
            "expected Lines (success + auto-promotion note), got {:?}",
            other
        ),
    }
}

/// The unknown-key error must list valid keys so a typo
/// (`border colr=#fff`) gives the user enough information to
/// self-correct without grepping the source.
#[test]
fn border_unknown_key_error_lists_valid_keys() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    match run("border colr=#fff", &mut doc) {
        ExecResult::Err(s) => {
            assert!(s.contains("unknown key 'colr'"), "got: {}", s);
            assert!(s.contains("valid keys"), "got: {}", s);
            // Cherry-pick a few keys that should appear.
            for k in &["preset", "color", "palette", "top"] {
                assert!(
                    s.contains(*k),
                    "expected '{}' in valid-keys list, got: {}",
                    k, s
                );
            }
        }
        other => panic!("expected Err, got {:?}", other),
    }
}

/// `border reset` clears any previously-set per-node override.
/// Asserts the round-trip: set a custom border, then reset, then
/// confirm `style.border` is `None`. Guards the
/// `set_node_border_config({clear: true, ..})` path through the
/// verb.
#[test]
fn border_reset_clears_per_node_override() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    // Set something first so reset has something to clear.
    let _ = run("border preset=heavy", &mut doc);
    assert!(doc.mindmap.nodes.get(&id).unwrap().style.border.is_some());
    match run("border reset", &mut doc) {
        ExecResult::Ok(_) | ExecResult::Lines(_) => {}
        other => panic!("expected Ok, got {:?}", other),
    }
    assert!(
        doc.mindmap.nodes.get(&id).unwrap().style.border.is_none(),
        "border reset must drop the per-node override"
    );
}
