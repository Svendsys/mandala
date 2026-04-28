// SPDX-License-Identifier: MPL-2.0

//! First-run initialisation for the native event loop. Called once
//! from [`super::run_native::NativeApp::resumed`].

#![cfg(not(target_arch = "wasm32"))]

use super::run_native::InitState;
use super::*;

use baumhard::mindmap::tree_builder::MindMapTree;

/// Build the fully-initialised [`InitState`] around a freshly-created
/// `Window`. Mindmap load is best-effort (on failure the document
/// stays `None` and the canvas renders empty).
pub(super) fn build(options: &Options, window: Arc<Window>) -> InitState {
    baumhard::font::fonts::init();

    // Hand wgpu the owned `Arc<Window>` rather than pre-snapshotting
    // raw handles via `SurfaceTargetUnsafe::from_window`: under
    // wgpu 29 + winit 0.30 the latter blew up with
    // `Hal(MissingDisplayHandle)` on EGL/GL Linux because the GL
    // surface ctor re-queries the display handle and won't accept a
    // captured raw struct. WASM uses the same safe API path.
    let instance = Instance::default();
    let surface = instance
        .create_surface(window.clone())
        .expect("Failed to create wgpu surface for window");

    let mut renderer = block_on(Renderer::new(instance, surface, Arc::clone(&window)));

    // Configure initial surface size.
    let size = window.inner_size();
    renderer.process_decree(RenderDecree::SetSurfaceSize(size.width, size.height));

    // Load mindmap — document and tree persist for interactive use.
    let mut document: Option<MindMapDocument> = None;
    let mut mindmap_tree: Option<MindMapTree> = None;
    // Keyed incremental rebuild: document-side cache of per-edge
    // pre-clip sample geometry. Populated lazily by
    // `build_scene_with_cache`; cleared by `rebuild_all` so any
    // structural change forces a fresh scene build.
    let scene_cache = baumhard::mindmap::scene_cache::SceneConnectionCache::new();
    // App-level scene host: owns the canvas-space tree for borders
    // today (registered via `update_border_tree_*`) and hosts the
    // console / color-picker overlays.
    let mut app_scene = crate::application::scene_host::AppScene::new();

    match MindMapDocument::load(&options.mindmap_path) {
        Ok(mut doc) => {
            // Four-source mutation registry: app bundle (shipped in the
            // binary) < user file ($XDG_CONFIG_HOME/mandala/mutations.json)
            // < map (in the .mindmap.json) < inline (on individual nodes).
            let (app_mutations, user_mutations) =
                crate::application::document::mutations_loader::load_app_and_user(None);
            doc.build_mutation_registry_with_app_and_user(&app_mutations, &user_mutations);
            // Rust-backed handlers for mutations too structural for
            // a pure-data `flat_mutations` reach (flower-layout,
            // tree-cascade, …).
            crate::application::document::mutations::register_builtin_handlers(&mut doc);
            // Canvas background: resolve through theme variables so
            // `"var(--bg)"` works, then hand off to the renderer as
            // the render-pass clear color.
            let vars = &doc.mindmap.canvas.theme_variables;
            let resolved_bg = baumhard::util::color::resolve_var(
                &doc.mindmap.canvas.background_color,
                vars,
            );
            renderer.set_clear_color_from_hex(resolved_bg);

            // Nodes: build Baumhard tree from MindMap hierarchy.
            let tree = doc.build_tree();
            renderer.rebuild_buffers_from_tree(&tree.tree);
            renderer.fit_camera_to_tree(&tree.tree);

            // Connections + borders: flat pipeline from RenderScene.
            // `fit_camera_to_tree` above settled the zoom, so pass
            // it through — the scene builder sizes connection
            // glyphs against the actual final zoom rather than the
            // default-init value.
            let scene = doc.build_scene(renderer.camera_zoom());
            update_connection_tree(&scene, &mut app_scene);
            update_border_tree_static(&doc, &mut app_scene);
            update_portal_tree(
                &doc,
                &std::collections::HashMap::new(),
                &mut app_scene,
                &mut renderer,
            );
            update_connection_label_tree(&scene, &mut app_scene, &mut renderer);
            flush_canvas_scene_buffers(&mut app_scene, &mut renderer);

            mindmap_tree = Some(tree);
            document = Some(doc);
        }
        Err(e) => {
            log::error!("{}", e);
        }
    }

    // Start rendering.
    renderer.process_decree(RenderDecree::StartRender);

    let keybinds: ResolvedKeybinds = options.keybind_config.resolve();
    // Cross-session history loaded from disk on startup; appended
    // to on every Enter; written back on close.
    let console_history: Vec<String> = load_console_history();

    // User-layer macros, loaded once at startup. Failures log a
    // warning and yield an empty registry — same posture as the
    // mutation loader.
    let mut macros = crate::application::macros::MacroRegistry::new();
    for m in crate::application::macros::loader::load_user_macros() {
        macros.insert(m);
    }
    if !macros.is_empty() {
        log::info!("loaded {} user macro(s)", macros.len());
    }

    InitState {
        window,
        renderer,
        document,
        mindmap_tree,
        scene_cache,
        app_scene,
        cursor_pos: (0.0, 0.0),
        drag_state: DragState::None,
        app_mode: AppMode::Normal,
        console_state: ConsoleState::Closed,
        console_history,
        label_edit_state: LabelEditState::Closed,
        portal_text_edit_state: PortalTextEditState::Closed,
        text_edit_state: TextEditState::Closed,
        color_picker_state: crate::application::color_picker::ColorPickerState::Closed,
        last_click: None,
        hovered_node: None,
        modifiers: ModifiersState::empty(),
        // True while the cursor is hovering a node with any trigger
        // bindings (a "button"). Tracked so we only call set_cursor
        // on transitions instead of every CursorMoved event.
        cursor_is_hand: false,
        // Picker hover gate: cursor-moves into the picker update
        // HSV + preview synchronously (cheap), but scene + overlay
        // rebuild runs through the unified adaptive throttle in
        // `AboutToWait`. Each active drag gets its own
        // `MutationFrequencyThrottle` inside its interaction
        // struct on entry (see `event_cursor_moved`).
        picker_hover: super::throttled_interaction::ColorPickerHoverInteraction::new(),
        keybinds,
        macros,
    }
}
