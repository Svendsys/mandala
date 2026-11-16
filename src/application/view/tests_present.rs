// SPDX-License-Identifier: MPL-2.0

//! Glyph-layer presentations on the glyph layers (`view::present`),
//! renderer-free: that an instance is written on the runs of its
//! elements and nowhere else, at its progress; that a color fades from
//! the glyph's own; that a run is put back
//! to the bit when its last instance ends, an overlay on it or not;
//! that the projector rewriting a run, and an overlay stamping it, each
//! bring the instance back on exactly that run; and that a pass's work
//! is the runs it names, counted.

use std::collections::BTreeSet;

use baumhard::arena::NodeId;
use baumhard::document::element::DocKind;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, DocMutatorTree, NodeOp};
use baumhard::document::mutation::step::{DocStep, DocWalk};
use baumhard::document::presentation::{parse_presentations, PresentationDef};
use baumhard::document::values::{ColorRef, Rgba};
use baumhard::document::writer::EditRequest;
use baumhard::engine::anchor::Anchors;
use baumhard::engine::key::BranchKey;
use baumhard::engine::mutator::Mutator;
use baumhard::gfx_structs::run::{GlyphRunData, RunShape};
use baumhard::projection::Layer;

use super::inspect::children_of_kind;
use super::overlay::SELECTION_HIGHLIGHT;
use super::present::Work;
use super::MapView;
use crate::application::document::tests_common::testament;
use crate::application::document::MindMapDocument;
use crate::application::fields::style;
use crate::application::keybinds::ColorAxis;

/// A run, as the reports name it.
type Run = (Layer, NodeId);

/// One presentation, from the JSON of its definition.
fn presentation(json: &str) -> PresentationDef {
    parse_presentations(&format!(r#"{{"presentations":[{json}]}}"#))
        .unwrap_or_else(|error| panic!("{json} reads: {error}"))
        .remove(0)
}

/// Swells and lifts the first three glyphs of a section, dims the run,
/// and reddens its first glyph: in over 100 ms, out over 100.
const SWELL: &str = r##"{"id":"swell","name":"Swell","play":"while","layer":"glyph","target":"sections",
"glyph":[{"run_opacity":0.5},
{"run_overrides":{"scale":{"range":[0,3],"value":2.0}}},
{"run_overrides":{"offset":{"range":[0,3],"value":[0.0,-4.0]}}},
{"run_overrides":{"color":{"range":[0,1],"value":"#ff0000"}}}],
"animate":{"ms":100}}"##;

/// Scales every glyph of a section's run, at once, for as long as it is
/// held.
const HELD: &str = r#"{"id":"held","name":"Held","play":"hold","layer":"glyph","target":"sections",
"glyph":[{"run_overrides":{"scale":{"range":[0,4096],"value":1.25}}}]}"#;

/// Reddens the first glyph of a section's run: in over 100 ms, out over
/// 100.
const REDDEN: &str = r##"{"id":"redden","name":"Redden","play":"while","layer":"glyph","target":"sections",
"glyph":[{"run_overrides":{"color":{"range":[0,1],"value":"#ff0000"}}}],
"animate":{"ms":100}}"##;

/// The text color a test gives a section's spans, so that its glyphs'
/// own color is a literal.
const OWN: Rgba = Rgba::opaque(0x20, 0x40, 0x60);

/// Half way from [`OWN`] to `#ff0000`, per channel, worked by hand:
/// `0x20 + (0xff - 0x20) / 2 = 143.5`, rounded to 144; `0x40 / 2 = 32`;
/// `0x60 / 2 = 48`.
const HALF_WAY: Rgba = Rgba::opaque(144, 32, 48);

/// Red, as `REDDEN` writes it.
const RED: Rgba = Rgba::opaque(0xff, 0, 0);

/// Reddens the first two glyphs of a section's run: in over 100 ms, out
/// over 100.
const REDDEN_TWO: &str = r##"{"id":"redden-two","name":"Redden two","play":"while","layer":"glyph","target":"sections",
"glyph":[{"run_overrides":{"color":{"range":[0,2],"value":"#ff0000"}}}],
"animate":{"ms":100}}"##;

/// Colors the first glyph of a section's run blue, at once, for as long
/// as it is held.
const BLUE_HELD: &str = r##"{"id":"blue","name":"Blue","play":"hold","layer":"glyph","target":"sections",
"glyph":[{"run_overrides":{"color":{"range":[0,1],"value":"#0000ff"}}}]}"##;

