// SPDX-License-Identifier: MPL-2.0

//! CODE_CONVENTIONS §8's "prose must not name an item the same change
//! deleted", held against the tree instead of left to a rename sweep:
//! **every `snake_case` identifier a doc comment writes in backticks,
//! when it is qualified or is a multi-word bare name, is one the
//! sources still declare or write — or is marked `(gone)`, so a
//! reader is told rather than sent looking.**
//!
//! [`crate::util::source_scan::scan_test_citations`] holds this for
//! one family of names, the `test_*` ones, because its extractor
//! reads the literal prefix. The names that are not tests have no
//! prefix to read, and a deletion leaves them behind just the same:
//! issue #163 counted nine stale references across three review
//! rounds of one series — a module header describing liveness
//! "written through `insert`" after that method was deleted, a
//! field's doc naming the function that used to overwrite it, a live
//! control instruction whose named line no longer fails the test it
//! names — and no gate saw any of them. Two were found only because a
//! human asked for a grep sweep after the review had approved the
//! round. The first run of this scan over the tree found over a
//! hundred more, many in test modules, where `cargo doc` never looks.
//!
//! **What is read.** Every `///` and `//!` the item walk reaches, in
//! every `.rs` file under the source roots, test modules and
//! target-gated modules included: a file's or inline module's header,
//! every item, the members of `impl` and `trait` blocks, struct
//! fields, enum variants and their fields. Not read: `//` comments
//! that are not doc comments, Markdown files, and doc comments on
//! statements inside a function body.
//!
//! **Scope is the whole design.** A scan for "an identifier in a doc
//! comment that does not resolve" drowns at once: `insert`, `get`,
//! `color` and `font` are English words, stdlib and third-party names
//! cannot be resolved from here, and a hypothetical ("if a future
//! method on the cache did X") is legitimate prose. So a span is a
//! candidate only when it is written in backticks, parses as a path of
//! identifiers (a call group, a macro's `!`, a `?`, a leading `&` or
//! `*` and any `<…>` generic arguments are dropped first), and ends in
//! a `snake_case` segment; and it is held to a different authority by
//! how it is qualified:
//!
//! - **`Type::name`** where `Type` is declared in the workspace: the
//!   name must be a field, method, associated item or variant that
//!   some `impl`, `trait` or definition of that type declares. This is
//!   the sharp case, and the one the stale references above were
//!   mostly written in: `SceneConnectionCache::insert` (gone) is
//!   checked against what `SceneConnectionCache` declares, not
//!   against whether `insert` appears in a file that also calls
//!   `HashMap::insert`. `Self::…` is this rule for the type of the
//!   `impl`, `trait`, struct or enum the doc sits in, and
//!   `Enum::Variant::field` is held to the enum.
//! - **`module::name`** where `module` is declared in the workspace:
//!   the name must exist somewhere — declared as an item, or written
//!   in some file's code.
//! - **bare** (`` `pre_clip_positions` ``): the name must exist
//!   somewhere in the same sense, and is read only when it has an
//!   underscore. A single word is where English, tools, crates and
//!   stdlib methods live; a two-word name that no file writes and
//!   nothing declares is a name that has left. The authority is the
//!   whole workspace's code rather than the one file's, because a doc
//!   routinely names a field of a struct in another module that its
//!   own file never touches.
//! - A **family** written with a trailing `_` or `_*`
//!   (`` `select_sibling_*` ``) exists when anything it prefixes does,
//!   the reading the citation scan gives the same spelling.
//! - A path **rooted** in `crate::`, `super::`, `self::` or a
//!   workspace package is the workspace's whatever else it names: a
//!   module or type under such a root that the workspace does not
//!   declare is reported, since the path can mean nothing else. A
//!   path rooted in a crate the manifests depend on or in `std`, and a
//!   two-segment path whose qualifier is no type or module the
//!   workspace declares — `HashMap::entry`, `serde_json::from_str`,
//!   `str::chars`, `std::sync::mpsc` — is external, and the span is
//!   not a candidate at all. So is an unrooted `Type::name` where the
//!   file imports `Type` from outside the workspace (`use
//!   std::cell::Cell;` makes a `set` on `Cell` in that file std's,
//!   whatever else is called `Cell` here), and where `Type` is a
//!   generic parameter (`T::void()`).
//! - A member a macro emits is invisible to the parse, so it is
//!   harvested from the text — the word after `fn`, a `pub name:`
//!   field line — and attributed by the braces to the type whose
//!   literal header encloses it: `impl Config {` written inside a
//!   `macro_rules!` body puts its methods on `Config`, parsed or not.
//!   A member written under `impl $name {`, or at a macro body's top
//!   level, belongs to a type the text cannot name; such a name
//!   resolves where a macro can have put it — on a type the parse saw
//!   that carries a custom derive (`Action`'s `ActionClassify`), and
//!   on a type a macro wrote — and on no plainly parsed type. A
//!   macro-written type is held to its literal headers that way,
//!   unless one of its members is itself a macro parameter
//!   (`KeybindConfig`'s `pub $simple_field:`, written by
//!   `keybind_surface!`), when its member list is unknowable and any
//!   name the workspace declares or writes resolves on it. The
//!   methods the known derives and traits supply (`clone`, `default`,
//!   `fmt`, `to_string`, strum's `iter`, …) resolve on a type that
//!   derives or implements the trait that supplies them, and on no
//!   other; a derive written in a macro body counts when it stands
//!   over a literal header.
//!
//! **The opt-out is written where the name is.** This project writes
//! past-tense prose that names things which no longer exist, and that
//! prose is correct and worth keeping: a regression test's doc naming
//! the field whose staleness made it fail, a history paragraph saying
//! what a function replaced. The disclosure
//! `SceneConnectionCache::refill`'s history paragraph already carried
//! — *"gone, named here as history, not as a function a reader will
//! find"* — is what the marker spells: `(gone)` directly after the
//! closing backtick, `` `reclaim_sample_buffer` (gone) ``. The
//! exemption is visible to the reader of the prose, and on a
//! qualified name it is itself checked: `(gone)` on a name the
//! sources still declare is reported, because then the prose is
//! lying in the other direction. On a bare name the marker is taken
//! at its word, since the bare rule's authority is any token in the
//! workspace's code — too loose to contradict a reader with.
//!
//! **What it does not catch**, so nobody over-trusts it:
//!
//! - A name written as English rather than in backticks. The claim
//!   in `color_picker_flow/commit.rs` that listed "fields the
//!   connection pass caches per-edge (body glyph, color, font)" was
//!   false from the first commit of the series that removed them, and
//!   no identifier scan can see it. **This gate is not a substitute
//!   for a sweep when a series deletes public surface.**
//! - A bare single word (`` `insert` ``, `` `refill` ``), deleted or
//!   not. Write such a name qualified and the sharp rule applies.
//! - A bare multi-word name that any file's code still writes — a
//!   deleted method would resolve while some test still binds a local
//!   of the same name. The sharp rule does not have this gap.
//! - A name moved to another module: the authority for bare and
//!   module-qualified names is "exists somewhere", so a reference
//!   whose target moved still resolves.
//! - A name that exists and is the wrong one: `bvh_find` named where
//!   `spatial_descend_best` was meant resolves, because both exist.
//!   The review of this gate's own landing found two such
//!   substitutions among the names it had corrected.
//! - A bare multi-word name of an external API that no file in the
//!   workspace calls — serde's `Serializer::serialize_tuple_struct`
//!   written without its type. It reads as a name that left, and the
//!   fix is to root it as shown; the handful of spans rooted when the
//!   gate landed is the price of the bare rule catching the many stale
//!   names it did.
//! - A two-segment path whose qualifier the workspace does not
//!   declare: `probe_module::probe_fn` is indistinguishable from a
//!   dependency's path, so only the rooted spelling is held to the
//!   workspace.
//! - An external import's shadowing is read file-wide: `use
//!   std::cell::Cell;` in one inline module makes every unrooted
//!   `Cell::…` in the file external, including one in a sibling
//!   module that meant the workspace's `Cell`. Scoping the set to the
//!   module is a push of the inner module's `use` leaves before the
//!   item walk recurses into it and a pop after; what is not free is
//!   `use super::*` and `use super::Cell`, which carry the parent's
//!   imports into the child and would have to be followed for the
//!   child's docs to keep the parent's shadowing. The census found no
//!   doc that the file-wide reading misjudges, so the walk stays flat
//!   until one does.
//! - A `#[derive(…)]` over a type a macro body writes is read from
//!   the text, wrapped over several lines or not; a derive that stands
//!   over a `$name` header, or is itself written as a parameter
//!   (`$(#[$meta])*`), grants nothing, since the text cannot say what
//!   it names.
//! - Dotted forms (`` `node.position` ``, `` `clock.wait` ``,
//!   `` `test.sh` ``): in this tree they are mostly JSON paths, IPC
//!   verbs and file names, and the scan does not read them. Nor is a
//!   possessive (`` `old_fn's` ``) or any other punctuation inside the
//!   backticks that is not a call group, a `!`, a `?`, a `*` or a
//!   generic argument group, or a span rustfmt broke across two doc
//!   lines, whose halves are not read.
//! - `CamelCase` types and `SCREAMING_SNAKE` constants, which are
//!   not `snake_case`; intra-doc link *targets*, which `cargo doc`
//!   already checks on live code.
//! - A private field a macro emits (no `pub` to read the line by), a
//!   type a macro emits without writing its name, and a declaration a
//!   macro builds from pieces, which no text harvest can see. The one
//!   exception is strum's `EnumDiscriminants` mirror, whose variants
//!   and traits the attribute on the parsed enum determines in full,
//!   so `ActionKind::iter()` is held to `Action`'s variants and to the
//!   `derive(EnumIter)` its `strum_discriminants` attribute lists.
//! - A sentence whose every identifier still exists but whose claim
//!   went false. That needs reading.
//!
//! Test-only and native-only, for the same reasons
//! [`crate::util::source_scan`] is.

use crate::util::doc_fixtures::repo_path;
use crate::util::manifests;
use crate::util::rust_source::{blank_string_literals, strip_comments};
use crate::util::source_scan::{
    impl_item_attrs, item_attrs, relative_to_repo, trait_item_attrs, workspace_rust_sources,
};
use std::collections::{BTreeMap, BTreeSet};
use syn::spanned::Spanned;
use syn::{Attribute, Fields, Item, UseTree};

/// The marker that exempts the span it follows: `` `name` (gone) ``.
/// Spelled as a constant so the scanner and its tests agree on one
/// string, and so this file's own prose can name it without becoming
/// an example of it.
pub(crate) const GONE_MARKER: &str = "(gone)";

/// Rust keywords, primitive types and the JSON literal words a doc
/// writes in backticks that are never a declared identifier.
const NOT_AN_IDENTIFIER: &[&str] = &[
    // keywords
    "as",
    "async",
    "await",
    "break",
    "const",
    "continue",
    "crate",
    "dyn",
    "else",
    "enum",
    "extern",
    "false",
    "fn",
    "for",
    "if",
    "impl",
    "in",
    "let",
    "loop",
    "match",
    "mod",
    "move",
    "mut",
    "pub",
    "ref",
    "return",
    "self",
    "static",
    "struct",
    "super",
    "trait",
    "true",
    "type",
    "unsafe",
    "use",
    "where",
    "while",
    "union",
    "macro_rules",
    // primitive types
    "bool",
    "char",
    "str",
    "f32",
    "f64",
    "i8",
    "i16",
    "i32",
    "i64",
    "i128",
    "isize",
    "u8",
    "u16",
    "u32",
    "u64",
    "u128",
    "usize",
    // attribute and cfg vocabulary a doc quotes as such
    "cfg",
    "test",
    "derive",
    "allow",
    "deny",
    "warn",
    "inline",
    "doc",
    // JSON
    "null",
];

/// The methods the traits a derive or an `impl` can name supply —
/// std's derivable and operator traits, serde's, strum's — by trait,
/// so `Type::clone` resolves on a type that derives `Clone`,
/// `Type::to_string` on one that implements `Display`, `Type::iter`
/// on one that derives `EnumIter` — and on no other. Deliberately not
/// a flat list: an `::iter` on a type that iterates nothing must not
/// pass because *some* type somewhere does.
const TRAIT_METHODS: &[(&str, &[&str])] = &[
    ("Default", &["default"]),
    ("Clone", &["clone", "clone_from"]),
    ("Debug", &["fmt"]),
    ("Display", &["fmt", "to_string"]),
    ("PartialEq", &["eq", "ne"]),
    ("Hash", &["hash"]),
    ("PartialOrd", &["partial_cmp", "lt", "le", "gt", "ge"]),
    ("Ord", &["cmp", "max", "min", "clamp"]),
    ("Serialize", &["serialize"]),
    ("Deserialize", &["deserialize"]),
    ("FromStr", &["from_str"]),
    ("From", &["from"]),
    ("TryFrom", &["try_from"]),
    ("Into", &["into"]),
    ("TryInto", &["try_into"]),
    ("AsRef", &["as_ref"]),
    ("AsMut", &["as_mut"]),
    ("Deref", &["deref"]),
    ("DerefMut", &["deref_mut"]),
    ("Drop", &["drop"]),
    ("Iterator", &["next"]),
    ("IntoIterator", &["into_iter"]),
    ("Index", &["index"]),
    ("IndexMut", &["index_mut"]),
    ("Borrow", &["borrow"]),
    ("BorrowMut", &["borrow_mut"]),
    ("ToOwned", &["to_owned"]),
    ("Add", &["add"]),
    ("Sub", &["sub"]),
    ("Mul", &["mul"]),
    ("Div", &["div"]),
    ("Neg", &["neg"]),
    ("AddAssign", &["add_assign"]),
    ("SubAssign", &["sub_assign"]),
    ("MulAssign", &["mul_assign"]),
    ("DivAssign", &["div_assign"]),
    ("EnumIter", &["iter"]),
    ("EnumString", &["from_str"]),
    ("AsRefStr", &["as_ref"]),
    ("IntoStaticStr", &["into"]),
];

