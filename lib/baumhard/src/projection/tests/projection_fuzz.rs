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
//!   bound is skipped for this property and counted, and so, apart, is
//!   one whose untangling reaches the ceiling below
//!   ([`projection_counts`](crate::projection::tests::projection_fuzz::projection_counts))
//!   and one the check's own edit takes out of the bound
//!   ([`projection_left`](crate::projection::tests::projection_fuzz::projection_left)),
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
//! extent an `f32` point is a thousandth of a unit or finer, and a glyph's
//! ink is set apart from the last in a few reads; 10¹⁰ out a point is a
//! thousand units from the next, and each glyph of a ring of dots read its
//! path a thousand times, 62 steps a glyph, every projection a check makes
//! (#374). A `spacing` is not part of where a ring lies: it parts a
//! placement's glyphs, and moves no outline and no path.
//!
//! **A ring's work is part of the bound** (ruling B47.3). Where a ring lies
//! does not bound how much work it is: a `Repeat` ring a little short of
//! the cap closes its spacing and takes some 40 steps a glyph wherever it
//! lies, and inside the extent such rings took 40 to 118 s of a check
//! under ASan, against the 20 s timeout. So a map is projected here only
//! when each `Repeat` ring may lay no more glyphs than `MOST_RING_GLYPHS`
//! (`repeat_glyphs`, bounded before it is laid): the most whose work —
//! `RING_STEPS_A_GLYPH` a glyph and `RING_SETUP_STEPS`, stated from sweeps
//! of the rings the bound admits — laid `RING_LAYS_A_CHECK` times, once in
//! each projection the check makes, fits the input's share,
//! `flow_fuzz::INPUT_STEPS`. The flow harnesses hold their rings to
//! `MAX_RING_GLYPHS` for the same reason (B17.2b). A ring's glyphs are
//! bounded as its outline's perimeter at its reach over the least step one
//! of them takes along it: a copy of its text shaped as the ring shapes it,
//! at the least size the ring can be, each of its clusters at the least
//! step placement can give it at any direction (`least_steps`; ruling
//! B47.4) — a dash, stepped by its ink, takes a hair where the path runs
//! steep — and its `spacing`, with `RING_STEP_SLACK` more for where a ring
//! turns. A ring of text laid by halves — any other fit — is held the
//! same way, to the steps one lay of it may take, `FRAME_STEPS_A_LAY`: its
//! text's characters and the glyphs its side ornament may lay
//! (`side_glyphs`), at `FRAME_STEPS_A_GLYPH` and `FRAME_STEPS_A_SIDE_GLYPH`,
//! and `FRAME_SETUP_STEPS` (`frame_steps`). The flow harnesses lay such a
//! frame once an input and give it the whole share; a check here lays it
//! up to ten times. The rates are measured, and an input no sweep tried can
//! break them: the check holds each ring the fuzzer lays to them
//! (`rings_within_their_work`), so a fuzz run finds the next one.
//!
//! **The shaper's `.notdef` passes are not measured** (#387). A lay that
//! shapes text again — a frame's reshape of a text that holds a tab, its
//! side ornament's shapings, a directional ring's one shaping of its texts
//! — pays the shaper's passes for a glyph no face draws: 2,200 steps for
//! each `.notdef` of one code point, and for each glyph of a line holding a
//! `.notdef` of several (`flow::place::halves::notdef_steps`), past any
//! rate a sweep of drawn text measured. CI's load_structured job on #380
//! found a frame of a tab beside U+0001 and a keycap no compiled face
//! draws, whose lay took 14,157 steps where the rates gave it 9,344. Which
//! line may lay a `.notdef` is read from the map: one no one face of the
//! font system maps all of (`may_lay_notdef`).
//!
//! - **A frame laid by halves that may pay them is excluded, not counted**
//!   (`frame_may_shape_notdef`): one with a side ornament or a `once`
//!   filler, or `Fit` with a tab in its text, a line of which may lay a
//!   `.notdef`. Its lay may
//!   shape such a line dozens of times, each a pass a glyph, and no cap of
//!   the engine's holds that below a lay's share here, so there is no count
//!   to grade it by. What that costs: such frames are never projected by a
//!   check — CI's own finding among them, on CI's faces, though it lays a
//!   third of the share — nor is a frame of several scripts no one face
//!   draws beside an ornament, or `Fit` with a tab, though it lays none.
//! - **A directional ring's one shaping is counted, exactly**
//!   (`directional_notdef_steps`): where a line of it may lay a `.notdef`,
//!   the bound shapes its texts, at the least size, which the charge does
//!   not depend on, and adds what that reshape charges
//!   (`flow::reshape_steps`), on any host and any face.
//! - A `Repeat` ring of spans alone shapes nothing as it lays, and pays no
//!   pass.
//!
//! The faces are the font system's, the host's with the compiled ones, as
//! the shaper's are: on CI's runner U+0001 is drawn by no face, and on a
//! host with Unifont it is, and in each the bound reads what that host
//! lays.
//!
//! **The check's own edit is held to the bound too** (the review of #378).
//! The bound reads the map as loaded; the check then edits it, and every
//! projection after the edit lays the map as edited. An edit can take an
//! admitted map out by any rule of the bound — a ring made `Repeat`, a node
//! moved or resized, a section's or a ring's text, an `inset` or a `gap`,
//! a side ornament — so the check reads the bound again after each change
//! and before its update, and abandons a map that left it, counting it
//! apart (`left_the_bound`, `projection_left`).
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
//! can ask for, the only axis the shaper sets. No glyph is laid wider.
//!
//! **What the bound takes on trust.** That the shaper lays no more than the
//! widest advance for a character of a section: a face's substitutions can
//! lay one character as several glyphs, and its positioning can widen a
//! pair, and a face that did either past the widest advance would draw its
//! box wider than the bound reads, by that excess. That a side ornament's
//! character advances no less than the narrowest glyph a face maps it to,
//! unless it is one of the code points a shaper may hide, which the bound
//! reads as advancing by nothing, a lower bound on any host
//! (`DEFAULT_IGNORABLE`;
//! `test_a_side_ornament_of_default_ignorables_lays_within_its_bound`).
//! That a `Repeat` ring steps along its outline no less than its clusters'
//! least steps over `RING_STEP_SLACK`. That a face whose `cmap` maps every
//! character of a cluster lays none of it as `.notdef`; that the shaper
//! tries every face before it gives a glyph up; and that a face that draws
//! a missing cluster forms it from the same characters, so its cluster
//! starts where the missing glyph does, which the fallback's merge needs
//! to take it — a face that ligated a character the first face drew with
//! one it lacked would not be taken (`may_lay_notdef`). And the rates
//! above. The end-to-end
//! properties (`test_every_ring_the_bound_admits_lies_within_the_extent`,
//! and in every check `rings_within_their_work`) hold the bound's own
//! cases, the reviews' probes and every map the fuzzer lays to them.
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
//! is at most the ceiling and the one lane that reaches it. The rest of
//! the check is bounded by the bound above and the engine's own caps, but
//! for shaping: every full projection shapes every flow again — a
//! section's, a ring's, a path's — and no step counts it (#383), so what
//! a check's projections spend shaping text in many faces is not held
//! here.

use std::cell::Cell;
use std::sync::OnceLock;

use glam::Vec2;
use rustc_hash::{FxHashMap, FxHashSet};

