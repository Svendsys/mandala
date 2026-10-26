// SPDX-License-Identifier: MPL-2.0

//! The browser transport (`format/ipc.md` §Transport "Browser"): the
//! page cannot listen, so it opens a WebSocket *toward* a relay —
//! `mandalactl bridge`, on loopback — and an agent's `mandalactl --ws`
//! connects to the relay's other role. The relay forwards text frames
//! verbatim in both directions and owns the single-controller rule;
//! what it adds to the wire is two control frames of its own, telling
//! the page that a controller attached or detached, which is how the
//! page knows when to greet and when to reset its session.
//!
//! This module is the part of that transport that is protocol, not
//! browser: how the page finds its relay (`?ipc=`, [`ipc_url_from_query`]),
//! how it reads the relay's control frames ([`BridgeFrame`]), which
//! controller each queued message belongs to ([`stamp`]), and how it
//! paces reconnects ([`Backoff`]). It compiles on both targets, so
//! the native test suite proves it; the socket itself — web-sys's
//! `WebSocket`, the timers, the callbacks — is `socket`, wasm32-only
//! by nature. The relay's side of the same frames is
//! `mandalactl::bridge`, and a test in the app crate holds the two
//! spellings equal.

#[cfg(target_arch = "wasm32")]
pub(crate) mod socket;

/// The query parameter that names the relay: `?ipc=ws://…/app/<token>`.
pub(crate) const QUERY_KEY: &str = "ipc";

/// The key a relay control frame carries, and no request does: the
/// envelope reserves `id` and `cmd`, and a request's parameters are a
/// command's own. A controller frame carrying it is dropped by the
/// relay, so a page never mistakes a controller's text for the relay's.
pub(crate) const BRIDGE_KEY: &str = "bridge";

/// The first reconnect pause after the socket closes, and the longest:
/// the pause doubles from one to the other and stays there.
const BACKOFF_FIRST_MS: u64 = 250;
const BACKOFF_MAX_MS: u64 = 5_000;

/// What the relay tells the page about its other role.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BridgeFrame {
    /// A controller connected: the page greets it with `hello`, as the
    /// native acceptor greets a connection, on a fresh session.
    Attached,
    /// The controller left: the page resets its session — waits
    /// canceled, subscriptions cleared — as native's `Gone` does.
    Detached,
}

impl BridgeFrame {
    /// The frame's text on the wire, as the relay spells it
    /// (`mandalactl::wire::BRIDGE_ATTACHED` / `BRIDGE_DETACHED`); the
    /// app crate's tests hold the two spellings equal, and the page
    /// only ever reads one.
    #[cfg(test)]
    pub(crate) fn text(self) -> &'static str {
        match self {
            BridgeFrame::Attached => r#"{"bridge":"attached"}"#,
            BridgeFrame::Detached => r#"{"bridge":"detached"}"#,
        }
    }

    /// Read one text message as a control frame: a JSON object whose
    /// only key is [`BRIDGE_KEY`], naming one of the two. Anything
    /// else — a request, or a frame with more keys — is `None`, for
    /// the request decoder.
    pub(crate) fn parse(text: &str) -> Option<BridgeFrame> {
        let value: serde_json::Value = serde_json::from_str(text).ok()?;
        let object = value.as_object()?;
        if object.len() != 1 {
            return None;
        }
        match object.get(BRIDGE_KEY)?.as_str()? {
            "attached" => Some(BridgeFrame::Attached),
            "detached" => Some(BridgeFrame::Detached),
            _ => None,
        }
    }
}

/// One queued arrival, as [`stamp`] reads it: what the page's socket
/// callbacks saw, in the order they saw it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arrival {
    /// The relay's `attached`: a new controller generation begins.
    Attached,
    /// The relay's `detached`, or the socket closing: the controller
    /// is gone.
    Departed,
    /// A controller's text: a request, or a line that fails to decode.
    Text,
    /// Anything else (the socket opened, a reconnect is due).
    Other,
}

/// The controller generation each arrival of a batch belongs to, and
/// the generation that is current once the whole batch has happened.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Stamps {
    /// Per arrival: for a [`Arrival::Text`], the generation of the
    /// controller attached when it arrived, or `None` if none was;
    /// `None` for every other arrival.
    pub(crate) of: Vec<Option<u64>>,
    /// The controller attached at the end of the batch, if any.
    pub(crate) current: Option<u64>,
}

impl Stamps {
    /// Whether the text at `index` is owed an answer: a controller
    /// sent it, and that controller is still the current one. Native's
    /// drain asks the same of `request.generation` against the live
    /// controller (`transport::IpcServer`), and a request that fails it
    /// is dropped unexecuted there too.
    pub(crate) fn owed(&self, index: usize) -> bool {
        self.of[index].is_some() && self.of[index] == self.current
    }
}

