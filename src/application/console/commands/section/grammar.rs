// SPDX-License-Identifier: MPL-2.0

//! `section …`'s grammar.
//!
//! `move` and `resize` each declare **two forms**, because
//! their shapes are genuinely alternatives rather than a menu.
//! `section move` takes `dx=`/`dy=` *or* `x=`/`y=`; `section resize`
//! takes the `fill` literal *or* `w=`/`h=`. Bracketing either pair
//! onto one line would document a command the verb refuses, so
//! `help` prints one line per form.
//!
//! The two are not enforced the same way, and the difference is
//! where the alternative is *written*. `resize`'s lives in a slot —
//! only one of its forms declares the `fill` literal — so the engine
//! narrows on it and `section resize fill w=99` is refused by name
//! (`spec::Form::admits_prefix`). `move`'s lives only in its key
//! lists, which an empty positional list admits equally, so the
//! engine offers and accepts their union, and refusing the mix,
//! naming both shapes, is the verb body's — a handwritten guard, one
//! of the two bespoke semantics issue #27's fix plan names as staying
//! behind the table.
//!
//! This level is also where `KNOWN_VERBS` used to live: a second
//! nine-entry vocabulary beside `VERBS`, in a different order, that
//! the compiler could not hold against it. There is one vocabulary
//! now, and the unknown-subverb listing is derived from it.

use baumhard::document::element::DocElement;

use crate::application::console::completion::Completion;
use crate::application::console::spec::{free, Form, Grammar, Key, Slot, Subverb, Vocabulary, Word};
use crate::application::console::ConsoleContext;

/// The `fill` literal `section resize` takes in place of a size:
/// clear the minimum size so the section fits its content (D5).
///
/// It renamed `none`, which read as "remove the section". One word,
/// so usage prints it bare rather than as a one-item choice.
const FILL_WORD: &[Word] = &[Word::new(
    "fill",
    "clear the minimum size; the section fits its text",
)];

/// The alignments `section align` takes.
const ALIGN_WORDS: &[Word] = &[
    Word::bare("left"),
    Word::bare("center"),
    Word::bare("right"),
    Word::new("reset", "inherit"),
];

/// The `section=<idx>` key: which section of a selected node a line
/// reaches, when no section is selected.
const SECTION_KEY: Key = Key::new(
    "section",
    "which section of the selected node",
    Vocabulary::Rows {
        placeholder: "idx",
        rows: section_idx_completions,
        sentinels: &[],
    },
);

/// The geometry, text and structure keys, and `section=`.
const SECTION_OWN_KEYS: &[Key] = &[
    Key::new("dx", "relative move along x axis (canvas units)", free("f64")),
    Key::new("dy", "relative move along y axis (canvas units)", free("f64")),
    Key::new("x", "absolute x offset within parent node", free("f64")),
    Key::new("y", "absolute y offset within parent node", free("f64")),
    Key::new("w", "section width (canvas units)", free("f64")),
    Key::new("h", "section height (canvas units)", free("f64")),
    Key::new(
        "text",
        "section text payload (quote multi-word values)",
        free("text"),
    ),
    Key::new("at", "insertion / split index", free("idx")),
    SECTION_KEY,
];

/// Every subverb, in the order the popup offers them.
///
/// All are [`Subverb::gated`]: an unquoted multi-word kv value
/// splits into two tokens whose second can spell a subverb name, so
/// a kv at or before the subverb slot means the line is kv form
/// whatever the positional happens to say. `section text=hello
/// world` is the live instance — it used to answer "unknown subverb
/// 'world'" and now suggests the quoting the user meant.
const SUBVERBS: &[Subverb] = &[
    Subverb::bare(
        "move",
        "geometry",
        "shift section offset (dx/dy delta or x/y absolute)",
    )
    .taking(&[
        Form::keys(&["dx", "dy"], &["section"]),
        Form::keys(&["x", "y"], &["section"]),
    ])
    .gated(),
    Subverb::bare(
        "resize",
        "geometry",
        "pin section size (w/h), or clear it to fit the text",
    )
    .taking(&[
        Form::keys(&["w", "h"], &["section"]),
        Form::slots(&[Slot::req(Vocabulary::Words(FILL_WORD))]).reading(&["section"]),
    ])
    .gated(),
    Subverb::bare("align", "geometry", "align the section's lines in its box")
        .taking(&[Form::slots(&[Slot::req(Vocabulary::Words(ALIGN_WORDS))]).reading(&["section"])])
        .gated(),
    Subverb::bare("show", "readout", "print the per-section properties")
        .taking(&[Form::opt(&["section"])])
        .gated(),
    Subverb::bare("text", "text", "replace section text with one span")
        .taking(&[Form::slots(&[Slot::opt(free("text"))]).reading(&["text", "section"])])
        .gated(),
    Subverb::bare(
        "edit",
        "editor",
        "open the section text editor on the resolved target",
    )
    .taking(&[Form::opt(&["section"])])
    .gated(),
    Subverb::bare("add", "structure", "insert a new section")
        .taking(&[Form::opt(&["at", "text"])])
        .gated(),
    Subverb::bare("delete", "structure", "remove the section")
        .taking(&[Form::opt(&["section"])])
        .gated(),
    Subverb::bare(
        "split",
        "structure",
        "split a section in two at a grapheme boundary",
    )
    .taking(&[Form::keys(&["at"], &["section"])])
    .gated(),
];

/// `section …`. No bare form: every operation names a subverb, which
/// is why an unknown first word answers with the subverb listing
/// rather than with a kv rejection.
pub static SECTION: Grammar = Grammar {
    label: "section",
    subverb_sets: &[SUBVERBS],
    key_sets: &[SECTION_OWN_KEYS],
    bare: None,
};

/// The popup for `section=<TAB>`: one row per section of the first
/// selected node, each hinted with a short preview of that section's
/// text so the user can tell which is which.
///
/// Grapheme-aware truncation via `take_graphemes` — a preview cut
/// at 20 *bytes* would slice a multi-codepoint emoji in half.
pub(crate) fn section_idx_completions(ctx: &ConsoleContext, partial: &str) -> Vec<Completion> {
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
            let text = crate::application::view::inspect::element_text(doc, section);
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
