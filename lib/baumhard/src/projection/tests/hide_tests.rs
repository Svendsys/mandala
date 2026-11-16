// SPDX-License-Identifier: MPL-2.0

//! Nodes hide the paths behind them (#332): a connector's glyph is left out
//! where it would come within its own gap of what a node it does not end at
//! draws — its text and its rings — and its label, whole, where the box
//! round the label's text meets it; both come back when the node moves
//! away, and a move re-hides only the paths whose glyphs or label the
//! node's old or new region meets. A path's own label, drawn, is held the
//! same way (#340): the path's glyphs within its gap of the label's text,
//! its caps' among them, are left out, and they follow the label as it is
//! moved, re-texted, removed or hidden. Glyph cells and inks are read from
//! the runs' layouts and metrics here, not through the projector's own
//! reading of them, and every test ends on the oracle.

use glam::Vec2;
use rustc_hash::FxHashSet;
use serde_json::{json, Value};

use crate::arena::NodeId;
use crate::document::element::DocKind;
use crate::document::mutation::ops::{DocMutation, FrameOp, NodeOp};
use crate::document::tests::writer_support::{request, single};
use crate::flow::{Curve, Rect};
use crate::gfx_structs::run::{GlyphRunData, RunShape};
use crate::projection::behind::PathCells;
use crate::projection::hit::{cell_count, cells as grid_cells, span};
use crate::projection::state::UnitBody;
use crate::projection::tests::maps::{
    blocker_text, grown_boxes, huge_box, ident, kid, load_map, merged, node, two_paths, AWAY, HUGE_LENGTHS,
    ON_P, ON_Q,
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

/// A text path's glyphs hide behind a node as an ornament's do, though
/// their inks are not read: each is its advance by the line height.
///
/// Control: with `glyph_ink` giving a glyph whose ink was not read no
/// box, red on the text path drawn whole across the node.
#[test]
fn test_projection_a_text_path_hides_behind_a_node() {
    let text = json!({"fit": "repeat", "spans": [{"text": "words along a path "}]});
    let mut p = Projected::new(two_paths(ON_P, json!({}), text));
    let path = path_of(&p, 2);
    let count = |run: Option<&GlyphRunData>| run.map_or(0, |r| r.shaped().glyph_count());
    let laid = count(p.projector.laid(path));
    let shown = count(Some(&drawn(&p, path)));
    assert!(laid >= 20, "precondition: the text path is laid, {laid} glyphs");
    assert!(
        shown < laid,
        "the text path's glyphs behind the node are hidden: {shown} of {laid} drawn"
    );
    let other = path_of(&p, 5);
    assert_eq!(
        count(Some(&drawn(&p, other))),
        count(p.projector.laid(other)),
        "the text path clear of the node is drawn whole"
    );
    p.check().unwrap();
}

/// A path's own glyphs stop one gap short of its own label's text, on
/// each side, and carry on past it; the label is drawn in full (#340:
/// text wins over a line). The path the label is not on is untouched by
/// it.
///
/// Control: with the hide stage ignoring the path's own label, red on the
/// glyphs left out.
#[test]
fn test_projection_a_path_stops_short_of_its_own_label() {
    let mut p = Projected::new(two_paths(AWAY, json!({}), labeled("LABEL", 0.5, 0.0)));
    let path = path_of(&p, 2);
    let label = label_of(&p, path);
    assert!(label_drawn(&p, path), "the label is drawn in full");
    let hidden = hides_exactly(&p, path, label);
    assert!(
        hidden >= 3,
        "the path stops short of its label, {hidden} glyphs left out"
    );
    let shown = inks(&drawn(&p, path));
    for (side, x) in [("left", label.origin.x), ("right", label.max().x)] {
        assert!(
            shown.iter().any(|i| (i.center().x < x) == (side == "left")),
            "the path carries on past its label's {side} side"
        );
    }
    p.check().unwrap();
}

/// A text path stops short of its own label as an ornament path does: its
/// letters, boxed by their advance and the line height, are left out
/// within its gap of the label's text, and the label is drawn.
///
/// Control: with the hide stage ignoring the path's own label, red on the
/// text path drawn whole.
#[test]
fn test_projection_a_text_path_stops_short_of_its_own_label() {
    let text = merged(
        json!({"fit": "repeat", "spans": [{"text": "words along a path "}]}),
        labeled("LABEL", 0.5, 0.0),
    );
    let mut p = Projected::new(two_paths(AWAY, json!({}), text));
    let path = path_of(&p, 2);
    let count = |run: Option<&GlyphRunData>| run.map_or(0, |r| r.shaped().glyph_count());
    let laid = count(p.projector.laid(path));
    let shown = count(Some(&drawn(&p, path)));
    assert!(label_drawn(&p, path), "the label is drawn");
    assert!(
        shown < laid,
        "the text path stops short of its label: {shown} of {laid} drawn"
    );
    p.check().unwrap();
}

/// A path's own glyphs follow its label as it changes, each time through
/// that path's own hide stage alone and with the oracle agreeing: the
/// label moved along the path (`SetT`), moved off it (`SetOffset`), its
/// text edited, the path's gap widened (`SetSpacing`), and the label
/// removed.
///
/// Control: with the hide stage ignoring the path's own label, red on the
/// glyphs left out where the label stands.
#[test]
fn test_projection_a_path_follows_its_own_label() {
    use crate::document::mutation::compose::remove;
    use crate::document::mutation::ops::{LabelOp, PathOp, SpanOp};
    use crate::document::writer::EditRequest;
    let mut p = Projected::new(two_paths(AWAY, json!({}), labeled("LABEL", 0.5, 0.0)));
    let path = path_of(&p, 2);
    let label = kid(&p.doc, path, DocKind::Label);
    let span = kid(&p.doc, label, DocKind::Span);
    let steps: [(&str, DocKind, NodeId, DocMutation, bool); 4] = [
        (
            "moved along",
            DocKind::Label,
            label,
            DocMutation::Label(LabelOp::SetT(Some(0.3))),
            true,
        ),
        (
            "moved off",
            DocKind::Label,
            label,
            DocMutation::Label(LabelOp::SetOffset(Some(40.0))),
            false,
        ),
        (
            "back on, its text longer",
            DocKind::Label,
            label,
            DocMutation::Label(LabelOp::SetOffset(Some(0.0))),
            true,
        ),
        (
            "the gap widened",
            DocKind::Connector,
            path,
            DocMutation::Path(PathOp::SetSpacing(Some(6.0))),
            true,
        ),
    ];
    for (step, kind, element, mutation, on) in steps {
        p.apply(request(single(kind, mutation), vec![element], step))
            .unwrap();
        if step.starts_with("back on") {
            let edit = single(
                DocKind::Span,
                DocMutation::Span(SpanOp::SetText("A LONGER LABEL".into())),
            );
            p.apply(request(edit, vec![span], "longer")).unwrap();
        }
        let hidden = hides_exactly(&p, path, label_of(&p, path));
        assert_eq!(hidden > 0, on, "{step}: {hidden} glyphs left out by the label");
        assert_eq!(
            hidden_again(&p),
            [path].into_iter().collect(),
            "{step}: that path alone is hidden again"
        );
        p.check().unwrap();
    }
    let steps = remove(&p.doc, DocKind::Label, &[label]).unwrap();
    p.apply(EditRequest::new(steps, "remove the label")).unwrap();
    assert_eq!(left_out(&p, path), 0, "the label removed, the path is whole");
    assert_eq!(
        hidden_again(&p),
        [path].into_iter().collect(),
        "the label removed: that path alone"
    );
    p.check().unwrap();
}

/// A label hidden whole behind another node hides nothing of its own
/// path: the node over the label's top, clear of the path's glyphs, hides
/// the label, and the path runs on under where the label would be.
///
/// Control: with a hidden label still hiding its path's glyphs, red on
/// the glyphs left out.
#[test]
fn test_projection_a_hidden_label_hides_nothing_of_its_path() {
    let mut p = Projected::new(two_paths(AWAY, json!({}), labeled("LABEL", 0.5, 0.0)));
    let path = path_of(&p, 2);
    let label = label_of(&p, path);
    // The blocker's text, 36 tall, its bottom 2 into the label's top.
    move_to(&mut p, 3, Vec2::new(label.origin.x, label.origin.y + 2.0 - 36.0));
    let region = region_of(&p, 3);
    let gap = own_gap(&p.projector, path);
    let laid = inks(p.projector.laid(path).unwrap());
    assert!(meet(&region, &label), "precondition: the node meets the label");
    assert!(
        !laid.iter().any(|i| meet(&i.grown(gap), &region)),
        "precondition: the node is clear of the path's glyphs"
    );
    assert!(!label_drawn(&p, path), "the label is hidden");
    assert_eq!(left_out(&p, path), 0, "the path runs on under its hidden label");
    p.check().unwrap();
}

/// The two rules meet as a node moves: a node moved onto a path's label,
/// clear of the path's glyphs, hides the label, and the glyphs the label
/// held back come back; moved away, the label is drawn and they are left
/// out again — twice. Then, the node on the label, its opacity set to 0
/// and back to 1, and its zoom window set to one that does not hold the
/// path's and taken away, flip the label and those glyphs together. The
/// oracle agrees after every step.
///
/// Control: with the path's glyphs giving way to its label whether it is
/// drawn or hidden, red on the glyphs left out under the hidden label.
#[test]
fn test_projection_a_path_follows_its_label_hidden_and_shown() {
    use crate::document::values::ZoomWindow;
    let mut p = Projected::new(two_paths(AWAY, json!({}), labeled("LABEL", 0.5, 0.0)));
    let path = path_of(&p, 2);
    let blocker = ident(&p.doc, 3);
    let label = label_of(&p, path);
    let held = left_out(&p, path);
    assert!(label_drawn(&p, path), "precondition: the label is drawn");
    assert!(held > 0, "precondition: the path stops short of its label");
    // The blocker's text, 36 tall, its bottom 2 into the label's top.
    let on = Vec2::new(label.origin.x, label.origin.y + 2.0 - 36.0);
    // Whether the label is drawn and the path stops short of it, or it is
    // hidden and the path runs on under it.
    let holds = |p: &Projected, drawn: bool, step: &str| {
        assert_eq!(label_drawn(p, path), drawn, "the label drawn, {step}");
        let want = if drawn { held } else { 0 };
        assert_eq!(left_out(p, path), want, "the path's glyphs left out, {step}");
    };
    for round in 0..2 {
        move_to(&mut p, 3, on);
        if round == 0 {
            let gap = own_gap(&p.projector, path);
            let region = region_of(&p, 3);
            let laid = inks(p.projector.laid(path).unwrap());
            assert!(meet(&region, &label), "precondition: the node meets the label");
            assert!(
                !laid.iter().any(|i| meet(&i.grown(gap), &region)),
                "precondition: the node is clear of the path's glyphs"
            );
        }
        holds(&p, false, &format!("the node moved onto it, round {round}"));
        p.check().unwrap();
        move_to(&mut p, 3, AWAY);
        holds(&p, true, &format!("the node moved away, round {round}"));
        p.check().unwrap();
    }
    move_to(&mut p, 3, on);
    holds(&p, false, "the node back on it");
    p.check().unwrap();
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
        holds(&p, drawn, &format!("the node at {step}"));
        p.check().unwrap();
    }
}

