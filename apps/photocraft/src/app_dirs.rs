//! Where the desktop app keeps its per-user data: preferences, brush presets, crash-recovery
//! autosaves and the GPU startup marker all live under the one directory [`config_dir`] returns.
//!
//! In order:
//! 1. `PHOTOCRAFT_CONFIG_DIR`, when set (tests, agents, custom setups).
//! 2. **Portable mode** (#228): a marker file ([`PORTABLE_MARKERS`]) beside the executable puts
//!    everything in `<exe dir>/PhotoCraftData`, so a portable copy on a USB stick leaves nothing in
//!    `%APPDATA%`. The Windows portable zip ships with `portable.txt`; the MSI doesn't. If that
//!    folder can't be written (read-only medium, Program Files), the app warns and falls back.
//! 3. The platform convention: macOS `~/Library/Application Support/Photocraft`, Windows
//!    `%APPDATA%\Photocraft`, Linux `$XDG_CONFIG_HOME/photocraft` or `~/.config/photocraft`.
//!
//! One exception: the platform convention keeps crash-recovery autosaves out of what is synced
//! with the settings: the Windows roaming profile and the Linux config folder ([`recovery_dir`]).

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Files beside the executable that switch on portable mode (either one; contents are ignored).
pub const PORTABLE_MARKERS: [&str; 2] = ["portable.txt", "PhotoCraft.portable"];
/// The data folder created beside the executable in portable mode.
pub const PORTABLE_DATA_DIR: &str = "PhotoCraftData";

/// How [`DataDir::dir`] was chosen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// `PHOTOCRAFT_CONFIG_DIR`.
    Override,
    /// `<exe dir>/PhotoCraftData`.
    Portable,
    /// The platform's per-user config directory.
    Platform,
}

/// The resolved data directory.
#[derive(Clone, Debug)]
pub struct DataDir {
    /// `None` only when no platform directory is known either (no `HOME`/`APPDATA`).
    pub dir: Option<PathBuf>,
    pub mode: Mode,
    /// Set when a portable marker was found but its data folder isn't writable.
    pub warning: Option<String>,
}

/// The data directory for this process, resolved once (the portable check probes the disk).
pub fn current() -> &'static DataDir {
    static DIR: OnceLock<DataDir> = OnceLock::new();
    DIR.get_or_init(|| {
        let exe = std::env::current_exe().ok();
        let d = resolve(|k| std::env::var_os(k), exe.as_deref().and_then(Path::parent));
        if let Some(w) = &d.warning {
            log::warn!("{w}");
            eprintln!("photocraft: {w}");
        }
        log::info!("settings directory ({:?}): {:?}", d.mode, d.dir);
        d
    })
}

/// The per-user settings directory (see the module docs).
pub fn config_dir() -> Option<PathBuf> {
    current().dir.clone()
}

/// The crash-recovery folder's name.
const RECOVERY: &str = "Recovery";

/// Where crash-recovery autosaves live, with any left in the settings directory by an earlier
/// version moved there first ([`adopt_legacy_recovery`]). Call it once, at startup.
pub fn recovery_dir() -> Option<PathBuf> {
    let data = current();
    let dir = resolve_recovery(data, |k| std::env::var_os(k))?;
    Some(match data.dir.as_ref().map(|d| d.join(RECOVERY)) {
        Some(legacy) => adopt_legacy_recovery(&legacy, &dir),
        None => dir,
    })
}

