// SPDX-License-Identifier: MPL-2.0

//! What the **derive macro** says the on-disk shape is, asked of the
//! generated code rather than of the source text that produced it.
//!
//! [`crate::util::serde_coverage`] answers the same questions by
//! parsing baumhard's own `.rs` files with `syn`. That walk is the
//! only thing `format/schema.md`'s published array list rests on, and
//! a walk is a *reading* of the source — it can be wrong in ways the
//! document it feeds is then wrong in too. The two agree because they
//! share a derivation, not because the derivation is right, and
//! [issue #122](https://github.com/Svendsys/mandala/issues/122) is the
//! record of that: a `Vec` behind a type alias, a `VecDeque`, a
//! `#[serde(rename(deserialize = "…"))]`, a container `rename_all` —
//! each one produces a wrong published name or a missing array, and
//! nothing disagrees.
//!
//! So something has to disagree, and it has to get its answer from
//! somewhere else. `serde_derive` already wrote the truth down: the
//! `Deserialize` impl it generates for a struct calls
//! `Deserializer::deserialize_struct(name, FIELDS, visitor)` where
//! `FIELDS` is the exact list of JSON member names — every `rename`
//! spelling resolved, `rename_all` applied, the identifier used only
//! where nothing overrode it. An enum's impl names its variants the
//! same way. A `Vec` field asks for [`Deserializer::deserialize_seq`]
//! whatever it is spelled or aliased as, and a tuple asks for
//! [`Deserializer::deserialize_tuple`], which is the distinction
//! between "an index an edit can move" and "a position the format
//! fixes".
//!
//! [`derived_shape`] hands the derived impls a `Deserializer` that
//! answers every request with the emptiest value that satisfies it and
//! writes down what was asked. Nothing is parsed, nothing is listed,
//! and no `.mindmap.json` is involved: the walk is the compiler's own
//! expansion of the model, driven from `MindMap::deserialize` outward.
//!
//! **It is not a replacement for the source walk and must not become
//! one.** It cannot see a type no value reaches (a serialize-only
//! proxy), it cannot see `#[serde(alias = "…")]` (serde matches
//! aliases without ever naming them), and it says nothing about
//! `deny_unknown_fields` or `flatten`. What it is good for is exactly
//! one thing: being a **second, independent** answer to "which JSON
//! members hold growable arrays of key-bearing values", so
//! `mindmap::unknown_keys::tests::test_the_derived_positional_arrays_survive_an_independent_derivation`
//! has something that can disagree.
//!
//! Test-only: nothing in a shipped build deserializes a model type
//! from a probe. Unlike its counterpart it reads no files and pulls in
//! no `syn`, so nothing about it is native-only — and it carries the
//! same `not(target_arch = "wasm32")` gate regardless, because every
//! consumer of it does. A module compiled into a build where nothing
//! calls it is a pile of dead-code warnings claiming a portability
//! that no gate exercises.

use serde::de::{
    DeserializeOwned, DeserializeSeed, EnumAccess, IntoDeserializer, MapAccess, SeqAccess, VariantAccess,
    Visitor,
};
use serde::Deserializer;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

/// The probe's error type.
///
/// `serde_json::Error` rather than an enum of our own:
/// [`CODE_CONVENTIONS.md §9`](../../../../CODE_CONVENTIONS.md) forbids
/// custom error types, `serde::de::Error` demands *some* type with a
/// `custom` constructor, and the one this workspace already
/// deserializes every map through satisfies it. The probe only ever
/// raises one error — a walk that would not terminate — so the type's
/// JSON-shaped extras cost nothing.
type Error = serde_json::Error;

/// How deep the probe will follow a chain of required values before
/// it decides the walk does not terminate.
///
/// A recursion reaches bottom one of two ways, and the probe has to
/// do both. Through an `Option`, a sequence or a map there is an
/// emptier value to answer with, and [`Mode::Terminate`] answers it.
/// Through a **required enum payload** there is not — `Repeat {
/// template: Box<MutatorNode> }` has no empty form — so terminating
/// means picking a different variant, which is what
/// [`terminating_variant`] does. What is left over is a type with no
/// bottoming variant anywhere on the cycle; that one really cannot be
/// deserialized, and the depth is capped so the run stops by name
/// rather than spinning.
const MAX_DEPTH: usize = 64;

/// How many passes the variant sweep will make before it gives up.
///
/// One pass per enum variant that needs reaching is the shape of the
/// bound; the cap is far above the model's variant count so that
/// crossing it means the choice tree grew a dimension nobody
/// intended, not that a variant was added.
const MAX_RUNS: usize = 4096;

/// Whether the walk is still discovering shape or only trying to
/// bottom out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mode {
    /// Sequences yield one element, `Option`s yield `Some`, maps
    /// yield one entry — every container is opened so what is inside
    /// it is recorded.
    Explore,
    /// Sequences, `Option`s and maps come back empty. Entered when a
    /// container reappears on its own path, which is the only way a
    /// deserializable type recurses.
    Terminate,
}

