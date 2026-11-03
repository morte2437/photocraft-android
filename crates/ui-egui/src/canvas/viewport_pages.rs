//! Viewport pages for a document whose display textures don't fit the GPU canvas budget
//! ([`super::gpu_canvas::GpuCanvas::fits_canvas`], from the detected RAM and VRAM, so also with
//! GPU Memory on Auto). Instead of the whole-document texture set, the view's pages are rendered
//! on two background workers at the view's reduction and uploaded as their own GPU textures,
//! under [`stream_key`]: the work and the GPU memory follow what is on screen, not the document.
//!
//! Documents that fit keep the texture set and everything around it (the layer budget, deferred
//! refresh, background redraw). Without a GPU canvas the CPU canvas draws every document.
//!
//! Pages are derived data: an edit marks the pages its damage reaches dirty (the rest stay) and,
//! for a small one, re-renders just the texels it reaches into the page held; a
//! dirty page stays on screen until its replacement is uploaded, and work for pages that are no
//! longer wanted (a pan, a zoom, an edit) is cancelled.

use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

use photocraft_doc::Document;
use photocraft_engine::display_color::CanvasDisplay;
use photocraft_geom::Rect as DRect;

use super::textures::texture_buffer;
use super::{ViewXform, canvas_display, damage_since, display_doc, effect_reach, texture_key, visible_doc_rect};
use crate::PhotocraftApp;

/// Document pixels per page side at the page's reduction.
pub(crate) const PAGE_SIDE: u32 = 512;
/// The most pages a view asks for: coarser pages are used beyond it.
const MAX_PAGES: usize = 256;
/// Pages rendering at once, over all views and documents.
const MAX_JOBS: usize = 2;
/// Pages uploaded per frame, and the texel bytes they may take: staging stays bounded however
/// many pages finish at once.
const UPLOADS_PER_FRAME: usize = 2;
const UPLOAD_BYTES_PER_FRAME: u64 = 32 << 20;
/// The coarsest reduction (document pixels per texel).
const MAX_FACTOR: u32 = 1 << 16;

static ACTIVE: AtomicUsize = AtomicUsize::new(0);

/// The GPU canvas key of `doc`'s pages, apart from its texture set and the preview keys (bits 58
/// to 63 tag those: the filter preview, adjustment proxies, gradients and thumbnails).
pub(crate) fn stream_key(doc: u64) -> u64 {
    doc ^ (1u64 << 57)
}

/// A page: `factor` document pixels per texel, column `x` and row `y` of `PAGE_SIDE` texels.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct PageId {
    pub factor: u32,
    pub x: u32,
    pub y: u32,
}

impl PageId {
    /// The GPU canvas page id.
    pub fn id(self) -> u64 {
        (u64::from(self.factor.trailing_zeros()) << 56) | (u64::from(self.y) << 28) | u64::from(self.x)
    }

    /// The document pixels this page draws.
    pub fn core(self, size: [u32; 2]) -> DRect {
        let side = u64::from(PAGE_SIDE) * u64::from(self.factor);
        let (x, y) = (u64::from(self.x) * side, u64::from(self.y) * side);
        if self.factor == 0 || x >= u64::from(size[0]) || y >= u64::from(size[1]) {
            return DRect::EMPTY;
        }
        let end = |o: u64, n: u32| (o + side).min(u64::from(n)) as i32;
        DRect::new(x as i32, y as i32, end(x, size[0]), end(y, size[1]))
    }

    /// The document pixels it is rendered from: the core and one texel around it, so filtering
    /// at its edge matches its neighbour's.
    pub fn source(self, size: [u32; 2]) -> DRect {
        let core = self.core(size);
        if core.is_empty() {
            return core;
        }
        core.inflate(self.factor as i32).intersect(&DRect::new(0, 0, size[0] as i32, size[1] as i32))
    }
}

/// The page reduction for `zoom` (screen pixels per document pixel): the largest power of two at
/// which a texel still covers at most one screen pixel.
pub(crate) fn factor(zoom: f32) -> u32 {
    if !zoom.is_finite() || zoom <= 0.0 {
        return 1;
    }
    let mut k = 1u32;
    while k < MAX_FACTOR && zoom * (k * 2) as f32 <= 1.0 {
        k *= 2;
    }
    k
}

/// The part of `page` a change to `damage` reaches, widened to whole texels: rendered from just
/// that area, its texels are the ones the whole page has (reductions are globally aligned).
fn partial_region(page: PageId, size: [u32; 2], damage: DRect) -> DRect {
    let k = page.factor.max(1) as i32;
    let source = page.source(size);
    let d = damage.intersect(&source);
    if d.is_empty() {
        return DRect::EMPTY;
    }
    let up = |v: i32| v.div_euclid(k).saturating_add(i32::from(v.rem_euclid(k) != 0)).saturating_mul(k);
    DRect::new(d.x0.div_euclid(k) * k, d.y0.div_euclid(k) * k, up(d.x1), up(d.y1)).intersect(&source)
}

