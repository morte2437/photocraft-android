//! GPU execution time of a composite (#7186), measured with wgpu timestamp queries.
//!
//! `GpuCanvas::refresh` reports `composite_ms`, the CPU cost of *encoding and submitting* the
//! composite. That is not the cost of running it: at 24 MP the encode costs ~7.7 ms and the GPU
//! then spends ~171 ms finishing the work, so every consumer of `composite_ms` under-reports a
//! GPU-bound canvas by more than an order of magnitude and cannot see a regression that moves
//! work onto the GPU.
//!
//! Waiting for the GPU to report the true number would serialise the two processors and make the
//! app slower, so this measures instead of waiting: a timestamp is written on the GPU timeline at
//! the start and end of every command encoder a refresh submits, and the values are read back
//! asynchronously. The sum of the spans is the time the GPU spent executing that refresh, with
//! the CPU encoding gaps between submissions excluded. Nothing blocks; the numbers simply arrive
//! a frame or two later, so each measurement carries the pixel count of the refresh it belongs to
//! and is never attributed to whichever refresh happens to be in flight when it lands.
//!
//! **What it does not count.** The spans run between bottom-of-pipe timestamps inside the
//! refresh's own command buffers. Two kinds of GPU work fall outside them: queue-level uploads
//! (`Queue::write_texture`), which wgpu executes in its own command buffer before ours, and
//! whatever the GPU still does after the final timestamp retires — the end-of-batch writeback of
//! the render targets. Measured against a waited-for wall clock on Intel UHD P630 / Mesa, with
//! no uploads outstanding, the uncounted remainder is **~5 % of the reported time**, and it is
//! proportional to the composite rather than a fixed submit latency (1.0 ms at 6 MP, 4.5 ms at
//! 24 MP, 7.6 ms at 42.6 MP, against 21 / 83 / 158 ms reported). So the number is a slight
//! under-report of the GPU's total contribution, by a few per cent rather than the ~22x of the
//! encode-and-submit time it replaces.
//!
//! **What it costs.** Nothing measurable. The same refresh on the same device with
//! `PHOTOCRAFT_NO_GPU_TIMING` set waits 88.1 ms against 87.9 ms measured — a difference of
//! −0.2 ms on an ~88 ms refresh, i.e. inside the noise of a shared host. Per refresh the
//! queries add two timestamp writes (two more per flush), one resolve and one copy of at most
//! 1 KiB, a `map_async` and an `Arc` allocation; the query set and both buffers are allocated
//! once per compositor and are size-independent, so a resize reallocates nothing.
//!
//! The measurement is *optional by construction*. Without `TIMESTAMP_QUERY` and
//! `TIMESTAMP_QUERY_INSIDE_ENCODERS` on the device (older drivers, most browsers, the Noop
//! backend), [`GpuTimer::new`] returns `None` and callers report no GPU time at all rather than a
//! plausible wrong number — reporting a plausible wrong number is the failure this fixes.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

/// The GPU execution time of one composite, with the refresh it measured.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GpuTiming {
    /// Time the GPU spent executing the refresh's command buffers (ms). Excludes queue-level
    /// uploads, which run in wgpu's own command buffer ahead of ours, and the end-of-batch work
    /// after the last timestamp: see the module doc for the size of that (~5 % here, more on a
    /// refresh that uploads).
    pub ms: f64,
    /// Pixels that refresh composited (so a late measurement stays interpretable).
    pub px: u64,
    /// Command buffers the refresh submitted (1 unless it flushed).
    pub spans: u32,
}

/// Device features [`GpuTimer`] needs.
pub const FEATURES: wgpu::Features = wgpu::Features::TIMESTAMP_QUERY.union(wgpu::Features::TIMESTAMP_QUERY_INSIDE_ENCODERS);

/// Spans one measurement may hold. A refresh submits once per flush; a 200 MP document flushes a
/// handful of times, so this is far above anything real. A measurement that would exceed it is
/// dropped rather than truncated (a truncated sum is exactly the under-report being fixed).
const SPANS: u32 = 64;
/// Any span total above this is a wrapped or garbage timestamp, not a composite.
const SANE_MS: f64 = 600_000.0;

/// Readback state of the buffer holding the resolved timestamps.
const UNMAPPED: u8 = 0;
const MAPPED: u8 = 1;
const FAILED: u8 = 2;

