//! The control and automation launch options, as clap types: `photocraft-cli mcp` and `serve`
//! flatten them, and so does the desktop app (#7071), so each flag and its environment variable
//! are declared once. A flag wins over its variable.
//!
//! Parse with [`blank_env_is_unset`] applied to the arguments, so an empty variable counts as unset,
//! and pass a parse error through [`name_env_vars`], so a bad value names the variable it came from.
//! Check the token pair with [`conflict`] only where a token is used, so a stale exported pair
//! doesn't stop a launch that needs none.

use std::ffi::OsString;
use std::path::PathBuf;

use clap::builder::{NonEmptyStringValueParser, OsStringValueParser, TypedValueParser};
use clap::error::{ContextKind, ContextValue, ErrorKind};
use clap::parser::ValueSource;
use clap::{Arg, ArgMatches, Args, Command};

use crate::{AuthorizedWorkspace, AutomationError};

/// The control token, for a server or for a client of one.
#[derive(Args)]
pub struct ControlArgs {
    /// The control token (64 hex digits)
    #[arg(long, value_name = "64-HEX", env = "PHOTOCRAFT_CONTROL_TOKEN", hide_env_values = true, value_parser = NonEmptyStringValueParser::new())]
    pub control_token: Option<String>,
    /// A file holding the control token, used instead of --control-token (a server creates it
    /// when missing)
    #[arg(long, value_name = "PATH", env = "PHOTOCRAFT_CONTROL_TOKEN_FILE")]
    pub control_token_file: Option<PathBuf>,
}

/// The folders automation may touch. Without a root, that direction of access is denied. No
/// variable stands in for them here: one exported for the desktop app must not widen headless
/// `photocraft-cli` access (#1551); the desktop app adds its own.
#[derive(Args)]
pub struct AutomationArgs {
    /// Ordered folders automation may read from; repeat for more roots (none without it)
    #[arg(long, value_name = "DIR", value_parser = root_path())]
    pub automation_read_root: Vec<PathBuf>,
    /// The folder automation may write to (none without it)
    #[arg(long, value_name = "DIR", value_parser = root_path())]
    pub automation_write_root: Option<PathBuf>,
}

fn root_path() -> impl TypedValueParser<Value = PathBuf> {
    OsStringValueParser::new().try_map(|value: OsString| if value.is_empty() { Err("root needs a non-empty directory") } else { Ok(PathBuf::from(value)) })
}

impl AutomationArgs {
    /// The roots, opened as capabilities.
    pub fn workspace(&self) -> Result<AuthorizedWorkspace, AutomationError> {
        AuthorizedWorkspace::with_read_roots(self.automation_read_root.iter().map(PathBuf::as_path), self.automation_write_root.as_deref())
    }
}

/// A control port, for clap's `value_parser`: `photocraft --control`, `photocraft-cli serve --port`
/// and PHOTOCRAFT_CONTROL_PORT, which may carry spaces.
pub fn control_port(s: &str) -> Result<u16, NotAPort> {
    s.trim().parse().map_err(|_| NotAPort)
}

/// A [`control_port`] value that isn't a port.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("expected a port, 0-65535")]
pub struct NotAPort;

/// For `Command::mut_args`: an empty or blank variable counts as unset, so `PHOTOCRAFT_CONTROL_PORT=`
/// turns a fallback off without `unset` (#2491). clap would pass the empty value to the parser.
pub fn blank_env_is_unset(arg: Arg) -> Arg {
    let blank = arg.get_env().and_then(std::env::var_os).is_some_and(|v| v.to_str().is_some_and(|v| v.trim().is_empty()));
    if blank { arg.env(None::<&str>) } else { arg }
}

