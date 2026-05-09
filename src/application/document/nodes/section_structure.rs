// SPDX-License-Identifier: MPL-2.0

//! Section structural mutators — `add_section`, `delete_section`,
//! `split_section`. These three change the *length* of a node's
//! `sections: Vec<MindSection>`, which shifts the indices of
//! subsequent sections; sibling section setters
//! (`set_section_offset`, `set_section_size`, `set_section_text`,
//! etc.) preserve indices and live in `mod.rs` / `section_text.rs`.
//!
//! All three push an `UndoAction::EditNodeStyle` undo entry — the
//! variant captures `before_sections: Vec<MindSection>` so undo
//! restores the entire structure (including text, runs, channels,
//! and trigger bindings on every section that was added / deleted /
//! split). Same envelope the index-preserving setters use; no new
//! undo variant required.
//!
//! Per `SECTIONS_BORDERS_RESIZE_PLAN.md` §4.5 (Batch 5).

use baumhard::mindmap::model::MindSection;

use super::super::undo_action::UndoAction;
use super::super::MindMapDocument;
use super::{grow_one_node_to_fit_border, validate_section_aabb};

impl MindMapDocument {
    /// Insert a new section into `node_id.sections` at `at` (default
    /// end). Validates the new section's AABB against the parent
    /// node's size. Pushes one `EditNodeStyle` undo entry.
    ///
    /// Returns the index the section was inserted at, or an error
    /// when the node doesn't exist or the AABB is invalid.
    ///
    /// `at` is clamped to `[0, sections.len()]`; inserting past the
    /// end is the same as appending. Authoring-time conveniences
    /// (`at = None` to append, `at = Some(0)` to prepend) cover the
    /// two common patterns; `Some(i)` for `i > sections.len()` is
    /// silently clamped rather than rejected so a console caller
    /// passing an off-by-one doesn't fail mid-multi-step macro.
    pub fn add_section(
        &mut self,
        node_id: &str,
        at: Option<usize>,
        section: MindSection,
    ) -> Result<usize, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Err(format!("add_section: node '{}' not found", node_id)),
        };
        let len = node.sections.len();
        let insert_at = at.unwrap_or(len).min(len);
        // Validate against the parent node's size — same shape the
        // index-preserving setters use, parameterised on the
        // would-be index.
        validate_section_aabb(node.size, insert_at, section.offset, section.size)?;

        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just confirmed exists");
        node.sections.insert(insert_at, section);

        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
        });
        self.dirty = true;

        // Adding a fill-parent section (size=None) can shift the
        // measured-text floor; run the floor passes so the next
        // unrelated edit doesn't see a stale under-floor node.
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just inserted");
        super::super::grow_one_node_to_fit_text(node);
        grow_one_node_to_fit_border(node, canvas_default.as_ref());

        Ok(insert_at)
    }

    /// Remove the section at `idx` from `node_id.sections`. Returns
    /// the removed section so callers can stash it (e.g. for a
    /// console-side "deleted; undo to restore" message). Errors
    /// when the node doesn't exist, `idx` is out of range, or the
    /// node has only one section (the model invariant: every
    /// renderable node has at least one section).
    ///
    /// Pushes one `EditNodeStyle` undo entry — the
    /// `before_sections` snapshot fully restores the deleted
    /// section on undo (including text, runs, channels, trigger
    /// bindings, frame border).
    pub fn delete_section(
        &mut self,
        node_id: &str,
        idx: usize,
    ) -> Result<MindSection, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Err(format!("delete_section: node '{}' not found", node_id)),
        };
        if idx >= node.sections.len() {
            return Err(format!(
                "delete_section: section[{}] not found on node '{}' (has {} section(s))",
                idx,
                node_id,
                node.sections.len()
            ));
        }
        if node.sections.len() == 1 {
            return Err(format!(
                "delete_section: cannot delete the only section on node '{}' \
                 (every renderable node has at least one section)",
                node_id
            ));
        }

        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just confirmed exists");
        let removed = node.sections.remove(idx);

        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
        });
        self.dirty = true;

        // Same floor-pass discipline as `add_section`: removing a
        // section can change the measured-text floor (and therefore
        // the node's required height), so re-run the passes against
        // the updated structure.
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just removed from");
        super::super::grow_one_node_to_fit_text(node);
        grow_one_node_to_fit_border(node, canvas_default.as_ref());

        Ok(removed)
    }

    /// Split the section at `idx` into two adjacent sections.
    /// `at_grapheme` is the grapheme boundary in the section's
    /// `text` where the split lands; the prefix stays at `idx`,
    /// the suffix becomes a new section at `idx + 1`. `None`
    /// defaults to the end of the existing text — equivalent to
    /// "clone this section with empty text and insert it after".
    ///
    /// **`text_runs` are dropped on the new section** (the split
    /// inherits a single empty `text_runs: Vec` rather than
    /// trying to split styled runs at an arbitrary grapheme
    /// boundary, which is a deeper concern that pulls in
    /// `TextRun::range` arithmetic; punted to a follow-up). The
    /// prefix section's runs are truncated to the split point so
    /// per-grapheme styling on the kept prefix survives.
    ///
    /// `frame_border`, `channel`, and `trigger_bindings` are
    /// **cloned** onto the new section — splitting visually
    /// creates two slices of the same authoring intent, and per-
    /// section overrides (frame style, click bindings, mutation
    /// channel) typically apply to both halves. Authors who want
    /// asymmetric overrides can edit the new section after the
    /// split.
    ///
    /// Pushes one `EditNodeStyle` undo entry. Returns the new
    /// section's index (`idx + 1`). Errors on missing node,
    /// out-of-range `idx`, or `at_grapheme` past the section's
    /// text length.
    pub fn split_section(
        &mut self,
        node_id: &str,
        idx: usize,
        at_grapheme: Option<usize>,
    ) -> Result<usize, String> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return Err(format!("split_section: node '{}' not found", node_id)),
        };
        let Some(section) = node.sections.get(idx) else {
            return Err(format!(
                "split_section: section[{}] not found on node '{}' (has {} section(s))",
                idx,
                node_id,
                node.sections.len()
            ));
        };

        let original_text = &section.text;
        // Resolve the grapheme split into a byte offset. `None`
        // means "split at end of text" — the new section gets
        // empty text. `Some(g)` resolves the grapheme cluster
        // boundary; `g > grapheme_count` errors.
        let split_byte = resolve_split_byte_index(original_text, at_grapheme)?;

        let prefix_text = original_text[..split_byte].to_string();
        let suffix_text = original_text[split_byte..].to_string();

        // Build the new (suffix) section. Clone the per-section
        // metadata that semantically applies to both halves
        // (channel, trigger_bindings, frame_border); offset / size
        // are re-derived from the original below.
        let mut new_section = section.clone();
        new_section.text = suffix_text;
        // Drop runs on the new section — splitting per-grapheme
        // styled runs at an arbitrary boundary is a deeper
        // concern than this verb wants to land. Authors who need
        // styled splits re-author after.
        new_section.text_runs.clear();

        let before_style = node.style.clone();
        let before_sections = node.sections.clone();
        let canvas_default = self.mindmap.canvas.default_border.clone();
        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just confirmed exists");
        // Truncate the prefix's text + drop runs that overflow the
        // split. Index-preserving runs (start..end fully within
        // the prefix byte range) survive; runs that straddle the
        // split or live in the suffix are dropped, matching the
        // "drop runs on the new section" posture.
        node.sections[idx].text = prefix_text;
        node.sections[idx]
            .text_runs
            .retain(|r| r.end <= split_byte);

        let new_idx = idx + 1;
        node.sections.insert(new_idx, new_section);

        self.undo_stack.push(UndoAction::EditNodeStyle {
            node_id: node_id.to_string(),
            before_style,
            before_sections,
        });
        self.dirty = true;

        let node = self
            .mindmap
            .nodes
            .get_mut(node_id)
            .expect("just split");
        super::super::grow_one_node_to_fit_text(node);
        grow_one_node_to_fit_border(node, canvas_default.as_ref());

        Ok(new_idx)
    }
}

