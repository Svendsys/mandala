// SPDX-License-Identifier: MPL-2.0

//! Reachability walk over the crate's own source, so a test can ask
//! "which types can this deserializer actually be handed?" and get an
//! answer that stays true as the model grows.
//!
//! The problem this exists to solve: the `.mandala.json` format is
//! strict — every type the loader reads denies unknown keys, none can
//! swallow one, none reads an index (`document::tests::format_strict_tests`)
//! — and that only holds while every type between the document root and
//! a key says so. `#[serde(flatten)]` swallows a key before
//! `deny_unknown_fields` is ever consulted; `untagged` and internally
//! tagged enums buffer their input and replay it through a deserializer
//! of their own. None of them produces a compile error. A
//! hand-maintained list of "types that must deny unknown fields" is
//! exactly the sort of twin surface that drifts the moment somebody adds
//! a field — the same failure mode `lib/baumhard/CONVENTIONS.md` §B4
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

use crate::util::source_scan::{is_test_gated, skip_unmodeled_option, test_gated_module_files};
use std::collections::{BTreeMap, BTreeSet, VecDeque};
use std::path::{Path, PathBuf};
use syn::{Attribute, Fields, GenericArgument, Item, PathArguments, Type};

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

/// Where a JSON member name comes from.
///
/// The distinction matters because only one of the two is a *field*
/// name: `serde_derive` hands its `FIELDS` list to
/// `deserialize_struct`, so a reader that wants to check this walk
/// against the compiler has to compare like with like.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MemberKind {
    /// A named field, under its serde name.
    Field,
    /// An unnamed field, which has a member name only when a variant
    /// wraps it. `{"Literal": [ … ]}` writes an externally tagged
    /// newtype variant's payload as the one member of a wrapper
    /// object, and the variant's name is that member — so a `Vec`
    /// payload is an array published under the variant name. Every
    /// other unnamed position is a slot in a fixed-arity array and
    /// has no member name at all.
    VariantPayload,
}

/// A type exactly as it was written, kept so the walk can ask
/// structural questions of it rather than of a bag of its
/// identifiers.
///
/// The wrapper exists for one reason: `syn::Type` implements `Debug`
/// only under syn's `extra-traits` feature, and turning that on for
/// the whole workspace to satisfy one `derive` here would compile a
/// trait tree nothing else wants. It prints as the list of names the
/// walk resolves the type by rather than as source — writing
/// `Vec<Leaf>` would claim a nesting the flat list cannot always
/// reconstruct, and the names are what a failure message needs.
#[derive(Clone)]
pub struct DeclaredType(pub Type);

impl std::fmt::Debug for DeclaredType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut names = Vec::new();
        collect_type_names(&self.0, &mut names);
        write!(f, "{names:?}")
    }
}

/// One place a deserializer can put on-disk data, and the JSON member
/// it is read under.
#[derive(Debug, Clone)]
pub struct SerdeField {
    /// The member name the *file* uses, or `None` where there is
    /// none to publish — a position inside a fixed-arity tuple, whose
    /// index no member names.
    ///
    /// Resolved the way serde resolves it: a field's own
    /// `rename`/`rename(deserialize = …)` first, then the container's
    /// `rename_all`, then the identifier. A route through a map is
    /// recorded against the shape that was *read*, so a
    /// `rename(serialize = …)` deliberately does not count.
    pub member: Option<String>,
    /// Whether [`Self::member`] is a field name or a variant name.
    pub kind: MemberKind,
    /// Every additional spelling `#[serde(alias = "…")]` makes
    /// acceptable. A file may use any of them, so a route may cross
    /// any of them.
    pub aliases: Vec<String>,
    /// The declared type, kept verbatim: whether it is an array is a
    /// question only the whole index can answer.
    pub ty: DeclaredType,
}

/// One indexed type: where it lives, what serde does with it, and
/// which other type names its fields mention.
#[derive(Debug, Clone)]
pub struct TypeInfo {
    /// The item's identifier, as written.
    pub name: String,
    /// The item's path inside the crate, without the `crate::` prefix
    /// — `document::values::Anchor` — which is what the index is keyed
    /// by: two modules may declare one name, and only the path tells
    /// them apart.
    pub path: String,
    /// The module the item is declared in (`document::values`, or the
    /// empty string for the crate root), whose `use` declarations say
    /// what a name its fields mention stands for.
    pub module: String,
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
    /// Every spelling an enum's variants can be written under, in
    /// source order and **under the names the file uses**; empty for
    /// a struct or an alias.
    ///
    /// An externally tagged enum writes its variant as the single
    /// member of a wrapper object, which is a JSON level serde's
    /// ignored-key path does not report — so the old model's route
    /// expansion (`expand_route` (gone)) had to put it back. The names
    /// are here so a test can drive a variant question over **every**
    /// variant a type can meet rather than the two somebody thought of.
    ///
    /// Because these are JSON member names, they are resolved the way
    /// serde resolves them — a variant's own `rename`, then the
    /// container's `rename_all` — and not read off the identifier.
    /// They were read off the identifier until #122, which would have
    /// driven the old model's `expand_route` (gone) over a level no
    /// document contains the moment anything renamed a variant.
    ///
    /// A variant that carries `#[serde(alias = "…")]` contributes
    /// **every** spelling, sorted, in place of the one — which is
    /// exactly the shape of the `VARIANTS` list `serde_derive` hands
    /// `deserialize_enum`, per variant in source order and sorted
    /// inside each variant's group. Publishing the alias is the same
    /// decision [`Self::fields`] carries for a field's: a file may
    /// write it, so a route may cross it.
    pub variants: Vec<String>,
    /// The item's type parameters, by name, in declaration order —
    /// `["E", "M"]` for `MutatorNode<E, M>`. A reference that writes
    /// arguments for them (`MutatorNode<GfxElement, GfxMutation>`)
    /// **binds** them for the walk below that reference: a parameter
    /// used as data is walked as its argument, and a projection
    /// through one (`E::Field`) as the associated type the argument's
    /// `impl` declares. The arguments themselves are not walked at the
    /// reference — an argument the item only projects through is not
    /// data the file holds.
    pub generics: Vec<String>,
    /// The declared type of every deserializable field — every
    /// variant's, for an enum — in source order, or the right-hand
    /// side of a `type` alias. Kept as types rather than names because
    /// what a name inside one means depends on the bindings the walk
    /// reached the item with.
    pub referenced: Vec<DeclaredType>,
    /// Every place a deserializer can put on-disk data into this
    /// type, with the JSON member it is read under and the type it
    /// reads into.
    ///
    /// The raw material for two questions the index alone can
    /// answer. **Which arrays a captured key's route crosses
    /// positionally** — above an array a route names a node by its id
    /// or a palette by its name and is stable across any edit; below
    /// one it is an index, and an edit can slide it onto a different
    /// element. And **what those arrays are called on disk**, which
    /// is not what they are called in Rust.
    ///
    /// The declared type is kept verbatim: what a field's type stands
    /// for needs the whole index to answer.
    pub fields: Vec<SerdeField>,
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
    /// Why an unrecognized key in a map would not survive a
    /// load through this type, or empty when it would.
    ///
    /// The predicate behind
    /// `format_strict_tests::test_every_type_the_format_reads_denies_unknown_fields`,
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
            // Verbatim: each entry already carries the explanation
            // that fits it. A half-read attribute and an attribute
            // nobody looked inside are unread for different reasons
            // and ask the reader for different things, and one
            // appended sentence cannot say both.
            reasons.push(attribute.clone());
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
    /// Every indexed item, keyed by its path ([`TypeInfo::path`]).
    items: BTreeMap<String, TypeInfo>,
    /// Bare name → the paths of every item declared under it, for the
    /// names a scope does not settle (see [`TypeGraph::resolve`]).
    by_name: BTreeMap<String, Vec<String>>,
    /// Every module the indexed sources declare, by path — what tells
    /// a path that starts in a child module from one that starts in
    /// another crate.
    modules: BTreeSet<String>,
    /// Each module's `use` declarations, read so a name its items
    /// mention is resolved the way the compiler resolves it.
    scopes: BTreeMap<String, Scope>,
    /// Paths declared more than once — the same name twice in one
    /// module, which no scope can tell apart.
    duplicates: BTreeSet<String>,
    /// Every associated type an `impl` block declares, keyed by the
    /// implementing type's name and the associated type's name —
    /// `("GfxElement", "Field") → GfxElementField` — with the module
    /// the `impl` is written in, which is where the names on its
    /// right-hand side are resolved. What a projection through a
    /// bound parameter (`E::Field`) resolves to. More than one entry
    /// under a key is kept, so a disagreement is visible rather than
    /// decided by declaration order.
    associated: BTreeMap<(String, String), Vec<(DeclaredType, String)>>,
}

/// One module's `use` declarations, as written: the paths are
/// resolved when a name is looked up, once every module is known.
#[derive(Debug, Clone, Default)]
struct Scope {
    /// Local name → the path it imports (`use a::B as C` records
    /// `C → [a, B]`).
    imports: BTreeMap<String, Vec<String>>,
    /// The paths of every `use path::*`.
    globs: Vec<Vec<String>>,
}

/// What a name written in one module stands for.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Resolution {
    /// The indexed item at this path.
    Found(String),
    /// Nothing in the index: a primitive, a parameter, or a type of
    /// another crate.
    NotIndexed,
    /// More than one indexed item carries the name and the scope it
    /// was written in does not say which: the paths of all of them.
    Ambiguous(Vec<String>),
}

/// What a name looked up in one module's scope stands for, before the
/// bare-name fallback ([`TypeGraph::resolve`]).
enum Lookup {
    /// The indexed item at this path.
    Found(String),
    /// The scope imports the name from another crate.
    Foreign,
    /// The scope does not say.
    Missing,
}

/// How deep [`TypeGraph::lookup`] follows re-exports and glob
/// imports before it gives up; a cycle of globs ends here.
const LOOKUP_DEPTH: usize = 16;

/// What a generic item's type parameters stand for at one place the
/// walk reached it: parameter name → the argument written there, with
/// any parameters of the *enclosing* item already substituted. The
/// argument is **canonical**: every name in it that the index resolves
/// is rewritten to its full `crate::` path where it was written, so it
/// means the same thing in the module of the item it is substituted
/// into ([`TypeGraph::canonical`]).
type Bindings = BTreeMap<String, Type>;

/// One instantiation of an indexed item the walk reached: the item,
/// and what its type parameters were bound to on the way in. A
/// non-generic item is reached with empty bindings, once.
struct Reached<'g> {
    info: &'g TypeInfo,
    bindings: Bindings,
}

/// One name a type mentions in a position that holds data, together
/// with what it resolves to and the type arguments written on it
/// there. The qualifiers of a path that does not start at a module
/// (`E::Field`, `serde_json::Value`) come along with no arguments, as
/// they always have — a name only matters when the index holds a type
/// by that name; those of a `crate::` / `self::` / `super::` path are
/// modules by the language's own rule and are not mentioned.
struct Mention {
    name: String,
    resolution: Resolution,
    arguments: Vec<Type>,
}

impl Mention {
    /// The path of the indexed item the name stands for, if exactly
    /// one.
    fn key(&self) -> Option<&str> {
        match &self.resolution {
            Resolution::Found(key) => Some(key.as_str()),
            _ => None,
        }
    }
}

impl TypeGraph {
    /// Parse every `.rs` file under `src_root` and index the items it
    /// declares, descending into inline `mod { … }` blocks.
    ///
    /// Test sources are skipped — a `struct StubCtx` inside a test
    /// module is not part of any on-disk contract, and letting one
    /// shadow a real model type would silently move the walk. Four
    /// shapes are excluded, and they are four because the first three
    /// left a hole: files named `tests.rs` / `*_tests.rs` /
    /// `*_test.rs`, directories named `tests`, inline `#[cfg(test)]
    /// mod x { … }` blocks, and — through
    /// [`test_gated_module_files`] — the files named by an
    /// **out-of-line** `#[cfg(test)] mod x;` together with everything
    /// those files declare.
    ///
    /// That fourth one is not a corner. `util/mod.rs` gates five
    /// modules exactly that way — `bench_surface`, `manifests`,
    /// `serde_coverage`, `serde_probe` and `source_scan` — and until
    /// it was closed all eleven types of the three gated at the time
    /// (`manifests`, `serde_coverage` and `test_logger`, the last
    /// since widened to `pub`) sat in this index — `TypeGraph` and
    /// `TypeInfo` among them — while the doc here said
    /// `#[cfg(test)]` modules were all excluded. Nothing had
    /// collided yet, which is the only reason it was quiet.
    ///
    /// **Every other file is indexed, the document's included**, and
    /// each item under its path ([`TypeInfo::path`]): the document
    /// declares names other modules declare too — `Anchor`
    /// (`document::values` and `core::primitives`), `Rgba` (a struct in
    /// `document::values`, an alias in `util::color`), `Direction`,
    /// `Edge` — and a name is resolved where it is written, through the
    /// module's own declarations and `use`s ([`Self::resolve`]). Until
    /// V1-C3 the index was keyed by bare name, and `document/element.rs`
    /// and `document/values.rs` were left out of it to keep it so.
    ///
    /// The module of a file is read off its path under `src_root`
    /// (`a/b.rs` and `a/b/mod.rs` are `a::b`; `lib.rs` is the root), so
    /// a `#[path = "…"]` module would be filed under the wrong one —
    /// [`test_gated_module_files`] refuses that attribute outright.
    ///
    /// Panics if a file cannot be read or parsed, or if a `mod x;`
    /// names no file on disk: this backs a test whose entire job is to
    /// notice that the source moved, so a silent skip would defeat it.
    pub fn build(src_root: &Path) -> Self {
        let mut graph = TypeGraph::default();
        let mut files = Vec::new();
        collect_source_files(src_root, &mut files);
        files.sort();
        let test_only = test_gated_module_files(&files);
        files.retain(|file| !test_only.contains(file));
        for file in files {
            let text = std::fs::read_to_string(&file)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
            let parsed = syn::parse_file(&text)
                .unwrap_or_else(|e| panic!("{} must parse as Rust: {e}", file.display()));
            let module = module_of_file(src_root, &file);
            graph.modules.insert(module.clone());
            graph.index_items(&parsed.items, &file, &module);
        }
        graph
    }

