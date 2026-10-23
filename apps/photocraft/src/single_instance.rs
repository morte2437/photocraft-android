//! One PhotoCraft per user on Windows and Linux (#3708): a file opened from Explorer or a file
//! manager lands as a new tab in the running window, like Photoshop, instead of starting a second
//! copy of the program.
//!
//! The first launch takes an exclusive OS lock on `instance.lock` in the settings directory (the OS
//! drops it when the process ends, crash included), listens on a loopback port (inside a Flatpak
//! sandbox, on a Unix socket: see [`transport`]) and writes where it listens plus a fresh 256-bit
//! token to `instance.json`. A later launch that finds the lock taken sends its
//! paths there; the primary queues them for the UI as [`OsEvent::Open`], which opens each one and
//! brings the window forward. Explorer starts one process per file when several are opened at once;
//! the lock decides the race, so they all end up as tabs in one window.
//!
//! The primary answers `ok` only once the UI has taken the paths. A launch that hears anything else
//! (the primary is still starting, hung, or quitting) keeps retrying the lock, so it becomes the
//! primary itself as soon as the old one lets go, and when its time runs out it starts a separate
//! instance: a file is never acknowledged and then dropped. [`Primary::shutdown`] runs when the
//! event loop ends (and before the CPU-compatibility relaunch), so a closing PhotoCraft refuses new
//! files and frees the lock.
//!
//! Only for the per-user settings folder ([`skip_reason`]): a portable copy's folder beside the
//! executable, or a `PHOTOCRAFT_CONFIG_DIR`, may be shared between users.
//! macOS doesn't need this: LaunchServices already sends a running app an open-documents event.

// Only the tests use it on macOS.
#![cfg_attr(target_os = "macos", allow(dead_code))]

use std::fs::{File, OpenOptions};
use std::io::{BufReader, Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
#[cfg(unix)]
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{SyncSender, sync_channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use photocraft_automation::security::{LineRead, generate_token, read_bounded_line, token_matches};
use photocraft_ui_egui::{OsEvent, OsEventsFn};
use serde_json::{Value, json};

const LOCK_FILE: &str = "instance.lock";
const INFO_FILE: &str = "instance.json";
/// The Unix socket a sandboxed primary listens on, in the app's runtime directory.
#[cfg(unix)]
const SOCKET_FILE: &str = "instance.sock";
/// More paths than this in one request is not a file manager.
const MAX_PATHS: usize = 4096;
/// Launches served at once; a stalled client holds only its own slot, never the next launch.
const MAX_CLIENTS: usize = 8;
/// Kept out of the hand-off's budget, so the primary answers before the launch gives up on it.
const REPLY_MARGIN: Duration = Duration::from_millis(500);

/// How long each side waits (shortened in tests).
#[derive(Clone, Copy, Debug)]
pub struct Timing {
    /// A later launch's whole budget for handing its paths over (or taking the lock over).
    pub handoff: Duration,
    /// How long the primary waits for its UI to take a launch's paths before refusing them.
    pub ack: Duration,
    /// Total time a connection gets to send its request line.
    pub request: Duration,
}

/// Generous enough for a primary that is still opening its window.
pub const TIMING: Timing = Timing { handoff: Duration::from_secs(10), ack: Duration::from_secs(8), request: Duration::from_secs(5) };

/// How the primary listens for later launches.
#[derive(Clone, Debug, PartialEq)]
pub enum Transport {
    /// A loopback port (the default): private to the machine, reachable by every process on it.
    Loopback,
    /// A Unix socket at this path. Inside a Flatpak sandbox a loopback port is not enough: each
    /// `flatpak run` has its own network namespace, so the second sandbox's `127.0.0.1` never
    /// reaches the first (#1540). `$XDG_RUNTIME_DIR/app/<app id>/` is the same directory in every
    /// sandbox of the app, and the user's alone (0700).
    #[cfg(unix)]
    Socket(PathBuf),
}

/// The transport for this process: a Unix socket when running inside a Flatpak (`FLATPAK_ID` is
/// set and `XDG_RUNTIME_DIR` names the runtime directory), else a loopback port.
pub fn transport() -> Transport {
    #[cfg(unix)]
    if let Some(path) = sandbox_socket(std::env::var_os("FLATPAK_ID").as_deref(), std::env::var_os("XDG_RUNTIME_DIR").as_deref()) {
        return Transport::Socket(path);
    }
    Transport::Loopback
}

/// `$XDG_RUNTIME_DIR/app/<FLATPAK_ID>/instance.sock`, when both are set.
#[cfg(unix)]
fn sandbox_socket(flatpak_id: Option<&std::ffi::OsStr>, runtime_dir: Option<&std::ffi::OsStr>) -> Option<PathBuf> {
    let id = flatpak_id.filter(|v| !v.is_empty())?;
    let dir = runtime_dir.filter(|v| !v.is_empty())?;
    Some(Path::new(dir).join("app").join(id).join(SOCKET_FILE))
}

/// Where the running primary listens, as `instance.json` records it.
#[derive(Clone, Debug, PartialEq)]
enum Endpoint {
    Port(u16),
    #[cfg(unix)]
    Socket(PathBuf),
}

/// Outcome of [`claim`].
pub enum Launch {
    /// This process is the running instance; keep [`Primary`] alive and attach it to the app.
    Primary(Primary),
    /// The running instance took the paths; this process should exit.
    Forwarded,
    /// Couldn't do either (reason given): run as an independent instance.
    Standalone(String),
}

/// What decides whether a launch takes part (see [`skip_reason`]).
#[derive(Clone, Copy)]
pub struct LaunchContext<'a> {
    /// The settings folder is the platform's per-user one (not portable, not an override).
    pub per_user_settings: bool,
    /// `--new-instance`.
    pub new_instance_flag: bool,
    /// `PHOTOCRAFT_NEW_INSTANCE`, if set.
    pub new_instance_env: Option<&'a str>,
    /// `--control` or `PHOTOCRAFT_CONTROL_PORT`: automation gets its own process.
    pub control: bool,
    /// Paths that aren't valid Unicode: this launch reports them.
    pub unreadable_paths: bool,
    /// Options that only apply to the process they're given to (`--safe-gpu`, …), including the
    /// ones this launch's environment sets (see [`env_launch_options`]).
    pub launch_options: &'a [&'static str],
}

