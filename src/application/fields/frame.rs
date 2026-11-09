// SPDX-License-Identifier: MPL-2.0

//! The frames of the selected nodes and their rings (design §6, D8,
//! D10, D25): a frame's fields, showing and hiding it, its rings and
//! their text, the cycle through the box-drawing glyphs, and the frame
//! preview, a transaction (§9).
//!
//! **Box-drawing glyphs are ring content** (D8): there are no presets.
//! A node with no frame that is given one — by `frame on`, by
//! [`toggle`], by [`cycle`], by a ring added to it — gets a frame of one
//! ring holding [`CYCLE`]'s first glyph, repeated around the outline
//! (`fit: repeat`), unless the ring added brings its own text.
//!
//! **Rings are counted in document order**, innermost first: `frame
//! ring 0` is the first ring of each selected node's frame.

use baumhard::arena::NodeId;
use baumhard::document::element::{
    DocElement, DocKind, FrameData, FrameStyle, RingData, RingStyle, TextStyle,
};
use baumhard::document::ids::Document;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, FrameOp, RingOp, TextOp};
use baumhard::document::resolve::resolve_frame;
use baumhard::document::values::{Direction, FitMode, FontSize, StartPoint};
use baumhard::engine::structure::Subtree;

use super::section::plain;
use super::style::frame_of;
use super::{selected, values, Groups, Steps};
use crate::application::document::{Interaction, MindMapDocument};
use crate::application::view::inspect::children_of_kind;

/// The glyphs [`cycle`] steps a frame's first ring through, in order,
/// wrapping: light, heavy, double.
pub(crate) const CYCLE: [&str; 3] = ["─", "━", "═"];

/// One frame field, as `frame <key>=<value>` and `SetFrameField` name
/// it; `None` inside a field inherits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FrameField {
    Start(Option<StartPoint>),
    Direction(Option<Direction>),
    Fit(Option<FitMode>),
    Gap(Option<f32>),
    Inset(Option<f32>),
}

/// The frame field keys, in the order help lists them.
pub(crate) const FIELD_KEYS: [&str; 5] = ["start", "direction", "fit", "gap", "inset"];

impl FrameField {
    /// The field `key` names, holding `value` read in its spelling
    /// ([`values`]), or `reset` to inherit.
    pub(crate) fn read(key: &str, value: &str) -> Result<FrameField, String> {
        use values::or_reset;
        let field = match key {
            "start" => FrameField::Start(or_reset(value, values::start)?),
            "direction" => FrameField::Direction(or_reset(value, values::direction)?),
            "fit" => FrameField::Fit(or_reset(value, values::fit)?),
            "gap" => FrameField::Gap(or_reset(value, values::number)?),
            "inset" => FrameField::Inset(or_reset(value, values::number)?),
            other => {
                return Err(format!(
                    "'{other}' is not a frame field: {}",
                    FIELD_KEYS.join(", ")
                ))
            }
        };
        Ok(field)
    }

    /// The op that writes it.
    fn op(&self) -> FrameOp {
        match self {
            FrameField::Start(start) => FrameOp::SetStart(*start),
            FrameField::Direction(direction) => FrameOp::SetDirection(*direction),
            FrameField::Fit(fit) => FrameOp::SetFit(fit.clone()),
            FrameField::Gap(gap) => FrameOp::SetGap(*gap),
            FrameField::Inset(inset) => FrameOp::SetInset(*inset),
        }
    }
}

/// The selected nodes, or the refusal that says to select one.
fn nodes(doc: &MindMapDocument) -> Result<Vec<NodeId>, String> {
    let nodes = selected(doc, DocKind::Node);
    if nodes.is_empty() {
        return Err("frame: select a node".to_string());
    }
    Ok(nodes)
}

/// The selected nodes' frames, refused when none has one.
fn frames(doc: &MindMapDocument) -> Result<Vec<NodeId>, String> {
    let d = doc.doc();
    let frames: Vec<NodeId> = nodes(doc)?
        .into_iter()
        .filter_map(|node| frame_of(d, node))
        .collect();
    if frames.is_empty() {
        return Err("frame: no selected node has a frame; `frame on` gives it one".to_string());
    }
    Ok(frames)
}

/// A ring holding `spans`, its size `size` (a fraction of the node's
/// shorter side) when given, on `channel`.
fn ring(
    spans: Vec<baumhard::document::element::SpanData>,
    size: Option<f32>,
    channel: usize,
) -> Subtree<DocElement> {
    Subtree {
        root: DocElement::Ring(RingData {
            channel,
            style: RingStyle {
                text: TextStyle {
                    size: size.map(FontSize::Rel),
                    ..TextStyle::default()
                },
                ..RingStyle::default()
            },
        }),
        children: spans
            .into_iter()
            .map(|span| Subtree::leaf(DocElement::Span(span)))
            .collect(),
    }
}

