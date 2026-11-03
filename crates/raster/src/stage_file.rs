//! Encoded data an exporter parks on the scratch disk instead of in memory (#5333).
//!
//! A large PSD/PSB save encodes every channel before it writes a byte (so it can still be
//! cancelled, and its scratch reads are checked, before the file is replaced). Kept in memory,
//! those channels are a second copy of the document. A [`StageFile`] holds them on the scratch
//! disk instead: an append-only file made like the tile scratch file (private, unlinked on Unix,
//! delete-on-close on Windows), so nothing is left behind at any exit, and the space returns as
//! soon as the save drops it.

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
        Ok(Self { file, path, linked, end: Mutex::new(0) })
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
        Ok(offset)
    }

    /// Fills `buf` from `offset`, which must lie within what was appended.
    pub fn read_exact_at(&self, buf: &mut [u8], offset: u64) -> std::io::Result<()> {
        if offset.checked_add(buf.len() as u64).is_none_or(|end| end > self.len()) {
            return Err(std::io::ErrorKind::UnexpectedEof.into());
        }
        read_exact_at(&self.file, buf, offset)
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
}
