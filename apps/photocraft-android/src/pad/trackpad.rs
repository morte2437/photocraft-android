//! Trackpad cursor (Touch Input): fingers move a cursor instead of touching things directly,
//! like a laptop trackpad, so the fingertip never covers what it points at (#4).
//!
//! - One finger sliding moves the cursor from wherever it was (times the speed setting, and
//!   further for quick swipes with acceleration on): it stays put between touches, like a mouse
//!   pointer.
//! - A tap clicks at the cursor, a double tap double-clicks, and a finger held still
//!   right-clicks. Over the interface the click is immediate; over the canvas it waits a moment
//!   for a second touch that would drag instead, so a stroke doesn't start with a dot.
//! - Holding the button: a tap and then a touch that drags (held still instead, it holds the
//!   button where the cursor is), or one finger resting still while another moves the cursor.
//! - Two fingers moving together: over the canvas they pan, zoom and rotate it; over the
//!   interface they scroll what's under the cursor. Quick two- and three-finger taps run their
//!   shortcuts on the canvas, or a two-finger tap right-clicks (a setting).
//!
//! The pen draws where it touches, as always (the cursor jumps there); the Shortcut Pad's touches
//! never get here.

use super::router::{Env, Routed, ViewMove};
use egui::{Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, TouchId, TouchPhase, Vec2};

/// A tap is quicker than this (seconds)...
const TAP_TIME: f64 = 0.3;
/// ...and moves less than this (points).
const TAP_SLOP: f32 = 12.0;
/// How long after a tap a second touch drags or double-clicks (seconds); over the canvas, the
/// tap's click waits this long for it.
const CLICK_WAIT: f64 = 0.25;
/// A finger held still this long right-clicks (or, after a tap, holds the button).
const HOLD_TIME: f64 = 0.5;
/// Two fingers decide what they do once one has moved this far (points)...
const TWO_LOCK: f32 = 12.0;
/// ...and a finger that has moved less than this meanwhile is resting (it holds the button).
const RESTING: f32 = 6.0;
/// Acceleration: the finger speeds (points per second) where the cursor starts moving further,
/// and where it reaches [`ACCEL_MAX`] times as far.
const ACCEL_FROM: f32 = 250.0;
const ACCEL_FULL: f32 = 1500.0;
const ACCEL_MAX: f32 = 2.5;

/// What two fingers do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Two {
    /// Not moved far enough to tell yet.
    Undecided,
    /// The cursor is over the canvas: pan, zoom and rotate it.
    Canvas,
    /// The cursor is over the interface: scroll what's under it.
    Scroll,
    /// One finger rests and holds the button; this one moves the cursor.
    Hold(TouchId),
}

#[derive(Clone, Copy, Debug)]
struct Finger {
    id: TouchId,
    start: Pos2,
    pos: Pos2,
}

#[derive(Debug, Default)]
pub struct Trackpad {
    /// Where the cursor is (none until the first touch).
    cursor: Option<Pos2>,
    fingers: Vec<Finger>,
    /// When the first finger of this touch went down.
    since: f64,
    /// The fingers moved past a tap's slop.
    moved: bool,
    /// The most fingers down at once.
    most: usize,
    /// The button is held (a tap-and-drag, or a resting finger).
    pressed: bool,
    /// This touch came right after a tap: dragging holds the button, a lift double-clicks...
    armed: bool,
    /// ...and that tap's click was already sent (over the interface).
    armed_clicked: bool,
    /// A tap's click waiting for [`CLICK_WAIT`] (when the tap ended), over the canvas.
    pending_click: Option<f64>,
    /// When the last tap ended, after its click was sent at once (over the interface).
    last_tap: Option<f64>,
    /// The hold already right-clicked.
    held: bool,
    two: Option<Two>,
    /// What two fingers moving together do here (canvas or scroll), decided when the second lands.
    two_target: Option<Two>,
    /// Two fingers' centre, spread and angle as last applied.
    pose: Option<(Pos2, f32, f32)>,
    /// When the cursor last moved (for acceleration).
    last_move: f64,
}

