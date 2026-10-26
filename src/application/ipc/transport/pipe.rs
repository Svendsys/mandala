// SPDX-License-Identifier: MPL-2.0

//! The Windows transport (#169): a named pipe at
//! `\\.\pipe\mandala-<name>` in place of the Unix socket, behind the
//! same three boundary threads (`format/ipc.md` §Windows).
//!
//! **What the pipe guarantees, and where it comes from.** Everything a
//! socket file's mode and directory do on Unix, the pipe's creation
//! does here:
//!
//! - **Owner-only.** The pipe is created with [`SDDL`], a protected DACL
//!   holding one ACE: generic-all for OWNER RIGHTS. No Everyone-read ACE,
//!   which the default named-pipe DACL would carry. The owner is the
//!   launching user — except for an elevated launch, where Windows makes
//!   the Administrators group the owner of what the process creates, so
//!   another elevated process of a member of that group could connect
//!   too. Naming the user's own SID instead would close that, and needs
//!   a token query with no safe API anywhere in the graph; this crate has
//!   no `unsafe`.
//! - **Local-only.** Every instance is created with
//!   `PIPE_REJECT_REMOTE_CLIENTS`.
//! - **One server per name.** The first instance is created with
//!   `FILE_FLAG_FIRST_PIPE_INSTANCE`, so a second Mandala launched with
//!   the same `--ipc` value fails at startup — the analog of the live-
//!   socket refusal. There is no stale state to heal: a pipe vanishes
//!   with its server's last handle.
//!
//! **Two libraries, one pipe.** The first instance is made by
//! `interprocess` (pinned exactly; the root manifest names the source
//! files each guarantee above was read in), because it is the only safe
//! way in the graph to hand `CreateNamedPipeW` a security descriptor.
//! A pipe's descriptor belongs to the pipe, not to an instance: it is
//! set when the first instance creates the pipe, and every later
//! instance is created *under* it — which takes create-instance access
//! the descriptor grants only to its owner. So every instance after
//! the first is a tokio [`NamedPipeServer`], made by `bind` before the
//! `interprocess` one is let go, so the pipe never has zero handles and
//! its name is never free for someone else to take. The one moment
//! that costs: a client that connects in the instant between the two
//! is disconnected before its `hello`, and sees the app close the
//! connection; the server is not listening yet by its own account
//! (`ipc: listening on …` is logged after).
//!
//! **Why tokio, and why its instances.** A pipe has no `SO_SNDTIMEO`,
//! and a blocking pipe write cannot be canceled without `unsafe`. The
//! stalled-write rule needs both. A tokio instance gives the timeout
//! (an overlapped write raced against a timer) and the cancellation:
//! [`NamedPipeServer::disconnect`], which fails the server's pending
//! write and the client's alike. Dropping the instance alone does not
//! — tokio's driver cancels a pending read on drop but lets a pending
//! write run to completion, so a client that stopped reading would keep
//! a dropped instance connected, and itself blocked on its own write,
//! forever. Every path that gives up on a client therefore disconnects
//! it before letting go.
//!
//! The thread shape does not change: the acceptor, each reader and each
//! outbound writer are still std threads, each blocking on one call at
//! a time (`Runtime::block_on` on a one-worker runtime), so every
//! boundary thread reads the same as on Unix and the readiness loop
//! `work_plans/LLM_IPC.md` §D2 rejected is not reintroduced.
//!
//! **Every wait ends.** The acceptor's wait for a client ends when the
//! server closes its endpoint ([`BoundPipe::close`], also on drop). A
//! write ends after [`STALLED_WRITE_TIMEOUT`] without progress, or at a
//! hang-up. A read waits on the client for as long as it stays — an
//! idle controller is a legal one — and ends at a hang-up, which the
//! server sends on every exit, graceful or not. A refusal ends after
//! [`REJECT_GRACE`].
//!
//! **Closing.** A connection that ended in order (after the `shutdown`
//! event, or a client that left) is closed by dropping the instance:
//! what the client has not read yet stays readable, as on a socket
//! after `shutdown`. One that failed — a stalled or broken write, or a
//! hang-up — is disconnected first, which discards it.

use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use interprocess::os::windows::named_pipe::{pipe_mode, PipeListenerOptions};
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};
use tokio::runtime::Runtime;
use tokio::sync::watch;

use super::connection::{Conn, HangUp, Listen, Outlet};
use super::pipe_name::pipe_path;
use super::STALLED_WRITE_TIMEOUT;

/// The pipe's security descriptor, in SDDL: a protected DACL (`P`, so
/// nothing is inherited) with one ACE granting generic-all to OWNER
/// RIGHTS (`OW`). See the module doc for who the owner is.
pub(super) const SDDL: &str = "D:P(A;;GA;;;OW)";

