// SPDX-License-Identifier: MPL-2.0

//! Interpolation: the [`Interpolate`] implementations (exact endpoints,
//! midpoints, byte rounding), [`Snap`]'s switch points, and
//! [`Journal::write_interpolated`](crate::engine::journal::Journal::write_interpolated)
//! over [`TestElement`] — equal to the reverted tree at `t = 0` and the
//! replayed tree at `t = 1` for every generated scenario, and halfway
//! between with each field obeying its rule.
//!
//! Follows the `do_*()` / `test_*()` split (§T2.2): every `do_*` body
//! has a bench entry in `benches/test_bench.rs`.

use crate::engine::interpolate::{Interpolate, Snap};
use crate::engine::journal::JournalBuilder;
use crate::engine::tests::journal_fixtures::{
    order, run_structural, scenario, snapshot, Scenario, StructuralRun, StructuralStats, SEEDS,
};
use crate::engine::tests::test_element::{add, TestElement};
use crate::engine::tree::Tree;
use crate::util::ordered_vec2::OrderedVec2;
use glam::Vec2;
use ordered_float::OrderedFloat;

// ── Interpolate ───────────────────────────────────────────────────

#[test]
fn test_interpolate_returns_the_endpoints_exactly() {
    do_interpolate_returns_the_endpoints_exactly();
}

/// At `t = 1` every implementation returns `to` itself and at `t = 0`
/// `from` itself, for pairs where the straight-line formula does not:
/// `1e8 + (1 - 1e8) * 1` is `0` in `f32` (the difference rounds to
/// `-1e8`), `1e17 → 1` does the same in `f64`, and `(∞ - 1) * 0` is
/// `NaN`. The preconditions show the formula failing on each pair, so
/// the assertions hold only because the endpoints are exact.
pub fn do_interpolate_returns_the_endpoints_exactly() {
    let naive = |from: f32, to: f32, t: f32| from + (to - from) * t;
    assert_ne!(naive(1e8, 1.0, 1.0), 1.0, "the pair must defeat the formula");
    assert!(
        naive(1.0, f32::INFINITY, 0.0).is_nan(),
        "the pair must defeat the formula"
    );
    let naive64 = |from: f64, to: f64, t: f64| from + (to - from) * t;
    assert_ne!(naive64(1e17, 1.0, 1.0), 1.0, "the pair must defeat the formula");

    assert_eq!(f32::lerp(&1e8, &1.0, 1.0), 1.0);
    assert_eq!(f32::lerp(&1.0, &f32::INFINITY, 0.0), 1.0);
    assert_eq!(f64::lerp(&1e17, &1.0, 1.0), 1.0);
    assert_eq!(
        OrderedFloat::<f32>::lerp(&OrderedFloat(1e8), &OrderedFloat(1.0), 1.0),
        OrderedFloat(1.0)
    );
    assert_eq!(
        <Vec2 as Interpolate>::lerp(&Vec2::new(1e8, 1.0), &Vec2::new(1.0, 1e8), 1.0),
        Vec2::new(1.0, 1e8)
    );
    assert_eq!(
        OrderedVec2::lerp(
            &OrderedVec2::new_f32(1e8, 2.0),
            &OrderedVec2::new_f32(1.0, 3.0),
            1.0
        ),
        OrderedVec2::new_f32(1.0, 3.0)
    );
    assert_eq!(
        <[f32; 4]>::lerp(&[1e8, 0.0, 0.0, 1.0], &[1.0, 0.5, 0.25, 0.0], 1.0),
        [1.0, 0.5, 0.25, 0.0]
    );
    assert_eq!(
        <[f32; 4]>::lerp(&[1.0, 0.5, 0.25, 0.0], &[f32::INFINITY, 0.0, 0.0, 1.0], 0.0),
        [1.0, 0.5, 0.25, 0.0]
    );
}

#[test]
fn test_interpolate_is_linear_between_the_endpoints() {
    do_interpolate_is_linear_between_the_endpoints();
}