fn press(out: &mut Vec<Routed>, at: Pos2, button: PointerButton, pressed: bool) {
    out.push(Routed::App(Event::PointerButton { pos: at, button, pressed, modifiers: Modifiers::NONE }));
}

fn click(out: &mut Vec<Routed>, at: Pos2, button: PointerButton) {
    out.push(Routed::App(Event::PointerMoved(at)));
    press(out, at, button, true);
    press(out, at, button, false);
}

fn wrap_degrees(d: f32) -> f32 {
    (d + 540.0).rem_euclid(360.0) - 180.0
}

/// How far the cursor moves for a finger moving `d` in `dt` seconds: further when the finger is
/// quick, so a swipe crosses the screen while slow moves stay precise.
pub fn accelerated(d: Vec2, dt: f64) -> Vec2 {
    if !(dt.is_finite() && dt > 0.001) {
        return d;
    }
    let speed = d.length() / dt as f32;
    let t = ((speed - ACCEL_FROM) / (ACCEL_FULL - ACCEL_FROM)).clamp(0.0, 1.0);
    d * (1.0 + t * (ACCEL_MAX - 1.0))
}

impl Trackpad {
    /// Where the cursor is, to draw it.
    pub fn cursor(&self) -> Option<Pos2> {
        self.cursor
    }

    /// The pen touched or moved here: the cursor follows it.
    pub fn pen_at(&mut self, p: Pos2) {
        self.cursor = Some(p);
    }

    /// Frames are needed while a finger is down or a click waits.
    pub fn wants_frames(&self) -> bool {
        !self.fingers.is_empty() || self.pending_click.is_some()
    }

    fn cursor_or_centre(&mut self, env: &Env) -> Pos2 {
        *self.cursor.get_or_insert(env.screen.center())
    }

    fn pose(&self) -> Option<(Pos2, f32, f32)> {
        let [a, b, ..] = self.fingers.as_slice() else { return None };
        let n = self.fingers.len() as f32;
        let c = self.fingers.iter().fold(Vec2::ZERO, |s, f| s + f.pos.to_vec2()) / n;
        Some((c.to_pos2(), a.pos.distance(b.pos), (b.pos.y - a.pos.y).atan2(b.pos.x - a.pos.x).to_degrees()))
    }

    /// Move the cursor by a finger's move `d` (speed and acceleration applied).
    fn move_cursor(&mut self, d: Vec2, env: &Env, out: &mut Vec<Routed>) {
        let cursor = self.cursor_or_centre(env);
        let d = d * env.config.trackpad_speed;
        let d = if env.config.trackpad_acceleration { accelerated(d, env.now - self.last_move) } else { d };
        self.last_move = env.now;
        let to = (cursor + d).clamp(env.screen.min, env.screen.max);
        self.cursor = Some(to);
        out.push(Routed::App(Event::PointerMoved(to)));
    }