use crate::arena::NodeId;
use crate::document::element::{DocElement, RingData, SectionData};
use crate::document::ids::Document;
use crate::document::resolve::{resolve_frame, resolve_ring_in, resolve_text_in, Binding, ResolvedText};
use crate::document::values::{FitMode, FontRef, FontSize};
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
    static LEFT: Cell<usize> = const { Cell::new(0) };
    /// Whether the check under way is of a map the bound admitted, which
    /// its edit must keep inside it ([`left_the_bound`]).
    static HELD: Cell<bool> = const { Cell::new(false) };
    /// Whether the check's edit took its map out of the bound.
    static GONE: Cell<bool> = const { Cell::new(false) };
    /// Test counter: the most glyph steps of a ring's lay that the work
    /// property held (`rings_within_their_work`, which records it as it
    /// returns) at its last read after the check's last change — the
    /// after-edit read in `updated`, and in a transaction the one after its
    /// commit or cancel, not after the stages before it. `Some(0)` where
    /// that change was made and the property read nothing after it, and
    /// `None` where the check made no change that kept the map inside the
    /// bound.
    #[cfg(test)]
    pub(crate) static LAID_AFTER: Cell<Option<usize>> = const { Cell::new(None) };
}

/// How many loaded maps [`check_projects`] abandoned because the check's
/// own edit took the map out of the bound (the review of #378,
/// `left_the_bound`), on this thread: counted apart from
/// [`projection_counts`]'s three, as a map cut short is.
pub fn projection_left() -> usize {
    LEFT.with(Cell::get)
}

/// Whether the map `p` holds has left the bound in a check of a map it
/// admitted (the review of #378): read again after each edit, stage,
/// interpolation, commit, cancel, undo and redo the check makes, before
/// the update and the full projection that would lay it. An edit can take
/// a map out by any rule of the bound — a ring's or a frame's `fit` or the
/// defaults made `Repeat` (`Ring::SetFit`, `Frame::SetFit`,
/// `Defaults::Replace`), a node moved or resized, a section's text, a
/// frame's `inset` or `gap`, a ring's text, a side ornament — and the lays
/// after it are of the map as edited, which the bound never read: a ring of
/// `·` made `Repeat` 1,300 out took a check 26.6 s under ASan. Marks the
/// check, which is then abandoned and counted ([`projection_left`]).
/// O(the document) where a check is held, O(1) where none is.
fn left_the_bound(p: &Projected) -> bool {
    if !HELD.with(Cell::get) {
        return false;
    }
    if within_bound(&p.doc) {
        // The lays that follow are of the map as changed: their most is
        // counted afresh ([`laid_within`]).
        #[cfg(any(test, fuzzing))]
        crate::flow::MOST_RING_LAY.with(|most| most.set(0));
        #[cfg(any(test, fuzzing))]
        crate::flow::MOST_OPEN_LAY.with(|most| most.set(0));
        #[cfg(test)]
        LAID_AFTER.with(|laid| laid.set(Some(0)));
        return false;
    }
    GONE.with(|gone| gone.set(true));
    true
}

