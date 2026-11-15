//! Live strokes paint on a thread of their own, so a heavy brush never holds up the frame: the
//! canvas hands the new pen points over, waits a little for them (`LIVE_STROKE_WAIT`), and shows
//! the newest step the stroke has painted. While the thread is behind, points that arrive meanwhile
//! are painted together as one step, and the cursor, panels and pen input keep their frame rate.
//!
//! Each step is the stroke's own copy of the document, as when it painted on the UI thread, so the
//! canvas shows and refreshes it the same way (`LiveStroke::since`).

use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;
use web_time::Instant;

use photocraft_engine::paint::StrokePoint;
use photocraft_geom::Rect;

use super::strokes::EngineStroke;

/// What painting one batch of points gave: the rectangle it changed, or why it failed.
pub(super) type Step = photocraft_engine::Result<Rect>;

/// The stroke as last painted, for the canvas to show.
#[derive(Clone)]
pub(super) struct Shown {
    pub(super) doc: Arc<photocraft_doc::Document>,
    /// See `EngineStroke::wants_time`.
    pub(super) wants_time: bool,
    /// The Brush's smoothing string (`photocraft_engine::brush_cmds::LiveStroke::smoothing_string`).
    pub(super) smoothing_string: Option<([f64; 2], [f64; 2])>,
}

impl Shown {
    fn of(stroke: &EngineStroke) -> Self {
        let smoothing_string = match stroke {
            EngineStroke::Brush(l) => l.smoothing_string(),
            _ => None,
        };
        Self { doc: stroke.doc().clone(), wants_time: stroke.wants_time(), smoothing_string }
    }
}

struct State {
    /// Points handed over that the stroke hasn't painted yet.
    pending: Vec<StrokePoint>,
    /// The stroke, while nothing is painting with it.
    stroke: Option<EngineStroke>,
    /// Steps painted since the canvas last took them, and what the last one shows.
    steps: Vec<Step>,
    shown: Shown,
    /// A step failed: later points are dropped and `shown` stays the last good step, as when the
    /// canvas stopped feeding the stroke after its error.
    failed: bool,
    /// The canvas let go of the stroke: the thread ends.
    closed: bool,
    /// Tests: the thread holds a step it took until this is cleared (see `PaintThread::hold`).
    #[cfg(test)]
    held: bool,
    /// Tests: the next step panics (see `PaintThread::panic_next_step`).
    #[cfg(test)]
    panic_next: bool,
}

impl State {
    fn idle(&self) -> bool {
        self.stroke.is_some() && self.pending.is_empty()
    }
}

struct Shared {
    state: Mutex<State>,
    changed: Condvar,
}

impl Shared {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Paint the pending points, if the stroke is free.
    fn paint_pending(&self) {
        let (pts, stroke) = {
            let mut st = self.lock();
            if st.pending.is_empty() {
                return;
            }
            let Some(stroke) = st.stroke.take() else { return };
            (std::mem::take(&mut st.pending), stroke)
        };
        #[cfg(test)]
        self.test_hooks();
        let mut stroke = stroke;
        let step = stroke.push(&pts);
        let mut st = self.lock();
        if step.is_ok() {
            st.shown = Shown::of(&stroke);
        } else {
            st.failed = true;
            st.pending.clear();
        }
        st.steps.push(step);
        st.stroke = Some(stroke);
        drop(st);
        self.changed.notify_all();
    }

    /// Tests: wait while the step is held (the stroke is busy with it), then panic if asked to.
    #[cfg(test)]
    fn test_hooks(&self) {
        let mut st = self.lock();
        while st.held && !st.closed {
            st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
        }
        let panic = std::mem::take(&mut st.panic_next);
        drop(st);
        assert!(!panic, "a step panicked on the live stroke's thread (test)");
    }

    fn run(&self) {
        loop {
            {
                let mut st = self.lock();
                while st.pending.is_empty() && !st.closed {
                    st = self.changed.wait(st).unwrap_or_else(PoisonError::into_inner);
                }
                if st.closed {
                    return;
                }
            }
            self.paint_pending();
        }
    }
}

/// A live stroke painting on its own thread (or inline, where there are no threads: the web).
pub(super) struct PaintThread {
    shared: Arc<Shared>,
    /// `None`: painting inline, in `push`.
    thread: Option<JoinHandle<()>>,
    shown: Shown,
}

