//! Stylus input: real pen pressure (and tilt / barrel rotation where the platform reports them)
//! for the painting tools. A mouse always paints at pressure 1 with no tilt.
//!
//! Sources, by platform (eframe 0.36.2, egui-winit 0.36.2, winit 0.30.13):
//!
//! - **Windows**: winit turns `WM_POINTER` pen and touch input into `WindowEvent::Touch` with a
//!   normalised force (`POINTER_PEN_INFO::pressure / 1024`); egui-winit forwards it as
//!   `egui::Event::Touch { force }` alongside the emulated pointer. [`Stylus::update`] reads it.
//!   winit drops the pen's tilt and rotation, so those stay 0, and reports pressure 0 (a tap
//!   lighter than the pen's first pressure level) as no force, which [`Stylus::update`] reads as
//!   pressure 0 once it knows a pen is in use.
//! - **Web**: eframe forwards touch force but not pen pointer events, so the web runner listens
//!   to `pointerdown`/`pointermove` itself and writes `pressure`, `tiltX`, `tiltY`, `twist` and
//!   the eraser button of `pointerType == "pen"` events into the [`StylusFeed`].
//! - **macOS**: winit 0.30 drops `NSEvent` tablet data, so the desktop app installs an AppKit
//!   local event monitor (the `photocraft-tablet` crate) that writes pressure, tilt, rotation and
//!   the eraser end into the [`StylusFeed`] before winit handles each event.
//! - **Linux X11**: the desktop app reads XInput2 raw valuator events on its own X connection
//!   (`photocraft-tablet`, x11rb) and writes them into the [`StylusFeed`].
//! - **Linux Wayland**: compositors give a pen only to clients that bind `zwp_tablet_v2`, which
//!   winit doesn't (it would have to share winit's connection), so with a pen attached the
//!   desktop app opens its window through Xwayland, which reports tablet valuators to the X11
//!   reader. `PHOTOCRAFT_NATIVE_WAYLAND=1` keeps native Wayland, where the pen has no input.
//!
//! Preferences › Tools › Use Tablet Pressure off makes a pen paint like a mouse. Flipping the pen
//! to its eraser end selects the Eraser tool and flipping back restores the previous tool, as in
//! Photoshop.
//!
//! Automation simulates a pen with `ui.pointer` events carrying `pressure`, `tiltX`, `tiltY`
//! and `rotation`.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, PoisonError};

/// One stylus reading. Tilt is in degrees (-90..90, W3C Pointer Events convention), rotation in
/// degrees 0..360.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PenSample {
    pub pressure: f32,
    pub tilt_x: f32,
    pub tilt_y: f32,
    pub rotation: f32,
    /// The pen's eraser end is in use.
    pub eraser: bool,
}

impl Default for PenSample {
    fn default() -> Self {
        Self { pressure: 1.0, tilt_x: 0.0, tilt_y: 0.0, rotation: 0.0, eraser: false }
    }
}

impl PenSample {
    /// Clamp to the documented ranges (and replace non-finite values).
    pub fn sanitized(self) -> Self {
        let f = |v: f32, lo: f32, hi: f32, d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        Self {
            pressure: f(self.pressure, 0.0, 1.0, 1.0),
            tilt_x: f(self.tilt_x, -90.0, 90.0, 0.0),
            tilt_y: f(self.tilt_y, -90.0, 90.0, 0.0),
            // `rem_euclid` rounds tiny negative angles up to exactly 360.
            rotation: if self.rotation.is_finite() { self.rotation.rem_euclid(360.0) % 360.0 } else { 0.0 },
            eraser: self.eraser,
        }
    }
}

/// Most samples a [`StylusFeed`] keeps between two frames (a stalled UI drops the oldest).
const FEED_QUEUE: usize = 1024;

#[derive(Debug, Default)]
struct FeedState {
    /// The current sample (`None` = no pen down).
    latest: Option<PenSample>,
    /// Every sample set since the canvas last drained them, oldest first.
    queue: VecDeque<PenSample>,
}

/// Shared state a platform backend writes pen samples into. Cloning shares it. Each sample both
/// becomes the current one and is queued, so a frame sees every sample the pen reported, in
/// order, not just the last one (the pressure of each pointer move is interpolated from them).
#[derive(Clone, Debug, Default)]
pub struct StylusFeed(Arc<Mutex<FeedState>>);

