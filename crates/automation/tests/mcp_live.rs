//! Plain `photocraft-cli mcp` with agents allowed (the ArtCraft Launcher's switch): the MCP server finds the desktop
//! app through its connection file before every call, follows it to a new port and token, and
//! applies the closed-app choice (start it, headless, or an error) when no app answers.

use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use photocraft_automation::PhotocraftMcp;
use photocraft_automation::agent_link::{self, Connection};
use photocraft_automation::live::LiveConfig;
use photocraft_engine::prefs::WhenAppClosed;
use rmcp::model::{CallToolRequestParams, CallToolResult, ClientConfig};
use rmcp::service::RunningService;
use rmcp::{ClientHandler, RoleClient, ServiceExt};
use serde_json::{Value, json};

#[derive(Clone, Default)]
struct Client;
impl ClientHandler for Client {
    fn get_info(&self) -> ClientConfig {
        ClientConfig::default()
    }
}

async fn connect(server: PhotocraftMcp) -> RunningService<RoleClient, Client> {
    let (s, c) = tokio::io::duplex(1 << 20);
    tokio::spawn(async move {
        if let Ok(running) = server.serve(s).await {
            let _ = running.waiting().await;
        }
    });
    Client.serve(c).await.expect("client init")
}

async fn call(client: &RunningService<RoleClient, Client>, name: &str, args: Value) -> CallToolResult {
    let mut p = CallToolRequestParams::new(name.to_owned());
    if let Value::Object(m) = args {
        p = p.with_arguments(m);
    }
    client.call_tool(p).await.expect("call_tool transport")
}

fn text(r: &CallToolResult) -> String {
    r.content.iter().filter_map(|c| c.as_text()).map(|t| t.text.clone()).collect::<Vec<_>>().join("\n")
}

fn settings_dir(tag: &str) -> PathBuf {
    static N: AtomicUsize = AtomicUsize::new(0);
    let d = std::env::temp_dir().join(format!("pc-mcp-live-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn prefs(dir: &Path, integrations: Value) {
    std::fs::write(dir.join("preferences.json"), json!({"integrations": integrations}).to_string()).unwrap();
}

/// A desktop app as `mcp` sees it: a control server on a free port with a fresh token, its
/// connection file published in `dir`. `ui.inspect` answers with `name`. [`quit`] stops it.
struct FakeApp {
    port: u16,
    stop: Arc<std::sync::atomic::AtomicBool>,
}

fn fake_app(dir: &Path, name: &'static str) -> FakeApp {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let token_file = dir.join(agent_link::TOKEN_FILE);
    let token = photocraft_automation::security::replace_token_file(&token_file).unwrap();
    agent_link::write_connection(dir, &Connection::new(port, token_file)).unwrap();
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stopped = stop.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            if stopped.load(Ordering::SeqCst) {
                break;
            }
            let token = token.clone();
            std::thread::spawn(move || {
                let mut out = stream.try_clone().unwrap();
                // The app's own handshake (challenge or token), as `control_server` runs it.
                let mut handshake = photocraft_automation::security::ServerHandshake::new();
                let mut authenticated = false;
                for line in BufReader::new(stream).lines() {
                    let Ok(line) = line else { break };
                    if !authenticated {
                        let (reply, step) = handshake.reply(&line, &token);
                        authenticated = step == photocraft_automation::security::AuthStep::Authenticated;
                        if writeln!(out, "{reply}").is_err() || step == photocraft_automation::security::AuthStep::Rejected {
                            break;
                        }
                        continue;
                    }
                    let req: Value = serde_json::from_str(&line).unwrap();
                    let id = req["id"].clone();
                    let reply = match req["method"].as_str().unwrap_or("") {
                        "ui.inspect" => json!({"id": id, "ok": true, "result": {"app": name}}),
                        m => json!({"id": id, "ok": false, "error": format!("unexpected {m}")}),
                    };
                    if writeln!(out, "{reply}").is_err() {
                        break;
                    }
                }
            });
        }
    });
    FakeApp { port, stop }
}

