// SPDX-License-Identifier: MPL-2.0

//! The projection's fuzz properties (design D29, §20 rule 10; rulings
//! B7.3 on #241): a loaded map projects, and an edit sequence keeps the
//! projection equal to a full one.
//!
//! - [`check_projects`](crate::projection::tests::projection_fuzz::check_projects)
//!   is what the loader targets (`load_raw`, `load_structured`) hold every
//!   map they load to: inside the projection's work bound
//!   ([`within_bound`](crate::projection::tests::projection_fuzz::within_bound)),
//!   it projects without a panic, one group per visible node in every
//!   layer, every run's box finite, the same layers when projected again,
//!   hits that do not panic, and one generated edit that the update
//!   brings the layers through to a full projection's. A map outside the
//!   bound is skipped for this property and counted, and so is one whose
//!   untangling reaches the ceiling below, apart
//!   ([`projection_counts`](crate::projection::tests::projection_fuzz::projection_counts)),
//!   which the targets report.
//! - [`check_projection_edits`](crate::projection::tests::projection_fuzz::check_projection_edits)
//!   is the projection_edits target's body: a generated map and a
//!   sequence of edits drawn from the input — field edits, structural
//!   edits, a rebinding near the root, theme and palette changes,
//!   transaction stages, undos and redos — the oracle checked after each.
//!
//! **The bound** (ruling B7.3). The engine bounds each placement (R41)
//! and each frame's work (its budget), never a whole map, so a hostile
//! map can ask for more work than a fuzz input's time (#316). Where the
//! engine's bound lies past the timeout the harness bounds what it feeds
//! (TEST_CONVENTIONS §T14): a map is projected here when it holds at most
//! one ring that lays text — a span's, or a `vertical` or `corners` text,
//! which a `Repeat` ring lays where its outline runs steep and where it
//! turns, all round where it has no span (`format/schema.md`,
//! "Directional ring") — of at most
//! [`MAX_FRAME_GLYPHS`](crate::flow::tests::flow_fuzz::MAX_FRAME_GLYPHS)
//! characters, all of its texts together — the flow harnesses' one frame
//! an input — and every coordinate a path or an outline is drawn from is
//! within [`EXTENT`](crate::projection::tests::projection_fuzz::EXTENT);
//! it draws at most
//! [`MAX_UNITS`](crate::projection::tests::projection_fuzz::MAX_UNITS)
//! sections, rings, connectors and links; and each `Repeat` ring may lay
//! at most
//! [`MOST_RING_GLYPHS`](crate::projection::tests::projection_fuzz::MOST_RING_GLYPHS)
//! glyphs.
//!
//! **A node's drawn rectangle and a frame's outline are coordinates of the
//! bound** (rulings B47.1, B47.2). A path is drawn from the rectangles its
//! ends are drawn as, and a ring round its node's: the larger of the node's
//! `min_size` and each of its sections' `offset` plus its box
//! (`geometry::drawn_rect`, D5), which grows with the section's text. Each
//! ring lies where the projection grows that rectangle's outline to: by its
//! frame's `inset`, a `gap` for each ring inside it and those rings' line
//! heights (`stages::clearance`). The loader holds an `inset`, a `gap`, an
//! `offset` and a `min_size` to the canvas bound (#364) and a text to its
//! bytes, so a ring can lie 10¹² out and a node's far side 10⁸ out; the
//! bound holds every node's drawn rectangle within `EXTENT`, and every
//! frame's outermost outline grown from it, as it holds every coordinate
//! (`drawn_bound`, `outline_reach`). The cap of a placement (R41) bounds
//! its glyphs, not their work: a ring's work is its glyphs times its
//! path's reads a glyph, and its budget bounds that, 196,608,000 glyph
//! steps at the cap (ruling B17.2c), far past the timeout. Inside the
//! extent an `f32` point is a thousandth of a unit or finer, a glyph's ink
//! is set apart from the last in a few reads, and the densest ring of dots
//! the bound admits takes under five steps a glyph at the cap; 10¹⁰ out a
//! point is a thousand units from the next, and each glyph of the same ring
//! read its path a thousand times, 62 steps a glyph, every projection a
//! check makes (#374;
//! `test_every_repeat_ring_the_bound_admits_fits_the_share`). A `spacing`
//! is not part of where a ring lies: it parts a placement's glyphs, and
//! moves no outline and no path.
//!
//! **A `Repeat` ring's work is part of the bound** (ruling B47.3). Where a
//! ring lies does not bound how much work it is: a `Repeat` ring a little
//! short of the cap closes its spacing and takes some 40 steps a glyph
//! wherever it lies, and inside the extent such rings took 40 to 118 s of
//! a check under ASan, against the 20 s timeout. So a map is projected
//! here only when each `Repeat` ring may lay no more glyphs than
//! `MOST_RING_GLYPHS` (`repeat_glyphs`, bounded before it is laid): the
//! most whose work — `RING_STEPS_A_GLYPH` a glyph and `RING_SETUP_STEPS`,
//! stated from a sweep of the rings the bound admits — laid
//! `RING_LAYS_A_CHECK` times, once in each projection the check makes, fits
//! the input's share, `flow_fuzz::INPUT_STEPS`. The flow harnesses hold
//! their rings to `MAX_RING_GLYPHS` for the same reason (B17.2b). A ring's
//! glyphs are bounded as its outline's perimeter at its reach over the
//! least step one of them takes along it: its advance, from the narrowest
//! glyph a face maps each character to, at the least size the ring can be,
//! and its `spacing`. A ring of text laid by halves — any other fit — is
//! held the same way, to the steps one lay of it may take,
//! `FRAME_STEPS_A_LAY`: its text's characters and the glyphs its side
//! ornament may lay (`side_glyphs`), at `FRAME_STEPS_A_GLYPH` and
//! `FRAME_STEPS_A_SIDE_GLYPH`, and `FRAME_SETUP_STEPS` (`frame_steps`). The
//! flow harnesses lay such a frame once an input and give it the whole
//! share; a check here lays it up to ten times.

