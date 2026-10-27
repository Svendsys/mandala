// SPDX-License-Identifier: MPL-2.0

//! The map level of a load: the format number, the theme, the
//! palettes, the definitions and the defaults — everything a node's
//! colors and triggers are resolved against, read before any node.

use crate::document::bounds;
use crate::document::element::DocKind;
use crate::document::element::{
    DefaultsData, DocElement, FlowStyle, FrameStyle, LabelStyle, NodeStyle, PaletteData, PaletteGroup,
    PortalEndStyle, RingStyle, TextStyle, ThemeData, ThemeVariant,
};
use crate::document::format::dto::{
    DefaultsDto, FlowDefaultsDto, LabelDefaultsDto, MapHeaderDto, NodeDefaultsDto, PaletteGroupDto,
    RingDefaultsDto, TextDefaultsDto, ThemeDto,
};
use crate::document::format::error::{LoadErrorKind, Seg};
use crate::document::format::from_dto::{sorted_names, Builder, LoadResult};
use crate::document::format::index::FORMAT;
use crate::document::format::spelling::{HexDto, Named};
use crate::document::ids::{Definitions, EditDef, PresentationDef, VerbDef};
use crate::document::place::Unit;
use crate::document::values::{Rgba, VerbId};
use indextree::NodeId;
use rustc_hash::FxHashSet;

/// A name and what it names, in a file's object.
type NameAndValue<'a, T> = (&'a str, &'a T);

impl Builder {
    /// The map's header: check `format`, then read what nodes resolve
    /// against — the theme, the palettes, the definitions, the defaults —
    /// into the map's row. Returns the definitions, which the document
    /// holds beside the tree.
    pub(super) fn map(&mut self, dto: &MapHeaderDto) -> LoadResult<Definitions> {
        if dto.format != FORMAT {
            return Err(self.error_at(
                "format",
                LoadErrorKind::UnsupportedFormat {
                    found: dto.format,
                    newest: FORMAT,
                },
            ));
        }
        self.counts(dto)?;
        let root = self.tree.root();
        let theme = match &dto.theme {
            Some(theme) => self.key("theme", |b| b.theme(theme))?,
            None => ThemeData::default(),
        };
        self.element(root, DocElement::Theme(theme), |_, _| Ok(()))?;
        self.key("palettes", |b| b.palettes(root, &dto.palettes))?;
        let definitions = self.definitions(dto)?;
        let defaults = match &dto.defaults {
            Some(defaults) => self.key("defaults", |b| b.defaults(defaults))?,
            None => DefaultsData::default(),
        };
        self.element(root, DocElement::Defaults(Box::new(defaults)), |_, _| Ok(()))?;
        Ok(definitions)
    }

    /// Refuse a file with more of something than an index addresses,
    /// before any of it is read.
    fn counts(&self, dto: &MapHeaderDto) -> LoadResult<()> {
        let check = |segs: &[Seg], count: usize, max: usize| {
            bounds::check_count(count, max)
                .map(|_| ())
                .map_err(|error| self.error_under(segs, LoadErrorKind::Bound { error }))
        };
        check(
            &[Seg::Key("palettes")],
            dto.palettes.0.len(),
            bounds::MAX_PALETTES,
        )?;
        check(&[Seg::Key("edits")], dto.edits.len(), bounds::MAX_EDITS)?;
        check(
            &[Seg::Key("presentations")],
            dto.presentations.len(),
            bounds::MAX_PRESENTATIONS,
        )?;
        check(&[Seg::Key("verbs")], dto.verbs.len(), bounds::MAX_VERBS)?;
        check(
            &[Seg::Key("theme"), Seg::Key("variants")],
            dto.theme.as_ref().map_or(0, |theme| theme.variants.0.len()),
            bounds::MAX_THEME_VARIANTS,
        )
    }

    /// The theme: variants in ascending name order, every variant
    /// defining the same variables, and the variables in ascending name
    /// order (the order of [`ThemeData::variables`], which is the order
    /// of a [`VarId`](crate::document::values::VarId)). The current path
    /// is `theme`.
    fn theme(&mut self, dto: &ThemeDto) -> LoadResult<ThemeData> {
        let variants = self.key("variants", |b| b.names(&dto.variants.0))?;
        let Some(&(first_name, first)) = variants.first() else {
            return Err(self.error_at(
                "active",
                LoadErrorKind::ActiveNotAVariant {
                    name: dto.active.clone(),
                },
            ));
        };
        let variables: Vec<String> = self
            .variant(first_name, first)?
            .iter()
            .map(|(name, _)| (*name).to_string())
            .collect();
        self.bounded_at(
            &[Seg::Key("variants"), Seg::Name(first_name.to_string())],
            bounds::check_count(variables.len(), bounds::MAX_THEME_VARIABLES),
        )?;
        let mut theme_variants = Vec::with_capacity(variants.len());
        for &(name, values) in &variants {
            let sorted = self.variant(name, values)?;
            self.same_variables(name, &variables, &sorted)?;
            theme_variants.push(ThemeVariant {
                name: name.to_string(),
                values: sorted.iter().map(|(_, hex)| Rgba(hex.0)).collect(),
            });
        }
        let active = variants
            .binary_search_by(|(name, _)| (*name).cmp(dto.active.as_str()))
            .ok()
            .and_then(|index| u16::try_from(index).ok())
            .ok_or_else(|| {
                self.error_at(
                    "active",
                    LoadErrorKind::ActiveNotAVariant {
                        name: dto.active.clone(),
                    },
                )
            })?;
        self.vars = variables.clone();
        Ok(ThemeData {
            variables,
            variants: theme_variants,
            active,
        })
    }

