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
use crate::application::document::tests_common::{first_testament_node_id, load_test_doc};

/// Build a node with two sections: section 0's runs all share
/// `#111111`, section 1's runs all share `#222222`. Returns
/// `(doc, node_id)`.
fn doc_with_two_uniform_sections() -> (crate::application::document::MindMapDocument, String) {
    use baumhard::mindmap::model::MindSection;
    let mut doc = load_test_doc();
    let id = first_testament_node_id(&doc);
    {
        let node = doc.mindmap.nodes.get_mut(&id).unwrap();
        node.sections
            .push(MindSection::new_default("second".into(), Vec::new()));
        for (i, section) in node.sections.iter_mut().enumerate() {
            let colour = if i == 0 { "#111111" } else { "#222222" };
            section.text_runs.clear();
            section.text_runs.push(baumhard::mindmap::model::TextRun {
                start: 0,
                end: section.text.chars().count().max(1),
                bold: false,
                italic: false,
                underline: false,
                font: "LiberationSans".into(),
                size_pt: 14,
                color: colour.into(),
                hyperlink: None,
            });
        }
    }
    (doc, id)
}

/// `current_color_at` on a Section handle returns the unanimous
/// run colour when every run on the section shares one. Pins
/// Item 8 of `SECTION_INTEGRATION_PLAN.md` (cascade primary).
#[test]
fn current_color_at_section_reads_unanimous_run_color() {
    let (doc, id) = doc_with_two_uniform_sections();
    let handle = PickerHandle::Section {
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
    let handle = PickerHandle::Section {
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
    let target = ColorTarget::Section {
        node_id: id.clone(),
        section_idx: 1,
        axis: SectionColorAxis::Text,
    };
    match target.resolve(&doc) {
        Some(PickerHandle::Section {
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
    let target = ColorTarget::Section {
        node_id: id,
        section_idx: 99,
        axis: SectionColorAxis::Text,
    };
    assert!(target.resolve(&doc).is_none());
}
