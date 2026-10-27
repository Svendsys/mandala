// SPDX-License-Identifier: MPL-2.0

//! Composite builders: the sanctioned shape of every change whose steps
//! are not obvious, so no caller derives them again.
//!
//! Each builder reads the document, checks what it was given, and
//! returns the ordered [`DocStep`](crate::document::mutation::step::DocStep)s
//! of the change. It writes nothing: running the steps — on one journal
//! builder, all or nothing — is the writer's (V1-B4). A step that needs
//! something an earlier step creates builds its walk from the document
//! as that step left it.
//!
//! - **Structure is built only as `Mutator::Structure`** (A4): inserts,
//!   removals, moves, splits and merges are engine commands, never
//!   topology code. An inserted node carries
//!   [`NodeIdent::UNASSIGNED`](crate::document::ids::NodeIdent::UNASSIGNED)
//!   until the writer assigns it an ident.
//! - **Nested selections are trimmed to their tops** before a removal or
//!   a move: a node removed or moved with its ancestor goes with it, and
//!   naming it again would fail the walk (`TargetRemovedEarlier`).
//! - **Text is addressed in grapheme clusters** (baumhard §B3). A
//!   section's index counts the clusters of its spans in order.
//! - **Spans are normalized** after every text change: neighbors with an
//!   equal style and link are merged, by repeated merge steps — one walk
//!   merges the head of each pair of equal neighbors, so a run halves at
//!   each step and `ceil(log2(spans)) + 1` steps normalize any section
//!   ([`merge_steps`](crate::document::mutation::compose::merge_steps)).
//!
//! Removing a node does not remove the links that lead to it: only the
//! writer holds the index of incoming links, and finding them otherwise
//! is a scan. The writer appends
//! [`remove_links`](crate::document::mutation::compose::remove_links) for them as a further
//! step of the same request (V1-B4 §2.4(b)).

use crate::document::defaults::DEFAULT_OPACITY;
use crate::document::element::{
    ConnectorData, DocElement, DocKind, NodeData, NodeStyle, PathData, PortalEndData, SectionData, SpanData,
    Triggers,
};
use crate::document::ids::{Document, NodeIdent};
use crate::document::mutation::error::MutationError;
use crate::document::mutation::membership::Membership;
use crate::document::mutation::ops::{DocMutation, DocMutatorTree, LinkOp, SpanOp, TextOp};
use crate::document::mutation::step::{DocStep, DocWalk};
use crate::document::values::{Anchor, LinkKind};
use crate::engine::anchor::Anchors;
use crate::engine::key::BranchKey;
use crate::engine::mutator::Mutator;
use crate::engine::structure::{StructOp, Subtree};
use crate::engine::tree::Placement;
use crate::util::grapheme_chad::count_grapheme_clusters;
use glam::Vec2;
use indextree::NodeId;
use rustc_hash::{FxHashMap, FxHashSet};

/// A mutator tree of one `Single` payload on `kind`.
fn single(mutation: DocMutation, kind: DocKind) -> DocMutatorTree {
    DocMutatorTree::new_with(Mutator::new(mutation, BranchKey::new(kind, 0)))
}

/// A mutator tree of one structural command on `kind`.
fn command(op: StructOp<DocElement>, kind: DocKind) -> DocMutatorTree {
    DocMutatorTree::new_with(Mutator::new_structure(op, BranchKey::new(kind, 0)))
}

/// `mutator` with a `Remove` child for each key in `keys`: every child
/// of the target with that key is removed after the walk. Keys are added
/// in order, which must be ascending, as a mutator row is sorted.
fn removing(
    mut mutator: DocMutatorTree,
    keys: &[BranchKey<DocKind>],
) -> Result<DocMutatorTree, MutationError> {
    let root = mutator.root;
    for &key in keys {
        mutator.insert_child(
            root,
            Mutator::new_structure(StructOp::Remove, key),
            Placement::Last,
        )?;
    }
    Ok(mutator)
}

/// The walk of `mutator` at `nodes`.
fn walk_at(mutator: DocMutatorTree, nodes: Vec<NodeId>) -> DocWalk {
    DocWalk::new(mutator, Anchors::Nodes(nodes))
}

