// SPDX-License-Identifier: MPL-2.0

//! Custom-mutation infrastructure — `apply_custom_mutation` and
//! its helpers. The bridge between the declarative
//! `CustomMutation` shape and the document's mutation-and-undo
//! plumbing.

use baumhard::core::primitives::ColorFontRegion;
use baumhard::font::fonts::family_name_of;
use baumhard::mindmap::custom_mutation::{
    apply_mutations_to_element, flat_mutations, mutator_reach, CustomMutation, DocumentAction,
    MutationBehavior, TargetScope,
};
use baumhard::mindmap::model::{MindNode, TextRun};
use baumhard::mindmap::tree_builder::MindMapTree;
use baumhard::util::color_conversion::rgba_to_hex;

use super::mutations_loader::MutationSource;
use super::nodes::clamp_runs_to_text;
use super::undo_action::UndoAction;
use super::MindMapDocument;

/// Default text-run colour when neither the tree-side region nor
/// a prior model run carries one. Matches the renderer's
/// fall-through-to-`#ffffff` floor on a node with no explicit
/// `style.text_color` override.
const DEFAULT_TEXT_RUN_COLOR: &str = "#ffffff";

/// Default font-size used by the renderer when no run pins one.
/// Mirrors `cosmic_text`'s 14pt fallback used at scene-build time.
const DEFAULT_TEXT_RUN_SIZE_PT: u32 = 14;

/// Roll a tree-side [`ColorFontRegion`] back into a model-side
/// [`TextRun`], merging fields the tree dropped during the
/// forward conversion against a `prior` run when the prior
/// covered the same `Range`. The forward path
/// (`tree_builder/node.rs::append_node_sections`) only carries
/// `range`, `color`, and `font` onto the tree-side region;
/// `bold` / `italic` / `underline` / `size_pt` / `hyperlink`
/// disappear into the cosmic-text default attribute set. The
/// reverse path can recover them only when a matching prior run
/// is available — which is true for round-trips through the
/// custom-mutation pipeline (the tree is rebuilt from the model
/// just before each apply, so every region's range was an
/// authored run before the mutation ran).
///
/// Limitations:
/// - `var(--name)` colour references collapse to their resolved
///   hex on the round trip — the tree-side `FloatRgba` carries
///   no record of the variable. Authors who edit text colours
///   through custom mutations and then save the model will see
///   the variable replaced with a hex literal.
/// - Unknown `AppFont` (corrupt tree state) falls through to
///   the empty string, matching the loader's tolerance for
///   missing-font runs.
/// Warn at apply time when a predicate gate filtered every
/// candidate out. Catches both authoring mistakes (a bare
/// `Predicate::new()` with no fields and `always_match=false` —
/// matches nothing) and structurally-impossible combos (e.g.
/// `target_scope: SectionsOnly` paired with
/// `(Flag(SectionRoot), Equals(true))`, where the gate matches
/// "flag clear" but every candidate has the flag set). Either
/// produces a silent no-op without this check; the warn surfaces
/// the issue at the dispatch site so the author can fix it
/// rather than chase a missing visual change.
fn warn_if_predicate_filtered_everything(
    mutation_id: &str,
    has_predicate: bool,
    seen: usize,
    passed: usize,
) {
    if has_predicate && seen > 0 && passed == 0 {
        log::warn!(
            "mutation '{}': top-level predicate filtered every candidate ({} elements seen, 0 passed); \
             the apply path completed but no element was mutated",
            mutation_id,
            seen
        );
    }
}

pub(super) fn region_to_text_run(region: &ColorFontRegion, prior: Option<&TextRun>) -> TextRun {
    // Preserve `var(--name)` references when the prior run
    // shares the region's range and carries one. Without theme-
    // variables resolution at sync time we can't tell whether a
    // mutation deliberately recoloured the run away from the
    // variable; trusting the prior keeps the variable reference
    // verbatim across mutations that didn't touch the colour.
    // Same documented trade-off as the selective gate: a
    // deliberate `SetRegionColor` on a `var()`-bearing run is
    // silently swallowed here — the run keeps the variable.
    let prior_var_color: Option<&str> = prior.and_then(|p| {
        if p.color.starts_with("var(")
            && p.start == region.range.start
            && p.end == region.range.end
        {
            Some(p.color.as_str())
        } else {
            None
        }
    });
    let color = match (prior_var_color, region.color) {
        (Some(var_color), _) => var_color.to_string(),
        (None, Some(rgba)) => rgba_to_hex(rgba),
        (None, None) => prior
            .map(|p| p.color.clone())
            .unwrap_or_else(|| DEFAULT_TEXT_RUN_COLOR.to_string()),
    };
    let font = match region.font.and_then(family_name_of) {
        Some(name) => name.to_string(),
        None => prior.map(|p| p.font.clone()).unwrap_or_default(),
    };
    let bold = prior.is_some_and(|p| p.bold);
    let italic = prior.is_some_and(|p| p.italic);
    let underline = prior.is_some_and(|p| p.underline);
    let size_pt = prior.map(|p| p.size_pt).unwrap_or(DEFAULT_TEXT_RUN_SIZE_PT);
    let hyperlink = prior.and_then(|p| p.hyperlink.clone());
    TextRun {
        start: region.range.start,
        end: region.range.end,
        bold,
        italic,
        underline,
        font,
        size_pt,
        color,
        hyperlink,
    }
}

