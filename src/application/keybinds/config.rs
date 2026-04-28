// SPDX-License-Identifier: MPL-2.0

//! `KeybindConfig` — the user-editable config struct + JSON loader +
//! `resolve()` step that produces the runtime `ResolvedKeybinds` table.

use log::warn;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use super::action::Action;
use super::bind::KeyBind;
use super::resolved::ResolvedKeybinds;

/// The raw, user-editable config. Every field is a list of binding strings
/// so users can assign multiple keys to the same action (e.g. Ctrl+Z and
/// the Undo key both mapped to `Undo`). Fields default via serde so a
/// partial config only has to mention the actions the user wants to
/// override.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct KeybindConfig {
    // ── Document-level (global) ──────────────────────────────────
    pub undo: Vec<String>,
    pub enter_reparent_mode: Vec<String>,
    pub enter_connect_mode: Vec<String>,
    pub delete_selection: Vec<String>,
    pub cancel_mode: Vec<String>,
    pub create_orphan_node: Vec<String>,
    pub orphan_selection: Vec<String>,
    pub edit_selection: Vec<String>,
    pub edit_selection_clean: Vec<String>,
    pub open_console: Vec<String>,
    pub save_document: Vec<String>,
    pub copy: Vec<String>,
    pub paste: Vec<String>,
    pub cut: Vec<String>,

    // ── Console ──────────────────────────────────────────────────
    pub console_close: Vec<String>,
    pub console_submit: Vec<String>,
    pub console_tab_complete: Vec<String>,
    pub console_history_up: Vec<String>,
    pub console_history_down: Vec<String>,
    pub console_cursor_left: Vec<String>,
    pub console_cursor_right: Vec<String>,
    pub console_cursor_home: Vec<String>,
    pub console_cursor_end: Vec<String>,
    pub console_delete_back: Vec<String>,
    pub console_delete_forward: Vec<String>,
    pub console_insert_space: Vec<String>,
    pub console_clear_line: Vec<String>,
    pub console_jump_start: Vec<String>,
    pub console_jump_end: Vec<String>,
    pub console_kill_to_start: Vec<String>,
    pub console_kill_word: Vec<String>,
    pub console_scroll_up: Vec<String>,
    pub console_scroll_down: Vec<String>,
    pub console_scroll_page_up: Vec<String>,
    pub console_scroll_page_down: Vec<String>,
    pub console_scroll_end: Vec<String>,
    pub console_scroll_home: Vec<String>,

    // ── Color Picker ─────────────────────────────────────────────
    pub picker_cancel: Vec<String>,
    pub picker_commit: Vec<String>,
    pub picker_nudge_hue_down: Vec<String>,
    pub picker_nudge_hue_up: Vec<String>,
    pub picker_nudge_sat_down: Vec<String>,
    pub picker_nudge_sat_up: Vec<String>,
    pub picker_nudge_val_down: Vec<String>,
    pub picker_nudge_val_up: Vec<String>,

    // ── Label Editor ─────────────────────────────────────────────
    pub label_edit_cancel: Vec<String>,
    pub label_edit_commit: Vec<String>,

    // ── Text Editor ──────────────────────────────────────────────
    pub text_edit_cancel: Vec<String>,

    // ── Mouse-gesture Actions ────────────────────────────────────
    pub double_click_activate: Vec<String>,
    pub create_orphan_node_and_edit: Vec<String>,
    pub pan_canvas: Vec<String>,
    pub commit_or_close_editor: Vec<String>,

    // ── Navigation / camera ──────────────────────────────────────
    pub zoom_in: Vec<String>,
    pub zoom_out: Vec<String>,
    pub zoom_reset: Vec<String>,
    pub zoom_fit: Vec<String>,
    pub pan_camera_north: Vec<String>,
    pub pan_camera_south: Vec<String>,
    pub pan_camera_east: Vec<String>,
    pub pan_camera_west: Vec<String>,
    pub center_on_selection: Vec<String>,
    pub jump_to_root: Vec<String>,

    // ── Selection ────────────────────────────────────────────────
    pub select_all: Vec<String>,
    pub deselect_all: Vec<String>,
    pub invert_selection: Vec<String>,
    pub select_parent: Vec<String>,
    pub select_child: Vec<String>,
    pub select_next_sibling: Vec<String>,
    pub select_prev_sibling: Vec<String>,

    // ── TextEdit cursor primitives ──────────────────────────────
    pub text_edit_cursor_left: Vec<String>,
    pub text_edit_cursor_right: Vec<String>,
    pub text_edit_cursor_up: Vec<String>,
    pub text_edit_cursor_down: Vec<String>,
    pub text_edit_cursor_home: Vec<String>,
    pub text_edit_cursor_end: Vec<String>,
    pub text_edit_word_left: Vec<String>,
    pub text_edit_word_right: Vec<String>,
    pub text_edit_delete_back: Vec<String>,
    pub text_edit_delete_forward: Vec<String>,
    pub text_edit_delete_word_back: Vec<String>,
    pub text_edit_delete_word_forward: Vec<String>,
    pub text_edit_commit: Vec<String>,

    // ── LabelEdit cursor primitives ─────────────────────────────
    pub label_edit_cursor_left: Vec<String>,
    pub label_edit_cursor_right: Vec<String>,
    pub label_edit_cursor_home: Vec<String>,
    pub label_edit_cursor_end: Vec<String>,
    pub label_edit_delete_back: Vec<String>,
    pub label_edit_delete_forward: Vec<String>,

    // ── Console-verb Actions ────────────────────────────────────
    pub open_color_picker: Vec<String>,
    pub close_color_picker: Vec<String>,
    pub label_edit_on_selection: Vec<String>,
    pub toggle_fps: Vec<String>,
    pub toggle_fps_debug: Vec<String>,
    pub new_document: Vec<String>,

    // ── Style / metadata ─────────────────────────────────────────
    /// Font family name for the console overlay.
    pub console_font: String,
    /// Font size in pixels for the console overlay.
    pub console_font_size: f32,
    /// Map of key combo → custom mutation id.
    pub custom_mutation_bindings: HashMap<String, String>,
}

