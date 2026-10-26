// SPDX-License-Identifier: MPL-2.0

//! Test fixtures that a user spells differently on each platform: a
//! working directory, an absolute path, what a relative one becomes.

/// `unix` on every platform but Windows, where it is `windows`.
///
/// Both spellings stay literals at the call site, rather than being
/// built with the `Path::join` the code under test uses: a test then
/// pins what a user on each platform actually sees, and a join that
/// went wrong could not agree with itself.
pub fn native(unix: &'static str, windows: &'static str) -> &'static str {
    if cfg!(windows) {
        windows
    } else {
        unix
    }
}