    /// The indexed entry for `name` — a path (`document::values::Anchor`)
    /// or a bare name — or `None` when the crate declares nothing by
    /// it.
    ///
    /// Panics when a bare name is declared in more than one module: the
    /// caller asked a question with two answers, and returning either
    /// would check an arbitrary one. Ask by path instead.
    pub fn get(&self, name: &str) -> Option<&TypeInfo> {
        if let Some(info) = self.items.get(name) {
            return Some(info);
        }
        match self.by_name.get(name).map(Vec::as_slice) {
            None | Some([]) => None,
            Some([only]) => self.items.get(only),
            Some(several) => {
                panic!("`{name}` is declared in more than one module ({several:?}); ask for one by path")
            }
        }
    }

    /// Every `#[serde(...)]` attribute anywhere in the indexed
    /// sources that this module's reader could not consume to the
    /// end, as `"<type> (<file>): <attribute>: <error>"`.
    ///
    /// The walk's readers are silent about what defeats them, and
    /// silence reads as "carries nothing" — which is precisely the
    /// answer that makes every guard built on this graph pass. This
    /// reports the blind spot instead, over **all** indexed types
    /// rather than only the ones reachable from the format's roots: a reader
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

    /// Paths declared more than once across the indexed sources — one
    /// name twice in one module, which is a compile error unless the
    /// two are `cfg`-gated apart. No scope can tell such a pair apart,
    /// so the index keeps the first and records the path here; callers
    /// assert this is empty rather than silently trusting whichever
    /// definition landed first.
    pub fn duplicate_names(&self) -> &BTreeSet<String> {
        &self.duplicates
    }

    /// Every place an indexed type mentions a name that more than one
    /// module declares **and the scope it is written in does not
    /// settle**, as `"<name> in <path> (<file>): one of <paths>"`.
    ///
    /// Two modules declaring one name is fine — the index is keyed by
    /// path. A mention the walk cannot pin to one of them is not: the
    /// walk would have to guess, and the guard built on it would be
    /// checking an arbitrary type. [`Self::resolve`] reads a module's
    /// own declarations, its `use`s (renames, groups, re-exports and
    /// globs among them) and explicit `crate::` / `self::` / `super::`
    /// paths; what none of those settles lands here. Over **every**
    /// indexed type, not only the reachable ones, for the reason
    /// [`Self::unread_serde_attrs`] gives: a resolver gap is a property
    /// of the resolver, and it should go red the run it appears.
    ///
    /// Cost: one pass over every indexed type's fields; no I/O.
    pub fn ambiguous_mentions(&self) -> Vec<String> {
        let mut out = Vec::new();
        for info in self.items.values() {
            let mut mentioned = Vec::new();
            for declared in &info.referenced {
                self.mentions(&declared.0, &info.module, &mut mentioned);
            }
            if let Some(proxy) = info.deserialize_proxy.as_deref() {
                mentioned.push(self.mention_of_proxy(proxy, &info.module));
            }
            // The arguments of a generic item are bindings rather than
            // mentions, and a binding is resolved where it is written
            // too ([`Self::canonical`]), so its names are asked here.
            let mut at = 0;
            while at < mentioned.len() {
                let arguments = std::mem::take(&mut mentioned[at].arguments);
                for argument in &arguments {
                    self.mentions(argument, &info.module, &mut mentioned);
                }
                at += 1;
            }
            for mention in mentioned {
                if let Resolution::Ambiguous(paths) = &mention.resolution {
                    out.push(format!(
                        "{} in {} ({}): one of {paths:?}",
                        mention.name,
                        info.path,
                        info.file.display()
                    ));
                }
            }
        }
        out
    }

    /// Every JSON member name a struct or struct variant reachable
    /// from `root` declares, keyed by the type that declares it.
    ///
    /// The walk's answer to "what is this field called on disk",
    /// separated from the array question so it can be held against
    /// the **compiler's** answer —
    /// [`crate::util::serde_probe::DerivedShape::members_of`] reads
    /// the same names out of the generated `Deserialize` impls.
    /// Variant payload names are deliberately excluded: they are
    /// variant names, and comparing them against a `FIELDS` list
    /// would be comparing two different things.
    ///
    /// **Every alias is here**: a name a file may write is a name this
    /// walk is answering about. The
    /// derive agrees — `serde_derive` puts a field's aliases straight
    /// into the `FIELDS` list it hands `deserialize_struct` — so the
    /// two surfaces would otherwise disagree about `alias` in
    /// opposite directions, which they did until #122's review.
    ///
    /// Cost: one walk from `root`; no I/O.
    pub fn member_names_from(&self, root: &str) -> BTreeMap<String, BTreeSet<String>> {
        let mut out: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        for info in self.reachable_from(root) {
            let entry = out.entry(info.name.clone()).or_default();
            for field in &info.fields {
                if field.kind != MemberKind::Field {
                    continue;
                }
                entry.extend(field.member.iter().cloned());
                entry.extend(field.aliases.iter().cloned());
            }
        }
        out
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
    /// Names are read with the reached item's parameters substituted,
    /// so a parameter bound at the reference is never a terminator;
    /// one that was left unbound, or a projection no `impl` resolves,
    /// is — and goes red the same way. A name more than one module
    /// declares that the scope it is written in does not settle is a
    /// terminator too: the walk does not guess, and
    /// [`ambiguous_mentions`](Self::ambiguous_mentions) names it.
    ///
    /// Cost: same walk as `reachable_from` plus one set insert per
    /// referenced name; no I/O.
    pub fn unresolved_from(&self, root: &str) -> BTreeSet<String> {
        self.unresolved_from_excluding(root, &[])
    }

    /// [`unresolved_from`](Self::unresolved_from) **without passing
    /// through** any type named in `blocked`, as
    /// [`reachable_from_excluding`](Self::reachable_from_excluding) walks:
    /// for a type whose `Deserialize` is written by hand, whose fields
    /// are therefore not what a deserializer reads.
    ///
    /// Cost: same walk as `reachable_from`; no I/O.
    pub fn unresolved_from_excluding(&self, root: &str, blocked: &[&str]) -> BTreeSet<String> {
        let mut out = BTreeSet::new();
        for reached in self.walk(root, blocked) {
            let module = reached.info.module.as_str();
            let mut mentioned = Vec::new();
            for declared in &reached.info.referenced {
                self.mentions(
                    &self.substitute(&declared.0, &reached.bindings),
                    module,
                    &mut mentioned,
                );
            }
            if let Some(proxy) = reached.info.deserialize_proxy.as_deref() {
                mentioned.push(self.mention_of_proxy(proxy, module));
            }
            for mention in mentioned {
                if mention.key().is_none() {
                    out.insert(mention.name);
                }
            }
        }
        out
    }

    /// Every indexed type reachable from `root` by following
    /// deserializable fields, including `root` itself — each once,
    /// however many instantiations of it the walk met.
    ///
    /// Names that are not indexed (`String`, `f64`,
    /// `serde_json::Value`) terminate the walk — they carry no fields
    /// of ours. Aliases forward to the types their right-hand side
    /// mentions, and a generic item's parameters are followed as what
    /// the reference bound them to (see [`TypeInfo::generics`]).
    ///
    /// Cost: O(reached instantiations × their field count); no I/O.
    pub fn reachable_from(&self, root: &str) -> Vec<&TypeInfo> {
        distinct_items(self.walk(root, &[]))
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
        distinct_items(self.walk(root, blocked))
    }

    /// The BFS behind [`reachable_from`],
    /// [`reachable_from_excluding`](Self::reachable_from_excluding)
    /// and [`unresolved_from_excluding`](Self::unresolved_from_excluding).
    /// Field references and `#[serde(from)]` proxies are followed; a
    /// `#[serde(into)]` proxy never is, since a serialize-only proxy is
    /// never handed to a deserializer. A name in `blocked` is never
    /// entered, so nothing reachable only through it is reported.
    ///
    /// The unit of the walk is an *instantiation*: an item together
    /// with what its parameters were bound to on the way in, so
    /// `Delta<GlyphAreaField>` and `Delta<GlyphModelField>` are both
    /// walked, and `MutatorNode<GfxElement, GfxMutation>` is walked as
    /// what those two arguments make its fields mean. An item reached
    /// again with the same bindings is not re-entered, which is what
    /// ends the walk on a recursive type.
    ///
    /// `root` and the names in `blocked` are each a path or a bare
    /// name, as [`get`](Self::get) takes them; a blocked bare name
    /// blocks every item declared under it.
    fn walk(&self, root: &str, blocked: &[&str]) -> Vec<Reached<'_>> {
        let mut seen: BTreeSet<String> = BTreeSet::new();
        let mut queue: VecDeque<(&TypeInfo, Bindings)> = VecDeque::new();
        let mut out: Vec<Reached<'_>> = Vec::new();
        let is_blocked = |info: &TypeInfo| {
            blocked
                .iter()
                .any(|name| *name == info.name || *name == info.path)
        };

        let Some(root) = self.get(root) else { return out };
        if is_blocked(root) {
            return out;
        }
        queue.push_back((root, Bindings::new()));
        seen.insert(instantiation_key(&root.path, &Bindings::new()));
        while let Some((info, bindings)) = queue.pop_front() {
            let module = info.module.as_str();
            let mut successors: Vec<(&TypeInfo, Bindings)> = Vec::new();
            for declared in &info.referenced {
                let mut mentioned = Vec::new();
                self.mentions(&self.substitute(&declared.0, &bindings), module, &mut mentioned);
                for mention in mentioned {
                    let Some(next) = mention.key().and_then(|key| self.items.get(key)) else {
                        continue;
                    };
                    // Canonical, so the argument means where the item
                    // it binds is declared what it meant here.
                    let arguments = mention
                        .arguments
                        .iter()
                        .map(|argument| self.canonical(argument, module));
                    let bound = next.generics.iter().cloned().zip(arguments).collect();
                    successors.push((next, bound));
                }
            }
            if let Some(proxy) = info.deserialize_proxy.as_deref() {
                let mention = self.mention_of_proxy(proxy, module);
                if let Some(next) = mention.key().and_then(|key| self.items.get(key)) {
                    successors.push((next, Bindings::new()));
                }
            }
            for (next, bound) in successors {
                if is_blocked(next) {
                    continue;
                }
                if seen.insert(instantiation_key(&next.path, &bound)) {
                    queue.push_back((next, bound));
                }
            }
            out.push(Reached { info, bindings });
        }
        out
    }

    /// `ty` with every parameter in `bindings` replaced by what it is
    /// bound to: a bare parameter (`M`) by its argument, a projection
    /// through one (`E::Field`, `<E as Inspect>::Field` read as its
    /// last segment) by the associated type the argument's `impl`
    /// declares, recursively through generic arguments, references,
    /// slices, arrays and tuples. A projection no `impl` resolves is
    /// left as written, so its names reach
    /// [`unresolved_from`](Self::unresolved_from) and are vetted
    /// there.
    fn substitute(&self, ty: &Type, bindings: &Bindings) -> Type {
        if bindings.is_empty() {
            return ty.clone();
        }
        let mut out = ty.clone();
        self.substitute_in_place(&mut out, bindings);
        out
    }

