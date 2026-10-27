// SPDX-License-Identifier: MPL-2.0

//! Nodes and everything a node holds: sections and their spans, the
//! frame and its rings, the connector, the links, and the caps, labels
//! and portal ends of the paths.
//!
//! Each function adds its element, then its children in the order of
//! the document's kinds (a node's sections, frame, connector, links,
//! child nodes; a path's spans, start cap, end cap, label), and then
//! checks the element's row against the shape rules. Sections, rings
//! and sibling nodes are read in ascending channel order
//! ([`Builder::each_by_channel`]), so every insert is an append.

use crate::document::bounds;
use crate::document::element::{
    CapData, ConnectorData, DocElement, FrameData, LabelData, LinkData, NodeData, NodeStyle, PathData,
    PortalEndData, RingData, RingStyle, SectionData, SpanData, TextStyle, Triggers,
};
use crate::document::format::dto::{
    AlignDto, AnchorDto, CapDto, FrameDto, LabelDto, LinkDto, LinkKindDto, LinkTargetDto, NodeDto, PathDto,
    PortalEndDto, RingDto, SectionDto, SpanDto,
};
use crate::document::format::error::{LoadErrorKind, Seg};
use crate::document::format::from_dto::{Builder, LoadResult, PendingLink};
use crate::document::format::SizeUnit;
use crate::document::ids::NodeIdent;
use crate::document::values::{Align, Anchor, LinkKind, LinkTarget, PaletteId};
use glam::Vec2;
use indextree::NodeId;

/// The anchors a path with none written has.
const AUTO_ANCHORS: (Anchor, Anchor) = (Anchor::Auto, Anchor::Auto);

/// The anchor a name stands for.
fn anchor(dto: AnchorDto) -> Anchor {
    match dto {
        AnchorDto::Auto => Anchor::Auto,
        AnchorDto::Top => Anchor::Top,
        AnchorDto::Right => Anchor::Right,
        AnchorDto::Bottom => Anchor::Bottom,
        AnchorDto::Left => Anchor::Left,
    }
}

/// The reason a span has no `size` inside a ring.
const SPAN_SIZE_IN_RING: &str =
    "a ring's text is sized by the ring, as a fraction of its node; a span inside a ring has no size of its own";

/// The reason a portal link has no path keys.
const PORTAL_HAS_NO_PATH: &str = "a portal link draws no path, only its two ends";

impl Builder {
    /// A node and everything under it, as a child of `parent`.
    pub(super) fn node(&mut self, parent: NodeId, dto: &NodeDto, is_root: bool) -> LoadResult<()> {
        let ident = self.bounded("id", bounds::check_node_ident(dto.id))?;
        if self.idents.contains_key(&ident) {
            return Err(self.error_at("id", LoadErrorKind::DuplicateId { id: ident }));
        }
        let channel = self.channel(dto.channel)?;
        let position = self.pair("position", dto.position, bounds::check_canvas_coord)?;
        let min_size = dto
            .min_size
            .map(|size| self.extent("min_size", size))
            .transpose()?;
        let style = NodeStyle {
            shape: dto.shape.map(|shape| self.shape(shape)).transpose()?,
            fill: match &dto.fill {
                Some(fill) => Some(self.color("fill", fill)?),
                None => None,
            },
            fill_fade: self.opt_number("fill_fade", dto.fill_fade, bounds::check_non_negative)?,
            text: self.text_style(text_in!(dto), SizeUnit::Pt)?,
        };
        let palette = match &dto.palette {
            Some(name) => Some(self.palette(name)?),
            None => None,
        };
        let zoom = dto
            .zoom
            .as_ref()
            .map(|zoom| self.zoom("zoom", zoom))
            .transpose()?;
        let opacity = self.number("opacity", dto.opacity, bounds::check_unit_interval)?;
        let on = match &dto.on {
            Some(on) => self.key("on", |b| {
                Ok(Triggers {
                    click: on.click.as_deref().map(|id| b.verb("click", id)).transpose()?,
                    hover: on.hover.as_deref().map(|id| b.verb("hover", id)).transpose()?,
                })
            })?,
            None => Triggers::default(),
        };
        if is_root && dto.connector.is_some() {
            return Err(self.error_at("connector", LoadErrorKind::ConnectorOnRoot));
        }

        let data = DocElement::Node(NodeData {
            ident: NodeIdent(ident),
            channel,
            position,
            min_size,
            style,
            palette,
            zoom,
            folded: dto.folded,
            hidden: dto.hidden,
            opacity,
            on,
        });
        self.element(parent, data, |b, node| {
            b.idents.insert(ident, node);
            b.node_row(node, dto, is_root)
        })?;
        Ok(())
    }

