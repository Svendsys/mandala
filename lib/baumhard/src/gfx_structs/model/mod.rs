// SPDX-License-Identifier: MPL-2.0

//! Glyph-model data types — the render-time shape of a tree node's
//! content. `GlyphModel` wraps a `GlyphMatrix` of `GlyphLine`s of
//! `GlyphComponent`s; the matrix's `place_in` paints them into a
//! shared text + regions pair. Mutations land via `DeltaGlyphModel`
//! / `GlyphModelCommand` through
//! [`Applicable`](crate::core::primitives::Applicable).

pub mod component;
pub mod glyph_model;
pub mod line;
pub mod matrix;
pub mod mutator;

pub use component::{GlyphComponent, GlyphComponentField};
pub use glyph_model::GlyphModel;
pub use line::GlyphLine;
pub use matrix::GlyphMatrix;
pub use mutator::{
    DeltaGlyphModel, GlyphModelCommand, GlyphModelCommandType, GlyphModelField,
    GlyphModelFieldType,
};