/// One growable array the derived impls asked for.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct DerivedSequence {
    /// The JSON member the array sits under — a struct field's
    /// serde name, or the variant name of an externally tagged
    /// newtype variant, which is the member an array payload is
    /// written under.
    pub member: String,
    /// `true` when an unrecognized key can be captured at or below an
    /// element: somewhere inside one, serde asks for a struct or a
    /// struct variant, which is the shape whose unclaimed members
    /// reach `deserialize_ignored_any`.
    pub key_bearing: bool,
    /// Where the probe met it, as `Container.member`, for a failure
    /// message a reader can act on.
    pub site: String,
}

/// Everything one [`derived_shape`] sweep recorded.
#[derive(Debug, Clone, Default)]
pub struct DerivedShape {
    members: BTreeMap<String, BTreeSet<String>>,
    variants: BTreeMap<String, Vec<String>>,
    sequences: BTreeMap<String, DerivedSequence>,
    unnamed_sequences: BTreeSet<String>,
}

impl DerivedShape {
    /// Every JSON member name that holds a growable array whose
    /// elements can carry named keys — the compiler's answer to the
    /// question `format/schema.md` publishes.
    ///
    /// Cost: one pass over the recorded sequences; allocates the set.
    pub fn key_bearing_sequences(&self) -> BTreeSet<String> {
        self.sequences
            .values()
            .filter(|seq| seq.key_bearing)
            .map(|seq| seq.member.clone())
            .collect()
    }

    /// Every growable array the sweep met, key-bearing or not, keyed
    /// by member name — so a caller can say *why* a member is absent
    /// from [`Self::key_bearing_sequences`] rather than only that it
    /// is.
    pub fn sequences(&self) -> &BTreeMap<String, DerivedSequence> {
        &self.sequences
    }

    /// The JSON member names the derived `Deserialize` impl for
    /// `container` accepts, or `None` when no value of that type was
    /// reached. Struct variants are keyed `"Enum::Variant"`.
    pub fn members_of(&self, container: &str) -> Option<&BTreeSet<String>> {
        self.members.get(container)
    }

    /// Every container name the sweep reached, in sorted order.
    pub fn containers(&self) -> impl Iterator<Item = &str> {
        self.members.keys().map(String::as_str)
    }

    /// The variant names the derived impl for `container` accepts.
    pub fn variants_of(&self, container: &str) -> Option<&[String]> {
        self.variants.get(container).map(Vec::as_slice)
    }

    /// Growable arrays the sweep met at a place with **no** JSON
    /// member name to publish them under — inside a tuple, or as the
    /// value of a map whose keys are data.
    ///
    /// Empty for the model today. It is reported rather than dropped
    /// for the same reason [`crate::util::serde_coverage`] reports a
    /// name it cannot resolve: an array with no publishable name is a
    /// positional route the document cannot warn anybody about, and
    /// silence about it reads as absence.
    pub fn unnamed_sequences(&self) -> &BTreeSet<String> {
        &self.unnamed_sequences
    }
}

/// One enum the walk had to pick a variant at.
#[derive(Debug, Clone)]
struct Decision {
    /// The enum's serde container name.
    container: String,
    /// How many variants it declared.
    options: usize,
    /// Which one this pass took.
    chosen: usize,
}

/// Whether a variant carries anything the walk has to go into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum VariantKind {
    /// A unit variant. Nothing below it, so it always bottoms out.
    Unit,
    /// A newtype, tuple or struct variant. Whether it bottoms out
    /// depends on what is inside it.
    Payload,
}

/// What one enum variant has turned out to be, accumulated over the
/// whole sweep rather than one pass.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct VariantFacts {
    /// `true` once an [`Mode::Explore`] pass has taken this variant —
    /// the coverage question the sweep's queue drains against. A
    /// [`Mode::Terminate`] visit deliberately does not set it: it
    /// walks an emptied subtree, so counting it as reached would end
    /// the sweep with shape still unseen.
    reached: bool,
    /// `Some(_)` once any pass has entered the variant and learned
    /// whether it carries a payload.
    kind: Option<VariantKind>,
    /// `true` once a [`Mode::Terminate`] visit of this variant
    /// returned — proof that taking it reaches bottom.
    bottoms_out: bool,
}

/// What the sweep remembers between passes, keyed by enum name and
/// indexed by variant.
///
/// Two questions live here rather than in a [`Run`] because both
/// outlive a single pass: which variants still need a pass of their
/// own, and which variant to take when the walk is only trying to
/// stop. The second is why the memory has to accumulate at all — the
/// pass that discovers `Repeat` carries a payload is the same pass
/// that then has to get out of `Repeat`.
#[derive(Debug, Default)]
struct Memory {
    facts: BTreeMap<String, Vec<VariantFacts>>,
}

impl Memory {
    /// The facts for `container`, sized to `options` on first
    /// sighting.
    fn facts_mut(&mut self, container: &str, options: usize) -> &mut Vec<VariantFacts> {
        self.facts
            .entry(container.to_string())
            .or_insert_with(|| vec![VariantFacts::default(); options])
    }

    /// Note what `container`'s variant number `at` turned out to be.
    /// Silently ignores an index the enum does not declare, which
    /// cannot happen — the index comes from the same `variants` slice.
    fn note(&mut self, container: &str, at: usize, kind: VariantKind) {
        if let Some(facts) = self.facts.get_mut(container).and_then(|all| all.get_mut(at)) {
            facts.kind = Some(kind);
        }
    }

