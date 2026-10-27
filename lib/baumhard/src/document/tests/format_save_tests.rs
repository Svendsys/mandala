// SPDX-License-Identifier: MPL-2.0

//! The saver: the order it writes keys in, what it leaves out, what it
//! refuses to write, and the numbers.

use crate::document::element::{
    ConnectorData, DocElement, FlowStyle, LabelData, LabelStyle, LinkData, PathData, ThemeData, ThemeVariant,
};
use crate::document::format::load;
use crate::document::tests::format_support::{fixture, node, same_document, written, FIXTURES};
use crate::document::tests::format_text::map_of;
use crate::document::tests::test_support::{example, path_data, DocumentBuilder};
use crate::document::values::{LinkKind, Rgba};
use serde_json::Value;

/// The position of each `"key":` at `indent` spaces in `text`, after
/// `from`, in the order asked; each must be found after the one before.
fn key_positions(text: &str, indent: usize, keys: &[&str]) -> Vec<usize> {
    let mut at = 0;
    keys.iter()
        .map(|key| {
            let needle = format!("\n{}\"{key}\":", " ".repeat(indent));
            let found = text[at..]
                .find(&needle)
                .unwrap_or_else(|| panic!("`{key}` not found at indent {indent} after offset {at}"));
            at += found + needle.len();
            at
        })
        .collect()
}

