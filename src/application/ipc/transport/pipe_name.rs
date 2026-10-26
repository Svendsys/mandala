// SPDX-License-Identifier: MPL-2.0

//! The Windows pipe name an `--ipc` value stands for
//! (`format/ipc.md` §Windows): `--ipc <name>` serves
//! `\\.\pipe\mandala-<name>`.
//!
//! A plain string function, compiled on every native target rather than
//! only on Windows, for one reason: `mandalactl` maps the same value the
//! same way to find the pipe, and a test here holds the two mappings
//! against each other on the platform CI runs everywhere. Two spellings
//! of one rule with nothing comparing them would drift the first time
//! either changed.

/// What every pipe this transport serves is named under. The prefix
/// keeps `--ipc mandala` from claiming a pipe another program owns.
pub(crate) const PIPE_PREFIX: &str = r"\\.\pipe\mandala-";

/// The longest pipe name Windows accepts after `\\.\pipe\`, in UTF-16
/// code units (`CreateNamedPipeW`: "can include any character other
/// than a backslash, including numbers and special characters. The
/// entire pipe name string can be up to 256 characters long").
const PIPE_NAME_MAX: usize = 256;

/// The full pipe path for an `--ipc` value, or the reason there is
/// none, worded for the startup error it becomes.
pub(crate) fn pipe_path(name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("`--ipc` needs a pipe name on Windows, e.g. --ipc mandala (served as \\\\.\\pipe\\mandala-mandala)".to_string());
    }
    if name.contains('\0') {
        return Err(format!(
            "`--ipc {name:?}`: a pipe name cannot contain a NUL character"
        ));
    }
    if name.contains('\\') {
        return Err(format!(
            "`--ipc {name}`: on Windows the value names a pipe, not a path, and a pipe name cannot contain a backslash"
        ));
    }
    let path = format!("{PIPE_PREFIX}{name}");
    let after_root = path.trim_start_matches(r"\\.\pipe\").encode_utf16().count();
    if after_root > PIPE_NAME_MAX {
        return Err(format!(
            "`--ipc {name}`: the pipe name `mandala-{name}` is {after_root} UTF-16 units; Windows allows {PIPE_NAME_MAX}"
        ));
    }
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pipe_path_prefixes_the_name() {
        assert_eq!(pipe_path("mandala").as_deref(), Ok(r"\\.\pipe\mandala-mandala"));
        assert_eq!(pipe_path("a b.c").as_deref(), Ok(r"\\.\pipe\mandala-a b.c"));
    }

    #[test]
    fn test_pipe_path_refuses_what_windows_would() {
        assert!(pipe_path("").unwrap_err().contains("needs a pipe name"));
        assert!(pipe_path(r"a\b").unwrap_err().contains("backslash"));
        assert!(pipe_path("a\0b").unwrap_err().contains("NUL"));
        // `mandala-` is 8 units, so 248 more is exactly the limit.
        assert!(pipe_path(&"x".repeat(248)).is_ok());
        assert!(pipe_path(&"x".repeat(249)).unwrap_err().contains("allows 256"));
    }

    /// The client finds the pipe by the same rule the server names it
    /// by. The failing input is either side changing its prefix or its
    /// refusals alone — the client would then look for a pipe the server
    /// never made.
    #[test]
    fn test_mandalactl_maps_an_ipc_value_to_the_same_pipe() {
        let long = "x".repeat(249);
        for name in ["mandala", "a b.c", "", r"a\b", "a\0b", "x", long.as_str()] {
            assert_eq!(
                pipe_path(name).is_ok(),
                mandalactl::pipe_path(name).is_ok(),
                "the two sides disagree on whether `{name}` names a pipe"
            );
            if let (Ok(server), Ok(client)) = (pipe_path(name), mandalactl::pipe_path(name)) {
                assert_eq!(server, client, "`--ipc {name}`");
            }
        }
    }
}
