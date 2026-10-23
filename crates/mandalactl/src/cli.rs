// SPDX-License-Identifier: MPL-2.0

//! The command line's outer grammar: global flags and the command
//! word. What follows the command word is [`crate::params`]'s.
//!
//! **Flags may sit anywhere** before a bare `--`, so
//! `mandalactl query.camera --json` and `mandalactl --json
//! query.camera` are the same call. That is the shape an agent writes
//! (`format/ipc.md`'s own examples end in the flags), and it costs
//! nothing because a command's arguments are bare words and
//! `name=value` pairs, never `--flags`. A word that starts with `--`
//! and is not a flag below is a usage error rather than a value, so a
//! typo like `--wait-settle` fails loudly instead of being sent as a
//! parameter; `--` ends flag parsing for a value that needs it.

use std::time::Duration;

/// How long to wait for any one reply, absent `--timeout`. Longer than
/// the 10 s the `FreezeWatchdog` allows a wedged app, so a freeze
/// surfaces as the server closing the socket, not as this deadline.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

/// The longest `--timeout` accepted: an hour is already a hung shell.
const MAX_TIMEOUT_SECS: f64 = 3600.0;

/// The shortest: a socket deadline of zero is an error, not "now".
const MIN_TIMEOUT: Duration = Duration::from_millis(1);

/// The global flags.
#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    /// `--ipc`; the environment variable is [`crate::run()`]'s to consult.
    pub ipc: Option<String>,
    /// Print the server's reply envelope as received, not a summary.
    pub json: bool,
    /// After a command that succeeds, wait for the app to settle.
    pub wait_settled: bool,
    /// Patience for each reply, and for the connection.
    pub timeout: Duration,
    /// Drive a server whose protocol this build does not speak.
    pub force: bool,
}

impl Default for Options {
    fn default() -> Self {
        Options {
            ipc: None,
            json: false,
            wait_settled: false,
            timeout: DEFAULT_TIMEOUT,
            force: false,
        }
    }
}

/// What the command line asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum Parsed {
    /// `--help`, `-h` or `help`, optionally about one command.
    Help(Option<String>),
    Version,
    Invoke(Invocation),
}

/// A command to run against the app.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub options: Options,
    /// The command word: a wire command, or `raw`.
    pub command: String,
    /// The words after it, flags removed, any `--` kept for
    /// [`crate::params::tokenize`].
    pub words: Vec<String>,
}

/// Read the command line (without the program name). `Err` is a
/// usage error, worded for the person or agent who typed it.
pub fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut options = Options::default();
    let mut command: Option<String> = None;
    let mut words: Vec<String> = Vec::new();
    let mut help = false;
    let mut version = false;
    let mut literal = false;
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        if literal {
            words.push(arg.clone());
        } else if arg == "--" {
            if command.is_none() {
                return Err("`--` came before any command".to_string());
            }
            literal = true;
            words.push(arg.clone());
        } else if let Some(flag) = arg.strip_prefix("--").filter(|f| !f.is_empty()) {
            let (name, inline) = match flag.split_once('=') {
                Some((name, value)) => (name, Some(value.to_string())),
                None => (flag, None),
            };
            match name {
                "ipc" => {
                    let path = value_of("--ipc", inline, &mut args)?;
                    if path.is_empty() {
                        return Err("`--ipc` needs a socket path, e.g. --ipc \"$XDG_RUNTIME_DIR/mandala.sock\"".to_string());
                    }
                    options.ipc = Some(path);
                }
                "timeout" => options.timeout = parse_timeout(&value_of("--timeout", inline, &mut args)?)?,
                "json" => options.json = flag_only("--json", inline)?,
                "wait-settled" => options.wait_settled = flag_only("--wait-settled", inline)?,
                "force" => options.force = flag_only("--force", inline)?,
                "help" => help = flag_only("--help", inline)?,
                "version" => version = flag_only("--version", inline)?,
                _ => {
                    return Err(format!(
                        "unknown flag `--{name}`. Flags: --ipc, --json, --wait-settled, --timeout, --force, --help, \
                         --version. To send a value that starts with `--`, put it after a bare `--`"
                    ))
                }
            }
        } else if arg == "-h" {
            help = true;
        } else if command.is_none() {
            command = Some(arg.clone());
        } else {
            words.push(arg.clone());
        }
    }
    if help {
        return Ok(Parsed::Help(command));
    }
    if version {
        return Ok(Parsed::Version);
    }
    match command {
        None => Err("no command given".to_string()),
        Some(command) if command == "help" => Ok(Parsed::Help(words.first().cloned())),
        Some(command) => Ok(Parsed::Invoke(Invocation {
            options,
            command,
            words,
        })),
    }
}

fn value_of<'a>(
    flag: &str,
    inline: Option<String>,
    rest: &mut impl Iterator<Item = &'a String>,
) -> Result<String, String> {
    if let Some(value) = inline {
        return Ok(value);
    }
    match rest.next() {
        Some(value) if !value.starts_with("--") => Ok(value.clone()),
        _ => Err(format!("`{flag}` needs a value")),
    }
}

fn flag_only(flag: &str, inline: Option<String>) -> Result<bool, String> {
    match inline {
        None => Ok(true),
        Some(_) => Err(format!("`{flag}` takes no value")),
    }
}

