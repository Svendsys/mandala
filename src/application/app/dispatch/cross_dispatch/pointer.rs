// SPDX-License-Identifier: MPL-2.0

//! Pointer-gesture apply_* helpers — the bodies mouse and touch
//! gestures run once their `MouseGesture` name has resolved to an
//! `Action` through the keybind table.
//!
//! [`DispatchHit`] lives here rather than in the native dispatcher
//! because both targets now populate one: the payload is what a
//! keyboard-driven dispatch cannot carry (what the press landed on,
//! and where it landed in canvas space), and it is made of plain
//! values that exist on both targets.
//!
//! [`resolve_double_click_route`] is the pure half of the
//! double-click behavior — the decision, with no renderer in sight,
//! so `cargo test` can pin every branch (`TEST_CONVENTIONS §T8`).
//! [`apply_double_click_activate`] is the renderer-driving half and
//! matches exhaustively on the route, so a new [`DoubleClickRoute`]
//! variant is a build error rather than a silently-dropped gesture.

use glam::Vec2;

use crate::application::document::{EdgeLabelSel, EdgeRef, SectionSel, SelectionState};
use crate::application::keybinds::{Action, ResolvedKeybinds};
use baumhard::mindmap::scene_cache::EdgeKey;

use crate::application::app::input_context_core::InputContextCore;
use crate::application::app::scene_rebuild::{rebuild_after_selection_change, rebuild_all};
use crate::application::app::ClickHit;

use super::apply_create_orphan_node_and_edit;

/// Per-event payload that pointer-driven Actions need but keyboard
/// dispatch doesn't. Populated by the mouse handlers on both
/// targets right before they call into the dispatcher; `None` for
/// keyboard / macro / touch callers.
#[derive(Debug, Clone)]
pub struct DispatchHit {
    /// What the click landed on. The `DoubleClickActivate` arm
    /// routes on this.
    pub click_hit: ClickHit,
    /// Canvas-space cursor position at the gesture's trigger time.
    /// Used by orphan-creation / open-editor arms.
    pub canvas_pos: Vec2,
}

/// What a double-click does, decided before anything is mutated.
///
/// Pure output of [`resolve_double_click_route`]: every variant
/// carries the identity the apply half needs, so the apply half
/// never re-inspects the `ClickHit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::application::app) enum DoubleClickRoute {
    /// Select the pointed-at node (section-aware) and open the
    /// inline node text editor on it, seeded with the existing text.
    OpenNodeEditor {
        node_id: String,
        /// `Some(idx)` when the press resolved to a specific section
        /// of a multi-section node. Preserved so the editor opens on
        /// the section the user pointed at rather than section 0.
        section_idx: Option<usize>,
    },
    /// Centre the camera on the *other* endpoint of a portal-mode
    /// edge and select that edge. Icon and text sub-parts share this
    /// route — same endpoint identity, same "navigate" intent.
    PanToPortalPartner { edge: EdgeKey, partner_id: String },
    /// Select the edge label and edit it. The selection commit is
    /// cross-platform; the editor open is native-only today.
    EditEdgeLabel { edge_ref: EdgeRef },
    /// Empty canvas, no edge selected, and the user has explicitly
    /// bound `CreateOrphanNodeAndEdit` somewhere: create a node at
    /// the press position and open its editor clean.
    CreateOrphanAndEdit,
    /// The gesture is a deliberate no-op. Reached for empty-canvas
    /// double-clicks when an edge is selected (the double-click is
    /// the user re-confirming the edge, not asking for a node) or
    /// when `CreateOrphanNodeAndEdit` is unbound — which is the
    /// shipped default.
    Nothing,
}

/// What the caller still has to do after
/// [`apply_double_click_activate`] returns.
///
/// Exists because exactly one branch of the double-click behavior
/// needs state only native has (the single-line editor). Returning
/// the residual — rather than cfg-gating inside the core — keeps one
/// body for the parts both targets share and names the one part they
/// do not.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(in crate::application::app) enum DoubleClickResidual {
    /// The core ran the whole behavior; nothing is left over.
    Done,
    /// The edge-label selection is committed and the scene is
    /// rebuilt. Native additionally opens the single-line editor on
    /// `edge_ref`; WASM has no single-line editor yet, so the browser
    /// stops at "the label is selected".
    OpenEdgeLabelEditor { edge_ref: EdgeRef },
}

