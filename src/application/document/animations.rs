// SPDX-License-Identifier: MPL-2.0

//! Animation runtime — the `MindMapDocument` methods that build
//! the mutation registry, evaluate triggered mutations, start /
//! tick / fast-forward animations. Also carries
//! `apply_position_mutations_to_node` — the scratch-node position
//! replay helper used during tick.

use baumhard::gfx_structs::area::GlyphAreaCommand;
use baumhard::gfx_structs::mutator::Mutation;
use baumhard::mindmap::custom_mutation::{CustomMutation, PlatformContext, Trigger};
use baumhard::mindmap::model::MindNode;
use baumhard::mindmap::tree_builder::MindMapTree;

use super::mutations_loader::MutationSource;
use super::types::AnimationInstance;
use super::MindMapDocument;

/// Apply position-bearing `Mutation`s to a `MindNode` to derive
/// the `to` snapshot for an animation. Mirrors the GlyphArea
/// command vocabulary of the existing tree mutator path so that
/// "what does this mutation do" has only one definition,
/// regardless of whether it lands instantly or via tween. v1
/// only handles `NudgeLeft` / `NudgeRight` / `NudgeUp` /
/// `NudgeDown`; other commands are no-ops on the model snapshot
/// (their tree-side effect still runs at completion via
/// `apply_custom_mutation`).
fn apply_position_mutations_to_node(mutations: &[Mutation], node: &mut MindNode) {
    for mutation in mutations {
        if let Mutation::AreaCommand(cmd) = mutation {
            match cmd.as_ref() {
                GlyphAreaCommand::NudgeLeft(dx) => {
                    node.position.x -= *dx as f64;
                }
                GlyphAreaCommand::NudgeRight(dx) => {
                    node.position.x += *dx as f64;
                }
                GlyphAreaCommand::NudgeUp(dy) => {
                    node.position.y -= *dy as f64;
                }
                GlyphAreaCommand::NudgeDown(dy) => {
                    node.position.y += *dy as f64;
                }
                // Other commands don't move the node — their
                // visible effect lands at completion via
                // `apply_custom_mutation`.
                _ => {}
            }
        }
    }
}

// `MutationSource` lives in `mutations_loader::MutationSource` —
// imported here via `use` at the top so registry-building can stamp
// source layers into `self.mutation_sources` alongside the registry
// writes. Keeping the type in the loader module groups it with the
// precedence definition it's inseparable from.

impl MindMapDocument {
    /// Build the mutation registry from map-level and inline node mutations.
    /// Inline mutations override map-level mutations with the same id.
    pub fn build_mutation_registry(&mut self) {
        self.build_mutation_registry_with_app_and_user(&[], &[]);
    }

    /// Variant retained for callers that already supply a user slice.
    /// Delegates to the four-source builder with an empty app slice.
    pub fn build_mutation_registry_with_user(&mut self, user_mutations: &[CustomMutation]) {
        self.build_mutation_registry_with_app_and_user(&[], user_mutations);
    }

    /// Build the registry from all four sources. See the
    /// "Where mutations come from" section in `format/mutations.md`
    /// for the canonical precedence description; this method's
    /// loop order (below) mirrors it and the [`MutationSource`]
    /// enum variants at the loader's module doc. Later writers
    /// override earlier ones with the same `id`.
    pub fn build_mutation_registry_with_app_and_user(
        &mut self,
        app_mutations: &[CustomMutation],
        user_mutations: &[CustomMutation],
    ) {
        self.mutation_registry.clear();
        self.mutation_sources.clear();
        for cm in app_mutations {
            self.mutation_registry.insert(cm.id.clone(), cm.clone());
            self.mutation_sources.insert(cm.id.clone(), MutationSource::App);
        }
        for cm in user_mutations {
            self.mutation_registry.insert(cm.id.clone(), cm.clone());
            self.mutation_sources.insert(cm.id.clone(), MutationSource::User);
        }
        for cm in &self.mindmap.custom_mutations {
            self.mutation_registry.insert(cm.id.clone(), cm.clone());
            self.mutation_sources.insert(cm.id.clone(), MutationSource::Map);
        }
        for node in self.mindmap.nodes.values() {
            for cm in &node.inline_mutations {
                self.mutation_registry.insert(cm.id.clone(), cm.clone());
                self.mutation_sources
                    .insert(cm.id.clone(), MutationSource::Inline);
            }
        }
    }

