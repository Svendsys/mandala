// SPDX-License-Identifier: MPL-2.0

//! [`Draft`]: what both text editors are made of — one text owner's
//! [`EditBuffer`], a caret and a shift-select anchor in its grapheme
//! clusters, and the transaction its changes are staged in. The section
//! editor ([`super::TextSession`]) and the single-line editor
//! (`app::single_line_edit`, native) each hold one and add what is
//! theirs: the section editor its node and its orphan, the single-line
//! editor the label it may have created.

use baumhard::arena::NodeId;
use baumhard::document::edit_buffer::EditBuffer;
use baumhard::document::element::SpanData;
use baumhard::document::mutation::compose::replace_spans;
use baumhard::document::writer::EditRequest;
use baumhard::util::grapheme_chad;

use super::{line_down, line_end, line_start, line_up, Edit};
use crate::application::document::{Interaction, MindMapDocument};

/// One editing primitive, as both editors' Actions name them.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(in crate::application::app) enum Step {
    /// One cluster left.
    Left,
    /// One cluster right.
    Right,
    /// One line up, keeping the column.
    Up,
    /// One line down, keeping the column.
    Down,
    /// The start of the line.
    Home,
    /// The end of the line.
    End,
    /// The start of the word before.
    WordLeft,
    /// The end of the word after.
    WordRight,
    /// Delete the cluster before the caret.
    DeleteBack,
    /// Delete the cluster after the caret.
    DeleteForward,
    /// Delete back to the start of the word before.
    DeleteWordBack,
    /// Delete on to the end of the word after.
    DeleteWordForward,
    /// Type a space.
    Space,
}

/// One owner's text, as an editor holds it while its transaction is open.
#[derive(Debug)]
pub(in crate::application::app) struct Draft {
    /// The element whose spans are edited.
    owner: NodeId,
    /// The interaction whose transaction its changes are staged in.
    interaction: Interaction,
    /// The working copy of the owner's spans.
    buffer: EditBuffer,
    /// What the document holds now: the buffer as last staged, which a
    /// refused stage returns the buffer to.
    staged: EditBuffer,
    /// The owner's spans when the editor opened.
    original: Vec<SpanData>,
    /// The caret, a grapheme index into the buffer.
    caret: usize,
    /// Where a shift-select began, while one is live.
    anchor: Option<usize>,
}

impl Draft {
    /// A draft of `buffer`, the spans of `owner`, staged in
    /// `interaction`'s transaction, its caret at the end. O(the spans).
    pub(in crate::application::app) fn new(
        owner: NodeId,
        interaction: Interaction,
        buffer: EditBuffer,
    ) -> Self {
        Draft {
            owner,
            interaction,
            caret: buffer.grapheme_len(),
            staged: buffer.clone(),
            original: buffer.spans().to_vec(),
            buffer,
            anchor: None,
        }
    }

    /// The element whose spans are edited. O(1).
    pub(in crate::application::app) fn owner(&self) -> NodeId {
        self.owner
    }

    /// The caret. O(1).
    pub(in crate::application::app) fn caret(&self) -> usize {
        self.caret
    }

    /// Where a live shift-select began. O(1).
    pub(in crate::application::app) fn anchor(&self) -> Option<usize> {
        self.anchor
    }

    /// The buffer. O(1).
    pub(in crate::application::app) fn buffer(&self) -> &EditBuffer {
        &self.buffer
    }

    /// Whether the buffer holds what the owner held when the editor
    /// opened. O(the spans).
    pub(in crate::application::app) fn unchanged(&self) -> bool {
        self.buffer.spans() == self.original.as_slice()
    }

    /// Whether its transaction is still the open one: an undo cancels an
    /// interactive transaction (ruling R36), and then the editor is over,
    /// with nothing written. O(1).
    pub(in crate::application::app) fn is_live(&self, doc: &MindMapDocument) -> bool {
        doc.open_interaction() == Some(self.interaction)
    }

