//! Full CPU redraws of the GPU canvas off the UI thread (#4024). When the wgpu compositor can't
//! draw a full refresh (the layers exceed its memory budget, or the document uses something it
//! doesn't cover), the CPU compositor redraws the whole document: minutes on a multi-gigabyte
//! PSB. With background jobs (the desktop app) a worker composites it in blocks instead, the
//! visible ones first, and each frame uploads the blocks that are done, so the window keeps
//! drawing and answering while the rest of the canvas fills in. Until a block lands the texture
//! keeps what it showed before.
//!
//! An edit while it runs restarts the worker on the new document with the blocks still to draw;
//! the edit's own damage is drawn by the canvas as usual. A full change (a new preview, a new
//! display transform) starts over. Without background jobs (web, tests) the canvas keeps the
//! synchronous [`GpuCanvas::upload_composite`].
//!
//! The last finished redraws are kept ([`Kept`]), keyed by the whole document they drew (#6528):
//! going back to a state drawn a moment ago (⌥-click on an eye and again, an undo and its redo)
//! uploads those blocks instead of compositing every layer again.

use std::collections::VecDeque;
#[cfg(test)]
use std::sync::atomic::AtomicU64;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TryRecvError};
use std::sync::{Arc, Weak};

use photocraft_doc::{DocId, Document};
use photocraft_engine::display_color::CanvasDisplay;
use photocraft_geom::Rect;
#[cfg(not(target_arch = "wasm32"))]
use rayon::prelude::*;

use crate::PhotocraftApp;
use crate::gpu_canvas::GpuCanvas;

/// Side of a block in document pixels: whole render tiles, and a power of two so each block's
/// mip levels don't depend on its neighbours.
const BLOCK: i32 = 1024;

/// Blocks composited at once (in parallel, on top of each block's own tiles), and how many
/// finished blocks may wait for the UI thread: bounds the memory a redraw holds.
const BATCH: usize = 4;

/// Finished redraws kept: a state and the one before it (a solo and its restore).
const KEEP: usize = 2;

/// [`GpuCanvas::refresh`] kind of a redraw served from a [`Kept`] composite.
pub(crate) const KEPT: &str = "kept";

/// Tests hold the redraw worker before it draws a batch, so an edit lands while blocks are still
/// to come. A real run never waits: a fast machine draws a test document before its next frame,
/// leaving nothing drawn "before" the edit.
#[cfg(test)]
static HOLD: AtomicBool = AtomicBool::new(false);

/// Identity of the next spawned redraw worker: lets a test tell the worker it watched from a
/// replacement that [`restart`] spawns after an edit.
#[cfg(test)]
static NEXT_WORKER_GENERATION: AtomicU64 = AtomicU64::new(1);

/// Hold the redraw worker until the guard drops.
#[cfg(test)]
pub(crate) fn hold_redraws() -> HoldRedraws {
    HOLD.store(true, Ordering::SeqCst);
    HoldRedraws
}

/// Wait until worker `generation` of document `doc`'s [`Redraw`] has reached its pre-batch
/// [`HOLD`]. A replacement worker has another generation, so an edit only lands while the worker
/// the test watched still has its blocks to draw. Returns `false` after `timeout`.
#[cfg(test)]
pub(crate) fn wait_for_held_redraw(app: &PhotocraftApp, doc: DocId, generation: u64, timeout: std::time::Duration) -> bool {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        if let Some(r) = app.redraws.get(&doc)
            && r.worker_generation == generation
            && r.reached_hold.load(Ordering::SeqCst)
        {
            return true;
        }
        if std::time::Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(std::time::Duration::from_millis(1));
    }
}

/// Releases [`hold_redraws`], including while a test unwinds.
#[cfg(test)]
pub(crate) struct HoldRedraws;

#[cfg(test)]
impl Drop for HoldRedraws {
    fn drop(&mut self) {
        HOLD.store(false, Ordering::SeqCst);
    }
}

/// One finished block, as texels of the canvas texture.
struct Piece {
    block: usize,
    texels: Vec<u8>,
}