/// The cap of `path` in slot `k` (1 its start cap, 2 its end cap) as
/// laid, every glyph.
fn cap_laid(p: &Projected, path: NodeId, k: usize) -> GlyphRunData {
    let Some(UnitBody::Line(line)) = p.projector.units.get(&path).map(|u| &u.body) else {
        panic!("{path:?} is a line");
    };
    line.laid[k].clone().expect("the cap is laid")
}

/// The run the connections layer holds for the cap of `path` in slot
/// `k`, `None` where it holds no element for it.
fn cap_drawn(p: &Projected, path: NodeId, k: usize) -> Option<GlyphRunData> {
    let slot = &p.projector.units[&path].slots[k];
    let glyph = slot.glyph?;
    p.layers.tree(slot.layer).arena()[glyph]
        .get()
        .glyph_run()
        .cloned()
}

/// A path's own label hides the cap under it as it hides the rest of its
/// line (the owner's answer on #345: the label hides the arrowhead). With
/// the label at the path's end (`t = 1.0`), over its end cap, the end cap
/// is not drawn, and the start cap, clear of the label, is drawn whole.
/// The label moved to the path's middle, the end cap is drawn whole
/// again.
///
/// Control: with the path's own label sparing its caps
/// (`own_label.filter(|_| k == 0)` in `hide`), red on the end cap drawn
/// under the label.
#[test]
fn test_projection_a_paths_own_label_hides_the_cap_under_it() {
    use crate::document::mutation::ops::LabelOp;
    let path = merged(
        labeled("LABEL", 1.0, 0.0),
        json!({
            "cap_start": {"spans": [{"text": "◀"}]},
            "cap_end": {"spans": [{"text": "▶"}]},
        }),
    );
    let mut p = Projected::new(two_paths(AWAY, json!({}), path));
    let path = path_of(&p, 2);
    let gap = own_gap(&p.projector, path);
    // How many of the cap's inks in slot `k` come within the path's gap
    // of its label's text, the gap scaled by `slack`, of how many.
    let near = |p: &Projected, k: usize, slack: f32| {
        let label = label_of(p, path);
        let cap = inks(&cap_laid(p, path, k));
        assert!(!cap.is_empty(), "precondition: the cap in slot {k} has ink");
        let near = cap.iter().filter(|i| meet(&i.grown(gap * slack), &label));
        (near.count(), cap.len())
    };
    let drawn_inks = |p: &Projected, k: usize| cap_drawn(p, path, k).map_or(0, |run| inks(&run).len());

    assert!(label_drawn(&p, path), "precondition: the label is drawn");
    let (under, of) = near(&p, 2, 0.95);
    assert_eq!(under, of, "precondition: the end cap is under the label");
    assert_eq!(
        near(&p, 1, 1.05).0,
        0,
        "precondition: the start cap is clear of the label"
    );
    assert_eq!(
        drawn_inks(&p, 2),
        0,
        "the end cap under the path's own label is not drawn"
    );
    assert_eq!(
        cap_drawn(&p, path, 1),
        Some(cap_laid(&p, path, 1)),
        "the start cap, clear of the label, is drawn whole"
    );
    assert!(left_out(&p, path) > 0, "the body stops short of the label");
    p.check().unwrap();

    let label = kid(&p.doc, path, DocKind::Label);
    let to_the_middle = single(DocKind::Label, DocMutation::Label(LabelOp::SetT(Some(0.5))));
    p.apply(request(to_the_middle, vec![label], "to the middle"))
        .unwrap();
    assert!(label_drawn(&p, path), "precondition: the label is still drawn");
    for k in [1, 2] {
        assert_eq!(
            near(&p, k, 1.05).0,
            0,
            "precondition: the cap in slot {k} is clear of the label at the middle"
        );
        assert_eq!(
            cap_drawn(&p, path, k),
            Some(cap_laid(&p, path, k)),
            "the label moved off it, the cap in slot {k} is drawn whole"
        );
    }
    assert_eq!(
        hidden_again(&p),
        [path].into_iter().collect(),
        "the label moved: that path alone is hidden again"
    );
    p.check().unwrap();
}

