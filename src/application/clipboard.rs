// SPDX-License-Identifier: MPL-2.0

//! The clipboard: the platform's (native uses `arboard`; WASM is a logged
//! stub pending an async-browser integration), the app's own, and what
//! Copy, Cut and Paste take from and give to the selection (V1-C4, #245).
//!
//! **Per selected element**, as on `main`:
//!
//! | Selected | Copy | Cut also | Paste |
//! |---|---|---|---|
//! | a section | its text, and its spans and style ([`SectionFragment`]) | empties its spans | the fragment's spans and style; other text as one unstyled span |
//! | part of a section ([`Selection::range`]) | that text | deletes it | replaces it with the text |
//! | a node | its sections' text, joined by `\n` | empties every section | the text as section 0's one span |
//! | a connector, link, label or portal end | its resolved color, as hex | clears its own color | a color in the file's spelling (`#rrggbb`, `var(name)`, `palette(text)`, …) as its own; empty text clears it |
//!
//! Several elements copy as their texts joined by
//! `MULTI_TARGET_SEPARATOR`; a paste onto as many elements as the text
//! splits into gives each its piece, and otherwise gives every one the
//! whole text.
//!
//! **The app's clipboard** ([`Clipboard`], held by the document it was
//! copied from) keeps the text the platform was given beside a copied
//! section's fragment. A paste uses the fragment only while the
//! platform's text is still that text — so text copied in another app in
//! between wins — and reads the app's text where the platform has none
//! (the browser). The fragment is the section in the document's own
//! types, but a theme variable and a verb are indices that shift when a
//! name is added before theirs, so the fragment keeps the names its
//! indices meant when it was copied, and a paste maps each one back by
//! name in the document as it then is (a name that is gone pastes as
//! inherited). The OS clipboard only ever gets plain text.
//!
//! Every change Cut and Paste make is one request: the selection's
//! elements are grouped by what they receive, and each group is one walk
//! anchored at all of its elements.

use baumhard::arena::NodeId;
use baumhard::document::element::{DocElement, DocKind, SectionData, SpanData, TextStyle};
use baumhard::document::format;
use baumhard::document::ids::Document;
use baumhard::document::mutation::compose::{delete_text, insert_text, replace_spans};
use baumhard::document::mutation::error::MutationError;
use baumhard::document::mutation::ops::{
    DocMutation, DocMutatorTree, LabelOp, PathOp, PortalEndOp, SectionOp, TextOp,
};
use baumhard::document::mutation::step::{DocStep, DocWalk};
use baumhard::document::resolve::resolve_text;
use baumhard::document::values::{Align, ColorRef, LinkTarget, VarId, VerbId};
use baumhard::engine::anchor::Anchors;
use baumhard::engine::key::BranchKey;
use baumhard::engine::mutator::Mutator;
use baumhard::util::grapheme_chad::find_byte_index_of_grapheme;

use crate::application::document::selection::kind_of;
use crate::application::document::Selection;
use crate::application::view::inspect::{children_of_kind, element_text};

/// Between the texts of several copied elements. Doubled, so a section
/// whose text holds a line break still splits back into one piece.
const MULTI_TARGET_SEPARATOR: &str = "\n\n";

/// Read text from the system clipboard, or `None` on failure.
#[cfg(not(any(target_arch = "wasm32", test)))]
pub fn read_clipboard() -> Option<String> {
    arboard::Clipboard::new()
        .ok()
        .and_then(|mut cb| cb.get_text().ok())
        .filter(|s| !s.is_empty())
}

/// Write text to the system clipboard; ignores failures (interactive
/// paths must not panic — §9).
#[cfg(not(any(target_arch = "wasm32", test)))]
pub fn write_clipboard(text: &str) {
    if let Ok(mut cb) = arboard::Clipboard::new() {
        let _ = cb.set_text(text);
    }
}