/// Blue, as `BLUE_HELD` writes it.
const BLUE: Rgba = Rgba::opaque(0, 0, 0xff);

/// Half way from the selection's tint `#00e5ff` to `#ff0000`, worked by
/// hand: `0xff / 2 = 127.5`, rounded to 128; `0xe5 / 2 = 114.5`, to 115;
/// `0xff / 2`, 128.
const TINT_HALF_WAY: Rgba = Rgba::opaque(128, 115, 128);

/// Half way from [`BLUE`] to `#ff0000`, worked by hand: 128, 0, 128.
const BLUE_HALF_WAY: Rgba = Rgba::opaque(128, 0, 128);

/// Give every span of `node`'s first section the text color `color`, by
/// an edit each (a span's own color is what its glyphs are drawn in),
/// and leave nothing selected. Returns the section.
fn color_first_section(doc: &mut MindMapDocument, node: NodeId, color: Rgba) -> NodeId {
    let section = children_of_kind(doc.doc(), node, DocKind::Section)
        .next()
        .expect("a section");
    let spans: Vec<NodeId> = children_of_kind(doc.doc(), section, DocKind::Span).collect();
    assert!(!spans.is_empty(), "precondition: the section has spans");
    for span in spans {
        doc.selection.select_only(span);
        let steps = style::color(doc, ColorAxis::Text, Some(ColorRef::Rgba(color))).expect("composes");
        doc.apply_for_test(EditRequest {
            steps,
            label: "color".into(),
        })
        .expect("applies");
    }
    doc.selection.clear();
    section
}

/// The color glyph `index` of box run `run` is shaped in, read straight
/// off its laid-out buffer: what the glyph is drawn in with no override.
fn shaped_color(view: &MapView, run: Run, index: usize) -> Option<Rgba> {
    let RunShape::Box(shaped) = run_data(view, run).shaped() else {
        return None;
    };
    shaped
        .buffer
        .layout_runs()
        .flat_map(|line| line.glyphs.iter())
        .nth(index)
        .and_then(|glyph| glyph.color_opt)
        .map(|color| Rgba::new(color.r(), color.g(), color.b(), color.a()))
}

/// The runs `node`'s sections are drawn as.
fn section_runs(view: &MapView, doc: &MindMapDocument, node: NodeId) -> Vec<Run> {
    let mut runs: Vec<Run> = children_of_kind(doc.doc(), node, DocKind::Section)
        .flat_map(|section| view.projector().glyphs_of(section))
        .collect();
    runs.sort_unstable_by_key(|&(layer, glyph)| (layer.index(), glyph));
    runs
}

/// The run at `run`.
fn run_data(view: &MapView, run: Run) -> &GlyphRunData {
    view.layers()
        .tree(run.0)
        .get(run.1)
        .and_then(|node| node.get().glyph_run())
        .expect("a run")
}

/// Everything of a run a run mutation can change, bit for bit.
type Bits = (u32, [u32; 2], Vec<[u32; 2]>, Vec<Option<Rgba>>, Vec<u32>);

fn bits(view: &MapView, run: Run) -> Bits {
    let data = run_data(view, run);
    let overrides = data.overrides();
    (
        data.opacity().to_bits(),
        [data.origin().x.to_bits(), data.origin().y.to_bits()],
        overrides
            .offset()
            .iter()
            .map(|offset| [offset.x.to_bits(), offset.y.to_bits()])
            .collect(),
        overrides.color().to_vec(),
        overrides.scale().iter().map(|scale| scale.to_bits()).collect(),
    )
}

/// Two drawn top-level nodes with a section each, and a third.
fn three_drawn_roots(doc: &MindMapDocument, view: &MapView) -> (NodeId, NodeId, NodeId) {
    let roots: Vec<NodeId> = doc
        .doc()
        .children(doc.doc().root())
        .filter(|&id| doc.doc().get(id).map(|e| e.kind()) == Some(DocKind::Node))
        .filter(|&id| !section_runs(view, doc, id).is_empty())
        .collect();
    assert!(
        roots.len() >= 3,
        "the testament map has three drawn top-level nodes"
    );
    (roots[0], roots[1], roots[2])
}

