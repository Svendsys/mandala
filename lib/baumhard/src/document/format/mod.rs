// SPDX-License-Identifier: MPL-2.0

//! The file format: a [`Document`] as strict, typed JSON with a flat
//! node list — `.mandala.json` (design §7, D30).
//!
//! [`load`] reads a file into a document and [`save`] (or [`save_to`])
//! writes one. The shape, key by key, is `format/schema.md`; this module
//! is its reference implementation, and the two are held to each other
//! by the fixture corpus.
//!
//! **Flat.** `nodes` is one array holding every node of the map in
//! depth-first pre-order, and each node names its parent by `id`. No
//! node has a `children` key, so the JSON nesting of a file does not
//! grow with the depth of its tree: a node's own content nests at most
//! 9 levels, and the tree may be as deep as memory allows (D30, R13).
//! Every walk here — the load, the save, [`check_savable`] — is
//! iterative and linear in the size of the map, whatever its depth.
//!
//! **Strict.** Every object denies unknown keys, a wrong type is an
//! error, and a reference — a `parent`, a link's `to`, a verb, a
//! palette, a theme variable, a font — that names nothing is an error.
//! A row listed out of order (nodes, sections, rings) is refused, never
//! sorted. There is no preserved-unknown-keys mechanism and no
//! skipped-construct report: a file is a document or it does not load,
//! and the error is the first thing wrong, with the JSON path of the
//! value, the node it is in and its line ([`LoadError`]).
//!
//! **Typed on read.** Names are resolved to their typed forms while the
//! file is read (`"var(accent)"` to a
//! [`VarId`](crate::document::values::VarId), a palette name to a
//! [`PaletteId`](crate::document::values::PaletteId), a family name to
//! an [`AppFont`](crate::font::fonts::AppFont)); the document never
//! holds a string form. Every number goes through
//! [`bounds`](crate::document::bounds).
//!
//! **Deterministic on write.** An unchanged document saves
//! byte-identically: keys in schema order, name-keyed objects in
//! ascending byte order, nodes in the tree's pre-order, numbers in
//! serde_json's shortest form of the stored `f32`. The layout — the
//! header pretty-printed, then one node per line — is decided in one
//! place, `writer.rs`.
//!
//! **One spelling per value, in the whole file.** The value spellings
//! (a color, a weight, a fit mode, a start point) are the types in
//! `document::format::spelling`, names throughout,
//! never an index: indices are assigned by sorted name, so a saved one
//! would name another entry the moment a name was added. A predicate
//! saved in an edit (V1-D1) is spelled with them too, and the strictness
//! gate keeps every index type out of the types this module reads.
//!
//! The old `.mindmap.json` format is not read here and has no knowledge
//! in this module.

#[macro_use]
mod macros;
mod chain;
pub mod context;
pub(crate) mod dto;
pub mod error;
mod from_dto;
mod index;
mod load_map;
mod load_node;
pub(crate) mod load_style;
pub(crate) mod spelling;
mod strict;
pub(crate) mod to_dto;
mod writer;

pub use error::{LoadError, LoadErrorKind, NodeContext, SaveError, SaveErrorKind, TextPosition};

use crate::document::format::context::{dto_excerpt, offset_in, peek_excerpt, peek_id, position_at};
use crate::document::format::dto::{MapHeaderDto, NodeDto};
use crate::document::format::error::Seg;
use crate::document::format::from_dto::{Builder, Keep};
use crate::document::format::index::MapIndex;
use crate::document::ids::Document;

/// Ceiling on the length of a map's text, in bytes, or `None` where the
/// platform imposes none.
///
/// **`None` on native, and that is the design rather than an
/// omission.** Mandala is meant to open maps as large as the machine
/// can hold. The user picked the file; a 10 GB map costing 10 GB is the
/// feature, and a failed allocation is an honest outcome of a choice
/// they made. A limit defended by the size of today's fixtures is a
/// limit that expires.
///
/// **`Some` on wasm32, because there the limit is physics.** `usize` is
/// 32 bits and the address space is 4 GiB, shared with the runtime, the
/// GPU buffers and the browser itself, and a failed allocation traps and
/// kills the page. Memory is the only limit, and it has two axes: what a
/// map costs grows with its text and with the number of elements it
/// becomes, and a short text can become many elements. So wasm32 bounds
/// both — this, and [`MAX_ELEMENTS`] — each to half of
/// [`WASM32_MEMORY_BUDGET`], from costs measured per byte and per
/// element (see [`WASM32_MAX_MAP_BYTES`]). This is a sanctioned target
/// divergence, registered in `CLAUDE.md`'s dual-target section.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_MAP_BYTES: Option<u64> = None;