impl PaintThread {
    pub(super) fn start(stroke: EngineStroke) -> Self {
        let shown = Shown::of(&stroke);
        let state = State {
            pending: Vec::new(),
            stroke: Some(stroke),
            steps: Vec::new(),
            shown: shown.clone(),
            failed: false,
            closed: false,
            #[cfg(test)]
            held: false,
            #[cfg(test)]
            panic_next: false,
        };
        let shared = Arc::new(Shared { state: Mutex::new(state), changed: Condvar::new() });
        let thread = if cfg!(target_arch = "wasm32") {
            None
        } else {
            let worker = shared.clone();
            // Without a thread (the system refused one) the stroke paints inline, as it used to.
            std::thread::Builder::new().name("live stroke".into()).spawn(move || worker.run()).ok()
        };
        Self { shared, thread, shown }
    }

    /// Hand points over to be painted (none after a failed step).
    pub(super) fn push(&mut self, pts: Vec<StrokePoint>) {
        if pts.is_empty() {
            return;
        }
        {
            let mut st = self.shared.lock();
            if st.failed {
                return;
            }
            st.pending.extend(pts);
        }
        if self.thread.is_some() {
            self.shared.changed.notify_all();
        } else {
            self.shared.paint_pending();
        }
    }

    /// Wait up to `wait` for everything handed over to be painted; returns the steps painted since
    /// the last call, oldest first, and updates what `shown` shows.
    pub(super) fn catch_up(&mut self, wait: Duration) -> Vec<Step> {
        let (shown, steps) = {
            let mut st = self.wait_idle(wait);
            (st.shown.clone(), std::mem::take(&mut st.steps))
        };
        self.shown = shown;
        steps
    }

    /// Is the stroke still painting points it was given, or holding steps `catch_up` hasn't taken?
    /// (A step can finish just after a frame's wait ran out; the next frame must collect it.)
    pub(super) fn busy(&self) -> bool {
        let st = self.shared.lock();
        !st.steps.is_empty() || (!st.idle() && self.thread.as_ref().is_some_and(|t| !t.is_finished()))
    }

    pub(super) fn shown(&self) -> &Shown {
        &self.shown
    }

    /// Wait for everything handed over to be painted; the steps since the last `catch_up` and the
    /// stroke itself (`None` if its thread stopped mid-step).
    pub(super) fn finish(mut self) -> (Vec<Step>, Option<EngineStroke>) {
        let steps = self.catch_up(Duration::MAX);
        let stroke = self.shared.lock().stroke.take();
        (steps, stroke)
    }

    /// Tests: from now on the thread holds the next step it takes, as a slow brush would, until
    /// the returned function lets it go (from any thread).
    #[cfg(test)]
    pub(super) fn hold(&self) -> impl FnOnce() + Send + 'static {
        self.shared.lock().held = true;
        let shared = self.shared.clone();
        move || {
            shared.lock().held = false;
            shared.changed.notify_all();
        }
    }

    /// Tests: the next step panics on the thread, which then stops with the stroke.
    #[cfg(test)]
    pub(super) fn panic_next_step(&self) {
        self.shared.lock().panic_next = true;
    }

    /// Tests: is the thread still running? (It keeps the shared state alive until it ends.)
    #[cfg(test)]
    pub(super) fn thread_alive(&self) -> impl Fn() -> bool + use<> {
        let weak = Arc::downgrade(&self.shared);
        move || weak.strong_count() > 0
    }

    /// Look at the stroke once it has painted everything handed over.
    #[cfg(test)]
    pub(super) fn peek<R>(&self, f: impl FnOnce(&EngineStroke) -> R) -> Option<R> {
        self.wait_idle(Duration::MAX).stroke.as_ref().map(f)
    }

    fn wait_idle(&self, wait: Duration) -> MutexGuard<'_, State> {
        let deadline = Instant::now().checked_add(wait);
        let mut st = self.shared.lock();
        // A stopped thread (inline painting never waits) has nothing more to give.
        while !st.idle() && self.thread.as_ref().is_some_and(|t| !t.is_finished()) {
            let left = deadline.map_or(Duration::MAX, |d| d.saturating_duration_since(Instant::now()));
            if left.is_zero() {
                break;
            }
            // Wake now and then to notice a thread that stopped without a word.
            let (guard, _) = self.shared.changed.wait_timeout(st, left.min(Duration::from_millis(50))).unwrap_or_else(PoisonError::into_inner);
            st = guard;
        }
        st
    }
}

