use super::*;
use std::cell::Cell;

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("pc-atomic-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
        }
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Only the target remains: no temporary file was left behind.
fn assert_no_temp(dir: &Path) {
    let leftovers: Vec<_> =
        std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp")).collect();
    assert!(leftovers.is_empty(), "temporary files left behind: {leftovers:?}");
}

const ORIGINAL: &[u8] = b"the user's precious original document";

/// Real file system with failures injected at chosen steps.
#[derive(Default)]
struct Faulty {
    /// Write this many bytes, then fail like a full disk.
    fail_write_after: Option<usize>,
    /// Fail every rename with this kind.
    fail_rename: Option<io::ErrorKind>,
    /// Fail the first N renames with PermissionDenied (a transient lock), then succeed.
    locked_renames: Cell<u32>,
    renames: Cell<u32>,
    /// Fail the directory flush (the durability barrier after publication, #4968, #7858)
    /// with this raw OS error, or with this kind.
    fail_sync_dir: Option<Result<i32, io::ErrorKind>>,
    synced: Cell<u32>,
}

impl AtomicFs for Faulty {
    fn write_new(&self, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
        if let Some(n) = self.fail_write_after {
            let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(tmp)?;
            f.write_all(&bytes[..n.min(bytes.len())])?;
            return Err(io::Error::new(io::ErrorKind::StorageFull, "injected: no space left on device"));
        }
        RealFs.write_new(tmp, bytes)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.renames.set(self.renames.get() + 1);
        if let Some(kind) = self.fail_rename {
            return Err(io::Error::new(kind, "injected rename failure"));
        }
        if self.locked_renames.get() > 0 {
            self.locked_renames.set(self.locked_renames.get() - 1);
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "injected: sharing violation"));
        }
        RealFs.rename(from, to)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        RealFs.remove(path)
    }
    /// Count publication barriers and optionally fail before the real directory flush.
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        self.synced.set(self.synced.get() + 1);
        match self.fail_sync_dir {
            Some(Ok(code)) => Err(io::Error::from_raw_os_error(code)),
            Some(Err(kind)) => Err(io::Error::new(kind, "injected directory flush failure")),
            None => RealFs.sync_dir(dir),
        }
    }
    fn copy_metadata(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.copy_metadata(from, to)
    }
}

const NO_RETRY: RenameRetry = RenameRetry { attempts: 0, first_delay_ms: 0 };

/// A successful rename does not establish power-loss durability. A failed Windows publication
/// barrier must leave the new bytes in place, report uncertainty, and allow a subsequent retry.
#[cfg(windows)]
#[test]
fn a_failed_publication_flush_is_reported_and_can_be_retried() {
    let d = TempDir::new("publication-flush");
    let p = d.0.join("document.pcraft");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_sync_dir: Some(Err(io::ErrorKind::Other)), ..Default::default() };

    let error = atomic_write_with(&fs, NO_RETRY, &p, b"new document").unwrap_err();
    assert_eq!(fs.synced.get(), 1, "{error}");
    assert_eq!(error.kind(), io::ErrorKind::Other);
    assert!(error.to_string().contains("document.pcraft"), "{error}");
    assert!(error.to_string().contains("may not survive"), "{error}");
    assert!(!error.to_string().contains(UNCHANGED), "the replacement already happened: {error}");
    assert_eq!(std::fs::read(&p).unwrap(), b"new document");
    assert_no_temp(&d.0);

    atomic_write(&p, b"retried document").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"retried document");
    assert_no_temp(&d.0);
}

/// A real Windows directory handle must flush successfully and preserve directory-open errors.
#[cfg(windows)]
#[test]
fn windows_directory_flush_opens_a_directory_and_reports_open_errors() {
    let d = TempDir::new("sync-directory");
    RealFs.sync_dir(&d.0).unwrap();
    assert_eq!(RealFs.sync_dir(&d.0.join("missing")).unwrap_err().kind(), io::ErrorKind::NotFound);
}

#[test]
fn replaces_and_creates() {
    let d = TempDir::new("ok");
    let p = d.0.join("doc.psd");
    atomic_write(&p, b"first").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"first");
    atomic_write(&p, b"second, longer").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"second, longer");
    assert_no_temp(&d.0);
}

