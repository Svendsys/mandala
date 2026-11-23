// SPDX-License-Identifier: MPL-2.0

//! The section editor's lifecycle, renderer-free (TEST_CONVENTIONS §T8):
//! a session is a transaction — typing stages, commit is one entry whose
//! saved map is the one a single `replace_spans` makes, cancel leaves the
//! map byte-identical, undo restores it — and `CreateOrphanNodeAndEdit`
//! is one transaction holding the node and its text, its identity stable
//! across every stage. The keys go through the session as `main`'s went
//! through its editor: the cursor primitives, the literal path and its
//! IME payloads. What the view and the input ladders add is the GPU
//! tier's (`app::tests_gpu_text`).

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind, SpanData};
use baumhard::document::ids::NodeIdent;
use baumhard::document::mutation::compose::replace_spans;
use baumhard::document::writer::transaction::TransactionKind;
use baumhard::document::writer::{ChangeSource, EditRequest};
use glam::Vec2;

use super::session::EditRefusal;
use super::{typed_payload, Edit, TextSession};
use crate::application::document::{Interaction, MindMapDocument};
use crate::application::keybinds::Action;
use crate::application::platform::input::{Key, NamedKey, SmolStr};
use crate::application::view::inspect::children_of_kind;

/// The editors' fixture: node 1 of two sections — "Head" plain and
/// "line" bold, then "Body" — with a ring, a line link and a portal
/// link; node 2, its child, with a connector, a cap and a label; node 3.
pub(crate) fn text_fixture() -> MindMapDocument {
    let path = format!(
        "{}/src/application/app/dispatch/families/fixtures/text.mandala.json",
        env!("CARGO_MANIFEST_DIR")
    );
    MindMapDocument::load(&path, baumhard::document::writer::DEFAULT_HISTORY_LIMIT)
        .expect("the fixture loads")
}

/// Node `ident`'s section `index`.
pub(crate) fn section(doc: &MindMapDocument, ident: u32, index: usize) -> NodeId {
    let node = doc
        .doc()
        .node_by_ident(NodeIdent(ident))
        .expect("the node is in the fixture");
    children_of_kind(doc.doc(), node, DocKind::Section)
        .nth(index)
        .expect("the section is in the fixture")
}

/// The span children of `owner`.
pub(crate) fn spans(doc: &MindMapDocument, owner: NodeId) -> Vec<SpanData> {
    doc.doc()
        .children(owner)
        .filter_map(|id| match doc.doc().get(id) {
            Some(DocElement::Span(span)) => Some(span.clone()),
            _ => None,
        })
        .collect()
}

/// The text of `owner`'s spans.
fn text(doc: &MindMapDocument, owner: NodeId) -> String {
    spans(doc, owner).iter().map(|s| s.text.as_str()).collect()
}

/// How many entries the history holds.
pub(crate) fn entries(doc: &MindMapDocument) -> usize {
    doc.doc().history().count()
}

/// A `Key::Character` of `text`.
fn ch(text: &str) -> Key {
    Key::Character(SmolStr::new(text))
}

/// Type `text` into `session`, a character at a time, staging each, as
/// the keyboard does.
fn type_into(session: &mut TextSession, doc: &mut MindMapDocument, text: &str) {
    for c in text.chars() {
        assert_eq!(session.type_key(None, &ch(&c.to_string())), Edit::Text);
        assert!(session.stage(doc), "the stage of {c:?} is accepted");
    }
}

