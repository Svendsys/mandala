// SPDX-License-Identifier: MPL-2.0

//! The Windows transport (#169): a named pipe at
//! `\\.\pipe\mandala-<name>` in place of the Unix socket, behind the
//! same three boundary threads (`format/ipc.md` §Windows).
//!
//! **What the pipe guarantees, and where it comes from.** Everything a
//! socket file's mode and directory do on Unix, the pipe's creation
//! does here, through `interprocess` (pinned exactly; the root manifest
//! names the source files each guarantee was read in):
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
//! **Why tokio here and not on Unix.** A pipe has no `SO_SNDTIMEO`, and
//! a blocking pipe write cannot be cancelled without `unsafe`. The
//! stalled-write rule needs both, so the pipe's I/O runs on a one-worker
//! tokio runtime where dropping a timed-out write future cancels the
//! overlapped write. The thread shape does not change: the acceptor,
//! each reader and each outbound writer are still std threads, each
//! blocking on one call at a time (`Runtime::block_on`), so every
//! boundary thread reads the same as on Unix and the readiness loop
//! `work_plans/LLM_IPC.md` §D2 rejected is not reintroduced.
//!
//! **Closing.** Closing a pipe handle discards what the client has not
//! read yet — the opposite of a socket, whose kernel buffer still
//! drains after `shutdown`. So the write half flushes before it closes
//! when the connection ended in order (after the `shutdown` event or a
//! `connection_rejected`), bounded by the stalled-write timeout and by
//! a hang-up, and skips the flush when the connection failed.

use std::io::{self, ErrorKind};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use interprocess::os::windows::named_pipe::tokio::{
    DuplexPipeStream, PipeListener, RecvPipeStream, SendPipeStream,
};
use interprocess::os::windows::named_pipe::{pipe_mode, PipeListenerOptions};
use interprocess::os::windows::security_descriptor::SecurityDescriptor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
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
/// frame before the instance is closed anyway. The refusal runs on the
/// runtime, not on the acceptor, so this bounds a task, never the next
/// accept.
const REJECT_GRACE: Duration = Duration::from_secs(1);

type Bytes = pipe_mode::Bytes;

/// The pipe this process serves. Holding it keeps the I/O runtime alive
/// for as long as the server is.
pub(super) struct BoundPipe {
    path: PathBuf,
    _runtime: Arc<Runtime>,
}

impl BoundPipe {
    pub(super) fn path(&self) -> &Path {
        &self.path
    }

