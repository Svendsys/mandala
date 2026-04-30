// SPDX-License-Identifier: MPL-2.0

//! WASM event-loop body for [`super::Application::run`]. Browser
//! thread owned by winit-web's loop; on shutdown the closure returns
//! and winit propagates any internal failure.

#![cfg(target_arch = "wasm32")]

use super::*;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::Action;
use baumhard::mindmap::tree_builder::MindMapTree;

/// Pending left-click awaiting a release. `None` on init and after
/// release consumed; `Empty` after a click-down on empty canvas;
/// `Node(id)` after a click-down on a node. Full drag machine
/// (pan, move, reparent, connect) deferred to a later
/// WASM-parity session.
///
/// Module-level (not closure-local) so helpers in sibling modules
/// can take `&WasmInputState` parameters that include the field.
pub(super) enum PendingClick {
    None,
    Empty,
    Node(String),
    /// Cursor landed on a portal **icon** at mouse-down. Committed
    /// at mouse-up into a `SelectionState::PortalLabel`. Carries
    /// both the owning-edge key and the endpoint id the marker
    /// belongs to, matching the native click dispatch surface.
    PortalMarker {
        edge_key: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint_node_id: String,
    },
    /// Cursor landed on a portal **text** at mouse-down.
    /// Committed at mouse-up into a `SelectionState::PortalText`.
    /// Shares the identity shape with `PortalMarker`; only the
    /// mouse-up selection routing differs.
    PortalText {
        edge_key: baumhard::mindmap::scene_cache::EdgeKey,
        endpoint_node_id: String,
    },
    /// Cursor landed on a line-mode edge's label AABB at
    /// mouse-down. Committed at mouse-up into
    /// `SelectionState::EdgeLabel` so per-label color / font /
    /// copy operations target the label instead of the edge
    /// body. Double-click is handled inline by the press-time
    /// dispatcher — WASM doesn't open the inline editor modal
    /// yet so the dbl-click branch falls back to the same
    /// selection commit for parity with single click.
    EdgeLabel(baumhard::mindmap::scene_cache::EdgeKey),
}

/// Shared state between the WASM rAF render loop and the winit
/// event loop. Mirrors native's `InputHandlerContext` for the 9
/// fields both targets share. Promoted to module-level (was
/// closure-local in `run`) so the `cross_dispatch` /
/// `dispatch_macro_core` helpers can take `&mut WasmInputState`
/// directly — closure-local types aren't reachable from sibling
/// modules.
pub(super) struct WasmInputState {
    pub document: MindMapDocument,
    pub mindmap_tree: Option<MindMapTree>,
    pub text_edit_state: TextEditState,
    pub last_click: Option<LastClick>,
    pub cursor_pos: (f64, f64),
    pub pending_click: PendingClick,
    pub modifiers: winit::keyboard::ModifiersState,
    /// Mirror of native's `app_scene` so the canvas-scene tree
    /// path (borders, eventually connections / portals) works
    /// identically on WASM. Threaded into every
    /// `rebuild_all` / `rebuild_scene_only` call below.
    pub app_scene: crate::application::scene_host::AppScene,
    /// Mirror of native's `scene_cache` so the cache-aware
    /// `build_scene_with_cache` entry point skips `sample_path`
    /// work for unchanged edges. Threaded into every rebuild
    /// helper the same way native does.
    pub scene_cache: baumhard::mindmap::scene_cache::SceneConnectionCache,
    /// Macro registry. Mirrors native's `InputHandlerContext::macros`
    /// — App + User tiers loaded at startup; Map + Inline tiers
    /// refreshed by `loader::rebuild_document_macros` whenever a
    /// document is loaded. Consulted by the keyboard handler's
    /// Action → Macro → CustomMutation fall-through.
    pub macros: crate::application::macros::MacroRegistry,
}

impl WasmInputState {
    /// Build the cross-platform `InputContextCore` view for
    /// `dispatch_action_core::dispatch_compatible`. `renderer` and
    /// `keybinds` aren't fields on `WasmInputState` (held in the
    /// closure scope of `run`); the caller passes them in. Mirrors
    /// native's `InputHandlerContext::split_borrow` shape — short
    /// `'s` lifetime tied to `&mut self`, dropped before the next
    /// re-borrow of `self`.
    pub(super) fn input_context_core<'s>(
        &'s mut self,
        renderer: &'s mut Renderer,
        keybinds: &'s crate::application::keybinds::ResolvedKeybinds,
    ) -> super::input_context_core::InputContextCore<'s> {
        super::input_context_core::InputContextCore {
            document: Some(&mut self.document),
            mindmap_tree: &mut self.mindmap_tree,
            app_scene: &mut self.app_scene,
            renderer,
            scene_cache: &mut self.scene_cache,
            text_edit_state: &mut self.text_edit_state,
            last_click: &mut self.last_click,
            cursor_pos: &mut self.cursor_pos,
            modifiers: &self.modifiers,
            keybinds,
            macros: &mut self.macros,
        }
    }
}