/// Whether the rings `p` laid since the bound was last read held to the
/// work it read for them (`rings_within_their_work`), in a check of a
/// map the bound admitted: what the sweep asks of every update and full
/// projection after its first, as of the first. `Ok` with the most glyph
/// steps of a ring's lay the property held, as it returns them; `Ok(0)`
/// where no check is held, and in a build that counts no steps. O(`p`'s
/// rings and their texts) where a check is held, O(1) where none is.
fn laid_within(p: &Projected) -> Result<usize, String> {
    #[cfg(any(test, fuzzing))]
    if HELD.with(Cell::get) {
        return rings_within_their_work(p, crate::flow::MOST_RING_LAY.with(Cell::get));
    }
    let _ = p;
    Ok(0)
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
/// commit (`sweep`). A fuzz build shares no placement between them. Each
/// lays a map the bound admitted: the check reads the bound again after
/// each change, before the lays that follow it (`left_the_bound`).
pub const RING_LAYS_A_CHECK: usize = 10;

/// The most glyph steps a `Repeat` ring the bound admits takes for each
/// glyph `repeat_glyphs` bounds it to, past [`RING_SETUP_STEPS`] (rulings
/// B47.3, B47.4). Stated from measurement: the engine's own bound is its
/// budget, `STEPS_PER_GLYPH`, 3,000 a glyph over a floor of 400,000 steps a
/// lay (B17.2c), under which ten lays fit no ring at all into the input's
/// share, and its spacing solves are read against that budget and nothing
/// less. Three sweeps. 576 rings of the default face — `·` at a quarter and
/// at the whole of its node, `MANDALA `, `*` at a twentieth, and `·`, a
/// ring of `|` and one of corners spaced, round an ellipse, a sharp and a
/// rounded rectangle 1 to 1,000 across, from the node's side to 3,000 out.
/// After the review of #378 found a ring past the first rate, 420 rings of
/// 31 texts laid in fallback faces and as `.notdef`: emoji, Hebrew, Arabic,
/// Devanagari, Thai, Tamil, Mongolian, Cherokee, hieroglyphs, U+1BCA0,
/// U+F0000, U+10FFFD, U+FDFD, split vowels, alone and beside `·`, round an
/// ellipse and a rounded rectangle at their edge of the bound, at half and
/// at a quarter of it; the most was 206 a bound glyph. And after ruling
/// B47.4 bounded a ring's step by the least its clusters can take
/// (`least_steps`), which admits rings of dashes and bars spaced, 1,890
/// rings of 36 texts — those, `·`, `*`, `MANDALA `, dashes, bars and
/// ellipses of the default face — at a twentieth and a quarter of their
/// node, spaced 0, 0.1 and 1, round an ellipse, a sharp and a rounded
/// rectangle, at the largest node up to 1,000 across the bound admits them
/// round, at their edge, half of it and on its side. Laid with the bound as
/// it stood, at 256 a glyph, the most any took was 298 steps a glyph of its
/// bound: `😀·` at a quarter of an ellipse 1,000 across, 2,234 out, 52,585
/// steps for 110 glyphs laid of 149 bounded. 384 holds that with a quarter
/// as much again, rounded up to a multiple of 128; at 384 the bound admits
/// 1,854 of those rings, the most of which took 249 a glyph past
/// [`RING_SETUP_STEPS`]. Measured rates are what an input no sweep tried
/// breaks: the check holds every ring the fuzzer lays to them
/// (`rings_within_their_work`), so a fuzz run finds the next one.
/// `test_every_repeat_ring_the_bound_admits_fits_the_share` holds the
/// densest rings the bound admits to it.
pub const RING_STEPS_A_GLYPH: usize = 384;

/// How many more glyphs a `Repeat` ring may lay than its outline's length
/// over the least its glyphs step (`least_steps`; ruling B47.3, the review
/// of #378): round a turn and across its seam a ring sets each glyph's ink
/// as far from the last as a line would, which its step does not read. A
/// margin, not a measured overshoot: no ring in the tests or the sweeps
/// lays past the bound without it, so it has no red control, and only the
/// property every check of a fuzz run holds (`rings_within_their_work`)
/// would find a ring that needs it. The closest any came, on this
/// container, was `·\u{1BCA0}` at a twentieth of a sharp rectangle 10
/// across: 126 glyphs laid, where the bound without the slack reads about
/// 127 (158 with it). A quarter more.
pub const RING_STEP_SLACK: f64 = 1.25;

/// The most glyph steps a `Repeat` ring the bound admits takes past
/// [`RING_STEPS_A_GLYPH`] a glyph of its bound (ruling B47.3): its setup,
/// its path's tables, and shaping and inking its copy, which do not grow
/// with its glyphs. Stated from the same sweeps: the first two's most past
/// 64 a glyph was 5,481, the third's past 384 a glyph 1,344; rounded up to
/// a power of two.
pub const RING_SETUP_STEPS: usize = 8_192;

/// The most glyphs a `Repeat` ring the bound admits may lay (rulings
/// B47.3, B47.4), 99: the most whose work, [`RING_STEPS_A_GLYPH`] a glyph
/// and [`RING_SETUP_STEPS`], laid [`RING_LAYS_A_CHECK`] times, fits the
/// input's share, `flow_fuzz::INPUT_STEPS` at `flow_fuzz::ASAN_STEP_NANOS` a
/// step — the flow harnesses' share for an input, whose rings they lay at
/// `MAX_RING_GLYPHS` (B17.2b) for the same reason. The densest ring it
/// admits, by the steps it takes, `😀·` at a quarter of an ellipse 1,000
/// across, 1,304 out — 72 glyphs laid of 99 bounded, 32,861 steps a lay —
/// ran its input in 0.73 s under ASan (`cargo fuzz run load_raw <file> --
/// -runs=1`, the load average some 3), against the 20 s timeout,
/// `flow_fuzz::FUZZ_TIMEOUT_SECONDS`.
pub const MOST_RING_GLYPHS: usize = (INPUT_STEPS / RING_LAYS_A_CHECK - RING_SETUP_STEPS) / RING_STEPS_A_GLYPH;

/// The most glyphs `ring` may lay, laid `Repeat` (ruling B47.3), bounded
/// before it is laid: the perimeter its outline can have, over the least
/// one of its glyphs can step along it, with [`RING_STEP_SLACK`] more, and
/// a copy and its corners more (rulings B47.3, B47.4). `None` for a ring of
/// another fit, which lays its text once.
///
/// - **The perimeter**: its node's drawn rectangle at most
///   ([`drawn_bound`]) grown by its frame's outermost reach
///   ([`outline_reach`]), whose perimeter, 2 (w + h) + 8 × reach, is at
///   least that of any convex outline inside it — every node shape is
///   convex, and an outline grown from one is.
/// - **A glyph's step at the least**: each text it lays — its spans', each
///   in its own style, or the `vertical` its spans have nothing to draw
///   beside — shaped as the ring shapes a copy of it (`flow::shape_run`),
///   at the least point size its ring can be: the least a copy can step
///   along any path, each cluster by the rule placement steps it by
///   ([`least_steps`]), and its `spacing`, which a `Repeat` ring adds to
///   every cluster's step, over its glyphs, however many a character lays.
///   The size is relative to its node's drawn rectangle, at least
///   `min_size` (D5), and so at least `min_size`'s point size. A ring that
///   lays by direction steps a run down a side by its glyphs' ink, upright,
///   and not by their advance (`place::directional`): its `spacing` alone
///   is a step it takes at least. One whose step can be nothing, or less,
///   lays the cap ([`glyphs_round`]).
/// - The glyphs of a copy past that, a copy on each of the four runs of a
///   ring laid by direction, and its corners; and the cap,
///   `MAX_PLACED_GLYPHS`, at most.
///
/// What it takes on trust: that the ring steps along its outline no less
/// than its clusters' least steps over [`RING_STEP_SLACK`] — round a turn
/// and across its seam it sets inks apart as a line would — as every ring
/// the end-to-end property reads does (`rings_within_the_extent` holds what
/// each lays to this, and `rings_within_their_work` holds every ring a fuzz
/// run lays). O(its texts), shaped once each.
pub(crate) fn repeat_glyphs(tree: &Tree<DocElement>, frame: NodeId, ring: NodeId) -> Option<usize> {
    use crate::flow::{shape_run, FlowSpan};
    let arena = tree.arena();
    let resolved = resolve_ring_in(tree, ring, Binding::Carried(None), &mut ());
    if !matches!(resolved.fit, FitMode::Repeat) {
        return None;
    }
    let node = frame.parent(arena)?;
    let DocElement::Node(data) = arena[node].get() else {
        return None;
    };
    // The least rectangle its node is drawn as, and so the least size its
    // text is shaped at (D5).
    let least_rect = Rect::new(Vec2::ZERO, geometry::usable_size(data.min_size));
    let at_least = |style: ResolvedText| ResolvedText {
        size: FontSize::Pt(geometry::points(style.size, least_rect)),
        ..style
    };
    let style = at_least(resolve_text_in(tree, ring, Binding::Carried(None), &mut ()));
    // Each text it lays: its spans', one copy, each in its own style, and
    // its `vertical`, in its own.
    let spans: Vec<(ResolvedText, &str)> = ring
        .children(arena)
        .filter_map(|span| match arena[span].get() {
            DocElement::Span(text) => Some((
                at_least(resolve_text_in(tree, span, Binding::Carried(None), &mut ())),
                text.text.as_str(),
            )),
            _ => None,
        })
        .filter(|(_, text)| !text.is_empty())
        .collect();
    let mut texts: Vec<Vec<(ResolvedText, &str)>> = Vec::new();
    if !spans.is_empty() {
        texts.push(spans);
    }
    if let Some(vertical) = resolved.directional.vertical.as_deref() {
        texts.push(vec![(style, vertical)]);
    }
    let corners: usize = resolved
        .directional
        .corners
        .as_ref()
        .map_or(0, |corners| corners.iter().map(|c| c.chars().count()).sum());
    if texts.is_empty() {
        return Some(corners.min(MAX_PLACED_GLYPHS));
    }
    let spacing = if resolved.spacing.is_finite() {
        f64::from(resolved.spacing.max(0.0))
    } else {
        0.0
    };
    // Each text shaped as the ring shapes a copy of it, at the least size:
    // its glyphs and how far a copy advances.
    crate::font::fonts::init();
    let mut lease = FontSystemLease::new("the projection's work bound");
    // The least a glyph of each text steps along the outline, and the most
    // glyphs a copy of any holds.
    let mut least = f64::INFINITY;
    let mut copy = 0usize;
    for text in &texts {
        let spans: Vec<FlowSpan<'_>> = text
            .iter()
            .map(|(style, text)| FlowSpan {
                text,
                style,
                link: None,
            })
            .collect();
        let run = shape_run(&spans, 1.0, &mut lease);
        let glyphs = run.glyphs.len();
        if glyphs == 0 {
            continue;
        }
        let (steps, clusters) = least_steps(&run);
        // A ring that lays by direction steps a run down a side by its
        // glyphs' ink, upright, not by their advance (`place::directional`):
        // only its `spacing` is a step it takes at least.
        let step = if resolved.directional.is_set() {
            clusters as f64 * spacing
        } else {
            steps + clusters as f64 * spacing
        };
        least = least.min(step / glyphs as f64);
        copy = copy.max(glyphs);
    }
    drop(lease);
    if copy == 0 {
        return Some(corners.min(MAX_PLACED_GLYPHS));
    }
    let drawn = drawn_bound(tree, node);
    let reach = outline_reach(tree, frame);
    let perimeter = 2.0 * f64::from(drawn.size.x + drawn.size.y) + 8.0 * reach;
    // A ring laid by direction lays a glyph at least on each of its four
    // runs, however short.
    let runs = if resolved.directional.is_set() { 4 } else { 1 };
    Some(glyphs_round(perimeter, least, runs * copy + corners))
}

/// The most glyphs a ring may lay round an outline of `perimeter` whose
/// glyphs each step `least` along it at least, and `more` past those: the
/// perimeter over `least`, with [`RING_STEP_SLACK`] more — round a turn
/// and across its seam a ring sets each glyph's ink as far from the last
/// as a line would, which `least` does not read — at most the cap,
/// `MAX_PLACED_GLYPHS`. The cap where `least` is not more than nothing: a
/// copy whose glyphs may step back as far as on (`least_steps`) gains no
/// ground the bound can count. O(1).
pub(crate) fn glyphs_round(perimeter: f64, least: f64, more: usize) -> usize {
    let glyphs = perimeter / least * RING_STEP_SLACK;
    if !(least > 0.0 && glyphs.is_finite() && glyphs < MAX_PLACED_GLYPHS as f64) {
        return MAX_PLACED_GLYPHS;
    }
    (glyphs.ceil() as usize + more).min(MAX_PLACED_GLYPHS)
}

/// The least a copy of `run` steps along any path, and its grapheme
/// clusters (ruling B47.4): the sum over its clusters of the least step
/// each can take, by the rule placement steps it by (`flow::place`,
/// `PathWalk::take`), at every direction a chord can run. A cluster of
/// text is a box its advance wide and the run's line high, whose extent
/// along a chord is at least the lesser of the two. In an ornament — a run
/// with no letter or digit — a cluster with ink steps by its ink box's
/// extent along the chord, at least the lesser of the box's width and
/// height, plus its side bearings, its advance less the ink's width, which
/// do not turn with the path and are negative where the ink overhangs it:
/// a dash, as wide as its advance and a hair high, steps by a hair where
/// the path runs steep. A blank cluster, or one that draws nothing, steps
/// its advance. An ornament's step is never less than nothing, as `take`
/// floors it; a text cluster's is not floored, as `take` does not floor a
/// box's (`PathWalk::step`), and a cluster whose advances sum to less than
/// nothing steps back: a glyph's advance is the shaper's `x_advance`, which
/// a face's positioning (GPOS) can make negative — a pair adjustment wider
/// than the advance — so one can reach `take` from a face a host holds.
/// The `spacing` a ring adds to each step is not counted here. O(its
/// glyphs).
pub(crate) fn least_steps(run: &crate::flow::ShapedRun) -> (f64, usize) {
    let ornament = !run.glyphs.iter().any(|glyph| glyph.letter);
    let line = f64::from(run.line_height);
    let (mut steps, mut clusters) = (0.0, 0);
    let mut start = 0;
    while start < run.glyphs.len() {
        let end = run.glyphs[start + 1..]
            .iter()
            .position(|glyph| glyph.grapheme_start)
            .map_or(run.glyphs.len(), |at| start + 1 + at);
        let cluster = &run.glyphs[start..end];
        let advance: f64 = cluster.iter().map(|glyph| f64::from(glyph.advance)).sum();
        let left = cluster.iter().map(|glyph| glyph.x).fold(f32::INFINITY, f32::min);
        let ink = cluster
            .iter()
            .filter_map(|glyph| {
                let pen = Vec2::new(glyph.x - left, 0.0);
                glyph.ink.map(|ink| (pen + ink.min, pen + ink.max))
            })
            .reduce(|(min, max), (lo, hi)| (min.min(lo), max.max(hi)));
        let step = match ink.filter(|_| ornament && !cluster[0].blank) {
            Some((min, max)) => {
                let size = max - min;
                (f64::from(size.x.min(size.y)) + advance - f64::from(size.x)).max(0.0)
            }
            None if ornament => advance.max(0.0),
            None => advance.min(line),
        };
        steps += step;
        clusters += 1;
        start = end;
    }
    (steps, clusters)
}

/// Whether every `Repeat` ring of `frame` may lay no more glyphs than
/// [`MOST_RING_GLYPHS`] ([`repeat_glyphs`]; ruling B47.3), and one lay of
/// it, a directional one's `.notdef` passes counted (#387), take no more
/// than its share of the input's, `flow_fuzz::INPUT_STEPS` over
/// [`RING_LAYS_A_CHECK`] ([`ring_steps`]) — which a ring of no `.notdef`
/// count takes whenever it lays no more than `MOST_RING_GLYPHS`. O(its
/// rings' texts × the faces).
fn repeat_rings_fit(tree: &Tree<DocElement>, frame: NodeId) -> bool {
    let arena = tree.arena();
    frame
        .children(arena)
        .filter(|&ring| matches!(arena[ring].get(), DocElement::Ring(_)))
        .all(|ring| {
            repeat_glyphs(tree, frame, ring).is_none_or(|glyphs| {
                glyphs <= MOST_RING_GLYPHS
                    && ring_steps(tree, frame, ring)
                        .is_some_and(|steps| steps <= INPUT_STEPS / RING_LAYS_A_CHECK)
            })
        })
}

/// The most glyph steps a frame laid by halves — a ring of text of any fit
/// but `Repeat` — takes for each character of its text, past
/// [`FRAME_SETUP_STEPS`] and its side ornament's (ruling B47.3, on the
/// review of #378). Stated from measurement, as [`RING_STEPS_A_GLYPH`] is:
/// the engine bounds a frame's work by its budget, 400,000 steps at the
/// least (R44.2), which the flow harnesses give one lay of an input's share
/// and a check here lays up to [`RING_LAYS_A_CHECK`] times. A sweep of
/// 1,080 frames — 1 to 151 characters of Latin, of seven scripts and of
/// code points no face holds; `spread`, `fit` and `once`; no ornament, `✦`,
/// a 60-character one and a 256-character one; round ellipses 1 to 8,000
/// across from a twentieth of their size to the least — took at most 192 a
/// character with 320 a side glyph and 8,192 (`latin spread`, a `✦`, 100
/// across: 1,730 short of it).
/// `test_every_text_frame_the_bound_admits_fits_the_share` holds the
/// densest admitted frames to it.
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
/// frame admitted by that one lay's share took 18.4 s of a check under ASan
/// in three lays — 80 letters `fit` with a 60-character ornament round an
/// ellipse 1,000 across, 185,336 steps a lay (184,557 since #382) — where
/// ten would have taken a minute. Under ASan (`cargo fuzz run load_raw
/// <file> -- -runs=1`, the load average some 2.5), the densest frame this
/// admits by its steps, 151 letters `spread` round an ellipse 1,000 across,
/// 22,872 steps a lay, ran its input in 0.50 s. The slowest it admitted,
/// 151 characters of seven scripts `once` with a `✦` round one 100 across,
/// 21,702 steps a lay, ran in 3.1 s in three lays, some 10 s in ten, under
/// the 20 s timeout: most of its time is shaping its text in the faces its
/// scripts fall back to, which a full projection does again and no step
/// counts. It is excluded since #387: no one face draws its seven scripts,
/// so its side shapings may pay `.notdef` passes the bound cannot count
/// (`frame_may_shape_notdef`). That shaping is what a check's projections
/// cost for every flow, a section's and a path's too, and this bound does
/// not hold it.
pub const FRAME_STEPS_A_LAY: usize = INPUT_STEPS / RING_LAYS_A_CHECK;

/// The most glyphs the side ornament `sides` of a frame laid by halves may
/// lay (ruling B47.3), bounded before it is laid as [`repeat_glyphs`]
/// bounds a `Repeat` ring's: the outline's `perimeter`, over the least a
/// copy of it advances at `size`, the least size its ring can be laid at,
/// each of its grapheme clusters at its narrowest in `font`
/// ([`FaceMetrics::narrowest_cluster`]), one the shaper hides at nothing
/// ([`DEFAULT_IGNORABLE`]); and a copy more at each of the two breaks it
/// fills. The cap, `MAX_PLACED_GLYPHS`, where a copy can advance by
/// nothing. What it takes on trust is the module docs'. O(its text).
fn side_glyphs(sides: &str, font: &FontRef, size: f64, perimeter: f64) -> usize {
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
    let (text, sides) = frame_glyphs(tree, frame, ring)?;
    Some(
        FRAME_STEPS_A_GLYPH
            .saturating_mul(text)
            .saturating_add(FRAME_STEPS_A_SIDE_GLYPH.saturating_mul(sides))
            .saturating_add(FRAME_SETUP_STEPS),
    )
}

/// Whether a lay of `ring`, a ring laid by halves, may shape again a line
/// that may lay a `.notdef` (#387), and so pay the shaper's `.notdef`
/// passes, past any rate a sweep of drawn text measured: such a frame is
/// excluded from the bound ([`text_frames_fit`]), not counted. A frame's
/// reshapes take their run's shaping and lay it out at another size, which
/// shapes nothing (`flow::shape::reshape`), but where its text holds a tab
/// — a ring's run is one line, so nothing else stops it — or the run
/// carries a filler: so a lay shapes again where it has a side ornament
/// (each build shapes the `x` and the ornament its copy is sized by, and
/// the text with the ornament appended, `halves::side_run`; a `once`
/// filler is the frame's sides where they are unset), where it has a
/// `once` filler — sides or none, `sides: false` included: R27 appends it
/// to the text where the frame's halves cannot be measured
/// (`lines::place_lines_in`), whether or not that branch is reachable
/// from a map — or where it is `Fit` and its text holds a tab (R27's
/// reshapes and the shrink's). Its lines: the text, the ornament, the `x`
/// and the filler, each read apart ([`may_lay_notdef`]). On an outline
/// whose halves can be measured a filler is shaped only as the sides it
/// is where they are unset, so a frame excluded for a filler's line alone
/// pays no pass there: an exclusion for the other branch.
///
/// Why not counted. One lay may make dozens of such shapings — R27's 1 +
/// `FIT_ROUNDS`, twice on the branch whose halves cannot be measured, the
/// shrink's 1 + `SHRINK_ROUNDS`, three a build beside an ornament — each
/// up to `SHAPE_NOTDEF_STEPS` a glyph of its line, and no cap the engine
/// sets holds them below the share a lay has here ([`FRAME_STEPS_A_LAY`]),
/// its budget being 400,000 steps at the least. A count that held every
/// such lay would admit no frame it counted anything for. O(its texts ×
/// the faces), and once a process [`face_metrics`].
pub(crate) fn frame_may_shape_notdef(tree: &Tree<DocElement>, ring: NodeId) -> bool {
    let arena = tree.arena();
    let resolved = resolve_ring_in(tree, ring, Binding::Carried(None), &mut ());
    let filler = match &resolved.fit {
        FitMode::Repeat => return false,
        FitMode::Once { filler } => filler.as_deref(),
        _ => None,
    };
    let text: String = ring
        .children(arena)
        .filter_map(|span| match arena[span].get() {
            DocElement::Span(span) => Some(span.text.as_str()),
            _ => None,
        })
        .collect();
    let sides = resolved.sides.as_ref().map(|sides| sides.content.as_str());
    let shapes_again = sides.is_some()
        || filler.is_some()
        || (matches!(resolved.fit, FitMode::Fit { .. }) && text.contains('\t'));
    let metrics = face_metrics();
    shapes_again
        && [Some(text.as_str()), sides, sides.map(|_| "x"), filler]
            .into_iter()
            .flatten()
            .any(|line| may_lay_notdef(metrics, line.chars()))
}

/// Each text of `ring` its run is shaped from, a span's, in its own
/// style at the least size its ring can be ([`least_style`]); spans of no
/// text left out. O(its spans).
fn least_spans(tree: &Tree<DocElement>, ring: NodeId, least_rect: Rect) -> Vec<(ResolvedText, &str)> {
    let arena = tree.arena();
    ring.children(arena)
        .filter_map(|span| match arena[span].get() {
            DocElement::Span(text) => Some((
                least_style(
                    resolve_text_in(tree, span, Binding::Carried(None), &mut ()),
                    least_rect,
                ),
                text.text.as_str(),
            )),
            _ => None,
        })
        .filter(|(_, text)| !text.is_empty())
        .collect()
}

/// `style` at the size it has in a node drawn as `least_rect`, the least
/// rectangle a node is drawn as and so the least size a ring's text is
/// shaped at (D5). O(1).
fn least_style(style: ResolvedText, least_rect: Rect) -> ResolvedText {
    ResolvedText {
        size: FontSize::Pt(geometry::points(style.size, least_rect)),
        ..style
    }
}

/// The glyph steps one lay of `ring`, a `Repeat` ring of `frame` that
/// lays text by direction, charges its one reshape, where a line it
/// shapes may lay a `.notdef` (#387): its spans' text, each span in its
/// own style, with its `vertical` text and its corners appended on a line
/// of their own, weighed as the lay weighs it (`flow::reshape_steps`) —
/// exactly, `.notdef` passes and all, on any host and any face, a
/// decomposing one included. It is shaped at the least size its ring can
/// be (`min_size`, D5), where the projection shapes it at the size the
/// node is drawn at, and the charge does not depend on the size: the
/// buffers are laid out with no wrap and no width, so its layout lines are
/// its buffer lines at any size; harfrust shapes in font units; the
/// weights read only which glyphs were shaped, the faces they are in and
/// the lines they are on (`halves::shaping_steps`, `halves::notdef_steps`);
/// and neither a tab's measure nor the least font size changes a glyph.
/// Nothing where neither
/// line may lay a `.notdef` ([`may_lay_notdef`], the cheap gate: the
/// shaping of a line one face draws is what `RING_STEPS_A_GLYPH` was
/// measured with), and for any other ring — a `Repeat` ring of spans
/// alone shapes nothing as it lays. A lay that finds its outline too
/// small for two runs reshapes nothing, and this is then more than it
/// charges. O(its texts × the faces), and a shaping of them.
pub(crate) fn directional_notdef_steps(tree: &Tree<DocElement>, frame: NodeId, ring: NodeId) -> usize {
    use crate::flow::{shape_run, FlowSpan};
    let arena = tree.arena();
    let resolved = resolve_ring_in(tree, ring, Binding::Carried(None), &mut ());
    if !matches!(resolved.fit, FitMode::Repeat) || !resolved.directional.is_set() {
        return 0;
    }
    let Some(DocElement::Node(data)) = frame.parent(arena).map(|node| arena[node].get()) else {
        return 0;
    };
    let least_rect = Rect::new(Vec2::ZERO, geometry::usable_size(data.min_size));
    let spans = least_spans(tree, ring, least_rect);
    let text: String = spans.iter().map(|(_, text)| *text).collect();
    let appended: Vec<&str> = resolved
        .directional
        .vertical
        .iter()
        .chain(resolved.directional.corners.iter().flatten())
        .map(String::as_str)
        .collect();
    let appended = appended.join(" ");
    let metrics = face_metrics();
    if !may_lay_notdef(metrics, text.chars()) && !may_lay_notdef(metrics, appended.chars()) {
        return 0;
    }
    let style = least_style(
        resolve_text_in(tree, ring, Binding::Carried(None), &mut ()),
        least_rect,
    );
    crate::font::fonts::init();
    let mut lease = FontSystemLease::new("the projection's work bound");
    let flow_spans: Vec<FlowSpan<'_>> = spans
        .iter()
        .map(|(style, text)| FlowSpan {
            text,
            style,
            link: None,
        })
        .collect();
    let run = shape_run(&flow_spans, 1.0, &mut lease);
    crate::flow::reshape_steps(&run, &resolved.directional, &style, lease.fonts())
}

/// The glyphs one lay of `ring`, a ring of `frame` laid by halves, may
/// lay (ruling B47.3), as [`frame_steps`] counts them: its text's
/// characters, and the glyphs its side ornament may lay ([`side_glyphs`]).
/// `None` where [`frame_steps`] is. O(its texts), and once a process
/// [`face_metrics`].
pub(crate) fn frame_glyphs(tree: &Tree<DocElement>, frame: NodeId, ring: NodeId) -> Option<(usize, usize)> {
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
            side_glyphs(&sides.content, &style.font, size, perimeter)
        }
        None => 0,
    };
    Some((text, sides))
}

