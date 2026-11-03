//! Headless PhotoCraft command line. `run` is the whole program, so it can be
//! tested in-process as well as through the binary.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

use std::ffi::OsString;
use std::io::Write;

use clap::CommandFactory;

mod actions;
mod args;
mod automation;
mod batch;
mod document;
mod error;
mod output;

pub use actions::{ActionsError, Steps, load_actions, load_actions_file, parse_actions};
use args::{Command, parse};
use automation::{commands, mcp, serve};
use batch::{batch, droplet};
use document::{convert, info, run_cmds};
pub(crate) use error::{CliError, Result};

/// The command line's clap definition, for generating shell completions and man pages
/// (`examples/man-and-completions.rs`).
pub fn command() -> clap::Command {
    args::Cli::command()
}

/// Run the CLI on its arguments (without the program name). Returns the process exit code (0 ok,
/// 1 failure, 2 usage). clap reads the arguments as they are: a path that is not valid Unicode (a
/// Latin-1 file name on Linux) is taken, and one where text is expected is a usage error (exit 2).
/// Never a panic, as `std::env::args()` aborts on one (issue #1108).
pub fn run<I, T>(args: I, out: &mut dyn Write, err: &mut dyn Write) -> i32
where
    I: IntoIterator<Item = T>,
    T: Into<OsString>,
{
    let args: Vec<OsString> = args.into_iter().map(Into::into).collect();
    // Help and `--version` (`help`, `-h`, `<subcommand> --help`, `-V`) come back as errors too,
    // for `out`, exit 0: the subcommand never runs.
    let cli = match parse(&args) {
        Ok(cli) => cli,
        Err(e) => {
            let to: &mut dyn Write = if e.use_stderr() { err } else { out };
            let _ = write!(to, "{}", e.render());
            return e.exit_code();
        }
    };
    let result = match &cli.command {
        Command::Convert(a) => convert(a, out, err),
        Command::Info(a) => info(a, out),
        Command::Run(a) => run_cmds(a, out, err),
        Command::Batch(a) => batch(a, out, err),
        Command::Droplet(a) => droplet(a, out, err),
        Command::Commands(a) => commands(a, out),
        Command::Mcp(a) => mcp(a, err),
        Command::Serve(a) => serve(a, err),
    };
    match result {
        Ok(()) => 0,
        Err(e) => {
            let _ = writeln!(err, "error: {e}");
            1
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    /// What [`crate::run`] returned and wrote.
    pub(crate) struct Invoke {
        pub(crate) code: i32,
        pub(crate) stdout: String,
        pub(crate) stderr: String,
    }

    pub(crate) fn invoke(args: &[&str]) -> Invoke {
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        let code = crate::run(args, &mut stdout, &mut stderr);
        Invoke { code, stdout: String::from_utf8(stdout).unwrap(), stderr: String::from_utf8(stderr).unwrap() }
    }
}
