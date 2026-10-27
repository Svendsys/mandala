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
//! child node's default connector and a portal link's default ends are
//! left out. Nothing derived is written: no fitted size, no depth, no
//! resolved color.
//!
//! **Names, never indices.** A theme variable, a palette, a font and a
//! verb are written by name, from the document's own tables, so a saved
//! file does not depend on how the indices were assigned.
//!
//! **A value the file cannot spell is left out, and logged.** The
//! document can hold, through the test builder or a mutation that does
//! not check, a few things the file has no spelling for: a size in the
//! wrong unit (points on a ring, a fraction on a section), the fallback
//! face as an explicit font, an index into a table that has no such
//! entry. Writing any of them would give a file that loads as something
//! else (a fraction `0.5` read back as `0.5` points would retarget
//! silently), so the saver writes nothing and warns, and the mutations
//! that make them (V1-B3) refuse them.

use crate::document::defaults::{FIT_MAX, FIT_MIN};
use crate::document::element::{
    CapData, DefaultsData, DocElement, DocKind, FlowStyle, FrameStyle, LabelStyle, NodeStyle, PathData,
    PortalEndData, PortalEndStyle, RingStyle, TextStyle,
};
use crate::document::format::dto::{
    AlignDto, AnchorDto, CapDto, DefaultsDto, DirectionDto, EditDto, FlowDefaultsDto, FrameDefaultsDto,
    FrameDto, LabelDefaultsDto, LabelDto, LinkDto, LinkKindDto, LinkTargetDto, MapDto, NodeDefaultsDto,
    NodeDto, OnDto, PaletteGroupDto, PathDto, PortalEndDefaultsDto, PortalEndDto, PresentationDto,
    RingDefaultsDto, RingDto, SectionDto, SpanDto, TextDefaultsDto, ThemeDto, VerbDto, ZoomDto,
};
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
use rustc_hash::FxHashMap;

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
}

/// The DTOs of `doc`. O(nodes + elements): one walk.
pub(super) fn dtos_of(doc: &Document) -> MapDto {
    let mut saver = Saver {
        doc,
        vars: &doc.theme().variables,
        palettes: doc.palettes().map(|(_, palette)| palette.name.as_str()).collect(),
        verbs: &doc.definitions().verbs,
        fonts: FxHashMap::default(),
    };
    saver.map()
}

impl<'a> Saver<'a> {
    /// The whole file.
    fn map(&mut self) -> MapDto {
        let doc = self.doc;
        let name = match doc.get(doc.root()) {
            Some(DocElement::Map(map)) => map.name.clone(),
            _ => String::new(),
        };
        let definitions = doc.definitions();
        let defaults = self.defaults(doc.defaults());
        MapDto {
            format: 1,
            name,
            theme: self.theme(),
            palettes: self.palettes(),
            defaults,
            edits: definitions
                .edits
                .iter()
                .map(|d| EditDto {
                    id: d.id.clone(),
                    name: d.name.clone(),
                })
                .collect(),
            presentations: definitions
                .presentations
                .iter()
                .map(|d| PresentationDto {
                    id: d.id.clone(),
                    name: d.name.clone(),
                })
                .collect(),
            verbs: definitions
                .verbs
                .iter()
                .map(|d| VerbDto {
                    id: d.id.clone(),
                    name: d.name.clone(),
                })
                .collect(),
            nodes: self.nodes(doc.root()),
        }
    }

    /// The theme: variants and variables in ascending name order.
    fn theme(&self) -> ThemeDto {
        let theme = self.doc.theme();
        let mut variants: Vec<(String, Named<HexDto>)> = theme
            .variants
            .iter()
            .map(|variant| {
                let mut values: Vec<(String, HexDto)> = theme
                    .variables
                    .iter()
                    .zip(&variant.values)
                    .map(|(name, rgba)| (name.clone(), HexDto(rgba.0)))
                    .collect();
                values.sort_by(|a, b| a.0.cmp(&b.0));
                (variant.name.clone(), Named(values))
            })
            .collect();
        variants.sort_by(|a, b| a.0.cmp(&b.0));
        let active = match theme.variants.get(usize::from(theme.active)) {
            Some(variant) => variant.name.clone(),
            None => {
                log::warn!("save: the theme's active variant is not one of its variants; writing none");
                String::new()
            }
        };
        ThemeDto {
            active,
            variants: Named(variants),
        }
    }

