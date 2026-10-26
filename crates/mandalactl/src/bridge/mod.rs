// SPDX-License-Identifier: MPL-2.0

//! `mandalactl bridge`: the loopback relay a browser app is driven
//! through (`format/ipc.md` §Transport "Browser"). A page cannot
//! listen, so it connects *out* to this relay's app role
//! (`ws://127.0.0.1:<port>/app/<token>`), and `mandalactl --ws`
//! connects to the controller role (`…/controller/<token>`); the relay
//! forwards text frames verbatim between the two and owns the
//! single-controller rule ([`hub`]).
//!
//! **Trust.** The relay listens on loopback only and refuses any other
//! address. Anything on this machine that can open a TCP connection —
//! and that includes every page in every browser, which is the delta
//! from the Unix socket's `0600` — reaches the listener, so both roles
//! are behind a token in the URL path: a connection whose path names
//! another token, or no role, is refused at the handshake with an HTTP
//! error and nothing relayed. The token is 128 bits from the OS
//! (`getrandom`) unless `token=` names one, and is printed once, at
//! startup, as the two URLs; it travels in a `ws://` URL, which the
//! page's own `location.search` holds in the clear, so it is a
//! same-user secret, not a network one — the same tier the socket path
//! is (`format/ipc.md` §Trust model).
//!
//! **Threads.** One per connection, after its handshake: the thread
//! owns the socket and polls it under a short read timeout
//! ([`POLL`]), reading what its peer sent and writing what the hub
//! queued for it, so a frame crosses the relay in at most two polls.
//! A thread decides nothing — every rule is the hub's, applied under
//! one lock and returned as effects — so the rules are tested without
//! a socket, and these threads are only the carrying of bytes. Caps
//! ([`CONTROLLER_MESSAGE_CAP`], [`APP_MESSAGE_CAP`]) bound what the
//! relay buffers; the protocol's own caps are the app's to enforce.

mod hub;

