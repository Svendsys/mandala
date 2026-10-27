// SPDX-License-Identifier: MPL-2.0

//! The format's round trip and what a load reads: every fixture saves
//! back byte for byte, the full fixture is read as it is written, and a
//! document built in memory survives a save and a load.

use crate::document::element::{
    CapData, DocElement, DocKind, FrameStyle, LabelData, LabelStyle, LinkData, NodeStyle, PathData,
    PortalEndStyle, RingStyle, TextStyle, ThemeData, ThemeVariant,
};
use crate::document::format::load;
use crate::document::ids::Definitions;
use crate::document::tests::format_support::{fixture, kid, kids, node, same_document, written, FIXTURES};
use crate::document::tests::test_support::{example, node_data, DocumentBuilder};
use crate::document::values::{
    Align, Anchor, ColorRef, Direction, Edge, FitMode, FontSize, LinkKind, LinkTarget, PaletteField,
    PaletteId, Percent, PlainColor, Rgba, Shape, StartPoint, VarId, VerbId, Weight, ZoomWindow,
};
use crate::font::fonts::AppFont;
use glam::Vec2;

#[test]
fn test_format_fixtures_round_trip_byte_for_byte() {
    do_format_fixtures_round_trip_byte_for_byte();
}

/// Every fixture is the exact text the saver writes for the document
/// the loader reads from it: `save(load(f)) == f`, and a second
/// round gives the same text again.
pub fn do_format_fixtures_round_trip_byte_for_byte() {
    for (name, text) in FIXTURES {
        let doc = load(text).unwrap_or_else(|error| panic!("fixture {name} must load: {error}"));
        let saved = written(&doc);
        assert_eq!(saved, text, "fixture {name} is not what the saver writes for it");
        assert!(saved.ends_with("}\n"), "{name} ends in a newline");
        assert!(!saved.ends_with("\n\n"), "{name} ends in exactly one");
        let again = load(&saved).unwrap_or_else(|error| panic!("{name}, saved, must load: {error}"));
        same_document(&doc, &again).unwrap_or_else(|why| panic!("{name} changed over a round trip: {why}"));
    }
}

#[test]
fn test_format_full_fixture_reads_as_written() {
    do_format_full_fixture_reads_as_written();
}

