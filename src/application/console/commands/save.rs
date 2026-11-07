// SPDX-License-Identifier: MPL-2.0

//! `save [path]` — write the current map to disk.
//!
//! No args: writes to the document's bound `file_path`. Errors if no
//! path is bound (e.g. after `new` without a path).
//!
//! With a path: writes there and rebinds the document to the new path,
//! so subsequent saves (including `Ctrl+S`) target the new file. Mirrors
//! the "Save As" gesture in conventional editors — the original file on
//! disk is left untouched. A path that is not a `.mandala.json` is
//! refused by name. The write is `MindMapDocument::save_to`: the
//! document's `save_json`, written through a temporary file and a
//! rename, then `mark_saved`.

use super::Command;
use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{free, kvs, Bare, Form, Grammar, Slot};
use crate::application::console::{ConsoleEffects, ExecResult};

/// The path slot offers no rows: paths are free-form and the console
/// does not (yet) shell out to a filesystem walker. Declaring it is
/// still what puts `save [<path>]` in `help save`.
pub static GRAMMAR: Grammar = Grammar {
    label: "save",
    subverb_sets: &[],
    key_sets: &[],
    bare: Some(Bare::new("file", &[Form::slots(&[Slot::opt(free("path"))])])),
};

pub const COMMAND: Command = Command {
    name: "save",
    aliases: &[],
    summary: "Save the current map to disk",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["write", "persist", "file"],
    execute: execute_save,
};

fn execute_save(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    let target_path: String = match args.positional(0) {
        Some(p) => p.to_string(),
        None => match &eff.document().file_path {
            Some(p) => p.clone(),
            None => {
                return ExecResult::err("no file path bound; use `save <path>` to choose one");
            }
        },
    };
    match eff.document_mut().save_to(&target_path) {
        Ok(()) => ExecResult::ok_msg(format!("saved to {}", target_path)),
        Err(e) => ExecResult::err(format!("save: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::assert_exec_err_contains;
    use crate::application::document::selection::every_node;
    use crate::application::document::MindMapDocument;
    use baumhard::util::test_temp::TempDir;

    /// Run `save` with `args` against `doc`. `save` produces no side
    /// effect, so only the result comes back — the interesting
    /// outcomes are on `doc` and on disk.
    fn run_save(args: &[&str], doc: &mut MindMapDocument) -> ExecResult {
        let tokens: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut eff = ConsoleEffects::new(doc);
        execute_save(&Args::new(&tokens), &mut eff)
    }

    /// A document with no bound path is told so rather than guessing a
    /// filename, and stays dirty — the work is still unwritten.
    ///
    /// Negative control: binding a real path to the *same* document and
    /// repeating the *same* argument-free call succeeds and leaves it
    /// clean, which pins the rejection on the missing binding.
    #[test]
    fn test_save_without_a_bound_path_reports_it_and_stays_dirty() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        doc.create_root_for_test();

        assert_exec_err_contains(run_save(&[], &mut doc), "no file path bound");
        assert!(
            doc.is_dirty(),
            "a save that never wrote must leave the document dirty"
        );

        let dir = TempDir::new("console-save-unbound");
        let path = dir.join("bound.mandala.json");
        doc.file_path = Some(path.to_string_lossy().to_string());
        assert!(
            matches!(run_save(&[], &mut doc), ExecResult::Ok(_)),
            "control: a bound document must accept an argument-free save"
        );
        assert!(
            !doc.is_dirty(),
            "control: a save that wrote leaves the document clean"
        );
        assert!(path.exists(), "control: the bound save must write the file");
    }

    /// `save <path>` is Save As: it writes there, rebinds the document,
    /// leaves it clean, and leaves the file the document *was* bound to
    /// exactly as it found it.
    #[test]
    fn test_save_to_a_new_path_rebinds_and_leaves_the_original_untouched() {
        let dir = TempDir::new("console-save-as");
        let original = dir.join("original.mandala.json");
        let target = dir.join("target.mandala.json");

        let mut doc = MindMapDocument::new_blank(Some(original.to_string_lossy().to_string()), 50);
        assert!(matches!(run_save(&[], &mut doc), ExecResult::Ok(_)));
        let original_bytes = std::fs::read(&original).expect("the original must exist to be compared");

        // Diverge the in-memory map from what is on disk, so "the
        // original is untouched" has an observable value to hold.
        doc.create_root_for_test();
        assert!(doc.is_dirty());

        let target_str = target.to_string_lossy().to_string();
        assert!(matches!(run_save(&[&target_str], &mut doc), ExecResult::Ok(_)));

        assert_eq!(
            doc.file_path.as_deref(),
            Some(target_str.as_str()),
            "save <path> must rebind the document to the path it wrote"
        );
        assert!(!doc.is_dirty(), "a successful save must leave the document clean");
        assert_eq!(
            std::fs::read(&original).expect("the original must survive a Save As"),
            original_bytes,
            "save <path> must not write through to the previously bound file"
        );
        let written = MindMapDocument::load(&target_str, 50).expect("the Save As target must reload");
        assert_eq!(
            every_node(written.doc()).count(),
            1,
            "the target holds the node made after the first save"
        );
    }

    /// A failed write is reported, and the document keeps both its
    /// binding and its unsaved changes — the user's next `Ctrl+S` still
    /// targets the file they chose, and still has work to write. The
    /// input is a bound path inside a directory that was never created.
    #[test]
    fn test_save_that_cannot_write_keeps_the_binding_and_the_dirty_state() {
        let dir = TempDir::new("console-save-unwritable");
        let bound = dir.join("no-such-dir").join("map.mandala.json");
        let bound_str = bound.to_string_lossy().to_string();

        let mut doc = MindMapDocument::new_blank(Some(bound_str.clone()), 50);
        doc.create_root_for_test();

        assert_exec_err_contains(run_save(&[], &mut doc), "could not write");
        assert_eq!(doc.file_path.as_deref(), Some(bound_str.as_str()));
        assert!(doc.is_dirty(), "a failed save must leave the document dirty");
    }

    /// `save <path>` with another extension is refused by name, and the
    /// document keeps its binding.
    #[test]
    fn test_save_refuses_another_extension() {
        let dir = TempDir::new("console-save-extension");
        let target = dir.join("map.json");
        let mut doc = MindMapDocument::new_blank(None, 50);
        assert_exec_err_contains(run_save(&[&target.to_string_lossy()], &mut doc), ".mandala.json");
        assert!(!target.exists());
        assert_eq!(doc.file_path, None);
    }
}
