// SPDX-License-Identifier: MPL-2.0

//! `Outcome` — the result type for a single capability-trait call.
//! Aggregated by the dispatcher into a per-kv report line.

/// Outcome of a single trait call.
#[derive(Clone, Debug, PartialEq)]
pub enum Outcome {
    /// Setter changed something.
    Applied,
    /// Setter ran but the value already matched. Distinct from
    /// `Applied` so callers can surface "already set" feedback.
    Unchanged,
    /// Target doesn't implement this trait. Reported per-pair so
    /// `color bg=#fff text=accent` can apply only the supported pairs.
    NotApplicable,
    /// Value rejected by the target (e.g. negative font size).
    Invalid(String),
}

impl Outcome {
    pub fn applied(changed: bool) -> Self {
        if changed {
            Outcome::Applied
        } else {
            Outcome::Unchanged
        }
    }
}

/// Result of a copy/cut on a component. Parallels [`Outcome`] for
/// data-producing operations (`Text`/`Empty`/`NotApplicable`).
#[derive(Clone, Debug, PartialEq)]
pub enum ClipboardContent {
    Text(String),
    /// Structured per-section payload — the full set of fields on
    /// `MindSection` (`text_runs`, `offset`, `size`, `channel`,
    /// `trigger_bindings`) bundled with the plain `text` that the
    /// OS clipboard can carry. The `text` field is what
    /// cross-app paste sees through `arboard`; the `payload` is
    /// preserved in the in-process structured buffer for
    /// within-app section→section paste so the round trip
    /// preserves per-run formatting and section chrome instead
    /// of falling back to template inheritance via
    /// `set_section_text`.
    Section {
        text: String,
        payload: SectionPayload,
    },
    /// Copy supported but nothing to provide (e.g. empty field).
    Empty,
    NotApplicable,
}

/// Per-section snapshot used by the structured clipboard path
/// (`ClipboardContent::Section`) and by the in-process buffer in
/// `application::clipboard`. Mirrors the user-facing fields on
/// `MindSection` so a round-trip preserves text-run formatting,
/// offset, size, channel routing, and per-section trigger
/// bindings — the audit's Q5 lossy-paste case.
///
/// `Position` and `Size` are re-exported from baumhard so the
/// trait layer doesn't have to depend directly on the model
/// crate's full surface; cloning a section's fields into this
/// struct is cheap (every contained type is `Clone`).
#[derive(Clone, Debug, PartialEq)]
pub struct SectionPayload {
    pub text_runs: Vec<baumhard::mindmap::model::TextRun>,
    pub offset: baumhard::mindmap::model::Position,
    pub size: Option<baumhard::mindmap::model::Size>,
    pub channel: Option<usize>,
    pub trigger_bindings: Vec<baumhard::mindmap::custom_mutation::TriggerBinding>,
}

impl SectionPayload {
    /// Snapshot a `MindSection` into a payload. Every contained
    /// field is `Clone` (`text_runs`, `trigger_bindings` are
    /// owned `Vec`s; `offset`/`size`/`channel` are tiny POD).
    pub fn from_section(section: &baumhard::mindmap::model::MindSection) -> Self {
        Self {
            text_runs: section.text_runs.clone(),
            offset: section.offset.clone(),
            size: section.size.clone(),
            channel: section.channel,
            trigger_bindings: section.trigger_bindings.clone(),
        }
    }
}
