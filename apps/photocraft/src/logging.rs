//! The desktop app's logger: `log::` records go to standard error and to a log file.
//!
//! Without a logger every `log::warn!`/`log::error!` in the workspace (GPU startup fallbacks, a
//! lost device, the crash guard's panic report) vanished. A launch from a desktop menu has no
//! terminal, so the file is what a bug report can attach: `<settings dir>/logs/photocraft.log`
//! (see `app_dirs`). Each start moves the previous log to `photocraft.1.log` (and that one to
//! `.2`), so the log of a run that crashed survives the next launch. A start while another
//! instance is still running leaves that instance's log alone and writes `photocraft-<pid>.log`
//! (see [`rotate`]).
//!
//! Levels: `info` for PhotoCraft's own crates, `warn` for everything else (wgpu and naga are
//! chatty). `RUST_LOG` replaces that with env_logger-style directives: `debug`,
//! `warn,photocraft_gpu=trace`, `wgpu_core=info`. A directive ending in `*` matches every target
//! that starts with it (`photocraft*=debug`).
//!
//! Records logged before the settings directory is known are kept (up to [`MAX_PENDING`]) and
//! written once the file is attached. Writing never panics: a file that can't be created or
//! written leaves standard error as the only sink.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::SystemTime;

use log::LevelFilter;

/// The current log file's name inside the log directory.
pub const LOG_FILE: &str = "photocraft.log";
/// How many previous logs are kept (`photocraft.1.log` … `photocraft.<KEEP>.log`).
pub const KEEP: usize = 2;
/// The log file stops growing past this size (a runaway warning can't fill the disk).
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Records kept in memory until the log file is attached.
pub const MAX_PENDING: usize = 512;
/// The built-in filter when `RUST_LOG` is unset or empty.
pub const DEFAULT_FILTER: &str = "warn,photocraft*=info";

/// Per-target level filter parsed from env_logger-style directives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    default: LevelFilter,
    directives: Vec<(String, LevelFilter)>,
}

impl Filter {
    /// Parse `spec`; unknown levels and empty parts are skipped. Without a bare level, targets no
    /// directive names log errors only.
    pub fn parse(spec: &str) -> Filter {
        let mut f = Filter { default: LevelFilter::Error, directives: Vec::new() };
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some((name, level)) => {
                    let name = name.trim();
                    if let (false, Some(level)) = (name.is_empty(), parse_level(level)) {
                        f.directives.push((name.to_owned(), level));
                    }
                }
                None => match parse_level(part) {
                    Some(level) => f.default = level,
                    // A bare target name: everything from it (env_logger does the same).
                    None => f.directives.push((part.to_owned(), LevelFilter::Trace)),
                },
            }
        }
        f
    }

    /// The level that applies to `target` (the most specific matching directive wins).
    pub fn level_for(&self, target: &str) -> LevelFilter {
        self.directives.iter().filter(|(name, _)| matches(name, target)).max_by_key(|(name, _)| name.len()).map_or(self.default, |(_, level)| *level)
    }

    /// The most verbose level any target can reach (for `log::set_max_level`).
    pub fn max(&self) -> LevelFilter {
        self.directives.iter().map(|(_, level)| *level).fold(self.default, Ord::max)
    }
}

fn parse_level(s: &str) -> Option<LevelFilter> {
    s.trim().parse().ok()
}

/// `name` matches `target` itself and its submodules (`a` matches `a` and `a::b`, not `ab`);
/// `name*` matches every target starting with `name`.
fn matches(name: &str, target: &str) -> bool {
    match name.strip_suffix('*') {
        Some(prefix) => target.starts_with(prefix),
        None => target.strip_prefix(name).is_some_and(|rest| rest.is_empty() || rest.starts_with("::")),
    }
}