impl Drop for PaintThread {
    fn drop(&mut self) {
        // The thread finishes the step it is on, if any, and ends; nothing waits for it.
        self.shared.lock().closed = true;
        self.shared.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_engine::Session;
    use photocraft_engine::brush_cmds::LiveStroke;
    use serde_json::json;

    #[test]
    fn a_stroke_painted_on_the_thread_in_any_batches_equals_one_painted_inline() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 120, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "paint"})).unwrap();
        let begin = || {
            let p = json!({"points": [[20, 60, 1.0]], "brush": {"size": 40, "hardness": 0.3, "smoothing": {"amount": 0.4}}, "seed": 3});
            EngineStroke::Brush(Box::new(LiveStroke::begin(&s, &p).unwrap()))
        };
        let pts: Vec<StrokePoint> = (1..=60).map(|i| StrokePoint::new(20.0 + 6.0 * f64::from(i), 60.0 + 20.0 * (f64::from(i) / 9.0).sin(), 1.0)).collect();

        // Never waiting, so the thread merges points that arrive while it paints into one step.
        let mut thread = PaintThread::start(begin());
        assert!(thread.thread.is_some(), "a thread of its own");
        let mut steps = Vec::new();
        for chunk in pts.chunks(3) {
            thread.push(chunk.to_vec());
            steps.extend(thread.catch_up(Duration::ZERO));
        }
        let (rest, painted) = thread.finish();
        steps.extend(rest);
        assert!(!steps.is_empty() && steps.iter().all(Result::is_ok), "every step painted");
        let painted = painted.expect("the stroke comes back");

        let mut inline = begin();
        inline.push(&pts).unwrap();
        let layer = s.active().unwrap().active_layer.unwrap();
        let surface = |e: &EngineStroke| e.doc().layer(layer).unwrap().surface().unwrap().clone();
        let (a, b) = (surface(&painted), surface(&inline));
        let differ = (0..120).flat_map(|y| (0..400).map(move |x| (x, y))).filter(|&(x, y)| a.rgba(x, y) != b.rgba(x, y)).count();
        assert_eq!(differ, 0, "pixels differ from the inline stroke");
        assert!(b.rgba(200, 60 + (20.0 * (30.0f64 / 9.0).sin()) as i32)[3] > 0.5, "the stroke is there");
    }

    #[test]
    fn a_painted_step_keeps_the_stroke_busy_until_the_canvas_takes_it() {
        // The step finishes after the frame stopped waiting: the canvas must ask for another frame.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
        let p = json!({"points": [[20, 50, 1.0]], "brush": {"size": 20}});
        let mut thread = PaintThread::start(EngineStroke::Brush(Box::new(LiveStroke::begin(&s, &p).unwrap())));
        thread.push(vec![StrokePoint::new(120.0, 50.0, 1.0)]);
        thread.peek(|_| ());
        assert!(thread.busy(), "a painted step nobody took yet");
        assert_eq!(thread.catch_up(Duration::ZERO).len(), 1);
        assert!(!thread.busy());
    }

    #[test]
    fn after_a_failed_step_the_stroke_paints_nothing_more_and_shows_its_last_good_step() {
        // The Mixer Brush keeps its last good preview after an error (`take_steps`); points queued
        // behind the failed batch must not paint over it.
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
        s.execute("layer.new.layer", json!({"name": "paint"})).unwrap();
        let p = json!({"points": [[30, 50]], "size": 16});
        let mixer = photocraft_engine::brush_cmds::mixer_live::LiveMixer::begin(&s, &p).unwrap();
        let mut thread = PaintThread::start(EngineStroke::Mixer(Box::new(mixer)));
        let good = thread.shown().doc.clone();
        // More samples than a live Mixer stroke takes, then one it would take.
        thread.push(vec![StrokePoint::new(40.0, 50.0, 1.0); 70_000]);
        thread.peek(|_| ());
        thread.push(vec![StrokePoint::new(150.0, 50.0, 1.0)]);
        let steps = thread.catch_up(Duration::MAX);
        assert!(matches!(steps.as_slice(), [Err(_)]), "only the failed step");
        assert!(Arc::ptr_eq(&thread.shown().doc, &good), "the last good step stays shown");
        assert!(!thread.busy());
    }
}
