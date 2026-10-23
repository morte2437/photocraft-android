//! Crash-safe file replacement: the one way PhotoCraft writes a document to disk.
//!
//! [`atomic_write`] never truncates the destination. It writes a temporary file in the **same
//! directory**, flushes it to stable storage (`sync_all`), renames it over the destination and,
//! flushes the directory so the rename itself survives a power loss. Failures before replacement
//! keep the destination's previous bytes. On Windows a failed publication flush reports that the
//! new file is in place but its durability could not be confirmed (#7858).
//!
//! - **Windows:** antivirus scanners and the search indexer briefly hold files open, which makes
//!   the replacing rename fail with `PermissionDenied`. The rename is retried with exponential
//!   backoff before giving up. (`std::fs::rename` uses `MoveFileExW(MOVEFILE_REPLACE_EXISTING)`,
//!   i.e. replace semantics.)
//! - **Cross-device (`EXDEV`):** cannot happen while the temporary file sits beside the target;
//!   if it does anyway, the save fails with a clear error. It never falls back to writing the
//!   destination in place.
//! - **Unwritable folder:** the save fails with a clear error instead of writing in place.
//! - **Read-only file:** the save is refused before anything is written (#6600). A rename only
//!   needs the folder to be writable, so without this check Linux and macOS would silently replace
//!   a `chmod 444` file while Windows refuses; now every platform refuses, as Photoshop does, and
//!   the user saves a copy with Save As.
//! - **Network shares (macOS):** `sync_all` asks for a full flush to the disk's platters
//!   (`F_FULLFSYNC`), which SMB shares and some virtual file systems refuse while a plain `fsync`
//!   works. [`sync_file`] falls back to the plain one there (#1336).
//!
//! On the web there is no file system: writes go through the platform services (downloads), so
//! this module is never reached there; on `wasm32` every call returns the `std` "unsupported"
//! error.
//!
//! Replacing a file also carries its metadata across — permission bits, extended attributes,
//! the "downloaded from the internet" mark and the access-control list — so that saving a
//! document does not quietly undo a security decision someone made about the file. That part is
//! [`meta`], and it is best effort: it never fails a save and never touches the bytes.
//!
//! [`atomic_write_with`] takes an [`AtomicFs`] so tests can inject failures at each step.

pub mod meta;

use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// The file-system steps [`atomic_write_with`] performs. [`RealFs`] is the real thing; tests
/// wrap it to inject failures.
pub trait AtomicFs {
    /// Whether the existing file at `path` is marked read-only (a missing file is not). A
    /// read-only destination is never replaced (#6600). By default nothing is read-only.
    fn is_readonly(&self, path: &Path) -> bool {
        let _ = path;
        false
    }
    /// Create `tmp` (it must not exist), write all of `bytes` and flush it ([`sync_file`]).
    fn write_new(&self, tmp: &Path, bytes: &[u8]) -> io::Result<()>;
    /// [`write_new`](Self::write_new), reporting the fraction of `bytes` written so far to
    /// `progress` (a save's progress bar, #3389). By default it reports nothing.
    fn write_new_progress(&self, tmp: &Path, bytes: &[u8], progress: &dyn Fn(f32)) -> io::Result<()> {
        let _ = progress;
        self.write_new(tmp, bytes)
    }
    /// Rename `from` over `to`, replacing it.
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Remove a (temporary) file.
    fn remove(&self, path: &Path) -> io::Result<()>;
    /// Flush a directory entry change on Unix and Windows; a no-op elsewhere.
    fn sync_dir(&self, dir: &Path) -> io::Result<()>;
    /// Copy the metadata a replacement should keep — mode, extended attributes, ACL — from the
    /// file about to be replaced onto the temporary file. Best effort; see [`meta`].
    fn copy_metadata(&self, from: &Path, to: &Path) -> io::Result<()>;
    /// Replace the **existing** file `to` with `from`. Everywhere but Windows this is
    /// [`rename`](Self::rename); on Windows it keeps the destination's DACL, named streams and
    /// creation time ([`meta::replace_existing`]). The default is `rename`, which is what the
    /// failure-injecting test file systems want.
    fn replace_existing(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.rename(from, to)
    }
}

/// The real file system.
#[derive(Debug, Clone, Copy, Default)]
pub struct RealFs;

