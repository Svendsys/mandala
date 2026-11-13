// SPDX-License-Identifier: MPL-2.0

//! Nodes hide the paths behind them (#332): a connector's glyph is left
//! out where it would come within its own gap of what a node it does not
//! end at draws — its text and its rings — and its label, whole, where
//! the box round the label's text meets it; both come back when the node
//! moves away, and a move re-hides only the paths whose glyphs or label
//! the node's old or new region meets. Glyph cells and inks are read from
//! the runs' layouts and metrics here, not through the projector's own
//! reading of them, and every test ends on the oracle.

use glam::Vec2;
use rustc_hash::FxHashSet;
use serde_json::{json, Value};

use crate::arena::NodeId;
use crate::document::element::DocKind;
use crate::document::mutation::ops::{DocMutation, NodeOp};
use crate::document::tests::writer_support::{request, single};
use crate::flow::{Curve, Rect};
use crate::gfx_structs::run::{GlyphRunData, RunShape};
use crate::projection::state::UnitBody;
use crate::projection::tests::maps::{
    blocker_text, ident, kid, load_map, merged, node, two_paths, AWAY, ON_P, ON_Q,
};
use crate::projection::tests::support::Projected;
use crate::projection::{Layer, Projector, Stage};

/// The canvas box of each inked glyph of a path run: its pen the run's
/// baseline below its box's corner, its ink about the pen as shaped.
fn inks(run: &GlyphRunData) -> Vec<Rect> {
    let RunShape::Path { run: shaped, placed } = run.shaped() else {
        panic!("a path run");
    };
    placed
        .glyphs
        .iter()
        .filter_map(|g| {
            let ink = shaped.glyphs[g.glyph as usize].ink?;
            let pen = run.origin() + g.pos + Vec2::new(0.0, shaped.baseline());
            Some(Rect::new(pen + ink.min, ink.max - ink.min))
        })
        .collect()
}

/// The canvas cell of each laid-out glyph a box run draws: its advance
/// by its line's height, from the run's origin; a glyph of whitespace
/// draws nothing.
fn cells(run: &GlyphRunData) -> Vec<Rect> {
    let RunShape::Box(shaped) = run.shaped() else {
        panic!("a box run");
    };
    shaped
        .buffer
        .layout_runs()
        .flat_map(|line| {
            let (top, height, text) = (line.line_top, line.line_height, line.text);
            line.glyphs
                .iter()
                .filter(move |g| g.w > 0.0 && !text[g.start..g.end].trim().is_empty())
                .map(move |g| Rect::new(Vec2::new(g.x, top), Vec2::new(g.w, height)))
                .collect::<Vec<_>>()
        })
        .map(|r| Rect::new(run.origin() + r.origin, r.size))
        .collect()
}

/// The box round `boxes`, `None` for none.
fn bounds(boxes: &[Rect]) -> Option<Rect> {
    boxes.iter().copied().reduce(|a, b| a.union(&b))
}

/// The run each glyph element of `unit` holds in `layer`.
fn runs(p: &Projected, unit: NodeId, layer: Layer) -> Vec<GlyphRunData> {
    p.projector
        .glyphs_of(unit)
        .into_iter()
        .filter(|(l, _)| *l == layer)
        .filter_map(|(l, glyph)| p.layers.tree(l).arena()[glyph].get().glyph_run().cloned())
        .collect()
}

/// What `node` draws, as its layers hold it: the box round its text's
/// glyph cells, and the box round its rings' inks.
fn drawn_by(p: &Projected, node: NodeId) -> (Option<Rect>, Option<Rect>) {
    let units = &p.projector.nodes[&node].units;
    let text: Vec<Rect> = units
        .iter()
        .flat_map(|&u| runs(p, u, Layer::Sections))
        .flat_map(|r| cells(&r))
        .collect();
    let rings: Vec<Rect> = units
        .iter()
        .flat_map(|&u| runs(p, u, Layer::Frames))
        .flat_map(|r| inks(&r))
        .collect();
    (bounds(&text), bounds(&rings))
}

/// The box round everything `node` draws: what it hides by.
fn region_of(p: &Projected, node: u32) -> Rect {
    let (text, rings) = drawn_by(p, ident(&p.doc, node));
    match (text, rings) {
        (Some(a), Some(b)) => a.union(&b),
        (a, b) => a.or(b).expect("the node draws something"),
    }
}

/// The run of `element` the connections layer holds.
fn drawn(p: &Projected, element: NodeId) -> GlyphRunData {
    runs(p, element, Layer::Connections)
        .into_iter()
        .next()
        .expect("the path's body is drawn")
}

/// Whether `a` and `b` meet, edges included.
fn meet(a: &Rect, b: &Rect) -> bool {
    a.origin.x <= b.max().x && b.origin.x <= a.max().x && a.origin.y <= b.max().y && b.origin.y <= a.max().y
}

/// The gap between two boxes, 0 where they meet.
fn box_gap(a: &Rect, b: &Rect) -> f32 {
    (b.origin - a.max())
        .max(a.origin - b.max())
        .max(Vec2::ZERO)
        .length()
}

