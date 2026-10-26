// SPDX-License-Identifier: MPL-2.0

//! How one fired `CustomMutation` lands, on either target, whichever
//! route fired it.
//!
//! A `CustomMutation` reaches the app through three routes: a bound
//! keystroke or a macro step (`dispatch_custom_mutation_for_key`, the
//! two `MacroDispatchTarget` impls), a node's `OnClick` binding fired
//! by a click or a tap (`click_triggers::fire_onclick_triggers`), and
//! the `mutation apply` console verb. The first two share this body
//! — before #144 each carried its own copy of the animated-vs-instant
//! routing, and the copies disagreed about what a mutation owes when
//! no tree is installed. The console verb is instant by construction
//! (it has no clock to start an animation on) and shares the instant
//! half, [`MindMapDocument::apply_custom_mutation_instantly`].
//!
//! The target is section-aware. A click lands on a section of a
//! multi-section node and the hit carries its index, so a
//! section-bound trigger animates against that section
//! (`start_animation_at`) and two sections of one node can run the
//! same mutation at once. A keystroke targets the selected node and
//! passes `None`: `SelectionState::Single` is what the keystroke tier
//! resolves, and routing a `Section` selection through here is a
//! widening this signature leaves open rather than one it performs.

use baumhard::mindmap::custom_mutation::CustomMutation;
use baumhard::mindmap::scene_cache::SceneConnectionCache;
use baumhard::mindmap::tree_builder::MindMapTree;

use crate::application::document::MindMapDocument;

/// Land `cm` on `node_id` (and `section_idx` of it, when the route
/// that fired it names one), as of app time `now_ms`.
///
/// Animated (`timing.duration_ms > 0`): start the instance; the frame
/// (`animation_drain::drain_animation_tick`, on both targets) lerps
/// it and commits the final state through `apply_custom_mutation` at
/// completion, which is where its undo entry is pushed. Instant:
/// apply through the installed tree, or through a scratch one when
/// none is installed ([`MindMapDocument::apply_custom_mutation_instantly`]),
/// and drop the connection samples, which the apply may have moved.
/// Then, either way, the mutation's document actions — the theme
/// switch rides along whether or not the mutator half animates, and
/// whether or not a tree was there.
///
/// **No failure return, by decision (#144).** Lacking a tree is not a
/// reason to apply less, so there is nothing here that can decline.
/// Whether a keystroke *found* a mutation to fire is the lookups'
/// answer, made before this is reached. The caller owes a scene
/// rebuild afterward: the instant path moved the model (and the
/// installed tree, when it had one), the animated path will move it
/// on the next frame, and a document action repaints every node.
///
/// Cross-platform: `MindMapDocument`, `Option<MindMapTree>`,
/// `SceneConnectionCache` and a clock reading, nothing renderer-shaped.
/// `pub(crate)` for the parity tests in
/// `crate::application::document::tests_mutations`.
pub(crate) fn fire_custom_mutation(
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<MindMapTree>,
    scene_cache: &mut SceneConnectionCache,
    cm: &CustomMutation,
    node_id: &str,
    section_idx: Option<usize>,
    now_ms: u64,
) {
    if cm.timing.as_ref().is_some_and(|t| t.duration_ms > 0) {
        doc.start_animation_at(cm, node_id, section_idx, now_ms);
    } else {
        doc.apply_custom_mutation_instantly(cm, node_id, mindmap_tree.as_mut());
        scene_cache.clear();
    }
    // Two of the three routes from a map to `apply_document_actions`
    // run through here, and this body carries no `SourceTier`: route
    // 1's (a keystroke or a macro step) is checked at `dispatch_macro`
    // before it gets here, and route 2 (a node's `OnClick` binding)
    // has none at all — it is authored in the `.mindmap.json`, which
    // this project treats as untrusted (`macros::SourceTier`'s
    // "hostile shared mindmap"). Both shipped variants are in-memory
    // theme writes, so there is nothing to gate today; the first
    // variant that touches the filesystem, the network or another
    // process must be refused on route 2 as well as at
    // `dispatch_macro`, or it stays reachable from any node's
    // `OnClick` — and a gate placed here has to be told which route
    // fired it. CODE_CONVENTIONS §3 carries the rule and enumerates
    // the routes.
    doc.apply_document_actions(cm);
}