/// The derives whose output is known: std's, serde's and strum's,
/// whose members [`TRAIT_METHODS`] lists. A derive of anything else is
/// a custom derive, which may have emitted members the parse cannot
/// see.
const KNOWN_DERIVES: &[&str] = &[
    "Clone",
    "Copy",
    "Debug",
    "Default",
    "PartialEq",
    "Eq",
    "PartialOrd",
    "Ord",
    "Hash",
    "Serialize",
    "Deserialize",
    "EnumIter",
    "EnumString",
    "EnumCount",
    "EnumDiscriminants",
    "AsRefStr",
    "IntoStaticStr",
    "Display",
    "VariantNames",
    // `derivative`'s umbrella derive; the traits it implements are
    // listed in its own `#[derivative(…)]` attribute, read alongside.
    "Derivative",
];

/// One source file, read and parsed once, for both the index and the
/// scan to consume.
pub(crate) struct Source {
    /// Repo-relative path, as the reports name it.
    pub rel: String,
    /// The file's text.
    pub text: String,
    /// The file's items.
    pub parsed: syn::File,
}

impl Source {
    /// Parse `text` as `rel`. Panics naming the file on text `syn`
    /// cannot parse: a source that does not parse is not a source
    /// this scan can be trusted on, and dropping it would shrink the
    /// declared set in silence.
    pub(crate) fn parse(rel: &str, text: &str) -> Self {
        let parsed = syn::parse_file(text).unwrap_or_else(|e| panic!("{rel} must parse as Rust: {e}"));
        Self {
            rel: rel.to_string(),
            text: text.to_string(),
            parsed,
        }
    }
}

/// Every `.rs` file under the source roots, read and parsed once.
///
/// Cost: one read and one `syn` parse per file.
pub(crate) fn read_workspace_sources() -> Vec<Source> {
    workspace_rust_sources()
        .iter()
        .map(|file| {
            let text = std::fs::read_to_string(file)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
            Source::parse(&relative_to_repo(file), &text)
        })
        .collect()
}

/// What the workspace declares, read once and consulted per span.
///
/// Built from the `syn` parse of every file — which is what knows a
/// struct's fields, an `impl` block's methods and a `trait`'s items —
/// plus a text harvest of the word after each declaring keyword, so
/// that a declaration a `macro_rules!` body emits still counts.
/// Over-reading a declaration weakens the check by one name;
/// under-reading one fails a run over prose that is correct, which is
/// the worse of the two.
pub(crate) struct WorkspaceIndex {
    /// Every name declared as an item, field, variant or associated
    /// item anywhere, by parse or by text, plus every module name and
    /// every source file stem and directory, since a `mod x;` names
    /// one.
    declared: BTreeSet<String>,
    /// Names declared as a `struct`, `enum`, `trait`, `type` or
    /// `union` — the qualifiers the sharp rule applies to.
    types: BTreeSet<String>,
    /// The subset of [`Self::types`] whose declaration is fully known:
    /// the parse saw it, or a known derive writes it from a declaration
    /// the parse saw (strum's `EnumDiscriminants` mirror, whose
    /// variants are the enum's and whose traits its attribute lists).
    /// A type in `types` and not here was written by a macro.
    parsed_types: BTreeSet<String>,
    /// Names declared as a `mod`, plus file stems and directory
    /// names under the source roots.
    modules: BTreeSet<String>,
    /// Type or trait name → the fields, variants, methods and
    /// associated items its definition, `impl` blocks and `trait`
    /// body declare, across the whole workspace.
    members: BTreeMap<String, BTreeSet<String>>,
    /// Type name → the traits it has an `impl Trait for Type` for or
    /// derives, so a default method a workspace trait body declares,
    /// and a method a std trait supplies, resolve on the type.
    implements: BTreeMap<String, BTreeSet<String>>,
    /// Types that derive a trait outside [`KNOWN_DERIVES`]: a custom
    /// derive, whose output may hold members the parse cannot see.
    custom_derived: BTreeSet<String>,
    /// Names the text harvest declares and no parse of any file does:
    /// the members a `macro_rules!` body or a derive's `quote!` emits.
    macro_declared: BTreeSet<String>,
    /// The subset of [`Self::macro_declared`] written under no literal
    /// type header — at a macro body's top level, or inside an
    /// `impl $name {` whose type is a macro parameter. These belong to
    /// a type the text cannot name, so a type a macro can have written
    /// them onto resolves them: one a macro wrote, or one with a custom
    /// derive.
    macro_unscoped: BTreeSet<String>,
    /// Type name → the members the text harvest read under that
    /// type's own literal header, `impl Config {`, `impl Default for
    /// Config {` or `pub struct Config {`, wherever the header stands
    /// — a `macro_rules!` body included, where it is the one place a
    /// macro-emitted member is attributable to its type.
    text_members: BTreeMap<String, BTreeSet<String>>,
    /// Types whose literal header encloses a member written as a macro
    /// parameter (`pub $field:`, `fn $method`): the parse never saw
    /// them, and the text cannot finish listing their members — the
    /// names come from the invocation's tokens — so any name the
    /// workspace declares or writes resolves on them.
    open_members: BTreeSet<String>,
    /// Every identifier token any file's code writes, comments
    /// stripped and string literals kept. A bare name is held to this
    /// rather than to a declaration: a file's code writes every
    /// function it calls, every field it touches and every name it
    /// imports, and a JSON key or a criterion id lives in a string.
    code: BTreeSet<String>,
    /// The workspace's own package names, which root an internal path.
    crates: BTreeSet<String>,
    /// The crates the manifests depend on, plus `std`, `core` and
    /// `alloc`: a path rooted in one is external, whatever a
    /// workspace module happens to be called.
    external_crates: BTreeSet<String>,
}

impl WorkspaceIndex {
    /// The index of `sources`, with the workspace's package and
    /// dependency names read from its manifests.
    ///
    /// Cost: one comment strip and one string blanking per source, one
    /// walk of each parse, and one read per manifest.
    pub(crate) fn of_workspace(sources: &[Source]) -> Self {
        let crates: BTreeSet<String> = manifests::member_manifests()
            .iter()
            .map(|path| {
                let text = std::fs::read_to_string(repo_path(path))
                    .unwrap_or_else(|e| panic!("{path} must be readable: {e}"));
                package_name(path, &text)
            })
            .collect();
        let external: BTreeSet<String> = manifests::workspace_manifests()
            .iter()
            .flat_map(|manifest| manifest.deps.iter())
            .filter(|dep| !dep.is_path)
            .map(|dep| dep.crate_name().replace('-', "_"))
            .collect();
        Self::from_sources(sources, &crates, &external)
    }

    /// The index of `sources`, for a workspace whose packages are
    /// `crates` and whose dependencies are `external`.
    pub(crate) fn from_sources(
        sources: &[Source],
        crates: &BTreeSet<String>,
        external: &BTreeSet<String>,
    ) -> Self {
        let mut index = Self {
            declared: BTreeSet::new(),
            types: BTreeSet::new(),
            parsed_types: BTreeSet::new(),
            modules: BTreeSet::new(),
            members: BTreeMap::new(),
            implements: BTreeMap::new(),
            custom_derived: BTreeSet::new(),
            macro_declared: BTreeSet::new(),
            macro_unscoped: BTreeSet::new(),
            text_members: BTreeMap::new(),
            open_members: BTreeSet::new(),
            code: BTreeSet::new(),
            crates: crates.clone(),
            external_crates: external.clone(),
        };
        for name in ["std", "core", "alloc"] {
            index.external_crates.insert(name.to_string());
        }
        // Two passes, so that what the text harvest adds beyond the
        // parse is exactly the macro-emitted set: a name `syn` declares
        // in a later file must not be mistaken for one a macro emits
        // because the text saw it first.
        let mut stripped = Vec::with_capacity(sources.len());
        for source in sources {
            index.harvest_path(&source.rel);
            index.harvest_items(&source.parsed.items);
            let code = strip_comments(&source.text);
            index.code.extend(identifier_tokens(&code).map(str::to_string));
            stripped.push(code);
        }
        let parsed_declared = index.declared.clone();
        let mut text_declared = BTreeSet::new();
        let mut text_unscoped = BTreeSet::new();
        for code in &stripped {
            index.harvest_text(
                &blank_string_literals(code),
                &mut text_declared,
                &mut text_unscoped,
            );
        }
        index.macro_declared = text_declared.difference(&parsed_declared).cloned().collect();
        index.macro_unscoped = text_unscoped
            .intersection(&index.macro_declared)
            .cloned()
            .collect();
        index
    }

    /// File stems and directory names are module names: `mod x;`
    /// names `x.rs` or `x/mod.rs`.
    fn harvest_path(&mut self, rel: &str) {
        let stem = rel.strip_suffix(".rs").unwrap_or(rel);
        for component in stem.split('/') {
            if component != "mod" && is_identifier(component) {
                self.modules.insert(component.to_string());
                self.declared.insert(component.to_string());
            }
        }
    }

