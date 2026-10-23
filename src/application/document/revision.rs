// SPDX-License-Identifier: MPL-2.0

//! The document's revision: a monotonic count of committed model
//! changes, and the funnel that made the latest one.
//!
//! `dirty` cannot answer "did anything change since I last looked":
//! it is the unsaved-changes flag, set by a setter and cleared only by
//! a save. The revision is the counter that can. It advances in
//! exactly two places, and both are here: [`MindMapDocument::mark_dirty`]
//! (which every document setter ends in, so a mutation cannot flip
//! `dirty` without also advancing the revision) and
//! [`MindMapDocument::advance_revision`] (which undo calls, because
//! undo mutates the model without dirtying it). A transient preview
//! touches neither, so a hover costs no revision.
//!
//! **Monotonic per process, not per document.** `open` and `new`
//! replace the whole document; [`MindMapDocument::succeed`] carries
//! the counter across the swap and bumps `generation`, so a client
//! comparing revisions never sees one go backwards, and the swap
//! itself is observable as a generation change.
//!
//! **The source** is which funnel was driving when the revision
//! advanced. Funnels declare themselves with
//! [`MindMapDocument::enter_source`] / [`MindMapDocument::leave_source`]
//! (the action dispatcher, the console line executor) or
//! [`MindMapDocument::with_source`] (undo, custom mutations, animation
//! completion, which are document methods). The innermost declaration
//! wins, so an undo reached through an `Action` reports `undo`.
//! Nothing declared means direct manipulation: a drag, a text-edit
//! commit, a picker commit.

use super::MindMapDocument;

/// Which funnel was driving when the revision last advanced.
///
/// The browser build declares none of these (nothing there reads the
/// source), so on wasm32 the variants are never constructed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[cfg_attr(target_arch = "wasm32", allow(dead_code))]
pub enum ChangeSource {
    /// No funnel was declared: direct manipulation reaching the model
    /// from a gesture handler (a drag commit, a text-edit commit).
    #[default]
    Input,
    /// `dispatch_action`: a key binding, a gesture, a macro step.
    Action,
    /// `execute_console_line`: a console verb.
    Console,
    /// A custom mutation applied, or an animated one completing.
    Mutation,
    /// `undo`.
    Undo,
}

impl ChangeSource {
    /// The word `document_changed` events carry (`format/ipc.md`
    /// §`events`).
    // Read by the native-only IPC event layer.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn word(self) -> &'static str {
        match self {
            ChangeSource::Input => "input",
            ChangeSource::Action => "action",
            ChangeSource::Console => "console",
            ChangeSource::Mutation => "mutation",
            ChangeSource::Undo => "undo",
        }
    }
}

/// The counters and funnel bookkeeping behind
/// [`MindMapDocument::revision`]. Owned by the document; every field
/// is written through the methods below.
#[derive(Debug, Clone, Default)]
pub struct Revision {
    /// How many documents this process has replaced before this one.
    generation: u64,
    /// Committed model changes, carried across a replacement.
    count: u64,
    /// The funnel that was driving at the latest advance.
    source: ChangeSource,
    /// The funnel driving right now.
    driver: ChangeSource,
    /// Animations started so far, carried across a replacement like
    /// the counters are: the identity of a running animation
    /// (`AnimationInstance::seq`) must not repeat in one process.
    animation_seq: u64,
}

impl MindMapDocument {
    /// The revision: committed model changes so far. Strictly
    /// increases across every mutation, undo and replacement; a read,
    /// a preview and a selection change leave it alone. O(1).
    // Read by the native-only IPC layer and the screenshot sidecar.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn revision(&self) -> u64 {
        self.revision.count
    }

    /// How many documents this process has replaced before this one.
    /// A change here is an `open` or `new`, not an edit. O(1).
    // Read by the native-only IPC event layer.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn generation(&self) -> u64 {
        self.revision.generation
    }

    /// The funnel that was driving when [`Self::revision`] last
    /// advanced. O(1).
    // Read by the native-only IPC event layer.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn last_change_source(&self) -> ChangeSource {
        self.revision.source
    }

    /// The model changed and is not saved: set `dirty` and advance the
    /// revision. **The one place a setter flips `dirty` on**
    /// (`test_no_production_code_sets_dirty_directly` holds that),
    /// which is what makes the revision complete without a second call
    /// beside each setter.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.advance_revision();
    }

    /// Advance the revision without touching `dirty`. For a change
    /// that is real but is not new unsaved work, which today is undo.
    pub fn advance_revision(&mut self) {
        self.revision.count += 1;
        self.revision.source = self.revision.driver;
    }

    /// Declare `source` the funnel now driving, and return the one it
    /// displaces for [`Self::leave_source`]. Funnels nest, and the
    /// innermost wins.
    pub fn enter_source(&mut self, source: ChangeSource) -> ChangeSource {
        std::mem::replace(&mut self.revision.driver, source)
    }

    /// Hand driving back to the funnel [`Self::enter_source`] displaced.
    pub fn leave_source(&mut self, displaced: ChangeSource) {
        self.revision.driver = displaced;
    }

    /// Run `body` with `source` driving.
    pub fn with_source<R>(&mut self, source: ChangeSource, body: impl FnOnce(&mut Self) -> R) -> R {
        let displaced = self.enter_source(source);
        let result = body(self);
        self.leave_source(displaced);
        result
    }

    /// The identity for an animation that is starting now.
    pub(super) fn next_animation_seq(&mut self) -> u64 {
        self.revision.animation_seq += 1;
        self.revision.animation_seq
    }

    /// Make `self` the successor of `predecessor` in this process: the
    /// revision continues from where it stopped, one past it, so the
    /// swap is itself a change, and the generation moves on. Called
    /// where a document replaces another (`open`, `new`).
    // Called by the console's document swap, which the browser lacks.
    #[cfg_attr(target_arch = "wasm32", allow(dead_code))]
    pub fn succeed(&mut self, predecessor: &MindMapDocument) {
        self.revision.generation = predecessor.revision.generation + 1;
        self.revision.count = predecessor.revision.count;
        self.revision.animation_seq = predecessor.revision.animation_seq;
        self.advance_revision();
    }
}
