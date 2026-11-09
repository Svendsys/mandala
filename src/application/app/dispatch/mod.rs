// SPDX-License-Identifier: MPL-2.0

//! Dispatch funnel — every Action and macro firing routes through
//! here. Sub-modules:
//!
//! - [`families`]: the Actions whose bodies edited the old model, one
//!   file per owner issue that rebuilds them on the one writer; each
//!   answers `Unhandled` until its owner lands. Asked first, on both
//!   targets.
//! - [`cross_dispatch`]: cross-platform `apply_*` arm bodies,
//!   `ViewContext` and `DispatchOutcome`. Reachable from both
//!   dispatchers.
//! - [`action_core`]: the cross-platform `dispatch_compatible`
//!   action dispatcher. Routes every Compatible-classified variant
//!   into a `cross_dispatch::apply_*` helper.
//! - [`macro_core`]: the cross-platform `dispatch_macro` macro
//!   dispatcher with the privilege-gate enforcement loop. Abstracted
//!   over the `MacroDispatchTarget` trait so native and WASM share the
//!   body byte-for-byte.
//! - `native` (plain code-span — `cfg`-gated to native, so a link
//!   would break the wasm32 doc build): the native-side
//!   `dispatch_action` funnel that wraps `dispatch_compatible` and adds
//!   the NativeOnly arm match (console, modes, files). WASM reaches
//!   `dispatch_compatible` directly from `run_wasm`.

pub(in crate::application::app) mod action_core;
pub(in crate::application::app) mod cross_dispatch;
pub(in crate::application::app) mod families;
pub(in crate::application::app) mod macro_core;

#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) mod native;

// Re-exports so `super::dispatch::dispatch_action` etc. stay
// callable without callers learning the sub-module split.
#[cfg(not(target_arch = "wasm32"))]
pub(in crate::application::app) use native::{
    dispatch_action, dispatch_held_action, dispatch_macro, run_macro_steps,
};

// Cross-platform re-exports — both dispatchers import via the
// shorter `super::dispatch::*` form.
pub(in crate::application::app) use cross_dispatch::{
    apply_touch_effect, drive_touch_event, touch_phase, DispatchHit, DispatchOutcome, TouchStep,
};
