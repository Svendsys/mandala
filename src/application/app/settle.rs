// SPDX-License-Identifier: MPL-2.0

//! The "rendered" half of `clock.wait until=settled` (`format/ipc.md`
//! §`clock.wait`): whether the frame the app last asked for has been
//! drawn.
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
//! **Drawn means prepared, not presented.** A render whose CPU half ran
//! — every buffer current, every glyph prepared — has drawn, whether
//! or not the surface then had a texture to present into: an occluded
//! or headless window that never presents must still let `settled`
//! resolve. A frame the renderer skipped (the font system contended, a
//! failed prepare) has not drawn, and the ask stays outstanding.

/// Redraws asked for, and the last ask a render satisfied.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct RedrawLedger {
    asked: u64,
    drawn: u64,
}

impl RedrawLedger {
    /// A redraw was requested.
    pub(super) fn ask(&mut self) {
        self.asked += 1;
    }

    /// How many redraws have been asked for: what a render that starts
    /// now will satisfy if it draws.
    pub(super) fn asked(&self) -> u64 {
        self.asked
    }

    /// A render that began when [`Self::asked`] read `asked` drew.
    pub(super) fn drew(&mut self, asked: u64) {
        self.drawn = self.drawn.max(asked);
    }

    /// Nothing asked for is still undrawn.
    pub(super) fn is_current(&self) -> bool {
        self.drawn == self.asked
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_a_ledger_with_nothing_asked_is_current() {
        assert!(RedrawLedger::default().is_current());
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
