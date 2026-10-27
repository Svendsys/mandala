// SPDX-License-Identifier: MPL-2.0

//! Validation of document mutations, in two halves.
//!
//! **Before a walk**, [`validate`](crate::document::mutation::validate::validate)
//! judges a mutator tree against the document it is about to apply to,
//! from the mutator alone: every payload matches its mutator's kind,
//! every literal is in its bound ([`document::bounds`](crate::document::bounds)),
//! every reference — a verb, a palette, a theme variable or variant, a
//! palette group, a link's target — exists, and every structural command
//! is one the document allows. Nothing is applied when it refuses.
//! [`validate_for_presentation`](crate::document::mutation::validate::validate_for_presentation)
//! also refuses every structural command (D17).
//!
//! **After the steps of a request**,
//! [`validate_applied`](crate::document::mutation::validate::validate_applied)
//! judges what the walks did, from their journal: what only the walk
//! knows — which element a key reached — decides these. The rows the
//! structural commands changed and the nodes they moved or inserted
//! keep the shape rules, a link whose kind changed holds the children
//! its kind needs, an inserted link leads to another node, and every
//! text size the request wrote is in the unit its place takes, so an
//! edit never leaves a map a load would refuse. It runs once the last
//! step has run, because a request's intermediate states need not be
//! valid: a line link switched to a portal holds its first portal end
//! for one step before it holds both (ruling R10 on #237).

use crate::document::bounds::{
    check_canvas_coord, check_control_points, check_extent, check_fit, check_font_size_pt,
    check_non_negative, check_rel_size, check_unit_interval, check_zoom_window, BoundError,
};
use crate::document::element::{
    DefaultsData, DocElement, DocKind, FlowStyle, FrameStyle, LabelStyle, NodeStyle, PaletteGroup, PathData,
    PortalEndStyle, TextStyle,
};
use crate::document::ids::{owning_node, Document, NodeIdent};
use crate::document::mutation::error::{MutationError, MutatorPath, StructureRule};
use crate::document::mutation::ops::{
    DefaultsOp, DocMutation, DocMutatorTree, FrameOp, LabelOp, LinkOp, MapOp, NodeOp, PaletteOp, PathOp,
    PortalEndOp, RingOp, SectionOp, SpanOp, TextOp, ThemeOp,
};
use crate::document::shape::validate_shape;
use crate::document::values::{
    ColorRef, FitMode, FontSize, LinkKind, LinkTarget, PaletteId, PlainColor, StartPoint, VarId, VerbId,
    ZoomWindow,
};
use crate::engine::journal::{Journal, Touch};
use crate::engine::key::BranchKey;
use crate::engine::mutator::{Instruction, Mutator};
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tree::Tree;
use glam::Vec2;
use indextree::NodeId;
use rustc_hash::FxHashSet;

/// Judge `mutator` against `doc` before it is applied. See the module
/// docs for what is checked; the first refusal is returned, naming the
/// mutator by its path from the root.
///
/// Cost: one pass over the mutator tree, each literal checked once; an
/// inserted subtree is walked once more by the shape check. A palette
/// group index reads the palettes once. Allocates the path being walked
/// (as deep as the mutator tree).
pub fn validate(mutator: &DocMutatorTree, doc: &Document) -> Result<(), MutationError> {
    Validator::new(doc, false).run(mutator)
}

/// [`validate`], and refuse every structural command: a presentation
/// changes fields only (D17). Hiding a node and fading it are fields,
/// so a presentation can still conceal and reveal.
///
/// Cost: [`validate`]'s.
pub fn validate_for_presentation(mutator: &DocMutatorTree, doc: &Document) -> Result<(), MutationError> {
    Validator::new(doc, true).run(mutator)
}

/// What a text size may be where it sits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum Unit {
    /// Points only: everything outside a frame.
    Points,
    /// Relative only: a frame, a ring and a ring's spans.
    Relative,
    /// Either: a span whose place is not known before the walk —
    /// [`validate_applied`] judges it once it is.
    Either,
}