impl MindMapDocument {
    /// `true` when the registered mutation at `mutation_id` will
    /// dispatch through its Rust [`super::mutations::DynamicMutationHandler`]
    /// at apply time. Two conditions must hold:
    ///
    /// - A handler is registered for this id.
    /// - The mutation's source layer is [`MutationSource::App`] — i.e.
    ///   the definition the user sees actually is the one the handler
    ///   was written for. If the user / map / inline layer overrode
    ///   the id, their declarative shape wins and the bundled handler
    ///   is bypassed.
    ///
    /// This prevents a subtle hijack: a user mutation carrying the
    /// same id as a bundled handler (e.g. `"flower-layout"`) would
    /// otherwise win in the registry but still get executed by the
    /// bundled Rust algorithm, silently discarding the user's
    /// declared `mutator` and `target_scope`.
    pub fn will_dispatch_to_handler(&self, mutation_id: &str) -> bool {
        self.mutation_handlers.contains_key(mutation_id)
            && self.mutation_sources.get(mutation_id) == Some(&MutationSource::App)
    }

    /// Apply a custom mutation to the tree and optionally sync to the model.
    /// For Persistent mutations, snapshots affected nodes for undo and sets dirty flag.
    /// For Toggle mutations, tracks active state without model sync.
    ///
    /// The `tree` argument is only consulted on the declarative
    /// flat-apply path. When [`Self::will_dispatch_to_handler`]
    /// returns `true` for `custom.id` the handler mutates the model
    /// directly; callers that know ahead of time the handler will
    /// fire may pass `None` and skip the (expensive) tree build
    /// entirely. Passing `None` on the declarative path logs a
    /// warning and is otherwise a no-op (the mutation isn't applied).
    pub fn apply_custom_mutation(
        &mut self,
        custom: &CustomMutation,
        node_id: &str,
        mut tree: Option<&mut MindMapTree>,
    ) {
        // For toggle behavior, check if already active and reverse if so.
        if custom.behavior == MutationBehavior::Toggle {
            let key = (node_id.to_string(), custom.id.clone());
            if self.active_toggles.contains(&key) {
                // Second trigger: remove from active set. The tree
                // mutation from the first trigger is *not* inverted
                // in place — Mutations aren't guaranteed invertible.
                // The caller is expected to rebuild the tree from
                // the model next frame (the model is untouched
                // because Toggle skips the persistent-path model
                // sync). Console and event-loop callers both rebuild
                // scene state on every dispatch so this is the
                // conventional shape; trigger dispatchers that keep
                // a persistent tree across events must explicitly
                // call `build_tree()` after a toggle-off.
                self.active_toggles.remove(&key);
                self.dirty = true;
                return;
            }
            self.active_toggles.insert(key);
            // Toggle mutations apply to tree only (visual), no model sync.
            if let Some(tree) = tree.as_deref_mut() {
                self.apply_to_tree(custom, node_id, tree);
            } else {
                log::warn!(
                    "apply_custom_mutation: Toggle mutation '{}' called with None tree; \
                     visual toggle skipped. Pass Some(&mut tree) for Toggle mutations.",
                    custom.id
                );
            }
            return;
        }

        // Authoring-mistake guard: if the mutator AST walks deeper
        // than `target_scope` snapshots, undo will silently miss
        // the deeper edits. Log a `warn!` — still applies the
        // mutation (the author may have their own reasons, or the
        // warning may surface a bug worth fixing) but flags the
        // scope mismatch so it doesn't pass unnoticed.
        if let Some(mutator) = custom.mutator.as_ref() {
            let reach = mutator_reach(mutator);
            if !custom.target_scope.covers_reach(reach) {
                log::warn!(
                    "mutation '{}': mutator reach is {:?} but target_scope is \
                     {:?}; undo will not capture edits beyond the declared scope",
                    custom.id,
                    reach,
                    custom.target_scope
                );
            }
        }

        // Persistent: snapshot, apply, sync/push undo.
        let affected_ids = self.collect_affected_node_ids(node_id, &custom.target_scope);
        let snapshots: Vec<(String, MindNode)> = affected_ids
            .iter()
            .filter_map(|id| self.mindmap.nodes.get(id).map(|n| (id.clone(), n.clone())))
            .collect();

        // Handler dispatch: only fires when the mutation at this id
        // actually came from the app bundle — a user / map / inline
        // override of the same id keeps the declarative path so the
        // user's mutator is honoured. See
        // [`Self::will_dispatch_to_handler`] for the rationale.
        if self.will_dispatch_to_handler(&custom.id) {
            if let Some(handler) = self.mutation_handlers.get(&custom.id).copied() {
                handler(self, node_id);
            }
        } else if let Some(tree) = tree.as_deref_mut() {
            self.apply_to_tree(custom, node_id, tree);
            for id in &affected_ids {
                self.sync_node_from_tree(id, tree);
            }
        } else {
            log::warn!(
                "apply_custom_mutation: declarative mutation '{}' called with None tree; \
                 flat-apply skipped. Pass Some(&mut tree) when the mutation isn't \
                 handler-dispatched.",
                custom.id
            );
            // Fall through: document_actions still push undo below,
            // but no tree/model changes occurred.
            return;
        }

        if !snapshots.is_empty() {
            self.undo_stack.push(UndoAction::CustomMutation {
                node_snapshots: snapshots,
            });
            self.dirty = true;
        }
    }