/// Resolve a grapheme-cluster index into a byte offset within
/// `text`. `None` → end-of-text (text.len()). `Some(g)` → the byte
/// position of the start of the `g`-th grapheme cluster, or
/// `text.len()` when `g == grapheme_count`. Errors when `g >
/// grapheme_count`.
fn resolve_split_byte_index(text: &str, at_grapheme: Option<usize>) -> Result<usize, String> {
    use unicode_segmentation::UnicodeSegmentation;
    let Some(g) = at_grapheme else {
        return Ok(text.len());
    };
    // Build a vector of (byte_index, grapheme) pairs once. For
    // typical section text lengths this is sub-microsecond; the
    // alternative (iter::nth) requires walking the iter twice
    // because we need both the grapheme count for the bounds
    // check and the byte index for the `g`-th boundary.
    let pairs: Vec<(usize, &str)> = text.grapheme_indices(true).collect();
    if g > pairs.len() {
        return Err(format!(
            "split_section: at_grapheme={} > grapheme_count={} (text='{}')",
            g,
            pairs.len(),
            text
        ));
    }
    if g == pairs.len() {
        return Ok(text.len());
    }
    Ok(pairs[g].0)
}

#[cfg(test)]
mod tests {
    use super::super::super::tests_common::{first_testament_node_id, load_test_doc};
    use baumhard::mindmap::model::{MindSection, Position, Size};

