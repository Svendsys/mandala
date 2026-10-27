// SPDX-License-Identifier: MPL-2.0

//! Names an author chooses (design B.8): a theme variant, a theme
//! variable, a palette, and the id of an edit, a presentation or a verb.
//! Any non-empty string without a control or bidi character, compared
//! byte for byte; quoted in a path when it is not a plain identifier.

use crate::document::element::PaletteData;
use crate::document::format::{check_savable, load, LoadErrorKind, SaveErrorKind};
use crate::document::place::{name_fault, NameFault};
use crate::document::tests::format_text::{map_with, rejection};
use crate::document::tests::test_support::DocumentBuilder;

/// A map whose theme, palettes and definitions all use `name`, written
/// as it would be inside a JSON string (escapes allowed), with one of
/// them chosen by `slot` and the others plain.
fn map_naming(slot: usize, name: &str) -> String {
    let pick = |n: usize, plain: &str| {
        if n == slot {
            name.to_string()
        } else {
            plain.to_string()
        }
    };
    let (variant, variable, palette, edit, presentation, verb) = (
        pick(0, "d"),
        pick(1, "bg"),
        pick(2, "p"),
        pick(3, "e"),
        pick(4, "s"),
        pick(5, "v"),
    );
    format!(
        r##"{{"format":1,"name":"t","theme":{{"active":"{variant}","variants":{{"{variant}":{{"{variable}":"#000000"}}}}}},"palettes":{{"{palette}":[{{"fill":"#000000","frame":"#000000","text":"#000000"}}]}},"edits":[{{"id":"{edit}","name":"E"}}],"presentations":[{{"id":"{presentation}","name":"S"}}],"verbs":[{{"id":"{verb}","name":"V"}}]}}"##
    )
}

