// SPDX-License-Identifier: MPL-2.0

//! Keys a `.mindmap.json` carries that this build's model has no field
//! for — found at load, reported once each, and written back untouched
//! at save.
//!
//! **The problem this solves is forward compatibility across
//! versions.** A map authored by a newer build carries keys an older
//! build does not know. Refusing the load leaves the reader with
//! nothing; ignoring the keys is worse, because the editor resaves the
//! whole model and the next save deletes them. So the load keeps them:
//! every unrecognized key is captured with the route that leads to it,
//! warned about once, carried on
//! [`MindMap`](crate::mindmap::model::MindMap),
//! and spliced back into the serialized document before it reaches
//! disk.
//!
//! **Why the capture is not a per-type `#[serde(flatten)]` catch-all.**
//! A flattened `HashMap<String, Value>` field cannot go on a type that
//! derives `Copy`, `Eq`, or `Hash` — `serde_json::Value` implements
//! none of the three — and the graph reachable from a load is full of
//! such types (`Color`, `OrderedVec2`, `Position`, `Anchor`, `Range`,
//! `GlyphMatrix`, …). A catch-all on only the types that can hold one
//! would preserve keys in some places and silently drop them in
//! others, which is the failure the policy exists to remove. Capturing
//! at the deserializer instead covers **every** type in the graph, and
//! types the source walk cannot even see (anything `build.rs` emits
//! into `$OUT_DIR`), with no per-type opt-in and therefore no drift
//! surface.
//!
//! What holds the mechanism honest is the other direction: a reachable
//! type must not be able to *swallow* a key before the capture sees
//! it. `#[serde(deny_unknown_fields)]` would abort the load, and
//! `#[serde(flatten)]`, `#[serde(untagged)]` and `#[serde(tag = "…")]`
//! would absorb the key without ever reaching
//! `deserialize_ignored_any`. `mindmap::loader`'s
//! `test_no_loadable_type_can_swallow_an_unknown_key` walks the model's
//! own source and fails if any of them appears.
//!
//! # The model's own serialization is the oracle
//!
//! Two of the three hard problems here are solved by the same thing: a
//! **probe**, the `serde_json::Value` the freshly loaded model
//! serializes to. It is taken once per load, and only for a document
//! that carried something unrecognized, so a map this build authored
//! never pays for it.
//!
//! - **Levels the JSON has that serde's ignored-key path does not.**
//!   An externally tagged enum writes `{"Void": {…}}`, and the variant
//!   name never travels through a `MapAccess`, so serde's path is one
//!   step short. Deciding "is this one-member object a variant wrapper
//!   or the thing I am addressing?" from its member count alone is a
//!   guess, and it guesses wrong the moment a payload carries a key
//!   spelled like the variant. The probe answers it without guessing:
//!   **a key the model itself writes at a level cannot be the key
//!   serde ignored there**, so the walk descends past it. See
//!   [`expand_route`].
//! - **Containers the saver omits.** `#[serde(skip_serializing_if)]`
//!   means a container that holds its own default is simply absent
//!   from the saved document, and a route through it would die with
//!   nothing to hold on to. Comparing the authored document against
//!   the probe finds those levels at load time, so the save can put
//!   the container back before writing the key into it. See
//!   [`Scaffold`].
//! - **Array elements that move.** A route below an array is
//!   positional, and deleting an earlier element silently slides the
//!   route onto a different one. The probe's element fingerprints are
//!   captured alongside the index, so the save re-finds the element it
//!   was actually attached to — and refuses, loudly, when it cannot.
//!   See [`IndexAnchor`].

use serde::Deserialize;
use serde_json::{Map, Value};
use std::hash::{Hash, Hasher};

/// One step of the route from the document root to a captured key.
///
/// A route is what a JSON pointer would be, kept structured rather
/// than joined: a Dewey-decimal node id contains `.` and `/`-joining
/// it would make the route ambiguous to read back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    /// An object member, named by its key.
    Key(String),
    /// An array element, by zero-based index.
    Index(usize),
}

/// A content hash of one JSON value, used to recognize an array
/// element again after the model has been edited around it.
///
/// `Value`'s object member order is `BTreeMap` order, so
/// [`Value::to_string`] is canonical and two structurally equal values
/// always hash alike. The hash is compared only between two values
/// produced inside **one process run** (the load-time probe and the
/// save-time document), which is what makes `DefaultHasher` — whose
/// output is explicitly not stable across releases — sound here.
///
/// Cost: one `String` render of `value` plus one hash of it.
fn fingerprint(value: &Value) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    value.to_string().hash(&mut hasher);
    hasher.finish()
}

/// The identity of one array along a captured key's route, so the
/// save can tell "the element I was attached to" from "whatever is at
/// that index now".
///
/// Without this, deleting an earlier element of the same array
/// silently reattaches the key to its neighbor — the route still
/// resolves, just to the wrong element, which is worse than losing the
/// key because nothing about it looks wrong.
#[derive(Debug, Clone)]
struct IndexAnchor {
    /// Position in the route of the [`Step::Index`] this describes.
    step: usize,
    /// [`fingerprint`] of every element of the array as the model
    /// serialized it at load time, in order.
    siblings: Vec<u64>,
}

/// A container the saver leaves out, and the value that puts it back.
///
/// `#[serde(skip_serializing_if = "…")]` on a non-`Option` field means
/// a container holding its own default never reaches the saved
/// document at all. A captured key nested inside one has nowhere to
/// land, and would be lost on a **zero-edit** load → save — no user
/// action, no warning that means anything. So the load records the
/// authored container (with every captured key already lifted out of
/// it) and the save re-creates it on the way past.
///
/// Recorded only when the container is absent from the load-time probe
/// — that is, when a save of the map *as loaded* would already have
/// omitted it. That condition is what makes re-inserting the authored
/// container faithful rather than a resurrection: the model's value
/// for it satisfied the omission predicate at load and satisfies it
/// again at save, so the authored bytes still describe the model's
/// value. A container the model wrote at load and dropped later was
/// dropped **by an edit**, and that is a deliberate deletion the save
/// reports rather than undoes.
#[derive(Debug, Clone)]
struct Scaffold {
    /// Route of the object that has to receive [`Self::member`].
    owner: Vec<Step>,
    /// Name of the omitted container.
    member: String,
    /// The container as it was authored, minus every key this capture
    /// took out of it — those are spliced back at their own routes.
    value: Value,
}