/// How long a refused client is given to take its `connection_rejected`
/// frame before the instance is disconnected anyway. The refusal runs
/// on the runtime, not on the acceptor, so this bounds a task, never
/// the next accept.
const REJECT_GRACE: Duration = Duration::from_secs(1);

/// The pipe this process serves. Holding it keeps the I/O runtime alive
/// for as long as the server is; closing it, or dropping it, stops the
/// acceptor.
pub(super) struct BoundPipe {
    path: PathBuf,
    stop: watch::Sender<bool>,
    _runtime: Arc<Runtime>,
}

impl BoundPipe {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Stop the acceptor: its wait for the next client ends, and the
    /// waiting instance closes with it. There is no file to remove — a
    /// pipe vanishes with its last handle, which a connection still
    /// being retired may hold a moment longer.
    pub(super) fn close(&self) {
        self.stop.send_replace(true);
    }
}

impl Drop for BoundPipe {
    fn drop(&mut self) {
        self.close();
    }
}

/// Create the pipe `--ipc <name>` names, owner-only, and the runtime
/// its I/O runs on.
///
/// Every failure is a message naming the pipe and the remedy, for the
/// one startup `expect` (CODE_CONVENTIONS §9).
pub(super) fn bind(name: &Path) -> Result<(PipeAcceptor, BoundPipe), String> {
    let name = name
        .to_str()
        .ok_or_else(|| format!("`--ipc {}`: a pipe name must be valid Unicode", name.display()))?;
    let path = pipe_path(name)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(1)
        .thread_name("mandala-ipc-io")
        .enable_all()
        .build()
        .map(Arc::new)
        .map_err(|e| format!("cannot start the IPC I/O runtime for {path}: {e}"))?;
    let descriptor = owner_only()
        .map_err(|e| format!("cannot build the owner-only security descriptor for {path}: {e}"))?;
    // Creates the pipe, with the descriptor and the first-instance flag.
    let first = PipeListenerOptions::new()
        .path(path.as_str())
        .security_descriptor(Some(descriptor))
        .create_duplex::<pipe_mode::Bytes>()
        .map_err(|e| create_error(&path, &e))?;
    // The instance the acceptor waits on, created under that descriptor
    // while `first` still holds the pipe open.
    let listening = create_instance(&runtime, &path)
        .map_err(|e| format!("cannot create a waiting instance of the pipe {path}: {e}"))?;
    drop(first);
    let (stop, stopped) = watch::channel(false);
    let acceptor = PipeAcceptor {
        runtime: Arc::clone(&runtime),
        path: path.clone(),
        listening,
        stopped,
    };
    Ok((
        acceptor,
        BoundPipe {
            path: PathBuf::from(path),
            stop,
            _runtime: runtime,
        },
    ))
}

/// The security descriptor [`SDDL`] spells.
fn owner_only() -> io::Result<SecurityDescriptor> {
    let sddl = widestring::U16CString::from_str(SDDL)
        .map_err(|e| io::Error::new(ErrorKind::InvalidInput, e.to_string()))?;
    SecurityDescriptor::deserialize(&sddl)
}

/// One more instance of the pipe at `path`, which already exists: not
/// the first, local clients only, byte mode — the parameters the first
/// was made with. It inherits the pipe's descriptor.
fn create_instance(runtime: &Runtime, path: &str) -> io::Result<NamedPipeServer> {
    // Tokio pipe objects register with the runtime they are made in.
    let _context = runtime.enter();
    ServerOptions::new()
        .first_pipe_instance(false)
        .reject_remote_clients(true)
        .create(path)
}

/// Why the first instance could not be made, in words that name the
/// next step.
fn create_error(path: &str, error: &io::Error) -> String {
    match error.kind() {
        // `FILE_FLAG_FIRST_PIPE_INSTANCE` on a name that already exists.
        ErrorKind::PermissionDenied => format!(
            "{path} is already being served: another Mandala was launched with the same --ipc name, or another \
             program owns that pipe. Pick another name, or close the other one"
        ),
        _ => format!("cannot create the pipe {path}: {error}"),
    }
}

/// The acceptor's end of the pipe: one instance waiting for a client.
/// Each accept hands that instance over and leaves a fresh one waiting.
pub(super) struct PipeAcceptor {
    runtime: Arc<Runtime>,
    path: String,
    listening: NamedPipeServer,
    /// Flips (or closes) when the server closes its endpoint.
    stopped: watch::Receiver<bool>,
}

impl Listen for PipeAcceptor {
    type Conn = PipeConn;

