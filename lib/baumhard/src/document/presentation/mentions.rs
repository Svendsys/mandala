// SPDX-License-Identifier: MPL-2.0

//! What a presentation says about the rest of the map, anywhere in it
//! (its filter or its body): the names it writes, and whether it can show
//! what a file saves undrawn. `maptool verify` reads it beside
//! [`EditDef::mentions`](crate::document::edit::EditDef::mentions), by
//! the same walk: a presentation's filter and document-layer mutations
//! are spelled as an edit's are.
//!
//! The definition is destructured without `..` and its body matched
//! without a wildcard, so a part added to the format fails to compile
//! here until it is counted or listed as saying nothing.

use crate::document::edit::mentions::Walk;
use crate::document::edit::{Filter, Mention, Mutations, Shows};
use crate::document::format::presentations::{GlyphOpDto, RunOverrideDto};
use crate::document::presentation::{GlyphOps, PresentationBody, PresentationDef};

impl PresentationDef {
    /// Calls `mention` with each name this presentation writes that its
    /// map must define — a verb id (a node's `set_click` or `set_hover`,
    /// a span's `set_link` to `{"verb": …}`, a filter's `on_click` or
    /// `on_hover`), a theme variable or a palette, as
    /// [`EditDef::mentions`](crate::document::edit::EditDef::mentions)
    /// reports them — and returns
    /// what it can show while it plays: a node it writes
    /// `set_hidden: false` on. A glyph-layer body names nothing and
    /// shows nothing: it writes runs that are already drawn.
    ///
    /// A presentation's mutations are written on its target's elements
    /// and on nothing below them — it has no mutator — and a load holds
    /// each to the target's kind. So one that reveals plays on nodes,
    /// and none gives a portal end a marker: no target is a portal end,
    /// and [`Shows::marks_portal_ends`] is never set for one that read.
    /// Which nodes it plays on is decided when it starts, by the nodes
    /// it is started on, and is not narrowed here.
    ///
    /// Cost: one walk of the definition, one call of `mention` per name it
    /// writes, and the allocation of that name's path.
    pub fn mentions<'a>(&'a self, mut mention: impl FnMut(Mention<'a>)) -> Shows {
        let PresentationDef {
            id: _,
            name: _,
            description: _,
            play: _,
            layer: _,
            target: _,
            filter,
            body,
            animate: _,
            ret: _,
        } = self;
        let mut walk = Walk::new(&mut mention);
        if let Some(Filter(filter)) = filter {
            walk.filter(filter);
        }
        match body {
            PresentationBody::Mutations(Mutations(mutations)) => walk.mutations(mutations),
            PresentationBody::Glyph(GlyphOps(ops)) => glyph(ops),
        }
        walk.shows
    }
}

/// A glyph-layer body names nothing and shows nothing: it writes runs
/// that are already drawn, and its colors are literal. Every op is
/// listed, so that one that takes a name fails to compile here until it
/// is counted.
fn glyph(ops: &[GlyphOpDto]) {
    for op in ops {
        match op {
            GlyphOpDto::Opacity(_) | GlyphOpDto::Origin(_) => {}
            GlyphOpDto::Overrides(overrides) => match overrides {
                RunOverrideDto::Offset { range: _, value: _ }
                | RunOverrideDto::Color { range: _, value: _ }
                | RunOverrideDto::Scale { range: _, value: _ }
                | RunOverrideDto::Clear => {}
            },
        }
    }
}
