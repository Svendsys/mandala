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
/// `Inline` tiers with a `warn!`. All four tiers load today on
/// native; the gate is fully active.
///
/// Document-mutating step kinds (`Action`, `CustomMutation`) also
/// gate via [`MacroSource::allows_action`] — see the denylist
/// there for which Actions are blocked from non-User tiers.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum MacroSource {
    /// Shipped with the binary via
    /// `assets/macros/application.json`. Loaded by
    /// [`crate::application::macros::loader::load_app_macros`] at
    /// startup.
    App,
    /// Loaded from the user's `macros.json`
    /// (`$XDG_CONFIG_HOME/mandala/macros.json` on native). Same
    /// trust posture as `keybinds.json` — the user owns the file.
    User,
    /// Declared in `MindMap.macros` on the currently-loaded
    /// document. Reloaded on every `open` / `new` console verb via
    /// [`crate::application::macros::loader::rebuild_map_macros`].
    Map,
    /// Declared on individual nodes via `MindNode.inline_macros`.
    /// Walked across every node and aggregated into the registry
    /// by
    /// [`crate::application::macros::loader::rebuild_inline_macros`].
    /// Highest precedence — overrides Map / User / App on id
    /// collision.
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

    /// Whether macros from this source may invoke the given Action
    /// via a `MacroStep::Action`. Symmetric with
    /// [`allows_console_line`]: only `User`-tier macros can fire
    /// the destructive / I/O / clipboard-touching Actions, since
    /// other tiers may load from untrusted sources (a hostile
    /// `.mindmap.json` could otherwise bind `Action::SaveDocument`
    /// to a hotkey and overwrite the user's file the next time
    /// they press the bound key).
    ///
    /// All four tiers load today on native; the gate is fully
    /// active. Adding a new privileged `Action` variant requires
    /// updating the denylist below by hand — the denylist is
    /// open by default, so a missing entry silently bypasses the
    /// gate rather than raising a compile error. The structural
    /// reminder lives on the `Action::wasm_compatibility` exhaustive
    /// match, which `#[non_exhaustive]` *does* enforce; that match
    /// is the forcing function for "review every new variant
    /// against the cross-cutting policy concerns" — including
    /// whether the variant belongs on this denylist.
    pub fn allows_action(self, action: &Action) -> bool {
        if matches!(self, MacroSource::User) {
            return true;
        }
        // Block destructive / persistent / clipboard / document-
        // lifecycle Actions for non-User tiers. The closed list
        // makes the gate explicit; new Actions default to "allowed"
        // because they are typically navigation / view-state shaped.
        // Adding a new Action that deserves blocking goes here.
        //
        // `DoubleClickActivate` is blocked because its `Empty`-hit
        // branch synthesises an orphan-create gesture (which
        // bypasses the direct `CreateOrphanNodeAndEdit` block).
        // Today macros never carry a `DispatchHit` so the empty
        // branch is unreachable from a macro — but this is a
        // forward-compat block for any future contributor who
        // synthesises a hit for macro-triggered double-click.
        //
        // `EditSelection` / `EditSelectionClean` are blocked
        // because their `EdgeLabel` / `PortalText` / `PortalLabel`
        // selection branches open inline editors that mutate the
        // model on commit. A hostile macro firing
        // `EditSelectionClean` while an edge label is selected
        // would erase the label's content via the empty buffer
        // open. The Compatible Node-only branch alone isn't
        // worth the gate exception.
        let blocked = matches!(
            action,
            Action::SaveDocument
                | Action::DeleteSelection
                | Action::OrphanSelection
                | Action::CreateOrphanNode
                | Action::CreateOrphanNodeAndEdit
                | Action::DoubleClickActivate
                | Action::EditSelection
                | Action::EditSelectionClean
                // Reaches `open_label_edit` / `open_portal_text_edit` —
                // the same dispatch surface that gates `EditSelection`.
                // Omitting this was a privilege-denylist gap caught in
                // post-PR review.
                | Action::LabelEditOnSelection
                | Action::Copy
                | Action::Cut
                | Action::Paste
                | Action::NewDocument
        );
        !blocked
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

/// Number of `MacroSource` tiers. Lock-stepped with the variant
/// count of the enum; the registry's per-id slot array is sized
/// against this constant.
const TIER_COUNT: usize = 4;

impl MacroSource {
    /// Index into the per-tier slot array on `MacroRegistry`. Order
    /// matches the variant declaration so `MacroSource as usize`
    /// would conceptually agree, but the explicit `match` survives
    /// future re-ordering and `#[non_exhaustive]` keeps it honest.
    /// Higher index = higher precedence. Module-private — only the
    /// registry's slot array consumes it.
    const fn index(self) -> usize {
        match self {
            MacroSource::App => 0,
            MacroSource::User => 1,
            MacroSource::Map => 2,
            MacroSource::Inline => 3,
        }
    }
}

/// In-memory lookup table. Built once at startup from the loader's
/// merged slices, then refreshed on every document-replace.
///
/// **Shadow-stacked storage.** Each id maps to a per-tier slot
/// array indexed by [`MacroSource::index`]: `[App, User, Map,
/// Inline]`. Lookup walks high-to-low precedence; the first
/// non-None slot wins. Higher-tier entries SHADOW lower-tier ones
/// rather than displacing them — `clear_tier(Inline)` removes
/// only the Inline slot, and `get` / `get_with_source` then
/// resolve to whichever lower-tier entry exists. This fixes the
/// "displacement is permanent within the session" problem the
/// reviewers flagged: a User-tier `id="x"` shadowed by a Map-tier
/// `id="x"` re-emerges naturally when the document is replaced
/// (which clears the Map tier).
///
/// Within-tier collisions (e.g. two entries with `id="x"` in the
/// same Map-tier `mindmap.macros` array) still last-writer-wins —
/// only the cross-tier reveal property is new.
#[derive(Debug, Clone, Default)]
pub struct MacroRegistry {
    macros: HashMap<String, [Option<Macro>; TIER_COUNT]>,
}

impl MacroRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    /// Write a macro into the slot for `source`. Returns the
    /// previous entry AT THE SAME TIER — not the displaced lower-
    /// tier entry, since lower tiers are no longer displaced.
    /// Within-tier last-writer-wins; cross-tier coexistence is
    /// preserved.
    pub fn insert(&mut self, m: Macro, source: MacroSource) -> Option<Macro> {
        let id = m.id.clone();
        let slots = self
            .macros
            .entry(id)
            .or_insert_with(|| std::array::from_fn(|_| None));
        slots[source.index()].replace(m)
    }

    /// Look up the highest-tier macro for `id`. Walks the slot
    /// array from Inline → Map → User → App and returns the first
    /// non-None entry.
    pub fn get(&self, id: &str) -> Option<&Macro> {
        let slots = self.macros.get(id)?;
        for i in (0..TIER_COUNT).rev() {
            if let Some(m) = &slots[i] {
                return Some(m);
            }
        }
        None
    }

    /// Look up the highest-tier macro for `id` and the tier that
    /// holds it. The dispatcher uses this pair to consult the
    /// privilege gate (`MacroSource::allows_console_line`,
    /// `allows_action`). Same walk order as `get`.
    pub fn get_with_source(&self, id: &str) -> Option<(&Macro, MacroSource)> {
        let slots = self.macros.get(id)?;
        // Walk tiers high-to-low. The list is hand-written so the
        // tier→index mapping stays explicit; if a future tier is
        // added, this list must extend AND `index` above must too.
        for tier in [
            MacroSource::Inline,
            MacroSource::Map,
            MacroSource::User,
            MacroSource::App,
        ] {
            if let Some(m) = &slots[tier.index()] {
                return Some((m, tier));
            }
        }
        None
    }

    /// Whether any tier slot holds an entry for `id`.
    pub fn contains(&self, id: &str) -> bool {
        self.macros
            .get(id)
            .map_or(false, |slots| slots.iter().any(Option::is_some))
    }

    /// Number of distinct ids with at least one tier slot occupied.
    /// An id present at multiple tiers counts once.
    pub fn len(&self) -> usize {
        self.macros
            .values()
            .filter(|slots| slots.iter().any(Option::is_some))
            .count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Iterate registered macro ids. An id present at multiple
    /// tiers appears once. HashMap iteration order is unspecified;
    /// sort at the call site if a stable display is needed.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.macros
            .iter()
            .filter(|(_, slots)| slots.iter().any(Option::is_some))
            .map(|(k, _)| k.as_str())
    }

    /// Clear the slot for `source` across every id. Lower-tier
    /// entries with the same id survive in their slots —
    /// `get` / `get_with_source` will resolve them once this tier
    /// is cleared. Drops the HashMap entry entirely if all four
    /// slots become None.
    ///
    /// This is the load-bearing piece of the shadow-stack design:
    /// document-replace paths can clear Map + Inline tiers without
    /// disturbing the App + User tiers loaded at startup, AND the
    /// previously-shadowed User-tier entries re-emerge naturally
    /// in subsequent lookups.
    pub fn clear_tier(&mut self, source: MacroSource) {
        let idx = source.index();
        self.macros.retain(|_, slots| {
            slots[idx] = None;
            slots.iter().any(Option::is_some)
        });
    }

    /// Bulk-insert macros at the given tier. Within-tier id
    /// collisions follow last-writer-wins (later iterator entries
    /// override earlier ones). Cross-tier entries at OTHER tiers
    /// survive in their slots — this is the shadow-stacking
    /// property.
    pub fn extend_with_tier<I: IntoIterator<Item = Macro>>(
        &mut self,
        macros: I,
        source: MacroSource,
    ) {
        for m in macros {
            self.insert(m, source);
        }
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
    fn macro_source_allows_action_gates_destructive_actions_for_non_user() {
        // User passes everything.
        for a in [
            Action::SaveDocument,
            Action::DeleteSelection,
            Action::Cut,
            Action::Paste,
            Action::Copy,
            Action::OrphanSelection,
            Action::CreateOrphanNode,
            Action::CreateOrphanNodeAndEdit,
            Action::NewDocument,
        ] {
            assert!(
                MacroSource::User.allows_action(&a),
                "User tier should allow {:?}",
                a
            );
        }
        // Non-User tiers reject all of the above.
        for tier in [MacroSource::App, MacroSource::Map, MacroSource::Inline] {
            for a in [
                Action::SaveDocument,
                Action::DeleteSelection,
                Action::Cut,
                Action::Paste,
                Action::Copy,
                Action::OrphanSelection,
                Action::CreateOrphanNode,
                Action::CreateOrphanNodeAndEdit,
                // Mixed-branch destructive Actions — their dispatch
                // arms reach editor modals that mutate model state
                // on commit.
                Action::DoubleClickActivate,
                Action::EditSelection,
                Action::EditSelectionClean,
                // Same dispatch surface (`open_label_edit` /
                // `open_portal_text_edit`) as `EditSelection` — a
                // hostile macro firing this with an edge label /
                // portal selection forces the user into an inline
                // editor over selected content.
                Action::LabelEditOnSelection,
                Action::NewDocument,
            ] {
                assert!(
                    !tier.allows_action(&a),
                    "{:?} tier should reject {:?}",
                    tier,
                    a
                );
            }
        }
    }

    #[test]
    fn macro_source_allows_action_passes_navigation_for_non_user() {
        // Non-User tiers may still invoke navigation / view-state
        // Actions — they don't touch the filesystem or destroy data.
        for tier in [MacroSource::App, MacroSource::Map, MacroSource::Inline] {
            for a in [
                Action::ZoomIn,
                Action::ZoomOut,
                Action::ZoomReset,
                Action::ZoomFit,
                Action::SelectAll,
                Action::DeselectAll,
                Action::CenterOnSelection,
                Action::JumpToRoot,
                Action::Undo,
            ] {
                assert!(
                    tier.allows_action(&a),
                    "{:?} tier should allow non-destructive {:?}",
                    tier,
                    a
                );
            }
        }
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

    /// Inline-tier macros override every lower tier on id
    /// collision. Locks the precedence contract for the highest-
    /// precedence tier — the property load-bearing for the
    /// privilege gate, since Inline is the most-likely-untrusted
    /// Inline tier wins on lookup when multiple tiers hold an
    /// entry for the same id. Critical for the privilege gate —
    /// Inline-tier macros are denylisted from `ConsoleLine` and
    /// destructive Actions, so a hostile mindmap shadowing a
    /// User-tier macro must be picked up at the Inline tier on
    /// lookup, not the lower User tier.
    #[test]
    fn macro_registry_inline_overrides_user_and_map_by_id() {
        let mut reg = MacroRegistry::new();
        let mk = |name: &str| Macro {
            id: "shared".into(),
            name: name.into(),
            description: "".into(),
            steps: vec![],
        };
        // Order matches run_native_init::build: App → User →
        // Map → Inline.
        reg.insert(mk("app"), MacroSource::App);
        reg.insert(mk("user"), MacroSource::User);
        reg.insert(mk("map"), MacroSource::Map);
        // Inserting Inline does NOT displace Map; both coexist
        // under the shadow-stack design. `insert` returns the
        // previous slot at the SAME tier (None here — this is
        // the first Inline write).
        let prev = reg.insert(mk("inline"), MacroSource::Inline);
        assert!(
            prev.is_none(),
            "no prior Inline-tier entry; Map slot should not have been touched"
        );
        // Lookup walks high-to-low and finds Inline first.
        let (m, src) = reg.get_with_source("shared").unwrap();
        assert_eq!(m.name, "inline");
        assert_eq!(src, MacroSource::Inline);
    }

    /// Higher-tier macros SHADOW lower-tier ones with the same id —
    /// they don't displace. Clearing the higher tier reveals the
    /// lower-tier entry underneath. The reveal property fixes the
    /// "displacement is permanent within the session" issue
    /// reviewers flagged.
    #[test]
    fn macro_registry_user_overrides_app_by_id() {
        let mut reg = MacroRegistry::new();
        let app_macro = Macro {
            id: "shared-id".into(),
            name: "App version".into(),
            description: String::new(),
            steps: vec![],
        };
        let user_macro = Macro {
            id: "shared-id".into(),
            name: "User version".into(),
            description: String::new(),
            steps: vec![MacroStep::Action {
                action: Action::Undo,
            }],
        };
        // Insert order matches run_native_init::build: App first.
        reg.insert(app_macro, MacroSource::App);
        // First write to the User tier — no prior User entry to
        // return.
        let prev = reg.insert(user_macro, MacroSource::User);
        assert!(prev.is_none(), "no prior User-tier entry");
        // Lookup walks high-to-low → User wins.
        let (m, src) = reg.get_with_source("shared-id").unwrap();
        assert_eq!(m.name, "User version");
        assert_eq!(src, MacroSource::User);
        // Critical for the privilege gate: the tier upgrade is
        // honoured (User can run ConsoleLine / privileged Actions
        // even if an App macro had the same id first).
        assert!(src.allows_console_line());

        // Reveal property: clearing User exposes the App entry
        // underneath. (The TODO note about displacement-is-
        // permanent is closed by this test.)
        reg.clear_tier(MacroSource::User);
        let (m, src) = reg
            .get_with_source("shared-id")
            .expect("App entry must re-emerge");
        assert_eq!(m.name, "App version");
        assert_eq!(src, MacroSource::App);
        assert!(!src.allows_console_line(), "App tier rejects ConsoleLine");
    }

    /// Sentinel for `TIER_COUNT` ↔ `MacroSource` variant-count drift.
    /// If a future contributor adds a fifth `MacroSource` variant
    /// without bumping `TIER_COUNT`, `MacroSource::index()` would
    /// return an out-of-bounds index for the new variant and
    /// every lookup involving it would panic in `dispatch_macro`.
    /// Inserting at every existing tier and asserting the count
    /// catches the omission at test time rather than at runtime.
    #[test]
    fn macro_registry_every_tier_holds_an_entry() {
        let mut reg = MacroRegistry::new();
        let make = |id: &str| Macro {
            id: id.into(),
            name: String::new(),
            description: String::new(),
            steps: vec![],
        };
        for (i, src) in [
            MacroSource::App,
            MacroSource::User,
            MacroSource::Map,
            MacroSource::Inline,
        ]
        .iter()
        .enumerate()
        {
            reg.insert(make(&format!("id-{}", i)), *src);
            assert!(
                reg.get_with_source(&format!("id-{}", i)).is_some(),
                "tier {:?} (TIER_COUNT={}) failed to store an entry — \
                 likely a TIER_COUNT/MacroSource drift",
                src,
                TIER_COUNT,
            );
        }
        assert_eq!(reg.len(), 4);
    }

    /// `clear_tier` removes only entries with the matching source —
    /// other tiers stay in place. Critical for the document-replace
    /// path: opening a different document must wipe Map-tier macros
    /// from the previous document while keeping App / User intact.
    #[test]
    fn macro_registry_clear_tier_only_drops_matching() {
        let mut reg = MacroRegistry::new();
        reg.insert(
            Macro {
                id: "app".into(),
                name: "".into(),
                description: "".into(),
                steps: vec![],
            },
            MacroSource::App,
        );
        reg.insert(
            Macro {
                id: "user".into(),
                name: "".into(),
                description: "".into(),
                steps: vec![],
            },
            MacroSource::User,
        );
        reg.insert(
            Macro {
                id: "map".into(),
                name: "".into(),
                description: "".into(),
                steps: vec![],
            },
            MacroSource::Map,
        );
        assert_eq!(reg.len(), 3);
        reg.clear_tier(MacroSource::Map);
        assert_eq!(reg.len(), 2);
        assert!(reg.contains("app"));
        assert!(reg.contains("user"));
        assert!(!reg.contains("map"));
    }

    /// Shadow-stack reveal property — the central new behaviour
    /// of the per-tier slot design. Clearing a higher tier reveals
    /// the lower-tier entry underneath instead of leaving the id
    /// permanently displaced.
    #[test]
    fn clear_higher_tier_reveals_lower_tier() {
        let mut reg = MacroRegistry::new();
        let mk = |name: &str| Macro {
            id: "x".into(),
            name: name.into(),
            description: String::new(),
            steps: vec![],
        };
        reg.insert(mk("user"), MacroSource::User);
        reg.insert(mk("map"), MacroSource::Map);

        // Map shadows User on lookup.
        let (m, src) = reg.get_with_source("x").unwrap();
        assert_eq!(m.name, "map");
        assert_eq!(src, MacroSource::Map);

        // Clearing Map reveals User — does not leave "x" gone.
        reg.clear_tier(MacroSource::Map);
        let (m, src) = reg
            .get_with_source("x")
            .expect("User entry should re-emerge after Map clear");
        assert_eq!(m.name, "user");
        assert_eq!(src, MacroSource::User);
    }

    /// `clear_tier(X)` only zeroes the slot for tier `X` on each
    /// id. The other tiers' slots survive in place. (The
    /// `macro_registry_clear_tier_only_drops_matching` test
    /// above covers the case where each id has only one slot
    /// occupied; this test covers the case where ids have
    /// multiple tiers stacked.)
    #[test]
    fn clear_tier_removes_only_matching_slot() {
        let mut reg = MacroRegistry::new();
        let mk = |id: &str, name: &str| Macro {
            id: id.into(),
            name: name.into(),
            description: String::new(),
            steps: vec![],
        };
        // Stack two tiers on "x" and one tier each on "y" / "z".
        reg.insert(mk("x", "x.app"), MacroSource::App);
        reg.insert(mk("x", "x.user"), MacroSource::User);
        reg.insert(mk("y", "y.user"), MacroSource::User);
        reg.insert(mk("z", "z.map"), MacroSource::Map);

        reg.clear_tier(MacroSource::User);

        // "x" still has its App entry — id survives, just at a
        // lower tier.
        let (m, src) = reg.get_with_source("x").unwrap();
        assert_eq!(m.name, "x.app");
        assert_eq!(src, MacroSource::App);
        // "y" had only User → id evicted entirely.
        assert!(!reg.contains("y"));
        // "z" had only Map → unaffected.
        let (m, src) = reg.get_with_source("z").unwrap();
        assert_eq!(m.name, "z.map");
        assert_eq!(src, MacroSource::Map);
    }

    /// Document-replace cycle: a Map-tier macro shadows a
    /// User-tier macro in document A; opening document B (which
    /// has no `mindmap.macros`) clears the Map tier and the
    /// User-tier macro re-emerges. Locks the load-bearing
    /// user-visible behaviour change shadow-stacking enables.
    #[test]
    fn cross_tier_within_session_round_trip() {
        let mut reg = MacroRegistry::new();
        let mk = |source_label: &str| Macro {
            id: "save-and-quit".into(),
            name: source_label.into(),
            description: String::new(),
            steps: vec![],
        };
        // Startup: User tier loaded.
        reg.insert(mk("user-macro"), MacroSource::User);
        // Open document A: Map tier shadows User.
        reg.insert(mk("docA-macro"), MacroSource::Map);
        assert_eq!(
            reg.get_with_source("save-and-quit").unwrap().0.name,
            "docA-macro"
        );

        // Open document B: replace path runs `clear_tier(Map)`
        // first, then `extend_with_tier(empty, Map)`.
        reg.clear_tier(MacroSource::Map);
        reg.extend_with_tier(Vec::<Macro>::new(), MacroSource::Map);

        // User entry re-emerges.
        let (m, src) = reg
            .get_with_source("save-and-quit")
            .expect("User entry restored");
        assert_eq!(m.name, "user-macro");
        assert_eq!(src, MacroSource::User);
    }

    /// Within-tier id collisions follow last-writer-wins —
    /// documented at `format/macros.md` "Within-tier and cross-
    /// tier collision semantics". A Map-tier `mindmap.macros`
    /// array with two entries both `id: "x"` keeps only the
    /// second.
    #[test]
    fn macro_registry_extend_with_tier_within_tier_collision_is_last_writer() {
        let mut reg = MacroRegistry::new();
        let macros = vec![
            Macro {
                id: "x".into(),
                name: "first".into(),
                description: "".into(),
                steps: vec![],
            },
            Macro {
                id: "x".into(),
                name: "second".into(),
                description: "".into(),
                steps: vec![],
            },
        ];
        reg.extend_with_tier(macros, MacroSource::Map);
        assert_eq!(reg.len(), 1);
        let (m, _src) = reg.get_with_source("x").unwrap();
        assert_eq!(m.name, "second", "later writer wins within a tier");
    }

    /// `extend_with_tier` is the bulk-insert form used by the
    /// document-load path. Combined with `clear_tier`, it gives a
    /// "wipe and reinstall this tier" idiom.
    #[test]
    fn macro_registry_extend_with_tier_inserts_at_correct_source() {
        let mut reg = MacroRegistry::new();
        let macros = vec![
            Macro {
                id: "a".into(),
                name: "".into(),
                description: "".into(),
                steps: vec![],
            },
            Macro {
                id: "b".into(),
                name: "".into(),
                description: "".into(),
                steps: vec![],
            },
        ];
        reg.extend_with_tier(macros, MacroSource::Map);
        assert_eq!(reg.len(), 2);
        let (_, src_a) = reg.get_with_source("a").unwrap();
        let (_, src_b) = reg.get_with_source("b").unwrap();
        assert_eq!(src_a, MacroSource::Map);
        assert_eq!(src_b, MacroSource::Map);
        // Both tagged Map — should reject ConsoleLine and destructive
        // Actions (the privilege gate).
        assert!(!src_a.allows_console_line());
        assert!(!src_a.allows_action(&Action::SaveDocument));
    }

    /// `MacroRegistry::get_with_source` returns the loader-pinned
    /// tier alongside the macro. This is the load-bearing accessor
    /// the dispatcher uses to gate `ConsoleLine` and privileged
    /// `Action` steps. Without it, a future caller that uses bare
    /// `get` would silently bypass the gate.
    #[test]
    fn macro_registry_get_with_source_returns_pinned_tier() {
        let mut reg = MacroRegistry::new();
        let map_macro = Macro {
            id: "hostile".into(),
            name: "From a hostile mindmap".into(),
            description: String::new(),
            steps: vec![MacroStep::ConsoleLine {
                line: "save /tmp/evil".into(),
            }],
        };
        reg.insert(map_macro, MacroSource::Map);
        let (m, src) = reg.get_with_source("hostile").unwrap();
        assert_eq!(src, MacroSource::Map);
        // The dispatcher looks at this exact pair to decide gating.
        assert!(!src.allows_console_line());
        assert!(matches!(&m.steps[0], MacroStep::ConsoleLine { .. }));
    }
}
