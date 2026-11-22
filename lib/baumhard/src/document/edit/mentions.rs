// SPDX-License-Identifier: MPL-2.0

//! What an edit says about the rest of the map, anywhere in it (its
//! filter or its body): the verbs, theme variables and palettes it
//! names, and whether it can make drawn what a file saves undrawn.
//! `maptool verify` reads it, so that a verb an edit names is not
//! reported as unreferenced, a node or portal an edit can show is not
//! reported as never shown, and a name an edit writes that its map does
//! not define is reported ([`Mention::is_defined_in`]).
//!
//! Every edit type is walked by name: structs are destructured without
//! `..` and enums matched without a wildcard, so a step added to the
//! format fails to compile here until it is counted or listed as
//! saying nothing.

use crate::document::edit::resolve::{palette_of, var_of, verb_of, Fault, NameKind};
use crate::document::edit::{EditBody, EditDef, EditError, Filter, Mutations, MutatorAst};
use crate::document::format::dto::LinkTargetDto;
use crate::document::format::edits::{
    FieldDto, FilterDto, FrameOpDto, InstructionDto, LabelOpDto, LayoutOpDto, MutationDto, MutatorDto,
    NodeOpDto, PathOpDto, PortalEndOpDto, RingOpDto, SectionOpDto, SpanOpDto, TextOpDto,
};
use crate::document::format::spelling::{ColorDto, SidesDto, SidesObjectDto};
use crate::document::ids::Document;

/// What an edit or a presentation can make drawn, as
/// [`EditDef::mentions`] and
/// [`PresentationDef::mentions`](crate::document::presentation::PresentationDef::mentions)
/// find it.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub struct Shows {
    /// Whether it writes `set_hidden: false`.
    pub reveals: bool,
    /// Whether it writes a portal end's `set_content` to a
    /// non-empty marker.
    pub marks_portal_ends: bool,
}

/// What a name an edit or a presentation writes names, and so what the
/// map must define for a run to resolve it. Fonts are not here: a family
/// is held to the faces this build has when the definition is read, on
/// every map.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Named {
    /// A verb, by id: a node's `set_click` or `set_hover`, a span's
    /// `set_link` to `{"verb": …}`, a filter's `on_click` or `on_hover`.
    Verb,
    /// A theme variable, in a color written `var(name)`.
    Variable,
    /// A palette, by name: a node's `set_palette`, a filter's `palette`.
    Palette,
}

/// One name a definition writes, as [`EditDef::mentions`] and
/// [`PresentationDef::mentions`](crate::document::presentation::PresentationDef::mentions)
/// report it.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Mention<'a> {
    /// What the name names.
    pub kind: Named,
    /// The field that writes it, as a refusal of a run names it
    /// ([`EditError::UnknownName`]):
    /// `set_click`, `color`, ….
    pub field: &'static str,
    /// The name.
    pub name: &'a str,
}

impl Mention<'_> {
    /// Whether `doc` defines the name: what the run of an edit asks of
    /// the open map, by the same probes (`resolve`'s `verb_of`, `var_of`
    /// and `palette_of`), so a name this answers `false` for is one a
    /// run is refused for
    /// ([`EditError::UnknownName`]),
    /// and one it answers `true` for is not. A map's verbs, theme
    /// variables and palettes are the file's own and change only by
    /// loading another: no operation adds, removes or renames one.
    ///
    /// Cost: one probe of the document's index.
    pub fn is_defined_in(&self, doc: &Document) -> bool {
        let names = Some(doc);
        match self.kind {
            Named::Verb => verb_of(names, self.name).is_some(),
            Named::Variable => var_of(names, self.name).is_some(),
            Named::Palette => palette_of(names, self.name).is_some(),
        }
    }

    /// The refusal a run of the edit `edit` earns for this name when the
    /// map does not define it, to the word: what `verify` prints for it.
    ///
    /// Cost: a few small allocations.
    pub fn refusal(&self, edit: &str) -> EditError {
        let kind = match self.kind {
            Named::Verb => NameKind::Verb,
            Named::Variable => NameKind::Variable,
            Named::Palette => NameKind::Palette,
        };
        Fault::Unknown {
            field: self.field,
            kind,
            name: self.name.to_string(),
        }
        .into_error(edit)
    }
}

