// SPDX-License-Identifier: MPL-2.0

//! [`LoadError`] and [`SaveError`]: why a `.mandala.json` did not load
//! or could not be saved, and where in the file.
//!
//! **Every error names its JSON path** (design §16): the location of
//! the offending value, written the way a reader would navigate to it,
//! `nodes[3].frame.rings[0].spans[2].color`. Paths address nodes by
//! their position in the flat `nodes` list, never by id, so a path is
//! always one `jq` can follow. The errors without one are about the
//! whole text: [`TooLarge`](LoadErrorKind::TooLarge),
//! [`ByteOrderMark`](LoadErrorKind::ByteOrderMark), text that is not
//! UTF-8, and a [`Syntax`](LoadErrorKind::Syntax) error the reader
//! raised where no value had begun.
//!
//! **A path alone does not help a person at 100,000 nodes**, so an
//! error inside `nodes[i]` also carries the node's context
//! ([`NodeContext`]: its `id` and an excerpt of its text), and every
//! error that has a place in the text carries a [`TextPosition`]. The
//! rendered error reads
//! `nodes[48213].sections[0].spans[2].color: the theme defines no variable `acent` (node 70211 "In the beginning…", line 48217)`.
//!
//! **There is one error.** A load is refused at the first thing wrong
//! with it and returns nothing else (§7.2): no partial document, no
//! list of problems, no recovery. Which error is "first" is fixed by
//! the order the loader reads in: the whole text (its size, its
//! byte-order mark, its encoding), then JSON syntax anywhere in it, then
//! `format`, then the other top-level keys in schema order, then the
//! nodes in list order — within a node its structure (`id`, `parent`,
//! `channel`) before its content — and the links last, because a link
//! may name a node that appears after it.
//!
//! **No raw control character reaches a message.** Text from the file
//! — a name, a key, an excerpt — is shown with its control, line
//! separator and bidi characters escaped
//! ([`context::shown`](crate::document::format::context::shown)), so a
//! hostile map cannot rewrite the terminal, dialog or IPC line an error
//! is printed on.

use crate::document::bounds::BoundError;
use crate::document::format::context::shown;
use std::fmt;

/// How many ids a [`ParentNotOnChain`](LoadErrorKind::ParentNotOnChain)
/// error lists as the ones a node there could name. The chain can be as
/// long as the tree is deep, and an error is not the place to print all
/// of it.
pub const CHAIN_SHOWN: usize = 8;

/// The node an error is inside: present on every [`LoadError`] and
/// [`SaveError`] whose path starts with `nodes[i]`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NodeContext {
    /// `i` in `nodes[i]`: the node's position in the list.
    pub index: usize,
    /// The node's `id`. `None` only when the id itself is what is wrong
    /// (missing, not a number, out of range): a node whose other keys
    /// fail to read still has its id read on its own.
    pub id: Option<u32>,
    /// The start of the node's text — the first span of its
    /// lowest-channel section, else of its first ring — cut to 32
    /// grapheme clusters (`…` when cut) and escaped
    /// ([`context::excerpt`](crate::document::format::context::excerpt)).
    /// `None` when the node has no text.
    pub excerpt: Option<String>,
}

/// A place in the text, both parts 1-based. The column counts bytes
/// from the start of the line, as the JSON reader counts them.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct TextPosition {
    /// The line.
    pub line: u32,
    /// The column, in bytes.
    pub column: u32,
}

/// Why a load failed, and where.
#[derive(Clone, Debug, PartialEq)]
pub struct LoadError {
    /// The JSON path of the offending value: `nodes[3].frame.rings[0]`.
    /// A key is written after a `.`, an array position in brackets, and
    /// the first segment has no leading dot. A key that is not a plain
    /// identifier (a name an author chose, holding a dot, or empty) is
    /// quoted in brackets instead, `palettes["a.b"][0].fill`. Empty when
    /// the error is about the whole text. For a key that is missing, the
    /// path is the object that lacks it and the key is in the
    /// [kind](LoadErrorKind::MissingKey); for a key that is not allowed,
    /// the path ends in the key itself.
    pub path: String,
    /// The node the path is inside, when it starts with `nodes[i]`.
    pub node: Option<NodeContext>,
    /// Where in the text: exact for an error the JSON reader raised (a
    /// syntax error, a wrong type, an unknown or missing key); the start
    /// of the node, or of the top-level value, for one the loader found
    /// after reading it (a bound, a reference, the chain); `None` for an
    /// error about the whole text.
    pub position: Option<TextPosition>,
    /// What is wrong there.
    pub kind: LoadErrorKind,
}