fn parse_timeout(text: &str) -> Result<Duration, String> {
    let refusal = || {
        format!(
            "`--timeout` takes seconds from {} to {MAX_TIMEOUT_SECS}, got `{text}`",
            MIN_TIMEOUT.as_secs_f64()
        )
    };
    match text.parse::<f64>() {
        Ok(secs) if secs.is_finite() && secs > 0.0 && secs <= MAX_TIMEOUT_SECS => {
            // Checked on the `Duration`, not the float: `1e-12` is above
            // zero and still rounds to a zero `Duration`, which no socket
            // call accepts as a deadline.
            let timeout = Duration::from_secs_f64(secs);
            if timeout >= MIN_TIMEOUT {
                Ok(timeout)
            } else {
                Err(refusal())
            }
        }
        _ => Err(refusal()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(line: &[&str]) -> Vec<String> {
        line.iter().map(|a| a.to_string()).collect()
    }

    fn invocation(line: &[&str]) -> Invocation {
        match parse(&args(line)).expect("parses") {
            Parsed::Invoke(invocation) => invocation,
            other => panic!("not an invocation: {other:?}"),
        }
    }

    #[test]
    fn test_flags_may_precede_or_follow_the_command() {
        let before = invocation(&["--json", "--ipc", "/s", "query.camera"]);
        let after = invocation(&["query.camera", "--ipc", "/s", "--json"]);
        assert_eq!(before, after);
        assert!(before.options.json);
        assert_eq!(before.options.ipc.as_deref(), Some("/s"));
        assert_eq!(before.command, "query.camera");
        assert!(before.words.is_empty());
    }

    #[test]
    fn test_inline_values_are_accepted() {
        let call = invocation(&["ping", "--ipc=/run/m.sock", "--timeout=2.5"]);
        assert_eq!(call.options.ipc.as_deref(), Some("/run/m.sock"));
        assert_eq!(call.options.timeout, Duration::from_millis(2500));
    }

    #[test]
    fn test_words_after_the_command_keep_their_order_with_flags_removed() {
        let call = invocation(&["input.drag", "1", "2", "--wait-settled", "3", "4", "steps=5"]);
        assert_eq!(call.words, args(&["1", "2", "3", "4", "steps=5"]));
        assert!(call.options.wait_settled);
    }

    #[test]
    fn test_double_dash_ends_flags_and_is_kept_for_the_tokenizer() {
        let call = invocation(&["act.console", "--", "--json", "-h"]);
        assert!(!call.options.json);
        assert_eq!(call.words, args(&["--", "--json", "-h"]));
    }

    #[test]
    fn test_negative_numbers_are_words_not_flags() {
        let call = invocation(&["input.wheel", "-3"]);
        assert_eq!(call.words, args(&["-3"]));
    }

    #[test]
    fn test_an_unknown_long_flag_is_refused_not_sent_as_a_value() {
        let err = parse(&args(&["ping", "--wait-settle"])).expect_err("a typo");
        assert!(
            err.contains("--wait-settle") && err.contains("--wait-settled"),
            "{err}"
        );
    }

    #[test]
    fn test_flags_that_need_values_say_so() {
        for line in [
            &["ping", "--ipc"][..],
            &["ping", "--ipc", "--json"],
            &["--timeout"],
            &["ping", "--ipc="],
        ] {
            assert!(parse(&args(line)).is_err(), "{line:?}");
        }
        assert!(parse(&args(&["ping", "--json=1"])).is_err());
    }

    #[test]
    fn test_timeout_must_be_a_positive_bounded_number() {
        for bad in ["0", "-1", "abc", "nan", "inf", "3601", "1e-12", "0.0005"] {
            let err = parse(&args(&["ping", "--timeout", bad])).expect_err("out of range");
            assert!(err.contains("--timeout"), "{bad}: {err}");
        }
        assert_eq!(
            invocation(&["ping", "--timeout", "0.25"]).options.timeout,
            Duration::from_millis(250)
        );
    }

    #[test]
    fn test_help_forms_resolve_to_help_about_the_named_command() {
        assert_eq!(parse(&args(&["--help"])), Ok(Parsed::Help(None)));
        assert_eq!(parse(&args(&["-h"])), Ok(Parsed::Help(None)));
        assert_eq!(parse(&args(&["help"])), Ok(Parsed::Help(None)));
        assert_eq!(
            parse(&args(&["help", "act.action"])),
            Ok(Parsed::Help(Some("act.action".into())))
        );
        assert_eq!(
            parse(&args(&["act.action", "--help"])),
            Ok(Parsed::Help(Some("act.action".into())))
        );
        assert_eq!(parse(&args(&["--version"])), Ok(Parsed::Version));
    }

    #[test]
    fn test_no_command_is_a_usage_error() {
        assert!(parse(&[]).expect_err("nothing").contains("no command"));
        assert!(parse(&args(&["--json"])).is_err());
        assert!(parse(&args(&["--", "ping"])).is_err());
    }

    #[test]
    fn test_the_timeout_defaults_above_the_freeze_watchdog() {
        assert!(Options::default().timeout > Duration::from_secs(10));
    }
}