impl Unit {
    /// The unit an element of `kind` takes, when its own kind decides
    /// it.
    fn of(kind: DocKind) -> Unit {
        match kind {
            DocKind::Frame | DocKind::Ring => Unit::Relative,
            DocKind::Span => Unit::Either,
            _ => Unit::Points,
        }
    }

    /// Whether `size` is a unit this place takes.
    fn admits(self, size: FontSize) -> bool {
        matches!(
            (self, size),
            (Unit::Either, _) | (Unit::Points, FontSize::Pt(_)) | (Unit::Relative, FontSize::Rel(_))
        )
    }
}

/// The walk over a mutator tree, with what the document says the
/// references may name.
struct Validator<'d> {
    doc: &'d Document,
    presentation: bool,
    /// The mutator path being walked.
    path: Vec<usize>,
    /// The subtree path, inside an insert.
    subtree: Option<Vec<usize>>,
}

impl<'d> Validator<'d> {
    fn new(doc: &'d Document, presentation: bool) -> Self {
        Validator {
            doc,
            presentation,
            path: Vec::new(),
            subtree: None,
        }
    }

    /// Where the walk is, for an error.
    fn at(&self) -> MutatorPath {
        MutatorPath {
            mutator: self.path.clone(),
            subtree: self.subtree.clone(),
        }
    }

    fn bound<T>(&self, field: &'static str, result: Result<T, BoundError>) -> Result<(), MutationError> {
        result.map(drop).map_err(|error| MutationError::Bound {
            at: self.at(),
            field,
            error,
        })
    }

    fn missing(&self, what: &'static str, index: usize) -> MutationError {
        MutationError::Missing {
            at: self.at(),
            what,
            index,
        }
    }

    fn run(mut self, mutator: &DocMutatorTree) -> Result<(), MutationError> {
        self.node(mutator, mutator.root)
    }

    /// Judge the mutator at `id` and everything below it.
    fn node(&mut self, tree: &DocMutatorTree, id: NodeId) -> Result<(), MutationError> {
        let Some(node) = tree.get(id) else {
            return Ok(());
        };
        let mutator = node.get();
        let key = mutator.key();
        match mutator {
            Mutator::Void { .. } => {}
            Mutator::Single { mutation, .. } => self.payload(tree, id, mutation, key)?,
            Mutator::Macro { mutations, .. } => {
                for mutation in mutations {
                    self.payload(tree, id, mutation, key)?;
                }
            }
            Mutator::Instruction {
                instruction,
                mutation,
                ..
            } => {
                if !matches!(instruction, Instruction::RepeatWhile(_)) {
                    return Err(MutationError::Instruction { at: self.at() });
                }
                if let Some(mutation) = mutation {
                    self.payload(tree, id, mutation, key)?;
                }
            }
            Mutator::Structure { op, .. } => {
                if self.presentation {
                    return Err(MutationError::PresentationStructure { at: self.at() });
                }
                self.structure(op, key)?;
            }
        }
        for (position, child) in id.children(tree.arena()).enumerate() {
            self.path.push(position);
            self.node(tree, child)?;
            self.path.pop();
        }
        Ok(())
    }

