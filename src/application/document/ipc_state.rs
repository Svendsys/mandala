// SPDX-License-Identifier: MPL-2.0

//! IPC projections of document-layer state (`format/ipc.md` §`query`).
//!
//! Each projection sits here, beside the state it exposes, as an
//! inherent method that takes `&self` — a read cannot mutate, so a
//! query can neither flip `dirty`, push an undo entry nor touch a
//! scene signature (CODE_CONVENTIONS §2: the seam is the state's own
//! accessor, not a parallel copy of it). There are no mirror structs
//! to keep in sync (the drift class #24 / #32 exist to kill):
//! where a type is pure serde (`MindMap`, `AnimationTiming`) the
//! projection *is* its serializer, and where a type owns
//! non-serializable internals the projection is a stated subset whose
//! omissions are named at its method and in `format/ipc.md`.
//!
//! **Every projection is an exhaustive `match` with no wildcard arm.**
//! A new `SelectionState` or `UndoAction` variant therefore breaks the
//! build *here*, on purpose, until the wire form of the new variant
//! is decided and documented (the same SSOT posture as the
//! `ActionClassify` taxonomy). The `kind` word of each variant is the
//! variant's own name.
//!
//! Native-only with the IPC module that consumes it (`CLAUDE.md`
//! "Dual-target status"): the browser serves no socket.

use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::{json, Value};

use baumhard::mindmap::model::MindMap;
use baumhard::mindmap::scene_cache::EdgeKey;

use super::nodes::BorderPreviewTarget;
use super::{AnimationInstance, EdgeRef, MindMapDocument, SectionSel, SelectionState, UndoAction};

/// An `f32` as the JSON number a reader expects: the shortest decimal
/// that names it (`0.1`, not `0.10000000149011612`), through
/// Baumhard's [`shortest_f64`](baumhard::util::shortest_float::shortest_f64),
/// the rule the scene dumps serialize by too. A non-finite value, which
/// JSON cannot carry, is `null`.
pub(crate) fn f32_json(value: f32) -> Value {
    baumhard::util::shortest_float::shortest_f64(value)
        .and_then(serde_json::Number::from_f64)
        .map_or(Value::Null, Value::Number)
}

/// `{"from_id", "to_id", "edge_type"}`: an edge's identity triple.
fn edge_json(from_id: &str, to_id: &str, edge_type: &str) -> Value {
    json!({ "from_id": from_id, "to_id": to_id, "edge_type": edge_type })
}

fn edge_key_json(key: &EdgeKey) -> Value {
    edge_json(&key.from_id, &key.to_id, &key.edge_type)
}

impl EdgeRef {
    /// The edge's identity triple, as it appears everywhere on the
    /// wire an edge is named.
    pub(crate) fn to_ipc_json(&self) -> Value {
        edge_json(&self.from_id, &self.to_id, &self.edge_type)
    }
}

impl SectionSel {
    /// `{"node_id", "section_idx"}`.
    pub(crate) fn to_ipc_json(&self) -> Value {
        json!({ "node_id": self.node_id, "section_idx": self.section_idx })
    }
}

impl SelectionState {
    /// The selection as `query.selection` reports it: an object whose
    /// `kind` is the variant's name, with that variant's ids beside
    /// it. Lossless: every field of every variant is carried.
    pub(crate) fn to_ipc_json(&self) -> Value {
        match self {
            SelectionState::None => json!({ "kind": "None" }),
            SelectionState::Single(node_id) => json!({ "kind": "Single", "node_id": node_id }),
            SelectionState::Multi(node_ids) => json!({ "kind": "Multi", "node_ids": node_ids }),
            SelectionState::Section(sel) => json!({
                "kind": "Section",
                "node_id": sel.node_id,
                "section_idx": sel.section_idx,
            }),
            SelectionState::MultiSection(sels) => json!({
                "kind": "MultiSection",
                "sections": sels.iter().map(SectionSel::to_ipc_json).collect::<Vec<_>>(),
            }),
            SelectionState::SectionRange {
                sel,
                section_span,
                grapheme_range,
            } => json!({
                "kind": "SectionRange",
                "node_id": sel.node_id,
                "section_idx": sel.section_idx,
                "section_span": { "lo": section_span.lo(), "hi": section_span.hi() },
                "grapheme_range": { "start": grapheme_range.start(), "end": grapheme_range.end() },
            }),
            SelectionState::Edge(edge) => json!({ "kind": "Edge", "edge": edge.to_ipc_json() }),
            SelectionState::EdgeLabel(label) => json!({
                "kind": "EdgeLabel",
                "edge": label.edge_ref.to_ipc_json(),
            }),
            SelectionState::PortalLabel(sel) => json!({
                "kind": "PortalLabel",
                "edge": edge_key_json(&sel.edge_key),
                "endpoint_node_id": sel.endpoint_node_id,
            }),
            SelectionState::PortalText(sel) => json!({
                "kind": "PortalText",
                "edge": edge_key_json(&sel.edge_key),
                "endpoint_node_id": sel.endpoint_node_id,
            }),
        }
    }
}