/// The paths `index` passes for `rect`, in ascending id: one entry for
/// each cell a path is found in, and one where it is kept apart.
fn meeting(index: &PathCells, rect: Rect) -> Vec<NodeId> {
    let mut found = Vec::new();
    index.each_meeting(rect, |path| found.push(path));
    found.sort_unstable();
    found
}

/// A path with a glyph box past the cells — a glyph grown by a `spacing`
/// a map could carry, 4·10¹¹ to 4·10³⁷ — is kept apart, as the body index
/// keeps a body (#364): a region anywhere inside that box finds it, one
/// outside does not, and it leaves the index whole.
///
/// Controls: with the cells' count the product of two spans in `i64`, as
/// it was, a debug build panics at the first box set, and a release
/// build, which wraps the product, is red on the precondition that the
/// box lists no cells (past which the index would file it under every
/// one). And each rule of the index broken in turn: a box past the cells
/// dropped; the path's other boxes dropped with it; a path kept apart
/// passed without its box being tried against the region; a path kept
/// apart left behind when it is taken out, or its cells left behind.
/// Each is red on the assertion it is named in.
#[test]
fn test_projection_a_glyph_box_past_the_cells_is_found_by_every_region_inside_it() {
    let doc = load_map(json!({"nodes": [node(1, None, json!({})), node(2, None, json!({}))]}));
    let (wide, near) = (ident(&doc, 1), ident(&doc, 2));
    let sorted = |mut paths: Vec<NodeId>| {
        paths.sort_unstable();
        paths
    };
    let glyph = Rect::new(Vec2::new(500.0, 20.0), Vec2::new(8.0, 2.0));
    let far = Rect::new(Vec2::new(3.0e5, -2.0e5), Vec2::splat(50.0));
    for length in HUGE_LENGTHS {
        let grown = glyph.grown(length);
        assert_eq!(
            grid_cells(grown),
            None,
            "precondition: the glyph grown by {length} lists no cells; listed, it would be filed under every one"
        );
        let mut index = PathCells::default();
        index.set(near, &[glyph]);
        index.set(wide, &[glyph, grown]);
        assert_eq!(
            meeting(&index, far),
            vec![wide],
            "a region far from the glyph, inside its box grown by {length}"
        );
        assert_eq!(
            meeting(&index, glyph),
            sorted(vec![wide, wide, near]),
            "a region at the glyph: the wide path's glyph and its grown box, and the other path"
        );
        let beyond = Rect::new(Vec2::splat(length * 4.0), Vec2::splat(50.0));
        assert_eq!(
            meeting(&index, beyond),
            vec![],
            "a region outside the box grown by {length}"
        );
        index.remove(wide);
        assert_eq!(meeting(&index, far), vec![], "the wide path has left");
        assert_eq!(meeting(&index, glyph), vec![near], "and the other stays");
    }
}

