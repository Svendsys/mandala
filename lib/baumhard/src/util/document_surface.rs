// SPDX-License-Identifier: MPL-2.0

//! The document's surface, held against the tree rather than trusted
//! (#235's acceptance):
//!
//! - **It replaces the model; it does not lean on it.** No file under
//!   `lib/baumhard/src/document/` names the `mindmap` module or the
//!   glyph module in code, nor the structure the document removes: a
//!   `parent_id`, an `edge_type`, a `parent_child` relation.
//! - **A `String` is parsed once, at load.** The element data holds a
//!   `String` only where #235 lists one — a span's text, the map's and
//!   a palette's names, the theme's variable and variant names, a URL,
//!   a fit-mode filler and the two `content` fields — so no projection
//!   has a string left to parse.
//! - **A field D13 deletes is absent, not present-but-unused.**
//!
//! The first check is lexical, through
//! [`references_to`](crate::util::engine_isolation::references_to);
//! the other two parse `element.rs` and `values.rs` with `syn` and read
//! every field of every struct and enum variant there.
//!
//! Test-only and native-only, for the same reasons
//! [`crate::util::source_scan`] is.

use crate::util::doc_fixtures::repo_path;
use crate::util::engine_isolation::{references_to, rust_files};
use crate::util::rust_source::{blank_string_literals, strip_comments};
use crate::util::source_scan::{is_test_gated, item_attrs};
use std::collections::{BTreeMap, BTreeSet};
use syn::visit::{self, Visit};
use syn::{Fields, Item, PathSegment, Type};
use syn::{FnArg, ImplItem, ReturnType, Signature, TypeReference, Visibility};

/// The identifier no file outside the document and the benches may write
/// in code: the document's test builder, a back door around the loader
/// and the writer.
const BUILDER: &[&str] = &["DocumentBuilder"];

/// The builder's module path, which no such file may write either. The
/// qualified path, not the bare module name, so another `test_support`
/// module elsewhere stays free to exist.
const BUILDER_MODULE: &str = "document::tests::test_support";

/// Every `(line, needle)` in `source` that names the builder in code: its
/// type by name ([`BUILDER`]), or its module by path
/// ([`BUILDER_MODULE`]). Comments are stripped and string literals
/// blanked first.
fn builder_references(source: &str) -> Vec<(usize, &'static str)> {
    let mut found = references_to(source, BUILDER);
    let code = blank_string_literals(&strip_comments(source));
    for (index, line) in code.lines().enumerate() {
        if line.contains(BUILDER_MODULE) {
            found.push((index + 1, BUILDER_MODULE));
        }
    }
    found
}

/// Identifiers no document file may write in code.
const FORBIDDEN: &[&str] = &["mindmap", "gfx_structs", "parent_id", "edge_type", "parent_child"];

/// The `String`-holding fields #235 allows on document elements, as
/// `Type.field` (a tuple field by its index, an enum's by its variant).
const ALLOWED_STRINGS: &[&str] = &[
    "FitMode::Once.filler",
    "FlowStyle.content",
    "LinkTarget::Url.0",
    "MapData.name",
    "PaletteData.name",
    "PortalEndStyle.content",
    "SpanData.text",
    "ThemeData.variables",
    "ThemeVariant.name",
];

/// Field names D13 deletes, and the structure fields the document's
/// nesting replaces (§5.1).
const DELETED_FIELDS: &[&str] = &[
    "notes",
    "underline",
    "layout",
    "show_shadow",
    "padding",
    "width",
    "line_style",
    "corner_radius_percent",
    "frame_thickness",
    "parent_id",
    "edge_type",
];

/// Whether `ty` mentions `String` anywhere: `String`, `Vec<String>`,
/// `Option<String>`.
fn mentions_string(ty: &Type) -> bool {
    struct Finder(bool);
    impl<'ast> Visit<'ast> for Finder {
        fn visit_path_segment(&mut self, segment: &'ast PathSegment) {
            self.0 |= segment.ident == "String";
            visit::visit_path_segment(self, segment);
        }
    }
    let mut finder = Finder(false);
    finder.visit_type(ty);
    finder.0
}

/// Every field of every struct and enum variant in `source`, as
/// `(owner, field name, holds a String)`; `owner` is `Type` or
/// `Type::Variant`, and a tuple field is named by its index.
fn fields(source: &str) -> Vec<(String, String, bool)> {
    let file = syn::parse_file(source).expect("the document's sources parse");
    let mut out = Vec::new();
    let mut push = |owner: String, fields: &Fields| {
        for (index, field) in fields.iter().enumerate() {
            let name = field
                .ident
                .as_ref()
                .map_or(index.to_string(), ToString::to_string);
            out.push((owner.clone(), name, mentions_string(&field.ty)));
        }
    };
    for item in &file.items {
        match item {
            Item::Struct(item) => push(item.ident.to_string(), &item.fields),
            Item::Enum(item) => {
                for variant in &item.variants {
                    push(format!("{}::{}", item.ident, variant.ident), &variant.fields);
                }
            }
            _ => {}
        }
    }
    out
}

/// [`fields`] of the document's element data and values.
fn document_fields() -> Vec<(String, String, bool)> {
    let mut out = Vec::new();
    for file in ["element.rs", "values.rs"] {
        let path = repo_path("lib/baumhard/src/document").join(file);
        let source = std::fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
        out.extend(fields(&source));
    }
    out
}

/// The file that may hand out `&mut` into a document, and why: the test
/// builder, which exists to make and edit documents for tests and is
/// held to the tests and the benches by
/// `test_only_the_document_names_its_test_builder`.
const MAY_EDIT: &[&str] = &["tests/test_support.rs"];

