// SPDX-License-Identifier: MPL-2.0

//! An instance's progress `t` over time (#251 §2): which way it is
//! going, and where it is at a reading of the clock.
//!
//! | `play` | In | Holds | Out | Ends |
//! |---|---|---|---|---|
//! | `while` | `0→1` | until stopped | `current→0` | at `0` |
//! | `once` | `0→1` | no | straight back, `1→0` | at `0` |
//! | `loop` | `0→1` | no | `1→0`, then in again | stopped, at the end of the cycle it is in |
//! | `hold` | `0→1` | until stopped | `current→0` | at `0` |
//!
//! **A direction is a segment**: from the `t` the instance had when it
//! began, to `1` (in) or `0` (out), following that direction's
//! [`Animate`] whole — held at its start for `delay_ms`, then eased over
//! `ms`. Stopping on the way in, and restarting on the way out, begin a
//! segment from the current `t`, so nothing jumps.
//!
//! **Time is a number handed in**, the app clock's reading, so a
//! lifecycle is a pure function of the readings it is given: under a
//! virtual clock every instant is exact. A segment that ends between two
//! readings hands over at its own end, not at the reading that noticed,
//! so where an instance is at an instant does not depend on how often it
//! was asked. A loop that was not asked for a long time skips its whole
//! cycles by arithmetic: one reading costs O(1) however long the gap.

use crate::document::edit::Animate;
use crate::document::presentation::Play;
use crate::engine::interpolate::Snap;

/// Which way an instance is going.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Direction {
    /// Toward `1`.
    In,
    /// At `1`, until it is stopped.
    Hold,
    /// Toward `0`.
    Out,
}

impl Direction {
    /// The direction's name, as the console and IPC report it. O(1).
    pub fn name(self) -> &'static str {
        match self {
            Direction::In => "in",
            Direction::Hold => "hold",
            Direction::Out => "out",
        }
    }
}

/// How long a direction takes from its start to its end, its delay
/// included, in milliseconds. O(1).
pub(crate) fn span_ms(timing: &Animate) -> f64 {
    f64::from(timing.delay_ms) + f64::from(timing.ms)
}

/// One direction: from `from` to `to`, following `timing` from
/// `started_ms`.
#[derive(Copy, Clone, Debug, PartialEq)]
struct Segment {
    from: f32,
    to: f32,
    timing: Animate,
    started_ms: f64,
}

impl Segment {
    /// The clock reading it ends at.
    fn end_ms(&self) -> f64 {
        self.started_ms + span_ms(&self.timing)
    }

    /// Whether it has run its course at `now_ms`: at its end exactly, it
    /// has.
    fn is_done(&self, now_ms: f64) -> bool {
        now_ms >= self.end_ms()
    }

    /// Where it is at `now_ms`, before its end: `from` through the
    /// delay, then along the eased line to `to`.
    fn at(&self, now_ms: f64) -> f32 {
        let progress = self.timing.progress(now_ms - self.started_ms);
        if progress <= 0.0 {
            self.from
        } else {
            self.from + (self.to - self.from) * progress
        }
    }
}

/// Where an instance is in its life.
#[derive(Copy, Clone, Debug, PartialEq)]
enum Phase {
    In(Segment),
    Hold,
    Out(Segment),
    Ended,
}

/// One instance's lifecycle. See the [module docs](self).
#[derive(Copy, Clone, Debug, PartialEq)]
pub(crate) struct Lifecycle {
    play: Play,
    animate: Animate,
    out: Animate,
    phase: Phase,
    /// A loop that was stopped and is finishing its cycle.
    stopping: bool,
    /// Where it was at the last reading.
    t: f32,
}

impl Lifecycle {
    /// A lifecycle of `play`, in over `animate` and out over `out`,
    /// started at `now_ms`: on its way in, from `0`. O(1).
    pub(crate) fn start(play: Play, animate: Animate, out: Animate, now_ms: f64) -> Self {
        let mut life = Lifecycle {
            play,
            animate,
            out,
            phase: Phase::In(Segment {
                from: 0.0,
                to: 1.0,
                timing: animate,
                started_ms: now_ms,
            }),
            stopping: false,
            t: 0.0,
        };
        life.advance(now_ms);
        life
    }

