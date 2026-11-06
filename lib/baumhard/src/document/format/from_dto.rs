// SPDX-License-Identifier: MPL-2.0

//! The loader's builder: a parsed header, then the nodes one at a time,
//! become a [`Document`].
//!
//! **Phase 3, the header** ([`Builder::begin`]): the theme's variables,
//! the palettes and the verbs are read first, so that a color, a node's
//! palette or a trigger can be resolved to its [`VarId`],
//! [`PaletteId`](crate::document::values::PaletteId) or [`VerbId`] as
//! it is met. Names become indices here and nowhere else.
//!
//! **Phase 4, the nodes** ([`Builder::node`], in `chain.rs`): each node
//! of the flat list is placed on the chain of ancestors the list's
//! pre-order implies, appended under its parent, and its own content is
//! built under it. Only one node's DTO is alive at a time, and of each row
//! it holds only one item ([`Rows`]). Nothing
//! recurses on the depth of the tree: the chain is a heap `Vec` and a
//! node's row is checked when it leaves the chain.
//!
//! **Phases 5 and 6** ([`Builder::finish`]): links are resolved last,
//! because a link may name a node that comes later in the list, then the
//! map row is checked and the document made, with the ident index the
//! build already holds.
//!
//! **Every insert is an append.** A parent's children are sorted by key
//! (kind, then channel), and the builder hands them to the tree in that
//! order, so every insert is the O(1) append at the end of the row. A
//! row the file lists out of order — nodes, sections or rings with a
//! descending channel — is refused, never sorted (design B.4, C.2): a
//! streaming loader cannot sort siblings spread across the list, and
//! appending them out of order would make `Tree::insert_child` scan back
//! past every sibling already placed — quadratic, from a file alone.
//!
//! **The first error ends the load**, with the path of the value
//! ([`LoadError`]). The builder keeps the path as a stack of segments
//! and renders it only for an error, so the happy path allocates no
//! strings for it. An error inside a node carries the node's context
//! ([`NodeContext`]), which the caller completes with the excerpt and
//! the position it alone knows.

use crate::arena::NodeId;
use crate::document::bounds::{self, BoundError};
use crate::document::element::{DocElement, DocKind, MapData};
use crate::document::format::dto::MapHeaderDto;
use crate::document::format::error::{render_path, LoadError, LoadErrorKind, NodeContext, Seg};
use crate::document::format::rows::{Rows, RowsError};
use crate::document::ids::{Definitions, Document, IdentIndex, NodeIdent};
use crate::document::keyed::KeyedMap;
use crate::document::shape::validate_shape;
use crate::document::values::{
    ColorRef, FitMode, FontSize, Percent, PlainColor, Rgba, Shape, StartPoint, VarId, VerbId, Weight,
    ZoomWindow,
};
use crate::document::writer::Derived;
use crate::engine::tree::{Placement, Tree};
use crate::font::fonts::{app_font_by_family, AppFont};
use glam::Vec2;

/// What every step of the builder returns.
pub(super) type LoadResult<T> = Result<T, LoadError>;

/// A link whose `to` waits for every node to have an arena id.
pub(super) struct PendingLink {
    /// The link element.
    pub link: NodeId,
    /// The node that holds it.
    pub owner: NodeId,
    /// The holder's place in the list and its id, for the error.
    pub owner_at: (usize, u32),
    /// The ident its `to` names.
    pub to: u32,
    /// `j` in the holder's `links[j]`: with `owner_at`, where its `to`
    /// is, rendered only for an error. Nothing per link is kept beyond
    /// these words.
    pub at: usize,
}

impl PendingLink {
    /// Where the link's `to` is: `nodes[i].links[j].to`.
    pub fn to_path(&self) -> Vec<Seg> {
        vec![
            Seg::Key("nodes"),
            Seg::Index(self.owner_at.0),
            Seg::Key("links"),
            Seg::Index(self.at),
            Seg::Key("to"),
        ]
    }
}

