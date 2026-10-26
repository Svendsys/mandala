// SPDX-License-Identifier: MPL-2.0

//! The loader's migration advice, held to the CLI it names (#199).
//!
//! When the app refuses a map for a legacy shape, the message tells
//! the user which `maptool` command fixes it. That message is the
//! one thing they have at the moment of refusal, and it lives in
//! baumhard while the argument handling it describes lives here, so
//! nothing but a test can keep the two in step. They were not: three
//! messages spelled `convert --<verb> <file>`, a one-path form every
//! `convert` verb refuses with `missing <out.json>`.
//!
//! Each [`CASES`] row produces a refusal from the real loader; the
//! test cuts the backticked `maptool ...` command out of the message
//! and runs it through [`run`] — the same dispatch `main` calls —
//! over the refused file. Then the file has to load. So a message
//! that names the wrong arity, the wrong verb, or a verb that does
//! not cure the shape it was raised for fails here, and the expected
//! outcome is the loader's own verdict rather than anything this
//! module computes.
//!
//! The rows are a hand-written list, so a second test holds them to
//! the source: every string literal in the shipped baumhard and app
//! code that carries a backticked `maptool` command has to be the
//! literal one row's refusal came from, and every row has to come
//! from one. A new advice message with no row fails there instead of
//! going unfollowed (TEST_CONVENTIONS §T13.1 shape 5).

use super::{run, CliError};
use baumhard::mindmap::loader::load_from_file;
use baumhard::util::doc_fixtures::repo_path;
use baumhard::util::rust_source::{production_code, string_literals};
use baumhard::util::test_temp::TempDir;
use regex::Regex;
use serde_json::Value;
use std::path::Path;

/// One legacy shape the loader refuses with `maptool` advice.
struct Case {
    /// Names the scratch directory and every failure message.
    label: &'static str,
    /// The part of the refusal only this shape produces (§T13.1
    /// shape 3), so a map refused for some other reason cannot pass.
    shape: &'static str,
    /// Plants the shape into an otherwise current map.
    break_it: fn(&mut Value),
}

const CASES: &[Case] = &[
    Case {
        label: "advice-portals",
        shape: "legacy `portals` field present",
        break_it: |json| json["portals"] = Value::Array(Vec::new()),
    },
    Case {
        label: "advice-node-text",
        shape: "legacy `text` / `text_runs`",
        break_it: |json| {
            let node = json["nodes"]["0"].as_object_mut().unwrap();
            node.remove("sections");
            node.insert("text".into(), Value::String("Lord God".into()));
        },
    },
    // `validate::zero_section_node`'s message, the one `maptool
    // verify` and the load-failure placard print too.
    Case {
        label: "advice-zero-sections",
        shape: "ships zero sections",
        break_it: |json| json["nodes"]["0"]["sections"] = Value::Array(Vec::new()),
    },
];

/// Write testament with `case`'s shape planted, and return the
/// directory holding it, its path, and the loader's refusal — after
/// demanding the refusal is for that shape.
fn refused(case: &Case) -> (TempDir, std::path::PathBuf, String) {
    let dir = TempDir::new(case.label);
    let path = dir.join("map.mindmap.json");
    let text = std::fs::read_to_string(repo_path("maps/testament.mindmap.json")).unwrap();
    let mut json: Value = serde_json::from_str(&text).unwrap();
    (case.break_it)(&mut json);
    std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap()).unwrap();

    let message =
        load_from_file(&path).expect_err(&format!("{}: the broken map must be refused", case.label));
    assert!(
        message.contains(case.shape),
        "{}: the map must be refused for the shape under test or this test proves nothing; \
         got: {message}",
        case.label
    );
    (dir, path, message)
}

/// The single backticked `maptool ...` span in `message`. Exactly
/// one: a message naming two commands, or none, is not advice this
/// harness can follow, and should say so rather than pass on the
/// first.
fn advised_command(message: &str) -> &str {
    let commands: Vec<&str> = message
        .split('`')
        .skip(1)
        .step_by(2)
        .filter(|span| span.starts_with("maptool "))
        .collect();
    assert_eq!(
        commands.len(),
        1,
        "expected exactly one backticked `maptool ...` command in: {message}"
    );
    commands[0]
}

/// Follow `message`'s advice on the map at `path`: every `<file>`
/// placeholder becomes `path`, and the rest goes to [`run`] as argv.
///
/// A placeholder this function does not know panics instead of being
/// passed through, so a message that grows a `<out.json>` is caught
/// here and taught, not run with a literal `<out.json>` argument.
fn follow_advice(message: &str, path: &Path) -> Result<(), CliError> {
    let path = path.to_str().unwrap();
    let mut words = advised_command(message).split_whitespace();
    assert_eq!(words.next(), Some("maptool"));
    let args: Vec<String> = words
        .map(|word| match word {
            "<file>" => path.to_string(),
            w if w.starts_with('<') && w.ends_with('>') => {
                panic!("unknown placeholder {w} in advice: {message}")
            }
            w => w.to_string(),
        })
        .collect();
    run(&args)
}

