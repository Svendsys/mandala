// SPDX-License-Identifier: MPL-2.0

//! Cross-platform context-bundles for the unified `dispatch_action`
//! funnel. Track C from `WASM_CONVERGENCE.md` (final WASM convergence
//! step after Tracks A + B):
//!
//! - [`InputContextCore`] holds the 11 fields **both** native's
//!   `InputHandlerContext` (`input_context.rs`) and WASM's
//!   `WasmInputState` (`run_wasm/`) carry — document, mindmap_tree,
//!   app_scene, renderer, scene_cache, text_edit_state, last_click,
//!   cursor_pos, modifiers, keybinds, macros. This is the parameter
//!   `dispatch_action` takes post-Track-C; both targets' keyboard
//!   handlers can construct one and call the same dispatcher.
//! - [`NativeContextExt`] holds the 10 native-only fields (drag_state,
//!   app_mode, console_state, console_history, label_edit_state,
//!   portal_text_edit_state, color_picker_state, hovered_node,
//!   cursor_is_hand, picker_hover) — modal / console / picker state
//!   that doesn't exist in the browser. Cfg-gated to native.
//!
//! Native dispatchers pass `Some(&mut ext)`; WASM passes `None`.
//! Per-arm bodies pattern-match on the `Option` for the ~14 arms that
//! genuinely need native-only state. The other ~50 arms only see
//! `core` and are callable on both targets without per-arm
//! cfg-gating — that's the structural gap Track C closed.
//!
//! **Why a struct, not a trait** — the macro-target pattern from
//! Track B (`MacroDispatchTarget`) was the right shape for a
//! per-step loop dispatched a few times per user gesture. For
//! `dispatch_action`, which runs on every keystroke, virtual calls
//! and the borrow-checker headaches of multi-field accessor
//! methods (each `&mut Self` accessor closes over the whole self)
//! tip the design toward a concrete struct with split borrows.

use winit::keyboard::ModifiersState;

use crate::application::document::MindMapDocument;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::macros::MacroRegistry;
use crate::application::renderer::Renderer;
use crate::application::scene_host::AppScene;

use super::text_edit::TextEditState;
use super::LastClick;

/// Cross-platform action-dispatch context. Borrows every field by
/// `&mut` for write paths and `&` for read-only fields
/// (`modifiers`, `keybinds`). Built once per dispatch call from
/// either `InputHandlerContext::split_borrow` (native) or
/// `WasmInputState::input_context_core` (WASM, added in the C3
/// step that wires WASM at the unified dispatcher).
///
/// Lifetime `'a` ties every borrow back to a single source so the
/// struct is a re-packaging of borrows, not a new owner. The
/// `'a` reborrow pattern (`&'a mut *self.field`) on the native
/// side keeps `InputHandlerContext` and the constructed core
/// happy under the borrow checker.
pub(in crate::application::app) struct InputContextCore<'a> {
    /// The loaded mindmap document, or `None` before the first
    /// successful load. Shape `Option<&mut _>` (rather than `&mut
    /// Option<_>`) so both targets can construct without ownership
    /// shuffles: native passes `ctx.document.as_mut()`; WASM
    /// passes `Some(&mut input.document)` even though
    /// `WasmInputState` owns the document by value.
    pub document: Option<&'a mut MindMapDocument>,
    /// Baumhard tree projection of the document. Rebuilt /
    /// mutated in lockstep with `document`.
    pub mindmap_tree: &'a mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    /// App-layer scene host owning every tree-rendered component.
    pub app_scene: &'a mut AppScene,
    /// The active renderer.
    pub renderer: &'a mut Renderer,
    /// Per-edge connection glyph cache.
    pub scene_cache: &'a mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    /// Inline node text editor state — modal-steal target on both
    /// targets. WASM has its own implementation today; the field
    /// is the load-bearing piece that lets `EditSelection`'s
    /// Single branch be cross-platform.
    pub text_edit_state: &'a mut TextEditState,
    /// Previous click (time, position, hit) for double-click
    /// detection. Used by `CancelMode`'s WASM-relevant slice
    /// (clear `last_click` so a post-Esc click isn't paired
    /// with a pre-Esc one).
    pub last_click: &'a mut Option<LastClick>,
    /// Last-known cursor position in screen space.
    pub cursor_pos: &'a mut (f64, f64),
    /// Modifier snapshot maintained by `ModifiersChanged` events.
    /// `&` (immutable) — no dispatch arm mutates modifiers.
    pub modifiers: &'a ModifiersState,
    /// Resolved user keybinds. `&` (immutable) — dispatch arms
    /// only call `&self` query methods (`has_any_binding_for` etc.).
    pub keybinds: &'a ResolvedKeybinds,
    /// Macro registry. App + User tiers loaded at startup; Map +
    /// Inline tiers refreshed by `loader::rebuild_document_macros`
    /// whenever a document loads. Cross-platform per Track B.
    pub macros: &'a mut MacroRegistry,
}

/// Native-only extension carrying the modal / console / picker /
/// drag fields that don't exist in the browser. Passed alongside
/// [`InputContextCore`] to `dispatch_action`; native callers pass
/// `Some(&mut ext)` and WASM passes `None`. Per-arm bodies that
/// need any field here `if let Some(ext) = ext { ... }` and bail
/// to `Unhandled` otherwise — a posture symmetric with the way
/// `Action::wasm_compatibility` already classifies these arms
/// `NativeOnly`.
///
/// **Field set is the 10 fields not on `WasmInputState`** plus
/// `drag_state` (which WASM substitutes for via `pending_click` on
/// `WasmInputState`, kept outside the unified context — `pending_click`
/// isn't a 1:1 mirror).
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) struct NativeContextExt<'a> {
    /// Current pointer / drag state machine.
    pub drag_state: &'a mut super::DragState,
    /// Reparent / Connect modal mode for the next click.
    pub app_mode: &'a mut super::AppMode,
    /// Console (slash-command overlay) state.
    pub console_state: &'a mut crate::application::console::ConsoleState,
    /// Console command-history ring.
    pub console_history: &'a mut Vec<String>,
    /// Inline edge-label editor state.
    pub label_edit_state: &'a mut super::label_edit::LabelEditState,
    /// Inline portal-text editor state.
    pub portal_text_edit_state: &'a mut super::label_edit::PortalTextEditState,
    /// Glyph-wheel color-picker state.
    pub color_picker_state: &'a mut crate::application::color_picker::ColorPickerState,
    /// The node the cursor is currently over, if any.
    pub hovered_node: &'a mut Option<String>,
    /// Per-frame cursor-icon flag — flipped to "hand" over a
    /// button node by the cursor-move handler.
    pub cursor_is_hand: &'a mut bool,
    /// Throttled color-picker hover interaction.
    pub picker_hover: &'a mut super::throttled_interaction::ColorPickerHoverInteraction,
}