/// The `EdgeRef` identity a `ClickHit::EdgeLabel` names, if the hit
/// is one. Single source for the `EdgeKey` → `EdgeRef` conversion so
/// the route resolver and the native residual cannot disagree about
/// which edge the double-click meant.
pub(in crate::application::app) fn edge_label_target(click_hit: &ClickHit) -> Option<EdgeRef> {
    match click_hit {
        ClickHit::EdgeLabel(key) => Some(EdgeRef::new(
            key.from_id.as_str(),
            key.to_id.as_str(),
            key.edge_type.as_str(),
        )),
        _ => None,
    }
}

/// Is the current selection already exactly this edge label?
///
/// The double-click's first click has normally committed the
/// selection already, so re-committing it and rebuilding the scene
/// is work with no visible result. Pinned as a predicate rather than
/// written inline because it is the one place the two pre-unification
/// implementations disagreed, and §4's mobile budget makes the skip
/// worth keeping rather than smoothing away.
pub(in crate::application::app) fn edge_label_selection_is_current(
    selection: &SelectionState,
    edge_ref: &EdgeRef,
) -> bool {
    matches!(selection, SelectionState::EdgeLabel(s) if &s.edge_ref == edge_ref)
}

/// Decide what a double-click on `click_hit` should do.
///
/// Pure: takes the press hit, the current selection, and the
/// resolved keybind table; returns the route. No renderer, no tree,
/// no mutation — so every branch is reachable from `cargo test`.
pub(in crate::application::app) fn resolve_double_click_route(
    click_hit: &ClickHit,
    selection: &SelectionState,
    keybinds: &ResolvedKeybinds,
) -> DoubleClickRoute {
    match click_hit {
        ClickHit::Node(node_id, section_idx) => DoubleClickRoute::OpenNodeEditor {
            node_id: node_id.clone(),
            section_idx: *section_idx,
        },
        ClickHit::PortalMarker { edge, endpoint } | ClickHit::PortalText { edge, endpoint } => {
            let partner_id = if *endpoint == edge.from_id {
                edge.to_id.clone()
            } else {
                edge.from_id.clone()
            };
            DoubleClickRoute::PanToPortalPartner {
                edge: edge.clone(),
                partner_id,
            }
        }
        ClickHit::EdgeLabel(_) => match edge_label_target(click_hit) {
            Some(edge_ref) => DoubleClickRoute::EditEdgeLabel { edge_ref },
            // Unreachable: the arm already matched `EdgeLabel`.
            // Kept total rather than `expect`-ing, per §9 (no panics
            // on interactive paths).
            None => DoubleClickRoute::Nothing,
        },
        ClickHit::Empty => {
            // Empty-canvas double-click only creates when the user
            // opted in by binding `CreateOrphanNodeAndEdit`
            // somewhere — the gesture ships unbound. An edge
            // selection suppresses it outright: the user is working
            // on the edge, not asking for a node.
            let edge_selected = matches!(selection, SelectionState::Edge(_));
            if !edge_selected && keybinds.has_any_binding_for(Action::CreateOrphanNodeAndEdit) {
                DoubleClickRoute::CreateOrphanAndEdit
            } else {
                DoubleClickRoute::Nothing
            }
        }
    }
}

