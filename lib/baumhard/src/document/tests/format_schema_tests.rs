// SPDX-License-Identifier: MPL-2.0

//! `format/schema.md` held to the code: its tables list the keys of
//! every object the loader reads, in the order the saver writes them,
//! and its example loads. The doc is the contract; these tests only read
//! it, so a key added to a DTO and not to the doc, or listed out of
//! order, goes red here.

use crate::document::format::load;
use crate::util::doc_fixtures::{documented_json_block, format_doc_path};
use crate::util::serde_coverage::{crate_src_root, TypeGraph, TypeKind};
use std::collections::{BTreeMap, BTreeSet};

/// The first cell of a table row: the text between the first two
/// unescaped pipes, with its backticks removed.
fn first_cell(row: &str) -> Option<String> {
    let inner = row.trim().strip_prefix('|')?;
    let mut end = 0;
    let bytes = inner.as_bytes();
    while end < bytes.len() && !(bytes[end] == b'|' && (end == 0 || bytes[end - 1] != b'\\')) {
        end += 1;
    }
    Some(inner[..end].trim().trim_matches('`').to_string())
}

/// Every `###` section's tables, by heading: the first column of each
/// row of a table headed `| Key |`, in order, a second such table in a
/// section appended to the first.
fn documented_keys() -> BTreeMap<String, Vec<String>> {
    let path = format_doc_path("schema.md");
    let text =
        std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{} must be readable: {e}", path.display()));
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut heading: Option<String> = None;
    let mut in_table = false;
    for line in text.lines() {
        if let Some(title) = line.strip_prefix("### ") {
            heading = Some(title.trim().to_string());
            in_table = false;
        } else if line.starts_with("## ") {
            heading = None;
            in_table = false;
        } else if line.starts_with('|') {
            let Some(cell) = first_cell(line) else { continue };
            if cell == "Key" {
                in_table = true;
            } else if in_table && !cell.starts_with("---") {
                if let Some(heading) = &heading {
                    out.entry(heading.clone()).or_default().push(cell);
                }
            }
        } else {
            in_table = false;
        }
    }
    out
}

/// The `###` headings of the doc that hold a table of keys, and the
/// DTOs each describes. A heading with several DTOs describes them all
/// at once: they have the same keys.
const SECTIONS: &[(&str, &[&str])] = &[
    ("Top level", &["MapDto"]),
    ("Theme", &["ThemeDto"]),
    ("Palette group", &["PaletteGroupDto"]),
    ("Definitions", &["EditDto", "PresentationDto", "VerbDto"]),
    ("Defaults", &["DefaultsDto"]),
    ("Node defaults", &["NodeDefaultsDto"]),
    ("Text style", &["TextDefaultsDto"]),
    ("Frame defaults", &["FrameDefaultsDto"]),
    ("Ring defaults", &["RingDefaultsDto"]),
    ("Path defaults", &["FlowDefaultsDto"]),
    ("Label defaults", &["LabelDefaultsDto"]),
    ("Portal-end defaults", &["PortalEndDefaultsDto"]),
    ("Node", &["NodeDto"]),
    ("Triggers", &["OnDto"]),
    ("Zoom", &["ZoomDto"]),
    ("Section", &["SectionDto"]),
    ("Span", &["SpanDto"]),
    ("Frame", &["FrameDto"]),
    ("Ring", &["RingDto"]),
    ("Connector", &["PathDto"]),
    ("Link", &["LinkDto"]),
    ("Cap", &["CapDto"]),
    ("Label", &["LabelDto"]),
    ("Portal end", &["PortalEndDto"]),
];

/// The DTOs whose keys share one table, "Start point"'s second:
/// `radius`, `filler`, `min`, `max`, `edge`, `at`.
const SPELLING_OBJECTS: &[&str] = &["RectangleDto", "FitOnceDto", "FitRangeDto", "StartPointDto"];

/// The JSON keys of `dto`, in declaration order.
fn members(graph: &TypeGraph, dto: &str) -> Vec<String> {
    let info = graph
        .get(dto)
        .unwrap_or_else(|| panic!("{dto} is not in the index"));
    info.fields
        .iter()
        .filter_map(|field| field.member.clone())
        .collect()
}

#[test]
fn test_the_schema_tables_list_every_key_in_the_order_the_saver_writes_them() {
    let graph = TypeGraph::build(&crate_src_root());
    let documented = documented_keys();

    for (heading, dtos) in SECTIONS {
        let listed = documented
            .get(*heading)
            .unwrap_or_else(|| panic!("format/schema.md has no table under `### {heading}`"));
        for dto in *dtos {
            assert_eq!(
                &members(&graph, dto),
                listed,
                "`### {heading}` lists the keys of {dto} in another order, or lacks one"
            );
        }
    }

    // The spelling objects' keys come in one table, in this order.
    let spelled: Vec<String> = SPELLING_OBJECTS
        .iter()
        .flat_map(|dto| members(&graph, dto))
        .collect();
    let listed = documented
        .get("Start point")
        .expect("`### Start point` has a table of keys");
    // `### Start point` holds the named-start table first, which has no
    // `Key` column; the keys table is the one that parsed.
    assert_eq!(&spelled, listed, "the keys of the objects that spell a value");

    // Every struct the loader reads is described somewhere.
    let described: BTreeSet<&str> = SECTIONS
        .iter()
        .flat_map(|(_, dtos)| dtos.iter().copied())
        .chain(SPELLING_OBJECTS.iter().copied())
        .collect();
    let read: BTreeSet<String> = graph
        .reachable_from("MapDto")
        .iter()
        .filter(|info| info.kind == TypeKind::Struct && info.derives_deserialize && info.has_named_fields)
        .map(|info| info.name.clone())
        .collect();
    let missing: Vec<&String> = read
        .iter()
        .filter(|name| !described.contains(name.as_str()))
        .collect();
    assert!(
        missing.is_empty(),
        "these structs are read by the loader and format/schema.md does not describe them \
         (add a table, and a row to SECTIONS): {missing:?}"
    );

    // The link target's two keys are variant names, not fields.
    let target = documented
        .get("Link target")
        .expect("`### Link target` has a table");
    let variants = &graph.get("LinkTargetDto").expect("LinkTargetDto").variants;
    assert_eq!(target, variants);
}

#[test]
fn test_the_schema_example_loads() {
    let path = format_doc_path("schema.md");
    let example = documented_json_block(&path, "## Example", 0);
    let doc =
        load(&example).unwrap_or_else(|error| panic!("the example in format/schema.md must load: {error}"));
    assert_eq!(doc.theme().variables, ["accent", "bg"]);
    assert!(doc.node_by_ident(crate::document::ids::NodeIdent(2)).is_some());
}