/// One key the model has no field for, with the route that leads to
/// it, the value it held, and what the save needs in order to put it
/// back where it came from.
///
/// Plain data. Cost: the route's `String`s plus the captured value,
/// both moved out of the parsed document rather than cloned, plus one
/// `u64` per element of each array the route passes through.
#[derive(Debug, Clone)]
pub struct UnknownKey {
    /// Full route from the document root, the last step being the key
    /// itself. Literal: every level the JSON has is a step, including
    /// the ones serde's own path elides (see [`expand_route`]).
    route: Vec<Step>,
    /// The value the key held, taken out of the parsed document.
    value: Value,
    /// The container the saver omits and the splice has to rebuild,
    /// when there is one.
    scaffold: Option<Scaffold>,
    /// Element identities for every positional step of the route.
    anchors: Vec<IndexAnchor>,
    /// Whether a save can put this key back at all. `false` only when
    /// the route dies at a positional step the load could not anchor
    /// and the splice cannot rebuild — the one shape this mechanism
    /// cannot honor, and the load says so rather than promising
    /// otherwise.
    preservable: bool,
}

impl UnknownKey {
    /// The key as the author wrote it. O(1).
    pub fn key(&self) -> &str {
        match self.route.last() {
            Some(Step::Key(key)) => key,
            // Unreachable in practice: serde only ignores *named*
            // members, so a captured route always ends in a key.
            // Degrading to the empty name keeps a malformed route out
            // of a panic on an interactive path (CODE_CONVENTIONS §9).
            _ => "",
        }
    }

    /// The addressable part of the document the key sits in, stamped
    /// the way `maptool verify` and [`MindMap::edge_locations`] stamp
    /// it — `node "1.2"`, `edge[3]`, `palette "coral"`,
    /// `custom_mutations[0]`, `canvas`, or `map` for a key at the top
    /// level.
    ///
    /// [`MindMap::edge_locations`]: crate::mindmap::model::MindMap::edge_locations
    ///
    /// Cost: one `String` allocation.
    pub fn location(&self) -> String {
        location_of(&self.route).0
    }

    /// Where inside [`Self::location`] the key sits, rendered as a
    /// field path (`style.shpe`, `sections[0].txet`, or just the key
    /// when it hangs directly off the part).
    ///
    /// Cost: one `String` allocation.
    pub fn path_within_location(&self) -> String {
        let (_, rest) = location_of(&self.route);
        let mut out = String::new();
        for step in rest {
            match step {
                Step::Key(key) => {
                    if !out.is_empty() {
                        out.push('.');
                    }
                    out.push_str(key);
                }
                Step::Index(index) => out.push_str(&format!("[{index}]")),
            }
        }
        out
    }

    /// The value the key held in the authored file — exactly what a
    /// save writes back. O(1) borrow.
    pub fn value(&self) -> &Value {
        &self.value
    }

    /// The single line the loader logs for this key, and the single
    /// place its wording lives.
    ///
    /// Carries the `"<area>: message"` prefix CODE_CONVENTIONS §9
    /// requires, so the `log::warn!` call site formats nothing of its
    /// own and a test can assert the exact text.
    ///
    /// **Two wordings, because there are two outcomes.** Almost every
    /// captured key is written back byte-for-byte by the next save,
    /// and the line says so. A key whose route the save cannot
    /// reconstruct is told apart at load time rather than promised
    /// something the save will not deliver — see
    /// [`UnknownKey::preservable`].
    ///
    /// Cost: one `String` allocation.
    pub fn warning(&self) -> String {
        if self.preservable {
            format!(
                "loader: {}: unrecognized key `{}` — this build has no field for it, so it is \
                 kept as written and saved back unchanged. Check the spelling if you meant an \
                 existing key; see format/schema.md.",
                self.location(),
                self.path_within_location()
            )
        } else {
            format!(
                "loader: {}: unrecognized key `{}` — this build has no field for it, and the \
                 place it sits in has no counterpart in what this build writes, so the next \
                 save will not carry it. Copy it out before saving if you need it; see \
                 format/schema.md.",
                self.location(),
                self.path_within_location()
            )
        }
    }
}

/// Split a route into the location stamp of the addressable part it
/// falls in and the remaining steps inside that part.
fn location_of(route: &[Step]) -> (String, &[Step]) {
    match route {
        [Step::Key(head), Step::Key(id), rest @ ..] if head == "nodes" => (format!("node {id:?}"), rest),
        [Step::Key(head), Step::Key(name), rest @ ..] if head == "palettes" => {
            (format!("palette {name:?}"), rest)
        }
        [Step::Key(head), Step::Index(i), rest @ ..] if head == "edges" => (format!("edge[{i}]"), rest),
        [Step::Key(head), Step::Index(i), rest @ ..] if head == "custom_mutations" => {
            (format!("custom_mutations[{i}]"), rest)
        }
        [Step::Key(head), rest @ ..] if head == "canvas" => ("canvas".to_string(), rest),
        rest => ("map".to_string(), rest),
    }
}

/// Every unrecognized key one load found, in the order the
/// deserializer met them.
///
/// Lives on [`MindMap`](crate::mindmap::model::MindMap) so the save path can
/// put the keys back. `#[serde(skip)]` on that field is what keeps
/// this out of the on-disk shape — the keys are written at their own
/// routes, not collected into a side object.
#[derive(Debug, Clone, Default)]
pub struct UnknownKeys {
    entries: Vec<UnknownKey>,
}

