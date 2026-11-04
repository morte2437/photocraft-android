//! The binary re-executes itself with glibc's trim threshold set (#8743), keeping its arguments
//! and a value the user set.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

use photocraft_engine::malloc_tuning::{TRIM_THRESHOLD, TRIM_THRESHOLD_VAR};

/// The environment of a running `serve` once it answered a request, as `NAME=value` entries.
fn serve_environ(set: Option<&str>) -> Vec<String> {
    let mut cmd = Command::new(crate::binary());
    for var in [TRIM_THRESHOLD_VAR, "MALLOC_MMAP_THRESHOLD_", "MALLOC_TOP_PAD_", "MALLOC_MMAP_MAX_", "MALLOC_ARENA_MAX", "GLIBC_TUNABLES"] {
        cmd.env_remove(var);
    }
    if let Some(v) = set {
        cmd.env(TRIM_THRESHOLD_VAR, v);
    }
    let mut child = cmd.arg("serve").stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().unwrap();
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, r#"{{"id":1,"method":"session.list"}}"#).unwrap();
    let mut reply = String::new();
    BufReader::new(child.stdout.take().unwrap()).read_line(&mut reply).unwrap();
    assert!(reply.contains(r#""id":1"#), "serve answered after the re-exec: {reply}");
    // The same process: `exec` keeps the PID, so this is the re-executed program's environment.
    let environ = std::fs::read(format!("/proc/{}/environ", child.id())).unwrap();
    drop(stdin);
    child.wait().unwrap();
    environ.split(|&b| b == 0).map(|e| String::from_utf8_lossy(e).into_owned()).collect()
}

#[test]
fn sets_the_trim_threshold() {
    let env = serve_environ(None);
    assert!(env.contains(&format!("{TRIM_THRESHOLD_VAR}={TRIM_THRESHOLD}")), "{env:?}");
}

#[test]
fn keeps_a_value_the_user_set() {
    let env = serve_environ(Some("999999"));
    assert!(env.contains(&format!("{TRIM_THRESHOLD_VAR}=999999")), "{env:?}");
}