    /// Apply any document-level actions carried by a custom mutation. These
    /// operate on `self.mindmap.canvas` rather than any tree node, so they
    /// run independently of `apply_custom_mutation`'s tree walk. When any
    /// action would actually change state, a `CanvasSnapshot` undo entry is
    /// pushed capturing the pre-action canvas, and the document is marked
    /// dirty. Returns true if the canvas was modified.
    pub fn apply_document_actions(&mut self, custom: &CustomMutation) -> bool {
        if custom.document_actions.is_empty() {
            return false;
        }
        let snapshot = self.mindmap.canvas.clone();
        let mut changed = false;
        for action in &custom.document_actions {
            match action {
                DocumentAction::SetThemeVariant(name) => {
                    if let Some(preset) = self.mindmap.canvas.theme_variants.get(name) {
                        let new_vars = preset.clone();
                        if new_vars != self.mindmap.canvas.theme_variables {
                            self.mindmap.canvas.theme_variables = new_vars;
                            changed = true;
                        }
                    }
                    // Unknown variant: silently ignored (graceful).
                }
                DocumentAction::SetThemeVariables(map) => {
                    for (k, v) in map {
                        let existing = self.mindmap.canvas.theme_variables.get(k);
                        if existing.map(|s| s != v).unwrap_or(true) {
                            self.mindmap.canvas.theme_variables.insert(k.clone(), v.clone());
                            changed = true;
                        }
                    }
                }
                // `DocumentAction` is `#[non_exhaustive]` so future
                // variants can be added without breaking dependents.
                // Any new variant that performs file I/O, network
                // access, or arbitrary content load MUST be gated
                // at the macro-dispatcher site — see
                // `MacroSource::allows_console_line` and
                // `lib/baumhard/src/mindmap/custom_mutation/mod.rs`
                // doc-comment on `DocumentAction`.
                _ => {
                    log::warn!("apply_document_actions: unknown DocumentAction variant; skipping");
                }
            }
        }
        if changed {
            self.undo_stack
                .push(UndoAction::CanvasSnapshot { canvas: snapshot });
            self.dirty = true;
        }
        changed
    }

