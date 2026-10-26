// SPDX-License-Identifier: MPL-2.0

//! IPC projections of the app shell's own state (`format/ipc.md`
//! §`query`): the interaction mode, the drag state machine, the four
//! modal editors. The camera's projection is the protocol's own
//! coordinate-space transform and lives with it (`ipc/space.rs`); the
//! document-layer half lives beside
//! the document (`document/ipc_state.rs`); this half lives in `app`
//! because `DragState`, `TextEditState`, `SingleLineEditor` and the
//! throttled gestures are private to it.
//!
//! The rules are the document half's. Every projection is an
//! inherent method or function over `&` — a read cannot mutate — and
//! every `match` over an app-state enum is **exhaustive with no
//! wildcard arm**, so a new `InteractionMode`, `DragState` or
//! `ThrottledDrag` variant breaks the build here, on purpose, until
//! its wire form is decided and written into `format/ipc.md`. Each
//! projection's `kind` word is the variant's own name. Where a type
//! owns internals with no wire meaning (cached layout, scratch
//! buffers, pre-edit snapshots) the projection is a stated subset and
//! the omission is named at its method.
//!
//! Native-only, with `ipc_target.rs` that consumes it.

#![cfg(not(target_arch = "wasm32"))]

use serde_json::{json, Value};

use baumhard::mindmap::tree_builder::{EdgeHandleKind, ResizeHandleSide};

use crate::application::color_picker::{
    ColorPickerState, OpenPicker, PickerGesture, PickerHandle, PickerMode,
};
use crate::application::console::{ConsoleLine, ConsoleState};
use crate::application::document::{f32_json, MindMapDocument};

use super::interaction_mode::ResizeTarget;
use super::modal_editor::ModalEditor;
use super::single_line_edit::{SingleLineEditTarget, SingleLineEditor};
use super::text_edit::TextEditState;
use super::throttled_interaction::ThrottledDrag;
use super::{DragState, Grab, HeldInput, InteractionMode, PanOwner};

/// How many scrollback lines `query.editors` reports for an open
/// console: the tail, which is what is on screen. The total is
/// `scrollback_len`.
pub(super) const CONSOLE_SCROLLBACK_TAIL: usize = 50;

/// `{"x", "y"}`.
fn xy(x: Value, y: Value) -> Value {
    json!({ "x": x, "y": y })
}

fn xy_f64(point: (f64, f64)) -> Value {
    xy(json!(point.0), json!(point.1))
}

fn xy_vec2(point: glam::Vec2) -> Value {
    xy(f32_json(point.x), f32_json(point.y))
}

impl ResizeTarget {
    fn to_ipc_json(&self) -> Value {
        match self {
            ResizeTarget::Node(node_id) => json!({ "kind": "Node", "node_id": node_id }),
            ResizeTarget::Section { node_id, section_idx } => json!({
                "kind": "Section",
                "node_id": node_id,
                "section_idx": section_idx,
            }),
        }
    }
}

impl InteractionMode {
    /// The mode as `query.mode` reports it: `kind` is the variant's
    /// name, with the variant's ids beside it. Lossless.
    pub(in crate::application) fn to_ipc_json(&self) -> Value {
        match self {
            InteractionMode::Default => json!({ "kind": "Default" }),
            InteractionMode::Reparent { sources } => json!({ "kind": "Reparent", "sources": sources }),
            InteractionMode::Connect { source } => json!({ "kind": "Connect", "source": source }),
            InteractionMode::NodeEdit { node_id } => json!({ "kind": "NodeEdit", "node_id": node_id }),
            InteractionMode::Resize { target } => json!({ "kind": "Resize", "target": target.to_ipc_json() }),
        }
    }
}

fn side_word(side: ResizeHandleSide) -> &'static str {
    match side {
        ResizeHandleSide::NW => "NW",
        ResizeHandleSide::N => "N",
        ResizeHandleSide::NE => "NE",
        ResizeHandleSide::E => "E",
        ResizeHandleSide::SE => "SE",
        ResizeHandleSide::S => "S",
        ResizeHandleSide::SW => "SW",
        ResizeHandleSide::W => "W",
    }
}

fn edge_handle_json(handle: EdgeHandleKind) -> Value {
    match handle {
        EdgeHandleKind::AnchorFrom => json!({ "kind": "AnchorFrom" }),
        EdgeHandleKind::AnchorTo => json!({ "kind": "AnchorTo" }),
        EdgeHandleKind::ControlPoint(index) => json!({ "kind": "ControlPoint", "index": index }),
        EdgeHandleKind::Midpoint => json!({ "kind": "Midpoint" }),
    }
}