/// A region past the cells finds the paths it meets by scanning the cells
/// that hold a path, never by walking its own (#364): the box round a
/// ring set out by each length a map could carry finds the five paths
/// near the origin, and the one 3·10¹² out only once it reaches it.
///
/// Controls: as the test above's. A debug build panics at the first
/// region, and a release build is red on the precondition that the region
/// spans more cells than hold a path (past which it would walk every one
/// of its own). With the scan passing nothing, or passing every path
/// without trying its box against the region, red on the paths met.
#[test]
fn test_projection_a_region_past_the_cells_scans_the_cells_that_hold_a_path() {
    let nodes: Vec<Value> = (1..=6).map(|id| node(id, None, json!({}))).collect();
    let doc = load_map(json!({ "nodes": nodes }));
    let paths: Vec<NodeId> = (1..=6).map(|id| ident(&doc, id)).collect();
    let mut index = PathCells::default();
    let mut boxes = Vec::new();
    for (k, &path) in paths.iter().enumerate() {
        // One box a path, in a cell of its own: five four cells apart, and
        // the last far out, where only the larger regions reach.
        let at = if k < 5 {
            Vec2::new(k as f32 * 1024.0 + 10.0, 10.0)
        } else {
            Vec2::splat(3.0e12)
        };
        let glyph = Rect::new(at, Vec2::splat(20.0));
        index.set(path, &[glyph]);
        boxes.push(glyph);
    }
    let mut regions: Vec<Rect> = HUGE_LENGTHS.into_iter().map(huge_box).collect();
    regions.push(Rect::new(Vec2::splat(-f32::MAX / 2.0), Vec2::splat(f32::MAX)));
    let mut reached = 0;
    for region in regions {
        let (from, to) = span(region).expect("a finite region has a span");
        assert!(
            cell_count(from, to) > 6,
            "precondition: {region:?} spans more cells than the six that hold a path; fewer, it would walk its own"
        );
        let mut want: Vec<NodeId> = paths
            .iter()
            .zip(&boxes)
            .filter(|(_, glyph)| meet(glyph, &region))
            .map(|(path, _)| *path)
            .collect();
        want.sort_unstable();
        reached += usize::from(want.len() == 6);
        assert_eq!(meeting(&index, region), want, "the paths {region:?} meets");
    }
    assert!(
        (1..12).contains(&reached),
        "precondition: the far path is inside some regions and outside others, {reached} of 12"
    );
}