/// WASM impl of `MacroDispatchTarget`. Wraps `&mut WasmInputState`
/// + `&mut Renderer` and forwards each operation to the same
/// helpers the keyboard handler uses. Privilege gating happens in
/// `dispatch_macro_core::dispatch_macro` (single-source contract).
struct WasmMacroDispatchTarget<'a> {
    input: &'a mut WasmInputState,
    renderer: &'a mut Renderer,
    /// `keybinds` belongs in `InputContextCore` (Track C); WASM
    /// holds its `ResolvedKeybinds` in the closure scope of `run`,
    /// not on `WasmInputState`. Threaded here so
    /// `MacroDispatchTarget::dispatch_action` can build the core.
    keybinds: &'a crate::application::keybinds::ResolvedKeybinds,
}

impl<'a> super::dispatch_macro_core::MacroDispatchTarget for WasmMacroDispatchTarget<'a> {
    fn registry(&self) -> &crate::application::macros::MacroRegistry {
        &self.input.macros
    }

    fn dispatch_action(&mut self, action: Action) -> super::cross_dispatch::DispatchOutcome {
        // Track C: route through the unified cross-platform
        // dispatcher. Pre-Track-C this called the WASM-only
        // `dispatch_compatible_action_wasm` shim; that shim is
        // deleted and both the keyboard handler and this trait
        // impl reach `dispatch_action_core::dispatch_compatible`.
        // The mixed-branch lift below restores `Handled` returns
        // for `CancelMode`/`EditSelection*` so the macro loop's
        // `any_ran` flag bumps correctly — see
        // `lift_mixed_branch_for_wasm_macro`'s rustdoc.
        let outcome = {
            let mut core = self.input.input_context_core(self.renderer, self.keybinds);
            super::dispatch_action_core::dispatch_compatible(&action, &mut core)
        };
        super::dispatch_action_core::lift_mixed_branch_for_wasm_macro(&action, outcome)
    }

    fn apply_custom_mutation(&mut self, id: &str, node_id: &str) -> bool {
        let cm = self
            .input
            .document
            .mutation_registry
            .get(id)
            .cloned();
        let Some(cm) = cm else {
            log::warn!("macro step: unknown custom-mutation id '{}'", id);
            return false;
        };
        let now = super::now_ms() as u64;
        let applied = super::cross_dispatch::apply_keybind_custom_mutation(
            &mut self.input.document,
            &mut self.input.mindmap_tree,
            &mut self.input.scene_cache,
            &cm,
            node_id,
            now,
        );
        if applied {
            // Match native pre-Track-B: rebuild via plain
            // `rebuild_all` (no extra `scene_cache.clear()`).
            // `apply_keybind_custom_mutation` already cleared the
            // cache on the non-animated branch; the animated
            // branch deliberately leaves it for the animation
            // envelope to invalidate. Going through
            // `rebuild_after_geometry_change` here would clear
            // again on non-animated and add an unwanted clear on
            // animated.
            super::scene_rebuild::rebuild_all(
                &self.input.document,
                &mut self.input.mindmap_tree,
                &mut self.input.app_scene,
                self.renderer,
                &mut self.input.scene_cache,
            );
            true
        } else {
            false
        }
    }

    fn execute_console_line(&mut self, line: &str) -> bool {
        // WASM has no `execute_console_line` runtime
        // (`console_input` is `cfg(not(target_arch = "wasm32"))`).
        // The privilege gate already rejects ConsoleLine from
        // non-User tiers above this; User-tier ConsoleLine on WASM
        // logs warn + skips per `format/macros.md` §
        // ConsoleLine on WASM. The macro continues to the next
        // step — fail-soft, matching the User-tier "step failed"
        // posture native uses for unknown CustomMutation ids.
        //
        // Returns `false` so the macro's `any_ran` flag doesn't
        // bump on the no-op path — a `[ConsoleLine]`-only macro
        // on WASM correctly reports "didn't fire" so the keystroke
        // isn't artificially consumed. Mirrors native's pre-doc-
        // load posture (false return on the warn arm).
        log::warn!(
            "macros: ConsoleLine step '{}' has no console runtime on WASM; skipping",
            line,
        );
        false
    }

    fn current_selection_node_id(&self) -> Option<String> {
        if let crate::application::document::SelectionState::Single(nid) =
            &self.input.document.selection
        {
            Some(nid.clone())
        } else {
            None
        }
    }

    fn has_node(&self, node_id: &str) -> bool {
        self.input.document.mindmap.nodes.contains_key(node_id)
    }
}