    /// What a node holds, in the order of the kinds: its sections, its
    /// frame, its connector, its links, its child nodes.
    fn node_row(&mut self, node: NodeId, dto: &NodeDto, is_root: bool) -> LoadResult<()> {
        self.each_by_channel(
            "sections",
            &dto.sections,
            |s| s.channel,
            |b, section| b.section(node, section),
        )?;
        if let Some(frame) = &dto.frame {
            self.key("frame", |b| b.frame(node, frame))?;
        }
        if !is_root {
            match &dto.connector {
                Some(connector) => self.key("connector", |b| b.connector(node, connector))?,
                None => {
                    // A child node with no `connector` has one with
                    // every field inherited and nothing under it.
                    let data = DocElement::Connector(ConnectorData { path: default_path() });
                    self.element(node, data, |_, _| Ok(()))?;
                }
            }
        }
        self.each("links", &dto.links, |b, link| b.link(node, link))?;
        self.each_by_channel(
            "children",
            &dto.children,
            |n| n.channel,
            |b, child| b.node(node, child, false),
        )
    }

    /// A palette by name.
    fn palette(&self, name: &str) -> LoadResult<PaletteId> {
        self.palettes
            .binary_search_by(|known| known.as_str().cmp(name))
            .ok()
            .and_then(|index| u16::try_from(index).ok())
            .map(PaletteId)
            .ok_or_else(|| {
                self.error_at(
                    "palette",
                    LoadErrorKind::UnknownPalette {
                        name: name.to_string(),
                    },
                )
            })
    }

    /// A section and its spans.
    fn section(&mut self, node: NodeId, dto: &SectionDto) -> LoadResult<()> {
        let channel = self.channel(dto.channel)?;
        let offset = self.pair("offset", dto.offset, bounds::check_canvas_coord)?;
        let min_size = dto
            .min_size
            .map(|size| self.extent("min_size", size))
            .transpose()?;
        let text = self.text_style(text_in!(dto), SizeUnit::Pt)?;
        let data = DocElement::Section(SectionData {
            channel,
            offset,
            min_size,
            align: dto.align.map(|align| match align {
                AlignDto::Left => Align::Left,
                AlignDto::Center => Align::Center,
                AlignDto::Right => Align::Right,
            }),
            text,
        });
        self.element(node, data, |b, section| b.spans(section, &dto.spans, false))
            .map(|_| ())
    }

    /// The spans under `parent`, in file order. `in_ring` refuses a
    /// `size`.
    fn spans(&mut self, parent: NodeId, spans: &[SpanDto], in_ring: bool) -> LoadResult<()> {
        self.each("spans", spans, |b, span| b.span(parent, span, in_ring))
    }

    /// One span.
    fn span(&mut self, parent: NodeId, dto: &SpanDto, in_ring: bool) -> LoadResult<()> {
        if in_ring && dto.size.is_some() {
            return Err(self.error_at(
                "size",
                LoadErrorKind::KeyNotAllowed {
                    key: "size",
                    reason: SPAN_SIZE_IN_RING,
                },
            ));
        }
        let style = self.text_style(text_in!(dto), SizeUnit::Pt)?;
        let link = match &dto.link {
            Some(link) => Some(self.key("link", |b| match link {
                LinkTargetDto::Url(url) => Ok(LinkTarget::Url(url.clone())),
                LinkTargetDto::Verb(id) => b.verb("verb", id).map(LinkTarget::Verb),
            })?),
            None => None,
        };
        let data = DocElement::Span(SpanData {
            text: dto.text.clone(),
            style,
            link,
        });
        self.element(parent, data, |_, _| Ok(())).map(|_| ())
    }

    /// A node's frame and its rings.
    fn frame(&mut self, node: NodeId, dto: &FrameDto) -> LoadResult<()> {
        let style = self.frame_style(frame_in!(dto))?;
        self.element(node, DocElement::Frame(FrameData { style }), |b, frame| {
            b.each_by_channel("rings", &dto.rings, |r| r.channel, |b, ring| b.ring(frame, ring))
        })
        .map(|_| ())
    }