    /// Note that a [`Mode::Terminate`] visit of `container`'s variant
    /// number `at` reached bottom.
    fn note_bottomed(&mut self, container: &str, at: usize) {
        if let Some(facts) = self.facts.get_mut(container).and_then(|all| all.get_mut(at)) {
            facts.bottoms_out = true;
        }
    }

    /// Whether an [`Mode::Explore`] pass has taken this variant.
    fn reached(&self, container: &str, at: usize) -> bool {
        self.facts
            .get(container)
            .and_then(|all| all.get(at))
            .is_some_and(|facts| facts.reached)
    }
}

/// The variant to take when the walk is no longer discovering shape
/// and only needs to stop, in order of how sure the choice is.
///
/// A unit variant is the emptiest value an enum admits and always
/// bottoms out; one an earlier terminating visit already walked to
/// the end is proven; one nothing has classified yet may be either
/// and is worth trying, since every attempt classifies it and so
/// removes it from the next level's candidates. Variant 0 is the last
/// resort, and reaching it means every variant is known to carry a
/// payload and none has been seen to bottom out — a cycle the depth
/// cap is right to stop.
fn terminating_variant(memory: &Memory, container: &str, options: usize) -> usize {
    let Some(facts) = memory.facts.get(container) else {
        return 0;
    };
    let pick = |wanted: fn(&VariantFacts) -> bool| {
        facts
            .iter()
            .take(options)
            .position(wanted)
            .filter(|at| *at < options)
    };
    pick(|facts| facts.kind == Some(VariantKind::Unit))
        .or_else(|| pick(|facts| facts.bottoms_out))
        .or_else(|| pick(|facts| facts.kind.is_none()))
        .unwrap_or(0)
}

/// State threaded through one pass of the walk.
struct Run<'a> {
    /// Variant index to take at each decision, by decision order. A
    /// decision past the end of the plan takes variant 0.
    plan: Vec<usize>,
    /// The decisions this pass actually met, in order.
    trace: Vec<Decision>,
    /// What the sweep has learned about enum variants so far.
    memory: &'a mut Memory,
    /// Container names currently open on the walk's path — the
    /// recursion guard.
    active: Vec<String>,
    /// How many times a struct or a struct variant has been asked
    /// for. Compared before and after an element to decide whether
    /// the element can carry named keys.
    named_key_sites: usize,
    /// What the sweep has recorded.
    shape: DerivedShape,
}

/// Ask the derived `Deserialize` impls reachable from `T` what shape
/// they expect, and record it.
///
/// The sweep runs the walk repeatedly. Each pass takes one variant at
/// every enum it meets; afterwards every variant it did *not* take,
/// and that no earlier pass took, is queued as a plan of its own —
/// the same choices up to that enum, then the untaken variant. The
/// queue drains when every reachable variant has been inside at least
/// one pass, so a `Vec` that only exists under `Mutation`'s
/// fourteenth variant is still found.
///
/// Panics when a pass fails, when the walk cannot bottom out
/// ([`MAX_DEPTH`]), or when the sweep exceeds [`MAX_RUNS`]. All three
/// are the probe declining to answer rather than answering short: an
/// under-explored shape is exactly the silent wrongness this module
/// exists to expose.
///
/// Cost: one pass per reachable enum variant, each a full walk of the
/// type graph. No I/O and no parsing.
pub fn derived_shape<T: DeserializeOwned>() -> DerivedShape {
    let mut memory = Memory::default();
    let mut shape = DerivedShape::default();
    let mut targeted: BTreeSet<(String, usize)> = BTreeSet::new();
    let mut queue: VecDeque<Vec<usize>> = VecDeque::new();
    queue.push_back(Vec::new());

    let mut passes = 0usize;
    while let Some(plan) = queue.pop_front() {
        passes += 1;
        assert!(
            passes <= MAX_RUNS,
            "the variant sweep passed {MAX_RUNS} runs without draining its queue. Each \
             queued plan targets one enum variant nothing had reached yet, so crossing \
             this means the choice tree grew a dimension — raise the cap deliberately \
             rather than by reflex."
        );
        let mut run = Run {
            plan,
            trace: Vec::new(),
            memory: &mut memory,
            active: Vec::new(),
            named_key_sites: 0,
            shape,
        };
        let probe = Probe {
            run: &mut run,
            member: None,
            mode: Mode::Explore,
            depth: 0,
        };
        if let Err(e) = T::deserialize(probe) {
            panic!(
                "the probe could not deserialize {} from its own answers: {e}. It serves \
                 the emptiest value every request admits, so a failure here is a shape it \
                 does not know how to satisfy — teach it that shape rather than narrowing \
                 what it walks.",
                std::any::type_name::<T>()
            );
        }
        let trace = run.trace;
        shape = run.shape;
        for (at, decision) in trace.iter().enumerate() {
            for option in 0..decision.options {
                let pair = (decision.container.clone(), option);
                // One plan per unreached variant, not one per place
                // it could be reached from. The walk is deterministic
                // for a given plan, so replaying this trace's prefix
                // arrives back at this same decision and takes the
                // untaken option — a second plan for the same variant
                // would only find it again, and enqueueing every
                // (site, option) pair is what turns a sweep bounded
                // by the variant count into one bounded by the size
                // of the choice tree.
                if option == decision.chosen
                    || memory.reached(&decision.container, option)
                    || !targeted.insert(pair)
                {
                    continue;
                }
                let mut next: Vec<usize> = trace[..at].iter().map(|d| d.chosen).collect();
                next.push(option);
                queue.push_back(next);
            }
        }
    }
    shape
}

