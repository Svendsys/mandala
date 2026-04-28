// SPDX-License-Identifier: MPL-2.0

//! Macro types and registry.
//!
//! A macro is a named `Vec<MacroStep>` invoked atomically through
//! `crate::application::app::dispatch::dispatch_macro`. Three step
//! kinds — built-in `Action`, parameterised `CustomMutation`,
//! console-line — cover every existing user-driven entry point at the
//! application layer. Plugins, future macro-recording UI, and the
//! `keybind_bindings` resolution tier all reach the same dispatcher.
//!
//! Loading parallels the custom-mutation loader at
//! `src/application/document/mutations_loader/`: a registry is built
//! once at startup, queried by id, and bindable from `keybinds.json`
//! via the `macro_bindings: HashMap<String, String>` field on
//! `KeybindConfig`.

#[cfg(not(target_arch = "wasm32"))]
pub mod loader;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::application::keybinds::Action;

/// Tier a macro was loaded from. Mirrors `MutationSource` in
/// `document/mutations_loader/mod.rs`. Variants are in ascending
/// precedence: `App` < `User` < `Map` < `Inline`. Higher-tier
/// macros override lower-tier ones with the same id.
///
/// **Privilege model.** [`MacroStep::ConsoleLine`] runs an
/// arbitrary console verb — including filesystem-touching ones
/// (`save <path>`, `open <path>`). To prevent a hostile shared
/// mindmap from doing arbitrary file I/O, only [`MacroSource::User`]
/// macros are allowed to contain `ConsoleLine` steps. The
/// dispatcher rejects `ConsoleLine` from `App`, `Map`, and
/// `Inline` tiers with a `warn!`. Today only the `User` tier
/// loads macros (`loader::load_user_macros`), so the gate is
/// dormant — but it must hold before any other tier ships.
///
/// Document-mutating step kinds (`Action`, `CustomMutation`) have
/// no privilege constraint — they can only do what their backing
/// machinery already permits.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MacroSource {
    /// Shipped with the binary (placeholder; no app-bundle loader
    /// today).
    App,
    /// Loaded from the user's `macros.json`. Same trust posture as
    /// `keybinds.json` — the user owns the file.
    User,
    /// Declared in the currently-loaded map's macros array
    /// (placeholder; no inline-on-map loader today).
    Map,
    /// Declared on a specific node's inline-macros array
    /// (placeholder).
    Inline,
}

impl MacroSource {
    /// Whether macros from this source may carry
    /// `MacroStep::ConsoleLine` steps. Only `User` macros pass —
    /// app-bundled / map-inline / node-inline macros loaded from
    /// untrusted sources cannot execute arbitrary console verbs.
    pub fn allows_console_line(self) -> bool {
        matches!(self, MacroSource::User)
    }
}

/// One step inside a macro. A macro is a sequence of these executed
/// atomically. Each variant routes through one of the application's
/// existing dispatch surfaces, so adding a new step kind is a matter
/// of forwarding to the relevant dispatcher — no new behaviour.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind")]
pub enum MacroStep {
    /// Run a built-in `Action` against the current `InputHandlerContext`.
    /// The dispatcher routes through `dispatch_action` exactly as if
    /// the user had pressed the action's bound key.
    Action {
        action: Action,
    },
    /// Apply a custom mutation by id, against the resolved target.
    /// Forwards to the same path keybind-triggered custom mutations
    /// use (animation-aware, document-actions parity).
    CustomMutation {
        id: String,
        #[serde(default)]
        target: MacroTarget,
    },
    /// Re-parse the given line through the console parser and run it
    /// as if typed. Lets macros leverage every parameterised verb
    /// (e.g. `border preset=triple`) without needing a bespoke step
    /// kind.
    ConsoleLine {
        line: String,
    },
}

/// How a `MacroStep::CustomMutation` resolves its target node id at
/// dispatch time. `CurrentSelection` matches the selection-driven
/// click-trigger path; `NodeId` lets a macro target a specific node
/// regardless of selection state (useful for "open the inbox" style
/// shortcuts).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum MacroTarget {
    /// Use whichever node is currently selected (must be a single
    /// selection — multi-selection or edge selections cause the step
    /// to skip).
    #[default]
    CurrentSelection,
    /// Always target the named node. Skips if the node id doesn't
    /// resolve in the current document.
    NodeId(String),
}

/// A user-defined macro: id + ordered list of steps. Loaded from
/// `~/.config/mandala/macros.json` on native (Phase 8 scaffolding —
/// app-bundle and inline-on-document layers are left for a
/// future commit, mirroring how the mutation loader grew).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Macro {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub description: String,
    pub steps: Vec<MacroStep>,
}

/// In-memory lookup table. Built once at startup from the loader's
/// merged slices. Each entry carries its source tier so the
/// dispatcher can gate privileged step kinds (see [`MacroSource`]).
#[derive(Debug, Clone, Default)]
pub struct MacroRegistry {
    macros: HashMap<String, (Macro, MacroSource)>,
}