/// Text typed into `node`'s first section: an edit that rewrites the
/// node's own runs.
fn type_into(doc: &mut MindMapDocument, node: NodeId) {
    let section = children_of_kind(doc.doc(), node, DocKind::Section)
        .next()
        .expect("a section");
    let steps = compose::insert_text(doc.doc(), section, 0, "Z".into()).expect("composes");
    doc.apply_for_test(EditRequest {
        steps,
        label: "type".into(),
    })
    .expect("applies");
}

/// One node op on `node`, as an edit.
fn node_op(doc: &mut MindMapDocument, node: NodeId, op: NodeOp) {
    let mutator = DocMutatorTree::new_with(Mutator::new(
        DocMutation::Node(op),
        BranchKey::new(DocKind::Node, 0),
    ));
    doc.apply_for_test(EditRequest::new(
        vec![DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(vec![node])))],
        "node op",
    ))
    .expect("applies");
}

/// What a pass's report says the stage wrote.
fn presented(report: &super::SyncReport) -> Vec<Run> {
    report.presented.clone()
}

/// The work the stage did since `before`.
fn work_since(view: &MapView, before: Work) -> (usize, usize, usize) {
    let now = view.stage().work;
    (
        now.put_back - before.put_back,
        now.based - before.based,
        now.applied - before.applied,
    )
}

/// An instance is written on the runs of its elements — the node's
/// sections, not its rings, and no other node's — at its progress:
/// offsets, scales and the run's opacity blend, from a run that had no
/// override, and a color from the glyph's own (to the value:
/// `test_a_color_fades_from_the_glyphs_own_color_and_back`). Its end
/// puts every run back to the bit, and the stage keeps no base.
///
/// Controls: with the neutral overrides not given first, the scale at
/// half way is the run's own `1` (or absent), not `1.5`; with the end
/// not putting the run back, the last comparison finds the run still
/// swollen.
#[test]
fn test_an_instance_is_written_on_its_runs_at_its_progress_and_put_back_to_the_bit() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, b, _) = three_drawn_roots(&doc, &view);
    let runs = section_runs(&view, &doc, a);
    let before: Vec<Bits> = runs.iter().map(|&run| bits(&view, run)).collect();
    assert!(
        runs.iter()
            .all(|&run| run_data(&view, run).overrides().is_empty()),
        "precondition: a projected run has no overrides"
    );
    let saved = doc.save_json().unwrap();

    doc.start_presentation(&presentation(SWELL), &[a], 0.0).unwrap();
    let report = view.sync_for_test(&mut doc).expect("a start is synced");
    assert_eq!(report.projected, 0, "a glyph-layer start changes no document");
    assert_eq!(presented(&report), runs, "node a's section runs, and no other");
    assert_eq!(view.stage().based_runs(), runs);
    // At 0 it draws as the run did: its opacity, every glyph at rest,
    // and the glyph it colors in the color it is drawn in.
    for &run in &runs {
        let data = run_data(&view, run);
        assert_eq!(data.opacity(), 1.0);
        assert!(data.overrides().scale().iter().all(|&scale| scale == 1.0));
        let colors = data.overrides().color();
        assert_eq!(colors[0], shaped_color(&view, run, 0), "its own color");
        assert!(colors[1..].iter().all(Option::is_none));
    }

    doc.tick_presentations(50.0);
    view.sync_for_test(&mut doc).expect("a move is synced");
    let first = run_data(&view, runs[0]);
    assert_eq!(first.opacity(), 0.75);
    assert_eq!(first.overrides().scale()[0], 1.5, "half way from 1 to 2");
    assert_eq!(first.overrides().offset()[0], glam::Vec2::new(0.0, -2.0));
    let fading = first.overrides().color()[0];
    assert!(
        fading.is_some() && fading != Some(RED) && fading != shaped_color(&view, runs[0], 0),
        "a color fades from the glyph's own: {fading:?}"
    );

    doc.tick_presentations(100.0);
    view.sync_for_test(&mut doc).expect("synced");
    let first = run_data(&view, runs[0]);
    assert_eq!(first.opacity(), 0.5);
    assert_eq!(first.overrides().scale()[0], 2.0);
    assert_eq!(first.overrides().color()[0], Some(RED));
    // Nothing of it is the document's.
    assert_eq!(doc.save_json().unwrap(), saved);
    assert!(
        section_runs(&view, &doc, b)
            .iter()
            .all(|&run| run_data(&view, run).overrides().is_empty()),
        "node b is untouched"
    );
    assert!(
        view.sync_for_test(&mut doc).is_none(),
        "holding: a frame with nothing to do"
    );

    doc.stop_presentation("swell", &[a], 100.0);
    doc.tick_presentations(200.0);
    let report = view.sync_for_test(&mut doc).expect("an end is synced");
    assert_eq!(presented(&report), runs, "put back, each of them");
    let after: Vec<Bits> = runs.iter().map(|&run| bits(&view, run)).collect();
    assert_eq!(after, before, "to the bit");
    assert!(view.stage().based_runs().is_empty(), "no base is kept");
    assert!(view.sync_for_test(&mut doc).is_none());
}

