// SPDX-License-Identifier: MPL-2.0

//! The corpus held against the document. A console line edits only
//! through the one writer, so a line the console refuses — an unknown
//! key, a value that does not read, a verb its owner has not rebuilt
//! (V1-C6) — leaves the document as it was, its saved form and its
//! revision alike; and a line that edits is one history entry.

use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

use super::oracle::doc_for;
use super::oracle_corpus::EXEC_CORPUS;

/// The document as a save writes it.
fn saved(doc: &mut MindMapDocument) -> String {
    doc.save_json().expect("a fixture saves")
}

/// **A refused line changes nothing, and a line that changes the
/// document is one entry.** Every line of the oracle's corpus — which
/// names every verb the console has — runs on its fixture: one answered
/// with an error leaves the saved form and the revision as they were;
/// one that moved the revision added exactly one history entry.
///
/// Controls: the corpus holds both kinds — many refusals, and field
/// edits that apply — so neither half passes by finding none.
#[test]
fn test_a_refused_console_line_changes_nothing_and_an_edit_is_one_entry() {
    let (mut refused, mut edited) = (0, 0);
    for (sel, line) in EXEC_CORPUS {
        let ParseResult::Ok { cmd, args } = parse(line) else {
            continue;
        };
        let mut doc = doc_for(*sel);
        let (before, revision) = (saved(&mut doc), doc.revision());
        let entries = doc.doc().history().count();
        let mut effects = ConsoleEffects::new(&mut doc);
        let result = (cmd.execute)(&Args::new(&args), &mut effects);
        drop(effects);
        if matches!(result, ExecResult::Err(_)) {
            assert_eq!(
                doc.revision(),
                revision,
                "`{line}` was refused but moved the revision"
            );
            assert_eq!(
                saved(&mut doc),
                before,
                "`{line}` was refused but changed the document"
            );
            refused += 1;
        } else if doc.revision() != revision {
            assert_eq!(doc.doc().history().count(), entries + 1, "`{line}` is one entry");
            edited += 1;
        }
    }
    assert!(refused > 30, "the corpus refused {refused} lines");
    assert!(edited > 5, "the corpus edited with {edited} lines");
}