    fn accept(&mut self) -> io::Result<Option<PipeConn>> {
        let PipeAcceptor {
            runtime,
            listening,
            stopped,
            ..
        } = self;
        let connected = runtime.block_on(async {
            tokio::select! {
                biased;
                // Closed, or the server dropped without closing.
                _ = stopped.wait_for(|stop| *stop) => None,
                connected = listening.connect() => Some(connected),
            }
        });
        let Some(connected) = connected else {
            return Ok(None);
        };
        // The next instance is made before this one is handed over, so
        // the pipe is never without a waiting instance or a handle.
        let next = create_instance(&self.runtime, &self.path);
        if let Err(e) = connected {
            // A client that came and went before the connect completed.
            // The instance is replaced rather than reused: a fresh one
            // has no state left over from the failed connect.
            match next {
                Ok(next) => self.listening = next,
                Err(_) => {
                    let _ = self.listening.disconnect();
                }
            }
            return Err(e);
        }
        match next {
            Ok(next) => {
                let pipe = std::mem::replace(&mut self.listening, next);
                Ok(Some(PipeConn {
                    runtime: Arc::clone(&self.runtime),
                    pipe,
                }))
            }
            Err(e) => {
                // No instance to hand over and keep one waiting: turn
                // this client away and wait on its instance again.
                let _ = self.listening.disconnect();
                Err(e)
            }
        }
    }
}

/// One connected pipe instance, not yet split.
pub(super) struct PipeConn {
    runtime: Arc<Runtime>,
    pipe: NamedPipeServer,
}

/// What a connection's three handles share.
struct Shared {
    runtime: Arc<Runtime>,
    pipe: NamedPipeServer,
    /// Set when the connection is to end: by a hang-up, or by the
    /// writer as it closes. The reader returns end-of-file on it, and a
    /// write in progress gives up.
    hung: watch::Sender<bool>,
}

impl Conn for PipeConn {
    type Reader = PipeReader;
    type Writer = PipeWriter;
    type Hangup = PipeHangup;

    fn into_controller(self) -> io::Result<(PipeReader, PipeWriter, PipeHangup)> {
        let (hung, _) = watch::channel(false);
        let shared = Arc::new(Shared {
            runtime: self.runtime,
            pipe: self.pipe,
            hung,
        });
        Ok((
            PipeReader {
                hung: shared.hung.subscribe(),
                shared: Arc::clone(&shared),
            },
            PipeWriter {
                hung: shared.hung.subscribe(),
                shared: Arc::clone(&shared),
                failed: false,
            },
            PipeHangup(shared),
        ))
    }

    fn reject(self, frame: &[u8]) {
        let PipeConn { runtime, pipe } = self;
        let frame = frame.to_vec();
        // On the runtime rather than here, so a refused client that
        // never reads cannot hold up the next accept.
        runtime.spawn(async move {
            let offered = tokio::time::timeout(REJECT_GRACE, write_all(&pipe, &frame)).await;
            if !matches!(offered, Ok(Ok(()))) {
                let _ = pipe.disconnect();
            }
            // Dropped: the frame, if it went, stays readable.
        });
    }
}

/// Wait until the pipe takes some of `bytes`, and say how many.
async fn write_some(pipe: &NamedPipeServer, bytes: &[u8]) -> io::Result<usize> {
    loop {
        pipe.writable().await?;
        match pipe.try_write(bytes) {
            Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
            written => return written,
        }
    }
}

/// Write all of `bytes`, however many steps it takes.
async fn write_all(pipe: &NamedPipeServer, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        match write_some(pipe, bytes).await? {
            0 => return Err(io::Error::from(ErrorKind::WriteZero)),
            n => bytes = &bytes[n..],
        }
    }
    Ok(())
}

/// The read half: a blocking [`io::Read`] over the pipe that a hang-up
/// ends as end-of-file.
pub(super) struct PipeReader {
    shared: Arc<Shared>,
    hung: watch::Receiver<bool>,
}

impl io::Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let PipeReader { shared, hung } = self;
        shared.runtime.block_on(async {
            tokio::select! {
                biased;
                // A hang-up reads as EOF: the reader then retires the
                // controller as on a client disconnect.
                _ = hung.wait_for(|hung| *hung) => Ok(0),
                read = async {
                    loop {
                        shared.pipe.readable().await?;
                        match shared.pipe.try_read(buf) {
                            Err(e) if e.kind() == ErrorKind::WouldBlock => continue,
                            // A client that closed its end reads as EOF
                            // here too — tokio turns the broken pipe into
                            // `Ok(0)` — so a final unterminated line is
                            // still executed.
                            read => return read,
                        }
                    }
                } => read,
            }
        })
    }
}