/// A color fades from the color the glyph is drawn in, its own, on the
/// way in and back on the way out; a glyph the presentation does not
/// color gets no override; and its end leaves the run, which had no
/// color override before it, bit for bit as it was.
///
/// Controls: with no own color given first (the seed left out), the
/// glyph is uncolored at 50 ms, not half way; seeded with white rather
/// than its own color, it is half way from white; with the seed's
/// stretch spanning the whole run, glyph 1 is colored.
#[test]
fn test_a_color_fades_from_the_glyphs_own_color_and_back() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, _, _) = three_drawn_roots(&doc, &view);
    let section = color_first_section(&mut doc, a, OWN);
    view.sync_for_test(&mut doc).expect("the edit is synced");
    let run = view.projector().glyphs_of(section)[0];
    assert!(
        run_data(&view, run).overrides().is_empty(),
        "precondition: no override, of color or any other"
    );
    assert_eq!(
        shaped_color(&view, run, 0),
        Some(OWN),
        "precondition: the glyph is drawn in its section's color"
    );
    let before = bits(&view, run);

    doc.start_presentation(&presentation(REDDEN), &[a], 0.0).unwrap();
    view.sync_for_test(&mut doc).expect("started");
    let colors = |view: &MapView| run_data(view, run).overrides().color().to_vec();
    assert_eq!(colors(&view)[0], Some(OWN), "at 0, what is drawn");
    assert!(
        colors(&view)[1..].iter().all(Option::is_none),
        "a glyph it does not color gets no override"
    );

    doc.tick_presentations(50.0);
    view.sync_for_test(&mut doc).expect("moved");
    assert_eq!(colors(&view)[0], Some(HALF_WAY), "half way from its own color");

    doc.tick_presentations(100.0);
    view.sync_for_test(&mut doc).expect("in");
    assert_eq!(colors(&view)[0], Some(RED));

    doc.stop_presentation("redden", &[a], 100.0);
    doc.tick_presentations(150.0);
    view.sync_for_test(&mut doc).expect("on its way out");
    assert_eq!(colors(&view)[0], Some(HALF_WAY), "half way back");

    doc.tick_presentations(200.0);
    view.sync_for_test(&mut doc).expect("ended");
    assert_eq!(doc.doc().presentations().count(), 0, "it has ended");
    assert_eq!(
        bits(&view, run),
        before,
        "no color override, as before: to the bit"
    );
    assert!(view.stage().based_runs().is_empty());
}

/// A color fades from the selection's tint where the tint is on the
/// glyph, not from the glyph's own color under it; and what it fades from
/// is read again whenever the run is written, so a deselect while it
/// plays moves the fade's start to the glyph's own color at the same
/// progress, and a reselect moves it back. Ended selected, the run is
/// the tinted one, to the bit.
///
/// Control: with the seed given to every glyph the instance colors,
/// already colored or not, glyph 0 fades from its own color under the
/// tint (`#902030`, not `#807380`).
#[test]
fn test_a_color_fades_from_the_tint_and_follows_it() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, _, _) = three_drawn_roots(&doc, &view);
    let section = color_first_section(&mut doc, a, OWN);
    view.sync_for_test(&mut doc).expect("the edit is synced");
    let run = view.projector().glyphs_of(section)[0];
    doc.selection.select_only(a);
    view.sync_for_test(&mut doc).expect("tinted");
    let tinted = bits(&view, run);
    assert_eq!(
        run_data(&view, run).overrides().color()[0],
        Some(SELECTION_HIGHLIGHT),
        "precondition: the tint is on glyph 0"
    );
    let colors = |view: &MapView| run_data(view, run).overrides().color().to_vec();

    doc.start_presentation(&presentation(REDDEN), &[a], 0.0).unwrap();
    doc.tick_presentations(50.0);
    view.sync_for_test(&mut doc).expect("moved");
    assert_eq!(colors(&view)[0], Some(TINT_HALF_WAY), "half way from the tint");

    doc.selection.clear();
    view.sync_for_test(&mut doc).expect("deselected");
    assert_eq!(
        colors(&view)[0],
        Some(HALF_WAY),
        "the tint gone, half way from its own color"
    );

    doc.selection.select_only(a);
    view.sync_for_test(&mut doc).expect("selected again");
    assert_eq!(
        colors(&view)[0],
        Some(TINT_HALF_WAY),
        "half way from the tint again"
    );

    doc.stop_presentation("redden", &[a], 50.0);
    doc.tick_presentations(150.0);
    view.sync_for_test(&mut doc).expect("ended");
    assert_eq!(doc.doc().presentations().count(), 0, "it has ended");
    assert_eq!(bits(&view, run), tinted, "the tinted run, to the bit");
}

