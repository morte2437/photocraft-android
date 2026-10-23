//! PhotoCraft desktop app.
//!
//! Usage: `photocraft [--control <port>] [--control-token <64-hex> |
//! --control-token-file <path>] [--automation-read-root <dir>]
//! [--automation-write-root <dir>] [--safe-gpu] [--in-window-menus] [--present-mode <mode>]
//! [--frame-latency <n>] [--stroke-trace <stderr|file.csv>] [--new-instance] [--] [files…]`
//! (`photocraft --help` prints [`USAGE`]; `--version` prints the version).
//!
//! Any other argument starting with `-` is a usage error (exit status 2, issue #6431), so a typo
//! such as `--safegpu` doesn't open the window and then fail to open the flag as a file. A file
//! whose name starts with `-` goes after `--`.
//!
//! On Windows and Linux a launch hands its files to the PhotoCraft that is already running (new
//! tabs in its window) and exits; `--new-instance` starts a separate copy instead, as does any
//! `--control` launch (see `single_instance`).
//!
//! `--present-mode` (`fifo`, `autovsync`, `novsync`) and `--frame-latency` (1–3) override how
//! frames reach the display for this launch (`PHOTOCRAFT_PRESENT_MODE` / `…_FRAME_LATENCY`;
//! `presentation`); `--stroke-trace` times each brush stroke's frames (`PHOTOCRAFT_STROKE_TRACE`;
//! `photocraft_ui_egui::stroke_trace`).
//!
//! `--safe-gpu` starts with the CPU renderer (no GPU canvas; a software adapter for the window
//! where the platform has one) for this launch, e.g. after a graphics driver crash. A start that
//! crashes inside the driver also falls back by itself next time (see `gpu_startup`).
//!
//! `--in-window-menus` (or `PHOTOCRAFT_IN_WINDOW_MENUS=1`) keeps the menus inside the window on
//! macOS instead of the macOS menu bar (`mac_menu`).
//!
//! `--control <port>` (or `PHOTOCRAFT_CONTROL_PORT`) starts a localhost JSON-lines control server.
//! The first line must authenticate; subsequent request lines get reply lines.
//! `{"id":1,"ok":true,"result":…}`. See `photocraft_ui_egui::control` for the methods.

// Release builds on Windows are GUI-subsystem apps, so launching from the Start Menu or Explorer
// doesn't open a console window. (`--version` output then only shows when redirected.)
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod app_dirs;
mod app_icon;
mod appearance;
#[cfg(target_os = "macos")]
mod apple_events;
mod caps_lock;
mod caption_style;
mod control_server;
mod crash_guard;
mod cursor;
mod file_dialog_probe;
mod gpu_startup;
#[cfg(target_os = "linux")]
mod icon_theme;
#[cfg(target_os = "macos")]
mod mac_menu;
#[cfg(target_os = "macos")]
mod mac_window;
// Pure logic is tested on every platform; only Linux runs the check.
#[cfg(any(target_os = "linux", test))]
mod linux_libs;
mod logging;
mod monitor_profile;
mod photoshop_settings;
// Save As through the desktop portal, which reports the file type picked (#6121).
#[cfg(target_os = "linux")]
mod portal_save;
mod presentation;
// The print portal needs the session bus (zbus, Linux only); its pure helpers are tested everywhere.
#[cfg(any(target_os = "linux", test))]
mod print_portal;
mod screen_color;
mod services;
// macOS gets one instance from LaunchServices (open-documents Apple events).
#[cfg(any(not(target_os = "macos"), test))]
mod single_instance;
// Tablet support (macOS AppKit monitor, Linux X11/Wayland, Windows barrel button).
mod tablet;
mod ui_state;
mod window_frame;

use photocraft_engine::Session;
use photocraft_ui_egui::PhotocraftApp;

/// Matches the `.desktop` file and hicolor icon name, so Wayland docks pick up the icon.
const APP_ID: &str = "ai.storyteller.photocraft";

/// Windows and Linux: no OS title bar; the app's top bar is the title bar, with its own caption
/// buttons and edge resizing (`photocraft_ui_egui::titlebar`), as Photoshop does on Windows. macOS
/// keeps its traffic lights over the integrated title strip.
const CUSTOM_TITLEBAR: bool = !cfg!(target_os = "macos");

/// Whether this start draws its own title bar: Windows and Linux do, unless Preferences ›
/// Interface › System Title Bar asks for the system's (#1271, #1316). Read from the saved
/// preferences before the window opens; a missing or unreadable file keeps the default.
fn custom_titlebar(prefs_file: Option<&std::path::Path>) -> bool {
    let prefs: photocraft_engine::prefs::Preferences =
        prefs_file.and_then(|p| std::fs::read_to_string(p).ok()).and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
    CUSTOM_TITLEBAR && !prefs.interface.system_title_bar
}

/// The main window: 1440 × 900 (shrunk to fit the monitor, and maximized on the first frame
/// when it still doesn't fit, `work_area::fit_window`), centred on the main monitor. Without
/// `centered`, Windows cascades each new window from the top-left corner, so it opened at a
/// different offset every launch (#419). A later launch restores the saved position instead
/// (`main` turns `centered` off, #4826). Wayland compositors place windows themselves.
fn native_options(custom_titlebar: bool) -> eframe::NativeOptions {
    let viewport = egui::ViewportBuilder::default().with_icon(app_icon::viewport_icon(std::env::current_exe().ok().as_deref()));
    eframe::NativeOptions {
        viewport: viewport
            .with_app_id(APP_ID)
            .with_title("PhotoCraft")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([760.0, 480.0])
            .with_drag_and_drop(true)
            .with_decorations(!custom_titlebar)
            .with_fullsize_content_view(true)
            .with_titlebar_shown(false)
            .with_title_shown(false),
        centered: true,
        // eframe saves native window geometry and egui panel/window sizes on exit.
        // Keep that state beside preferences, including config overrides and portable mode.
        persistence_path: services::config_dir().map(|dir| dir.join("ui.ron")),
        window_builder: Some(Box::new(restored_window)),
        ..Default::default()
    }
}

/// The window eframe restores from `ui.ron`, corrected before it is created: at least the
/// minimum size, and never full screen. PhotoCraft always starts in Standard Screen Mode, but
/// eframe also saves and restores full screen, and a window closed in a full-screen mode is
/// saved full screen and maximized at once: the overlap `view_cmds::screen_mode_commands` keeps
/// apart at run time. [OBSERVED 2026-10-12, Windows 11] Restored that way, View › Screen Mode ›
/// Full Screen did nothing until `F` had cycled through Standard.
fn restored_window(builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    at_least_min_size(builder).with_fullscreen(false)
}