/// The write half: whole writes that fail once one makes no progress
/// for [`STALLED_WRITE_TIMEOUT`] or the connection is hung up.
pub(super) struct PipeWriter {
    shared: Arc<Shared>,
    hung: watch::Receiver<bool>,
    /// A write failed or stalled: the client is gone or not reading.
    failed: bool,
}

impl Outlet for PipeWriter {
    fn send_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        let PipeWriter { shared, hung, failed } = self;
        let pipe = &shared.pipe;
        let sent = shared.runtime.block_on(async {
            let mut written = 0;
            while written < bytes.len() {
                // One write per step, each with the full timeout: the
                // rule is "no progress for this long", as `SO_SNDTIMEO`
                // makes it on Unix, not a deadline for the whole batch.
                let step = tokio::select! {
                    biased;
                    _ = hung.wait_for(|hung| *hung) => {
                        Err(io::Error::new(ErrorKind::ConnectionAborted, "the connection was hung up"))
                    }
                    step = tokio::time::timeout(STALLED_WRITE_TIMEOUT, write_some(pipe, &bytes[written..])) => match step {
                        Err(_elapsed) => Err(io::Error::new(
                            ErrorKind::TimedOut,
                            "the client made no room for output within the stalled-write timeout",
                        )),
                        Ok(Ok(0)) => Err(io::Error::from(ErrorKind::WriteZero)),
                        Ok(step) => step,
                    },
                };
                written += step?;
            }
            Ok(())
        });
        if sent.is_err() {
            *failed = true;
        }
        sent
    }

    fn close(self) {
        let PipeWriter { shared, hung, failed } = self;
        // A failed write, or a hang-up: nothing more is owed to this
        // client, and a write still pending would keep the instance —
        // and the client blocked on its own writes — alive forever.
        if failed || *hung.borrow() {
            let _ = shared.pipe.disconnect();
        }
        // Ends the reader's pending read, so the instance closes and the
        // controller is retired — what `shutdown(Both)` does on Unix.
        shared.hung.send_replace(true);
    }
}

/// The main thread's handle for ending a connection: both halves see it
/// and finish, the writer disconnecting the client on its way out.
pub(super) struct PipeHangup(Arc<Shared>);

impl HangUp for PipeHangup {
    fn hang_up(&self) {
        self.0.hung.send_replace(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The descriptor text the pipe is created with, pinned, and
    /// accepted by Windows. The failing input is an edit that widens it:
    /// dropping `P` would let the pipe inherit ACEs, and any added ACE
    /// would let someone besides the owner in.
    ///
    /// This holds the text, not its application: deleting
    /// `.security_descriptor(Some(descriptor))` from `bind` passes it,
    /// and so does the same-user round trip. Reading the descriptor
    /// back off the live pipe (`GetSecurityInfo`) has no safe API in
    /// the graph, so that line is held by review, not by a test.
    #[test]
    fn test_the_pipe_is_created_owner_only() {
        assert_eq!(SDDL, "D:P(A;;GA;;;OW)");
        owner_only().expect("Windows accepts the descriptor");
    }

    /// A second server under a name already served is refused at
    /// startup, naming the pipe — the analog of the live-socket refusal.
    /// It still is once the server's first instance is gone and only
    /// its later ones hold the pipe: the failing input is a `bind` that
    /// let the name lapse between the two.
    #[test]
    fn test_a_second_server_under_the_same_name_is_refused() {
        let name = super::super::tests::unique_endpoint("ipc-pipe-twice");
        let (_first, bound) = bind(&name).expect("the first server binds");
        let Err(refused) = bind(&name) else {
            panic!("a second server under the same name was allowed");
        };
        assert!(
            refused.contains(&bound.path().display().to_string()) && refused.contains("already being served"),
            "{refused}"
        );
    }

    /// Closing the endpoint ends the acceptor's wait. The failing input
    /// is an accept that waits on the client alone: it would never
    /// return here, and the test fails on its deadline instead.
    #[test]
    fn test_closing_the_endpoint_ends_the_acceptors_wait() {
        let name = super::super::tests::unique_endpoint("ipc-pipe-close");
        let (mut acceptor, bound) = bind(&name).expect("binds");
        let (done_tx, done) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = done_tx.send(acceptor.accept().map(|conn| conn.is_none()));
        });
        bound.close();
        let stopped = done
            .recv_timeout(Duration::from_secs(10))
            .expect("the acceptor returned after the endpoint closed");
        assert!(matches!(stopped, Ok(true)), "a closed endpoint accepts nothing: {stopped:?}");
    }
}