impl StylusFeed {
    /// The current sample (`None` = no pen down); a sample is also queued for [`Self::drain`].
    pub fn set(&self, s: Option<PenSample>) {
        let mut g = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        let s = s.map(PenSample::sanitized);
        g.latest = s;
        if let Some(s) = s {
            if g.queue.len() >= FEED_QUEUE {
                g.queue.pop_front();
            }
            g.queue.push_back(s);
        }
    }

    pub fn get(&self) -> Option<PenSample> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).latest
    }

    /// The samples set since the last drain, oldest first.
    pub fn drain(&self) -> Vec<PenSample> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner).queue.drain(..).collect()
    }
}

/// Per-app stylus state.
#[derive(Clone, Debug)]
pub struct Stylus {
    /// Pen samples pushed by the platform (desktop tablet monitor, web runner) or by automation.
    pub feed: StylusFeed,
    /// Preferences › Tools › Use Tablet Pressure: off, a pen paints like a mouse.
    pub use_pressure: bool,
    /// Preferences › Tools › Pressure Curve (`None`: linear), with the points it was built from.
    pressure_curve: Option<photocraft_engine::prefs::PressureCurve>,
    curve_points: Vec<[f32; 2]>,
    /// The pen end last seen (`Some(true)` = eraser), for the eraser tool switch.
    end: Option<bool>,
    /// The tool to restore when the pen tip comes back after the eraser end switched tools.
    pub(crate) tool_before_eraser: Option<crate::state::Tool>,
    /// Force of the touch/pen contact currently down (egui `Event::Touch`).
    touch: Option<f32>,
    /// The touch contacts currently down, so a touch move outside them is a hovering pen.
    contacts: Vec<egui::TouchId>,
    /// A pen reported through touch events (it hovered, or a contact carried a force): its
    /// contacts without a force are at pressure 0, not a finger's full pressure.
    touch_pen: bool,
    /// The contact ended this frame: keep its force for this frame's last tool events, clear next frame.
    lifted: bool,
    /// This frame's pen samples, oldest first (the feed's queue and the touch forces).
    frame: Vec<PenSample>,
    /// The last pen sample of an earlier frame: where this frame's interpolation starts.
    prev: Option<PenSample>,
    /// The sample [`Self::select`] interpolated for the pointer move being processed.
    current: Option<PenSample>,
    /// The highest pen pressure (before the curve) since the drag began, for a tap's one dab.
    peak: f32,
    /// The pen's touch contacts in the raw input, so a held pen is no press-and-hold right click.
    pub(crate) pen_contacts: PenContacts,
    /// The egui frame [`Self::update_for_frame`] last ran in.
    updated_frame: Option<u64>,
    /// Tilt X, tilt Y, rotation of each point of the current drag (parallel to its points).
    pub(crate) stroke: Vec<[f32; 3]>,
    /// Time in milliseconds of each point of the current drag (parallel to its points), from
    /// [`Stylus::clock_ms`]. Speed spacing, airbrush build-up and smoothing catch-up use it.
    pub(crate) times: Vec<f64>,
    /// Time of the tool event being processed, in milliseconds. The canvas spreads a frame's
    /// samples between the previous and the current frame time (egui's events carry none).
    pub(crate) clock_ms: f64,
}

impl Default for Stylus {
    fn default() -> Self {
        Self {
            feed: StylusFeed::default(),
            use_pressure: true,
            pressure_curve: None,
            curve_points: Vec::new(),
            end: None,
            tool_before_eraser: None,
            touch: None,
            contacts: Vec::new(),
            touch_pen: false,
            lifted: false,
            frame: Vec::new(),
            prev: None,
            current: None,
            peak: 0.0,
            pen_contacts: PenContacts::default(),
            updated_frame: None,
            stroke: Vec::new(),
            times: Vec::new(),
            clock_ms: 0.0,
        }
    }
}