/// The full fixture is read as its text says: the theme, the palettes,
/// the defaults, the definitions, and a node with all it holds. A round
/// trip alone would pass for a loader that read every key as something
/// else and a saver that wrote it back.
pub fn do_format_full_fixture_reads_as_written() {
    let doc = load(fixture("full")).unwrap();

    // The theme: variables and variants in ascending name order, the
    // variants' values in the variables' order, `active` an index.
    let theme = doc.theme();
    assert_eq!(theme.variables, ["accent", "bg", "fg"]);
    assert_eq!(theme.active, 0);
    assert_eq!(
        theme.variants,
        [
            ThemeVariant {
                name: "dark".to_string(),
                values: vec![
                    Rgba::opaque(0xd4, 0xa2, 0x4c),
                    Rgba::opaque(0x10, 0x10, 0x14),
                    Rgba::opaque(0xe0, 0xe0, 0xe0)
                ],
            },
            ThemeVariant {
                name: "light".to_string(),
                values: vec![
                    Rgba::opaque(0x8a, 0x5a, 0x00),
                    Rgba::opaque(0xf6, 0xf3, 0xec),
                    Rgba::opaque(0x20, 0x20, 0x24)
                ],
            },
        ]
    );

    // The palettes, ids by ascending name; a group with no title has none.
    let palettes: Vec<_> = doc.palettes().collect();
    assert_eq!(palettes.len(), 2);
    assert_eq!(
        (palettes[0].0, palettes[0].1.name.as_str()),
        (PaletteId(0), "mono")
    );
    assert_eq!(
        (palettes[1].0, palettes[1].1.name.as_str()),
        (PaletteId(1), "rainbow")
    );
    let mono = &palettes[0].1.groups[0];
    assert_eq!(mono.text, PlainColor::Rgba(Rgba::new(0xff, 0xff, 0xff, 0x80)));
    assert_eq!(mono.title, None);
    let rainbow = &palettes[1].1.groups;
    assert_eq!(
        rainbow[0].title,
        Some(PlainColor::Rgba(Rgba::opaque(0xff, 0xff, 0xff)))
    );
    assert_eq!(rainbow[1].fill, PlainColor::Var(VarId(1)), "var(bg)");
    assert_eq!(rainbow[1].frame, PlainColor::Var(VarId(0)), "var(accent)");
    assert_eq!(rainbow[1].title, None, "R5: a group may have no title");

    // The definitions: ids, names, and the verbs' order.
    let definitions = doc.definitions();
    assert_eq!(definitions.edits.len() + definitions.presentations.len(), 2);
    assert_eq!(
        definitions
            .verbs
            .iter()
            .map(|verb| verb.id.as_str())
            .collect::<Vec<_>>(),
        ["glow", "reveal"]
    );
    assert_eq!(definitions.verbs[1].name, "Reveal chapter two");

    // The defaults, field by field, in the unit each sits in.
    let defaults = doc.defaults();
    assert_eq!(
        defaults.node,
        NodeStyle {
            shape: Some(Shape::Rectangle {
                radius: Percent::new(10)
            }),
            fill: Some(ColorRef::Var(VarId(1))),
            fill_fade: Some(4.0),
            text: TextStyle {
                font: Some(AppFont::PublicPixel),
                size: Some(FontSize::Pt(20.0)),
                color: Some(ColorRef::Var(VarId(2))),
                weight: Some(Weight::NORMAL),
                italic: Some(false),
            },
        }
    );
    assert_eq!(defaults.section.weight, Some(Weight::new(500).unwrap()));
    assert_eq!(
        defaults.frame,
        FrameStyle {
            visible: Some(true),
            start: Some(StartPoint {
                edge: Edge::Top,
                at: 0.0
            }),
            direction: Some(Direction::Clockwise),
            fit: Some(FitMode::Fit { min: 0.5, max: 2.0 }),
            gap: Some(2.0),
            inset: Some(4.0),
            text: TextStyle {
                font: Some(AppFont::PublicPixel),
                size: Some(FontSize::Rel(0.05)),
                color: Some(ColorRef::Var(VarId(0))),
                weight: Some(Weight::BOLD),
                italic: Some(true),
            },
        }
    );
    assert_eq!(
        defaults.ring,
        RingStyle {
            fit: Some(FitMode::Repeat),
            text: TextStyle {
                font: Some(AppFont::Novem),
                size: Some(FontSize::Rel(0.06)),
                color: Some(ColorRef::Rgba(Rgba::opaque(0x30, 0xb0, 0x82))),
                weight: Some(Weight::NORMAL),
                italic: Some(false),
            },
        }
    );
    assert_eq!(defaults.connector.content.as_deref(), Some("\u{2500}"));
    assert_eq!(defaults.connector.size, Some(12.0));
    assert_eq!(defaults.link.fit, Some(FitMode::Spread));
    assert_eq!(
        defaults.label,
        LabelStyle {
            t: Some(0.5),
            offset: Some(6.0),
            text: TextStyle {
                font: Some(AppFont::PublicPixel),
                size: Some(FontSize::Pt(13.2)),
                color: Some(ColorRef::Var(VarId(2))),
                weight: Some(Weight::NORMAL),
                italic: Some(true),
            },
        }
    );
    assert_eq!(
        defaults.portal_end,
        PortalEndStyle {
            border_t: Some(0.25),
            offset: Some(3.0),
            color: Some(ColorRef::Rgba(Rgba::opaque(0xaa, 0x88, 0xcc))),
            size: Some(40.0),
            content: Some("\u{25C8}".to_string()),
        }
    );
    assert_eq!(defaults.span.size, Some(FontSize::Pt(16.0)));

    // Node 1: its own fields.
    let (one, data) = node(&doc, 1);
    assert_eq!(data.channel, 0);
    assert_eq!(data.position, Vec2::ZERO);
    assert_eq!(data.min_size, Some(Vec2::new(320.0, 200.0)));
    assert_eq!(
        data.style.shape,
        Some(Shape::Rectangle {
            radius: Percent::new(25)
        })
    );
    assert_eq!(data.style.fill, Some(ColorRef::Var(VarId(1))));
    assert_eq!(data.style.fill_fade, Some(6.5));
    assert_eq!(data.palette, Some(PaletteId(1)), "rainbow is the second palette");
    assert_eq!(
        data.zoom,
        Some(ZoomWindow {
            min: Some(0.25),
            max: Some(8.0)
        })
    );
    assert!(data.folded && !data.hidden);
    assert_eq!(data.opacity, 0.75);
    assert_eq!(data.on.click, Some(VerbId(1)), "reveal is the second verb");
    assert_eq!(data.on.hover, Some(VerbId(0)), "glow is the first");
    assert_eq!(
        data.style.text,
        TextStyle {
            font: Some(AppFont::PublicPixel),
            size: Some(FontSize::Pt(28.0)),
            color: Some(ColorRef::Palette(PaletteField::Text)),
            weight: Some(Weight::BOLD),
            italic: Some(false),
        }
    );

    // Its sections, by channel, and the spans of the first.
    let sections = kids(&doc, one, DocKind::Section);
    assert_eq!(sections.len(), 3);
    let DocElement::Section(first) = sections[0].1 else { panic!("a section") };
    assert_eq!(first.offset, Vec2::new(8.0, 8.0));
    assert_eq!(first.min_size, Some(Vec2::new(200.0, 60.0)));
    assert_eq!(first.align, Some(Align::Center));
    assert_eq!(first.text.weight, Some(Weight::new(300).unwrap()));
    let spans: Vec<_> = kids(&doc, sections[0].0, DocKind::Span)
        .into_iter()
        .map(|(_, element)| match element {
            DocElement::Span(span) => span,
            _ => panic!("a span"),
        })
        .collect();
    assert_eq!(spans.len(), 4);
    assert_eq!(spans[0].style.size, Some(FontSize::Pt(24.0)));
    assert_eq!(spans[1].style.color, Some(ColorRef::Var(VarId(0))));
    assert_eq!(
        spans[2].link,
        Some(LinkTarget::Url("https://example.com/a?b=c&d=e".to_string()))
    );
    assert_eq!(spans[3].link, Some(LinkTarget::Verb(VerbId(0))));
    let DocElement::Section(second) = sections[1].1 else { panic!("a section") };
    assert_eq!((second.channel, second.align), (1, Some(Align::Right)));
    assert_eq!(second.offset, Vec2::ZERO, "an absent offset is the origin");
    let DocElement::Section(third) = sections[2].1 else { panic!("a section") };
    assert_eq!(third.channel, 2);
    assert!(kids(&doc, sections[2].0, DocKind::Span).is_empty());

    // Its frame: sizes are fractions, and the rings come in channel order.
    let (frame, element) = kid(&doc, one, DocKind::Frame, 0);
    let DocElement::Frame(frame_data) = element else { panic!("a frame") };
    assert_eq!(frame_data.style.visible, Some(true));
    assert_eq!(frame_data.style.fit, Some(FitMode::Fit { min: 0.5, max: 2.0 }));
    assert_eq!(frame_data.style.text.size, Some(FontSize::Rel(0.05)));
    let rings = kids(&doc, frame, DocKind::Ring);
    assert_eq!(rings.len(), 2);
    let DocElement::Ring(ring) = rings[1].1 else { panic!("a ring") };
    assert_eq!(ring.channel, 1);
    assert_eq!(ring.style.fit, Some(FitMode::Spread));
    assert_eq!(ring.style.text.size, Some(FontSize::Rel(0.04)));
    assert_eq!(ring.style.text.weight, Some(Weight::BOLD), "700");

    // Its links: a line with every path key, and a portal to node 5.
    let links = kids(&doc, one, DocKind::Link);
    assert_eq!(links.len(), 2);
    let DocElement::Link(line) = links[0].1 else { panic!("a link") };
    assert_eq!(line.kind, LinkKind::Line);
    assert_eq!(node_of(&doc, line), 3, "the line leads to node 3");
    assert_eq!(line.path.anchors, (Anchor::Right, Anchor::Left));
    assert_eq!(
        line.path.control_points,
        [Vec2::new(40.0, -20.0), Vec2::new(80.0, 20.0)]
    );
    assert!(!line.path.visible);
    assert_eq!(
        line.path.zoom,
        Some(ZoomWindow {
            min: Some(1.0),
            max: None
        })
    );
    assert_eq!(line.path.flow.size, Some(14.0));
    assert_eq!(line.path.flow.color, Some(ColorRef::Palette(PaletteField::Frame)));
    let cap = kid(&doc, links[0].0, DocKind::CapStart, 0).1;
    assert_eq!(
        cap,
        &DocElement::CapStart(CapData {
            text: TextStyle {
                font: Some(AppFont::Novem),
                size: Some(FontSize::Pt(16.0)),
                color: Some(ColorRef::Rgba(Rgba::opaque(0xff, 0, 0))),
                weight: Some(Weight::BOLD),
                italic: Some(true),
            }
        })
    );
    assert_eq!(kids(&doc, links[0].0, DocKind::CapEnd).len(), 1);
    let DocElement::Label(label) = kid(&doc, links[0].0, DocKind::Label, 0).1 else { panic!("a label") };
    assert_eq!((label.style.t, label.style.offset), (Some(0.25), Some(-6.0)));
    let DocElement::Link(portal) = links[1].1 else { panic!("a link") };
    assert_eq!(portal.kind, LinkKind::Portal);
    assert_eq!(node_of(&doc, portal), 5, "forward reference: node 5 comes after");
    let ends = kids(&doc, links[1].0, DocKind::PortalEnd);
    assert_eq!(ends.len(), 2);
    let (DocElement::PortalEnd(source), DocElement::PortalEnd(target)) = (ends[0].1, ends[1].1) else {
        panic!("portal ends")
    };
    assert_eq!((source.end, target.end), (0, 1));
    assert_eq!(source.style.content.as_deref(), Some("\u{25C6}"));
    assert_eq!(source.style.size, Some(40.0));
    assert_eq!((target.style.border_t, target.style.color), (Some(0.75), None));
    assert_eq!(kids(&doc, ends[0].0, DocKind::Label).len(), 1);

    // The children: node 2 with its connector, node 3 with none written.
    let (two, two_data) = node(&doc, 2);
    assert_eq!(doc.parent(two), Some(one));
    assert_eq!(two_data.style.shape, Some(Shape::Ellipse));
    assert!(two_data.hidden);
    let DocElement::Connector(connector) = kid(&doc, two, DocKind::Connector, 0).1 else {
        panic!("a connector")
    };
    assert_eq!(connector.path.flow.fit, Some(FitMode::Once { filler: None }));
    assert_eq!(connector.path.anchors, (Anchor::Bottom, Anchor::Top));
    assert_eq!(connector.path.control_points, [Vec2::new(10.0, 10.0)]);
    let (three, _) = node(&doc, 3);
    let DocElement::Connector(default) = kid(&doc, three, DocKind::Connector, 0).1 else {
        panic!("a connector")
    };
    assert_eq!(
        default.path,
        PathData {
            flow: Default::default(),
            anchors: (Anchor::Auto, Anchor::Auto),
            control_points: Vec::new(),
            visible: true,
            zoom: None,
        },
        "a child node with no connector has one with every field inherited"
    );
    assert!(doc
        .children(three)
        .all(|c| doc.get(c).map(DocElement::kind) != Some(DocKind::Span)));
    let (four, four_data) = node(&doc, 4);
    assert_eq!(doc.parent(four), Some(two));
    assert_eq!(four_data.channel, 3);
    assert_eq!(node(&doc, 5).1.channel, 2);
    assert_eq!(
        doc.next_ident().map(|ident| ident.0),
        Some(6),
        "past the highest id"
    );
}