    fn harvest_items(&mut self, items: &[Item]) {
        for item in items {
            match item {
                Item::Fn(item) => {
                    self.declared.insert(item.sig.ident.to_string());
                }
                Item::Struct(item) => {
                    let name = item.ident.to_string();
                    self.declare_parsed_type(&name, &item.attrs);
                    self.harvest_fields(&name, &item.fields);
                }
                Item::Enum(item) => {
                    let name = item.ident.to_string();
                    self.declare_parsed_type(&name, &item.attrs);
                    for variant in &item.variants {
                        self.declare_member(&name, &variant.ident.to_string());
                        self.harvest_fields(&name, &variant.fields);
                    }
                    if let Some(mirror) = discriminants_mirror(&item.attrs) {
                        self.declare_parsed_type(&mirror.name, &[]);
                        // A custom derive on the enum (`ActionClassify`)
                        // writes onto the mirror too, since the mirror is
                        // the type its output names.
                        if self.custom_derived.contains(&name) {
                            self.custom_derived.insert(mirror.name.clone());
                        }
                        for variant in &item.variants {
                            self.declare_member(&mirror.name, &variant.ident.to_string());
                        }
                        self.implements
                            .entry(mirror.name)
                            .or_default()
                            .extend(mirror.derives);
                    }
                }
                Item::Union(item) => {
                    let name = item.ident.to_string();
                    self.declare_parsed_type(&name, &item.attrs);
                    for field in &item.fields.named {
                        if let Some(ident) = &field.ident {
                            self.declare_member(&name, &ident.to_string());
                        }
                    }
                }
                Item::Trait(item) => {
                    let name = item.ident.to_string();
                    self.declare_parsed_type(&name, &item.attrs);
                    for inner in &item.items {
                        let member = match inner {
                            syn::TraitItem::Const(c) => Some(c.ident.to_string()),
                            syn::TraitItem::Fn(f) => Some(f.sig.ident.to_string()),
                            syn::TraitItem::Type(t) => Some(t.ident.to_string()),
                            _ => None,
                        };
                        if let Some(member) = member {
                            self.declare_member(&name, &member);
                        }
                    }
                }
                Item::Impl(item) => {
                    let Some(name) = last_path_segment(&item.self_ty) else { continue };
                    if let Some((trait_path, _)) = &item.trait_ {
                        if let Some(trait_name) = trait_path.segments.last() {
                            self.implements
                                .entry(name.clone())
                                .or_default()
                                .insert(trait_name.ident.to_string());
                        }
                    }
                    for inner in &item.items {
                        let member = match inner {
                            syn::ImplItem::Const(c) => Some(c.ident.to_string()),
                            syn::ImplItem::Fn(f) => Some(f.sig.ident.to_string()),
                            syn::ImplItem::Type(t) => Some(t.ident.to_string()),
                            _ => None,
                        };
                        if let Some(member) = member {
                            self.declare_member(&name, &member);
                        }
                    }
                }
                Item::Mod(item) => {
                    let name = item.ident.to_string();
                    self.modules.insert(name.clone());
                    self.declared.insert(name);
                    if let Some((_, inner)) = &item.content {
                        self.harvest_items(inner);
                    }
                }
                Item::Const(item) => {
                    self.declared.insert(item.ident.to_string());
                }
                Item::Static(item) => {
                    self.declared.insert(item.ident.to_string());
                }
                Item::Type(item) => {
                    self.declare_parsed_type(&item.ident.to_string(), &item.attrs);
                }
                Item::TraitAlias(item) => {
                    self.declare_parsed_type(&item.ident.to_string(), &item.attrs);
                }
                Item::Macro(item) => {
                    if let Some(ident) = &item.ident {
                        self.declared.insert(ident.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    fn harvest_fields(&mut self, owner: &str, fields: &Fields) {
        if let Fields::Named(named) = fields {
            for field in &named.named {
                if let Some(ident) = &field.ident {
                    self.declare_member(owner, &ident.to_string());
                }
            }
        }
    }

    /// The word after a declaring keyword, read as text so that a
    /// declaration a `macro_rules!` body or a `quote!` block emits —
    /// which `syn` sees as opaque tokens — still counts. Runs over
    /// every file's code with strings blanked, after every file has
    /// been parsed; each name it reads goes into `text_declared` too,
    /// and the names there that no parse declared are the
    /// macro-emitted set.
    ///
    /// Each declaration is also attributed to the type whose literal
    /// header encloses it, by the braces: `impl Config {` opens a frame
    /// `Config` owns, `impl Default for Config {` one that also records
    /// the trait, a `fn` body one nobody owns that is read through to
    /// the frame around it, and `impl $name {` one owned by a type the
    /// text cannot name, under which a member goes to
    /// `text_unscoped`. A `#[derive(…)]` line immediately above a
    /// header is read as the header's traits.
    fn harvest_text(
        &mut self,
        text: &str,
        text_declared: &mut BTreeSet<String>,
        text_unscoped: &mut BTreeSet<String>,
    ) {
        const DECLARING: &[&str] = &[
            "fn",
            "mod",
            "struct",
            "enum",
            "const",
            "static",
            "trait",
            "type",
            "union",
            "macro_rules",
        ];
        const TYPE_LIKE: &[&str] = &["struct", "enum", "trait", "type", "union"];
        let mut frames: Vec<Frame> = Vec::new();
        let mut pending: Option<Frame> = None;
        let mut derives: Vec<String> = Vec::new();
        // A `#[derive(` rustfmt wrapped over several lines, gathered
        // until its `)]` so it reads as the one line it is, with the
        // count of lines gathered so far.
        let mut open_derive: Option<(String, usize)> = None;
        for line in text.lines() {
            let trimmed = line.trim();
            if let Some((gathered, gathered_lines)) = &mut open_derive {
                if *gathered_lines < MAX_WRAPPED_DERIVE_LINES && can_continue_derive(trimmed) {
                    gathered.push(' ');
                    gathered.push_str(trimmed);
                    *gathered_lines += 1;
                    if trimmed.ends_with(")]") {
                        derives.extend(derive_line(gathered).unwrap_or_default());
                        open_derive = None;
                    }
                    continue;
                }
                // A derive that never closes grants nothing, and this
                // line is read as the text it is.
                open_derive = None;
            }
            if is_derive_opening(trimmed) && !trimmed.ends_with(")]") {
                open_derive = Some((trimmed.to_string(), 1));
                continue;
            }
            if let Some(traits) = derive_line(trimmed) {
                derives.extend(traits);
            } else if let Some(header) = header_frame(trimmed) {
                if let Frame::Owned(name) = &header {
                    self.implements
                        .entry(name.clone())
                        .or_default()
                        .extend(derives.drain(..));
                } else {
                    derives.clear();
                }
                pending = Some(header);
            } else if !(trimmed.is_empty() || trimmed.starts_with('#')) {
                derives.clear();
            }
            let owner = frames.iter().rev().find_map(|frame| match frame {
                Frame::Owned(name) => Some(Some(name.clone())),
                Frame::Parameter => Some(None),
                Frame::Body => None,
            });
            let mut attribute = |name: &str, text_unscoped: &mut BTreeSet<String>| match &owner {
                Some(Some(ty)) => {
                    self.text_members
                        .entry(ty.clone())
                        .or_default()
                        .insert(name.to_string());
                }
                _ => {
                    text_unscoped.insert(name.to_string());
                }
            };
            // `pub name: Type` is a field declaration and nothing
            // else: a struct literal's fields and a `let` binding's
            // type ascription never carry `pub`. Read so a field a
            // macro emits counts as declared.
            if let Some(field) = pub_field_name(line) {
                text_declared.insert(field.to_string());
                self.declared.insert(field.to_string());
                attribute(field, text_unscoped);
            }
            let mut previous = "";
            for word in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                if word.is_empty() {
                    continue;
                }
                if DECLARING.contains(&previous) && is_identifier(word) {
                    text_declared.insert(word.to_string());
                    self.declared.insert(word.to_string());
                    attribute(word, text_unscoped);
                    if TYPE_LIKE.contains(&previous) {
                        self.types.insert(word.to_string());
                    }
                    if previous == "mod" {
                        self.modules.insert(word.to_string());
                    }
                }
                previous = word;
            }
            if let Some(Some(ty)) = &owner {
                if declares_a_parameter(trimmed) {
                    self.open_members.insert(ty.clone());
                }
            }
            // `blank_string_literals` keeps char literals, so a `'{'`
            // or `'}'` in code is still on the line; counted, it would
            // open a frame no brace closes and leak the enclosing type
            // to the end of the file.
            for c in without_brace_char_literals(trimmed).chars() {
                match c {
                    '{' => frames.push(pending.take().unwrap_or(Frame::Body)),
                    '}' => {
                        frames.pop();
                    }
                    _ => {}
                }
            }
            if trimmed.ends_with(';') {
                pending = None;
            }
        }
    }

    /// Whether `owner` declares `name`: itself, through a workspace
    /// trait it implements, through a known trait it derives or
    /// implements that supplies the method, or through a macro. A
    /// macro-emitted name resolves where a macro can have put it: on
    /// the type whose literal header the text harvest read it under,
    /// on a type with a custom derive when the name was written under
    /// no literal header, and on a type a macro wrote. A macro-written
    /// type is held to the members its literal headers list, the
    /// traits they implement and the unscoped set — unless one of its
    /// members is itself a macro parameter, when its member list is
    /// unknowable and any name the workspace declares or writes
    /// resolves on it.
    fn has_member(&self, owner: &str, name: &str) -> bool {
        let declares = |ty: &str| self.members.get(ty).is_some_and(|members| members.contains(name));
        let traits = self.implements.get(owner);
        let supplied_by_trait = traits.is_some_and(|traits| {
            traits.iter().any(|t| declares(t))
                || TRAIT_METHODS
                    .iter()
                    .any(|(trait_name, methods)| traits.contains(*trait_name) && methods.contains(&name))
        });
        let written_under_own_header = self
            .text_members
            .get(owner)
            .is_some_and(|members| members.contains(name));
        if !self.parsed_types.contains(owner) {
            if self.open_members.contains(owner) {
                return self.exists(name);
            }
            return written_under_own_header || supplied_by_trait || self.macro_unscoped.contains(name);
        }
        declares(owner)
            || supplied_by_trait
            || (self.macro_declared.contains(name) && written_under_own_header)
            || (self.macro_unscoped.contains(name) && self.custom_derived.contains(owner))
    }

    /// Whether `name` exists somewhere: declared as an item, or
    /// written in some file's code. A name ending in `_` is a family
    /// and exists when anything it prefixes does, the reading
    /// [`crate::util::source_scan::scan_test_citations`] gives a
    /// trailing underscore.
    fn exists(&self, name: &str) -> bool {
        if name.ends_with('_') {
            let after = |set: &BTreeSet<String>| {
                set.range(name.to_string()..)
                    .next()
                    .is_some_and(|k| k.starts_with(name))
            };
            after(&self.declared) || after(&self.code)
        } else {
            self.declared.contains(name) || self.code.contains(name)
        }
    }

    /// How the first segment of a multi-segment path places it.
    fn root_of(&self, segment: &str) -> Root {
        if ["crate", "super", "self"].contains(&segment) || self.crates.contains(segment) {
            Root::Internal
        } else if self.external_crates.contains(segment) {
            Root::External
        } else if self.modules.contains(segment) || self.types.contains(segment) {
            Root::Internal
        } else {
            Root::External
        }
    }

    /// A type the parse saw, with its derives recorded: the known ones
    /// as implemented traits, any other as a custom derive.
    fn declare_parsed_type(&mut self, name: &str, attrs: &[Attribute]) {
        self.types.insert(name.to_string());
        self.parsed_types.insert(name.to_string());
        self.declared.insert(name.to_string());
        for derived in derived_traits(attrs, "derive")
            .into_iter()
            .chain(derived_traits(attrs, "derivative"))
        {
            if KNOWN_DERIVES.contains(&derived.as_str()) {
                self.implements
                    .entry(name.to_string())
                    .or_default()
                    .insert(derived);
            } else {
                self.custom_derived.insert(name.to_string());
            }
        }
    }

    fn declare_member(&mut self, owner: &str, member: &str) {
        self.members
            .entry(owner.to_string())
            .or_default()
            .insert(member.to_string());
        self.declared.insert(member.to_string());
    }
}

/// The trait names a `#[<attribute>(…)]` on `attrs` lists, by their
/// last path segment (`serde::Serialize` is `Serialize`), for
/// `derive` and for `derivative`, whose entries may carry an `=
/// "option"` that is read past.
fn derived_traits(attrs: &[Attribute], attribute: &str) -> Vec<String> {
    let mut out = Vec::new();
    for attr in attrs.iter().filter(|attr| attr.path().is_ident(attribute)) {
        let _ = attr.parse_nested_meta(|meta| {
            if let Some(last) = meta.path.segments.last() {
                out.push(last.ident.to_string());
            }
            if meta.input.peek(syn::Token![=]) {
                let _: syn::Expr = meta.value()?.parse()?;
            }
            Ok(())
        });
    }
    out
}

/// The enum strum's `EnumDiscriminants` writes beside an enum that
/// derives it: named by `#[strum_discriminants(name(X))]`, with the
/// enum's variants as unit variants, and implementing the traits the
/// attribute's `derive(…)` lists over strum's own `Clone`, `Copy`,
/// `Debug`, `PartialEq` and `Eq`. `None` when the attribute names no
/// mirror. The one macro-written type whose declaration is known in
/// full, since the derive's output is a function of the input.
struct DiscriminantsMirror {
    name: String,
    derives: Vec<String>,
}

fn discriminants_mirror(attrs: &[Attribute]) -> Option<DiscriminantsMirror> {
    let mut name = None;
    let mut derives: Vec<String> = ["Clone", "Copy", "Debug", "PartialEq", "Eq"]
        .iter()
        .map(|t| t.to_string())
        .collect();
    for attr in attrs
        .iter()
        .filter(|attr| attr.path().is_ident("strum_discriminants"))
    {
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("name") {
                meta.parse_nested_meta(|inner| {
                    name = inner.path.get_ident().map(ToString::to_string);
                    Ok(())
                })?;
            } else if meta.path.is_ident("derive") {
                meta.parse_nested_meta(|inner| {
                    if let Some(last) = inner.path.segments.last() {
                        derives.push(last.ident.to_string());
                    }
                    Ok(())
                })?;
            } else if meta.input.peek(syn::Token![=]) {
                let _: syn::Expr = meta.value()?.parse()?;
            } else if meta.input.peek(syn::token::Paren) {
                meta.parse_nested_meta(|_| Ok(()))?;
            }
            Ok(())
        });
    }
    name.map(|name| DiscriminantsMirror { name, derives })
}

/// The last segment of a type path — `SceneConnectionCache` for
/// `impl<'a> crate::mindmap::scene_cache::SceneConnectionCache<'a>`.
/// `None` for a self type that is not a path (a reference, a tuple,
/// a trait object), whose members no doc comment can qualify by name.
fn last_path_segment(ty: &syn::Type) -> Option<String> {
    match ty {
        syn::Type::Path(path) => path.path.segments.last().map(|s| s.ident.to_string()),
        syn::Type::Reference(reference) => last_path_segment(&reference.elem),
        _ => None,
    }
}

fn is_identifier(word: &str) -> bool {
    let mut chars = word.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Whether `name` is the `snake_case` shape this scan reads:
/// lowercase first letter, then lowercase letters, digits and
/// underscores. Rules out `CamelCase` types, `SCREAMING_SNAKE`
/// constants and the vocabulary in [`NOT_AN_IDENTIFIER`].
fn is_snake_case_identifier(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_lowercase())
        && chars.all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_')
        && !NOT_AN_IDENTIFIER.contains(&name)
}

/// The field a `pub name: Type` / `pub(crate) name: Type` line
/// declares, or `None` for any other line.
fn pub_field_name(line: &str) -> Option<&str> {
    let rest = line.trim_start().strip_prefix("pub")?;
    let rest = match rest.strip_prefix('(') {
        Some(after) => after.split_once(')')?.1,
        None => rest,
    };
    let rest = rest.strip_prefix(char::is_whitespace)?.trim_start();
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let name = &rest[..end];
    let after = rest[end..].trim_start();
    (is_identifier(name) && after.starts_with(':') && !after.starts_with("::")).then_some(name)
}

/// What a brace opened, for attributing the declarations inside it.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Frame {
    /// An `impl`, `struct`, `enum`, `union` or `trait` header naming
    /// a type literally: the declarations inside belong to it.
    Owned(String),
    /// Such a header whose type is a macro parameter (`impl $name {`,
    /// `struct #ident {`): the declarations inside belong to a type the
    /// text cannot name, and to no enclosing one.
    Parameter,
    /// Anything else — a `fn` body, a `mod`, a `macro_rules!` arm, a
    /// block — which is read through to the frame around it.
    Body,
}

/// The frame a line opens when it is a type header: `impl Config {`,
/// `impl<T> Trait<T> for Config<T> where …`, `pub(crate) struct
/// Config {`, `trait Describe {`. `None` for every other line. The
/// type is the header's self type by its last path segment, with
/// visibility, `unsafe`, generics, `for`, references and lifetimes
/// read past.
fn header_frame(line: &str) -> Option<Frame> {
    let mut rest = line;
    if let Some(after) = rest.strip_prefix("pub") {
        rest = match after.strip_prefix('(') {
            Some(after) => after.split_once(')')?.1,
            None => after,
        };
        rest = rest.strip_prefix(char::is_whitespace)?.trim_start();
    }
    for modifier in ["unsafe", "default"] {
        if let Some(after) = rest.strip_prefix(modifier) {
            if let Some(after) = after.strip_prefix(char::is_whitespace) {
                rest = after.trim_start();
            }
        }
    }
    let end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let keyword = &rest[..end];
    if !["impl", "struct", "enum", "union", "trait"].contains(&keyword) {
        return None;
    }
    let rest = &rest[end..];
    if !(rest.starts_with(char::is_whitespace) || rest.starts_with('<')) {
        return None;
    }
    let rest = skip_angle_group(rest.trim_start()).trim_start();
    let rest = if keyword == "impl" {
        match rest.find(" for ") {
            Some(at) => &rest[at + " for ".len()..],
            None => rest,
        }
    } else {
        rest
    };
    let rest = rest.trim_start_matches(|c: char| c == '&' || c.is_whitespace());
    let rest = match rest.strip_prefix('\'') {
        Some(after) => after.split_once(char::is_whitespace)?.1.trim_start(),
        None => rest,
    };
    let rest = rest.strip_prefix("mut ").unwrap_or(rest).trim_start();
    if rest.starts_with('$') || rest.starts_with('#') {
        return Some(Frame::Parameter);
    }
    let path_end = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_' || c == ':'))
        .unwrap_or(rest.len());
    let name = rest[..path_end].rsplit("::").next().unwrap_or("");
    is_identifier(name).then(|| Frame::Owned(name.to_string()))
}

/// `rest` past a balanced `<…>` group at its start, or `rest` itself.
fn skip_angle_group(rest: &str) -> &str {
    if !rest.starts_with('<') {
        return rest;
    }
    let mut depth = 0usize;
    for (at, c) in rest.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => {
                depth -= 1;
                if depth == 0 {
                    return &rest[at + 1..];
                }
            }
            _ => {}
        }
    }
    rest
}

