// SPDX-License-Identifier: MPL-2.0

//! Shared per-call context for the native event-loop dispatchers.
//!
//! Each handler reaches into the same mutable bundle on `InitState`.
//! `InputHandlerContext<'a>` borrows each persistent field as
//! `&'a mut T` so handlers can destructure it; per-event payloads stay
//! as separate function parameters. Built once per event in
//! [`super::run_native::InitState::input_context`].

#![cfg(not(target_arch = "wasm32"))]

use winit::keyboard::ModifiersState;

use crate::application::color_picker::ColorPickerState;
use crate::application::console::ConsoleState;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::macros::MacroRegistry;
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;

use super::throttled_interaction::ColorPickerHoverInteraction;
use super::{
    AppMode, DragState, LabelEditState, LastClick, PortalTextEditState, TextEditState,
};

/// Borrowed view of the persistent state every interactive-path
/// dispatcher reads and writes. Built once per event by
/// [`crate::application::app::run_native::InitState::input_context`]
/// and passed to `handle_mouse_input`, `handle_cursor_moved`,
/// `handle_keyboard_input`, and `submit_line`.
///
/// The lifetime `'a` ties every field borrow to a single `&mut
/// InitState` — the struct is a re-packaging of existing borrows,
/// not a new owner of state.
pub(in crate::application::app) struct InputHandlerContext<'a> {
    /// The loaded mindmap document, or `None` before the first
    /// successful `loader::load_from_file`.
    pub document: &'a mut Option<MindMapDocument>,
    /// Baumhard tree projection of the document. Rebuilt / mutated
    /// in lockstep with `document`.
    pub mindmap_tree: &'a mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    /// App-layer scene host owning every tree-rendered component.
    pub app_scene: &'a mut AppScene,
    /// The active renderer.
    pub renderer: &'a mut Renderer,
    /// Per-edge connection glyph cache.
    pub scene_cache: &'a mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    /// Current pointer / drag state machine.
    pub drag_state: &'a mut DragState,
    /// Reparent / Connect modal mode for the next click.
    pub app_mode: &'a mut AppMode,
    /// Console (slash-command overlay) state.
    pub console_state: &'a mut ConsoleState,
    /// Console command-history ring.
    pub console_history: &'a mut Vec<String>,
    /// Inline edge-label editor state.
    pub label_edit_state: &'a mut LabelEditState,
    /// Inline portal-text editor state.
    pub portal_text_edit_state: &'a mut PortalTextEditState,
    /// Inline node text editor state.
    pub text_edit_state: &'a mut TextEditState,
    /// Glyph-wheel color-picker state.
    pub color_picker_state: &'a mut ColorPickerState,
    /// Previous click (time, position, hit) for double-click detection.
    pub last_click: &'a mut Option<LastClick>,
    /// The node the cursor is currently over, if any.
    pub hovered_node: &'a mut Option<String>,
    /// Last-known cursor position in screen space.
    pub cursor_pos: &'a mut (f64, f64),
    /// Modifier snapshot maintained by `ModifiersChanged` events.
    pub modifiers: &'a ModifiersState,
    /// Per-frame cursor-icon flag — flipped to "hand" over a button
    /// node by the cursor-move handler.
    pub cursor_is_hand: &'a mut bool,
    /// Throttled color-picker hover interaction. The picker's
    /// input paths set `.dirty` when HSV state changes; the
    /// per-frame drain rebuilds the scene + overlay through the
    /// unified adaptive-throttle shell.
    pub picker_hover: &'a mut ColorPickerHoverInteraction,
    /// Resolved user keybinds.
    pub keybinds: &'a mut ResolvedKeybinds,
    /// Macro registry (App + User tiers loaded at startup; Map tier
    /// re-loaded whenever a document is replaced via `open` / `new`).
    /// Mutable so the document-replace path in
    /// `execute_console_line` can rebuild the Map tier without
    /// taking a separate ad-hoc borrow.
    pub macros: &'a mut MacroRegistry,
}