/// Grow a restored window that is smaller than the minimum size, keeping its centre. On
/// Windows, eframe saves a minimized window's size (160 pt wide, 64 pt tall once eframe has
/// clamped it), and the minimum size isn't applied to a restored one: PhotoCraft then reopened
/// as a 160 × 64 strip on every launch.
fn at_least_min_size(mut builder: egui::ViewportBuilder) -> egui::ViewportBuilder {
    let (Some(size), Some(min)) = (builder.inner_size, builder.min_inner_size) else { return builder };
    let grown = size.max(min);
    if grown != size {
        builder.position = builder.position.map(|pos| pos - (grown - size) / 2.0);
        builder.inner_size = Some(grown);
    }
    builder
}

/// Parse a `--control` / `PHOTOCRAFT_CONTROL_PORT` value. An unparseable port is an error that
/// names the value (issue #701): silently running with no control server leaves a launcher or
/// agent unable to tell a typo from a successful grant.
fn parse_control_port(value: &str, source: &str) -> Result<u16, String> {
    value.trim().parse().map_err(|_| format!("{source}: `{value}` is not a valid port (expected a number from 0 to 65535)"))
}

/// Process exit status for the collected usage errors (a bad `--control` /
/// `PHOTOCRAFT_CONTROL_PORT`, an unknown option): `None` when there are none, else 2 (a
/// command-line usage error), never 0.
fn usage_exit_code(errors: &[String]) -> Option<i32> {
    if errors.is_empty() { None } else { Some(2) }
}

/// `photocraft --help`. Keep in step with the options `main` accepts and the module docs.
const USAGE: &str = "\
Usage: photocraft [options] [--] [files…]

Opens PhotoCraft with the given files.

Options:
  -h, --help                       Print this help and exit
      --version                    Print the version, commit and build date and exit
      --safe-gpu                   Start with the CPU renderer for this launch (after a graphics driver crash)
      --in-window-menus            macOS: keep the menus inside the window instead of the menu bar
      --new-instance               Windows and Linux: start a separate PhotoCraft instead of opening
                                   the files in the one already running
      --present-mode <mode>        How frames reach the display: fifo, autovsync or novsync
      --frame-latency <n>          Frames queued for the display: 1 to 3
      --stroke-trace <stderr|file.csv>
                                   Time each brush stroke's frames
      --control <port>             Start the localhost JSON-lines control server on this port
      --control-token <64-hex>     Control server token (default: a new random one, printed)
      --control-token-file <path>  Read the control server token from this file, or create it
                                   with a new one
      --automation-read-root <dir> Directory the control server may read files from
      --automation-write-root <dir>
                                   Directory the control server may write files to
  --                               Treat every later argument as a file, even one starting with `-`

A bad --present-mode, --frame-latency or --stroke-trace value is reported and ignored.
Environment: PHOTOCRAFT_CONTROL_PORT, PHOTOCRAFT_CONTROL_TOKEN, PHOTOCRAFT_CONTROL_TOKEN_FILE,
PHOTOCRAFT_AUTOMATION_READ_ROOT, PHOTOCRAFT_AUTOMATION_WRITE_ROOT, PHOTOCRAFT_PRESENT_MODE,
PHOTOCRAFT_FRAME_LATENCY, PHOTOCRAFT_STROKE_TRACE, PHOTOCRAFT_IN_WINDOW_MENUS, PHOTOCRAFT_NEW_INSTANCE,
PHOTOCRAFT_CONFIG_DIR.
";

/// Print to standard output for `--help` / `--version`. Unlike `println!`, never panics when
/// stdout is closed or a pipe (`photocraft --help | head -1`).
fn print_stdout(text: &str) {
    use std::io::Write as _;
    let mut out = std::io::stdout().lock();
    let _ = out.write_all(text.as_bytes()).and_then(|()| out.flush());
}

/// Everything the desktop launcher takes from the command line, on top of the environment
/// defaults handed in to `parse_launch_args`.
#[derive(Debug, Default, PartialEq, Eq)]
struct LaunchArgs {
    control_port: Option<u16>,
    control_token: Option<String>,
    control_token_file: Option<std::path::PathBuf>,
    automation_read_root: Option<std::path::PathBuf>,
    automation_write_root: Option<std::path::PathBuf>,
    files: Vec<String>,
    /// Arguments that are not valid Unicode (issue #1108): a file manager can hand over a Latin-1
    /// file name on Linux, and `std::env::args()` would panic before any window opened. Paths are
    /// reported in a notice once the app is up; a control token is a usage error like a bad port.
    unreadable_paths: Vec<String>,
    safe_gpu: bool,
    new_instance: bool,
    /// Options that apply only to the process they're given to; a launch with any keeps its own.
    launch_options: Vec<&'static str>,
    present_mode_arg: Option<String>,
    frame_latency_arg: Option<String>,
    stroke_trace_arg: Option<String>,
    in_window_menus: bool,
    /// `--help` / `-h`: print [`USAGE`] and exit 0; parsing stops there.
    help: bool,
    /// `--version`: print the version and exit 0; parsing stops there.
    version: bool,
    /// Usage errors (a bad or missing control/automation value, an unknown option); a non-empty
    /// list fails the launch with exit status 2 (issues #701, #6429, #6431).
    errors: Vec<String>,
}

