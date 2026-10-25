// SPDX-License-Identifier: MPL-2.0

//! The "rendered" half of `clock.wait until=settled` (`format/ipc.md`
//! §`clock.wait`): whether the frame the app last asked for has been
//! drawn — and, with no window, the record of which frames are owed at
//! all, which the headless driver draws by ([`RedrawLedger::frame_owed`]).
//!
//! **Asked, then drawn.** Every redraw the native shell requests goes
//! through `InitState::request_redraw`, which counts it here; a
//! `RedrawRequested` whose render ran records that it drew everything
//! asked for up to that moment. The two counters being equal is the
//! statement `settled` needs — no redraw outstanding, and the last
//! render showing every change that asked for one — and it holds for
//! view-only changes (a zoom, a selection) exactly as for document
//! ones, because what is counted is the ask, not what caused it. That
//! is the render revision §D7 calls for, kept where the asks are made
//! rather than inferred from the document.
//!
//! **Drawn means rendered, not presented.** A render whose CPU half ran
//! — every buffer current, every glyph prepared — has drawn, whether
//! or not the surface then had a texture to present into: an occluded
//! window that never presents must still let `settled` resolve, and so
//! must a headless app, whose frames draw nothing at all because a
//! capture draws its own. A frame the renderer skipped (the font system
//! contended, a failed prepare) has not drawn, and the ask stays
//! outstanding — retried when the skip was transient, and otherwise
//! stalled until something new is asked for.

/// Redraws asked for, the last ask a render satisfied, and how the
/// renders so far have gone.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct RedrawLedger {
    asked: u64,
    drawn: u64,
    /// A render has run — drawn or skipped, asked for or not: a
    /// window's first frame is delivered unasked when it is exposed, and
    /// a headless app owes one from the start ([`Self::frame_owed`]).
    attempted: bool,
    /// The last render was skipped for a reason asking again would not
    /// cure (`FrameSkip::is_transient` false): the frame stays owed, and
    /// nothing will draw it until something new is asked for.
    stalled: bool,
}

impl RedrawLedger {
    /// A redraw was requested. A new ask is worth a new render even
    /// after a stall: what failed may have changed.
    pub(super) fn ask(&mut self) {
        self.asked += 1;
        self.stalled = false;
    }

    /// How many redraws have been asked for: what a render that starts
    /// now will satisfy if it draws.
    pub(super) fn asked(&self) -> u64 {
        self.asked
    }

    /// A render that began when [`Self::asked`] read `asked` drew.
    pub(super) fn drew(&mut self, asked: u64) {
        self.drawn = self.drawn.max(asked);
        self.attempted = true;
        self.stalled = false;
    }

    /// A render was skipped; `transient` says whether the same render
    /// asked for again would likely draw. A transient skip's retry is
    /// the caller's to ask for; a lasting one stalls the ledger.
    pub(super) fn skipped(&mut self, transient: bool) {
        self.attempted = true;
        self.stalled = !transient;
    }

    /// Whether a render has run at all: the app has had its first
    /// frame, and with it a window of the size it was given. What a
    /// one-shot capture waits for beyond settling
    /// (`capture_flow::one_shot`).
    pub(super) fn attempted(&self) -> bool {
        self.attempted
    }

    /// Whether the frame owed is one no render will draw until
    /// something new is asked for ([`Self::skipped`]).
    pub(super) fn is_stalled(&self) -> bool {
        self.stalled
    }

    /// Nothing asked for is still undrawn.
    pub(super) fn is_current(&self) -> bool {
        self.drawn == self.asked
    }

    /// Whether a render is owed now: the first frame, or an ask no
    /// render has drawn and that has not stalled. What the headless
    /// driver draws by — the role winit's `RedrawRequested` plays under
    /// a window, which also delivers the first frame unasked and one
    /// frame per coalesced batch of asks, and none for a stalled one.
    pub(super) fn frame_owed(&self) -> bool {
        !self.attempted || (!self.is_current() && !self.stalled)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_ledger_with_nothing_asked_is_current() {
        assert!(RedrawLedger::default().is_current());
    }

    /// The first frame is owed before anything is asked, and counts
    /// whether or not it drew; an unasked first frame satisfies no ask
    /// made after it.
    #[test]
    fn test_the_first_frame_is_owed_unasked() {
        let mut ledger = RedrawLedger::default();
        assert!(ledger.frame_owed() && !ledger.attempted());
        let unasked = ledger.asked();
        ledger.drew(unasked);
        assert!(ledger.attempted() && !ledger.frame_owed());
        ledger.ask();
        assert!(ledger.frame_owed() && !ledger.is_current());
    }

    /// A lasting skip stalls the owed frame — no render is owed until a
    /// new ask, which retries — while a transient one leaves the retry
    /// to the ask the caller makes.
    #[test]
    fn test_a_lasting_skip_stalls_the_frame_until_the_next_ask() {
        let mut ledger = RedrawLedger::default();
        ledger.ask();
        ledger.skipped(false);
        assert!(ledger.attempted() && ledger.is_stalled());
        assert!(!ledger.frame_owed(), "retrying would spin");
        assert!(!ledger.is_current(), "and the frame is still undrawn");
        ledger.ask();
        assert!(
            !ledger.is_stalled() && ledger.frame_owed(),
            "a new ask tries again"
        );

        let mut transient = RedrawLedger::default();
        transient.ask();
        transient.skipped(true);
        assert!(!transient.is_stalled() && transient.frame_owed());
    }

    /// An ask is owed until a render that started after it draws.
    #[test]
    fn test_an_ask_is_owed_until_a_render_draws_it() {
        let mut ledger = RedrawLedger::default();
        ledger.ask();
        assert!(!ledger.is_current());
        let asked = ledger.asked();
        ledger.drew(asked);
        assert!(ledger.is_current());
    }

    /// A render that was skipped records nothing, so the ask stays owed;
    /// and an ask made after a render began is not satisfied by it.
    #[test]
    fn test_a_skipped_render_or_a_later_ask_leaves_the_frame_owed() {
        let mut ledger = RedrawLedger::default();
        ledger.ask();
        let asked = ledger.asked();
        // The render is skipped: `drew` is never called.
        assert!(!ledger.is_current());
        ledger.ask();
        ledger.drew(asked);
        assert!(!ledger.is_current(), "the second ask came after the render began");
        let asked = ledger.asked();
        ledger.drew(asked);
        assert!(ledger.is_current());
    }

    /// Two asks coalesced into one render are both satisfied by it, as
    /// winit coalesces the requests into one `RedrawRequested`.
    #[test]
    fn test_one_render_satisfies_every_ask_before_it() {
        let mut ledger = RedrawLedger::default();
        ledger.ask();
        ledger.ask();
        ledger.ask();
        let asked = ledger.asked();
        ledger.drew(asked);
        assert!(ledger.is_current());
    }
}