impl Default for KeybindConfig {
    fn default() -> Self {
        KeybindConfig {
            // Document-level
            undo: vec!["Ctrl+Z".into(), "Undo".into()],
            enter_reparent_mode: vec!["Ctrl+P".into()],
            enter_connect_mode: vec!["Ctrl+D".into()],
            delete_selection: vec!["Delete".into()],
            cancel_mode: vec!["Escape".into()],
            create_orphan_node: vec!["Ctrl+N".into()],
            orphan_selection: vec!["Ctrl+O".into()],
            edit_selection: vec!["Enter".into()],
            edit_selection_clean: vec!["Backspace".into()],
            open_console: vec!["/".into()],
            save_document: vec!["Ctrl+S".into()],
            copy: vec!["Ctrl+C".into(), "Copy".into()],
            paste: vec!["Ctrl+V".into(), "Paste".into()],
            cut: vec!["Ctrl+X".into(), "Cut".into()],

            // Console
            console_close: vec!["Escape".into()],
            console_submit: vec!["Enter".into()],
            console_tab_complete: vec!["Tab".into()],
            console_history_up: vec!["ArrowUp".into(), "Up".into()],
            console_history_down: vec!["ArrowDown".into(), "Down".into()],
            console_cursor_left: vec!["ArrowLeft".into(), "Left".into()],
            console_cursor_right: vec!["ArrowRight".into(), "Right".into()],
            console_cursor_home: vec!["Home".into()],
            console_cursor_end: vec!["End".into()],
            console_delete_back: vec!["Backspace".into()],
            console_delete_forward: vec!["Delete".into()],
            console_insert_space: vec!["Space".into()],
            console_clear_line: vec!["Ctrl+C".into()],
            console_jump_start: vec!["Ctrl+A".into()],
            console_jump_end: vec!["Ctrl+E".into()],
            console_kill_to_start: vec!["Ctrl+U".into()],
            console_kill_word: vec!["Ctrl+W".into()],
            console_scroll_up: vec!["Shift+ArrowUp".into(), "Shift+Up".into()],
            console_scroll_down: vec!["Shift+ArrowDown".into(), "Shift+Down".into()],
            console_scroll_page_up: vec!["PageUp".into()],
            console_scroll_page_down: vec!["PageDown".into()],
            console_scroll_end: vec!["Shift+End".into()],
            console_scroll_home: vec!["Shift+Home".into()],

            // Color Picker
            picker_cancel: vec!["Escape".into()],
            picker_commit: vec!["Enter".into()],
            picker_nudge_hue_down: vec!["h".into()],
            picker_nudge_hue_up: vec!["Shift+h".into()],
            picker_nudge_sat_down: vec!["s".into()],
            picker_nudge_sat_up: vec!["Shift+s".into()],
            picker_nudge_val_down: vec!["v".into()],
            picker_nudge_val_up: vec!["Shift+v".into()],

            // Label Editor
            label_edit_cancel: vec!["Escape".into()],
            label_edit_commit: vec!["Enter".into()],

            // Text Editor
            text_edit_cancel: vec!["Escape".into()],

            // Mouse-gesture Actions. Bodies for these arms land in
            // Phase 4. Defaults that the user explicitly approved are
            // set here; the rest ship empty until their arm is wired.
            //
            // `create_orphan_node_and_edit` is intentionally `vec![]` —
            // the user found the empty-canvas double-click annoying;
            // it's now opt-in.
            double_click_activate: vec!["DoubleClick".into()],
            create_orphan_node_and_edit: vec![],
            pan_canvas: vec!["LeftDrag".into(), "MiddleClick".into()],
            commit_or_close_editor: vec![],

            // Navigation / camera. `ZoomIn`/`ZoomOut` default to mouse
            // wheel per user request; key shortcuts (e.g. Ctrl++/Ctrl+-)
            // can be added in user keybinds.json.
            zoom_in: vec!["WheelUp".into()],
            zoom_out: vec!["WheelDown".into()],
            zoom_reset: vec![],
            zoom_fit: vec![],
            pan_camera_north: vec![],
            pan_camera_south: vec![],
            pan_camera_east: vec![],
            pan_camera_west: vec![],
            center_on_selection: vec![],
            jump_to_root: vec![],

            // Selection. All defaults empty; users opt in via
            // keybinds.json.
            select_all: vec![],
            deselect_all: vec![],
            invert_selection: vec![],
            select_parent: vec![],
            select_child: vec![],
            select_next_sibling: vec![],
            select_prev_sibling: vec![],

            // TextEdit cursor primitives. Bodies live in
            // dispatch::apply_text_edit_action; the editor's modal
            // handler routes through dispatch when a binding matches
            // and falls back to literal-character insertion otherwise.
            text_edit_cursor_left: vec!["ArrowLeft".into()],
            text_edit_cursor_right: vec!["ArrowRight".into()],
            text_edit_cursor_up: vec!["ArrowUp".into()],
            text_edit_cursor_down: vec!["ArrowDown".into()],
            text_edit_cursor_home: vec!["Home".into()],
            text_edit_cursor_end: vec!["End".into()],
            text_edit_word_left: vec!["Ctrl+ArrowLeft".into()],
            text_edit_word_right: vec!["Ctrl+ArrowRight".into()],
            text_edit_delete_back: vec!["Backspace".into()],
            text_edit_delete_forward: vec!["Delete".into()],
            text_edit_delete_word_back: vec!["Ctrl+Backspace".into()],
            text_edit_delete_word_forward: vec!["Ctrl+Delete".into()],
            // Default unbound — Enter is literal `\n` in the multi-
            // line node editor. Users who want commit-on-Enter bind
            // it themselves (and lose newline insertion in exchange).
            text_edit_commit: vec![],

            // LabelEdit cursor primitives. Same routing path as
            // TextEdit but single-line — no Up/Down/Word*. Defaults
            // mirror what `route_label_edit_key` previously
            // hardcoded.
            label_edit_cursor_left: vec!["ArrowLeft".into()],
            label_edit_cursor_right: vec!["ArrowRight".into()],
            label_edit_cursor_home: vec!["Home".into()],
            label_edit_cursor_end: vec!["End".into()],
            label_edit_delete_back: vec!["Backspace".into()],
            label_edit_delete_forward: vec!["Delete".into()],

            // Console-verb Actions. Bodies land in Phase 6. Defaults
            // empty — these mirror typed console verbs and the user
            // opts in by binding a key.
            open_color_picker: vec![],
            close_color_picker: vec![],
            label_edit_on_selection: vec![],
            toggle_fps: vec![],
            toggle_fps_debug: vec![],
            new_document: vec![],

            // Style / metadata
            console_font: String::new(),
            console_font_size: 16.0,
            custom_mutation_bindings: HashMap::new(),
        }
    }
}

