// SPDX-License-Identifier: MPL-2.0

//! The browser's socket: one WebSocket toward the relay's app role,
//! re-opened whenever it closes, and the session the controller behind
//! the relay drives through it ([`BrowserIpc`]). The wasm32 half of
//! [`super`]; the protocol half is tested on native.
//!
//! **Where things run.** Everything here runs on the browser's main
//! thread, in two kinds of task. The socket's callbacks (`onopen`,
//! `onmessage`, `onclose`) are tasks of their own: each pushes what it
//! saw onto a queue and wakes the app. The app drains the queue in
//! [`BrowserIpc::pump`], from the wake, from the end of every frame
//! and from a timer armed at a pending `clock.wait`'s deadline — the
//! three moments native's event loop drains, observes and evaluates
//! too. A command therefore executes with the same access an input
//! handler has, between handlers and never inside one, which is the
//! whole of the threading contract on a target that has no threads.
//!
//! **What the relay adds.** The relay tells the page when a controller
//! attached or detached ([`BridgeFrame`]); the page greets an attached
//! controller with `hello` on a fresh session and resets the session
//! when it detaches — the accept/disconnect lifecycle of the native
//! acceptor, mapped onto a connection that stays up across
//! controllers. A controller's text that arrives while none is
//! attached is dropped: nothing is owed to nobody.
//!
//! **Caps.** A text message over the inbound line cap is answered
//! `parse_error`, as the native reader answers an overlong line; a
//! reply over the outbound cap is `reply_too_large`, by the envelope's
//! own encoder. Events are offered only while the socket's send buffer
//! is under [`EVENT_BUFFER_CAP`], and stay queued — coalesced behind
//! newer ones of their class — until it drains; a reply is always
//! sent, since a request's answer is owed.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{CloseEvent, MessageEvent, WebSocket};

use super::{Backoff, BridgeFrame};
use crate::application::ipc::envelope::{
    decode_line, Decoded, Event, Frame, IpcRequest, Reply, INBOUND_LINE_CAP, OUTBOUND_FRAME_CAP,
};
use crate::application::ipc::events::{Driver, Queued, Session};
use crate::application::ipc::wait::{PendingWait, Settling};
use crate::application::ipc::{execute, CommandTarget, Executed};

/// Bytes the socket may hold unsent before events wait for it to
/// drain (`WebSocket.bufferedAmount`). Replies are not held back.
const EVENT_BUFFER_CAP: u32 = 1 << 20;

/// What a socket callback or a timer saw, queued for the next pump.
enum Inbound {
    Opened,
    Text(String),
    Closed,
    /// The reconnect pause is over: open a new socket.
    Reconnect,
}

/// A `setTimeout` kept alive until it fires or is dropped, which
/// clears it.
struct Timer {
    handle: i32,
    _closure: Closure<dyn FnMut()>,
}

impl Timer {
    /// `f` in `delay_ms`, on the window's timer.
    fn after(delay_ms: u64, f: impl FnMut() + 'static) -> Option<Timer> {
        let closure = Closure::<dyn FnMut()>::new(f);
        let handle = web_sys::window()?
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                closure.as_ref().unchecked_ref(),
                i32::try_from(delay_ms).unwrap_or(i32::MAX),
            )
            .ok()?;
        Some(Timer {
            handle,
            _closure: closure,
        })
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        if let Some(window) = web_sys::window() {
            window.clear_timeout_with_handle(self.handle);
        }
    }
}

/// An open attempt: the socket and the callbacks it calls into. Dropped
/// whole on close, after the callbacks are detached, so a late event
/// from a dead socket reaches no dropped closure.
struct Live {
    socket: WebSocket,
    _on_open: Closure<dyn FnMut(JsValue)>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
}

impl Drop for Live {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onclose(None);
        self.socket.set_onerror(None);
        let _ = self.socket.close();
    }
}

/// What one pump did that the loop must act on.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pumped {
    /// Some executed command changed pixels; ask for a frame.
    pub(crate) repaint: bool,
}

/// The relay connection and the controller session behind it.
pub(crate) struct BrowserIpc {
    url: String,
    live: Option<Live>,
    /// A controller is attached behind the relay.
    attached: bool,
    /// Bumped per attached controller: what a request is stamped
    /// with, so one from a departed controller is never executed.
    generation: u64,
    line_index: u64,
    inbound: Rc<RefCell<VecDeque<Inbound>>>,
    wake: Rc<dyn Fn()>,
    session: Session,
    published_map: Option<String>,
    backoff: Backoff,
    reconnect: Option<Timer>,
    /// The timer armed at the pending wait's deadline, and the
    /// deadline it was armed for.
    wait_timer: Option<(web_time::Instant, Timer)>,
}

