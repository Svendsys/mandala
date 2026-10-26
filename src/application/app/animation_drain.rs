// SPDX-License-Identifier: MPL-2.0

//! The animation tick: what one frame owes a running
//! `CustomMutation` animation, on both targets.
//!
//! A frame has an input-driven half and a time-driven half
//! (`heartbeat`'s module doc), and this is the time-driven half both
//! targets run: native's heartbeat runs it from the event loop's
//! `about_to_wait` and from a virtual clock's step, the browser's
//! frame runs it from `WindowEvent::RedrawRequested`
//! (`run_wasm::event_redraw_requested`). The body is one function
//! because the two targets must advance an animation the same way —
//! before #144 it lived in the native-gated `drain_frame.rs`, and an
//! animated mutation started in the browser never ticked.
//!
//! Two decisions live here with the tick so the targets cannot make
//! them differently: [`drain_animation_tick`] is *what* a frame does
//! for an animation, and [`keeps_ticking`] is *whether the loop owes
//! another frame* for it — native's `needs_continuation` and the
//! browser's "ask for the next frame" both read the latter.

use crate::application::clock::{AppClock, ClockMode};
use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;

use super::scene_rebuild::rebuild_all;

/// Advance every active animation to the app clock's whole-millisecond
/// reading `now_ms`, and report whether anything moved — the half of
/// the tick that needs no renderer, so the sequence a frame runs is
/// pinnable without one (TEST_CONVENTIONS §T8).
///
/// Each tick lerps the from / to snapshots into the model and, on
/// completion, routes the final state through `apply_custom_mutation`
/// so the standard model-sync + undo-push runs once
/// (`document::animations`). `true` means the caller owes a rebuild:
/// a position moved, or a completion committed.
pub(super) fn advance_animations(
    now_ms: u64,
    document: Option<&mut MindMapDocument>,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
) -> bool {
    match document {
        Some(doc) if doc.has_active_animations() => doc.tick_animations(now_ms, mindmap_tree.as_mut()),
        _ => false,
    }
}

/// Tick any active animations at `now_ms` ([`advance_animations`])
/// and, when something advanced, rebuild the scene so the frame
/// about to be drawn shows it. The heartbeat passes the app clock's
/// reading, so under a virtual clock the tick lands on the step's
/// instants.
///
/// Under native the loop's `ControlFlow::Wait` / `ControlFlow::Poll`
/// choice is decided in `pump::iterate` from `needs_continuation`,
/// which reads [`keeps_ticking`] — so when no animation is active and
/// no other source needs continuation, the loop parks until the next
/// OS event. The browser's frame reads the same predicate to decide
/// whether to ask for the next `requestAnimationFrame`.
pub(super) fn drain_animation_tick(
    now_ms: u64,
    document: Option<&mut MindMapDocument>,
    interaction_mode: &super::InteractionMode,
    mindmap_tree: &mut Option<baumhard::mindmap::tree_builder::MindMapTree>,
    app_scene: &mut crate::application::scene_host::AppScene,
    renderer: &mut Renderer,
    scene_cache: &mut baumhard::mindmap::scene_cache::SceneConnectionCache,
) {
    let Some(doc) = document else {
        return;
    };
    if !advance_animations(now_ms, Some(doc), mindmap_tree) {
        return;
    }
    // Animation ticks lerp positions (and on completion route
    // through `apply_custom_mutation`) in place; the cache's
    // `pre_clip_positions` go stale under both paths. Clear before
    // re-sampling.
    scene_cache.clear();
    rebuild_all(
        doc,
        interaction_mode,
        mindmap_tree,
        app_scene,
        renderer,
        scene_cache,
    );
}