    /// Judge one payload on the mutator `id`, keyed `key`.
    fn payload(
        &self,
        tree: &DocMutatorTree,
        id: NodeId,
        mutation: &DocMutation,
        key: BranchKey<DocKind>,
    ) -> Result<(), MutationError> {
        let kind = key.kind;
        if !mutation.applies_to(kind) {
            return Err(MutationError::PayloadKind {
                at: self.at(),
                kind,
                payload: mutation.name(),
            });
        }
        match mutation {
            DocMutation::Map(MapOp::SetName(_)) => Ok(()),
            DocMutation::Theme(op) => self.theme(op),
            DocMutation::Palette(PaletteOp::SetGroup { index, group }) => {
                // The group must exist in some palette; a palette the
                // walk reaches that is shorter is left as it is (see
                // `ops`).
                if !self.doc.palettes().any(|(_, p)| *index < p.groups.len()) {
                    return Err(self.missing("palette group", *index));
                }
                self.palette_group(group)
            }
            DocMutation::Defaults(DefaultsOp::Replace(defaults)) => self.defaults(defaults),
            DocMutation::Node(op) => self.node_op(op),
            DocMutation::Section(op) => match op {
                SectionOp::SetOffset(offset) => self.canvas("section.offset", *offset),
                SectionOp::Nudge(delta) => self.finite("section.nudge", *delta),
                SectionOp::SetMinSize(size) => self.extent("section.min_size", *size),
                SectionOp::SetAlign(_) | SectionOp::SetChannel(_) => Ok(()),
                SectionOp::Text(op) => self.text_op(op, Unit::Points),
            },
            DocMutation::Span(op) => match op {
                SpanOp::SetText(_) | SpanOp::InsertText { .. } => Ok(()),
                SpanOp::DeleteText { start, end } => {
                    if start > end {
                        // An inverted range names nothing to delete; it
                        // is a malformed request, not an empty one.
                        #[allow(clippy::cast_precision_loss)]
                        let (min, max) = (*start as f32, *end as f32);
                        return self.bound(
                            "span.delete_text",
                            Err::<(), _>(BoundError::Inverted { min, max }),
                        );
                    }
                    Ok(())
                }
                SpanOp::SetLink(link) => self.link_target(link.as_ref()),
                SpanOp::Text(op) => self.text_op(op, Unit::Either),
            },
            DocMutation::Frame(op) => match op {
                FrameOp::SetVisible(_) | FrameOp::SetDirection(_) => Ok(()),
                FrameOp::SetStart(start) => self.start(start.as_ref()),
                FrameOp::SetFit(fit) => self.fit("frame.fit", fit.as_ref()),
                FrameOp::SetGap(gap) => self.non_negative("frame.gap", *gap),
                FrameOp::SetInset(inset) => self.non_negative("frame.inset", *inset),
                FrameOp::Text(op) => self.text_op(op, Unit::Relative),
            },
            DocMutation::Ring(op) => match op {
                RingOp::SetFit(fit) => self.fit("ring.fit", fit.as_ref()),
                RingOp::SetChannel(_) => Ok(()),
                RingOp::Text(op) => self.text_op(op, Unit::Relative),
            },
            DocMutation::Path(op) => self.path_op(op),
            DocMutation::Link(LinkOp::SetKind(link_kind)) => self.kind_switch(tree, id, *link_kind),
            DocMutation::Cap(op) => self.text_op(op, Unit::Points),
            DocMutation::Label(op) => match op {
                LabelOp::SetT(t) => self.unit_interval("label.t", *t),
                LabelOp::SetOffset(offset) => self.canvas_scalar("label.offset", *offset),
                LabelOp::Text(op) => self.text_op(op, Unit::Points),
            },
            DocMutation::PortalEnd(op) => match op {
                PortalEndOp::SetBorderT(t) => self.unit_interval("portal_end.border_t", *t),
                PortalEndOp::SetOffset(offset) => self.canvas_scalar("portal_end.offset", *offset),
                PortalEndOp::SetColor(color) => self.color(color.as_ref()),
                PortalEndOp::SetSize(size) => self.points("portal_end.size", *size),
                PortalEndOp::SetContent(_) => Ok(()),
            },
        }
    }

    fn theme(&self, op: &ThemeOp) -> Result<(), MutationError> {
        let theme = self.doc.theme();
        match op {
            ThemeOp::SetActive(variant) => self.variant(usize::from(*variant), theme.variants.len()),
            ThemeOp::SetValue { variant, var, .. } => {
                self.variant(usize::from(*variant), theme.variants.len())?;
                self.var(*var)
            }
        }
    }

    fn variant(&self, index: usize, count: usize) -> Result<(), MutationError> {
        if index < count {
            Ok(())
        } else {
            Err(self.missing("theme variant", index))
        }
    }

    fn var(&self, var: VarId) -> Result<(), MutationError> {
        if usize::from(var.0) < self.doc.theme().variables.len() {
            Ok(())
        } else {
            Err(self.missing("theme variable", usize::from(var.0)))
        }
    }

