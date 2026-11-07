// SPDX-License-Identifier: MPL-2.0

//! `new [path]` — start a fresh, blank map.
//!
//! Like `open`, refuses to discard a dirty document, and refuses a path
//! that is not a `.mandala.json`. Without a path the new map is unbound
//! — `save` will require an explicit path before it can write. With a
//! path, the blank map is also written immediately so the binding is
//! real on disk and `Ctrl+S` works from then on. The new document keeps
//! the current one's history limit (D18).

use crate::application::console::parser::Args;
use crate::application::console::predicates::always;
use crate::application::console::spec::descent::descend;
use crate::application::console::spec::{free, kvs, Bare, Form, Grammar, Slot};
use crate::application::console::{ConsoleEffects, ExecResult};
use crate::application::document::MindMapDocument;

use super::Command;

pub static GRAMMAR: Grammar = Grammar {
    label: "new",
    subverb_sets: &[],
    key_sets: &[],
    bare: Some(Bare::new("file", &[Form::slots(&[Slot::opt(free("path"))])])),
};

pub const COMMAND: Command = Command {
    name: "new",
    aliases: &[],
    summary: "Start a new blank map, replacing the current one",
    applicable: always,
    grammar: &GRAMMAR,
    synonyms: &["blank", "create", "file"],
    execute: execute_new,
};

