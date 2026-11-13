// SPDX-License-Identifier: MPL-2.0

//! The frames of the selected nodes and their rings (design §6, D8,
//! D10, D25): a frame's fields, showing and hiding it, its rings and
//! their text, the cycle through the box-drawing glyphs, and the frame
//! preview, a transaction (§9).
//!
//! **Box-drawing glyphs are ring content** (D8): there are no presets.
//! What [`cycle`] sets is a ring's ordinary keys — its spans, its
//! `vertical` and its `corners` (#324, #338) — which the map saves and
//! the console edits like any other. A node with no frame that is given
//! one — by `frame on`, by [`toggle`], by [`cycle`] — gets a frame of
//! one ring of [`CYCLE`]'s first set, the light box, repeated around the
//! outline (`fit: repeat`); a ring added to it (`frame ring add`) brings
//! its own text instead.
//!
//! **Rings are counted in document order**, innermost first: `frame
//! ring 0` is the first ring of each selected node's frame.

use baumhard::arena::NodeId;
use baumhard::document::element::{
    DirectionalGlyphs, DocElement, DocKind, FrameData, FrameStyle, RingData, RingStyle, SpanData, TextStyle,
};
use baumhard::document::ids::Document;
use baumhard::document::mutation::compose;
use baumhard::document::mutation::ops::{DocMutation, FrameOp, LayoutOp, RingOp, TextOp};
use baumhard::document::resolve::resolve_frame;
use baumhard::document::values::{ColorRef, Direction, FitMode, FontSize, LineSpan, Sides, StartPoint};
use baumhard::engine::structure::Subtree;

use super::section::plain;
use super::style::frame_of;
use super::{selected, values, Groups, Steps};
use crate::application::document::{Interaction, MindMapDocument};
use crate::application::keybinds::FramePreviewTarget;
use crate::application::view::inspect::children_of_kind;

/// One set of box-drawing glyphs [`cycle`] gives a ring (#338): its
/// spans, laid where the outline runs level, its `vertical`, laid where
/// it runs steep, and its four `corners`, clockwise from the top left,
/// laid where it turns as a box does (`format/schema.md` "Directional
/// ring"). Round a rectangle a ring of one set reads as a closed box.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BoxGlyphs {
    /// The text of its one span.
    pub level: &'static str,
    /// Its `vertical` text.
    pub vertical: &'static str,
    /// Its `corners`: top left, top right, bottom right, bottom left.
    pub corners: [&'static str; 4],
}

impl BoxGlyphs {
    /// Its spans: one, holding [`level`](Self::level).
    fn spans(&self) -> Vec<SpanData> {
        plain(self.level)
    }

    /// Its `vertical` and `corners`, as a ring holds them.
    fn directional(&self) -> DirectionalGlyphs {
        DirectionalGlyphs {
            vertical: Some(self.vertical.to_string()),
            corners: Some(self.corners.map(str::to_string)),
        }
    }

    /// Whether a ring whose text is `text` and whose directional keys
    /// are `directional` is exactly this set: all three keys, and
    /// nothing else in them.
    fn is(&self, text: &str, directional: Option<&DirectionalGlyphs>) -> bool {
        text == self.level && directional == Some(&self.directional())
    }
}

/// The ops that set a ring's `vertical` and `corners` to `set`'s, or
/// clear both when `set` is `None`: the two keys always move together,
/// and the corners all four or none.
fn directional_ops(set: Option<&BoxGlyphs>) -> Vec<DocMutation> {
    let directional = set.map(BoxGlyphs::directional).unwrap_or_default();
    vec![
        DocMutation::Ring(RingOp::SetVertical(directional.vertical)),
        DocMutation::Ring(RingOp::SetCorners(directional.corners)),
    ]
}

