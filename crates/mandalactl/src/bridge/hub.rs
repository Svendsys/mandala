// SPDX-License-Identifier: MPL-2.0

//! The relay's rules, as a pure state machine: who is connected in
//! which role, what each text frame does, and what a departure means
//! (`format/ipc.md` §Transport "Browser"). Every decision is a
//! function of the state and one event, and comes back as effects for
//! the caller to carry out — a frame to send, a connection to close —
//! so the rules are tested here without a socket, and the socket
//! threads in [`super`] carry effects without deciding anything.
//!
//! The rules are the native acceptor's, mapped onto a relay:
//!
//! - **One app.** The page connects to the app role; a second page
//!   while one is connected is closed at once, since two apps behind
//!   one relay could not be told apart.
//! - **One controller**, as `format/ipc.md` §Single controller: a
//!   second is refused `connection_rejected` with `reason: "busy"` and
//!   closed; one that arrives before any app is refused with
//!   `reason: "no_app"` and closed, since nothing would greet it.
//! - **Attach and detach are told to the app** with the relay's two
//!   control frames, which is how the app knows to greet with `hello`
//!   on a fresh session and to reset it when the controller leaves. A
//!   controller's frame that looks like one is dropped, never relayed,
//!   so the app cannot be lied to about its controller.
//! - **An app that leaves** ends the controller's session the way an
//!   exiting native app does: the controller is sent `shutdown` and
//!   closed.
//! - Text from the controller goes to the app, text from the app goes
//!   to the controller, verbatim; text from the app with no controller
//!   attached is dropped, as the native transport drops events nobody
//!   subscribed to.

use crate::wire::{is_bridge_frame, BRIDGE_ATTACHED, BRIDGE_DETACHED};

/// A connection's identity for the relay's lifetime.
pub type ConnId = u64;

/// Which role a connection asked for in its URL path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Role {
    /// `/app/<token>`: the browser page.
    App,
    /// `/controller/<token>`: `mandalactl --ws`, or any client.
    Controller,
}

impl Role {
    /// The path segment that names the role.
    pub fn segment(self) -> &'static str {
        match self {
            Role::App => "app",
            Role::Controller => "controller",
        }
    }
}

/// What the relay asks its threads to do.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Effect {
    /// Send one text frame to a connection.
    Send(ConnId, String),
    /// Close a connection, after whatever was sent to it.
    Close(ConnId),
}

/// The frame a refused controller receives instead of a greeting.
fn connection_rejected(reason: &str) -> String {
    format!(r#"{{"event":"connection_rejected","data":{{"reason":"{reason}"}}}}"#)
}

/// What a controller receives when its app goes away.
const SHUTDOWN: &str = r#"{"event":"shutdown","data":{}}"#;

/// Who is connected.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Hub {
    app: Option<ConnId>,
    controller: Option<ConnId>,
}

impl Hub {
    /// A connection completed its handshake as `role`.
    pub fn connected(&mut self, id: ConnId, role: Role) -> Vec<Effect> {
        match role {
            Role::App => {
                if self.app.is_some() {
                    return vec![Effect::Close(id)];
                }
                // No controller can be waiting: one is admitted only
                // while an app is here, and the app leaving takes it
                // (`closed`). A new page waits for a new controller.
                debug_assert!(self.controller.is_none(), "a controller never outlives its app");
                self.app = Some(id);
                vec![]
            }
            Role::Controller => {
                if self.controller.is_some() {
                    return vec![Effect::Send(id, connection_rejected("busy")), Effect::Close(id)];
                }
                let Some(app) = self.app else {
                    return vec![Effect::Send(id, connection_rejected("no_app")), Effect::Close(id)];
                };
                self.controller = Some(id);
                vec![Effect::Send(app, BRIDGE_ATTACHED.to_string())]
            }
        }
    }

    /// A connection sent one text frame.
    pub fn message(&mut self, from: ConnId, text: String) -> Vec<Effect> {
        if Some(from) == self.app {
            return match self.controller {
                Some(controller) => vec![Effect::Send(controller, text)],
                None => vec![],
            };
        }
        if Some(from) == self.controller {
            if is_bridge_frame(&text) {
                return vec![];
            }
            return match self.app {
                Some(app) => vec![Effect::Send(app, text)],
                None => vec![],
            };
        }
        // A connection that was refused and is being closed, or one
        // that already left: nothing it says goes anywhere.
        vec![]
    }

    /// A connection ended, for any reason.
    pub fn closed(&mut self, id: ConnId) -> Vec<Effect> {
        if Some(id) == self.app {
            self.app = None;
            return match self.controller.take() {
                Some(controller) => vec![
                    Effect::Send(controller, SHUTDOWN.to_string()),
                    Effect::Close(controller),
                ],
                None => vec![],
            };
        }
        if Some(id) == self.controller {
            self.controller = None;
            return match self.app {
                Some(app) => vec![Effect::Send(app, BRIDGE_DETACHED.to_string())],
                None => vec![],
            };
        }
        vec![]
    }