//! **A section's box is bounded before it is shaped** (ruling B47.2). The
//! shaper sizes it to the larger of its `min_size` and its text's natural
//! size, its widest line by the sum of its lines' heights, its lines
//! broken at `\n` alone (`flow::shape_box`). The bound reads that from the
//! document (`box_bound`): each line as tall as `LINE_HEIGHT_FACTOR` times
//! the largest point size among the spans on it, as the shaper makes a
//! line, and as wide as each of its characters at its span's point size
//! times the widest advance any glyph can have, a tab as wide as eight —
//! a tab is laid to its next stop, at most eight spaces on
//! (`TAB_STOP_SPACES`). The sizes are the projection's: resolved as it
//! resolves a section's spans and turned into points as it turns them. The
//! widest advance is read from the faces the font system holds, the host's
//! with the compiled ones (`widest_advance`): the most of every glyph's
//! advance at the default instance plus, in a variable face, every
//! positive delta its `HVAR` holds for it, each region's scalar being one
//! at most — or, in one with no `HVAR`, its advance at every weight a span
//! can ask for, the only axis the shaper sets. No glyph is laid wider. What
//! the bound takes on trust is that the shaper lays no more than that
//! advance for a character: a face's substitutions can lay one character
//! as several glyphs, and its positioning can widen a pair. A face that
//! did either past the widest advance would draw its box wider than the
//! bound reads, by that excess, and the end-to-end property
//! (`test_every_ring_the_bound_admits_lies_within_the_extent`) holds the
//! bound's own cases and the review's probes to it.
//!
//! The coordinates bound how long a path is in map units, not how many
//! glyphs or samples it takes: those go by its length over its size, and
//! a link at the smallest sizes a few thousand units long reaches the cap
//! of a placement (R41) and of a curve's samples in `untangle`. What one
//! such path costs is bounded by those caps — its glyphs at a
//! placement's, each read a few times where its points are near, and in
//! `untangle` at most `UNTANGLE_ROUNDS` rounds of each of its curves
//! sampled to the cap, one round where its nearness is certain from
//! samples no halving moves (#346) — not by this bound. Each is shaped and
//! placed in every full projection the check makes, so the work grows with
//! their count: libFuzzer reported a 4,000-node chain with a link on every
//! node, 8,000 paths, as a slow unit of 16 to 18 s against its 20 s
//! timeout.
//!
//! **The untangling ceiling** (#346). What `untangle` samples cannot be
//! told before the paths are laid out — a path's width comes from its
//! shaped glyphs, its sides from where its nodes are drawn — and an
//! estimate from the map alone, every path end at the sampling's cap for
//! every round, rates the seeds above maps that take several times their
//! work, and would skip two thirds of the maps projected. So the check
//! measures it: it sets `projection::geometry::UNTANGLE_CEILING` to
//! [`MAX_UNTANGLE_SAMPLES`](crate::projection::tests::projection_fuzz::MAX_UNTANGLE_SAMPLES)
//! samples for the whole check, every projection of it together;
//! `untangle` takes from it what each lane samples, and when a lane's
//! samples would take more than is left it grids no further lane and
//! takes no further round, and no samples at all after that in the check,
//! and marks the check. A marked check is abandoned: what it found is not
//! this map's, so nothing is asserted of it, and it is counted as cut
//! short. The ceiling and the mark are compiled into test and fuzz builds
//! only (`cfg(test)`, `cfg(fuzzing)`). So one check's work in `untangle`
//! is at most the ceiling and the one lane that reaches it; the rest of
//! the check is bounded by the coordinate bound above and the engine's
//! own caps.

use std::cell::Cell;
use std::sync::OnceLock;

use glam::Vec2;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::arena::NodeId;
use crate::document::element::{DocElement, RingData, SectionData};
use crate::document::ids::Document;
use crate::document::resolve::{resolve_frame, resolve_ring_in, resolve_text_in, Binding};
use crate::document::values::FitMode;
use crate::document::writer::transaction::TransactionKind;
use crate::document::writer::ChangeSource;
use crate::engine::interpolate::Snap;
use crate::engine::tree::Tree;
use crate::flow::tests::flow_fuzz::{INPUT_STEPS, MAX_FRAME_GLYPHS};
use crate::flow::{Rect, LINE_HEIGHT_FACTOR, MAX_PLACED_GLYPHS};
use crate::font::fallback::DEFAULT_FACE;
use crate::font::fonts::{AppFont, FontSystemLease, COMPILED_FONT_ID_MAP, MAX_FONT_SIZE_PT};
use crate::projection::geometry;
use crate::projection::tests::corpus::{edit, edit_named, generated, EDITS};
use crate::projection::tests::support::Projected;
use crate::projection::Layer;
use crate::util::tests::choices::Choices;
use cosmic_text::fontdb::ID;

/// How far from the origin, per axis, a coordinate of a map the loader
/// targets project may be: positions, sizes, offsets and control points.
pub const EXTENT: f32 = 8192.0;

/// The most sections, rings, connectors and links a map the loader
/// targets project may draw (see the module docs).
pub const MAX_UNITS: usize = 2048;

/// The most samples `untangle` may take in one check of a map, every
/// projection of the check together: past it the check is abandoned and
/// the map counted as cut short (see the module docs). Chosen from
/// counts: the two seeds this bound was made for, each a line link beside
/// a connector (`line_link_beside_connector_*`), take 769,760 and 461,832
/// in a check and are projected; the input that timed the target out took
/// 9,326,695 (#346).
pub const MAX_UNTANGLE_SAMPLES: usize = 2_000_000;

thread_local! {
    static PROJECTED: Cell<usize> = const { Cell::new(0) };
    static SKIPPED: Cell<usize> = const { Cell::new(0) };
    static CUT: Cell<usize> = const { Cell::new(0) };
}

/// How many loaded maps [`check_projects`] has projected, how many it
/// skipped as outside the bound, and how many it abandoned when their
/// untangling reached [`MAX_UNTANGLE_SAMPLES`], on this thread: what a
/// fuzz run reports (ruling B7.3), and, since a test runs on a thread of
/// its own, exactly what a test's own sweep made.
pub fn projection_counts() -> (usize, usize, usize) {
    (
        PROJECTED.with(Cell::get),
        SKIPPED.with(Cell::get),
        CUT.with(Cell::get),
    )
}

/// Whether `v` is within [`EXTENT`] on both axes.
fn near(v: Vec2) -> bool {
    v.is_finite() && v.x.abs() <= EXTENT && v.y.abs() <= EXTENT
}

/// Whether `doc` is inside the projection's work bound (see the module
/// docs). O(the document), and once a process O(the font system's glyphs)
/// for `widest_advance`.
pub fn within_bound(doc: &Document) -> bool {
    let tree = doc.tree();
    let arena = tree.arena();
    let mut rings_with_text = 0;
    let mut units = 0;
    for id in tree.descendants() {
        if matches!(
            arena[id].get(),
            DocElement::Section(_) | DocElement::Ring(_) | DocElement::Connector(_) | DocElement::Link(_)
        ) {
            units += 1;
            if units > MAX_UNITS {
                return false;
            }
        }
        let ok = match arena[id].get() {
            DocElement::Node(node) => {
                near(node.position)
                    && node.min_size.is_none_or(near)
                    && grown_near(drawn_bound(tree, id), 0.0)
            }
            DocElement::Section(section) => near(section.offset) && section.min_size.is_none_or(near),
            DocElement::Connector(c) => c.path.control_points.iter().all(|&p| near(p)),
            DocElement::Link(l) => l.path.control_points.iter().all(|&p| near(p)),
            DocElement::Label(label) => label.style.offset.is_none_or(|o| o.abs() <= EXTENT),
            DocElement::PortalEnd(end) => end.style.offset.is_none_or(|o| o.abs() <= EXTENT),
            DocElement::Frame(_) => {
                outline_near(tree, id) && repeat_rings_fit(tree, id) && text_frames_fit(tree, id)
            }
            DocElement::Ring(ring) => {
                let spans: usize = id
                    .children(arena)
                    .filter_map(|s| match arena[s].get() {
                        DocElement::Span(span) => Some(span.text.chars().count()),
                        _ => None,
                    })
                    .sum();
                let chars = spans + directional_chars(ring);
                if chars > 0 {
                    rings_with_text += 1;
                }
                chars <= MAX_FRAME_GLYPHS
            }
            _ => true,
        };
        if !ok {
            return false;
        }
    }
    rings_with_text <= 1
}