/// The path's own gap, the margin it is hidden by, measured here: the
/// median distance between its consecutive laid inks, box to box — not
/// read from how `hide` reckons it. A `repeat` path spreads its slack over
/// its gaps, so this is its font's gap and a sliver more: the checks that
/// read it allow a twentieth either way.
fn own_gap(projector: &Projector, path: NodeId) -> f32 {
    let laid = inks(projector.laid(path).expect("laid"));
    let mut gaps: Vec<f32> = laid.windows(2).map(|w| box_gap(&w[0], &w[1])).collect();
    gaps.sort_by(f32::total_cmp);
    let gap = gaps.get(gaps.len() / 2).copied().unwrap_or(0.0);
    assert!(gap > 0.0, "precondition: the path's glyphs keep a gap, {gap}");
    gap
}

/// The connectors whose hide stage ran in the last call.
fn hidden_again(p: &Projected) -> FxHashSet<NodeId> {
    p.projector
        .ran
        .iter()
        .filter(|d| d.stage == Stage::Hide)
        .map(|d| d.unit)
        .filter(|&u| p.doc.get(u).is_some_and(|e| e.kind() == DocKind::Connector))
        .collect()
}

/// Move node `id` to `to`.
fn move_to(p: &mut Projected, id: u32, to: Vec2) {
    let node = ident(&p.doc, id);
    let mutation = single(DocKind::Node, DocMutation::Node(NodeOp::SetPosition(to)));
    p.apply(request(mutation, vec![node], "move")).unwrap();
}

/// The connector of node `id`.
fn path_of(p: &Projected, id: u32) -> NodeId {
    kid(&p.doc, ident(&p.doc, id), DocKind::Connector)
}

/// How many of `path`'s laid glyphs its layer leaves out, having held
/// that they are the ones within its gap of `region`: every glyph drawn
/// is one laid, and keeps clear of the region by its gap; every glyph
/// left out comes within its gap of it.
fn hides_exactly(p: &Projected, path: NodeId, region: Rect) -> usize {
    let gap = own_gap(&p.projector, path);
    let laid = inks(p.projector.laid(path).expect("laid"));
    let drawn = inks(&drawn(p, path));
    for ink in &drawn {
        assert!(
            laid.contains(ink),
            "a drawn glyph at {} is one laid",
            ink.center()
        );
        assert!(
            !meet(&ink.grown(gap * 0.95), &region),
            "a drawn glyph at {} within the gap {gap} of the node at {region:?}",
            ink.center()
        );
    }
    let hidden: Vec<&Rect> = laid.iter().filter(|i| !drawn.contains(i)).collect();
    for ink in &hidden {
        assert!(
            meet(&ink.grown(gap * 1.05), &region),
            "a glyph at {} left out, not within the gap {gap} of the node at {region:?}",
            ink.center()
        );
    }
    hidden.len()
}

/// How many of `path`'s laid glyphs its layer leaves out.
fn left_out(p: &Projected, path: NodeId) -> usize {
    inks(p.projector.laid(path).expect("laid")).len() - inks(&drawn(p, path)).len()
}

/// A ring of middle dots `size` of the node's shorter side, 4 out.
fn dotted(size: f32) -> Value {
    json!({"inset": 4.0, "rings": [{"fit": "repeat", "size": size, "spans": [{"text": "·"}]}]})
}

/// A connector across a node it does not end at draws no glyph within its
/// own gap of what the node draws, and every other glyph, on both sides of
/// it.
///
/// Control: with `hide` leaving every glyph in, red on the drawn glyph
/// within the gap.
#[test]
fn test_projection_a_node_hides_the_glyphs_of_a_path_behind_it() {
    let mut p = Projected::new(two_paths(ON_P, json!({}), json!({})));
    let path = path_of(&p, 2);
    let region = region_of(&p, 3);
    let behind = hides_exactly(&p, path, region);
    assert!(
        behind >= 3,
        "precondition: the path runs behind the node, {behind} glyphs"
    );
    let shown = inks(&drawn(&p, path));
    for (side, x) in [("left", region.origin.x), ("right", region.max().x)] {
        assert!(
            shown.iter().any(|i| (i.center().x < x) == (side == "left")),
            "the path is drawn on the node's {side}"
        );
    }
    assert_eq!(
        left_out(&p, path_of(&p, 5)),
        0,
        "the path the node is not on is drawn whole"
    );
    p.check().unwrap();
}

/// A node hides by what it draws, not by its rectangle (the owner's answer
/// on #334): a path across the empty part of a node's box, clear of its
/// text, is drawn whole — a node `min_size` far larger than its text, at
/// its top-left — and a path across a section authored much wider than
/// its text and centered, as the frames demo's "half" is, is hidden by
/// the text alone and drawn over the box's empty sides.
///
/// Controls: with the region the node's drawn rectangle, red on the
/// glyphs left out; with a box's text measured from its left, not as
/// aligned, red on the glyphs drawn over the centered text.
#[test]
fn test_projection_a_node_hides_by_what_it_draws_not_its_rectangle() {
    // A box 400 by 300 from y = -100, its text at its top-left, ending
    // some 60 above the path along y = 20.
    let wide = json!({"min_size": [400.0, 300.0]});
    let mut p = Projected::new(two_paths(Vec2::new(300.0, -100.0), wide, json!({})));
    let path = path_of(&p, 2);
    let rect = p.projector.node_rect(ident(&p.doc, 3)).unwrap();
    let gap = own_gap(&p.projector, path);
    let laid = inks(p.projector.laid(path).unwrap());
    assert!(
        laid.iter().any(|i| meet(&i.grown(gap), &rect)),
        "precondition: the path crosses the node's rectangle"
    );
    assert_eq!(
        hides_exactly(&p, path, region_of(&p, 3)),
        0,
        "the path is drawn whole"
    );
    p.check().unwrap();
    // A section 400 wide from x = 300, its text centered in it, across
    // the path.
    let centered = json!({"sections": [{"min_size": [400.0, 40.0], "align": "center", "size": 30.0, "spans": [{"text": "HIDE"}]}]});
    let mut p = Projected::new(two_paths(Vec2::new(300.0, 0.0), centered, json!({})));
    let path = path_of(&p, 2);
    let text = region_of(&p, 3);
    assert!(
        text.origin.x > 400.0 && text.max().x < 600.0,
        "precondition: the text is centered in its box, {text:?}"
    );
    assert!(
        hides_exactly(&p, path, text) >= 3,
        "the centered text hides the path"
    );
    let shown = inks(&drawn(&p, path));
    for (side, range) in [("left", 310.0..390.0), ("right", 610.0..690.0)] {
        assert!(
            shown.iter().any(|i| range.contains(&i.center().x)),
            "the path is drawn over the box's empty {side} side"
        );
    }
    p.check().unwrap();
}

