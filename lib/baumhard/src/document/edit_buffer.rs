// SPDX-License-Identifier: MPL-2.0

//! [`EditBuffer`]: the working copy of one text owner's spans, which a
//! text editor changes keystroke by keystroke and stages into the
//! document whole (design §9: "`replace_spans` from the editor's span
//! buffer, once per change").
//!
//! It is pure: it reads the document once, at
//! [`EditBuffer::from_owner`], and never again. Every change after that
//! is made to the buffer alone, and the editor hands the document its
//! [`spans`](EditBuffer::spans) through
//! [`replace_spans`](crate::document::mutation::compose::replace_spans)
//! — one request per change, never a growing list of them.
//!
//! **Indices are grapheme clusters** (baumhard §B3), counted span by
//! span in order, exactly as the span builders count a section's text
//! ([`insert_text`](crate::document::mutation::compose::insert_text),
//! [`delete_text`](crate::document::mutation::compose::delete_text),
//! [`style_range`](crate::document::mutation::compose::style_range)):
//! a span's clusters are its own, so an index here, a selected range of
//! the app's and an index those builders take name the same place in the
//! same text.
//!
//! **The spans are always normalized**, by the span builders' rules: no
//! span is empty, and no two neighbors share their style and their link
//! (they would be one span). A buffer of no text holds no span; text
//! typed into it lands in a new span with nothing set, as
//! [`insert_text`](crate::document::mutation::compose::insert_text)
//! makes one.

use crate::arena::NodeId;
use crate::document::element::{DocElement, DocKind, SpanData};
use crate::document::ids::Document;
use crate::document::mutation::error::MutationError;
use crate::document::mutation::inherited::{inherit, inherited_seed, DocInherited};
use crate::document::mutation::ops::{apply_text, TextOp};
use crate::document::shape::cardinality;
use crate::util::grapheme_chad::{
    count_grapheme_clusters, delete_grapheme_range, insert_str_at_grapheme_counted,
};

/// The working copy of one text owner's spans. See the
/// [module docs](self).
#[derive(Clone, Debug, PartialEq)]
pub struct EditBuffer {
    /// The spans, normalized.
    spans: Vec<SpanData>,
    /// What the owner hands its spans: what a relative size op
    /// ([`TextOp::GrowSize`], [`TextOp::ScaleSize`]) reads, as the
    /// document's own walk would hand it.
    inherited: DocInherited,
}

impl EditBuffer {
    /// The spans of `owner` — a section, a ring, a cap, a label, a portal
    /// end, a connector or a line link — as a buffer, normalized.
    ///
    /// Errors: [`MutationError::NotA`] when `owner` is not an element
    /// that holds spans by the shape rules (a node, a frame, a portal
    /// link, or an id the document does not hold).
    ///
    /// Cost: O(the owner's children and their text), and the inherited
    /// state's fold (`inherited_seed`, at most five elements).
    pub fn from_owner(doc: &Document, owner: NodeId) -> Result<Self, MutationError> {
        let element = match doc.get(owner) {
            Some(element) if cardinality(element, false, DocKind::Span).is_some() => element,
            _ => {
                return Err(MutationError::NotA {
                    element: owner,
                    expected: "an element that holds spans",
                })
            }
        };
        let spans = doc
            .children(owner)
            .filter_map(|id| match doc.get(id) {
                Some(DocElement::Span(span)) => Some(span.clone()),
                _ => None,
            })
            .collect();
        let inherited = inherit(element, &inherited_seed(doc.tree(), owner));
        let mut buffer = EditBuffer { spans, inherited };
        buffer.normalize();
        Ok(buffer)
    }

    /// The buffer's text: every span's, in order. O(the text).
    pub fn text(&self) -> String {
        self.spans.iter().map(|span| span.text.as_str()).collect()
    }

    /// How many grapheme clusters the buffer holds, counted span by span.
    /// O(the text).
    pub fn grapheme_len(&self) -> usize {
        self.spans
            .iter()
            .map(|span| count_grapheme_clusters(&span.text))
            .sum()
    }

    /// The spans, normalized. O(1).
    pub fn spans(&self) -> &[SpanData] {
        &self.spans
    }