/// The most line height a ring is placed at: its run's, its largest size
/// times [`LINE_HEIGHT_FACTOR`], every size held to [`MAX_FONT_SIZE_PT`]
/// as it is shaped, at whatever scale a `Fit` lays it.
pub(crate) const MOST_RING_LINE_HEIGHT: f32 = LINE_HEIGHT_FACTOR * MAX_FONT_SIZE_PT;

/// The characters of the texts `ring` lays by direction, its `vertical`
/// and its four `corners` (#324), which a `Repeat` ring lays where its
/// spans have nothing to draw too (`format/schema.md`, "Directional
/// ring"). Counted whatever its fit, which is more than a ring of another
/// fit lays. O(those texts).
fn directional_chars(ring: &RingData) -> usize {
    ring.directional.as_deref().map_or(0, |d| {
        d.vertical.as_deref().map_or(0, |v| v.chars().count())
            + d.corners
                .as_ref()
                .map_or(0, |corners| corners.iter().map(|c| c.chars().count()).sum())
    })
}

/// Whether `ring` may lay a glyph and so take a line height
/// (`RingState::extent`): it holds a span — even of no text, which is
/// shaped as an empty line — or a `vertical` or `corners` text (ruling
/// B47.2). A ring of neither shapes a run of no glyphs; one of a frame's
/// `sides` alone, or of a `Once` filler alone, places none (the review of
/// #378). O(its children).
fn lays_glyphs(tree: &Tree<DocElement>, ring: NodeId) -> bool {
    let arena = tree.arena();
    let directional = match arena[ring].get() {
        DocElement::Ring(data) => data.directional.as_deref().is_some_and(|d| d.is_set()),
        _ => false,
    };
    directional
        || ring
            .children(arena)
            .any(|span| matches!(arena[span].get(), DocElement::Span(_)))
}

/// How far past its node's drawn rectangle `frame`'s outermost ring's
/// outline may lie (ruling B47.1): the clearance the projection grows the
/// node's outline by for it (`stages::clearance`), its frame's `inset` and
/// a `gap` for each ring inside it, as resolved, and the line heights of
/// the rings inside it — each [`MOST_RING_LINE_HEIGHT`] at most where it
/// may lay a glyph ([`lays_glyphs`]), and none where it may not. No ring of
/// the frame lies further out than its outermost. `0` for a frame of no
/// rings. O(its rings).
pub(crate) fn outline_reach(tree: &Tree<DocElement>, frame: NodeId) -> f64 {
    let arena = tree.arena();
    let rings: Vec<NodeId> = frame
        .children(arena)
        .filter(|&ring| matches!(arena[ring].get(), DocElement::Ring(_)))
        .collect();
    let Some((_, inside)) = rings.split_last() else {
        return 0.0;
    };
    let laying = inside.iter().filter(|&&ring| lays_glyphs(tree, ring)).count();
    let resolved = resolve_frame(tree, frame, &mut ());
    f64::from(resolved.inset)
        + laying as f64 * f64::from(MOST_RING_LINE_HEIGHT)
        + inside.len() as f64 * f64::from(resolved.gap)
}

/// Whether `rect` grown by `reach` on every side lies within [`EXTENT`]
/// on both axes. O(1).
fn grown_near(rect: Rect, reach: f64) -> bool {
    let extent = f64::from(EXTENT);
    let (near, far) = (rect.origin, rect.max());
    [(near.x, far.x), (near.y, far.y)].into_iter().all(|(near, far)| {
        let (near, far) = (f64::from(near), f64::from(far));
        near.is_finite() && far.is_finite() && near - reach >= -extent && far + reach <= extent
    })
}

/// Whether every ring of `frame` lies within [`EXTENT`] (rulings B47.1,
/// B47.2): its node's drawn rectangle, at most [`drawn_bound`], grown by
/// [`outline_reach`]. A frame with no node lays nothing. O(its rings and
/// its node's sections' text).
fn outline_near(tree: &Tree<DocElement>, frame: NodeId) -> bool {
    let arena = tree.arena();
    let Some(node) = frame.parent(arena) else {
        return true;
    };
    if !matches!(arena[node].get(), DocElement::Node(_)) {
        return true;
    }
    grown_near(drawn_bound(tree, node), outline_reach(tree, frame))
}

/// The rectangle at most as large as the one `node` is drawn as
/// (ruling B47.2): `geometry::drawn_rect`, as the projection measures a
/// node (`stages::measure`), from its `position` and `min_size` and each
/// of its sections' far corner, its `offset` plus [`box_bound`]. Each far
/// corner is at least the one the projection reads, so the rectangle is.
/// The rectangle of no size at the origin for what is not a node.
/// O(its sections' text).
pub(crate) fn drawn_bound(tree: &Tree<DocElement>, node: NodeId) -> Rect {
    let arena = tree.arena();
    let DocElement::Node(data) = arena[node].get() else {
        return Rect::new(Vec2::ZERO, Vec2::ZERO);
    };
    let corners = node
        .children(arena)
        .map_while(|section| match arena[section].get() {
            DocElement::Section(s) => Some(geometry::usable_point(s.offset) + box_bound(tree, section, s)),
            _ => None,
        });
    geometry::drawn_rect(data.position, data.min_size, corners)
}

/// How many times a check lays a ring at most (ruling B47.3): once in each
/// full projection it makes — its first, and the sweep's comparison — and
/// for its one edit at most four updates, each followed by the oracle's
/// full projection: a transaction's two stages, its interpolation and its
/// commit (`sweep`). A fuzz build shares no placement between them.
pub const RING_LAYS_A_CHECK: usize = 10;

/// The most glyph steps a `Repeat` ring the bound admits takes for each
/// glyph `repeat_glyphs` bounds it to, past [`RING_SETUP_STEPS`] (ruling
/// B47.3). Stated from measurement: the engine's own bound is its budget,
/// `STEPS_PER_GLYPH`, 3,000 a glyph over a floor of 400,000 steps a lay
/// (B17.2c), under which ten lays fit no ring at all into the input's
/// share, and its spacing solves are read against that budget and nothing
/// less. A sweep of 576 admitted rings — `·` at a quarter and at the whole
/// of its node, `MANDALA `, `*` at a twentieth, and `·`, a ring of `|` and
/// one of corners spaced, round an ellipse, a sharp and a rounded rectangle
/// 1 to 1,000 across, from the node's side to 3,000 out — and the densest
/// rings this bound admits took at most some 82 steps a glyph of their
/// bound: `*` at a twentieth of an ellipse 1,000 across, 1,127 out, 48,974
/// steps for 363 glyphs laid of 597 bounded. 128 holds that with half as
/// much again. `test_every_repeat_ring_the_bound_admits_fits_the_share`
/// holds the densest rings the bound admits to it. What a step costs under
/// ASan, 6.6 to 10 µs where measured against the share's 32.3, and the
/// three to five lays these maps' checks make against the ten counted
/// leave a ring several times past this still within the timeout.
pub const RING_STEPS_A_GLYPH: usize = 128;

