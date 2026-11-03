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
//! Failures never panic: a scratch write or file creation that fails, or a volume running low
//! on free space, keeps the tile in memory and pauses spilling (reported in
//! [`SpillStats::last_error`]); it is retried with exponential backoff and resumes on its own.
//! [`epoch`] tells owners of memory caps (history) to re-apply them meanwhile. A failed read retains its disk extent and
//! poisons the tile: edits and saves check the health of the document they publish, plus
//! [`check_since`] for failures that came and went during the operation. The
//! infallible display API uses a temporary placeholder, never replacement pixel storage.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, PoisonError, RwLock, RwLockReadGuard, RwLockWriteGuard, Weak};
use std::time::{Duration, Instant};

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
    /// Pixel bytes to keep resident; 0 = spilling off or paused.
    budget: AtomicUsize,
    /// Spilling is paused after a write, create or free-space failure ([`Config::paused`]).
    paused: AtomicBool,
    /// Changes whenever spilling turns on or off, so owners of memory caps re-apply them.
    epoch: AtomicU64,
    /// Scratch bytes allowed before the volume would run low or the size cap is reached.
    space_limit: AtomicU64,
    /// Re-read the volume's free space once the scratch file grows past this.
    next_space_check: AtomicU64,
    /// A sweep wrote successfully since the last resume: reset the retry backoff.
    recovering: AtomicBool,
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
    /// The budget [`configure`] / [`set_budget`] asked for; restored when a pause ends.
    budget: usize,
    paused: Option<Pause>,
    /// Delay before the next retry; doubles per failed retry up to [`RETRY_MAX`].
    backoff: Duration,
    /// Explicit scratch size limit ([`set_size_cap`]); the free-space reserve always applies.
    size_cap: Option<u64>,
}

struct Pause {
    message: String,
    retry_at: Instant,
}

const RETRY_FIRST: Duration = Duration::from_secs(1);
const RETRY_MAX: Duration = Duration::from_secs(60);
/// Re-check free space after this much scratch growth.
const SPACE_CHECK_EVERY: u64 = 64 << 20;
/// Leave at least max(1 GiB, 5 % of the volume) free for the system and other programs.
const RESERVE_MIN: u64 = 1 << 30;