/// A ringed ellipse whose text fills its box hides by its text's box,
/// corners and all, and by the ellipse of its rings: the text's corner
/// cells, outside the ellipse, hide as the rest of its text does.
///
/// Control: with the whole region shaped as the ellipse, as one box,
/// red on the glyphs drawn over the text's corners.
#[test]
fn test_projection_an_ellipses_text_hides_to_its_corners() {
    // Fit-to-text: the ellipse is inscribed in the text's box, and its
    // thin ring a little outside it. The path runs 3 below the text's top.
    let fitted = json!({"shape": "ellipse", "min_size": [0.0, 0.0], "sections": [{"size": 30.0, "spans": [{"text": "HIDE HIDE HIDE HIDE"}]}], "frame": dotted(0.05)});
    let mut p = Projected::new(two_paths(Vec2::new(300.0, 17.0), fitted, json!({})));
    let path = path_of(&p, 2);
    let (text, rings) = drawn_by(&p, ident(&p.doc, 3));
    let (text, rings) = (text.unwrap(), rings.expect("the ring is drawn"));
    let gap = own_gap(&p.projector, path);
    let (center, half) = (rings.center(), rings.size * 0.5);
    let corners = inks(p.projector.laid(path).unwrap())
        .into_iter()
        .filter(|i| {
            let grown = i.grown(gap);
            let nearest = center.clamp(grown.origin, grown.max());
            meet(&grown, &text) && ((nearest - center) / half).length() >= 1.05
        })
        .count();
    assert!(
        corners >= 2,
        "precondition: the path meets the text's corners outside the ellipse, {corners} glyphs"
    );
    for ink in inks(&drawn(&p, path)) {
        assert!(
            !meet(&ink.grown(gap * 0.95), &text),
            "a drawn glyph at {} within the gap {gap} of the text at {text:?}",
            ink.center()
        );
    }
    p.check().unwrap();
}

/// A node moved off a path shows its glyphs again, without the path being
/// laid again: the hide stage is a view over the placement. Only the path
/// its old region met is hidden again.
///
/// Control: with `hide` recording no reads, red on the glyphs shown —
/// nothing marks the path when the node leaves it.
#[test]
fn test_projection_a_node_moved_off_a_path_shows_its_glyphs_again() {
    let mut p = Projected::new(two_paths(ON_P, json!({}), json!({})));
    let path = path_of(&p, 2);
    let before = left_out(&p, path);
    move_to(&mut p, 3, AWAY);
    assert!(before > 0, "precondition: the node hid glyphs, {before}");
    assert_eq!(left_out(&p, path), 0, "every glyph is drawn again");
    assert_eq!(
        hidden_again(&p),
        [path].into_iter().collect(),
        "only the path it left is hidden again"
    );
    assert!(
        !p.projector
            .ran
            .iter()
            .any(|d| d.unit == path && d.stage == Stage::Place),
        "the path is not laid again"
    );
    p.check().unwrap();
}

/// A node moved onto a path hides its glyphs there, and re-hides that
/// path alone; moved on from it to the other path, it re-hides the two —
/// the one its old region met and the one its new region meets — and the
/// first is drawn whole again.
///
/// Controls: with `rehide` finding no path in the paths' index, red on
/// the glyphs hidden behind it; with it queuing every path, red on the
/// path it is not on hidden again.
#[test]
fn test_projection_a_node_moved_onto_a_path_rehides_only_the_paths_it_meets() {
    let mut p = Projected::new(two_paths(AWAY, json!({}), json!({})));
    let (one, two) = (path_of(&p, 2), path_of(&p, 5));
    move_to(&mut p, 3, ON_P);
    assert!(
        hides_exactly(&p, one, region_of(&p, 3)) >= 3,
        "the path it moved onto is hidden"
    );
    assert_eq!(hidden_again(&p), [one].into_iter().collect(), "onto P: P alone");
    p.check().unwrap();
    move_to(&mut p, 3, ON_Q);
    assert!(
        hides_exactly(&p, two, region_of(&p, 3)) >= 3,
        "the path it moved onto is hidden"
    );
    assert_eq!(left_out(&p, one), 0, "the path it left is whole");
    assert_eq!(
        hidden_again(&p),
        [one, two].into_iter().collect(),
        "from P to Q: both"
    );
    p.check().unwrap();
}