impl AnimationInstance {
    /// One running animation as `query.animations` reports it, read
    /// against `now_ms`, the app's wall clock (`common::now_ms()`,
    /// milliseconds since process start) and the same clock the
    /// per-frame drain ticks animations against. (A virtual clock,
    /// which would make this deterministic, is IPC-09's `clock`
    /// family and does not exist yet.)
    ///
    /// Projection: the target, the mutation id, the clock fields and
    /// the timing envelope (the type's own serializer). **Omitted by
    /// name:** `from_node` / `to_node` (two whole-node snapshots, the
    /// document's own business, and large) and `cm`'s mutation list.
    ///
    /// `paused` is whether a tree-mutating drag has suspended the
    /// tick. The caller then passes the clock *frozen at the pause's
    /// start*, because that is the value the resume shift
    /// (`shift_active_animations_start_ms`) restores, and `phase` is
    /// `"paused"`. Otherwise `phase` is `"delay"` before the delay
    /// elapses, `"running"` while it interpolates, and `"due"` once
    /// the full envelope has elapsed and the next tick commits it.
    /// `progress` is the linear `0..=1` position through
    /// `duration_ms`, before easing.
    pub(crate) fn to_ipc_json(&self, now_ms: u64, paused: bool) -> Value {
        let timing = self.timing();
        let elapsed = now_ms.saturating_sub(self.start_ms);
        let delay = u64::from(timing.delay_ms);
        let duration = u64::from(timing.duration_ms);
        let (phase, progress) = if elapsed < delay {
            ("delay", 0.0)
        } else if elapsed >= delay + duration {
            ("due", 1.0)
        } else {
            // `elapsed < delay + duration` makes `duration > 0` here.
            ("running", (elapsed - delay) as f64 / duration as f64)
        };
        json!({
            "target_id": self.target_id,
            "section_idx": self.section_idx,
            "mutation_id": self.mutation_id(),
            "start_ms": self.start_ms,
            "elapsed_ms": elapsed,
            "phase": if paused { "paused" } else { phase },
            "progress": progress,
            "timing": serde_json::to_value(timing).unwrap_or(Value::Null),
        })
    }
}

impl UndoAction {
    /// The entry as `query.undo` reports it: `kind` (the variant's
    /// name) plus what it touches, `node_ids` and/or an edge `index`.
    ///
    /// Projection, not the snapshot: the before-images an entry
    /// carries (whole nodes, section arrays, a canvas) are what undo
    /// restores from, not what a reader of the stack needs.
    pub(crate) fn to_ipc_json(&self) -> Value {
        match self {
            UndoAction::MoveNodes { original_positions } => json!({
                "kind": "MoveNodes",
                "node_ids": original_positions.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            }),
            UndoAction::CustomMutation { node_snapshots } => json!({
                "kind": "CustomMutation",
                "node_ids": node_snapshots.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            }),
            UndoAction::ReparentNodes { entries, .. } => json!({
                "kind": "ReparentNodes",
                "node_ids": entries.iter().map(|(id, _)| id).collect::<Vec<_>>(),
            }),
            UndoAction::DeleteEdge { index, .. } => json!({ "kind": "DeleteEdge", "index": index }),
            UndoAction::CreateEdge { index } => json!({ "kind": "CreateEdge", "index": index }),
            UndoAction::EditEdge { index, .. } => json!({ "kind": "EditEdge", "index": index }),
            UndoAction::CreateNode { node_id } => {
                json!({ "kind": "CreateNode", "node_ids": [node_id] })
            }
            UndoAction::EditNodeText { node_id, .. } => {
                json!({ "kind": "EditNodeText", "node_ids": [node_id] })
            }
            UndoAction::EditNodeStyle { node_id, .. } => {
                json!({ "kind": "EditNodeStyle", "node_ids": [node_id] })
            }
            UndoAction::EditNodeZoom { node_id, .. } => {
                json!({ "kind": "EditNodeZoom", "node_ids": [node_id] })
            }
            UndoAction::CanvasSnapshot { .. } => json!({ "kind": "CanvasSnapshot" }),
            UndoAction::EditNodeAabb { node_id, .. } => {
                json!({ "kind": "EditNodeAabb", "node_ids": [node_id] })
            }
            UndoAction::DeleteNode { node, .. } => {
                json!({ "kind": "DeleteNode", "node_ids": [node.id] })
            }
        }
    }
}

/// One selectable part of `query.document`'s reply. The wire name of
/// each is [`Self::name`], and [`Self::ALL`] is the order they appear
/// in the document and in the reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentField {
    /// The bound file path, or `null` for a document never saved.
    FilePath,
    /// The unsaved-changes flag.
    Dirty,
    /// How many entries the undo stack holds.
    UndoDepth,
    /// `{"name", "version"}` of the loaded map.
    Map,
    /// `{"nodes", "edges", "sections", "palettes", "custom_mutations", "macros"}`.
    Counts,
    /// The whole `MindMap`, through its own serde: exactly the shape a
    /// save writes (minus the loader's unknown-key splice).
    Mindmap,
}

impl DocumentField {
    /// Every field, in reply order.
    pub(crate) const ALL: [DocumentField; 6] = [
        DocumentField::FilePath,
        DocumentField::Dirty,
        DocumentField::UndoDepth,
        DocumentField::Map,
        DocumentField::Counts,
        DocumentField::Mindmap,
    ];

    /// What a request that names no `fields` gets: everything but the
    /// model, which is the one part whose size is unbounded.
    pub(crate) fn is_default(self) -> bool {
        !matches!(self, DocumentField::Mindmap)
    }

    /// The wire word.
    pub(crate) fn name(self) -> &'static str {
        match self {
            DocumentField::FilePath => "file_path",
            DocumentField::Dirty => "dirty",
            DocumentField::UndoDepth => "undo_depth",
            DocumentField::Map => "map",
            DocumentField::Counts => "counts",
            DocumentField::Mindmap => "mindmap",
        }
    }

    /// The field a wire word names.
    pub(crate) fn parse(word: &str) -> Option<DocumentField> {
        DocumentField::ALL.into_iter().find(|f| f.name() == word)
    }
}

