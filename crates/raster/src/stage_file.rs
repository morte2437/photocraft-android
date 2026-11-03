//! Encoded data an exporter parks on the scratch disk instead of in memory (#5333).
//!
//! A large PSD/PSB save encodes every channel before it writes a byte (so it can still be
//! cancelled, and its scratch reads are checked, before the file is replaced). Kept in memory,
//! those channels are a second copy of the document. A [`StageFile`] holds them on the scratch
//! disk instead: an append-only file made like the tile scratch file (private, unlinked on Unix,
//! delete-on-close on Windows), so nothing is left behind at any exit, and the space returns as
//! soon as the save drops it.
//!
//! Like the tile scratch file, every append carries a CRC-32: a read pass that covers an append
//! from its start to its end (whole, or in consecutive parts, as a save copies it) fails on its
//! last part if the disk handed back different bytes, so the save errors before it replaces
//! anything instead of writing them into the document.

use std::collections::BTreeMap;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use crate::scratch_file::{open_private, read_exact_at, write_all_at};

pub struct StageFile {
    file: File,
    path: PathBuf,
    /// The directory entry still exists (the unlink was refused): remove it on drop.
    linked: bool,
    /// Bytes reserved so far; appends take their offset from it.
    end: Mutex<u64>,
    /// Each append by its offset, with its checksum and the read pass under way.
    extents: Mutex<BTreeMap<u64, Extent>>,
}

struct Extent {
    len: u64,
    crc32: u32,
    /// Where the current read pass continues (`None` once it skipped or jumped: not checked).
    next: Option<u64>,
    hasher: crc32fast::Hasher,
}

impl std::fmt::Debug for StageFile {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("StageFile").field("path", &self.path).field("len", &self.len()).finish()
    }
}

impl StageFile {
    /// A new, empty stage file in `dir`.
    pub fn create(dir: &Path) -> std::io::Result<Self> {
        let (file, path, linked) = open_private(dir)?;
        Ok(Self { file, path, linked, end: Mutex::new(0), extents: Mutex::default() })
    }

    /// A stage file in the configured scratch directory, when spilling is on (`None` otherwise,
    /// or when the file cannot be made: the caller then keeps its data in memory).
    pub fn in_scratch() -> Option<Self> {
        if !crate::spill::enabled() {
            return None;
        }
        Self::create(&crate::spill::stats().dir?).ok()
    }

    /// Appends `data` and returns its offset. Parallel appends each get their own range.
    pub fn append(&self, data: &[u8]) -> std::io::Result<u64> {
        let offset = {
            let mut end = self.end.lock().unwrap_or_else(PoisonError::into_inner);
            let offset = *end;
            *end = offset.checked_add(data.len() as u64).ok_or_else(|| std::io::Error::other("stage file too large"))?;
            offset
        };
        write_all_at(&self.file, data, offset)?;
        let extent = Extent { len: data.len() as u64, crc32: crc32fast::hash(data), next: Some(offset), hasher: crc32fast::Hasher::new() };
        self.extents.lock().unwrap_or_else(PoisonError::into_inner).insert(offset, extent);
        Ok(offset)
    }

    /// Fills `buf` from `offset`, which must lie within what was appended. A read that ends a
    /// pass over one append from its start fails with `InvalidData` if the append's bytes changed.
    pub fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        let Some(end) = offset.checked_add(buf.len() as u64).filter(|&end| end <= self.len()) else {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        };
        read_exact_at(&self.file, buf, offset)?;
        let mut extents = self.extents.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((&start, extent)) = extents.range_mut(..=offset).next_back() else { return Ok(()) };
        let extent_end = start + extent.len;
        if end > extent_end {
            return Ok(()); // spans appends (no save reads this way): not checked
        }
        if offset == start {
            extent.hasher = crc32fast::Hasher::new();
            extent.next = Some(start);
        }
        if extent.next != Some(offset) {
            extent.next = None;
            return Ok(());
        }
        extent.hasher.update(buf);
        extent.next = Some(end);
        if end == extent_end {
            let crc32 = std::mem::take(&mut extent.hasher).finalize();
            extent.next = None;
            if crc32 != extent.crc32 {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "staged save data changed on the scratch disk (checksum mismatch)"));
            }
        }
        Ok(())
    }

    /// Bytes appended (reserved) so far.
    pub fn len(&self) -> u64 {
        *self.end.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for StageFile {
    fn drop(&mut self) {
        if self.linked {
            let _ = std::fs::remove_file(&self.path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_read_back_and_leave_nothing_behind() {
        let dir = std::env::temp_dir().join(format!("pc-stage-{}", std::process::id()));
        let stage = StageFile::create(&dir).unwrap();
        #[cfg(unix)]
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0, "the stage file is unlinked while open");
        let parts: Vec<Vec<u8>> = (0..8u8).map(|i| vec![i; 1000 + usize::from(i)]).collect();
        let offsets: Vec<u64> = std::thread::scope(|s| {
            let handles: Vec<_> = parts.iter().map(|p| s.spawn(|| stage.append(p).unwrap())).collect();
            handles.into_iter().map(|h| h.join().unwrap()).collect()
        });
        assert_eq!(stage.len(), parts.iter().map(|p| p.len() as u64).sum::<u64>());
        for (p, &at) in parts.iter().zip(&offsets) {
            let mut buf = vec![0; p.len()];
            stage.read_exact_at(&mut buf, at).unwrap();
            assert_eq!(&buf, p);
        }
        let mut past = [0u8; 2];
        assert!(stage.read_exact_at(&mut past, stage.len() - 1).is_err(), "reads stop at what was appended");
        drop(stage);
        assert_eq!(std::fs::read_dir(&dir).unwrap().count(), 0);
        let _ = std::fs::remove_dir(&dir);
    }

    /// Bytes that change on the disk fail the pass that copies them, whole or in parts.
    #[test]
    fn a_changed_append_fails_its_read_pass() {
        let dir = std::env::temp_dir().join(format!("pc-stage-crc-{}", std::process::id()));
        let stage = StageFile::create(&dir).unwrap();
        let first = stage.append(&[1; 100]).unwrap();
        let second = stage.append(&[2; 100]).unwrap();
        let pass = |at: u64, part: usize| -> std::io::Result<()> {
            let mut buf = vec![0; part];
            (0..100 / part as u64).try_for_each(|i| stage.read_exact_at(&mut buf, at + i * part as u64))
        };
        pass(first, 10).unwrap();
        pass(second, 100).unwrap();
        write_all_at(&stage.file, &[9], second + 57).unwrap();
        pass(first, 25).unwrap();
        assert_eq!(pass(second, 10).unwrap_err().kind(), std::io::ErrorKind::InvalidData, "in parts");
        assert_eq!(pass(second, 100).unwrap_err().kind(), std::io::ErrorKind::InvalidData, "whole");
        let mut part = [0; 10];
        stage.read_exact_at(&mut part, second + 50).unwrap(); // a read that starts mid-append is not a pass
        drop(stage);
        let _ = std::fs::remove_dir(&dir);
    }
}
