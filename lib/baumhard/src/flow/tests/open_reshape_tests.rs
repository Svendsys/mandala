// SPDX-License-Identifier: MPL-2.0

//! A placement along an open path counts its reshapes in its work (#387):
//! a `Fit`'s, one each round, and a `Once` filler's shaping with the text,
//! charged to its budget (`RingWork::reshaped`), recorded
//! (`Trace::reshaped`) and predicted (`place::path_reshape_steps`). A
//! round reshapes only where the round before it found the budget able
//! to afford its phase 2, so at most one reshape runs past the budget.

use glam::Vec2;

use crate::document::values::FitMode;
use crate::flow::place::halves::{BUDGET_OVERRIDE, PAST_BUDGET};
use crate::flow::place::ring::{ring_overshoot, RESHAPES_ALONG};
use crate::flow::place::{path_reshape_steps, place_on_open_capped, Trace, LAST_OPEN};
use crate::flow::tests::flow_fuzz::InputWork;
use crate::flow::tests::halves_tests::with_seam;
use crate::flow::tests::support::{style, with_fonts};
use crate::flow::{shape_run, Curve, FlowSpan, PlacedRun, MAX_PLACED_GLYPHS};

/// The size the rows are shaped at.
const POINTS: f32 = 16.0;

/// One placement along an open path as laid.
struct Laid {
    placed: PlacedRun,
    trace: Trace,
    /// The reshapes it counted (`RESHAPES_ALONG`), all it took past its
    /// budget (`PAST_BUDGET`), and the cluster that bounds what may find
    /// its budget spent (`LastRing::copy`).
    reshapes: usize,
    past: usize,
    copy: usize,
    /// The flow harnesses' check of it (`InputWork::open`, under the same
    /// budget), with its reshape predicted, and with none.
    held: Result<(), String>,
    unpredicted: Result<(), String>,
}

/// `text` laid along `curve` in `fit`, with `budget` in place of its own
/// when given, and the reshape [`path_reshape_steps`] predicts for it.
fn lay(text: &str, curve: Curve, fit: &FitMode, budget: Option<usize>) -> (Laid, usize) {
    let flow = style(POINTS);
    with_fonts(|fs| {
        let run = shape_run(
            &[FlowSpan {
                text,
                style: &flow,
                link: None,
            }],
            1.0,
            fs,
        );
        let predicted = path_reshape_steps(&run, fit, &flow, fs.fonts());
        RESHAPES_ALONG.with(|n| n.set(0));
        PAST_BUDGET.with(|n| n.set(0));
        let lay = |fs: &mut crate::font::fonts::FontSystemLease| {
            let mut trace = Trace::default();
            let placed = place_on_open_capped(
                &run,
                &[curve],
                fit,
                0.0,
                &flow,
                fs,
                MAX_PLACED_GLYPHS,
                Some(&mut trace),
            )
            .1;
            let check = |reshape: usize| {
                let tables = trace.tables;
                InputWork::default().open(
                    &run,
                    MAX_PLACED_GLYPHS,
                    tables,
                    reshape,
                    &placed,
                    &trace,
                    "a placement",
                )
            };
            let (held, unpredicted) = (check(predicted), check(0));
            (placed, trace, held, unpredicted)
        };
        let (placed, trace, held, unpredicted) = match budget {
            Some(budget) => with_seam(&BUDGET_OVERRIDE, budget, || lay(fs)),
            None => lay(fs),
        };
        let (last, _, _) = LAST_OPEN
            .with(std::cell::Cell::get)
            .expect("a placement along an open path is recorded");
        let laid = Laid {
            placed,
            trace,
            reshapes: RESHAPES_ALONG.with(std::cell::Cell::get),
            past: PAST_BUDGET.with(std::cell::Cell::get),
            copy: last.copy,
            held,
            unpredicted,
        };
        (laid, predicted)
    })
}

