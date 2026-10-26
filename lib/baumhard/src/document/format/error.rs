// SPDX-License-Identifier: MPL-2.0

//! [`LoadError`](crate::document::format::error::LoadError): why a
//! `.mandala.json` did not load, and where in the file.
//!
//! **Every error names its JSON path** (design §16): the location of
//! the offending value, written the way a reader would navigate to it,
//! `nodes[3].frame.rings[0].spans[2].color`. The only error without
//! one is [`TooLarge`](LoadErrorKind::TooLarge), which is about the
//! whole text rather than a place in it. A
//! [`Syntax`](LoadErrorKind::Syntax) error carries the line and column
//! the reader gave up at, and the path of the value it was in when it
//! did, unless it died on something that has no path (the end of the
//! text, a key that is not a string), in which case the path is empty.
//!
//! **There is one error.** A load is refused at the first thing wrong
//! with it and returns nothing else (§7.2): no partial document, no
//! list of problems, no recovery. Where a file has several, which one
//! is "first" is decided by the order the loader reads it in: the text
//! as a whole, then the map's own keys in schema order, then each node
//! depth first; links are resolved last, because a link may name a node
//! that appears after it.

use crate::document::bounds::BoundError;
use std::fmt;

/// Why a load failed, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadError {
    /// The JSON path of the offending value: `nodes[3].frame.rings[0]`.
    /// A key is written after a `.`, an array position in brackets, and
    /// the first segment has no leading dot. Empty when the error is
    /// about the whole text. For a key that is missing, the path is the
    /// object that lacks it and the key is in the
    /// [kind](LoadErrorKind::MissingKey); for a key that is not allowed,
    /// the path ends in the key itself.
    pub path: String,
    /// What is wrong there.
    pub kind: LoadErrorKind,
}

/// What is wrong at a [`LoadError`]'s path.
#[derive(Clone, Debug, PartialEq)]
pub enum LoadErrorKind {
    /// The text is longer than
    /// [`MAX_MAP_BYTES`](crate::document::format::MAX_MAP_BYTES) allows
    /// on this target. Checked before anything is parsed.
    TooLarge {
        /// The text's length in bytes.
        bytes: u64,
        /// The longest text this target loads.
        max: u64,
    },
    /// The text is not JSON, or nests deeper than the JSON reader
    /// follows (128 levels of array and object, which is a chain of
    /// about 60 nested nodes). `message` says where, by line and column.
    Syntax {
        /// What the reader reported.
        message: String,
    },
    /// An object has a key the schema does not define. The path ends in
    /// the key.
    UnknownKey {
        /// The key.
        key: String,
    },
    /// An object lacks a key the schema requires. The path is the
    /// object.
    MissingKey {
        /// The key.
        key: String,
    },
    /// An object has the same key twice. The path is the object.
    DuplicateKey {
        /// The key.
        key: String,
    },
    /// A value is of the wrong JSON type, or is not one of the
    /// spellings the schema allows for it. `message` is the reader's,
    /// naming what was found and what was expected, with its line and
    /// column.
    Invalid {
        /// What the reader reported.
        message: String,
    },
    /// `format` is not `1`.
    UnsupportedFormat {
        /// The number the file gives.
        found: u64,
    },
    /// A number or a count is outside the bound
    /// [`document::bounds`](crate::document::bounds) puts on it.
    Bound {
        /// Which bound, and the value that broke it.
        error: BoundError,
    },
    /// Two nodes carry the same `id`. The path is the second.
    DuplicateId {
        /// The id.
        id: u32,
    },
    /// Two entries of an `edits`, `presentations` or `verbs` list carry
    /// the same `id`. The path is the second.
    DuplicateDefinition {
        /// The id.
        id: String,
    },
    /// A link's `to` names no node.
    DanglingLink {
        /// The id it names.
        to: u32,
    },
    /// A link's `to` names the node that holds it.
    SelfLink,
    /// A `click`, `hover` or span `link` names a verb the map does not
    /// define.
    UnknownVerb {
        /// The verb id.
        id: String,
    },
    /// A node's `palette` names a palette the map does not define.
    UnknownPalette {
        /// The palette name.
        name: String,
    },
    /// A `var(name)` names a variable the theme does not define.
    UnknownVariable {
        /// The variable name.
        name: String,
    },
    /// A `font` names no font compiled into this build.
    UnknownFont {
        /// The family name.
        family: String,
    },
    /// The theme's `active` names no variant.
    ActiveNotAVariant {
        /// The name `active` gives.
        name: String,
    },
    /// A theme variant does not define the same variables as the
    /// others. The path is the variant; `variable` is a name that the
    /// first variant defines and this one lacks (`missing`), or that
    /// this one defines and the first lacks.
    VariantsDiffer {
        /// The variable.
        variable: String,
        /// `true` when this variant lacks it, `false` when this variant
        /// has one the first lacks.
        missing: bool,
    },
    /// A `palette(...)` color inside `palettes`: a palette cannot refer
    /// to a palette.
    PaletteInPalette,
    /// A top-level node has a `connector`: a connector is the line to
    /// the node's parent, and a top-level node has none.
    ConnectorOnRoot,
    /// A key that the element it is on cannot use. `reason` says why.
    KeyNotAllowed {
        /// The key.
        key: &'static str,
        /// Why this element cannot have it.
        reason: &'static str,
    },
    /// The loader built something the document refuses: a row that
    /// breaks the shape rules, a tree that is not a document, an insert
    /// the tree rejects. The loader checks everything the document
    /// checks, in order to name the path, so this reports a defect in
    /// the loader rather than in the file.
    Internal {
        /// What the document reported.
        message: String,
    },
}

