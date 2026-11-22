// SPDX-License-Identifier: MPL-2.0

//! What an edit says about the rest of the map, anywhere in it (its
//! filter or its body): the verbs, theme variables and palettes it
//! names, and whether it can make drawn what a file saves undrawn.
//! `maptool verify` reads it, so that a verb an edit names is not
//! reported as unreferenced, a node or portal an edit can show is not
//! reported as never shown, and a name an edit writes that its map does
//! not define is reported ([`Mention::is_defined_in`]). The same walk
//! finds the font families an edit writes ([`EditDef::fonts`]), which
//! the missing-font report reads.
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
use crate::document::format::error::{render_path, Seg};
use crate::document::format::spelling::{ColorDto, SidesDto, SidesObjectDto};
use crate::document::ids::Document;
use crate::document::presentation::PresentError;

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
/// map must define for a run to resolve it. Fonts are not here: a font
/// is not a name a map defines, and the loader's font rules decide it
/// (`format/fonts.md`); [`EditDef::fonts`] reports the families a
/// definition writes.
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
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mention<'a> {
    /// What the name names.
    pub kind: Named,
    /// The field that writes it, as a refusal of a run names it
    /// ([`EditError::UnknownName`]): the key the file writes, `set_click`,
    /// `set_color`, `background`, `palette`, ….
    pub field: &'static str,
    /// The name.
    pub name: &'a str,
    /// Where in the definition it is written, as the loader renders a
    /// JSON path: `mutations[1].node.text.set_color`,
    /// `filter.fields[0][0].background`,
    /// `mutator.void.children[0].macro.mutations[0].node.set_click`. So
    /// two mentions of one name in one definition can be told apart.
    pub path: String,
}

impl Mention<'_> {
    /// Whether `doc` defines the name: what the run of an edit asks of
    /// the open map, by the same probes (`resolve`'s `verb_of`, `var_of`
    /// and `palette_of`), so a name this answers `false` for is one a
    /// run is refused for ([`EditError::UnknownName`]), and one it
    /// answers `true` for is not. A map's verbs, theme variables and
    /// palettes are the file's own and change only by loading another:
    /// no operation adds, removes or renames one.
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

    /// The fault resolving this name against a map that lacks it is: the
    /// part a run's refusal shares whoever defines the name.
    pub(crate) fn fault(&self) -> Fault {
        Fault::Unknown {
            field: self.field,
            kind: match self.kind {
                Named::Verb => NameKind::Verb,
                Named::Variable => NameKind::Variable,
                Named::Palette => NameKind::Palette,
            },
            name: self.name.to_string(),
        }
    }

    /// The refusal a run of the edit `edit` earns for this name when the
    /// map does not define it, to the word: `fault` worded as an
    /// edit's refusal is.
    ///
    /// Cost: a few small allocations.
    pub fn refusal(&self, edit: &str) -> EditError {
        self.fault().into_error(edit)
    }

    /// The refusal the start of the presentation `presentation` earns for
    /// this name when the map does not define it, to the word:
    /// `fault` worded as a presentation's refusal is.
    ///
    /// Cost: a few small allocations.
    pub fn presentation_refusal(&self, presentation: &str) -> PresentError {
        PresentError::of(self.fault(), presentation)
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
    /// stops at, in the order the definition is walked: its filter, then
    /// its mutations in order, or its mutator's nodes in pre-order, each
    /// node's macro before its children.
    ///
    /// Cost: one walk of the definition, one call of `mention` per name
    /// it writes, and the allocation of that name's path.
    pub fn mentions<'a>(&'a self, mut mention: impl FnMut(Mention<'a>)) -> Shows {
        self.walk(Walk::new(Some(&mut mention), &mut |_| {}))
    }

    /// Calls `font` with each font family name this edit writes — a text
    /// op's `set_font`, a filter's `font` field — wherever in it, in the
    /// order [`Self::mentions`] walks: what tells the missing-font report
    /// which definitions name a family a replacement does not rewrite
    /// (#329). A family is not a [`Mention`]: any valid name loads, as a
    /// compiled or an installed family (`format/fonts.md`).
    ///
    /// Cost: one walk of the definition, one call of `font` per name, and
    /// no allocation.
    pub fn fonts<'a>(&'a self, mut font: impl FnMut(&'a str)) {
        self.walk(Walk::new(None, &mut font));
    }

    /// `walk` over this edit's filter and body.
    fn walk<'a>(&'a self, mut walk: Walk<'a, '_>) -> Shows {
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
        if let Some(Filter(filter)) = filter {
            walk.filter(filter);
        }
        match body {
            EditBody::Mutations(Mutations(mutations)) => walk.mutations(mutations),
            EditBody::Mutator(MutatorAst(mutator)) => walk.at(&[Seg::Key("mutator")], |w| w.mutator(mutator)),
            EditBody::Handler(_) => {}
        }
        walk.shows
    }
}