/// A node's text, edited shorter in a box its minimum size keeps as wide,
/// no longer hides the glyphs past its last letter: the region follows
/// the text, not the box, through the section's placement.
///
/// Control: with `section_place` not noting its node when its text moved,
/// red on the glyphs left out.
#[test]
fn test_projection_a_nodes_text_shortened_shows_the_path_past_it() {
    use crate::document::mutation::ops::SpanOp;
    let long = json!({"min_size": [500.0, 60.0], "sections": [{"size": 30.0, "spans": [{"text": "HIDE HIDE HIDE HIDE"}]}]});
    let mut p = Projected::new(two_paths(Vec2::new(250.0, -10.0), long, json!({})));
    let path = path_of(&p, 2);
    let before = hides_exactly(&p, path, region_of(&p, 3));
    let section = kid(&p.doc, ident(&p.doc, 3), DocKind::Section);
    let span = kid(&p.doc, section, DocKind::Span);
    let edit = single(DocKind::Span, DocMutation::Span(SpanOp::SetText("H".into())));
    p.apply(request(edit, vec![span], "shorten")).unwrap();
    let after = hides_exactly(&p, path, region_of(&p, 3));
    assert!(
        after >= 1 && after * 4 < before,
        "the glyphs past the last letter are drawn again: {before} hidden, then {after}"
    );
    p.check().unwrap();
}

/// A node whose text is deleted, and which draws nothing else, hides
/// nothing: the section that left takes its part of the region with it,
/// though the node's rectangle, held by its minimum size, stays as it was.
///
/// Control: with `drop_unit` not noting the node, red on the glyphs left
/// out.
#[test]
fn test_projection_a_node_whose_text_is_deleted_shows_the_path() {
    use crate::document::mutation::compose::remove;
    use crate::document::writer::EditRequest;
    let wide = json!({"min_size": [120.0, 60.0]});
    let mut p = Projected::new(two_paths(ON_P, wide, json!({})));
    let path = path_of(&p, 2);
    assert!(left_out(&p, path) > 0, "precondition: the node hides glyphs");
    let section = kid(&p.doc, ident(&p.doc, 3), DocKind::Section);
    let steps = remove(&p.doc, DocKind::Section, &[section]).unwrap();
    p.apply(EditRequest::new(steps, "delete the text")).unwrap();
    assert_eq!(left_out(&p, path), 0, "every glyph is drawn again");
    p.check().unwrap();
}

/// A path's own end nodes never hide it: with a ring round each, its end
/// glyphs and its caps sit within its gap of the inks they stop at, and
/// are drawn.
///
/// Control: with `hide` not sparing the ends, red on the glyphs drawn.
#[test]
fn test_projection_a_paths_own_ends_never_hide_it() {
    let end = merged(json!({"min_size": [100.0, 40.0]}), json!({"frame": dotted(0.25)}));
    let caps = json!({"straight": true, "cap_start": {"spans": [{"text": "◆"}]}, "cap_end": {"spans": [{"text": "◆"}]}});
    let doc = load_map(json!({"nodes": [
        node(1, None, end.clone()),
        node(2, Some(1), merged(merged(end, json!({"position": [1000.0, 0.0]})), json!({"connector": caps}))),
    ]}));
    let mut p = Projected::new(doc);
    let path = path_of(&p, 2);
    let gap = own_gap(&p.projector, path);
    let ends = [1, 2].map(|n| region_of(&p, n));
    let laid = inks(p.projector.laid(path).unwrap());
    for (k, end) in ends.iter().enumerate() {
        assert!(
            laid.iter().any(|i| meet(&i.grown(gap * 1.25), end)),
            "precondition: its end glyph comes within its gap of end {k}'s ring, as rule 5 sets it (0.8 to 1.2 gaps)"
        );
    }
    assert_eq!(left_out(&p, path), 0, "every glyph of the body is drawn");
    let Some(UnitBody::Line(line)) = p.projector.units.get(&path).map(|u| &u.body) else {
        panic!("a line");
    };
    for k in [1, 2] {
        let cap = line.laid[k].as_ref().expect("a cap is laid");
        assert!(
            ends.iter().any(|end| inks(cap).iter().any(|i| meet(i, end))),
            "precondition: cap {k} sits on its end's ring"
        );
        assert_eq!(
            p.projector.units[&path].slots[k].want.as_ref(),
            Some(cap),
            "cap {k} is drawn whole"
        );
    }
    p.check().unwrap();
}

/// A node's rings hide as its text does: a path clear of its text by more
/// than its gap but over its ring is hidden there.
///
/// Control: with the region its text alone, red on the glyphs hidden.
#[test]
fn test_projection_a_nodes_ring_hides_a_path_as_its_text_does() {
    // The blocker's text runs from y = 26 down, past the gap of the path
    // along y = 20; its ring of dots, a little further out, reaches over it.
    let doc = two_paths(Vec2::new(480.0, 26.0), json!({"frame": dotted(0.25)}), json!({}));
    let p = Projected::new(doc);
    let path = path_of(&p, 2);
    let (text, rings) = drawn_by(&p, ident(&p.doc, 3));
    let (text, rings) = (text.unwrap(), rings.expect("the ring is drawn"));
    let gap = own_gap(&p.projector, path);
    let laid = inks(p.projector.laid(path).unwrap());
    assert!(
        !laid.iter().any(|i| meet(&i.grown(gap), &text)),
        "precondition: the path keeps its gap from the text"
    );
    assert!(
        rings.origin.y < text.origin.y,
        "precondition: the ring reaches above the text"
    );
    assert!(
        hides_exactly(&p, path, text.union(&rings)) >= 1,
        "the ring hides the path"
    );
}

