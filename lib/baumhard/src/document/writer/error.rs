// SPDX-License-Identifier: MPL-2.0

//! [`ApplyError`]: why the
//! writer refused a change. Every refusal leaves the document exactly as
//! it was.

use crate::document::mutation::error::MutationError;
use crate::document::writer::ChangeSource;
use std::fmt;

/// Why [`Document::apply`](crate::document::ids::Document::apply), a
/// transaction call or [`Document::begin`](crate::document::ids::Document::begin)
/// refused. The document is unchanged after every one of them: nothing
/// was applied, or everything applied was undone.
#[derive(Clone, Debug, PartialEq)]
pub enum ApplyError {
    /// An interactive transaction is open — a text editor session, a
    /// drag or a resize, the color picker, a frame preview — and the
    /// change would cut across it (D28). The caller shows a short status
    /// message; the change is not queued.
    TransactionOpen,
    /// The transaction named is not the open one: it was committed,
    /// cancelled, cancelled by an undo or a redo, or fast-forwarded and
    /// committed when another change arrived during its animation.
    TransactionClosed,
    /// [`Document::interpolate`](crate::document::ids::Document::interpolate)
    /// on a transaction that is not animated.
    NotAnimated,
    /// A change declared a source no change may declare:
    /// [`ChangeSource::Undo`] and [`ChangeSource::Redo`] belong to the
    /// history's own moves, [`ChangeSource::Presentation`] to the
    /// presentation layer, which never enters the history.
    Source(ChangeSource),
    /// The request failed validation, before its walks or after them
    /// (`mutation::validate`, `validate_applied`), or one of its walks
    /// failed. Names the element and what is wrong with it.
    Invalid(MutationError),
    /// The request inserts a node, and no node id is left to issue: the
    /// counter has reached
    /// [`MAX_NODE_IDENT`](crate::document::bounds::MAX_NODE_IDENT). An id
    /// is never wrapped, saturated or issued twice.
    IdentSpaceExhausted {
        /// The highest id a node may carry.
        max: u32,
    },
    /// The change would leave the document with more elements than this
    /// target holds ([`MAX_ELEMENTS`](crate::document::format::MAX_ELEMENTS),
    /// wasm32 only): the same bound a load is held to.
    TooManyElements {
        /// How many elements the document would hold.
        elements: u64,
        /// The most it may.
        max: u64,
    },
    /// The change would leave the document with more bytes of text than
    /// this target holds ([`MAX_MAP_BYTES`](crate::document::format::MAX_MAP_BYTES),
    /// wasm32 only).
    TooMuchText {
        /// How many bytes of text the document would hold.
        bytes: u64,
        /// The most it may.
        max: u64,
    },
}

impl fmt::Display for ApplyError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ApplyError::TransactionOpen => {
                f.write_str("another change is in progress; finish or cancel it first")
            }
            ApplyError::TransactionClosed => f.write_str("the change in progress has already ended"),
            ApplyError::NotAnimated => f.write_str("the change in progress is not an animation"),
            ApplyError::Source(source) => write!(f, "a change cannot declare the source {}", source.name()),
            ApplyError::Invalid(error) => write!(f, "{error}"),
            ApplyError::IdentSpaceExhausted { max } => {
                write!(
                    f,
                    "no node id is left to issue: every id up to {max} has been used"
                )
            }
            ApplyError::TooManyElements { elements, max } => write!(
                f,
                "the map would hold {elements} elements; this platform holds at most {max}"
            ),
            ApplyError::TooMuchText { bytes, max } => write!(
                f,
                "the map would hold {bytes} bytes of text; this platform holds at most {max}"
            ),
        }
    }
}

impl std::error::Error for ApplyError {}

impl From<MutationError> for ApplyError {
    fn from(error: MutationError) -> Self {
        ApplyError::Invalid(error)
    }
}
