# Findings: baumhard core (lib.rs, core/, gfx_structs/, mutator_builder/) — Part 1

## Architecture assessment
Coherent mutation-first substrate: Tree/MutatorTree + walk_tree_from + Applicable form one clear dispatch spine; AABB/BVH caching discipline is real and tested; newer surfaces (shape.rs, zoom_visibility.rs, camera.rs, scene.rs, MapChildren/SpatialDescend, mutator-builder DSL) are exemplary §B9-grade. Structural risk concentrates in: (1) walker has two child-alignment implementations with different correctness guarantees (align_child_walks sorts; RepeatWhile path doesn't); (2) area-side and model-side mutation surfaces are hand-maintained structural twins with drift already present; (3) ring of documented-but-phantom machinery (region-index fields, crossbeam sender, core/animation.rs, Anchor system, event-subscriber shape) — docs assert invariants code doesn't implement.

### B1. Area-side and model-side mutation surfaces are hand-maintained structural twins
P1 | duplication | high
Files: gfx_structs/area_mutators.rs:29-140,204-258,297-403; area_fields.rs:104-130,314-334; model/mutator.rs:27-43,72-89,99-172,188-207,254-274; area.rs:278-351; model/glyph_model.rs:117-131; element.rs:50-62; mutator.rs:79-91,176-190
Five parallel pairs duplicated by hand: discriminant enums + variant() maps (GlyphAreaFieldType, GlyphModelFieldType, GlyphAreaCommandType, GlyphModelCommandType, GfxElementType, MutatorType, MutationType — all exactly what strum EnumDiscriminants generates; strum already a dep); Delta wrappers (DeltaGlyphArea/DeltaGlyphModel: byte-similar fields FxHashMap, identical new() loops area_mutators.rs:251-258 vs model/mutator.rs:107-113, identical operation_variant() with Noop default, per-field accessor ladders); same_type() duplicated verbatim 3× (area_fields.rs:332, model/mutator.rs:86, model/mutator.rs:272); apply_operation bodies share shape. Drift already happened: B2, B13, accessor asymmetry (area borrows text_ref()->Option<&str>, model clones glyph_matrix()->Option<GlyphMatrix>).
Fix: (a) #[derive(EnumDiscriminants)] replaces 4 hand enums+variant(); (b) generic Delta<FT,F> or macro for new()/operation_variant(); keep field enums separate (they're the seam §B4); (c) model accessors borrow. Effort M.

### B2. DeltaGlyphModel carrying GlyphLine/GlyphLines silently does nothing
P1 | correctness | high
Files: model/glyph_model.rs:117-131; model/mutator.rs:50-63,134-152
apply_operation only calls delta.position()/layer()/glyph_matrix(); delta.glyph_line() and glyph_lines() never called from any apply path (zero callers repo-wide). Serde-authorable Mutation::ModelDelta with GlyphModelField::GlyphLine(3,line) matches channel and mutates NOTHING silently, though variant docs promise "Replace one line at line_num". Tests only exercise as hand-applied expected values via ensure_line, never through apply_to.
Fix: add two branches (operation.apply(matrix.ensure_line(n), line)) + do_* tests through apply_to — or delete variants per §10. Effort S.

### B3. RepeatWhile sibling alignment order-dependent + drops mutator-without-target matches
P1 | correctness | high (mechanism), med (production impact today)
Files: tree_walker.rs:181-241 (compare_apply_repeat_while), 32-68 (DEFAULT_TERMINATOR), 272-334 (align_child_walks = fixed comparison point)
align_child_walks hardened to sort both sibling rows by channel; RepeatWhile path got no fix and its doc claims the opposite invariant. Worse: advance logic loses matches even for SORTED input — when m_chan < t_chan it advances BOTH cursors. Targets [2,3] vs mutators [1,2]: pair(1,2) no match → advance both → (2,3) no match → return; channel-2 mutator never reaches channel-2 target (correct sorted merge advances only mutator). DEFAULT_TERMINATOR (50-67) has same ascending-order assumption (else if channel > t_chan break). All RepeatWhile tests use ascending gap-free channels so neither defect caught.
Fix: reuse collect_sorted_children in compare_apply_repeat_while + DEFAULT_TERMINATOR scan; standard merge advance rule (m<t advance mutator; m>t advance target; m==t apply+advance target). Tests: non-ascending channels under RepeatWhile; mutator channel with no target counterpart followed by matching pair. Effort M.

### B4. GlyphAreaCommand::ChangeRegionRange panics on missing region — reachable from user JSON
P1 | error-handling | high
Files: area.rs:417-424; area_mutators.rs:175-177
change_region_range: self.regions.get(*current_range).expect("No region found"). Dispatched unconditionally from Applicable<GlyphArea> for GlyphAreaCommand. GlyphAreaCommand is Serialize/Deserialize, rides Mutation::AreaCommand in user/map/inline custom-mutation JSON flat Macro lists (custom/mod.rs:292-316 applies each Mutation to live elements). Stale range panics editor mid-edit. §9. submit_region (primitives.rs:182-194) already converted panic→warn-and-drop for exactly this reason.
Fix: let Some(current) = ... else { warn!; return; }; update # Panics doc; do_* regression test. Effort S.

### B5. Predicate comparator semantics internally inconsistent; one negation flag dropped
P1 | correctness | high
Files: predicate.rs:107-122 (documented convention), 205-210 (Channel), 222-227 (Id), 440-447 (Layer/GlyphLines GreaterThan), 449-471 (LessThan arms)
compare_f32 documents element-side-left convention. Id follows (element.unique_id() > *id). Channel inverts (*channel > element.channel()). Model Layer inverts (*layer > target_model.layer); GlyphLines compares reference-left. Layer under LessThan DROPS negation flag: Layer(layer) => *layer < target_model.layer (no != *negation) — LessThan(true) (documented >=) evaluates as plain <. Exists on Channel falls into _ => false even for Exists(false) (documented "returns true unconditionally") while Id handles it. predicate_tests pins only compare_f32 + equality/flag paths.
Fix: normalize every arm to element-side-left (behavior change OK pre-V1 §10), restore negation on Layer/LessThan, Channel Exists arm, truth-table test (OVERLAPS_TEST lazy-static style) field × comparator × negation. Effort M.

### B6. CONVENTIONS §B6/CONCEPTS describe region-index maintenance pipeline that does not exist
P1 | ssot | high
Files: lib/baumhard/CONVENTIONS.md §B6; tree.rs:97-106 (apply_to), 108-138 (fields), 179 (_scene_index_sender); util/regions.rs:351-363 (RegionElementKeyPair doc); CONCEPTS §2 RegionParams ("currently the index is scene-wide")
apply_to clears two AABB cells and walks — no region-index write anywhere. region_params/region_index private #[allow(dead_code)], written at construction, never read. Tree::new's Sender<RegionElementKeyPair> "not currently wired" yet forces every caller to build a crossbeam channel; only caller = one test (tree_tests.rs:1036-1038). Production RegionIndexer consumers: none (app builds trees via new_non_indexed*, 3 sites). §B6 commands "Never mutate ColorFontRegions outside the mutator pipeline … index drifts silently" — invariant with no index behind it.
Fix: either wire minimally (apply_to updates region_index when present) or rewrite §B6+CONCEPTS+docs to "tested-but-unwired subsystem"; drop unused Sender param. Effort M.

### B7. Subtree-AABB dirty flag goes stale within/after walk mixing SpatialDescend with later mutations
P2 | correctness | high (mechanism), low-med (trigger today)
Files: tree.rs:97-106, 370-376; tree_walker.rs:576-584
apply_to sets dirty BEFORE walk; spatial_descend calls ensure_subtree_aabbs() mid-walk which clears flag; later element mutations in same walk can't re-set tree-level Cell. (a) second SpatialDescend in same mutator tree resolves against stale AABBs; (b) after apply_to returns flag stays false → subsequent descendant_at/component_at hit-tests use stale subtree AABBs until next apply_to.
Fix: move invalidation to AFTER walk in apply_to (walk…; invalidate_caches();). One-line + regression test (SpatialDescend-after-move). Effort S.

### B8. Mutator-builder panics on malformed AST shapes JSON can express
P2 | error-handling | high (paths), med (reachability)
Files: mutator_builder/build.rs:22-24 (Repeat-at-root panic!), 101 (expect SectionIndex outside Repeat), 135 (unreachable! Repeat-as-template), 196-199 (expect AreaDelta outside Repeat); context.rs:27-78 (unreachable!() defaults)
Five panic paths on serde-deserializable shapes. Three pinned as #[should_panic] tests — deliberate posture ("misuse is loud at runtime"). Today build() called only with app-bundled picker spec (include_str!, trusted; widgets/color_picker_widget.rs:146); custom-mutation dispatcher uses flat_mutations, skips build(). But widget spec header declares "first step toward user-authored widgets"; custom/mod.rs says walker path "is the home for these … until it's wired".
Fix: validate(node) -> Result pre-flight in mutator_builder (root not Repeat, no Repeat-as-template, SectionIndex/AreaDelta only inside Repeat, runtime labels declared); require for untrusted JSON; keep internal panics as programmer contract. Effort M.

### B9. iter_section_channels and append disagree on nested Repeats
P2 | correctness | med
Files: build.rs:41-71 vs 139-182
append expands Repeat template then recurses into node_children(template) — nested Repeat inside template's children expands correctly. iter_section_channels emits tuples but NEVER descends into template. Purpose is keeping initial-build channel set aligned with mutator path — for nested-Repeat specs the two walkers disagree (mutator targets channels initial build never saw).
Fix: recurse into node_children(template) per iteration, or reject nested Repeats in B8 validator. Test comparing both walkers. Effort S.
# Findings: baumhard core — Part 2 (B10-B20)

### B10. GlyphLine::perform_op ignore_initial_space: char index used as byte index, unguarded self.line[i]
P2 | correctness | high (defects), med (reachability — serde can set flag)
Files: model/line.rs:113-168; model/component.rs:157-163
(1) index_of_first_non_space_char returns char ordinal (chars().enumerate()); perform_op feeds it to String::split_off (BYTE index) → panic on non-char-boundary (U+3000 ideographic space 3 bytes); then also reused as grapheme offset (overriding_insert) — three index units in four lines, §B3 violation. (2) SubAssign/MulAssign arms index self.line[i] unguarded while dead AddAssign arm guards. rhs longer than self + ignore_initial_space → index panic. (3) second loop self.line.insert(i,…) can panic when begin_comp>0. No production constructor sets ignore_initial_space=true (tests only) but GlyphLine is Serialize/Deserialize — JSON ModelDelta GlyphMatrix payload can set it; matrix *Assign ops then run this path from mutation pipeline (§9).
Fix: route through grapheme_chad (split_off_graphemes after locating first non-ws grapheme); guard self.line.get(i) all arms; tests U+3000 + rhs-longer. Delete or implement GlyphLineOp::AddAssign/Noop (dead arms mislabeled as seam). Effort M.

### B11. ColorFontRegions::split_and_separate produces inverted/empty ranges; zero production callers
P2 | correctness | high
Files: core/primitives.rs:196-220; core/tests/primitives_tests.rs:32-59
Region [5,10) with range=[3,7): overlaps → left becomes [5,3) INVERTED (magnitude underflow later), right [7,14); correct = pure shift [9,14). Region equal to range leaves empty husk. Tests only cover regions starting before range.start. No callers outside tests/bench (live path = insert_regions_at/shift_regions_after).
Fix: restrict split branch to true straddlers (region.start < range.start && region.end > range.start), shift regions with start >= range.start wholesale, add tests — or delete per §10. Effort S.

### B12. GlyphArea::rotate doesn't rotate around pivot (no translate-back); dead but exported
P2 | correctness | high
Files: area.rs:471-478; contrast element.rs:418-423, model/glyph_model.rs:96-100
position = rotate(position - pivot) — missing + pivot. Teleports toward origin. Siblings use clockwise_rotation_around_pivot (degrees, clockwise); this takes radians counterclockwise (Vec2::from_angle). Zero callers, zero tests.
Fix: reimplement via clockwise_rotation_around_pivot for parity, or delete (§10). do_* test either way. Effort S.

### B13. ApplyOperation semantics field-dependent, silently lossy
P2 | api-design | high
Files: core/primitives.rs:442-483; area.rs:301-350; model/matrix.rs:46-96 ("wtf does it mean to multiply two glyphmatrices" at 63-66); model/line.rs:53-58; model/glyph_model.rs:124-126
Delete works on numeric fields via T::default() but on Text/ColorFontRegions falls through _ => {} (area.rs:316,327) silent no-op contradicting enum doc. Multiply on Text/Regions no-ops unlogged; on Position/Bounds does component-wise multiply (nonsense for absolute positions). Trait bound AddAssign+SubAssign+MulAssign+Default forces invented arithmetic: GlyphLine::add_assign performs Assign deliberately, so ApplyOperation::Add on matrix = per-line overwrite; MulAssign improvised. operation.apply(&mut self.layer, delta_layer) Subtract underflows usize (debug panic/release wrap) when delta > layer.
Fix: extend apply_overwrite_or_reset-style explicit handling (area side already has the pattern for Outline/Shape/ZoomVisibility); define Delete on Text (clear)/Regions (clear) or log-ignore; warn on unsupported ops; saturating_sub layer; per-field operation tables in docs (§B9). Consider dropping fake MulAssign/SubAssign impls on matrix/line/component. Effort M.

### B14. core/animation.rs dead module, unusable API shape, zero tests
P2 | dead-code | high
Files: core/animation.rs:23-26,33-42,61-85
Zero references outside file. App's animation driver (custom_mutation AnimationTiming) doesn't use it. Shapes can't work: AnimationDef<T: Mutable>.mutators: Vec<Box<T>> boxes the mutated VALUE type not mutators; AnimationMutator::update(instance) no self, consumes by value, returns nothing; Mutable = empty marker, zero implementers. No tests, no bench. CONCEPTS presents as "today's vocabulary for motion" — drift.
Fix: rewrite to usable shape when Followup lands (Vec<Box<dyn Mutator<T>>>, update(&mut …)), or delete and point CONCEPTS at custom_mutation/timing.rs (§10). Effort M.

### B15. Tree's phantom seams: never-read private fields, forced crossbeam channel, dead import, Arc/Rc mismatch
P2 | dead-code | high
Files: tree.rs:18,108-152,154-234,260-268; CONCEPTS §2 Tree
position/pending_mutations/region_params/region_index private #[allow(dead_code)], no accessor/setter/reader anywhere — not usable seams. CONCEPTS claims position "admits multi-viewport" and both "used narrowly today" — neither used at all; Scene's per-entry offset (scene.rs:76-79) is the live implementation of same idea (duplicate concept). pending_mutations uses Arc; Tree::new demands crossbeam Sender in a "no channels no threads" codebase (§3); region_params Arc vs region_index Rc mixed postures. Tree::import/import_arena zero callers + mutate arena WITHOUT invalidate_caches() — the one in-crate violation of the file's own invalidation discipline.
Fix: delete position/pending_mutations (Scene offset owns concept; §10) or expose real pub surface; remove _scene_index_sender param (B6); import: add invalidate + caller/test or delete; Arc→Rc. Effort M.

### B16. Event-subscriber surface fights single-threaded design: Send+Sync bounds, poisoned-lock expect on interactive path, per-event Vec clone
P2 | api-design | high
Files: tree.rs:36-40; element.rs:576-583; core/primitives.rs:594-595
EventSubscriber = Arc<Mutex<dyn FnMut + Send + Sync>>. App has zero references — seam is sanctioned; shape is not: (a) Send+Sync forces future plugin closures thread-safe in §3 single-threaded app, excluding Rc/RefCell captures; (b) accept_event does sub.lock().expect(…) — panic on interactive dispatch (§9) if prior subscriber panicked while locked; guaranteed self-deadlock if subscriber's reaction re-delivers event to itself (std Mutex not reentrant); (c) every delivery clones whole subscriber Vec (element.rs:578). Flag::MutationEvents doc reads as implemented behavior; no walker code checks flag.
Fix: Rc<RefCell<dyn FnMut>>; try_borrow/log-skip; scratch buffer or index iteration; reword MutationEvents doc "reserved". Effort M.

### B17. Bench coverage gaps: 7 test modules with §T2.2 headers not imported by test_bench.rs; doc bench-name drift
P2 | testing | high
Files: benches/test_bench.rs:3-23; gfx_structs/tests/{map_children,spatial_descend,bvh_descent,subtree_aabb,camera,predicate,element}_tests.rs; CONVENTIONS §B6; zoom_visibility.rs:18,82
Bench imports 12/19 scope test modules. Missing: map_children, spatial_descend, bvh_descent, subtree_aabb, camera, predicate, element — each header claims "every public body is benchmarkable". MapChildren/SpatialDescend are user-visible walker primitives (§B7); camera math per pointer event; predicate eval inside every RepeatWhile iteration. No stale imports found. Doc drift: §B6 names region_indexer_insert, region_params_calculate_pixel_from_region etc. — actual: region_indexer_insert_and_remove, region_params_pixel_to_region, region_params_region_to_pixel, region_rect_exhaustive_4x4_grid; zoom_visibility.rs cites zoom_visibility_contains bench that doesn't exist (real: ten zoom_visibility_* names).
Fix: add 7 imports + bench_function entries (mechanical); correct names in §B6 + zoom_visibility docs. Effort S.

### B18. Hot-path allocations: model delta clones per apply, component ops clone needlessly, scene allocates per pointer event/frame
P2 | performance | high (existence), med (measured)
Files: model/mutator.rs:116-123,134-152; model/component.rs:52-89; model/matrix.rs:46-96,161-206; scene.rs:231-234,257-286; tree_walker.rs:341-358
By frequency: Scene::ids_in_layer_order clones id Vec per call (per frame); component_at collects candidates Vec per hit test (per pointer event). DeltaGlyphModel::apply_to → glyph_matrix() deep-clones whole matrix EVERY apply even when operation Noop/Delete (§B7 names this fn hot by name). GlyphComponent::add_assign: self.text = self.text.clone() + &rhs.text (clone receiver instead of push_str); mul_assign clones rhs.text it owns. GlyphMatrix::{add,sub,mul}_assign clone every rhs line despite owning rhs. GlyphMatrix::place_in calls component.length() twice per component (two full grapheme walks; per scene rebuild). collect_sorted_children allocates 2 Vecs per aligned parent pair (documented + benched, acceptable; SmallVec<[_;8]> would remove).
Fix: consume rhs by value (into_iter); glyph_matrix() -> Option<&GlyphMatrix> clone only in Add/Assign arms; push_str; cache length() in place_in; &[SceneTreeId] or caller buffer for ids_in_layer_order; index-loop component_at. Bench before/after. Effort M.

### B19. primitives.rs grab-bag; anchor third is dead vocabulary
P3 | dead-code | high
Files: core/primitives.rs:580-777 (Flag/Anchor/AnchorBox/AnchorPoint/AnchorTarget/Positioned/Bounded); whole file 777 lines
Four concepts: (1) ColorFontRegions+Range (live), (2) ApplyOperation+Applicable, (3) Flags, (4) complete anchoring system. Anchor family + Flag::Anchored: ZERO references outside file. Positioned/Bounded zero implementers. CONCEPTS documents AnchorBox "for layout-solver pinning" — no solver exists.
Fix: split into regions.rs/apply.rs/flags.rs; delete Positioned/Bounded and Anchor family (§10) or mark reserved. Effort M.

### B20. Doc statements contradicting code (beyond B6): color_at_region, background_color, § citation drift
P2 | docs | high
Files: element.rs:371-378; area.rs:123-131; core/primitives.rs:185,403; tree_walker.rs:12-14,250-254; util/regions.rs:22-24; model/line.rs:60-61; area_fields.rs:111-113
color_at_region doc "O(1) hash map" — regions is BTreeSet O(log n). GlyphArea.background_color doc "Mutations can modify this directly through the tree walker" — NO field/command variant exists for background_color, background_padding, align_center (3 renderable fields outside §B4 mutation surface). Systematic §-mis-citations post-renumber: no-panic rules cited as §4 (primitives.rs:185, tree_walker.rs:252) and §7 (primitives.rs:403, regions.rs:22) — actual §9; seams cited as §6 (tree.rs:111/156, line.rs:61, area_fields.rs:112, model/mutator.rs:35/199) — actual §7. tree_walker module doc claims "everything else … kept pub" — everything else is private.
Fix: one sweep; also add missing background/align field variants or state rebuild-only. Effort S.
# Findings: baumhard core — Part 3 (B21-B29 + clean list)

### B21. §B9 field-doc gaps (coverage otherwise ~97%)
P3 | docs | high
Files: area.rs:46-49,80-91 (EdgePadding fields+accessors); util/hitbox.rs:16,55-58 (HitBox.rectangles, BoundingRectangle fields); element.rs:84-124 (GfxElement variant fields other than subtree_aabb); camera.rs:54-81 (CameraMutation variant fields); animation.rs:166-170 (TimelineEvent::Interpolation fields)
~30 pub fields lack ///. Fix: one pass, one-liners. Effort S.

### B22. DEFAULT_TERMINATOR half-exposed seam: pub constant, private consumer
P3 | api-design | high
Files: tree_walker.rs:26-68 (pub const), 487-548 (private repeat_while / apply_repeat_while_to_children)
pub "so mutator authors can substitute custom terminators" but only fns accepting terminator are private; compare_apply_repeat_while hardcodes DEFAULT_TERMINATOR (line 216). Unattachable seam. Cosmetics: 1970s is_some()/unwrap() loops (50-66, 537-547); condition.test(&target) passing &&mut.
Fix: make repeat_while pub with documented terminator param, or make DEFAULT_TERMINATOR private + fix module doc. Modernize unwrap loops. Effort S.

### B23. InstructionSpec shadows serde-able Instruction; MutationListSrc lone #[non_exhaustive]
P3 | duplication | high
Files: mutator_builder/ast.rs:210-241; gfx_structs/mutator.rs:27-74,162
Instruction itself derives Serialize/Deserialize yet InstructionSpec re-declares all 4 variants + RepeatWhileAlwaysTrue sugar + hand into_instruction(). Sugar is only delta. MutationListSrc #[non_exhaustive] but siblings ChannelSrc/CountSrc/MutationSrc/CellField not.
Fix: collapse with serde alias/from shim, or document why (sugar) + align non_exhaustive posture. Effort S.

### B24. SceneEntry ownership can't be recovered
P3 | api-design | high
Files: scene.rs:52-95,159-166,231-234
Scene::remove returns Option<SceneEntry> "returning ownership" but fields private, accessors borrow — caller must clone whole arena to reuse. Fix: SceneEntry::into_tree(self)/into_parts. Effort S.

### B25. Add impls for DeltaGlyphArea/GlyphAreaField no production callers; Operation+Operation logs spurious mismatch warn
P3 | dead-code | high
Files: area_mutators.rs:223-245; area_fields.rs:200-289 (Operation arm 273, fallback 283-288)
No consumer of delta+delta outside impl/tests. Within GlyphAreaField::add, Operation(_) arm is empty {} falling through to mismatch fallback — adding two well-formed deltas emits warn!("mismatched variants") though variants match.
Fix: explicit rhs-wins return in Operation arm; find consumer (drag-path delta coalescing) or drop (§10). Effort S.

### B26. GlyphComponentField exported dead vocabulary
P3 | dead-code | high
Files: model/component.rs:17-28; model/mod.rs:26
Declared, documented, re-exported, referenced nowhere. No DeltaGlyphComponent exists.
Fix: delete or annotate reserved with named consumer. Effort S.

### B27. British spellings (120 occurrences across 40 baumhard files)
P3 | convention | high
Concentrations: core/primitives.rs (17 colour), area.rs (8), element.rs (5), area_mutators.rs (5), shape.rs (5 centre/behaviour), model/component.rs (5), camera_tests.rs (4 centred/centre); serialisable (predicate.rs:10), memoised (scene.rs), normalised (shape.rs), initialise (bench name region_indexer_initialise test_bench.rs:264). NOTE: do_shape_ellipse_contains_centre_and_rim is bench-coupled — rename = two-file change per §B8.
Fix: mechanical sweep + two bench-coupled identifiers same commit. Effort S.

### B28. Pre-convention bench bodies violate do_* naming split
P3 | testing | high
Files: gfx_structs/tests/model_tests.rs (matrix_place_in_1, line_add_assign_*, overriding_insert_*), tree_tests.rs (basics_solo_mutation, complex_tree_mutation, event_propagation_*), tree_walker_tests.rs (repeat_while_skip_while, macro_applies_all_mutations_in_order); test_bench.rs:106-259
Three oldest suites export bare names imported directly by bench; newer suites follow do_*. Fix: rename + wrappers + bench update same commit (§B8). Effort S.

### B29. hard_get — panicking test-only helper on production type
P3 | error-handling | high
Files: core/primitives.rs:398-413
"Test-only convenience … Not for interactive paths" yet ordinary pub method on ColorFontRegions beside get, debug! dump loop + expect. All callers tests.
Fix: move into tests tree as free helper, or #[doc(hidden)] + #[track_caller]. Effort S.

## CLEAN (verified by agent)
- AABB cache discipline on blessed path (apply_to invalidates once; memoization correct, well tested)
- bvh_find: pruning, slack inflation, shape refinement, area tie-break, inclusive boundaries — correct, §B7 zero-alloc
- align_child_walks sorted channel-merge correct incl. broadcast both directions
- MapChildren zip: force-apply bypass, nested-instruction forwarding, excess handling, borrow discipline — correct, excellent tests
- insert_regions_at / shrink_regions_after / shift_regions_after / set_or_insert — verified correct
- shape.rs, zoom_visibility.rs, camera.rs — exemplary (math, docs, tests)
- Scene layering/visibility/offset + lazy layer-order cache correct
- RegionParams/RegionIndexer math correct (exhaustive brute-force tests) — issue is nothing uses it (B6)
- debug! in walker hot paths compiled out in release (log release_max_level_off both crates)
- Bench-import drift: none (all imported names resolve)
- pub mod tests pattern, no-custom-errors, silent type-mismatch, RotateWhile stub consistently documented, no unsafe
- Grapheme discipline in live text paths (pop_front/pop_back/place_in/expanding_insert/overriding_insert via grapheme_chad, emoji/ZWJ fixtures) — exception: dormant ignore_initial_space path (B10)
- mutator_builder Repeat expansion: literal/runtime counts, skip-stride channels, section concat, JSON round-trips — correct, thorough tests
