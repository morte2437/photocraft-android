//! The desktop app's command line, as clap types (#7071): `photocraft --help` comes from these doc
//! comments, so the help and the parser can't disagree. Anything they don't declare is a usage
//! error naming it (exit status 2), so a typo such as `--safegpu` doesn't open the window and then
//! fail to open the flag as a file (#6431); so is a flag without its value, rather than a silent
//! fall back to a generated token or to no root (#6429), a repeated flag, and a bad
//! `--present-mode` or `--frame-latency`. A value never starts with `-` (write `--opt=-value`),
//! except for negative numbers, and a file whose name starts with `-` goes after `--`.
//!
//! The control and automation flags are `photocraft-automation`'s, shared with `photocraft-cli`.
//! Only the desktop app reads the automation roots from `PHOTOCRAFT_AUTOMATION_*_ROOT` (#1551).

use std::ffi::OsString;
use std::fmt;
use std::path::PathBuf;

use clap::builder::FalseyValueParser;
use clap::parser::ValueSource;
use clap::{Command, CommandFactory, FromArgMatches, Parser};
use eframe::wgpu::PresentMode;
use photocraft_automation::args::{AutomationArgs, ControlArgs, blank_env_is_unset, conflict, control_port, name_env_vars};

use crate::presentation;

/// Opens PhotoCraft with the given files
#[derive(Parser)]
#[command(name = "photocraft", version = photocraft_engine::build_info::long_version_static(), after_help = ENVIRONMENT)]
pub(crate) struct LaunchArgs {
    /// Files to open. On Windows and Linux they open as new tabs in the PhotoCraft already running.
    /// Put `--` before a name that starts with `-`
    pub(crate) files: Vec<PathBuf>,
    /// Start with the CPU renderer for this launch (after a graphics driver crash)
    #[arg(long)]
    pub(crate) safe_gpu: bool,
    /// macOS: keep the menus inside the window instead of the menu bar (the variable: 0, false, no
    /// or off is off; empty is unset)
    #[arg(long, env = "PHOTOCRAFT_IN_WINDOW_MENUS", value_parser = FalseyValueParser::new())]
    pub(crate) in_window_menus: bool,
    /// Windows and Linux: start a separate PhotoCraft instead of opening the files in the one
    /// already running
    #[arg(long)]
    pub(crate) new_instance: bool,
    /// How frames reach the display: fifo, autovsync or novsync (also immediate or mailbox).
    /// Without it, PHOTOCRAFT_PRESENT_MODE
    #[arg(long, value_name = "MODE", value_parser = present_mode)]
    pub(crate) present_mode: Option<PresentMode>,
    /// Frames queued for the display, 1 to 3. Without it, PHOTOCRAFT_FRAME_LATENCY
    // allow_negative_numbers makes `--frame-latency -1` a bad value rather than an unexpected argument.
    #[arg(long, value_name = "1-3", value_parser = frame_latency, allow_negative_numbers = true)]
    pub(crate) frame_latency: Option<u32>,
    /// Time each brush stroke's frames: stderr prints a summary per stroke, a CSV file path also
    /// records every frame. Without it, PHOTOCRAFT_STROKE_TRACE
    #[arg(long, value_name = "stderr|FILE.csv")]
    pub(crate) stroke_trace: Option<String>,
    /// Start the localhost JSON-lines control server on this port
    #[arg(long, value_name = "PORT", env = "PHOTOCRAFT_CONTROL_PORT", value_parser = control_port, allow_negative_numbers = true)]
    pub(crate) control: Option<u16>,
    #[command(flatten)]
    pub(crate) auth: ControlArgs,
    #[command(flatten)]
    pub(crate) automation: AutomationArgs,
    /// The per-process options given on the command line, set by [`parse`].
    #[arg(skip)]
    pub(crate) launch_options: Vec<PerProcess>,
}

/// The variables that aren't a flag's fallback, for the end of `--help`.
const ENVIRONMENT: &str = "\
Environment: PHOTOCRAFT_NEW_INSTANCE (anything but 0 or empty: as --new-instance), PHOTOCRAFT_CONFIG_DIR (the settings folder)";

