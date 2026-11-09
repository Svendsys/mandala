// SPDX-License-Identifier: MPL-2.0

//! The D28 helper — [`MindMapDocument::begin`] holds the one open
//! transaction, [`MindMapDocument::apply`] reports what it refuses in
//! one place, and the handle is forgotten once the writer closed what
//! it named — and the repository's converted maps, which load and save
//! back to the bytes they were read from.

use baumhard::document::mutation::compose;
use baumhard::document::writer::error::ApplyError;
use baumhard::document::writer::transaction::TransactionKind;
use baumhard::document::writer::{ChangeSource, EditRequest};

use super::{MindMapDocument, TRANSACTION_OPEN_STATUS};

/// A request that creates a root node: a real edit the writer accepts
/// whenever no transaction holds the document.
fn create_root(doc: &MindMapDocument) -> EditRequest {
    EditRequest {
        steps: compose::create_root(doc.doc(), glam::Vec2::new(10.0, 10.0)).expect("a root composes"),
        label: "test: create root".to_string(),
    }
}

/// **An open interactive transaction refuses every other change, and the
/// refusal is reported once, on the status line.** `begin` holds the
/// handle; `apply` while it is open returns `TransactionOpen`, leaves the
/// revision where it was and sets [`TRANSACTION_OPEN_STATUS`]; a second
/// `begin` is refused the same way.
///
/// Control: the same request applies before the transaction opens, and
/// clears the status line when it does — so a writer that refused
/// everything, or a helper that never set the status, fails here.
#[test]
fn test_an_open_transaction_refuses_an_apply_and_says_so_on_the_status_line() {
    let mut doc = MindMapDocument::new_blank(None, 50);
    let req = create_root(&doc);
    let applied = doc
        .apply(req, ChangeSource::Action)
        .expect("the control: no transaction is open");
    assert_eq!(doc.revision(), applied);
    assert_eq!(doc.status(), None, "an accepted change leaves no status");

    assert!(
        doc.begin(
            ChangeSource::Input,
            TransactionKind::Interactive,
            crate::application::document::Interaction::Test
        ),
        "begin opens one"
    );
    assert!(doc.transaction.is_some(), "and the helper holds its handle");
    let req = create_root(&doc);
    assert_eq!(
        doc.apply(req, ChangeSource::Action),
        Err(ApplyError::TransactionOpen),
        "the open transaction refuses the apply"
    );
    assert_eq!(doc.revision(), applied, "the refused apply changed nothing");
    assert_eq!(doc.status(), Some(TRANSACTION_OPEN_STATUS), "and said so");

    doc.status = None;
    assert!(
        !doc.begin(
            ChangeSource::Input,
            TransactionKind::Interactive,
            crate::application::document::Interaction::Test
        ),
        "a second begin is refused while one is open"
    );
    assert_eq!(
        doc.status(),
        Some(TRANSACTION_OPEN_STATUS),
        "and says so the same way"
    );
    assert!(doc.transaction.is_some(), "the first handle is still held");
}

/// **An undo cancels an open interactive transaction and the helper
/// forgets the stale handle** (ruling R36), so the next `begin` and the
/// next `apply` are accepted rather than refused by a transaction that
/// no longer exists.
///
/// Control: before the undo the handle is held and an apply is refused,
/// so a helper that dropped the handle at `begin` fails the first
/// assertion, and one that never dropped it fails the last two.
#[test]
fn test_an_undo_closes_the_transaction_and_the_helper_forgets_its_handle() {
    let mut doc = MindMapDocument::new_blank(None, 50);
    assert!(doc.begin(
        ChangeSource::Input,
        TransactionKind::Interactive,
        crate::application::document::Interaction::Test
    ));
    assert!(doc.transaction.is_some(), "control: the handle is held");
    let req = create_root(&doc);
    assert!(
        doc.apply(req, ChangeSource::Action).is_err(),
        "control: an apply is refused"
    );

    doc.undo();
    assert!(
        doc.transaction.is_none(),
        "the undo closed it and the helper forgot the handle"
    );
    let req = create_root(&doc);
    assert!(
        doc.apply(req, ChangeSource::Action).is_ok(),
        "nothing holds the document any more"
    );
    assert!(
        doc.begin(
            ChangeSource::Input,
            TransactionKind::Interactive,
            crate::application::document::Interaction::Test
        ),
        "and a new one opens"
    );
}

/// Every map directly under `maps/` — the maps the cut-over converted
/// and committed in the saver's spelling — read off the disk so a new
/// one is covered the day it lands. `maps/demos/` is hand-written and
/// is not in the saver's spelling, so it is not held to this.
fn converted_maps() -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let root = baumhard::util::doc_fixtures::repo_path("maps");
    for entry in std::fs::read_dir(&root).unwrap_or_else(|e| panic!("{} lists: {e}", root.display())) {
        let path = entry.expect("a directory entry").path();
        if path.to_string_lossy().ends_with(super::MAP_EXTENSION) {
            out.push(path);
        }
    }
    out.sort();
    out
}

/// **Loading a converted map and saving it gives back its bytes.** The
/// maps are committed in the saver's own spelling, so the app's load →
/// save path (the one `save` runs) is the identity on each of them — a
/// change to either half that moves a byte shows up here, against the
/// real maps rather than a fixture written for the saver.
///
/// Controls: the four maps the cut-over converted are among those
/// checked, so the loop cannot pass by finding none; and the same map
/// re-spelled with its keys reordered and whitespace added loads to the
/// same document but does **not** save back to its own bytes, so the
/// comparison is one a non-canonical file fails.
#[test]
fn test_every_converted_map_saves_back_to_the_bytes_it_was_loaded_from() {
    let maps = converted_maps();
    for name in ["testament", "theme_demo", "palette_cascade", "stress_long_edges"] {
        let expected = format!("{name}{}", super::MAP_EXTENSION);
        assert!(
            maps.iter()
                .any(|path| path.file_name().is_some_and(|file| file == expected.as_str())),
            "control: maps/{expected} is one of the maps checked: {maps:?}"
        );
    }
    for path in &maps {
        let text = std::fs::read_to_string(path).expect("the map reads");
        let mut doc = MindMapDocument::load(path.to_str().expect("UTF-8"), 50)
            .unwrap_or_else(|e| panic!("{} loads: {e}", path.display()));
        let saved = doc.save_json().expect("the map saves");
        assert!(
            saved == text,
            "{} does not save back to its own bytes (first difference at byte {})",
            path.display(),
            saved
                .bytes()
                .zip(text.bytes())
                .position(|(a, b)| a != b)
                .unwrap_or(saved.len().min(text.len()))
        );
    }

    let original =
        std::fs::read_to_string(crate::application::document::tests_common::map_path("theme_demo"))
            .expect("the map reads");
    let value: serde_json::Value = serde_json::from_str(&original).expect("JSON");
    let respelled = serde_json::to_string_pretty(&value).expect("JSON") + "\n\n";
    assert_ne!(
        respelled, original,
        "control: the re-spelling differs from the file"
    );
    let dir = baumhard::util::test_temp::TempDir::new("map-respelled");
    let path = dir.join("theme_demo.mandala.json");
    std::fs::write(&path, &respelled).expect("the re-spelled map writes");
    let mut doc = MindMapDocument::load(path.to_str().expect("UTF-8"), 50).expect("a re-spelled map loads");
    let saved = doc.save_json().expect("it saves");
    assert_eq!(
        saved, original,
        "a re-spelled map saves in the canonical spelling"
    );
    assert_ne!(
        saved, respelled,
        "control: so load → save is not the identity on any text"
    );
}
