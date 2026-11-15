//! Exclusive publication and rename for files in one directory.
//!
//! Prefer a native exclusive rename, with atomic hard-link publication when that operation is
//! unavailable. The caller handles failures between separate file operations. Never replace
//! an exclusive operation with an existence check followed by an overwriting rename.

use std::{
    ffi::OsStr,
    fs::{File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

/// Publication failure distinguishes an unchanged destination from a published file whose cleanup or directory flush failed.
///
/// With `published: false` the destination is unchanged (for example `AlreadyExists` when another
/// file got there first), so the caller may pick another name or retry. With `published: true` the
/// destination already holds the complete, flushed bytes; only cleanup or the directory flush
/// failed. Don't retry that as unpublished: a retry would fail with `AlreadyExists` against the
/// caller's own file. Report the write as done but not confirmed durable instead.
#[derive(Debug)]
pub struct PublishNewError {
    /// Whether the destination name now exists with the complete bytes despite the error.
    pub published: bool,
    /// The failed operation: staging, writing or the rename when `published` is false; source
    /// cleanup or the directory flush when it is true.
    pub error: io::Error,
}

impl std::fmt::Display for PublishNewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.published { write!(f, "file published; completion unconfirmed: {}", self.error) } else { std::fmt::Display::fmt(&self.error, f) }
    }
}
impl std::error::Error for PublishNewError {}

/// Publish a complete, flushed file only if its destination is still absent.
/// Uses an exclusive staging file beside the destination. The caller must use a stable parent
/// directory where untrusted writers cannot replace the staging entry.
/// On Windows, non-Unicode paths require a filesystem that supports hard links.
pub fn write_noreplace(path: &Path, bytes: &[u8]) -> Result<(), PublishNewError> {
    use crate::atomic::{AtomicFs, RealFs};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let before = |error| PublishNewError { published: false, error };
    let dir = path.parent().ok_or_else(|| before(io::Error::new(io::ErrorKind::InvalidInput, "missing publication directory")))?;
    // A bare filename still needs a real directory path for the final flush.
    let dir = if dir.as_os_str().is_empty() { Path::new(".") } else { dir };
    let name = path.file_name().ok_or_else(|| before(io::Error::new(io::ErrorKind::InvalidInput, "missing publication filename")))?;
    let (temp, mut file) = create_stage(dir, name, &NEXT).map_err(before)?;
    let written = file.write_all(bytes).and_then(|_| crate::atomic::sync_file(&file));
    drop(file);
    let published = written
        .map_err(before)
        .and_then(|()| move_noreplace(&temp, path, platform, |from, to| std::fs::hard_link(from, to), |path| std::fs::remove_file(path)));
    let published = published.map_err(|failure| retry_stage_cleanup(failure, &temp, |path| std::fs::remove_file(path)));
    finish_publication(published, || RealFs.sync_dir(dir))
}

/// Retry removing the staging name after a failure; if that fails too, name the leftover
/// staging file in the error so the caller can report or remove it.
fn retry_stage_cleanup(failure: PublishNewError, temp: &Path, remove: impl FnOnce(&Path) -> io::Result<()>) -> PublishNewError {
    match remove(temp) {
        Err(cleanup) if cleanup.kind() != io::ErrorKind::NotFound => PublishNewError {
            published: failure.published,
            error: io::Error::new(failure.error.kind(), format!("{}; staging file {} remains", failure.error, temp.display())),
        },
        _ => failure,
    }
}