/// `2026-10-08T07:59:17.728Z` (UTC, milliseconds). A clock before 1970 reads as the epoch.
/// A clock far past chrono's year range still converts (the calendar repeats every 400 years), so
/// logging never panics and never silently resets to the epoch for a convertible instant.
pub fn timestamp(t: SystemTime) -> String {
    let since = t.duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    // Pre-1970 is clamped to the epoch by `unwrap_or_default` above. `as_secs` is `u64`; a value
    // past `i64::MAX` seconds saturates rather than falling back to the epoch.
    let secs = i64::try_from(since.as_secs()).unwrap_or(i64::MAX);
    let (y, t) = utc_parts(secs, since.subsec_nanos());
    // `{:04}` rather than `%Y`, which writes a `+` before years past 9999.
    format!("{y:04}-{}", t.format("%m-%dT%H:%M:%S%.3fZ"))
}

/// Unix seconds (+ nanos) → (proleptic Gregorian year, the same wall-clock instant moved into
/// chrono's range by whole 400-year cycles). The calendar repeats every 400 years (146_097 days),
/// so month, day and time of day come from chrono and only the year is carried, which keeps every
/// `i64` exact. Replaces the old hand-rolled `civil_from_days`; chrono is the only calendar logic.
fn utc_parts(secs: i64, nanos: u32) -> (i64, chrono::NaiveDateTime) {
    use chrono::Datelike;
    let (days, sod) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (cycles, rest) = (days.div_euclid(146_097), days.rem_euclid(146_097));
    // `rest + 719_163` (1970-01-01 is CE day 719_163) is always a valid CE day, and `sod` < 86_400.
    let date = i32::try_from(rest + 719_163).ok().and_then(chrono::NaiveDate::from_num_days_from_ce_opt);
    let time = u32::try_from(sod).ok().and_then(|s| chrono::NaiveTime::from_num_seconds_from_midnight_opt(s, nanos.min(999_999_999)));
    match (date, time) {
        (Some(d), Some(t)) => {
            let y = i128::from(d.year()) + i128::from(cycles) * 400;
            (i64::try_from(y).unwrap_or(if y < 0 { i64::MIN } else { i64::MAX }), d.and_time(t))
        }
        _ => (1970, chrono::NaiveDateTime::default()), // unreachable
    }
}

fn numbered(dir: &Path, n: usize) -> PathBuf {
    if n == 0 { dir.join(LOG_FILE) } else { dir.join(format!("photocraft.{n}.log")) }
}

/// Held by the instance that writes [`LOG_FILE`], from its start until it exits. A separate lock
/// file rather than a lock on the log itself: Windows locks are mandatory, and a locked log could
/// not be opened for a bug report while the app runs.
pub const OWNER_LOCK: &str = "photocraft.lock";
/// Held while one start rotates and prunes the logs, so two starts at once can't interleave.
pub const ROTATE_LOCK: &str = "rotate.lock";

/// The log of an instance started while another one was writing [`LOG_FILE`].
fn instance_log(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("photocraft-{pid}.log"))
}

/// The lock file that marks [`instance_log`] as in use.
fn instance_lock(dir: &Path, pid: u32) -> PathBuf {
    dir.join(format!("photocraft-{pid}.lock"))
}

/// The process id in an [`instance_log`] file name.
fn instance_pid(name: &str) -> Option<u32> {
    let digits = name.strip_prefix("photocraft-")?.strip_suffix(".log")?;
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    digits.parse().ok()
}

fn open_lock(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)
}

/// Take `file`'s lock without waiting: `Ok(true)` holds it, `Ok(false)` when another process holds it.
fn try_hold(file: &File) -> std::io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// Open a fresh, empty log file in `dir` (created if missing).
///
/// Normally the previous logs shift up one (`.log` → `.1.log` → … → `.<KEEP>.log`, the oldest
/// dropped). While another instance is still writing [`LOG_FILE`] (it holds [`OWNER_LOCK`]), the
/// chain is left alone and this start writes `photocraft-<pid>.log` instead: rotating would move
/// that instance's live log away and, a few launches later, delete it (#6432). Such side-by-side
/// logs are pruned to the newest [`KEEP`] once their instances have exited. On a file system
/// without locks every start rotates, as before.
pub fn rotate(dir: &Path) -> std::io::Result<LogFile> {
    std::fs::create_dir_all(dir)?;
    // Waits only for another start's few file operations; without locking support, go on.
    let guard = open_lock(&dir.join(ROTATE_LOCK)).ok().filter(|g| g.lock().is_ok());
    let log = rotate_locked(dir);
    drop(guard);
    log
}

