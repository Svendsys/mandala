// SPDX-License-Identifier: MPL-2.0

//! Reachability walk over the crate's own source, so a test can ask
//! "which types can this deserializer actually be handed?" and get an
//! answer that stays true as the model grows.
//!
//! The problem this exists to solve: a `.mindmap.json` key this build
//! has no field for is captured at load and written back at save
//! (`mindmap::unknown_keys`), and that only works while no type
//! between the document root and the key can absorb it first.
//! `#[serde(deny_unknown_fields)]` aborts the load; `#[serde(flatten)]`
//! swallows the key before `deserialize_ignored_any` is ever reached.
//! Neither produces a compile error, and the two even combine — with
//! `deny` silently winning and the flattened map left empty. A
//! hand-maintained list of "types that must not do that" is exactly
//! the sort of twin surface that drifts the moment somebody adds a
//! field — the same failure mode `lib/baumhard/CONVENTIONS.md` §B4
//! records for the field-tag enums.
//!
//! So nothing is listed. [`TypeGraph::build`] parses every non-test
//! `.rs` file under baumhard's `src/` with `syn`, indexes every
//! struct / enum / type alias, and [`TypeGraph::reachable_from`]
//! walks the field graph outward from a root type name. Adding a
//! field of a new type extends the covered set automatically; the
//! test that consumes this fails until the new type opts in.
//!
//! **`src/` is the whole of what it can see**, and that is a real
//! limit rather than a phrasing detail: a type emitted by
//! `lib/baumhard/build.rs` into `$OUT_DIR` is reachable from a load
//! and invisible here. [`TypeGraph::unresolved_from`] reports every
//! name the walk gave up on so a test can hold that gap to an
//! explicit list instead of letting it grow in silence.
//!
//! Test-only and native-only: it reads `.rs` files off the
//! filesystem, which is also why it is `#[cfg(test)]` — `syn` is a
//! dev-dependency and nothing in a shipped build should carry a
//! parser for its own source.

use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use syn::{Attribute, Fields, GenericArgument, Item, PathArguments, Type};

/// Whether a walk follows `#[serde(into = "...")]` proxies. Read
/// questions must not (a serialize-only proxy is never handed to a
/// deserializer); write questions must.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FollowSerializeProxy {
    /// Follow `into` proxies — the walk is about the save path.
    Yes,
    /// Ignore them — the walk is about the load path.
    No,
}

/// Whether an indexed item is a struct, an enum, or a type alias.
/// Aliases carry no attributes of their own; they exist in the index
/// so `type DeltaGlyphArea = Delta<GlyphAreaField>` forwards the walk
/// to both `Delta` and `GlyphAreaField`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TypeKind {
    /// A `struct` item.
    Struct,
    /// An `enum` item.
    Enum,
    /// A `type X = ...;` alias — a pass-through node in the walk.
    Alias,
}

/// One indexed type: where it lives, what serde does with it, and
/// which other type names its fields mention.
#[derive(Debug, Clone)]
pub struct TypeInfo {
    /// The item's identifier, as written.
    pub name: String,
    /// Repo-relative-ish path of the file that defines it, for error
    /// messages that a reader can act on.
    pub file: PathBuf,
    /// Struct, enum, or alias.
    pub kind: TypeKind,
    /// `true` when the item's derive list includes `Deserialize`.
    pub derives_deserialize: bool,
    /// `true` when the item carries `#[serde(deny_unknown_fields)]` —
    /// which turns an unrecognized key into a load failure instead of
    /// letting the loader capture and preserve it.
    pub denies_unknown_fields: bool,
    /// Every field marked `#[serde(flatten)]`, by name. A flattened
    /// field consumes the members no declared field matched, so the
    /// keys never reach `deserialize_ignored_any` and the loader's
    /// capture never sees them. Collected for enums too, across all
    /// variants.
    pub flattened_fields: Vec<String>,
    /// The proxy named by `#[serde(from = "...")]` /
    /// `#[serde(try_from = "...")]`, if any. Such a type never
    /// deserializes its own shape — the proxy does — so the
    /// requirement moves to the proxy.
    pub deserialize_proxy: Option<String>,
    /// The proxy named by `#[serde(into = "...")]`, if any. Such a
    /// type never serializes its own shape — it converts first — so
    /// the *write* path's contract lives on the proxy. Followed only
    /// by [`TypeGraph::omit_predicates_from`], never by
    /// [`TypeGraph::reachable_from`]: a serialize-only proxy is never
    /// handed to a deserializer and has no business in the load-path
    /// coverage set.
    pub serialize_proxy: Option<String>,
    /// `true` when the item carries `#[serde(untagged)]`. An untagged
    /// enum decides its variant by trial deserialization, which means
    /// serde buffers the object into a `Content` value and replays it
    /// through a deserializer of its own — one the loader's capture
    /// wrapper is not part of. Keys a variant does not claim are
    /// dropped inside that replay.
    pub untagged: bool,
    /// `true` when the item carries `#[serde(tag = "…")]`, with or
    /// without a `content`. Internally and adjacently tagged enums
    /// buffer through `Content` for the same reason an untagged one
    /// does, and hide unrecognized keys the same way.
    pub internally_tagged: bool,
    /// `true` when some part of the item can consume named JSON
    /// object keys: a struct with named fields, or an enum with at
    /// least one struct variant. Tuple and unit shapes have no field
    /// names for a stray key to hide in.
    pub has_named_fields: bool,
    /// Every variant name an enum declares, in source order; empty
    /// for a struct or an alias.
    ///
    /// An externally tagged enum writes its variant as the single
    /// member of a wrapper object, which is a JSON level serde's
    /// ignored-key path does not report — so
    /// `mindmap::unknown_keys::expand_route` has to put it back. The
    /// names are here so a test can drive that over **every** variant
    /// the model can meet rather than the two somebody thought of.
    pub variants: Vec<String>,
    /// Every type name mentioned by a deserializable field, in
    /// source order and de-duplicated by the walk rather than here.
    pub referenced: Vec<String>,
    /// Every deserializable field whose type mentions `Vec`, as
    /// `(JSON member name, the type names the field's type mentions)`.
    ///
    /// An array is the one container a captured key's route crosses
    /// **positionally**: above one, a route names a node by its id or
    /// a palette by its name and is stable across any edit; below
    /// one, it is an index, and an edit to the array can slide it
    /// onto a different element. `format/schema.md` publishes the set
    /// of places that happens, and the resolution of which element
    /// types can actually hold a key needs the whole index — so what
    /// is recorded here is the raw pair and
    /// [`TypeGraph::key_bearing_sequences_from`] answers the
    /// question.
    pub sequence_fields: Vec<(String, Vec<String>)>,
    /// Every predicate named by a field's
    /// `#[serde(skip_serializing_if = "...")]`, in source order.
    ///
    /// These are the only sanctioned reason a key present in an
    /// authored file may be absent from the saved one, so a
    /// round-trip test needs the set to tell "the saver dropped a
    /// key" from "the key held its own default and serde left it
    /// out". Collected from *every* field, including ones
    /// `#[serde(skip)]` keeps out of [`Self::referenced`] — this is a
    /// question about the write path, not the read path.
    pub omit_predicates: Vec<String>,
    /// Every `#[serde(...)]` attribute on this item or one of its
    /// fields that the reader in this module could **not** consume to
    /// the end, rendered as written and paired with the parse error.
    ///
    /// This is the field that keeps every other one honest. The
    /// readers below discard their parse errors on purpose, so an
    /// attribute they only half-read is indistinguishable from one
    /// that carried nothing — and since the answer they produce is
    /// "this type does not hide keys", a half-read attribute is a
    /// *silent pass*. A caller that trusts [`Self::untagged`] or
    /// [`Self::denies_unknown_fields`] must check this first, or it is
    /// trusting a flag that may simply never have been looked at. See
    /// [`unread_serde_attrs`] for how it is produced and
    /// [`TypeGraph::unread_serde_attrs`] for the crate-wide view.
    pub unread_serde: Vec<String>,
    /// `true` when the item carries a container-level
    /// `#[serde(...)]`, whatever its derive list looks like.
    ///
    /// The second half of "does this type take part in
    /// deserialization?", and the half [`Self::derives_deserialize`]
    /// cannot answer: that one matches the identifier `Deserialize`
    /// in a `derive` list, so `use serde::Deserialize as De;` +
    /// `#[derive(De)]` answers `false` and takes the type out of
    /// [`Self::unknown_key_hiding_reasons`] altogether — a
    /// `#[serde(untagged)]` sitting right beside it and never looked
    /// at. See [`has_serde_container_attr`].
    pub has_serde_container_attr: bool,
}

