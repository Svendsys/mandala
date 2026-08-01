// SPDX-License-Identifier: MPL-2.0

//! The release half of a throttled drag's lifecycle: what each
//! gesture writes to the model when the button comes up, and what
//! it owes the canvas afterwards.
//!
//! Every release body does the same four things in the same order —
//! flush whatever the throttle left pending, commit the gesture to
//! the model (with its undo entry), clear the scene cache when the
//! per-frame drains left stale samples in it, and rebuild. Only the
//! first two are gesture-specific; the last two are a decree the
//! body *names* rather than performs, so the whole commit stays
//! renderer-free and therefore testable (TEST_CONVENTIONS §T8 keeps
//! live wgpu out of the harness). [`ReleaseRefresh::execute`] is the
//! shell that runs the decree.

#![cfg(not(target_arch = "wasm32"))]

use baumhard::mindmap::scene_cache::SceneConnectionCache;
use baumhard::mindmap::tree_builder::MindMapTree;

use crate::application::document::MindMapDocument;

use super::super::scene_rebuild::{rebuild_all, rebuild_scene_only};
use super::DrainContext;

/// The renderer-free half of a release-commit's context: everything
/// a commit body reads or mutates before it names the canvas work
/// it owes.
///
/// `document` is the `Option` rather than a resolved `&mut
/// MindMapDocument` on purpose: two of the seven gestures do
/// tree-side or cursor-side work *before* they check whether a
/// document is loaded, and resolving the option in the shell would
/// silently reorder that.
pub(in crate::application::app) struct ReleaseCommit<'a> {
    pub document: &'a mut Option<MindMapDocument>,
    pub mindmap_tree: &'a mut Option<MindMapTree>,
    pub scene_cache: &'a mut SceneConnectionCache,
}

/// The canvas work a release commit owes once the model write has
/// landed.
///
/// Named rather than performed so [`ReleaseCommit`] can stay free of
/// `&mut Renderer`. The variants are ordered by how much they
/// repaint; the ordering carries no meaning beyond readability.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum ReleaseRefresh {
    /// Nothing to repaint. Reached only when no document is loaded,
    /// in which case the commit found nothing to write either.
    None,
    /// Scene-only rebuild. The gesture moved something that lives
    /// purely in the scene projection (an edge label), so the node
    /// trees are untouched and a full rebuild would be a wasted
    /// walk.
    SceneOnly,
    /// Full rebuild from the authoritative model. Also the
    /// snap-back path: a rejected commit leaves the model unchanged
    /// and the rebuild pulls the on-screen state back to it.
    All,
}

impl ReleaseRefresh {
    /// Run the decree against the live renderer.
    pub(in crate::application::app) fn execute(self, ctx: DrainContext<'_>) {
        let DrainContext {
            document,
            mindmap_tree,
            app_scene,
            renderer,
            scene_cache,
            interaction_mode,
            ..
        } = ctx;
        let Some(doc) = document.as_ref() else {
            return;
        };
        match self {
            Self::None => {}
            Self::SceneOnly => {
                rebuild_scene_only(doc, interaction_mode, app_scene, renderer, scene_cache);
            }
            Self::All => {
                rebuild_all(
                    doc,
                    interaction_mode,
                    mindmap_tree,
                    app_scene,
                    renderer,
                    scene_cache,
                );
            }
        }
    }
}