/// Run the browser event loop against `app`.
pub(super) fn run(mut app: Application) {
use wasm_bindgen::JsCast;
use winit::platform::web::WindowExtWebSys;
use std::rc::Rc;
use std::cell::{Cell, RefCell};
use baumhard::mindmap::tree_builder::MindMapTree;

baumhard::font::fonts::init();

// Load keybindings from the WASM environment (URL query param or
// localStorage) with a defaults fallback. Failure is non-fatal —
// see KeybindConfig::load_for_web().
app.options.keybind_config =
    crate::application::keybinds::KeybindConfig::load_for_web();

// Attach canvas to DOM
let canvas = app.window.canvas().expect("Failed to get canvas");
let web_window = web_sys::window().expect("No global window");
let document = web_window.document().expect("No document");
let body = document.body().expect("No body");
body.append_child(&canvas).expect("Failed to append canvas");
canvas.set_width(web_window.inner_width().unwrap().as_f64().unwrap() as u32);
canvas.set_height(web_window.inner_height().unwrap().as_f64().unwrap() as u32);
let cw = canvas.width();
let ch = canvas.height();
log::info!("WASM: canvas sized {}x{}", cw, ch);
if cw == 0 || ch == 0 {
    log::warn!(
        "WASM: canvas has zero dimension — render surface will be empty"
    );
}

// Canvas must be focusable for keyboard events to reach winit.
// Without tabindex, an HTMLCanvasElement never receives focus.
canvas.set_attribute("tabindex", "0").ok();
let _ = canvas.focus();

// Re-focus on mousedown so clicking the canvas after tabbing
// to another element restores keyboard input.
{
    let canvas_for_focus = canvas.clone();
    let focus_cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |_: web_sys::Event| {
            let _ = canvas_for_focus.focus();
        },
    );
    canvas
        .add_event_listener_with_callback("mousedown", focus_cb.as_ref().unchecked_ref())
        .ok();
    focus_cb.forget(); // leak — lives for the page lifetime
}

// preventDefault on keydown while the text editor is open so
// Tab/Enter/Backspace/arrows don't fire browser defaults
// (tab-navigation, history-back, page-scroll).
let suppress_keys: Rc<Cell<bool>> = Rc::new(Cell::new(false));
{
    let suppress = suppress_keys.clone();
    let pd_cb = wasm_bindgen::closure::Closure::<dyn FnMut(web_sys::Event)>::new(
        move |evt: web_sys::Event| {
            if suppress.get() {
                evt.prevent_default();
            }
        },
    );
    canvas
        .add_event_listener_with_callback("keydown", pd_cb.as_ref().unchecked_ref())
        .ok();
    pd_cb.forget();
}

let renderer_window = Arc::clone(&app.window);

// On WASM, check for ?map= query parameter to override the default path
let mindmap_path = {
    let web_window = web_sys::window().expect("No global window");
    let search = web_window.location().search().unwrap_or_default();
    let mut map_path: Option<String> = None;
    let trimmed = search.trim_start_matches('?');
    for pair in trimmed.split('&') {
        if let Some(val) = pair.strip_prefix("map=") {
            map_path = Some(val.to_string());
        }
    }
    map_path.unwrap_or_else(|| app.options.mindmap_path.clone())
};

// Shared state between the rAF render loop and the winit event
// loop. Two RefCells so input handlers can borrow InputState
// and Renderer simultaneously without conflict.
// `WasmInputState` and `PendingClick` are now declared at module
// scope (above `fn run`) so cross-platform helpers can take them
// by `&mut`. Construction of the actual instance happens later in
// `spawn_local`.

let renderer_rc: Rc<RefCell<Option<Renderer>>> = Rc::new(RefCell::new(None));
let input_rc: Rc<RefCell<Option<WasmInputState>>> = Rc::new(RefCell::new(None));

// Clone Rcs for the spawn_local init future
let renderer_for_init = renderer_rc.clone();
let input_for_init = input_rc.clone();

