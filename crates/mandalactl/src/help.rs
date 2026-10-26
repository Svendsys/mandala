// SPDX-License-Identifier: MPL-2.0

//! `--help`, written for an agent that has not seen this tool: what it
//! connects to, how arguments are spelled, what the exit codes mean,
//! and — for every command — a line saying what it does and a command
//! line to copy.
//!
//! The per-command part is generated from [`crate::verbs::VERBS`], so
//! it cannot list a command that does not exist or omit one that does;
//! the rest is prose that a test holds to the flags and examples the
//! parser really accepts.

use crate::verbs::{self, family, type_word, Effect, Verb, VERBS};

/// Single-line commands the overview teaches. Each is parsed in the
/// tests, so none can rot into advice that no longer works.
pub const WORKFLOW: &[&str] = &[
    "mandalactl ping",
    "mandalactl describe input.drag",
    "mandalactl query.selection --json",
    "mandalactl scene.find_text Introduction",
    "mandalactl input.click_node 1.2",
    "mandalactl input.pending",
    "mandalactl capture.screenshot /tmp/after.png fit=true width=1920 height=1080",
    "mandalactl capture.record_start /tmp/zoom fps=20",
    "mandalactl act.action ZoomIn",
    "mandalactl capture.record_stop",
    "mandalactl events.subscribe document_changed --count 1",
];

/// The client's own commands, which never touch the socket. Each is
/// parsed in the tests, as [`WORKFLOW`] is.
pub const LOCAL: &[&str] = &[
    "mandalactl record assemble /tmp/zoom",
    "mandalactl record assemble /tmp/zoom out=zoom.gif width=640 fps=15",
    "mandalactl record assemble /tmp/zoom format=mp4 overwrite=true",
];

/// The relay's command lines, parsed in the tests like [`LOCAL`]'s.
/// Kept apart from it because the relay is read by its own argument
/// reader, and because it is not native-less: the browser build of
/// this crate has no relay.
pub const BRIDGE: &[&str] = &[
    "mandalactl bridge",
    "mandalactl bridge listen=127.0.0.1:41816 token=0123456789abcdef",
];

/// Argument spellings the overview teaches, one per line. Also parsed
/// in the tests.
pub const SPELLINGS: &[&str] = &[
    "mandalactl scene.hit_test 412 96",
    "mandalactl scene.hit_test x=412 y=96 space=surface",
    "mandalactl input.key z mods=ctrl,shift",
    "mandalactl capture.screenshot region:='{\"x\":0,\"y\":0,\"w\":200,\"h\":100}'",
    "mandalactl act.console -- 'fps=on'",
];

/// The overview: everything but the per-command reference is prose.
pub fn overview() -> String {
    let mut text = String::new();
    text.push_str(HEAD);
    text.push_str("\nSPELLINGS\n");
    for line in SPELLINGS {
        text.push_str(&format!("  {line}\n"));
    }
    text.push_str(MIDDLE);
    text.push_str("\nWORKFLOW\n");
    for line in WORKFLOW {
        text.push_str(&format!("  {line}\n"));
    }
    text.push_str(RAW_EXAMPLE);
    text.push_str(LOCAL_HEAD);
    for line in LOCAL.iter().chain(BRIDGE) {
        text.push_str(&format!("  {line}\n"));
    }
    text.push_str("\nCOMMANDS  ([mutates] = changes the app, so --wait-settled applies)\n");
    let mut current = "";
    for verb in VERBS {
        let family = family(verb.name);
        if family != current {
            text.push_str(&format!("\n  {family}\n"));
            current = family;
        }
        let tag = if verb.effect == Effect::Mutates {
            " [mutates]"
        } else {
            ""
        };
        text.push_str(&format!("    {}{tag}\n", usage(verb)));
        text.push_str(&format!("        {}\n", verb.summary));
        text.push_str(&format!("        $ {}\n", verb.example));
    }
    text.push_str(TAIL);
    text
}