    /// Find custom mutations triggered by a given trigger on a
    /// specific node. Checks the node's trigger_bindings and
    /// filters by platform context. When `section_idx` is
    /// `Some(idx)` and the node has at least `idx + 1` sections,
    /// the targeted section's
    /// [`baumhard::mindmap::model::MindSection::trigger_bindings`]
    /// fire **first** — the user explicitly pointed at that
    /// section, so its overrides take precedence over the whole-
    /// node bindings. Whole-node bindings still fire afterwards
    /// so a section-targeted gesture doesn't accidentally drop a
    /// node-level handler that an author wrote unconditionally.
    pub fn find_triggered_mutations(
        &self,
        node_id: &str,
        trigger: &Trigger,
        platform: &PlatformContext,
    ) -> Vec<CustomMutation> {
        self.find_triggered_mutations_at(node_id, None, trigger, platform)
    }

    /// Section-aware variant of [`Self::find_triggered_mutations`].
    /// `section_idx = Some(_)` consults the targeted section's
    /// per-section bindings first; `None` matches the legacy
    /// whole-node-only flow.
    pub fn find_triggered_mutations_at(
        &self,
        node_id: &str,
        section_idx: Option<usize>,
        trigger: &Trigger,
        platform: &PlatformContext,
    ) -> Vec<CustomMutation> {
        let node = match self.mindmap.nodes.get(node_id) {
            Some(n) => n,
            None => return vec![],
        };
        let mut results = Vec::new();
        let dispatch = |bindings: &[baumhard::mindmap::custom_mutation::TriggerBinding],
                        out: &mut Vec<CustomMutation>| {
            for binding in bindings {
                if &binding.trigger != trigger {
                    continue;
                }
                if !binding.contexts.is_empty() && !binding.contexts.contains(platform) {
                    continue;
                }
                if let Some(cm) = self.mutation_registry.get(&binding.mutation_id) {
                    out.push(cm.clone());
                }
            }
        };
        // Section-level bindings fire first — the user's pointer
        // landed on that specific section, and a section-targeted
        // override (e.g. a different OnClick mutation per section
        // of a multi-stratum node) should beat the catch-all node
        // binding. Dedup by `cm.id` after merging so an author
        // who bound the same `mutation_id` at both layers (e.g.
        // for platform-context splits, or carelessly) doesn't
        // get the mutation applied twice — which would push two
        // undo entries for one click and double the resulting
        // delta.
        if let Some(idx) = section_idx {
            if let Some(section) = node.sections.get(idx) {
                dispatch(&section.trigger_bindings, &mut results);
            }
        }
        dispatch(&node.trigger_bindings, &mut results);
        let mut seen: rustc_hash::FxHashSet<String> = rustc_hash::FxHashSet::default();
        results.retain(|cm| seen.insert(cm.id.clone()));
        results
    }

    // ---- Animation lifecycle ----
    //
    // Animations are an *envelope* on `apply_custom_mutation` — when
    // a `CustomMutation` carries `timing: Some(AnimationTiming { ... })`
    // with a non-zero `duration_ms`, the dispatcher routes it through
    // `start_animation` instead of applying instantly. Each tick
    // computes a blended `MindNode` snapshot and writes it back into
    // `mindmap.nodes` so the existing `rebuild_all` path sees the
    // in-progress state and repaints. The tree never sees the from
    // state mid-flight; the render pipeline reads model → builds tree
    // → walks → shapes, so the model write is the single source of
    // truth for the animated frame.
    //
    // The architecture mirrors the dragging / editing invariant: the
    // *tree* (or in this case, the model — drag and edit work
    // tree-only because they're transient previews) carries the
    // in-progress state, the model is the boundary commit. For
    // animations we write to the model directly because the
    // animation IS the commit — `apply_custom_mutation` would have
    // produced the same final state anyway, just in one step
    // instead of many. The undo entry is pushed once at completion.
    //
    // Interpolation scope: position / size / color are the candidate
    // fields; structural changes (text replacement, region count
    // shifts) snap at the boundary. Currently only `position` is
    // interpolated — the other fields need per-mutation snapshot
    // logic that's deferred until a concrete consumer arrives.