/// An ellipse hides by its outline, from both sides: a ringed circle
/// centered on a diagonal path hides the glyphs that come within the
/// path's gap of the ellipse in its rings' box, and draws those inside
/// that box but clear of the ellipse, where the diagonal crosses the
/// box's corners. The ellipse is tested here on its own terms: the
/// nearest point of each grown ink to its center, inside its radii or out.
///
/// Controls: with the region shaped as a rectangle, red on the glyphs
/// drawn in the corners; with no region for an ellipse, red on the glyphs
/// hidden inside it.
#[test]
fn test_projection_an_ellipse_hides_by_its_outline() {
    let circle =
        json!({"min_size": [120.0, 120.0], "shape": "ellipse", "sections": [], "frame": dotted(0.05)});
    let doc = load_map(json!({"nodes": [
        node(1, None, json!({"min_size": [100.0, 40.0]})),
        node(2, Some(1), json!({"position": [900.0, 900.0], "min_size": [100.0, 40.0], "connector": {"straight": true}})),
        node(3, None, merged(json!({"position": [AWAY.x, AWAY.y]}), circle)),
    ]}));
    let mut p = Projected::new(doc);
    let path = path_of(&p, 2);
    let Some(&[Curve::Line(a, b)]) = p.projector.curve(path) else {
        panic!("a straight path");
    };
    move_to(&mut p, 3, (a + b) * 0.5 - Vec2::splat(60.0));
    let rings = region_of(&p, 3);
    let (center, half) = (rings.center(), rings.size * 0.5);
    let gap = own_gap(&p.projector, path);
    let drawn = inks(&drawn(&p, path));
    let (mut inside, mut corner) = (0, 0);
    for ink in inks(p.projector.laid(path).unwrap()) {
        let grown = ink.grown(gap);
        let nearest = center.clamp(grown.origin, grown.max());
        let reach = ((nearest - center) / half).length();
        if reach <= 0.95 {
            inside += 1;
            assert!(
                !drawn.contains(&ink),
                "a glyph at {} inside the ellipse is drawn",
                ink.center()
            );
        } else if reach >= 1.05 && meet(&grown, &rings) {
            corner += 1;
            assert!(
                drawn.contains(&ink),
                "a glyph at {} past the ellipse, in its box, is left out",
                ink.center()
            );
        }
    }
    assert!(
        inside >= 3,
        "precondition: the path runs through the ellipse, {inside} glyphs"
    );
    assert!(
        corner >= 1,
        "precondition: the path crosses the box's corners, {corner} glyphs"
    );
    p.check().unwrap();
}

/// A container — a node whose box holds a path's two nodes — hides the
/// path by what it draws: one that draws nothing hides nothing, so the path
/// is drawn whole; one with a ring round it hides the path whole, since the
/// box round its ring holds it.
///
/// Controls: with the region the node's drawn rectangle, red on the
/// glyphs left out by the empty container; with `hide` leaving every
/// glyph in, red on the glyphs drawn inside the ringed one.
#[test]
fn test_projection_a_container_hides_by_what_it_draws() {
    for (frame, whole) in [(Value::Null, true), (dotted(0.01), false)] {
        let mut container = json!({"position": [-200.0, -280.0], "min_size": [1600.0, 600.0]});
        if !frame.is_null() {
            container["frame"] = frame;
        }
        let doc = load_map(json!({"nodes": [
            node(1, None, json!({"min_size": [100.0, 40.0]})),
            node(2, Some(1), json!({"position": [1000.0, 0.0], "min_size": [100.0, 40.0], "connector": {"straight": true}})),
            node(3, None, container),
        ]}));
        let mut p = Projected::new(doc);
        let path = path_of(&p, 2);
        let laid = inks(p.projector.laid(path).unwrap()).len();
        assert!(laid >= 100, "precondition: the path is laid, {laid} glyphs");
        let drawn = inks(&drawn(&p, path)).len();
        if whole {
            assert_eq!(drawn, laid, "a container that draws nothing hides nothing");
        } else {
            assert_eq!(drawn, 0, "a ringed container hides the path inside it whole");
        }
        p.check().unwrap();
    }
}