/// A color fades from what an instance started before it shows, glyph
/// by glyph: over a range where only glyph 0 is colored before it (blue,
/// held), glyph 0 fades from blue and glyph 1 from its own color. When
/// the earlier instance ends while it plays, glyph 0 fades from its own
/// color from then on. Started the other way round, the held blue is
/// written over the fade. Each ends leaving the run as it was, to the
/// bit.
///
/// Controls: with the seed given to every glyph the instance colors,
/// already colored or not, glyph 0 is `#902030` where `#800080` is
/// asserted (the case that holds the seed to the glyphs no override
/// colors yet, within a range that has both); with no seed, glyph 1 is
/// uncolored at 50 ms, not half way.
#[test]
fn test_a_color_fades_from_what_an_earlier_instance_shows_glyph_by_glyph() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, _, _) = three_drawn_roots(&doc, &view);
    let section = color_first_section(&mut doc, a, OWN);
    view.sync_for_test(&mut doc).expect("the edit is synced");
    let run = view.projector().glyphs_of(section)[0];
    let before = bits(&view, run);
    let colors = |view: &MapView| run_data(view, run).overrides().color().to_vec();

    doc.start_presentation(&presentation(BLUE_HELD), &[a], 0.0)
        .unwrap();
    view.sync_for_test(&mut doc).expect("held");
    assert_eq!(
        colors(&view)[..2],
        [Some(BLUE), None],
        "precondition: glyph 0 colored, glyph 1 not"
    );

    doc.start_presentation(&presentation(REDDEN_TWO), &[a], 0.0)
        .unwrap();
    doc.tick_presentations(50.0);
    view.sync_for_test(&mut doc).expect("moved");
    assert_eq!(
        colors(&view)[..2],
        [Some(BLUE_HALF_WAY), Some(HALF_WAY)],
        "glyph 0 half way from blue, glyph 1 half way from its own color"
    );

    doc.stop_presentation("blue", &[a], 50.0);
    doc.tick_presentations(50.0);
    view.sync_for_test(&mut doc).expect("the blue ended");
    assert_eq!(doc.doc().presentations().count(), 1, "the blue has ended");
    assert_eq!(
        colors(&view)[..2],
        [Some(HALF_WAY), Some(HALF_WAY)],
        "both half way from their own color"
    );

    doc.stop_presentation("redden-two", &[a], 50.0);
    doc.tick_presentations(150.0);
    view.sync_for_test(&mut doc).expect("ended");
    assert_eq!(doc.doc().presentations().count(), 0);
    assert_eq!(bits(&view, run), before, "as it was, to the bit");

    // The other way round: the fade first, the held blue over it.
    doc.start_presentation(&presentation(REDDEN_TWO), &[a], 200.0)
        .unwrap();
    doc.start_presentation(&presentation(BLUE_HELD), &[a], 200.0)
        .unwrap();
    doc.tick_presentations(250.0);
    view.sync_for_test(&mut doc).expect("moved");
    assert_eq!(
        colors(&view)[..2],
        [Some(BLUE), Some(HALF_WAY)],
        "the blue written over the fade on glyph 0"
    );
    doc.stop_presentation("blue", &[a], 250.0);
    doc.stop_presentation("redden-two", &[a], 250.0);
    doc.tick_presentations(350.0);
    view.sync_for_test(&mut doc).expect("ended");
    assert_eq!(doc.doc().presentations().count(), 0);
    assert_eq!(bits(&view, run), before, "as it was, to the bit");
}

