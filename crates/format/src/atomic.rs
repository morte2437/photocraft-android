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
//! - **Hidden flags (network shares):** the temporary file's name never starts with a dot. An SMB
//!   server that hides dot files marks one hidden when it is created, the mark survives the
//!   rename, and the saved document then doesn't show in Finder (#11624). See [`temp_name`].
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

use std::io::{self, Seek, Write};
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
    /// Create `tmp` (it must not exist), let `write` fill it as it serializes, and flush it
    /// ([`sync_file`]). By default the bytes are collected in memory and handed to
    /// [`write_new`](Self::write_new) (test doubles); [`RealFs`] streams them into the file, so the
    /// whole file never sits in memory (#6607).
    fn write_new_stream(&self, tmp: &Path, write: &mut StreamWriter<'_>) -> io::Result<()> {
        let mut bytes = io::Cursor::new(Vec::new());
        write(&mut bytes)?;
        self.write_new(tmp, bytes.get_ref())
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

/// A stream [`atomic_write_stream`]'s writer fills: the temporary file (or, in tests, memory).
pub trait WriteSeek: Write + Seek {}
impl<T: Write + Seek + ?Sized> WriteSeek for T {}

/// Fills the temporary file of a streaming atomic write, starting at its beginning.
pub type StreamWriter<'a> = dyn FnMut(&mut dyn WriteSeek) -> io::Result<()> + 'a;

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
    fn write_new_stream(&self, tmp: &Path, write: &mut StreamWriter<'_>) -> io::Result<()> {
        let f = std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)?;
        let mut out = io::BufWriter::new(f);
        // `AlreadyExists` means a temporary name collided, which makes the caller retry with
        // another name and leave this one alone: from the writer, once the file is ours, it is
        // a plain failure (the partial file is removed, the writer is not run again).
        write(&mut out).map_err(|e| if e.kind() == io::ErrorKind::AlreadyExists { io::Error::other(e) } else { e })?;
        let f = out.into_inner().map_err(io::IntoInnerError::into_error)?;
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
            open_dir_for_sync(dir)?.map_or(Ok(()), |d| sync_file(&d))
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
    replace_with(fs, retry, path, true, &mut |tmp| fs.write_new_progress(tmp, bytes, progress))
}

/// [`atomic_write`] that never follows a symlink at `path`: a link there is replaced by the new
/// file (the rename swaps the directory entry), so a link planted between a caller's own check
/// and this write cannot carry it outside the folder (a store managing its own entries).
pub fn atomic_write_no_follow(path: &Path, bytes: &[u8]) -> io::Result<()> {
    replace_with(&RealFs, RenameRetry::platform(), path, false, &mut |tmp| RealFs.write_new_progress(tmp, bytes, &|_| {}))
}

/// [`atomic_write`] for a file serialized as it is written (#6607): `write` fills the temporary
/// file beside `path` from its start (it may seek back to patch lengths), then the file is
/// flushed and renamed over `path` with [`atomic_write`]'s retries and errors. If `write` fails,
/// the temporary file is removed and `path` is left as it was. `write` runs once (again only if
/// a temporary name collided before anything was written).
pub fn atomic_write_stream(path: &Path, write: &mut StreamWriter<'_>) -> io::Result<()> {
    atomic_write_stream_with(&RealFs, RenameRetry::platform(), path, write)
}

/// [`atomic_write_stream`] reporting the fraction written to `progress` as it goes: the furthest
/// position reached over `expected_len` (an estimate; capped below 1.0 until `write` is done).
pub fn atomic_write_stream_progress(path: &Path, expected_len: u64, progress: &dyn Fn(f32), write: &mut StreamWriter<'_>) -> io::Result<()> {
    atomic_write_stream_with(&RealFs, RenameRetry::platform(), path, &mut |w| {
        let mut counted = Progress { w, pos: 0, reached: 0, reported: 0, expected: expected_len.max(1), progress };
        write(&mut counted)?;
        progress(1.0);
        Ok(())
    })
}

/// [`atomic_write_stream`] through an explicit file system and retry policy.
pub fn atomic_write_stream_with(fs: &dyn AtomicFs, retry: RenameRetry, path: &Path, write: &mut StreamWriter<'_>) -> io::Result<()> {
    replace_with(fs, retry, path, true, &mut |tmp| fs.write_new_stream(tmp, write))
}

/// A stream reporting how far it has got (see [`atomic_write_stream_progress`]), every
/// [`WRITE_CHUNK`] bytes.
struct Progress<'a, 'p> {
    w: &'a mut dyn WriteSeek,
    /// Tracked here: asking a buffered file for its position flushes it.
    pos: u64,
    reached: u64,
    reported: u64,
    expected: u64,
    progress: &'p dyn Fn(f32),
}