fn rotate_locked(dir: &Path) -> std::io::Result<LogFile> {
    prune_instance_logs(dir);
    let owner = open_lock(&dir.join(OWNER_LOCK))?;
    let state = try_hold(&owner);
    if let Ok(false) = state {
        let pid = std::process::id();
        let lock = open_lock(&instance_lock(dir, pid))?;
        let held = try_hold(&lock).unwrap_or(false);
        let path = instance_log(dir, pid);
        let file = File::create(&path)?;
        return Ok(LogFile { path, file, owner: held.then_some(lock) });
    }
    // This start holds the owner lock (or the file system has no locks).
    let held = state.unwrap_or(false);
    for n in (1..=KEEP).rev() {
        match std::fs::rename(numbered(dir, n - 1), numbered(dir, n)) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    let path = numbered(dir, 0);
    let file = File::create(&path)?;
    Ok(LogFile { path, file, owner: held.then_some(owner) })
}

/// Remove the side-by-side logs of instances that have exited, all but the newest [`KEEP`].
/// A log whose lock file is held belongs to a running instance and is never touched. Failures
/// are ignored: pruning only saves disk space.
fn prune_instance_logs(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut finished: Vec<(SystemTime, u32)> = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(instance_pid) else { continue };
        // A missing lock file means nobody holds it; an unknown lock state counts as running.
        let running = match OpenOptions::new().read(true).write(true).open(instance_lock(dir, pid)) {
            Ok(lock) => !try_hold(&lock).unwrap_or(false),
            Err(_) => false,
        };
        if !running {
            let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(SystemTime::UNIX_EPOCH);
            finished.push((modified, pid));
        }
    }
    finished.sort_unstable_by(|a, b| b.cmp(a));
    for &(_, pid) in finished.iter().skip(KEEP) {
        let _ = std::fs::remove_file(instance_log(dir, pid));
        let _ = std::fs::remove_file(instance_lock(dir, pid));
    }
}

/// A log file opened by [`rotate`].
pub struct LogFile {
    pub path: PathBuf,
    pub file: File,
    /// The held lock that marks `path` as in use; `None` where locking isn't supported.
    owner: Option<File>,
}

/// One formatted log line: `<timestamp> <LEVEL> [<thread>] <target>: <message>\n`.
pub fn format_line(ts: &str, level: log::Level, thread: &str, target: &str, message: &str) -> String {
    let message = message.strip_suffix('\n').unwrap_or(message);
    format!("{ts} {level:<5} [{thread}] {target}: {message}\n")
}

/// Where formatted lines go besides standard error: kept in memory until a file is attached,
/// then appended to it until it reaches its size cap.
pub struct Sink {
    file: Option<File>,
    pending: Vec<String>,
    dropped: usize,
    written: u64,
    max_bytes: u64,
    capped: bool,
}

impl Sink {
    pub fn new(max_bytes: u64) -> Sink {
        Sink { file: None, pending: Vec::new(), dropped: 0, written: 0, max_bytes, capped: false }
    }

    /// Append `line` to the file, or keep it until [`Sink::attach`].
    pub fn write(&mut self, line: &str) {
        if self.file.is_none() {
            if self.pending.len() < MAX_PENDING {
                self.pending.push(line.to_owned());
            } else {
                self.dropped = self.dropped.saturating_add(1);
            }
            return;
        }
        self.append(line);
    }

    /// Start writing to `file`, first the lines kept so far.
    pub fn attach(&mut self, file: File) {
        self.file = Some(file);
        for line in std::mem::take(&mut self.pending) {
            self.append(&line);
        }
        if self.dropped > 0 {
            let note = format!("{} earlier log lines were dropped before the log file was opened\n", self.dropped);
            self.dropped = 0;
            self.append(&note);
        }
    }