/// The macros `document/` may invoke where an item or an `impl` member
/// goes, each with the file under `document/` that defines it (`None` for
/// one the standard library defines), and why it cannot hand out
/// `&mut`: a macro's expansion is invisible to the gate, so every other
/// one is refused, and so is a `macro_rules!` under `document/` that
/// reuses one of these names anywhere but its own file.
///
/// - `macro_rules` *defines* a macro, which writes nothing until it is
///   invoked, and is then held to this list.
/// - `thread_local` declares a private static (`format/rows.rs`).
/// - `spelled` derives a spelling enum and its conversions.
/// - `forward_methods` and `forward_scalars` forward a deserializer's
///   methods to the one it wraps, on a type private to `format`.
const ALLOWED_MACROS: &[(&str, Option<&str>)] = &[
    ("macro_rules", None),
    ("thread_local", None),
    ("spelled", Some("format/macros.rs")),
    ("forward_methods", Some("format/strict.rs")),
    ("forward_scalars", Some("format/strict.rs")),
];

/// The visible items the gate allows one shape in their return type,
/// each with the shape and why it grants nothing.
///
/// An `impl Trait` allowed here is an `impl Iterator` whose `Item` the
/// gate still judges by the whole rule, so it yields nothing that grants
/// — ids, shared references, copies: a read API that walks what it reads
/// without allocating. A caller-chosen type parameter allowed here is
/// serde's (`T: Deserialize`, `D: Deserializer`, `S: Serializer`), or the
/// error type a caller's own callback returns: the value is built from
/// text or handed back from the caller's code, never a borrow into a
/// document.
const EXCEPTIONS: &[(&str, &str, &str)] = &[
    ("Document::children", IMPL_TRAIT, "an iterator of child ids"),
    (
        "Document::palettes",
        IMPL_TRAIT,
        "an iterator of shared palette references",
    ),
    (
        "Document::history",
        IMPL_TRAIT,
        "an iterator of history labels, sources and revisions",
    ),
    (
        "palettes",
        IMPL_TRAIT,
        "an iterator of shared palette references, over a tree",
    ),
    (
        "inputs_changed",
        IMPL_TRAIT,
        "an iterator of the resolution inputs a change touches, by value",
    ),
    (
        "idents_named_by",
        IMPL_TRAIT,
        "an iterator of node ids a definition names, by value",
    ),
    (
        "parse_extent",
        PARAM,
        "serde: the value a deserializer reads from text",
    ),
    (
        "present",
        PARAM,
        "serde: the value a deserializer reads from text, and its error",
    ),
    ("serialize_spelled", PARAM, "serde: the serializer's own output"),
    (
        "deserialize_spelled",
        PARAM,
        "serde: the value a deserializer reads from text, and its error",
    ),
    ("Rows::each", PARAM, "the error the caller's own callback returns"),
];

/// The traits whose methods the gate allows a caller-chosen type
/// parameter in their return type, and why: serde's, whose methods give
/// back the serializer's own output or the value a deserializer reads
/// from text — built from a document's values, never a borrow into it.
const EXCEPTED_TRAITS: &[(&str, &str)] = &[
    ("Serialize", "serde: the serializer's own output"),
    ("Deserialize", "serde: the value read from text"),
    ("Deserializer", "serde: the visitor's own value"),
    ("Visitor", "serde: the visitor's own value"),
];

/// The shapes [`EXCEPTIONS`] can allow, in the words [`judge`] names them.
const IMPL_TRAIT: &str = "an `impl Trait`";
/// See [`IMPL_TRAIT`].
const PARAM: &str = "a caller-chosen type parameter";

/// Standard types a return may name from a function that takes a
/// `&mut`, because they hold no borrow of their own: whatever they hold
/// is named in their arguments and judged there.
const OWNED_STD: &[&str] = &[
    "Result", "Option", "Vec", "String", "Box", "bool", "char", "u8", "u16", "u32", "u64", "u128", "usize",
    "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64",
];

/// What the gate knows of the crate while it judges `document/`.
#[derive(Default)]
struct Crate {
    /// Every type a file of the crate declares, by name, and whether one
    /// so named carries a lifetime — a type that can hold a borrow.
    borrows: BTreeMap<String, bool>,
    /// Every type, alias and trait declared under `document/`, by name,
    /// and whether one so named is visible beyond `document`.
    local: BTreeMap<String, bool>,
    /// Every alias declared under `document/`, by name.
    aliases: BTreeMap<String, Type>,
}

impl Crate {
    /// Whether the type `name` is reachable beyond `document`: declared
    /// there and visible, or not declared there at all.
    fn reachable(&self, name: &str) -> bool {
        self.local.get(name).copied().unwrap_or(true)
    }
}

/// What a signature is judged in: its caller-chosen type parameters, the
/// associated types its `impl` defines, and whether it takes a `&mut`.
struct Scope<'c> {
    krate: &'c Crate,
    params: BTreeSet<String>,
    assoc: BTreeMap<String, Type>,
    takes_mut: bool,
}