/// One command's help: usage, parameters, example.
///
/// `Err` when `name` is not in the table; the message says where the
/// connected app's own list is.
pub fn command_help(name: &str) -> Result<String, String> {
    if name == "raw" {
        return Ok(RAW_HELP.to_string());
    }
    if name == crate::assemble::COMMAND {
        return Ok(RECORD_HELP.to_string());
    }
    if name == "bridge" {
        return Ok(BRIDGE_HELP.to_string());
    }
    let Some(verb) = verbs::lookup(name) else {
        return Err(format!(
            "`{name}` is not in this build's command table. If the connected app serves it, `mandalactl describe {name}` \
             documents it and `mandalactl {name} name=value ...` calls it"
        ));
    };
    let mut text = format!("{}\n    {}\n", usage(verb), verb.summary);
    if verb.params.is_empty() {
        text.push_str("\n  takes no parameters\n");
    } else {
        text.push_str("\n  parameters:\n");
        for param in verb.params {
            let position = verb
                .positional
                .iter()
                .position(|p| *p == param.name)
                .map_or(String::new(), |i| format!("  (bare word {})", i + 1));
            text.push_str(&format!(
                "    {:<16}{:<16}{}{position}\n",
                param.name,
                type_word(param.ty),
                if param.required { "required" } else { "optional" }
            ));
        }
    }
    text.push_str(&format!("\n  example: {}\n", verb.example));
    if verb.effect == Effect::Mutates {
        text.push_str(
            "  changes the app: --wait-settled returns only once the result is rendered (needs an app that serves clock.wait)\n",
        );
    }
    text.push_str(&format!(
        "  every parameter and result field is documented by the app itself: mandalactl describe {name}\n"
    ));
    Ok(text)
}

/// `mandalactl name <positional> [optional positional] [name=<type>] ...`
pub fn usage(verb: &Verb) -> String {
    let mut line = format!("mandalactl {}", verb.name);
    for name in verb.positional {
        let required = verb.params.iter().any(|p| p.name == *name && p.required);
        if required {
            line.push_str(&format!(" <{name}>"));
        } else {
            line.push_str(&format!(" [{name}]"));
        }
    }
    for param in verb.params.iter().filter(|p| !verb.positional.contains(&p.name)) {
        if param.required {
            line.push_str(&format!(" {}=<{}>", param.name, type_word(param.ty)));
        } else {
            line.push_str(&format!(" [{}=<{}>]", param.name, type_word(param.ty)));
        }
    }
    line
}

const HEAD: &str = "\
mandalactl - drive a running Mandala from the shell, over its --ipc socket or, for the browser build,
             through a WebSocket relay this tool also is (mandalactl bridge)

USAGE
  mandalactl [--ipc <socket> | --ws <url>] [--json] [--wait-settled] [--timeout <secs>] [--count <n>] <command> [args...]
  mandalactl raw '<json request>'     send one request frame as written ('raw -' reads stdin)
  mandalactl help [<command>]         this text, or one command's parameters

Each <command> is a wire command of format/ipc.md, spelled as the wire spells it. `mandalactl
describe` lists what the connected app serves, with every parameter and result, so a command newer
than this build is still callable (name=value arguments are then typed from that list).

