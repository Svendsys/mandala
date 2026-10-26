// SPDX-License-Identifier: MPL-2.0

//! Order in the format: rows are built in key order whatever order the
//! file lists them in, and names get their indices from sorted order,
//! never from where they sit in the file.

use crate::document::element::{DocElement, DocKind};
use crate::document::format::{load, save};
use crate::document::ids::Document;
use crate::document::tests::format_support::{kids, node, same_document};
use crate::document::tests::format_text::{map_with, node_of};
use crate::document::values::{ColorRef, PaletteId, VarId};
use indextree::NodeId;

/// How many siblings the descending rows hold. Enough that a scan past
/// each one before the next is placed would be a million key reads.
const SIBLINGS: usize = 1000;

/// A map whose every row the file lists in descending order of channel:
/// the top-level nodes, one node's sections, its rings and its child
/// nodes, with a link and a frame between them (kinds that do not sort
/// among the channels).
fn descending_map() -> String {
    // `item(i, channel)` for channels SIBLINGS down to 1, joined.
    let descending = |item: &dyn Fn(usize, usize) -> String| {
        (0..SIBLINGS)
            .map(|i| item(i, SIBLINGS - i))
            .collect::<Vec<_>>()
            .join(",")
    };
    let sections =
        descending(&|_, channel| format!(r#"{{"channel":{channel},"spans":[{{"text":"s{channel}"}}]}}"#));
    let rings =
        descending(&|_, channel| format!(r#"{{"channel":{channel},"spans":[{{"text":"r{channel}"}}]}}"#));
    let children = descending(&|i, channel| node_of(5000 + i as u32, &format!(r#","channel":{channel}"#)));
    // The first root, on the highest channel, holds everything above;
    // the rest are bare. Node 2, the second, is where the link leads.
    let roots = descending(&|i, channel| {
        let body = if i == 0 {
            format!(
                r#","channel":{channel},"sections":[{sections}],"frame":{{"rings":[{rings}]}},"links":[{{"to":2,"kind":"line"}}],"children":[{children}]"#
            )
        } else {
            format!(r#","channel":{channel}"#)
        };
        node_of(1 + i as u32, &body)
    });
    map_with("", &roots)
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
    let doc = load(&descending_map()).expect("the descending map loads");

    let rows = assert_every_row_was_appended(&doc);
    assert!(rows > SIBLINGS, "precondition: every row was read ({rows})");

    // The rows are in channel order: the file's descending order was
    // put right, not kept.
    doc.tree().check_sorted().expect("every row is sorted by key");
    let channels = |parent: NodeId, kind: DocKind| -> Vec<usize> {
        kids(&doc, parent, kind)
            .iter()
            .map(|(_, e)| e.channel())
            .collect()
    };
    let ascending: Vec<usize> = (1..=SIBLINGS).collect();
    let roots = channels(doc.root(), DocKind::Node);
    assert_eq!(roots, ascending, "the top-level nodes");
    // The node holding the rest is the one with the highest channel.
    let (one, _) = node(&doc, 1);
    assert_eq!(channels(one, DocKind::Section), ascending, "the sections");
    assert_eq!(channels(one, DocKind::Node), ascending, "the child nodes");
    let (frame, _) = kids(&doc, one, DocKind::Frame)[0];
    assert_eq!(channels(frame, DocKind::Ring), ascending, "the rings");
    // And what is not on a channel stayed where the kinds put it:
    // sections, frame, links, then nodes.
    let order: Vec<DocKind> = doc
        .children(one)
        .filter_map(|c| doc.get(c).map(DocElement::kind))
        .collect();
    let mut by_kind = order.clone();
    by_kind.sort();
    assert_eq!(order, by_kind);
    assert_eq!(order.iter().filter(|k| **k == DocKind::Link).count(), 1);
}

#[test]
fn test_rows_with_equal_channels_keep_the_files_order() {
    // The sort is stable: three sections on one channel stay in file
    // order, and so do sibling nodes, whatever surrounds them.
    let json = map_with(
        "",
        &format!(
            "{},{}",
            node_of(
                1,
                r#","sections":[{"channel":2,"spans":[{"text":"c"}]},{"channel":1,"spans":[{"text":"a"}]},{"channel":1,"spans":[{"text":"b"}]},{"channel":2,"spans":[{"text":"d"}]}]"#
            ),
            node_of(2, "")
        ),
    );
    let doc = load(&json).unwrap();
    let (one, _) = node(&doc, 1);
    let texts: Vec<String> = kids(&doc, one, DocKind::Section)
        .into_iter()
        .map(|(section, _)| match kids(&doc, section, DocKind::Span)[0].1 {
            DocElement::Span(span) => span.text.clone(),
            _ => panic!("a span"),
        })
        .collect();
    assert_eq!(texts, ["a", "b", "c", "d"]);

    // Nodes on one channel: file order.
    let json = map_with(
        "",
        &format!("{},{},{}", node_of(30, ""), node_of(10, ""), node_of(20, "")),
    );
    let doc = load(&json).unwrap();
    let idents: Vec<u32> = kids(&doc, doc.root(), DocKind::Node)
        .into_iter()
        .map(|(_, e)| e.as_node().map_or(0, |n| n.ident.0))
        .collect();
    assert_eq!(idents, [30, 10, 20]);
}

#[test]
fn test_a_descending_file_saves_in_ascending_order() {
    // Not byte-identical to its input, and meant not to be: the saver
    // writes the order the document holds, and that order reloads to
    // the same document.
    let doc = load(&descending_map()).unwrap();
    let saved = save(&doc);
    let again = load(&saved).unwrap();
    same_document(&doc, &again).unwrap();
    assert_eq!(save(&again), saved);
    assert!(saved.find("\"s1\"").unwrap() < saved.find("\"s2\"").unwrap());
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
        assert_eq!(save(&reference), save(&shuffled));
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

    let text = save(&doc);
    assert!(
        text.contains(r#""fill": "var(zed)""#),
        "a variable is written by name:\n{text}"
    );
    assert!(
        text.contains(r#""palette": "first""#),
        "a palette is written by name:\n{text}"
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