/// **Each reshape is counted, and one at most runs past the budget**
/// (#387). Before #387 no work counted a placement along an open path's
/// reshapes at all: a `Fit` reshapes once a round, up to 40 rounds, and a
/// text that shapes again — one with a tab, or of code points no face
/// draws — pays a shaping each time.
///
/// - "Ἐν ἀρχῇ\tἦν ὁ λόγος" round a tight cubic in `Fit { min: 0.1, max: 1.0 }`
///   takes several rounds at its own budget, each reshape charging what
///   `path_reshape_steps` predicts (`Trace::reshaped`), and the flow
///   harnesses' check of a placement accepts the prediction and refuses
///   none. Laid at every budget from what its first walks spend to what
///   it spends whole, it never runs past its budget by more than the
///   overshoot with one reshape (`ring_overshoot` of its reshape), at some
///   of them past the overshoot with none, and never makes more reshapes
///   than at its own budget; any budget short of what it spends whole
///   leaves it reported.
/// - "MANDALA" with a filler "·" along a line it leaves room on charges its
///   filler's shaping with the text, exactly as predicted; along one it
///   fills, it makes no reshape and records none.
///
/// Controls, each run and seen red: `fit_to` charging no reshape, the
/// recorded reshape 0 against the prediction; `once` charging no filler,
/// its filler's shaping unseen; `InputWork::open`'s bound with no
/// reshape, the budgets a reshape crosses refused; `path_reshape_steps`
/// without its `Once` arm, the filler's charge not as predicted.
#[test]
fn test_flow_open_counts_each_reshape_and_runs_one_past_its_budget() {
    let tight = Curve::Cubic(
        Vec2::ZERO,
        Vec2::new(10.0, 200.0),
        Vec2::new(20.0, -200.0),
        Vec2::new(30.0, 0.0),
    );
    let fit = FitMode::Fit { min: 0.1, max: 1.0 };
    let text = "Ἐν ἀρχῇ\tἦν ὁ λόγος";
    let (own, predicted) = lay(text, tight, &fit, None);
    assert!(
        own.reshapes >= 2,
        "precondition: its fit takes {} rounds",
        own.reshapes
    );
    assert!(predicted > 0, "precondition: a reshape is predicted");
    assert_eq!(own.trace.reshaped, predicted, "each reshape charged as predicted");
    assert!(
        own.trace.ring.is_none(),
        "precondition: laid whole at its own budget"
    );
    assert!(own.held.is_ok(), "held to its prediction: {:?}", own.held);
    assert!(own.unpredicted.is_err(), "refused with no reshape predicted");
    let (first, _) = lay(text, tight, &fit, Some(1));
    let (from, to) = (first.trace.spent, own.trace.spent);
    assert!(
        from < to,
        "precondition: its first walks spend less than it does whole"
    );
    let mut past_with_none = 0;
    for step in 0..=60 {
        let budget = from + (to - from) * step / 60;
        let (laid, _) = lay(text, tight, &fit, Some(budget));
        let allowed = ring_overshoot(laid.copy, laid.trace.tables, predicted);
        assert!(
            laid.trace.reshaped == 0 || laid.trace.reshaped == predicted,
            "budget {budget}: a reshape charged {}, predicted {predicted}",
            laid.trace.reshaped
        );
        assert!(
            laid.past <= allowed,
            "budget {budget}: {} past its budget, against {allowed} with one reshape",
            laid.past
        );
        assert!(
            laid.held.is_ok(),
            "budget {budget}: the flow harnesses' check with its reshape predicted: {:?}",
            laid.held
        );
        assert!(
            laid.reshapes <= own.reshapes,
            "budget {budget}: {} reshapes, past its own budget's {}",
            laid.reshapes,
            own.reshapes
        );
        if budget < to {
            assert!(
                laid.trace.ring.is_some() || laid.placed == own.placed,
                "budget {budget}: laid otherwise than whole, and reported nothing"
            );
        }
        past_with_none += usize::from(laid.past > ring_overshoot(laid.copy, laid.trace.tables, 0));
    }
    assert!(
        past_with_none > 0,
        "precondition: some budget is run past by a reshape, past the overshoot with none"
    );
    let once = FitMode::Once {
        filler: Some("·".into()),
    };
    let (room, filled) = lay(
        "MANDALA",
        Curve::Line(Vec2::ZERO, Vec2::new(400.0, 0.0)),
        &once,
        None,
    );
    assert_eq!(room.reshapes, 1, "its filler shaped once");
    assert_eq!(
        room.trace.reshaped, filled,
        "its filler's shaping charged as predicted"
    );
    assert!(room.held.is_ok(), "held to its prediction: {:?}", room.held);
    assert!(filled > 0, "and a filler's shaping is predicted");
    let (full, _) = lay(
        "MANDALA",
        Curve::Line(Vec2::ZERO, Vec2::new(20.0, 0.0)),
        &once,
        None,
    );
    assert_eq!(full.reshapes, 0, "a text that leaves no room shapes no filler");
    assert_eq!(full.trace.reshaped, 0, "and records none");
}