/// How much [`RealFs::write_new_progress`] writes between progress reports.
const WRITE_CHUNK: usize = 8 << 20;

impl AtomicFs for RealFs {
    fn write_new(&self, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
        self.write_new_progress(tmp, bytes, &|_| {})
    }
    fn write_new_progress(&self, tmp: &Path, bytes: &[u8], progress: &dyn Fn(f32)) -> io::Result<()> {
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)?;
        let mut written = 0usize;
        for chunk in bytes.chunks(WRITE_CHUNK) {
            f.write_all(chunk)?;
            written = written.saturating_add(chunk.len());
            progress(written as f32 / bytes.len().max(1) as f32);
        }
        sync_file(&f)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        std::fs::rename(from, to)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        std::fs::remove_file(path)
    }
    /// Flush directory publication using a directory handle on Unix and Windows.
    /// Windows requests only `FILE_ADD_FILE`; opening or flushing the handle can fail the save.
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            sync_file(&std::fs::File::open(dir)?)
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;

            // FILE_ADD_FILE is the directory equivalent of FILE_WRITE_DATA. Requesting only
            // this right permits FlushFileBuffers without requiring GENERIC_WRITE's additional
            // directory permissions. BACKUP_SEMANTICS opens a directory, not a volume: no
            // administrative privilege or volume-wide flush is needed.
            const FILE_ADD_FILE: u32 = 0x0002;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            let directory = std::fs::OpenOptions::new().access_mode(FILE_ADD_FILE).custom_flags(FILE_FLAG_BACKUP_SEMANTICS).open(dir)?;
            directory.sync_all()
        }
        #[cfg(not(any(unix, windows)))]
        {
            let _ = dir;
            Ok(())
        }
    }
    fn copy_metadata(&self, from: &Path, to: &Path) -> io::Result<()> {
        meta::copy_metadata(from, to)
    }
    fn replace_existing(&self, from: &Path, to: &Path) -> io::Result<()> {
        meta::replace_existing(from, to)
    }
    fn is_readonly(&self, path: &Path) -> bool {
        std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.permissions().readonly())
    }
}

/// Flush `file` to stable storage (`sync_all`). On Apple platforms that is a full flush
/// (`F_FULLFSYNC`); where the file system can't do one (SMB shares refuse it with `ENOTSUP`,
/// #1336; Tart's shared folders with `ENOTTY`) this falls back to a plain `fsync`, as Apple's own
/// `removefile` does. Any other error is returned as it is.
pub fn sync_file(file: &std::fs::File) -> io::Result<()> {
    #[cfg(target_vendor = "apple")]
    {
        or_plain_fsync(file.sync_all(), || rustix::fs::fsync(file).map_err(io::Error::from))
    }
    #[cfg(not(target_vendor = "apple"))]
    {
        file.sync_all()
    }
}

/// `full`, or `fsync()` when `full` failed only because the file system can't do a full flush.
pub fn or_plain_fsync(full: io::Result<()>, fsync: impl FnOnce() -> io::Result<()>) -> io::Result<()> {
    match full {
        Err(e) if full_flush_unsupported(&e) => fsync(),
        other => other,
    }
}

/// Whether `e` is a file system refusing a full flush (`F_FULLFSYNC`): `ENOTSUP` (45, SMB) or
/// `ENOTTY` (25) on Apple platforms. Never elsewhere: there `sync_all` is a plain `fsync`.
fn full_flush_unsupported(e: &io::Error) -> bool {
    const ENOTTY: i32 = 25;
    const ENOTSUP: i32 = 45;
    cfg!(target_vendor = "apple") && matches!(e.raw_os_error(), Some(ENOTSUP | ENOTTY))
}

/// How hard to retry a rename that fails with `PermissionDenied`.
#[derive(Debug, Clone, Copy)]
pub struct RenameRetry {
    /// Retries after the first attempt (0 = no retry).
    pub attempts: u32,
    /// Delay before the first retry; doubles each time.
    pub first_delay_ms: u64,
}

impl RenameRetry {
    /// The platform default: retry on Windows (about 1.3 s in total), never elsewhere, where
    /// `PermissionDenied` is a real permission problem that waiting won't fix.
    pub fn platform() -> Self {
        if cfg!(windows) { Self { attempts: 7, first_delay_ms: 10 } } else { Self { attempts: 0, first_delay_ms: 0 } }
    }
}