/// Variables documented as the equivalent of a per-launch flag, each with the flag it stands for
/// (`PHOTOCRAFT_PRESENT_MODE` is `--present-mode`) and the rule that says whether its value asks for
/// that option at all. Setting one asks for the same thing as giving the flag, so it belongs to this
/// process alone.
const ENV_LAUNCH_OPTIONS: [(&str, &str, ValueRule); 6] = [
    ("PHOTOCRAFT_PRESENT_MODE", "--present-mode", present_mode_is_on),
    ("PHOTOCRAFT_FRAME_LATENCY", "--frame-latency", frame_latency_is_on),
    ("PHOTOCRAFT_STROKE_TRACE", "--stroke-trace", stroke_trace_is_on),
    ("PHOTOCRAFT_IN_WINDOW_MENUS", "--in-window-menus", flag_is_on),
    ("PHOTOCRAFT_AUTOMATION_READ_ROOT", "--automation-read-root", path_is_set),
    ("PHOTOCRAFT_AUTOMATION_WRITE_ROOT", "--automation-write-root", path_is_set),
];

/// Whether a variable's value asks this launch for its own option. Each rule matches how that
/// variable is read where it takes effect, so a value ignored there keeps no process here either.
type ValueRule = fn(&str) -> bool;

/// A value `presentation::parse` accepts, handed over as written: it trims, treats empty as unset and
/// warns and ignores anything it does not recognise, so an unrecognised `PHOTOCRAFT_PRESENT_MODE`
/// changes nothing there and so needs no process of its own here.
fn present_mode_is_on(value: &str) -> bool {
    crate::presentation::parse(Some(value), None).0.mode.is_some()
}

/// [`present_mode_is_on`] for `PHOTOCRAFT_FRAME_LATENCY`, which takes `1` to `3`.
fn frame_latency_is_on(value: &str) -> bool {
    crate::presentation::parse(None, Some(value)).0.latency.is_some()
}

/// Anything except `""`, `0`, `off` and `false`, which is how `StrokeTrace::from_value` reads it: the
/// rest is a stderr summary or a CSV path. It trims, so `" 0 "` is off there and off here.
fn stroke_trace_is_on(value: &str) -> bool {
    !matches!(value.trim(), "" | "0" | "off" | "false")
}

