//! Where each frame's time goes (#9328), gathered every frame, and the slow ones kept.
//!
//! [`FrameStages`] is one frame's record. Each stage has one writer: the raw input hook starts
//! the frame, `App::run` adds commands, the GPU canvas path adds damage tracking and budgeting,
//! [`crate::gpu_canvas::Perf::record`] adds composites and uploads, and the canvas adds stroke
//! rendering. `App::ui` closes it ([`SlowFrames::finish`]). A frame over [`SLOW_FRAME_MS`] goes
//! into [`SlowFrames`], the last [`SLOW_FRAMES`] of them, which the control channel's `perf`
//! output lists as `slowFrames`; one `info` log line per second names them. The stroke trace
//! (`stroke_trace`) reads the same record, so the app times a frame in one place.
//!
//! On the web `now_ms()` is 0, so no frame is ever slow and the ring stays empty.

use std::collections::VecDeque;

/// A frame slower than this (ms) is kept and logged.
pub const SLOW_FRAME_MS: f64 = 50.0;
/// How many slow frames are kept.
pub const SLOW_FRAMES: usize = 64;
/// At most one slow-frame log line per this many ms.
const LOG_EVERY_MS: f64 = 1000.0;

/// One frame's stages. Times in ms.
#[derive(Clone, Debug, Default, PartialEq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameStages {
    /// Frame start (the raw input hook), from [`crate::gpu_canvas::now_ms`].
    pub t: f64,
    /// The whole frame: from its start until its UI is built. Never less than the stages below
    /// (a command run between frames counts toward the next one).
    pub frame_ms: f64,
    /// From the raw input hook to the start of the UI build (input handling, minus commands).
    pub input_ms: f64,
    /// Commands run through the app since the previous frame ended.
    pub command_ms: f64,
    /// The last of those commands ("" for none).
    pub command: String,
    /// Damage tracking and GPU memory budgeting on the GPU canvas path.
    pub damage_ms: f64,
    /// Canvas composites, CPU side (on the GPU path: encoding and submitting them).
    pub composite_ms: f64,
    /// Canvas conversions and uploads.
    pub upload_ms: f64,
    /// The GPU's own time for a recent composite, when the canvas is on the GPU and it reports one.
    pub gpu_ms: Option<f64>,
    /// eframe's CPU time for the previous frame minus that frame's UI build: tessellation and
    /// GPU submission (the previous frame's, so it isn't part of `frame_ms`).
    pub present_ms: Option<f64>,
    /// The last canvas refresh's kind ("full", "rect", "gpu-full", …; "" for none).
    pub refresh: &'static str,
    /// Pixels the frame's canvas refreshes covered.
    pub refresh_px: u64,
    /// "gpu" or "cpu": who drew the canvas.
    pub path: &'static str,
    /// Building the UI (`App::ui`), the canvas refresh included.
    #[serde(skip)]
    pub ui_ms: f64,
    /// Rendering stroke points (the stroke trace's `stroke`).
    #[serde(skip)]
    pub stroke_ms: f64,
    /// Pointer moves the frame delivered.
    #[serde(skip)]
    pub moves: usize,
    /// eframe's CPU time for the previous frame.
    #[serde(skip)]
    pub cpu_prev_ms: Option<f64>,
    /// The raw input hook ran for this frame.
    #[serde(skip)]
    started: bool,
}

impl FrameStages {
    /// The raw input hook: the frame starts at `now` with `moves` pointer moves.
    pub fn start(&mut self, now: f64, moves: usize) {
        self.t = now;
        self.moves = moves;
        self.started = true;
    }

    /// The UI build starts at `now`; `cpu_prev` is eframe's CPU time for the previous frame, in s.
    pub fn ui_start(&mut self, now: f64, cpu_prev: Option<f32>) {
        if !self.started {
            self.start(now, 0);
        }
        self.input_ms = (now - self.t - self.command_ms).max(0.0);
        self.cpu_prev_ms = cpu_prev.map(|s| f64::from(s) * 1000.0);
    }

    /// A command `id` ran for `ms`.
    pub fn command(&mut self, id: &str, ms: f64) {
        self.command_ms += ms;
        id.clone_into(&mut self.command);
    }

    /// A canvas refresh of kind `kind` and `px` pixels.
    pub fn refresh(&mut self, kind: &'static str, px: u64, composite_ms: f64, upload_ms: f64) {
        self.composite_ms += composite_ms;
        self.upload_ms += upload_ms;
        self.refresh = kind;
        self.refresh_px += px;
    }

