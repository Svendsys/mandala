// SPDX-License-Identifier: MPL-2.0

//! Cross-platform context-bundles for the unified `dispatch_action`
//! funnel. Track C from `work_plans/WASM_CONVERGENCE.md` (final WASM convergence
//! step after Tracks A + B):
//!
//! - [`InputContextCore`] holds the fields **both** native's
//!   `InputHandlerContext` (`input_context.rs`) and WASM's
//!   `WasmInputState` (`run_wasm/`) carry — document, view, renderer,
//!   last_click, cursor_pos, modifiers, macros, interaction_mode. This
//!   is the parameter `dispatch_action` takes post-Track-C; both
//!   targets' keyboard handlers can construct one and call the same
//!   dispatcher.
//! - `NativeContextExt` holds the native-only fields (drag_state,
//!   console_state, console_history, color_picker_state, cursor_is_hand,
//!   cursor_icon_last) — modal / console / picker state
//!   that doesn't exist in the browser. Cfg-gated to native, which is
//!   also why it is named here rather than linked: an intra-doc link
//!   to it does not resolve when the docs are built for
//!   `wasm32-unknown-unknown`.
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

use crate::application::platform::input::Modifiers as ModifiersState;

use crate::application::document::MindMapDocument;
use crate::application::macros::MacroRegistry;
use crate::application::renderer::Renderer;

use crate::application::view::MapView;

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
    /// The document's view: the projector, the glyph layers and the
    /// overlays. `Some` exactly when `document` is.
    pub view: &'a mut Option<MapView>,
    /// The active renderer.
    pub renderer: &'a mut Renderer,
    /// Previous click (time, position, hit) for double-click
    /// detection. Used by `ExitMode`'s cross-platform slice
    /// (clear `last_click` so a post-Esc click isn't paired
    /// with a pre-Esc one).
    pub last_click: &'a mut Option<LastClick>,
    /// Last-known cursor position in screen space.
    pub cursor_pos: &'a mut (f64, f64),
    /// Modifier snapshot maintained by `ModifiersChanged` events.
    /// `&` (immutable) — no dispatch arm mutates modifiers.
    ///
    /// Carried, not yet read: the arms that want modifiers still run
    /// against native's `InputHandlerContext`. What moves them onto
    /// this bundle is Track A.3 in
    /// `work_plans/WASM_CONVERGENCE.md` — lifting per-arm bodies
    /// into `dispatch::cross_dispatch` — and that is the named
    /// consumer for this field and [`Self::macros`] both. (Track C,
    /// context-type unification, is what *created* this bundle; it
    /// has already landed.)
    #[allow(dead_code)]
    pub modifiers: &'a ModifiersState,
    /// Macro registry: the App and User tiers, loaded at startup.
    /// Cross-platform per Track B.
    /// Carried, not yet read — see [`Self::modifiers`].
    #[allow(dead_code)]
    pub macros: &'a mut MacroRegistry,
    /// High-level interaction mode. Cross-platform; see
    /// `super::interaction_mode::InteractionMode`.
    pub interaction_mode: &'a mut super::InteractionMode,
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
/// **Field set is the fields not on `WasmInputState`.** One of
/// them is `drag_state`, which WASM substitutes for via
/// `pending_click` on `WasmInputState`, kept outside the unified
/// context — `pending_click` isn't a 1:1 mirror.
///
/// **Every field is carried, none is read yet.** `split_borrow`
/// produces the bundle and `dispatch_action` threads it through, but
/// the native-only arms still reach their state through
/// `InputHandlerContext` directly; moving them across is Track A.3
/// in `work_plans/WASM_CONVERGENCE.md` — lifting per-arm bodies into
/// `dispatch::cross_dispatch` — which is the named consumer.
/// The allow sits on the struct rather than on each field because
/// the answer is the same for all of them and will stop being true for
/// all of them at once.
#[allow(dead_code)]
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) struct NativeContextExt<'a> {
    /// Current pointer / drag state machine.
    pub drag_state: &'a mut super::DragState,
    /// Console (slash-command overlay) state.
    pub console_state: &'a mut crate::application::console::ConsoleState,
    /// Console command-history ring.
    pub console_history: &'a mut Vec<String>,
    /// Glyph-wheel color-picker state.
    pub color_picker_state: &'a mut crate::application::color_picker::ColorPickerState,
    /// Per-frame cursor-icon flag — flipped to "hand" over a
    /// button node by the cursor-move handler.
    pub cursor_is_hand: &'a mut bool,
    /// Last cursor icon written via `Window::set_cursor`. Cached
    /// so the cursor_moved handler skips the per-frame call when
    /// the desired icon hasn't changed (winit doesn't dedup
    /// `set_cursor` on Windows / Wayland).
    pub cursor_icon_last: &'a mut winit::window::CursorIcon,
}