// Renderer init is async on the browser (adapter + surface setup
// are Promise-backed). Spawn as a future so the event loop doesn't
// block waiting for wgpu.
wasm_bindgen_futures::spawn_local(async move {
    let instance = Instance::default();
    let surface = instance
        .create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))
        .expect("Failed to create surface");
    let mut renderer = Renderer::new(
        instance,
        surface,
        renderer_window,
    )
    .await;
    log::info!("WASM: adapter + surface + renderer ready");

    let size = canvas.width();
    let height = canvas.height();
    renderer.process_decree(RenderDecree::SetSurfaceSize(size, height));
    log::info!("WASM: surface configured {}x{}", size, height);

    // std::fs is unavailable in the browser; fetch over the page origin instead.
    let mut doc_opt: Option<MindMapDocument> = None;
    let mut tree_opt: Option<MindMapTree> = None;
    // Local AppScene used only for the initial border tree
    // build; it's then dropped, and `WasmInputState`'s own
    // `app_scene` takes over for the live event loop.
    let mut init_app_scene =
        crate::application::scene_host::AppScene::new();
    match fetch_map_json(&mindmap_path).await {
        Ok(json) => match MindMapDocument::from_json_str(&json, Some(mindmap_path.clone())) {
            Ok(mut doc) => {
                // Canvas background: resolve through theme variables
                // so `"var(--bg)"` works, then hand off to the
                // renderer as the render-pass clear color. Mirrors
                // run_native.rs so the WASM canvas paints against
                // the doc's configured background instead of the
                // default pitch black.
                let vars = &doc.mindmap.canvas.theme_variables;
                let resolved_bg = baumhard::util::color::resolve_var(
                    &doc.mindmap.canvas.background_color,
                    vars,
                );
                renderer.set_clear_color_from_hex(resolved_bg);

                // Four-source mutation registry, matching the native
                // path: app bundle (shipped in the binary) < user
                // source (?mutations= query param + localStorage) <
                // map (custom_mutations in the .mindmap.json) <
                // inline (on individual nodes). Plus the Rust-backed
                // handlers for layouts too structural for pure data.
                let (app_mutations, user_mutations) =
                    crate::application::document::mutations_loader::load_app_and_user();
                doc.build_mutation_registry_with_app_and_user(
                    &app_mutations,
                    &user_mutations,
                );
                crate::application::document::mutations::register_builtin_handlers(
                    &mut doc,
                );

                let mindmap_tree = doc.build_tree();
                renderer.rebuild_buffers_from_tree(&mindmap_tree.tree);
                renderer.fit_camera_to_tree(&mindmap_tree.tree);

                let scene = doc.build_scene(renderer.camera_zoom());
                update_connection_tree(&scene, &mut init_app_scene);
                update_border_tree_static(&doc, &mut init_app_scene);
                update_portal_tree(
                    &doc,
                    &std::collections::HashMap::new(),
                    &mut init_app_scene,
                    &mut renderer,
                );
                update_connection_label_tree(&scene, &mut init_app_scene, &mut renderer);
                flush_canvas_scene_buffers(&mut init_app_scene, &mut renderer);
                tree_opt = Some(mindmap_tree);
                doc_opt = Some(doc);
            }
            Err(e) => log::error!(
                "WASM: failed to construct document from '{}': {}",
                mindmap_path, e
            ),
        },
        Err(e) => log::error!("WASM: failed to fetch '{}': {}", mindmap_path, e),
    }

    renderer.process_decree(RenderDecree::StartRender);
    log::info!("WASM: StartRender dispatched, rAF loop starting");

    // Populate the shared state now that init is complete.
    *renderer_for_init.borrow_mut() = Some(renderer);

    if let Some(doc) = doc_opt {
        // Build the macro registry — App + User tiers from the
        // bundled JSON / `?macros=` / localStorage; Map + Inline
        // tiers from the just-loaded document. Mirrors
        // `run_native_init.rs:117-142` precedence and logging
        // shape so cross-target log triage stays uniform.
        let mut macros = crate::application::macros::MacroRegistry::new();
        let mut app_count = 0usize;
        for m in crate::application::macros::loader::load_app_macros() {
            macros.insert(m, crate::application::macros::MacroSource::App);
            app_count += 1;
        }
        let mut user_count = 0usize;
        for m in crate::application::macros::loader::load_user_macros() {
            macros.insert(m, crate::application::macros::MacroSource::User);
            user_count += 1;
        }
        if app_count > 0 || user_count > 0 {
            log::info!(
                "loaded {} macro(s): {} app-tier, {} user-tier",
                macros.len(),
                app_count,
                user_count,
            );
        }
        // Document-derived tiers — Map first, then Inline. The
        // shared `rebuild_document_macros` helper enforces the
        // ordering so it can't drift between the native and WASM
        // load sites.
        crate::application::macros::loader::rebuild_document_macros(&mut macros, &doc);

        *input_for_init.borrow_mut() = Some(WasmInputState {
            document: doc,
            mindmap_tree: tree_opt,
            text_edit_state: TextEditState::Closed,
            last_click: None,
            cursor_pos: (0.0, 0.0),
            pending_click: PendingClick::None,
            modifiers: winit::keyboard::ModifiersState::empty(),
            app_scene: crate::application::scene_host::AppScene::new(),
            scene_cache: baumhard::mindmap::scene_cache::SceneConnectionCache::new(),
            macros,
        });
    }

    // WASM render loop via requestAnimationFrame
    use wasm_bindgen::closure::Closure;
    let renderer_for_raf = renderer_for_init.clone();
    let f: Rc<RefCell<Option<Closure<dyn FnMut()>>>> =
        Rc::new(RefCell::new(None));
    let g = f.clone();

    *g.borrow_mut() = Some(Closure::new(move || {
        if let Some(r) = renderer_for_raf.borrow_mut().as_mut() {
            r.process();
        }
        // Reschedule against the same closure we were called from.
        // `f` is set to `Some(closure)` immediately below this
        // `Closure::new(...)` expression and never cleared — the
        // setup completes before any rAF fires, so inside this
        // body the Option is always `Some`. The `None` arm is
        // therefore unreachable in practice; rather than panic
        // (§9), we log and let the loop halt — the browser is
        // about to tear down the tab if this ever fires, and a
        // dropped loop is the correct outcome in that case
        // because the closure (`f`'s inner value) is the very
        // thing that would have been rescheduled.
        let closure_ref = f.borrow();
        let Some(closure) = closure_ref.as_ref() else {
            log::error!("RAF closure unexpectedly cleared — tab teardown in progress");
            return;
        };
        request_animation_frame(closure);
    }));
    request_animation_frame(
        g.borrow()
            .as_ref()
            .expect("render closure installed immediately above"),
    );
});