impl ThrottledDrag {
    /// The gesture as `query.mode` reports it, under `gesture` (the
    /// variant's name) with what it is acting on. **Omitted by name:**
    /// the pre-drag snapshots (`original`, `start_*`), the pending
    /// delta and the throttle, which are the gesture's bookkeeping,
    /// not its identity.
    fn to_ipc_json(&self) -> Value {
        match self {
            ThrottledDrag::MovingNode(i) => json!({
                "gesture": "MovingNode",
                "node_ids": i.node_ids,
                "individual": i.individual,
            }),
            ThrottledDrag::MovingSection(i) => json!({
                "gesture": "MovingSection",
                "node_id": i.node_id,
                "section_idx": i.section_idx,
            }),
            ThrottledDrag::SectionResize(i) => json!({
                "gesture": "SectionResize",
                "node_id": i.node_id,
                "section_idx": i.section_idx,
                "side": side_word(i.side),
            }),
            ThrottledDrag::NodeResize(i) => json!({
                "gesture": "NodeResize",
                "node_id": i.node_id,
                "side": side_word(i.side),
            }),
            ThrottledDrag::EdgeHandle(i) => json!({
                "gesture": "EdgeHandle",
                "edge": i.edge_ref.to_ipc_json(),
                "handle": edge_handle_json(i.handle),
            }),
            ThrottledDrag::PortalLabel(i) => json!({
                "gesture": "PortalLabel",
                "edge": i.edge_ref.to_ipc_json(),
                "endpoint_node_id": i.endpoint_node_id,
            }),
            ThrottledDrag::EdgeLabel(i) => json!({
                "gesture": "EdgeLabel",
                "edge": i.edge_ref.to_ipc_json(),
            }),
        }
    }
}

impl DragState {
    /// The drag state machine as `query.mode` reports it. `kind` is
    /// the variant's name. Positions are what each variant carries:
    /// `start_pos` in surface pixels, `*_canvas` in canvas space.
    pub(super) fn to_ipc_json(&self) -> Value {
        match self {
            DragState::None => json!({ "kind": "None" }),
            DragState::Pending(press) => json!({
                "kind": "Pending",
                "start_pos": xy_f64(press.start_pos),
                "hit_node": press.hit_node,
                "hit_section_idx": press.hit_section_idx,
                "grabs": press.grabs().map(Grab::word).collect::<Vec<_>>(),
            }),
            DragState::PendingRight {
                start_pos,
                start_canvas,
                hit_node,
                hit_section_idx,
            } => json!({
                "kind": "PendingRight",
                "start_pos": xy_f64(*start_pos),
                "start_canvas": xy_vec2(*start_canvas),
                "hit_node": hit_node,
                "hit_section_idx": hit_section_idx,
            }),
            DragState::Panning { owner } => match owner {
                PanOwner::Toggled => json!({ "kind": "Panning", "owner": "toggled" }),
                PanOwner::Held(HeldInput::Button(button)) => json!({
                    "kind": "Panning",
                    "owner": "button",
                    "button": crate::application::ipc::injected::button_word(*button),
                }),
                PanOwner::Held(HeldInput::Key(key)) => json!({
                    "kind": "Panning",
                    "owner": "key",
                    "key": key,
                }),
            },
            DragState::SelectingRect {
                start_canvas,
                current_canvas,
            } => json!({
                "kind": "SelectingRect",
                "start_canvas": xy_vec2(*start_canvas),
                "current_canvas": xy_vec2(*current_canvas),
            }),
            DragState::Throttled(drag) => {
                let mut json = drag.to_ipc_json();
                json["kind"] = json!("Throttled");
                json
            }
        }
    }
}

impl TextEditState {
    /// The node text editor as `query.editors` reports it: `null`
    /// when closed. `cursor_grapheme_pos` and `selection_anchor` are
    /// grapheme-cluster indices into `buffer` (what the user sees as
    /// characters), never bytes.
    ///
    /// **Omitted by name:** `buffer_regions`, `original_text` and
    /// `original_regions`: the styling the buffer carries and the
    /// pre-edit snapshot cancel restores from.
    pub(super) fn to_ipc_json(&self) -> Value {
        match self {
            TextEditState::Closed => Value::Null,
            TextEditState::Open {
                node_id,
                section_idx,
                buffer,
                cursor_grapheme_pos,
                selection_anchor,
                exit_to_default_on_close,
                buffer_regions: _,
                original_text: _,
                original_regions: _,
            } => json!({
                "node_id": node_id,
                "section_idx": section_idx,
                "buffer": buffer,
                "cursor_grapheme_pos": cursor_grapheme_pos,
                "selection_anchor": selection_anchor,
                "exit_to_default_on_close": exit_to_default_on_close,
            }),
        }
    }
}

