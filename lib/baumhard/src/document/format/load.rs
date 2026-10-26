// SPDX-License-Identifier: MPL-2.0

//! The loader's builder: the DTOs of a parsed file become a
//! [`Document`](crate::document::ids::Document), in one pass.
//!
//! **One pass, in key order.** The builder walks the DTOs depth first
//! and appends each element to its parent. It never inserts into the
//! middle of a row: a parent's children are sorted by key (kind, then
//! channel) and the builder hands them to the tree in that order, so
//! every insert is the O(1) append at the end of the row. A file lists
//! sections, rings and sibling nodes in the order its author wrote
//! them, which may descend by channel; each such row is put in order
//! first (a stable sort, skipped when the row is already ascending),
//! because `Tree::insert_child` finds a new child's place by scanning
//! back from the end of its row, and a row inserted in descending order
//! would scan past every sibling already placed: quadratic, from a file
//! alone.
//!
//! **Names become indices here and nowhere else.** The theme's
//! variables, the palettes and the verbs are read first, so a color,
//! a node's palette or a trigger can be resolved to its
//! [`VarId`](crate::document::values::VarId),
//! [`PaletteId`](crate::document::values::PaletteId) or
//! [`VerbId`](crate::document::values::VerbId) as it is met. Links are
//! the exception: a link may name a node that comes later in the file,
//! so each is recorded and resolved once every node has an arena id.
//!
//! **The first error ends the load**, with the path of the value
//! ([`LoadError`]). The builder keeps the path as a stack of segments
//! and renders it only for an error, so the happy path allocates no
//! strings for it.

use crate::document::bounds::{self, BoundError};
use crate::document::element::{DocElement, MapData};
use crate::document::format::dto::MapDto;
use crate::document::format::error::{render_path, LoadError, LoadErrorKind, Seg};
use crate::document::format::SizeUnit;
use crate::document::ids::Document;
use crate::document::shape::validate_shape;
use crate::document::values::{
    ColorRef, FitMode, FontSize, Percent, PlainColor, Rgba, Shape, StartPoint, VarId, VerbId, Weight,
    ZoomWindow,
};
use crate::engine::tree::{Placement, Tree};
use crate::font::fonts::{app_font_by_family, AppFont};
use glam::Vec2;
use indextree::NodeId;
use rustc_hash::FxHashMap;

/// What every step of the builder returns.
pub(super) type LoadResult<T> = Result<T, LoadError>;

/// A link whose `to` waits for every node to have an arena id.
pub(super) struct PendingLink {
    /// The link element.
    pub link: NodeId,
    /// The node that holds it.
    pub owner: NodeId,
    /// The ident its `to` names.
    pub to: u32,
    /// Where its `to` is, for the error.
    pub path: Vec<Seg>,
}

/// The state of one load.
pub(super) struct Builder {
    /// The tree being built.
    pub tree: Tree<DocElement>,
    /// Where in the file the builder is.
    pub path: Vec<Seg>,
    /// The theme's variable names, ascending: a name's position is its
    /// [`VarId`].
    pub vars: Vec<String>,
    /// The palette names, ascending: a name's position is its
    /// [`PaletteId`](crate::document::values::PaletteId).
    pub palettes: Vec<String>,
    /// The verbs, by the id the file names them with.
    pub verbs: FxHashMap<String, VerbId>,
    /// The node idents seen so far, with their arena ids.
    pub idents: FxHashMap<u32, NodeId>,
    /// The links to resolve once every node is built.
    pub links: Vec<PendingLink>,
    /// The font families already looked up: `None` for one this build
    /// has no face for.
    pub fonts: FxHashMap<String, Option<AppFont>>,
}

/// The document a parsed file describes, or the first thing wrong
/// with it. O(file): one pass over the DTOs, one hash insert per node.
pub(super) fn build(dto: &MapDto) -> LoadResult<Document> {
    let mut builder = Builder {
        tree: Tree::new_non_indexed_with(DocElement::Map(MapData {
            name: dto.name.clone(),
        })),
        path: Vec::new(),
        vars: Vec::new(),
        palettes: Vec::new(),
        verbs: FxHashMap::default(),
        idents: FxHashMap::default(),
        links: Vec::new(),
        fonts: FxHashMap::default(),
    };
    let definitions = builder.map(dto)?;
    builder.resolve_links()?;
    let root = builder.tree.root();
    builder.finish(root)?;
    Document::from_tree(builder.tree, definitions).map_err(|error| LoadError {
        path: String::new(),
        kind: LoadErrorKind::Internal {
            message: error.to_string(),
        },
    })
}

impl Builder {
    /// An error of `kind` at the current path.
    pub fn error(&self, kind: LoadErrorKind) -> LoadError {
        LoadError {
            path: render_path(&self.path),
            kind,
        }
    }

