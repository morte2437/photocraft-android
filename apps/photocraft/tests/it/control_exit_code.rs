//! `--control` setup failures must exit non-zero (#1816): a launcher that sees
//! status 0 believes the control server is running when it is not.
//!
//! These tests run the real binary as a subprocess and assert the exit code,
//! covering control authentication, the automation workspace and listener binding.

/// A syntactically valid 64-hex-char token, so the workspace test reaches the
/// workspace check instead of failing on token validation first.
const VALID_TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

/// How a run of `photocraft` ended.
struct Run {
    status: std::process::ExitStatus,
    stderr: String,
}

/// Run `photocraft` with the given arguments in an isolated config dir and
/// return how it ended. Each call gets its own config dir (tests run in
/// parallel), and no display variables, so a regression fails fast instead of opening a window.
fn run_photocraft(args: &[&str]) -> Run {
    run_with(args, &[])
}

/// [`run_photocraft`], with `vars` set.
fn run_with(args: &[&str], vars: &[(&str, &str)]) -> Run {
    static NEXT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("photocraft-control-exit-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let mut child = std::process::Command::new(crate::binary())
        .args(args)
        .env("PHOTOCRAFT_CONFIG_DIR", &dir)
        // Isolate from the host environment so the test is reproducible.
        .env_remove("PHOTOCRAFT_CONTROL_PORT")
        .env_remove("PHOTOCRAFT_CONTROL_TOKEN")
        .env_remove("PHOTOCRAFT_CONTROL_TOKEN_FILE")
        .env_remove("PHOTOCRAFT_AUTOMATION_READ_ROOT")
        .env_remove("PHOTOCRAFT_AUTOMATION_WRITE_ROOT")
        .env_remove("DISPLAY")
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("WAYLAND_SOCKET")
        .envs(vars.iter().copied())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("failed to run photocraft binary");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    let timed_out = loop {
        if child.try_wait().unwrap().is_some() {
            break false;
        }
        if std::time::Instant::now() >= deadline {
            child.kill().unwrap();
            break true;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    };
    let output = child.wait_with_output().unwrap();
    let gpu_started = dir.join("gpu-starting.json").exists();
    std::fs::remove_dir_all(&dir).ok();
    assert!(!timed_out, "PhotoCraft did not exit: {}", String::from_utf8_lossy(&output.stderr));
    assert!(!gpu_started, "control setup errors must exit before GPU startup");
    Run { status: output.status, stderr: String::from_utf8_lossy(&output.stderr).into_owned() }
}

#[test]
fn an_occupied_control_port_exits_with_status_2() {
    let held = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = held.local_addr().unwrap().port().to_string();
    let Run { status, stderr } = run_photocraft(&["--control", &port, "--control-token", VALID_TOKEN]);
    assert_eq!(status.code(), Some(2), "occupied control port must exit 2, stderr: {stderr}");
    assert!(stderr.contains(&format!("failed to bind 127.0.0.1:{port}")), "stderr: {stderr}");
}

#[test]
fn invalid_control_token_exits_with_status_2() {
    // `validate_token` rejects a token that is not exactly 64 hex characters.
    let Run { status, stderr } = run_photocraft(&["--control", "0", "--control-token", "invalid"]);
    assert_eq!(status.code(), Some(2), "invalid control token must exit 2, stderr: {stderr}");
    assert!(stderr.contains("cannot configure control authentication"), "stderr: {stderr}");
}

#[test]
fn both_token_and_token_file_exits_with_status_2() {
    // The command line rejects a supplied token together with a token file (#7071), before
    // `server_token` would.
    let dir = std::env::temp_dir().join(format!("photocraft-token-file-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let token_file = dir.join("token.txt");
    std::fs::write(&token_file, VALID_TOKEN).unwrap();
    let Run { status, stderr } = run_photocraft(&["--control", "0", "--control-token", VALID_TOKEN, "--control-token-file", token_file.to_str().unwrap()]);
    std::fs::remove_dir_all(&dir).ok();
    assert_eq!(status.code(), Some(2), "token + token file must exit 2, stderr: {stderr}");
    assert!(stderr.contains("cannot be used with"), "stderr: {stderr}");
}

#[test]
fn missing_automation_read_root_exits_with_status_2() {
    // `AuthorizedWorkspace::new` fails when the read root does not exist.
    let Run { status, stderr } =
        run_photocraft(&["--control", "0", "--control-token", VALID_TOKEN, "--automation-read-root", "/nonexistent/photocraft-test-path"]);
    assert_eq!(status.code(), Some(2), "missing read root must exit 2, stderr: {stderr}");
    assert!(stderr.contains("cannot configure automation workspace"), "stderr: {stderr}");
}

#[test]
fn missing_automation_write_root_exits_with_status_2() {
    // `AuthorizedWorkspace::new` fails when the write root does not exist.
    let Run { status, stderr } =
        run_photocraft(&["--control", "0", "--control-token", VALID_TOKEN, "--automation-write-root", "/nonexistent/photocraft-test-path"]);
    assert_eq!(status.code(), Some(2), "missing write root must exit 2, stderr: {stderr}");
    assert!(stderr.contains("cannot configure automation workspace"), "stderr: {stderr}");
}

#[test]
fn automation_roots_come_from_the_environment_too() {
    for var in ["PHOTOCRAFT_AUTOMATION_READ_ROOT", "PHOTOCRAFT_AUTOMATION_WRITE_ROOT"] {
        let Run { status, stderr } = run_with(&["--control", "0", "--control-token", VALID_TOKEN], &[(var, "/nonexistent/photocraft-test-path")]);
        assert_eq!(status.code(), Some(2), "{var}: missing root must exit 2, stderr: {stderr}");
        assert!(stderr.contains("cannot configure automation workspace"), "{var}: stderr: {stderr}");
    }
}