/// #3389: a large write reports how far it has got, chunk by chunk, up to 1.0, and writes the
/// same bytes as a plain atomic write.
#[test]
fn reports_write_progress() {
    let d = TempDir::new("progress");
    let p = d.0.join("big.psd");
    let bytes: Vec<u8> = (0..WRITE_CHUNK * 2 + 123).map(|i| (i % 251) as u8).collect();
    let seen = std::sync::Mutex::new(Vec::new());
    atomic_write_progress(&p, &bytes, &|f| seen.lock().unwrap().push(f)).unwrap();
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.len(), 3, "one report per chunk: {seen:?}");
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "monotonic: {seen:?}");
    assert!(seen[0] > 0.0 && seen[0] < 0.6, "{seen:?}");
    assert_eq!(seen.last().copied(), Some(1.0));
    assert!(std::fs::read(&p).unwrap() == bytes, "the file holds the bytes");
    assert_no_temp(&d.0);
}

/// Writes "head", a 4-byte placeholder, `len` bytes, then patches the placeholder by seeking back.
fn patched_writer(len: usize) -> impl FnMut(&mut dyn WriteSeek) -> io::Result<()> {
    move |w| {
        w.write_all(b"head")?;
        let at = w.stream_position()?;
        w.write_all(&[0; 4])?;
        w.write_all(&vec![9u8; len])?;
        w.seek(io::SeekFrom::Start(at))?;
        w.write_all(&(len as u32).to_be_bytes())?;
        w.seek(io::SeekFrom::End(0))?;
        Ok(())
    }
}

fn patched_bytes(len: usize) -> Vec<u8> {
    let mut v = b"head".to_vec();
    v.extend_from_slice(&(len as u32).to_be_bytes());
    v.extend(std::iter::repeat_n(9u8, len));
    v
}

/// #6607: a streamed save replaces the file with what its writer wrote, seeks included, and leaves
/// no temporary file.
#[test]
fn streamed_write_replaces_and_creates() {
    let d = TempDir::new("stream");
    let p = d.0.join("doc.psd");
    atomic_write_stream(&p, &mut patched_writer(3)).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), patched_bytes(3));
    atomic_write_stream(&p, &mut patched_writer(100_000)).unwrap();
    assert!(std::fs::read(&p).unwrap() == patched_bytes(100_000));
    assert_no_temp(&d.0);
}

