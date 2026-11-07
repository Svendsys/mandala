// SPDX-License-Identifier: MPL-2.0

//! App-layer scene host for the **screen-space overlays**: the trees of
//! the modals drawn over the canvas — the console and the color picker —
//! in a Baumhard [`Scene`].
//!
//! The map itself is not here. It is drawn from the projection's glyph
//! layers and the chrome layer (`crate::application::view`), which the
//! renderer keeps per glyph element and updates from journals; nothing
//! map-shaped is registered into a scene, signed or re-projected.
//!
//! Each overlay's tree is registered when its modal opens and removed
//! when it closes, and updated in between by §B2 mutator dispatch
//! driven by a structural signature: a change that keeps the tree's
//! shape is applied in place, one that changes it rebuilds the tree.

use std::collections::HashMap;

#[cfg(test)]
use baumhard::arena::NodeId;
use baumhard::gfx_structs::scene::{Scene, SceneTreeId};
use baumhard::gfx_structs::tree::{GfxMutatorTree, GfxTree};
use glam::Vec2;

/// The fixed **screen-space** overlay components that the app's
/// `AppScene` can host. Overlay trees are drawn without the camera
/// transform at `scale = 1.0` (see the renderer's chrome pass).
#[derive(Copy, Clone, Eq, PartialEq, Hash, Debug)]
pub enum OverlayRole {
    /// Command-line overlay. Registered when the console opens,
    /// removed on close. See `src/application/console/`.
    Console,
    /// Inline color wheel. Registered when the picker opens,
    /// removed on commit or cancel. See
    /// `src/application/color_picker/`.
    ColorPicker,
}

/// The two arms of the §B2 overlay-tree dispatch, returned by
/// [`AppScene::overlay_dispatch`] so the console / color-picker rebuild
/// paths choose between the mutator and full-rebuild arms by one idiom.
#[derive(Copy, Clone, Eq, PartialEq, Debug)]
pub enum OverlayDispatch {
    /// Signature matched the registered tree — apply the mutator.
    InPlaceMutator,
    /// Signature mismatch (or nothing registered) — rebuild + re-register.
    FullRebuild,
}

/// Conventional layer integers used when inserting trees into the
/// overlay scene. Higher layers draw on top.
pub mod layers {
    /// Glyph-wheel color picker overlay.
    pub const COLOR_PICKER: i32 = 100;
    /// Console overlay.
    pub const CONSOLE: i32 = 200;
}

/// The overlay trees of the screen-space modals. See the [module
/// docs](self).
pub struct AppScene {
    overlay: Scene,
    console: Option<SceneTreeId>,
    color_picker: Option<SceneTreeId>,
    /// The structural signature each overlay's registered tree was built
    /// at. The console uses `(scrollback_rows, completion_rows)`; the
    /// color picker none yet (its cell count is a compile-time constant).
    /// Cleared on [`Self::unregister_overlay`].
    overlay_signatures: HashMap<OverlayRole, u64>,
}

impl Default for AppScene {
    fn default() -> Self {
        Self::new()
    }
}

impl AppScene {
    /// Empty scene with no trees registered.
    pub fn new() -> Self {
        AppScene {
            overlay: Scene::new(),
            console: None,
            color_picker: None,
            overlay_signatures: HashMap::new(),
        }
    }

    /// Read-only view of the overlay scene. Used by the renderer's
    /// chrome (screen-space) pass.
    pub fn overlay_scene(&self) -> &Scene {
        &self.overlay
    }

    /// Layer-ordered handles into the overlay scene, as a vector so the
    /// renderer can iterate without holding a `&mut Scene`.
    pub fn overlay_ids_in_layer_order(&mut self) -> Vec<SceneTreeId> {
        self.overlay.ids_in_layer_order()
    }

    /// Register (or replace) the tree backing a named overlay role. A
    /// tree already registered for the role is removed first, so
    /// open-close cycles leak no slab entries.
    pub fn register_overlay(&mut self, role: OverlayRole, tree: GfxTree, offset: Vec2) -> SceneTreeId {
        if let Some(old) = self.overlay_role_slot_mut(role).take() {
            self.overlay.remove(old);
        }
        let layer = match role {
            OverlayRole::Console => layers::CONSOLE,
            OverlayRole::ColorPicker => layers::COLOR_PICKER,
        };
        let id = self.overlay.insert(tree, layer, offset);
        *self.overlay_role_slot_mut(role) = Some(id);
        id
    }

