// SPDX-License-Identifier: MPL-2.0

//! The projector's shaping cache (V1-B14, #304; ruling B7.4): N units
//! with one key make one shaping, every unit holds the same `Arc`, and
//! the cache keeps no shaping alive and no more entries than its bound.
//!
//! Shapings are counted off the shaper's per-thread counters
//! ([`cosmic_text::shape_work`]), never a clock.

use std::sync::Arc;

use cosmic_text::shape_work;

use crate::arena::NodeId;
use crate::document::defaults::BUILT_IN;
use crate::document::element::DocKind;
use crate::document::format::load;
use crate::document::ids::Document;
use crate::flow::{shape_run, FlowSpan, ShapedRun};
use crate::projection::shapes::ShapeCache;
use crate::projection::state::RunKey;
use crate::projection::tests::corpus::of_kind;
use crate::projection::tests::support::fonts;
use crate::projection::{GlyphLayers, Projector};

/// A map of `texts.len()` nodes, each with one section of one span.
fn sections(texts: &[&str]) -> Document {
    let nodes: Vec<String> = texts
        .iter()
        .enumerate()
        .map(|(i, text)| {
            format!(
                r#"{{"id":{},"position":[{},0],"sections":[{{"spans":[{{"text":"{text}"}}]}}]}}"#,
                i + 1,
                i * 300
            )
        })
        .collect();
    let text = format!(r#"{{"format":1,"name":"m","nodes":[{}]}}"#, nodes.join(","));
    load(&text).unwrap_or_else(|error| panic!("precondition: the map loads: {error}"))
}

/// `doc` projected in full: its projector, and the shapings of its text
/// it made — not counting the font system's one-time questions of whether
/// some face draws a letter (`ShapeWork::coverage_probes`), which the
/// first map to hold the letter pays.
fn projected(doc: &Document) -> (Projector, u64) {
    let mut fonts = fonts();
    let mut layers = GlyphLayers::new();
    let before = shape_work();
    let projector = Projector::project_full(doc, &mut layers, &mut fonts);
    drop(fonts);
    let work = shape_work().since(before);
    (projector, work.shapes - work.coverage_probes)
}

#[test]
fn test_units_with_one_key_share_one_shaping() {
    const N: usize = 8;
    let shared = sections(&["Shared words"; N]);
    let one = sections(&["Shared words"]);
    let apart: Vec<String> = (0..N).map(|i| format!("Words {i}")).collect();
    let apart = sections(&apart.iter().map(String::as_str).collect::<Vec<_>>());

    let (projector, shared_shapes) = projected(&shared);
    let (_, one_shapes) = projected(&one);
    let (_, apart_shapes) = projected(&apart);
    assert!(
        apart_shapes > one_shapes,
        "precondition: {N} sections of their own texts shape more than one does \
         ({apart_shapes} against {one_shapes})"
    );
    assert_eq!(
        shared_shapes, one_shapes,
        "{N} sections of one text shape as often as one does"
    );
    assert_eq!(
        projector.shapes.hits,
        N - 1,
        "every section but the first takes the first's shaping"
    );
    let boxes: Vec<_> = of_kind(&shared, DocKind::Section)
        .into_iter()
        .map(|section: NodeId| {
            projector
                .section_box(section)
                .map(|(_, shaped)| shaped.clone())
                .expect("precondition: every section is projected")
        })
        .collect();
    assert_eq!(boxes.len(), N, "precondition: {N} sections");
    assert!(
        boxes.iter().all(|shaped| Arc::ptr_eq(shaped, &boxes[0])),
        "every section holds the one shaping"
    );
}

/// A run key of one span of `text` in the built-in section style.
fn key(text: &str) -> Arc<RunKey> {
    Arc::new(RunKey {
        spans: vec![(text.to_string(), BUILT_IN.section)],
    })
}

/// `text`'s key, and its shaping through `cache`: what a unit holds, the
/// key as a unit's `RunFlow` does. A lease per shaping, let go before any
/// assertion, so a failing one does not poison the font lock for every
/// other font test.
fn held(text: &str, cache: &mut ShapeCache) -> (Arc<RunKey>, Arc<ShapedRun>) {
    let mut key = key(text);
    let mut fonts = fonts();
    let shaped = cache.run(&mut key, |key| {
        let style = BUILT_IN.section;
        Arc::new(shape_run(
            &[FlowSpan {
                text: &key.spans[0].0,
                style: &style,
                link: None,
            }],
            1.0,
            &mut fonts,
        ))
    });
    (key, shaped)
}

#[test]
fn test_the_shaping_cache_keeps_no_shaping_alive_and_stays_bounded() {
    let mut cache = ShapeCache::default();
    // Shapings no unit keeps: their entries are swept, so the cache does
    // not grow with them.
    for i in 0..1000 {
        drop(held(&format!("gone {i}"), &mut cache));
    }
    assert!(
        cache.len() <= 64,
        "a thousand dropped shapings leave {} entries; the sweep keeps at most its floor",
        cache.len()
    );
    // A shaping a unit keeps is found again, and is not shaped twice.
    let kept = held("kept", &mut cache);
    let hits = cache.hits;
    let again = held("kept", &mut cache);
    assert!(Arc::ptr_eq(&kept.1, &again.1), "a kept shaping is shared");
    assert_eq!(cache.hits, hits + 1, "and found, not shaped again");
    // Dropped, it is not kept alive by the cache: an equal key shapes anew.
    let weak = Arc::downgrade(&kept.1);
    drop((kept, again));
    assert!(weak.upgrade().is_none(), "the cache holds no shaping alive");
    let hits = cache.hits;
    drop(held("kept", &mut cache));
    assert_eq!(cache.hits, hits, "a key no unit holds is shaped again");
}

#[test]
fn test_the_shaping_cache_keeps_no_key_alive() {
    // Keys of any size, each dropped by its unit, while one unit stays: a
    // dropped key, and its text, is freed at once, not at the next sweep
    // (#304 review). Fewer than the sweep's floor, so no sweep runs.
    let mut cache = ShapeCache::default();
    let live = held("stays", &mut cache);
    let gone: Vec<std::sync::Weak<RunKey>> = (0..63)
        .map(|i| {
            let (key, shaped) = held(&format!("{i} {}", "x".repeat(4096)), &mut cache);
            let weak = Arc::downgrade(&key);
            drop((key, shaped));
            weak
        })
        .collect();
    assert!(
        cache.len() > 1,
        "precondition: the dropped units' entries are still in the cache ({})",
        cache.len()
    );
    let alive = gone.iter().filter(|weak| weak.strong_count() > 0).count();
    assert_eq!(alive, 0, "{alive} dropped keys are kept alive by the cache");
    let hits = cache.hits;
    let again = held("stays", &mut cache);
    assert!(
        Arc::ptr_eq(&live.1, &again.1),
        "the live unit's shaping is still shared"
    );
    assert_eq!(cache.hits, hits + 1, "and found");
}

#[test]
fn test_keys_equal_but_for_the_sign_of_a_zero_share_one_shaping() {
    // `-0.0` and `0.0` compare equal, so two keys that differ only there
    // are equal, and must hash equal to share (#304 review, shapes.rs:49).
    use crate::document::values::Align;
    use crate::flow::shape_box;
    use crate::projection::state::BoxKey;
    let mut cache = ShapeCache::default();
    let style = BUILT_IN.section;
    let boxed = |min: glam::Vec2, cache: &mut ShapeCache| {
        let mut key = Arc::new(BoxKey {
            spans: vec![("zero".to_string(), style)],
            min: Some(min),
            align: Align::Left,
        });
        let mut fonts = fonts();
        let shaped = cache.boxed(&mut key, |key| {
            let span = FlowSpan {
                text: "zero",
                style: &style,
                link: None,
            };
            Arc::new(shape_box(&[span], key.min, key.align, &mut fonts))
        });
        (key, shaped)
    };
    let positive = boxed(glam::Vec2::new(0.0, 10.0), &mut cache);
    let negative = boxed(glam::Vec2::new(-0.0, 10.0), &mut cache);
    assert_eq!(*positive.0, *negative.0, "precondition: the two keys are equal");
    assert!(
        Arc::ptr_eq(&positive.1, &negative.1),
        "keys equal but for the sign of a zero share one shaping"
    );
}

#[test]
fn test_a_share_outlives_the_unit_that_shaped_it() {
    // Review 5813361973 (shapes.rs:149): two units share a shaping; the one
    // that shaped it lets its key go; a third unit with the same key still
    // takes the shaping the second holds, rather than shaping again. Every
    // sharer holds the one key, so the entry lives as long as any of them.
    let mut cache = ShapeCache::default();
    let first = held("dup words", &mut cache);
    let second = held("dup words", &mut cache);
    assert!(
        Arc::ptr_eq(&first.1, &second.1),
        "precondition: the second unit shares the first's shaping"
    );
    assert!(
        Arc::ptr_eq(&first.0, &second.0),
        "and holds the first's key, not a key of its own"
    );
    drop(first);
    let hits = cache.hits;
    let third = held("dup words", &mut cache);
    assert!(
        Arc::ptr_eq(&second.1, &third.1),
        "the shaping the second unit holds is shared with the third once the first lets go"
    );
    assert_eq!(cache.hits, hits + 1, "found, not shaped again");
}