    /// An error of `kind` at the current path and then `key`.
    pub fn error_at(&self, key: &'static str, kind: LoadErrorKind) -> LoadError {
        let mut path = render_path(&self.path);
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(key);
        LoadError { path, kind }
    }

    /// An error of `kind` at the current path followed by `segs`.
    pub fn error_under(&self, segs: &[Seg], kind: LoadErrorKind) -> LoadError {
        let mut path = self.path.clone();
        path.extend_from_slice(segs);
        LoadError {
            path: render_path(&path),
            kind,
        }
    }

    /// `result` with a bound error placed under the current path at
    /// `segs`.
    pub fn bounded_at<T>(&self, segs: &[Seg], result: Result<T, BoundError>) -> LoadResult<T> {
        result.map_err(|error| self.error_under(segs, LoadErrorKind::Bound { error }))
    }

    /// Run `f` one segment deeper.
    pub fn at<T>(&mut self, seg: Seg, f: impl FnOnce(&mut Self) -> LoadResult<T>) -> LoadResult<T> {
        self.path.push(seg);
        let out = f(self);
        self.path.pop();
        out
    }

    /// Run `f` inside the key `key`.
    pub fn key<T>(&mut self, key: &'static str, f: impl FnOnce(&mut Self) -> LoadResult<T>) -> LoadResult<T> {
        self.at(Seg::Key(key), f)
    }

    /// Run `f` for each item of the array under `key`, in file order,
    /// inside `key[i]`.
    pub fn each<'a, T>(
        &mut self,
        key: &'static str,
        items: &'a [T],
        mut f: impl FnMut(&mut Self, &'a T) -> LoadResult<()>,
    ) -> LoadResult<()> {
        self.key(key, |b| {
            for (index, item) in items.iter().enumerate() {
                b.at(Seg::Index(index), |b| f(b, item))?;
            }
            Ok(())
        })
    }

    /// Run `f` for each item of the array under `key` in ascending
    /// order of `channel_of` (a stable sort, so equal channels keep the
    /// file's order), inside `key[i]` where `i` is the item's position
    /// in the file.
    ///
    /// This is how a row reaches the tree in key order. A row already
    /// ascending, which is every row this crate's saver writes, is read
    /// in place; only a row that descends somewhere is sorted, and the
    /// sort is of positions, not of the items.
    pub fn each_by_channel<'a, T>(
        &mut self,
        key: &'static str,
        items: &'a [T],
        channel_of: impl Fn(&T) -> usize,
        mut f: impl FnMut(&mut Self, &'a T) -> LoadResult<()>,
    ) -> LoadResult<()> {
        let ascending = items.windows(2).all(|pair| match pair {
            [before, after] => channel_of(before) <= channel_of(after),
            _ => true,
        });
        if ascending {
            return self.each(key, items, f);
        }
        let mut order: Vec<usize> = (0..items.len()).collect();
        order.sort_by_key(|&index| items.get(index).map_or(0, &channel_of));
        self.key(key, |b| {
            for index in order {
                if let Some(item) = items.get(index) {
                    b.at(Seg::Index(index), |b| f(b, item))?;
                }
            }
            Ok(())
        })
    }

    /// Append `element` under `parent`: the O(1) end of its row, since
    /// the builder adds a row in key order.
    pub fn add(&mut self, parent: NodeId, element: DocElement) -> LoadResult<NodeId> {
        self.tree
            .insert_child(parent, element, Placement::Last)
            .map_err(|error| {
                self.error(LoadErrorKind::Internal {
                    message: error.to_string(),
                })
            })
    }

    /// Check the shape rules for the element at `id`, whose children
    /// are all in.
    pub fn finish(&self, id: NodeId) -> LoadResult<()> {
        validate_shape(&self.tree, id).map_err(|error| {
            self.error(LoadErrorKind::Internal {
                message: error.to_string(),
            })
        })
    }

    /// `result` with a bound error placed at `key` under the current
    /// path.
    pub fn bounded<T>(&self, key: &'static str, result: Result<T, BoundError>) -> LoadResult<T> {
        result.map_err(|error| self.error_at(key, LoadErrorKind::Bound { error }))
    }

    /// `value` if `check` accepts it, else a bound error at `key`.
    pub fn number(
        &self,
        key: &'static str,
        value: f32,
        check: fn(f32) -> Result<f32, BoundError>,
    ) -> LoadResult<f32> {
        self.bounded(key, check(value))
    }

    /// [`Self::number`] for a key that may be absent.
    pub fn opt_number(
        &self,
        key: &'static str,
        value: Option<f32>,
        check: fn(f32) -> Result<f32, BoundError>,
    ) -> LoadResult<Option<f32>> {
        value.map(|v| self.number(key, v, check)).transpose()
    }

