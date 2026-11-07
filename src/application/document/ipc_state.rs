// SPDX-License-Identifier: MPL-2.0

//! The document's state as the IPC surface reads it (`format/ipc.md`
//! §`query`): the selection, the document's summary and its model, and
//! the undo history. Each projection lives here, beside the state it
//! reads, so a change to that state and to its wire form are one edit.
//!
//! Elements are named the way `view::inspect::element_ref` names them —
//! `{"node": <ident>}` and the kinds below a node — so a name read from
//! `query.selection` is the name `scene.*` and the events use.

use serde::ser::{Serialize, SerializeMap, Serializer};
use serde_json::{json, Value};

use baumhard::document::format;

use crate::application::view::inspect::element_ref;

use super::selection::every_node;
use super::MindMapDocument;

/// One selectable part of `query.document`'s reply. The wire name of
/// each is [`Self::name`], and [`Self::ALL`] is the order they appear
/// in the reply.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DocumentField {
    /// The bound file path, or `null` for a document never saved.
    FilePath,
    /// Whether the document has changes since it was loaded or saved.
    Dirty,
    /// The map's name.
    Name,
    /// How many nodes the map holds.
    Nodes,
    /// The document's revision: how many changes it has taken.
    Revision,
    /// `{"undo", "redo"}`: how many entries the history holds each way.
    History,
    /// The whole map, in the `.mandala.json` shape a save writes.
    Document,
}

impl DocumentField {
    /// Every field, in reply order.
    pub(crate) const ALL: [DocumentField; 7] = [
        DocumentField::FilePath,
        DocumentField::Dirty,
        DocumentField::Name,
        DocumentField::Nodes,
        DocumentField::Revision,
        DocumentField::History,
        DocumentField::Document,
    ];

    /// What a request that names no `fields` gets: everything but the
    /// model, which is the one part whose size is unbounded.
    pub(crate) fn is_default(self) -> bool {
        !matches!(self, DocumentField::Document)
    }

    /// The wire word.
    pub(crate) fn name(self) -> &'static str {
        match self {
            DocumentField::FilePath => "file_path",
            DocumentField::Dirty => "dirty",
            DocumentField::Name => "name",
            DocumentField::Nodes => "nodes",
            DocumentField::Revision => "revision",
            DocumentField::History => "history",
            DocumentField::Document => "document",
        }
    }

    /// The field a wire word names.
    pub(crate) fn parse(word: &str) -> Option<DocumentField> {
        DocumentField::ALL.into_iter().find(|f| f.name() == word)
    }
}

/// A `pointer` that names nothing in the model.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct PointerMiss;

/// `query.document`'s reply: the chosen [`DocumentField`]s of one
/// document, as a serializable object. Serialize it to a `Value` for an
/// inline reply, or stream it into a writer for `to_file`.
pub(crate) struct DocumentReply<'a> {
    doc: &'a MindMapDocument,
    fields: &'a [DocumentField],
    /// The model, when [`DocumentField::Document`] is asked for: the
    /// whole of it, or the value a `pointer` selected.
    model: Option<Value>,
}

impl MindMapDocument {
    /// The selection as `query.selection` and `selection_changed` carry
    /// it: `{"count", "elements"}`, each element by its wire name, in the
    /// order they were selected. O(the selection, times each element's
    /// depth within its node).
    pub(crate) fn ipc_selection_json(&self) -> Value {
        let elements: Vec<Value> = self
            .selection
            .iter()
            .map(|id| element_ref(&self.doc, id))
            .collect();
        json!({ "count": elements.len(), "elements": elements })
    }

    /// The undo history as `query.undo` reports it: how many entries it
    /// holds each way, and the newest `limit` applied ones, newest first
    /// — the order Undo takes them — each with its label, its source and
    /// the revision it made.
    pub(crate) fn ipc_undo_json(&self, limit: usize) -> Value {
        let applied: Vec<_> = self.doc.history().collect();
        let entries: Vec<Value> = applied
            .iter()
            .rev()
            .take(limit)
            .map(|entry| {
                json!({
                    "label": entry.label,
                    "source": entry.source.name(),
                    "revision": entry.revision.0,
                })
            })
            .collect();
        json!({
            "depth": applied.len(),
            "redo_depth": self.doc.redo_len(),
            "entries": entries,
        })
    }

    /// The model as a value: the map as a save would write it, a change
    /// a transaction holds open included. `None` when the document cannot
    /// be written, which the writer prevents.
    fn ipc_model(&self) -> Option<Value> {
        let text = format::save(&self.doc).ok()?;
        serde_json::from_str(&text).ok()
    }

    /// The reply for `fields`, with `pointer` (when the model is among
    /// them) resolved **up front**, so a miss is known before anything
    /// is written anywhere and serializing the reply cannot fail on it.
    ///
    /// Cost: every field but [`DocumentField::Document`] is O(1) apart
    /// from [`DocumentField::Nodes`], one walk of the nodes; the model
    /// is one save of the map, and a pointer into it a walk of the
    /// tokens.
    pub(crate) fn ipc_document_reply<'a>(
        &'a self,
        fields: &'a [DocumentField],
        pointer: Option<&str>,
    ) -> Result<DocumentReply<'a>, PointerMiss> {
        let model = if fields.contains(&DocumentField::Document) {
            let whole = self.ipc_model().ok_or(PointerMiss)?;
            Some(match pointer {
                Some(pointer) => whole.pointer(pointer).cloned().ok_or(PointerMiss)?,
                None => whole,
            })
        } else {
            None
        };
        Ok(DocumentReply {
            doc: self,
            fields,
            model,
        })
    }

    /// One part of `query.document`'s reply as a value.
    #[cfg(test)]
    pub(crate) fn ipc_field_json(&self, field: DocumentField) -> Value {
        let fields = [field];
        let reply = self
            .ipc_document_reply(&fields, None)
            .expect("no pointer to miss");
        serde_json::to_value(&reply).expect("serializes")[field.name()].clone()
    }
}