/// See the native definition above for the rationale, and
/// [`WASM32_MAX_MAP_BYTES`] for the value.
#[cfg(target_arch = "wasm32")]
pub const MAX_MAP_BYTES: Option<u64> = Some(WASM32_MAX_MAP_BYTES);

/// Ceiling on the number of elements a map becomes, or `None` where the
/// platform imposes none: [`MAX_MAP_BYTES`]'s other axis. `None` on
/// native for the same reason; on wasm32, [`WASM32_MAX_ELEMENTS`]. The
/// builder counts each element as it appends it and refuses the one past
/// the bound with
/// [`TooManyElements`](LoadErrorKind::TooManyElements), before it is
/// built.
#[cfg(not(target_arch = "wasm32"))]
pub const MAX_ELEMENTS: Option<u64> = None;

/// See the native definition above.
#[cfg(target_arch = "wasm32")]
pub const MAX_ELEMENTS: Option<u64> = Some(WASM32_MAX_ELEMENTS);

/// The part of wasm32's 4 GiB address space a load or a save may use:
/// 3 GiB. The remaining 1 GiB is reserved for the runtime (the module's
/// code, data and stack) and for the first projection the app builds
/// from the document — what the scene builders make of it, which this
/// module does not measure.
pub const WASM32_MEMORY_BUDGET: u64 = 3 * 1024 * 1024 * 1024;

/// The most heap a load or a save of a map needs **per byte of its
/// text**, beside what its elements cost: the text itself, the strings
/// the document holds from it, the extents the index keeps of it, and
/// on a save the text being written. Measured
/// (`tests/format_memory.rs`) on a map that is one long text, where the
/// elements cost nothing next to it, and rounded up.
pub const WASM32_BYTES_PER_TEXT_BYTE: u64 = 7;

/// The most heap a load or a save of a map needs **per element**,
/// beside what its text costs ([`WASM32_BYTES_PER_TEXT_BYTE`] per
/// byte): the arena slot, what the element holds on the heap, and a
/// load's or a check's bookkeeping per node and per link. Measured
/// (`tests/format_memory.rs`) on the densest maps the format allows —
/// every kind of row a node can hold in bulk, rows of empty sections,
/// empty rings, links and empty spans among them — at the load's peak
/// and at the save's, and rounded up.
pub const WASM32_BYTES_PER_ELEMENT: u64 = 560;

/// wasm32's ceiling on a map's text: half of [`WASM32_MEMORY_BUDGET`]
/// over [`WASM32_BYTES_PER_TEXT_BYTE`].
///
/// **Why two halves.** The measured costs say a map's peak is at most
/// `text × WASM32_BYTES_PER_TEXT_BYTE + elements × WASM32_BYTES_PER_ELEMENT`
/// (`tests/format_memory.rs` checks every measured map against exactly
/// that). A map under both ceilings therefore peaks under half the
/// budget plus half the budget — whatever its shape, a long text with
/// few elements, many elements from a short text, or anything between.
pub const WASM32_MAX_MAP_BYTES: u64 = WASM32_MEMORY_BUDGET / 2 / WASM32_BYTES_PER_TEXT_BYTE;

/// wasm32's ceiling on a map's elements: half of
/// [`WASM32_MEMORY_BUDGET`] over [`WASM32_BYTES_PER_ELEMENT`]. See
/// [`WASM32_MAX_MAP_BYTES`] for the model.
pub const WASM32_MAX_ELEMENTS: u64 = WASM32_MEMORY_BUDGET / 2 / WASM32_BYTES_PER_ELEMENT;