/// The recovery folder for `data`. Autosaves hold whole documents and are rewritten every few
/// minutes, so with the platform convention they stay out of what is synced with the settings:
/// - Windows (#5648): the machine-local `%LOCALAPPDATA%\Photocraft\Recovery`; the roaming
///   `%APPDATA%` may be synced at sign-in and sign-out or capped by policy.
/// - Linux and other XDG systems (#6612): `$XDG_STATE_HOME/photocraft/Recovery`, by default
///   `~/.local/state/photocraft/Recovery`, where the XDG Base Directory spec puts state that
///   should outlive a restart but isn't configuration; dotfile managers and config sync often copy
///   `~/.config` whole. As the spec asks, a relative `XDG_STATE_HOME` is ignored.
/// - macOS: `~/Library/Application Support/Photocraft/Recovery` as before. Apple's guidelines put
///   app-created data that can't be regenerated there; `~/Library/Caches` may be purged by the
///   system.
///
/// They are never in a temp or cache folder, which cleanup tools may empty before a crashed
/// document is recovered. The override and portable folders keep everything in one place.
pub fn resolve_recovery(data: &DataDir, env: impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    if data.mode == Mode::Platform {
        if cfg!(windows) {
            if let Some(local) = env("LOCALAPPDATA").filter(|l| !l.is_empty()) {
                return Some(PathBuf::from(local).join("Photocraft").join(RECOVERY));
            }
        } else if !cfg!(target_os = "macos") {
            let state = env("XDG_STATE_HOME")
                .map(PathBuf::from)
                .filter(|s| s.is_absolute())
                .or_else(|| env("HOME").filter(|h| !h.is_empty()).map(|h| PathBuf::from(h).join(".local").join("state")));
            if let Some(state) = state {
                return Some(state.join("photocraft").join(RECOVERY));
            }
        }
    }
    data.dir.as_ref().map(|d| d.join(RECOVERY))
}

/// The lock file, beside the recovery folder, that keeps two PhotoCraft processes starting at
/// once (`--new-instance`, control launches) from moving the same autosaves.
const MIGRATION_LOCK: &str = "recovery-migration.lock";

/// How long a launch waits for another one's move before going ahead without moving anything.
const MIGRATION_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Suffix of a copy staged for publishing ([`stage_copy`]): disposable, the original still exists.
const STAGED: &str = ".moving";

/// Suffix of an original set aside while it is copied to another volume ([`move_across`]): it
/// may be the only copy, so it is put back, never thrown away, if the move doesn't finish.
const HELD: &str = ".held";

/// Move autosaves from `legacy` (the recovery folder of versions before #5648) into `dir`, so a
/// crash just before an update is still offered for recovery. Returns the folder this process
/// uses for its recovery store. That is `legacy` only when `dir` doesn't exist and no process can
/// be moving autosaves (the move failed, or its lock can't be made), so nothing deletes what is
/// written there. A launch that finds another one moving uses `dir`, which the move never
/// replaces or removes. See [`adopt_locked`] for how they move.
pub fn adopt_legacy_recovery(legacy: &Path, dir: &Path) -> PathBuf {
    adopt_legacy_recovery_waiting(legacy, dir, MIGRATION_WAIT)
}

/// [`adopt_legacy_recovery`], waiting up to `wait` for another process's move.
fn adopt_legacy_recovery_waiting(legacy: &Path, dir: &Path, wait: std::time::Duration) -> PathBuf {
    let held_aside = || legacy.parent().is_some_and(|p| !side_entries(p, legacy, HELD).is_empty());
    if legacy == dir || (!legacy.is_dir() && !held_aside()) {
        return dir.to_path_buf();
    }
    // An empty old folder has nothing to move (`remove_dir` only removes an empty one).
    if !held_aside() && std::fs::remove_dir(legacy).is_ok() {
        return dir.to_path_buf();
    }
    let unmoved = || if dir.exists() || !legacy.is_dir() { dir.to_path_buf() } else { legacy.to_path_buf() };
    let Some(parent) = dir.parent() else { return unmoved() };
    let lock =
        std::fs::create_dir_all(parent).and_then(|()| std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(parent.join(MIGRATION_LOCK)));
    let Ok(lock) = lock else {
        // No lock file means no other process can be moving: the old folder is safe to use.
        log::warn!("couldn't open the recovery move lock in {}; autosaves stay where they are", parent.display());
        return unmoved();
    };
    let deadline = std::time::Instant::now() + wait;
    loop {
        match lock.try_lock() {
            Ok(()) => break,
            Err(std::fs::TryLockError::WouldBlock) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(50)),
            Err(std::fs::TryLockError::WouldBlock) => {
                // Another process is moving and may yet remove the old folder: never write there.
                log::info!("another PhotoCraft is moving crash-recovery autosaves; using {}", dir.display());
                return dir.to_path_buf();
            }
            Err(std::fs::TryLockError::Error(e)) => {
                // Locking fails for every process here alike, so none can be moving.
                log::warn!("couldn't lock {}: {e}; autosaves stay where they are", parent.join(MIGRATION_LOCK).display());
                return unmoved();
            }
        }
    }
    // The lock file stays (removing it would race a process about to lock it); the OS releases
    // the lock when `lock` is dropped or the process ends.
    let used = adopt_locked(legacy, dir);
    if used == legacy {
        // This process writes its autosaves to the old folder: it keeps the lock for as long as it
        // runs, so no other launch moves (and removes) the folder under it.
        KEPT_LOCKS.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(lock);
    }
    used
}