impl Stylus {
    /// [`Self::update`] once per egui frame: the canvas runs once per view, and egui can run a
    /// frame in several passes; a second update would drain an empty queue and lose the frame's
    /// pen samples.
    pub fn update_for_frame(&mut self, frame: u64, events: &[egui::Event]) {
        if self.updated_frame.replace(frame) != Some(frame) {
            self.update(events);
        }
    }

    /// Track touch/pen contacts from this frame's input events.
    pub fn update(&mut self, events: &[egui::Event]) {
        if std::mem::take(&mut self.lifted) {
            self.touch = None;
        }
        if let Some(last) = self.frame.last() {
            self.prev = Some(*last);
        }
        self.frame = self.feed.drain();
        self.current = None;
        for e in events {
            if let egui::Event::Touch { id, phase, force, .. } = e {
                let force = force.filter(|f| f.is_finite()).map(|f| f.clamp(0.0, 1.0));
                match phase {
                    egui::TouchPhase::Start | egui::TouchPhase::Move => {
                        let start = *phase == egui::TouchPhase::Start;
                        if start {
                            // A new contact keeps its force into the next frame, even when an
                            // earlier contact lifted in this one.
                            self.lifted = false;
                            if !self.contacts.contains(id) {
                                if self.contacts.len() >= 16 {
                                    self.contacts.remove(0);
                                }
                                self.contacts.push(*id);
                            }
                        }
                        let down = start || self.contacts.contains(id);
                        // Only a pen moves without touching, and only a pen (or a force-sensing
                        // screen) reports a force.
                        if !down || force.is_some() {
                            self.touch_pen = true;
                        }
                        // Windows reports a pen pressed lighter than its first pressure level with
                        // no force at all (winit maps pressure 0 to `None`): a light tap is
                        // pressure 0, never a mouse click's full pressure. A finger on a screen
                        // without force sensing stays at full pressure.
                        let force = force.or((down && self.touch_pen && (start || self.touch.is_none())).then_some(0.0));
                        if let Some(f) = force {
                            self.touch = Some(f);
                            self.frame.push(PenSample { pressure: f, ..Default::default() });
                        }
                    }
                    egui::TouchPhase::End | egui::TouchPhase::Cancel => {
                        self.lifted = true;
                        self.contacts.retain(|c| c != id);
                    }
                }
            }
        }
        // A pen reports pressure 0 while it hovers and as it lifts (X11, macOS): that is no
        // contact, so a frame that also touched keeps only its contact samples. A stroke's last
        // moves then keep the pressure the pen left with, and a tap its own (#1798).
        if self.frame.iter().any(|s| s.pressure > 0.0) {
            self.frame.retain(|s| s.pressure > 0.0);
        }
        self.peak = self.frame.iter().fold(self.peak, |m, s| m.max(s.pressure));
    }

    /// The pen's current sample. When the pen touched during this frame but has lifted since
    /// (the feed is cleared or back at pressure 0 by the time the UI runs, as for a tap shorter
    /// than a frame), the frame's last contact sample: never full pressure as for a mouse, and
    /// never the lift's 0.
    fn latest(&self) -> Option<PenSample> {
        let feed = self.feed.get();
        match self.frame.last() {
            Some(s) if s.pressure > 0.0 && feed.is_none_or(|f| f.pressure <= 0.0) => Some(*s),
            _ => feed,
        }
    }

    /// The current pen sample, `None` for a mouse (and for any pen while Use Tablet Pressure is
    /// off).
    pub fn sample(&self) -> Option<PenSample> {
        if !self.use_pressure {
            return None;
        }
        let s = self.current.or_else(|| self.latest()).or(self.touch.map(|pressure| PenSample { pressure, ..Default::default() }))?;
        // The pen's pressure through Preferences › Tools › Pressure Curve, before any brush sees it.
        Some(match &self.pressure_curve {
            Some(c) => PenSample { pressure: c.eval(s.pressure), ..s },
            None => s,
        })
    }

    /// Preferences › Tools › Pressure Curve (rebuilt only when the points change).
    pub fn set_pressure_curve(&mut self, points: &[[f32; 2]]) {
        if self.curve_points != points {
            self.curve_points = points.to_vec();
            let c = photocraft_engine::prefs::PressureCurve::new(points);
            self.pressure_curve = (!c.is_linear()).then_some(c);
        }
    }