/// A redraw in progress for one document's GPU texture.
pub(crate) struct Redraw {
    blocks: Vec<Rect>,
    pending: Vec<bool>,
    left: usize,
    /// Pixels uploaded so far.
    drawn: u64,
    rx: Receiver<Piece>,
    stop: Arc<AtomicBool>,
    started_ms: f64,
    fallback: Option<crate::render_path::CpuReason>,
    /// Time spent on the failed GPU composite attempts that led to this redraw (ms), recorded once
    /// when it ends.
    gpu_attempt_ms: f64,
    /// The state the blocks are drawn from, and what the texels depend on besides it.
    doc: Arc<Document>,
    display: u64,
    text_gamma: u32,
    format: eframe::wgpu::TextureFormat,
    /// Uploaded blocks of `doc` kept for [`Kept`] (empty once over `cap`), and their bytes.
    collected: Vec<Option<Vec<u8>>>,
    collected_bytes: u64,
    /// Bytes the kept composites may hold: what Memory Usage leaves beside the document.
    cap: u64,
    /// Set when this worker first reaches the test [`HOLD`], and this worker's generation.
    #[cfg(test)]
    reached_hold: Arc<AtomicBool>,
    #[cfg(test)]
    worker_generation: u64,
}

impl Redraw {
    /// The text gamma changed while it runs (even if back since): its blocks may mix both, so
    /// none of them are kept. Checked every frame for every redraw, shown or not ([`retain`]); a
    /// change and back between two frames (one batch of agent commands) isn't seen.
    fn forget_if_gamma_changed(&mut self, gamma: u32) {
        if self.text_gamma != gamma {
            self.collected = Vec::new();
            self.collected_bytes = 0;
        }
    }

    /// Share of the document drawn, 0..=1.
    pub fn fraction(&self) -> f32 {
        if self.blocks.is_empty() { 1.0 } else { 1.0 - self.left as f32 / self.blocks.len() as f32 }
    }
}

impl Drop for Redraw {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// A finished redraw: the canvas texels of every block of `doc`.
pub(crate) struct Kept {
    /// Weak, so a kept composite never keeps a state alive (and its tiles allocated, outside Memory
    /// Usage) once History has dropped it; such an entry is dead and goes at the next trim.
    doc: Weak<Document>,
    id: DocId,
    display: u64,
    text_gamma: u32,
    format: eframe::wgpu::TextureFormat,
    blocks: Vec<Rect>,
    texels: Vec<Vec<u8>>,
    bytes: u64,
}

impl Kept {
    /// Whether these are the texels a redraw of `doc` would draw. The key is the whole document
    /// (layers and their pixels, visibility, opacity, blending, masks, adjustments and everything
    /// else it holds), so a stale composite can't come back; Arc-shared tiles compare without
    /// reading their pixels. Besides the document, the display transform and the text gamma
    /// ([`text_gamma`]) decide the texels.
    fn matches(&self, doc: &Arc<Document>, display: u64, text_gamma: u32, format: eframe::wgpu::TextureFormat, blocks: &[Rect]) -> bool {
        self.display == display
            && self.text_gamma == text_gamma
            && self.format == format
            && self.blocks == blocks
            && self.doc.upgrade().is_some_and(|kept| Arc::ptr_eq(&kept, doc) || *kept == **doc)
    }
}

/// The last [`KEEP`] finished redraws, newest first, within a byte cap.
#[derive(Default)]
pub(crate) struct KeptComposites {
    entries: VecDeque<Kept>,
    /// The entries' documents and their revisions, and Memory Usage, when the cap was last taken.
    capped: (Vec<(DocId, u64)>, u64),
}

impl KeptComposites {
    /// The kept redraw of exactly this state, now the newest.
    fn find(&mut self, doc: &Arc<Document>, display: u64, text_gamma: u32, format: eframe::wgpu::TextureFormat, blocks: &[Rect]) -> Option<&Kept> {
        let i = self.entries.iter().position(|k| k.matches(doc, display, text_gamma, format, blocks))?;
        let k = self.entries.remove(i)?;
        self.entries.push_front(k);
        self.entries.front()
    }

