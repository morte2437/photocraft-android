//! The control channel's `ui.screenshot`: bounded, cancelable capture requests.
//!
//! egui 0.37 removed `Event::Screenshot`/`UserData`; `ViewportCommand::Screenshot` now carries a
//! one-shot callback that may run on an arbitrary thread. The shared `craft_ui::capture` slot is
//! the only correlation primitive: it stores at most one image, validates it against a per-image
//! byte bound, wakes the owning viewport, and makes a late result after cancellation or drop
//! inert. This module owns everything the slot deliberately leaves to the application:
//!
//! - request identity (token, output path, reply), deadlines and settling policy;
//! - the 4-request / 128 MiB-per-image / 256 MiB-aggregate admission budget, counted over
//!   outstanding (issued, completed-but-undrained and unissued) requests. A request that arrives
//!   before the first UI pass has recorded the geometry waits (bounded by the same count and
//!   deadline) instead of failing, so a launch-and-capture client behaves as it did on 0.15;
//! - the document/activation identity captured at admission, which is never rebound (the
//!   revision is recorded for attribution only);
//! - the capture geometry epoch recorded at issuance, so an image is only saved for the layout it
//!   was actually rendered from. Any size, scale, zoom, texture-side or minimized change advances
//!   the epoch, and an epoch advance between issuance and collection is a retry even when the
//!   values change back;
//! - a monotonic clock (`web_time::Instant`) for settling, retries and timeouts. Never
//!   `ctx.input(|i| i.time)` (stale in `run_logic`) and never the wall clock (NTP can jump).
//!
//! Nothing here writes a file or encodes a PNG: completed images are handed back to the UI thread,
//! which owns the automation writer ([`crate::control::save_screenshot_rgba`]).
//!
//! Issuance and the geometry epoch are UI-phase work ([`Captures::issue`] is called from
//! `App::ui`); draining, admission and hidden-paint requests are logic work. A hidden `run_logic`
//! tick never counts as a settling frame and never issues a capture.
//!
//! Limitation: an invisible same-size surface recreation (a backend swap that reports the same
//! size, scale and zoom) cannot be observed from egui's input and is not detected here.

use std::sync::Arc;
use std::time::Duration;

use craft_ui::capture::CaptureSlot;
use egui::{ColorImage, Context, ScreenshotCallback, ViewportId};
use web_time::Instant;

use crate::control::ControlResponse;

/// At most this many requests may be outstanding at once (issued, completed, waiting for
/// geometry or waiting to be issued).
pub const MAX_PENDING: usize = 4;
/// Largest single image the slot may retain, and the largest a request may reserve.
pub const MAX_IMAGE_BYTES: usize = 128 * 1024 * 1024;
/// Aggregate reserved bytes over every outstanding request.
pub const MAX_TOTAL_BYTES: usize = 256 * 1024 * 1024;
/// A request expires after this long, including retries and settling.
pub const DEADLINE: Duration = Duration::from_secs(8);
/// UI passes a request waits before it is issued, on top of the animation delay.
pub const SETTLE_PASSES: u32 = 3;
/// Retries after the layout/geometry changed while a capture was in flight.
pub const MAX_RETRIES: u8 = 2;
/// The base delay that lets modal/popup open-close animations finish.
const BASE_SETTLE_MS: f64 = 60.0;

/// The document identity a screenshot belongs to. Never rebound after admission.
///
/// `generation` is the session's monotonic active-document generation. It changes whenever a
/// different document becomes active, including a switch away and back inside one pass, so an
/// image is never attributed to a document the pass did not stably show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentId {
    pub document: u64,
    pub revision: u64,
    pub generation: u64,
}

impl DocumentId {
    /// Request identity is the document, not its revision: an edit while a capture is in flight
    /// must not throw the image away. The revision is recorded for attribution only.
    pub fn same_document(self, other: Self) -> bool {
        self.document == other.document
    }

    /// The same document in the same activation: an intervening switch away and back fails it.
    pub fn same_activation(self, other: Self) -> bool {
        self.same_document(other) && self.generation == other.generation
    }
}

/// The viewport geometry an image is expected to have.
///
/// Position is deliberately absent: moving a window does not change the captured surface. Size,
/// scale, zoom, the texture-side limit and the minimized state do.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CaptureGeometry {
    pub viewport: ViewportId,
    pub logical_size: egui::Vec2,
    pub pixels_per_point: f32,
    pub zoom_factor: f32,
    pub max_texture_side: usize,
    pub minimized: bool,
    pub document: Option<DocumentId>,
}

