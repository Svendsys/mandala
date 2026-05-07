// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use crate::application::app::{Application, Options};
use crate::application::common::{InputMode, WindowMode};
use crate::application::keybinds::KeybindConfig;
use log::info;

mod application;

const DEFAULT_MINDMAP: &str = "maps/testament.mindmap.json";

#[cfg(not(target_arch = "wasm32"))]
fn parse_cli() -> (String, Option<std::path::PathBuf>) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut mindmap_path: Option<String> = None;
    let mut keybinds_path: Option<std::path::PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--keybinds" {
            if let Some(val) = args.get(i + 1) {
                keybinds_path = Some(std::path::PathBuf::from(val));
                i += 2;
                continue;
            }
        } else if let Some(val) = a.strip_prefix("--keybinds=") {
            keybinds_path = Some(std::path::PathBuf::from(val));
        } else if !a.starts_with("--") && mindmap_path.is_none() {
            mindmap_path = Some(a.clone());
        }
        i += 1;
    }
    (
        mindmap_path.unwrap_or_else(|| DEFAULT_MINDMAP.to_string()),
        keybinds_path,
    )
}

#[cfg(not(target_arch = "wasm32"))]
fn create_options() -> Options {
    let (mindmap_path, keybinds_path) = parse_cli();
    let keybind_config = KeybindConfig::load_for_desktop(keybinds_path.as_deref());

    Options {
        launch_gpu_prefer_low_power: false,
        should_exit: false,
        window_mode: WindowMode::WindowedFullscreen,
        ui_scale: 0,
        window_title_text: "Mandala",
        input_mode: InputMode::MappedToInstruction,
        avail_cores: num_cpus::get(),
        render_must_be_main: false,
        mindmap_path,
        keybind_config,
    }
}

#[cfg(target_arch = "wasm32")]
fn create_options() -> Options {
    // WASM: mindmap_path and keybind_config are replaced later by run_wasm.
    Options {
        launch_gpu_prefer_low_power: false,
        should_exit: false,
        window_mode: WindowMode::WindowedFullscreen,
        ui_scale: 0,
        window_title_text: "Mandala",
        input_mode: InputMode::MappedToInstruction,
        avail_cores: 1,
        render_must_be_main: false,
        mindmap_path: DEFAULT_MINDMAP.to_string(),
        keybind_config: KeybindConfig::default(),
    }
}

fn main() {
    baumhard::util::log::init();
    #[cfg(not(target_arch = "wasm32"))]
    info!("Starting Mandala (native)");
    #[cfg(target_arch = "wasm32")]
    info!("Starting Mandala (WASM)");

    let app = Application::new(create_options());
    app.run();
}
