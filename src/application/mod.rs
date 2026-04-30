// SPDX-License-Identifier: MPL-2.0

//! Application-crate root: every interactive surface Mandala
//! exposes to the user lives under this module tree. The split
//! follows the model/view/dispatch boundary documented in
//! `CODE_CONVENTIONS.md §3`:
//!
//! - [`document`] owns `MindMapDocument` + the mutation /
//!   undo cascade — pure data + transformations, no GPU.
//! - [`renderer`] owns the wgpu pipelines + cosmic-text
//!   integration — pure presentation, no document mutation.
//! - [`app`] owns the event loop, modal state machines, and
//!   the dispatch funnel that ties the two halves together.
//!   `app::dispatch` is the single funnel every user-driven
//!   `Action` flows through (§3 "Single dispatch funnel").
//! - Subsystems: [`keybinds`] (config-loaded action table),
//!   [`console`] (verb parser + completion), [`macros`]
//!   (multi-step replay registry), [`color_picker`] +
//!   [`color_picker_overlay`] (glyph-wheel HSV picker),
//!   [`clipboard`] (cross-platform copy/paste), [`scene_host`]
//!   (canvas vs. overlay scene-tree slots), [`user_config`]
//!   (XDG / web-storage shared loader plumbing),
//!   [`frame_throttle`] (per-frame cap math), [`common`]
//!   (the small enums shared across the rest:
//!   `RedrawMode`, `RenderDecree`, `FpsDisplayMode`,
//!   `PollTimer`, `StopWatch`).
//!
//! The crate's binary entry point (`src/main.rs`) constructs
//! [`app::Application`] and calls [`app::Application::run`];
//! everything else hangs off that.

pub(crate) mod app;
pub(crate) mod clipboard;
pub(crate) mod color_picker;
pub(crate) mod color_picker_overlay;
pub(crate) mod common;
pub(crate) mod console;
pub(crate) mod document;
pub(crate) mod frame_throttle;
pub(crate) mod keybinds;
pub(crate) mod macros;
pub mod renderer;
pub(crate) mod scene_host;
pub(crate) mod user_config;
pub(crate) mod widgets;
