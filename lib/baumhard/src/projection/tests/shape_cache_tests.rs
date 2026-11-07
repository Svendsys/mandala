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

#[test]
fn test_the_shaping_cache_keeps_no_shaping_alive_and_stays_bounded() {
    let mut cache = ShapeCache::default();
    // A lease per shaping, let go before any assertion, so a failing one
    // does not poison the font lock for every other font test.
    let shape = |text: &str, cache: &mut ShapeCache| -> Arc<ShapedRun> {
        let key = key(text);
        let mut fonts = fonts();
        cache.run(&key, || {
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
        })
    };
    // Shapings no unit keeps: their entries are swept, so the cache does
    // not grow with them.
    for i in 0..1000 {
        drop(shape(&format!("gone {i}"), &mut cache));
    }
    assert!(
        cache.len() <= 64,
        "a thousand dropped shapings leave {} entries; the sweep keeps at most its floor",
        cache.len()
    );
    // A shaping a unit keeps is found again, and is not shaped twice.
    let kept = shape("kept", &mut cache);
    let hits = cache.hits;
    let again = shape("kept", &mut cache);
    assert!(Arc::ptr_eq(&kept, &again), "a kept shaping is shared");
    assert_eq!(cache.hits, hits + 1, "and found, not shaped again");
    // Dropped, it is not kept alive by the cache: an equal key shapes anew.
    let weak = Arc::downgrade(&kept);
    drop((kept, again));
    assert!(weak.upgrade().is_none(), "the cache holds no shaping alive");
    let hits = cache.hits;
    drop(shape("kept", &mut cache));
    assert_eq!(cache.hits, hits, "a key no unit holds is shaped again");
}
