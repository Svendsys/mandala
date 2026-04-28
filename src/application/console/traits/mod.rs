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
    AcceptsFontFamily, AcceptsWheelColor, HandlesCopy, HandlesCut, HandlesPaste, HasBgColor,
    HasBorderColor, HasLabel, HasTextColor,
};
pub use color_value::ColorValue;
pub use dispatch::{apply_kvs, apply_to_targets, DispatchReport};
pub use outcome::{ClipboardContent, Outcome};
// Re-exported as the surface type of `view_for` / `selection_targets`;
// the lint can't see implicit reach-through usage.
#[allow(unused_imports)]
pub use view::{selection_targets, view_for, TargetId, TargetView};
