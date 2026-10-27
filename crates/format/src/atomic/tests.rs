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