/// The `Deserializer` the derived impls are handed. Every method
/// answers with the emptiest value that satisfies the request and
/// writes down what was asked for.
struct Probe<'a, 'r> {
    run: &'r mut Run<'a>,
    /// The JSON member this value sits under, when it has one. `None`
    /// inside a sequence element, a tuple position, or a map value —
    /// places whose name is data rather than schema.
    member: Option<String>,
    mode: Mode,
    depth: usize,
}

/// One step further down, or the error that says the walk does not
/// bottom out.
fn deeper(depth: usize, what: &str) -> Result<usize, Error> {
    if depth >= MAX_DEPTH {
        return Err(serde::de::Error::custom(format!(
            "the probe reached depth {MAX_DEPTH} at `{what}` without bottoming out. Once a \
             container reappears on its own path the probe empties every `Option`, \
             sequence and map below it, and takes the emptiest variant it can find at \
             every enum — so a cycle that still spins has no bottoming variant on it \
             anywhere, and no document can hold a value of it."
        )));
    }
    Ok(depth + 1)
}

/// `Container.member`, or the container alone where the value has no
/// member name, for a message that names a place in the model.
fn site_of(run: &Run<'_>, member: Option<&str>) -> String {
    let container = run.active.last().map_or("<root>", String::as_str);
    match member {
        Some(name) => format!("{container}.{name}"),
        None => format!("{container} (no member name)"),
    }
}

/// Record a growable array under the member that holds it, merging
/// with any earlier sighting: the same member name met twice is
/// key-bearing if it was key-bearing anywhere.
fn record_sequence(run: &mut Run<'_>, member: Option<String>, site: String, key_bearing: bool) {
    let Some(name) = member else {
        run.shape.unnamed_sequences.insert(site);
        return;
    };
    let entry = run
        .shape
        .sequences
        .entry(name.clone())
        .or_insert_with(|| DerivedSequence {
            member: name,
            key_bearing: false,
            site,
        });
    entry.key_bearing |= key_bearing;
}

/// The scalar answers, each the emptiest value its visitor accepts.
macro_rules! probe_scalar {
    ($($method:ident => $visit:ident($($value:expr),*),)*) => {
        $(
            fn $method<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
                visitor.$visit($($value),*)
            }
        )*
    };
}

impl<'de, 'a, 'r> Deserializer<'de> for Probe<'a, 'r> {
    type Error = Error;

    probe_scalar! {
        deserialize_any => visit_unit(),
        deserialize_ignored_any => visit_unit(),
        deserialize_unit => visit_unit(),
        deserialize_bool => visit_bool(false),
        deserialize_i8 => visit_i8(0),
        deserialize_i16 => visit_i16(0),
        deserialize_i32 => visit_i32(0),
        deserialize_i64 => visit_i64(0),
        deserialize_i128 => visit_i128(0),
        deserialize_u8 => visit_u8(0),
        deserialize_u16 => visit_u16(0),
        deserialize_u32 => visit_u32(0),
        deserialize_u64 => visit_u64(0),
        deserialize_u128 => visit_u128(0),
        deserialize_f32 => visit_f32(0.0),
        deserialize_f64 => visit_f64(0.0),
        deserialize_char => visit_char('a'),
        deserialize_str => visit_str(""),
        deserialize_string => visit_str(""),
        deserialize_bytes => visit_bytes(&[]),
        deserialize_byte_buf => visit_bytes(&[]),
        deserialize_identifier => visit_str(""),
    }