/// The fake app quits: its port stops accepting (connections already open end with the test).
fn quit(app: FakeApp) {
    app.stop.store(true, Ordering::SeqCst);
    let _ = std::net::TcpStream::connect(("127.0.0.1", app.port));
    while agent_link::probe(app.port, Duration::from_millis(200)) {
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn live(dir: &Path, when_closed: Option<WhenAppClosed>) -> LiveConfig {
    let mut c = LiveConfig::new(dir.to_path_buf());
    c.when_closed = when_closed;
    c.launch_timeout = Duration::from_secs(10);
    // The machine's own ArtCraft Launcher and feature flags stay out of the tests: agents are
    // allowed exactly when the test's preferences say so (as without the AI-MCP flag).
    c.allowed = |i| i.connect_agents;
    c
}

#[tokio::test(flavor = "multi_thread")]
async fn a_closed_app_is_a_tool_error_that_says_what_to_do() {
    let dir = settings_dir("closed");
    prefs(&dir, json!({"connectAgents": true}));
    let client = connect(PhotocraftMcp::live(live(&dir, Some(WhenAppClosed::Error)))).await;
    // The tool list doesn't need the app.
    assert!(client.list_all_tools().await.unwrap().iter().any(|t| t.name == "ui_inspect"));
    let r = call(&client, "command_list", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("isn't running"), "{}", text(&r));
    // A connection file left by a crashed app (nothing listens) counts as closed.
    let gone = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = gone.local_addr().unwrap().port();
    drop(gone);
    agent_link::write_connection(&dir, &Connection::new(port, dir.join(agent_link::TOKEN_FILE))).unwrap();
    let r = call(&client, "ui_inspect", json!({})).await;
    assert!(text(&r).contains("isn't running"), "{}", text(&r));
    client.cancel().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn follows_the_app_across_restarts_on_new_ports_and_tokens() {
    let dir = settings_dir("follow");
    prefs(&dir, json!({"connectAgents": true, "whenAppClosed": "error"}));
    let client = connect(PhotocraftMcp::live(live(&dir, None))).await;
    let first = fake_app(&dir, "first");
    let r = call(&client, "ui_inspect", json!({})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    assert!(text(&r).contains("first"));
    // Restart: another port, a new token.
    quit(first);
    let second = fake_app(&dir, "second");
    let r = call(&client, "ui_inspect", json!({})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    assert!(text(&r).contains("second"), "{}", text(&r));
    quit(second);
    std::fs::remove_file(dir.join(agent_link::CONNECTION_FILE)).unwrap();
    let r = call(&client, "ui_inspect", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    client.cancel().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn launch_starts_the_app_once_and_only_when_agents_may_connect() {
    let dir = settings_dir("launch");
    let launches = Arc::new(AtomicUsize::new(0));
    let apps = Arc::new(std::sync::Mutex::new(Vec::new()));
    let config = |dir: &Path| {
        let mut c = live(dir, Some(WhenAppClosed::Launch));
        let (launches, apps, dir) = (launches.clone(), apps.clone(), dir.to_path_buf());
        c.launch = Some(Box::new(move || {
            launches.fetch_add(1, Ordering::SeqCst);
            let (apps, dir) = (apps.clone(), dir.clone());
            // The app takes a moment to start listening.
            std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(300));
                apps.lock().unwrap().push(fake_app(&dir, "launched"));
            });
            Ok(())
        }));
        c
    };
    // Agents not allowed: the app would never listen, so it isn't started; the session works
    // without a window, as `mcp` always did.
    let client = connect(PhotocraftMcp::live(config(&dir))).await;
    let r = call(&client, "ui_inspect", json!({})).await;
    assert!(text(&r).contains("bridge"), "UI tools need the open app: {}", text(&r));
    let r = call(&client, "command_list", json!({})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    assert_eq!(launches.load(Ordering::SeqCst), 0);
    client.cancel().await.unwrap();

    // The ArtCraft Launcher's switch is the consent: the app is started on it alone.
    let mut c = config(&dir);
    c.allowed = |_| true;
    let client = connect(PhotocraftMcp::live(c)).await;
    let r = call(&client, "ui_inspect", json!({})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    client.cancel().await.unwrap();
    for app in apps.lock().unwrap().drain(..) {
        quit(app);
    }
    launches.store(0, Ordering::SeqCst);

    prefs(&dir, json!({"connectAgents": true}));
    let client = connect(PhotocraftMcp::live(config(&dir))).await;
    // Two calls at once start one app.
    let (a, b) = tokio::join!(call(&client, "ui_inspect", json!({})), call(&client, "ui_inspect", json!({})));
    for r in [a, b] {
        assert_eq!(r.is_error, Some(false), "{}", text(&r));
        assert!(text(&r).contains("launched"));
    }
    assert_eq!(launches.load(Ordering::SeqCst), 1);
    client.cancel().await.unwrap();
    for app in apps.lock().unwrap().drain(..) {
        quit(app);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_launch_that_never_listens_times_out_with_advice() {
    let dir = settings_dir("timeout");
    prefs(&dir, json!({"connectAgents": true}));
    let mut c = live(&dir, Some(WhenAppClosed::Launch));
    c.launch_timeout = Duration::from_millis(600);
    c.launch = Some(Box::new(|| Ok(())));
    let client = connect(PhotocraftMcp::live(c)).await;
    let r = call(&client, "ui_inspect", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("didn't accept agents"), "{}", text(&r));
    // A launcher that fails says so.
    let mut c = live(&dir, Some(WhenAppClosed::Launch));
    c.launch = Some(Box::new(|| Err("no such file".into())));
    let client2 = connect(PhotocraftMcp::live(c)).await;
    let r = call(&client2, "ui_inspect", json!({})).await;
    assert!(text(&r).contains("couldn't start PhotoCraft: no such file"), "{}", text(&r));
    // No launcher (Flatpak, the app not installed beside the CLI): ask the user.
    let client3 = connect(PhotocraftMcp::live(live(&dir, Some(WhenAppClosed::Launch)))).await;
    let r = call(&client3, "ui_inspect", json!({})).await;
    assert!(text(&r).contains("open PhotoCraft"), "{}", text(&r));
    for c in [client, client2, client3] {
        c.cancel().await.unwrap();
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[tokio::test(flavor = "multi_thread")]
async fn headless_uses_the_app_folders_and_stays_headless() {
    let dir = settings_dir("headless");
    let work = dir.join("work");
    std::fs::create_dir_all(&work).unwrap();
    prefs(&dir, json!({"connectAgents": true, "whenAppClosed": "headless", "writeRoot": work.to_string_lossy()}));
    let client = connect(PhotocraftMcp::live(live(&dir, None))).await;
    let r = call(&client, "doc_new", json!({"width": 8, "height": 8})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    let r = call(&client, "doc_save", json!({"path": "out.pcraft"})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    assert!(work.join("out.pcraft").exists(), "saved into the app's write folder");
    // No read folder was chosen: reads are refused.
    let r = call(&client, "doc_open", json!({"path": "out.pcraft"})).await;
    assert_eq!(r.is_error, Some(true));
    // The app starting later doesn't move the documents away from the agent.
    let app = fake_app(&dir, "late");
    let r = call(&client, "ui_inspect", json!({})).await;
    assert!(text(&r).contains("bridge"), "UI tools stay unavailable in the headless session: {}", text(&r));
    let r = call(&client, "session_list", json!({})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    client.cancel().await.unwrap();
    quit(app);
    // A folder that doesn't exist is a clear error, not a crash.
    prefs(&dir, json!({"connectAgents": true, "whenAppClosed": "headless", "readRoot": dir.join("missing").to_string_lossy()}));
    std::fs::remove_file(dir.join(agent_link::CONNECTION_FILE)).unwrap();
    let client = connect(PhotocraftMcp::live(live(&dir, None))).await;
    let r = call(&client, "command_list", json!({})).await;
    assert_eq!(r.is_error, Some(true));
    assert!(text(&r).contains("agent folders"), "{}", text(&r));
    client.cancel().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Agents allowed, app closed, headless: a root flag given to `mcp` replaces the app's folder for
/// its own direction only; the other direction still falls back to the app's folder.
#[tokio::test(flavor = "multi_thread")]
async fn headless_root_flags_override_each_direction_on_their_own() {
    let dir = settings_dir("headless-override");
    let (app_read, app_write, given) = (dir.join("app-read"), dir.join("app-write"), dir.join("given"));
    for d in [&app_read, &app_write, &given] {
        std::fs::create_dir_all(d).unwrap();
    }
    prefs(&dir, json!({"connectAgents": true, "whenAppClosed": "headless", "readRoot": app_read.to_string_lossy(), "writeRoot": app_write.to_string_lossy()}));
    // Only the write root is given: saves go there, reads still come from the app's read folder.
    let mut c = live(&dir, None);
    c.write_root = Some(given.clone());
    let client = connect(PhotocraftMcp::live(c)).await;
    let r = call(&client, "doc_new", json!({"width": 8, "height": 8})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    let r = call(&client, "doc_save", json!({"path": "out.pcraft"})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    assert!(given.join("out.pcraft").exists(), "saved into the given write root");
    assert!(!app_write.join("out.pcraft").exists(), "not into the app's write folder");
    std::fs::copy(given.join("out.pcraft"), app_read.join("in.pcraft")).unwrap();
    let r = call(&client, "doc_open", json!({"path": app_read.join("in.pcraft").to_string_lossy()})).await;
    assert_eq!(r.is_error, Some(false), "the app's read folder still applies: {}", text(&r));
    client.cancel().await.unwrap();
    // Only a read root is given: it replaces the app's read folder; saves still go to the app's write folder.
    let mut c = live(&dir, None);
    c.read_roots = vec![given.clone()];
    let client = connect(PhotocraftMcp::live(c)).await;
    let r = call(&client, "doc_open", json!({"path": app_read.join("in.pcraft").to_string_lossy()})).await;
    assert_eq!(r.is_error, Some(true), "the app's read folder is replaced by the given one");
    let r = call(&client, "doc_open", json!({"path": given.join("out.pcraft").to_string_lossy()})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    let r = call(&client, "doc_save", json!({"path": "again.pcraft"})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    assert!(app_write.join("again.pcraft").exists(), "saved into the app's write folder");
    client.cancel().await.unwrap();
    let _ = std::fs::remove_dir_all(&dir);
}

/// Agents not allowed: `mcp` serves a session without a window as it always did, with only the
/// folders given to the command (the app's own folders are for agents the person let in), never
/// starts the app, and prepares the session as the command asked (`--gpu`).
#[tokio::test(flavor = "multi_thread")]
async fn without_consent_mcp_is_headless_with_its_own_folders() {
    static PREPARED: AtomicUsize = AtomicUsize::new(0);
    let dir = settings_dir("no-consent");
    let (app_folder, given) = (dir.join("app-folder"), dir.join("given"));
    std::fs::create_dir_all(&app_folder).unwrap();
    std::fs::create_dir_all(&given).unwrap();
    prefs(&dir, json!({"whenAppClosed": "launch", "writeRoot": app_folder.to_string_lossy()}));
    let launches = Arc::new(AtomicUsize::new(0));
    let config = |write_root: Option<PathBuf>| {
        let mut c = live(&dir, None);
        c.write_root = write_root;
        let launches = launches.clone();
        c.launch = Some(Box::new(move || {
            launches.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }));
        c.prepare_headless = Some(Box::new(|_| {
            PREPARED.fetch_add(1, Ordering::SeqCst);
        }));
        c
    };
    let client = connect(PhotocraftMcp::live(config(None))).await;
    let r = call(&client, "doc_new", json!({"width": 8, "height": 8})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    let r = call(&client, "doc_save", json!({"path": "out.pcraft"})).await;
    assert_eq!(r.is_error, Some(true), "the app's folder isn't granted without consent");
    assert!(!app_folder.join("out.pcraft").exists());
    client.cancel().await.unwrap();
    let client = connect(PhotocraftMcp::live(config(Some(given.clone())))).await;
    call(&client, "doc_new", json!({"width": 8, "height": 8})).await;
    let r = call(&client, "doc_save", json!({"path": "out.pcraft"})).await;
    assert_eq!(r.is_error, Some(false), "{}", text(&r));
    assert!(given.join("out.pcraft").exists(), "the command's own root is used");
    client.cancel().await.unwrap();
    assert_eq!(launches.load(Ordering::SeqCst), 0, "never started without consent");
    assert_eq!(PREPARED.load(Ordering::SeqCst), 2, "each headless session is prepared once");
    let _ = std::fs::remove_dir_all(&dir);
}