/// Stamp a batch of queued arrivals with their controller generations,
/// starting from the page's state before the batch: `generation` is the
/// last one begun, `attached` whether its controller is still there.
///
/// The socket's callbacks queue what they see and the page drains the
/// queue later, in order — and later can be seconds on a throttled
/// tab. By the time a request is reached, its controller may already
/// have left and another attached: `[request(A), detached, attached]`
/// is a queue a slow pump really finds. Executing A's request then
/// would send its reply to B, ahead of B's `hello`. So the page looks
/// at the whole batch first: what is current is what is current once
/// every queued arrival has happened, which is what native's
/// generation check compares against too — its reader stamps a request
/// on arrival, and the drain drops it if the controller of that
/// generation is no longer the live one by the time it runs.
pub(crate) fn stamp(batch: &[Arrival], mut generation: u64, mut attached: bool) -> Stamps {
    let of = batch
        .iter()
        .map(|arrival| match arrival {
            Arrival::Attached => {
                generation += 1;
                attached = true;
                None
            }
            Arrival::Departed => {
                attached = false;
                None
            }
            Arrival::Text => attached.then_some(generation),
            Arrival::Other => None,
        })
        .collect();
    Stamps {
        of,
        current: attached.then_some(generation),
    }
}

/// `url` with everything after its first path segment replaced by `…`:
/// `ws://127.0.0.1:41816/app/<token>` → `ws://127.0.0.1:41816/app/…`.
/// What a log line shows of the relay URL. The token is the one guard
/// on the relay's port, and on the browser a `warn!`/`error!` during a
/// command is also captured into the reply's `warnings` — so no log
/// line carries it. `mandalactl::redacted` is the same rule for the
/// client's errors (the page cannot link the client), and a test in
/// `ipc::commands` holds the two equal.
pub(crate) fn redacted(url: &str) -> String {
    let authority = url.find("://").map_or(0, |at| at + 3);
    let Some(path) = url[authority..].find('/').map(|at| authority + at) else {
        return url.to_string();
    };
    match url[path + 1..].find('/') {
        Some(at) => format!("{}/…", &url[..path + 1 + at]),
        None => url.to_string(),
    }
}

/// The relay's URL from the page's query string (`location.search`,
/// with or without its leading `?`): `Ok(None)` when no `ipc` key is
/// present — the default, and IPC stays off — `Ok(Some(url))` for a
/// `ws://` or `wss://` URL, and `Err` naming the fault for a value
/// that is neither, so a typo is reported in the console rather than
/// silently ignored. The value is percent-decoded first; a bare URL
/// needs no encoding, but a browser or a user may have encoded it.
pub(crate) fn ipc_url_from_query(search: &str) -> Result<Option<String>, String> {
    let query = search.strip_prefix('?').unwrap_or(search);
    let Some(raw) = query.split('&').find_map(|pair| {
        pair.strip_prefix(QUERY_KEY)
            .and_then(|rest| rest.strip_prefix('='))
    }) else {
        return Ok(None);
    };
    let url = percent_decode(raw);
    if url.is_empty() {
        return Err(format!(
            "`?{QUERY_KEY}=` is empty; it names the relay, e.g. ws://127.0.0.1:41816/app/<token>"
        ));
    }
    if !(url.starts_with("ws://") || url.starts_with("wss://")) {
        return Err(format!(
            "`?{QUERY_KEY}={}` is not a WebSocket URL: it must start with ws:// or wss://",
            redacted(&url)
        ));
    }
    Ok(Some(url))
}

