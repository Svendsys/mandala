// SPDX-License-Identifier: MPL-2.0

//! `InteractionMode` — what the user is doing at the canvas level.
//!
//! The modes that edit the map arrive with the issues that rebuild
//! their editing on the one writer: reparent and connect with V1-C4
//! (#245), resize with V1-C7 (#248), node editing with V1-C8 (#249).
//! Until then the canvas has one mode. The type is kept rather than
//! dropped because every surface that reports a mode — the IPC
//! `query.mode`, the mode-status line, `Action::ExitMode` — reads it,
//! and those issues add variants to it rather than inventing it again.
//!
//! Cross-platform: it carries no GPU handles, so both targets compile
//! it.

/// What the user is doing right now at the canvas level. One is always
/// active; transitions go through `Action` dispatch.
#[derive(Debug, Clone, PartialEq, Default)]
#[non_exhaustive]
pub enum InteractionMode {
    /// The canvas's default mode: a click selects, a drag pans.
    #[default]
    Default,
}

impl InteractionMode {
    /// The mode's name, as `query.mode` reports it. O(1).
    pub fn word(&self) -> &'static str {
        match self {
            InteractionMode::Default => "default",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::InteractionMode;

    /// The mode a fresh app starts in is the default one, and it says so
    /// to `query.mode` in the word the protocol publishes.
    #[test]
    fn test_interaction_mode_starts_default_and_names_itself() {
        assert_eq!(InteractionMode::default(), InteractionMode::Default);
        assert_eq!(InteractionMode::Default.word(), "default");
    }
}