/// **A placement whose reshape leaves no cap is cut, and counted** (#387):
/// held to the timeout, the flow harnesses lay a placement along an open
/// path with its one predicted reshape counted in what it may take
/// (`open_cap`), and cut its text until a cap holds it
/// ([`open_in_time`](crate::flow::tests::flow_fuzz::open_in_time)), or lay
/// a `Once` with no filler where its filler's shaping leaves none even for
/// no text. On the compiled faces alone, a tab and 250 code points no face
/// draws along a line 2,000 long:
///
/// - in `Fit`, its reshape — a shaping, a `.notdef` pass a code point — is
///   past what any cap leaves, the whole text is held by none; it is cut,
///   counted, and laid within the timeout, its reshape charged as
///   predicted for the run it is laid as;
/// - as a `Once` filler of "MANDALA", the filler's shaping leaves no cap
///   even for no text: it is left off, counted, and "MANDALA" is laid
///   whole within the timeout;
/// - replaying a finding, neither is cut nor left off, and each is held
///   to the most it may take with its reshape.
///
/// In a child process (`util::tests::compiled_faces`): a host face here
/// maps every code point `compiled_faces::undrawn` reads.
///
/// Controls, each run and seen red: `open_cap` reading no reshape, the
/// `Fit` laid whole past the timeout; `open_in_time` cutting nothing, the
/// same; `open_in_time` keeping every filler, the filler not left off.
#[test]
fn test_flow_open_a_reshape_that_leaves_no_cap_is_cut_and_counted() {
    use crate::util::tests::compiled_faces::{on_compiled_faces, report_hosts, this_test, CHILD};
    let name = this_test!();
    if std::env::var(CHILD).as_deref() == Ok("cut") {
        report_hosts();
        cut_counted();
        println!("REPORT CUT counted");
        return;
    }
    let Some(reports) = on_compiled_faces(&name, "cut") else {
        return;
    };
    for report in &reports {
        eprintln!("{report}");
    }
    assert_eq!(
        reports.len(),
        5,
        "the child's four placements and its check: {reports:#?}"
    );
    assert_eq!(
        reports.last().map(String::as_str),
        Some("CUT counted"),
        "the child's check"
    );
}

/// [`test_flow_open_a_reshape_that_leaves_no_cap_is_cut_and_counted`]'s
/// check, in its child.
fn cut_counted() {
    use crate::flow::place::ring::ring_work_steps;
    use crate::flow::tests::flow_fuzz::{
        check_open, open_tables, INPUT_STEPS, OPEN_CUT, OPEN_FILLER_LEFT_OUT,
    };
    let flow = style(POINTS);
    let line = Curve::Line(Vec2::ZERO, Vec2::new(2_000.0, 0.0));
    let tables = open_tables(&[line]);
    let undrawn: String = with_fonts(|fs| {
        let undrawn = crate::util::tests::compiled_faces::undrawn(fs, 250);
        assert_eq!(undrawn.len(), 250, "precondition: 250 code points no face draws");
        undrawn.into_iter().collect()
    });
    let tabbed = format!("\t{undrawn}");
    let rows = [
        (
            "a tab and 250 code points no face draws, in `Fit`",
            tabbed.as_str(),
            FitMode::Fit { min: 0.5, max: 2.0 },
            true,
        ),
        (
            "\"MANDALA\" with a filler of 250 code points no face draws",
            "MANDALA",
            FitMode::Once {
                filler: Some(undrawn.clone()),
            },
            false,
        ),
    ];
    for (what, text, fit, cut) in rows {
        for bounded in [true, false] {
            let spans = [FlowSpan {
                text,
                style: &flow,
                link: None,
            }];
            let (whole, cuts, left, checked) = with_fonts(|fs| {
                let run = shape_run(&spans, 1.0, fs);
                let reshape = path_reshape_steps(&run, &fit, &flow, fs.fonts());
                // What it may take whole at no cap at all, its reshape
                // counted: read apart from `open_cap`, so that the cut is
                // the harness's.
                let whole = ring_work_steps(0, run.glyphs.len(), tables, reshape);
                let (c0, l0) = (
                    OPEN_CUT.with(std::cell::Cell::get),
                    OPEN_FILLER_LEFT_OUT.with(std::cell::Cell::get),
                );
                let mut work = InputWork::default();
                let checked = check_open(&spans, &run, &line, &fit, 0.0, 1.0, bounded, &flow, &mut work, fs)
                    .and_then(|()| {
                        if bounded {
                            work.within_the_timeout()
                        } else {
                            Ok(())
                        }
                    });
                let cuts = OPEN_CUT.with(std::cell::Cell::get) - c0;
                let left = OPEN_FILLER_LEFT_OUT.with(std::cell::Cell::get) - l0;
                (whole, cuts, left, checked)
            });
            println!("REPORT OPEN {what}, held to the timeout {bounded}: whole at no cap {whole}, cut {cuts}, filler left off {left}, {checked:?}");
            assert!(
                whole > INPUT_STEPS,
                "{what}: precondition: no cap holds it whole with its reshape ({whole})"
            );
            assert!(
                checked.is_ok(),
                "{what}, held to the timeout {bounded}: {checked:?}"
            );
            if bounded {
                assert_eq!(cuts, usize::from(cut), "{what}: cut, and counted");
                assert_eq!(
                    left,
                    usize::from(!cut),
                    "{what}: its filler left off, and counted"
                );
            } else {
                assert_eq!((cuts, left), (0, 0), "{what}: replaying a finding, laid as found");
            }
        }
    }
}
