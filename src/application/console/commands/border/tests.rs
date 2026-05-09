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
    first_testament_node_id as first_node_id, load_test_doc as fixture_doc,
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
        assert!(blob.contains(key), "hint missing '{}': {}", key, blob);
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
    let ids: Vec<String> = doc.mindmap.nodes.keys().take(3).cloned().collect();
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
    match run("border preset=heavy top=\"###(*)###\"", &mut doc) {
        ExecResult::Lines(rows) => {
            let joined = join_lines(&rows);
            assert!(
                joined.contains("auto-promoted") && joined.contains("'heavy'"),
                "expected auto-promotion note mentioning 'heavy', got:\n{}",
                joined
            );
        }
        other => panic!("expected Lines (success + auto-promotion note), got {:?}", other),
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
                assert!(s.contains(*k), "expected '{}' in valid-keys list, got: {}", k, s);
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

// ─────────────────────────────────────────────────────────────────
// Mutation core: `apply_border_field_to_selection`. Same setter path
// the verb uses — these tests pin the single-kv shape the parametric
// `Action::SetBorderField` Action arm calls.
// ─────────────────────────────────────────────────────────────────

#[test]
fn apply_border_field_writes_preset() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    let changed = super::apply_border_field_to_selection(&mut doc, "preset", "heavy");
    assert!(changed);
    let cfg = doc
        .mindmap
        .nodes
        .get(&id)
        .unwrap()
        .style
        .border
        .as_ref()
        .expect("preset write should leave a border config");
    assert_eq!(cfg.preset, "heavy");
}

#[test]
fn apply_border_field_returns_false_with_no_selection() {
    let mut doc = fixture_doc();
    // Default selection is None — borders are node-only.
    assert!(!super::apply_border_field_to_selection(
        &mut doc, "preset", "heavy"
    ));
}

#[test]
fn apply_border_field_returns_false_on_invalid_value() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    // `stage_kv` rejects unknown presets — the core silently no-ops
    // (Action arm warns upstream; verb path surfaces the typed err).
    assert!(!super::apply_border_field_to_selection(
        &mut doc,
        "preset",
        "totally-invalid"
    ));
}

#[test]
fn apply_border_field_returns_false_for_edge_selection() {
    let mut doc = fixture_doc();
    let e = doc.mindmap.edges.first().expect("testament edges").clone();
    doc.selection = SelectionState::Edge(crate::application::document::EdgeRef::new(
        &e.from_id,
        &e.to_id,
        &e.edge_type,
    ));
    // Edge-adjacent selections are not applicable to borders — the
    // core's selection resolver returns Err, the core returns false.
    assert!(!super::apply_border_field_to_selection(
        &mut doc, "preset", "heavy"
    ));
}

// ─── Plan §5.2 positional subverbs ─────────────────────────────────

#[test]
fn border_preset_positional_writes_through() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    assert_exec_ok(run("border preset heavy", &mut doc));
    assert_eq!(
        doc.mindmap.nodes.get(&id).unwrap().style.border.as_ref().map(|c| c.preset.as_str()),
        Some("heavy")
    );
}

#[test]
fn border_preset_cycle_advances_to_next_in_list() {
    use baumhard::mindmap::border::BORDER_PRESETS;
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    // Pin to the first preset, then cycle and assert second.
    assert_exec_ok(run(
        &format!("border preset {}", BORDER_PRESETS[0]),
        &mut doc,
    ));
    assert_exec_ok(run("border preset cycle", &mut doc));
    assert_eq!(
        doc.mindmap.nodes.get(&id).unwrap().style.border.as_ref().map(|c| c.preset.as_str()),
        Some(BORDER_PRESETS[1])
    );
}

#[test]
fn border_preset_cycle_wraps_at_last() {
    use baumhard::mindmap::border::BORDER_PRESETS;
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    // Pin to the last preset, cycle, expect wrap to first.
    assert_exec_ok(run(
        &format!("border preset {}", BORDER_PRESETS[BORDER_PRESETS.len() - 1]),
        &mut doc,
    ));
    assert_exec_ok(run("border preset cycle", &mut doc));
    assert_eq!(
        doc.mindmap.nodes.get(&id).unwrap().style.border.as_ref().map(|c| c.preset.as_str()),
        Some(BORDER_PRESETS[0])
    );
}

#[test]
fn border_preset_unknown_rejects_with_pick_one_hint() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id);
    assert_exec_err_contains(run("border preset wibble", &mut doc), "pick one of");
}

#[test]
fn border_color_positional_writes_through() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    assert_exec_ok(run("border color #ff8800", &mut doc));
    assert_eq!(
        doc.mindmap.nodes.get(&id).unwrap().style.border.as_ref().and_then(|c| c.color.as_deref()),
        Some("#ff8800")
    );
}

#[test]
fn border_padding_positional_writes_through() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    assert_exec_ok(run("border padding 12", &mut doc));
    assert_eq!(
        doc.mindmap.nodes.get(&id).unwrap().style.border.as_ref().map(|c| c.padding),
        Some(12.0)
    );
}