/// `%XX` escapes decoded, `+` left alone (a query value here is a URL,
/// where `+` is literal). A malformed escape is kept as written.
fn percent_decode(raw: &str) -> String {
    let bytes = raw.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let decoded = (bytes[i] == b'%' && i + 3 <= bytes.len())
            .then(|| u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?, 16).ok())
            .flatten();
        match decoded {
            Some(byte) => {
                out.push(byte);
                i += 3;
            }
            None => {
                out.push(bytes[i]);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// How long to wait before reconnecting after the socket closes:
/// [`BACKOFF_FIRST_MS`] the first time, doubling to [`BACKOFF_MAX_MS`],
/// reset by a socket that opened.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Backoff {
    next_ms: u64,
}

impl Default for Backoff {
    fn default() -> Self {
        Backoff {
            next_ms: BACKOFF_FIRST_MS,
        }
    }
}

impl Backoff {
    /// The pause before the next attempt, and the one after it grows.
    pub(crate) fn next(&mut self) -> u64 {
        let pause = self.next_ms;
        self.next_ms = (self.next_ms * 2).min(BACKOFF_MAX_MS);
        pause
    }

    /// A connection opened: the next failure starts over.
    pub(crate) fn reset(&mut self) {
        *self = Backoff::default();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_relay_url_is_read_from_the_query_among_other_keys() {
        let url = "ws://127.0.0.1:41816/app/t0k3n";
        assert_eq!(
            ipc_url_from_query(&format!("?map=maps/a.mindmap.json&ipc={url}")),
            Ok(Some(url.to_string()))
        );
        assert_eq!(
            ipc_url_from_query(&format!("ipc={url}&map=x")),
            Ok(Some(url.to_string()))
        );
        assert_eq!(
            ipc_url_from_query("?map=maps/a.mindmap.json"),
            Ok(None),
            "absent: IPC is off"
        );
        assert_eq!(ipc_url_from_query(""), Ok(None));
        assert_eq!(
            ipc_url_from_query("?ipcx=ws://h/app/t"),
            Ok(None),
            "a key that merely starts with `ipc` is another key"
        );
    }

    #[test]
    fn test_an_encoded_relay_url_is_decoded_and_a_malformed_escape_is_kept() {
        assert_eq!(
            ipc_url_from_query("?ipc=ws%3A%2F%2F127.0.0.1%3A41816%2Fapp%2Ft"),
            Ok(Some("ws://127.0.0.1:41816/app/t".to_string()))
        );
        assert_eq!(percent_decode("a%2"), "a%2");
        assert_eq!(percent_decode("a%zz"), "a%zz");
        assert_eq!(percent_decode("%"), "%");
    }

    #[test]
    fn test_a_relay_url_that_is_not_a_websocket_url_is_refused_by_name() {
        for bad in [
            "?ipc=",
            "?ipc=http://127.0.0.1:41816/app/t",
            "?ipc=127.0.0.1:41816",
        ] {
            let err = ipc_url_from_query(bad).expect_err(bad);
            assert!(err.contains("ipc"), "{bad}: {err}");
        }
        assert!(ipc_url_from_query("?ipc=wss://h/app/t").is_ok());
    }

    #[test]
    fn test_bridge_frames_round_trip_and_nothing_else_parses_as_one() {
        for frame in [BridgeFrame::Attached, BridgeFrame::Detached] {
            assert_eq!(BridgeFrame::parse(frame.text()), Some(frame));
        }
        for text in [
            r#"{"id":1,"cmd":"ping"}"#,
            r#"{"bridge":"attached","id":1}"#,
            r#"{"bridge":"gone"}"#,
            r#"{"bridge":1}"#,
            r#""attached""#,
            "not json",
        ] {
            assert_eq!(BridgeFrame::parse(text), None, "{text}");
        }
    }

    /// **A request whose controller left before the pump reached it is
    /// not owed an answer** — the queue a throttled tab really finds.
    /// The control is the same request with no departure behind it,
    /// which is owed, and the next controller's request in the same
    /// batch, which is owed too.
    #[test]
    fn test_a_request_from_a_departed_controller_is_not_owed_in_its_batch() {
        use Arrival::{Attached, Departed, Text};
        let stamps = stamp(&[Text, Departed, Attached, Text], 1, true);
        assert_eq!(stamps.of, vec![Some(1), None, None, Some(2)]);
        assert_eq!(stamps.current, Some(2));
        assert!(!stamps.owed(0), "A's request: A left behind it");
        assert!(stamps.owed(3), "B's request: B is current");

        // Control: nothing departs behind it, and it is owed.
        let stamps = stamp(&[Text], 1, true);
        assert!(stamps.owed(0));
    }

    /// The socket closing is a departure as `detached` is, a text sent
    /// while no controller is attached belongs to none, and a batch
    /// that only attaches leaves its controller current.
    #[test]
    fn test_a_close_retires_and_an_unattached_text_belongs_to_no_one() {
        use Arrival::{Attached, Departed, Other, Text};
        let stamps = stamp(&[Text, Other, Departed], 4, true);
        assert!(!stamps.owed(0), "the socket closed behind it");
        assert_eq!(stamps.current, None);

        let stamps = stamp(&[Text, Attached], 4, false);
        assert_eq!(stamps.of[0], None, "nobody was attached when it came");
        assert!(!stamps.owed(0));
        assert_eq!(stamps.current, Some(5));

        // Control: attached before it, and still attached at the end.
        let stamps = stamp(&[Attached, Other, Text], 4, false);
        assert!(stamps.owed(2));
    }

    #[test]
    fn test_backoff_doubles_to_its_cap_and_resets() {
        let mut backoff = Backoff::default();
        let pauses: Vec<u64> = (0..6).map(|_| backoff.next()).collect();
        assert_eq!(pauses, vec![250, 500, 1000, 2000, 4000, 5000]);
        assert_eq!(backoff.next(), 5000, "stays at the cap");
        backoff.reset();
        assert_eq!(backoff.next(), 250);
    }
}
