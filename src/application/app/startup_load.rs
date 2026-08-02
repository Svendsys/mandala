// SPDX-License-Identifier: MPL-2.0

//! What the shell does with the initial map load — the one answer
//! both targets use.
//!
//! Startup is the surface most likely to be a user's first contact
//! with the format, and it used to be the only one that swallowed a
//! loader error: the console `open` verb puts the message in the
//! overlay, `maptool` puts it on `stderr` and exits nonzero, and
//! startup logged it and rendered an empty canvas. On a desktop
//! launch — double-click, `.desktop` entry, file association — no
//! terminal is attached, so that log line does not exist for the
//! person who needs it.
//!
//! The answer is a *document*: a rejected load becomes the
//! [`baumhard::mindmap::placard`] map, which the shell then installs
//! exactly as it installs a real one. The consequences are the point
//! of doing it this way — the window opens, the canvas has content,
//! the camera fits it, input works, and the console (native) can
//! `open` something else without restarting. None of that needed a
//! new render path, and none of it needs a GPU to test.
//!
//! **Both targets go through [`adopt`].** The native and browser
//! init paths call it and nothing else, so the two cannot drift into
//! reporting the same failure differently; `test_both_targets_adopt_
//! the_startup_surface` reads their sources and fails if either one
//! stops. That is a source-level pin rather than a runtime one
//! because `cargo test` only ever runs the native leg
//! (TEST_CONVENTIONS §T9), so a runtime assertion could not see the
//! browser arm at all.

use crate::application::document::MindMapDocument;

/// The decision: given the result of the initial load, what goes on
/// the canvas.
///
/// Every arm carries a document, because the shell always has
/// something to render — that is the fix. The variants exist so the
/// *difference* between them stays explicit and exhaustively
/// matched: a third outcome (a map that loaded with warnings worth
/// showing, say) is a compile error in [`adopt`] rather than a
/// silent fallthrough back to the empty window.
pub(crate) enum StartupSurface {
    /// The map loaded. Render it.
    Loaded(MindMapDocument),
    /// The map did not load. Render `placard` — which carries
    /// `message`, the loader's own text — and report `message`.
    Rejected {
        /// The one-node map standing in for the map that failed.
        placard: MindMapDocument,
        /// The loader's message, verbatim. Held separately from the
        /// placard so the log line and the canvas cannot disagree
        /// about what went wrong.
        message: String,
    },
}

/// Decide what the shell shows for `load`, the result of the initial
/// map load of `source` (the CLI path on native, the `?map=` URL in
/// the browser).
///
/// Pure: builds a document either way and reports nothing. The
/// reporting lives in [`adopt`], which is what makes this function
/// assertable line by line without a logger or a device.
pub(crate) fn startup_surface(source: &str, load: Result<MindMapDocument, String>) -> StartupSurface {
    match load {
        Ok(document) => StartupSurface::Loaded(document),
        Err(message) => StartupSurface::Rejected {
            placard: MindMapDocument::load_failure_placard(source, &message),
            message,
        },
    }
}

/// Take the document the canvas will render, reporting the loader's
/// message on the way past when there was one.
///
/// The single place either target turns a [`StartupSurface`] into a
/// document. The log line still fires — a terminal user and a bug
/// report both want it, and the placard is an addition to it, not a
/// replacement for it.
///
/// Three lines long on purpose: everything that *decides* lives in
/// [`resolve`], which is pure and pinned, so the only thing not
/// covered by a test here is the `log::error!` invocation itself.
pub(crate) fn adopt(surface: StartupSurface) -> MindMapDocument {
    let (document, report) = resolve(surface);
    if let Some(message) = report {
        log::error!("startup: {}", message);
    }
    document
}

