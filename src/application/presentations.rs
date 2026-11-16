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
use rustc_hash::FxHashMap;

/// The bundled presentations.
const APP_PRESENTATIONS: &str = include_str!("../../assets/presentations.json");

/// The `presentations.json` file name's label, for the log.
const LABEL: &str = "presentations";

/// Where a presentation came from, which `present list` reports: a
/// tier of its own, beside the edits' (`EditTier`), as the two kinds of
/// behavior are different lists everywhere (D6).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum PresentationTier {
    /// Bundled with the app: `assets/presentations.json`.
    App,
    /// The user's `presentations.json`, or
    /// `localStorage["mandala_presentations"]` in the browser.
    User,
    /// The open map's own `presentations`.
    Map,
}

impl PresentationTier {
    /// The tier's name, as the console shows it.
    // Read by the console's `present` verb, native as the console shell is.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub(crate) fn name(self) -> &'static str {
        match self {
            PresentationTier::App => "app",
            PresentationTier::User => "user",
            PresentationTier::Map => "map",
        }
    }
}

/// The App and User presentations, filled at startup on both targets.
pub(crate) struct PresentationRegistry {
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "what starts and stops a presentation today is the console shell, which is native; \
                      the map's triggers (V1-D3, #252) do on both targets"
        )
    )]
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
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "what starts and stops a presentation today is the console shell, which is native; \
                      the map's triggers (V1-D3, #252) do on both targets"
        )
    )]
    pub(crate) fn find<'a>(
        &'a self,
        doc: &'a Document,
        id: &str,
    ) -> Option<(&'a PresentationDef, PresentationTier)> {
        let find = |list: &'a [PresentationDef]| list.iter().find(|def| def.id == id);
        find(&doc.definitions().presentations)
            .map(|def| (def, PresentationTier::Map))
            .or_else(|| find(&self.user).map(|def| (def, PresentationTier::User)))
            .or_else(|| find(&self.app).map(|def| (def, PresentationTier::App)))
    }

    /// Every presentation that plays on `doc` — each id's winning
    /// definition — by id: one pass over the three tiers, lowest first,
    /// a later tier's definition replacing an earlier one's in a map by
    /// id, then the winners sorted. O(presentations log presentations).
    #[cfg_attr(
        target_arch = "wasm32",
        expect(
            dead_code,
            reason = "what starts and stops a presentation today is the console shell, which is native; \
                      the map's triggers (V1-D3, #252) do on both targets"
        )
    )]
    pub(crate) fn list<'a>(&'a self, doc: &'a Document) -> Vec<(&'a PresentationDef, PresentationTier)> {
        let tiers: [(&'a [PresentationDef], PresentationTier); 3] = [
            (&self.app, PresentationTier::App),
            (&self.user, PresentationTier::User),
            (&doc.definitions().presentations, PresentationTier::Map),
        ];
        let mut winners: FxHashMap<&'a str, (&'a PresentationDef, PresentationTier)> = FxHashMap::default();
        for (list, tier) in tiers {
            for def in list {
                // Within one tier the first of an id wins, as `find`
                // reads it; a later tier replaces it whole.
                match winners.get(def.id.as_str()) {
                    Some((_, held)) if *held == tier => {}
                    _ => {
                        winners.insert(def.id.as_str(), (def, tier));
                    }
                }
            }
        }
        let mut list: Vec<(&'a PresentationDef, PresentationTier)> = winners.into_values().collect();
        list.sort_unstable_by(|(a, _), (b, _)| a.id.cmp(&b.id));
        list
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

    /// A presentation that would change the map's structure reaches no
    /// tier by a file: the App's and the User's are read by one
    /// function, which refuses the whole file at the mutation's path —
    /// so a user file holding one beside a good presentation gives no
    /// User tier at all (`load_desktop_layered` keeps nothing of a file
    /// that does not parse). The map's own list is refused by the same
    /// reader (`presentation_tests`), and a definition built in code
    /// cannot hold one by its type; the layer refuses a mutator that
    /// does all the same (`layer::tests`).
    ///
    /// Control: the same file with the field write alone reads.
    #[test]
    fn test_a_file_with_a_structural_mutation_gives_no_tier() {
        let file = |mutation: &str| {
            format!(
                r#"{{"presentations":[{},{{"id":"s","name":"S","play":"hold","target":"node","mutations":[{{"node":{{"set_hidden":true}}}},{mutation}]}}]}}"#,
                entry("good", "Good", "while")
            )
        };
        assert_eq!(
            parse(&file(r#"{"node":{"set_opacity":0.5}}"#)).map(|read| read.len()),
            Ok(2)
        );
        for structural in [
            r#"{"node":{"remove":true}}"#,
            r#"{"node":{"insert_child":{"id":9}}}"#,
            r#"{"section":{"split":{"at":1}}}"#,
            r#"{"structure":{"remove":true}}"#,
        ] {
            let refused = parse(&file(structural)).expect_err(structural);
            assert!(
                refused.contains("presentations[1].mutations[1]"),
                "{structural}: refused where it is written: {refused}"
            );
        }
        // And the bundled file, read by the same function, holds none.
        assert_eq!(parse(APP_PRESENTATIONS).map(|read| read.len()), Ok(3));
    }

    /// App < User < Map, by id, whole definition — and each tier is
    /// reported; the list holds each id's winner, in id order.
    ///
    /// Control, seen red: the list reading the tiers map first lets the
    /// app's `lift` win over the map's.
    #[test]
    fn test_a_later_tier_replaces_a_presentation_by_id() {
        let mut registry = PresentationRegistry::bundled();
        let blank = load(r#"{"format":1,"name":"t","nodes":[]}"#).unwrap();
        let (app, tier) = registry.find(&blank, "lift").unwrap();
        assert_eq!((app.name.as_str(), tier), ("Lift", PresentationTier::App));
        registry.user = vec![user("lift", "Mine")];
        let (mine, tier) = registry.find(&blank, "lift").unwrap();
        assert_eq!((mine.name.as_str(), tier), ("Mine", PresentationTier::User));
        let map = map_with("lift", "The map's");
        let (theirs, tier) = registry.find(&map, "lift").unwrap();
        assert_eq!((theirs.name.as_str(), tier), ("The map's", PresentationTier::Map));
        // The map's replaces the whole definition: its lifecycle, not
        // the user's.
        assert_eq!(theirs.play, Play::Once);
        // Control: an id only the app has is still the app's.
        assert_eq!(registry.find(&map, "pulse").unwrap().1, PresentationTier::App);
        assert!(registry.find(&map, "no-such").is_none());
        // The list holds each id once, the winner's.
        let list = registry.list(&map);
        assert_eq!(list.len(), 3);
        assert_eq!(list.iter().filter(|(def, _)| def.id == "lift").count(), 1);
        // And each entry is what `find` answers for its id, in id order.
        for (def, tier) in &list {
            assert_eq!(registry.find(&map, &def.id), Some((*def, *tier)), "{}", def.id);
        }
        let ids: Vec<&str> = list.iter().map(|(def, _)| def.id.as_str()).collect();
        assert_eq!(ids, ["concealed", "lift", "pulse"]);
    }
}
