// SPDX-License-Identifier: MPL-2.0

//! What the saver refuses. A save never writes text that the loader
//! refuses or reads differently, and never drops a value: every state a
//! document can hold that the file has no spelling for is a `SaveError`
//! naming the path the value would have had, and `check_savable` says
//! the same thing `save` does. One test per rule, each with its path.
//! The documents are built through the test builder, unchecked, because
//! they are exactly the ones the mutations (V1-B3) refuse to make.

use crate::document::element::{
    DefaultsData, DocElement, FlowStyle, FrameData, FrameStyle, PaletteData, PaletteGroup, PortalEndData,
    RingData, RingStyle, SectionData, SpanData, TextStyle, ThemeData, ThemeVariant,
};
use crate::document::format::{
    check_savable, check_savable_holding, check_savable_under, load, load_under, save, Limits, LoadErrorKind,
    SaveError, SaveErrorKind,
};
use crate::document::ids::{Definitions, EditDef, VerbDef};
use crate::document::tests::format_memory_shapes::elements_of;
use crate::document::tests::format_support::{same_document, written};
use crate::document::tests::format_text::{map_of, map_of_two, map_with, node_of};
use crate::document::tests::test_support::{node_data, path_data, DocumentBuilder};
use crate::document::values::{ColorRef, FontSize, LinkKind, PlainColor, Rgba, VarId, VerbId};
use crate::font::fonts::AppFont;

/// The error `save` gives for the document `builder` makes; `check_savable`
/// must give the same one.
fn refused(builder: DocumentBuilder) -> SaveError {
    let doc = builder.build_unchecked().expect("the map row is fine");
    let error = save(&doc).expect_err("the document must not save");
    assert_eq!(
        check_savable(&doc).expect_err("check_savable must refuse what save refuses"),
        error
    );
    // The one-node check refuses it too: a pass there is never a
    // document the full build refuses.
    assert_eq!(
        check_savable_holding(&doc).1,
        None,
        "the one-node check passed a document the full build refuses: {error}"
    );
    error
}

/// `builder`'s document is refused at `path` with `kind`.
fn refuses(builder: DocumentBuilder, path: &str, kind: SaveErrorKind) {
    let error = refused(builder);
    assert_eq!((error.path.as_str(), &error.kind), (path, &kind));
}

/// A theme of one variant `d` with the variables `names`, all black.
fn theme_of(names: &[&str]) -> ThemeData {
    ThemeData {
        variables: names.iter().map(|name| (*name).to_string()).collect(),
        variants: vec![ThemeVariant {
            name: "d".to_string(),
            values: names.iter().map(|_| Rgba::opaque(0, 0, 0)).collect(),
        }],
        active: 0,
    }
}

/// A builder holding one top-level node, `1`, and that node.
fn with_node(f: impl FnOnce(&mut crate::document::element::NodeData)) -> DocumentBuilder {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let mut data = node_data(1);
    f(&mut data);
    builder.node_with(root, data);
    builder
}

#[test]
fn test_the_document_the_builder_starts_with_round_trips() {
    // The empty theme is `ThemeData::default()` and has exactly one
    // spelling, no `theme` key; an absent `theme` loads as that default.
    let doc = DocumentBuilder::new().build().unwrap();
    let text = written(&doc);
    assert!(!text.contains("\"theme\""), "{text}");
    let back = load(&text).unwrap();
    assert_eq!(back.theme(), &ThemeData::default());
    same_document(&doc, &back).unwrap();
    assert_eq!(written(&back), text);
    check_savable(&doc).unwrap();
}

