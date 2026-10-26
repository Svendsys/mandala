// SPDX-License-Identifier: MPL-2.0

//! The predicate surface: what a `Predicate<DocElement>` sees of an
//! element, how each comparator reads a field, and how a field is
//! spelled in JSON.

use crate::document::element::{DocElement, DocKind};
use crate::document::inspect::DocField;
use crate::document::tests::test_support::example;
use crate::document::values::{ColorRef, FitMode, FontSize, PaletteField, Rgba, Shape};
use crate::engine::predicate::{Comparator, Inspect, Predicate};

#[test]
fn test_document_inspect_kind_and_channel() {
    do_document_inspect_kind_and_channel();
}

/// Kind and channel answer on every element; a predicate on kind picks
/// out the nodes of a row.
pub fn do_document_inspect_kind_and_channel() {
    let (builder, ids) = example();
    let tree = builder.tree();
    let is_node = Predicate::<DocElement> {
        fields: vec![(DocField::Kind(DocKind::Node), Comparator::equals())],
        always_match: false,
    };
    let nodes: Vec<_> = ids
        .a
        .children(tree.arena())
        .filter(|&c| is_node.test(tree.arena()[c].get()))
        .collect();
    assert_eq!(nodes, vec![ids.b]);
    let target = tree.arena()[ids.portal_target].get();
    assert_eq!(
        target.test_field(&DocField::Channel(1), &Comparator::equals()),
        Some(true)
    );
    assert_eq!(
        target.test_field(&DocField::Channel(0), &Comparator::greater()),
        Some(true)
    );
    assert_eq!(
        target.test_field(&DocField::Kind(DocKind::Node), &Comparator::not_equals()),
        Some(true)
    );
}

#[test]
fn test_document_inspect_a_field_the_kind_lacks_falls_through() {
    do_document_inspect_a_field_the_kind_lacks_falls_through();
}

/// A section has no opacity: the pair answers `None`, and the predicate
/// moves on to its next pair. Control: on a node the same pair answers.
pub fn do_document_inspect_a_field_the_kind_lacks_falls_through() {
    let (builder, ids) = example();
    let tree = builder.tree();
    let section = tree.arena()[ids.a_title].get();
    let node = tree.arena()[ids.a].get();
    assert_eq!(
        section.test_field(&DocField::Opacity(1.0), &Comparator::equals()),
        None
    );
    assert_eq!(
        node.test_field(&DocField::Opacity(1.0), &Comparator::equals()),
        Some(true)
    );
    let either = Predicate::<DocElement> {
        fields: vec![
            (DocField::Opacity(1.0), Comparator::equals()),
            (DocField::Kind(DocKind::Section), Comparator::equals()),
        ],
        always_match: false,
    };
    assert!(either.test(section), "the second pair decides for the section");
}

#[test]
fn test_document_inspect_optional_fields() {
    do_document_inspect_optional_fields();
}

/// An unset optional field: `Exists` fails, `==` fails, `!=` passes,
/// and neither order passes, negated or not. Set, it compares by value.
pub fn do_document_inspect_optional_fields() {
    let (mut builder, ids) = example();
    let unset = builder.tree().arena()[ids.d].get().clone();
    let fade = DocField::FillFade(1.0);
    assert_eq!(unset.test_field(&fade, &Comparator::exists()), Some(false));
    assert_eq!(unset.test_field(&fade, &Comparator::not_exists()), Some(true));
    assert_eq!(unset.test_field(&fade, &Comparator::equals()), Some(false));
    assert_eq!(unset.test_field(&fade, &Comparator::not_equals()), Some(true));
    for order in [
        Comparator::greater(),
        Comparator::less_or_equal(),
        Comparator::less(),
        Comparator::greater_or_equal(),
    ] {
        assert_eq!(unset.test_field(&fade, &order), Some(false), "{order:?}");
    }
    builder.edit_node(ids.d, |n| n.style.fill_fade = Some(2.0));
    let set = builder.tree().arena()[ids.d].get();
    assert_eq!(set.test_field(&fade, &Comparator::exists()), Some(true));
    assert_eq!(set.test_field(&fade, &Comparator::greater()), Some(true));
    assert_eq!(set.test_field(&fade, &Comparator::less()), Some(false));
    assert_eq!(
        set.test_field(&DocField::FillFade(2.0), &Comparator::equals()),
        Some(true)
    );
}

#[test]
fn test_document_inspect_values_are_stored_not_resolved() {
    do_document_inspect_values_are_stored_not_resolved();
}