    /// Apply a custom mutation's payload to the Baumhard tree, iterating
    /// every affected model node and applying the flat `Vec<Mutation>`
    /// extracted from the MutatorNode to each target element. Mutations
    /// without a `mutator` (document-actions-only) are no-ops here —
    /// [`Self::apply_document_actions`] handles their canvas effects
    /// separately. Mutations whose MutatorNode can't be reduced to a
    /// flat list (runtime-hole-bearing, size-aware) are skipped at this
    /// layer; a later session wires the richer `mutator_builder::build`
    /// path for those.
    ///
    /// **Section fan-out.** Post-section the per-node element shape is
    /// "chrome-only container + N section-areas + N section-models". A
    /// pre-section custom mutation that mutated the per-node area (text,
    /// runs, scale) used to land on the one-area-per-node entry; today
    /// it has to fan out across every section-area, otherwise the
    /// mutation lands on a no-glyph container with no visible effect.
    /// Position-affecting mutations apply to *both* container and
    /// sections so the whole node moves in lockstep (sections store
    /// absolute canvas positions in the tree).
    fn apply_to_tree(&self, custom: &CustomMutation, node_id: &str, tree: &mut MindMapTree) {
        use baumhard::core::primitives::{Flag, Flaggable};

        let Some(mutator) = custom.mutator.as_ref() else { return };
        let Some(mutations) = flat_mutations(mutator) else {
            // Non-flat mutator AST (e.g. `scope::descendants` —
            // `Instruction`-rooted, no Macro at the root). The
            // current flat-apply path can't extract a mutation
            // list from these shapes; pre-fix this returned
            // silently and the entire apply was a no-op with no
            // diagnostic. The richer `mutator_builder` walker
            // path is the home for these (size-aware /
            // predicate-filtered descendant iteration); until
            // it's wired into `apply_custom_mutation`, warn so
            // authors notice the silent drop instead of chasing
            // a missing visual change.
            log::warn!(
                "mutation '{}': mutator AST is non-flat (root is not `Macro` with literal list); \
                 the flat-apply path can't evaluate this shape — apply skipped. \
                 Use `scope::self_only` / `scope::self_and_descendants` for now, \
                 or wait for the walker-based `apply_custom_mutation` path.",
                custom.id
            );
            return;
        };
        // Top-level predicate gate (item E3). When `Some`, every
        // candidate element must satisfy `predicate.test(element)`
        // before mutations land on it. Authors typically reach for
        // this to add an element-level filter on top of the
        // structural `target_scope` — e.g.
        // `Predicate { fields: [(Flag(SectionRoot), Equals(false))], … }`
        // matches every element with `SectionRoot` set (i.e.
        // sections), so paired with `target_scope: SelfOnly` it
        // lands the mutation on sections only and skips the
        // chrome-only container. The inverse
        // `(Flag(SectionRoot), Equals(true))` matches the
        // container + sibling mind-nodes (anything with
        // `SectionRoot` *clear*).
        //
        // **§9 footgun guard.** A bare `Predicate::new()` (`fields=[]`,
        // `always_match=false`) matches nothing; pairing it as
        // `predicate: Some(Predicate::new())` silently filters every
        // candidate out, the apply path runs but no element changes.
        // Warn so authors notice the typo at apply time. Same warn
        // covers the "predicate filtered everything" case below
        // (e.g. `SectionsOnly` + `(Flag(SectionRoot), Equals(true))`
        // — structurally impossible because every `SectionsOnly`
        // candidate has `SectionRoot` set).
        let predicate_gate = custom.predicate.as_ref();
        if let Some(p) = predicate_gate {
            if p.fields.is_empty() && !p.always_match {
                log::warn!(
                    "mutation '{}': predicate has no fields and `always_match` is false; \
                     every candidate element will be filtered out (apply runs but produces no change)",
                    custom.id
                );
            }
        }
        let passes = |element: &baumhard::gfx_structs::element::GfxElement| -> bool {
            predicate_gate.is_none_or(|p| p.test(element))
        };
        let mut candidates_seen: usize = 0;
        let mut candidates_passed: usize = 0;

        // `SectionsOnly` (item E4) bypasses the container fan-out:
        // mutations land on every section-area directly. Channel
        // collisions with sibling mind-nodes don't reach in here
        // because we walk the section_map by `(triggering_node_id,
        // section_idx)` tuples — child mind-nodes that share a
        // channel with a section live elsewhere in the arena.
        if custom.target_scope == TargetScope::SectionsOnly {
            let targets = self.collect_affected_section_targets(node_id);
            for (mind_id, section_idx) in &targets {
                let Some(section_arena_id) = tree.section_arena_id(mind_id, *section_idx) else {
                    continue;
                };
                if let Some(node) = tree.tree.arena.get_mut(section_arena_id) {
                    candidates_seen += 1;
                    if passes(node.get()) {
                        candidates_passed += 1;
                        apply_mutations_to_element(&mutations, node.get_mut());
                    }
                }
            }
            warn_if_predicate_filtered_everything(
                &custom.id,
                predicate_gate.is_some(),
                candidates_seen,
                candidates_passed,
            );
            // Invalidate the tree's `subtree_aabb` and other geometry
            // caches — `apply_mutations_to_element` mutated arena
            // element fields directly (bypassing
            // `MutatorTree::apply_to`'s wrapper that owns the
            // invalidation). Pre-fix the dirty flag stayed false
            // so a downstream `ensure_subtree_aabbs()` call (e.g.
            // the editor click-outside-commit's overflow-aware
            // `point_in_node_aabb`) was a no-op against a stale
            // cache. Same shape as `MutatorTree::apply_to`'s
            // invalidation (`lib/baumhard/src/gfx_structs/tree.rs`).
            tree.tree.invalidate_caches();
            return;
        }

        let affected = self.collect_affected_node_ids(node_id, &custom.target_scope);
        for id in &affected {
            let Some(container_arena_id) = tree.arena_id_for(id.as_str()) else {
                continue;
            };
            // Apply to the container (carries position; text/regions
            // are empty so text-affecting mutations are visually no-op
            // here, but `area.scale` and position deltas land cleanly).
            // Container is `SectionRoot`-clear, so a
            // `(Flag(SectionRoot), Equals(false))` predicate (matches
            // when the flag IS set) filters the container out — only
            // sections survive, which is the documented "sections
            // only" idiom.
            if let Some(node) = tree.tree.arena.get_mut(container_arena_id) {
                candidates_seen += 1;
                if passes(node.get()) {
                    candidates_passed += 1;
                    apply_mutations_to_element(&mutations, node.get_mut());
                }
            }
            // Fan out across every immediate `Flag::SectionRoot` child
            // of the container — that's where `text`, `text_runs`
            // (regions), and per-section `scale` actually live in the
            // post-section tree shape. Without this fan-out every
            // text/font/region custom mutation silently no-ops on the
            // empty container. Mirrors `apply_tree_highlights`'s
            // walk so mutations and highlights both reach sections by
            // the same primitive.
            //
            // §B7 borrow-split note: collecting into a
            // `Vec<indextree::NodeId>` here is the smallest correct
            // shape. `container_arena_id.children(&tree.tree.arena)`
            // borrows the arena immutably; the loop body needs
            // `&mut tree.tree.arena.get_mut(...)` to apply the
            // mutation. Holding the iterator across the mutable
            // borrow won't compile, so the fix is to materialise the
            // section ids first and drop the immutable borrow
            // before the mutation pass starts. The vec is `O(sections
            // per node)` — bounded by user authoring (typically 1–4
            // per node), per affected-node-per-call, so allocation
            // cost is negligible relative to the mutation walker.
            let section_arena_ids: Vec<indextree::NodeId> = container_arena_id
                .children(&tree.tree.arena)
                .filter(|cid| {
                    tree.tree
                        .arena
                        .get(*cid)
                        .map(|n| n.get().flag_is_set(Flag::SectionRoot))
                        .unwrap_or(false)
                })
                .collect();
            for sid in section_arena_ids {
                if let Some(node) = tree.tree.arena.get_mut(sid) {
                    candidates_seen += 1;
                    if passes(node.get()) {
                        candidates_passed += 1;
                        apply_mutations_to_element(&mutations, node.get_mut());
                    }
                }
            }
        }
        warn_if_predicate_filtered_everything(
            &custom.id,
            predicate_gate.is_some(),
            candidates_seen,
            candidates_passed,
        );
        // Same invalidation as the `SectionsOnly` branch above:
        // `apply_mutations_to_element` writes arena element fields
        // directly without going through `MutatorTree::apply_to`,
        // so the tree's `subtree_aabbs_dirty` flag is unset until
        // we explicitly mark it. Downstream callers that read
        // `subtree_aabb()` (notably the overflow-aware
        // `point_in_node_aabb` from the editor click-outside-commit)
        // would otherwise hit a stale cache.
        tree.tree.invalidate_caches();
    }

