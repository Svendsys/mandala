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
/// - `text_in`, `frame_in`, `flow_in` and `portal_in` build a style
///   from a DTO's fields, in a body, as an expression.
const ALLOWED_MACROS: &[(&str, Option<&str>)] = &[
    ("macro_rules", None),
    ("thread_local", None),
    ("spelled", Some("format/macros.rs")),
    ("forward_methods", Some("format/strict.rs")),
    ("forward_scalars", Some("format/strict.rs")),
    ("text_in", Some("format/macros.rs")),
    ("frame_in", Some("format/macros.rs")),
    ("flow_in", Some("format/macros.rs")),
    ("portal_in", Some("format/macros.rs")),
];

/// The macros of `std`, `core`, `alloc` and `log` that `document/` may
/// invoke inside a body, as a statement or an expression, where
/// [`ALLOWED_MACROS`] does not name them (ruling R40 on #238). Each
/// expands to an expression or a statement built from its arguments and
/// emits no item of its own, so the gate reads the arguments instead: an
/// item keyword among them ([`ITEM_KEYWORDS`]) is refused, and so is a
/// macro invoked among them that this list does not name. A
/// `macro_rules!` that reuses one of these names is refused under
/// `document/`, and one defined anywhere else in the crate makes the name
/// the crate's, no longer one of these.
const EXPRESSION_MACROS: &[&str] = &[
    "assert",
    "assert_eq",
    "assert_ne",
    "debug_assert",
    "debug_assert_eq",
    "debug_assert_ne",
    "matches",
    "vec",
    "format",
    "format_args",
    "write",
    "writeln",
    "panic",
    "unreachable",
    "todo",
    "unimplemented",
    "concat",
    "stringify",
    "include_str",
    "debug",
    "error",
    "warn",
    "info",
    "trace",
];

/// The keywords an item is written with, which an expression macro's
/// arguments never need: one among them is an item hidden from the gate
/// in tokens it does not parse. A lifetime's name (`'static`) is not one.
const ITEM_KEYWORDS: &[&str] = &[
    "impl", "fn", "struct", "enum", "union", "trait", "mod", "static", "const", "extern", "use", "type",
];

/// The visible items the gate allows one shape, each by the file under
/// `document/` that defines it and its full path there (ruling R40 on
/// #238: a bare name would excuse an impostor of the same name in another
/// file or module), with the shape and why it grants nothing.
///
/// An `impl Trait` allowed here is an `impl Iterator` whose `Item` the
/// gate still judges by the whole rule, so it yields nothing that grants
/// — ids, shared references, copies: a read API that walks what it reads
/// without allocating. A caller-chosen type parameter allowed here is
/// serde's (`T: Deserialize`, `D: Deserializer`, `S: Serializer`), or the
/// error type a caller's own callback returns: the value is built from
/// text or handed back from the caller's code, never a borrow into a
/// document.
const EXCEPTIONS: &[(&str, &str, &str, &str)] = &[
    (
        "ids.rs",
        "Document::children",
        IMPL_TRAIT,
        "an iterator of child ids",
    ),
    (
        "ids.rs",
        "Document::palettes",
        IMPL_TRAIT,
        "an iterator of shared palette references",
    ),
    (
        "writer/mod.rs",
        "Document::history",
        IMPL_TRAIT,
        "an iterator of history labels, sources and revisions",
    ),
    (
        "ids.rs",
        "palettes",
        IMPL_TRAIT,
        "an iterator of shared palette references, over a tree",
    ),
    (
        "deps.rs",
        "inputs_changed",
        IMPL_TRAIT,
        "an iterator of the resolution inputs a change touches, by value",
    ),
    (
        "ids.rs",
        "idents_named_by",
        IMPL_TRAIT,
        "an iterator of node ids a definition names, by value",
    ),
    (
        "format/index.rs",
        "parse_extent",
        PARAM,
        "serde: the value a deserializer reads from text",
    ),
    (
        "format/spelling.rs",
        "present",
        PARAM,
        "serde: the value a deserializer reads from text, and its error",
    ),
    (
        "format/spelling.rs",
        "serialize_spelled",
        PARAM,
        "serde: the serializer's own output",
    ),
    (
        "format/spelling.rs",
        "deserialize_spelled",
        PARAM,
        "serde: the value a deserializer reads from text, and its error",
    ),
    (
        "format/rows.rs",
        "Rows::each",
        PARAM,
        "the error the caller's own callback returns",
    ),
    (
        "format/rows.rs",
        "RowSource::each",
        HANDS,
        "the saver's row of items: the `&mut` it hands is to the sink that takes them, not into a document",
    ),
    (
        "format/rows.rs",
        "Rows::arena",
        LENDS,
        "takes the saver's row of items, whose `each` it hands that sink",
    ),
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

/// The shapes [`EXCEPTIONS`] can allow, in the words [`judge`] and
/// `judge_fn` name them.
const IMPL_TRAIT: &str = "an `impl Trait`";
/// See [`IMPL_TRAIT`].
const LENDS: &str = "lends a borrow to the caller's callback";
/// See [`IMPL_TRAIT`].
const HANDS: &str = "hands its implementor a borrow";
/// See [`IMPL_TRAIT`].
const PARAM: &str = "a caller-chosen type parameter";

/// Standard types a return may name from a function that takes a
/// `&mut`, because they hold no borrow of their own: whatever they hold
/// is named in their arguments and judged there.
const OWNED_STD: &[&str] = &[
    "Result", "Option", "Vec", "String", "Box", "bool", "char", "u8", "u16", "u32", "u64", "u128", "usize",
    "i8", "i16", "i32", "i64", "i128", "isize", "f32", "f64",
];

/// The document's internals: what a function that takes a `&mut` must
/// neither take nor give back by value (ruling R39 on #238), nor take
/// behind another `&mut` (R40), and what a visible `const`, `static` or
/// field must not hold by value (R40). A caller that could take the tree
/// out, edit it and put it back — or swap its own in — would change a
/// document without a `&mut` into it ever crossing; a shared reference
/// to one grants nothing, and is not refused. The document's elements
/// ([`Crate::elements`]) are held to the same rule but for fields and
/// consts, which carry an edit's values to `apply` and a default's.
const INTERNALS: &[&str] = &[
    "Tree",
    "Arena",
    "Definitions",
    "IdentIndex",
    "IncomingIndex",
    "Derived",
    "Writer",
    "History",
    "HistoryEntry",
    "Journal",
    "JournalBuilder",
];

/// What the gate names a visible function's round trip of an internal by.
const INTERNAL_BY_VALUE: &str = "takes or gives back a document internal by value";
/// See [`INTERNAL_BY_VALUE`].
const INTERNAL_BEHIND_MUT: &str = "takes a document internal behind a `&mut`";
/// See [`INTERNAL_BY_VALUE`].
const ELEMENT_BY_VALUE: &str = "takes or gives back a document element by value";
/// See [`INTERNAL_BY_VALUE`].
const ELEMENT_BEHIND_MUT: &str = "takes a document element behind a `&mut`";

/// What a visible `const`, `static` or field type is held to beyond the
/// rule for returns: nothing more (an alias or associated type, which
/// holds no value), [`INTERNALS`] by value (a `const` or a field), or
/// those and the elements by value (a `static`, the one that can stash).
#[derive(Clone, Copy, PartialEq)]
enum Holds {
    Nothing,
    Value,
    Stash,
}

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
    /// Every trait of the crate a method of which takes, beside its
    /// receiver, something that can hold a borrow: a callback trait, whose
    /// implementor a function would hand that borrow to.
    lending: BTreeSet<String>,
    /// Every type, alias and trait declared outside `document/`, by name:
    /// one so named is reachable, whatever `document/` declares of the
    /// same name.
    outside: BTreeSet<String>,
    /// Every `macro_rules!` declared outside `document/`, by name: one so
    /// named is the crate's, not one [`ALLOWED_MACROS`] or
    /// [`EXPRESSION_MACROS`] allows.
    outside_macros: BTreeSet<String>,
    /// The document's elements: every type `document/element.rs`
    /// declares that carries data — a struct or union, or an enum with a
    /// variant that has fields. A fieldless enum is a tag, holding no
    /// document data to round trip.
    elements: BTreeSet<String>,
}