impl SingleLineEditTarget {
    fn to_ipc_json(&self) -> Value {
        match self {
            SingleLineEditTarget::EdgeLabel { edge_ref } => {
                json!({ "kind": "EdgeLabel", "edge": edge_ref.to_ipc_json() })
            }
            SingleLineEditTarget::PortalText {
                edge_ref,
                endpoint_node_id,
            } => json!({
                "kind": "PortalText",
                "edge": edge_ref.to_ipc_json(),
                "endpoint_node_id": endpoint_node_id,
            }),
        }
    }
}

impl SingleLineEditor {
    /// The edge-label / portal-caption editor as `query.editors`
    /// reports it: `null` when closed. `original` is the string the
    /// target held when the editor opened (`null` if it had none).
    pub(super) fn to_ipc_json(&self) -> Value {
        match self {
            SingleLineEditor::Closed => Value::Null,
            SingleLineEditor::Open {
                target,
                buffer,
                cursor_grapheme_pos,
                original,
            } => json!({
                "target": target.to_ipc_json(),
                "buffer": buffer,
                "cursor_grapheme_pos": cursor_grapheme_pos,
                "original": original,
            }),
        }
    }
}

fn console_line_json(line: &ConsoleLine) -> Value {
    match line {
        ConsoleLine::Input(text) => json!({ "kind": "Input", "text": text }),
        ConsoleLine::Output { text, .. } => json!({ "kind": "Output", "text": text }),
        ConsoleLine::Error(text) => json!({ "kind": "Error", "text": text }),
    }
}

/// The console as `query.editors` reports it: `null` when closed.
/// `cursor` is a grapheme index into `input`. The scrollback is its
/// last [`CONSOLE_SCROLLBACK_TAIL`] lines, the rest counted in
/// `scrollback_len`.
///
/// **Omitted by name:** each output line's pinned `font_family`, the
/// completion rows (only their count and the highlighted index are
/// reported) and the wheel accumulator.
pub(super) fn console_to_ipc_json(state: &ConsoleState) -> Value {
    match state {
        ConsoleState::Closed => Value::Null,
        ConsoleState::Open {
            input,
            cursor,
            history,
            history_idx,
            scrollback,
            completions,
            completion_idx,
            scroll_offset,
            wheel_accum: _,
        } => json!({
            "input": input,
            "cursor": cursor,
            "history_len": history.len(),
            "history_idx": history_idx,
            "scrollback_len": scrollback.len(),
            "scrollback": scrollback
                .iter()
                .skip(scrollback.len().saturating_sub(CONSOLE_SCROLLBACK_TAIL))
                .map(console_line_json)
                .collect::<Vec<_>>(),
            "completion_count": completions.len(),
            "completion_idx": completion_idx,
            "scroll_offset": scroll_offset,
        }),
    }
}

fn picker_handle_json(handle: &PickerHandle) -> Value {
    use crate::application::color_picker::{NodeColorAxis, SectionColorAxis};
    match handle {
        PickerHandle::Edge(edge_index) => json!({ "kind": "Edge", "edge_index": edge_index }),
        PickerHandle::Node { id, axis } => json!({
            "kind": "Node",
            "node_id": id,
            "axis": match axis {
                NodeColorAxis::Bg => "Bg",
                NodeColorAxis::Text => "Text",
                NodeColorAxis::Border => "Border",
            },
        }),
        PickerHandle::Section {
            node_id,
            section_idx,
            axis,
            range,
        } => json!({
            "kind": "Section",
            "node_id": node_id,
            "section_idx": section_idx,
            "axis": match axis {
                SectionColorAxis::Text => "Text",
            },
            "grapheme_range": range.map(|r| json!({ "start": r.start(), "end": r.end() })),
        }),
    }
}

impl OpenPicker {
    /// An open picker as `query.editors` reports it. `hue_deg` is in
    /// `[0, 360)`, `sat` and `val` in `[0, 1]`: the HSV the wheel
    /// would commit. `mode` is `Standalone` (commit writes to the
    /// document's selection) or `Contextual` (commit writes to its
    /// `handle`).
    ///
    /// **Omitted by name:** the cached layout, the glyph-measurement
    /// caches, the wheel's screen placement and the dynamic-apply
    /// key. That is all geometry, and `scene.dump` (IPC-05) owns what
    /// the wheel looks like.
    fn to_ipc_json(&self) -> Value {
        let mode = match &self.mode {
            PickerMode::Standalone => json!({ "kind": "Standalone" }),
            PickerMode::Contextual {
                handle,
                seed_var_ref,
                seed_hsv,
            } => json!({
                "kind": "Contextual",
                "handle": picker_handle_json(handle),
                "seed_var_ref": seed_var_ref,
                "seed_hsv": { "h": f32_json(seed_hsv.0), "s": f32_json(seed_hsv.1), "v": f32_json(seed_hsv.2) },
            }),
        };
        let gesture = self.gesture.as_ref().map(|g| match g {
            PickerGesture::Move { .. } => "Move",
            PickerGesture::Resize { .. } => "Resize",
        });
        json!({
            "mode": mode,
            "hue_deg": f32_json(self.hue_deg),
            "sat": f32_json(self.sat),
            "val": f32_json(self.val),
            "size_scale": f32_json(self.size_scale),
            "gesture": gesture,
            "hover_preview": self.hover_preview.map(|(h, s, v)| json!({
                "h": f32_json(h), "s": f32_json(s), "v": f32_json(v),
            })),
        })
    }
}

