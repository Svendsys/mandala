// SPDX-License-Identifier: MPL-2.0

//! **Presentations**: the "perform the map" kind of behavior (design
//! §10.2, D6, D17; V1-D2, #251). A presentation is never saved, never in
//! the history and never marks the map changed: it animates on top of
//! the document as it stands, whatever is edited underneath.
//!
//! A [`PresentationDef`](crate::document::presentation::PresentationDef)
//! names the elements it plays on — a
//! [`Target`](crate::document::edit::Target) within each node it is
//! started on, and an optional filter — what it writes there, and how it
//! lives ([`Play`](crate::document::presentation::Play)):
//!
//! - on the **document layer**, field mutations in the document's
//!   vocabulary, never structure (hiding a node and fading it are
//!   fields, D17), which go through projection as an edit's do;
//! - on the **glyph layer**, run mutations on the elements' projected
//!   runs — per-glyph offsets, colors and scale — which mean nothing in
//!   the data.
//!
//! Started on N nodes, a presentation is N **instances**, one per node
//! (amendment 2 on #251), each with its own progress `t` in `0..=1`
//! ([`lifecycle`](crate::document::presentation::lifecycle)).
//! The instances form a layer on top of the document
//! ([`layer`](crate::document::presentation::layer)):
//!
//! ```text
//! committed document ⊕ open transaction ⊕ presentation layer   →  projection
//! ```
//!
//! A definition is read from a file with its names, and those names are
//! resolved against the open map when it starts, as an edit's are when
//! it runs.

// Each module's own docs say what it is. (A doc comment here would
// have rustdoc read the module's own links in this scope.)
pub mod layer;
pub mod lifecycle;
pub(crate) mod load;
mod mentions;

pub use layer::{InstanceId, InstanceInfo};
pub use lifecycle::Direction;
pub use load::{parse_presentations, PresentationsError};

use crate::document::edit::resolve::Fault;
use crate::document::edit::{Animate, Easing, Filter, Mutations, Target};
use crate::document::format::context::shown;
use crate::document::format::presentations::{GlyphOpDto, LayerDto, PlayDto};
use crate::document::writer::error::ApplyError;
use crate::engine::interpolate::Snap;
use std::fmt;

/// How an instance lives (design §10.2, #251 §2).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Play {
    /// In, held until it is stopped, out.
    While,
    /// In, then straight back out.
    Once,
    /// In, out, in again; stopped, it finishes the cycle it is in.
    Loop,
    /// In, held until it is stopped or the map closes, out.
    Hold,
}

/// The vocabulary a presentation's body is written in.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum PresentationLayer {
    /// Document mutations, drawn through projection.
    Document,
    /// Run mutations on the elements' projected runs.
    Glyph,
}

/// A glyph-layer body, as a file writes it: run mutations in the
/// document's own words, since the document names no glyph type. What
/// they are on a run is
/// [`projection::glyph_mutations`](crate::projection::glyph_mutations).
#[derive(Clone, Debug, PartialEq)]
pub struct GlyphOps(pub(crate) Vec<GlyphOpDto>);

/// What a presentation writes on the elements it plays on.
#[derive(Clone, Debug, PartialEq)]
pub enum PresentationBody {
    /// The document layer: each mutation, once on each element.
    Mutations(Mutations),
    /// The glyph layer: each run mutation, once on each element's run.
    Glyph(GlyphOps),
}

/// A presentation (design §10.2). See the [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct PresentationDef {
    /// The id the console and verbs name it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
    /// What it does.
    pub description: String,
    /// How its instances live.
    pub play: Play,
    /// The vocabulary of its body.
    pub layer: PresentationLayer,
    /// Which elements within each node it is started on.
    pub target: Target,
    /// Which of those it keeps.
    pub filter: Option<Filter>,
    /// What it writes on them.
    pub body: PresentationBody,
    /// How it plays in.
    pub animate: Animate,
    /// How it plays back out; as [`Self::animate`] when `None`.
    pub ret: Option<Animate>,
}

impl PresentationDef {
    /// How it plays back out: [`Self::ret`], or [`Self::animate`]. O(1).
    pub fn out(&self) -> Animate {
        self.ret.unwrap_or(self.animate)
    }
}

/// The timing of a direction a file leaves out: instant, linear, the
/// fields that do not blend switching at the end (design §4.6).
pub const INSTANT: Animate = Animate {
    ms: 0,
    delay_ms: 0,
    easing: Easing::Linear,
    snap: Snap::End,
};