/// Migration locks of the old folders this process uses (see [`adopt_legacy_recovery`]), kept
/// until it ends.
static KEPT_LOCKS: std::sync::Mutex<Vec<std::fs::File>> = std::sync::Mutex::new(Vec::new());

/// The move itself, under the migration lock. First, what a process that ended mid-move left is
/// tidied: originals it set aside are put back, copies it staged are removed. An empty `legacy`
/// is then just removed. Otherwise the whole folder moves if `dir` doesn't exist yet; when both
/// exist, each autosave (`<key>.pcraft` bundle and `<key>.json` sidecar, see
/// `photocraft_format::autosave`) moves as a whole, bundle first, unless `dir` already has either
/// part, and one that stays is left in `legacy` untouched. Every move is a [`move_path`].
fn adopt_locked(legacy: &Path, dir: &Path) -> PathBuf {
    if let Some(parent) = legacy.parent() {
        for held in side_entries(parent, legacy, HELD) {
            put_back(&held, legacy);
        }
    }
    // Autosave parts held aside inside the old folder by a per-autosave move.
    let held_parts: Vec<(PathBuf, String)> = std::fs::read_dir(legacy)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().and_then(|n| side_of(n, HELD)).map(|orig| (e.path(), orig.to_owned())))
        .collect();
    for (held, original) in held_parts {
        put_back(&held, &legacy.join(original));
    }
    for folder in [dir.parent(), Some(dir)].into_iter().flatten() {
        remove_stale_staging(folder);
    }
    // Another process may have finished the move while this one waited.
    if !legacy.is_dir() {
        return dir.to_path_buf();
    }
    if std::fs::remove_dir(legacy).is_ok() {
        return dir.to_path_buf();
    }
    if !dir.exists() {
        // The common case: the whole folder moves at once.
        return match move_path(legacy, dir) {
            Ok(()) => {
                log::info!("moved crash-recovery autosaves from {} to {}", legacy.display(), dir.display());
                dir.to_path_buf()
            }
            Err(e) => {
                log::warn!("couldn't move crash-recovery autosaves from {} to {} ({e}); using the old folder", legacy.display(), dir.display());
                if legacy.is_dir() { legacy.to_path_buf() } else { dir.to_path_buf() }
            }
        };
    }
    move_entries(legacy, dir);
    // Only an emptied folder goes; anything that couldn't move stays for a later launch.
    let _ = std::fs::remove_dir(legacy);
    dir.to_path_buf()
}