    /// One ring and its spans.
    fn ring(&mut self, frame: NodeId, dto: &RingDto) -> LoadResult<()> {
        let channel = self.channel(dto.channel)?;
        let fit = match &dto.fit {
            Some(fit) => Some(self.fit("fit", fit)?),
            None => None,
        };
        let text = self.text_style(text_in!(dto), SizeUnit::Rel)?;
        let data = DocElement::Ring(RingData {
            channel,
            style: RingStyle { fit, text },
        });
        self.element(frame, data, |b, ring| b.spans(ring, &dto.spans, true))
            .map(|_| ())
    }

    /// A connector: the path, then what is on it.
    fn connector(&mut self, node: NodeId, dto: &PathDto) -> LoadResult<()> {
        let path = self.path_data(
            flow_in!(dto),
            dto.anchors,
            &dto.control_points,
            dto.visible,
            dto.zoom.as_ref(),
        )?;
        self.element(
            node,
            DocElement::Connector(ConnectorData { path }),
            |b, connector| {
                b.on_path(
                    connector,
                    &dto.spans,
                    dto.cap_start.as_ref(),
                    dto.cap_end.as_ref(),
                    dto.label.as_ref(),
                )
            },
        )
        .map(|_| ())
    }

    /// The path a connector or a line link draws.
    fn path_data(
        &mut self,
        flow: crate::document::format::load_style::FlowIn<'_>,
        anchors: Option<[AnchorDto; 2]>,
        control_points: &[[f32; 2]],
        visible: Option<bool>,
        zoom: Option<&crate::document::format::dto::ZoomDto>,
    ) -> LoadResult<PathData> {
        let flow = self.flow_style(flow)?;
        self.bounded(
            "control_points",
            bounds::check_control_points(control_points.len()),
        )?;
        let mut points = Vec::with_capacity(control_points.len());
        self.key("control_points", |b| {
            for (index, point) in control_points.iter().enumerate() {
                let point = b.at(Seg::Index(index), |b| {
                    let [x, y] = *point;
                    let x = b.at(Seg::Index(0), |b| b.bounded_here(bounds::check_canvas_coord(x)))?;
                    let y = b.at(Seg::Index(1), |b| b.bounded_here(bounds::check_canvas_coord(y)))?;
                    Ok(Vec2::new(x, y))
                })?;
                points.push(point);
            }
            Ok(())
        })?;
        Ok(PathData {
            flow,
            anchors: anchors.map_or(AUTO_ANCHORS, |[from, to]| (anchor(from), anchor(to))),
            control_points: points,
            visible: visible.unwrap_or(true),
            zoom: zoom.map(|zoom| self.zoom("zoom", zoom)).transpose()?,
        })
    }

    /// What lies on a path, in the order of the kinds: its spans, its
    /// start cap, its end cap, its label.
    fn on_path(
        &mut self,
        path: NodeId,
        spans: &[SpanDto],
        cap_start: Option<&CapDto>,
        cap_end: Option<&CapDto>,
        label: Option<&LabelDto>,
    ) -> LoadResult<()> {
        self.spans(path, spans, false)?;
        if let Some(cap) = cap_start {
            self.key("cap_start", |b| b.cap(path, cap, true))?;
        }
        if let Some(cap) = cap_end {
            self.key("cap_end", |b| b.cap(path, cap, false))?;
        }
        if let Some(label) = label {
            self.key("label", |b| b.label(path, label))?;
        }
        Ok(())
    }

    /// A start or end cap and its spans.
    fn cap(&mut self, path: NodeId, dto: &CapDto, start: bool) -> LoadResult<()> {
        let text: TextStyle = self.text_style(text_in!(dto), SizeUnit::Pt)?;
        let data = CapData { text };
        let element = if start {
            DocElement::CapStart(data)
        } else {
            DocElement::CapEnd(data)
        };
        self.element(path, element, |b, cap| b.spans(cap, &dto.spans, false))
            .map(|_| ())
    }

    /// A label and its spans.
    fn label(&mut self, parent: NodeId, dto: &LabelDto) -> LoadResult<()> {
        let style = self.label_style(dto.t, dto.offset, text_in!(dto))?;
        self.element(parent, DocElement::Label(LabelData { style }), |b, label| {
            b.spans(label, &dto.spans, false)
        })
        .map(|_| ())
    }