impl UnknownKeys {
    /// Whether the load found nothing it did not understand — true for
    /// every map this build authored itself. O(1).
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// How many keys were captured. O(1).
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// The captured keys, in the order the deserializer met them. O(1).
    pub fn iter(&self) -> std::slice::Iter<'_, UnknownKey> {
        self.entries.iter()
    }

    /// Put every captured key back into a freshly serialized document.
    ///
    /// # What it reconstructs before writing
    ///
    /// A route does not have to survive serialization intact for the
    /// key to. Two things are rebuilt on the way past, and both exist
    /// because without them a key is lost with no user action at all:
    ///
    /// - **A container the saver omitted.** A `skip_serializing_if`
    ///   field holding its own default is absent from the saved
    ///   document, so a route through it has nothing to walk. The load
    ///   recorded the authored container ([`Scaffold`]); it is put back
    ///   before the key is written into it. This is what makes a
    ///   **zero-edit load → save key-set-preserving**, which is the
    ///   whole point of the mechanism.
    /// - **An array element that moved.** Every positional step
    ///   carries the load-time fingerprints of the array's elements
    ///   ([`IndexAnchor`]), so deleting or reordering earlier elements
    ///   re-finds the right one instead of silently writing the key
    ///   onto its neighbor.
    ///
    /// # What it refuses, and says so
    ///
    /// - The route no longer resolves — the node, edge, or section the
    ///   key hung off was deleted. The key goes with it.
    /// - The element the key was attached to cannot be identified any
    ///   more: it was edited *and* its siblings changed, so neither its
    ///   fingerprint nor its position is evidence. Refusing is the
    ///   point — writing the key onto a different element would be
    ///   wrong data rather than missing data.
    /// - This build has since grown a field of that name at that
    ///   place. The model owns the name now and the captured value is
    ///   stale.
    ///
    /// Each of the three is a distinct `warn!`, because each asks the
    /// reader to do something different.
    ///
    /// # What is still not preserved
    ///
    /// Nothing else on the load graph. `format/schema.md`
    /// §"Where a preserved key can still be lost" is the checked
    /// statement of the residue and
    /// `loader::tests::test_a_zero_edit_round_trip_keeps_every_authored_key`
    /// is what keeps it honest.
    ///
    /// Cost: O(captured keys × route length), plus one fingerprint of
    /// each array the routes pass through, plus one `Value` clone per
    /// captured key. Nothing at all when the capture is empty, which
    /// is every map this build wrote.
    pub fn splice_into(&self, document: &mut Value) {
        for entry in &self.entries {
            // Rebuild the omitted container first: until it is there,
            // the key's own route has no owner to land in.
            if let Some(scaffold) = &entry.scaffold {
                if let Some(owner) = resolve_owner(document, &scaffold.owner, &entry.anchors) {
                    if !owner.contains_key(&scaffold.member) {
                        owner.insert(scaffold.member.clone(), scaffold.value.clone());
                    }
                }
            }
            let Some((Step::Key(key), parent)) = entry.route.split_last() else {
                continue;
            };
            let Some(owner) = resolve_owner(document, parent, &entry.anchors) else {
                log::warn!(
                    "loader: {}: unrecognized key `{}` could not be written back — the place \
                     it was loaded from is no longer in the document, or the element it hung \
                     off can no longer be identified. It is dropped from the saved file.",
                    entry.location(),
                    entry.path_within_location()
                );
                continue;
            };
            if owner.contains_key(key) {
                log::warn!(
                    "loader: {}: unrecognized key `{}` was not written back — this build now \
                     writes a key of that name there, and the value it writes wins. The \
                     value read from the file is dropped.",
                    entry.location(),
                    entry.path_within_location()
                );
                continue;
            }
            owner.insert(key.clone(), entry.value.clone());
        }
    }
}

/// Follow a literal route (as [`expand_route`] produced) and hand back
/// the object at the end of it, resolving each positional step against
/// the element identities the load recorded.
///
/// No wrapper skipping and no guessing about object levels: each
/// `Key` step must name a member that is really there. That
/// strictness is the point on the save path — a route that no longer
/// resolves means the document changed shape, and the caller reports
/// that rather than placing the key somewhere plausible.
///
/// Cost: O(route length), plus one fingerprint pass over each array a
/// positional step lands on.
fn resolve_owner<'v>(
    document: &'v mut Value,
    route: &[Step],
    anchors: &[IndexAnchor],
) -> Option<&'v mut Map<String, Value>> {
    let mut current = document;
    for (position, step) in route.iter().enumerate() {
        current = match (current, step) {
            (Value::Object(members), Step::Key(key)) => members.get_mut(key)?,
            (Value::Array(items), Step::Index(index)) => {
                let anchor = anchors.iter().find(|anchor| anchor.step == position);
                let resolved = resolve_index(items, *index, anchor)?;
                items.get_mut(resolved)?
            }
            _ => return None,
        };
    }
    current.as_object_mut()
}

/// Which element of `items` the route's `index` means *now*.
///
/// The recorded index alone is not an answer: deleting an earlier
/// element slides every later one down, and the index then names a
/// different element with no sign that anything went wrong. So the
/// load recorded what the array looked like, and this asks three
/// questions in order, stopping at the first that identifies exactly
/// one element:
///
/// 1. **Is the element still at that index?** The overwhelmingly
///    common answer, and free.
/// 2. **Is it somewhere else in the array?** Reordering
///    (`sections.reverse()`) and deleting a *different* element both
///    land here, and both re-find the right element. Only a unique
///    match counts; two identical elements are not evidence.
/// 3. **Is this array the same array with only that one element
///    edited?** Same length, and every other element byte-identical to
///    what the load saw. Then the element at the index is the same
///    element, changed — editing a section's text must not cost it the
///    keys the section carried.
///
/// Anything else — the element is gone, or it changed *and* its
/// siblings did — is `None`. The caller warns and drops the key.
/// Writing it onto whatever now sits at the index is the one outcome
/// worth avoiding: wrong data reads as authored, missing data does
/// not.
///
/// Without an anchor (a route below a container the model does not
/// serialize, where there was nothing to fingerprint) the index is
/// taken literally, which is all there is to go on.
fn resolve_index(items: &[Value], index: usize, anchor: Option<&IndexAnchor>) -> Option<usize> {
    let Some(anchor) = anchor else {
        return (index < items.len()).then_some(index);
    };
    let want = *anchor.siblings.get(index)?;
    let now: Vec<u64> = items.iter().map(fingerprint).collect();
    if now.get(index) == Some(&want) {
        return Some(index);
    }
    let mut matches = now.iter().enumerate().filter(|(_, seen)| **seen == want);
    if let (Some((only, _)), None) = (matches.next(), matches.next()) {
        return Some(only);
    }
    let only_this_one_changed = now.len() == anchor.siblings.len()
        && now
            .iter()
            .zip(&anchor.siblings)
            .enumerate()
            .all(|(position, (seen, was))| position == index || seen == was);
    only_this_one_changed.then_some(index)
}