impl LoadError {
    /// An error of `kind` at `path`, with no node context or position
    /// yet: the phase that knows them adds them.
    pub(crate) fn new(path: String, kind: LoadErrorKind) -> Self {
        LoadError {
            path,
            node: None,
            position: None,
            kind,
        }
    }
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
    /// The map holds more elements than
    /// [`MAX_ELEMENTS`](crate::document::format::MAX_ELEMENTS) allows on
    /// this target: the other half of the memory budget
    /// [`TooLarge`](Self::TooLarge) bounds by bytes. Counted as the
    /// elements are built, and raised at the one past the bound, before
    /// it is appended.
    TooManyElements {
        /// The most elements this target builds.
        max: u64,
    },
    /// The text starts with a UTF-8 byte-order mark. A file is UTF-8
    /// without one (RFC 8259 §8.1), and accepting one would be a second
    /// spelling of the start of the file.
    ByteOrderMark,
    /// The text is not UTF-8, is not JSON, or nests deeper than the JSON
    /// reader follows inside one value (128 levels of array and object;
    /// a node's own content nests at most 9, so only a hostile text
    /// reaches it). `message` says what the reader reported.
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
    /// naming what was found and what was expected.
    Invalid {
        /// What the reader reported.
        message: String,
    },
    /// `format` is a positive integer this build does not read: a file
    /// written by a newer version. Reported before anything else in the
    /// file, whose other keys may be ones this build does not know.
    UnsupportedFormat {
        /// The number the file gives.
        found: u64,
        /// The newest format this build reads.
        newest: u64,
    },
    /// A number or a count is outside the bound
    /// [`document::bounds`](crate::document::bounds) puts on it.
    Bound {
        /// Which bound, and the value that broke it.
        error: BoundError,
    },
    /// Two nodes carry the same `id`. The path is the second one's `id`.
    /// A git merge of two branches that each added a node is the usual
    /// cause.
    DuplicateId {
        /// The id.
        id: u32,
        /// Where the first node with it is: `i` in `nodes[i]`.
        first_index: usize,
    },
    /// A node's `parent` is its own `id`.
    ParentIsSelf {
        /// The id.
        id: u32,
    },
    /// A node's `parent` names no node before it in the list: one that
    /// never appears, or one that appears later. A parent comes before
    /// its children.
    ParentNotEarlier {
        /// The id `parent` gives.
        parent: u32,
    },
    /// A node's `parent` comes earlier in the list, but its subtree has
    /// already ended: the list is in depth-first pre-order, so a node's
    /// parent is the node before it or one of that node's ancestors.
    ParentNotOnChain {
        /// The id `parent` gives.
        parent: u32,
        /// Where that node is: `i` in `nodes[i]`.
        parent_index: usize,
        /// The ids a node here could name: the node before it, then its
        /// ancestors, nearest first, at most [`CHAIN_SHOWN`].
        expected: Vec<u32>,
        /// Whether `expected` stops short of a root.
        truncated: bool,
    },
    /// A node, a section or a ring has a lower `channel` than the
    /// sibling listed before it. Siblings are listed in the order the
    /// document keeps them: ascending channel, equal channels in the
    /// order written. The path is the element's `channel`.
    SiblingsOutOfOrder {
        /// The channel of the sibling listed before it.
        previous_channel: u64,
        /// That sibling's `id`, when it is a node.
        previous_id: Option<u32>,
    },
    /// A name an author chose — a theme variant or variable, a palette,
    /// the id of an edit, presentation or verb — is empty.
    EmptyName,
    /// A name an author chose holds a control character (C0, DEL or C1)
    /// or a bidi embedding, override or isolate control, which can make
    /// it display as something else in a menu, a terminal or an error.
    NameNotPrintable {
        /// The offending character's code point.
        code_point: u32,
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
            LoadErrorKind::TooManyElements { max } => write!(
                f,
                "map holds more than {max} elements, the most this build can hold in memory: \
                 refusing to load it"
            ),
            LoadErrorKind::ByteOrderMark => f.write_str(
                "the text starts with a byte-order mark, which a map does not have: save it as UTF-8 without a BOM",
            ),
            LoadErrorKind::Syntax { message } | LoadErrorKind::Invalid { message } => {
                write!(f, "{}", shown(message))
            }
            LoadErrorKind::UnknownKey { key } => write!(f, "unknown key `{}`", shown(key)),
            LoadErrorKind::MissingKey { key } => write!(f, "missing key `{}`", shown(key)),
            LoadErrorKind::DuplicateKey { key } => write!(f, "the key `{}` appears twice", shown(key)),
            LoadErrorKind::UnsupportedFormat { found, newest } => write!(
                f,
                "this map is format {found}, written by a newer version of Mandala: this build reads format {newest}"
            ),
            LoadErrorKind::Bound { error } => write!(f, "{error}"),
            LoadErrorKind::DuplicateId { id, first_index } => {
                write!(f, "another node, nodes[{first_index}], already has the id {id}")
            }
            LoadErrorKind::ParentIsSelf { id } => write!(f, "node {id} names itself as its parent"),
            LoadErrorKind::ParentNotEarlier { parent } => write!(
                f,
                "no node before this one has the id {parent}; a parent comes before its children"
            ),
            LoadErrorKind::ParentNotOnChain {
                parent,
                parent_index,
                expected,
                truncated,
            } => {
                write!(
                    f,
                    "node {parent} (nodes[{parent_index}]) is earlier, but its subtree has ended; a node here can name "
                )?;
                for (n, id) in expected.iter().enumerate() {
                    if n > 0 {
                        f.write_str(", ")?;
                    }
                    write!(f, "{id}")?;
                }
                if *truncated {
                    f.write_str(", …")?;
                }
                if expected.is_empty() {
                    f.write_str("none, as a new root")
                } else {
                    f.write_str(", or none for a new root")
                }
            }
            LoadErrorKind::SiblingsOutOfOrder {
                previous_channel,
                previous_id,
            } => match previous_id {
                Some(id) => write!(
                    f,
                    "siblings are listed in ascending channel, and the sibling before this one (node {id}) has channel {previous_channel}"
                ),
                None => write!(
                    f,
                    "siblings are listed in ascending channel, and the one before this has channel {previous_channel}"
                ),
            },
            LoadErrorKind::EmptyName => f.write_str("a name cannot be empty"),
            LoadErrorKind::NameNotPrintable { code_point } => write!(
                f,
                "a name cannot hold U+{code_point:04X}, a control or bidi character that would change how it displays"
            ),
            LoadErrorKind::DuplicateDefinition { id } => {
                write!(f, "another entry of this list already has the id `{}`", shown(id))
            }
            LoadErrorKind::DanglingLink { to } => write!(f, "no node has the id {to}"),
            LoadErrorKind::SelfLink => f.write_str("a link cannot lead to the node that holds it"),
            LoadErrorKind::UnknownVerb { id } => write!(f, "the map defines no verb `{}`", shown(id)),
            LoadErrorKind::UnknownPalette { name } => {
                write!(f, "the map defines no palette `{}`", shown(name))
            }
            LoadErrorKind::UnknownVariable { name } => {
                write!(f, "the theme defines no variable `{}`", shown(name))
            }
            LoadErrorKind::UnknownFont { family } => {
                write!(f, "no font named `{}` is compiled into this build", shown(family))
            }
            LoadErrorKind::ActiveNotAVariant { name } => {
                write!(f, "`{}` is not one of the theme's variants", shown(name))
            }
            LoadErrorKind::VariantsDiffer { variable, missing } => {
                if *missing {
                    write!(
                        f,
                        "this variant lacks the variable `{}`, which the others define",
                        shown(variable)
                    )
                } else {
                    write!(
                        f,
                        "this variant defines the variable `{}`, which the others lack",
                        shown(variable)
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
                    "the loader built a map the document refuses ({}): this is a bug",
                    shown(message)
                )
            }
        }
    }
}