    /// The stages that make up `frame_ms` (`present_ms` belongs to the previous frame).
    pub fn stage_ms(&self) -> f64 {
        self.input_ms + self.command_ms + self.damage_ms + self.composite_ms + self.upload_ms
    }

    /// The frame ends at `now`.
    fn end(&mut self, now: f64) {
        if !self.started {
            self.t = now;
        }
        self.frame_ms = (now - self.t).max(self.stage_ms());
    }

    /// One line for the log.
    pub fn line(&self) -> String {
        let opt = |v: Option<f64>| v.map_or("-".to_string(), |v| format!("{v:.1}"));
        let command = if self.command.is_empty() { String::new() } else { format!(" ({})", self.command) };
        format!(
            "slow frame: {:.1} ms on the {} path: input {:.1}, command {:.1}{command}, damage {:.1}, composite {:.1}, upload {:.1}, gpu {}, present {}; refresh {} {} px",
            self.frame_ms,
            self.path,
            self.input_ms,
            self.command_ms,
            self.damage_ms,
            self.composite_ms,
            self.upload_ms,
            opt(self.gpu_ms),
            opt(self.present_ms),
            if self.refresh.is_empty() { "none" } else { self.refresh },
            self.refresh_px,
        )
    }
}

/// The last [`SLOW_FRAMES`] frames over the threshold.
#[derive(Debug)]
pub struct SlowFrames {
    frames: VecDeque<FrameStages>,
    threshold_ms: f64,
    /// The previous frame's UI build (for its `present_ms`).
    prev_ui_ms: Option<f64>,
    /// When the last log line went out, and the slow frames since that weren't logged.
    logged_at: Option<f64>,
    unlogged: usize,
}

impl Default for SlowFrames {
    fn default() -> Self {
        Self { frames: VecDeque::with_capacity(SLOW_FRAMES), threshold_ms: SLOW_FRAME_MS, prev_ui_ms: None, logged_at: None, unlogged: 0 }
    }
}

impl SlowFrames {
    /// The kept frames, oldest first.
    pub fn frames(&self) -> impl DoubleEndedIterator<Item = &FrameStages> {
        self.frames.iter()
    }

    /// Keep frames slower than `ms` instead of [`SLOW_FRAME_MS`] (tests).
    #[cfg(test)]
    pub(crate) fn set_threshold_ms(&mut self, ms: f64) {
        self.threshold_ms = ms;
    }

    /// Close frame `s` at `now`: `gpu` says the canvas is on the GPU, `gpu_ms` is the GPU's
    /// latest composite time. A slow frame is kept, and logged at most once a second.
    pub fn finish(&mut self, mut s: FrameStages, now: f64, gpu: bool, gpu_ms: Option<f64>) -> FrameStages {
        s.end(now);
        let on_gpu = if s.refresh.is_empty() { gpu } else { s.refresh.starts_with("gpu") };
        s.path = if on_gpu { "gpu" } else { "cpu" };
        s.gpu_ms = gpu_ms.filter(|_| on_gpu);
        s.present_ms = s.cpu_prev_ms.zip(self.prev_ui_ms).map(|(cpu, ui)| (cpu - ui).max(0.0));
        self.prev_ui_ms = Some(s.ui_ms);
        if let Some(line) = self.push(&s) {
            log::info!("{line}");
        }
        s
    }

    /// Keep `s` if it's slow; the log line to write, if one is due.
    fn push(&mut self, s: &FrameStages) -> Option<String> {
        if s.frame_ms.is_nan() || s.frame_ms <= self.threshold_ms {
            return None;
        }
        if self.frames.len() == SLOW_FRAMES {
            self.frames.pop_front();
        }
        self.frames.push_back(s.clone());
        if self.logged_at.is_some_and(|at| s.t - at < LOG_EVERY_MS) {
            self.unlogged += 1;
            return None;
        }
        let mut line = s.line();
        if self.unlogged > 0 {
            line.push_str(&format!(" ({} more slow frames)", self.unlogged));
        }
        self.logged_at = Some(s.t);
        self.unlogged = 0;
        Some(line)
    }
}

