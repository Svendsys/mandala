// SPDX-License-Identifier: MPL-2.0

//! A frame's work, counted by kind of outline (#309): path lookups,
//! glyph steps, tracks, reshapes, inkings and pairs weighed, over the
//! aspect sweep's frames.

use std::fmt;

use crate::document::values::{FitMode, LineSpan, Rgba};
use crate::flow::place::RingLayout;
use crate::flow::tests::halves_tests::{frame_with, sweep_outlines, QUATRAIN};
use crate::flow::Outline;

/// The Ecclesiastes center frame's text.
const ECCLESIASTES: &str = "and a time to every purpose under the heaven: ";

/// What an outline is made of, as its path walks it: straight sides
/// only (a sharp rectangle), circular arcs only (a circle), an ellipse's
/// table, or sides and arcs (a rounded rectangle).
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Kind {
    Lines,
    Arcs,
    Ellipse,
    Mixed,
}

impl Kind {
    pub(crate) fn of(outline: &Outline) -> Kind {
        match *outline {
            Outline::RoundedRect { rect, .. } => {
                let r = outline.corner_radius();
                if r <= 0.0 {
                    Kind::Lines
                } else if rect.size.x == rect.size.y && 2.0 * r >= rect.size.x {
                    Kind::Arcs
                } else {
                    Kind::Mixed
                }
            }
            Outline::Ellipse { rect } => {
                let radii = rect.size * 0.5;
                if (radii.x - radii.y).abs() <= radii.x.max(radii.y) * 1e-6 {
                    Kind::Arcs
                } else {
                    Kind::Ellipse
                }
            }
        }
    }
}

/// The thread's work counters at one moment.
#[derive(Copy, Clone, Debug, Default)]
pub(crate) struct Counters {
    pub(crate) lookups: usize,
    pub(crate) searches: usize,
    pub(crate) steps: (usize, usize),
    pub(crate) side_steps: (usize, usize),
    pub(crate) tracks: usize,
    pub(crate) reshapes: usize,
    pub(crate) inkings: usize,
    pub(crate) weighed: usize,
    pub(crate) track_hits: usize,
    pub(crate) outline_reads: usize,
    pub(crate) table_samples: usize,
}

impl Counters {
    pub(crate) fn now() -> Counters {
        use crate::flow::path::{PATH_LOOKUPS, PATH_SEARCHES, TABLE_SAMPLES};
        use crate::flow::place::halves::{
            BOXES_WEIGHED, INKINGS, OUTLINE_READS, RESHAPES, SIDE_STEP_LOOKUPS, STEP_LOOKUPS, TRACKS,
            TRACK_HITS,
        };
        let get = |c: &'static std::thread::LocalKey<std::cell::Cell<usize>>| c.with(std::cell::Cell::get);
        Counters {
            lookups: get(&PATH_LOOKUPS),
            searches: get(&PATH_SEARCHES),
            steps: STEP_LOOKUPS.with(std::cell::Cell::get),
            side_steps: SIDE_STEP_LOOKUPS.with(std::cell::Cell::get),
            tracks: get(&TRACKS),
            reshapes: get(&RESHAPES),
            inkings: get(&INKINGS),
            weighed: get(&BOXES_WEIGHED),
            track_hits: get(&TRACK_HITS),
            outline_reads: get(&OUTLINE_READS),
            table_samples: get(&TABLE_SAMPLES),
        }
    }

    fn since(self, before: Counters) -> Counters {
        Counters {
            lookups: self.lookups - before.lookups,
            searches: self.searches - before.searches,
            steps: (self.steps.0 - before.steps.0, self.steps.1 - before.steps.1),
            side_steps: (
                self.side_steps.0 - before.side_steps.0,
                self.side_steps.1 - before.side_steps.1,
            ),
            tracks: self.tracks - before.tracks,
            reshapes: self.reshapes - before.reshapes,
            inkings: self.inkings - before.inkings,
            weighed: self.weighed - before.weighed,
            track_hits: self.track_hits - before.track_hits,
            outline_reads: self.outline_reads - before.outline_reads,
            table_samples: self.table_samples - before.table_samples,
        }
    }
}

/// A kind of frame's work, summed over its frames.
#[derive(Clone, Debug, Default)]
pub(crate) struct Tally {
    pub(crate) frames: usize,
    /// The glyphs placed: the text's, and the side ornaments'.
    pub(crate) text: usize,
    pub(crate) sides: usize,
    /// All of the frames' work counters.
    pub(crate) work: Counters,
    /// The glyph steps a frame's work took ([`Trace::spent`]).
    pub(crate) spent: usize,
}