    fn append(&mut self, line: &str) {
        if self.capped {
            return;
        }
        let Some(file) = self.file.as_mut() else { return };
        let len = u64::try_from(line.len()).unwrap_or(u64::MAX);
        let text = if self.written.saturating_add(len) > self.max_bytes {
            self.capped = true;
            format!("log file reached {} bytes; later records go to standard error only\n", self.max_bytes)
        } else {
            self.written = self.written.saturating_add(len);
            line.to_owned()
        };
        // A full disk or a vanished file must not take the app down: drop the file sink.
        if file.write_all(text.as_bytes()).is_err() {
            self.file = None;
            self.capped = true;
        }
    }
}

/// The installed logger: a [`Filter`], standard error and a [`Sink`].
pub struct AppLogger {
    filter: Filter,
    stderr: bool,
    sink: Mutex<Sink>,
    /// The file [`AppLogger::attach_dir`] opened, so a failure can tell the user what to attach.
    path: Mutex<Option<PathBuf>>,
    /// The lock that marks the attached log file as in use (see [`rotate`]); held until exit.
    owner: Mutex<Option<File>>,
}

impl AppLogger {
    pub fn new(filter: Filter, stderr: bool) -> AppLogger {
        AppLogger { filter, stderr, sink: Mutex::new(Sink::new(MAX_FILE_BYTES)), path: Mutex::new(None), owner: Mutex::new(None) }
    }

    /// Rotate the logs in `dir` and send records to the fresh file; returns its path.
    pub fn attach_dir(&self, dir: &Path) -> Result<PathBuf, String> {
        let LogFile { path, file, owner } = rotate(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        self.sink.lock().unwrap_or_else(PoisonError::into_inner).attach(file);
        *self.path.lock().unwrap_or_else(PoisonError::into_inner) = Some(path.clone());
        *self.owner.lock().unwrap_or_else(PoisonError::into_inner) = owner;
        Ok(path)
    }

    /// The log file records are written to, once [`AppLogger::attach_dir`] opened one: `None` when
    /// the file could not be created (standard error is the only sink, see [`install`]).
    pub fn path(&self) -> Option<PathBuf> {
        self.path.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl log::Log for AppLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        metadata.level() <= self.filter.level_for(metadata.target())
    }

    fn log(&self, record: &log::Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        let thread = std::thread::current();
        let line = format_line(&timestamp(SystemTime::now()), record.level(), thread.name().unwrap_or("?"), record.target(), &record.args().to_string());
        if self.stderr {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        self.sink.lock().unwrap_or_else(PoisonError::into_inner).write(&line);
    }

    fn flush(&self) {}
}

/// Install the logger (filter from `RUST_LOG`, else [`DEFAULT_FILTER`]); `None` when another
/// logger was installed first. Call [`AppLogger::attach_dir`] once the settings directory is known.
pub fn install() -> Option<&'static AppLogger> {
    static LOGGER: OnceLock<AppLogger> = OnceLock::new();
    let spec = std::env::var("RUST_LOG").ok().filter(|s| !s.trim().is_empty());
    let filter = Filter::parse(spec.as_deref().unwrap_or(DEFAULT_FILTER));
    let max = filter.max();
    let logger = LOGGER.get_or_init(|| AppLogger::new(filter, true));
    log::set_logger(logger).ok()?;
    log::set_max_level(max);
    Some(logger)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, UNIX_EPOCH};

