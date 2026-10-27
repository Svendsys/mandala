// SPDX-License-Identifier: MPL-2.0

//! Order in the format: a row is listed in the order the document
//! keeps it — ascending channel, equal channels in the order written —
//! and a row listed in any other order is refused, never sorted (design
//! B.4); so every row is built by appending. Names get their indices
//! from sorted order, never from where they sit in the file.

use crate::document::element::{DocElement, DocKind};
use crate::document::format::{load, LoadErrorKind};
use crate::document::ids::Document;
use crate::document::tests::format_support::written;
use crate::document::tests::format_support::{kids, node, same_document};
use crate::document::tests::format_text::{map_with, node_of, rejection};
use crate::document::values::{ColorRef, PaletteId, VarId};
use indextree::NodeId;

/// How many siblings the rows of [`ascending_map`] hold. Enough that a
/// scan past each one before the next is placed would be a million key
/// reads.
const SIBLINGS: usize = 1000;

/// A map whose every row is long and lists its channels from 1 up: the
/// top-level nodes, one node's sections, its rings and its child nodes,
/// with a link and a frame between them (kinds that do not sort among
/// the channels).
fn ascending_map() -> String {
    // `item(i, channel)` for channels 1 up to SIBLINGS, joined.
    let ascending = |item: &dyn Fn(usize, usize) -> String| {
        (0..SIBLINGS)
            .map(|i| item(i, i + 1))
            .collect::<Vec<_>>()
            .join(",")
    };
    let sections =
        ascending(&|_, channel| format!(r#"{{"channel":{channel},"spans":[{{"text":"s{channel}"}}]}}"#));
    let rings =
        ascending(&|_, channel| format!(r#"{{"channel":{channel},"spans":[{{"text":"r{channel}"}}]}}"#));
    let children =
        ascending(&|i, channel| node_of(5000 + i as u32, &format!(r#","parent":1,"channel":{channel}"#)));
    // The first root, on the lowest channel, holds everything above and
    // is followed by its children; the rest are bare. Node 2 is where
    // the link leads.
    let first = node_of(
        1,
        &format!(
            r#","channel":1,"sections":[{sections}],"frame":{{"rings":[{rings}]}},"links":[{{"to":2,"kind":"line"}}]"#
        ),
    );
    let rest: Vec<String> = (1..SIBLINGS)
        .map(|i| node_of(1 + i as u32, &format!(r#","channel":{}"#, i + 1)))
        .collect();
    map_with("", &format!("{first},{children},{}", rest.join(",")))
}

/// Every row of `doc` is in ascending arena order: each child was
/// appended after the sibling before it, which is the only way to
/// reach a sorted row without scanning. The arena hands out ids in
/// creation order and a load removes nothing, so a child inserted
/// ahead of a sibling already in its row has the *larger* id of the
/// two.
fn assert_every_row_was_appended(doc: &Document) -> usize {
    let tree = doc.tree();
    let mut rows = 0;
    for parent in tree.descendants() {
        let row: Vec<NodeId> = parent.children(tree.arena()).collect();
        rows += 1;
        for pair in row.windows(2) {
            assert!(
                pair[0] < pair[1],
                "a child of {parent} was inserted ahead of a sibling already in its row \
                 ({} before {}): the row was not added in key order, so the insert scanned",
                pair[1],
                pair[0]
            );
        }
    }
    rows
}

#[test]
fn test_every_row_of_a_loaded_map_is_built_by_appending() {
    let doc = load(&ascending_map()).expect("the ascending map loads");

    let rows = assert_every_row_was_appended(&doc);
    assert!(rows > SIBLINGS, "precondition: every row was read ({rows})");
    doc.tree().check_sorted().expect("every row is sorted by key");
    let channels = |parent: NodeId, kind: DocKind| -> Vec<usize> {
        kids(&doc, parent, kind)
            .iter()
            .map(|(_, e)| e.channel())
            .collect()
    };
    let ascending: Vec<usize> = (1..=SIBLINGS).collect();
    assert_eq!(
        channels(doc.root(), DocKind::Node),
        ascending,
        "the top-level nodes"
    );
    let (one, _) = node(&doc, 1);
    assert_eq!(channels(one, DocKind::Section), ascending, "the sections");
    assert_eq!(channels(one, DocKind::Node), ascending, "the child nodes");
    let (frame, _) = kids(&doc, one, DocKind::Frame)[0];
    assert_eq!(channels(frame, DocKind::Ring), ascending, "the rings");
    // And what is not on a channel stayed where the kinds put it:
    // sections, frame, connector, links, then nodes.
    let order: Vec<DocKind> = doc
        .children(one)
        .filter_map(|c| doc.get(c).map(DocElement::kind))
        .collect();
    let mut by_kind = order.clone();
    by_kind.sort();
    assert_eq!(order, by_kind);
    assert_eq!(order.iter().filter(|k| **k == DocKind::Link).count(), 1);
}

/// A fan: node 1 and `count` children of it, with `channel(i)` for the
/// `i`th.
fn fan(count: usize, channel: impl Fn(usize) -> usize) -> String {
    let children: Vec<String> = (0..count)
        .map(|i| node_of(2 + i as u32, &format!(r#","parent":1,"channel":{}"#, channel(i))))
        .collect();
    map_with("", &format!("{},{}", node_of(1, ""), children.join(",")))
}

#[test]
fn test_a_wide_fan_in_channel_order_loads_by_appending_and_a_descending_one_is_refused() {
    // 100,000 siblings in ascending order: every one an append.
    const WIDE: usize = 100_000;
    let doc = load(&fan(WIDE, |i| i)).expect("an ascending fan loads");
    let (one, _) = node(&doc, 1);
    assert_eq!(kids(&doc, one, DocKind::Node).len(), WIDE);
    assert_every_row_was_appended(&doc);
    // The same fan descending is refused at its second child, the first
    // sibling below the one before it, before anything else is read: it
    // is never loaded slowly.
    let error = rejection(&fan(WIDE, |i| WIDE - i));
    assert_eq!(error.path, "nodes[2].channel");
    assert_eq!(
        error.kind,
        LoadErrorKind::SiblingsOutOfOrder {
            previous_channel: WIDE as u64,
            previous_id: Some(2),
        }
    );
    assert_eq!(error.node.as_ref().and_then(|node| node.id), Some(3));
}

#[test]
fn test_a_row_listed_out_of_channel_order_is_refused() {
    // Sibling nodes: node 3 is listed after node 2's subtree, on a lower
    // channel than node 2.
    let json = map_with(
        "",
        &[
            node_of(1, ""),
            node_of(2, r#","parent":1,"channel":5"#),
            node_of(4, r#","parent":2"#),
            node_of(3, r#","parent":1,"channel":4"#),
        ]
        .join(","),
    );
    let error = rejection(&json);
    assert_eq!(
        (error.path.as_str(), error.kind),
        (
            "nodes[3].channel",
            LoadErrorKind::SiblingsOutOfOrder {
                previous_channel: 5,
                previous_id: Some(2),
            }
        )
    );
    // Top-level nodes, the same way.
    let error = rejection(&map_with(
        "",
        &format!("{},{}", node_of(1, r#","channel":1"#), node_of(2, "")),
    ));
    assert_eq!(
        (error.path.as_str(), error.kind),
        (
            "nodes[1].channel",
            LoadErrorKind::SiblingsOutOfOrder {
                previous_channel: 1,
                previous_id: Some(1),
            }
        )
    );
    // Sections and rings, which have no id.
    let section = |channel: u32| format!(r#"{{"channel":{channel},"spans":[{{"text":"x"}}]}}"#);
    let error = rejection(&map_with(
        "",
        &node_of(
            1,
            &format!(r#","sections":[{},{},{}]"#, section(0), section(2), section(1)),
        ),
    ));
    assert_eq!(
        (error.path.as_str(), error.kind),
        (
            "nodes[0].sections[2].channel",
            LoadErrorKind::SiblingsOutOfOrder {
                previous_channel: 2,
                previous_id: None,
            }
        )
    );
    let error = rejection(&map_with(
        "",
        &node_of(
            1,
            &format!(r#","frame":{{"rings":[{},{}]}}"#, section(3), section(0)),
        ),
    ));
    assert_eq!(
        (error.path.as_str(), error.kind),
        (
            "nodes[0].frame.rings[1].channel",
            LoadErrorKind::SiblingsOutOfOrder {
                previous_channel: 3,
                previous_id: None,
            }
        )
    );
    // The controls: each row ascending, or with equal channels in either
    // order, loads.
    assert!(load(&map_with(
        "",
        &[
            node_of(1, ""),
            node_of(2, r#","parent":1,"channel":4"#),
            node_of(4, r#","parent":2"#),
            node_of(3, r#","parent":1,"channel":4"#),
        ]
        .join(",")
    ))
    .is_ok());
    assert!(load(&map_with(
        "",
        &format!(
            "{},{}",
            node_of(1, r#","channel":1"#),
            node_of(2, r#","channel":1"#)
        )
    ))
    .is_ok());
    assert!(load(&map_with(
        "",
        &node_of(
            1,
            &format!(r#","sections":[{},{},{}]"#, section(0), section(1), section(1))
        )
    ))
    .is_ok());
    assert!(load(&map_with(
        "",
        &node_of(
            1,
            &format!(r#","frame":{{"rings":[{},{}]}}"#, section(0), section(3))
        )
    ))
    .is_ok());
}

#[test]
fn test_rows_with_equal_channels_keep_the_files_order() {
    // Three sections on one channel stay in file order, and so do
    // sibling nodes, whatever surrounds them, through a save and a load.
    let json = map_with(
        "",
        &format!(
            "{},{}",
            node_of(
                1,
                r#","sections":[{"channel":1,"spans":[{"text":"b"}]},{"channel":1,"spans":[{"text":"a"}]},{"channel":2,"spans":[{"text":"d"}]},{"channel":2,"spans":[{"text":"c"}]}]"#
            ),
            node_of(2, "")
        ),
    );
    let doc = load(&json).unwrap();
    let texts = |doc: &Document| -> Vec<String> {
        let (one, _) = node(doc, 1);
        kids(doc, one, DocKind::Section)
            .into_iter()
            .map(|(section, _)| match kids(doc, section, DocKind::Span)[0].1 {
                DocElement::Span(span) => span.text.clone(),
                _ => panic!("a span"),
            })
            .collect()
    };
    assert_eq!(texts(&doc), ["b", "a", "d", "c"]);
    let again = load(&written(&doc)).unwrap();
    assert_eq!(texts(&again), ["b", "a", "d", "c"]);

    // Nodes on one channel: file order, and again after a round trip.
    let json = map_with(
        "",
        &format!("{},{},{}", node_of(30, ""), node_of(10, ""), node_of(20, "")),
    );
    let doc = load(&json).unwrap();
    let idents = |doc: &Document| -> Vec<u32> {
        kids(doc, doc.root(), DocKind::Node)
            .into_iter()
            .map(|(_, e)| e.as_node().map_or(0, |n| n.ident.0))
            .collect()
    };
    assert_eq!(idents(&doc), [30, 10, 20]);
    let again = load(&written(&doc)).unwrap();
    same_document(&doc, &again).unwrap();
    assert_eq!(idents(&again), [30, 10, 20]);
}

#[test]
fn test_indices_follow_sorted_names_not_the_order_of_the_file() {
    // The same theme and palettes, written with their keys in three
    // different orders, are the same document: VarIds and PaletteIds
    // come from ascending names.
    let theme = |variants: &[(&str, &[(&str, &str)])]| {
        let body: Vec<String> = variants
            .iter()
            .map(|(name, vars)| {
                let vars: Vec<String> = vars.iter().map(|(v, c)| format!(r#""{v}":"{c}""#)).collect();
                format!(r#""{name}":{{{}}}"#, vars.join(","))
            })
            .collect();
        format!(r#""theme":{{"active":"dark","variants":{{{}}}}}"#, body.join(","))
    };
    let dark: &[(&str, &str)] = &[("accent", "#010101"), ("bg", "#020202"), ("fg", "#030303")];
    let light: &[(&str, &str)] = &[("accent", "#040404"), ("bg", "#050505"), ("fg", "#060606")];
    fn rotate<'a>(items: &[(&'a str, &'a str)], by: usize) -> Vec<(&'a str, &'a str)> {
        let mut v = items.to_vec();
        v.rotate_left(by);
        v
    }
    let palettes = |order: &[&str]| {
        let body: Vec<String> = order
            .iter()
            .map(|name| format!(r##""{name}":[{{"fill":"#000000","frame":"#000000","text":"#000000"}}]"##))
            .collect();
        format!(r#","palettes":{{{}}}"#, body.join(","))
    };
    let text = |variants: String, palette_order: &[&str]| {
        format!(
            r#"{{"format":1,"name":"t",{variants}{},"nodes":[{}]}}"#,
            palettes(palette_order),
            node_of(1, r#","fill":"var(fg)","palette":"zeta""#)
        )
    };
    let reference = load(&text(
        theme(&[("dark", dark), ("light", light)]),
        &["alpha", "beta", "zeta"],
    ))
    .unwrap();
    for (variants, order) in [
        (
            theme(&[("light", light), ("dark", dark)]),
            ["zeta", "beta", "alpha"],
        ),
        (
            theme(&[("light", &rotate(light, 1)), ("dark", &rotate(dark, 2))]),
            ["beta", "zeta", "alpha"],
        ),
    ] {
        let shuffled = load(&text(variants, &order)).unwrap();
        same_document(&reference, &shuffled).expect("key order in the file changes nothing");
        assert_eq!(written(&reference), written(&shuffled));
    }
    // And the indices are what the names sort to.
    assert_eq!(reference.theme().variables, ["accent", "bg", "fg"]);
    assert_eq!(reference.theme().variants[0].name, "dark");
    let (_, data) = node(&reference, 1);
    assert_eq!(
        data.style.fill,
        Some(ColorRef::Var(VarId(2))),
        "fg is the third name"
    );
    assert_eq!(data.palette, Some(PaletteId(2)), "zeta is the third name");
}

#[test]
fn test_an_index_never_reaches_the_file() {
    // A document whose theme and palettes are in an order the file's
    // ascending order disagrees with: the index in memory differs from
    // the position in the file, and the file must still say the name.
    use crate::document::element::{PaletteData, ThemeData, ThemeVariant};
    use crate::document::tests::test_support::{node_data, DocumentBuilder};
    use crate::document::values::Rgba;

    let mut builder = DocumentBuilder::new();
    builder.theme(ThemeData {
        variables: vec!["zed".to_string(), "alpha".to_string()],
        variants: vec![ThemeVariant {
            name: "only".to_string(),
            values: vec![Rgba::opaque(1, 1, 1), Rgba::opaque(2, 2, 2)],
        }],
        active: 0,
    });
    let mut palette = |name: &str| {
        builder.palette(PaletteData {
            name: name.to_string(),
            groups: Vec::new(),
        })
    };
    palette("second");
    let first_in_file = palette("first");
    let root = builder.root();
    let mut data = node_data(1);
    data.palette = Some(first_in_file);
    data.style.fill = Some(ColorRef::Var(VarId(0)));
    builder.node_with(root, data);
    let doc = builder.build().unwrap();

    let text = written(&doc);
    assert!(
        text.contains(r#""fill": "var(zed)""#),
        "a variable is written by name:\n{text}"
    );
    assert!(
        text.contains(r#""palette": "first""#),
        "a palette is written by name:\n{text}"
    );
    // Name-keyed objects are written in ascending byte order of the name,
    // whatever order the document holds them in.
    let before = |first: &str, second: &str| text.find(first).unwrap() < text.find(second).unwrap();
    assert!(
        before(r#""alpha":"#, r#""zed":"#),
        "variables are written by ascending name:\n{text}"
    );
    assert!(
        before(r#""first":"#, r#""second":"#),
        "palettes are written by ascending name:\n{text}"
    );
    // Reloaded, the names are the same and the indices are the file's.
    let back = load(&text).unwrap();
    let (_, data) = node(&back, 1);
    assert_eq!(back.theme().variables, ["alpha", "zed"]);
    assert_eq!(
        data.style.fill,
        Some(ColorRef::Var(VarId(1))),
        "zed is the second name now"
    );
    assert_eq!(data.palette, Some(PaletteId(0)), "first sorts before second");
    assert_eq!(
        back.theme().variants[0].values[1],
        Rgba::opaque(1, 1, 1),
        "zed's value"
    );
}
