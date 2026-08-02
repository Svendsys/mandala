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
//! warned about once, carried on [`MindMap`](crate::mindmap::MindMap),
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
//! `#[serde(flatten)]` would absorb the key without ever reaching
//! `deserialize_ignored_any`. `mindmap::loader`'s
//! `test_no_loadable_type_can_swallow_an_unknown_key` walks the model's
//! own source and fails if either appears.

use serde::Deserialize;
use serde_json::{Map, Value};

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

/// One key the model has no field for, with the route that leads to it
/// and the value it held.
///
/// Plain data. Cost: the route's `String`s plus the captured value,
/// both moved out of the parsed document rather than cloned.
#[derive(Debug, Clone)]
pub struct UnknownKey {
    /// Full route from the document root, the last step being the key
    /// itself. Literal: every level the JSON has is a step, including
    /// the ones serde's own path elides (see [`expand_route`]).
    route: Vec<Step>,
    /// The value the key held, taken out of the parsed document.
    value: Value,
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
    /// [`MindMap::edge_locations`]: crate::mindmap::MindMap::edge_locations
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
    /// own and a test can assert the exact text without installing a
    /// logger.
    ///
    /// Cost: one `String` allocation.
    pub fn warning(&self) -> String {
        format!(
            "loader: {}: unrecognized key `{}` — this build has no field for it, so it is \
             kept as written and saved back unchanged. Check the spelling if you meant an \
             existing key; see format/schema.md.",
            self.location(),
            self.path_within_location()
        )
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
/// Lives on [`MindMap`](crate::mindmap::MindMap) so the save path can
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
    /// A key is written only where its route still resolves and the
    /// model has not since learned the name — an editor that removed
    /// the node an unknown key hung off has removed the key with it,
    /// and a build that grew a field of that name owns the value now.
    /// Anything that cannot be placed is reported at `warn!` rather
    /// than dropped in silence: it is the one case where this
    /// mechanism loses data, so it says so.
    ///
    /// **Routes below an array element are positional.** Reordering or
    /// deleting a node's sections between load and save moves the
    /// element a captured key was attached to; the key lands on
    /// whatever now sits at that index. Above the array — nodes by id,
    /// palettes by name — the route is stable.
    ///
    /// Cost: O(captured keys × route length), and one `Value` clone
    /// per captured key. Nothing at all when the capture is empty,
    /// which is every map this build wrote.
    pub fn splice_into(&self, document: &mut Value) {
        for entry in &self.entries {
            let Some((Step::Key(key), parent)) = entry.route.split_last() else {
                continue;
            };
            let placed = owner_of_mut(document, parent).is_some_and(|owner| {
                if owner.contains_key(key) {
                    return false;
                }
                owner.insert(key.clone(), entry.value.clone());
                true
            });
            if !placed {
                log::warn!(
                    "loader: {}: unrecognized key `{}` could not be written back — the \
                     place it was loaded from is no longer in the document. It is dropped \
                     from the saved file.",
                    entry.location(),
                    entry.path_within_location()
                );
            }
        }
    }
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
    // brace parses clean.
    deserializer.end()?;
    Ok((value, routes))
}

/// Lift routes collected by [`deserialize_capturing`] into
/// [`UnknownKeys`], moving each key's value out of `document`.
///
/// `document` is the same JSON parsed as a `serde_json::Value`. The
/// values are **taken**, not copied, so the captured keys are gone
/// from `document` afterwards — the caller is expected to be done with
/// it.
///
/// A route that does not resolve is skipped. That is not reachable
/// from a route this crate produced; it is what keeps a caller
/// pairing a route list with the wrong document out of a panic.
///
/// Cost: O(routes × route length); no allocation beyond the captured
/// values themselves.
pub fn take_from(document: &mut Value, routes: Vec<Vec<Step>>) -> UnknownKeys {
    let mut entries = Vec::with_capacity(routes.len());
    for route in routes {
        let Some(route) = expand_route(document, &route) else {
            continue;
        };
        let Some((Step::Key(key), parent)) = route.split_last() else {
            continue;
        };
        let Some(value) = owner_of_mut(document, parent).and_then(|owner| owner.remove(key)) else {
            continue;
        };
        entries.push(UnknownKey { route, value });
    }
    UnknownKeys { entries }
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

/// Rewrite a route from serde's ignored-key path into one that names
/// every JSON level literally, resolved against the document the keys
/// were read from. `None` when the route does not lead anywhere,
/// which cannot happen for a route this crate produced.
///
/// **What has to be put back: externally tagged enum levels.**
/// `MutatorNode::Void` is written `{"Void": { … }}`, and the variant
/// name never travels through a `MapAccess`, so serde's path has one
/// fewer step than the JSON has levels. When the object in hand
/// cannot answer the next step and holds exactly one member, that
/// member is the variant payload; the walk descends and records the
/// variant name as a step of its own. The test is exact rather than a
/// guess for two reasons: every enum reachable from a load is
/// externally tagged (the crate has no `#[serde(tag)]` and no
/// `#[serde(untagged)]`), and the key is known to be *somewhere*
/// below — serde would not have reported it otherwise.
///
/// Doing the expansion once, here, is what lets
/// [`UnknownKeys::splice_into`] walk a saved document with no
/// heuristic at all: the stored route is a literal path, and a
/// one-member object on the way back is just a one-member object.
///
/// Recursive, bounded by `route.len()`, which is bounded by the parse
/// that produced it — `serde_json` caps recursion at 128 levels.
///
/// Cost: O(route length), plus one `String` clone per step of the
/// rewritten route.
fn expand_route(document: &Value, route: &[Step]) -> Option<Vec<Step>> {
    fn walk<'v>(current: &'v Value, route: &[Step], out: &mut Vec<Step>) -> Option<&'v Value> {
        let Some((step, rest)) = route.split_first() else {
            return Some(current);
        };
        let mut node = current;
        while is_tag_wrapper(node, step) {
            let (variant, payload) = node.as_object()?.iter().next()?;
            out.push(Step::Key(variant.clone()));
            node = payload;
        }
        let next = match (node, step) {
            (Value::Object(members), Step::Key(key)) => members.get(key)?,
            (Value::Array(items), Step::Index(index)) => items.get(*index)?,
            _ => return None,
        };
        out.push(step.clone());
        walk(next, rest, out)
    }
    let mut out = Vec::with_capacity(route.len());
    walk(document, route, &mut out).map(|_| out)
}