/// `id` if it is an element of `kind` in `doc`'s tree, or the error that
/// names what it should have been.
fn require(doc: &Document, id: NodeId, kind: DocKind, expected: &'static str) -> Result<(), MutationError> {
    match doc.get(id) {
        Some(element) if element.kind() == kind && doc.tree().contains(id) => Ok(()),
        _ => Err(MutationError::NotA {
            element: id,
            expected,
        }),
    }
}

/// A node at `position` with nothing else set, and no ident yet.
fn new_node(position: Vec2) -> DocElement {
    DocElement::Node(NodeData {
        ident: NodeIdent::UNASSIGNED,
        channel: 0,
        position,
        min_size: None,
        style: NodeStyle::default(),
        palette: None,
        zoom: None,
        folded: false,
        hidden: false,
        opacity: DEFAULT_OPACITY,
        on: Triggers::default(),
    })
}

/// A connector with nothing set: a visible line attached automatically
/// at both ends, every style field inherited.
fn new_connector() -> DocElement {
    DocElement::Connector(ConnectorData {
        path: PathData {
            flow: Default::default(),
            anchors: (Anchor::Auto, Anchor::Auto),
            control_points: Vec::new(),
            visible: true,
            zoom: None,
        },
    })
}

/// A section on channel 0 with nothing set and no spans.
fn new_section() -> Subtree<DocElement> {
    Subtree::leaf(DocElement::Section(SectionData::default()))
}

/// The nodes of `nodes` that are not below another of them, each once,
/// in the order given. Every one must be a node in `doc`.
///
/// Cost: amortized O(1) per node and per ancestor of the selection —
/// each ancestor is climbed through once over the whole selection — so
/// nothing per node grows with its depth (§20 rule 11).
fn tops(doc: &Document, nodes: &[NodeId]) -> Result<Vec<NodeId>, MutationError> {
    tops_counting(doc, nodes).0
}

/// [`tops`], and how many ancestors its climbs went through — the
/// membership check's and the nesting check's together — for the tests
/// that hold it to the selection's distinct ancestors (§20 rule 11).
pub(crate) fn tops_counting(doc: &Document, nodes: &[NodeId]) -> (Result<Vec<NodeId>, MutationError>, usize) {
    let mut climbed = 0;
    let result = tops_climbing(doc, nodes, &mut climbed);
    (result, climbed)
}

fn tops_climbing(doc: &Document, nodes: &[NodeId], climbed: &mut usize) -> Result<Vec<NodeId>, MutationError> {
    let tree = doc.tree();
    let arena = tree.arena();
    let given: FxHashSet<NodeId> = nodes.iter().copied().collect();
    let mut members = Membership::default();
    // For every element a climb went through: whether it or one of its
    // ancestors is given. A climb stops at the first element already
    // answered, or at a given one, so each ancestor is climbed through
    // once over the whole selection.
    let mut under_given: FxHashMap<NodeId, bool> = FxHashMap::default();
    let mut climb: Vec<NodeId> = Vec::new();
    let mut seen: FxHashSet<NodeId> = FxHashSet::default();
    let mut tops = Vec::new();
    for &node in nodes {
        if doc.get(node).map(DocElement::kind) != Some(DocKind::Node) || !members.contains(tree, node) {
            return Err(MutationError::NotA {
                element: node,
                expected: "a node",
            });
        }
        let mut at = arena[node].parent();
        let nested = loop {
            let Some(ancestor) = at else {
                break false;
            };
            if given.contains(&ancestor) {
                break true;
            }
            if let Some(&known) = under_given.get(&ancestor) {
                break known;
            }
            climb.push(ancestor);
            *climbed += 1;
            at = arena[ancestor].parent();
        };
        for ancestor in climb.drain(..) {
            under_given.insert(ancestor, nested);
        }
        if !nested && seen.insert(node) {
            tops.push(node);
        }
    }
    *climbed += members.climbed();
    Ok(tops)
}

/// Insert a new node under `parent` at `position`, at the end of its
/// child nodes: a node with one empty section and a connector with
/// nothing set, so it is drawn as an empty box joined to its parent.
///
/// Errors: [`MutationError::NotA`] when `parent` is not a node. One
/// step.
pub fn create_child(doc: &Document, parent: NodeId, position: Vec2) -> Result<Vec<DocStep>, MutationError> {
    require(doc, parent, DocKind::Node, "a node")?;
    let subtree = Subtree {
        root: new_node(position),
        children: vec![new_section(), Subtree::leaf(new_connector())],
    };
    let insert = command(
        StructOp::Insert {
            subtree,
            at: Placement::Last,
        },
        DocKind::Node,
    );
    Ok(vec![DocStep::fixed(walk_at(insert, vec![parent]))])
}

