// SPDX-License-Identifier: MPL-2.0

//! Capability traits and the [`TargetView`] dispatcher — the core of
//! the console's trait-dispatched cross-cutting command layer.
//!
//! A command like `color bg=accent text=#fff` materialises a
//! `Vec<TargetView>` from the selection and, for each kv pair, calls
//! the matching capability trait on every target. Variants that don't
//! implement the trait return [`Outcome::NotApplicable`]; the
//! dispatcher aggregates outcomes into a single per-kv report.

mod capabilities;
mod color_value;
mod dispatch;
mod outcome;
mod view;

#[cfg(test)]
mod tests;

pub use capabilities::{
    AcceptsFontFamily, HandlesCopy, HandlesCut, HandlesPaste, HasBgColor, HasBorderColor, HasLabel,
    HasTextColor,
};
// Native-only: `AcceptsWheelColor` is consulted by the inline
// color-picker modal in `app/color_picker_flow/`, which is
// native-gated. WASM has no inline picker yet.
#[cfg(not(target_arch = "wasm32"))]
pub use capabilities::AcceptsWheelColor;
pub use color_value::ColorValue;
pub use dispatch::{apply_kvs, apply_to_targets, DispatchReport};
pub use outcome::{ClipboardContent, Outcome, SectionPayload};
// Re-exported as the surface type of `view_for` / `selection_targets`;
// the lint can't see implicit reach-through usage.
#[allow(unused_imports)]
pub use view::{selection_targets, view_for, TargetId, TargetView};