impl TypeInfo {
    /// Why an unrecognized `.mindmap.json` key would not survive a
    /// load through this type, or empty when it would.
    ///
    /// The predicate behind
    /// `mindmap::loader::tests::test_no_loadable_type_can_swallow_an_unknown_key`,
    /// which reads the policy narrative; this is the mechanism. It
    /// lives here rather than inline in that test for two reasons:
    /// the question is entirely about serde attributes, which is what
    /// this module models, and a predicate whose *false* answer is a
    /// green suite has to be drivable over inputs that do not exist
    /// in the crate yet — see
    /// `tests::test_a_type_whose_serde_attributes_were_not_read_is_never_called_safe`.
    ///
    /// Cost: O(flattened fields + unread attributes); allocates one
    /// `String` per reason and nothing when there are none.
    pub fn unknown_key_hiding_reasons(&self) -> Vec<String> {
        // **Unknown beats known.** Both exemptions below read a flag
        // off the source, and a flag is only worth as much as the
        // reading that produced it. If any attribute on this type
        // went unread, neither exemption is trustworthy — so they are
        // skipped and the unread entries are reported instead.
        if self.unread_serde.is_empty() {
            // A type serde never deserializes cannot hide a key from
            // a load. "Never deserializes" has to mean more than "no
            // `Deserialize` in its derive list", because
            // [`derives_deserialize`] matches an identifier: an
            // aliased import or a wrapped derive answers `false`, and
            // `false` here does not mis-flag — it drops the type out
            // of the check entirely. A container-level `serde`
            // attribute is independent evidence that it takes part.
            if !self.derives_deserialize && !self.has_serde_container_attr {
                return Vec::new();
            }
            // A type that delegates its shape through
            // `#[serde(from = "...")]` never reads its own fields —
            // the requirement follows the proxy, which the walk also
            // reaches.
            if self.deserialize_proxy.is_some() {
                return Vec::new();
            }
        }
        let mut reasons = Vec::new();
        // `deny_unknown_fields` and `flatten` act on *named members*:
        // they decide what happens to a key no declared field
        // claimed. A type with no named fields anywhere — an enum
        // whose variants are all newtype, tuple, or unit — has no
        // such key to decide about, and serde rejects both attributes
        // there anyway.
        if self.has_named_fields {
            if self.denies_unknown_fields {
                reasons.push("`deny_unknown_fields` (fails the load)".to_string());
            }
            for field in &self.flattened_fields {
                reasons.push(format!("`flatten` on `{field}` (absorbs the key)"));
            }
        }
        // `untagged` and `tag = "…"` act on the *whole value*: serde
        // buffers the object into its own `Content` and replays it
        // through a deserializer the capture is not wrapping. Variant
        // shape has nothing to do with it — a newtype-only enum
        // buffers exactly the same way — so these two are asked of
        // every reachable type. The `has_named_fields` guard above
        // used to sit in front of them, which left 24 of the 70
        // reachable types unchecked and let a planted
        // `#[serde(untagged)]` on `Mutation` through.
        if self.untagged {
            reasons.push("`untagged` (replays through a buffered Content)".to_string());
        }
        if self.internally_tagged {
            reasons.push("`tag = \"...\"` (replays through a buffered Content)".to_string());
        }
        // The four flags above are read off the source by a
        // hand-rolled parser, and the caller's assertion is that this
        // list is *empty* — so an attribute the parser could not
        // finish reads as "carries none of them", which is a pass.
        // A type whose attributes were not fully read is therefore an
        // offender in its own right: what it does is unknown, and
        // unknown is not safe.
        for attribute in &self.unread_serde {
            reasons.push(format!("{attribute} — unread, so its flags were never seen"));
        }
        reasons
    }
}

/// Index of every struct, enum, and type alias defined under one
/// source root.
///
/// Cost: one full read and `syn` parse of every non-test `.rs` file
/// under the root — tens of milliseconds for this crate, paid once
/// per test that builds a graph.
#[derive(Debug, Clone, Default)]
pub struct TypeGraph {
    items: BTreeMap<String, TypeInfo>,
    duplicates: BTreeSet<String>,
}

impl TypeGraph {
    /// Parse every `.rs` file under `src_root` and index the items it
    /// declares, descending into inline `mod { … }` blocks.
    ///
    /// Test sources are skipped — a `struct StubCtx` inside a test
    /// module is not part of any on-disk contract, and letting one
    /// shadow a real model type would silently move the walk. Files
    /// named `tests.rs` / `*_tests.rs` / `*_test.rs`, directories
    /// named `tests`, and `#[cfg(test)]` modules are all excluded.
    ///
    /// Panics if a file cannot be read or parsed: this backs a test
    /// whose entire job is to notice that the source moved, so a
    /// silent skip would defeat it.
    pub fn build(src_root: &Path) -> Self {
        let mut graph = TypeGraph::default();
        let mut files = Vec::new();
        collect_source_files(src_root, &mut files);
        files.sort();
        for file in files {
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
            let parsed = syn::parse_file(&text)
                .unwrap_or_else(|e| panic!("{} must parse as Rust: {e}", file.display()));
            graph.index_items(&parsed.items, &file);
        }
        graph
    }

    /// The indexed entry for `name`, or `None` when the crate does
    /// not declare a type by that name.
    pub fn get(&self, name: &str) -> Option<&TypeInfo> {
        self.items.get(name)
    }

    /// Every `#[serde(...)]` attribute anywhere in the indexed
    /// sources that this module's reader could not consume to the
    /// end, as `"<type> (<file>): <attribute>: <error>"`.
    ///
    /// The walk's readers are silent about what defeats them, and
    /// silence reads as "carries nothing" — which is precisely the
    /// answer that makes every guard built on this graph pass. This
    /// reports the blind spot instead, over **all** indexed types
    /// rather than only the ones reachable from `MindMap`: a reader
    /// gap is a property of the parser, and it should go red the run
    /// it appears rather than the later run where a type that has it
    /// happens to join the load graph.
    ///
    /// Cost: one pass over the index; no I/O.
    pub fn unread_serde_attrs(&self) -> Vec<String> {
        self.items
            .values()
            .flat_map(|info| {
                info.unread_serde
                    .iter()
                    .map(move |attr| format!("{} ({}): {attr}", info.name, info.file.display()))
            })
            .collect()
    }

    /// Type names declared more than once across the indexed sources.
    /// The walk resolves by bare name, so a collision would make its
    /// answer ambiguous; callers assert this is empty rather than
    /// silently trusting whichever definition landed last.
    pub fn duplicate_names(&self) -> &BTreeSet<String> {
        &self.duplicates
    }

    /// Every distinct `#[serde(skip_serializing_if = "...")]`
    /// predicate named by a type reachable from `root`.
    ///
    /// This is the complete set of reasons the saver is allowed to
    /// write out fewer keys than it read in. A round-trip test pins
    /// it against a hand-modeled allowlist, so a new predicate cannot
    /// quietly widen what "no authored key is lost" tolerates.
    ///
    /// Walks the **write** graph, which is not the read graph:
    /// `#[serde(into = "...")]` proxies are followed here and nowhere
    /// else. `CustomMutation` is the live case — its keys are written
    /// by `CustomMutationOut`, and one of that type's fields is the
    /// only `String::is_empty` omission a saved map can produce.
    ///
    /// Cost: one BFS over the reached types; no I/O.
    pub fn omit_predicates_from(&self, root: &str) -> BTreeSet<String> {
        self.walk(root, FollowSerializeProxy::Yes, &[])
            .iter()
            .flat_map(|info| info.omit_predicates.iter().cloned())
            .collect()
    }

    /// Every JSON member name reachable from `root` that holds an
    /// array whose **elements can carry named keys** — the complete
    /// set of places a captured key's route is positional and an edit
    /// can therefore move it.
    ///
    /// `format/schema.md` §"Unknown keys are kept" publishes this set
    /// for the reader, and
    /// `loader::tests::test_the_published_positional_arrays_are_the_ones_the_model_has`
    /// holds the two together in both directions. It was a
    /// hand-written list before that, and it had already drifted —
    /// `control_points` and a palette's `groups` were missing — which
    /// is the twin-surface shape `lib/baumhard/CONVENTIONS.md` §B4 is
    /// about.
    ///
    /// "Can carry named keys" is the discriminator, not "is an
    /// array": `macros` and `inline_macros` are `Vec<Value>`, held
    /// deliberately opaque, so nothing inside one is ever captured
    /// and no route ever crosses their indexes.
    ///
    /// Cost: one walk from `root` plus one index lookup per element
    /// type named; no I/O.
    pub fn key_bearing_sequences_from(&self, root: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for info in self.reachable_from(root) {
            for (member, mentioned) in &info.sequence_fields {
                if mentioned.iter().any(|name| self.carries_named_keys(name)) {
                    out.insert(member.clone());
                }
            }
        }
        out
    }

    /// Whether an unrecognized key can be captured *at or below* a
    /// value of the type called `name` — the precondition for the
    /// loader's capture to ever produce a route through it.
    ///
    /// Asked of the whole subgraph rather than of the type itself,
    /// because an externally tagged enum with only newtype variants
    /// has no named fields of its own and still carries a payload
    /// that does: `{"Void": {"channel": 0, "zzz": 1}}` is a key
    /// captured below a variant. Proxies come along for free — the
    /// walk follows `#[serde(from = "…")]`, which is the shape the
    /// file actually carries.
    ///
    /// `Vec<String>` and `Vec<Value>` come out `false`: no member of
    /// theirs is ever "unrecognized", so no route crosses their
    /// indexes.
    fn carries_named_keys(&self, name: &str) -> bool {
        self.items.contains_key(name)
            && self
                .reachable_from(name)
                .iter()
                .any(|info| info.derives_deserialize && info.has_named_fields)
    }