/// Move each autosave of `from` into `to` (both folders): bundle first, then sidecar, skipped if
/// `to` has either part. Temp files of an interrupted write (`photocraft_format::atomic`) go
/// along too, for the recovery store's startup sweep there.
fn move_entries(from: &Path, to: &Path) {
    let Ok(entries) = std::fs::read_dir(from) else { return };
    let names: Vec<String> = entries.flatten().filter_map(|e| e.file_name().to_str().map(str::to_owned)).collect();
    let keys: std::collections::BTreeSet<&str> = names.iter().filter_map(|n| n.strip_suffix(".pcraft").or_else(|| n.strip_suffix(".json"))).collect();
    for key in keys {
        let parts = [format!("{key}.pcraft"), format!("{key}.json")];
        if parts.iter().any(|p| std::fs::symlink_metadata(to.join(p)).is_ok()) {
            continue;
        }
        // A bundle that moves without its sidecar is still listed (`list_recovery` reads a
        // committed bundle's manifest); a sidecar must never move without its bundle.
        for part in &parts {
            let src = from.join(part);
            if std::fs::symlink_metadata(&src).is_ok()
                && let Err(e) = move_path(&src, &to.join(part))
            {
                log::warn!("couldn't move {} to {}: {e}", src.display(), to.display());
                break;
            }
        }
    }
    for name in names.iter().filter(|n| n.ends_with(".pcsave.tmp")) {
        let _ = move_path(&from.join(name), &to.join(name));
    }
}

/// How deep [`copy_tree`] follows folders: an autosave bundle is a few levels deep.
const MAX_COPY_DEPTH: u32 = 16;

fn already_exists(path: &Path) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::AlreadyExists, format!("{} already exists", path.display()))
}

/// A path beside `of`, named `.<name>.<pid>-<n><suffix>`, that no other call uses.
fn side_path(of: &Path, suffix: &str) -> std::io::Result<PathBuf> {
    static NEXT: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
    let parent = of.parent().ok_or_else(|| std::io::Error::other(format!("{} has no parent folder", of.display())))?;
    let name = of.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(parent.join(format!(".{name}.{}-{}{suffix}", std::process::id(), NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed))))
}

/// The name `side` was made from, if it is a [`side_path`] name with `suffix`.
fn side_of<'a>(side: &'a str, suffix: &str) -> Option<&'a str> {
    let rest = side.strip_prefix('.')?.strip_suffix(suffix)?;
    let (name, ids) = rest.rsplit_once('.')?;
    let (pid, n) = ids.split_once('-')?;
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    (!name.is_empty() && digits(pid) && digits(n)).then_some(name)
}

/// The side paths with `suffix` in `folder` made from `of`.
fn side_entries(folder: &Path, of: &Path, suffix: &str) -> Vec<PathBuf> {
    let Some(name) = of.file_name().and_then(|n| n.to_str()) else { return Vec::new() };
    std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.file_name().to_str().and_then(|n| side_of(n, suffix)) == Some(name))
        .map(|e| e.path())
        .collect()
}

/// Move the file or folder `from` to `to` without ever replacing anything at `to`. On one volume
/// a folder is renamed (Windows refuses an existing target; elsewhere only an empty folder could
/// be replaced) and a file is hard-linked, which fails on an existing target, and then unlinked
/// at `from`. Otherwise (another volume, e.g. a roaming profile redirected to a network share
/// while `%LOCALAPPDATA%` stays local) it is copied ([`move_across`]).
fn move_path(from: &Path, to: &Path) -> std::io::Result<()> {
    if std::fs::symlink_metadata(to).is_ok() {
        return Err(already_exists(to));
    }
    let meta = std::fs::symlink_metadata(from)?;
    let moved = if meta.is_dir() { std::fs::rename(from, to) } else { std::fs::hard_link(from, to).and_then(|()| std::fs::remove_file(from)) };
    match moved {
        Ok(()) => Ok(()),
        Err(e) if matches!(e.kind(), std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::DirectoryNotEmpty) => Err(e),
        Err(_) => move_across(from, to),
    }
}

/// Move `from` to `to` by copying. `from` is first renamed aside on its own volume
/// (`.<name>.<pid>-<n>.held`, [`hold`]), so a process still writing to the old path (another
/// launch that couldn't take the lock, an older PhotoCraft) starts a new `from` there instead of
/// adding to what is copied, and nothing it writes is removed. The held original is copied to a
/// staged path ([`stage_copy`]), published without replacing anything ([`publish`]), and only
/// then removed. If the copy or publish fails, the held original is put back ([`put_back`]).
fn move_across(from: &Path, to: &Path) -> std::io::Result<()> {
    let held = hold(from)?;
    match stage_copy(&held, to).and_then(|staged| publish(&staged, to)) {
        Ok(()) => {
            let removed = if std::fs::symlink_metadata(&held)?.is_dir() { std::fs::remove_dir_all(&held) } else { std::fs::remove_file(&held) };
            if let Err(e) = removed {
                log::warn!("copied {} to {} but couldn't remove {}: {e}", from.display(), to.display(), held.display());
            }
            Ok(())
        }
        Err(e) => {
            put_back(&held, from);
            Err(e)
        }
    }
}

