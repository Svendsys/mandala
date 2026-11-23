// SPDX-License-Identifier: MPL-2.0

//! The single-line editor's lifecycle, renderer-free (TEST_CONVENTIONS
//! §T8), on every owner it reaches — a label, a portal end's marker and
//! its label, a ring, a connector's and a line link's content, a cap —
//! as a transaction: typing stages, commit is one entry whose saved map
//! is one `replace_spans`, cancel leaves the map byte-identical. A label
//! it opens on that is not there yet is inserted into the transaction's
//! base (ruling K3 on #249); emptied, a label is removed, or canceled
//! away when the session made it; a portal link's label is refused
//! (ruling K4).

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind, LinkData};
use baumhard::document::ids::NodeIdent;
use baumhard::document::mutation::compose::replace_spans;
use baumhard::document::values::LinkKind;
use baumhard::document::writer::EditRequest;

use super::{LineRefusal, LineSession, SingleLineTarget};
use crate::application::app::text_edit::tests::{entries, spans, text_fixture};
use crate::application::app::text_edit::Edit;
use crate::application::document::MindMapDocument;
use crate::application::keybinds::Action;
use crate::application::platform::input::{Key, SmolStr};
use crate::application::view::inspect::children_of_kind;

/// Node `ident`.
fn node(doc: &MindMapDocument, ident: u32) -> NodeId {
    doc.doc()
        .node_by_ident(NodeIdent(ident))
        .expect("the node is in the fixture")
}

/// The first child of `of` of `kind`.
fn child(doc: &MindMapDocument, of: NodeId, kind: DocKind) -> NodeId {
    children_of_kind(doc.doc(), of, kind)
        .next()
        .expect("the fixture holds it")
}

/// Node 1's link of `kind`.
fn link(doc: &MindMapDocument, kind: LinkKind) -> NodeId {
    children_of_kind(doc.doc(), node(doc, 1), DocKind::Link)
        .find(|&l| matches!(doc.doc().get(l), Some(DocElement::Link(LinkData { kind: k, .. })) if *k == kind))
        .expect("the fixture holds the link")
}

/// The text of `owner`'s spans.
fn text(doc: &MindMapDocument, owner: NodeId) -> String {
    spans(doc, owner).iter().map(|s| s.text.as_str()).collect()
}

/// Type `text`, a character at a time, staging each.
fn type_into(session: &mut LineSession, doc: &mut MindMapDocument, text: &str) {
    for c in text.chars() {
        assert_eq!(
            session.type_key(&Key::Character(SmolStr::new(c.to_string()))),
            Edit::Text
        );
        assert!(session.stage(doc));
    }
}

/// Every owner the single-line editor reaches, on the fixture.
fn owners(doc: &MindMapDocument) -> Vec<(&'static str, NodeId)> {
    let connector = child(doc, node(doc, 2), DocKind::Connector);
    let portal = link(doc, LinkKind::Portal);
    let end = child(doc, portal, DocKind::PortalEnd);
    vec![
        ("a connector's label", child(doc, connector, DocKind::Label)),
        ("a connector's content", connector),
        ("a cap", child(doc, connector, DocKind::CapEnd)),
        (
            "a ring",
            child(doc, child(doc, node(doc, 1), DocKind::Frame), DocKind::Ring),
        ),
        ("a line link's content", link(doc, LinkKind::Line)),
        ("a portal end's marker", end),
        ("a portal end's label", child(doc, end, DocKind::Label)),
    ]
}

/// **On every owner, typing and committing is one entry, the map one
/// `replace_spans` makes; cancel is byte-identical; undo restores.**
/// Controls: typing made no entry of its own, and changed the owner.
#[test]
fn test_every_owner_edits_as_one_transaction() {
    for (what, owner) in owners(&text_fixture()) {
        let mut doc = text_fixture();
        let before = doc.save_json().expect("saves");
        let was = text(&doc, owner);
        let mut session = LineSession::open(&mut doc, SingleLineTarget::Owner(owner), false).expect(what);
        type_into(&mut session, &mut doc, " edited");
        assert_eq!(entries(&doc), 0, "{what}: control: no entry while typing");
        assert_eq!(
            text(&doc, owner),
            format!("{was} edited"),
            "{what}: the stage is the document"
        );
        let typed = spans(&doc, owner);
        session.commit(&mut doc).expect(what);
        assert_eq!(entries(&doc), 1, "{what}");

        let mut once = text_fixture();
        let steps = replace_spans(once.doc(), &[owner], &typed).expect("composes");
        once.apply_for_test(EditRequest::new(steps, "replace"))
            .expect("applies");
        assert_eq!(
            doc.save_json().expect("saves"),
            once.save_json().expect("saves"),
            "{what}"
        );
        doc.undo();
        assert_eq!(doc.save_json().expect("saves"), before, "{what}: undo restores");

        let mut session = LineSession::open(&mut doc, SingleLineTarget::Owner(owner), true).expect(what);
        type_into(&mut session, &mut doc, "x");
        session.cancel(&mut doc);
        assert_eq!(
            doc.save_json().expect("saves"),
            before,
            "{what}: cancel is byte-identical"
        );
        assert_eq!(
            entries(&doc),
            0,
            "{what}: and makes no entry (the undone one is on the redo side)"
        );
    }
}