/// Whether `value` is the one-member object an externally tagged enum
/// variant is wrapped in, rather than the thing `step` addresses.
/// Only sound while the addressed key is known to exist below —
/// see [`expand_route`].
fn is_tag_wrapper(value: &Value, step: &Step) -> bool {
    match (value, step) {
        (Value::Object(members), Step::Key(key)) => members.len() == 1 && !members.contains_key(key),
        (Value::Object(members), Step::Index(_)) => members.len() == 1,
        _ => false,
    }
}

/// Follow a literal route (as [`expand_route`] produced) and hand back
/// the object at the end of it.
///
/// No wrapper skipping and no guessing: each step must name a member
/// or an index that is really there. That strictness is the point on
/// the save path — a route that no longer resolves means the document
/// changed shape, and the caller reports that rather than placing the
/// key somewhere plausible.
///
/// Recursive, bounded by `route.len()`. Cost: O(route length).
fn owner_of_mut<'v>(document: &'v mut Value, route: &[Step]) -> Option<&'v mut Map<String, Value>> {
    let Some((step, rest)) = route.split_first() else {
        return document.as_object_mut();
    };
    let next = match (document, step) {
        (Value::Object(members), Step::Key(key)) => members.get_mut(key)?,
        (Value::Array(items), Step::Index(index)) => items.get_mut(*index)?,
        _ => return None,
    };
    owner_of_mut(next, rest)
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// A route through an externally tagged enum has one fewer step
    /// than the JSON has levels — `{"Void": {…}}` costs a level that
    /// never travels through a `MapAccess`. The expansion has to put
    /// it back, or every key captured inside a mutator lands nowhere.
    #[test]
    fn test_expand_route_restores_an_external_enum_tag() {
        let document = serde_json::json!({
            "custom_mutations": [{"mutator": {"Void": {"channel": 0, "surprise": 7}}}]
        });
        let expanded = expand_route(
            &document,
            &route(serde_json::json!(["custom_mutations", 0, "mutator", "surprise"])),
        )
        .expect("the enum payload must be reachable");
        assert_eq!(
            expanded,
            route(serde_json::json!(["custom_mutations", 0, "mutator", "Void", "surprise"]))
        );
    }

    /// The tag skip must not fire on a one-member object that *is*
    /// the addressed thing — a node whose only authored key is the
    /// one being captured.
    #[test]
    fn test_expand_route_prefers_a_real_member_over_a_tag_skip() {
        let document = serde_json::json!({"nodes": {"0": {"mystery": 1}}});
        let steps = route(serde_json::json!(["nodes", "0", "mystery"]));
        assert_eq!(expand_route(&document, &steps), Some(steps.clone()));
    }

    /// A route that leads nowhere resolves to nothing rather than to
    /// something near it.
    #[test]
    fn test_expand_route_returns_none_for_a_route_that_is_gone() {
        let document = serde_json::json!({"nodes": {"0": {}}});
        assert!(expand_route(&document, &route(serde_json::json!(["nodes", "9", "x"]))).is_none());
    }

    /// The expanded route is literal, so the save-side walk needs no
    /// heuristic — and must not apply one. A one-member object on the
    /// way back is just a one-member object.
    #[test]
    fn test_owner_of_mut_takes_a_route_literally() {
        let mut document = serde_json::json!({"canvas": {"background_color": "#000"}});
        let owner = owner_of_mut(&mut document, &route(serde_json::json!(["canvas"])))
            .expect("the canvas must be reachable");
        owner.insert("grid_snap".to_string(), serde_json::json!(8));
        assert_eq!(document["canvas"]["grid_snap"], serde_json::json!(8));
    }

    /// Capture takes the key out of the document it was read from, so
    /// nothing downstream sees a half-owned value.
    #[test]
    fn test_take_from_moves_the_value_out_of_the_document() {
        let mut document = serde_json::json!({"nodes": {"0": {"mystery": [1, 2]}}});
        let captured = take_from(&mut document, vec![route(serde_json::json!(["nodes", "0", "mystery"]))]);
        assert_eq!(captured.len(), 1);
        assert_eq!(captured.iter().next().expect("one entry").value(), &serde_json::json!([1, 2]));
        assert_eq!(document, serde_json::json!({"nodes": {"0": {}}}));
    }

    /// The location stamp is what the reader has to open, and each
    /// addressable part is stamped the way `maptool verify` stamps
    /// it.
    #[test]
    fn test_location_names_each_addressable_part() {
        let cases: &[(serde_json::Value, &str, &str)] = &[
            (serde_json::json!(["nodes", "1.2", "style", "shpe"]), "node \"1.2\"", "style.shpe"),
            (serde_json::json!(["edges", 3, "arrowhead"]), "edge[3]", "arrowhead"),
            (serde_json::json!(["palettes", "coral", "hue"]), "palette \"coral\"", "hue"),
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
            let entry = UnknownKey {
                route: route(steps.clone()),
                value: serde_json::Value::Null,
            };
            assert_eq!(&entry.location(), location);
            assert_eq!(&entry.path_within_location(), within);
        }
    }

    /// A key whose place in the document is gone is dropped, and says
    /// so. Silence here would be the one hole in the preservation
    /// guarantee, so it is a `warn!` and a test that the surviving
    /// document is not corrupted around it.
    #[test]
    fn test_splice_skips_a_route_the_document_no_longer_has() {
        let keys = UnknownKeys {
            entries: vec![UnknownKey {
                route: route(serde_json::json!(["nodes", "gone", "x"])),
                value: serde_json::json!(1),
            }],
        };
        let mut document = serde_json::json!({"nodes": {"0": {}}});
        keys.splice_into(&mut document);
        assert_eq!(document, serde_json::json!({"nodes": {"0": {}}}));
    }

    /// A build that grew a field of the captured name owns that name
    /// now; the stale capture must not overwrite what the model just
    /// wrote.
    #[test]
    fn test_splice_does_not_overwrite_a_key_the_model_now_writes() {
        let keys = UnknownKeys {
            entries: vec![UnknownKey {
                route: route(serde_json::json!(["canvas", "grid_snap"])),
                value: serde_json::json!(8),
            }],
        };
        let mut document = serde_json::json!({"canvas": {"grid_snap": 16}});
        keys.splice_into(&mut document);
        assert_eq!(document["canvas"]["grid_snap"], serde_json::json!(16));
    }
}
