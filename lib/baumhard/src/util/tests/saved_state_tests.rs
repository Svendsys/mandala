// SPDX-License-Identifier: MPL-2.0

//! No production code records the saved state but the save path
//! (design §8.4's gate, in the writer's model). Whether a map has
//! unsaved work is derived: the writer compares its history with the
//! position `Document::mark_saved` last recorded, so no code can set
//! "dirty" — the one write is `mark_saved`, and the one production
//! caller is `MindMapDocument::save_to`, after the file is written. A
//! stray call would make unsaved work read as saved, past the guard
//! that keeps `open` and `new` from discarding it. The compiler cannot
//! hold that, since `mark_saved` is public for the writer's own tests;
//! this scan of the app's and the tools' sources does.

use std::path::PathBuf;

use crate::util::source_scan::{
    production_source, relative_to_repo, test_gated_module_files, workspace_rust_sources,
};

/// The file that holds the one allowed call.
const SAVE_PATH: &str = "src/application/document/mod.rs";

/// The writer's method that records the saved state.
const NAME: &str = "mark_saved";

/// Where [`saved_state_sites`] found `mark_saved` named in one file, by
/// 1-based line.
#[derive(Debug, Default, PartialEq)]
struct Sites {
    /// In the body of `MindMapDocument::save_to`, in [`SAVE_PATH`].
    save_to: Vec<usize>,
    /// Anywhere else.
    stray: Vec<usize>,
}

/// Every mention of the identifier `mark_saved` in `source`, the
/// production text of the file at the repo-relative path `rel`, read as
/// syntax: a method call, a path (`Document::mark_saved(&mut doc)`,
/// `<Document>::mark_saved`, a function pointer), a `use`, an item of
/// that name, or a token inside a macro's arguments. A mention counts
/// as the save path only inside the body of `save_to` in an inherent
/// `impl MindMapDocument` in [`SAVE_PATH`], and not inside an item
/// nested in that body. Doc comments are not syntax and are not read.
/// Panics when `source` does not parse.
fn saved_state_sites(rel: &str, source: &str) -> Sites {
    use syn::visit::{self, Visit};

    struct Scan {
        save_path: bool,
        in_document_impl: bool,
        in_save_to: bool,
        found: Sites,
    }

    impl Scan {
        fn hit(&mut self, line: usize) {
            if self.in_save_to {
                self.found.save_to.push(line);
            } else {
                self.found.stray.push(line);
            }
        }

        /// Visit an item nested where `save_to`'s body or the document's
        /// impl would otherwise still be in force: it is neither.
        fn outside<'a, T>(&mut self, item: &'a T, visit: impl FnOnce(&mut Self, &'a T)) {
            let outer = (self.in_document_impl, self.in_save_to);
            self.in_document_impl = false;
            self.in_save_to = false;
            visit(self, item);
            (self.in_document_impl, self.in_save_to) = outer;
        }
    }

    impl<'ast> Visit<'ast> for Scan {
        fn visit_item_impl(&mut self, item: &'ast syn::ItemImpl) {
            let document = self.save_path
                && item.trait_.is_none()
                && matches!(
                    &*item.self_ty,
                    syn::Type::Path(ty) if ty.qself.is_none()
                        && ty.path.segments.last().is_some_and(|s| s.ident == "MindMapDocument")
                );
            self.outside(item, |scan, item| {
                scan.in_document_impl = document;
                visit::visit_item_impl(scan, item);
            });
        }

        fn visit_impl_item_fn(&mut self, item: &'ast syn::ImplItemFn) {
            let save_to = self.in_document_impl && item.sig.ident == "save_to";
            let outer = std::mem::replace(&mut self.in_save_to, save_to);
            visit::visit_impl_item_fn(self, item);
            self.in_save_to = outer;
        }

        fn visit_item_fn(&mut self, item: &'ast syn::ItemFn) {
            self.outside(item, visit::visit_item_fn);
        }

        fn visit_item_mod(&mut self, item: &'ast syn::ItemMod) {
            self.outside(item, visit::visit_item_mod);
        }

        fn visit_item_trait(&mut self, item: &'ast syn::ItemTrait) {
            self.outside(item, visit::visit_item_trait);
        }

        fn visit_ident(&mut self, ident: &'ast proc_macro2::Ident) {
            if ident == NAME {
                self.hit(ident.span().start().line);
            }
        }

        fn visit_macro(&mut self, mac: &'ast syn::Macro) {
            let mut stack = vec![mac.tokens.clone()];
            while let Some(tokens) = stack.pop() {
                for token in tokens {
                    match token {
                        proc_macro2::TokenTree::Ident(ident) if ident == NAME => {
                            self.hit(ident.span().start().line)
                        }
                        proc_macro2::TokenTree::Group(group) => stack.push(group.stream()),
                        _ => {}
                    }
                }
            }
            visit::visit_macro(self, mac);
        }
    }

    let parsed = syn::parse_file(source).unwrap_or_else(|e| panic!("{rel} must parse as Rust: {e}"));
    let mut scan = Scan {
        save_path: rel == SAVE_PATH,
        in_document_impl: false,
        in_save_to: false,
        found: Sites::default(),
    };
    scan.visit_file(&parsed);
    scan.found.save_to.sort_unstable();
    scan.found.stray.sort_unstable();
    scan.found
}