/// clap names the flag when a value from its variable is bad or conflicts with another, as if the
/// flag had been given: name the variable instead when the flag isn't among `args` (without the
/// binary name; a first one naming a subcommand selects whose arguments to look in) (#2491).
pub fn name_env_vars(mut error: clap::Error, cmd: &Command, args: &[OsString]) -> clap::Error {
    // `--flag value` or `--flag=value`.
    let given = |long: &str| args.iter().filter_map(|a| a.to_str()?.strip_prefix("--")?.strip_prefix(long)).any(|r| r.is_empty() || r.starts_with('='));
    let sub = args.first().and_then(|name| cmd.find_subcommand(name)).unwrap_or(cmd);
    for kind in [ContextKind::InvalidArg, ContextKind::PriorArg] {
        let Some(ContextValue::String(shown)) = error.get(kind) else { continue };
        let Some(arg) = sub.get_arguments().find(|a| a.to_string() == *shown) else { continue };
        let (Some(long), Some(var)) = (arg.get_long(), arg.get_env()) else { continue };
        if !given(long) {
            error.insert(kind, ContextValue::String(var.to_string_lossy().into_owned()));
        }
    }
    error
}

/// clap's either/or usage error when `m`, parsed by `cmd`, has both `a` and `b`, each named as it was
/// given: the flag, or its variable. For a conflict that depends on the mode, such as the token pair
/// (only a server or bridge reads a token), which clap's `conflicts_with` can't express.
pub fn conflict(cmd: &mut Command, m: &ArgMatches, a: &str, b: &str) -> Result<(), clap::Error> {
    if m.value_source(a).is_none() || m.value_source(b).is_none() {
        return Ok(());
    }
    let shown = |id: &str| {
        let arg = cmd.get_arguments().find(|arg| arg.get_id() == id);
        match (m.value_source(id), arg.and_then(Arg::get_env)) {
            (Some(ValueSource::EnvVariable), Some(var)) => var.to_string_lossy().into_owned(),
            _ => arg.map_or_else(|| id.to_owned(), |arg| arg.to_string()),
        }
    };
    let message = format!("the argument '{}' cannot be used with '{}'", shown(a), shown(b));
    Err(cmd.error(ErrorKind::ArgumentConflict, message))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn control_port_accepts_valid_numbers() {
        assert_eq!(control_port("50494"), Ok(50494));
        assert_eq!(control_port(" 8080 "), Ok(8080));
        assert_eq!(control_port("0"), Ok(0));
    }

    #[test]
    fn control_port_rejects_anything_else() {
        for bad in ["78787", "-1", "", "x"] {
            assert_eq!(control_port(bad), Err(NotAPort), "{bad:?}");
        }
    }

    #[derive(clap::Parser)]
    struct RootCli {
        #[command(flatten)]
        automation: AutomationArgs,
    }

    #[test]
    fn repeated_read_roots_preserve_order_and_write_roots_stay_single() {
        use clap::Parser;
        let parsed = RootCli::try_parse_from(["test", "--automation-read-root", "first", "--automation-read-root=second"]).unwrap();
        assert_eq!(parsed.automation.automation_read_root, vec![PathBuf::from("first"), PathBuf::from("second")]);
        for args in [
            vec!["test", "--automation-write-root", "one", "--automation-write-root=two"],
            vec!["test", "--automation-write-root=one", "--automation-write-root", "one"],
            vec!["test", "--automation-read-root="],
            vec!["test", "--automation-read-root", "--automation-write-root=work"],
            vec!["test", "--automation-write-root="],
        ] {
            assert!(RootCli::try_parse_from(args.clone()).is_err(), "accepted {args:?}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_root_paths_are_preserved_without_lossy_conversion() {
        use clap::Parser;
        use std::os::unix::ffi::OsStringExt;
        let value = OsString::from_vec(b"library-\xff".to_vec());
        let parsed = RootCli::try_parse_from([OsString::from("test"), OsString::from("--automation-read-root"), value.clone()]).unwrap();
        assert_eq!(parsed.automation.automation_read_root, vec![PathBuf::from(value.clone())]);
        let mut inline = OsString::from("--automation-read-root=");
        inline.push(&value);
        let parsed = RootCli::try_parse_from([OsString::from("test"), inline]).unwrap();
        assert_eq!(parsed.automation.automation_read_root, vec![PathBuf::from(value)]);
    }
}