/// A node hides a path only where it is drawn wherever the path is (the
/// owner's answer on #334): at opacity 0, or in a zoom window that does
/// not hold the path's, it hides nothing — a run has one zoom window, so
/// a glyph cannot be hidden only while the node shows. Half transparent,
/// or in a window holding the path's, it hides.
///
/// Both bounds of "holds" are tried. The node's opacity set to 0 shows
/// the glyphs it hid; its zoom window set to one that does not hold the
/// path's shows them, and taken away hides them again.
///
/// Controls: with `hide` heeding neither opacity nor zoom, red on the
/// nodes that hide nothing; with no window held, red on the window that
/// holds the path's; with `drawn_wherever`'s `max` side reading `min`,
/// red on a `max` case; with `hide` not reading the node's opacity, red
/// on the glyphs shown when it is set to 0; with `hide` not reading its
/// visibility, red on the glyphs shown when its zoom window is set.
#[test]
fn test_projection_a_node_hides_only_where_it_is_drawn() {
    let cases = [
        (json!({"opacity": 0.0}), json!({}), false),
        (json!({"zoom": {"min": 50.0}}), json!({}), false),
        (
            json!({"zoom": {"min": 0.5}}),
            json!({"zoom": {"min": 0.1}}),
            false,
        ),
        (json!({"opacity": 0.5}), json!({}), true),
        (
            json!({"zoom": {"min": 0.1}}),
            json!({"zoom": {"min": 0.5, "max": 20.0}}),
            true,
        ),
        (json!({"zoom": {"max": 5.0}}), json!({}), false),
        (
            json!({"zoom": {"max": 5.0}}),
            json!({"zoom": {"max": 10.0}}),
            false,
        ),
        (
            json!({"zoom": {"max": 10.0}}),
            json!({"zoom": {"max": 5.0}}),
            true,
        ),
    ];
    for (blocker, path_fields, hides) in cases {
        let mut p = Projected::new(two_paths(ON_P, blocker.clone(), path_fields.clone()));
        let path = path_of(&p, 2);
        if hides {
            assert!(
                hides_exactly(&p, path, region_of(&p, 3)) >= 3,
                "a node of {blocker} hides a path of {path_fields}"
            );
        } else {
            assert_eq!(
                left_out(&p, path),
                0,
                "a node of {blocker} hides nothing of a path of {path_fields}"
            );
        }
        p.check().unwrap();
    }
    let mut p = Projected::new(two_paths(ON_P, json!({}), json!({})));
    let path = path_of(&p, 2);
    assert!(left_out(&p, path) > 0, "precondition: the node hides glyphs");
    let mutation = single(DocKind::Node, DocMutation::Node(NodeOp::SetOpacity(0.0)));
    p.apply(request(mutation, vec![ident(&p.doc, 3)], "fade"))
        .unwrap();
    assert_eq!(left_out(&p, path), 0, "a node faded to nothing shows the path");
    p.check().unwrap();
    // The zoom half: a window set that does not hold the path's shows it,
    // and the window taken away hides it again.
    let mut p = Projected::new(two_paths(ON_P, json!({}), json!({})));
    let path = path_of(&p, 2);
    let hidden = left_out(&p, path);
    assert!(hidden > 0, "precondition: the node hides glyphs");
    let window = crate::document::values::ZoomWindow {
        min: Some(50.0),
        max: None,
    };
    for (zoom, want) in [(Some(window), 0), (None, hidden)] {
        let mutation = single(DocKind::Node, DocMutation::Node(NodeOp::SetZoom(zoom)));
        p.apply(request(mutation, vec![ident(&p.doc, 3)], "zoom"))
            .unwrap();
        assert_eq!(left_out(&p, path), want, "with the node's zoom window {zoom:?}");
        p.check().unwrap();
    }
}

/// A node that stops being drawn — hidden — shows the glyphs it hid.
///
/// Control: with `rehide` keeping a node that left in the regions' index,
/// red on the glyphs shown and the oracle.
#[test]
fn test_projection_a_node_hidden_shows_the_path_behind_it() {
    let mut p = Projected::new(two_paths(ON_P, json!({}), json!({})));
    let path = path_of(&p, 2);
    assert!(left_out(&p, path) > 0, "precondition: the node hides glyphs");
    let mutation = single(DocKind::Node, DocMutation::Node(NodeOp::SetHidden(true)));
    p.apply(request(mutation, vec![ident(&p.doc, 3)], "hide"))
        .unwrap();
    assert_eq!(left_out(&p, path), 0, "every glyph is drawn again");
    p.check().unwrap();
}

/// A path dropped with its node leaves the paths' index, and a node
/// deleted leaves the regions, as a full projection holds them: the
/// oracle compares both indices.
///
/// Control: with `drop_unit` not calling `unhide`, red on the oracle's
/// path cells.
#[test]
fn test_projection_a_deleted_path_and_node_leave_the_indices() {
    use crate::document::mutation::compose::delete;
    use crate::document::writer::EditRequest;
    let mut p = Projected::new(two_paths(ON_P, json!({}), json!({})));
    p.check().unwrap();
    for id in [2, 3] {
        let node = ident(&p.doc, id);
        let steps = delete(&p.doc, &[node]).unwrap();
        p.apply(EditRequest::new(steps, "delete")).unwrap();
        p.check().unwrap();
    }
}

/// Paths that leave one hub share its cells in the paths' index; one of
/// them hidden again leaves those cells and joins them anew, and the
/// others' entries there stay pointed at right: the oracle compares the
/// index, back-pointers and all, after the middle one is hidden and shown.
///
/// Control: with `PathCells::remove` not pointing the entry it moves at
/// its new place, red on the oracle's path cells.
#[test]
fn test_projection_paths_that_share_a_hubs_cells_are_hidden_one_by_one() {
    let child = |id: u32, y: f32| {
        node(
            id,
            Some(1),
            json!({"position": [1000.0, y], "min_size": [100.0, 40.0], "connector": {}}),
        )
    };
    let doc = load_map(json!({"nodes": [
        node(1, None, json!({"min_size": [100.0, 40.0]})),
        child(2, -300.0),
        child(3, 0.0),
        child(4, 300.0),
        node(5, None, json!({"position": [AWAY.x, AWAY.y], "min_size": [60.0, 60.0], "sections": blocker_text()})),
    ]}));
    let mut p = Projected::new(doc);
    let middle = path_of(&p, 3);
    move_to(&mut p, 5, ON_P);
    assert!(
        hidden_again(&p).contains(&middle),
        "precondition: the middle path is hidden again"
    );
    p.check().unwrap();
    move_to(&mut p, 5, AWAY);
    p.check().unwrap();
}