impl Crate {
    /// Whether the type `name` is reachable beyond `document`: declared
    /// outside it, or declared there and visible, or not declared at all.
    fn reachable(&self, name: &str) -> bool {
        self.outside.contains(name) || self.local.get(name).copied().unwrap_or(true)
    }

    /// Whether `name` is a macro [`ALLOWED_MACROS`] allows, and not one
    /// the crate defines outside `document/` under the same name.
    fn allowed(&self, name: &str) -> bool {
        ALLOWED_MACROS.iter().any(|(allowed, _)| *allowed == name) && !self.outside_macros.contains(name)
    }

    /// Whether `name` is one of [`EXPRESSION_MACROS`], and not one the
    /// crate defines outside `document/` under the same name.
    fn expression(&self, name: &str) -> bool {
        EXPRESSION_MACROS.contains(&name) && !self.outside_macros.contains(name)
    }

    /// Whether `ty`, handed to someone else's code, can carry a way into
    /// a document: a `&mut` or a `*mut`, a lifetime, a type of the crate
    /// that declares one, or a type from outside the crate but the owned
    /// standard ones. A shared reference grants nothing by itself, and a
    /// name in `params` is the caller's own type.
    fn can_borrow(&self, ty: &Type, params: &BTreeSet<String>) -> bool {
        struct Borrow<'k> {
            krate: &'k Crate,
            params: &'k BTreeSet<String>,
            found: bool,
        }
        impl<'ast> Visit<'ast> for Borrow<'_> {
            fn visit_type_reference(&mut self, reference: &'ast TypeReference) {
                self.found |= reference.mutability.is_some();
                visit::visit_type_reference(self, reference);
            }
            fn visit_type_ptr(&mut self, pointer: &'ast syn::TypePtr) {
                self.found |= matches!(pointer.mutability, syn::PointerMutability::Mut(_));
                visit::visit_type_ptr(self, pointer);
            }
            fn visit_lifetime(&mut self, lifetime: &'ast syn::Lifetime) {
                self.found |= lifetime.ident != "static";
            }
            fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
                if path.qself.is_none() {
                    let last = path
                        .path
                        .segments
                        .last()
                        .map_or(String::new(), |segment| segment.ident.to_string());
                    self.found |= match self.krate.borrows.get(&last) {
                        Some(&borrows) => borrows,
                        None => {
                            !OWNED_STD.contains(&last.as_str())
                                && last != "Self"
                                && !self.params.contains(&last)
                        }
                    };
                }
                visit::visit_type_path(self, path);
            }
        }
        let mut borrow = Borrow {
            krate: self,
            params,
            found: false,
        };
        borrow.visit_type(ty);
        borrow.found
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

/// The shape [`judge`] names a `*mut` by.
const RAW: &str = "a `*mut`";

/// Why a type grants access, if it does: every offending shape found in
/// `ty`, each once and in the order met, resolving aliases and
/// associated types. Empty when it grants nothing. Judges a return type,
/// and the type of a visible `const`, `static`, field, alias or
/// associated type.
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
        fn visit_type_ptr(&mut self, pointer: &'ast syn::TypePtr) {
            if matches!(pointer.mutability, syn::PointerMutability::Mut(_)) {
                self.found(RAW);
            }
            visit::visit_type_ptr(self, pointer);
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
/// `document` that it judged, as `(file, path)`, and every way it found
/// to hand out access — `Owner::name: why` for a method, `name: why` for
/// a function or any other item, `Type.field: why` for a field, `name!`
/// for a macro, each path led by the inline modules and the enclosing
/// items it is nested in (`inner::deep`, `hide::Document::tree_in_fn`).
/// `file` is the file under `document/` being walked.
#[derive(Debug, Default, PartialEq)]
struct Access {
    file: String,
    read: Vec<(String, String)>,
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

/// The first pass over one file of the crate: what it declares, nested
/// items included. `file` is its path under `document/`, for one there.
fn declare(items: &[Item], depth: usize, in_document: bool, file: Option<&str>, krate: &mut Crate) {
    for item in items {
        if is_test_gated(item_attrs(item)) {
            continue;
        }
        if !matches!(item, Item::Mod(_)) {
            declare(&Nested::of_item(item).items, depth, in_document, None, krate);
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
                    declare(items, depth + 1, in_document, file, krate);
                }
                continue;
            }
            Item::Macro(mac) => {
                if let (false, Some(ident)) = (in_document, &mac.ident) {
                    krate.outside_macros.insert(ident.to_string());
                }
                continue;
            }
            _ => continue,
        };
        let carries_data = match item {
            Item::Struct(_) | Item::Union(_) => true,
            Item::Enum(item) => item.variants.iter().any(|variant| !variant.fields.is_empty()),
            _ => false,
        };
        if in_document && file == Some("element.rs") && carries_data {
            krate.elements.insert(name.clone());
        }
        let lifetimes = generics.lifetimes().next().is_some();
        *krate.borrows.entry(name.clone()).or_insert(false) |= lifetimes;
        if in_document {
            *krate.local.entry(name).or_insert(false) |= visible(vis, depth);
        } else {
            krate.outside.insert(name);
        }
    }
}

/// The second half of the first pass, once every type is known: the
/// callback traits of the crate ([`Crate::lending`]), nested ones too.
/// It reads traits; every other kind declares none.
fn find_lending(items: &[Item], krate: &mut Crate) {
    for item in items {
        if is_test_gated(item_attrs(item)) {
            continue;
        }
        if !matches!(item, Item::Mod(_)) {
            find_lending(&Nested::of_item(item).items, krate);
        }
        match item {
            Item::Trait(item) => {
                let params = type_params(&item.generics);
                let lends = item.items.iter().any(|member| match member {
                    syn::TraitItem::Fn(method) => method.sig.inputs.iter().any(|input| match input {
                        FnArg::Typed(typed) => krate.can_borrow(&typed.ty, &params),
                        FnArg::Receiver(_) => false,
                    }),
                    _ => false,
                });
                if lends {
                    krate.lending.insert(item.ident.to_string());
                }
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    find_lending(items, krate);
                }
            }
            _ => {}
        }
    }
}

/// The items and macros nested in an item's blocks and expressions — fn
/// bodies, `const` and `static` initializers, `const _` blocks, array
/// lengths, discriminants — which its own arms in [`walk_items`] do not
/// reach (ruling R40 on #238). It stops at each nested item, which is
/// walked in its turn, and skips test-gated members. A macro in a body is
/// one a statement, an expression or a pattern invokes (a pattern's is
/// an expression macro to `syn`); one in an item,
/// `impl` or trait position is judged where it stands. `type_macros`
/// counts macros in type position, whose expansion is a type the gate
/// cannot read.
#[derive(Default)]
struct Nested {
    items: Vec<Item>,
    macros: Vec<syn::Macro>,
    type_macros: usize,
    entered: bool,
}

impl Nested {
    /// What `item` nests. A module's items are walked as its own.
    fn of_item(item: &Item) -> Self {
        let mut nested = Nested::default();
        nested.visit_item(item);
        nested
    }

    /// What an expression nests: a `thread_local!` static's initializer.
    fn of_expr(expr: &syn::Expr) -> Self {
        let mut nested = Nested {
            entered: true,
            ..Nested::default()
        };
        nested.visit_expr(expr);
        nested
    }
}

