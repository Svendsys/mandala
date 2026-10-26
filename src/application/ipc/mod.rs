// SPDX-License-Identifier: MPL-2.0

//! Local inter-process control: the socket an external process — an
//! LLM agent, a test harness, `mandalactl` — drives the running app
//! through. The wire contract is `format/ipc.md`; the design and its
//! rejected alternatives are `work_plans/LLM_IPC.md`.
//!
//! **The boundary contract** (CODE_CONVENTIONS §3, "Sanctioned
//! boundary threads"). Opt-in per launch with `--ipc <path>`; without
//! the flag nothing in this module runs. With it, dedicated threads
//! own the socket and move *protocol values* — decoded requests,
//! frames to encode — across `std::sync::mpsc`; they never see
//! `InitState`, the document or the renderer. Every command executes
//! on the main thread, between winit events, with the same access an
//! input handler has, so IPC never races the user and a command's
//! effects are ordered with theirs. The threads wake the parked event
//! loop through a winit user event ([`IpcWake`]), which is what lets
//! an idle app — parked in `ControlFlow::Wait`, the state an agent
//! most often finds it in — answer with no polling and no idle cost.
//!
//! Native-only at this module boundary, the `console_input`
//! precedent: browsers cannot serve local sockets (`CLAUDE.md`
//! "Dual-target status"). Within native, the transport is a Unix
//! socket, or a named pipe on Windows (#169).

mod commands;
mod envelope;
pub(crate) mod events;
pub(crate) mod injected;
mod registry;
pub(crate) mod space;
mod target;
mod transport;
mod wait;

pub(crate) use envelope::PROTOCOL_VERSION;
pub(crate) use registry::execute;
// The decoder-to-wire test runner, for the app's GPU-backed tests
// (`app::tests_gpu`): an `act.*` request run against a real app the
// way the socket would run it, minus the socket.
#[cfg(test)]
pub(crate) use registry::tests::run_attached;
pub(crate) use target::{ActionOutcome, CommandTarget, Injected, SceneView, Screenshot, ShellState};
pub(crate) use transport::IpcServer;
pub(crate) use wait::{Settling, Until};

/// The winit user event the boundary threads send to wake the event
/// loop. It carries nothing: the work is on the server's ordered
/// queue, and a wake only says "drain it". Several wakes for one
/// drain are harmless — an empty drain is a no-op.
#[derive(Debug, Clone, Copy)]
pub(crate) struct IpcWake;

/// What executing one request produced.
pub(crate) struct Executed {
    /// The reply, or `None` for one held back: a `clock.wait` whose
    /// reply the transport sends when it resolves (`wait`).
    pub(crate) reply: Option<envelope::Reply>,
    /// The command changed something on screen. The event loop then
    /// requests a redraw, exactly as a winit input handler does after
    /// a mutating event — `about_to_wait`'s continuation check alone
    /// does not schedule a present for a one-shot change.
    pub(crate) repaint: bool,
}

/// What one `IpcServer::drain` did that the event loop must act on.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct DrainOutcome {
    /// Some executed command changed pixels; request a redraw.
    pub(crate) repaint: bool,
    /// The drain budget ran out with messages still queued; keep the
    /// loop iterating rather than parking.
    pub(crate) backlog: bool,
}
