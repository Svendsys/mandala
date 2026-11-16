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
//! and it draws at most
//! [`MAX_UNITS`](crate::projection::tests::projection_fuzz::MAX_UNITS)
//! sections, rings, connectors and links.
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
//! `test_every_dot_ring_the_bound_admits_takes_a_few_steps_a_glyph`). The
//! bound holds where a ring lies, not how much work it is: a `Repeat` ring
//! a little short of the cap takes some 40 steps a glyph wherever it lies,
//! and inside the bound such a ring took 40 to 118 s of a check under ASan
//! against the 20 s timeout (`DOT_STEPS_A_GLYPH` in the tests records the
//! measurements; what bounds that work in this harness is open, the review
//! of #378). A `spacing` is not part of the bound: it parts a placement's
//! glyphs, and moves no outline and no path.
//!
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
use crate::document::resolve::{resolve_frame, resolve_text_in, Binding};
use crate::document::writer::transaction::TransactionKind;
use crate::document::writer::ChangeSource;
use crate::engine::interpolate::Snap;
use crate::engine::tree::Tree;
use crate::flow::tests::flow_fuzz::MAX_FRAME_GLYPHS;
use crate::flow::{Rect, LINE_HEIGHT_FACTOR};
use crate::font::fonts::{FontSystemLease, MAX_FONT_SIZE_PT};
use crate::projection::geometry;
use crate::projection::tests::corpus::{edit, edit_named, generated, EDITS};
use crate::projection::tests::support::Projected;
use crate::projection::Layer;
use crate::util::tests::choices::Choices;

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
/// for [`widest_advance`].
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
            DocElement::Frame(_) => outline_near(tree, id),
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

/// The widest advance, in ems, of any glyph of any face the font system
/// holds — the compiled faces and, on native, the host's — at any
/// instance the shaper lays a variable face at (ruling B47.2; see the
/// module docs): what a character's glyph advances its line by at most,
/// times its point size. Read once a process, from every face
/// ([`face_widest_advance`]). O(the font system's glyphs) the first time,
/// O(1) after.
pub(crate) fn widest_advance() -> f32 {
    static WIDEST: OnceLock<f32> = OnceLock::new();
    *WIDEST.get_or_init(|| {
        crate::font::fonts::init();
        let mut lease = FontSystemLease::new("the projection's work bound");
        let db = lease.fonts().db();
        // A face registered at several weights is one source read once.
        let mut read: FxHashSet<(usize, usize, u32)> = FxHashSet::default();
        let mut widest = 0.0f32;
        for face in db.faces() {
            let most = db.with_face_data(face.id, |data, index| {
                read.insert((data.as_ptr() as usize, data.len(), index))
                    .then(|| face_widest_advance(data, index))
                    .flatten()
            });
            widest = widest.max(most.flatten().unwrap_or(0.0));
        }
        widest
    })
}

/// The widest advance of a glyph of the face `index` of `data`, in ems:
/// each glyph's advance at its default instance and, in a variable face,
/// the most its variations add — every positive delta its `HVAR` holds for
/// it, and a unit for the rounding of their sum, since each region's
/// scalar is at most one; in one with no `HVAR`, its advance at every
/// weight from 100 to 900, the one axis the shaper sets (`wght`, the
/// span's weight). `None` for a face that cannot be read, which the shaper
/// cannot lay a glyph from either. O(its glyphs), times 801 for a variable
/// face with no `HVAR`.
fn face_widest_advance(data: &[u8], index: u32) -> Option<f32> {
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
    let most = (0..glyphs).map(|g| advance(&default, g)).fold(0.0f32, f32::max);
    if font.axes().is_empty() {
        return Some(most / units);
    }
    // Every glyph's default advance and its positive deltas: `None` where
    // the face has no `HVAR`, or one that cannot be read.
    let varied = font.hvar().ok().and_then(|hvar| {
        let store = hvar.item_variation_store().ok()?;
        let mapping = hvar.advance_width_mapping().and_then(Result::ok);
        let mut most = most;
        for glyph in 0..glyphs {
            let at = match &mapping {
                Some(map) => map.get(glyph).ok(),
                None => u16::try_from(glyph)
                    .ok()
                    .map(|inner| DeltaSetIndex { outer: 0, inner }),
            };
            let grown: i64 = at
                .filter(|at| *at != DeltaSetIndex::NO_VARIATION_INDEX)
                .and_then(|at| {
                    let deltas = store.item_variation_data().get(usize::from(at.outer))?.ok()?;
                    Some(deltas.delta_set(at.inner).map(|d| i64::from(d.max(0))).sum())
                })
                .unwrap_or(0);
            most = most.max(advance(&default, glyph) + grown as f32 + 1.0);
        }
        Some(most)
    });
    if let Some(varied) = varied {
        return Some(varied / units);
    }
    let mut most = most;
    if let Some(axis) = font.axes().get_by_tag(Tag::new(b"wght")) {
        for weight in 100..=900u16 {
            let weight = f32::from(weight).clamp(axis.min_value(), axis.max_value());
            let location = font.axes().location([(Tag::new(b"wght"), weight)]);
            let metrics = font.glyph_metrics(Size::unscaled(), &location);
            most = (0..glyphs).map(|g| advance(&metrics, g)).fold(most, f32::max);
        }
    }
    Some(most / units)
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