/// The most glyph steps a `Repeat` ring the bound admits takes past
/// [`RING_STEPS_A_GLYPH`] a glyph of its bound (ruling B47.3): its setup,
/// its path's tables, and shaping and inking its copy, which do not grow
/// with its glyphs. Stated from the same sweep, whose most past 64 a glyph
/// was 5,481, rounded up to a power of two.
pub const RING_SETUP_STEPS: usize = 8_192;

/// The most glyphs a `Repeat` ring the bound admits may lay (ruling
/// B47.3), 298: the most whose work, [`RING_STEPS_A_GLYPH`] a glyph and
/// [`RING_SETUP_STEPS`], laid [`RING_LAYS_A_CHECK`] times, fits the input's
/// share, `flow_fuzz::INPUT_STEPS` at `flow_fuzz::ASAN_STEP_NANOS` a step —
/// the flow harnesses' share for an input, whose rings they lay at
/// `MAX_RING_GLYPHS` (B17.2b) for the same reason. The densest ring it
/// admits, by the steps it takes, `*` at a twentieth of an ellipse 1,000
/// across, 311 out — 182 glyphs laid of 298 bounded, 19,750 steps a lay —
/// ran its whole input in 0.82 s under ASan (`cargo fuzz run load_raw
/// <file> -- -runs=1`, the load average some 5), against the 20 s
/// timeout, `flow_fuzz::FUZZ_TIMEOUT_SECONDS`.
pub const MOST_RING_GLYPHS: usize = (INPUT_STEPS / RING_LAYS_A_CHECK - RING_SETUP_STEPS) / RING_STEPS_A_GLYPH;

/// The most glyphs `ring` may lay, laid `Repeat` (ruling B47.3), bounded
/// before it is laid: the perimeter its outline can have, over the least
/// one of its glyphs can advance along it, and a copy and its corners more.
/// `None` for a ring of another fit, which lays its text once.
///
/// - **The perimeter**: its node's drawn rectangle at most
///   ([`drawn_bound`]) grown by its frame's outermost reach
///   ([`outline_reach`]), whose perimeter, 2 (w + h) + 8 × reach, is at
///   least that of any convex outline inside it — every node shape is
///   convex, and an outline grown from one is.
/// - **A glyph's step at the least**: each text it lays — its spans', each
///   in its own font, or the `vertical` its spans have nothing to draw
///   beside — over its characters, as its grapheme clusters advance at
///   least ([`FaceMetrics::narrowest_cluster`]) at the least point size its
///   ring can be, plus its `spacing`, which a `Repeat` ring adds between
///   every two glyph steps. The size is relative to its node's drawn
///   rectangle, at least `min_size` (D5), and so at least `min_size`'s
///   point size. A ring that lays by direction steps a run down a side by
///   its glyphs' ink, upright, and not by their advance
///   (`place::directional`): its `spacing` alone is a step it takes at
///   least. One whose step can be nothing lays the cap.
/// - The glyphs of a copy past that, a copy on each of the four runs of a
///   ring laid by direction, and its corners; and the cap,
///   `MAX_PLACED_GLYPHS`, at most.
///
/// What it takes on trust, as [`box_bound`] does: that a character lays a
/// glyph at most, in its span's family where a face of that family maps
/// it, and no narrower than the glyph it maps to; and that a `Repeat` ring
/// of its spans steps along its outline by its glyphs' advances, as every
/// ring the end-to-end property reads does (`rings_within_the_extent`
/// holds what each lays to this). O(its texts), and once a process
/// [`face_metrics`].
pub(crate) fn repeat_glyphs(tree: &Tree<DocElement>, frame: NodeId, ring: NodeId) -> Option<usize> {
    use unicode_segmentation::UnicodeSegmentation;
    let arena = tree.arena();
    let resolved = resolve_ring_in(tree, ring, Binding::Carried(None), &mut ());
    if !matches!(resolved.fit, FitMode::Repeat) {
        return None;
    }
    let node = frame.parent(arena)?;
    let DocElement::Node(data) = arena[node].get() else {
        return None;
    };
    let style = resolve_text_in(tree, ring, Binding::Carried(None), &mut ());
    // Each text it lays, as runs of a font: its spans', one copy, each in
    // its own font, and its `vertical`, in its own.
    let spans: Vec<(AppFont, &str)> = ring
        .children(arena)
        .filter_map(|span| match arena[span].get() {
            DocElement::Span(text) => Some((
                resolve_text_in(tree, span, Binding::Carried(None), &mut ()).font,
                text.text.as_str(),
            )),
            _ => None,
        })
        .filter(|(_, text)| !text.is_empty())
        .collect();
    let mut texts: Vec<Vec<(AppFont, &str)>> = Vec::new();
    if !spans.is_empty() {
        texts.push(spans);
    }
    if let Some(vertical) = resolved.directional.vertical.as_deref() {
        texts.push(vec![(style.font, vertical)]);
    }
    let corners: usize = resolved
        .directional
        .corners
        .as_ref()
        .map_or(0, |corners| corners.iter().map(|c| c.chars().count()).sum());
    if texts.is_empty() {
        return Some(corners.min(MAX_PLACED_GLYPHS));
    }
    let least_rect = Rect::new(Vec2::ZERO, geometry::usable_size(data.min_size));
    let size = f64::from(geometry::points(style.size, least_rect));
    let spacing = if resolved.spacing.is_finite() {
        f64::from(resolved.spacing.max(0.0))
    } else {
        0.0
    };
    let faces = face_metrics();
    // The least a glyph of each text advances along the outline, and the
    // most glyphs a copy of any holds.
    let mut least = f64::INFINITY;
    let mut copy = 0usize;
    for text in &texts {
        let (mut chars, mut clusters, mut advance) = (0usize, 0usize, 0.0f64);
        for &(font, run) in text {
            chars += run.chars().count();
            for cluster in run.graphemes(true) {
                clusters += 1;
                advance += f64::from(faces.narrowest_cluster(font, cluster));
            }
        }
        // A ring that lays by direction steps a run down a side by its
        // glyphs' ink, upright, not by their advance (`place::directional`):
        // only its `spacing` is a step it takes at least.
        let step = if resolved.directional.is_set() {
            clusters as f64 * spacing
        } else {
            advance * size + clusters as f64 * spacing
        };
        least = least.min(step / chars as f64);
        copy = copy.max(chars);
    }
    let drawn = drawn_bound(tree, node);
    let reach = outline_reach(tree, frame);
    let perimeter = 2.0 * f64::from(drawn.size.x + drawn.size.y) + 8.0 * reach;
    let glyphs = perimeter / least;
    if !(glyphs.is_finite() && glyphs < MAX_PLACED_GLYPHS as f64) {
        return Some(MAX_PLACED_GLYPHS);
    }
    // A ring laid by direction lays a glyph at least on each of its four
    // runs, however short.
    let runs = if resolved.directional.is_set() { 4 } else { 1 };
    Some((glyphs.ceil() as usize + runs * copy + corners).min(MAX_PLACED_GLYPHS))
}

/// Whether every `Repeat` ring of `frame` may lay no more glyphs than
/// [`MOST_RING_GLYPHS`] ([`repeat_glyphs`]; ruling B47.3). O(its rings'
/// texts).
fn repeat_rings_fit(tree: &Tree<DocElement>, frame: NodeId) -> bool {
    let arena = tree.arena();
    frame
        .children(arena)
        .filter(|&ring| matches!(arena[ring].get(), DocElement::Ring(_)))
        .all(|ring| repeat_glyphs(tree, frame, ring).is_none_or(|glyphs| glyphs <= MOST_RING_GLYPHS))
}