/// A node the list has placed: where it is in the tree, how deep, and
/// where it is in the list.
#[derive(Copy, Clone, Debug)]
pub(super) struct PlacedNode {
    /// Its element.
    pub node: NodeId,
    /// How many nodes are above it: `0` for a top-level node.
    pub depth: usize,
    /// `i` in `nodes[i]`.
    pub index: usize,
}

/// One node of the chain: the path from a top-level node to the node
/// listed last. `chain[d]` is the node at depth `d` on that path.
#[derive(Copy, Clone, Debug)]
pub(super) struct ChainEntry {
    /// Its element.
    pub node: NodeId,
    /// Its id.
    pub id: u32,
    /// Its channel, which the next sibling's may not be below.
    pub channel: usize,
    /// `i` in `nodes[i]`.
    pub index: usize,
}

/// What a [`Builder`] keeps of what it builds.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(super) enum Keep {
    /// Everything: a load, which makes the document.
    Document,
    /// One node at a time: [`check_savable`](super::check_savable),
    /// which asks the builder whether a document's file would load, and
    /// needs every check but not a second copy of the document. Each
    /// node is built, its row checked, and its elements freed before
    /// the next is read; the ident checks and the links are answered
    /// from the document being saved (see `check_savable`).
    OneNode,
}

/// The state of one load.
pub(super) struct Builder {
    /// The tree being built.
    pub tree: Tree<DocElement>,
    /// Where in the file the builder is.
    pub path: Vec<Seg>,
    /// The node being built, while there is one: its place in the list
    /// and, once read, its id. Every error raised meanwhile names it.
    pub current: Option<(usize, Option<u32>)>,
    /// The theme's variable names, ascending: a name's position is its
    /// [`VarId`].
    pub vars: Vec<String>,
    /// The palette names, ascending: a name's position is its
    /// [`PaletteId`](crate::document::values::PaletteId).
    pub palettes: Vec<String>,
    /// The verbs, by the id the file names them with.
    pub verbs: KeyedMap<String, VerbId>,
    /// Every node placed so far, by id.
    pub idents: KeyedMap<u32, PlacedNode>,
    /// The chain: the path from a top-level node to the node placed
    /// last.
    pub chain: Vec<ChainEntry>,
    /// The highest id placed so far, for the ident counter.
    pub highest: u32,
    /// The links to resolve once every node is built.
    pub links: Vec<PendingLink>,
    /// The font families already looked up: `None` for one this build
    /// has no face for.
    pub fonts: KeyedMap<String, Option<AppFont>>,
    /// The definitions the header carries, for the document.
    pub definitions: Definitions,
    /// What the document derives from its tree, gathered as it is
    /// built: the links into each node as each is resolved, and the
    /// bytes of text of every element appended (the element count is
    /// [`Self::elements`]). The writer moves it from there.
    pub derived: Derived,
    /// How many elements have had their row checked against the shape
    /// rules. Every element is checked once: an element is added by
    /// [`Self::element`], which checks it once its children are in, or
    /// is a node, which is checked when it leaves the chain. The count
    /// is how a test sees that.
    pub checked: usize,
    /// What the builder keeps.
    pub keep: Keep,
    /// Under [`Keep::OneNode`], the element every node below the top is
    /// built under in place of its parent, which is gone: a copy of the
    /// first top-level node's own element, so that a node below it is
    /// judged under a node, as it is in the document.
    pub stand_in: Option<NodeId>,
    /// How many elements have been appended, the root included, and
    /// definitions read: what [`Self::max_elements`] bounds.
    pub elements: u64,
    /// The most elements this target builds
    /// ([`MAX_ELEMENTS`](super::MAX_ELEMENTS)), or `None` for no bound.
    pub max_elements: Option<u64>,
    /// How many elements are in the tree now, and the most there have
    /// been: the memory a [`Keep::OneNode`] check holds, which a test
    /// bounds.
    pub live: usize,
    /// See [`Self::live`].
    pub live_peak: usize,
}

