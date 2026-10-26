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
//! Each test below produces the refusal from the real loader, cuts
//! the backticked `maptool ...` command out of the message, and runs
//! it through [`run`] — the same dispatch `main` calls — over the
//! refused file. Then the file has to load. So a message that names
//! the wrong arity, the wrong verb, or a verb that does not cure the
//! shape it was raised for fails here, and the expected outcome is
//! the loader's own verdict rather than anything this module
//! computes.

use super::{run, CliError};
use baumhard::mindmap::loader::load_from_file;
use baumhard::util::test_temp::TempDir;
use serde_json::Value;
use std::path::{Path, PathBuf};

fn testament_json() -> Value {
    let mut p = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    p.pop(); // crates/maptool -> crates
    p.pop(); // crates -> root
    p.push("maps/testament.mindmap.json");
    serde_json::from_str(&std::fs::read_to_string(&p).unwrap()).unwrap()
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

/// Write testament with `break_it` applied, demand the loader refuse
/// it with a message containing `shape` (the part only this refusal
/// produces, §T13.1 shape 3), follow that message's advice, and
/// demand the file then loads.
fn assert_advice_cures(label: &str, shape: &str, break_it: impl FnOnce(&mut Value)) {
    let dir = TempDir::new(label);
    let path = dir.join("map.mindmap.json");
    let mut json = testament_json();
    break_it(&mut json);
    std::fs::write(&path, serde_json::to_string_pretty(&json).unwrap()).unwrap();

    let message = load_from_file(&path).expect_err("the broken map must be refused");
    assert!(
        message.contains(shape),
        "the map must be refused for the shape under test or this test proves nothing; got: {message}"
    );

    if let Err(e) = follow_advice(&message, &path) {
        panic!("the loader's advice failed as a maptool command: {e:?}\nmessage: {message}");
    }
    if let Err(e) = load_from_file(&path) {
        panic!("the map still does not load after following the advice: {e}\nmessage: {message}");
    }
}

#[test]
fn test_legacy_portals_advice_runs_and_cures_the_map() {
    assert_advice_cures("advice-portals", "legacy `portals` field present", |json| {
        json["portals"] = Value::Array(Vec::new());
    });
}

#[test]
fn test_legacy_node_text_advice_runs_and_cures_the_map() {
    assert_advice_cures("advice-node-text", "legacy `text` / `text_runs`", |json| {
        let node = json["nodes"]["0"].as_object_mut().unwrap();
        node.remove("sections");
        node.insert("text".into(), Value::String("Lord God".into()));
    });
}

/// The zero-section refusal is `validate::zero_section_node`'s
/// message, the one `maptool verify` and the load-failure placard
/// print too.
#[test]
fn test_zero_section_advice_runs_and_cures_the_map() {
    assert_advice_cures("advice-zero-sections", "ships zero sections", |json| {
        json["nodes"]["0"]["sections"] = Value::Array(Vec::new());
    });
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