/// Why a return type grants access, if it does: every offending shape
/// found in `ty`, each once and in the order met, resolving aliases and
/// associated types. Empty when it grants nothing.
fn judge(ty: &Type, scope: &Scope<'_>, depth: usize) -> Vec<&'static str> {
    struct Judge<'s, 'c> {
        scope: &'s Scope<'c>,
        depth: usize,
        found: Vec<&'static str>,
    }
    impl Judge<'_, '_> {
        fn found(&mut self, why: &'static str) {
            if !self.found.contains(&why) {
                self.found.push(why);
            }
        }
    }
    impl<'ast> Visit<'ast> for Judge<'_, '_> {
        fn visit_type_reference(&mut self, reference: &'ast TypeReference) {
            if reference.mutability.is_some() {
                self.found("a `&mut`");
            }
            visit::visit_type_reference(self, reference);
        }
        fn visit_type_impl_trait(&mut self, imp: &'ast syn::TypeImplTrait) {
            self.found("an `impl Trait`");
            visit::visit_type_impl_trait(self, imp);
        }
        fn visit_type_trait_object(&mut self, object: &'ast syn::TypeTraitObject) {
            self.found("a `dyn Trait`");
            visit::visit_type_trait_object(self, object);
        }
        fn visit_lifetime(&mut self, lifetime: &'ast syn::Lifetime) {
            if self.scope.takes_mut && lifetime.ident != "static" {
                self.found("a borrow of a `&mut` argument");
            }
        }
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            let segments: Vec<String> = path.path.segments.iter().map(|s| s.ident.to_string()).collect();
            let first = segments.first().cloned().unwrap_or_default();
            if self.scope.params.contains(&first) {
                self.found("a caller-chosen type parameter");
            }
            // `Self::X` or `<Self as Trait>::X`: the type the impl gives it.
            let assoc = match (&path.qself, segments.as_slice()) {
                (Some(_), [.., last]) => Some(last.clone()),
                (None, [this, name, ..]) if this == "Self" => Some(name.clone()),
                _ => None,
            };
            let resolved = assoc.and_then(|name| self.scope.assoc.get(&name)).or_else(|| {
                (segments.len() == 1)
                    .then(|| self.scope.krate.aliases.get(&first))
                    .flatten()
            });
            if let Some(resolved) = resolved.filter(|_| self.depth < 8) {
                for why in judge(resolved, self.scope, self.depth + 1) {
                    self.found(why);
                }
            }
            if self.scope.takes_mut && path.qself.is_none() {
                let last = segments.last().cloned().unwrap_or_default();
                let borrows = match self.scope.krate.borrows.get(&last) {
                    Some(&borrows) => borrows,
                    None => {
                        !OWNED_STD.contains(&last.as_str())
                            && last != "Self"
                            && !self.scope.params.contains(&last)
                    }
                };
                if borrows && !self.scope.assoc.contains_key(&last) {
                    self.found("a type that can hold a borrow of a `&mut` argument");
                }
            }
            visit::visit_type_path(self, path);
        }
    }
    let mut judge = Judge {
        scope,
        depth,
        found: Vec::new(),
    };
    judge.visit_type(ty);
    judge.found
}

/// Whether `vis`, on an item `depth` modules below the crate root
/// (`document/mod.rs` is 1, `document/ids.rs` 2), reaches beyond
/// `document`: `pub`; `pub(crate)`; `pub(in …)` naming a path outside
/// `document`; `pub(super)` (and `pub(in super::…)`) climbing to the
/// crate root, as it does in `document/mod.rs`.
fn visible(vis: &Visibility, depth: usize) -> bool {
    match vis {
        Visibility::Public(_) => true,
        Visibility::Inherited => false,
        Visibility::Restricted(restricted) => {
            let segments: Vec<String> = restricted
                .path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            match segments.first().map(String::as_str) {
                Some("crate") => segments.get(1).map(String::as_str) != Some("document"),
                Some("self") => false,
                Some("super") => segments.iter().take_while(|s| *s == "super").count() >= depth,
                _ => true,
            }
        }
    }
}

/// What [`mutable_access`] found: every function visible beyond
/// `document` that it judged, and every way it found to hand out access
/// — `Owner::name: why` for a method, `name: why` for a function,
/// `Type.field` for a field, `name!` for a macro.
#[derive(Debug, Default, PartialEq)]
struct Access {
    read: Vec<String>,
    offenders: Vec<String>,
}

/// The name of a type's last path segment, or an empty string.
fn type_name(ty: &Type) -> String {
    match ty {
        Type::Path(path) => path
            .path
            .segments
            .last()
            .map_or(String::new(), |segment| segment.ident.to_string()),
        Type::Reference(reference) => type_name(&reference.elem),
        _ => String::new(),
    }
}

/// The name of an invoked macro, its path's last segment.
fn macro_name(mac: &syn::Macro) -> String {
    mac.path
        .segments
        .last()
        .map_or(String::new(), |segment| segment.ident.to_string())
}

/// The type parameters `generics` declares.
fn type_params(generics: &syn::Generics) -> BTreeSet<String> {
    generics
        .type_params()
        .map(|param| param.ident.to_string())
        .collect()
}

/// The first pass over one file of the crate: what it declares.
fn declare(items: &[Item], depth: usize, in_document: bool, krate: &mut Crate) {
    for item in items {
        if is_test_gated(item_attrs(item)) {
            continue;
        }
        let (name, generics, vis) = match item {
            Item::Struct(item) => (item.ident.to_string(), &item.generics, &item.vis),
            Item::Enum(item) => (item.ident.to_string(), &item.generics, &item.vis),
            Item::Union(item) => (item.ident.to_string(), &item.generics, &item.vis),
            Item::Type(item) => {
                if in_document {
                    krate.aliases.insert(item.ident.to_string(), (*item.ty).clone());
                }
                (item.ident.to_string(), &item.generics, &item.vis)
            }
            Item::Trait(item) => (item.ident.to_string(), &item.generics, &item.vis),
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    declare(items, depth + 1, in_document, krate);
                }
                continue;
            }
            _ => continue,
        };
        let lifetimes = generics.lifetimes().next().is_some();
        *krate.borrows.entry(name.clone()).or_insert(false) |= lifetimes;
        if in_document {
            *krate.local.entry(name).or_insert(false) |= visible(vis, depth);
        }
    }
}

