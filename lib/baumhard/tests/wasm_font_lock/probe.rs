// SPDX-License-Identifier: MPL-2.0

//! The wasm32 side of `tests/wasm_font_lock.rs`: a `cdylib` that
//! exports one function per scenario, each calling baumhard's real
//! font-lock helpers, so a node runner can execute them in a real
//! WebAssembly engine.
//!
//! This file is not part of any cargo target in the repository. The
//! test copies it into a staging crate under `target/` and builds it
//! there, against the workspace's own `Cargo.lock`, so it compiles the
//! dependency graph the app ships rather than a second one that could
//! drift.
//!
//! Every export returns `1` on success. The `reentrant_*` exports are
//! *meant* to panic: on `wasm32-unknown-unknown` a panic aborts as a
//! trap, which says nothing about its cause, so the hook below hands
//! the message to the host first. The runner decides pass or fail from
//! that message, not from the trap: a trap on its own is exactly what
//! `Instant::now()` produces too, so only the message tells a
//! deliberate panic from a bug.

use std::sync::Once;

use baumhard::font::fonts::{acquire_font_system_read, acquire_font_system_write, try_acquire_font_system_write};

// The explicit module is what makes this an import. Without it the
// symbol is merely undefined, and current rustc no longer links
// wasm32-unknown-unknown with `--allow-undefined` to paper over that.
#[link(wasm_import_module = "env")]
extern "C" {
    /// Host import: receives each panic's message.
    fn host_panic(ptr: *const u8, len: usize);
}

fn install_hook() {
    static HOOK: Once = Once::new();
    HOOK.call_once(|| {
        std::panic::set_hook(Box::new(|info| {
            let message = info.to_string();
            // SAFETY: the host only reads `len` bytes at `ptr` during
            // the call, and `message` outlives it.
            unsafe { host_panic(message.as_ptr(), message.len()) };
        }));
    });
}

/// Take the write guard, use it, release it, take it again. The second
/// acquire proves the release cleared the thread's holding record, and
/// the deref proves the guard type reaches the `FontSystem`.
#[no_mangle]
pub extern "C" fn uncontended_write() -> u32 {
    install_hook();
    let first = acquire_font_system_write("probe::first");
    let _faces = first.db().len();
    drop(first);
    let second = acquire_font_system_write("probe::second");
    drop(second);
    1
}

#[no_mangle]
pub extern "C" fn uncontended_try_write() -> u32 {
    install_hook();
    u32::from(try_acquire_font_system_write("probe::try").is_some())
}

/// Two nested reads on one thread are legal, and are the case a plain
/// (non-recursive) read could deadlock behind a queued writer.
#[no_mangle]
pub extern "C" fn uncontended_read() -> u32 {
    install_hook();
    let outer = acquire_font_system_read("probe::outer");
    let inner = acquire_font_system_read("probe::inner");
    let _faces = inner.db().len();
    drop(inner);
    drop(outer);
    1
}

#[no_mangle]
pub extern "C" fn reentrant_write_write() -> u32 {
    install_hook();
    let _outer = acquire_font_system_write("probe::outer");
    let _inner = acquire_font_system_write("probe::inner");
    0
}

#[no_mangle]
pub extern "C" fn reentrant_write_try_write() -> u32 {
    install_hook();
    let _outer = acquire_font_system_write("probe::outer");
    let _inner = try_acquire_font_system_write("probe::inner");
    0
}

#[no_mangle]
pub extern "C" fn reentrant_write_read() -> u32 {
    install_hook();
    let _outer = acquire_font_system_write("probe::outer");
    let _inner = acquire_font_system_read("probe::inner");
    0
}

#[no_mangle]
pub extern "C" fn reentrant_read_write() -> u32 {
    install_hook();
    let _outer = acquire_font_system_read("probe::outer");
    let _inner = acquire_font_system_write("probe::inner");
    0
}