    /// Keep `k`, the redraw of `doc` (in place of an entry of the same state), within `cap` bytes
    /// in all.
    fn store(&mut self, k: Kept, doc: &Arc<Document>, cap: u64) {
        // Another list: [`retain`] takes the cap over all its documents again.
        self.capped = Default::default();
        self.entries.retain(|e| !e.matches(doc, k.display, k.text_gamma, k.format, &k.blocks));
        self.entries.push_front(k);
        self.trim(cap);
    }

    /// Drop dead entries (their state is gone), then the oldest until at most [`KEEP`], holding at
    /// most `cap` bytes, are left.
    fn trim(&mut self, cap: u64) {
        self.entries.retain(|k| k.doc.strong_count() > 0);
        while self.entries.len() > KEEP || self.bytes() > cap {
            self.entries.pop_back();
        }
    }

    fn bytes(&self) -> u64 {
        self.entries.iter().fold(0, |n, k| n.saturating_add(k.bytes))
    }

    fn retain(&mut self, live: impl Fn(DocId) -> bool) {
        self.entries.retain(|k| live(k.id));
    }
}

/// Color Settings › "Blend Text Colors Using Gamma": type layers composite with it, but it is an
/// application setting, not part of the document. Read from the session's settings, which the
/// engine keeps in step with the process-wide value compose reads (`psblend::set_text_gamma`).
fn text_gamma(app: &PhotocraftApp) -> u32 {
    app.session.color.settings.blend_text_gamma.to_bits()
}

/// Bytes kept composites of document `doc` may hold: what Memory Usage leaves beside its pixels
/// and History. It shrinks as the document grows, which drops them under memory pressure.
fn keep_cap(app: &PhotocraftApp, doc: DocId) -> u64 {
    let budget = app.session.prefs().performance.history_budget_bytes() as u64;
    let used = app.session.documents().iter().find(|st| st.doc.id == doc).map_or(u64::MAX, |st| st.memory_bytes() as u64);
    budget.saturating_sub(used)
}

/// Whether full CPU refreshes run in the background here.
pub(crate) fn enabled(app: &PhotocraftApp) -> bool {
    cfg!(not(target_arch = "wasm32")) && app.background_jobs
}

/// `bounds` cut into [`BLOCK`]-sized blocks, row by row.
fn blocks(bounds: Rect) -> Vec<Rect> {
    let mut out = Vec::new();
    let mut y = bounds.y0;
    while y < bounds.y1 {
        let mut x = bounds.x0;
        while x < bounds.x1 {
            out.push(Rect::new(x, y, (x + BLOCK).min(bounds.x1), (y + BLOCK).min(bounds.y1)));
            x += BLOCK;
        }
        y += BLOCK;
    }
    out
}

/// The order to draw `wanted` blocks in: those `visible` shows first, then the rest, each group
/// nearest the view's centre first.
fn order(blocks: &[Rect], wanted: impl Iterator<Item = usize>, visible: Rect) -> Vec<usize> {
    let (cx, cy) = ((visible.x0 as i64 + visible.x1 as i64) / 2, (visible.y0 as i64 + visible.y1 as i64) / 2);
    let mut out: Vec<usize> = wanted.collect();
    out.sort_by_key(|&i| {
        let b = blocks[i];
        let (bx, by) = ((b.x0 as i64 + b.x1 as i64) / 2, (b.y0 as i64 + b.y1 as i64) / 2);
        (b.intersect(&visible).is_empty(), (bx - cx).pow(2) + (by - cy).pow(2))
    });
    out
}

/// Redraw `region` of document `doc` (`doc.id`'s texture, prepared by
/// [`GpuCanvas::refresh_with`]) in the background. A redraw already running for it continues on
/// `doc` with `region`'s blocks added.
#[allow(clippy::too_many_arguments)]
pub(crate) fn start(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    gpu: &GpuCanvas,
    doc: Arc<Document>,
    display: Option<Arc<CanvasDisplay>>,
    visible: Rect,
    region: Rect,
    fallback: Option<crate::render_path::CpuReason>,
    gpu_attempt_ms: f64,
) {
    let all = blocks(doc.bounds());
    let display_key = display.as_deref().map_or(0, |d| d.texture_key);
    let gamma = text_gamma(app);
    // Taken again when a running redraw continues: the edit (or Memory Usage) may have changed it.
    let cap = keep_cap(app, doc.id);
    app.kept.trim(cap);
    let (mut pending, since, fallback, gpu_attempt_ms, mut keep) = match app.redraws.remove(&doc.id) {
        Some(mut old) if old.blocks == all => {
            // Blocks drawn from another state aren't part of this one's composite.
            let same = old.display == display_key && old.text_gamma == gamma && (Arc::ptr_eq(&old.doc, &doc) || *old.doc == *doc);
            let (collected, bytes) =
                if same && old.collected_bytes <= cap { (std::mem::take(&mut old.collected), old.collected_bytes) } else { (vec![None; all.len()], 0) };
            (
                std::mem::take(&mut old.pending),
                (old.started_ms, old.drawn),
                fallback.or_else(|| old.fallback.take()),
                gpu_attempt_ms + old.gpu_attempt_ms,
                (collected, bytes, cap),
            )
        }
        _ => (vec![false; all.len()], (crate::gpu_canvas::now_ms(), 0), fallback, gpu_attempt_ms, (vec![None; all.len()], 0, cap)),
    };
    for (p, b) in pending.iter_mut().zip(&all) {
        *p |= !b.intersect(&region).is_empty();
    }
    if !pending.contains(&true) {
        return;
    }
    let Some(format) = gpu.prepare_texture(doc.id.0, &doc) else { return };
    // This exact state was drawn a moment ago: upload its blocks rather than composite them.
    if let Some(k) = app.kept.find(&doc, display_key, gamma, format, &all) {
        let t0 = crate::gpu_canvas::now_ms();
        let mut px = 0u64;
        for (i, p) in pending.iter_mut().enumerate() {
            if let (true, Some(b), Some(t)) = (*p, all.get(i), k.texels.get(i))
                && gpu.upload_rect(doc.id.0, [b.x0 as u32, b.y0 as u32], [b.width(), b.height()], t)
            {
                *p = false;
                px = px.saturating_add(b.width() as u64 * b.height() as u64);
            }
        }
        if !pending.contains(&true) {
            app.perf.record(KEPT, px, 0.0, crate::gpu_canvas::now_ms() - t0);
            return;
        }
        keep = (vec![None; all.len()], 0, keep.2);
    }
    spawn(app, ctx, Spawn { doc, display, display_key, text_gamma: gamma, format, visible, blocks: all, pending, since, fallback, gpu_attempt_ms, keep });
}

/// After an edit drew its own damage: continue document `doc.id`'s redraw, if one is running, on
/// the edited document, so blocks still to come don't bring back what was there before it.
pub(crate) fn restart(app: &mut PhotocraftApp, ctx: &egui::Context, gpu: &GpuCanvas, doc: Arc<Document>, display: Option<Arc<CanvasDisplay>>, visible: Rect) {
    if app.redraws.contains_key(&doc.id) {
        start(app, ctx, gpu, doc, display, visible, Rect::EMPTY, None, 0.0);
    }
}

/// Upload the blocks document `doc`'s redraw has finished since the last frame. When the last one
/// lands, the redraw is recorded like the synchronous refresh it replaces (and explained once if
/// it was slow).
pub(crate) fn poll(app: &mut PhotocraftApp, gpu: &GpuCanvas, doc: DocId) {
    let gamma = text_gamma(app);
    let Some(r) = app.redraws.get_mut(&doc) else { return };
    r.forget_if_gamma_changed(gamma);
    loop {
        match r.rx.try_recv() {
            Ok(p) => {
                let b = r.blocks[p.block];
                if r.pending[p.block] && gpu.upload_rect(doc.0, [b.x0 as u32, b.y0 as u32], [b.width(), b.height()], &p.texels) {
                    r.pending[p.block] = false;
                    r.left -= 1;
                    r.drawn += b.width() as u64 * b.height() as u64;
                    collect(r, p);
                }
            }
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => break,
        }
    }
    let Some(mut r) = app.redraws.remove(&doc) else { return };
    if r.left > 0 {
        log::warn!("canvas redraw stopped with {} of {} blocks left", r.left, r.blocks.len());
        return;
    }
    if r.collected.len() == r.blocks.len() && r.collected.iter().all(Option::is_some) {
        let texels = std::mem::take(&mut r.collected).into_iter().flatten().collect();
        let k = Kept {
            doc: Arc::downgrade(&r.doc),
            id: r.doc.id,
            display: r.display,
            text_gamma: r.text_gamma,
            format: r.format,
            blocks: r.blocks.clone(),
            texels,
            bytes: r.collected_bytes,
        };
        // The cap as it is now, not when the redraw began (once per redraw, not per frame).
        let cap = keep_cap(app, doc);
        app.kept.store(k, &r.doc, cap);
    }
    let ms = crate::gpu_canvas::now_ms() - r.started_ms;
    app.perf.record("background", r.drawn, ms, 0.0);
    app.perf.record_gpu_attempt(r.gpu_attempt_ms);
    crate::notices::slow_cpu_refresh(app, doc.0, ms, r.fallback.as_ref());
}

/// Keep uploaded block `p` for [`Kept`], unless the composite would pass the cap.
fn collect(r: &mut Redraw, p: Piece) {
    let len = p.texels.len() as u64;
    let Some(slot) = r.collected.get_mut(p.block) else { return };
    let before = slot.as_ref().map_or(0, |t| t.len() as u64);
    let bytes = r.collected_bytes.saturating_sub(before).saturating_add(len);
    if bytes > r.cap {
        r.collected = Vec::new();
        r.collected_bytes = 0;
        return;
    }
    *slot = Some(p.texels);
    r.collected_bytes = bytes;
}

/// The active document's redraw in the status bar: "Redrawing" and how much is done.
pub(crate) fn status(app: &PhotocraftApp, ui: &mut egui::Ui) {
    let Some(r) = app.session.active().and_then(|st| app.redraws.get(&st.doc.id)) else { return };
    let t = crate::theme::Tokens::get(ui.ctx());
    ui.label(egui::RichText::new(format!("{:.0}%", r.fraction() * 100.0)).color(t.text_dim).size(11.5).monospace());
    ui.label(egui::RichText::new(tl!("Redrawing")).color(t.text_dim).size(12.0));
}

/// Stop document `doc`'s redraw (its canvas was redrawn some other way, or is gone).
pub(crate) fn cancel(app: &mut PhotocraftApp, doc: DocId) {
    app.redraws.remove(&doc);
}

/// Stop redraws of documents that are no longer open.
pub(crate) fn retain(app: &mut PhotocraftApp) {
    let docs = app.session.documents();
    app.redraws.retain(|id, _| docs.iter().any(|st| st.doc.id == *id));
    app.kept.retain(|id| docs.iter().any(|st| st.doc.id == id));
    let gamma = text_gamma(app);
    for r in app.redraws.values_mut() {
        r.forget_if_gamma_changed(gamma);
    }
    // Not only when the next redraw starts: states History dropped (purged or trimmed) go at once,
    // and the cap is taken again when a kept document (its pixels and History) or Memory Usage
    // changed.
    app.kept.entries.retain(|k| k.doc.strong_count() > 0);
    if app.kept.entries.is_empty() {
        return;
    }
    let docs = app.session.documents();
    let mut ids: Vec<(DocId, u64)> = app.kept.entries.iter().filter_map(|k| docs.iter().find(|st| st.doc.id == k.id).map(|st| (k.id, st.revision))).collect();
    ids.dedup();
    let capped = (ids, app.session.prefs().performance.history_budget_bytes() as u64);
    if capped != app.kept.capped {
        let cap = capped.0.iter().map(|&(id, _)| keep_cap(app, id)).min().unwrap_or(0);
        app.kept.capped = capped;
        app.kept.trim(cap);
    }
}

/// What [`spawn`] starts a redraw with.
struct Spawn {
    doc: Arc<Document>,
    display: Option<Arc<CanvasDisplay>>,
    display_key: u64,
    text_gamma: u32,
    format: eframe::wgpu::TextureFormat,
    visible: Rect,
    blocks: Vec<Rect>,
    pending: Vec<bool>,
    /// When it started and the pixels uploaded so far.
    since: (f64, u64),
    fallback: Option<crate::render_path::CpuReason>,
    gpu_attempt_ms: f64,
    /// Blocks collected for [`Kept`], their bytes, and the cap.
    keep: (Vec<Option<Vec<u8>>>, u64, u64),
}

fn spawn(app: &mut PhotocraftApp, ctx: &egui::Context, s: Spawn) {
    let Spawn {
        doc,
        display,
        display_key,
        text_gamma,
        format,
        visible,
        blocks,
        pending,
        since: (started_ms, drawn),
        fallback,
        gpu_attempt_ms,
        keep: (collected, collected_bytes, cap),
    } = s;
    let id = doc.id;
    let kept_doc = doc.clone();
    let todo = order(&blocks, (0..blocks.len()).filter(|&i| pending[i]), visible);
    let left = todo.len();
    let (tx, rx) = std::sync::mpsc::sync_channel(BATCH);
    let stop = Arc::new(AtomicBool::new(false));
    #[cfg(test)]
    let reached_hold = Arc::new(AtomicBool::new(false));
    #[cfg(test)]
    let worker_generation = NEXT_WORKER_GENERATION.fetch_add(1, Ordering::SeqCst);
    #[cfg(test)]
    let worker_hold = reached_hold.clone();
    let work = (blocks.clone(), stop.clone(), ctx.clone());
    let spawned = std::thread::Builder::new().name("canvas-redraw".into()).spawn(move || {
        let (blocks, stop, ctx) = work;
        // A panic stops the redraw: the receiver sees the worker gone and drops it.
        let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            #[cfg(test)]
            draw(&doc, display.as_deref(), format, &blocks, &todo, &stop, &tx, &ctx, &worker_hold);
            #[cfg(not(test))]
            draw(&doc, display.as_deref(), format, &blocks, &todo, &stop, &tx, &ctx);
        }));
        ctx.request_repaint();
    });
    if let Err(e) = spawned {
        log::warn!("couldn't start the canvas redraw: {e}");
        return;
    }
    app.redraws.insert(
        id,
        Redraw {
            blocks,
            pending,
            left,
            drawn,
            rx,
            stop,
            started_ms,
            fallback,
            gpu_attempt_ms,
            doc: kept_doc,
            display: display_key,
            text_gamma,
            format,
            collected,
            collected_bytes,
            cap,
            #[cfg(test)]
            reached_hold,
            #[cfg(test)]
            worker_generation,
        },
    );
}

