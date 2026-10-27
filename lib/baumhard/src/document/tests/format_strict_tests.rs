// SPDX-License-Identifier: MPL-2.0

//! The strictness of the format, held against the source rather than
//! trusted: every type the loader reads denies unknown keys, none can
//! swallow one, none names an index, and the fixtures use every key.
//!
//! These read the crate's source through
//! [`TypeGraph`](crate::util::serde_coverage::TypeGraph), so they are
//! test-only and native-only, like the graph. The graph indexes types by
//! **bare name**: the format's types carry a `Dto` suffix (or are
//! `Named`, `Objects`, …) so as not to reuse a name another file
//! declares, and `test_type_graph_has_no_ambiguous_names` fails the day
//! one does.

use crate::document::tests::format_support::FIXTURES;
use crate::util::serde_coverage::{crate_src_root, TypeGraph, TypeKind};
use crate::util::test_temp::TempDir;
use serde_json::Value;
use std::collections::BTreeSet;

/// The types the loader reads a file into: the header, every top-level
/// key but `nodes`, and each node of the list, which is parsed on its
/// own from its extent. Everything the loader reads is reachable from
/// one of them.
const ROOTS: [&str; 2] = ["MapHeaderDto", "NodeDto"];

/// What `f` gives for each of [`ROOTS`], gathered.
fn from_roots<T: Ord>(f: impl Fn(&str) -> Vec<T>) -> Vec<T> {
    let mut out: Vec<T> = ROOTS.iter().flat_map(|root| f(root)).collect();
    out.sort();
    out.dedup();
    out
}

/// Every reason a type reachable from `root` could let an unknown key
/// through a load, as `"<type> — <reason>"`: it is a struct (or an enum
/// with struct variants) that does not deny unknown fields; it flattens;
/// it is untagged or internally tagged, which buffers its value; or an
/// attribute on it could not be read, which is unknown, and unknown is
/// not strict.
fn leaks_from(graph: &TypeGraph, root: &str) -> Vec<String> {
    let mut leaks = Vec::new();
    for info in graph.reachable_from(root) {
        // A type that reads its fields through another is judged there.
        if info.deserialize_proxy.is_some() {
            continue;
        }
        let kind = match info.kind {
            TypeKind::Struct => "struct",
            TypeKind::Enum => "enum",
            TypeKind::Alias => continue,
        };
        let mut why = Vec::new();
        if info.derives_deserialize && info.has_named_fields && !info.denies_unknown_fields {
            why.push("does not `deny_unknown_fields`".to_string());
        }
        for field in &info.flattened_fields {
            why.push(format!("flattens `{field}`"));
        }
        if info.untagged {
            why.push("is `untagged`".to_string());
        }
        if info.internally_tagged {
            why.push("is internally tagged".to_string());
        }
        why.extend(info.unread_serde.iter().cloned());
        if !why.is_empty() {
            leaks.push(format!("{kind} {} — {}", info.name, why.join(", ")));
        }
    }
    leaks
}

/// The types the format must never read: the document's index types,
/// and the predicate fields that spell colors, palettes and verbs as
/// indices. A saved index would name another entry the moment a name
/// was added or removed; the file spells names.
const INDEX_FORMS: &[&str] = &[
    "VarId",
    "PaletteId",
    "VerbId",
    "EditId",
    "PresentationId",
    "NodeIdent",
    "ColorRef",
    "PlainColor",
    "AppFont",
    "DocField",
    "Predicate",
];

/// Every index form the types reachable from `root` mention.
fn index_forms_in(graph: &TypeGraph, root: &str) -> Vec<String> {
    let mut found: BTreeSet<String> = graph.unresolved_from(root);
    found.extend(graph.reachable_from(root).iter().map(|info| info.name.clone()));
    found
        .into_iter()
        .filter(|name| INDEX_FORMS.contains(&name.as_str()))
        .collect()
}

