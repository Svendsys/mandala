// SPDX-License-Identifier: MPL-2.0

//! `open <path>` — replace the current map with one loaded from disk.
//!
//! Refuses to load over a dirty document so unsaved work is never
//! silently discarded — the user has to `save` (or `save <path>`)
//! first — and refuses a path that is not a `.mandala.json`, naming the
//! extension it wanted (`document::extension_refusal`). The dispatcher
//! consumes the `ReplaceDocument` effect: it installs the document and
//! projects it whole (`view::MapView::load`). The new document keeps the
//! current one's history limit, which `settings.json` set (D18).

use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{free, kvs, usage, Bare, Form, Grammar, Slot};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

use super::Command;

pub static GRAMMAR: Grammar = Grammar {
    label: "open",
    subverb_sets: &[],
    key_sets: &[],
    bare: Some(Bare::new("file", &[Form::slots(&[Slot::req(free("path"))])])),
};

pub const COMMAND: Command = Command {
    name: "open",
    aliases: &[],
    summary: "Open a .mandala.json map, replacing the current one",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["load", "file"],
    execute: execute_open,
};

fn execute_open(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    let path = match args.positional(0) {
        Some(p) => p.to_string(),
        None => return ExecResult::err(usage::no_arguments_message(&GRAMMAR)),
    };
    if eff.document().is_dirty() {
        return ExecResult::err("unsaved changes; save before opening another map");
    }
    match MindMapDocument::load(&path, eff.document().history_limit()) {
        Ok(doc) => {
            eff.side_effect = Some(super::super::ConsoleSideEffect::ReplaceDocument(Box::new(doc)));
            ExecResult::ok_msg(format!("opened {}", path))
        }
        // The loader's message verbatim, prefixed with the verb.
        // `MindMapDocument::load` reports nothing on its own — every
        // caller is a different surface, and this one is the console.
        Err(e) => ExecResult::err(format!("open: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::assert_exec_err_contains;
    use crate::application::console::ConsoleSideEffect;
    use baumhard::util::test_temp::TempDir;

    /// Run `open <path>` against `doc`, returning the result and any
    /// side effect. The effect has to be lifted out before the
    /// `ConsoleEffects` borrow of `doc` ends.
    fn run_open(path: &str, doc: &mut MindMapDocument) -> (ExecResult, Option<ConsoleSideEffect>) {
        let tokens = vec![path.to_string()];
        let mut eff = ConsoleEffects::new(doc);
        let result = execute_open(&Args::new(&tokens), &mut eff);
        let side = eff.side_effect.take();
        (result, side)
    }

    /// A map on disk to open: a blank one saved at `name` in `dir`.
    fn a_saved_map(dir: &TempDir, name: &str) -> String {
        let path = dir.join(name).to_string_lossy().to_string();
        MindMapDocument::new_blank(None, 50)
            .save_to(&path)
            .expect("a blank map saves");
        path
    }

    /// A rejected `open` puts the loader's own message in the console
    /// and leaves the current document alone.
    #[test]
    fn test_open_of_a_rejected_map_reports_the_loader_message() {
        let dir = TempDir::new("console-open-rejected");
        let path = dir.join("broken.mandala.json");
        std::fs::write(&path, "{ this is not json").expect("seed the broken map");
        let path_str = path.to_string_lossy().to_string();

        let mut doc = MindMapDocument::new_blank(None, 50);
        let (result, side) = run_open(&path_str, &mut doc);
        match result {
            ExecResult::Err(message) => assert!(message.contains("broken.mandala.json"), "{message}"),
            other => panic!("a broken map must be refused: {other:?}"),
        }
        assert!(side.is_none(), "a rejected open must not replace the document");
    }

    /// A path that is not a `.mandala.json` is refused by name, before
    /// anything is read — even a file that is a map.
    #[test]
    fn test_open_refuses_another_extension_naming_the_one_it_wants() {
        let dir = TempDir::new("console-open-extension");
        let good = a_saved_map(&dir, "map.mandala.json");
        let other = dir.join("map.json");
        std::fs::copy(&good, &other).expect("the same bytes under another name");

        let mut doc = MindMapDocument::new_blank(None, 50);
        let (result, side) = run_open(&other.to_string_lossy(), &mut doc);
        assert_exec_err_contains(result, ".mandala.json");
        assert!(side.is_none());
        // Control: the same bytes under the map extension open.
        let (result, side) = run_open(&good, &mut doc);
        assert!(matches!(result, ExecResult::Ok(_)), "{result:?}");
        assert!(side.is_some());
    }

    /// The dirty guard is the only thing between a typo'd `open` and
    /// losing unsaved work, and it is checked before the loader runs: a
    /// dirty document is refused even when the path names a map that
    /// would have loaded cleanly.
    ///
    /// Negative control: saving the same document (clean again) and
    /// repeating the same call succeeds, which pins the rejection on the
    /// dirty state rather than on the path or the document.
    #[test]
    fn test_open_on_a_dirty_document_is_refused_before_the_load() {
        let dir = TempDir::new("console-open-dirty");
        let path = a_saved_map(&dir, "other.mandala.json");
        let mut doc = MindMapDocument::new_blank(None, 50);
        doc.create_root_for_test();
        assert!(doc.is_dirty());

        let (result, side) = run_open(&path, &mut doc);
        assert_exec_err_contains(result, "unsaved changes");
        assert!(side.is_none(), "a refused open must not replace the document");

        doc.save_to(&dir.join("mine.mandala.json").to_string_lossy())
            .expect("the dirty document saves");
        let (result, side) = run_open(&path, &mut doc);
        assert!(
            matches!(result, ExecResult::Ok(_)),
            "control: a clean document must accept the same open, got {result:?}"
        );
        assert!(side.is_some(), "control: a clean open must replace the document");
    }

    /// The opened document keeps the current one's history limit — the
    /// limit `settings.json` set at startup.
    #[test]
    fn test_open_keeps_the_history_limit() {
        let dir = TempDir::new("console-open-limit");
        let path = a_saved_map(&dir, "limit.mandala.json");
        let mut doc = MindMapDocument::new_blank(None, 7);
        let (_, side) = run_open(&path, &mut doc);
        let Some(ConsoleSideEffect::ReplaceDocument(opened)) = side else {
            panic!("the open replaces the document");
        };
        assert_eq!(opened.history_limit(), 7);
        assert_eq!(opened.file_path.as_deref(), Some(path.as_str()));
    }
}
