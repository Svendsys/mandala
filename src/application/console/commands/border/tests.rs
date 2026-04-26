// SPDX-License-Identifier: MPL-2.0

//! Integration-style tests for the `border` console verb against
//! the testament map. Mirrors `font.rs::tests`'s shape: load a
//! fresh document per test, drive the verb through `tokenize` +
//! `execute_border`, assert on `ExecResult` and the resulting
//! model fields.

use crate::application::console::parser::{tokenize, Args};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::{MindMapDocument, SelectionState};

fn fixture_doc() -> MindMapDocument {
    let path = format!(
        "{}/maps/testament.mindmap.json",
        env!("CARGO_MANIFEST_DIR")
    );
    MindMapDocument::load(&path).expect("testament map loads")
}

fn run(line: &str, doc: &mut MindMapDocument) -> ExecResult {
    let toks = tokenize(line);
    let mut eff = ConsoleEffects::new(doc);
    super::execute_border(&Args::new(&toks[1..]), &mut eff)
}

fn first_node_id(doc: &MindMapDocument) -> String {
    doc.mindmap
        .nodes
        .keys()
        .next()
        .cloned()
        .expect("testament map has nodes")
}

#[test]
fn border_on_then_off_toggles_show_frame() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    // Testament fixture defaults to show_frame=false; turn on
    // first, then back off, and assert each leg actually moved.
    match run("border on", &mut doc) {
        ExecResult::Ok(_) => {}
        other => panic!("expected Ok, got {:?}", other),
    }
    assert!(doc.mindmap.nodes.get(&id).unwrap().style.show_frame);
    match run("border off", &mut doc) {
        ExecResult::Ok(_) => {}
        other => panic!("expected Ok, got {:?}", other),
    }
    assert!(!doc.mindmap.nodes.get(&id).unwrap().style.show_frame);
}

#[test]
fn border_preset_writes_field() {
    let mut doc = fixture_doc();
    let id = first_node_id(&doc);
    doc.selection = SelectionState::Single(id.clone());
    match run("border preset=heavy", &mut doc) {
        ExecResult::Ok(_) => {}
        other => panic!("expected Ok, got {:?}", other),
    }
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
    match run("border top=\"###(*)###\"", &mut doc) {
        ExecResult::Ok(_) => {}
        other => panic!("expected Ok, got {:?}", other),
    }
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

#[test]
fn border_palette_records_palette_name() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    // Pick any palette that exists in the testament map.
    let palette_name: String = doc
        .mindmap
        .palettes
        .keys()
        .next()
        .cloned()
        .expect("testament map has palettes");
    let line = format!("border palette={}", palette_name);
    match run(&line, &mut doc) {
        ExecResult::Ok(_) => {}
        other => panic!("expected Ok, got {:?}", other),
    }
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
    let palette_name: String = doc
        .mindmap
        .palettes
        .keys()
        .next()
        .cloned()
        .expect("testament map has palettes");
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
            let joined: String = rows
                .iter()
                .map(|l| l.text.clone())
                .collect::<Vec<_>>()
                .join("\n");
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
    match run("border preset=heavy", &mut doc) {
        ExecResult::Err(s) => assert!(s.contains("no selection")),
        other => panic!("expected Err, got {:?}", other),
    }
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
    match run(
        "border top=\"##########(*)##########\"",
        &mut doc,
    ) {
        ExecResult::Ok(_) => {}
        other => panic!("expected Ok, got {:?}", other),
    }
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
    match run("border bogus=1", &mut doc) {
        ExecResult::Err(s) => assert!(s.contains("unknown key")),
        other => panic!("expected Err, got {:?}", other),
    }
}

#[test]
fn border_unknown_subverb_errors_clearly() {
    let mut doc = fixture_doc();
    doc.selection = SelectionState::Single(first_node_id(&doc));
    match run("border frobnicate", &mut doc) {
        ExecResult::Err(s) => assert!(s.contains("unknown subverb")),
        other => panic!("expected Err, got {:?}", other),
    }
}