/// The most glyph steps a frame laid by halves — a ring of text of any
/// fit but `Repeat` — takes for each character of its text, past
/// [`FRAME_SETUP_STEPS`] and its side ornament's (ruling B47.3, on the
/// review of #378). Stated from measurement, as [`RING_STEPS_A_GLYPH`] is:
/// the engine bounds a frame's work by its budget, 400,000 steps at the
/// least (R44.2), which the flow harnesses give one lay of an input's
/// share and a check here lays up to [`RING_LAYS_A_CHECK`] times. A sweep
/// of 1,080 frames — 1 to 151 characters of Latin, of seven scripts and of
/// code points no face holds; `spread`, `fit` and `once`; no ornament, `✦`,
/// a 60-character one and a 256-character one; round ellipses 1 to 8,000
/// across from a twentieth of their size to the least — took at most 192
/// a character with 320 a side glyph and 8,192 (`latin spread`, a `✦`, 100
/// across: 1,730 short of it). `test_every_text_frame_the_bound_admits_fits_the_share`
/// holds the densest admitted frames to it.
pub const FRAME_STEPS_A_GLYPH: usize = 192;

/// The most glyph steps a frame laid by halves takes for each glyph its
/// side ornament may lay (`side_glyphs`), past its text's: from the same
/// sweep, whose ornaments took at most 292 a glyph of their bound.
pub const FRAME_STEPS_A_SIDE_GLYPH: usize = 320;

/// The most glyph steps a frame laid by halves takes past its glyphs'
/// steps: from the same sweep, as [`RING_SETUP_STEPS`].
pub const FRAME_SETUP_STEPS: usize = 8_192;

/// The most glyph steps one lay of a frame laid by halves may take inside
/// the bound (ruling B47.3): the input's share, `flow_fuzz::INPUT_STEPS`,
/// over the [`RING_LAYS_A_CHECK`] times a check lays it, 46,439. The flow
/// harnesses give one frame the whole share, since they lay it once; a
/// frame admitted by that one lay's share took 18.4 s of a check under
/// ASan in three lays — 80 letters `fit` with a 60-character ornament round
/// an ellipse 1,000 across, 185,336 steps a lay — where ten would have
/// taken a minute. Under ASan (`cargo fuzz run load_raw <file> --
/// -runs=1`, the load average some 2.5), the densest frame this admits by
/// its steps, 151 letters `spread` round an ellipse 1,000 across, 22,872
/// steps a lay, ran its input in 0.50 s. The slowest it admits, 151
/// characters of seven scripts `once` with a `✦` round one 100 across,
/// 21,702 steps a lay, ran in 3.1 s in three lays, some 10 s in ten, under
/// the 20 s timeout: most of its time is shaping its text in the faces its
/// scripts fall back to, which a full projection does again and no step
/// counts. That shaping is what a check's projections cost for every flow,
/// a section's and a path's too, and this bound does not hold it.
pub const FRAME_STEPS_A_LAY: usize = INPUT_STEPS / RING_LAYS_A_CHECK;

/// The most glyphs the side ornament `sides` of a frame laid by halves may
/// lay (ruling B47.3), bounded before it is laid as [`repeat_glyphs`]
/// bounds a `Repeat` ring's: the outline's `perimeter`, over the least a
/// copy of it advances at `size`, the least size its ring can be laid at,
/// each of its grapheme clusters at its narrowest in `font`
/// ([`FaceMetrics::narrowest_cluster`]); and a copy more at each of the
/// two breaks it fills. The cap, `MAX_PLACED_GLYPHS`, where a copy can
/// advance by nothing. O(its text).
fn side_glyphs(sides: &str, font: AppFont, size: f64, perimeter: f64) -> usize {
    use unicode_segmentation::UnicodeSegmentation;
    let faces = face_metrics();
    let chars = sides.chars().count();
    let copy: f64 = sides
        .graphemes(true)
        .map(|cluster| f64::from(faces.narrowest_cluster(font, cluster)))
        .sum::<f64>()
        * size;
    let copies = perimeter / copy;
    if !(copies.is_finite() && copies * (chars as f64) < MAX_PLACED_GLYPHS as f64) {
        return MAX_PLACED_GLYPHS;
    }
    ((copies.ceil() as usize + 2) * chars).min(MAX_PLACED_GLYPHS)
}

/// The most glyph steps one lay of `ring`, a ring of `frame` laid by
/// halves — any fit but `Repeat` — may take (ruling B47.3): its text's
/// glyphs, a glyph a character, at [`FRAME_STEPS_A_GLYPH`], its side
/// ornament's ([`side_glyphs`]) at [`FRAME_STEPS_A_SIDE_GLYPH`], and
/// [`FRAME_SETUP_STEPS`]. The ornament's least size is its ring's at
/// `min_size` (D5), times a `Fit`'s least scale where that is less than
/// one. `None` for a `Repeat` ring ([`repeat_glyphs`] bounds those) and a
/// ring of no text, which lays nothing. O(its texts), and once a process
/// [`face_metrics`].
pub(crate) fn frame_steps(tree: &Tree<DocElement>, frame: NodeId, ring: NodeId) -> Option<usize> {
    let arena = tree.arena();
    let resolved = resolve_ring_in(tree, ring, Binding::Carried(None), &mut ());
    if matches!(resolved.fit, FitMode::Repeat) {
        return None;
    }
    let text: usize = ring
        .children(arena)
        .filter_map(|span| match arena[span].get() {
            DocElement::Span(span) => Some(span.text.chars().count()),
            _ => None,
        })
        .sum();
    if text == 0 {
        return None;
    }
    let node = frame.parent(arena)?;
    let DocElement::Node(data) = arena[node].get() else {
        return None;
    };
    let sides = match &resolved.sides {
        Some(sides) => {
            let style = resolve_text_in(tree, ring, Binding::Carried(None), &mut ());
            let least_rect = Rect::new(Vec2::ZERO, geometry::usable_size(data.min_size));
            let scale = match resolved.fit {
                FitMode::Fit { min, .. } if min.is_finite() => f64::from(min.clamp(0.0, 1.0)),
                _ => 1.0,
            };
            let size = (f64::from(geometry::points(style.size, least_rect)) * scale)
                .max(f64::from(crate::font::fonts::MIN_FONT_SIZE_PT));
            let drawn = drawn_bound(tree, node);
            let perimeter = 2.0 * f64::from(drawn.size.x + drawn.size.y) + 8.0 * outline_reach(tree, frame);
            side_glyphs(&sides.content, style.font, size, perimeter)
        }
        None => 0,
    };
    Some(
        FRAME_STEPS_A_GLYPH
            .saturating_mul(text)
            .saturating_add(FRAME_STEPS_A_SIDE_GLYPH.saturating_mul(sides))
            .saturating_add(FRAME_SETUP_STEPS),
    )
}