impl serde::Serialize for SlowFrames {
    fn serialize<S: serde::Serializer>(&self, ser: S) -> Result<S::Ok, S::Error> {
        ser.collect_seq(self.frames.iter())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(t: f64, ms: f64) -> FrameStages {
        let mut s = FrameStages::default();
        s.start(t, 0);
        s.ui_start(t + 1.0, None);
        s.refresh("full", 100, ms / 2.0, 1.0);
        s
    }

    #[test]
    fn slow_frames_keep_the_last_64_over_the_threshold() {
        let mut r = SlowFrames::default();
        for k in 0..100 {
            let t = f64::from(k) * 100.0;
            r.finish(frame(t, 0.0), t + 60.0 + f64::from(k), false, None);
        }
        assert_eq!(r.frames().count(), SLOW_FRAMES);
        let first = r.frames().next().unwrap();
        assert_eq!(first.t, 3600.0, "the oldest are dropped");
        assert_eq!(r.frames().last().unwrap().frame_ms, 159.0);
        assert!(r.frames().all(|f| f.path == "cpu" && f.refresh_px == 100));
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json.as_array().map(Vec::len), Some(SLOW_FRAMES));
        assert_eq!(json[0]["frameMs"], 96.0);
    }

    #[test]
    fn a_fast_frame_is_not_recorded() {
        let mut r = SlowFrames::default();
        let s = r.finish(frame(0.0, 0.0), 49.0, true, Some(3.0));
        assert_eq!(s.frame_ms, 49.0);
        assert_eq!(s.path, "cpu", "the refresh says who drew it");
        assert_eq!(s.gpu_ms, None, "no GPU time for a CPU refresh");
        r.finish(frame(100.0, 0.0), 150.0, false, None);
        assert_eq!(r.frames().count(), 0, "exactly 50 ms isn't over");
    }

    #[test]
    fn stages_never_exceed_the_frame() {
        let mut r = SlowFrames::default();
        // A command run between frames (before the hook) makes the stages longer than the clock.
        let mut s = FrameStages::default();
        s.command("layer.setProps", 80.0);
        s.start(1000.0, 2);
        s.ui_start(1005.0, Some(0.004));
        s.damage_ms = 2.0;
        s.refresh("gpu-full", 12, 30.0, 10.0);
        s.ui_ms = 40.0;
        let s = r.finish(s, 1050.0, true, Some(7.0));
        assert!(s.stage_ms() <= s.frame_ms, "{s:?}");
        assert_eq!((s.input_ms, s.frame_ms, s.path, s.gpu_ms), (0.0, 122.0, "gpu", Some(7.0)));
        assert_eq!(s.present_ms, None, "no previous frame");
        let mut next = frame(1100.0, 4.0);
        next.cpu_prev_ms = Some(45.0);
        let next = r.finish(next, 1110.0, true, None);
        assert_eq!(next.present_ms, Some(5.0), "the previous frame's CPU time minus its UI build");
        assert!(next.stage_ms() <= next.frame_ms);
    }

    #[test]
    fn slow_frame_log_is_rate_limited() {
        let mut r = SlowFrames::default();
        let lines: Vec<String> = (0..10)
            .filter_map(|k| {
                let mut s = frame(f64::from(k) * 100.0, 0.0);
                s.end(s.t + 60.0);
                r.push(&s)
            })
            .collect();
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert!(lines[0].starts_with("slow frame: 60.0 ms") && !lines[0].contains("more"), "{}", lines[0]);
        let mut s = frame(1000.0, 0.0);
        s.end(1070.0);
        let line = r.push(&s).expect("a second later, the next line");
        assert!(line.ends_with("(9 more slow frames)"), "{line}");
        assert_eq!(r.frames().count(), 11);
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod app_tests {
    use serde_json::json;

    use crate::PhotocraftApp;

    #[test]
    fn an_app_frame_with_a_full_refresh_records_its_stages() {
        let (w, h) = (4000u64, 3000u64);
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
        let layer = s.active().and_then(|d| d.active_layer).expect("the background").0;
        let mut harness = egui_kittest::Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(4).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            PhotocraftApp::new(s, crate::Services::default())
        });
        harness.run_steps(6);
        // Every frame counts as slow, so the test doesn't depend on the machine's speed.
        harness.state_mut().slow_frames.set_threshold_ms(0.0);
        harness.state_mut().run("layer.setProps", json!({"layer": layer, "visible": false})).unwrap();
        harness.step();
        let app = harness.state();
        let f = app.slow_frames.frames().rev().find(|f| f.command == "layer.setProps").expect("the toggle's frame is kept").clone();
        // The CPU canvas (kittest has no wgpu) redraws the whole document. A damage refresh counts
        // reduced texels times the factor squared, which can fall a few rows short (see P80).
        assert_eq!(f.path, "cpu", "{f:?}");
        assert!(matches!(f.refresh, "full" | "rect"), "{f:?}");
        assert!(f.refresh_px * 100 >= w * h * 95 && f.refresh_px <= w * h, "{f:?}");
        assert!(f.composite_ms > 0.0 && f.command_ms > 0.0, "{f:?}");
        assert!(f.stage_ms() <= f.frame_ms, "{f:?}");
    }
}
