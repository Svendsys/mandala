// SPDX-License-Identifier: MPL-2.0

//! The shared `section=N` / `range=A..B` targeting vocabulary — the two
//! [`Key`] declarations the levels that speak them splice into their
//! own key sets, and [`section_idx_completions`], the popup those same
//! verbs offer on the value side of `section=`. One declaration each,
//! so the surfaces that carry `section=` describe it one way and answer
//! it one way.
//!
//! They live here because `color`, `font` and `section` all speak these
//! two kvs. What went wrong before was not a copied value list but a
//! *missing* one: each verb's `KvValue` arm matched against its whole
//! `KEYS` list and answered with the single vocabulary it happened to
//! know, so `font section=<TAB>` offered point sizes while `color
//! section=<TAB>` offered color presets. Giving a shared kv one shared
//! answer is what retired the class.
//!
//! The parsers that read these keys went with the verbs' old-model
//! bodies; V1-C5 (#246) deletes this vocabulary with those keys and
//! replaces it with the `range` verb.

use baumhard::arena::NodeId;
use baumhard::document::element::DocElement;
use baumhard::document::ids::Document;

use crate::application::console::completion::Completion;
use crate::application::console::spec::{free, Key, Vocabulary};
use crate::application::console::ConsoleContext;

/// Both shared targeting keys, for the levels that take a grapheme
/// range as well as a section index — `color` and `font`.
pub(super) const TARGET_KEYS: &[Key] = &[SECTION_KEY, RANGE_KEY];

/// The `section=<idx>` targeting key, declared once for every level
/// that speaks it.
///
/// A level splices this slice into its own `key_sets` rather than
/// re-declaring it, which is what stops the popups that surface
/// `section=` from coming to describe it several ways. It is also why
/// `section=<TAB>` answers with section indices at every one: each
/// verb's hand-written `KvValue` arm used to match its *whole* key
/// list and answer with the single vocabulary it happened to know, so
/// `font section=<TAB>` offered point sizes and `color section=<TAB>`
/// offered color names.
pub(super) const SECTION_KEYS: &[Key] = &[SECTION_KEY];

const RANGE_KEY: Key = Key::new(
    "range",
    "grapheme range A..B inside the targeted section",
    free("A..B"),
);

const SECTION_KEY: Key = Key::new(
    "section",
    "target section index inside a multi-section node",
    Vocabulary::Rows {
        placeholder: "idx",
        rows: section_idx_rows,
        sentinels: &[],
    },
);

/// [`section_idx_completions`] in the shape a [`Vocabulary`] wants.
fn section_idx_rows(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    section_idx_completions(ctx, partial)
}

/// The text of `section`: its spans' text, in order. One allocation.
fn section_text(doc: &Document, section: NodeId) -> String {
    doc.children(section)
        .filter_map(|child| match doc.get(child) {
            Some(DocElement::Span(span)) => Some(span.text.as_str()),
            _ => None,
        })
        .collect()
}

/// The popup for `section=<TAB>`: one row per section of the first
/// selected node, each hinted with a short preview of that section's
/// text so the user can tell which is which.
///
/// Grapheme-aware truncation via `take_graphemes` — a preview cut
/// at 20 *bytes* would slice a multi-codepoint emoji in half.
pub(super) fn section_idx_completions(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
    use baumhard::util::grapheme_chad::take_graphemes;
    let doc = ctx.document.doc();
    let Some(node) = ctx.document.selection.nodes(doc).next() else {
        return Vec::new();
    };
    doc.children(node)
        .filter(|&child| matches!(doc.get(child), Some(DocElement::Section(_))))
        .enumerate()
        .filter(|(idx, _)| idx.to_string().starts_with(partial))
        .map(|(idx, section)| {
            // Empty sections render `(empty)` so the row isn't a bare
            // bullet.
            let text = section_text(doc, section);
            let (preview, overflow) = take_graphemes(&text, 20);
            let hint = if preview.is_empty() {
                "(empty)".to_string()
            } else if overflow {
                format!("\"{}…\"", preview)
            } else {
                format!("\"{}\"", preview)
            };
            Completion {
                text: idx.to_string(),
                display: idx.to_string(),
                hint: Some(hint),
                font_family: None,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::MindMapDocument;

    /// `section=<TAB>` lists the first selected node's sections by
    /// index, hinted with each one's text, and filters on the partial.
    #[test]
    fn test_section_completion_lists_the_selected_nodes_sections() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        doc.create_root_for_test();
        let node = crate::application::document::selection::every_node(doc.doc())
            .next()
            .expect("the map has the node just created");
        let sections = doc
            .doc()
            .children(node)
            .filter(|&c| matches!(doc.doc().get(c), Some(DocElement::Section(_))))
            .count();
        assert!(sections > 0, "a new node has a section");
        // Control: with nothing selected there is nothing to list.
        assert!(section_idx_completions(&ConsoleContext::from_document(&doc), "").is_empty());
        doc.selection.select_only(node);
        let rows = section_idx_completions(&ConsoleContext::from_document(&doc), "");
        assert_eq!(rows.len(), sections);
        assert_eq!(rows[0].text, "0");
        assert!(rows[0].hint.is_some());
        assert!(section_idx_completions(&ConsoleContext::from_document(&doc), "9").is_empty());
    }
}
