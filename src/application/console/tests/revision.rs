// SPDX-License-Identifier: MPL-2.0

//! The document revision against the whole console grammar.
//!
//! The revision is only worth carrying on the wire if it is
//! *complete*: a model change that does not advance it is a change an
//! agent polling on it never sees. `MindMapDocument::mark_dirty` makes
//! that structural for the setters, and this holds it for the surface
//! that reaches them: every line of the differential oracle's corpus,
//! which names every verb the console has, is run against a fixture
//! and the model is compared before and after.

use baumhard::mindmap::loader;

use crate::application::console::parser::{parse, Args, ParseResult};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

use super::oracle::doc_for;
use super::oracle_corpus::EXEC_CORPUS;

/// The model as the loader would write it: the one serialization of
/// a `MindMap`, so equal strings are equal models.
fn model(doc: &MindMapDocument) -> String {
    loader::to_json_value(&doc.mindmap)
        .expect("a fixture model serializes")
        .to_string()
}

#[test]
fn test_no_console_line_changes_the_model_without_advancing_the_revision() {
    let mut advanced = 0;
    let mut changed = 0;
    for (sel, line) in EXEC_CORPUS {
        let ParseResult::Ok { cmd, args } = parse(line) else {
            continue;
        };
        let mut doc = doc_for(*sel);
        let (before_model, before_revision) = (model(&doc), doc.revision());
        let mut effects = ConsoleEffects::new(&mut doc);
        let result = (cmd.execute)(&Args::new(&args), &mut effects);
        drop(effects);

        let model_changed = model(&doc) != before_model;
        let revision_advanced = doc.revision() > before_revision;
        changed += usize::from(model_changed);
        advanced += usize::from(revision_advanced);
        assert!(
            !model_changed || revision_advanced,
            "`{line}` changed the model and left the revision at {before_revision}"
        );
        assert!(
            doc.revision() >= before_revision,
            "`{line}` moved the revision backwards"
        );
        if matches!(result, ExecResult::Err(_)) {
            assert_eq!(
                doc.revision(),
                before_revision,
                "`{line}` failed, and a failed line commits nothing"
            );
        }
    }
    // The sweep means nothing if the corpus stopped reaching the
    // mutating verbs.
    assert!(
        changed >= 25 && advanced >= changed,
        "the corpus changed the model on {changed} lines and advanced the revision on {advanced}"
    );
}