/// Write ` (node 70211 "In the beginning…", line 48217)`, leaving out
/// each part that is absent, and nothing at all when none is known.
fn write_context(
    f: &mut fmt::Formatter<'_>,
    node: Option<&NodeContext>,
    position: Option<TextPosition>,
) -> fmt::Result {
    let id = node.and_then(|node| node.id);
    let excerpt = node.and_then(|node| node.excerpt.as_deref());
    if id.is_none() && excerpt.is_none() && position.is_none() {
        return Ok(());
    }
    f.write_str(" (")?;
    let mut written = false;
    if id.is_some() || excerpt.is_some() {
        f.write_str("node")?;
        if let Some(id) = id {
            write!(f, " {id}")?;
        }
        if let Some(excerpt) = excerpt {
            // Already escaped: the excerpt holds no raw control or quote.
            write!(f, " \"{excerpt}\"")?;
        }
        written = true;
    }
    if let Some(position) = position {
        if written {
            f.write_str(", ")?;
        }
        write!(f, "line {}", position.line)?;
    }
    f.write_str(")")
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.path.is_empty() {
            write!(f, "{}: ", self.path)?;
        }
        write!(f, "{}", self.kind)?;
        write_context(f, self.node.as_ref(), self.position)
    }
}

impl std::error::Error for LoadError {}