/// A frame holding `ring`; with [`CYCLE`]'s first glyph repeated when
/// `ring` is `None`.
fn new_frame(ring_given: Option<Subtree<DocElement>>) -> Subtree<DocElement> {
    let (fit, ring_tree) = match ring_given {
        Some(ring_tree) => (None, ring_tree),
        None => (Some(FitMode::Repeat), ring(plain(CYCLE[0]), None, 0)),
    };
    Subtree {
        root: DocElement::Frame(FrameData {
            style: FrameStyle {
                fit,
                ..FrameStyle::default()
            },
        }),
        children: vec![ring_tree],
    }
}

/// The steps that give every one of `bare`, nodes with no frame, a new
/// frame ([`new_frame`]).
fn frame_the(doc: &Document, bare: &[NodeId], ring_given: Option<Subtree<DocElement>>) -> Steps {
    compose::insert(doc, DocKind::Node, bare, new_frame(ring_given)).map_err(|e| e.to_string())
}

/// The selected nodes split into their frames and the nodes with none.
fn split(doc: &MindMapDocument) -> Result<(Vec<NodeId>, Vec<NodeId>), String> {
    let d = doc.doc();
    let mut framed = Vec::new();
    let mut bare = Vec::new();
    for node in nodes(doc)? {
        match frame_of(d, node) {
            Some(frame) => framed.push(frame),
            None => bare.push(node),
        }
    }
    Ok((framed, bare))
}

/// The steps that set `fields` on every selected node's frame.
pub(crate) fn set(doc: &MindMapDocument, fields: &[FrameField]) -> Steps {
    let d = doc.doc();
    let ops: Vec<DocMutation> = fields.iter().map(|f| DocMutation::Frame(f.op())).collect();
    compose::set(d, DocKind::Frame, ops, &frames(doc)?).map_err(|e| e.to_string())
}

/// The steps that show (`on`) or hide every selected node's frame; a
/// node with no frame is given one when shown.
pub(crate) fn visible(doc: &MindMapDocument, on: bool) -> Steps {
    let (framed, bare) = split(doc)?;
    let d = doc.doc();
    let mut steps = compose::set(
        d,
        DocKind::Frame,
        vec![DocMutation::Frame(FrameOp::SetVisible(Some(on)))],
        &framed,
    )
    .map_err(|e| e.to_string())?;
    if on {
        steps.extend(frame_the(d, &bare, None)?);
    }
    Ok(steps)
}

/// The steps that show each selected node's frame that is hidden and
/// hide each that is shown — as resolved, so a frame the defaults hide
/// is shown — and give a node with no frame one.
pub(crate) fn toggle(doc: &MindMapDocument) -> Steps {
    let (framed, bare) = split(doc)?;
    let d = doc.doc();
    let mut groups = Groups::default();
    for frame in framed {
        let shown = resolve_frame(d.tree(), frame, &mut ()).visible;
        groups.add(
            d,
            frame,
            vec![DocMutation::Frame(FrameOp::SetVisible(Some(!shown)))],
        );
    }
    let mut steps = groups.steps(d)?;
    steps.extend(frame_the(d, &bare, None)?);
    Ok(steps)
}

/// The steps that replace the spans of each selected frame's first ring
/// with one span of the next of [`CYCLE`] after its text — the first
/// when its text is none of them — and give a frame with no ring, and a
/// node with no frame, one ring of the first.
pub(crate) fn cycle(doc: &MindMapDocument) -> Steps {
    let (framed, bare) = split(doc)?;
    let d = doc.doc();
    let mut by_glyph: Vec<(&str, Vec<NodeId>)> = CYCLE.iter().map(|&glyph| (glyph, Vec::new())).collect();
    let mut ringless = Vec::new();
    for frame in framed {
        match children_of_kind(d, frame, DocKind::Ring).next() {
            Some(first) => {
                let text = crate::application::view::inspect::element_text(d, first);
                let next = CYCLE
                    .iter()
                    .position(|&glyph| glyph == text)
                    .map_or(0, |at| (at + 1) % CYCLE.len());
                by_glyph[next].1.push(first);
            }
            None => ringless.push(frame),
        }
    }
    let mut steps = Vec::new();
    for (glyph, rings) in by_glyph {
        steps.extend(compose::replace_spans(d, &rings, &plain(glyph)).map_err(|e| e.to_string())?);
    }
    steps.extend(
        compose::insert(d, DocKind::Frame, &ringless, ring(plain(CYCLE[0]), None, 0))
            .map_err(|e| e.to_string())?,
    );
    steps.extend(frame_the(d, &bare, None)?);
    Ok(steps)
}