    /// The variables of one variant in ascending name order; a variable
    /// written twice is an error. The current path is `theme`.
    fn variant<'a>(
        &mut self,
        name: &str,
        values: &'a Named<HexDto>,
    ) -> LoadResult<Vec<NameAndValue<'a, HexDto>>> {
        self.key("variants", |b| {
            b.at(Seg::Name(name.to_string()), |b| b.names(&values.0))
        })
    }

    /// `entries` in ascending name order. A name written twice is an
    /// error at the second. The current path is the object.
    fn names<'a, T>(&self, entries: &'a [(String, T)]) -> LoadResult<Vec<NameAndValue<'a, T>>> {
        let sorted = sorted_names(entries);
        for pair in sorted.windows(2) {
            if let [before, after] = pair {
                if before.0 == after.0 {
                    return Err(self.error_under(
                        &[Seg::Name(after.0.clone())],
                        LoadErrorKind::DuplicateKey { key: after.0.clone() },
                    ));
                }
            }
        }
        Ok(sorted
            .into_iter()
            .map(|(name, value)| (name.as_str(), value))
            .collect())
    }

    /// Whether the variant `name`, its entries in ascending order,
    /// defines exactly `variables`. The current path is `theme`.
    fn same_variables(
        &self,
        name: &str,
        variables: &[String],
        entries: &[NameAndValue<'_, HexDto>],
    ) -> LoadResult<()> {
        use std::cmp::Ordering;
        let differ = |variable: &str, missing: bool| {
            self.error_under(
                &[Seg::Key("variants"), Seg::Name(name.to_string())],
                LoadErrorKind::VariantsDiffer {
                    variable: variable.to_string(),
                    missing,
                },
            )
        };
        let mut want = variables.iter().peekable();
        let mut have = entries.iter().peekable();
        loop {
            match (want.peek(), have.peek()) {
                (None, None) => return Ok(()),
                (Some(wanted), None) => return Err(differ(wanted, true)),
                (None, Some((found, _))) => return Err(differ(found, false)),
                (Some(wanted), Some((found, _))) => match wanted.as_str().cmp(found) {
                    Ordering::Equal => {
                        want.next();
                        have.next();
                    }
                    Ordering::Less => return Err(differ(wanted, true)),
                    Ordering::Greater => return Err(differ(found, false)),
                },
            }
        }
    }

    /// The palettes, as tree elements in ascending name order, which is
    /// the order of their [`PaletteId`](crate::document::values::PaletteId)s.
    /// The current path is `palettes`.
    fn palettes(&mut self, root: NodeId, palettes: &Named<Vec<PaletteGroupDto>>) -> LoadResult<()> {
        let sorted = self.names(&palettes.0)?;
        self.palettes = sorted.iter().map(|(name, _)| (*name).to_string()).collect();
        for (name, groups) in sorted {
            let data = self.at(Seg::Name(name.to_string()), |b| {
                let mut out = Vec::with_capacity(groups.len());
                for (index, group) in groups.iter().enumerate() {
                    out.push(b.at(Seg::Index(index), |b| b.palette_group(group))?);
                }
                Ok(PaletteData {
                    name: name.to_string(),
                    groups: out,
                })
            })?;
            self.element(root, DocElement::Palette(data), |_, _| Ok(()))?;
        }
        Ok(())
    }

    /// One palette depth.
    fn palette_group(&mut self, group: &PaletteGroupDto) -> LoadResult<PaletteGroup> {
        Ok(PaletteGroup {
            fill: self.plain_color("fill", &group.fill)?,
            frame: self.plain_color("frame", &group.frame)?,
            text: self.plain_color("text", &group.text)?,
            title: match &group.title {
                Some(title) => Some(self.plain_color("title", title)?),
                None => None,
            },
        })
    }

    /// The edits, presentations and verbs: ids unique within each list,
    /// and the verbs numbered in file order.
    fn definitions(&mut self, dto: &MapHeaderDto) -> LoadResult<Definitions> {
        let edits = self.defined("edits", &dto.edits, |d| (&d.id, &d.name))?;
        let presentations = self.defined("presentations", &dto.presentations, |d| (&d.id, &d.name))?;
        let verbs = self.defined("verbs", &dto.verbs, |d| (&d.id, &d.name))?;
        for (index, (id, _)) in verbs.iter().enumerate() {
            if let Ok(index) = u16::try_from(index) {
                self.verbs.insert(id.clone(), VerbId(index));
            }
        }
        Ok(Definitions {
            edits: edits.into_iter().map(|(id, name)| EditDef { id, name }).collect(),
            presentations: presentations
                .into_iter()
                .map(|(id, name)| PresentationDef { id, name })
                .collect(),
            verbs: verbs.into_iter().map(|(id, name)| VerbDef { id, name }).collect(),
        })
    }

    /// One list of definitions as `(id, name)`, ids unique.
    fn defined<T>(
        &mut self,
        key: &'static str,
        items: &[T],
        parts: impl Fn(&T) -> (&String, &String),
    ) -> LoadResult<Vec<(String, String)>> {
        let mut seen: FxHashSet<&str> = FxHashSet::default();
        let mut out = Vec::with_capacity(items.len());
        self.each(key, items, |b, item| {
            let (id, name) = parts(item);
            if !seen.insert(id.as_str()) {
                return Err(b.error_at("id", LoadErrorKind::DuplicateDefinition { id: id.clone() }));
            }
            out.push((id.clone(), name.clone()));
            Ok(())
        })?;
        Ok(out)
    }

    /// The map's defaults: one style struct per kind. The current path
    /// is `defaults`.
    fn defaults(&mut self, dto: &DefaultsDto) -> LoadResult<DefaultsData> {
        Ok(DefaultsData {
            node: match &dto.node {
                Some(node) => self.key("node", |b| b.node_defaults(node))?,
                None => NodeStyle::default(),
            },
            section: match &dto.section {
                Some(text) => self.key("section", |b| b.text_defaults(text, DocKind::Section))?,
                None => TextStyle::default(),
            },
            frame: match &dto.frame {
                Some(frame) => self.key("frame", |b| b.frame_style(frame_in!(frame)))?,
                None => FrameStyle::default(),
            },
            ring: match &dto.ring {
                Some(ring) => self.key("ring", |b| b.ring_defaults(ring))?,
                None => RingStyle::default(),
            },
            connector: match &dto.connector {
                Some(flow) => self.key("connector", |b| b.flow_defaults(flow))?,
                None => FlowStyle::default(),
            },
            link: match &dto.link {
                Some(flow) => self.key("link", |b| b.flow_defaults(flow))?,
                None => FlowStyle::default(),
            },
            label: match &dto.label {
                Some(label) => self.key("label", |b| b.label_defaults(label))?,
                None => LabelStyle::default(),
            },
            portal_end: match &dto.portal_end {
                Some(end) => self.key("portal_end", |b| b.portal_end_style(portal_in!(end)))?,
                None => PortalEndStyle::default(),
            },
            span: match &dto.span {
                Some(text) => self.key("span", |b| b.text_defaults(text, DocKind::Span))?,
                None => TextStyle::default(),
            },
        })
    }

    /// `defaults.node`.
    fn node_defaults(&mut self, dto: &NodeDefaultsDto) -> LoadResult<NodeStyle> {
        Ok(NodeStyle {
            shape: dto.shape.map(|shape| self.shape(shape)).transpose()?,
            fill: match &dto.fill {
                Some(fill) => Some(self.color("fill", fill)?),
                None => None,
            },
            fill_fade: self.opt_number("fill_fade", dto.fill_fade, bounds::check_non_negative)?,
            text: self.text_style(text_in!(dto), Unit::of_tier(DocKind::Node))?,
        })
    }

    /// `defaults.section` and `defaults.span`.
    fn text_defaults(&mut self, dto: &TextDefaultsDto, kind: DocKind) -> LoadResult<TextStyle> {
        self.text_style(text_in!(dto), Unit::of_tier(kind))
    }

    /// `defaults.ring`.
    fn ring_defaults(&mut self, dto: &RingDefaultsDto) -> LoadResult<RingStyle> {
        Ok(RingStyle {
            fit: match &dto.fit {
                Some(fit) => Some(self.fit("fit", fit)?),
                None => None,
            },
            text: self.text_style(text_in!(dto), Unit::of_tier(DocKind::Ring))?,
        })
    }

    /// `defaults.connector` and `defaults.link`.
    fn flow_defaults(&mut self, dto: &FlowDefaultsDto) -> LoadResult<FlowStyle> {
        self.flow_style(flow_in!(dto))
    }

    /// `defaults.label`.
    fn label_defaults(&mut self, dto: &LabelDefaultsDto) -> LoadResult<LabelStyle> {
        self.label_style(dto.t, dto.offset, text_in!(dto))
    }
}