#[test]
fn border_palette_positional_with_field_kv_writes_both() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    assert_exec_ok(run("border palette rainbow field=text", &mut doc));
    let cfg = doc.mindmap.nodes.get(&id).unwrap().style.border.as_ref().unwrap();
    assert_eq!(cfg.color_palette.as_deref(), Some("rainbow"));
}

#[test]
fn border_toggle_flips_show_frame_per_node() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    let before = doc.mindmap.nodes.get(&id).unwrap().style.show_frame;
    assert_exec_ok(run("border toggle", &mut doc));
    let after = doc.mindmap.nodes.get(&id).unwrap().style.show_frame;
    assert_ne!(before, after, "toggle must flip show_frame");
}

#[test]
fn border_side_top_positional_writes_through() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    // Have to land preset=custom first; per the new posture, side
    // overrides on non-custom presets fall under the auto-promote
    // path until B6.7 lands the explicit error replacement.
    assert_exec_ok(run("border preset custom", &mut doc));
    assert_exec_ok(run("border side top \"=##=\"", &mut doc));
    let cfg = doc.mindmap.nodes.get(&id).unwrap().style.border.as_ref().unwrap();
    assert!(cfg.glyphs.is_some(), "side write must populate glyphs slot");
}

/// `border side top reset` must restore the cascade fall-
/// through. The CustomBorderGlyphs fields are plain Strings
/// not Options, so "reset" empties them; the renderer treats
/// empty as "use the cascade default".
#[test]
fn border_side_reset_empties_per_node_override() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    assert_exec_ok(run("border preset custom", &mut doc));
    assert_exec_ok(run("border side top \"=##=\"", &mut doc));
    let after_set = doc
        .mindmap
        .nodes
        .get(&id)
        .unwrap()
        .style
        .border
        .as_ref()
        .and_then(|c| c.glyphs.as_ref())
        .map(|g| g.top.clone())
        .unwrap_or_default();
    assert_eq!(after_set, "=##=", "side write should land the pattern");
    assert_exec_ok(run("border side top reset", &mut doc));
    // After reset the per-node side override is gone (either glyphs
    // is None or top is empty / default).
    let after_reset = doc
        .mindmap
        .nodes
        .get(&id)
        .unwrap()
        .style
        .border
        .as_ref()
        .and_then(|c| c.glyphs.as_ref())
        .map(|g| g.top.clone());
    assert_ne!(
        after_reset.as_deref(),
        Some("=##="),
        "reset must clear the per-side override"
    );
}

#[test]
fn border_side_unknown_which_rejects() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id);
    assert_exec_err_contains(run("border side diagonal \"xxx\"", &mut doc), "unknown");
}

#[test]
fn border_corner_positional_writes_through() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    assert_exec_ok(run("border preset custom", &mut doc));
    assert_exec_ok(run("border corner tl +", &mut doc));
    let g = doc
        .mindmap
        .nodes
        .get(&id)
        .unwrap()
        .style
        .border
        .as_ref()
        .and_then(|c| c.glyphs.as_ref())
        .expect("custom preset + corner write must populate glyphs");
    assert_eq!(g.top_left, "+", "tl write must land the glyph");
}

#[test]
fn border_corner_all_fans_to_four_corners() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    assert_exec_ok(run("border preset custom", &mut doc));
    assert_exec_ok(run("border corner all +", &mut doc));
    let cfg = doc.mindmap.nodes.get(&id).unwrap().style.border.as_ref().unwrap();
    let g = cfg.glyphs.as_ref().unwrap();
    assert_eq!(g.top_left, "+");
    assert_eq!(g.top_right, "+");
    assert_eq!(g.bottom_left, "+");
    assert_eq!(g.bottom_right, "+");
}

/// Plan §5.4 #3: setting a side glyph on a non-custom preset
/// errors with the explicit "run `border preset custom` first"
/// hint instead of silently auto-promoting the preset.
#[test]
fn border_side_on_non_custom_preset_errors_with_hint() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id);
    assert_exec_ok(run("border preset heavy", &mut doc));
    assert_exec_err_contains(
        run("border side top \"=##=\"", &mut doc),
        "run `border preset custom` first",
    );
}

#[test]
fn border_corner_on_non_custom_preset_errors_with_hint() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id);
    assert_exec_ok(run("border preset rounded", &mut doc));
    assert_exec_err_contains(
        run("border corner tl +", &mut doc),
        "run `border preset custom` first",
    );
}

/// `border side WHICH reset` is allowed on any preset — the
/// reset path doesn't need preset=custom because it's restoring
/// the preset's own default.
#[test]
fn border_side_reset_works_on_non_custom_preset() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id);
    assert_exec_ok(run("border preset heavy", &mut doc));
    // reset on heavy is a no-op (heavy already has its own default
    // top), so we just assert it doesn't error.
    let r = run("border side top reset", &mut doc);
    assert!(matches!(r, ExecResult::Ok(_) | ExecResult::Lines(_)),
        "reset on non-custom must succeed: {:?}", r);
}