/// The two ceilings a load and a save are held to on this target.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) struct Limits {
    /// The longest text: [`MAX_MAP_BYTES`].
    pub bytes: Option<u64>,
    /// The most elements: [`MAX_ELEMENTS`].
    pub elements: Option<u64>,
}

/// This target's [`Limits`].
pub(crate) const LIMITS: Limits = Limits {
    bytes: MAX_MAP_BYTES,
    elements: MAX_ELEMENTS,
};

/// [`load`] for bytes: the entry point for input that is not known to
/// be text (a file read, a socket, a fuzzer's corpus). The text is
/// checked against [`MAX_MAP_BYTES`] before it is looked at, and then
/// must be UTF-8; bytes that are not are a
/// [`Syntax`](LoadErrorKind::Syntax) error naming the offset.
///
/// Pure and in-memory: it reads no file, so a caller that reads one
/// wraps this. It never panics, on any input: every refusal is a
/// [`LoadError`], and nothing is allocated in proportion to the input
/// before the size ceiling and the reader's depth limit have been
/// applied to it.
pub fn load_from_bytes(bytes: &[u8]) -> Result<Document, LoadError> {
    load_bytes_under(bytes, LIMITS)
}

/// [`load_from_bytes`] against caller-supplied limits.
pub(crate) fn load_bytes_under(bytes: &[u8], limits: Limits) -> Result<Document, LoadError> {
    let length = bytes.len() as u64;
    if let Some(max) = limits.bytes.filter(|max| length > *max) {
        return Err(LoadError::new(
            String::new(),
            LoadErrorKind::TooLarge { bytes: length, max },
        ));
    }
    if bytes.starts_with(BOM.as_bytes()) {
        return Err(LoadError::new(String::new(), LoadErrorKind::ByteOrderMark));
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => load_under(text, limits),
        Err(error) => Err(LoadError::new(
            String::new(),
            LoadErrorKind::Syntax {
                message: format!("the text is not UTF-8: {error}"),
            },
        )),
    }
}

/// Read a `.mandala.json` into a document.
///
/// Three linear passes over the text, none recursing on the depth of
/// the tree (design B.14):
///
/// 1. **Gates**: the text is refused outright if it is longer than
///    [`MAX_MAP_BYTES`] (checked before anything is parsed, on every
///    transport: the browser receives its map as a string and never
///    touches the filesystem path), or starts with a byte-order mark.
/// 2. **Index**: one parse finds the extent of each top-level value and
///    of each node, borrowed from the text, building nothing; a syntax
///    error anywhere is reported here, then `format` is checked.
/// 3. **Build**: the header is read, then each node is parsed from its
///    extent and appended to the tree — only one node's DTO is alive at
///    a time — and links are resolved last.
///
/// The first error is returned and nothing else: no partial document.
///
/// Cost: O(text) for the index and the parse, then O(elements) with one
/// hash insert or lookup per node, per `parent` and per link. Every
/// insert is an append. Memory beside the text and the tree: one fat
/// pointer per node, the ident index (made once, at its size), the
/// chain (as long as the tree is deep), a few words per pending link
/// and one node's DTO. Every element is counted against
/// [`MAX_ELEMENTS`] as it is appended.
pub fn load(json: &str) -> Result<Document, LoadError> {
    load_under(json, LIMITS)
}

/// [`load`] against caller-supplied limits.
///
/// Split out so the refusals stay testable. [`MAX_MAP_BYTES`] and
/// [`MAX_ELEMENTS`] are `None` on every target the test suite runs on,
/// so a test written against the constants would assert nothing and
/// pass — and the arm it stopped covering is the one the browser
/// depends on. This takes the limits as a parameter, so a native test
/// can hand it small ones and drive the same code the wasm32 build
/// reaches with real ones.
pub(crate) fn load_under(json: &str, limits: Limits) -> Result<Document, LoadError> {
    load_counted(json, limits).map(|(document, _)| document)
}

/// [`load`], and how many elements of the document had their row
/// checked against the shape rules: every element, which a test asserts.
#[cfg(test)]
pub(crate) fn load_counting_checks(json: &str) -> Result<(Document, usize), LoadError> {
    load_counted(json, LIMITS)
}

