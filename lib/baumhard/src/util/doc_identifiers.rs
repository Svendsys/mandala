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
//! round. The first run of this scan over the tree found a hundred
//! more, most of them in test modules, where `cargo doc` never looks.
//!
//! **Scope is the whole design.** A scan for "an identifier in a doc
//! comment that does not resolve" drowns at once: `insert`, `get`,
//! `color` and `font` are English words, stdlib and third-party names
//! cannot be resolved from here, and a hypothetical ("if a future
//! method on the cache did X") is legitimate prose. So a span is a
//! candidate only when it is written in backticks, parses as a path of
//! identifiers, and ends in a `snake_case` segment; and it is held to
//! a different authority by how it is qualified:
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
//! - **bare** (`` `pre_clip_positions` ``): the name must exist somewhere in
//!   the same sense, and is read only when it has an underscore. A
//!   single word is where English, tools, crates and stdlib methods
//!   live; a two-word name that no file writes and nothing declares is
//!   a name that has left. The authority is the whole workspace's code
//!   rather than the one file's, because a doc routinely names a field
//!   of a struct in another module that its own file never touches.
//! - A **family** written with a trailing `_` or `_*`
//!   (`` `select_sibling_*` ``) exists when anything it prefixes does,
//!   the reading the citation scan gives the same spelling.
//! - A path rooted in a crate the manifests depend on, in `std`, or in
//!   a type or module the workspace does not declare —
//!   `HashMap::entry`, `serde_json::from_str`, `str::chars`,
//!   `std::sync::mpsc` — is external, and the span is not a candidate
//!   at all. So is `Type::name` where the file imports `Type` from
//!   outside the workspace (`use std::cell::Cell;` makes a `set` on
//!   `Cell` in that file std's, whatever else is called `Cell` here), and
//!   where `Type` is a generic parameter (`T::void()`).
//! - A member a macro emits — a derive's `is_destructive`, a
//!   `macro_rules!` body's `pub known_keys:` field — belongs to a type
//!   no parse of the source can name, so `Type::name` resolves for any
//!   type when `name` is declared only inside a macro body. The
//!   methods std's derivable traits supply (`clone`, `default`, `fmt`,
//!   …) are allowed on every type for the same reason.
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
//! - A bare multi-word name of an external API that no file in the
//!   workspace calls — serde's `Serializer::serialize_tuple_struct`
//!   written without its type. It reads as a name that left, and the
//!   fix is to root it as shown; the handful of spans rooted when the
//!   gate landed is the price of the bare rule catching the many stale
//!   names it did.
//! - Dotted forms (`` `node.position` ``, `` `clock.wait` ``,
//!   `` `test.sh` ``): in this tree they are mostly JSON paths, IPC
//!   verbs and file names, and the scan does not read them.
//! - `CamelCase` types and `SCREAMING_SNAKE` constants, which are
//!   not `snake_case`; intra-doc link *targets*, which `cargo doc`
//!   already checks on live code; `//` comments that are not doc
//!   comments; Markdown files; doc comments on statements inside a
//!   function body, which the item walk does not descend into; a
//!   span rustfmt broke across two doc lines, whose halves are not
//!   read.
//! - A private field a macro emits (no `pub` to read the line by), and
//!   a declaration a macro builds from pieces, which no text harvest
//!   can see.
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

