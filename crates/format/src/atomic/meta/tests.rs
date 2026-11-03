//! What a save keeps from the file it replaces.
//!
//! The Unix tests run for real. The Windows ones exercise `ReplaceFileW` through
//! [`crate::atomic::atomic_write`] and can only run on Windows, so they are `#[cfg(windows)]`
//! and are covered by the Windows CI job, not by a developer on a Mac.

use crate::atomic::atomic_write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

struct TempDir(PathBuf);
impl TempDir {
    fn new(tag: &str) -> Self {
        static N: AtomicU64 = AtomicU64::new(0);
        let d = std::env::temp_dir().join(format!("pc-meta-{tag}-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Self(d)
    }
    fn join(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// No temporary or backup file is left beside the saved document.
fn assert_no_temp(dir: &Path) {
    let leftovers: Vec<_> =
        std::fs::read_dir(dir).unwrap().flatten().map(|e| e.file_name().to_string_lossy().into_owned()).filter(|n| n.ends_with(".tmp")).collect();
    assert!(leftovers.is_empty(), "temporary files left behind: {leftovers:?}");
}

// ---------------------------------------------------------------- extended attributes (Unix)

#[cfg(any(target_os = "linux", target_os = "android", target_vendor = "apple"))]
mod xattrs {
    use super::*;

    /// macOS wants a reverse-DNS name; Linux only lets an unprivileged process write `user.*`.
    #[cfg(target_os = "macos")]
    const TAG: &str = "com.apple.metadata:_kMDItemUserTags";
    #[cfg(not(target_os = "macos"))]
    const TAG: &str = "user.photocraft.test";

    fn set(path: &Path, name: &str, value: &[u8]) -> bool {
        rustix::fs::setxattr(path, name, value, rustix::fs::XattrFlags::empty()).is_ok()
    }

    fn get(path: &Path, name: &str) -> Option<Vec<u8>> {
        let mut buf = vec![0u8; 1 << 20];
        let len = rustix::fs::getxattr(path, name, buf.as_mut_slice()).ok()?;
        buf.truncate(len);
        Some(buf)
    }

    /// #6603: the attributes of the replaced file — a Finder tag, a `user.*` attribute — are on
    /// the file the user is left with.
    #[test]
    fn replacing_a_file_keeps_its_extended_attributes() {
        let d = TempDir::new("xattr");
        let p = d.join("doc.psd");
        atomic_write(&p, b"first").unwrap();
        if !set(&p, TAG, b"tagged") {
            // tmpfs and some CI file systems have no xattr support; nothing to assert.
            return;
        }

        atomic_write(&p, b"second, longer").unwrap();

        assert_eq!(std::fs::read(&p).unwrap(), b"second, longer");
        assert_eq!(get(&p, TAG).as_deref(), Some(&b"tagged"[..]), "the replaced file's attribute was dropped");
        assert_no_temp(&d.0);
    }

    /// A large value still comes across (and is not silently truncated).
    #[test]
    fn a_large_attribute_value_survives() {
        let d = TempDir::new("xattr-big");
        let p = d.join("doc.psd");
        atomic_write(&p, b"first").unwrap();
        // Comfortably past the first read buffer, so the grow-on-ERANGE path runs.
        let value = vec![0xABu8; 64 << 10];
        if !set(&p, TAG, &value) {
            return;
        }

        atomic_write(&p, b"second").unwrap();

        assert_eq!(get(&p, TAG), Some(value));
    }

    /// A read-only file is refused outright (#6600), so a save never reaches the metadata copy and
    /// the file keeps everything it had. The writable cases above cover the copy itself.
    #[test]
    fn a_read_only_file_keeps_its_attributes_and_its_mode() {
        use std::os::unix::fs::PermissionsExt;
        let d = TempDir::new("xattr-ro");
        let p = d.join("doc.psd");
        atomic_write(&p, b"first").unwrap();
        if !set(&p, TAG, b"tagged") {
            return;
        }
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o444)).unwrap();

        // #6600: a read-only file is refused before anything is written, so the question this test
        // asks — does a replace keep the attributes and the mode — is answered by the save not
        // happening. The writable case above covers attributes surviving a replace.
        let e = atomic_write(&p, b"second, longer").expect_err("a read-only file is refused");
        assert_eq!(e.kind(), std::io::ErrorKind::PermissionDenied);

        assert_eq!(std::fs::read(&p).unwrap(), b"first", "the original was left alone");
        assert_eq!(get(&p, TAG).as_deref(), Some(&b"tagged"[..]), "a read-only file lost its attribute");
        assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o444);
    }

    /// Creating a file that was not there sets no attributes and does not fail.
    #[test]
    fn creating_a_new_file_needs_no_source() {
        let d = TempDir::new("xattr-new");
        let p = d.join("fresh.psd");
        atomic_write(&p, b"bytes").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"bytes");
        assert!(get(&p, TAG).is_none());
    }

