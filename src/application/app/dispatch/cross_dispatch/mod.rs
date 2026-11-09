// SPDX-License-Identifier: MPL-2.0

//! Cross-platform `Action` arm bodies, grouped by concept.
//!
//! Each function in this directory implements one or more `Action::*`
//! variant bodies in a form callable from BOTH the native dispatcher
//! ([`super::action_core`] + `super::native::dispatch_action`) and the
//! WASM dispatcher (`super::super::run_wasm`). Both of those are plain
//! code-spans: each is cfg-gated to one target, and rustdoc resolves
//! links against the active target's module tree, so either link breaks
//! the other target's doc build.
//!
//! A body that changed the document or the selection ends by syncing the
//! view ([`ViewContext::sync`]): the one pipeline from the document's
//! change feed to the renderer (`crate::application::view`). A body that
//! moves the camera syncs nothing (§12.4).
//!
//! ## Per-concept layout
//!
//! - [`lifecycle`]: undo and redo.
//! - [`fps`]: renderer-side FPS overlay toggles (no document mutation).
//! - [`camera`]: zoom step / reset / fit, pan nudges, center-on-selection
//!   and jump-to-root.
//! - [`selection`]: the selection-changing arms (`SelectAll`,
//!   `DeselectAll`, `InvertSelection`, `SelectParent`, `SelectChild`,
//!   `SelectNext/PrevSibling`), and the selection a click or a tap makes.
//! - [`mod@pointer`]: the pointer-gesture payload ([`DispatchHit`]) and
//!   the touch route.
//!
//! The Actions whose bodies edited the old model are not here: each is
//! answered by its owner's family (`super::families`).

use crate::application::document::MindMapDocument;
use crate::application::renderer::Renderer;
use crate::application::view::MapView;

mod camera;
mod fps;
mod lifecycle;
mod pointer;
mod selection;

pub(in crate::application::app) use camera::*;
pub(in crate::application::app) use fps::*;
pub(in crate::application::app) use lifecycle::*;
pub(in crate::application::app) use pointer::*;
pub(in crate::application::app) use selection::*;

/// Whether an `Action` dispatch ran its body.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum DispatchOutcome {
    /// The body ran.
    Handled,
    /// Nothing ran: the Action does not apply in the current context, or
    /// its family is rebuilt by an owner issue (`super::families`).
    Unhandled,
    /// Nothing ran: the arm's guard declined, for this reason (#194) —
    /// on both targets: the structure family's declines and D28 refusals
    /// are the browser's.
    Refused(&'static str),
}

/// What a body that changes the document or the selection needs: the
/// document, its view and the renderer the view draws into.
pub(in crate::application::app) struct ViewContext<'a> {
    pub document: &'a mut MindMapDocument,
    pub view: &'a mut MapView,
    pub renderer: &'a mut Renderer,
}

impl ViewContext<'_> {
    /// Bring the view and the renderer up to date with the document and
    /// its selection: the one pipeline (`MapView::sync`).
    pub fn sync(&mut self) {
        self.view.sync(self.document, self.renderer);
    }
}

/// A [`ViewContext`] over `doc` and the view and renderer `ctx` holds
/// (any context with `view: &mut Option<MapView>` and `renderer`
/// fields), or `None` before a view exists.
macro_rules! view_ctx {
    ($ctx:expr, $doc:expr) => {
        $ctx.view.as_mut().map(
            |view| $crate::application::app::dispatch::cross_dispatch::ViewContext {
                document: $doc,
                view,
                renderer: &mut *$ctx.renderer,
            },
        )
    };
}
pub(in crate::application::app) use view_ctx;