/// Whether a running animation owes the loop another frame: one is
/// active, and the clock moves on its own. Under a virtual clock the
/// heartbeats that advance an animation are a step's, so it keeps
/// nothing awake — with time frozen there is nothing a frame could do
/// for it (`heartbeat::needs_continuation`).
///
/// One predicate for both targets, because the two spell the
/// consequence differently and must not drift in the condition:
/// native answers `ControlFlow::Poll`, the browser asks for the next
/// `requestAnimationFrame` from inside the one it is drawing.
pub(super) fn keeps_ticking(document: Option<&MindMapDocument>, clock: &AppClock) -> bool {
    document.is_some_and(MindMapDocument::has_active_animations) && clock.mode() == ClockMode::Real
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::{
        first_testament_node_id, load_test_doc, TestNudgeMutation,
    };
    use baumhard::mindmap::animation::{AnimationTiming, Easing};
    use baumhard::mindmap::custom_mutation::{CustomMutation, TargetScope};

    /// `NudgeRight(100)` over `duration_ms`, linear.
    fn slide(duration_ms: u32) -> CustomMutation {
        TestNudgeMutation::new("slide", TargetScope::SelfOnly)
            .magnitude(100.0)
            .timing(AnimationTiming {
                duration_ms,
                delay_ms: 0,
                easing: Easing::Linear,
                then: None,
            })
            .build()
    }

    fn virtual_clock() -> AppClock {
        let mut clock = AppClock::default();
        clock.set_mode(ClockMode::Virtual);
        clock
    }

    /// Nothing to advance, nothing advanced: no document, a document
    /// with no animation. The negative control for every row below
    /// — a tick that reported `true` here would rebuild the scene
    /// on every idle frame of the session.
    #[test]
    fn test_a_frame_with_nothing_running_advances_nothing() {
        let mut tree = None;
        assert!(!advance_animations(16, None, &mut tree), "no document");
        let mut idle = load_test_doc();
        assert!(
            !advance_animations(16, Some(&mut idle), &mut tree),
            "no animation"
        );
        assert!(
            !keeps_ticking(None, &AppClock::default()),
            "no document owes no frame"
        );
        assert!(
            !keeps_ticking(Some(&idle), &AppClock::default()),
            "no animation owes no frame"
        );
    }

    /// **The browser's loop, as a sequence of frames.** Each frame
    /// does what `run_wasm::event_redraw_requested` does with the
    /// renderer taken out: advance at the app clock, then ask for
    /// another frame iff [`keeps_ticking`]. Driven at 16 ms the way
    /// `requestAnimationFrame` paces it, the sequence lerps the node
    /// across the duration, lands it at `from + delta` exactly once
    /// (the completion's `apply_custom_mutation`), pushes the one undo
    /// entry, and then stops asking — which is the acceptance criterion
    /// of #144 for the half of it that has no GPU in it.
    ///
    /// Fails on the pre-fix shape in the only way it could: with no
    /// frame to drive the tick, the instance is started and nothing
    /// below it happens.
    #[test]
    fn test_frames_driven_until_keeps_ticking_is_false_complete_the_animation_once() {
        let mut doc = load_test_doc();
        let node = first_testament_node_id(&doc);
        let from_x = doc.mindmap.nodes.get(&node).unwrap().position.x;
        doc.undo_stack.clear();
        let mut tree = Some(doc.build_tree());
        let clock = AppClock::default();

        doc.start_animation(&slide(160), &node, 0);
        assert!(
            keeps_ticking(Some(&doc), &clock),
            "a started animation owes the first frame"
        );

        let mut frames = 0;
        let mut mid_flight_seen = false;
        let mut now_ms = 0;
        while keeps_ticking(Some(&doc), &clock) {
            now_ms += 16;
            frames += 1;
            assert!(
                frames <= 32,
                "an animation of 160 ms must not need more than a few frames"
            );
            let advanced = advance_animations(now_ms, Some(&mut doc), &mut tree);
            assert!(advanced, "every frame of a running animation moves something");
            let x = doc.mindmap.nodes.get(&node).unwrap().position.x;
            if x > from_x + 1.0 && x < from_x + 99.0 {
                mid_flight_seen = true;
            }
        }
        assert!(
            mid_flight_seen,
            "the frames lerped the node, rather than snapping it"
        );
        assert_eq!(
            frames, 10,
            "160 ms at 16 ms per frame: nine lerps and the completing tick"
        );

        let landed_x = doc.mindmap.nodes.get(&node).unwrap().position.x;
        assert!(
            (landed_x - (from_x + 100.0)).abs() < 1e-3,
            "the completion lands at from + delta once; got {landed_x}, from {from_x}"
        );
        assert_eq!(
            doc.undo_stack.len(),
            1,
            "the completion pushed its one undo entry"
        );
        assert!(!doc.has_active_animations());
        assert!(
            !advance_animations(now_ms + 16, Some(&mut doc), &mut tree),
            "a frame after completion advances nothing"
        );
    }

    /// Under a virtual clock a running animation owes the loop no
    /// frame: the step's heartbeats advance it, and a frame of the
    /// loop could do nothing for it. The real-clock row beside it is
    /// the control, so the predicate is pinned on the clock term and
    /// not only on the animation term.
    #[test]
    fn test_a_running_animation_keeps_only_a_real_clock_ticking() {
        let mut doc = load_test_doc();
        let node = first_testament_node_id(&doc);
        doc.start_animation(&slide(500), &node, 0);
        assert!(keeps_ticking(Some(&doc), &AppClock::default()));
        assert!(!keeps_ticking(Some(&doc), &virtual_clock()));
    }
}