    /// Collect the section targets affected by a `SectionsOnly`
    /// mutation: every section of `node_id`. Returns `(mind_id,
    /// section_idx)` tuples — the right shape for `tree
    /// .section_arena_id(...)` lookups in the apply-to-tree branch.
    /// Empty when `node_id` doesn't exist in the model or carries
    /// zero sections (loader-rejected at load time, but defensive
    /// for synthesized in-memory docs).
    fn collect_affected_section_targets(&self, node_id: &str) -> Vec<(String, usize)> {
        let Some(node) = self.mindmap.nodes.get(node_id) else {
            return Vec::new();
        };
        (0..node.sections.len())
            .map(|idx| (node_id.to_string(), idx))
            .collect()
    }

    /// Collect the IDs of all nodes affected by a mutation with the given scope.
    pub(super) fn collect_affected_node_ids(&self, node_id: &str, scope: &TargetScope) -> Vec<String> {
        match scope {
            // `SectionsOnly` lives on the triggering node — every
            // affected section belongs to that one node. The
            // node-level affected list is the snapshot window for
            // undo (whole-`MindNode` clone covers section state),
            // so a single entry is correct.
            TargetScope::SelfOnly | TargetScope::SectionsOnly => vec![node_id.to_string()],
            TargetScope::Children => self
                .mindmap
                .children_of(node_id)
                .iter()
                .map(|n| n.id.clone())
                .collect(),
            TargetScope::Descendants => self.mindmap.all_descendants(node_id),
            TargetScope::SelfAndDescendants => {
                let mut ids = vec![node_id.to_string()];
                ids.extend(self.mindmap.all_descendants(node_id));
                ids
            }
            TargetScope::Parent => self
                .mindmap
                .nodes
                .get(node_id)
                .and_then(|n| n.parent_id.clone())
                .into_iter()
                .collect(),
            TargetScope::Siblings => self
                .mindmap
                .nodes
                .get(node_id)
                .and_then(|n| n.parent_id.as_deref())
                .map(|pid| {
                    self.mindmap
                        .children_of(pid)
                        .iter()
                        .filter(|n| n.id != node_id)
                        .map(|n| n.id.clone())
                        .collect()
                })
                .unwrap_or_default(),
        }
    }

