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
//! Native only in effect: the transport is a Unix socket, or the app's
//! named pipe on Windows (#169), and the crate compiles elsewhere (the
//! workspace is type-checked for `wasm32-unknown-unknown`) with the
//! connection stubbed to fail naming the gap.
//!
//! The lints below are CODE_CONVENTIONS §9's `unwrap()` rule in its
//! editor-time form; `baumhard::util::unwrap_posture` is the gate.
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

mod assemble;
mod cli;
// The frame reader, the greeting check and the connection's constants
// are read only by the Unix and Windows clients. Elsewhere (the wasm32
// type-check) the modules compile with nothing draining them, scoped so
// the native lint stays fully armed.
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

pub use client::pipe_path;
pub use run::{run, Exit, Io};
pub use wire::{FRAME_CAP, PROTOCOL, REQUEST_CAP};