/// The three phases of [`load`], counting the rows checked.
fn load_counted(json: &str, limits: Limits) -> Result<(Document, usize), LoadError> {
    check_text_cap_against(json, limits.bytes)?;
    let index = index::index(json)?;
    let header = header(&index)?;
    let mut builder = Builder::begin(&header, Keep::Document, limits.elements)
        .map_err(|error| placed(error, &index, None))?;
    // One entry per node, made once: the map does not grow, and so never
    // holds its old table and its new one at the same time. No more than
    // the element bound, which a list of more nodes reaches first.
    let reserve = limits.elements.map_or(index.nodes.len(), |max| {
        index.nodes.len().min(usize::try_from(max).unwrap_or(usize::MAX))
    });
    builder.idents.reserve(reserve);
    for (i, extent) in index.nodes.iter().enumerate() {
        let dto: NodeDto = index::parse_extent(index.text, extent, &[Seg::Key("nodes"), Seg::Index(i)])
            .map_err(|error| LoadError {
                // The node did not read, so its id and text are read on
                // their own, leniently, for the error.
                node: Some(NodeContext {
                    index: i,
                    id: peek_id(extent),
                    excerpt: peek_excerpt(extent),
                }),
                ..error
            })?;
        builder
            .node(i, &dto)
            .map_err(|error| placed(error, &index, Some((i, &dto))))?;
    }
    builder.finish().map_err(|error| placed(error, &index, None))
}

/// The header's top-level values, each parsed from its extent in the
/// order the schema lists them (design B.3).
fn header<'t>(index: &MapIndex<'t>) -> Result<MapHeaderDto<'t>, LoadError> {
    Ok(MapHeaderDto {
        format: index::FORMAT,
        name: optional(index, "name")?.unwrap_or_default(),
        theme: optional(index, "theme")?,
        palettes: optional(index, "palettes")?.unwrap_or_default(),
        defaults: optional(index, "defaults")?,
        edits: optional(index, "edits")?.unwrap_or_default(),
        presentations: optional(index, "presentations")?.unwrap_or_default(),
        verbs: optional(index, "verbs")?.unwrap_or_default(),
    })
}

/// The top-level value at `key`, parsed from its extent, or `None` when
/// the text leaves the key out.
fn optional<'t, T: serde::Deserialize<'t>>(
    index: &MapIndex<'t>,
    key: &'static str,
) -> Result<Option<T>, LoadError> {
    index
        .value(key)
        .map(|extent| index::parse_extent(index.text, extent, &[Seg::Key(key)]))
        .transpose()
}

/// `error`, found after its value was read, completed with what only the
/// loader knows: the excerpt of the node being built (`current`, its
/// place in the list and its DTO), and a position — the start of the
/// node an error is in, or of the top-level value its path starts in.
/// An error the reader raised keeps its own, exact, position.
fn placed(mut error: LoadError, index: &MapIndex<'_>, current: Option<(usize, &NodeDto)>) -> LoadError {
    if let Some(context) = error.node.as_mut() {
        if let Some((_, dto)) = current.filter(|(i, _)| *i == context.index) {
            if context.excerpt.is_none() {
                context.excerpt = dto_excerpt(dto);
            }
            if context.id.is_none() {
                context.id = crate::document::bounds::check_node_ident(dto.id).ok();
            }
        }
    }
    if error.position.is_none() {
        let extent = match &error.node {
            Some(context) => index.nodes.get(context.index).copied(),
            None => {
                let key = error.path.split(['.', '[']).next().unwrap_or_default();
                index.value(key)
            }
        };
        error.position =
            extent.map(|extent| position_at(index.text, offset_in(index.text, extent.get()).unwrap_or(0)));
    }
    error
}

/// The UTF-8 byte-order mark, which a map does not start with.
const BOM: &str = "\u{FEFF}";

/// Phase 1 of a load, for a text: refuse `json` if it is longer than
/// `ceiling`, then if it starts with a byte-order mark (design B.2).
/// O(1).
pub(crate) fn check_text_cap_against(json: &str, ceiling: Option<u64>) -> Result<(), LoadError> {
    let bytes = json.len() as u64;
    if let Some(max) = ceiling.filter(|max| bytes > *max) {
        return Err(LoadError::new(
            String::new(),
            LoadErrorKind::TooLarge { bytes, max },
        ));
    }
    if json.starts_with(BOM) {
        return Err(LoadError::new(String::new(), LoadErrorKind::ByteOrderMark));
    }
    Ok(())
}

