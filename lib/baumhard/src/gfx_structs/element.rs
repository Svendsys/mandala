// SPDX-License-Identifier: MPL-2.0

//! `GfxElement` — the value type every `GfxTree` node
//! holds. Five variants cover the four renderable glyph types
//! (`GlyphArea` for text regions, `GlyphModel` for composed glyph
//! shapes, `GlyphRun` for a whole shaped and placed TextFlow, `Fill`
//! for a node background) plus a lightweight `Void` placeholder used
//! to pad tree structure without rendering cost. This module also owns the
//! field-selector enum (`GfxElementField`) used by the mutator
//! pipeline to address a single facet of an element, the
//! discriminant tag (`GfxElementType`), and the per-node AABB
//! caching that accelerates the hit-test BVH descent.

use crate::core::primitives::{ColorFontRegionField, Flag, Flaggable, Range};
use crate::document::values::Rgba;
use crate::engine::element::TreeElement;
use crate::engine::interpolate::{Interpolable, Interpolate, Snap};
use crate::flow::Rect;
use crate::gfx_structs::area::{GlyphArea, GlyphAreaField};
use crate::gfx_structs::fill::FillData;
use crate::gfx_structs::model::{GlyphModel, GlyphModelField};
use crate::gfx_structs::mutator::GlyphTreeEventInstance;
use crate::gfx_structs::run::{GlyphOverrides, GlyphRunData};
use crate::gfx_structs::tree::{EventSubscriber, GlyphTreeCache, TreeEventConsumer};
use crate::util::color::FloatRgba;
use crate::util::geometry::clockwise_rotation_around_pivot;
use crate::util::ordered_vec2::OrderedVec2;
use glam::Vec2;
use log::warn;
use ordered_float::OrderedFloat;
use rustc_hash::FxHashSet;
use serde::{Deserialize, Serialize};
use std::fmt::{Debug, Formatter};
use std::rc::Rc;
use strum_macros::EnumDiscriminants;

/// Identifies a single field-level change to a [`GfxElement`].
///
/// Used by the mutator pipeline to describe which part of an element is
/// being mutated. Each variant wraps the field enum of the corresponding
/// inner type (`GlyphAreaField`, `GlyphModelField`) or targets
/// element-level metadata (channel, id, flag). Cost: O(1) construction,
/// no allocation beyond what the inner field enum carries.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum GfxElementField {
    /// A field change targeting the inner [`GlyphArea`].
    GlyphArea(GlyphAreaField),
    /// A field change targeting the inner [`GlyphModel`].
    GlyphModel(GlyphModelField),
    /// A field change targeting a specific [`ColorFontRegion`](crate::core::primitives::ColorFontRegion)
    /// within the given [`Range`].
    Region(Range, ColorFontRegionField),
    /// Reassign the element's branch channel index.
    Channel(usize),
    /// Reassign the element's unique id.
    Id(usize),
    /// Toggle or set a [`Flag`] on the element.
    Flag(Flag),
    /// The element's variant, so a predicate can select runs or fills
    /// (or areas, models, voids) from among their siblings. `Equals`
    /// compares the variant; `Exists` is always present, like a
    /// channel; an ordering comparator on a tag has no meaning and is a
    /// non-match.
    ElementType(GfxElementType),
    /// The opacity of a [`GlyphRun`](GfxElement::GlyphRun) or
    /// [`Fill`](GfxElement::Fill), compared with the comparator's
    /// ordering; any other element does not match.
    Opacity(OrderedFloat<f32>),
}

