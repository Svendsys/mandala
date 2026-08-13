// SPDX-License-Identifier: MPL-2.0

//! The one place the test suite overrides `XDG_CONFIG_HOME` / `HOME`.
//!
//! Environment variables are per-process, and cargo runs unit tests
//! on one thread per core, so every test that touches them has to
//! take the *same* lock — a second mutex somewhere else would
//! serialize its own callers against each other and nothing else.
//! That is why this is a module rather than a helper copied into each
//! test module that needs it.
//!
//! Two kinds of caller:
//!
//! - Tests *of* the path resolution ([`with_env`]) — `xdg.rs` sets
//!   the variables to fixed values and asserts on the path that comes
//!   out.
//! - Tests of anything that reaches a user-tier loader
//!   ([`with_no_user_config`]). `load_desktop_layered` builds up to
//!   two layers, so a test asserting that a broken explicit file
//!   falls back to the built-in defaults is really asserting that
//!   *and* that the machine running it has no
//!   `~/.config/mandala/<file>.json`. On a developer's own laptop it
//!   quietly does have one, the fallback lands on that file instead,
//!   and the test passes or fails on the contents of a file nobody
//!   wrote for it. Pointing the XDG layer at an empty scratch
//!   directory makes "the defaults" mean the defaults.
//!
//! Native-only: `wasm32` has no environment and no desktop loader to
//! isolate.

use std::sync::Mutex;

use baumhard::util::test_temp::TempDir;

/// Process-wide mutex serializing tests that mutate `std::env::*`.
///
/// Poisoning is tolerated — if a previous test panicked while holding
/// the guard the environment may be slightly off, but taking the lock
/// anyway beats deadlocking the rest of the suite behind it.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Run `body` with `XDG_CONFIG_HOME` and `HOME` overridden, then
/// restored. Holds [`ENV_LOCK`] for the duration, so concurrent
/// env-touching tests never observe each other's mid-mutation state.
///
/// `None` for either variable removes it for the duration.
///
/// Do not nest this inside itself or inside
/// [`with_no_user_config`] — [`ENV_LOCK`] is a plain `Mutex` and a
/// re-entrant acquisition deadlocks. Run the two phases one after
/// the other instead.
pub fn with_env<F: FnOnce()>(xdg: Option<&str>, home: Option<&str>, body: F) {
    let _guard = ENV_LOCK.lock().unwrap_or_else(|p| p.into_inner());
    let prev_xdg = std::env::var_os("XDG_CONFIG_HOME");
    let prev_home = std::env::var_os("HOME");
    match xdg {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    match home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
    body();
    match prev_xdg {
        Some(v) => std::env::set_var("XDG_CONFIG_HOME", v),
        None => std::env::remove_var("XDG_CONFIG_HOME"),
    }
    match prev_home {
        Some(v) => std::env::set_var("HOME", v),
        None => std::env::remove_var("HOME"),
    }
}

/// Run `body` on a machine that has no user config of any kind:
/// both `XDG_CONFIG_HOME` and `HOME` point into a fresh scratch
/// directory with no `mandala/` inside it.
///
/// Reach for this in any test that asserts a loader fell back to its
/// built-in defaults. Both variables are set rather than unset
/// because unsetting them makes `xdg_mandala_path` return `None`,
/// which skips the layer for a *different* reason than the one the
/// test is exercising — an empty directory keeps the layer built and
/// filtered on `exists()`, which is the real code path.
///
/// The scratch directory is removed when `body` returns.
pub fn with_no_user_config<F: FnOnce()>(body: F) {
    let empty = TempDir::new("no-user-config");
    let root = empty.path().display().to_string();
    with_env(Some(&root), Some(&root), body);
}