/// The ident of the node a link leads to.
fn node_of(doc: &crate::document::ids::Document, link: &LinkData) -> u32 {
    match doc.get(link.to) {
        Some(DocElement::Node(node)) => node.ident.0,
        other => panic!("a link leads to {other:?}"),
    }
}

#[test]
fn test_format_builder_documents_round_trip() {
    do_format_builder_documents_round_trip();
}

/// `load(save(d)) == d` for documents built in memory: the `example`
/// document with its theme given variants, and one that sets a field
/// of every kind. Compared by structure, with a link's target by ident.
///
/// **One exception, stated rather than hidden:** the path a portal link
/// retains from being a line link is kept within a session and never
/// saved (the file's path keys belong to line links), so `same_document`
/// does not compare the path of a portal link.
/// `format_savable_tests` holds the exception in place: a portal link
/// with a path of its own does reload with the default one.
pub fn do_format_builder_documents_round_trip() {
    for (name, doc) in builder_documents() {
        let text = written(&doc);
        let back = load(&text).unwrap_or_else(|error| panic!("{name}, saved, must load: {error}\n{text}"));
        same_document(&doc, &back)
            .unwrap_or_else(|why| panic!("{name} changed over a round trip: {why}\n{text}"));
        assert_eq!(written(&back), text, "{name}: a second save writes the same text");
    }
}