/// The attributes of an `impl` member.
fn impl_member_attrs(member: &ImplItem) -> &[syn::Attribute] {
    match member {
        ImplItem::Const(member) => &member.attrs,
        ImplItem::Fn(member) => &member.attrs,
        ImplItem::Type(member) => &member.attrs,
        ImplItem::Macro(member) => &member.attrs,
        _ => &[],
    }
}

/// The attributes of a trait member.
fn trait_member_attrs(member: &syn::TraitItem) -> &[syn::Attribute] {
    match member {
        syn::TraitItem::Const(member) => &member.attrs,
        syn::TraitItem::Fn(member) => &member.attrs,
        syn::TraitItem::Type(member) => &member.attrs,
        syn::TraitItem::Macro(member) => &member.attrs,
        _ => &[],
    }
}

impl<'ast> Visit<'ast> for Nested {
    fn visit_item(&mut self, item: &'ast Item) {
        if !self.entered {
            self.entered = true;
            if !matches!(item, Item::Mod(_)) {
                visit::visit_item(self, item);
            }
        } else if !is_test_gated(item_attrs(item)) {
            self.items.push(item.clone());
        }
    }
    fn visit_impl_item(&mut self, member: &'ast ImplItem) {
        if !is_test_gated(impl_member_attrs(member)) {
            visit::visit_impl_item(self, member);
        }
    }
    fn visit_trait_item(&mut self, member: &'ast syn::TraitItem) {
        if !is_test_gated(trait_member_attrs(member)) {
            visit::visit_trait_item(self, member);
        }
    }
    fn visit_stmt_macro(&mut self, mac: &'ast syn::StmtMacro) {
        if !is_test_gated(&mac.attrs) {
            self.macros.push(mac.mac.clone());
        }
    }
    fn visit_expr_macro(&mut self, mac: &'ast syn::ExprMacro) {
        self.macros.push(mac.mac.clone());
    }
    fn visit_type_macro(&mut self, _: &'ast syn::TypeMacro) {
        self.type_macros += 1;
    }
}

/// Whether a signature's parameters or bounds lend: hand a caller's
/// callback something that can hold a borrow — through a closure bound
/// (`Fn`, `FnMut`, `FnOnce`), a bare `fn` pointer, or a callback trait
/// ([`Crate::lending`]) taken as `dyn`, `impl` or a generic bound. A
/// closure or `fn` input that is or holds a `&mut` or a `*mut` lends
/// whatever the function takes; one that can only hold a shared borrow
/// lends when the function takes a `&mut` (`takes_mut`), as it then has
/// a mutable borrow to hand on — the rule [`judge`] applies to returns.
fn lends(sig: &Signature, krate: &Crate, params: &BTreeSet<String>, takes_mut: bool) -> bool {
    struct Lends<'k> {
        krate: &'k Crate,
        params: &'k BTreeSet<String>,
        takes_mut: bool,
        found: bool,
    }
    impl Lends<'_> {
        fn input(&mut self, ty: &Type) {
            let mut mutable = MutFinder(false);
            mutable.visit_type(ty);
            let mut raw = RawFinder(false);
            raw.visit_type(ty);
            self.found |= mutable.0 || raw.0 || (self.takes_mut && self.krate.can_borrow(ty, self.params));
        }
    }
    impl<'ast> Visit<'ast> for Lends<'_> {
        fn visit_parenthesized_generic_arguments(
            &mut self,
            arguments: &'ast syn::ParenthesizedGenericArguments,
        ) {
            for input in &arguments.inputs {
                self.input(&input.ty);
            }
            visit::visit_parenthesized_generic_arguments(self, arguments);
        }
        fn visit_type_fn_ptr(&mut self, function: &'ast syn::TypeFnPtr) {
            for input in &function.inputs {
                self.input(&input.ty);
            }
            visit::visit_type_fn_ptr(self, function);
        }
        fn visit_trait_bound(&mut self, bound: &'ast syn::TraitBound) {
            let name = bound
                .path
                .segments
                .last()
                .map_or(String::new(), |segment| segment.ident.to_string());
            self.found |= self.krate.lending.contains(&name);
            visit::visit_trait_bound(self, bound);
        }
    }
    let mut finder = Lends {
        krate,
        params,
        takes_mut,
        found: false,
    };
    for input in &sig.inputs {
        if let FnArg::Typed(typed) = input {
            finder.visit_type(&typed.ty);
        }
    }
    finder.visit_generics(&sig.generics);
    finder.found
}

/// Whether `ty` names a type `wanted` picks by value — outside any
/// reference and any raw pointer — or, with `through_mut`, behind a
/// `&mut` too. Aliases are resolved, so an alias of one is one.
fn holds(ty: &Type, krate: &Crate, through_mut: bool, wanted: &dyn Fn(&str) -> bool) -> bool {
    holds_at(ty, krate, through_mut, wanted, 0)
}

/// [`holds`], `depth` aliases deep.
fn holds_at(
    ty: &Type,
    krate: &Crate,
    through_mut: bool,
    wanted: &dyn Fn(&str) -> bool,
    depth: usize,
) -> bool {
    struct Holds<'k> {
        krate: &'k Crate,
        through_mut: bool,
        wanted: &'k dyn Fn(&str) -> bool,
        depth: usize,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Holds<'_> {
        fn visit_type_reference(&mut self, reference: &'ast TypeReference) {
            if self.through_mut && reference.mutability.is_some() {
                visit::visit_type_reference(self, reference);
            }
        }
        fn visit_type_ptr(&mut self, _: &'ast syn::TypePtr) {}
        fn visit_type_path(&mut self, path: &'ast syn::TypePath) {
            if let (None, [segment]) = (
                &path.qself,
                path.path.segments.iter().collect::<Vec<_>>().as_slice(),
            ) {
                if let Some(alias) = self.krate.aliases.get(&segment.ident.to_string()) {
                    self.found |= self.depth < 8
                        && holds_at(alias, self.krate, self.through_mut, self.wanted, self.depth + 1);
                }
            }
            visit::visit_type_path(self, path);
        }
        fn visit_path_segment(&mut self, segment: &'ast PathSegment) {
            self.found |= (self.wanted)(&segment.ident.to_string());
            visit::visit_path_segment(self, segment);
        }
    }
    let mut finder = Holds {
        krate,
        through_mut,
        wanted,
        depth,
        found: false,
    };
    finder.visit_type(ty);
    finder.found
}

/// Whether `name` is one of [`INTERNALS`].
fn is_internal(name: &str) -> bool {
    INTERNALS.contains(&name)
}

