use super::*;
use std::path::PathBuf;
use std::sync::mpsc::channel;

const FAST: Timing = Timing { handoff: Duration::from_millis(1500), ack: Duration::from_millis(500), request: Duration::from_millis(800) };

fn temp_dir(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-single-instance-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Launch with `paths` on another thread (it blocks until the primary's UI takes them).
fn launch(dir: &Path, paths: &[&str], timing: Timing) -> std::thread::JoinHandle<Launch> {
    let (dir, paths): (PathBuf, Vec<String>) = (dir.to_path_buf(), paths.iter().map(|p| p.to_string()).collect());
    std::thread::spawn(move || claim_with(&dir, &paths, timing))
}

/// Play the UI: drain until `n` events arrived (or 5 s passed).
fn drain(ui: &mut OsEventsFn, n: usize) -> Vec<OsEvent> {
    let mut got = Vec::new();
    let deadline = Instant::now() + Duration::from_secs(5);
    while got.len() < n && Instant::now() < deadline {
        got.extend(ui());
        std::thread::sleep(Duration::from_millis(10));
    }
    got
}

fn primary(dir: &Path) -> Primary {
    match claim_with(dir, &[], FAST) {
        Launch::Primary(p) => p,
        _ => panic!("first launch should be primary"),
    }
}

/// A socket path short enough for every Unix (`sun_path` is about 100 bytes; macOS: 104).
#[cfg(unix)]
fn short_socket_path(tag: &str) -> PathBuf {
    std::env::temp_dir().join(format!("pc-si-{}-{tag}", std::process::id())).join(SOCKET_FILE)
}

/// The loopback port the primary under `dir` listens on.
fn port_of(dir: &Path) -> u16 {
    match read_info(dir).unwrap().0 {
        Endpoint::Port(port) => port,
        #[cfg(unix)]
        Endpoint::Socket(path) => panic!("expected a loopback port, found {}", path.display()),
    }
}

#[test]
fn second_launch_hands_its_paths_to_the_first() {
    let dir = temp_dir("handoff");
    let primary = primary(&dir);
    let mut ui = primary.connect(&egui::Context::default());
    let paths = ["C:\\art\\one.psd", "/home/a/two é.png"];
    let second = launch(&dir, &paths, TIMING);
    assert_eq!(drain(&mut ui, 1), vec![OsEvent::Open(paths.iter().map(|p| p.to_string()).collect())]);
    assert!(matches!(second.join().unwrap(), Launch::Forwarded));
    // A plain second start (no files) just brings the window forward.
    let third = launch(&dir, &[], TIMING);
    assert_eq!(drain(&mut ui, 1), vec![OsEvent::Open(Vec::new())]);
    assert!(matches!(third.join().unwrap(), Launch::Forwarded));
    drop(primary);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn simultaneous_launches_all_land_in_one_instance() {
    let dir = temp_dir("race");
    let (tx, rx) = channel();
    for i in 0..6 {
        let (dir, tx) = (dir.clone(), tx.clone());
        std::thread::spawn(move || {
            let _ = tx.send(claim(&dir, &[format!("file{i}.psd")]));
        });
    }
    drop(tx);
    let (mut primaries, mut forwarded, mut events) = (Vec::new(), 0, Vec::new());
    let mut ui: Option<OsEventsFn> = None;
    let deadline = Instant::now() + Duration::from_secs(10);
    while primaries.len() + forwarded < 6 && Instant::now() < deadline {
        while let Ok(launch) = rx.try_recv() {
            match launch {
                Launch::Primary(p) => primaries.push(p),
                Launch::Forwarded => forwarded += 1,
                Launch::Standalone(reason) => panic!("standalone: {reason}"),
            }
        }
        if ui.is_none()
            && let Some(p) = primaries.first()
        {
            ui = Some(p.connect(&egui::Context::default()));
        }
        if let Some(ui) = ui.as_mut() {
            events.extend(ui());
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!((primaries.len(), forwarded, events.len()), (1, 5, 5));
    drop(primaries);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_crashed_instance_leaves_nothing_in_the_way() {
    let dir = temp_dir("stale");
    std::fs::create_dir_all(&dir).unwrap();
    // instance.json from a process that is gone; the lock is free.
    std::fs::write(dir.join(INFO_FILE), r#"{"port":1,"token":"x","pid":1}"#).unwrap();
    let primary = primary(&dir);
    assert!(read_info(&dir).is_some_and(|(endpoint, _)| endpoint != Endpoint::Port(1)));
    drop(primary);
    // Released with the handle, as on process exit.
    assert!(matches!(claim_with(&dir, &[], FAST), Launch::Primary(_)));
    let _ = std::fs::remove_dir_all(&dir);
}

/// Review of #3755, point 1 (and the quitting half of point 3): a launch waiting on a primary
/// that shuts down (closing, or about to relaunch itself in CPU mode) isn't told `ok`; it takes
/// the lock over and opens the file itself.
#[test]
fn a_launch_waiting_on_a_closing_instance_takes_over() {
    let dir = temp_dir("closing");
    let primary = primary(&dir);
    // The UI never drains (it is shutting down, or never came up).
    let waiting = launch(&dir, &["a.psd"], FAST);
    std::thread::sleep(Duration::from_millis(200));
    primary.shutdown();
    assert!(matches!(waiting.join().unwrap(), Launch::Primary(_)), "the waiting launch becomes the primary");
    // After shutdown new connections are refused too.
    let (endpoint, token) = read_info(&dir).unwrap();
    drop(primary);
    let short = Instant::now() + Duration::from_millis(300);
    assert!(send(&endpoint, &token, &["b.psd".into()], short).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

/// Review of #3755, point 3: a primary whose UI never takes the paths (hung) doesn't
/// acknowledge them, and doesn't keep them queued to open later as well.
#[test]
fn a_hung_instance_never_swallows_a_file() {
    let dir = temp_dir("hung-ui");
    let primary = primary(&dir);
    let mut ui = primary.connect(&egui::Context::default());
    let started = Instant::now();
    assert!(matches!(claim_with(&dir, &["a.psd".into()], FAST), Launch::Standalone(_)));
    assert!(started.elapsed() < FAST.handoff + Duration::from_secs(1), "{:?}", started.elapsed());
    assert!(ui().is_empty(), "withdrawn, not opened later");
    drop(primary);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn requests_without_the_token_or_malformed_are_rejected() {
    let token = generate_token().unwrap();
    let ok = parse_request(&json!({"token": token, "open": ["a"], "wait_ms": 250}).to_string(), &token);
    assert_eq!(ok, Some((vec!["a".to_string()], Duration::from_millis(250))));
    let wrong = generate_token().unwrap();
    assert_eq!(parse_request(&json!({"token": wrong, "open": ["a"]}).to_string(), &token), None);
    for bad in ["", "not json", "{}", r#"{"token":5}"#, &json!({"token": token}).to_string(), &json!({"token": token, "open": [1]}).to_string()] {
        assert_eq!(parse_request(bad, &token), None, "{bad:?}");
    }
    let many: Vec<String> = (0..=MAX_PATHS).map(|i| i.to_string()).collect();
    assert_eq!(parse_request(&json!({"token": token, "open": many}).to_string(), &token), None);
}

#[test]
fn a_stalled_client_does_not_hold_up_the_next_launch() {
    let dir = temp_dir("stalled");
    let primary = primary(&dir);
    let mut ui = primary.connect(&egui::Context::default());
    let port = port_of(&dir);
    // Connects and never sends a line (each would otherwise hold the listener).
    let stalled: Vec<TcpStream> = (0..3).map(|_| TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap()).collect();
    let started = Instant::now();
    let next = launch(&dir, &["b.psd"], FAST);
    assert_eq!(drain(&mut ui, 1), vec![OsEvent::Open(vec!["b.psd".into()])]);
    assert!(matches!(next.join().unwrap(), Launch::Forwarded));
    assert!(started.elapsed() < FAST.request, "took {:?}", started.elapsed());
    drop(stalled);
    drop(primary);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A client that trickles a byte at a time can't keep its slot past the request deadline.
#[test]
fn a_trickling_client_is_cut_off_at_the_request_deadline() {
    let dir = temp_dir("trickle");
    let primary = primary(&dir);
    let port = port_of(&dir);
    let mut slow = TcpStream::connect((Ipv4Addr::LOCALHOST, port)).unwrap();
    let started = Instant::now();
    let mut cut_off = false;
    while started.elapsed() < FAST.request * 4 {
        if slow.write_all(b"{").is_err() {
            cut_off = true;
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(cut_off, "still connected after {:?}", started.elapsed());
    assert!(started.elapsed() < FAST.request * 3, "{:?}", started.elapsed());
    drop(primary);
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(unix)]
#[test]
fn the_token_file_is_private_to_the_user() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("mode");
    std::fs::create_dir_all(&dir).unwrap();
    // A wide leftover temporary file from a crashed start doesn't widen the new one.
    let tmp = dir.join(format!("{INFO_FILE}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, "{}").unwrap();
    std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
    write_info(&dir, &Endpoint::Port(1), &generate_token().unwrap()).unwrap();
    let mode = std::fs::metadata(dir.join(INFO_FILE)).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_unreachable_instance_falls_back_to_a_standalone_launch() {
    let dir = temp_dir("unreachable");
    std::fs::create_dir_all(&dir).unwrap();
    // Someone holds the lock but nothing listens on the advertised port.
    let held = File::create(dir.join(LOCK_FILE)).unwrap();
    held.lock().unwrap();
    let closed = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).unwrap().local_addr().unwrap().port();
    write_info(&dir, &Endpoint::Port(closed), &generate_token().unwrap()).unwrap();
    assert!(matches!(claim_with(&dir, &["a.psd".into()], FAST), Launch::Standalone(_)));
    drop(held);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Review of #3755, point 2 and the smaller points: who takes part in single-instance.
#[test]
fn only_per_user_launches_without_process_options_take_part() {
    let base = LaunchContext {
        per_user_settings: true,
        new_instance_flag: false,
        new_instance_env: None,
        control: false,
        unreadable_paths: false,
        launch_options: &[],
    };
    assert_eq!(skip_reason(&base), None);
    // A portable copy's folder or a PHOTOCRAFT_CONFIG_DIR may be shared between users.
    assert!(skip_reason(&LaunchContext { per_user_settings: false, ..base }).is_some());
    assert!(skip_reason(&LaunchContext { new_instance_flag: true, ..base }).is_some());
    assert!(skip_reason(&LaunchContext { new_instance_env: Some("1"), ..base }).is_some());
    assert_eq!(skip_reason(&LaunchContext { new_instance_env: Some("0"), ..base }), None);
    assert_eq!(skip_reason(&LaunchContext { new_instance_env: Some(""), ..base }), None);
    assert!(skip_reason(&LaunchContext { control: true, ..base }).is_some());
    assert!(skip_reason(&LaunchContext { unreadable_paths: true, ..base }).is_some());
    let reason = skip_reason(&LaunchContext { launch_options: &[PerProcess::SafeGpu, PerProcess::GpuAdapterEnv], ..base });
    assert_eq!(reason.as_deref(), Some("--safe-gpu, PHOTOCRAFT_GPU_ADAPTER only applies to a new process"));
}

/// #1540: inside a Flatpak sandbox the hand-off runs over a Unix socket in the app's runtime
/// directory, which every sandbox of the app shares, with the same lock, token and reply.
#[cfg(unix)]
#[test]
fn a_sandboxed_second_launch_hands_its_paths_over_a_unix_socket() {
    use std::os::unix::fs::PermissionsExt;
    let dir = temp_dir("socket");
    // Short: a Unix socket path is limited to about 100 bytes (macOS: 104).
    let socket = short_socket_path("a");
    let transport = Transport::Socket(socket.clone());
    let primary = match claim_in(&dir, &[], FAST, &transport) {
        Launch::Primary(p) => p,
        _ => panic!("first launch should be primary"),
    };
    assert_eq!(std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777, 0o600);
    let info: Value = serde_json::from_slice(&std::fs::read(dir.join(INFO_FILE)).unwrap()).unwrap();
    assert_eq!(info.get("socket").and_then(Value::as_str), Some(socket.to_str().unwrap()));
    assert!(info.get("port").is_none());
    let mut ui = primary.connect(&egui::Context::default());
    let paths = ["/home/a/one.psd".to_string()];
    let (d, p, t) = (dir.clone(), paths.clone(), transport.clone());
    let second = std::thread::spawn(move || claim_in(&d, &p, TIMING, &t));
    assert_eq!(drain(&mut ui, 1), vec![OsEvent::Open(paths.to_vec())]);
    assert!(matches!(second.join().unwrap(), Launch::Forwarded));
    // The token still gates the socket.
    let (endpoint, _) = read_info(&dir).unwrap();
    assert!(send(&endpoint, "not the token", &["x.psd".into()], Instant::now() + FAST.handoff).is_err());
    primary.shutdown();
    assert!(!socket.exists(), "shutdown removes the socket file");
    // The next primary binds the same path; the old one's drop (a second shutdown) must leave it.
    let next = match claim_in(&dir, &[], FAST, &transport) {
        Launch::Primary(p) => p,
        _ => panic!("the lock is free again"),
    };
    drop(primary);
    assert!(socket.exists(), "a second shutdown doesn't remove the next primary's socket");
    drop(next);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A socket file left by a crashed primary (and a stale `instance.json` naming it) is replaced.
#[cfg(unix)]
#[test]
fn a_crashed_primary_s_socket_file_does_not_block_the_next_one() {
    use std::os::unix::fs::FileTypeExt;
    let dir = temp_dir("stale-socket");
    let socket = short_socket_path("b");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(socket.parent().unwrap()).unwrap();
    std::fs::write(&socket, b"not a socket").unwrap();
    write_info(&dir, &Endpoint::Socket(socket.clone()), &generate_token().unwrap()).unwrap();
    let primary = match claim_in(&dir, &["a.psd".into()], FAST, &Transport::Socket(socket.clone())) {
        Launch::Primary(p) => p,
        Launch::Forwarded => panic!("nothing was listening"),
        Launch::Standalone(e) => panic!("{e}"),
    };
    assert!(std::fs::metadata(&socket).unwrap().file_type().is_socket());
    drop(primary);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(socket.parent().unwrap());
}

/// The socket transport is chosen only inside a Flatpak with a runtime directory.
#[cfg(unix)]
#[test]
fn the_socket_transport_needs_a_flatpak_id_and_a_runtime_directory() {
    use std::ffi::OsStr;
    let (id, run) = (OsStr::new("ai.storyteller.photocraft"), OsStr::new("/run/user/1000"));
    assert_eq!(sandbox_socket(Some(id), Some(run)), Some(PathBuf::from("/run/user/1000/app/ai.storyteller.photocraft/instance.sock")));
    assert_eq!(sandbox_socket(None, Some(run)), None);
    assert_eq!(sandbox_socket(Some(id), None), None);
    assert_eq!(sandbox_socket(Some(OsStr::new("")), Some(run)), None);
    assert_eq!(sandbox_socket(Some(id), Some(OsStr::new(""))), None);
}

/// A variable documented as the equivalent of a per-launch flag counts as that flag: a launch
/// that sets only variables has per-process options, so it keeps its own process rather than
/// handing its files to the running instance, which never sees the override.
#[test]
fn an_environment_override_counts_as_a_process_option() {
    let with = |values: &[(&str, &str)]| env_launch_options_from(|n| values.iter().find(|(k, _)| *k == n).map(|(_, v)| (*v).into()));
    assert_eq!(with(&[("PHOTOCRAFT_PRESENT_MODE", "novsync")]), vec![PerProcess::PresentMode]);
    assert_eq!(with(&[("PHOTOCRAFT_STROKE_TRACE", "strokes.csv"), ("PHOTOCRAFT_FRAME_LATENCY", "2")]), vec![PerProcess::FrameLatency, PerProcess::StrokeTrace]);
    // Nothing set, or a value that means off, is an ordinary launch. Each variable is off the way
    // it is read where it takes effect, not by one rule shared across all three.
    assert!(with(&[]).is_empty());
    assert!(with(&[("PHOTOCRAFT_FRAME_LATENCY", "")]).is_empty());
    // The trace is off at `off` and `false` as well, and a value the presentation parser rejects
    // is ignored there, so neither keeps a process of its own here.
    assert!(with(&[("PHOTOCRAFT_STROKE_TRACE", "off"), ("PHOTOCRAFT_PRESENT_MODE", "maybe"), ("PHOTOCRAFT_FRAME_LATENCY", "4")]).is_empty());
    // The trace and the two presentation values are trimmed where they take effect.
    assert_eq!(with(&[("PHOTOCRAFT_PRESENT_MODE", " novsync "), ("PHOTOCRAFT_STROKE_TRACE", " 1 ")]), vec![PerProcess::PresentMode, PerProcess::StrokeTrace]);
    // The menus and the automation roots are flags' fallbacks: `launch_args::parse` counts them.
    assert!(with(&[("PHOTOCRAFT_IN_WINDOW_MENUS", "1"), ("PHOTOCRAFT_AUTOMATION_WRITE_ROOT", "/tmp/out")]).is_empty());
    let base = LaunchContext {
        per_user_settings: true,
        new_instance_flag: false,
        new_instance_env: None,
        control: false,
        unreadable_paths: false,
        launch_options: &[],
    };
    let reason = skip_reason(&LaunchContext { launch_options: &with(&[("PHOTOCRAFT_PRESENT_MODE", "novsync")]), ..base });
    assert!(reason.as_deref().is_some_and(|r| r.contains("--present-mode")), "{reason:?}");
}