    /// Every name a reachable type mentions that this index does not
    /// resolve — the walk's **terminators**.
    ///
    /// [`reachable_from`](Self::reachable_from) drops an unresolved
    /// name on the floor, which is right for `String` and `f64` and
    /// wrong for a type that really is part of the on-disk contract
    /// but was not indexed: anything `build.rs` emits into `$OUT_DIR`
    /// (`AppFont`), a derive-generated tag (strum's
    /// `*Discriminant`), a type from another crate. The walk cannot
    /// tell those two cases apart — both are simply "a name I do not
    /// have" — so it reports them and
    /// `tests::test_every_walk_terminator_is_an_expected_one` decides,
    /// against [`EXPECTED_TERMINATORS`]. A newly unresolved name goes
    /// red instead of silently shrinking the covered set.
    ///
    /// Cost: same walk as `reachable_from` plus one set insert per
    /// referenced name; no I/O.
    pub fn unresolved_from(&self, root: &str) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for info in self.reachable_from(root) {
            let mentioned = info.referenced.iter().chain(info.deserialize_proxy.as_ref());
            for name in mentioned {
                if !self.items.contains_key(name.as_str()) {
                    out.insert(name.clone());
                }
            }
        }
        out
    }

    /// Every indexed type reachable from `root` by following
    /// deserializable fields, including `root` itself.
    ///
    /// Names that are not indexed (`String`, `f64`,
    /// `serde_json::Value`, generic parameters) terminate the walk —
    /// they carry no fields of ours. Aliases forward to the types
    /// their right-hand side mentions.
    ///
    /// Cost: O(reached types × their field count); no I/O.
    pub fn reachable_from(&self, root: &str) -> Vec<&TypeInfo> {
        self.walk(root, FollowSerializeProxy::No, &[])
    }

    /// Every indexed type reachable from `root` **without passing
    /// through** any type named in `blocked` — and excluding the
    /// blocked types themselves.
    ///
    /// The difference from [`reachable_from`](Self::reachable_from)
    /// is the difference between "can be reached from X" and "can
    /// *only* be reached through X", and only the second answers a
    /// question of the form "is everything of this kind inside
    /// something the loader can skip?". A type used in both a
    /// skippable and an unskippable position satisfies the first and
    /// fails the second, which is the case worth catching: the
    /// skippable use is not what makes the unskippable one safe.
    ///
    /// Cost: same walk as `reachable_from`; no I/O.
    pub fn reachable_from_excluding(&self, root: &str, blocked: &[&str]) -> Vec<&TypeInfo> {
        self.walk(root, FollowSerializeProxy::No, blocked)
    }

    /// The BFS behind [`reachable_from`],
    /// [`reachable_from_excluding`](Self::reachable_from_excluding)
    /// and [`omit_predicates_from`]. Field references and
    /// `#[serde(from)]` proxies are always followed;
    /// `#[serde(into)]` proxies only when the caller is asking a
    /// question about the write path. A name in `blocked` is never
    /// entered, so nothing reachable only through it is reported.
    fn walk(&self, root: &str, serialize: FollowSerializeProxy, blocked: &[&str]) -> Vec<&TypeInfo> {
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        let mut out: Vec<&TypeInfo> = Vec::new();

        if blocked.contains(&root) {
            return out;
        }
        queue.push_back(root);
        seen.insert(root);
        while let Some(name) = queue.pop_front() {
            let Some(info) = self.items.get(name) else { continue };
            out.push(info);
            let write_proxy = match serialize {
                FollowSerializeProxy::Yes => info.serialize_proxy.as_deref(),
                FollowSerializeProxy::No => None,
            };
            let successors = info
                .referenced
                .iter()
                .map(String::as_str)
                .chain(info.deserialize_proxy.as_deref())
                .chain(write_proxy);
            for next in successors {
                if blocked.contains(&next) {
                    continue;
                }
                if let Some(entry) = self.items.get_key_value(next) {
                    if seen.insert(entry.0.as_str()) {
                        queue.push_back(entry.0.as_str());
                    }
                }
            }
        }
        out
    }

    fn index_items(&mut self, items: &[Item], file: &Path) {
        for item in items {
            match item {
                Item::Struct(item) => {
                    let serde = SerdeAttrs::read(&item.attrs);
                    self.insert(TypeInfo {
                        name: item.ident.to_string(),
                        file: file.to_path_buf(),
                        kind: TypeKind::Struct,
                        derives_deserialize: derives_deserialize(&item.attrs),
                        denies_unknown_fields: serde.deny_unknown_fields,
                        deserialize_proxy: serde.proxy,
                        serialize_proxy: serde.into_proxy,
                        untagged: serde.untagged,
                        internally_tagged: serde.internally_tagged,
                        flattened_fields: flattened_fields(&item.fields),
                        has_named_fields: matches!(item.fields, Fields::Named(_)),
                        variants: Vec::new(),
                        referenced: referenced_types(&item.fields),
                        omit_predicates: omit_predicates(&item.fields),
                        sequence_fields: sequence_fields(&item.fields),
                        unread_serde: unread_serde_attrs_with_fields(&item.attrs, &item.fields),
                        has_serde_container_attr: has_serde_container_attr(&item.attrs),
                    });
                }
                Item::Enum(item) => {
                    let serde = SerdeAttrs::read(&item.attrs);
                    let mut referenced = Vec::new();
                    let mut predicates = Vec::new();
                    let mut flattened = Vec::new();
                    let mut has_named_fields = false;
                    let mut variants = Vec::new();
                    let mut sequences = Vec::new();
                    let mut unread = unread_serde_attrs(&item.attrs);
                    for variant in &item.variants {
                        variants.push(variant.ident.to_string());
                        has_named_fields |= matches!(variant.fields, Fields::Named(_));
                        referenced.extend(referenced_types(&variant.fields));
                        predicates.extend(omit_predicates(&variant.fields));
                        sequences.extend(sequence_fields(&variant.fields));
                        flattened.extend(flattened_fields(&variant.fields));
                        unread.extend(unread_serde_attrs_with_fields(&variant.attrs, &variant.fields));
                    }
                    self.insert(TypeInfo {
                        name: item.ident.to_string(),
                        file: file.to_path_buf(),
                        kind: TypeKind::Enum,
                        derives_deserialize: derives_deserialize(&item.attrs),
                        denies_unknown_fields: serde.deny_unknown_fields,
                        deserialize_proxy: serde.proxy,
                        serialize_proxy: serde.into_proxy,
                        untagged: serde.untagged,
                        internally_tagged: serde.internally_tagged,
                        flattened_fields: flattened,
                        has_named_fields,
                        variants,
                        referenced,
                        omit_predicates: predicates,
                        sequence_fields: sequences,
                        unread_serde: unread,
                        has_serde_container_attr: has_serde_container_attr(&item.attrs),
                    });
                }
                Item::Type(item) => {
                    let mut referenced = Vec::new();
                    collect_type_names(&item.ty, &mut referenced);
                    self.insert(TypeInfo {
                        name: item.ident.to_string(),
                        file: file.to_path_buf(),
                        kind: TypeKind::Alias,
                        derives_deserialize: false,
                        denies_unknown_fields: false,
                        deserialize_proxy: None,
                        serialize_proxy: None,
                        untagged: false,
                        internally_tagged: false,
                        flattened_fields: Vec::new(),
                        has_named_fields: false,
                        variants: Vec::new(),
                        referenced,
                        omit_predicates: Vec::new(),
                        sequence_fields: Vec::new(),
                        unread_serde: unread_serde_attrs(&item.attrs),
                        has_serde_container_attr: has_serde_container_attr(&item.attrs),
                    });
                }
                Item::Mod(item) => {
                    if is_test_gated(&item.attrs) {
                        continue;
                    }
                    if let Some((_, inner)) = &item.content {
                        self.index_items(inner, file);
                    }
                }
                _ => {}
            }
        }
    }

    fn insert(&mut self, info: TypeInfo) {
        if self.items.contains_key(&info.name) {
            self.duplicates.insert(info.name.clone());
            return;
        }
        self.items.insert(info.name.clone(), info);
    }
}

/// Recursively gather `.rs` files under `dir`, skipping test sources.
fn collect_source_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries =
        std::fs::read_dir(dir).unwrap_or_else(|e| panic!("{} must be readable: {e}", dir.display()));
    for entry in entries {
        let path = entry.expect("directory entry").path();
        let name = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        if path.is_dir() {
            if name != "tests" {
                collect_source_files(&path, out);
            }
        } else if name.ends_with(".rs") && !is_test_file_name(&name) {
            out.push(path);
        }
    }
}

fn is_test_file_name(name: &str) -> bool {
    name == "tests.rs" || name.ends_with("_tests.rs") || name.ends_with("_test.rs")
}

