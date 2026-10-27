// SPDX-License-Identifier: MPL-2.0

//! The saver: a [`Document`] becomes
//! the DTOs of its file, and serde writes them.
//!
//! **Deterministic.** The DTO fields are declared in schema order, so
//! serde writes the keys in that order; name-keyed objects (theme
//! variants and variables, palettes) are written in ascending byte order
//! of the name; nodes are written in sibling order, which is the order
//! of the arena (a row is kept sorted by kind and channel); numbers are
//! serde_json's shortest round-trip form of the stored `f32`. A key
//! equal to its inherited `None`, a default value, an empty array, a
//! child node's default connector, a portal link's default ends and an
//! empty theme are left out. Nothing derived is written: no fitted size,
//! no depth, no resolved color.
//!
//! **Names, never indices.** A theme variable, a palette, a font and a
//! verb are written by name, from the document's own tables, so a saved
//! file does not depend on how the indices were assigned.
//!
//! **Nothing is dropped, and nothing is written that does not load.**
//! The document can hold, through the test builder or a mutation that
//! does not check, things the file has no spelling for: a size in the
//! wrong unit (points on a ring, a fraction on a section), the fallback
//! face as an explicit font, an index into a table that has no such
//! entry, two palettes of one name, a portal link with one end. Writing
//! any of them would give a file that does not load, or loads as
//! something else (a fraction `0.5` read back as `0.5` points would
//! retarget silently). So the saver refuses with a [`SaveError`] naming
//! the path the value would have in the file, and writes nothing. After
//! the walk, the DTOs it built are built into a document by the loader's
//! own builder, so a number outside its bound is refused here, at the
//! same path, as the loader would refuse it.

use crate::document::defaults::{FIT_MAX, FIT_MIN};
use crate::document::element::{
    CapData, DefaultsData, DocElement, FlowStyle, FrameStyle, LabelStyle, NodeStyle, PathData, PortalEndData,
    PortalEndStyle, RingStyle, TextStyle,
};
use crate::document::format::context::tree_excerpt;
use crate::document::format::dto::{
    AlignDto, AnchorDto, CapDto, DefaultsDto, DirectionDto, EditDto, FlowDefaultsDto, FrameDefaultsDto,
    FrameDto, LabelDefaultsDto, LabelDto, LinkDto, LinkKindDto, LinkTargetDto, MapHeaderDto, NodeDefaultsDto,
    NodeDto, OnDto, PaletteGroupDto, PathDto, PortalEndDefaultsDto, PortalEndDto, PresentationDto,
    RingDefaultsDto, RingDto, SectionDto, SpanDto, TextDefaultsDto, ThemeDto, VerbDto, ZoomDto,
};
use crate::document::format::error::{render_path, NodeContext, SaveError, SaveErrorKind, Seg};
use crate::document::format::index::FORMAT;
use crate::document::format::spelling::{
    ColorDto, EdgeDto, FitDto, FitOnceDto, FitRangeDto, HexDto, Named, PaletteFieldDto, RectangleDto,
    ShapeDto, StartDto, StartPointDto, WeightDto,
};
use crate::document::format::SizeUnit;
use crate::document::ids::{Document, VerbDef};
use crate::document::values::{
    Align, Anchor, ColorRef, Direction, Edge, FitMode, FontSize, LinkKind, LinkTarget, PaletteField,
    PlainColor, Shape, ZoomWindow,
};
use crate::font::fonts::{family_name_of, AppFont};
use indextree::NodeId;
use rustc_hash::{FxHashMap, FxHashSet};

/// What every step of the saver returns.
type SaveResult<T> = Result<T, SaveError>;

/// The five text-style keys of one element.
struct TextOut {
    font: Option<String>,
    size: Option<f32>,
    color: Option<ColorDto>,
    weight: Option<WeightDto>,
    italic: Option<bool>,
}

/// The state of one save.
struct Saver<'a> {
    doc: &'a Document,
    /// The theme's variable names, in the order a `VarId` indexes.
    vars: &'a [String],
    /// The palette names, in the order a `PaletteId` indexes.
    palettes: Vec<&'a str>,
    /// The verbs, in the order a `VerbId` indexes.
    verbs: &'a [VerbDef],
    /// The family name of each face already looked up.
    fonts: FxHashMap<AppFont, Option<&'static str>>,
    /// Where in the file the saver is, as the loader would say it.
    path: Vec<Seg>,
}

/// A saver for `doc`, at the top of the file.
fn saver(doc: &Document) -> Saver<'_> {
    Saver {
        doc,
        vars: &doc.theme().variables,
        palettes: doc.palettes().map(|(_, palette)| palette.name.as_str()).collect(),
        verbs: &doc.definitions().verbs,
        fonts: FxHashMap::default(),
        path: Vec::new(),
    }
}