/// **Typing 20 characters and committing is one history entry, and the
/// map it saves is the map one `replace_spans` makes** — the session's
/// twenty stages left nothing behind but the last. Undo puts the map back
/// exactly. Controls: the twenty stages left the history alone until the
/// commit, and the text did change.
#[test]
fn test_twenty_characters_then_commit_is_one_entry_and_one_replace_spans() {
    let mut doc = text_fixture();
    let before = doc.save_json().expect("saves");
    let owner = section(&doc, 1, 0);
    let mut session = TextSession::open(&mut doc, owner, false, false).expect("opens");
    type_into(&mut session, &mut doc, "twenty characters!!!");
    assert_eq!(entries(&doc), 0, "control: typing makes no entry of its own");
    assert_eq!(
        text(&doc, owner),
        "Headlinetwenty characters!!!",
        "the stage is the document"
    );
    let typed = spans(&doc, owner);
    session.commit(&mut doc).expect("a changed session commits");
    assert_eq!(entries(&doc), 1, "one entry for the whole session");

    let mut once = text_fixture();
    let steps = replace_spans(once.doc(), &[section(&once, 1, 0)], &typed).expect("composes");
    once.apply_for_test(EditRequest::new(steps, "replace"))
        .expect("applies");
    assert_eq!(
        doc.save_json().expect("saves"),
        once.save_json().expect("saves"),
        "the session saves what one replace_spans saves"
    );
    assert_ne!(
        doc.save_json().expect("saves"),
        before,
        "control: the map changed"
    );

    doc.undo();
    assert_eq!(
        doc.save_json().expect("saves"),
        before,
        "undo restores the map exactly"
    );
}

/// **Cancel leaves the map byte-identical, with no entry**, after typing,
/// deleting and the clean opening's clear. Control: before the cancel the
/// document held the typing.
#[test]
fn test_cancel_leaves_the_map_byte_identical_with_no_entry() {
    for clean in [false, true] {
        let mut doc = text_fixture();
        let before = doc.save_json().expect("saves");
        let revision = doc.revision();
        let owner = section(&doc, 1, 0);
        let mut session = TextSession::open(&mut doc, owner, clean, false).expect("opens");
        type_into(&mut session, &mut doc, "abc");
        assert_eq!(session.apply_action(&Action::TextEditDeleteBack), Edit::Text);
        session.stage(&mut doc);
        assert_ne!(
            text(&doc, owner),
            "Headline",
            "control: the stage held the typing"
        );
        session.cancel(&mut doc);
        assert_eq!(doc.save_json().expect("saves"), before, "clean={clean}");
        assert_eq!(entries(&doc), 0);
        assert_eq!(doc.revision(), revision);
        assert_eq!(doc.open_interaction(), None, "the transaction is closed");
    }
}

/// **A session that changes nothing leaves no entry**: committing it
/// cancels, as `main` skipped a dead undo entry — a caret move, and a key
/// typed and deleted again, whose stage replaced the spans with equal
/// ones; a clean opening is a change (it cleared the text), and commits.
/// Control: the same session with a key typed commits one.
#[test]
fn test_an_unchanged_commit_leaves_no_entry_and_a_clean_one_does() {
    let mut doc = text_fixture();
    let owner = section(&doc, 1, 1);
    let mut session = TextSession::open(&mut doc, owner, false, false).expect("opens");
    assert_eq!(session.apply_action(&Action::TextEditCursorLeft), Edit::Caret);
    assert_eq!(session.commit(&mut doc), None);
    assert_eq!(entries(&doc), 0, "a caret move is no change");
    assert_eq!(doc.open_interaction(), None);

    let mut session = TextSession::open(&mut doc, owner, false, false).expect("opens");
    type_into(&mut session, &mut doc, "q");
    assert_eq!(session.apply_action(&Action::TextEditDeleteBack), Edit::Text);
    assert!(session.stage(&mut doc), "a stage of the original spans");
    assert_eq!(session.commit(&mut doc), None);
    assert_eq!(entries(&doc), 0, "typed and deleted again is no change");

    let session = TextSession::open(&mut doc, owner, true, false).expect("opens clean");
    assert!(
        session.commit(&mut doc).is_some(),
        "a clean opening cleared the text"
    );
    assert_eq!(text(&doc, owner), "");

    let mut session = TextSession::open(&mut doc, owner, false, false).expect("opens");
    type_into(&mut session, &mut doc, "x");
    assert!(session.commit(&mut doc).is_some(), "control: a typed key commits");
    assert_eq!(entries(&doc), 2);
}

