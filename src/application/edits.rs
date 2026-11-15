// SPDX-License-Identifier: MPL-2.0

//! The edits the app can run, by tier (`format/edits.md` §"Where edits
//! come from"; design §10.1, V1-D1):
//!
//! - **App**: `assets/edits.json`, compiled in;
//! - **User**: the user's `edits.json` — `$XDG_CONFIG_HOME/mandala/` on
//!   native, `localStorage["mandala_edits"]` in the browser;
//! - **Map**: the open document's own `edits`.
//!
//! Precedence is App < User < Map, by `id`, whole definition. The App
//! and User tiers are read once at startup and held here with the
//! handler registry; the Map tier is the document's, read where it is
//! asked for, so a map that is replaced never leaves its edits behind.

use baumhard::document::edit::{parse_edits, EditDef};
use baumhard::document::ids::Document;
use baumhard::edit_handlers::{registry, Handlers};

/// The bundled edits.
const APP_EDITS: &str = include_str!("../../assets/edits.json");

/// The `edits.json` file name's label, for the log.
const LABEL: &str = "edits";

/// Where a definition came from, which `edit help` reports.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum EditTier {
    /// Bundled with the app.
    App,
    /// The user's `edits.json`.
    User,
    /// The open map.
    Map,
}

impl EditTier {
    /// The tier's name, as the console shows it.
    // Read by the console's `edit` verb, native as the console shell is.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn name(self) -> &'static str {
        match self {
            EditTier::App => "app",
            EditTier::User => "user",
            EditTier::Map => "map",
        }
    }
}

/// The App and User edits, and the handlers, filled at startup on both
/// targets.
pub(crate) struct EditRegistry {
    app: Vec<EditDef>,
    user: Vec<EditDef>,
    handlers: Handlers,
}

impl EditRegistry {
    /// The registry of `app` and `user` edits.
    pub(crate) fn new(app: Vec<EditDef>, user: Vec<EditDef>) -> Self {
        EditRegistry {
            app,
            user,
            handlers: registry(),
        }
    }

    /// The bundled edits alone: what a launch reads before the user's.
    /// A bundled file that does not read is a build defect, logged, and
    /// leaves no App tier.
    pub(crate) fn bundled() -> Self {
        let app = parse_edits(APP_EDITS).unwrap_or_else(|error| {
            log::error!("edits: the bundled assets/edits.json does not read: {error}");
            Vec::new()
        });
        Self::new(app, Vec::new())
    }

    /// The bundled edits and the user's (`edits.json`), never failing:
    /// a user file that does not read is logged and ignored whole.
    pub(crate) fn load() -> Self {
        let mut registry = Self::bundled();
        registry.user = load_user();
        registry
    }

    /// The handlers.
    pub(crate) fn handlers(&self) -> &Handlers {
        &self.handlers
    }

    /// The edit `id` runs on `doc`, and the tier it came from: the
    /// map's, else the user's, else the app's. O(edits).
    pub(crate) fn find<'a>(&'a self, doc: &'a Document, id: &str) -> Option<(&'a EditDef, EditTier)> {
        let find = |list: &'a [EditDef]| list.iter().find(|edit| edit.id == id);
        find(&doc.definitions().edits)
            .map(|edit| (edit, EditTier::Map))
            .or_else(|| find(&self.user).map(|edit| (edit, EditTier::User)))
            .or_else(|| find(&self.app).map(|edit| (edit, EditTier::App)))
    }

    /// Every edit that runs on `doc` — each id's winning definition — by
    /// id. O(edits log edits).
    // Read by the console's `edit` verb, native as the console shell is.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn list<'a>(&'a self, doc: &'a Document) -> Vec<(&'a EditDef, EditTier)> {
        let mut ids: Vec<&str> = doc
            .definitions()
            .edits
            .iter()
            .chain(&self.user)
            .chain(&self.app)
            .map(|edit| edit.id.as_str())
            .collect();
        ids.sort_unstable();
        ids.dedup();
        ids.into_iter().filter_map(|id| self.find(doc, id)).collect()
    }
}

/// Read a user `edits.json`.
fn parse(json: &str) -> Result<Vec<EditDef>, String> {
    parse_edits(json).map_err(|error| error.to_string())
}

/// The user's edits: the XDG `edits.json` on native.
#[cfg(not(target_arch = "wasm32"))]
fn load_user() -> Vec<EditDef> {
    match crate::application::user_config::load_desktop_layered(LABEL, "edits.json", None, parse) {
        Some((edits, source)) => {
            log::info!("loaded {} edits from {source}", edits.len());
            edits
        }
        None => Vec::new(),
    }
}

/// The user's edits: `localStorage["mandala_edits"]` in the browser.
#[cfg(target_arch = "wasm32")]
fn load_user() -> Vec<EditDef> {
    match crate::application::user_config::web_storage::load_web_storage_only(LABEL, "mandala_edits", parse) {
        Some((edits, source)) => {
            log::info!("loaded {} edits from {source}", edits.len());
            edits
        }
        None => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::document::format::load;

    fn map_with_edit(id: &str, name: &str) -> Document {
        load(&format!(
            r#"{{"format":1,"name":"t","edits":[{{"id":"{id}","name":"{name}","reach":"self","target":"node","handler":"cascade"}}],"nodes":[]}}"#
        ))
        .unwrap()
    }

    fn user_edit(id: &str, name: &str) -> EditDef {
        let file = format!(
            r#"{{"edits":[{{"id":"{id}","name":"{name}","reach":"self","target":"node","handler":"flower"}}]}}"#
        );
        parse_edits(&file).unwrap().remove(0)
    }

    /// The bundled file reads, and holds the five edits `main` shipped,
    /// under their V1 ids.
    #[test]
    fn test_the_bundled_edits_read() {
        let registry = EditRegistry::bundled();
        let ids: Vec<&str> = registry.app.iter().map(|edit| edit.id.as_str()).collect();
        assert_eq!(
            ids,
            [
                "grow-font-2pt",
                "shrink-font-2pt",
                "nudge-right-50",
                "flower",
                "cascade"
            ]
        );
    }

    /// App < User < Map, by id, whole definition — and each tier is
    /// reported.
    #[test]
    fn test_a_later_tier_replaces_an_edit_by_id() {
        let mut registry = EditRegistry::bundled();
        let blank = load(r#"{"format":1,"name":"t","nodes":[]}"#).unwrap();
        let (app, tier) = registry.find(&blank, "flower").unwrap();
        assert_eq!((app.name.as_str(), tier), ("Flower layout", EditTier::App));
        registry.user = vec![user_edit("flower", "Mine")];
        let (user, tier) = registry.find(&blank, "flower").unwrap();
        assert_eq!((user.name.as_str(), tier), ("Mine", EditTier::User));
        let map = map_with_edit("flower", "The map's");
        let (theirs, tier) = registry.find(&map, "flower").unwrap();
        assert_eq!((theirs.name.as_str(), tier), ("The map's", EditTier::Map));
        // The map's replaces the whole definition: its handler, not the
        // user's.
        assert_eq!(
            theirs.body,
            baumhard::document::edit::EditBody::Handler("cascade".to_string())
        );
        // Control: an id only the app has is still the app's.
        assert_eq!(registry.find(&map, "cascade").unwrap().1, EditTier::App);
        // The list holds each id once, the winner's.
        let list = registry.list(&map);
        assert_eq!(list.len(), 5);
        assert_eq!(list.iter().filter(|(edit, _)| edit.id == "flower").count(), 1);
    }
}
