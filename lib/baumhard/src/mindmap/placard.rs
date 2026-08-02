// SPDX-License-Identifier: MPL-2.0

//! The map that gets shown when a map could not be loaded.
//!
//! A load failure has to reach the person holding the file, and the
//! only surface guaranteed to exist at that moment is the canvas: on
//! a desktop launch — double-click, `.desktop` entry, file
//! association — there is no terminal attached, so `stderr` is not
//! merely unread, it is gone. The answer here is to hand the shell a
//! *map* rather than a special-cased overlay: a placard is an
//! ordinary one-node [`MindMap`](crate::mindmap::model::MindMap)
//! carrying the loader's own message as
//! its text, so it renders through the same tree projection as any
//! other document (CODE_CONVENTIONS §3 "Render document content
//! through the Baumhard tree") and needs no second pipeline, no
//! renderer pass, and no GPU to test.
//!
//! **A placard is deliberately not the user's map.** It has its own
//! name, and the application binds it to no file path, so a reflexive
//! `Ctrl+S` cannot write it over the file that failed to parse. That
//! property lives at the application boundary — this module only
//! promises that the placard is a valid, self-contained map.
//!
//! The message is reproduced **in full**. Truncating it would defeat
//! the point: the loader's messages name the offending node id, the
//! offending key, and the migration verb, and the tail is where the
//! actionable part usually is. Loader messages are bounded in
//! practice — the longest shape serde produces is an
//! ``unknown field `x`, expected one of ...`` field enumeration — so
//! there is no runaway to guard against, and the node's axis clamp
//! ([`MAX_NODE_AXIS`](crate::mindmap::model::MAX_NODE_AXIS)) is the
//! backstop if that ever stops being true.

use crate::mindmap::model::{
    MindMap, MindNode, MindSection, NodeLayout, NodeStyle, Position, Size, TextRun,
};
use crate::util::grapheme_chad::{count_grapheme_clusters, grapheme_display_width, wrap_to_display_width};

/// `MindMap::name` every placard carries. Distinct from any name a
/// real map is likely to hold, so a placard is recognizable in a log
/// line or a window title without inspecting its nodes.
pub const PLACARD_MAP_NAME: &str = "load-failure";

/// Node id of a placard's single node. Dewey-decimal root, matching
/// what [`MindMap::new_blank`] would produce for a first node.
pub const PLACARD_NODE_ID: &str = "0";

/// Column count the message is wrapped to. Wide enough that a
/// typical loader message reads as two or three lines rather than a
/// column of fragments, narrow enough that a long one does not
/// become a single unreadable strip.
pub const PLACARD_COLUMNS: usize = 64;

/// First line of every placard. States the outcome before the
/// diagnosis, because that is the order the reader needs it in.
pub const PLACARD_HEADLINE: &str = "This map did not load.";

/// Last line of every placard. Answers the question the headline
/// raises — whether anything was damaged.
pub const PLACARD_FOOTER: &str = "Nothing was opened; the file on disk is unchanged.";

/// Point size of the placard's text run.
const PLACARD_FONT_SIZE_PT: u32 = 24;

/// Font family of the placard's text run. The family every other
/// freshly-authored run in the workspace uses, so the placard reads
/// as part of the application rather than as a system dialog.
const PLACARD_FONT_FAMILY: &str = "LiberationSans";

/// Text and frame color. The red the browser build's DOM overlay
/// used before the placard replaced it, carried over so the visual
/// identity of "your map was rejected" survives the change of
/// mechanism.
const PLACARD_ERROR_COLOR: &str = "#ff6b6b";

/// Node fill. Matches the fill a freshly-created node takes, so the
/// placard sits on the canvas as a node rather than as chrome.
const PLACARD_BACKGROUND_COLOR: &str = "#141414";

/// Advance of one text cell as a fraction of the point size, and the
/// line advance as a multiple of it. Both are estimates: this module
/// has no `FONT_SYSTEM` access and does not want one on a cold error
/// path. The application's grow-to-fit pass re-measures the block
/// with real metrics and can only enlarge the box, so an estimate
/// that runs small is corrected and an estimate that runs large is
/// not — which is why these lean conservative.
const PLACARD_CELL_WIDTH_FRAC: f64 = 0.55;
/// See [`PLACARD_CELL_WIDTH_FRAC`].
const PLACARD_LINE_HEIGHT_FRAC: f64 = 1.3;

