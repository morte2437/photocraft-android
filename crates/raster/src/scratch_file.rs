//! The scratch file: where cold tiles go when pixel memory is over budget.
//!
//! - One file per configured directory, never visible after the process ends. Unix creates it
//!   with private permissions and unlinks it right after opening, so the space is returned when
//!   the last handle closes, even on a crash or `exit` without destructors. Windows opens it
//!   delete-on-close. Files of dead processes (an older build, or a crash between creating and
//!   unlinking) are reaped when the directory is next configured.
//! - Space is handed out in 4 KiB-aligned extents from a free list (best fit, coalescing), else
//!   appended at the end.
//! - All I/O is positioned: no shared cursor, no lock held during I/O. The allocator lock only
//!   guards the free list.
//! - Unix reads and writes with `pread`/`pwrite` on the one handle (no reopen by path, which
//!   is impossible once unlinked). Windows reads through one handle per thread (the file opened
//!   again by path): it serialises I/O on one synchronous handle, so parallel readers sharing it
//!   would run one at a time.
//!
//! Ported from Fotox's `fx-tiles` scratch file (same author), minus its `unsafe` `ReOpenFile`.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::path::{Path, PathBuf};
#[cfg(windows)]
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

const ALIGN: u64 = 4096;

/// A region of the scratch file holding one compressed tile.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Extent {
    pub offset: u64,
    /// Exact length of the data.
    pub len: u32,
    /// Allocated length (multiple of 4 KiB, ≥ `len`).
    pub alloc: u64,
    /// CRC-32 of the stored bytes, checked on every read.
    pub crc32: u32,
}

/// Free-list allocator. Pure bookkeeping, no I/O.
#[derive(Debug, Default)]
pub(crate) struct Allocator {
    by_offset: BTreeMap<u64, u64>,
    by_size: BTreeSet<(u64, u64)>,
    end: u64,
}

impl Allocator {
    /// Bytes currently allocated (end of file minus free space).
    pub fn used(&self) -> u64 {
        self.end - self.by_offset.values().sum::<u64>()
    }

    pub fn alloc(&mut self, len: u32) -> Extent {
        let size = u64::from(len).div_ceil(ALIGN).max(1) * ALIGN;
        if let Some(&(free_size, offset)) = self.by_size.range((size, 0)..).next() {
            self.remove_free(offset, free_size);
            if free_size > size {
                self.insert_free(offset + size, free_size - size);
            }
            return Extent { offset, len, alloc: size, crc32: 0 };
        }
        let offset = self.end;
        self.end += size;
        Extent { offset, len, alloc: size, crc32: 0 }
    }

    pub fn free(&mut self, extent: Extent) {
        let mut offset = extent.offset;
        let mut size = extent.alloc;
        if let Some((&prev_off, &prev_size)) = self.by_offset.range(..offset).next_back()
            && prev_off + prev_size == offset
        {
            self.remove_free(prev_off, prev_size);
            offset = prev_off;
            size += prev_size;
        }
        if let Some(&next_size) = self.by_offset.get(&(offset + size)) {
            self.remove_free(offset + size, next_size);
            size += next_size;
        }
        if offset + size == self.end {
            // Free space at the end of the file just shrinks the used range.
            self.end = offset;
        } else {
            self.insert_free(offset, size);
        }
    }

    fn insert_free(&mut self, offset: u64, size: u64) {
        self.by_offset.insert(offset, size);
        self.by_size.insert((size, offset));
    }

    fn remove_free(&mut self, offset: u64, size: u64) {
        self.by_offset.remove(&offset);
        self.by_size.remove(&(size, offset));
    }
}

pub(crate) struct ScratchFile {
    main: File,
    #[cfg(windows)]
    readers: OnceLock<Vec<File>>,
    path: PathBuf,
    /// The directory entry still exists (Unix unlink failed): remove it on drop.
    #[cfg(not(windows))]
    linked: bool,
    allocator: Mutex<Allocator>,
}

impl std::fmt::Debug for ScratchFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ScratchFile").field("path", &self.path).finish()
    }
}