/// Judge one signature, as `name`, and note it read. `defined` is set
/// for a trait's own method, which a caller can implement: its
/// parameters are then judged as what the trait hands its implementor.
fn judge_fn(
    name: String,
    trait_name: Option<&str>,
    defined: bool,
    sig: &Signature,
    mut scope: Scope<'_>,
    access: &mut Access,
) {
    scope.params.extend(type_params(&sig.generics));
    let typed: Vec<&Type> = sig
        .inputs
        .iter()
        .filter_map(|input| match input {
            FnArg::Typed(typed) => Some(&*typed.ty),
            FnArg::Receiver(_) => None,
        })
        .collect();
    scope.takes_mut = sig.inputs.iter().any(|input| match input {
        FnArg::Receiver(receiver) => matches!(receiver.kind, syn::ReceiverKind::Reference(_, _, Some(_))),
        FnArg::Typed(typed) => {
            let mut finder = MutFinder(false);
            finder.visit_type(&typed.ty);
            finder.0
        }
    });
    let file = access.file.clone();
    let excepted = |why: &str| {
        EXCEPTIONS
            .iter()
            .any(|(home, item, shape, _)| *home == file && *item == name && *shape == why)
            || (why == PARAM && trait_name.is_some_and(|t| EXCEPTED_TRAITS.iter().any(|(e, _)| *e == t)))
    };
    let mut found: Vec<&'static str> = Vec::new();
    if lends(sig, scope.krate, &scope.params, scope.takes_mut) {
        found.push(LENDS);
    }
    if defined && typed.iter().any(|ty| scope.krate.can_borrow(ty, &scope.params)) {
        found.push(HANDS);
    }
    if typed.iter().any(|ty| {
        let mut raw = RawFinder(false);
        raw.visit_type(ty);
        raw.0
    }) {
        found.push(RAW);
    }
    let returned = match &sig.output {
        ReturnType::Type(_, ty) => Some(&**ty),
        ReturnType::Default => None,
    };
    // A round trip (rulings R39 and R40 on #238): an internal or an
    // element taken or given back by value by a function that takes a
    // `&mut`, or taken behind a `&mut` by one that takes another — a
    // swap needs two places to write.
    let mutable: Vec<bool> = sig
        .inputs
        .iter()
        .map(|input| match input {
            FnArg::Receiver(receiver) => matches!(receiver.kind, syn::ReceiverKind::Reference(_, _, Some(_))),
            FnArg::Typed(typed) => {
                let mut finder = MutFinder(false);
                finder.visit_type(&typed.ty);
                finder.0
            }
        })
        .collect();
    let is_element = |name: &str| scope.krate.elements.contains(name);
    for (wanted, by_value, behind) in [
        (
            &is_internal as &dyn Fn(&str) -> bool,
            INTERNAL_BY_VALUE,
            INTERNAL_BEHIND_MUT,
        ),
        (&is_element, ELEMENT_BY_VALUE, ELEMENT_BEHIND_MUT),
    ] {
        let mut shapes = Vec::new();
        for (index, input) in sig.inputs.iter().enumerate() {
            let FnArg::Typed(typed) = input else { continue };
            let another = mutable.iter().enumerate().any(|(other, m)| other != index && *m);
            if scope.takes_mut && holds(&typed.ty, scope.krate, false, wanted) {
                shapes.push(by_value);
            } else if another && holds(&typed.ty, scope.krate, true, wanted) {
                shapes.push(behind);
            }
        }
        if scope.takes_mut && returned.is_some_and(|ty| holds(ty, scope.krate, false, wanted)) {
            shapes.push(by_value);
        }
        for shape in shapes {
            if !found.contains(&shape) {
                found.push(shape);
            }
        }
    }
    if let Some(ty) = returned {
        found.extend(
            judge(ty, &scope, 0)
                .into_iter()
                .filter(|why| !excepted(why) || (*why == IMPL_TRAIT && !iterator_items_pass(ty, &scope))),
        );
    }
    for why in found
        .into_iter()
        .filter(|why| !excepted(why) || *why == IMPL_TRAIT)
    {
        access.offenders.push(format!("{name}: {why}"));
    }
    access.read.push((file, name));
}

/// Judge the type of a visible `const`, `static`, field, alias or
/// associated type, as `name`, the way a return type is judged — and,
/// for one that holds a callback, the way a parameter is — and what it
/// may not hold by value besides ([`Holds`]).
fn judge_type(
    name: String,
    trait_name: Option<&str>,
    ty: &Type,
    scope: &Scope<'_>,
    holding: Holds,
    access: &mut Access,
) {
    let excepted =
        |why: &str| why == PARAM && trait_name.is_some_and(|t| EXCEPTED_TRAITS.iter().any(|(e, _)| *e == t));
    if let Some(why) = judge(ty, scope, 0).into_iter().find(|why| !excepted(why)) {
        access.offenders.push(format!("{name}: {why}"));
    }
    if holding != Holds::Nothing && holds(ty, scope.krate, false, &is_internal) {
        access
            .offenders
            .push(format!("{name}: holds a document internal by value"));
    }
    if holding == Holds::Stash
        && holds(ty, scope.krate, false, &|name| {
            scope.krate.elements.contains(name)
        })
    {
        access
            .offenders
            .push(format!("{name}: holds a document element by value"));
    }
    struct Callback<'k> {
        krate: &'k Crate,
        found: bool,
    }
    impl<'ast> Visit<'ast> for Callback<'_> {
        fn visit_type_fn_ptr(&mut self, function: &'ast syn::TypeFnPtr) {
            for input in &function.inputs {
                self.found |= self.krate.can_borrow(&input.ty, &BTreeSet::new());
            }
            visit::visit_type_fn_ptr(self, function);
        }
    }
    let mut callback = Callback {
        krate: scope.krate,
        found: false,
    };
    callback.visit_type(ty);
    if callback.found {
        access.offenders.push(format!("{name}: {CALLBACK}"));
    }
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

/// Finds any `&mut` in a type.
struct MutFinder(bool);

impl<'ast> Visit<'ast> for MutFinder {
    fn visit_type_reference(&mut self, reference: &'ast TypeReference) {
        self.0 |= reference.mutability.is_some();
        visit::visit_type_reference(self, reference);
    }
}

/// Finds any `*mut` in a type.
struct RawFinder(bool);

impl<'ast> Visit<'ast> for RawFinder {
    fn visit_type_ptr(&mut self, pointer: &'ast syn::TypePtr) {
        self.0 |= matches!(pointer.mutability, syn::PointerMutability::Mut(_));
        visit::visit_type_ptr(self, pointer);
    }
}

/// The shape [`judge_type`] names a `fn` pointer it holds by, whose
/// inputs can hold a borrow: a field a caller sets to its own function,
/// which the document's code then calls with what it has.
const CALLBACK: &str = "holds a callback that can be handed a borrow";

/// The shape a kind of item the gate does not judge is reported by.
const UNJUDGED: &str = "an item kind the gate does not judge";

/// Judge the fields of a visible struct, union or enum variant.
fn judge_fields(
    owner: &str,
    fields: &Fields,
    shown: impl Fn(&Visibility) -> bool,
    scope: &Scope<'_>,
    access: &mut Access,
) {
    for (i, field) in fields.iter().enumerate() {
        if shown(&field.vis) {
            let name = field.ident.as_ref().map_or(i.to_string(), ToString::to_string);
            judge_type(
                format!("{owner}.{name}"),
                None,
                &field.ty,
                scope,
                Holds::Value,
                access,
            );
        }
    }
}