/// Whether one lay of every ring of `frame` laid by halves may take no
/// more than [`FRAME_STEPS_A_LAY`] ([`frame_steps`]; ruling B47.3).
/// O(its rings' texts).
fn text_frames_fit(tree: &Tree<DocElement>, frame: NodeId) -> bool {
    let arena = tree.arena();
    frame
        .children(arena)
        .filter(|&ring| matches!(arena[ring].get(), DocElement::Ring(_)))
        .all(|ring| frame_steps(tree, frame, ring).is_none_or(|steps| steps <= FRAME_STEPS_A_LAY))
}

/// How many spaces on a tab is laid to its next stop at most: the tab
/// width of cosmic-text's `Buffer`, which `flow::shape_box` keeps, a stop
/// every eight spaces.
const TAB_STOP_SPACES: f64 = 8.0;

/// A size at least that of the box `section` is shaped as
/// (`flow::shape_box`; ruling B47.2): the larger, per axis, of its
/// `min_size` as the shaper reads one and its text's natural size bounded
/// — each line [`LINE_HEIGHT_FACTOR`] times the largest point size of the
/// spans on it high, and the sum over its characters of their span's
/// point size times [`widest_advance`] wide, a tab [`TAB_STOP_SPACES`]
/// times that (see the module docs). Lines break at `\n` alone; no span is
/// no text and no line. The point sizes are the projection's: each span
/// resolved as it resolves a section's (`resolve_text_in`), but with no
/// palette binding — a binding gives a color, never a size, and finding
/// one climbs the node's ancestors, which a chain makes quadratic — and
/// turned into points with no rectangle,
/// as a section's are (`geometry::points`). Each sum is grown by what the
/// shaper's `f32` sums of as many terms may round up by. O(its text).
fn box_bound(tree: &Tree<DocElement>, section: NodeId, data: &SectionData) -> Vec2 {
    let arena = tree.arena();
    let mut natural = (0.0f64, 0.0f64);
    // The characters and the lines summed, for the rounding below.
    let (mut chars, mut lines) = (0usize, 0usize);
    // The line being read: its width so far and its largest size.
    let mut line: Option<(f64, f64)> = None;
    for span in section.children(arena) {
        let DocElement::Span(text) = arena[span].get() else {
            continue;
        };
        let style = resolve_text_in(tree, span, Binding::Carried(None), &mut ());
        let size = f64::from(geometry::points(style.size, Rect::new(Vec2::ZERO, Vec2::ZERO)));
        let (mut width, mut tallest) = line.unwrap_or((0.0, 0.0));
        tallest = tallest.max(size);
        for c in text.text.chars() {
            if c == '\n' {
                natural.0 = natural.0.max(width);
                natural.1 += tallest * f64::from(LINE_HEIGHT_FACTOR);
                lines += 1;
                (width, tallest) = (0.0, size);
            } else {
                let spaces = if c == '\t' { TAB_STOP_SPACES } else { 1.0 };
                width += spaces * size * f64::from(widest_advance());
                chars += 1;
            }
        }
        line = Some((width, tallest));
    }
    if let Some((width, tallest)) = line {
        natural.0 = natural.0.max(width);
        natural.1 += tallest * f64::from(LINE_HEIGHT_FACTOR);
        lines += 1;
    }
    // The shaper sums in `f32`, which may round each term and the sum up:
    // `n` terms are within `n + 4` epsilons of their exact sum, which these
    // `f64` sums are at least.
    let rounded =
        |sum: f64, terms: usize| (sum * (1.0 + (terms + 4) as f64 * f64::from(f32::EPSILON))) as f32;
    let natural = Vec2::new(rounded(natural.0, chars), rounded(natural.1, lines));
    natural.max(geometry::usable_size(data.min_size))
}

/// What the bound reads of the faces the font system holds — the compiled
/// faces and, on native, the host's — at any instance the shaper lays a
/// variable face at (rulings B47.2, B47.3; see the module docs), in ems.
/// Read once a process ([`face_metrics`]).
pub(crate) struct FaceMetrics {
    /// The widest advance of any glyph of any face: what a character's
    /// glyph advances its line by at most, times its point size.
    pub(crate) widest: f32,
    /// For each code point some face maps, the narrowest advance of the
    /// glyph any face maps it to: what a character the shaper falls back
    /// for advances by at least, times its point size.
    pub(crate) narrowest: FxHashMap<u32, f32>,
    /// For each compiled family, and each code point a face of it maps, the
    /// narrowest advance of the glyph a face of it maps it to: what a
    /// character of a span in that family advances by at least.
    pub(crate) family: FxHashMap<AppFont, FxHashMap<u32, f32>>,
    /// The narrowest `.notdef` of any face: what a character no face maps
    /// advances by at least.
    pub(crate) notdef: f32,
}

impl FaceMetrics {
    /// The narrowest advance `c` may take in a span of `font`, in ems: the
    /// narrowest its family's faces map it to, where one maps it — the
    /// shaper lays a character in the family it is pinned to, or the
    /// default face, whenever a face of it holds the character — or the
    /// narrowest any face maps it to, where none does, or `.notdef`'s.
    /// O(1).
    pub(crate) fn narrowest_char(&self, font: AppFont, c: char) -> f32 {
        let font = if font == AppFont::Any {
            DEFAULT_FACE.0
        } else {
            font
        };
        let code = u32::from(c);
        self.family
            .get(&font)
            .and_then(|family| family.get(&code))
            .or_else(|| self.narrowest.get(&code))
            .copied()
            .unwrap_or(self.notdef)
    }

    /// The narrowest advance a grapheme cluster of `cluster` may take in a
    /// span of `font`, in ems: its one character's ([`Self::narrowest_char`]),
    /// or for a cluster of several the narrowest any face maps any of them
    /// to — a mark or a joiner of none, which the shaper may draw over its
    /// base or into one glyph with it, in whichever face holds the whole
    /// cluster. O(its characters).
    pub(crate) fn narrowest_cluster(&self, font: AppFont, cluster: &str) -> f32 {
        let mut chars = cluster.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => self.narrowest_char(font, c),
            _ => cluster
                .chars()
                .map(|c| self.narrowest.get(&u32::from(c)).copied().unwrap_or(self.notdef))
                .fold(f32::INFINITY, f32::min)
                .min(self.notdef),
        }
    }
}

/// The widest advance any glyph can have, in ems ([`FaceMetrics::widest`]).
/// O(1) once [`face_metrics`] has read the faces.
pub(crate) fn widest_advance() -> f32 {
    face_metrics().widest
}

