// SPDX-License-Identifier: MPL-2.0

//! Differential oracle over the whole console grammar.
//!
//! Three views of every verb — what it accepts, what it rejects and
//! with which exact words, and what its completion popup offers —
//! recorded as data so a refactor of the machinery underneath is
//! answerable in one place. The corpus lives in
//! [`super::oracle_corpus`]; the pinned answers in
//! [`super::oracle_expected`]; the comparison is the three tests at
//! the bottom of this file.
//!
//! This exists because the console's grammar was hand-written two
//! to four times per verb (parse, complete, usage, hints) and the
//! copies drifted. The oracle is what makes "the framework changed
//! nothing a user can see" a checkable claim rather than a hope.

use crate::application::console::completion::complete;
use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleContext, ConsoleEffects, ExecResult};
use crate::application::document::tests_common::{load_test_doc, pinned_two_section_node};
use crate::application::document::{EdgeLabelSel, EdgeRef, MindMapDocument, SectionSel, SelectionState};

/// Selection fixtures the corpus runs each line under.
#[derive(Clone, Copy)]
pub enum Sel {
    None,
    Node,
    TwoSectionNode,
    Section,
    Multi,
    Edge,
    EdgeLabel,
}

pub fn doc_for(sel: Sel) -> MindMapDocument {
    match sel {
        Sel::None => load_test_doc(),
        Sel::Node => {
            let mut doc = load_test_doc();
            let id = doc.mindmap.nodes.keys().min().cloned().expect("nodes");
            doc.selection = SelectionState::Single(id);
            doc
        }
        Sel::TwoSectionNode => {
            let (mut doc, id) = pinned_two_section_node();
            doc.selection = SelectionState::Single(id);
            doc
        }
        Sel::Section => {
            let (mut doc, id) = pinned_two_section_node();
            doc.selection = SelectionState::Section(SectionSel {
                node_id: id,
                section_idx: 0,
            });
            doc
        }
        Sel::Multi => {
            let mut doc = load_test_doc();
            let mut ids: Vec<String> = doc.mindmap.nodes.keys().cloned().collect();
            ids.sort();
            ids.truncate(2);
            doc.selection = SelectionState::Multi(ids);
            doc
        }
        Sel::Edge => {
            let mut doc = load_test_doc();
            let e = doc.mindmap.edges.first().expect("edges").clone();
            doc.selection = SelectionState::Edge(EdgeRef::new(&e.from_id, &e.to_id, &e.edge_type));
            doc
        }
        Sel::EdgeLabel => {
            let mut doc = load_test_doc();
            let e = doc.mindmap.edges.first().expect("edges").clone();
            doc.selection = SelectionState::EdgeLabel(EdgeLabelSel::new(EdgeRef::new(
                &e.from_id,
                &e.to_id,
                &e.edge_type,
            )));
            doc
        }
    }
}