/// Insert a new root node at `position`, after the other root nodes: a
/// node with one empty section and no connector. One step.
pub fn create_root(doc: &Document, position: Vec2) -> Result<Vec<DocStep>, MutationError> {
    let subtree = Subtree {
        root: new_node(position),
        children: vec![new_section()],
    };
    let insert = command(
        StructOp::Insert {
            subtree,
            at: Placement::Last,
        },
        DocKind::Map,
    );
    Ok(vec![DocStep::fixed(walk_at(insert, vec![doc.root()]))])
}

/// Remove `nodes` and their subtrees, trimmed to their tops. The links
/// that lead into the removed subtrees are the writer's next step
/// ([`remove_links`]; see the module docs).
///
/// Errors: [`MutationError::NotA`] for an element that is not a node.
/// One step, or none for no nodes.
pub fn delete(doc: &Document, nodes: &[NodeId]) -> Result<Vec<DocStep>, MutationError> {
    let tops = tops(doc, nodes)?;
    if tops.is_empty() {
        return Ok(Vec::new());
    }
    Ok(vec![DocStep::fixed(walk_at(
        command(StructOp::Remove, DocKind::Node),
        tops,
    ))])
}

/// A step that removes `links`: the links that lead into a subtree an
/// earlier step removed. A link no longer in the tree — removed with
/// its own node in the same request — or not a link is skipped; a step
/// left with none builds nothing.
pub fn remove_links(links: Vec<NodeId>) -> DocStep {
    DocStep::new(move |doc| {
        let mut members = Membership::default();
        let live: Vec<NodeId> = links
            .into_iter()
            .filter(|&l| doc.get(l).map(DocElement::kind) == Some(DocKind::Link) && members.contains(doc.tree(), l))
            .collect();
        Ok((!live.is_empty()).then(|| walk_at(command(StructOp::Remove, DocKind::Link), live)))
    })
}

/// Move `nodes`, trimmed to their tops, under `new_parent`, after its
/// other child nodes. Under a node, a moved node that was a root gets a
/// connector with nothing set; under the map — `new_parent` is the
/// document's root — every moved node's connector is removed.
///
/// Errors: [`MutationError::NotA`] for an element of `nodes` that is
/// not a node, or a `new_parent` that is neither a node nor the map. A
/// node moved under itself or its own subtree is refused when the steps
/// run (`TreeError::Cycle`). One step under the map, two under a node.
pub fn reparent(doc: &Document, nodes: &[NodeId], new_parent: NodeId) -> Result<Vec<DocStep>, MutationError> {
    let tops = tops(doc, nodes)?;
    if tops.is_empty() {
        return Ok(Vec::new());
    }
    let to_map = new_parent == doc.root();
    if !to_map {
        require(doc, new_parent, DocKind::Node, "a node or the map")?;
    }
    let move_to = command(
        StructOp::Move {
            to: new_parent,
            at: Placement::Last,
        },
        DocKind::Node,
    );
    if to_map {
        let move_to = removing(move_to, &[BranchKey::new(DocKind::Connector, 0)])?;
        return Ok(vec![DocStep::fixed(walk_at(move_to, tops))]);
    }
    let roots_of = tops.clone();
    let connect = DocStep::new(move |doc| {
        let roots: Vec<NodeId> = roots_of
            .into_iter()
            .filter(|&n| doc.parent(n) == Some(doc.root()))
            .collect();
        let insert = command(
            StructOp::Insert {
                subtree: Subtree::leaf(new_connector()),
                at: Placement::Last,
            },
            DocKind::Node,
        );
        Ok((!roots.is_empty()).then(|| walk_at(insert, roots)))
    });
    Ok(vec![connect, DocStep::fixed(walk_at(move_to, tops))])
}