    /// Choose the pen sample for move `k` of the `n` pointer moves this frame delivered: the pen
    /// reports at its own rate, so each move takes the samples interpolated at its share of the
    /// frame (from the previous frame's last sample), instead of all sharing the latest one. A
    /// mouse, or a frame without pen samples, keeps the current sample.
    pub(crate) fn select(&mut self, k: usize, n: usize) {
        self.current = None;
        if !self.use_pressure || n == 0 || self.frame.is_empty() || self.sample().is_none() {
            return;
        }
        let m = self.frame.len();
        let start = self.prev.or(self.frame.first().copied()).unwrap_or_default();
        let at = |i: usize| if i == 0 { start } else { self.frame.get(i - 1).copied().unwrap_or(start) };
        let pos = (k + 1).min(n) as f32 / n as f32 * m as f32;
        let i = pos.floor() as usize;
        let (a, b, t) = (at(i), at((i + 1).min(m)), pos - pos.floor());
        let lerp = |x: f32, y: f32| x + (y - x) * t;
        // Rotation takes the shorter way round.
        let dr = (b.rotation - a.rotation + 540.0).rem_euclid(360.0) - 180.0;
        self.current = Some(
            PenSample {
                pressure: lerp(a.pressure, b.pressure),
                tilt_x: lerp(a.tilt_x, b.tilt_x),
                tilt_y: lerp(a.tilt_y, b.tilt_y),
                rotation: a.rotation + dr * t,
                eraser: b.eraser,
            }
            .sanitized(),
        );
    }

    /// Back to the current sample after a frame's moves.
    pub(crate) fn clear_selection(&mut self) {
        self.current = None;
    }

    /// Did the pen just flip to its eraser end (`Some(true)`) or back to its tip (`Some(false)`)?
    /// Each flip is reported once; a mouse in between changes nothing.
    pub(crate) fn take_end_flip(&mut self) -> Option<bool> {
        let eraser = self.feed.get()?.eraser;
        let flipped = self.end.map_or(eraser, |e| e != eraser);
        self.end = Some(eraser);
        flipped.then_some(eraser)
    }

    /// Switch to the Eraser when the pen's eraser end comes in, and back to the previous tool when
    /// the tip does (Photoshop). Not during a drag. Returns whether the tool changed.
    pub fn sync_eraser_tool(app: &mut crate::PhotocraftApp) -> bool {
        use crate::state::Tool;
        if app.drag.is_some() {
            return false;
        }
        match app.stylus.take_end_flip() {
            Some(true) if app.ui.tool != Tool::Eraser => {
                app.stylus.tool_before_eraser = Some(app.ui.tool);
                app.ui.tool = Tool::Eraser;
                true
            }
            Some(false) => match app.stylus.tool_before_eraser.take() {
                Some(t) if app.ui.tool == Tool::Eraser => {
                    app.ui.tool = t;
                    true
                }
                _ => false,
            },
            _ => false,
        }
    }

    /// Pressure for the next tool event (1 for a mouse).
    pub fn pressure(&self) -> f32 {
        self.sample().map_or(1.0, |s| s.pressure)
    }

    /// Pressure for a tap, a drag released where it was pressed: the highest the pen pressed
    /// since the drag began (through the pressure curve), as the first sample of a contact is
    /// often far lighter than the tap. 1 for a mouse.
    pub(crate) fn tap_pressure(&self) -> f32 {
        let Some(s) = self.sample() else { return 1.0 };
        let peak = match &self.pressure_curve {
            Some(c) => c.eval(self.peak),
            None => self.peak,
        };
        if self.peak > 0.0 { peak.max(s.pressure) } else { s.pressure }
    }

    /// Start recording a drag's tilt/rotation.
    pub(crate) fn begin_stroke(&mut self) {
        // A new stroke's first moves interpolate from this frame's samples, never from a sample
        // left over from an earlier stroke.
        self.prev = None;
        self.peak = self.frame.iter().fold(0.0, |m, s| m.max(s.pressure));
        self.stroke.clear();
        self.times.clear();
        self.record_point();
    }