/// The most lines a wrapped `#[derive(` is gathered over before it is
/// given up as never closing — far past any list rustfmt writes — so a
/// malformed macro body cannot swallow the rest of its file's harvest.
const MAX_WRAPPED_DERIVE_LINES: usize = 32;

/// Whether a line can be part of a wrapped derive list: one holding a
/// brace or a semicolon is code past the attribute, so the gather
/// stops there rather than reading it as more traits.
fn can_continue_derive(line: &str) -> bool {
    !line.contains(['{', '}', ';'])
}

/// Whether a line opens a `#[derive(…)]` or `#[derivative(…)]`
/// attribute, closed on this line or a later one.
fn is_derive_opening(line: &str) -> bool {
    ["#[derive(", "#[derivative("]
        .iter()
        .any(|prefix| line.starts_with(prefix))
}

/// `line` with every `'{'`, `'}'`, `b'{'` and `b'}'` char literal
/// removed, so the braces that remain are the ones that open and
/// close blocks. A `'\u{…}'` escape keeps its balanced pair.
fn without_brace_char_literals(line: &str) -> String {
    let mut out = line.to_string();
    for literal in ["b'{'", "b'}'", "'{'", "'}'"] {
        out = out.replace(literal, "");
    }
    out
}

/// The traits a `#[derive(…)]` or `#[derivative(…)]` attribute lists,
/// by their last path segment and with a `derivative` entry's `= "…"`
/// option read past; `None` for any other line. A wrapped attribute is
/// gathered onto one line before it is read here.
fn derive_line(line: &str) -> Option<Vec<String>> {
    let inner = ["#[derive(", "#[derivative("]
        .iter()
        .find_map(|prefix| line.strip_prefix(prefix))?;
    let inner = inner.rsplit_once(")]").map_or(inner, |(inner, _)| inner);
    Some(
        inner
            .split(',')
            .map(|entry| entry.split(['=', '(']).next().unwrap_or("").trim())
            .map(|path| path.rsplit("::").next().unwrap_or("").to_string())
            .filter(|name| is_identifier(name))
            .collect(),
    )
}

/// Whether a line declares a member whose name is a macro parameter:
/// `pub $field: Ty`, `fn $method(…)`, `const $NAME: …`.
fn declares_a_parameter(line: &str) -> bool {
    let mut previous = "";
    for word in line.split_whitespace() {
        let declaring =
            ["pub", "fn", "const", "type", "static"].contains(&previous) || previous.starts_with("pub(");
        if declaring && (word.starts_with('$') || word.starts_with('#')) {
            return true;
        }
        previous = word;
    }
    false
}

/// A single uppercase letter, optionally numbered — `T`, `F`, `T2` —
/// is a generic parameter by convention, and no type this scan
/// judges.
fn is_generic_parameter(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase()) && chars.all(|c| c.is_ascii_digit())
}

/// Where a multi-segment path's first segment places it.
#[derive(Debug, PartialEq, Eq, Clone, Copy)]
enum Root {
    Internal,
    External,
}

/// The identifier tokens in `code`, in order, with repeats.
fn identifier_tokens(code: &str) -> impl Iterator<Item = &str> {
    code.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .filter(|word| !word.is_empty())
}

/// The `name` under `[package]` in a manifest, read as text: the
/// first `name = "…"` line, which in every manifest here is the
/// package's. Panics naming the manifest when there is none, because
/// a workspace member with no package name is not a member.
fn package_name(path: &str, text: &str) -> String {
    text.lines()
        .map(str::trim)
        .find_map(|line| {
            line.strip_prefix("name")
                .map(str::trim_start)
                .and_then(|rest| rest.strip_prefix('='))
                .map(|rest| rest.trim().trim_matches('"').to_string())
        })
        .unwrap_or_else(|| panic!("{path} must declare a package name"))
}

/// What [`scan_doc_identifiers`] found.
pub(crate) struct DocIdentifierScan {
    /// One rendered `"<file>:<line>: `<span>` — <reason>"` per
    /// candidate that resolves to nothing, or that is marked
    /// [`GONE_MARKER`] while the sources still declare it.
    pub unresolved: Vec<String>,
    /// One rendered `"<file>:<line>: `<span>`"` per candidate the
    /// marker exempted, so a reader can list every deliberate
    /// mention of a vanished name in the tree.
    pub exempted: Vec<String>,
    /// Resolved candidates per repo-relative file, so a caller can
    /// notice that the walk stopped looking somewhere.
    pub resolved: BTreeMap<String, usize>,
}

impl DocIdentifierScan {
    fn new() -> Self {
        Self {
            unresolved: Vec::new(),
            exempted: Vec::new(),
            resolved: BTreeMap::new(),
        }
    }
}

/// Every backticked identifier path in a doc comment of every `.rs`
/// file under the source roots, held against [`WorkspaceIndex`].
///
/// Cost: one read and one `syn` parse per file, shared by the index
/// and the scan; the index adds one comment strip and one string
/// blanking per file and one read per manifest.
pub(crate) fn scan_doc_identifiers() -> DocIdentifierScan {
    let sources = read_workspace_sources();
    let index = WorkspaceIndex::of_workspace(&sources);
    let mut scan = DocIdentifierScan::new();
    for source in &sources {
        scan_file(source, &index, &mut scan);
    }
    scan
}

/// Read every doc comment in `source` — the file's own `//!` header,
/// every item's `///`, the members of `impl` and `trait` blocks, the
/// fields of structs and the variants of enums, into inline modules
/// — and record each candidate span in `scan`.
///
/// Doc comments on statements inside a function body are not
/// visited; the walk is over items.
pub(crate) fn scan_file(source: &Source, index: &WorkspaceIndex, scan: &mut DocIdentifierScan) {
    let mut external_imports = BTreeSet::new();
    collect_external_imports(&source.parsed.items, index, &mut external_imports);
    let mut file = FileScan {
        rel: &source.rel,
        external_imports,
        index,
        scan,
    };
    file.attrs(&source.parsed.attrs, None);
    file.items(&source.parsed.items, None);
}

/// The names a file brings in from outside the workspace — `use
/// std::cell::Cell;`, `use cosmic_text::{Buffer, Metrics};` — so a
/// `std::cell::Cell::set` written without its path in that file is
/// read as the external type's even when some workspace type is also
/// called `Cell`. Read file-wide, inline modules included: a `use` in
/// one module shadows for the whole file, which the module header
/// lists among the things not caught, with what scoping it would take.
fn collect_external_imports(items: &[Item], index: &WorkspaceIndex, out: &mut BTreeSet<String>) {
    for item in items {
        match item {
            Item::Use(item) => collect_use_leaves(&item.tree, None, index, out),
            Item::Mod(item) => {
                if let Some((_, inner)) = &item.content {
                    collect_external_imports(inner, index, out);
                }
            }
            _ => {}
        }
    }
}

fn collect_use_leaves(
    tree: &UseTree,
    root: Option<&str>,
    index: &WorkspaceIndex,
    out: &mut BTreeSet<String>,
) {
    match tree {
        UseTree::Path(path) => {
            let ident = path.ident.to_string();
            let root = root.map(str::to_string).unwrap_or(ident);
            collect_use_leaves(&path.tree, Some(&root), index, out);
        }
        UseTree::Group(group) => {
            for item in &group.items {
                collect_use_leaves(item, root, index, out);
            }
        }
        UseTree::Name(name) => {
            if root.is_some_and(|r| index.root_of(r) == Root::External) {
                out.insert(name.ident.to_string());
            }
        }
        UseTree::Rename(rename) => {
            if root.is_some_and(|r| index.root_of(r) == Root::External) {
                out.insert(rename.rename.to_string());
            }
        }
        UseTree::Glob(_) => {}
    }
}

struct FileScan<'a> {
    rel: &'a str,
    /// Type names this file imports from outside the workspace.
    external_imports: BTreeSet<String>,
    index: &'a WorkspaceIndex,
    scan: &'a mut DocIdentifierScan,
}

/// How a span names its target, which decides what it is held to.
#[derive(Debug, PartialEq, Eq)]
enum Qualifier {
    /// `` `name` ``.
    Bare,
    /// `` `Self::…` ``.
    SelfType,
    /// `` `Type::name` ``: the segment before the name starts with
    /// an uppercase letter.
    Type(String),
    /// `` `module::name` ``, including `crate::`, `super::` and
    /// `self::`, whose module this scan does not resolve by name.
    Module(String),
}

/// One backticked span that is in scope for the scan.
#[derive(Debug, PartialEq, Eq)]
struct Candidate {
    /// The `snake_case` last segment.
    name: String,
    qualifier: Qualifier,
    /// The first segment of a path of three or more — the crate or
    /// module that roots it — which decides whether the path is the
    /// workspace's to judge at all, and whether an unknown qualifier
    /// under it is a vanished name rather than an external one.
    root: Option<String>,
    /// The segment before the qualifier, when there is one: the enum
    /// in `GfxElement::GlyphArea::subtree_aabb`, whose variant the
    /// qualifier then is.
    enclosing: Option<String>,
}

enum Resolution {
    Resolved,
    /// Qualified by a type or module the workspace does not declare,
    /// under no root that makes it the workspace's: not this scan's
    /// to judge.
    External,
    Unresolved(String),
}

