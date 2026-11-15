// SPDX-License-Identifier: MPL-2.0

//! What a presentation says about the rest of the map, anywhere in it
//! (its filter or its body): the verbs it names, and whether it can show
//! what a file saves undrawn. `maptool verify` reads it beside
//! [`EditDef::mentions`](crate::document::edit::EditDef::mentions), by
//! the same walk: a presentation's filter and document-layer mutations
//! are spelled as an edit's are.
//!
//! The definition is destructured without `..` and its body matched
//! without a wildcard, so a part added to the format fails to compile
//! here until it is counted or listed as saying nothing.

use crate::document::edit::mentions::Walk;
use crate::document::edit::{Filter, Mutations, Shows};
use crate::document::presentation::{GlyphOps, PresentationBody, PresentationDef};

impl PresentationDef {
    /// Calls `verb` with each verb id this presentation names — a node's
    /// `set_click` or `set_hover`, a span's `set_link` to
    /// `{"verb": …}`, a filter's `on_click` or `on_hover` — and returns
    /// what it can show while it plays: a node it writes
    /// `set_hidden: false` on, a portal end it gives a marker. A
    /// glyph-layer body names nothing and shows nothing: it writes runs
    /// that are already drawn.
    ///
    /// What the presentation writes, not where it plays: that is decided
    /// when it starts, by the nodes it is started on.
    ///
    /// Cost: one walk of the definition, one call of `verb` per verb id
    /// it writes, and no allocation.
    pub fn mentions<'a>(&'a self, mut verb: impl FnMut(&'a str)) -> Shows {
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
        let mut walk = Walk::new(&mut verb);
        if let Some(Filter(filter)) = filter {
            walk.filter(filter);
        }
        match body {
            PresentationBody::Mutations(Mutations(mutations)) => walk.mutations(mutations),
            PresentationBody::Glyph(GlyphOps(_)) => {}
        }
        walk.shows
    }
}