/// Switch `link` to `kind`, with the children the new kind holds: a
/// portal gets two portal ends with nothing set and loses its spans,
/// caps and label; a line loses its portal ends (with their markers and
/// labels).
///
/// Errors: [`MutationError::NotA`] when `link` is not a link. No step
/// when it already is of `kind`; one to a line; three to a portal —
/// each end is its own insert, then the switch with the removals.
pub fn set_link_kind(doc: &Document, link: NodeId, kind: LinkKind) -> Result<Vec<DocStep>, MutationError> {
    require(doc, link, DocKind::Link, "a link")?;
    let current = match doc.get(link) {
        Some(DocElement::Link(data)) => data.kind,
        _ => {
            return Err(MutationError::NotA {
                element: link,
                expected: "a link",
            })
        }
    };
    if current == kind {
        return Ok(Vec::new());
    }
    let switch = single(DocMutation::Link(LinkOp::SetKind(kind)), DocKind::Link);
    match kind {
        LinkKind::Line => {
            let switch = removing(
                switch,
                &[
                    BranchKey::new(DocKind::PortalEnd, 0),
                    BranchKey::new(DocKind::PortalEnd, 1),
                ],
            )?;
            Ok(vec![DocStep::fixed(walk_at(switch, vec![link]))])
        }
        LinkKind::Portal => {
            let end = |end: u8| {
                let insert = command(
                    StructOp::Insert {
                        subtree: Subtree::leaf(DocElement::PortalEnd(PortalEndData {
                            end,
                            ..PortalEndData::default()
                        })),
                        at: Placement::Last,
                    },
                    DocKind::Link,
                );
                DocStep::fixed(walk_at(insert, vec![link]))
            };
            let switch = removing(
                switch,
                &[
                    BranchKey::new(DocKind::Span, 0),
                    BranchKey::new(DocKind::CapStart, 0),
                    BranchKey::new(DocKind::CapEnd, 0),
                    BranchKey::new(DocKind::Label, 0),
                ],
            )?;
            Ok(vec![end(0), end(1), DocStep::fixed(walk_at(switch, vec![link]))])
        }
    }
}

/// The spans of `section` with the cluster range each covers in the
/// section, in order. O(text of the section).
fn span_ranges(doc: &Document, section: NodeId) -> Vec<(NodeId, usize, usize)> {
    let mut start = 0;
    doc.children(section)
        .filter_map(|id| match doc.get(id) {
            Some(DocElement::Span(span)) => {
                let end = start + count_grapheme_clusters(&span.text);
                let range = (id, start, end);
                start = end;
                Some(range)
            }
            _ => None,
        })
        .collect()
}

/// How many spans `section` holds.
fn span_count(doc: &Document, section: NodeId) -> usize {
    doc.children(section)
        .filter(|&id| doc.get(id).map(DocElement::kind) == Some(DocKind::Span))
        .count()
}

/// A step that splits the span of `section` that cluster boundary `at`
/// falls strictly inside, so that `at` becomes a span edge. A boundary
/// already on an edge — or outside the text — needs no split: the span
/// split refuses `at == 0` and `at == len` (A4), so it builds nothing.
fn split_at(section: NodeId, at: usize) -> DocStep {
    DocStep::new(move |doc| {
        let inside = span_ranges(doc, section)
            .into_iter()
            .find(|&(_, start, end)| start < at && at < end);
        Ok(inside.map(|(span, start, _)| {
            walk_at(
                command(StructOp::Split { at: at - start }, DocKind::Span),
                vec![span],
            )
        }))
    })
}

/// Whether two spans may merge: equal style and link, as the span merge
/// requires.
fn mergeable(a: &SpanData, b: &SpanData) -> bool {
    a.style == b.style && a.link == b.link
}

/// One normalization step over `section`: merge the head of every run
/// of equal neighbors into the span after it. Pairs are taken left to
/// right without overlap, so no merge names a span another merge of the
/// same walk absorbs. Builds nothing once no equal neighbors remain.
fn merge_step(section: NodeId) -> DocStep {
    DocStep::new(move |doc| {
        let spans: Vec<(NodeId, &SpanData)> = doc
            .children(section)
            .filter_map(|id| match doc.get(id) {
                Some(DocElement::Span(span)) => Some((id, span)),
                _ => None,
            })
            .collect();
        let mut heads = Vec::new();
        let mut i = 0;
        while i + 1 < spans.len() {
            if mergeable(spans[i].1, spans[i + 1].1) {
                heads.push(spans[i].0);
                i += 2;
            } else {
                i += 1;
            }
        }
        Ok((!heads.is_empty()).then(|| walk_at(command(StructOp::Merge, DocKind::Span), heads)))
    })
}