    /// Start an animation for `cm` targeting `target_id`. Snapshots
    /// the current node state, applies the mutation to a scratch
    /// copy to derive the `to` snapshot, and pushes an
    /// [`AnimationInstance`] onto [`Self::active_animations`]. The
    /// caller has already verified
    /// `cm.timing.as_ref().is_some_and(|t| t.duration_ms > 0)`.
    ///
    /// **Current scope** (expanded as concrete consumers arrive):
    /// only `TargetScope::SelfOnly` interpolates per-frame; other
    /// scopes apply at the boundary. Only `position` is lerped
    /// continuously; text / regions / structural fields snap at
    /// completion. `Followup` variants (`Reverse`, `Chain`, `Loop`)
    /// are recorded on the instance but not yet enacted.
    /// Section-aware overload of [`Self::start_animation`]. When
    /// `section_idx = Some(_)`, the re-trigger dedup key includes
    /// the section index so adjacent sections of the same node
    /// can host concurrent animations bound to the same mutation
    /// id without coalescing. Section-targeted animations also
    /// carry the index on the resulting [`AnimationInstance`] so
    /// future per-section interpolators can lerp the right
    /// element.
    ///
    /// Today the interpolation surface is whole-node `position`
    /// only — the section-aware completion still routes through
    /// `apply_custom_mutation` which honours `target_scope:
    /// SectionsOnly`, so the committed final state lands on the
    /// section. Per-frame interpolation of section-area
    /// `position` is the named seam this signature opens for
    /// future work.
    pub fn start_animation_at(
        &mut self,
        cm: &CustomMutation,
        target_id: &str,
        section_idx: Option<usize>,
        now_ms: u64,
    ) {
        self.start_animation_inner(cm, target_id, section_idx, now_ms);
    }

    pub fn start_animation(&mut self, cm: &CustomMutation, target_id: &str, now_ms: u64) {
        self.start_animation_inner(cm, target_id, None, now_ms);
    }

    fn start_animation_inner(
        &mut self,
        cm: &CustomMutation,
        target_id: &str,
        section_idx: Option<usize>,
        now_ms: u64,
    ) {
        // Invariant check the `AnimationInstance::timing()`
        // projection relies on: `cm.timing` must be Some with a
        // non-zero duration, else the caller should have taken
        // the instant-mutation path.
        if !cm.timing.as_ref().is_some_and(|t| t.duration_ms > 0) {
            return;
        }

        // Re-trigger the same (mutation_id, node_id, section_idx)
        // mid-flight is a silent no-op — otherwise a held button
        // could spawn dozens of overlapping instances and the blend
        // would overshoot. Section_idx is part of the key so two
        // simultaneous animations of the same mutation against
        // different sections of the same node coexist instead of
        // coalescing.
        if self
            .active_animations
            .iter()
            .any(|a| a.mutation_id() == cm.id && a.target_id == target_id && a.section_idx == section_idx)
        {
            return;
        }

        // Snapshot the from state.
        let Some(from_node) = self.mindmap.nodes.get(target_id).cloned() else {
            return;
        };

        // Compute the to state by applying the mutation to a scratch
        // copy of the document. The scratch path uses the existing
        // GlyphArea command vocabulary so animation receives the same
        // final state instant-mode would have landed on — there's
        // only one source of truth for "what does this mutation do".
        // Extract the flat Mutation list from the mutator AST for the
        // scratch-node replay. MutatorNode shapes with runtime holes
        // (size-aware mutations) can't be previewed against a single
        // model node — the scratch stays at `from` and the animation
        // lerps to whatever the mutator produces at completion.
        let mut scratch = from_node.clone();
        let flat = cm
            .mutator
            .as_ref()
            .and_then(baumhard::mindmap::custom_mutation::flat_mutations)
            .unwrap_or_default();
        apply_position_mutations_to_node(&flat, &mut scratch);
        let to_node = scratch;

        self.active_animations.push(AnimationInstance {
            target_id: target_id.to_string(),
            section_idx,
            from_node,
            to_node,
            start_ms: now_ms,
            cm: cm.clone(),
        });
    }