    fn deserialize_unit_struct<V: Visitor<'de>>(
        self,
        _name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        visitor.visit_unit()
    }

    fn deserialize_newtype_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        visitor: V,
    ) -> Result<V::Value, Error> {
        let depth = deeper(self.depth, name)?;
        visitor.visit_newtype_struct(Probe {
            run: self.run,
            member: self.member,
            mode: self.mode,
            depth,
        })
    }

    fn deserialize_option<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        if self.mode == Mode::Terminate {
            return visitor.visit_none();
        }
        let depth = deeper(self.depth, "Option")?;
        visitor.visit_some(Probe {
            run: self.run,
            member: self.member,
            mode: self.mode,
            depth,
        })
    }

    fn deserialize_seq<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        let Probe {
            run,
            member,
            mode,
            depth,
        } = self;
        let site = site_of(run, member.as_deref());
        if mode == Mode::Terminate {
            let out = visitor.visit_seq(Elements {
                run: &mut *run,
                remaining: 0,
                mode,
                depth,
            })?;
            record_sequence(run, member, site, false);
            return Ok(out);
        }
        let depth = deeper(depth, &site)?;
        // The element is probed inside `visit_seq`, so the count of
        // named-key sites before and after it is exactly "did serde
        // ask for a struct anywhere at or below one element". That is
        // the question the published list turns on, and asking it
        // this way costs no second traversal.
        let before = run.named_key_sites;
        let out = visitor.visit_seq(Elements {
            run: &mut *run,
            remaining: 1,
            mode,
            depth,
        })?;
        let key_bearing = run.named_key_sites > before;
        record_sequence(run, member, site, key_bearing);
        Ok(out)
    }

    fn deserialize_tuple<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Error> {
        // A tuple is an array whose arity the format fixes, so an
        // edit cannot slide a route onto a different position. It is
        // walked for what is inside it and never recorded as a
        // growable array.
        let depth = deeper(self.depth, "tuple")?;
        visitor.visit_seq(Elements {
            run: self.run,
            remaining: len,
            mode: self.mode,
            depth,
        })
    }

    fn deserialize_tuple_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        len: usize,
        visitor: V,
    ) -> Result<V::Value, Error> {
        let depth = deeper(self.depth, name)?;
        visitor.visit_seq(Elements {
            run: self.run,
            remaining: len,
            mode: self.mode,
            depth,
        })
    }

    fn deserialize_map<V: Visitor<'de>>(self, visitor: V) -> Result<V::Value, Error> {
        // Not a named-key site: a map's members are data — a palette
        // name, a node id — so none of them is ever "unrecognized".
        let entries = usize::from(self.mode == Mode::Explore);
        let depth = deeper(self.depth, "map")?;
        visitor.visit_map(Entries {
            run: self.run,
            remaining: entries,
            mode: self.mode,
            depth,
        })
    }

    fn deserialize_struct<V: Visitor<'de>>(
        self,
        name: &'static str,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        let Probe { run, mode, depth, .. } = self;
        let depth = deeper(depth, name)?;
        record_container(run, name, fields);
        // A struct hands every member it does not claim to
        // `deserialize_ignored_any`, which is what the loader's
        // capture wraps — so this is the shape that makes an element
        // able to carry a key.
        run.named_key_sites += 1;
        let mode = descend_mode(run, name, mode);
        run.active.push(name.to_string());
        let out = visitor.visit_map(Fields {
            run: &mut *run,
            fields,
            at: 0,
            mode,
            depth,
        });
        run.active.pop();
        out
    }

    fn deserialize_enum<V: Visitor<'de>>(
        self,
        name: &'static str,
        variants: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        let Probe { run, mode, depth, .. } = self;
        let depth = deeper(depth, name)?;
        run.shape
            .variants
            .entry(name.to_string())
            .or_insert_with(|| variants.iter().map(|v| (*v).to_string()).collect());
        let mode = descend_mode(run, name, mode);
        run.memory.facts_mut(name, variants.len());
        // A terminate-mode pass records no decision, and that is the
        // load-bearing half: the sweep's plans are indexed by decision
        // order, so a choice that varied with how deep the recursion
        // happened to be would move every later index out from under
        // the plan that produced it. Which variant it takes is free
        // to depend on what the sweep has learned, because nothing
        // downstream is indexed by it.
        let chosen = if mode == Mode::Terminate {
            terminating_variant(run.memory, name, variants.len())
        } else {
            let want = run.plan.get(run.trace.len()).copied().unwrap_or(0);
            let chosen = want.min(variants.len().saturating_sub(1));
            run.trace.push(Decision {
                container: name.to_string(),
                options: variants.len(),
                chosen,
            });
            if let Some(facts) = run.memory.facts_mut(name, variants.len()).get_mut(chosen) {
                facts.reached = true;
            }
            chosen
        };
        let Some(variant) = variants.get(chosen) else {
            return Err(serde::de::Error::custom(format!(
                "`{name}` declares no variants, so the probe has nothing to hand its \
                 deserializer"
            )));
        };
        run.active.push(name.to_string());
        let out = visitor.visit_enum(Variant {
            run: &mut *run,
            container: name,
            variant,
            at: chosen,
            mode,
            depth,
        });
        run.active.pop();
        if mode == Mode::Terminate && out.is_ok() {
            run.memory.note_bottomed(name, chosen);
        }
        out
    }

    fn is_human_readable(&self) -> bool {
        true
    }
}

/// Note the JSON member names a container accepts, merging with any
/// earlier sighting of the same container.
fn record_container(run: &mut Run<'_>, name: &str, fields: &[&str]) {
    run.shape
        .members
        .entry(name.to_string())
        .or_default()
        .extend(fields.iter().map(|field| (*field).to_string()));
}

/// [`Mode::Terminate`] once `name` is already open on the walk's own
/// path — the only way a deserializable type recurses — and never
/// back to [`Mode::Explore`] once it has been left.
fn descend_mode(run: &Run<'_>, name: &str, mode: Mode) -> Mode {
    if mode == Mode::Terminate || run.active.iter().any(|open| open == name) {
        Mode::Terminate
    } else {
        Mode::Explore
    }
}

