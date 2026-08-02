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
//! ## One call per target, taking only the source
//!
//! `native_startup_document` and `wasm_startup_document` are the
//! whole surface. Each takes the path or URL and returns the document
//! to install; neither hands its caller a `Result` to look at. That
//! is deliberate and it is the part worth defending: while the init
//! sites composed the load themselves, the loader's message passed
//! through their hands, and blanking it on the browser leg alone was
//! a one-line change that left every test green and put a placard
//! with an empty diagnosis on the canvas. There is now no `Result` at
//! either init site to intercept.
//!
//! What stays browser-only is `fetch_map_json` and the two-line
//! `wasm_startup_document` wrapper — `cargo test` runs the native
//! leg only (`TEST_CONVENTIONS.md` §T9), so neither can be reached at
//! runtime from here. The composition between them is therefore
//! lifted into [`browser_load`], which is target-independent,
//! compiled on native and tested there against the exact error
//! strings the fetch produces. The residue — one `.await` and one
//! call — is pinned against the source by
//! `test_both_targets_go_through_one_startup_call`.

use crate::application::document::MindMapDocument;

/// The decision: given the result of the initial load, what goes on
/// the canvas.
///
/// Every arm carries a document, because the shell always has
/// something to render — that is the fix. The variants exist so the
/// *difference* between them stays explicit and exhaustively
/// matched: a third outcome (a map that loaded with warnings worth
/// showing, say) is a compile error in [`resolve`] rather than a
/// silent fallthrough back to the empty window.
///
/// Private to this module, which the single-call entry points above
/// are what make possible: nothing outside can build one, take one
/// apart, or interpose on the way to [`adopt`].
enum StartupSurface {
    /// The map loaded. Render it.
    Loaded(MindMapDocument),
    /// The map did not load. Render `placard` — which carries the
    /// loader's own text — and report [`report_line`].
    Rejected {
        /// The one-node map standing in for the map that failed.
        placard: MindMapDocument,
        /// The path or URL that was asked for.
        source: String,
        /// The loader's message, verbatim. Held separately from the
        /// placard so the log line and the canvas cannot disagree
        /// about what went wrong.
        message: String,
    },
}

/// Native entry point: load `path` and return the document to
/// install, reporting on the way past if it did not load.
///
/// The whole of what `run_native_init` does about the initial load.
/// It takes a path and gets a document — see the module docs for why
/// no `Result` crosses that boundary.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn native_startup_document(path: &str) -> MindMapDocument {
    adopt(startup_surface(path, MindMapDocument::load(path)))
}

/// Browser entry point: fetch `url` over the page origin, parse it,
/// and return the document to install, reporting on the way past if
/// either half failed.
///
/// The peer of `native_startup_document`, with the same signature
/// shape and the same guarantee (CODE_CONVENTIONS §4). Everything it
/// does other than the fetch is [`browser_load`], which is compiled
/// and tested on native.
#[cfg(target_arch = "wasm32")]
pub(crate) async fn wasm_startup_document(url: &str) -> MindMapDocument {
    let fetched = fetch_map_json(url).await;
    adopt(startup_surface(url, browser_load(url, fetched)))
}

/// Collapse the browser's two-stage load — fetch, then parse — into
/// the single `Result` the surface takes, tagging the document with
/// `url` so a later save-back knows where it came from.
///
/// Target-independent on purpose. The fetch is the caller's, so this
/// compiles on native and
/// `test_the_browser_leg_carries_every_fetch_message_through` runs it
/// against the real strings `fetch_map_json` emits, rather than
/// leaving the browser's only failure path to a source scan.
fn browser_load(url: &str, fetched: Result<String, String>) -> Result<MindMapDocument, String> {
    fetched.and_then(|json| MindMapDocument::from_json_str(&json, Some(url.to_string())))
}