impl CaptureGeometry {
    /// Current input/host viewport geometry. A full pass refreshes all input before logic;
    /// run_logic refreshes viewport metadata while retaining the previous UI screen rectangle.
    pub fn current(ctx: &Context, document: Option<DocumentId>) -> Self {
        let (viewport, logical_size, minimized, native_scale) = ctx.input(|i| {
            let size = i.viewport().inner_rect.map_or_else(|| i.viewport_rect().size(), |r| r.size());
            (i.raw.viewport_id, size, i.viewport().minimized.unwrap_or(false), i.viewport().native_pixels_per_point)
        });
        Self {
            viewport,
            logical_size,
            pixels_per_point: native_scale
                .filter(|scale| scale.is_finite() && *scale > 0.0)
                .map_or_else(|| ctx.pixels_per_point(), |scale| scale * ctx.zoom_factor()),
            zoom_factor: ctx.zoom_factor(),
            max_texture_side: ctx.input(|i| i.max_texture_side),
            minimized,
            document,
        }
    }

    /// The identity compared for stale-image rejection. Window movement and the document identity
    /// are separate concerns (the latter is compared exactly and never retried).
    fn epoch_key(&self) -> EpochKey {
        EpochKey {
            viewport: self.viewport,
            width: self.logical_size.x,
            height: self.logical_size.y,
            ppp: self.pixels_per_point,
            zoom: self.zoom_factor,
            max_texture_side: self.max_texture_side,
            minimized: self.minimized,
        }
    }

    /// The expected image size in pixels, from the logical size and the point scale.
    ///
    /// `None` for a degenerate viewport; a request is rejected before issuance rather than
    /// guessing. A window larger than `max_texture_side` may be fitted by the renderer (E2), so
    /// the size itself is not rejected here; [`check_image`] accepts a uniform fit bounded by
    /// that texture side.
    pub fn expected_pixels(&self) -> Option<[usize; 2]> {
        let valid = |v: f32| v.is_finite() && v > 0.0;
        if !valid(self.logical_size.x) || !valid(self.logical_size.y) || !valid(self.pixels_per_point) || !valid(self.zoom_factor) {
            return None;
        }
        let w = (self.logical_size.x * self.pixels_per_point).round();
        let h = (self.logical_size.y * self.pixels_per_point).round();
        if w < 1.0 || h < 1.0 || w > usize::MAX as f32 || h > usize::MAX as f32 {
            return None;
        }
        Some([w as usize, h as usize])
    }

    /// The retained bytes this geometry reserves, from checked arithmetic.
    pub fn expected_bytes(&self) -> Option<usize> {
        let [w, h] = self.expected_pixels()?;
        w.checked_mul(h)?.checked_mul(std::mem::size_of::<egui::Color32>())
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct EpochKey {
    viewport: ViewportId,
    width: f32,
    height: f32,
    ppp: f32,
    zoom: f32,
    max_texture_side: usize,
    minimized: bool,
}

/// One request that arrived before a UI pass had recorded the window geometry. It is admitted on
/// the first logic tick after that pass, within the same count/deadline budget.
struct WaitingAdmit {
    token: u64,
    path: Option<String>,
    reply: std::sync::mpsc::Sender<ControlResponse>,
    document: Option<DocumentId>,
    settle_ms: f64,
    arrived: Instant,
    deadline: Instant,
}

/// One outstanding request. The record is app-owned and holds the strong slot; the screenshot
/// callback only holds a weak reference to the slot's state.
pub struct PendingCapture {
    token: u64,
    path: Option<String>,
    reply: Option<std::sync::mpsc::Sender<ControlResponse>>,
    deadline: Instant,
    ready_at: Instant,
    settle_passes: u32,
    geometry: CaptureGeometry,
    /// Geometry epoch of `geometry`: at admission, and again whenever it is finalized after a
    /// layout change. Issuance restarts settling when it differs from the current epoch.
    epoch: u64,
    /// Epoch of the UI pass the capture command was issued from. A later advance is a retry.
    issued_epoch: Option<u64>,
    reserved_bytes: usize,
    document: Option<DocumentId>,
    issued: bool,
    slot: Option<CaptureSlot>,
    retries: u8,
}

impl PendingCapture {
    pub fn token(&self) -> u64 {
        self.token
    }

    pub fn issued(&self) -> bool {
        self.issued
    }
}

impl core::fmt::Debug for PendingCapture {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PendingCapture")
            .field("token", &self.token)
            .field("issued", &self.issued)
            .field("retries", &self.retries)
            .field("epoch", &self.epoch)
            .field("issued_epoch", &self.issued_epoch)
            .finish_non_exhaustive()
    }
}

/// A request that is over: saved-and-replied by the caller, or failed with `result`.
pub struct Finished {
    pub token: u64,
    pub path: Option<String>,
    pub reply: Option<std::sync::mpsc::Sender<ControlResponse>>,
    pub result: Result<Arc<ColorImage>, String>,
}

impl core::fmt::Debug for Finished {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Finished").field("token", &self.token).field("path", &self.path).field("ok", &self.result.is_ok()).finish_non_exhaustive()
    }
}