    /// The palettes, by name.
    fn palettes(&self) -> Named<Vec<PaletteGroupDto>> {
        let mut palettes: Vec<(String, Vec<PaletteGroupDto>)> = self
            .doc
            .palettes()
            .map(|(_, palette)| {
                let groups = palette
                    .groups
                    .iter()
                    .map(|group| PaletteGroupDto {
                        fill: self.plain(group.fill),
                        frame: self.plain(group.frame),
                        text: self.plain(group.text),
                        title: group.title.map(|title| self.plain(title)),
                    })
                    .collect();
                (palette.name.clone(), groups)
            })
            .collect();
        palettes.sort_by(|a, b| a.0.cmp(&b.0));
        Named(palettes)
    }

    /// A palette color. A palette group has all of its colors, so one
    /// naming a variable the theme lacks is written as `var()`, which
    /// the loader refuses by name: the document that holds one is not
    /// valid, and nothing here can say what it meant.
    fn plain(&self, color: PlainColor) -> ColorDto {
        match color {
            PlainColor::Rgba(rgba) => ColorDto::Hex(HexDto(rgba.0)),
            PlainColor::Var(var) => ColorDto::Var(self.var_name(var.0).unwrap_or_default()),
        }
    }

    /// A theme variable's name, or `None` (and a warning) when the
    /// theme defines no such variable.
    fn var_name(&self, index: u16) -> Option<String> {
        let name = self.vars.get(usize::from(index)).cloned();
        if name.is_none() {
            log::warn!("save: a color names theme variable {index}, which the theme does not define");
        }
        name
    }

    /// A color as the file spells it; `None` for an unset color and for
    /// one naming a variable the theme lacks.
    fn color(&self, color: Option<ColorRef>) -> Option<ColorDto> {
        Some(match color? {
            ColorRef::Rgba(rgba) => ColorDto::Hex(HexDto(rgba.0)),
            ColorRef::Var(var) => ColorDto::Var(self.var_name(var.0)?),
            ColorRef::Palette(field) => ColorDto::Palette(match field {
                PaletteField::Fill => PaletteFieldDto::Fill,
                PaletteField::Frame => PaletteFieldDto::Frame,
                PaletteField::Text => PaletteFieldDto::Text,
                PaletteField::Title => PaletteFieldDto::Title,
            }),
        })
    }

    /// A font's family name, or `None` for the fallback face (which has
    /// no name) and for a face this build cannot name.
    fn font(&mut self, font: Option<AppFont>) -> Option<String> {
        let font = font?;
        let name = *self.fonts.entry(font).or_insert_with(|| {
            if font == AppFont::Any {
                None
            } else {
                family_name_of(font)
            }
        });
        if name.is_none() {
            log::warn!("save: a font ({font:?}) has no family name a file can write; leaving it out");
        }
        name.map(str::to_string)
    }

    /// A size in the unit this position takes.
    fn size(size: Option<FontSize>, unit: SizeUnit) -> Option<f32> {
        match (size?, unit) {
            (FontSize::Pt(points), SizeUnit::Pt) => Some(points),
            (FontSize::Rel(fraction), SizeUnit::Rel) => Some(fraction),
            (other, _) => {
                log::warn!("save: the size {other:?} is in the wrong unit for where it sits; leaving it out");
                None
            }
        }
    }