/// Build the placard map for a load failure of `source` (the path or
/// URL that was asked for) reporting `message` (the loader's own
/// text, verbatim).
///
/// The result is a complete, valid `.mindmap.json` document: it
/// round-trips through [`crate::mindmap::loader::load_from_str`] and
/// satisfies every invariant that loader enforces, which is what
/// lets the shell treat it as an ordinary document instead of a
/// special case.
///
/// Cost: one wrap pass over `source` + `message`
/// ([`wrap_to_display_width`]), one `String` for the joined body,
/// and the map's own allocations. Cold — reached once, on a load
/// that already failed.
pub fn load_failure(source: &str, message: &str) -> MindMap {
    let text = load_failure_text(source, message);
    let mut map = MindMap::new_blank(PLACARD_MAP_NAME);
    map.nodes
        .insert(PLACARD_NODE_ID.to_string(), placard_node(text));
    map
}

/// The placard's body text: headline, the source that was asked for,
/// the loader's message, and the footer, each separated by a blank
/// line and each wrapped to [`PLACARD_COLUMNS`].
///
/// Split out from [`load_failure`] because it is the whole of what a
/// reader sees, and asserting on a `String` beats digging it back
/// out of a `MindMap`.
///
/// Cost: one [`wrap_to_display_width`] pass over each of `source`
/// and `message`, plus the joined result.
pub fn load_failure_text(source: &str, message: &str) -> String {
    let mut lines: Vec<String> = vec![PLACARD_HEADLINE.to_string(), String::new()];
    lines.extend(wrap_to_display_width(source, PLACARD_COLUMNS));
    lines.push(String::new());
    lines.extend(wrap_to_display_width(message, PLACARD_COLUMNS));
    lines.push(String::new());
    lines.push(PLACARD_FOOTER.to_string());
    lines.join("\n")
}

/// The placard's single node, sized to its own text and styled to
/// read as a rejection rather than as content.
fn placard_node(text: String) -> MindNode {
    let (width, height) = estimated_size(&text);
    let run = TextRun {
        start: 0,
        end: count_grapheme_clusters(&text),
        bold: false,
        italic: false,
        underline: false,
        font: PLACARD_FONT_FAMILY.to_string(),
        size_pt: PLACARD_FONT_SIZE_PT,
        color: PLACARD_ERROR_COLOR.to_string(),
        hyperlink: None,
    };
    MindNode {
        id: PLACARD_NODE_ID.to_string(),
        parent_id: None,
        position: Position { x: 0.0, y: 0.0 },
        size: Size { width, height },
        sections: vec![MindSection::new_default(text, vec![run])],
        style: NodeStyle {
            background_color: PLACARD_BACKGROUND_COLOR.to_string(),
            frame_color: PLACARD_ERROR_COLOR.to_string(),
            text_color: PLACARD_ERROR_COLOR.to_string(),
            shape: "rectangle".to_string(),
            corner_radius_percent: 10.0,
            frame_thickness: 4.0,
            show_frame: true,
            show_shadow: false,
            border: None,
        },
        layout: NodeLayout {
            layout_type: "map".to_string(),
            direction: "auto".to_string(),
            spacing: 50.0,
        },
        folded: false,
        notes: String::new(),
        color_schema: None,
        channel: 0,
        trigger_bindings: Vec::new(),
        inline_mutations: Vec::new(),
        inline_macros: Vec::new(),
        min_zoom_to_render: None,
        max_zoom_to_render: None,
    }
}