/// Fetch `url` as text over the page origin. Browser-only:
/// `std::fs` does not exist there, and this is the counterpart to
/// native's `loader::load_from_file`.
///
/// Lives here rather than in `run_wasm` because it is one half of the
/// initial load, and the module that owns the initial load is the one
/// whose tests pin it.
///
/// **None of the five messages names `url`.** They do not have to:
/// [`report_line`] puts the source in front of whatever comes back,
/// once, for both targets — so a 404 reads `startup: could not load
/// 'maps/x.mindmap.json': HTTP 404 Not Found` rather than the bare
/// `HTTP 404 Not Found` that told a reader with more than one map
/// nothing.
#[cfg(target_arch = "wasm32")]
async fn fetch_map_json(url: &str) -> Result<String, String> {
    use wasm_bindgen::JsCast;
    let window = web_sys::window().ok_or("no global window")?;
    let promise = window.fetch_with_str(url);
    let resp_value = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("fetch failed: {:?}", e))?;
    let resp: web_sys::Response = resp_value
        .dyn_into()
        .map_err(|_| "fetch did not return a Response".to_string())?;
    if !resp.ok() {
        return Err(format!("HTTP {} {}", resp.status(), resp.status_text()));
    }
    let text_promise = resp
        .text()
        .map_err(|e| format!("Response::text() failed: {:?}", e))?;
    wasm_bindgen_futures::JsFuture::from(text_promise)
        .await
        .map_err(|e| format!("reading response body failed: {:?}", e))?
        .as_string()
        .ok_or_else(|| "response body was not a string".to_string())
}

/// Decide what the shell shows for `load`, the result of the initial
/// map load of `source` (the CLI path on native, the `?map=` URL in
/// the browser).
///
/// Pure: builds a document either way and reports nothing. The
/// reporting lives in [`adopt`], which is what makes this function
/// assertable line by line without a logger or a device.
fn startup_surface(source: &str, load: Result<MindMapDocument, String>) -> StartupSurface {
    match load {
        Ok(document) => StartupSurface::Loaded(document),
        Err(message) => StartupSurface::Rejected {
            placard: MindMapDocument::load_failure_placard(source, &message),
            source: source.to_string(),
            message,
        },
    }
}

