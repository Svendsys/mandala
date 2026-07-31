// SPDX-License-Identifier: MPL-2.0

//! Reachability walk over the crate's own source, so a test can ask
//! "which types can this deserializer actually be handed?" and get an
//! answer that stays true as the model grows.
//!
//! The problem this exists to solve: `serde(deny_unknown_fields)` is
//! the thing that stops a hand-authored `.mindmap.json` key from
//! being dropped on load and destroyed on the next save. It is a
//! per-type opt-in. A hand-maintained list of "types that must carry
//! it" is exactly the sort of twin surface that drifts the moment
//! somebody adds a field — the same failure mode
//! `lib/baumhard/CONVENTIONS.md` §B4 records for the field-tag enums.
//!
//! So nothing is listed. [`TypeGraph::build`] parses the crate with
//! `syn`, indexes every struct / enum / type alias, and
//! [`TypeGraph::reachable_from`] walks the field graph outward from a
//! root type name. Adding a field of a new type extends the covered
//! set automatically; the test that consumes this fails until the new
//! type opts in.
//!
//! Test-only and native-only: it reads `.rs` files off the
//! filesystem, which is also why it is `#[cfg(test)]` — `syn` is a
//! dev-dependency and nothing in a shipped build should carry a
//! parser for its own source.

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
    /// `true` when the item carries `#[serde(deny_unknown_fields)]`.
    pub denies_unknown_fields: bool,
    /// The proxy named by `#[serde(from = "...")]` /
    /// `#[serde(try_from = "...")]`, if any. Such a type never
    /// deserializes its own shape — the proxy does — so the
    /// requirement moves to the proxy.
    pub deserialize_proxy: Option<String>,
    /// `true` when the item carries `#[serde(untagged)]`. Untagged
    /// enums decide a variant by trial deserialization, so denying
    /// unknown fields on them changes which variant matches rather
    /// than merely tightening a check.
    pub untagged: bool,
    /// `true` when some part of the item can consume named JSON
    /// object keys: a struct with named fields, or an enum with at
    /// least one struct variant. Tuple and unit shapes have no field
    /// names for a stray key to hide in.
    pub has_named_fields: bool,
    /// Every type name mentioned by a deserializable field, in
    /// source order and de-duplicated by the walk rather than here.
    pub referenced: Vec<String>,
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

    /// Type names declared more than once across the indexed sources.
    /// The walk resolves by bare name, so a collision would make its
    /// answer ambiguous; callers assert this is empty rather than
    /// silently trusting whichever definition landed last.
    pub fn duplicate_names(&self) -> &BTreeSet<String> {
        &self.duplicates
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
        let mut seen: BTreeSet<&str> = BTreeSet::new();
        let mut queue: VecDeque<&str> = VecDeque::new();
        let mut out: Vec<&TypeInfo> = Vec::new();

        queue.push_back(root);
        seen.insert(root);
        while let Some(name) = queue.pop_front() {
            let Some(info) = self.items.get(name) else { continue };
            out.push(info);
            let successors = info
                .referenced
                .iter()
                .map(String::as_str)
                .chain(info.deserialize_proxy.as_deref());
            for next in successors {
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
                        untagged: serde.untagged,
                        has_named_fields: matches!(item.fields, Fields::Named(_)),
                        referenced: referenced_types(&item.fields),
                    });
                }
                Item::Enum(item) => {
                    let serde = SerdeAttrs::read(&item.attrs);
                    let mut referenced = Vec::new();
                    let mut has_named_fields = false;
                    for variant in &item.variants {
                        has_named_fields |= matches!(variant.fields, Fields::Named(_));
                        referenced.extend(referenced_types(&variant.fields));
                    }
                    self.insert(TypeInfo {
                        name: item.ident.to_string(),
                        file: file.to_path_buf(),
                        kind: TypeKind::Enum,
                        derives_deserialize: derives_deserialize(&item.attrs),
                        denies_unknown_fields: serde.deny_unknown_fields,
                        deserialize_proxy: serde.proxy,
                        untagged: serde.untagged,
                        has_named_fields,
                        referenced,
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
                        untagged: false,
                        has_named_fields: false,
                        referenced,
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
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
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

fn is_test_gated(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        if !attr.path().is_ident("cfg") {
            return false;
        }
        let mut gated = false;
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("test") {
                gated = true;
            }
            Ok(())
        });
        gated
    })
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
    proxy: Option<String>,
}

impl SerdeAttrs {
    fn read(attrs: &[Attribute]) -> Self {
        let mut out = SerdeAttrs::default();
        for attr in attrs {
            if !attr.path().is_ident("serde") {
                continue;
            }
            // Parse errors are ignored on purpose: an option this
            // walk does not model (a nested `bound(...)`, say) must
            // not take the whole file down. The flags it does model
            // are plain idents or `= "literal"` pairs, both handled
            // below.
            let _ = attr.parse_nested_meta(|meta| {
                if meta.path.is_ident("deny_unknown_fields") {
                    out.deny_unknown_fields = true;
                } else if meta.path.is_ident("untagged") {
                    out.untagged = true;
                } else if meta.path.is_ident("from") || meta.path.is_ident("try_from") {
                    let literal: syn::LitStr = meta.value()?.parse()?;
                    out.proxy = Some(last_path_segment(&literal.value()));
                } else if meta.input.peek(syn::Token![=]) {
                    // Consume the value of an option we do not model
                    // so the remaining options in the same attribute
                    // still get seen.
                    let value = meta.value()?;
                    value.parse::<syn::Expr>()?;
                }
                Ok(())
            });
        }
        out
    }
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

fn is_skipped(attrs: &[Attribute]) -> bool {
    let mut skipped = false;
    for attr in attrs {
        if !attr.path().is_ident("serde") {
            continue;
        }
        let _ = attr.parse_nested_meta(|meta| {
            if meta.path.is_ident("skip") || meta.path.is_ident("skip_deserializing") {
                skipped = true;
            } else if meta.input.peek(syn::Token![=]) {
                let value = meta.value()?;
                value.parse::<syn::Expr>()?;
            }
            Ok(())
        });
    }
    skipped
}

/// Every identifier appearing in `ty`, including generic arguments.
/// Module qualifiers come along harmlessly — a name only matters when
/// the index holds a type by that name.
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
            assert!(reached.contains(&expected), "{expected} must be reachable from MindMap");
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
}
