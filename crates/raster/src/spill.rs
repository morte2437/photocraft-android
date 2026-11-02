//! Scratch disk: tile pixels beyond a memory budget move to a file and come back on access.
//!
//! Every [`Tile`](crate::Tile) keeps its bytes in a [`Cell`]: resident (`Some`) or on disk
//! (`None`, with its [`Extent`] in the scratch file). Readers take the cell's read lock, so a
//! tile in use is never evicted; a spilled tile is read back (LZ4 + CRC-checked) by whoever
//! touches it first.
//!
//! - **Accounting.** [`stats`] counts the bytes of every live tile that is resident, whether or
//!   not a scratch disk is configured.
//! - **Eviction** is the clock ("second chance") algorithm over a registry of weak cell
//!   pointers: a tile touched since the hand last passed keeps its place once. A background
//!   thread evicts down to 90 % of the budget; a thread that pushes memory past 125 % helps.
//! - **Clean tiles are free to drop.** A tile written once and not changed since keeps its
//!   extent, so evicting it again only frees memory; changing it frees the extent.
//! - **Off by default.** Until [`configure`] sets a directory and a budget nothing is evicted
//!   (tests, the web build).
//!
//! Failures never panic: a scratch write that fails keeps the tile in memory and turns spilling
//! off (reported in [`SpillStats::last_error`]). A failed read retains its disk extent and
//! poisons the tile: edits and saves check the health of the document they publish, plus
//! [`check_since`] for failures that came and went during the operation. The
//! infallible display API uses a temporary placeholder, never replacement pixel storage.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};

use crate::scratch_file::{Extent, ScratchFile};

/// One tile's bytes and where they live.
pub(crate) struct Cell {
    data: RwLock<Option<Vec<u8>>>,
    /// Byte length of the tile (fixed for its lifetime).
    len: usize,
    error: Mutex<Option<String>>,
    /// A copy on disk that matches `data` (kept while the tile is unchanged).
    slot: Mutex<Option<(Arc<ScratchFile>, Extent)>>,
    /// Touched since the clock hand last passed.
    referenced: AtomicBool,
}

impl Cell {
    pub(crate) fn new(bytes: Vec<u8>) -> Arc<Cell> {
        let len = bytes.len();
        let cell = Arc::new(Cell { data: RwLock::new(Some(bytes)), len, error: Mutex::new(None), slot: Mutex::new(None), referenced: AtomicBool::new(true) });
        M.live.fetch_add(1, Ordering::Relaxed);
        M.resident.fetch_add(len, Ordering::Relaxed);
        M.register(&cell);
        maybe_evict();
        cell
    }

    pub(crate) fn len(&self) -> usize {
        self.len
    }

    pub(crate) fn read_error(&self) -> Option<String> {
        self.error.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }

    fn set_error(&self, error: Option<String>) {
        let mut old = self.error.lock().unwrap_or_else(PoisonError::into_inner);
        match (old.is_some(), error.is_some()) {
            (false, true) => {
                M.failed.fetch_add(1, Ordering::Release);
                M.read_errors.fetch_add(1, Ordering::Release);
            }
            (true, false) => {
                // Invalidate computations that started while pixels were unreadable too.
                // Publish this before reporting the tile healthy to commit guards.
                M.read_errors.fetch_add(1, Ordering::Release);
                M.failed.fetch_sub(1, Ordering::Release);
            }
            _ => {}
        }
        *old = error;
    }

    #[inline]
    fn touch(&self) {
        if !self.referenced.load(Ordering::Relaxed) {
            self.referenced.store(true, Ordering::Relaxed);
        }
    }