    /// One finger touch: its id, phase and position. `ignore`: a palm (the pen is near).
    pub fn touch(&mut self, (id, phase, pos): (TouchId, TouchPhase, Pos2), ignore: bool, covered: &dyn Fn(Pos2) -> bool, env: &Env, out: &mut Vec<Routed>) {
        match phase {
            TouchPhase::Start => {
                if ignore {
                    return;
                }
                let cursor = self.cursor_or_centre(env);
                if self.fingers.is_empty() {
                    self.since = env.now;
                    self.last_move = env.now;
                    self.moved = false;
                    self.most = 0;
                    self.held = false;
                    // Right after a tap: this touch drags or double-clicks. A click still waiting
                    // (over the canvas) is dropped; one already sent counts as the first of two.
                    let waiting = self.pending_click.take().is_some();
                    let sent = self.last_tap.take().is_some_and(|t| env.now - t <= CLICK_WAIT);
                    self.armed = waiting || sent;
                    self.armed_clicked = sent && !waiting;
                    out.push(Routed::App(Event::PointerMoved(cursor)));
                }
                self.fingers.retain(|f| f.id != id);
                self.fingers.push(Finger { id, start: pos, pos });
                self.most = self.most.max(self.fingers.len());
                if self.fingers.len() == 2 {
                    if self.pressed {
                        self.pressed = false;
                        press(out, cursor, PointerButton::Primary, false);
                    }
                    self.armed = false;
                    let on_canvas = env.canvas.is_some_and(|r| r.contains(cursor)) && !covered(cursor);
                    self.two_target = Some(if on_canvas { Two::Canvas } else { Two::Scroll });
                    self.two = Some(Two::Undecided);
                } else if self.fingers.len() > 2 && self.two == Some(Two::Undecided) {
                    self.two = self.two_target;
                }
                self.pose = self.pose();
            }
            TouchPhase::Move => {
                let Some(i) = self.fingers.iter().position(|f| f.id == id) else { return };
                let last = self.fingers[i].pos;
                self.fingers[i].pos = pos;
                if self.fingers[i].start.distance(pos) > TAP_SLOP {
                    self.moved = true;
                }
                match (self.fingers.len(), self.two) {
                    (1, _) => {
                        if self.armed && self.moved && !self.pressed {
                            self.pressed = true;
                            let cursor = self.cursor_or_centre(env);
                            press(out, cursor, PointerButton::Primary, true);
                        }
                        self.move_cursor(pos - last, env, out);
                    }
                    // Decided at the end of the frame (`tick`), once every finger's move is in: the
                    // fingers' moves arrive one by one, and the first would make the other look
                    // still.
                    (_, Some(Two::Undecided)) => {}
                    (_, Some(Two::Hold(mover))) => {
                        // The resting finger holds the button; only the other one moves the cursor.
                        if id == mover {
                            self.move_cursor(pos - last, env, out);
                        }
                    }
                    _ => self.two_fingers(env, out),
                }
            }
            TouchPhase::End | TouchPhase::Cancel => {
                let Some(i) = self.fingers.iter().position(|f| f.id == id) else { return };
                self.fingers.remove(i);
                let cursor = self.cursor_or_centre(env);
                if matches!(self.two, Some(Two::Hold(_))) {
                    // Either finger lifting lets go of the button; one left on moves the cursor.
                    self.two = None;
                    if self.pressed {
                        self.pressed = false;
                        press(out, cursor, PointerButton::Primary, false);
                    }
                }
                self.pose = self.pose();
                if !self.fingers.is_empty() {
                    return;
                }
                let lifted = phase == TouchPhase::End;
                let quick = lifted && !self.moved && env.now - self.since <= TAP_TIME;
                if self.pressed {
                    self.pressed = false;
                    press(out, cursor, PointerButton::Primary, false);
                } else if self.most >= 2 {
                    if quick {
                        if self.most == 2 && env.config.trackpad_two_finger_right_click {
                            click(out, cursor, PointerButton::Secondary);
                        } else if self.two_target == Some(Two::Canvas) {
                            out.push(Routed::Tap(self.most));
                        }
                    }
                } else if quick && !self.held {
                    if self.armed {
                        // A second tap: a double click (one click if the first was already sent).
                        click(out, cursor, PointerButton::Primary);
                        if !self.armed_clicked {
                            click(out, cursor, PointerButton::Primary);
                        }
                    } else if env.canvas.is_some_and(|r| r.contains(cursor)) && !covered(cursor) {
                        // Over the canvas the click waits: a second touch may drag (paint) instead.
                        self.pending_click = Some(env.now);
                    } else {
                        click(out, cursor, PointerButton::Primary);
                        self.last_tap = Some(env.now);
                    }
                }
                self.armed = false;
                self.two = None;
                self.two_target = None;
                self.pose = None;
            }
        }
    }