/// `remaining` sequence or tuple positions, each probed with no
/// member name of its own.
struct Elements<'a, 'r> {
    run: &'r mut Run<'a>,
    remaining: usize,
    mode: Mode,
    depth: usize,
}

impl<'de, 'a, 'r> SeqAccess<'de> for Elements<'a, 'r> {
    type Error = Error;

    fn next_element_seed<T: DeserializeSeed<'de>>(&mut self, seed: T) -> Result<Option<T::Value>, Error> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        seed.deserialize(Probe {
            run: &mut *self.run,
            member: None,
            mode: self.mode,
            depth: self.depth,
        })
        .map(Some)
    }

    fn size_hint(&self) -> Option<usize> {
        Some(self.remaining)
    }
}

/// `remaining` entries of a map whose keys are data.
struct Entries<'a, 'r> {
    run: &'r mut Run<'a>,
    remaining: usize,
    mode: Mode,
    depth: usize,
}

impl<'de, 'a, 'r> MapAccess<'de> for Entries<'a, 'r> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Error> {
        if self.remaining == 0 {
            return Ok(None);
        }
        self.remaining -= 1;
        seed.deserialize(Probe {
            run: &mut *self.run,
            member: None,
            mode: self.mode,
            depth: self.depth,
        })
        .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        seed.deserialize(Probe {
            run: &mut *self.run,
            member: None,
            mode: self.mode,
            depth: self.depth,
        })
    }
}

/// A struct's or struct variant's members, offered in the order the
/// derived impl declared them.
struct Fields<'a, 'r> {
    run: &'r mut Run<'a>,
    fields: &'static [&'static str],
    at: usize,
    mode: Mode,
    depth: usize,
}

impl<'de, 'a, 'r> MapAccess<'de> for Fields<'a, 'r> {
    type Error = Error;

    fn next_key_seed<K: DeserializeSeed<'de>>(&mut self, seed: K) -> Result<Option<K::Value>, Error> {
        let Some(name) = self.fields.get(self.at) else {
            return Ok(None);
        };
        seed.deserialize(IntoDeserializer::<'de, Error>::into_deserializer(*name))
            .map(Some)
    }

    fn next_value_seed<V: DeserializeSeed<'de>>(&mut self, seed: V) -> Result<V::Value, Error> {
        let member = self.fields.get(self.at).map(|name| (*name).to_string());
        self.at += 1;
        seed.deserialize(Probe {
            run: &mut *self.run,
            member,
            mode: self.mode,
            depth: self.depth,
        })
    }
}

/// The one variant this pass takes, and whatever payload it carries.
struct Variant<'a, 'r> {
    run: &'r mut Run<'a>,
    container: &'static str,
    variant: &'static str,
    /// Its index in the enum's `VARIANTS` list, so what it turns out
    /// to be can be written back to [`Memory`].
    at: usize,
    mode: Mode,
    depth: usize,
}

impl<'de, 'a, 'r> EnumAccess<'de> for Variant<'a, 'r> {
    type Error = Error;
    type Variant = Self;

    fn variant_seed<V: DeserializeSeed<'de>>(self, seed: V) -> Result<(V::Value, Self), Error> {
        let name = self.variant;
        let chosen = seed.deserialize(IntoDeserializer::<'de, Error>::into_deserializer(name))?;
        Ok((chosen, self))
    }
}

impl<'de, 'a, 'r> VariantAccess<'de> for Variant<'a, 'r> {
    type Error = Error;

    fn unit_variant(self) -> Result<(), Error> {
        self.run.memory.note(self.container, self.at, VariantKind::Unit);
        Ok(())
    }

