// SPDX-License-Identifier: MPL-2.0

//! Logger initialisation — single entry point for both targets.
//!
//! The `log` crate's macros (`log::info!` / `warn!` / ...) are
//! the universal Rust idiom: every alternative (`tracing`,
//! `defmt`, structured collectors) implements `log::Log` or
//! provides the same names. Wrapping the macros themselves
//! gains nothing portable, so callsites continue to use
//! `log::warn!(...)` etc. directly.
//!
//! What WAS scattered across `src/main.rs` was the per-target
//! init — `env_logger::init()` on native, `console_log::init_with_level`
//! on WASM, plus the panic hook wiring on WASM. This module
//! collapses both onto one [`init`] call.

/// Initialise the global logger for whichever target this binary
/// is built for. Native uses `env_logger` (reads `RUST_LOG`);
/// WASM uses `console_log` at `Info` level and installs the
/// `console_error_panic_hook` so a panic surfaces a JS-side
/// stack trace.
///
/// Idempotent in the sense that calling twice is a programming
/// error — both backends will panic on a second init. Should
/// fire once at program start.
pub fn init() {
    #[cfg(not(target_arch = "wasm32"))]
    {
        env_logger::init();
    }

    #[cfg(target_arch = "wasm32")]
    {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));
        console_log::init_with_level(log::Level::Info).expect("failed to init logger");
    }
}