#[test]
fn test_every_type_the_format_reads_denies_unknown_fields() {
    let graph = TypeGraph::build(&crate_src_root());
    let reached: Vec<&crate::util::serde_coverage::TypeInfo> = {
        let mut seen = BTreeSet::new();
        ROOTS
            .iter()
            .flat_map(|root| graph.reachable_from(root))
            .filter(|info| seen.insert(info.name.clone()))
            .collect()
    };

    // The walk found the format: every struct of any size, the nested
    // ones among them. A walk that reached a handful would pass the
    // check below for the wrong reason.
    let names: BTreeSet<&str> = reached.iter().map(|info| info.name.as_str()).collect();
    for expected in [
        "MapHeaderDto",
        "ThemeDto",
        "PaletteGroupDto",
        "DefaultsDto",
        "NodeDto",
        "SectionDto",
        "SpanDto",
        "FrameDto",
        "RingDto",
        "PathDto",
        "LinkDto",
        "CapDto",
        "LabelDto",
        "PortalEndDto",
        "ZoomDto",
        "OnDto",
        "VerbDto",
        "RectangleDto",
        "FitOnceDto",
        "FitRangeDto",
        "StartPointDto",
        "LinkTargetDto",
    ] {
        assert!(
            names.contains(expected),
            "the walk from {ROOTS:?} did not reach {expected}"
        );
    }
    let with_fields = reached
        .iter()
        .filter(|info| info.derives_deserialize && info.has_named_fields)
        .count();
    assert!(
        with_fields >= 25,
        "precondition: the walk reached {with_fields} types with named fields, and the format has more"
    );

    let leaks = from_roots(|root| leaks_from(&graph, root));
    assert!(
        leaks.is_empty(),
        "these types are read by the loader and could let an unknown key through, so a \
         `.mandala.json` with a stray key would load:\n  {}\n\n\
         Every struct the loader reads is `#[serde(deny_unknown_fields)]`, and none uses \
         `flatten`, `untagged` or `tag`, which hide a key from it.",
        leaks.join("\n  ")
    );
}

/// A graph over a source planted in a scratch directory.
fn graph_of(source: &str) -> TypeGraph {
    let dir = TempDir::new("format-strict");
    std::fs::write(dir.join("planted.rs"), source).expect("the planted source must be writable");
    TypeGraph::build(dir.path())
}

#[test]
fn test_the_strictness_gate_reports_a_type_that_lets_a_key_through() {
    // The negative control: the gate is only worth anything if it can
    // fail. Each plant leaks in one of the ways the gate names, and the
    // clean control, next to them, is silent.
    let clean = graph_of(
        "#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct Root { pub leaf: Leaf }\n\
         #[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct Leaf { pub x: f64 }\n",
    );
    assert!(
        leaks_from(&clean, "Root").is_empty(),
        "precondition: a strict format passes"
    );
    assert_eq!(
        clean.reachable_from("Root").len(),
        2,
        "precondition: the walk reaches the leaf"
    );

    for (source, reason) in [
        // A nested DTO without the attribute.
        (
            "#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct Root { pub leaf: Leaf }\n\
             #[derive(Deserialize)] pub struct Leaf { pub x: f64 }\n",
            "Leaf — does not `deny_unknown_fields`",
        ),
        // The root itself.
        (
            "#[derive(Deserialize)] pub struct Root { pub x: f64 }\n",
            "Root — does not `deny_unknown_fields`",
        ),
        // Behind a vector and an option.
        (
            "#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct Root { pub leaves: Vec<Option<Leaf>> }\n\
             #[derive(Deserialize)] pub struct Leaf { pub x: f64 }\n",
            "Leaf — does not `deny_unknown_fields`",
        ),
        // An enum with a struct variant.
        (
            "#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct Root { pub payload: Payload }\n\
             #[derive(Deserialize)] pub enum Payload { Tall { height: f64 } }\n",
            "Payload — does not `deny_unknown_fields`",
        ),
        // Flatten swallows the key.
        (
            "#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct Root { #[serde(flatten)] pub rest: Leaf }\n\
             #[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct Leaf { pub x: f64 }\n",
            "Root — flattens `rest`",
        ),
        // Untagged buffers the value and hides the key.
        (
            "#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct Root { pub payload: Payload }\n\
             #[derive(Deserialize)] #[serde(untagged)] pub enum Payload { Leaf(f64) }\n",
            "Payload — is `untagged`",
        ),
    ] {
        let graph = graph_of(source);
        let leaks = leaks_from(&graph, "Root");
        assert!(
            leaks.iter().any(|leak| leak.contains(reason)),
            "the gate missed `{reason}` in:\n{source}\nit reported {leaks:?}"
        );
    }
}