impl Serialize for DocumentReply<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let doc = self.doc;
        let mut map = serializer.serialize_map(Some(self.fields.len()))?;
        for &field in self.fields {
            let value = match field {
                DocumentField::FilePath => json!(doc.file_path),
                DocumentField::Dirty => json!(doc.is_dirty()),
                DocumentField::Name => json!(doc.name()),
                DocumentField::Nodes => json!(every_node(doc.doc()).count()),
                DocumentField::Revision => json!(doc.revision().0),
                DocumentField::History => json!({
                    "undo": doc.doc().history().count(),
                    "redo": doc.doc().redo_len(),
                }),
                DocumentField::Document => self.model.clone().unwrap_or(Value::Null),
            };
            map.serialize_entry(field.name(), &value)?;
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `query.document`'s summary reads the document as it stands: a
    /// change moves its revision, its node count, its history and its
    /// dirty flag, and a save clears the flag and keeps the rest.
    #[test]
    fn test_the_document_summary_follows_its_changes() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        let before =
            serde_json::to_value(doc.ipc_document_reply(&DocumentField::ALL[..6], None).unwrap()).unwrap();
        assert_eq!(before["nodes"], json!(0));
        assert_eq!(before["dirty"], json!(false));
        assert_eq!(before["history"], json!({"undo": 0, "redo": 0}));
        assert_eq!(before["name"], json!("untitled"));

        doc.create_root_for_test();
        let after =
            serde_json::to_value(doc.ipc_document_reply(&DocumentField::ALL[..6], None).unwrap()).unwrap();
        assert_eq!(after["nodes"], json!(1));
        assert_eq!(after["dirty"], json!(true));
        assert_eq!(after["history"], json!({"undo": 1, "redo": 0}));
        assert!(after["revision"].as_u64() > before["revision"].as_u64());

        doc.undo();
        assert_eq!(
            doc.ipc_field_json(DocumentField::History),
            json!({"undo": 0, "redo": 1})
        );
    }

    /// The model is the saved map, and a pointer selects a part of it; a
    /// pointer that names nothing is a miss, known before any reply.
    #[test]
    fn test_the_model_is_the_saved_map_and_a_pointer_selects_into_it() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        doc.create_root_for_test();
        let saved: Value = serde_json::from_str(&doc.save_json().unwrap()).unwrap();
        assert_eq!(doc.ipc_field_json(DocumentField::Document), saved);
        let fields = [DocumentField::Document];
        let name = serde_json::to_value(doc.ipc_document_reply(&fields, Some("/name")).unwrap()).unwrap();
        assert_eq!(name["document"], saved["name"]);
        assert!(doc.ipc_document_reply(&fields, Some("/no/such")).is_err());
        // Control: a pointer is only read with the model asked for.
        assert!(doc
            .ipc_document_reply(&[DocumentField::Dirty], Some("/no/such"))
            .is_ok());
    }

    /// Every field's wire word parses back to it, and only the model is
    /// left out by default.
    #[test]
    fn test_document_fields_round_trip_and_the_model_is_opt_in() {
        for field in DocumentField::ALL {
            assert_eq!(DocumentField::parse(field.name()), Some(field));
            assert_eq!(field.is_default(), field != DocumentField::Document);
        }
        assert_eq!(DocumentField::parse("mindmap"), None);
    }

    /// The undo history is newest first, labeled, and bounded by `limit`.
    #[test]
    fn test_the_undo_history_is_newest_first_and_bounded() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        doc.create_root_for_test();
        doc.create_root_for_test();
        let all = doc.ipc_undo_json(10);
        assert_eq!(all["depth"], json!(2));
        assert_eq!(all["entries"].as_array().unwrap().len(), 2);
        let newest = &all["entries"][0];
        assert_eq!(newest["label"], json!("test: create root"));
        assert_eq!(newest["source"], json!("action"));
        assert!(newest["revision"].as_u64() > all["entries"][1]["revision"].as_u64());
        assert_eq!(doc.ipc_undo_json(1)["entries"].as_array().unwrap().len(), 1);
    }

    /// The selection names each element by its integer node id, the
    /// order it was selected in.
    #[test]
    fn test_the_selection_names_nodes_by_integer_id() {
        let mut doc = MindMapDocument::new_blank(None, 50);
        doc.create_root_for_test();
        doc.create_root_for_test();
        let nodes: Vec<_> = every_node(doc.doc()).collect();
        doc.selection.set([nodes[1], nodes[0]]);
        let selection = doc.ipc_selection_json();
        assert_eq!(selection["count"], json!(2));
        let ids: Vec<u64> = selection["elements"]
            .as_array()
            .unwrap()
            .iter()
            .map(|e| e["node"].as_u64().expect("an integer id"))
            .collect();
        assert_ne!(ids[0], ids[1]);
        assert_eq!(
            json!({"count": 0, "elements": []}),
            MindMapDocument::new_blank(None, 50).ipc_selection_json()
        );
    }
}
