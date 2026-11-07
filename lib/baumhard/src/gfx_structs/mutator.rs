// SPDX-License-Identifier: MPL-2.0

//! The glyph payload: [`GfxMutation`](crate::gfx_structs::mutator::GfxMutation), the
//! [`Mutation`](crate::engine::mutation::Mutation) every glyph
//! mutator carries, its [`GfxMutator`](crate::gfx_structs::mutator::GfxMutator) node alias, and the
//! `GlyphTreeEvent` side channel that a [`GfxMutation::Event`](crate::gfx_structs::mutator::GfxMutation::Event)
//! delivers to an element's subscribers.

use crate::core::primitives::{Applicable, Discriminated};
use crate::engine::mutation::Mutation;
use crate::engine::mutator::Mutator;
use crate::gfx_structs::area::{DeltaGlyphArea, GlyphArea, GlyphAreaCommand};
use crate::gfx_structs::element::GfxElement;
use crate::gfx_structs::model::{DeltaGlyphModel, GlyphModel, GlyphModelCommand};
use crate::gfx_structs::mutator::GfxMutation::{
    AreaCommand, AreaDelta, Event, ModelCommand, ModelDelta, RunOpacity, RunOrigin, RunOverrides, RunSet,
};
use crate::gfx_structs::run::{GlyphRunData, RunOverrideOp};
use crate::gfx_structs::template::CellField;
use crate::gfx_structs::tree::TreeEventConsumer;
use glam::Vec2;
use log::debug;
use ordered_float::OrderedFloat;
use serde::{Deserialize, Serialize};
use strum_macros::{Display, EnumDiscriminants};

/// One node of a glyph mutator tree: the engine's
/// [`Mutator`] instantiated over [`GfxElement`] and [`GfxMutation`].
pub type GfxMutator = Mutator<GfxElement, GfxMutation>;

/// A timestamped occurrence of a [`GlyphTreeEvent`] delivered to a
/// target element's event subscribers via
/// [`GfxMutation::Event`]. Unlike field mutations, events do not change
/// element data directly — they invoke the registered
/// [`EventSubscriber`](crate::gfx_structs::tree::EventSubscriber)
/// callbacks, which may in turn enqueue further mutations.
///
/// Cost: one allocation for the boxed closure dispatch per subscriber;
/// no arena walk.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct GlyphTreeEventInstance {
    /// The kind of event being delivered.
    pub event_type: GlyphTreeEvent,
    /// Milliseconds since application launch — used to order and
    /// deduplicate event sequences. Wraps after ~50 days; callers
    /// that sequence events across long sessions should account for
    /// rollover.
    pub event_time_millis: usize,
}

impl GlyphTreeEventInstance {
    /// Construct an event instance. O(1), no allocation.
    pub fn new(event_type: GlyphTreeEvent, event_time_millis: usize) -> Self {
        GlyphTreeEventInstance {
            event_type,
            event_time_millis,
        }
    }
}

/// Payload carried by [`GlyphTreeEvent::MouseEvent`]. Contains the
/// canvas-space coordinates of the mouse interaction so that the
/// receiving element (or its [`EventSubscriber`](crate::gfx_structs::tree::EventSubscriber))
/// knows *where* the event occurred.
///
/// Uses [`OrderedFloat`] so the struct is `Eq + Hash`, consistent
/// with other position types in baumhard (`OrderedVec2`, etc.).
///
/// Cost: 8 bytes, `Copy`.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Hash)]
pub struct MouseEventData {
    /// Canvas-space X coordinate.
    pub x: OrderedFloat<f32>,
    /// Canvas-space Y coordinate.
    pub y: OrderedFloat<f32>,
}

impl MouseEventData {
    /// Construct from raw `f32` coordinates. O(1), no allocation.
    pub fn new(x: f32, y: f32) -> Self {
        Self {
            x: OrderedFloat(x),
            y: OrderedFloat(y),
        }
    }
}

/// The kind of event a [`GlyphTreeEventInstance`] carries. Each
/// variant represents a category of stimulus that an element's
/// [`EventSubscriber`](crate::gfx_structs::tree::EventSubscriber)
/// may react to. Cheap to clone (`Copy`-like; inner data is at most
/// one `usize`).
#[derive(Clone, Debug, Serialize, Deserialize, Display, Eq, PartialEq)]
pub enum GlyphTreeEvent {
    /// Keyboard input events
    KeyboardEvent,
    /// Mouse input events with canvas-space coordinates.
    MouseEvent(MouseEventData),
    /// Events that are defined by the software application
    AppEvent,
    /// The recipient should start preparing to shut down now
    CloseEvent,
    /// The recipient will be terminated any time
    KillEvent,
    /// A mutation has been performed
    /// This allows EventSubscribers respond to mutations
    MutationEvent,
    /// This is used for testing mainly
    NoopEvent(usize),
}