/// Any value except empty and exactly `0`, as `main.rs` reads `PHOTOCRAFT_IN_WINDOW_MENUS` - which is
/// why `off` means on there, why this rule is not shared with the stroke trace above, and why it does
/// not trim: a padded `" 0 "` enables the menus there, so it keeps a process here too.
fn flag_is_on(value: &str) -> bool {
    !value.is_empty() && value != "0"
}

/// A path is an option; an empty value is no path. Not trimmed: a path may end in a space. An empty
/// root is read as no root unless `--control` is given, and that flag keeps its own process already.
fn path_is_set(value: &str) -> bool {
    !value.is_empty()
}

/// The per-process options this launch's environment sets, named as the flags they stand for. A
/// launch that sets only variables would otherwise hand its files to the running instance, which
/// never sees the override, and exit.
pub fn env_launch_options() -> Vec<&'static str> {
    env_launch_options_from(|name| std::env::var_os(name))
}

/// [`env_launch_options`] with the reader given, so the rule is testable without the environment.
fn env_launch_options_from(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Vec<&'static str> {
    ENV_LAUNCH_OPTIONS.iter().filter(|(name, _, is_on)| get(name).is_some_and(|v| is_on(&v.to_string_lossy()))).map(|(_, flag, _)| *flag).collect()
}

/// Why this launch runs as its own instance, or `None` to take part in single-instance.
pub fn skip_reason(c: &LaunchContext) -> Option<String> {
    if !c.per_user_settings {
        return Some("the settings folder isn't the per-user one (portable or PHOTOCRAFT_CONFIG_DIR)".into());
    }
    if c.new_instance_flag || c.new_instance_env.is_some_and(|v| !v.is_empty() && v != "0") {
        return Some("--new-instance".into());
    }
    if c.control {
        return Some("automation (--control) runs its own process".into());
    }
    if c.unreadable_paths {
        return Some("some paths aren't valid Unicode".into());
    }
    if !c.launch_options.is_empty() {
        return Some(format!("{} only applies to a new process", c.launch_options.join(", ")));
    }
    None
}

/// The running instance: holds the lock and receives other launches' paths.
pub struct Primary {
    lock: Mutex<Option<File>>,
    shared: Arc<Shared>,
    /// The socket file to remove on the first shutdown (a Unix socket transport only); taken
    /// then, so the second call (`Drop`) can't remove the socket the next primary has bound.
    #[cfg(unix)]
    socket: Mutex<Option<PathBuf>>,
}

struct Shared {
    inbox: Mutex<Inbox>,
    /// Set by [`Primary::shutdown`]: new connections are dropped, waiting ones refused.
    closed: AtomicBool,
}

#[derive(Default)]
struct Inbox {
    pending: Vec<Pending>,
    ctx: Option<egui::Context>,
    next_id: u64,
}

/// Paths waiting for the UI; `taken` tells the connection that sent them.
struct Pending {
    id: u64,
    paths: Vec<String>,
    taken: SyncSender<()>,
}

impl Primary {
    /// The queue the UI drains (see `Services::os_events`); a later launch wakes `ctx`.
    pub fn connect(&self, ctx: &egui::Context) -> OsEventsFn {
        let mut inbox = self.shared.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        if !inbox.pending.is_empty() {
            ctx.request_repaint();
        }
        inbox.ctx = Some(ctx.clone());
        drop(inbox);
        let shared = Arc::clone(&self.shared);
        Box::new(move || shared.take())
    }

    /// Stop taking files and release the lock: launches waiting on this one are refused and take
    /// the lock over themselves. Call when the event loop has ended, and before relaunching.
    pub fn shutdown(&self) {
        self.shared.closed.store(true, Ordering::Release);
        // Dropping the senders refuses every connection still waiting for the UI.
        self.shared.inbox.lock().unwrap_or_else(PoisonError::into_inner).pending.clear();
        #[cfg(unix)]
        if let Some(path) = self.socket.lock().unwrap_or_else(PoisonError::into_inner).take() {
            let _ = std::fs::remove_file(path);
        }
        drop(self.lock.lock().unwrap_or_else(PoisonError::into_inner).take());
    }
}

