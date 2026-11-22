// SPDX-License-Identifier: MPL-2.0

//! Installed font families a map names (#329): the report of where it
//! names each ([`installed_families`], [`missing_families`]) and the one
//! undoable edit that replaces one ([`replace_steps`]). Both over a chain
//! as deep as memory allows are `deep_tests`'
//! `do_document_deep_chain_reports_and_replaces_an_installed_family`,
//! which the wasm32 probe and the benches share.

use crate::arena::NodeId;
use crate::document::fonts::{installed_families, missing_families, replace_steps, NamedFamily};
use crate::document::format::{self, load};
use crate::document::ids::Document;
use crate::document::mutation::error::MutationError;
use crate::document::values::{FamilyName, FontRef};
use crate::document::writer::{ChangeSource, EditRequest};
use crate::font::fonts::AppFont;

/// A map that names the installed family `Nope Sans` on a node, a
/// section, two spans and the defaults' span style, `Other Missing` on
/// one span, and the compiled `Public Pixel` on another; an edit and a
/// presentation name `Nope Sans` too. No machine this runs on has either
/// installed family.
const MAP: &str = r#"{"format":1,"name":"fonts",
  "defaults":{"span":{"font":"Nope Sans"}},
  "edits":[{"id":"e","name":"E","reach":"subtree","target":"spans","mutations":[{"span":{"text":{"set_font":"Nope Sans"}}}]}],
  "presentations":[{"id":"p","name":"P","play":"hold","target":"spans","mutations":[{"span":{"text":{"set_font":"Nope Sans"}}}]}],
  "nodes":[
    {"id":1,"position":[0.0,0.0],"font":"Nope Sans","sections":[
      {"font":"Nope Sans","spans":[{"text":"a","font":"Nope Sans"},{"text":"b","font":"Other Missing"}]},
      {"spans":[{"text":"c","font":"Nope Sans"},{"text":"d","font":"Public Pixel"}]}]}]}"#;

/// The installed family `name`.
fn family(name: &str) -> FamilyName {
    match format::font(name) {
        Ok(FontRef::Installed(family)) => family,
        other => panic!("precondition: {name:?} is an installed family, not {other:?}"),
    }
}

/// Every element's own font, and the defaults' span font, in tree order.
fn fonts(doc: &Document) -> Vec<(NodeId, Option<FontRef>)> {
    let mut out: Vec<(NodeId, Option<FontRef>)> = doc
        .tree()
        .descendants()
        .filter_map(|id| Some((id, doc.get(id)?.text_style()?.font.clone())))
        .collect();
    out.push((doc.root(), doc.defaults().span.font.clone()));
    out
}

/// The report's rows, as `(family, elements, definitions)`.
fn rows(named: &[NamedFamily]) -> Vec<(String, usize, Vec<String>)> {
    named
        .iter()
        .map(|named| {
            (
                named.family.to_string(),
                named.elements,
                named.definitions.clone(),
            )
        })
        .collect()
}

#[test]
fn test_the_report_names_every_installed_family_and_counts_where_the_map_names_it() {
    // Exact counts: `Nope Sans` on the node, the first section, two
    // spans and the defaults' span style is 5; `Other Missing` on one
    // span is 1; the compiled family is not listed; the edit and the
    // presentation that name `Nope Sans` are listed by id. Neither family
    // is installed anywhere this runs, so the missing report is the same.
    // Controls: counting only spans turns the first count red at 2;
    // skipping the defaults turns it red at 4; listing a compiled family
    // adds a row; not walking the presentations drops its id.
    let doc = load(MAP).expect("the map loads");
    let expected = vec![
        (
            "Nope Sans".to_string(),
            5,
            vec!["edit `e`".to_string(), "presentation `p`".to_string()],
        ),
        ("Other Missing".to_string(), 1, Vec::new()),
    ];
    assert_eq!(rows(&installed_families(&doc)), expected);
    assert_eq!(
        rows(&missing_families(&doc)),
        expected,
        "neither is installed here"
    );
}