impl MindMapDocument {
    /// One part of `query.document`'s reply, written through
    /// `serializer`. The one body behind both the inline reply (a
    /// `serde_json::Value`) and the `to_file` stream, so the two
    /// cannot differ. [`DocumentField::Mindmap`] serializes the model
    /// straight from `&self.mindmap`, with no intermediate value tree.
    fn serialize_field<S: Serializer>(&self, field: DocumentField, serializer: S) -> Result<S::Ok, S::Error> {
        match field {
            DocumentField::FilePath => self.file_path.serialize(serializer),
            DocumentField::Dirty => self.dirty.serialize(serializer),
            DocumentField::UndoDepth => self.undo_stack.len().serialize(serializer),
            DocumentField::Map => json!({
                "name": self.mindmap.name,
                "version": self.mindmap.version,
            })
            .serialize(serializer),
            DocumentField::Counts => json!({
                "nodes": self.mindmap.nodes.len(),
                "edges": self.mindmap.edges.len(),
                "sections": self.mindmap.nodes.values().map(|n| n.sections.len()).sum::<usize>(),
                "palettes": self.mindmap.palettes.len(),
                "custom_mutations": self.mindmap.custom_mutations.len(),
                "macros": self.mindmap.macros.len(),
            })
            .serialize(serializer),
            DocumentField::Mindmap => self.mindmap.serialize(serializer),
        }
    }

    /// One part of `query.document`'s reply as a value.
    ///
    /// Cost: every field but [`DocumentField::Mindmap`] and
    /// [`DocumentField::Counts`] is O(1); `Counts` walks the nodes
    /// once and `Mindmap` builds the whole model as a value tree,
    /// which is why a request names it explicitly (and why
    /// `to_file` and `pointer` do not go through here).
    // Test-gated: the reply is built through `DocumentReply`; this is
    // the single-field read the tests assert against.
    #[cfg(test)]
    pub(crate) fn ipc_field_json(&self, field: DocumentField) -> Value {
        self.serialize_field(field, serde_json::value::Serializer)
            .unwrap_or(Value::Null)
    }

    /// The value at JSON Pointer `pointer` (RFC 6901) in the
    /// serialized model, or `None` when it names nothing there.
    /// Equivalent to `serde_json::to_value(&self.mindmap)?.pointer(p)`,
    /// but builds a value tree only for the **selected subtree**: the
    /// first one or two tokens are resolved against the typed model
    /// (`/nodes/<id>`, `/edges/<index>`, `/palettes/<name>`, …), so a
    /// poll for `/nodes/0/position` costs one node's serialization
    /// and not the whole map's. Only the empty pointer, which names
    /// the whole model, builds all of it.
    ///
    /// The destructure of `MindMap` below names every field and has
    /// no `..`: a new serialized field breaks the build here until it
    /// is given a pointer arm, and `skip_serializing_if` fields
    /// (`palettes`, `custom_mutations`, `macros`) are absent while
    /// empty, as they are in the serialized form.
    pub(crate) fn ipc_model_pointer(&self, pointer: &str) -> Option<Value> {
        let tokens = pointer_tokens(pointer)?;
        let MindMap {
            version,
            name,
            canvas,
            palettes,
            nodes,
            edges,
            custom_mutations,
            macros,
            unknown_keys: _,
            skipped_constructs: _,
        } = &self.mindmap;
        let Some((first, rest)) = tokens.split_first() else {
            return serde_json::to_value(&self.mindmap).ok();
        };
        let (subtree, rest) = match first.as_str() {
            "version" => (serde_json::to_value(version).ok()?, rest),
            "name" => (serde_json::to_value(name).ok()?, rest),
            "canvas" => (serde_json::to_value(canvas).ok()?, rest),
            "palettes" if !palettes.is_empty() => keyed(palettes, rest)?,
            "nodes" => keyed(nodes, rest)?,
            "edges" => indexed(edges, rest)?,
            "custom_mutations" if !custom_mutations.is_empty() => indexed(custom_mutations, rest)?,
            "macros" if !macros.is_empty() => indexed(macros, rest)?,
            _ => return None,
        };
        descend(subtree, rest)
    }

    /// `query.animations`: whether any instance is running, whether a
    /// tree-mutating drag has suspended the tick (`paused_since`, the
    /// app-clock ms the pause began, `None` when not paused), and
    /// every instance, in the order they were started.
    ///
    /// While paused the instances are read at the *pause's start*:
    /// the drain skips ticks for the whole drag, and on release shifts
    /// each `start_ms` forward by the pause, so the elapsed time at
    /// the pause's start is exactly what the animation resumes from.
    /// A reply that kept advancing `elapsed_ms` through the drag
    /// would promise a commit the next tick does not make.
    pub(crate) fn ipc_animations_json(&self, now_ms: u64, paused_since: Option<u64>) -> Value {
        let clock = paused_since.map_or(now_ms, |since| since.min(now_ms));
        json!({
            "active": self.has_active_animations(),
            "paused": paused_since.is_some(),
            "count": self.active_animations.len(),
            "animations": self
                .active_animations
                .iter()
                .map(|a| a.to_ipc_json(clock, paused_since.is_some()))
                .collect::<Vec<_>>(),
        })
    }

    /// `query.undo`: the stack's depth and its newest `limit` entries,
    /// **newest first** (the order `Undo` will take them).
    pub(crate) fn ipc_undo_json(&self, limit: usize) -> Value {
        json!({
            "depth": self.undo_stack.len(),
            "entries": self
                .undo_stack
                .iter()
                .rev()
                .take(limit)
                .map(UndoAction::to_ipc_json)
                .collect::<Vec<_>>(),
        })
    }