    fn substitute_in_place(&self, ty: &mut Type, bindings: &Bindings) {
        match ty {
            Type::Path(path) => {
                if path.qself.is_none() {
                    let segments = &path.path.segments;
                    let first = segments.first().map(|segment| segment.ident.to_string());
                    if let Some(bound) = first.as_deref().and_then(|name| bindings.get(name)) {
                        match segments.len() {
                            1 => {
                                *ty = bound.clone();
                                return;
                            }
                            2 => {
                                let projected = segments[1].ident.to_string();
                                if let Some(resolved) = self.project(bound, &projected) {
                                    *ty = resolved;
                                    return;
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(qself) = &mut path.qself {
                    self.substitute_in_place(&mut qself.ty, bindings);
                }
                for segment in path.path.segments.iter_mut() {
                    if let PathArguments::AngleBracketed(arguments) = &mut segment.arguments {
                        for argument in arguments.args.iter_mut() {
                            if let GenericArgument::Type(inner) = argument {
                                self.substitute_in_place(inner, bindings);
                            }
                        }
                    }
                }
            }
            Type::Reference(inner) => self.substitute_in_place(&mut inner.elem, bindings),
            Type::Slice(inner) => self.substitute_in_place(&mut inner.elem, bindings),
            Type::Array(inner) => self.substitute_in_place(&mut inner.elem, bindings),
            Type::Paren(inner) => self.substitute_in_place(&mut inner.elem, bindings),
            Type::Group(inner) => self.substitute_in_place(&mut inner.elem, bindings),
            Type::Tuple(tuple) => {
                for inner in tuple.elems.iter_mut() {
                    self.substitute_in_place(inner, bindings);
                }
            }
            _ => {}
        }
    }

    /// The associated type `name` that `owner`'s `impl` declares, when
    /// exactly one spelling of it is declared. `None` for an owner
    /// that is not a plain path, an associated type no `impl`
    /// declares, or one two `impl`s declare differently.
    ///
    /// The answer is canonical ([`Self::canonical`]) in the module of
    /// the `impl` that declares it, since that is where its names were
    /// written; two declarations agree when their canonical forms do.
    fn project(&self, owner: &Type, name: &str) -> Option<Type> {
        let Type::Path(owner) = owner else {
            return None;
        };
        let owner = owner.path.segments.last()?.ident.to_string();
        let candidates = self.associated.get(&(owner, name.to_string()))?;
        let mut canonical = candidates
            .iter()
            .map(|(declared, module)| self.canonical(&declared.0, module));
        let first = canonical.next()?;
        let spelled = render(&first);
        canonical.all(|other| render(&other) == spelled).then_some(first)
    }

    /// Every name `ty` mentions in a position that holds data, with
    /// the arguments written on it and what it resolves to in `module`
    /// — [`collect_type_names`]'s answer, minus the arguments of an
    /// indexed generic item: those are bindings for the item's
    /// parameters, followed through the item when it uses them, and
    /// not data at the place they are written.
    fn mentions(&self, ty: &Type, module: &str, out: &mut Vec<Mention>) {
        match ty {
            Type::Path(path) => {
                if let Some(qself) = &path.qself {
                    self.mentions(&qself.ty, module, out);
                }
                let segments: Vec<String> = path
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect();
                let rooted = segments
                    .first()
                    .is_some_and(|first| matches!(first.as_str(), "crate" | "self" | "super"));
                for (at, segment) in path.path.segments.iter().enumerate() {
                    let last = at + 1 == segments.len();
                    if rooted && !last {
                        // A module, by the language's own rule.
                        continue;
                    }
                    let resolution = if last {
                        self.resolve(module, &segments)
                    } else {
                        self.resolve(module, &segments[at..=at])
                    };
                    let binds = match &resolution {
                        Resolution::Found(key) => {
                            self.items.get(key).is_some_and(|info| !info.generics.is_empty())
                        }
                        _ => false,
                    };
                    let mut arguments = Vec::new();
                    match &segment.arguments {
                        PathArguments::AngleBracketed(args) => {
                            for arg in &args.args {
                                if let GenericArgument::Type(inner) = arg {
                                    if binds {
                                        arguments.push(inner.clone());
                                    } else {
                                        self.mentions(inner, module, out);
                                    }
                                }
                            }
                        }
                        PathArguments::Parenthesized(args) => {
                            for inner in &args.inputs {
                                self.mentions(&inner.ty, module, out);
                            }
                        }
                        PathArguments::None => {}
                    }
                    out.push(Mention {
                        name: segments[at].clone(),
                        resolution,
                        arguments,
                    });
                }
            }
            Type::Reference(inner) => self.mentions(&inner.elem, module, out),
            Type::Slice(inner) => self.mentions(&inner.elem, module, out),
            Type::Array(inner) => self.mentions(&inner.elem, module, out),
            Type::Paren(inner) => self.mentions(&inner.elem, module, out),
            Type::Group(inner) => self.mentions(&inner.elem, module, out),
            Type::Ptr(inner) => self.mentions(&inner.elem, module, out),
            Type::Tuple(tuple) => {
                for inner in &tuple.elems {
                    self.mentions(inner, module, out);
                }
            }
            // The unreadable forms keep their sentinels — see
            // [`collect_type_names`], whose arm this mirrors. A
            // sentinel is never an indexed name.
            other => {
                let mut names = Vec::new();
                collect_type_names(other, &mut names);
                out.extend(names.into_iter().map(|name| Mention {
                    name,
                    resolution: Resolution::NotIndexed,
                    arguments: Vec::new(),
                }));
            }
        }
    }

    /// The proxy a `#[serde(from = "…")]` names, written in `module`,
    /// as a mention: the string is a path, resolved like one.
    fn mention_of_proxy(&self, proxy: &str, module: &str) -> Mention {
        let segments: Vec<String> = proxy
            .split("::")
            .map(|segment| segment.trim().to_string())
            .collect();
        Mention {
            name: segments.last().cloned().unwrap_or_default(),
            resolution: self.resolve(module, &segments),
            arguments: Vec::new(),
        }
    }

    /// What the path `segments`, written in `module`, names — the way
    /// the compiler reads it, as far as the index can follow:
    ///
    /// - a `crate::` / `self::` / `super::` path, or one whose first
    ///   segment is a child module or a module the scope imports, is
    ///   looked up in the module it leads to;
    /// - a single name is looked up in `module` itself: what it
    ///   declares, then what it imports by name (renames, groups and
    ///   re-exports followed to the item), then what its globs bring
    ///   in;
    /// - a path whose first segment is none of those, or a name the
    ///   scope imports from another crate, is not ours.
    ///
    /// What the scope does not settle falls back to the bare name: the
    /// one item declared under it, or [`Resolution::Ambiguous`] when
    /// there are several. That fallback is what the bare-name index
    /// always did, and it is what keeps a gap in this reader from
    /// dropping a type out of the walk; the ambiguous case is reported
    /// ([`Self::ambiguous_mentions`]) rather than guessed.
    fn resolve(&self, module: &str, segments: &[String]) -> Resolution {
        let Some((name, qualifiers)) = segments.split_last() else {
            return Resolution::NotIndexed;
        };
        // The scope first, whatever the name: `use a::Anchor as Pin`
        // makes `Pin` a name no item is declared under.
        let scoped = if qualifiers.is_empty() {
            self.lookup(module, name, 0)
        } else {
            match self.module_path(module, qualifiers) {
                Some(base) => self.lookup(&base, name, 0),
                None => Lookup::Foreign,
            }
        };
        match scoped {
            Lookup::Found(key) => Resolution::Found(key),
            Lookup::Foreign => Resolution::NotIndexed,
            Lookup::Missing => match self.by_name.get(name).map(Vec::as_slice) {
                None | Some([]) => Resolution::NotIndexed,
                Some([only]) => Resolution::Found(only.clone()),
                Some(several) => Resolution::Ambiguous(several.to_vec()),
            },
        }
    }

    /// `name` as `module`'s scope sees it: an item `module` declares,
    /// a name it imports (followed to the item, through re-exports),
    /// or one exactly one of its globs brings in.
    fn lookup(&self, module: &str, name: &str, depth: usize) -> Lookup {
        if depth > LOOKUP_DEPTH {
            return Lookup::Missing;
        }
        let key = join_path(module, name);
        if self.items.contains_key(&key) {
            return Lookup::Found(key);
        }
        let Some(scope) = self.scopes.get(module) else {
            return Lookup::Missing;
        };
        if let Some(target) = scope.imports.get(name) {
            let Some((imported, qualifiers)) = target.split_last() else {
                return Lookup::Missing;
            };
            return match self.module_path(module, qualifiers) {
                Some(base) => self.lookup(&base, imported, depth + 1),
                None => Lookup::Foreign,
            };
        }
        let mut found = BTreeSet::new();
        for glob in &scope.globs {
            if let Some(base) = self.module_path(module, glob) {
                if let Lookup::Found(key) = self.lookup(&base, name, depth + 1) {
                    found.insert(key);
                }
            }
        }
        let mut found = found.into_iter();
        match (found.next(), found.next()) {
            (Some(only), None) => Lookup::Found(only),
            _ => Lookup::Missing,
        }
    }

    /// The module the qualifiers of a path written in `module` lead to,
    /// or `None` when they start outside this crate. An empty list
    /// leads to `module` itself.
    fn module_path(&self, module: &str, qualifiers: &[String]) -> Option<String> {
        self.module_path_at(module, qualifiers, 0)
    }

    fn module_path_at(&self, module: &str, qualifiers: &[String], depth: usize) -> Option<String> {
        if depth > LOOKUP_DEPTH {
            return None;
        }
        let Some((first, rest)) = qualifiers.split_first() else {
            return Some(module.to_string());
        };
        let mut base: Vec<&str> = if module.is_empty() {
            Vec::new()
        } else {
            module.split("::").collect()
        };
        match first.as_str() {
            "crate" => base.clear(),
            "self" => {}
            "super" => {
                base.pop()?;
            }
            _ => {
                let child = join_path(module, first);
                if self.modules.contains(&child) {
                    base.push(first.as_str());
                } else {
                    let imported = self
                        .scopes
                        .get(module)
                        .and_then(|scope| scope.imports.get(first.as_str()))?;
                    let resolved = self.module_path_at(module, imported, depth + 1)?;
                    let relative: Vec<String> = std::iter::once("self".to_string())
                        .chain(rest.iter().cloned())
                        .collect();
                    return self.module_path_at(&resolved, &relative, depth + 1);
                }
            }
        }
        for segment in rest {
            match segment.as_str() {
                "super" => {
                    base.pop()?;
                }
                "self" => {}
                other => base.push(other),
            }
        }
        Some(base.join("::"))
    }

    /// `ty` with every name in it that `module`'s scope resolves to an
    /// indexed item rewritten to that item's full `crate::` path, so it
    /// means the same thing wherever it is substituted. The arguments
    /// on the item's own segment are kept, canonical in turn; a name
    /// that does not resolve is left as written.
    fn canonical(&self, ty: &Type, module: &str) -> Type {
        let mut out = ty.clone();
        self.canonicalize_in_place(&mut out, module);
        out
    }

    fn canonicalize_in_place(&self, ty: &mut Type, module: &str) {
        match ty {
            Type::Path(path) => {
                if let Some(qself) = &mut path.qself {
                    self.canonicalize_in_place(&mut qself.ty, module);
                    return;
                }
                for segment in path.path.segments.iter_mut() {
                    if let PathArguments::AngleBracketed(arguments) = &mut segment.arguments {
                        for argument in arguments.args.iter_mut() {
                            if let GenericArgument::Type(inner) = argument {
                                self.canonicalize_in_place(inner, module);
                            }
                        }
                    }
                }
                let segments: Vec<String> = path
                    .path
                    .segments
                    .iter()
                    .map(|segment| segment.ident.to_string())
                    .collect();
                let Resolution::Found(key) = self.resolve(module, &segments) else {
                    return;
                };
                let arguments = path
                    .path
                    .segments
                    .last()
                    .map(|segment| segment.arguments.clone())
                    .unwrap_or(PathArguments::None);
                let mut rewritten = syn::Path {
                    leading_colon: None,
                    segments: syn::punctuated::Punctuated::new(),
                };
                for segment in std::iter::once("crate").chain(key.split("::")) {
                    let ident = syn::Ident::new(segment, proc_macro2::Span::call_site());
                    rewritten.segments.push(syn::PathSegment::from(ident));
                }
                if let Some(last) = rewritten.segments.last_mut() {
                    last.arguments = arguments;
                }
                path.path = rewritten;
            }
            Type::Reference(inner) => self.canonicalize_in_place(&mut inner.elem, module),
            Type::Slice(inner) => self.canonicalize_in_place(&mut inner.elem, module),
            Type::Array(inner) => self.canonicalize_in_place(&mut inner.elem, module),
            Type::Paren(inner) => self.canonicalize_in_place(&mut inner.elem, module),
            Type::Group(inner) => self.canonicalize_in_place(&mut inner.elem, module),
            Type::Ptr(inner) => self.canonicalize_in_place(&mut inner.elem, module),
            Type::Tuple(tuple) => {
                for inner in tuple.elems.iter_mut() {
                    self.canonicalize_in_place(inner, module);
                }
            }
            _ => {}
        }
    }

    fn index_items(&mut self, items: &[Item], file: &Path, module: &str) {
        for item in items {
            match item {
                Item::Struct(item) => {
                    let serde = SerdeAttrs::read(&item.attrs);
                    let names = SerdeNames::read(&item.attrs);
                    let owner = item.ident.to_string();
                    self.insert(TypeInfo {
                        fields: serde_fields(names.rule(&owner), None, &item.fields),
                        path: join_path(module, &owner),
                        module: module.to_string(),
                        name: owner.clone(),
                        file: file.to_path_buf(),
                        kind: TypeKind::Struct,
                        derives_deserialize: derives_deserialize(&item.attrs),
                        denies_unknown_fields: serde.deny_unknown_fields,
                        deserialize_proxy: serde.proxy,
                        untagged: serde.untagged,
                        internally_tagged: serde.internally_tagged,
                        flattened_fields: flattened_fields(&item.fields),
                        has_named_fields: matches!(item.fields, Fields::Named(_)),
                        variants: Vec::new(),
                        generics: type_parameters(&item.generics),
                        referenced: referenced_types(&item.fields),
                        unread_serde: unread_serde_attrs_with_fields(&item.attrs, &item.fields),
                        has_serde_container_attr: has_serde_container_attr(&item.attrs),
                    });
                }
                Item::Enum(item) => {
                    let serde = SerdeAttrs::read(&item.attrs);
                    let names = SerdeNames::read(&item.attrs);
                    let owner = item.ident.to_string();
                    let mut referenced = Vec::new();
                    let mut flattened = Vec::new();
                    let mut has_named_fields = false;
                    let mut variants = Vec::new();
                    let mut fields = Vec::new();
                    let mut unread = unread_serde_attrs(&item.attrs);
                    for variant in &item.variants {
                        // A `#[serde(skip)]` / `skip_deserializing`
                        // variant is never read: it has no name on
                        // disk (`serde_derive` leaves it out of
                        // `VARIANTS`) and nothing in it is reached.
                        if is_skipped(&variant.attrs) {
                            continue;
                        }
                        has_named_fields |= matches!(variant.fields, Fields::Named(_));
                        referenced.extend(referenced_types(&variant.fields));
                        flattened.extend(flattened_fields(&variant.fields));
                        unread.extend(unread_serde_attrs_with_fields(&variant.attrs, &variant.fields));
                        // A variant renames two separate things, under
                        // two separate options: its own JSON name (its
                        // `rename`, else the container's `rename_all`),
                        // and the names of the fields inside it (its
                        // own `rename_all`, else the container's
                        // `rename_all_fields`).
                        let variant_names = SerdeNames::read(&variant.attrs);
                        let variant_member = variant_names.rename.clone().unwrap_or_else(|| {
                            let ident = variant.ident.to_string();
                            names
                                .rule(&owner)
                                .map_or(ident.clone(), |rule| rule.apply_to_variant(&ident))
                        });
                        let inner_rule = variant_names.rule(&owner).or_else(|| names.field_rule(&owner));
                        fields.extend(serde_fields(
                            inner_rule,
                            Some((variant_member.as_str(), variant_names.aliases.as_slice())),
                            &variant.fields,
                        ));
                        // Sorted inside the variant's own group and
                        // appended in source order, which is how
                        // `serde_derive` lays out `VARIANTS`: it
                        // flat-maps a `BTreeSet` per variant, and the
                        // set holds the resolved name alongside every
                        // alias.
                        let mut group: BTreeSet<String> = variant_names.aliases.into_iter().collect();
                        group.insert(variant_member);
                        variants.extend(group);
                    }
                    self.insert(TypeInfo {
                        fields,
                        path: join_path(module, &owner),
                        module: module.to_string(),
                        name: owner,
                        file: file.to_path_buf(),
                        kind: TypeKind::Enum,
                        derives_deserialize: derives_deserialize(&item.attrs),
                        denies_unknown_fields: serde.deny_unknown_fields,
                        deserialize_proxy: serde.proxy,
                        untagged: serde.untagged,
                        internally_tagged: serde.internally_tagged,
                        flattened_fields: flattened,
                        has_named_fields,
                        variants,
                        generics: type_parameters(&item.generics),
                        referenced,
                        unread_serde: unread,
                        has_serde_container_attr: has_serde_container_attr(&item.attrs),
                    });
                }
                Item::Type(item) => {
                    let referenced = vec![DeclaredType((*item.ty).clone())];
                    self.insert(TypeInfo {
                        fields: Vec::new(),
                        path: join_path(module, &item.ident.to_string()),
                        module: module.to_string(),
                        name: item.ident.to_string(),
                        file: file.to_path_buf(),
                        kind: TypeKind::Alias,
                        derives_deserialize: false,
                        denies_unknown_fields: false,
                        deserialize_proxy: None,
                        untagged: false,
                        internally_tagged: false,
                        flattened_fields: Vec::new(),
                        has_named_fields: false,
                        variants: Vec::new(),
                        generics: type_parameters(&item.generics),
                        referenced,
                        unread_serde: unread_serde_attrs(&item.attrs),
                        has_serde_container_attr: has_serde_container_attr(&item.attrs),
                    });
                }
                Item::Impl(item) => {
                    // An `impl` declares no type of its own; what it
                    // contributes is the associated types a
                    // projection through a bound parameter resolves to.
                    if is_test_gated(&item.attrs) {
                        continue;
                    }
                    let Type::Path(owner) = &*item.self_ty else {
                        continue;
                    };
                    let Some(owner) = owner.path.segments.last() else {
                        continue;
                    };
                    for member in &item.items {
                        if let syn::ImplItem::Type(associated) = member {
                            self.associated
                                .entry((owner.ident.to_string(), associated.ident.to_string()))
                                .or_default()
                                .push((DeclaredType(associated.ty.clone()), module.to_string()));
                        }
                    }
                }
                Item::Mod(item) => {
                    if is_test_gated(&item.attrs) {
                        continue;
                    }
                    if let Some((_, inner)) = &item.content {
                        let inner_module = join_path(module, &item.ident.to_string());
                        self.modules.insert(inner_module.clone());
                        self.index_items(inner, file, &inner_module);
                    }
                }
                Item::Use(item) => {
                    if is_test_gated(&item.attrs) {
                        continue;
                    }
                    let scope = self.scopes.entry(module.to_string()).or_default();
                    record_use(&item.tree, &mut Vec::new(), scope);
                }
                _ => {}
            }
        }
    }

    fn insert(&mut self, info: TypeInfo) {
        if self.items.contains_key(&info.path) {
            self.duplicates.insert(info.path.clone());
            return;
        }
        self.by_name
            .entry(info.name.clone())
            .or_default()
            .push(info.path.clone());
        self.items.insert(info.path.clone(), info);
    }
}

/// `module::name`, or `name` alone in the crate root.
fn join_path(module: &str, name: &str) -> String {
    if module.is_empty() {
        name.to_string()
    } else {
        format!("{module}::{name}")
    }
}

/// The module a source file is, read off its path under `src_root`:
/// `a/b.rs` and `a/b/mod.rs` are `a::b`, and `lib.rs` / `main.rs` at
/// the root are the crate root.
fn module_of_file(src_root: &Path, file: &Path) -> String {
    let relative = file.strip_prefix(src_root).unwrap_or(file);
    let mut segments: Vec<String> = relative
        .components()
        .map(|component| component.as_os_str().to_string_lossy().into_owned())
        .collect();
    if let Some(last) = segments.pop() {
        let stem = last.strip_suffix(".rs").unwrap_or(&last).to_string();
        let root_file = segments.is_empty() && (stem == "lib" || stem == "main");
        if stem != "mod" && !root_file {
            segments.push(stem);
        }
    }
    segments.join("::")
}

/// Record every name one `use` tree imports into `scope`, with
/// `prefix` the segments above this subtree.
fn record_use(tree: &syn::UseTree, prefix: &mut Vec<String>, scope: &mut Scope) {
    match tree {
        syn::UseTree::Path(path) => {
            prefix.push(path.ident.to_string());
            record_use(&path.tree, prefix, scope);
            prefix.pop();
        }
        syn::UseTree::Name(name) => {
            let ident = name.ident.to_string();
            if ident == "self" {
                // `use a::{self}` imports the module `a` under its name.
                if let Some(module) = prefix.last().cloned() {
                    scope.imports.insert(module, prefix.clone());
                }
            } else {
                let mut target = prefix.clone();
                target.push(ident.clone());
                scope.imports.insert(ident, target);
            }
        }
        syn::UseTree::Rename(rename) => {
            let local = rename.rename.to_string();
            if local == "_" {
                return;
            }
            let mut target = prefix.clone();
            if rename.ident != "self" {
                target.push(rename.ident.to_string());
            }
            scope.imports.insert(local, target);
        }
        syn::UseTree::Glob(_) => scope.globs.push(prefix.clone()),
        syn::UseTree::Group(group) => {
            for inner in &group.items {
                record_use(inner, prefix, scope);
            }
        }
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

/// Every `#[serde(...)]` attribute in `attrs` that this module's
/// reader cannot consume to the end, as `"<attribute>: <error>"`.
///
/// The fail-safe half of the parser, and the reason the polarity of
/// the guard test above it is survivable. [`SerdeAttrs::read`],
/// [`flattened_fields`] and [`is_skipped`] all
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
            out.push(format!(
                "{}: {e} — unread from that point on, so the flags written after it \
                 were never seen. Teach `skip_unmodeled_option` the shape rather than \
                 deleting the entry.",
                attribute_source(attr)
            ));
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

/// The declared types of the fields serde can populate. A field
/// marked `#[serde(skip)]` / `#[serde(skip_deserializing)]` never
/// receives on-disk data, so it does not extend the contract.
fn referenced_types(fields: &Fields) -> Vec<DeclaredType> {
    fields
        .iter()
        .filter(|field| !is_skipped(&field.attrs))
        .map(|field| DeclaredType(field.ty.clone()))
        .collect()
}

/// The names of an item's type parameters, in declaration order.
/// Lifetimes and const parameters bind nothing a field type names.
fn type_parameters(generics: &syn::Generics) -> Vec<String> {
    generics
        .type_params()
        .map(|param| param.ident.to_string())
        .collect()
}

/// The `#[serde(rename_all = "…")]` spellings, and what each does to
/// an identifier.
///
/// Serde applies a rule differently to a field and to a variant
/// because their conventional casings differ — `snake_case` is a
/// no-op on a field and a real transform on a variant, and
/// `PascalCase` is the reverse. Both directions are here because both
/// produce member names: a field's, and — for a newtype variant
/// whose payload is an array — the variant's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RenameRule {
    /// `"lowercase"`.
    Lower,
    /// `"UPPERCASE"`.
    Upper,
    /// `"PascalCase"`.
    Pascal,
    /// `"camelCase"`.
    Camel,
    /// `"snake_case"`.
    Snake,
    /// `"SCREAMING_SNAKE_CASE"`.
    ScreamingSnake,
    /// `"kebab-case"`.
    Kebab,
    /// `"SCREAMING-KEBAB-CASE"`.
    ScreamingKebab,
}

impl RenameRule {
    /// The rule serde knows by that name, or `None` when nothing
    /// here matches — which [`SerdeNames::rule`] turns into a stopped
    /// run rather than a shrug.
    fn parse(spelling: &str) -> Option<RenameRule> {
        match spelling {
            "lowercase" => Some(RenameRule::Lower),
            "UPPERCASE" => Some(RenameRule::Upper),
            "PascalCase" => Some(RenameRule::Pascal),
            "camelCase" => Some(RenameRule::Camel),
            "snake_case" => Some(RenameRule::Snake),
            "SCREAMING_SNAKE_CASE" => Some(RenameRule::ScreamingSnake),
            "kebab-case" => Some(RenameRule::Kebab),
            "SCREAMING-KEBAB-CASE" => Some(RenameRule::ScreamingKebab),
            _ => None,
        }
    }

    /// The rule applied to a field identifier, which serde assumes is
    /// already `snake_case`.
    fn apply_to_field(self, ident: &str) -> String {
        match self {
            RenameRule::Lower | RenameRule::Snake => ident.to_string(),
            RenameRule::Upper | RenameRule::ScreamingSnake => ident.to_ascii_uppercase(),
            RenameRule::Pascal => snake_to_pascal(ident),
            RenameRule::Camel => lower_first(&snake_to_pascal(ident)),
            RenameRule::Kebab => ident.replace('_', "-"),
            RenameRule::ScreamingKebab => ident.to_ascii_uppercase().replace('_', "-"),
        }
    }

    /// The rule applied to a variant identifier, which serde assumes
    /// is already `PascalCase`.
    fn apply_to_variant(self, ident: &str) -> String {
        match self {
            RenameRule::Lower => ident.to_ascii_lowercase(),
            RenameRule::Upper => ident.to_ascii_uppercase(),
            RenameRule::Pascal => ident.to_string(),
            RenameRule::Camel => lower_first(ident),
            RenameRule::Snake => pascal_to_snake(ident),
            RenameRule::ScreamingSnake => pascal_to_snake(ident).to_ascii_uppercase(),
            RenameRule::Kebab => pascal_to_snake(ident).replace('_', "-"),
            RenameRule::ScreamingKebab => pascal_to_snake(ident).to_ascii_uppercase().replace('_', "-"),
        }
    }
}

/// `"control_points"` → `"ControlPoints"`.
///
/// **ASCII case, deliberately**, here and in the two helpers below.
/// Serde's own rule reaches for `to_ascii_uppercase` throughout, so
/// `ß_thing` stays `ßThing` for it while Unicode case would make it
/// `SSThing` — a name the loader will never match, produced silently,
/// which is the exact failure #122 is about. What the case *decides*
/// stays Unicode-aware where serde's does: `pascal_to_snake` asks
/// `char::is_uppercase`, not `is_ascii_uppercase`.
fn snake_to_pascal(ident: &str) -> String {
    let mut out = String::with_capacity(ident.len());
    let mut capitalize = true;
    for character in ident.chars() {
        if character == '_' {
            capitalize = true;
        } else if capitalize {
            out.push(character.to_ascii_uppercase());
            capitalize = false;
        } else {
            out.push(character);
        }
    }
    out
}

/// `"ControlPoints"` → `"control_points"`. Serde's own rule inserts a
/// separator before every uppercase letter but the first, so a run of
/// capitals becomes one underscore per letter — matching it matters
/// more than reading well.
fn pascal_to_snake(ident: &str) -> String {
    let mut out = String::with_capacity(ident.len());
    for (at, character) in ident.chars().enumerate() {
        if character.is_uppercase() && at > 0 {
            out.push('_');
        }
        out.push(character.to_ascii_lowercase());
    }
    out
}

/// `"ControlPoints"` → `"controlPoints"`, lowercasing the first
/// character only.
///
/// Serde slices the first *byte* rather than the first character, so
/// on an identifier starting outside ASCII its own macro panics
/// before this walk could disagree with it. Taking the character is
/// the same answer everywhere the two can both produce one.
fn lower_first(ident: &str) -> String {
    let mut chars = ident.chars();
    match chars.next() {
        Some(first) => std::iter::once(first.to_ascii_lowercase()).chain(chars).collect(),
        None => String::new(),
    }
}

/// The `rename` / `rename_all` / `alias` names a `#[serde(...)]` list
/// carries, read for the **deserialize** side.
///
/// A captured route is recorded against the shape that was read, so
/// where serde lets the two sides differ — `rename(serialize = "a",
/// deserialize = "b")` — only the `deserialize` arm is a member name
/// this walk has any business publishing.
#[derive(Debug, Default)]
struct SerdeNames {
    /// `#[serde(rename = "…")]` or the `deserialize` arm of
    /// `#[serde(rename(…))]`.
    rename: Option<String>,
    /// `#[serde(rename_all = "…")]`, or its `deserialize` arm, as
    /// written — resolved to a [`RenameRule`] by the caller so an
    /// unknown spelling stays visible.
    rename_all: Option<String>,
    /// `#[serde(rename_all_fields = "…")]` on an enum, which renames
    /// the fields of its struct variants rather than its variants.
    rename_all_fields: Option<String>,
    /// Every `#[serde(alias = "…")]`, in source order.
    aliases: Vec<String>,
}

impl SerdeNames {
    /// Read the naming options out of `attrs`.
    ///
    /// The `deserialize` arm of a list-form option is picked out
    /// rather than the list being skipped whole — reading only
    /// `rename = "…"` is what published `control` for a field the
    /// file calls `points`. Everything else is consumed by
    /// [`skip_unmodeled_option`] so the options written after it are
    /// still seen, and an attribute this cannot finish is reported by
    /// [`unread_serde_attrs`] exactly as before.
    fn read(attrs: &[Attribute]) -> Self {
        let mut out = SerdeNames::default();
        for attr in attrs {
            if !attr.path().is_ident("serde") {
                continue;
            }
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("rename") {
                    out.rename = read_deserialize_arm(&meta)?;
                } else if meta.path.is_ident("rename_all") {
                    out.rename_all = read_deserialize_arm(&meta)?;
                } else if meta.path.is_ident("rename_all_fields") {
                    out.rename_all_fields = read_deserialize_arm(&meta)?;
                } else if meta.path.is_ident("alias") {
                    let literal: syn::LitStr = meta.value()?.parse()?;
                    out.aliases.push(literal.value());
                } else {
                    skip_unmodeled_option(&meta)?;
                }
                Ok(())
            });
        }
        out
    }

    /// The [`RenameRule`] named by `rename_all`, or `None` when the
    /// container names none.
    ///
    /// **Panics on a spelling this walk does not know**, naming
    /// `context`. Returning `None` there would look like "no rename"
    /// and publish the Rust identifier under a name no file
    /// contains — a silent wrong answer, which is the whole subject
    /// of #122. Serde has eight rules and a ninth would be a
    /// dependency bump somebody is already reading a changelog for;
    /// stopping by name is the same posture
    /// [`crate::util::source_scan`] takes toward a `mod` it cannot
    /// follow.
    fn rule(&self, context: &str) -> Option<RenameRule> {
        Self::resolve(self.rename_all.as_deref(), "rename_all", context)
    }

    /// The [`RenameRule`] named by `rename_all_fields`; see
    /// [`Self::rule`], including the panic.
    fn field_rule(&self, context: &str) -> Option<RenameRule> {
        Self::resolve(self.rename_all_fields.as_deref(), "rename_all_fields", context)
    }

    fn resolve(spelling: Option<&str>, option: &str, context: &str) -> Option<RenameRule> {
        let spelling = spelling?;
        Some(RenameRule::parse(spelling).unwrap_or_else(|| {
            panic!(
                "{context}: `#[serde({option} = \"{spelling}\")]` names a rule this walk \
                 does not know, so it cannot say what the fields are called on disk. \
                 Teach `RenameRule` the spelling — answering \"no rename\" here would \
                 publish the Rust identifiers under names no document contains."
            )
        }))
    }
}

/// The value of an option written either as `name = "x"` or as
/// `name(serialize = "x", deserialize = "y")`, taking the
/// `deserialize` arm of the list form and nothing else.
fn read_deserialize_arm(meta: &syn::meta::ParseNestedMeta) -> syn::Result<Option<String>> {
    if meta.input.peek(syn::Token![=]) {
        let literal: syn::LitStr = meta.value()?.parse()?;
        return Ok(Some(literal.value()));
    }
    if !meta.input.peek(syn::token::Paren) {
        return Ok(None);
    }
    let mut read = None;
    meta.parse_nested_meta(|nested| {
        if nested.path.is_ident("deserialize") {
            let literal: syn::LitStr = nested.value()?.parse()?;
            read = Some(literal.value());
        } else {
            skip_unmodeled_option(&nested)?;
        }
        Ok(())
    })?;
    Ok(read)
}

/// Every place a deserializer can put on-disk data into a struct's
/// fields, or into one variant's.
///
/// `rule` is the container's `rename_all` as it applies to *these* fields;
/// `payload` is the JSON name of the variant that wraps them together
/// with every alias spelling that variant accepts, present only for
/// an enum variant.
fn serde_fields(
    rule: Option<RenameRule>,
    payload: Option<(&str, &[String])>,
    fields: &Fields,
) -> Vec<SerdeField> {
    let mut out = Vec::new();
    let unnamed_arity = match fields {
        Fields::Unnamed(unnamed) => unnamed.unnamed.len(),
        _ => 0,
    };
    for field in fields {
        if is_skipped(&field.attrs) {
            continue;
        }
        let names = SerdeNames::read(&field.attrs);
        let member = match field.ident.as_ref() {
            Some(ident) => Some(names.rename.clone().unwrap_or_else(|| {
                let ident = ident.to_string();
                rule.map_or(ident.clone(), |rule| rule.apply_to_field(&ident))
            })),
            // A newtype's single unnamed field carries no name of its
            // own: a newtype *variant* is written as the one member
            // of a wrapper object and takes the variant's name, and a
            // newtype *struct* is written as its inner value with no
            // wrapper at all, so it is reached through
            // `transparent_target` from whatever names it. Anything
            // wider is a fixed-arity array whose positions no member
            // names.
            None if unnamed_arity == 1 => payload.map(|(member, _)| member.to_string()),
            None => None,
        };
        let kind = if field.ident.is_some() {
            MemberKind::Field
        } else {
            MemberKind::VariantPayload
        };
        // A newtype variant's payload is published under the
        // *variant's* names, so the extra spellings that reach it are
        // the variant's aliases and not the unnamed field's — which
        // serde would have nothing to do with anyway.
        let aliases = match kind {
            MemberKind::Field => names.aliases,
            MemberKind::VariantPayload => payload.map(|(_, aliases)| aliases.to_vec()).unwrap_or_default(),
        };
        // A newtype struct is reached through its parent; emitting it
        // here as well would report the same array twice under two
        // different answers.
        if member.is_none() && field.ident.is_none() && unnamed_arity == 1 {
            continue;
        }
        out.push(SerdeField {
            member,
            kind,
            aliases,
            ty: DeclaredType(field.ty.clone()),
        });
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

/// Stand-in names for the `Type` forms [`collect_type_names`] cannot
/// read the inner types out of. Angle-bracketed so they can never be
/// confused with a Rust identifier, and deliberately absent from
/// [`EXPECTED_TERMINATORS`] so the first one to appear on the load
/// graph goes red rather than quiet.
const UNREADABLE_DYN: &str = "<dyn Trait>";
/// See [`UNREADABLE_DYN`].
const UNREADABLE_IMPL: &str = "<impl Trait>";
/// See [`UNREADABLE_DYN`].
const UNREADABLE_MACRO: &str = "<macro-generated type>";
/// See [`UNREADABLE_DYN`]. The catch-all: three forms were named and
/// the arm under them still swallowed everything else `syn::Type` can
/// be, which is the shape of the defect the other three close.
const UNREADABLE_OTHER: &str = "<unreadable type form>";

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
                            collect_type_names(&inner.ty, out);
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
        _ => out.push(UNREADABLE_OTHER.to_string()),
    }
}

/// The items a walk reached, each once and in the order first met —
/// the per-item view of a per-instantiation walk, for the questions
/// that are about an item's own attributes and members rather than
/// about what its parameters were bound to.
fn distinct_items(reached: Vec<Reached<'_>>) -> Vec<&TypeInfo> {
    let mut seen: BTreeSet<&str> = BTreeSet::new();
    reached
        .into_iter()
        .filter(|entry| seen.insert(entry.info.path.as_str()))
        .map(|entry| entry.info)
        .collect()
}

/// The identity of one instantiation: the item's path and what each
/// parameter is bound to, in parameter-name order.
fn instantiation_key(path: &str, bindings: &Bindings) -> String {
    let mut key = path.to_string();
    for (parameter, bound) in bindings {
        key.push_str(&format!(" {parameter}={}", render(bound)));
    }
    key
}

/// `ty` as canonical text — enough to tell two types apart and to
/// tell two spellings of one type alike. Not source: a lifetime or a
/// const argument renders as `_`, and an array's length is not kept.
fn render(ty: &Type) -> String {
    let mut out = String::new();
    render_into(ty, &mut out);
    out
}

fn render_into(ty: &Type, out: &mut String) {
    match ty {
        Type::Path(path) => {
            if let Some(qself) = &path.qself {
                out.push('<');
                render_into(&qself.ty, out);
                out.push('>');
            }
            for (index, segment) in path.path.segments.iter().enumerate() {
                if index > 0 || path.path.leading_colon.is_some() {
                    out.push_str("::");
                }
                out.push_str(&segment.ident.to_string());
                match &segment.arguments {
                    PathArguments::AngleBracketed(arguments) => {
                        out.push('<');
                        for argument in &arguments.args {
                            match argument {
                                GenericArgument::Type(inner) => render_into(inner, out),
                                _ => out.push('_'),
                            }
                            out.push(',');
                        }
                        out.push('>');
                    }
                    PathArguments::Parenthesized(arguments) => {
                        out.push('(');
                        for inner in &arguments.inputs {
                            render_into(&inner.ty, out);
                            out.push(',');
                        }
                        out.push(')');
                    }
                    PathArguments::None => {}
                }
            }
        }
        Type::Reference(inner) => {
            out.push('&');
            render_into(&inner.elem, out);
        }
        Type::Slice(inner) => {
            out.push('[');
            render_into(&inner.elem, out);
            out.push(']');
        }
        Type::Array(inner) => {
            out.push('[');
            render_into(&inner.elem, out);
            out.push_str(";_]");
        }
        Type::Paren(inner) => render_into(&inner.elem, out),
        Type::Group(inner) => render_into(&inner.elem, out),
        Type::Ptr(inner) => {
            out.push('*');
            render_into(&inner.elem, out);
        }
        Type::Tuple(tuple) => {
            out.push('(');
            for inner in &tuple.elems {
                render_into(inner, out);
                out.push(',');
            }
            out.push(')');
        }
        other => {
            let mut names = Vec::new();
            collect_type_names(other, &mut names);
            out.push_str(&names.join(" "));
        }
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

    /// The types the format's loader reads a file into: the header and
    /// each node of the list (`document::tests::format_strict_tests`
    /// names the same two). Everything a load can hand a deserializer is
    /// reachable from one of them.
    const FORMAT_ROOTS: [&str; 2] = ["MapHeaderDto", "NodeDto"];

    /// The format's types whose `Deserialize` is written by hand, which
    /// the terminator walk does not enter: `Source`, what a `Rows` holds
    /// — its array's text, read item by item, or the saver's trait
    /// object (`Source::Arena`), which no deserializer is handed. What a
    /// row's items are is followed through `Rows`' `items` marker.
    const HAND_READ: &[&str] = &["Source"];

    /// Every name the walk from [`FORMAT_ROOTS`] stops at, gathered.
    fn format_terminators(graph: &TypeGraph) -> BTreeSet<String> {
        FORMAT_ROOTS
            .iter()
            .flat_map(|root| graph.unresolved_from_excluding(root, HAND_READ))
            .collect()
    }

    /// The exclusion is the hand-read type and nothing wider: walked
    /// into, `Source` stops at the trait object and the raw text the
    /// loader holds, which no derived impl reads; walked around, the
    /// row's items are still reached.
    #[test]
    fn test_the_hand_read_rows_are_walked_around_not_dropped() {
        let graph = TypeGraph::build(&crate_src_root());
        let through = graph.unresolved_from("NodeDto");
        assert!(
            through.contains("<dyn Trait>") && through.contains("RawValue"),
            "control: walked into, the hand-read source stops at the saver's trait object              and the raw text: {through:?}"
        );
        let around = format_terminators(&graph);
        assert!(
            !around.contains("<dyn Trait>") && !around.contains("RawValue"),
            "walked around, neither is a terminator: {around:?}"
        );
        let reached: Vec<&str> = graph
            .reachable_from_excluding("NodeDto", HAND_READ)
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        assert!(
            reached.contains(&"SpanDto"),
            "a row's items are reached around the source: {reached:?}"
        );
    }

    /// Every name the walk from the format's roots is allowed to stop at.
    ///
    /// Two kinds of entry, and the distinction is the whole point of the
    /// list:
    ///
    /// 1. **Primitives and containers** — `String`, `str`, `f32`,
    ///    `Option`, `Vec`, `Cow`, and the `PhantomData` a `Rows` names its
    ///    items with. No fields of ours; stopping is correct and always
    ///    will be.
    /// 2. **Foreign types** — none today. A generic parameter is not on
    ///    the list: the walk follows it as what the reference bound it to
    ///    (`TypeInfo::generics`), so one that reaches this list was left
    ///    unbound. A foreign type's shape is not ours to annotate; if one
    ///    ever lands on the load path, adding it here is a review
    ///    decision, not a silent one. The walk's sentinels for a form it
    ///    cannot read (`<dyn Trait>` and the like) are never listed: see
    ///    [`HAND_READ`] for the one the format holds, walked around.
    ///
    /// A type a build script emits into `$OUT_DIR` would be a third kind:
    /// the module's promise — a new field of a new type extends the
    /// covered set on its own — is *false* for it. The format names
    /// fonts by family string, so none is reachable today, and the list
    /// is what says so the day one is.
    const EXPECTED_TERMINATORS: &[&str] = &[
        // 1. primitives and containers
        "Cow",
        "Option",
        "PhantomData",
        "String",
        "Vec",
        "bool",
        "f32",
        "str",
        "u16",
        "u32",
        "u64",
        "u8",
    ];

    /// **The walker's blind spot, made visible.** The walk stops at
    /// any name it did not index, and it cannot tell "known-terminal
    /// primitive" from "type I failed to resolve" — a type in `$OUT_DIR`
    /// reachable from a real map would look exactly like `String`.
    ///
    /// So the list is here rather than in a comment: every terminator
    /// must be one somebody looked at. When a field of a new unindexed
    /// type lands, this fails naming it, and the reader decides whether
    /// it belongs on the list or whether the walk has to learn to reach
    /// it. That is the same drift discipline
    /// `format_strict_tests::test_every_type_the_format_reads_denies_unknown_fields`
    /// applies to the types the walk *does* see.
    #[test]
    fn test_every_walk_terminator_is_an_expected_one() {
        let graph = TypeGraph::build(&crate_src_root());
        let unexpected: Vec<String> = format_terminators(&graph)
            .into_iter()
            .filter(|name| !EXPECTED_TERMINATORS.contains(&name.as_str()))
            .collect();
        assert!(
            unexpected.is_empty(),
            "the reachability walk stops at {} name(s) nobody has vetted. Each is \
             either a primitive (add it to EXPECTED_TERMINATORS) or a type that is \
             part of the on-disk contract and is silently missing from the \
             strictness coverage:\n  {}",
            unexpected.len(),
            unexpected.join("\n  ")
        );
    }

    /// The allowlist itself must not rot: an entry for a name the walk
    /// no longer reaches is a claim about the format that stopped being
    /// true, and it would keep covering for a *different* name that
    /// happened to reappear later.
    #[test]
    fn test_no_expected_terminator_is_stale() {
        let graph = TypeGraph::build(&crate_src_root());
        let actual = format_terminators(&graph);
        let stale: Vec<&&str> = EXPECTED_TERMINATORS
            .iter()
            .filter(|name| !actual.contains(**name))
            .collect();
        assert!(
            stale.is_empty(),
            "EXPECTED_TERMINATORS lists name(s) the walk from the format's roots no \
             longer stops at — delete them: {stale:?}"
        );
    }

    /// The format's types live in one module: every type a load can
    /// reach is declared under `document/format/`, so a name another
    /// module declares can never be the one the walk resolves — the
    /// format's types carry a `Dto` suffix for exactly that reason.
    #[test]
    fn test_the_format_is_walked_inside_its_own_module() {
        let graph = TypeGraph::build(&crate_src_root());
        let format = crate_src_root().join("document").join("format");
        for root in FORMAT_ROOTS {
            assert!(graph.get(root).is_some(), "{root} is indexed");
            for reached in graph.reachable_from(root) {
                assert!(
                    reached.file.starts_with(&format),
                    "{} is reached from {root} and declared outside the format: {}",
                    reached.name,
                    reached.file.display()
                );
            }
        }
    }

    /// The walk is only as good as its index: a path declared twice
    /// would make "which `Position`?" unanswerable, and so would a
    /// mention of a name two modules declare that the scope it is
    /// written in does not settle — the coverage assertion built on
    /// top would be checking an arbitrary one.
    #[test]
    fn test_type_graph_has_no_ambiguous_names() {
        let graph = TypeGraph::build(&crate_src_root());
        assert!(
            graph.duplicate_names().is_empty(),
            "paths declared more than once — no scope can tell them apart: {:?}",
            graph.duplicate_names()
        );
        let ambiguous = graph.ambiguous_mentions();
        assert!(
            ambiguous.is_empty(),
            "names more than one module declares, mentioned where the scope does not \
             say which — import the one meant, or write its path:\n  {}",
            ambiguous.join("\n  ")
        );
    }

    /// **Every serialized document type is in the index, under its
    /// path** — the two files the bare-name index had to leave out
    /// (`document/element.rs` and `document/values.rs`) included, and
    /// each name they share with another module (`Anchor`, `Rgba`,
    /// `Direction`, `Edge`) held apart from its namesake.
    ///
    /// The list is read off the sources here, independently of the
    /// index: every struct and enum under `document/` whose derive list
    /// names `Serialize` or `Deserialize`.
    #[test]
    fn test_every_serialized_document_type_is_indexed_under_its_path() {
        let root = crate_src_root();
        let graph = TypeGraph::build(&root);
        let mut files = Vec::new();
        collect_source_files(&root.join("document"), &mut files);
        let test_only = test_gated_module_files(&files);
        files.retain(|file| !test_only.contains(file));
        let mut checked = Vec::new();
        for file in &files {
            let text = std::fs::read_to_string(file).expect("readable");
            let parsed = syn::parse_file(&text).expect("parses");
            let module = module_of_file(&root, file);
            for item in &parsed.items {
                let (ident, attrs) = match item {
                    Item::Struct(item) => (&item.ident, &item.attrs),
                    Item::Enum(item) => (&item.ident, &item.attrs),
                    _ => continue,
                };
                let serialized = attrs
                    .iter()
                    .filter(|attr| attr.path().is_ident("derive"))
                    .any(|attr| {
                        let derives = attribute_source(attr);
                        derives.contains("Serialize") || derives.contains("Deserialize")
                    });
                if !serialized {
                    continue;
                }
                let path = join_path(&module, &ident.to_string());
                let info = graph
                    .get(&path)
                    .unwrap_or_else(|| panic!("{path} ({}) is serialized and not indexed", file.display()));
                assert_eq!(&info.file, file, "{path} is indexed from its own file");
                checked.push(path);
            }
        }
        for expected in [
            "document::values::Anchor",
            "document::values::Rgba",
            "document::values::Direction",
            "document::values::Edge",
            "document::element::DocKind",
            "document::inspect::DocField",
            "document::format::dto::NodeDto",
        ] {
            assert!(
                checked.iter().any(|path| path == expected),
                "control: {expected} is one of the serialized document types checked: {checked:?}"
            );
        }
        for (document, namesake) in [
            ("document::values::Anchor", "core::primitives::Anchor"),
            ("document::values::Rgba", "util::color::Rgba"),
        ] {
            let ours = graph.get(document).expect("indexed");
            let theirs = graph.get(namesake).expect("the namesake is indexed too");
            assert_ne!(
                ours.file, theirs.file,
                "{document} and {namesake} are two entries"
            );
        }
    }

    /// A document type that mentions a shared name reaches **its own
    /// module's** item: `PathData`'s anchors are the document's
    /// `Anchor` and `ThemeVariant`'s values the document's `Rgba`,
    /// imported by `document/element.rs`, never `core::primitives`'
    /// `Anchor` or `util::color`'s alias.
    #[test]
    fn test_a_shared_name_resolves_to_the_item_its_module_imports() {
        let graph = TypeGraph::build(&crate_src_root());
        for (root, meant, namesake) in [
            (
                "document::element::PathData",
                "document::values::Anchor",
                "core::primitives::Anchor",
            ),
            (
                "document::element::ThemeVariant",
                "document::values::Rgba",
                "util::color::Rgba",
            ),
        ] {
            let reached: Vec<&str> = graph
                .reachable_from(root)
                .iter()
                .map(|info| info.path.as_str())
                .collect();
            assert!(reached.contains(&meant), "{root} reaches {meant}: {reached:?}");
            assert!(
                !reached.contains(&namesake),
                "{root} does not reach {namesake}: {reached:?}"
            );
        }
    }

    /// The resolver over every shape a scope can settle a shared name
    /// with — an import, an explicit path, a glob, a rename, a
    /// re-export — and the one it cannot: a bare mention with neither
    /// an import nor a path, which is reported, not guessed. And a
    /// generic argument means what it meant **where it was written**:
    /// `Holder<Anchor>` written under `a`'s import binds `T` to `a`'s
    /// `Anchor`, though `Holder`'s own module imports `b`'s.
    #[test]
    fn test_a_shared_name_is_resolved_by_the_scope_it_is_written_in() {
        let graph = graph_of(
            r#"
            mod a { pub struct Anchor { pub x: u8 } }
            mod b { pub struct Anchor { pub y: u8 } }
            mod reexport { pub use super::b::Anchor; }
            mod c {
                use super::a::Anchor;
                pub struct Imported { pub anchor: Anchor }
                pub struct Qualified { pub anchor: super::b::Anchor }
                pub struct Rooted { pub anchor: crate::planted::b::Anchor }
                pub struct Uses { pub held: super::h::Holder<Anchor> }
            }
            mod e {
                use super::b::*;
                pub struct Globbed { pub anchor: Anchor }
            }
            mod f {
                use super::a::Anchor as Pin;
                pub struct Renamed { pub anchor: Pin }
            }
            mod g {
                use super::reexport::Anchor;
                pub struct Reexported { pub anchor: Anchor }
            }
            mod h {
                use super::b::Anchor;
                pub struct Holder<T> { pub inner: T, pub own: Anchor }
            }
            mod d {
                pub struct Unsettled { pub anchor: Anchor }
                pub struct UnsettledArgument { pub held: super::h::Holder<Anchor> }
            }
            "#,
        );
        let reaches = |root: &str| -> Vec<String> {
            graph
                .reachable_from(root)
                .iter()
                .map(|info| info.path.clone())
                .collect()
        };
        let a = "planted::a::Anchor".to_string();
        let b = "planted::b::Anchor".to_string();
        for (root, meant) in [
            ("planted::c::Imported", &a),
            ("planted::c::Qualified", &b),
            ("planted::c::Rooted", &b),
            ("planted::e::Globbed", &b),
            ("planted::f::Renamed", &a),
            ("planted::g::Reexported", &b),
        ] {
            let reached = reaches(root);
            let other = if meant == &a { &b } else { &a };
            assert!(reached.contains(meant), "{root} reaches {meant}: {reached:?}");
            assert!(
                !reached.contains(other),
                "{root} does not reach {other}: {reached:?}"
            );
        }
        let uses = reaches("planted::c::Uses");
        assert!(
            uses.contains(&a) && uses.contains(&b),
            "`T` is bound to `a`'s Anchor where it was written, and the holder's own field \
             is `b`'s: {uses:?}"
        );
        let mut ambiguous = graph.ambiguous_mentions();
        ambiguous.sort();
        assert_eq!(
            ambiguous.len(),
            2,
            "control: only the two mentions no scope settles are reported — a field's, and a \
             generic argument's: {ambiguous:?}"
        );
        assert!(
            ambiguous[0].starts_with("Anchor in planted::d::Unsettled "),
            "{ambiguous:?}"
        );
        assert!(
            ambiguous[1].starts_with("Anchor in planted::d::UnsettledArgument "),
            "{ambiguous:?}"
        );
        assert!(
            graph.unresolved_from("planted::d::Unsettled").contains("Anchor"),
            "an unsettled name is a terminator, not a guess"
        );
        let asked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            graph.get("Anchor").map(|info| info.path.clone())
        }));
        assert!(
            asked.is_err(),
            "a bare name two modules declare has no one answer"
        );
    }

    /// A path the scope imports from another crate is not ours, though
    /// an indexed item carries its last name: `serde_json::Value` is
    /// not the planted `Value`.
    #[test]
    fn test_a_foreign_import_never_resolves_to_a_namesake() {
        let graph = graph_of(
            r#"
            pub struct Value { pub inner: u8 }
            mod uses {
                use serde_json::Value;
                pub struct Imported { pub value: Value }
                pub struct Qualified { pub value: serde_json::Value }
                pub struct Local { pub value: super::Value }
            }
            "#,
        );
        for root in ["planted::uses::Imported", "planted::uses::Qualified"] {
            let reached: Vec<&str> = graph
                .reachable_from(root)
                .iter()
                .map(|info| info.path.as_str())
                .collect();
            assert!(!reached.contains(&"planted::Value"), "{root}: {reached:?}");
            assert!(
                graph.unresolved_from(root).contains("Value"),
                "{root} stops at the foreign Value"
            );
        }
        let local: Vec<&str> = graph
            .reachable_from("planted::uses::Local")
            .iter()
            .map(|info| info.path.as_str())
            .collect();
        assert!(
            local.contains(&"planted::Value"),
            "control: the crate's own Value: {local:?}"
        );
    }

    /// A spot check that the walk actually descends rather than
    /// returning its root: `NodeDto` must reach a type two field hops
    /// away (`NodeDto` → `SectionDto` → `SpanDto`), and the header its
    /// theme.
    #[test]
    fn test_reachable_from_follows_nested_fields() {
        let graph = TypeGraph::build(&crate_src_root());
        for (root, expected) in [
            ("NodeDto", ["NodeDto", "SectionDto", "SpanDto"]),
            ("MapHeaderDto", ["MapHeaderDto", "ThemeDto", "DefaultsDto"]),
        ] {
            let reached: Vec<&str> = graph
                .reachable_from(root)
                .iter()
                .map(|info| info.name.as_str())
                .collect();
            for name in expected {
                assert!(reached.contains(&name), "{name} must be reachable from {root}");
            }
        }
    }

    /// `#[serde(from = "...")]` moves the on-disk shape onto the proxy,
    /// so the walk has to follow it — the proxy is named only in that
    /// attribute, never as a field type. Planted, because the format
    /// reads through no proxy today; the control is the same root with
    /// the attribute taken off, which reaches neither.
    #[test]
    fn test_reachable_from_follows_a_serde_proxy() {
        let with_proxy = graph_of(
            "#[derive(serde::Deserialize)]\n#[serde(from = \"WireIn\")]\npub struct Model { pub a: u8 }\n\
             #[derive(serde::Deserialize)]\npub struct WireIn { pub inner: Inner }\n\
             #[derive(serde::Deserialize)]\npub struct Inner { pub b: u8 }\n",
        );
        let reached: Vec<&str> = with_proxy
            .reachable_from("Model")
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        assert!(
            reached.contains(&"WireIn") && reached.contains(&"Inner"),
            "the deserialize proxy and what it holds must be reachable: {reached:?}"
        );
        let without = graph_of(
            "#[derive(serde::Deserialize)]\npub struct Model { pub a: u8 }\n\
             #[derive(serde::Deserialize)]\npub struct WireIn { pub inner: Inner }\n\
             #[derive(serde::Deserialize)]\npub struct Inner { pub b: u8 }\n",
        );
        let reached: Vec<&str> = without
            .reachable_from("Model")
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        assert_eq!(
            reached,
            vec!["Model"],
            "control: with no proxy, nothing else is reached"
        );
    }

    /// Test-only declarations must stay out of the index, or a stub
    /// struct in a test module could shadow a real model type.
    ///
    /// **Both spellings of "test module", because for a long time
    /// only one of them was excluded.** `StubCtx` sits inside an
    /// inline `#[cfg(test)] mod tests { … }`, which the walk sees as
    /// an attribute on an item it is holding. The four types below
    /// are declared by *files* — `util/mod.rs` gates `manifests`,
    /// `serde_coverage`, `bench_surface` and `source_scan` with an
    /// out-of-line `#[cfg(test)] mod x;`, and a walk over the
    /// filesystem never meets that attribute at all. All eleven of
    /// their types were indexed as model types while this test passed
    /// on the strength of the one case it did check.
    ///
    /// The five named here are this module's own and three
    /// independent counterparts', chosen because a rename that
    /// emptied the check would be a rename of something a reader of
    /// this file is already looking at. `serde_probe`'s own `Fields`
    /// and `Variant` would collide with nothing today and everything
    /// tomorrow, which is the case `DerivedShape` stands in for. The
    /// planted case that depends on nothing in the crate is
    /// `source_scan::tests::test_an_out_of_line_test_module_and_its_subtree_are_excluded`.
    ///
    /// `Recorder` stood here until `test_logger` stopped being
    /// `cfg(test)`-gated — its callers are in `mandala` now, which
    /// links baumhard as an ordinary dependency and cannot see
    /// anything behind baumhard's own `cfg(test)`. It is a genuine
    /// member of the on-disk source tree today, so it is no longer a
    /// witness for this claim; `BenchEntry` replaces it rather than
    /// leaving the list one shorter.
    #[test]
    fn test_type_graph_excludes_test_sources() {
        let graph = TypeGraph::build(&crate_src_root());
        assert!(
            graph.get("StubCtx").is_none(),
            "a struct declared inside an inline test module must not be indexed"
        );
        for test_only in ["TypeGraph", "TypeInfo", "BenchEntry", "Manifest", "DerivedShape"] {
            assert!(
                graph.get(test_only).is_none(),
                "`{test_only}` is declared in a file that `util/mod.rs` gates with an \
                 out-of-line `#[cfg(test)] mod`, so it is no more part of the on-disk \
                 contract than `StubCtx` is — and the index promises to hold neither"
            );
        }
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
    /// feeds — `format_strict_tests::test_every_type_the_format_reads_denies_unknown_fields`
    /// — asks "does any loadable type carry
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

    /// The same shape on a *field* attribute, where both of the
    /// remaining readers live. A field-level `bound(...)` hides a
    /// later `flatten` or `skip` exactly the way a container-level
    /// `rename_all(...)` hides a later `deny_unknown_fields`, and each
    /// of those silently widens what the guards above believe is safe.
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
    /// the format's roots today — a reader gap should go red the run it
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

    /// A `rename_all` spelling the walk does not know stops the run
    /// rather than answering "no rename".
    ///
    /// The quiet answer is the dangerous one: it publishes the Rust
    /// identifiers under names no document contains, which is exactly
    /// the failure mode #122 is about, and it does it while every
    /// other check stays green. Serde's eight rules are enumerated in
    /// [`RenameRule`]; a ninth arrives with a dependency bump
    /// somebody is already reading a changelog for.
    #[test]
    #[should_panic(expected = "names a rule this walk does not know")]
    fn test_an_unknown_rename_all_spelling_stops_the_run() {
        let _ = graph_of(
            "#[derive(Deserialize)] #[serde(rename_all = \"SpongeCase\")] \
             pub struct Planted { pub control_points: Vec<Leaf> }\n\
             #[derive(Deserialize)] pub struct Leaf { pub x: f64 }\n",
        );
    }

    /// A generic item's arguments are **bound**, not walked.
    /// `Holder` writes `Wrapper<Marker, Payload>`; `Wrapper` holds its
    /// `M` as data and reaches its `E` only through a projection,
    /// `E::Field`, which `Marker`'s `impl` resolves to `Leaf`. So the
    /// walk reaches `Payload` and `Leaf` — and not `Marker`, nor the
    /// `Secret` only `Marker` holds, which no file can contain under
    /// `Holder`. Both arrays are arrays of what their parameters were
    /// bound to, and no parameter is left over as a terminator.
    ///
    /// The control drops the `impl`: the projection then resolves to
    /// nothing, and its names are reported rather than walked as
    /// whatever the argument happens to be.
    #[test]
    fn test_a_generic_items_parameters_are_walked_as_their_bindings() {
        const SOURCE: &str = "#[derive(Deserialize)] pub struct Holder { pub wrapped: Wrapper<Marker, Payload> }\n\
             #[derive(Deserialize)] pub struct Wrapper<E: Probe, M> { pub list: Vec<M>, pub probe: Vec<E::Field> }\n\
             pub struct Marker { pub hidden: Secret }\n\
             pub trait Probe { type Field; }\n\
             #[derive(Deserialize)] pub struct Payload { pub x: f64 }\n\
             #[derive(Deserialize)] pub struct Leaf { pub y: f64 }\n\
             pub struct Secret { pub z: f64 }\n";
        const IMPL: &str = "impl Probe for Marker { type Field = Leaf; }\n";

        let graph = graph_of(&format!("{SOURCE}{IMPL}"));
        let reached: Vec<&str> = graph
            .reachable_from("Holder")
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        for data in ["Holder", "Wrapper", "Payload", "Leaf"] {
            assert!(
                reached.contains(&data),
                "{data} is data under Holder: {reached:?}"
            );
        }
        for not_data in ["Marker", "Secret"] {
            assert!(
                !reached.contains(&not_data),
                "{not_data} is only projected through, not held: {reached:?}"
            );
        }
        let unresolved = graph.unresolved_from("Holder");
        for parameter in ["E", "M", "Field"] {
            assert!(
                !unresolved.contains(parameter),
                "a bound parameter is not a terminator: {unresolved:?}"
            );
        }
        let unimplemented = graph_of(SOURCE).unresolved_from("Holder");
        assert!(
            unimplemented.contains("E") && unimplemented.contains("Field"),
            "the control: an unresolvable projection is reported: {unimplemented:?}"
        );
    }

    /// **Sixteen rule × direction paths, none of which the model
    /// exercises.**
    ///
    /// `RenameRule` is a *published* derivation: what it says a field
    /// or a variant is called is what the strictness gates read, and
    /// what the old model's `expand_route` (gone) was driven over. Nothing in this crate
    /// carries a `rename_all` today, so replacing
    /// `apply_to_variant`'s body with `format!("MUTANT_{ident}")`
    /// left all 1354 tests green, as did seven of the eight
    /// `apply_to_field` arms — an unexercised branch of a published
    /// derivation is a wrong doc waiting for the first field that
    /// uses it, which is a sentence this module already contained.
    ///
    /// So all sixteen are planted, and held against `serde_derive`'s
    /// own expansion of a copy of the same declarations rather than
    /// against names written out by hand. Three identifiers do the
    /// work no plain one can:
    ///
    /// - `ABCd`, because serde's variant snake rule inserts a
    ///   separator before *every* capital after the first, so a run
    ///   of them becomes one underscore per letter (`"a_b_cd"`) rather
    ///   than the `"ab_cd"` a reader would write;
    /// - `thing_ß` and `NaÏve`, because serde's rules are
    ///   `to_ascii_uppercase` and `to_ascii_lowercase` throughout.
    ///   Unicode case maps `ß` to `SS` and `Ï` to `ï`, so a walk that
    ///   reached for `str::to_uppercase` publishes `THING_SS` and
    ///   `naïve` — names no loader will ever match, produced in
    ///   silence.
    #[test]
    fn test_every_rename_all_rule_is_the_one_serde_applies() {
        use crate::util::serde_probe::derived_shape;

        /// One planted container per rule per direction, emitted
        /// twice from one list: as declarations the compiler derives
        /// `Deserialize` for, and as the source text the walk parses.
        ///
        /// The two are the same characters by construction. Every
        /// other planted check in this module keeps a hand-written
        /// copy and relies on the comparison failing when the two
        /// drift; with sixteen containers that copy is where the
        /// mistakes would live, and a generated one cannot drift at
        /// all.
        macro_rules! planted {
            (
                fields $field_body:tt
                variants $variant_body:tt
                $($rule:literal => $fields:ident / $variants:ident,)*
            ) => {
                $(
                    #[derive(serde::Deserialize)]
                    #[serde(rename_all = $rule)]
                    #[allow(dead_code, non_snake_case)]
                    struct $fields $field_body

                    #[derive(serde::Deserialize)]
                    #[serde(rename_all = $rule)]
                    #[allow(dead_code)]
                    enum $variants $variant_body
                )*

                /// Reaches every one of them, so the probe's sweep
                /// does too.
                #[derive(serde::Deserialize)]
                #[allow(dead_code, non_snake_case)]
                struct Rules {
                    $(
                        $fields: $fields,
                        $variants: $variants,
                    )*
                }

                // The declarations above, as text for the source
                // walk — built from the *same* token trees the
                // declarations used, so the two halves cannot drift.
                // `concat!` takes only literals, so this is assembled
                // at run time; it is a test, and a contract that
                // cannot drift beats a `const`.
                // An item, not a `let`: `macro_rules!` bindings are
                // hygienic and would not be visible to the caller,
                // whereas the items above (`Rules`) already are.
                fn planted_source() -> String {
                    // `stringify!` on the whole braced group, so the
                    // text and the declaration are the same tokens.
                    let field_body = stringify!($field_body);
                    let variant_body = stringify!($variant_body);
                    let mut s = String::new();
                    $(
                        s.push_str(&format!(
                            "#[derive(Deserialize)]\n#[serde(rename_all = \"{}\")]\npub struct {} {}\n",
                            $rule,
                            stringify!($fields),
                            field_body,
                        ));
                        s.push_str(&format!(
                            "#[derive(Deserialize)]\n#[serde(rename_all = \"{}\")]\npub enum {} {}\n",
                            $rule,
                            stringify!($variants),
                            variant_body,
                        ));
                    )*
                    s.push_str("#[derive(Deserialize)] pub struct Rules {");
                    $(
                        s.push_str(&format!(" pub {}: {},", stringify!($fields), stringify!($fields)));
                        s.push_str(&format!(" pub {}: {},", stringify!($variants), stringify!($variants)));
                    )*
                    s.push_str(" }\n");
                    s
                }
            };
        }

        planted! {
            // `wayPoint` is load-bearing, not decoration. `lowercase`
            // and `snake_case` are *identity* on fields — `case.rs`
            // reads `None | LowerCase | SnakeCase => field.to_owned()`
            // — so a field that is already lowercase cannot tell the
            // real rule from a plausible-looking
            // `ident.to_ascii_lowercase()`. Only an identifier with an
            // ASCII uppercase letter makes serde's leaving it alone
            // observable, and without one those two arms have no true
            // positive available.
            fields { pub control_points: f64, pub thing_ß: f64, pub wayPoint: f64, }
            variants { ControlPoints, ABCd, NaÏve, }
            "lowercase" => LowerFields / LowerVariants,
            "UPPERCASE" => UpperFields / UpperVariants,
            "PascalCase" => PascalFields / PascalVariants,
            "camelCase" => CamelFields / CamelVariants,
            "snake_case" => SnakeFields / SnakeVariants,
            "SCREAMING_SNAKE_CASE" => ScreamingSnakeFields / ScreamingSnakeVariants,
            "kebab-case" => KebabFields / KebabVariants,
            "SCREAMING-KEBAB-CASE" => ScreamingKebabFields / ScreamingKebabVariants,
        }

        let graph = graph_of(&planted_source());
        let derived = derived_shape::<Rules>();
        let walked = graph.member_names_from("Rules");

        let mut compared = 0usize;
        for (fields_of, variants_of) in [
            ("LowerFields", "LowerVariants"),
            ("UpperFields", "UpperVariants"),
            ("PascalFields", "PascalVariants"),
            ("CamelFields", "CamelVariants"),
            ("SnakeFields", "SnakeVariants"),
            ("ScreamingSnakeFields", "ScreamingSnakeVariants"),
            ("KebabFields", "KebabVariants"),
            ("ScreamingKebabFields", "ScreamingKebabVariants"),
        ] {
            let fields = walked.get(fields_of);
            assert!(
                fields.is_some_and(|fields| fields.len() == 3),
                "`{fields_of}`'s three fields have to reach the walk before their names can \
                 be compared: {fields:?}"
            );
            assert_eq!(
                fields,
                derived.members_of(fields_of),
                "`{fields_of}`: the walk and `serde_derive` disagree about what \
                 `rename_all` makes these fields called on disk"
            );
            let variants = graph.get(variants_of).map(|info| info.variants.as_slice());
            assert!(
                variants.is_some_and(|variants| variants.len() == 3),
                "`{variants_of}`'s three variants have to reach the walk too: {variants:?}"
            );
            assert_eq!(
                variants,
                derived.variants_of(variants_of),
                "`{variants_of}`: the walk and `serde_derive` disagree about what \
                 `rename_all` makes these variants called on disk"
            );
            compared += 2;
        }
        assert_eq!(
            compared, 16,
            "one rule × direction path each, and there are sixteen"
        );

        // Named as well as compared, because an agreement is only
        // worth as much as the transform it agrees about. Each of
        // these is a name a reader would get wrong writing it by
        // hand, and the first three are the whole reason `ABCd`,
        // `thing_ß` and `NaÏve` are in the plant.
        for (container, expected) in [
            ("SnakeVariants", "a_b_cd"),
            ("ScreamingKebabVariants", "A-B-CD"),
            ("UpperFields", "THING_ß"),
            ("LowerVariants", "naÏve"),
            ("CamelFields", "controlPoints"),
            ("PascalFields", "ControlPoints"),
            ("KebabFields", "control-points"),
            ("ScreamingSnakeFields", "CONTROL_POINTS"),
        ] {
            let names: Vec<&str> = walked
                .get(container)
                .into_iter()
                .flatten()
                .chain(
                    graph
                        .get(container)
                        .into_iter()
                        .flat_map(|info| info.variants.iter()),
                )
                .map(String::as_str)
                .collect();
            assert!(
                names.contains(&expected),
                "`{container}` must publish `{expected}`, which is what serde's rule \
                 produces and not what the identifier reads as: {names:?}"
            );
        }
    }

    /// **`#[serde(alias = "…")]` is in the published contract, and it
    /// has to be in *both* published surfaces.**
    ///
    /// The decision is not arbitrary and it is not the walk's to make
    /// alone. `serde_derive` puts a field's aliases into the same
    /// `FIELDS` list it hands `deserialize_struct` and a variant's
    /// into `VARIANTS`, so the compiler already publishes them; a
    /// file may write any of them, and a captured route is recorded
    /// against whichever it wrote. The array walk this module had
    /// before V1-C3 said so from the start. `member_names_from` dropped
    /// them and the
    /// variant reader discarded them, which made one walk answer
    /// "alias is a name" and two answer "alias is nothing" — and it
    /// put the variant surface in *direct* conflict with the probe,
    /// which sees a variant alias whether or not the walk does.
    ///
    /// Held against `serde_probe` in the same test for the same
    /// reason the planted-shape check exists: an expectation written
    /// by hand proves the walk does what its author believed and
    /// nothing about whether serde agrees. Sorted-inside-the-group
    /// ordering is checked explicitly — `VARIANTS` flat-maps a
    /// `BTreeSet` per variant, so `lit` follows `Literal` rather than
    /// preceding it.
    #[test]
    fn test_every_alias_reaches_both_published_surfaces() {
        use crate::util::serde_probe::derived_shape;

        /// Kept identical to `SOURCE` below.
        #[derive(serde::Deserialize)]
        struct AliasRoot {
            #[serde(alias = "waypoints")]
            #[allow(dead_code)]
            control_points: Vec<AliasLeaf>,
            #[allow(dead_code)]
            payload: AliasPayload,
        }

        /// Kept identical to `SOURCE` below.
        #[derive(serde::Deserialize)]
        enum AliasPayload {
            #[serde(alias = "lit")]
            #[allow(dead_code)]
            Literal(Vec<AliasLeaf>),
            #[allow(dead_code)]
            Bare,
        }

        /// Kept identical to `SOURCE` below.
        #[derive(serde::Deserialize)]
        struct AliasLeaf {
            #[allow(dead_code)]
            x: f64,
        }

        /// The declarations above, as text for the source walk.
        const SOURCE: &str = "\
#[derive(Deserialize)]
pub struct AliasRoot {
    #[serde(alias = \"waypoints\")]
    pub control_points: Vec<AliasLeaf>,
    pub payload: AliasPayload,
}
#[derive(Deserialize)]
pub enum AliasPayload {
    #[serde(alias = \"lit\")]
    Literal(Vec<AliasLeaf>),
    Bare,
}
#[derive(Deserialize)]
pub struct AliasLeaf {
    pub x: f64,
}
";

        let graph = graph_of(SOURCE);
        let derived = derived_shape::<AliasRoot>();

        let members = graph.member_names_from("AliasRoot");
        assert_eq!(
            members.get("AliasRoot"),
            derived.members_of("AliasRoot"),
            "the member-name surface carries the field's alias too, which is what the \
             derive's `FIELDS` list says"
        );
        let names = members.get("AliasRoot").expect("the root is walked");
        for spelling in ["control_points", "waypoints"] {
            assert!(
                names.contains(spelling),
                "and the agreement has to be about something — the field and its alias are \
                 the spellings the surfaces disagreed on: {names:?}"
            );
        }

        let variants = graph.get("AliasPayload").map(|info| info.variants.as_slice());
        assert_eq!(
            variants,
            derived.variants_of("AliasPayload"),
            "and the variant surface carries the variant's, in `VARIANTS` order — \
             sorted inside each variant's group, groups in source order"
        );
        assert_eq!(
            variants,
            Some(["Literal".to_string(), "lit".to_string(), "Bare".to_string()].as_slice()),
            "which is not the same as sorting the whole list"
        );
    }

    /// A variant serde skips is not on either published surface: the
    /// walk leaves it out of the enum's variants exactly as
    /// `serde_derive`'s `VARIANTS` does, and reaches nothing through
    /// it — the engine's `Mutator::Structure` is such a variant, and
    /// the `NodeId` inside it is no part of any file. The control: the
    /// variant beside it is on both surfaces and reaches its leaf.
    #[test]
    fn test_a_skipped_variant_is_on_no_published_surface() {
        use crate::util::serde_probe::derived_shape;

        /// Kept identical to `SOURCE` below.
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        enum SkipPayload {
            Shown(ShownLeaf),
            #[serde(skip)]
            Hidden(HiddenLeaf),
        }

        /// Kept identical to `SOURCE` below.
        #[derive(serde::Deserialize)]
        #[allow(dead_code)]
        struct ShownLeaf {
            x: f64,
        }

        /// Kept identical to `SOURCE` below; not `Deserialize`, as
        /// the skipped variant asks nothing of it.
        #[allow(dead_code)]
        struct HiddenLeaf {
            y: f64,
        }

        /// The declarations above, as text for the source walk.
        const SOURCE: &str = "\
#[derive(Deserialize)]
pub enum SkipPayload {
    Shown(ShownLeaf),
    #[serde(skip)]
    Hidden(HiddenLeaf),
}
#[derive(Deserialize)]
pub struct ShownLeaf {
    pub x: f64,
}
pub struct HiddenLeaf {
    pub y: f64,
}
";

        let graph = graph_of(SOURCE);
        let variants = graph.get("SkipPayload").map(|info| info.variants.as_slice());
        assert_eq!(
            variants,
            derived_shape::<SkipPayload>().variants_of("SkipPayload")
        );
        assert_eq!(variants, Some(["Shown".to_string()].as_slice()));
        let reached: Vec<&str> = graph
            .reachable_from("SkipPayload")
            .iter()
            .map(|info| info.name.as_str())
            .collect();
        assert!(
            reached.contains(&"ShownLeaf"),
            "the control reaches its leaf: {reached:?}"
        );
        assert!(
            !reached.contains(&"HiddenLeaf"),
            "the skipped variant reaches nothing: {reached:?}"
        );
    }

    /// **"Reachable from X" and "only reachable through X" are
    /// different questions, and only the second one is worth
    /// asking.** The old loader's
    /// `test_every_variant_bearing_enum_sits_inside_a_skippable_construct` (gone)
    /// asked whether every enum a load can meet sits inside something
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
        assert!(
            !info.derives_deserialize,
            "the premise: the derive is unrecognizable"
        );
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

    /// **The field half of the fail-safe, which nothing observed.**
    /// [`unread_serde_attrs_with_fields`] exists because a field-level
    /// attribute the reader cannot finish hides a later `flatten` on
    /// the same field exactly the way a container-level one hides a
    /// later `deny_unknown_fields` — and its own doc says so. Every
    /// other test here drives the container half, so dropping the
    /// field loop entirely left the suite green.
    ///
    /// What that costs is the whole contract:
    /// `#[serde("not a meta item", flatten)]` defeats
    /// [`flattened_fields`] too, so with the loop gone the type
    /// reports **no hiding reason at all** — "unknown is safe"
    /// restored, which is the one answer
    /// [`TypeInfo::unknown_key_hiding_reasons`] must never give.
    #[test]
    fn test_a_field_serde_attribute_the_reader_cannot_finish_is_a_hiding_reason() {
        let planted = graph_of(
            "#[derive(Deserialize)] pub struct Planted { \
               #[serde(\"not a meta item\", flatten)] pub rest: Value }\n",
        );
        let info = planted.get("Planted").expect("the planted type must be indexed");
        assert!(
            info.flattened_fields.is_empty(),
            "the premise: the same attribute defeats the flatten reader, so nothing else \
             is left to report this type"
        );
        let reasons = info.unknown_key_hiding_reasons();
        assert_eq!(reasons.len(), 1, "expected one reason, got {reasons:?}");
        assert!(reasons[0].contains("unread"), "{}", reasons[0]);

        // The same attribute on an enum's struct variant, which is a
        // second call site of the field loop and would otherwise be
        // covered only by inference.
        let variant = graph_of(
            "#[derive(Deserialize)] pub enum Planted { \
               A { #[serde(\"not a meta item\", flatten)] rest: Value } }\n",
        );
        let reasons = variant
            .get("Planted")
            .expect("the planted enum must be indexed")
            .unknown_key_hiding_reasons();
        assert_eq!(reasons.len(), 1, "expected one reason, got {reasons:?}");
        assert!(reasons[0].contains("unread"), "{}", reasons[0]);
    }
}