    fn verb(&self, verb: Option<VerbId>) -> Result<(), MutationError> {
        match verb {
            Some(verb) if usize::from(verb.0) >= self.doc.definitions().verbs.len() => {
                Err(self.missing("verb", usize::from(verb.0)))
            }
            _ => Ok(()),
        }
    }

    fn palette(&self, palette: Option<PaletteId>) -> Result<(), MutationError> {
        match palette {
            Some(palette) if !self.doc.palettes().any(|(id, _)| id == palette) => {
                Err(self.missing("palette", usize::from(palette.0)))
            }
            _ => Ok(()),
        }
    }

    fn color(&self, color: Option<&ColorRef>) -> Result<(), MutationError> {
        match color {
            Some(ColorRef::Var(var)) => self.var(*var),
            _ => Ok(()),
        }
    }

    fn plain(&self, color: &PlainColor) -> Result<(), MutationError> {
        match color {
            PlainColor::Var(var) => self.var(*var),
            PlainColor::Rgba(_) => Ok(()),
        }
    }

    fn palette_group(&self, group: &PaletteGroup) -> Result<(), MutationError> {
        self.plain(&group.fill)?;
        self.plain(&group.frame)?;
        self.plain(&group.text)?;
        group.title.as_ref().map_or(Ok(()), |title| self.plain(title))
    }

    fn link_target(&self, link: Option<&LinkTarget>) -> Result<(), MutationError> {
        match link {
            Some(LinkTarget::Verb(verb)) => self.verb(Some(*verb)),
            _ => Ok(()),
        }
    }

    fn canvas(&self, field: &'static str, value: Vec2) -> Result<(), MutationError> {
        self.bound(field, check_canvas_coord(value.x))?;
        self.bound(field, check_canvas_coord(value.y))
    }

    fn canvas_scalar(&self, field: &'static str, value: Option<f32>) -> Result<(), MutationError> {
        value.map_or(Ok(()), |v| self.bound(field, check_canvas_coord(v)))
    }

    fn finite(&self, field: &'static str, value: Vec2) -> Result<(), MutationError> {
        for v in [value.x, value.y] {
            if !v.is_finite() {
                return self.bound(field, Err::<(), _>(BoundError::NotFinite(v)));
            }
        }
        Ok(())
    }

    fn extent(&self, field: &'static str, size: Option<Vec2>) -> Result<(), MutationError> {
        match size {
            Some(size) => {
                self.bound(field, check_extent(size.x))?;
                self.bound(field, check_extent(size.y))
            }
            None => Ok(()),
        }
    }

    fn non_negative(&self, field: &'static str, value: Option<f32>) -> Result<(), MutationError> {
        value.map_or(Ok(()), |v| self.bound(field, check_non_negative(v)))
    }

    fn unit_interval(&self, field: &'static str, value: Option<f32>) -> Result<(), MutationError> {
        value.map_or(Ok(()), |v| self.bound(field, check_unit_interval(v)))
    }

    fn points(&self, field: &'static str, value: Option<f32>) -> Result<(), MutationError> {
        value.map_or(Ok(()), |v| self.bound(field, check_font_size_pt(v)))
    }

    fn zoom(&self, field: &'static str, zoom: Option<&ZoomWindow>) -> Result<(), MutationError> {
        zoom.map_or(Ok(()), |z| self.bound(field, check_zoom_window(z.min, z.max)))
    }

    fn fit(&self, field: &'static str, fit: Option<&FitMode>) -> Result<(), MutationError> {
        match fit {
            Some(FitMode::Fit { min, max }) => self.bound(field, check_fit(*min, *max)),
            _ => Ok(()),
        }
    }

    fn start(&self, start: Option<&StartPoint>) -> Result<(), MutationError> {
        start.map_or(Ok(()), |s| self.bound("frame.start", check_unit_interval(s.at)))
    }