    /// Read access; reads the tile back from the scratch disk if it was evicted.
    pub(crate) fn read(&self) -> RwLockReadGuard<'_, Option<Vec<u8>>> {
        let mut loaded_pixels = false;
        loop {
            self.touch();
            let g = self.data.read().unwrap_or_else(PoisonError::into_inner);
            if g.is_some() {
                if loaded_pixels {
                    // Pin the tile before eviction: even a budget smaller than one tile
                    // must not repeatedly evict the pixels this reader just loaded.
                    maybe_evict();
                }
                return g;
            }
            drop(g);
            let mut w = self.data.write().unwrap_or_else(PoisonError::into_inner);
            let loaded = self.load(&mut w).is_ok();
            drop(w);
            if !loaded {
                return self.data.read().unwrap_or_else(PoisonError::into_inner);
            }
            loaded_pixels = true;
        }
    }

    /// Write access: resident, and any disk copy is released (it no longer matches).
    pub(crate) fn write(&self) -> RwLockWriteGuard<'_, Option<Vec<u8>>> {
        self.touch();
        let mut w = self.data.write().unwrap_or_else(PoisonError::into_inner);
        let loaded = w.is_none();
        if self.load(&mut w).is_err() {
            // Keep the only original copy. The write guard's temporary placeholder is
            // discarded, and the transaction's integrity check refuses to commit it.
            return w;
        }
        if let Some((file, extent)) = self.slot.lock().unwrap_or_else(PoisonError::into_inner).take() {
            file.free(extent);
        }
        if loaded {
            // Evicting here could only pick other tiles (this one is locked), so it is safe, and
            // a long run of writes to spilled tiles must not outgrow the budget.
            maybe_evict();
        }
        w
    }

    /// Fill `w` from the scratch disk if it is not resident (`w` is this cell's write guard).
    fn load(&self, w: &mut Option<Vec<u8>>) -> Result<(), String> {
        if w.is_some() {
            return Ok(());
        }
        let slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner).clone();
        let bytes = match slot {
            Some((file, extent)) => {
                let mut packed = Vec::new();
                match file.read_into(extent, &mut packed).map_err(|e| e.to_string()).and_then(|()| unpack(&packed, self.len)) {
                    Ok(b) => b,
                    Err(e) => {
                        let error = format!("scratch read failed ({e}); editing and saving are blocked until the tile is readable");
                        self.set_error(Some(error.clone()));
                        M.error(error.clone());
                        return Err(error);
                    }
                }
            }
            None => {
                let error = "a spilled tile has no scratch copy; editing and saving are blocked".to_string();
                self.set_error(Some(error.clone()));
                M.error(error.clone());
                return Err(error);
            }
        };
        *w = Some(bytes);
        self.set_error(None);
        M.resident.fetch_add(self.len, Ordering::Relaxed);
        M.reloads.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// Move this tile to disk if nobody is using it. Returns the bytes freed.
    fn evict(&self, file: &Arc<ScratchFile>) -> Result<usize, String> {
        let Ok(mut w) = self.data.try_write() else { return Ok(0) };
        let Some(bytes) = w.as_ref() else { return Ok(0) };
        let mut slot = self.slot.lock().unwrap_or_else(PoisonError::into_inner);
        if slot.is_none() {
            let packed = lz4_flex::block::compress(bytes);
            let extent = file.write(&packed).map_err(|e| format!("scratch write to {} failed: {e}", file.path().display()))?;
            M.written.fetch_add(packed.len() as u64, Ordering::Relaxed);
            *slot = Some((file.clone(), extent));
        }
        drop(slot);
        *w = None;
        M.resident.fetch_sub(self.len, Ordering::Relaxed);
        M.evictions.fetch_add(1, Ordering::Relaxed);
        Ok(self.len)
    }
}

impl Drop for Cell {
    fn drop(&mut self) {
        if self.error.get_mut().unwrap_or_else(PoisonError::into_inner).is_some() {
            M.read_errors.fetch_add(1, Ordering::Release);
            M.failed.fetch_sub(1, Ordering::Release);
        }
        if let Some(bytes) = self.data.get_mut().unwrap_or_else(PoisonError::into_inner).take() {
            M.resident.fetch_sub(self.len, Ordering::Relaxed);
            // The next tile of this size reuses the buffer, as main's pool expects.
            crate::scratch::give(bytes);
        }
        if let Some((file, extent)) = self.slot.get_mut().unwrap_or_else(PoisonError::into_inner).take() {
            file.free(extent);
        }
        M.live.fetch_sub(1, Ordering::Relaxed);
    }
}

fn unpack(packed: &[u8], len: usize) -> Result<Vec<u8>, String> {
    let mut bytes = crate::scratch::take_exact(len, 0u8);
    match lz4_flex::block::decompress_into(packed, &mut bytes) {
        Ok(n) if n == len => Ok(bytes),
        Ok(n) => {
            crate::scratch::give(bytes);
            Err(format!("tile decoded to {n} bytes, expected {len}"))
        }
        Err(e) => {
            crate::scratch::give(bytes);
            Err(e.to_string())
        }
    }
}

/// Process-wide state. Plain statics: everything here is const-constructible.
struct Manager {
    failed: AtomicUsize,
    read_errors: AtomicU64,
    resident: AtomicUsize,
    live: AtomicUsize,
    /// Pixel bytes to keep resident; 0 = spilling off.
    budget: AtomicUsize,
    registry: Mutex<Registry>,
    config: Mutex<Config>,
    evictions: AtomicU64,
    reloads: AtomicU64,
    written: AtomicU64,
}