/// The browser has no clipboard yet: nothing is read, and a paste uses
/// the app's clipboard. A test build reads none either: the user's
/// clipboard is not a test's to read or write, and every test then runs
/// the browser's path.
#[cfg(any(target_arch = "wasm32", test))]
pub fn read_clipboard() -> Option<String> {
    log::debug!("clipboard read not supported here");
    None
}

/// The browser has no clipboard yet, and a test build touches none:
/// nothing is written, and the app's clipboard holds the copy.
#[cfg(any(target_arch = "wasm32", test))]
pub fn write_clipboard(_text: &str) {
    log::debug!("clipboard write not supported here");
}

/// What Copy and Cut left in the app ([module docs](self)).
#[derive(Clone, Debug, PartialEq)]
pub struct Clipboard {
    /// The text the platform's clipboard was given.
    pub text: String,
    /// The section copied, when exactly one whole section was.
    pub section: Option<SectionFragment>,
}

/// A copied section in the file's terms (`format/schema.md`
/// §"Sections"): its spans and the style fields of its own, without its
/// place in its node (channel, offset, minimum size). An internal
/// representation: the OS clipboard gets the text.
#[derive(Clone, Debug, PartialEq)]
pub struct SectionFragment {
    /// How its lines align.
    align: Option<Align>,
    /// The text defaults its spans inherit.
    style: TextStyle,
    /// Its spans, in order.
    spans: Vec<SpanData>,
    /// The copied document's theme variables, by name, in the order a
    /// `VarId` indexes them then.
    vars: Vec<String>,
    /// Its verbs' ids, in the order a `VerbId` indexes them then.
    verbs: Vec<String>,
}

impl SectionFragment {
    /// The fragment's style and spans with every theme variable and verb
    /// it names mapped, by name, to its index in `doc`: what pasting it
    /// into `doc` as it now is means. A name `doc` no longer has is left
    /// unset, so the span inherits. O(the spans, and a search per name).
    fn in_doc(&self, doc: &Document) -> (TextStyle, Vec<SpanData>) {
        let color = |color: Option<ColorRef>| match color {
            Some(ColorRef::Var(VarId(at))) => {
                let name = self.vars.get(usize::from(at))?;
                let at = doc.theme().variables.binary_search(name).ok()?;
                Some(ColorRef::Var(VarId(u16::try_from(at).ok()?)))
            }
            other => other,
        };
        let style = |style: &TextStyle| TextStyle {
            color: color(style.color),
            ..style.clone()
        };
        let link = |link: &Option<LinkTarget>| match link {
            Some(LinkTarget::Verb(VerbId(at))) => {
                let id = self.verbs.get(usize::from(*at))?;
                let at = doc.definitions().verbs.iter().position(|verb| verb.id == *id)?;
                Some(LinkTarget::Verb(VerbId(u16::try_from(at).ok()?)))
            }
            other => other.clone(),
        };
        let spans = self
            .spans
            .iter()
            .map(|span| SpanData {
                text: span.text.clone(),
                style: style(&span.style),
                link: link(&span.link),
            })
            .collect();
        (style(&self.style), spans)
    }
}

