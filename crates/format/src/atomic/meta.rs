//! What a replacement keeps from the file it replaces.
//!
//! Saving writes a new file and renames it over the old one, so the user gets a file with the
//! right bytes and none of the old file's history: everything the operating system and other
//! programs had attached to the file that was just unlinked is gone. Some of it is a security
//! decision — *this came from the internet*, *only these people may read it* — so a save that
//! drops it quietly weakens the user's protections (#6603, #7073, #7094, #7095, #7096).
//!
//! | | macOS | Linux | Windows |
//! |---|---|---|---|
//! | permission bits | [`copy_metadata`] | [`copy_metadata`] | [`copy_metadata`] (read-only flag) |
//! | extended attributes (Finder tags, `user.*`) | `copy_xattrs` | `copy_xattrs` | — |
//! | "downloaded from the internet" | `com.apple.quarantine`, an xattr | — | `Zone.Identifier`, a named stream kept by [`replace_existing`] |
//! | access-control list | `copy_acl` (`exacl`) | not carried over (see below) | DACL, kept by [`replace_existing`] |
//! | creation time | `copy_created` (#9388) | — | kept by [`replace_existing`] |
//!
//! Everything here except [`replace_existing`] is **best effort and never touches the file's
//! bytes**: a save that cannot carry a piece of metadata across is still a save, because a
//! document the user can open matters more than a Finder tag. [`replace_existing`] *is* the
//! replacement step, so it holds the atomicity guarantee instead: it never leaves the
//! destination missing, and it falls back to a plain rename rather than fail a save.
//!
//! Not carried over, and why: Linux POSIX ACLs live in `system.posix_acl_access`, which
//! `listxattr` does not report, so copying the listed attributes misses them; they would need
//! their own step.

use std::io;
use std::path::Path;

#[cfg(unix)]
use std::os::fd::BorrowedFd;

/// Largest attribute-name list [`copy_xattrs`] will read. Names are short; a file with a
/// megabyte of them is not something a save should spend memory on.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
const MAX_XATTR_NAMES: usize = 1 << 20;

/// Largest single attribute value [`copy_xattrs`] will carry over. Finder tags and `user.*`
/// attributes are tens of bytes; a resource fork can be large, and past this it is not worth
/// holding a copy of it in memory during a save.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
const MAX_XATTR_VALUE: usize = 16 << 20;

/// First read size for an attribute list or value; grown on `ERANGE`.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
const XATTR_CHUNK: usize = 4 << 10;

/// Copy, best effort, everything [`crate::atomic::atomic_write`] should carry from the file it
/// is about to replace (`from`) onto the temporary file that will replace it (`to`).
///
/// Errors are the caller's to ignore: each piece is attempted on its own, and the first failure
/// is returned only so a caller that wants to log one can.
pub fn copy_metadata(from: &Path, to: &Path) -> io::Result<()> {
    // The permission bits go on last. Writing an attribute needs the destination open for
    // writing, and replacing a read-only (0444) file would copy that mode onto the temporary
    // file and then be unable to touch it again.
    #[cfg(unix)]
    let attributes = copy_xattrs_at(from, to);
    #[cfg(not(unix))]
    let attributes = Ok(());

    // The creation time before the ACL: it needs the destination open for writing, which a
    // copied ACL denying its owner write access would refuse.
    #[cfg(target_os = "macos")]
    let created = copy_created(from, to);
    #[cfg(not(target_os = "macos"))]
    let created = Ok(());

    #[cfg(target_os = "macos")]
    let acl = copy_acl(from, to);
    #[cfg(not(target_os = "macos"))]
    let acl = Ok(());

    // A save over a 0600 file must not widen it to whatever the process default is.
    let permissions = std::fs::metadata(from).and_then(|m| std::fs::set_permissions(to, m.permissions()));

    attributes.and(acl).and(created).and(permissions)
}

/// The creation time ("Created" in Finder) of the file being replaced, onto the file replacing
/// it: the rename leaves a new file born at the time of the save, where Photoshop keeps the
/// original's (#9388). The rename keeps the time set here.
#[cfg(target_os = "macos")]
fn copy_created(from: &Path, to: &Path) -> io::Result<()> {
    let old = std::fs::File::open(from)?;
    let new = std::fs::OpenOptions::new().write(true).open(to)?;
    copy_created_file(&old, &new)
}

/// [`copy_created`] through two open files rather than their paths, for the automation write
/// path's capability-checked handles (see [`copy_xattrs`]). `to` must be open for writing.
/// Nothing to do outside macOS: Windows keeps the time in [`replace_existing`], and Linux has no
/// settable creation time.
pub fn copy_created_file(from: &std::fs::File, to: &std::fs::File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    {
        use std::os::macos::fs::FileTimesExt;
        let created = from.metadata()?.created()?;
        to.set_times(std::fs::FileTimes::new().set_created(created))
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (from, to);
        Ok(())
    }
}