/// The second pass over one file of `document/`: every item, by kind,
/// each named under `path` — the inline modules and enclosing items it is
/// nested in, `::`-terminated. A kind it has no arm for is an offender
/// (ruling R39 on #238), so an item `syn` grows is judged before it can
/// be used; and every item's blocks and expressions are walked for the
/// items and macros nested in them (R40, [`Nested`]).
fn walk_items(items: &[Item], depth: usize, file: &str, path: &str, krate: &Crate, access: &mut Access) {
    let scope = |params: BTreeSet<String>, assoc: BTreeMap<String, Type>| Scope {
        krate,
        params,
        assoc,
        takes_mut: false,
    };
    let plain = || scope(BTreeSet::new(), BTreeMap::new());
    for item in items {
        if is_test_gated(item_attrs(item)) {
            continue;
        }
        match item {
            Item::Fn(item) => {
                if visible(&item.vis, depth) {
                    let name = format!("{path}{}", item.sig.ident);
                    judge_fn(name, None, false, &item.sig, plain(), access);
                }
            }
            Item::Const(item) => {
                if visible(&item.vis, depth) {
                    let name = format!("{path}{}", item.ident);
                    judge_type(name, None, &item.ty, &plain(), Holds::Value, access);
                }
            }
            Item::Static(item) => {
                if visible(&item.vis, depth) {
                    let name = format!("{path}{}", item.ident);
                    judge_type(name, None, &item.ty, &plain(), Holds::Stash, access);
                }
                if matches!(item.mutability, syn::StaticMutability::Mut(_)) {
                    access
                        .offenders
                        .push(format!("{path}{}: a `static mut`", item.ident));
                }
            }
            Item::Type(item) => {
                if visible(&item.vis, depth) {
                    let name = format!("type {path}{}", item.ident);
                    judge_type(name, None, &item.ty, &plain(), Holds::Nothing, access);
                }
            }
            Item::Struct(item) => {
                if visible(&item.vis, depth) {
                    let owner = format!("{path}{}", item.ident);
                    judge_fields(&owner, &item.fields, |vis| visible(vis, depth), &plain(), access);
                }
            }
            Item::Union(item) => {
                if visible(&item.vis, depth) {
                    let fields = Fields::Named(item.fields.clone());
                    let owner = format!("{path}{}", item.ident);
                    judge_fields(&owner, &fields, |vis| visible(vis, depth), &plain(), access);
                }
            }
            Item::Enum(item) => {
                if visible(&item.vis, depth) {
                    for variant in &item.variants {
                        let owner = format!("{path}{}::{}", item.ident, variant.ident);
                        judge_fields(&owner, &variant.fields, |_| true, &plain(), access);
                    }
                }
            }
            Item::Trait(item) => {
                if visible(&item.vis, depth) {
                    walk_trait(item, path, krate, access);
                }
            }
            Item::Impl(item) => walk_impl(item, depth, path, krate, access),
            Item::Macro(mac) => {
                let name = macro_name(&mac.mac);
                if name == "macro_rules" {
                    let defined = mac.ident.as_ref().map(ToString::to_string).unwrap_or_default();
                    if let Some((_, home)) = ALLOWED_MACROS.iter().find(|(allowed, _)| *allowed == defined) {
                        if *home != Some(file) {
                            access
                                .offenders
                                .push(format!("macro_rules! {path}{defined}: reuses an allowed name"));
                        }
                    }
                    if EXPRESSION_MACROS.contains(&defined.as_str()) {
                        access.offenders.push(format!(
                            "macro_rules! {path}{defined}: reuses a standard macro's name"
                        ));
                    }
                } else if name == "thread_local" && krate.allowed(&name) {
                    thread_local(&mac.mac, depth, file, path, krate, access);
                } else {
                    invoked(&name, path, krate, access);
                }
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    let inner = format!("{path}{}::", module.ident);
                    walk_items(items, depth + 1, file, &inner, krate, access);
                }
            }
            // A `use` names an item; re-exporting one that is not visible
            // is the compiler's to refuse (E0364). An `extern crate`
            // names a crate.
            Item::Use(_) | Item::ExternCrate(_) => {}
            other => access
                .offenders
                .push(format!("{path}{}: {UNJUDGED}", item_kind(other))),
        }
        if !matches!(item, Item::Mod(_) | Item::Use(_) | Item::ExternCrate(_)) {
            let place = format!("{path}{}", item_place(item));
            walk_nested(Nested::of_item(item), &place, depth, file, krate, access);
        }
    }
}

/// The name an item's nested items are walked under: its own, its self
/// type's for an `impl`, its macro's for a macro.
fn item_place(item: &Item) -> String {
    match item {
        Item::Fn(item) => item.sig.ident.to_string(),
        Item::Const(item) => item.ident.to_string(),
        Item::Static(item) => item.ident.to_string(),
        Item::Type(item) => item.ident.to_string(),
        Item::Struct(item) => item.ident.to_string(),
        Item::Union(item) => item.ident.to_string(),
        Item::Enum(item) => item.ident.to_string(),
        Item::Trait(item) => item.ident.to_string(),
        Item::Impl(item) => type_name(&item.self_ty),
        Item::Macro(mac) => format!("{}!", macro_name(&mac.mac)),
        other => item_kind(other).to_string(),
    }
}

/// Walk what an item or expression at `place` nests: its items, under
/// `place`, by the rules of [`walk_items`]; the macros of its bodies by
/// [`body_macro`]; and a macro in type position, which is refused.
fn walk_nested(nested: Nested, place: &str, depth: usize, file: &str, krate: &Crate, access: &mut Access) {
    for mac in &nested.macros {
        body_macro(mac, place, depth, file, krate, access);
    }
    if nested.type_macros > 0 {
        access
            .offenders
            .push(format!("{place}: a macro in type position"));
    }
    walk_items(&nested.items, depth, file, &format!("{place}::"), krate, access);
}

/// Judge a macro a body at `place` invokes. One [`ALLOWED_MACROS`] names
/// is read as it is where an item goes; a `thread_local!` is parsed and
/// its statics judged; one of [`EXPRESSION_MACROS`] passes when its
/// arguments hold no item ([`argument_offense`]); any other is refused.
fn body_macro(mac: &syn::Macro, place: &str, depth: usize, file: &str, krate: &Crate, access: &mut Access) {
    let name = macro_name(mac);
    if name == "thread_local" && krate.allowed(&name) {
        thread_local(mac, depth, file, &format!("{place}::"), krate, access);
    } else if name == "macro_rules" || !(krate.allowed(&name) || krate.expression(&name)) {
        access.offenders.push(format!("{place}: {name}!"));
    } else if krate.expression(&name) {
        if let Some(why) = argument_offense(mac.tokens.clone(), krate) {
            access.offenders.push(format!("{place}: {name}! {why}"));
        }
    }
}

/// Why an expression macro's arguments hide an item, if they do: an
/// [`ITEM_KEYWORDS`] word outside a lifetime, or a macro invoked among
/// them that is not one of [`EXPRESSION_MACROS`].
fn argument_offense(tokens: proc_macro2::TokenStream, krate: &Crate) -> Option<String> {
    use proc_macro2::TokenTree;
    let trees: Vec<TokenTree> = tokens.into_iter().collect();
    for (index, tree) in trees.iter().enumerate() {
        match tree {
            TokenTree::Ident(ident) => {
                let word = ident.to_string();
                let lifetime =
                    index > 0 && matches!(&trees[index - 1], TokenTree::Punct(p) if p.as_char() == '\'');
                if !lifetime && ITEM_KEYWORDS.contains(&word.as_str()) {
                    return Some(format!("holds an item (`{word}`) in its arguments"));
                }
                let invokes = matches!(trees.get(index + 1), Some(TokenTree::Punct(p)) if p.as_char() == '!')
                    && matches!(trees.get(index + 2), Some(TokenTree::Group(_)));
                if invokes && !krate.expression(&word) {
                    return Some(format!("invokes `{word}!` in its arguments"));
                }
            }
            TokenTree::Group(group) => {
                if let Some(why) = argument_offense(group.stream(), krate) {
                    return Some(why);
                }
            }
            TokenTree::Punct(_) | TokenTree::Literal(_) => {}
        }
    }
    None
}

/// One static a `thread_local!` declares.
struct LocalStatic {
    attrs: Vec<syn::Attribute>,
    vis: Visibility,
    ident: syn::Ident,
    ty: Type,
    expr: syn::Expr,
}

/// The statics a `thread_local!` declares, parsed from its tokens:
/// `[attrs] [vis] static NAME: Type = init;`, the last `;` optional.
fn local_statics(tokens: proc_macro2::TokenStream) -> syn::Result<Vec<LocalStatic>> {
    use syn::parse::Parser;
    let parse = |input: syn::parse::ParseStream<'_>| {
        let mut statics = Vec::new();
        while !input.is_empty() {
            let attrs = input.call(syn::Attribute::parse_outer)?;
            let vis: Visibility = input.parse()?;
            input.parse::<syn::Token![static]>()?;
            let ident: syn::Ident = input.parse()?;
            input.parse::<syn::Token![:]>()?;
            let ty: Type = input.parse()?;
            input.parse::<syn::Token![=]>()?;
            let expr: syn::Expr = input.parse()?;
            if !input.is_empty() {
                input.parse::<syn::Token![;]>()?;
            }
            statics.push(LocalStatic {
                attrs,
                vis,
                ident,
                ty,
                expr,
            });
        }
        Ok(statics)
    };
    parse.parse2(tokens)
}