impl MacroRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert / replace a macro by id, tagged with its loader tier.
    /// Returns the previous entry if any was present — caller decides
    /// whether to log the override.
    pub fn insert(&mut self, m: Macro, source: MacroSource) -> Option<Macro> {
        self.macros
            .insert(m.id.clone(), (m, source))
            .map(|(prev, _)| prev)
    }

    /// Look up a macro by id.
    pub fn get(&self, id: &str) -> Option<&Macro> {
        self.macros.get(id).map(|(m, _)| m)
    }

    /// Look up a macro and its source tier — needed by the dispatcher
    /// to decide whether `ConsoleLine` steps are allowed.
    pub fn get_with_source(&self, id: &str) -> Option<(&Macro, MacroSource)> {
        self.macros.get(id).map(|(m, s)| (m, *s))
    }

    /// Whether the registry knows about this id.
    pub fn contains(&self, id: &str) -> bool {
        self.macros.contains_key(id)
    }

    /// Number of registered macros.
    pub fn len(&self) -> usize {
        self.macros.len()
    }

    pub fn is_empty(&self) -> bool {
        self.macros.is_empty()
    }

    /// Iterate registered macro ids — used by completion / inspection
    /// surfaces. HashMap iteration order is unspecified; sort at
    /// the call site if a stable display is needed.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.macros.keys().map(|s| s.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn macro_registry_insert_and_get() {
        let mut reg = MacroRegistry::new();
        let m = Macro {
            id: "test".into(),
            name: "Test".into(),
            description: String::new(),
            steps: vec![MacroStep::Action {
                action: Action::Undo,
            }],
        };
        assert!(reg.is_empty());
        reg.insert(m.clone(), MacroSource::User);
        assert_eq!(reg.len(), 1);
        assert!(reg.contains("test"));
        let got = reg.get("test").unwrap();
        assert_eq!(got.id, "test");
        assert_eq!(got.steps.len(), 1);
        let (_, src) = reg.get_with_source("test").unwrap();
        assert_eq!(src, MacroSource::User);
    }

    #[test]
    fn macro_registry_insert_replaces_by_id() {
        let mut reg = MacroRegistry::new();
        let m1 = Macro {
            id: "x".into(),
            name: "first".into(),
            description: String::new(),
            steps: vec![],
        };
        let m2 = Macro {
            id: "x".into(),
            name: "second".into(),
            description: String::new(),
            steps: vec![MacroStep::ConsoleLine {
                line: "border on".into(),
            }],
        };
        reg.insert(m1, MacroSource::User);
        let prev = reg.insert(m2, MacroSource::User);
        assert!(prev.is_some());
        assert_eq!(reg.get("x").unwrap().name, "second");
    }

    #[test]
    fn macro_source_console_line_gating() {
        assert!(MacroSource::User.allows_console_line());
        assert!(!MacroSource::App.allows_console_line());
        assert!(!MacroSource::Map.allows_console_line());
        assert!(!MacroSource::Inline.allows_console_line());
    }

    #[test]
    fn macro_step_serde_round_trip() {
        let steps = vec![
            MacroStep::Action {
                action: Action::Undo,
            },
            MacroStep::ConsoleLine {
                line: "fps on".into(),
            },
            MacroStep::CustomMutation {
                id: "nudge-right".into(),
                target: MacroTarget::CurrentSelection,
            },
        ];
        let json = serde_json::to_string(&steps).unwrap();
        let parsed: Vec<MacroStep> = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.len(), 3);
        assert!(matches!(parsed[0], MacroStep::Action { action: Action::Undo }));
        if let MacroStep::ConsoleLine { line } = &parsed[1] {
            assert_eq!(line, "fps on");
        } else {
            panic!("step 1 should be ConsoleLine");
        }
    }

    /// Locks the on-disk JSON shape for `MacroStep`. Hand-authored
    /// macro files in `~/.config/mandala/macros.json` use these
    /// keys; a future serde-derive change that rearranges them
    /// would silently break user configs.
    #[test]
    fn macro_step_action_json_shape_locked() {
        let step = MacroStep::Action { action: Action::Undo };
        let json = serde_json::to_string(&step).unwrap();
        assert_eq!(json, r#"{"kind":"Action","action":"Undo"}"#);
    }

    #[test]
    fn macro_step_custom_mutation_default_target_omittable() {
        // Authors who omit `target` get CurrentSelection by default.
        let json = r#"{"kind":"CustomMutation","id":"x"}"#;
        let parsed: MacroStep = serde_json::from_str(json).unwrap();
        match parsed {
            MacroStep::CustomMutation { id, target } => {
                assert_eq!(id, "x");
                assert!(matches!(target, MacroTarget::CurrentSelection));
            }
            _ => panic!("expected CustomMutation"),
        }
    }

    #[test]
    fn macro_step_custom_mutation_node_id_target() {
        let json = r#"{"kind":"CustomMutation","id":"x","target":{"node_id":"abc"}}"#;
        let parsed: MacroStep = serde_json::from_str(json).unwrap();
        match parsed {
            MacroStep::CustomMutation { target: MacroTarget::NodeId(s), .. } => {
                assert_eq!(s, "abc");
            }
            _ => panic!("expected NodeId target"),
        }
    }

    #[test]
    fn macro_step_console_line_json_shape() {
        let step = MacroStep::ConsoleLine { line: "save".into() };
        let json = serde_json::to_string(&step).unwrap();
        assert_eq!(json, r#"{"kind":"ConsoleLine","line":"save"}"#);
    }
}
