// SPDX-License-Identifier: MPL-2.0

//! One test per rule the loader enforces, each asserting the JSON path
//! the error names (design §16: every strict-loader error names its
//! path). A load is refused at the first thing wrong, with nothing
//! else; none of these maps is a document.

use crate::document::bounds::{self, BoundError};
use crate::document::format::{load, load_under, Limits, LoadErrorKind};
use crate::document::tests::format_text::{map_of, map_of_two, map_with, node_of, rejection, THEME};

/// The load of `json` fails with this path and this kind.
fn rejects(json: &str, path: &str, kind: LoadErrorKind) {
    let error = rejection(json);
    assert_eq!(
        (error.path.as_str(), &error.kind),
        (path, &kind),
        "the error for {json}"
    );
}

/// The load of `json` fails at `path` with a message from the reader:
/// a wrong type or a spelling the schema does not allow.
fn rejects_as_invalid(json: &str, path: &str) {
    let error = rejection(json);
    assert!(
        matches!(error.kind, LoadErrorKind::Invalid { .. }),
        "expected an invalid value for {json}, got {error:?}"
    );
    assert_eq!(error.path, path, "the path for {json}");
}

/// The load of `json` fails at `path`, the reader's message being all
/// there is to say of why. For the shapes serde_json reports as a
/// syntax error from inside a value: a third element in a pair, a
/// second key in an enum.
fn rejects_at(json: &str, path: &str) {
    let error = rejection(json);
    assert!(
        matches!(
            error.kind,
            LoadErrorKind::Invalid { .. } | LoadErrorKind::Syntax { .. }
        ),
        "expected the reader's message for {json}, got {error:?}"
    );
    assert_eq!(error.path, path, "the path for {json}");
}

/// The load of `json` fails at `path` with this bound error.
fn rejects_bound(json: &str, path: &str, error: BoundError) {
    rejects(json, path, LoadErrorKind::Bound { error });
}

// ---- the text as a whole ------------------------------------------

#[test]
fn test_a_map_over_the_size_ceiling_is_refused_before_it_is_read() {
    // `MAX_MAP_BYTES` is `None` on every target the suite runs on, so
    // the refusal the browser depends on is driven with a ceiling of
    // its own: the same code, a number a test can afford.
    let json = map_of("");
    let size = json.len() as u64;
    let error = load_under(
        &json,
        Limits {
            bytes: Some(size - 1),
            elements: None,
        },
    )
    .unwrap_err();
    assert_eq!(error.path, "");
    assert_eq!(
        error.kind,
        LoadErrorKind::TooLarge {
            bytes: size,
            max: size - 1
        }
    );
    assert!(
        load_under(
            &json,
            Limits {
                bytes: Some(size),
                elements: None
            }
        )
        .is_ok(),
        "a text exactly at the ceiling is inside it"
    );
    assert!(
        load_under(
            &json,
            Limits {
                bytes: None,
                elements: None
            }
        )
        .is_ok(),
        "no ceiling means no refusal"
    );
    // Refused before parsing: a text that is not JSON is too large, not
    // a syntax error.
    let garbage = "x".repeat(64);
    assert!(matches!(
        load_under(
            &garbage,
            Limits {
                bytes: Some(32),
                elements: None
            }
        )
        .unwrap_err()
        .kind,
        LoadErrorKind::TooLarge { .. }
    ));
}

#[test]
fn test_the_native_ceiling_is_none_and_the_browser_is_bounded() {
    #[cfg(not(target_arch = "wasm32"))]
    assert_eq!(crate::document::format::MAX_MAP_BYTES, None);
    #[cfg(target_arch = "wasm32")]
    assert_eq!(
        crate::document::format::MAX_MAP_BYTES,
        Some(crate::document::format::WASM32_MAX_MAP_BYTES)
    );
    #[cfg(not(target_arch = "wasm32"))]
    assert_eq!(crate::document::format::MAX_ELEMENTS, None);
    #[cfg(target_arch = "wasm32")]
    assert_eq!(
        crate::document::format::MAX_ELEMENTS,
        Some(crate::document::format::WASM32_MAX_ELEMENTS)
    );
    // The derived values, written out: half of 3 GiB over 7 bytes per
    // byte of text, and over 576 bytes per element.
    assert_eq!(crate::document::format::WASM32_MAX_MAP_BYTES, 230_087_533);
    assert_eq!(crate::document::format::WASM32_MAX_ELEMENTS, 2_796_202);
}

#[test]
fn test_text_that_is_not_json_is_a_syntax_error() {
    for text in ["", "{", "not json", r#"{"format":1,}"#, r#"{"format" 1}"#] {
        let error = rejection(text);
        assert_eq!(error.path, "", "{text:?}");
        assert!(
            matches!(error.kind, LoadErrorKind::Syntax { .. }),
            "{text:?}: {error:?}"
        );
    }
}

#[test]
fn test_text_after_the_map_is_a_syntax_error() {
    let json = format!("{} {{}}", map_of(""));
    let error = rejection(&json);
    assert!(matches!(error.kind, LoadErrorKind::Syntax { .. }), "{error:?}");
}

