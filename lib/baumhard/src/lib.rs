// SPDX-License-Identifier: MPL-2.0

//! Baumhard — glyph-oriented rendering primitives for Mandala.
//!
//! Owns the mutation engine — trees, mutators, the walker, predicates
//! and the declarative builder DSL, generic over the element type and
//! the mutation payload — and its glyph instantiation, the `GfxTree`
//! that underpins every glyph layout, plus the mindmap model and scene
//! builder and the shader entry points.
//!
//! Prescriptive rules (mutation-not-rebuild, arena discipline,
//! benchmark-reuse, no-unsafe) live in
//! `lib/baumhard/CONVENTIONS.md` — read them before touching this
//! crate.

// CODE_CONVENTIONS §9 closes with "Bare `unwrap()` outside tests is
// a bug", and this is the half of that rule an editor can tell you
// about while you type. `util::unwrap_posture` is the other half —
// it reads the workspace's source text and fails every `./test.sh`
// run; the lint fails only `./test.sh --lint`, a hard gate since
// clippy's baseline reached zero, but one you have to ask for. Two
// mechanisms rather than one because they disagree usefully: the lint sees
// post-expansion code the text scan cannot read, and the scan sees
// the `pub mod tests;` trees the lint has to be told about.
//
// The `cfg_attr` is what keeps the lint off test code. A
// `#[cfg(test)] mod` does not exist in the build where the lint is
// live, and in the build where it does exist the whole crate is
// allowed — so `unwrap()` stays the right spelling in a test and a
// bug everywhere else.
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

/// Low-level primitives: color regions, outlines, apply-operations,
/// and pure-data value types.
pub mod core;
/// The document: a map as a `Tree<DocElement>` — typed values,
/// identity, shape rules and per-field style resolution.
pub mod document;
/// The mutation engine, generic over the element type and the
/// mutation payload: `Tree` / `MutatorTree`, `Mutator` and
/// `Instruction`, the walker, `Predicate`, and the builder DSL.
pub mod engine;
/// TextFlow: shaping spans into runs and boxes, outlines and curves,
/// and placing upright glyphs along paths — the one text primitive
/// everything drawn as text is built on.
pub mod flow;
/// Font loading, shaping, and glyph-metric lookups backed by
/// cosmic-text. Owns the long-lived font cache.
pub mod font;
/// On-disk format primitives — the JSON loader facade for
/// non-mindmap configs (keybinds, user macros, embedded widget
/// specs). The mindmap format itself lives under
/// [`mindmap::loader`].
pub mod format;
/// GPU-facing structs, the engine's glyph instantiation:
/// `GfxElement`, `GfxMutation` / `GfxMutator`, `GlyphArea`,
/// `GfxTree` / `GfxMutatorTree`, and the glyph spatial queries.
pub mod gfx_structs;
/// `.mindmap.json` data model, loaders, scene/tree builders, and
/// the `CustomMutation` carrier.
pub mod mindmap;
/// Shared math, container, and formatting helpers across the crate.
pub mod util;

/// The vector math baumhard's public API speaks — `Vec2` positions and
/// sizes throughout `flow`, `document` and `gfx_structs` — re-exported
/// so a dependent names the very glam baumhard was built with instead
/// of pinning a version of its own that can drift from it.
pub use glam;