/// A node element in the Baumhard glyph tree.
///
/// Every node in a [`GfxTree`](crate::gfx_structs::tree::GfxTree)
/// stores one `GfxElement`. The five variants cover the four renderable
/// glyph types ([`GlyphArea`] for text regions, [`GlyphModel`] for
/// composed glyph shapes, [`GlyphRun`](GfxElement::GlyphRun) for a
/// whole shaped TextFlow, [`Fill`](GfxElement::Fill) for a node
/// background) and a lightweight [`Void`](GfxElement::Void)
/// placeholder used to pad tree structure without rendering cost.
///
/// All variants carry a `channel` (branch routing), a `unique_id`
/// (application-assigned identity), a set of [`Flag`]s, and a list of
/// [`EventSubscriber`] callbacks.
///
/// Cost: construction allocates a `Box` for the inner glyph type
/// (except `Void`). Field access is O(1) with a single match.
///
/// `Clone` copies every field — payload, flags, channel, id and the
/// subscriber handles (`Rc` clones, not new callbacks). A journal's
/// before-image is a clone, so a clone that dropped a field would make
/// the journal's `revert` drop it too.
///
/// The payload-free `GfxElementType` tag is derived from this enum
/// and reached through
/// [`Discriminated::variant`](crate::core::primitives::Discriminated::variant).
#[derive(Clone, EnumDiscriminants)]
#[strum_discriminants(name(GfxElementType))]
#[strum_discriminants(derive(Hash, Serialize, Deserialize))]
#[strum_discriminants(doc = "Payload-free tag for [`GfxElement`], derived by strum. Lets a")]
#[strum_discriminants(doc = "caller branch on the variant without destructuring the full")]
#[strum_discriminants(doc = "element. O(1), no allocation.")]
pub enum GfxElement {
    /// A text region rendered through cosmic-text layout.
    ///
    /// Wraps a heap-allocated [`GlyphArea`] that owns the text content,
    /// scale, position, color-font regions, and hit-box. This is the
    /// primary renderable element for mindmap nodes, labels, and borders.
    GlyphArea {
        glyph_area: Box<GlyphArea>,
        flags: FxHashSet<Flag>,
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
    },
    /// A composed glyph shape built from lines and components.
    ///
    /// Wraps a heap-allocated [`GlyphModel`]. Typically a child of a
    /// [`GlyphArea`] node — the model provides the visual geometry while
    /// the parent area supplies the text-layout context.
    GlyphModel {
        glyph_model: Box<GlyphModel>,
        flags: FxHashSet<Flag>,
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
    },
    /// One whole TextFlow: what was shaped, where its glyphs sit, and
    /// the per-glyph effects on top (design §13.2).
    ///
    /// Wraps a heap-allocated [`GlyphRunData`] that shares its shaping
    /// by `Arc`. One element whatever the glyph count: a run is never
    /// split into per-glyph elements. Nothing renders it until V1-C2.
    GlyphRun {
        run: Box<GlyphRunData>,
        flags: FxHashSet<Flag>,
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
    },
    /// A node background: a color shaped like the node's outline with a
    /// soft edge fade (design §13.4).
    ///
    /// Wraps a heap-allocated [`FillData`]. Hit-tested by its rectangle.
    Fill {
        fill: Box<FillData>,
        flags: FxHashSet<Flag>,
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
    },
    /// A no-op placeholder node that produces no rendering output.
    ///
    /// Used to pad tree structure so that a mutator tree can target
    /// specific positions without requiring a renderable element at
    /// every slot. Carries only metadata (channel, id, flags,
    /// subscribers) and no glyph payload.
    Void {
        channel: usize,
        unique_id: usize,
        event_subscribers: Vec<EventSubscriber>,
        flags: FxHashSet<Flag>,
    },
}

impl GfxElement {
    /// Create a [`GlyphArea`] element with `unique_id` defaulting to `0`.
    ///
    /// Convenience wrapper around [`new_area_non_indexed_with_id`](Self::new_area_non_indexed_with_id).
    /// Allocates a `Box<GlyphArea>`. The element is *not* inserted into a
    /// region index — callers that need indexing must register it
    /// separately.
    pub fn new_area_non_indexed(section: GlyphArea, channel: usize) -> GfxElement {
        Self::new_area_non_indexed_with_id(section, channel, 0)
    }
    /// Create a [`GlyphArea`] element with an explicit `unique_id`.
    ///
    /// * `section` — the [`GlyphArea`] payload (text, scale, position, bounds).
    /// * `channel` — branch-routing index for the tree walker.
    /// * `unique_id` — application-assigned identity for this element.
    ///
    /// Cost: one heap allocation (`Box<GlyphArea>`). Flags and
    /// subscribers start empty.
    pub fn new_area_non_indexed_with_id(section: GlyphArea, channel: usize, unique_id: usize) -> GfxElement {
        GfxElement::GlyphArea {
            glyph_area: Box::new(section),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
        }
    }

