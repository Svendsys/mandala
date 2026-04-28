// SPDX-License-Identifier: MPL-2.0

//! Baumhard — glyph-oriented rendering primitives for Mandala.
//!
//! Owns the GPU-adjacent data model: the `Tree<GfxElement,
//! GfxMutator>` that underpins every glyph layout, the mindmap
//! model and scene builder, shader entry points, and the
//! declarative mutator-builder DSL.
//!
//! Prescriptive rules (mutation-not-rebuild, arena discipline,
//! benchmark-reuse, no-unsafe) live in
//! `lib/baumhard/CONVENTIONS.md` — read them before touching this
//! crate.

pub mod util;
pub mod font;
pub mod gfx_structs;
pub mod core;
pub mod shaders;
pub mod mindmap;
pub mod mutator_builder;