/// Deserialize `T` from `json`, collecting the route of every key the
/// model had no field for.
///
/// The typed value is produced by the same one-pass parse a load
/// already paid for; the routes come from wrapping that parse so an
/// ignored member reports where it was. A map with nothing unknown in
/// it costs the empty `Vec` and no more — in particular, no second
/// pass over the document.
///
/// Errors are `serde_json`'s own, unchanged, so a caller can still
/// diagnose them.
///
/// Cost: one parse of `json`, plus one `Vec<Step>` per unrecognized
/// key.
pub fn deserialize_capturing<'de, T: Deserialize<'de>>(
    json: &'de str,
) -> Result<(T, Vec<Vec<Step>>), serde_json::Error> {
    let mut routes: Vec<Vec<Step>> = Vec::new();
    let mut deserializer = serde_json::Deserializer::from_str(json);
    let value: T = serde_ignored::deserialize(&mut deserializer, |path| routes.push(route_of(&path)))?;
    // `serde_json::from_str` does this for us; a hand-driven
    // `Deserializer` has to, or trailing garbage after the closing
    // brace parses clean. `loader::tests::test_trailing_garbage_after_the_document_is_rejected`
    // is what notices when it goes away.
    deserializer.end()?;
    Ok((value, routes))
}

/// Deserialize `T` out of an already-parsed document, collecting the
/// same routes [`deserialize_capturing`] collects.
///
/// The tolerant load path needs this: it excises the constructs this
/// build cannot name from a `serde_json::Value` and then has to read
/// the remainder, which is no longer the text on disk. `&Value` is
/// itself a `Deserializer` and reports ignored members exactly as the
/// streaming one does, so the capture is the same capture — no
/// buffering through serde's `Content`, and therefore no second way
/// for a key to go missing.
///
/// Cost: one walk of `document`, plus one `Vec<Step>` per
/// unrecognized key.
pub fn deserialize_value_capturing<T: serde::de::DeserializeOwned>(
    document: &Value,
) -> Result<(T, Vec<Vec<Step>>), serde_json::Error> {
    let mut routes: Vec<Vec<Step>> = Vec::new();
    let value: T = serde_ignored::deserialize(document, |path| routes.push(route_of(&path)))?;
    Ok((value, routes))
}

/// Lift routes collected by [`deserialize_capturing`] into
/// [`UnknownKeys`], moving each key's value out of `document` and
/// working out what a save will need in order to put it back.
///
/// `document` is the same JSON parsed as a `serde_json::Value`.
/// `probe` is the model's own serialization of what that JSON loaded
/// as — the oracle described in the module header, and `None` only
/// when serializing the model failed, in which case the capture
/// degrades to the routes alone.
///
/// The values are **taken**, not copied, so the captured keys are gone
/// from `document` afterwards — the caller is expected to be done with
/// it. That also matters for correctness: a rebuilt container is
/// snapshotted from `document` *after* every capture has been lifted
/// out, so it cannot write a captured key a second time.
///
/// A route that does not resolve is skipped. That is not reachable
/// from a route this crate produced; it is what keeps a caller
/// pairing a route list with the wrong document out of a panic.
///
/// Cost: O(routes × route length), plus one fingerprint of each array
/// the routes pass through, plus one clone of each container the saver
/// omits.
pub fn take_from(document: &mut Value, probe: Option<&Value>, routes: Vec<Vec<Step>>) -> UnknownKeys {
    // Pass 1 lifts every captured value out. Pass 2 has to run
    // afterwards rather than alongside: a container it snapshots for
    // rebuilding must not still contain a key that will be spliced
    // back separately, or the save writes that key twice — once
    // inside the rebuilt container, once at its own route.
    let mut taken: Vec<(Vec<Step>, Value)> = Vec::with_capacity(routes.len());
    for route in routes {
        let Some(route) = expand_route(document, probe, &route) else {
            continue;
        };
        let Some((Step::Key(key), parent)) = route.split_last() else {
            continue;
        };
        let (key, parent) = (key.clone(), parent.to_vec());
        let Some(value) = resolve_owner(document, &parent, &[]).and_then(|owner| owner.remove(&key)) else {
            continue;
        };
        taken.push((route, value));
    }
    let entries = taken
        .into_iter()
        .map(|(route, value)| {
            let plan = plan_write_back(document, probe, &route);
            UnknownKey {
                route,
                value,
                scaffold: plan.scaffold,
                anchors: plan.anchors,
                preservable: plan.preservable,
            }
        })
        .collect();
    UnknownKeys { entries }
}

/// What [`UnknownKeys::splice_into`] will need in order to write one
/// captured key back where it came from.
struct WriteBackPlan {
    anchors: Vec<IndexAnchor>,
    scaffold: Option<Scaffold>,
    preservable: bool,
}

/// Walk the probe alongside a captured key's route and record what the
/// save will be missing.
///
/// Two things come out of the walk. Every positional step gets an
/// [`IndexAnchor`], so the save can recognize the element rather than
/// trust the index. And the **first** level the probe does not have is
/// a container `skip_serializing_if` leaves out of the saved document
/// — the load records it as a [`Scaffold`] so the save can put it
/// back. Recording only the first is enough: re-inserting the authored
/// container restores everything below it in one move.
///
/// Cost: O(route length), plus one fingerprint pass over each array on
/// the way.
fn plan_write_back(document: &Value, probe: Option<&Value>, route: &[Step]) -> WriteBackPlan {
    let unanchored = WriteBackPlan {
        anchors: Vec::new(),
        scaffold: None,
        preservable: true,
    };
    let (Some(probe), Some((_, parent))) = (probe, route.split_last()) else {
        return unanchored;
    };
    let mut anchors: Vec<IndexAnchor> = Vec::new();
    let mut current = probe;
    for (position, step) in parent.iter().enumerate() {
        let next = match (current, step) {
            (Value::Object(members), Step::Key(key)) => members.get(key),
            (Value::Array(items), Step::Index(index)) => {
                anchors.push(IndexAnchor {
                    step: position,
                    siblings: items.iter().map(fingerprint).collect(),
                });
                items.get(*index)
            }
            _ => None,
        };
        let Some(next) = next else {
            // The saver omits this level. Rebuilding it needs a name
            // to hang the authored container off; a positional step
            // has none, and inventing an array element the model does
            // not hold would put a value into the document that
            // nothing in the model stands behind.
            let (Step::Key(member), Some(value)) = (step, value_at(document, &route[..=position])) else {
                return WriteBackPlan {
                    anchors,
                    scaffold: None,
                    preservable: false,
                };
            };
            return WriteBackPlan {
                anchors,
                scaffold: Some(Scaffold {
                    owner: route[..position].to_vec(),
                    member: member.clone(),
                    value: value.clone(),
                }),
                preservable: true,
            };
        };
        current = next;
    }
    WriteBackPlan {
        anchors,
        scaffold: None,
        preservable: true,
    }
}

