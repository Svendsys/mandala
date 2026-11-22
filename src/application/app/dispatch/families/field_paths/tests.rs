// SPDX-License-Identifier: MPL-2.0

use baumhard::document::format;
use baumhard::document::writer::DEFAULT_HISTORY_LIMIT;

use crate::application::document::MindMapDocument;

/// The fixture every case starts from.
const FIXTURE: &str = include_str!("../fixtures/paths.mandala.json");

/// The fixture, loaded, with nothing selected.
fn fixture() -> MindMapDocument {
    MindMapDocument::from_json_str(FIXTURE, None, DEFAULT_HISTORY_LIMIT).expect("the fixture loads")
}

#[test]
fn test_fixture_spelling() {
    let saved = format::save(fixture().doc()).expect("saves");
    assert_eq!(saved, FIXTURE);
}