impl Drop for Primary {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Shared {
    /// Everything queued, acknowledged to the senders while the inbox is locked (see `hand_over`).
    fn take(&self) -> Vec<OsEvent> {
        let mut inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        inbox
            .pending
            .drain(..)
            .map(|p| {
                let _ = p.taken.try_send(());
                OsEvent::Open(p.paths)
            })
            .collect()
    }

    /// Queue `paths` for the UI and wait up to `wait` for it to take them. False (and nothing left
    /// queued) when it didn't: the sender then retries or starts its own instance.
    fn hand_over(&self, paths: Vec<String>, wait: Duration) -> bool {
        let (taken, rx) = sync_channel(1);
        let id = {
            let mut inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
            if self.closed.load(Ordering::Acquire) {
                return false;
            }
            let id = inbox.next_id;
            inbox.next_id = id.wrapping_add(1);
            inbox.pending.push(Pending { id, paths, taken });
            if let Some(ctx) = &inbox.ctx {
                ctx.request_repaint();
            }
            id
        };
        if rx.recv_timeout(wait).is_ok() {
            return true;
        }
        // Timed out or refused: withdraw, unless the UI took them in the meantime (it acknowledges
        // while holding the lock, so the answer is settled here).
        let mut inbox = self.inbox.lock().unwrap_or_else(PoisonError::into_inner);
        if rx.try_recv().is_ok() {
            return true;
        }
        inbox.pending.retain(|p| p.id != id);
        false
    }
}

/// Become the running instance under `dir` or hand `paths` (absolute) to the one that is.
pub fn claim(dir: &Path, paths: &[String]) -> Launch {
    claim_with(dir, paths, TIMING)
}

/// [`claim`] with explicit timeouts, over the transport this process would use ([`transport`]).
pub fn claim_with(dir: &Path, paths: &[String], timing: Timing) -> Launch {
    claim_in(dir, paths, timing, &transport())
}

/// [`claim`] over `transport`. Until `timing.handoff` runs out, each round tries the lock (free
/// once a quitting or crashed primary is gone) and then the running primary, wherever
/// `instance.json` says it listens.
pub fn claim_in(dir: &Path, paths: &[String], timing: Timing, transport: &Transport) -> Launch {
    if let Err(e) = std::fs::create_dir_all(dir) {
        return Launch::Standalone(format!("{}: {e}", dir.display()));
    }
    let lock_path = dir.join(LOCK_FILE);
    let lock = match OpenOptions::new().read(true).write(true).create(true).truncate(false).open(&lock_path) {
        Ok(lock) => lock,
        Err(e) => return Launch::Standalone(format!("{}: {e}", lock_path.display())),
    };
    let deadline = Instant::now() + timing.handoff;
    let mut last = "no instance.json".to_string();
    loop {
        match lock.try_lock() {
            Ok(()) => {
                return match serve(dir, lock, timing, transport) {
                    Ok(primary) => Launch::Primary(primary),
                    Err(e) => Launch::Standalone(e),
                };
            }
            Err(std::fs::TryLockError::WouldBlock) => {}
            Err(std::fs::TryLockError::Error(e)) => return Launch::Standalone(format!("{}: {e}", lock_path.display())),
        }
        if let Some((endpoint, token)) = read_info(dir) {
            match send(&endpoint, &token, paths, deadline) {
                Ok(()) => return Launch::Forwarded,
                Err(e) => last = e,
            }
        }
        if Instant::now() >= deadline {
            return Launch::Standalone(format!("the running PhotoCraft didn't take the files ({last})"));
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn serve(dir: &Path, lock: File, timing: Timing, transport: &Transport) -> Result<Primary, String> {
    let (listener, endpoint) = Listener::bind(transport)?;
    let token = generate_token().map_err(|e| e.to_string())?;
    write_info(dir, &endpoint, &token)?;
    #[cfg(unix)]
    let socket = match &endpoint {
        Endpoint::Socket(path) => Some(path.clone()),
        Endpoint::Port(_) => None,
    };
    let shared = Arc::new(Shared { inbox: Mutex::default(), closed: AtomicBool::new(false) });
    let listening = Arc::clone(&shared);
    let spawned = std::thread::Builder::new().name("single-instance".into()).spawn(move || {
        let token = Arc::new(token);
        let clients = Arc::new(AtomicUsize::new(0));
        loop {
            let stream = match listener.accept() {
                Ok(stream) => stream,
                // Back off instead of spinning when accept keeps failing (e.g. out of descriptors).
                Err(_) => {
                    std::thread::sleep(Duration::from_millis(100));
                    continue;
                }
            };
            // Closing: the sender sees the connection drop and takes the lock over.
            if listening.closed.load(Ordering::Acquire) {
                continue;
            }
            // Each launch on its own thread, so one that connects and stalls doesn't hold the
            // ones after it; past the cap a connection is dropped and that launch retries.
            if clients.fetch_add(1, Ordering::AcqRel) >= MAX_CLIENTS {
                clients.fetch_sub(1, Ordering::AcqRel);
                continue;
            }
            let (token, shared, slot) = (Arc::clone(&token), Arc::clone(&listening), Arc::clone(&clients));
            let spawned = std::thread::Builder::new().name("single-instance-client".into()).spawn(move || {
                serve_client(stream, &token, &shared, timing);
                slot.fetch_sub(1, Ordering::AcqRel);
            });
            if spawned.is_err() {
                clients.fetch_sub(1, Ordering::AcqRel);
            }
        }
    });
    spawned.map_err(|e| format!("single-instance listener thread: {e}"))?;
    Ok(Primary {
        lock: Mutex::new(Some(lock)),
        shared,
        #[cfg(unix)]
        socket: Mutex::new(socket),
    })
}

/// The primary's listener on either transport.
enum Listener {
    Tcp(TcpListener),
    #[cfg(unix)]
    Unix(UnixListener),
}

impl Listener {
    fn bind(transport: &Transport) -> Result<(Listener, Endpoint), String> {
        match transport {
            Transport::Loopback => {
                let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).map_err(|e| format!("loopback listener: {e}"))?;
                let port = listener.local_addr().map_err(|e| format!("loopback listener: {e}"))?.port();
                Ok((Listener::Tcp(listener), Endpoint::Port(port)))
            }
            #[cfg(unix)]
            Transport::Socket(path) => {
                let at = |e: std::io::Error| format!("{}: {e}", path.display());
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent).map_err(at)?;
                }
                // This process holds the lock, so a file already there is a crashed primary's.
                let _ = std::fs::remove_file(path);
                let listener = UnixListener::bind(path).map_err(at)?;
                // The runtime directory is the user's alone already; keep the socket so too.
                std::fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(0o600)).map_err(at)?;
                Ok((Listener::Unix(listener), Endpoint::Socket(path.clone())))
            }
        }
    }