/// [`FaceMetrics`] of every face the font system holds, read once a
/// process ([`face_advances`]). O(the font system's glyphs and code
/// points) the first time, O(1) after.
pub(crate) fn face_metrics() -> &'static FaceMetrics {
    static METRICS: OnceLock<FaceMetrics> = OnceLock::new();
    METRICS.get_or_init(|| {
        crate::font::fonts::init();
        let compiled: FxHashSet<ID> = COMPILED_FONT_ID_MAP
            .values()
            .flat_map(|ids| ids.iter().copied())
            .collect();
        let mut lease = FontSystemLease::new("the projection's work bound");
        let db = lease.fonts().db();
        // A face registered at several weights is one source, read once:
        // each face's source, and each compiled source's code points.
        let mut source_of: FxHashMap<ID, (usize, usize, u32)> = FxHashMap::default();
        let mut compiled_maps: FxHashMap<(usize, usize, u32), Vec<(u32, f32)>> = FxHashMap::default();
        let mut read: FxHashSet<(usize, usize, u32)> = FxHashSet::default();
        let mut metrics = FaceMetrics {
            widest: 0.0,
            narrowest: FxHashMap::default(),
            family: FxHashMap::default(),
            notdef: f32::INFINITY,
        };
        for face in db.faces() {
            db.with_face_data(face.id, |data, index| {
                let source = (data.as_ptr() as usize, data.len(), index);
                source_of.insert(face.id, source);
                if !read.insert(source) {
                    return;
                }
                let Some(face_read) = face_advances(data, index) else {
                    return;
                };
                metrics.widest = metrics.widest.max(face_read.widest);
                metrics.notdef = metrics.notdef.min(face_read.notdef);
                for &(code, least) in &face_read.codes {
                    narrowest_into(&mut metrics.narrowest, code, least);
                }
                if compiled.contains(&face.id) {
                    compiled_maps.insert(source, face_read.codes);
                }
            });
        }
        for (font, ids) in COMPILED_FONT_ID_MAP.iter() {
            let family = metrics.family.entry(*font).or_default();
            for codes in ids.iter().filter_map(|id| compiled_maps.get(source_of.get(id)?)) {
                for &(code, least) in codes {
                    narrowest_into(family, code, least);
                }
            }
        }
        if !metrics.notdef.is_finite() {
            metrics.notdef = 0.0;
        }
        metrics
    })
}

/// Keep `least` as `code`'s narrowest in `into` where it is narrower.
fn narrowest_into(into: &mut FxHashMap<u32, f32>, code: u32, least: f32) {
    into.entry(code)
        .and_modify(|was| *was = was.min(least))
        .or_insert(least);
}

/// What [`face_advances`] reads of one face, in ems.
struct FaceAdvances {
    /// Its widest glyph.
    widest: f32,
    /// Its `.notdef`, at its narrowest.
    notdef: f32,
    /// Each code point it maps, with the narrowest its glyph can be.
    codes: Vec<(u32, f32)>,
}

/// The advances of the face `index` of `data`, in ems: each glyph's
/// advance at its default instance, and in a variable face what its
/// variations add or take at most — every positive and every negative
/// delta its `HVAR` holds for it, each region's scalar being at most one,
/// and a unit either way for the rounding of their sum; in one with no
/// `HVAR`, its advance at every weight from 100 to 900, the one axis the
/// shaper sets (`wght`, the span's weight). `None` for a face that cannot
/// be read, which the shaper cannot lay a glyph from either. O(its glyphs
/// and code points), its glyphs 801 times over for a variable face with no
/// `HVAR`.
fn face_advances(data: &[u8], index: u32) -> Option<FaceAdvances> {
    use cosmic_text::skrifa::instance::{LocationRef, Size};
    use cosmic_text::skrifa::raw::tables::variations::DeltaSetIndex;
    use cosmic_text::skrifa::raw::TableProvider;
    use cosmic_text::skrifa::{FontRef, GlyphId, MetadataProvider, Tag};
    let font = FontRef::from_index(data, index).ok()?;
    let units = f32::from(font.head().ok()?.units_per_em());
    let glyphs = u32::from(font.maxp().ok()?.num_glyphs());
    let default = font.glyph_metrics(Size::unscaled(), LocationRef::default());
    let advance = |metrics: &cosmic_text::skrifa::metrics::GlyphMetrics<'_>, glyph: u32| {
        metrics.advance_width(GlyphId::new(glyph)).unwrap_or(0.0)
    };
    // Each glyph's narrowest and widest advance, in font units.
    let mut range: Vec<(f32, f32)> = (0..glyphs)
        .map(|g| {
            let at = advance(&default, g);
            (at, at)
        })
        .collect();
    if !font.axes().is_empty() {
        let varied = font.hvar().ok().and_then(|hvar| {
            let store = hvar.item_variation_store().ok()?;
            let mapping = hvar.advance_width_mapping().and_then(Result::ok);
            for (glyph, (least, most)) in range.iter_mut().enumerate() {
                let glyph = glyph as u32;
                let at = match &mapping {
                    Some(map) => map.get(glyph).ok(),
                    None => u16::try_from(glyph)
                        .ok()
                        .map(|inner| DeltaSetIndex { outer: 0, inner }),
                };
                let (taken, added) = at
                    .filter(|at| *at != DeltaSetIndex::NO_VARIATION_INDEX)
                    .and_then(|at| {
                        let deltas = store.item_variation_data().get(usize::from(at.outer))?.ok()?;
                        Some(
                            deltas
                                .delta_set(at.inner)
                                .fold((0i64, 0i64), |(taken, added), d| {
                                    (taken + i64::from(d.min(0)), added + i64::from(d.max(0)))
                                }),
                        )
                    })
                    .unwrap_or((0, 0));
                *least = (*least + taken as f32 - 1.0).max(0.0);
                *most += added as f32 + 1.0;
            }
            Some(())
        });
        if varied.is_none() {
            if let Some(axis) = font.axes().get_by_tag(Tag::new(b"wght")) {
                for weight in 100..=900u16 {
                    let weight = f32::from(weight).clamp(axis.min_value(), axis.max_value());
                    let location = font.axes().location([(Tag::new(b"wght"), weight)]);
                    let at = font.glyph_metrics(Size::unscaled(), &location);
                    for (glyph, (least, most)) in range.iter_mut().enumerate() {
                        let width = advance(&at, glyph as u32);
                        *least = least.min(width);
                        *most = most.max(width);
                    }
                }
            }
        }
    }
    let codes = font
        .charmap()
        .mappings()
        .filter_map(|(code, glyph)| {
            let &(least, _) = range.get(glyph.to_u32() as usize)?;
            Some((code, least / units))
        })
        .collect();
    Some(FaceAdvances {
        widest: range.iter().map(|&(_, most)| most).fold(0.0f32, f32::max) / units,
        notdef: range.first().map_or(f32::INFINITY, |&(least, _)| least / units),
        codes,
    })
}

/// The projection properties of a map the loader accepted (see the module
/// docs): `Ok` when they hold, when the map is outside the bound, or when
/// its untangling reaches [`MAX_UNTANGLE_SAMPLES`] — each skipped, and
/// counted.
pub fn check_projects(doc: &Document) -> Result<(), String> {
    if !within_bound(doc) {
        SKIPPED.with(|n| n.set(n.get() + 1));
        return Ok(());
    }
    #[cfg(any(test, fuzzing))]
    {
        let (checked, cut) = under_ceiling(|| check_within_ceiling(doc));
        // Cut short, the map was not laid out as it lays out: whatever the
        // check found, it is not this map's, and is not reported.
        if cut {
            CUT.with(|n| n.set(n.get() + 1));
            return Ok(());
        }
        PROJECTED.with(|n| n.set(n.get() + 1));
        checked
    }
    #[cfg(not(any(test, fuzzing)))]
    {
        PROJECTED.with(|n| n.set(n.get() + 1));
        check_within_ceiling(doc)
    }
}