    /// The transient `*_preview` substitutions the scene is showing
    /// in place of committed model state, each `null` when none is
    /// live. They never serialize, push undo or flip `dirty`
    /// (that is what makes them previews), so an agent that sees
    /// pixels disagree with `query.document` reads them here.
    ///
    /// An associated function over `Option<&Self>` so that **one**
    /// list of keys serves both the document and no document at all,
    /// where every preview is `null`. A preview added here is
    /// therefore present in both replies by construction.
    ///
    /// **Omitted by name:** a border preview's `edits` (a
    /// `BorderConfigEdits` bundle with no serializer): the target says
    /// *what* is being previewed, not the staged values.
    pub(crate) fn ipc_previews_json(doc: Option<&Self>) -> Value {
        json!({
            "label_edit": doc.and_then(|d| d.label_edit_preview.as_ref()).map(|(key, buffer)| json!({
                "edge": edge_key_json(key),
                "buffer": buffer,
            })),
            "portal_text_edit": doc
                .and_then(|d| d.portal_text_edit_preview.as_ref())
                .map(|(key, endpoint, buffer)| json!({
                    "edge": edge_key_json(key),
                    "endpoint_node_id": endpoint,
                    "buffer": buffer,
                })),
            "color_picker": doc.and_then(|d| d.color_picker_preview.as_ref()).map(|p| json!({
                "edge": edge_key_json(&p.key),
                "color": p.color,
            })),
            "border": doc.and_then(|d| d.border_preview.as_ref()).map(|p| match &p.target {
                BorderPreviewTarget::Nodes(ids) => json!({ "kind": "Nodes", "node_ids": ids }),
                BorderPreviewTarget::Sections(sections) => json!({
                    "kind": "Sections",
                    "sections": sections
                        .iter()
                        .map(|(node_id, section_idx)| json!({ "node_id": node_id, "section_idx": section_idx }))
                        .collect::<Vec<_>>(),
                }),
                BorderPreviewTarget::CanvasDefault => json!({ "kind": "CanvasDefault" }),
                BorderPreviewTarget::CanvasSectionFrame => json!({ "kind": "CanvasSectionFrame" }),
                BorderPreviewTarget::CanvasSectionFrameFocused => {
                    json!({ "kind": "CanvasSectionFrameFocused" })
                }
            }),
            "rect_select": doc.and_then(|d| d.rect_select_preview()),
        })
    }
}

/// The JSON Pointer's reference tokens, unescaped (`~1` is `/`, then
/// `~0` is `~`, in that order, per RFC 6901). `""` is the empty
/// token list, the whole document; any other pointer must begin with
/// `/`.
fn pointer_tokens(pointer: &str) -> Option<Vec<String>> {
    if pointer.is_empty() {
        return Some(Vec::new());
    }
    Some(
        pointer
            .strip_prefix('/')?
            .split('/')
            .map(|token| token.replace("~1", "/").replace("~0", "~"))
            .collect(),
    )
}

/// An array index token: `0`, or digits with no leading zero and no
/// sign (RFC 6901, and what `Value::pointer` accepts).
fn array_index(token: &str) -> Option<usize> {
    let well_formed = token == "0"
        || (token.starts_with(|c: char| ('1'..='9').contains(&c))
            && token.bytes().all(|b| b.is_ascii_digit()));
    if well_formed {
        token.parse().ok()
    } else {
        None
    }
}

/// The value of a map-shaped top-level field: the entry `rest`'s
/// first token names, or the whole collection when there is none.
fn keyed<'t, V: Serialize>(
    entries: &std::collections::HashMap<String, V>,
    rest: &'t [String],
) -> Option<(Value, &'t [String])> {
    match rest.split_first() {
        Some((key, rest)) => Some((serde_json::to_value(entries.get(key)?).ok()?, rest)),
        None => Some((serde_json::to_value(entries).ok()?, rest)),
    }
}

/// [`keyed`], for an array-shaped top-level field.
fn indexed<'t, V: Serialize>(items: &[V], rest: &'t [String]) -> Option<(Value, &'t [String])> {
    match rest.split_first() {
        Some((index, rest)) => Some((serde_json::to_value(items.get(array_index(index)?)?).ok()?, rest)),
        None => Some((serde_json::to_value(items).ok()?, rest)),
    }
}

/// Walk `tokens` down `value`, consuming it, and return what they
/// name. Moves the answer out rather than cloning it.
fn descend(mut value: Value, tokens: &[String]) -> Option<Value> {
    for token in tokens {
        value = match value {
            Value::Object(mut map) => map.remove(token)?,
            Value::Array(mut items) => {
                let index = array_index(token)?;
                if index >= items.len() {
                    return None;
                }
                items.swap_remove(index)
            }
            _ => return None,
        };
    }
    Some(value)
}

/// `query.document`'s reply: the chosen [`DocumentField`]s of one
/// document, as a serializable object. Serialize it to a `Value` for an
/// inline reply, or **stream it into a writer** for `to_file`: either
/// way the model is serialized straight from `&MindMap`, with no value
/// tree held beside it. Only a `pointer` into the model builds a value,
/// and only for the subtree it selects.
pub(crate) struct DocumentReply<'a> {
    doc: &'a MindMapDocument,
    fields: &'a [DocumentField],
    /// The value a `pointer` selected, standing in for the whole
    /// model under [`DocumentField::Mindmap`].
    subtree: Option<Value>,
}

/// A `pointer` that names nothing in the model.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PointerMiss;

impl MindMapDocument {
    /// The reply for `fields`, with `pointer` (when the model is among
    /// them) resolved **up front**, so a miss is known before anything
    /// is written anywhere and serializing the reply cannot fail on it.
    pub(crate) fn ipc_document_reply<'a>(
        &'a self,
        fields: &'a [DocumentField],
        pointer: Option<&str>,
    ) -> Result<DocumentReply<'a>, PointerMiss> {
        let subtree = match pointer {
            Some(pointer) if fields.contains(&DocumentField::Mindmap) => {
                Some(self.ipc_model_pointer(pointer).ok_or(PointerMiss)?)
            }
            _ => None,
        };
        Ok(DocumentReply {
            doc: self,
            fields,
            subtree,
        })
    }
}