/// Whether `attrs` compile the item only under `cfg(test)`.
///
/// `all(...)` is descended into and `any(...)` / `not(...)`
/// deliberately are not: `#[cfg(all(test, not(target_arch =
/// "wasm32")))]` — the spelling this crate actually uses for its
/// native-only test modules — gates on `test`, while
/// `#[cfg(any(test, feature = "x"))]` does not and
/// `#[cfg(not(test))]` gates on its absence. Reading the bare
/// `test` ident alone missed the `all(...)` form, which is the
/// same list-form blind spot [`skip_unmodeled_option`] closes on
/// the serde side, with the same consequence: a test-only
/// declaration was indexed as though it were part of the on-disk
/// model.
fn is_test_gated(attrs: &[Attribute]) -> bool {
    attrs
        .iter()
        .any(|attr| attr.path().is_ident("cfg") && cfg_predicate_requires_test(attr))
}

/// `true` when the `cfg` predicate written in `attr`, or in an
/// `all(...)` nested to any depth inside it, names the bare `test`
/// flag.
fn cfg_predicate_requires_test(attr: &Attribute) -> bool {
    let mut gated = false;
    let _ = attr.parse_nested_meta(|meta| cfg_arm_requires_test(&meta, &mut gated));
    gated
}

/// One arm of a `cfg` predicate. `all(...)` conjoins, so descending
/// into it preserves the meaning; every other shape is consumed
/// whole so the arms after it are still read.
fn cfg_arm_requires_test(meta: &syn::meta::ParseNestedMeta, gated: &mut bool) -> syn::Result<()> {
    if meta.path.is_ident("test") {
        *gated = true;
    } else if meta.path.is_ident("all") {
        meta.parse_nested_meta(|nested| cfg_arm_requires_test(&nested, gated))?;
    } else {
        skip_unmodeled_option(meta)?;
    }
    Ok(())
}

fn derives_deserialize(attrs: &[Attribute]) -> bool {
    let mut found = false;
    for attr in attrs {
        if !attr.path().is_ident("derive") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta
                .path
                .segments
                .last()
                .is_some_and(|seg| seg.ident == "Deserialize")
            {
                found = true;
            }
            Ok(())
        });
    }
    found
}

/// The subset of `#[serde(...)]` container options this walk cares
/// about.
#[derive(Debug, Default)]
struct SerdeAttrs {
    deny_unknown_fields: bool,
    untagged: bool,
    /// `#[serde(tag = "…")]`, with or without `content`.
    internally_tagged: bool,
    /// `#[serde(from = "…")]` / `#[serde(try_from = "…")]`.
    proxy: Option<String>,
    /// `#[serde(into = "…")]`.
    into_proxy: Option<String>,
}

impl SerdeAttrs {
    fn read(attrs: &[Attribute]) -> Self {
        let mut out = SerdeAttrs::default();
        for attr in attrs {
            if !attr.path().is_ident("serde") {
                continue;
            }
            // Parse errors are ignored on purpose: an option this
            // walk does not model must not take the whole file down.
            // That discard is also why every reader here pairs with
            // `unread_serde_attrs` — see `TypeInfo::unread_serde`.
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("deny_unknown_fields") {
                    out.deny_unknown_fields = true;
                } else if meta.path.is_ident("untagged") {
                    out.untagged = true;
                } else if meta.path.is_ident("tag") {
                    out.internally_tagged = true;
                    let _: syn::LitStr = meta.value()?.parse()?;
                } else if meta.path.is_ident("from") || meta.path.is_ident("try_from") {
                    let literal: syn::LitStr = meta.value()?.parse()?;
                    out.proxy = Some(last_path_segment(&literal.value()));
                } else if meta.path.is_ident("into") {
                    let literal: syn::LitStr = meta.value()?.parse()?;
                    out.into_proxy = Some(last_path_segment(&literal.value()));
                } else {
                    skip_unmodeled_option(&meta)?;
                }
                Ok(())
            });
        }
        out
    }
}

/// Consume whatever follows a serde option this walk does not model,
/// so the options written **after** it in the same attribute are
/// still seen.
///
/// Serde's option grammar has three shapes: a bare flag
/// (`deny_unknown_fields`), a `name = value` pair (`tag = "kind"`),
/// and a nested list (`rename_all(deserialize = "camelCase")`,
/// `bound(...)`, `rename(...)`). The list form is the one that used
/// to end the parse — `parse_nested_meta` hands over the path, finds
/// `(` where it wanted `,`, and errors; the caller discards the
/// error, and every option after it in that attribute is never read.
/// `#[serde(rename_all(deserialize = "camelCase"), deny_unknown_fields)]`
/// therefore read as carrying no `deny_unknown_fields` at all, which
/// is a *silent* restoration of reject-at-load.
///
/// The list arm recurses through syn's own nested-meta parser rather
/// than matching the three spellings serde has today: the next option
/// serde grows would re-open the hole, and the point of this module
/// is that nothing is listed.
///
/// Cost: one token-tree walk of the skipped option; no allocation
/// beyond what syn's parser needs.
fn skip_unmodeled_option(meta: &syn::meta::ParseNestedMeta) -> syn::Result<()> {
    if meta.input.peek(syn::Token![=]) {
        meta.value()?.parse::<syn::Expr>()?;
    } else if meta.input.peek(syn::token::Paren) {
        meta.parse_nested_meta(|nested| skip_unmodeled_option(&nested))?;
    }
    Ok(())
}

/// Every `#[serde(...)]` attribute in `attrs` that this module's
/// reader cannot consume to the end, as `"<attribute>: <error>"`.
///
/// The fail-safe half of the parser, and the reason the polarity of
/// the guard test above it is survivable. [`SerdeAttrs::read`],
/// [`omit_predicates`], [`flattened_fields`] and [`is_skipped`] all
/// discard their parse errors, so a partially-read attribute yields
/// the same `false`/empty answer as an absent one. Here the same
/// attributes are walked a second time by a reader that claims
/// nothing and only has to reach the end; whatever defeats it is
/// reported by name instead of being mistaken for good news.
///
/// Cost: a second `syn` pass over the item's serde attributes only —
/// no file I/O, nothing per field type.
fn unread_serde_attrs(attrs: &[Attribute]) -> Vec<String> {
    let mut out = Vec::new();
    for attr in attrs {
        // **An attribute this walk never looks inside is the third
        // way to be wrong, and until this it had no reporter.** The
        // readers here all decide what to examine by matching the
        // attribute's path, so anything that *wraps* a serde option
        // is invisible to every one of them — and invisible to the
        // check below, which only reports attributes it tried and
        // could not finish. `#[cfg_attr(all(), serde(untagged))]`
        // therefore made a reachable enum genuinely untagged while
        // the drift test stayed green.
        //
        // Reading through it is not the answer: that would mean
        // deciding which `cfg` predicates are active, which a source
        // walk cannot know, and it would turn a fail-safe into a
        // judgment call. Reporting it keeps the module's own rule —
        // nothing is listed, and unknown is not safe.
        if attr.path().is_ident("cfg_attr") {
            out.push(format!(
                "{} — not read, so any serde option inside it was never seen. This is \
                 not a claim that it hides a key; it is a claim that nobody can tell. \
                 Either move the option out of the `cfg_attr`, or add the type here \
                 once somebody has looked.",
                attribute_source(attr)
            ));
            continue;
        }
        if !attr.path().is_ident("serde") {
            continue;
        }
        if let Err(e) = attr.parse_nested_meta(|meta| skip_unmodeled_option(&meta)) {
            out.push(format!("{}: {e}", attribute_source(attr)));
        }
    }
    out
}

/// Whether `attrs` carries a container-level `#[serde(...)]`.
///
/// Read as "this type is configured for serde, however its derive is
/// spelled". [`derives_deserialize`] matches the identifier
/// `Deserialize` in a `derive` list, so an aliased import
/// (`use serde::Deserialize as De;`) or a `cfg_attr`-wrapped derive
/// answers `false` — and `false` does not mis-flag, it removes the
/// type from the check altogether. A serde container attribute is
/// independent evidence that the type takes part, and a serde
/// container attribute on a type that really does not derive serde
/// traits is itself something a reader should be shown.
///
/// Container-level only, deliberately: a field-level
/// `skip_serializing_if` says nothing about whether the *container*
/// is deserialized, and counting it would pull every serialize-only
/// type into the check.
fn has_serde_container_attr(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| attr.path().is_ident("serde"))
}

/// A `#[serde(...)]` attribute rendered back toward source text, so a
/// failure message names the attribute a reader has to go and look
/// at. Leans on the token stream's own `Display` rather than adding
/// `quote` as a dependency for one string.
fn attribute_source(attr: &Attribute) -> String {
    let name = attr
        .path()
        .get_ident()
        .map_or_else(|| "?".to_string(), ToString::to_string);
    match &attr.meta {
        syn::Meta::List(list) => format!("#[{name}({})]", list.tokens),
        _ => format!("#[{name}]"),
    }
}

/// Every `#[serde(...)]` attribute carried by `attrs` or by any field
/// in `fields` that the reader cannot finish. The two are collected
/// together because a field-level `bound(...)` hides a later
/// `flatten` on the same field exactly the way a container-level one
/// hides a later `deny_unknown_fields`.
fn unread_serde_attrs_with_fields(attrs: &[Attribute], fields: &Fields) -> Vec<String> {
    let mut out = unread_serde_attrs(attrs);
    for field in fields {
        out.extend(unread_serde_attrs(&field.attrs));
    }
    out
}