impl Write for Progress<'_, '_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        let n = self.w.write(b)?;
        self.pos += n as u64;
        let at = self.pos;
        if at > self.reached {
            self.reached = at;
            if at - self.reported >= WRITE_CHUNK as u64 {
                self.reported = at;
                (self.progress)((at as f64 / self.expected as f64).min(0.999) as f32);
            }
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.w.flush()
    }
}

impl Seek for Progress<'_, '_> {
    fn seek(&mut self, to: io::SeekFrom) -> io::Result<u64> {
        self.pos = self.w.seek(to)?;
        Ok(self.pos)
    }
}

/// The steps every atomic write shares: `write_tmp` fills a fresh temporary file beside the
/// target, which is then renamed over it.
fn replace_with(fs: &dyn AtomicFs, retry: RenameRetry, path: &Path, follow: bool, write_tmp: &mut dyn FnMut(&Path) -> io::Result<()>) -> io::Result<()> {
    let target = if follow { resolve_symlink(path).map_err(|e| context(e, format!("cannot save {}", path.display())))? } else { path.to_path_buf() };
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
        match write_tmp(&candidate) {
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
    // A link that is replaced rather than followed has nothing to carry over: reading the target's
    // attributes would open whatever it points at (a FIFO with no writer blocks there).
    let replaced_link = !follow && std::fs::symlink_metadata(&target).is_ok_and(|m| m.file_type().is_symlink());
    if replacing && !replaced_link {
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
    // The new bytes are in place, but until the directory is flushed the rename may not survive
    // a power loss (#4968). Windows fails the save on any failed flush (#7858). Elsewhere a file
    // system that can't flush directories at all counts as done; any other failure is reported,
    // so the caller keeps the document unsaved and its recovery data instead of discarding them
    // on a save that may not last.
    #[cfg(windows)]
    dir_synced(fs.sync_dir(&dir)).map_err(|e| publication_error(e, path))?;
    #[cfg(not(windows))]
    dir_synced(fs.sync_dir(&dir)).map_err(|e| unconfirmed_error(e, path))?;
    Ok(())
}

/// `dir` opened to flush it, or `None` when it can't be opened for that because it has no read
/// permission (a write-and-search-only folder): no flush can be attempted there, so none failed.
/// Any other error is returned.
#[cfg(unix)]
fn open_dir_for_sync(dir: &Path) -> io::Result<Option<std::fs::File>> {
    match std::fs::File::open(dir) {
        Ok(d) => Ok(Some(d)),
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied => Ok(None),
        Err(e) => Err(e),
    }
}

/// `result` of flushing a directory, with "this file system can't flush directories" counted as
/// success: `EINVAL` (Linux file systems without directory fsync), `ENOTSUP`/`EOPNOTSUPP` and,
/// on Apple platforms, `ENOTTY`, or [`io::ErrorKind::Unsupported`]. Waiting or retrying won't
/// change those, and failing every save there would make saving impossible. On Windows that is
/// `ERROR_INVALID_FUNCTION` (1) and `ERROR_NOT_SUPPORTED` (50), which network shares (SMB, #9440)
/// return when asked to flush a directory handle; access denied and I/O errors stay failures.
/// Any other error is returned: the flush was attempted and failed.
pub(crate) fn dir_synced(result: io::Result<()>) -> io::Result<()> {
    match result {
        Err(e) if dir_sync_unsupported(&e) => Ok(()),
        other => other,
    }
}

fn dir_sync_unsupported(e: &io::Error) -> bool {
    if e.kind() == io::ErrorKind::Unsupported {
        return true;
    }
    // Win32 codes, not errno: ERROR_INVALID_FUNCTION and ERROR_NOT_SUPPORTED.
    if cfg!(windows) {
        return matches!(e.raw_os_error(), Some(1 | 50));
    }
    // errno values: EINVAL is 22 everywhere; ENOTSUP and EOPNOTSUPP are 95 on Linux and
    // Android, 45 and 102 on Apple platforms, 45 on the BSDs; ENOTTY is 25. Elsewhere 45 means
    // something else (illumos: EDEADLK), so only EINVAL and `Unsupported` count there.
    const EINVAL: i32 = 22;
    let unsupported: &[i32] = if cfg!(any(target_os = "linux", target_os = "android")) {
        &[EINVAL, 95]
    } else if cfg!(target_vendor = "apple") {
        &[EINVAL, 45, 102, 25]
    } else if cfg!(any(target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", target_os = "dragonfly")) {
        &[EINVAL, 45]
    } else if cfg!(unix) {
        &[EINVAL]
    } else {
        &[]
    };
    e.raw_os_error().is_some_and(|code| unsupported.contains(&code))
}

/// The error for a file that was published but whose directory flush failed: same kind, saying
/// the file is in place but may not survive a power loss. (Windows reports `publication_error`.)
#[cfg(not(windows))]
fn unconfirmed_error(e: io::Error, path: &Path) -> io::Error {
    context(
        e,
        format!(
            "{} was written, but the folder could not be flushed to disk, so the save may not survive a power loss or crash; save again or to another folder",
            path.display()
        ),
    )
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

/// What every [`temp_name`] ends with.
const TEMP_SUFFIX: &str = ".pcsave.tmp";

/// How many characters of the file name a [`temp_name`] keeps.
const TEMP_STEM_CHARS: usize = 64;

/// A unique temporary file name to write beside `file` before renaming over it:
/// `{file}.{pid}-{n}.pcsave.tmp`.
///
/// It does not start with a dot of its own, so it is not hidden: an SMB server that hides dot
/// files sets the hidden flag on such a file when it is created, the flag survives the rename,
/// and the saved document would be invisible in Finder (#11624). The cost is that the file shows
/// in a folder listing while a save writes it, and one an interrupted save leaves beside the
/// document stays visible until it is removed (#6604). The
/// process id, counter and suffix keep it apart from the files people name, and it is created
/// with `create_new`, so it can never replace one. Older versions wrote
/// `.{file}.{pid}-{n}.pcsave.tmp`; [`temp_owner`] recognizes both.
pub fn temp_name(file: &str) -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let n = COUNTER.fetch_add(1, Ordering::Relaxed);
    // Keep the name short: a long file name plus the suffix must stay under NAME_MAX.
    let stem: String = file.chars().take(TEMP_STEM_CHARS).collect();
    format!("{stem}.{}-{n}{TEMP_SUFFIX}", std::process::id())
}

/// The id of the process that wrote `name`, if it is a [`temp_name`]: today's
/// `{file}.{pid}-{n}.pcsave.tmp` or the `.{file}.{pid}-{n}.pcsave.tmp` older versions wrote, so
/// their leftovers are still cleaned up.
pub fn temp_owner(name: &str) -> Option<u32> {
    let rest = name.strip_suffix(TEMP_SUFFIX)?;
    let (stem, ids) = rest.rsplit_once('.')?;
    let (pid, n) = ids.split_once('-')?;
    // Only a file part `temp_name` writes: at most `TEMP_STEM_CHARS` characters, or one more when
    // the first is the dot older versions put in front of them.
    let chars = stem.chars().count();
    if chars > TEMP_STEM_CHARS && !(chars == TEMP_STEM_CHARS + 1 && stem.starts_with('.')) {
        return None;
    }
    let stem = stem.strip_prefix('.').unwrap_or(stem);
    // Only numbers `temp_name` writes: plain decimal digits with no leading zero (a lone "0" is
    // fine), a `u32` pid and the `u64` counter it counts with. Any other name is someone else's
    // file, and the sweep must not delete it.
    let decimal = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) && (s == "0" || !s.starts_with('0'));
    if stem.is_empty() || !decimal(pid) || !decimal(n) || n.parse::<u64>().is_err() {
        return None;
    }
    pid.parse().ok()
}

fn context(e: io::Error, msg: String) -> io::Error {
    io::Error::new(e.kind(), format!("{msg}: {e}"))
}

const UNCHANGED: &str = "the original file was not changed";

/// Marks the error of a Windows replacement that moved the original to its backup name and then
/// could neither complete the save nor put the original back: the original survives only at that
/// backup, so the destination is not the unchanged original and must never be reported as such
/// (#11178). `rename_error` recognizes the marker and passes the error through untouched.
const RECOVERY_UNRESOLVED: &str = "could not restore the original";

/// The error for that unresolved Windows replacement state: names the recovery copy and says the
/// destination is currently missing, instead of the usual `UNCHANGED` reassurance (#11178).
/// When the restoration failed because the backup itself is gone (`NotFound`), nothing survives
/// at that name and saying so would be a second false comfort after the lost original.
/// Its kind is deliberately not `PermissionDenied`: the retry loop must not pick a fresh backup
/// name over the unresolved recovery and forget where the original is.
#[cfg(any(windows, test))]
pub(crate) fn unresolved_recovery_error(dest: &Path, completion: &io::Error, restoration: &io::Error, recovery: &Path) -> io::Error {
    let msg = if restoration.kind() == io::ErrorKind::NotFound {
        format!(
            "cannot save {}: the previous version moved to its backup name but {RECOVERY_UNRESOLVED} from it \
             (completing the save failed: {completion}; the backup {} no longer exists: {restoration}), \
             so the destination currently holds no file under that name — check whether it was moved",
            dest.display(),
            recovery.display()
        )
    } else {
        format!(
            "cannot save {}: the previous version moved to its backup name but {RECOVERY_UNRESOLVED} from it \
             (completing the save failed: {completion}; restoring the backup failed: {restoration}); \
             your document survives at {}, recover it from there before saving again",
            dest.display(),
            recovery.display()
        )
    };
    io::Error::other(msg)
}

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
    // A Windows replacement that moved the original to its backup and failed to restore it is
    // already fully described (with the recovery location); never append the `UNCHANGED`
    // reassurance, which would be false in that state (#11178).
    if e.to_string().contains(RECOVERY_UNRESOLVED) {
        return e;
    }
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