    /// Remove an overlay's tree if registered, and its recorded
    /// structural signature: a re-register starts from a full rebuild.
    pub fn unregister_overlay(&mut self, role: OverlayRole) {
        if let Some(id) = self.overlay_role_slot_mut(role).take() {
            self.overlay.remove(id);
        }
        self.overlay_signatures.remove(&role);
    }

    /// Record the structural signature of the tree registered for
    /// `role`.
    pub fn set_overlay_signature(&mut self, role: OverlayRole, signature: u64) {
        self.overlay_signatures.insert(role, signature);
    }

    /// The structural signature recorded for `role`, if any: `None` when
    /// no tree is registered or none was recorded, which the caller
    /// treats as "rebuild".
    pub fn overlay_signature(&self, role: OverlayRole) -> Option<u64> {
        self.overlay_signatures.get(&role).copied()
    }

    /// Which §B2 arm to take for `role` at `signature`.
    pub fn overlay_dispatch(&self, role: OverlayRole, signature: u64) -> OverlayDispatch {
        if self.overlay_id(role).is_some() && self.overlay_signature(role) == Some(signature) {
            OverlayDispatch::InPlaceMutator
        } else {
            OverlayDispatch::FullRebuild
        }
    }

    /// Handle currently assigned to the named overlay role.
    pub fn overlay_id(&self, role: OverlayRole) -> Option<SceneTreeId> {
        match role {
            OverlayRole::Console => self.console,
            OverlayRole::ColorPicker => self.color_picker,
        }
    }

    /// Apply a mutator to the tree registered for an overlay role.
    pub fn apply_overlay_mutator(&mut self, role: OverlayRole, mutator: &GfxMutatorTree) {
        if let Some(id) = self.overlay_id(role) {
            self.overlay.apply_mutator(id, mutator);
        }
    }

    /// Hit-test the overlay scene: which overlay, and which element of
    /// its tree, is under `screen_pt`. Test-gated: each modal hit-tests
    /// its own geometry (the picker's `hit_test_picker`, the console's
    /// row math), so nothing asks the scene which overlay a point is
    /// over.
    #[cfg(test)]
    pub fn overlay_at(&mut self, screen_pt: Vec2) -> Option<(OverlayRole, NodeId)> {
        let hit = self.overlay.component_at(screen_pt)?;
        let role = self.overlay_role_for_id(hit.0)?;
        Some((role, hit.1))
    }

    fn overlay_role_slot_mut(&mut self, role: OverlayRole) -> &mut Option<SceneTreeId> {
        match role {
            OverlayRole::Console => &mut self.console,
            OverlayRole::ColorPicker => &mut self.color_picker,
        }
    }

    #[cfg(test)]
    fn overlay_role_for_id(&self, id: SceneTreeId) -> Option<OverlayRole> {
        if Some(id) == self.console {
            Some(OverlayRole::Console)
        } else if Some(id) == self.color_picker {
            Some(OverlayRole::ColorPicker)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A re-registered overlay replaces the old tree rather than adding
    /// one, and unregistering forgets the signature, so the next open
    /// rebuilds before it can mutate in place.
    #[test]
    fn test_overlay_register_replaces_and_unregister_forgets_the_signature() {
        let mut scene = AppScene::new();
        let first = scene.register_overlay(OverlayRole::Console, GfxTree::new_non_indexed(), Vec2::ZERO);
        scene.set_overlay_signature(OverlayRole::Console, 7);
        assert_eq!(scene.overlay_dispatch(OverlayRole::Console, 7), OverlayDispatch::InPlaceMutator);
        assert_eq!(scene.overlay_dispatch(OverlayRole::Console, 8), OverlayDispatch::FullRebuild);
        let second = scene.register_overlay(OverlayRole::Console, GfxTree::new_non_indexed(), Vec2::ZERO);
        assert_ne!(first, second);
        assert_eq!(scene.overlay_ids_in_layer_order(), vec![second], "the first tree was removed");
        scene.unregister_overlay(OverlayRole::Console);
        assert_eq!(scene.overlay_id(OverlayRole::Console), None);
        assert_eq!(scene.overlay_signature(OverlayRole::Console), None);
        assert_eq!(scene.overlay_dispatch(OverlayRole::Console, 7), OverlayDispatch::FullRebuild);
    }
}
