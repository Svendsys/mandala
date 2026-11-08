// SPDX-License-Identifier: MPL-2.0

//! The old model is gone from the repository, not only from the build:
//! no file names `mindmap::` except the planted strings two scanners'
//! own tests feed them, and the design history under `work_plans/`.
//! A path that compiles cannot name a deleted module, but prose and
//! string literals can, and nothing else would notice.

use std::path::{Path, PathBuf};

use crate::util::doc_fixtures::repo_path;

/// The occurrences of `mindmap::` that are planted test inputs, by
/// file and count: each is a string literal a scanner's own test hands
/// it to prove it would catch the old model coming back.
const PLANTED: &[(&str, usize)] = &[
    ("lib/baumhard/src/util/document_surface.rs", 1),
    ("lib/baumhard/src/util/engine_isolation.rs", 1),
    ("lib/baumhard/src/util/tests/old_model_tests.rs", 6),
];

/// Directories the walk does not enter: build output, vendored
/// sources, the design history, version control, fuzz artifacts, and
/// other checkouts nested under `.claude/worktrees/`.
const SKIPPED_DIRS: &[&str] = &[
    "target",
    "vendor",
    "dist",
    "work_plans",
    ".git",
    "corpus",
    "artifacts",
    "worktrees",
    "node_modules",
];

/// The files of the repository a person reads or the build compiles:
/// Rust, Markdown, TOML, shell, JSON, YAML and HTML.
fn repository_files() -> Vec<PathBuf> {
    fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();
            if path.is_dir() {
                if !SKIPPED_DIRS.contains(&name.as_str()) {
                    walk(&path, out);
                }
            } else if ["rs", "md", "toml", "sh", "json", "yml", "yaml", "html"]
                .iter()
                .any(|ext| name.ends_with(&format!(".{ext}")))
            {
                out.push(path);
            }
        }
    }
    let mut out = Vec::new();
    walk(&repo_path(""), &mut out);
    out
}

/// Each file of `files` (repo-relative, with its text) that names the
/// old model more often than [`PLANTED`] allows it, with the count.
fn old_model_mentions(files: &[(String, String)]) -> Vec<String> {
    files
        .iter()
        .filter_map(|(relative, text)| {
            let found = text.matches("mindmap::").count();
            let allowed = PLANTED
                .iter()
                .find(|(file, _)| file == relative)
                .map_or(0, |(_, count)| *count);
            (found > allowed).then(|| format!("{relative}: {found} (planted: {allowed})"))
        })
        .collect()
}

/// **No file names the deleted `mindmap::` model** outside the planted
/// scanner inputs and `work_plans/`.
///
/// Controls: the walk reads the repository (more than five hundred
/// files, this one among them), and a planted mention in any other
/// file is found.
#[test]
fn test_no_file_names_the_deleted_mindmap_model() {
    let root = repo_path("");
    let files: Vec<(String, String)> = repository_files()
        .into_iter()
        .filter_map(|path| {
            let text = std::fs::read_to_string(&path).ok()?;
            let relative = path
                .strip_prefix(&root)
                .ok()?
                .to_string_lossy()
                .replace('\\', "/");
            Some((relative, text))
        })
        .collect();
    assert!(
        files.len() > 500
            && files
                .iter()
                .any(|(relative, _)| relative.ends_with("old_model_tests.rs")),
        "precondition: the walk read the repository ({} files)",
        files.len()
    );
    let found = old_model_mentions(&files);
    assert!(
        found.is_empty(),
        "these name the deleted old model (`mindmap::`); say what is true of the document instead:\n  {}",
        found.join("\n  ")
    );
    let planted = vec![(
        "src/application/x.rs".to_string(),
        "// see `mindmap::loader`".to_string(),
    )];
    assert_eq!(
        old_model_mentions(&planted),
        vec!["src/application/x.rs: 1 (planted: 0)".to_string()],
        "control: a mention in any other file is found"
    );
}