// Resolve the keybind config once. `action_for(key, ctrl, shift, alt)`
// answers the dispatch question for every keydown.
let keybinds: ResolvedKeybinds = app.options.keybind_config.resolve();

// Clone Rcs for the event loop closure
let renderer_for_events = renderer_rc.clone();
let input_for_events = input_rc.clone();
let suppress_for_events = suppress_keys.clone();

app.event_loop.run(move |event, _window_target| {
    _ = (&app.window, &mut app.options);

    match event {
        Event::WindowEvent {
            event: WindowEvent::Resized(size), ..
        } => {
            if let Some(renderer) = renderer_for_events.borrow_mut().as_mut() {
                renderer.process_decree(
                    RenderDecree::SetSurfaceSize(size.width, size.height),
                );
            }
        }
        Event::WindowEvent {
            event: WindowEvent::CloseRequested, ..
        } => {
            // WASM doesn't really close
        }

        // --- Modifier tracking ---
        Event::WindowEvent {
            event: WindowEvent::ModifiersChanged(mods), ..
        } => {
            if let Some(input) = input_for_events.borrow_mut().as_mut() {
                input.modifiers = mods.state();
            }
        }

        // --- Keyboard input ---
        Event::WindowEvent {
            event: WindowEvent::KeyboardInput {
                event: KeyEvent {
                    state: ElementState::Pressed,
                    logical_key: ref logical_key,
                    ..
                },
                ..
            },
            ..
        } => {
            let key_name = crate::application::keybinds::key_to_name(logical_key);

            let mut input_borrow = input_for_events.borrow_mut();
            let mut renderer_borrow = renderer_for_events.borrow_mut();
            let (Some(input), Some(renderer)) =
                (input_borrow.as_mut(), renderer_borrow.as_mut())
            else { return; };

            // Editor keyboard-steal: if open, route all keys
            // to the editor so hotkeys don't collide with typed text.
            if input.text_edit_state.is_open() {
                handle_text_edit_key(
                    &key_name,
                    logical_key,
                    input.modifiers.control_key(),
                    input.modifiers.shift_key(),
                    input.modifiers.alt_key(),
                    &keybinds,
                    &mut input.text_edit_state,
                    &mut input.document,
                    &mut input.mindmap_tree,
                    &mut input.app_scene,
                    renderer,
                    &mut input.scene_cache,
                );
                suppress_for_events.set(input.text_edit_state.is_open());
                return;
            }

            // Hotkey dispatch via keybinds.
            let action = key_name.as_deref().and_then(|k| {
                keybinds.action_for_context(
                    crate::application::keybinds::InputContext::Document,
                    k,
                    input.modifiers.control_key(),
                    input.modifiers.shift_key(),
                    input.modifiers.alt_key(),
                )
            });
            // WASM dispatch ladder. Action body lives in the
            // unified `dispatch_action_core::dispatch_compatible`
            // (Track C) — both the keyboard path here and the
            // `WasmMacroDispatchTarget::dispatch_action` impl
            // reach the same body. Native's `dispatch::dispatch_action`
            // delegates to it as well; one source of truth for
            // every Compatible arm across both targets.
            //
            // After the action lookup completes, fall through to
            // macro lookup — the same Action → Macro →
            // CustomMutation chain native uses (`event_keyboard.rs`).
            if let Some(a) = action.clone() {
                // Pin "did the user just trigger an EditSelection?"
                // before the dispatch — `suppress_for_events` is
                // ONLY updated for that pair (pre-Track-B, the
                // suppress call lived inside the EditSelection
                // pre-filter arm). Other Compatible Actions don't
                // touch suppress.
                let was_edit_selection =
                    matches!(a, Action::EditSelection | Action::EditSelectionClean);
                let _ = {
                    let mut core = input.input_context_core(renderer, &keybinds);
                    super::dispatch_action_core::dispatch_compatible(&a, &mut core)
                };
                if was_edit_selection {
                    // Mirror pre-Track-B `set(is_open())`: flip
                    // suppress to whatever the modal state ended
                    // up at — true if the editor opened, false if
                    // it didn't (e.g. selection wasn't Single).
                    // Always-set, NOT gated on dispatch outcome.
                    // Track-C-Commit-3 incorrectly gated on
                    // `Handled` which left suppress stuck-true on a
                    // non-Single EditSelection (Unhandled outcome);
                    // restored here per the design reviewer's flag.
                    suppress_for_events.set(input.text_edit_state.is_open());
                }
            } else {
                // No built-in Action bound to this combo — fall
                // through to macro lookup. Mirrors native's
                // `event_keyboard.rs` chain: Action → Macro →
                // (CustomMutation tier on native; macros only on
                // WASM today). Privilege gate runs inside
                // `dispatch_macro_core::dispatch_macro` so a
                // hostile Map / Inline tier macro can't slip
                // destructive Actions or ConsoleLine past.
                if let Some(macro_id) = key_name.as_deref().and_then(|k| {
                    keybinds.macro_for(
                        k,
                        input.modifiers.control_key(),
                        input.modifiers.shift_key(),
                        input.modifiers.alt_key(),
                    )
                }) {
                    let macro_id = macro_id.to_string();
                    let mut target = WasmMacroDispatchTarget {
                        input,
                        renderer,
                        keybinds: &keybinds,
                    };
                    let _ = super::dispatch_macro_core::dispatch_macro(&macro_id, &mut target);
                }
            }
        }

        // --- Mouse input ---
        Event::WindowEvent {
            event: WindowEvent::CursorMoved { position, .. }, ..
        } => {
            if let Some(input) = input_for_events.borrow_mut().as_mut() {
                input.cursor_pos = (position.x, position.y);
            }
        }

        Event::WindowEvent {
            event: WindowEvent::MouseInput {
                state: btn_state,
                button: MouseButton::Left,
                ..
            },
            ..
        } => {
            if btn_state == ElementState::Pressed {
                // --- Left mouse Pressed ---
                let mut input_borrow = input_for_events.borrow_mut();
                let Some(input) = input_borrow.as_mut() else { return; };

                // Compute canvas position via renderer
                let canvas_pos = {
                    let renderer_borrow = renderer_for_events.borrow();
                    match renderer_borrow.as_ref() {
                        Some(r) => r.screen_to_canvas(
                            input.cursor_pos.0 as f32,
                            input.cursor_pos.1 as f32,
                        ),
                        None => return,
                    }
                };

                // Hit test against nodes + portal sub-parts + edge
                // labels. Cross-platform helper — the priority chain
                // is byte-identical to native (`compute_click_hit`
                // in `app/mod.rs`), so the previously-duplicated
                // hit-routing block now lives in one place.
                let now = now_ms();
                let parts = {
                    let renderer_borrow = renderer_for_events.borrow();
                    let Some(renderer) = renderer_borrow.as_ref() else { return; };
                    super::compute_click_hit(
                        canvas_pos,
                        input.mindmap_tree.as_mut(),
                        renderer,
                    )
                };
                let super::ClickHitParts {
                    click_hit,
                    hit_node,
                    portal_text_hit,
                    portal_icon_hit,
                    edge_label_hit,
                } = parts;
                let already_editing_same_target = input.text_edit_state
                    .node_id()
                    .map(|id| hit_node.as_deref() == Some(id))
                    .unwrap_or(false);
                let is_dblclick = !already_editing_same_target
                    && input.last_click
                        .as_ref()
                        .map(|prev| is_double_click(prev, now, input.cursor_pos, &click_hit))
                        .unwrap_or(false);

                if is_dblclick {
                    input.last_click = None;

                    let mut renderer_borrow = renderer_for_events.borrow_mut();
                    let Some(renderer) = renderer_borrow.as_mut() else { return; };

                    match &click_hit {
                        ClickHit::Node(node_id) => {
                            let nid = node_id.clone();
                            input.document.selection = SelectionState::Single(nid.clone());
                            rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer, &mut input.scene_cache);
                            open_text_edit(
                                &nid, false,
                                &mut input.document,
                                &mut input.text_edit_state,
                                &mut input.mindmap_tree,
                                &mut input.app_scene,
                                renderer,
                            );
                        }
                        ClickHit::PortalMarker { edge, endpoint }
                        | ClickHit::PortalText { edge, endpoint } => {
                            // Double-click on icon or text both
                            // jump to the partner endpoint — they
                            // share the same endpoint identity
                            // and the same "navigate" intent.
                            let other_id = if *endpoint == edge.from_id {
                                edge.to_id.clone()
                            } else {
                                edge.from_id.clone()
                            };
                            if let Some(node) = input.document.mindmap.nodes.get(&other_id) {
                                let target = glam::Vec2::new(
                                    node.position.x as f32
                                        + node.size.width as f32 * 0.5,
                                    node.position.y as f32
                                        + node.size.height as f32 * 0.5,
                                );
                                renderer.set_camera_center(target);
                            }
                            input.document.selection = SelectionState::Edge(
                                crate::application::document::EdgeRef::new(
                                    &edge.from_id,
                                    &edge.to_id,
                                    &edge.edge_type,
                                ),
                            );
                            rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer, &mut input.scene_cache);
                        }
                        ClickHit::EdgeLabel(edge_key) => {
                            // Edge-label double-click is a parity
                            // placeholder on WASM. Native opens the
                            // inline label editor; WASM's modal
                            // editor path isn't available here yet,
                            // so the user falls back to the
                            // `/label edit` console verb. The
                            // previous single-click (release 1 in
                            // the dbl-click pair) already committed
                            // `SelectionState::EdgeLabel` and
                            // rebuilt the scene — this branch has
                            // nothing to add. Skipping the
                            // redundant commit + rebuild is both
                            // correct and meaningfully cheaper on
                            // mobile browsers (§4 mobile budget).
                            // If the selection somehow drifted
                            // between the two clicks, the `match`
                            // below handles re-committing; the
                            // guard just avoids the wasted
                            // rebuild in the common case.
                            let expected_er = crate::application::document::EdgeRef::new(
                                edge_key.from_id.as_str(),
                                edge_key.to_id.as_str(),
                                edge_key.edge_type.as_str(),
                            );
                            let already_selected = matches!(
                                &input.document.selection,
                                SelectionState::EdgeLabel(s) if s.edge_ref == expected_er
                            );
                            if !already_selected {
                                input.document.selection = SelectionState::EdgeLabel(
                                    crate::application::document::EdgeLabelSel::new(
                                        expected_er,
                                    ),
                                );
                                rebuild_scene_only(
                                    &input.document,
                                    &mut input.app_scene,
                                    renderer,
                                    &mut input.scene_cache,
                                );
                            }
                        }
                        ClickHit::Empty => {
                            // Match native: empty-canvas double-click is
                            // a no-op unless the user has explicitly
                            // bound `CreateOrphanNodeAndEdit`. Default
                            // ships unbound — addresses the user's
                            // "annoying" complaint on the WASM target
                            // too.
                            let allow_create = !matches!(
                                input.document.selection,
                                SelectionState::Edge(_)
                            ) && keybinds.has_any_binding_for(
                                crate::application::keybinds::Action::CreateOrphanNodeAndEdit,
                            );
                            if allow_create {
                                let new_id = input.document.create_orphan_and_select(canvas_pos);
                                rebuild_all(&input.document, &mut input.mindmap_tree, &mut input.app_scene, renderer, &mut input.scene_cache);
                                open_text_edit(
                                    &new_id, true,
                                    &mut input.document,
                                    &mut input.text_edit_state,
                                    &mut input.mindmap_tree,
                                    &mut input.app_scene,
                                    renderer,
                                );
                            }
                        }
                    }
                    suppress_for_events.set(input.text_edit_state.is_open());
                    return;
                }

                input.pending_click = if let Some(id) = hit_node.clone() {
                    PendingClick::Node(id)
                } else if let Some((key, endpoint)) = portal_text_hit.clone() {
                    // Portal **text** click — committed to
                    // `SelectionState::PortalText` on mouse-up.
                    PendingClick::PortalText {
                        edge_key: key,
                        endpoint_node_id: endpoint,
                    }
                } else if let Some((key, endpoint)) = portal_icon_hit.clone() {
                    // Portal **icon** click — committed to
                    // `SelectionState::PortalLabel` on mouse-up.
                    // Double-click already fired above so a
                    // pending marker click can only mean "select
                    // this label".
                    PendingClick::PortalMarker {
                        edge_key: key,
                        endpoint_node_id: endpoint,
                    }
                } else if let Some(key) = edge_label_hit.clone() {
                    // Edge label click — committed to
                    // `SelectionState::EdgeLabel` on mouse-up.
                    PendingClick::EdgeLabel(key)
                } else {
                    PendingClick::Empty
                };
                input.last_click = Some(LastClick {
                    time: now,
                    screen_pos: input.cursor_pos,
                    hit: click_hit,
                });
            } else {
                // --- Left mouse Released ---
                let mut input_borrow = input_for_events.borrow_mut();
                let Some(input) = input_borrow.as_mut() else { return; };

                let pending = std::mem::replace(&mut input.pending_click, PendingClick::None);
                if matches!(pending, PendingClick::None) { return; }

                if input.text_edit_state.is_open() {
                    let mut renderer_borrow = renderer_for_events.borrow_mut();
                    let Some(renderer) = renderer_borrow.as_mut() else { return; };
                    let release_canvas = renderer.screen_to_canvas(
                        input.cursor_pos.0 as f32,
                        input.cursor_pos.1 as f32,
                    );

                    let inside_edit_node = input.text_edit_state
                        .node_id()
                        .zip(input.mindmap_tree.as_ref())
                        .map(|(id, tree)| {
                            crate::application::document::point_in_node_aabb(
                                release_canvas, id, tree,
                            )
                        })
                        .unwrap_or(false);

                    if inside_edit_node {
                        return;
                    }

                    close_text_edit(
                        true,
                        &mut input.document,
                        &mut input.text_edit_state,
                        &mut input.mindmap_tree,
                        &mut input.app_scene,
                        renderer,
                        &mut input.scene_cache,
                    );
                    suppress_for_events.set(false);
                    return;
                }

                // Plain selection click. Snapshot the previous
                // selection so `rebuild_after_selection_change`
                // can pick between `rebuild_all` (needed when
                // either side is a node selection — tree
                // highlights must be applied or cleared) and the
                // cheaper `rebuild_scene_only` (edge-adjacent →
                // edge-adjacent transitions).
                let prev_selection = input.document.selection.clone();
                input.document.selection = match pending {
                    PendingClick::Node(node_id) => SelectionState::Single(node_id),
                    PendingClick::PortalMarker {
                        edge_key,
                        endpoint_node_id,
                    } => SelectionState::PortalLabel(
                        crate::application::document::PortalLabelSel {
                            edge_key,
                            endpoint_node_id,
                        },
                    ),
                    PendingClick::PortalText {
                        edge_key,
                        endpoint_node_id,
                    } => SelectionState::PortalText(
                        crate::application::document::PortalLabelSel {
                            edge_key,
                            endpoint_node_id,
                        },
                    ),
                    PendingClick::EdgeLabel(edge_key) => {
                        let er = crate::application::document::EdgeRef::new(
                            edge_key.from_id.as_str(),
                            edge_key.to_id.as_str(),
                            edge_key.edge_type.as_str(),
                        );
                        SelectionState::EdgeLabel(
                            crate::application::document::EdgeLabelSel::new(er),
                        )
                    }
                    _ => SelectionState::None,
                };
                let mut renderer_borrow = renderer_for_events.borrow_mut();
                if let Some(renderer) = renderer_borrow.as_mut() {
                    rebuild_after_selection_change(
                        &prev_selection,
                        &input.document,
                        &mut input.mindmap_tree,
                        &mut input.app_scene,
                        renderer,
                        &mut input.scene_cache,
                    );
                }
            }
        }

        Event::WindowEvent {
            event: WindowEvent::MouseWheel { delta, .. }, ..
        } => {
            let scroll_y = match delta {
                MouseScrollDelta::LineDelta(_, y) => y as f64,
                MouseScrollDelta::PixelDelta(pos) => pos.y / 50.0,
            };
            let factor = if scroll_y > 0.0 { 1.1 } else { 1.0 / 1.1 };
            let mut input_borrow = input_for_events.borrow_mut();
            let mut renderer_borrow = renderer_for_events.borrow_mut();
            if let (Some(input), Some(renderer)) =
                (input_borrow.as_mut(), renderer_borrow.as_mut())
            {
                // A zoom mid-click invalidates the pending selection:
                // the canvas coord the user pressed over has shifted
                // to a new screen position, so committing the pending
                // click on the eventual mouse-up would select whatever
                // now sits under the release cursor — not what the
                // user pressed on. Clear it so release falls through
                // to empty-click handling.
                input.pending_click = PendingClick::None;
                renderer.process_decree(RenderDecree::CameraZoom {
                    screen_x: input.cursor_pos.0 as f32,
                    screen_y: input.cursor_pos.1 as f32,
                    factor: factor as f32,
                });
                // Zoom touches scene geometry (connection glyph
                // sample spacing, viewport cull rect) but not the
                // node text tree — scene-only rebuild is enough.
                rebuild_scene_only(&input.document, &mut input.app_scene, renderer, &mut input.scene_cache);
            }
        }

        _ => {}
    }
}).expect("Event loop error");
}

/// Schedule `f` on the next browser animation frame — the
/// `requestAnimationFrame` handshake winit-web uses to drive its
/// render ticks. Kept next to the event-loop body because that's
/// its sole caller.
///
/// Called once per frame from inside the RAF closure — an
/// interactive-path caller per `CODE_CONVENTIONS.md §9`. Missing
/// `window` or a rejected rAF request degrades to a logged warning
/// and a dropped frame rather than a panic. In practice the browser
/// keeps both available for the lifetime of the page, so failure
/// here would indicate the tab is being torn down.
fn request_animation_frame(f: &wasm_bindgen::closure::Closure<dyn FnMut()>) {
    use wasm_bindgen::JsCast;
    let Some(window) = web_sys::window() else {
        log::error!("requestAnimationFrame: no `window` available — dropping frame");
        return;
    };
    if let Err(err) = window.request_animation_frame(f.as_ref().unchecked_ref()) {
        log::error!("requestAnimationFrame rejected: {:?} — dropping frame", err);
    }
}

/// HTTP-fetch a mindmap JSON file. Maps are bundled into the page
/// origin by trunk's `copy-dir` directive in `web/index.html`.
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