/// The screenshot request queue and its admission budget.
#[derive(Default)]
pub struct Captures {
    records: Vec<PendingCapture>,
    /// Arrived before the first UI pass recorded the geometry.
    waiting: Vec<WaitingAdmit>,
    geometry: Option<CaptureGeometry>,
    epoch: u64,
}

impl Captures {
    /// Every outstanding request, including ones still waiting for the first UI pass.
    pub fn pending(&self) -> usize {
        self.records.len().saturating_add(self.waiting.len())
    }

    pub fn reserved_bytes(&self) -> usize {
        self.records.iter().map(|r| r.reserved_bytes).sum()
    }

    pub fn any_pending(&self) -> bool {
        !self.records.is_empty() || !self.waiting.is_empty()
    }

    /// Requests still waiting to be issued; only these need `RequestPaintWhileHidden` from logic
    /// (an issued capture's own `Screenshot` command forces its paint).
    pub fn any_unissued(&self) -> bool {
        !self.waiting.is_empty() || self.records.iter().any(|r| !r.issued)
    }

    pub fn last_geometry(&self) -> Option<CaptureGeometry> {
        self.geometry
    }

    /// Observe host/input geometry before callback delivery or UI issuance. This may advance the
    /// epoch in logic, but never counts a settling pass: only App::ui calls note_pass.
    pub fn note_geometry(&mut self, geometry: CaptureGeometry) -> u64 {
        let changed = self.geometry.is_none_or(|old| old.epoch_key() != geometry.epoch_key());
        if changed {
            self.epoch = self.epoch.wrapping_add(1);
        }
        self.geometry = Some(geometry);
        self.epoch
    }

    /// Admit a request before it is issued. The queue must have been drained first, so completed
    /// images have already released their budget. With no UI pass yet (a launch-and-capture
    /// client) the request is parked within the same count and deadline budget.
    pub fn admit(
        &mut self,
        token: u64,
        path: Option<String>,
        reply: std::sync::mpsc::Sender<ControlResponse>,
        document: Option<DocumentId>,
        settle_ms: f64,
        now: Instant,
    ) -> Result<(), (std::sync::mpsc::Sender<ControlResponse>, String)> {
        let reject = |reply: std::sync::mpsc::Sender<ControlResponse>, e: String| Err((reply, e));
        if self.records.iter().any(|r| r.token == token) || self.waiting.iter().any(|w| w.token == token) {
            return reject(reply, format!("screenshot token {token} is already pending"));
        }
        if self.pending() >= MAX_PENDING {
            return reject(reply, format!("too many screenshots pending (at most {MAX_PENDING})"));
        }
        if self.geometry.is_none() {
            self.waiting.push(WaitingAdmit { token, path, reply, document, settle_ms, arrived: now, deadline: now + DEADLINE });
            return Ok(());
        }
        self.admit_with_geometry(token, path, reply, document, settle_ms, now)
    }

    /// Admit requests parked before the first UI pass, once the geometry is known. Called from
    /// logic before `drain` processes completions; expired parked requests get their error here.
    pub fn admit_waiting(&mut self, now: Instant) {
        if self.waiting.is_empty() {
            return;
        }
        let parked = std::mem::take(&mut self.waiting);
        let mut still_waiting = Vec::new();
        for w in parked {
            if now >= w.deadline {
                let _ = w.reply.send(serde_json::json!({"ok": false, "error": timeout_error(w.token)}));
                continue;
            }
            if self.geometry.is_none() {
                still_waiting.push(w);
                continue;
            }
            if let Err((reply, e)) = self.admit_with_geometry(w.token, w.path, w.reply, w.document, w.settle_ms, w.arrived) {
                let _ = reply.send(serde_json::json!({"ok": false, "error": e}));
            }
        }
        self.waiting = still_waiting;
    }

