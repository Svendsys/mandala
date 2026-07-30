// SPDX-License-Identifier: MPL-2.0

//! Native user-tier macro loader. Reads
//! `$XDG_CONFIG_HOME/mandala/macros.json` (fallback
//! `~/.config/mandala/macros.json`) and parses through the shared
//! [`super::parse_user_macros_json`].
//!
//! The read, the size cap, and the fallback walk itself all belong
//! to [`crate::application::user_config`]; this module only names
//! the layer and the parser. The keybinds and mutations desktop
//! loaders name theirs the same way against the same driver, which
//! is also where the shared resilience posture lives: the app boots
//! with an empty user tier when the file is absent or malformed, and
//! warns on failure so the user notices.

use super::Macro;
use crate::application::user_config::{load_layered, read_capped, xdg::xdg_mandala_path, ConfigLayer};

/// Load the user-layer macros. Tier: `SourceTier::User`, assigned
/// at the call site in `run_native_init::build`.
///
/// Returns an empty `Vec` when the file is absent, oversized, or
/// malformed; failures log a warning so users notice but the app
/// still boots. Files larger than `MAX_USER_PAYLOAD_BYTES` are
/// rejected before `read_to_string` runs.
pub fn load_user_macros() -> Vec<Macro> {
    let Some(path) = xdg_mandala_path("macros.json") else {
        log::debug!("macros: no HOME / XDG_CONFIG_HOME; user macro file disabled");
        return Vec::new();
    };
    let source = path.display().to_string();
    // An absent macros.json is the normal case, so the layer
    // reports itself missing rather than surfacing a stat error.
    let fetch = || path.exists().then(|| read_capped(&path));
    let layers = [ConfigLayer {
        source: &source,
        fetch: &fetch,
    }];
    match load_layered("macros", layers, super::parse_user_macros_json) {
        Some((v, source)) => {
            if !v.is_empty() {
                log::info!("macros: loaded {} user macro(s) from {}", v.len(), source);
            }
            v
        }
        None => Vec::new(),
    }
}
