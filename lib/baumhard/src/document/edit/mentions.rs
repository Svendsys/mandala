// SPDX-License-Identifier: MPL-2.0

//! What an edit says about the rest of the map, anywhere in it (its
//! filter or its body): the verbs it names, and whether it can make
//! drawn what a file saves undrawn. `maptool verify` reads it, so that a
//! verb an edit names is not reported as unreferenced, and a node or
//! portal an edit can show is not reported as never shown.
//!
//! Every edit type is walked by name: structs are destructured without
//! `..` and enums matched without a wildcard, so a step added to the
//! format fails to compile here until it is counted or listed as
//! saying nothing.

use crate::document::edit::{EditBody, EditDef, Filter, Mutations, MutatorAst};
use crate::document::format::dto::LinkTargetDto;
use crate::document::format::edits::{
    FieldDto, FilterDto, FrameOpDto, InstructionDto, LabelOpDto, LayoutOpDto, MutationDto, MutatorDto,
    NodeOpDto, PathOpDto, PortalEndOpDto, RingOpDto, SectionOpDto, SpanOpDto, TextOpDto,
};

/// What an edit can make drawn, as [`EditDef::mentions`] finds it.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Shows {
    /// Whether it writes `set_hidden: false`.
    pub reveals: bool,
    /// Whether it writes a portal end's `set_content` to a
    /// non-empty marker.
    pub marks_portal_ends: bool,
}

impl EditDef {
    /// Calls `verb` with each verb id this edit names — a node's
    /// `set_click` or `set_hover`, a span's `set_link` to `{"verb": …}`, a
    /// filter's `on_click` or `on_hover` — and returns what it can show,
    /// wherever in it they are written: its filter, its mutations, a
    /// mutator's every macro and `repeat_while` test, at any depth. A
    /// handler names nothing and shows nothing: each one is a layout that
    /// writes positions ([`edit_handlers`](crate::edit_handlers)).
    ///
    /// What the edit writes, not where it runs: which elements it
    /// reaches depends, in general, on the anchors it is run at and on the
    /// map then.
    ///
    /// Cost: one walk of the definition, one call of `verb` per verb id it
    /// writes, and no allocation.
    pub fn mentions<'a>(&'a self, mut verb: impl FnMut(&'a str)) -> Shows {
        let EditDef {
            id: _,
            name: _,
            description: _,
            reach: _,
            target: _,
            filter,
            body,
            animate: _,
        } = self;
        let mut walk = Walk {
            verb: &mut verb,
            shows: Shows::default(),
        };
        if let Some(Filter(filter)) = filter {
            walk.filter(filter);
        }
        match body {
            EditBody::Mutations(Mutations(mutations)) => walk.mutations(mutations),
            EditBody::Mutator(MutatorAst(mutator)) => walk.mutator(mutator),
            EditBody::Handler(_) => {}
        }
        walk.shows
    }
}

/// One walk of an edit: where its verb ids go, and what it has found it
/// can show.
struct Walk<'a, 'f> {
    verb: &'f mut dyn FnMut(&'a str),
    shows: Shows,
}