/// What the workspace declares, read once and consulted per span.
///
/// Built from a `syn` parse of every file — which is what knows a
/// struct's fields, an `impl` block's methods and a `trait`'s items —
/// plus a text harvest of the word after each declaring keyword, so
/// that a declaration a `macro_rules!` body emits still counts.
/// Over-reading a declaration weakens the check by one name;
/// under-reading one fails a run over prose that is correct, which is
/// the worse of the two.
pub(crate) struct WorkspaceIndex {
    /// Every `snake_case`-or-otherwise name declared as an item,
    /// field, variant or associated item anywhere, plus every module
    /// name and every source file stem and directory, since a
    /// `mod x;` names one.
    declared: BTreeSet<String>,
    /// Names declared as a `struct`, `enum`, `trait`, `type` or
    /// `union` — the qualifiers the sharp rule applies to.
    types: BTreeSet<String>,
    /// Names declared as a `mod`, plus file stems and directory
    /// names under the source roots.
    modules: BTreeSet<String>,
    /// Type or trait name → the fields, variants, methods and
    /// associated items its definition, `impl` blocks and `trait`
    /// body declare, across the whole workspace.
    members: BTreeMap<String, BTreeSet<String>>,
    /// Type name → the traits it has an `impl Trait for Type` for, so
    /// a default method the trait body declares resolves on the type.
    implements: BTreeMap<String, BTreeSet<String>>,
    /// Names the text harvest declares and `syn` does not: the
    /// members a `macro_rules!` body or a derive's `quote!` emits,
    /// which belong to a type no parse of the source can name. A
    /// `Type::name` whose `name` is here resolves on any type.
    macro_declared: BTreeSet<String>,
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
    /// The index of every `.rs` file under the source roots, with the
    /// workspace's package and dependency names read from its
    /// manifests.
    ///
    /// Cost: one read and one `syn` parse per file, one comment strip
    /// and one string blanking per file, and one read per manifest.
    pub(crate) fn of_workspace() -> Self {
        let sources: Vec<(String, String)> = workspace_rust_sources()
            .iter()
            .map(|file| {
                let text = std::fs::read_to_string(file)
                    .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
                (relative_to_repo(file), text)
            })
            .collect();
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
        Self::from_sources(&sources, &crates, &external)
    }

