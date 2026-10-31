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
use std::collections::BTreeSet;
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

/// The document's internals beside its element data: the types a `&mut`
/// handed out of `document` would let someone change it through.
const INTERNALS: &[&str] = &[
    "Document",
    "DocElement",
    "Tree",
    "Arena",
    "Node",
    "Definitions",
    "IdentIndex",
    "IncomingIndex",
    "Derived",
    "Budget",
    "Writer",
    "History",
    "Journal",
    "JournalBuilder",
];

/// The file that may hand out `&mut` into a document, and why: the test
/// builder, which exists to make and edit documents for tests and is
/// held to the tests and the benches by
/// `test_only_the_document_names_its_test_builder`.
const MAY_EDIT: &[&str] = &["tests/test_support.rs"];

/// Every type the gate counts as the document's: [`INTERNALS`] and every
/// struct and enum `element.rs` declares.
fn document_types() -> BTreeSet<String> {
    let path = repo_path("lib/baumhard/src/document/element.rs");
    let source =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
    let file = syn::parse_file(&source).expect("element.rs parses");
    let mut types: BTreeSet<String> = INTERNALS.iter().map(ToString::to_string).collect();
    for item in &file.items {
        match item {
            Item::Struct(item) => {
                types.insert(item.ident.to_string());
            }
            Item::Enum(item) => {
                types.insert(item.ident.to_string());
            }
            _ => {}
        }
    }
    types
}

/// What [`mutable_access`] found in one source: every function visible
/// outside `document` that it read, and those of them whose signature
/// hands out `&mut` into a document — `Owner::name` for a method.
#[derive(Debug, Default, PartialEq)]
struct Access {
    read: Vec<String>,
    offenders: Vec<String>,
}

/// Whether `vis` reaches beyond `document`: `pub`, or `pub(crate)` /
/// `pub(in crate)`, which reach the rest of baumhard.
fn visible(vis: &Visibility) -> bool {
    match vis {
        Visibility::Public(_) => true,
        Visibility::Restricted(restricted) => restricted.path.is_ident("crate"),
        Visibility::Inherited => false,
    }
}

/// Finds a `&mut` to one of `types` in a type; inside an `impl` of one
/// of them, `Self` is one.
struct MutRef<'t> {
    types: &'t BTreeSet<String>,
    self_is_one: bool,
    found: bool,
}

impl<'ast> Visit<'ast> for MutRef<'_> {
    fn visit_type_reference(&mut self, reference: &'ast TypeReference) {
        if reference.mutability.is_some() {
            let mut names = Names {
                types: self.types,
                self_is_one: self.self_is_one,
                found: false,
            };
            names.visit_type(&reference.elem);
            self.found |= names.found;
        }
        visit::visit_type_reference(self, reference);
    }
}

/// Finds a name of one of `types` anywhere in a type.
struct Names<'t> {
    types: &'t BTreeSet<String>,
    self_is_one: bool,
    found: bool,
}

impl<'ast> Visit<'ast> for Names<'_> {
    fn visit_path_segment(&mut self, segment: &'ast PathSegment) {
        let name = segment.ident.to_string();
        self.found |= self.types.contains(&name) || (self.self_is_one && name == "Self");
        visit::visit_path_segment(self, segment);
    }
}

/// Whether `sig` returns, or takes in a parameter other than its
/// receiver, a `&mut` to one of `types`.
fn hands_out(sig: &Signature, types: &BTreeSet<String>, self_is_one: bool) -> bool {
    let mut finder = MutRef {
        types,
        self_is_one,
        found: false,
    };
    for input in &sig.inputs {
        if let FnArg::Typed(typed) = input {
            finder.visit_type(&typed.ty);
        }
    }
    if let ReturnType::Type(_, ty) = &sig.output {
        finder.visit_type(ty);
    }
    finder.found
}

/// [`mutable_access`] over `items`, nested modules included.
fn walk_items(items: &[Item], types: &BTreeSet<String>, access: &mut Access) {
    for item in items {
        if is_test_gated(item_attrs(item)) {
            continue;
        }
        match item {
            Item::Fn(item) if visible(&item.vis) => {
                let name = item.sig.ident.to_string();
                if hands_out(&item.sig, types, false) {
                    access.offenders.push(name.clone());
                }
                access.read.push(name);
            }
            Item::Impl(item) if item.trait_.is_none() => {
                let owner = match &*item.self_ty {
                    Type::Path(path) => path
                        .path
                        .segments
                        .last()
                        .map_or(String::new(), |segment| segment.ident.to_string()),
                    _ => String::new(),
                };
                let self_is_one = types.contains(&owner);
                for member in &item.items {
                    let ImplItem::Fn(method) = member else {
                        continue;
                    };
                    if !visible(&method.vis) || is_test_gated(&method.attrs) {
                        continue;
                    }
                    let name = format!("{owner}::{}", method.sig.ident);
                    if hands_out(&method.sig, types, self_is_one) {
                        access.offenders.push(name.clone());
                    }
                    access.read.push(name);
                }
            }
            Item::Mod(module) => {
                if let Some((_, items)) = &module.content {
                    walk_items(items, types, access);
                }
            }
            _ => {}
        }
    }
}