/// Whether one lay of every ring of `frame` laid by halves may take no
/// more than [`FRAME_STEPS_A_LAY`] ([`frame_steps`]; ruling B47.3), and
/// none may shape again a line that may lay a `.notdef`
/// ([`frame_may_shape_notdef`], #387), which excludes it. O(its rings'
/// texts × the faces).
fn text_frames_fit(tree: &Tree<DocElement>, frame: NodeId) -> bool {
    let arena = tree.arena();
    frame
        .children(arena)
        .filter(|&ring| matches!(arena[ring].get(), DocElement::Ring(_)))
        .all(|ring| {
            !frame_may_shape_notdef(tree, ring)
                && frame_steps(tree, frame, ring).is_none_or(|steps| steps <= FRAME_STEPS_A_LAY)
        })
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
    /// For each face, every code point its `cmap` maps to a glyph other
    /// than `.notdef` (glyph 0): what [`may_lay_notdef`] reads (#387).
    /// Held once a process with the rest, every mapped code point of every
    /// face: some 57,000 for Unifont alone.
    pub(crate) drawn: Vec<FxHashSet<u32>>,
}

/// The code points the bound reads as advancing by nothing (the review of
/// #378; ruling B47.4): the whole of Unicode's `Default_Ignorable_Code_Point`
/// (Unicode 16.0.0, `DerivedCoreProperties.txt`), its ranges merged. No
/// crate the workspace builds exposes the property.
///
/// Why nothing, on every host. The advance read for a character is a
/// lower bound: `side_glyphs` divides an outline by a copy's least advance,
/// so reading less counts more copies and more glyphs, never fewer.
/// Nothing is the least an advance can be, so reading it is sound
/// whatever the shaper lays the character as. What is unsound is reading
/// more than the shaper lays: a face can map one of these code points to a
/// glyph an em wide, which the shaper then hides at no advance (U+180E in
/// Unifont, on a host that has it). So the table must hold every code point
/// a shaper may hide, and the property is that set: HarfBuzz and harfrust
/// hide a subset of it. Where a shaper lays one of them after all — a
/// Hangul filler, a shorthand format control, or one no face maps, laid as
/// a `.notdef` — reading nothing counts more glyphs than it lays.
pub(crate) const DEFAULT_IGNORABLE: [(u32, u32); 17] = [
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x061C, 0x061C),
    (0x115F, 0x1160),
    (0x17B4, 0x17B5),
    (0x180B, 0x180F),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x206F),
    (0x3164, 0x3164),
    (0xFE00, 0xFE0F),
    (0xFEFF, 0xFEFF),
    (0xFFA0, 0xFFA0),
    (0xFFF0, 0xFFF8),
    (0x1BCA0, 0x1BCA3),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE0FFF),
];