    /// A link: a line, or a portal with its two ends.
    fn link(&mut self, node: NodeId, dto: &LinkDto) -> LoadResult<()> {
        let kind = match dto.kind {
            LinkKindDto::Line => LinkKind::Line,
            LinkKindDto::Portal => LinkKind::Portal,
        };
        let path = match kind {
            LinkKind::Line => {
                if dto.ends.is_some() {
                    return Err(self.error_at(
                        "ends",
                        LoadErrorKind::KeyNotAllowed {
                            key: "ends",
                            reason: "only a portal link has ends",
                        },
                    ));
                }
                self.path_data(
                    flow_in!(dto),
                    dto.anchors,
                    &dto.control_points,
                    dto.visible,
                    dto.zoom.as_ref(),
                )?
            }
            LinkKind::Portal => {
                self.refuse_path_keys(dto)?;
                default_path()
            }
        };
        // `to` waits: the node it names may come later in the file. The
        // root stands in until `resolve_links`.
        let placeholder = self.tree.root();
        let data = DocElement::Link(LinkData {
            to: placeholder,
            kind,
            path,
        });
        let mut to_path = self.path.clone();
        to_path.push(Seg::Key("to"));
        self.element(node, data, |b, link| {
            b.links.push(PendingLink {
                link,
                owner: node,
                to: dto.to,
                path: to_path,
            });
            match kind {
                LinkKind::Line => b.on_path(
                    link,
                    &dto.spans,
                    dto.cap_start.as_ref(),
                    dto.cap_end.as_ref(),
                    dto.label.as_ref(),
                ),
                LinkKind::Portal => match &dto.ends {
                    Some([source, target]) => b.key("ends", |b| {
                        b.at(Seg::Index(0), |b| b.portal_end(link, 0, source))?;
                        b.at(Seg::Index(1), |b| b.portal_end(link, 1, target))
                    }),
                    None => {
                        for end in 0..2 {
                            let data = DocElement::PortalEnd(PortalEndData {
                                end,
                                ..PortalEndData::default()
                            });
                            b.element(link, data, |_, _| Ok(()))?;
                        }
                        Ok(())
                    }
                },
            }
        })
        .map(|_| ())
    }

    /// Refuse any key of a line that a portal link was given.
    fn refuse_path_keys(&self, dto: &LinkDto) -> LoadResult<()> {
        let present = [
            ("fit", dto.fit.is_some()),
            ("spacing", dto.spacing.is_some()),
            ("size", dto.size.is_some()),
            ("color", dto.color.is_some()),
            ("content", dto.content.is_some()),
            ("anchors", dto.anchors.is_some()),
            ("control_points", !dto.control_points.is_empty()),
            ("visible", dto.visible.is_some()),
            ("zoom", dto.zoom.is_some()),
            ("spans", !dto.spans.is_empty()),
            ("cap_start", dto.cap_start.is_some()),
            ("cap_end", dto.cap_end.is_some()),
            ("label", dto.label.is_some()),
        ];
        match present.into_iter().find(|(_, present)| *present) {
            Some((key, _)) => Err(self.error_at(
                key,
                LoadErrorKind::KeyNotAllowed {
                    key,
                    reason: PORTAL_HAS_NO_PATH,
                },
            )),
            None => Ok(()),
        }
    }

    /// One end of a portal link, and its marker spans and label.
    fn portal_end(&mut self, link: NodeId, end: u8, dto: &PortalEndDto) -> LoadResult<()> {
        let style = self.portal_end_style(portal_in!(dto))?;
        let data = DocElement::PortalEnd(PortalEndData { end, style });
        self.element(link, data, |b, id| {
            b.spans(id, &dto.spans, false)?;
            if let Some(label) = &dto.label {
                b.key("label", |b| b.label(id, label))?;
            }
            Ok(())
        })
        .map(|_| ())
    }
}

/// The path of a connector the file does not write: every field
/// inherited, attached automatically at both ends, drawn.
fn default_path() -> PathData {
    PathData {
        flow: Default::default(),
        anchors: AUTO_ANCHORS,
        control_points: Vec::new(),
        visible: true,
        zoom: None,
    }
}