/// The documents `do_format_builder_documents_round_trip` saves.
fn builder_documents() -> Vec<(&'static str, crate::document::ids::Document)> {
    let theme = || ThemeData {
        variables: vec!["accent".to_string(), "fg".to_string()],
        variants: vec![
            ThemeVariant {
                name: "dark".to_string(),
                values: vec![Rgba::opaque(0xd4, 0xa2, 0x4c), Rgba::new(0xe0, 0xe0, 0xe0, 0x80)],
            },
            ThemeVariant {
                name: "light".to_string(),
                values: vec![Rgba::opaque(0x8a, 0x5a, 0x00), Rgba::opaque(0x20, 0x20, 0x24)],
            },
        ],
        active: 1,
    };

    // The B1 example: every kind, nothing set.
    let (mut builder, _) = example();
    builder.theme(theme());
    let plain = builder.build().unwrap();

    // The same, with a field of every kind set.
    let (mut builder, ids) = example();
    builder.theme(theme());
    builder.edit_node(ids.a, |a| {
        a.channel = 2;
        a.position = Vec2::new(-12.5, 3.25);
        a.min_size = Some(Vec2::new(100.0, 40.0));
        a.style.shape = Some(Shape::Ellipse);
        a.style.fill = Some(ColorRef::Palette(PaletteField::Fill));
        a.style.fill_fade = Some(1.5);
        a.zoom = Some(ZoomWindow {
            min: None,
            max: Some(3.0),
        });
        a.folded = true;
        a.opacity = 0.5;
        a.style.text.font = Some(AppFont::Novem);
        a.style.text.size = Some(FontSize::Pt(31.0));
        a.style.text.color = Some(ColorRef::Var(VarId(1)));
        a.style.text.weight = Some(Weight::new(300).unwrap());
        a.style.text.italic = Some(true);
    });
    builder.edit(ids.frame, |frame| {
        if let DocElement::Frame(frame) = frame {
            frame.style.start = Some(StartPoint {
                edge: Edge::Left,
                at: 0.3,
            });
            frame.style.direction = Some(Direction::CounterClockwise);
            frame.style.fit = Some(FitMode::Once {
                filler: Some("*".to_string()),
            });
            frame.style.text.size = Some(FontSize::Rel(0.1));
        }
    });
    builder.edit(ids.ring, |ring| {
        if let DocElement::Ring(ring) = ring {
            ring.channel = 4;
            ring.style.fit = Some(FitMode::Fit { min: 0.5, max: 3.0 });
        }
    });
    builder.edit(ids.line, |link| {
        if let DocElement::Link(LinkData { path, .. }) = link {
            path.anchors = (Anchor::Left, Anchor::Top);
            path.control_points = vec![Vec2::new(5.0, 6.0)];
            path.flow.content = Some("-".to_string());
        }
    });
    builder.edit(ids.portal_source, |end| {
        if let DocElement::PortalEnd(end) = end {
            end.style.border_t = Some(0.125);
        }
    });
    builder.edit(ids.line_label, |label| {
        *label = DocElement::Label(LabelData {
            style: LabelStyle {
                t: Some(0.9),
                ..LabelStyle::default()
            },
        });
    });
    let busy = builder.build().unwrap();

    // Two roots with channels in descending order and no nodes below.
    let mut builder = DocumentBuilder::new();
    builder.theme(theme());
    let root = builder.root();
    builder.node_with(root, {
        let mut data = node_data(20);
        data.channel = 1;
        data
    });
    builder.node_with(root, node_data(10));
    let rows = builder.build().unwrap();

    // The bare builder map has no variant to name; a one-variant theme.
    let mut builder = DocumentBuilder::new();
    builder.theme(ThemeData {
        variables: Vec::new(),
        variants: vec![ThemeVariant {
            name: "only".to_string(),
            values: Vec::new(),
        }],
        active: 0,
    });
    builder.definitions(Definitions::default());
    let bare = builder.build().unwrap();

    vec![("plain", plain), ("busy", busy), ("rows", rows), ("bare", bare)]
}