#[test]
fn test_a_name_is_not_empty_and_holds_no_control_or_bidi_character() {
    // The rule, shared with the mutations.
    assert_eq!(name_fault(""), Some(NameFault::Empty));
    assert_eq!(name_fault("a\u{7}"), Some(NameFault::NotPrintable('\u{7}')));
    assert_eq!(name_fault("a\u{9b}b"), Some(NameFault::NotPrintable('\u{9b}')));
    assert_eq!(
        name_fault("x\u{202E}y"),
        Some(NameFault::NotPrintable('\u{202E}'))
    );
    assert_eq!(
        name_fault("x\u{2067}y"),
        Some(NameFault::NotPrintable('\u{2067}'))
    );
    assert_eq!(name_fault("My Palette"), None);
    assert_eq!(name_fault("ünïcode-név"), None);

    // In a file, at each kind of name, with the path of the name.
    let paths = [
        (0, r#"theme.variants[""]"#, r#"theme.variants["a\u0007"]"#),
        (1, r#"theme.variants.d[""]"#, r#"theme.variants.d["a\u0007"]"#),
        (2, r#"palettes[""]"#, r#"palettes["a\u0007"]"#),
        (3, "edits[0].id", "edits[0].id"),
        (4, "presentations[0].id", "presentations[0].id"),
        (5, "verbs[0].id", "verbs[0].id"),
    ];
    for (slot, empty_at, control_at) in paths {
        let error = rejection(&map_naming(slot, ""));
        assert_eq!(
            (error.path.as_str(), &error.kind),
            (empty_at, &LoadErrorKind::EmptyName),
            "{slot}"
        );
        let error = rejection(&map_naming(slot, r"a\u0007"));
        assert_eq!(
            (error.path.as_str(), &error.kind),
            (control_at, &LoadErrorKind::NameNotPrintable { code_point: 7 }),
            "{slot}"
        );
        let error = rejection(&map_naming(slot, "x\u{202E}y"));
        assert_eq!(
            error.kind,
            LoadErrorKind::NameNotPrintable { code_point: 0x202E },
            "{slot}"
        );
        // Control: the same map with a plain name there loads.
        assert!(load(&map_naming(slot, "plain")).is_ok(), "{slot}");
    }
}

#[test]
fn test_a_name_that_is_not_an_identifier_is_quoted_in_a_path() {
    // "My Palette" is a name, and an error inside it says so in quotes.
    let palette = |name: &str, fill: &str| {
        map_with(
            &format!(
                r##","palettes":{{"{name}":[{{"fill":"{fill}","frame":"#000000","text":"#000000"}}]}}"##
            ),
            "",
        )
    };
    assert!(load(&palette("My Palette", "#000000")).is_ok());
    let error = rejection(&palette("My Palette", "var(nope)"));
    assert_eq!(error.path, r#"palettes["My Palette"][0].fill"#);
    // Control: a plain name is written bare.
    let error = rejection(&palette("plain", "var(nope)"));
    assert_eq!(error.path, "palettes.plain[0].fill");
}

#[test]
fn test_a_document_holding_a_name_the_file_refuses_cannot_be_saved() {
    // check_savable asks the loader's own rule, at the name's path.
    let saved = |name: &str| {
        let mut builder = DocumentBuilder::new();
        builder.palette(PaletteData {
            name: name.to_string(),
            groups: Vec::new(),
        });
        check_savable(&builder.build().expect("a document"))
    };
    let error = saved("").expect_err("an empty name has no spelling");
    assert_eq!(error.path, r#"palettes[""]"#);
    assert!(
        matches!(&error.kind, SaveErrorKind::NotLoadable { error } if error.kind == LoadErrorKind::EmptyName),
        "{error:?}"
    );
    let error = saved("a\u{1b}").expect_err("a control character has no place in a name");
    assert!(
        matches!(&error.kind, SaveErrorKind::NotLoadable { error } if error.kind == LoadErrorKind::NameNotPrintable { code_point: 0x1b }),
        "{error:?}"
    );
    // Control: a plain one saves.
    assert_eq!(saved("plain"), Ok(()));
}

#[test]
fn test_each_keyed_map_hashes_an_id_with_a_key_of_its_own() {
    // An id a file chose lands somewhere no file can predict: two maps
    // hash the same id differently, and neither as an unkeyed hash does.
    use crate::document::keyed::KeyedState;
    use std::hash::BuildHasher;
    let (a, b) = (KeyedState::new(), KeyedState::new());
    let ids = [7u32, 1 << 16, 2 << 16, 3 << 16];
    let hashes = |state: &KeyedState| ids.map(|id| state.hash_one(id));
    assert_ne!(hashes(&a), hashes(&b));
    let unkeyed = rustc_hash::FxBuildHasher;
    assert_ne!(hashes(&a), ids.map(|id| unkeyed.hash_one(id)));
    // Control: one state hashes one id the same way every time.
    assert_eq!(hashes(&a), hashes(&a));
}

/// Set in a child of the test below: print the first state's hash of 7.
const PRINT_HASH: &str = "MANDALA_PRINT_KEYED_HASH";

#[test]
fn test_the_hash_key_is_drawn_anew_in_each_process() {
    // Two states of one process differ by their number alone, so that
    // says nothing about whether a file can predict the key. Two
    // processes can: the first state of each hashes 7 with the key it
    // drew, and with a key a file could know they would agree.
    use crate::document::keyed::KeyedState;
    use std::hash::BuildHasher;
    if std::env::var_os(PRINT_HASH).is_some() {
        println!("KEYED {}", KeyedState::new().hash_one(7u32));
        return;
    }
    let exe = std::env::current_exe().expect("the test binary");
    let hash = || {
        let out = std::process::Command::new(&exe)
            .args([
                "--exact",
                "document::tests::format_names_tests::test_the_hash_key_is_drawn_anew_in_each_process",
                "--nocapture",
                "--test-threads=1",
            ])
            .env(PRINT_HASH, "1")
            .output()
            .expect("the test binary runs");
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        stdout
            .lines()
            .find_map(|line| line.split("KEYED ").nth(1).map(|hash| hash.trim().to_string()))
            .unwrap_or_else(|| panic!("the child printed no hash:\n{stdout}"))
    };
    let (first, second) = (hash(), hash());
    assert_ne!(
        first, second,
        "two processes hashed 7 alike: the key is not secret"
    );
}