struct Registry {
    cells: Vec<Weak<Cell>>,
    hand: usize,
}

struct Config {
    dir: Option<PathBuf>,
    /// Opened at configuration time; startup also reaps abandoned Unix files.
    file: Option<Arc<ScratchFile>>,
    last_error: Option<String>,
}

static M: Manager = Manager {
    failed: AtomicUsize::new(0),
    read_errors: AtomicU64::new(0),
    resident: AtomicUsize::new(0),
    live: AtomicUsize::new(0),
    budget: AtomicUsize::new(0),
    registry: Mutex::new(Registry { cells: Vec::new(), hand: 0 }),
    config: Mutex::new(Config { dir: None, file: None, last_error: None }),
    evictions: AtomicU64::new(0),
    reloads: AtomicU64::new(0),
    written: AtomicU64::new(0),
};

impl Manager {
    fn register(&self, cell: &Arc<Cell>) {
        let mut r = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
        r.cells.push(Arc::downgrade(cell));
        // Drop the pointers of freed tiles once they outnumber the live ones.
        if r.cells.len() > self.live.load(Ordering::Relaxed).saturating_mul(2).saturating_add(4096) {
            r.cells.retain(|w| w.strong_count() > 0);
            r.hand = 0;
        }
    }

    fn config(&self) -> std::sync::MutexGuard<'_, Config> {
        self.config.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn error(&self, msg: String) {
        log_error(&msg);
        self.config().last_error = Some(msg);
    }

    /// The scratch file, created on first use; `None` (and spilling off) if it can't be.
    fn file(&self) -> Option<Arc<ScratchFile>> {
        let mut c = self.config();
        if let Some(f) = &c.file {
            return Some(f.clone());
        }
        let dir = c.dir.clone()?;
        match ScratchFile::create(&dir) {
            Ok(f) => {
                let f = Arc::new(f);
                c.file = Some(f.clone());
                Some(f)
            }
            Err(e) => {
                let msg = format!("could not create a scratch file in {}: {e}; spilling is off", dir.display());
                log_error(&msg);
                c.last_error = Some(msg);
                c.dir = None;
                self.budget.store(0, Ordering::Relaxed);
                None
            }
        }
    }

    /// Evict until resident memory is at most `target` or `max_scan` cells were looked at.
    /// Returns the bytes freed.
    fn sweep(&self, target: usize, max_scan: usize) -> usize {
        let Some(file) = self.file() else { return 0 };
        let mut freed = 0;
        let mut scanned = 0;
        while self.resident.load(Ordering::Relaxed) > target && scanned < max_scan {
            // Pick victims under the registry lock (cheap), evict them after releasing it.
            let mut victims = Vec::with_capacity(64);
            {
                let mut r = self.registry.lock().unwrap_or_else(PoisonError::into_inner);
                let mut i = 0;
                while victims.len() < 64 && i < 4096 && !r.cells.is_empty() {
                    if r.hand >= r.cells.len() {
                        r.hand = 0;
                    }
                    let h = r.hand;
                    i += 1;
                    match r.cells.get(h).and_then(Weak::upgrade) {
                        None => {
                            r.cells.swap_remove(h);
                            continue;
                        }
                        Some(cell) => {
                            r.hand += 1;
                            if cell.referenced.swap(false, Ordering::Relaxed) {
                                continue;
                            }
                            if cell.data.try_read().is_ok_and(|d| d.is_some()) {
                                victims.push(cell);
                            }
                        }
                    }
                }
                scanned += i;
                if i == 0 {
                    break;
                }
            }
            for cell in victims {
                match cell.evict(&file) {
                    Ok(n) => freed += n,
                    Err(e) => {
                        // Disk full or gone: keep everything in memory from now on.
                        self.budget.store(0, Ordering::Relaxed);
                        self.error(format!("{e}; spilling is off"));
                        return freed;
                    }
                }
            }
        }
        freed
    }
}

/// Called after resident memory grew.
fn maybe_evict() {
    let budget = M.budget.load(Ordering::Relaxed);
    if budget == 0 {
        return;
    }
    let resident = M.resident.load(Ordering::Relaxed);
    if resident <= budget {
        return;
    }
    let worker = MAINTENANCE.load(Ordering::Acquire) > 0;
    // Far over budget (or no worker thread): this thread helps.
    if !worker || resident > budget.saturating_add(budget / 4) {
        M.sweep(low_water(budget), 1 << 16);
    }
}

fn low_water(budget: usize) -> usize {
    budget / 10 * 9
}

