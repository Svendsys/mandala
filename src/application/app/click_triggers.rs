// SPDX-License-Identifier: MPL-2.0

//! Cross-platform `OnClick`-trigger fan-out, called by the
//! native click handler, the WASM mouse-released handler and the
//! touch tap. The per-section / per-node
//! `find_triggered_mutations_at` lookup is identical on both
//! platforms, and each triggered mutation lands through
//! [`fire_custom_mutation`], the one body the keystroke and macro
//! routes land through too; only the `PlatformContext` and the
//! `now` source differ, both injected.

use baumhard::mindmap::custom_mutation::{PlatformContext, Trigger};

use super::fire_custom_mutation::fire_custom_mutation;
use crate::application::document::MindMapDocument;

/// Fire any `OnClick` triggers bound to `(node_id, hit_section)`
/// on the given `platform`, each through [`fire_custom_mutation`]
/// with `hit_section` as its section target.
///
/// Returns whether **anything** fired, which is the caller's signal
/// that the document may have changed beyond its selection. Both
/// click paths feed it to
/// [`RebuildTier::for_click`](crate::application::app::scene_rebuild::RebuildTier::for_click):
/// a trigger can carry a document action that repaints every node,
/// so the selection delta alone stops being a safe answer the moment
/// one runs. Deliberately coarse — `true` on any triggered mutation,
/// including one whose actions turn out to be a no-op — because the
/// consequence of over-reporting is one extra rebuild and the
/// consequence of under-reporting is a stale canvas.
pub(in crate::application::app) fn fire_onclick_triggers(
    doc: &mut MindMapDocument,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
    hit_node_id: &str,
    hit_section: Option<usize>,
    platform: PlatformContext,
    now_ms: u64,
) -> bool {
    let triggered = doc.find_triggered_mutations_at(hit_node_id, hit_section, &Trigger::OnClick, &platform);
    let fired = !triggered.is_empty();
    for cm in triggered {
        fire_custom_mutation(
            doc,
            mindmap_tree,
            scene_cache,
            &cm,
            hit_node_id,
            hit_section,
            now_ms,
        );
    }
    fired
}
