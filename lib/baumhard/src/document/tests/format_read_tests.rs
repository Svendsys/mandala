// SPDX-License-Identifier: MPL-2.0

//! What a load reads from a short map: the defaults for what the file
//! leaves out, which side of the unit line a size falls, and the
//! spellings that read to one value.

use crate::document::element::{DocElement, DocKind};
use crate::document::format::{load, save};
use crate::document::tests::format_support::{kid, kids, node};
use crate::document::tests::format_text::{load_node, map_of, map_with, node_of};
use crate::document::values::{
    Align, Anchor, ColorRef, FitMode, FontSize, PaletteField, PlainColor, Rgba, StartPoint, VarId, Weight,
};
use crate::font::fonts::AppFont;
use glam::Vec2;

#[test]
fn test_format_reads_what_the_loader_defaults() {
    // Opacity, folded, hidden, channel and the section offset are
    // omitted when default and read back as default.
    let doc = load_node("").unwrap();
    let (_, data) = node(&doc, 1);
    assert_eq!(
        (data.opacity, data.folded, data.hidden, data.channel),
        (1.0, false, false, 0)
    );
    assert_eq!(data.min_size, None);
    assert_eq!(data.palette, None);
    let doc = load_node(r#","sections":[{}]"#).unwrap();
    let (id, _) = node(&doc, 1);
    let DocElement::Section(section) = kid(&doc, id, DocKind::Section, 0).1 else { panic!("a section") };
    assert_eq!(
        (section.offset, section.channel, section.align),
        (Vec2::ZERO, 0, None)
    );
}

#[test]
fn test_a_path_without_anchors_or_visibility_reads_as_auto_and_drawn() {
    // The defaults for fields that are not `Option`: anchors `auto,auto`,
    // visible `true`, no control points, no zoom.
    let doc = load(&map_of(
        r#","links":[{"to":2,"kind":"line"}],"children":[{"id":2,"position":[0,0]}]"#,
    ))
    .unwrap();
    let (one, _) = node(&doc, 1);
    let DocElement::Link(link) = kid(&doc, one, DocKind::Link, 0).1 else { panic!("a link") };
    assert_eq!(link.path.anchors, (Anchor::Auto, Anchor::Auto));
    assert!(link.path.visible);
    assert!(link.path.control_points.is_empty());
    assert_eq!(link.path.zoom, None);
    // And an explicit default is the same document: it reads, and saves
    // without the key.
    let explicit = load(&map_of(
        r#","links":[{"to":2,"kind":"line","anchors":["auto","auto"],"visible":true}],"children":[{"id":2,"position":[0,0]}]"#,
    ))
    .unwrap();
    assert_eq!(save(&explicit), save(&doc));
}

#[test]
fn test_a_frame_that_says_nothing_about_visibility_inherits_it() {
    // Amendment: a frame's `visible` is optional. Absent, the frame
    // stores nothing and the built-in default (shown, D25) decides.
    let doc = load_node(r#","frame":{}"#).unwrap();
    let (one, _) = node(&doc, 1);
    let DocElement::Frame(frame) = kid(&doc, one, DocKind::Frame, 0).1 else { panic!("a frame") };
    assert_eq!(frame.style.visible, None);
    let doc = load_node(r#","frame":{"visible":false}"#).unwrap();
    let (one, _) = node(&doc, 1);
    let DocElement::Frame(frame) = kid(&doc, one, DocKind::Frame, 0).1 else { panic!("a frame") };
    assert_eq!(frame.style.visible, Some(false));
    // A node with no frame has no frame element at all.
    let doc = load_node("").unwrap();
    let (one, _) = node(&doc, 1);
    assert!(kids(&doc, one, DocKind::Frame).is_empty());
}

#[test]
fn test_a_palette_group_may_have_no_title() {
    // R5: `title` is optional, and an absent one is `None`, not a copy
    // of `text` (the first section takes `text` in the same tier).
    let doc = load(&map_with(
        r##","palettes":{"p":[{"fill":"#010101","frame":"#020202","text":"#030303"},{"fill":"#010101","frame":"#020202","text":"#030303","title":"var(fg)"}]}"##,
        "",
    ))
    .unwrap();
    let groups = &doc.palettes().next().unwrap().1.groups;
    assert_eq!(groups[0].title, None);
    assert_eq!(groups[0].text, PlainColor::Rgba(Rgba::opaque(3, 3, 3)));
    assert_eq!(groups[1].title, Some(PlainColor::Var(VarId(1))));
    // Saved, the untitled group has no `title` key.
    let text = save(&doc);
    assert_eq!(text.matches("\"title\"").count(), 1, "{text}");
}

#[test]
fn test_a_size_is_points_or_a_fraction_by_where_it_sits() {
    // The same key, `size`, is points on everything that draws text
    // and a fraction of the node inside a frame; the loader knows by
    // position and stores the unit.
    let doc = load_node(
        r#","size":12,"sections":[{"size":13,"spans":[{"text":"a","size":14}]}],"frame":{"size":0.5,"rings":[{"size":0.25}]}"#,
    )
    .unwrap();
    let (one, data) = node(&doc, 1);
    assert_eq!(data.style.text.size, Some(FontSize::Pt(12.0)));
    let (section, element) = kid(&doc, one, DocKind::Section, 0);
    let DocElement::Section(section_data) = element else { panic!("a section") };
    assert_eq!(section_data.text.size, Some(FontSize::Pt(13.0)));
    let DocElement::Span(span) = kid(&doc, section, DocKind::Span, 0).1 else { panic!("a span") };
    assert_eq!(span.style.size, Some(FontSize::Pt(14.0)));
    let (frame, element) = kid(&doc, one, DocKind::Frame, 0);
    let DocElement::Frame(frame_data) = element else { panic!("a frame") };
    assert_eq!(frame_data.style.text.size, Some(FontSize::Rel(0.5)));
    let DocElement::Ring(ring) = kid(&doc, frame, DocKind::Ring, 0).1 else { panic!("a ring") };
    assert_eq!(ring.style.text.size, Some(FontSize::Rel(0.25)));
    // A path's and a portal end's `size` is points too.
    let doc = load_node(
        r#","links":[{"to":2,"kind":"line","size":15}],"children":[{"id":2,"position":[0,0],"connector":{"size":16}}]"#,
    )
    .unwrap();
    let (one, _) = node(&doc, 1);
    let DocElement::Link(link) = kid(&doc, one, DocKind::Link, 0).1 else { panic!("a link") };
    assert_eq!(link.path.flow.size, Some(15.0));
}

#[test]
fn test_the_weight_spellings_read_to_one_value_and_save_as_one() {
    for (written, weight) in [
        ("\"normal\"", 400),
        ("400", 400),
        ("\"bold\"", 700),
        ("700", 700),
        ("100", 100),
        ("900", 900),
    ] {
        let doc = load_node(&format!(r#","weight":{written}"#)).unwrap();
        let (_, data) = node(&doc, 1);
        assert_eq!(
            data.style.text.weight,
            Some(Weight::new(weight).unwrap()),
            "{written}"
        );
        let saved = save(&doc);
        let canonical = match weight {
            400 => "\"normal\"".to_string(),
            700 => "\"bold\"".to_string(),
            other => other.to_string(),
        };
        assert!(
            saved.contains(&format!(r#""weight": {canonical}"#)),
            "{written} saves as {canonical}:\n{saved}"
        );
    }
}

#[test]
fn test_the_start_spellings_read_to_one_value_and_save_as_one() {
    // A named start and the same point written out are one value, and
    // the saver writes the name.
    for (named, edge, at) in [
        ("top-left", "top", 0.0),
        ("top", "top", 0.5),
        ("top-right", "right", 0.0),
        ("right", "right", 0.5),
        ("bottom-right", "bottom", 0.0),
        ("bottom", "bottom", 0.5),
        ("bottom-left", "left", 0.0),
        ("left", "left", 0.5),
    ] {
        let by_name = load_node(&format!(r#","frame":{{"start":"{named}"}}"#)).unwrap();
        let by_point = load_node(&format!(r#","frame":{{"start":{{"edge":"{edge}","at":{at}}}}}"#)).unwrap();
        let start = |doc: &crate::document::ids::Document| {
            let (one, _) = node(doc, 1);
            match kid(doc, one, DocKind::Frame, 0).1 {
                DocElement::Frame(frame) => frame.style.start,
                _ => panic!("a frame"),
            }
        };
        assert_eq!(start(&by_name), start(&by_point), "{named}");
        assert!(start(&by_name).is_some());
        assert_eq!(
            save(&by_point),
            save(&by_name),
            "{named}: the name is what is written"
        );
        assert!(save(&by_name).contains(&format!(r#""start": "{named}""#)));
    }
    // A point no name matches is written as a point.
    let doc = load_node(r#","frame":{"start":{"edge":"top","at":0.75}}"#).unwrap();
    let (one, _) = node(&doc, 1);
    let DocElement::Frame(frame) = kid(&doc, one, DocKind::Frame, 0).1 else { panic!("a frame") };
    assert_eq!(
        frame.style.start,
        Some(StartPoint {
            edge: crate::document::values::Edge::Top,
            at: 0.75
        })
    );
    assert!(save(&doc).contains("\"at\": 0.75"));
}

#[test]
fn test_the_fit_spellings_read_and_the_bare_fit_means_the_built_in_bounds() {
    let fit = |written: &str| -> Option<FitMode> {
        let doc = load_node(&format!(r#","frame":{{"fit":{written}}}"#)).unwrap();
        let (one, _) = node(&doc, 1);
        match kid(&doc, one, DocKind::Frame, 0).1 {
            DocElement::Frame(frame) => frame.style.fit.clone(),
            _ => panic!("a frame"),
        }
    };
    assert_eq!(fit(r#""repeat""#), Some(FitMode::Repeat));
    assert_eq!(fit(r#""spread""#), Some(FitMode::Spread));
    assert_eq!(fit(r#""once""#), Some(FitMode::Once { filler: None }));
    assert_eq!(
        fit(r#"{"once":{"filler":"·"}}"#),
        Some(FitMode::Once {
            filler: Some("·".to_string())
        })
    );
    assert_eq!(fit(r#""fit""#), Some(FitMode::Fit { min: 0.5, max: 2.0 }));
    assert_eq!(
        fit(r#"{"fit":{"min":0.5,"max":2.0}}"#),
        Some(FitMode::Fit { min: 0.5, max: 2.0 }),
        "the bare form and the explicit bounds are one value"
    );
    assert_eq!(
        fit(r#"{"fit":{"min":0.25,"max":4}}"#),
        Some(FitMode::Fit { min: 0.25, max: 4.0 })
    );
    // The built-in bounds are written as the bare form.
    let doc = load_node(r#","frame":{"fit":{"fit":{"min":0.5,"max":2}}}"#).unwrap();
    assert!(save(&doc).contains("\"fit\": \"fit\""));
}

#[test]
fn test_a_color_reads_in_every_spelling_the_schema_lists() {
    let fill = |written: &str| -> Option<ColorRef> {
        let doc = load_node(&format!(r#","fill":"{written}""#)).unwrap();
        node(&doc, 1).1.style.fill
    };
    assert_eq!(
        fill("#102030"),
        Some(ColorRef::Rgba(Rgba::opaque(0x10, 0x20, 0x30)))
    );
    assert_eq!(
        fill("#10203040"),
        Some(ColorRef::Rgba(Rgba::new(0x10, 0x20, 0x30, 0x40)))
    );
    assert_eq!(fill("var(fg)"), Some(ColorRef::Var(VarId(1))));
    assert_eq!(fill("palette(fill)"), Some(ColorRef::Palette(PaletteField::Fill)));
    assert_eq!(
        fill("palette(frame)"),
        Some(ColorRef::Palette(PaletteField::Frame))
    );
    assert_eq!(fill("palette(text)"), Some(ColorRef::Palette(PaletteField::Text)));
    assert_eq!(
        fill("palette(title)"),
        Some(ColorRef::Palette(PaletteField::Title))
    );
    // Hex digits in either case read to one value; it saves in lowercase.
    assert_eq!(fill("#ABCDEF"), fill("#abcdef"));
    let doc = load_node(r##","fill":"#ABCDEF""##).unwrap();
    assert!(save(&doc).contains("\"fill\": \"#abcdef\""));
    // Opaque is written without alpha digits.
    let doc = load_node(r##","fill":"#102030ff""##).unwrap();
    assert!(save(&doc).contains("\"fill\": \"#102030\""));
}

#[test]
fn test_a_font_reads_by_family_name_and_saves_by_family_name() {
    let doc = load_node(r#","font":"Public Pixel","sections":[{"align":"left"}]"#).unwrap();
    let (one, data) = node(&doc, 1);
    assert_eq!(data.style.text.font, Some(AppFont::PublicPixel));
    let DocElement::Section(section) = kid(&doc, one, DocKind::Section, 0).1 else { panic!("a section") };
    assert_eq!(section.align, Some(Align::Left));
    assert!(save(&doc).contains("\"font\": \"Public Pixel\""));
}

#[test]
fn test_a_link_may_name_a_node_that_comes_later() {
    let doc = load(&map_with(
        "",
        &format!(
            "{},{}",
            node_of(1, r#","links":[{"to":9,"kind":"line"}]"#),
            node_of(9, "")
        ),
    ))
    .unwrap();
    let (one, _) = node(&doc, 1);
    let (nine, _) = node(&doc, 9);
    let DocElement::Link(link) = kid(&doc, one, DocKind::Link, 0).1 else { panic!("a link") };
    assert_eq!(link.to, nine, "the link leads to the arena node of id 9");
}

#[test]
fn test_every_element_of_a_loaded_map_has_had_its_row_checked() {
    // The loader runs `validate_shape` for every element it builds, the
    // map itself included: the count of rows checked is the count of
    // elements in the tree, for every fixture.
    use crate::document::format::load_counting_checks;
    use crate::document::tests::format_support::FIXTURES;
    let mut total = 0;
    for (name, text) in FIXTURES {
        let (doc, checked) = load_counting_checks(text).unwrap();
        let elements = doc.tree().descendants().count();
        assert_eq!(checked, elements, "{name}: a row went unchecked");
        total += elements;
    }
    assert!(total > 150, "precondition: {total} elements across the corpus");
}