/// The document to render, and the line to report if there is one.
///
/// The exhaustive match: a new [`StartupSurface`] variant is a
/// compile error here, not a silent fallthrough back to the empty
/// window #107 is about. Pure, so both halves of the answer — which
/// document, and what gets said about it — are assertable without a
/// logger or a device.
fn resolve(surface: StartupSurface) -> (MindMapDocument, Option<String>) {
    match surface {
        StartupSurface::Loaded(document) => (document, None),
        StartupSurface::Rejected { placard, message } => (placard, Some(message)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::placard;

    /// The failing shape a hand-author actually meets: valid JSON,
    /// but a node with no sections. Routed through the real loader
    /// rather than a fabricated `Err`, so the message under test is
    /// the message the loader emits.
    const ZERO_SECTION_MAP: &str = r##"{
        "version": "1.0",
        "name": "broken",
        "canvas": {"background_color": "#000", "default_border": null,
                   "default_connection": null, "theme_variables": {},
                   "theme_variants": {}},
        "nodes": {"0": {
            "id": "0", "parent_id": null,
            "position": {"x": 0, "y": 0},
            "size": {"width": 100, "height": 50},
            "sections": [],
            "style": {"background_color":"#000","frame_color":"#000",
                      "text_color":"#fff","shape":"rectangle",
                      "corner_radius_percent":0,"frame_thickness":0,
                      "show_frame":false,"show_shadow":false},
            "layout": {"type":"map","direction":"auto","spacing":0},
            "folded": false, "notes": "",
            "color_schema": null
        }},
        "edges": []
    }"##;

    /// The load result for a map the loader rejects, produced by the
    /// loader itself.
    fn rejected() -> (String, Result<MindMapDocument, String>) {
        let result = MindMapDocument::from_json_str(ZERO_SECTION_MAP, None);
        let message = result
            .as_ref()
            .err()
            .cloned()
            .expect("the zero-section fixture must be rejected");
        (message, result)
    }

    /// A load that succeeded is installed untouched — no placard, no
    /// rewriting of the document, and the file binding survives so
    /// `Ctrl+S` still knows where to write.
    #[test]
    fn test_successful_load_is_adopted_unchanged() {
        let path = crate::application::document::tests_common::test_map_path();
        let doc = MindMapDocument::load(&path.to_string_lossy()).expect("the testament fixture loads");
        let name = doc.mindmap.name.clone();
        let nodes = doc.mindmap.nodes.len();

        let surface = startup_surface("maps/testament.mindmap.json", Ok(doc));
        assert!(matches!(surface, StartupSurface::Loaded(_)));
        let (adopted, report) = resolve(surface);
        assert!(report.is_none(), "a successful load reports nothing: {report:?}");
        assert_eq!(adopted.mindmap.name, name);
        assert_eq!(adopted.mindmap.nodes.len(), nodes);
        assert!(adopted.file_path.is_some(), "a real load stays bound to its file");
    }

    /// A rejected load reports the loader's message verbatim, under
    /// §9's `"<area>: message"` prefix idiom once [`adopt`] stamps
    /// it. The canvas is the surface #107 is about, but the log line
    /// is what a terminal user and a bug report still get, so it is
    /// pinned to the same words rather than left to drift.
    #[test]
    fn test_rejected_load_reports_the_loader_message_verbatim() {
        let (message, result) = rejected();
        let (_, report) = resolve(startup_surface("broken.mindmap.json", result));
        assert_eq!(
            report.as_deref(),
            Some(message.as_str()),
            "the reported line must be the loader's message, not a paraphrase"
        );
    }

    /// **The fix, stated as a test.** A rejected load does not leave
    /// the shell with nothing to render: it gets a document, and
    /// that document carries the loader's own words.
    #[test]
    fn test_rejected_load_puts_the_loader_message_on_the_canvas() {
        let (message, result) = rejected();
        let doc = adopt(startup_surface("broken.mindmap.json", result));

        let text = doc
            .mindmap
            .nodes
            .values()
            .map(|n| {
                n.sections
                    .iter()
                    .map(|s| s.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect::<Vec<_>>()
            .join("\n");
        assert!(!text.trim().is_empty(), "the placard must not be blank");

        // The loader's message, not a paraphrase of it. Compared
        // word-by-word because the placard wraps to a column.
        let on_canvas = text.split_whitespace().collect::<Vec<_>>().join(" ");
        let expected = message.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            on_canvas.contains(&expected),
            "the placard does not carry the loader message.\nwanted: {expected}\ngot: {text}"
        );
        assert!(
            on_canvas.contains("broken.mindmap.json"),
            "the placard must name the map that failed: {text}"
        );
    }

    /// The placard is bound to no file, so the save paths — which
    /// both refuse a document without one — cannot write it over the
    /// file that failed to parse. That file is unparseable, which
    /// means it is very likely the only copy of whatever the author
    /// was hand-editing.
    #[test]
    fn test_placard_is_not_bound_to_the_file_that_failed() {
        let (_, result) = rejected();
        let doc = adopt(startup_surface("/home/user/precious.mindmap.json", result));
        assert!(doc.file_path.is_none(), "a placard must never be savable");
        assert!(!doc.dirty, "a placard is not unsaved work");
        assert_eq!(doc.mindmap.name, placard::PLACARD_MAP_NAME);
    }

    /// The placard is a live document, not a still image: the shell
    /// installs it through the same path as a real map, so the tree
    /// projection has to succeed on it. A placard that could not
    /// build a tree would render the same empty canvas #107 is
    /// about.
    #[test]
    fn test_placard_projects_into_a_renderable_tree() {
        let (_, result) = rejected();
        let doc = adopt(startup_surface("broken.mindmap.json", result));
        let tree = doc.build_tree();
        assert_eq!(tree.node_count(), 1, "the placard must project its one node");
        assert_eq!(
            tree.section_count_for(placard::PLACARD_NODE_ID),
            1,
            "the placard's text must project as a renderable section"
        );
    }

    /// **[`adopt`] still logs.** The placard is an addition to the
    /// stderr line, not a replacement for it — a terminal user and a
    /// bug report both want the line, and removing it would make
    /// this change a net loss for anyone who *does* have a terminal.
    ///
    /// Deleting that one statement is the only mutation of this
    /// module a runtime assertion cannot see: `log::error!` writes
    /// through a process-global logger the suite does not install,
    /// and standing one up would be a second logging sink for the
    /// tests to keep in step (TEST_CONVENTIONS §T10). So it is
    /// pinned against the source, the same way the parity of the two
    /// init paths is below.
    ///
    /// Only the half of the file above `#[cfg(test)]` is searched.
    /// That is not tidiness: the needle appears verbatim in this
    /// test's own body, so a whole-file scan would match itself and
    /// pass with `adopt` gutted — which is exactly the shape of
    /// non-test this exercise exists to catch. Whitespace-flattened
    /// so a re-wrap does not fail it; the prefix is asserted too,
    /// because §9 requires `"<area>: message"` and a bare message is
    /// what the old `run_native_init` arm emitted.
    #[test]
    fn test_adopt_still_reports_the_message_to_the_log() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src/application/app/startup_load.rs");
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
        let production = source
            .split_once("#[cfg(test)]")
            .map(|(before, _)| before)
            .expect("this file declares a `#[cfg(test)]` module");
        let flat = production.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains(r#"log::error!("startup: {}", message);"#),
            "`adopt` must still write the loader's message to the log under the \
             `\"<area>: message\"` prefix — the placard is an addition to that line, \
             not a replacement for it"
        );
    }

    /// **§9 records the decision, in the direction the code
    /// implements it.**
    ///
    /// #107's acceptance asked for exactly this: the convention
    /// prescribed `expect` for the initial `loader::load_from_file`
    /// and the code did something else, so one of the two had to
    /// move. §9 moved — but a paragraph is only a decision for as
    /// long as it survives the next edit of the file, and a reader
    /// reaching §9 for "what do I do when startup fails" has to find
    /// the answer this module implements rather than the one it
    /// replaced.
    ///
    /// Scoped to §9 and directional on purpose, in the same shape as
    /// `baumhard::util::log`'s pin of the release-log boundary in
    /// the same section: a whole-file `contains` would stay green
    /// when the paragraph drifts into another section, and naming
    /// the vocabulary without the direction would stay green when
    /// §9 says the opposite of what ships.
    #[test]
    fn test_code_conventions_section_9_records_the_startup_load_decision() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};

        let section = section_text(&repo_path("CODE_CONVENTIONS.md"), "## §9 Error handling");
        let flat = section.split_whitespace().collect::<Vec<_>>().join(" ");

        assert!(
            flat.contains("The initial map load is the one startup path that does not `expect`"),
            "CODE_CONVENTIONS.md §9 must carve the initial map load out of the `expect` \
             list — otherwise the convention still prescribes the crash this module exists \
             to avoid"
        );
        assert!(
            flat.contains("app::startup_load"),
            "CODE_CONVENTIONS.md §9 must name `app::startup_load` as where the carve-out \
             is implemented, so a reader can get from the rule to the code"
        );
        assert!(
            flat.contains("keep the shell alive"),
            "CODE_CONVENTIONS.md §9 must state the outcome — the shell survives a rejected \
             map — and not merely that `expect` is unwanted"
        );

        // The `expect` list itself must still exist and must still
        // hold the program-precondition entries. A §9 that dropped
        // them would read as "startup never panics", which is not
        // the decision and is not what ships.
        for precondition in ["`Renderer::new`", "`fonts::init`"] {
            assert!(
                flat.contains(precondition),
                "CODE_CONVENTIONS.md §9 must keep {precondition} in the `expect` list — the \
                 carve-out is for user data, not for broken program preconditions"
            );
        }
    }

    /// **Both init paths adopt through [`adopt`], and neither takes
    /// the surface apart itself.**
    ///
    /// CODE_CONVENTIONS §4 makes the two targets peers, and a
    /// startup-error surface that exists on one of them is not a fix.
    /// Runtime tests cannot see this: `cargo test` runs the native
    /// leg only (TEST_CONVENTIONS §T9), so the browser arm would be
    /// free to drift under a green suite. The check is therefore made
    /// against the sources, in the same idiom `util::serde_coverage`
    /// and `util::manifests` already use to assert properties of the
    /// repository rather than of a value.
    #[test]
    fn test_both_targets_adopt_the_startup_surface() {
        for relative in [
            "src/application/app/run_native_init.rs",
            "src/application/app/run_wasm/mod.rs",
        ] {
            let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(relative);
            let source = std::fs::read_to_string(&path)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
            assert!(
                source.contains("startup_load::adopt("),
                "{relative} must install its document through `startup_load::adopt` — \
                 otherwise the two targets can report a load failure differently"
            );
            assert!(
                !source.contains("StartupSurface::"),
                "{relative} must not take `StartupSurface` apart itself; `adopt` is the \
                 single place the rejected arm is handled, and duplicating that match is \
                 how the two targets drift"
            );
        }
    }
}