    fn admit_with_geometry(
        &mut self,
        token: u64,
        path: Option<String>,
        reply: std::sync::mpsc::Sender<ControlResponse>,
        document: Option<DocumentId>,
        settle_ms: f64,
        now: Instant,
    ) -> Result<(), (std::sync::mpsc::Sender<ControlResponse>, String)> {
        // The reply is returned on every rejection so the caller can still answer exactly once.
        let reject = |reply: std::sync::mpsc::Sender<ControlResponse>, e: String| Err((reply, e));
        if self.records.iter().any(|r| r.token == token) {
            return reject(reply, format!("screenshot token {token} is already pending"));
        }
        if self.records.len() >= MAX_PENDING {
            return reject(reply, format!("too many screenshots pending (at most {MAX_PENDING})"));
        }
        let Some(geometry) = self.geometry else {
            return reject(reply, "the window geometry is not known yet; try again after a frame has been drawn".into());
        };
        let expected = match image_budget(&geometry) {
            Ok(bytes) => bytes,
            Err(e) => return reject(reply, e),
        };
        if let Err(e) = aggregate_budget(self.reserved_bytes(), expected) {
            return reject(reply, e);
        }
        // `Duration::from_secs_f64` panics on NaN/inf/negative: never trust a style value here.
        let settle = if settle_ms.is_finite() { settle_ms.clamp(0.0, 2000.0) } else { BASE_SETTLE_MS };
        self.records.push(PendingCapture {
            token,
            path,
            reply: Some(reply),
            deadline: now + DEADLINE,
            ready_at: now + Duration::from_secs_f64(settle / 1000.0),
            settle_passes: 0,
            geometry,
            epoch: self.epoch,
            issued_epoch: None,
            reserved_bytes: expected,
            document,
            issued: false,
            slot: None,
            retries: 0,
        });
        Ok(())
    }

    /// Count one UI pass towards settling (logic-only ticks never call this).
    pub fn note_pass(&mut self) {
        for r in &mut self.records {
            r.settle_passes = r.settle_passes.saturating_add(1);
        }
    }