/// #6607: a writer that fails halfway (a serializer error, a full disk) leaves the original as it
/// was, with the same message as a buffered save.
#[test]
fn streamed_write_failure_halfway_keeps_original() {
    let d = TempDir::new("stream-fail");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let e = atomic_write_stream(&p, &mut |w| {
        w.write_all(&[1; 70_000])?;
        Err(io::Error::new(io::ErrorKind::StorageFull, "injected: no space left on device"))
    })
    .unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::StorageFull);
    assert!(e.to_string().contains("the disk is full") && e.to_string().contains("not changed"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

/// A writer failing with `AlreadyExists` (from whatever it reads) is a failed save, not a
/// temporary name collision: it runs once and its partial file is removed.
#[test]
fn streamed_write_failing_with_already_exists_runs_once() {
    let d = TempDir::new("stream-exists");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let runs = Cell::new(0);
    let e = atomic_write_stream(&p, &mut |w| {
        runs.set(runs.get() + 1);
        w.write_all(&[1; 70_000])?;
        Err(io::Error::new(io::ErrorKind::AlreadyExists, "injected"))
    })
    .unwrap_err();
    assert_eq!(runs.get(), 1);
    assert!(e.to_string().contains("not changed"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

/// A streamed save renames like a buffered one: retried while locked, the original kept when the
/// lock outlasts the retries.
#[test]
fn streamed_write_retries_a_locked_rename() {
    let d = TempDir::new("stream-locked");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { locked_renames: Cell::new(2), ..Default::default() };
    atomic_write_stream_with(&fs, RenameRetry { attempts: 3, first_delay_ms: 1 }, &p, &mut patched_writer(10)).unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), patched_bytes(10));
    assert_eq!(fs.renames.get(), 3);
    let fs = Faulty { locked_renames: Cell::new(10), ..Default::default() };
    let e = atomic_write_stream_with(&fs, RenameRetry { attempts: 1, first_delay_ms: 1 }, &p, &mut patched_writer(20)).unwrap_err();
    assert!(e.to_string().contains("locked"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), patched_bytes(10));
    assert_no_temp(&d.0);
}

/// A streamed save reports the furthest position over the expected length, chunk by chunk, and
/// 1.0 once the writer is done.
#[test]
fn streamed_write_reports_progress() {
    let d = TempDir::new("stream-progress");
    let p = d.0.join("big.psd");
    let len = WRITE_CHUNK * 2 + 123;
    let seen = std::sync::Mutex::new(Vec::new());
    let mut write = |w: &mut dyn WriteSeek| {
        for chunk in patched_bytes(len).chunks(WRITE_CHUNK / 4) {
            w.write_all(chunk)?;
        }
        Ok(())
    };
    atomic_write_stream_progress(&p, len as u64 + 8, &|f| seen.lock().unwrap().push(f), &mut write).unwrap();
    let seen = seen.into_inner().unwrap();
    assert_eq!(seen.len(), 3, "two chunks, then done: {seen:?}");
    assert!(seen.windows(2).all(|w| w[0] < w[1]), "monotonic: {seen:?}");
    assert_eq!(seen.last().copied(), Some(1.0));
    assert!(std::fs::read(&p).unwrap() == patched_bytes(len));
    assert_no_temp(&d.0);
}

#[test]
fn creates_missing_parent_folders() {
    let d = TempDir::new("parent");
    let p = d.0.join("a/b/out.png");
    atomic_write(&p, b"x").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"x");
}

/// A partial temporary write must preserve the original and never reach publication or its flush.
#[test]
fn write_failure_halfway_keeps_original() {
    let d = TempDir::new("write");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_write_after: Some(5), ..Default::default() };
    let e = atomic_write_with(&fs, NO_RETRY, &p, &[7u8; 4096]).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::StorageFull);
    assert!(e.to_string().contains("not changed"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_eq!(fs.renames.get(), 0, "never renames after a failed write");
    assert_eq!(fs.synced.get(), 0, "nothing was published");
    assert_no_temp(&d.0);
}

/// A failed replacement must preserve the original and skip the publication barrier.
#[test]
fn rename_failure_keeps_original() {
    let d = TempDir::new("rename");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_rename: Some(io::ErrorKind::Other), ..Default::default() };
    assert!(atomic_write_with(&fs, NO_RETRY, &p, b"new").is_err());
    assert_eq!(fs.synced.get(), 0, "a failed replacement never reaches the publication barrier");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

/// The unresolved Windows recovery state (branch coverage, #11178): the error names the backup
/// where the original survives, `rename_error` passes it through without the false `UNCHANGED`
/// promise, and `retry_rename` does not retry it into a fresh backup name that would forget it.
#[test]
fn an_unresolved_recovery_reports_the_backup_and_is_not_retried() {
    let unresolved = || {
        unresolved_recovery_error(
            Path::new("/docs/report.psd"),
            &io::Error::new(io::ErrorKind::PermissionDenied, "completion locked"),
            &io::Error::new(io::ErrorKind::PermissionDenied, "restoration locked"),
            Path::new("/docs/.report.psd.7.pcsave.tmp"),
        )
    };
    let reported = rename_error(unresolved(), Path::new("/docs/report.psd"));
    let msg = reported.to_string();
    assert!(msg.contains(".report.psd.7.pcsave.tmp") && msg.contains("recover"), "the recovery location survives reporting: {msg}");
    assert!(!msg.contains(UNCHANGED), "the destination is not the unchanged original in this state: {msg}");
    let calls = Cell::new(0);
    let e = retry_rename(RenameRetry { attempts: 7, first_delay_ms: 0 }, || {
        calls.set(calls.get() + 1);
        Err(unresolved())
    })
    .unwrap_err();
    assert_eq!(calls.get(), 1, "the retry loop must not run again over an unresolved recovery");
    assert!(!e.to_string().contains(UNCHANGED), "{e}");
}

#[test]
fn cross_device_rename_errors_clearly_and_never_writes_in_place() {
    let d = TempDir::new("exdev");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_rename: Some(io::ErrorKind::CrossesDevices), ..Default::default() };
    let e = atomic_write_with(&fs, NO_RETRY, &p, b"new").unwrap_err();
    assert!(e.to_string().contains("different volume"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

#[test]
fn locked_target_rename_is_retried() {
    let d = TempDir::new("locked");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { locked_renames: Cell::new(2), ..Default::default() };
    atomic_write_with(&fs, RenameRetry { attempts: 3, first_delay_ms: 1 }, &p, b"new").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    assert_eq!(fs.renames.get(), 3);
    assert_no_temp(&d.0);
}

#[test]
fn lock_that_outlasts_the_retries_keeps_original() {
    let d = TempDir::new("locked2");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { locked_renames: Cell::new(10), ..Default::default() };
    let e = atomic_write_with(&fs, RenameRetry { attempts: 2, first_delay_ms: 1 }, &p, b"new").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    assert!(e.to_string().contains("locked"), "{e}");
    assert_eq!(fs.renames.get(), 3);
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

/// A file system that reports the target as read-only and records every write and rename.
#[derive(Default)]
struct ReadOnlyTarget {
    writes: Cell<u32>,
    renames: Cell<u32>,
}

impl AtomicFs for ReadOnlyTarget {
    fn is_readonly(&self, _path: &Path) -> bool {
        true
    }
    fn write_new(&self, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
        self.writes.set(self.writes.get() + 1);
        RealFs.write_new(tmp, bytes)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        self.renames.set(self.renames.get() + 1);
        RealFs.rename(from, to)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        RealFs.remove(path)
    }
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        RealFs.sync_dir(dir)
    }
    fn copy_metadata(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.copy_metadata(from, to)
    }
}

/// #6600: a read-only destination is refused before anything is written, with an error that says
/// so, instead of being replaced by the rename (which only needs the folder to be writable).
#[test]
fn read_only_target_is_refused_before_anything_is_written() {
    let d = TempDir::new("rofake");
    let p = d.0.join("a.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = ReadOnlyTarget::default();
    let e = atomic_write_with(&fs, NO_RETRY, &p, b"new").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    assert!(e.to_string().contains("a.psd") && e.to_string().contains("read-only") && e.to_string().contains("not changed"), "{e}");
    assert_eq!((fs.writes.get(), fs.renames.get()), (0, 0), "nothing is written or renamed");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    assert_no_temp(&d.0);
}

/// #6600 on the real file system: a file marked read-only (`chmod 444`, or the Windows read-only
/// attribute) keeps its bytes and the save says why; a writable one still saves.
#[test]
fn read_only_file_on_disk_is_never_replaced() {
    let d = TempDir::new("rofile");
    let p = d.0.join("a.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let mut perms = std::fs::metadata(&p).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&p, perms.clone()).unwrap();
    let r = atomic_write(&p, b"new");
    let unchanged = std::fs::read(&p).unwrap();
    let still_readonly = std::fs::metadata(&p).unwrap().permissions().readonly();
    // Make it writable again so the folder can be removed (Windows refuses to delete it otherwise).
    #[allow(clippy::permissions_set_readonly_false)]
    perms.set_readonly(false);
    std::fs::set_permissions(&p, perms).unwrap();
    let e = r.unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    assert!(e.to_string().contains("the file is read-only;"), "a clear reason, not a failed rename: {e}");
    assert_eq!(unchanged, ORIGINAL);
    assert!(still_readonly);
    assert_no_temp(&d.0);
    // New files and writable files are unaffected.
    atomic_write(&p, b"new").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    atomic_write(&d.0.join("fresh.psd"), b"x").unwrap();
}

#[cfg(unix)]
#[test]
fn read_only_folder_errors_clearly_and_never_writes_in_place() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("ro");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    // The file itself stays writable: an in-place fallback would succeed and must not happen.
    std::fs::set_permissions(&d.0, std::fs::Permissions::from_mode(0o555)).unwrap();
    if std::fs::File::create(d.0.join("probe")).is_ok() {
        return; // running as root: permissions aren't enforced
    }
    let e = atomic_write(&p, b"new").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::PermissionDenied);
    assert!(e.to_string().contains("not writable"), "{e}");
    assert_eq!(std::fs::read(&p).unwrap(), ORIGINAL);
    std::fs::set_permissions(&d.0, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_no_temp(&d.0);
}

#[cfg(unix)]
#[test]
fn keeps_the_original_file_mode() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("mode");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();
    atomic_write(&p, b"new").unwrap();
    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
}

#[cfg(unix)]
#[test]
fn new_file_follows_the_process_umask() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("umask");
    let probe = d.0.join("probe.psd");
    std::fs::File::create(&probe).unwrap();
    let expected = std::fs::metadata(&probe).unwrap().permissions().mode() & 0o777;
    let p = d.0.join("fresh.psd");
    atomic_write(&p, b"new").unwrap();
    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, expected);
}

#[cfg(unix)]
#[test]
fn writes_through_a_symlink() {
    let d = TempDir::new("link");
    let real = d.0.join("real.psd");
    let link = d.0.join("link.psd");
    std::fs::write(&real, ORIGINAL).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    atomic_write(&link, b"new").unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read(&real).unwrap(), b"new");
}

#[cfg(unix)]
#[test]
fn no_follow_replaces_a_symlink_and_leaves_its_target() {
    let d = TempDir::new("nofollow");
    let real = d.0.join("real.psd");
    let link = d.0.join("link.psd");
    std::fs::write(&real, ORIGINAL).unwrap();
    std::os::unix::fs::symlink(&real, &link).unwrap();
    crate::atomic::atomic_write_no_follow(&link, b"new").unwrap();
    assert!(!std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
    assert_eq!(std::fs::read(&link).unwrap(), b"new");
    assert_eq!(std::fs::read(&real).unwrap(), ORIGINAL);
}

/// Replacing a link that points at a FIFO with no writer must not open the FIFO to copy its attributes.
#[cfg(unix)]
#[test]
fn no_follow_never_opens_the_target_of_a_replaced_link() {
    let d = TempDir::new("nofollow-fifo");
    let fifo = d.0.join("pipe");
    let link = d.0.join("link.psd");
    let Ok(made) = std::process::Command::new("mkfifo").arg(&fifo).status() else { return };
    if !made.success() {
        return;
    }
    std::os::unix::fs::symlink(&fifo, &link).unwrap();
    // Run apart, so a regression fails the test instead of hanging it.
    let (tx, rx) = std::sync::mpsc::channel();
    let path = link.clone();
    std::thread::spawn(move || {
        let _ = tx.send(crate::atomic::atomic_write_no_follow(&path, b"new"));
    });
    let r = rx.recv_timeout(std::time::Duration::from_secs(20)).expect("the write does not block on the FIFO");
    r.unwrap();
    assert_eq!(std::fs::read(&link).unwrap(), b"new");
}

/// #6605: saving onto a dangling symlink writes through to its target instead of replacing the
/// link with a regular file, even when a folder on the way to the target doesn't exist yet.
#[cfg(unix)]
#[test]
fn writes_through_a_dangling_symlink() {
    let d = TempDir::new("dangling");
    let real = d.0.join("missing/dir/a.psd");
    let link = d.0.join("link.psd");
    std::os::unix::fs::symlink(&real, &link).unwrap();
    atomic_write(&link, b"new").unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link survives");
    assert_eq!(std::fs::read(&real).unwrap(), b"new", "the bytes land at the target");
    assert_no_temp(real.parent().unwrap_or(Path::new(".")));
}

/// A dangling link with a relative target resolves against the link's own folder.
#[cfg(unix)]
#[test]
fn writes_through_a_dangling_relative_symlink() {
    let d = TempDir::new("dangling-rel");
    let here = d.0.join("here");
    std::fs::create_dir(&here).unwrap();
    let link = here.join("link.psd");
    std::os::unix::fs::symlink("../elsewhere/real.psd", &link).unwrap();
    atomic_write(&link, b"new").unwrap();
    assert!(std::fs::symlink_metadata(&link).unwrap().file_type().is_symlink(), "the link survives");
    assert_eq!(std::fs::read(d.0.join("elsewhere/real.psd")).unwrap(), b"new");
    assert_no_temp(&d.0.join("elsewhere"));
}

/// A symlink loop is an error; it never replaces either link.
#[cfg(unix)]
#[test]
fn symlink_loop_errors_without_replacing_the_links() {
    let d = TempDir::new("loop");
    let a = d.0.join("a.psd");
    let b = d.0.join("b.psd");
    std::os::unix::fs::symlink(&b, &a).unwrap();
    std::os::unix::fs::symlink(&a, &b).unwrap();
    let e = atomic_write(&a, b"new").unwrap_err();
    assert!(e.to_string().contains("symbolic links"), "{e}");
    assert!(std::fs::symlink_metadata(&a).unwrap().file_type().is_symlink());
    assert!(std::fs::symlink_metadata(&b).unwrap().file_type().is_symlink());
    assert_no_temp(&d.0);
}

#[test]
fn directory_target_or_bare_root_is_an_error_not_a_panic() {
    let d = TempDir::new("dir");
    assert!(atomic_write(&d.0, b"x").is_err());
    assert!(atomic_write(Path::new("/"), b"x").is_err());
    assert!(atomic_write(Path::new(""), b"x").is_err());
}

/// A share that refuses the full flush (`F_FULLFSYNC`, #1336) still saves through a plain fsync on
/// Apple platforms; every other sync error still fails the save.
#[test]
fn a_refused_full_flush_falls_back_to_a_plain_fsync_on_apple_only() {
    let refused = |errno| Err(io::Error::from_raw_os_error(errno));
    for (errno, name) in [(45, "ENOTSUP, SMB"), (25, "ENOTTY")] {
        let fsynced = Cell::new(false);
        let r = or_plain_fsync(refused(errno), || {
            fsynced.set(true);
            Ok(())
        });
        let apple = cfg!(target_vendor = "apple");
        assert_eq!(r.is_ok(), apple, "{name}");
        assert_eq!(fsynced.get(), apple, "{name}: the plain fsync runs only on Apple platforms");
    }
    // The plain fsync's own failure is reported.
    if cfg!(target_vendor = "apple") {
        let r = or_plain_fsync(refused(45), || Err(io::Error::from_raw_os_error(5)));
        assert_eq!(r.map_err(|e| e.raw_os_error()), Err(Some(5)));
    }
    // Real errors (EIO, ENOSPC, EACCES) never fall back, and success needs no fallback.
    for errno in [5, 28, 13] {
        let r = or_plain_fsync(refused(errno), || panic!("no fallback for errno {errno}"));
        assert_eq!(r.map_err(|e| e.raw_os_error()), Err(Some(errno)));
    }
    assert!(or_plain_fsync(Ok(()), || panic!("no fallback after a full flush")).is_ok());
}

#[test]
fn sync_file_flushes_a_local_file() {
    let dir = TempDir::new("sync");
    let path = dir.0.join("f.bin");
    // Opened for writing, as the saves do: Windows can't flush a read-only handle.
    let file = std::fs::OpenOptions::new().write(true).create_new(true).open(&path).unwrap();
    sync_file(&file).unwrap();
}

/// #4968: the rename succeeded but the directory flush failed. The new bytes are in place, and
/// the save is reported as unconfirmed (same kind, saying so) instead of as a success, so the
/// caller keeps the document unsaved and its recovery data.
#[test]
fn a_failed_directory_flush_is_reported_not_swallowed() {
    let d = TempDir::new("syncdir");
    let p = d.0.join("doc.pcraft");
    std::fs::write(&p, ORIGINAL).unwrap();
    let fs = Faulty { fail_sync_dir: Some(Err(io::ErrorKind::Other)), ..Default::default() };
    let e = atomic_write_with(&fs, NO_RETRY, &p, b"new").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::Other);
    let msg = e.to_string();
    assert!(msg.contains("doc.pcraft") && msg.contains("may not survive"), "{msg}");
    assert_eq!(std::fs::read(&p).unwrap(), b"new", "published: the new bytes are in place");
    assert_no_temp(&d.0);
}

/// A file system that can't flush directories at all (Linux's EINVAL, ENOTSUP, or an
/// `Unsupported` error) still saves: failing every save there would make saving impossible.
/// Windows fails the save on any failed flush (#7858), so this is the Unix rule.
#[cfg(not(windows))]
#[test]
fn a_directory_flush_the_file_system_cannot_do_still_saves() {
    let d = TempDir::new("syncdir-unsupported");
    let p = d.0.join("doc.pcraft");
    let fs = Faulty { fail_sync_dir: Some(Err(io::ErrorKind::Unsupported)), ..Default::default() };
    atomic_write_with(&fs, NO_RETRY, &p, b"new").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    // EINVAL (22) means "can't flush a directory" only on Unix; EIO (5) is a real failure.
    assert_eq!(dir_synced(Err(io::Error::from_raw_os_error(22))).is_ok(), cfg!(unix));
    assert!(dir_synced(Err(io::Error::from_raw_os_error(5))).is_err());
    let fs = Faulty { fail_sync_dir: Some(Ok(5)), ..Default::default() };
    assert!(atomic_write_with(&fs, NO_RETRY, &p, b"newer").is_err());
}

/// #9440: network shares (SMB) refuse a directory flush with `ERROR_INVALID_FUNCTION` (1) or
/// `ERROR_NOT_SUPPORTED` (50) on Windows; those count as flushed. Real failures still don't.
#[test]
fn windows_unsupported_directory_flush_codes_are_classified() {
    let code = |c| dir_synced(Err(io::Error::from_raw_os_error(c)));
    assert!(dir_synced(Err(io::Error::from(io::ErrorKind::Unsupported))).is_ok());
    // Off Windows these are errno values (1 is EPERM): a real failure, never swallowed.
    assert_eq!(code(1).is_ok(), cfg!(windows), "code 1 counts as flushed only on Windows");
    assert_eq!(code(50).is_ok(), cfg!(windows), "code 50 counts as flushed only on Windows");
    if cfg!(windows) {
        assert!(code(5).is_err(), "ERROR_ACCESS_DENIED stays a failure");
        assert!(code(1117).is_err(), "ERROR_IO_DEVICE stays a failure");
    }
    assert!(dir_synced(Err(io::Error::from(io::ErrorKind::Other))).is_err());
}

/// #9440: the Windows mirror of `a_directory_flush_the_file_system_cannot_do_still_saves`. A share
/// refusing the directory flush with `ERROR_INVALID_FUNCTION` (1) or `ERROR_NOT_SUPPORTED` (50)
/// still saves through `atomic_write_with`; `ERROR_ACCESS_DENIED` (5) still fails the save.
#[cfg(windows)]
#[test]
fn a_share_that_cannot_flush_a_directory_still_saves() {
    let d = TempDir::new("syncdir-smb");
    let p = d.0.join("doc.pcraft");
    for code in [1, 50] {
        let fs = Faulty { fail_sync_dir: Some(Ok(code)), ..Default::default() };
        atomic_write_with(&fs, NO_RETRY, &p, b"new").unwrap();
        assert_eq!(fs.synced.get(), 1, "code {code}: the flush was attempted");
        assert_eq!(std::fs::read(&p).unwrap(), b"new");
    }
    let fs = Faulty { fail_sync_dir: Some(Ok(5)), ..Default::default() };
    assert!(atomic_write_with(&fs, NO_RETRY, &p, b"newer").is_err());
    assert_no_temp(&d.0);
}

/// #9388: on macOS a save keeps the replaced file's creation time ("Created" in Finder), as
/// Photoshop's does; the rename would otherwise leave a file born at the time of the save.
#[cfg(target_os = "macos")]
#[test]
fn keeps_the_original_creation_time() {
    use std::os::macos::fs::FileTimesExt;
    let d = TempDir::new("created");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    // A creation time well in the past, so a reset to "now" can't pass for it.
    let born = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_times(std::fs::FileTimes::new().set_created(born)).unwrap();
    assert_eq!(std::fs::metadata(&p).unwrap().created().unwrap(), born);
    atomic_write(&p, b"new").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    assert_eq!(std::fs::metadata(&p).unwrap().created().unwrap(), born, "the creation time survives the save");
    assert_no_temp(&d.0);
}

/// #9401 review: a replaced file whose ACL denies its owner writing still keeps its creation
/// time. The ACL lands on the temporary file too, so the time has to be set before it is.
#[cfg(target_os = "macos")]
#[test]
fn keeps_the_creation_time_under_a_deny_write_acl() {
    use std::os::macos::fs::FileTimesExt;
    let d = TempDir::new("created-acl");
    let p = d.0.join("doc.psd");
    std::fs::write(&p, ORIGINAL).unwrap();
    let born = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1_600_000_000);
    std::fs::OpenOptions::new().write(true).open(&p).unwrap().set_times(std::fs::FileTimes::new().set_created(born)).unwrap();
    let Ok(user) = std::env::var("USER") else { return };
    let deny = exacl::AclEntry::deny_user(&user, exacl::Perm::WRITE, None);
    if exacl::setfacl(&[&p], &[deny], None).is_err() {
        return; // a file system without ACLs: nothing to assert
    }
    atomic_write(&p, b"new").unwrap();
    assert_eq!(std::fs::read(&p).unwrap(), b"new");
    assert_eq!(std::fs::metadata(&p).unwrap().created().unwrap(), born, "the creation time survives a deny-write ACL");
    let _ = exacl::setfacl(&[&p], &[], None);
    assert_no_temp(&d.0);
}