    /// A size in the unit `unit` takes, within that unit's bound.
    fn size(&self, field: &'static str, size: Option<FontSize>, unit: Unit) -> Result<(), MutationError> {
        let Some(size) = size else {
            return Ok(());
        };
        if !unit.admits(size) {
            return Err(MutationError::Unit {
                at: self.at(),
                field,
                size,
            });
        }
        match size {
            FontSize::Pt(v) => self.bound(field, check_font_size_pt(v)),
            FontSize::Rel(v) => self.bound(field, check_rel_size(v)),
        }
    }

    fn text_style(&self, style: &TextStyle, unit: Unit) -> Result<(), MutationError> {
        self.size("text.size", style.size, unit)?;
        self.color(style.color.as_ref())
    }

    fn text_op(&self, op: &TextOp, unit: Unit) -> Result<(), MutationError> {
        match op {
            TextOp::SetFont(_) | TextOp::SetWeight(_) | TextOp::SetItalic(_) => Ok(()),
            TextOp::SetSize(size) => self.size("text.size", *size, unit),
            TextOp::GrowSize(delta) => {
                if delta.is_finite() {
                    Ok(())
                } else {
                    self.bound("text.grow_size", Err::<(), _>(BoundError::NotFinite(*delta)))
                }
            }
            TextOp::ScaleSize(factor) => {
                if !factor.is_finite() {
                    self.bound("text.scale_size", Err::<(), _>(BoundError::NotFinite(*factor)))
                } else if *factor <= 0.0 {
                    self.bound(
                        "text.scale_size",
                        Err::<(), _>(BoundError::Below {
                            value: *factor,
                            min: 0.0,
                            inclusive: false,
                        }),
                    )
                } else {
                    Ok(())
                }
            }
            TextOp::SetColor(color) => self.color(color.as_ref()),
        }
    }

    fn node_op(&self, op: &NodeOp) -> Result<(), MutationError> {
        match op {
            NodeOp::SetPosition(position) => self.canvas("node.position", *position),
            NodeOp::Nudge(delta) => self.finite("node.nudge", *delta),
            NodeOp::SetMinSize(size) => self.extent("node.min_size", *size),
            NodeOp::SetShape(_)
            | NodeOp::SetFolded(_)
            | NodeOp::ToggleFolded
            | NodeOp::SetHidden(_)
            | NodeOp::SetChannel(_) => Ok(()),
            NodeOp::SetFill(fill) => self.color(fill.as_ref()),
            NodeOp::SetFillFade(fade) => self.non_negative("node.fill_fade", *fade),
            NodeOp::SetPalette(palette) => self.palette(*palette),
            NodeOp::SetZoom(zoom) => self.zoom("node.zoom", zoom.as_ref()),
            NodeOp::SetOpacity(opacity) => self.bound("node.opacity", check_unit_interval(*opacity)),
            NodeOp::SetClick(verb) | NodeOp::SetHover(verb) => self.verb(*verb),
            NodeOp::Text(op) => self.text_op(op, Unit::Points),
        }
    }

    fn path_op(&self, op: &PathOp) -> Result<(), MutationError> {
        match op {
            PathOp::SetFit(fit) => self.fit("path.fit", fit.as_ref()),
            PathOp::SetSpacing(spacing) => self.non_negative("path.spacing", *spacing),
            PathOp::SetSize(size) => self.points("path.size", *size),
            PathOp::SetColor(color) => self.color(color.as_ref()),
            PathOp::SetContent(_) | PathOp::SetAnchors(..) | PathOp::SetVisible(_) => Ok(()),
            PathOp::SetControlPoints(points) => self.control_points(points),
            PathOp::MoveControlPoint { index, to } => {
                // The point must be one a path can have; whether the
                // path reached has it is the walk's (see `ops`).
                self.bound(
                    "path.control_points",
                    check_control_points(index.saturating_add(1)),
                )?;
                self.canvas("path.control_points", *to)
            }
            PathOp::SetZoom(zoom) => self.zoom("path.zoom", zoom.as_ref()),
        }
    }

    fn control_points(&self, points: &[Vec2]) -> Result<(), MutationError> {
        self.bound("path.control_points", check_control_points(points.len()))?;
        points
            .iter()
            .try_for_each(|point| self.canvas("path.control_points", *point))
    }