/// The header of `doc`'s file — every top-level key but `nodes` — or the
/// first thing in it the file cannot spell. O(header).
pub(super) fn header_of(doc: &Document) -> SaveResult<MapHeaderDto> {
    saver(doc).map()
}

/// Run `each` on every node of `doc` in the order the file lists them —
/// the depth-first pre-order of the tree, siblings in the order the
/// arena keeps them — with its position in the list, its element, and
/// its DTO, which names its parent. One DTO exists at a time; `each`
/// gets it by value and drops it. The first thing a node holds that the
/// file cannot spell stops the walk, with the node's context.
///
/// The walk is the arena's own pre-order (`Tree::descendants`), which
/// follows parent and sibling links: no stack, no recursion, every
/// element visited once, so O(elements) whatever the depth. A node's
/// own content is turned into its DTO by a walk bounded by the schema
/// (a node's content nests at most 9 levels), never by the tree.
pub(super) fn each_node(
    doc: &Document,
    mut each: impl FnMut(usize, NodeId, NodeDto) -> SaveResult<()>,
) -> SaveResult<()> {
    let mut saver = saver(doc);
    let tree = doc.tree();
    let mut index = 0;
    for id in tree.descendants() {
        let Some(DocElement::Node(node)) = doc.get(id) else {
            continue;
        };
        let parent = doc.parent(id).and_then(|parent| match doc.get(parent) {
            Some(DocElement::Node(parent)) => Some(u64::from(parent.ident.0)),
            _ => None,
        });
        saver.path.clear();
        saver.path.extend([Seg::Key("nodes"), Seg::Index(index)]);
        let dto = saver.node(id, parent).map_err(|error| SaveError {
            node: Some(NodeContext {
                index,
                id: Some(node.ident.0),
                excerpt: tree_excerpt(tree, id),
            }),
            ..error
        })?;
        each(index, id, dto)?;
        index += 1;
    }
    Ok(())
}

impl<'a> Saver<'a> {
    /// An error of `kind` at the current path.
    fn error(&self, kind: SaveErrorKind) -> SaveError {
        SaveError::new(render_path(&self.path), kind)
    }

    /// An error of `kind` at the current path and then `key`.
    fn error_at(&self, key: &'static str, kind: SaveErrorKind) -> SaveError {
        let mut path = self.path.clone();
        path.push(Seg::Key(key));
        SaveError::new(render_path(&path), kind)
    }

    /// Run `f` one segment deeper.
    fn at<T>(&mut self, seg: Seg, f: impl FnOnce(&mut Self) -> SaveResult<T>) -> SaveResult<T> {
        self.path.push(seg);
        let out = f(self);
        self.path.pop();
        out
    }

    /// Run `f` inside the key `key`.
    fn key<T>(&mut self, key: &'static str, f: impl FnOnce(&mut Self) -> SaveResult<T>) -> SaveResult<T> {
        self.at(Seg::Key(key), f)
    }

    /// The header: every top-level key but `nodes`.
    fn map(&mut self) -> SaveResult<MapHeaderDto> {
        let doc = self.doc;
        let name = match doc.get(doc.root()) {
            Some(DocElement::Map(map)) => map.name.clone(),
            _ => String::new(),
        };
        let definitions = doc.definitions();
        let theme = self.theme()?;
        let palettes = self.palettes()?;
        let defaults = self.key("defaults", |s| s.defaults(doc.defaults()))?;
        let edits = self.definitions("edits", definitions.edits.iter().map(|d| (&d.id, &d.name)))?;
        let presentations = self.definitions(
            "presentations",
            definitions.presentations.iter().map(|d| (&d.id, &d.name)),
        )?;
        let verbs = self.definitions("verbs", definitions.verbs.iter().map(|d| (&d.id, &d.name)))?;
        Ok(MapHeaderDto {
            format: FORMAT,
            name,
            theme,
            palettes,
            defaults,
            edits: edits.into_iter().map(|(id, name)| EditDto { id, name }).collect(),
            presentations: presentations
                .into_iter()
                .map(|(id, name)| PresentationDto { id, name })
                .collect(),
            verbs: verbs.into_iter().map(|(id, name)| VerbDto { id, name }).collect(),
        })
    }