    /// Clear the buffer: an editor that opens clean. O(the text).
    pub(in crate::application::app) fn clear(&mut self) {
        self.buffer.delete(0, self.buffer.grapheme_len());
        self.caret = 0;
        self.anchor = None;
    }

    /// Run one primitive. A move with `select` keeps the anchor, seeding
    /// it where the caret was; every other step drops it, as on `main`.
    /// O(the text).
    pub(in crate::application::app) fn step(&mut self, step: Step, select: bool) -> Edit {
        let (caret, anchor) = (self.caret, self.anchor);
        if select {
            self.anchor.get_or_insert(caret);
        } else {
            self.anchor = None;
        }
        let len = self.buffer.grapheme_len();
        let text = self.buffer.text();
        let mut changed = false;
        match step {
            Step::Left => self.caret = caret.saturating_sub(1),
            Step::Right => self.caret = (caret + 1).min(len),
            Step::Up => self.caret = line_up(&text, caret),
            Step::Down => self.caret = line_down(&text, caret),
            Step::Home => self.caret = line_start(&text, caret),
            Step::End => self.caret = line_end(&text, caret),
            Step::WordLeft => self.caret = grapheme_chad::word_left(&text, caret),
            Step::WordRight => self.caret = grapheme_chad::word_right(&text, caret),
            Step::DeleteBack if caret > 0 => {
                self.buffer.delete(caret - 1, caret);
                self.caret = caret - 1;
                changed = true;
            }
            Step::DeleteForward if caret < len => {
                self.buffer.delete(caret, caret + 1);
                changed = true;
            }
            Step::DeleteWordBack => {
                let target = grapheme_chad::word_left(&text, caret);
                if target < caret {
                    self.buffer.delete(target, caret);
                    self.caret = target;
                    changed = true;
                }
            }
            Step::DeleteWordForward => {
                let target = grapheme_chad::word_right(&text, caret).min(len);
                if target > caret {
                    self.buffer.delete(caret, target);
                    changed = true;
                }
            }
            Step::Space => {
                self.caret = caret + self.buffer.insert(caret, " ");
                changed = true;
            }
            Step::DeleteBack | Step::DeleteForward => {}
        }
        self.caret = self.caret.min(self.buffer.grapheme_len());
        if changed {
            Edit::Text
        } else if self.caret != caret || self.anchor != anchor {
            Edit::Caret
        } else {
            Edit::None
        }
    }

    /// Insert `text` at the caret and move the caret past it, dropping
    /// the anchor. A combining mark that merges into the cluster before it
    /// leaves the caret where it was, though the text changed — `main`'s
    /// IME primitive. O(the text).
    pub(in crate::application::app) fn insert(&mut self, text: &str) -> Edit {
        if text.is_empty() {
            return Edit::None;
        }
        self.anchor = None;
        self.caret += self.buffer.insert(self.caret, text);
        self.caret = self.caret.min(self.buffer.grapheme_len());
        Edit::Text
    }

    /// Stage the buffer under `label`: one request replacing the owner's
    /// spans with the buffer's (design §9: once per change, never a
    /// growing list — the writer reverts the previous stage first). A
    /// refused stage — a text over the document's budget, say — leaves the
    /// document as it was, and the buffer goes back to what is staged, so
    /// the two never part. Returns whether it was accepted.
    ///
    /// Cost: the previous stage's revert and one walk per span.
    pub(in crate::application::app) fn stage(&mut self, doc: &mut MindMapDocument, label: &str) -> bool {
        let accepted = replace_spans(doc.doc(), &[self.owner], self.buffer.spans())
            .ok()
            .is_some_and(|steps| {
                doc.stage(self.interaction, EditRequest::new(steps, label))
                    .is_ok()
            });
        if accepted {
            self.staged.clone_from(&self.buffer);
        } else {
            self.buffer.clone_from(&self.staged);
            self.caret = self.caret.min(self.buffer.grapheme_len());
            self.anchor = None;
        }
        accepted
    }
}