impl Builder {
    /// Phase 3: a builder holding the map's root and everything its
    /// nodes resolve against, built from the header. O(header).
    pub(super) fn begin(header: &MapHeaderDto, keep: Keep, max_elements: Option<u64>) -> LoadResult<Builder> {
        let root = DocElement::Map(MapData {
            name: header.name.clone(),
            ..MapData::default()
        });
        let mut derived = Derived::default();
        derived.appended(&root);
        let mut builder = Builder {
            tree: Tree::new_non_indexed_with(root),
            path: Vec::new(),
            current: None,
            vars: Vec::new(),
            palettes: Vec::new(),
            verbs: KeyedMap::default(),
            idents: KeyedMap::default(),
            chain: Vec::new(),
            highest: 0,
            links: Vec::new(),
            fonts: KeyedMap::default(),
            definitions: Definitions::default(),
            derived,
            checked: 0,
            keep,
            stand_in: None,
            elements: 1,
            max_elements,
            live: 1,
            live_peak: 1,
        };
        builder.definitions = builder.map(header)?;
        Ok(builder)
    }

    /// Phases 5 and 6, once every node is in: close the chain, resolve
    /// the links, check the map row, and make the document — with the
    /// ident index the build already holds, so nothing walks the tree
    /// again. Returns the document and how many elements had their row
    /// checked. O(nodes + links).
    pub(super) fn finish(mut self) -> LoadResult<(Document, usize)> {
        self.close(0)?;
        self.resolve_links()?;
        let root = self.tree.root();
        self.finish_element(root)?;
        let checked = self.checked;
        let index: IdentIndex = self
            .idents
            .iter()
            .map(|(id, placed)| (NodeIdent(*id), placed.node))
            .collect();
        // The loader's own count is the budget's: every element appended,
        // the root included, and every definition read.
        let mut derived = self.derived;
        derived.budget.elements = self.elements;
        Document::from_loaded(self.tree, self.definitions, index, self.highest, derived)
            .map(|document| (document, checked))
            .map_err(|error| {
                LoadError::new(
                    String::new(),
                    LoadErrorKind::Internal {
                        message: error.to_string(),
                    },
                )
            })
    }

    /// [`Self::finish`] for a [`Keep::OneNode`] check: close the chain
    /// and check the map row and the header's counts, as a load does,
    /// without making a document. Every link was answered as it was met.
    /// Returns the most elements the tree held at once.
    pub(super) fn finish_check(mut self) -> LoadResult<usize> {
        self.close(0)?;
        if let Some(stand_in) = self.stand_in.take() {
            self.free(stand_in)?;
        }
        let root = self.tree.root();
        self.finish_element(root)?;
        crate::document::ids::check_loaded(&self.tree, &self.definitions).map_err(|error| {
            LoadError::new(
                String::new(),
                LoadErrorKind::Internal {
                    message: error.to_string(),
                },
            )
        })?;
        Ok(self.live_peak)
    }

    /// The context of the node at `index` with `id`, for an error about
    /// a node that is no longer the one being built (its row, its links).
    /// The excerpt is read from the tree, where the node already is.
    pub fn context_of(&self, index: usize, id: u32) -> NodeContext {
        let excerpt = self
            .idents
            .get(&id)
            .and_then(|placed| crate::document::format::context::tree_excerpt(&self.tree, placed.node));
        NodeContext {
            index,
            id: Some(id),
            excerpt,
        }
    }

    /// An error of `kind` at the current path.
    pub fn error(&self, kind: LoadErrorKind) -> LoadError {
        self.located(render_path(&self.path), kind)
    }