    fn accept(&self) -> std::io::Result<Stream> {
        match self {
            Listener::Tcp(l) => l.accept().map(|(s, _)| Stream::Tcp(s)),
            #[cfg(unix)]
            Listener::Unix(l) => l.accept().map(|(s, _)| Stream::Unix(s)),
        }
    }
}

/// One connection, on either transport.
enum Stream {
    Tcp(TcpStream),
    #[cfg(unix)]
    Unix(UnixStream),
}

impl Stream {
    /// Connect to the primary; `timeout` bounds the loopback connect (a Unix connect is immediate).
    fn connect(endpoint: &Endpoint, timeout: Duration) -> std::io::Result<Stream> {
        match endpoint {
            Endpoint::Port(port) => TcpStream::connect_timeout(&SocketAddr::from((Ipv4Addr::LOCALHOST, *port)), timeout).map(Stream::Tcp),
            #[cfg(unix)]
            Endpoint::Socket(path) => UnixStream::connect(path).map(Stream::Unix),
        }
    }

    fn set_read_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        match self {
            Stream::Tcp(s) => s.set_read_timeout(dur),
            #[cfg(unix)]
            Stream::Unix(s) => s.set_read_timeout(dur),
        }
    }

    fn set_write_timeout(&self, dur: Option<Duration>) -> std::io::Result<()> {
        match self {
            Stream::Tcp(s) => s.set_write_timeout(dur),
            #[cfg(unix)]
            Stream::Unix(s) => s.set_write_timeout(dur),
        }
    }

    /// One read through a shared reference (both socket types read through `&self`).
    fn read_once(&self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            Stream::Tcp(s) => (&*s).read(buf),
            #[cfg(unix)]
            Stream::Unix(s) => (&*s).read(buf),
        }
    }
}