impl Serialize for DocumentReply<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        /// One field's value, serialized in place.
        struct Entry<'r, 'a>(&'r DocumentReply<'a>, DocumentField);

        impl Serialize for Entry<'_, '_> {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                match (&self.0.subtree, self.1) {
                    (Some(subtree), DocumentField::Mindmap) => subtree.serialize(serializer),
                    (_, field) => self.0.doc.serialize_field(field, serializer),
                }
            }
        }

        let mut map = serializer.serialize_map(Some(self.fields.len()))?;
        for &field in self.fields {
            map.serialize_entry(field.name(), &Entry(self, field))?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::document::tests_common::{load_test_doc, TestNudgeMutation};
    use crate::application::document::{
        EdgeLabelSel, GraphemeRange, PortalLabelSel, SectionSpan, SelectionState,
    };
    use baumhard::mindmap::model::MindMap;

    fn edge() -> EdgeRef {
        EdgeRef::new("a", "b", "cross_link")
    }

    fn portal_sel() -> PortalLabelSel {
        PortalLabelSel {
            edge_key: EdgeKey::from(&edge()),
            endpoint_node_id: "b".into(),
        }
    }

    /// One value of **every** `SelectionState` variant. The match is
    /// the completeness guard: it has no wildcard, so a new variant is
    /// a compile error here until it gets a value in this list *and*
    /// a wire form in `to_ipc_json`.
    fn every_selection() -> Vec<SelectionState> {
        let all = vec![
            SelectionState::None,
            SelectionState::Single("1".into()),
            SelectionState::Multi(vec!["1".into(), "2".into()]),
            SelectionState::Section(SectionSel::new("1", 2)),
            SelectionState::MultiSection(vec![SectionSel::new("1", 0), SectionSel::new("2", 1)]),
            SelectionState::SectionRange {
                sel: SectionSel::new("1", 3),
                section_span: SectionSpan::new(4, 2),
                grapheme_range: GraphemeRange::new(9, 5),
            },
            SelectionState::Edge(edge()),
            SelectionState::EdgeLabel(EdgeLabelSel::new(edge())),
            SelectionState::PortalLabel(portal_sel()),
            SelectionState::PortalText(portal_sel()),
        ];
        for selection in &all {
            match selection {
                SelectionState::None
                | SelectionState::Single(_)
                | SelectionState::Multi(_)
                | SelectionState::Section(_)
                | SelectionState::MultiSection(_)
                | SelectionState::SectionRange { .. }
                | SelectionState::Edge(_)
                | SelectionState::EdgeLabel(_)
                | SelectionState::PortalLabel(_)
                | SelectionState::PortalText(_) => {}
            }
        }
        all
    }

    /// Every variant serializes, to an object whose `kind` is the
    /// variant's own name, and the ten kinds are distinct.
    #[test]
    fn test_every_selection_variant_serializes_under_its_own_name() {
        let mut kinds = Vec::new();
        for selection in every_selection() {
            let json = selection.to_ipc_json();
            let kind = json["kind"].as_str().expect("every selection carries a kind");
            let debug = format!("{selection:?}");
            assert!(
                debug.starts_with(kind),
                "kind `{kind}` is not the variant name in {debug}"
            );
            kinds.push(kind.to_string());
        }
        let mut deduped = kinds.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(deduped.len(), kinds.len(), "kinds are distinct: {kinds:?}");
        assert_eq!(kinds.len(), 10, "SelectionState has ten variants");
    }

    #[test]
    fn test_selection_payloads_are_lossless() {
        assert_eq!(
            SelectionState::Multi(vec!["1".into(), "2".into()]).to_ipc_json(),
            json!({ "kind": "Multi", "node_ids": ["1", "2"] })
        );
        assert_eq!(
            SelectionState::SectionRange {
                sel: SectionSel::new("1", 3),
                section_span: SectionSpan::new(4, 2),
                grapheme_range: GraphemeRange::new(9, 5),
            }
            .to_ipc_json(),
            json!({
                "kind": "SectionRange", "node_id": "1", "section_idx": 3,
                "section_span": { "lo": 2, "hi": 4 },
                "grapheme_range": { "start": 5, "end": 9 },
            }),
            "the constructors normalize order, and the wire carries the normalized pair"
        );
        assert_eq!(
            SelectionState::PortalText(portal_sel()).to_ipc_json(),
            json!({
                "kind": "PortalText",
                "edge": { "from_id": "a", "to_id": "b", "edge_type": "cross_link" },
                "endpoint_node_id": "b",
            })
        );
    }

    /// `UndoAction` variants, one of each. Same no-wildcard guard.
    fn every_undo_action() -> Vec<UndoAction> {
        use baumhard::mindmap::model::{Canvas, MindEdge, MindNode, Position, Size};
        let doc = load_test_doc();
        let node: MindNode = doc
            .mindmap
            .nodes
            .values()
            .next()
            .expect("fixture has nodes")
            .clone();
        let edge: MindEdge = doc.mindmap.edges.first().expect("fixture has edges").clone();
        let all = vec![
            UndoAction::MoveNodes {
                original_positions: vec![("1".into(), Position { x: 0.0, y: 0.0 })],
            },
            UndoAction::CustomMutation {
                node_snapshots: vec![("1".into(), node.clone())],
            },
            UndoAction::ReparentNodes {
                entries: vec![("1".into(), None)],
                old_edges: vec![],
            },
            UndoAction::DeleteEdge {
                index: 3,
                edge: edge.clone(),
            },
            UndoAction::CreateEdge { index: 4 },
            UndoAction::EditEdge {
                index: 5,
                before: edge.clone(),
            },
            UndoAction::CreateNode { node_id: "n".into() },
            UndoAction::EditNodeText {
                node_id: "n".into(),
                before_sections: vec![],
                before_position: Position { x: 0.0, y: 0.0 },
                before_size: Size {
                    width: 1.0,
                    height: 1.0,
                },
                before_selection: SelectionState::None,
            },
            UndoAction::EditNodeStyle {
                node_id: "n".into(),
                before_style: node.style.clone(),
                before_color_schema: None,
                before_sections: vec![],
                before_position: Position { x: 0.0, y: 0.0 },
                before_size: Size {
                    width: 1.0,
                    height: 1.0,
                },
                before_selection: SelectionState::None,
            },
            UndoAction::EditNodeZoom {
                node_id: "n".into(),
                before_min: None,
                before_max: None,
            },
            UndoAction::CanvasSnapshot {
                canvas: Box::new(Canvas::default()),
            },
            UndoAction::EditNodeAabb {
                node_id: "n".into(),
                before_position: Position { x: 0.0, y: 0.0 },
                before_size: Size {
                    width: 1.0,
                    height: 1.0,
                },
            },
            UndoAction::DeleteNode {
                node,
                removed_edges: vec![],
                orphaned_children: vec![],
            },
        ];
        for action in &all {
            match action {
                UndoAction::MoveNodes { .. }
                | UndoAction::CustomMutation { .. }
                | UndoAction::ReparentNodes { .. }
                | UndoAction::DeleteEdge { .. }
                | UndoAction::CreateEdge { .. }
                | UndoAction::EditEdge { .. }
                | UndoAction::CreateNode { .. }
                | UndoAction::EditNodeText { .. }
                | UndoAction::EditNodeStyle { .. }
                | UndoAction::EditNodeZoom { .. }
                | UndoAction::CanvasSnapshot { .. }
                | UndoAction::EditNodeAabb { .. }
                | UndoAction::DeleteNode { .. } => {}
            }
        }
        all
    }

    #[test]
    fn test_every_undo_variant_serializes_under_its_own_name() {
        let all = every_undo_action();
        assert_eq!(all.len(), 13, "UndoAction has thirteen variants");
        for action in &all {
            let json = action.to_ipc_json();
            let kind = json["kind"].as_str().expect("every entry carries a kind");
            let debug = format!("{action:?}");
            assert!(
                debug.starts_with(kind),
                "kind `{kind}` is not the variant name in {debug}"
            );
            assert!(
                json.get("node_ids").is_some() || json.get("index").is_some() || kind == "CanvasSnapshot",
                "{kind} names what it touches: {json}"
            );
        }
    }

    #[test]
    fn test_undo_json_reports_newest_first_and_honors_the_limit() {
        let mut doc = load_test_doc();
        doc.undo_stack = vec![
            UndoAction::CreateEdge { index: 0 },
            UndoAction::CreateEdge { index: 1 },
            UndoAction::CreateEdge { index: 2 },
        ];
        let json = doc.ipc_undo_json(2);
        assert_eq!(json["depth"], json!(3));
        assert_eq!(
            json["entries"],
            json!([
                { "kind": "CreateEdge", "index": 2 },
                { "kind": "CreateEdge", "index": 1 },
            ])
        );
        assert_eq!(doc.ipc_undo_json(0)["entries"], json!([]));
        assert_eq!(doc.ipc_undo_json(99)["entries"].as_array().map(Vec::len), Some(3));
    }

    /// **`query.document` byte-round-trips against the loaded model**
    /// (issue #64's acceptance criterion): the `mindmap` field,
    /// parsed back as a `MindMap` and serialized again, is the value
    /// the document itself serializes to.
    #[test]
    fn test_the_mindmap_field_round_trips_through_serde() {
        let doc = load_test_doc();
        let wire = doc.ipc_field_json(DocumentField::Mindmap);
        assert!(wire.is_object(), "the model serializes to an object");
        let reparsed: MindMap = serde_json::from_value(wire.clone()).expect("the wire form is a MindMap");
        assert_eq!(
            serde_json::to_value(&reparsed).expect("re-serializes"),
            wire,
            "the reply is the model's own serde, with nothing added or lost"
        );
        assert_eq!(reparsed.nodes.len(), doc.mindmap.nodes.len());
        assert_eq!(reparsed.edges.len(), doc.mindmap.edges.len());
    }

    #[test]
    fn test_document_fields_report_the_documents_own_state() {
        let mut doc = load_test_doc();
        doc.file_path = Some("/tmp/x.mindmap.json".into());
        doc.dirty = true;
        doc.undo_stack.push(UndoAction::CreateEdge { index: 0 });
        assert_eq!(
            doc.ipc_field_json(DocumentField::FilePath),
            json!("/tmp/x.mindmap.json")
        );
        assert_eq!(doc.ipc_field_json(DocumentField::Dirty), json!(true));
        assert_eq!(doc.ipc_field_json(DocumentField::UndoDepth), json!(1));
        assert_eq!(
            doc.ipc_field_json(DocumentField::Map)["name"],
            json!(doc.mindmap.name)
        );
        let counts = doc.ipc_field_json(DocumentField::Counts);
        assert_eq!(counts["nodes"], json!(doc.mindmap.nodes.len()));
        assert_eq!(counts["edges"], json!(doc.mindmap.edges.len()));
        let sections: usize = doc.mindmap.nodes.values().map(|n| n.sections.len()).sum();
        assert_eq!(counts["sections"], json!(sections));
        doc.file_path = None;
        assert_eq!(doc.ipc_field_json(DocumentField::FilePath), Value::Null);
    }

    #[test]
    fn test_document_field_words_round_trip_and_the_default_excludes_only_the_model() {
        for field in DocumentField::ALL {
            assert_eq!(DocumentField::parse(field.name()), Some(field));
        }
        assert_eq!(DocumentField::parse("Path"), None);
        assert_eq!(
            DocumentField::parse("path"),
            None,
            "`path` is the to_file sidecar's reply key, not a field"
        );
        let defaults: Vec<_> = DocumentField::ALL
            .into_iter()
            .filter(|f| f.is_default())
            .collect();
        assert_eq!(defaults.len(), DocumentField::ALL.len() - 1);
        assert!(!DocumentField::Mindmap.is_default());
    }

    /// **Zero mutation, asserted** (issue #64's acceptance
    /// criterion): every document-layer projection, run against a
    /// document carrying every kind of state a query reads, leaves
    /// `dirty`, the undo depth, the selection, the animations, the
    /// previews and the whole model exactly as they were. (`&self`
    /// makes this a type-level fact; the test keeps it from becoming
    /// a convention the day a projection grows an interior-mutable
    /// cache.)
    #[test]
    fn test_no_document_projection_mutates_the_document() {
        let mut doc = load_test_doc();
        doc.selection = SelectionState::Edge(edge());
        doc.undo_stack.push(UndoAction::CreateEdge { index: 0 });
        doc.dirty = true;
        let model_before = serde_json::to_value(&doc.mindmap).expect("serializes");
        let selection_before = doc.selection.clone();

        for field in DocumentField::ALL {
            let _ = doc.ipc_field_json(field);
        }
        let _ = doc.selection.to_ipc_json();
        let _ = doc.ipc_animations_json(123, None);
        let _ = doc.ipc_undo_json(10);
        let _ = MindMapDocument::ipc_previews_json(Some(&doc));

        assert!(doc.dirty, "dirty is untouched");
        assert_eq!(doc.undo_stack.len(), 1, "no undo entry is pushed or popped");
        assert_eq!(doc.selection, selection_before);
        assert_eq!(
            serde_json::to_value(&doc.mindmap).expect("serializes"),
            model_before
        );
        assert!(!doc.has_active_animations());
    }

    /// **The typed pointer path answers exactly what a pointer into the
    /// whole serialized model would** (review of #174), for every kind
    /// of first token, present, absent and skipped-when-empty.
    #[test]
    fn test_ipc_model_pointer_agrees_with_a_pointer_into_the_whole_value() {
        let doc = load_test_doc();
        let whole = serde_json::to_value(&doc.mindmap).expect("serializes");
        let node = doc
            .mindmap
            .nodes
            .keys()
            .next()
            .expect("fixture has nodes")
            .clone();
        let palette = doc
            .mindmap
            .palettes
            .keys()
            .next()
            .expect("fixture has palettes")
            .clone();
        let pointers = [
            String::new(),
            "/version".into(),
            "/name".into(),
            "/canvas".into(),
            "/canvas/background_color".into(),
            "/canvas/no_such_key".into(),
            "/palettes".into(),
            format!("/palettes/{palette}"),
            "/palettes/no_such_palette".into(),
            "/nodes".into(),
            format!("/nodes/{node}"),
            format!("/nodes/{node}/position"),
            format!("/nodes/{node}/position/x"),
            format!("/nodes/{node}/sections/0"),
            format!("/nodes/{node}/sections/0/text"),
            format!("/nodes/{node}/sections/99"),
            format!("/nodes/{node}/sections/00"),
            format!("/nodes/{node}/sections/-1"),
            "/nodes/no_such_node".into(),
            "/edges".into(),
            "/edges/0".into(),
            "/edges/0/from_id".into(),
            "/edges/00".into(),
            "/edges/+1".into(),
            "/edges/999999".into(),
            "/edges/x".into(),
            "/custom_mutations".into(),
            "/macros".into(),
            "/unknown_keys".into(),
            "/skipped_constructs".into(),
            "/nothing".into(),
            "nodes".into(),
            "/".into(),
        ];
        for pointer in &pointers {
            assert_eq!(
                doc.ipc_model_pointer(pointer).as_ref(),
                whole.pointer(pointer),
                "pointer `{pointer}`"
            );
        }
    }

    /// The same equivalence for fields the testament fixture leaves
    /// empty, and so absent from its serialized form: they come back
    /// when the model carries them, and are `None` when it does not.
    #[test]
    fn test_ipc_model_pointer_treats_empty_skipped_fields_as_absent() {
        let mut doc = load_test_doc();
        assert!(doc.mindmap.macros.is_empty() && doc.mindmap.custom_mutations.is_empty());
        assert_eq!(doc.ipc_model_pointer("/macros"), None);
        assert_eq!(doc.ipc_model_pointer("/custom_mutations"), None);
        doc.mindmap.macros.push(json!({ "id": "m" }));
        doc.mindmap.custom_mutations.push(
            TestNudgeMutation::new("cm", baumhard::mindmap::custom_mutation::TargetScope::SelfOnly).build(),
        );
        let whole = serde_json::to_value(&doc.mindmap).expect("serializes");
        for pointer in [
            "/macros",
            "/macros/0",
            "/macros/0/id",
            "/custom_mutations",
            "/custom_mutations/0/id",
        ] {
            assert_eq!(
                doc.ipc_model_pointer(pointer).as_ref(),
                whole.pointer(pointer),
                "pointer `{pointer}`"
            );
            assert!(
                whole.pointer(pointer).is_some(),
                "`{pointer}` exists once populated"
            );
        }
    }

    #[test]
    fn test_pointer_tokens_unescape_in_the_rfc_order() {
        assert_eq!(pointer_tokens(""), Some(vec![]));
        assert_eq!(pointer_tokens("/"), Some(vec![String::new()]));
        assert_eq!(pointer_tokens("/a/b"), Some(vec!["a".into(), "b".into()]));
        assert_eq!(
            pointer_tokens("/a~1b/m~0n/~01"),
            Some(vec!["a/b".into(), "m~n".into(), "~1".into()]),
            "`~01` is a literal `~1`, not a slash"
        );
        assert_eq!(pointer_tokens("a"), None);
    }

    #[test]
    fn test_descend_moves_the_answer_out_without_a_clone() {
        let value = json!({ "a": [10, { "b": "x" }], "c": null });
        let tokens = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            descend(value.clone(), &tokens(&["a", "1", "b"])),
            Some(json!("x"))
        );
        assert_eq!(descend(value.clone(), &tokens(&["c"])), Some(Value::Null));
        assert_eq!(descend(value.clone(), &tokens(&["a", "2"])), None);
        assert_eq!(descend(value.clone(), &tokens(&["a", "01"])), None);
        assert_eq!(descend(value.clone(), &tokens(&["c", "x"])), None);
        assert_eq!(
            descend(value, &[]),
            Some(json!({ "a": [10, { "b": "x" }], "c": null }))
        );
    }

    /// The streamed reply is the inline reply: serializing a
    /// `DocumentReply` to bytes and to a value agree, with and
    /// without a pointer, and a miss is known before anything is
    /// serialized.
    #[test]
    fn test_document_reply_streams_what_it_builds_inline() {
        let doc = load_test_doc();
        let fields = DocumentField::ALL;
        let reply = doc.ipc_document_reply(&fields, None).expect("no pointer");
        let value = serde_json::to_value(&reply).expect("value");
        let streamed: Value =
            serde_json::from_slice(&serde_json::to_vec(&reply).expect("bytes")).expect("JSON");
        assert_eq!(streamed, value);
        for field in DocumentField::ALL {
            assert_eq!(value[field.name()], doc.ipc_field_json(field), "{}", field.name());
        }

        let pointed = doc
            .ipc_document_reply(&fields, Some("/nodes"))
            .expect("pointer resolves");
        let value = serde_json::to_value(&pointed).expect("value");
        assert_eq!(value["mindmap"], doc.ipc_model_pointer("/nodes").expect("nodes"));
        assert_eq!(value["dirty"], json!(false), "the other fields are unaffected");

        assert_eq!(
            doc.ipc_document_reply(&fields, Some("/nodes/no_such_node")).err(),
            Some(PointerMiss)
        );
        assert!(
            doc.ipc_document_reply(&[DocumentField::Dirty], Some("/nodes/no_such_node"))
                .is_ok(),
            "a pointer narrows the model, so without the model it is not resolved"
        );
    }

    /// **One list of preview keys** (review of #174): with no
    /// document, and with a document that has no preview live, the
    /// reply is the same all-`null` object; a live preview fills in
    /// exactly its own key and the key set never changes.
    #[test]
    fn test_previews_share_one_key_list_with_and_without_a_document() {
        let mut doc = load_test_doc();
        let none = MindMapDocument::ipc_previews_json(None);
        assert_eq!(none, MindMapDocument::ipc_previews_json(Some(&doc)));
        let keys: Vec<&String> = none.as_object().expect("object").keys().collect();
        for key in &keys {
            assert_eq!(none[key.as_str()], Value::Null, "{key}");
        }

        doc.label_edit_preview = Some((EdgeKey::new("a", "b", "cross_link"), "typing".into()));
        let live = MindMapDocument::ipc_previews_json(Some(&doc));
        assert_eq!(live.as_object().expect("object").keys().collect::<Vec<_>>(), keys);
        assert_eq!(live["label_edit"]["buffer"], json!("typing"));
        for key in keys.iter().filter(|k| k.as_str() != "label_edit") {
            assert_eq!(live[key.as_str()], Value::Null, "{key}");
        }
    }

    /// **`query.animations` reports a drag pause** (review of #174):
    /// while a tree-mutating drag has suspended the tick, the
    /// instances read at the pause's start (the value the resume
    /// shift restores), never advance with the wall clock, never
    /// reach `due`, and say `paused`.
    #[test]
    fn test_animations_report_the_pause_a_drag_puts_on_the_tick() {
        use baumhard::mindmap::animation::{AnimationTiming, Easing};
        let mut doc = load_test_doc();
        let node_id = doc
            .mindmap
            .nodes
            .keys()
            .next()
            .expect("fixture has nodes")
            .clone();
        let cm = TestNudgeMutation::new("anim", baumhard::mindmap::custom_mutation::TargetScope::SelfOnly)
            .magnitude(100.0)
            .timing(AnimationTiming {
                duration_ms: 1000,
                delay_ms: 0,
                easing: Easing::Linear,
                then: None,
            })
            .build();
        doc.start_animation(&cm, &node_id, 10_000);

        let running = doc.ipc_animations_json(10_250, None);
        assert_eq!(running["active"], json!(true));
        assert_eq!(running["paused"], json!(false));
        assert_eq!(running["animations"][0]["phase"], json!("running"));
        assert_eq!(running["animations"][0]["elapsed_ms"], json!(250));
        assert_eq!(running["animations"][0]["progress"], json!(0.25));

        // Paused at 10_250; a poll at 99_999 still reads 250 ms in, not due.
        let paused = doc.ipc_animations_json(99_999, Some(10_250));
        assert_eq!(paused["active"], json!(true));
        assert_eq!(paused["paused"], json!(true));
        assert_eq!(paused["animations"][0]["phase"], json!("paused"));
        assert_eq!(paused["animations"][0]["elapsed_ms"], json!(250));
        assert_eq!(paused["animations"][0]["progress"], json!(0.25));

        // Unpaused far later it would be due: the pause is what differs.
        let due = doc.ipc_animations_json(99_999, None);
        assert_eq!(due["animations"][0]["phase"], json!("due"));
    }

    #[test]
    fn test_f32_json_names_the_shortest_decimal() {
        assert_eq!(f32_json(0.1), json!(0.1));
        assert_eq!(f32_json(0.75), json!(0.75));
        assert_eq!(f32_json(-3.0), json!(-3.0));
        assert_eq!(f32_json(f32::NAN), Value::Null);
        assert_eq!(f32_json(f32::INFINITY), Value::Null);
    }
}