    /// Whether a page is connected.
    #[cfg(test)]
    pub fn has_app(&self) -> bool {
        self.app.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn send(to: ConnId, text: &str) -> Effect {
        Effect::Send(to, text.to_string())
    }

    #[test]
    fn test_a_controller_before_any_app_is_refused_no_app_and_closed() {
        let mut hub = Hub::default();
        assert_eq!(
            hub.connected(1, Role::Controller),
            vec![send(1, &connection_rejected("no_app")), Effect::Close(1)]
        );
        assert_eq!(hub, Hub::default(), "nothing was admitted");
        assert!(!hub.has_app());
    }

    #[test]
    fn test_an_app_then_a_controller_attaches_and_relays_both_ways() {
        let mut hub = Hub::default();
        assert_eq!(
            hub.connected(1, Role::App),
            vec![],
            "an app alone is told nothing"
        );
        assert!(hub.has_app());
        assert_eq!(hub.connected(2, Role::Controller), vec![send(1, BRIDGE_ATTACHED)]);
        assert_eq!(
            hub.message(2, "{\"id\":1,\"cmd\":\"ping\"}".into()),
            vec![send(1, "{\"id\":1,\"cmd\":\"ping\"}")]
        );
        assert_eq!(
            hub.message(1, "{\"id\":1,\"ok\":true}".into()),
            vec![send(2, "{\"id\":1,\"ok\":true}")]
        );
    }

    /// **The app leaving frees the controller slot at once**, before
    /// the shut-down controller's own close reaches the relay: in that
    /// window a fresh controller is refused `no_app` (not `busy`), a
    /// fresh page is told nothing, and the late close of the old
    /// controller tells the new page nothing either. The control is the
    /// same sequence with the old controller's close first, which ends
    /// identically: the window changes nothing.
    #[test]
    fn test_a_departing_app_frees_the_controller_slot_before_the_controller_closes() {
        let mut hub = Hub::default();
        hub.connected(1, Role::App);
        hub.connected(2, Role::Controller);
        assert_eq!(hub.closed(1), vec![send(2, SHUTDOWN), Effect::Close(2)]);
        // Controller 2's own `closed` has not arrived yet.
        assert_eq!(
            hub.connected(3, Role::Controller),
            vec![send(3, &connection_rejected("no_app")), Effect::Close(3)],
            "no app: the slot is free, so the refusal names the missing app"
        );
        assert_eq!(
            hub.connected(4, Role::App),
            vec![],
            "a new page waits for a new controller"
        );
        assert_eq!(
            hub.closed(2),
            vec![],
            "the old controller's late close is nobody's news"
        );
        assert_eq!(hub.connected(5, Role::Controller), vec![send(4, BRIDGE_ATTACHED)]);

        // Control: the old controller's close first.
        let mut ordered = Hub::default();
        ordered.connected(1, Role::App);
        ordered.connected(2, Role::Controller);
        ordered.closed(1);
        assert_eq!(ordered.closed(2), vec![]);
        assert_eq!(ordered.connected(4, Role::App), vec![]);
        assert_eq!(
            ordered.connected(5, Role::Controller),
            vec![send(4, BRIDGE_ATTACHED)]
        );
        assert_eq!(ordered, hub);
    }

    #[test]
    fn test_a_second_controller_is_busy_and_a_second_app_is_closed() {
        let mut hub = Hub::default();
        hub.connected(1, Role::App);
        hub.connected(2, Role::Controller);
        assert_eq!(
            hub.connected(3, Role::Controller),
            vec![send(3, &connection_rejected("busy")), Effect::Close(3)]
        );
        assert_eq!(hub.connected(4, Role::App), vec![Effect::Close(4)]);
        // The refused ones say nothing and leave nothing behind.
        assert_eq!(hub.message(3, "{\"id\":9,\"cmd\":\"ping\"}".into()), vec![]);
        assert_eq!(hub.closed(3), vec![]);
        assert_eq!(hub.closed(4), vec![]);
        assert_eq!(
            hub.message(2, "x".into()),
            vec![send(1, "x")],
            "the first controller is still attached"
        );
    }

    #[test]
    fn test_a_departing_controller_detaches_and_the_next_one_attaches_fresh() {
        let mut hub = Hub::default();
        hub.connected(1, Role::App);
        hub.connected(2, Role::Controller);
        assert_eq!(hub.closed(2), vec![send(1, BRIDGE_DETACHED)]);
        assert_eq!(
            hub.message(1, "{\"event\":\"document_changed\"}".into()),
            vec![],
            "nobody to hear it"
        );
        assert_eq!(hub.connected(3, Role::Controller), vec![send(1, BRIDGE_ATTACHED)]);
    }

    #[test]
    fn test_a_departing_app_shuts_its_controller_down_and_a_new_app_waits_for_a_new_controller() {
        let mut hub = Hub::default();
        hub.connected(1, Role::App);
        hub.connected(2, Role::Controller);
        assert_eq!(hub.closed(1), vec![send(2, SHUTDOWN), Effect::Close(2)]);
        assert!(!hub.has_app());
        assert_eq!(hub.closed(2), vec![], "already forgotten");
        assert_eq!(hub.connected(3, Role::App), vec![]);
        assert_eq!(hub.connected(4, Role::Controller), vec![send(3, BRIDGE_ATTACHED)]);
    }

    #[test]
    fn test_a_controllers_forged_control_frame_is_dropped() {
        let mut hub = Hub::default();
        hub.connected(1, Role::App);
        hub.connected(2, Role::Controller);
        assert_eq!(hub.message(2, BRIDGE_DETACHED.into()), vec![]);
        assert_eq!(hub.message(2, r#"{"bridge":"attached"}"#.into()), vec![]);
        assert_eq!(
            hub.message(2, r#"{"bridge":"x","id":1,"cmd":"ping"}"#.into()),
            vec![send(1, r#"{"bridge":"x","id":1,"cmd":"ping"}"#)],
            "a request with a stray key is the app's to refuse"
        );
    }
}