#[test]
fn test_a_theme_that_is_present_still_needs_a_variant() {
    // The file side of the same rule: `theme` absent is the empty theme,
    // `theme` present with nothing in it is not.
    let error = load(r#"{"format":1,"name":"t","theme":{"active":"","variants":{}}}"#).unwrap_err();
    assert_eq!(error.path, "theme.active");
    assert!(matches!(
        error.kind,
        crate::document::format::LoadErrorKind::ActiveNotAVariant { .. }
    ));
}

#[test]
fn test_a_theme_with_variables_or_an_active_index_but_no_variants_has_no_spelling() {
    let mut builder = DocumentBuilder::new();
    builder.theme(ThemeData {
        variables: vec!["x".to_string()],
        variants: Vec::new(),
        active: 0,
    });
    refuses(
        builder,
        "theme",
        SaveErrorKind::ThemeWithoutVariants {
            variables: 1,
            active: 0,
        },
    );
    let mut builder = DocumentBuilder::new();
    builder.theme(ThemeData {
        variables: Vec::new(),
        variants: Vec::new(),
        active: 2,
    });
    refuses(
        builder,
        "theme",
        SaveErrorKind::ThemeWithoutVariants {
            variables: 0,
            active: 2,
        },
    );
}

#[test]
fn test_the_active_variant_must_be_one_of_the_variants() {
    let mut builder = DocumentBuilder::new();
    let mut theme = theme_of(&["x"]);
    theme.active = 4;
    builder.theme(theme);
    refuses(
        builder,
        "theme.active",
        SaveErrorKind::ActiveNotAVariant {
            active: 4,
            variants: 1,
        },
    );
}

#[test]
fn test_a_variant_holds_one_value_per_variable() {
    let mut builder = DocumentBuilder::new();
    let mut theme = theme_of(&["x", "y"]);
    theme.variants[0].values.pop();
    builder.theme(theme);
    refuses(
        builder,
        "theme.variants.d",
        SaveErrorKind::VariantLength {
            values: 1,
            variables: 2,
        },
    );
}

#[test]
fn test_two_variants_two_variables_or_two_palettes_of_one_name_have_no_spelling() {
    let named = |name: &str| SaveErrorKind::DuplicateName {
        name: name.to_string(),
    };
    // Two variants named `d`.
    let mut builder = DocumentBuilder::new();
    let mut theme = theme_of(&["x"]);
    theme.variants.push(theme.variants[0].clone());
    builder.theme(theme);
    refuses(builder, "theme.variants.d", named("d"));
    // Two variables named `x`.
    let mut builder = DocumentBuilder::new();
    builder.theme(theme_of(&["x", "x"]));
    refuses(builder, "theme.variants.d.x", named("x"));
    // Two palettes named `a.b`, whose name is quoted in the path.
    let mut builder = DocumentBuilder::new();
    for _ in 0..2 {
        builder.palette(PaletteData {
            name: "a.b".to_string(),
            groups: Vec::new(),
        });
    }
    refuses(builder, r#"palettes["a.b"]"#, named("a.b"));
}

#[test]
fn test_two_definitions_of_one_id_have_no_spelling() {
    let mut builder = DocumentBuilder::new();
    let edit = |id: &str| EditDef {
        id: id.to_string(),
        name: "n".to_string(),
    };
    builder.definitions(Definitions {
        edits: vec![edit("a"), edit("b"), edit("a")],
        ..Definitions::default()
    });
    refuses(
        builder,
        "edits[2].id",
        SaveErrorKind::DuplicateDefinition { id: "a".to_string() },
    );
    let mut builder = DocumentBuilder::new();
    let verb = |id: &str| VerbDef {
        id: id.to_string(),
        name: "n".to_string(),
    };
    builder.definitions(Definitions {
        verbs: vec![verb("v"), verb("v")],
        ..Definitions::default()
    });
    refuses(
        builder,
        "verbs[1].id",
        SaveErrorKind::DuplicateDefinition { id: "v".to_string() },
    );
}

#[test]
fn test_a_color_naming_a_variable_the_theme_lacks_has_no_spelling() {
    // No more `var()` with nothing behind it: the saver says so, at the
    // color's path, where the loader would only have been able to refuse
    // the file it wrote.
    let unknown = |index| SaveErrorKind::UnknownVariable { index };
    let mut builder = with_node(|node| node.style.text.color = Some(ColorRef::Var(VarId(9))));
    builder.theme(theme_of(&["x"]));
    refuses(builder, "nodes[0].color", unknown(9));
    let mut builder = with_node(|node| node.style.fill = Some(ColorRef::Var(VarId(1))));
    builder.theme(theme_of(&["x"]));
    refuses(builder, "nodes[0].fill", unknown(1));
    // A palette group has all its colors; one that cannot be named is
    // refused, not written as a variable no theme defines.
    let mut builder = DocumentBuilder::new();
    builder.theme(theme_of(&[]));
    builder.palette(PaletteData {
        name: "p".to_string(),
        groups: vec![PaletteGroup {
            fill: PlainColor::Var(VarId(3)),
            frame: PlainColor::Rgba(Rgba::opaque(0, 0, 0)),
            text: PlainColor::Rgba(Rgba::opaque(0, 0, 0)),
            title: None,
        }],
    });
    refuses(builder, "palettes.p[0].fill", unknown(3));
    // A variable that is defined is written.
    let mut builder = with_node(|node| node.style.fill = Some(ColorRef::Var(VarId(0))));
    builder.theme(theme_of(&["x"]));
    let text = written(&builder.build().unwrap());
    assert!(text.contains("\"fill\": \"var(x)\""), "{text}");
}

#[test]
fn test_a_palette_or_a_verb_the_map_lacks_has_no_spelling() {
    let builder = with_node(|node| node.palette = Some(crate::document::values::PaletteId(5)));
    refuses(
        builder,
        "nodes[0].palette",
        SaveErrorKind::UnknownPalette { index: 5 },
    );
    let builder = with_node(|node| node.on.click = Some(VerbId(2)));
    refuses(
        builder,
        "nodes[0].on.click",
        SaveErrorKind::UnknownVerb { index: 2 },
    );
    let builder = with_node(|node| node.on.hover = Some(VerbId(0)));
    refuses(
        builder,
        "nodes[0].on.hover",
        SaveErrorKind::UnknownVerb { index: 0 },
    );
}

#[test]
fn test_the_fallback_face_has_no_family_name_to_write() {
    let builder = with_node(|node| node.style.text.font = Some(AppFont::Any));
    refuses(builder, "nodes[0].font", SaveErrorKind::NoFamilyName);
    // And in the defaults, which the file spells the same way.
    let mut builder = DocumentBuilder::new();
    builder.defaults(DefaultsData {
        section: TextStyle {
            font: Some(AppFont::Any),
            ..TextStyle::default()
        },
        ..DefaultsData::default()
    });
    refuses(builder, "defaults.section.font", SaveErrorKind::NoFamilyName);
}

#[test]
fn test_a_size_in_the_wrong_unit_has_no_spelling() {
    let points_where_a_fraction_goes = SaveErrorKind::WrongUnit { found_points: true };
    let fraction_where_points_go = SaveErrorKind::WrongUnit { found_points: false };
    // A fraction on a node, a section and a span.
    let builder = with_node(|node| node.style.text.size = Some(FontSize::Rel(0.5)));
    refuses(builder, "nodes[0].size", fraction_where_points_go.clone());
    let mut builder = with_node(|_| {});
    let node = builder
        .tree()
        .root()
        .children(builder.tree().arena())
        .next_back()
        .unwrap();
    builder.add(
        node,
        DocElement::Section(SectionData {
            text: TextStyle {
                size: Some(FontSize::Rel(0.2)),
                ..TextStyle::default()
            },
            ..SectionData::default()
        }),
    );
    refuses(
        builder,
        "nodes[0].sections[0].size",
        fraction_where_points_go.clone(),
    );
    // Points on a frame, on a ring.
    let mut builder = with_node(|_| {});
    let node = builder
        .tree()
        .root()
        .children(builder.tree().arena())
        .next_back()
        .unwrap();
    builder.add(
        node,
        DocElement::Frame(FrameData {
            style: FrameStyle {
                text: TextStyle {
                    size: Some(FontSize::Pt(24.0)),
                    ..TextStyle::default()
                },
                ..FrameStyle::default()
            },
        }),
    );
    refuses(
        builder,
        "nodes[0].frame.size",
        points_where_a_fraction_goes.clone(),
    );
    let mut builder = with_node(|_| {});
    let node = builder
        .tree()
        .root()
        .children(builder.tree().arena())
        .next_back()
        .unwrap();
    let frame = builder.add(node, DocElement::Frame(FrameData::default()));
    builder.add(
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
    refuses(
        builder,
        "nodes[0].frame.rings[0].size",
        points_where_a_fraction_goes.clone(),
    );
    // The defaults take the same units as what they default.
    let mut builder = DocumentBuilder::new();
    builder.defaults(DefaultsData {
        ring: RingStyle {
            fit: None,
            text: TextStyle {
                size: Some(FontSize::Pt(24.0)),
                ..TextStyle::default()
            },
        },
        ..DefaultsData::default()
    });
    refuses(builder, "defaults.ring.size", points_where_a_fraction_goes);
    let mut builder = DocumentBuilder::new();
    builder.defaults(DefaultsData {
        span: TextStyle {
            size: Some(FontSize::Rel(0.1)),
            ..TextStyle::default()
        },
        ..DefaultsData::default()
    });
    refuses(builder, "defaults.span.size", fraction_where_points_go);
}

#[test]
fn test_a_span_in_a_ring_has_no_size_of_its_own() {
    let mut builder = with_node(|_| {});
    let node = builder
        .tree()
        .root()
        .children(builder.tree().arena())
        .next_back()
        .unwrap();
    let frame = builder.add(node, DocElement::Frame(FrameData::default()));
    let ring = builder.add(frame, DocElement::Ring(RingData::default()));
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
    refuses(
        builder,
        "nodes[0].frame.rings[0].spans[0].size",
        SaveErrorKind::SizeInRing,
    );
}

#[test]
fn test_a_link_leads_to_another_node() {
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let a = builder.node(root);
    builder.node(root);
    // A link to the element that is not a node: the theme.
    let theme = builder.theme_id();
    builder.link(a, theme, LinkKind::Line);
    refuses(builder, "nodes[0].links[0].to", SaveErrorKind::LinkToNothing);
    // A link to its own node.
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let a = builder.node(root);
    builder.node(root);
    builder.link(a, a, LinkKind::Line);
    refuses(builder, "nodes[0].links[0].to", SaveErrorKind::SelfLink);
    // The second link is the one named, by its place among the links.
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let a = builder.node(root);
    let b2 = builder.node(root);
    builder.link(a, b2, LinkKind::Line);
    builder.link(a, a, LinkKind::Portal);
    refuses(builder, "nodes[0].links[1].to", SaveErrorKind::SelfLink);
}

#[test]
fn test_a_portal_link_has_two_ends() {
    for ends in [0_u8, 1, 3] {
        let mut builder = DocumentBuilder::new();
        let root = builder.root();
        let a = builder.node(root);
        let b = builder.node(root);
        // The builder's own portal link has its two ends; this one is
        // added by hand with `ends`.
        let link = builder.add(
            a,
            DocElement::Link(crate::document::element::LinkData {
                to: b,
                kind: LinkKind::Portal,
                path: path_data(),
            }),
        );
        for end in 0..ends {
            builder.add(
                link,
                DocElement::PortalEnd(PortalEndData {
                    end,
                    ..PortalEndData::default()
                }),
            );
        }
        refuses(
            builder,
            "nodes[0].links[0]",
            SaveErrorKind::PortalEnds {
                count: usize::from(ends),
            },
        );
    }
}

#[test]
fn test_a_value_the_loader_would_refuse_is_refused_at_the_same_path() {
    // The saver builds what it wrote with the loader's own builder, so a
    // number outside its bound is refused here, with what the loader said.
    let builder = with_node(|node| node.opacity = 2.0);
    let error = refused(builder);
    assert_eq!(error.path, "nodes[0].opacity");
    let SaveErrorKind::NotLoadable { error: loader } = &error.kind else {
        panic!("expected the loader's refusal, got {error:?}");
    };
    assert_eq!(loader.path, error.path);
    // A channel past the bound every target holds.
    #[cfg(target_pointer_width = "64")]
    {
        let past = crate::document::bounds::MAX_CHANNEL + 1;
        let builder = with_node(|node| node.channel = past);
        let error = refused(builder);
        assert_eq!(error.path, "nodes[0].channel");
        assert!(
            matches!(error.kind, SaveErrorKind::NotLoadable { .. }),
            "{error:?}"
        );
    }
    // And the highest one there is saves.
    let top = crate::document::bounds::MAX_CHANNEL;
    let doc = with_node(|node| node.channel = top).build().unwrap();
    load(&written(&doc)).unwrap();
}

#[test]
fn test_a_portal_link_does_not_carry_its_retained_path_through_a_save() {
    // The one exception to "load(save(d)) is d", stated: a portal link
    // keeps the fields of a path within a session so it can switch kind
    // without losing them, and the file's path keys belong to line links.
    let mut builder = DocumentBuilder::new();
    let root = builder.root();
    let a = builder.node(root);
    let b = builder.node(root);
    let link = builder.link(a, b, LinkKind::Portal);
    builder.edit(link, |element| {
        if let DocElement::Link(data) = element {
            data.path.visible = false;
            data.path.flow = FlowStyle {
                spacing: Some(3.0),
                ..FlowStyle::default()
            };
        }
    });
    let doc = builder.build().unwrap();
    let back = load(&written(&doc)).unwrap();
    let path_of = |doc: &crate::document::ids::Document| {
        let a = doc
            .children(doc.root())
            .find_map(|id| doc.get(id).filter(|e| e.as_node().is_some()).map(|_| id))
            .unwrap();
        doc.children(a)
            .find_map(|id| match doc.get(id) {
                Some(DocElement::Link(link)) => Some(link.path.clone()),
                _ => None,
            })
            .unwrap()
    };
    assert_ne!(path_of(&doc), path_of(&back));
    assert_eq!(path_of(&back), path_data());
    // Nothing else changed: the link, its kind and its target.
    assert_eq!(written(&back), written(&doc));
}

#[test]
fn test_check_savable_holds_one_node_not_a_second_document() {
    // A fan of 300 nodes and a chain of 300, each node with three
    // labelled sections: a node holds 7 or 8 elements, the map 2400.
    let label = r#","sections":[{"spans":[{"text":"a"}]},{"channel":1,"spans":[{"text":"b"}]},{"channel":2,"spans":[{"text":"c"}]}]"#;
    let fan = (1..=300)
        .map(|id| {
            node_of(
                id,
                &if id == 1 {
                    label.to_string()
                } else {
                    format!(r#","parent":1{label}"#)
                },
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    let chain = (1..=300)
        .map(|id| {
            node_of(
                id,
                &if id == 1 {
                    label.to_string()
                } else {
                    format!(r#","parent":{}{label}"#, id - 1)
                },
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    for (name, nodes) in [("fan", fan), ("chain", chain)] {
        let doc = load(&map_with("", &nodes)).expect("the map loads");
        let (result, held) = check_savable_holding(&doc);
        assert_eq!(result, Ok(()), "{name}");
        let held = held.expect("the one-node check passed");
        // The root, the theme and the defaults, the stand-in, and one
        // node's eight.
        assert!(
            held <= 3 + 1 + 8,
            "{name}: the check held {held} elements at once"
        );
        assert!(elements_of(&doc) > 2000, "precondition: {name} is big");
    }
}

#[test]
fn test_a_map_past_the_element_bound_is_refused_at_the_element_past_it() {
    // MAX_ELEMENTS is `None` on every target the suite runs on, so the
    // refusal the browser depends on is driven with a bound of its own.
    let bound = |elements: u64| Limits {
        bytes: None,
        elements: Some(elements),
    };
    // The root, the theme, the defaults, node 1, its section and its
    // span: the span is the sixth element.
    let json = map_of(r#","sections":[{"spans":[{"text":"x"}]}]"#);
    let doc = load(&json).expect("the map loads");
    assert_eq!(elements_of(&doc), 6);
    assert!(
        load_under(&json, bound(6)).is_ok(),
        "a map at the bound is within it"
    );
    let error = load_under(&json, bound(5)).expect_err("one element past the bound");
    assert_eq!(
        (error.path.as_str(), &error.kind),
        (
            "nodes[0].sections[0].spans[0]",
            &LoadErrorKind::TooManyElements { max: 5 }
        )
    );
    // A definition counts as an element does, at its list.
    let json = map_with(
        r#","verbs":[{"id":"a","name":"A"},{"id":"b","name":"B"}]"#,
        &node_of(1, ""),
    );
    let defined = load(&json).expect("the map loads");
    assert_eq!(elements_of(&defined), 6);
    // Counted after the root and the theme, before the defaults.
    assert!(load_under(&json, bound(6)).is_ok());
    let error = load_under(&json, bound(3)).expect_err("the verbs are past the bound");
    assert_eq!(
        (error.path.as_str(), &error.kind),
        ("verbs", &LoadErrorKind::TooManyElements { max: 3 })
    );
    // And a document the bound refuses cannot be saved where it holds,
    // at the same path: the file would not load there.
    let refusal = check_savable_under(&doc, bound(5)).expect_err("past the bound");
    assert_eq!(refusal.path, "nodes[0].sections[0].spans[0]");
    assert!(
        matches!(&refusal.kind, SaveErrorKind::NotLoadable { error } if error.kind == LoadErrorKind::TooManyElements { max: 5 }),
        "{refusal:?}"
    );
    assert_eq!(check_savable_under(&doc, bound(6)), Ok(()));
}

#[test]
fn test_a_document_whose_file_is_past_the_byte_bound_cannot_be_saved_where_it_holds() {
    let doc = load(&map_of_two("", "")).expect("the map loads");
    let length = save(&doc).expect("it saves").len() as u64;
    let bound = |bytes: u64| Limits {
        bytes: Some(bytes),
        elements: None,
    };
    assert_eq!(check_savable_under(&doc, bound(length)), Ok(()));
    let refusal = check_savable_under(&doc, bound(length - 1)).expect_err("one byte past the bound");
    assert_eq!(refusal.path, "");
    assert!(
        matches!(&refusal.kind, SaveErrorKind::NotLoadable { error }
            if error.kind == LoadErrorKind::TooLarge { bytes: length, max: length - 1 }),
        "{refusal:?}"
    );
}