    fn newtype_variant_seed<T: DeserializeSeed<'de>>(self, seed: T) -> Result<T::Value, Error> {
        // Noted before the payload is walked, not after: the pass
        // that finds `Repeat { template: Box<MutatorNode> }` is the
        // same pass that then has to terminate inside it, and it can
        // only rule the variant out if the fact is already recorded.
        self.run.memory.note(self.container, self.at, VariantKind::Payload);
        // An externally tagged newtype variant writes its payload as
        // the single member of a wrapper object, so the variant name
        // *is* the JSON member a `Vec` payload sits under — and the
        // member a positional route through it would be published as.
        seed.deserialize(Probe {
            run: self.run,
            member: Some(self.variant.to_string()),
            mode: self.mode,
            depth: self.depth,
        })
    }

    fn tuple_variant<V: Visitor<'de>>(self, len: usize, visitor: V) -> Result<V::Value, Error> {
        self.run.memory.note(self.container, self.at, VariantKind::Payload);
        visitor.visit_seq(Elements {
            run: self.run,
            remaining: len,
            mode: self.mode,
            depth: self.depth,
        })
    }

    fn struct_variant<V: Visitor<'de>>(
        self,
        fields: &'static [&'static str],
        visitor: V,
    ) -> Result<V::Value, Error> {
        self.run.memory.note(self.container, self.at, VariantKind::Payload);
        let key = format!("{}::{}", self.container, self.variant);
        record_container(self.run, &key, fields);
        self.run.named_key_sites += 1;
        visitor.visit_map(Fields {
            run: self.run,
            fields,
            at: 0,
            mode: self.mode,
            depth: self.depth,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use std::collections::BTreeSet as StdBTreeSet;

    /// An object with a member no field claims — the shape whose
    /// presence below an array is what makes the array's indexes
    /// worth publishing.
    #[derive(Deserialize)]
    struct Leaf {
        #[allow(dead_code)]
        x: f64,
    }

    /// The same, orderable, so it can sit in a set.
    #[derive(Deserialize, PartialEq, Eq, PartialOrd, Ord)]
    struct OrderedLeaf {
        #[allow(dead_code)]
        x: i64,
    }

    /// `Vec<Leaf>` wearing a name that says nothing about what it is.
    type Leaves = Vec<Leaf>;

    /// The names the *file* uses, none of which is the identifier.
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Renaming {
        #[allow(dead_code)]
        control_points: Vec<Leaf>,
        #[serde(rename(serialize = "pts", deserialize = "points"))]
        #[allow(dead_code)]
        spare: Vec<Leaf>,
        #[allow(dead_code)]
        plain_scalar: f64,
    }

    /// Three arrays, one of which is not spelled `Vec` and one of
    /// which is not spelled at all, plus one whose elements no key
    /// can hide in.
    #[derive(Deserialize)]
    struct Sequences {
        #[allow(dead_code)]
        aliased: Leaves,
        #[allow(dead_code)]
        ordered: StdBTreeSet<OrderedLeaf>,
        #[allow(dead_code)]
        opaque: Vec<f64>,
    }

    /// A variant whose payload is an array, a variant whose payload
    /// is a fixed-arity tuple, and a fixed-arity array field.
    #[derive(Deserialize)]
    #[allow(dead_code)]
    enum Payloads {
        Literal(Vec<Leaf>),
        Pair(f64, Leaf),
    }

    #[derive(Deserialize)]
    struct CarriesPayload {
        #[allow(dead_code)]
        payload: Payloads,
        #[allow(dead_code)]
        fixed: [Leaf; 2],
    }

    /// An array reachable only through the *second* variant of an
    /// enum, which one pass of the walk cannot see.
    #[derive(Deserialize)]
    enum Branching {
        #[allow(dead_code)]
        First(Shallow),
        #[allow(dead_code)]
        Second(Deep),
    }

    #[derive(Deserialize)]
    struct Shallow {
        #[allow(dead_code)]
        a: f64,
    }

    #[derive(Deserialize)]
    struct Deep {
        #[allow(dead_code)]
        buried: Vec<Leaf>,
    }

    /// A type that contains itself, which every real model does
    /// somewhere.
    #[derive(Deserialize)]
    struct Recursive {
        #[allow(dead_code)]
        children: Vec<Recursive>,
        #[allow(dead_code)]
        name: String,
    }

    /// An array with nowhere to be published: position 0 of a tuple
    /// variant, which no member names.
    #[derive(Deserialize)]
    enum Tupled {
        #[allow(dead_code)]
        Both(Vec<Leaf>, f64),
    }

    /// Recursion that passes through **neither** an `Option`, a
    /// sequence nor a map: a boxed newtype variant, which
    /// [`Mode::Terminate`] has no emptier value to answer with. The
    /// only way out is to take a different variant, and `Stop` is
    /// deliberately declared *second* — the shape only bottoms out by
    /// accident if the walk always takes variant 0.
    #[derive(Deserialize)]
    enum Spinner {
        #[allow(dead_code)]
        Deeper(Box<Holder>),
        Stop,
    }

    #[derive(Deserialize)]
    struct Holder {
        #[allow(dead_code)]
        s: Spinner,
        #[allow(dead_code)]
        leaves: Vec<Leaf>,
    }

    /// **The member names come from the generated code, so every
    /// spelling of `rename` is resolved before the probe ever sees
    /// one.**
    ///
    /// All three names asserted here are ones the Rust identifier
    /// does not supply: `rename_all = "camelCase"` produces
    /// `controlPoints` and `plainScalar`, and the `deserialize` arm of
    /// a list-form `rename` produces `points` while the `serialize`
    /// arm is correctly ignored — a captured route is recorded
    /// against the shape that was read.
    #[test]
    fn test_the_probe_reads_the_member_names_the_derive_generated() {
        let shape = derived_shape::<Renaming>();
        let members = shape
            .members_of("Renaming")
            .expect("the probe must have reached the root type");
        let expected: BTreeSet<String> = ["controlPoints", "plainScalar", "points"]
            .iter()
            .map(|name| (*name).to_string())
            .collect();
        assert_eq!(
            members, &expected,
            "the derived impl's FIELDS list is the on-disk truth, identifiers and \
             `serialize` renames included in neither"
        );
        assert_eq!(
            shape.key_bearing_sequences(),
            ["controlPoints", "points"]
                .iter()
                .map(|name| (*name).to_string())
                .collect::<BTreeSet<String>>(),
            "and the arrays are published under those same names"
        );
    }

    /// **An array is what asks for a sequence, not what is spelled
    /// `Vec`.** A type alias hides the spelling entirely and a
    /// `BTreeSet` never had it; both are JSON arrays whose element
    /// order a save can change. `Vec<f64>` is the control — an array
    /// whose elements have no members for a key to hide in, so no
    /// captured route ever crosses its indexes.
    #[test]
    fn test_the_probe_sees_an_aliased_and_a_non_vec_sequence() {
        let shape = derived_shape::<Sequences>();
        let arrays = shape.key_bearing_sequences();
        assert!(
            arrays.contains("aliased"),
            "a `Vec` behind a type alias is still a `Vec`: {arrays:?}"
        );
        assert!(
            arrays.contains("ordered"),
            "a `BTreeSet` is an array on disk and a save re-sorts it, which is if \
             anything a stronger reason to publish it: {arrays:?}"
        );
        assert!(
            !arrays.contains("opaque"),
            "`Vec<f64>` holds nothing a key can be unrecognized inside: {arrays:?}"
        );
        assert!(
            shape.sequences().contains_key("opaque"),
            "though it is still an array, and the walk has to know that to say why it \
             is absent: {:?}",
            shape.sequences()
        );
    }

    /// **An externally tagged newtype variant publishes its payload
    /// under the variant's name**, which is the member a route
    /// crosses — `{"Literal": [ … ]}`. A tuple variant and a
    /// fixed-arity array are the controls: both are arrays on disk,
    /// neither has a length an edit can change, so neither is a place
    /// a route can be *moved*.
    #[test]
    fn test_the_probe_publishes_a_newtype_variant_payload_under_the_variant_name() {
        let shape = derived_shape::<CarriesPayload>();
        let arrays = shape.key_bearing_sequences();
        assert!(
            arrays.contains("Literal"),
            "the variant name is the JSON member its array payload sits under: {arrays:?}"
        );
        assert!(
            !arrays.contains("Pair"),
            "a tuple variant's arity is fixed by the format: {arrays:?}"
        );
        assert!(!arrays.contains("fixed"), "and so is `[Leaf; 2]`'s: {arrays:?}");
        assert!(
            shape.unnamed_sequences().is_empty(),
            "nothing here is an array without a name: {:?}",
            shape.unnamed_sequences()
        );
    }

    /// **One pass through an enum sees one variant.** The sweep
    /// re-runs the walk with the untaken variant planned in, and
    /// `buried` exists only under the second one — a probe that
    /// stopped after a single pass would report the type as carrying
    /// no arrays at all, which is the silent-under-report this whole
    /// module exists to rule out.
    #[test]
    fn test_the_probe_reaches_an_array_behind_a_second_enum_variant() {
        let arrays = derived_shape::<Branching>().key_bearing_sequences();
        assert!(
            arrays.contains("buried"),
            "the variant sweep has to reach every variant, not the first one: {arrays:?}"
        );
    }

    /// A type that contains itself has to bottom out rather than
    /// spin: the walk empties every container once a name reappears
    /// on its own path, and the array is still recorded from the
    /// outer sighting.
    #[test]
    fn test_the_probe_terminates_on_a_recursive_type() {
        let arrays = derived_shape::<Recursive>().key_bearing_sequences();
        assert!(
            arrays.contains("children"),
            "the recursive array is still an array: {arrays:?}"
        );
    }

    /// **Emptying every container is not enough to bottom out.** A
    /// required enum payload is a recursion [`Mode::Terminate`] has
    /// no emptier answer for, so terminating has to mean *choosing* —
    /// a unit variant where there is one, and otherwise a variant
    /// nothing has yet found a payload behind.
    ///
    /// `MutatorNode::Repeat { template: Box<MutatorNode> }` is this
    /// shape in the live model, and it survives only because `Void`
    /// is declared before it. Declaration order is not a property the
    /// format has an opinion about — an externally tagged enum reads
    /// the same however its variants are ordered, and both
    /// derivations reorder together — so a purely cosmetic reorder
    /// must not be able to stop the sweep. `Stop` is second here for
    /// exactly that reason.
    #[test]
    fn test_the_probe_terminates_through_a_boxed_enum_newtype_variant() {
        let shape = derived_shape::<Holder>();
        assert_eq!(
            shape.variants_of("Spinner"),
            Some(["Deeper".to_string(), "Stop".to_string()].as_slice()),
            "the sweep has to have reached the enum at all"
        );
        assert!(
            shape.key_bearing_sequences().contains("leaves"),
            "and to have walked past it to the rest of the container: {:?}",
            shape.key_bearing_sequences()
        );
    }

    /// An array at a place with no member name is reported rather
    /// than dropped. `format/schema.md` publishes member names, and
    /// there is none here to publish — so the honest answer is to
    /// hand it to a reader, not to leave a positional route the
    /// document cannot mention.
    #[test]
    fn test_the_probe_reports_an_array_that_has_no_member_name() {
        let shape = derived_shape::<Tupled>();
        assert_eq!(
            shape.unnamed_sequences().len(),
            1,
            "position 0 of a tuple variant is an array nothing names: {:?}",
            shape.unnamed_sequences()
        );
        assert!(
            shape.key_bearing_sequences().is_empty(),
            "and it must not be published under a name it does not have: {:?}",
            shape.key_bearing_sequences()
        );
    }
}