/// How many normalization steps a section that will hold at most
/// `spans` spans needs: `ceil(log2(spans)) + 1` (ruling R14.6 on #237).
///
/// A step merges the head of each pair of equal neighbors, pairs taken
/// left to right without overlap, so a run of `L` equal spans leaves
/// `ceil(L / 2)`, and merging never makes two runs equal — runs end
/// where style or link changes, and a merge keeps both. The longest run
/// is at most `spans` long, so `ceil(log2(spans))` steps merge it to one
/// span; the last step finds nothing, which is what proves the section
/// normalized. O(1).
pub fn merge_steps(spans: usize) -> usize {
    let log2 = if spans <= 1 {
        0
    } else {
        (usize::BITS - (spans - 1).leading_zeros()) as usize
    };
    log2 + 1
}

/// The normalization steps of a section that will hold at most `spans`
/// spans ([`merge_steps`]).
fn normalize(section: NodeId, spans: usize) -> impl Iterator<Item = DocStep> {
    (0..merge_steps(spans)).map(move |_| merge_step(section))
}

/// Apply `op` to the text of `section` between grapheme clusters
/// `start` and `end`: split the spans at both boundaries (skipping a
/// boundary already on a span edge), apply `op` to every span now
/// inside the range, then normalize.
///
/// Errors: [`MutationError::NotA`] when `section` is not a section. No
/// step for an empty range; otherwise two splits, the styling, and
/// [`merge_steps`] of the span count the section can then hold.
pub fn style_range(
    doc: &Document,
    section: NodeId,
    start: usize,
    end: usize,
    op: TextOp,
) -> Result<Vec<DocStep>, MutationError> {
    require(doc, section, DocKind::Section, "a section")?;
    if start >= end {
        return Ok(Vec::new());
    }
    let style = DocStep::new(move |doc| {
        let inside: Vec<NodeId> = span_ranges(doc, section)
            .into_iter()
            .filter(|&(_, s, e)| start <= s && e <= end && s < e)
            .map(|(span, _, _)| span)
            .collect();
        Ok((!inside.is_empty())
            .then(|| walk_at(single(DocMutation::Span(SpanOp::Text(op)), DocKind::Span), inside)))
    });
    let mut steps = vec![split_at(section, start), split_at(section, end), style];
    steps.extend(normalize(section, span_count(doc, section) + 2));
    Ok(steps)
}

/// Insert `text` into `section` before grapheme cluster `at`: into the
/// span that holds that cluster, or at the end of the span before a span
/// edge, so text typed after a span's last cluster takes its style; at
/// or past the end of the text, at the end of the last span; into a new
/// span with nothing set when the section has none. Then normalize.
///
/// Errors: [`MutationError::NotA`] when `section` is not a section. No
/// step for empty text.
pub fn insert_text(
    doc: &Document,
    section: NodeId,
    at: usize,
    text: String,
) -> Result<Vec<DocStep>, MutationError> {
    require(doc, section, DocKind::Section, "a section")?;
    if text.is_empty() {
        return Ok(Vec::new());
    }
    let insert = DocStep::new(move |doc| {
        let ranges = span_ranges(doc, section);
        let target = ranges
            .iter()
            .find(|&&(_, s, e)| s <= at && at <= e)
            .or(ranges.last())
            .copied();
        Ok(Some(match target {
            Some((span, s, e)) => walk_at(
                single(
                    DocMutation::Span(SpanOp::InsertText {
                        at: at.min(e) - s,
                        text,
                    }),
                    DocKind::Span,
                ),
                vec![span],
            ),
            None => {
                let span = DocElement::Span(SpanData {
                    text,
                    ..SpanData::default()
                });
                walk_at(
                    command(
                        StructOp::Insert {
                            subtree: Subtree::leaf(span),
                            at: Placement::Last,
                        },
                        DocKind::Section,
                    ),
                    vec![section],
                )
            }
        }))
    });
    let mut steps = vec![insert];
    steps.extend(normalize(section, span_count(doc, section) + 1));
    Ok(steps)
}