    /// Bring it to `now_ms` and return where it is: `t` in `0..=1`.
    /// A segment that ended at or before `now_ms` hands over at its own
    /// end. O(1): at most one whole cycle is stepped through, the ones
    /// before it skipped by arithmetic.
    pub(crate) fn advance(&mut self, now_ms: f64) -> f32 {
        // In → Out → In → Out is the longest chain one reading walks: a
        // loop whose out ended lands, after the skip, inside one cycle.
        for _ in 0..4 {
            match self.phase {
                Phase::In(segment) if segment.is_done(now_ms) => {
                    self.t = segment.to;
                    self.phase = match self.play {
                        Play::While | Play::Hold => Phase::Hold,
                        Play::Once | Play::Loop => Phase::Out(Segment {
                            from: segment.to,
                            to: 0.0,
                            timing: self.out,
                            started_ms: segment.end_ms(),
                        }),
                    };
                }
                Phase::In(segment) => {
                    self.t = segment.at(now_ms);
                    return self.t;
                }
                Phase::Hold => {
                    self.t = 1.0;
                    return self.t;
                }
                Phase::Out(segment) if segment.is_done(now_ms) => {
                    self.t = 0.0;
                    let cycle = span_ms(&self.animate) + span_ms(&self.out);
                    self.phase = if self.play == Play::Loop && !self.stopping && cycle > 0.0 {
                        // The whole cycles between this end and `now_ms`
                        // each begin and end at `0`: skip them.
                        let end = segment.end_ms();
                        let skipped = ((now_ms - end) / cycle).floor().max(0.0);
                        Phase::In(Segment {
                            from: 0.0,
                            to: 1.0,
                            timing: self.animate,
                            started_ms: end + skipped * cycle,
                        })
                    } else {
                        Phase::Ended
                    };
                }
                Phase::Out(segment) => {
                    self.t = segment.at(now_ms);
                    return self.t;
                }
                Phase::Ended => {
                    self.t = 0.0;
                    return self.t;
                }
            }
        }
        self.t
    }

    /// Stop it at `now_ms`: a `while`, a `once` or a `hold` turns back
    /// from where it is, with no jump; a `loop` finishes the cycle it is
    /// in. Nothing for one already on its way out, or ended. O(1).
    pub(crate) fn stop(&mut self, now_ms: f64) {
        let t = self.advance(now_ms);
        match (self.play, self.phase) {
            (_, Phase::Ended) => {}
            (Play::Loop, _) => self.stopping = true,
            (_, Phase::In(_) | Phase::Hold) => {
                self.phase = Phase::Out(Segment {
                    from: t,
                    to: 0.0,
                    timing: self.out,
                    started_ms: now_ms,
                });
            }
            (_, Phase::Out(_)) => {}
        }
    }

    /// Start it again at `now_ms`, in place: its direction becomes In
    /// from where it is (#251 §2). One on its way in, or holding, goes
    /// on as it was; a loop that was finishing its cycle loops again.
    /// O(1).
    pub(crate) fn restart(&mut self, now_ms: f64) {
        let t = self.advance(now_ms);
        self.stopping = false;
        match self.phase {
            Phase::In(_) | Phase::Hold => {}
            Phase::Out(_) | Phase::Ended => {
                self.phase = Phase::In(Segment {
                    from: t,
                    to: 1.0,
                    timing: self.animate,
                    started_ms: now_ms,
                });
                self.advance(now_ms);
            }
        }
    }

    /// Where it was at the last reading. O(1).
    pub(crate) fn t(&self) -> f32 {
        self.t
    }

    /// Whether it has ended: out, at `0`, with nothing more to play.
    /// O(1).
    pub(crate) fn ended(&self) -> bool {
        self.phase == Phase::Ended
    }

    /// Whether its `t` moves with time: on its way in or out. One that
    /// holds needs no frame. O(1).
    pub(crate) fn moving(&self) -> bool {
        matches!(self.phase, Phase::In(_) | Phase::Out(_))
    }

    /// Whether it plays on with nobody stopping it: a loop that was not
    /// stopped. O(1).
    pub(crate) fn unending(&self) -> bool {
        self.play == Play::Loop && !self.stopping && !self.ended()
    }

    /// Which way it is going; an ended one was last going out. O(1).
    pub(crate) fn direction(&self) -> Direction {
        match self.phase {
            Phase::In(_) => Direction::In,
            Phase::Hold => Direction::Hold,
            Phase::Out(_) | Phase::Ended => Direction::Out,
        }
    }

    /// When the fields that do not blend switch, on the way it is going:
    /// its way in's `snap`, or its way out's. O(1).
    pub(crate) fn snap(&self) -> Snap {
        match self.phase {
            Phase::In(_) | Phase::Hold => self.animate.snap,
            Phase::Out(_) | Phase::Ended => self.out.snap,
        }
    }
}