#[test]
fn test_every_advice_runs_and_cures_its_map() {
    for case in CASES {
        let (_dir, path, message) = refused(case);
        if let Err(e) = follow_advice(&message, &path) {
            panic!(
                "{}: the loader's advice failed as a maptool command: {e:?}\nmessage: {message}",
                case.label
            );
        }
        if let Err(e) = load_from_file(&path) {
            panic!(
                "{}: the map still does not load after following the advice: {e}\nmessage: {message}",
                case.label
            );
        }
    }
}

/// The harness itself must refuse the spelling #199 shipped: if
/// [`follow_advice`] accepted a one-path `convert`, every test above
/// would pass on the bug it exists to catch.
#[test]
fn test_follow_advice_refuses_a_one_path_convert() {
    let dir = TempDir::new("advice-one-path");
    let path = dir.join("map.mindmap.json");
    let one_path = "run `maptool convert --sections <file>` to migrate";
    match follow_advice(one_path, &path) {
        Err(CliError::Usage(m)) => assert!(
            m.contains("missing <out.json>"),
            "a one-path convert must be refused for its missing output, got: {m}"
        ),
        other => panic!("a one-path convert must be a usage error, got: {other:?}"),
    }
}

/// Every `.rs` file under `root`, repo-relative.
fn rust_files(root: &str, out: &mut Vec<String>) {
    let entries =
        std::fs::read_dir(repo_path(root)).unwrap_or_else(|e| panic!("{root} must be readable: {e}"));
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_str().unwrap();
        let relative = format!("{root}/{name}");
        if path.is_dir() {
            rust_files(&relative, out);
        } else if name.ends_with(".rs") {
            out.push(relative);
        }
    }
}

/// Whether `literal`, a format string as written in source, is the
/// one `message` was rendered from: with its `\`-newline
/// continuations joined, every run between its `{…}` placeholders
/// appears in `message`, in order.
fn renders(literal: &str, message: &str) -> bool {
    let joined = Regex::new(r"\\\n\s*").unwrap().replace_all(literal, "");
    let placeholder = Regex::new(r"\{[^{}]*\}").unwrap();
    let mut rest = message;
    for run in placeholder.split(&joined).filter(|r| !r.is_empty()) {
        match rest.find(run) {
            Some(at) => rest = &rest[at + run.len()..],
            None => return false,
        }
    }
    true
}

/// [`CASES`] is held to the source it stands for: every string
/// literal in shipped baumhard and app code that carries a backticked
/// `maptool` command is the literal exactly one row's refusal was
/// rendered from, and every row's refusal comes from exactly one.
///
/// Shipped code is [`production_code`]: comments and inline test
/// modules are cut. An out-of-line test file is read whole, so a
/// `maptool` literal in one shows up here as a stray — noise a reader
/// settles, never an advice message that goes unchecked.
#[test]
fn test_every_maptool_advice_in_the_source_has_a_case() {
    let mut files = Vec::new();
    rust_files("lib/baumhard/src", &mut files);
    rust_files("src", &mut files);
    assert!(
        files.iter().any(|f| f == "lib/baumhard/src/mindmap/loader.rs"),
        "the walk must reach the loader or this test reads nothing"
    );

    let messages: Vec<(&str, String)> = CASES.iter().map(|case| (case.label, refused(case).2)).collect();
    let mut matched = vec![0usize; CASES.len()];
    for file in &files {
        let code = production_code(file);
        for literal in string_literals(&code) {
            if !literal.contains("`maptool ") {
                continue;
            }
            let rows: Vec<usize> = (0..messages.len())
                .filter(|&i| renders(literal, &messages[i].1))
                .collect();
            assert_eq!(
                rows.len(),
                1,
                "{file}: this maptool advice must be the source of exactly one CASES row's refusal, \
                 matched {:?} — add a row that produces it:\n{literal}",
                rows.iter().map(|&i| messages[i].0).collect::<Vec<_>>()
            );
            matched[rows[0]] += 1;
        }
    }
    for (i, count) in matched.iter().enumerate() {
        assert_eq!(
            *count, 1,
            "{}: its refusal must come from exactly one maptool literal in shipped source, \
             found {count}: {}",
            messages[i].0, messages[i].1
        );
    }
}