/// Rename `from` aside on its own volume (see [`move_across`]); returns where it is now.
fn hold(from: &Path) -> std::io::Result<PathBuf> {
    let held = side_path(from, HELD)?;
    std::fs::rename(from, &held)?;
    Ok(held)
}

/// Return the held original `held` to `original`. If `original` has been made again meanwhile (a
/// writer that kept using the old path), the held folder's entries move into it without replacing
/// anything ([`move_entries`]); whatever can't move stays held, logged, and is tried again under
/// the next move's lock ([`adopt_locked`] puts back what a process that ended mid-move held).
/// Nothing held is ever removed with data in it.
fn put_back(held: &Path, original: &Path) {
    if std::fs::symlink_metadata(original).is_err() && std::fs::rename(held, original).is_ok() {
        return;
    }
    if held.is_dir() && original.is_dir() {
        move_entries(held, original);
        if std::fs::remove_dir(held).is_ok() {
            return;
        }
    }
    log::warn!("{} is kept aside: {} exists", held.display(), original.display());
}

/// Copy `from` to a staging path beside `to` that this call creates exclusively, so nothing
/// another process made is ever written to or cleaned up. On failure the staged copy is removed.
fn stage_copy(from: &Path, to: &Path) -> std::io::Result<PathBuf> {
    let staged = side_path(to, STAGED)?;
    let is_dir = std::fs::symlink_metadata(from)?.is_dir();
    let copied = if is_dir { copy_tree(from, &staged, 0) } else { copy_file(from, &staged) };
    if let Err(e) = copied {
        // `copy_tree` and `copy_file` create `staged` exclusively, and everything under a
        // fresh folder is new: `AlreadyExists` means `staged` was someone else's to keep.
        if e.kind() != std::io::ErrorKind::AlreadyExists {
            let _ = if is_dir { std::fs::remove_dir_all(&staged) } else { std::fs::remove_file(&staged) };
        }
        return Err(e);
    }
    Ok(staged)
}

/// Remove copies a process left staged in `folder` when it ended mid-move: disposable, since the
/// original is still in place or held aside. Called only under the migration lock, which every
/// staging happens under. Links are never followed.
fn remove_stale_staging(folder: &Path) {
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for entry in entries.flatten() {
        if entry.file_name().to_str().is_none_or(|n| side_of(n, STAGED).is_none()) {
            continue;
        }
        let removed = match entry.file_type() {
            Ok(t) if t.is_dir() => std::fs::remove_dir_all(entry.path()),
            Ok(t) if t.is_file() => std::fs::remove_file(entry.path()),
            _ => continue,
        };
        if let Err(e) = removed {
            log::warn!("couldn't remove the stale staged copy {}: {e}", entry.path().display());
        }
    }
}

/// Give the staged copy its final name `to` without replacing anything there (see [`move_path`]
/// for the folder and file cases). On failure the staged copy is removed and `to` is untouched.
fn publish(staged: &Path, to: &Path) -> std::io::Result<()> {
    let is_dir = std::fs::symlink_metadata(staged)?.is_dir();
    let published = if std::fs::symlink_metadata(to).is_ok() {
        Err(already_exists(to))
    } else if is_dir {
        std::fs::rename(staged, to)
    } else {
        std::fs::hard_link(staged, to).and_then(|()| std::fs::remove_file(staged))
    };
    if published.is_err() {
        let _ = if is_dir { std::fs::remove_dir_all(staged) } else { std::fs::remove_file(staged) };
    }
    published
}