impl FileScan<'_> {
    fn items(&mut self, items: &[Item], self_type: Option<&str>) {
        for item in items {
            match item {
                Item::Impl(item) => {
                    let owner = last_path_segment(&item.self_ty);
                    let owner = owner.as_deref();
                    self.attrs(&item.attrs, owner);
                    for inner in &item.items {
                        self.attrs(impl_item_attrs(inner), owner);
                    }
                }
                Item::Trait(item) => {
                    let owner = item.ident.to_string();
                    self.attrs(&item.attrs, Some(&owner));
                    for inner in &item.items {
                        self.attrs(trait_item_attrs(inner), Some(&owner));
                    }
                }
                Item::Struct(item) => {
                    let owner = item.ident.to_string();
                    self.attrs(&item.attrs, Some(&owner));
                    self.fields(&item.fields, Some(&owner));
                }
                Item::Enum(item) => {
                    let owner = item.ident.to_string();
                    self.attrs(&item.attrs, Some(&owner));
                    for variant in &item.variants {
                        self.attrs(&variant.attrs, Some(&owner));
                        self.fields(&variant.fields, Some(&owner));
                    }
                }
                Item::Union(item) => {
                    let owner = item.ident.to_string();
                    self.attrs(&item.attrs, Some(&owner));
                    for field in &item.fields.named {
                        self.attrs(&field.attrs, Some(&owner));
                    }
                }
                Item::Mod(item) => {
                    self.attrs(&item.attrs, self_type);
                    if let Some((_, inner)) = &item.content {
                        self.items(inner, None);
                    }
                }
                other => self.attrs(item_attrs(other), self_type),
            }
        }
    }

    fn fields(&mut self, fields: &Fields, self_type: Option<&str>) {
        match fields {
            Fields::Named(named) => {
                for field in &named.named {
                    self.attrs(&field.attrs, self_type);
                }
            }
            Fields::Unnamed(unnamed) => {
                for field in &unnamed.unnamed {
                    self.attrs(&field.attrs, self_type);
                }
            }
            Fields::Unit => {}
        }
    }

    fn attrs(&mut self, attrs: &[Attribute], self_type: Option<&str>) {
        for attr in attrs {
            let Some((text, first_line)) = doc_text(attr) else { continue };
            for (offset, line) in text.lines().enumerate() {
                self.line(line, first_line + offset, self_type);
            }
        }
    }

    /// Every backticked span on one line of doc text. An unclosed
    /// backtick ends the line's reading: rustfmt can break a long
    /// span across two doc lines, and half a name is not a candidate.
    fn line(&mut self, text: &str, line: usize, self_type: Option<&str>) {
        let mut rest = text;
        while let Some(open) = rest.find('`') {
            let after_open = &rest[open + 1..];
            let Some(close) = after_open.find('`') else { return };
            let span = &after_open[..close];
            let tail = &after_open[close + 1..];
            let marked_gone = tail.trim_start().starts_with(GONE_MARKER);
            self.span(span, marked_gone, line, self_type);
            rest = tail;
        }
    }

    fn span(&mut self, span: &str, marked_gone: bool, line: usize, self_type: Option<&str>) {
        let Some(candidate) = classify(span) else { return };
        let spelled = span.trim();
        let where_written = format!("{}:{line}", self.rel);
        match (self.resolve(&candidate, self_type), marked_gone) {
            (Resolution::External, _) => {}
            (Resolution::Resolved, false) => {
                *self.scan.resolved.entry(self.rel.to_string()).or_default() += 1;
            }
            (Resolution::Resolved, true) => {
                // A bare name resolves through any token the
                // workspace's code writes, which is too loose an
                // authority to contradict a reader who says the thing
                // is gone. A qualified name resolved against a
                // declaration, and that is sharp enough to hold the
                // marker to.
                if candidate.qualifier == Qualifier::Bare {
                    self.scan.exempted.push(format!("{where_written}: `{spelled}`"));
                } else {
                    self.scan.unresolved.push(format!(
                        "{where_written}: `{spelled}` — marked {GONE_MARKER}, but the sources still declare it"
                    ));
                }
            }
            (Resolution::Unresolved(_), true) => {
                self.scan.exempted.push(format!("{where_written}: `{spelled}`"));
            }
            (Resolution::Unresolved(reason), false) => {
                self.scan
                    .unresolved
                    .push(format!("{where_written}: `{spelled}` — {reason}"));
            }
        }
    }

    fn resolve(&self, candidate: &Candidate, self_type: Option<&str>) -> Resolution {
        let name = candidate.name.as_str();
        // A path of three or more segments is placed by its root. Under
        // an external root nothing is judged; under an internal one
        // (`crate::`, `super::`, a workspace package) every segment is
        // the workspace's, so an unknown qualifier is a vanished name,
        // not a foreign one.
        let root = candidate.root.as_deref().map(|root| self.index.root_of(root));
        if root == Some(Root::External) {
            return Resolution::External;
        }
        let rooted = root == Some(Root::Internal);
        match &candidate.qualifier {
            Qualifier::Bare => {
                // A single word is where English, tools, crates and
                // stdlib methods live; a bare name is read only when
                // it has the underscore that makes it `snake_case`.
                if !name.contains('_') {
                    Resolution::External
                } else if self.index.exists(name) {
                    Resolution::Resolved
                } else {
                    Resolution::Unresolved(
                        "written in no file's code and declared nowhere in the workspace".to_string(),
                    )
                }
            }
            Qualifier::SelfType => match self_type {
                Some(owner) => self.resolve_member(owner, name, true),
                None => self.resolve(
                    &Candidate {
                        name: name.to_string(),
                        qualifier: Qualifier::Bare,
                        root: None,
                        enclosing: None,
                    },
                    None,
                ),
            },
            Qualifier::Type(owner) => {
                // `Enum::Variant::field`: the qualifier is a variant,
                // and the fields it carries are recorded on the enum.
                // Tried second, because a variant may share its name
                // with a type (`GfxElement::GlyphArea` wraps a
                // `GlyphArea`), and then the type is asked first.
                let variant_of = candidate.enclosing.as_deref().filter(|enclosing| {
                    self.index.types.contains(*enclosing) && self.index.has_member(enclosing, owner)
                });
                match variant_of {
                    None => self.resolve_member(owner, name, rooted),
                    // The variant is no type of its own: the enum's
                    // answer is the only one there is.
                    Some(enclosing) if !self.index.types.contains(owner) => {
                        self.resolve_member(enclosing, name, rooted)
                    }
                    // The variant shares a type's name: the type is
                    // asked first, the enum second, and when neither
                    // declares the member the type's answer names what
                    // the reader most likely meant.
                    Some(enclosing) => match self.resolve_member(owner, name, rooted) {
                        Resolution::Resolved => Resolution::Resolved,
                        on_owner => match self.resolve_member(enclosing, name, rooted) {
                            Resolution::Resolved => Resolution::Resolved,
                            _ => on_owner,
                        },
                    },
                }
            }
            Qualifier::Module(module) => {
                let known = ["crate", "super", "self"].contains(&module.as_str())
                    || self.index.modules.contains(module)
                    || self.index.types.contains(module);
                if !known {
                    if rooted {
                        Resolution::Unresolved(format!("`{module}` is declared nowhere in the workspace"))
                    } else {
                        Resolution::External
                    }
                } else if self.index.exists(name) {
                    Resolution::Resolved
                } else {
                    Resolution::Unresolved(format!(
                        "`{name}` is written in no file's code and declared nowhere in the workspace"
                    ))
                }
            }
        }
    }

    /// `owner::name` held to what `owner` declares. `rooted` says the
    /// path came under an internal root, where a generic parameter or
    /// an external import cannot be meant and an unknown type is a
    /// vanished one.
    fn resolve_member(&self, owner: &str, name: &str, rooted: bool) -> Resolution {
        if !rooted && (is_generic_parameter(owner) || self.external_imports.contains(owner)) {
            return Resolution::External;
        }
        if !self.index.types.contains(owner) {
            return if rooted {
                Resolution::Unresolved(format!("`{owner}` is declared nowhere in the workspace"))
            } else {
                Resolution::External
            };
        }
        if self.index.has_member(owner, name) {
            Resolution::Resolved
        } else {
            Resolution::Unresolved(format!("`{owner}` declares no `{name}`"))
        }
    }
}

/// The text of a `#[doc = "…"]` attribute — which is what `///` and
/// `//!` parse to — and the line it starts on. `None` for any other
/// attribute, including `#[doc(hidden)]`.
fn doc_text(attr: &Attribute) -> Option<(String, usize)> {
    if !attr.path().is_ident("doc") {
        return None;
    }
    let syn::Meta::NameValue(pair) = &attr.meta else { return None };
    let syn::Expr::Lit(syn::ExprLit {
        lit: syn::Lit::Str(text),
        ..
    }) = &pair.value
    else {
        return None;
    };
    Some((text.value(), attr.span().start().line))
}

/// The candidate a backticked span names, or `None` when the span is
/// not a single identifier path: a dotted form, a flag, a file name,
/// a snippet with spaces or operators, a `CamelCase` or
/// `SCREAMING_SNAKE` name, a keyword, a primitive.
///
/// A trailing call group is dropped — `` `refill(&key, params)` ``
/// names `refill` — as are a macro's `!`, a `?`, a leading `&`,
/// `&mut` or `*`, and any `<…>` generic arguments
/// (`` `Tree<GfxElement, GfxMutator>::descendant_at` `` names
/// `Tree::descendant_at`).
fn classify(span: &str) -> Option<Candidate> {
    let mut s = span.trim();
    for prefix in ["&mut ", "&", "*"] {
        if let Some(stripped) = s.strip_prefix(prefix) {
            s = stripped;
        }
    }
    if s.ends_with(')') {
        let open = matching_open_paren(s)?;
        // No Rust expression begins with `--`; `var(--accent)` is CSS.
        if open == 0 || s[open + 1..].starts_with("--") {
            return None;
        }
        s = &s[..open];
    }
    s = s.strip_suffix('!').unwrap_or(s);
    s = s.strip_suffix('?').unwrap_or(s);
    // `select_sibling_*` is the glob a comment writes for a family;
    // dropping the `*` leaves the trailing `_` that reads as one.
    s = s.strip_suffix('*').unwrap_or(s);
    let ends_in_a_group = s.ends_with('>');
    let without_generics = strip_angle_groups(s)?;
    // `parse::<u8>` leaves `parse::` once its turbofish is dropped;
    // the path meant is `parse`. Only a dropped group earns the trim:
    // a span written `a::b::` is not a path.
    let s = match without_generics.strip_suffix("::") {
        Some(path) if ends_in_a_group => path,
        _ => without_generics.as_str(),
    };
    if s.is_empty()
        || !s
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == ':')
    {
        return None;
    }
    let segments: Vec<&str> = s.split("::").collect();
    if segments.iter().any(|segment| !is_identifier(segment)) {
        return None;
    }
    let name = segments[segments.len() - 1];
    if !is_snake_case_identifier(name) {
        return None;
    }
    let qualifier = match segments.len() {
        1 => Qualifier::Bare,
        n => match segments[n - 2] {
            "Self" => Qualifier::SelfType,
            q if q.starts_with(|c: char| c.is_ascii_uppercase()) => Qualifier::Type(q.to_string()),
            q => Qualifier::Module(q.to_string()),
        },
    };
    let root = (segments.len() >= 3).then(|| segments[0].to_string());
    let enclosing = (segments.len() >= 3).then(|| segments[segments.len() - 3].to_string());
    Some(Candidate {
        name: name.to_string(),
        qualifier,
        root,
        enclosing,
    })
}

/// `s` with every balanced `<…>` group that stands where generic
/// arguments stand — directly before a `::` or at the end, a
/// turbofish's `::<…>` included — removed,
/// or `None` when the angle brackets do not balance (an `a < b`
/// snippet). A group anywhere else, as in the placeholder shape
/// `test_<short>_<and_more>`, is left in place and fails the identifier
/// check, which is what keeps a shape from being read as a name.
fn strip_angle_groups(s: &str) -> Option<String> {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0usize;
    let mut group = String::new();
    let chars: Vec<char> = s.chars().collect();
    for (i, &c) in chars.iter().enumerate() {
        match c {
            '<' => {
                if depth == 0 {
                    group.clear();
                }
                depth += 1;
                group.push(c);
            }
            '>' => {
                depth = depth.checked_sub(1)?;
                group.push(c);
                if depth == 0 {
                    let generic_position = chars.get(i + 1).is_none() || s[i + 1..].starts_with("::");
                    if !generic_position {
                        out.push_str(&group);
                    }
                }
            }
            _ if depth == 0 => out.push(c),
            _ => group.push(c),
        }
    }
    (depth == 0).then_some(out)
}