use std::collections::HashMap;
use std::net::{SocketAddr, TcpListener, TcpStream, ToSocketAddrs};
use std::sync::mpsc::{self, Receiver, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::json;
use tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tungstenite::protocol::WebSocketConfig;
use tungstenite::{Message, WebSocket};

use crate::cli::Invocation;
use crate::params::{tokenize, Token};
use crate::render;
use crate::run::{emit, usage, Exit, Io};
use crate::wire::{FRAME_CAP, REQUEST_CAP};
use hub::ConnId;
pub use hub::{Effect, Hub, Role};

/// The command word.
pub const COMMAND: &str = "bridge";

/// Where the relay listens unless `listen=` says otherwise: loopback,
/// one above the dev server's `41815` (`Trunk.toml`).
pub const DEFAULT_LISTEN: &str = "127.0.0.1:41816";

/// How long a connection thread waits for its peer before looking at
/// what the hub queued for it: the relay's added latency, at most.
const POLL: Duration = Duration::from_millis(5);

/// The largest text message taken from a controller: the app answers
/// a request over its own 1 MiB cap with `parse_error`, so the relay
/// only has to bound its memory, generously above that.
const CONTROLLER_MESSAGE_CAP: usize = 4 * REQUEST_CAP;

/// The largest text message taken from the app: its replies are capped
/// at 8 MiB by the app itself.
const APP_MESSAGE_CAP: usize = FRAME_CAP + REQUEST_CAP;

/// How long a close handshake is given before the socket is dropped.
const CLOSE_GRACE: Duration = Duration::from_millis(500);

/// A frame bound for one connection's thread.
enum Outbound {
    Text(String),
    Close,
}

/// What the threads share: the rules, and each live connection's
/// outbound queue.
struct Shared {
    hub: Mutex<Hub>,
    queues: Mutex<HashMap<ConnId, Sender<Outbound>>>,
    token: String,
}

impl Shared {
    /// Apply what the hub decided: queue each frame and close for the
    /// thread that owns the connection. A queue that is gone belongs to
    /// a connection that already left; the hub learns that from its own
    /// `closed`, so a late effect is dropped here.
    fn apply(&self, effects: Vec<Effect>) {
        let queues = self
            .queues
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        for effect in effects {
            let (id, message) = match effect {
                Effect::Send(id, text) => (id, Outbound::Text(text)),
                Effect::Close(id) => (id, Outbound::Close),
            };
            if let Some(queue) = queues.get(&id) {
                let _ = queue.send(message);
            }
        }
    }

    /// Run one hub step under the lock and carry out its effects.
    fn decide(&self, step: impl FnOnce(&mut Hub) -> Vec<Effect>) {
        let effects = {
            let mut hub = self.hub.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            step(&mut hub)
        };
        self.apply(effects);
    }
}

/// The command-line arguments.
struct Args {
    listen: String,
    token: Option<String>,
}

/// `mandalactl bridge [listen=<addr:port>] [token=<token>]`: print the
/// two URLs, then relay until the process is ended.
pub fn run(invocation: &Invocation, io: &mut Io<'_>) -> Exit {
    if let Err(why) = refuse_socket_options(&invocation.options) {
        return usage(io, &why);
    }
    let args = match parse_args(&invocation.words) {
        Ok(args) => args,
        Err(why) => return usage(io, &why),
    };
    let listener = match bind(&args.listen) {
        Ok(listener) => listener,
        Err(why) => {
            emit(io.err, &format!("mandalactl: {why}\n"));
            return Exit::CommandError;
        }
    };
    let token = match args.token {
        Some(token) => token,
        None => match fresh_token() {
            Ok(token) => token,
            Err(why) => {
                emit(io.err, &format!("mandalactl: {why}\n"));
                return Exit::CommandError;
            }
        },
    };
    let bound = match listener.local_addr() {
        Ok(address) => address,
        Err(e) => {
            emit(
                io.err,
                &format!("mandalactl: cannot read the bound address: {e}\n"),
            );
            return Exit::CommandError;
        }
    };
    let result = json!({
        "listen": bound.to_string(),
        "app_url": url_for(bound, Role::App, &token),
        "controller_url": url_for(bound, Role::Controller, &token),
    });
    let text = if invocation.options.json {
        format!("{result}\n")
    } else {
        render::prefixed("", &result)
    };
    emit(io.out, &text);
    let _ = io.out.flush();
    serve(listener, token);
    Exit::Success
}

/// The URL a `role` connects to on a relay bound at `address`.
pub fn url_for(address: SocketAddr, role: Role, token: &str) -> String {
    format!("ws://{address}/{}/{token}", role.segment())
}

/// Bind the listener, on loopback only.
pub fn bind(listen: &str) -> Result<TcpListener, String> {
    let addresses: Vec<SocketAddr> = listen
        .to_socket_addrs()
        .map_err(|e| format!("`listen={listen}` is not an address: {e}; the default is {DEFAULT_LISTEN}"))?
        .collect();
    let Some(address) = addresses.first() else {
        return Err(format!("`listen={listen}` names no address"));
    };
    if !address.ip().is_loopback() {
        return Err(format!(
            "`listen={listen}` is not a loopback address; the relay serves this machine only (127.0.0.1 or [::1]), \
             since any page in any browser here can reach it and a token is its only guard"
        ));
    }
    TcpListener::bind(address).map_err(|e| match e.kind() {
        std::io::ErrorKind::AddrInUse => format!(
            "{address} is in use: another relay, or something else, listens there; pass listen=127.0.0.1:<port>"
        ),
        _ => format!("cannot listen on {address}: {e}"),
    })
}

/// The shortest `token=` the relay accepts. The token is the only guard
/// on a port every page on this machine can reach; a convenience token
/// of a few characters would quietly be no guard at all. Sixteen
/// characters of the accepted alphabet (64 symbols) are 96 bits — short
/// of the 128 a generated token carries, long past guessing by a page.
const MIN_TOKEN_LEN: usize = 16;

/// 128 bits from the OS, as 32 hex digits.
fn fresh_token() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes)
        .map_err(|e| format!("the OS gave no randomness for a token: {e}; pass token="))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn parse_args(words: &[String]) -> Result<Args, String> {
    let mut listen = DEFAULT_LISTEN.to_string();
    let mut token = None;
    for word in tokenize(words) {
        match word {
            Token::Named { name, value } => match name.as_str() {
                "listen" => listen = value,
                "token" => {
                    if value.is_empty()
                        || !value
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
                    {
                        return Err(
                            "`token=` takes letters, digits, `-` and `_`, and cannot be empty".to_string()
                        );
                    }
                    if value.len() < MIN_TOKEN_LEN {
                        return Err(format!(
                            "`token=` is {} characters; it is the relay's only guard, so it takes at least \
                             {MIN_TOKEN_LEN}. Leave it out for 128 random bits",
                            value.len()
                        ));
                    }
                    token = Some(value);
                }
                other => return Err(format!("`bridge` takes listen= and token=, not `{other}=`")),
            },
            Token::Positional(word) => {
                return Err(format!(
                    "`bridge` takes no bare words (got `{word}`): listen=<addr:port> and token=<token>"
                ))
            }
            Token::Json { name, .. } => {
                return Err(format!(
                    "`{name}:=` sends JSON to an app, and `bridge` is not one"
                ))
            }
        }
    }
    Ok(Args { listen, token })
}