/// How many glyphs of `path`'s body its layer draws: none where every one
/// is hidden, and the layer holds no run for it.
fn drawn_glyphs(p: &Projected, path: NodeId) -> usize {
    runs(p, path, Layer::Connections)
        .iter()
        .map(|run| run.shaped().glyph_count())
        .sum()
}

/// How many glyphs of `path`'s body are laid.
fn laid_glyphs(p: &Projected, path: NodeId) -> usize {
    p.projector.laid(path).map_or(0, |run| run.shaped().glyph_count())
}

/// The rings part of node `id`'s region, as the region index holds it.
fn rings_region(p: &Projected, id: u32) -> Option<Rect> {
    p.projector.regions.get(ident(&p.doc, id)).1.map(|(rect, _)| rect)
}

/// Set the `gap` of node `id`'s frame.
fn set_gap(p: &mut Projected, id: u32, gap: f32) {
    let frame = kid(&p.doc, ident(&p.doc, id), DocKind::Frame);
    let mutation = single(DocKind::Frame, DocMutation::Frame(FrameOp::SetGap(Some(gap))));
    p.apply(request(mutation, vec![frame], "gap")).unwrap();
}

/// A node whose ring is set far enough out that its box is past the cells
/// projects, and hides by that box as any ringed node does (#364). Its
/// ring is the last of 601, its dots spaced 10⁹ so that they run all the
/// way round it. With the rings drawn in, both paths are clear of it and
/// drawn whole. With them a `gap` of 10⁹ apart — the most a length is held
/// to wherever the loader bounds one — the ring's box is 1.2·10¹² across:
/// the region set again finds both paths, which no longer lie in its
/// cells but among the paths it scans, and each, inside the ring, is
/// hidden whole. The node moved, they stay hidden; the rings drawn in
/// again, they are shown.
///
/// Controls: with the cells' count the product of two spans in `i64`, as
/// it was, a debug build panics where the gap is set, setting the node's
/// region, and a release build files the region under every cell it
/// covers until an allocation fails, which ends the process. With a
/// region past the cells scanning no path, red on the paths hidden again;
/// with a query not trying the bodies kept apart, red on the glyphs drawn
/// inside the ring; with a body set again not taken out first, red on the
/// oracle after the gap is set; with a body kept apart left in its list
/// when it is set again, red on the oracle after the move and, that one
/// taken out, on the last; with a region set again not marking the paths
/// that read it, red on the paths shown.
#[test]
fn test_projection_a_ring_past_the_cells_hides_the_paths_inside_it() {
    let mut rings: Vec<Value> = (0..600).map(|_| json!({})).collect();
    rings.push(json!({"fit": "repeat", "size": 0.25, "spacing": 1.0e9, "spans": [{"text": "·"}]}));
    let ringed = json!({"frame": {"gap": 0.0, "rings": rings}});
    let mut p = Projected::new(two_paths(AWAY, ringed, json!({})));
    let paths = [path_of(&p, 2), path_of(&p, 5)];
    let near = rings_region(&p, 3).expect("the ring is laid, and is the node's region");
    assert!(
        grid_cells(near).is_some(),
        "precondition: drawn in, the region lists its cells, {near:?}"
    );
    for path in paths {
        let laid = laid_glyphs(&p, path);
        assert!(laid >= 20, "precondition: the path is laid, {laid} glyphs");
        assert_eq!(
            drawn_glyphs(&p, path),
            laid,
            "precondition: clear of the ring, the path is drawn whole"
        );
    }

    set_gap(&mut p, 3, 1.0e9);
    let far = rings_region(&p, 3).expect("the ring is laid 600 gaps out");
    assert!(
        far.size.min_element() > 1.0e12,
        "precondition: the ring's box is 600 gaps out each way, {far:?}"
    );
    assert_eq!(
        hidden_again(&p),
        FxHashSet::from_iter(paths),
        "the region set again finds both paths"
    );
    for path in paths {
        assert_eq!(drawn_glyphs(&p, path), 0, "inside the ring, it is hidden whole");
    }
    p.check().unwrap();

    move_to(&mut p, 3, AWAY + Vec2::new(1.0e8, 0.0));
    let moved = rings_region(&p, 3).expect("the ring is laid where the node went");
    assert!(
        moved.origin.x > far.origin.x,
        "precondition: the region moved with the node, {far:?} to {moved:?}"
    );
    p.check().unwrap();

    set_gap(&mut p, 3, 0.0);
    for path in paths {
        assert_eq!(
            drawn_glyphs(&p, path),
            laid_glyphs(&p, path),
            "clear of the ring again, the path is drawn whole"
        );
    }
    p.check().unwrap();
}