/// Midpoints and quarter points, per axis and per channel. Bytes round
/// to the nearest value, halves away from zero — `127.5` is `128` and
/// `10.5` is `11` — where truncating would give `127` and `10`.
pub fn do_interpolate_is_linear_between_the_endpoints() {
    assert_eq!(f32::lerp(&2.0, &4.0, 0.5), 3.0);
    assert_eq!(f32::lerp(&0.0, &8.0, 0.25), 2.0);
    assert_eq!(f64::lerp(&-10.0, &10.0, 0.75), 5.0);
    assert_eq!(
        OrderedFloat::<f32>::lerp(&OrderedFloat(10.0), &OrderedFloat(20.0), 0.5),
        OrderedFloat(15.0)
    );
    assert_eq!(
        <Vec2 as Interpolate>::lerp(&Vec2::new(0.0, 10.0), &Vec2::new(4.0, -10.0), 0.5),
        Vec2::new(2.0, 0.0)
    );
    assert_eq!(
        OrderedVec2::lerp(
            &OrderedVec2::new_f32(0.0, 100.0),
            &OrderedVec2::new_f32(8.0, 0.0),
            0.25
        ),
        OrderedVec2::new_f32(2.0, 75.0)
    );
    assert_eq!(
        <[f32; 4]>::lerp(&[0.0, 1.0, 0.5, 1.0], &[1.0, 0.0, 0.5, 0.0], 0.5),
        [0.5, 0.5, 0.5, 0.5]
    );
    assert_eq!(
        <[u8; 4]>::lerp(&[0, 10, 255, 1], &[255, 11, 0, 2], 0.5),
        [128, 11, 128, 2]
    );
    assert_eq!(
        <[u8; 4]>::lerp(&[0, 0, 0, 0], &[255, 100, 4, 8], 0.25),
        [64, 25, 1, 2]
    );
    assert_eq!(
        <[u8; 4]>::lerp(&[0, 255, 0, 0], &[255, 0, 0, 0], 2.0),
        [255, 0, 0, 0],
        "an extrapolated byte stays in range"
    );
}

#[test]
fn test_snap_switches_where_it_says() {
    do_snap_switches_where_it_says();
}

/// `Start` takes the after value as soon as `t > 0`; `End` only at
/// `t == 1`. `pick` follows `takes_after`.
pub fn do_snap_switches_where_it_says() {
    assert!(!Snap::Start.takes_after(0.0));
    assert!(Snap::Start.takes_after(0.001));
    assert!(Snap::Start.takes_after(1.0));
    assert!(!Snap::End.takes_after(0.0));
    assert!(!Snap::End.takes_after(0.999));
    assert!(Snap::End.takes_after(1.0));
    assert_eq!(*Snap::Start.pick(&"before", &"after", 0.5), "after");
    assert_eq!(*Snap::End.pick(&"before", &"after", 0.5), "before");
}

// ── write_interpolated over TestElement ───────────────────────────

#[test]
fn test_write_interpolated_at_the_ends_equals_revert_and_replay() {
    do_write_interpolated_at_the_ends_equals_revert_and_replay();
}

/// For every seed and both snaps, starting from the tree the calls
/// left: `write_interpolated` at `0` gives the snapshot from before the
/// calls, at `1` the snapshot from after them, and again at `0` after a
/// halfway write — whichever arrangement the tree is in, the write
/// reaches the one it was asked for. `t` outside `0..=1` clamps, and a
/// `NaN` reads as `0`.
pub fn do_write_interpolated_at_the_ends_equals_revert_and_replay() {
    let mut changed = 0;
    for seed in 0..SEEDS {
        let Scenario { mut tree, calls } = scenario(seed);
        let before = snapshot(&tree);
        let mut builder = JournalBuilder::new();
        for call in &calls {
            call.run(&mut tree, &mut builder).unwrap();
        }
        let after = snapshot(&tree);
        if before != after {
            changed += 1;
        }
        let journal = builder.close(&tree);
        for snap in [Snap::Start, Snap::End] {
            journal.write_interpolated(&mut tree, 0.0, snap).unwrap();
            assert_eq!(
                snapshot(&tree),
                before,
                "seed {seed}, {snap:?}: t = 0 is the reverted tree"
            );
            journal.write_interpolated(&mut tree, 1.0, snap).unwrap();
            assert_eq!(
                snapshot(&tree),
                after,
                "seed {seed}, {snap:?}: t = 1 is the replayed tree"
            );
            journal.write_interpolated(&mut tree, 0.5, snap).unwrap();
            tree.check_sorted().unwrap();
            journal.write_interpolated(&mut tree, -3.0, snap).unwrap();
            assert_eq!(
                snapshot(&tree),
                before,
                "seed {seed}, {snap:?}: t < 0 clamps to 0"
            );
            journal.write_interpolated(&mut tree, 7.0, snap).unwrap();
            assert_eq!(snapshot(&tree), after, "seed {seed}, {snap:?}: t > 1 clamps to 1");
            journal.write_interpolated(&mut tree, f32::NAN, snap).unwrap();
            assert_eq!(snapshot(&tree), before, "seed {seed}, {snap:?}: NaN reads as 0");
            journal.replay(&mut tree).unwrap();
        }
    }
    assert!(changed > SEEDS / 2, "most seeds must change something: {changed}");

    write_interpolated_snaps_structure_at_every_t();
}