/// The global flags that are about reaching an app, which this command
/// is the relay for and never a client of.
fn refuse_socket_options(options: &crate::cli::Options) -> Result<(), String> {
    let defaults = crate::cli::Options::default();
    let mut given = Vec::new();
    if options.ipc.is_some() {
        given.push("--ipc");
    }
    if options.ws.is_some() {
        given.push("--ws");
    }
    if options.wait_settled {
        given.push("--wait-settled");
    }
    if options.timeout != defaults.timeout {
        given.push("--timeout");
    }
    if options.force {
        given.push("--force");
    }
    if options.count.is_some() {
        given.push("--count");
    }
    if given.is_empty() {
        return Ok(());
    }
    Err(format!(
        "`bridge` is the relay, not a client of one, so {} would do nothing here. Remove {}; only --json applies",
        given.join(", "),
        if given.len() == 1 { "it" } else { "them" }
    ))
}

/// Accept forever, one thread per connection. Returns only when the
/// listener fails.
pub fn serve(listener: TcpListener, token: String) {
    let shared = Arc::new(Shared {
        hub: Mutex::new(Hub::default()),
        queues: Mutex::new(HashMap::new()),
        token,
    });
    let mut next_id: ConnId = 1;
    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let id = next_id;
        next_id += 1;
        let shared = Arc::clone(&shared);
        let _ = thread::Builder::new()
            .name(format!("mandalactl-bridge-{id}"))
            .spawn(move || connection(stream, id, &shared));
    }
}

/// The role a request's path asks for, when its token is `token`.
fn role_of(path: &str, token: &str) -> Option<Role> {
    let mut segments = path.trim_start_matches('/').splitn(2, '/');
    let role = match segments.next()? {
        "app" => Role::App,
        "controller" => Role::Controller,
        _ => return None,
    };
    tokens_match(segments.next()?, token).then_some(role)
}

/// Whether `given` is `token`, compared without an early exit: every
/// byte of an equal-length guess is looked at whatever it holds, so how
/// long a refusal takes says nothing about how much of the guess was
/// right. The token is the relay's only guard against any page on the
/// machine, and a page can time a refused handshake. The length is not
/// secret (a generated token is always 32 hex digits) and is compared
/// first. The property is in the shape of the loop — a fold with no
/// branch on the bytes — not something a test can time.
fn tokens_match(given: &str, token: &str) -> bool {
    given.len() == token.len()
        && given
            .bytes()
            .zip(token.bytes())
            .fold(0u8, |differ, (a, b)| differ | (a ^ b))
            == 0
}

/// One connection, handshake to close. (The handshake callback's `Err`
/// is tungstenite's own response type, large by its making, returned
/// once per connection.)
#[allow(clippy::result_large_err)]
fn connection(stream: TcpStream, id: ConnId, shared: &Shared) {
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    // The role is admitted inside the handshake callback, before the
    // `101` is written: once a client's handshake completes, the relay
    // already counts it, so a controller that connects the moment a
    // page's handshake returned finds the page there. What the hub
    // decides on admission (a second page's close, the page's
    // `attached`) waits in this connection's queue until the relay
    // loop below drains it.
    let (sender, receiver) = mpsc::channel();
    // A `Cell`: a handshake error keeps the callback borrowed.
    let admitted = std::cell::Cell::new(None);
    let check = |request: &Request, response: Response| -> Result<Response, ErrorResponse> {
        match role_of(request.uri().path(), &shared.token) {
            Some(role) => {
                shared
                    .queues
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner())
                    .insert(id, sender);
                shared.decide(|hub| hub.connected(id, role));
                admitted.set(Some(role));
                Ok(response)
            }
            None => Err(tungstenite::http::Response::builder()
                .status(403)
                .body(Some(
                    "this relay serves /app/<token> and /controller/<token>; the path or the token is wrong"
                        .to_string(),
                ))
                .unwrap_or_default()),
        }
    };
    // The cap is the role's, and the role is known only after the
    // callback ran: start with the stricter one and raise it for the
    // app once the handshake says so.
    let config = WebSocketConfig::default().max_message_size(Some(CONTROLLER_MESSAGE_CAP));
    let accepted = tungstenite::accept_hdr_with_config(stream, check, Some(config));
    let Some(role) = admitted.get() else {
        return;
    };
    // Admitted but the handshake then failed: the hub is told it left.
    if let Ok(mut socket) = accepted {
        if role == Role::App {
            socket.set_config(|config| config.max_message_size = Some(APP_MESSAGE_CAP));
        }
        relay(&mut socket, id, shared, &receiver);
    }
    shared
        .queues
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .remove(&id);
    shared.decide(|hub| hub.closed(id));
}