/// Whether `c` is a [`DEFAULT_IGNORABLE`] code point. O(1).
pub(crate) fn default_ignorable(c: char) -> bool {
    let code = u32::from(c);
    DEFAULT_IGNORABLE
        .iter()
        .any(|&(first, last)| (first..=last).contains(&code))
}

/// The code points the shaper hides, so that none is ever laid as a
/// `.notdef` (#387): harfrust 0.5.2's default ignorables (its
/// `hb/unicode.rs`), which its hiding pass (`hb/ot_shape.rs`) lays as the
/// face's space glyph, or deletes where the face has none, unless a
/// substitution has made it a glyph of the face's own. cosmic-text sets
/// neither `PRESERVE_DEFAULT_IGNORABLES` nor a not-found variation
/// selector on the buffer, either of which would keep one. It is
/// [`DEFAULT_IGNORABLE`] less what harfrust lays as any other character:
/// the Hangul fillers (U+115F, U+1160, U+3164, U+FFA0), the shorthand
/// format controls (U+1BCA0–U+1BCA3) and U+180F.
/// `test_the_shaper_hides_every_code_point_the_bound_reads_as_hidden`
/// holds the shaper to it.
pub(crate) const SHAPER_HIDDEN: [(u32, u32); 13] = [
    (0x00AD, 0x00AD),
    (0x034F, 0x034F),
    (0x061C, 0x061C),
    (0x17B4, 0x17B5),
    (0x180B, 0x180E),
    (0x200B, 0x200F),
    (0x202A, 0x202E),
    (0x2060, 0x206F),
    (0xFE00, 0xFE0F),
    (0xFEFF, 0xFEFF),
    (0xFFF0, 0xFFF8),
    (0x1D173, 0x1D17A),
    (0xE0000, 0xE0FFF),
];