    /// Two fingers have just moved: one resting while the other moves holds the button (when
    /// that's on); both moving pan, zoom and turn the canvas or scroll, from where they are now.
    fn decide_two(&mut self, env: &Env, out: &mut Vec<Routed>) {
        let travel: Vec<f32> = self.fingers.iter().map(|f| f.start.distance(f.pos)).collect();
        let (Some(most), Some(least)) = (travel.iter().copied().reduce(f32::max), travel.iter().copied().reduce(f32::min)) else { return };
        if most <= TWO_LOCK {
            return;
        }
        self.moved = true;
        if env.config.trackpad_rest_to_drag && self.fingers.len() == 2 && least < RESTING {
            let Some(mover) = self.fingers.iter().zip(&travel).find(|(_, t)| **t == most).map(|(f, _)| f.id) else { return };
            self.two = Some(Two::Hold(mover));
            self.pressed = true;
            let cursor = self.cursor_or_centre(env);
            out.push(Routed::App(Event::PointerMoved(cursor)));
            press(out, cursor, PointerButton::Primary, true);
        } else {
            self.two = self.two_target;
            self.pose = self.pose();
        }
    }

    /// Two or more fingers moved: the canvas pans, zooms and turns, or the interface scrolls.
    fn two_fingers(&mut self, env: &Env, out: &mut Vec<Routed>) {
        let (Some(now), Some(last)) = (self.pose(), self.pose) else { return };
        self.pose = Some(now);
        let cursor = self.cursor_or_centre(env);
        match self.two {
            Some(Two::Canvas) => {
                let c = env.config;
                let spread = last.1 > 1.0 && now.1 > 1.0;
                let m = ViewMove {
                    from: last.0,
                    to: if c.pan { now.0 } else { last.0 },
                    scale: if spread && c.zoom { now.1 / last.1 } else { 1.0 },
                    rotate: if spread && c.rotate { wrap_degrees(now.2 - last.2) } else { 0.0 },
                };
                if m.from != m.to || m.scale != 1.0 || m.rotate != 0.0 {
                    out.push(Routed::View(m));
                }
            }
            Some(Two::Scroll) if env.config.two_finger_scroll => {
                out.push(Routed::App(Event::PointerMoved(cursor)));
                out.push(Routed::App(Event::MouseWheel { unit: MouseWheelUnit::Point, delta: now.0 - last.0, phase: TouchPhase::Move, modifiers: Modifiers::NONE }));
            }
            _ => {}
        }
    }