impl KeybindConfig {
    /// Parse a JSON string into a config. Missing fields fall back to
    /// defaults thanks to `#[serde(default)]` on the struct.
    pub fn from_json(json: &str) -> Result<Self, String> {
        serde_json::from_str(json).map_err(|e| format!("parse keybinds JSON: {}", e))
    }

    /// Parse every binding string into concrete `KeyBind` values. Any
    /// binding that fails to parse is logged and skipped so a single typo
    /// doesn't break the entire config.
    pub fn resolve(&self) -> ResolvedKeybinds {
        let mut binds: Vec<(Action, KeyBind)> = Vec::new();
        let sets: &[(Action, &Vec<String>)] = &[
            // Document-level
            (Action::Undo, &self.undo),
            (Action::EnterReparentMode, &self.enter_reparent_mode),
            (Action::EnterConnectMode, &self.enter_connect_mode),
            (Action::DeleteSelection, &self.delete_selection),
            (Action::CancelMode, &self.cancel_mode),
            (Action::CreateOrphanNode, &self.create_orphan_node),
            (Action::OrphanSelection, &self.orphan_selection),
            (Action::EditSelection, &self.edit_selection),
            (Action::EditSelectionClean, &self.edit_selection_clean),
            (Action::OpenConsole, &self.open_console),
            (Action::SaveDocument, &self.save_document),
            (Action::Copy, &self.copy),
            (Action::Paste, &self.paste),
            (Action::Cut, &self.cut),
            // Console
            (Action::ConsoleClose, &self.console_close),
            (Action::ConsoleSubmit, &self.console_submit),
            (Action::ConsoleTabComplete, &self.console_tab_complete),
            (Action::ConsoleHistoryUp, &self.console_history_up),
            (Action::ConsoleHistoryDown, &self.console_history_down),
            (Action::ConsoleCursorLeft, &self.console_cursor_left),
            (Action::ConsoleCursorRight, &self.console_cursor_right),
            (Action::ConsoleCursorHome, &self.console_cursor_home),
            (Action::ConsoleCursorEnd, &self.console_cursor_end),
            (Action::ConsoleDeleteBack, &self.console_delete_back),
            (Action::ConsoleDeleteForward, &self.console_delete_forward),
            (Action::ConsoleInsertSpace, &self.console_insert_space),
            (Action::ConsoleClearLine, &self.console_clear_line),
            (Action::ConsoleJumpStart, &self.console_jump_start),
            (Action::ConsoleJumpEnd, &self.console_jump_end),
            (Action::ConsoleKillToStart, &self.console_kill_to_start),
            (Action::ConsoleKillWord, &self.console_kill_word),
            (Action::ConsoleScrollUp, &self.console_scroll_up),
            (Action::ConsoleScrollDown, &self.console_scroll_down),
            (Action::ConsoleScrollPageUp, &self.console_scroll_page_up),
            (Action::ConsoleScrollPageDown, &self.console_scroll_page_down),
            (Action::ConsoleScrollEnd, &self.console_scroll_end),
            (Action::ConsoleScrollHome, &self.console_scroll_home),
            // Color Picker
            (Action::PickerCancel, &self.picker_cancel),
            (Action::PickerCommit, &self.picker_commit),
            (Action::PickerNudgeHueDown, &self.picker_nudge_hue_down),
            (Action::PickerNudgeHueUp, &self.picker_nudge_hue_up),
            (Action::PickerNudgeSatDown, &self.picker_nudge_sat_down),
            (Action::PickerNudgeSatUp, &self.picker_nudge_sat_up),
            (Action::PickerNudgeValDown, &self.picker_nudge_val_down),
            (Action::PickerNudgeValUp, &self.picker_nudge_val_up),
            // Label Editor
            (Action::LabelEditCancel, &self.label_edit_cancel),
            (Action::LabelEditCommit, &self.label_edit_commit),
            // Text Editor
            (Action::TextEditCancel, &self.text_edit_cancel),
            // Mouse-gesture Actions
            (Action::DoubleClickActivate, &self.double_click_activate),
            (Action::CreateOrphanNodeAndEdit, &self.create_orphan_node_and_edit),
            (Action::PanCanvas, &self.pan_canvas),
            (Action::CommitOrCloseEditor, &self.commit_or_close_editor),
            // Navigation / camera
            (Action::ZoomIn, &self.zoom_in),
            (Action::ZoomOut, &self.zoom_out),
            (Action::ZoomReset, &self.zoom_reset),
            (Action::ZoomFit, &self.zoom_fit),
            (Action::PanCameraNorth, &self.pan_camera_north),
            (Action::PanCameraSouth, &self.pan_camera_south),
            (Action::PanCameraEast, &self.pan_camera_east),
            (Action::PanCameraWest, &self.pan_camera_west),
            (Action::CenterOnSelection, &self.center_on_selection),
            (Action::JumpToRoot, &self.jump_to_root),
            // Selection
            (Action::SelectAll, &self.select_all),
            (Action::DeselectAll, &self.deselect_all),
            (Action::InvertSelection, &self.invert_selection),
            (Action::SelectParent, &self.select_parent),
            (Action::SelectChild, &self.select_child),
            (Action::SelectNextSibling, &self.select_next_sibling),
            (Action::SelectPrevSibling, &self.select_prev_sibling),
            // TextEdit cursor primitives
            (Action::TextEditCursorLeft, &self.text_edit_cursor_left),
            (Action::TextEditCursorRight, &self.text_edit_cursor_right),
            (Action::TextEditCursorUp, &self.text_edit_cursor_up),
            (Action::TextEditCursorDown, &self.text_edit_cursor_down),
            (Action::TextEditCursorHome, &self.text_edit_cursor_home),
            (Action::TextEditCursorEnd, &self.text_edit_cursor_end),
            (Action::TextEditWordLeft, &self.text_edit_word_left),
            (Action::TextEditWordRight, &self.text_edit_word_right),
            (Action::TextEditDeleteBack, &self.text_edit_delete_back),
            (Action::TextEditDeleteForward, &self.text_edit_delete_forward),
            (Action::TextEditDeleteWordBack, &self.text_edit_delete_word_back),
            (Action::TextEditDeleteWordForward, &self.text_edit_delete_word_forward),
            (Action::TextEditCommit, &self.text_edit_commit),
            // LabelEdit cursor primitives
            (Action::LabelEditCursorLeft, &self.label_edit_cursor_left),
            (Action::LabelEditCursorRight, &self.label_edit_cursor_right),
            (Action::LabelEditCursorHome, &self.label_edit_cursor_home),
            (Action::LabelEditCursorEnd, &self.label_edit_cursor_end),
            (Action::LabelEditDeleteBack, &self.label_edit_delete_back),
            (Action::LabelEditDeleteForward, &self.label_edit_delete_forward),
            // Console-verb Actions
            (Action::OpenColorPicker, &self.open_color_picker),
            (Action::CloseColorPicker, &self.close_color_picker),
            (Action::LabelEditOnSelection, &self.label_edit_on_selection),
            (Action::ToggleFps, &self.toggle_fps),
            (Action::ToggleFpsDebug, &self.toggle_fps_debug),
            (Action::NewDocument, &self.new_document),
        ];
        for (action, strings) in sets {
            for s in *strings {
                match KeyBind::parse(s) {
                    Ok(k) => binds.push((*action, k)),
                    Err(e) => warn!("skipping invalid keybind '{}': {}", s, e),
                }
            }
        }
        let mut custom_binds: Vec<(KeyBind, String)> = Vec::new();
        for (combo, mutation_id) in &self.custom_mutation_bindings {
            match KeyBind::parse(combo) {
                Ok(k) => custom_binds.push((k, mutation_id.clone())),
                Err(e) => warn!(
                    "skipping invalid custom_mutation_binding '{}': {}",
                    combo, e
                ),
            }
        }

        ResolvedKeybinds::new(
            binds,
            custom_binds,
            self.console_font.clone(),
            self.console_font_size.max(4.0),
        )
    }
}