/// Judge one signature, as `name`, and note it read.
fn judge_fn(
    name: String,
    trait_name: Option<&str>,
    sig: &Signature,
    mut scope: Scope<'_>,
    access: &mut Access,
) {
    scope.params.extend(type_params(&sig.generics));
    scope.takes_mut = sig.inputs.iter().any(|input| match input {
        FnArg::Receiver(receiver) => matches!(receiver.kind, syn::ReceiverKind::Reference(_, _, Some(_))),
        FnArg::Typed(typed) => {
            let mut finder = MutFinder(false);
            finder.visit_type(&typed.ty);
            finder.0
        }
    });
    let mut lends = Lends(false);
    for input in &sig.inputs {
        if let FnArg::Typed(typed) = input {
            lends.visit_type(&typed.ty);
        }
    }
    lends.visit_generics(&sig.generics);
    if lends.0 {
        access
            .offenders
            .push(format!("{name}: lends a `&mut` to the caller's closure"));
    }
    if let ReturnType::Type(_, ty) = &sig.output {
        let excepted = |why: &str| {
            EXCEPTIONS
                .iter()
                .any(|(item, shape, _)| *item == name && *shape == why)
                || (why == PARAM && trait_name.is_some_and(|t| EXCEPTED_TRAITS.iter().any(|(e, _)| *e == t)))
        };
        let refused = judge(ty, &scope, 0)
            .into_iter()
            .find(|why| !excepted(why) || (*why == IMPL_TRAIT && !iterator_items_pass(ty, &scope)));
        if let Some(why) = refused {
            access.offenders.push(format!("{name}: {why}"));
        }
    }
    access.read.push(name);
}

/// Whether every `Item` an `impl Iterator<Item = …>` return names passes
/// the rule itself, so an excepted iterator yields nothing that grants.
fn iterator_items_pass(ty: &Type, scope: &Scope<'_>) -> bool {
    struct Items<'s, 'c>(&'s Scope<'c>, bool);
    impl<'ast> Visit<'ast> for Items<'_, '_> {
        fn visit_assoc_type(&mut self, assoc: &'ast syn::AssocType) {
            if assoc.ident == "Item" && !judge(&assoc.ty, self.0, 0).is_empty() {
                self.1 = false;
            }
            visit::visit_assoc_type(self, assoc);
        }
    }
    let mut items = Items(scope, true);
    items.visit_type(ty);
    items.1
}

/// Finds a closure bound — `Fn`, `FnMut`, `FnOnce` — that takes a
/// `&mut`: a parameter or a bound through which a function would hand
/// one to its caller's code.
struct Lends(bool);

impl<'ast> Visit<'ast> for Lends {
    fn visit_parenthesized_generic_arguments(&mut self, arguments: &'ast syn::ParenthesizedGenericArguments) {
        let mut finder = MutFinder(false);
        for input in &arguments.inputs {
            finder.visit_type(&input.ty);
        }
        self.0 |= finder.0;
        visit::visit_parenthesized_generic_arguments(self, arguments);
    }
}

/// Finds any `&mut` in a type.
struct MutFinder(bool);

impl<'ast> Visit<'ast> for MutFinder {
    fn visit_type_reference(&mut self, reference: &'ast TypeReference) {
        self.0 |= reference.mutability.is_some();
        visit::visit_type_reference(self, reference);
    }
}

/// The second pass over one file of `document/`.
fn walk_items(items: &[Item], depth: usize, file: &str, krate: &Crate, access: &mut Access) {
    let scope = |params: BTreeSet<String>, assoc: BTreeMap<String, Type>| Scope {
        krate,
        params,
        assoc,
        takes_mut: false,
    };
    for item in items {
        if is_test_gated(item_attrs(item)) {
            continue;
        }
        match item {
            Item::Fn(item) if visible(&item.vis, depth) => {
                judge_fn(
                    item.sig.ident.to_string(),
                    None,
                    &item.sig,
                    scope(BTreeSet::new(), BTreeMap::new()),
                    access,
                );
            }
            Item::Struct(item) if visible(&item.vis, depth) => {
                for (i, field) in item.fields.iter().enumerate() {
                    let mut finder = MutFinder(false);
                    finder.visit_type(&field.ty);
                    if finder.0 && visible(&field.vis, depth) {
                        let name = field.ident.as_ref().map_or(i.to_string(), ToString::to_string);
                        access
                            .offenders
                            .push(format!("{}.{name}: a `&mut` field", item.ident));
                    }
                }
            }
            Item::Enum(item) if visible(&item.vis, depth) => {
                for variant in &item.variants {
                    for (i, field) in variant.fields.iter().enumerate() {
                        let mut finder = MutFinder(false);
                        finder.visit_type(&field.ty);
                        if finder.0 {
                            let name = field.ident.as_ref().map_or(i.to_string(), ToString::to_string);
                            access.offenders.push(format!(
                                "{}::{}.{name}: a `&mut` field",
                                item.ident, variant.ident
                            ));
                        }
                    }
                }
            }
            Item::Trait(item) if visible(&item.vis, depth) => {
                let defaults: BTreeMap<String, Type> = item
                    .items
                    .iter()
                    .filter_map(|member| match member {
                        syn::TraitItem::Type(ty) => ty
                            .default
                            .as_ref()
                            .map(|(_, default)| (ty.ident.to_string(), default.clone())),
                        _ => None,
                    })
                    .collect();
                for member in &item.items {
                    if let syn::TraitItem::Fn(method) = member {
                        if is_test_gated(&method.attrs) {
                            continue;
                        }
                        judge_fn(
                            format!("{}::{}", item.ident, method.sig.ident),
                            Some(&item.ident.to_string()),
                            &method.sig,
                            scope(type_params(&item.generics), defaults.clone()),
                            access,
                        );
                    }
                }
            }
            Item::Impl(item) => {
                let owner = type_name(&item.self_ty);
                let trait_name = item
                    .trait_
                    .as_ref()
                    .and_then(|(path, _)| path.segments.last())
                    .map(|segment| segment.ident.to_string());
                // An impl is reachable when its type is, and — for a
                // trait's — its trait is: whatever its self type, a trait
                // implemented for a type outside `document` is reachable.
                let reachable =
                    krate.reachable(&owner) && trait_name.as_deref().is_none_or(|name| krate.reachable(name));
                if !reachable {
                    continue;
                }
                let assoc: BTreeMap<String, Type> = item
                    .items
                    .iter()
                    .filter_map(|member| match member {
                        ImplItem::Type(ty) => Some((ty.ident.to_string(), ty.ty.clone())),
                        _ => None,
                    })
                    .collect();
                for member in &item.items {
                    match member {
                        ImplItem::Fn(method) => {
                            let shown = item.trait_.is_some() || visible(&method.vis, depth);
                            if !shown || is_test_gated(&method.attrs) {
                                continue;
                            }
                            judge_fn(
                                format!("{owner}::{}", method.sig.ident),
                                trait_name.as_deref(),
                                &method.sig,
                                scope(type_params(&item.generics), assoc.clone()),
                                access,
                            );
                        }
                        ImplItem::Macro(mac) if !is_test_gated(&mac.attrs) => {
                            invoked(&macro_name(&mac.mac), &format!("{owner}: "), access);
                        }
                        _ => {}
                    }
                }
            }
            Item::Macro(mac) => {
                let name = macro_name(&mac.mac);
                if name == "macro_rules" {
                    let defined = mac.ident.as_ref().map(ToString::to_string).unwrap_or_default();
                    if let Some((_, home)) = ALLOWED_MACROS.iter().find(|(allowed, _)| *allowed == defined) {
                        if *home != Some(file) {
                            access
                                .offenders
                                .push(format!("macro_rules! {defined}: reuses an allowed name"));
                        }
                    }
                } else {
                    invoked(&name, "", access);
                }
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    walk_items(items, depth + 1, file, krate, access);
                }
            }
            _ => {}
        }
    }
}