    /// Insert `text` before grapheme cluster `at`: into the span that
    /// holds that cluster, or — at a span edge — at the end of the span
    /// before it, so text typed after a span's last cluster takes its
    /// style; at or past the end, at the end of the last span; into a new
    /// span with nothing set when the buffer holds none.
    ///
    /// Returns how many clusters the caret moves past what was inserted:
    /// a combining mark that merges into the cluster before it moves it
    /// by none, though the text changed (`main`'s IME primitive,
    /// `insert_str_at_grapheme_counted`). Nothing for empty `text`.
    ///
    /// Cost: O(the text).
    pub fn insert(&mut self, at: usize, text: &str) -> usize {
        if text.is_empty() {
            return 0;
        }
        let mut start = 0;
        let mut target = None;
        for (index, span) in self.spans.iter().enumerate() {
            let end = start + count_grapheme_clusters(&span.text);
            if start <= at && at <= end {
                target = Some((index, start));
                break;
            }
            start = end;
        }
        let (index, local) = match target {
            Some((index, start)) => (index, at - start),
            None => match self.spans.len().checked_sub(1) {
                Some(last) => (last, count_grapheme_clusters(&self.spans[last].text)),
                None => {
                    self.spans.push(SpanData {
                        text: String::new(),
                        ..SpanData::default()
                    });
                    (0, 0)
                }
            },
        };
        let advance = insert_str_at_grapheme_counted(&mut self.spans[index].text, local, text);
        self.normalize();
        advance
    }

    /// Delete grapheme clusters `start..end`. A span the range empties is
    /// removed, and the two parts of a span the range was inside are
    /// joined again. Nothing for an empty range; a range past the end
    /// deletes to the end.
    ///
    /// Cost: O(the text).
    pub fn delete(&mut self, start: usize, end: usize) {
        if start >= end {
            return;
        }
        let mut at = 0;
        for span in &mut self.spans {
            let len = count_grapheme_clusters(&span.text);
            let (from, to) = (start.max(at), end.min(at + len));
            if from < to {
                delete_grapheme_range(&mut span.text, from - at, to - at);
            }
            at += len;
        }
        self.normalize();
    }

    /// Apply `op` to the style of grapheme clusters `start..end`: the
    /// spans are split at both boundaries, `op` applied to every span
    /// inside the range — a relative size op against the size the owner
    /// hands down, as the document's walk would apply it — and the buffer
    /// normalized. Nothing for an empty range.
    ///
    /// Cost: O(the text).
    pub fn style(&mut self, start: usize, end: usize, op: &TextOp) {
        if start >= end {
            return;
        }
        let mut out = Vec::with_capacity(self.spans.len() + 2);
        let mut at = 0;
        for span in self.spans.drain(..) {
            let len = count_grapheme_clusters(&span.text);
            let (from, to) = (start.clamp(at, at + len) - at, end.clamp(at, at + len) - at);
            at += len;
            if from == to {
                out.push(span);
                continue;
            }
            let mut text = span.text;
            let after = crate::util::grapheme_chad::split_off_graphemes(&mut text, to);
            let inside = crate::util::grapheme_chad::split_off_graphemes(&mut text, from);
            let mut styled = SpanData {
                text: inside,
                style: span.style.clone(),
                link: span.link.clone(),
            };
            apply_text(op, &mut styled.style, DocKind::Span, &self.inherited);
            for piece in [
                SpanData {
                    text,
                    style: span.style.clone(),
                    link: span.link.clone(),
                },
                styled,
                SpanData {
                    text: after,
                    style: span.style,
                    link: span.link,
                },
            ] {
                out.push(piece);
            }
        }
        self.spans = out;
        self.normalize();
    }

    /// Drop every empty span and merge every pair of neighbors that share
    /// their style and their link — the span builders' normalization,
    /// in one pass. O(the spans and their text).
    fn normalize(&mut self) {
        let mut out: Vec<SpanData> = Vec::with_capacity(self.spans.len());
        for span in self.spans.drain(..) {
            if span.text.is_empty() {
                continue;
            }
            match out.last_mut() {
                Some(last) if last.style == span.style && last.link == span.link => {
                    last.text.push_str(&span.text);
                }
                _ => out.push(span),
            }
        }
        self.spans = out;
    }
}