#[test]
fn test_a_map_that_names_only_compiled_families_reports_none() {
    // Control: the same report on a map without an installed family is
    // empty, so the rows above come from what the map names; and its own
    // control, the tree walk counting compiled faces under their names
    // too, turns the first arm red.
    let doc = load(r#"{"format":1,"name":"f","nodes":[{"id":1,"position":[0.0,0.0],"font":"Public Pixel","sections":[{"spans":[{"text":"a","font":"Galatia SIL"}]}]}]}"#)
        .expect("the map loads");
    assert!(installed_families(&doc).is_empty());
    assert!(missing_families(&doc).is_empty());
}

#[test]
fn test_a_replacement_rewrites_every_reference_as_one_edit_that_undo_reverts() {
    // #329: picking a replacement rewrites the map's references to the
    // family — every element that names it in its own style, and the
    // defaults — as one request: one history entry, whose undo restores
    // every reference exactly and whose redo makes them the replacement
    // again. Nothing else is touched: `Other Missing` and `Public Pixel`
    // keep their spans. The edit and the presentation still name the
    // family (design §5.7; the owner's ruling on #329 is pending), so the
    // report lists it with no element and both definitions.
    //
    // Controls: `replace_steps` without the defaults' step turns the
    // defaults arm red (still `Nope Sans`); with one request per kind
    // instead of one request, the history grows by 3 and turns its arm
    // red; a step that sets every element's font turns the untouched arm
    // red.
    let mut doc = load(MAP).expect("the map loads");
    let before = fonts(&doc);
    let history = doc.history().count();
    let from = family("Nope Sans");
    let to = FontRef::Compiled(AppFont::PublicPixel);
    let named = before
        .iter()
        .filter(|(_, font)| font.as_ref().and_then(FontRef::installed) == Some(&from))
        .count();
    assert_eq!(named, 5, "precondition: the map names the family five times");
    let steps = replace_steps(&doc, &from, &to).expect("the steps");
    doc.apply(EditRequest::new(steps, "font replace"), ChangeSource::Action)
        .expect("the replacement applies");
    let after = fonts(&doc);
    assert_eq!(doc.history().count(), history + 1, "one undo entry");
    for ((id, was), (_, now)) in before.iter().zip(&after) {
        if was.as_ref().and_then(FontRef::installed) == Some(&from) {
            assert_eq!(
                now.as_ref(),
                Some(&to),
                "{id:?}: the reference is the replacement"
            );
        } else {
            assert_eq!(
                now, was,
                "{id:?}: an element that named another face is untouched"
            );
        }
    }
    assert_eq!(
        rows(&installed_families(&doc)),
        vec![
            (
                "Nope Sans".to_string(),
                0,
                vec!["edit `e`".to_string(), "presentation `p`".to_string()]
            ),
            ("Other Missing".to_string(), 1, Vec::new()),
        ],
        "the definitions still name it, which the replacement leaves"
    );
    doc.undo().expect("the replacement undoes");
    assert_eq!(fonts(&doc), before, "undo restores every reference");
    doc.redo().expect("the replacement redoes");
    assert_eq!(fonts(&doc), after, "redo makes them the replacement again");
}

#[test]
fn test_a_replacement_may_be_an_installed_family_and_never_the_default_face() {
    // A replacement is any family a map can name: another installed one
    // is, the default face is not (a file cannot name it), and replacing
    // a family with itself, or one the map names nowhere, is no step.
    // Controls: `replace_steps` not refusing a face no file can name turns
    // the default-face arm red; not returning early for a family replaced
    // by itself turns the self-replacement arm red.
    let doc = load(MAP).expect("the map loads");
    let from = family("Nope Sans");
    let other = FontRef::Installed(family("Other Missing"));
    assert!(!replace_steps(&doc, &from, &other).expect("the steps").is_empty());
    assert!(matches!(
        replace_steps(&doc, &from, &FontRef::DEFAULT),
        Err(MutationError::AnyFont { .. })
    ));
    assert!(replace_steps(&doc, &from, &FontRef::Installed(from.clone()))
        .expect("the steps")
        .is_empty());
    assert!(replace_steps(&doc, &family("Named Nowhere"), &other)
        .expect("the steps")
        .is_empty());
}
