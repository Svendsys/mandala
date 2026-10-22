// SPDX-License-Identifier: MPL-2.0

//! IPC projections of the app shell's own state (`format/ipc.md`
//! §`query`): the interaction mode, the drag state machine, the four
//! modal editors, the camera. The document-layer half lives beside
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

use baumhard::gfx_structs::camera::Camera2D;
use baumhard::mindmap::tree_builder::{EdgeHandleKind, ResizeHandleSide};

use crate::application::color_picker::{
    ColorPickerState, OpenPicker, PickerGesture, PickerHandle, PickerMode,
};
use crate::application::console::{ConsoleLine, ConsoleState};
use crate::application::document::{f32_json, MindMapDocument};

use super::interaction_mode::ResizeTarget;
use super::single_line_edit::{SingleLineEditTarget, SingleLineEditor};
use super::text_edit::TextEditState;
use super::throttled_interaction::ThrottledDrag;
use super::{DragState, InteractionMode, PendingPress};

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

/// The camera as `query.camera` reports it: the `surface` ↔ `canvas`
/// transform (`format/ipc.md` §Coordinate spaces). `center` is the
/// canvas point at the middle of the viewport; a canvas point `p`
/// lands at surface `(p - center) * zoom + surface / 2`.
pub(super) fn camera_to_ipc_json(camera: &Camera2D) -> Value {
    json!({
        "center": xy_vec2(camera.position),
        "zoom": f32_json(camera.zoom),
        "zoom_min": f32_json(Camera2D::MIN_ZOOM),
        "zoom_max": f32_json(Camera2D::MAX_ZOOM),
        "surface": { "width": camera.viewport_size.0, "height": camera.viewport_size.1 },
    })
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
    pub(super) fn to_ipc_json(&self) -> Value {
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

impl PendingPress {
    /// What the press landed on, by name: every populated grab slot,
    /// in the precedence order the threshold-cross arm applies
    /// (a handle beats a node).
    fn grabs(&self) -> Vec<&'static str> {
        let mut grabs = Vec::new();
        if self.hit_edge_handle.is_some() {
            grabs.push("edge_handle");
        }
        if self.hit_portal_label.is_some() {
            grabs.push("portal_label");
        }
        if self.hit_edge_label.is_some() {
            grabs.push("edge_label");
        }
        if self.hit_section_resize_handle.is_some() {
            grabs.push("section_resize_handle");
        }
        if self.hit_node_resize_handle.is_some() {
            grabs.push("node_resize_handle");
        }
        grabs
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
                "grabs": press.grabs(),
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
            DragState::Panning => json!({ "kind": "Panning" }),
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

/// Which modal currently takes the keyboard first, by the order the
/// keyboard handler steals in (`event_keyboard.rs`): the console,
/// then the color picker, then the edge-label / portal-caption
/// editor; the node text editor is reached through the action
/// dispatcher rather than a steal, so it comes last. `"none"` when
/// all four are closed.
fn modal_word(
    console: &ConsoleState,
    picker: &ColorPickerState,
    single_line: &SingleLineEditor,
    text: &TextEditState,
) -> &'static str {
    if console.is_open() {
        "console"
    } else if picker.as_open().is_some() {
        "color_picker"
    } else if single_line.is_open() {
        "single_line_edit"
    } else if text.is_open() {
        "text_edit"
    } else {
        "none"
    }
}

/// `query.mode`'s reply: the interaction mode and the drag state
/// machine, which modal is open, and where the pointer is.
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
    json!({
        "interaction_mode": mode.to_ipc_json(),
        "drag_state": drag.to_ipc_json(),
        "modal": modal_word(console, picker, single_line, text),
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
        "previews": previews_json(document),
    })
}

/// The document's `previews` half of `query.editors` (see
/// `MindMapDocument::ipc_previews_json`), or an all-`null` object when
/// no document is loaded.
fn previews_json(document: Option<&MindMapDocument>) -> Value {
    match document {
        Some(doc) => doc.ipc_previews_json(),
        None => json!({
            "label_edit": null,
            "portal_text_edit": null,
            "color_picker": null,
            "border": null,
            "rect_select": null,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::app::throttled_interaction::{
        EdgeHandleInteraction, EdgeLabelInteraction, MovingNodeInteraction, MovingSectionInteraction,
        NodeResizeInteraction, PortalLabelInteraction, SectionResizeInteraction,
    };
    use crate::application::document::tests_common::load_test_doc;
    use crate::application::document::EdgeRef;
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
            DragState::Panning,
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
                | DragState::Panning
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
                DragState::Panning => "Panning",
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

    #[test]
    fn test_camera_json_names_the_transform_and_the_zoom_clamp() {
        let mut camera = Camera2D::new(1920, 1080);
        camera.position = glam::Vec2::new(12.0, -340.5);
        camera.zoom = 0.75;
        assert_eq!(
            camera_to_ipc_json(&camera),
            json!({
                "center": { "x": 12.0, "y": -340.5 },
                "zoom": 0.75,
                "zoom_min": 0.05,
                "zoom_max": 5.0,
                "surface": { "width": 1920, "height": 1080 },
            })
        );
    }

    #[test]
    fn test_the_modal_word_follows_the_keyboard_steal_order() {
        let closed = (
            ConsoleState::Closed,
            ColorPickerState::Closed,
            SingleLineEditor::Closed,
            TextEditState::Closed,
        );
        assert_eq!(modal_word(&closed.0, &closed.1, &closed.2, &closed.3), "none");
        let open_single = SingleLineEditor::Open {
            target: SingleLineEditTarget::EdgeLabel {
                edge_ref: EdgeRef::new("a", "b", "cross_link"),
            },
            buffer: String::new(),
            cursor_grapheme_pos: 0,
            original: None,
        };
        assert_eq!(
            modal_word(&closed.0, &closed.1, &open_single, &closed.3),
            "single_line_edit"
        );
        let open_console = ConsoleState::open(vec![]);
        assert_eq!(
            modal_word(&open_console, &closed.1, &open_single, &closed.3),
            "console",
            "the console steals first"
        );
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
                "modal": "none",
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

    #[test]
    fn test_previews_are_all_null_without_a_document_and_with_no_preview_live() {
        let none = previews_json(None);
        let doc = load_test_doc();
        assert_eq!(
            none,
            previews_json(Some(&doc)),
            "a fresh document has no preview live"
        );
        for key in [
            "label_edit",
            "portal_text_edit",
            "color_picker",
            "border",
            "rect_select",
        ] {
            assert_eq!(none[key], Value::Null, "{key}");
        }
    }
}