    /// Issue every due request. Call from `App::ui`; `send` installs the callback with
    /// `ViewportCommand::Screenshot`. A headless test can capture the callbacks instead.
    pub fn issue(&mut self, ctx: &Context, now: Instant, document: Option<DocumentId>, mut send: impl FnMut(ScreenshotCallback)) -> Vec<Finished> {
        let mut finished = Vec::new();
        let Some(current) = self.geometry else { return finished };
        // The pass's start and end must show the same active document: a switch inside the pass
        // means neither the drawn frame nor the pass's document identity is trustworthy.
        let pass_changed = !same_activation(current.document, document);
        for i in 0..self.records.len() {
            if self.records[i].issued {
                continue;
            }
            let token = self.records[i].token;
            if now >= self.records[i].deadline {
                finished.push(finish(&mut self.records[i], Err(timeout_error(token))));
                continue;
            }
            if !same_activation(document, self.records[i].document) || !same_activation(current.document, self.records[i].document) {
                finished.push(finish(&mut self.records[i], Err(document_changed_error(token))));
                continue;
            }
            if pass_changed {
                continue;
            }
            // A minimized window has no surface to read back; fail with an actionable error
            // instead of waiting for the 8 s timeout.
            if current.minimized {
                finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: the window is minimized"))));
                continue;
            }
            if self.records[i].settle_passes < SETTLE_PASSES || now < self.records[i].ready_at {
                continue;
            }
            // No pixels exist yet: finalize the geometry at issuance, re-checking the budget, and
            // let the new layout settle for another pass.
            if self.records[i].epoch != self.epoch || current.epoch_key() != self.records[i].geometry.epoch_key() {
                let others: usize = self.records.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| r.reserved_bytes).sum();
                let epoch = self.epoch;
                match revalidate_reservation(&mut self.records[i], others, current, epoch) {
                    Ok(()) => {}
                    Err(e) => finished.push(finish(&mut self.records[i], Err(e))),
                }
                continue;
            }
            let (slot, callback) = CaptureSlot::new(ctx, current.viewport, MAX_IMAGE_BYTES);
            let r = &mut self.records[i];
            r.slot = Some(slot);
            r.issued = true;
            r.issued_epoch = Some(self.epoch);
            send(callback);
        }
        self.records.retain(|r| !finished.iter().any(|f| f.token == r.token));
        finished
    }

    /// Collect completed requests and enforce deadlines. Call from logic. The deadline is checked
    /// before a completed image is saved, so a callback that arrives late is a timeout, not a
    /// successful capture.
    pub fn drain(&mut self, now: Instant, current: Option<CaptureGeometry>, document: Option<DocumentId>) -> Vec<Finished> {
        // Requests that arrived before the first UI pass start here, once its geometry is known.
        self.admit_waiting(now);
        let mut finished = Vec::new();
        for i in 0..self.records.len() {
            let token = self.records[i].token;
            if now >= self.records[i].deadline {
                if let Some(slot) = self.records[i].slot.take() {
                    slot.cancel();
                }
                finished.push(finish(&mut self.records[i], Err(timeout_error(token))));
                continue;
            }
            if !same_activation(document, self.records[i].document) || !same_activation(current.and_then(|c| c.document), self.records[i].document) {
                if let Some(slot) = self.records[i].slot.take() {
                    slot.cancel();
                }
                finished.push(finish(&mut self.records[i], Err(document_changed_error(token))));
                continue;
            }
            let Some(slot) = self.records[i].slot.take() else { continue };
            let Some(result) = slot.take() else {
                // Still in flight: keep the live request (and its strong slot) in the record.
                self.records[i].slot = Some(slot);
                continue;
            };
            let image = match result {
                Ok(image) => image,
                Err(e) => {
                    slot.cancel();
                    finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: {e}"))));
                    continue;
                }
            };
            let Some(current) = current else {
                finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: the window geometry is unknown"))));
                continue;
            };
            // An epoch advance means the surface changed after issuance, even if it later came
            // back to the same values: the image may belong to the intermediate layout.
            let epoch_advanced = self.records[i].issued_epoch.is_none_or(|issued| issued != self.epoch);
            let pass_mixed = !same_activation(current.document, document);
            if epoch_advanced || current.epoch_key() != self.records[i].geometry.epoch_key() || pass_mixed {
                slot.cancel();
                if self.records[i].retries >= MAX_RETRIES {
                    finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: the layout changed while it was captured"))));
                    continue;
                }
                self.records[i].retries = self.records[i].retries.saturating_add(1);
                let others: usize = self.records.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, r)| r.reserved_bytes).sum();
                let epoch = self.epoch;
                match revalidate_reservation(&mut self.records[i], others, current, epoch) {
                    Ok(()) => {
                        self.records[i].ready_at = now;
                    }
                    Err(e) => finished.push(finish(&mut self.records[i], Err(e))),
                }
                continue;
            }
            if let Err(e) = check_image(&image, &self.records[i].geometry) {
                finished.push(finish(&mut self.records[i], Err(format!("screenshot {token} failed: {e}"))));
                continue;
            }
            slot.cancel();
            finished.push(finish(&mut self.records[i], Ok(image)));
        }
        self.records.retain(|r| !finished.iter().any(|f| f.token == r.token));
        finished
    }

    /// Drop every outstanding request (application exit). No reply is invented for a request that
    /// was already finished; `Finished` still carries the sender so the caller can report it.
    pub fn cancel_all(&mut self, reason: &str) -> Vec<Finished> {
        let mut finished = Vec::new();
        for w in self.waiting.drain(..) {
            finished.push(Finished { token: w.token, path: w.path, reply: Some(w.reply), result: Err(format!("screenshot {} cancelled: {reason}", w.token)) });
        }
        for mut r in self.records.drain(..) {
            if let Some(slot) = r.slot.take() {
                slot.cancel();
            }
            finished.push(Finished {
                token: r.token,
                path: r.path.take(),
                reply: r.reply.take(),
                result: Err(format!("screenshot {} cancelled: {reason}", r.token)),
            });
        }
        finished
    }
}

/// The retained bytes a geometry may reserve, or the reason it cannot be captured at all.
fn image_budget(geometry: &CaptureGeometry) -> Result<usize, String> {
    let Some(expected) = geometry.expected_bytes() else {
        return Err(format!(
            "the viewport is not capturable ({}×{} points at {} px/pt, texture side {})",
            geometry.logical_size.x, geometry.logical_size.y, geometry.pixels_per_point, geometry.max_texture_side
        ));
    };
    if expected > MAX_IMAGE_BYTES {
        return Err(format!("a {} MiB screenshot is over the {} MiB per-image limit", expected / (1 << 20), MAX_IMAGE_BYTES / (1 << 20)));
    }
    Ok(expected)
}