/// Carry bytes until the peer closes, the hub closes it, or the socket
/// fails: each turn reads what arrived within [`POLL`] and then writes
/// what the hub queued.
fn relay(socket: &mut WebSocket<TcpStream>, id: ConnId, shared: &Shared, queue: &Receiver<Outbound>) {
    let _ = socket.get_ref().set_read_timeout(Some(POLL));
    let mut closing: Option<Instant> = None;
    loop {
        match socket.read() {
            Ok(Message::Text(text)) => shared.decide(|hub| hub.message(id, text.as_str().to_string())),
            Ok(Message::Close(_))
            | Err(tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed) => return,
            Ok(_) => {}
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock
                        | std::io::ErrorKind::TimedOut
                        | std::io::ErrorKind::Interrupted
                ) => {}
            Err(_) => return,
        }
        if closing.is_some_and(|since| since.elapsed() > CLOSE_GRACE) {
            return;
        }
        loop {
            match queue.try_recv() {
                Ok(Outbound::Text(text)) => {
                    if socket.send(Message::Text(text.into())).is_err() {
                        return;
                    }
                }
                Ok(Outbound::Close) => {
                    if closing.is_none() {
                        closing = Some(Instant::now());
                        let _ = socket.close(None);
                        let _ = socket.flush();
                    }
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => return,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_the_role_is_read_from_the_path_and_the_token_must_match() {
        assert_eq!(role_of("/app/t0k", "t0k"), Some(Role::App));
        assert_eq!(role_of("/controller/t0k", "t0k"), Some(Role::Controller));
        assert_eq!(role_of("/controller/t0k", "other"), None, "a wrong token");
        assert_eq!(role_of("/controller", "t0k"), None, "no token");
        assert_eq!(role_of("/viewer/t0k", "t0k"), None, "no such role");
        assert_eq!(role_of("/", "t0k"), None);
        assert_eq!(
            role_of("/app/t0k/extra", "t0k"),
            None,
            "a token with a slash in it is another token"
        );
    }

    /// The token comparison accepts exactly the token: a guess wrong in
    /// its first byte, in its last, or by length is refused. The
    /// control is the token itself.
    #[test]
    fn test_the_token_comparison_accepts_exactly_the_token() {
        let token = "0123456789abcdef0123456789abcdef";
        assert!(tokens_match(token, token));
        for wrong in [
            "x123456789abcdef0123456789abcdef",
            "0123456789abcdef0123456789abcdex",
            "0123456789abcdef0123456789abcde",
            "0123456789abcdef0123456789abcdef0",
            "",
        ] {
            assert!(!tokens_match(wrong, token), "{wrong}");
        }
    }

    #[test]
    fn test_the_default_listen_address_is_loopback_and_others_are_refused() {
        assert!(bind("0.0.0.0:0").expect_err("not loopback").contains("loopback"));
        assert!(bind("not an address").is_err());
        let listener = bind("127.0.0.1:0").expect("loopback binds");
        assert!(listener.local_addr().expect("bound").ip().is_loopback());
        assert!(DEFAULT_LISTEN.starts_with("127.0.0.1:"));
    }

    #[test]
    fn test_a_fresh_token_is_thirty_two_hex_digits_and_never_repeats() {
        let a = fresh_token().expect("randomness");
        let b = fresh_token().expect("randomness");
        assert_eq!(a.len(), 32);
        assert!(a.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_ne!(a, b);
    }

    #[test]
    fn test_arguments_are_listen_and_token_only() {
        let args = parse_args(&[
            "listen=127.0.0.1:5000".to_string(),
            "token=abc-1_abc-1_abc-".to_string(),
        ])
        .expect("parses");
        assert_eq!(
            (args.listen.as_str(), args.token.as_deref()),
            ("127.0.0.1:5000", Some("abc-1_abc-1_abc-"))
        );
        let defaults = parse_args(&[]).expect("parses");
        assert_eq!((defaults.listen.as_str(), defaults.token), (DEFAULT_LISTEN, None));
        assert!(parse_args(&["token=".to_string()]).is_err());
        assert!(parse_args(&["token=a b".to_string()]).is_err());
        // The length floor, at its edge: one short is refused by name,
        // the floor itself is accepted.
        let short = "a".repeat(MIN_TOKEN_LEN - 1);
        let Err(refused) = parse_args(&[format!("token={short}")]) else {
            panic!("a {}-character token was accepted", short.len());
        };
        assert!(refused.contains("at least 16"), "{refused}");
        assert!(parse_args(&[format!("token={}", "a".repeat(MIN_TOKEN_LEN))]).is_ok());
        assert!(parse_args(&["port=1".to_string()]).is_err());
        assert!(parse_args(&["5000".to_string()]).is_err());
        assert!(parse_args(&["x:=1".to_string()]).is_err());
    }

    #[test]
    fn test_the_urls_name_the_role_and_the_token() {
        let address: SocketAddr = "127.0.0.1:41816".parse().expect("an address");
        assert_eq!(url_for(address, Role::App, "t"), "ws://127.0.0.1:41816/app/t");
        assert_eq!(
            url_for(address, Role::Controller, "t"),
            "ws://127.0.0.1:41816/controller/t"
        );
    }
}