/// Why a presentation could not start. Nothing was started.
#[derive(Clone, Debug, PartialEq)]
pub enum PresentError {
    /// It names something the open map does not define.
    UnknownName {
        /// The presentation's id.
        presentation: String,
        /// The field that names it.
        field: &'static str,
        /// What it names, and as what.
        name: String,
    },
    /// A value it writes is not one its field takes, or its body is not
    /// its layer's, or it loops in no time.
    Value {
        /// The presentation's id.
        presentation: String,
        /// The field.
        field: &'static str,
        /// Why.
        message: String,
    },
    /// The document refused what it writes: a structural command, a
    /// reference the map lacks, a size its place does not take, or a
    /// write past this platform's bounds.
    Refused {
        /// The presentation's id.
        presentation: String,
        /// The document's refusal. Boxed: it is the largest part of any
        /// refusal, and a start that succeeds carries none.
        error: Box<ApplyError>,
    },
}

/// Every part that may hold what a file wrote — the presentation's id,
/// the name it gave a field, a message that quotes either — is shown as
/// file text is: every control, line separator and bidi control escaped
/// (`format::context::shown`), as an edit's refusal shows it (#365).
impl fmt::Display for PresentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PresentError::UnknownName {
                presentation,
                field,
                name,
            } => write!(
                f,
                "presentation `{}`: `{field}` names {}, which this map does not define",
                shown(presentation),
                shown(name)
            ),
            PresentError::Value {
                presentation,
                field,
                message,
            } => write!(
                f,
                "presentation `{}`: `{field}`: {}",
                shown(presentation),
                shown(message)
            ),
            PresentError::Refused { presentation, error } => {
                write!(f, "presentation `{}`: {error}", shown(presentation))
            }
        }
    }
}

impl std::error::Error for PresentError {}

impl PresentError {
    /// The refusal of a start of `presentation` that `fault` is.
    pub(crate) fn of(fault: Fault, presentation: &str) -> Self {
        let presentation = presentation.to_string();
        match fault.described() {
            (field, Ok(name)) => PresentError::UnknownName {
                presentation,
                field,
                name,
            },
            (field, Err(message)) => PresentError::Value {
                presentation,
                field,
                message,
            },
        }
    }
}

/// A loop needs time to be anywhere in: why one whose way in and way
/// out both take none is refused, at load and at start.
pub(crate) const LOOP_TAKES_NO_TIME: &str =
    "a `loop` whose `animate` and `return` both take no time is at no place at any instant: give one a duration";

/// Whether `play` over `animate` and `out` has a state at every
/// instant: every lifecycle but a loop whose cycle takes no time. O(1).
pub(crate) fn takes_time(play: Play, animate: &Animate, out: &Animate) -> bool {
    play != Play::Loop || lifecycle::span_ms(animate) + lifecycle::span_ms(out) > 0.0
}

/// How a presentations file spells `dto`: the name its `serde` form
/// writes. O(1).
fn spelled<D: serde::Serialize>(dto: D) -> String {
    match serde_json::to_value(dto) {
        Ok(serde_json::Value::String(name)) => name,
        other => format!("{other:?}"),
    }
}

impl Play {
    /// As a presentations file spells it: `while`, `once`, `loop`,
    /// `hold`. O(1).
    pub fn spelling(self) -> String {
        spelled(PlayDto::from(self))
    }
}

impl PresentationLayer {
    /// As a presentations file spells it: `document` or `glyph`. O(1).
    pub fn spelling(self) -> String {
        spelled(LayerDto::from(self))
    }
}

impl From<PlayDto> for Play {
    fn from(dto: PlayDto) -> Self {
        match dto {
            PlayDto::While => Play::While,
            PlayDto::Once => Play::Once,
            PlayDto::Loop => Play::Loop,
            PlayDto::Hold => Play::Hold,
        }
    }
}

impl From<Play> for PlayDto {
    fn from(play: Play) -> Self {
        match play {
            Play::While => PlayDto::While,
            Play::Once => PlayDto::Once,
            Play::Loop => PlayDto::Loop,
            Play::Hold => PlayDto::Hold,
        }
    }
}

impl From<LayerDto> for PresentationLayer {
    fn from(dto: LayerDto) -> Self {
        match dto {
            LayerDto::Document => PresentationLayer::Document,
            LayerDto::Glyph => PresentationLayer::Glyph,
        }
    }
}

impl From<PresentationLayer> for LayerDto {
    fn from(layer: PresentationLayer) -> Self {
        match layer {
            PresentationLayer::Document => LayerDto::Document,
            PresentationLayer::Glyph => LayerDto::Glyph,
        }
    }
}