/// **A label on a path that has none is inserted when the editor opens**
/// (ruling K3 on #249), in the transaction's base: committed with text, it
/// is one entry holding the label and its text; canceled, or committed
/// empty, the path has no label and the map is byte-identical. Control:
/// while the session is open the label is there.
#[test]
fn test_a_new_label_is_inserted_into_the_session_and_leaves_with_it() {
    let mut doc = text_fixture();
    let path = link(&doc, LinkKind::Line);
    let before = doc.save_json().expect("saves");
    let labels = |doc: &MindMapDocument| children_of_kind(doc.doc(), path, DocKind::Label).count();
    assert_eq!(labels(&doc), 0, "precondition: the line link has no label");

    let target = SingleLineTarget::label_of(doc.doc(), path).expect("a line link takes a label");
    assert_eq!(target, SingleLineTarget::NewLabel { host: path });
    let mut session = LineSession::open(&mut doc, target, false).expect("opens");
    assert!(session.created_label());
    assert_eq!(labels(&doc), 1, "control: the label is there while editing");
    let label = session.owner();
    type_into(&mut session, &mut doc, "new");
    session.commit(&mut doc).expect("commits");
    assert_eq!(entries(&doc), 1, "one entry: the label and its text");
    assert_eq!(text(&doc, label), "new");
    doc.undo();
    assert_eq!(doc.save_json().expect("saves"), before);

    for commit in [false, true] {
        let mut session = LineSession::open(&mut doc, target, false).expect("opens");
        if commit {
            assert_eq!(
                session.commit(&mut doc),
                None,
                "an empty new label commits nothing"
            );
        } else {
            type_into(&mut session, &mut doc, "zap");
            session.cancel(&mut doc);
        }
        assert_eq!(labels(&doc), 0, "commit={commit}: no label is left");
        assert_eq!(doc.save_json().expect("saves"), before);
    }
}

/// **An emptied label that was there before is removed**, as `main`
/// dropped one, in one entry an undo takes back; any other owner emptied
/// keeps no spans and stays. Control: the label is there before.
#[test]
fn test_an_emptied_label_is_removed_and_other_owners_stay() {
    let mut doc = text_fixture();
    let connector = child(&doc, node(&doc, 2), DocKind::Connector);
    let label = child(&doc, connector, DocKind::Label);
    let mut session = LineSession::open(&mut doc, SingleLineTarget::Owner(label), true).expect("opens");
    assert!(session.stage(&mut doc));
    session.commit(&mut doc).expect("commits the removal");
    assert!(doc.doc().get(label).is_none(), "the label is gone");
    assert_eq!(entries(&doc), 1);
    doc.undo();
    assert!(doc.doc().get(label).is_some(), "control: an undo brings it back");

    let cap = child(&doc, connector, DocKind::CapEnd);
    let session = LineSession::open(&mut doc, SingleLineTarget::Owner(cap), true).expect("opens");
    session.commit(&mut doc).expect("commits");
    assert!(
        doc.doc().get(cap).is_some() && spans(&doc, cap).is_empty(),
        "a cap keeps no spans and stays"
    );
}

/// **What the editor opens on**: a press on a ring, a label, a cap, a
/// portal end or a path edits it; a section, a node, a frame and a
/// portal link are none of its owners. A portal link's label is refused
/// with the way in named (ruling K4); a connector's is its own.
#[test]
fn test_what_the_single_line_editor_opens_on() {
    let doc = text_fixture();
    for (what, owner) in owners(&doc) {
        assert_eq!(
            SingleLineTarget::of_element(doc.doc(), owner),
            Some(SingleLineTarget::Owner(owner)),
            "{what}"
        );
    }
    let portal = link(&doc, LinkKind::Portal);
    for not_one in [
        node(&doc, 1),
        child(&doc, node(&doc, 1), DocKind::Section),
        portal,
    ] {
        assert_eq!(SingleLineTarget::of_element(doc.doc(), not_one), None);
    }
    assert_eq!(
        SingleLineTarget::label_of(doc.doc(), portal),
        Err(LineRefusal::PORTAL_LABEL)
    );
    let connector = child(&doc, node(&doc, 2), DocKind::Connector);
    assert_eq!(
        SingleLineTarget::label_of(doc.doc(), connector),
        Ok(SingleLineTarget::Owner(child(&doc, connector, DocKind::Label))),
        "control: a connector's label is its own"
    );
}

/// **The keys are `main`'s**: left, right, home, end, delete back and
/// forward, the space; a control character typed is dropped, and a named
/// key types nothing.
#[test]
fn test_the_single_line_keys_are_mains() {
    let mut doc = text_fixture();
    let owner = owners(&doc)[0].1;
    let mut session = LineSession::open(&mut doc, SingleLineTarget::Owner(owner), false).expect("opens");
    assert_eq!(session.draft().caret(), 5, "at the end of `child`");
    session.apply_action(&Action::LabelEditCursorHome);
    assert_eq!(session.apply_action(&Action::LabelEditDeleteForward), Edit::Text);
    session.apply_action(&Action::LabelEditCursorEnd);
    assert_eq!(session.apply_action(&Action::LabelEditInsertSpace), Edit::Text);
    session.apply_action(&Action::LabelEditCursorLeft);
    assert_eq!(session.apply_action(&Action::LabelEditDeleteBack), Edit::Text);
    assert_eq!(session.type_key(&Key::Character(SmolStr::new("\t"))), Edit::None);
    assert_eq!(
        session.type_key(&Key::Named(crate::application::platform::input::NamedKey::Enter)),
        Edit::None
    );
    assert_eq!(
        session.apply_action(&Action::TextEditCursorLeft),
        Edit::None,
        "another editor's Action"
    );
    session.stage(&mut doc);
    assert_eq!(text(&doc, owner), "hil ");
}