/// A node whose region spans more grid cells than hold any path glyph
/// finds the paths it meets by scanning those cells, not by walking its
/// own: a ringed node 3000 wide, moved onto P, hides it.
///
/// Control: with the scan finding nothing, red on the glyphs hidden.
#[test]
fn test_projection_a_node_wider_than_the_paths_cells_finds_them_by_scanning() {
    let wide = json!({"min_size": [3000.0, 3000.0], "sections": [], "frame": dotted(0.01)});
    let mut p = Projected::new(two_paths(Vec2::new(-1000.0, 3000.0), wide, json!({})));
    let path = path_of(&p, 2);
    move_to(&mut p, 3, Vec2::new(-1000.0, 10.0));
    assert!(
        hides_exactly(&p, path, region_of(&p, 3)) >= 3,
        "the wide node hides the path"
    );
    p.check().unwrap();
}

/// The box round a path's label's text's glyph cells, as laid.
fn label_of(p: &Projected, path: NodeId) -> Rect {
    let Some(UnitBody::Line(line)) = p.projector.units.get(&path).map(|u| &u.body) else {
        panic!("{path:?} is a line");
    };
    let run = line.laid[3].as_ref().expect("the label is laid");
    bounds(&cells(run)).expect("the label draws text")
}

/// The box a path's label's run spans as laid: its shaped box, spaces
/// and all.
fn label_box_of(p: &Projected, path: NodeId) -> Rect {
    let Some(UnitBody::Line(line)) = p.projector.units.get(&path).map(|u| &u.body) else {
        panic!("{path:?} is a line");
    };
    let run = line.laid[3].as_ref().expect("the label is laid");
    let RunShape::Box(shaped) = run.shaped() else {
        panic!("a box run");
    };
    Rect::new(run.origin(), shaped.size)
}

/// Whether the labels layer draws `path`'s label.
fn label_drawn(p: &Projected, path: NodeId) -> bool {
    !runs(p, path, Layer::Labels).is_empty()
}

/// A path's label, `text` at `t` along it, `offset` off it.
fn labeled(text: &str, t: f32, offset: f32) -> Value {
    json!({"label": {"t": t, "offset": offset, "spans": [{"text": text}]}})
}

/// A path's label that meets the region of a node the path does not end
/// at is hidden whole (the owner's answer on #334): the label of P, at its
/// middle, under the blocker's text, is not drawn; Q's, clear of every
/// node, is.
///
/// Control: with `hide` never hiding a label, red on the label drawn.
#[test]
fn test_projection_a_label_over_another_node_is_hidden_whole() {
    let mut p = Projected::new(two_paths(
        Vec2::new(520.0, 0.0),
        json!({}),
        labeled("LABEL", 0.5, 0.0),
    ));
    let path = path_of(&p, 2);
    let label = label_of(&p, path);
    assert!(
        meet(&label, &region_of(&p, 3)),
        "precondition: the label meets the blocker's text, {label:?}"
    );
    assert!(!label_drawn(&p, path), "the label behind the node is not drawn");
    assert!(
        label_drawn(&p, path_of(&p, 5)),
        "the label clear of every node is drawn"
    );
    p.check().unwrap();
}

/// A node moved off a label shows it again, and moved back onto it hides
/// it again, each time through the path's hide stage alone and with the
/// oracle agreeing: the label is found through the paths' index and read
/// through `Region`, as the path's glyphs are.
///
/// Control: with the label's box kept out of the paths' index, red on the
/// label drawn once the node is back on it.
#[test]
fn test_projection_a_label_shows_and_hides_as_a_node_moves() {
    // The label sits 40 above P, and the node over it, clear of the path's
    // glyphs: only the label brings the node and the path together.
    let start = Vec2::new(250.0, -50.0);
    let mut p = Projected::new(two_paths(start, json!({}), labeled("LABEL", 0.2, 40.0)));
    let path = path_of(&p, 2);
    let label = label_of(&p, path);
    assert_eq!(
        left_out(&p, path),
        0,
        "precondition: the node is clear of the path's glyphs"
    );
    assert!(
        meet(&label, &region_of(&p, 3)),
        "precondition: the label is under the node"
    );
    assert!(!label_drawn(&p, path), "precondition: the label is hidden");
    for (at, drawn) in [(Vec2::new(250.0, -400.0), true), (start, false)] {
        move_to(&mut p, 3, at);
        assert_eq!(label_drawn(&p, path), drawn, "the label with the node at {at}");
        assert!(
            hidden_again(&p).contains(&path),
            "the path is hidden again with the node at {at}"
        );
        p.check().unwrap();
    }
}

/// A path's label over one of its own two nodes is never hidden by it: a
/// label at the very start of a path, over the text of the node it starts
/// at, is drawn.
///
/// Control: with `hide` not sparing the ends, red on the label not drawn.
#[test]
fn test_projection_a_label_over_its_own_end_is_drawn() {
    let end = json!({"sections": blocker_text()});
    let doc = load_map(json!({"nodes": [
        node(1, None, end.clone()),
        node(2, Some(1), merged(merged(end, json!({"position": [1000.0, 0.0]})), json!({"connector": merged(json!({"straight": true}), labeled("LABEL", 0.0, 0.0))}))),
    ]}));
    let mut p = Projected::new(doc);
    let path = path_of(&p, 2);
    let label = label_of(&p, path);
    assert!(
        meet(&label, &region_of(&p, 1)),
        "precondition: the label meets its start node's text, {label:?}"
    );
    assert!(label_drawn(&p, path), "the label over its own end is drawn");
    p.check().unwrap();
}