    /// Sync a node's position **and per-section text + runs** from
    /// the Baumhard tree back to the MindMap model. Used after
    /// persistent mutations so the model reflects every tree-side
    /// edit — without the section pass, custom mutations that
    /// touch text / colour / font would land on the live tree but
    /// the next `rebuild_all` would overwrite the tree from the
    /// stale model.
    ///
    /// **Selective section sync.** For each section we compare the
    /// tree-side `(text, regions)` against the model's current
    /// `(text, text_runs)` snapshot; sections whose tree state
    /// equals the model state byte-identically skip the rewrite.
    /// Without this gate every `apply_to_tree` call would round-
    /// trip *every* section's runs through `region_to_text_run`,
    /// which is lossy: the converter rebuilds runs from
    /// `area.regions` and would strip bold / italic / underline /
    /// size_pt / hyperlink fields from sections the mutation
    /// didn't touch (those fields don't survive the forward
    /// conversion). The selective gate ensures only sections that
    /// changed pay the lossy round-trip; untouched sections keep
    /// their original runs verbatim.
    fn sync_node_from_tree(&mut self, node_id: &str, tree: &MindMapTree) {
        let Some(tree_nid) = tree.arena_id_for(node_id) else {
            return;
        };
        let Some(element) = tree.tree.arena.get(tree_nid).map(|n| n.get()) else {
            return;
        };
        let Some(area) = element.glyph_area() else {
            return;
        };
        let new_pos = (area.position.x.0 as f64, area.position.y.0 as f64);

        // Gather every section's tree-side `(text, regions, position,
        // size)` before we acquire `&mut` on the model. The arena
        // lookup needs `&tree`; the model write needs `&mut self`;
        // sequencing them avoids overlapping borrows on
        // `self.mindmap`. Capturing position + size lets us write
        // `section.offset` / `section.size` back from the tree, so a
        // `SectionsOnly` mutation that translates / resizes a
        // section persists past the next `rebuild_all`.
        let section_count = self
            .mindmap
            .nodes
            .get(node_id)
            .map(|n| n.sections.len())
            .unwrap_or(0);
        struct SectionSnapshot {
            text: String,
            regions: Vec<ColorFontRegion>,
            tree_position: (f32, f32),
            tree_size: (f32, f32),
        }
        let mut section_snapshots: Vec<Option<SectionSnapshot>> = Vec::with_capacity(section_count);
        for idx in 0..section_count {
            let snapshot = tree
                .section_arena_id(node_id, idx)
                .and_then(|sid| tree.tree.arena.get(sid))
                .and_then(|n| n.get().glyph_area())
                .map(|sec_area| SectionSnapshot {
                    text: sec_area.text.clone(),
                    regions: sec_area
                        .regions
                        .all_regions()
                        .into_iter()
                        .copied()
                        .collect::<Vec<ColorFontRegion>>(),
                    tree_position: (sec_area.position.x.0, sec_area.position.y.0),
                    tree_size: (sec_area.render_bounds.x.0, sec_area.render_bounds.y.0),
                });
            section_snapshots.push(snapshot);
        }

        let Some(model_node) = self.mindmap.nodes.get_mut(node_id) else {
            return;
        };
        model_node.position.x = new_pos.0;
        model_node.position.y = new_pos.1;
        let node_pos_x = new_pos.0 as f32;
        let node_pos_y = new_pos.1 as f32;
        let node_size_x = model_node.size.width as f32;
        let node_size_y = model_node.size.height as f32;

        for (idx, snapshot) in section_snapshots.into_iter().enumerate() {
            let Some(snapshot) = snapshot else {
                continue;
            };
            let Some(section) = model_node.sections.get_mut(idx) else {
                continue;
            };

            // Write `section.offset` back from the tree's section-
            // area position so a `SectionsOnly` translate mutation
            // persists. The forward path computes
            // `section_area.position = node.pos + section.offset`,
            // so the inverse is `section.offset = section_area.position
            // - node.pos`. Section-area position is canvas-space
            // float; model `Position` is canvas-space f64 — same
            // unit, just wider. Without this, a `Translate` /
            // `MoveTo` on a section-area lands on the live tree
            // and reverts on the next `rebuild_all`.
            let new_offset_x = (snapshot.tree_position.0 - node_pos_x) as f64;
            let new_offset_y = (snapshot.tree_position.1 - node_pos_y) as f64;
            if section.offset.x != new_offset_x || section.offset.y != new_offset_y {
                section.offset.x = new_offset_x;
                section.offset.y = new_offset_y;
            }
            // Write `section.size` back when the model carries an
            // explicit size. `None` size means "fill the parent
            // node", which the tree resolves to the node's full
            // render_bounds — *don't* eagerly materialise it as
            // `Some(node.size)`, that would surprise authors who
            // chose the inheriting shape. Materialise only when the
            // tree's render_bounds diverges from the node's full
            // size (i.e. the mutation explicitly resized the
            // section, or the model already carried a Some).
            let tree_size_diverges =
                (snapshot.tree_size.0 - node_size_x).abs() > f32::EPSILON
                    || (snapshot.tree_size.1 - node_size_y).abs() > f32::EPSILON;
            if section.size.is_some() || tree_size_diverges {
                section.size = Some(baumhard::mindmap::model::Size {
                    width: snapshot.tree_size.0 as f64,
                    height: snapshot.tree_size.1 as f64,
                });
            }

            // Selective gate: tree-side state matches the model
            // snapshot? Skip the text/regions round-trip so
            // untouched sections keep their bold / italic /
            // underline / size_pt / hyperlink. Range / colour /
            // font are everything the forward conversion
            // preserves.
            //
            // **Range-keyed comparison.** Tree-side
            // `all_regions()` returns runs in `Range` order
            // (`BTreeSet`-keyed); model `text_runs: Vec<TextRun>`
            // is load-order. A positional `zip` would mis-align
            // any model whose runs were authored out of range
            // order, trip a false mismatch, and run the lossy
            // round-trip — silently stripping the prior styling
            // from sections the mutation didn't touch. Build a
            // map keyed by `(start, end)` and compare each
            // tree-side region against the same-range prior.
            let model_runs_by_range: rustc_hash::FxHashMap<(usize, usize), &TextRun> = section
                .text_runs
                .iter()
                .map(|r| ((r.start, r.end), r))
                .collect();
            let model_regions_match = model_runs_by_range.len() == snapshot.regions.len()
                && snapshot.regions.iter().all(|region| {
                    let key = (region.range.start, region.range.end);
                    let Some(run) = model_runs_by_range.get(&key) else {
                        return false;
                    };
                    // Colour comparison is **case-insensitive on
                    // hex**: `rgba_to_hex` always emits lowercase,
                    // but model-side `run.color` may have been
                    // hand-authored as `#FFFFFF` or mixed case. A
                    // byte-equal `==` would always-mismatch those
                    // and trigger the lossy round-trip on every
                    // apply_to_tree call. `var(--name)` references
                    // never structurally match a tree-side hex —
                    // treat them as "different" so the documented
                    // round-trip collapse runs (replacing the
                    // variable with its resolved hex; see
                    // `region_to_text_run`).
                    let region_color_hex = region.color.map(rgba_to_hex);
                    let model_color_hex = if run.color.starts_with('#') {
                        Some(run.color.clone())
                    } else {
                        None
                    };
                    let model_is_var = run.color.starts_with("var(");
                    let colors_equal = match (region_color_hex.as_deref(), model_color_hex.as_deref()) {
                        (Some(a), Some(b)) => str::eq_ignore_ascii_case(a, b),
                        (None, None) => true,
                        // `(Some(hex), None)` with the model carrying
                        // a `var(--…)` reference: presume the
                        // variable resolves to the tree-side hex
                        // and treat as equal. Without theme-variables
                        // resolution at sync time we can't compare
                        // structurally; trusting the model preserves
                        // the `var()` reference across mutations
                        // that didn't touch this section's regions.
                        // **Documented limit**: a custom mutation
                        // that *deliberately* recolours a
                        // `var()`-bearing run is silently swallowed
                        // here — the run keeps the variable, the
                        // explicit recolour is lost. Authors who
                        // need the recolour should switch the run
                        // to a hex literal first or use the
                        // `set_section_text_color` document setter.
                        (Some(_), None) if model_is_var => true,
                        _ => false,
                    };
                    if !colors_equal {
                        return false;
                    }
                    // Forward path: model `font: String` → tree
                    // `region.font: Option<AppFont>`; the reverse
                    // path uses `family_name_of`. Empty model
                    // font and `None` AppFont collide on
                    // "no pin", so equate them here.
                    let region_font_name = region.font.and_then(family_name_of);
                    let model_font_name: Option<&str> = if run.font.is_empty() {
                        None
                    } else {
                        Some(run.font.as_str())
                    };
                    region_font_name == model_font_name
                });
            if section.text == snapshot.text && model_regions_match {
                continue;
            }

            // Build the new run list by merging each tree-side
            // region with the prior run sharing the **same range,
            // or the dominant overlapping range** when the
            // mutation resized / split / shifted the run boundary.
            // A range-strict lookup loses every prior styling
            // (bold / italic / underline / size_pt / hyperlink)
            // on `ChangeRegionRange`-style mutations because no
            // prior matches the new range exactly; the overlap
            // fallback inherits from the prior whose intersection
            // is largest, preserving authored styling across
            // range edits.
            let prior_runs: Vec<&TextRun> = section.text_runs.iter().collect();
            let new_runs: Vec<TextRun> = snapshot
                .regions
                .iter()
                .map(|region| {
                    let prior = exact_or_dominant_overlap(&prior_runs, region.range.start, region.range.end);
                    region_to_text_run(region, prior)
                })
                .collect();

            section.text = snapshot.text;
            section.text_runs = new_runs;
            // Ensure no run extends past the new grapheme count —
            // `clamp_runs_to_text` is already idempotent on
            // already-clean run lists.
            clamp_runs_to_text(section);
        }
    }
}