/// The pages covering `visible` of a `size` document at `zoom`, centre first: at most
/// `max_pages` (coarser when the view needs more).
pub(crate) fn pages(size: [u32; 2], visible: DRect, zoom: f32, max_pages: usize) -> Vec<PageId> {
    let visible = visible.intersect(&DRect::new(0, 0, size[0].min(i32::MAX as u32) as i32, size[1].min(i32::MAX as u32) as i32));
    if visible.is_empty() || max_pages == 0 {
        return Vec::new();
    }
    let mut k = factor(zoom);
    loop {
        let side = u64::from(PAGE_SIDE) * u64::from(k);
        let span = |a: i32, b: i32| (a as u64 / side, (b as u64).div_ceil(side));
        let ((x0, x1), (y0, y1)) = (span(visible.x0, visible.x1), span(visible.y0, visible.y1));
        if (x1 - x0) * (y1 - y0) <= max_pages.min(MAX_PAGES) as u64 {
            let mut out: Vec<PageId> = (y0..y1).flat_map(|y| (x0..x1).map(move |x| PageId { factor: k, x: x as u32, y: y as u32 })).collect();
            let (cx, cy) = ((x0 + x1) as i64, (y0 + y1) as i64);
            out.sort_by_key(|p| (i64::from(p.x) * 2 + 1 - cx).abs() + (i64::from(p.y) * 2 + 1 - cy).abs());
            return out;
        }
        if k >= MAX_FACTOR {
            return Vec::new();
        }
        k *= 2;
    }
}

/// What a stream's pages show: the document revision, its preview and display keys, its size.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Identity {
    pub revision: u64,
    /// The raw preview key ([`display_doc`]); `display` is folded in for [`damage_since`].
    pub preview: u64,
    pub display: u64,
    pub size: [u32; 2],
}

/// A page on the GPU.
#[derive(Debug)]
struct Page {
    /// Rendered before the latest edit that reached it: shown until its replacement arrives.
    dirty: bool,
    /// The exact reduction (not the quick preview).
    exact: bool,
    /// While dirty, the only document area that changed since it was uploaded (aligned to its
    /// texels): just that part is rendered again and written into the page. `None`: all of it.
    partial: Option<DRect>,
    /// Bumped whenever the page's content is invalidated: a result from older work is dropped.
    epoch: u64,
    touched: u64,
    bytes: u64,
}

struct Job {
    page: PageId,
    epoch: u64,
    cancel: Arc<AtomicBool>,
}

/// A finished page render.
pub(crate) struct Done {
    pub page: PageId,
    pub epoch: u64,
    pub exact: bool,
    /// Rendered for [`Page::partial`]: written into the page held, not a page of its own.
    pub region: Option<DRect>,
    pub result: Result<photocraft_compose::Buffer, String>,
}

/// The counters `inspect` reports.
#[derive(Clone, Debug, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Stats {
    pub wanted: usize,
    pub ready: usize,
    pub resident: usize,
    pub resident_bytes: u64,
    pub limit: u64,
    pub jobs: usize,
    pub rendered: u64,
    pub uploads: u64,
    pub cancelled: u64,
    pub dirtied: u64,
    /// Pages updated in part, by re-rendering only what an edit reached.
    pub partial: u64,
    pub evicted: u64,
    pub last_error: Option<String>,
}

/// One document's viewport pages ([`PhotocraftApp::streams`]).
pub(crate) struct Stream {
    pages: HashMap<PageId, Page>,
    /// Epochs of pages not (or no longer) on the GPU, so their results can still be checked.
    epochs: HashMap<PageId, u64>,
    jobs: Vec<Job>,
    ready: Vec<Done>,
    failed: HashSet<(PageId, u64)>,
    tx: mpsc::Sender<Done>,
    rx: mpsc::Receiver<Done>,
    pub seen: Option<Identity>,
    clock: u64,
    /// The page ids last given to the GPU canvas, to tell it only of changes.
    synced: Vec<u64>,
    health: (u64, u64),
    pub stats: Stats,
}

impl Default for Stream {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            pages: HashMap::new(),
            epochs: HashMap::new(),
            jobs: Vec::new(),
            ready: Vec::new(),
            failed: HashSet::new(),
            tx,
            rx,
            seen: None,
            clock: 0,
            synced: Vec::new(),
            health: (0, 0),
            stats: Stats::default(),
        }
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        for j in &self.jobs {
            j.cancel.store(true, Ordering::Relaxed);
        }
    }
}

impl Stream {
    fn epoch(&self, page: PageId) -> u64 {
        self.pages.get(&page).map_or_else(|| self.epochs.get(&page).copied().unwrap_or(0), |p| p.epoch)
    }

    fn bump(&mut self, page: PageId) {
        let e = self.epoch(page) + 1;
        match self.pages.get_mut(&page) {
            Some(p) => p.epoch = e,
            None => {
                self.epochs.insert(page, e);
            }
        }
    }