/// The line a rejected load reports: the source that was asked for,
/// then the loader's own message.
///
/// **The source is here because nothing else puts it there.** Exactly
/// one of the loader's failure modes names the file —
/// `load_from_file`'s *read* error — while `load_from_str`'s parse
/// errors and all five of `fetch_map_json`'s never do. Without this
/// the log read `startup: Failed to parse mindmap JSON: key must be a
/// string at line 1 column 3`, which does not say *which* map, and a
/// line that cannot be pasted into a bug report is not worth
/// emitting. The placard already names the source on the canvas; this
/// is the copy for whoever is reading `stderr` or the browser
/// console.
///
/// Quoted because a path may contain spaces and what follows it is
/// prose.
fn report_line(source: &str, message: &str) -> String {
    format!("could not load '{source}': {message}")
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
fn adopt(surface: StartupSurface) -> MindMapDocument {
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
        StartupSurface::Rejected {
            placard,
            source,
            message,
        } => (placard, Some(report_line(&source, &message))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use baumhard::mindmap::placard;
    use baumhard::util::rust_source::{braced_block_after, production_code};

    /// This module's own path, for the pins that read it.
    const THIS_FILE: &str = "src/application/app/startup_load.rs";

    /// The two init files. Each must reach the canvas through one
    /// call into this module and nothing else.
    const INIT_FILES: &[(&str, &str)] = &[
        (
            "src/application/app/run_native_init.rs",
            "startup_load::native_startup_document(&options.mindmap_path)",
        ),
        (
            "src/application/app/run_wasm/mod.rs",
            "startup_load::wasm_startup_document(&mindmap_path).await",
        ),
    ];

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
        let report = report.expect("a rejected load must report");
        assert!(
            report.contains(&message),
            "the reported line must carry the loader's message, not a paraphrase.\n\
             wanted to find: {message}\ngot: {report}"
        );
    }

    /// **The reported line names the map.** The loader names the file
    /// in exactly one of its failure modes — the *read* error — and
    /// in none of the others: a parse failure says `Failed to parse
    /// mindmap JSON: …` and the browser's fetch says `HTTP 404 Not
    /// Found`, neither of which tells a reader with more than one map
    /// open which one died. The placard carries the source on the
    /// canvas; this pins the copy that goes to `stderr` and the
    /// browser console, which is where a bug report is pasted from.
    ///
    /// Runtime, not a source scan: [`report_line`] is pure and
    /// compiled on both targets, so the guarantee is executed rather
    /// than read.
    #[test]
    fn test_the_reported_line_names_the_map_that_failed() {
        // The parse failure is the case that regressed: its message
        // is produced by `load_from_str`, which never sees a path.
        let (message, result) = rejected();
        assert!(
            !message.contains("precious"),
            "fixture assumption broken — the loader message must not already name the source: {message}"
        );
        let (_, report) = resolve(startup_surface("/home/user/precious.mindmap.json", result));
        let report = report.expect("a rejected load must report");
        assert!(
            report.contains("/home/user/precious.mindmap.json"),
            "the reported line must name the map that failed: {report}"
        );
        assert!(
            report.contains(&message),
            "…without dropping the loader's own words: {report}"
        );

        // Every shape the browser's fetch can produce is a bare
        // diagnosis with no URL in it. Same guarantee, same line.
        for bare in [
            "HTTP 404 Not Found",
            "fetch failed: JsValue(TypeError)",
            "fetch did not return a Response",
            "Response::text() failed: JsValue(TypeError)",
            "reading response body failed: JsValue(TypeError)",
            "response body was not a string",
        ] {
            let line = report_line("maps/testament.mindmap.json", bare);
            assert!(
                line.contains("maps/testament.mindmap.json") && line.contains(bare),
                "{bare:?} must reach the log with its source attached, got {line:?}"
            );
        }
    }

    /// **The browser leg keeps the loader's words.** Blanking the
    /// message on that leg alone used to be a one-line edit at the
    /// init site that no test could see, because `cargo test` runs
    /// the native leg only (TEST_CONVENTIONS §T9).
    ///
    /// [`browser_load`] is the whole of that leg apart from the
    /// `await`, and it is target-independent precisely so this can be
    /// *executed* rather than scanned: each of the six strings the
    /// fetch can return, and a parse failure of a body that did
    /// arrive, must come out the far end intact.
    #[test]
    fn test_the_browser_leg_carries_every_fetch_message_through() {
        const URL: &str = "maps/from-the-query-string.mindmap.json";
        for bare in [
            "HTTP 404 Not Found",
            "fetch failed: JsValue(TypeError)",
            "fetch did not return a Response",
            "Response::text() failed: JsValue(TypeError)",
            "reading response body failed: JsValue(TypeError)",
            "response body was not a string",
        ] {
            let (_, report) = resolve(startup_surface(URL, browser_load(URL, Err(bare.to_string()))));
            let report = report.expect("a failed fetch must report");
            assert!(
                report.contains(bare) && report.contains(URL),
                "the browser leg lost {bare:?} or its URL: {report}"
            );
        }

        // A body that arrived and did not parse: the loader's own
        // message, not the fetch's.
        let (parse_message, _) = rejected();
        let (_, report) = resolve(startup_surface(
            URL,
            browser_load(URL, Ok(ZERO_SECTION_MAP.to_string())),
        ));
        let report = report.expect("a rejected body must report");
        assert!(
            report.contains(&parse_message) && report.contains(URL),
            "the browser leg lost the parse message or its URL: {report}"
        );

        // A body that *did* parse keeps its origin, so the browser's
        // save-back knows where the document came from.
        let good = std::fs::read_to_string(crate::application::document::tests_common::test_map_path())
            .expect("the testament fixture is readable");
        let doc = browser_load(URL, Ok(good)).expect("the testament fixture parses");
        assert_eq!(doc.file_path.as_deref(), Some(URL));
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

    /// **The native entry point, end to end.** Everything above tests
    /// a piece; this runs the function `run_native_init` actually
    /// calls, on a path that does not exist, and checks the shell
    /// gets something to render.
    #[test]
    fn test_native_startup_document_returns_a_placard_for_a_map_that_is_not_there() {
        let missing = "/nonexistent/definitely-not-here.mindmap.json";
        let doc = native_startup_document(missing);
        assert_eq!(doc.mindmap.name, placard::PLACARD_MAP_NAME);
        assert!(doc.file_path.is_none(), "a placard must never be savable");
        let text = &doc.mindmap.nodes[placard::PLACARD_NODE_ID].sections[0].text;
        let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(flat.contains(missing), "the placard must name the map: {text}");
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

    /// **A placard is not announced as a map that loaded.** Under
    /// `RUST_LOG=info` the shell used to print
    /// `document: loaded mindmap "load-failure" with 1 node(s)`
    /// immediately beside `startup: could not load …`, because
    /// `MindMapDocument::finalize` carried the announcement and the
    /// placard goes through `finalize` like any other map. A log
    /// that contradicts itself in two consecutive lines is worse
    /// than a quiet one.
    ///
    /// The same shape as the pin above, for the same reason — an
    /// `info!` has no sink the suite installs — and the same two
    /// scopings: [`production_code`] strips the comments that quote
    /// these names, and [`braced_block_after`] asks the question of
    /// one function body at a time. Both directions are checked, so
    /// deleting the announcement outright fails here too.
    #[test]
    fn test_the_placard_is_not_announced_as_a_loaded_map() {
        const DOCUMENT: &str = "src/application/document/mod.rs";
        let code = production_code(DOCUMENT);
        // A file whose test modules are all external: if the reader
        // ever truncates at one of those declarations it hands back
        // nothing, and every assertion below passes vacuously.
        assert!(
            code.contains("fn finalize("),
            "{DOCUMENT} did not reach this test — the reader truncated it"
        );

        for (item, announces) in [
            ("pub fn load(", true),
            ("pub fn from_json_str(", true),
            ("pub fn load_failure_placard(", false),
            ("fn finalize(", false),
        ] {
            let body = braced_block_after(&code, item)
                .unwrap_or_else(|| panic!("{DOCUMENT} must still declare `{item}`"));
            assert_eq!(
                body.contains("log_loaded("),
                announces,
                "`{item}` {} announce the load. The line belongs to the constructors that \
                 really loaded a map — not to `finalize`, which the load-failure placard also \
                 goes through. Body seen: {body}",
                if announces { "must" } else { "must not" }
            );
        }
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
    /// tests to keep in step (TEST_CONVENTIONS §T10).
    ///
    /// Scoped twice, by [`production_code`] and
    /// [`braced_block_after`]. Comments are gone, so gutting `adopt`
    /// and leaving `// this used to be log::error!(…)` fails — that
    /// is not a contrived mutation, it is the ordinary shape of a
    /// deletion. Test modules are gone — including the ones written
    /// under a run of attributes, which is the shape that let a shim
    /// supply a needle for a while — so the needle spelled out in
    /// this very body cannot satisfy the scan. And the search is the
    /// body of `adopt` alone, so moving the statement out of the
    /// function it belongs to fails too. Whitespace-flattened so a
    /// re-wrap does not fail it; the prefix is asserted with it,
    /// because §9 requires `"<area>: message"` and a bare message is
    /// what the old `run_native_init` arm emitted.
    #[test]
    fn test_adopt_still_reports_the_message_to_the_log() {
        let code = production_code(THIS_FILE);
        let body = braced_block_after(&code, "fn adopt(")
            .unwrap_or_else(|| panic!("{THIS_FILE} must still declare `fn adopt(`"));
        let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
        assert!(
            flat.contains(r#"log::error!("startup: {}", message);"#),
            "`adopt` must still write the report to the log under the \
             `\"<area>: message\"` prefix — the placard is an addition to that line, \
             not a replacement for it. Body seen: {flat}"
        );
    }

    /// **Both targets reach the canvas through one call, and hand it
    /// nothing but the source.**
    ///
    /// CODE_CONVENTIONS §4 makes the two targets peers, and a
    /// startup-error surface that exists on one of them is not a fix.
    /// Runtime tests cannot see this: `cargo test` runs the native
    /// leg only (TEST_CONVENTIONS §T9), so the browser arm would be
    /// free to drift under a green suite.
    ///
    /// What is *not* asserted here, because the compiler asserts it:
    /// `StartupSurface`, `startup_surface` and `adopt` are private to
    /// this module, so no init site can build a surface, take one
    /// apart, or interpose between the loader and the log. That used
    /// to be a `!source.contains("StartupSurface::")` and it is now a
    /// visibility.
    ///
    /// What is left to check is the *argument*, and it is checked
    /// exactly: each init file must contain its entry call spelled
    /// with the source and nothing else. `.map_err(…)` on the way in
    /// — the mutation that blanked the browser's diagnosis while
    /// every test stayed green — changes that text and fails here.
    /// The scan reads [`production_code`], so neither a comment nor
    /// an appended test module can supply the needle.
    #[test]
    fn test_both_targets_go_through_one_startup_call() {
        for (relative, call) in INIT_FILES {
            let code = production_code(relative);
            let flat = code.split_whitespace().collect::<Vec<_>>().join(" ");
            assert!(
                flat.contains(call),
                "{relative} must install its document with exactly `{call}` — anything \
                 between the loader and `startup_load` is a place the two targets can \
                 report the same failure differently"
            );
        }

        // The corollary: neither init file still loads for itself.
        // A second, unrouted load is how the old arm would come back.
        let native = production_code(INIT_FILES[0].0);
        assert!(
            !native.contains("MindMapDocument::load("),
            "run_native_init must not load the map itself; `native_startup_document` does"
        );
        let wasm = production_code(INIT_FILES[1].0);
        for own_work in ["from_json_str(", "fetch_map_json("] {
            assert!(
                !wasm.contains(own_work),
                "run_wasm must not do the load itself ({own_work}); \
                 `wasm_startup_document` does"
            );
        }
    }

    /// Where §9's startup roster must resolve to, and whether §9
    /// classes each entry as a site that *can fail* — which is a
    /// claim about the code, and so is checked against the code: a
    /// fallible site must appear in a statement that also carries an
    /// `.expect(`.
    ///
    /// The list exists because §9's used to name `Renderer::new`,
    /// which the bootstrap split replaced, and `fonts::init`, which
    /// returns `()` and has no failure to report. Both survived
    /// review because nothing checked them.
    const STARTUP_SITES: &[(&str, &str, bool)] = &[
        ("EventLoop::new", "src/application/app/run_native.rs", true),
        ("create_window", "src/application/app/run_native.rs", true),
        ("create_surface", "src/application/renderer/mod.rs", true),
        ("web_sys::window", "src/application/app/run_wasm/mod.rs", true),
        (
            "Renderer::bootstrap_native",
            "src/application/app/run_native_init.rs",
            false,
        ),
        (
            "Renderer::bootstrap_wasm",
            "src/application/app/run_wasm/mod.rs",
            false,
        ),
        ("fonts::init", "src/application/app/run_native_init.rs", false),
        ("strip_prefix", "src/application/app/run_wasm/mod.rs", false),
    ];

    /// Backticked words in the `expect` bullet that are Rust
    /// *vocabulary* rather than startup sites — they are what the
    /// rule is about, not entries in its list.
    const NOT_A_SITE: &[&str] = &["expect", "unwrap"];

    /// The bold lead of §9's `expect` bullet, verbatim.
    const EXPECT_LEAD: &str = "**Startup paths use `expect(\"<reason>\")` with a human-readable message.**";

    /// The bold lead of §9's carve-out bullet, verbatim. This is the
    /// sentence that states the direction of the decision, and it is
    /// pinned by equality rather than by `contains` for that reason.
    const CARVE_OUT_LEAD: &str =
        "**The initial map load is the one startup path that does not `expect`, and the reason names the boundary.**";

    /// Split a Markdown section into its top-level `- ` bullets, each
    /// whitespace-flattened with its continuation lines folded in.
    fn bullets(section: &str) -> Vec<String> {
        let mut raw: Vec<String> = Vec::new();
        for line in section.lines() {
            if let Some(rest) = line.strip_prefix("- ") {
                raw.push(rest.to_string());
            } else if let Some(last) = raw.last_mut() {
                last.push(' ');
                last.push_str(line.trim());
            }
        }
        raw.iter()
            .map(|b| b.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    }

    /// The one bullet of `section` whose flattened text starts with
    /// `lead`. Panics naming both, because a caller that cannot find
    /// its bullet has found the drift it was looking for.
    fn bullet_led_by<'a>(bullets: &'a [String], lead: &str) -> &'a str {
        bullets
            .iter()
            .find(|b| b.starts_with(lead))
            .map(String::as_str)
            .unwrap_or_else(|| {
                panic!(
                    "CODE_CONVENTIONS.md §9 no longer opens a bullet with:\n  {lead}\nbullet leads seen:\n{}",
                    bullets
                        .iter()
                        .map(|b| format!("  {}", b.split(".**").next().unwrap_or(b)))
                        .collect::<Vec<_>>()
                        .join("\n")
                )
            })
    }

    /// Every backticked span in `text`.
    fn backticked(text: &str) -> Vec<&str> {
        text.split('`').skip(1).step_by(2).collect()
    }

    /// Whether `token` is a bare Rust identifier or `::` path — the
    /// shape a symbol reference takes. `expect("<reason>")` and
    /// `?map=` are not.
    fn is_path_shaped(token: &str) -> bool {
        !token.is_empty()
            && token.split("::").all(|seg| {
                !seg.is_empty()
                    && seg.starts_with(|c: char| c.is_ascii_alphabetic() || c == '_')
                    && seg.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            })
    }

    /// `code` split into statements, each whitespace-flattened.
    /// Crude on purpose — the question asked of it is only "does
    /// *this* call carry an `.expect(`", and a statement is the unit
    /// that answers it.
    fn statements(code: &str) -> Vec<String> {
        code.split(';')
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .collect()
    }

    /// **§9's `expect` roster names things that exist, and only
    /// things that exist.**
    ///
    /// The bullet used to enumerate `Renderer::new` — replaced by
    /// `bootstrap_native` / `bootstrap_wasm` — and `fonts::init`,
    /// which returns `()` and so has nothing to `expect` on. Neither
    /// was caught, because a convention that lists code is only as
    /// accurate as the last person who checked, and nobody had.
    ///
    /// Three checks, in the order they fail:
    ///
    /// 1. every path-shaped name the bullet backticks is in
    ///    [`STARTUP_SITES`] — so adding a stale one fails here rather
    ///    than surviving a decade;
    /// 2. every roster entry is *called* in the file the roster names
    ///    — so a rename fails;
    /// 3. every call of an entry the roster marks fallible sits in a
    ///    statement that also carries an `.expect(` — so listing
    ///    something that cannot fail fails, and so does one arm of a
    ///    two-arm site quietly dropping its reason.
    ///
    /// All three read [`production_code`], so a name that survives
    /// only in a comment does not count as surviving.
    #[test]
    fn test_code_conventions_section_9_expect_roster_names_real_sites() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};

        let section = section_text(&repo_path("CODE_CONVENTIONS.md"), "## §9 Error handling");
        let all = bullets(&section);
        let bullet = bullet_led_by(&all, EXPECT_LEAD);

        for token in backticked(bullet).into_iter().filter(|t| is_path_shaped(t)) {
            if NOT_A_SITE.contains(&token) {
                continue;
            }
            assert!(
                STARTUP_SITES.iter().any(|(name, _, _)| *name == token),
                "CODE_CONVENTIONS.md §9 names `{token}` as a startup site, but this test's \
                 roster does not know it. Add it here with the file it lives in and whether \
                 it can fail — or take it out of §9, which is what `Renderer::new` needed."
            );
        }

        for (name, relative, fallible) in STARTUP_SITES {
            assert!(
                bullet.contains(&format!("`{name}`")),
                "CODE_CONVENTIONS.md §9's `expect` bullet must still name `{name}`"
            );
            let code = production_code(relative);
            // The paren makes this a *call* rather than a mention:
            // without it `create_surface` also matches
            // `create_surface_config`, which is infallible and would
            // answer the next question wrongly.
            let call = format!("{name}(");
            let calls: Vec<String> = statements(&code)
                .into_iter()
                .filter(|s| s.contains(&call))
                .collect();
            assert!(
                !calls.is_empty(),
                "§9 names `{name}` but {relative} never calls it — the convention is \
                 pointing at something that moved"
            );
            // *Every* call, not merely one of them. `create_surface`
            // is reached from both `bootstrap_native` and
            // `bootstrap_wasm`; an `any` here would let one of the
            // two stop saying why it died while §9 goes on claiming
            // both do.
            let all_expect = calls.iter().all(|s| s.contains(".expect("));
            assert_eq!(
                all_expect,
                *fallible,
                "§9 classes `{name}` as {}, but {relative} says otherwise: a fallible startup \
                 site is one whose every call carries an `.expect(\"<reason>\")`. \
                 Calls seen: {calls:#?}",
                if *fallible {
                    "a site that can fail"
                } else {
                    "a site that cannot fail"
                }
            );
        }
    }

    /// **§9 records the decision, in the direction the code
    /// implements it.**
    ///
    /// #107's acceptance asked for exactly this: the convention
    /// prescribed `expect` for the initial `loader::load_from_file`
    /// and the code did something else, so one of the two had to
    /// move. §9 moved — but a paragraph is only a decision for as
    /// long as it survives the next edit of the file.
    ///
    /// **What "directional" can and cannot mean here.** No test reads
    /// prose. An earlier version of this one asserted a handful of
    /// `contains` on flattened §9 text and called itself directional;
    /// it was not — a §9 rewritten to say the rule was *withdrawn*
    /// kept every needle and stayed green. Three things replace that:
    ///
    /// - the bullet's **bold lead** is pinned by equality, not
    ///   `contains`. The lead is where the rule is stated, and a
    ///   negation cannot be wrapped around a string that has to match
    ///   exactly;
    /// - the `expect` bullet must **not** name the initial load. "The
    ///   initial load now `expect`s" has to be written down
    ///   somewhere, and that list is where;
    /// - the claim itself is checked **against the code**: the two
    ///   init sites must carry no `expect`/`unwrap` on the startup
    ///   document, and the two modules §9 names must exist. A §9 that
    ///   says the opposite of the code is then a §9 that disagrees
    ///   with a green suite.
    ///
    /// The residual, stated plainly: a bullet that keeps its lead
    /// verbatim and contradicts it in its own body is out of reach of
    /// any text test, and is a self-refuting document rather than a
    /// drift.
    #[test]
    fn test_code_conventions_section_9_records_the_startup_load_decision() {
        use baumhard::util::doc_fixtures::{repo_path, section_text};

        let section = section_text(&repo_path("CODE_CONVENTIONS.md"), "## §9 Error handling");
        let all = bullets(&section);
        let carve_out = bullet_led_by(&all, CARVE_OUT_LEAD);
        let expect_bullet = bullet_led_by(&all, EXPECT_LEAD);

        // The `expect` list is where "the initial load now `expect`s"
        // would have to be written. It must not be.
        for excluded in [
            "load_from_file",
            "from_json_str",
            "initial map load",
            "native_startup_document",
            "wasm_startup_document",
        ] {
            assert!(
                !expect_bullet.contains(excluded),
                "CODE_CONVENTIONS.md §9's `expect` bullet names {excluded:?} — the initial map \
                 load is carved *out* of that list, and putting it back is the reversal this \
                 test exists to catch"
            );
        }

        // The carve-out has to say where it is implemented, or a
        // reader cannot get from the rule to the code.
        for named in [
            "app::startup_load",
            "baumhard::mindmap::placard",
            "keep the shell alive",
        ] {
            assert!(
                carve_out.contains(named),
                "CODE_CONVENTIONS.md §9's carve-out must name {named:?}"
            );
        }

        // …and what it names has to be there.
        for module in [
            "src/application/app/startup_load.rs",
            "lib/baumhard/src/mindmap/placard.rs",
        ] {
            assert!(
                repo_path(module).is_file(),
                "§9 points at {module}, which does not exist"
            );
        }

        // The claim, checked against the code: no init site turns a
        // rejected map into a panic. This is the half that a §9
        // rewritten to say the opposite cannot make true.
        for (relative, call) in INIT_FILES {
            let code = production_code(relative);
            let statement = statements(&code)
                .into_iter()
                .find(|s| s.contains("startup_document("))
                .unwrap_or_else(|| panic!("{relative} no longer calls a startup entry point"));
            assert!(
                statement.contains(call),
                "{relative}'s startup statement is {statement:?}, not {call:?}"
            );
            for panicking in [".expect(", ".unwrap(", "panic!("] {
                assert!(
                    !statement.contains(panicking),
                    "{relative} turns the initial load into a panic with {panicking} — §9 says \
                     it does not, and §9 is the decision this module implements: {statement}"
                );
            }
        }
    }
}
