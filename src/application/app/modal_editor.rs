// SPDX-License-Identifier: MPL-2.0

//! The inline text-editor ladder.
//!
//! Two modal editors can own the keyboard and the pointer: the
//! multi-line node text editor ([`super::text_edit`]) and the
//! single-line editor ([`super::single_line_edit`]) bound to an edge
//! label or a portal caption. They are different enough to keep
//! apart — one previews by mutating the live tree and hit-tests a
//! node AABB, the other stages a string and hit-tests a glyph role
//! — but the *ladder* around them is the same twice over:
//!
//! - **Steal**: while open, resolve the key in the editor's input
//!   context; if it is that editor's commit or cancel, dispatch it
//!   through the funnel; otherwise hand the raw key to the editor's
//!   own handler. Either way the key is consumed.
//! - **Release**: a pointer release inside the edited element keeps
//!   editing and consumes the release; a release outside commits
//!   through the funnel and lets the click route normally.
//!
//! [`ModalEditor`] is that ladder, written once.

#![cfg(not(target_arch = "wasm32"))]

use crate::application::keybinds::{Action, InputContext};
use crate::application::platform::input::Key;

use super::input_context::InputHandlerContext;

/// One inline text editor, as the ladder sees it.
///
/// Ordering is load-bearing where the variants are enumerated:
/// [`Self::LADDER`] lists the node text editor first, matching the
/// order the release path has always checked them in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ModalEditor {
    /// The multi-line node text editor.
    Text,
    /// The single-line edge-label / portal-caption editor.
    SingleLine,
}

impl ModalEditor {
    /// Every modal editor, in check order.
    const LADDER: [ModalEditor; 2] = [ModalEditor::Text, ModalEditor::SingleLine];

    /// Is this editor currently open?
    fn is_open(self, ctx: &InputHandlerContext<'_>) -> bool {
        match self {
            ModalEditor::Text => ctx.text_edit_state.is_open(),
            ModalEditor::SingleLine => ctx.single_line_edit_state.is_open(),
        }
    }

    /// The keybind context this editor's keys resolve in.
    fn context(self) -> InputContext {
        match self {
            ModalEditor::Text => InputContext::TextEdit,
            ModalEditor::SingleLine => InputContext::LabelEdit,
        }
    }

    /// The `Action` a click outside the edited element dispatches.
    fn commit_action(self) -> Action {
        match self {
            ModalEditor::Text => Action::TextEditCommit,
            ModalEditor::SingleLine => Action::LabelEditCommit,
        }
    }

    /// Does this editor own `action` as its commit / cancel pair?
    /// Those two are user-named effects, so they belong in the
    /// funnel rather than in the modal handler (CODE_CONVENTIONS
    /// §3); everything else the editor claims is the literal-Key
    /// carve-out.
    fn owns_commit_or_cancel(self, action: &Action) -> bool {
        match self {
            ModalEditor::Text => matches!(action, Action::TextEditCommit | Action::TextEditCancel),
            ModalEditor::SingleLine => {
                matches!(action, Action::LabelEditCommit | Action::LabelEditCancel)
            }
        }
    }

    /// The editor currently stealing keyboard input, if any. The
    /// single-line editor is checked first, matching the order the
    /// keyboard handler has always used; the two are mutually
    /// exclusive by construction anyway (a node selection opens the
    /// text editor, an edge-label / portal selection opens the
    /// single-line one).
    pub(super) fn stealing(ctx: &InputHandlerContext<'_>) -> Option<ModalEditor> {
        if ModalEditor::SingleLine.is_open(ctx) {
            Some(ModalEditor::SingleLine)
        } else if ModalEditor::Text.is_open(ctx) {
            Some(ModalEditor::Text)
        } else {
            None
        }
    }
}