/// Canvas-unit box for `text` at the placard's point size, from the
/// widest line and the line count. An estimate by construction — see
/// [`PLACARD_CELL_WIDTH_FRAC`].
///
/// Cost: one grapheme walk per line of `text`.
fn estimated_size(text: &str) -> (f64, f64) {
    let widest = text.lines().map(grapheme_display_width).max().unwrap_or(0);
    let line_count = text.lines().count().max(1);
    let pt = f64::from(PLACARD_FONT_SIZE_PT);
    (
        widest as f64 * pt * PLACARD_CELL_WIDTH_FRAC,
        line_count as f64 * pt * PLACARD_LINE_HEIGHT_FRAC,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mindmap::loader;

    const SOURCE: &str = "/home/user/maps/broken.mindmap.json";
    const MESSAGE: &str = "node \"0.3\" ships zero sections — every renderable node needs at \
                           least one. Run `maptool convert --sections <file>` to migrate, or \
                           add an explicit `sections` array.";

    /// The whole reason the placard exists: the reader has to be
    /// able to find the loader's own words on it. Every non-blank
    /// run of the message must survive the wrap, so a reworded
    /// loader cannot quietly stop being reported.
    #[test]
    fn test_placard_reproduces_the_loader_message_in_full() {
        let text = load_failure_text(SOURCE, MESSAGE);
        let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let expected = MESSAGE.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flattened.contains(&expected),
            "the placard dropped part of the loader message:\n{text}"
        );
    }

    /// The path the user asked for is on the placard too — "this map
    /// did not load" is not actionable when three maps are open in
    /// three windows.
    #[test]
    fn test_placard_names_the_source_that_failed() {
        let text = load_failure_text(SOURCE, MESSAGE);
        let flattened = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flattened.contains(SOURCE), "source missing from:\n{text}");
        assert!(text.starts_with(PLACARD_HEADLINE));
        assert!(text.ends_with(PLACARD_FOOTER));
    }

    /// No line of the placard runs past its column, except where a
    /// single indivisible cluster is wider than the whole column —
    /// the one case [`wrap_to_display_width`] documents as
    /// over-running rather than dropping input.
    #[test]
    fn test_placard_respects_its_column_width() {
        let long_path = format!("/home/user/{}/map.mindmap.json", "d".repeat(200));
        for (source, message) in [
            (SOURCE, MESSAGE),
            (long_path.as_str(), "Failed to parse mindmap JSON: EOF while parsing a value at line 1 column 0"),
            ("", ""),
        ] {
            let text = load_failure_text(source, message);
            for line in text.lines() {
                assert!(
                    grapheme_display_width(line) <= PLACARD_COLUMNS
                        || count_grapheme_clusters(line) == 1,
                    "line {line:?} exceeds {PLACARD_COLUMNS} columns"
                );
            }
        }
    }

    /// **The placard is an ordinary map, not a special case.** It is
    /// serialized and fed back through the real loader, so every
    /// invariant the loader enforces — required keys, the
    /// zero-section rejection, the section cap, the parent-cycle
    /// screen, closed objects — is enforced against it too. Without
    /// this, a model change could turn the surface that reports a
    /// broken map into a second broken map.
    #[test]
    fn test_placard_is_a_map_the_loader_accepts() {
        let map = load_failure(SOURCE, MESSAGE);
        let json = serde_json::to_string(&map).expect("placard serializes");
        let reloaded = loader::load_from_str(&json).unwrap_or_else(|e| {
            panic!("the placard must be a loadable map: {e}\n{json}");
        });
        assert_eq!(reloaded.name, PLACARD_MAP_NAME);
        assert_eq!(reloaded.nodes.len(), 1);
        assert!(reloaded.edges.is_empty());
        let node = reloaded.nodes.get(PLACARD_NODE_ID).expect("placard node");
        assert_eq!(node.sections.len(), 1);
        assert!(node.parent_id.is_none());
    }

    /// A placard with an empty message is still a *renderable* map —
    /// the loader rejects a node whose text is nothing at all only
    /// via the zero-section rule, but a placard that showed a blank
    /// box would be the blank window #107 is about, restored by the
    /// back door.
    #[test]
    fn test_empty_message_still_produces_a_readable_placard() {
        let map = load_failure("", "");
        let node = map.nodes.get(PLACARD_NODE_ID).expect("placard node");
        let text = &node.sections[0].text;
        assert!(text.contains(PLACARD_HEADLINE));
        assert!(text.contains(PLACARD_FOOTER));
        assert!(node.size.width > 0.0 && node.size.height > 0.0);
    }

    /// The text run has to cover the whole text in grapheme units —
    /// a run that stops short leaves the tail unstyled, and the
    /// message is exactly the part that would go missing.
    #[test]
    fn test_placard_run_covers_every_grapheme_of_the_text() {
        let map = load_failure(SOURCE, "unknown field `colour` 👨‍👩‍👧 日本語");
        let section = &map.nodes[PLACARD_NODE_ID].sections[0];
        assert_eq!(section.text_runs.len(), 1);
        assert_eq!(section.text_runs[0].start, 0);
        assert_eq!(
            section.text_runs[0].end,
            count_grapheme_clusters(&section.text)
        );
    }

    /// The node box grows with the message. A fixed box would clip a
    /// long diagnosis against exactly the failure mode that produces
    /// the longest ones.
    #[test]
    fn test_placard_box_grows_with_the_message() {
        let short = load_failure(SOURCE, "no");
        let long = load_failure(SOURCE, &"a word ".repeat(200));
        let short_node = &short.nodes[PLACARD_NODE_ID];
        let long_node = &long.nodes[PLACARD_NODE_ID];
        assert!(
            long_node.size.height > short_node.size.height,
            "a longer message must produce a taller placard"
        );
    }
}