/// Follow a literal route through a document without modifying it.
fn value_at<'v>(root: &'v Value, route: &[Step]) -> Option<&'v Value> {
    let mut current = root;
    for step in route {
        current = match (current, step) {
            (Value::Object(members), Step::Key(key)) => members.get(key)?,
            (Value::Array(items), Step::Index(index)) => items.get(*index)?,
            _ => return None,
        };
    }
    Some(current)
}

/// Flatten one `serde_ignored::Path` into a route.
///
/// `Some` / `NewtypeStruct` / `NewtypeVariant` are dropped: serde adds
/// them to mark a layer of the *Rust* type that the JSON does not have
/// a level for.
fn route_of(path: &serde_ignored::Path) -> Vec<Step> {
    fn walk(path: &serde_ignored::Path, out: &mut Vec<Step>) {
        use serde_ignored::Path;
        match path {
            Path::Root => {}
            Path::Seq { parent, index } => {
                walk(parent, out);
                out.push(Step::Index(*index));
            }
            Path::Map { parent, key } => {
                walk(parent, out);
                out.push(Step::Key(key.clone()));
            }
            Path::Some { parent } | Path::NewtypeStruct { parent } | Path::NewtypeVariant { parent } => {
                walk(parent, out)
            }
        }
    }
    let mut out = Vec::new();
    walk(path, &mut out);
    out
}

/// What the next step of a route addresses at the level the walk is
/// standing on.
enum Descend {
    /// The step names a member (or an element) of this very value.
    Member,
    /// This value is a level serde's path elides — an externally
    /// tagged enum's variant wrapper — named by the tag inside.
    Wrapper(String),
    /// Neither; the route does not lead anywhere from here.
    Nowhere,
}

/// Rewrite a route from serde's ignored-key path into one that names
/// every JSON level literally.
///
/// **What has to be put back: externally tagged enum levels.**
/// `MutatorNode::Void` is written `{"Void": { … }}`, and the variant
/// name never travels through a `MapAccess`, so serde's path has one
/// fewer step than the JSON has levels.
///
/// **How the missing level is identified.** Not by counting members —
/// that is a guess, and it guesses wrong on
/// `{"Void": {"channel": 0, "Void": 99}}`, where the payload carries a
/// key spelled like the variant. It is identified by asking the model
/// what *it* writes at that level, via `probe`:
///
/// - **A key the model writes here cannot be the ignored key.** serde
///   only reports members no field claimed, so if the probe has the
///   key at this level, the one serde meant is deeper and this level
///   is a wrapper. That single rule is what makes the collision case
///   come out right.
/// - **A key the model does not write, that the author did, is the
///   member.** Either the unrecognized key itself or a field the saver
///   omits.
/// - Otherwise the level is a wrapper, and the probe has to agree on
///   the tag's spelling for the walk to take it.
///
/// **Below a container the model does not serialize there is no
/// probe**, and the walk falls back to the shape: a one-member object
/// that cannot answer the step is taken as a wrapper. That is the old
/// guess, kept only where nothing better exists, and it is bounded —
/// the containers the saver omits (`format/schema.md`
/// §"Where a preserved key can still be lost") hold no externally
/// tagged enum today, so the fallback resolves plain objects.
///
/// Doing the expansion once, here, is what lets
/// [`UnknownKeys::splice_into`] walk a saved document with no
/// heuristic at all: the stored route is a literal path.
///
/// Recursive, bounded by `route.len()`, which is bounded by the parse
/// that produced it — `serde_json` caps recursion at 128 levels.
///
/// Cost: O(route length), plus one `String` clone per step of the
/// rewritten route.
fn expand_route(document: &Value, probe: Option<&Value>, route: &[Step]) -> Option<Vec<Step>> {
    fn walk<'v>(
        authored: &'v Value,
        model: Option<&Value>,
        route: &[Step],
        out: &mut Vec<Step>,
    ) -> Option<&'v Value> {
        let Some((step, rest)) = route.split_first() else {
            return Some(authored);
        };
        let mut authored = authored;
        let mut model = model;
        loop {
            match descend_kind(authored, model, step, rest.is_empty()) {
                Descend::Member => {
                    let next = child(authored, step)?;
                    out.push(step.clone());
                    return walk(next, model.and_then(|model| child(model, step)), rest, out);
                }
                Descend::Wrapper(tag) => {
                    model = model.and_then(|model| model.get(&tag));
                    authored = authored.get(&tag)?;
                    out.push(Step::Key(tag));
                }
                Descend::Nowhere => return None,
            }
        }
    }
    let mut out = Vec::with_capacity(route.len());
    walk(document, probe, route, &mut out).map(|_| out)
}

/// The child of `value` that `step` addresses, if it is there.
fn child<'v>(value: &'v Value, step: &Step) -> Option<&'v Value> {
    match (value, step) {
        (Value::Object(members), Step::Key(key)) => members.get(key),
        (Value::Array(items), Step::Index(index)) => items.get(*index),
        _ => None,
    }
}

/// Whether `step` addresses something in `authored` itself, or
/// `authored` is a variant wrapper the walk has to descend through
/// first. See [`expand_route`] for why the model decides this rather
/// than the member count.
fn descend_kind(authored: &Value, model: Option<&Value>, step: &Step, last: bool) -> Descend {
    let model_writes_it = match (model, step) {
        (Some(Value::Object(members)), Step::Key(key)) => members.contains_key(key),
        _ => false,
    };
    // A key the model writes is a real member — except at the last
    // step, where it cannot be: serde does not ignore a member a field
    // claimed, so the key it meant is one level down.
    if model_writes_it && !last {
        return Descend::Member;
    }
    if !model_writes_it && child(authored, step).is_some() {
        return Descend::Member;
    }
    wrapper_of(authored, model)
}