    fn flow(&self, flow: &FlowStyle) -> Result<(), MutationError> {
        self.fit("path.fit", flow.fit.as_ref())?;
        self.non_negative("path.spacing", flow.spacing)?;
        self.points("path.size", flow.size)?;
        self.color(flow.color.as_ref())
    }

    fn path_data(&self, path: &PathData) -> Result<(), MutationError> {
        self.flow(&path.flow)?;
        self.control_points(&path.control_points)?;
        self.zoom("path.zoom", path.zoom.as_ref())
    }

    fn node_style(&self, style: &NodeStyle) -> Result<(), MutationError> {
        self.color(style.fill.as_ref())?;
        self.non_negative("node.fill_fade", style.fill_fade)?;
        self.text_style(&style.text, Unit::Points)
    }

    fn frame_style(&self, style: &FrameStyle) -> Result<(), MutationError> {
        self.start(style.start.as_ref())?;
        self.fit("frame.fit", style.fit.as_ref())?;
        self.non_negative("frame.gap", style.gap)?;
        self.non_negative("frame.inset", style.inset)?;
        self.text_style(&style.text, Unit::Relative)
    }

    fn label_style(&self, style: &LabelStyle) -> Result<(), MutationError> {
        self.unit_interval("label.t", style.t)?;
        self.canvas_scalar("label.offset", style.offset)?;
        self.text_style(&style.text, Unit::Points)
    }

    fn portal_end_style(&self, style: &PortalEndStyle) -> Result<(), MutationError> {
        self.unit_interval("portal_end.border_t", style.border_t)?;
        self.canvas_scalar("portal_end.offset", style.offset)?;
        self.color(style.color.as_ref())?;
        self.points("portal_end.size", style.size)
    }

    /// Every default, each in the unit its tier serves: the frame's and
    /// the ring's relative, every other one in points (ruling R10 on
    /// #237).
    fn defaults(&self, defaults: &DefaultsData) -> Result<(), MutationError> {
        self.node_style(&defaults.node)?;
        self.text_style(&defaults.section, Unit::Points)?;
        self.frame_style(&defaults.frame)?;
        self.fit("ring.fit", defaults.ring.fit.as_ref())?;
        self.text_style(&defaults.ring.text, Unit::Relative)?;
        self.flow(&defaults.connector)?;
        self.flow(&defaults.link)?;
        self.label_style(&defaults.label)?;
        self.portal_end_style(&defaults.portal_end)?;
        self.text_style(&defaults.span, Unit::Points)
    }

    /// One element of an inserted subtree, under `parent` (its parent in
    /// the subtree; `None` for the subtree's root).
    fn element(&self, element: &DocElement, parent: Option<DocKind>) -> Result<(), MutationError> {
        match element {
            DocElement::Node(node) => {
                if node.ident != NodeIdent::UNASSIGNED {
                    return Err(MutationError::AssignedIdent {
                        at: self.at(),
                        ident: node.ident,
                    });
                }
                self.canvas("node.position", node.position)?;
                self.extent("node.min_size", node.min_size)?;
                self.node_style(&node.style)?;
                self.palette(node.palette)?;
                self.zoom("node.zoom", node.zoom.as_ref())?;
                self.bound("node.opacity", check_unit_interval(node.opacity))?;
                self.verb(node.on.click)?;
                self.verb(node.on.hover)
            }
            DocElement::Section(section) => {
                self.canvas("section.offset", section.offset)?;
                self.extent("section.min_size", section.min_size)?;
                self.text_style(&section.text, Unit::Points)
            }
            DocElement::Span(span) => {
                let unit = match parent {
                    Some(DocKind::Ring) => Unit::Relative,
                    Some(_) => Unit::Points,
                    None => Unit::Either,
                };
                self.text_style(&span.style, unit)?;
                self.link_target(span.link.as_ref())
            }
            DocElement::Frame(frame) => self.frame_style(&frame.style),
            DocElement::Ring(ring) => {
                self.fit("ring.fit", ring.style.fit.as_ref())?;
                self.text_style(&ring.style.text, Unit::Relative)
            }
            DocElement::Connector(connector) => self.path_data(&connector.path),
            DocElement::Link(link) => {
                if self.doc.get(link.to).map(DocElement::kind) != Some(DocKind::Node)
                    || !self.doc.tree().contains(link.to)
                {
                    return Err(MutationError::LinkTarget {
                        at: self.at(),
                        to: link.to,
                    });
                }
                self.path_data(&link.path)
            }
            DocElement::CapStart(cap) | DocElement::CapEnd(cap) => self.text_style(&cap.text, Unit::Points),
            DocElement::Label(label) => self.label_style(&label.style),
            DocElement::PortalEnd(end) => self.portal_end_style(&end.style),
            DocElement::Map(_) | DocElement::Theme(_) | DocElement::Palette(_) => Ok(()),
            DocElement::Defaults(defaults) => self.defaults(defaults),
        }
    }