/// A connector whose `spacing` puts its glyphs' boxes past the cells —
/// each glyph is hidden by its box grown by the path's gap — projects,
/// and is hidden by a node that far from it as by one across it (#364):
/// with a node 1000 away the path is hidden whole; that node hidden, it is
/// drawn whole; shown again, the node's region finds the path, kept apart
/// from the cells, and the oracle holds it hidden again. At each length a
/// map could carry, 4·10¹¹ to 4·10³⁷.
///
/// Controls: as the test above's. A debug build panics in the first
/// projection, hiding the path, and a release build walks every cell the
/// glyph's box covers there, which does not end (stopped after 300 s).
/// With a query past the cells handed no body, red on the glyphs drawn
/// within the gap; with the paths kept apart not tried by a region, red
/// on the path found and, that assertion taken out, on the oracle.
#[test]
fn test_projection_a_path_spaced_past_the_cells_is_hidden_by_any_node_in_its_gap() {
    for spacing in HUGE_LENGTHS {
        let mut p = Projected::new(two_paths(AWAY, json!({}), json!({"spacing": spacing})));
        let path = path_of(&p, 2);
        let laid = laid_glyphs(&p, path);
        assert!(
            laid >= 1,
            "precondition: a path spaced {spacing} lays a glyph, {laid}"
        );
        assert_eq!(
            drawn_glyphs(&p, path),
            0,
            "within its gap of a node it does not end at, spaced {spacing}"
        );

        let blocker = ident(&p.doc, 3);
        let hide = |hidden| single(DocKind::Node, DocMutation::Node(NodeOp::SetHidden(hidden)));
        p.apply(request(hide(true), vec![blocker], "hide")).unwrap();
        assert_eq!(
            drawn_glyphs(&p, path),
            laid,
            "precondition: the node hidden, the path spaced {spacing} is drawn whole"
        );
        p.apply(request(hide(false), vec![blocker], "show")).unwrap();
        assert!(
            hidden_again(&p).contains(&path),
            "the node's region finds the path spaced {spacing}, kept apart from the cells"
        );
        p.check().unwrap();
    }
}