    /// Record the tilt/rotation and time of a point just added to the drag.
    pub(crate) fn record_point(&mut self) {
        let s = self.sample().unwrap_or_default();
        self.stroke.push([s.tilt_x, s.tilt_y, s.rotation]);
        self.times.push(self.clock_ms);
    }

    /// Time of the drag's last recorded point.
    pub(crate) fn last_point_ms(&self) -> Option<f64> {
        self.times.last().copied()
    }

    /// Time of drag point `i` (the last known time past the end).
    pub(crate) fn point_ms(&self, i: usize) -> f64 {
        self.times.get(i).or(self.times.last()).copied().unwrap_or(0.0)
    }

    /// Stroke points for `paint.stroke`: `[x, y, pressure]`, extended with
    /// `tiltX, tiltY, rotation` when the drag carried any tilt or rotation, and with `timeMs`
    /// (from the drag's first point) when its points carry times.
    pub fn stroke_points(&self, points: &[[f64; 3]]) -> Vec<Vec<f64>> {
        let has_pose = self.stroke.iter().any(|t| t.iter().any(|v| *v != 0.0));
        let t0 = self.point_ms(0);
        let has_time = self.times.iter().any(|t| *t != t0);
        points
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let mut v = vec![p[0], p[1], p[2]];
                if has_pose || has_time {
                    let t = self.stroke.get(i).or(self.stroke.last()).copied().unwrap_or_default();
                    v.extend(t.map(f64::from));
                }
                if has_time {
                    v.push(self.point_ms(i) - t0);
                }
                v
            })
            .collect()
    }
}

/// The touch contacts a pen makes, read from the raw input. egui takes every touch contact held
/// still past `max_click_duration` (0.8 s) for a touch screen's press-and-hold: a secondary click
/// (a context menu) that also drops the drag. On Windows a pen arrives as touch events, so a pen
/// held still opened a context menu. A pen held still is just a pen, as in Photoshop.
#[derive(Clone, Debug, Default)]
pub(crate) struct PenContacts {
    /// Touch ids that moved without touching: only a pen hovers.
    hovering: Vec<egui::TouchId>,
    /// The contacts down, and whether each is a pen.
    down: Vec<(egui::TouchId, bool)>,
}

/// Most touch ids [`PenContacts`] remembers (the oldest go first).
const CONTACTS: usize = 16;

impl PenContacts {
    /// Follow a frame's touch events. True when a pen contact was down at any point of the frame.
    pub(crate) fn observe(&mut self, events: &[egui::Event]) -> bool {
        let pressed = |f: &Option<f32>| f.is_some_and(|f| f > 0.0);
        let mut pen = self.down.iter().any(|(_, p)| *p);
        for e in events {
            let egui::Event::Touch { id, phase, force, .. } = e else { continue };
            match phase {
                egui::TouchPhase::Start => {
                    let is_pen = self.hovering.contains(id) || pressed(force);
                    self.down.retain(|(c, _)| c != id);
                    if self.down.len() >= CONTACTS {
                        self.down.remove(0);
                    }
                    self.down.push((*id, is_pen));
                    pen |= is_pen;
                }
                egui::TouchPhase::Move => match self.down.iter_mut().find(|(c, _)| c == id) {
                    Some(c) if pressed(force) => {
                        c.1 = true;
                        pen = true;
                    }
                    Some(_) => {}
                    None if !self.hovering.contains(id) => {
                        if self.hovering.len() >= CONTACTS {
                            self.hovering.remove(0);
                        }
                        self.hovering.push(*id);
                    }
                    None => {}
                },
                egui::TouchPhase::End | egui::TouchPhase::Cancel => self.down.retain(|(c, _)| c != id),
            }
        }
        pen
    }
}

/// Turn egui's touch press-and-hold off while a pen contact is down (and through the frame it
/// lifts in, so the release is the click a quick tap would be), back on otherwise for fingers.
pub(crate) fn pen_hold_is_no_right_click(ctx: &egui::Context, pen_down: bool) {
    let want = if pen_down { f64::INFINITY } else { egui::InputOptions::default().max_click_duration };
    if ctx.options(|o| o.input_options.max_click_duration) != want {
        ctx.options_mut(|o| o.input_options.max_click_duration = want);
    }
}

#[cfg(test)]
mod tests;
