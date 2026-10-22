// SPDX-License-Identifier: MPL-2.0

//! The native [`CommandTarget`]: what an IPC command handler drives
//! the app through. It wraps the [`InputHandlerContext`] every input
//! handler receives, and each method is one existing funnel and
//! nothing else (`work_plans/LLM_IPC.md` §D5, "Crossing the `app`
//! boundary"). An IPC-driven action, console line or macro therefore
//! takes the same arms, the same gates and the same undo entries as
//! a keystroke would.
//!
//! Native-only at this module boundary, with the module it serves
//! (`CLAUDE.md` "Dual-target status").

#![cfg(not(target_arch = "wasm32"))]

use glam::Vec2;

use crate::application::console::LineOutcome;
use crate::application::ipc::space::{Point, Space};
use crate::application::ipc::CommandTarget;
use crate::application::keybinds::Action;
use crate::application::macros::MacroStep;
use crate::application::source_tier::SourceTier;

use super::console_input::exec::execute_console_line;
use super::dispatch::{self, DispatchHit, DispatchOutcome};
use super::input_context::InputHandlerContext;

/// How an anonymous `act.macro` step sequence is named in the macro
/// dispatcher's diagnostics.
const IPC_STEPS_LABEL: &str = "(ipc steps)";

/// The app, as one IPC command sees it. Built per command, like an
/// [`InputHandlerContext`] is built per event.
pub(super) struct IpcCommandTarget<'a> {
    ctx: InputHandlerContext<'a>,
}

impl<'a> IpcCommandTarget<'a> {
    pub(super) fn new(ctx: InputHandlerContext<'a>) -> Self {
        IpcCommandTarget { ctx }
    }

    /// Stand a synthetic pointer at `point`, as a real one would have
    /// stood before its press: the cursor moves there, and the click
    /// hit is what the canonical hit test finds under it
    /// (`compute_click_hit`, the one mouse presses use, never a
    /// fork of it).
    fn place_pointer(&mut self, point: Point) -> DispatchHit {
        let (surface, canvas) = match point.space {
            Space::Surface => (
                Vec2::new(point.x, point.y),
                self.ctx.renderer.screen_to_canvas(point.x, point.y),
            ),
            Space::Canvas => {
                let canvas = Vec2::new(point.x, point.y);
                (self.ctx.renderer.canvas_to_screen(canvas), canvas)
            }
        };
        *self.ctx.cursor_pos = (f64::from(surface.x), f64::from(surface.y));
        let parts = super::compute_click_hit(canvas, self.ctx.mindmap_tree.as_mut(), self.ctx.app_scene);
        DispatchHit {
            click_hit: parts.click_hit,
            canvas_pos: canvas,
        }
    }
}

impl CommandTarget for IpcCommandTarget<'_> {
    fn dispatch_action(&mut self, action: Action, pointer: Option<Point>) -> bool {
        let hit = pointer.map(|point| self.place_pointer(point));
        matches!(
            dispatch::dispatch_action(action, &mut self.ctx, hit.as_ref()),
            DispatchOutcome::Handled
        )
    }

    fn run_console_line(&mut self, line: &str) -> Option<LineOutcome> {
        let ctx = &mut self.ctx;
        let doc = ctx.document.as_mut()?;
        Some(execute_console_line(
            line,
            ctx.console_state,
            ctx.single_line_edit_state,
            ctx.color_picker_state,
            ctx.text_edit_state,
            doc,
            ctx.interaction_mode,
            ctx.mindmap_tree,
            ctx.app_scene,
            ctx.renderer,
            ctx.scene_cache,
            ctx.macros,
        ))
    }

    fn has_macro(&self, macro_id: &str) -> bool {
        self.ctx.macros.get_with_source(macro_id).is_some()
    }

    fn run_macro(&mut self, macro_id: &str) -> bool {
        dispatch::dispatch_macro(macro_id, &mut self.ctx)
    }

    fn run_macro_steps(&mut self, steps: &[MacroStep]) -> bool {
        // `User`: the steps were authored by the IPC client, and IPC
        // executes at the User tier (format/ipc.md §Trust model).
        dispatch::run_macro_steps(IPC_STEPS_LABEL, steps, SourceTier::User, &mut self.ctx)
    }
}