    /// Every element of an inserted subtree, in pre-order, naming each
    /// by its path in the subtree.
    fn walk_subtree(
        &mut self,
        subtree: &Subtree<DocElement>,
        parent: Option<DocKind>,
    ) -> Result<(), MutationError> {
        self.element(&subtree.root, parent)?;
        for (position, child) in subtree.children.iter().enumerate() {
            if let Some(path) = self.subtree.as_mut() {
                path.push(position);
            }
            self.walk_subtree(child, Some(subtree.root.kind()))?;
            if let Some(path) = self.subtree.as_mut() {
                path.pop();
            }
        }
        Ok(())
    }

    /// Judge a structural command keyed `key`.
    fn structure(&mut self, op: &StructOp<DocElement>, key: BranchKey<DocKind>) -> Result<(), MutationError> {
        let refuse = |this: &Self, rule| Err(MutationError::Structure { at: this.at(), rule });
        let map_row = matches!(key.kind, DocKind::Theme | DocKind::Palette | DocKind::Defaults);
        match op {
            StructOp::Insert { subtree, .. } => {
                let inserted = subtree.root.kind();
                if inserted == DocKind::Map {
                    return refuse(self, StructureRule::Map);
                }
                if key.kind == DocKind::Map && inserted != DocKind::Node {
                    return refuse(self, StructureRule::MapRow);
                }
                Document::validate_subtree(subtree)
                    .map_err(|error| MutationError::Subtree { at: self.at(), error })?;
                self.subtree = Some(Vec::new());
                let result = self.walk_subtree(subtree, None);
                self.subtree = None;
                result
            }
            StructOp::Remove => {
                if key.kind == DocKind::Map {
                    refuse(self, StructureRule::Map)
                } else if map_row {
                    refuse(self, StructureRule::MapRow)
                } else {
                    Ok(())
                }
            }
            StructOp::Move { to, .. } => {
                if key.kind != DocKind::Node {
                    return refuse(self, StructureRule::MoveNotANode);
                }
                let destination = self.doc.get(*to).map(DocElement::kind);
                if !self.doc.tree().contains(*to)
                    || !matches!(destination, Some(DocKind::Node | DocKind::Map))
                {
                    return refuse(self, StructureRule::MoveDestination);
                }
                Ok(())
            }
            StructOp::Split { .. } | StructOp::Merge => {
                if key.kind == DocKind::Span {
                    Ok(())
                } else {
                    refuse(self, StructureRule::SplitMergeNotASpan)
                }
            }
        }
    }

    /// A `SetKind` on the mutator `id` comes with a `Remove` for every
    /// kind of child the new kind does not hold, as children of the same
    /// mutator: the structural half of the switch, which the composite
    /// builder [`set_link_kind`](crate::document::mutation::compose::set_link_kind)
    /// writes. Whether the link ends up with the children its kind needs
    /// is [`validate_applied`]'s, once every step has run.
    fn kind_switch(&self, tree: &DocMutatorTree, id: NodeId, kind: LinkKind) -> Result<(), MutationError> {
        let required: &[BranchKey<DocKind>] = match kind {
            LinkKind::Portal => &[
                BranchKey::new(DocKind::Span, 0),
                BranchKey::new(DocKind::CapStart, 0),
                BranchKey::new(DocKind::CapEnd, 0),
                BranchKey::new(DocKind::Label, 0),
            ],
            LinkKind::Line => &[
                BranchKey::new(DocKind::PortalEnd, 0),
                BranchKey::new(DocKind::PortalEnd, 1),
            ],
        };
        let removes = |key: &BranchKey<DocKind>| {
            id.children(tree.arena()).any(|child| {
                matches!(
                    tree.arena()[child].get(),
                    Mutator::Structure { key: k, op: StructOp::Remove } if k == key
                )
            })
        };
        if required.iter().all(removes) {
            Ok(())
        } else {
            Err(MutationError::BareKindSwitch { at: self.at(), kind })
        }
    }
}