/// **An undo while the session is open cancels it** (ruling R36): the
/// session is no longer live, and a commit then writes nothing. Control:
/// before the undo it was live.
#[test]
fn test_an_undo_ends_an_open_session_with_nothing_written() {
    let mut doc = text_fixture();
    let before = doc.save_json().expect("saves");
    let owner = section(&doc, 1, 0);
    let mut session = TextSession::open(&mut doc, owner, false, false).expect("opens");
    type_into(&mut session, &mut doc, "zz");
    assert!(session.is_live(&doc), "control: live before the undo");
    doc.undo();
    assert!(!session.is_live(&doc));
    assert_eq!(session.commit(&mut doc), None);
    assert_eq!(doc.save_json().expect("saves"), before);
}

/// **D28: a session does not open while another interaction holds the
/// writer**, and says why. Control: once that interaction ends, it opens.
#[test]
fn test_a_session_is_refused_while_another_interaction_is_open() {
    let mut doc = text_fixture();
    assert!(doc.begin(
        ChangeSource::Input,
        TransactionKind::Interactive,
        Interaction::Test
    ));
    let owner = section(&doc, 1, 0);
    assert_eq!(
        TextSession::open(&mut doc, owner, false, false).err(),
        Some(EditRefusal::BUSY)
    );
    assert_eq!(
        TextSession::open_orphan(&mut doc, Vec2::new(500.0, 500.0)).err(),
        Some(EditRefusal::BUSY)
    );
    assert!(doc.cancel(Interaction::Test));
    assert!(
        TextSession::open(&mut doc, owner, false, false).is_ok(),
        "control"
    );
}

/// **`CreateOrphanNodeAndEdit` is one transaction**: the node is in the
/// base for the whole session — its `NodeId` and its ident the same at
/// every stage (ruling R7 on #238) — commit is one entry holding the node
/// and its text, and cancel leaves no node at all, the map byte-identical.
#[test]
fn test_create_orphan_and_edit_is_one_transaction() {
    let mut doc = text_fixture();
    let before = doc.save_json().expect("saves");
    let nodes =
        |doc: &MindMapDocument| crate::application::document::selection::every_node(doc.doc()).count();
    let count = nodes(&doc);

    let mut session = TextSession::open_orphan(&mut doc, Vec2::new(600.0, 40.0)).expect("opens");
    let (node, owner) = (session.node(), session.section());
    let ident = |doc: &MindMapDocument| match doc.doc().get(node) {
        Some(DocElement::Node(data)) => data.ident,
        _ => panic!("the node is in the document"),
    };
    let first_ident = ident(&doc);
    assert_eq!(nodes(&doc), count + 1, "the node is drawn from the start");
    for c in "orphan".chars() {
        session.type_key(None, &ch(&c.to_string()));
        session.stage(&mut doc);
        assert_eq!(ident(&doc), first_ident, "the ident holds at every stage");
        assert_eq!(
            doc.doc().parent(owner),
            Some(node),
            "and so does the section's NodeId"
        );
    }
    assert_eq!(entries(&doc), 0);
    session.commit(&mut doc).expect("commits");
    assert_eq!(entries(&doc), 1, "one entry: the node and its text");
    assert_eq!(text(&doc, owner), "orphan");
    assert!(doc.selection.contains(node), "the new node is selected");
    doc.undo();
    assert_eq!(
        doc.save_json().expect("saves"),
        before,
        "undone, the node goes with its text"
    );

    let mut session = TextSession::open_orphan(&mut doc, Vec2::new(600.0, 40.0)).expect("opens");
    type_into(&mut session, &mut doc, "gone");
    session.cancel(&mut doc);
    assert_eq!(nodes(&doc), count, "cancel leaves no node");
    assert_eq!(doc.save_json().expect("saves"), before);
}