/// The steps that add a ring holding `text` (one span; none when
/// `None`), of relative size `size`, after the rings of each selected
/// node's frame; a node with no frame is given one holding that ring.
pub(crate) fn add_ring(doc: &MindMapDocument, text: Option<&str>, size: Option<f32>) -> Steps {
    let (framed, bare) = split(doc)?;
    let d = doc.doc();
    let spans = text.map(plain).unwrap_or_default();
    let channel = framed
        .iter()
        .flat_map(|&frame| children_of_kind(d, frame, DocKind::Ring))
        .filter_map(|ring| match d.get(ring) {
            Some(DocElement::Ring(ring)) => Some(ring.channel),
            _ => None,
        })
        .max()
        .unwrap_or(0);
    let mut steps = compose::insert(d, DocKind::Frame, &framed, ring(spans.clone(), size, channel))
        .map_err(|e| e.to_string())?;
    steps.extend(frame_the(d, &bare, Some(ring(spans, size, 0)))?);
    Ok(steps)
}

/// Ring `index` of each selected node's frame, refused when one has no
/// such ring.
fn rings_at(doc: &MindMapDocument, index: usize) -> Result<Vec<NodeId>, String> {
    let d = doc.doc();
    frames(doc)?
        .into_iter()
        .map(|frame| {
            children_of_kind(d, frame, DocKind::Ring)
                .nth(index)
                .ok_or_else(|| format!("frame ring {index}: a selected node's frame has no ring {index}"))
        })
        .collect()
}

/// What `frame ring <i>` sets on the ring: each given.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct RingFields {
    /// Its text, as one span.
    pub text: Option<String>,
    /// Its size, a fraction of the node's shorter side; `None` inside
    /// inherits.
    pub size: Option<Option<f32>>,
    /// Its fit, over the frame's; `None` inside inherits.
    pub fit: Option<Option<FitMode>>,
}

/// The steps that set `fields` on ring `index` of each selected node's
/// frame.
pub(crate) fn set_ring(doc: &MindMapDocument, index: usize, fields: &RingFields) -> Steps {
    let rings = rings_at(doc, index)?;
    let d = doc.doc();
    let mut steps = match &fields.text {
        Some(text) => compose::replace_spans(d, &rings, &plain(text)).map_err(|e| e.to_string())?,
        None => Vec::new(),
    };
    let mut ops = Vec::new();
    ops.extend(
        fields
            .size
            .map(|size| RingOp::Text(TextOp::SetSize(size.map(FontSize::Rel)))),
    );
    ops.extend(fields.fit.clone().map(RingOp::SetFit));
    let ops = ops.into_iter().map(DocMutation::Ring).collect();
    steps.extend(compose::set(d, DocKind::Ring, ops, &rings).map_err(|e| e.to_string())?);
    if steps.is_empty() {
        return Err(format!("frame ring {index}: give text=, size=, fit= or delete"));
    }
    Ok(steps)
}

/// The steps that delete ring `index` of each selected node's frame.
pub(crate) fn delete_ring(doc: &MindMapDocument, index: usize) -> Steps {
    compose::remove(doc.doc(), DocKind::Ring, &rings_at(doc, index)?).map_err(|e| e.to_string())
}

/// The label of a frame preview's stage and of the entry its commit makes.
pub(crate) const PREVIEW_LABEL: &str = "frame preview";

/// Stage `fields` on the selected nodes' frames as the preview: opening
/// the preview's transaction first when none is open (D28: refused
/// while another interaction holds the document). A second preview
/// replaces the first, as a stage replaces a stage.
pub(crate) fn preview(
    doc: &mut MindMapDocument,
    fields: &[FrameField],
    source: baumhard::document::writer::ChangeSource,
) -> Result<(), super::Refused> {
    use baumhard::document::writer::transaction::TransactionKind;
    use baumhard::document::writer::EditRequest;
    let steps = set(doc, fields).map_err(super::Refused::Because)?;
    let began = match doc.open_interaction() {
        None if doc.begin(source, TransactionKind::Interactive, Interaction::FramePreview) => true,
        Some(Interaction::FramePreview) => false,
        _ => return Err(super::Refused::Busy),
    };
    let staged = doc.stage(Interaction::FramePreview, EditRequest::new(steps, PREVIEW_LABEL));
    if let Err(error) = staged {
        // A preview that never staged leaves nothing to commit.
        if began {
            doc.cancel(Interaction::FramePreview);
        }
        return Err(super::Refused::Because(format!("{PREVIEW_LABEL}: {error}")));
    }
    Ok(())
}