// The engine owns and joins the worker. Standalone raster consumers keep synchronous
// eviction; this crate never starts a background thread or retains its owner.
static MAINTENANCE: AtomicUsize = AtomicUsize::new(0);

/// Register an externally owned residency worker. Keep this guard until that worker exits.
pub struct Maintenance;

pub fn background_maintenance() -> Maintenance {
    MAINTENANCE.fetch_add(1, Ordering::AcqRel);
    Maintenance
}

impl Drop for Maintenance {
    fn drop(&mut self) {
        MAINTENANCE.fetch_sub(1, Ordering::AcqRel);
    }
}

/// One bounded clock pass. The caller owns scheduling and shutdown.
pub fn maintain_budget() -> usize {
    let budget = M.budget.load(Ordering::Relaxed);
    if budget == 0 || M.resident.load(Ordering::Relaxed) <= budget {
        return 0;
    }
    M.sweep(low_water(budget), 4096)
}

/// Turn the scratch disk on (`dir` and a non-zero `budget_bytes`) or off. Tiles already on disk
/// stay readable either way. Changing the directory starts a new scratch file there; the old one
/// is deleted once its last tile is freed or changed.
pub fn set_budget(budget_bytes: usize) {
    if enabled() {
        M.budget.store(budget_bytes.max(1), Ordering::Relaxed);
        maybe_evict();
    }
}

pub fn configure(dir: Option<&Path>, budget_bytes: usize) -> Result<(), String> {
    let mut c = M.config();
    let dir = dir.filter(|_| budget_bytes > 0).map(Path::to_path_buf);
    if let Some(d) = &dir
        && let Err(e) = std::fs::create_dir_all(d)
    {
        let msg = format!("scratch disk {}: {e}; spilling is off", d.display());
        c.dir = None;
        c.file = None;
        c.last_error = Some(msg.clone());
        M.budget.store(0, Ordering::Relaxed);
        return Err(msg);
    }
    if dir != c.dir || (dir.is_some() && c.file.is_none()) {
        c.file = match dir.as_deref().map(ScratchFile::create).transpose() {
            Ok(file) => file.map(Arc::new),
            Err(e) => {
                let msg = format!("scratch disk: {e}; spilling is off");
                c.dir = None;
                c.last_error = Some(msg.clone());
                M.budget.store(0, Ordering::Relaxed);
                return Err(msg);
            }
        };
    }
    c.dir = dir.clone();
    c.last_error = None;
    M.budget.store(if dir.is_some() { budget_bytes } else { 0 }, Ordering::Relaxed);
    drop(c);
    maybe_evict();
    Ok(())
}

/// Where the tile memory stands.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SpillStats {
    /// Pixel bytes of live tiles in memory.
    pub resident_bytes: usize,
    pub live_tiles: usize,
    /// 0 = spilling off.
    pub budget_bytes: usize,
    pub dir: Option<PathBuf>,
    /// Bytes allocated in the current scratch file.
    pub scratch_bytes: u64,
    pub evictions: u64,
    pub reloads: u64,
    /// Compressed bytes written to scratch so far.
    pub written_bytes: u64,
    pub last_error: Option<String>,
}

pub fn stats() -> SpillStats {
    let c = M.config();
    SpillStats {
        resident_bytes: M.resident.load(Ordering::Relaxed),
        live_tiles: M.live.load(Ordering::Relaxed),
        budget_bytes: M.budget.load(Ordering::Relaxed),
        dir: c.dir.clone(),
        scratch_bytes: c.file.as_ref().map_or(0, |f| f.used()),
        evictions: M.evictions.load(Ordering::Relaxed),
        reloads: M.reloads.load(Ordering::Relaxed),
        written_bytes: M.written.load(Ordering::Relaxed),
        last_error: c.last_error.clone(),
    }
}

/// Last storage error, without scanning the scratch allocator for UI status updates.
pub fn last_error() -> Option<String> {
    M.config().last_error.clone()
}

/// Whether spilling is on (tiles beyond the budget go to disk).
pub fn enabled() -> bool {
    M.budget.load(Ordering::Relaxed) > 0
}

/// Whether any live tile in the process is currently unreadable (cheap; the fast path of
/// every per-document health check).
pub fn any_failed() -> bool {
    M.failed.load(Ordering::Acquire) > 0
}

/// Process-wide status for the UI: some live tile, in any document, is unreadable. Commit
/// guards check only the pixels they publish ([`Tile::scratch_health`](crate::Tile::scratch_health)
/// and the document's health) so one document's failure does not block the others.
pub fn check_integrity() -> Result<(), String> {
    if any_failed() {
        Err(M.config().last_error.clone().unwrap_or_else(|| "scratch pixels are unreadable; editing and saving are blocked".into()))
    } else {
        Ok(())
    }
}