    /// An error of `kind` at `path`, naming the node being built, if
    /// there is one.
    fn located(&self, path: String, kind: LoadErrorKind) -> LoadError {
        LoadError {
            node: self.current.map(|(index, id)| NodeContext {
                index,
                id,
                excerpt: None,
            }),
            ..LoadError::new(path, kind)
        }
    }

    /// An error of `kind` at the current path and then `key`.
    pub fn error_at(&self, key: &'static str, kind: LoadErrorKind) -> LoadError {
        let mut path = render_path(&self.path);
        if !path.is_empty() {
            path.push('.');
        }
        path.push_str(key);
        self.located(path, kind)
    }

    /// An error of `kind` at the current path followed by `segs`.
    pub fn error_under(&self, segs: &[Seg], kind: LoadErrorKind) -> LoadError {
        let mut path = self.path.clone();
        path.extend_from_slice(segs);
        self.located(render_path(&path), kind)
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

    /// Run `f` for each item of the row under `key`, in file order,
    /// inside `key[i]`: one item read (or, for a check, made by the
    /// saver) at a time, never the whole row ([`Rows`]).
    pub fn each_row<'r, T: serde::Deserialize<'r>>(
        &mut self,
        key: &'static str,
        rows: &Rows<'r, T>,
        mut f: impl FnMut(&mut Self, &T) -> LoadResult<()>,
    ) -> LoadResult<()> {
        self.key(key, |b| {
            let walked = rows.each(|index, item| b.at(Seg::Index(index), |b| f(b, &item)));
            match walked {
                Ok(()) => Ok(()),
                Err(RowsError::Item(error)) => Err(error),
                // The row was checked as it was read, and the saver's
                // checked before the builder sees it: either is a defect.
                Err(RowsError::Read(error)) => Err(b.error(LoadErrorKind::Internal {
                    message: error.to_string(),
                })),
                Err(RowsError::Save(error)) => Err(b.error(LoadErrorKind::Internal {
                    message: error.to_string(),
                })),
            }
        })
    }

    /// Run `f` for each item of the row under `key`, as [`Self::each_row`],
    /// refusing an item whose channel (`channel_of`) is below the one
    /// before it with
    /// [`SiblingsOutOfOrder`](LoadErrorKind::SiblingsOutOfOrder) at its
    /// `channel`. Equal channels keep the file's order.
    ///
    /// This is how a row of sections or rings reaches the tree in key
    /// order without a sort: the file must already be in it (design
    /// B.4). One spelling per row, and every insert an append.
    pub fn each_ascending<'r, T: serde::Deserialize<'r>>(
        &mut self,
        key: &'static str,
        rows: &Rows<'r, T>,
        channel_of: impl Fn(&T) -> u64,
        mut f: impl FnMut(&mut Self, &T) -> LoadResult<()>,
    ) -> LoadResult<()> {
        let mut previous: Option<u64> = None;
        self.each_row(key, rows, |b, item| {
            let channel = channel_of(item);
            if let Some(previous_channel) = previous.filter(|previous| channel < *previous) {
                let kind = LoadErrorKind::SiblingsOutOfOrder {
                    previous_channel,
                    previous_id: None,
                };
                return Err(b.out_of_order(channel, kind));
            }
            previous = Some(channel);
            f(b, item)
        })
    }

    /// A [`SiblingsOutOfOrder`](LoadErrorKind::SiblingsOutOfOrder)
    /// refusal of the element at the current path, whose channel is
    /// `channel`: at its `channel` key, or at the element itself when the
    /// channel is 0 — the default, which a file writes by leaving the key
    /// out, so that the path always names something the text holds.
    pub fn out_of_order(&self, channel: u64, kind: LoadErrorKind) -> LoadError {
        if channel == 0 {
            self.error(kind)
        } else {
            self.error_at("channel", kind)
        }
    }

    /// Add `element` under `parent`, let `under` add what lies under it,
    /// then check the element's row against the shape rules
    /// ([`validate_shape`]). **The only way to put an element in the
    /// tree**, so none is added without its check.
    ///
    /// The add is the O(1) append at the end of the parent's row, since
    /// the builder adds a row in key order.
    pub fn element(
        &mut self,
        parent: NodeId,
        element: DocElement,
        under: impl FnOnce(&mut Self, NodeId) -> LoadResult<()>,
    ) -> LoadResult<NodeId> {
        let kind = element.kind();
        let id = self.add(parent, element)?;
        under(self, id)?;
        self.finish_element(id)?;
        // A check frees a row's item once it is judged: a node may hold
        // any number of sections, links, rings or spans, and the check
        // holds one at a time. The rows that bound how many their parent
        // holds (a frame, a connector, a cap, a label, a portal end) stay
        // until their parent is judged; a row of any number is never
        // counted in its parent's judgment.
        if self.keep == Keep::OneNode
            && matches!(
                kind,
                DocKind::Section | DocKind::Ring | DocKind::Span | DocKind::Link
            )
        {
            self.free(id)?;
        }
        Ok(id)
    }

    /// Append `element` under `parent`: the O(1) end of its row, since
    /// the builder adds a row in key order. [`Self::element`] calls it,
    /// and so does the node build, which checks a node's row when the
    /// node leaves the chain rather than here.
    ///
    /// Every element is counted here, and the one past
    /// [`Self::max_elements`] is refused with
    /// [`TooManyElements`](LoadErrorKind::TooManyElements) before it is
    /// appended.
    pub(super) fn add(&mut self, parent: NodeId, element: DocElement) -> LoadResult<NodeId> {
        self.count(1)?;
        self.derived.appended(&element);
        let id = self
            .tree
            .insert_child(parent, element, Placement::Last)
            .map_err(|error| {
                self.error(LoadErrorKind::Internal {
                    message: error.to_string(),
                })
            })?;
        self.live += 1;
        self.live_peak = self.live_peak.max(self.live);
        Ok(id)
    }

    /// Count `more` elements, or definitions, against
    /// [`Self::max_elements`] and the arena's capacity
    /// ([`ARENA_MAX_ELEMENTS`](super::ARENA_MAX_ELEMENTS)): past either
    /// is [`TooManyElements`](LoadErrorKind::TooManyElements) at the
    /// current path, before anything is built.
    pub(super) fn count(&mut self, more: usize) -> LoadResult<()> {
        self.elements = self
            .elements
            .saturating_add(u64::try_from(more).unwrap_or(u64::MAX));
        let max = super::element_bound(self.max_elements);
        if self.elements > max {
            return Err(self.error(LoadErrorKind::TooManyElements { max }));
        }
        Ok(())
    }

    /// Under [`Keep::OneNode`], free `id` and everything under it.
    pub(super) fn free(&mut self, id: NodeId) -> LoadResult<()> {
        let count = id.descendants(self.tree.arena()).count();
        self.tree.remove_subtree(id).map_err(|error| {
            self.error(LoadErrorKind::Internal {
                message: error.to_string(),
            })
        })?;
        self.live = self.live.saturating_sub(count);
        Ok(())
    }

    /// Check the shape rules for the element at `id`, whose children
    /// are all in.
    pub(super) fn finish_element(&mut self, id: NodeId) -> LoadResult<()> {
        self.checked += 1;
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

    /// A font size in the unit its place takes
    /// ([`Unit`](crate::document::place::Unit), the rule the mutations
    /// share), checked against the bound for that unit. A file's number
    /// has no unit of its own: the loader always knows the place it
    /// reads one in, so it never asks for
    /// [`Unit::Either`](crate::document::place::Unit::Either), and would read one as
    /// points, the unit a span's place takes.
    pub fn size(&self, value: f32, unit: crate::document::place::Unit) -> LoadResult<FontSize> {
        use crate::document::place::Unit;
        match unit {
            Unit::Points | Unit::Either => self
                .bounded("size", bounds::check_font_size_pt(value))
                .map(FontSize::Pt),
            Unit::Relative => self
                .bounded("size", bounds::check_rel_size(value))
                .map(FontSize::Rel),
        }
    }

    /// A `channel`, checked against [`bounds::MAX_CHANNEL`] as a `u64`,
    /// so that every target refuses the same files: a `usize` is 32 bits
    /// on wasm32, and reading the number into one would refuse a smaller
    /// range there, with another message.
    pub fn channel(&self, value: u64) -> LoadResult<usize> {
        let channel = usize::try_from(value)
            .map_err(|_| BoundError::NotAChannel(value))
            .and_then(|channel| bounds::check_channel(channel).map(|()| channel));
        self.bounded("channel", channel)
    }

    /// A font weight, checked.
    pub fn weight(&self, value: u16) -> LoadResult<Weight> {
        self.bounded("weight", Weight::new(value))
    }

    /// The face a font family name stands for. A name this build has no
    /// face for is an error, and so is one that is only an installed
    /// system font: it resolves to the fallback face
    /// ([`AppFont::Any`]), which has no name a file could write, so
    /// the document could not save it back. Which faces a document may
    /// hold is [`nameable_font`](crate::document::place::nameable_font),
    /// the rule the mutations share.
    pub fn font(&mut self, family: &str) -> LoadResult<AppFont> {
        let found = match self.fonts.get(family) {
            Some(found) => *found,
            None => {
                let found =
                    app_font_by_family(family).filter(|font| crate::document::place::nameable_font(*font));
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
            // A filler is held to the sides' bounds, since round a frame
            // it is its sides when they are unset (ruling R35.4).
            FitDto::Once(None) => Ok(FitMode::Once { filler: None }),
            FitDto::Once(Some(once)) => b.key("once", |b| {
                let filler = b.bounded("filler", bounds::check_ornament(&once.filler))?;
                Ok(FitMode::Once {
                    filler: Some(filler.to_owned()),
                })
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

    /// A zoom range, checked, at `key`, by the rule the mutations share
    /// ([`bounds::check_zoom_window`]): each bound given is finite and
    /// above `0` — a camera zoom is always above `0`, so a bound at or
    /// below it is never reached — and `min` is not above `max`. A bound
    /// that fails alone is reported at it, an inverted window at the
    /// window.
    pub fn zoom(
        &mut self,
        key: &'static str,
        zoom: &crate::document::format::dto::ZoomDto,
    ) -> LoadResult<ZoomWindow> {
        self.key(key, |b| {
            b.bounded("min", bounds::check_zoom_window(zoom.min, None))?;
            b.bounded("max", bounds::check_zoom_window(None, zoom.max))?;
            b.bounded_here(bounds::check_zoom_window(zoom.min, zoom.max))?;
            Ok(ZoomWindow {
                min: zoom.min,
                max: zoom.max,
            })
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

/// The arena's capacity as a load's element bound (V1-A7, #301). Inline
/// because it drives [`Builder::count`], which the format's tests trees
/// cannot reach, at a count no map in a test could be built to.
#[cfg(test)]
mod arena_bound_tests {
    use super::{Builder, Keep};
    use crate::document::format::dto::MapHeaderDto;
    use crate::document::format::error::LoadErrorKind;
    use crate::document::format::{
        begin_build, element_bound, index, load, node_reservation, Limits, ARENA_MAX_ELEMENTS, LIMITS,
        MIN_NODE_TEXT,
    };

    /// With no target bound — native's — the arena's capacity is the
    /// bound: the element that fills the arena is counted, and the one
    /// past it is refused with `TooManyElements` naming the capacity,
    /// where the arena itself would panic.
    #[test]
    fn test_a_load_past_the_arenas_capacity_is_refused_not_a_panic() {
        let header = MapHeaderDto {
            format: 1,
            name: String::new(),
            theme: None,
            background: None,
            palettes: Default::default(),
            defaults: None,
            edits: Vec::new(),
            presentations: Vec::new(),
            verbs: Vec::new(),
        };
        let mut builder = Builder::begin(&header, Keep::Document, None).expect("an empty header builds");
        builder.elements = ARENA_MAX_ELEMENTS - 1;
        assert!(
            builder.count(1).is_ok(),
            "precondition: the element that fills the arena is counted"
        );
        let refused = builder
            .count(1)
            .expect_err("one past the arena's capacity is refused");
        assert_eq!(
            refused.kind,
            LoadErrorKind::TooManyElements {
                max: ARENA_MAX_ELEMENTS
            }
        );
    }

    /// The bound is the lower of the target's and the arena's.
    #[test]
    fn test_the_element_bound_is_the_lower_of_the_targets_and_the_arenas() {
        assert_eq!(ARENA_MAX_ELEMENTS, u64::from(u32::MAX) - 1);
        assert_eq!(element_bound(None), ARENA_MAX_ELEMENTS);
        assert_eq!(element_bound(Some(10)), 10);
        assert_eq!(element_bound(Some(u64::MAX)), ARENA_MAX_ELEMENTS);
    }

    /// A map of one `node`.
    fn map_of(node: &str) -> String {
        format!(r#"{{"format":1,"name":"m","nodes":[{node}]}}"#)
    }

    /// The shortest node that loads is [`MIN_NODE_TEXT`] bytes: it
    /// loads, and leaving out either key or writing the position
    /// shorter is refused — the id is one digit and the text has no
    /// whitespace already.
    #[test]
    fn test_the_shortest_node_that_loads_is_min_node_text_bytes() {
        let shortest = r#"{"id":1,"position":[0,0]}"#;
        assert_eq!(shortest.len(), MIN_NODE_TEXT);
        assert!(load(&map_of(shortest)).is_ok(), "the shortest node loads");
        for shorter in [
            r#"{"position":[0,0]}"#,
            r#"{"id":1}"#,
            r#"{"id":1,"position":[0]}"#,
        ] {
            assert!(
                load(&map_of(shorter)).is_err(),
                "{shorter}, shorter, must not load"
            );
        }
    }

    /// A list of a million `0`s, refused at its first entry, makes no
    /// more room than its text could hold as nodes that load: the
    /// arena's room is the text's length over [`MIN_NODE_TEXT`] beside
    /// the header's elements, not a slot per entry.
    #[test]
    fn test_a_list_of_tiny_entries_reserves_no_more_than_its_text_holds() {
        let text = map_of(&vec!["0"; 1_000_000].join(","));
        let index = index::index(&text).expect("the text indexes");
        assert_eq!(index.nodes.len(), 1_000_000, "precondition: a million entries");
        let error = load(&text).expect_err("an entry of 0 is no node");
        assert_eq!(error.path, "nodes[0]", "precondition: refused at its first entry");

        let builder = begin_build(&index, LIMITS).expect("the header builds");
        let arena = builder.tree.arena();
        let cap = text.len() / MIN_NODE_TEXT;
        assert!(cap < 100_000, "precondition: the text holds {cap} nodes at most");
        assert!(
            arena.capacity() >= cap,
            "precondition: the room was made ({})",
            arena.capacity()
        );
        assert!(
            arena.capacity() <= arena.len() + cap,
            "{} slots of room for {} entries, past the {cap} the text could hold",
            arena.capacity(),
            index.nodes.len()
        );
        assert_eq!(node_reservation(1_000_000, text.len(), LIMITS), cap);
        let bounded = Limits {
            bytes: None,
            elements: Some(10),
        };
        assert_eq!(
            node_reservation(1_000_000, text.len(), bounded),
            10,
            "the element bound caps the room too"
        );
    }
}