/// **The cursor primitives are `main`'s**: left and right by a cluster,
/// home and end of the line, up and down a line keeping the column, by
/// word; Backspace and Delete remove a whole cluster; a `*Select` move
/// seeds and keeps the anchor and any other drops it, and a live range
/// becomes the selection on commit — the swept graphemes of the section.
#[test]
fn test_the_cursor_primitives_and_the_shift_select_lift() {
    let mut doc = text_fixture();
    let owner = section(&doc, 3, 0);
    let mut session = TextSession::open(&mut doc, owner, true, false).expect("opens clean");
    type_into(&mut session, &mut doc, "ab");
    session.type_key(Some("enter"), &Key::Named(NamedKey::Enter));
    type_into(&mut session, &mut doc, "cdef");
    assert_eq!(text(&doc, owner), "ab\ncdef");
    let caret = |s: &TextSession| s.draft().caret();
    assert_eq!(caret(&session), 7);
    session.apply_action(&Action::TextEditCursorUp);
    assert_eq!(
        caret(&session),
        2,
        "up keeps the column, held to the line above's end"
    );
    session.apply_action(&Action::TextEditCursorDown);
    assert_eq!(caret(&session), 5);
    session.apply_action(&Action::TextEditCursorHome);
    assert_eq!(caret(&session), 3);
    session.apply_action(&Action::TextEditCursorEnd);
    assert_eq!(caret(&session), 7);
    session.apply_action(&Action::TextEditWordLeft);
    assert_eq!(caret(&session), 3);
    session.apply_action(&Action::TextEditCursorLeftSelect);
    session.apply_action(&Action::TextEditCursorLeftSelect);
    assert_eq!((session.draft().anchor(), caret(&session)), (Some(3), 1));
    assert_eq!(session.apply_action(&Action::TextEditDeleteForward), Edit::Text);
    assert_eq!(session.draft().anchor(), None, "an edit drops the anchor");
    session.stage(&mut doc);
    assert_eq!(text(&doc, owner), "a\ncdef");
    session.apply_action(&Action::TextEditCursorRightSelect);
    session.apply_action(&Action::TextEditCursorRightSelect);
    session.commit(&mut doc).expect("commits");
    assert_eq!(
        doc.selection.range(),
        Some((owner, 1, 3)),
        "the swept range is selected"
    );
}

/// **Literal keys are `main`'s**: Enter a line break and Tab a tab in a
/// paragraph editor; a payload's control characters dropped; an IME
/// payload — a Hangul syllable of three jamo — one cluster, the caret
/// past it; a combining mark typed after its base merges, the caret where
/// it was, the text changed. A named key that carries no text is none.
#[test]
fn test_literal_keys_and_ime_payloads_are_mains() {
    assert_eq!(typed_payload(&ch("a\tb")).as_deref(), Some("ab"));
    assert_eq!(typed_payload(&ch("\u{7}")), None);
    assert_eq!(typed_payload(&Key::Named(NamedKey::ArrowLeft)), None);

    let mut doc = text_fixture();
    let owner = section(&doc, 3, 0);
    let mut session = TextSession::open(&mut doc, owner, true, false).expect("opens clean");
    assert_eq!(
        session.type_key(Some("tab"), &Key::Named(NamedKey::Tab)),
        Edit::Text
    );
    session.type_key(None, &ch("e"));
    assert_eq!(
        session.type_key(None, &ch("\u{0301}")),
        Edit::Text,
        "the mark changes the text"
    );
    assert_eq!(session.draft().caret(), 2, "but not the caret");
    // In the middle of the text, where no clamp at its end could stand in
    // for the count: the syllable is one cluster, the caret past it.
    session.apply_action(&Action::TextEditCursorLeft);
    let jamo = "\u{1112}\u{1161}\u{11AB}";
    session.type_key(None, &ch(jamo));
    assert_eq!(session.draft().caret(), 2, "the syllable is one cluster");
    assert_eq!(
        session.type_key(Some("arrowleft"), &Key::Named(NamedKey::ArrowLeft)),
        Edit::None
    );
    session.stage(&mut doc);
    assert_eq!(text(&doc, owner), format!("\t{jamo}e\u{0301}"));
}