    /// Nothing to remove: a pipe has no file, and vanishes with its last
    /// handle. Named for the socket's counterpart so a graceful exit is
    /// the same line on both platforms.
    pub(super) fn unlink(&self) {}
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
    let listener = {
        // Tokio pipe objects register with the runtime they are made in.
        let _context = runtime.enter();
        PipeListenerOptions::new()
            .path(path.as_str())
            .security_descriptor(Some(descriptor))
            .create_tokio_duplex::<Bytes>()
    }
    .map_err(|e| create_error(&path, &e))?;
    let acceptor = PipeAcceptor {
        runtime: Arc::clone(&runtime),
        listener,
    };
    Ok((
        acceptor,
        BoundPipe {
            path: PathBuf::from(path),
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

/// The acceptor's end of the pipe: each accept hands over the instance
/// a client connected to and leaves a fresh one waiting.
pub(super) struct PipeAcceptor {
    runtime: Arc<Runtime>,
    listener: PipeListener<Bytes, Bytes>,
}

impl Listen for PipeAcceptor {
    type Conn = PipeConn;

    fn accept(&self) -> io::Result<PipeConn> {
        let stream = self.runtime.block_on(self.listener.accept())?;
        Ok(PipeConn {
            runtime: Arc::clone(&self.runtime),
            stream,
        })
    }
}

/// One connected pipe instance, not yet split.
pub(super) struct PipeConn {
    runtime: Arc<Runtime>,
    stream: DuplexPipeStream<Bytes>,
}

impl Conn for PipeConn {
    type Reader = PipeReader;
    type Writer = PipeWriter;
    type Hangup = PipeHangup;

    fn into_controller(self) -> io::Result<(PipeReader, PipeWriter, PipeHangup)> {
        let (recv, send) = self.stream.split();
        let (hung, hung_rx) = watch::channel(false);
        let hung = Arc::new(hung);
        Ok((
            PipeReader {
                runtime: Arc::clone(&self.runtime),
                recv,
                hung: hung_rx.clone(),
            },
            PipeWriter {
                runtime: self.runtime,
                send,
                hang_up: Arc::clone(&hung),
                hung: hung_rx,
                failed: false,
            },
            PipeHangup(hung),
        ))
    }

    fn reject(self, frame: &[u8]) {
        let PipeConn { runtime, stream } = self;
        let frame = frame.to_vec();
        // On the runtime rather than here, so a refused client that
        // never reads cannot hold up the next accept.
        runtime.spawn(async move {
            let _ = tokio::time::timeout(REJECT_GRACE, async {
                (&stream).write_all(&frame).await?;
                stream.flush().await
            })
            .await;
            // Flushed, or given up on; either way not handed to a
            // background flush that could wait on this client forever.
            stream.assume_flushed();
        });
    }
}

/// The read half: a blocking [`io::Read`] over the pipe that a hang-up
/// ends as end-of-file.
pub(super) struct PipeReader {
    runtime: Arc<Runtime>,
    recv: RecvPipeStream<Bytes>,
    hung: watch::Receiver<bool>,
}

impl io::Read for PipeReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let PipeReader { runtime, recv, hung } = self;
        runtime.block_on(async {
            tokio::select! {
                biased;
                // A hang-up (or every hang-up handle gone) reads as EOF:
                // the reader then retires the controller as on a
                // client disconnect.
                _ = hung.wait_for(|hung| *hung) => Ok(0),
                // A client that closes its end reads as EOF here too —
                // `interprocess` turns the broken pipe into `Ok(0)` —
                // so a final unterminated line is still executed.
                read = recv.read(buf) => read,
            }
        })
    }
}

/// The write half: whole writes that fail once one makes no progress
/// for [`STALLED_WRITE_TIMEOUT`], and a close that flushes first when
/// nothing failed.
pub(super) struct PipeWriter {
    runtime: Arc<Runtime>,
    send: SendPipeStream<Bytes>,
    hang_up: Arc<watch::Sender<bool>>,
    hung: watch::Receiver<bool>,
    /// A write failed or stalled: the client is gone or not reading, so
    /// closing must not wait on it.
    failed: bool,
}

impl Outlet for PipeWriter {
    fn send_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        let PipeWriter {
            runtime,
            send,
            hung,
            failed,
            ..
        } = self;
        let sent = runtime.block_on(async {
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
                    step = tokio::time::timeout(STALLED_WRITE_TIMEOUT, send.write(&bytes[written..])) => match step {
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
        let PipeWriter {
            runtime,
            send,
            hang_up,
            mut hung,
            failed,
        } = self;
        if !failed && !*hung.borrow() {
            runtime.block_on(async {
                tokio::select! {
                    biased;
                    _ = hung.wait_for(|hung| *hung) => {}
                    _ = tokio::time::timeout(STALLED_WRITE_TIMEOUT, send.flush()) => {}
                }
            });
        }
        // Not handed to `interprocess`'s background flush: whatever was
        // owed has been flushed above or given up on.
        send.assume_flushed();
        // Ends the reader's pending read, so the instance closes and the
        // controller is retired — what `shutdown(Both)` does on Unix.
        hang_up.send_replace(true);
    }
}

/// The main thread's handle for ending a connection: both halves see it
/// and finish, and the instance closes with them.
pub(super) struct PipeHangup(Arc<watch::Sender<bool>>);

impl HangUp for PipeHangup {
    fn hang_up(&self) {
        self.0.send_replace(true);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The descriptor text the pipe is created with, pinned, and
    /// accepted by Windows. The failing input is an edit that widens it:
    /// dropping `P` would let the pipe inherit ACEs, and any added ACE
    /// would let someone besides the owner in.
    #[test]
    fn test_the_pipe_is_created_owner_only() {
        assert_eq!(SDDL, "D:P(A;;GA;;;OW)");
        owner_only().expect("Windows accepts the descriptor");
    }

    /// A second server under a name already served is refused at
    /// startup, naming the pipe — the analog of the live-socket refusal.
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
}