/// `error`, which the loader's builder raised against what the saver
/// would write, as the saver's refusal at the same path.
fn not_loadable(error: LoadError) -> SaveError {
    SaveError {
        path: error.path.clone(),
        node: error.node.clone(),
        kind: SaveErrorKind::NotLoadable {
            error: Box::new(error),
        },
    }
}

/// Whether `doc` can be saved: `Ok` exactly when [`save`] would give a
/// file that [`load`] reads back as the same document. Otherwise the
/// first thing the file cannot say, with the path it would have had and
/// the node it is in.
///
/// Two checks, in one walk. The saver's own walk refuses what the file
/// has no spelling for (a size in the wrong unit, a name two entries
/// share, a color naming a variable that is not there), at the path the
/// value would have had. Then what it would write is handed, node by
/// node, to the loader's own builder, which refuses a number outside its
/// bound, a channel past
/// [`MAX_CHANNEL`](crate::document::bounds::MAX_CHANNEL) and every other
/// rule the loader holds, at the same path. The saver therefore cannot
/// disagree with the loader about what a loadable file is: it asks it.
///
/// The one exception to "the same document" is deliberate and stated in
/// `format/schema.md`: a portal link keeps the path it had as a line link
/// within a session, so that it can switch kind without losing it, and the
/// file never saves it.
///
/// B3's mutations and B4's editor call this, or the parts of it they
/// need, when they apply an edit. [`save`] runs it itself.
///
/// **It holds one node, not a second document.** The builder runs in
/// `Keep::OneNode`: each node is built from what the saver would
/// write, its row checked, and its elements freed before the next. What
/// a load answers from the nodes it keeps is answered from `doc`
/// instead: each node's id must be the one `doc`'s ident index holds
/// for it — which makes the ids unique — a link's `to` must be an id
/// the index holds, other than its own node's, and at the end the index
/// must hold no id but the nodes'. A parent is looked for on the chain,
/// where a load finds it. So a check that passes is one the full build
/// passes. A refusal about a node's own content, a bound, the element
/// limit or anything the saver refuses is the one a load would give,
/// and is returned as it is. Only a refusal about ids, parents or links
/// — which the check answers from `doc`, not from the nodes a load keeps
/// — is named by checking again with the full build, which keeps what a
/// load keeps: that refusal, and only it, costs a second copy of the
/// document. A document grown past the element limit is therefore
/// refused without one.
///
/// Cost: one iterative walk of the tree, building one node's DTO and
/// elements at a time; O(elements) whatever the depth, with one hash
/// lookup per node and per link. Memory beside `doc`: one node's DTO
/// and elements, the header's elements, and the chain (a few words per
/// level of the deepest path). On a target with [`MAX_MAP_BYTES`], one
/// more walk counts the bytes the file would have, writing none of
/// them, since a file longer than that does not load there.
pub fn check_savable(doc: &Document) -> Result<(), SaveError> {
    check_savable_under(doc, LIMITS)
}

/// [`check_savable`] against caller-supplied limits, for the reason
/// [`load_under`] takes them.
pub(crate) fn check_savable_under(doc: &Document, limits: Limits) -> Result<(), SaveError> {
    match check_one_node(doc, limits.elements) {
        OneNode::Passed(_) => {}
        OneNode::Refused(refusal) => return Err(refusal),
        OneNode::Unsure => check_by_building(doc, limits.elements)?,
    }
    if let Some(max) = limits.bytes {
        let bytes = saved_length(doc)?;
        if bytes > max {
            return Err(not_loadable(LoadError::new(
                String::new(),
                LoadErrorKind::TooLarge { bytes, max },
            )));
        }
    }
    Ok(())
}