/// `"serialized::CustomMutationIn"` → `"CustomMutationIn"`.
fn last_path_segment(path: &str) -> String {
    path.rsplit("::").next().unwrap_or(path).trim().to_string()
}

/// Type names mentioned by fields that serde can populate. A field
/// marked `#[serde(skip)]` / `#[serde(skip_deserializing)]` never
/// receives on-disk data, so it does not extend the contract.
fn referenced_types(fields: &Fields) -> Vec<String> {
    let mut out = Vec::new();
    for field in fields {
        if is_skipped(&field.attrs) {
            continue;
        }
        collect_type_names(&field.ty, &mut out);
    }
    out
}

/// Every non-skipped field whose type mentions `Vec`, paired with
/// the type names that type mentions. See
/// [`TypeInfo::sequence_fields`].
///
/// A tuple field is not reported: it has no JSON member name for a
/// route to be published under, and serde writes the struct as an
/// array whose positions no document names.
fn sequence_fields(fields: &Fields) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    for field in fields {
        if is_skipped(&field.attrs) {
            continue;
        }
        let Some(ident) = field.ident.as_ref() else {
            continue;
        };
        let mut mentioned = Vec::new();
        collect_type_names(&field.ty, &mut mentioned);
        if !mentioned.iter().any(|name| name == "Vec") {
            continue;
        }
        out.push((json_member_name(field, ident), mentioned));
    }
    out
}

/// The name a field is written under on disk: its
/// `#[serde(rename = "...")]` when it has one, its identifier
/// otherwise. `MindEdge::edge_type` is written `"type"`, and a
/// published list of member names has to say what the file says.
fn json_member_name(field: &syn::Field, ident: &syn::Ident) -> String {
    let mut renamed = None;
    for attr in &field.attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("rename") && meta.input.peek(syn::Token![=]) {
                let literal: syn::LitStr = meta.value()?.parse()?;
                renamed = Some(literal.value());
            } else {
                skip_unmodeled_option(&meta)?;
            }
            Ok(())
        });
    }
    renamed.unwrap_or_else(|| ident.to_string())
}

/// The `#[serde(skip_serializing_if = "...")]` predicate of every
/// field that carries one, as written (`"Option::is_none"`,
/// `"is_default_position"`).
///
/// Unlike [`referenced_types`] this looks at every field, skipped or
/// not: it answers a question about what the *saver* may leave out.
fn omit_predicates(fields: &Fields) -> Vec<String> {
    let mut out = Vec::new();
    for field in fields {
        for attr in &field.attrs {
            if !attr.path().is_ident("serde") {
                continue;
            }
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("skip_serializing_if") {
                    let literal: syn::LitStr = meta.value()?.parse()?;
                    out.push(literal.value());
                } else {
                    skip_unmodeled_option(&meta)?;
                }
                Ok(())
            });
        }
    }
    out
}

/// The names of the fields marked `#[serde(flatten)]`.
///
/// A flattened field is the one shape that makes an unrecognized key
/// invisible to the loader's capture: serde routes every member no
/// declared field claimed into it instead of handing the value to
/// `deserialize_ignored_any`. Unnamed fields report as `<unnamed>` —
/// serde rejects `flatten` on a tuple field, so the name is only ever
/// missing on source that will not compile anyway, and losing the
/// entry would be worse than an ugly one.
fn flattened_fields(fields: &Fields) -> Vec<String> {
    let mut out = Vec::new();
    for field in fields {
        for attr in &field.attrs {
            if !attr.path().is_ident("serde") {
                continue;
            }
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("flatten") {
                    out.push(
                        field
                            .ident
                            .as_ref()
                            .map_or_else(|| "<unnamed>".to_string(), ToString::to_string),
                    );
                } else {
                    skip_unmodeled_option(&meta)?;
                }
                Ok(())
            });
        }
    }
    out
}

fn is_skipped(attrs: &[Attribute]) -> bool {
    let mut skipped = false;
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") || meta.path.is_ident("skip_deserializing") {
                skipped = true;
            } else {
                skip_unmodeled_option(&meta)?;
            }
            Ok(())
        });
    }
    skipped
}

/// Stand-in names for the three `Type` forms [`collect_type_names`]
/// cannot read the inner types out of. Angle-bracketed so they can
/// never be confused with a Rust identifier, and deliberately absent
/// from [`EXPECTED_TERMINATORS`] so the first one to appear on the
/// load graph goes red rather than quiet.
const UNREADABLE_DYN: &str = "<dyn Trait>";
/// See [`UNREADABLE_DYN`].
const UNREADABLE_IMPL: &str = "<impl Trait>";
/// See [`UNREADABLE_DYN`].
const UNREADABLE_MACRO: &str = "<macro-generated type>";

/// Every identifier appearing in `ty`, including generic arguments.
/// Module qualifiers come along harmlessly — a name only matters when
/// the index holds a type by that name.
///
/// A form whose inner types cannot be named contributes a sentinel
/// instead of nothing — see [`UNREADABLE_DYN`].
fn collect_type_names(ty: &Type, out: &mut Vec<String>) {
    match ty {
        Type::Path(path) => {
            if let Some(qself) = &path.qself {
                collect_type_names(&qself.ty, out);
            }
            for segment in &path.path.segments {
                out.push(segment.ident.to_string());
                match &segment.arguments {
                    PathArguments::AngleBracketed(args) => {
                        for arg in &args.args {
                            if let GenericArgument::Type(inner) = arg {
                                collect_type_names(inner, out);
                            }
                        }
                    }
                    PathArguments::Parenthesized(args) => {
                        for inner in &args.inputs {
                            collect_type_names(inner, out);
                        }
                    }
                    PathArguments::None => {}
                }
            }
        }
        Type::Reference(inner) => collect_type_names(&inner.elem, out),
        Type::Slice(inner) => collect_type_names(&inner.elem, out),
        Type::Array(inner) => collect_type_names(&inner.elem, out),
        Type::Paren(inner) => collect_type_names(&inner.elem, out),
        Type::Group(inner) => collect_type_names(&inner.elem, out),
        Type::Ptr(inner) => collect_type_names(&inner.elem, out),
        Type::Tuple(tuple) => {
            for inner in &tuple.elems {
                collect_type_names(inner, out);
            }
        }
        // **A type form this walk cannot read becomes a name it
        // cannot resolve, on purpose.** These three carry inner types
        // this function has no way to name, and dropping them was
        // silent: unlike an unresolved *name*, a dropped one leaves
        // nothing for `unresolved_from` to report and nothing for
        // [`EXPECTED_TERMINATORS`] to adjudicate. Pushing a sentinel
        // routes the case into the fail-safe that already works
        // rather than adding a second one — the walk reports a
        // terminator nobody has vetted, and a reader decides.
        //
        // Not a claim that anything is hidden here; a claim that the
        // walk cannot say. The sentinels are angle-bracketed so they
        // cannot collide with a Rust identifier.
        Type::TraitObject(_) => out.push(UNREADABLE_DYN.to_string()),
        Type::ImplTrait(_) => out.push(UNREADABLE_IMPL.to_string()),
        Type::Macro(_) => out.push(UNREADABLE_MACRO.to_string()),
        _ => {}
    }
}