impl Tally {
    fn add(&mut self, text: usize, sides: usize, work: Counters, spent: usize) {
        let w = &mut self.work;
        self.frames += 1;
        self.text += text;
        self.sides += sides;
        self.spent += spent;
        w.lookups += work.lookups;
        w.searches += work.searches;
        w.steps.0 += work.steps.0;
        w.steps.1 += work.steps.1;
        w.side_steps.0 += work.side_steps.0;
        w.side_steps.1 += work.side_steps.1;
        w.tracks += work.tracks;
        w.reshapes += work.reshapes;
        w.inkings += work.inkings;
        w.weighed += work.weighed;
        w.track_hits += work.track_hits;
        w.outline_reads += work.outline_reads;
        w.table_samples += work.table_samples;
    }

    /// The glyph steps the text's lines took, and their path lookups.
    pub(crate) fn line_steps(&self) -> (usize, usize) {
        (
            self.work.steps.0 - self.work.side_steps.0,
            self.work.steps.1 - self.work.side_steps.1,
        )
    }

    /// The path lookups the side ornaments' lays took a side glyph placed.
    pub(crate) fn side_lookups_a_glyph(&self) -> f64 {
        self.work.side_steps.1 as f64 / self.sides.max(1) as f64
    }
}

impl fmt::Display for Tally {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let w = &self.work;
        let placed = (self.text + self.sides).max(1) as f64;
        let (line_steps, line_lookups) = self.line_steps();
        write!(
            f,
            "{} frames, {} glyphs ({} sides); lookups {} ({:.1} a glyph placed), searches {}; \
             line steps {} ({:.2} lookups a step, {:.1} steps a text glyph); \
             side steps {} ({} lookups, {:.1} a side glyph); tracks {} ({:.1} a frame); \
             reshapes {}, inkings {} ({} outlines read), pairs weighed {}; tracks remembered {}; \
             table samples {}; spent {} ({:.1} a glyph)",
            self.frames,
            self.text + self.sides,
            self.sides,
            w.lookups,
            w.lookups as f64 / placed,
            w.searches,
            line_steps,
            line_lookups as f64 / line_steps.max(1) as f64,
            line_steps as f64 / self.text.max(1) as f64,
            w.side_steps.0,
            w.side_steps.1,
            self.side_lookups_a_glyph(),
            w.tracks,
            w.tracks as f64 / self.frames.max(1) as f64,
            w.reshapes,
            w.inkings,
            w.outline_reads,
            w.weighed,
            w.track_hits,
            w.table_samples,
            self.spent,
            self.spent as f64 / placed,
        )
    }
}

/// The aspect sweep's frames' work by kind of outline and whether the
/// frame has sides: the frames `do_flow_halves_hold_on_every_aspect`
/// lays, each laid once more here with its work counted.
pub(crate) fn census() -> &'static [((Kind, bool), Tally)] {
    static CENSUS: std::sync::OnceLock<Vec<((Kind, bool), Tally)>> = std::sync::OnceLock::new();
    CENSUS.get_or_init(|| {
        let mut out: Vec<((Kind, bool), Tally)> = Vec::new();
        let fit = FitMode::Fit { min: 0.5, max: 2.0 };
        for (_, shape, _, points) in sweep_outlines() {
            let kind = Kind::of(&shape);
            for text in [ECCLESIASTES, QUATRAIN] {
                for outward in [true, false] {
                    for fit in [fit.clone(), FitMode::Once { filler: None }] {
                        let sides_tried = if outward { 2 } else { 1 };
                        for sides in [None, Some(("·".to_owned(), Rgba::opaque(9, 9, 9)))]
                            .into_iter()
                            .take(sides_tried)
                        {
                            for span in [
                                LineSpan::WHOLE,
                                LineSpan::new(0.5).unwrap_or(LineSpan::WHOLE),
                                LineSpan::NATURAL,
                            ] {
                                let layout = RingLayout {
                                    span,
                                    sides: sides.clone(),
                                    ..RingLayout::whole()
                                };
                                let before = Counters::now();
                                let (_, placed, trace) =
                                    frame_with(text, points, &shape, &fit, &layout, outward);
                                let work = Counters::now().since(before);
                                let text_glyphs: usize = trace.lines.iter().map(|l| l.glyphs.len()).sum();
                                let key = (kind, sides.is_some());
                                let at = match out.iter().position(|(k, _)| *k == key) {
                                    Some(at) => at,
                                    None => {
                                        out.push((key, Tally::default()));
                                        out.len() - 1
                                    }
                                };
                                out[at].1.add(
                                    text_glyphs,
                                    placed.glyphs.len() - text_glyphs,
                                    work,
                                    trace.spent,
                                );
                            }
                        }
                    }
                }
            }
        }
        out.sort_by_key(|(key, _)| *key);
        out
    })
}

/// The census, printed: `MANDALA_FLOW_WORK_REPORT=1` to read it.
#[test]
fn test_flow_work_census_reports_each_kind_of_frame() {
    let census = census();
    assert!(
        census.len() >= 8,
        "precondition: every kind of outline, with sides and without: {census:?}"
    );
    if std::env::var_os("MANDALA_FLOW_WORK_REPORT").is_some() {
        for ((kind, sides), tally) in census {
            eprintln!("{kind:?} sides {sides}: {tally}");
        }
    }
}