/// Whether the shaper hides `c` ([`SHAPER_HIDDEN`]). O(1).
pub(crate) fn shaper_hidden(c: char) -> bool {
    let code = u32::from(c);
    SHAPER_HIDDEN
        .iter()
        .any(|&(first, last)| (first..=last).contains(&code))
}

/// Whether shaping `text`, one line, may lay a `.notdef` (glyph 0) in a
/// font system of `metrics`' faces, and so cost the shaper's `.notdef`
/// passes (`flow::place::halves::notdef_steps`, #387): unless one face
/// maps every character of it the shaper is handed, other than those it
/// hides ([`SHAPER_HIDDEN`]), a tab and a line break each handed over as a
/// space (`flow::is_paragraph_separator`; cosmic-text lays a tab as a
/// space).
///
/// Why one face, and not each character mapped by some face. The shaper
/// gives up a glyph only where no face of the font system draws the whole
/// cluster it is part of: each fallback face shapes the run, and its
/// glyphs replace a missing cluster's only where it lays none of that
/// cluster's as glyph 0 (cosmic-text's fallback,
/// `vendor/cosmic-text/src/shape.rs`). A cluster can be several characters
/// — a base and its marks, a variation selector, a
/// joined sequence, a syllable of a complex script — each of which some
/// face maps and no one face maps together, and then it is a `.notdef` of
/// several code points, which costs a pass a glyph of its line. Which
/// characters harfrust joins into one cluster is its own, script by
/// script, and this reads none of it: a face that maps all of the text
/// draws every cluster of it whole, however it is formed. Nor does it read
/// where cosmic-text parts a line into the runs it shapes apart, which no
/// cluster crosses: its words, from line-break opportunities, whitespace
/// and ligature probes, a run of which can hold a space (`a )`). A line is
/// the least it can be sure no cluster crosses, and a run appended on a
/// line of its own — an ornament, a filler — is shaped apart from the
/// text, so each line is read on its own.
///
/// What it takes on trust: that a face whose `cmap` maps each character of
/// a cluster to a glyph other than 0 lays none of it as glyph 0 — no
/// substitution of its own yields `.notdef`; that the fallback tries every
/// face before it gives a glyph up, as cosmic-text's does; and that the
/// face that draws a missing cluster forms it from the same characters, so
/// that its cluster starts where the missing glyph does, which the
/// fallback's merge requires before it takes one (cosmic-text's merge in
/// `vendor/cosmic-text/src/shape.rs`): harfrust joins a mark, a variation
/// selector, a joiner, a keycap or a modifier to its base whatever the
/// face, but a face that ligated a character the first face drew with one
/// it lacked would start its cluster at the drawn one, and leave the
/// `.notdef`. It reads
/// the faces the font system holds, the host's with the compiled ones, as
/// the shaper does: what it answers for a text is the host's, and so is
/// what the shaper lays. O(its characters × the faces).
pub(crate) fn may_lay_notdef(metrics: &FaceMetrics, text: impl IntoIterator<Item = char>) -> bool {
    let mut wanted: Vec<u32> = text
        .into_iter()
        .map(|c| {
            if c == '\t' || crate::flow::is_paragraph_separator(c) {
                ' '
            } else {
                c
            }
        })
        .filter(|&c| !shaper_hidden(c))
        .map(u32::from)
        .collect();
    wanted.sort_unstable();
    wanted.dedup();
    !wanted.is_empty()
        && !metrics
            .drawn
            .iter()
            .any(|face| wanted.iter().all(|code| face.contains(code)))
}