/// A single atomic change that can be applied to a [`GfxElement`] —
/// the glyph instantiation's [`Mutation`] payload.
///
/// A [`GfxMutator::Single`] carries one, a [`GfxMutator::Macro`]
/// carries a `Vec` of them. They are dispatched through
/// [`Mutation::apply_to`], which routes to the appropriate element
/// type. Boxed variants keep the enum size uniform (one pointer +
/// discriminant) regardless of inner payload. There is no no-op
/// variant: a mutator that carries no payload says so structurally
/// (a `Void` node, or an `Instruction` whose `mutation` is `None`).
///
/// The payload-free `MutationType` tag is derived from this enum and
/// reached through
/// [`crate::core::primitives::Discriminated::variant`].
///
/// `PartialEq` is IEEE equality over the whole payload, derived. Not
/// `Eq`, and **not reflexive**: the numeric payloads are `f32`, so a
/// mutation carrying a `NaN` is not equal to itself. Use
/// [`GfxMutation::writes_the_same`] — not `==` — for "would these two
/// write the same thing", which is the question
/// `flat_mutations` (gone) asked before collapsing an AST's nested
/// payloads into one list.
/// `==` is still the right operator for an ordinary value
/// comparison; it is only the *agreement* predicate that needs the
/// reflexive form.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, EnumDiscriminants)]
#[strum_discriminants(name(MutationType))]
#[strum_discriminants(derive(Hash, Serialize, Deserialize))]
#[strum_discriminants(doc = "Payload-free tag for [`GfxMutation`], derived by strum. Lets a")]
#[strum_discriminants(doc = "caller inspect which kind of mutation is carried without")]
#[strum_discriminants(doc = "destructuring it. `Copy` — no heap cost.")]
pub enum GfxMutation {
    /// A field-level delta applied to a [`GlyphArea`]. The
    /// [`DeltaGlyphArea`] may add, assign, or subtract from one or
    /// more area fields depending on its
    /// [`ApplyOperation`](crate::core::primitives::ApplyOperation).
    /// Cost: O(k) in the number of fields in the delta.
    AreaDelta(Box<DeltaGlyphArea>),
    /// An imperative command applied to a [`GlyphArea`] (nudge,
    /// move-to, pop-text, font resize, etc.). Each
    /// [`GlyphAreaCommand`] variant encodes both the operation and
    /// its parameter. Cost: O(1) per command.
    AreaCommand(Box<GlyphAreaCommand>),
    /// A field-level delta applied to a [`GlyphModel`]. Semantics
    /// mirror [`GfxMutation::AreaDelta`] but target model fields
    /// (glyph matrix, layer, position).
    ModelDelta(Box<DeltaGlyphModel>),
    /// An imperative command applied to a [`GlyphModel`] (nudge,
    /// rotate, insert line, etc.). Mirrors
    /// [`GfxMutation::AreaCommand`] for model elements.
    ModelCommand(Box<GlyphModelCommand>),
    /// Delivers a [`GlyphTreeEventInstance`] to the target element's
    /// event subscribers. Does not modify element data directly;
    /// subscribers may enqueue further mutations in response.
    Event(GlyphTreeEventInstance),
    /// Replace the whole [`GlyphRun`](GfxElement::GlyphRun) payload: what
    /// projection writes when a flow is reshaped or re-placed. The
    /// bounds come with the new data. Cost: one clone of the payload —
    /// two `Arc` bumps and the override vectors.
    ///
    /// **Never authored, never serialized.** It holds `Arc`s of shaped
    /// text, which have no wire form, so it is `#[serde(skip)]`:
    /// serializing one is an error and no file can contain one.
    #[serde(skip)]
    RunSet(Box<GlyphRunData>),
    /// Move a run: set its `origin`. The bounds follow. O(1).
    RunOrigin(Vec2),
    /// Set a run's opacity, multiplied into every glyph's alpha. O(1).
    RunOpacity(f32),
    /// Edit a run's per-glyph overrides: what glyph-layer presentations
    /// write. See [`RunOverrideOp`]. Cost: O(glyphs in the range).
    RunOverrides(RunOverrideOp),
}

impl AsRef<GfxMutation> for GfxMutation {
    fn as_ref(&self) -> &GfxMutation {
        self
    }
}

impl GfxMutation {
    /// Wrap a [`DeltaGlyphArea`] into a boxed `GfxMutation::AreaDelta`.
    /// One heap allocation for the box.
    pub fn area_delta(area_delta: DeltaGlyphArea) -> Self {
        AreaDelta(Box::new(area_delta))
    }