    /// The five text-style keys.
    fn text(&mut self, style: &TextStyle, unit: SizeUnit) -> TextOut {
        TextOut {
            font: self.font(style.font),
            size: Self::size(style.size, unit),
            color: self.color(style.color),
            weight: style.weight.map(|weight| WeightDto(weight.get())),
            italic: style.italic,
        }
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

    /// A verb's id.
    fn verb_id(&self, index: u16) -> Option<String> {
        let id = self.verbs.get(usize::from(index)).map(|verb| verb.id.clone());
        if id.is_none() {
            log::warn!("save: a trigger names verb {index}, which the map does not define; leaving it out");
        }
        id
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

    /// The child nodes of `parent`, with everything under them.
    fn nodes(&mut self, parent: NodeId) -> Vec<NodeDto> {
        self.children(parent)
            .into_iter()
            .filter(|(_, element)| element.kind() == DocKind::Node)
            .filter_map(|(id, _)| self.node(id))
            .collect()
    }

    /// One node.
    fn node(&mut self, id: NodeId) -> Option<NodeDto> {
        let Some(DocElement::Node(node)) = self.element(id) else {
            return None;
        };
        let text = self.text(&node.style.text, SizeUnit::Pt);
        let on = OnDto {
            click: node.on.click.and_then(|verb| self.verb_id(verb.0)),
            hover: node.on.hover.and_then(|verb| self.verb_id(verb.0)),
        };
        // What lies under the node, in the order of the kinds.
        let (mut sections, mut frame, mut connector, mut links, mut children) =
            (Vec::new(), None, None, Vec::new(), Vec::new());
        for (child, element) in self.children(id) {
            match element {
                DocElement::Section(_) => sections.extend(self.section(child)),
                DocElement::Frame(_) => frame = self.frame(child),
                DocElement::Connector(_) => connector = self.connector(child),
                DocElement::Link(_) => links.extend(self.link(child)),
                DocElement::Node(_) => children.extend(self.node(child)),
                _ => {}
            }
        }
        Some(NodeDto {
            id: node.ident.0,
            channel: channel_of(node.channel),
            position: node.position.to_array(),
            min_size: node.min_size.map(|size| size.to_array()),
            shape: node.style.shape.map(shape),
            fill: self.color(node.style.fill),
            fill_fade: node.style.fill_fade,
            palette: node.palette.and_then(|palette| {
                let name = self
                    .palettes
                    .get(usize::from(palette.0))
                    .map(|name| (*name).to_string());
                if name.is_none() {
                    log::warn!(
                        "save: a node names palette {}, which the map does not define",
                        palette.0
                    );
                }
                name
            }),
            zoom: Self::zoom(node.zoom),
            folded: node.folded,
            hidden: node.hidden,
            opacity: node.opacity,
            on: (on.click.is_some() || on.hover.is_some()).then_some(on),
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
            sections,
            frame,
            connector,
            links,
            children,
        })
    }

    /// One section.
    fn section(&mut self, id: NodeId) -> Option<SectionDto> {
        let Some(DocElement::Section(section)) = self.element(id) else {
            return None;
        };
        let text = self.text(&section.text, SizeUnit::Pt);
        Some(SectionDto {
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
            spans: self.spans(id, false),
        })
    }

    /// The spans under `parent`. A span in a ring has no size of its
    /// own to write.
    fn spans(&mut self, parent: NodeId, in_ring: bool) -> Vec<SpanDto> {
        let mut out = Vec::new();
        for (_, element) in self.children(parent) {
            let DocElement::Span(span) = element else {
                continue;
            };
            let mut text = self.text(&span.style, SizeUnit::Pt);
            if in_ring {
                text.size = None;
            }
            out.push(SpanDto {
                text: span.text.clone(),
                font: text.font,
                size: text.size,
                color: text.color,
                weight: text.weight,
                italic: text.italic,
                link: span.link.as_ref().and_then(|link| match link {
                    LinkTarget::Url(url) => Some(LinkTargetDto::Url(url.clone())),
                    LinkTarget::Verb(verb) => self.verb_id(verb.0).map(LinkTargetDto::Verb),
                }),
            });
        }
        out
    }

    /// A frame and its rings.
    fn frame(&mut self, id: NodeId) -> Option<FrameDto> {
        let Some(DocElement::Frame(frame)) = self.element(id) else {
            return None;
        };
        let style = self.frame_style(&frame.style);
        let mut rings = Vec::new();
        for (ring_id, element) in self.children(id) {
            let DocElement::Ring(ring) = element else {
                continue;
            };
            let text = self.text(&ring.style.text, SizeUnit::Rel);
            rings.push(RingDto {
                channel: channel_of(ring.channel),
                fit: Self::fit(&ring.style.fit),
                font: text.font,
                size: text.size,
                color: text.color,
                weight: text.weight,
                italic: text.italic,
                spans: self.spans(ring_id, true),
            });
        }
        Some(FrameDto {
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
    fn frame_style(&mut self, style: &FrameStyle) -> FrameOut {
        FrameOut {
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
            text: self.text(&style.text, SizeUnit::Rel),
        }
    }

    /// The connector of a node, or `None` when the file leaves it out:
    /// a connector with every field inherited and nothing under it.
    fn connector(&mut self, id: NodeId) -> Option<PathDto> {
        let Some(DocElement::Connector(connector)) = self.element(id) else {
            return None;
        };
        let children = self.children(id);
        if connector.path == default_path() && children.is_empty() {
            return None;
        }
        let flow = self.flow(&connector.path.flow);
        let (spans, cap_start, cap_end, label) = self.on_path(id);
        Some(PathDto {
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
        })
    }

    /// A path's style keys.
    fn flow(&mut self, flow: &FlowStyle) -> FlowOut {
        FlowOut {
            fit: Self::fit(&flow.fit),
            spacing: flow.spacing,
            size: flow.size,
            color: self.color(flow.color),
            content: flow.content.clone(),
        }
    }

    /// What lies on a path: its spans, caps and label.
    fn on_path(&mut self, id: NodeId) -> (Vec<SpanDto>, Option<CapDto>, Option<CapDto>, Option<LabelDto>) {
        let spans = self.spans(id, false);
        let (mut cap_start, mut cap_end, mut label) = (None, None, None);
        for (child, element) in self.children(id) {
            match element {
                DocElement::CapStart(cap) => cap_start = Some(self.cap(child, cap)),
                DocElement::CapEnd(cap) => cap_end = Some(self.cap(child, cap)),
                DocElement::Label(_) => label = self.label(child),
                _ => {}
            }
        }
        (spans, cap_start, cap_end, label)
    }

    /// A cap.
    fn cap(&mut self, id: NodeId, cap: &CapData) -> CapDto {
        let text = self.text(&cap.text, SizeUnit::Pt);
        CapDto {
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
            spans: self.spans(id, false),
        }
    }

    /// A label.
    fn label(&mut self, id: NodeId) -> Option<LabelDto> {
        let Some(DocElement::Label(label)) = self.element(id) else {
            return None;
        };
        let text = self.text(&label.style.text, SizeUnit::Pt);
        Some(LabelDto {
            t: label.style.t,
            offset: label.style.offset,
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
            spans: self.spans(id, false),
        })
    }

    /// A link.
    fn link(&mut self, id: NodeId) -> Option<LinkDto> {
        let Some(DocElement::Link(link)) = self.element(id) else {
            return None;
        };
        let to = match self.element(link.to) {
            Some(DocElement::Node(target)) => target.ident.0,
            _ => {
                log::warn!("save: a link leads to something that is not a node; leaving it out");
                return None;
            }
        };
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
                let flow = self.flow(&link.path.flow);
                let (spans, cap_start, cap_end, label) = self.on_path(id);
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
                let mut ends = Vec::with_capacity(2);
                let mut default_ends = true;
                for (end_id, element) in self.children(id) {
                    let DocElement::PortalEnd(end) = element else {
                        continue;
                    };
                    default_ends &=
                        *end == PortalEndData {
                            end: end.end,
                            style: PortalEndStyle::default(),
                        } && self.children(end_id).is_empty();
                    ends.push(self.portal_end(end_id, end));
                }
                if !default_ends {
                    if let Ok(ends) = <[PortalEndDto; 2]>::try_from(ends) {
                        dto.ends = Some(ends);
                    }
                }
            }
        }
        Some(dto)
    }

    /// One end of a portal link.
    fn portal_end(&mut self, id: NodeId, end: &PortalEndData) -> PortalEndDto {
        let style = &end.style;
        let mut spans = Vec::new();
        let mut label = None;
        spans.extend(self.spans(id, false));
        for (child, element) in self.children(id) {
            if let DocElement::Label(_) = element {
                label = self.label(child);
            }
        }
        PortalEndDto {
            border_t: style.border_t,
            offset: style.offset,
            color: self.color(style.color),
            size: style.size,
            content: style.content.clone(),
            spans,
            label,
        }
    }

    /// The map's defaults, or `None` when they are all unset.
    fn defaults(&mut self, defaults: &DefaultsData) -> Option<DefaultsDto> {
        if *defaults == DefaultsData::default() {
            return None;
        }
        let node = self.node_defaults(&defaults.node);
        let section = self.text_defaults(&defaults.section);
        let frame = self.frame_defaults(&defaults.frame);
        let ring = self.ring_defaults(&defaults.ring);
        let connector = self.flow_defaults(&defaults.connector);
        let link = self.flow_defaults(&defaults.link);
        let label = self.label_defaults(&defaults.label);
        let portal_end = self.portal_end_defaults(&defaults.portal_end);
        let span = self.text_defaults(&defaults.span);
        Some(DefaultsDto {
            node,
            section,
            frame,
            ring,
            connector,
            link,
            label,
            portal_end,
            span,
        })
    }

    fn node_defaults(&mut self, style: &NodeStyle) -> Option<NodeDefaultsDto> {
        if *style == NodeStyle::default() {
            return None;
        }
        let text = self.text(&style.text, SizeUnit::Pt);
        Some(NodeDefaultsDto {
            shape: style.shape.map(shape),
            fill: self.color(style.fill),
            fill_fade: style.fill_fade,
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
        })
    }

    fn text_defaults(&mut self, style: &TextStyle) -> Option<TextDefaultsDto> {
        if *style == TextStyle::default() {
            return None;
        }
        let text = self.text(style, SizeUnit::Pt);
        Some(TextDefaultsDto {
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
        })
    }

    fn frame_defaults(&mut self, style: &FrameStyle) -> Option<FrameDefaultsDto> {
        if *style == FrameStyle::default() {
            return None;
        }
        let out = self.frame_style(style);
        Some(FrameDefaultsDto {
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
        })
    }

    fn ring_defaults(&mut self, style: &RingStyle) -> Option<RingDefaultsDto> {
        if *style == RingStyle::default() {
            return None;
        }
        let text = self.text(&style.text, SizeUnit::Rel);
        Some(RingDefaultsDto {
            fit: Self::fit(&style.fit),
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
        })
    }

    fn flow_defaults(&mut self, style: &FlowStyle) -> Option<FlowDefaultsDto> {
        if *style == FlowStyle::default() {
            return None;
        }
        let flow = self.flow(style);
        Some(FlowDefaultsDto {
            fit: flow.fit,
            spacing: flow.spacing,
            size: flow.size,
            color: flow.color,
            content: flow.content,
        })
    }

    fn label_defaults(&mut self, style: &LabelStyle) -> Option<LabelDefaultsDto> {
        if *style == LabelStyle::default() {
            return None;
        }
        let text = self.text(&style.text, SizeUnit::Pt);
        Some(LabelDefaultsDto {
            t: style.t,
            offset: style.offset,
            font: text.font,
            size: text.size,
            color: text.color,
            weight: text.weight,
            italic: text.italic,
        })
    }

    fn portal_end_defaults(&mut self, style: &PortalEndStyle) -> Option<PortalEndDefaultsDto> {
        if *style == PortalEndStyle::default() {
            return None;
        }
        Some(PortalEndDefaultsDto {
            border_t: style.border_t,
            offset: style.offset,
            color: self.color(style.color),
            size: style.size,
            content: style.content.clone(),
        })
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