    /// The descriptor-based copy is what the automation write path uses (#7095): it works on two
    /// open files and never looks at a path.
    #[test]
    fn copy_xattrs_works_on_descriptors() {
        use std::os::fd::AsFd;
        let d = TempDir::new("xattr-fd");
        let (from, to) = (d.join("from"), d.join("to"));
        std::fs::write(&from, b"a").unwrap();
        std::fs::write(&to, b"b").unwrap();
        if !set(&from, TAG, b"carried") {
            return;
        }

        let old = std::fs::File::open(&from).unwrap();
        let new = std::fs::OpenOptions::new().write(true).open(&to).unwrap();
        super::super::copy_xattrs(old.as_fd(), new.as_fd()).unwrap();

        assert_eq!(get(&to, TAG).as_deref(), Some(&b"carried"[..]));
        assert_eq!(std::fs::read(&to).unwrap(), b"b", "copying attributes must not touch the bytes");
    }
}

// --------------------------------------------------------------------------- mode (Unix)

/// The mode was already kept before any of this; it still is.
#[cfg(unix)]
#[test]
fn replacing_a_file_keeps_its_mode() {
    use std::os::unix::fs::PermissionsExt;
    let d = TempDir::new("mode");
    let p = d.join("doc.psd");
    atomic_write(&p, b"first").unwrap();
    std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600)).unwrap();

    atomic_write(&p, b"second").unwrap();

    assert_eq!(std::fs::metadata(&p).unwrap().permissions().mode() & 0o777, 0o600);
    assert_no_temp(&d.0);
}

// ----------------------------------------------------------------------------- ACL (macOS)

#[cfg(target_os = "macos")]
mod acl {
    use super::*;

    /// #7073: an ACL entry added with `chmod +a` is still there after a save.
    #[test]
    fn replacing_a_file_keeps_its_acl() {
        let d = TempDir::new("acl");
        let p = d.join("doc.psd");
        atomic_write(&p, b"first").unwrap();

        // The same thing `chmod +a` does: one explicit entry for the user running the test, so
        // no second account has to exist.
        let Ok(user) = std::env::var("USER") else { return };
        let entries = [exacl::AclEntry::allow_user(&user, exacl::Perm::READ | exacl::Perm::WRITE, None)];
        if exacl::setfacl(&[&p], &entries, None).is_err() {
            // A file system without ACL support (a RAM disk, some CI volumes).
            return;
        }
        let before = exacl::getfacl(&p, None).unwrap();
        assert!(!before.is_empty(), "the test could not set an ACL to begin with");

        atomic_write(&p, b"second, longer").unwrap();

        assert_eq!(std::fs::read(&p).unwrap(), b"second, longer");
        assert_eq!(exacl::getfacl(&p, None).unwrap(), before, "the replaced file's ACL was dropped");
        assert_no_temp(&d.0);
    }

    /// A file with no ACL keeps having no ACL, rather than gaining an empty one.
    #[test]
    fn a_file_without_an_acl_is_left_alone() {
        let d = TempDir::new("acl-none");
        let p = d.join("doc.psd");
        atomic_write(&p, b"first").unwrap();
        let before = exacl::getfacl(&p, None).unwrap();

        atomic_write(&p, b"second").unwrap();

        assert_eq!(exacl::getfacl(&p, None).unwrap(), before);
    }
}

