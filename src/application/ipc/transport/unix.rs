// SPDX-License-Identifier: MPL-2.0

//! The Unix socket behind the boundary threads' seam
//! ([`super::connection`]'s traits). Every call here is the one the
//! threads made on the socket before the seam existed, moved rather than
//! changed: the write timeout is the socket's own `SO_SNDTIMEO`, the
//! halves are `try_clone`s of one stream, and ending a connection is
//! `shutdown(Both)`, which wakes a reader blocked on it.
//!
//! The one addition is the acceptor's stop: `accept()` on a listening
//! socket cannot be interrupted portably, so closing the endpoint sets
//! a flag and then connects to the socket once
//! (`socket::BoundSocket::close`). The acceptor wakes for that
//! connection, sees the flag, and ends.

use std::io::{self, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use super::connection::{Conn, HangUp, Listen, Outlet};
use super::STALLED_WRITE_TIMEOUT;

/// The listening socket and the flag that ends its acceptor.
#[derive(Debug)]
pub(super) struct SocketAcceptor {
    pub(super) listener: UnixListener,
    pub(super) stop: Arc<AtomicBool>,
}

impl Listen for SocketAcceptor {
    type Conn = UnixStream;

    fn accept(&mut self) -> io::Result<Option<UnixStream>> {
        let accepted = self.listener.accept();
        // Checked after the wait, whatever ended it: the wake-up
        // connection, a client that raced it, or an error.
        if self.stop.load(Ordering::Acquire) {
            return Ok(None);
        }
        accepted.map(|(stream, _)| Some(stream))
    }
}

impl Conn for UnixStream {
    type Reader = UnixStream;
    type Writer = UnixStream;
    type Hangup = UnixStream;

    fn into_controller(self) -> io::Result<(UnixStream, UnixStream, UnixStream)> {
        self.set_write_timeout(Some(STALLED_WRITE_TIMEOUT))?;
        let read_half = self.try_clone()?;
        let hangup = self.try_clone()?;
        Ok((read_half, self, hangup))
    }

    /// One non-blocking write, then close. A rejectee that never reads
    /// cannot stall the acceptor — on a full buffer the write simply
    /// fails and the connection is closed without the frame.
    fn reject(self, frame: &[u8]) {
        let _ = self.set_nonblocking(true);
        let mut writer = &self;
        let _ = writer.write(frame);
    }
}

impl Outlet for UnixStream {
    fn send_all(&mut self, bytes: &[u8]) -> io::Result<()> {
        let mut writer = &*self;
        writer.write_all(bytes)
    }

    /// What the kernel has buffered still reaches the client after a
    /// `shutdown`, so there is nothing to flush first.
    fn close(self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}

impl HangUp for UnixStream {
    fn hang_up(&self) {
        let _ = self.shutdown(Shutdown::Both);
    }
}