/// Delete grapheme clusters `start..end` of `section`'s text: split the
/// spans at both boundaries, remove every span now inside the range —
/// so no span is left emptied — then normalize, which joins the two
/// parts of a span the range was inside.
///
/// Errors: [`MutationError::NotA`] when `section` is not a section. No
/// step for an empty range; a range past the end deletes to the end.
pub fn delete_text(
    doc: &Document,
    section: NodeId,
    start: usize,
    end: usize,
) -> Result<Vec<DocStep>, MutationError> {
    require(doc, section, DocKind::Section, "a section")?;
    if start >= end {
        return Ok(Vec::new());
    }
    let remove = DocStep::new(move |doc| {
        let inside: Vec<NodeId> = span_ranges(doc, section)
            .into_iter()
            .filter(|&(_, s, e)| start <= s && e <= end && s < e)
            .map(|(span, _, _)| span)
            .collect();
        Ok((!inside.is_empty()).then(|| walk_at(command(StructOp::Remove, DocKind::Span), inside)))
    });
    let mut steps = vec![split_at(section, start), split_at(section, end), remove];
    steps.extend(normalize(section, span_count(doc, section) + 2));
    Ok(steps)
}

/// Split `section` before grapheme cluster `at`: the text from `at` on
/// moves to a new section right after it, which takes the section's
/// channel, offset, alignment and text style but not its minimum size.
/// The spans are split at `at` first when it falls inside one; the ones
/// after it are copied into the new section and removed from this one.
///
/// Errors: [`MutationError::NotA`] when `section` is not a section of a
/// node. Three steps: the split, the new section with its spans, and the
/// removal of the spans it took (none when `at` is at or past the end).
pub fn split_section(doc: &Document, section: NodeId, at: usize) -> Result<Vec<DocStep>, MutationError> {
    require(doc, section, DocKind::Section, "a section")?;
    let node = doc.parent(section).ok_or(MutationError::NotA {
        element: section,
        expected: "a section of a node",
    })?;
    let insert = DocStep::new(move |doc| {
        let Some(DocElement::Section(data)) = doc.get(section) else {
            return Err(MutationError::NotA {
                element: section,
                expected: "a section",
            });
        };
        let children = span_ranges(doc, section)
            .into_iter()
            .filter(|&(_, s, _)| at <= s)
            .filter_map(|(span, _, _)| doc.get(span).cloned().map(Subtree::leaf))
            .collect();
        let new = SectionData {
            min_size: None,
            ..data.clone()
        };
        let insert = command(
            StructOp::Insert {
                subtree: Subtree {
                    root: DocElement::Section(new),
                    children,
                },
                at: Placement::After(section),
            },
            DocKind::Node,
        );
        Ok(Some(walk_at(insert, vec![node])))
    });
    let remove = DocStep::new(move |doc| {
        let taken: Vec<NodeId> = span_ranges(doc, section)
            .into_iter()
            .filter(|&(_, s, _)| at <= s)
            .map(|(span, _, _)| span)
            .collect();
        Ok((!taken.is_empty()).then(|| walk_at(command(StructOp::Remove, DocKind::Span), taken)))
    });
    Ok(vec![split_at(section, at), insert, remove])
}

/// Add an empty section to `node`: right after the section `after`,
/// on its channel, or before every section on channel 0 when `after`
/// is `None`.
///
/// Errors: [`MutationError::NotA`] when `node` is not a node or `after`
/// is not one of its sections. One step.
pub fn add_section(
    doc: &Document,
    node: NodeId,
    after: Option<NodeId>,
) -> Result<Vec<DocStep>, MutationError> {
    require(doc, node, DocKind::Node, "a node")?;
    let (channel, at) = match after {
        Some(after) => match doc.get(after) {
            Some(DocElement::Section(section))
                if doc.parent(after) == Some(node) && doc.tree().contains(after) =>
            {
                (section.channel, Placement::After(after))
            }
            _ => {
                return Err(MutationError::NotA {
                    element: after,
                    expected: "a section of the node",
                })
            }
        },
        None => (0, Placement::First),
    };
    let section = DocElement::Section(SectionData {
        channel,
        ..SectionData::default()
    });
    let insert = command(
        StructOp::Insert {
            subtree: Subtree::leaf(section),
            at,
        },
        DocKind::Node,
    );
    Ok(vec![DocStep::fixed(walk_at(insert, vec![node]))])
}