/// Timestamp queries for one compositor, reused for every refresh.
///
/// At most one measurement is in flight: while the last one is still being read back a refresh
/// goes unmeasured, which costs a sample rather than correctness.
pub struct GpuTimer {
    set: wgpu::QuerySet,
    /// Resolved timestamps (`QUERY_RESOLVE | COPY_SRC`).
    resolve: wgpu::Buffer,
    /// Host-visible copy of `resolve` (`MAP_READ | COPY_DST`).
    read: wgpu::Buffer,
    /// Nanoseconds per timestamp tick, as the queue reports it.
    period_ns: f32,
    /// Timestamps written into the current recording.
    written: u32,
    /// Whether a span is open (a start timestamp waiting for its end).
    open: bool,
    /// Pixels of the recording in progress.
    px: u64,
    /// The recording overflowed [`SPANS`]: finish without resolving.
    overflowed: bool,
    /// The recording in progress has already had command buffers submitted (it flushed), so its
    /// query slots may still be in use by the GPU.
    flushed: bool,
    /// A recording was abandoned after it had already submitted timestamps. Writing a query
    /// again resets that slot (wgpu-core resets per write), and resetting one an in-flight
    /// submission is still writing is undefined, with no way to learn when it has drained.
    /// That only happens when a refresh fails after a flush that nothing waited for (a refresh
    /// whose page retry waited for the GPU calls [`Self::drained`] first), which is rare: usually
    /// the device faulted and the canvas is about to drop to the CPU compositor anyway. So
    /// measuring simply stops: no number, which is the contract everywhere else here.
    stopped: bool,
    /// Timestamps and pixels of the measurement being read back, and its readback flag.
    in_flight: Option<(u32, u64, Arc<AtomicU8>)>,
    /// [`Self::finish`] recorded a readback that [`Self::submitted`] has yet to map. Without
    /// this, a refresh that starts before the last measurement lands would map the readback
    /// buffer a second time while the first map is still pending, which wgpu rejects.
    unmapped: bool,
    /// The most recent completed measurement, until a caller takes it.
    ready: Option<GpuTiming>,
}

impl std::fmt::Debug for GpuTimer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("GpuTimer").field("recording", &self.open).field("in_flight", &self.in_flight.is_some()).field("ready", &self.ready).finish()
    }
}