/// Atomically replace (or create) `path` with `bytes`. See the module docs.
///
/// Errors keep their [`io::ErrorKind`]. Before replacement they say the original is unchanged;
/// a Windows publication-flush error says the new file may not survive a power loss.
pub fn atomic_write(path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_with(&RealFs, RenameRetry::platform(), path, bytes)
}

/// [`atomic_write`] reporting the fraction of `bytes` written to `progress` as it goes (a save's
/// progress bar, #3389); the flush and rename follow the last report.
pub fn atomic_write_progress(path: &Path, bytes: &[u8], progress: &dyn Fn(f32)) -> io::Result<()> {
    atomic_write_with_progress(&RealFs, RenameRetry::platform(), path, bytes, progress)
}

/// [`atomic_write`] through an explicit file system and retry policy (the failure-injection seam).
pub fn atomic_write_with(fs: &dyn AtomicFs, retry: RenameRetry, path: &Path, bytes: &[u8]) -> io::Result<()> {
    atomic_write_with_progress(fs, retry, path, bytes, &|_| {})
}

/// [`atomic_write_with`] reporting write progress (see [`atomic_write_progress`]).
pub fn atomic_write_with_progress(fs: &dyn AtomicFs, retry: RenameRetry, path: &Path, bytes: &[u8], progress: &dyn Fn(f32)) -> io::Result<()> {
    let target = resolve_symlink(path).map_err(|e| context(e, format!("cannot save {}", path.display())))?;
    let name = target.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{}: not a file path", path.display())))?;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    std::fs::create_dir_all(&dir).map_err(|e| context(e, format!("cannot create the folder {}", dir.display())))?;
    if fs.is_readonly(&target) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("cannot save {}: the file is read-only; {UNCHANGED}", path.display())));
    }

    // Create a fresh temporary file next to the target (same directory, so the rename never
    // crosses a device). The name is unique per process and call; retry a few on collisions.
    let mut tmp = None;
    for _ in 0..8 {
        let candidate = dir.join(temp_name(&name.to_string_lossy()));
        match fs.write_new_progress(&candidate, bytes, progress) {
            Ok(()) => {
                tmp = Some(candidate);
                break;
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                // A partial temp file may exist; the destination was never touched.
                let _ = fs.remove(&candidate);
                return Err(write_error(e, path, &dir));
            }
        }
    }
    let tmp = tmp.ok_or_else(|| io::Error::new(io::ErrorKind::AlreadyExists, format!("{}: could not create a temporary file", dir.display())))?;

    // Keep what the file being replaced carried: its mode (e.g. 0600) instead of the process
    // default, its extended attributes, and its ACL. Best effort, and before the swap, so the
    // file the user ends up with never passes through a more permissive state.
    let replacing = target.exists();
    if replacing {
        let _ = fs.copy_metadata(&target, &tmp);
    }

    // Replacing an existing file goes through `replace_existing`, which on Windows is the one
    // call that can keep the destination's DACL and named streams; creating a new one is a
    // plain rename, with nothing to preserve.
    let swap = || if replacing { fs.replace_existing(&tmp, &target) } else { fs.rename(&tmp, &target) };
    if let Err(e) = retry_rename(retry, swap) {
        let _ = fs.remove(&tmp);
        return Err(rename_error(e, path));
    }
    #[cfg(windows)]
    fs.sync_dir(&dir).map_err(|e| publication_error(e, path))?;
    // Keep the existing Unix policy here; #5420 separately changes its error handling.
    #[cfg(not(windows))]
    let _ = fs.sync_dir(&dir);
    Ok(())
}