impl ScratchFile {
    pub fn create(dir: &Path) -> std::io::Result<Self> {
        std::fs::create_dir_all(dir)?;
        static COUNTER: AtomicUsize = AtomicUsize::new(0);
        // A reused process id must not collide with an abandoned file from an older launch.
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
            .saturating_add(COUNTER.fetch_add(1, Ordering::Relaxed) as u128);
        let path = dir.join(format!("photocraft-{}-{n}.scratch", std::process::id()));
        let mut options = std::fs::OpenOptions::new();
        options.read(true).write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        #[cfg(windows)]
        {
            use std::os::windows::fs::OpenOptionsExt;
            const FILE_FLAG_DELETE_ON_CLOSE: u32 = 0x0400_0000;
            options.custom_flags(FILE_FLAG_DELETE_ON_CLOSE);
        }
        let main = options.open(&path)?;
        #[cfg(unix)]
        reap_abandoned(dir, &main);
        // Unix: the open handle keeps the inode; nothing is left behind at any exit. A
        // filesystem that refuses the unlink keeps the name until drop or the next reap.
        #[cfg(not(windows))]
        let linked = std::fs::remove_file(&path).is_err();
        Ok(Self {
            main,
            #[cfg(windows)]
            readers: OnceLock::new(),
            path,
            #[cfg(not(windows))]
            linked,
            allocator: Mutex::new(Allocator::default()),
        })
    }

    /// Another handle to the open file (the Unix one has no name), for fault-injection tests.
    pub(crate) fn duplicate_handle(&self) -> std::io::Result<File> {
        self.main.try_clone()
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Reserve space and write `data` there.
    pub fn write(&self, data: &[u8]) -> std::io::Result<Extent> {
        let len = u32::try_from(data.len()).map_err(|_| std::io::Error::other("block larger than 4 GiB"))?;
        let mut extent = self.lock().alloc(len);
        if let Err(e) = write_all_at(&self.main, data, extent.offset) {
            self.free(extent);
            return Err(e);
        }
        extent.crc32 = crc32fast::hash(data);
        Ok(extent)
    }

    /// Read an extent into `buf` (resized to it) and check its CRC.
    pub fn read_into(&self, extent: Extent, buf: &mut Vec<u8>) -> std::io::Result<()> {
        buf.resize(extent.len as usize, 0);
        read_exact_at(self.reader(), buf, extent.offset)?;
        if crc32fast::hash(buf) != extent.crc32 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("scratch CRC mismatch at offset {}", extent.offset)));
        }
        Ok(())
    }

    pub fn free(&self, extent: Extent) {
        let mut allocator = self.lock();
        allocator.free(extent);
        if allocator.end == 0 {
            // Release the physical disk space after the last document/history tile closes.
            // Keep the allocator locked so a new writer cannot reserve an extent before
            // this truncation; failures retain excess space, never live tile bytes.
            let _ = self.main.set_len(0);
        }
    }

    pub fn used(&self) -> u64 {
        self.lock().used()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Allocator> {
        self.allocator.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Positioned reads on one handle run in parallel on Unix.
    #[cfg(not(windows))]
    fn reader(&self) -> &File {
        &self.main
    }

    /// A handle for reading on the calling thread.
    #[cfg(windows)]
    fn reader(&self) -> &File {
        let extra = self.readers.get_or_init(|| {
            // The same file opened again by path: std shares read, write and delete, so this
            // works while `main` (opened delete-on-close) is alive. A failure leaves fewer
            // handles, never an error.
            let count = std::thread::available_parallelism().map_or(4, |n| n.get()).clamp(1, 16) - 1;
            (0..count).map_while(|_| std::fs::OpenOptions::new().read(true).open(&self.path).ok()).collect()
        });
        match thread_slot() % (extra.len() + 1) {
            0 => &self.main,
            i => extra.get(i - 1).unwrap_or(&self.main),
        }
    }
}

/// Accept only the names we generate, never other temporary files or directories.
#[cfg(any(unix, test))]
fn scratch_pid(name: &str) -> Option<u32> {
    let (pid, nonce) = name.strip_prefix("photocraft-")?.strip_suffix(".scratch")?.split_once('-')?;
    if pid.is_empty() || nonce.is_empty() || !pid.bytes().chain(nonce.bytes()).all(|b| b.is_ascii_digit()) {
        return None;
    }
    let pid = pid.parse::<u32>().ok()?;
    (pid > 0 && nonce.parse::<u128>().is_ok()).then_some(pid)
}

#[cfg(unix)]
fn reap_abandoned(dir: &Path, own: &File) {
    use std::os::unix::fs::MetadataExt;
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    let Ok(metadata) = own.metadata() else { return };
    let mut system = System::new();
    system.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    // Fail closed when process enumeration isn't available. Live files are never removed.
    if system.process(Pid::from_u32(std::process::id())).is_none() {
        return;
    }
    reap_with(dir, metadata.uid(), |pid| system.process(Pid::from_u32(pid)).is_some());
}

#[cfg(unix)]
fn reap_with(dir: &Path, uid: u32, live: impl Fn(u32) -> bool) {
    use std::os::unix::fs::MetadataExt;
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(scratch_pid) else { continue };
        if pid == std::process::id() || live(pid) {
            continue;
        }
        let path = entry.path();
        let Ok(meta) = std::fs::symlink_metadata(&path) else { continue };
        // Shared /tmp: do not remove other users' files or follow symlinks.
        if meta.file_type().is_file() && meta.uid() == uid {
            let _ = std::fs::remove_file(path);
        }
    }
}