/// A connector spaced so far that its glyphs' boxes, grown by its gap,
/// are past what an `f32` holds — 1.8·10³⁸ and `f32::MAX`, which the
/// loader lets through — has boxes with no far corner: `Rect::grown` gives
/// each an origin of −∞ and a size of +∞, which sum to no number. Such a
/// box meets no node's region, as the paths' index leaves it out (#364):
/// the path is drawn whole beside a node 1000 away — one with only text,
/// one ringed, one round and ringed — and with that node across it, and
/// an update holds what a full projection does once the node is hidden,
/// shown again and moved across the path. The finite edge beside them,
/// 1.6·10³⁸, is hidden by each node, as a path spaced past the cells is
/// (the test above).
///
/// Controls: with such a box tried against the regions, as it was, the
/// round node's region panics on it (`f32::clamp` of a box whose far
/// corner is no number), and the others hide it in a full projection, red
/// on the path drawn whole; that assertion taken out, red on the oracle
/// after the node is shown again, whose update does not find the path.
#[test]
fn test_projection_a_path_spaced_past_what_a_box_holds_is_hidden_by_no_node() {
    let blockers = [
        ("a node with text", json!({})),
        ("a ringed node", json!({"frame": dotted(0.05)})),
        (
            "a round ringed node",
            json!({"shape": "ellipse", "frame": dotted(0.05)}),
        ),
    ];
    let hide = |hidden| single(DocKind::Node, DocMutation::Node(NodeOp::SetHidden(hidden)));
    for (what, blocker) in blockers {
        // Each spacing, and whether its boxes have a far corner.
        for (spacing, finite) in [(1.6e38, true), (1.8e38, false), (f32::MAX, false)] {
            let mut p = Projected::new(two_paths(AWAY, blocker.clone(), json!({"spacing": spacing})));
            let path = path_of(&p, 2);
            let laid = laid_glyphs(&p, path);
            assert!(
                laid >= 1,
                "precondition: a path spaced {spacing} lays a glyph, {laid}"
            );
            let boxes = grown_boxes(&p, path);
            assert!(
                !boxes.is_empty() && boxes.iter().all(|b| b.max().is_finite() == finite),
                "precondition: spaced {spacing}, the glyphs' boxes have a far corner: {finite}, {boxes:?}"
            );
            let want = if finite { 0 } else { laid };
            assert_eq!(
                drawn_glyphs(&p, path),
                want,
                "{what} 1000 away, the path spaced {spacing}: glyphs drawn"
            );
            p.check().unwrap();
            let node = ident(&p.doc, 3);
            p.apply(request(hide(true), vec![node], "hide")).unwrap();
            assert_eq!(
                drawn_glyphs(&p, path),
                laid,
                "precondition: {what} hidden, the path spaced {spacing} is drawn whole"
            );
            p.apply(request(hide(false), vec![node], "show")).unwrap();
            p.check().unwrap();
            move_to(&mut p, 3, ON_P);
            assert_eq!(
                drawn_glyphs(&p, path),
                want,
                "{what} across the path spaced {spacing}: glyphs drawn"
            );
            p.check().unwrap();
        }
    }
}

/// The frame of `maps/theme_demo.mandala.json`'s first node, as shipped:
/// a directional ring, a corner glyph at each corner of its outline.
const SHIPPED_FRAME: &str = r#""frame": {"inset": 7.0, "rings": [{"fit": "repeat", "size": 0.175, "color": "var(accent)", "spans": [{"text": "─"}], "vertical": "│", "corners": ["┌", "┐", "┘", "└"]}]}"#;