/// Run `rename`, retrying with backoff while it fails with `PermissionDenied` (see
/// [`RenameRetry`]). For callers that rename through their own handles (capability directories).
pub fn retry_rename(retry: RenameRetry, mut rename: impl FnMut() -> io::Result<()>) -> io::Result<()> {
    let mut delay = retry.first_delay_ms;
    let mut left = retry.attempts;
    loop {
        match rename() {
            Err(e) if e.kind() == io::ErrorKind::PermissionDenied && left > 0 => {
                left -= 1;
                sleep_ms(delay);
                delay = delay.saturating_mul(2);
            }
            r => return r,
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn sleep_ms(ms: u64) {
    std::thread::sleep(std::time::Duration::from_millis(ms));
}
#[cfg(target_arch = "wasm32")]
fn sleep_ms(_ms: u64) {}

/// How deep a symlink chain we follow before giving up, like the OS limit.
const MAX_LINK_DEPTH: u32 = 40;

/// Write through a symlink to the file it points at, instead of replacing the link.
///
/// A dangling link (its target, or a folder on the way to it, doesn't exist yet) resolves to its
/// target too, so saving through one creates the target instead of silently turning the link
/// into a regular file (#6605). A chain that never ends is an error, never a link replacement.
fn resolve_symlink(path: &Path) -> io::Result<PathBuf> {
    if !std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_symlink()) {
        return Ok(path.to_path_buf());
    }
    // A resolvable chain: canonicalize follows every link and relative target in one go.
    if let Ok(real) = std::fs::canonicalize(path) {
        return Ok(real);
    }
    // Dangling chain: walk the links lexically; the final target may not exist yet.
    let mut current = path.to_path_buf();
    for _ in 0..MAX_LINK_DEPTH {
        let Ok(link) = std::fs::read_link(&current) else {
            return Ok(current); // not (or no longer) a link: the chain ends here
        };
        let base = current.parent().unwrap_or(Path::new(".")).to_path_buf();
        // `..` stays in the path for the OS to resolve: folding it by text is wrong when a folder
        // on the way is itself a symlink.
        current = if link.is_absolute() { link } else { base.join(link) };
        if !std::fs::symlink_metadata(&current).is_ok_and(|m| m.file_type().is_symlink()) {
            return Ok(current);
        }
    }
    Err(io::Error::new(io::ErrorKind::InvalidInput, format!("{}: too many levels of symbolic links", path.display())))
}

/// A unique hidden temporary file name to write beside `file` before renaming over it.
pub fn temp_name(file: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Keep the name short: a long file name plus the suffix must stay under NAME_MAX.
    let stem: String = file.chars().take(64).collect();
    format!(".{stem}.{}-{n}.pcsave.tmp", std::process::id())
}

fn context(e: io::Error, msg: String) -> io::Error {
    io::Error::new(e.kind(), format!("{msg}: {e}"))
}

const UNCHANGED: &str = "the original file was not changed";

/// Replacement already happened: never claim that the original file is unchanged or undo it.
#[cfg(windows)]
pub(crate) fn publication_error(error: io::Error, path: &Path) -> io::Error {
    io::Error::new(
        error.kind(),
        format!("wrote {}, but could not confirm the save is durable; it may not survive a power loss or crash: {error}", path.display()),
    )
}

fn write_error(e: io::Error, path: &Path, dir: &Path) -> io::Error {
    let msg = match e.kind() {
        io::ErrorKind::PermissionDenied | io::ErrorKind::ReadOnlyFilesystem => {
            format!("cannot save {}: the folder {} is not writable ({e}); {UNCHANGED}", path.display(), dir.display())
        }
        io::ErrorKind::StorageFull => format!("cannot save {}: the disk is full ({e}); {UNCHANGED}", path.display()),
        _ => format!("cannot save {}: {e}; {UNCHANGED}", path.display()),
    };
    io::Error::new(e.kind(), msg)
}

fn rename_error(e: io::Error, path: &Path) -> io::Error {
    let msg = if is_cross_device(&e) {
        format!("cannot save {}: the temporary file is on a different volume than the destination ({e}); {UNCHANGED}", path.display())
    } else if e.kind() == io::ErrorKind::PermissionDenied {
        format!("cannot save {}: the file is locked or read-only ({e}); {UNCHANGED}", path.display())
    } else {
        format!("cannot save {}: {e}; {UNCHANGED}", path.display())
    };
    io::Error::new(e.kind(), msg)
}

fn is_cross_device(e: &io::Error) -> bool {
    // EXDEV is 18 on Linux, macOS and the BSDs; ERROR_NOT_SAME_DEVICE is 17 on Windows.
    e.kind() == io::ErrorKind::CrossesDevices || (cfg!(unix) && e.raw_os_error() == Some(18)) || (cfg!(windows) && e.raw_os_error() == Some(17))
}

#[cfg(test)]
mod tests;