/// [`copy_metadata`]'s extended-attribute step, by path: open both files and hand the
/// descriptors to [`copy_xattrs`].
#[cfg(unix)]
fn copy_xattrs_at(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::fd::AsFd;
    let old = std::fs::File::open(from)?;
    // Writing an attribute needs write access to the file, not only to its directory.
    let new = std::fs::OpenOptions::new().write(true).open(to)?;
    copy_xattrs(old.as_fd(), new.as_fd())
}

/// Copy every extended attribute of `from` onto `to`, best effort, through the two open files
/// rather than their paths.
///
/// Descriptors, not paths, because the automation write path holds capability-checked handles
/// and must never reach back out to an ambient path to finish a write (#7095). The same reason
/// makes this the right shape for [`copy_metadata`] too.
///
/// An attribute that cannot be read or cannot be set is skipped: `com.apple.provenance` and
/// `com.apple.rootless` on macOS and `security.*` on Linux belong to the system and refuse to
/// be written by an ordinary process, and failing a document save over one would be absurd.
/// `Err` means the attribute *list* could not be read, so nothing was copied.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
pub fn copy_xattrs(from: BorrowedFd<'_>, to: BorrowedFd<'_>) -> io::Result<()> {
    use rustix::fs::{XattrFlags, fsetxattr};

    for name in list_xattrs(from)?.split(|b| *b == 0) {
        if name.is_empty() || skip_xattr(name) {
            continue;
        }
        let Some(value) = read_xattr(from, name) else { continue };
        let _ = fsetxattr(to, name, &value, XattrFlags::empty());
    }
    Ok(())
}

/// [`copy_xattrs`] where rustix offers no extended-attribute calls (FreeBSD and the other BSDs):
/// nothing is copied, as on Windows.
#[cfg(all(unix, not(any(target_os = "linux", target_os = "android", target_vendor = "apple"))))]
pub fn copy_xattrs(_from: BorrowedFd<'_>, _to: BorrowedFd<'_>) -> io::Result<()> {
    Ok(())
}

/// Attributes that describe how the *old* file's bytes are stored rather than anything about
/// the document, and so must never be pinned onto a file holding different bytes.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn skip_xattr(name: &[u8]) -> bool {
    // `com.apple.decmpfs` marks a file whose data lives compressed inside the attribute itself.
    // The kernel hides it from a plain `listxattr` and refuses to set it, but a file claiming
    // compressed storage it does not have would read as garbage, so never try.
    name == b"com.apple.decmpfs"
}

/// The NUL-separated attribute names of `fd`.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn list_xattrs(fd: BorrowedFd<'_>) -> io::Result<Vec<u8>> {
    let mut buf = vec![0u8; XATTR_CHUNK];
    loop {
        match rustix::fs::flistxattr(fd, buf.as_mut_slice()) {
            Ok(len) => {
                buf.truncate(len.min(buf.len()));
                return Ok(buf);
            }
            Err(e) if e == rustix::io::Errno::RANGE && buf.len() < MAX_XATTR_NAMES => {
                buf.resize(buf.len().saturating_mul(4).min(MAX_XATTR_NAMES), 0);
            }
            Err(e) => return Err(e.into()),
        }
    }
}

/// The value of one attribute, or `None` if it cannot be read or is larger than
/// [`MAX_XATTR_VALUE`]. A missing value is normal: another process may remove an attribute
/// between the listing and the read.
#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
fn read_xattr(fd: BorrowedFd<'_>, name: &[u8]) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; XATTR_CHUNK];
    loop {
        match rustix::fs::fgetxattr(fd, name, buf.as_mut_slice()) {
            Ok(len) => {
                buf.truncate(len.min(buf.len()));
                return Some(buf);
            }
            Err(e) if e == rustix::io::Errno::RANGE && buf.len() < MAX_XATTR_VALUE => {
                buf.resize(buf.len().saturating_mul(4).min(MAX_XATTR_VALUE), 0);
            }
            Err(_) => return None,
        }
    }
}