impl Write for Stream {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            Stream::Tcp(s) => s.write(buf),
            #[cfg(unix)]
            Stream::Unix(s) => s.write(buf),
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            Stream::Tcp(s) => s.flush(),
            #[cfg(unix)]
            Stream::Unix(s) => s.flush(),
        }
    }
}

/// Read one request, hand its paths to the UI and answer `ok` only if the UI took them.
fn serve_client(stream: Stream, token: &str, shared: &Shared, timing: Timing) {
    let taken = match read_request(&stream, timing.request) {
        Some(line) => match parse_request(&line, token) {
            Some((paths, wait)) => shared.hand_over(paths, wait.min(timing.ack)),
            None => false,
        },
        None => return,
    };
    let reply = if taken { json!({"ok": true}) } else { json!({"ok": false, "error": "not taken"}) };
    let mut out = stream;
    let _ = out.set_write_timeout(Some(timing.request));
    let _ = writeln!(out, "{reply}").and_then(|()| out.flush());
}

/// A reader that fails once `until` passes, however slowly the bytes trickle in.
struct Deadline<'a> {
    stream: &'a Stream,
    until: Instant,
}

impl Read for Deadline<'_> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        let left = self.until.checked_duration_since(Instant::now()).filter(|d| !d.is_zero());
        let left = left.ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "request deadline passed"))?;
        self.stream.set_read_timeout(Some(left))?;
        self.stream.read_once(buf)
    }
}

/// The request line, if it arrives complete within `total`.
fn read_request(stream: &Stream, total: Duration) -> Option<String> {
    let mut reader = BufReader::new(Deadline { stream, until: Instant::now() + total });
    let mut line = String::new();
    matches!(read_bounded_line(&mut reader, &mut line), Ok(LineRead::Line)).then_some(line)
}

/// Endpoint and token, written to a temporary file and renamed so a reader never sees half of it.
/// On Unix the file is the user's alone (0600): another local user who read the token could make
/// this app open any path. (Windows keeps the per-user ACL of the settings folder.)
fn write_info(dir: &Path, endpoint: &Endpoint, token: &str) -> Result<(), String> {
    let path = dir.join(INFO_FILE);
    let tmp = dir.join(format!("{INFO_FILE}.{}.tmp", std::process::id()));
    let mut body = json!({"token": token, "pid": std::process::id()});
    match endpoint {
        Endpoint::Port(port) => body["port"] = json!(port),
        #[cfg(unix)]
        Endpoint::Socket(socket) => body["socket"] = json!(socket.to_string_lossy()),
    }
    let body = body.to_string();
    // A leftover from a crash could carry wider permissions; the mode applies only on create.
    let _ = std::fs::remove_file(&tmp);
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut options, 0o600);
    let written = options.open(&tmp).and_then(|mut f| f.write_all(body.as_bytes())).and_then(|()| std::fs::rename(&tmp, &path));
    written.map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {e}", path.display())
    })
}

fn read_info(dir: &Path) -> Option<(Endpoint, String)> {
    let v: Value = serde_json::from_slice(&std::fs::read(dir.join(INFO_FILE)).ok()?).ok()?;
    let token = v.get("token")?.as_str()?.to_string();
    #[cfg(unix)]
    if let Some(socket) = v.get("socket").and_then(Value::as_str) {
        return Some((Endpoint::Socket(PathBuf::from(socket)), token));
    }
    let port = u16::try_from(v.get("port")?.as_u64()?).ok()?;
    Some((Endpoint::Port(port), token))
}

/// `{"token":…,"open":[…],"wait_ms":…}`: the paths, and how long the sender will wait for the UI.
fn parse_request(line: &str, token: &str) -> Option<(Vec<String>, Duration)> {
    let v: Value = serde_json::from_str(line).ok()?;
    if !token_matches(token, v.get("token")?.as_str()?) {
        return None;
    }
    let open = v.get("open")?.as_array()?;
    if open.len() > MAX_PATHS {
        return None;
    }
    let paths = open.iter().map(|p| p.as_str().map(str::to_string)).collect::<Option<Vec<_>>>()?;
    let wait = Duration::from_millis(v.get("wait_ms").and_then(Value::as_u64).unwrap_or(0));
    Some((paths, wait))
}