/// On a run another instance already gives overrides — a scale, and no
/// color — a later one's color still fades from the glyph's own; ending
/// it leaves the run as the first alone shows it, and ending that one
/// leaves it as it was, each to the bit.
///
/// Control: with the own color given only to a run that has no
/// overrides at all, as the neutral ones are, the glyph is uncolored at
/// 50 ms.
#[test]
fn test_a_color_fades_from_its_own_on_a_run_with_other_overrides() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, _, _) = three_drawn_roots(&doc, &view);
    let section = color_first_section(&mut doc, a, OWN);
    view.sync_for_test(&mut doc).expect("the edit is synced");
    let run = view.projector().glyphs_of(section)[0];
    let before = bits(&view, run);

    doc.start_presentation(&presentation(HELD), &[a], 0.0).unwrap();
    view.sync_for_test(&mut doc).expect("held");
    let held = bits(&view, run);
    let data = run_data(&view, run);
    assert!(
        !data.overrides().is_empty() && data.overrides().color().iter().all(Option::is_none),
        "precondition: the run has overrides, and no color among them"
    );

    doc.start_presentation(&presentation(REDDEN), &[a], 0.0).unwrap();
    doc.tick_presentations(50.0);
    view.sync_for_test(&mut doc).expect("moved");
    let data = run_data(&view, run);
    assert_eq!(data.overrides().scale()[0], 1.25, "the first, under it");
    assert_eq!(
        data.overrides().color()[0],
        Some(HALF_WAY),
        "half way from its own color"
    );

    // Its way out takes its whole 100 ms, from where it turned.
    doc.stop_presentation("redden", &[a], 50.0);
    doc.tick_presentations(150.0);
    view.sync_for_test(&mut doc).expect("ended");
    assert_eq!(doc.doc().presentations().count(), 1, "the second has ended");
    assert_eq!(bits(&view, run), held, "the first alone, to the bit");

    doc.stop_presentation("held", &[a], 150.0);
    doc.tick_presentations(150.0);
    view.sync_for_test(&mut doc).expect("ended");
    assert_eq!(doc.doc().presentations().count(), 0);
    assert_eq!(bits(&view, run), before, "as it was, to the bit");
}

/// With an overlay on the same run — the selection's tint — the
/// instance is written over the tint, the overlay stamping and clearing
/// the run while it plays leave both right, and when the instance ends
/// the run is what the projector and the overlay alone make it, to the
/// bit: the tint with the node selected, the bare run without.
///
/// Controls: with the overlay's runs not taken into the pass, a
/// deselect's `Clear` wipes the instance's scale; with the end not
/// putting the run back, the tinted run keeps the instance's opacity.
#[test]
fn test_an_overlay_and_an_instance_share_a_run_and_the_end_leaves_the_overlays_run() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, _, _) = three_drawn_roots(&doc, &view);
    let runs = section_runs(&view, &doc, a);
    let bare: Vec<Bits> = runs.iter().map(|&run| bits(&view, run)).collect();
    doc.selection.select_only(a);
    view.sync_for_test(&mut doc).expect("tinted");
    let tinted: Vec<Bits> = runs.iter().map(|&run| bits(&view, run)).collect();
    assert_ne!(tinted, bare, "precondition: the tint is on the runs");
    let tint = Some(SELECTION_HIGHLIGHT);

    doc.start_presentation(&presentation(SWELL), &[a], 0.0).unwrap();
    doc.tick_presentations(50.0);
    view.sync_for_test(&mut doc).expect("synced");
    let first = run_data(&view, runs[0]);
    assert_eq!(first.overrides().scale()[0], 1.5);
    assert_eq!(first.overrides().color()[1], tint, "the tint, under it");
    // Both sides color the first glyph, so it blends: half way from the
    // tint to red.
    let blended = first.overrides().color()[0].expect("colored");
    assert_ne!(Some(blended), tint);
    assert_ne!(blended, Rgba::opaque(0xff, 0, 0));

    // Deselected while it plays: the overlay clears the run, and the
    // instance is back on the bare run.
    doc.selection.clear();
    let report = view.sync_for_test(&mut doc).expect("the selection changed");
    assert_eq!(presented(&report), runs, "exactly the runs the overlay wrote");
    let first = run_data(&view, runs[0]);
    assert_eq!(first.overrides().scale()[0], 1.5, "still swollen");
    assert_eq!(first.overrides().color()[1], None, "the tint is gone");
    assert_eq!(first.opacity(), 0.75);

    // Selected again, then played to its end and out.
    doc.selection.select_only(a);
    view.sync_for_test(&mut doc).expect("tinted again");
    assert_eq!(run_data(&view, runs[0]).overrides().color()[1], tint);
    doc.tick_presentations(100.0);
    doc.stop_presentation("swell", &[a], 100.0);
    doc.tick_presentations(150.0);
    view.sync_for_test(&mut doc).expect("on its way out");
    doc.tick_presentations(200.0);
    view.sync_for_test(&mut doc).expect("ended");
    let after: Vec<Bits> = runs.iter().map(|&run| bits(&view, run)).collect();
    assert_eq!(
        after, tinted,
        "the projector's run and the overlay's tint, to the bit"
    );
    assert!(view.stage().based_runs().is_empty());

    // And ended with the node not selected: the bare run.
    doc.start_presentation(&presentation(SWELL), &[a], 300.0).unwrap();
    doc.tick_presentations(350.0);
    view.sync_for_test(&mut doc).expect("playing");
    doc.selection.clear();
    view.sync_for_test(&mut doc).expect("deselected");
    doc.stop_presentation("swell", &[a], 350.0);
    doc.tick_presentations(450.0);
    view.sync_for_test(&mut doc).expect("ended");
    let after: Vec<Bits> = runs.iter().map(|&run| bits(&view, run)).collect();
    assert_eq!(after, bare, "the projector's run, to the bit");
}

