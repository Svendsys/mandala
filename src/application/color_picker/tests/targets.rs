// SPDX-License-Identifier: MPL-2.0

//! `targets.rs` unit tests — `ColorTarget` resolution and
//! `current_color_at` cascade for each `PickerHandle` shape.
//!
//! The Section arms are the primary surface added by Tier 2A of
//! `SECTION_INTEGRATION_PLAN.md`; the existing Edge / Node arms are
//! covered indirectly through the integration tests. These tests
//! pin the per-section read cascade so the picker opens with the
//! visible colour pre-seeded (same UX as the Node and Edge arms).

use crate::application::color_picker::{current_color_at, ColorTarget, PickerHandle, SectionColorAxis};
use crate::application::document::tests_common::{
    first_testament_node_id, load_test_doc, make_two_section_node_with_pinned_runs,
};

/// Build a node with two sections, each carrying a distinct
/// uniform run colour so the cascade-primary read on either
/// section returns a different value (`#111111` for section 0,
/// `#222222` for section 1). Returns `(doc, node_id)`.
fn doc_with_two_uniform_sections() -> (crate::application::document::MindMapDocument, String) {
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    // text_color_default is set to one of the per-section colours
    // so the section-1 cascade falls back to it cleanly when a
    // disagreement test appends a contrarian run.
    make_two_section_node_with_pinned_runs(
        &mut doc,
        &id,
        "#abcdef",
        ["#111111", "#222222"],
        "LiberationSans",
        14,
    );
    (doc, id)
}

/// `current_color_at` on a Section handle returns the unanimous
/// run colour when every run on the section shares one. Pins
/// Item 8 of `SECTION_INTEGRATION_PLAN.md` (cascade primary).
#[test]
fn current_color_at_section_reads_unanimous_run_color() {
    let (doc, id) = doc_with_two_uniform_sections();
    let handle = PickerHandle::Section { range: None,
        node_id: id,
        section_idx: 1,
        axis: SectionColorAxis::Text,
    };
    assert_eq!(
        current_color_at(&doc, &handle).as_deref(),
        Some("#222222"),
        "section 1's unanimous run colour should be returned"
    );
}

/// When a section's runs disagree on colour, the cascade falls
/// back to the node's `style.text_color` default. Pins Item 8
/// (cascade fallback). Mirrors the write-side cascade source
/// `set_section_text_color` reads.
#[test]
fn current_color_at_section_falls_back_to_node_default_on_run_disagreement() {
    let (mut doc, id) = doc_with_two_uniform_sections();
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.style.text_color = "#abcdef".into();
        // Append a second run on section 1 with a different colour
        // so the runs no longer agree.
        let section = node.sections.get_mut(1).expect("section 1 exists");
        section.text_runs.push(baumhard::mindmap::model::TextRun {
            start: 0,
            end: 1,
            bold: false,
            italic: false,
            underline: false,
            font: "LiberationSans".into(),
            size_pt: 14,
            color: "#999999".into(),
            hyperlink: None,
        });
    }
    let handle = PickerHandle::Section { range: None,
        node_id: id,
        section_idx: 1,
        axis: SectionColorAxis::Text,
    };
    assert_eq!(
        current_color_at(&doc, &handle).as_deref(),
        Some("#abcdef"),
        "disagreement between runs should fall back to node.style.text_color"
    );
}

/// `ColorTarget::Section.resolve()` produces a matching
/// `PickerHandle::Section` when the node + section index resolve.
/// Stale-index defensive check is exercised by the negative test
/// below.
#[test]
fn color_target_section_resolves_to_picker_handle() {
    let (doc, id) = doc_with_two_uniform_sections();
    let target = ColorTarget::Section { range: None,
        node_id: id.clone(),
        section_idx: 1,
        axis: SectionColorAxis::Text,
    };
    match target.resolve(&doc) {
        Some(PickerHandle::Section { range: None,
            node_id,
            section_idx,
            axis,
        }) => {
            assert_eq!(node_id, id);
            assert_eq!(section_idx, 1);
            assert_eq!(axis, SectionColorAxis::Text);
        }
        other => panic!("expected PickerHandle::Section, got {:?}", other),
    }
}

/// A section index past the end of `node.sections` resolves to
/// `None` rather than producing a handle that would later panic
/// in `current_color_at`. Mirrors the Edge variant's
/// stale-position defensive check.
#[test]
fn color_target_section_resolves_to_none_when_index_out_of_range() {
    let (doc, id) = doc_with_two_uniform_sections();
    let target = ColorTarget::Section { range: None,
        node_id: id,
        section_idx: 99,
        axis: SectionColorAxis::Text,
    };
    assert!(target.resolve(&doc).is_none());
}

/// `current_color_at` over a sub-range scans only the in-range
/// runs. With section 1 set up so different ranges yield
/// different unanimous colours, the picker reads each correctly.
/// Pins the N4-C.b.1 range-aware seed.
#[test]
fn current_color_at_section_range_reads_in_range_runs() {
    use baumhard::mindmap::model::TextRun;
    let (mut doc, id) = doc_with_two_uniform_sections();
    // Replace section 1's runs: [0..3 #aaaaaa, 3..7 #bbbbbb, 7..10 #cccccc]
    {
        let s = &mut doc.mindmap.nodes.get_mut(&id).unwrap().sections[1];
        s.text = "abcdefghij".into();
        s.text_runs.clear();
        for (start, end, color) in [(0, 3, "#aaaaaa"), (3, 7, "#bbbbbb"), (7, 10, "#cccccc")] {
            s.text_runs.push(TextRun {
                start,
                end,
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 14,
                color: color.into(),
                hyperlink: None,
            });
        }
    }
    // Range [3, 7) = the middle run only → unanimous #bbbbbb.
    let handle_in_range = PickerHandle::Section {
        node_id: id.clone(),
        section_idx: 1,
        axis: SectionColorAxis::Text,
        range: Some((3, 7)),
    };
    assert_eq!(
        current_color_at(&doc, &handle_in_range).as_deref(),
        Some("#bbbbbb"),
        "range-restricted cascade reads only the in-range run's colour"
    );
    // Range [0, 7) = first two runs disagree (#aaaaaa, #bbbbbb)
    // → cascade falls back to node.style.text_color (the
    // fixture's default is "#abcdef").
    let handle_disagree = PickerHandle::Section {
        node_id: id,
        section_idx: 1,
        axis: SectionColorAxis::Text,
        range: Some((0, 7)),
    };
    assert_eq!(
        current_color_at(&doc, &handle_disagree).as_deref(),
        Some("#abcdef"),
        "non-unanimous in-range runs fall back to node default"
    );
}
