// SPDX-License-Identifier: MPL-2.0

//! Fixtures the app's tests share: the converted maps under `maps/`,
//! loaded through the one loader the app uses.

use super::MindMapDocument;

/// The path of `maps/<name>.mandala.json` in the repository.
pub(crate) fn map_path(name: &str) -> String {
    baumhard::util::doc_fixtures::repo_path(&format!("maps/{name}.mandala.json"))
        .to_str()
        .expect("a UTF-8 repo path")
        .to_string()
}

/// `maps/<name>.mandala.json`, loaded, with the default history limit.
pub(crate) fn load_map(name: &str) -> MindMapDocument {
    MindMapDocument::load(&map_path(name), baumhard::document::writer::DEFAULT_HISTORY_LIMIT)
        .unwrap_or_else(|e| panic!("maps/{name}.mandala.json loads: {e}"))
}

/// The testament map: the largest fixture, with sections, frames,
/// connectors, links, labels and portals.
pub(crate) fn testament() -> MindMapDocument {
    load_map("testament")
}

/// [`testament`], under the name the older suites call it by.
pub(crate) fn load_test_doc() -> MindMapDocument {
    testament()
}
