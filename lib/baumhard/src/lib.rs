// SPDX-License-Identifier: MPL-2.0

//! Baumhard — glyph-oriented rendering primitives for Mandala.
//!
//! Owns the mutation engine — trees, mutators, the walker, predicates
//! and the declarative builder DSL, generic over the element type and
//! the mutation payload — and its glyph instantiation, the `GfxTree`
//! that underpins every glyph layout, plus the document and its one
//! writer, the `.mandala.json` format, the projector that draws a
//! document as glyph layers, and the shader entry points.
//!
//! Prescriptive rules (mutation-not-rebuild, arena discipline,
//! benchmark-reuse, no-unsafe) live in
//! `lib/baumhard/CONVENTIONS.md` — read them before touching this
//! crate.

// CONVENTIONS.md's no-unsafe rule, held by the compiler rather than by
// review. It also closes the raw-pointer route around the document's
// privacy (ruling R39 on #238): a `*mut` handed out of `document/`
// cannot be dereferenced in this crate, and `util::document_surface`
// refuses one in any signature or type that reaches past `document/`,
// crates downstream included. The counting allocator's `unsafe` lives
// in `cap`, outside the crate.
#![forbid(unsafe_code)]
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

/// The arena tree every tree here is stored in: [`Arena`](arena::Arena)
/// of linked nodes named by generational [`NodeId`](arena::NodeId)s,
/// with O(1) links, iterative walks and O(1) same-row re-attach.
pub mod arena;
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
/// On-disk format primitives — the JSON loader facade for the configs
/// that are not maps (keybinds, user macros, embedded widget specs). The
/// map format itself lives under [`document::format`].
pub mod format;
/// GPU-facing structs, the engine's glyph instantiation:
/// `GfxElement`, `GfxMutation` / `GfxMutator`, `GlyphArea`,
/// `GfxTree` / `GfxMutatorTree`, and the glyph spatial queries.
pub mod gfx_structs;
/// Projection: the document into glyph layers, forward only, with work
/// proportional to what changed (design §12).
pub mod projection;
/// Shared math, container, and formatting helpers across the crate.
pub mod util;

/// The vector math baumhard's public API speaks — `Vec2` positions and
/// sizes throughout `flow`, `document` and `gfx_structs` — re-exported
/// so a dependent names the very glam baumhard was built with instead
/// of pinning a version of its own that can drift from it.
pub use glam;
