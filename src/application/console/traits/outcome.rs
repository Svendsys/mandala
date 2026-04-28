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
        if changed { Outcome::Applied } else { Outcome::Unchanged }
    }
}

/// Result of a copy/cut on a component. Parallels [`Outcome`] for
/// data-producing operations (`Text`/`Empty`/`NotApplicable`).
#[derive(Clone, Debug, PartialEq)]
pub enum ClipboardContent {
    Text(String),
    /// Copy supported but nothing to provide (e.g. empty field).
    Empty,
    NotApplicable,
}