/// Run the double-click behavior for `hit`. The single body both
/// targets' `Action::DoubleClickActivate` dispatch reaches.
///
/// No-ops (returning [`DoubleClickResidual::Done`]) when no document
/// is loaded — every branch needs one, and an interactive path must
/// degrade rather than panic (§9).
pub(in crate::application::app) fn apply_double_click_activate(
    hit: &DispatchHit,
    core: &mut InputContextCore<'_>,
) -> DoubleClickResidual {
    let route = {
        let Some(doc) = core.document.as_deref() else {
            return DoubleClickResidual::Done;
        };
        resolve_double_click_route(&hit.click_hit, &doc.selection, core.keybinds)
    };
    match route {
        DoubleClickRoute::Nothing => DoubleClickResidual::Done,
        DoubleClickRoute::OpenNodeEditor {
            node_id,
            section_idx,
        } => {
            if let Some(doc) = core.document.as_deref_mut() {
                // Section identity is preserved so the editor opens
                // on the section the user pointed at; collapsing to
                // `Single` here would send `open_text_edit` to
                // section 0 on every multi-section node.
                doc.selection = match section_idx {
                    Some(idx) => SelectionState::Section(SectionSel {
                        node_id: node_id.clone(),
                        section_idx: idx,
                    }),
                    None => SelectionState::Single(node_id.clone()),
                };
                rebuild_all(
                    doc,
                    core.interaction_mode,
                    core.mindmap_tree,
                    core.app_scene,
                    core.renderer,
                    core.scene_cache,
                );
                crate::application::app::text_edit::open_text_edit(
                    &node_id,
                    false,
                    doc,
                    core.text_edit_state,
                    core.mindmap_tree,
                    core.app_scene,
                    core.renderer,
                );
            }
            DoubleClickResidual::Done
        }
        DoubleClickRoute::PanToPortalPartner { edge, partner_id } => {
            // Camera first, then selection + rebuild — the order the
            // scene rebuild depends on, since the connection pass
            // projects against the current camera.
            if let Some(doc) = core.document.as_deref() {
                if let Some(node) = doc.mindmap.nodes.get(&partner_id) {
                    core.renderer.set_camera_center(node.center_vec2());
                }
            }
            if let Some(doc) = core.document.as_deref_mut() {
                doc.selection = SelectionState::Edge(EdgeRef::new(
                    &edge.from_id,
                    &edge.to_id,
                    &edge.edge_type,
                ));
                rebuild_all(
                    doc,
                    core.interaction_mode,
                    core.mindmap_tree,
                    core.app_scene,
                    core.renderer,
                    core.scene_cache,
                );
            }
            DoubleClickResidual::Done
        }
        DoubleClickRoute::EditEdgeLabel { edge_ref } => {
            if let Some(doc) = core.document.as_deref_mut() {
                if !edge_label_selection_is_current(&doc.selection, &edge_ref) {
                    let prev = doc.selection.clone();
                    doc.selection =
                        SelectionState::EdgeLabel(EdgeLabelSel::new(edge_ref.clone()));
                    rebuild_after_selection_change(
                        &prev,
                        doc,
                        core.interaction_mode,
                        core.mindmap_tree,
                        core.app_scene,
                        core.renderer,
                        core.scene_cache,
                    );
                }
            }
            DoubleClickResidual::OpenEdgeLabelEditor { edge_ref }
        }
        DoubleClickRoute::CreateOrphanAndEdit => {
            if let Some(doc) = core.document.as_deref_mut() {
                let mut rc = super::rebuild_ctx!(core, doc);
                apply_create_orphan_node_and_edit(hit.canvas_pos, &mut rc, core.text_edit_state);
            }
            DoubleClickResidual::Done
        }
    }
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    //! Route resolution is the half of the double-click behavior
    //! that differed between the two pre-unification
    //! implementations, and it is the half that runs without a
    //! renderer — so it carries the coverage. The apply half is a
    //! renderer-driving `match` over these routes and is out of
    //! scope per `TEST_CONVENTIONS §T8`.

    use super::*;
    use crate::application::document::SectionSel;
    use crate::application::keybinds::KeybindConfig;

    fn keybinds_with_orphan_edit_bound() -> ResolvedKeybinds {
        let mut cfg = KeybindConfig::default();
        cfg.create_orphan_node_and_edit = vec!["Ctrl+Shift+N".into()];
        cfg.resolve()
    }

    fn keybinds_default() -> ResolvedKeybinds {
        KeybindConfig::default().resolve()
    }

    fn edge_key(from: &str, to: &str) -> EdgeKey {
        EdgeKey::new(from, to, "cross_link")
    }

    #[test]
    fn test_double_click_route_node_without_section_opens_whole_node() {
        let route = resolve_double_click_route(
            &ClickHit::Node("n1".into(), None),
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(
            route,
            DoubleClickRoute::OpenNodeEditor {
                node_id: "n1".into(),
                section_idx: None,
            }
        );
    }

    #[test]
    fn test_double_click_route_node_preserves_section_index() {
        let route = resolve_double_click_route(
            &ClickHit::Node("n1".into(), Some(2)),
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(
            route,
            DoubleClickRoute::OpenNodeEditor {
                node_id: "n1".into(),
                section_idx: Some(2),
            }
        );
    }

    #[test]
    fn test_double_click_route_portal_icon_targets_the_far_endpoint() {
        let route = resolve_double_click_route(
            &ClickHit::PortalMarker {
                edge: edge_key("a", "b"),
                endpoint: "a".into(),
            },
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(
            route,
            DoubleClickRoute::PanToPortalPartner {
                edge: edge_key("a", "b"),
                partner_id: "b".into(),
            }
        );
    }

    /// Standing on the `to` endpoint navigates back to `from` — the
    /// partner is "whichever end I am not on", not "always `to`".
    #[test]
    fn test_double_click_route_portal_icon_from_far_end_targets_the_near_endpoint() {
        let route = resolve_double_click_route(
            &ClickHit::PortalMarker {
                edge: edge_key("a", "b"),
                endpoint: "b".into(),
            },
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(
            route,
            DoubleClickRoute::PanToPortalPartner {
                edge: edge_key("a", "b"),
                partner_id: "a".into(),
            }
        );
    }

    /// Portal text and portal icon share one route: same endpoint
    /// identity, same navigate intent.
    #[test]
    fn test_double_click_route_portal_text_matches_portal_icon() {
        let icon = resolve_double_click_route(
            &ClickHit::PortalMarker {
                edge: edge_key("a", "b"),
                endpoint: "a".into(),
            },
            &SelectionState::None,
            &keybinds_default(),
        );
        let text = resolve_double_click_route(
            &ClickHit::PortalText {
                edge: edge_key("a", "b"),
                endpoint: "a".into(),
            },
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(icon, text);
    }

    #[test]
    fn test_double_click_route_edge_label_carries_the_edge_ref() {
        let route = resolve_double_click_route(
            &ClickHit::EdgeLabel(edge_key("a", "b")),
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(
            route,
            DoubleClickRoute::EditEdgeLabel {
                edge_ref: EdgeRef::new("a", "b", "cross_link"),
            }
        );
    }

    /// Shipped default: empty-canvas double-click does nothing,
    /// because `CreateOrphanNodeAndEdit` is unbound out of the box.
    #[test]
    fn test_double_click_route_empty_canvas_is_a_no_op_when_unbound() {
        let route = resolve_double_click_route(
            &ClickHit::Empty,
            &SelectionState::None,
            &keybinds_default(),
        );
        assert_eq!(route, DoubleClickRoute::Nothing);
    }

    #[test]
    fn test_double_click_route_empty_canvas_creates_when_bound() {
        let route = resolve_double_click_route(
            &ClickHit::Empty,
            &SelectionState::None,
            &keybinds_with_orphan_edit_bound(),
        );
        assert_eq!(route, DoubleClickRoute::CreateOrphanAndEdit);
    }

    /// An edge selection suppresses the create even when the Action
    /// is bound — the user is working on the edge.
    #[test]
    fn test_double_click_route_empty_canvas_suppressed_by_edge_selection() {
        let route = resolve_double_click_route(
            &ClickHit::Empty,
            &SelectionState::Edge(EdgeRef::new("a", "b", "cross_link")),
            &keybinds_with_orphan_edit_bound(),
        );
        assert_eq!(route, DoubleClickRoute::Nothing);
    }

    /// Only an *edge* selection suppresses it. An edge-label or a
    /// node selection does not — pinned so a future widening of the
    /// guard to "anything edge-adjacent" is a visible diff.
    #[test]
    fn test_double_click_route_empty_canvas_not_suppressed_by_edge_label_selection() {
        let route = resolve_double_click_route(
            &ClickHit::Empty,
            &SelectionState::EdgeLabel(EdgeLabelSel::new(EdgeRef::new("a", "b", "cross_link"))),
            &keybinds_with_orphan_edit_bound(),
        );
        assert_eq!(route, DoubleClickRoute::CreateOrphanAndEdit);
    }

    #[test]
    fn test_edge_label_target_is_none_for_non_label_hits() {
        assert!(edge_label_target(&ClickHit::Empty).is_none());
        assert!(edge_label_target(&ClickHit::Node("n".into(), None)).is_none());
        assert!(edge_label_target(&ClickHit::PortalMarker {
            edge: edge_key("a", "b"),
            endpoint: "a".into(),
        })
        .is_none());
        assert_eq!(
            edge_label_target(&ClickHit::EdgeLabel(edge_key("a", "b"))),
            Some(EdgeRef::new("a", "b", "cross_link")),
        );
    }

    #[test]
    fn test_edge_label_selection_is_current_matches_only_the_same_label() {
        let er = EdgeRef::new("a", "b", "cross_link");
        assert!(edge_label_selection_is_current(
            &SelectionState::EdgeLabel(EdgeLabelSel::new(er.clone())),
            &er
        ));
        assert!(!edge_label_selection_is_current(
            &SelectionState::EdgeLabel(EdgeLabelSel::new(EdgeRef::new("a", "c", "cross_link"))),
            &er
        ));
        // The *edge* being selected is not the *label* being
        // selected — the double-click must still commit.
        assert!(!edge_label_selection_is_current(
            &SelectionState::Edge(er.clone()),
            &er
        ));
        assert!(!edge_label_selection_is_current(&SelectionState::None, &er));
        assert!(!edge_label_selection_is_current(
            &SelectionState::Section(SectionSel::new("n", 0)),
            &er
        ));
    }
}
