// SPDX-License-Identifier: MPL-2.0

//! The app-side execution context a command handler drives the
//! application through (`work_plans/LLM_IPC.md` §D5 "Crossing the
//! `app` boundary").
//!
//! The funnels a command reaches are `dispatch_action`,
//! `dispatch_macro` and `execute_console_line`. They are private to
//! `crate::application::app`, and this module is that module's
//! sibling. So `app` implements [`CommandTarget`] over the same
//! `InputHandlerContext` an input handler receives and passes it in
//! as `&mut dyn`. `MacroDispatchTarget` uses the same indirection,
//! and it keeps both the funnels private and this module free of
//! `app` internals.
//!
//! **The trait adds no route around a gate.** Each method is one of
//! the existing funnels and nothing else. A future command that
//! needs a path past a privilege gate is a redesign of `format/ipc.md`
//! §Trust model, not a new method here (§D4's composition rule).

use crate::application::console::LineOutcome;
use crate::application::keybinds::Action;
use crate::application::macros::MacroStep;

use super::space::Point;

/// What a command may ask of the running application.
///
/// Every method executes on the main thread, inside the IPC drain
/// that runs right after `drain_inputs`, with the access an input
/// handler has.
pub(crate) trait CommandTarget {
    /// Run `action` through `dispatch_action`, exactly as a key
    /// binding fires it. With `pointer`, the dispatch also carries the
    /// payload a mouse handler builds before it dispatches. The
    /// cursor stands at that point, and the `DispatchHit` holds what
    /// the canonical click hit test finds there, so pointer-driven
    /// arms (`DoubleClickActivate`, the orphan-creation and zoom
    /// arms) behave as they do under a real pointer.
    ///
    /// Returns whether the funnel reported `Handled`.
    fn dispatch_action(&mut self, action: Action, pointer: Option<Point>) -> bool;

    /// Parse and run one console line through the console's own
    /// parse → execute → side-effect drain, without opening the
    /// console and without writing to its scrollback. `None` when no
    /// document is loaded: every verb runs against one.
    fn run_console_line(&mut self, line: &str) -> Option<LineOutcome>;

    /// Whether a macro with this id is registered in any tier.
    fn has_macro(&self, macro_id: &str) -> bool;

    /// Run a registered macro through `dispatch_macro`, under the
    /// tier its loader pinned. Returns the dispatcher's
    /// any-step-executed flag.
    fn run_macro(&mut self, macro_id: &str) -> bool;

    /// Run an anonymous step sequence through the same gated step
    /// loop, at the `User` tier: the steps were authored by the IPC
    /// client, and IPC executes as `User` (`format/ipc.md` §Trust
    /// model). Returns the any-step-executed flag.
    fn run_macro_steps(&mut self, steps: &[MacroStep]) -> bool;
}