    /// After a frame's touches: what two fingers that have moved do, then the timers: a tap's
    /// click once nothing followed it, a still finger's right-click, a tap and a
    /// held touch's press.
    pub fn tick(&mut self, env: &Env, out: &mut Vec<Routed>) {
        if self.two == Some(Two::Undecided) {
            self.decide_two(env, out);
        }
        if let Some(t) = self.pending_click
            && env.now - t >= CLICK_WAIT
        {
            self.pending_click = None;
            let cursor = self.cursor_or_centre(env);
            click(out, cursor, PointerButton::Primary);
        }
        if self.fingers.len() == 1 && !self.moved && !self.held && !self.pressed && env.now - self.since >= HOLD_TIME {
            self.held = true;
            let cursor = self.cursor_or_centre(env);
            if self.armed {
                // A tap, then a touch held still: hold the button where the cursor is.
                self.pressed = true;
                out.push(Routed::App(Event::PointerMoved(cursor)));
                press(out, cursor, PointerButton::Primary, true);
            } else {
                click(out, cursor, PointerButton::Secondary);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::touch::config::TouchConfig;
    use egui::{Rect, pos2, vec2};

    const SCREEN: Rect = Rect { min: pos2(0.0, 0.0), max: pos2(1000.0, 800.0) };
    const CANVAS: Rect = Rect { min: pos2(100.0, 50.0), max: pos2(900.0, 750.0) };

    struct Rig {
        t: Trackpad,
        config: TouchConfig,
        now: f64,
        covered: bool,
    }

    impl Rig {
        fn new() -> Self {
            // Acceleration off: the moves below are exact (it has its own test).
            let config = TouchConfig { trackpad: true, trackpad_acceleration: false, ..Default::default() };
            Self { t: Trackpad::default(), config, now: 10.0, covered: false }
        }
        fn env(&self) -> Env<'_> {
            Env {
                canvas: Some(CANVAS),
                screen: SCREEN,
                is_finger: &|_| true,
                pen_near: false,
                config: &self.config,
                list_item: &|_| false,
                secondary: &|_| false,
                scroll_strip: None,
                now: self.now,
            }
        }
        fn touch(&mut self, id: u64, phase: TouchPhase, pos: Pos2) -> Vec<Routed> {
            let mut out = Vec::new();
            let covered = self.covered;
            let mut t = std::mem::take(&mut self.t);
            {
                let env = self.env();
                t.touch((TouchId(id), phase, pos), false, &|_| covered, &env, &mut out);
                t.tick(&env, &mut out);
            }
            self.t = t;
            out
        }
        /// Several touches in one frame (as Android batches fingers moving together).
        fn frame(&mut self, touches: &[(u64, TouchPhase, Pos2)]) -> Vec<Routed> {
            let mut out = Vec::new();
            let covered = self.covered;
            let mut t = std::mem::take(&mut self.t);
            {
                let env = self.env();
                for (id, phase, pos) in touches {
                    t.touch((TouchId(*id), *phase, *pos), false, &|_| covered, &env, &mut out);
                }
                t.tick(&env, &mut out);
            }
            self.t = t;
            out
        }
        fn wait(&mut self, s: f64) -> Vec<Routed> {
            self.now += s;
            let mut out = Vec::new();
            let mut t = std::mem::take(&mut self.t);
            t.tick(&self.env(), &mut out);
            self.t = t;
            out
        }
    }

    /// The pointer button presses in `r`: (button, pressed, where).
    fn presses(r: &[Routed]) -> Vec<(PointerButton, bool, Pos2)> {
        r.iter()
            .filter_map(|r| match r {
                Routed::App(Event::PointerButton { button, pressed, pos, .. }) => Some((*button, *pressed, *pos)),
                _ => None,
            })
            .collect()
    }

    fn moved_to(r: &[Routed]) -> Option<Pos2> {
        r.iter().rev().find_map(|r| match r {
            Routed::App(Event::PointerMoved(p)) => Some(*p),
            _ => None,
        })
    }

    use PointerButton::{Primary, Secondary};

    #[test]
    fn sliding_moves_the_cursor_from_where_it_was() {
        let mut g = Rig::new();
        g.touch(1, TouchPhase::Start, pos2(300.0, 600.0));
        let out = g.touch(1, TouchPhase::Move, pos2(340.0, 580.0));
        assert_eq!(moved_to(&out), Some(pos2(540.0, 380.0)), "from the screen's centre, by the finger's move");
        assert!(presses(&out).is_empty(), "moving only hovers");
        g.touch(1, TouchPhase::End, pos2(340.0, 580.0));
        // A new touch somewhere else doesn't jump the cursor.
        let out = g.touch(2, TouchPhase::Start, pos2(800.0, 700.0));
        assert_eq!(moved_to(&out), Some(pos2(540.0, 380.0)));
        let out = g.touch(2, TouchPhase::Move, pos2(790.0, 700.0));
        assert_eq!(moved_to(&out), Some(pos2(530.0, 380.0)));
        // Faster, and never off the screen.
        g.config.trackpad_speed = 2.0;
        let out = g.touch(2, TouchPhase::Move, pos2(-2000.0, 700.0));
        assert_eq!(moved_to(&out), Some(pos2(0.0, 380.0)));
    }

    #[test]
    fn quick_swipes_go_further_with_acceleration() {
        // 10 points in 0.1 s (100 pt/s): as is. 60 in 0.04 s (1500 pt/s): 2.5 times.
        assert_eq!(accelerated(vec2(10.0, 0.0), 0.1), vec2(10.0, 0.0));
        assert_eq!(accelerated(vec2(60.0, 0.0), 0.04), vec2(150.0, 0.0));
        assert_eq!(accelerated(vec2(60.0, 0.0), 0.0), vec2(60.0, 0.0), "no time: as is");
        let mut g = Rig::new();
        g.config.trackpad_acceleration = true;
        g.touch(1, TouchPhase::Start, pos2(300.0, 600.0));
        g.now += 0.016;
        let out = g.touch(1, TouchPhase::Move, pos2(340.0, 600.0)); // 2500 pt/s
        assert_eq!(moved_to(&out), Some(pos2(600.0, 400.0)), "100 points for a quick 40");
    }

    #[test]
    fn a_tap_on_the_canvas_clicks_after_a_moment_and_a_double_tap_double_clicks() {
        let mut g = Rig::new();
        g.t.pen_at(pos2(200.0, 100.0));
        g.touch(1, TouchPhase::Start, pos2(600.0, 600.0));
        let out = g.touch(1, TouchPhase::End, pos2(602.0, 601.0));
        assert!(presses(&out).is_empty(), "the click waits for a possible drag");
        let out = g.wait(0.3);
        assert_eq!(presses(&out), [(Primary, true, pos2(200.0, 100.0)), (Primary, false, pos2(200.0, 100.0))], "at the cursor, not the finger");
        // Two quick taps: a double click.
        g.wait(1.0);
        g.touch(1, TouchPhase::Start, pos2(600.0, 600.0));
        g.touch(1, TouchPhase::End, pos2(600.0, 600.0));
        g.now += 0.1;
        g.touch(2, TouchPhase::Start, pos2(600.0, 600.0));
        g.now += 0.05;
        let out = g.touch(2, TouchPhase::End, pos2(600.0, 600.0));
        assert_eq!(presses(&out).iter().filter(|(_, p, _)| *p).count(), 2);
        assert!(presses(&g.wait(1.0)).is_empty(), "nothing more");
    }

    #[test]
    fn over_the_interface_a_tap_clicks_at_once() {
        let mut g = Rig::new();
        g.covered = true; // a menu or panel under the cursor
        g.touch(1, TouchPhase::Start, pos2(600.0, 600.0));
        let out = g.touch(1, TouchPhase::End, pos2(600.0, 600.0));
        assert_eq!(presses(&out), [(Primary, true, SCREEN.center()), (Primary, false, SCREEN.center())]);
        // A second tap right after: the second click of a double click.
        g.now += 0.1;
        g.touch(2, TouchPhase::Start, pos2(600.0, 600.0));
        g.now += 0.05;
        let out = g.touch(2, TouchPhase::End, pos2(600.0, 600.0));
        assert_eq!(presses(&out).iter().filter(|(_, p, _)| *p).count(), 1);
        // A tap and then a drag still drags (a slider).
        g.wait(1.0);
        g.touch(1, TouchPhase::Start, pos2(600.0, 600.0));
        g.touch(1, TouchPhase::End, pos2(600.0, 600.0));
        g.now += 0.1;
        g.touch(2, TouchPhase::Start, pos2(600.0, 600.0));
        let out = g.touch(2, TouchPhase::Move, pos2(640.0, 600.0));
        assert_eq!(presses(&out), [(Primary, true, SCREEN.center())]);
        assert_eq!(presses(&g.touch(2, TouchPhase::End, pos2(640.0, 600.0))), [(Primary, false, pos2(540.0, 400.0))]);
    }

    #[test]
    fn holding_still_right_clicks() {
        let mut g = Rig::new();
        g.touch(1, TouchPhase::Start, pos2(600.0, 600.0));
        let out = g.wait(0.6);
        assert_eq!(presses(&out), [(Secondary, true, SCREEN.center()), (Secondary, false, SCREEN.center())]);
        assert!(presses(&g.touch(1, TouchPhase::End, pos2(600.0, 600.0))).is_empty(), "and the lift adds nothing");
        assert!(presses(&g.wait(1.0)).is_empty());
    }

    #[test]
    fn tap_then_drag_holds_the_button_to_paint() {
        let mut g = Rig::new();
        g.t.pen_at(pos2(400.0, 400.0));
        g.touch(1, TouchPhase::Start, pos2(600.0, 600.0));
        g.touch(1, TouchPhase::End, pos2(600.0, 600.0));
        g.now += 0.1;
        g.touch(2, TouchPhase::Start, pos2(500.0, 500.0));
        let out = g.touch(2, TouchPhase::Move, pos2(530.0, 500.0));
        assert_eq!(presses(&out), [(Primary, true, pos2(400.0, 400.0))], "pressed where the cursor was: no dot from the tap");
        assert_eq!(moved_to(&out), Some(pos2(430.0, 400.0)));
        let out = g.touch(2, TouchPhase::Move, pos2(560.0, 520.0));
        assert_eq!(moved_to(&out), Some(pos2(460.0, 420.0)));
        let out = g.touch(2, TouchPhase::End, pos2(560.0, 520.0));
        assert_eq!(presses(&out), [(Primary, false, pos2(460.0, 420.0))]);
        assert!(presses(&g.wait(1.0)).is_empty());
        // A tap, then a touch held still: the button is held where the cursor is.
        g.touch(1, TouchPhase::Start, pos2(600.0, 600.0));
        g.touch(1, TouchPhase::End, pos2(600.0, 600.0));
        g.now += 0.1;
        g.touch(2, TouchPhase::Start, pos2(600.0, 600.0));
        assert_eq!(presses(&g.wait(0.6)), [(Primary, true, pos2(460.0, 420.0))]);
        assert_eq!(presses(&g.touch(2, TouchPhase::End, pos2(600.0, 600.0))), [(Primary, false, pos2(460.0, 420.0))]);
    }

    #[test]
    fn a_resting_finger_holds_the_button_while_another_moves_the_cursor() {
        let mut g = Rig::new();
        g.t.pen_at(pos2(400.0, 400.0));
        g.touch(1, TouchPhase::Start, pos2(200.0, 700.0)); // the thumb, resting
        g.touch(2, TouchPhase::Start, pos2(600.0, 600.0));
        let out = g.touch(2, TouchPhase::Move, pos2(620.0, 600.0));
        assert_eq!(presses(&out), [(Primary, true, pos2(400.0, 400.0))], "pressed where the cursor is");
        assert!(!out.iter().any(|r| matches!(r, Routed::View(_))), "not a zoom");
        let out = g.touch(2, TouchPhase::Move, pos2(650.0, 610.0));
        assert_eq!(moved_to(&out), Some(pos2(430.0, 410.0)), "the moving finger drives the cursor");
        assert!(moved_to(&g.touch(1, TouchPhase::Move, pos2(202.0, 701.0))).is_none(), "the resting one doesn't");
        // Lifting the thumb lets go; the other finger then only moves the cursor.
        assert_eq!(presses(&g.touch(1, TouchPhase::End, pos2(202.0, 701.0))), [(Primary, false, pos2(430.0, 410.0))]);
        let out = g.touch(2, TouchPhase::Move, pos2(660.0, 610.0));
        assert!(presses(&out).is_empty());
        assert_eq!(moved_to(&out), Some(pos2(440.0, 410.0)));
        assert!(presses(&g.touch(2, TouchPhase::End, pos2(660.0, 610.0))).is_empty());
        // Switched off, the same move zooms the canvas instead.
        g.wait(1.0);
        g.config.trackpad_rest_to_drag = false;
        g.touch(1, TouchPhase::Start, pos2(200.0, 700.0));
        g.touch(2, TouchPhase::Start, pos2(600.0, 600.0));
        g.touch(2, TouchPhase::Move, pos2(620.0, 600.0));
        let out = g.touch(2, TouchPhase::Move, pos2(650.0, 610.0));
        assert!(presses(&out).is_empty() && out.iter().any(|r| matches!(r, Routed::View(_))), "{out:?}");
    }

    #[test]
    fn two_fingers_move_the_canvas_or_scroll_the_interface() {
        let mut g = Rig::new();
        g.t.pen_at(pos2(500.0, 400.0)); // over the canvas
        g.touch(1, TouchPhase::Start, pos2(300.0, 600.0));
        g.touch(2, TouchPhase::Start, pos2(400.0, 600.0));
        // Both fingers spread apart (their moves come in the same frame).
        g.frame(&[(1, TouchPhase::Move, pos2(280.0, 600.0)), (2, TouchPhase::Move, pos2(420.0, 600.0))]);
        let out = g.frame(&[(1, TouchPhase::Move, pos2(260.0, 600.0)), (2, TouchPhase::Move, pos2(450.0, 600.0))]);
        assert!(out.iter().any(|r| matches!(r, Routed::View(m) if m.scale > 1.0)), "spreading zooms: {out:?}");
        assert!(presses(&out).is_empty());
        g.touch(1, TouchPhase::End, pos2(280.0, 600.0));
        g.touch(2, TouchPhase::End, pos2(450.0, 600.0));
        // A quick two-finger tap on the canvas: its shortcut (undo)...
        g.wait(1.0);
        g.touch(1, TouchPhase::Start, pos2(300.0, 600.0));
        g.touch(2, TouchPhase::Start, pos2(400.0, 600.0));
        g.touch(1, TouchPhase::End, pos2(300.0, 600.0));
        let out = g.touch(2, TouchPhase::End, pos2(400.0, 600.0));
        assert!(out.iter().any(|r| matches!(r, Routed::Tap(2))));
        // ...or a right-click, with that setting.
        g.wait(1.0);
        g.config.trackpad_two_finger_right_click = true;
        g.touch(1, TouchPhase::Start, pos2(300.0, 600.0));
        g.touch(2, TouchPhase::Start, pos2(400.0, 600.0));
        g.touch(1, TouchPhase::End, pos2(300.0, 600.0));
        let out = g.touch(2, TouchPhase::End, pos2(400.0, 600.0));
        assert_eq!(presses(&out), [(Secondary, true, pos2(500.0, 400.0)), (Secondary, false, pos2(500.0, 400.0))]);
        assert!(!out.iter().any(|r| matches!(r, Routed::Tap(_))));
        // Over a panel: two fingers moving together scroll under the cursor.
        g.wait(1.0);
        g.covered = true;
        g.touch(1, TouchPhase::Start, pos2(300.0, 600.0));
        g.touch(2, TouchPhase::Start, pos2(400.0, 600.0));
        g.frame(&[(1, TouchPhase::Move, pos2(300.0, 580.0)), (2, TouchPhase::Move, pos2(400.0, 580.0))]);
        let out = g.frame(&[(1, TouchPhase::Move, pos2(300.0, 560.0)), (2, TouchPhase::Move, pos2(400.0, 560.0))]);
        assert!(out.iter().any(|r| matches!(r, Routed::App(Event::MouseWheel { delta, .. }) if delta.y < 0.0)), "{out:?}");
        assert_eq!(moved_to(&out), Some(pos2(500.0, 400.0)), "at the cursor");
    }
}