impl Drop for ScratchFile {
    fn drop(&mut self) {
        // Windows deletes on close and Unix unlinked at creation; only a refused unlink remains.
        #[cfg(not(windows))]
        if self.linked {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

/// A stable small number per thread, handed out in turn, so the worker threads of a pool land
/// on different handles.
#[cfg(windows)]
fn thread_slot() -> usize {
    static NEXT: AtomicUsize = AtomicUsize::new(0);
    thread_local! {
        static SLOT: usize = NEXT.fetch_add(1, Ordering::Relaxed);
    }
    SLOT.with(|slot| *slot)
}

#[cfg(unix)]
fn write_all_at(file: &File, data: &[u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::write_all_at(file, data, offset)
}

#[cfg(unix)]
fn read_exact_at(file: &File, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
    std::os::unix::fs::FileExt::read_exact_at(file, buf, offset)
}

#[cfg(windows)]
fn write_all_at(file: &File, mut data: &[u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !data.is_empty() {
        let n = file.seek_write(data, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::WriteZero.into());
        }
        data = data.get(n..).unwrap_or_default();
        offset += n as u64;
    }
    Ok(())
}

#[cfg(windows)]
fn read_exact_at(file: &File, mut buf: &mut [u8], mut offset: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    while !buf.is_empty() {
        let n = file.seek_read(buf, offset)?;
        if n == 0 {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        buf = buf.get_mut(n..).unwrap_or_default();
        offset += n as u64;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn write_all_at(_file: &File, _data: &[u8], _offset: u64) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(not(any(unix, windows)))]
fn read_exact_at(_file: &File, _buf: &mut [u8], _offset: u64) -> std::io::Result<()> {
    Err(std::io::ErrorKind::Unsupported.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_names_are_strict() {
        assert_eq!(scratch_pid("photocraft-123-987.scratch"), Some(123));
        for name in [
            "photocraft-0-1.scratch",
            "photocraft-1-.scratch",
            "photocraft-1-../notes.scratch",
            "photocraft-1-2.scratch.bak",
            "other-1-2.scratch",
            "photocraft-4294967296-2.scratch",
        ] {
            assert_eq!(scratch_pid(name), None, "{name}");
        }
    }

    #[cfg(any(unix, windows))]
    #[test]
    fn abrupt_exit_does_not_leave_permanent_scratch_files() {
        const CHILD_DIR: &str = "PHOTOCRAFT_SCRATCH_EXIT_TEST_DIR";
        if let Some(dir) = std::env::var_os(CHILD_DIR) {
            let file = ScratchFile::create(Path::new(&dir)).unwrap();
            file.write(b"unreleased pixels").unwrap();
            // No Rust destructors run: this exercises the lifetime of the static manager.
            std::process::exit(0);
        }
        let dir = std::env::temp_dir().join(format!("pc-abrupt-exit-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let result = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "scratch_file::tests::abrupt_exit_does_not_leave_permanent_scratch_files", "--nocapture"])
            .env(CHILD_DIR, &dir)
            .status()
            .unwrap();
        assert!(result.success());
        // Windows: delete-on-close. Unix: unlinked right after opening. Neither needs Drop.
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "no scratch file outlives an exit without destructors");
        let next = ScratchFile::create(&dir).unwrap();
        // Unix unlinks the open file; Windows keeps the entry until the delete-on-close handle closes.
        let live_entries = if cfg!(unix) { 0 } else { 1 };
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), live_entries, "a live scratch file is unlinked on Unix only");
        next.write(b"still usable").unwrap();
        drop(next);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "closing the last scratch file leaves the directory empty");
        std::fs::remove_dir(dir).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn unix_files_are_private_and_reaping_preserves_live_and_unrelated_files() {
        use std::os::unix::fs::{MetadataExt, PermissionsExt};
        let dir = std::env::temp_dir().join(format!("pc-reap-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let own = ScratchFile::create(&dir).unwrap();
        let meta = own.main.metadata().unwrap();
        assert_eq!(meta.permissions().mode() & 0o777, 0o600);
        let stale = dir.join("photocraft-123-1.scratch");
        let live = dir.join("photocraft-124-1.scratch");
        let unrelated = dir.join("notes.scratch");
        let link = dir.join("photocraft-125-1.scratch");
        for p in [&stale, &live, &unrelated] {
            std::fs::write(p, b"test").unwrap();
        }
        std::os::unix::fs::symlink(&unrelated, &link).unwrap();
        reap_with(&dir, meta.uid().wrapping_add(1), |_| false);
        assert!(stale.exists(), "another owner's files remain");
        reap_with(&dir, meta.uid(), |pid| pid == 124);
        assert!(!stale.exists());
        assert!(live.exists() && unrelated.exists() && link.exists());
        assert!(!own.path.exists() && !own.linked, "the open scratch file is already unlinked");
        drop(own);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parallel_positioned_reads_share_one_unlinked_file() {
        let dir = std::env::temp_dir().join(format!("pc-scratch-parallel-{}", std::process::id()));
        let scratch = ScratchFile::create(&dir).unwrap();
        let extents: Vec<_> = (0..32u8).map(|i| scratch.write(&vec![i; 5000 + usize::from(i)]).unwrap()).collect();
        std::thread::scope(|s| {
            for t in 0..8 {
                let (scratch, extents) = (&scratch, &extents);
                s.spawn(move || {
                    let mut buf = Vec::new();
                    for (i, e) in extents.iter().enumerate().skip(t) {
                        scratch.read_into(*e, &mut buf).unwrap();
                        assert_eq!(buf, vec![i as u8; 5000 + i]);
                    }
                });
            }
        });
        drop(scratch);
        std::fs::remove_dir(dir).unwrap();
    }

    #[test]
    fn alloc_reuses_and_coalesces() {
        let mut a = Allocator::default();
        let x = a.alloc(100);
        let y = a.alloc(5000);
        let z = a.alloc(4096);
        assert_eq!((x.offset, y.offset, z.offset), (0, 4096, 12288));
        assert_eq!(a.used(), 16384);
        a.free(x);
        a.free(y); // coalesces with x: one free extent of 12 KiB at 0
        let w = a.alloc(12000);
        assert_eq!(w.offset, 0, "coalesced hole is reused");
        a.free(z); // tail extent: the file end shrinks
        assert_eq!(a.used(), 12288);
        a.free(w);
        assert_eq!(a.used(), 0);
        assert_eq!(a.end, 0, "everything freed collapses the file");
    }

    #[test]
    fn best_fit_splits_larger_holes() {
        let mut a = Allocator::default();
        let big = a.alloc(40000);
        let _tail = a.alloc(1);
        a.free(big);
        assert_eq!(a.alloc(10).offset, 0);
        assert_eq!(a.alloc(10).offset, 4096, "remainder of the split hole is used next");
    }

    #[test]
    fn file_roundtrip_and_crc() {
        let dir = std::env::temp_dir().join("photocraft-scratch-test");
        let scratch = ScratchFile::create(&dir).unwrap();
        let a = scratch.write(b"hello tiles").unwrap();
        let b = scratch.write(&[7u8; 9000]).unwrap();
        let mut buf = Vec::new();
        scratch.read_into(a, &mut buf).unwrap();
        assert_eq!(buf, b"hello tiles");
        scratch.read_into(b, &mut buf).unwrap();
        assert_eq!(buf, vec![7u8; 9000]);
        let bad = Extent { crc32: a.crc32 ^ 1, ..a };
        assert!(scratch.read_into(bad, &mut buf).is_err(), "a CRC mismatch is an error, not garbage");
        scratch.free(a);
        scratch.free(b);
        assert_eq!(scratch.main.metadata().unwrap().len(), 0, "closing the last tile releases physical disk space");
        let path = scratch.path().to_path_buf();
        drop(scratch);
        assert!(!path.exists(), "scratch file is removed on drop");
    }
}