// --------------------------------------------------------------- named streams and ACL (Windows)

/// Unverified on a Mac: these run only on Windows (CI's Windows job).
#[cfg(windows)]
mod windows {
    use super::*;

    /// Open a named stream of `path`, e.g. `Zone.Identifier`.
    fn stream(path: &Path, name: &str) -> PathBuf {
        let mut s = path.as_os_str().to_os_string();
        s.push(":");
        s.push(name);
        PathBuf::from(s)
    }

    /// #7094: a file marked as downloaded from the internet is still marked after a save, so the
    /// shell and other applications keep warning about it.
    #[test]
    fn replacing_a_file_keeps_its_mark_of_the_web() {
        let d = TempDir::new("motw");
        let p = d.join("doc.psd");
        atomic_write(&p, b"first").unwrap();
        let motw = b"[ZoneTransfer]\r\nZoneId=3\r\n";
        std::fs::write(stream(&p, "Zone.Identifier"), motw).unwrap();

        atomic_write(&p, b"second, longer").unwrap();

        assert_eq!(std::fs::read(&p).unwrap(), b"second, longer");
        assert_eq!(std::fs::read(stream(&p, "Zone.Identifier")).unwrap(), motw, "the Mark of the Web was dropped");
        assert_no_temp(&d.0);
    }

    /// Any other named stream rides along too, because `ReplaceFileW` keeps the lot.
    #[test]
    fn replacing_a_file_keeps_other_named_streams() {
        let d = TempDir::new("ads");
        let p = d.join("doc.psd");
        atomic_write(&p, b"first").unwrap();
        std::fs::write(stream(&p, "photocraft.test"), b"side data").unwrap();

        atomic_write(&p, b"second").unwrap();

        assert_eq!(std::fs::read(stream(&p, "photocraft.test")).unwrap(), b"side data");
    }

    /// #7096: an explicit ACL entry survives, instead of the file falling back to whatever the
    /// folder hands down. Compared as the string `icacls` prints, which is what a user checks.
    #[test]
    fn replacing_a_file_keeps_its_explicit_acl() {
        let d = TempDir::new("dacl");
        let p = d.join("doc.psd");
        atomic_write(&p, b"first").unwrap();

        // Grant the current user an entry that inheritance would not have produced.
        let user = std::env::var("USERNAME").unwrap_or_default();
        if user.is_empty() {
            return;
        }
        let granted = std::process::Command::new("icacls").arg(&p).arg("/grant").arg(format!("{user}:(RX)")).output();
        if !granted.map(|o| o.status.success()).unwrap_or(false) {
            return;
        }
        let before = std::process::Command::new("icacls").arg(&p).output().unwrap().stdout;

        atomic_write(&p, b"second, longer").unwrap();

        let after = std::process::Command::new("icacls").arg(&p).output().unwrap().stdout;
        assert_eq!(String::from_utf8_lossy(&after), String::from_utf8_lossy(&before), "the explicit ACL was dropped");
        assert_no_temp(&d.0);
    }

    /// `ReplaceFileW` also keeps the creation time, which the rename used to reset (#6603).
    #[test]
    fn replacing_a_file_keeps_its_creation_time() {
        let d = TempDir::new("born");
        let p = d.join("doc.psd");
        atomic_write(&p, b"first").unwrap();
        let before = std::fs::metadata(&p).unwrap().created().unwrap();

        std::thread::sleep(std::time::Duration::from_millis(50));
        atomic_write(&p, b"second").unwrap();

        assert_eq!(std::fs::metadata(&p).unwrap().created().unwrap(), before);
    }

    /// Creating a file where there was none still works: there is nothing to replace, so the
    /// write never reaches `ReplaceFileW`.
    #[test]
    fn creating_a_new_file_still_works() {
        let d = TempDir::new("new");
        let p = d.join("fresh.psd");
        atomic_write(&p, b"bytes").unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"bytes");
        assert_no_temp(&d.0);
    }
}