#[test]
fn test_the_format_reads_no_index_and_no_predicate_field() {
    let graph = TypeGraph::build(&crate_src_root());
    let found = from_roots(|root| index_forms_in(&graph, root));
    assert!(
        found.is_empty(),
        "the types the loader reads mention {found:?}. An index is assigned by sorted \
         name, so a saved one would name another entry once a name was added; a file \
         spells names (`var(accent)`, a palette name, a verb id, a family name), and \
         resolves them when it is read or when an edit runs. A predicate saved in an \
         edit is spelled with the format's own value types \
         (`document::format::spelling`), never with `DocField`."
    );

    // What the walk does reach is the format's own spelling, and the
    // primitives under it. The set is pinned so a new unresolved name
    // goes red here rather than slipping past the check above.
    let terminators: BTreeSet<String> = from_roots(|root| graph.unresolved_from(root).into_iter().collect())
        .into_iter()
        .collect();
    let expected: BTreeSet<String> = [
        // `Cow<'a, str>`: a node's text, borrowed from the document when
        // the saver writes it, owned when the loader reads it.
        "bool", "Cow", "f32", "Option", "str", "String", "u16", "u32", "u64", "u8", "usize", "Vec",
    ]
    .into_iter()
    .map(str::to_string)
    .collect();
    let unvetted: Vec<&String> = terminators.difference(&expected).collect();
    assert!(
        unvetted.is_empty(),
        "the walk from {ROOTS:?} ends in names nobody has vetted: {unvetted:?}. Add the \
         name to this list if it is a primitive, or look at what the format now reads."
    );
}

#[test]
fn test_the_index_gate_reports_an_index_the_format_would_read() {
    // Negative control: a DTO holding an index, or a predicate field,
    // is found, whether it is named in a field or reached through one.
    for field in [
        "VarId",
        "PaletteId",
        "VerbId",
        "ColorRef",
        "DocField",
        "Predicate<DocElement>",
        "Vec<VarId>",
        "Option<AppFont>",
    ] {
        let graph = graph_of(&format!(
            "#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct Root {{ pub value: {field} }}\n"
        ));
        let found = index_forms_in(&graph, "Root");
        assert!(!found.is_empty(), "the gate missed {field}");
    }
    // And a clean one is silent.
    let graph = graph_of(
        "#[derive(Deserialize)] #[serde(deny_unknown_fields)] pub struct Root { pub value: String }\n",
    );
    assert!(index_forms_in(&graph, "Root").is_empty());
}

/// Every object key in `value`, at any depth.
fn keys_of(value: &Value, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, inner) in map {
                out.insert(key.clone());
                keys_of(inner, out);
            }
        }
        Value::Array(items) => items.iter().for_each(|item| keys_of(item, out)),
        _ => {}
    }
}

#[test]
fn test_the_fixtures_use_every_key_the_format_defines() {
    let graph = TypeGraph::build(&crate_src_root());
    let mut defined: BTreeSet<String> = BTreeSet::new();
    for root in ROOTS {
        for (_, members) in graph.member_names_from(root) {
            defined.extend(members);
        }
    }
    // `nodes` is the one top-level key no header field holds: the list
    // is read element by element, never as a field.
    defined.insert("nodes".to_string());
    // The keys of a type that reads its own shape (an enum tag spelled
    // by a visitor) are not fields to the graph; they are listed by
    // the enum or here.
    defined.extend(["rectangle", "once", "fit"].map(str::to_string));
    for info in ROOTS.iter().flat_map(|root| graph.reachable_from(root)) {
        if info.name == "LinkTargetDto" {
            defined.extend(info.variants.iter().cloned());
        }
    }
    assert!(
        defined.len() >= 60,
        "precondition: the format defines {} keys, and the walk found {}",
        defined.len(),
        defined.len()
    );
    for key in [
        "id",
        "parent",
        "radius",
        "filler",
        "edge",
        "url",
        "verb",
        "border_t",
        "control_points",
    ] {
        assert!(defined.contains(key), "precondition: the walk knows `{key}`");
    }

    let mut used: BTreeSet<String> = BTreeSet::new();
    for (name, text) in FIXTURES {
        let value: Value = serde_json::from_str(text).unwrap_or_else(|e| panic!("{name} is JSON: {e}"));
        keys_of(&value, &mut used);
    }
    let unused: Vec<&String> = defined.difference(&used).collect();
    assert!(
        unused.is_empty(),
        "the fixture corpus never uses these keys, so a loader that ignored or misread \
         one would pass every round trip: {unused:?}"
    );
}