/// The sets [`cycle`] steps a frame's first ring through, in order,
/// wrapping: light, heavy, double. Every glyph of them is drawn by
/// `Mandala Ornaments` on every target (`scripts/ornament_fonts.sh`).
pub(crate) const CYCLE: [BoxGlyphs; 3] = [
    BoxGlyphs {
        level: "─",
        vertical: "│",
        corners: ["┌", "┐", "┘", "└"],
    },
    BoxGlyphs {
        level: "━",
        vertical: "┃",
        corners: ["┏", "┓", "┛", "┗"],
    },
    BoxGlyphs {
        level: "═",
        vertical: "║",
        corners: ["╔", "╗", "╝", "╚"],
    },
];

/// The set of [`CYCLE`] that `ring` is exactly ([`BoxGlyphs::is`]), if
/// any: a ring the cycle made, or one spelled as it would make it.
fn set_of(doc: &Document, ring: NodeId) -> Option<&'static BoxGlyphs> {
    let Some(DocElement::Ring(data)) = doc.get(ring) else {
        return None;
    };
    let text = crate::application::view::inspect::element_text(doc, ring);
    CYCLE
        .iter()
        .find(|set| set.is(&text, data.directional.as_deref()))
}

/// One frame field, as `frame <key>=<value>` and `SetFrameField` name
/// it; `None` inside a field inherits.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum FrameField {
    Start(Option<StartPoint>),
    Direction(Option<Direction>),
    Fit(Option<FitMode>),
    Gap(Option<f32>),
    Inset(Option<f32>),
    /// How much of its half each line of a two-line ring covers (#291).
    Span(Option<LineSpan>),
    /// The ornament in the breaks between a ring's lines (#291).
    Sides(Option<Sides>),
    /// The space a `repeat` ring adds between glyph steps (#291).
    Spacing(Option<f32>),
}

/// The frame field keys, in the order help lists them: the frame's own,
/// then how its rings are laid (#291), which they inherit.
pub(crate) const FIELD_KEYS: [&str; 8] = [
    "start",
    "direction",
    "fit",
    "gap",
    "inset",
    "span",
    "sides",
    "spacing",
];