/// The worker: composite `todo`'s blocks of `doc` in batches and hand each over, until done or
/// stopped.
#[allow(clippy::too_many_arguments)]
fn draw(
    doc: &Document,
    display: Option<&CanvasDisplay>,
    format: eframe::wgpu::TextureFormat,
    blocks: &[Rect],
    todo: &[usize],
    stop: &AtomicBool,
    tx: &SyncSender<Piece>,
    ctx: &egui::Context,
    #[cfg(test)] reached_hold: &AtomicBool,
) {
    for batch in todo.chunks(BATCH) {
        // A test may hold the worker here; a cancelled redraw must still exit, not wait for release.
        #[cfg(test)]
        while HOLD.load(Ordering::SeqCst) && !stop.load(Ordering::Relaxed) {
            // This worker, identified by its own acknowledgement, reached the hold: a replacement
            // spawned after the edit starts with another one and can't satisfy the test's wait.
            reached_hold.store(true, Ordering::SeqCst);
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
        if stop.load(Ordering::Relaxed) {
            return;
        }
        // Never reached in the browser ([`enabled`]); it only has to build there, without rayon.
        #[cfg(not(target_arch = "wasm32"))]
        let batch = batch.par_iter();
        #[cfg(target_arch = "wasm32")]
        let batch = batch.iter();
        let pieces: Vec<Piece> = batch
            .map(|&block| Piece { block, texels: crate::gpu_canvas::display_texels(format, display, &photocraft_compose::render(doc, blocks[block])) })
            .collect();
        for p in pieces {
            if stop.load(Ordering::Relaxed) || tx.send(p).is_err() {
                return;
            }
            ctx.request_repaint();
        }
    }
}

#[cfg(test)]
mod tests;