/// Run one line and render its outcome as a single stable string.
pub fn exec_signature(line: &str, sel: Sel) -> String {
    let mut doc = doc_for(sel);
    let (cmd, tokens) = match parse(line) {
        ParseResult::Ok { cmd, args } => (cmd, args),
        ParseResult::Empty => return "PARSE-EMPTY".to_string(),
        ParseResult::Unknown(s) => return format!("PARSE-UNKNOWN {s}"),
    };
    let mut eff = ConsoleEffects::new(&mut doc);
    let res = (cmd.execute)(&Args::new(&tokens), &mut eff);
    match res {
        ExecResult::Ok(m) => format!("OK {}", escape(&m)),
        ExecResult::Lines(ls) => format!(
            "LINES {}",
            escape(&ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n"))
        ),
        ExecResult::Err(m) => format!("ERR {}", escape(&m)),
    }
}

/// Render the completion popup for `line` (cursor at end) as a
/// single stable string.
pub fn completion_signature(line: &str, sel: Sel) -> String {
    let doc = doc_for(sel);
    let ctx = ConsoleContext::from_document(&doc);
    let out = complete(line, line.len(), &ctx);
    if out.is_empty() {
        return "-".to_string();
    }
    out.iter()
        .map(|c| {
            let mut s = c.text.clone();
            if c.display != c.text {
                s.push_str(&format!("/{}", c.display));
            }
            if let Some(h) = &c.hint {
                s.push_str(&format!(" [{}]", h));
            }
            if let Some(f) = &c.font_family {
                s.push_str(&format!(" <{}>", f));
            }
            escape(&s)
        })
        .collect::<Vec<_>>()
        .join(" ; ")
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('\n', "\\n")
}

/// Cut a measured readout down to its host-independent head.
pub fn stable_head(sig: &str) -> &str {
    match sig.find("\\nsize:") {
        Some(i) => &sig[..i],
        None => sig,
    }
}

/// Every corpus line's outcome still reads exactly the way it was
/// pinned. Positional: expectation N answers corpus line N.
#[test]
fn test_console_oracle_execution_outcomes_are_unchanged() {
    let corpus = super::oracle_corpus::EXEC_CORPUS;
    let expected = super::oracle_expected::EXEC_EXPECTED;
    assert_eq!(
        corpus.len(),
        expected.len(),
        "EXEC_CORPUS and EXEC_EXPECTED must stay the same length"
    );
    for (i, ((sel, line), want)) in corpus.iter().zip(expected).enumerate() {
        assert_eq!(
            &exec_signature(line, *sel),
            want,
            "oracle row {i} drifted for input {line:?}"
        );
    }
}

/// The measured readouts still open the way they were pinned. Only
/// the host-independent head is compared — see
/// [`super::oracle_corpus::EXEC_PREFIX_CORPUS`].
#[test]
fn test_console_oracle_measured_readouts_open_unchanged() {
    let corpus = super::oracle_corpus::EXEC_PREFIX_CORPUS;
    let expected = super::oracle_expected::EXEC_PREFIX_EXPECTED;
    assert_eq!(corpus.len(), expected.len());
    for (i, ((sel, line), want)) in corpus.iter().zip(expected).enumerate() {
        let got = exec_signature(line, *sel);
        assert_eq!(
            stable_head(&got),
            *want,
            "oracle prefix row {i} drifted for input {line:?}"
        );
    }
}

/// Every corpus line's completion popup still offers exactly the
/// rows, hints and insert texts it was pinned with.
#[test]
fn test_console_oracle_completions_are_unchanged() {
    let corpus = super::oracle_corpus::COMPLETION_CORPUS;
    let expected = super::oracle_expected::COMPLETION_EXPECTED;
    assert_eq!(
        corpus.len(),
        expected.len(),
        "COMPLETION_CORPUS and COMPLETION_EXPECTED must stay the same length"
    );
    for (i, ((sel, line), want)) in corpus.iter().zip(expected).enumerate() {
        assert_eq!(
            &completion_signature(line, *sel),
            want,
            "oracle completion row {i} drifted for input {line:?}"
        );
    }
}

/// The two vocabularies the pinned tables deliberately leave out —
/// they enumerate the *host's* installed fonts, so a byte-pin would
/// assert something about the machine rather than about the
/// console. Pinned structurally instead.
#[test]
fn test_console_oracle_font_vocabularies_track_the_loaded_families() {
    let families: Vec<&str> = baumhard::font::fonts::loaded_families_iter().collect();
    assert!(
        !families.is_empty(),
        "the font system must be initialized for this fixture"
    );

    let doc = doc_for(Sel::Node);
    let ctx = ConsoleContext::from_document(&doc);
    let rows = complete("border font ", "border font ".len(), &ctx);
    assert_eq!(
        rows.iter().map(|c| c.display.as_str()).collect::<Vec<_>>(),
        families,
        "`border font <TAB>` offers one row per loaded family"
    );
    assert!(
        rows.iter().all(|c| c.font_family.is_some()),
        "each family row shapes its own label"
    );

    match exec_signature("font list", Sel::Node).strip_prefix("LINES ") {
        Some(body) => assert_eq!(body.split("\\n").count(), families.len()),
        other => panic!("`font list` should emit Lines, got {other:?}"),
    }
}