impl EditDef {
    /// Calls `mention` with each name this edit writes that its map must
    /// define — a verb id (a node's `set_click` or `set_hover`, a span's
    /// `set_link` to `{"verb": …}`, a filter's `on_click` or `on_hover`),
    /// a theme variable (a color written `var(name)`, in a style op, a
    /// path or portal end color, a ring's sides, or a filter's `color` or
    /// `background`) or a palette (a node's `set_palette`, a filter's
    /// `palette`) — and returns what it can show, wherever in it they are
    /// written: its filter, its mutations, a mutator's every macro and
    /// `repeat_while` test, at any depth. A handler names nothing and
    /// shows nothing: each one is a layout that writes positions
    /// ([`edit_handlers`](crate::edit_handlers)).
    ///
    /// What the edit writes, not where it runs: which elements it
    /// reaches depends, in general, on the anchors it is run at and on the
    /// map then. Every name is reported, not only the first one a run
    /// stops at, in the order the definition writes them.
    ///
    /// Cost: one walk of the definition, one call of `mention` per name
    /// it writes, and no allocation.
    pub fn mentions<'a>(&'a self, mut mention: impl FnMut(Mention<'a>)) -> Shows {
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
        let mut walk = Walk::new(&mut mention);
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

/// One walk of a definition — an edit, or a presentation, whose filter
/// and mutations are spelled as an edit's are: where the names it writes
/// go, and what it has found it can show.
pub(in crate::document) struct Walk<'a, 'f> {
    mention: &'f mut dyn FnMut(Mention<'a>),
    pub(in crate::document) shows: Shows,
}

impl<'a, 'f> Walk<'a, 'f> {
    /// A walk that hands each name it meets to `mention`, having found
    /// nothing yet.
    pub(in crate::document) fn new(mention: &'f mut dyn FnMut(Mention<'a>)) -> Self {
        Walk {
            mention,
            shows: Shows::default(),
        }
    }

    /// Reports `name`, which `field` writes, as a `kind`.
    fn name(&mut self, kind: Named, field: &'static str, name: &'a str) {
        (self.mention)(Mention { kind, field, name });
    }

    /// A color: its variable, if it is one. A palette color is a field of
    /// the owning node's palette, not a name.
    ///
    /// The field is `color` for every color, a filter's `background`
    /// included, as a run's refusal of one says it.
    fn color(&mut self, color: &'a ColorDto<'_>) {
        match color {
            ColorDto::Var(name) => self.name(Named::Variable, "color", name),
            ColorDto::Hex(_) | ColorDto::Palette(_) => {}
        }
    }

    /// An optional color, a style op's or a path's: `null` clears it.
    fn color_opt(&mut self, color: &'a Option<ColorDto<'_>>) {
        if let Some(color) = color {
            self.color(color);
        }
    }

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