/// The structural half of
/// [`do_write_interpolated_at_the_ends_equals_revert_and_replay`]: for
/// every seed of [`run_structural`] and both snaps, `write_interpolated`
/// at `0` gives the snapshot from before the calls and at `1` the one
/// after them. Halfway, the structure snaps whole: the tree has
/// exactly the arrangement after — every node, parent and sibling
/// order — with [`Snap::Start`], and exactly the one before with
/// [`Snap::End`], sorted either way. A revert from halfway gives the
/// tree from before.
fn write_interpolated_snaps_structure_at_every_t() {
    let mut stats = StructuralStats::default();
    let mut restructured = 0;
    for seed in 0..SEEDS {
        let StructuralRun {
            mut tree,
            before,
            after,
            journal,
        } = run_structural(seed, &mut stats);
        if order(&before) != order(&after) {
            restructured += 1;
        }
        for snap in [Snap::Start, Snap::End] {
            journal.write_interpolated(&mut tree, 0.0, snap).unwrap();
            assert_eq!(
                snapshot(&tree),
                before,
                "seed {seed}, {snap:?}: t = 0 is the reverted tree"
            );
            journal.write_interpolated(&mut tree, 1.0, snap).unwrap();
            assert_eq!(
                snapshot(&tree),
                after,
                "seed {seed}, {snap:?}: t = 1 is the replayed tree"
            );
            journal.write_interpolated(&mut tree, 0.5, snap).unwrap();
            let expected = if snap == Snap::Start { &after } else { &before };
            assert_eq!(
                order(&snapshot(&tree)),
                order(expected),
                "seed {seed}, {snap:?}: halfway, the structure is the snapped side's"
            );
            tree.check_sorted().unwrap();
            journal.revert(&mut tree).unwrap();
            assert_eq!(
                snapshot(&tree),
                before,
                "seed {seed}, {snap:?}: revert from halfway"
            );
            journal.replay(&mut tree).unwrap();
        }
    }
    assert!(
        restructured > SEEDS / 2,
        "most seeds must change the arrangement: {restructured}"
    );
}

#[test]
fn test_write_interpolated_halfway_lerps_value_and_snaps_the_rest() {
    do_write_interpolated_halfway_lerps_value_and_snaps_the_rest();
}

/// Row `a b c`: `a` (channel 0, value 10), `b` (0, 21), `c` (1, 30). One
/// update takes `a` to value 30 on channel 2, so the row becomes `b c a`.
/// Halfway, `a`'s value is the midpoint, 20. With `Snap::End` its
/// channel is still 0 and the row is `a b c`; with `Snap::Start` its
/// channel is already 2 and the row is `b c a` — the arrangement goes
/// with the key, so the row is sorted either way. `b` was not touched
/// and is never written.
pub fn do_write_interpolated_halfway_lerps_value_and_snaps_the_rest() {
    let mut tree = Tree::new_non_indexed_with(TestElement::new(0, 0));
    let root = tree.root;
    let a = add(&mut tree, root, TestElement::new(0, 10));
    let b = add(&mut tree, root, TestElement::new(0, 21));
    let c = add(&mut tree, root, TestElement::new(1, 30));
    let mut builder = JournalBuilder::new();
    tree.update_journaled(a, &mut builder, |e| {
        e.value = 30;
        e.channel = 2;
    })
    .unwrap();
    let journal = builder.close(&tree);
    let row = |tree: &Tree<TestElement>| root.children(tree.arena()).collect::<Vec<_>>();

    journal.write_interpolated(&mut tree, 0.5, Snap::End).unwrap();
    assert_eq!(tree.arena()[a].get(), &TestElement::new(0, 20));
    assert_eq!(row(&tree), vec![a, b, c]);
    tree.check_sorted().unwrap();

    journal.write_interpolated(&mut tree, 0.5, Snap::Start).unwrap();
    assert_eq!(tree.arena()[a].get(), &TestElement::new(2, 20));
    assert_eq!(row(&tree), vec![b, c, a]);
    tree.check_sorted().unwrap();

    journal.write_interpolated(&mut tree, 0.25, Snap::End).unwrap();
    assert_eq!(tree.arena()[a].get(), &TestElement::new(0, 15));
    assert_eq!(row(&tree), vec![a, b, c]);
    assert_eq!(tree.arena()[b].get(), &TestElement::new(0, 21));
}