/// Find the prior `TextRun` for a tree-side region by range.
/// Prefers exact `(start, end)` match; falls back to the prior
/// run whose intersection with `[start, end)` is largest. Used by
/// `sync_node_from_tree`'s reverse converter so a custom mutation
/// that resizes / splits a region (e.g. `ChangeRegionRange`)
/// still inherits authored styling instead of zeroing every
/// field. Ties broken in favour of earlier `start`.
///
/// Returns `None` only when no prior run overlaps the new range
/// at all (e.g. a fresh region inserted by the mutation).
pub(super) fn exact_or_dominant_overlap<'a>(
    priors: &[&'a TextRun],
    start: usize,
    end: usize,
) -> Option<&'a TextRun> {
    if let Some(exact) = priors.iter().find(|r| r.start == start && r.end == end) {
        return Some(exact);
    }
    let mut best: Option<(&'a TextRun, usize)> = None;
    for run in priors.iter() {
        if run.end <= start || run.start >= end {
            continue;
        }
        let lo = run.start.max(start);
        let hi = run.end.min(end);
        if hi <= lo {
            continue;
        }
        let overlap = hi - lo;
        match best {
            None => best = Some((run, overlap)),
            Some((_, prev)) if overlap > prev => best = Some((run, overlap)),
            _ => {}
        }
    }
    best.map(|(r, _)| r)
}

#[cfg(test)]
mod region_converter_tests {
    use super::{exact_or_dominant_overlap, region_to_text_run, DEFAULT_TEXT_RUN_COLOR, DEFAULT_TEXT_RUN_SIZE_PT};
    use baumhard::core::primitives::{ColorFontRegion, Range};
    use baumhard::mindmap::model::TextRun;

    fn run(start: usize, end: usize, color: &str, font: &str) -> TextRun {
        TextRun {
            start,
            end,
            bold: false,
            italic: false,
            underline: false,
            font: font.into(),
            size_pt: 14,
            color: color.into(),
            hyperlink: None,
        }
    }

    fn styled_run(start: usize, end: usize) -> TextRun {
        TextRun {
            start,
            end,
            bold: true,
            italic: true,
            underline: true,
            font: "LiberationSans".into(),
            size_pt: 21,
            color: "#aabbcc".into(),
            hyperlink: Some("https://example.org".into()),
        }
    }

