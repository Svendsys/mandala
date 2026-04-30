// SPDX-License-Identifier: MPL-2.0

//! Dispatch funnel — every Action / macro / custom-mutation
//! firing routes through here. Sub-modules:
//!
//! - [`cross_dispatch`]: cross-platform `apply_*` arm bodies +
//!   `RebuildContext` + `DispatchOutcome`. Reachable from both
//!   dispatchers; the helpers take `&mut RebuildContext` and
//!   delegate to the matching mutation core in
//!   `console/commands/`.
//! - [`action_core`]: the cross-platform `dispatch_compatible`
//!   action dispatcher. Routes every Compatible-classified
//!   variant into a `cross_dispatch::apply_*` helper.
//! - [`macro_core`]: the cross-platform `dispatch_macro` macro
//!   dispatcher with the privilege-gate enforcement loop.
//!   Abstracted over the `MacroDispatchTarget` trait so native
//!   and WASM share the body byte-for-byte.
//! - [`native`]: the native-side `dispatch_action` funnel that
//!   wraps `dispatch_compatible` and adds the NativeOnly arm
//!   match (console verbs / app-mode toggles / inline modal
//!   editors / filesystem). Native-only by `cfg`; WASM reaches
//!   `dispatch_compatible` directly from `run_wasm`.
//!
//! Both targets call `cross_dispatch::*` and
//! `action_core::dispatch_compatible` directly. The `native::*`
//! re-exports below let `super::dispatch::dispatch_action`
//! callers stay terse without knowing the internal split.

pub(in crate::application::app) mod action_core;
pub(in crate::application::app) mod cross_dispatch;
pub(in crate::application::app) mod macro_core;

#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) mod native;

// Re-exports so `super::dispatch::dispatch_action` etc. stay
// callable without callers learning the sub-module split.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) use native::{
    apply_label_edit_action, apply_label_edit_action_to_buffer, dispatch_action,
    dispatch_custom_mutation_for_key, dispatch_macro, DispatchHit,
};

// Cross-platform re-exports — both dispatchers and the
// `tests_mutations` parity tests import via the shorter
// `super::dispatch::*` form.
pub(in crate::application::app) use cross_dispatch::DispatchOutcome;
// `pub(crate)` so `tests_mutations` (in `document/`) and the
// WASM run loop (`run_wasm/mod.rs`) can both reach it through this
// canonical re-export. Native sub-modules (`native.rs`) also call
// here rather than the sibling path so the surface stays
// uniform across targets.
pub(crate) use cross_dispatch::apply_keybind_custom_mutation;