/// Judge what the steps of a request did, from `journal`, against
/// `tree` as they left it. See the module docs.
///
/// - `validate_shape` on every parent a node left or joined
///   ([`Journal::parents`]), and on every node that joined a row —
///   inserted at the top of its subtree, or moved — since a node's
///   connector rule depends on where it sits;
/// - `validate_shape` on every link whose kind changed;
/// - an inserted link leads to a node in the tree other than its own;
/// - every inserted or changed element's text size is in the unit its
///   place takes: relative in a frame, a ring and a ring's spans, points
///   everywhere else.
///
/// Errors: [`MutationError::Shape`], [`MutationError::SelfLink`],
/// [`MutationError::DanglingLink`], [`MutationError::PlacedUnit`]. The
/// caller undoes the request on any of them.
///
/// Cost: proportional to the journal: per touched element O(1) checks,
/// one owning-node walk per inserted link, and one row pass per changed
/// parent, each parent judged once (a hash set of them).
pub fn validate_applied(tree: &Tree<DocElement>, journal: &Journal<DocElement>) -> Result<(), MutationError> {
    let mut judged: FxHashSet<NodeId> = FxHashSet::default();
    let mut shape = |id: NodeId| -> Result<(), MutationError> {
        if tree.contains(id) && judged.insert(id) {
            validate_shape(tree, id).map_err(MutationError::Shape)?;
        }
        Ok(())
    };
    for (id, touch) in journal.touched() {
        if let Some((before, after)) = journal.parents(id) {
            for parent in [before, after].into_iter().flatten() {
                shape(parent)?;
            }
            if after.is_some() {
                shape(id)?;
            }
        }
        if touch == Touch::Removed || !tree.contains(id) {
            continue;
        }
        let Some(element) = tree.get(id).map(|n| n.get()) else {
            continue;
        };
        if let DocElement::Link(link) = element {
            let switched =
                matches!(journal.before(id), Some(DocElement::Link(before)) if before.kind != link.kind);
            if switched {
                shape(id)?;
            }
            if touch == Touch::Inserted {
                let target_is_node = tree.contains(link.to)
                    && tree.get(link.to).map(|n| n.get().kind()) == Some(DocKind::Node);
                if !target_is_node {
                    return Err(MutationError::DanglingLink {
                        link: id,
                        to: link.to,
                    });
                }
                if owning_node(tree, id) == Some(link.to) {
                    return Err(MutationError::SelfLink { link: id });
                }
            }
        }
        placed_unit(tree, id, element)?;
    }
    Ok(())
}

/// The element at `id`'s own text size, if it sets one, is in the unit
/// its place takes. O(1).
fn placed_unit(tree: &Tree<DocElement>, id: NodeId, element: &DocElement) -> Result<(), MutationError> {
    let Some(size) = element.text_style().and_then(|style| style.size) else {
        return Ok(());
    };
    let unit = match element.kind() {
        DocKind::Span => {
            let parent = tree.get(id).and_then(|n| n.parent());
            match parent.and_then(|p| tree.get(p)).map(|n| n.get().kind()) {
                Some(DocKind::Ring) => Unit::Relative,
                _ => Unit::Points,
            }
        }
        kind => Unit::of(kind),
    };
    if unit.admits(size) {
        Ok(())
    } else {
        Err(MutationError::PlacedUnit { element: id, size })
    }
}
