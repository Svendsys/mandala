// SPDX-License-Identifier: MPL-2.0

//! The `mandalactl` binary: arguments and environment in, the
//! library's [`mandalactl::run`] in the middle, its exit code out.
#![warn(clippy::unwrap_used)]
#![cfg_attr(test, allow(clippy::unwrap_used))]

use std::process::ExitCode;

use mandalactl::{run, Exit, Io};

fn main() -> ExitCode {
    let mut stdout = std::io::stdout().lock();
    let mut stderr = std::io::stderr().lock();
    let mut stdin = std::io::stdin().lock();
    let cwd = std::env::current_dir().ok();
    let mut io = Io {
        out: &mut stdout,
        err: &mut stderr,
        cwd: cwd.as_deref(),
        stdin: &mut stdin,
    };
    let Some(args) = std::env::args_os()
        .skip(1)
        .map(|a| a.into_string().ok())
        .collect::<Option<Vec<String>>>()
    else {
        return refuse(&mut io, "an argument is not valid UTF-8");
    };
    let env_ipc = match std::env::var("MANDALA_IPC") {
        Ok(path) => Some(path),
        Err(std::env::VarError::NotPresent) => None,
        Err(std::env::VarError::NotUnicode(_)) => return refuse(&mut io, "MANDALA_IPC is not valid UTF-8"),
    };
    ExitCode::from(run(&args, env_ipc.as_deref(), &mut io).code())
}

/// A usage error found before `run` could be asked.
fn refuse(io: &mut Io<'_>, why: &str) -> ExitCode {
    let _ = writeln!(io.err, "mandalactl: {why}");
    ExitCode::from(Exit::UsageError.code())
}