    /// The document changed to `now`: the pages whose source `damage` reaches (every page
    /// without it) are dirty, and work on them is cancelled. Nothing else is scanned: the cost
    /// is the pages held, not the document's layers or tiles.
    pub fn invalidate(&mut self, now: Identity, damage: Option<DRect>) {
        let size_changed = self.seen.is_some_and(|s| s.size != now.size);
        self.seen = Some(now);
        if size_changed {
            self.reset();
            return;
        }
        let hit = |p: PageId| damage.is_none_or(|d| !p.source(now.size).intersect(&d).is_empty());
        let pages: Vec<PageId> = self.pages.keys().copied().filter(|p| hit(*p)).collect();
        for p in &pages {
            if let Some(page) = self.pages.get_mut(p) {
                // An exact page that is current, or only partly stale, takes the new damage too.
                let partial = damage.filter(|_| page.exact && (!page.dirty || page.partial.is_some())).and_then(|d| {
                    let r = page.partial.unwrap_or(DRect::EMPTY).union(&partial_region(*p, now.size, d));
                    let source = p.source(now.size);
                    // Most of the page: rendering it whole is no slower.
                    (r.width() as u64 * r.height() as u64 * 2 <= source.width() as u64 * source.height() as u64).then_some(r)
                });
                page.dirty = true;
                page.exact &= partial.is_some();
                page.partial = partial;
            }
            self.bump(*p);
        }
        self.stats.dirtied += pages.len() as u64;
        let jobs: Vec<PageId> = self.jobs.iter().map(|j| j.page).filter(|p| hit(*p)).collect();
        for p in jobs {
            if !self.pages.contains_key(&p) {
                self.bump(p);
            }
        }
        self.cancel_stale();
        self.ready.retain(|d| !hit(d.page));
        self.failed.retain(|(p, _)| !hit(*p));
    }

    /// Forget every page (the GPU canvas lost them, or the document changed size).
    pub fn reset(&mut self) {
        let pages: Vec<PageId> = self.pages.keys().chain(self.jobs.iter().map(|j| &j.page)).copied().collect();
        for p in pages {
            let e = self.epoch(p) + 1;
            self.epochs.insert(p, e);
        }
        self.pages.clear();
        self.ready.clear();
        self.failed.clear();
        self.synced.clear();
        self.cancel_stale();
    }

    /// Cancel jobs whose page moved to a newer epoch.
    fn cancel_stale(&mut self) {
        for j in &self.jobs {
            if j.epoch != self.epoch(j.page) && !j.cancel.swap(true, Ordering::Relaxed) {
                self.stats.cancelled += 1;
            }
        }
    }

    /// Take finished work, cancel work for pages not `wanted`, and start work for wanted pages
    /// that are missing, dirty or still a quick preview (`faithful`: the document has a quick
    /// preview at reductions), as `spawn(page, epoch, exact, cancel, region)` allows: `region`, the
    /// part of a held page to render again ([`Page::partial`]), or the whole page.
    pub fn update(&mut self, wanted: &[PageId], faithful: bool, mut spawn: impl FnMut(PageId, u64, bool, Arc<AtomicBool>, Option<DRect>) -> bool) {
        self.clock += 1;
        let want: HashSet<PageId> = wanted.iter().copied().collect();
        while let Ok(d) = self.rx.try_recv() {
            let Some(i) = self.jobs.iter().position(|j| j.page == d.page && j.epoch == d.epoch) else { continue };
            // Cancelled work ends in an interruption, not a failure: the page is asked for again.
            let cancelled = self.jobs.swap_remove(i).cancel.load(Ordering::Relaxed);
            if cancelled || d.epoch != self.epoch(d.page) || !want.contains(&d.page) {
                continue;
            }
            // A part of a page no longer held (evicted meanwhile): the whole page is asked for.
            if d.region.is_some() && !self.pages.get(&d.page).is_some_and(|q| q.dirty && q.partial == d.region) {
                continue;
            }
            match &d.result {
                Ok(_) => {
                    self.stats.rendered += 1;
                    self.ready.retain(|r| r.page != d.page);
                    self.ready.push(d);
                }
                Err(e) => {
                    log::warn!("viewport page {:?}: {e}", d.page);
                    self.stats.last_error = Some(e.clone());
                    self.failed.insert((d.page, d.epoch));
                }
            }
        }
        for j in &self.jobs {
            if !want.contains(&j.page) && !j.cancel.swap(true, Ordering::Relaxed) {
                self.stats.cancelled += 1;
            }
        }
        self.ready.retain(|d| want.contains(&d.page));
        for p in wanted {
            if let Some(page) = self.pages.get_mut(p) {
                page.touched = self.clock;
            }
        }
        let current = |s: &Self, p: &PageId| s.pages.get(p).is_some_and(|q| !q.dirty) || s.ready.iter().any(|d| d.page == *p);
        let all_shown = wanted.iter().all(|p| current(self, p));
        for p in wanted {
            let live = self.jobs.iter().filter(|j| !j.cancel.load(Ordering::Relaxed)).count();
            if live >= MAX_JOBS {
                break;
            }
            let epoch = self.epoch(*p);
            if self.jobs.iter().any(|j| j.page == *p && j.epoch == epoch) || self.failed.contains(&(*p, epoch)) {
                continue;
            }
            // First every page in view (a quick preview at a reduction), then the exact ones.
            let exact = p.factor == 1 || !faithful || all_shown;
            let have = self.ready.iter().find(|d| d.page == *p).map(|d| d.exact).or_else(|| self.pages.get(p).filter(|q| !q.dirty).map(|q| q.exact));
            if have.is_some_and(|e| e || !exact) {
                continue;
            }
            let region = self.pages.get(p).filter(|q| q.dirty).and_then(|q| q.partial);
            let cancel = Arc::new(AtomicBool::new(false));
            if !spawn(*p, epoch, exact || region.is_some(), cancel.clone(), region) {
                break;
            }
            self.jobs.push(Job { page: *p, epoch, cancel });
        }
        self.stats.wanted = wanted.len();
        self.stats.ready = wanted.iter().filter(|p| self.pages.contains_key(p)).count();
        self.stats.jobs = self.jobs.len();
    }