/// Capture before an edit/import/save or derived cache computation. A tile that fails or
/// recovers while the operation runs might have fed it placeholder pixels and then been
/// dropped or repaired; its result must still never be committed as real pixels. Both failure
/// and recovery change this generation. Tiles that were already unreadable when the operation
/// began do not change it: callers also check the health of the pixels they read.
pub fn read_error_generation() -> u64 {
    M.read_errors.load(Ordering::Acquire)
}

/// Refuse a result if any tile failed or recovered since `generation` was captured. A retry
/// (capturing a new generation) succeeds once the transition is over.
pub fn check_since(generation: u64) -> Result<(), String> {
    if generation == read_error_generation() {
        Ok(())
    } else {
        Err(last_error().unwrap_or_else(|| "scratch read failed during this operation; the result was discarded".into()))
    }
}

/// A second handle to the current scratch file, for tests that damage or restore its bytes.
/// The Unix file is unlinked, so it cannot be reopened by path. All scratch I/O is positioned,
/// so the shared file offset of a duplicated handle does not affect it.
#[doc(hidden)]
pub fn scratch_handle_for_tests() -> Option<std::fs::File> {
    M.config().file.as_ref().and_then(|f| f.duplicate_handle().ok())
}

/// Evict down to the budget now, on this thread (tests, and before a measurement).
pub fn settle() {
    let budget = M.budget.load(Ordering::Relaxed);
    // Stops when nothing more can be evicted (every remaining tile is in use).
    while budget > 0 && M.resident.load(Ordering::Relaxed) > budget {
        if M.sweep(budget, usize::MAX) == 0 {
            break;
        }
    }
}

fn log_error(msg: &str) {
    #[cfg(not(target_arch = "wasm32"))]
    eprintln!("photocraft: {msg}");
    #[cfg(target_arch = "wasm32")]
    let _ = msg;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failed_read_retains_backing_and_blocks_commit_until_repaired() {
        let dir = std::env::temp_dir().join(format!("photocraft-read-failure-{}", std::process::id()));
        let file = Arc::new(ScratchFile::create(&dir).unwrap());
        let mut tile = crate::Tile { cell: Cell::new(vec![73; 1024]) };
        let generation = read_error_generation();
        assert_eq!(tile.cell.evict(&file).unwrap(), 1024);
        let good = tile.cell.slot.lock().unwrap().as_ref().unwrap().1;
        tile.cell.slot.lock().unwrap().as_mut().unwrap().1.crc32 ^= 1;

        assert!(tile.try_bytes().is_err());
        let failed_generation = read_error_generation();
        assert!(tile.cell.data.read().unwrap().is_none(), "placeholder must never become stored pixels");
        assert!(check_integrity().is_err());
        assert!(tile.scratch_health().is_err(), "the tile reports its own failure");
        assert!(crate::Tile::from_vec(vec![1; 16]).scratch_health().is_ok(), "other tiles stay healthy");
        assert!(tile.try_bytes().is_err());
        assert!(check_since(failed_generation).is_ok(), "re-reading a tile that already failed is not a new transition");
        tile.bytes_mut()[0] = 9;
        assert!(tile.cell.data.read().unwrap().is_none());
        assert!(tile.cell.slot.lock().unwrap().is_some(), "a failed mutation must preserve the original extent");
        let snapshot = tile.clone();
        assert!(Arc::ptr_eq(&tile.cell, &snapshot.cell));
        configure(None, 0).unwrap();
        assert!(check_integrity().is_err(), "changing preferences must not clear unreadable pixels");

        tile.cell.slot.lock().unwrap().as_mut().unwrap().1 = good;
        assert_eq!(&*tile.try_bytes().unwrap(), &[73; 1024]);
        assert!(check_integrity().is_ok());
        assert!(tile.scratch_health().is_ok());
        assert!(check_since(generation).is_err(), "repairing or dropping a failed tile must not validate an earlier result");
        assert!(check_since(failed_generation).is_err(), "recovery must reject a computation begun during the failure");
        assert_eq!(tile.cell.evict(&file).unwrap(), 1024);
        tile.cell.slot.lock().unwrap().as_mut().unwrap().1.crc32 ^= 1;
        assert!(tile.try_bytes().is_err());
        let dropped_generation = read_error_generation();
        drop((snapshot, tile, file));
        assert!(check_integrity().is_ok());
        assert!(check_since(dropped_generation).is_err(), "dropping the failed tile must also reject its placeholder result");
        let _ = std::fs::remove_dir(&dir);
    }
}
