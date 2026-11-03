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

use crate::launch_args::PerProcess;

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
    pub launch_options: &'a [PerProcess],
}

/// Variables documented as the equivalent of a per-launch flag, each with the option it stands for
/// (`PHOTOCRAFT_PRESENT_MODE` is `--present-mode`) and the rule that says whether its value asks for
/// that option at all. Setting one asks for the same thing as giving the flag, so it belongs to this
/// process alone. These are read where they take effect, not by the command line; the variables
/// that are a flag's fallback (`PHOTOCRAFT_IN_WINDOW_MENUS`, the automation roots) are counted by
/// [`crate::launch_args::parse`], as clap reads them.
const ENV_LAUNCH_OPTIONS: [(&str, PerProcess, ValueRule); 3] = [
    ("PHOTOCRAFT_PRESENT_MODE", PerProcess::PresentMode, present_mode_is_on),
    ("PHOTOCRAFT_FRAME_LATENCY", PerProcess::FrameLatency, frame_latency_is_on),
    ("PHOTOCRAFT_STROKE_TRACE", PerProcess::StrokeTrace, stroke_trace_is_on),
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

/// The per-process options this launch's environment sets. A launch that sets only variables would
/// otherwise hand its files to the running instance, which never sees the override, and exit.
pub fn env_launch_options() -> Vec<PerProcess> {
    env_launch_options_from(|name| std::env::var_os(name))
}

/// [`env_launch_options`] with the reader given, so the rule is testable without the environment.
fn env_launch_options_from(get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Vec<PerProcess> {
    ENV_LAUNCH_OPTIONS.iter().filter(|(name, _, is_on)| get(name).is_some_and(|v| is_on(&v.to_string_lossy()))).map(|(_, option, _)| *option).collect()
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
        let options: Vec<String> = c.launch_options.iter().map(ToString::to_string).collect();
        return Some(format!("{} only applies to a new process", options.join(", ")));
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
mod tests;