static M: Manager = Manager {
    failed: AtomicUsize::new(0),
    read_errors: AtomicU64::new(0),
    resident: AtomicUsize::new(0),
    live: AtomicUsize::new(0),
    budget: AtomicUsize::new(0),
    paused: AtomicBool::new(false),
    epoch: AtomicU64::new(0),
    space_limit: AtomicU64::new(u64::MAX),
    next_space_check: AtomicU64::new(0),
    recovering: AtomicBool::new(false),
    registry: Mutex::new(Registry { cells: Vec::new(), hand: 0 }),
    config: Mutex::new(Config { dir: None, file: None, last_error: None, budget: 0, paused: None, backoff: RETRY_FIRST, size_cap: None }),
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

    /// Set the active budget, bumping [`epoch`] when spilling turns on or off.
    fn set_active(&self, budget: usize) {
        let old = self.budget.swap(budget, Ordering::AcqRel);
        if (old > 0) != (budget > 0) {
            self.epoch.fetch_add(1, Ordering::AcqRel);
        }
    }

    /// Pause spilling after a failure: every tile stays in memory (owners re-apply their
    /// memory caps through [`epoch`]) and a retry is scheduled with exponential backoff.
    fn pause(&self, c: &mut Config, reason: &str) {
        let wait = c.backoff;
        c.backoff = c.backoff.saturating_mul(2).min(RETRY_MAX);
        let message = format!("{reason}; spilling is paused (tiles stay in memory), retrying in {} s", wait.as_secs());
        log_error(&message);
        c.last_error = Some(message.clone());
        c.paused = Some(Pause { message, retry_at: Instant::now().checked_add(wait).unwrap_or_else(Instant::now) });
        self.recovering.store(false, Ordering::Relaxed);
        self.paused.store(true, Ordering::Release);
        self.set_active(0);
    }

    /// End a pause: the directory is usable and has room again.
    fn resume(&self, c: &mut Config) {
        if let Some(p) = c.paused.take()
            && c.last_error.as_deref() == Some(p.message.as_str())
        {
            c.last_error = None;
        }
        self.paused.store(false, Ordering::Release);
        self.recovering.store(true, Ordering::Relaxed);
        self.set_active(c.budget);
    }

    /// Open the scratch file and check its volume's room; pause (and say why) if either fails.
    /// The caller holds the config lock. Returns whether spilling can go ahead.
    fn prepare(&self, c: &mut Config) -> bool {
        let Some(dir) = c.dir.clone() else { return false };
        if c.file.is_none() {
            match std::fs::create_dir_all(&dir).and_then(|()| ScratchFile::create(&dir)) {
                Ok(f) => c.file = Some(Arc::new(f)),
                Err(e) => {
                    self.pause(c, &format!("could not create a scratch file in {}: {e}", dir.display()));
                    return false;
                }
            }
        }
        let used = c.file.as_ref().map_or(0, |f| f.used());
        refresh_space(c, &dir, used);
        if used >= self.space_limit.load(Ordering::Relaxed) {
            let msg = no_room(&dir, c.size_cap);
            self.pause(c, &msg);
            return false;
        }
        true
    }

    /// Retry a paused scratch disk once its backoff has elapsed. Cheap when not paused.
    fn retry(&self) {
        if !self.paused.load(Ordering::Acquire) {
            return;
        }
        let mut c = match self.config.try_lock() {
            Ok(c) => c,
            Err(std::sync::TryLockError::Poisoned(p)) => p.into_inner(),
            // Someone else is configuring or retrying.
            Err(std::sync::TryLockError::WouldBlock) => return,
        };
        let due = c.paused.as_ref().is_some_and(|p| Instant::now() >= p.retry_at);
        if !due {
            return;
        }
        if c.dir.is_none() || c.budget == 0 {
            c.paused = None;
            self.paused.store(false, Ordering::Release);
            return;
        }
        if self.prepare(&mut c) {
            self.resume(&mut c);
        }
    }

    /// The scratch file, if spilling is on and the volume still has room.
    fn file(&self) -> Option<Arc<ScratchFile>> {
        let mut c = self.config();
        if self.budget.load(Ordering::Relaxed) == 0 {
            return None;
        }
        if c.file.is_none() && !self.prepare(&mut c) {
            return None;
        }
        c.file.clone()
    }

    /// Whether the scratch file may grow further; re-reads free space every
    /// [`SPACE_CHECK_EVERY`] bytes and pauses spilling when the volume runs low.
    fn has_room(&self, file: &ScratchFile) -> bool {
        let used = file.used();
        if used < self.next_space_check.load(Ordering::Relaxed) && used < self.space_limit.load(Ordering::Relaxed) {
            return true;
        }
        let mut c = self.config();
        let Some(dir) = c.dir.clone() else { return false };
        refresh_space(&c, &dir, used);
        if used < self.space_limit.load(Ordering::Relaxed) {
            return true;
        }
        if c.paused.is_none() {
            let msg = no_room(&dir, c.size_cap);
            self.pause(&mut c, &msg);
        }
        false
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
                if !self.has_room(&file) {
                    return freed;
                }
                match cell.evict(&file) {
                    Ok(n) => freed += n,
                    Err(e) => {
                        // Disk full or gone: keep tiles in memory until a retry succeeds.
                        let mut c = self.config();
                        if c.paused.is_none() {
                            self.pause(&mut c, &e);
                        }
                        return freed;
                    }
                }
            }
        }
        if freed > 0 && self.recovering.swap(false, Ordering::Relaxed) {
            self.config().backoff = RETRY_FIRST;
        }
        freed
    }
}

/// Free space on the volume holding `dir`, minus the reserve, sets how far the file may grow.
fn refresh_space(c: &Config, dir: &Path, used: u64) {
    let room = free_space(dir).map_or(u64::MAX, |(available, total)| used.saturating_add(available.saturating_sub(RESERVE_MIN.max(total / 20))));
    let limit = c.size_cap.map_or(room, |cap| cap.min(room));
    M.space_limit.store(limit, Ordering::Relaxed);
    M.next_space_check.store(used.saturating_add(SPACE_CHECK_EVERY), Ordering::Relaxed);
}

fn no_room(dir: &Path, cap: Option<u64>) -> String {
    match cap {
        Some(cap) => format!("scratch disk {} reached its size limit ({} MB) or is low on free space", dir.display(), cap >> 20),
        None => format!("scratch disk {} is low on free space", dir.display()),
    }
}

/// (available, total) bytes of the volume holding `dir`: the mount point that is its longest
/// prefix. `None` if unknown, which disables only the free-space check, not spilling.
#[cfg(not(target_arch = "wasm32"))]
fn free_space(dir: &Path) -> Option<(u64, u64)> {
    let dir = std::fs::canonicalize(dir).ok()?;
    let disks = sysinfo::Disks::new_with_refreshed_list_specifics(sysinfo::DiskRefreshKind::nothing().with_storage()).ok()?;
    disks.iter().filter(|d| dir.starts_with(d.mount_point())).max_by_key(|d| d.mount_point().as_os_str().len()).map(|d| (d.available_space(), d.total_space()))
}

#[cfg(target_arch = "wasm32")]
fn free_space(_dir: &Path) -> Option<(u64, u64)> {
    None
}

