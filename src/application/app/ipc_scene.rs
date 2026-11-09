// SPDX-License-Identifier: MPL-2.0

//! The app-side half of `scene.hit_test` (`format/ipc.md` §`scene`):
//! what a click at a point would target, read through the canonical
//! click hit chain rather than a copy of it.
//!
//! The chain is private to `app` — `compute_click_hit` asks the
//! projector what is under the point (`MapView::hit`) and resolves the
//! element a click selects for it — so the report is assembled here and
//! handed to the `ipc` side as JSON, the shape
//! [`CommandTarget::shell_state`](crate::application::ipc::CommandTarget::shell_state)
//! already uses. The callee is the very function a mouse press, a
//! release and a touch tap run, which is the whole of the parity
//! guarantee: there is no second hit test to drift.
//!
//! Both targets' — each target's command target consumes it over the
//! same view, which is what keeps `scene.hit_test` one answer. The
//! point resolver beside it ([`resolve_point`], [`surface_of`]) is
//! shared for the same reason: an IPC point lands where the camera
//! says, on either target.

use glam::Vec2;
use serde_json::{json, Value};

use crate::application::document::MindMapDocument;
use crate::application::view::inspect::{element_ref, point_json};
use crate::application::view::MapView;

use crate::application::ipc::injected::Surface;
use crate::application::ipc::space::{Point, Space};
use crate::application::ipc::ActionOutcome;
use crate::application::renderer::Renderer;

use super::dispatch::DispatchOutcome;
use super::{compute_click_hit, ClickHit};

/// `point` in both spaces, `(surface, canvas)`, through `renderer`'s
/// camera as it stands now. One body for both targets' command targets
/// and both targets' injected-input landing: where an IPC point lands
/// is the camera's answer, and the camera is the same type on both.
pub(super) fn resolve_point(renderer: &Renderer, point: Point) -> (Vec2, Vec2) {
    match point.space {
        Space::Surface => (
            Vec2::new(point.x, point.y),
            renderer.screen_to_canvas(point.x, point.y),
        ),
        Space::Canvas => {
            let canvas = Vec2::new(point.x, point.y);
            (renderer.canvas_to_screen(canvas), canvas)
        }
    }
}

/// Where `point` lands on the surface, through `renderer`'s camera as
/// it stands now: how an injected event's position is resolved.
pub(super) fn surface_of(renderer: &Renderer, point: Point) -> Surface {
    let (surface, _) = resolve_point(renderer, point);
    (f64::from(surface.x), f64::from(surface.y))
}

/// The funnel's verdict in the protocol's vocabulary. A refusal keeps
/// its reason, which `act.action` returns (#194). One body for both
/// targets' command targets, so the browser's `act.action` reports a
/// refusal the way native's does — once it has one: `Refused` is
/// native-only, as its constructors are, so the browser's match has no
/// arm for it until a browser arm declines for a stated reason.
pub(super) fn action_outcome(outcome: DispatchOutcome) -> ActionOutcome {
    match outcome {
        DispatchOutcome::Handled => ActionOutcome::Handled,
        DispatchOutcome::Unhandled => ActionOutcome::Unhandled,
        #[cfg(not(target_arch = "wasm32"))]
        DispatchOutcome::Refused(reason) => ActionOutcome::Refused(reason),
    }
}

impl ClickHit {
    /// The collapsed hit as `scene.hit_test` reports it: `kind` is the
    /// variant's own name, with the element beside it. Exhaustive with no
    /// wildcard arm, so a new variant breaks the build here until its
    /// wire form is written into `format/ipc.md`.
    fn to_ipc_json(self, doc: &MindMapDocument) -> Value {
        match self {
            ClickHit::Empty => json!({ "kind": "Empty" }),
            ClickHit::Element(element) => {
                json!({ "kind": "Element", "element": element_ref(doc.doc(), element) })
            }
        }
    }
}

/// Everything a click at `canvas` would resolve: the projector's hit
/// (`MapView::hit`) — the element whose glyph is under the point, the
/// node it belongs to — and the element a click selects for it
/// (`compute_click_hit`, the one hit chain a press and a tap run).
///
/// `surface` is reported beside `canvas` so the agent sees both
/// spellings of the point it asked about.
///
/// # Costs
///
/// Two descents of the projector's hit index (one for the report, one
/// through `compute_click_hit`, so the two cannot disagree). Mutates
/// only the layers' memoized subtree bounds.
pub(super) fn hit_report(canvas: Vec2, surface: Vec2, doc: &MindMapDocument, view: &mut MapView) -> Value {
    let hit = view.hit(canvas).map(|hit| {
        json!({
            "element": element_ref(doc.doc(), hit.element),
            "node": element_ref(doc.doc(), hit.node),
        })
    });
    let click_hit = compute_click_hit(Some(doc), Some(view), canvas);
    json!({
        "canvas": point_json(canvas),
        "surface": point_json(surface),
        "click_hit": click_hit.to_ipc_json(doc),
        "hit": hit,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::selection::every_node;
    use crate::application::view::inspect::element_bounds;

    /// The report's `click_hit` is exactly what a click there selects,
    /// and its `hit` names the node the click's element belongs to — at
    /// the middle of every drawn node and at a point off the map.
    #[test]
    fn test_hit_report_agrees_with_a_click_at_every_probe() {
        let doc = crate::application::document::tests_common::testament();
        let mut renderer_free = MapView::for_test(&doc);
        let probes: Vec<Vec2> = every_node(doc.doc())
            .filter_map(|n| element_bounds(doc.doc(), renderer_free.projector(), renderer_free.layers(), n))
            .map(|r| r.origin + r.size / 2.0)
            .take(40)
            .chain([Vec2::new(-1.0e6, -1.0e6)])
            .collect();
        assert!(probes.len() > 2, "the fixture draws nodes");
        let mut hits = 0;
        for probe in probes {
            let report = hit_report(probe, probe, &doc, &mut renderer_free);
            let click = compute_click_hit(Some(&doc), Some(&mut renderer_free), probe);
            assert_eq!(report["click_hit"], click.to_ipc_json(&doc), "{probe}");
            match click {
                ClickHit::Element(_) => {
                    hits += 1;
                    assert!(report["hit"]["node"]["node"].is_u64(), "{report}");
                }
                ClickHit::Empty => assert!(report["hit"].is_null(), "{report}"),
            }
        }
        assert!(hits > 0, "some probe lands on a node");
    }

    /// A refused dispatch keeps its reason all the way to the
    /// `act.action` reply that shows it. The controls are the other two
    /// verdicts: neither invents a reason, and a `Handled` stays one.
    #[test]
    fn test_a_refused_dispatch_keeps_its_reason_for_ipc() {
        assert_eq!(
            action_outcome(DispatchOutcome::Refused("no reason-4c1e")),
            ActionOutcome::Refused("no reason-4c1e")
        );
        assert_eq!(
            action_outcome(DispatchOutcome::Unhandled),
            ActionOutcome::Unhandled
        );
        assert_eq!(action_outcome(DispatchOutcome::Handled), ActionOutcome::Handled);
    }
}