/// Offer `paths` to the primary at `endpoint`; `Ok` once its UI has taken them.
fn send(endpoint: &Endpoint, token: &str, paths: &[String], deadline: Instant) -> Result<(), String> {
    let left = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero()).ok_or("out of time")?;
    let mut stream = Stream::connect(endpoint, left).map_err(|e| e.to_string())?;
    // The primary must answer before our deadline, so it waits a little less than we do.
    let wait_ms = u64::try_from(left.saturating_sub(REPLY_MARGIN).as_millis()).unwrap_or(u64::MAX);
    let request = json!({"token": token, "open": paths, "wait_ms": wait_ms});
    stream.set_write_timeout(Some(left)).map_err(|e| e.to_string())?;
    writeln!(stream, "{request}").and_then(|()| stream.flush()).map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(Deadline { stream: &stream, until: deadline });
    let mut line = String::new();
    match read_bounded_line(&mut reader, &mut line) {
        Ok(LineRead::Line) => {}
        Ok(_) => return Err("no reply".into()),
        Err(e) => return Err(e.to_string()),
    }
    let ok = serde_json::from_str::<Value>(&line).ok().and_then(|v| v.get("ok")?.as_bool()) == Some(true);
    if ok { Ok(()) } else { Err("not taken".into()) }
}

/// Absolute form of a command-line path, so the running instance (another working directory)
/// opens the same file.
pub fn absolute(path: &str) -> String {
    std::path::absolute(path).map(|p| p.to_string_lossy().into_owned()).unwrap_or_else(|_| path.to_string())
}

#[cfg(test)]
mod tests {
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
        let reason = skip_reason(&LaunchContext { launch_options: &["--safe-gpu"], ..base });
        assert!(reason.is_some_and(|r| r.contains("--safe-gpu")));
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
        assert_eq!(with(&[("PHOTOCRAFT_PRESENT_MODE", "novsync")]), vec!["--present-mode"]);
        assert_eq!(
            with(&[("PHOTOCRAFT_STROKE_TRACE", "strokes.csv"), ("PHOTOCRAFT_AUTOMATION_WRITE_ROOT", "/tmp/out")]),
            vec!["--stroke-trace", "--automation-write-root"]
        );
        // Nothing set, or a value that means off, is an ordinary launch. Each variable is off the way
        // it is read where it takes effect, not by one rule shared across all six.
        assert!(with(&[]).is_empty());
        assert!(with(&[("PHOTOCRAFT_IN_WINDOW_MENUS", "0"), ("PHOTOCRAFT_FRAME_LATENCY", "")]).is_empty());
        // The trace is off at `off` and `false` as well, and a value the presentation parser rejects
        // is ignored there, so neither keeps a process of its own here.
        assert!(with(&[("PHOTOCRAFT_STROKE_TRACE", "off"), ("PHOTOCRAFT_PRESENT_MODE", "maybe"), ("PHOTOCRAFT_FRAME_LATENCY", "4")]).is_empty());
        assert!(with(&[("PHOTOCRAFT_AUTOMATION_READ_ROOT", "")]).is_empty());
        // Spaces are part of the value only where the reader keeps them: the trace and the two
        // presentation values are trimmed where they take effect, the menu flag and a path are not.
        assert_eq!(with(&[("PHOTOCRAFT_PRESENT_MODE", " novsync "), ("PHOTOCRAFT_STROKE_TRACE", " 1 ")]), vec!["--present-mode", "--stroke-trace"]);
        assert_eq!(
            with(&[("PHOTOCRAFT_IN_WINDOW_MENUS", " 0 "), ("PHOTOCRAFT_AUTOMATION_WRITE_ROOT", " /tmp/out ")]),
            vec!["--in-window-menus", "--automation-write-root"]
        );
        // A whitespace-only menu value is not empty, so it enables the menus there and here too.
        assert_eq!(with(&[("PHOTOCRAFT_IN_WINDOW_MENUS", " ")]), vec!["--in-window-menus"]);
        // `PHOTOCRAFT_IN_WINDOW_MENUS=off` means menus in the window there, so it means it here too.
        assert_eq!(with(&[("PHOTOCRAFT_IN_WINDOW_MENUS", "off")]), vec!["--in-window-menus"]);
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
}