impl fmt::Display for LoadErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LoadErrorKind::TooLarge { bytes, max } => write!(
                f,
                "map is {bytes} bytes, over the {max} byte limit: refusing to load it. \
                 A map this size is either damaged or hostile."
            ),
            LoadErrorKind::Syntax { message } | LoadErrorKind::Invalid { message } => f.write_str(message),
            LoadErrorKind::UnknownKey { key } => write!(f, "unknown key `{key}`"),
            LoadErrorKind::MissingKey { key } => write!(f, "missing key `{key}`"),
            LoadErrorKind::DuplicateKey { key } => write!(f, "the key `{key}` appears twice"),
            LoadErrorKind::UnsupportedFormat { found } => {
                write!(f, "format {found} is not supported: this build reads format 1")
            }
            LoadErrorKind::Bound { error } => write!(f, "{error}"),
            LoadErrorKind::DuplicateId { id } => write!(f, "another node already has the id {id}"),
            LoadErrorKind::DuplicateDefinition { id } => {
                write!(f, "another entry of this list already has the id `{id}`")
            }
            LoadErrorKind::DanglingLink { to } => write!(f, "no node has the id {to}"),
            LoadErrorKind::SelfLink => f.write_str("a link cannot lead to the node that holds it"),
            LoadErrorKind::UnknownVerb { id } => write!(f, "the map defines no verb `{id}`"),
            LoadErrorKind::UnknownPalette { name } => write!(f, "the map defines no palette `{name}`"),
            LoadErrorKind::UnknownVariable { name } => write!(f, "the theme defines no variable `{name}`"),
            LoadErrorKind::UnknownFont { family } => {
                write!(f, "no font named `{family}` is compiled into this build")
            }
            LoadErrorKind::ActiveNotAVariant { name } => {
                write!(f, "`{name}` is not one of the theme's variants")
            }
            LoadErrorKind::VariantsDiffer { variable, missing } => {
                if *missing {
                    write!(
                        f,
                        "this variant lacks the variable `{variable}`, which the others define"
                    )
                } else {
                    write!(
                        f,
                        "this variant defines the variable `{variable}`, which the others lack"
                    )
                }
            }
            LoadErrorKind::PaletteInPalette => {
                f.write_str("a palette's colors are `#rrggbb` or `var(name)`, never `palette(...)`")
            }
            LoadErrorKind::ConnectorOnRoot => {
                f.write_str("a top-level node has no parent, so it has no connector")
            }
            LoadErrorKind::KeyNotAllowed { key, reason } => {
                write!(f, "`{key}` is not allowed here: {reason}")
            }
            LoadErrorKind::Internal { message } => {
                write!(
                    f,
                    "the loader built a map the document refuses ({message}): this is a bug"
                )
            }
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.path.is_empty() {
            write!(f, "{}", self.kind)
        } else {
            write!(f, "{}: {}", self.path, self.kind)
        }
    }
}

impl std::error::Error for LoadError {}

/// One step of a JSON path while the loader walks the file.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Seg {
    /// A key the schema defines.
    Key(&'static str),
    /// A key the author chose: a theme variant or variable, a palette.
    Name(String),
    /// A position in an array.
    Index(usize),
}

/// `segs` as a JSON path: keys after a dot (none before the first
/// segment), positions in brackets. O(length of the path).
pub(crate) fn render_path(segs: &[Seg]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for seg in segs {
        match seg {
            Seg::Key(key) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(key);
            }
            Seg::Name(name) => {
                if !out.is_empty() {
                    out.push('.');
                }
                out.push_str(name);
            }
            Seg::Index(index) => {
                // Writing to a `String` cannot fail.
                let _ = write!(out, "[{index}]");
            }
        }
    }
    out
}