/// [`check_savable_under`] without the byte bound, and what its one-node
/// check answered: the most elements it held at once when it passed. A
/// test bounds the count by one node's, and asserts which refusals the
/// full build names.
#[cfg(test)]
pub(crate) fn check_savable_holding_under(
    doc: &Document,
    max_elements: Option<u64>,
) -> (Result<(), SaveError>, OneNode) {
    let answer = check_one_node(doc, max_elements);
    let result = match &answer {
        OneNode::Passed(_) => Ok(()),
        OneNode::Refused(refusal) => Err(refusal.clone()),
        OneNode::Unsure => check_by_building(doc, max_elements),
    };
    (result, answer)
}

/// [`check_savable_holding_under`] at this target's element bound, with
/// the answer as the most elements held, `None` when the one-node check
/// did not pass.
#[cfg(test)]
pub(crate) fn check_savable_holding(doc: &Document) -> (Result<(), SaveError>, Option<usize>) {
    let (result, answer) = check_savable_holding_under(doc, LIMITS.elements);
    let held = match answer {
        OneNode::Passed(held) => Some(held),
        _ => None,
    };
    (result, held)
}

/// What the one-node check of [`check_savable`] found.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum OneNode {
    /// The file would load; the most elements the check held at once.
    Passed(usize),
    /// The file would not load, for a reason the check names exactly as
    /// a load would: a bound, the element limit, a value out of place,
    /// anything the saver itself refuses. Each depends only on the node
    /// it is in and the header, which the check holds as a load does.
    Refused(SaveError),
    /// Something about ids, parents or links refused, or a defect: the
    /// check answers those from the document rather than from the nodes
    /// a load keeps, so the full build names it.
    Unsure,
}

/// Whether a refusal the one-node builder raised is about what that
/// builder does not keep — ids, parents, links — or a defect, and so
/// must be named by the full build.
fn needs_the_full_build(kind: &LoadErrorKind) -> bool {
    matches!(
        kind,
        LoadErrorKind::DuplicateId { .. }
            | LoadErrorKind::ParentIsSelf { .. }
            | LoadErrorKind::ParentNotEarlier { .. }
            | LoadErrorKind::ParentNotOnChain { .. }
            | LoadErrorKind::DanglingLink { .. }
            | LoadErrorKind::SelfLink
            | LoadErrorKind::Internal { .. }
    )
}

/// The one-node check of [`check_savable`]. See [`OneNode`].
fn check_one_node(doc: &Document, max_elements: Option<u64>) -> OneNode {
    let header = match to_dto::header_of(doc) {
        Ok(header) => header,
        Err(refusal) => return OneNode::Refused(refusal),
    };
    let mut builder = match Builder::begin(&header, Keep::OneNode, max_elements) {
        Ok(builder) => builder,
        Err(error) if needs_the_full_build(&error.kind) => return OneNode::Unsure,
        Err(error) => return OneNode::Refused(not_loadable(error)),
    };
    // Set when `each_node` is stopped for the full build to name why.
    let mut unsure = false;
    let mut nodes = 0usize;
    let walked = to_dto::each_node(doc, |i, id, dto| {
        let stop = |unsure: &mut bool| {
            *unsure = true;
            SaveError::new(
                String::new(),
                SaveErrorKind::Io {
                    message: String::new(),
                },
            )
        };
        // The ident index must hold this node under its id: no other
        // node of the document can then have the same id.
        let own = u32::try_from(dto.id).ok().map(crate::document::ids::NodeIdent);
        if own.and_then(|own| doc.node_by_ident(own)) != Some(id) {
            return Err(stop(&mut unsure));
        }
        // A link names a node the index holds, other than its own.
        for link in &dto.links {
            let to = u32::try_from(link.to).ok().map(crate::document::ids::NodeIdent);
            match to.and_then(|to| doc.node_by_ident(to)) {
                Some(target) if target != id => {}
                _ => return Err(stop(&mut unsure)),
            }
        }
        nodes += 1;
        builder.node(i, &dto).map_err(|error| {
            if needs_the_full_build(&error.kind) {
                return stop(&mut unsure);
            }
            node_refusal(doc, i, id, error)
        })
    });
    match walked {
        Err(_) if unsure => return OneNode::Unsure,
        Err(refusal) => return OneNode::Refused(refusal),
        Ok(()) => {}
    }
    // And the index holds nothing else, so every id a link named is a
    // node the file lists.
    if nodes != doc.ident_count() {
        return OneNode::Unsure;
    }
    match builder.finish_check() {
        Ok(held) => OneNode::Passed(held),
        Err(error) if needs_the_full_build(&error.kind) => OneNode::Unsure,
        Err(error) => OneNode::Refused(not_loadable(error)),
    }
}

