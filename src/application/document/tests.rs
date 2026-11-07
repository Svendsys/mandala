// SPDX-License-Identifier: MPL-2.0

//! The D28 helper: [`MindMapDocument::begin`] holds the one open
//! transaction, [`MindMapDocument::apply`] reports what it refuses in
//! one place, and the handle is forgotten once the writer closed what
//! it named.

use baumhard::document::mutation::compose;
use baumhard::document::writer::error::ApplyError;
use baumhard::document::writer::transaction::TransactionKind;
use baumhard::document::writer::{ChangeSource, EditRequest};

use super::{MindMapDocument, TRANSACTION_OPEN_STATUS};

/// A request that creates a root node: a real edit the writer accepts
/// whenever no transaction holds the document.
fn create_root(doc: &MindMapDocument) -> EditRequest {
    EditRequest {
        steps: compose::create_root(doc.doc(), glam::Vec2::new(10.0, 10.0)).expect("a root composes"),
        label: "test: create root".to_string(),
    }
}

/// **An open interactive transaction refuses every other change, and the
/// refusal is reported once, on the status line.** `begin` holds the
/// handle; `apply` while it is open returns `TransactionOpen`, leaves the
/// revision where it was and sets [`TRANSACTION_OPEN_STATUS`]; a second
/// `begin` is refused the same way.
///
/// Control: the same request applies before the transaction opens, and
/// clears the status line when it does — so a writer that refused
/// everything, or a helper that never set the status, fails here.
#[test]
fn test_an_open_transaction_refuses_an_apply_and_says_so_on_the_status_line() {
    let mut doc = MindMapDocument::new_blank(None, 50);
    let req = create_root(&doc);
    let applied = doc
        .apply(req, ChangeSource::Action)
        .expect("the control: no transaction is open");
    assert_eq!(doc.revision(), applied);
    assert_eq!(doc.status(), None, "an accepted change leaves no status");

    assert!(
        doc.begin(ChangeSource::Input, TransactionKind::Interactive),
        "begin opens one"
    );
    assert!(doc.transaction.is_some(), "and the helper holds its handle");
    let req = create_root(&doc);
    assert_eq!(
        doc.apply(req, ChangeSource::Action),
        Err(ApplyError::TransactionOpen),
        "the open transaction refuses the apply"
    );
    assert_eq!(doc.revision(), applied, "the refused apply changed nothing");
    assert_eq!(doc.status(), Some(TRANSACTION_OPEN_STATUS), "and said so");

    doc.status = None;
    assert!(
        !doc.begin(ChangeSource::Input, TransactionKind::Interactive),
        "a second begin is refused while one is open"
    );
    assert_eq!(
        doc.status(),
        Some(TRANSACTION_OPEN_STATUS),
        "and says so the same way"
    );
    assert!(doc.transaction.is_some(), "the first handle is still held");
}

/// **An undo cancels an open interactive transaction and the helper
/// forgets the stale handle** (ruling R36), so the next `begin` and the
/// next `apply` are accepted rather than refused by a transaction that
/// no longer exists.
///
/// Control: before the undo the handle is held and an apply is refused,
/// so a helper that dropped the handle at `begin` fails the first
/// assertion, and one that never dropped it fails the last two.
#[test]
fn test_an_undo_closes_the_transaction_and_the_helper_forgets_its_handle() {
    let mut doc = MindMapDocument::new_blank(None, 50);
    assert!(doc.begin(ChangeSource::Input, TransactionKind::Interactive));
    assert!(doc.transaction.is_some(), "control: the handle is held");
    let req = create_root(&doc);
    assert!(
        doc.apply(req, ChangeSource::Action).is_err(),
        "control: an apply is refused"
    );

    doc.undo();
    assert!(
        doc.transaction.is_none(),
        "the undo closed it and the helper forgot the handle"
    );
    let req = create_root(&doc);
    assert!(
        doc.apply(req, ChangeSource::Action).is_ok(),
        "nothing holds the document any more"
    );
    assert!(
        doc.begin(ChangeSource::Input, TransactionKind::Interactive),
        "and a new one opens"
    );
}