/// Read the variant tag off a level serde's path elides.
///
/// The authored value has to be a one-member object — that is what an
/// externally tagged variant is written as. When the model still has
/// this level it must name the same single member, which is what turns
/// the answer from a guess into a check; the walk gives up rather than
/// descend past a level the model disagrees about.
fn wrapper_of(authored: &Value, model: Option<&Value>) -> Descend {
    let Some(members) = authored.as_object() else {
        return Descend::Nowhere;
    };
    if members.len() != 1 {
        return Descend::Nowhere;
    }
    let Some((tag, _)) = members.iter().next() else {
        return Descend::Nowhere;
    };
    match model.and_then(Value::as_object) {
        Some(model_members) if model_members.len() == 1 && model_members.contains_key(tag) => {
            Descend::Wrapper(tag.clone())
        }
        // The model has this level and does not agree that it is a
        // one-member wrapper spelled that way.
        Some(_) => Descend::Nowhere,
        // No model here at all — below a container the saver omits.
        None => Descend::Wrapper(tag.clone()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::util::test_logger;

    /// Build a route out of a JSON array literal, so a table of
    /// cases reads as paths rather than as constructor calls.
    fn route(steps: serde_json::Value) -> Vec<Step> {
        steps
            .as_array()
            .expect("route literal")
            .iter()
            .map(|step| match step {
                serde_json::Value::String(key) => Step::Key(key.clone()),
                serde_json::Value::Number(index) => Step::Index(index.as_u64().expect("index") as usize),
                other => panic!("unsupported route step {other}"),
            })
            .collect()
    }

    /// An entry with nothing for the save to rebuild — the shape every
    /// capture had before scaffolds and anchors existed.
    fn plain(steps: serde_json::Value, value: serde_json::Value) -> UnknownKey {
        UnknownKey {
            route: route(steps),
            value,
            scaffold: None,
            anchors: Vec::new(),
            preservable: true,
        }
    }

    /// A route through an externally tagged enum has one fewer step
    /// than the JSON has levels — `{"Void": {…}}` costs a level that
    /// never travels through a `MapAccess`. The expansion has to put
    /// it back, or every key captured inside a mutator lands nowhere.
    #[test]
    fn test_expand_route_restores_an_external_enum_tag() {
        let document = serde_json::json!({
            "custom_mutations": [{"mutator": {"Void": {"channel": 0, "surprise": 7}}}]
        });
        let probe = serde_json::json!({
            "custom_mutations": [{"mutator": {"Void": {"channel": 0, "children": []}}}]
        });
        let expanded = expand_route(
            &document,
            Some(&probe),
            &route(serde_json::json!(["custom_mutations", 0, "mutator", "surprise"])),
        )
        .expect("the enum payload must be reachable");
        assert_eq!(
            expanded,
            route(serde_json::json!([
                "custom_mutations",
                0,
                "mutator",
                "Void",
                "surprise"
            ]))
        );
    }

    /// **The case the member-count guess got wrong.** An unknown key
    /// spelled like the variant it sits inside made the old test
    /// (`one member, and it isn't the thing I'm looking for`) decide
    /// the wrapper was not a wrapper: it reported a key named after a
    /// variant this build *does* know, at a location that does not
    /// exist, holding the whole payload — and then refused to write it
    /// back. Asking the model settles it: the model writes `Void` at
    /// the wrapper level, so the ignored key cannot be the one there.
    #[test]
    fn test_expand_route_resolves_a_key_spelled_like_its_own_variant() {
        let document = serde_json::json!({
            "custom_mutations": [{"mutator": {"Void": {"channel": 0, "Void": 99}}}]
        });
        let probe = serde_json::json!({
            "custom_mutations": [{"mutator": {"Void": {"channel": 0, "children": []}}}]
        });
        let expanded = expand_route(
            &document,
            Some(&probe),
            &route(serde_json::json!(["custom_mutations", 0, "mutator", "Void"])),
        )
        .expect("the collided key must be reachable");
        assert_eq!(
            expanded,
            route(serde_json::json!([
                "custom_mutations",
                0,
                "mutator",
                "Void",
                "Void"
            ])),
            "the key inside the payload is the unrecognized one, not the variant tag"
        );
    }

    /// The tag skip must not fire on a one-member object that *is*
    /// the addressed thing — a node whose only authored key is the
    /// one being captured.
    #[test]
    fn test_expand_route_prefers_a_real_member_over_a_tag_skip() {
        let document = serde_json::json!({"nodes": {"0": {"mystery": 1}}});
        let steps = route(serde_json::json!(["nodes", "0", "mystery"]));
        assert_eq!(expand_route(&document, None, &steps), Some(steps.clone()));
    }

    /// A route that leads nowhere resolves to nothing rather than to
    /// something near it.
    #[test]
    fn test_expand_route_returns_none_for_a_route_that_is_gone() {
        let document = serde_json::json!({"nodes": {"0": {}}});
        assert!(expand_route(&document, None, &route(serde_json::json!(["nodes", "9", "x"]))).is_none());
    }

    /// The expanded route is literal, so the save-side walk needs no
    /// heuristic — and must not apply one. A one-member object on the
    /// way back is just a one-member object.
    #[test]
    fn test_resolve_owner_takes_a_route_literally() {
        let mut document = serde_json::json!({"canvas": {"background_color": "#000"}});
        let owner = resolve_owner(&mut document, &route(serde_json::json!(["canvas"])), &[])
            .expect("the canvas must be reachable");
        owner.insert("grid_snap".to_string(), serde_json::json!(8));
        assert_eq!(document["canvas"]["grid_snap"], serde_json::json!(8));
    }

    /// Capture takes the key out of the document it was read from, so
    /// nothing downstream sees a half-owned value.
    #[test]
    fn test_take_from_moves_the_value_out_of_the_document() {
        let mut document = serde_json::json!({"nodes": {"0": {"mystery": [1, 2]}}});
        let captured = take_from(
            &mut document,
            None,
            vec![route(serde_json::json!(["nodes", "0", "mystery"]))],
        );
        assert_eq!(captured.len(), 1);
        assert_eq!(
            captured.iter().next().expect("one entry").value(),
            &serde_json::json!([1, 2])
        );
        assert_eq!(document, serde_json::json!({"nodes": {"0": {}}}));
    }

    /// **The zero-edit loss, at the unit that causes it.** The saver
    /// omits a container holding its own default, so the route through
    /// it dies with nothing to hold on to. The capture records the
    /// authored container and the splice puts it back.
    #[test]
    fn test_take_from_records_the_container_the_saver_omits() {
        let mut document = serde_json::json!({"nodes": {"0": {"offset": {"x": 0, "grid_hint": 7}}}});
        // The model writes no `offset` at all — it held its default.
        let probe = serde_json::json!({"nodes": {"0": {}}});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["nodes", "0", "offset", "grid_hint"]))],
        );
        let mut saved = serde_json::json!({"nodes": {"0": {}}});
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"nodes": {"0": {"offset": {"x": 0, "grid_hint": 7}}}}),
            "the omitted container has to come back, or the key has nowhere to go"
        );
    }

    /// The rebuilt container is snapshotted *after* the capture lifted
    /// its keys out, so two unknown keys in one omitted container are
    /// each written once rather than one of them twice.
    #[test]
    fn test_two_keys_in_one_omitted_container_are_each_written_once() {
        let mut document =
            serde_json::json!({"nodes": {"0": {"offset": {"x": 0, "one": 1, "two": 2}}}});
        let probe = serde_json::json!({"nodes": {"0": {}}});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![
                route(serde_json::json!(["nodes", "0", "offset", "one"])),
                route(serde_json::json!(["nodes", "0", "offset", "two"])),
            ],
        );
        let mut saved = serde_json::json!({"nodes": {"0": {}}});
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"nodes": {"0": {"offset": {"x": 0, "one": 1, "two": 2}}}})
        );
    }

    /// A container the model *did* write at load and does not write at
    /// save was emptied by an edit. Rebuilding it from what the author
    /// wrote would resurrect the value the edit removed, so the key is
    /// dropped and said so instead.
    #[test]
    fn test_a_container_an_edit_emptied_is_not_resurrected() {
        let mut document = serde_json::json!({"nodes": {"zz-edit": {"offset": {"x": 5, "hint": 7}}}});
        let probe = serde_json::json!({"nodes": {"zz-edit": {"offset": {"x": 5.0}}}});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["nodes", "zz-edit", "offset", "hint"]))],
        );
        // The user moved the offset back to its default; the saver
        // leaves it out entirely.
        let mut saved = serde_json::json!({"nodes": {"zz-edit": {}}});
        test_logger::install();
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"nodes": {"zz-edit": {}}}),
            "the edit deleted the container; the save must not put the old one back"
        );
        assert_eq!(
            test_logger::lines_containing("zz-edit").len(),
            1,
            "the drop has to be reported"
        );
    }

    /// A key whose place in the document is gone is dropped, and says
    /// so. Silence here would be the one hole in the preservation
    /// guarantee, so it is a `warn!` and a test that the surviving
    /// document is not corrupted around it.
    #[test]
    fn test_splice_skips_a_route_the_document_no_longer_has() {
        let keys = UnknownKeys {
            entries: vec![plain(
                serde_json::json!(["nodes", "zz-gone", "x"]),
                serde_json::json!(1),
            )],
        };
        let mut document = serde_json::json!({"nodes": {"0": {}}});
        test_logger::install();
        keys.splice_into(&mut document);
        assert_eq!(document, serde_json::json!({"nodes": {"0": {}}}));
        let reported = test_logger::lines_containing("zz-gone");
        assert_eq!(reported.len(), 1, "expected one warning, got {reported:?}");
        assert!(
            reported[0].contains("no longer in the document"),
            "the reader has to be told the place is gone: {}",
            reported[0]
        );
    }

    /// A build that grew a field of the captured name owns that name
    /// now; the stale capture must not overwrite what the model just
    /// wrote — and the line that reports it must not claim the place
    /// is missing, because the place is right there.
    #[test]
    fn test_splice_does_not_overwrite_a_key_the_model_now_writes() {
        let keys = UnknownKeys {
            entries: vec![plain(
                serde_json::json!(["canvas", "zz-grid-snap"]),
                serde_json::json!(8),
            )],
        };
        let mut document = serde_json::json!({"canvas": {"zz-grid-snap": 16}});
        test_logger::install();
        keys.splice_into(&mut document);
        assert_eq!(document["canvas"]["zz-grid-snap"], serde_json::json!(16));
        let reported = test_logger::lines_containing("zz-grid-snap");
        assert_eq!(reported.len(), 1, "expected one warning, got {reported:?}");
        assert!(
            reported[0].contains("this build now writes a key of that name"),
            "the two reasons a key is not written back must not share one message: {}",
            reported[0]
        );
    }

    /// **Deleting an edge must not hand its neighbor a key it never
    /// had.** The route is positional, so `edges.remove(0)` slides
    /// every later edge down one; the fingerprints recorded at load
    /// are what re-find the edge the key was actually attached to.
    #[test]
    fn test_a_positional_route_follows_its_element_when_an_earlier_one_is_deleted() {
        let mut document = serde_json::json!({"edges": [
            {"from_id": "0", "to_id": "1"},
            {"from_id": "0", "to_id": "2", "authored_note": "keep me"},
            {"from_id": "0", "to_id": "3"}
        ]});
        let probe = serde_json::json!({"edges": [
            {"from_id": "0", "to_id": "1"},
            {"from_id": "0", "to_id": "2"},
            {"from_id": "0", "to_id": "3"}
        ]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["edges", 1, "authored_note"]))],
        );
        let mut saved = serde_json::json!({"edges": [
            {"from_id": "0", "to_id": "2"},
            {"from_id": "0", "to_id": "3"}
        ]});
        captured.splice_into(&mut saved);
        assert_eq!(
            saved["edges"][0]["authored_note"],
            serde_json::json!("keep me"),
            "the note belongs to edge 0→2 wherever that edge now sits"
        );
        assert_eq!(saved["edges"][1].get("authored_note"), None);
    }

    /// Reordering moves the element without changing it, so the
    /// fingerprint finds it again.
    #[test]
    fn test_a_positional_route_follows_its_element_through_a_reversal() {
        let mut document = serde_json::json!({"sections": [
            {"text": "a", "note": 1},
            {"text": "b"}
        ]});
        let probe = serde_json::json!({"sections": [{"text": "a"}, {"text": "b"}]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["sections", 0, "note"]))],
        );
        let mut saved = serde_json::json!({"sections": [{"text": "b"}, {"text": "a"}]});
        captured.splice_into(&mut saved);
        assert_eq!(saved["sections"][1]["note"], serde_json::json!(1));
        assert_eq!(saved["sections"][0].get("note"), None);
    }

    /// Editing the element the key hangs off must not cost it the key
    /// — that is an ordinary edit, and the element is still the same
    /// element. Recognized by everything around it being untouched.
    #[test]
    fn test_a_positional_route_survives_an_edit_to_its_own_element() {
        let mut document = serde_json::json!({"sections": [
            {"text": "a", "note": 1},
            {"text": "b"}
        ]});
        let probe = serde_json::json!({"sections": [{"text": "a"}, {"text": "b"}]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["sections", 0, "note"]))],
        );
        let mut saved = serde_json::json!({"sections": [{"text": "A"}, {"text": "b"}]});
        captured.splice_into(&mut saved);
        assert_eq!(saved["sections"][0]["note"], serde_json::json!(1));
    }

    /// When the element cannot be identified any more — it changed
    /// *and* the array around it changed — the key is dropped with a
    /// warning rather than written onto whatever now sits at the
    /// index. Wrong data reads as authored; missing data does not.
    #[test]
    fn test_an_unidentifiable_element_is_a_warned_drop_not_a_wrong_write() {
        let mut document = serde_json::json!({"edges": [
            {"from_id": "0", "to_id": "1"},
            {"from_id": "0", "to_id": "2", "zz-note": "keep me"}
        ]});
        let probe = serde_json::json!({"edges": [
            {"from_id": "0", "to_id": "1"},
            {"from_id": "0", "to_id": "2"}
        ]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["edges", 1, "zz-note"]))],
        );
        // Edge 0 deleted and edge 0→2 re-pointed: neither the
        // fingerprint nor the position identifies it.
        let mut saved = serde_json::json!({"edges": [{"from_id": "9", "to_id": "2"}]});
        test_logger::install();
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"edges": [{"from_id": "9", "to_id": "2"}]}),
            "no key may be attached to an element that cannot be identified"
        );
        assert_eq!(test_logger::lines_containing("zz-note").len(), 1);
    }

    /// Two elements that look alike are not evidence. When the
    /// fingerprint matches in more than one place the array cannot say
    /// which one the key came from, and guessing the first is how a
    /// key ends up on the wrong element.
    #[test]
    fn test_two_matching_elements_are_not_an_identification() {
        let mut document = serde_json::json!({"sections": [
            {"text": "a", "zz-twin": 1},
            {"text": "a"},
            {"text": "b"}
        ]});
        let probe = serde_json::json!({"sections": [{"text": "a"}, {"text": "a"}, {"text": "b"}]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["sections", 0, "zz-twin"]))],
        );
        // Element 0 replaced, and the `b` at the end swapped for
        // another `a`: the fingerprint now matches twice and matches
        // nothing at index 0.
        let mut saved = serde_json::json!({"sections": [{"text": "x"}, {"text": "a"}, {"text": "a"}]});
        test_logger::install();
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"sections": [{"text": "x"}, {"text": "a"}, {"text": "a"}]}),
            "an ambiguous match must place nothing"
        );
        assert_eq!(test_logger::lines_containing("zz-twin").len(), 1);
    }

    /// The index is trusted only when everything around it is
    /// untouched, which is what proves the element at it is the same
    /// element. Once a sibling changed too, position is no longer
    /// evidence of identity — an element could have been removed and
    /// another appended — so the key is dropped rather than written
    /// onto whatever is there.
    #[test]
    fn test_the_index_is_not_trusted_once_a_sibling_changed_too() {
        let mut document = serde_json::json!({"sections": [
            {"text": "a", "zz-shifted": 1},
            {"text": "b"}
        ]});
        let probe = serde_json::json!({"sections": [{"text": "a"}, {"text": "b"}]});
        let captured = take_from(
            &mut document,
            Some(&probe),
            vec![route(serde_json::json!(["sections", 0, "zz-shifted"]))],
        );
        let mut saved = serde_json::json!({"sections": [{"text": "x"}, {"text": "y"}]});
        test_logger::install();
        captured.splice_into(&mut saved);
        assert_eq!(
            saved,
            serde_json::json!({"sections": [{"text": "x"}, {"text": "y"}]}),
            "neither element is identifiable, so neither may receive the key"
        );
        assert_eq!(test_logger::lines_containing("zz-shifted").len(), 1);
    }

    /// The model has to *agree* that a level is a one-member wrapper
    /// spelled that way. When it does not, the document and the model
    /// have stopped corresponding and the walk stops — descending on
    /// the author's word alone is the guess this replaced.
    #[test]
    fn test_the_walk_stops_where_the_model_disagrees_about_the_wrapper() {
        let document = serde_json::json!({"mutator": {"Void": {"surprise": 1}}});
        let probe = serde_json::json!({"mutator": {"Macro": {"channel": 0}}});
        assert!(
            expand_route(
                &document,
                Some(&probe),
                &route(serde_json::json!(["mutator", "surprise"]))
            )
            .is_none(),
            "the model names a different variant here; the walk has no business \
             descending past a level it cannot corroborate"
        );
    }

    /// The location stamp is what the reader has to open, and each
    /// addressable part is stamped the way `maptool verify` stamps
    /// it.
    #[test]
    fn test_location_names_each_addressable_part() {
        let cases: &[(serde_json::Value, &str, &str)] = &[
            (
                serde_json::json!(["nodes", "1.2", "style", "shpe"]),
                "node \"1.2\"",
                "style.shpe",
            ),
            (
                serde_json::json!(["edges", 3, "arrowhead"]),
                "edge[3]",
                "arrowhead",
            ),
            (
                serde_json::json!(["palettes", "coral", "hue"]),
                "palette \"coral\"",
                "hue",
            ),
            (
                serde_json::json!(["custom_mutations", 0, "flavor"]),
                "custom_mutations[0]",
                "flavor",
            ),
            (serde_json::json!(["canvas", "grid_snap"]), "canvas", "grid_snap"),
            (serde_json::json!(["authors"]), "map", "authors"),
            (
                serde_json::json!(["nodes", "0", "sections", 0, "txet"]),
                "node \"0\"",
                "sections[0].txet",
            ),
        ];
        for (steps, location, within) in cases {
            let entry = plain(steps.clone(), serde_json::Value::Null);
            assert_eq!(&entry.location(), location);
            assert_eq!(&entry.path_within_location(), within);
        }
    }

    /// The load-time line must not promise what the save cannot
    /// deliver. A route the save has no way to rebuild says so at the
    /// moment the reader is looking, rather than reading as preserved
    /// and vanishing later.
    #[test]
    fn test_the_warning_stops_promising_what_cannot_be_written_back() {
        let kept = plain(serde_json::json!(["canvas", "grid_snap"]), serde_json::json!(1));
        assert!(kept.warning().contains("saved back unchanged"));
        let mut lost = kept.clone();
        lost.preservable = false;
        assert!(
            lost.warning().contains("the next save will not carry it"),
            "got: {}",
            lost.warning()
        );
        assert!(!lost.warning().contains("saved back unchanged"));
    }
}