/// The maps #364 was found on project: `theme_demo` with its first node's
/// frame set out by each inset the defect was seen at — the shipped ring
/// at nine insets from 5·10¹¹ to 10³⁷, and at 10²⁵ a plain ring spaced
/// 10²⁵ and a frame's text — and the directional ring of 4·10³⁷ found
/// beside them. The ring's box, where it is laid, is the node's region
/// and reaches its inset out, and a rubber band as large covers every
/// node; the first map's node is then moved, and the oracle holds the
/// region set again.
///
/// A ring is laid round each of these outlines today. One round an
/// outline too long to lay draws nothing (ruling B42.1 on #363), so it
/// has no region: that is allowed only of the insets whose outline is
/// past that length, and the five up to 10¹⁶ must have theirs.
///
/// Controls: with the cells' count the product of two spans in `i64`, as
/// it was, a debug build panics in the first map's projection, setting
/// the node's region (attempt to multiply with overflow), and a release
/// build files the region under every cell it covers until an allocation
/// fails, which ends the process. With a query past the cells handed no
/// body, red on the rubber band; with a body kept apart left in its list
/// when it is set again, red on the oracle.
#[test]
fn test_projection_the_frame_insets_of_364_project() {
    let path = crate::util::doc_fixtures::repo_path("maps/theme_demo.mandala.json");
    let text = std::fs::read_to_string(path).expect("the shipped map is read");
    assert!(
        text.contains(SHIPPED_FRAME),
        "precondition: the map holds the frame the cases replace"
    );
    let ring = |inset: &str, keys: &str| {
        format!(
            r#""frame": {{"inset": {inset}, "rings": [{{"size": 0.175, "color": "var(accent)", {keys}}}]}}"#
        )
    };
    let mut cases: Vec<(String, f32, String)> = Vec::new();
    for (inset, value) in [
        ("5e11", 5.0e11),
        ("6e11", 6.0e11),
        ("1e12", 1.0e12),
        ("1e13", 1.0e13),
        ("1e16", 1.0e16),
        ("1e19", 1.0e19),
        ("1e25", 1.0e25),
        ("1e30", 1.0e30),
        ("1e37", 1.0e37),
    ] {
        cases.push((
            format!("the shipped ring, inset {inset}"),
            value,
            SHIPPED_FRAME.replace("7.0", inset),
        ));
    }
    cases.push((
        "a ring of ─ spaced 1e25, inset 1e25".to_owned(),
        1.0e25,
        ring(
            "1e25",
            r#""fit": "repeat", "spacing": 1e25, "spans": [{"text": "─"}]"#,
        ),
    ));
    cases.push((
        "a frame's text, inset 1e25".to_owned(),
        1.0e25,
        ring("1e25", r#""fit": "once", "spans": [{"text": "MANDALA"}]"#),
    ));
    cases.push((
        "a directional ring of ab, inset 4e37".to_owned(),
        4.0e37,
        ring(
            "4e37",
            r#""fit": "repeat", "spacing": 0, "spans": [{"text": "ab"}], "vertical": "│""#,
        ),
    ));
    for (k, (name, inset, frame)) in cases.iter().enumerate() {
        let map = text.replacen(SHIPPED_FRAME, frame, 1);
        let doc = crate::document::format::load(&map).unwrap_or_else(|e| panic!("{name} loads: {e}"));
        let mut p = Projected::new(doc);
        match rings_region(&p, 1) {
            Some(region) => assert!(
                region.size.min_element() >= *inset,
                "{name}: the ring's box reaches its inset out, {region:?}"
            ),
            None => assert!(
                *inset > 1.0e18,
                "{name}: only a ring round an outline too long to lay has no region"
            ),
        }
        let mut every: Vec<NodeId> = p.projector.nodes.keys().copied().collect();
        every.sort_unstable();
        assert_eq!(
            p.projector.nodes_in(huge_box(*inset)),
            every,
            "{name}: a rubber band as large as the ring covers every node"
        );
        if k == 0 {
            let before = rings_region(&p, 1);
            move_to(&mut p, 1, Vec2::new(1.0e6, -220.0));
            assert_ne!(
                rings_region(&p, 1),
                before,
                "precondition: {name}: the region moved with the node"
            );
            p.check().unwrap();
        }
    }
}