    /// Create a [`Void`](GfxElement::Void) placeholder with `unique_id`
    /// defaulting to `0`.
    ///
    /// No heap allocation — `Void` carries only metadata.
    pub fn new_void(channel: usize) -> GfxElement {
        Self::new_void_with_id(channel, 0)
    }

    /// Create a [`Void`](GfxElement::Void) placeholder with an explicit
    /// `unique_id`.
    ///
    /// * `channel` — branch-routing index.
    /// * `unique_id` — application-assigned identity.
    ///
    /// Cost: no heap allocation. Flags and subscribers start empty.
    pub fn new_void_with_id(channel: usize, unique_id: usize) -> GfxElement {
        GfxElement::Void {
            channel,
            unique_id,
            event_subscribers: vec![],
            flags: Default::default(),
        }
    }
    /// Create a [`GlyphModel`] element. Delegates to
    /// [`new_model_non_indexed_with_id`](Self::new_model_non_indexed_with_id).
    ///
    /// Cost: one heap allocation (`Box<GlyphModel>`).
    pub fn new_model_non_indexed(model: GlyphModel, channel: usize, unique_id: usize) -> Self {
        Self::new_model_non_indexed_with_id(model, channel, unique_id)
    }

    /// Create a [`GlyphModel`] element with an explicit `unique_id`.
    ///
    /// * `model` — the [`GlyphModel`] payload (lines, components, position).
    /// * `channel` — branch-routing index.
    /// * `unique_id` — application-assigned identity.
    ///
    /// Cost: one heap allocation (`Box<GlyphModel>`). Flags and
    /// subscribers start empty.
    pub fn new_model_non_indexed_with_id(model: GlyphModel, channel: usize, unique_id: usize) -> Self {
        GfxElement::GlyphModel {
            glyph_model: Box::new(model),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
        }
    }

    /// Create a [`GlyphModel`] element with a default (empty) model.
    /// Delegates to [`new_model_blank_with_id`](Self::new_model_blank_with_id).
    ///
    /// Cost: one heap allocation for the empty `Box<GlyphModel>`.
    pub fn new_model_blank(channel: usize, unique_id: usize) -> GfxElement {
        Self::new_model_blank_with_id(channel, unique_id)
    }

    /// Create a [`GlyphModel`] element wrapping a default-constructed
    /// [`GlyphModel::new()`].
    ///
    /// * `channel` — branch-routing index.
    /// * `unique_id` — application-assigned identity.
    ///
    /// Cost: one heap allocation (`Box<GlyphModel>`). Flags and
    /// subscribers start empty.
    pub fn new_model_blank_with_id(channel: usize, unique_id: usize) -> GfxElement {
        GfxElement::GlyphModel {
            glyph_model: Box::new(GlyphModel::new()),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
        }
    }

    /// Create a [`GlyphRun`](GfxElement::GlyphRun) element.
    ///
    /// * `run` — the validated run payload ([`GlyphRunData::new`]).
    /// * `channel` — branch-routing index.
    /// * `unique_id` — application-assigned identity.
    ///
    /// Cost: one heap allocation (`Box<GlyphRunData>`). Flags and
    /// subscribers start empty.
    pub fn new_run(run: GlyphRunData, channel: usize, unique_id: usize) -> GfxElement {
        GfxElement::GlyphRun {
            run: Box::new(run),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
        }
    }

    /// Create a [`Fill`](GfxElement::Fill) element.
    ///
    /// * `fill` — the fill payload.
    /// * `channel` — branch-routing index.
    /// * `unique_id` — application-assigned identity.
    ///
    /// Cost: one heap allocation (`Box<FillData>`). Flags and
    /// subscribers start empty.
    pub fn new_fill(fill: FillData, channel: usize, unique_id: usize) -> GfxElement {
        GfxElement::Fill {
            fill: Box::new(fill),
            flags: Default::default(),
            channel,
            unique_id,
            event_subscribers: vec![],
        }
    }