/// Create a staging entry whose name cannot be the destination's native filename.
fn create_stage(dir: &Path, destination: &OsStr, next: &AtomicU64) -> io::Result<(PathBuf, File)> {
    // Ignore a Windows stream suffix. A different last ASCII digit stays distinct across
    // case folding and trailing-dot/space normalization. An uppercase 8.3 staging name
    // already satisfies DOS short-name rules, so it needs no second name.
    let name = destination.as_encoded_bytes().split(|&byte| byte == b':').next().unwrap_or_default();
    let extension = if name.iter().rev().find(|byte| byte.is_ascii_digit()) == Some(&b'1') { "TM2" } else { "TM1" };
    for _ in 0..32 {
        let sequence = next.fetch_add(1, Ordering::Relaxed);
        let nonce = (u64::from(std::process::id()) << 8).wrapping_add(sequence) & 0xff_ffff;
        let temp = dir.join(format!("PC{nonce:06X}.{extension}"));
        match OpenOptions::new().write(true).create_new(true).open(&temp) {
            Ok(file) => return Ok((temp, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    // Not `AlreadyExists`: that means the destination is taken, and callers may pick another name.
    Err(io::Error::other("no unused publication temp name"))
}

/// Rename a file within one stable directory, refusing an existing destination atomically.
/// Directories are not supported: the hard-link fallback cannot link them.
/// The caller must ensure untrusted writers cannot replace the source entry.
/// Unsupported rename primitives use a hard link followed by unlink, never an overwriting
/// rename or cross-volume copy. If unlink fails, both names retain the same bytes and this
/// returns an error, so the caller reports retirement as incomplete.
/// The caller must flush the directory before reporting crash-durable migration completion;
/// the existing `atomic::AtomicFs::sync_dir` provides that separately fallible operation.
pub fn rename_noreplace(from: &Path, to: &Path) -> io::Result<()> {
    if from == to || from.parent().is_none() || from.parent() != to.parent() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no-replace retirement requires distinct names in one directory"));
    }
    move_noreplace(from, to, platform, |from, to| std::fs::hard_link(from, to), |path| std::fs::remove_file(path)).map_err(|failure| failure.error)
}

/// Use fallback only when the exclusive rename primitive itself is unsupported.
fn unsupported_rename(error: &io::Error) -> bool {
    #[cfg(unix)]
    {
        error.kind() == io::ErrorKind::Unsupported
            || matches!(error.raw_os_error(), Some(code) if code == rustix::io::Errno::INVAL.raw_os_error() || code == rustix::io::Errno::NOSYS.raw_os_error())
    }
    #[cfg(not(unix))]
    {
        error.kind() == io::ErrorKind::Unsupported
    }
}

/// Linking claims an absent destination atomically; a later unlink failure is already published.
fn move_noreplace(
    from: &Path,
    to: &Path,
    rename: impl FnOnce(&Path, &Path) -> io::Result<()>,
    link: impl FnOnce(&Path, &Path) -> io::Result<()>,
    remove: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), PublishNewError> {
    match rename(from, to) {
        Ok(()) => Ok(()),
        Err(error) if unsupported_rename(&error) => {
            link(from, to).map_err(|error| PublishNewError { published: false, error })?;
            remove(from).map_err(|error| PublishNewError { published: true, error })
        }
        Err(error) => Err(PublishNewError { published: false, error }),
    }
}

/// A successful name change is durable only after the separately fallible directory flush.
fn finish_publication(result: Result<(), PublishNewError>, sync: impl FnOnce() -> io::Result<()>) -> Result<(), PublishNewError> {
    match result {
        Ok(()) => sync().map_err(|error| PublishNewError { published: true, error }),
        Err(failure) => {
            if failure.published {
                let _ = sync();
            }
            Err(failure)
        }
    }
}

/// Prefer the native exclusive rename when the Windows wrapper can represent both paths.
#[cfg(windows)]
fn platform(from: &Path, to: &Path) -> io::Result<()> {
    let (Some(from), Some(to)) = (from.to_str(), to.to_str()) else {
        // The existing hard-link fallback takes native paths, including unpaired surrogates.
        return Err(io::ErrorKind::Unsupported.into());
    };
    if from.contains('\0') || to.contains('\0') {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "file path contains a NUL character"));
    }
    // MoveFile (unlike MoveFileEx with REPLACE_EXISTING) fails if the destination exists.
    // The same-directory restriction above also excludes its cross-volume copy behavior.
    winsafe::MoveFile(from, to).map_err(|error| io::Error::from_raw_os_error(error.raw() as i32))
}

#[cfg(all(unix, any(target_os = "linux", target_os = "android", target_os = "redox", target_vendor = "apple")))]
fn platform(from: &Path, to: &Path) -> io::Result<()> {
    // rustix maps NOREPLACE to Linux renameat2 or Apple's RENAME_EXCL, without unsafe FFI here.
    rustix::fs::renameat_with(rustix::fs::CWD, from, rustix::fs::CWD, to, rustix::fs::RenameFlags::NOREPLACE).map_err(io::Error::from)
}

#[cfg(not(any(windows, all(unix, any(target_os = "linux", target_os = "android", target_os = "redox", target_vendor = "apple")))))]
fn platform(_from: &Path, _to: &Path) -> io::Result<()> {
    Err(io::Error::new(io::ErrorKind::Unsupported, "this platform has no supported no-replace retirement primitive"))
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Dir {
        fn new() -> io::Result<Self> {
            static NEXT: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!("pc-noreplace-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
            // Never clean a preexisting directory to make the fixture pass.
            std::fs::create_dir(&path)?;
            Ok(Self(path))
        }
    }
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn retirement_moves_the_original_bytes_without_overwriting_a_backup() {
        let dir = Dir::new().unwrap();
        let from = dir.0.join("legacy");
        let to = dir.0.join("retained");
        std::fs::write(&from, b"exact original bytes").unwrap();
        std::fs::write(&to, b"existing backup").unwrap();
        assert_eq!(rename_noreplace(&from, &to).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&from).unwrap(), b"exact original bytes");
        assert_eq!(std::fs::read(&to).unwrap(), b"existing backup");
        std::fs::remove_file(&to).unwrap();
        rename_noreplace(&from, &to).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"exact original bytes");
    }

    #[test]
    fn retirement_refuses_same_name_and_missing_source_without_changing_existing_data() {
        let dir = Dir::new().unwrap();
        let from = dir.0.join("legacy");
        let to = dir.0.join("retained");
        std::fs::write(&to, b"existing backup").unwrap();
        assert!(rename_noreplace(&from, &to).is_err());
        assert_eq!(std::fs::read(&to).unwrap(), b"existing backup");
        assert_eq!(rename_noreplace(&to, &to).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert_eq!(std::fs::read(&to).unwrap(), b"existing backup");
    }

    #[test]
    fn publication_keeps_an_existing_destination_and_leaves_no_owned_temp() {
        let dir = Dir::new().unwrap();
        let path = dir.0.join("actions.atn");
        std::fs::write(&path, b"existing ATN").unwrap();
        let error = write_noreplace(&path, b"replacement").unwrap_err();
        assert!(!error.published);
        assert_eq!(std::fs::read(&path).unwrap(), b"existing ATN");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    #[test]
    fn publication_creates_complete_bytes_then_removes_its_temp_name() {
        let dir = Dir::new().unwrap();
        let path = dir.0.join("actions.atn");
        write_noreplace(&path, b"complete ATN").unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"complete ATN");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    /// Even the first generated candidate cannot create the destination before publication.
    #[test]
    fn staging_does_not_create_a_destination_alias() {
        let dir = Dir::new().unwrap();
        let (first, file) = create_stage(&dir.0, OsStr::new("actions.atn"), &AtomicU64::new(0)).unwrap();
        drop(file);
        std::fs::remove_file(&first).unwrap();
        let name = first.file_name().unwrap().to_str().unwrap();
        for alias in [name.to_owned(), name.to_ascii_lowercase(), format!("{name}. "), format!("{name}::$DATA")] {
            let (temp, file) = create_stage(&dir.0, OsStr::new(&alias), &AtomicU64::new(0)).unwrap();
            assert_eq!(temp.extension(), Some(OsStr::new("TM2")), "{alias}");
            assert!(!first.try_exists().unwrap());
            drop(file);
            std::fs::remove_file(temp).unwrap();
        }
    }

    /// The native-path fallback preserves both byte content and no-clobber behavior.
    #[cfg(windows)]
    #[test]
    fn publication_and_retirement_accept_native_windows_paths() {
        use std::{ffi::OsString, os::windows::ffi::OsStringExt};

        let dir = Dir::new().unwrap();
        let path = |suffix: &str| {
            let mut name = OsString::from_wide(&[0xd800]);
            name.push(suffix);
            assert!(name.to_str().is_none());
            dir.0.join(name)
        };
        let from = path("-legacy");
        let to = path("-retained");
        write_noreplace(&from, b"original").unwrap();
        let error = write_noreplace(&from, b"other").unwrap_err();
        assert!(!error.published);
        assert_eq!(error.error.kind(), io::ErrorKind::AlreadyExists);
        std::fs::write(&to, b"existing").unwrap();
        assert_eq!(rename_noreplace(&from, &to).unwrap_err().kind(), io::ErrorKind::AlreadyExists);
        assert_eq!(std::fs::read(&from).unwrap(), b"original");
        assert_eq!(std::fs::read(&to).unwrap(), b"existing");
        std::fs::remove_file(&to).unwrap();
        rename_noreplace(&from, &to).unwrap();
        assert!(!from.try_exists().unwrap());
        assert_eq!(std::fs::read(&to).unwrap(), b"original");
        assert_eq!(std::fs::read_dir(&dir.0).unwrap().count(), 1);
    }

    /// An existing destination survives both the fallback and the native primitive unchanged.
    #[test]
    fn fallback_is_exclusive_and_moves_complete_bytes() {
        let dir = Dir::new().unwrap();
        let (from, to) = (dir.0.join("legacy"), dir.0.join("backup"));
        std::fs::write(&from, b"original").unwrap();
        std::fs::write(&to, b"existing").unwrap();
        let unsupported = |_: &Path, _: &Path| Err(io::Error::from(io::ErrorKind::Unsupported));
        let failed = move_noreplace(&from, &to, unsupported, |from, to| std::fs::hard_link(from, to), |path| std::fs::remove_file(path)).unwrap_err();
        assert!(!failed.published);
        assert_eq!(std::fs::read(&to).unwrap(), b"existing");
        assert_eq!(std::fs::read(&from).unwrap(), b"original");
        std::fs::remove_file(&to).unwrap();
        move_noreplace(&from, &to, unsupported, |from, to| std::fs::hard_link(from, to), |path| std::fs::remove_file(path)).unwrap();
        assert!(!from.exists());
        assert_eq!(std::fs::read(&to).unwrap(), b"original");
    }

    /// A published destination cannot be called unchanged when source cleanup or fsync fails.
    #[test]
    fn fallback_unlink_and_directory_flush_failures_report_publication() {
        let dir = Dir::new().unwrap();
        let (from, to) = (dir.0.join("legacy"), dir.0.join("backup"));
        std::fs::write(&from, b"original").unwrap();
        let error = move_noreplace(
            &from,
            &to,
            |_, _| Err(io::ErrorKind::Unsupported.into()),
            |from, to| std::fs::hard_link(from, to),
            |_| Err(io::ErrorKind::PermissionDenied.into()),
        )
        .unwrap_err();
        assert!(error.published);
        assert_eq!(std::fs::read(&from).unwrap(), b"original");
        assert_eq!(std::fs::read(&to).unwrap(), b"original");
        let error = finish_publication(Err(error), || Ok(())).unwrap_err();
        assert!(error.published);
        assert_eq!(error.error.kind(), io::ErrorKind::PermissionDenied);
        let error = finish_publication(Ok(()), || Err(io::ErrorKind::Other.into())).unwrap_err();
        assert!(error.published);
        assert_eq!(std::fs::read(&to).unwrap(), b"original");
    }

    /// Running out of staging names must not look like a taken destination.
    #[test]
    fn staging_exhaustion_is_not_reported_as_an_existing_destination() {
        let dir = Dir::new().unwrap();
        let next = AtomicU64::new(0);
        for _ in 0..32 {
            drop(create_stage(&dir.0, OsStr::new("actions.atn"), &next).unwrap().1);
        }
        let error = create_stage(&dir.0, OsStr::new("actions.atn"), &AtomicU64::new(0)).unwrap_err();
        assert_ne!(error.kind(), io::ErrorKind::AlreadyExists);
    }

    /// A staging name that cannot be removed is named in the error, keeping its kind and state.
    #[test]
    fn failed_staging_cleanup_names_the_leftover_file() {
        let temp = Path::new("PC000001.TM1");
        let failure = || PublishNewError { published: true, error: io::ErrorKind::PermissionDenied.into() };
        let error = retry_stage_cleanup(failure(), temp, |_| Err(io::ErrorKind::PermissionDenied.into()));
        assert!(error.published);
        assert_eq!(error.error.kind(), io::ErrorKind::PermissionDenied);
        assert!(error.error.to_string().contains("PC000001.TM1"), "{}", error.error);
        let error = retry_stage_cleanup(failure(), temp, |_| Ok(()));
        assert!(!error.error.to_string().contains("PC000001.TM1"));
        let error = retry_stage_cleanup(failure(), temp, |_| Err(io::ErrorKind::NotFound.into()));
        assert!(!error.error.to_string().contains("PC000001.TM1"));
    }

    /// Ordinary rename failures must not trigger a second filesystem mutation route.
    #[test]
    fn ordinary_rename_errors_do_not_attempt_fallback() {
        for kind in [io::ErrorKind::AlreadyExists, io::ErrorKind::PermissionDenied, io::ErrorKind::NotFound] {
            let result = move_noreplace(
                Path::new("from"),
                Path::new("to"),
                |_, _| Err(kind.into()),
                |_, _| panic!("ordinary failure must not link"),
                |_| panic!("ordinary failure must not unlink"),
            );
            let error = result.unwrap_err();
            assert!(!error.published);
            assert_eq!(error.error.kind(), kind);
        }
    }
}