    /// Wrap a [`GlyphAreaCommand`] into a boxed `GfxMutation::AreaCommand`.
    /// One heap allocation for the box.
    pub fn area_command(area_command: GlyphAreaCommand) -> Self {
        AreaCommand(Box::new(area_command))
    }

    /// Wrap a [`DeltaGlyphModel`] into a boxed `GfxMutation::ModelDelta`.
    /// One heap allocation for the box.
    pub fn model_delta(model_delta: DeltaGlyphModel) -> Self {
        ModelDelta(Box::new(model_delta))
    }

    /// Wrap a [`GlyphModelCommand`] into a boxed `GfxMutation::ModelCommand`.
    /// One heap allocation for the box.
    pub fn model_command(model_command: GlyphModelCommand) -> Self {
        ModelCommand(Box::new(model_command))
    }

    /// Whether these two mutations would write the same thing —
    /// the **reflexive** payload-agreement predicate, as opposed to
    /// `==`.
    ///
    /// Derived `PartialEq` over `f32` is not an equivalence relation:
    /// `NaN != NaN`, so a mutation carrying a `NaN` is not `==` to
    /// itself. That matters because
    /// `flat_mutations` (gone) collapsed an AST only when every payload
    /// beneath the root
    /// agrees with the one it returns, and
    /// `scope::self_and_descendants` (gone)
    /// built its AST by cloning **one** list into two `Macro` nodes
    /// — so that comparison is a payload against itself. Under `==`
    /// a `NaN` there declines the whole mutator while the identical
    /// payload under
    /// `scope::self_only` (gone)
    /// applies: two helpers differing only in scope disagreeing on
    /// whether the mutation runs at all.
    ///
    /// Two payloads agree when they are `==` **or** when they are
    /// structurally identical. The second disjunct is what restores
    /// reflexivity; the first is what keeps `0.0` agreeing with
    /// `-0.0`, which "would write the same thing" requires. The
    /// union is itself an equivalence relation: structural identity
    /// implies equal values except for `NaN` payload bits, so a
    /// chain through either disjunct stays consistent.
    ///
    /// Structural identity is taken over the derived `Debug`
    /// rendering, which is the NaN-normalizing view of these types
    /// that costs no hand-written traversal: `f32`'s `Debug` prints
    /// every `NaN` as `NaN` while distinguishing `inf`, `-0.0` and
    /// every finite value (its shortest round-tripping form is
    /// injective). None of the types reachable from a `GfxMutation`
    /// has a hand-written `Debug`, so the rendering is total — nothing
    /// is elided, and two mutations rendering alike really are alike.
    ///
    /// The serialized form is **not** usable for this: `serde_json`
    /// writes every non-finite float as `null`, so it would make a
    /// `NaN` payload agree with an `inf` one — and `inf` *is*
    /// reachable from a `.mindmap.json` (an out-of-range literal), so
    /// that would be a silently-dropped payload rather than merely an
    /// over-strict decline.
    ///
    /// Costs: O(1) on the common path — `==` answers, and only a
    /// genuine disagreement or a `NaN` reaches the fallback, which
    /// renders both operands into `String`s.
    pub fn writes_the_same(&self, other: &Self) -> bool {
        self == other || format!("{self:?}") == format!("{other:?}")
    }

    /// Whether this is one of the run payloads (`RunSet` …
    /// `RunOverrides`). O(1).
    fn is_run(&self) -> bool {
        matches!(self, RunSet(_) | RunOrigin(_) | RunOpacity(_) | RunOverrides(_))
    }

    /// Apply this mutation directly to a [`GlyphArea`]. A model
    /// variant is silently ignored (debug-logged). Cost: O(k) for
    /// deltas, O(1) for commands.
    ///
    /// `pub(crate)` because the only correct entry point is
    /// [`Mutation::apply_to`] (which routes `Event` to the element's
    /// subscribers before dispatching to area / model). Direct
    /// callers would bypass `Event` handling — the `unreachable!`
    /// in the `Event` arm below pins this invariant.
    pub(crate) fn apply_to_area(&self, area: &mut GlyphArea) {
        match self {
            AreaDelta(mutation) => mutation.apply_to(area),
            AreaCommand(mutation) => mutation.apply_to(area),
            ModelDelta(_) | ModelCommand(_) => {
                debug!("Tried to apply a model mutation to an area, ignoring.")
            }
            RunSet(_) | RunOrigin(_) | RunOpacity(_) | RunOverrides(_) => self.refuse_run("a GlyphArea"),
            // `apply_to` filters `Event` before reaching here.
            // A direct caller hitting this arm is a regression in
            // the dispatch invariant, not a runtime user error.
            Event(_) => {
                unreachable!("Event reached apply_to_area; apply_to() must filter Event before dispatch")
            }
        }
    }