/// Copy an explicit macOS access-control list (`chmod +a`) from `from` onto `to` (#7073).
///
/// `fcopyfile(COPYFILE_ACL)` is the obvious call and is an `unsafe fn`, which this workspace
/// forbids; `fclonefileat` with `CLONE_ACL` is safe but only works on APFS and would mean
/// writing every save as a clone-then-truncate. `exacl` is a small, safe wrapper over
/// `acl_get_file`/`acl_set_file` that works on every file system that has ACLs at all.
///
/// An old file with no ACL leaves `to` alone: the new file may have picked up the folder's
/// inherited entries, and taking access away is a worse surprise than leaving it as the folder
/// says it should be.
#[cfg(target_os = "macos")]
fn copy_acl(from: &Path, to: &Path) -> io::Result<()> {
    let entries = exacl::getfacl(from, None)?;
    if entries.is_empty() {
        return Ok(());
    }
    exacl::setfacl(&[to], &entries, None)
}

/// Replace the **existing** file `to` with `from`, keeping what the destination owns rather than
/// its bytes.
///
/// Everywhere but Windows this is exactly [`std::fs::rename`], and the metadata is carried by
/// [`copy_metadata`] onto `from` beforehand. On Windows it is `ReplaceFileW`, which preserves
/// the replaced file's DACL (#7096), its named streams — `Zone.Identifier`, the Mark of the Web
/// (#7094) — its creation time, object identifier, compression and encryption, in the one call
/// Microsoft documents for the save-through-a-temporary-file pattern. None of those have a safe
/// path-based API to copy by hand.
pub fn replace_existing(from: &Path, to: &Path) -> io::Result<()> {
    #[cfg(windows)]
    {
        windows_replace(from, to)
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(from, to)
    }
}

/// `ReplaceFileW`, with the plain rename as the fallback. See [`replace_existing`].
///
/// **A backup name is not optional.** `ReplaceFile` without one deletes the replaced file before
/// the final rename, and documents `ERROR_UNABLE_TO_MOVE_REPLACEMENT`: "the replaced file no
/// longer exists and the replacement file exists under its original name" — the user's document
/// gone, its replacement left under a hidden temporary name. With a backup name the same failure
/// leaves both files where they were. So this always passes one, and deletes it on the way out.
///
/// The failures and what each leaves behind (from the `ReplaceFileW` reference):
///
/// - `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2` (1177) is the only one that moves anything: the
///   replacement keeps its name but has already inherited the destination's streams and
///   attributes, and the destination's old content is at the backup name. Finishing the rename
///   by hand completes the save *with* its metadata; if even that fails, the backup goes back to
///   the destination so the user's file is where they left it.
/// - Every other error leaves the replaced and replacement files under their own names, so a
///   plain rename still saves the document — without the metadata, which is where this started.
#[cfg(windows)]
fn windows_replace(from: &Path, to: &Path) -> io::Result<()> {
    use winsafe::co::REPLACEFILE;

    /// `ERROR_UNABLE_TO_MOVE_REPLACEMENT_2`: the destination now lives under the backup name.
    const MOVE_REPLACEMENT_2: i32 = 1177;

    let backup = backup_path(to);
    let paths = (to.to_str(), from.to_str(), backup.as_deref().and_then(Path::to_str));
    // `ReplaceFileW` takes `&str`; a path that is not valid Unicode, or one past MAX_PATH, is
    // the plain rename's job.
    let (Some(replaced), Some(replacement), Some(backup_str)) = paths else {
        return std::fs::rename(from, to);
    };

    let flags = REPLACEFILE::IGNORE_MERGE_ERRORS | REPLACEFILE::IGNORE_ACL_ERRORS;
    match winsafe::ReplaceFile(replaced, replacement, Some(backup_str), flags) {
        Ok(()) => {
            remove_backup(backup.as_deref());
            Ok(())
        }
        Err(e) if e.raw() as i32 == MOVE_REPLACEMENT_2 => match std::fs::rename(from, to) {
            Ok(()) => {
                remove_backup(backup.as_deref());
                Ok(())
            }
            Err(rename_failed) => {
                if let Some(backup) = backup.as_deref() {
                    let _ = std::fs::rename(backup, to);
                }
                Err(rename_failed)
            }
        },
        Err(_) => {
            remove_backup(backup.as_deref());
            std::fs::rename(from, to)
        }
    }
}

/// A unique name beside `to` for `ReplaceFile` to park the replaced file under. It shares the
/// temporary-file suffix, so the save's own cleanup and the tests' leftover check cover it.
#[cfg(windows)]
fn backup_path(to: &Path) -> Option<std::path::PathBuf> {
    let name = to.file_name()?.to_string_lossy().into_owned();
    let dir = match to.parent() {
        Some(d) if !d.as_os_str().is_empty() => d,
        _ => Path::new("."),
    };
    Some(dir.join(super::temp_name(&name)))
}

#[cfg(windows)]
fn remove_backup(backup: Option<&Path>) {
    if let Some(backup) = backup {
        let _ = std::fs::remove_file(backup);
    }
}

#[cfg(test)]
mod tests;
