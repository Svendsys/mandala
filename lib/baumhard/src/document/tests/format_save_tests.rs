// SPDX-License-Identifier: MPL-2.0

//! The saver: the order it writes keys in, what it leaves out, what it
//! refuses to write, and the numbers.

use crate::document::element::{
    ConnectorData, DocElement, FlowStyle, FrameData, LabelData, LabelStyle, LinkData, PathData, RingData,
    RingStyle, SectionData, SpanData, TextStyle, ThemeData, ThemeVariant,
};
use crate::document::format::{load, save};
use crate::document::tests::format_support::{fixture, node, same_document};
use crate::document::tests::format_text::map_of;
use crate::document::tests::test_support::{example, node_data, path_data, DocumentBuilder};
use crate::document::values::{ColorRef, FontSize, LinkKind, Rgba, VarId};
use crate::font::fonts::AppFont;
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
    let node = [
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
        "children",
    ];
    let start = text.find("\n      \"id\": 1").expect("the first node");
    assert_eq!(key_positions(&text[start..], 6, &node).len(), node.len());
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
    // The same map with its keys shuffled loads and saves to the
    // fixture's text.
    let value: Value = serde_json::from_str(fixture("full")).unwrap();
    let reversed = serde_json::to_string(&value).unwrap(); // serde_json's maps are sorted
    let doc = load(&reversed).unwrap();
    assert_eq!(save(&doc), fixture("full"));
    // And compact or spaced, the same.
    let spaced = serde_json::to_string_pretty(&value).unwrap().replace("  ", "\t");
    assert_eq!(save(&load(&spaced).unwrap()), fixture("full"));
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
    let value: Value = serde_json::from_str(&save(&builder.build().unwrap())).unwrap();

    // Not written: defaults, empty arrays, empty objects, derived data.
    let a = ["nodes", "0"];
    for key in [
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
    let b = ["nodes", "0", "children", "0"];
    assert!(
        has(&value, &b, "connector"),
        "b's connector has spans, caps and a label"
    );
    let c = ["nodes", "0", "children", "0", "children", "0"];
    assert!(
        !has(&value, &c, "connector"),
        "a child node's connector with every field inherited and nothing under it is not written"
    );
    for key in [
        "channel", "children", "sections", "links", "frame", "palette", "font", "size", "color",
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
        "parent",
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
    let text = save(&builder.build().unwrap());
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
    let text = save(&builder.build().unwrap());
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
fn test_the_saver_writes_nothing_the_loader_would_read_as_something_else() {
    // A document can hold, through the builder or a mutation that does
    // not check, values the file has no spelling for. Writing any of
    // them would give a file that loads as another map, so they are
    // left out; each case below would be silently retargeted, or would
    // not load, if it were written.
    let mut builder = DocumentBuilder::new();
    builder.theme(ThemeData {
        variables: vec!["x".to_string()],
        variants: vec![ThemeVariant {
            name: "d".to_string(),
            values: vec![Rgba::opaque(1, 2, 3)],
        }],
        active: 0,
    });
    let root = builder.root();
    let mut data = node_data(1);
    // The fallback face has no name; points on a node are fine, a
    // fraction on one is not; a variable the theme does not define.
    data.style.text.font = Some(AppFont::Any);
    data.style.text.size = Some(FontSize::Rel(0.5));
    data.style.text.color = Some(ColorRef::Var(VarId(9)));
    data.style.fill = Some(ColorRef::Var(VarId(0)));
    let a = builder.node_with(root, data);
    // Points on a ring (never read), a fraction on a section.
    let frame = builder.add(a, DocElement::Frame(FrameData::default()));
    let ring = builder.add(
        frame,
        DocElement::Ring(RingData {
            channel: 0,
            style: RingStyle {
                fit: None,
                text: TextStyle {
                    size: Some(FontSize::Pt(24.0)),
                    ..TextStyle::default()
                },
            },
        }),
    );
    // A span in the ring that has a size of its own.
    builder.add(
        ring,
        DocElement::Span(SpanData {
            text: "r".to_string(),
            style: TextStyle {
                size: Some(FontSize::Pt(12.0)),
                ..TextStyle::default()
            },
            link: None,
        }),
    );
    builder.add(
        a,
        DocElement::Section(SectionData {
            text: TextStyle {
                size: Some(FontSize::Rel(0.2)),
                ..TextStyle::default()
            },
            ..SectionData::default()
        }),
    );
    let doc = builder.build().unwrap();
    let text = save(&doc);
    let value: Value = serde_json::from_str(&text).unwrap();
    let node = &value["nodes"][0];
    assert!(
        node.get("font").is_none(),
        "the fallback face has no family name:\n{text}"
    );
    assert!(node.get("size").is_none(), "a fraction is not points:\n{text}");
    assert!(
        node.get("color").is_none(),
        "a variable the theme lacks is not written:\n{text}"
    );
    assert_eq!(node["fill"], "var(x)", "a variable that is defined is");
    assert!(
        node["frame"]["rings"][0].get("size").is_none(),
        "points on a ring:\n{text}"
    );
    assert!(
        node["frame"]["rings"][0]["spans"][0].get("size").is_none(),
        "a span in a ring has no size:\n{text}"
    );
    assert!(
        node["sections"][0].get("size").is_none(),
        "a fraction on a section:\n{text}"
    );
    // And what it did write loads.
    load(&text).unwrap_or_else(|error| panic!("{error}\n{text}"));
}

#[test]
fn test_a_palette_color_naming_a_missing_variable_is_written_so_that_it_does_not_load() {
    use crate::document::element::{PaletteData, PaletteGroup};
    use crate::document::values::PlainColor;
    let mut builder = DocumentBuilder::new();
    builder.theme(ThemeData {
        variables: Vec::new(),
        variants: vec![ThemeVariant {
            name: "d".to_string(),
            values: Vec::new(),
        }],
        active: 0,
    });
    builder.palette(PaletteData {
        name: "p".to_string(),
        groups: vec![PaletteGroup {
            fill: PlainColor::Var(VarId(3)),
            frame: PlainColor::Rgba(Rgba::opaque(0, 0, 0)),
            text: PlainColor::Rgba(Rgba::opaque(0, 0, 0)),
            title: None,
        }],
    });
    let text = save(&builder.build().unwrap());
    // A palette group has all its colors, so one that cannot be named
    // is written as a variable no theme defines: a refusal at load that
    // names it, never a color that was not asked for.
    let error = load(&text).unwrap_err();
    assert_eq!(error.path, "palettes.p[0].fill");
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
    assert_eq!(save(&doc), save(&doc));
    let back = load(&save(&doc)).unwrap();
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
    let back = load(&save(&doc)).unwrap();
    same_document(&doc, &back).unwrap();
}