/// When the projector rewrites a run an instance plays on, the instance
/// is written on it again — on exactly the rewritten runs it targets,
/// read off the projector's own journals — and an edit that rewrites no
/// targeted run brings none back.
///
/// Control: a stage that does not read the `LayerChanges` leaves the
/// rewritten run without its scale.
#[test]
fn test_a_projector_rewrite_brings_the_instance_back_on_exactly_that_run() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, b, c) = three_drawn_roots(&doc, &view);
    doc.start_presentation(&presentation(HELD), &[a, b], 0.0).unwrap();
    view.sync_for_test(&mut doc).expect("started");
    let scaled = |view: &MapView, run: Run| {
        let scales = run_data(view, run).overrides().scale();
        !scales.is_empty() && scales.contains(&1.25)
    };
    let targeted: BTreeSet<Run> = section_runs(&view, &doc, a)
        .into_iter()
        .chain(section_runs(&view, &doc, b))
        .collect();
    assert!(targeted.iter().all(|&run| scaled(&view, run)));

    type_into(&mut doc, a);
    let report = view.sync_for_test(&mut doc).expect("the edit is synced");
    let rewritten: BTreeSet<Run> = report.rewritten.iter().copied().collect();
    let expected: Vec<Run> = targeted.intersection(&rewritten).copied().collect();
    assert!(!expected.is_empty(), "the edit rewrote a run of node a's");
    assert!(
        section_runs(&view, &doc, b)
            .iter()
            .all(|run| !expected.contains(run)),
        "and none of node b's"
    );
    let mut written = presented(&report);
    written.sort();
    assert_eq!(written, expected, "the rewritten runs it targets, and only those");
    assert!(
        expected.iter().all(|&run| scaled(&view, run)),
        "the override is on the rewritten run again"
    );
    assert!(targeted.iter().all(|&run| scaled(&view, run)));

    // An edit that rewrites runs elsewhere brings nothing back.
    type_into(&mut doc, c);
    let report = view.sync_for_test(&mut doc).expect("synced");
    assert!(report.projected > 0);
    assert_eq!(presented(&report), [], "no targeted run was rewritten");

    // A node hidden and shown again: its runs leave and come back, and
    // the instance is on the new ones.
    node_op(&mut doc, b, NodeOp::SetHidden(true));
    view.sync_for_test(&mut doc).expect("hidden");
    assert!(section_runs(&view, &doc, b).is_empty());
    node_op(&mut doc, b, NodeOp::SetHidden(false));
    let report = view.sync_for_test(&mut doc).expect("shown");
    let back = section_runs(&view, &doc, b);
    assert!(!back.is_empty());
    assert!(back.iter().all(|&run| scaled(&view, run)));
    assert!(back.iter().all(|run| presented(&report).contains(run)));
}