/// A real file system that records the name of every temporary file a save writes.
#[derive(Default)]
struct TempNames(std::cell::RefCell<Vec<String>>);

impl AtomicFs for TempNames {
    fn write_new(&self, tmp: &Path, bytes: &[u8]) -> io::Result<()> {
        self.0.borrow_mut().push(tmp.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        RealFs.write_new(tmp, bytes)
    }
    fn rename(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.rename(from, to)
    }
    fn remove(&self, path: &Path) -> io::Result<()> {
        RealFs.remove(path)
    }
    fn sync_dir(&self, dir: &Path) -> io::Result<()> {
        RealFs.sync_dir(dir)
    }
    fn copy_metadata(&self, from: &Path, to: &Path) -> io::Result<()> {
        RealFs.copy_metadata(from, to)
    }
}

/// #11624: the temporary file a save renames over the document has no leading dot. An SMB share
/// that hides dot files marks such a file hidden when it is created, and the mark stayed on the
/// saved document, so it didn't show in Finder.
#[test]
fn the_temporary_file_is_not_a_dot_file() {
    let d = TempDir::new("visible");
    let p = d.0.join("report.psd");
    let fs = TempNames::default();
    atomic_write_with(&fs, NO_RETRY, &p, b"first").unwrap();
    atomic_write_with(&fs, NO_RETRY, &p, b"second").unwrap();
    let names = fs.0.take();
    assert_eq!(names.len(), 2, "{names:?}");
    for name in &names {
        assert!(!name.starts_with('.'), "a dot file is hidden on some shares: {name}");
        assert!(name.starts_with("report.psd.") && name.ends_with(".pcsave.tmp"), "{name}");
        assert_eq!(temp_owner(name), Some(std::process::id()), "{name}");
    }
    assert_ne!(names[0], names[1]);
    assert_eq!(std::fs::read(&p).unwrap(), b"second");
    assert_no_temp(&d.0);
}

/// #11624: both temp name forms are recognized, so the leftovers of versions that wrote
/// `.{file}.{pid}-{n}.pcsave.tmp` are still cleaned up; nothing else is.
#[test]
fn temp_owner_reads_both_name_forms_and_nothing_else() {
    assert_eq!(temp_owner(&temp_name("doc.pcraft")), Some(std::process::id()));
    assert_eq!(temp_owner("doc.pcraft.42-7.pcsave.tmp"), Some(42));
    assert_eq!(temp_owner(".doc.pcraft.42-7.pcsave.tmp"), Some(42), "the dot-prefixed form older versions wrote");
    assert_eq!(temp_owner(".hidden.psd.42-0.pcsave.tmp"), Some(42), "a dot file's own temp file");
    assert_eq!(temp_owner("doc.42-18446744073709551615.pcsave.tmp"), Some(42), "the largest counter");
    assert_eq!(temp_owner("doc.0-0.pcsave.tmp"), Some(0), "a lone zero is a number temp_name writes");
    for other in [
        "doc.pcraft",
        "doc.pcraft.42-7.tmp",
        "doc.42-7.pcsave",
        ".42-7.pcsave.tmp",
        "doc.x-7.pcsave.tmp",
        "doc.+42-7.pcsave.tmp",
        "doc.42-.pcsave.tmp",
        "doc.42-x.pcsave.tmp",
        "doc.42-18446744073709551616.pcsave.tmp",
        "doc.4294967296-7.pcsave.tmp",
        "doc.42-07.pcsave.tmp",
        "doc.42-00.pcsave.tmp",
        "doc.042-7.pcsave.tmp",
        "doc.00-7.pcsave.tmp",
        "doc.42.pcsave.tmp",
        "42-7.pcsave.tmp",
    ] {
        assert_eq!(temp_owner(other), None, "{other}");
    }
}

/// A recovery folder can hold someone else's file that only looks like a temp name. `temp_name`
/// keeps at most 64 characters (not bytes) of the file name, and older versions put one dot in
/// front of them, so a longer file part is not a leftover of a save and the sweep must keep it.
#[test]
fn temp_owner_takes_only_a_file_part_temp_name_writes() {
    let pid = Some(std::process::id());
    for file in ["a".repeat(200), "🚀".repeat(70), format!(".{}", "a".repeat(100))] {
        assert_eq!(temp_owner(&temp_name(&file)), pid, "{file}");
    }
    let today = |stem: &str| format!("{stem}.42-7.pcsave.tmp");
    let older = |stem: &str| format!(".{stem}.42-7.pcsave.tmp");
    for c in ["a", "é", "🚀"] {
        let (at, over) = (c.repeat(64), c.repeat(65));
        assert_eq!(temp_owner(&today(&at)), Some(42), "64 characters: {c}");
        assert_eq!(temp_owner(&older(&at)), Some(42), "64 characters after the old dot: {c}");
        assert_eq!(temp_owner(&today(&over)), None, "65 characters: {c}");
        assert_eq!(temp_owner(&older(&over)), None, "65 characters after the old dot: {c}");
        // A dot file's own dot counts in its 64 characters; the old form adds one more dot.
        let dotted = |k: usize| format!(".{}", c.repeat(k));
        assert_eq!(temp_owner(&today(&dotted(63))), Some(42), "a dot file today: {c}");
        assert_eq!(temp_owner(&older(&dotted(63))), Some(42), "a dot file in the old form: {c}");
        assert_eq!(temp_owner(&older(&dotted(64))), None, "a dot file one character over: {c}");
    }
}
