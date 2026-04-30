// SPDX-License-Identifier: MPL-2.0

//! Application-crate root. The three load-bearing modules
//! follow the model/view/dispatch boundary documented in
//! `CODE_CONVENTIONS.md §3`:
//!
//! - [`document`] — `MindMapDocument` + mutation / undo cascade.
//!   Pure data + transformations, no GPU.
//! - [`renderer`] — wgpu pipelines + cosmic-text integration.
//!   Pure presentation, no document mutation.
//! - [`app`] — event loop, modal state machines, and the
//!   single dispatch funnel every user-driven `Action` flows
//!   through (§3 "Single dispatch funnel").
//!
//! The crate's binary entry point (`src/main.rs`) constructs
//! [`app::Application`] and calls [`app::Application::run`];
//! everything else hangs off that. Other modules are subsystems
//! of those three (each has its own `//!` header describing
//! its concept).

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