/// Called after resident memory grew.
fn maybe_evict() {
    let budget = M.budget.load(Ordering::Relaxed);
    if budget == 0 {
        M.retry();
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

/// One bounded clock pass, plus a due retry of a paused scratch disk. The caller owns
/// scheduling and shutdown.
pub fn maintain_budget() -> usize {
    M.retry();
    let budget = M.budget.load(Ordering::Relaxed);
    if budget == 0 || M.resident.load(Ordering::Relaxed) <= budget {
        return 0;
    }
    M.sweep(low_water(budget), 4096)
}

/// Change the memory budget of a configured scratch disk (the memory limit moved). A paused
/// disk keeps its pause and resumes with this budget.
pub fn set_budget(budget_bytes: usize) {
    let mut c = M.config();
    if c.dir.is_none() || c.budget == 0 {
        return;
    }
    c.budget = budget_bytes.max(1);
    if c.paused.is_none() {
        M.set_active(c.budget);
    }
    drop(c);
    maybe_evict();
}

/// Limit how large the scratch file may grow (`None`: only the free-space reserve applies).
/// Reaching the limit pauses spilling like a full disk; changing it retries a paused disk now.
pub fn set_size_cap(cap: Option<u64>) {
    let mut c = M.config();
    if c.size_cap == cap {
        return;
    }
    c.size_cap = cap;
    M.next_space_check.store(0, Ordering::Relaxed);
    if let Some(p) = &mut c.paused {
        p.retry_at = Instant::now();
        c.backoff = RETRY_FIRST;
    }
    drop(c);
    M.retry();
}

/// Turn the scratch disk on (`dir` and a non-zero `budget_bytes`) or off. Tiles already on disk
/// stay readable either way. Changing the directory starts a new scratch file there; the old one
/// is deleted once its last tile is freed or changed. A directory that can't be used is kept and
/// retried with backoff (spilling stays paused meanwhile); the error is returned and reported.
pub fn configure(dir: Option<&Path>, budget_bytes: usize) -> Result<(), String> {
    let mut c = M.config();
    let dir = dir.filter(|_| budget_bytes > 0).map(Path::to_path_buf);
    if dir != c.dir {
        c.file = None;
    }
    c.dir = dir.clone();
    c.budget = if dir.is_some() { budget_bytes } else { 0 };
    c.backoff = RETRY_FIRST;
    c.paused = None;
    c.last_error = None;
    M.paused.store(false, Ordering::Release);
    M.next_space_check.store(0, Ordering::Relaxed);
    if dir.is_some() && !M.prepare(&mut c) {
        return Err(c.last_error.clone().unwrap_or_default());
    }
    M.set_active(c.budget);
    drop(c);
    maybe_evict();
    Ok(())
}

/// Changes whenever spilling turns on or off, including a pause after a disk error and the
/// resume after it. Owners of memory caps that depend on [`enabled`] re-apply them on a change.
pub fn epoch() -> u64 {
    M.epoch.load(Ordering::Acquire)
}

/// Whether spilling is paused after a scratch disk failure (it retries on its own).
pub fn paused() -> bool {
    M.paused.load(Ordering::Acquire)
}

/// Retry a paused scratch disk on the next maintenance pass instead of after its backoff.
#[doc(hidden)]
pub fn retry_now_for_tests() {
    if let Some(p) = &mut M.config().paused {
        p.retry_at = Instant::now();
    }
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
    /// Spilling is paused after a scratch disk failure and will be retried.
    pub paused: bool,
    /// Scratch file size limit ([`set_size_cap`]); `None` = only the free-space reserve.
    pub size_cap_bytes: Option<u64>,
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
        paused: c.paused.is_some(),
        size_cap_bytes: c.size_cap,
    }
}

/// Last storage error, without scanning the scratch allocator for UI status updates.
pub fn last_error() -> Option<String> {
    M.config().last_error.clone()
}

/// Uncompressed tile bytes the scratch disk can still take before it reaches its size limit
/// ([`set_size_cap`]) or the volume's free-space reserve, so an import can admit tile pixels
/// beyond physical RAM. `None` while spilling is off or paused, and when neither the free space
/// nor a size limit is known (nothing to promise). Tiles are written compressed; this counts
/// them uncompressed, less 1/64 for the allocator's block rounding, so it never over-promises.
pub fn scratch_capacity() -> Option<u64> {
    if !enabled() || paused() {
        return None;
    }
    let (dir, used, cap) = {
        let c = M.config();
        (c.dir.clone()?, c.file.as_ref().map_or(0, |f| f.used()), c.size_cap)
    };
    let room = free_space(&dir).map(|(available, total)| available.saturating_sub(RESERVE_MIN.max(total / 20)));
    let room = match (room, cap) {
        (Some(room), Some(cap)) => room.min(cap.saturating_sub(used)),
        (Some(room), None) => room,
        (None, Some(cap)) => cap.saturating_sub(used),
        (None, None) => return None,
    };
    Some(room - room / 64)
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