/// Why a document cannot be saved, and where in the file the value
/// would have been.
///
/// **A save never writes text that [`load`](crate::document::format::load)
/// refuses or reads differently**, and it never drops a value either:
/// what the file has no spelling for is refused here, with the path the
/// value would have had, in the notation of [`LoadError::path`]. The
/// mutations (V1-B3) refuse to make any of these states, so one reaching
/// the saver is a defect in a mutation, not a thing a user did.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveError {
    /// Where in the file the offending value would be.
    pub path: String,
    /// The node the path is inside, when it starts with `nodes[i]`: its
    /// position in the list the saver writes, its id and its excerpt.
    pub node: Option<NodeContext>,
    /// What is wrong with it.
    pub kind: SaveErrorKind,
}

impl SaveError {
    /// An error of `kind` at `path`, with no node context yet.
    pub(crate) fn new(path: String, kind: SaveErrorKind) -> Self {
        SaveError {
            path,
            node: None,
            kind,
        }
    }
}

/// What is wrong at a [`SaveError`]'s path.
#[derive(Clone, Debug, PartialEq)]
pub enum SaveErrorKind {
    /// A size in the other unit from the one this position takes:
    /// points on a ring or a frame, a fraction anywhere else. The file
    /// writes a number, and the unit is whatever the position says, so
    /// writing it would load as another size.
    WrongUnit {
        /// `true` when the size is in points where a fraction belongs.
        found_points: bool,
    },
    /// A span inside a ring has a `size`. A ring sizes its own text.
    SizeInRing,
    /// A font the file cannot name: the fallback face, or a face this
    /// build has no family name for.
    NoFamilyName,
    /// A color names a theme variable the theme does not define.
    UnknownVariable {
        /// The index it holds.
        index: u16,
    },
    /// A node names a palette the map does not define.
    UnknownPalette {
        /// The index it holds.
        index: u16,
    },
    /// A trigger or a span link names a verb the map does not define.
    UnknownVerb {
        /// The index it holds.
        index: u16,
    },
    /// Two theme variants, two theme variables or two palettes have one
    /// name. The file keys them by name.
    DuplicateName {
        /// The name.
        name: String,
    },
    /// Two edits, two presentations or two verbs have one id.
    DuplicateDefinition {
        /// The id.
        id: String,
    },
    /// The theme has variables or an active variant and no variants. The
    /// empty theme, which has none of these, is written as no `theme`.
    ThemeWithoutVariants {
        /// How many variables it has.
        variables: usize,
        /// The active index it holds.
        active: u16,
    },
    /// The active variant is not one of the theme's variants.
    ActiveNotAVariant {
        /// The index it holds.
        active: u16,
        /// How many variants there are.
        variants: usize,
    },
    /// A variant holds a different number of values than the theme has
    /// variables.
    VariantLength {
        /// How many values the variant holds.
        values: usize,
        /// How many variables the theme has.
        variables: usize,
    },
    /// A link leads to something that is not a node.
    LinkToNothing,
    /// A link leads to the node that holds it.
    SelfLink,
    /// A portal link without exactly two ends.
    PortalEnds {
        /// How many ends it has.
        count: usize,
    },
    /// The tree holds another element than the one this place requires:
    /// the document is not shaped as the model says.
    WrongElement,
    /// The document wrote a file the loader refuses: a number outside
    /// its bound, or another rule the saver does not check on its own.
    /// `error` is what the loader said, at the path the file would give.
    NotLoadable {
        /// The loader's refusal.
        error: Box<LoadError>,
    },
    /// The text could not be written: the writer [`save_to`](crate::document::format::save_to)
    /// was given failed. The path is the node being written, or empty
    /// for the header.
    Io {
        /// What the writer reported.
        message: String,
    },
}