#[test]
fn test_the_saver_writes_keys_in_the_order_the_schema_lists_them() {
    let text = fixture("full");
    // `key_positions` insists on each key being found after the last,
    // so finding them all is the assertion.
    let top = [
        "format",
        "name",
        "theme",
        "palettes",
        "defaults",
        "edits",
        "presentations",
        "verbs",
        "nodes",
    ];
    assert_eq!(key_positions(text, 2, &top).len(), top.len());
    // A node is one line, its keys in schema order: the first node, a
    // top-level one, then a nested one for `parent` and `channel`.
    let in_line = |line: &str, keys: &[&str]| {
        let mut at = 0;
        for key in keys {
            let needle = format!("\"{key}\": ");
            let found = line[at..]
                .find(&needle)
                .unwrap_or_else(|| panic!("`{key}` not found after offset {at} in {line}"));
            at += found + needle.len();
        }
    };
    let line = |id: u32| {
        text.lines()
            .find(|line| line.starts_with(&format!("    {{\"id\": {id},")))
            .unwrap_or_else(|| panic!("node {id} has a line of its own"))
    };
    in_line(
        line(1),
        &[
            "id",
            "position",
            "min_size",
            "shape",
            "fill",
            "fill_fade",
            "palette",
            "zoom",
            "folded",
            "opacity",
            "on",
            "font",
            "size",
            "color",
            "weight",
            "italic",
            "sections",
            "frame",
            "links",
        ],
    );
    assert!(line(4).starts_with(r#"    {"id": 4, "parent": 2, "channel": 3, "position": "#));
    assert!(line(2).contains(r#""sections": "#) && line(2).contains(r#""connector": "#));
    in_line(
        line(2),
        &[
            "id",
            "parent",
            "position",
            "shape",
            "hidden",
            "sections",
            "connector",
        ],
    );
    let defaults = [
        "node",
        "section",
        "frame",
        "ring",
        "connector",
        "link",
        "label",
        "portal_end",
        "span",
    ];
    assert_eq!(key_positions(text, 4, &defaults).len(), defaults.len());
}

#[test]
fn test_the_saver_writes_the_same_text_whatever_order_the_file_was_in() {
    // Every fixture with every object's keys sorted, as `jq -S` would
    // write it, loads to the same document and saves to the fixture's
    // exact bytes: the order of keys never matters.
    for (name, text) in FIXTURES {
        let value: Value = serde_json::from_str(text).unwrap();
        let sorted = serde_json::to_string(&value).unwrap(); // serde_json's maps are sorted
        assert_ne!(sorted, *text, "{name}: precondition, the text changed");
        let doc = load(&sorted).unwrap_or_else(|e| panic!("{name}, keys sorted: {e}"));
        assert_eq!(written(&doc), *text, "{name}");
        same_document(&doc, &load(text).unwrap()).unwrap();
        // And compact or spaced, the same.
        let spaced = serde_json::to_string_pretty(&value).unwrap().replace("  ", "\t");
        assert_eq!(written(&load(&spaced).unwrap()), *text, "{name}");
    }
    // Key order never matters; the order of a row does. A map with its
    // node list reversed is refused, not reordered.
    let mut rows: Value = serde_json::from_str(fixture("full")).unwrap();
    if let Some(nodes) = rows["nodes"].as_array_mut() {
        nodes.reverse();
    }
    assert!(load(&serde_json::to_string(&rows).unwrap()).is_err());
}

/// Whether a JSON object at the end of `path` in `value` has `key`.
fn has(value: &Value, path: &[&str], key: &str) -> bool {
    let mut at = value;
    for step in path {
        at = match step.parse::<usize>() {
            Ok(index) => &at[index],
            Err(_) => &at[*step],
        };
    }
    at.get(key).is_some()
}

#[test]
fn test_the_saver_leaves_out_what_the_file_would_say_by_default() {
    let (mut builder, _) = example();
    builder.theme(ThemeData {
        variables: Vec::new(),
        variants: vec![ThemeVariant {
            name: "d".to_string(),
            values: Vec::new(),
        }],
        active: 0,
    });
    // Nothing of the example's is set but node `a`'s palette, and that
    // is the point: every other field is at its default.
    let value: Value = serde_json::from_str(&written(&builder.build().unwrap())).unwrap();

    // Not written: defaults, empty arrays, empty objects, derived data.
    let a = ["nodes", "0"];
    for key in [
        "parent",
        "channel",
        "folded",
        "hidden",
        "opacity",
        "min_size",
        "zoom",
        "on",
        "connector",
        "shape",
        "fill",
    ] {
        assert!(!has(&value, &a, key), "node a has no `{key}` to write");
    }
    let b = ["nodes", "1"];
    assert!(
        has(&value, &b, "connector"),
        "b's connector has spans, caps and a label"
    );
    let c = ["nodes", "2"];
    assert!(
        !has(&value, &c, "connector"),
        "a child node's connector with every field inherited and nothing under it is not written"
    );
    for key in [
        "channel", "sections", "links", "frame", "palette", "font", "size", "color",
    ] {
        assert!(!has(&value, &c, key), "node c has no `{key}` to write");
    }
    assert!(!has(&value, &[], "edits") && !has(&value, &[], "verbs") && !has(&value, &[], "defaults"));
    assert!(!has(&value, &[], "presentations"));
    // The portal link's two default ends are not written; the line link's
    // untouched path is just its content.
    let links = &value["nodes"][0]["links"];
    assert_eq!(links[1]["kind"], "portal");
    assert!(
        links[1].get("ends").is_some(),
        "this portal's source end has a marker and a label"
    );
    // A section with nothing set but its spans writes only the spans.
    assert_eq!(
        value["nodes"][0]["sections"][0]
            .as_object()
            .map(|o| o.keys().cloned().collect::<Vec<_>>()),
        Some(vec!["spans".to_string()])
    );
    // Nothing derived or structural: only keys the schema defines.
    let mut keys = std::collections::BTreeSet::new();
    fn collect(value: &Value, out: &mut std::collections::BTreeSet<String>) {
        match value {
            Value::Object(map) => map.iter().for_each(|(k, v)| {
                out.insert(k.clone());
                collect(v, out);
            }),
            Value::Array(items) => items.iter().for_each(|v| collect(v, out)),
            _ => {}
        }
    }
    collect(&value, &mut keys);
    for derived in [
        "depth",
        "children",
        "parent_id",
        "fitted_size",
        "width",
        "height",
        "resolved",
    ] {
        assert!(
            !keys.contains(derived),
            "`{derived}` is derived and never written"
        );
    }
}

#[test]
fn test_a_portal_link_with_default_ends_is_written_with_none() {
    // The example's portal has a marker and a label on its source end,
    // so its ends are written.
    let (mut builder, _) = example();
    builder.theme(ThemeData {
        variables: Vec::new(),
        variants: vec![ThemeVariant {
            name: "d".to_string(),
            values: Vec::new(),
        }],
        active: 0,
    });
    let text = written(&builder.build().unwrap());
    assert!(
        text.contains(r#""ends""#),
        "the example's source end has a marker"
    );

    // A fresh portal made by the builder has two default ends.
    let mut builder = DocumentBuilder::new();
    builder.theme(ThemeData {
        variables: Vec::new(),
        variants: vec![ThemeVariant {
            name: "d".to_string(),
            values: Vec::new(),
        }],
        active: 0,
    });
    let root = builder.root();
    let a = builder.node(root);
    let b = builder.node(root);
    builder.link(a, b, LinkKind::Portal);
    let text = written(&builder.build().unwrap());
    assert!(text.contains(r#""kind": "portal""#));
    assert!(
        !text.contains(r#""ends""#),
        "default ends are not written:\n{text}"
    );
    // Loaded back, the two ends are there.
    let back = load(&text).unwrap();
    let (a, _) = node(&back, 1);
    let link =
        crate::document::tests::format_support::kid(&back, a, crate::document::element::DocKind::Link, 0).0;
    assert_eq!(
        crate::document::tests::format_support::kids(
            &back,
            link,
            crate::document::element::DocKind::PortalEnd
        )
        .len(),
        2
    );
}

#[test]
fn test_a_float_survives_the_text_exactly() {
    // The saver writes serde_json's shortest form of the stored f32, and
    // the loader reads it back to the same bits: an unchanged map saves
    // byte-identically only if no number drifts.
    let mut state = 0x9E37_79B9_7F4A_7C15_u64;
    let mut next = move || {
        state ^= state << 13;
        state ^= state >> 7;
        state ^= state << 17;
        state
    };
    let mut checked = 0;
    // Random bit patterns, and the neighbors of the interesting values.
    let mut values: Vec<f32> = (0..200_000)
        .map(|_| f32::from_bits((next() >> 32) as u32))
        .filter(|v| v.is_finite() && *v >= 0.0)
        .collect();
    for base in [
        0.1_f32,
        0.2,
        0.3,
        1.0,
        1.5,
        24.0,
        33.3,
        0.05,
        1.0e-7,
        16_777_216.0,
        4_096.0,
    ] {
        let bits = base.to_bits();
        values.extend((bits.saturating_sub(64)..bits + 64).map(f32::from_bits));
    }
    for value in values {
        let json = map_of(&format!(
            r#","fill_fade":{}"#,
            serde_json::to_string(&value).unwrap()
        ));
        let doc = load(&json).unwrap_or_else(|e| panic!("{value:e}: {e}"));
        let (_, data) = node(&doc, 1);
        assert_eq!(
            data.style.fill_fade.map(f32::to_bits),
            Some(value.to_bits()),
            "{value:e} did not survive the text {json}"
        );
        checked += 1;
    }
    assert!(checked > 100_000, "precondition: {checked} values were checked");
}

#[test]
fn test_a_document_of_every_kind_saves_the_same_text_twice() {
    let (mut builder, _) = example();
    builder.theme(ThemeData {
        variables: vec!["a".to_string()],
        variants: vec![ThemeVariant {
            name: "d".to_string(),
            values: vec![Rgba::opaque(0, 0, 0)],
        }],
        active: 0,
    });
    let doc = builder.build().unwrap();
    assert_eq!(written(&doc), written(&doc));
    let back = load(&written(&doc)).unwrap();
    same_document(&doc, &back).unwrap();
}

#[test]
fn test_a_connector_the_file_writes_loads_back_to_its_fields() {
    // A connector with a field, a link with a label, defaults for
    // each: the builder's data, saved and loaded, is the builder's.
    let mut builder = DocumentBuilder::new();
    builder.theme(ThemeData {
        variables: Vec::new(),
        variants: vec![ThemeVariant {
            name: "d".to_string(),
            values: Vec::new(),
        }],
        active: 0,
    });
    let root = builder.root();
    let a = builder.node(root);
    let b = builder.node(a);
    let connector = crate::document::tests::test_support::connector_of(&builder, b);
    builder.set(
        connector,
        DocElement::Connector(ConnectorData {
            path: PathData {
                flow: FlowStyle {
                    content: Some("=".to_string()),
                    ..FlowStyle::default()
                },
                visible: false,
                ..path_data()
            },
        }),
    );
    let link = builder.link(a, b, LinkKind::Line);
    builder.set(
        link,
        DocElement::Link(LinkData {
            to: b,
            kind: LinkKind::Line,
            path: PathData {
                visible: false,
                ..path_data()
            },
        }),
    );
    let label = builder.add(
        link,
        DocElement::Label(LabelData {
            style: LabelStyle::default(),
        }),
    );
    builder.span(label, "l");
    let doc = builder.build().unwrap();
    let back = load(&written(&doc)).unwrap();
    same_document(&doc, &back).unwrap();
}