    /// Retry failed pages once the scratch storage or memory configuration changed (`health`).
    pub fn retry_after(&mut self, health: (u64, u64)) {
        if health != self.health {
            self.health = health;
            self.failed.clear();
        }
    }

    /// Whether a wanted page can't be shown: its render or upload failed and nothing older of it
    /// is held, so the view would have a hole (the CPU canvas draws instead).
    pub fn stuck(&self, wanted: &[PageId]) -> bool {
        wanted.iter().any(|p| !self.pages.contains_key(p) && self.failed.contains(&(*p, self.epoch(*p))))
    }

    /// Whether work is outstanding (the view repaints until it is done).
    pub fn busy(&self) -> bool {
        !self.jobs.is_empty() || !self.ready.is_empty()
    }

    /// The finished pages to upload this frame: at most [`UPLOADS_PER_FRAME`], and
    /// [`UPLOAD_BYTES_PER_FRAME`] of texels (`texel_bytes` each), the first always.
    pub fn take_uploads(&mut self, wanted: &[PageId], texel_bytes: u64) -> Vec<Done> {
        let order = |d: &Done| wanted.iter().position(|p| *p == d.page).unwrap_or(usize::MAX);
        self.ready.sort_by_key(order);
        let mut out = Vec::new();
        let mut bytes = 0u64;
        while out.len() < UPLOADS_PER_FRAME && !self.ready.is_empty() {
            let b = self.ready[0].result.as_ref().map_or(0, |b| b.px.len() as u64 * texel_bytes);
            if !out.is_empty() && bytes + b > UPLOAD_BYTES_PER_FRAME {
                break;
            }
            bytes += b;
            out.push(self.ready.remove(0));
        }
        out
    }

    /// Make room for `bytes` more under `limit` by evicting the least recently wanted pages not
    /// in `wanted`: whether any was evicted, or `None` when the wanted pages leave no room.
    pub fn make_room(&mut self, page: PageId, bytes: u64, limit: u64, wanted: &[PageId]) -> Option<bool> {
        let held = |s: &Self| s.pages.iter().filter(|(p, _)| **p != page).map(|(_, q)| q.bytes).sum::<u64>();
        let mut evicted = false;
        while held(self) + bytes > limit {
            let cold = self.pages.iter().filter(|(p, _)| **p != page && !wanted.contains(p)).min_by_key(|(_, q)| q.touched).map(|(p, _)| *p)?;
            self.drop_page(cold);
            self.stats.evicted += 1;
            evicted = true;
        }
        Some(evicted)
    }

    /// The GPU page ids of the pages held.
    pub fn ids(&self) -> Vec<u64> {
        self.pages.keys().map(|p| p.id()).collect()
    }

    fn drop_page(&mut self, p: PageId) {
        if let Some(q) = self.pages.remove(&p) {
            self.epochs.insert(p, q.epoch);
        }
    }

    /// `done` is on the GPU (`ok`), or couldn't be stored.
    pub fn uploaded(&mut self, done: &Done, bytes: u64, ok: bool) {
        if !ok {
            self.stats.last_error = Some("the GPU canvas couldn't store a viewport page".into());
            self.failed.insert((done.page, done.epoch));
            return;
        }
        self.stats.uploads += 1;
        self.epochs.remove(&done.page);
        if done.region.is_some()
            && let Some(page) = self.pages.get_mut(&done.page)
        {
            (page.dirty, page.partial, page.epoch, page.touched) = (false, None, done.epoch, self.clock);
            self.stats.partial += 1;
            return;
        }
        self.pages.insert(done.page, Page { dirty: false, exact: done.exact, partial: None, epoch: done.epoch, touched: self.clock, bytes });
    }

    /// The part of `page` written by `done` couldn't be stored: drop the page, so it is
    /// rendered whole.
    pub fn partial_failed(&mut self, page: PageId) {
        self.drop_page(page);
    }

    /// Drop pages no longer worth keeping: once every wanted page is shown and current, the
    /// dirty ones and the ones finer than the view (they would draw over it). Coarser clean
    /// pages stay (under the view's own, for a quick pan or zoom out) until evicted. The GPU
    /// page ids to keep when they changed since the last call.
    pub fn prune(&mut self, wanted: &[PageId]) -> Option<Vec<u64>> {
        let all_current = wanted.iter().all(|p| self.pages.get(p).is_some_and(|q| !q.dirty));
        if all_current && let Some(f) = wanted.first().map(|p| p.factor) {
            let drop: Vec<PageId> = self.pages.iter().filter(|(p, q)| !wanted.contains(p) && (q.dirty || p.factor < f)).map(|(p, _)| *p).collect();
            for p in drop {
                self.drop_page(p);
            }
        }
        // Epochs only guard results still on their way.
        let jobs: HashSet<PageId> = self.jobs.iter().map(|j| j.page).collect();
        self.epochs.retain(|p, _| jobs.contains(p));
        let mut ids = self.ids();
        ids.sort_unstable();
        self.stats.resident = ids.len();
        self.stats.resident_bytes = self.pages.values().map(|q| q.bytes).sum();
        (ids != self.synced).then(|| {
            self.synced = ids.clone();
            ids
        })
    }

    /// The pages this stream believes are on the GPU.
    pub fn resident(&self) -> usize {
        self.pages.len()
    }
}