/// Hand a keystroke to the modal editor that is stealing input.
///
/// The commit / cancel pre-filter runs *before* the modal handler:
/// `dispatch_action`'s arm body owns the close-and-rebuild path, and
/// routing through it is what makes those two verbs reachable from
/// macros, the console and IPC as well as from the keyboard. The
/// modal handler keeps only what `Action` cannot carry — character
/// insertion and IME sequences — plus the rebindable cursor / delete
/// primitives, which it re-resolves in the same context.
pub(super) fn steal_key_for_modal(
    modal: ModalEditor,
    key_name: &Option<String>,
    logical_key: &Key,
    ctx: &mut InputHandlerContext<'_>,
) {
    let action = key_name.as_deref().and_then(|n| {
        ctx.keybinds.action_for_context(
            modal.context(),
            n,
            ctx.modifiers.control_key(),
            ctx.modifiers.shift_key(),
            ctx.modifiers.alt_key(),
        )
    });
    if let Some(modal_action) = action.filter(|a| modal.owns_commit_or_cancel(a)) {
        let _ = super::dispatch::dispatch_action(modal_action, ctx, None);
        return;
    }
    let (ctrl, shift, alt) = (
        ctx.modifiers.control_key(),
        ctx.modifiers.shift_key(),
        ctx.modifiers.alt_key(),
    );
    let Some(doc) = ctx.document.as_mut() else {
        return;
    };
    match modal {
        ModalEditor::Text => super::text_edit::handle_text_edit_key(
            key_name,
            logical_key,
            ctrl,
            shift,
            alt,
            ctx.keybinds,
            ctx.text_edit_state,
            doc,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        ),
        ModalEditor::SingleLine => super::single_line_edit::handle_single_line_edit_key(
            key_name,
            logical_key,
            ctrl,
            shift,
            alt,
            ctx.keybinds,
            ctx.single_line_edit_state,
            doc,
            ctx.interaction_mode,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
        ),
    }
}

/// Resolve a pointer release against every open modal editor.
///
/// Returns `true` when the release landed *inside* an edited
/// element: the editor keeps its buffer and the caller must return
/// without routing the click, because routing it would change the
/// selection out from under a live edit.
///
/// Otherwise every open editor commits through the funnel and the
/// caller falls through to the normal click path, so the new
/// selection lands on whatever was clicked. Walking
/// [`ModalEditor::LADDER`] rather than resolving a single active
/// editor keeps the pathological both-open case behaving exactly as
/// the three hand-written blocks this replaced did.
pub(super) fn commit_modal_editors_on_release(ctx: &mut InputHandlerContext<'_>) -> bool {
    for modal in ModalEditor::LADDER {
        if !modal.is_open(ctx) {
            continue;
        }
        if release_stays_inside(modal, ctx) {
            return true;
        }
        let _ = super::dispatch::dispatch_action(modal.commit_action(), ctx, None);
    }
    false
}

/// Did the release land on the element `modal` is editing?
fn release_stays_inside(modal: ModalEditor, ctx: &mut InputHandlerContext<'_>) -> bool {
    let release_canvas = ctx
        .renderer
        .screen_to_canvas(ctx.cursor_pos.0 as f32, ctx.cursor_pos.1 as f32);
    match modal {
        ModalEditor::Text => {
            // Refresh the subtree-AABB cache before the
            // overflow-aware containment check —
            // `point_in_node_aabb` reads `subtree_aabb()`, which
            // returns `None` when the cache is dirty (post-mutation
            // / post-tree-rebuild). A `None` falls back to the
            // container-only path, regressing the multi-section
            // overflow gesture this branch exists for.
            // `ensure_subtree_aabbs` is O(1) on a clean cache and
            // O(arena) on the first call after a mutation; either
            // way it is cheap relative to the click handler.
            if let Some(tree) = ctx.mindmap_tree.as_mut() {
                tree.tree.ensure_subtree_aabbs();
            }
            ctx.text_edit_state
                .node_id()
                .zip(ctx.mindmap_tree.as_ref())
                .map(|(id, tree)| crate::application::document::point_in_node_aabb(release_canvas, id, tree))
                .unwrap_or(false)
        }
        ModalEditor::SingleLine => {
            // Cloned so the `&mut AppScene` hit-test can borrow
            // `ctx` freely; the target is two or three short
            // strings.
            let target = ctx.single_line_edit_state.target().cloned();
            target
                .map(|t| t.release_stays_on_target(ctx.app_scene, release_canvas))
                .unwrap_or(false)
        }
    }
}