    fn empty_section() -> MindSection {
        MindSection {
            text: String::new(),
            text_runs: Vec::new(),
            offset: Position::default(),
            size: None,
            channel: None,
            trigger_bindings: Vec::new(),
            frame_border: None,
        }
    }

    #[test]
    fn add_section_appends_when_at_is_none() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let original_len = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        let mut s = empty_section();
        s.text = "appended".into();
        let idx = doc.add_section(&id, None, s).expect("add ok");
        assert_eq!(idx, original_len);
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections.len(),
            original_len + 1
        );
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections[idx].text,
            "appended"
        );
    }

    #[test]
    fn add_section_inserts_at_index_when_at_is_some() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let original = doc.mindmap.nodes.get(&id).unwrap().sections[0].text.clone();
        let mut s = empty_section();
        s.text = "prepended".into();
        let idx = doc.add_section(&id, Some(0), s).expect("add ok");
        assert_eq!(idx, 0);
        assert_eq!(doc.mindmap.nodes.get(&id).unwrap().sections[0].text, "prepended");
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections[1].text,
            original,
            "previous section[0] is now at idx 1"
        );
    }

    #[test]
    fn add_section_clamps_at_past_end() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let original_len = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        let mut s = empty_section();
        s.text = "clamped".into();
        let idx = doc.add_section(&id, Some(9999), s).expect("add ok");
        assert_eq!(idx, original_len, "at past len clamps to len (append)");
    }

    #[test]
    fn add_section_pushes_undo_and_dirty() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.undo_stack.clear();
        doc.dirty = false;
        let original_len = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        let _ = doc.add_section(&id, None, empty_section()).expect("add ok");
        assert_eq!(doc.undo_stack.len(), 1);
        assert!(doc.dirty);
        // Undo restores the original section count.
        assert!(doc.undo());
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections.len(),
            original_len
        );
    }

    #[test]
    fn add_section_rejects_unknown_node() {
        let mut doc = load_test_doc();
        let err = doc.add_section("nonexistent", None, empty_section()).unwrap_err();
        assert!(err.contains("not found"), "got: {}", err);
    }

    #[test]
    fn add_section_rejects_out_of_bounds_aabb() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let mut s = empty_section();
        // Negative offset triggers the section-AABB validator.
        s.offset = Position { x: -10.0, y: 0.0 };
        let err = doc.add_section(&id, None, s).unwrap_err();
        assert!(err.contains("negative"), "got: {}", err);
    }

    #[test]
    fn delete_section_removes_at_index_returns_section() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Ensure at least 2 sections.
        doc.add_section(&id, None, empty_section()).unwrap();
        let len_before = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        let removed_text = doc.mindmap.nodes.get(&id).unwrap().sections[0].text.clone();
        let removed = doc.delete_section(&id, 0).expect("delete ok");
        assert_eq!(removed.text, removed_text);
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections.len(),
            len_before - 1
        );
    }

    #[test]
    fn delete_section_rejects_last_remaining() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Force down to one section.
        while doc.mindmap.nodes.get(&id).unwrap().sections.len() > 1 {
            doc.delete_section(&id, 0).unwrap();
        }
        let err = doc.delete_section(&id, 0).unwrap_err();
        assert!(err.contains("only section"), "got: {}", err);
    }

    #[test]
    fn delete_section_rejects_out_of_range() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        let err = doc.delete_section(&id, 9999).unwrap_err();
        assert!(err.contains("not found"), "got: {}", err);
    }

    #[test]
    fn delete_section_pushes_undo_and_dirty() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Set up: 2 sections so delete is allowed.
        doc.add_section(&id, None, empty_section()).unwrap();
        let len_after_add = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        doc.undo_stack.clear();
        doc.dirty = false;
        doc.delete_section(&id, 0).unwrap();
        assert_eq!(doc.undo_stack.len(), 1);
        assert!(doc.dirty);
        assert!(doc.undo());
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections.len(),
            len_after_add,
            "undo restores the deleted section"
        );
    }

    #[test]
    fn split_section_splits_text_at_grapheme() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Set up: replace section[0]'s text with a known string.
        doc.set_section_text(&id, 0, "abcdef".to_string());
        let new_idx = doc
            .split_section(&id, 0, Some(3))
            .expect("split ok");
        assert_eq!(new_idx, 1);
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;
        assert_eq!(sections[0].text, "abc");
        assert_eq!(sections[1].text, "def");
    }

    #[test]
    fn split_section_at_end_creates_empty_suffix() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.set_section_text(&id, 0, "abc".to_string());
        let new_idx = doc.split_section(&id, 0, None).expect("split ok");
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;
        assert_eq!(sections[0].text, "abc");
        assert_eq!(sections[new_idx].text, "");
    }

    #[test]
    fn split_section_handles_unicode_graphemes() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        // Family emoji (multi-codepoint grapheme cluster) + ascii.
        doc.set_section_text(&id, 0, "👨‍👩‍👧 hi".to_string());
        // Split between the family emoji and the space (idx 1).
        let _new_idx = doc.split_section(&id, 0, Some(1)).expect("split ok");
        let sections = &doc.mindmap.nodes.get(&id).unwrap().sections;
        assert_eq!(sections[0].text, "👨\u{200d}👩\u{200d}👧");
        assert_eq!(sections[1].text, " hi");
    }

    #[test]
    fn split_section_rejects_grapheme_past_end() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.set_section_text(&id, 0, "abc".to_string());
        let err = doc.split_section(&id, 0, Some(99)).unwrap_err();
        assert!(err.contains("grapheme"), "got: {}", err);
    }

    #[test]
    fn split_section_pushes_undo_and_dirty() {
        let mut doc = load_test_doc();
        let id = first_testament_node_id(&doc);
        doc.set_section_text(&id, 0, "abcdef".to_string());
        let len_before = doc.mindmap.nodes.get(&id).unwrap().sections.len();
        doc.undo_stack.clear();
        doc.dirty = false;
        doc.split_section(&id, 0, Some(3)).unwrap();
        assert_eq!(doc.undo_stack.len(), 1);
        assert!(doc.dirty);
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections.len(),
            len_before + 1
        );
        assert!(doc.undo());
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections.len(),
            len_before,
            "undo restores the pre-split section count"
        );
        assert_eq!(
            doc.mindmap.nodes.get(&id).unwrap().sections[0].text,
            "abcdef",
            "undo restores the original text"
        );
    }
}
