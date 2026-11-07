// SPDX-License-Identifier: MPL-2.0

//! The corpus held against the document. Every editing verb is rebuilt
//! on the one writer by its owner (V1-C5, V1-C6) and refuses until then,
//! so no line of the differential oracle's corpus — which names every
//! verb the console has — may change the document: neither its saved
//! form nor its revision.

use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::ConsoleEffects;
use crate::application::document::MindMapDocument;

use super::oracle::doc_for;
use super::oracle_corpus::EXEC_CORPUS;

/// The document as a save writes it.
fn saved(doc: &mut MindMapDocument) -> String {
    doc.save_json().expect("a fixture saves")
}

#[test]
fn test_no_console_line_changes_the_document() {
    let mut ran = 0;
    for (sel, line) in EXEC_CORPUS {
        let ParseResult::Ok { cmd, args } = parse(line) else {
            continue;
        };
        let mut doc = doc_for(*sel);
        let (before, revision) = (saved(&mut doc), doc.revision());
        let mut effects = ConsoleEffects::new(&mut doc);
        let _ = (cmd.execute)(&Args::new(&args), &mut effects);
        drop(effects);
        assert_eq!(doc.revision(), revision, "`{line}` moved the revision");
        assert_eq!(saved(&mut doc), before, "`{line}` changed the document");
        ran += 1;
    }
    assert!(ran > 50, "the corpus reached {ran} verbs");
    // Control: a real change moves both, so the comparison can fail.
    let mut doc = doc_for(super::oracle::Sel::None);
    let (before, revision) = (saved(&mut doc), doc.revision());
    doc.create_root_for_test();
    assert_ne!(doc.revision(), revision);
    assert_ne!(saved(&mut doc), before);
}