/// Parse the desktop launch arguments over the environment defaults in `out`. A control or
/// automation flag with no value is a usage error naming the flag (issue #6429), never a silent
/// fall back to a generated token or to no root: like `--control` with a bad port, it exits 2.
/// So is an unknown option (#6431); a file whose name starts with `-` goes after `--`.
fn parse_launch_args(args: impl Iterator<Item = std::ffi::OsString>, mut out: LaunchArgs) -> LaunchArgs {
    let mut args = args;
    // After `--`, every argument is a file, even one that starts with `-`.
    let mut only_files = false;
    while let Some(a) = args.next() {
        if only_files {
            match a.into_string() {
                Ok(s) => out.files.push(s),
                Err(raw) => out.unreadable_paths.push(raw.to_string_lossy().into_owned()),
            }
            continue;
        }
        match a.to_str() {
            Some("--") => only_files = true,
            Some("--help" | "-h") => {
                out.help = true;
                break;
            }
            Some("--control") => match args.next() {
                Some(value) => match parse_control_port(&value.to_string_lossy(), "--control") {
                    Ok(port) => out.control_port = Some(port),
                    Err(error) => out.errors.push(error),
                },
                None => out.errors.push("--control: missing port value (expected `--control <port>`)".to_string()),
            },
            Some("--control-token") => match args.next().map(std::ffi::OsString::into_string) {
                Some(Ok(token)) => out.control_token = Some(token),
                Some(Err(raw)) => out.errors.push(format!("--control-token: value is not valid Unicode (`{}`)", raw.to_string_lossy())),
                None => out.errors.push("--control-token: missing token value (expected `--control-token <token>`)".to_string()),
            },
            Some("--control-token-file") => match args.next() {
                Some(value) => out.control_token_file = Some(std::path::PathBuf::from(value)),
                None => out.errors.push("--control-token-file: missing file path value (expected `--control-token-file <path>`)".to_string()),
            },
            Some("--automation-read-root") => {
                out.launch_options.push("--automation-read-root");
                match args.next() {
                    Some(value) => out.automation_read_root = Some(std::path::PathBuf::from(value)),
                    None => out.errors.push("--automation-read-root: missing directory value (expected `--automation-read-root <dir>`)".to_string()),
                }
            }
            Some("--automation-write-root") => {
                out.launch_options.push("--automation-write-root");
                match args.next() {
                    Some(value) => out.automation_write_root = Some(std::path::PathBuf::from(value)),
                    None => out.errors.push("--automation-write-root: missing directory value (expected `--automation-write-root <dir>`)".to_string()),
                }
            }
            Some("--safe-gpu") => {
                out.launch_options.push("--safe-gpu");
                out.safe_gpu = true;
            }
            Some(flag @ ("--present-mode" | "--frame-latency" | "--stroke-trace")) => {
                out.launch_options.push(match flag {
                    "--present-mode" => "--present-mode",
                    "--frame-latency" => "--frame-latency",
                    _ => "--stroke-trace",
                });
                let flag = flag.to_string();
                match args.next().map(std::ffi::OsString::into_string) {
                    Some(Ok(v)) => match flag.as_str() {
                        "--present-mode" => out.present_mode_arg = Some(v),
                        "--frame-latency" => out.frame_latency_arg = Some(v),
                        _ => out.stroke_trace_arg = Some(v),
                    },
                    // A bad value is reported and ignored; it never fails the launch.
                    _ => eprintln!("photocraft: {flag}: missing or unreadable value; ignored"),
                }
            }
            Some("--in-window-menus") => {
                out.launch_options.push("--in-window-menus");
                out.in_window_menus = true;
            }
            Some("--new-instance") => out.new_instance = true,
            Some("--version") => {
                out.version = true;
                break;
            }
            // Old macOS passes a process serial number when launched from Finder.
            Some(s) if s.starts_with("-psn_") => {}
            // A typo such as `--safegpu`, or `--help` spelt another way, must not open the window
            // and then fail to open the flag as a file (#6431).
            Some(s) if s.starts_with('-') => out.errors.push(format!("unknown option `{s}` (put `--` before a file name that starts with `-`)")),
            Some(s) => out.files.push(s.to_owned()),
            None => out.unreadable_paths.push(a.to_string_lossy().into_owned()),
        }
    }
    out
}

#[cfg(test)]
mod control_port_tests {
    use super::{parse_control_port, usage_exit_code};

    #[test]
    fn control_port_errors_exit_non_zero() {
        assert_eq!(usage_exit_code(&[]), None);
        let errors = vec![parse_control_port("nope", "--control").unwrap_err()];
        assert_eq!(usage_exit_code(&errors), Some(2));
    }

    #[test]
    fn control_port_accepts_valid_numbers() {
        assert_eq!(parse_control_port("50494", "--control"), Ok(50494));
        assert_eq!(parse_control_port(" 8080 ", "--control"), Ok(8080));
        assert_eq!(parse_control_port("0", "PHOTOCRAFT_CONTROL_PORT"), Ok(0));
    }

    #[test]
    fn control_port_rejects_unparseable_values_naming_them() {
        // Out of range, a following flag eaten as the value, and an empty value all fail,
        // naming the offending value in the message.
        let err = parse_control_port("78787", "--control").unwrap_err();
        assert!(err.contains("78787"), "{err}");
        assert!(parse_control_port("--safe-gpu", "--control").is_err());
        assert!(parse_control_port("", "--control").is_err());
    }
}

#[cfg(test)]
mod launch_args_tests {
    use super::{LaunchArgs, parse_launch_args, usage_exit_code};
    use std::ffi::OsString;

    fn parse(values: &[&str]) -> LaunchArgs {
        parse_launch_args(values.iter().map(OsString::from), LaunchArgs::default())
    }

    #[test]
    fn flags_without_a_value_are_usage_errors() {
        // Issue #6429: a missing value must fail the launch, not silently fall back to a
        // generated token or to no root.
        for flag in ["--control", "--control-token", "--control-token-file", "--automation-read-root", "--automation-write-root"] {
            let parsed = parse(&[flag]);
            assert_eq!(parsed.errors.len(), 1, "{flag}: {:?}", parsed.errors);
            assert!(parsed.errors[0].starts_with(flag), "{}", parsed.errors[0]);
            assert!(parsed.errors[0].contains("missing"), "{}", parsed.errors[0]);
            assert_eq!(usage_exit_code(&parsed.errors), Some(2));
        }
    }

    #[test]
    fn missing_value_does_not_clear_an_environment_default() {
        // `--automation-read-root` with no value must not drop a root set through the env var.
        let defaults = LaunchArgs { automation_read_root: Some("/env/read".into()), automation_write_root: Some("/env/write".into()), ..Default::default() };
        let parsed = parse_launch_args(["--automation-read-root"].iter().map(OsString::from), defaults);
        assert_eq!(parsed.automation_read_root.as_deref(), Some(std::path::Path::new("/env/read")));
        assert_eq!(parsed.automation_write_root.as_deref(), Some(std::path::Path::new("/env/write")));
        assert_eq!(parsed.errors.len(), 1, "{:?}", parsed.errors);
    }

    #[test]
    fn values_are_parsed_and_no_errors_remain() {
        let parsed = parse(&[
            "--control",
            "50494",
            "--control-token",
            "abcd",
            "--control-token-file",
            "/tmp/token",
            "--automation-read-root",
            "/r",
            "--automation-write-root",
            "/w",
            "--safe-gpu",
            "--in-window-menus",
            "--new-instance",
            "--present-mode",
            "mailbox",
            "photo.psd",
        ]);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
        assert_eq!(parsed.control_port, Some(50494));
        assert_eq!(parsed.control_token.as_deref(), Some("abcd"));
        assert_eq!(parsed.control_token_file.as_deref(), Some(std::path::Path::new("/tmp/token")));
        assert_eq!(parsed.automation_read_root.as_deref(), Some(std::path::Path::new("/r")));
        assert_eq!(parsed.automation_write_root.as_deref(), Some(std::path::Path::new("/w")));
        assert!(parsed.safe_gpu);
        assert!(parsed.in_window_menus);
        assert!(parsed.new_instance);
        assert_eq!(parsed.present_mode_arg.as_deref(), Some("mailbox"));
        assert_eq!(parsed.launch_options, vec!["--automation-read-root", "--automation-write-root", "--safe-gpu", "--in-window-menus", "--present-mode"]);
        assert_eq!(parsed.files, vec!["photo.psd".to_string()]);
    }