    /// The index of `sources`, each a `(repo-relative path, text)`
    /// pair, for a workspace whose packages are `crates` and whose
    /// dependencies are `external`. Panics naming the file on text
    /// `syn` cannot parse: a source that does not parse is not a
    /// source this scan can be trusted on, and dropping it would
    /// shrink the declared set in silence.
    pub(crate) fn from_sources(
        sources: &[(String, String)],
        crates: &BTreeSet<String>,
        external: &BTreeSet<String>,
    ) -> Self {
        let mut index = Self {
            declared: BTreeSet::new(),
            types: BTreeSet::new(),
            modules: BTreeSet::new(),
            members: BTreeMap::new(),
            implements: BTreeMap::new(),
            macro_declared: BTreeSet::new(),
            code: BTreeSet::new(),
            crates: crates.clone(),
            external_crates: external.clone(),
        };
        for name in ["std", "core", "alloc"] {
            index.external_crates.insert(name.to_string());
        }
        for (rel, text) in sources {
            index.harvest_path(rel);
            let parsed = syn::parse_file(text).unwrap_or_else(|e| panic!("{rel} must parse as Rust: {e}"));
            index.harvest_items(&parsed.items);
            let code = blank_string_literals(&strip_comments(text));
            index.harvest_text(&code);
            index
                .code
                .extend(identifier_tokens(&strip_comments(text)).map(str::to_string));
        }
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
                    self.declare_type(&name);
                    self.harvest_fields(&name, &item.fields);
                }
                Item::Enum(item) => {
                    let name = item.ident.to_string();
                    self.declare_type(&name);
                    for variant in &item.variants {
                        self.declare_member(&name, &variant.ident.to_string());
                        self.harvest_fields(&name, &variant.fields);
                    }
                }
                Item::Union(item) => {
                    let name = item.ident.to_string();
                    self.declare_type(&name);
                    for field in &item.fields.named {
                        if let Some(ident) = &field.ident {
                            self.declare_member(&name, &ident.to_string());
                        }
                    }
                }
                Item::Trait(item) => {
                    let name = item.ident.to_string();
                    self.declare_type(&name);
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
                    self.declare_type(&item.ident.to_string());
                }
                Item::TraitAlias(item) => {
                    self.declare_type(&item.ident.to_string());
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
    /// which `syn` sees as opaque tokens — still counts. Runs after
    /// [`Self::harvest_items`] on the same file's code, with strings
    /// blanked, so what it adds beyond the parse is exactly the
    /// macro-emitted set.
    fn harvest_text(&mut self, text: &str) {
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
        for line in text.lines() {
            // `pub name: Type` is a field declaration and nothing
            // else: a struct literal's fields and a `let` binding's
            // type ascription never carry `pub`. Read so a field a
            // macro emits (`KeybindConfig`'s bindings tables are
            // written by `keybind_surface!`) counts as declared.
            if let Some(field) = pub_field_name(line) {
                if !self.declared.contains(field) {
                    self.macro_declared.insert(field.to_string());
                }
                self.declared.insert(field.to_string());
            }
            let mut previous = "";
            for word in line.split(|c: char| !(c.is_ascii_alphanumeric() || c == '_')) {
                if word.is_empty() {
                    continue;
                }
                if DECLARING.contains(&previous) && is_identifier(word) {
                    if !self.declared.contains(word) {
                        self.macro_declared.insert(word.to_string());
                    }
                    self.declared.insert(word.to_string());
                    if TYPE_LIKE.contains(&previous) {
                        self.types.insert(word.to_string());
                    }
                    if previous == "mod" {
                        self.modules.insert(word.to_string());
                    }
                }
                previous = word;
            }
        }
    }

    /// Whether `owner` declares `name` itself, through a workspace
    /// trait it implements, or through a macro — whose output no
    /// parse attributes to a type, so a macro-emitted name is allowed
    /// on every type rather than refused on all of them.
    fn has_member(&self, owner: &str, name: &str) -> bool {
        let declares = |ty: &str| self.members.get(ty).is_some_and(|members| members.contains(name));
        declares(owner)
            || self
                .implements
                .get(owner)
                .is_some_and(|traits| traits.iter().any(|t| declares(t)))
            || self.macro_declared.contains(name)
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

    fn declare_type(&mut self, name: &str) {
        self.types.insert(name.to_string());
        self.declared.insert(name.to_string());
    }

    fn declare_member(&mut self, owner: &str, member: &str) {
        self.members
            .entry(owner.to_string())
            .or_default()
            .insert(member.to_string());
        self.declared.insert(member.to_string());
    }
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

/// A single uppercase letter, optionally numbered — `T`, `F`, `T2` —
/// is a generic parameter by convention, and no type this scan
/// judges.
fn is_generic_parameter(name: &str) -> bool {
    let mut chars = name.chars();
    chars.next().is_some_and(|c| c.is_ascii_uppercase()) && chars.all(|c| c.is_ascii_digit())
}

/// Where a multi-segment path's first segment places it.
#[derive(Debug, PartialEq, Eq)]
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

/// Method names the standard library's derivable and blanket traits
/// supply, so `Type::clone` or `Type::default` on a type that derives
/// them resolves without the workspace declaring either. A bounded
/// weakening: none of these is the shape of a deleted helper.
const STD_TRAIT_METHODS: &[&str] = &[
    "default",
    "clone",
    "clone_from",
    "fmt",
    "eq",
    "ne",
    "hash",
    "cmp",
    "partial_cmp",
    "lt",
    "le",
    "gt",
    "ge",
    "max",
    "min",
    "clamp",
    "to_owned",
    "to_string",
    "from",
    "into",
    "try_from",
    "try_into",
    "as_ref",
    "as_mut",
    "deref",
    "deref_mut",
    "drop",
    "serialize",
    "deserialize",
    "iter",
    "into_iter",
    "next",
    "from_str",
    "borrow",
    "borrow_mut",
    "index",
    "index_mut",
    "add",
    "sub",
    "mul",
    "div",
    "neg",
    "add_assign",
    "sub_assign",
    "mul_assign",
    "div_assign",
];

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
/// Cost: two reads and two `syn` parses per file — one pass builds
/// the index, the second reads the docs — plus one comment strip per
/// file for the "written in this file's code" rule.
pub(crate) fn scan_doc_identifiers() -> DocIdentifierScan {
    let index = WorkspaceIndex::of_workspace();
    let mut scan = DocIdentifierScan::new();
    for file in workspace_rust_sources() {
        let text = std::fs::read_to_string(&file)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
        scan_file(&relative_to_repo(&file), &text, &index, &mut scan);
    }
    scan
}

/// Read every doc comment in `text` — the file's own `//!` header,
/// every item's `///`, the members of `impl` and `trait` blocks, the
/// fields of structs and the variants of enums, into inline modules
/// — and record each candidate span in `scan`.
///
/// Doc comments on statements inside a function body are not
/// visited; the walk is over items.
pub(crate) fn scan_file(rel: &str, text: &str, index: &WorkspaceIndex, scan: &mut DocIdentifierScan) {
    let parsed = syn::parse_file(text).unwrap_or_else(|e| panic!("{rel} must parse as Rust: {e}"));
    let mut external_imports = BTreeSet::new();
    collect_external_imports(&parsed.items, index, &mut external_imports);
    let mut file = FileScan {
        rel,
        external_imports,
        index,
        scan,
    };
    file.attrs(&parsed.attrs, None);
    file.items(&parsed.items, None);
}

/// The names a file brings in from outside the workspace — `use
/// std::cell::Cell;`, `use cosmic_text::{Buffer, Metrics};` — so a
/// `std::cell::Cell::set` written without its path in that file is
/// read as the external type's even when some workspace type is also
/// called `Cell`. Descends into inline
/// modules, whose `use` items scope the same way for this purpose.
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
    /// workspace's to judge at all.
    root: Option<String>,
    /// The segment before the qualifier, when there is one: the enum
    /// in `GfxElement::GlyphArea::subtree_aabb`, whose variant the
    /// qualifier then is.
    enclosing: Option<String>,
}

enum Resolution {
    Resolved,
    /// Qualified by a type or module the workspace does not declare:
    /// not this scan's to judge.
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
                // A bare name resolves through any token the file's
                // code writes, which is too loose an authority to
                // contradict a reader who says the thing is gone. A
                // qualified name resolved against a declaration, and
                // that is sharp enough to hold the marker to.
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
        if candidate
            .root
            .as_deref()
            .is_some_and(|root| self.index.root_of(root) == Root::External)
        {
            return Resolution::External;
        }
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
                Some(owner) => self.resolve_member(owner, name),
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
                let on_owner = self.resolve_member(owner, name);
                match (on_owner, variant_of) {
                    (Resolution::Resolved, _) => Resolution::Resolved,
                    (on_owner, None) => on_owner,
                    (on_owner, Some(enclosing)) => match (self.resolve_member(enclosing, name), on_owner) {
                        (Resolution::Resolved, _) => Resolution::Resolved,
                        // The variant is no type of its own: the
                        // enum's answer is the only one there is.
                        (on_enclosing, Resolution::External) => on_enclosing,
                        // The variant shares a type's name and neither
                        // declares the member: the type's answer names
                        // what the reader most likely meant.
                        (_, on_owner) => on_owner,
                    },
                }
            }
            Qualifier::Module(module) => {
                if self.index.root_of(module) == Root::External {
                    Resolution::External
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

    fn resolve_member(&self, owner: &str, name: &str) -> Resolution {
        if is_generic_parameter(owner)
            || self.external_imports.contains(owner)
            || !self.index.types.contains(owner)
        {
            return Resolution::External;
        }
        if self.index.has_member(owner, name) || STD_TRAIT_METHODS.contains(&name) {
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
/// names `refill` — as are a macro's `!`, a `?`, and a leading `&`,
/// `&mut` or `*`.
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
        let owned: Vec<(String, String)> = sources
            .iter()
            .map(|(rel, text)| (rel.to_string(), text.to_string()))
            .collect();
        let external: BTreeSet<String> = ["ext_crate".to_string()].into_iter().collect();
        let index = WorkspaceIndex::from_sources(&owned, &BTreeSet::new(), &external);
        let mut scan = DocIdentifierScan::new();
        for (rel, text) in &owned {
            scan_file(rel, text, &index, &mut scan);
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

    /// A method a derive supplies, and a default method a workspace
    /// trait supplies, both resolve on the implementing type.
    #[test]
    fn test_derived_and_trait_default_methods_resolve_on_the_implementing_type() {
        let source = "\
/// `Cache::default` comes from the derive, `Cache::describe` from the trait body.
#[derive(Default)]
struct Cache;
trait Describe {
    fn describe(&self) -> &'static str {
        \"\"
    }
}
impl Describe for Cache {}
";
        let scan = scan_sources(&[("c.rs", source)]);
        assert!(scan.unresolved.is_empty(), "{:?}", scan.unresolved);
        assert_eq!(scan.resolved.get("c.rs"), Some(&2));
    }

    /// A `macro_rules!` body is opaque to `syn`; the text harvest is
    /// what lets a declaration written inside one count — as a bare
    /// name, and as a member of whichever type the doc attributes it
    /// to, since the parse cannot say which type the macro meant.
    #[test]
    fn test_a_declaration_a_macro_body_emits_still_counts() {
        let maker = "\
macro_rules! make {
    () => {
        fn made_by_macro() {}
        impl Config {
            pub fn known_keys() -> Vec<&'static str> {
                Vec::new()
            }
        }
    };
}
struct Config;
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