/// An option that applies only to the process it's given to: a launch with any of them keeps its
/// own process instead of handing its files to the running one (`single_instance`). Shown as the
/// flag, or the variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum PerProcess {
    AutomationReadRoot,
    AutomationWriteRoot,
    SafeGpu,
    InWindowMenus,
    PresentMode,
    FrameLatency,
    StrokeTrace,
    /// PHOTOCRAFT_GPU_ADAPTER, when startup will act on it (`gpu_startup`). Not a flag.
    GpuAdapterEnv,
}

impl PerProcess {
    /// The flags, in the order a log line lists them. A flag's variable asks for the same thing, so
    /// it counts too (#7242); the variables that aren't a flag's fallback are counted by
    /// `single_instance::env_launch_options`.
    pub(crate) const FLAGS: [PerProcess; 7] =
        [Self::AutomationReadRoot, Self::AutomationWriteRoot, Self::SafeGpu, Self::InWindowMenus, Self::PresentMode, Self::FrameLatency, Self::StrokeTrace];

    /// The flag's clap argument id.
    fn id(self) -> Option<&'static str> {
        Some(match self {
            Self::AutomationReadRoot => "automation_read_root",
            Self::AutomationWriteRoot => "automation_write_root",
            Self::SafeGpu => "safe_gpu",
            Self::InWindowMenus => "in_window_menus",
            Self::PresentMode => "present_mode",
            Self::FrameLatency => "frame_latency",
            Self::StrokeTrace => "stroke_trace",
            Self::GpuAdapterEnv => return None,
        })
    }
}

impl fmt::Display for PerProcess {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::AutomationReadRoot => "--automation-read-root",
            Self::AutomationWriteRoot => "--automation-write-root",
            Self::SafeGpu => "--safe-gpu",
            Self::InWindowMenus => "--in-window-menus",
            Self::PresentMode => "--present-mode",
            Self::FrameLatency => "--frame-latency",
            Self::StrokeTrace => "--stroke-trace",
            Self::GpuAdapterEnv => "PHOTOCRAFT_GPU_ADAPTER",
        })
    }
}

/// The files to open, as [`LaunchArgs::file_names`] sorts them.
#[derive(Debug, PartialEq)]
pub(crate) struct FileNames {
    pub(crate) openable: Vec<String>,
    /// Names that are not valid Unicode, shown lossily: reported, not opened.
    pub(crate) unreadable: Vec<String>,
}

impl LaunchArgs {
    /// The files to open, and those whose names are not valid Unicode (#1108): a file manager can
    /// hand over a Latin-1 name on Linux. Those are reported in a notice once the app is up.
    pub(crate) fn file_names(&self) -> FileNames {
        let mut names = FileNames { openable: Vec::new(), unreadable: Vec::new() };
        for f in &self.files {
            match f.to_str() {
                Some(s) => names.openable.push(s.to_owned()),
                None => names.unreadable.push(f.to_string_lossy().into_owned()),
            }
        }
        names
    }
}

/// The command line, with the desktop app's variables for the automation roots. An empty or blank
/// variable counts as unset.
pub(crate) fn command() -> Command {
    LaunchArgs::command()
        .mut_arg("automation_read_root", |a| a.env("PHOTOCRAFT_AUTOMATION_READ_ROOT"))
        .mut_arg("automation_write_root", |a| a.env("PHOTOCRAFT_AUTOMATION_WRITE_ROOT"))
        .mut_args(blank_env_is_unset)
}