/// A pass's work is the runs it names, counted, on a map with hundreds
/// of runs: an instance that moved costs its own runs; an edit and a
/// selection change elsewhere cost nothing; a selection change on its
/// node costs its runs; an edit that rewrites one of its runs costs
/// that run, and no putting back.
///
/// Control: a stage that rewrites every run it holds a base for on
/// every pass turns the three `(0, 0, 0)` rows into `(k, k, k)`.
#[test]
fn test_a_pass_costs_the_runs_it_names_and_no_others() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, b, c) = three_drawn_roots(&doc, &view);
    let k = section_runs(&view, &doc, a).len();
    let all_runs: usize = baumhard::projection::Layer::ALL
        .iter()
        .map(|&layer| view.layers().tree(layer).descendants().count())
        .sum();
    assert!(
        all_runs > 100 * k,
        "the map is far larger than what the pass names"
    );

    let mark = view.stage().work;
    doc.start_presentation(&presentation(SWELL), &[a], 0.0).unwrap();
    view.sync_for_test(&mut doc).expect("started");
    assert_eq!(
        work_since(&view, mark),
        (0, k, k),
        "a start: its runs based and written"
    );

    let mark = view.stage().work;
    doc.tick_presentations(50.0);
    view.sync_for_test(&mut doc).expect("moved");
    assert_eq!(work_since(&view, mark), (k, k, k), "a move: its runs, each once");

    let mark = view.stage().work;
    type_into(&mut doc, c);
    view.sync_for_test(&mut doc).expect("an edit elsewhere");
    doc.selection.select_only(b);
    view.sync_for_test(&mut doc).expect("a selection elsewhere");
    assert!(view.sync_for_test(&mut doc).is_none());
    assert_eq!(work_since(&view, mark), (0, 0, 0));

    let mark = view.stage().work;
    doc.selection.select_only(a);
    view.sync_for_test(&mut doc).expect("its node is selected");
    assert_eq!(work_since(&view, mark), (k, k, k));

    let mark = view.stage().work;
    type_into(&mut doc, a);
    let report = view.sync_for_test(&mut doc).expect("an edit of its node");
    let rewritten = presented(&report).len();
    assert!(rewritten >= 1 && rewritten <= k);
    assert_eq!(
        work_since(&view, mark),
        (0, rewritten, rewritten),
        "the rewritten runs: nothing to put back, the projector wrote them whole"
    );

    // A second instance on the same runs doubles the applications, not
    // the bases.
    let mark = view.stage().work;
    doc.start_presentation(&presentation(HELD), &[a], 50.0).unwrap();
    view.sync_for_test(&mut doc).expect("started");
    assert_eq!(work_since(&view, mark), (k, k, 2 * k));
}

/// Two instances on one run combine in start order — the later one
/// starts from what the earlier one shows — and ending the earlier
/// leaves the later on the bare run.
#[test]
fn test_instances_on_one_run_combine_in_start_order() {
    let mut doc = testament();
    let mut view = MapView::for_test(&doc);
    let (a, _, _) = three_drawn_roots(&doc, &view);
    let run = section_runs(&view, &doc, a)[0];
    let dim = |id: &str, opacity: f32, ms: u32| {
        presentation(&format!(
            r#"{{"id":"{id}","name":"D","play":"while","layer":"glyph","target":"sections","glyph":[{{"run_opacity":{opacity:?}}}],"animate":{{"ms":{ms}}}}}"#
        ))
    };
    doc.start_presentation(&dim("first", 0.5, 0), &[a], 0.0).unwrap();
    doc.start_presentation(&dim("second", 0.25, 1000), &[a], 0.0)
        .unwrap();
    doc.tick_presentations(500.0);
    view.sync_for_test(&mut doc).expect("synced");
    assert_eq!(
        run_data(&view, run).opacity(),
        0.375,
        "half way from the first's 0.5 to 0.25"
    );
    // Control for the order: the second alone is half way from 1.
    let mut alone_doc = testament();
    let mut alone = MapView::for_test(&alone_doc);
    alone_doc
        .start_presentation(&dim("second", 0.25, 1000), &[a], 0.0)
        .unwrap();
    alone_doc.tick_presentations(500.0);
    alone.sync_for_test(&mut alone_doc).expect("synced");
    assert_eq!(run_data(&alone, run).opacity(), 0.625);

    doc.stop_presentation("first", &[a], 500.0);
    doc.tick_presentations(750.0);
    view.sync_for_test(&mut doc).expect("synced");
    assert_eq!(doc.doc().presentations().count(), 1, "the first has ended");
    assert_eq!(
        run_data(&view, run).opacity(),
        1.0 + (0.25 - 1.0) * 0.75,
        "the second, on the bare run"
    );
}