    /// The entries of `edits`, `presentations` or `verbs` as (id, name)
    /// pairs, refusing an id that two entries share.
    fn definitions<'d>(
        &mut self,
        key: &'static str,
        entries: impl Iterator<Item = (&'d String, &'d String)>,
    ) -> SaveResult<Vec<(String, String)>> {
        let mut seen: FxHashSet<&str> = FxHashSet::default();
        let mut out = Vec::new();
        self.key(key, |s| {
            for (index, (id, name)) in entries.enumerate() {
                if !seen.insert(id.as_str()) {
                    return s.at(Seg::Index(index), |s| {
                        Err(s.error_at("id", SaveErrorKind::DuplicateDefinition { id: id.clone() }))
                    });
                }
                out.push((id.clone(), name.clone()));
            }
            Ok(())
        })?;
        Ok(out)
    }

    /// The theme: variants and variables in ascending name order, or
    /// `None` for the empty theme, which has exactly one spelling: no
    /// `theme` key.
    fn theme(&mut self) -> SaveResult<Option<ThemeDto>> {
        let theme = self.doc.theme();
        if theme.variants.is_empty() {
            return if theme.variables.is_empty() && theme.active == 0 {
                Ok(None)
            } else {
                self.key("theme", |s| {
                    Err(s.error(SaveErrorKind::ThemeWithoutVariants {
                        variables: theme.variables.len(),
                        active: theme.active,
                    }))
                })
            };
        }
        self.key("theme", |s| {
            let mut variants: Vec<(String, Named<HexDto>)> = Vec::with_capacity(theme.variants.len());
            for variant in &theme.variants {
                if variant.values.len() != theme.variables.len() {
                    return s.key("variants", |s| {
                        s.at(Seg::Name(variant.name.clone()), |s| {
                            Err(s.error(SaveErrorKind::VariantLength {
                                values: variant.values.len(),
                                variables: theme.variables.len(),
                            }))
                        })
                    });
                }
                let mut values: Vec<(String, HexDto)> = theme
                    .variables
                    .iter()
                    .zip(&variant.values)
                    .map(|(name, rgba)| (name.clone(), HexDto(rgba.0)))
                    .collect();
                values.sort_by(|a, b| a.0.cmp(&b.0));
                if let Some(pair) = values.windows(2).find(|pair| pair[0].0 == pair[1].0) {
                    let name = pair[1].0.clone();
                    return s.key("variants", |s| {
                        s.at(Seg::Name(variant.name.clone()), |s| {
                            s.at(Seg::Name(name.clone()), |s| {
                                Err(s.error(SaveErrorKind::DuplicateName { name }))
                            })
                        })
                    });
                }
                variants.push((variant.name.clone(), Named(values)));
            }
            let active = match theme.variants.get(usize::from(theme.active)) {
                Some(variant) => variant.name.clone(),
                None => {
                    return Err(s.error_at(
                        "active",
                        SaveErrorKind::ActiveNotAVariant {
                            active: theme.active,
                            variants: theme.variants.len(),
                        },
                    ))
                }
            };
            variants.sort_by(|a, b| a.0.cmp(&b.0));
            if let Some(pair) = variants.windows(2).find(|pair| pair[0].0 == pair[1].0) {
                let name = pair[1].0.clone();
                return s.key("variants", |s| {
                    s.at(Seg::Name(name.clone()), |s| {
                        Err(s.error(SaveErrorKind::DuplicateName { name }))
                    })
                });
            }
            Ok(Some(ThemeDto {
                active,
                variants: Named(variants),
            }))
        })
    }

    /// The palettes, by name.
    fn palettes(&mut self) -> SaveResult<Named<Vec<PaletteGroupDto>>> {
        let doc = self.doc;
        let mut palettes: Vec<(String, Vec<PaletteGroupDto>)> = Vec::new();
        self.key("palettes", |s| {
            for (_, palette) in doc.palettes() {
                let groups = s.at(Seg::Name(palette.name.clone()), |s| {
                    let mut groups = Vec::with_capacity(palette.groups.len());
                    for (index, group) in palette.groups.iter().enumerate() {
                        groups.push(s.at(Seg::Index(index), |s| {
                            Ok(PaletteGroupDto {
                                fill: s.plain("fill", group.fill)?,
                                frame: s.plain("frame", group.frame)?,
                                text: s.plain("text", group.text)?,
                                title: group.title.map(|title| s.plain("title", title)).transpose()?,
                            })
                        })?);
                    }
                    Ok(groups)
                })?;
                palettes.push((palette.name.clone(), groups));
            }
            palettes.sort_by(|a, b| a.0.cmp(&b.0));
            if let Some(pair) = palettes.windows(2).find(|pair| pair[0].0 == pair[1].0) {
                let name = pair[1].0.clone();
                return s.at(Seg::Name(name.clone()), |s| {
                    Err(s.error(SaveErrorKind::DuplicateName { name }))
                });
            }
            Ok(())
        })?;
        Ok(Named(palettes))
    }

    /// A palette color. A palette group has all of its colors, so one
    /// naming a variable the theme lacks has no spelling.
    fn plain(&self, key: &'static str, color: PlainColor) -> SaveResult<ColorDto> {
        Ok(match color {
            PlainColor::Rgba(rgba) => ColorDto::Hex(HexDto(rgba.0)),
            PlainColor::Var(var) => ColorDto::Var(self.var_name(key, var.0)?),
        })
    }

    /// A theme variable's name, or an error at `key` when the theme
    /// defines no such variable.
    fn var_name(&self, key: &'static str, index: u16) -> SaveResult<String> {
        self.vars
            .get(usize::from(index))
            .cloned()
            .ok_or_else(|| self.error_at(key, SaveErrorKind::UnknownVariable { index }))
    }

    /// A color as the file spells it; `None` for an unset color.
    fn color(&self, key: &'static str, color: Option<ColorRef>) -> SaveResult<Option<ColorDto>> {
        let Some(color) = color else {
            return Ok(None);
        };
        Ok(Some(match color {
            ColorRef::Rgba(rgba) => ColorDto::Hex(HexDto(rgba.0)),
            ColorRef::Var(var) => ColorDto::Var(self.var_name(key, var.0)?),
            ColorRef::Palette(field) => ColorDto::Palette(match field {
                PaletteField::Fill => PaletteFieldDto::Fill,
                PaletteField::Frame => PaletteFieldDto::Frame,
                PaletteField::Text => PaletteFieldDto::Text,
                PaletteField::Title => PaletteFieldDto::Title,
            }),
        }))
    }

    /// A font's family name. The fallback face has none, and neither has
    /// a face this build cannot name: the file could not say which it
    /// was.
    fn font(&mut self, font: Option<AppFont>) -> SaveResult<Option<String>> {
        let Some(font) = font else {
            return Ok(None);
        };
        let name = *self.fonts.entry(font).or_insert_with(|| {
            if font == AppFont::Any {
                None
            } else {
                family_name_of(font)
            }
        });
        name.map(|name| Some(name.to_string()))
            .ok_or_else(|| self.error_at("font", SaveErrorKind::NoFamilyName))
    }

    /// A size in the unit this position takes; a size in the other unit
    /// would load as another size.
    fn size(&self, size: Option<FontSize>, unit: SizeUnit) -> SaveResult<Option<f32>> {
        match (size, unit) {
            (None, _) => Ok(None),
            (Some(FontSize::Pt(points)), SizeUnit::Pt) => Ok(Some(points)),
            (Some(FontSize::Rel(fraction)), SizeUnit::Rel) => Ok(Some(fraction)),
            (Some(FontSize::Pt(_)), SizeUnit::Rel) => {
                Err(self.error_at("size", SaveErrorKind::WrongUnit { found_points: true }))
            }
            (Some(FontSize::Rel(_)), SizeUnit::Pt) => {
                Err(self.error_at("size", SaveErrorKind::WrongUnit { found_points: false }))
            }
        }
    }

    /// The five text-style keys.
    fn text(&mut self, style: &TextStyle, unit: SizeUnit) -> SaveResult<TextOut> {
        Ok(TextOut {
            font: self.font(style.font)?,
            size: self.size(style.size, unit)?,
            color: self.color("color", style.color)?,
            weight: style.weight.map(|weight| WeightDto(weight.get())),
            italic: style.italic,
        })
    }

    /// A fit mode as the file spells it.
    fn fit(fit: &Option<FitMode>) -> Option<FitDto> {
        Some(match fit.as_ref()? {
            FitMode::Repeat => FitDto::Repeat,
            FitMode::Spread => FitDto::Spread,
            FitMode::Once { filler } => FitDto::Once(filler.as_ref().map(|filler| FitOnceDto {
                filler: filler.clone(),
            })),
            FitMode::Fit { min, max } => {
                if *min == FIT_MIN && *max == FIT_MAX {
                    FitDto::Fit(None)
                } else {
                    FitDto::Fit(Some(FitRangeDto { min: *min, max: *max }))
                }
            }
        })
    }

    /// A zoom range.
    fn zoom(zoom: Option<ZoomWindow>) -> Option<ZoomDto> {
        zoom.map(|zoom| ZoomDto {
            min: zoom.min,
            max: zoom.max,
        })
    }

    /// A verb's id, or an error at `key` when the map defines no such
    /// verb.
    fn verb_id(&self, key: &'static str, index: u16) -> SaveResult<String> {
        self.verbs
            .get(usize::from(index))
            .map(|verb| verb.id.clone())
            .ok_or_else(|| self.error_at(key, SaveErrorKind::UnknownVerb { index }))
    }

    /// The element at `id`.
    fn element(&self, id: NodeId) -> Option<&'a DocElement> {
        let doc: &'a Document = self.doc;
        doc.get(id)
    }

    /// The children of `id`, with their elements, in sibling order.
    fn children(&self, id: NodeId) -> Vec<(NodeId, &'a DocElement)> {
        let doc: &'a Document = self.doc;
        doc.children(id)
            .filter_map(|child| doc.get(child).map(|element| (child, element)))
            .collect()
    }

    /// One node, which hangs from the node with the ident `parent`, or
    /// is a top-level node. Its child nodes are not in it: the file lists
    /// them after it.
    fn node(&mut self, id: NodeId, parent: Option<u64>) -> SaveResult<NodeDto> {
        let Some(DocElement::Node(node)) = self.element(id) else {
            return Err(self.error(SaveErrorKind::WrongElement));
        };
        let text = self.text(&node.style.text, SizeUnit::Pt)?;
        let on = self.key("on", |s| {
            let on = OnDto {
                click: node.on.click.map(|verb| s.verb_id("click", verb.0)).transpose()?,
                hover: node.on.hover.map(|verb| s.verb_id("hover", verb.0)).transpose()?,
            };
            Ok(if on.click.is_some() || on.hover.is_some() {
                Some(on)
            } else {
                None
            })
        })?;
        let fill = self.color("fill", node.style.fill)?;
        let palette = match node.palette {
            Some(palette) => Some(
                self.palettes
                    .get(usize::from(palette.0))
                    .map(|name| (*name).to_string())
                    .ok_or_else(|| {
                        self.error_at("palette", SaveErrorKind::UnknownPalette { index: palette.0 })
                    })?,
            ),
            None => None,
        };
        // What lies under the node, in the order of the kinds.
        let (mut sections, mut frame, mut connector, mut links) = (Vec::new(), None, None, Vec::new());
        for (child, element) in self.children(id) {
            match element {
                DocElement::Section(_) => {
                    let index = sections.len();
                    let section = self.key("sections", |s| s.at(Seg::Index(index), |s| s.section(child)))?;
                    sections.push(section);
                }
                DocElement::Frame(_) => frame = Some(self.key("frame", |s| s.frame(child))?),
                DocElement::Connector(_) => connector = self.key("connector", |s| s.connector(child))?,
                DocElement::Link(_) => {
                    let index = links.len();
                    let link = self.key("links", |s| s.at(Seg::Index(index), |s| s.link(id, child)))?;
                    links.push(link);
                }
                // A child node is listed after this one.
                _ => {}
            }
        }
        Ok(NodeDto {
            id: u64::from(node.ident.0),
            parent,
            channel: channel_of(node.channel),
            position: node.position.to_array(),
            min_size: node.min_size.map(|size| size.to_array()),
            shape: node.style.shape.map(shape),
            fill,
            fill_fade: node.style.fill_fade,
            palette,
            zoom: Self::zoom(node.zoom),
            folded: node.folded,
            hidden: node.hidden,
            opacity: node.opacity,
            on,
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
            sections,
            frame,
            connector,
            links,
        })
    }

    /// One section.
    fn section(&mut self, id: NodeId) -> SaveResult<SectionDto> {
        let Some(DocElement::Section(section)) = self.element(id) else {
            return Err(self.error(SaveErrorKind::WrongElement));
        };
        let text = self.text(&section.text, SizeUnit::Pt)?;
        Ok(SectionDto {
            channel: channel_of(section.channel),
            offset: section.offset.to_array(),
            min_size: section.min_size.map(|size| size.to_array()),
            align: section.align.map(|align| match align {
                Align::Left => AlignDto::Left,
                Align::Center => AlignDto::Center,
                Align::Right => AlignDto::Right,
            }),
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
            spans: self.spans(id, false)?,
        })
    }

    /// The spans under `parent`, at the current path's `spans`. A span in
    /// a ring has no size of its own: the ring sizes its text, and a size
    /// on the span would be one the file cannot say.
    fn spans(&mut self, parent: NodeId, in_ring: bool) -> SaveResult<Vec<SpanDto>> {
        let mut out = Vec::new();
        for (_, element) in self.children(parent) {
            let DocElement::Span(span) = element else {
                continue;
            };
            let index = out.len();
            let dto = self.key("spans", |s| {
                s.at(Seg::Index(index), |s| {
                    if in_ring && span.style.size.is_some() {
                        return Err(s.error_at("size", SaveErrorKind::SizeInRing));
                    }
                    let text = s.text(&span.style, SizeUnit::Pt)?;
                    let link = match &span.link {
                        Some(LinkTarget::Url(url)) => Some(LinkTargetDto::Url(url.clone())),
                        Some(LinkTarget::Verb(verb)) => Some(LinkTargetDto::Verb(s.verb_id("link", verb.0)?)),
                        None => None,
                    };
                    Ok(SpanDto {
                        text: span.text.clone(),
                        font: text.font,
                        size: text.size,
                        color: text.color,
                        weight: text.weight,
                        italic: text.italic,
                        link,
                    })
                })
            })?;
            out.push(dto);
        }
        Ok(out)
    }

    /// A frame and its rings.
    fn frame(&mut self, id: NodeId) -> SaveResult<FrameDto> {
        let Some(DocElement::Frame(frame)) = self.element(id) else {
            return Err(self.error(SaveErrorKind::WrongElement));
        };
        let style = self.frame_style(&frame.style)?;
        let mut rings = Vec::new();
        for (ring_id, element) in self.children(id) {
            let DocElement::Ring(ring) = element else {
                continue;
            };
            let index = rings.len();
            let dto = self.key("rings", |s| {
                s.at(Seg::Index(index), |s| {
                    let text = s.text(&ring.style.text, SizeUnit::Rel)?;
                    Ok(RingDto {
                        channel: channel_of(ring.channel),
                        fit: Self::fit(&ring.style.fit),
                        font: text.font,
                        size: text.size,
                        color: text.color,
                        weight: text.weight,
                        italic: text.italic,
                        spans: s.spans(ring_id, true)?,
                    })
                })
            })?;
            rings.push(dto);
        }
        Ok(FrameDto {
            visible: style.visible,
            start: style.start,
            direction: style.direction,
            fit: style.fit,
            gap: style.gap,
            inset: style.inset,
            font: style.text.font,
            size: style.text.size,
            color: style.text.color,
            weight: style.text.weight,
            italic: style.text.italic,
            rings,
        })
    }

    /// A frame's style keys.
    fn frame_style(&mut self, style: &FrameStyle) -> SaveResult<FrameOut> {
        Ok(FrameOut {
            visible: style.visible,
            start: style.start.map(|start| {
                StartDto(StartPointDto {
                    edge: match start.edge {
                        Edge::Top => EdgeDto::Top,
                        Edge::Right => EdgeDto::Right,
                        Edge::Bottom => EdgeDto::Bottom,
                        Edge::Left => EdgeDto::Left,
                    },
                    at: start.at,
                })
            }),
            direction: style.direction.map(|direction| match direction {
                Direction::Clockwise => DirectionDto::Clockwise,
                Direction::CounterClockwise => DirectionDto::Counterclockwise,
            }),
            fit: Self::fit(&style.fit),
            gap: style.gap,
            inset: style.inset,
            text: self.text(&style.text, SizeUnit::Rel)?,
        })
    }

    /// The connector of a node, or `None` when the file leaves it out:
    /// a connector with every field inherited and nothing under it.
    fn connector(&mut self, id: NodeId) -> SaveResult<Option<PathDto>> {
        let Some(DocElement::Connector(connector)) = self.element(id) else {
            return Err(self.error(SaveErrorKind::WrongElement));
        };
        let children = self.children(id);
        if connector.path == default_path() && children.is_empty() {
            return Ok(None);
        }
        let flow = self.flow(&connector.path.flow)?;
        let (spans, cap_start, cap_end, label) = self.on_path(id)?;
        Ok(Some(PathDto {
            fit: flow.fit,
            spacing: flow.spacing,
            size: flow.size,
            color: flow.color,
            content: flow.content,
            anchors: anchors(&connector.path),
            control_points: control_points(&connector.path),
            visible: (!connector.path.visible).then_some(false),
            zoom: Self::zoom(connector.path.zoom),
            spans,
            cap_start,
            cap_end,
            label,
        }))
    }

    /// A path's style keys.
    fn flow(&mut self, flow: &FlowStyle) -> SaveResult<FlowOut> {
        Ok(FlowOut {
            fit: Self::fit(&flow.fit),
            spacing: flow.spacing,
            size: flow.size,
            color: self.color("color", flow.color)?,
            content: flow.content.clone(),
        })
    }

    /// What lies on a path: its spans, caps and label.
    #[allow(clippy::type_complexity)]
    fn on_path(
        &mut self,
        id: NodeId,
    ) -> SaveResult<(Vec<SpanDto>, Option<CapDto>, Option<CapDto>, Option<LabelDto>)> {
        let spans = self.spans(id, false)?;
        let (mut cap_start, mut cap_end, mut label) = (None, None, None);
        for (child, element) in self.children(id) {
            match element {
                DocElement::CapStart(cap) => {
                    cap_start = Some(self.key("cap_start", |s| s.cap(child, cap))?);
                }
                DocElement::CapEnd(cap) => {
                    cap_end = Some(self.key("cap_end", |s| s.cap(child, cap))?);
                }
                DocElement::Label(_) => label = Some(self.key("label", |s| s.label(child))?),
                _ => {}
            }
        }
        Ok((spans, cap_start, cap_end, label))
    }

    /// A cap.
    fn cap(&mut self, id: NodeId, cap: &CapData) -> SaveResult<CapDto> {
        let text = self.text(&cap.text, SizeUnit::Pt)?;
        Ok(CapDto {
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
            spans: self.spans(id, false)?,
        })
    }

    /// A label.
    fn label(&mut self, id: NodeId) -> SaveResult<LabelDto> {
        let Some(DocElement::Label(label)) = self.element(id) else {
            return Err(self.error(SaveErrorKind::WrongElement));
        };
        let text = self.text(&label.style.text, SizeUnit::Pt)?;
        Ok(LabelDto {
            t: label.style.t,
            offset: label.style.offset,
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
            spans: self.spans(id, false)?,
        })
    }

    /// A link held by `owner`.
    fn link(&mut self, owner: NodeId, id: NodeId) -> SaveResult<LinkDto> {
        let Some(DocElement::Link(link)) = self.element(id) else {
            return Err(self.error(SaveErrorKind::WrongElement));
        };
        let to = match self.element(link.to) {
            Some(DocElement::Node(target)) => u64::from(target.ident.0),
            _ => return Err(self.error_at("to", SaveErrorKind::LinkToNothing)),
        };
        if link.to == owner {
            return Err(self.error_at("to", SaveErrorKind::SelfLink));
        }
        let mut dto = LinkDto {
            to,
            kind: match link.kind {
                LinkKind::Line => LinkKindDto::Line,
                LinkKind::Portal => LinkKindDto::Portal,
            },
            fit: None,
            spacing: None,
            size: None,
            color: None,
            content: None,
            anchors: None,
            control_points: Vec::new(),
            visible: None,
            zoom: None,
            spans: Vec::new(),
            cap_start: None,
            cap_end: None,
            label: None,
            ends: None,
        };
        match link.kind {
            LinkKind::Line => {
                let flow = self.flow(&link.path.flow)?;
                let (spans, cap_start, cap_end, label) = self.on_path(id)?;
                dto.fit = flow.fit;
                dto.spacing = flow.spacing;
                dto.size = flow.size;
                dto.color = flow.color;
                dto.content = flow.content;
                dto.anchors = anchors(&link.path);
                dto.control_points = control_points(&link.path);
                dto.visible = (!link.path.visible).then_some(false);
                dto.zoom = Self::zoom(link.path.zoom);
                dto.spans = spans;
                dto.cap_start = cap_start;
                dto.cap_end = cap_end;
                dto.label = label;
            }
            LinkKind::Portal => {
                // The path a portal link retains (so that it can become a
                // line link without losing it) is not saved: the file's
                // path keys belong to line links.
                let mut ends = Vec::with_capacity(2);
                let mut default_ends = true;
                let found: Vec<(NodeId, &PortalEndData)> = self
                    .children(id)
                    .into_iter()
                    .filter_map(|(end_id, element)| match element {
                        DocElement::PortalEnd(end) => Some((end_id, end)),
                        _ => None,
                    })
                    .collect();
                if found.len() != 2 {
                    return Err(self.error(SaveErrorKind::PortalEnds { count: found.len() }));
                }
                for (end_id, end) in found {
                    default_ends &=
                        *end == PortalEndData {
                            end: end.end,
                            style: PortalEndStyle::default(),
                        } && self.children(end_id).is_empty();
                    let index = ends.len();
                    let dto_end =
                        self.key("ends", |s| s.at(Seg::Index(index), |s| s.portal_end(end_id, end)))?;
                    ends.push(dto_end);
                }
                if !default_ends {
                    if let Ok(ends) = <[PortalEndDto; 2]>::try_from(ends) {
                        dto.ends = Some(ends);
                    }
                }
            }
        }
        Ok(dto)
    }

    /// One end of a portal link.
    fn portal_end(&mut self, id: NodeId, end: &PortalEndData) -> SaveResult<PortalEndDto> {
        let style = &end.style;
        let spans = self.spans(id, false)?;
        let mut label = None;
        for (child, element) in self.children(id) {
            if let DocElement::Label(_) = element {
                label = Some(self.key("label", |s| s.label(child))?);
            }
        }
        Ok(PortalEndDto {
            border_t: style.border_t,
            offset: style.offset,
            color: self.color("color", style.color)?,
            size: style.size,
            content: style.content.clone(),
            spans,
            label,
        })
    }

    /// The map's defaults, or `None` when they are all unset. The
    /// current path is `defaults`.
    fn defaults(&mut self, defaults: &DefaultsData) -> SaveResult<Option<DefaultsDto>> {
        if *defaults == DefaultsData::default() {
            return Ok(None);
        }
        let node = self.key("node", |s| s.node_defaults(&defaults.node))?;
        let section = self.key("section", |s| s.text_defaults(&defaults.section))?;
        let frame = self.key("frame", |s| s.frame_defaults(&defaults.frame))?;
        let ring = self.key("ring", |s| s.ring_defaults(&defaults.ring))?;
        let connector = self.key("connector", |s| s.flow_defaults(&defaults.connector))?;
        let link = self.key("link", |s| s.flow_defaults(&defaults.link))?;
        let label = self.key("label", |s| s.label_defaults(&defaults.label))?;
        let portal_end = self.key("portal_end", |s| s.portal_end_defaults(&defaults.portal_end))?;
        let span = self.key("span", |s| s.text_defaults(&defaults.span))?;
        Ok(Some(DefaultsDto {
            node,
            section,
            frame,
            ring,
            connector,
            link,
            label,
            portal_end,
            span,
        }))
    }

    fn node_defaults(&mut self, style: &NodeStyle) -> SaveResult<Option<NodeDefaultsDto>> {
        if *style == NodeStyle::default() {
            return Ok(None);
        }
        let text = self.text(&style.text, SizeUnit::Pt)?;
        Ok(Some(NodeDefaultsDto {
            shape: style.shape.map(shape),
            fill: self.color("fill", style.fill)?,
            fill_fade: style.fill_fade,
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
        }))
    }

    fn text_defaults(&mut self, style: &TextStyle) -> SaveResult<Option<TextDefaultsDto>> {
        if *style == TextStyle::default() {
            return Ok(None);
        }
        let text = self.text(style, SizeUnit::Pt)?;
        Ok(Some(TextDefaultsDto {
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
        }))
    }

    fn frame_defaults(&mut self, style: &FrameStyle) -> SaveResult<Option<FrameDefaultsDto>> {
        if *style == FrameStyle::default() {
            return Ok(None);
        }
        let out = self.frame_style(style)?;
        Ok(Some(FrameDefaultsDto {
            visible: out.visible,
            start: out.start,
            direction: out.direction,
            fit: out.fit,
            gap: out.gap,
            inset: out.inset,
            font: out.text.font,
            size: out.text.size,
            color: out.text.color,
            weight: out.text.weight,
            italic: out.text.italic,
        }))
    }

    fn ring_defaults(&mut self, style: &RingStyle) -> SaveResult<Option<RingDefaultsDto>> {
        if *style == RingStyle::default() {
            return Ok(None);
        }
        let text = self.text(&style.text, SizeUnit::Rel)?;
        Ok(Some(RingDefaultsDto {
            fit: Self::fit(&style.fit),
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
        }))
    }

    fn flow_defaults(&mut self, style: &FlowStyle) -> SaveResult<Option<FlowDefaultsDto>> {
        if *style == FlowStyle::default() {
            return Ok(None);
        }
        let flow = self.flow(style)?;
        Ok(Some(FlowDefaultsDto {
            fit: flow.fit,
            spacing: flow.spacing,
            size: flow.size,
            color: flow.color,
            content: flow.content,
        }))
    }

    fn label_defaults(&mut self, style: &LabelStyle) -> SaveResult<Option<LabelDefaultsDto>> {
        if *style == LabelStyle::default() {
            return Ok(None);
        }
        let text = self.text(&style.text, SizeUnit::Pt)?;
        Ok(Some(LabelDefaultsDto {
            t: style.t,
            offset: style.offset,
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
        }))
    }

    fn portal_end_defaults(&mut self, style: &PortalEndStyle) -> SaveResult<Option<PortalEndDefaultsDto>> {
        if *style == PortalEndStyle::default() {
            return Ok(None);
        }
        Ok(Some(PortalEndDefaultsDto {
            border_t: style.border_t,
            offset: style.offset,
            color: self.color("color", style.color)?,
            size: style.size,
            content: style.content.clone(),
        }))
    }
}