    #[test]
    fn version_stops_parsing() {
        let parsed = parse(&["--version", "--control"]);
        assert!(parsed.version);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);
    }

    #[test]
    fn help_stops_parsing_and_double_dash_ends_options() {
        let parsed = parse(&["-h", "--safegpu", "--control"]);
        assert!(parsed.help);
        assert!(parsed.errors.is_empty(), "{:?}", parsed.errors);

        let parsed = parse(&["--safegpu", "--", "--help", "-x"]);
        assert!(!parsed.help);
        assert_eq!(parsed.errors.len(), 1, "{:?}", parsed.errors);
        assert!(parsed.errors[0].contains("unknown option `--safegpu`"), "{}", parsed.errors[0]);
        assert_eq!(parsed.files, vec!["--help".to_string(), "-x".to_string()]);
    }
}

fn main() -> eframe::Result {
    // First, so the panic hook and every start-up warning are recorded (`logging`).
    let logger = logging::install();
    crash_guard::install_hook();
    // The system font scan can take seconds (minutes on some Macs): run it off the UI thread.
    photocraft_ui_egui::type_tool::warm_fonts();
    let mut launch = LaunchArgs {
        automation_read_root: std::env::var_os("PHOTOCRAFT_AUTOMATION_READ_ROOT").map(std::path::PathBuf::from),
        automation_write_root: std::env::var_os("PHOTOCRAFT_AUTOMATION_WRITE_ROOT").map(std::path::PathBuf::from),
        in_window_menus: std::env::var_os("PHOTOCRAFT_IN_WINDOW_MENUS").is_some_and(|v| !v.is_empty() && v != "0"),
        ..Default::default()
    };
    if let Some(value) = std::env::var("PHOTOCRAFT_CONTROL_PORT").ok().filter(|v| !v.trim().is_empty()) {
        match parse_control_port(&value, "PHOTOCRAFT_CONTROL_PORT") {
            Ok(port) => launch.control_port = Some(port),
            Err(error) => launch.errors.push(error),
        }
    }
    launch = parse_launch_args(std::env::args_os().skip(1), launch);
    if launch.help {
        print_stdout(USAGE);
        return Ok(());
    }
    if launch.version {
        print_stdout(&format!("photocraft {}\n", photocraft_engine::build_info::long_version()));
        return Ok(());
    }

    // A malformed control port must not silently drop the control server (issue #701), a control
    // or automation flag without its value must not silently fall back to a generated token or
    // to no root (issue #6429), and an unknown option must not open the window (#6431): name the
    // bad argument and fail the launch, like `photocraft-cli` does for the same typos. Exit
    // status 2 is the usual command-line usage error, so a launcher sees the failure.
    if let Some(code) = usage_exit_code(&launch.errors) {
        for error in &launch.errors {
            eprintln!("photocraft: {error}");
        }
        eprintln!("Run `photocraft --help` for usage.");
        std::process::exit(code);
    }
    let LaunchArgs {
        control_port,
        control_token,
        control_token_file,
        automation_read_root,
        automation_write_root,
        files,
        unreadable_paths,
        safe_gpu,
        new_instance,
        mut launch_options,
        present_mode_arg,
        frame_latency_arg,
        stroke_trace_arg,
        in_window_menus,
        help: _,
        version: _,
        errors: _,
    } = launch;

    // The log file lives under the settings directory; opened after the arguments, so `--version`
    // and usage errors leave no file behind. Records logged until now are written to it first.
    if let (Some(logger), Some(dir)) = (logger, services::config_dir()) {
        match logger.attach_dir(&dir.join("logs")) {
            Ok(path) => log::info!("PhotoCraft {}, log file {}", photocraft_engine::build_info::long_version(), path.display()),
            // Standard error only by now (`attach_dir` gave up on the file); unlike `eprintln!`, never panics.
            Err(e) => log::warn!("no log file: {e}"),
        }
    }

    // An adapter named in the environment is chosen at startup, which a forwarded launch never reaches.
    let rendering_mode = gpu_startup::read_rendering_prefs(services::prefs_file().as_deref()).1;
    if gpu_startup::adapter_override_is_process_specific(std::env::var("PHOTOCRAFT_GPU_ADAPTER").ok().as_deref(), rendering_mode, gpu_startup::ADAPTER_CHOICE) {
        launch_options.push("PHOTOCRAFT_GPU_ADAPTER");
    }
    // One PhotoCraft per user (#3708): files opened from Explorer or a file manager while it runs
    // become tabs in its window. Only with the per-user settings folder (a portable copy's may be
    // shared between users), and not for automation, `--new-instance` or per-process options.
    #[cfg(not(target_os = "macos"))]
    let single_instance = {
        let new_instance_env = std::env::var("PHOTOCRAFT_NEW_INSTANCE").ok();
        // A variable documented as the equivalent of a per-launch flag counts as that flag: a
        // launch that sets only `PHOTOCRAFT_PRESENT_MODE` (say) has per-process options too, so it
        // keeps its own process instead of handing its files over and being ignored.
        // Folded in here, inside the block that only non-macOS builds compile, so the binding
        // above keeps its `mut` off macOS, where nothing is folded in.
        let mut launch_options = launch_options;
        launch_options.extend_from_slice(&single_instance::env_launch_options());
        let context = single_instance::LaunchContext {
            per_user_settings: app_dirs::current().mode == app_dirs::Mode::Platform,
            new_instance_flag: new_instance,
            new_instance_env: new_instance_env.as_deref(),
            control: control_port.is_some(),
            unreadable_paths: !unreadable_paths.is_empty(),
            launch_options: &launch_options,
        };
        match (single_instance::skip_reason(&context), services::config_dir()) {
            (Some(reason), _) => {
                log::info!("running as a separate PhotoCraft: {reason}");
                None
            }
            (None, None) => None,
            (None, Some(dir)) => {
                let paths: Vec<String> = files.iter().map(|f| single_instance::absolute(f)).collect();
                match single_instance::claim(&dir, &paths) {
                    single_instance::Launch::Forwarded => {
                        log::info!("handed {} file(s) to the running PhotoCraft", paths.len());
                        return Ok(());
                    }
                    single_instance::Launch::Primary(primary) => Some(primary),
                    single_instance::Launch::Standalone(reason) => {
                        log::warn!("starting a separate PhotoCraft: {reason}");
                        None
                    }
                }
            }
        }
    };
    // Lives until the event loop returns, then `shutdown` lets the next launch become the primary.
    #[cfg(not(target_os = "macos"))]
    let single_instance = &single_instance;
    #[cfg(target_os = "macos")]
    let _ = (new_instance, &launch_options);

    // The display server to open the window on: Xwayland for a pen on Wayland, which gives this
    // app no pen input (#639).
    #[cfg(target_os = "linux")]
    let native_session = linux_libs::session_from_env(|name| std::env::var(name).ok());
    #[cfg(target_os = "linux")]
    let session = tablet::display_session();
    // Preferences › Performance › Linux display server = X11: Xwayland on a Wayland session too,
    // where native file drops work (winit 0.30 has none on Wayland, #386). Only when Xwayland can
    // run the window ($DISPLAY set, X11 libraries installed); otherwise the session's own.
    #[cfg(target_os = "linux")]
    let session = if session == linux_libs::DisplaySession::Wayland
        && gpu_startup::read_display_server(services::prefs_file().as_deref()) == photocraft_engine::prefs::LinuxDisplayServer::X11
        && std::env::var_os("DISPLAY").is_some_and(|d| !d.is_empty())
        && linux_libs::available(linux_libs::DisplaySession::X11)
    {
        eprintln!("photocraft: opening the window through Xwayland (Preferences › Performance › Linux display server)");
        linux_libs::DisplaySession::X11
    } else {
        session
    };

    // winit and wgpu dlopen the windowing and GPU libraries, and some of those crates panic when
    // one is missing (issue #201). Name the package to install and exit instead.
    #[cfg(target_os = "linux")]
    if let Err(message) = linux_libs::preflight(session) {
        eprint!("{message}");
        std::process::exit(1);
    }

    let control = if let Some(port) = control_port {
        let (supplied, token_file) = photocraft_automation::security::token_inputs(control_token, control_token_file);
        let token = match photocraft_automation::security::server_token(supplied.as_deref(), token_file.as_deref()) {
            Ok(token) => token,
            Err(e) => {
                eprintln!("photocraft: cannot configure control authentication: {e}");
                std::process::exit(2);
            }
        };
        if let Some(path) = token_file {
            eprintln!("photocraft: control token file: {}", path.display());
        } else if supplied.is_none() {
            eprintln!("photocraft: control token: {token}");
        } else {
            eprintln!("photocraft: using supplied control token");
        }
        let workspace = match photocraft_automation::AuthorizedWorkspace::new(automation_read_root.as_deref(), automation_write_root.as_deref()) {
            Ok(workspace) => workspace,
            Err(error) => {
                eprintln!("photocraft: cannot configure automation workspace: {error}");
                std::process::exit(2);
            }
        };
        Some((port, token, workspace))
    } else {
        None
    };
    // Finder / Dock / Open With deliver files as Apple events, not arguments; catch the one that
    // launched us as well as later ones. Lives until the event loop returns.
    #[cfg(target_os = "macos")]
    let apple_events = apple_events::AppleEvents::install();
    #[cfg(target_os = "macos")]
    let apple_events = &apple_events;

    // The macOS pen tablet monitor (installed once eframe created the app, see below); it lives
    // until the event loop returns.
    #[cfg(target_os = "macos")]
    let tablet_monitor = std::cell::OnceCell::new();
    #[cfg(target_os = "macos")]
    let tablet_monitor = &tablet_monitor;
    // The Windows pointer message hook; dropping it unhooks when the event loop returns.
    #[cfg(target_os = "windows")]
    let tablet_monitor_win = std::cell::OnceCell::new();
    #[cfg(target_os = "windows")]
    let tablet_monitor_win = &tablet_monitor_win;

    // Read the displays' ICC profiles while the window opens (colour-managed canvas; `None`
    // where the platform has no reader).
    #[cfg(target_os = "linux")]
    let monitor = monitor_profile::detect_async_for_session(native_session);
    #[cfg(not(target_os = "linux"))]
    let monitor = monitor_profile::detect_async();
    // Brush presets load in the background; the app attaches them when they arrive.
    let presets = services::presets_dir().map(photocraft_engine::preset_store::open_dir_async);
    let custom_titlebar = custom_titlebar(services::prefs_file().as_deref());
    let mut options = native_options(custom_titlebar);
    // winit picks Wayland whenever `WAYLAND_DISPLAY` is set; open on X11 when that was chosen.
    #[cfg(target_os = "linux")]
    if session == linux_libs::DisplaySession::X11 {
        use winit::platform::x11::EventLoopBuilderExtX11 as _;
        options.event_loop_builder = Some(Box::new(|builder| {
            builder.with_x11();
        }));
    }
    // eframe restores the saved window layout before our code runs; drop values that would crash it.
    ui_state::sanitize(options.persistence_path.as_deref());
    // Centre only the first window: eframe centres after restoring the saved position, so a
    // window closed on another monitor reopened in the middle of the main one (#4826).
    options.centered = !ui_state::has_saved_position(options.persistence_path.as_deref());
    // Crash-safe GPU startup (#4): pick the backend (a marker left by a start that died in the
    // driver moves to a safer one), and lock this start's marker until the first frames render.
    let t_sentinel = std::time::Instant::now();
    let os = gpu_startup::Os::current();
    let (pref, mode) = gpu_startup::read_rendering_prefs(services::prefs_file().as_deref());
    let (previous, sentinel) = match services::config_dir() {
        Some(dir) => gpu_startup::Sentinel::begin(&dir),
        None => (gpu_startup::Previous::Clean, None),
    };
    let env_backend = std::env::var("WGPU_BACKEND").ok();
    let plan = gpu_startup::plan_with_mode(pref, mode, previous.crashed(), env_backend.as_deref(), safe_gpu, os);
    if let Some(m) = previous.crashed() {
        log::warn!("the previous start didn't finish (GPU backend {}, adapter {:?}); {}", m.backend, m.adapter, plan.reason.as_deref().unwrap_or(""));
    }
    let sentinel: gpu_startup::SharedSentinel = std::sync::Arc::new(std::sync::Mutex::new(sentinel));
    if let Some(s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).as_mut() {
        let backend = match &plan.env {
            Some(v) => format!("env:{v}"),
            None => plan.backend.name().to_string(),
        };
        let marker = gpu_startup::Marker { backend, version: photocraft_engine::build_info::long_version().to_string(), ..Default::default() };
        if let Err(e) = s.write(marker) {
            log::warn!("GPU startup marker: {e}");
        }
    }
    let gpu_note: std::sync::Arc<std::sync::Mutex<Option<String>>> = Default::default();
    // The adapter's real texture limits (egui asks for 8192 px), so big documents stay on the GPU.
    let wanted_adapter =
        gpu_startup::wanted_adapter(std::env::var("PHOTOCRAFT_GPU_ADAPTER").ok().as_deref(), &gpu_startup::read_gpu_adapter(services::prefs_file().as_deref()));
    gpu_startup::configure(&mut options.wgpu_options.wgpu_setup, &plan, os, wanted_adapter, sentinel.clone(), gpu_note.clone());
    // Preferences › Performance › Low-Latency Drawing (#1343): without vsync a stroke shows about
    // 50 ms sooner; wgpu picks Immediate, else Mailbox, else Fifo by what the driver offers.
    if gpu_startup::read_low_latency(services::prefs_file().as_deref()) {
        options.wgpu_options.surface.present_mode = eframe::wgpu::PresentMode::AutoNoVsync;
    }
    // Testers' present mode and frame latency overrides (#1343), after the preference.
    let present = presentation::Overrides::from_args_or_env(present_mode_arg.as_deref(), frame_latency_arg.as_deref());
    options.wgpu_options.surface = present.apply(options.wgpu_options.surface);
    let present_desc = presentation::describe(&options.wgpu_options.surface, &present);
    // Without vsync, frames are capped at 240 fps (#4472) so repaint loops don't spin flat out.
    let uncapped = !matches!(
        options.wgpu_options.surface.present_mode,
        eframe::wgpu::PresentMode::Fifo | eframe::wgpu::PresentMode::FifoRelaxed | eframe::wgpu::PresentMode::AutoVsync
    );
    log::info!("presentation: {present_desc}");
    let sentinel_ms = t_sentinel.elapsed().as_secs_f64() * 1000.0;
    log::info!("GPU startup: {:?} ({sentinel_ms:.2} ms)", plan);
    let retry_cpu = !safe_gpu && plan.backend != photocraft_engine::prefs::GpuBackend::Cpu;
    let keep_marker = gpu_startup::keep_marker_after_error(&plan, os);
    let app_created = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let created_in_callback = app_created.clone();
    let started_sentinel = sentinel.clone();
    let result = eframe::run_native(
        "PhotoCraft",
        options,
        Box::new(move |cc| {
            created_in_callback.store(true, std::sync::atomic::Ordering::Relaxed);
            let automation = control.as_ref().map(|(_, _, workspace)| workspace.clone());
            let mut services = services::native(automation);
            services.preset_store = presets;
            #[cfg(target_os = "linux")]
            let display = tablet::DisplayKind::of(cc);
            #[cfg(target_os = "linux")]
            {
                services.is_wayland = display == Some(tablet::DisplayKind::Wayland);
                file_dialog_probe::start();
                if services.is_wayland {
                    let var = |name| std::env::var(name).ok();
                    services.xwayland_command =
                        services::xwayland_command(var("FLATPAK_ID").as_deref(), var("APPIMAGE").as_deref(), std::env::var_os("DISPLAY").is_some());
                }
            }
            #[cfg(target_os = "linux")]
            let session = {
                let mut session = Session::new();
                session.print_spooler = print_portal::spooler();
                session
            };
            #[cfg(not(target_os = "linux"))]
            let session = Session::new();
            let mut app = PhotocraftApp::new(session, services);
            app.min_frame_ms = uncapped.then_some(1000.0 / 240.0);
            app.integrated_titlebar = cfg!(target_os = "macos");
            app.custom_titlebar = custom_titlebar;
            // Only the title bar's free gap drags the window, never the menus (mac_window.rs).
            #[cfg(target_os = "macos")]
            mac_window::disable_native_title_drag();
            // Long commands and file opens run as background jobs with progress and Cancel (#210).
            app.background_jobs = std::env::var_os("PHOTOCRAFT_INLINE_JOBS").is_none();
            // Displays and their profiles (#569): wait briefly so the first frames already use
            // the right profile; a slower reading is applied when it arrives, and the shell reads
            // again when the displays may have changed.
            if let Some(rx) = monitor {
                #[cfg(target_os = "linux")]
                {
                    app.services.read_displays = Some(std::sync::Arc::new(move || monitor_profile::detect_async_for_session(native_session)));
                }
                #[cfg(not(target_os = "linux"))]
                {
                    app.services.read_displays = Some(std::sync::Arc::new(monitor_profile::detect_async));
                }
                match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                    Ok(r) => photocraft_ui_egui::monitor_status::apply(&mut app, r),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => photocraft_ui_egui::monitor_status::pending(&mut app, rx),
                    Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                        photocraft_ui_egui::monitor_status::apply(&mut app, Err("the display profile reader stopped without an answer".into()));
                    }
                }
            }
            // Preferences › Performance › Use Graphics Processor (and the GPU backend: `cpu`
            // composites on the CPU).
            let info = &mut app.perf.gpu_info;
            info.preference = pref.name().to_string();
            info.present = present_desc.clone();
            if let Some(v) = &stroke_trace_arg {
                app.stroke_trace.set(v);
            }
            info.selected = if plan.env.is_some() { "env".into() } else { plan.backend.name().to_string() };
            info.fallback = match (plan.reason.clone(), gpu_note.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone()) {
                (Some(reason), Some(note)) => Some(format!("{reason}. {note}")),
                (reason, note) => reason.or(note),
            };
            info.canvas = "cpu".into();
            if let Some(rs) = cc.wgpu_render_state.clone() {
                app.perf.gpu_info.set_adapter(&rs.adapter.get_info());
                app.perf.gpu_info.set_available(&rs.available_adapters);
                // Every egui texture uploads through this adapter, whichever canvas draws:
                // Iris Xe on DX12 shows stale memory in all-zero texels (texel_guard.rs).
                photocraft_ui_egui::texel_guard::install(&cc.egui_ctx, &rs.adapter.get_info());
                let software_window = rs.adapter.get_info().device_type == eframe::wgpu::DeviceType::Cpu;
                if software_window && mode != photocraft_engine::prefs::RenderingMode::Cpu {
                    // Keep an earlier note, such as a requested adapter that couldn't be used.
                    let software = "No compatible hardware graphics adapter; using software graphics.";
                    let fallback = &mut app.perf.gpu_info.fallback;
                    *fallback = Some(match fallback.take() {
                        Some(earlier) => format!("{earlier}. {software}"),
                        None => software.into(),
                    });
                }
                if !software_window
                    && plan.backend != photocraft_engine::prefs::GpuBackend::Cpu
                    && std::env::var_os("PHOTOCRAFT_CPU_CANVAS").is_none()
                    && app.session.prefs().performance.effective_rendering_mode() != photocraft_engine::prefs::RenderingMode::Cpu
                {
                    app.set_wgpu(rs);
                } else {
                    // The window still draws with wgpu: record its errors instead of panicking,
                    // and notice when the device is lost (#3438).
                    app.watch_window_device(&rs.device);
                }
            }
            let fallback_reason = std::env::var("PHOTOCRAFT_GPU_STARTUP_FAILURE").ok().or_else(|| {
                (app.perf.gpu_info.canvas == "cpu" && mode != photocraft_engine::prefs::RenderingMode::Cpu)
                    .then(|| app.perf.gpu_info.fallback.clone())
                    .flatten()
            });
            if let Some(reason) = fallback_reason {
                photocraft_ui_egui::gpu_status::queue_fallback_notice(&mut app, &reason);
            }
            app.perf.span("gpuSentinel", sentinel_ms);
            // CPU rendering after a failed GPU start is temporary. Only the
            // recovery-dialog choice may persist it (#3483).
            let remember = gpu_startup::backend_to_remember(&plan);
            app.on_started(move |app| {
                if let Some(s) = started_sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
                    s.finish();
                }
                if let Some(b) = remember
                    && app.session.prefs().performance.gpu_backend != b
                    && let Err(e) = app.run("prefs.set", serde_json::json!({"path": "performance.gpuBackend", "value": b.name()}))
                {
                    log::warn!("couldn't remember GPU backend {}: {e}", b.name());
                }
            });
            if let Some((port, token, _)) = control {
                let rx = control_server::start(port, token, cc.egui_ctx.clone());
                app = app.with_control(rx);
            }
            #[cfg(target_os = "macos")]
            {
                app.services.os_events = Some(apple_events.connect(&cc.egui_ctx));
                // Quit through AppKit, not by closing the window (Touch Bar Macs, #1575, #1458).
                app.services.quit = Some(Box::new(mac_window::terminate_later));
                // The macOS menu bar, installed now so winit's default menu doesn't stay up.
                if !in_window_menus {
                    app.services.native_menu = mac_menu::install(&cc.egui_ctx, &app);
                }
            }
            #[cfg(not(target_os = "macos"))]
            let _ = in_window_menus;
            // Files from later launches (Explorer, file managers) open here as new tabs.
            #[cfg(not(target_os = "macos"))]
            if let Some(primary) = single_instance {
                app.services.os_events = Some(primary.connect(&cc.egui_ctx));
            }
            // Where file drags and drops are (winit 0.30 doesn't say).
            app.services.cursor_pos = cursor::service(cc);
            // The custom title bar's undecorated window must stay borderless, or its caption and
            // border offset the content and the pointer on some Windows systems (#2246).
            app.services.window_frame = window_frame::service(cc, custom_titlebar);
            // Linux: the desktop's caption buttons (GNOME's, KDE's) on the sides its layout puts them.
            if custom_titlebar {
                app.services.caption_style = caption_style::service();
            }
            // Caps Lock state (X11/Windows/macOS; `None` on native Wayland): the canvas shows the
            // precise crosshair for painting tools while it is toggled on (#1758).
            app.services.caps_lock = caps_lock::service(cc);
            app.services.screen_pick = screen_color::service(cc, app.services.is_wayland);
            // Tablet pressure/tilt/eraser (winit drops them): the macOS monitor and the X11 reader
            // write into the stylus feed. The monitor goes in here, not before the event loop:
            // AppKit's shared application only exists once winit created it (#759).
            #[cfg(target_os = "macos")]
            if let Some(monitor) = tablet::install_macos(&app.stylus.feed) {
                let _ = tablet_monitor.set(monitor);
            }
            #[cfg(target_os = "windows")]
            if let Some(monitor) = tablet::install_windows() {
                let _ = tablet_monitor_win.set(monitor);
            }
            #[cfg(target_os = "linux")]
            tablet::spawn_x11(&app.stylus.feed, display);
            app.services.right_button_down = tablet::right_button_service();
            // Paths on the command line (Linux/Windows file associations, `photocraft a.psd`).
            app.open_paths(&files);
            if !unreadable_paths.is_empty() {
                use photocraft_ui_egui::i18n;
                let line = i18n::t("{path}: the path is not valid Unicode; rename the file and open it again.");
                let lines = unreadable_paths.iter().map(|p| i18n::fmt(line, &[("path", p.as_str())])).collect();
                photocraft_ui_egui::notices::post(&mut app, i18n::t("Could not open"), lines, true, None);
            }
            // Portable marker found but its data folder isn't writable (#228): say where settings went.
            if let Some(w) = &app_dirs::current().warning {
                photocraft_ui_egui::notices::post(&mut app, "Portable mode is off", vec![w.clone()], false, None);
            }
            Ok(Box::new(app))
        }),
    );
    // The window is gone: refuse further files and free the lock, so a launch that is waiting (or
    // the CPU-compatibility relaunch below) becomes the primary instead of handing its files to a
    // process that is exiting.
    #[cfg(not(target_os = "macos"))]
    if let Some(primary) = single_instance {
        primary.shutdown();
    }
    // Closed or failed outside graphics initialization: not a driver crash. A renderer error
    // keeps the marker, so the next start tries a safer backend, unless there is no safer one:
    // a failed CPU start would otherwise pin every later start to CPU.
    let renderer_error = gpu_startup::keep_marker_after_run(&result);
    if !(renderer_error && keep_marker)
        && let Some(mut s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take()
    {
        if result.is_err()
            && !renderer_error
            && let Some(marker) = previous.crashed()
        {
            // This failure supplies no new graphics-crash evidence: retain the previous
            // marker, rather than recording this attempt's fallback backend.
            if let Err(error) = s.write(marker.clone()) {
                log::warn!("couldn't restore previous GPU startup marker: {error}");
            }
        } else {
            s.finish();
        }
    }
    // Retry in a fresh process: winit event loops cannot be recreated reliably in-process.
    // Only renderer initialization failures qualify; never restart after editing has begun.
    if retry_cpu && !app_created.load(std::sync::atomic::Ordering::Relaxed) && matches!(&result, Err(eframe::Error::Wgpu(_))) {
        let reason = result.as_ref().err().map(ToString::to_string).unwrap_or_default();
        if let Some(s) = sentinel.lock().unwrap_or_else(std::sync::PoisonError::into_inner).take() {
            s.finish();
        }
        if let Ok(exe) = std::env::current_exe() {
            let launched = std::process::Command::new(exe)
                .args(std::env::args_os().skip(1))
                .arg("--safe-gpu")
                .env_remove("WGPU_BACKEND")
                .env("PHOTOCRAFT_GPU_STARTUP_FAILURE", &reason)
                .spawn();
            match launched {
                Ok(_) => return Ok(()),
                Err(error) => log::error!("could not start CPU compatibility mode: {error}"),
            }
        }
    }
    result
}