/// Parse the launch arguments (without the binary name) with `cmd`, from [`command`]. Old macOS
/// passes a process serial number (`-psn_…`) when launched from Finder: it is dropped first, unless
/// it comes after `--`. A bad value from a variable is a usage error naming the variable.
pub(crate) fn parse(mut cmd: Command, args: impl IntoIterator<Item = OsString>) -> Result<LaunchArgs, clap::Error> {
    let mut options = true;
    let args: Vec<OsString> = args
        .into_iter()
        .filter(|a| {
            options &= a != "--";
            !(options && a.to_str().is_some_and(|s| s.starts_with("-psn_")))
        })
        .collect();
    let argv = std::iter::once(OsString::from("photocraft")).chain(args.iter().cloned());
    let matches = cmd.try_get_matches_from_mut(argv).map_err(|e| name_env_vars(e, &cmd, &args))?;
    let mut launch = LaunchArgs::from_arg_matches(&matches)?;
    // Only the control server reads a token, so a stale exported pair doesn't stop a launch.
    if launch.control.is_some() {
        conflict(&mut cmd, &matches, "control_token", "control_token_file")?;
    }
    // `PHOTOCRAFT_IN_WINDOW_MENUS=0` (or off, false, no) is a variable that turns the menus off.
    let menus = launch.in_window_menus;
    let given = |p: &PerProcess| match p.id().and_then(|id| matches.value_source(id)) {
        Some(ValueSource::CommandLine) => true,
        Some(ValueSource::EnvVariable) => *p != PerProcess::InWindowMenus || menus,
        _ => false,
    };
    launch.launch_options = PerProcess::FLAGS.into_iter().filter(given).collect();
    Ok(launch)
}

/// Why a value was refused: clap prints it after the flag and its value (exit 2).
#[derive(Debug, thiserror::Error)]
enum ValueError {
    #[error("expected fifo, autovsync or novsync (or immediate, mailbox)")]
    PresentMode,
    #[error("expected 1, 2 or 3")]
    FrameLatency,
}

fn present_mode(s: &str) -> Result<PresentMode, ValueError> {
    presentation::present_mode(s).ok_or(ValueError::PresentMode)
}