    pub(in crate::document) fn filter(&mut self, filter: &'a FilterDto) {
        let FilterDto {
            fields,
            always_match: _,
        } = filter;
        for (field, _) in fields {
            match field {
                FieldDto::OnClick(verb) => self.name(Named::Verb, "on_click", verb),
                FieldDto::OnHover(verb) => self.name(Named::Verb, "on_hover", verb),
                FieldDto::Palette(palette) => self.name(Named::Palette, "palette", palette),
                FieldDto::Background(color) | FieldDto::Color(color) => self.color(color),
                FieldDto::Kind(_)
                | FieldDto::Channel(_)
                | FieldDto::Ident(_)
                | FieldDto::PositionX(_)
                | FieldDto::PositionY(_)
                | FieldDto::MinWidth(_)
                | FieldDto::MinHeight(_)
                | FieldDto::Shape(_)
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

    pub(in crate::document) fn mutations(&mut self, mutations: &'a [MutationDto]) {
        for mutation in mutations {
            match mutation {
                MutationDto::Node(op) => self.node(op),
                MutationDto::Section(op) => match op {
                    SectionOpDto::Text(op) => self.text(op),
                    SectionOpDto::SetOffset(_)
                    | SectionOpDto::Nudge(_)
                    | SectionOpDto::SetMinSize(_)
                    | SectionOpDto::SetAlign(_)
                    | SectionOpDto::SetChannel(_) => {}
                },
                MutationDto::Span(op) => match op {
                    SpanOpDto::SetLink(Some(LinkTargetDto::Verb(verb))) => {
                        self.name(Named::Verb, "set_link", verb)
                    }
                    SpanOpDto::SetLink(Some(LinkTargetDto::Url(_)) | None) => {}
                    SpanOpDto::Text(op) => self.text(op),
                    SpanOpDto::SetText(_)
                    | SpanOpDto::InsertText { at: _, text: _ }
                    | SpanOpDto::DeleteText { start: _, end: _ } => {}
                },
                MutationDto::Frame(op) => match op {
                    FrameOpDto::Layout(op) => self.layout(op),
                    FrameOpDto::Text(op) => self.text(op),
                    FrameOpDto::SetVisible(_)
                    | FrameOpDto::SetStart(_)
                    | FrameOpDto::SetDirection(_)
                    | FrameOpDto::SetFit(_)
                    | FrameOpDto::SetGap(_)
                    | FrameOpDto::SetInset(_) => {}
                },
                MutationDto::Ring(op) => match op {
                    RingOpDto::Layout(op) => self.layout(op),
                    RingOpDto::Text(op) => self.text(op),
                    RingOpDto::SetFit(_) | RingOpDto::SetChannel(_) => {}
                },
                MutationDto::Path(op) => match op {
                    PathOpDto::SetColor(color) => self.color_opt(color),
                    PathOpDto::SetFit(_)
                    | PathOpDto::SetSpacing(_)
                    | PathOpDto::SetSize(_)
                    | PathOpDto::SetContent(_)
                    | PathOpDto::SetAnchors(_, _)
                    | PathOpDto::SetControlPoints(_)
                    | PathOpDto::MoveControlPoint { index: _, to: _ }
                    | PathOpDto::SetVisible(_)
                    | PathOpDto::SetZoom(_) => {}
                },
                MutationDto::Cap(op) => self.text(op),
                MutationDto::Label(op) => match op {
                    LabelOpDto::Text(op) => self.text(op),
                    LabelOpDto::SetT(_) | LabelOpDto::SetOffset(_) => {}
                },
                MutationDto::PortalEnd(op) => match op {
                    PortalEndOpDto::SetContent(Some(marker)) if !marker.is_empty() => {
                        self.shows.marks_portal_ends = true
                    }
                    PortalEndOpDto::SetColor(color) => self.color_opt(color),
                    PortalEndOpDto::SetContent(_)
                    | PortalEndOpDto::SetBorderT(_)
                    | PortalEndOpDto::SetOffset(_)
                    | PortalEndOpDto::SetSize(_) => {}
                },
            }
        }
    }

    fn node(&mut self, op: &'a NodeOpDto) {
        match op {
            NodeOpDto::SetClick(verb) => verb
                .iter()
                .for_each(|verb| self.name(Named::Verb, "set_click", verb)),
            NodeOpDto::SetHover(verb) => verb
                .iter()
                .for_each(|verb| self.name(Named::Verb, "set_hover", verb)),
            NodeOpDto::SetPalette(palette) => palette
                .iter()
                .for_each(|palette| self.name(Named::Palette, "set_palette", palette)),
            NodeOpDto::SetHidden(hidden) => self.shows.reveals |= !hidden,
            NodeOpDto::Text(op) => self.text(op),
            NodeOpDto::SetPosition(_)
            | NodeOpDto::Nudge(_)
            | NodeOpDto::SetMinSize(_)
            | NodeOpDto::SetShape(_)
            | NodeOpDto::SetZoom(_)
            | NodeOpDto::SetFolded(_)
            | NodeOpDto::ToggleFolded
            | NodeOpDto::SetOpacity(_)
            | NodeOpDto::SetChannel(_) => {}
        }
    }
}

impl<'a> Walk<'a, '_> {
    /// A text style op: its color's variable, if it writes one. It shows
    /// nothing; the others are listed so that one added that names
    /// something fails to compile.
    fn text(&mut self, op: &'a TextOpDto) {
        match op {
            TextOpDto::SetColor(color) => self.color_opt(color),
            TextOpDto::SetFont(_)
            | TextOpDto::SetSize(_)
            | TextOpDto::GrowSize(_)
            | TextOpDto::ScaleSize(_)
            | TextOpDto::SetWeight(_)
            | TextOpDto::SetItalic(_) => {}
        }
    }

    /// A ring layout op: its sides' color's variable, if it writes one.
    /// It shows nothing; listed as [`Self::text`] is.
    fn layout(&mut self, op: &'a LayoutOpDto) {
        match op {
            LayoutOpDto::SetSides(Some(SidesDto::Object(SidesObjectDto { content: _, color }))) => {
                self.color_opt(color)
            }
            LayoutOpDto::SetSides(Some(SidesDto::Off | SidesDto::Text(_)) | None)
            | LayoutOpDto::SetSpan(_)
            | LayoutOpDto::SetSpacing(_) => {}
        }
    }
}
