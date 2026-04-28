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
/// merged slices.
#[derive(Debug, Clone, Default)]
pub struct MacroRegistry {
    macros: HashMap<String, Macro>,
}

impl MacroRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert / replace a macro by id. Returns the previous entry if
    /// any was present — caller decides whether to log the override.
    pub fn insert(&mut self, m: Macro) -> Option<Macro> {
        self.macros.insert(m.id.clone(), m)
    }

    /// Look up a macro by id.
    pub fn get(&self, id: &str) -> Option<&Macro> {
        self.macros.get(id)
    }

    /// Whether the registry knows about this id.
    pub fn contains(&self, id: &str) -> bool {
        self.macros.contains_key(id)
    }

    /// Number of registered macros. Used by the `mutation list`-
    /// shaped surface a future `macro list` console verb will share.
    pub fn len(&self) -> usize {
        self.macros.len()
    }

    pub fn is_empty(&self) -> bool {
        self.macros.is_empty()
    }

    /// Iterate registered macro ids — used by completion / inspection
    /// surfaces.
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
        reg.insert(m.clone());
        assert_eq!(reg.len(), 1);
        assert!(reg.contains("test"));
        let got = reg.get("test").unwrap();
        assert_eq!(got.id, "test");
        assert_eq!(got.steps.len(), 1);
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
        reg.insert(m1);
        let prev = reg.insert(m2);
        assert!(prev.is_some());
        assert_eq!(reg.get("x").unwrap().name, "second");
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
}