/// Judge the statics a `thread_local!` declares under `path` (ruling R40
/// on #238): a `pub` one is refused, a visible one is judged as a
/// `static` is, and each initializer is walked for what it nests.
fn thread_local(mac: &syn::Macro, depth: usize, file: &str, path: &str, krate: &Crate, access: &mut Access) {
    let Ok(statics) = local_statics(mac.tokens.clone()) else {
        access.offenders.push(format!(
            "{path}thread_local!: a declaration the gate cannot parse"
        ));
        return;
    };
    let scope = Scope {
        krate,
        params: BTreeSet::new(),
        assoc: BTreeMap::new(),
        takes_mut: false,
    };
    for local in statics {
        if is_test_gated(&local.attrs) {
            continue;
        }
        let name = format!("{path}{}", local.ident);
        if matches!(local.vis, Visibility::Public(_)) {
            access.offenders.push(format!("{name}: a `pub` `thread_local!`"));
        }
        if visible(&local.vis, depth) {
            judge_type(name.clone(), None, &local.ty, &scope, Holds::Stash, access);
        }
        walk_nested(Nested::of_expr(&local.expr), &name, depth, file, krate, access);
    }
}

/// A name for an item `syn` parsed that the gate has no arm for.
fn item_kind(item: &Item) -> &'static str {
    match item {
        Item::ForeignMod(_) => "extern block",
        Item::TraitAlias(_) => "trait alias",
        Item::Verbatim(_) => "verbatim item",
        _ => "item",
    }
}

/// Judge a visible trait: its methods (required and default) — their
/// parameters too, which its implementors are handed — its associated
/// consts and the defaults of its associated types.
fn walk_trait(item: &syn::ItemTrait, path: &str, krate: &Crate, access: &mut Access) {
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
    let scope = || Scope {
        krate,
        params: type_params(&item.generics),
        assoc: defaults.clone(),
        takes_mut: false,
    };
    let bare = item.ident.to_string();
    let owner = format!("{path}{bare}");
    for member in &item.items {
        match member {
            syn::TraitItem::Fn(method) => {
                if !is_test_gated(&method.attrs) {
                    let name = format!("{owner}::{}", method.sig.ident);
                    judge_fn(name, Some(&bare), true, &method.sig, scope(), access);
                }
            }
            syn::TraitItem::Const(constant) => {
                judge_type(
                    format!("{owner}::{}", constant.ident),
                    Some(&bare),
                    &constant.ty,
                    &scope(),
                    Holds::Value,
                    access,
                );
            }
            syn::TraitItem::Type(ty) => {
                if let Some((_, default)) = &ty.default {
                    judge_type(
                        format!("{owner}::{}", ty.ident),
                        Some(&bare),
                        default,
                        &scope(),
                        Holds::Nothing,
                        access,
                    );
                }
            }
            syn::TraitItem::Macro(mac) => {
                invoked(&macro_name(&mac.mac), &format!("{owner}: "), krate, access)
            }
            _ => access
                .offenders
                .push(format!("{owner}: {UNJUDGED} (a trait member)")),
        }
    }
}

/// Judge an `impl` that is reachable beyond `document`: its type is, and
/// — for a trait's — its trait is; whatever its self type, a trait
/// implemented for a type outside `document` is reachable. Its methods,
/// associated consts and associated types are judged.
fn walk_impl(item: &syn::ItemImpl, depth: usize, path: &str, krate: &Crate, access: &mut Access) {
    let bare = type_name(&item.self_ty);
    let owner = format!("{path}{bare}");
    let trait_name = item
        .trait_
        .as_ref()
        .and_then(|(path, _)| path.segments.last())
        .map(|segment| segment.ident.to_string());
    let reachable = krate.reachable(&bare) && trait_name.as_deref().is_none_or(|name| krate.reachable(name));
    let assoc: BTreeMap<String, Type> = item
        .items
        .iter()
        .filter_map(|member| match member {
            ImplItem::Type(ty) => Some((ty.ident.to_string(), ty.ty.clone())),
            _ => None,
        })
        .collect();
    let scope = || Scope {
        krate,
        params: type_params(&item.generics),
        assoc: assoc.clone(),
        takes_mut: false,
    };
    let shown = |vis: &Visibility| reachable && (item.trait_.is_some() || visible(vis, depth));
    for member in &item.items {
        match member {
            ImplItem::Fn(method) => {
                if shown(&method.vis) && !is_test_gated(&method.attrs) {
                    let name = format!("{owner}::{}", method.sig.ident);
                    judge_fn(name, trait_name.as_deref(), false, &method.sig, scope(), access);
                }
            }
            ImplItem::Const(constant) => {
                if shown(&constant.vis) {
                    judge_type(
                        format!("{owner}::{}", constant.ident),
                        trait_name.as_deref(),
                        &constant.ty,
                        &scope(),
                        Holds::Value,
                        access,
                    );
                }
            }
            ImplItem::Type(ty) => {
                if shown(&ty.vis) {
                    judge_type(
                        format!("{owner}::{}", ty.ident),
                        trait_name.as_deref(),
                        &ty.ty,
                        &scope(),
                        Holds::Nothing,
                        access,
                    );
                }
            }
            ImplItem::Macro(mac) => {
                if !is_test_gated(&mac.attrs) {
                    invoked(&macro_name(&mac.mac), &format!("{owner}: "), krate, access);
                }
            }
            _ => access
                .offenders
                .push(format!("{owner}: {UNJUDGED} (an impl member)")),
        }
    }
}

/// Refuse an invocation, where an item or a member goes, of `name` that
/// [`ALLOWED_MACROS`] does not allow ([`Crate::allowed`]).
fn invoked(name: &str, owner: &str, krate: &Crate, access: &mut Access) {
    if !krate.allowed(name) {
        access.offenders.push(format!("{owner}{name}!"));
    }
}