/// Render `page` of `doc` on a worker, if one of the [`MAX_JOBS`] is free.
fn spawn(
    ctx: &egui::Context,
    tx: mpsc::Sender<Done>,
    doc: Arc<Document>,
    display: Option<Arc<CanvasDisplay>>,
    (page, epoch, exact, region): (PageId, u64, bool, Option<DRect>),
    cancel: Arc<AtomicBool>,
) -> bool {
    if ACTIVE.fetch_add(1, Ordering::AcqRel) >= MAX_JOBS {
        ACTIVE.fetch_sub(1, Ordering::AcqRel);
        return false;
    }
    struct Slot(egui::Context);
    impl Drop for Slot {
        fn drop(&mut self) {
            ACTIVE.fetch_sub(1, Ordering::AcqRel);
            // A view waiting for a free worker (another document's) can start now.
            self.0.request_repaint();
        }
    }
    let slot = Slot(ctx.clone());
    let work = move || {
        let _slot = slot;
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| render(&doc, page, exact, region, display.as_deref(), &cancel)))
            .unwrap_or_else(|_| Err("viewport page render panicked".into()));
        let _ = tx.send(Done { page, epoch, exact, region, result });
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::Builder::new().name("pc-viewport-page".into()).spawn(work).is_ok()
    }
    #[cfg(target_arch = "wasm32")]
    {
        work();
        true
    }
}

/// One page as the GPU canvas stores it (encoded for the display like the texture set).
fn render(
    doc: &Document,
    page: PageId,
    exact: bool,
    region: Option<DRect>,
    display: Option<&CanvasDisplay>,
    cancel: &AtomicBool,
) -> Result<photocraft_compose::Buffer, String> {
    let size = [doc.size.width, doc.size.height];
    let source = region.unwrap_or_else(|| page.source(size));
    let rows = u64::from(source.height().min(128));
    let _working = photocraft_raster::memory::try_reserve(u64::from(source.width()) * rows * 16 + (16 << 20))?;
    let before = photocraft_raster::spill::read_error_generation();
    let check = || cancel.load(Ordering::Relaxed);
    let i = photocraft_raster::Interrupt::cancel_only(&check);
    let buf = if exact {
        photocraft_compose::viewport::render_page(doc, source, page.factor, i)?
    } else {
        photocraft_compose::viewport::preview_page(doc, source, page.factor, i)?
    };
    i.check().map_err(|e| e.to_string())?;
    photocraft_raster::spill::check_since(before)?;
    Ok(texture_buffer(display, &buf).into_owned())
}