/// What Copy (`cut: false`) or Cut takes from `selection`: the clipboard,
/// and for Cut the steps of the one request that clears what was taken
/// (empty for Copy). `None` when no selected element has a clipboard
/// form. See the [module docs](self).
///
/// Cost: the selected elements' text, and their children.
pub fn take(
    doc: &Document,
    selection: &Selection,
    cut: bool,
) -> Result<Option<(Clipboard, Vec<DocStep>)>, MutationError> {
    let mut pieces: Vec<String> = Vec::new();
    let mut fragment = None;
    let mut emptied: Vec<NodeId> = Vec::new();
    let mut cleared: Groups<DocKind> = Vec::new();
    let mut steps = Vec::new();
    for id in selection.iter() {
        match kind_of(doc, id) {
            Some(DocKind::Section) => match selection.range() {
                Some((section, start, end)) if section == id => {
                    pieces.push(clusters(&element_text(doc, id), start, end).to_string());
                    if cut {
                        steps.extend(delete_text(doc, id, start, end)?);
                    }
                }
                _ => {
                    pieces.push(element_text(doc, id));
                    fragment = section_fragment(doc, id);
                    emptied.push(id);
                }
            },
            Some(DocKind::Node) => {
                let sections: Vec<NodeId> = children_of_kind(doc, id, DocKind::Section).collect();
                let text: Vec<String> = sections.iter().map(|&s| element_text(doc, s)).collect();
                let text = text.join("\n");
                if !text.is_empty() {
                    pieces.push(text);
                    emptied.extend(sections);
                }
            }
            Some(kind @ (DocKind::Connector | DocKind::Link | DocKind::Label | DocKind::PortalEnd)) => {
                pieces.push(format::hex(resolve_text(doc.tree(), id, &mut ()).color));
                group(&mut cleared, kind, id);
            }
            _ => {}
        }
    }
    if pieces.is_empty() {
        return Ok(None);
    }
    if cut {
        steps.extend(replace_spans(doc, &emptied, &[])?);
        for (kind, ids) in cleared {
            steps.push(set_color(kind, None, ids));
        }
    }
    let section = if pieces.len() == 1 { fragment } else { None };
    let text = pieces.join(MULTI_TARGET_SEPARATOR);
    Ok(Some((Clipboard { text, section }, steps)))
}

/// The steps of the one request that pastes `text` into `selection`,
/// with `clipboard` the app's ([module docs](self)). Empty when no
/// selected element takes a paste.
///
/// Cost: the selected elements' children, and the text's clusters.
pub fn paste(
    doc: &Document,
    selection: &Selection,
    text: &str,
    clipboard: Option<&Clipboard>,
) -> Result<Vec<DocStep>, MutationError> {
    let fragment = clipboard
        .filter(|clipboard| clipboard.text == text)
        .and_then(|clipboard| clipboard.section.as_ref());
    let targets: Vec<NodeId> = selection.iter().collect();
    let pieces = pieces(text, targets.len());
    let mut steps = Vec::new();
    let mut styled: Vec<NodeId> = Vec::new();
    let mut plain: Groups<&str> = Vec::new();
    let mut colored: Groups<(DocKind, Option<ColorRef>)> = Vec::new();
    for (&id, piece) in targets.iter().zip(pieces) {
        match kind_of(doc, id) {
            Some(DocKind::Section) => match selection.range() {
                Some((section, start, end)) if section == id => {
                    steps.extend(delete_text(doc, id, start, end)?);
                    steps.extend(insert_text(doc, id, start, piece.to_string())?);
                }
                _ if fragment.is_some() => styled.push(id),
                _ => group(&mut plain, piece.trim_end(), id),
            },
            Some(DocKind::Node) => {
                if let Some(first) = children_of_kind(doc, id, DocKind::Section).next() {
                    group(&mut plain, piece.trim_end(), first);
                }
            }
            Some(kind @ (DocKind::Connector | DocKind::Link | DocKind::Label | DocKind::PortalEnd)) => {
                match color(doc, piece) {
                    Some(color) => group(&mut colored, (kind, color), id),
                    None => log::debug!(
                        "clipboard: {:?} is not a color; not pasted",
                        baumhard::util::log::excerpt(piece)
                    ),
                }
            }
            _ => {}
        }
    }
    if let (Some(fragment), false) = (fragment, styled.is_empty()) {
        let (style, spans) = fragment.in_doc(doc);
        steps.extend(replace_spans(doc, &styled, &spans)?);
        steps.push(section_style(fragment.align, &style, styled));
    }
    for (text, owners) in plain {
        let spans = [SpanData {
            text: text.to_string(),
            ..SpanData::default()
        }];
        steps.extend(replace_spans(
            doc,
            &owners,
            if text.is_empty() { &[] } else { &spans },
        )?);
    }
    for ((kind, color), ids) in colored {
        steps.push(set_color(kind, color, ids));
    }
    Ok(steps)
}