impl<'a> Walk<'a, '_> {
    fn mutator(&mut self, mutator: &'a MutatorDto) {
        let children = match mutator {
            MutatorDto::Void {
                kind: _,
                channel: _,
                children,
            } => children,
            MutatorDto::Macro {
                kind: _,
                channel: _,
                mutations,
                children,
            } => {
                self.mutations(mutations);
                children
            }
            MutatorDto::Instruction {
                kind: _,
                channel: _,
                instruction,
                children,
            } => {
                match instruction {
                    InstructionDto::RepeatWhile(filter) => self.filter(filter),
                    InstructionDto::RepeatWhileAlwaysTrue => {}
                }
                children
            }
        };
        children.iter().for_each(|child| self.mutator(child));
    }

    fn filter(&mut self, filter: &'a FilterDto) {
        let FilterDto {
            fields,
            always_match: _,
        } = filter;
        for (field, _) in fields {
            match field {
                FieldDto::OnClick(verb) | FieldDto::OnHover(verb) => (self.verb)(verb),
                FieldDto::Kind(_)
                | FieldDto::Background(_)
                | FieldDto::Channel(_)
                | FieldDto::Ident(_)
                | FieldDto::PositionX(_)
                | FieldDto::PositionY(_)
                | FieldDto::MinWidth(_)
                | FieldDto::MinHeight(_)
                | FieldDto::Shape(_)
                | FieldDto::Palette(_)
                | FieldDto::Folded(_)
                | FieldDto::Hidden(_)
                | FieldDto::Opacity(_)
                | FieldDto::ZoomMin(_)
                | FieldDto::ZoomMax(_)
                | FieldDto::OffsetX(_)
                | FieldDto::OffsetY(_)
                | FieldDto::Align(_)
                | FieldDto::Font(_)
                | FieldDto::FontSize(_)
                | FieldDto::Color(_)
                | FieldDto::Weight(_)
                | FieldDto::Italic(_)
                | FieldDto::Visible(_)
                | FieldDto::StartEdge(_)
                | FieldDto::StartAt(_)
                | FieldDto::Direction(_)
                | FieldDto::Fit(_)
                | FieldDto::Gap(_)
                | FieldDto::Inset(_)
                | FieldDto::Spacing(_)
                | FieldDto::AnchorFrom(_)
                | FieldDto::AnchorTo(_)
                | FieldDto::LinkKind(_)
                | FieldDto::LabelT(_)
                | FieldDto::Offset(_)
                | FieldDto::BorderT(_) => {}
            }
        }
    }

    fn mutations(&mut self, mutations: &'a [MutationDto]) {
        for mutation in mutations {
            match mutation {
                MutationDto::Node(op) => self.node(op),
                MutationDto::Section(op) => match op {
                    SectionOpDto::Text(op) => text(op),
                    SectionOpDto::SetOffset(_)
                    | SectionOpDto::Nudge(_)
                    | SectionOpDto::SetMinSize(_)
                    | SectionOpDto::SetAlign(_)
                    | SectionOpDto::SetChannel(_) => {}
                },
                MutationDto::Span(op) => match op {
                    SpanOpDto::SetLink(Some(LinkTargetDto::Verb(verb))) => (self.verb)(verb),
                    SpanOpDto::SetLink(Some(LinkTargetDto::Url(_)) | None) => {}
                    SpanOpDto::Text(op) => text(op),
                    SpanOpDto::SetText(_)
                    | SpanOpDto::InsertText { at: _, text: _ }
                    | SpanOpDto::DeleteText { start: _, end: _ } => {}
                },
                MutationDto::Frame(op) => match op {
                    FrameOpDto::Layout(op) => layout(op),
                    FrameOpDto::Text(op) => text(op),
                    FrameOpDto::SetVisible(_)
                    | FrameOpDto::SetStart(_)
                    | FrameOpDto::SetDirection(_)
                    | FrameOpDto::SetFit(_)
                    | FrameOpDto::SetGap(_)
                    | FrameOpDto::SetInset(_) => {}
                },
                MutationDto::Ring(op) => match op {
                    RingOpDto::Layout(op) => layout(op),
                    RingOpDto::Text(op) => text(op),
                    RingOpDto::SetFit(_) | RingOpDto::SetChannel(_) => {}
                },
                MutationDto::Path(op) => match op {
                    PathOpDto::SetFit(_)
                    | PathOpDto::SetSpacing(_)
                    | PathOpDto::SetSize(_)
                    | PathOpDto::SetColor(_)
                    | PathOpDto::SetContent(_)
                    | PathOpDto::SetAnchors(_, _)
                    | PathOpDto::SetControlPoints(_)
                    | PathOpDto::MoveControlPoint { index: _, to: _ }
                    | PathOpDto::SetVisible(_)
                    | PathOpDto::SetZoom(_) => {}
                },
                MutationDto::Cap(op) => text(op),
                MutationDto::Label(op) => match op {
                    LabelOpDto::Text(op) => text(op),
                    LabelOpDto::SetT(_) | LabelOpDto::SetOffset(_) => {}
                },
                MutationDto::PortalEnd(op) => match op {
                    PortalEndOpDto::SetContent(Some(marker)) if !marker.is_empty() => {
                        self.shows.marks_portal_ends = true
                    }
                    PortalEndOpDto::SetContent(_)
                    | PortalEndOpDto::SetBorderT(_)
                    | PortalEndOpDto::SetOffset(_)
                    | PortalEndOpDto::SetColor(_)
                    | PortalEndOpDto::SetSize(_) => {}
                },
            }
        }
    }

    fn node(&mut self, op: &'a NodeOpDto) {
        match op {
            NodeOpDto::SetClick(verb) | NodeOpDto::SetHover(verb) => {
                verb.iter().for_each(|verb| (self.verb)(verb))
            }
            NodeOpDto::SetHidden(hidden) => self.shows.reveals |= !hidden,
            NodeOpDto::Text(op) => text(op),
            NodeOpDto::SetPosition(_)
            | NodeOpDto::Nudge(_)
            | NodeOpDto::SetMinSize(_)
            | NodeOpDto::SetShape(_)
            | NodeOpDto::SetPalette(_)
            | NodeOpDto::SetZoom(_)
            | NodeOpDto::SetFolded(_)
            | NodeOpDto::ToggleFolded
            | NodeOpDto::SetOpacity(_)
            | NodeOpDto::SetChannel(_) => {}
        }
    }
}

/// A text style op names no verb and shows nothing; listed so that one
/// added that does fails to compile.
fn text(op: &TextOpDto) {
    match op {
        TextOpDto::SetFont(_)
        | TextOpDto::SetSize(_)
        | TextOpDto::GrowSize(_)
        | TextOpDto::ScaleSize(_)
        | TextOpDto::SetColor(_)
        | TextOpDto::SetWeight(_)
        | TextOpDto::SetItalic(_) => {}
    }
}

/// A ring layout op names no verb and shows nothing; listed as
/// [`text`] is.
fn layout(op: &LayoutOpDto) {
    match op {
        LayoutOpDto::SetSpan(_) | LayoutOpDto::SetSides(_) | LayoutOpDto::SetSpacing(_) => {}
    }
}