/// Every function in `source` visible beyond `document` ([`visible`])
/// whose signature holds a `&mut` to one of `types`: returned, or taken
/// by a parameter — a closure's parameter included — other than the
/// method's own receiver. Inside an `impl` of one of `types`, `Self` is
/// one of them. A trait's methods, test-gated items and private ones are
/// not read.
fn mutable_access(source: &str, types: &BTreeSet<String>) -> Access {
    let file = syn::parse_file(source).expect("the document's sources parse");
    let mut access = Access::default();
    walk_items(&file.items, types, &mut access);
    access
}

#[cfg(test)]
mod tests {
    use super::*;

    /// **No `pub` item under `document/` hands out `&mut` into a
    /// document** (design §8.4, #238 §7): no function the rest of the
    /// crate or the app can call returns, or lends to a closure, a `&mut`
    /// to the tree, an element, the document or its internals. The
    /// writer's own methods take `&mut self` and give nothing back.
    /// Precondition: the scan reads the writer's API.
    #[test]
    fn test_no_document_item_hands_out_mutable_access() {
        let types = document_types();
        assert!(
            types.contains("NodeData") && types.contains("Document"),
            "the gate must know the element data and the internals"
        );
        let root = repo_path("lib/baumhard/src/document");
        let files = rust_files(&root);
        let gated = crate::util::source_scan::test_gated_module_files(&files);
        let mut read = Vec::new();
        let mut offenders = Vec::new();
        for file in &files {
            let relative = file
                .strip_prefix(&root)
                .expect("under document/")
                .to_string_lossy()
                .to_string();
            if gated.contains(file) || MAY_EDIT.contains(&relative.as_str()) {
                continue;
            }
            let source = std::fs::read_to_string(file)
                .unwrap_or_else(|e| panic!("{} must be readable: {e}", file.display()));
            let access = mutable_access(&source, &types);
            read.extend(access.read);
            offenders.extend(
                access
                    .offenders
                    .into_iter()
                    .map(|name| format!("{relative}: {name}")),
            );
        }
        for witness in ["Document::apply", "Document::stage", "Document::tree"] {
            assert!(
                read.iter().any(|name| name == witness),
                "the scan must read {witness}"
            );
        }
        assert!(
            offenders.is_empty(),
            "these hand out `&mut` into a document; change it through `Document::apply`:\n{}",
            offenders.join("\n")
        );
    }

    /// The negative control of the gate above: each way a function can
    /// hand out `&mut` is reported, and each shape that does not is not.
    #[test]
    fn test_the_mutable_access_scan_reports_a_planted_back_door() {
        let types = document_types();
        let planted = r#"
impl Document {
    pub fn tree_mut(&mut self) -> &mut Tree<DocElement> { todo!() }
    pub fn me(&mut self) -> Option<&mut Self> { todo!() }
    pub(crate) fn each(&mut self, f: impl FnMut(&mut NodeData)) {}
    pub fn apply(&mut self, n: u32) -> Result<u32, u8> { todo!() }
    fn private(&mut self) -> &mut DocElement { todo!() }
    pub(super) fn near(&mut self) -> &mut DocElement { todo!() }
    #[cfg(test)]
    pub fn tested(&mut self) -> &mut DocElement { todo!() }
}
pub fn element(doc: &mut Document) {}
pub fn bytes(out: &mut Vec<u8>, doc: &Document) {}
impl Mutation<DocElement> for Op {
    fn apply_to(&self, e: &mut DocElement) {}
}
impl Builder {
    pub fn edit(&mut self) -> &mut Self { todo!() }
}
mod inner {
    pub fn deep() -> &'static mut SpanData { todo!() }
}
"#;
        let access = mutable_access(planted, &types);
        assert_eq!(
            access.offenders,
            vec!["Document::tree_mut", "Document::me", "Document::each", "element", "deep"],
            "a returned reference, `Self` in its own impl, a closure's parameter, a parameter, a nested module"
        );
        assert_eq!(
            access.read,
            vec![
                "Document::tree_mut",
                "Document::me",
                "Document::each",
                "Document::apply",
                "element",
                "bytes",
                "Builder::edit",
                "deep",
            ],
            "private, `pub(super)`, test-gated and trait methods are not read"
        );
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
