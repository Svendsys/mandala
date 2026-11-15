// SPDX-License-Identifier: MPL-2.0

//! The presentations the app can play, by tier
//! (`format/presentations.md` §"Where presentations come from"; design
//! §10.2, V1-D2):
//!
//! - **App**: `assets/presentations.json`, compiled in;
//! - **User**: the user's `presentations.json` —
//!   `$XDG_CONFIG_HOME/mandala/` on native,
//!   `localStorage["mandala_presentations"]` in the browser;
//! - **Map**: the open document's own `presentations`.
//!
//! Precedence is App < User < Map, by `id`, whole definition, as for
//! edits (`application::edits`). The App and User tiers are read once at
//! startup and held here; the Map tier is the document's, read where it
//! is asked for, so a map that is replaced never leaves its
//! presentations behind.
//!
//! A registry of its own, beside the edits': the two kinds of behavior
//! are different types and different lists everywhere (D6).

use baumhard::document::ids::Document;
use baumhard::document::presentation::{parse_presentations, PresentationDef};

use crate::application::edits::EditTier;

/// The bundled presentations.
const APP_PRESENTATIONS: &str = include_str!("../../assets/presentations.json");

/// The `presentations.json` file name's label, for the log.
const LABEL: &str = "presentations";

/// The App and User presentations, filled at startup on both targets.
pub(crate) struct PresentationRegistry {
    app: Vec<PresentationDef>,
    user: Vec<PresentationDef>,
}

impl PresentationRegistry {
    /// The registry of `app` and `user` presentations.
    pub(crate) fn new(app: Vec<PresentationDef>, user: Vec<PresentationDef>) -> Self {
        PresentationRegistry { app, user }
    }

    /// The bundled presentations alone: what a launch reads before the
    /// user's. A bundled file that does not read is a build defect,
    /// logged, and leaves no App tier.
    pub(crate) fn bundled() -> Self {
        let app = parse_presentations(APP_PRESENTATIONS).unwrap_or_else(|error| {
            log::error!("presentations: the bundled assets/presentations.json does not read: {error}");
            Vec::new()
        });
        Self::new(app, Vec::new())
    }

    /// The bundled presentations and the user's (`presentations.json`),
    /// never failing: a user file that does not read is logged and
    /// ignored whole.
    pub(crate) fn load() -> Self {
        let mut registry = Self::bundled();
        registry.user = load_user();
        registry
    }

    /// The presentation `id` plays as on `doc`, and the tier it came
    /// from: the map's, else the user's, else the app's.
    /// O(presentations).
    pub(crate) fn find<'a>(&'a self, doc: &'a Document, id: &str) -> Option<(&'a PresentationDef, EditTier)> {
        let find = |list: &'a [PresentationDef]| list.iter().find(|def| def.id == id);
        find(&doc.definitions().presentations)
            .map(|def| (def, EditTier::Map))
            .or_else(|| find(&self.user).map(|def| (def, EditTier::User)))
            .or_else(|| find(&self.app).map(|def| (def, EditTier::App)))
    }

    /// Every presentation that plays on `doc` — each id's winning
    /// definition — by id. O(presentations log presentations).
    pub(crate) fn list<'a>(&'a self, doc: &'a Document) -> Vec<(&'a PresentationDef, EditTier)> {
        let mut ids: Vec<&str> = doc
            .definitions()
            .presentations
            .iter()
            .chain(&self.user)
            .chain(&self.app)
            .map(|def| def.id.as_str())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter().filter_map(|id| self.find(doc, id)).collect()
    }
}

/// Read a user `presentations.json`.
fn parse(json: &str) -> Result<Vec<PresentationDef>, String> {
    parse_presentations(json).map_err(|error| error.to_string())
}

/// The user's presentations: the XDG `presentations.json` on native.
#[cfg(not(target_arch = "wasm32"))]
fn load_user() -> Vec<PresentationDef> {
    match crate::application::user_config::load_desktop_layered(LABEL, "presentations.json", None, parse) {
        Some((presentations, source)) => {
            log::info!("loaded {} presentations from {source}", presentations.len());
            presentations
        }
        None => Vec::new(),
    }
}

/// The user's presentations: `localStorage["mandala_presentations"]` in
/// the browser.
#[cfg(target_arch = "wasm32")]
fn load_user() -> Vec<PresentationDef> {
    match crate::application::user_config::web_storage::load_web_storage_only(
        LABEL,
        "mandala_presentations",
        parse,
    ) {
        Some((presentations, source)) => {
            log::info!("loaded {} presentations from {source}", presentations.len());
            presentations
        }
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::document::format::load;
    use baumhard::document::presentation::Play;

    /// A presentations-file entry `id`, called `name`, of `play`.
    fn entry(id: &str, name: &str, play: &str) -> String {
        format!(r#"{{"id":"{id}","name":"{name}","play":"{play}","target":"node","mutations":[]}}"#)
    }

    fn map_with(id: &str, name: &str) -> Document {
        load(&format!(
            r#"{{"format":1,"name":"t","presentations":[{}],"nodes":[]}}"#,
            entry(id, name, "once")
        ))
        .unwrap()
    }

    fn user(id: &str, name: &str) -> PresentationDef {
        parse_presentations(&format!(r#"{{"presentations":[{}]}}"#, entry(id, name, "while")))
            .unwrap()
            .remove(0)
    }

    /// The bundled file reads, and holds the three presentations the
    /// format's page describes.
    #[test]
    fn test_the_bundled_presentations_read() {
        let registry = PresentationRegistry::bundled();
        let ids: Vec<(&str, Play)> = registry
            .app
            .iter()
            .map(|def| (def.id.as_str(), def.play))
            .collect();
        assert_eq!(
            ids,
            [
                ("concealed", Play::Hold),
                ("lift", Play::While),
                ("pulse", Play::Once)
            ]
        );
    }

    /// App < User < Map, by id, whole definition — and each tier is
    /// reported.
    #[test]
    fn test_a_later_tier_replaces_a_presentation_by_id() {
        let mut registry = PresentationRegistry::bundled();
        let blank = load(r#"{"format":1,"name":"t","nodes":[]}"#).unwrap();
        let (app, tier) = registry.find(&blank, "lift").unwrap();
        assert_eq!((app.name.as_str(), tier), ("Lift", EditTier::App));
        registry.user = vec![user("lift", "Mine")];
        let (mine, tier) = registry.find(&blank, "lift").unwrap();
        assert_eq!((mine.name.as_str(), tier), ("Mine", EditTier::User));
        let map = map_with("lift", "The map's");
        let (theirs, tier) = registry.find(&map, "lift").unwrap();
        assert_eq!((theirs.name.as_str(), tier), ("The map's", EditTier::Map));
        // The map's replaces the whole definition: its lifecycle, not
        // the user's.
        assert_eq!(theirs.play, Play::Once);
        // Control: an id only the app has is still the app's.
        assert_eq!(registry.find(&map, "pulse").unwrap().1, EditTier::App);
        assert!(registry.find(&map, "no-such").is_none());
        // The list holds each id once, the winner's.
        let list = registry.list(&map);
        assert_eq!(list.len(), 3);
        assert_eq!(list.iter().filter(|(def, _)| def.id == "lift").count(), 1);
    }
}
