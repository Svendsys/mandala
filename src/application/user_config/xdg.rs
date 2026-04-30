// SPDX-License-Identifier: MPL-2.0

//! Resolve the canonical user-config path for a given filename
//! under `mandala/`. `$XDG_CONFIG_HOME` if set, else
//! `$HOME/.config`. Returns `None` only in degenerate environments
//! where neither variable is set — the caller treats that as
//! "no user config available, fall back to defaults."

use std::path::PathBuf;

/// `$XDG_CONFIG_HOME/mandala/<filename>` if `XDG_CONFIG_HOME` is set
/// and non-empty; else `$HOME/.config/mandala/<filename>` if `HOME`
/// is set and non-empty; else `None`. The `mandala` directory is the
/// project's XDG namespace. Callers pass the leaf filename (e.g.
/// `"keybinds.json"`); this helper does the directory-joining.
///
/// O(1) modulo the env-lookup syscalls; allocates the returned
/// `PathBuf` only on the success branch.
pub fn xdg_mandala_path(filename: &str) -> Option<PathBuf> {
    if let Ok(xdg) = std::env::var("XDG_CONFIG_HOME") {
        if !xdg.is_empty() {
            return Some(PathBuf::from(xdg).join("mandala").join(filename));
        }
    }
    let home = std::env::var("HOME").ok().filter(|s| !s.is_empty())?;
    Some(PathBuf::from(home).join(".config").join("mandala").join(filename))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Process-wide mutex serialising tests that mutate
    /// `std::env::*`. Cargo runs unit tests on multiple threads
    /// by default (one per logical core); env vars are global
    /// per-process, so two `with_env` calls on different threads
    /// would race on `XDG_CONFIG_HOME` / `HOME`. The mutex makes
    /// the save → set → body → restore dance atomic across the
    /// suite. Poisoning is tolerated — if a previous test
    /// panicked while holding the guard, we take the lock anyway
    /// (env state may be slightly off but we'd rather run than
    /// deadlock).
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    /// Run a closure with `XDG_CONFIG_HOME` and `HOME` overridden,
    /// then restored. Holds [`ENV_LOCK`] for the duration so
    /// concurrent env-touching tests don't observe each other's
    /// mid-mutation state.
    fn with_env<F: FnOnce()>(xdg: Option<&str>, home: Option<&str>, body: F) {
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

    #[test]
    fn xdg_wins_over_home_when_both_set() {
        with_env(Some("/tmp/xdg"), Some("/tmp/home"), || {
            let p = xdg_mandala_path("keybinds.json").unwrap();
            assert_eq!(p, PathBuf::from("/tmp/xdg/mandala/keybinds.json"));
        });
    }

    #[test]
    fn empty_xdg_falls_through_to_home() {
        with_env(Some(""), Some("/tmp/home"), || {
            let p = xdg_mandala_path("mutations.json").unwrap();
            assert_eq!(
                p,
                PathBuf::from("/tmp/home/.config/mandala/mutations.json"),
            );
        });
    }

    #[test]
    fn neither_set_returns_none() {
        with_env(None, None, || {
            assert!(xdg_mandala_path("macros.json").is_none());
        });
    }

    #[test]
    fn empty_home_with_unset_xdg_returns_none() {
        with_env(None, Some(""), || {
            assert!(xdg_mandala_path("macros.json").is_none());
        });
    }
}
