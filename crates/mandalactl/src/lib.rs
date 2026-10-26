// SPDX-License-Identifier: MPL-2.0

//! `mandalactl`: the command-line client for a running Mandala's
//! `--ipc` socket (`format/ipc.md`).
//!
//! An agent's native tool is the shell, and speaking a socket protocol
//! from bash — `nc`, hand-built JSON, hand-escaped quotes — is the
//! fragile incantation this crate exists to remove. One invocation is
//! one connection: it greets, sends one command (and, with
//! `--wait-settled`, the wait that follows it), prints the reply and
//! exits with a code the caller can branch on (see [`run()`]).
//!
//! **No vocabulary of its own.** The commands, parameters and types
//! are the wire's. [`verbs`] is a table of the ones this build knows,
//! held to the app's registry by a test over there; a command it lacks
//! is still callable, typed from the app's own `describe`.
//!
//! **Kept apart from `maptool` on purpose.** That tool reads
//! `.mindmap.json` files and never talks to a process; this talks to a
//! process and never reads a map. Its one client-side command,
//! `record assemble <dir>` (the `assemble` module) reads a *recording's* frames
//! and manifest — never a map, and never the socket.
//!
//! **Two apps, one client.** A native app is reached over its `--ipc`
//! Unix socket, or its named pipe on Windows (#169); a browser app over
//! a WebSocket, through the relay this crate also is — `mandalactl
//! bridge` (the `bridge` module), which a page opened with `?ipc=<its
//! app URL>` connects to and `--ws <its controller URL>` drives
//! (`format/ipc.md` §Transport "Browser"). The commands are the same;
//! what differs is written in `--help`. The crate compiles for
//! `wasm32-unknown-unknown` too (the workspace is type-checked for it)
//! with every transport and the relay stubbed or absent, since the
//! browser is the app's side of the relay, never the controller's.
//!
//! The lints below are CODE_CONVENTIONS §9's `unwrap()` rule in its
//! editor-time form; `baumhard::util::unwrap_posture` is the gate.
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod assemble;
#[cfg(not(target_arch = "wasm32"))]
mod bridge;
mod cli;
// The frame reader, the greeting check and the connection's constants
// are read by the native clients — the Unix socket, the Windows pipe
// and the WebSocket. Elsewhere (the wasm32 type-check) the modules
// compile with nothing draining them, scoped so the native lint stays
// fully armed.
#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
mod client;
mod help;
mod params;
mod render;
mod run;
pub mod verbs;
#[cfg_attr(not(any(unix, windows)), allow(dead_code))]
mod wire;

#[cfg(all(test, unix))]
mod loopback_tests;
#[cfg(all(test, unix))]
mod testing;
#[cfg(test)]
mod words;
#[cfg(all(test, unix))]
mod ws_tests;

pub use client::pipe_path;
pub use run::{run, Env, Exit, Io};
pub use wire::{redacted, BRIDGE_ATTACHED, BRIDGE_DETACHED, BRIDGE_KEY, FRAME_CAP, PROTOCOL, REQUEST_CAP};