/// Check one new reservation against the aggregate budget, excluding the request itself.
fn aggregate_budget(others: usize, expected: usize) -> Result<(), String> {
    let total = others.saturating_add(expected);
    if total > MAX_TOTAL_BYTES {
        return Err(format!("screenshot budget exhausted: {total} bytes reserved of a {MAX_TOTAL_BYTES}-byte aggregate limit"));
    }
    Ok(())
}

/// Finalize a request at a new geometry: recheck per-image and aggregate limits (excluding its
/// old reservation), then take the new reservation and restart settling.
fn revalidate_reservation(r: &mut PendingCapture, others: usize, geometry: CaptureGeometry, epoch: u64) -> Result<(), String> {
    let expected = image_budget(&geometry)?;
    aggregate_budget(others, expected)?;
    r.reserved_bytes = expected;
    r.geometry = geometry;
    r.epoch = epoch;
    r.issued_epoch = None;
    r.issued = false;
    r.settle_passes = 0;
    Ok(())
}

fn finish(r: &mut PendingCapture, result: Result<Arc<ColorImage>, String>) -> Finished {
    if let Some(slot) = r.slot.take() {
        slot.cancel();
    }
    Finished { token: r.token, path: r.path.take(), reply: r.reply.take(), result }
}

/// The returned image must be the issued viewport (within a pixel) or a uniform renderer fit
/// bounded by `max_texture_side` (E2). An arbitrarily smaller same-aspect image is not accepted:
/// it could be a stale or downscaled surface, not the frame that was asked for. A different
/// aspect ratio is a resize/recreation and must not be labelled with the old geometry.
fn check_image(image: &ColorImage, geometry: &CaptureGeometry) -> Result<(), String> {
    let Some([w, h]) = geometry.expected_pixels() else {
        return Err("the captured geometry has no valid pixel size".into());
    };
    let [iw, ih] = image.size;
    if iw == 0 || ih == 0 {
        return Err("the image has a zero side".into());
    }
    if iw > w.saturating_add(1) || ih > h.saturating_add(1) {
        return Err(format!("the image is {iw}×{ih}, larger than the issued viewport {w}×{h}"));
    }
    let close = |a: usize, b: usize| a.abs_diff(b) <= 1;
    if close(iw, w) && close(ih, h) {
        return Ok(());
    }
    let side = geometry.max_texture_side.max(1);
    let fit = (side as f64 / w.max(h) as f64).min(1.0);
    let fw = (w as f64 * fit).round() as usize;
    let fh = (h as f64 * fit).round() as usize;
    if fw >= 1 && fh >= 1 && close(iw, fw) && close(ih, fh) {
        return Ok(());
    }
    let expected_aspect = w as f32 / h as f32;
    let actual_aspect = iw as f32 / ih as f32;
    if !actual_aspect.is_finite() || (actual_aspect - expected_aspect).abs() > expected_aspect * 0.02 {
        return Err(format!("the image is {iw}×{ih}, not the issued {w}×{h} aspect ratio"));
    }
    Err(format!("the image is {iw}×{ih}, neither the issued {w}×{h} viewport nor its {fw}×{fh} texture-side fit"))
}

/// Whether two optional document identities are the same document in the same activation.
fn same_activation(a: Option<DocumentId>, b: Option<DocumentId>) -> bool {
    match (a, b) {
        (Some(a), Some(b)) => a.same_activation(b),
        (None, None) => true,
        _ => false,
    }
}

fn timeout_error(token: u64) -> String {
    format!("screenshot {token} timed out: no frame was presented within {} s", DEADLINE.as_secs())
}

fn document_changed_error(token: u64) -> String {
    format!("screenshot {token} failed: the document changed while the capture was queued")
}

/// The settle delay for a style's animation time, in milliseconds (egui's open/close fades).
pub fn settle_ms(animation_time: f32) -> f64 {
    f64::from(animation_time) * 2000.0 + BASE_SETTLE_MS
}

/// A monotonic clock for capture lifecycles.
pub fn now() -> Instant {
    Instant::now()
}

/// The `Screenshot` command that carries `callback` to the renderer.
pub fn send_command(ctx: &Context, callback: ScreenshotCallback) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(callback));
}
