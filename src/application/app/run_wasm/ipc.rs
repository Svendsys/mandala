// SPDX-License-Identifier: MPL-2.0

//! The browser loop's side of IPC: where the transport's pump meets
//! the app's state (`format/ipc.md` §Transport "Browser").
//!
//! [`IpcHost`] holds the same cells [`super::WasmApp`] holds, so a
//! pump can run from any of the three moments the transport asks for
//! one — a socket callback, the end of a frame, a wait's deadline
//! timer — and build the event-handler context ([`super::Borrowed`])
//! every command executes through, the way a winit event does. A
//! command that changed pixels asks for a frame exactly as a mutating
//! event handler does.
//!
//! **Re-borrow contract.** A pump borrows the input, the renderer and
//! the transport for its whole run. The socket's callbacks and the
//! timers are tasks of their own and never run inside a handler or a
//! frame, and the frame calls [`IpcHost::pump`] only after it has
//! released its own borrows — so no pump ever nests, and a nested one
//! (which only a future caller could introduce) is skipped rather than
//! panicked on.

#![cfg(target_arch = "wasm32")]

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::application::ipc::browser::socket::BrowserIpc;
use crate::application::keybinds::ResolvedKeybinds;
use crate::application::renderer::Renderer;

use super::ipc_target::IpcCommandTarget;
use super::{Borrowed, WasmInputState};

/// The cells a pump needs, shared with the loop's handler.
#[derive(Clone)]
pub(super) struct IpcHost {
    pub(super) input: Rc<RefCell<Option<WasmInputState>>>,
    pub(super) renderer: Rc<RefCell<Option<Renderer>>>,
    pub(super) keybinds: Rc<ResolvedKeybinds>,
    pub(super) suppress_keys: Rc<Cell<bool>>,
    /// `None` when the page was opened without `?ipc=`: IPC is off,
    /// and every pump returns at once.
    pub(super) ipc: Rc<RefCell<Option<BrowserIpc>>>,
}

impl IpcHost {
    /// Open the transport toward `url`; its wake is this host's pump.
    pub(super) fn connect(&self, url: String) {
        let host = self.clone();
        let wake: Rc<dyn Fn()> = Rc::new(move || host.pump());
        *self.ipc.borrow_mut() = Some(BrowserIpc::connect(url, wake));
    }

    /// Run the transport's pump over the app as it stands now.
    pub(super) fn pump(&self) {
        let Ok(mut ipc_borrow) = self.ipc.try_borrow_mut() else {
            log::warn!("ipc: a pump was asked for inside a pump; skipped");
            return;
        };
        let Some(ipc) = ipc_borrow.as_mut() else {
            return;
        };
        let mut input_borrow = self.input.borrow_mut();
        let mut renderer_borrow = self.renderer.borrow_mut();
        match (input_borrow.as_mut(), renderer_borrow.as_mut()) {
            (Some(input), Some(renderer)) => {
                let mut borrowed = Borrowed {
                    input,
                    renderer,
                    keybinds: &self.keybinds,
                    suppress_keys: &self.suppress_keys,
                };
                let settling = borrowed.settling();
                let mut target = IpcCommandTarget::new(&mut borrowed);
                let pumped = ipc.pump(Some(&mut target), Some(settling));
                if pumped.repaint {
                    borrowed.input.redraw_owed = true;
                    borrowed.renderer.request_redraw();
                }
            }
            // Before the app exists a command that needs it answers
            // `no_document`, as native's drain answers before its
            // window exists.
            _ => {
                ipc.pump(None, None);
            }
        }
    }
}