    /// `region_to_text_run` with `prior=Some` and a colour /
    /// font on the region: the merged TextRun gets the region's
    /// colour and font; bold / italic / underline / size_pt /
    /// hyperlink come from `prior`.
    #[test]
    fn region_to_text_run_merges_with_prior() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior = styled_run(0, 5);
        let out = region_to_text_run(&region, Some(&prior));
        assert_eq!(out.start, 0);
        assert_eq!(out.end, 5);
        assert_eq!(out.color, "#ff0000");
        assert!(out.bold);
        assert!(out.italic);
        assert!(out.underline);
        assert_eq!(out.size_pt, 21);
        assert_eq!(out.hyperlink.as_deref(), Some("https://example.org"));
    }

    /// `region_to_text_run` with `prior=None` falls back to
    /// defaults: `bold/italic/underline = false`, `size_pt = 14`,
    /// `hyperlink = None`, `font = ""`, `color = "#ffffff"`.
    #[test]
    fn region_to_text_run_falls_back_to_defaults_without_prior() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, None);
        let out = region_to_text_run(&region, None);
        assert!(!out.bold);
        assert!(!out.italic);
        assert!(!out.underline);
        assert_eq!(out.size_pt, DEFAULT_TEXT_RUN_SIZE_PT);
        assert_eq!(out.hyperlink, None);
        assert_eq!(out.font, "");
        assert_eq!(out.color, DEFAULT_TEXT_RUN_COLOR);
    }

    /// `region_to_text_run` with `prior=None` and a colour set
    /// on the region: colour comes from `rgba_to_hex(region.color)`,
    /// other defaults stand.
    #[test]
    fn region_to_text_run_uses_region_color_without_prior() {
        let region = ColorFontRegion::new(Range::new(0, 3), None, Some([0.0, 1.0, 0.0, 1.0]));
        let out = region_to_text_run(&region, None);
        assert_eq!(out.color, "#00ff00");
    }

    /// `region_to_text_run` preserves a `var(--name)` reference
    /// on the prior run when the region's range matches the
    /// prior's range exactly. Without theme-variables resolution
    /// at sync time, we can't structurally compare a var() to a
    /// resolved hex; trusting the prior keeps the variable
    /// reference verbatim across mutations that didn't touch the
    /// colour. **Documented limit**: a custom mutation that
    /// deliberately recolours a var()-bearing run is silently
    /// swallowed — the run keeps the variable. Authors who need
    /// the explicit recolour should switch the run to a hex
    /// literal first or use the `set_section_text_color` document
    /// setter (which bypasses the round-trip).
    #[test]
    fn region_to_text_run_preserves_var_color_when_range_matches() {
        let region = ColorFontRegion::new(Range::new(0, 5), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior_with_var = TextRun {
            color: "var(--accent)".into(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_with_var));
        assert_eq!(
            out.color, "var(--accent)",
            "var() reference is preserved when prior range matches"
        );
    }

    /// `region_to_text_run` documented limit: a `var(--name)`
    /// reference on a *partially-overlapping* prior run does NOT
    /// transfer — the region's range diverges from the prior's,
    /// so the heuristic that preserves var() doesn't fire.
    /// Range-mutating mutations on var()-bearing runs collapse
    /// to the resolved hex.
    #[test]
    fn region_to_text_run_loses_var_color_on_range_change() {
        // Region [0, 3) — prior is [0, 5), ranges differ.
        let region = ColorFontRegion::new(Range::new(0, 3), None, Some([1.0, 0.0, 0.0, 1.0]));
        let prior_with_var = TextRun {
            color: "var(--accent)".into(),
            ..styled_run(0, 5)
        };
        let out = region_to_text_run(&region, Some(&prior_with_var));
        assert_eq!(
            out.color, "#ff0000",
            "var() does NOT transfer across range changes"
        );
    }

    /// `exact_or_dominant_overlap`: exact-range match wins over
    /// any overlap. Pins the prefer-exact contract so range-
    /// stable mutations (every region keeps its range) inherit
    /// from the same prior every time.
    #[test]
    fn exact_overlap_match_wins_over_partial() {
        let r1 = run(0, 5, "#aabbcc", "");
        let r2 = run(2, 7, "#ddeeff", "");
        let priors = vec![&r1, &r2];
        let hit = exact_or_dominant_overlap(&priors, 0, 5).expect("exact match");
        assert_eq!(hit.color, "#aabbcc");
    }

    /// `exact_or_dominant_overlap`: when no exact match exists,
    /// the prior whose intersection with the new range is
    /// **largest** wins. Pins the dominant-overlap fallback that
    /// preserves authored styling across `ChangeRegionRange` /
    /// resize mutations.
    #[test]
    fn dominant_overlap_wins_when_no_exact_match() {
        let small = run(0, 1, "#000000", "");
        let large = run(0, 4, "#ffffff", "");
        let priors = vec![&small, &large];
        let hit = exact_or_dominant_overlap(&priors, 0, 5).expect("partial overlap");
        assert_eq!(hit.color, "#ffffff", "wider overlap wins");
    }

    /// `exact_or_dominant_overlap`: no overlap with any prior
    /// returns `None` (e.g. a fresh region inserted by the
    /// mutation past every existing run).
    #[test]
    fn no_overlap_returns_none() {
        let r1 = run(0, 5, "#aabbcc", "");
        let priors = vec![&r1];
        assert!(exact_or_dominant_overlap(&priors, 10, 15).is_none());
    }
}
