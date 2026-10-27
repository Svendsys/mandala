// SPDX-License-Identifier: MPL-2.0

//! What a value may be where it sits: the rules that depend on an
//! element's place rather than on the value alone, in one body for
//! everything that judges a document (ruling R14.16 on #236).
//!
//! Two judges hold a document to them. The file format's loader and
//! [`check_savable`](crate::document::format::check_savable) decide what
//! a file may say; the mutation verifier
//! ([`validate`](crate::document::mutation::validate::validate) and
//! [`validate_applied`](crate::document::mutation::validate::validate_applied))
//! decides what an edit may make. Were each to keep its own copy, an
//! edit could make a map the file cannot save, or a file could hold a
//! map no edit may reach. So each rule is a function here, and both
//! call it:
//!
//! - the **unit of a text size** ([`Unit`]): relative inside a frame
//!   and a ring, points everywhere else;
//! - a **span inside a ring** sets no size of its own
//!   ([`sizes_its_own_text`]): the ring sizes its text;
//! - a **font** a document holds has a family name a file can write
//!   ([`nameable_font`]);
//! - a **name** an author chose is not empty and holds no control or
//!   bidi character ([`name_fault`]);
//! - a **channel** is within
//!   [`MAX_CHANNEL`](crate::document::bounds::MAX_CHANNEL)
//!   ([`check_channel`](crate::document::bounds::check_channel), which
//!   both already call).

use crate::document::element::DocKind;
use crate::document::values::FontSize;
use crate::font::fonts::{family_name_of, AppFont};

/// What a text size may be where it sits.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Unit {
    /// Points: everything outside a frame.
    Points,
    /// A fraction of the node's shorter side: a frame and a ring.
    Relative,
    /// Either: a span whose parent is not known yet — an edit's
    /// payload before the walk places it. Judged again once it is.
    Either,
}

impl Unit {
    /// The unit of the text style of an element of `kind`, or of the
    /// map's defaults for that kind: relative for a frame and a ring,
    /// points for everything else. O(1).
    pub fn of_tier(kind: DocKind) -> Unit {
        match kind {
            DocKind::Frame | DocKind::Ring => Unit::Relative,
            _ => Unit::Points,
        }
    }

    /// The unit of the text style of an element of `kind` placed under
    /// an element of kind `parent`, or `None` when its place is not
    /// known yet: a span's is its tier's, points, once it has a parent
    /// (a span inside a ring sets no size at all,
    /// [`sizes_its_own_text`]), and [`Unit::Either`] before. O(1).
    pub fn of_place(kind: DocKind, parent: Option<DocKind>) -> Unit {
        match (kind, parent) {
            (DocKind::Span, None) => Unit::Either,
            _ => Unit::of_tier(kind),
        }
    }

    /// Whether `size` is in a unit this place takes. O(1).
    pub fn admits(self, size: FontSize) -> bool {
        matches!(
            (self, size),
            (Unit::Either, _) | (Unit::Points, FontSize::Pt(_)) | (Unit::Relative, FontSize::Rel(_))
        )
    }
}

/// Whether an element of `kind` under an element of kind `parent` may
/// set a text size of its own: every element but a span inside a ring,
/// whose ring sizes its text, as a fraction of the node. O(1).
pub fn sizes_its_own_text(kind: DocKind, parent: Option<DocKind>) -> bool {
    !(kind == DocKind::Span && parent == Some(DocKind::Ring))
}

/// Whether a document may hold `font` as an explicit face: one with a
/// family name a file can write. Not the fallback face,
/// [`AppFont::Any`], and not a face this build cannot name — a file
/// could not say which it was, so a map holding it could not be saved.
/// O(the compiled-in families, about 30): a scan of a cached table.
pub fn nameable_font(font: AppFont) -> bool {
    font != AppFont::Any && family_name_of(font).is_some()
}

/// Why a name an author chose cannot be one (design B.8).
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum NameFault {
    /// The name is empty.
    Empty,
    /// The name holds a control character (C0, DEL or C1) or a bidi
    /// embedding, override or isolate control (U+202A–U+202E,
    /// U+2066–U+2069): a character that makes it display as something
    /// else in a menu, a terminal or an error.
    NotPrintable(char),
}

/// Whether `name` may name a theme variant, a theme variable, a palette,
/// or an edit, presentation or verb: `None` when it may, else why not.
/// A name is any non-empty string without the characters
/// [`NameFault::NotPrintable`] lists; it is compared byte for byte and
/// never normalized, so `"My Palette"` is a name and two names that
/// differ only in normalization are two. O(length of the name).
pub fn name_fault(name: &str) -> Option<NameFault> {
    if name.is_empty() {
        return Some(NameFault::Empty);
    }
    name.chars()
        .find(|c| c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'))
        .map(NameFault::NotPrintable)
}