/// The production sources of the app (`src/`) and the tools
/// (`crates/`): no `tests` directory and no file a `#[cfg(test)] mod`
/// names.
fn app_and_tool_sources() -> Vec<(PathBuf, String)> {
    let files = workspace_rust_sources();
    let test_only = test_gated_module_files(&files);
    files
        .into_iter()
        .filter_map(|file| {
            let rel = relative_to_repo(&file);
            let shipped = (rel.starts_with("src/") || rel.starts_with("crates/"))
                && !file.components().any(|c| c.as_os_str() == "tests")
                && !test_only.contains(&file);
            shipped.then_some((file, rel))
        })
        .collect()
}

/// **No production code sets the unsaved state directly**: across the
/// app's and the tools' production sources, `mark_saved` is named
/// exactly once, in the body of `MindMapDocument::save_to`.
///
/// Controls: the walk reads more than a hundred sources, `crates/`
/// among them; a second method of `MindMapDocument` in the save path's
/// own file that calls it is found; so are the call as a path
/// (`Document::mark_saved(&mut doc)`), with a qualified self, as a
/// function pointer, inside a macro and inside a function nested in
/// `save_to`'s body; a `save_to` of another type, or in another file,
/// is not the save path; and the real `save_to` call is the one found.
#[test]
fn test_no_production_code_sets_dirty_directly() {
    let mut scanned = 0;
    let mut tools = 0;
    let mut save_to = Vec::new();
    let mut stray = Vec::new();
    for (file, rel) in app_and_tool_sources() {
        scanned += 1;
        if rel.starts_with("crates/") {
            tools += 1;
        }
        let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{rel} must be readable: {e}"));
        let found = saved_state_sites(&rel, &production_source(&file, &text));
        save_to.extend(found.save_to.into_iter().map(|line| format!("{rel}:{line}")));
        stray.extend(found.stray.into_iter().map(|line| format!("{rel}:{line}")));
    }
    assert!(
        scanned > 100,
        "control: the walk found only {scanned} production sources"
    );
    assert!(tools > 0, "control: the walk reads the tools under crates/");
    assert!(
        stray.is_empty(),
        "these record the saved state outside `MindMapDocument::save_to`:\n  {}",
        stray.join("\n  ")
    );
    assert_eq!(
        save_to.len(),
        1,
        "the save path calls `mark_saved` once: {save_to:?}"
    );
    assert!(save_to[0].starts_with(SAVE_PATH), "{save_to:?}");

    let sites = |rel: &str, source: &str| saved_state_sites(rel, source);
    assert_eq!(
        sites(
            SAVE_PATH,
            "impl MindMapDocument {\n    fn save_to(&mut self) {\n        self.doc.mark_saved();\n    }\n    fn sneak(&mut self) {\n        self.doc.mark_saved();\n    }\n}"
        ),
        Sites {
            save_to: vec![3],
            stray: vec![6],
        },
        "control: a second method in the save path's own file is found, and save_to's call is not"
    );
    const ELSEWHERE: &str = "src/application/app/x.rs";
    for (form, source) in [
        (
            "a method call",
            "fn f(d: &mut Document) {\n    d.mark_saved();\n}",
        ),
        (
            "the UFCS form",
            "fn f(d: &mut Document) {\n    Document::mark_saved(d);\n}",
        ),
        (
            "a qualified self",
            "fn f(d: &mut Document) {\n    <Document>::mark_saved(d);\n}",
        ),
        (
            "a function pointer",
            "fn f() {\n    let _ = Document::mark_saved;\n}",
        ),
        (
            "a macro's arguments",
            "fn f(d: &mut Document) {\n    log::info!(\"{:?}\", d.mark_saved());\n}",
        ),
    ] {
        assert_eq!(
            sites(ELSEWHERE, source),
            Sites {
                save_to: vec![],
                stray: vec![2],
            },
            "control: {form} outside the save path is found"
        );
        assert_eq!(
            sites(SAVE_PATH, source).stray,
            vec![2],
            "control: {form} in the save path's file, outside save_to, is found"
        );
    }
    assert_eq!(
        sites(
            SAVE_PATH,
            "impl MindMapDocument {\n    fn save_to(&mut self) {\n        fn sneak(d: &mut Document) {\n            Document::mark_saved(d);\n        }\n    }\n}"
        )
        .stray,
        vec![4],
        "control: a function nested in save_to's body is not the save path"
    );
    assert_eq!(
        sites(
            SAVE_PATH,
            "impl Other {\n    fn save_to(&mut self) {\n        self.mark_saved();\n    }\n}"
        )
        .stray,
        vec![3],
        "control: another type's save_to is not the save path"
    );
    assert_eq!(
        sites(
            ELSEWHERE,
            "impl MindMapDocument {\n    fn save_to(&mut self) {\n        self.doc.mark_saved();\n    }\n}"
        )
        .stray,
        vec![3],
        "control: a save_to in another file is not the save path"
    );
}