impl BrowserIpc {
    /// Open the socket toward `url` (the relay's app role). `wake` is
    /// called from a socket callback or a timer whenever there is
    /// something to pump; it must pump and return.
    pub(crate) fn connect(url: String, wake: Rc<dyn Fn()>) -> BrowserIpc {
        let mut ipc = BrowserIpc {
            url,
            live: None,
            attached: false,
            generation: 0,
            line_index: 0,
            inbound: Rc::new(RefCell::new(VecDeque::new())),
            wake,
            session: Session::default(),
            published_map: None,
            backoff: Backoff::default(),
            reconnect: None,
            wait_timer: None,
        };
        ipc.open();
        ipc
    }

    /// One attempt: a socket, with callbacks that queue what they see
    /// and wake the app.
    fn open(&mut self) {
        self.reconnect = None;
        let socket = match WebSocket::new(&self.url) {
            Ok(socket) => socket,
            Err(e) => {
                // A URL the browser refuses to even try (a malformed
                // one, or an insecure one from a secure page) will not
                // get better: say so once and stop.
                log::error!("ipc: cannot open a WebSocket to {}: {e:?}; IPC is off", self.url);
                return;
            }
        };
        let queue = |what: Inbound, inbound: &Rc<RefCell<VecDeque<Inbound>>>, wake: &Rc<dyn Fn()>| {
            inbound.borrow_mut().push_back(what);
            wake();
        };
        let (inbound, wake) = (Rc::clone(&self.inbound), Rc::clone(&self.wake));
        let on_open =
            Closure::<dyn FnMut(JsValue)>::new(move |_: JsValue| queue(Inbound::Opened, &inbound, &wake));
        let (inbound, wake) = (Rc::clone(&self.inbound), Rc::clone(&self.wake));
        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
            // Text only: the relay forwards text frames, and a binary
            // one is nothing the protocol spells.
            if let Some(text) = event.data().as_string() {
                queue(Inbound::Text(text), &inbound, &wake);
            }
        });
        let (inbound, wake) = (Rc::clone(&self.inbound), Rc::clone(&self.wake));
        let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |_: CloseEvent| {
            queue(Inbound::Closed, &inbound, &wake)
        });
        socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        // An error is always followed by a close, which is what is
        // acted on; the handler only keeps the console quiet.
        log::info!("ipc: connecting to {}", self.url);
        self.live = Some(Live {
            socket,
            _on_open: on_open,
            _on_message: on_message,
            _on_close: on_close,
        });
    }

    /// Drain what the callbacks queued, execute what a controller
    /// sent, deliver what the session owes, and evaluate a pending
    /// wait. `app` is the command target, or `None` before the app
    /// exists, when a command that needs it answers `no_document`.
    pub(crate) fn pump(
        &mut self,
        mut app: Option<&mut dyn CommandTarget>,
        settling: Option<Settling>,
    ) -> Pumped {
        let mut pumped = Pumped::default();
        // Look at what the user did before any command runs, so a
        // change of theirs is reported as theirs and not folded into
        // the first command's. Only when subscribed.
        if let Some(observed) = app.as_deref().and_then(|app| app.observed()) {
            if self.session.is_subscribed() {
                self.session.observe(&observed, Driver::App);
            }
        }
        loop {
            let next = self.inbound.borrow_mut().pop_front();
            let Some(message) = next else { break };
            match message {
                Inbound::Opened => {
                    log::info!("ipc: connected to the relay at {}", self.url);
                    self.backoff.reset();
                }
                Inbound::Closed => {
                    self.detach();
                    self.live = None;
                    let pause = self.backoff.next();
                    log::warn!("ipc: the relay connection closed; reconnecting in {pause} ms");
                    let wake = Rc::clone(&self.wake);
                    let inbound = Rc::clone(&self.inbound);
                    // Queued as a message so the open happens in a
                    // pump, where `&mut self` is.
                    self.reconnect = Timer::after(pause, move || {
                        inbound.borrow_mut().push_back(Inbound::Reconnect);
                        wake();
                    });
                }
                Inbound::Reconnect => {
                    if self.live.is_none() {
                        self.open();
                    }
                }
                Inbound::Text(text) => {
                    if let Some(frame) = BridgeFrame::parse(&text) {
                        match frame {
                            BridgeFrame::Attached => {
                                self.attach(app.as_deref().map(|app| app as &dyn CommandTarget))
                            }
                            BridgeFrame::Detached => self.detach(),
                        }
                        continue;
                    }
                    if !self.attached {
                        log::debug!(
                            "ipc: dropped {} bytes sent while no controller is attached",
                            text.len()
                        );
                        continue;
                    }
                    let line_index = self.line_index;
                    self.line_index += 1;
                    let decoded = if text.len() > INBOUND_LINE_CAP {
                        Decoded::Rejected(Reply::parse_error(
                            line_index,
                            format!(
                                "frame of {} bytes exceeds the {INBOUND_LINE_CAP} byte inbound cap",
                                text.len()
                            ),
                        ))
                    } else {
                        decode_line(text.as_bytes(), line_index, self.generation)
                    };
                    match decoded {
                        Decoded::Blank => {}
                        Decoded::Rejected(reply) => self.send(Frame::Reply(reply)),
                        Decoded::Request(request) => {
                            // Re-wrapped to shorten the trait object's
                            // lifetime to this one call: `&mut` is
                            // invariant (as `registry::Call::drive`).
                            let target = app.as_deref_mut().map(|app| app as &mut dyn CommandTarget);
                            let executed = self.execute(&request, target);
                            pumped.repaint |= executed.repaint;
                            if let Some(reply) = executed.reply {
                                self.send(Frame::Reply(reply));
                            }
                        }
                    }
                }
            }
        }
        if let (Some(app), Some(settling)) = (app.as_deref().map(|app| app as &dyn CommandTarget), settling) {
            self.evaluate_wait(app, settling);
            self.publish_map(app);
        }
        self.deliver_events();
        self.arm_wait_timer();
        pumped
    }

    /// A controller attached: a fresh session, greeted with `hello`.
    fn attach(&mut self, app: Option<&dyn CommandTarget>) {
        self.generation += 1;
        self.attached = true;
        self.session.reset_controller();
        if let Some(app) = app {
            self.publish_map(app);
        }
        log::info!("ipc: controller attached (generation {})", self.generation);
        self.send(Frame::Event(Event::hello(self.published_map.as_deref())));
    }

    /// The controller left (or the socket did): its waits and
    /// subscriptions go with it.
    fn detach(&mut self) {
        if self.attached {
            log::info!("ipc: controller detached");
        }
        self.attached = false;
        self.session.reset_controller();
        self.wait_timer = None;
    }

    fn execute(&mut self, request: &IpcRequest, app: Option<&mut dyn CommandTarget>) -> Executed {
        execute(request, app, &mut self.session)
    }

    /// Send the pending `clock.wait`'s reply if it is due: its
    /// condition holds for `settling`, or its deadline has passed.
    fn evaluate_wait(&mut self, app: &dyn CommandTarget, settling: Settling) {
        let Some(reply) = self.session.resolve_wait(web_time::Instant::now(), settling) else {
            return;
        };
        let Some(observed) = app.observed() else { return };
        let revisions = self.session.observe(&observed, Driver::App);
        self.send(Frame::Reply(reply.with_revisions(revisions)));
    }

    /// Arm (or re-arm) the timer that pumps at the pending wait's
    /// deadline, so a wait times out on an idle page without polling.
    fn arm_wait_timer(&mut self) {
        let Some(deadline) = self.session.wait().map(PendingWait::deadline) else {
            self.wait_timer = None;
            return;
        };
        if self
            .wait_timer
            .as_ref()
            .is_some_and(|(armed, _)| *armed == deadline)
        {
            return;
        }
        let delay_ms = deadline
            .saturating_duration_since(web_time::Instant::now())
            .as_millis() as u64
            + 1;
        let wake = Rc::clone(&self.wake);
        self.wait_timer = Timer::after(delay_ms, move || wake()).map(|timer| (deadline, timer));
    }

    /// Offer the queued events to the controller, behind the replies
    /// that announced them. One the socket has no room for stays
    /// queued, coalesced behind newer ones of its class, until a later
    /// pump finds room.
    fn deliver_events(&mut self) {
        if !self.attached {
            return;
        }
        while let Some(Queued { class, event }) = self.session.pop_pending() {
            let room = self
                .live
                .as_ref()
                .is_some_and(|live| live.socket.buffered_amount() < EVENT_BUFFER_CAP);
            if !room {
                self.session.requeue(Queued { class, event });
                return;
            }
            self.send(Frame::Event(event));
        }
    }

    /// Publish the document path `hello` reports.
    fn publish_map(&mut self, app: &dyn CommandTarget) {
        let path = app.document().and_then(|doc| doc.file_path.as_deref());
        if self.published_map.as_deref() != path {
            self.published_map = path.map(str::to_string);
        }
    }

    /// One frame to the relay, as one text message. The newline the
    /// line protocol ends a frame with is not sent: on this transport
    /// a message is a frame.
    fn send(&mut self, frame: Frame) {
        let Some(live) = self.live.as_ref() else {
            return;
        };
        let Some(mut bytes) = frame.encode(OUTBOUND_FRAME_CAP) else {
            log::warn!("ipc: an event over the {OUTBOUND_FRAME_CAP} byte cap was dropped");
            return;
        };
        if bytes.last() == Some(&b'\n') {
            bytes.pop();
        }
        let text = String::from_utf8_lossy(&bytes);
        if let Err(e) = live.socket.send_with_str(&text) {
            log::warn!("ipc: send failed: {e:?}");
        }
    }
}