    fn temp_dir(tag: &str) -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!("photocraft-logging-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    /// The startup-failure dialog names the log file, so the logger has to remember it (#7643).
    /// A start that couldn't create one keeps `None` and the dialog leaves the line out.
    #[test]
    fn path_names_the_attached_log_file() {
        let _spawns = crate::no_child_spawns();
        let dir = temp_dir("path");
        let logger = AppLogger::new(Filter::parse(DEFAULT_FILTER), false);
        assert_eq!(logger.path(), None, "nothing is written before a file is attached");
        let path = logger.attach_dir(&dir).expect("attach");
        assert_eq!(path, dir.join(LOG_FILE));
        assert_eq!(logger.path(), Some(path));
        drop(logger);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_default_filter_shows_photocraft_info_and_other_crates_warnings() {
        let f = Filter::parse(DEFAULT_FILTER);
        assert_eq!(f.level_for("photocraft"), LevelFilter::Info);
        assert_eq!(f.level_for("photocraft::gpu_startup"), LevelFilter::Info);
        assert_eq!(f.level_for("photocraft_gpu::health"), LevelFilter::Info);
        assert_eq!(f.level_for("wgpu_core::device"), LevelFilter::Warn);
        assert_eq!(f.level_for("naga"), LevelFilter::Warn);
        assert_eq!(f.max(), LevelFilter::Info);
    }

    #[test]
    fn directives_follow_env_logger_and_the_most_specific_one_wins() {
        let f = Filter::parse("info,wgpu_core=error,photocraft_gpu=trace,photocraft_gpu::health=off");
        assert_eq!(f.level_for("eframe"), LevelFilter::Info);
        assert_eq!(f.level_for("wgpu_core::instance"), LevelFilter::Error);
        assert_eq!(f.level_for("photocraft_gpu::canvas"), LevelFilter::Trace);
        assert_eq!(f.level_for("photocraft_gpu::health"), LevelFilter::Off);
        assert_eq!(f.max(), LevelFilter::Trace);
        // A module name is matched at `::` boundaries, not as a bare prefix.
        assert_eq!(f.level_for("wgpu_core_extra"), LevelFilter::Info);
        // A bare target name sets that target to the most verbose level, as env_logger does.
        assert_eq!(Filter::parse("naga").level_for("naga::front"), LevelFilter::Trace);
        assert_eq!(Filter::parse("naga").level_for("eframe"), LevelFilter::Error);
    }

    #[test]
    fn hostile_specs_never_panic_and_fall_back_sensibly() {
        for spec in ["", ",,,", "=", "=debug", "photocraft=", "nonsense=loud", "🦀=info", "*", "a=b=c", "warn,,photocraft=DEBUG"] {
            let f = Filter::parse(spec);
            let _ = f.level_for("photocraft");
            let _ = f.max();
        }
        assert_eq!(Filter::parse("warn,,photocraft=DEBUG").level_for("photocraft"), LevelFilter::Debug);
        assert_eq!(Filter::parse("nonsense=loud").level_for("nonsense"), LevelFilter::Error);
    }

    #[test]
    fn timestamps_are_utc_with_milliseconds() {
        assert_eq!(timestamp(UNIX_EPOCH), "1970-01-01T00:00:00.000Z");
        // 2026-10-08T07:59:17.728Z
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_millis(1_791_446_357_728)), "2026-10-08T07:59:17.728Z");
        // Leap day.
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(1_709_164_800)), "2024-02-29T00:00:00.000Z");
        // Before the epoch (a clock set wrong) clamps instead of panicking.
        assert_eq!(timestamp(UNIX_EPOCH - Duration::from_secs(5)), "1970-01-01T00:00:00.000Z");
        // Past chrono's year range still converts via the 400-year cycle (the old code did too,
        // through `civil_from_days`): never a silent reset to the epoch. Platform-safe: Windows'
        // SystemTime can't hold this instant, so it is only checked where it exists.
        // `u64::MAX / 2` seconds == `i64::MAX` seconds → year 292277026596 (old output).
        if let Some(far) = UNIX_EPOCH.checked_add(Duration::from_secs(u64::MAX / 2)) {
            assert_eq!(timestamp(far), "292277026596-12-04T15:30:07.000Z");
        }
        // Years past 9999 keep the plain zero-padded year (no `+`).
        assert_eq!(timestamp(UNIX_EPOCH + Duration::from_secs(253_402_300_800)), "10000-01-01T00:00:00.000Z");
        // 10_000_000_000_000 s is far beyond chrono's range; old output is year 318857.
        // Windows' SystemTime holds only up to about year 30828, so this is checked where it fits.
        if let Some(far) = UNIX_EPOCH.checked_add(Duration::from_secs(10_000_000_000_000)) {
            assert_eq!(timestamp(far), "318857-05-20T17:46:40.000Z");
        }
    }

    #[test]
    fn rotation_keeps_the_previous_logs_and_starts_an_empty_file() {
        let _spawns = crate::no_child_spawns();
        let dir = temp_dir("rotate");
        for run in 1..=4 {
            let LogFile { path, file: _file, owner: _owner } = rotate(&dir).expect("rotate");
            assert_eq!(path, dir.join(LOG_FILE));
            assert_eq!(std::fs::read_to_string(&path).expect("read"), "");
            std::fs::write(&path, format!("run {run}")).expect("write");
        }
        assert_eq!(std::fs::read_to_string(dir.join(LOG_FILE)).expect("current"), "run 4");
        assert_eq!(std::fs::read_to_string(dir.join("photocraft.1.log")).expect("1"), "run 3");
        assert_eq!(std::fs::read_to_string(dir.join("photocraft.2.log")).expect("2"), "run 2");
        assert!(!dir.join("photocraft.3.log").exists(), "only {KEEP} old logs are kept");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #6432: a second launch moved the running instance's live log to `.1.log`, a third to
    /// `.2.log`, and a fourth deleted it.
    #[test]
    fn a_second_start_leaves_the_running_instance_log_alone() {
        let _spawns = crate::no_child_spawns();
        let dir = temp_dir("second");
        let mut first = rotate(&dir).expect("first start");
        assert_eq!(first.path, dir.join(LOG_FILE));
        first.file.write_all(b"first, before\n").expect("write");
        for _ in 0..KEEP + 1 {
            let second = rotate(&dir).expect("second start");
            assert_eq!(second.path, instance_log(&dir, std::process::id()));
            assert_eq!(read(&second.path), "");
        }
        first.file.write_all(b"first, after\n").expect("write");
        assert_eq!(read(&dir.join(LOG_FILE)), "first, before\nfirst, after\n");
        assert!(!dir.join("photocraft.1.log").exists(), "the running instance's log was rotated");
        // Once the first instance is gone, the next start rotates as before.
        drop(first);
        let third = rotate(&dir).expect("third start");
        assert_eq!(third.path, dir.join(LOG_FILE));
        assert_eq!(read(&dir.join("photocraft.1.log")), "first, before\nfirst, after\n");
        drop(third);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn instance_logs_are_pruned_only_when_no_one_holds_them() {
        let _spawns = crate::no_child_spawns();
        let dir = temp_dir("prune");
        let first = rotate(&dir).expect("first start");
        // Earlier side-by-side runs that have exited, oldest first; the oldest has no lock file
        // (as if its lock file had been removed), the others an unlocked one.
        let now = SystemTime::now();
        for (age, pid) in [(400u64, 1u32), (300, 2), (200, 3), (100, 4)] {
            let path = instance_log(&dir, pid);
            std::fs::write(&path, format!("run {pid}")).expect("write");
            File::options().write(true).open(&path).expect("open").set_modified(now - Duration::from_secs(age)).expect("mtime");
            if pid != 1 {
                std::fs::write(instance_lock(&dir, pid), "").expect("lock file");
            }
        }
        // One older still running: its log is never removed, however old.
        let live = instance_log(&dir, 5);
        std::fs::write(&live, "live").expect("write");
        File::options().write(true).open(&live).expect("open").set_modified(now - Duration::from_secs(900)).expect("mtime");
        let held = File::create(instance_lock(&dir, 5)).expect("lock file");
        held.try_lock().expect("hold");

        let second = rotate(&dir).expect("second start");
        assert_eq!(second.path, instance_log(&dir, std::process::id()));
        // The newest KEEP finished ones stay, the rest go with their lock files.
        assert!(instance_log(&dir, 4).exists() && instance_log(&dir, 3).exists());
        assert!(!instance_log(&dir, 2).exists() && !instance_log(&dir, 1).exists());
        assert!(!instance_lock(&dir, 2).exists());
        assert_eq!(read(&live), "live");
        drop((first, second, held));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rotation_into_an_unusable_directory_is_an_error_not_a_panic() {
        let dir = temp_dir("blocked");
        std::fs::create_dir_all(dir.parent().expect("parent")).expect("tmp");
        std::fs::write(&dir, "a file where the directory should be").expect("block");
        assert!(rotate(&dir).is_err());
        let _ = std::fs::remove_file(&dir);
    }
    fn read(path: &Path) -> String {
        std::fs::read_to_string(path).expect("read log")
    }

    #[test]
    fn a_line_carries_time_level_thread_target_and_message() {
        assert_eq!(
            format_line("2026-10-08T07:59:17.728Z", log::Level::Warn, "main", "photocraft::gpu_startup", "GPU startup marker: denied"),
            "2026-10-08T07:59:17.728Z WARN  [main] photocraft::gpu_startup: GPU startup marker: denied\n"
        );
        // A multi-line message (a panic with a backtrace) keeps its lines; the record still ends in one newline.
        assert!(format_line("t", log::Level::Error, "w", "x", "a\nb\n").ends_with("x: a\nb\n"));
    }

    #[test]
    fn lines_logged_before_the_file_exists_are_written_when_it_is_attached() {
        let dir = temp_dir("pending");
        let LogFile { path, file, .. } = rotate(&dir).expect("rotate");
        let mut sink = Sink::new(MAX_FILE_BYTES);
        sink.write("early 1\n");
        sink.write("early 2\n");
        sink.attach(file);
        sink.write("late\n");
        assert_eq!(read(&path), "early 1\nearly 2\nlate\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_pending_buffer_is_bounded_and_says_how_much_it_dropped() {
        let dir = temp_dir("bounded");
        let LogFile { path, file, .. } = rotate(&dir).expect("rotate");
        let mut sink = Sink::new(MAX_FILE_BYTES);
        for i in 0..MAX_PENDING + 5 {
            sink.write(&format!("line {i}\n"));
        }
        sink.attach(file);
        let text = read(&path);
        assert_eq!(text.lines().filter(|l| l.starts_with("line ")).count(), MAX_PENDING);
        assert!(text.contains("5 earlier log lines were dropped"), "{text}");
        assert!(text.contains("line 0\n") && !text.contains(&format!("line {MAX_PENDING}\n")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_stops_at_its_size_cap_with_one_note() {
        let dir = temp_dir("cap");
        let LogFile { path, file, .. } = rotate(&dir).expect("rotate");
        let mut sink = Sink::new(100);
        sink.attach(file);
        for i in 0..50 {
            sink.write(&format!("line {i:02} with some padding\n"));
        }
        let text = read(&path);
        assert!(text.lines().filter(|l| l.starts_with("line ")).count() < 5, "{text}");
        assert_eq!(text.matches("log file reached").count(), 1, "{text}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_logger_filters_records_and_writes_them_to_the_attached_file() {
        use log::Log;
        let dir = temp_dir("logger");
        let logger = AppLogger::new(Filter::parse(DEFAULT_FILTER), false);
        let record = |level, target: &'static str, msg: &'static str| {
            logger.log(&log::Record::builder().level(level).target(target).args(format_args!("{msg}")).build());
        };
        record(log::Level::Info, "photocraft::app_dirs", "settings directory");
        record(log::Level::Info, "wgpu_core::device", "too chatty");
        let path = logger.attach_dir(&dir).expect("attach");
        assert_eq!(path, dir.join(LOG_FILE));
        record(log::Level::Warn, "wgpu_hal::vulkan", "a real warning");
        record(log::Level::Debug, "photocraft", "below info");
        let text = read(&path);
        assert!(text.contains(" INFO  [") && text.contains("photocraft::app_dirs: settings directory\n"), "{text}");
        assert!(text.contains("wgpu_hal::vulkan: a real warning\n"), "{text}");
        assert!(!text.contains("too chatty") && !text.contains("below info"), "{text}");
        assert!(logger.enabled(&log::Metadata::builder().level(log::Level::Info).target("photocraft_gpu").build()));
        assert!(!logger.enabled(&log::Metadata::builder().level(log::Level::Info).target("naga").build()));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn attaching_to_an_unusable_directory_reports_an_error_and_keeps_logging() {
        use log::Log;
        let dir = temp_dir("unusable");
        std::fs::write(&dir, "not a directory").expect("block");
        let logger = AppLogger::new(Filter::parse("info"), false);
        assert!(logger.attach_dir(&dir).is_err());
        logger.log(&log::Record::builder().level(log::Level::Error).target("x").args(format_args!("still fine")).build());
        let _ = std::fs::remove_file(&dir);
    }
}