impl FrameField {
    /// The field `key` names, holding `value` read in its spelling
    /// ([`values`]) — a color in `sides` as `doc` reads one — or `reset`
    /// to inherit.
    pub(crate) fn read(doc: &Document, key: &str, value: &str) -> Result<FrameField, String> {
        use values::or_reset;
        let field = match key {
            "span" => FrameField::Span(or_reset(value, values::line_span)?),
            "sides" => FrameField::Sides(or_reset(value, |text| values::sides(doc, text))?),
            "spacing" => FrameField::Spacing(or_reset(value, values::number)?),
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

    /// Write it into `style`: the map's default frame's
    /// (`defaults frame.<key>=`, and a `defaults` preview).
    pub(crate) fn write(self, style: &mut FrameStyle) {
        match self {
            FrameField::Start(start) => style.start = start,
            FrameField::Direction(direction) => style.direction = direction,
            FrameField::Fit(fit) => style.fit = fit,
            FrameField::Gap(gap) => style.gap = gap,
            FrameField::Inset(inset) => style.inset = inset,
            FrameField::Span(span) => style.ring_layout.span = span,
            FrameField::Sides(sides) => style.ring_layout.sides = sides,
            FrameField::Spacing(spacing) => style.ring_layout.spacing = spacing,
        }
    }

    /// The op that writes it on a frame.
    fn op(&self) -> FrameOp {
        match self {
            FrameField::Start(start) => FrameOp::SetStart(*start),
            FrameField::Direction(direction) => FrameOp::SetDirection(*direction),
            FrameField::Fit(fit) => FrameOp::SetFit(fit.clone()),
            FrameField::Gap(gap) => FrameOp::SetGap(*gap),
            FrameField::Inset(inset) => FrameOp::SetInset(*inset),
            FrameField::Span(span) => FrameOp::Layout(LayoutOp::SetSpan(*span)),
            FrameField::Sides(sides) => FrameOp::Layout(LayoutOp::SetSides(sides.clone())),
            FrameField::Spacing(spacing) => FrameOp::Layout(LayoutOp::SetSpacing(*spacing)),
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
            ..RingData::default()
        }),
        children: spans
            .into_iter()
            .map(|span| Subtree::leaf(DocElement::Span(span)))
            .collect(),
    }
}

/// A ring of `set`, on channel 0: its spans, `vertical` and `corners`.
fn box_ring(set: &BoxGlyphs) -> Subtree<DocElement> {
    let mut tree = ring(set.spans(), None, 0);
    if let DocElement::Ring(data) = &mut tree.root {
        data.directional = Some(Box::new(set.directional()));
    }
    tree
}

/// A frame holding `ring`; with a ring of [`CYCLE`]'s first set
/// repeated when `ring` is `None`.
fn new_frame(ring_given: Option<Subtree<DocElement>>) -> Subtree<DocElement> {
    let (fit, ring_tree) = match ring_given {
        Some(ring_tree) => (None, ring_tree),
        None => (Some(FitMode::Repeat), box_ring(&CYCLE[0])),
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
        groups.add(d, frame, !shown, || {
            vec![DocMutation::Frame(FrameOp::SetVisible(Some(!shown)))]
        });
    }
    let mut steps = groups.steps(d)?;
    steps.extend(frame_the(d, &bare, None)?);
    Ok(steps)
}

/// The steps that make each selected frame's first ring the next set of
/// [`CYCLE`] after the one its text is the level glyph of — the first
/// when its text is none of them — and give a frame with no ring, and a
/// node with no frame, one ring of the first. The ring takes the whole
/// set: its spans, its `vertical` and its four `corners`, replacing
/// whatever it held in them, an author's own included, so no key of the
/// set it leaves is left behind (#338). Its other keys are kept.
pub(crate) fn cycle(doc: &MindMapDocument) -> Steps {
    let (framed, bare) = split(doc)?;
    let d = doc.doc();
    let mut by_set: Vec<(&BoxGlyphs, Vec<NodeId>)> = CYCLE.iter().map(|set| (set, Vec::new())).collect();
    let mut ringless = Vec::new();
    for frame in framed {
        match children_of_kind(d, frame, DocKind::Ring).next() {
            Some(first) => {
                let text = crate::application::view::inspect::element_text(d, first);
                let next = CYCLE
                    .iter()
                    .position(|set| set.level == text)
                    .map_or(0, |at| (at + 1) % CYCLE.len());
                by_set[next].1.push(first);
            }
            None => ringless.push(frame),
        }
    }
    let mut steps = Vec::new();
    for (set, rings) in by_set {
        steps.extend(compose::replace_spans(d, &rings, &set.spans()).map_err(|e| e.to_string())?);
        steps.extend(
            compose::set(d, DocKind::Ring, directional_ops(Some(set)), &rings).map_err(|e| e.to_string())?,
        );
    }
    steps.extend(
        compose::insert(d, DocKind::Frame, &ringless, box_ring(&CYCLE[0])).map_err(|e| e.to_string())?,
    );
    steps.extend(frame_the(d, &bare, None)?);
    Ok(steps)
}

/// The steps that add a ring holding `text` (one span; none when
/// `None`), of relative size `size`, after the rings of each selected
/// node's frame, on that frame's last ring's channel, so the file a
/// frame saves to does not depend on what was selected beside it; a node
/// with no frame is given one holding that ring. One insert per channel
/// the frames' last rings are on.
pub(crate) fn add_ring(doc: &MindMapDocument, text: Option<&str>, size: Option<f32>) -> Steps {
    let (framed, bare) = split(doc)?;
    let d = doc.doc();
    let spans = text.map(plain).unwrap_or_default();
    let mut by_channel = Groups::default();
    for frame in framed {
        let channel = children_of_kind(d, frame, DocKind::Ring)
            .last()
            .and_then(|ring| match d.get(ring) {
                Some(DocElement::Ring(ring)) => Some(ring.channel),
                _ => None,
            })
            .unwrap_or(0);
        by_channel.add(d, frame, channel, Vec::new);
    }
    let mut steps = Vec::new();
    for (channel, frames) in by_channel.members() {
        let new = ring(spans.clone(), size, channel);
        steps.extend(compose::insert(d, DocKind::Frame, &frames, new).map_err(|e| e.to_string())?);
    }
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
    /// Its text color, its own; `None` inside inherits the frame's.
    pub color: Option<Option<ColorRef>>,
}

/// The steps that set `fields` on ring `index` of each selected node's
/// frame.
///
/// A new text moves a ring that is exactly a set of [`CYCLE`] off that
/// set (#338): its `vertical` and `corners` were the set's, not the
/// author's, so they follow the text — to the set the text is the level
/// glyph of, or cleared when it is none — rather than staying behind
/// round a text they no longer belong to. A ring that is not exactly a
/// set keeps its `vertical` and `corners` as they are.
pub(crate) fn set_ring(doc: &MindMapDocument, index: usize, fields: &RingFields) -> Steps {
    let rings = rings_at(doc, index)?;
    let d = doc.doc();
    let mut steps = match &fields.text {
        Some(text) => {
            let mut steps = compose::replace_spans(d, &rings, &plain(text)).map_err(|e| e.to_string())?;
            let boxed: Vec<NodeId> = rings
                .iter()
                .copied()
                .filter(|&ring| set_of(d, ring).is_some())
                .collect();
            let follows = CYCLE.iter().find(|set| set.level == text);
            steps.extend(
                compose::set(d, DocKind::Ring, directional_ops(follows), &boxed)
                    .map_err(|e| e.to_string())?,
            );
            steps
        }
        None => Vec::new(),
    };
    let mut ops = Vec::new();
    ops.extend(
        fields
            .size
            .map(|size| RingOp::Text(TextOp::SetSize(size.map(FontSize::Rel)))),
    );
    ops.extend(fields.fit.clone().map(RingOp::SetFit));
    ops.extend(fields.color.map(|color| RingOp::Text(TextOp::SetColor(color))));
    let ops = ops.into_iter().map(DocMutation::Ring).collect();
    steps.extend(compose::set(d, DocKind::Ring, ops, &rings).map_err(|e| e.to_string())?);
    if steps.is_empty() {
        return Err(format!(
            "frame ring {index}: give text=, size=, fit=, color= or delete"
        ));
    }
    Ok(steps)
}

/// The steps that delete ring `index` of each selected node's frame.
pub(crate) fn delete_ring(doc: &MindMapDocument, index: usize) -> Steps {
    compose::remove(doc.doc(), DocKind::Ring, &rings_at(doc, index)?).map_err(|e| e.to_string())
}

/// The label of a frame preview's stage and of the entry its commit makes.
pub(crate) const PREVIEW_LABEL: &str = "frame preview";

/// Stage `fields` — each a frame field's key and its value, read by
/// [`FrameField::read`] whichever the target, so both take the same
/// fields — on the frame `target` names, as the preview: the selected
/// nodes' frames, or the map's default frame, whose defaults are
/// replaced with those fields changed (`frame.<key>`,
/// [`FrameField::write`]). The preview's transaction
/// is opened first when none is open (D28: refused while another
/// interaction holds the document). A second preview replaces the
/// first, as a stage replaces a stage.
pub(crate) fn preview(
    doc: &mut MindMapDocument,
    target: FramePreviewTarget,
    fields: &[(&str, &str)],
    source: baumhard::document::writer::ChangeSource,
) -> Result<(), super::Refused> {
    use baumhard::document::writer::transaction::TransactionKind;
    use baumhard::document::writer::EditRequest;
    let steps = fields
        .iter()
        .map(|(key, value)| FrameField::read(doc.doc(), key, value))
        .collect::<Result<Vec<_>, _>>()
        .and_then(|fields| match target {
            FramePreviewTarget::Node => set(doc, &fields),
            FramePreviewTarget::Defaults => super::map::replace(doc, |defaults| {
                fields
                    .into_iter()
                    .for_each(|field| field.write(&mut defaults.frame));
                Ok(())
            }),
        })
        .map_err(super::Refused::Because)?;
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
