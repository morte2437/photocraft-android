//! `photocraft --help` and unknown options must not open the window (#6431): before the fix,
//! any argument the loop didn't know was taken as a file, so `--help` or a typo such as
//! `--safegpu` launched the GUI and logged "Couldn't open --help".
//!
//! These run the real binary, and every case must exit before the window opens: Windows and
//! macOS runners have a desktop, so a launch that gets that far hangs until the test times out.
//! Cases whose arguments are valid add a bad `--control` value, which fails the launch with the
//! other usage errors, so stderr shows whether those arguments were taken as unknown options.

/// Run `photocraft` with `args` in an isolated config dir; return the exit
/// status, stdout and stderr.
fn run_photocraft(args: &[&str]) -> (std::process::ExitStatus, String, String) {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("photocraft-usage-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_photocraft"))
        .args(args)
        .env("PHOTOCRAFT_CONFIG_DIR", &dir)
        .env_remove("PHOTOCRAFT_CONTROL_PORT")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .output()
        .expect("failed to run photocraft binary");
    std::fs::remove_dir_all(&dir).ok();
    (output.status, String::from_utf8_lossy(&output.stdout).into_owned(), String::from_utf8_lossy(&output.stderr).into_owned())
}

#[test]
fn help_prints_usage_and_exits_0() {
    for flag in ["--help", "-h"] {
        let (status, stdout, stderr) = run_photocraft(&[flag]);
        assert_eq!(status.code(), Some(0), "{flag} must exit 0, stderr: {stderr}");
        assert!(stdout.starts_with("Usage: photocraft"), "{flag} stdout: {stdout}");
        for option in ["--safe-gpu", "--new-instance", "--control <port>", "--version", "--"] {
            assert!(stdout.contains(option), "{flag} usage must list {option}: {stdout}");
        }
        assert!(!stderr.contains("Couldn't open"), "{flag} was taken as a file: {stderr}");
    }
}

#[test]
fn help_wins_over_later_arguments() {
    let (status, stdout, _) = run_photocraft(&["--help", "picture.psd", "--safegpu"]);
    assert_eq!(status.code(), Some(0));
    assert!(stdout.starts_with("Usage: photocraft"), "stdout: {stdout}");
}

#[test]
fn unknown_options_exit_2_naming_them() {
    for flag in ["--safegpu", "-x", "--help=yes"] {
        let (status, stdout, stderr) = run_photocraft(&[flag]);
        assert_eq!(status.code(), Some(2), "{flag} must be a usage error, stderr: {stderr}");
        assert!(stderr.contains(&format!("unknown option `{flag}`")), "stderr: {stderr}");
        assert!(stderr.contains("photocraft --help"), "stderr should point at --help: {stderr}");
        assert!(stdout.is_empty(), "stdout: {stdout}");
    }
}

#[test]
fn unknown_option_errors_are_reported_with_control_port_errors() {
    let (status, _, stderr) = run_photocraft(&["--control", "nope", "--safegpu"]);
    assert_eq!(status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("`nope` is not a valid port"), "stderr: {stderr}");
    assert!(stderr.contains("unknown option `--safegpu`"), "stderr: {stderr}");
}

#[test]
fn arguments_after_double_dash_are_files() {
    // `--` ends the options: `--help` and `-x` after it are file names, so neither prints usage
    // nor is an unknown option; only the bad port is reported.
    let (status, stdout, stderr) = run_photocraft(&["--control", "nope", "--", "--help", "-x"]);
    assert_eq!(status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("`nope` is not a valid port"), "stderr: {stderr}");
    assert!(!stdout.contains("Usage:"), "stdout: {stdout}");
    assert!(!stderr.contains("unknown option"), "stderr: {stderr}");
}

#[test]
fn macos_process_serial_number_is_still_ignored() {
    // Old macOS passes `-psn_<n>` when launching from Finder; it is neither a file nor an error.
    let (status, _, stderr) = run_photocraft(&["--control", "nope", "-psn_0_12345"]);
    assert_eq!(status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("`nope` is not a valid port"), "stderr: {stderr}");
    assert!(!stderr.contains("unknown option"), "stderr: {stderr}");
}

#[test]
fn new_instance_is_a_known_option() {
    let (status, _, stderr) = run_photocraft(&["--control", "nope", "--new-instance"]);
    assert_eq!(status.code(), Some(2), "stderr: {stderr}");
    assert!(stderr.contains("`nope` is not a valid port"), "stderr: {stderr}");
    assert!(!stderr.contains("unknown option"), "stderr: {stderr}");
}
