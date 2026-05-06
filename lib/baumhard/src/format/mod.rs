// SPDX-License-Identifier: MPL-2.0

//! On-disk format primitives shared by every Mandala loader. The
//! mindmap-format loader / saver is its own module
//! ([`crate::mindmap::loader`]); the helpers here cover everything
//! else (user keybinds, user macros, embedded widget specs).

/// Thin wrapper around `serde_json` that pins one canonical
/// `Result<T, String>` parse helper for every non-mindmap JSON
/// loader in the workspace, and re-exports `Value` so callers
/// don't need to import `serde_json` directly.
pub mod json;