fn frame_latency(s: &str) -> Result<u32, ValueError> {
    presentation::frame_latency(s).ok_or(ValueError::FrameLatency)
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::error::ErrorKind;

    /// Parse without the variables, so one exported where the tests run can't change them (the
    /// process tests in `tests/command_line_usage.rs` cover the variables).
    fn parse_args(args: &[&str]) -> Result<LaunchArgs, clap::Error> {
        parse(command().mut_args(|a| a.env(None::<&str>)), args.iter().map(OsString::from))
    }

    fn usage_error(args: &[&str]) -> String {
        let Err(e) = parse_args(args) else { panic!("{args:?} parsed") };
        assert_eq!(e.exit_code(), 2, "{args:?}: {e}");
        e.to_string()
    }

    #[test]
    fn control_port_rejects_unparseable_values_naming_them() {
        // Out of range, negative and empty values fail, naming the value (#701); a following
        // flag is never taken as the value.
        assert!(usage_error(&["--control", "78787"]).contains("invalid value '78787' for '--control <PORT>'"));
        assert!(usage_error(&["--control", "-1"]).contains("invalid value '-1'"));
        assert!(usage_error(&["--control="]).contains("invalid value '' for '--control <PORT>'"));
        assert!(usage_error(&["--control", "--safe-gpu"]).contains("a value is required for '--control <PORT>'"));
    }

    #[test]
    fn flags_without_a_value_are_usage_errors() {
        // #6429: a missing value fails the launch, rather than falling back to a generated token
        // or to no root. The tuning flags too, now (#7071).
        for flag in [
            "--control",
            "--control-token",
            "--control-token-file",
            "--automation-read-root",
            "--automation-write-root",
            "--present-mode",
            "--frame-latency",
            "--stroke-trace",
        ] {
            let err = usage_error(&[flag]);
            assert!(err.contains(&format!("a value is required for '{flag} <")), "{flag}: {err}");
        }
    }

    #[test]
    fn bad_tuning_values_and_repeats_are_usage_errors() {
        assert!(usage_error(&["--present-mode", "fast"]).contains("expected fifo, autovsync or novsync"));
        for latency in ["0", "4", "-1", "one"] {
            let err = usage_error(&["--frame-latency", latency]);
            assert!(err.contains(&format!("invalid value '{latency}' for '--frame-latency <1-3>'")), "{err}");
        }
        assert!(usage_error(&["--safe-gpu", "--safe-gpu"]).contains("cannot be used multiple times"));
        assert!(usage_error(&["--control", "1", "--control", "2"]).contains("cannot be used multiple times"));
        // A token and a token file are either/or for the control server, and ignored without it.
        let pair = ["--control-token", "abcd", "--control-token-file", "/tmp/token"];
        assert!(usage_error(&[&["--control", "1"][..], &pair[..]].concat()).contains("cannot be used with"));
        assert!(parse_args(&pair).is_ok());
    }

    #[test]
    fn values_are_parsed_and_per_process_options_recorded() {
        let parsed = parse_args(&[
            "--stroke-trace",
            "stderr",
            "--control",
            "50494",
            "--control-token",
            "abcd",
            "--automation-read-root",
            "/r",
            "--automation-write-root",
            "/w",
            "--safe-gpu",
            "--in-window-menus",
            "--new-instance",
            "--present-mode",
            "Mailbox",
            "--frame-latency",
            "1",
            "photo.psd",
        ])
        .unwrap();
        assert_eq!(parsed.control, Some(50494));
        assert_eq!(parsed.auth.control_token.as_deref(), Some("abcd"));
        assert_eq!(parsed.auth.control_token_file, None);
        assert_eq!(parsed.automation.automation_read_root.as_deref(), Some(std::path::Path::new("/r")));
        assert_eq!(parsed.automation.automation_write_root.as_deref(), Some(std::path::Path::new("/w")));
        assert!(parsed.safe_gpu && parsed.in_window_menus && parsed.new_instance);
        assert_eq!(parsed.present_mode, Some(PresentMode::AutoNoVsync));
        assert_eq!(parsed.frame_latency, Some(1));
        assert_eq!(parsed.stroke_trace.as_deref(), Some("stderr"));
        // In a fixed order: `single_instance` only lists them in a log line.
        assert_eq!(parsed.launch_options, PerProcess::FLAGS);
        assert_eq!(parsed.file_names(), FileNames { openable: vec!["photo.psd".to_string()], unreadable: vec![] });

        let parsed = parse_args(&["--control-token-file", "/tmp/token", "--new-instance"]).unwrap();
        assert_eq!(parsed.auth.control_token_file.as_deref(), Some(std::path::Path::new("/tmp/token")));
        assert!(parsed.launch_options.is_empty() && !parsed.in_window_menus, "{:?}", parsed.launch_options);
    }

    #[test]
    fn version_stops_parsing() {
        let e = parse_args(&["--version", "--control"]).err().unwrap();
        assert_eq!((e.kind(), e.exit_code()), (ErrorKind::DisplayVersion, 0));
        assert_eq!(e.to_string(), format!("photocraft {}\n", photocraft_engine::build_info::long_version()));
    }

    #[test]
    fn help_stops_parsing_and_double_dash_ends_options() {
        let e = parse_args(&["-h", "--safegpu", "--control"]).err().unwrap();
        assert_eq!((e.kind(), e.exit_code()), (ErrorKind::DisplayHelp, 0));

        assert!(usage_error(&["--safegpu", "--", "--help"]).contains("unexpected argument '--safegpu'"));
        let parsed = parse_args(&["a.psd", "--", "--help", "-x"]).unwrap();
        assert_eq!(parsed.file_names().openable, ["a.psd", "--help", "-x"]);
    }

    #[test]
    fn a_finder_process_serial_number_is_dropped() {
        let parsed = parse_args(&["-psn_0_12345", "a.psd"]).unwrap();
        assert_eq!(parsed.file_names().openable, ["a.psd"]);
        let parsed = parse_args(&["--", "-psn_x.psd"]).unwrap();
        assert_eq!(parsed.file_names().openable, ["-psn_x.psd"]);
    }

    /// #1108: a name that isn't valid Unicode is reported, not opened; a token must be text.
    #[cfg(unix)]
    #[test]
    fn non_unicode_file_names_are_reported_and_tokens_rejected() {
        use std::os::unix::ffi::OsStringExt as _;
        let latin1 = OsString::from_vec(b"caf\xe9.psd".to_vec());
        let parsed = parse(command().mut_args(|a| a.env(None::<&str>)), [latin1.clone()]).unwrap();
        assert_eq!(parsed.file_names(), FileNames { openable: vec![], unreadable: vec!["caf\u{fffd}.psd".to_string()] });
        let e = parse(command().mut_args(|a| a.env(None::<&str>)), [OsString::from("--control-token"), latin1]).err().unwrap();
        assert_eq!((e.kind(), e.exit_code()), (ErrorKind::InvalidUtf8, 2));
    }
}
