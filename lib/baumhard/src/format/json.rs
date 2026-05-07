// SPDX-License-Identifier: MPL-2.0

//! Single-source JSON parsing for non-mindmap loaders.
//!
//! Three loaders in the application crate (`keybinds`, `macros`,
//! `widgets/color_picker_widget`) all reach for `serde_json::from_str`
//! directly. Each formats its own error message in a slightly
//! different shape. This module collapses them onto one helper so
//! a future format swap (or a richer error wrapper) is one edit.
//!
//! The mindmap format itself is handled by
//! [`crate::mindmap::loader`] — that module owns the entire on-disk
//! schema and its legacy-detection rules, so it stays separate.
//!
//! Maptool (the CLI) keeps using `serde_json` directly because it
//! works below the typed shape (raw `Value` migrations) — the
//! audit policy carves convenience-only deps out of the wrapper
//! mandate, and that's the case here.

use serde::de::DeserializeOwned;

/// Re-export so callers don't need a `serde_json` import for the
/// dynamic-JSON case (legacy migration scaffolding, the
/// `Vec<serde_json::Value>` shape on `MindMap.macros`, etc).
pub use serde_json::Value;

/// Parse a JSON string into a typed value, returning a
/// caller-readable error string on failure. Wraps
/// [`serde_json::from_str`].
///
/// Use for any non-mindmap typed loader (keybinds, user macros,
/// embedded widget specs). The mindmap format has its own
/// loader at [`crate::mindmap::loader::load_from_str`] which
/// also handles legacy detection.
#[inline]
pub fn parse<T: DeserializeOwned>(source: &str) -> Result<T, String> {
    serde_json::from_str(source).map_err(|e| e.to_string())
}

/// Parse a `Value` into a typed value, returning a caller-readable
/// error string. Wraps [`serde_json::from_value`].
///
/// Used by the macro loader for the per-entry parse against the
/// `MindMap.macros: Vec<Value>` field. The `clone()` is on the
/// caller because the typical pattern iterates an `&[Value]`.
#[inline]
pub fn parse_value<T: DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|e| e.to_string())
}
