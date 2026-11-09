// SPDX-License-Identifier: MPL-2.0

//! A section's fields and structure: where it sits in its node, its
//! minimum size (D5), its alignment, its text as one span, and adding,
//! deleting and splitting sections.
//!
//! **Which sections.** The selected sections, when any is selected;
//! otherwise, for each selected node, its section `index` when one is
//! given (`section=<idx>`), or its one section when it has exactly one.
//! A node with several and no index is refused, naming the key that
//! picks one.

use baumhard::arena::NodeId;
use baumhard::document::element::{DocKind, SpanData};
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, SectionOp};
use baumhard::document::values::Align;
use glam::Vec2;

use super::{selected, Steps};
use crate::application::document::MindMapDocument;
use crate::application::view::inspect::children_of_kind;

/// The sections a section edit reaches. See the module docs.
pub(crate) fn targets(doc: &MindMapDocument, index: Option<usize>) -> Result<Vec<NodeId>, String> {
    let sections = selected(doc, DocKind::Section);
    if !sections.is_empty() {
        return Ok(sections);
    }
    let d = doc.doc();
    let nodes = selected(doc, DocKind::Node);
    if nodes.is_empty() {
        return Err("section: select a section, or a node".to_string());
    }
    let mut out = Vec::with_capacity(nodes.len());
    for node in nodes {
        let mut of_node = children_of_kind(d, node, DocKind::Section);
        let section = match index {
            Some(index) => of_node.nth(index),
            None => match (of_node.next(), of_node.next()) {
                (Some(only), None) => Some(only),
                (Some(_), Some(_)) => {
                    return Err(
                        "section: the node has several sections; pick one with section=<idx>".to_string(),
                    )
                }
                (None, _) => None,
            },
        };
        match section {
            Some(section) => out.push(section),
            None => return Err(format!("section: the node has no section {}", index.unwrap_or(0))),
        }
    }
    Ok(out)
}

/// The one section a single-section edit reaches (`split`), refused when
/// the selection reaches several.
fn one(doc: &MindMapDocument, index: Option<usize>) -> Result<NodeId, String> {
    match targets(doc, index)?.as_slice() {
        [section] => Ok(*section),
        _ => Err("section: select one section".to_string()),
    }
}

/// What a section edit sets on each section it reaches.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Field {
    /// Move it by a delta.
    Nudge(Vec2),
    /// Place it at an offset in its node.
    Offset(Vec2),
    /// Its minimum size; `None` fits its content.
    MinSize(Option<Vec2>),
    /// Its alignment; `None` inherits.
    Align(Option<Align>),
}

/// The steps that set `field` on every section reached.
pub(crate) fn set(doc: &MindMapDocument, index: Option<usize>, field: Field) -> Steps {
    let op = match field {
        Field::Nudge(delta) => SectionOp::Nudge(delta),
        Field::Offset(offset) => SectionOp::SetOffset(offset),
        Field::MinSize(size) => SectionOp::SetMinSize(size),
        Field::Align(align) => SectionOp::SetAlign(align),
    };
    compose::set(
        doc.doc(),
        DocKind::Section,
        vec![DocMutation::Section(op)],
        &targets(doc, index)?,
    )
    .map_err(|e| e.to_string())
}

/// One unstyled span holding `text`, or none for empty text: what a
/// section's or a ring's text becomes when it is set as plain text.
pub(crate) fn plain(text: &str) -> Vec<SpanData> {
    if text.is_empty() {
        return Vec::new();
    }
    vec![SpanData {
        text: text.to_string(),
        ..SpanData::default()
    }]
}

/// The steps that replace every reached section's spans with one span
/// holding `text`.
pub(crate) fn text(doc: &MindMapDocument, index: Option<usize>, text: &str) -> Steps {
    compose::replace_spans(doc.doc(), &targets(doc, index)?, &plain(text)).map_err(|e| e.to_string())
}

/// The steps that add a section holding `text` to the one selected node
/// (or a selected section's node), as its section `at`, or after the
/// last when `at` is `None`.
pub(crate) fn add(doc: &MindMapDocument, at: Option<usize>, text: &str) -> Steps {
    let d = doc.doc();
    let node = match (
        selected(doc, DocKind::Node).as_slice(),
        selected(doc, DocKind::Section).as_slice(),
    ) {
        ([node], []) => *node,
        ([], [section]) => d.parent(*section).ok_or("section add: the section has no node")?,
        _ => return Err("section add: select one node".to_string()),
    };
    let sections: Vec<NodeId> = children_of_kind(d, node, DocKind::Section).collect();
    let after = match at {
        Some(0) => None,
        Some(at) if at <= sections.len() => Some(sections[at - 1]),
        Some(at) => {
            return Err(format!(
                "section add: at={at} is past the node's {} sections",
                sections.len()
            ))
        }
        None => sections.last().copied(),
    };
    compose::add_section(d, node, after, &plain(text)).map_err(|e| e.to_string())
}

/// The steps that delete every reached section, as `DeleteSelection`
/// deletes a section (`compose::remove`, V1-C4).
pub(crate) fn delete(doc: &MindMapDocument, index: Option<usize>) -> Steps {
    compose::remove(doc.doc(), DocKind::Section, &targets(doc, index)?).map_err(|e| e.to_string())
}

/// The steps that apply `ops` to grapheme clusters `start..end` of every
/// reached section's text: its spans split at both ends, the ones inside
/// styled, then normalized (`compose::style_range`), once per op.
pub(crate) fn style_range(
    doc: &MindMapDocument,
    start: usize,
    end: usize,
    ops: &[baumhard::document::mutation::ops::TextOp],
) -> Steps {
    if start >= end {
        return Err(format!("range: {start}..{end} holds no grapheme"));
    }
    let d = doc.doc();
    let mut steps = Vec::new();
    for section in targets(doc, None)? {
        for op in ops {
            steps
                .extend(compose::style_range(d, section, start, end, op.clone()).map_err(|e| e.to_string())?);
        }
    }
    Ok(steps)
}

/// The steps that split the one reached section before grapheme cluster
/// `at`.
pub(crate) fn split(doc: &MindMapDocument, index: Option<usize>, at: usize) -> Steps {
    compose::split_section(doc.doc(), one(doc, index)?, at).map_err(|e| e.to_string())
}
