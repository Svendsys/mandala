// SPDX-License-Identifier: MPL-2.0

//! Keys the loader kept without understanding them.
//!
//! The loader's job is to not lose them; saying they are *wrong* is
//! this tool's job. `"min_zoom_to_rendr": 2.0` loads, warns once into
//! a log the author may never see, and is written back forever — so
//! without a check here a typo has no moment at which anybody finds
//! out. That was the strongest argument for reject-at-load and it is
//! answered here rather than by refusing the document.
//!
//! **Error, not warning.** A key nothing acts on is a key that does
//! nothing the author asked for, and `verify` exists to say so with a
//! nonzero exit. The map still loads and still renders; only the
//! verdict is severe.

use super::Violation;
use baumhard::mindmap::model::MindMap;

/// One violation per key the load did not recognize, stamped with the
/// part of the document it sits in.
///
/// Cost: O(preserved keys) — nothing at all for a map this build
/// authored, which carries none.
pub fn check(map: &MindMap) -> Vec<Violation> {
    map.unknown_keys
        .iter()
        .map(|entry| {
            Violation::at(
                "unknown_keys",
                entry.location(),
                format!(
                    "`{}` is not a key this build knows; it is preserved across saves but \
                     nothing reads it. Check the spelling, or the map was written by a \
                     newer version.",
                    entry.path_within_location()
                ),
            )
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A map with nothing unrecognized in it reports nothing — the
    /// negative control for the test below.
    #[test]
    fn test_a_map_with_no_preserved_keys_reports_nothing() {
        assert!(check(&MindMap::new_blank("t")).is_empty());
    }

    /// The key a load kept is the key `verify` complains about, named
    /// where the author has to go to fix it.
    #[test]
    fn test_a_preserved_key_is_reported_where_it_sits() {
        let json = r##"{
            "version": "1.0", "name": "typo",
            "canvas": {"background_color": "#000"},
            "nodes": {"0": {
                "id": "0", "parent_id": null,
                "position": {"x": 0, "y": 0},
                "size": {"width": 100, "height": 50},
                "sections": [{"text": "n"}],
                "style": {"background_color":"#000","frame_color":"#000",
                          "text_color":"#fff","shape":"rectangle",
                          "corner_radius_percent":0,"frame_thickness":0,
                          "show_frame":false,"show_shadow":false},
                "layout": {"type":"map","direction":"auto","spacing":0},
                "folded": false, "notes": "",
                "min_zoom_to_rendr": 2.0
            }},
            "edges": []
        }"##;
        let map = baumhard::mindmap::loader::load_from_str(json).expect("the map must load");
        let found = check(&map);
        assert_eq!(found.len(), 1, "one preserved key, one violation: {found:?}");
        assert_eq!(found[0].location, "node \"0\"");
        assert!(
            found[0].message.contains("min_zoom_to_rendr"),
            "must name the key: {}",
            found[0].message
        );
    }
}