/// One walk of a definition — an edit, or a presentation, whose filter
/// and mutations are spelled as an edit's are: where the names it writes
/// go, where in the definition the walk is, and what it has found it can
/// show.
pub(in crate::document) struct Walk<'a, 'f> {
    /// Where each name goes; `None` for a walk that wants only fonts,
    /// which then renders no path.
    mention: Option<&'f mut dyn FnMut(Mention<'a>)>,
    /// Where each font family name goes.
    font: &'f mut dyn FnMut(&'a str),
    path: Vec<Seg>,
    pub(in crate::document) shows: Shows,
}

impl<'a, 'f> Walk<'a, 'f> {
    /// A walk that hands each name it meets to `mention`, if there is
    /// one, and each font family name to `font`, having found nothing
    /// yet.
    pub(in crate::document) fn new(
        mention: Option<&'f mut dyn FnMut(Mention<'a>)>,
        font: &'f mut dyn FnMut(&'a str),
    ) -> Self {
        Walk {
            mention,
            font,
            path: Vec::new(),
            shows: Shows::default(),
        }
    }

    /// `f`, with the walk `segs` deeper in the definition.
    fn at<R>(&mut self, segs: &[Seg], f: impl FnOnce(&mut Self) -> R) -> R {
        let depth = self.path.len();
        self.path.extend_from_slice(segs);
        let out = f(self);
        self.path.truncate(depth);
        out
    }

    /// Reports `name`, a `kind` that the key `leaf` below the walk writes
    /// and a refusal calls `field`.
    fn name(&mut self, kind: Named, field: &'static str, leaf: &'static str, name: &'a str) {
        if self.mention.is_none() {
            return;
        }
        let path = self.at(&[Seg::Key(leaf)], |w| render_path(&w.path));
        let Some(mention) = self.mention.as_mut() else {
            return;
        };
        mention(Mention {
            kind,
            field,
            name,
            path,
        });
    }

    /// A color the key `field` writes: its variable, if it is one. A
    /// palette color is a field of the owning node's palette, not a name.
    fn color(&mut self, color: &'a ColorDto<'_>, field: &'static str) {
        self.color_in(color, field, field);
    }

    /// [`Self::color`], written at the key `leaf` below the walk.
    fn color_in(&mut self, color: &'a ColorDto<'_>, field: &'static str, leaf: &'static str) {
        match color {
            ColorDto::Var(name) => self.name(Named::Variable, field, leaf, name),
            ColorDto::Hex(_) | ColorDto::Palette(_) => {}
        }
    }

    /// An optional color, a style op's or a path's: `null` clears it.
    fn color_opt(&mut self, color: &'a Option<ColorDto<'_>>, field: &'static str) {
        if let Some(color) = color {
            self.color(color, field);
        }
    }

    fn mutator(&mut self, mutator: &'a MutatorDto) {
        match mutator {
            MutatorDto::Void {
                kind: _,
                channel: _,
                children,
            } => self.at(&[Seg::Key("void")], |w| w.children(children)),
            MutatorDto::Macro {
                kind: _,
                channel: _,
                mutations,
                children,
            } => self.at(&[Seg::Key("macro")], |w| {
                w.mutations(mutations);
                w.children(children);
            }),
            MutatorDto::Instruction {
                kind: _,
                channel: _,
                instruction,
                children,
            } => self.at(&[Seg::Key("instruction")], |w| {
                match instruction {
                    InstructionDto::RepeatWhile(filter) => w
                        .at(&[Seg::Key("instruction"), Seg::Key("repeat_while")], |w| {
                            w.fields(filter)
                        }),
                    InstructionDto::RepeatWhileAlwaysTrue => {}
                }
                w.children(children);
            }),
        }
    }

    fn children(&mut self, children: &'a [MutatorDto]) {
        for (i, child) in children.iter().enumerate() {
            self.at(&[Seg::Key("children"), Seg::Index(i)], |w| w.mutator(child));
        }
    }

    /// A definition's own `filter`.
    pub(in crate::document) fn filter(&mut self, filter: &'a FilterDto) {
        self.at(&[Seg::Key("filter")], |w| w.fields(filter));
    }

    /// The tests of a filter, wherever it is written.
    fn fields(&mut self, filter: &'a FilterDto) {
        let FilterDto {
            fields,
            always_match: _,
        } = filter;
        for (i, (field, _)) in fields.iter().enumerate() {
            // A test is `[field, comparator]`: the field is the first.
            self.at(&[Seg::Key("fields"), Seg::Index(i), Seg::Index(0)], |w| {
                w.field(field)
            });
        }
    }

    fn field(&mut self, field: &'a FieldDto) {
        match field {
            FieldDto::OnClick(verb) => self.name(Named::Verb, "on_click", "on_click", verb),
            FieldDto::OnHover(verb) => self.name(Named::Verb, "on_hover", "on_hover", verb),
            FieldDto::Palette(palette) => self.name(Named::Palette, "palette", "palette", palette),
            FieldDto::Background(color) => self.color(color, "background"),
            FieldDto::Color(color) => self.color(color, "color"),
            FieldDto::Font(family) => (self.font)(family),
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

    /// A definition's own `mutations`.
    pub(in crate::document) fn mutations(&mut self, mutations: &'a [MutationDto]) {
        for (i, mutation) in mutations.iter().enumerate() {
            self.at(&[Seg::Key("mutations"), Seg::Index(i)], |w| w.mutation(mutation));
        }
    }

    fn mutation(&mut self, mutation: &'a MutationDto) {
        match mutation {
            MutationDto::Node(op) => self.at(&[Seg::Key("node")], |w| w.node(op)),
            MutationDto::Section(op) => self.at(&[Seg::Key("section")], |w| match op {
                SectionOpDto::Text(op) => w.text_at(op),
                SectionOpDto::SetOffset(_)
                | SectionOpDto::Nudge(_)
                | SectionOpDto::SetMinSize(_)
                | SectionOpDto::SetAlign(_)
                | SectionOpDto::SetChannel(_) => {}
            }),
            MutationDto::Span(op) => self.at(&[Seg::Key("span")], |w| match op {
                SpanOpDto::SetLink(Some(LinkTargetDto::Verb(verb))) => {
                    // `{"set_link": {"verb": "go"}}`.
                    w.at(&[Seg::Key("set_link")], |w| {
                        w.name(Named::Verb, "set_link", "verb", verb)
                    })
                }
                SpanOpDto::SetLink(Some(LinkTargetDto::Url(_)) | None) => {}
                SpanOpDto::Text(op) => w.text_at(op),
                SpanOpDto::SetText(_)
                | SpanOpDto::InsertText { at: _, text: _ }
                | SpanOpDto::DeleteText { start: _, end: _ } => {}
            }),
            MutationDto::Frame(op) => self.at(&[Seg::Key("frame")], |w| match op {
                FrameOpDto::Layout(op) => w.layout_at(op),
                FrameOpDto::Text(op) => w.text_at(op),
                FrameOpDto::SetVisible(_)
                | FrameOpDto::SetStart(_)
                | FrameOpDto::SetDirection(_)
                | FrameOpDto::SetFit(_)
                | FrameOpDto::SetGap(_)
                | FrameOpDto::SetInset(_) => {}
            }),
            MutationDto::Ring(op) => self.at(&[Seg::Key("ring")], |w| match op {
                RingOpDto::Layout(op) => w.layout_at(op),
                RingOpDto::Text(op) => w.text_at(op),
                RingOpDto::SetFit(_) | RingOpDto::SetChannel(_) => {}
            }),
            MutationDto::Path(op) => self.at(&[Seg::Key("path")], |w| match op {
                PathOpDto::SetColor(color) => w.color_opt(color, "set_color"),
                PathOpDto::SetFit(_)
                | PathOpDto::SetSpacing(_)
                | PathOpDto::SetSize(_)
                | PathOpDto::SetContent(_)
                | PathOpDto::SetAnchors(_, _)
                | PathOpDto::SetControlPoints(_)
                | PathOpDto::MoveControlPoint { index: _, to: _ }
                | PathOpDto::SetVisible(_)
                | PathOpDto::SetZoom(_) => {}
            }),
            // A cap's ops are a text style's, written directly.
            MutationDto::Cap(op) => self.at(&[Seg::Key("cap")], |w| w.text(op)),
            MutationDto::Label(op) => self.at(&[Seg::Key("label")], |w| match op {
                LabelOpDto::Text(op) => w.text_at(op),
                LabelOpDto::SetT(_) | LabelOpDto::SetOffset(_) => {}
            }),
            MutationDto::PortalEnd(op) => self.at(&[Seg::Key("portal_end")], |w| match op {
                PortalEndOpDto::SetContent(Some(marker)) if !marker.is_empty() => {
                    w.shows.marks_portal_ends = true
                }
                PortalEndOpDto::SetColor(color) => w.color_opt(color, "set_color"),
                PortalEndOpDto::SetContent(_)
                | PortalEndOpDto::SetBorderT(_)
                | PortalEndOpDto::SetOffset(_)
                | PortalEndOpDto::SetSize(_) => {}
            }),
        }
    }

    fn node(&mut self, op: &'a NodeOpDto) {
        match op {
            NodeOpDto::SetClick(verb) => verb
                .iter()
                .for_each(|verb| self.name(Named::Verb, "set_click", "set_click", verb)),
            NodeOpDto::SetHover(verb) => verb
                .iter()
                .for_each(|verb| self.name(Named::Verb, "set_hover", "set_hover", verb)),
            NodeOpDto::SetPalette(palette) => palette
                .iter()
                .for_each(|palette| self.name(Named::Palette, "set_palette", "set_palette", palette)),
            NodeOpDto::SetHidden(hidden) => self.shows.reveals |= !hidden,
            NodeOpDto::Text(op) => self.text_at(op),
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

    /// A `text` op: the key, then the op.
    fn text_at(&mut self, op: &'a TextOpDto) {
        self.at(&[Seg::Key("text")], |w| w.text(op));
    }

    /// A text style op: its color's variable, if it writes one, and its
    /// `set_font`'s family. It shows nothing; the others are listed so
    /// that one added that names something fails to compile.
    fn text(&mut self, op: &'a TextOpDto) {
        match op {
            TextOpDto::SetColor(color) => self.color_opt(color, "set_color"),
            TextOpDto::SetFont(Some(family)) => (self.font)(family),
            TextOpDto::SetFont(None)
            | TextOpDto::SetSize(_)
            | TextOpDto::GrowSize(_)
            | TextOpDto::ScaleSize(_)
            | TextOpDto::SetWeight(_)
            | TextOpDto::SetItalic(_) => {}
        }
    }

    /// A `layout` op: the key, then the op.
    fn layout_at(&mut self, op: &'a LayoutOpDto) {
        self.at(&[Seg::Key("layout")], |w| w.layout(op));
    }

    /// A ring layout op: its sides' color's variable, if it writes one
    /// (`{"set_sides": {"content": …, "color": "var(x)"}}`, which a
    /// refusal calls `set_sides`). It shows nothing; listed as
    /// [`Self::text`] is.
    fn layout(&mut self, op: &'a LayoutOpDto) {
        match op {
            LayoutOpDto::SetSides(Some(SidesDto::Object(SidesObjectDto { content: _, color }))) => {
                if let Some(color) = color {
                    self.at(&[Seg::Key("set_sides")], |w| {
                        w.color_in(color, "set_sides", "color")
                    });
                }
            }
            LayoutOpDto::SetSides(Some(SidesDto::Off | SidesDto::Text(_)) | None)
            | LayoutOpDto::SetSpan(_)
            | LayoutOpDto::SetSpacing(_) => {}
        }
    }
}