/// Refuse an invocation of `name` that [`ALLOWED_MACROS`] does not name.
fn invoked(name: &str, owner: &str, access: &mut Access) {
    if !ALLOWED_MACROS.iter().any(|(allowed, _)| *allowed == name) {
        access.offenders.push(format!("{owner}{name}!"));
    }
}

/// Everything in `document` — each source its path under `document/`,
/// its text and how many modules below the crate root it sits — that
/// hands out access, judged by shape (ruling R37 on #238). A function
/// visible beyond `document` ([`visible`]) — free, inherent, or of any
/// trait defined there or implemented there whatever its self type,
/// default bodies included — offends when its return type holds, after
/// its aliases and associated types are resolved:
/// - a `&mut`, whatever it points at;
/// - an `impl Trait` or a `dyn Trait` (but [`EXCEPTIONS`]);
/// - a type parameter its caller chooses;
/// - when the function takes a `&mut`, a lifetime, or a type that can
///   hold one — a type of the crate that declares a lifetime, or any
///   type from outside it but [`OWNED_STD`].
///
/// So does one whose parameters or bounds hand a `&mut` to a closure
/// its caller passes. A visible field holding a `&mut` offends, and so
/// does a macro in item
/// position that [`ALLOWED_MACROS`] does not name, or a `macro_rules!`
/// that reuses an allowed name outside its file. `others` are the rest of
/// the crate's sources, read for the types they declare. Test-gated items
/// are not read. The compiler's privacy on the tree and the element
/// fields is what keeps a document closed; this makes sure nothing in
/// `document/` exports a way around it.
fn mutable_access(sources: &[(&str, &str, usize)], others: &[&str]) -> Access {
    let parse = |source: &str| syn::parse_file(source).expect("the crate's sources parse");
    let documents: Vec<(&str, syn::File, usize)> = sources
        .iter()
        .map(|&(file, source, depth)| (file, parse(source), depth))
        .collect();
    let mut krate = Crate::default();
    for other in others {
        declare(&parse(other).items, 1, false, &mut krate);
    }
    for (_, file, depth) in &documents {
        declare(&file.items, *depth, true, &mut krate);
    }
    let mut access = Access::default();
    for (path, file, depth) in &documents {
        walk_items(&file.items, *depth, path, &krate, &mut access);
    }
    access
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The sources the gate reads: every file under `document/` but the
    /// test builder and the test-gated ones, each as `(path under
    /// document/, text, modules below the crate root)`; and the text of
    /// every other file of the crate.
    fn gate_sources() -> (Vec<(String, String, usize)>, Vec<String>) {
        let src = repo_path("lib/baumhard/src");
        let root = src.join("document");
        let all = rust_files(&src);
        let gated = crate::util::source_scan::test_gated_module_files(&all);
        let mut documents = Vec::new();
        let mut others = Vec::new();
        for file in &all {
            let source = std::fs::read_to_string(file)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
            let Ok(relative) = file.strip_prefix(&root) else {
                others.push(source);
                continue;
            };
            let relative = relative.to_string_lossy().to_string();
            if gated.contains(file) || MAY_EDIT.contains(&relative.as_str()) {
                continue;
            }
            // `document/mod.rs` is one module below the crate root, a
            // file beside it two, a `mod.rs` below it as many as its
            // directories.
            let parts = relative.split('/').count();
            let depth = if relative.ends_with("mod.rs") {
                parts
            } else {
                parts + 1
            };
            documents.push((relative, source, depth));
        }
        (documents, others)
    }

    /// **No visible item under `document/` hands out a way to change a
    /// document** (design §8.4, #238 §7, ruling R37). No function the
    /// rest of the crate or the app can call — free, inherent, or of a
    /// trait defined or implemented there, whatever its self type —
    /// returns a `&mut`, an `impl Trait` or `dyn Trait` the exceptions do
    /// not name, a type its caller chooses, or a borrow of a `&mut` it
    /// took, or lends a `&mut` to its caller's closure; no visible field
    /// holds a `&mut`; and no macro whose expansion the gate cannot read
    /// is invoked where an item goes. Preconditions: the scan reads the
    /// writer's API and the trait impls of the elements, and every
    /// exception names an item it read.
    #[test]
    fn test_no_document_item_hands_out_mutable_access() {
        let (documents, others) = gate_sources();
        let sources: Vec<(&str, &str, usize)> = documents
            .iter()
            .map(|(path, text, depth)| (path.as_str(), text.as_str(), *depth))
            .collect();
        let others: Vec<&str> = others.iter().map(String::as_str).collect();
        assert!(
            others.len() > 50,
            "the scan must read the rest of the crate's types"
        );
        let access = mutable_access(&sources, &others);
        for witness in [
            "Document::apply",
            "Document::stage",
            "Document::tree",
            "DocElement::split",
        ] {
            assert!(
                access.read.iter().any(|name| name == witness),
                "the scan must read {witness}"
            );
        }
        for (item, _, _) in EXCEPTIONS {
            assert!(
                access.read.iter().any(|name| name == item),
                "the exception {item} must name an item the scan reads"
            );
        }
        assert!(
            access.offenders.is_empty(),
            "these hand out a way to change a document; change it through `Document::apply`:\n{}",
            access.offenders.join("\n")
        );
    }

    /// The negative control of the gate above: each shape a function, a
    /// trait, a field or a macro can hand out access by is reported —
    /// the review of #299's shapes among them — and each that cannot is
    /// not.
    #[test]
    fn test_the_mutable_access_scan_reports_a_planted_back_door() {
        let planted = r#"
impl Document {
    pub fn tree_mut(&mut self) -> &mut Tree<DocElement> { todo!() }
    pub fn me(&mut self) -> Option<&mut Self> { todo!() }
    pub(crate) fn each(&mut self, f: impl FnMut(&mut NodeData)) {}
    pub fn with_tree<F: FnOnce(&mut Tree<DocElement>)>(&mut self, f: F) {}
    pub fn with_where<F>(&mut self, f: F) where F: FnOnce(&mut DocElement) {}
    pub fn tree_alias(&mut self) -> TreeMut<'_> { todo!() }
    pub fn counter(&mut self) -> &mut u32 { todo!() }
    pub fn deref_tree(&mut self) -> impl DerefMut<Target = Tree<DocElement>> + '_ { todo!() }
    pub fn any(&mut self) -> &mut dyn Any { todo!() }
    pub fn get_mut<T: 'static>(&mut self) -> Option<&mut T> { todo!() }
    pub fn out_guard(&mut self) -> OutGuard<'_> { todo!() }
    pub fn out_elided(&mut self) -> OutGuard { todo!() }
    pub fn foreign(&mut self) -> std::slice::IterMut<u8> { todo!() }
    pub fn children(&self) -> impl Iterator<Item = NodeId> + '_ { todo!() }
    pub fn palettes(&self) -> impl Iterator<Item = &mut PaletteData> + '_ { todo!() }
    pub fn apply(&mut self, n: u32) -> Result<Revision, ApplyError> { todo!() }
    pub fn read(&self) -> &Tree<DocElement> { todo!() }
    fn private(&mut self) -> &mut DocElement { todo!() }
    pub(super) fn near(&mut self) -> &mut DocElement { todo!() }
    pub(in crate::document) fn inside(&mut self) -> &mut DocElement { todo!() }
    pub(in crate::engine) fn engine(&mut self) -> &mut DocElement { todo!() }
    #[cfg(test)]
    pub fn tested(&mut self) -> &mut DocElement { todo!() }
    tree_by_macro!();
    spelled!();
}
impl AsMut<Tree<DocElement>> for Document {
    fn as_mut(&mut self) -> &mut Tree<DocElement> { todo!() }
}
impl std::ops::DerefMut for Document {
    fn deref_mut(&mut self) -> &mut Self::Target { todo!() }
}
impl std::fmt::Display for Document {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { todo!() }
}
pub trait Reach { fn reach(doc: &mut Document) -> &mut Tree<DocElement>; }
impl Reach for u8 { fn reach(doc: &mut Document) -> &mut Tree<DocElement> { todo!() } }
pub trait Reach2 { fn reach2(doc: &mut Document) -> &mut Tree<DocElement> { todo!() } }
impl Reach2 for u16 {}
pub trait Lend { type Out<'a> where Self: 'a; fn lend(&mut self) -> Self::Out<'_>; }
impl Lend for Document {
    type Out<'a> = &'a mut Tree<DocElement>;
    fn lend(&mut self) -> Self::Out<'_> { todo!() }
}
struct Hidden;
impl Hidden { pub fn hidden(&mut self) -> &mut DocElement { todo!() } }
impl Mutation<DocElement> for Hidden { fn apply_to(&self, e: &mut DocElement) {} }
pub type TreeMut<'a> = &'a mut Tree<DocElement>;
pub struct Guard<'a> { pub tree: &'a mut Tree<DocElement>, count: u32 }
pub fn element(doc: &mut Document) {}
pub fn bytes(out: &mut Vec<u8>, doc: &Document) -> Result<(), SaveError> { todo!() }
mod inner {
    pub fn deep() -> &'static mut SpanData { todo!() }
}
macro_rules! tree_by_macro { () => {}; }
macro_rules! spelled { () => {}; }
hidden_items!();
"#;
        let out_of_document = "pub struct OutGuard<'a>(pub &'a mut Tree<DocElement>);\n\
                               pub struct Revision;\npub enum ApplyError {}\npub struct SaveError;\n";
        let access = mutable_access(&[("ids.rs", planted, 2)], &[out_of_document]);
        let in_mod_rs = "pub(super) fn tree_of(doc: &mut Document) -> &mut Tree<DocElement> { todo!() }\n\
                         pub(super) fn fine(doc: &Document) {}\n";
        let at_the_top = mutable_access(&[("mod.rs", in_mod_rs, 1)], &[]);
        assert_eq!(
            at_the_top.offenders,
            vec!["tree_of: a `&mut`"],
            "`pub(super)` in `document/mod.rs` reaches the crate"
        );
        let found: BTreeSet<&str> = access.offenders.iter().map(String::as_str).collect();
        let expected: BTreeSet<&str> = [
            "Document::tree_mut: a `&mut`",
            "Document::me: a `&mut`",
            "Document::each: lends a `&mut` to the caller's closure",
            "Document::with_tree: lends a `&mut` to the caller's closure",
            "Document::with_where: lends a `&mut` to the caller's closure",
            "Document::tree_alias: a `&mut`",
            "Document::counter: a `&mut`",
            "Document::deref_tree: an `impl Trait`",
            "Document::any: a `&mut`",
            "Document::get_mut: a `&mut`",
            "Document::out_guard: a type that can hold a borrow of a `&mut` argument",
            "Document::out_elided: a type that can hold a borrow of a `&mut` argument",
            "Document::foreign: a type that can hold a borrow of a `&mut` argument",
            "Document::palettes: an `impl Trait`",
            "Document::engine: a `&mut`",
            "Document: tree_by_macro!",
            "Document::as_mut: a `&mut`",
            "Document::deref_mut: a `&mut`",
            "Reach::reach: a `&mut`",
            "u8::reach: a `&mut`",
            "Reach2::reach2: a `&mut`",
            "Lend::lend: a type that can hold a borrow of a `&mut` argument",
            "Document::lend: a `&mut`",
            "Guard.tree: a `&mut` field",
            "deep: a `&mut`",
            "macro_rules! spelled: reuses an allowed name",
            "hidden_items!",
        ]
        .into_iter()
        .collect();
        assert_eq!(
            found.difference(&expected).collect::<Vec<_>>(),
            Vec::<&&str>::new(),
            "reported but not planted as a back door"
        );
        assert_eq!(
            expected.difference(&found).collect::<Vec<_>>(),
            Vec::<&&str>::new(),
            "planted but not reported"
        );
        for kept in [
            "Document::children",
            "Document::apply",
            "Document::read",
            "Document::fmt",
            "bytes",
            "element",
        ] {
            assert!(access.read.iter().any(|name| name == kept), "{kept} is read");
            assert!(
                !access
                    .offenders
                    .iter()
                    .any(|offender| offender.starts_with(&format!("{kept}:"))),
                "{kept} grants nothing"
            );
        }
        for unread in [
            "Document::private",
            "Document::near",
            "Document::inside",
            "Document::tested",
            "Hidden::hidden",
            "Hidden::apply_to",
        ] {
            assert!(
                !access.read.iter().any(|name| name == unread),
                "{unread} is not visible"
            );
        }
    }

    /// **`baumhard::document` names neither the model it replaces nor
    /// the glyph module, nor the structure it removes.** The failure
    /// lists every `file:line: identifier`.
    #[test]
    fn test_the_document_names_no_model_module() {
        let files = rust_files(&repo_path("lib/baumhard/src/document"));
        for witness in ["element.rs", "resolve.rs", "tests/test_support.rs"] {
            assert!(
                files.iter().any(|f| f.ends_with(witness)),
                "the scan must read document/{witness}; it read {files:?}"
            );
        }
        let mut offenders = Vec::new();
        for file in &files {
            let source = std::fs::read_to_string(file)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
            for (line, needle) in references_to(&source, FORBIDDEN) {
                offenders.push(format!("{}:{line}: {needle}", file.display()));
            }
        }
        assert!(
            offenders.is_empty(),
            "the document names:\n{}",
            offenders.join("\n")
        );
    }

    /// **The element data holds a `String` exactly where #235 allows
    /// one.** Both directions: a new `String` field fails, and so does
    /// an allowance for a field that no longer holds one.
    #[test]
    fn test_the_document_holds_strings_only_where_allowed() {
        let found: BTreeSet<String> = document_fields()
            .into_iter()
            .filter(|(_, _, string)| *string)
            .map(|(owner, field, _)| format!("{owner}.{field}"))
            .collect();
        let allowed: BTreeSet<String> = ALLOWED_STRINGS.iter().map(ToString::to_string).collect();
        assert_eq!(found, allowed, "document fields holding a String");
    }

    /// **No field D13 deletes exists in the document.** Precondition:
    /// the scan reads real fields, `opacity` among them.
    #[test]
    fn test_the_document_has_no_deleted_field() {
        let fields = document_fields();
        assert!(
            fields
                .iter()
                .any(|(owner, field, _)| owner == "NodeData" && field == "opacity"),
            "the scan must read NodeData's fields"
        );
        let deleted: Vec<String> = fields
            .iter()
            .filter(|(_, field, _)| DELETED_FIELDS.contains(&field.as_str()))
            .map(|(owner, field, _)| format!("{owner}.{field}"))
            .collect();
        assert!(deleted.is_empty(), "fields D13 deletes: {deleted:?}");
    }

    /// **Only the document and the benches name its test builder.**
    /// It is crate-private, which keeps the app crate out; this keeps
    /// the rest of baumhard out too. The failure lists every
    /// `file:line: identifier`.
    #[test]
    fn test_only_the_document_names_its_test_builder() {
        let files = crate::util::source_scan::workspace_rust_sources();
        let mut offenders = Vec::new();
        let mut read_inside = false;
        for file in &files {
            let relative = crate::util::source_scan::relative_to_repo(file);
            let inside = relative.starts_with("lib/baumhard/src/document/")
                || relative.starts_with("lib/baumhard/benches/");
            let source = std::fs::read_to_string(file)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
            let found = builder_references(&source);
            if inside {
                read_inside |= !found.is_empty();
                continue;
            }
            for (line, needle) in found {
                offenders.push(format!("{relative}:{line}: {needle}"));
            }
        }
        assert!(read_inside, "the scan must see the builder where it is allowed");
        assert!(
            offenders.is_empty(),
            "the document's test builder named outside it:\n{}",
            offenders.join("\n")
        );
    }

    /// **The writer's text budget counts every string the document may
    /// hold.** For each field [`ALLOWED_STRINGS`] names, every element
    /// that can carry it — the defaults' copies included — set to a
    /// seven-byte string, and nothing else: `text_bytes` counts seven.
    /// An allowance with no fixture fails, so a string field the
    /// document gains is a field the budget is made to count.
    #[test]
    fn test_the_text_budget_counts_every_string_the_document_allows() {
        use crate::document::element::PathData;
        use crate::document::element::{
            ConnectorData, DefaultsData, DocElement, FlowStyle, FrameData, FrameStyle, LinkData, MapData,
            PaletteData, PortalEndData, PortalEndStyle, RingData, RingStyle, SpanData, ThemeData,
            ThemeVariant,
        };
        use crate::document::values::{Anchor, FitMode, LinkKind, LinkTarget};
        use crate::document::writer::budget::text_bytes;

        const TEXT: &str = "seven b";
        let somewhere = indextree::Arena::new().new_node(());
        let flow = |flow: FlowStyle| PathData {
            flow,
            anchors: (Anchor::Auto, Anchor::Auto),
            control_points: Vec::new(),
            visible: true,
            zoom: None,
        };
        let filler = || {
            Some(FitMode::Once {
                filler: Some(TEXT.to_string()),
            })
        };
        let content = || FlowStyle {
            content: Some(TEXT.to_string()),
            ..FlowStyle::default()
        };
        let fills = || FlowStyle {
            fit: filler(),
            ..FlowStyle::default()
        };
        let defaults = |edit: &dyn Fn(&mut DefaultsData)| {
            let mut defaults = DefaultsData::default();
            edit(&mut defaults);
            DocElement::Defaults(Box::new(defaults))
        };
        let link = |path| {
            DocElement::Link(LinkData {
                to: somewhere,
                kind: LinkKind::Line,
                path,
            })
        };
        let fixtures = |field: &str| -> Vec<DocElement> {
            match field {
                "FitMode::Once.filler" => vec![
                    DocElement::Frame(FrameData {
                        style: FrameStyle {
                            fit: filler(),
                            ..FrameStyle::default()
                        },
                    }),
                    DocElement::Ring(RingData {
                        channel: 0,
                        style: RingStyle {
                            fit: filler(),
                            ..RingStyle::default()
                        },
                    }),
                    DocElement::Connector(ConnectorData { path: flow(fills()) }),
                    link(flow(fills())),
                    defaults(&|d| d.frame.fit = filler()),
                    defaults(&|d| d.ring.fit = filler()),
                    defaults(&|d| d.connector = fills()),
                    defaults(&|d| d.link = fills()),
                ],
                "FlowStyle.content" => vec![
                    DocElement::Connector(ConnectorData {
                        path: flow(content()),
                    }),
                    link(flow(content())),
                    defaults(&|d| d.connector = content()),
                    defaults(&|d| d.link = content()),
                ],
                "LinkTarget::Url.0" => vec![DocElement::Span(SpanData {
                    link: Some(LinkTarget::Url(TEXT.to_string())),
                    ..SpanData::default()
                })],
                "MapData.name" => vec![DocElement::Map(MapData {
                    name: TEXT.to_string(),
                })],
                "PaletteData.name" => vec![DocElement::Palette(PaletteData {
                    name: TEXT.to_string(),
                    groups: Vec::new(),
                })],
                "PortalEndStyle.content" => {
                    let style = PortalEndStyle {
                        content: Some(TEXT.to_string()),
                        ..PortalEndStyle::default()
                    };
                    vec![
                        DocElement::PortalEnd(PortalEndData {
                            end: 0,
                            style: style.clone(),
                        }),
                        defaults(&|d| d.portal_end = style.clone()),
                    ]
                }
                "SpanData.text" => vec![DocElement::Span(SpanData {
                    text: TEXT.to_string(),
                    ..SpanData::default()
                })],
                "ThemeData.variables" => vec![DocElement::Theme(ThemeData {
                    variables: vec![TEXT.to_string()],
                    ..ThemeData::default()
                })],
                "ThemeVariant.name" => vec![DocElement::Theme(ThemeData {
                    variants: vec![ThemeVariant {
                        name: TEXT.to_string(),
                        values: Vec::new(),
                    }],
                    ..ThemeData::default()
                })],
                other => panic!("no fixture for the allowed string {other}: the text budget must count it"),
            }
        };
        for field in ALLOWED_STRINGS {
            for element in fixtures(field) {
                assert_eq!(
                    text_bytes(&element),
                    TEXT.len() as u64,
                    "{field} on a {:?} is counted",
                    element.kind()
                );
            }
        }
    }

    /// The readers the gates above trust, held to planted sources.
    #[test]
    fn test_the_document_surface_scan_reads_what_it_claims_to() {
        let planted = "struct A { name: String, notes: Option<String>, count: u32 }\n\
                       enum B { Url(String), Verb(u16), Once { filler: Vec<String> } }\n";
        assert_eq!(
            fields(planted),
            vec![
                ("A".to_string(), "name".to_string(), true),
                ("A".to_string(), "notes".to_string(), true),
                ("A".to_string(), "count".to_string(), false),
                ("B::Url".to_string(), "0".to_string(), true),
                ("B::Verb".to_string(), "0".to_string(), false),
                ("B::Once".to_string(), "filler".to_string(), true),
            ]
        );
        let code = "use crate::mindmap::model::MindNode;\n// parent_id in prose\nfn f(edge_type: u8) {}\n";
        let planted = "use crate::document::tests::test_support::example;\nfn f(_: DocumentBuilder) {}\n\
                       mod test_support {}\n// DocumentBuilder in prose\n";
        assert_eq!(
            builder_references(planted),
            vec![(2, "DocumentBuilder"), (1, BUILDER_MODULE)],
            "the type and the qualified module are found; a bare `test_support` module and prose are not"
        );
        assert_eq!(
            references_to(code, FORBIDDEN),
            vec![(1, "mindmap"), (3, "edge_type")]
        );
    }
}