/// A label is hidden only by a node drawn wherever the label is, as a
/// path's glyphs are: a blocker over P's label at opacity 0, or in a zoom
/// window that does not hold the path's, leaves it drawn; half
/// transparent, or in a window that holds the path's, it hides it. Both
/// bounds of "holds" are tried.
///
/// Control: with the label hidden by any region it meets, whatever its
/// node's opacity and zoom window, red on the label not drawn.
#[test]
fn test_projection_a_label_is_hidden_only_where_its_node_is_drawn() {
    let cases = [
        (json!({"opacity": 0.0}), json!({}), false),
        (json!({"opacity": 0.5}), json!({}), true),
        (json!({"zoom": {"min": 50.0}}), json!({}), false),
        (json!({"zoom": {"max": 5.0}}), json!({}), false),
        (
            json!({"zoom": {"max": 5.0}}),
            json!({"zoom": {"max": 10.0}}),
            false,
        ),
        (
            json!({"zoom": {"max": 10.0}}),
            json!({"zoom": {"max": 5.0}}),
            true,
        ),
    ];
    for (blocker, path_fields, hides) in cases {
        let fields = merged(path_fields.clone(), labeled("LABEL", 0.5, 0.0));
        let mut p = Projected::new(two_paths(Vec2::new(520.0, 0.0), blocker.clone(), fields));
        let path = path_of(&p, 2);
        assert!(
            meet(&label_of(&p, path), &region_of(&p, 3)),
            "precondition: the label meets the blocker's text"
        );
        assert_eq!(
            label_drawn(&p, path),
            !hides,
            "the label under a node of {blocker}, on a path of {path_fields}"
        );
        p.check().unwrap();
    }
}

/// A node over a label alone — clear of the path's glyphs — hides it and
/// shows it as its opacity and its zoom window change: set to opacity 0
/// the label is drawn, back to 1 it is hidden; given a zoom window that
/// does not hold the path's it is drawn, the window taken away it is
/// hidden. The oracle agrees after each step.
///
/// Control: with the label reading its node's region but not its opacity
/// and visibility, red on the label still hidden at opacity 0.
#[test]
fn test_projection_a_label_follows_its_nodes_opacity_and_zoom() {
    use crate::document::values::ZoomWindow;
    let start = Vec2::new(250.0, -50.0);
    let mut p = Projected::new(two_paths(start, json!({}), labeled("LABEL", 0.2, 40.0)));
    let path = path_of(&p, 2);
    let blocker = ident(&p.doc, 3);
    assert_eq!(
        left_out(&p, path),
        0,
        "precondition: the node is clear of the path's glyphs"
    );
    assert!(!label_drawn(&p, path), "precondition: the label is hidden");
    let window = ZoomWindow {
        min: Some(50.0),
        max: None,
    };
    let steps = [
        ("opacity 0", NodeOp::SetOpacity(0.0), true),
        ("opacity 1", NodeOp::SetOpacity(1.0), false),
        (
            "a window that does not hold the path's",
            NodeOp::SetZoom(Some(window)),
            true,
        ),
        ("no window", NodeOp::SetZoom(None), false),
    ];
    for (step, op, drawn) in steps {
        let mutation = single(DocKind::Node, DocMutation::Node(op));
        p.apply(request(mutation, vec![blocker], step)).unwrap();
        assert_eq!(label_drawn(&p, path), drawn, "the label with its node at {step}");
        p.check().unwrap();
    }
}

/// A label is hidden by the box round its text alone — not grown by the
/// path's gap, as a glyph's is, nor its shaped box with the spaces it
/// carries: a label with trailing spaces, its text clear of a node's text
/// by less than the path's gap and its spaces over the node, is drawn.
///
/// Controls: with the label's text box grown by the gap, red on the label
/// not drawn; with the label's whole shaped box, red on the label not
/// drawn.
#[test]
fn test_projection_a_label_touching_nothing_is_drawn() {
    let mut p = Projected::new(two_paths(AWAY, json!({}), labeled("LABEL     ", 0.5, 0.0)));
    let path = path_of(&p, 2);
    let label = label_of(&p, path);
    // The blocker's text starts at its node's corner: put it a point past
    // the label's last letter, across the label's line.
    move_to(&mut p, 3, Vec2::new(label.max().x + 1.0, label.origin.y - 10.0));
    let region = region_of(&p, 3);
    let clear = region.origin.x - label.max().x;
    let gap = own_gap(&p.projector, path);
    assert!(
        clear > 0.0 && clear < gap,
        "precondition: the label's text is clear of the node by less than the gap, {clear} of {gap}"
    );
    assert!(
        meet(&label_box_of(&p, path), &region),
        "precondition: the label's spaces reach over the node"
    );
    assert!(
        region.origin.y < label.max().y && label.origin.y < region.max().y,
        "precondition: the node is across the label's line"
    );
    assert!(label_drawn(&p, path), "the label touching nothing is drawn");
    p.check().unwrap();
}