impl GpuTimer {
    /// A timer for `device`, or `None` when it has no timestamp queries (the caller then reports
    /// no GPU time, never an estimate).
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue) -> Option<Self> {
        if !device.features().contains(FEATURES) {
            return None;
        }
        let period_ns = queue.get_timestamp_period();
        if !period_ns.is_finite() || period_ns <= 0.0 {
            return None;
        }
        let bytes = u64::from(SPANS) * 2 * 8;
        Some(Self {
            set: device.create_query_set(&wgpu::QuerySetDescriptor { label: Some("pc_compose_time"), ty: wgpu::QueryType::Timestamp, count: SPANS * 2 }),
            resolve: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pc_compose_time_resolve"),
                size: bytes,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            }),
            read: device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("pc_compose_time_read"),
                size: bytes,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            }),
            period_ns,
            written: 0,
            open: false,
            px: 0,
            overflowed: false,
            flushed: false,
            stopped: false,
            in_flight: None,
            unmapped: false,
            ready: None,
        })
    }

    /// Start measuring a refresh of `px` pixels, opening its first span in `encoder`. Does
    /// nothing while a previous measurement is still being read back (that refresh goes
    /// unmeasured) — call [`Self::collect`] first to give it the chance to land.
    pub fn start(&mut self, encoder: &mut wgpu::CommandEncoder, px: u64) {
        if self.stopped || self.in_flight.is_some() || self.open {
            return;
        }
        self.written = 0;
        self.overflowed = false;
        self.flushed = false;
        self.px = px;
        self.open = true;
        self.stamp(encoder);
    }

    /// Close the span in `done` and open the next in `next`: one refresh that submits several
    /// command buffers is one measurement, summed over them.
    pub fn split(&mut self, done: &mut wgpu::CommandEncoder, next: &mut wgpu::CommandEncoder) {
        if !self.open {
            return;
        }
        self.stamp(done);
        self.stamp(next);
        self.flushed = true;
    }

    /// Close the last span and record the resolve and readback copy into `encoder`, which the
    /// caller is about to submit. Call [`Self::submitted`] once it has.
    pub fn finish(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.open {
            return;
        }
        self.stamp(encoder);
        self.open = false;
        let count = self.written;
        if self.overflowed || count < 2 || !count.is_multiple_of(2) {
            self.abandon();
            return;
        }
        let bytes = u64::from(count) * 8;
        encoder.resolve_query_set(&self.set, 0..count, &self.resolve, 0);
        encoder.copy_buffer_to_buffer(&self.resolve, 0, &self.read, 0, bytes);
        self.in_flight = Some((count, self.px, Arc::new(AtomicU8::new(UNMAPPED))));
        self.unmapped = true;
        self.written = 0;
        // This recording is over: its slots are protected by `in_flight` until the readback
        // lands, not by `flushed`. Leaving `flushed` set would let a later refresh that
        // recorded nothing (because `start` found the readback still in flight) stop measuring
        // for good when it failed, on the strength of this recording's flush.
        self.flushed = false;
    }

    /// Map the readback buffer after the caller submitted the encoder [`Self::finish`] wrote
    /// into. The map completes on a later device poll; nothing blocks here.
    pub fn submitted(&mut self) {
        if !self.unmapped {
            return;
        }
        self.unmapped = false;
        let Some((count, _, flag)) = &self.in_flight else { return };
        let (bytes, flag) = (u64::from(*count) * 8, flag.clone());
        self.read.slice(0..bytes).map_async(wgpu::MapMode::Read, move |r| {
            flag.store(if r.is_ok() { MAPPED } else { FAILED }, Ordering::Release);
        });
    }

    /// Abandon the recording in progress (the refresh failed part way, so its spans mean
    /// nothing).
    pub fn cancel(&mut self) {
        self.open = false;
        self.abandon();
    }

    /// Drop the recording in progress. Once its timestamps have reached the GPU its slots can't
    /// be reused safely, so measuring stops for good rather than resetting a live query.
    fn abandon(&mut self) {
        self.stopped |= self.flushed;
        self.written = 0;
        self.overflowed = false;
        self.flushed = false;
    }

    /// The caller waited until the GPU finished everything submitted so far: no timestamp of
    /// this recording is still in flight, so [`Self::cancel`] can reuse its slots safely.
    pub fn drained(&mut self) {
        self.flushed = false;
    }

    /// Whether a measurement is being read back (so this refresh won't be measured).
    pub fn busy(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Read back a finished measurement if the map completed, and return whatever is ready.
    /// Cheap and non-blocking; callers poll it per refresh or per frame.
    pub fn collect(&mut self) -> Option<GpuTiming> {
        if let Some((count, px, flag)) = &self.in_flight {
            match flag.load(Ordering::Acquire) {
                MAPPED => {
                    let (count, px) = (*count, *px);
                    self.ready = self.value(count, px);
                    self.read.unmap();
                    self.in_flight = None;
                }
                FAILED => {
                    self.in_flight = None;
                }
                _ => {}
            }
        }
        self.ready
    }

    /// Put a measurement back after a caller took it but could not use it (its refresh failed),
    /// so the sample is reported by the next one instead of being lost on the fallback path.
    /// A measurement that has landed since wins: it is the newer of the two.
    pub fn restore(&mut self, t: GpuTiming) {
        self.ready.get_or_insert(t);
    }

    /// Take the most recent completed measurement, leaving none behind, so a stale number is
    /// never reported twice as if it were new.
    pub fn take(&mut self) -> Option<GpuTiming> {
        self.collect();
        self.ready.take()
    }

    /// Sum the resolved spans of a mapped readback.
    fn value(&self, count: u32, px: u64) -> Option<GpuTiming> {
        let bytes = u64::from(count) * 8;
        let view = self.read.slice(0..bytes).get_mapped_range().ok()?;
        let mut ticks = 0u64;
        for pair in view.as_chunks::<16>().0 {
            let (a, b) = pair.split_at(8);
            let start = u64::from_le_bytes(a.try_into().ok()?);
            let end = u64::from_le_bytes(b.try_into().ok()?);
            // A driver that reset or wrapped its timer gives an end before its start: the whole
            // measurement is then unusable, and no number is better than a wrong one.
            if end < start {
                return None;
            }
            ticks += end - start;
        }
        drop(view);
        let ms = ticks as f64 * f64::from(self.period_ns) / 1.0e6;
        (ms.is_finite() && ms < SANE_MS).then_some(GpuTiming { ms, px, spans: count / 2 })
    }

    /// Write one timestamp, noting an overflow instead of wrapping round the query set.
    fn stamp(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if self.written >= SPANS * 2 {
            self.overflowed = true;
            return;
        }
        encoder.write_timestamp(&self.set, self.written);
        self.written += 1;
    }
}