    /// An `[x, y]` pair whose parts `check` accepts, as a vector. A
    /// part that fails is reported at `key[0]` or `key[1]`.
    pub fn pair(
        &mut self,
        key: &'static str,
        value: [f32; 2],
        check: fn(f32) -> Result<f32, BoundError>,
    ) -> LoadResult<Vec2> {
        self.key(key, |b| {
            let [x, y] = value;
            let x = b.at(Seg::Index(0), |b| b.bounded_here(check(x)))?;
            let y = b.at(Seg::Index(1), |b| b.bounded_here(check(y)))?;
            Ok(Vec2::new(x, y))
        })
    }

    /// `result` with a bound error at the current path.
    pub fn bounded_here<T>(&self, result: Result<T, BoundError>) -> LoadResult<T> {
        result.map_err(|error| self.error(LoadErrorKind::Bound { error }))
    }

    /// A font size in `unit`, checked against the bound for it.
    pub fn size(&self, value: f32, unit: SizeUnit) -> LoadResult<FontSize> {
        match unit {
            SizeUnit::Pt => self
                .bounded("size", bounds::check_font_size_pt(value))
                .map(FontSize::Pt),
            SizeUnit::Rel => self
                .bounded("size", bounds::check_rel_size(value))
                .map(FontSize::Rel),
        }
    }

    /// A font weight, checked.
    pub fn weight(&self, value: u16) -> LoadResult<Weight> {
        self.bounded("weight", Weight::new(value))
    }

    /// The face a font family name stands for. A name this build has no
    /// face for is an error, and so is one that is only an installed
    /// system font: it resolves to the fallback face
    /// ([`AppFont::Any`]), which has no name a file could write, so
    /// the document could not save it back.
    pub fn font(&mut self, family: &str) -> LoadResult<AppFont> {
        let found = match self.fonts.get(family) {
            Some(found) => *found,
            None => {
                let found = app_font_by_family(family).filter(|font| *font != AppFont::Any);
                self.fonts.insert(family.to_string(), found);
                found
            }
        };
        found.ok_or_else(|| {
            self.error_at(
                "font",
                LoadErrorKind::UnknownFont {
                    family: family.to_string(),
                },
            )
        })
    }

    /// A theme variable by name.
    pub fn var(&self, name: &str) -> LoadResult<VarId> {
        self.vars
            .binary_search_by(|known| known.as_str().cmp(name))
            .ok()
            .and_then(|index| u16::try_from(index).ok())
            .map(VarId)
            .ok_or_else(|| {
                self.error(LoadErrorKind::UnknownVariable {
                    name: name.to_string(),
                })
            })
    }

    /// A verb by id, at `key`.
    pub fn verb(&self, key: &'static str, id: &str) -> LoadResult<VerbId> {
        self.verbs
            .get(id)
            .copied()
            .ok_or_else(|| self.error_at(key, LoadErrorKind::UnknownVerb { id: id.to_string() }))
    }

    /// A shape, checked.
    pub fn shape(&self, shape: crate::document::format::spelling::ShapeDto) -> LoadResult<Shape> {
        use crate::document::format::spelling::ShapeDto;
        match shape {
            ShapeDto::Ellipse => Ok(Shape::Ellipse),
            ShapeDto::Rectangle(rectangle) => {
                let radius = rectangle.radius;
                let percent = u8::try_from(radius)
                    .map_err(|_| BoundError::Above {
                        value: radius as f32,
                        max: f32::from(bounds::MAX_PERCENT),
                    })
                    .and_then(bounds::check_percent);
                // Reported at the radius, inside the shape: the path
                // of the number that is out of bounds.
                let percent = percent.map_err(|error| {
                    self.error_under(
                        &[Seg::Key("shape"), Seg::Key("rectangle"), Seg::Key("radius")],
                        LoadErrorKind::Bound { error },
                    )
                })?;
                Ok(Shape::Rectangle {
                    radius: Percent::new(percent),
                })
            }
        }
    }

    /// A color as an element stores it, at `key`.
    pub fn color(
        &mut self,
        key: &'static str,
        color: &crate::document::format::spelling::ColorDto,
    ) -> LoadResult<ColorRef> {
        use crate::document::format::spelling::{ColorDto, PaletteFieldDto};
        use crate::document::values::PaletteField;
        self.key(key, |b| match color {
            ColorDto::Hex(hex) => Ok(ColorRef::Rgba(Rgba(hex.0))),
            ColorDto::Var(name) => b.var(name).map(ColorRef::Var),
            ColorDto::Palette(field) => Ok(ColorRef::Palette(match field {
                PaletteFieldDto::Fill => PaletteField::Fill,
                PaletteFieldDto::Frame => PaletteField::Frame,
                PaletteFieldDto::Text => PaletteField::Text,
                PaletteFieldDto::Title => PaletteField::Title,
            })),
        })
    }