/// Everything in `document` — each source its path under `document/`,
/// its text and how many modules below the crate root it sits — that
/// hands out a way to change a document, judged by shape and complete
/// by item kind (rulings R37, R39 and R40 on #238). Every item kind
/// `syn` parses is judged — a kind it has no arm for is an offender —
/// wherever it is nested: in a module, a fn body, a `const` or `static`
/// initializer, a `const _` block ([`Nested`]). An item reaches beyond
/// `document` by [`visible`]: a function, an `impl` method of a
/// reachable type, every method of a trait defined there or implemented
/// there whatever its self type (default bodies included).
///
/// A function offends when, after aliases and associated types are
/// resolved:
/// - its return holds a `&mut` or a `*mut`, whatever it points at;
/// - its return holds an `impl Trait` or a `dyn Trait` (but
///   [`EXCEPTIONS`]), or a type parameter its caller chooses;
/// - it takes a `&mut`, and its return holds a lifetime or a type that
///   can hold one ([`Crate::can_borrow`]), or it takes or gives back one
///   of [`INTERNALS`] or of the elements ([`Crate::elements`]) by value,
///   or takes one behind a `&mut` while it takes another `&mut` too;
/// - it lends: a closure bound or a bare `fn` pointer among its
///   parameters or bounds is handed a `&mut` or a `*mut` — or, when it
///   takes a `&mut`, anything that can hold a borrow — or a callback
///   trait is among them ([`lends`]);
/// - it is a trait's own method, and its parameters can hold a borrow,
///   which a caller's implementation would receive;
/// - a parameter holds a `*mut`.
///
/// A visible `const`, `static`, field, alias or associated type offends
/// as a return type would, and when it holds a `fn` pointer whose inputs
/// can hold a borrow ([`CALLBACK`]); a `const`, `static` or field when
/// it holds one of [`INTERNALS`] by value, and a `static` an element.
/// The statics a `thread_local!` declares are judged as a `static` is,
/// and a `pub` one offends. A `static mut` offends, and so does a macro
/// in item position that [`ALLOWED_MACROS`] does not name, a macro in a
/// body that neither it nor [`EXPRESSION_MACROS`] names or whose
/// arguments hide an item, a macro in type position, and a
/// `macro_rules!` that reuses an allowed name outside its file or a
/// standard one anywhere. `others` are the rest of the crate's
/// sources, read for the types and traits they declare. Test-gated
/// items are not read. The compiler's privacy on the tree and the
/// element fields is what keeps a document closed, and
/// `forbid(unsafe_code)` on the crate closes the raw-pointer route; this
/// makes sure nothing in `document/` exports a way around either.
fn mutable_access(sources: &[(&str, &str, usize)], others: &[&str]) -> Access {
    let parse = |source: &str| syn::parse_file(source).expect("the crate's sources parse");
    let documents: Vec<(&str, syn::File, usize)> = sources
        .iter()
        .map(|&(file, source, depth)| (file, parse(source), depth))
        .collect();
    let others: Vec<syn::File> = others.iter().map(|other| parse(other)).collect();
    let mut krate = Crate::default();
    for other in &others {
        declare(&other.items, 1, false, None, &mut krate);
    }
    for (path, file, depth) in &documents {
        declare(&file.items, *depth, true, Some(path), &mut krate);
    }
    for file in others.iter().chain(documents.iter().map(|(_, file, _)| file)) {
        find_lending(&file.items, &mut krate);
    }
    let mut access = Access::default();
    for (path, file, depth) in &documents {
        access.file = path.to_string();
        walk_items(&file.items, *depth, path, "", &krate, &mut access);
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
    /// document** (design §8.4, #238 §7, rulings R37, R39 and R40). No
    /// item the rest of the crate or the app can reach, wherever it is
    /// nested, offends by the rules of [`mutable_access`]: no function —
    /// free, inherent, or of a trait defined or implemented there,
    /// whatever its self type — returns a `&mut`, an `impl Trait` or
    /// `dyn Trait` the exceptions do not name, a type its caller chooses,
    /// or a borrow of a `&mut` it took, lends a borrow to its caller's
    /// callback, or round trips an internal or an element; no visible
    /// `const`, `static`, field or thread-local holds what it may not; and
    /// no macro whose expansion the gate cannot read is invoked.
    /// Preconditions: the scan reads the writer's API and the trait impls
    /// of the elements, and every exception names an item it read at its
    /// file and path.
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
                access.read.iter().any(|(_, name)| name == witness),
                "the scan must read {witness}"
            );
        }
        for (file, item, _, _) in EXCEPTIONS {
            assert!(
                access
                    .read
                    .iter()
                    .any(|(read, name)| read == file && name == item),
                "the exception {file}: {item} must name an item the scan reads"
            );
        }
        assert!(
            access.offenders.is_empty(),
            "these hand out a way to change a document; change it through `Document::apply`:\n{}",
            access.offenders.join("\n")
        );
    }

    /// The negative control of the gate above: each shape an item of any
    /// kind can hand out access by is reported — every shape the reviews
    /// of #299 found among them — and each that cannot is not. Each
    /// planted offender must be reported for the shape it was planted
    /// for; it may be reported for others too, and nothing else may be
    /// reported.
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
    pub fn boxed(&self) -> Box<dyn Any> { todo!() }
    pub fn chosen<T: Default>(&self) -> T { todo!() }
    pub fn byte<'a>(&'a mut self) -> Option<&'a u8> { todo!() }
    pub fn shared(&mut self) -> Option<&u8> { todo!() }
    pub fn visit_tree(&mut self, v: &mut dyn TreeVisitor) {}
    pub fn visit_generic<V: TreeVisitor>(&mut self, v: &mut V) {}
    pub fn visit_where<V>(&mut self, v: V) where V: TreeVisitor {}
    pub fn with_fn(&mut self, f: fn(&mut Tree<DocElement>)) {}
    pub fn with_guard(&mut self, f: impl FnOnce(OutGuard<'_>)) {}
    pub fn replace_tree(&mut self, t: Tree<DocElement>) -> Tree<DocElement> { todo!() }
    pub fn take_journal(&mut self) -> Option<Journal<DocElement>> { todo!() }
    pub fn put_tree(&mut self, t: Tree<DocElement>) {}
    pub fn tree_ptr(&mut self) -> *mut Tree<DocElement> { todo!() }
    pub fn from_ptr(&self, p: *mut Tree<DocElement>) {}
    pub fn shared_tree(&mut self, t: &Tree<DocElement>) {}
    pub fn visit_shared(&self, f: impl FnOnce(&Document)) {}
    pub const OPENER: fn(&mut Document) -> &mut Tree<DocElement> = open_tree;
    pub const LIMIT: usize = 3;
    pub fn swap_tree(&mut self, slot: &mut Tree<DocElement>) {}
    pub fn set_node(&mut self, node: NodeData) {}
    pub fn take_element(&mut self) -> DocElement { todo!() }
    pub fn swap_node(&mut self, slot: &mut NodeData) {}
    pub fn read_node(&self, node: NodeData) {}
    pub fn kind_of(&mut self, kind: DocKind) {}
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
pub trait TreeVisitor { fn visit(&mut self, tree: &mut Tree<DocElement>); }
pub trait Named { fn name(&self) -> String; }
pub fn named(n: &dyn Named) {}
pub const OPEN: fn(&mut Document) -> &mut Tree<DocElement> = open_tree;
pub static OPENED: fn(&mut Document) -> &mut Tree<DocElement> = open_tree;
pub static mut COUNTER: u32 = 0;
pub const RAW: Option<*mut Tree<DocElement>> = None;
pub struct RawHolder { pub p: *mut Tree<DocElement> }
pub struct Hooks { pub on_edit: fn(OutGuard<'_>) }
pub trait Opens {
    const OPEN: fn(&mut Document) -> &mut Tree<DocElement>;
    type Handle = TreeMut<'static>;
    tree_in_trait!();
}
impl Document { fn bodiless(); }
const _: () = { impl Document { pub fn tree_in_const(&mut self) -> &mut Tree<DocElement> { todo!() } } };
fn hide() { impl Document { pub fn tree_in_fn(&mut self) -> &mut Tree<DocElement> { todo!() } } }
pub static IN_STATIC: u8 = { impl Document { pub fn tree_in_static(&mut self) -> &mut Tree<DocElement> { todo!() } } 0 };
pub struct Host;
impl Host { fn host(&self) { impl Document { pub fn tree_in_method(&mut self) -> &mut Tree<DocElement> { todo!() } } } }
pub trait Defaulted { fn run(&self) { impl Document { pub fn tree_in_default(&mut self) -> &mut Tree<DocElement> { todo!() } } } }
fn hide_macro() { tree_by_macro!(); }
fn hide_in_args() { let _ = vec![{ impl Document { pub fn tree_in_vec(&mut self) -> &mut Tree<DocElement> { todo!() } } 1 }]; }
fn hide_nested_macro() { let _ = format!("{}", tree_by_macro!()); }
fn hide_macro_rules() { macro_rules! debug { () => {}; } }
fn uses_warn() { warn!("shadowed"); }
fn fine_body() { debug!("{}", 1); let _ = vec![1, 2]; assert!(matches!(Some(1), Some(_))); let _: &'static str = ""; }
pub fn typed(doc: &mut Document) -> tree_ty!() { todo!() }
pub static STASH: Mutex<Option<Tree<DocElement>>> = Mutex::new(None);
pub static ELEMENT_STASH: Mutex<Option<NodeData>> = Mutex::new(None);
pub const DEFAULT_NODE: Option<NodeData> = None;
pub const EMPTY: Option<Tree<DocElement>> = None;
pub struct Carrier { pub tree: Tree<DocElement>, pub node: NodeData }
pub type TreeAlias = Tree<DocElement>;
pub fn round_alias(doc: &mut Document, t: TreeAlias) {}
thread_local! {
    pub static TL_STASH: RefCell<Option<Tree<DocElement>>> = const { RefCell::new(None) };
    pub(crate) static TL_HOOK: Cell<Option<fn(&mut Tree<DocElement>)>> = const { Cell::new(None) };
    static TL_FINE: u8 = 0;
    static TL_HIDES: u8 = { impl Document { pub fn tree_in_local(&mut self) -> &mut Tree<DocElement> { todo!() } } 0 };
}
pub struct Edit;
impl Edit { pub fn apply_to(&self, e: &mut DocElement) {} }
pub mod shadow {
    pub trait RowSource { fn each(&self, tree: &mut Tree<DocElement>); }
    pub struct Rows;
    impl Rows { pub fn arena(doc: &mut Document, v: &dyn RowSource) {} }
    pub fn palettes() -> impl std::fmt::Debug { todo!() }
}
pub trait RowSource { fn each(&self, tree: &mut Tree<DocElement>); }
struct Revision;
impl Revision { pub fn tree_via_revision(&mut self) -> &mut Tree<DocElement> { todo!() } }
pub trait Odd { pub fn odd(); }
extern "C" {}
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
                               pub struct Revision;\npub enum ApplyError {}\npub struct SaveError;\n\
                               macro_rules! warn { ($($t:tt)*) => {}; }\n";
        let elements = "pub struct NodeData { pub x: u8 }\npub enum DocKind { Node }\n\
                        pub enum DocElement { Node(NodeData) }\n";
        let access = mutable_access(
            &[("ids.rs", planted, 2), ("element.rs", elements, 2)],
            &[out_of_document],
        );
        let in_mod_rs = "pub(super) fn tree_of(doc: &mut Document) -> &mut Tree<DocElement> { todo!() }\n\
                         pub(super) fn fine(doc: &Document) {}\n";
        let at_the_top = mutable_access(&[("mod.rs", in_mod_rs, 1)], &[]);
        assert!(
            at_the_top.offenders.iter().any(|o| o == "tree_of: a `&mut`")
                && at_the_top.offenders.iter().all(|o| o.starts_with("tree_of: ")),
            "`pub(super)` in `document/mod.rs` reaches the crate: {:?}",
            at_the_top.offenders
        );
        let expected: BTreeSet<&str> = [
            "Document::tree_mut: a `&mut`",
            "Document::me: a `&mut`",
            "Document::each: lends a borrow to the caller's callback",
            "Document::with_tree: lends a borrow to the caller's callback",
            "Document::with_where: lends a borrow to the caller's callback",
            "Document::tree_alias: a `&mut`",
            "Document::counter: a `&mut`",
            "Document::deref_tree: an `impl Trait`",
            "Document::any: a `&mut`",
            "Document::get_mut: a `&mut`",
            "Document::out_guard: a type that can hold a borrow of a `&mut` argument",
            "Document::out_elided: a type that can hold a borrow of a `&mut` argument",
            "Document::foreign: a type that can hold a borrow of a `&mut` argument",
            "Document::boxed: a `dyn Trait`",
            "Document::chosen: a caller-chosen type parameter",
            "Document::byte: a borrow of a `&mut` argument",
            "Document::palettes: an `impl Trait`",
            "Document::engine: a `&mut`",
            "Document::visit_tree: lends a borrow to the caller's callback",
            "Document::visit_generic: lends a borrow to the caller's callback",
            "Document::visit_where: lends a borrow to the caller's callback",
            "Document::with_fn: lends a borrow to the caller's callback",
            "Document::with_guard: lends a borrow to the caller's callback",
            "Document::replace_tree: takes or gives back a document internal by value",
            "Document::take_journal: takes or gives back a document internal by value",
            "Document::put_tree: takes or gives back a document internal by value",
            "Document::tree_ptr: a `*mut`",
            "Document::from_ptr: a `*mut`",
            "Document::OPENER: a `&mut`",
            "Document: tree_by_macro!",
            "Document::as_mut: a `&mut`",
            "Document::deref_mut: a `&mut`",
            "TreeVisitor::visit: hands its implementor a borrow",
            "OPEN: a `&mut`",
            "OPENED: a `&mut`",
            "COUNTER: a `static mut`",
            "RAW: a `*mut`",
            "RawHolder.p: a `*mut`",
            "extern block: an item kind the gate does not judge",
            "Hooks.on_edit: holds a callback that can be handed a borrow",
            "Opens::OPEN: a `&mut`",
            "Opens::Handle: a `&mut`",
            "Opens: tree_in_trait!",
            "Document: an item kind the gate does not judge (an impl member)",
            "Odd: an item kind the gate does not judge (a trait member)",
            "Reach::reach: a `&mut`",
            "u8::reach: a `&mut`",
            "Reach2::reach2: a `&mut`",
            "Lend::lend: a type that can hold a borrow of a `&mut` argument",
            "Document::lend: a `&mut`",
            "Document::Out: a `&mut`",
            "type TreeMut: a `&mut`",
            "Guard.tree: a `&mut`",
            "inner::deep: a `&mut`",
            "_::Document::tree_in_const: a `&mut`",
            "hide::Document::tree_in_fn: a `&mut`",
            "IN_STATIC::Document::tree_in_static: a `&mut`",
            "Host::Document::tree_in_method: a `&mut`",
            "Defaulted::Document::tree_in_default: a `&mut`",
            "hide_macro: tree_by_macro!",
            "hide_in_args: vec! holds an item (`impl`) in its arguments",
            "hide_nested_macro: format! invokes `tree_by_macro!` in its arguments",
            "macro_rules! hide_macro_rules::debug: reuses a standard macro's name",
            "uses_warn: warn!",
            "typed: a macro in type position",
            "STASH: holds a document internal by value",
            "ELEMENT_STASH: holds a document element by value",
            "EMPTY: holds a document internal by value",
            "Carrier.tree: holds a document internal by value",
            "round_alias: takes or gives back a document internal by value",
            "TL_STASH: a `pub` `thread_local!`",
            "TL_STASH: holds a document internal by value",
            "TL_HOOK: a `&mut`",
            "TL_HIDES::Document::tree_in_local: a `&mut`",
            "Document::swap_tree: takes a document internal behind a `&mut`",
            "Document::set_node: takes or gives back a document element by value",
            "Document::take_element: takes or gives back a document element by value",
            "Document::swap_node: takes a document element behind a `&mut`",
            "shadow::RowSource::each: hands its implementor a borrow",
            "shadow::Rows::arena: lends a borrow to the caller's callback",
            "shadow::palettes: an `impl Trait`",
            "RowSource::each: hands its implementor a borrow",
            "Revision::tree_via_revision: a `&mut`",
            "macro_rules! spelled: reuses an allowed name",
            "hidden_items!",
        ]
        .into_iter()
        .collect();
        let found: BTreeSet<&str> = access.offenders.iter().map(String::as_str).collect();
        assert_eq!(
            expected.difference(&found).collect::<Vec<_>>(),
            Vec::<&&str>::new(),
            "planted but not reported for the shape it was planted for"
        );
        let name = |offender: &str| offender.split(": ").next().unwrap_or(offender).to_string();
        let names: BTreeSet<String> = expected.iter().map(|e| name(e)).collect();
        let stray: Vec<&String> = access
            .offenders
            .iter()
            .filter(|offender| !names.contains(&name(offender)))
            .collect();
        assert!(
            stray.is_empty(),
            "reported but not planted as a back door: {stray:?}"
        );
        for kept in [
            "Document::children",
            "Document::apply",
            "Document::read",
            "Document::shared",
            "Document::shared_tree",
            "Document::visit_shared",
            "Document::fmt",
            "Document::read_node",
            "Document::kind_of",
            "Edit::apply_to",
            "named",
            "bytes",
            "element",
        ] {
            assert!(access.read.iter().any(|(_, name)| name == kept), "{kept} is read");
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
                !access.read.iter().any(|(_, name)| name == unread),
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