fn execute_new(args: &Args, eff: &mut ConsoleEffects) -> ExecResult {
    let descent = descend(&GRAMMAR, args.tokens());
    if let Err(msg) = kvs::read_strict(&descent, args) {
        return ExecResult::err(msg);
    }
    if eff.document().is_dirty() {
        return ExecResult::err("unsaved changes; save before starting a new map");
    }
    let path = args.positional(0).map(|p| p.to_string());
    let mut doc = MindMapDocument::new_blank(path.clone(), eff.document().history_limit());
    if let Some(ref p) = path {
        if let Err(e) = doc.save_to(p) {
            return ExecResult::err(format!("new: {e}"));
        }
    }
    eff.side_effect = Some(super::super::ConsoleSideEffect::ReplaceDocument(Box::new(doc)));
    match path {
        Some(p) => ExecResult::ok_msg(format!("new map at {}", p)),
        None => ExecResult::ok_msg("new map (no file path; use `save <path>` to bind one)"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::console::tests::fixtures::assert_exec_err_contains;
    use crate::application::console::ConsoleSideEffect;
    use baumhard::util::test_temp::TempDir;

    /// Run `new` with `args` against `doc`, returning the result and
    /// any side effect.
    fn run_new(args: &[&str], doc: &mut MindMapDocument) -> (ExecResult, Option<ConsoleSideEffect>) {
        let tokens: Vec<String> = args.iter().map(|s| (*s).to_string()).collect();
        let mut eff = ConsoleEffects::new(doc);
        let result = execute_new(&Args::new(&tokens), &mut eff);
        let side = eff.side_effect.take();
        (result, side)
    }

    /// Unwrap the replacement document a successful `new` hands the
    /// dispatcher, panicking with the offending variant otherwise.
    fn replacement(side: Option<ConsoleSideEffect>) -> MindMapDocument {
        match side {
            Some(ConsoleSideEffect::ReplaceDocument(doc)) => *doc,
            other => panic!("expected ReplaceDocument, got {:?}", other),
        }
    }

    /// A dirty document is never discarded, and the guard runs *before*
    /// the write — so a rejected `new <path>` also leaves the filesystem
    /// alone.
    ///
    /// Negative control: the same call once the document is saved
    /// (clean) succeeds, which pins the rejection on the dirty state
    /// rather than on the path or a verb that never succeeds.
    #[test]
    fn test_new_on_a_dirty_document_is_refused_before_anything_is_written() {
        let dir = TempDir::new("console-new-dirty");
        let path = dir.join("fresh.mandala.json");
        let path_str = path.to_string_lossy().to_string();

        let mut doc = MindMapDocument::new_blank(None, 50);
        doc.create_root_for_test();
        let (result, side) = run_new(&[&path_str], &mut doc);

        assert_exec_err_contains(result, "unsaved changes");
        assert!(side.is_none(), "a refused new must not replace the document");
        assert!(
            !path.exists(),
            "the dirty guard must run before the write; {} was created",
            path.display()
        );

        doc.save_to(&dir.join("mine.mandala.json").to_string_lossy())
            .expect("the document saves");
        let (result, side) = run_new(&[&path_str], &mut doc);
        assert!(
            matches!(result, ExecResult::Ok(_)),
            "control: a clean document must accept new, got {result:?}"
        );
        assert!(side.is_some(), "control: a clean new must replace the document");
        assert!(path.exists(), "control: a clean new must write the file");
    }

    /// `new <path>` binds the replacement to that path and makes the
    /// binding real on disk: the bytes written load back as a map, and
    /// the replacement is clean.
    #[test]
    fn test_new_with_a_path_writes_the_blank_map_and_binds_it() {
        let dir = TempDir::new("console-new-bound");
        let path = dir.join("fresh.mandala.json");
        let path_str = path.to_string_lossy().to_string();

        let mut doc = MindMapDocument::new_blank(None, 9);
        let (result, side) = run_new(&[&path_str], &mut doc);
        match result {
            ExecResult::Ok(msg) => assert!(msg.contains(&path_str), "{msg:?}"),
            other => panic!("expected Ok, got {other:?}"),
        }
        let replaced = replacement(side);
        assert_eq!(replaced.file_path.as_deref(), Some(path_str.as_str()));
        assert!(!replaced.is_dirty(), "a map just written is clean");
        assert_eq!(replaced.history_limit(), 9, "the limit carries over");
        MindMapDocument::load(&path_str, 50).expect("the written blank map reloads");
    }

    /// A path that is not a `.mandala.json` is refused by name, and
    /// nothing is written.
    #[test]
    fn test_new_refuses_another_extension() {
        let dir = TempDir::new("console-new-extension");
        let path = dir.join("fresh.mindmap.json");
        let mut doc = MindMapDocument::new_blank(None, 50);
        let (result, side) = run_new(&[&path.to_string_lossy()], &mut doc);
        assert_exec_err_contains(result, ".mandala.json");
        assert!(side.is_none());
        assert!(!path.exists());
    }

    /// No path: an unbound blank map, and nothing on disk.
    #[test]
    fn test_new_without_a_path_is_unbound() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        let (result, side) = run_new(&[], &mut doc);
        assert!(matches!(result, ExecResult::Ok(_)));
        assert_eq!(replacement(side).file_path, None);
    }

    /// A write that fails is reported and the current document
    /// survives — the user is left with the map they had, not with a
    /// blank one bound to a file that does not exist. The path is inside
    /// a directory that was never created, so the write cannot stage.
    /// Control: the same name in the directory that exists is written
    /// and replaces the document.
    #[test]
    fn test_new_reports_a_failed_write_and_keeps_the_current_document() {
        let dir = TempDir::new("console-new-unwritable");
        let path = dir.join("no-such-dir").join("fresh.mandala.json");
        let mut doc = MindMapDocument::new_blank(None, 50);
        let (result, side) = run_new(&[&path.to_string_lossy()], &mut doc);
        assert_exec_err_contains(result, "new:");
        assert!(
            side.is_none(),
            "a new whose write failed must not replace the document"
        );
        assert!(!path.exists());

        let good = dir.join("fresh.mandala.json");
        let (result, side) = run_new(&[&good.to_string_lossy()], &mut doc);
        assert!(matches!(result, ExecResult::Ok(_)), "{result:?}");
        assert!(
            side.is_some() && good.exists(),
            "control: a writable path is written"
        );
    }
}