/// Draw document `idx` from viewport pages when its texture set doesn't fit the GPU canvas
/// budget: true when the view should paint [`stream_key`] with the GPU canvas.
pub(super) fn show(app: &mut PhotocraftApp, ctx: &egui::Context, idx: usize, xf: &ViewXform, zoom: f32) -> bool {
    let Some(gpu) = app.gpu.clone() else { return false };
    if app.ui.view.pattern_preview || gpu.fault().is_some() {
        return false;
    }
    let Some((revision, last_damage, id, size)) = app
        .session
        .documents()
        .get(idx)
        .map(|st| (st.revision, st.last_damage.map(|r| if r.is_empty() { r } else { r.inflate(effect_reach(&st.doc.layers)) }), st.doc.id, st.doc.size))
    else {
        return false;
    };
    let size = [size.width, size.height];
    let (doc, raw_key) = display_doc(app, idx);
    // A size-changing preview draws as main does; a document that fits keeps its texture set.
    if [doc.size.width, doc.size.height] != size || gpu.fits_canvas(&doc) {
        if let Some(s) = app.streams.remove(&id) {
            drop(s);
            gpu.retain_pages(stream_key(id.0), &[]);
        }
        return false;
    }
    let (display, _) = canvas_display(app, &doc, None);
    let display_key = texture_key(display.as_deref());
    let now = Identity { revision, preview: raw_key, display: display_key, size };
    let seen = app.streams.get(&id).and_then(|s| s.seen);
    if seen != Some(now) {
        let damage = seen
            .filter(|s| s.size == size && s.display == display_key)
            .and_then(|s| damage_since(app, idx, (s.revision, s.preview ^ display_key), (revision, raw_key), display_key, last_damage));
        app.streams.entry(id).or_default().invalidate(now, damage);
    }
    let key = stream_key(id.0);
    // Half the per-document canvas budget: the rest is headroom for what the compositor needs.
    let limit = gpu.canvas_budget() / 2;
    let page_bytes = gpu.page_texture_bytes(doc.depth, size, PAGE_SIDE + 2, PAGE_SIDE + 2).max(1);
    let max_pages = usize::try_from(limit / page_bytes).unwrap_or(usize::MAX);
    let wanted = pages(size, visible_doc_rect(xf), zoom, max_pages);
    if gpu.texture_info(id.0).is_some() {
        // Switching from the texture set: free it, the pages are budgeted against that memory.
        gpu.release_texture_set(id.0);
    }
    let stream = app.streams.entry(id).or_default();
    stream.retry_after((photocraft_raster::spill::read_error_generation(), photocraft_raster::memory::configuration_generation()));
    // No room for a single page, or a page that failed: the CPU canvas draws the document, as
    // main does for every document over the budget.
    if max_pages == 0 || stream.stuck(&wanted) {
        return false;
    }
    let faithful = photocraft_compose::proxy::proxy_faithful(&doc);
    let texel_bytes = gpu.page_texture_bytes(doc.depth, size, 1, 1);
    // The GPU canvas dropped the pages (released, or pruned with a closed document): start over.
    if stream.resident() > 0 && gpu.pages(key).0.len() != stream.resident() {
        stream.reset();
    }
    // Drawn from pages: no texture set, so no layer budget or background redraw applies.
    crate::background_redraw::cancel(app, id);
    if app.perf.gpu_layer_fit.is_some_and(|f| f.doc == id.0) {
        app.perf.gpu_layer_fit = None;
    }
    app.navigator_gpu.remove(&id);
    crate::render_path::note(app, id.0, crate::render_path::Path::Gpu { full: true });
    let stream = app.streams.entry(id).or_default();
    let tx = stream.tx.clone();
    stream.update(&wanted, faithful, |page, epoch, exact, cancel, region| {
        spawn(ctx, tx.clone(), doc.clone(), display.clone(), (page, epoch, exact, region), cancel)
    });
    for d in stream.take_uploads(&wanted, texel_bytes) {
        let Ok(buf) = &d.result else { continue };
        if d.region.is_some() {
            if gpu.update_page(key, d.page.id(), buf) {
                stream.uploaded(&d, 0, true);
            } else {
                stream.partial_failed(d.page);
                gpu.retain_pages(key, &stream.ids());
            }
            continue;
        }
        let bytes = buf.px.len() as u64 * texel_bytes;
        let ok = match stream.make_room(d.page, bytes, limit, &wanted) {
            None => false,
            Some(evicted) => {
                if evicted {
                    gpu.retain_pages(key, &stream.ids());
                }
                gpu.upload_page(crate::gpu_canvas::PageUpload {
                    key,
                    page: d.page.id(),
                    size,
                    depth: doc.depth,
                    factor: d.page.factor,
                    core: d.page.core(size),
                    buf,
                })
            }
        };
        stream.uploaded(&d, bytes, ok);
    }
    if let Some(ids) = stream.prune(&wanted) {
        gpu.retain_pages(key, &ids);
    }
    stream.stats.limit = limit;
    if stream.busy() {
        ctx.request_repaint_after(std::time::Duration::from_millis(16));
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIZE: [u32; 2] = [20_000, 12_000];

    fn id(revision: u64) -> Identity {
        Identity { revision, preview: 1, display: 2, size: SIZE }
    }

    fn page(factor: u32, x: u32, y: u32) -> PageId {
        PageId { factor, x, y }
    }

    fn done(p: PageId, epoch: u64, exact: bool, side: i32) -> Done {
        let rect = DRect::new(0, 0, side, side);
        Done { page: p, epoch, exact, region: None, result: Ok(photocraft_compose::Buffer { rect, px: vec![[0.0; 4]; (side * side) as usize] }) }
    }

    /// A stream holding `held` clean, exact pages.
    fn holding(held: &[PageId]) -> Stream {
        let mut s = Stream::default();
        s.invalidate(id(1), None);
        for p in held {
            s.uploaded(&done(*p, s.epoch(*p), true, 4), 100, true);
        }
        s
    }

    /// Spawns every job it is offered, recording it with its cancel flag.
    fn recorder(log: &mut Vec<(PageId, u64, bool, Arc<AtomicBool>)>) -> impl FnMut(PageId, u64, bool, Arc<AtomicBool>, Option<DRect>) -> bool + '_ {
        |p, e, x, c, _| {
            log.push((p, e, x, c));
            true
        }
    }

    #[test]
    fn factor_keeps_a_texel_at_most_one_screen_pixel() {
        assert_eq!(factor(4.0), 1);
        assert_eq!(factor(1.0), 1);
        assert_eq!(factor(0.5), 2);
        assert_eq!(factor(0.3), 2);
        assert_eq!(factor(0.25), 4);
        assert_eq!(factor(0.0), 1);
        assert_eq!(factor(f32::NAN), 1);
        assert_eq!(factor(1e-9), MAX_FACTOR);
    }

    #[test]
    fn pages_cover_the_view_centre_first_and_coarsen_past_the_limit() {
        let visible = DRect::new(1000, 1000, 2500, 2000);
        let got = pages(SIZE, visible, 1.0, 64);
        // Columns 1..=4, rows 1..=3 of 512 px pages.
        assert_eq!(got.len(), 4 * 3);
        assert!(got.iter().all(|p| p.factor == 1));
        let union = got.iter().fold(DRect::EMPTY, |u, p| u.union(&p.core(SIZE)));
        assert_eq!(union.intersect(&visible), visible, "every visible pixel is on a page");
        let centre = got[0].core(SIZE);
        assert!(centre.inflate(512).contains(1750, 1500), "centre first: {:?}", got[0]);
        // The whole document at 1:1 is 40×24 pages: over the limit it uses a reduction.
        let whole = DRect::new(0, 0, SIZE[0] as i32, SIZE[1] as i32);
        let coarse = pages(SIZE, whole, 1.0, 64);
        assert!(!coarse.is_empty() && coarse.len() <= 64);
        assert!(coarse.iter().all(|p| p.factor == 4), "{:?}", coarse[0]);
        assert!(pages(SIZE, whole, 1.0, 0).is_empty());
        assert!(pages(SIZE, DRect::new(-500, -500, -1, -1), 1.0, 64).is_empty(), "off the document");
    }

    #[test]
    fn a_page_core_tiles_the_document_and_its_source_adds_one_texel() {
        let p = page(2, 1, 0);
        assert_eq!(p.core(SIZE), DRect::new(1024, 0, 2048, 1024));
        assert_eq!(p.source(SIZE), DRect::new(1022, 0, 2050, 1026));
        // The last column is clipped to the document.
        let last = page(1, SIZE[0] / 512, 0);
        assert_eq!(last.core(SIZE).x1, SIZE[0] as i32);
        assert!(page(1, 1000, 0).core(SIZE).is_empty());
        // Ids differ across factors and positions.
        assert_ne!(page(1, 1, 0).id(), page(2, 1, 0).id());
        assert_ne!(page(1, 1, 0).id(), page(1, 0, 1).id());
    }

    #[test]
    fn an_edit_dirties_only_the_pages_it_reaches() {
        let (a, b, c) = (page(1, 0, 0), page(1, 1, 0), page(1, 4, 4));
        let mut s = holding(&[a, b, c]);
        // Damage on page a's right edge also reaches b's one-texel apron.
        s.invalidate(id(2), Some(DRect::new(500, 10, 512, 20)));
        assert!(s.pages[&a].dirty && s.pages[&b].dirty);
        assert!(!s.pages[&c].dirty && s.pages[&c].exact, "far page untouched");
        assert_eq!(s.stats.dirtied, 2);
        // A dirty page stays on screen until replaced; no damage known dirties every page.
        s.invalidate(id(3), None);
        assert_eq!(s.pages.len(), 3);
        assert!(s.pages.values().all(|p| p.dirty));
        // A new size starts over.
        s.invalidate(Identity { size: [100, 100], ..id(4) }, None);
        assert_eq!(s.resident(), 0);
    }

    #[test]
    fn pan_zoom_and_edits_cancel_obsolete_work() {
        let mut s = holding(&[]);
        let mut log = Vec::new();
        let view = [page(1, 0, 0), page(1, 1, 0), page(1, 2, 0)];
        s.update(&view, true, recorder(&mut log));
        assert_eq!(log.len(), MAX_JOBS, "bounded workers");
        assert!(log.iter().all(|j| j.2), "1:1 pages are rendered exactly");
        // Pan: the first page is no longer wanted.
        s.update(&view[1..], true, recorder(&mut Vec::new()));
        assert!(log[0].3.load(Ordering::Relaxed), "work for a page out of view is cancelled");
        assert!(!log[1].3.load(Ordering::Relaxed));
        // Edit on the second page: its result would be stale.
        s.invalidate(id(2), Some(DRect::new(600, 0, 601, 1)));
        assert!(log[1].3.load(Ordering::Relaxed), "work on an edited page is cancelled");
        // Its old result is dropped when it arrives, and so is a cancelled job's interruption.
        s.tx.send(Done { page: log[0].0, epoch: log[0].1, exact: true, region: None, result: Err("interrupted".into()) }).unwrap();
        s.tx.send(done(log[1].0, log[1].1, true, 4)).unwrap();
        s.update(&view[1..], true, |_, _, _, _, _| false);
        assert!(s.ready.is_empty(), "stale result dropped");
        assert!(!s.stuck(&view) && s.stats.last_error.is_none(), "a cancellation is not a failure");
        // Zoom: every page changes factor.
        let mut log2 = Vec::new();
        s.update(&[page(1, 1, 0)], true, recorder(&mut log2));
        let jobs = s.jobs.len();
        s.update(&[page(2, 0, 0)], true, |_, _, _, _, _| true);
        assert!(log2.iter().all(|j| j.3.load(Ordering::Relaxed)), "zoom cancels the old factor's work");
        assert!(s.stats.cancelled >= 3 && jobs > 0);
    }

    #[test]
    fn reductions_show_a_quick_preview_before_the_exact_page() {
        let mut s = holding(&[]);
        let view = [page(2, 0, 0)];
        let mut log = Vec::new();
        s.update(&view, true, recorder(&mut log));
        assert_eq!(log.len(), 1);
        assert!(!log[0].2, "a faithful document previews first");
        s.tx.send(done(view[0], log[0].1, false, 4)).unwrap();
        let mut log2 = Vec::new();
        s.update(&view, true, recorder(&mut log2));
        assert!(log2.len() == 1 && log2[0].2, "then the exact page, once every page is shown");
        let mut log3 = Vec::new();
        holding(&[]).update(&view, false, recorder(&mut log3));
        assert!(log3[0].2, "an unfaithful preview is skipped");
    }

    #[test]
    fn a_small_edit_re_renders_only_the_texels_it_reaches() {
        let (a, b) = (page(4, 1, 1), page(4, 3, 3));
        let mut s = holding(&[a, b]);
        // Page a: core 2048..4096 at 4 px per texel, source one texel more.
        s.invalidate(id(2), Some(DRect::new(2101, 2102, 2110, 2111)));
        assert_eq!(s.pages[&a].partial, Some(DRect::new(2100, 2100, 2112, 2112)), "whole texels");
        assert!(s.pages[&a].dirty && s.pages[&a].exact);
        // More damage on a dirty page adds to its part.
        s.invalidate(id(3), Some(DRect::new(2200, 2100, 2204, 2104)));
        assert_eq!(s.pages[&a].partial, Some(DRect::new(2100, 2100, 2204, 2112)));
        let mut regions = Vec::new();
        s.update(&[a], true, |p, e, x, _, r| {
            regions.push((p, e, x, r));
            true
        });
        assert_eq!(regions, vec![(a, s.epoch(a), true, s.pages[&a].partial)]);
        // The part is written into the page held: same page, same bytes, current again.
        let mut d = done(a, regions[0].1, true, 4);
        d.region = regions[0].3;
        s.tx.send(d).unwrap();
        s.update(&[a], true, |_, _, _, _, _| panic!("nothing left to render"));
        let up = s.take_uploads(&[a], 16);
        assert_eq!(up.len(), 1);
        s.uploaded(&up[0], 0, true);
        assert!(!s.pages[&a].dirty && s.pages[&a].partial.is_none() && s.pages[&a].bytes == 100);
        assert_eq!(s.stats.partial, 1);
        // Most of a page, or damage of unknown extent, renders it whole.
        s.invalidate(id(4), Some(DRect::new(2048, 2048, 4000, 4000)));
        assert!(s.pages[&a].dirty && s.pages[&a].partial.is_none() && !s.pages[&a].exact);
        s.invalidate(id(5), None);
        assert!(s.pages[&b].partial.is_none());
        // A part of a page evicted meanwhile is dropped: the whole page is asked for.
        let mut s = holding(&[a]);
        s.invalidate(id(2), Some(DRect::new(2101, 2102, 2110, 2111)));
        let region = s.pages[&a].partial;
        let epoch = s.epoch(a);
        s.partial_failed(a);
        let mut d = done(a, epoch, true, 4);
        d.region = region;
        s.tx.send(d).unwrap();
        let mut log = Vec::new();
        s.update(&[a], true, recorder(&mut log));
        assert!(s.ready.is_empty());
        assert_eq!(log.len(), 1, "the whole page");
    }

    #[test]
    fn uploads_per_frame_are_bounded() {
        let mut s = holding(&[]);
        let view: Vec<PageId> = (0..4).map(|x| page(1, x, 0)).collect();
        for p in &view {
            s.ready.push(done(*p, 0, true, 514));
        }
        // 514² texels of 16 bytes: 4.2 MB each.
        let first = s.take_uploads(&view, 16);
        assert_eq!(first.len(), UPLOADS_PER_FRAME);
        assert_eq!(first[0].page, view[0], "in view order");
        // A page over the byte cap still goes alone.
        let big = s.take_uploads(&view, 1 << 10);
        assert_eq!(big.len(), 1);
        assert_eq!(s.ready.len(), 1);
    }

    #[test]
    fn the_budget_evicts_cold_pages_but_never_wanted_ones() {
        let held: Vec<PageId> = (0..4).map(|x| page(1, x, 0)).collect();
        let mut s = holding(&held);
        s.clock = 10;
        s.pages.get_mut(&held[2]).unwrap().touched = 9;
        let wanted = [held[0], held[1], page(1, 9, 9)];
        // 400 held, limit 450: room for 100 more means evicting one cold page, the oldest.
        assert_eq!(s.make_room(wanted[2], 100, 450, &wanted), Some(true));
        assert!(!s.pages.contains_key(&held[3]) && s.pages.contains_key(&held[2]));
        assert_eq!(s.make_room(wanted[2], 100, 399, &wanted), Some(true));
        assert_eq!(s.resident(), 2);
        assert_eq!(s.make_room(wanted[2], 100, 250, &wanted), None, "wanted pages are never evicted");
        assert_eq!(s.stats.evicted, 2);
    }

    #[test]
    fn a_failed_page_falls_back_until_it_is_retried() {
        let mut s = holding(&[]);
        let view = [page(1, 0, 0)];
        let mut log = Vec::new();
        s.update(&view, true, recorder(&mut log));
        s.tx.send(Done { page: view[0], epoch: log[0].1, exact: true, region: None, result: Err("out of memory".into()) }).unwrap();
        s.update(&view, true, |_, _, _, _, _| panic!("no retry while failed"));
        assert!(s.stuck(&view));
        assert_eq!(s.stats.last_error.as_deref(), Some("out of memory"));
        s.retry_after((7, 7));
        assert!(!s.stuck(&view), "a new memory configuration retries");
        s.invalidate(id(2), None);
        assert!(!s.stuck(&view));
    }

    #[test]
    fn prune_drops_finer_and_dirty_pages_once_the_view_is_current() {
        let (fine, coarse, view) = (page(1, 0, 0), page(4, 0, 0), page(2, 0, 0));
        let mut s = holding(&[fine, coarse]);
        assert!(s.prune(&[view]).is_some());
        assert_eq!(s.resident(), 2, "kept until the view's own page is shown");
        s.uploaded(&done(view, s.epoch(view), true, 4), 100, true);
        let ids = s.prune(&[view]).expect("changed");
        assert_eq!(ids.len(), 2);
        assert!(!s.pages.contains_key(&fine) && s.pages.contains_key(&coarse));
        assert!(s.prune(&[view]).is_none(), "unchanged: nothing to tell the GPU");
        assert!(s.epochs.is_empty(), "epochs only for work in flight");
    }
}