    /// Return a mutable reference to this element's event-subscriber list.
    ///
    /// Works across all variants. O(1) — single match, no allocation.
    pub fn subscribers_mut(&mut self) -> &mut Vec<EventSubscriber> {
        match self {
            GfxElement::GlyphArea {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::GlyphModel {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::GlyphRun {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::Fill {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::Void {
                event_subscribers, ..
            } => event_subscribers,
        }
    }

    /// Return a shared reference to this element's event-subscriber list.
    ///
    /// Works across all variants. O(1) — single match, no allocation.
    pub fn subscribers_as_ref(&self) -> &Vec<EventSubscriber> {
        match self {
            GfxElement::GlyphArea {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::GlyphModel {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::GlyphRun {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::Fill {
                event_subscribers, ..
            } => event_subscribers,
            GfxElement::Void {
                event_subscribers, ..
            } => event_subscribers,
        }
    }

    /// Overwrite the element's `unique_id`.
    ///
    /// * `id` — the new identity value.
    ///
    /// Works across all variants. O(1), no allocation.
    pub fn set_unique_id(&mut self, id: usize) {
        match self {
            GfxElement::GlyphArea { unique_id, .. } => *unique_id = id,
            GfxElement::GlyphModel { unique_id, .. } => *unique_id = id,
            GfxElement::GlyphRun { unique_id, .. } => *unique_id = id,
            GfxElement::Fill { unique_id, .. } => *unique_id = id,
            GfxElement::Void { unique_id, .. } => *unique_id = id,
        }
    }

    /// Return a mutable reference to the inner [`GlyphArea`], or `None`
    /// if this element is not a `GlyphArea` variant.
    ///
    /// O(1) match, no allocation.
    pub fn glyph_area_mut(&mut self) -> Option<&mut GlyphArea> {
        match self {
            GfxElement::GlyphArea {
                glyph_area: section, ..
            } => Some(section.as_mut()),
            GfxElement::Void { .. }
            | GfxElement::GlyphModel { .. }
            | GfxElement::GlyphRun { .. }
            | GfxElement::Fill { .. } => None,
        }
    }

    /// Return a shared reference to the inner [`GlyphArea`], or `None`
    /// if this element is not a `GlyphArea` variant.
    ///
    /// O(1) match, no allocation.
    pub fn glyph_area(&self) -> Option<&GlyphArea> {
        match self {
            GfxElement::GlyphArea {
                glyph_area: section, ..
            } => Some(section),
            GfxElement::Void { .. }
            | GfxElement::GlyphModel { .. }
            | GfxElement::GlyphRun { .. }
            | GfxElement::Fill { .. } => None,
        }
    }

    /// Return a shared reference to the inner [`GlyphModel`], or `None`
    /// if this element is not a `GlyphModel` variant.
    ///
    /// O(1) match, no allocation.
    pub fn glyph_model(&self) -> Option<&GlyphModel> {
        match self {
            GfxElement::GlyphModel { glyph_model, .. } => Some(glyph_model),
            _ => None,
        }
    }

    /// Return a mutable reference to the inner [`GlyphModel`], or `None`
    /// if this element is not a `GlyphModel` variant.
    ///
    /// O(1) match, no allocation.
    pub fn glyph_model_mut(&mut self) -> Option<&mut GlyphModel> {
        match self {
            GfxElement::GlyphModel { glyph_model, .. } => Some(glyph_model),
            _ => None,
        }
    }

    /// Return a shared reference to the inner [`GlyphRunData`], or
    /// `None` if this element is not a `GlyphRun` variant.
    ///
    /// O(1) match, no allocation.
    pub fn glyph_run(&self) -> Option<&GlyphRunData> {
        match self {
            GfxElement::GlyphRun { run, .. } => Some(run),
            _ => None,
        }
    }

    /// Return a shared reference to the inner [`FillData`], or `None`
    /// if this element is not a `Fill` variant.
    ///
    /// O(1) match, no allocation.
    pub fn fill(&self) -> Option<&FillData> {
        match self {
            GfxElement::Fill { fill, .. } => Some(fill),
            _ => None,
        }
    }

    /// Return the application-assigned identity for this element.
    ///
    /// Works across all variants. O(1), no allocation.
    pub fn unique_id(&self) -> usize {
        match self {
            GfxElement::GlyphArea { unique_id, .. } => *unique_id,
            GfxElement::Void { unique_id, .. } => *unique_id,
            GfxElement::GlyphModel { unique_id, .. } => *unique_id,
            GfxElement::GlyphRun { unique_id, .. } => *unique_id,
            GfxElement::Fill { unique_id, .. } => *unique_id,
        }
    }

    /// Return the world-space position of this element.
    ///
    /// * `GlyphArea` / `GlyphModel` — returns the stored position as a
    ///   `Vec2`.
    /// * `GlyphRun` — the top-left of its bounds. `Fill` — the top-left
    ///   of its rectangle.
    /// * `Void` — returns `Vec2::NAN` (void nodes have no spatial
    ///   meaning).
    ///
    /// O(1), no allocation.
    pub fn position(&self) -> Vec2 {
        match self {
            GfxElement::GlyphArea {
                glyph_area: section, ..
            } => section.position.to_vec2(),
            GfxElement::Void { .. } => Vec2::NAN,
            GfxElement::GlyphModel { glyph_model, .. } => glyph_model.position.to_vec2(),
            GfxElement::GlyphRun { run, .. } => run.bounds().0,
            GfxElement::Fill { fill, .. } => fill.rect.origin,
        }
    }

    /// Return the color stored in the color-font region at `range`,
    /// if one exists.
    ///
    /// * `GlyphArea` — looks up the region and returns its color.
    /// * `GlyphModel` / `Void` — returns `None` (no color-font
    ///   regions on these variants).
    ///
    /// O(log n) region lookup: `ColorFontRegions` is a `BTreeSet`
    /// keyed by range.
    pub fn color_at_region(&self, range: Range) -> Option<FloatRgba> {
        match self {
            GfxElement::GlyphArea { glyph_area, .. } => {
                glyph_area.regions.get(range).and_then(|region| region.color)
            }
            GfxElement::GlyphModel { .. }
            | GfxElement::GlyphRun { .. }
            | GfxElement::Fill { .. }
            | GfxElement::Void { .. } => None,
        }
    }

    /// Set the world-space position of this element.
    ///
    /// * `GlyphArea` / `GlyphModel` — overwrites the stored position.
    /// * `GlyphRun` — moves the run so its bounds' top-left is at
    ///   `position` (the origin shifts by the difference). `Fill` —
    ///   moves the rectangle's top-left to `position`.
    /// * `Void` — no-op (void nodes have no spatial meaning).
    ///
    /// O(1), no allocation.
    pub fn set_position(&mut self, position: Vec2) {
        match self {
            GfxElement::GlyphArea {
                ref mut glyph_area, ..
            } => {
                glyph_area.position = OrderedVec2::from_vec2(position);
            }
            GfxElement::Void { .. } => {
                // Nothing needs doing
            }
            GfxElement::GlyphModel {
                ref mut glyph_model, ..
            } => {
                glyph_model.position = OrderedVec2::from_vec2(position);
            }
            GfxElement::GlyphRun { ref mut run, .. } => {
                let shift = position - run.bounds().0;
                run.set_origin(run.origin() + shift);
            }
            GfxElement::Fill { ref mut fill, .. } => {
                fill.rect.origin = position;
            }
        }
    }

    /// Rotate this element's position clockwise around `pivot` by
    /// `degrees`.
    ///
    /// Void elements (whose position is `NAN`) are skipped. Delegates to
    /// [`clockwise_rotation_around_pivot`]. O(1), no allocation.
    pub fn rotate(&mut self, pivot: Vec2, degrees: f32) {
        let position = self.position();
        if !position.is_nan() {
            self.set_position(clockwise_rotation_around_pivot(position, pivot, degrees));
        }
    }
}

impl Flaggable for GfxElement {
    fn flag_is_set(&self, flag: Flag) -> bool {
        match self {
            GfxElement::GlyphArea { flags, .. } => flags.contains(&flag),
            GfxElement::GlyphModel { flags, .. } => flags.contains(&flag),
            GfxElement::GlyphRun { flags, .. } => flags.contains(&flag),
            GfxElement::Fill { flags, .. } => flags.contains(&flag),
            GfxElement::Void { .. } => false,
        }
    }

    fn set_flag(&mut self, flag: Flag) {
        match self {
            GfxElement::GlyphArea { flags, .. } => {
                flags.insert(flag);
            }
            GfxElement::GlyphModel { flags, .. } => {
                flags.insert(flag);
            }
            GfxElement::GlyphRun { flags, .. } => {
                flags.insert(flag);
            }
            GfxElement::Fill { flags, .. } => {
                flags.insert(flag);
            }
            GfxElement::Void { .. } => {}
        }
    }

    fn clear_flag(&mut self, flag: Flag) {
        match self {
            GfxElement::GlyphArea { flags, .. } => {
                flags.remove(&flag);
            }
            GfxElement::GlyphModel { flags, .. } => {
                flags.remove(&flag);
            }
            GfxElement::GlyphRun { flags, .. } => {
                flags.remove(&flag);
            }
            GfxElement::Fill { flags, .. } => {
                flags.remove(&flag);
            }
            GfxElement::Void { .. } => {}
        }
    }
}

impl Debug for GfxElement {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        match self {
            GfxElement::GlyphArea {
                channel,
                unique_id,
                glyph_area,
                ..
            } => {
                write!(
                    f,
                    "GlyphArea(id={}, ch={}, text={:?})",
                    unique_id, channel, glyph_area.text
                )
            }
            GfxElement::GlyphModel {
                channel, unique_id, ..
            } => {
                write!(f, "GlyphModel(id={}, ch={})", unique_id, channel)
            }
            GfxElement::GlyphRun {
                channel,
                unique_id,
                run,
                ..
            } => {
                write!(
                    f,
                    "GlyphRun(id={}, ch={}, glyphs={})",
                    unique_id,
                    channel,
                    run.shaped().glyph_count()
                )
            }
            GfxElement::Fill {
                channel, unique_id, ..
            } => {
                write!(f, "Fill(id={}, ch={})", unique_id, channel)
            }
            GfxElement::Void {
                channel, unique_id, ..
            } => {
                write!(f, "Void(id={}, ch={})", unique_id, channel)
            }
        }
    }
}

impl PartialEq for GfxElement {
    fn eq(&self, other: &Self) -> bool {
        // Match both discriminants in one step so each arm binds the
        // payload directly — no `variant()` + `unwrap()` dance, and
        // no mixed-variant unwrap panic is reachable by construction.
        match (self, other) {
            (GfxElement::GlyphArea { glyph_area: a, .. }, GfxElement::GlyphArea { glyph_area: b, .. }) => {
                a == b
            }
            (
                GfxElement::GlyphModel { glyph_model: a, .. },
                GfxElement::GlyphModel { glyph_model: b, .. },
            ) => a == b,
            (GfxElement::GlyphRun { run: a, .. }, GfxElement::GlyphRun { run: b, .. }) => a == b,
            (GfxElement::Fill { fill: a, .. }, GfxElement::Fill { fill: b, .. }) => a == b,
            (GfxElement::Void { .. }, GfxElement::Void { .. }) => true,
            _ => false,
        }
    }
}

impl TreeEventConsumer for GfxElement {
    fn accept_event(&mut self, event: &GlyphTreeEventInstance) {
        // Snapshot the `Rc` handles before invoking any callback.
        // Callbacks receive `&mut self` and may mutate the subscriber
        // list (e.g. remove themselves), so we must not index the live
        // list during delivery. The previous code cloned the whole
        // `Vec<EventSubscriber>`; we clone only the `Rc` handles, which
        // is one refcount bump per subscriber instead of an allocation
        // per event per element.
        let subs: Vec<_> = self.subscribers_as_ref().iter().map(Rc::clone).collect();
        for sub in subs {
            let mut callback = match sub.try_borrow_mut() {
                Ok(cb) => cb,
                Err(_) => {
                    // Re-entrant delivery: a subscriber's reaction
                    // re-delivered an event to the same element.
                    // Skip rather than deadlock/panic.
                    warn!("gfx_structs::element: EventSubscriber already borrowed (re-entrant delivery); skipping");
                    continue;
                }
            };
            callback(self, event.clone());
        }
    }
}

/// The glyph instantiation of the engine. One kind, so routing stays
/// channel-only; no inherited state; the AABB memo and region index
/// are the tree's [`GlyphTreeCache`]. Spatial: a `GlyphArea` with a
/// positive render size answers [`TreeElement::spatial_bounds`]
/// with its position and render bounds, a `GlyphRun` with its bounds
/// and a `Fill` with its rectangle; a model or a `Void` has no
/// box of its own. A glyph element is never split or merged
/// (`SplitPoint = ()`, the default `split` and `merge`), so a glyph
/// tree refuses those two structural commands.
impl TreeElement for GfxElement {
    type Kind = ();
    type Cache = GlyphTreeCache;
    type Inherited = ();
    type SplitPoint = ();
    const SPATIAL: bool = true;

    fn kind(&self) {}

    fn channel(&self) -> usize {
        match self {
            GfxElement::GlyphArea { channel, .. } => *channel,
            GfxElement::Void { channel, .. } => *channel,
            GfxElement::GlyphModel { channel, .. } => *channel,
            GfxElement::GlyphRun { channel, .. } => *channel,
            GfxElement::Fill { channel, .. } => *channel,
        }
    }

    fn void() -> Self {
        Self::new_void_with_id(0, 0)
    }

    /// The same variant with the same payload state
    /// ([`GlyphArea::same_state`], [`GlyphModel::same_state`],
    /// [`GlyphRunData::same_state`] — which compares the shaping by
    /// `Arc` identity — and [`FillData::same_state`]), the
    /// same flags, channel and unique id. The event subscribers are
    /// callbacks, not state, and are not compared; the subtree AABBs
    /// are derived, and live in the tree's [`GlyphTreeCache`], not on
    /// the element.
    fn same_state(&self, other: &Self) -> bool {
        match (self, other) {
            (
                GfxElement::GlyphArea {
                    glyph_area,
                    flags,
                    channel,
                    unique_id,
                    event_subscribers: _,
                },
                GfxElement::GlyphArea {
                    glyph_area: other_area,
                    flags: other_flags,
                    channel: other_channel,
                    unique_id: other_id,
                    event_subscribers: _,
                },
            ) => {
                glyph_area.same_state(other_area)
                    && flags == other_flags
                    && channel == other_channel
                    && unique_id == other_id
            }
            (
                GfxElement::GlyphModel {
                    glyph_model,
                    flags,
                    channel,
                    unique_id,
                    event_subscribers: _,
                },
                GfxElement::GlyphModel {
                    glyph_model: other_model,
                    flags: other_flags,
                    channel: other_channel,
                    unique_id: other_id,
                    event_subscribers: _,
                },
            ) => {
                glyph_model.same_state(other_model)
                    && flags == other_flags
                    && channel == other_channel
                    && unique_id == other_id
            }
            (
                GfxElement::GlyphRun {
                    run,
                    flags,
                    channel,
                    unique_id,
                    event_subscribers: _,
                },
                GfxElement::GlyphRun {
                    run: other_run,
                    flags: other_flags,
                    channel: other_channel,
                    unique_id: other_id,
                    event_subscribers: _,
                },
            ) => {
                run.same_state(other_run)
                    && flags == other_flags
                    && channel == other_channel
                    && unique_id == other_id
            }
            (
                GfxElement::Fill {
                    fill,
                    flags,
                    channel,
                    unique_id,
                    event_subscribers: _,
                },
                GfxElement::Fill {
                    fill: other_fill,
                    flags: other_flags,
                    channel: other_channel,
                    unique_id: other_id,
                    event_subscribers: _,
                },
            ) => {
                fill.same_state(other_fill)
                    && flags == other_flags
                    && channel == other_channel
                    && unique_id == other_id
            }
            (
                GfxElement::Void {
                    channel,
                    unique_id,
                    flags,
                    event_subscribers: _,
                },
                GfxElement::Void {
                    channel: other_channel,
                    unique_id: other_id,
                    flags: other_flags,
                    event_subscribers: _,
                },
            ) => channel == other_channel && unique_id == other_id && flags == other_flags,
            _ => false,
        }
    }

    fn inherit(&self, _parent: &()) {}

    /// The element's own box as `(top_left, size)`, when its size is
    /// positive on both axes — the same box, and the same positivity
    /// rule, as the BVH behind `Tree::descendant_at` uses:
    ///
    /// - a `GlyphArea`: its position and render bounds;
    /// - a `GlyphRun`: its [`bounds`](GlyphRunData::bounds);
    /// - a `Fill`: its rectangle.
    ///
    /// A model or a `Void` has no box of its own.
    fn spatial_bounds(&self) -> Option<(Vec2, Vec2)> {
        let (top_left, size) = match self {
            GfxElement::GlyphArea { glyph_area, .. } => {
                (glyph_area.position.to_vec2(), glyph_area.render_bounds.to_vec2())
            }
            GfxElement::GlyphRun { run, .. } => run.bounds(),
            GfxElement::Fill { fill, .. } => (fill.rect.origin, fill.rect.size),
            GfxElement::GlyphModel { .. } | GfxElement::Void { .. } => return None,
        };
        (size.x > 0.0 && size.y > 0.0).then_some((top_left, size))
    }
}

/// The glyph instantiation's interpolation. Between two `GlyphArea`s,
/// `position`, `render_bounds`, `scale` and `line_height` interpolate,
/// and so does `background_color` when both sides have one (per
/// channel, rounded); between two `GlyphModel`s, `position`
/// interpolates; between two `GlyphRun`s, `origin`, `opacity` and each
/// override vector (element-wise, when the two vectors are the same
/// length, a color only where both sides have one); between two
/// `Fill`s, `rect`, `color` (per channel), `fade` and `opacity`. A run's
/// `shaped` and `zoom` and a fill's `shape` and `zoom` snap. Every other field — the text, the regions, a
/// background color present on one side only, the flags, the channel,
/// the id — takes the before or the after element by `snap`, and so
/// does the whole element when the two are different variants.
///
/// Cost: one clone of the element `snap` picks, plus O(1) per
/// interpolated field.
impl Interpolable for GfxElement {
    fn interpolate(before: &Self, after: &Self, t: f32, snap: Snap) -> Self {
        let mut out = snap.pick(before, after, t).clone();
        match (before, after, &mut out) {
            (
                GfxElement::GlyphArea { glyph_area: from, .. },
                GfxElement::GlyphArea { glyph_area: to, .. },
                GfxElement::GlyphArea { glyph_area: area, .. },
            ) => {
                area.position = OrderedVec2::lerp(&from.position, &to.position, t);
                area.render_bounds = OrderedVec2::lerp(&from.render_bounds, &to.render_bounds, t);
                area.scale = OrderedFloat::lerp(&from.scale, &to.scale, t);
                area.line_height = OrderedFloat::lerp(&from.line_height, &to.line_height, t);
                if let (Some(from_color), Some(to_color)) = (from.background_color, to.background_color) {
                    area.background_color = Some(<[u8; 4]>::lerp(&from_color, &to_color, t));
                }
            }
            (
                GfxElement::GlyphModel {
                    glyph_model: from, ..
                },
                GfxElement::GlyphModel { glyph_model: to, .. },
                GfxElement::GlyphModel {
                    glyph_model: model, ..
                },
            ) => {
                model.position = OrderedVec2::lerp(&from.position, &to.position, t);
            }
            (
                GfxElement::GlyphRun { run: from, .. },
                GfxElement::GlyphRun { run: to, .. },
                GfxElement::GlyphRun { run, .. },
            ) => {
                run.set_origin(<Vec2 as Interpolate>::lerp(&from.origin(), &to.origin(), t));
                run.set_opacity(f32::lerp(&from.opacity(), &to.opacity(), t));
                let overrides = GlyphOverrides::lerped(from.overrides(), to.overrides(), run.overrides(), t);
                run.set_overrides(overrides);
            }
            (
                GfxElement::Fill { fill: from, .. },
                GfxElement::Fill { fill: to, .. },
                GfxElement::Fill { fill, .. },
            ) => {
                fill.rect = Rect::new(
                    <Vec2 as Interpolate>::lerp(&from.rect.origin, &to.rect.origin, t),
                    <Vec2 as Interpolate>::lerp(&from.rect.size, &to.rect.size, t),
                );
                fill.color = Rgba::lerp(&from.color, &to.color, t);
                fill.fade = f32::lerp(&from.fade, &to.fade, t);
                fill.opacity = f32::lerp(&from.opacity, &to.opacity, t);
            }
            _ => {}
        }
        out
    }
}

impl Default for GfxElement {
    fn default() -> Self {
        Self::void()
    }
}