/// Absolute path to baumhard's own `src/` directory, resolved from
/// the crate's `CARGO_MANIFEST_DIR` so the walk finds the sources no
/// matter which crate's test invoked it.
pub fn crate_src_root() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/src"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name the walk from `MindMap` is allowed to stop at.
    ///
    /// Four kinds of entry, and the distinction is the whole point of
    /// the list:
    ///
    /// 1. **Primitives and containers** — `String`, `f32`, `Option`,
    ///    `Vec`, `HashMap`, `Value`. No fields of ours; stopping is
    ///    correct and always will be.
    /// 2. **Module path qualifiers.** `collect_type_names` pushes
    ///    every segment of a path, so
    ///    `crate::mindmap::animation::AnimationTiming` contributes
    ///    three names that are not types alongside the one that is.
    /// 3. **Foreign types, traits, and generic parameters** — `Vec2`
    ///    (glam), `OrderedFloat` (ordered-float), the `Discriminated`
    ///    trait named by a `<F as Discriminated>::Discriminant`
    ///    qself, and `F` itself. Their shape is not ours to annotate;
    ///    if one ever grows named JSON keys a map can write, that is
    ///    a review decision, not a silent one.
    /// 4. **Types this walk genuinely cannot see** — `AppFont`, which
    ///    `lib/baumhard/build.rs` emits into `$OUT_DIR`, and strum's
    ///    generated `Discriminant` tags. Both are unit enums today,
    ///    so they carry no JSON keys and cost nothing. They are on
    ///    the list because the module's promise — a new field of a
    ///    new type extends the covered set on its own — is *false*
    ///    for anything a build script emits, and a comment saying so
    ///    drifts where a checked list does not.
    const EXPECTED_TERMINATORS: &[&str] = &[
        // 1. primitives and containers
        "BTreeSet",
        "Box",
        "FxHashMap",
        "HashMap",
        "Option",
        "String",
        "Value",
        "Vec",
        "bool",
        "f32",
        "f64",
        "i16",
        "i32",
        "u32",
        "u8",
        "usize",
        // 2. module path qualifiers, not types
        "animation",
        "crate",
        "mindmap",
        "serde_json",
        // 3. foreign types, traits, and generic parameters
        "Discriminated",
        "F",
        "OrderedFloat",
        "Vec2",
        // 4. real types this walk cannot see
        "AppFont",
        "Discriminant",
    ];

    /// **The walker's blind spot, made visible.** The walk stops at
    /// any name it did not index, and it cannot tell "known-terminal
    /// primitive" from "type I failed to resolve" — `AppFont` lives
    /// in `$OUT_DIR` and is reachable from a real map, and nothing
    /// about the walk announces that.
    ///
    /// So the list is here rather than in a comment: every terminator
    /// must be one somebody looked at. When a field of a new
    /// unindexed type lands, this fails naming it, and the reader
    /// decides whether it belongs on the list or whether the walk has
    /// to learn to reach it. That is the same drift discipline
    /// `test_no_loadable_type_can_swallow_an_unknown_key` applies to the
    /// types the walk *does* see.
    #[test]
    fn test_every_walk_terminator_is_an_expected_one() {
        let graph = TypeGraph::build(&crate_src_root());
        let unexpected: Vec<String> = graph
            .unresolved_from("MindMap")
            .into_iter()
            .filter(|name| !EXPECTED_TERMINATORS.contains(&name.as_str()))
            .collect();
        assert!(
            unexpected.is_empty(),
            "the reachability walk stops at {} name(s) nobody has vetted. Each is \
             either a primitive (add it to EXPECTED_TERMINATORS) or a type that is \
             part of the on-disk contract and is silently missing from the \
             unknown-key coverage — the case `AppFont` is on the list for:\n  {}",
            unexpected.len(),
            unexpected.join("\n  ")
        );
    }

    /// The allowlist itself must not rot: an entry for a name the
    /// walk no longer reaches is a claim about the model that stopped
    /// being true, and it would keep covering for a *different* name
    /// that happened to reappear later.
    #[test]
    fn test_no_expected_terminator_is_stale() {
        let graph = TypeGraph::build(&crate_src_root());
        let actual = graph.unresolved_from("MindMap");
        let stale: Vec<&&str> = EXPECTED_TERMINATORS
            .iter()
            .filter(|name| !actual.contains(**name))
            .collect();
        assert!(
            stale.is_empty(),
            "EXPECTED_TERMINATORS lists name(s) the walk from MindMap no longer \
             stops at — delete them: {stale:?}"
        );
    }

    /// The walk is only as good as its index: a name declared twice
    /// would make "which `Position`?" unanswerable, and the coverage
    /// assertion built on top would be checking an arbitrary one.
    #[test]
    fn test_type_graph_has_no_ambiguous_names() {
        let graph = TypeGraph::build(&crate_src_root());
        assert!(
            graph.duplicate_names().is_empty(),
            "type names declared more than once — the reachability walk \
             resolves by bare name and cannot tell them apart: {:?}",
            graph.duplicate_names()
        );
    }

    /// A spot check that the walk actually descends rather than
    /// returning its root: `MindMap` must reach a type three field
    /// hops away (`MindMap` → `MindNode` → `MindSection` →
    /// `TextRun`).
    #[test]
    fn test_reachable_from_follows_nested_fields() {
        let graph = TypeGraph::build(&crate_src_root());
        let reached: Vec<&str> = graph
            .reachable_from("MindMap")
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        for expected in ["MindMap", "MindNode", "MindSection", "TextRun"] {
            assert!(
                reached.contains(&expected),
                "{expected} must be reachable from MindMap"
            );
        }
    }

    /// `#[serde(from = "...")]` moves the on-disk shape onto the
    /// proxy, so the walk has to follow it — `CustomMutationIn` is
    /// named only in that attribute, never as a field type.
    #[test]
    fn test_reachable_from_follows_a_serde_proxy() {
        let graph = TypeGraph::build(&crate_src_root());
        let reached: Vec<&str> = graph
            .reachable_from("MindMap")
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        assert!(
            reached.contains(&"CustomMutationIn"),
            "the deserialize proxy behind CustomMutation must be reachable"
        );
    }

    /// The write graph is not the read graph. `CustomMutation`'s keys
    /// are written by `CustomMutationOut` via
    /// `#[serde(into = "...")]`, a type no deserializer ever sees —
    /// so `reachable_from` must not reach it while
    /// `omit_predicates_from` must. `String::is_empty` appears on
    /// exactly one field in the whole crate, `CustomMutationOut`'s,
    /// which makes it the witness.
    #[test]
    fn test_omit_predicates_follow_the_serialize_proxy() {
        let graph = TypeGraph::build(&crate_src_root());
        let read_side: Vec<&str> = graph
            .reachable_from("MindMap")
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        assert!(
            !read_side.contains(&"CustomMutationOut"),
            "a serialize-only proxy must stay out of the load-path coverage set"
        );
        assert!(
            graph.omit_predicates_from("MindMap").contains("String::is_empty"),
            "the write walk must reach CustomMutationOut and see its \
             skip_serializing_if predicates"
        );
    }

    /// Test-only declarations must stay out of the index, or a stub
    /// struct in a test module could shadow a real model type.
    #[test]
    fn test_type_graph_excludes_test_sources() {
        let graph = TypeGraph::build(&crate_src_root());
        assert!(
            graph.get("StubCtx").is_none(),
            "a struct declared inside a test module must not be indexed"
        );
    }

    /// The attributes below are parsed rather than planted on a real
    /// model type: a planted attribute proves the reader sees *that*
    /// spelling on *that* type, and the defect these tests exist for
    /// was a whole grammatical shape the reader walked past.
    fn parse_attrs(source: &str) -> Vec<Attribute> {
        syn::parse::Parser::parse_str(Attribute::parse_outer, source)
            .unwrap_or_else(|e| panic!("the test's own attribute must parse: {e}\n{source}"))
    }

    fn parse_struct(source: &str) -> syn::ItemStruct {
        syn::parse_str(source).unwrap_or_else(|e| panic!("the test's own struct must parse: {e}\n{source}"))
    }

    /// **The evasion this reader must not have.** A serde option
    /// written in list form — `rename_all(...)`, `rename(...)`,
    /// `bound(...)` — used to end the parse where it started, because
    /// `parse_nested_meta` wants a `,` and finds a `(`. The error is
    /// discarded by design, so every option after it in the same
    /// attribute went unread and the container reported as carrying
    /// none of them.
    ///
    /// That is not a cosmetic gap in a source walker. The guard this
    /// feeds — `loader::tests::test_no_loadable_type_can_swallow_an_
    /// unknown_key` — asks "does any loadable type carry
    /// `deny_unknown_fields`?", and an unread attribute answers *no*.
    /// A `#[serde(rename_all(deserialize = "camelCase"),
    /// deny_unknown_fields)]` on any model struct would therefore
    /// restore reject-at-load, the policy #115 exists to reverse, on
    /// a green suite.
    #[test]
    fn test_a_serde_flag_after_a_list_form_option_is_still_read() {
        let denying = parse_attrs(r#"#[serde(rename_all(deserialize = "camelCase"), deny_unknown_fields)]"#);
        assert!(
            SerdeAttrs::read(&denying).deny_unknown_fields,
            "a `deny_unknown_fields` written after a list-form option must be read — \
             missing it reports the type as preserving unknown keys when it rejects them"
        );

        let untagged = parse_attrs(r#"#[serde(rename(serialize = "m"), untagged)]"#);
        assert!(
            SerdeAttrs::read(&untagged).untagged,
            "an `untagged` written after a list-form option must be read"
        );

        let proxied = parse_attrs(r#"#[serde(bound(deserialize = "T: Sized"), from = "Shape")]"#);
        assert_eq!(
            SerdeAttrs::read(&proxied).proxy.as_deref(),
            Some("Shape"),
            "a `from` proxy written after a list-form option must be read — missing it \
             leaves the type that really owns the on-disk shape out of the walk"
        );

        // **The spelling must not matter.** `rename_all`, `rename` and
        // `bound` are the three list-form options serde has today, and
        // a fix that recognized those three by name would be re-opened
        // by the fourth. `skip_unmodeled_option` recurses through syn's
        // own nested-meta grammar instead, so an option nobody has
        // written yet — nested to any depth, with or without values —
        // is consumed the same way.
        let invented = parse_attrs(
            r#"#[serde(zz_from_a_future_serde(alpha = "x", beta, gamma(delta = 1)), deny_unknown_fields)]"#,
        );
        assert!(
            SerdeAttrs::read(&invented).deny_unknown_fields,
            "the reader must handle list *shape*, not the three list-form spellings serde \
             happens to have today — otherwise the next option serde adds re-opens the \
             evasion this whole file exists to close"
        );
        assert!(
            unread_serde_attrs(&invented).is_empty(),
            "and it must be consumed cleanly rather than merely survived: {:?}",
            unread_serde_attrs(&invented)
        );
    }

    /// The same shape on a *field* attribute, where all three of the
    /// remaining readers live. A field-level `bound(...)` hides a
    /// later `flatten`, `skip`, or `skip_serializing_if` exactly the
    /// way a container-level `rename_all(...)` hides a later
    /// `deny_unknown_fields`, and each of those three silently
    /// widens what the guards above believe is safe.
    #[test]
    fn test_a_field_serde_flag_after_a_list_form_option_is_still_read() {
        let flattening =
            parse_struct(r#"struct S { #[serde(bound(deserialize = "T: Sized"), flatten)] rest: Value }"#);
        assert_eq!(
            flattened_fields(&flattening.fields),
            vec!["rest".to_string()],
            "a `flatten` written after a list-form option must be read — a flattened \
             field absorbs the key before the loader's capture ever sees it"
        );

        let skipping = parse_struct(
            r#"struct S { #[serde(bound(deserialize = "T: Sized"), skip)] hidden: ControlPoint }"#,
        );
        assert!(
            referenced_types(&skipping.fields).is_empty(),
            "a `skip` written after a list-form option must be read — missing it walks \
             into a type no deserializer can reach and holds it to the load-path rules"
        );

        let omitting = parse_struct(
            r#"struct S { #[serde(bound(deserialize = "T: Sized"), skip_serializing_if = "Option::is_none")] a: Option<f64> }"#,
        );
        assert_eq!(
            omit_predicates(&omitting.fields),
            vec!["Option::is_none".to_string()],
            "a `skip_serializing_if` written after a list-form option must be read — \
             missing it lets a new omission reason bypass OMITTABLE_WHEN"
        );
    }

    /// **The property the fix above is worth less than.** Closing one
    /// grammatical shape does not make the reader complete, and the
    /// readers all discard their parse errors, so the next shape they
    /// cannot finish would be silent in exactly the same way.
    ///
    /// So a half-read attribute has to be *reportable*, and it is:
    /// [`unread_serde_attrs`] walks the same attributes claiming
    /// nothing and only having to reach the end. Here it is handed a
    /// `#[serde(...)]` whose content is not a meta item at all, which
    /// no amount of option-shape coverage will ever parse.
    #[test]
    fn test_a_serde_attribute_the_reader_cannot_finish_is_reported() {
        let readable = parse_attrs(r#"#[serde(rename_all(deserialize = "camelCase"), untagged)]"#);
        assert!(
            unread_serde_attrs(&readable).is_empty(),
            "an attribute the reader consumes to the end must not be reported: {:?}",
            unread_serde_attrs(&readable)
        );

        let unreadable = parse_attrs(r#"#[serde("not a meta item", deny_unknown_fields)]"#);
        let reported = unread_serde_attrs(&unreadable);
        assert_eq!(
            reported.len(),
            1,
            "an attribute the reader cannot consume to the end must be reported once, \
             so a caller cannot mistake `read` returning its defaults for the attribute \
             carrying nothing: {reported:?}"
        );
        assert!(
            reported[0].starts_with("#[serde("),
            "the report must name the attribute as written, or a reader cannot find \
             it: {reported:?}"
        );
    }

    /// And the report has to be *watched*, over the whole index
    /// rather than the part of it that happens to be reachable from
    /// `MindMap` today — a reader gap should go red the run it
    /// appears, not the later run where a type carrying it joins the
    /// load graph.
    #[test]
    fn test_no_serde_attribute_defeats_the_reader() {
        let graph = TypeGraph::build(&crate_src_root());
        let unread = graph.unread_serde_attrs();
        assert!(
            unread.is_empty(),
            "the serde attribute reader could not consume {} attribute(s) to the end. \
             Every flag written after the point it stopped is invisible to it, and \
             invisible reads as absent — which is the answer that makes the \
             unknown-key guards pass. Teach `skip_unmodeled_option` the shape:\n  {}",
            unread.len(),
            unread.join("\n  ")
        );
    }

    /// A `TypeGraph` over one planted source file, so a predicate
    /// whose *empty* answer is a green suite can be driven over
    /// shapes this crate does not contain — including the shapes
    /// nobody would ever write on purpose.
    fn graph_of(source: &str) -> TypeGraph {
        let dir = crate::util::test_temp::TempDir::new("serde-coverage");
        std::fs::write(dir.join("planted.rs"), source).expect("planted source must be writable");
        TypeGraph::build(dir.path())
    }

    /// The four attributes
    /// [`TypeInfo::unknown_key_hiding_reasons`] exists to catch, each
    /// on a type that carries nothing else, plus the clean control
    /// that has to stay silent. Driven over planted source rather
    /// than over the crate, because the crate is supposed to contain
    /// none of them — a check whose only inputs are the negative
    /// cases passes just as loudly when it has stopped looking.
    #[test]
    fn test_every_key_hiding_serde_attribute_is_named_as_a_reason() {
        let cases = [
            ("#[serde(deny_unknown_fields)]", "deny_unknown_fields"),
            ("#[serde(untagged)]", "untagged"),
            (r#"#[serde(tag = "kind")]"#, "tag"),
        ];
        for (attribute, expected) in cases {
            let graph = graph_of(&format!(
                "#[derive(Deserialize)]\n{attribute}\npub struct Planted {{ pub x: f64 }}\n"
            ));
            let reasons = graph
                .get("Planted")
                .expect("the planted type must be indexed")
                .unknown_key_hiding_reasons();
            assert!(
                reasons.iter().any(|r| r.contains(expected)),
                "{attribute} hides an unrecognized key and must be reported: {reasons:?}"
            );
        }

        let flattening =
            graph_of("#[derive(Deserialize)]\npub struct Planted { #[serde(flatten)] pub rest: Value }\n");
        assert!(
            flattening
                .get("Planted")
                .expect("the planted type must be indexed")
                .unknown_key_hiding_reasons()
                .iter()
                .any(|r| r.contains("flatten")),
            "a flattened field absorbs the key before the capture sees it"
        );

        let clean = graph_of("#[derive(Deserialize)]\npub struct Planted { pub x: f64 }\n");
        assert!(
            clean
                .get("Planted")
                .expect("the planted type must be indexed")
                .unknown_key_hiding_reasons()
                .is_empty(),
            "serde's default hands an unrecognized key to `deserialize_ignored_any`, \
             which is exactly what the loader's capture wraps — a plain struct is safe \
             and must not be reported"
        );
    }

    /// The derived array list is **published in a spec authors
    /// read**, so the names in it have to be the names the file uses,
    /// not the identifiers the model happens to spell them with —
    /// `MindEdge::edge_type` is written `"type"`. And a field serde
    /// never populates is not a place a route can cross, so it has no
    /// business on the list either.
    ///
    /// Neither is exercised by the model today: nothing renames a
    /// sequence field and nothing skips one. That is precisely why
    /// they are planted here — an unexercised branch of a *published*
    /// derivation is a wrong doc waiting for the first field that
    /// uses it.
    #[test]
    fn test_the_published_member_name_is_the_one_the_file_uses() {
        let graph = graph_of(
            "#[derive(Deserialize)] pub struct Root { \
               #[serde(rename = \"points\")] pub control: Vec<Leaf>, \
               #[serde(skip)] pub hidden: Vec<Leaf> }\n\
             #[derive(Deserialize)] pub struct Leaf { pub x: f64 }\n",
        );
        let names = graph.key_bearing_sequences_from("Root");
        assert!(
            names.contains("points"),
            "a renamed field must be published under the name the file carries: {names:?}"
        );
        assert!(
            !names.contains("control"),
            "and not under the identifier, which appears in no document: {names:?}"
        );
        assert!(
            !names.contains("hidden"),
            "a `#[serde(skip)]` field never receives on-disk data, so no captured \
             route crosses it: {names:?}"
        );
    }

    /// **"Reachable from X" and "only reachable through X" are
    /// different questions, and only the second one is worth
    /// asking.** `loader`'s
    /// `test_every_variant_bearing_enum_sits_inside_a_skippable_construct`
    /// asks whether every enum a load can meet sits inside something
    /// the loader can excise; a type used in *both* a skippable and
    /// an unskippable position answers the weaker question yes and
    /// the real one no, and the unskippable use is the one that kills
    /// the document.
    ///
    /// Planted rather than driven over the crate, because the crate
    /// is supposed to have no such type — so the case that would
    /// break the check cannot be built out of what is there.
    #[test]
    fn test_a_type_reached_both_ways_survives_the_exclusion() {
        let graph = graph_of(
            "#[derive(Deserialize)] pub struct Root { pub node: Node, pub skippable: Skippable }\n\
             #[derive(Deserialize)] pub struct Node { pub shared: Shared }\n\
             #[derive(Deserialize)] pub struct Skippable { pub shared: Shared, pub own: OnlyHere }\n\
             #[derive(Deserialize)] pub enum Shared { A, B }\n\
             #[derive(Deserialize)] pub enum OnlyHere { C, D }\n",
        );
        let reached: Vec<&str> = graph
            .reachable_from_excluding("Root", &["Skippable"])
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        assert!(
            reached.contains(&"Shared"),
            "`Shared` sits on an unskippable node as well as inside the skippable \
             construct, so cutting the construct out must still reach it — that double \
             use is exactly the case the weaker `reachable_from` question misses: \
             {reached:?}"
        );
        assert!(
            !reached.contains(&"OnlyHere"),
            "`OnlyHere` is reachable only through the blocked type and must be gone: \
             {reached:?}"
        );
        assert!(
            !reached.contains(&"Skippable"),
            "the blocked type itself is not part of what remains: {reached:?}"
        );
    }

    /// **Unknown is not safe.** The predicate's contract is that an
    /// empty answer means "an unrecognized key survives a load
    /// through this type". An attribute the reader could not finish
    /// makes every flag written after it invisible, so an empty
    /// answer would be asserting something nothing checked — and the
    /// suite would be green for the one policy reversal
    /// [issue #115](https://github.com/Svendsys/mandala/issues/115)
    /// exists to prevent.
    #[test]
    fn test_a_type_whose_serde_attributes_were_not_read_is_never_called_safe() {
        let graph = graph_of(
            "#[derive(Deserialize)]\n#[serde(\"not a meta item\")]\npub struct Planted { pub x: f64 }\n",
        );
        let info = graph.get("Planted").expect("the planted type must be indexed");
        assert!(
            !info.unread_serde.is_empty(),
            "the reader cannot finish this attribute, and has to say so"
        );
        assert!(
            info.unknown_key_hiding_reasons()
                .iter()
                .any(|reason| reason.contains("unread")),
            "a type carrying an attribute the reader could not finish must be reported \
             as hiding keys: what it does is unknown, and unknown is not safe"
        );
    }

    /// The `cfg` reader has the same list-form blind spot, and it was
    /// live: this crate gates its native-only test modules with
    /// `#[cfg(all(test, not(target_arch = "wasm32")))]`, and reading
    /// only the bare `test` ident classified every one of them as
    /// production source. Nothing collided yet — the modules declare
    /// no types today — which is exactly why the claim
    /// `test_type_graph_excludes_test_sources` makes is pinned here
    /// against the reader rather than against the current sources.

    /// **The third way to be wrong: a construct the walk never looked
    /// at.** Two fail-safes already existed — `unresolved_from` +
    /// [`EXPECTED_TERMINATORS`] for a *name* that would not resolve,
    /// and [`unread_serde_attrs`] for an *attribute* the reader tried
    /// and could not finish. Neither covered an attribute it never
    /// tried, because every reader here decides what to examine by
    /// matching the attribute's path, and the fail-safe inherited the
    /// same filter.
    ///
    /// `#[cfg_attr(all(), serde(untagged))]` is that case, and `all()`
    /// is unconditionally true — the enum really is untagged in the
    /// built artifact, a key inside a payload really is swallowed,
    /// and the drift test really did stay green.
    #[test]
    fn test_a_serde_option_wrapped_in_cfg_attr_is_reported_not_missed() {
        let graph = graph_of(
            "#[derive(Deserialize)] #[cfg_attr(all(), serde(untagged))] \
             pub enum Root { A(Leaf) }\n\
             #[derive(Deserialize)] pub struct Leaf { pub x: f64 }\n",
        );
        let reasons = graph.get("Root").expect("indexed").unknown_key_hiding_reasons();
        assert_eq!(reasons.len(), 1, "expected one reason, got {reasons:?}");
        assert!(reasons[0].contains("cfg_attr"), "{}", reasons[0]);
        assert!(
            reasons[0].contains("nobody can tell"),
            "the message must not accuse — it reports that the option was never seen: {}",
            reasons[0]
        );
    }

    /// The same wrapper on a *field*, where it hides `flatten` — one
    /// of the four attributes the check exists for. Container-level
    /// reporting alone would leave this silent.
    #[test]
    fn test_a_field_serde_option_wrapped_in_cfg_attr_is_reported() {
        let graph = graph_of(
            "#[derive(Deserialize)] pub struct Root { \
               #[cfg_attr(all(), serde(flatten))] pub extra: Leaf }\n\
             #[derive(Deserialize)] pub struct Leaf { pub x: f64 }\n",
        );
        let reasons = graph.get("Root").expect("indexed").unknown_key_hiding_reasons();
        assert_eq!(reasons.len(), 1, "expected one reason, got {reasons:?}");
        assert!(reasons[0].contains("cfg_attr"), "{}", reasons[0]);
    }

    /// **The derive wrapped too**, which is the worse half: a type
    /// whose `Deserialize` this walk cannot see is not mis-flagged, it
    /// leaves the check altogether. Reporting the `cfg_attr` is only
    /// useful if the type is still asked, so the exemption has to
    /// stand down when anything went unread.
    #[test]
    fn test_a_type_whose_derive_is_also_wrapped_is_still_asked() {
        let graph = graph_of(
            "#[cfg_attr(all(), derive(Deserialize))] #[cfg_attr(all(), serde(untagged))] \
             pub enum Root { A(Leaf) }\n\
             #[derive(Deserialize)] pub struct Leaf { pub x: f64 }\n",
        );
        let info = graph.get("Root").expect("indexed");
        assert!(
            !info.derives_deserialize,
            "the premise: the walk cannot see the derive, which is why the old gate \
             dropped this type"
        );
        assert_eq!(
            info.unknown_key_hiding_reasons().len(),
            2,
            "both wrapped attributes are reported: {:?}",
            info.unknown_key_hiding_reasons()
        );
    }

    /// **A serde container attribute is evidence in its own right.**
    /// [`derives_deserialize`] matches the identifier `Deserialize`,
    /// so an aliased import defeats it — and `false` there removes
    /// the type from the check rather than mis-flagging it, with a
    /// `#[serde(untagged)]` sitting right beside it.
    ///
    /// Nothing is resolved through the `use`: the fix is that a
    /// container-level `serde` attribute is enough to ask the
    /// question, whatever the derive list looks like.
    #[test]
    fn test_a_serde_configured_type_is_asked_however_its_derive_is_spelled() {
        let graph = graph_of(
            "#[derive(De)] #[serde(untagged)] pub enum Root { A(Leaf) }\n\
             #[derive(Deserialize)] pub struct Leaf { pub x: f64 }\n",
        );
        let info = graph.get("Root").expect("indexed");
        assert!(!info.derives_deserialize, "the premise: the derive is unrecognizable");
        assert!(
            info.has_serde_container_attr,
            "but the serde attribute is right there"
        );
        let reasons = info.unknown_key_hiding_reasons();
        assert_eq!(reasons.len(), 1, "expected one reason, got {reasons:?}");
        assert!(reasons[0].contains("untagged"), "{}", reasons[0]);
    }

    /// The negative control for the two widenings above: a type that
    /// is neither derived nor serde-configured, and one that is
    /// ordinary, both stay silent. Without this the widened gate
    /// could be passing by flagging everything.
    #[test]
    fn test_widening_the_gate_does_not_flag_an_innocent_type() {
        let graph = graph_of(
            "pub struct Bare { pub x: f64 }\n\
             #[derive(Deserialize)] pub struct Clean { pub leaf: Leaf }\n\
             #[derive(Deserialize)] pub struct Leaf { pub x: f64 }\n",
        );
        for name in ["Bare", "Clean", "Leaf"] {
            let reasons = graph.get(name).expect("indexed").unknown_key_hiding_reasons();
            assert!(reasons.is_empty(), "{name} must stay silent, got {reasons:?}");
        }
    }

    /// **A type form the walk cannot read becomes a name it cannot
    /// resolve.** `collect_type_names` used to drop `dyn Trait`,
    /// `impl Trait` and a macro-generated type on the floor, and
    /// unlike an unresolved name a dropped one leaves nothing for
    /// [`EXPECTED_TERMINATORS`] to adjudicate. Routing them into the
    /// fail-safe that already works costs no second mechanism.
    #[test]
    fn test_an_unreadable_type_form_surfaces_as_a_terminator() {
        let graph = graph_of(
            "#[derive(Deserialize)] pub struct Root { \
               pub a: Box<dyn Something>, pub b: Wrapper<impl Other>, pub c: some_macro!() }\n",
        );
        let stops = graph.unresolved_from("Root");
        for sentinel in ["<dyn Trait>", "<impl Trait>", "<macro-generated type>"] {
            assert!(
                stops.contains(sentinel),
                "{sentinel} must reach the terminator allowlist for a reader to \
                 adjudicate: {stops:?}"
            );
            assert!(
                !EXPECTED_TERMINATORS.contains(&sentinel),
                "{sentinel} must NOT be pre-vetted, or the first one on the load graph \
                 passes in silence"
            );
        }
    }

    #[test]
    fn test_a_test_gate_written_as_all_is_still_a_test_gate() {
        for source in [
            "#[cfg(test)]",
            r#"#[cfg(all(test, not(target_arch = "wasm32")))]"#,
            "#[cfg(all(unix, all(test)))]",
        ] {
            assert!(
                is_test_gated(&parse_attrs(source)),
                "{source} compiles its item only under `cfg(test)`, so the item is not \
                 part of any on-disk contract and must stay out of the index"
            );
        }
        for source in [
            "#[cfg(not(test))]",
            r#"#[cfg(any(test, feature = "x"))]"#,
            r#"#[cfg(target_arch = "wasm32")]"#,
        ] {
            assert!(
                !is_test_gated(&parse_attrs(source)),
                "{source} does not gate on `test`, and treating it as a test gate would \
                 drop real model types out of the walk"
            );
        }
    }
}