/// The saver's refusal for `error`, which the builder raised building
/// node `i` (the document's `id`), named with the node's excerpt as the
/// document holds it.
fn node_refusal(doc: &Document, i: usize, id: indextree::NodeId, error: LoadError) -> SaveError {
    let mut refusal = not_loadable(error);
    if let Some(context) = refusal.node.as_mut().filter(|context| context.index == i) {
        context.excerpt = context::tree_excerpt(doc.tree(), id);
    }
    refusal
}

/// [`check_by_building`], for a test that holds the one-node check's
/// refusals to the full build's.
#[cfg(test)]
pub(crate) fn check_savable_by_building(doc: &Document, max_elements: Option<u64>) -> Result<(), SaveError> {
    check_by_building(doc, max_elements)
}

/// The full build of [`check_savable`]: the document a load of the file
/// would make, built and dropped, so that a refusal is exactly a load's.
fn check_by_building(doc: &Document, max_elements: Option<u64>) -> Result<(), SaveError> {
    let header = to_dto::header_of(doc)?;
    let mut builder = Builder::begin(&header, Keep::Document, max_elements).map_err(not_loadable)?;
    to_dto::each_node(doc, |i, id, dto| {
        builder
            .node(i, &dto)
            .map_err(|error| node_refusal(doc, i, id, error))
    })?;
    builder.finish().map(|_| ()).map_err(not_loadable)
}

/// How many bytes [`save`] would write for `doc`, counted without
/// keeping any of them.
fn saved_length(doc: &Document) -> Result<u64, SaveError> {
    let mut counter = Counter(0);
    write_unchecked(doc, &mut counter)?;
    Ok(counter.0)
}

/// A writer that only counts.
struct Counter(u64);

impl std::io::Write for Counter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0 += buf.len() as u64;
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// Write a document as a `.mandala.json`: deterministic, the header
/// two-space indented, then one node per line, ending in a newline.
///
/// Nothing derived is written, and nothing is dropped: what the file
/// cannot spell is a [`SaveError`] (see [`check_savable`]), never a
/// value left out with a warning. Cost: [`check_savable`]'s, then one
/// more iterative walk writing each node as it is reached; O(elements).
pub fn save(doc: &Document) -> Result<String, SaveError> {
    let mut text = Vec::new();
    save_to(doc, &mut text)?;
    String::from_utf8(text).map_err(|error| {
        SaveError::new(
            String::new(),
            SaveErrorKind::Io {
                message: error.to_string(),
            },
        )
    })
}

/// [`save`], streamed to `out`: the header, then each node's line as the
/// walk reaches it, so a large map is never held as one text. Nothing is
/// written unless [`check_savable`] passes first. A failure of `out` is
/// [`SaveErrorKind::Io`] at the path of the node being written (empty
/// for the header and the close).
pub fn save_to(doc: &Document, out: &mut impl std::io::Write) -> Result<(), SaveError> {
    check_savable(doc)?;
    write_unchecked(doc, out)
}

/// The writing half of [`save_to`], with no check.
fn write_unchecked(doc: &Document, out: &mut impl std::io::Write) -> Result<(), SaveError> {
    let io = |path: String| {
        move |error: std::io::Error| {
            SaveError::new(
                path,
                SaveErrorKind::Io {
                    message: error.to_string(),
                },
            )
        }
    };
    let header = to_dto::header_of(doc)?;
    let has_nodes = doc
        .children(doc.root())
        .any(|child| doc.get(child).is_some_and(|element| element.as_node().is_some()));
    writer::header(out, &header, has_nodes).map_err(io(String::new()))?;
    to_dto::each_node(doc, |i, _, dto| {
        writer::node(out, i == 0, &dto).map_err(io(error::render_path(&[Seg::Key("nodes"), Seg::Index(i)])))
    })?;
    writer::end(out, has_nodes).map_err(io(String::new()))
}