/// The color picker as `query.editors` reports it: `null` when closed.
pub(super) fn picker_to_ipc_json(state: &ColorPickerState) -> Value {
    match state {
        ColorPickerState::Closed => Value::Null,
        ColorPickerState::Open(open) => open.to_ipc_json(),
    }
}

/// Where keystrokes go, as the keyboard handler routes them
/// (`event_keyboard.rs`), which is **not** one owner:
///
/// 1. The **console**, when open, takes every key.
/// 2. The **color picker**, when open, claims only its own keys (its
///    commit, cancel and nudge verbs and its clipboard keys). Every
///    other key falls through to the steps below, so an open picker
///    never answers "who gets the next ordinary key".
/// 3. The **single-line editor**, then the **node text editor**
///    ([`ModalEditor::STEAL_ORDER`]), take every key that reaches them.
/// 4. With none of those open, a key reaches normal dispatch.
///
/// Returns `(modals, key_owner)`: every open modal in that order, and
/// the one that receives an *ordinary* key (`"console"`,
/// `"single_line_edit"`, `"text_edit"`, or `"none"` for normal
/// dispatch). The picker appears in `modals` and is never the
/// `key_owner`.
fn key_routing(
    console: &ConsoleState,
    picker: &ColorPickerState,
    single_line: &SingleLineEditor,
    text: &TextEditState,
) -> (Vec<&'static str>, &'static str) {
    let editor_word = |editor: ModalEditor| match editor {
        ModalEditor::SingleLine => "single_line_edit",
        ModalEditor::Text => "text_edit",
    };
    let mut modals = Vec::new();
    if console.is_open() {
        modals.push("console");
    }
    if picker.as_open().is_some() {
        modals.push("color_picker");
    }
    modals.extend(ModalEditor::open_in_steal_order(single_line.is_open(), text.is_open()).map(editor_word));
    let key_owner = if console.is_open() {
        "console"
    } else {
        ModalEditor::stealing_from(single_line.is_open(), text.is_open()).map_or("none", editor_word)
    };
    (modals, key_owner)
}

/// `query.mode`'s reply: the interaction mode and the drag state
/// machine, which modals are open and who receives an ordinary key
/// ([`key_routing`]), and where the pointer is.
/// `hovered_node` is the node under the cursor (`null` over none) and
/// `cursor_pos` the last known pointer position in **surface** pixels.
#[allow(clippy::too_many_arguments)]
pub(super) fn mode_to_ipc_json(
    mode: &InteractionMode,
    drag: &DragState,
    console: &ConsoleState,
    picker: &ColorPickerState,
    single_line: &SingleLineEditor,
    text: &TextEditState,
    hovered_node: Option<&str>,
    cursor_pos: (f64, f64),
) -> Value {
    let (modals, key_owner) = key_routing(console, picker, single_line, text);
    json!({
        "interaction_mode": mode.to_ipc_json(),
        "drag_state": drag.to_ipc_json(),
        "modals": modals,
        "key_owner": key_owner,
        "hovered_node": hovered_node,
        "cursor_pos": xy_f64(cursor_pos),
    })
}