    /// A color a palette group may hold, at `key`.
    pub fn plain_color(
        &mut self,
        key: &'static str,
        color: &crate::document::format::spelling::ColorDto,
    ) -> LoadResult<PlainColor> {
        use crate::document::format::spelling::ColorDto;
        match color {
            ColorDto::Palette(_) => Err(self.error_at(key, LoadErrorKind::PaletteInPalette)),
            other => match self.color(key, other)? {
                ColorRef::Rgba(rgba) => Ok(PlainColor::Rgba(rgba)),
                ColorRef::Var(var) => Ok(PlainColor::Var(var)),
                // Rejected above; a defect in this function otherwise.
                ColorRef::Palette(_) => Err(self.error_at(key, LoadErrorKind::PaletteInPalette)),
            },
        }
    }

    /// A fit mode, checked, at `key`.
    pub fn fit(
        &mut self,
        key: &'static str,
        fit: &crate::document::format::spelling::FitDto,
    ) -> LoadResult<FitMode> {
        use crate::document::defaults::{FIT_MAX, FIT_MIN};
        use crate::document::format::spelling::FitDto;
        self.key(key, |b| match fit {
            FitDto::Repeat => Ok(FitMode::Repeat),
            FitDto::Spread => Ok(FitMode::Spread),
            FitDto::Once(once) => Ok(FitMode::Once {
                filler: once.as_ref().map(|once| once.filler.clone()),
            }),
            FitDto::Fit(None) => Ok(FitMode::Fit {
                min: FIT_MIN,
                max: FIT_MAX,
            }),
            FitDto::Fit(Some(range)) => {
                let (min, max) = b.bounded_here(bounds::check_fit(range.min, range.max))?;
                Ok(FitMode::Fit { min, max })
            }
        })
    }

    /// A frame's start point, checked, at `key`.
    pub fn start(
        &mut self,
        key: &'static str,
        start: &crate::document::format::spelling::StartDto,
    ) -> LoadResult<StartPoint> {
        use crate::document::format::spelling::EdgeDto;
        use crate::document::values::Edge;
        let point = start.0;
        let at = self.key(key, |b| b.bounded("at", bounds::check_unit_interval(point.at)))?;
        Ok(StartPoint {
            edge: match point.edge {
                EdgeDto::Top => Edge::Top,
                EdgeDto::Right => Edge::Right,
                EdgeDto::Bottom => Edge::Bottom,
                EdgeDto::Left => Edge::Left,
            },
            at,
        })
    }

    /// A zoom range, checked, at `key`.
    pub fn zoom(
        &mut self,
        key: &'static str,
        zoom: &crate::document::format::dto::ZoomDto,
    ) -> LoadResult<ZoomWindow> {
        self.key(key, |b| {
            let min = b.opt_number("min", zoom.min, bounds::check_non_negative)?;
            let max = b.opt_number("max", zoom.max, bounds::check_non_negative)?;
            if let (Some(min), Some(max)) = (min, max) {
                if min > max {
                    return Err(b.error(LoadErrorKind::Bound {
                        error: BoundError::Inverted { min, max },
                    }));
                }
            }
            Ok(ZoomWindow { min, max })
        })
    }

    /// A node's or section's minimum size: non-negative, and within the
    /// canvas.
    pub fn extent(&mut self, key: &'static str, value: [f32; 2]) -> LoadResult<Vec2> {
        fn extent(value: f32) -> Result<f32, BoundError> {
            bounds::check_non_negative(value).and_then(bounds::check_canvas_coord)
        }
        self.pair(key, value, extent)
    }

    /// Resolve every recorded link's `to` now that every node has an
    /// arena id.
    pub fn resolve_links(&mut self) -> LoadResult<()> {
        let links = std::mem::take(&mut self.links);
        for link in links {
            let target = self.idents.get(&link.to).copied();
            let fail = |kind| LoadError {
                path: render_path(&link.path),
                kind,
            };
            let Some(target) = target else {
                return Err(fail(LoadErrorKind::DanglingLink { to: link.to }));
            };
            if target == link.owner {
                return Err(fail(LoadErrorKind::SelfLink));
            }
            self.tree
                .update(link.link, |element| {
                    if let DocElement::Link(data) = element {
                        data.to = target;
                    }
                })
                .map_err(|error| {
                    fail(LoadErrorKind::Internal {
                        message: error.to_string(),
                    })
                })?;
        }
        Ok(())
    }
}

/// The entries of an object whose keys the author chose — the theme's
/// variants and variables, the palettes — in ascending byte order of
/// the key, which is the order their indices are assigned in. A stable
/// sort of references, so a key written twice lands side by side.
pub(super) fn sorted_names<T>(entries: &[(String, T)]) -> Vec<&(String, T)> {
    let mut sorted: Vec<&(String, T)> = entries.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    sorted
}