impl fmt::Display for SaveErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SaveErrorKind::WrongUnit { found_points: true } => f.write_str(
                "this size is in points, but here a size is a fraction of the node's shorter side",
            ),
            SaveErrorKind::WrongUnit { found_points: false } => {
                f.write_str("this size is a fraction, but here a size is in points")
            }
            SaveErrorKind::SizeInRing => {
                f.write_str("a span inside a ring has no size of its own: the ring sizes its text")
            }
            SaveErrorKind::NoFamilyName => {
                f.write_str("this font has no family name a file can write (the fallback face, or one this build cannot name)")
            }
            SaveErrorKind::UnknownVariable { index } => {
                write!(f, "the theme defines no variable {index}")
            }
            SaveErrorKind::UnknownPalette { index } => write!(f, "the map defines no palette {index}"),
            SaveErrorKind::UnknownVerb { index } => write!(f, "the map defines no verb {index}"),
            SaveErrorKind::DuplicateName { name } => {
                write!(f, "two entries are named `{}`, and the file keys them by name", shown(name))
            }
            SaveErrorKind::DuplicateDefinition { id } => {
                write!(f, "two entries of this list have the id `{}`", shown(id))
            }
            SaveErrorKind::ThemeWithoutVariants { variables, active } => write!(
                f,
                "the theme has {variables} variables and the active index {active} but no variants; \
                 only the empty theme can be written (as no `theme`)"
            ),
            SaveErrorKind::ActiveNotAVariant { active, variants } => {
                write!(f, "the active variant {active} is not one of the {variants} variants")
            }
            SaveErrorKind::VariantLength { values, variables } => {
                write!(f, "this variant holds {values} values for {variables} variables")
            }
            SaveErrorKind::LinkToNothing => f.write_str("this link leads to something that is not a node"),
            SaveErrorKind::SelfLink => f.write_str("a link cannot lead to the node that holds it"),
            SaveErrorKind::PortalEnds { count } => {
                write!(f, "a portal link has two ends, and this one has {count}")
            }
            SaveErrorKind::WrongElement => f.write_str(
                "the tree holds another kind of element here than the document model allows: this is a bug",
            ),
            SaveErrorKind::NotLoadable { error } => {
                write!(f, "the file this would write does not load ({})", error.kind)
            }
            SaveErrorKind::Io { message } => write!(f, "the text could not be written: {}", shown(message)),
        }
    }
}

impl fmt::Display for SaveError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if !self.path.is_empty() {
            write!(f, "{}: ", self.path)?;
        }
        write!(f, "{}", self.kind)?;
        write_context(f, self.node.as_ref(), None)
    }
}

impl std::error::Error for SaveError {}

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

/// Whether `name` can follow a dot in a path without being quoted: a
/// letter or `_`, then letters, digits and `_`.
fn is_plain(name: &str) -> bool {
    let mut chars = name.chars();
    matches!(chars.next(), Some(first) if first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// Write the key `name` after what `out` holds: `.name` when it is a
/// plain identifier (and just `name` first), otherwise `["name"]`, quoted
/// the way JSON quotes a string and with every control, line separator
/// and bidi control escaped as well, as all file text in a message is
/// ([`quoted`](crate::document::format::context::quoted)). A name an
/// author chose can hold a dot, a bracket or nothing at all, and
/// unquoted it would read as more path (`palettes.a.b[0].fill`) or none.
fn push_key(out: &mut String, name: &str) {
    if is_plain(name) {
        if !out.is_empty() {
            out.push('.');
        }
        out.push_str(name);
    } else {
        out.push('[');
        out.push_str(&crate::document::format::context::quoted(name));
        out.push(']');
    }
}

/// `segs` as a JSON path: keys after a dot (none before the first
/// segment) and quoted in brackets when they are not plain identifiers
/// (`palettes["a.b"][0].fill`), positions in brackets. O(length of the
/// path).
pub(crate) fn render_path(segs: &[Seg]) -> String {
    use std::fmt::Write;
    let mut out = String::new();
    for seg in segs {
        match seg {
            Seg::Key(key) => push_key(&mut out, key),
            Seg::Name(name) => push_key(&mut out, name),
            Seg::Index(index) => {
                // Writing to a `String` cannot fail.
                let _ = write!(out, "[{index}]");
            }
        }
    }
    out
}

/// `prefix`, then the path the JSON reader was at inside the value
/// `prefix` names, in the notation of [`render_path`]; and whether the
/// reader had a step it could not name (a key that was not a string).
/// Rendered from the path's segments rather than its `Display`, which
/// writes every key bare.
pub(crate) fn render_parse_path(prefix: &[Seg], path: &serde_path_to_error::Path) -> (String, bool) {
    use serde_path_to_error::Segment;
    use std::fmt::Write;
    let mut out = render_path(prefix);
    let mut unknown = false;
    for segment in path.iter() {
        match segment {
            Segment::Map { key } => push_key(&mut out, key),
            Segment::Enum { variant } => push_key(&mut out, variant),
            Segment::Seq { index } => {
                // Writing to a `String` cannot fail.
                let _ = write!(out, "[{index}]");
            }
            Segment::Unknown => unknown = true,
        }
    }
    (out, unknown)
}