/// Tests that spawn child processes hold this for reading, tests that check who holds a file lock
/// for writing ([`no_child_spawns`]): until its `exec`, a child holds a copy of every open file,
/// `flock`s included, so a spawn in another test could keep a lock this process just dropped held
/// for a moment.
#[cfg(test)]
pub(crate) static CHILD_SPAWNS: std::sync::RwLock<()> = std::sync::RwLock::new(());

/// Held by a test that checks who holds a file lock: no other test spawns a child meanwhile.
#[cfg(test)]
pub(crate) fn no_child_spawns() -> std::sync::RwLockWriteGuard<'static, ()> {
    CHILD_SPAWNS.write().unwrap_or_else(std::sync::PoisonError::into_inner)
}

#[cfg(test)]
mod tests {
    #[test]
    fn window_and_panel_geometry_survive_a_restart() {
        let options = super::native_options(super::CUSTOM_TITLEBAR);
        assert!(options.persist_window);
        assert_eq!(options.persistence_path, super::services::config_dir().map(|dir| dir.join("ui.ron")));

        #[derive(Default)]
        struct Storage(std::collections::BTreeMap<String, String>);
        impl eframe::Storage for Storage {
            fn get_string(&self, key: &str) -> Option<String> {
                self.0.get(key).cloned()
            }
            fn set_string(&mut self, key: &str, value: String) {
                self.0.insert(key.into(), value);
            }
            fn remove_string(&mut self, key: &str) {
                self.0.remove(key);
            }
            fn flush(&mut self) {}
        }
        let ctx = egui::Context::default();
        let input = || egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1200.0, 800.0))), ..Default::default() };
        let mut output = ctx.run_ui(input(), |ui| {
            egui::Panel::right("dock").exact_size(410.0).show(ui, |ui| ui.set_min_width(ui.available_width()));
        });
        output.textures_delta.clear();
        let mut storage = Storage::default();
        ctx.memory(|memory| eframe::set_value(&mut storage, "egui", memory));
        let restored = egui::Context::default();
        restored.memory_mut(|memory| *memory = eframe::get_value(&storage, "egui").unwrap());
        let mut output = restored.run_ui(input(), |ui| {
            egui::Panel::right("dock").default_size(290.0).show(ui, |ui| ui.set_min_width(ui.available_width()));
        });
        output.textures_delta.clear();
        let panel = egui::containers::panel::PanelState::load(&restored, egui::Id::new("dock")).unwrap();
        assert_eq!(panel.size().x, 410.0);
    }

    #[test]
    fn the_window_opens_centred_at_its_default_size() {
        let o = super::native_options(super::CUSTOM_TITLEBAR);
        assert!(o.centered, "#419: centred, not cascaded from the top-left corner");
        assert_eq!(o.viewport.inner_size, Some(egui::vec2(1440.0, 900.0)));
        // eframe shrinks the start size to the monitor, so the centred position is on-screen.
        assert_ne!(o.viewport.clamp_size_to_monitor_size, Some(false));
    }

    #[test]
    fn a_window_saved_while_minimized_reopens_at_the_minimum_size() {
        let hook = super::native_options(super::CUSTOM_TITLEBAR).window_builder.expect("a window builder hook");
        // Saved minimized on Windows, then centred on a 1920 × 1080 monitor by eframe.
        let saved = super::native_options(super::CUSTOM_TITLEBAR).viewport.with_inner_size([160.0, 64.0]).with_position([880.0, 508.0]);
        let opened = hook(saved);
        assert_eq!(opened.inner_size, Some(egui::vec2(760.0, 480.0)));
        assert_eq!(opened.position, Some(egui::pos2(580.0, 300.0)), "still centred");
        // Too narrow only: only the width grows.
        let opened = super::at_least_min_size(super::native_options(false).viewport.with_inner_size([500.0, 900.0]));
        assert_eq!(opened.inner_size, Some(egui::vec2(760.0, 900.0)));
        // A window at or above the minimum, or with no size, is left alone.
        for size in [[760.0, 480.0], [1100.0, 760.0]] {
            let builder = super::native_options(false).viewport.with_inner_size(size).with_position([10.0, 20.0]);
            let opened = super::at_least_min_size(builder);
            assert_eq!((opened.inner_size, opened.position), (Some(egui::Vec2::from(size)), Some(egui::pos2(10.0, 20.0))));
        }
        let mut builder = super::native_options(false).viewport;
        builder.inner_size = None;
        assert_eq!(super::at_least_min_size(builder).inner_size, None);
    }

    #[test]
    fn a_window_saved_in_full_screen_reopens_maximized_not_full_screen() {
        let hook = super::native_options(super::CUSTOM_TITLEBAR).window_builder.expect("a window builder hook");
        // What eframe restores from a ui.ron saved in a full-screen mode on Windows.
        let saved = super::native_options(super::CUSTOM_TITLEBAR)
            .viewport
            .with_inner_size([2560.0, 1392.0])
            .with_position([0.0, 0.0])
            .with_fullscreen(true)
            .with_maximized(true);
        let opened = hook(saved);
        assert_eq!(opened.fullscreen, Some(false), "Standard Screen Mode is where PhotoCraft starts");
        assert_eq!(opened.maximized, Some(true), "the rest of the saved geometry is kept");
        assert_eq!(opened.inner_size, Some(egui::vec2(2560.0, 1392.0)));
    }

    #[test]
    fn the_system_title_bar_preference_keeps_the_window_decorations() {
        // #1271, #1316: Preferences › Interface › System Title Bar gives the window back its
        // system decorations on Windows and Linux; macOS always has them.
        let dir = std::env::temp_dir().join(format!("pc-titlebar-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("prefs.json");
        assert_eq!(super::custom_titlebar(None), super::CUSTOM_TITLEBAR, "no preferences file: the default");
        std::fs::write(&file, "not json").unwrap();
        assert_eq!(super::custom_titlebar(Some(&file)), super::CUSTOM_TITLEBAR, "an unreadable file: the default");
        std::fs::write(&file, r#"{"interface":{"systemTitleBar":true}}"#).unwrap();
        assert!(!super::custom_titlebar(Some(&file)));
        assert_eq!(super::native_options(false).viewport.decorations, Some(true));
        std::fs::write(&file, r#"{"interface":{"systemTitleBar":false}}"#).unwrap();
        assert_eq!(super::custom_titlebar(Some(&file)), super::CUSTOM_TITLEBAR);
        if super::CUSTOM_TITLEBAR {
            assert_eq!(super::native_options(true).viewport.decorations, Some(false));
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// X11 startup notification and docks match a window to its launcher by `StartupWMClass`,
    /// so it has to be the class the window carries: the app id, on Wayland and X11 alike.
    #[test]
    fn the_desktop_entry_names_the_window_class() {
        let entry = include_str!("../../../packaging/linux/ai.storyteller.photocraft.desktop");
        let class = entry.lines().find_map(|line| line.strip_prefix("StartupWMClass="));
        assert_eq!(class, Some(super::APP_ID));
    }
}
