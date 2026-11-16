// SPDX-License-Identifier: MPL-2.0

//! Reading the map, not the performance, across a dispatch (ruling D2.1
//! §2 on #251; `format/presentations.md` "The document layer").
//!
//! A presentation playing on the document shows values that are not the
//! map's: a node lifted on hover, a span recolored, a node concealed.
//! The writer's funnel keeps them out of every change it applies, but a
//! change is often *built* from values read first — a copy, a paste
//! beside what is selected, a range of a section's text, a layout that
//! reads where things are — and a value read with the layer applied is
//! the performance's. So the funnels an input reaches the map through
//! set the layer aside for as long as what they run may read the map,
//! and put it back after:
//!
//! - an `Action` (`action_core::dispatch_compatible`, which native's
//!   `dispatch_action` runs first and whose arms are the only ones that
//!   read the map), unless it only moves the camera, changes the
//!   selection or toggles a readout: those read what is drawn, and
//!   leave the layer applied;
//! - a console verb's execution (`console_input::exec`), and `edit
//!   inspect`'s reading (`console_input::edits`);
//! - an edit (`families::edits::run_edit`), from a key, a macro step or
//!   the console.
//!
//! [`OnTheMap`] is the guard the Action funnel holds, since it hands
//! its context down; the other two hold the document's own
//! (`MindMapDocument::suspended`). Either way the layer comes back when
//! the guard is dropped — at the funnel's end, on an early return, and
//! while a panic unwinds — so no path through a dispatch can leave the
//! map unperformed. Suspensions nest
//! (`Document::suspend_presentations`), so a console line that runs an
//! edit, and a macro that dispatches Actions, each set it aside once
//! more and put it back once more.
//!
//! The view may be brought up to date inside a guard (an edit's handler
//! reads where things are drawn): it then projects the map itself,
//! which is what such a read wants.

use std::ops::{Deref, DerefMut};

use crate::application::document::{MindMapDocument, Suspension};

use super::super::input_context_core::InputContextCore;

/// A dispatch context whose map has its presentation layer set aside,
/// until this is dropped. See the [module docs](self).
pub(in crate::application::app) struct OnTheMap<'c, 'a> {
    core: &'c mut InputContextCore<'a>,
    /// The suspension this guard made, when it set the layer aside, and
    /// so gives back.
    suspended: Option<Suspension>,
}

impl<'c, 'a> OnTheMap<'c, 'a> {
    /// `core`, with its map's presentation layer set aside when
    /// `reads_the_map` — and left applied, the guard doing nothing,
    /// when what is about to run reads only what is drawn.
    pub(in crate::application::app) fn when(core: &'c mut InputContextCore<'a>, reads_the_map: bool) -> Self {
        let suspended = reads_the_map
            .then(|| {
                core.document
                    .as_deref_mut()
                    .map(MindMapDocument::suspend_presentations)
            })
            .flatten();
        OnTheMap { core, suspended }
    }
}

impl<'a> Deref for OnTheMap<'_, 'a> {
    type Target = InputContextCore<'a>;

    fn deref(&self) -> &InputContextCore<'a> {
        self.core
    }
}

impl<'a> DerefMut for OnTheMap<'_, 'a> {
    fn deref_mut(&mut self) -> &mut InputContextCore<'a> {
        self.core
    }
}

impl Drop for OnTheMap<'_, '_> {
    /// Put the layer back, on the document the context holds.
    fn drop(&mut self) {
        if let Some(suspension) = self.suspended.take() {
            if let Some(doc) = self.core.document.as_deref_mut() {
                doc.resume_presentations(suspension);
            }
        }
    }
}