/// `query.editors`' reply: each modal editor's live state (`null`
/// when closed) and the document's `previews`.
pub(super) fn editors_to_ipc_json(
    console: &ConsoleState,
    picker: &ColorPickerState,
    single_line: &SingleLineEditor,
    text: &TextEditState,
    document: Option<&MindMapDocument>,
) -> Value {
    json!({
        "console": console_to_ipc_json(console),
        "color_picker": picker_to_ipc_json(picker),
        "single_line_edit": single_line.to_ipc_json(),
        "text_edit": text.to_ipc_json(),
        "previews": MindMapDocument::ipc_previews_json(document),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::{
        EdgeHandleInteraction, EdgeLabelInteraction, MovingNodeInteraction, MovingSectionInteraction,
        NodeResizeInteraction, PortalLabelInteraction, SectionResizeInteraction,
    };
    use crate::application::app::PendingPress;
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::EdgeRef;
    use crate::application::platform::input::MouseButton;
    use baumhard::mindmap::model::{Position, Size};
    use std::collections::HashSet;

    /// One `InteractionMode` of every variant. The match below is the
    /// completeness guard: no wildcard, so a new variant fails to
    /// compile here until it has a value in the list and a wire form
    /// in `to_ipc_json`.
    fn every_mode() -> Vec<InteractionMode> {
        let all = vec![
            InteractionMode::Default,
            InteractionMode::Reparent {
                sources: vec!["1".into(), "2".into()],
            },
            InteractionMode::Connect { source: "1".into() },
            InteractionMode::NodeEdit { node_id: "1".into() },
            InteractionMode::Resize {
                target: ResizeTarget::Node("1".into()),
            },
            InteractionMode::Resize {
                target: ResizeTarget::Section {
                    node_id: "1".into(),
                    section_idx: 2,
                },
            },
        ];
        for mode in &all {
            match mode {
                InteractionMode::Default
                | InteractionMode::Reparent { .. }
                | InteractionMode::Connect { .. }
                | InteractionMode::NodeEdit { .. }
                | InteractionMode::Resize { .. } => {}
            }
        }
        all
    }

    #[test]
    fn test_every_interaction_mode_serializes_under_its_own_name() {
        let kinds: Vec<String> = every_mode()
            .iter()
            .map(|mode| {
                let json = mode.to_ipc_json();
                let kind = json["kind"].as_str().expect("kind").to_string();
                assert!(format!("{mode:?}").starts_with(&kind), "{kind} vs {mode:?}");
                kind
            })
            .collect();
        let mut distinct = kinds.clone();
        distinct.sort();
        distinct.dedup();
        assert_eq!(distinct.len(), 5, "InteractionMode has five variants: {kinds:?}");
        assert_eq!(
            InteractionMode::Resize {
                target: ResizeTarget::Section {
                    node_id: "1".into(),
                    section_idx: 2
                }
            }
            .to_ipc_json(),
            json!({ "kind": "Resize", "target": { "kind": "Section", "node_id": "1", "section_idx": 2 } })
        );
    }

    /// One `DragState` of every variant, and under `Throttled` one of
    /// every `ThrottledDrag`. Both matches are wildcard-free.
    fn every_drag_state() -> Vec<DragState> {
        let doc = load_test_doc();
        let edge = doc.mindmap.edges.first().expect("fixture has edges").clone();
        let edge_ref = EdgeRef::new(
            edge.from_id.as_str(),
            edge.to_id.as_str(),
            edge.edge_type.as_str(),
        );
        let origin = Position { x: 0.0, y: 0.0 };
        let unit = Size {
            width: 1.0,
            height: 1.0,
        };
        let throttled = |drag| DragState::Throttled(Box::new(drag));
        let all = vec![
            DragState::None,
            DragState::Pending(Box::new(PendingPress {
                start_pos: (4.0, 5.0),
                hit_node: Some("1".into()),
                hit_section_idx: Some(2),
                hit_edge_handle: None,
                hit_portal_label: None,
                hit_edge_label: None,
                hit_section_resize_handle: None,
                hit_node_resize_handle: Some(("1".into(), ResizeHandleSide::SE)),
            })),
            DragState::PendingRight {
                start_pos: (1.0, 2.0),
                start_canvas: glam::Vec2::new(3.0, 4.0),
                hit_node: None,
                hit_section_idx: None,
            },
            DragState::Panning {
                owner: PanOwner::Held(HeldInput::Button(MouseButton::Middle)),
            },
            DragState::SelectingRect {
                start_canvas: glam::Vec2::new(1.0, 2.0),
                current_canvas: glam::Vec2::new(3.0, 4.5),
            },
            throttled(ThrottledDrag::MovingNode(MovingNodeInteraction::new(
                vec!["1".into()],
                true,
                HashSet::new(),
            ))),
            throttled(ThrottledDrag::MovingSection(MovingSectionInteraction::new(
                "1".into(),
                2,
                (0.0, 0.0),
            ))),
            throttled(ThrottledDrag::SectionResize(SectionResizeInteraction::new(
                "1".into(),
                2,
                ResizeHandleSide::NW,
                origin,
                unit,
                false,
            ))),
            throttled(ThrottledDrag::NodeResize(NodeResizeInteraction::new(
                "1".into(),
                ResizeHandleSide::E,
                origin,
                unit,
                true,
            ))),
            throttled(ThrottledDrag::EdgeHandle(EdgeHandleInteraction::new(
                edge_ref.clone(),
                EdgeHandleKind::ControlPoint(1),
                edge.clone(),
                glam::Vec2::ZERO,
            ))),
            throttled(ThrottledDrag::PortalLabel(PortalLabelInteraction::new(
                edge_ref.clone(),
                "1".into(),
                edge.clone(),
            ))),
            throttled(ThrottledDrag::EdgeLabel(EdgeLabelInteraction::new(
                edge_ref, edge,
            ))),
        ];
        for drag in &all {
            match drag {
                DragState::None
                | DragState::Pending(_)
                | DragState::PendingRight { .. }
                | DragState::Panning { .. }
                | DragState::SelectingRect { .. } => {}
                DragState::Throttled(inner) => match **inner {
                    ThrottledDrag::MovingNode(_)
                    | ThrottledDrag::MovingSection(_)
                    | ThrottledDrag::SectionResize(_)
                    | ThrottledDrag::NodeResize(_)
                    | ThrottledDrag::EdgeHandle(_)
                    | ThrottledDrag::PortalLabel(_)
                    | ThrottledDrag::EdgeLabel(_) => {}
                },
            }
        }
        all
    }

    #[test]
    fn test_every_drag_state_variant_serializes() {
        let all = every_drag_state();
        assert_eq!(
            all.len(),
            5 + 7,
            "five DragState variants, Throttled carrying seven gestures"
        );
        let mut gestures = Vec::new();
        for drag in &all {
            let json = drag.to_ipc_json();
            let kind = json["kind"].as_str().expect("every drag state carries a kind");
            let expected = match drag {
                DragState::None => "None",
                DragState::Pending(_) => "Pending",
                DragState::PendingRight { .. } => "PendingRight",
                DragState::Panning { .. } => "Panning",
                DragState::SelectingRect { .. } => "SelectingRect",
                DragState::Throttled(_) => "Throttled",
            };
            assert_eq!(kind, expected, "the kind is the variant's own name");
            if let DragState::Throttled(_) = drag {
                gestures.push(
                    json["gesture"]
                        .as_str()
                        .expect("a throttled drag names its gesture")
                        .to_string(),
                );
            }
        }
        gestures.sort();
        gestures.dedup();
        assert_eq!(gestures.len(), 7, "the seven gestures are distinct: {gestures:?}");
    }

    #[test]
    fn test_drag_payloads_carry_what_the_gesture_is_acting_on() {
        let all = every_drag_state();
        assert_eq!(
            all[1].to_ipc_json(),
            json!({
                "kind": "Pending",
                "start_pos": { "x": 4.0, "y": 5.0 },
                "hit_node": "1",
                "hit_section_idx": 2,
                "grabs": ["node_resize_handle"],
            })
        );
        assert_eq!(
            all[4].to_ipc_json(),
            json!({
                "kind": "SelectingRect",
                "start_canvas": { "x": 1.0, "y": 2.0 },
                "current_canvas": { "x": 3.0, "y": 4.5 },
            })
        );
        let moving = all[5].to_ipc_json();
        assert_eq!(moving["gesture"], json!("MovingNode"));
        assert_eq!(moving["node_ids"], json!(["1"]));
        assert_eq!(moving["individual"], json!(true));
        assert_eq!(all[8].to_ipc_json()["side"], json!("E"));
        assert_eq!(
            all[9].to_ipc_json()["handle"],
            json!({ "kind": "ControlPoint", "index": 1 })
        );
    }

    /// A pan reports what ends it (#153) — the owner is the one fact
    /// about a pan an agent driving it needs, since it decides whether
    /// the next `input.mouse_button` release, `input.key` release or
    /// `act.action PanCanvas` ends it. Each row is the others'
    /// negative control: three owners, three distinct replies.
    #[test]
    fn test_a_pan_reports_the_input_that_ends_it() {
        let pan = |owner| DragState::Panning { owner }.to_ipc_json();
        assert_eq!(
            pan(PanOwner::Held(HeldInput::Button(MouseButton::Middle))),
            json!({ "kind": "Panning", "owner": "button", "button": "middle" })
        );
        assert_eq!(
            pan(PanOwner::Held(HeldInput::Key("space".into()))),
            json!({ "kind": "Panning", "owner": "key", "key": "space" })
        );
        assert_eq!(
            pan(PanOwner::Toggled),
            json!({ "kind": "Panning", "owner": "toggled" })
        );
    }

    #[test]
    fn test_a_closed_editor_is_null_and_an_open_one_carries_its_buffer() {
        assert_eq!(TextEditState::Closed.to_ipc_json(), Value::Null);
        assert_eq!(SingleLineEditor::Closed.to_ipc_json(), Value::Null);
        assert_eq!(console_to_ipc_json(&ConsoleState::Closed), Value::Null);
        assert_eq!(picker_to_ipc_json(&ColorPickerState::Closed), Value::Null);

        let editor = SingleLineEditor::Open {
            target: SingleLineEditTarget::PortalText {
                edge_ref: EdgeRef::new("a", "b", "cross_link"),
                endpoint_node_id: "b".into(),
            },
            buffer: "héllo 👨‍👩‍👧".into(),
            cursor_grapheme_pos: 7,
            original: None,
        };
        assert_eq!(
            editor.to_ipc_json(),
            json!({
                "target": {
                    "kind": "PortalText",
                    "edge": { "from_id": "a", "to_id": "b", "edge_type": "cross_link" },
                    "endpoint_node_id": "b",
                },
                "buffer": "héllo 👨‍👩‍👧",
                "cursor_grapheme_pos": 7,
                "original": null,
            }),
            "the cursor is a grapheme index and the buffer travels as typed"
        );
    }

    #[test]
    fn test_an_open_console_reports_its_tail_and_its_total() {
        let mut scrollback = vec![ConsoleLine::Input("help".into())];
        scrollback.extend((0..CONSOLE_SCROLLBACK_TAIL + 5).map(|n| ConsoleLine::Output {
            text: format!("line {n}"),
            font_family: Some("x".into()),
        }));
        scrollback.push(ConsoleLine::Error("boom".into()));
        let total = scrollback.len();
        let state = ConsoleState::Open {
            input: "fps".into(),
            cursor: 3,
            history: vec!["a".into()],
            history_idx: Some(0),
            scrollback,
            completions: vec![],
            completion_idx: None,
            scroll_offset: 0,
            wheel_accum: 0.0,
        };
        let json = console_to_ipc_json(&state);
        assert_eq!(json["scrollback_len"], json!(total));
        let tail = json["scrollback"].as_array().expect("tail");
        assert_eq!(tail.len(), CONSOLE_SCROLLBACK_TAIL);
        assert_eq!(
            tail.last().expect("last"),
            &json!({ "kind": "Error", "text": "boom" })
        );
        assert_eq!(json["input"], json!("fps"));
        assert_eq!(json["history_idx"], json!(0));
    }

    fn open_picker() -> ColorPickerState {
        use crate::application::color_picker::CROSSHAIR_CENTER_CELL;
        ColorPickerState::Open(Box::new(OpenPicker {
            mode: PickerMode::Standalone,
            hue_deg: 120.0,
            sat: 0.5,
            val: 0.25,
            last_cursor_pos: None,
            max_cell_advance: 16.0,
            max_ring_advance: 24.0,
            measurement_font_size: 16.0,
            arm_top_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_bottom_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_left_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            arm_right_ink_offsets: [(0.0, 0.0); CROSSHAIR_CENTER_CELL],
            preview_ink_offset: (0.0, 0.0),
            layout: None,
            center_override: None,
            size_scale: 1.0,
            gesture: None,
            hovered_hit: None,
            hover_preview: None,
            pending_error_flash: false,
            last_dynamic_apply: None,
        }))
    }

    fn open_text_editor() -> TextEditState {
        TextEditState::Open {
            node_id: "1".into(),
            section_idx: 0,
            buffer: "hi".into(),
            cursor_grapheme_pos: 2,
            buffer_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
            original_text: String::new(),
            original_regions: baumhard::core::primitives::ColorFontRegions::new_empty(),
            selection_anchor: None,
            exit_to_default_on_close: false,
        }
    }

    fn open_single_line() -> SingleLineEditor {
        SingleLineEditor::Open {
            target: SingleLineEditTarget::EdgeLabel {
                edge_ref: EdgeRef::new("a", "b", "cross_link"),
            },
            buffer: String::new(),
            cursor_grapheme_pos: 0,
            original: None,
        }
    }

    /// **Key routing is a stack, not one owner** (review of #174): an
    /// open color picker claims only its own keys, so it is listed in
    /// `modals` and is never the `key_owner`; an ordinary key goes to
    /// the console, else the editors in `ModalEditor::STEAL_ORDER`.
    #[test]
    fn test_key_routing_lists_every_open_modal_and_names_who_gets_an_ordinary_key() {
        let closed = (
            ConsoleState::Closed,
            ColorPickerState::Closed,
            SingleLineEditor::Closed,
            TextEditState::Closed,
        );
        assert_eq!(
            key_routing(&closed.0, &closed.1, &closed.2, &closed.3),
            (vec![], "none")
        );
        assert_eq!(
            key_routing(&closed.0, &closed.1, &open_single_line(), &closed.3),
            (vec!["single_line_edit"], "single_line_edit")
        );
        assert_eq!(
            key_routing(&closed.0, &open_picker(), &closed.2, &closed.3),
            (vec!["color_picker"], "none"),
            "a picker alone leaves ordinary keys to normal dispatch"
        );
        assert_eq!(
            key_routing(&closed.0, &open_picker(), &closed.2, &open_text_editor()),
            (vec!["color_picker", "text_edit"], "text_edit"),
            "the picker is open, but every key it does not claim goes to the text editor"
        );
        let console = ConsoleState::open(vec![]);
        assert_eq!(
            key_routing(&console, &open_picker(), &open_single_line(), &open_text_editor()),
            (
                vec!["console", "color_picker", "single_line_edit", "text_edit"],
                "console"
            ),
            "the console takes every key, and the editors list in steal order"
        );
    }

    #[test]
    fn test_the_editors_steal_order_is_the_one_list() {
        assert_eq!(
            ModalEditor::stealing_from(true, true),
            Some(ModalEditor::SingleLine)
        );
        assert_eq!(ModalEditor::stealing_from(false, true), Some(ModalEditor::Text));
        assert_eq!(ModalEditor::stealing_from(false, false), None);
        assert_eq!(
            ModalEditor::open_in_steal_order(true, true).collect::<Vec<_>>(),
            ModalEditor::STEAL_ORDER.to_vec()
        );
    }

    /// The grab order `query.mode` reports is the order the
    /// threshold-cross step resolves (review of #174): the press that
    /// hit an edge label *and* an edge handle reports the label first,
    /// because that is the gesture it becomes.
    #[test]
    fn test_pending_grabs_report_in_the_threshold_cross_precedence() {
        let key = baumhard::mindmap::scene_cache::EdgeKey::new("a", "b", "cross_link");
        let press = PendingPress {
            start_pos: (0.0, 0.0),
            hit_node: Some("1".into()),
            hit_section_idx: None,
            hit_edge_handle: Some((EdgeRef::new("a", "b", "cross_link"), EdgeHandleKind::Midpoint)),
            hit_portal_label: None,
            hit_edge_label: Some(key),
            hit_section_resize_handle: Some(("1".into(), 0, ResizeHandleSide::N)),
            hit_node_resize_handle: Some(("1".into(), ResizeHandleSide::S)),
        };
        assert_eq!(press.first_grab(), Some(Grab::EdgeLabel));
        assert_eq!(
            press.grabs().map(Grab::word).collect::<Vec<_>>(),
            vec![
                "edge_label",
                "edge_handle",
                "node_resize_handle",
                "section_resize_handle"
            ]
        );
        let bare = PendingPress {
            start_pos: (0.0, 0.0),
            hit_node: None,
            hit_section_idx: None,
            hit_edge_handle: None,
            hit_portal_label: None,
            hit_edge_label: None,
            hit_section_resize_handle: None,
            hit_node_resize_handle: None,
        };
        assert_eq!(bare.first_grab(), None);
        for grab in Grab::PRECEDENCE {
            assert_eq!(
                Grab::PRECEDENCE.iter().filter(|g| **g == grab).count(),
                1,
                "{grab:?} appears once in the precedence list"
            );
        }
    }

    #[test]
    fn test_an_open_picker_reports_its_mode_and_hsv() {
        let json = picker_to_ipc_json(&open_picker());
        assert_eq!(json["mode"], json!({ "kind": "Standalone" }));
        assert_eq!(json["hue_deg"], json!(120.0));
        assert_eq!(json["sat"], json!(0.5));
        assert_eq!(json["val"], json!(0.25));
        assert_eq!(json["gesture"], Value::Null);
    }

    #[test]
    fn test_mode_and_editors_replies_have_their_documented_shape() {
        let closed = (
            ConsoleState::Closed,
            ColorPickerState::Closed,
            SingleLineEditor::Closed,
            TextEditState::Closed,
        );
        let mode = mode_to_ipc_json(
            &InteractionMode::Default,
            &DragState::None,
            &closed.0,
            &closed.1,
            &closed.2,
            &closed.3,
            Some("1.2"),
            (10.0, 20.5),
        );
        assert_eq!(
            mode,
            json!({
                "interaction_mode": { "kind": "Default" },
                "drag_state": { "kind": "None" },
                "modals": [],
                "key_owner": "none",
                "hovered_node": "1.2",
                "cursor_pos": { "x": 10.0, "y": 20.5 },
            })
        );
        let editors = editors_to_ipc_json(&closed.0, &closed.1, &closed.2, &closed.3, None);
        for key in ["console", "color_picker", "single_line_edit", "text_edit"] {
            assert_eq!(editors[key], Value::Null, "{key}");
        }
        assert!(editors["previews"].is_object());
    }
}