/// Copy the folder `from` to the new folder `to` (created here; an existing `to` is an error):
/// plain files and folders only (a link or other special file is refused), at most
/// [`MAX_COPY_DEPTH`] levels.
fn copy_tree(from: &Path, to: &Path, depth: u32) -> std::io::Result<()> {
    if depth > MAX_COPY_DEPTH {
        return Err(std::io::Error::other(format!("{} is nested too deep", from.display())));
    }
    std::fs::create_dir(to)?;
    for entry in std::fs::read_dir(from)? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            copy_tree(&entry.path(), &target, depth.saturating_add(1))?;
        } else {
            copy_file(&entry.path(), &target)?;
        }
    }
    Ok(())
}

/// Copy one plain file (a link or other special file is refused) to the new file `to` (an
/// existing `to` is an error), flushed to disk.
fn copy_file(from: &Path, to: &Path) -> std::io::Result<()> {
    if !std::fs::symlink_metadata(from)?.file_type().is_file() {
        return Err(std::io::Error::other(format!("{} is not a plain file", from.display())));
    }
    let mut src = std::fs::File::open(from)?;
    let mut dst = std::fs::OpenOptions::new().write(true).create_new(true).open(to)?;
    std::io::copy(&mut src, &mut dst)?;
    dst.sync_all()
}

/// Resolve the data directory from the environment (`env`) and the executable's folder.
pub fn resolve(env: impl Fn(&str) -> Option<OsString>, exe_dir: Option<&Path>) -> DataDir {
    if let Some(d) = env("PHOTOCRAFT_CONFIG_DIR").filter(|d| !d.is_empty()) {
        return DataDir { dir: Some(PathBuf::from(d)), mode: Mode::Override, warning: None };
    }
    let platform = platform_dir(&env);
    let Some((exe_dir, marker)) = exe_dir.and_then(|d| portable_marker(d).map(|m| (d, m))) else {
        return DataDir { dir: platform, mode: Mode::Platform, warning: None };
    };
    let data = exe_dir.join(PORTABLE_DATA_DIR);
    match ensure_writable(&data) {
        Ok(()) => DataDir { dir: Some(data), mode: Mode::Portable, warning: None },
        Err(e) => {
            let fallback = platform.as_ref().map_or_else(|| "nowhere (no user folder found)".to_string(), |p| p.display().to_string());
            let warning =
                format!("portable mode ({} found) but {} isn't writable ({e}); settings are kept in {fallback} instead", marker.display(), data.display());
            DataDir { dir: platform, mode: Mode::Platform, warning: Some(warning) }
        }
    }
}

/// The platform convention, from `env` (so tests can fake `HOME`/`APPDATA`/`XDG_CONFIG_HOME`).
pub fn platform_dir(env: &impl Fn(&str) -> Option<OsString>) -> Option<PathBuf> {
    let home = env("HOME").filter(|h| !h.is_empty()).map(PathBuf::from);
    if cfg!(target_os = "macos") {
        return home.map(|h| h.join("Library/Application Support/Photocraft"));
    }
    if cfg!(windows) {
        return env("APPDATA").filter(|a| !a.is_empty()).map(|a| PathBuf::from(a).join("Photocraft"));
    }
    env("XDG_CONFIG_HOME").filter(|x| !x.is_empty()).map(PathBuf::from).or_else(|| home.map(|h| h.join(".config"))).map(|c| c.join("photocraft"))
}

/// The first portable marker file in `exe_dir`, if any.
pub fn portable_marker(exe_dir: &Path) -> Option<PathBuf> {
    PORTABLE_MARKERS.iter().map(|m| exe_dir.join(m)).find(|p| p.is_file())
}

/// Create `dir` and prove a file can be written in it (a folder can exist yet be read-only).
fn ensure_writable(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let probe = dir.join(format!(".write-test-{}", std::process::id()));
    std::fs::write(&probe, b"")?;
    // A leftover probe is harmless; don't fail portable mode over it.
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

#[cfg(test)]
mod tests;