/// `check` under the untangling ceiling, [`MAX_UNTANGLE_SAMPLES`], and
/// whether the ceiling cut it short. The ceiling and the mark are clear
/// again however `check` ends, by return or by panic. Test and fuzz
/// builds only. O(`check`).
#[cfg(any(test, fuzzing))]
pub(crate) fn under_ceiling<R>(check: impl FnOnce() -> R) -> (R, bool) {
    use crate::projection::geometry::{UNTANGLE_CEILING, UNTANGLE_CUT};
    /// Clears the ceiling and the mark when it goes.
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            UNTANGLE_CEILING.with(|ceiling| ceiling.set(None));
            UNTANGLE_CUT.with(|cut| cut.set(false));
        }
    }
    let _clear = Clear;
    UNTANGLE_CEILING.with(|ceiling| ceiling.set(Some(MAX_UNTANGLE_SAMPLES)));
    UNTANGLE_CUT.with(|cut| cut.set(false));
    let checked = check();
    (checked, UNTANGLE_CUT.with(Cell::get))
}

/// [`check_projects`] once the map is inside the bound, under the ceiling
/// it sets: abandoned, with `Ok`, as soon as its first projection is cut
/// short.
fn check_within_ceiling(doc: &Document) -> Result<(), String> {
    // In the test build the loader suites' maps, up to a hundred thousand
    // nodes, are projected sharing the shapings of equal keys as the
    // oracle's full projections do (`stages::memo`): every property here
    // compares by value. A fuzz build shapes every flow.
    #[cfg(test)]
    let mut p = Projected::new_shared(doc.clone());
    #[cfg(not(test))]
    let mut p = Projected::new(doc.clone());
    #[cfg(any(test, fuzzing))]
    if crate::projection::geometry::UNTANGLE_CUT.with(Cell::get) {
        return Ok(());
    }
    for layer in Layer::ALL {
        let tree = p.layers.tree(layer);
        let mut groups = 0;
        for glyph in tree.root().descendants(tree.arena()) {
            match tree.arena()[glyph].get().glyph_run() {
                None => groups += 1,
                Some(run) => {
                    let (corner, size) = run.bounds();
                    if !(corner.is_finite() && size.is_finite()) {
                        return Err(format!("a {layer:?} run's box is not finite: {corner} {size}"));
                    }
                }
            }
        }
        if groups != p.projector.nodes.len() + 1 {
            return Err(format!(
                "the {layer:?} layer holds {groups} groups for {} visible nodes and its root",
                p.projector.nodes.len()
            ));
        }
    }
    let mut points = vec![Vec2::ZERO];
    points.extend(p.projector.nodes.values().take(8).map(|n| n.rect.center()));
    for point in points {
        p.projector.hit(&mut p.layers, point);
    }
    // The same layers when projected again (the sweep's first check: a
    // full projection compared with these), then one edit through the
    // oracle.
    let mut c = Choices::seeded(doc.tree().arena().len() as u64);
    let mut covered = FxHashMap::default();
    let mut turn = 0;
    sweep("a loaded map", &mut p, &mut c, 1, &mut turn, &mut covered)
}

/// The projection_edits target's body: a generated map, and up to sixteen
/// steps drawn from `bytes`, each through the oracle (see the module
/// docs).
pub fn check_projection_edits(bytes: &[u8]) -> Result<(), String> {
    let mut c = Choices::new(bytes);
    let doc = generated(&mut c);
    let steps = 1 + c.below(16) as usize;
    let mut p = Projected::new(doc);
    let mut covered = FxHashMap::default();
    let mut turn = c.index(EDITS.len());
    sweep("projection_edits", &mut p, &mut c, steps, &mut turn, &mut covered)
}

/// Take `p` through `steps` edits — the edit [`EDITS`] names at `turn`,
/// advancing it, its targets and values drawn from `c`, applied and, one
/// in three, undone and redone — transaction stages, and batches: two to
/// five edits and undos applied before one update is fed every record
/// they made, as a frame feeds them (the review of #317). The oracle is
/// checked after every update; each edit applied is counted in
/// `covered`. `Err` names the case, the step and the edit.
pub fn sweep(
    case: &str,
    p: &mut Projected,
    c: &mut Choices<'_>,
    steps: usize,
    turn: &mut usize,
    covered: &mut FxHashMap<&'static str, usize>,
) -> Result<(), String> {
    p.check()
        .map_err(|e| format!("{case}, the full projection: {e}"))?;
    for step in 0..steps {
        let at = |what: &str, e: String| format!("{case}, step {step}, {what}: {e}");
        if c.one_in(6) {
            // A batch: several records, one update.
            let mut names = Vec::new();
            for _ in 0..2 + c.below(4) {
                if c.one_in(4) {
                    if p.doc.undo().is_some() {
                        names.push("undo");
                    }
                    continue;
                }
                let name = EDITS[*turn % EDITS.len()];
                *turn += 1;
                if let Some((name, req)) = edit_named(c, &p.doc, name) {
                    if p.doc.apply(req, ChangeSource::Action).is_ok() {
                        *covered.entry(name).or_default() += 1;
                        names.push(name);
                    }
                }
            }
            if names.len() > 1 {
                *covered.entry("batch").or_default() += 1;
            }
            p.sync_and_discard();
            p.check().map_err(|e| at(&format!("the batch {names:?}"), e))?;
            continue;
        }
        if c.one_in(6) {
            // A transaction: two stages, then a commit or a cancel; an
            // animated one is interpolated half-way first.
            let kind = if c.one_in(2) {
                TransactionKind::Interactive
            } else {
                TransactionKind::Animated
            };
            let Ok(tx) = p.doc.begin(ChangeSource::Input, kind) else {
                continue;
            };
            for stage in 0..2 {
                let Some((name, req)) = edit(c, &p.doc) else {
                    continue;
                };
                if p.doc.stage(&tx, req).is_ok() {
                    *covered.entry("transaction::stage").or_default() += 1;
                    p.sync_and_discard();
                    p.check().map_err(|e| at(&format!("stage {stage} ({name})"), e))?;
                }
            }
            if kind == TransactionKind::Animated && p.doc.stage_progress(&tx, 0.5, Snap::End).is_ok() {
                *covered.entry("transaction::interpolate").or_default() += 1;
                p.sync_and_discard();
                p.check().map_err(|e| at("interpolate", e))?;
            }
            if c.one_in(2) {
                p.doc.commit(tx);
            } else {
                p.doc.cancel(tx);
            }
            p.sync_and_discard();
            p.check().map_err(|e| at("commit or cancel", e))?;
            continue;
        }
        let name = EDITS[*turn % EDITS.len()];
        *turn += 1;
        let Some((name, req)) = edit_named(c, &p.doc, name) else {
            continue;
        };
        if p.apply(req).is_err() {
            continue;
        }
        *covered.entry(name).or_default() += 1;
        p.check().map_err(|e| at(name, e))?;
        if c.one_in(3) {
            p.doc.undo();
            p.sync_and_discard();
            p.check().map_err(|e| at(&format!("undo of {name}"), e))?;
            p.doc.redo();
            p.sync_and_discard();
            p.check().map_err(|e| at(&format!("redo of {name}"), e))?;
            *covered.entry("history::undo_and_redo").or_default() += 1;
        }
    }
    Ok(())
}