impl FaceMetrics {
    /// The narrowest advance `c` may take in a span of `font`, in ems:
    /// nothing for a [`default_ignorable`] character, which a shaper may hide
    /// (nothing is a lower bound on any advance); else the narrowest its
    /// family's faces map it to, where one maps it — the shaper lays a
    /// character in the family it is pinned to, or the default face, whenever
    /// a face of it holds the character — or the narrowest any face maps it
    /// to, where none does, or `.notdef`'s. An installed family (#329) is
    /// read as any face: it is drawn in its faces on this machine, or in
    /// the default face where it is missing, and the narrowest any face
    /// maps the character to bounds both. O(1).
    pub(crate) fn narrowest_char(&self, font: &FontRef, c: char) -> f32 {
        if default_ignorable(c) {
            return 0.0;
        }
        let family = match font {
            FontRef::Compiled(AppFont::Any) => self.family.get(&DEFAULT_FACE.0),
            FontRef::Compiled(font) => self.family.get(font),
            FontRef::Installed(_) => None,
        };
        let code = u32::from(c);
        family
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
    /// cluster — and nothing where one of them is [`default_ignorable`].
    /// O(its characters).
    pub(crate) fn narrowest_cluster(&self, font: &FontRef, cluster: &str) -> f32 {
        let mut chars = cluster.chars();
        match (chars.next(), chars.next()) {
            (Some(c), None) => self.narrowest_char(font, c),
            _ => cluster
                .chars()
                .map(|c| {
                    if default_ignorable(c) {
                        0.0
                    } else {
                        self.narrowest.get(&u32::from(c)).copied().unwrap_or(self.notdef)
                    }
                })
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
            drawn: Vec::new(),
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
                metrics.drawn.push(face_read.drawn);
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
    /// Each code point it maps to a glyph other than `.notdef`.
    drawn: FxHashSet<u32>,
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
    // One walk of the charmap: each code point's narrowest advance, and
    // whether it maps to a glyph other than `.notdef`.
    let mut codes = Vec::new();
    let mut drawn = FxHashSet::default();
    for (code, glyph) in font.charmap().mappings() {
        if glyph.to_u32() != 0 {
            drawn.insert(code);
        }
        if let Some(&(least, _)) = range.get(glyph.to_u32() as usize) {
            codes.push((code, least / units));
        }
    }
    Some(FaceAdvances {
        widest: range.iter().map(|&(_, most)| most).fold(0.0f32, f32::max) / units,
        notdef: range.first().map_or(f32::INFINITY, |&(least, _)| least / units),
        codes,
        drawn,
    })
}

/// The projection properties of a map the loader accepted (see the module
/// docs): `Ok` when they hold, when the map is outside the bound, when its
/// untangling reaches [`MAX_UNTANGLE_SAMPLES`], or when the check's own
/// edit takes it out of the bound — each skipped, and counted apart.
pub fn check_projects(doc: &Document) -> Result<(), String> {
    if !within_bound(doc) {
        SKIPPED.with(|n| n.set(n.get() + 1));
        return Ok(());
    }
    let (checked, gone) = held(|| {
        #[cfg(any(test, fuzzing))]
        {
            under_ceiling(|| check_within_ceiling(doc))
        }
        #[cfg(not(any(test, fuzzing)))]
        {
            (check_within_ceiling(doc), false)
        }
    });
    let (checked, cut) = checked;
    // Cut short, the map was not laid out as it lays out: whatever the
    // check found, it is not this map's, and is not reported.
    if cut {
        CUT.with(|n| n.set(n.get() + 1));
        return Ok(());
    }
    // Taken out of the bound by the check's own edit: what it laid after
    // is of a map the bound never read.
    if gone {
        LEFT.with(|n| n.set(n.get() + 1));
        return Ok(());
    }
    PROJECTED.with(|n| n.set(n.get() + 1));
    checked
}

/// `check` with its map held to the bound ([`left_the_bound`]), and
/// whether the check's edit took the map out of it. The hold and the mark
/// are clear again however `check` ends, by return or by panic.
/// O(`check`).
fn held<R>(check: impl FnOnce() -> R) -> (R, bool) {
    /// Clears the hold and the mark when it goes.
    struct Clear;
    impl Drop for Clear {
        fn drop(&mut self) {
            HELD.with(|held| held.set(false));
            GONE.with(|gone| gone.set(false));
        }
    }
    let _clear = Clear;
    HELD.with(|held| held.set(true));
    GONE.with(|gone| gone.set(false));
    #[cfg(test)]
    LAID_AFTER.with(|laid| laid.set(None));
    let checked = check();
    (checked, GONE.with(Cell::get))
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

/// The work bound's end-to-end property (ruling B47.3), read from the
/// projection `p` of a map inside the bound, whose rings' placements took
/// at most `most` glyph steps each (`flow::MOST_RING_LAY`, cleared before
/// it): every `Repeat` ring lays no more glyphs than [`repeat_glyphs`]
/// read for it, and no ring's lay took more steps than the most the bound
/// allows any ring of the map — [`RING_STEPS_A_GLYPH`] a glyph of a
/// `Repeat` ring's bound and [`RING_SETUP_STEPS`], [`frame_steps`] for a
/// ring laid by halves, and the larger setup for a ring of no text. The
/// map holds one ring of text at most, so the most of its lays is that
/// ring's, or a ring's of no text. The steps and the rates are stated
/// from measurement, which an input no sweep tried can break: this is
/// where a fuzz run finds it: a loader check reads it after its first
/// projection, and after every update and full projection its edit makes
/// of a map still inside the bound (`laid_within`). `Ok(most)`, the steps
/// it held. A placement the test build shares from an earlier projection
/// (`stages::memo`) is not laid again, and counts nothing.
///
/// And every connector's and line link's body laid along its open path,
/// as the projection laid them since `flow::MOST_OPEN_LAY` was last
/// cleared, which it is wherever `MOST_RING_LAY` is: no body's lay took
/// more steps than the most any body of the map may take
/// ([`line_body_steps`], #387) — its budget and what may run past it, its
/// one reshape included. Before #387 no property of the projection read a
/// body's lay at all. What is checked is the most any body's lay took
/// against the most any body of the map may take, as for rings: a body
/// that runs past its own bound passes beside a body of the same map
/// whose bound is larger. Holding each lay to its own bound, rings' and
/// bodies' alike, is #397's; the map's bodies together held to the
/// timeout is #316's.
///
/// O(its rings and their texts, and its lines' bodies, each predicted).
#[cfg(any(test, fuzzing))]
pub(crate) fn rings_within_their_work(p: &Projected, most: usize) -> Result<usize, String> {
    let open = crate::flow::MOST_OPEN_LAY.with(Cell::get);
    if open > 0 {
        let allowed = line_body_steps(p).unwrap_or(0);
        if open > allowed {
            return Err(format!(
                "a body's lay along an open path took {open} glyph steps, past the {allowed} the bound allows its map's bodies (#387)"
            ));
        }
    }
    use crate::projection::state::UnitBody;
    let tree = p.doc.tree();
    let mut allowed = RING_SETUP_STEPS.max(FRAME_SETUP_STEPS);
    for (&id, unit) in &p.projector.units {
        let UnitBody::Ring(ring) = &unit.body else {
            continue;
        };
        let Some(frame) = ring.frame else {
            continue;
        };
        if let Some(bound) = repeat_glyphs(tree, frame, id) {
            let placed = ring.flow.placed.as_ref().map_or(0, |p| p.placed.glyphs.len());
            if placed > bound {
                return Err(format!("ring {id:?} laid {placed} glyphs, its bound {bound}"));
            }
        }
        if let Some(steps) = ring_steps(tree, frame, id) {
            allowed = allowed.max(steps);
        }
    }
    if most > allowed {
        return Err(format!(
            "a ring's lay took {most} glyph steps, past the {allowed} the bound allows its map"
        ));
    }
    #[cfg(test)]
    LAID_AFTER.with(|laid| laid.set(laid.get().map(|_| most)));
    Ok(most)
}

/// The most glyph steps any connector's or line link's body in `p` laid
/// along its open path may take (#387): `flow::open_work_steps` of each,
/// read as the projector holds it — its input as shaped, its resolved fit,
/// spacing and style, and its curves as its stage built and placed them,
/// whose tables it builds again as the placement does — the largest of
/// them. A body laid as whole glyphs (`stages::laid_pitched`, the stage's
/// own test) is not laid along its path, and counts nothing; nor does one
/// not yet placed, which has no curves. One that is not drawn is counted
/// all the same: its place stage lays it (`stages::line_place` reads no
/// `drawn`). `None` where none is laid along its path.
/// O(its lines' bodies), each a prediction of its reshape: a shaping for
/// a `Once` with a filler.
#[cfg(any(test, fuzzing))]
pub(crate) fn line_body_steps(p: &Projected) -> Option<usize> {
    use crate::projection::state::UnitBody;
    crate::font::fonts::init();
    let mut lease = FontSystemLease::new("the projection's work bound");
    let mut most: Option<usize> = None;
    for unit in p.projector.units.values() {
        let UnitBody::Line(line) = &unit.body else {
            continue;
        };
        let (Some((resolved, style)), Some(body)) = (&line.resolved, line.flows[0].flow.as_ref()) else {
            continue;
        };
        if line.path.is_empty() || crate::projection::stages::laid_pitched(&body.shaped, resolved) {
            continue;
        }
        let steps = crate::flow::open_work_steps(
            &body.shaped,
            &line.path,
            &resolved.fit,
            resolved.spacing,
            style,
            lease.fonts(),
        );
        most = Some(most.map_or(steps, |most| most.max(steps)));
    }
    most
}

/// The most glyph steps one lay of `ring`, a ring of `frame`, may take
/// inside the bound (ruling B47.3): a `Repeat` ring's
/// [`RING_STEPS_A_GLYPH`] a glyph of its bound ([`repeat_glyphs`]) and
/// [`RING_SETUP_STEPS`], and a directional one's `.notdef` passes
/// ([`directional_notdef_steps`], #387), or a ring laid by halves'
/// [`frame_steps`]. `None` for a ring of no text, which the setup bounds.
/// O(its texts × the faces).
pub(crate) fn ring_steps(tree: &Tree<DocElement>, frame: NodeId, ring: NodeId) -> Option<usize> {
    match repeat_glyphs(tree, frame, ring) {
        Some(bound) => Some(
            RING_STEPS_A_GLYPH
                .saturating_mul(bound)
                .saturating_add(RING_SETUP_STEPS)
                .saturating_add(directional_notdef_steps(tree, frame, ring)),
        ),
        None => frame_steps(tree, frame, ring),
    }
}

/// [`check_projects`] once the map is inside the bound, under the ceiling
/// it sets: abandoned, with `Ok`, as soon as its first projection is cut
/// short.
fn check_within_ceiling(doc: &Document) -> Result<(), String> {
    // In the test build the loader suites' maps, up to a hundred thousand
    // nodes, are projected sharing the shapings of equal keys as the
    // oracle's full projections do (`stages::memo`): every property here
    // compares by value. A fuzz build shapes every flow.
    #[cfg(any(test, fuzzing))]
    crate::flow::MOST_RING_LAY.with(|most| most.set(0));
    #[cfg(any(test, fuzzing))]
    crate::flow::MOST_OPEN_LAY.with(|most| most.set(0));
    #[cfg(test)]
    let mut p = Projected::new_shared(doc.clone());
    #[cfg(not(test))]
    let mut p = Projected::new(doc.clone());
    #[cfg(any(test, fuzzing))]
    if crate::projection::geometry::UNTANGLE_CUT.with(Cell::get) {
        return Ok(());
    }
    // What the bound read of each ring holds of how it was laid (ruling
    // B47.3): a fuzz run finds the next ring past it.
    #[cfg(any(test, fuzzing))]
    rings_within_their_work(&p, crate::flow::MOST_RING_LAY.with(Cell::get))?;
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

/// Bring `p` through the change just made to its document: the update,
/// then the oracle (`Projected::check`), then, in a loader check, the
/// work of the rings the two laid ([`laid_within`]) — the after-edit read
/// of the work bound's property. `Ok(false)`, with nothing laid, where
/// the change took the map out of the bound ([`left_the_bound`]) and the
/// sweep stops. O(an update and a full projection).
fn updated(p: &mut Projected) -> Result<bool, String> {
    if left_the_bound(p) {
        return Ok(false);
    }
    p.sync_and_discard();
    p.check()?;
    laid_within(p)?;
    Ok(true)
}

/// Take `p` through `steps` edits — the edit [`EDITS`] names at `turn`,
/// advancing it, its targets and values drawn from `c`, applied and, one
/// in three, undone and redone — transaction stages, and batches: two to
/// five edits and undos applied before one update is fed every record
/// they made, as a frame feeds them (the review of #317). The oracle is
/// checked after every update; each edit applied is counted in
/// `covered`. In a loader check, the map is read against the bound again
/// after each change and before its update, and the sweep stops with `Ok`
/// where the change took it out (`left_the_bound`); where it did not,
/// the rings laid after it are held to the work the bound read for them
/// (`laid_within`). `Err` names the case, the step and the edit.
pub fn sweep(
    case: &str,
    p: &mut Projected,
    c: &mut Choices<'_>,
    steps: usize,
    turn: &mut usize,
    covered: &mut FxHashMap<&'static str, usize>,
) -> Result<(), String> {
    p.check()
        .and_then(|()| laid_within(p).map(|_| ()))
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
            if !updated(p).map_err(|e| at(&format!("the batch {names:?}"), e))? {
                return Ok(());
            }
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
                    if !updated(p).map_err(|e| at(&format!("stage {stage} ({name})"), e))? {
                        return Ok(());
                    }
                }
            }
            if kind == TransactionKind::Animated && p.doc.stage_progress(&tx, 0.5, Snap::End).is_ok() {
                *covered.entry("transaction::interpolate").or_default() += 1;
                if !updated(p).map_err(|e| at("interpolate", e))? {
                    return Ok(());
                }
            }
            if c.one_in(2) {
                p.doc.commit(tx);
            } else {
                p.doc.cancel(tx);
            }
            if !updated(p).map_err(|e| at("commit or cancel", e))? {
                return Ok(());
            }
            continue;
        }
        let name = EDITS[*turn % EDITS.len()];
        *turn += 1;
        let Some((name, req)) = edit_named(c, &p.doc, name) else {
            continue;
        };
        if p.doc.apply(req, ChangeSource::Action).is_err() {
            continue;
        }
        *covered.entry(name).or_default() += 1;
        if !updated(p).map_err(|e| at(name, e))? {
            return Ok(());
        }
        if c.one_in(3) {
            p.doc.undo();
            if !updated(p).map_err(|e| at(&format!("undo of {name}"), e))? {
                return Ok(());
            }
            p.doc.redo();
            if !updated(p).map_err(|e| at(&format!("redo of {name}"), e))? {
                return Ok(());
            }
            *covered.entry("history::undo_and_redo").or_default() += 1;
        }
    }
    Ok(())
}