SOCKET
  Start the app with IPC on:  mandala maps/testament.mindmap.json --ipc \"$XDG_RUNTIME_DIR/mandala.sock\"
  With no display (a container, CI), add --headless: no window, and a clock that moves only by
  clock.step until clock.set_mode real (format/ipc.md \"Headless and one-shot launches\")
  Name it here with --ipc <path>, or once per shell with:  export MANDALA_IPC=<path>
  On Windows the value is a pipe name, the app's own --ipc value: --ipc mandala is \\\\.\\pipe\\mandala-mandala
  One client is served at a time. A refusal for that is retried for 2 s, then reported.

BROWSER
  A browser build cannot listen, so it connects out to a relay: run `mandalactl bridge` (prints an app
  URL and a controller URL, both on loopback, both behind one token), open the page with
  ?ipc=<app URL> beside ?map=, then name the controller URL here with --ws <url> or once per shell
  with:  export MANDALA_WS=<url>. Same commands, same replies; `describe` says which commands the
  browser does not serve (act.console, capture.record_*), and capture.screenshot comes back inline and
  is written to the <path> given here. `mandalactl help bridge` has the rest.

ARGUMENTS
  <value>        a bare word fills the command's next positional parameter (<name> in the list below)
  name=value     a parameter, typed as the command declares it: x=412 is a number, text=412 a string,
                 fit=true a boolean, mods=ctrl,shift a list (mods= is the empty list, \"hold none\")
  name:=<json>   a parameter sent as the JSON written, for anything else
  paths          path=, to_file= and the like may be relative: they are made absolute against the
                 current directory, since the app requires absolute paths and has no directory of its own
  --             ends flags: every word after it is a bare word, even one that looks like name=value
  Quote words with spaces or JSON; single quotes keep JSON intact.";

const MIDDLE: &str = "
FLAGS (anywhere before a bare --)
  --ipc <path>      the socket of a native app, or on Windows its pipe name (else $MANDALA_IPC)
  --ws <url>        the controller URL of a `mandalactl bridge` a browser app is behind (else $MANDALA_WS);
                    exclusive with --ipc, and so are the two variables
  --json            print the app's reply exactly as received, one JSON line per request, instead of a summary
  --wait-settled    after a command that changes the app, wait until its consequences are rendered
                    (clock.wait until=settled), so the next read or screenshot sees them; refused on
                    read-only commands. An app without clock.wait is detected first: exit 1, and the
                    command is not sent.
  --timeout <secs>  patience for the connection and for each reply (default 15; the wait gets the same).
                    A clock.wait reply is waited on at least to its own timeout_ms, plus 2 s
  --count <n>       events.subscribe only: stop after printing n events (else it runs until the app exits
                    or stdout closes; one line per event, the frame as received with --json)
  --force           drive an app whose protocol differs from this build's
  --help, --version

OUTPUT
  stdout: one `path: value` line per field, values as JSON (so \"1\" and 1 differ), or with --json the
  app's reply envelope. act.console prints the console's own lines. stderr: errors as
  `error: <code>: <message>` and the app's warnings as `warning: <code>: <message>`.

EXIT CODES
  0  success: the app answered ok:true (warnings do not change this)
  1  the app said no: ok:false, code and message on stderr
  2  the app could not be reached, or the request did not go out whole: not running, busy,
     refused, wrong protocol. Nothing ran, so repeating the command is safe
  3  bad command line: nothing was sent
  4  the request was sent and the invocation did not complete: no reply (timeout, the app
     closed or is shutting down), or --wait-settled's wait failed after the command succeeded.
     The command may have run: read the app's state before repeating one that changes it";

const RAW_HELP: &str = "\
mandalactl raw '<json request>'
mandalactl raw -
    Send one request frame exactly as written and print the reply. The frame is a JSON object with
    \"cmd\" and the command's parameters flat beside it, as format/ipc.md writes them; \"id\" is optional
    and one is supplied when absent. `-` reads the frame from stdin, the way to send JSON with no
    shell quoting at all (stdin is read to its end, so run it from a pipe or a heredoc, not a bare
    terminal). --json, --wait-settled, --timeout and the exit codes work as for any command.

  example: mandalactl raw '{\"cmd\":\"ping\"}'
";

const LOCAL_HEAD: &str = "

CLIENT-SIDE (read files, never the socket: no app needs to be running, no --ipc)
  mandalactl record assemble <dir> [out=<file>] [format=gif|mp4] [width=<px>] [fps=<n>] [max_frames=<n>] [overwrite=true]
      Turn a recording's frames and manifest.json (made by capture.record_start / capture.record_stop)
      into an animated GIF, which GitHub shows inline in a PR. `mandalactl help record` says how.
  mandalactl bridge [listen=127.0.0.1:41816] [token=<token>]
      The relay a browser build is driven through: prints its app URL (for the page's ?ipc=) and its
      controller URL (for --ws), then relays until stopped. `mandalactl help bridge` says how.
";

const BRIDGE_HELP: &str = "\
mandalactl bridge [listen=<addr:port>] [token=<token>]
    The relay between a browser build of Mandala and this tool (format/ipc.md \"Browser\"). A page
    cannot listen, so it connects out to the relay's app role, and `mandalactl --ws` connects to its
    controller role; the relay passes frames between the two verbatim and serves one controller at a
    time, as the native socket does. It runs until the process is ended (Ctrl+C) and prints, first,
    one `path: value` line each (or one JSON object with --json):

    listen           the address it listens on, loopback only (listen=127.0.0.1:41816 by default)
    app_url          ws://<listen>/app/<token>: open the page as  http://<trunk host>/?map=<map>&ipc=<app_url>
    controller_url   ws://<listen>/controller/<token>: pass it as --ws, or  export MANDALA_WS=<controller_url>

    token=           the secret both roles must name in their URL; 128 random bits from the OS unless given,
                     and at least 16 letters, digits, `-` or `_` when it is.
                     Anything on this machine can reach a loopback port — every page in every browser
                     included, which is what the token guards against; it is a same-user secret, the
                     tier the socket path is, and never a network one.

    A controller that connects before the page is refused (connection_rejected, reason no_app: exit 2
    here, with the remedy). When the page leaves, its controller is sent `shutdown` and closed; when the
    controller leaves, the page resets its session and the next controller is greeted afresh. Over
    --ws, `capture.screenshot <path>` writes the PNG the page returns inline, and its sidecar beside it.
";

const RECORD_HELP: &str = "\
mandalactl record assemble <dir> [out=<file>] [format=gif|mp4] [width=<px>] [fps=<n>] [max_frames=<n>] [overwrite=true]
    Assemble a recording into one file. <dir> is the directory capture.record_stop reported: frames
    (frame_00000.png ...) and manifest.json. The command is the client's own — it reads files and never
    opens the socket, so no app need be running and no --ipc is needed. A relative <dir> or out= is read
    against the current directory.

    gif (default)  an animated GIF that loops, written by this tool with nothing else installed. Each
                   frame is shown for as long as the manifest says (a stretch the recording missed holds
                   the frame before it), so it plays at the speed it was recorded. A GIF cannot show a
                   frame for under 20 ms (browsers play it as 100 ms), so frames closer than that are left
                   out, and the result says how many.
    mp4            H.264 through the host's ffmpeg, which must be on PATH (never required for the GIF).

    out=           the file to write; default <dir>/recording.gif (or .mp4). An existing file is kept
                   unless overwrite=true.
    width=         scale the frames down to this many pixels wide, keeping their shape. The way to bring a
                   GIF under GitHub's 10 MiB inline limit, which the result warns about.
    fps=           show at most this many frames a second (1-60): fewer frames, same speed.
    max_frames=    GIF only: keep at most this many frames (default 600, up to 3600), thinned evenly, same
                   speed. (An mp4 keeps every frame; fps= sets its frame rate.)

    stdout: one `path: value` line per field (--json: one JSON object): out, format, frames_in,
    frames_out, width, height, duration_ms, bytes. stderr: warnings. Exit 0 made, 1 could not (no
    manifest, a bad frame, no ffmpeg, a file in the way), 3 bad command line.

  example: mandalactl record assemble /tmp/zoom out=zoom.gif width=640
";

const RAW_EXAMPLE: &str = "  mandalactl raw - <<'JSON'
  {\"cmd\":\"act.macro\",\"steps\":[{\"kind\":\"Action\",\"action\":\"ZoomFit\"}]}
  JSON";

const TAIL: &str = "
A command the app serves but this list lacks (one newer than this build) is
called by name with name=value arguments: mandalactl family.verb name=value. `mandalactl describe`
shows what the connected app has.";

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::{parse, Parsed};
    use crate::params::{build_params, tokenize, Signature};
    use crate::words::split;

    /// Run a taught command line through the real parser and argument
    /// builder, returning the command it names.
    fn accepted(line: &str) -> String {
        let words = split(line);
        assert_eq!(words[0], "mandalactl", "{line}");
        let Parsed::Invoke(call) = parse(&words[1..]).unwrap_or_else(|e| panic!("`{line}`: {e}")) else {
            panic!("`{line}` is not an invocation");
        };
        let verb = verbs::lookup(&call.command).unwrap_or_else(|| panic!("`{line}`: no such verb"));
        build_params(&Signature::of(verb), &tokenize(&call.words), None)
            .unwrap_or_else(|e| panic!("`{line}`: {e}"));
        if call.options.wait_settled {
            assert_eq!(
                verb.effect,
                Effect::Mutates,
                "`{line}`: --wait-settled on a read-only command"
            );
        }
        call.command
    }

    #[test]
    fn test_every_verb_example_is_a_command_line_that_parses_for_its_own_verb() {
        for verb in VERBS {
            assert_eq!(
                accepted(verb.example),
                verb.name,
                "the example of `{}` runs another command",
                verb.name
            );
        }
    }

    #[test]
    fn test_every_taught_workflow_and_spelling_parses() {
        for line in WORKFLOW.iter().chain(SPELLINGS) {
            accepted(line);
        }
    }

    /// The client's own commands are read by the real parser and the
    /// real argument reader, so the overview cannot teach one that is
    /// refused.
    #[test]
    fn test_every_client_side_example_is_read_by_the_command_it_teaches() {
        for line in LOCAL.iter().chain(&["mandalactl record assemble /d"]) {
            let words = split(line);
            let Parsed::Invoke(call) = parse(&words[1..]).unwrap_or_else(|e| panic!("`{line}`: {e}")) else {
                panic!("`{line}` is not an invocation");
            };
            assert_eq!(call.command, crate::assemble::COMMAND, "{line}");
            crate::assemble::parse_args(&call.words, Some(std::path::Path::new("/w")))
                .unwrap_or_else(|e| panic!("`{line}`: {e}"));
        }
        let overview = overview();
        for line in LOCAL {
            assert!(overview.contains(line), "the overview never teaches {line}");
        }
        assert!(command_help("record")
            .expect("known")
            .contains("never\n    opens the socket"));
    }

    /// The relay's command lines are read by the real parser, and the
    /// overview teaches them.
    #[test]
    fn test_every_bridge_example_parses_and_is_taught() {
        let overview = overview();
        for line in BRIDGE {
            let words = split(line);
            let Parsed::Invoke(call) = parse(&words[1..]).unwrap_or_else(|e| panic!("`{line}`: {e}")) else {
                panic!("`{line}` is not an invocation");
            };
            assert_eq!(call.command, "bridge", "{line}");
        }
        assert!(
            overview.contains("mandalactl bridge [listen="),
            "the overview never teaches the relay"
        );
        assert!(command_help("bridge").expect("known").contains("app_url"));
    }

    #[test]
    fn test_the_overview_lists_every_verb_with_its_summary_and_example() {
        let text = overview();
        for verb in VERBS {
            assert!(text.contains(&usage(verb)), "no usage line for {}", verb.name);
            assert!(text.contains(verb.summary), "no summary for {}", verb.name);
            assert!(
                text.contains(&format!("$ {}", verb.example)),
                "no example for {}",
                verb.name
            );
        }
    }

    #[test]
    fn test_the_overview_names_every_flag_exit_code_and_the_discovery_variable() {
        let text = overview();
        for needle in [
            "--ipc",
            "--ws",
            "MANDALA_WS",
            "--json",
            "--wait-settled",
            "--timeout",
            "--force",
            "--help",
            "--version",
            "MANDALA_IPC",
            "raw",
            "describe",
            "  0  ",
            "  1  ",
            "  2  ",
            "  3  ",
            "  4  ",
        ] {
            assert!(text.contains(needle), "the overview never mentions {needle:?}");
        }
    }

    #[test]
    fn test_usage_marks_required_optional_and_named_parameters() {
        let hit = usage(verbs::lookup("scene.hit_test").expect("known"));
        assert_eq!(hit, "mandalactl scene.hit_test <x> <y> [space=<string>]");
        let undo = usage(verbs::lookup("query.undo").expect("known"));
        assert_eq!(undo, "mandalactl query.undo [limit]");
        let key = usage(verbs::lookup("input.key").expect("known"));
        assert_eq!(key, "mandalactl input.key <key> [state=<string>] [mods=<array>]");
    }

    #[test]
    fn test_command_help_lists_parameters_with_their_bare_word_positions() {
        let text = command_help("scene.hit_test").expect("known");
        assert!(
            text.contains("(bare word 1)") && text.contains("(bare word 2)"),
            "{text}"
        );
        assert!(
            text.contains("example: mandalactl scene.hit_test 412 96 space=surface"),
            "{text}"
        );
        assert!(
            !text.contains("--wait-settled"),
            "a read-only command does not advertise it: {text}"
        );
        let text = command_help("input.drag").expect("known");
        assert!(text.contains("--wait-settled"), "{text}");
        assert!(command_help("ping")
            .expect("known")
            .contains("takes no parameters"));
    }

    #[test]
    fn test_command_help_for_a_stranger_points_at_describe() {
        let err = command_help("future.verb").expect_err("not in the table");
        assert!(err.contains("mandalactl describe future.verb"), "{err}");
    }
}