/// Elements grouped by what they receive, in the order each key was
/// first met.
type Groups<K> = Vec<(K, Vec<NodeId>)>;

/// Add `id` to `key`'s group. O(the groups).
fn group<K: PartialEq>(groups: &mut Groups<K>, key: K, id: NodeId) {
    match groups.iter_mut().find(|(k, _)| *k == key) {
        Some((_, ids)) => ids.push(id),
        None => groups.push((key, vec![id])),
    }
}

/// `text` for `targets` elements: its pieces when there are several
/// targets and it splits into one piece each, and otherwise the whole of
/// it for every one.
fn pieces(text: &str, targets: usize) -> Vec<&str> {
    let split: Vec<&str> = text.split(MULTI_TARGET_SEPARATOR).collect();
    if targets > 1 && split.len() == targets {
        split
    } else {
        vec![text; targets]
    }
}

/// Grapheme clusters `start..end` of `text`, clamped to its end.
fn clusters(text: &str, start: usize, end: usize) -> &str {
    let at = |cluster| find_byte_index_of_grapheme(text, cluster).unwrap_or(text.len());
    &text[at(start)..at(end).max(at(start))]
}

/// `section`'s fragment: its own style fields and its spans.
fn section_fragment(doc: &Document, section: NodeId) -> Option<SectionFragment> {
    let Some(DocElement::Section(SectionData { align, text, .. })) = doc.get(section) else {
        return None;
    };
    let spans = doc
        .children(section)
        .filter_map(|id| match doc.get(id) {
            Some(DocElement::Span(span)) => Some(span.clone()),
            _ => None,
        })
        .collect();
    Some(SectionFragment {
        align: *align,
        style: text.clone(),
        spans,
        vars: doc.theme().variables.clone(),
        verbs: doc
            .definitions()
            .verbs
            .iter()
            .map(|verb| verb.id.clone())
            .collect(),
    })
}

/// The walk that gives `sections` a fragment's own style fields.
fn section_style(align: Option<Align>, style: &TextStyle, sections: Vec<NodeId>) -> DocStep {
    let text = |op| DocMutation::Section(SectionOp::Text(op));
    let mutations = vec![
        DocMutation::Section(SectionOp::SetAlign(align)),
        text(TextOp::SetFont(style.font)),
        text(TextOp::SetSize(style.size)),
        text(TextOp::SetColor(style.color)),
        text(TextOp::SetWeight(style.weight)),
        text(TextOp::SetItalic(style.italic)),
    ];
    let mutator =
        DocMutatorTree::new_with(Mutator::new_macro(mutations, BranchKey::new(DocKind::Section, 0)));
    DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(sections)))
}

/// The walk that sets the own color of `ids`, elements of `kind`: a
/// path's text, a label's text, a portal end's marker.
fn set_color(kind: DocKind, color: Option<ColorRef>, ids: Vec<NodeId>) -> DocStep {
    let mutation = match kind {
        DocKind::Label => DocMutation::Label(LabelOp::Text(TextOp::SetColor(color))),
        DocKind::PortalEnd => DocMutation::PortalEnd(PortalEndOp::SetColor(color)),
        _ => DocMutation::Path(PathOp::SetColor(color)),
    };
    let mutator = DocMutatorTree::new_with(Mutator::new(mutation, BranchKey::new(kind, 0)));
    DocStep::fixed(DocWalk::new(mutator, Anchors::Nodes(ids)))
}

/// The color pasted text names, read as the file reads one
/// (`format::color`): `Some(None)` for empty text, which clears an
/// element's own color; `None` for anything the file refuses.
fn color(doc: &Document, text: &str) -> Option<Option<ColorRef>> {
    let text = text.trim();
    if text.is_empty() {
        return Some(None);
    }
    format::color(doc, text).map(Some)
}
