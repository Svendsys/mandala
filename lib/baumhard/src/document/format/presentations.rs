// SPDX-License-Identifier: MPL-2.0

//! A presentation as a file writes it (`format/presentations.md`): the
//! definition, its lifecycle and its body, spelled with the format's own
//! value types and with the edit format's target, filter, mutations and
//! timing (`document::format::edits`), which it shares.
//!
//! **Names, never indices**, as for an edit (amendment 4 on #251): a
//! document-layer body's theme variables, palettes, fonts and verbs are
//! written by name and resolved against the open map when the
//! presentation starts.
//!
//! **Strict.** Every object denies unknown keys and every enum is
//! externally tagged in `snake_case`, inside the gate V1-B2 holds the
//! format to (`document::tests::format_strict_tests`). A glyph-layer
//! body spells the three run mutations a presentation may write
//! (`format/mutations.md`, "Glyph run mutations") in the map format's
//! own spelling: a color is `#rrggbb` or `#rrggbbaa`, a range
//! `[start, end]`.

use crate::document::format::edits::{AnimateDto, FilterDto, MutationDto, TargetDto};
use crate::document::format::spelling::HexDto;
use serde::{Deserialize, Deserializer, Serialize};

/// A value that may be `null` and must be written: the key is required,
/// where a bare `Option` would read a missing key as `null` and give the
/// file two spellings of one thing.
fn nullable<'de, D: Deserializer<'de>, T: Deserialize<'de>>(deserializer: D) -> Result<Option<T>, D::Error> {
    Option::<T>::deserialize(deserializer)
}

/// One presentation definition (design §10.2), as a file writes it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresentationDto {
    /// The id the console and verbs name it by.
    pub id: String,
    /// The name shown to the user.
    pub name: String,
    /// What it does, for `present list`.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Its lifecycle.
    pub play: PlayDto,
    /// The vocabulary its body is written in; the document's when
    /// absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layer: Option<LayerDto>,
    /// Which elements within each node it is started on.
    pub target: TargetDto,
    /// Which of those elements it keeps.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<FilterDto>,
    /// The document layer's body: the mutations applied once to each
    /// element.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mutations: Option<Vec<MutationDto>>,
    /// The glyph layer's body: the run mutations applied once to each
    /// element's run.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub glyph: Option<Vec<GlyphOpDto>>,
    /// How it plays in; instantly when absent.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub animate: Option<AnimateDto>,
    /// How it plays back out; as `animate` when absent.
    #[serde(default, rename = "return", skip_serializing_if = "Option::is_none")]
    pub ret: Option<AnimateDto>,
}

/// A file of presentations outside a map: the bundled
/// `assets/presentations.json` and the user's `presentations.json`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct PresentationsFileDto {
    /// The presentations, in the file's order.
    pub presentations: Vec<PresentationDto>,
}

/// `play`: the lifecycle.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum PlayDto {
    /// `"while"`: in, held until stopped, out.
    While,
    /// `"once"`: in, then straight back out.
    Once,
    /// `"loop"`: in, out, in again, until stopped.
    Loop,
    /// `"hold"`: in, held until stopped or the map closes, out.
    Hold,
}

/// `layer`: the vocabulary the body is written in.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(crate) enum LayerDto {
    /// `"document"`: document mutations, drawn through projection.
    Document,
    /// `"glyph"`: run mutations on the elements' projected runs.
    Glyph,
}

/// One run mutation of a glyph-layer body.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub(crate) enum GlyphOpDto {
    /// `run_opacity`: set the run's opacity, `0` to `1`.
    #[serde(rename = "run_opacity")]
    Opacity(f32),
    /// `run_origin`: set the run's origin, `[x, y]`.
    #[serde(rename = "run_origin")]
    Origin([f32; 2]),
    /// `run_overrides`: write the run's per-glyph overrides.
    #[serde(rename = "run_overrides")]
    Overrides(RunOverrideDto),
}

/// One edit of a run's per-glyph overrides. `range` is `[start, end]`,
/// half-open, over the run's placed glyphs.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum RunOverrideDto {
    /// Offset each glyph of the range by `value`, `[dx, dy]`.
    Offset {
        /// The placed glyphs written.
        range: [usize; 2],
        /// The offset, in canvas units.
        value: [f32; 2],
    },
    /// Color each glyph of the range; `null` returns them to their own
    /// color.
    Color {
        /// The placed glyphs written.
        range: [usize; 2],
        /// The color, or `null`: written either way, never left out.
        #[serde(deserialize_with = "nullable")]
        value: Option<HexDto>,
    },
    /// Scale each glyph of the range to `value` times its shaped size.
    Scale {
        /// The placed glyphs written.
        range: [usize; 2],
        /// The scale; `1` is as shaped.
        value: f32,
    },
    /// Drop every override of the run.
    Clear,
}