/// A frame's style keys, as the file spells them.
struct FrameOut {
    visible: Option<bool>,
    start: Option<StartDto>,
    direction: Option<DirectionDto>,
    fit: Option<FitDto>,
    gap: Option<f32>,
    inset: Option<f32>,
    text: TextOut,
}

/// A path's style keys, as the file spells them.
struct FlowOut {
    fit: Option<FitDto>,
    spacing: Option<f32>,
    size: Option<f32>,
    color: Option<ColorDto>,
    content: Option<String>,
}

/// The path of a connector the file leaves out, and of a portal link.
fn default_path() -> PathData {
    PathData {
        flow: FlowStyle::default(),
        anchors: (Anchor::Auto, Anchor::Auto),
        control_points: Vec::new(),
        visible: true,
        zoom: None,
    }
}

/// A channel as the file's integer. A `usize` always fits a `u64` on the
/// targets this builds for.
fn channel_of(channel: usize) -> u64 {
    u64::try_from(channel).unwrap_or(u64::MAX)
}

/// A node's outline as the file spells it.
fn shape(shape: Shape) -> ShapeDto {
    match shape {
        Shape::Ellipse => ShapeDto::Ellipse,
        Shape::Rectangle { radius } => ShapeDto::Rectangle(RectangleDto {
            radius: u32::from(radius.get()),
        }),
    }
}

/// A path's anchors, or `None` when they are the default.
fn anchors(path: &PathData) -> Option<[AnchorDto; 2]> {
    let name = |anchor| match anchor {
        Anchor::Auto => AnchorDto::Auto,
        Anchor::Top => AnchorDto::Top,
        Anchor::Right => AnchorDto::Right,
        Anchor::Bottom => AnchorDto::Bottom,
        Anchor::Left => AnchorDto::Left,
    };
    let (from, to) = path.anchors;
    (path.anchors != (Anchor::Auto, Anchor::Auto)).then(|| [name(from), name(to)])
}

/// A path's control points as pairs.
fn control_points(path: &PathData) -> Vec<[f32; 2]> {
    path.control_points.iter().map(|point| point.to_array()).collect()
}
