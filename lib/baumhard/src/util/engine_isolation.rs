// SPDX-License-Identifier: MPL-2.0

//! The engine is not glyph-bound, held against the tree: no file under
//! `lib/baumhard/src/engine/` — its tests included, since
//! `engine::tests` is as reachable from `engine::` as the walker is —
//! names a glyph module (`gfx_structs`, `mindmap`, `font`) or a glyph
//! type in code. Prose may: comments are stripped and string literals
//! blanked before the search, so a doc comment that explains the glyph
//! instantiation is not an import of it.
//!
//! The reader is lexical — it compares whole identifiers, so
//! `GfxElementish` is not `GfxElement` — and deliberately blunt: a
//! needle in code is reported whether it is a `use`, a path or a type
//! annotation, because each one ties the engine to a glyph type the
//! same way.
//!
//! Test-only and native-only, for the same reasons
//! [`crate::util::source_scan`] is.

use crate::util::doc_fixtures::repo_path;
use crate::util::rust_source::{blank_string_literals, strip_comments};
use std::path::{Path, PathBuf};

/// Identifiers no engine file may write in code: the glyph-side
/// modules, and the glyph types an engine API could otherwise be
/// tempted to name.
const FORBIDDEN: &[&str] = &[
    "gfx_structs",
    "mindmap",
    "font",
    "GfxElement",
    "GfxElementField",
    "GfxMutation",
    "GfxMutator",
    "GfxTree",
    "GfxMutatorTree",
    "GlyphArea",
    "GlyphModel",
    "CellField",
];

/// Every `(line, identifier)` in `source` that is a [`FORBIDDEN`]
/// identifier in code. Comments are stripped and string literals
/// blanked first; line numbers are 1-based and survive both.
fn forbidden_references(source: &str) -> Vec<(usize, &'static str)> {
    let code = blank_string_literals(&strip_comments(source));
    let mut found = Vec::new();
    for (index, line) in code.lines().enumerate() {
        for word in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
            if let Some(needle) = FORBIDDEN.iter().find(|n| **n == word) {
                found.push((index + 1, *needle));
            }
        }
    }
    found
}

/// Every `.rs` file under `dir`, recursively, sorted.
fn rust_files(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let entries =
            std::fs::read_dir(&next).unwrap_or_else(|e| panic!("{} must be readable: {e}", next.display()));
        for entry in entries {
            let path = entry.expect("directory entry is readable").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|e| e == "rs") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **`baumhard::engine` names no glyph module and no glyph type.**
    /// The failure lists every `file:line: identifier`; the fix is to
    /// move the dependency to the glyph instantiation in `gfx_structs`
    /// and reach it through the engine's traits.
    #[test]
    fn test_the_engine_names_no_glyph_module_or_type() {
        let root = repo_path("lib/baumhard/src/engine");
        let files = rust_files(&root);
        for witness in ["walker.rs", "builder/build.rs", "tests/test_element.rs"] {
            assert!(
                files.iter().any(|f| f.ends_with(witness)),
                "the scan must read engine/{witness}; it read {files:?}"
            );
        }
        let mut offenders = Vec::new();
        for file in &files {
            let source = std::fs::read_to_string(file)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
            for (line, needle) in forbidden_references(&source) {
                offenders.push(format!("{}:{line}: {needle}", file.display()));
            }
        }
        assert!(
            offenders.is_empty(),
            "the engine names glyph items:\n{}",
            offenders.join("\n")
        );
    }

    /// The reader the gate above trusts, held to a planted import and
    /// to the two places a needle may legitimately appear. Without
    /// this, a reader that matched nothing would pass the gate.
    #[test]
    fn test_the_isolation_scan_flags_code_and_spares_prose() {
        let planted = "use crate::gfx_structs::element::GfxElement;\nfn f(_: &GlyphArea) {}\n";
        assert_eq!(
            forbidden_references(planted),
            vec![(1, "gfx_structs"), (1, "GfxElement"), (2, "GlyphArea")]
        );

        let prose = "/// The glyph instantiation is `GfxElement` in `gfx_structs`.\n\
                     // and GlyphArea in a line comment\n\
                     const S: &str = \"mindmap::GfxMutation\";\n\
                     fn font_size(_: GfxElementish) {}\n";
        assert_eq!(forbidden_references(prose), vec![]);
    }
}