    /// Tick every active animation against the wall clock at
    /// `now_ms`. For each instance, lerp position from `from_node`
    /// to `to_node` according to the easing curve and write the
    /// blended state back into `mindmap.nodes`. Returns `true` iff
    /// any animation advanced (so the caller knows to trigger a
    /// scene rebuild).
    ///
    /// Animations whose elapsed time has reached `duration_ms +
    /// delay_ms` complete: their final state is committed via
    /// `apply_custom_mutation` (so the standard
    /// model-sync + undo-push path runs exactly once), then the
    /// instance is dropped. Drain order is back-to-front so
    /// `swap_remove` is safe.
    pub fn tick_animations(&mut self, now_ms: u64, mut tree: Option<&mut MindMapTree>) -> bool {
        if self.active_animations.is_empty() {
            return false;
        }

        let mut completed_indices: Vec<usize> = Vec::new();
        let mut any_advanced = false;

        for (idx, anim) in self.active_animations.iter().enumerate() {
            let timing = anim.timing();
            let elapsed = now_ms.saturating_sub(anim.start_ms);
            let total = timing.delay_ms as u64 + timing.duration_ms as u64;
            if elapsed >= total {
                completed_indices.push(idx);
                continue;
            }
            // Skip the delay phase — node stays at `from` until the
            // delay elapses.
            if elapsed < timing.delay_ms as u64 {
                continue;
            }
            let progress = (elapsed - timing.delay_ms as u64) as f32 / timing.duration_ms as f32;
            let t = timing.easing.evaluate(progress);

            let node = match self.mindmap.nodes.get_mut(&anim.target_id) {
                Some(n) => n,
                None => continue,
            };
            let lerped = anim.from_node.pos_vec2().lerp(anim.to_node.pos_vec2(), t);
            node.position.x = lerped.x as f64;
            node.position.y = lerped.y as f64;
            any_advanced = true;
        }

        if !completed_indices.is_empty() {
            // Drain completed animations. Apply each one's final
            // state through `apply_custom_mutation` — that's the
            // single path that handles model-sync + undo-push for
            // both Persistent and Toggle behaviour, so the tree
            // animation's commit is indistinguishable from the
            // instant-mode equivalent.
            for idx in completed_indices.into_iter().rev() {
                let anim = self.active_animations.swap_remove(idx);
                if let Some(tree) = tree.as_deref_mut() {
                    self.apply_custom_mutation(&anim.cm, &anim.target_id, Some(tree));
                } else {
                    // No tree available — at minimum restore the
                    // model to the `to` snapshot so the next
                    // rebuild_all sees the post-animation state.
                    if let Some(node) = self.mindmap.nodes.get_mut(&anim.target_id) {
                        node.position = anim.to_node.position.clone();
                    }
                }
                any_advanced = true;
            }
        }

        any_advanced
    }

    /// `true` while one or more animations are still ticking.
    /// Used by the event loop to decide whether to keep emitting
    /// `AboutToWait` work and rebuilding the scene.
    pub fn has_active_animations(&self) -> bool {
        !self.active_animations.is_empty()
    }

    /// Fast-forward every active animation to its `to` state and
    /// commit it through `apply_custom_mutation` (which pushes
    /// one undo entry per completed animation). Called by the
    /// Ctrl+Z handler before `undo()` so mid-animation Ctrl+Z
    /// has predictable semantics: the animation snaps to
    /// completion, the undo entry it would have pushed at the
    /// natural boundary is pushed immediately, and the
    /// subsequent `undo()` pops that entry — so Ctrl+Z during
    /// an animated transition reverses the animation's effect
    /// in one keystroke, same as Ctrl+Z after the animation
    /// completed naturally.
    ///
    /// Drains `active_animations` wholesale. Order within the
    /// drain doesn't matter because each instance commits
    /// independently and pushes its own undo entry.
    pub fn fast_forward_animations(&mut self, tree: Option<&mut MindMapTree>) {
        if self.active_animations.is_empty() {
            return;
        }
        let drained = std::mem::take(&mut self.active_animations);
        let mut tree = tree;
        for anim in drained {
            if let Some(tree) = tree.as_deref_mut() {
                self.apply_custom_mutation(&anim.cm, &anim.target_id, Some(tree));
            } else if let Some(node) = self.mindmap.nodes.get_mut(&anim.target_id) {
                // No tree available — restore the model to the
                // `to` snapshot directly. Undo path is then the
                // caller's responsibility, matching what
                // `tick_animations` does on its no-tree path.
                node.position = anim.to_node.position.clone();
            }
        }
    }
}