#[test]
fn test_a_number_the_reader_cannot_hold_is_refused() {
    // JSON has no NaN or infinity, but a number past `f64` is one in
    // disguise; the reader refuses it.
    let error = rejection(&map_of(r#","fill_fade":1e999"#));
    assert!(matches!(error.kind, LoadErrorKind::Syntax { .. }), "{error:?}");
}

#[test]
fn test_nesting_past_the_readers_depth_is_refused_not_a_crash() {
    // JSON nesting inside one node's content, far past anything the
    // schema allows there: refused at the node, by the first value the
    // schema has no room for, and never by exhausting a stack. The index
    // skips nested values iteratively, and a node is read from its own
    // extent, so the reader never descends further than the schema does.
    let deep = format!(r#","sections":{}{}"#, "[".repeat(200), "]".repeat(200));
    let error = rejection(&map_of(&deep));
    assert!(error.path.starts_with("nodes[0].sections"), "{error:?}");
    assert_eq!(error.node.as_ref().and_then(|node| node.id), Some(1), "{error:?}");
    // The same inside a top-level value names that value.
    let error = rejection(&map_with(
        &format!(r#","defaults":{}1{}"#, r#"{"node":"#.repeat(150), "}".repeat(150)),
        "",
    ));
    assert!(error.path.starts_with("defaults.node"), "{error:?}");
    // A syntax error deep inside such a nesting is still a syntax error,
    // found by the index before anything is read, at the node.
    let broken = format!(r#","sections":{}{}"#, "[".repeat(200), "]".repeat(199));
    let error = rejection(&map_of(&broken));
    assert!(matches!(error.kind, LoadErrorKind::Syntax { .. }), "{error:?}");
    assert_eq!(error.path, "nodes[0]", "{error:?}");
    // A tree's depth is not JSON nesting: a chain of 200 nodes, far past
    // what the reader would follow nested, is a flat list and loads.
    let chain: Vec<String> = (1..=200)
        .map(|id| match id {
            1 => node_of(1, ""),
            _ => node_of(id, &format!(r#","parent":{}"#, id - 1)),
        })
        .collect();
    let doc = load(&map_with("", &chain.join(","))).unwrap();
    let (leaf, _) = crate::document::tests::format_support::node(&doc, 200);
    assert_eq!(doc.depth(leaf), 199);
}

// ---- keys ---------------------------------------------------------

#[test]
fn test_an_unknown_key_at_the_top_level_is_refused() {
    let json = format!(r#"{{"format":1,"name":"t",{THEME},"bogus":1}}"#);
    rejects(
        &json,
        "bogus",
        LoadErrorKind::UnknownKey {
            key: "bogus".to_string(),
        },
    );
}

#[test]
fn test_a_name_the_author_chose_is_quoted_in_a_path_unless_it_is_plain() {
    // Names an author picks (palettes, theme variants and variables)
    // can hold a dot or a bracket or nothing at all; unquoted they would
    // read as more path, or none. The notation is the same whether the
    // reader (a parse error) or the builder (a reference that names
    // nothing) found the fault.
    let color = |name: &str, value: &str| {
        format!(
            r##","palettes":{{{}:[{{"fill":"{value}","frame":"#000000","text":"#000000"}}]}}"##,
            serde_json::to_string(name).unwrap()
        )
    };
    let unknown_variable = |name: &str| LoadErrorKind::UnknownVariable {
        name: name.to_string(),
    };
    // Found by the builder.
    rejects(
        &map_with(&color("a.b", "var(nope)"), ""),
        r#"palettes["a.b"][0].fill"#,
        unknown_variable("nope"),
    );
    // An empty name is quoted too, and is no name at all (design B.8).
    rejects(
        &map_with(&color("", "var(nope)"), ""),
        r#"palettes[""]"#,
        LoadErrorKind::EmptyName,
    );
    rejects(
        &map_with(&color("say \"hi\"", "var(nope)"), ""),
        r#"palettes["say \"hi\""][0].fill"#,
        unknown_variable("nope"),
    );
    rejects(
        &map_with(&color("a[0]", "var(nope)"), ""),
        r#"palettes["a[0]"][0].fill"#,
        unknown_variable("nope"),
    );
    // A plain name stays a plain key: no brackets, no noise.
    rejects(
        &map_with(&color("_p9", "var(nope)"), ""),
        "palettes._p9[0].fill",
        unknown_variable("nope"),
    );
    // A name that starts with a digit is not an identifier.
    rejects(
        &map_with(&color("9p", "var(nope)"), ""),
        r#"palettes["9p"][0].fill"#,
        unknown_variable("nope"),
    );
    rejects(
        r##"{"format":1,"name":"t","theme":{"active":"a","variants":{"a":{"x":"#000000"},"x y":{"y":"#000000"}}}}"##,
        r#"theme.variants["x y"]"#,
        LoadErrorKind::VariantsDiffer {
            variable: "x".to_string(),
            missing: true,
        },
    );
    // An empty variant name is refused, quoted, before the variants are
    // compared.
    rejects(
        r##"{"format":1,"name":"t","theme":{"active":"","variants":{"":{"x":"#000000"},"a":{"y":"#000000"}}}}"##,
        r#"theme.variants[""]"#,
        LoadErrorKind::EmptyName,
    );
    // Found by the reader.
    rejects_as_invalid(
        r##"{"format":1,"name":"t","palettes":{"a.b":[{"fill":"#gggggg","frame":"#000000","text":"#000000"}]},"nodes":[]}"##,
        r#"palettes["a.b"][0].fill"#,
    );
    rejects_as_invalid(
        r##"{"format":1,"name":"t","theme":{"active":"a","variants":{"a":{"x y":"#gggggg"}}}}"##,
        r#"theme.variants.a["x y"]"#,
    );
    // An unknown key is named in the path the same way, whatever it holds.
    let unknown = |key: &str| LoadErrorKind::UnknownKey { key: key.to_string() };
    rejects(&map_of(r#","":1"#), r#"nodes[0][""]"#, unknown(""));
    rejects(&map_of(r#","a.b":1"#), r#"nodes[0]["a.b"]"#, unknown("a.b"));
    rejects(&map_of(r#","bogus":1"#), "nodes[0].bogus", unknown("bogus"));
}

#[test]
fn test_an_unknown_key_in_a_nested_object_is_refused() {
    let key = |key: &str| LoadErrorKind::UnknownKey { key: key.to_string() };
    rejects(&map_of(r#","bogus":1"#), "nodes[0].bogus", key("bogus"));
    rejects(
        &map_of(r#","sections":[{"spans":[{"text":"a","bogus":1}]}]"#),
        "nodes[0].sections[0].spans[0].bogus",
        key("bogus"),
    );
    rejects(
        &map_of(r#","frame":{"rings":[{"bogus":true}]}"#),
        "nodes[0].frame.rings[0].bogus",
        key("bogus"),
    );
    rejects(
        &map_with(r#","defaults":{"label":{"bogus":0}}"#, ""),
        "defaults.label.bogus",
        key("bogus"),
    );
    rejects(
        &map_of(r#","shape":{"rectangle":{"radius":5,"bogus":0}}"#),
        "nodes[0].shape.rectangle.bogus",
        key("bogus"),
    );
    rejects(
        &map_of(r#","frame":{"fit":{"fit":{"min":1,"max":2,"bogus":0}}}"#),
        "nodes[0].frame.fit.fit.bogus",
        key("bogus"),
    );
    rejects(
        &map_of(r#","frame":{"start":{"edge":"top","at":0,"bogus":0}}"#),
        "nodes[0].frame.start.bogus",
        key("bogus"),
    );
}

#[test]
fn test_a_missing_required_key_is_refused() {
    let missing = |key: &str| LoadErrorKind::MissingKey { key: key.to_string() };
    rejects(r#"{"name":"t"}"#, "", missing("format"));
    // The theme is the one top-level key that may be absent: no theme
    // is the empty theme, which has no other spelling.
    assert!(load(r#"{"format":1,"name":"t"}"#).is_ok());
    rejects(&format!(r#"{{"format":1,{THEME}}}"#), "", missing("name"));
    rejects(&map_with("", r#"{"position":[0,0]}"#), "nodes[0]", missing("id"));
    rejects(&map_with("", r#"{"id":1}"#), "nodes[0]", missing("position"));
    rejects(
        &map_of(r#","sections":[{"spans":[{}]}]"#),
        "nodes[0].sections[0].spans[0]",
        missing("text"),
    );
    rejects(
        &map_of(r#","links":[{"kind":"line"}]"#),
        "nodes[0].links[0]",
        missing("to"),
    );
    rejects(
        &map_of(r#","links":[{"to":1}]"#),
        "nodes[0].links[0]",
        missing("kind"),
    );
    rejects(
        &map_of(r#","frame":{"fit":{"once":{}}}"#),
        "nodes[0].frame.fit.once",
        missing("filler"),
    );
    rejects(
        &map_with(r##","palettes":{"p":[{"fill":"#000000"}]}"##, ""),
        "palettes.p[0]",
        missing("frame"),
    );
    rejects(
        r#"{"format":1,"name":"t","theme":{"variants":{"d":{}}}}"#,
        "theme",
        missing("active"),
    );
}

#[test]
fn test_a_key_written_twice_is_refused() {
    let json = format!(r#"{{"format":1,"name":"t",{THEME},"name":"u"}}"#);
    let error = rejection(&json);
    assert_eq!(
        error.kind,
        LoadErrorKind::DuplicateKey {
            key: "name".to_string()
        }
    );
    // A name-keyed object too: two variants, two variables, two palettes.
    rejects(
        r##"{"format":1,"name":"t","theme":{"active":"d","variants":{"d":{},"d":{}}}}"##,
        "theme.variants.d",
        LoadErrorKind::DuplicateKey { key: "d".to_string() },
    );
    rejects(
        r##"{"format":1,"name":"t","theme":{"active":"d","variants":{"d":{"a":"#000000","a":"#111111"}}}}"##,
        "theme.variants.d.a",
        LoadErrorKind::DuplicateKey { key: "a".to_string() },
    );
    rejects(
        &map_with(r#","palettes":{"p":[],"p":[]}"#, ""),
        "palettes.p",
        LoadErrorKind::DuplicateKey { key: "p".to_string() },
    );
}

// ---- types and spellings ------------------------------------------

#[test]
fn test_a_value_of_the_wrong_type_is_refused() {
    rejects_as_invalid(&map_of(r#","sections":3"#), "nodes[0].sections");
    rejects_as_invalid(
        &format!(r#"{{"format":1,"name":"t",{THEME},"nodes":{{}}}}"#),
        "nodes",
    );
    rejects_as_invalid(&format!(r#"{{"format":1,"name":7,{THEME}}}"#), "name");
    rejects_as_invalid(&format!(r#"{{"format":"1","name":"t",{THEME}}}"#), "format");
    rejects_as_invalid(&map_with("", r#"{"id":"1","position":[0,0]}"#), "nodes[0].id");
    rejects_as_invalid(&map_with("", r#"{"id":1.5,"position":[0,0]}"#), "nodes[0].id");
    rejects_as_invalid(&map_with("", r#"{"id":-1,"position":[0,0]}"#), "nodes[0].id");
    rejects_as_invalid(&map_of(r#","folded":"yes""#), "nodes[0].folded");
    rejects_as_invalid(&map_of(r#","opacity":"half""#), "nodes[0].opacity");
    rejects_as_invalid(&map_of(r#","channel":-1"#), "nodes[0].channel");
    rejects_as_invalid(&map_of(r#","channel":1.5"#), "nodes[0].channel");
    rejects_as_invalid(
        &map_of(r#","sections":[{"spans":[{"text":5}]}]"#),
        "nodes[0].sections[0].spans[0].text",
    );
    rejects_as_invalid(
        &map_with("", r#"{"id":1,"position":[0,"0"]}"#),
        "nodes[0].position[1]",
    );
    rejects_at(
        &map_with("", r#"{"id":1,"position":[0,0,0]}"#),
        "nodes[0].position",
    );
    rejects_as_invalid(&map_with("", r#"{"id":1,"position":[0]}"#), "nodes[0].position");
    rejects_as_invalid(&map_of(r#","italic":1"#), "nodes[0].italic");
}

#[test]
fn test_null_is_not_a_way_to_write_an_absent_key() {
    for body in [
        r#","fill":null"#,
        r#","min_size":null"#,
        r#","font":null"#,
        r#","frame":null"#,
        r#","connector":null"#,
        r#","palette":null"#,
        r#","on":null"#,
    ] {
        let error = rejection(&map_of(body));
        assert!(
            matches!(error.kind, LoadErrorKind::Invalid { .. }),
            "{body}: {error:?}"
        );
        assert!(error.path.starts_with("nodes[0]."), "{body}: {error:?}");
    }
    rejects_as_invalid(&map_with(r#","defaults":null"#, ""), "defaults");
    // An array that is `null` is not an empty array either.
    rejects_as_invalid(&map_of(r#","sections":null"#), "nodes[0].sections");
}

#[test]
fn test_an_object_written_as_an_array_of_its_values_is_refused() {
    // serde would read a struct from a positional array; the format has
    // one way to write an object.
    rejects_as_invalid("[1,\"t\"]", "");
    rejects_as_invalid(
        &format!(r#"{{"format":1,"name":"t",{THEME},"nodes":[[1,0]]}}"#),
        "nodes[0]",
    );
    rejects_as_invalid(&map_of(r#","sections":[[]]"#), "nodes[0].sections[0]");
    rejects_as_invalid(&map_of(r#","frame":[]"#), "nodes[0].frame");
    rejects_as_invalid(&map_of(r#","zoom":[1,2]"#), "nodes[0].zoom");
    rejects_as_invalid(
        &map_of(r#","shape":{"rectangle":[5]}"#),
        "nodes[0].shape.rectangle",
    );
    rejects_as_invalid(
        &map_of(r#","frame":{"fit":{"fit":[1,2]}}"#),
        "nodes[0].frame.fit.fit",
    );
    rejects_as_invalid(&map_of(r#","frame":{"start":["top",0]}"#), "nodes[0].frame.start");
    rejects_as_invalid(r#"{"format":1,"name":"t","theme":[],"nodes":[]}"#, "theme");
}

#[test]
fn test_a_name_written_as_an_object_is_refused() {
    // serde would read a unit variant from `{"left": null}` as well.
    rejects_as_invalid(
        &map_of(r#","sections":[{"align":{"left":null}}]"#),
        "nodes[0].sections[0].align",
    );
    rejects_as_invalid(
        &map_of(r#","links":[{"to":1,"kind":{"line":null}}]"#),
        "nodes[0].links[0].kind",
    );
    rejects_as_invalid(
        &map_of(r#","frame":{"direction":{"clockwise":null}}"#),
        "nodes[0].frame.direction",
    );
    rejects_as_invalid(
        &map_of(r#","frame":{"start":{"edge":{"top":null},"at":0}}"#),
        "nodes[0].frame.start.edge",
    );
    rejects_as_invalid(&map_of(r#","shape":{"ellipse":null}"#), "nodes[0].shape");
}

#[test]
fn test_a_spelling_the_schema_does_not_list_is_refused() {
    // Colors.
    for color in [
        "red",
        "#fff",
        "#ggg000",
        "#12345",
        "#1234567",
        "var(",
        "var(x",
        "palette(fill",
        "palette(none)",
        "rgb(1,2,3)",
        "",
        "#",
    ] {
        rejects_as_invalid(&map_of(&format!(r#","fill":"{color}""#)), "nodes[0].fill");
    }
    // The other enums.
    rejects_as_invalid(
        &map_of(r#","sections":[{"align":"justify"}]"#),
        "nodes[0].sections[0].align",
    );
    rejects_as_invalid(
        &map_of(r#","links":[{"to":1,"kind":"curve"}]"#),
        "nodes[0].links[0].kind",
    );
    rejects_as_invalid(&map_of(r#","shape":"circle""#), "nodes[0].shape");
    rejects_as_invalid(&map_of(r#","shape":"rectangle""#), "nodes[0].shape");
    rejects_as_invalid(
        &map_of(r#","frame":{"direction":"counter_clockwise"}"#),
        "nodes[0].frame.direction",
    );
    rejects_as_invalid(&map_of(r#","frame":{"start":"middle"}"#), "nodes[0].frame.start");
    rejects_as_invalid(&map_of(r#","frame":{"start":"Top"}"#), "nodes[0].frame.start");
    rejects_as_invalid(&map_of(r#","frame":{"fit":"stretch"}"#), "nodes[0].frame.fit");
    rejects_as_invalid(&map_of(r#","frame":{"fit":{}}"#), "nodes[0].frame.fit");
    rejects_as_invalid(&map_of(r#","frame":{"weight":"heavy"}"#), "nodes[0].frame.weight");
    rejects_as_invalid(&map_of(r#","weight":-100"#), "nodes[0].weight");
    rejects_as_invalid(&map_of(r#","weight":70000"#), "nodes[0].weight");
    rejects_as_invalid(&map_of(r#","weight":true"#), "nodes[0].weight");
    rejects_at(
        &map_of(r#","sections":[{"spans":[{"text":"a","link":{"url":"x","verb":"y"}}]}]"#),
        "nodes[0].sections[0].spans[0].link",
    );
    rejects_as_invalid(
        &map_of(r#","sections":[{"spans":[{"text":"a","link":{"href":"x"}}]}]"#),
        "nodes[0].sections[0].spans[0].link",
    );
    rejects_as_invalid(
        &map_of(r#","links":[{"to":1,"kind":"line","anchors":["auto"]}]"#),
        "nodes[0].links[0].anchors",
    );
    rejects_as_invalid(
        &map_of(r#","links":[{"to":1,"kind":"line","anchors":["auto","middle"]}]"#),
        "nodes[0].links[0].anchors[1]",
    );
    rejects_at(
        &map_of(r#","links":[{"to":1,"kind":"line","control_points":[[1,2,3]]}]"#),
        "nodes[0].links[0].control_points[0]",
    );
    rejects_as_invalid(
        &map_of(r#","links":[{"to":1,"kind":"portal","ends":[{}]}]"#),
        "nodes[0].links[0].ends",
    );
    rejects_at(
        &map_of(r#","links":[{"to":1,"kind":"portal","ends":[{},{},{}]}]"#),
        "nodes[0].links[0].ends",
    );
}

#[test]
fn test_every_color_spelling_but_the_listed_ones_is_refused_and_the_listed_read() {
    // The spellings the schema lists load.
    for color in [
        "#000000",
        "#abcdef",
        "#abcdef12",
        "#abcdef00",
        "var(bg)",
        "palette(fill)",
        "palette(frame)",
        "palette(text)",
        "palette(title)",
    ] {
        assert!(load(&map_of(&format!(r#","fill":"{color}""#))).is_ok(), "{color}");
    }
}

#[test]
fn test_a_color_has_one_spelling_and_the_refusal_names_it() {
    // Uppercase digits and an opaque alpha are second spellings of a
    // color that has a canonical one, and the message says which.
    let refused = |json: &str, path: &str, canonical: &str| {
        let error = rejection(json);
        let LoadErrorKind::Invalid { message } = &error.kind else {
            panic!("expected the reader's message for {json}, got {error:?}");
        };
        assert_eq!(error.path, path, "the path for {json}");
        assert!(
            message.contains(&format!("write {canonical:?}")),
            "{json}: the message should name {canonical}: {message}"
        );
    };
    refused(&map_of(r##","fill":"#ABCDEF""##), "nodes[0].fill", "#abcdef");
    refused(&map_of(r##","fill":"#AbCdEf""##), "nodes[0].fill", "#abcdef");
    refused(&map_of(r##","fill":"#ABCDEF12""##), "nodes[0].fill", "#abcdef12");
    refused(&map_of(r##","fill":"#abcdefff""##), "nodes[0].fill", "#abcdef");
    refused(&map_of(r##","fill":"#ABCDEFFF""##), "nodes[0].fill", "#abcdef");
    // The same in a theme's literal colors and in a palette's.
    refused(
        r##"{"format":1,"name":"t","theme":{"active":"d","variants":{"d":{"bg":"#000000FF"}}},"nodes":[]}"##,
        "theme.variants.d.bg",
        "#000000",
    );
    refused(
        r##"{"format":1,"name":"t","palettes":{"p":[{"fill":"#FFFFFF","frame":"#000000","text":"#000000"}]},"nodes":[]}"##,
        "palettes.p[0].fill",
        "#ffffff",
    );
    // A text that is not a color at all keeps the reader's wording.
    rejects_as_invalid(&map_of(r##","fill":"#abc""##), "nodes[0].fill");
    rejects_as_invalid(&map_of(r##","fill":"#gggggg""##), "nodes[0].fill");
}

#[test]
fn test_the_format_must_be_one() {
    let unsupported = |found| LoadErrorKind::UnsupportedFormat { found, newest: 1 };
    rejects(
        &format!(r#"{{"format":2,"name":"t",{THEME}}}"#),
        "format",
        unsupported(2),
    );
    // A number that is not a positive integer is not a format at all.
    for format in ["0", "-1", "1.0", "1e0", r#""1""#] {
        rejects_as_invalid(&format!(r#"{{"format":{format},"name":"t",{THEME}}}"#), "format");
    }
    // Whatever else is wrong with a file of another format, that is the
    // thing to report: it will have keys this build does not know.
    rejects(
        r#"{"format":2,"name":"t","something_new":true}"#,
        "format",
        unsupported(2),
    );
    rejects(r#"{"format":99}"#, "format", unsupported(99));
}

// ---- references ---------------------------------------------------

#[test]
fn test_a_duplicate_node_id_is_refused_where_the_second_is() {
    // The second 2 is under 3, two lines after the first: the error is
    // at the second, and names where the first is.
    let json = map_with(
        "",
        &[
            node_of(1, ""),
            node_of(2, r#","parent":1"#),
            node_of(3, ""),
            node_of(4, r#","parent":3"#),
            node_of(2, r#","parent":3,"channel":1"#),
        ]
        .join(","),
    );
    rejects(
        &json,
        "nodes[4].id",
        LoadErrorKind::DuplicateId {
            id: 2,
            first_index: 1,
        },
    );
    rejects(
        &map_with("", &format!("{},{}", node_of(7, ""), node_of(7, ""))),
        "nodes[1].id",
        LoadErrorKind::DuplicateId {
            id: 7,
            first_index: 0,
        },
    );
}

#[test]
fn test_a_link_to_no_node_is_refused() {
    rejects(
        &map_of(r#","links":[{"to":99,"kind":"line"}]"#),
        "nodes[0].links[0].to",
        LoadErrorKind::DanglingLink { to: 99 },
    );
    // `0` is no node's id, and is refused as the bound it breaks.
    rejects_bound(
        &map_of(r#","links":[{"to":0,"kind":"portal"}]"#),
        "nodes[0].links[0].to",
        BoundError::NotAnIdent(0),
    );
    // The second of two links is the one at fault.
    let json = map_with(
        "",
        &format!(
            "{},{}",
            node_of(1, r#","links":[{"to":2,"kind":"line"},{"to":3,"kind":"line"}]"#),
            node_of(2, "")
        ),
    );
    rejects(
        &json,
        "nodes[0].links[1].to",
        LoadErrorKind::DanglingLink { to: 3 },
    );
}

#[test]
fn test_a_link_to_its_own_node_is_refused() {
    rejects(
        &map_of(r#","links":[{"to":1,"kind":"line"}]"#),
        "nodes[0].links[0].to",
        LoadErrorKind::SelfLink,
    );
    rejects(
        &map_of(r#","links":[{"to":1,"kind":"portal"}]"#),
        "nodes[0].links[0].to",
        LoadErrorKind::SelfLink,
    );
    // A link inside a child that names the child.
    rejects(
        &map_of_two(r#""#, r#","links":[{"to":2,"kind":"line"}]"#),
        "nodes[1].links[0].to",
        LoadErrorKind::SelfLink,
    );
    // But a link to the parent, or the child, is not a self link.
    assert!(load(&map_of_two(
        r#","links":[{"to":2,"kind":"line"}]"#,
        r#","links":[{"to":1,"kind":"line"}]"#,
    ))
    .is_ok());
}

#[test]
fn test_a_verb_the_map_does_not_define_is_refused() {
    let unknown = |id: &str| LoadErrorKind::UnknownVerb { id: id.to_string() };
    rejects(
        &map_of(r#","on":{"click":"nope"}"#),
        "nodes[0].on.click",
        unknown("nope"),
    );
    rejects(
        &map_of(r#","on":{"hover":"nope"}"#),
        "nodes[0].on.hover",
        unknown("nope"),
    );
    rejects(
        &map_of(r#","sections":[{"spans":[{"text":"a","link":{"verb":"nope"}}]}]"#),
        "nodes[0].sections[0].spans[0].link.verb",
        unknown("nope"),
    );
    // A verb of another kind of list is not a verb.
    rejects(
        &map_with(
            r#","edits":[{"id":"e","name":"E"}]"#,
            &node_of(1, r#","on":{"click":"e"}"#),
        ),
        "nodes[0].on.click",
        unknown("e"),
    );
    // And one that is defined is.
    assert!(load(&map_with(
        r#","verbs":[{"id":"v","name":"V"}]"#,
        &node_of(1, r#","on":{"click":"v"}"#)
    ))
    .is_ok());
}

#[test]
fn test_a_palette_the_map_does_not_define_is_refused() {
    rejects(
        &map_of(r#","palette":"nope""#),
        "nodes[0].palette",
        LoadErrorKind::UnknownPalette {
            name: "nope".to_string(),
        },
    );
}

#[test]
fn test_a_font_this_build_has_no_face_for_is_refused() {
    let unknown = |family: &str| LoadErrorKind::UnknownFont {
        family: family.to_string(),
    };
    rejects(
        &map_of(r#","font":"No Such Font""#),
        "nodes[0].font",
        unknown("No Such Font"),
    );
    rejects(
        &map_of(r#","sections":[{"spans":[{"text":"a","font":"No Such Font"}]}]"#),
        "nodes[0].sections[0].spans[0].font",
        unknown("No Such Font"),
    );
    rejects(
        &map_with(r#","defaults":{"ring":{"font":"No Such Font"}}"#, ""),
        "defaults.ring.font",
        unknown("No Such Font"),
    );
    rejects(&map_of(r#","font":"""#), "nodes[0].font", unknown(""));
    // A compiled-in face loads.
    assert!(load(&map_of(r#","font":"Public Pixel""#)).is_ok());
}

#[test]
fn test_a_system_font_is_not_a_face_the_file_can_name() {
    // An installed system font resolves to the fallback face on native,
    // which has no name a file could write back. Which families a
    // machine has is the machine's; the test finds one if there is one.
    let system = crate::font::fonts::loaded_families_iter().find(|family| {
        crate::font::fonts::app_font_by_family(family) == Some(crate::font::fonts::AppFont::Any)
    });
    let Some(family) = system else {
        crate::util::test_skip::note_skipped(
            "no system font is installed: the fallback-face arm is unreachable here",
        );
        return;
    };
    rejects(
        &map_of(&format!(r#","font":{}"#, serde_json::to_string(family).unwrap())),
        "nodes[0].font",
        LoadErrorKind::UnknownFont {
            family: family.to_string(),
        },
    );
}

#[test]
fn test_a_theme_variable_that_is_not_defined_is_refused() {
    let unknown = |name: &str| LoadErrorKind::UnknownVariable {
        name: name.to_string(),
    };
    rejects(
        &map_of(r#","fill":"var(nope)""#),
        "nodes[0].fill",
        unknown("nope"),
    );
    rejects(
        &map_of(r#","color":"var(nope)""#),
        "nodes[0].color",
        unknown("nope"),
    );
    rejects(
        &map_of(r#","sections":[{"spans":[{"text":"a","color":"var(nope)"}]}]"#),
        "nodes[0].sections[0].spans[0].color",
        unknown("nope"),
    );
    rejects(
        &map_with(
            r##","palettes":{"p":[{"fill":"var(nope)","frame":"#000000","text":"#000000"}]}"##,
            "",
        ),
        "palettes.p[0].fill",
        unknown("nope"),
    );
    rejects(
        &map_with(r#","defaults":{"connector":{"color":"var(nope)"}}"#, ""),
        "defaults.connector.color",
        unknown("nope"),
    );
    // A name is exact: no case folding, no padding.
    rejects(&map_of(r#","fill":"var(BG)""#), "nodes[0].fill", unknown("BG"));
    rejects(&map_of(r#","fill":"var( bg)""#), "nodes[0].fill", unknown(" bg"));
}

#[test]
fn test_theme_variants_must_define_the_same_variables() {
    let differ = |variable: &str, missing| LoadErrorKind::VariantsDiffer {
        variable: variable.to_string(),
        missing,
    };
    // A variant lacking a variable the first defines.
    rejects(
        r##"{"format":1,"name":"t","theme":{"active":"a","variants":{"a":{"x":"#000000","y":"#000000"},"b":{"x":"#000000"}}}}"##,
        "theme.variants.b",
        differ("y", true),
    );
    // A variant with one the first lacks.
    rejects(
        r##"{"format":1,"name":"t","theme":{"active":"a","variants":{"a":{"x":"#000000"},"b":{"x":"#000000","z":"#000000"}}}}"##,
        "theme.variants.b",
        differ("z", false),
    );
    // Same count, different names.
    rejects(
        r##"{"format":1,"name":"t","theme":{"active":"a","variants":{"a":{"x":"#000000"},"b":{"y":"#000000"}}}}"##,
        "theme.variants.b",
        differ("x", true),
    );
    // The order of the keys in the file does not matter.
    assert!(load(r##"{"format":1,"name":"t","theme":{"active":"a","variants":{"b":{"y":"#000000","x":"#000000"},"a":{"x":"#000000","y":"#000000"}}}}"##).is_ok());
}

#[test]
fn test_the_active_variant_must_be_one_of_the_variants() {
    let not_a_variant = |name: &str| LoadErrorKind::ActiveNotAVariant {
        name: name.to_string(),
    };
    rejects(
        r##"{"format":1,"name":"t","theme":{"active":"nope","variants":{"a":{}}}}"##,
        "theme.active",
        not_a_variant("nope"),
    );
    // A theme with no variants has nothing to name.
    rejects(
        r##"{"format":1,"name":"t","theme":{"active":"a","variants":{}}}"##,
        "theme.active",
        not_a_variant("a"),
    );
    rejects(
        r##"{"format":1,"name":"t","theme":{"active":"A","variants":{"a":{}}}}"##,
        "theme.active",
        not_a_variant("A"),
    );
}

#[test]
fn test_a_palette_color_cannot_be_a_palette() {
    for field in ["fill", "frame", "text", "title"] {
        let group = |value: &str| match field {
            "fill" => format!(r##"{{"fill":"{value}","frame":"#000000","text":"#000000"}}"##),
            "frame" => format!(r##"{{"fill":"#000000","frame":"{value}","text":"#000000"}}"##),
            "text" => format!(r##"{{"fill":"#000000","frame":"#000000","text":"{value}"}}"##),
            _ => format!(r##"{{"fill":"#000000","frame":"#000000","text":"#000000","title":"{value}"}}"##),
        };
        rejects(
            &map_with(
                &format!(r#","palettes":{{"p":[{}]}}"#, group("palette(fill)")),
                "",
            ),
            &format!("palettes.p[0].{field}"),
            LoadErrorKind::PaletteInPalette,
        );
        assert!(
            load(&map_with(
                &format!(r#","palettes":{{"p":[{}]}}"#, group("var(bg)")),
                ""
            ))
            .is_ok(),
            "a variable is a color a palette may hold ({field})"
        );
    }
}

#[test]
fn test_a_top_level_node_has_no_connector() {
    rejects(
        &map_of(r#","connector":{}"#),
        "nodes[0].connector",
        LoadErrorKind::ConnectorOnRoot,
    );
    rejects(
        &map_of(r#","connector":{"visible":false}"#),
        "nodes[0].connector",
        LoadErrorKind::ConnectorOnRoot,
    );
    // A child may.
    assert!(load(&map_of_two(r#""#, r#","connector":{}"#,)).is_ok());
}

#[test]
fn test_an_id_in_a_list_of_definitions_is_unique_within_its_list() {
    let twice = |id: &str| LoadErrorKind::DuplicateDefinition { id: id.to_string() };
    for list in ["edits", "presentations", "verbs"] {
        rejects(
            &map_with(
                &format!(
                    r#","{list}":[{{"id":"a","name":"A"}},{{"id":"b","name":"B"}},{{"id":"a","name":"C"}}]"#
                ),
                "",
            ),
            &format!("{list}[2].id"),
            twice("a"),
        );
    }
    // The same id in two lists is two ids.
    assert!(load(&map_with(
        r#","edits":[{"id":"a","name":"A"}],"verbs":[{"id":"a","name":"A"}]"#,
        ""
    ))
    .is_ok());
}

#[test]
fn test_a_key_an_element_cannot_use_is_refused() {
    let not_allowed = |key: &'static str, reason: &'static str| LoadErrorKind::KeyNotAllowed { key, reason };

    // A span inside a ring has no size of its own.
    let reason = "a ring's text is sized by the ring, as a fraction of its node; a span inside a ring has no size of its own";
    rejects(
        &map_of(r#","frame":{"rings":[{"spans":[{"text":"a","size":12}]}]}"#),
        "nodes[0].frame.rings[0].spans[0].size",
        not_allowed("size", reason),
    );
    // A span outside a ring does.
    assert!(load(&map_of(r#","sections":[{"spans":[{"text":"a","size":12}]}]"#)).is_ok());

    // Only a portal link has ends.
    rejects(
        &map_of_two(r#","links":[{"to":2,"kind":"line","ends":[{},{}]}]"#, r#""#),
        "nodes[0].links[0].ends",
        not_allowed("ends", "only a portal link has ends"),
    );

    // A portal link draws no path: every key of a line is refused.
    let reason = "a portal link draws no path, only its two ends";
    for (key, value) in [
        ("fit", r#""repeat""#),
        ("spacing", "1"),
        ("size", "12"),
        ("color", r##""#000000""##),
        ("content", r#""-""#),
        ("anchors", r#"["auto","auto"]"#),
        ("control_points", "[[1,1]]"),
        ("visible", "true"),
        ("zoom", "{}"),
        ("spans", r#"[{"text":"a"}]"#),
        ("cap_start", "{}"),
        ("cap_end", "{}"),
        ("label", "{}"),
    ] {
        let key: &'static str = key;
        rejects(
            &map_of_two(
                &format!(r#","links":[{{"to":2,"kind":"portal","{key}":{value}}}]"#),
                "",
            ),
            &format!("nodes[0].links[0].{key}"),
            not_allowed(key, reason),
        );
    }
}

// ---- bounds -------------------------------------------------------

#[test]
fn test_a_canvas_coordinate_past_the_ceiling_is_refused() {
    let past = BoundError::Above {
        value: 2.0e9,
        max: bounds::MAX_CANVAS_COORD,
    };
    let below = BoundError::Below {
        value: -2.0e9,
        min: -bounds::MAX_CANVAS_COORD,
        inclusive: true,
    };
    rejects_bound(
        &map_with("", r#"{"id":1,"position":[0,2000000000]}"#),
        "nodes[0].position[1]",
        past,
    );
    rejects_bound(
        &map_with("", r#"{"id":1,"position":[-2000000000,0]}"#),
        "nodes[0].position[0]",
        below,
    );
    rejects_bound(
        &map_of(r#","sections":[{"offset":[2000000000,0]}]"#),
        "nodes[0].sections[0].offset[0]",
        past,
    );
    rejects_bound(
        &map_of_two(
            r#","links":[{"to":2,"kind":"line","control_points":[[0,0],[0,2000000000]]}]"#,
            r#""#,
        ),
        "nodes[0].links[0].control_points[1][1]",
        past,
    );
    rejects_bound(
        &map_of_two(r#""#, r#","connector":{"control_points":[[2000000000,0]]}"#),
        "nodes[1].connector.control_points[0][0]",
        past,
    );
    rejects_bound(
        &map_of_two(
            r#","links":[{"to":2,"kind":"line","label":{"offset":2000000000}}]"#,
            r#""#,
        ),
        "nodes[0].links[0].label.offset",
        past,
    );
    rejects_bound(
        &map_with(r#","defaults":{"portal_end":{"offset":-2000000000}}"#, ""),
        "defaults.portal_end.offset",
        below,
    );
    // The ceiling itself is in.
    assert!(load(&map_with("", r#"{"id":1,"position":[1000000000,-1000000000]}"#)).is_ok());
}

#[test]
fn test_a_font_size_in_points_outside_the_shapers_range_is_refused() {
    let low = BoundError::Below {
        value: 0.25,
        min: bounds::MIN_FONT_SIZE_PT,
        inclusive: true,
    };
    let high = BoundError::Above {
        value: 5000.0,
        max: bounds::MAX_FONT_SIZE_PT,
    };
    rejects_bound(&map_of(r#","size":0.25"#), "nodes[0].size", low);
    rejects_bound(&map_of(r#","size":5000"#), "nodes[0].size", high);
    rejects_bound(
        &map_of(r#","sections":[{"size":0.25}]"#),
        "nodes[0].sections[0].size",
        low,
    );
    rejects_bound(
        &map_of(r#","sections":[{"spans":[{"text":"a","size":5000}]}]"#),
        "nodes[0].sections[0].spans[0].size",
        high,
    );
    rejects_bound(
        &map_with(r#","defaults":{"span":{"size":0.25}}"#, ""),
        "defaults.span.size",
        low,
    );
    rejects_bound(
        &map_with(r#","defaults":{"node":{"size":5000}}"#, ""),
        "defaults.node.size",
        high,
    );
    rejects_bound(
        &map_with(r#","defaults":{"connector":{"size":0.25}}"#, ""),
        "defaults.connector.size",
        low,
    );
    rejects_bound(
        &map_with(r#","defaults":{"portal_end":{"size":5000}}"#, ""),
        "defaults.portal_end.size",
        high,
    );
    rejects_bound(
        &map_of_two(
            r#","links":[{"to":2,"kind":"portal","ends":[{"size":0.25},{}]}]"#,
            r#""#,
        ),
        "nodes[0].links[0].ends[0].size",
        low,
    );
    rejects_bound(
        &map_of_two(r#","links":[{"to":2,"kind":"line","size":5000}]"#, r#""#),
        "nodes[0].links[0].size",
        high,
    );
    // Both ends are in.
    assert!(load(&map_of(r#","size":0.5,"sections":[{"size":4096}]"#)).is_ok());
}

#[test]
fn test_a_size_in_a_frame_is_a_fraction_so_points_are_refused() {
    // Inside a frame, a ring and the defaults for them, `size` is a
    // fraction of the node's shorter side in (0, 1]. There is no way to
    // write points there: 24 is a fraction past all of it.
    let past = BoundError::Above {
        value: 24.0,
        max: 1.0,
    };
    rejects_bound(&map_of(r#","frame":{"size":24}"#), "nodes[0].frame.size", past);
    rejects_bound(
        &map_of(r#","frame":{"rings":[{"size":24}]}"#),
        "nodes[0].frame.rings[0].size",
        past,
    );
    rejects_bound(
        &map_with(r#","defaults":{"frame":{"size":24}}"#, ""),
        "defaults.frame.size",
        past,
    );
    rejects_bound(
        &map_with(r#","defaults":{"ring":{"size":24}}"#, ""),
        "defaults.ring.size",
        past,
    );
    // Zero, negative and past one.
    let zero = BoundError::Below {
        value: 0.0,
        min: 0.0,
        inclusive: false,
    };
    let negative = BoundError::Below {
        value: -0.5,
        min: 0.0,
        inclusive: false,
    };
    let over = BoundError::Above { value: 1.5, max: 1.0 };
    rejects_bound(&map_of(r#","frame":{"size":0}"#), "nodes[0].frame.size", zero);
    rejects_bound(
        &map_of(r#","frame":{"rings":[{"size":-0.5}]}"#),
        "nodes[0].frame.rings[0].size",
        negative,
    );
    rejects_bound(
        &map_with(r#","defaults":{"ring":{"size":1.5}}"#, ""),
        "defaults.ring.size",
        over,
    );
    // Outside a frame the same numbers are points: 0.5 is the floor.
    assert!(load(&map_of(r#","size":0.5"#)).is_ok());
    rejects_bound(
        &map_of(r#","size":0"#),
        "nodes[0].size",
        BoundError::Below {
            value: 0.0,
            min: bounds::MIN_FONT_SIZE_PT,
            inclusive: true,
        },
    );
    // And one is a whole node.
    assert!(load(&map_of(r#","frame":{"size":1}"#)).is_ok());
}

#[test]
fn test_a_number_outside_the_unit_interval_is_refused() {
    let over = BoundError::Above { value: 1.5, max: 1.0 };
    let under = BoundError::Below {
        value: -0.25,
        min: 0.0,
        inclusive: true,
    };
    rejects_bound(&map_of(r#","opacity":1.5"#), "nodes[0].opacity", over);
    rejects_bound(&map_of(r#","opacity":-0.25"#), "nodes[0].opacity", under);
    rejects_bound(
        &map_of(r#","frame":{"start":{"edge":"top","at":1.5}}"#),
        "nodes[0].frame.start.at",
        over,
    );
    rejects_bound(
        &map_of_two(r#","links":[{"to":2,"kind":"line","label":{"t":-0.25}}]"#, r#""#),
        "nodes[0].links[0].label.t",
        under,
    );
    rejects_bound(
        &map_of_two(
            r#","links":[{"to":2,"kind":"portal","ends":[{},{"border_t":1.5}]}]"#,
            r#""#,
        ),
        "nodes[0].links[0].ends[1].border_t",
        over,
    );
    rejects_bound(
        &map_with(r#","defaults":{"label":{"t":1.5}}"#, ""),
        "defaults.label.t",
        over,
    );
    rejects_bound(
        &map_with(r#","defaults":{"portal_end":{"border_t":-0.25}}"#, ""),
        "defaults.portal_end.border_t",
        under,
    );
    rejects_bound(
        &map_with(
            r#","defaults":{"frame":{"start":{"edge":"left","at":-0.25}}}"#,
            "",
        ),
        "defaults.frame.start.at",
        under,
    );
    // Both ends are in.
    assert!(load(&map_of(r#","opacity":0,"frame":{"start":{"edge":"top","at":1}}"#)).is_ok());
    assert!(load(&map_of(r#","opacity":1"#)).is_ok());
}

#[test]
fn test_a_number_that_must_not_be_negative_is_refused() {
    let negative = |value| BoundError::Below {
        value,
        min: 0.0,
        inclusive: true,
    };
    rejects_bound(
        &map_of(r#","fill_fade":-1"#),
        "nodes[0].fill_fade",
        negative(-1.0),
    );
    rejects_bound(
        &map_of(r#","min_size":[-1,0]"#),
        "nodes[0].min_size[0]",
        negative(-1.0),
    );
    rejects_bound(
        &map_of(r#","sections":[{"min_size":[0,-2]}]"#),
        "nodes[0].sections[0].min_size[1]",
        negative(-2.0),
    );
    rejects_bound(
        &map_of(r#","frame":{"gap":-3}"#),
        "nodes[0].frame.gap",
        negative(-3.0),
    );
    rejects_bound(
        &map_of(r#","frame":{"inset":-4}"#),
        "nodes[0].frame.inset",
        negative(-4.0),
    );
    rejects_bound(
        &map_of_two(r#","links":[{"to":2,"kind":"line","spacing":-5}]"#, r#""#),
        "nodes[0].links[0].spacing",
        negative(-5.0),
    );
    rejects_bound(
        &map_with(r#","defaults":{"node":{"fill_fade":-6}}"#, ""),
        "defaults.node.fill_fade",
        negative(-6.0),
    );
    rejects_bound(
        &map_with(r#","defaults":{"frame":{"gap":-7}}"#, ""),
        "defaults.frame.gap",
        negative(-7.0),
    );
    rejects_bound(
        &map_with(r#","defaults":{"connector":{"spacing":-8}}"#, ""),
        "defaults.connector.spacing",
        negative(-8.0),
    );
    // And a minimum size past the canvas.
    rejects_bound(
        &map_of(r#","min_size":[2000000000,0]"#),
        "nodes[0].min_size[0]",
        BoundError::Above {
            value: 2.0e9,
            max: bounds::MAX_CANVAS_COORD,
        },
    );
    // Zero is in.
    assert!(load(&map_of(
        r#","fill_fade":0,"min_size":[0,0],"frame":{"gap":0,"inset":0}"#
    ))
    .is_ok());
}

#[test]
fn test_a_zoom_bound_is_above_zero() {
    // A camera zoom is always above 0, so a bound at 0 or below it is
    // never reached: one rule with the mutations' (`check_zoom_window`).
    let not_above_zero = |value: f32| BoundError::Below {
        value,
        min: 0.0,
        inclusive: false,
    };
    rejects_bound(
        &map_of(r#","zoom":{"min":0}"#),
        "nodes[0].zoom.min",
        not_above_zero(0.0),
    );
    rejects_bound(
        &map_of(r#","zoom":{"max":-1}"#),
        "nodes[0].zoom.max",
        not_above_zero(-1.0),
    );
    rejects_bound(
        &map_of_two(r#""#, r#","connector":{"zoom":{"min":-0.5}}"#),
        "nodes[1].connector.zoom.min",
        not_above_zero(-0.5),
    );
    // Control: the smallest positive bound is in.
    assert!(load(&map_of(r#","zoom":{"min":1e-6,"max":1e-6}"#)).is_ok());
}

#[test]
fn test_a_zoom_range_must_not_be_inverted() {
    rejects_bound(
        &map_of(r#","zoom":{"min":4,"max":2}"#),
        "nodes[0].zoom",
        BoundError::Inverted { min: 4.0, max: 2.0 },
    );
    assert!(load(&map_of(r#","zoom":{"min":2,"max":2}"#)).is_ok());
}

#[test]
fn test_a_fit_range_must_be_positive_and_not_inverted() {
    rejects_bound(
        &map_of(r#","frame":{"fit":{"fit":{"min":0,"max":1}}}"#),
        "nodes[0].frame.fit",
        BoundError::Below {
            value: 0.0,
            min: 0.0,
            inclusive: false,
        },
    );
    rejects_bound(
        &map_of(r#","frame":{"rings":[{"fit":{"fit":{"min":2,"max":1}}}]}"#),
        "nodes[0].frame.rings[0].fit",
        BoundError::Inverted { min: 2.0, max: 1.0 },
    );
    rejects_bound(
        &map_with(
            r#","defaults":{"connector":{"fit":{"fit":{"min":-1,"max":1}}}}"#,
            "",
        ),
        "defaults.connector.fit",
        BoundError::Below {
            value: -1.0,
            min: 0.0,
            inclusive: false,
        },
    );
    assert!(load(&map_of(r#","frame":{"fit":{"fit":{"min":0.001,"max":0.001}}}"#)).is_ok());
}

#[test]
fn test_a_corner_radius_past_one_hundred_is_refused() {
    let over = |value| BoundError::Above { value, max: 100.0 };
    rejects_bound(
        &map_of(r#","shape":{"rectangle":{"radius":101}}"#),
        "nodes[0].shape.rectangle.radius",
        over(101.0),
    );
    rejects_bound(
        &map_of(r#","shape":{"rectangle":{"radius":300}}"#),
        "nodes[0].shape.rectangle.radius",
        over(300.0),
    );
    rejects_bound(
        &map_with(
            r#","defaults":{"node":{"shape":{"rectangle":{"radius":101}}}}"#,
            "",
        ),
        "defaults.node.shape.rectangle.radius",
        over(101.0),
    );
    for radius in [0, 100] {
        assert!(load(&map_of(&format!(
            r#","shape":{{"rectangle":{{"radius":{radius}}}}}"#
        )))
        .is_ok());
    }
}

#[test]
fn test_a_weight_must_be_one_of_the_nine() {
    for weight in [0, 50, 99, 101, 450, 1000] {
        rejects_bound(
            &map_of(&format!(r#","weight":{weight}"#)),
            "nodes[0].weight",
            BoundError::NotAWeight(weight),
        );
    }
    rejects_bound(
        &map_of(r#","frame":{"rings":[{"weight":650}]}"#),
        "nodes[0].frame.rings[0].weight",
        BoundError::NotAWeight(650),
    );
    for weight in ["100", "400", "700", "900", r#""normal""#, r#""bold""#] {
        assert!(
            load(&map_of(&format!(r#","weight":{weight}"#))).is_ok(),
            "{weight}"
        );
    }
}

#[test]
fn test_a_path_has_at_most_two_control_points() {
    let json = |points: &str| {
        map_of_two(
            &format!(r#","links":[{{"to":2,"kind":"line","control_points":{points}}}]"#),
            "",
        )
    };
    for points in ["[]", "[[1,1]]", "[[1,1],[2,2]]"] {
        assert!(load(&json(points)).is_ok(), "{points}");
    }
    rejects_bound(
        &json("[[1,1],[2,2],[3,3]]"),
        "nodes[0].links[0].control_points",
        BoundError::TooMany {
            count: 3,
            max: bounds::MAX_CONTROL_POINTS,
        },
    );
    rejects_bound(
        &map_of_two(
            r#""#,
            r#","connector":{"control_points":[[0,0],[0,0],[0,0],[0,0]]}"#,
        ),
        "nodes[1].connector.control_points",
        BoundError::TooMany {
            count: 4,
            max: bounds::MAX_CONTROL_POINTS,
        },
    );
}

#[test]
fn test_a_node_id_must_be_a_node_id() {
    let not_an_id = |value| BoundError::NotAnIdent(value);
    rejects_bound(
        &map_with("", r#"{"id":0,"position":[0,0]}"#),
        "nodes[0].id",
        not_an_id(0),
    );
    rejects_bound(
        &map_with("", &format!(r#"{{"id":{},"position":[0,0]}}"#, u32::MAX)),
        "nodes[0].id",
        not_an_id(u64::from(u32::MAX)),
    );
    rejects_bound(
        &map_with(
            "",
            &format!(r#"{},{{"id":0,"parent":1,"position":[0,0]}}"#, node_of(1, "")),
        ),
        "nodes[1].id",
        not_an_id(0),
    );
    // The highest id there is loads, and leaves the counter nothing to issue.
    let doc = load(&map_with(
        "",
        &format!(r#"{{"id":{},"position":[0,0]}}"#, bounds::MAX_NODE_IDENT),
    ))
    .unwrap();
    assert_eq!(doc.next_ident(), None);
    // One below it leaves exactly one.
    let doc = load(&map_with(
        "",
        &format!(r#"{{"id":{},"position":[0,0]}}"#, bounds::MAX_NODE_IDENT - 1),
    ))
    .unwrap();
    assert_eq!(
        doc.next_ident().map(|ident| ident.0),
        Some(bounds::MAX_NODE_IDENT)
    );
}

#[test]
fn test_a_channel_is_a_bound_every_target_holds() {
    // A `usize` is 32 bits on wasm32, so the bound is `u32::MAX`
    // and the number is read as a `u64` first: one refusal, with one
    // message, on every target (a `usize` field would refuse a smaller
    // range there, with the reader's wording).
    let past = u64::try_from(bounds::MAX_CHANNEL).unwrap() + 1;
    let not_a_channel = BoundError::NotAChannel(past);
    rejects_bound(
        &map_with("", &format!(r#"{{"id":1,"channel":{past},"position":[0,0]}}"#)),
        "nodes[0].channel",
        not_a_channel,
    );
    rejects_bound(
        &map_of_two("", &format!(r#","channel":{past}"#)),
        "nodes[1].channel",
        not_a_channel,
    );
    rejects_bound(
        &map_of(&format!(r#","sections":[{{}},{{"channel":{past}}}]"#)),
        "nodes[0].sections[1].channel",
        not_a_channel,
    );
    rejects_bound(
        &map_of(&format!(r#","frame":{{"rings":[{{"channel":{past}}}]}}"#)),
        "nodes[0].frame.rings[0].channel",
        not_a_channel,
    );
    // A negative number is not a channel either, and the reader says so.
    rejects_as_invalid(
        &map_with("", r#"{"id":1,"channel":-1,"position":[0,0]}"#),
        "nodes[0].channel",
    );
    // The bound itself loads, in all three places.
    let top = bounds::MAX_CHANNEL;
    let doc = load(&map_with(
        "",
        &format!(
            r#"{{"id":1,"channel":{top},"position":[0,0],"sections":[{{"channel":{top}}}],"frame":{{"rings":[{{"channel":{top}}}]}}}}"#
        ),
    ))
    .unwrap();
    let mut channels = Vec::new();
    let mut open = vec![doc.root()];
    while let Some(id) = open.pop() {
        channels.push(doc.get(id).unwrap().channel());
        open.extend(doc.children(id));
    }
    assert_eq!(
        channels.iter().filter(|channel| **channel == top).count(),
        3,
        "the node, its section and its ring: {channels:?}"
    );
}

/// `count` entries named `p000000`, …, as the text of the object's
/// members.
fn members(count: usize, value: &str) -> String {
    (0..count)
        .map(|n| format!(r#""p{n:06}":{value}"#))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn test_more_palettes_than_an_index_addresses_is_refused() {
    let json = |count| map_with(&format!(r#","palettes":{{{}}}"#, members(count, "[]")), "");
    let doc = load(&json(bounds::MAX_PALETTES)).unwrap();
    assert_eq!(doc.palettes().count(), bounds::MAX_PALETTES);
    assert_eq!(
        doc.palettes().last().map(|(id, _)| id.0),
        Some(u16::MAX),
        "the last palette has the last index, none aliased"
    );
    rejects_bound(
        &json(bounds::MAX_PALETTES + 1),
        "palettes",
        BoundError::TooMany {
            count: bounds::MAX_PALETTES + 1,
            max: bounds::MAX_PALETTES,
        },
    );
}

#[test]
fn test_more_theme_variables_than_an_index_addresses_is_refused() {
    let json = |count| {
        format!(
            r##"{{"format":1,"name":"t","theme":{{"active":"d","variants":{{"d":{{{}}}}}}}}}"##,
            members(count, r##""#000000""##)
        )
    };
    let doc = load(&json(bounds::MAX_THEME_VARIABLES)).unwrap();
    assert_eq!(doc.theme().variables.len(), bounds::MAX_THEME_VARIABLES);
    rejects_bound(
        &json(bounds::MAX_THEME_VARIABLES + 1),
        "theme.variants.d",
        BoundError::TooMany {
            count: bounds::MAX_THEME_VARIABLES + 1,
            max: bounds::MAX_THEME_VARIABLES,
        },
    );
}

#[test]
fn test_more_theme_variants_than_an_index_addresses_is_refused() {
    let json = |count| {
        format!(
            r#"{{"format":1,"name":"t","theme":{{"active":"p000000","variants":{{{}}}}}}}"#,
            members(count, "{}")
        )
    };
    let doc = load(&json(bounds::MAX_THEME_VARIANTS)).unwrap();
    assert_eq!(doc.theme().variants.len(), bounds::MAX_THEME_VARIANTS);
    rejects_bound(
        &json(bounds::MAX_THEME_VARIANTS + 1),
        "theme.variants",
        BoundError::TooMany {
            count: bounds::MAX_THEME_VARIANTS + 1,
            max: bounds::MAX_THEME_VARIANTS,
        },
    );
}

/// `count` definitions, as the text of an array's elements.
fn definitions(count: usize) -> String {
    (0..count)
        .map(|n| format!(r#"{{"id":"d{n}","name":""}}"#))
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn test_more_definitions_than_an_index_addresses_are_refused() {
    for (key, max) in [
        ("verbs", bounds::MAX_VERBS),
        ("edits", bounds::MAX_EDITS),
        ("presentations", bounds::MAX_PRESENTATIONS),
    ] {
        let json = |count| map_with(&format!(r#","{key}":[{}]"#, definitions(count)), "");
        assert!(load(&json(max)).is_ok(), "{key}: exactly the bound loads");
        rejects_bound(&json(max + 1), key, BoundError::TooMany { count: max + 1, max });
    }
    // The last verb has the last index, and a node can name it.
    let json = map_with(
        &format!(r#","verbs":[{}]"#, definitions(bounds::MAX_VERBS)),
        &node_of(1, &format!(r#","on":{{"click":"d{}"}}"#, bounds::MAX_VERBS - 1)),
    );
    let doc = load(&json).unwrap();
    let (_, data) = crate::document::tests::format_support::node(&doc, 1);
    assert_eq!(data.on.click.map(|verb| verb.0), Some(u16::MAX));
}