/// The index of the `(` that balances the `)` ending `s`, or `None`
/// when the parentheses do not balance.
fn matching_open_paren(s: &str) -> Option<usize> {
    let mut depth = 0usize;
    for (index, byte) in s.bytes().enumerate().rev() {
        match byte {
            b')' => depth += 1,
            b'(' => {
                depth -= 1;
                if depth == 0 {
                    return Some(index);
                }
            }
            _ => {}
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three path prefixes [`scan_doc_identifiers`] must be able
    /// to show its work in. Each carries resolved candidates today, so
    /// a walk that lost a root goes red naming the prefix instead of
    /// quietly shrinking the checked set.
    const RESOLUTION_WITNESSES: &[&str] = &["lib/baumhard/src/", "src/application/", "crates/"];

    /// Run the scan over in-memory sources, each `(file, text)`, for a
    /// workspace with no packages named and `ext_crate` as its one
    /// dependency; the index is built from the same sources.
    fn scan_sources(sources: &[(&str, &str)]) -> DocIdentifierScan {
        let parsed: Vec<Source> = sources
            .iter()
            .map(|(rel, text)| Source::parse(rel, text))
            .collect();
        let external: BTreeSet<String> = ["ext_crate".to_string()].into_iter().collect();
        let index = WorkspaceIndex::from_sources(&parsed, &BTreeSet::new(), &external);
        let mut scan = DocIdentifierScan::new();
        for source in &parsed {
            scan_file(source, &index, &mut scan);
        }
        scan
    }

    fn bare(name: &str) -> Option<Candidate> {
        Some(Candidate {
            name: name.to_string(),
            qualifier: Qualifier::Bare,
            root: None,
            enclosing: None,
        })
    }

    /// The whole point of the gate, as the negative control #163 asks
    /// for: a function renamed without its doc being touched.
    #[test]
    fn test_a_renamed_function_leaves_its_doc_pointing_at_nothing_and_the_scan_says_where() {
        let before = "\
/// Calls `alpha_beta` once.
fn caller() {
    alpha_beta();
}
fn alpha_beta() {}
";
        let clean = scan_sources(&[("fixture.rs", before)]);
        assert!(clean.unresolved.is_empty(), "{:?}", clean.unresolved);
        assert_eq!(clean.resolved.get("fixture.rs"), Some(&1));

        let renamed = before.replace("alpha_beta()", "alpha_gamma()");
        assert!(
            renamed.contains("`alpha_beta`") && !renamed.contains("alpha_beta()"),
            "precondition: the rename reached the code and not the doc"
        );
        let stale = scan_sources(&[("fixture.rs", &renamed)]);
        assert_eq!(
            stale.unresolved,
            vec![
                "fixture.rs:1: `alpha_beta` — written in no file's code and declared nowhere in the workspace"
                    .to_string()
            ],
            "the report names the file, the line and the identifier"
        );

        let disclosed = renamed.replace("`alpha_beta` once", "`alpha_beta` (gone) once");
        let exempt = scan_sources(&[("fixture.rs", &disclosed)]);
        assert!(exempt.unresolved.is_empty(), "{:?}", exempt.unresolved);
        assert_eq!(exempt.exempted, vec!["fixture.rs:1: `alpha_beta`".to_string()]);
    }

    /// The sharp rule: a qualified name is held to what its type
    /// declares, not to whether the word appears somewhere in the
    /// code. `insert` is still written in this fixture's code, as
    /// `HashMap::insert` was in the file #163's headline case lived
    /// in, and the deleted method is reported anyway.
    #[test]
    fn test_a_qualified_name_is_held_to_the_types_own_declarations() {
        let source = "\
use std::collections::HashMap;
/// Writes through `Cache::insert`, then reads `Cache::get_one`.
struct Cache {
    entries: HashMap<u8, u8>,
}
impl Cache {
    fn get_one(&self) -> Option<&u8> {
        self.entries.get(&1)
    }
    fn fill(&mut self) {
        self.entries.insert(1, 2);
    }
}
";
        let scan = scan_sources(&[("cache.rs", source)]);
        assert_eq!(
            scan.unresolved,
            vec!["cache.rs:2: `Cache::insert` — `Cache` declares no `insert`".to_string()]
        );
        assert_eq!(
            scan.resolved.get("cache.rs"),
            Some(&1),
            "`Cache::get_one` resolved"
        );

        let disclosed = source.replace("`Cache::insert`,", "`Cache::insert` (gone),");
        let exempt = scan_sources(&[("cache.rs", &disclosed)]);
        assert!(exempt.unresolved.is_empty(), "{:?}", exempt.unresolved);
        assert_eq!(exempt.exempted, vec!["cache.rs:2: `Cache::insert`".to_string()]);
    }

    /// The marker makes a claim, and on a qualified name the claim is
    /// checkable: `(gone)` on a method the type still declares is the
    /// prose lying in the other direction.
    #[test]
    fn test_a_gone_marker_on_a_name_the_type_still_declares_is_reported() {
        let source = "\
/// Replaced `Cache::fill` (gone) with nothing yet.
struct Cache;
impl Cache {
    fn fill(&mut self) {}
}
";
        let scan = scan_sources(&[("cache.rs", source)]);
        assert_eq!(
            scan.unresolved,
            vec!["cache.rs:1: `Cache::fill` — marked (gone), but the sources still declare it".to_string()]
        );
        assert!(scan.exempted.is_empty());
    }

    /// A bare name resolves through any token the workspace's code
    /// writes, which is too loose to contradict a reader, so a bare
    /// `(gone)` is taken at its word.
    #[test]
    fn test_a_gone_marker_on_a_bare_name_is_taken_at_its_word() {
        let source = "\
/// The `old_insert` (gone) that used to live here.
fn fill(map: &mut std::collections::HashMap<u8, u8>) {
    let old_insert = 1;
    map.insert(old_insert, 2);
}
";
        let scan = scan_sources(&[("cache.rs", source)]);
        assert!(scan.unresolved.is_empty(), "{:?}", scan.unresolved);
        assert_eq!(scan.exempted, vec!["cache.rs:1: `old_insert`".to_string()]);
    }

    /// The marker must follow the span directly. Written anywhere
    /// else in the sentence it exempts nothing, so a reader can tell
    /// which name it is about.
    #[test]
    fn test_the_gone_marker_exempts_only_the_span_it_follows() {
        let source = "\
/// Both `one_thing` and `other_thing` are (gone).
fn remains() {}
";
        let scan = scan_sources(&[("f.rs", source)]);
        assert_eq!(scan.unresolved.len(), 2, "{:?}", scan.unresolved);
        assert!(scan.exempted.is_empty());
    }

    /// `Self::name` is `Type::name` for the enclosing `impl`, in a
    /// field's doc for the enclosing struct, and bare where there is
    /// no enclosing type to mean.
    #[test]
    fn test_self_resolves_against_the_enclosing_impl_or_struct() {
        let source = "\
struct Widget {
    /// Read by `Self::spin_up`, cleared by `Self::wind_down`.
    turns: u32,
}
impl Widget {
    /// See `Self::spin_up`.
    fn other(&self) {}
    fn spin_up(&self) {}
}
/// `Self::spin_up` means nothing here; `spin_up` is in this file.
fn free() {}
";
        let scan = scan_sources(&[("w.rs", source)]);
        assert_eq!(
            scan.unresolved,
            vec!["w.rs:2: `Self::wind_down` — `Widget` declares no `wind_down`".to_string()]
        );
        assert_eq!(scan.resolved.get("w.rs"), Some(&4));
    }

    /// A type the workspace does not declare — `HashMap`,
    /// `serde_json`, a primitive — is not this scan's to judge.
    #[test]
    fn test_an_external_qualifier_is_not_a_candidate() {
        let source = "\
/// Uses `HashMap::entry`, `serde_json::from_str`, `f32::clamp`,
/// `str::chars`, `Vec::with_capacity` and `std::mem::take`.
fn uses() {}
";
        let scan = scan_sources(&[("e.rs", source)]);
        assert!(scan.unresolved.is_empty(), "{:?}", scan.unresolved);
        assert!(
            scan.resolved.is_empty(),
            "an external name is neither resolved nor not"
        );
    }

    /// A workspace type that shares its name with an external one is
    /// told apart by the file: where the file imports the name from
    /// outside the workspace, or the doc writes the external root,
    /// the span is the external type's.
    #[test]
    fn test_an_imported_or_rooted_external_type_shadows_a_workspace_type_of_the_same_name() {
        let ours = "\
/// Our `Cell::set` and `Cell::take_one`.
pub struct Cell(u8);
impl Cell {
    pub fn take_one(&self) -> u8 {
        self.0
    }
}
";
        let theirs = "\
use std::cell::Cell;
/// Std's `Cell::set`, and `ext_crate::Buffer::set_rich_text` and `std::sync::mpsc`.
fn uses(c: &Cell<u8>) {
    c.set(1);
}
pub struct Buffer;
mod sync {}
";
        let scan = scan_sources(&[("ours.rs", ours), ("theirs.rs", theirs)]);
        assert_eq!(
            scan.unresolved,
            vec!["ours.rs:1: `Cell::set` — `Cell` declares no `set`".to_string()],
            "only the file that means the workspace `Cell` is held to it"
        );
        assert_eq!(
            scan.resolved.get("theirs.rs"),
            None,
            "every span in `theirs.rs` is external"
        );
    }

    /// `Enum::Variant::field` is how rustdoc links a variant's field,
    /// and the variant is no type: the field is held to the enum.
    #[test]
    fn test_an_enum_variant_path_is_held_to_the_enum() {
        let source = "\
/// See `Shape::Boxed::corner_radius`, never `Shape::Boxed::corner_count`.
enum Shape {
    Boxed { corner_radius: f32 },
    Round,
}
";
        let scan = scan_sources(&[("s.rs", source)]);
        assert_eq!(
            scan.unresolved,
            vec!["s.rs:1: `Shape::Boxed::corner_count` — `Shape` declares no `corner_count`".to_string()]
        );
        assert_eq!(scan.resolved.get("s.rs"), Some(&1));

        // A variant that wraps a type of the same name: the field is
        // the variant's, so the enum answers when the type cannot.
        let wrapped = "\
/// See `Element::Area::subtree_aabb` and `Element::Area::scale`, never `Element::Area::spin`.
struct Area {
    scale: f32,
}
enum Element {
    Area { inner: Area, subtree_aabb: f32 },
}
";
        let scan = scan_sources(&[("w.rs", wrapped)]);
        assert_eq!(
            scan.unresolved,
            vec!["w.rs:1: `Element::Area::spin` — `Area` declares no `spin`".to_string()]
        );
        assert_eq!(scan.resolved.get("w.rs"), Some(&2));
    }

    /// A path under `crate::`, `super::`, `self::` or a workspace
    /// package can mean nothing but the workspace, so a module or type
    /// in it that the workspace does not declare is a vanished name,
    /// not a foreign one — the renamed-type half of #163.
    #[test]
    fn test_an_internal_root_holds_every_segment_to_the_workspace() {
        let probe = "\
/// `crate::util::probe_gone_module::probe_gone_fn`,
/// `crate::gfx_structs::ProbeGoneType::probe_gone_method`,
/// `super::probe_gone_module::probe_gone_fn` and `baumhard::ProbeGoneType::probe_gone_method`;
/// `crate::gfx_structs::Shape::corner_count` and `crate::util::geometry::almost_equal` are
/// controls, and `std::probe_gone::probe_gone_fn` is external.
fn probe() {}
";
        let real = "\
pub struct Shape;
impl Shape {
    pub fn corner_count(&self) -> u8 {
        4
    }
}
pub fn almost_equal() {}
";
        let parsed: Vec<Source> = [
            ("lib/baumhard/src/util/probe.rs", probe),
            ("lib/baumhard/src/gfx_structs/geometry.rs", real),
        ]
        .iter()
        .map(|(rel, text)| Source::parse(rel, text))
        .collect();
        let crates: BTreeSet<String> = ["baumhard".to_string()].into_iter().collect();
        let index = WorkspaceIndex::from_sources(&parsed, &crates, &BTreeSet::new());
        let mut scan = DocIdentifierScan::new();
        for source in &parsed {
            scan_file(source, &index, &mut scan);
        }
        assert_eq!(
            scan.unresolved,
            vec![
                "lib/baumhard/src/util/probe.rs:1: `crate::util::probe_gone_module::probe_gone_fn` — \
                 `probe_gone_module` is declared nowhere in the workspace"
                    .to_string(),
                "lib/baumhard/src/util/probe.rs:2: `crate::gfx_structs::ProbeGoneType::probe_gone_method` — \
                 `ProbeGoneType` is declared nowhere in the workspace"
                    .to_string(),
                "lib/baumhard/src/util/probe.rs:3: `super::probe_gone_module::probe_gone_fn` — \
                 `probe_gone_module` is declared nowhere in the workspace"
                    .to_string(),
                "lib/baumhard/src/util/probe.rs:3: `baumhard::ProbeGoneType::probe_gone_method` — \
                 `ProbeGoneType` is declared nowhere in the workspace"
                    .to_string(),
            ]
        );
        assert_eq!(
            scan.resolved.get("lib/baumhard/src/util/probe.rs"),
            Some(&2),
            "the two controls resolve and the std-rooted probe is not judged"
        );
    }

    /// Generic arguments are dropped before the path is read, so
    /// `Tree<GfxElement, GfxMutator>::descendant_at` is held to `Tree`,
    /// and a turbofish is dropped with the `::` that carried it, so
    /// `parse_all::<u8>` is held as `parse_all`.
    #[test]
    fn test_generic_arguments_and_a_turbofish_are_dropped_before_the_path_is_read() {
        let source = "\
/// `Tree<Elem, Mut>::descendant_at` and `Tree<Elem>::probe_gone` and `parse_all::<u8>`;
/// `probe_gone_fn::<T>` and `Tree::probe_gone::<T>` are nobody's.
struct Tree<E, M>(E, M);
impl<E, M> Tree<E, M> {
    fn descendant_at(&self) {}
}
fn parse_all() {}
";
        let scan = scan_sources(&[("t.rs", source)]);
        assert_eq!(
            scan.unresolved,
            vec![
                "t.rs:1: `Tree<Elem>::probe_gone` — `Tree` declares no `probe_gone`".to_string(),
                "t.rs:2: `probe_gone_fn::<T>` — written in no file's code and declared nowhere in the \
                 workspace"
                    .to_string(),
                "t.rs:2: `Tree::probe_gone::<T>` — `Tree` declares no `probe_gone`".to_string(),
            ]
        );
        assert_eq!(scan.resolved.get("t.rs"), Some(&2));
        assert_eq!(
            classify("parse_all::<u8>"),
            Some(Candidate {
                name: "parse_all".into(),
                qualifier: Qualifier::Bare,
                root: None,
                enclosing: None,
            })
        );
        assert_eq!(
            classify("Tree<GfxElement, GfxMutator>::descendant_at"),
            Some(Candidate {
                name: "descendant_at".into(),
                qualifier: Qualifier::Type("Tree".into()),
                root: None,
                enclosing: None,
            })
        );
        assert_eq!(classify("a < b"), None);
        assert_eq!(
            classify("test_<short>_<and_more>"),
            None,
            "a placeholder shape is not a name, however its angle brackets balance"
        );
    }

    /// `T::void()` names a generic parameter's associated function,
    /// which belongs to no type the workspace declares.
    #[test]
    fn test_a_generic_parameter_is_not_a_type_the_scan_judges() {
        let source = "\
/// Only `T::void()` and `F2::call_it` are needed.
fn generic<T, F2>() {}
";
        let scan = scan_sources(&[("g.rs", source)]);
        assert!(scan.unresolved.is_empty(), "{:?}", scan.unresolved);
    }

    /// A module-qualified name is held to the workspace's
    /// declarations and code, and a module the workspace declares is
    /// one named by a `mod` item or by a source file's own path.
    #[test]
    fn test_a_module_qualified_name_must_exist_somewhere() {
        let geometry = "pub fn almost_equal(a: f32, b: f32) -> bool { a == b }\n";
        let user = "\
/// Compares with `geometry::almost_equal`, never `geometry::nearly_equal`;
/// `crate::util::geometry::almost_equal` is the same function.
fn compare() {}
";
        let scan = scan_sources(&[("lib/util/geometry.rs", geometry), ("lib/user.rs", user)]);
        assert_eq!(
            scan.unresolved,
            vec![
                "lib/user.rs:1: `geometry::nearly_equal` — `nearly_equal` is written in no file's code and \
                  declared nowhere in the workspace"
                    .to_string()
            ]
        );
        assert_eq!(scan.resolved.get("lib/user.rs"), Some(&2));
    }

    /// A bare name exists when any file's code writes it — a field
    /// it touches, a local it binds, a function it calls — or when
    /// some file declares it. A single word is not read at all.
    #[test]
    fn test_a_bare_name_resolves_through_any_files_code_or_a_declaration() {
        let other = "\
pub fn declared_elsewhere() {}
fn local_user() {
    let only_in_other_code = 1;
    let _ = only_in_other_code;
}
";
        let source = "\
/// Touches `cache_entries`, calls `declared_elsewhere`, binds `only_in_other_code`
/// somewhere else, names `never_anywhere`, and `insert` is one word.
fn touch(cache: &mut Cache) {
    cache.cache_entries.clear();
}
struct Cache {
    cache_entries: Vec<u8>,
}
";
        let scan = scan_sources(&[("a.rs", source), ("b.rs", other)]);
        assert_eq!(
            scan.unresolved,
            vec![
                "a.rs:2: `never_anywhere` — written in no file's code and declared nowhere in the workspace"
                    .to_string()
            ]
        );
        assert_eq!(scan.resolved.get("a.rs"), Some(&3));
    }

    /// A trailing underscore is a family, as the citation scan reads
    /// it: `max_` exists while anything it prefixes does.
    #[test]
    fn test_a_trailing_underscore_is_a_family_prefix() {
        let source = "\
/// The `max_` and `min_` fields, and the `zzz_` family that is not.
struct Config {
    max_font_size_pt: f32,
}
";
        let scan = scan_sources(&[("c.rs", source)]);
        assert_eq!(
            scan.unresolved,
            vec![
                "c.rs:1: `min_` — written in no file's code and declared nowhere in the workspace"
                    .to_string(),
                "c.rs:1: `zzz_` — written in no file's code and declared nowhere in the workspace"
                    .to_string(),
            ]
        );
        assert_eq!(scan.resolved.get("c.rs"), Some(&1));
    }

    /// A method a std trait supplies resolves on the type that derives
    /// or implements that trait, and on no other; a default method a
    /// workspace trait supplies resolves on its implementors.
    #[test]
    fn test_std_and_trait_default_methods_resolve_only_on_the_implementing_type() {
        let source = "\
/// `Cache::default` is the derive's, `Cache::to_string` the `Display` impl's,
/// `Cache::describe` the trait body's; `Cache::iter` and `Cache::clone` are
/// nobody's, and `Bare::default` has no derive to come from.
#[derive(Default)]
struct Cache;
impl std::fmt::Display for Cache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, \"\")
    }
}
trait Describe {
    fn describe(&self) -> &'static str {
        \"\"
    }
}
impl Describe for Cache {}
struct Bare;
";
        let scan = scan_sources(&[("c.rs", source)]);
        assert_eq!(
            scan.unresolved,
            vec![
                "c.rs:2: `Cache::iter` — `Cache` declares no `iter`".to_string(),
                "c.rs:2: `Cache::clone` — `Cache` declares no `clone`".to_string(),
                "c.rs:3: `Bare::default` — `Bare` declares no `default`".to_string(),
            ]
        );
        assert_eq!(scan.resolved.get("c.rs"), Some(&3));

        // strum's `EnumIter` supplies `iter`; `derivative` lists the
        // traits it implements in its own attribute, options included.
        let derived = "\
/// `Kind::iter`, `Area::eq` and `Area::fmt`; `Kind::eq` is nobody's.
#[derive(EnumIter)]
enum Kind {
    A,
}
#[derive(Derivative)]
#[derivative(PartialEq, Debug = \"transparent\")]
struct Area;
";
        let scan = scan_sources(&[("d.rs", derived)]);
        assert_eq!(
            scan.unresolved,
            vec!["d.rs:1: `Kind::eq` — `Kind` declares no `eq`".to_string()]
        );
        assert_eq!(scan.resolved.get("d.rs"), Some(&3));
    }

    /// A `macro_rules!` body is opaque to `syn`; the text harvest is
    /// what lets a declaration written inside one count — as a bare
    /// name, and as a member of a type the macro also wrote, since the
    /// parse cannot say which type the macro meant and a type the
    /// macro wrote is where it can have put the member.
    #[test]
    fn test_a_declaration_a_macro_body_emits_still_counts() {
        let maker = "\
macro_rules! make {
    () => {
        fn made_by_macro() {}
        pub struct Config;
        impl Config {
            pub fn known_keys() -> Vec<&'static str> {
                Vec::new()
            }
        }
    };
}
make!();
";
        let user =
            "/// Calls `made_by_macro` and `Config::known_keys`, never `Config::lost_keys`.\nfn user() {}\n";
        let scan = scan_sources(&[("maker.rs", maker), ("user.rs", user)]);
        assert_eq!(
            scan.unresolved,
            vec!["user.rs:1: `Config::lost_keys` — `Config` declares no `lost_keys`".to_string()]
        );
        assert_eq!(scan.resolved.get("user.rs"), Some(&2));

        // A type a macro wrote is held to what its literal headers
        // list: `default` from the `impl Default for Config` inside the
        // macro, `made_by_macro` as a name written under no header that
        // a macro can have put anywhere, and nothing the parse declared
        // elsewhere — `elsewhere` is a plain function, not a member.
        let derived_in_macro = "\
macro_rules! make {
    () => {
        pub struct Config;
        impl Default for Config {
            fn default() -> Self {
                Config
            }
        }
        fn made_by_macro() {}
    };
}
make!();
fn elsewhere() {}
/// `Config::default()` is the macro's impl and `Config::made_by_macro` may be;
/// `Config::elsewhere` and `Config::never_declared` are nobody's.
fn user() {}
";
        let scan = scan_sources(&[("m.rs", derived_in_macro)]);
        assert_eq!(
            scan.unresolved,
            vec![
                "m.rs:15: `Config::elsewhere` — `Config` declares no `elsewhere`".to_string(),
                "m.rs:15: `Config::never_declared` — `Config` declares no `never_declared`".to_string(),
            ]
        );
        assert_eq!(scan.resolved.get("m.rs"), Some(&2));

        // A macro-written type with a member that is itself a macro
        // parameter has no listable members — `bindings` is a token at
        // the invocation — so any name the workspace declares or writes
        // resolves on it, and a name written nowhere still does not.
        let open = "\
macro_rules! make_open {
    ($field:ident) => {
        pub struct Open {
            pub $field: u8,
        }
    };
}
make_open!(bindings);
fn elsewhere() {}
/// `Open::bindings` and `Open::elsewhere` cannot be ruled out; `Open::never_declared` can.
fn user() {}
";
        let scan = scan_sources(&[("o.rs", open)]);
        assert_eq!(
            scan.unresolved,
            vec!["o.rs:10: `Open::never_declared` — `Open` declares no `never_declared`".to_string()]
        );
        assert_eq!(scan.resolved.get("o.rs"), Some(&2));
    }

    /// A macro-emitted name resolves only where a macro can have put
    /// it: on the type whose literal header the text read it under, on
    /// a type with a custom derive when it was written under no header,
    /// or on a type a macro wrote. On a plainly parsed type without a
    /// custom derive, `Type::name` is held to what the parse saw plus
    /// what a macro body writes under that type's own header, so a
    /// macro elsewhere cannot vouch for a member every type would then
    /// be granted.
    #[test]
    fn test_a_macro_emitted_member_resolves_only_on_a_macro_written_or_custom_derived_type() {
        let source = "\
macro_rules! make {
    () => {
        fn probe_emitted_fn() {}
        impl Host {
            pub fn emitted_on_host() {}
        }
    };
}
make!();
/// `Plain::probe_emitted_fn` and `Plain::emitted_on_host` are nobody's;
/// `Classified::probe_emitted_fn` may be the derive's, and
/// `Host::emitted_on_host` is the macro's; `Classified::emitted_on_host`
/// is nobody's, since the macro wrote it under `Host`.
struct Plain;
#[derive(Clone, ActionClassify)]
struct Classified;
struct Host;
";
        let scan = scan_sources(&[("m.rs", source)]);
        assert_eq!(
            scan.unresolved,
            vec![
                "m.rs:10: `Plain::probe_emitted_fn` — `Plain` declares no `probe_emitted_fn`".to_string(),
                "m.rs:10: `Plain::emitted_on_host` — `Plain` declares no `emitted_on_host`".to_string(),
                "m.rs:12: `Classified::emitted_on_host` — `Classified` declares no `emitted_on_host`"
                    .to_string(),
            ]
        );
        assert_eq!(scan.resolved.get("m.rs"), Some(&2));

        // The header reader behind the attribution, line by line.
        assert_eq!(header_frame("impl Config {"), Some(Frame::Owned("Config".into())));
        assert_eq!(
            header_frame("impl<'a, T: Clone> Describe<T> for &'a mut a::b::Config<T> where T: Copy {"),
            Some(Frame::Owned("Config".into()))
        );
        assert_eq!(
            header_frame("pub(crate) struct Config {"),
            Some(Frame::Owned("Config".into()))
        );
        assert_eq!(
            header_frame("pub unsafe trait Describe {"),
            Some(Frame::Owned("Describe".into()))
        );
        assert_eq!(header_frame("impl $name {"), Some(Frame::Parameter));
        assert_eq!(header_frame("impl Default for #ident {"), Some(Frame::Parameter));
        assert_eq!(header_frame("impl_marker!(Config);"), None);
        assert_eq!(header_frame("let s = Config {"), None);
        assert_eq!(header_frame("fn config() {"), None);
        assert_eq!(
            derive_line("#[derive(Clone, strum::EnumIter)]"),
            Some(vec!["Clone".into(), "EnumIter".into()])
        );
        assert_eq!(
            derive_line("#[derivative(PartialEq, Debug = \"transparent\")]"),
            Some(vec!["PartialEq".into(), "Debug".into()])
        );
        assert_eq!(derive_line("#[cfg(test)]"), None);
        assert!(declares_a_parameter("pub $field: Vec<u8>,"));
        assert!(declares_a_parameter("fn $method(&self) {}"));
        assert!(declares_a_parameter("pub(crate) const #name: u8 = 1;"));
        assert!(!declares_a_parameter("pub field: Vec<$ty>,"));
        assert!(!declares_a_parameter("let $x = 1;"));
    }

    /// `blank_string_literals` keeps char literals, so a `'{'` in code
    /// would be counted as an opening brace and leave the enclosing
    /// `impl` frame open to the end of the file: every later macro
    /// member would be attributed to that type, and leave the unscoped
    /// set a custom derive resolves. The brace literals are removed
    /// before the count; the `'x'` control shows the attribution the
    /// fixture gets when nothing leaks.
    #[test]
    fn test_a_brace_char_literal_does_not_open_a_frame() {
        let fixture = |literal: &str| {
            format!(
                "\
struct Leaky;
impl Leaky {{
    pub fn count(&self, c: char) -> u8 {{
        match c {{
            {literal} => 1,
            _ => 0,
        }}
    }}
}}
macro_rules! probe_leak {{
    () => {{
        fn probe_leaked_fn() {{}}
    }};
}}
probe_leak!();
/// `Leaky::probe_leaked_fn` is nobody's; `Classified::probe_leaked_fn` may be the derive's.
#[derive(ActionClassify)]
pub struct Classified;
"
            )
        };
        let expected =
            vec!["l.rs:16: `Leaky::probe_leaked_fn` — `Leaky` declares no `probe_leaked_fn`".to_string()];
        let control = scan_sources(&[("l.rs", &fixture("'x'"))]);
        assert_eq!(
            control.unresolved, expected,
            "control: no brace literal, nothing leaks"
        );
        assert_eq!(control.resolved.get("l.rs"), Some(&1));
        for literal in ["'{'", "'}'", "b'{'", "b'}'"] {
            let scan = scan_sources(&[("l.rs", &fixture(literal))]);
            assert_eq!(
                scan.unresolved, expected,
                "a {literal} literal must not open or close a frame"
            );
            assert_eq!(scan.resolved.get("l.rs"), Some(&1), "{literal}");
        }
        assert_eq!(
            without_brace_char_literals("match c { '{' => 1, b'}' => 2 }"),
            "match c {  => 1,  => 2 }"
        );
        assert_eq!(without_brace_char_literals("'\\u{7b}'"), "'\\u{7b}'");
    }

    /// A `#[derive(…)]` rustfmt wrapped over several lines above a
    /// literal header in a macro body is gathered and read as one; the
    /// control is the same header with the trait left out of the list.
    #[test]
    fn test_a_wrapped_derive_in_a_macro_body_grants_its_traits() {
        let fixture = |traits: &str| {
            format!(
                "\
macro_rules! make {{
    () => {{
        #[derive(
            {traits}
        )]
        pub struct Wrapped;
    }};
}}
make!();
/// `Wrapped::default()` and `Wrapped::clone()`; `Wrapped::never_declared` is nobody's.
fn user() {{}}
"
            )
        };
        let scan = scan_sources(&[("w.rs", &fixture("Clone,\n            Default,"))]);
        assert_eq!(
            scan.unresolved,
            vec!["w.rs:11: `Wrapped::never_declared` — `Wrapped` declares no `never_declared`".to_string()]
        );
        assert_eq!(scan.resolved.get("w.rs"), Some(&2));
        let control = scan_sources(&[("w.rs", &fixture("Clone,"))]);
        assert_eq!(
            control.unresolved,
            vec![
                "w.rs:10: `Wrapped::default()` — `Wrapped` declares no `default`".to_string(),
                "w.rs:10: `Wrapped::never_declared` — `Wrapped` declares no `never_declared`".to_string(),
            ],
            "control: a trait the wrapped list does not name is not granted"
        );
        assert!(is_derive_opening("#[derive("));
        assert!(is_derive_opening("#[derivative(Debug)]"));
        assert!(!is_derive_opening("#[cfg(test)]"));
    }

    /// A wrapped derive that never closes stops gathering, and the text
    /// after it is still harvested: at the first line that cannot
    /// continue a derive list, or at `MAX_WRAPPED_DERIVE_LINES` when
    /// every line could. Before the cap the gather ran to the end of
    /// the file, so nothing after a malformed derive was declared.
    /// "Never closes" is the gather's view — no line ends in `)]` —
    /// since the fixtures must still tokenize as Rust.
    #[test]
    fn test_a_never_closing_wrapped_derive_stops_and_the_rest_is_harvested() {
        // Stopped by a line a derive list cannot hold.
        let stopped = "\
macro_rules! make {
    () => {
        #[derive(
            Clone)] pub struct Broken;
        pub struct After {
            pub after_field: u8,
        }
    };
}
make!();
/// `After::after_field` is declared after the broken derive.
fn user() {}
";
        let scan = scan_sources(&[("s.rs", stopped)]);
        assert_eq!(
            scan.unresolved,
            Vec::<String>::new(),
            "the struct after the broken derive is harvested"
        );
        assert_eq!(scan.resolved.get("s.rs"), Some(&1));

        // Stopped only by the cap: every gathered line could continue
        // a derive list, and the field past the cap is still read.
        let filler = "            Clone,\n".repeat(MAX_WRAPPED_DERIVE_LINES + 4);
        let capped = format!(
            "\
macro_rules! make {{
    () => {{
        pub struct Capped {{
            #[derive(
{filler}            pub capped_field: u8,
            ) ]
            pub other_field: u8,
        }}
    }};
}}
make!();
/// `Capped::capped_field` is past the cap.
fn user() {{}}
"
        );
        assert!(
            capped
                .lines()
                .all(|line| !line.contains("capped_field:") || can_continue_derive(line.trim())),
            "the field line must be one the shape check alone would keep gathering"
        );
        let scan = scan_sources(&[("c.rs", &capped)]);
        assert_eq!(
            scan.unresolved,
            Vec::<String>::new(),
            "the field past the cap is harvested"
        );
        assert_eq!(scan.resolved.get("c.rs"), Some(&1));
    }

    /// strum's `EnumDiscriminants` writes a mirror enum the parse never
    /// sees, but its declaration is a function of the parsed one: the
    /// variants are the enum's, the traits are strum's defaults plus
    /// the attribute's `derive(…)`, and a custom derive on the enum
    /// writes onto the mirror. So the mirror is held like a parsed
    /// type, not like a macro-written one.
    #[test]
    fn test_strums_discriminants_mirror_is_held_to_the_enums_variants_and_listed_derives() {
        let source = "\
/// `Kind::iter()`, `Kind::clone()` and `Kind::classify` are the mirror's;
/// `Kind::next` and `Kind::nothing` are nobody's, and `Plain::iter()` has
/// no `EnumIter` to come from.
#[derive(EnumDiscriminants, ActionClassify)]
#[strum_discriminants(name(Kind), derive(Hash, EnumIter), doc = \"mirror\")]
enum Action {
    Alpha,
    Beta(u8),
}
#[derive(EnumDiscriminants)]
#[strum_discriminants(name(Plain))]
enum Other {
    Gamma,
}
macro_rules! write_classifier {
    () => {
        impl ActionClassify for #name {
            fn classify(&self) {}
        }
    };
}
";
        let scan = scan_sources(&[("k.rs", source)]);
        assert_eq!(
            scan.unresolved,
            vec![
                "k.rs:2: `Kind::next` — `Kind` declares no `next`".to_string(),
                "k.rs:2: `Kind::nothing` — `Kind` declares no `nothing`".to_string(),
                "k.rs:2: `Plain::iter()` — `Plain` declares no `iter`".to_string(),
            ]
        );
        assert_eq!(scan.resolved.get("k.rs"), Some(&3));
    }

    /// A name `syn` declares in a later file is not macro-emitted just
    /// because an earlier file's macro body also spells it; the set is
    /// the difference over the whole workspace, not a running one.
    #[test]
    fn test_the_macro_emitted_set_is_taken_over_the_whole_workspace() {
        let early = "\
macro_rules! make {
    () => {
        fn shared_name() {}
    };
}
make!();
/// `Widget::shared_name` may be the derive's only while no parse declares it.
#[derive(ActionClassify)]
struct Widget;
";
        let late = "fn shared_name() {}\n";
        let alone = scan_sources(&[("a_early.rs", early)]);
        assert!(
            alone.unresolved.is_empty(),
            "precondition: with the name declared only in the macro body, the custom derive vouches \
             for it: {:?}",
            alone.unresolved
        );
        let scan = scan_sources(&[("a_early.rs", early), ("b_late.rs", late)]);
        assert_eq!(
            scan.unresolved,
            vec!["a_early.rs:7: `Widget::shared_name` — `Widget` declares no `shared_name`".to_string()],
            "a parsed declaration anywhere takes the name out of the macro-emitted set"
        );
    }

    /// A `pub` field a macro writes belongs to a struct the parse never
    /// sees; the text harvest reads the `pub name:` line, and no other
    /// `name:` line, so `Type::field` resolves for it.
    #[test]
    fn test_a_pub_field_a_macro_emits_still_counts() {
        let source = "\
macro_rules! make_config {
    () => {
        pub struct Config {
            pub macro_bindings: Vec<u8>,
            private_bindings: Vec<u8>,
        }
    };
}
make_config!();
/// `Config::macro_bindings` exists; `Config::private_bindings` is invisible; the
/// `literal_field` below is a struct literal's, not a declaration.
fn build() {
    let _ = Other { literal_field: 1 };
}
struct Other {
    other_field: u8,
}
";
        let scan = scan_sources(&[("c.rs", source)]);
        assert_eq!(
            scan.unresolved,
            vec!["c.rs:10: `Config::private_bindings` — `Config` declares no `private_bindings`".to_string()]
        );
        assert_eq!(pub_field_name("    pub(crate) name: String,"), Some("name"));
        assert_eq!(pub_field_name("    pub name: String,"), Some("name"));
        assert_eq!(pub_field_name("    pub fn name() {}"), None);
        assert_eq!(pub_field_name("    pub const NAME: u8 = 1;"), None);
        assert_eq!(pub_field_name("    pub use a::b;"), None);
        assert_eq!(pub_field_name("    name: String,"), None);
        assert_eq!(pub_field_name("pub mod x;"), None);
    }

    /// A declaration written inside a string literal is not one: the
    /// text harvest runs on string-blanked code, so a test fixture
    /// like the ones in this module declares no type and no member to
    /// the index. The bare rule keeps strings on purpose — a JSON key
    /// or a criterion id lives in one — so the same word resolves
    /// bare.
    #[test]
    fn test_a_declaration_inside_a_string_literal_declares_nothing() {
        let source = "\
/// Names `Phantom::ghost_fn`, `Phantom::other_fn` and `ghost_fn`.
const FIXTURE: &str = \"struct Phantom; impl Phantom { fn ghost_fn() {} }\";
";
        let scan = scan_sources(&[("s.rs", source)]);
        assert!(
            scan.unresolved.is_empty(),
            "`Phantom` is no type the index knows, so both qualified spans are external: {:?}",
            scan.unresolved
        );
        assert_eq!(
            scan.resolved.get("s.rs"),
            Some(&1),
            "only the bare `ghost_fn` is read, and it resolves through the string"
        );
    }

    /// Only a backticked identifier path is a candidate. Everything a
    /// doc comment also puts in backticks — flags, file names, JSON
    /// paths, CSS, snippets, types, constants, keywords, primitives —
    /// is not read, and the ones that are read lose their call group,
    /// macro bang and reference sigil.
    #[test]
    fn test_only_backticked_identifier_paths_are_candidates() {
        for not_a_candidate in [
            "test.sh",
            "node.position",
            "clock.wait",
            "--workspace",
            "var(--accent)",
            "Vec<Vec2>",
            "MAX_MAP_BYTES",
            "MindMap",
            "f32",
            "true",
            "null",
            "cfg(test)",
            "let x = 1;",
            "a::b::",
            "::leading",
            "(gone)",
            "",
            "   ",
            "0.6",
            "p=0.00",
        ] {
            assert_eq!(
                classify(not_a_candidate),
                None,
                "`{not_a_candidate}` must not be a candidate"
            );
        }
        assert_eq!(classify("refill"), bare("refill"));
        assert_eq!(classify("refill()"), bare("refill"));
        assert_eq!(classify("refill(&key, params)"), bare("refill"));
        assert_eq!(classify("retain_keys(&HashSet<EdgeKey>)"), bare("retain_keys"));
        assert_eq!(classify("format!"), bare("format"));
        assert_eq!(classify("select_sibling_*"), bare("select_sibling_"));
        assert_eq!(classify("&mut entries"), bare("entries"));
        assert_eq!(classify("*cached"), bare("cached"));
        assert_eq!(
            classify("Self::refill"),
            Some(Candidate {
                name: "refill".into(),
                qualifier: Qualifier::SelfType,
                root: None,
                enclosing: None,
            })
        );
        assert_eq!(
            classify("SceneConnectionCache::refill()"),
            Some(Candidate {
                name: "refill".into(),
                qualifier: Qualifier::Type("SceneConnectionCache".into()),
                root: None,
                enclosing: None,
            })
        );
        assert_eq!(
            classify("crate::util::geometry::almost_equal"),
            Some(Candidate {
                name: "almost_equal".into(),
                qualifier: Qualifier::Module("geometry".into()),
                root: Some("crate".into()),
                enclosing: Some("util".into()),
            })
        );
        assert_eq!(
            classify("super::release"),
            Some(Candidate {
                name: "release".into(),
                qualifier: Qualifier::Module("super".into()),
                root: None,
                enclosing: None,
            })
        );
    }

    /// Half a name is not a name. A span broken across two doc lines
    /// leaves an unpaired backtick on each: the first line stops
    /// reading at it, and the second line's pairing is off by one, so
    /// it reads prose as spans and finds no candidate in them. Both
    /// fail toward silence, never toward a report.
    #[test]
    fn test_a_span_broken_across_two_lines_is_not_read() {
        let source = "\
/// Named across `a_very_long_function_name_that_wrapped_onto_the_next_line_of_the
/// _doc_comment` and then `real_one` on the same line, but
/// `real_one` here is read.
fn real_one() {}
";
        let scan = scan_sources(&[("w.rs", source)]);
        assert!(scan.unresolved.is_empty(), "{:?}", scan.unresolved);
        assert_eq!(
            scan.resolved.get("w.rs"),
            Some(&1),
            "line 2's `real_one` falls between mispaired backticks; line 3's is read"
        );
    }

    /// Every doc comment the item walk reaches is read at its own
    /// line: the file header, a nested module's header, a trait item,
    /// an enum variant, a field, and a doc after a blank doc line.
    #[test]
    fn test_doc_lines_are_read_wherever_items_carry_them_and_at_their_own_lines() {
        let source = "\
//! Header names `header_gone`.
mod inner {
    //! Inner header names `inner_gone`.
    trait T {
        /// Trait item names `trait_gone`.
        fn f(&self);
    }
    enum E {
        /// Variant names `variant_gone`.
        A {
            /// Field names `field_gone`.
            x: u8,
        },
    }
    fn uses(_: E, _: &dyn T) {}
}
/// First line.
///
/// Third line names `third_gone`.
fn documented() {}
";
        let scan = scan_sources(&[("l.rs", source)]);
        let lines: Vec<&str> = scan
            .unresolved
            .iter()
            .map(|s| s.split(" — ").next().unwrap_or(s))
            .collect();
        assert_eq!(
            lines,
            vec![
                "l.rs:1: `header_gone`",
                "l.rs:3: `inner_gone`",
                "l.rs:5: `trait_gone`",
                "l.rs:9: `variant_gone`",
                "l.rs:11: `field_gone`",
                "l.rs:19: `third_gone`",
            ]
        );
    }

    /// **Every `snake_case` identifier a doc comment writes in
    /// backticks is one the sources declare, or says it is gone.**
    ///
    /// Nine references had rotted across one series before this
    /// existed (#163), all of the same shape as the two the citation
    /// scan was built for: something was deleted or renamed and the
    /// prose that pointed at it was not. The dead spellings are not
    /// written here, because this scan would report them.
    ///
    /// A stale name is not a typo. These comments are how a reader
    /// learns why the current shape exists, and a pointer into
    /// nothing either sends them looking or, when it sits in a
    /// control instruction, has them run a check that proves
    /// something else. The module header lists what this does not
    /// catch; the first item there is why a deletion of public
    /// surface still gets a sweep.
    #[test]
    fn test_no_doc_names_an_identifier_that_does_not_exist() {
        let scan = scan_doc_identifiers();
        assert!(
            scan.unresolved.is_empty(),
            "{} doc span(s) name an identifier the sources do not declare. Either the item was \
             renamed or deleted and the prose was not — fix the prose — or the prose means the \
             vanished thing on purpose: write `{GONE_MARKER}` directly after the closing backtick \
             and it reads as history:\n  {}",
            scan.unresolved.len(),
            scan.unresolved.join("\n  ")
        );
    }

    /// The scan above answers "nothing is wrong" just as loudly when
    /// it has stopped looking, so what it *did* read is pinned: each
    /// witness prefix carries resolved spans today, and the marker is
    /// in use, so a root that moved or a marker parser that broke
    /// goes red naming itself.
    #[test]
    fn test_the_identifier_scan_reads_every_place_it_claims_to() {
        let scan = scan_doc_identifiers();
        for witness in RESOLUTION_WITNESSES {
            let found: usize = scan
                .resolved
                .iter()
                .filter(|(file, _)| file.starts_with(witness))
                .map(|(_, count)| count)
                .sum();
            assert!(
                found > 0,
                "the identifier scan resolved nothing under `{witness}`, so it is no longer looking \
                 there — a stale name in it would pass in silence"
            );
        }
        assert!(
            !scan.exempted.is_empty(),
            "the tree carries deliberate mentions of vanished names, so an empty exemption list \
             means the `{GONE_MARKER}` marker is no longer being read"
        );
    }
}