/// A predicate sees what an element stores: `a`'s palette makes its
/// fill a palette color when resolved, but `a` stores no fill. Colors,
/// shapes and fit modes compare by value and do not order.
pub fn do_document_inspect_values_are_stored_not_resolved() {
    let (mut builder, ids) = example();
    let red = ColorRef::Rgba(Rgba::opaque(255, 0, 0));
    assert_eq!(
        builder.tree().arena()[ids.a]
            .get()
            .test_field(&DocField::Fill(red), &Comparator::exists()),
        Some(false)
    );
    builder.edit_node(ids.a, |n| {
        n.style.fill = Some(red);
        n.style.shape = Some(Shape::Ellipse);
    });
    let a = builder.tree().arena()[ids.a].get();
    assert_eq!(
        a.test_field(&DocField::Fill(red), &Comparator::equals()),
        Some(true)
    );
    let palette_fill = DocField::Fill(ColorRef::Palette(PaletteField::Fill));
    assert_eq!(a.test_field(&palette_fill, &Comparator::equals()), Some(false));
    assert_eq!(
        a.test_field(&DocField::Fill(red), &Comparator::greater()),
        Some(false)
    );
    assert_eq!(
        a.test_field(&DocField::Shape(Shape::Ellipse), &Comparator::equals()),
        Some(true)
    );
    assert_eq!(
        a.test_field(&DocField::Palette(0), &Comparator::equals()),
        Some(true)
    );
    assert_eq!(
        a.test_field(&DocField::Ident(1), &Comparator::equals()),
        Some(true)
    );
    let ring = builder.tree().arena()[ids.ring].get();
    assert_eq!(
        ring.test_field(&DocField::Fit(FitMode::Repeat), &Comparator::exists()),
        Some(false)
    );
}

#[test]
fn test_document_inspect_text_fields_across_kinds() {
    do_document_inspect_text_fields_across_kinds();
}

/// `FontSize` and `Color` read a text style's fields, a path's flow
/// size (in points) and color, and a portal end's; sizes order only
/// within one unit.
pub fn do_document_inspect_text_fields_across_kinds() {
    let (mut builder, ids) = example();
    builder.edit(ids.b_connector, |e| {
        if let DocElement::Connector(c) = e {
            c.path.flow.size = Some(12.0);
        }
    });
    builder.edit(ids.ring, |e| {
        if let DocElement::Ring(r) = e {
            r.style.text.size = Some(FontSize::Rel(0.1));
        }
    });
    let tree = builder.tree();
    let connector = tree.arena()[ids.b_connector].get();
    assert_eq!(
        connector.test_field(&DocField::FontSize(FontSize::Pt(12.0)), &Comparator::equals()),
        Some(true)
    );
    assert_eq!(
        connector.test_field(&DocField::FontSize(FontSize::Pt(10.0)), &Comparator::greater()),
        Some(true)
    );
    assert_eq!(
        connector.test_field(&DocField::Weight(400), &Comparator::exists()),
        None,
        "a path has no weight"
    );
    let ring = tree.arena()[ids.ring].get();
    assert_eq!(
        ring.test_field(&DocField::FontSize(FontSize::Rel(0.05)), &Comparator::greater()),
        Some(true)
    );
    assert_eq!(
        ring.test_field(&DocField::FontSize(FontSize::Pt(0.05)), &Comparator::greater()),
        Some(false)
    );
    assert_eq!(
        ring.test_field(&DocField::FontSize(FontSize::Pt(0.1)), &Comparator::not_equals()),
        Some(true)
    );
    let end = tree.arena()[ids.portal_target].get();
    assert_eq!(
        end.test_field(
            &DocField::Color(ColorRef::Rgba(Rgba::opaque(0, 0, 0))),
            &Comparator::exists()
        ),
        Some(false)
    );
}

#[test]
fn test_document_field_json_spelling() {
    do_document_field_json_spelling();
}

/// Predicates are serialized: fields and kinds in `snake_case`, and a
/// predicate survives a round trip.
pub fn do_document_field_json_spelling() {
    assert_eq!(
        serde_json::to_string(&DocKind::PortalEnd).unwrap(),
        "\"portal_end\""
    );
    assert_eq!(
        serde_json::to_string(&DocField::Kind(DocKind::CapStart)).unwrap(),
        r#"{"kind":"cap_start"}"#
    );
    assert_eq!(
        serde_json::to_string(&DocField::FillFade(1.5)).unwrap(),
        r#"{"fill_fade":1.5}"#
    );
    assert_eq!(
        serde_json::to_string(&DocField::Color(ColorRef::Palette(PaletteField::Title))).unwrap(),
        r#"{"color":{"palette":"title"}}"#
    );
    assert_eq!(
        serde_json::to_string(&DocField::Shape(Shape::Rectangle {
            radius: crate::document::values::Percent::new(25)
        }))
        .unwrap(),
        r#"{"shape":{"rectangle":{"radius":25}}}"#
    );
    let predicate: Predicate<DocElement> = serde_json::from_str(
        r#"{"fields":[[{"kind":"node"},{"Equals":false}],[{"opacity":0.5},{"LessThan":false}]]}"#,
    )
    .unwrap();
    assert_eq!(predicate.fields.len(), 2);
    assert_eq!(predicate.fields[1].0, DocField::Opacity(0.5));
    let shape: Shape = serde_json::from_str(r#"{"rectangle":{"radius":250}}"#).unwrap();
    assert_eq!(
        shape,
        Shape::Rectangle {
            radius: crate::document::values::Percent::new(100)
        },
        "a percentage read from JSON clamps"
    );
}