    /// Apply this mutation directly to a [`GlyphModel`]. An area
    /// variant is silently ignored (debug-logged). Cost: O(k) for
    /// deltas, O(1) for commands.
    ///
    /// See [`Self::apply_to_area`] for the `pub(crate)` rationale.
    pub(crate) fn apply_to_model(&self, model: &mut GlyphModel) {
        match self {
            ModelDelta(mutation) => mutation.apply_to(model),
            ModelCommand(mutation) => mutation.apply_to(model),
            AreaDelta(_) | AreaCommand(_) => {
                debug!("Tried to apply an area mutation to a model, ignoring.");
            }
            RunSet(_) | RunOrigin(_) | RunOpacity(_) | RunOverrides(_) => self.refuse_run("a GlyphModel"),
            Event(_) => {
                unreachable!("Event reached apply_to_model; apply_to() must filter Event before dispatch")
            }
        }
    }

    /// Apply this mutation directly to a run's data. Only the `Run*`
    /// variants write one; an area or model payload is ignored as at a
    /// `Void`.
    ///
    /// Cost: O(1), except `RunSet` (a payload copy, written into the
    /// run's own override vectors by `clone_from`, so a payload with
    /// overrides no longer than the run's allocates nothing) and
    /// `RunOverrides` (O(glyphs in range)). See [`Self::apply_to_area`]
    /// for the `pub(crate)` rationale.
    pub(crate) fn apply_to_run(&self, run: &mut GlyphRunData) {
        match self {
            RunSet(data) => run.clone_from(data),
            RunOrigin(origin) => run.set_origin(*origin),
            RunOpacity(opacity) => run.set_opacity(*opacity),
            RunOverrides(op) => run.apply_override(op),
            // An area or model payload at a run: not the run's to
            // apply, as at a `Void`.
            AreaDelta(_) | AreaCommand(_) | ModelDelta(_) | ModelCommand(_) => {
                debug!("Tried to apply an area or model mutation to a run, ignoring.");
            }
            Event(_) => {
                unreachable!("Event reached apply_to_run; apply_to() must filter Event before dispatch")
            }
        }
    }

    /// A run payload reached an element of another kind (`target`):
    /// nothing changes, in every build profile.
    ///
    /// The three authored variants (`RunOrigin`, `RunOpacity`,
    /// `RunOverrides`) can come from authored input, which is untrusted, and a
    /// scope such as `SelfAndDescendants` sends one list to elements of
    /// every kind. They are a logged no-op, as an area or model payload
    /// at the wrong element is, and never assert.
    ///
    /// `RunSet` comes only from projection and is never authored, so
    /// landing on the wrong element is a projection bug: it also trips a
    /// `debug_assert!`.
    fn refuse_run(&self, target: &str) {
        debug!(
            "gfx_structs: a {:?} mutation reached {target}, ignoring.",
            self.variant()
        );
        debug_assert!(
            !matches!(self, RunSet(_)),
            "a projection-only {:?} mutation reached {target}",
            self.variant()
        );
    }
}

/// The glyph payload. Glyph elements inherit nothing (`Inherited =
/// ()`), so `inherited` is ignored; its template is a list of
/// [`CellField`] slots (see `gfx_structs::template`).
impl Mutation<GfxElement> for GfxMutation {
    type Template = Vec<CellField>;

    /// Apply this mutation to the given [`GfxElement`]. Events are
    /// dispatched to the element's subscribers; field mutations are
    /// routed to the matching element type (area, model or run).
    /// Applying to a `Void` element or a type mismatch is a no-op
    /// (logged at debug level), a run payload at an element of another
    /// kind included; only `RunSet`, which is never authored, also
    /// trips a `debug_assert!` there. Cost: O(k)
    /// in the number of delta fields, or O(1) for commands and events.
    fn apply_to(&self, target: &mut GfxElement, _inherited: &()) {
        if let Event(event) = self {
            target.accept_event(event);
            return;
        }
        match target {
            GfxElement::GlyphArea { glyph_area, .. } => {
                self.apply_to_area(glyph_area);
            }
            GfxElement::GlyphModel { glyph_model, .. } => {
                self.apply_to_model(glyph_model);
            }
            GfxElement::GlyphRun { run, .. } => {
                self.apply_to_run(run);
            }
            GfxElement::Void { .. } => {
                if self.is_run() {
                    self.refuse_run("a Void");
                }
            }
        }
    }
}
