//! Trackpad cursor (Touch Input): fingers move a cursor instead of touching things directly,
//! like a laptop trackpad, so the fingertip never covers what it points at (#4).
//!
//! - One finger sliding moves the cursor by the same amount (times the speed setting), from
//!   wherever it was: it stays put between touches, like a mouse pointer.
//! - A tap clicks at the cursor, a double tap double-clicks, and a finger held still
//!   right-clicks.
//! - A tap and then a touch that drags holds the button down while it drags (painting, moving a
//!   layer, dragging a slider); held still instead, it holds the button where the cursor is. The
//!   first tap's click waits a moment for that second touch, so a stroke doesn't start with a dot.
//! - Two fingers: over the canvas they pan, zoom and rotate it (and quick two- and three-finger
//!   taps run their shortcuts); over the interface they scroll what's under the cursor.
//!
//! The pen draws where it touches, as always (the cursor jumps there); the Shortcut Pad's touches
//! never get here.

use super::router::{Env, Routed, ViewMove};
use egui::{Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, TouchId, TouchPhase, Vec2};

/// A tap is quicker than this (seconds)...
const TAP_TIME: f64 = 0.3;
/// ...and moves less than this (points).
const TAP_SLOP: f32 = 12.0;
/// How long a tap's click waits for a second touch that would drag instead (seconds).
const CLICK_WAIT: f64 = 0.25;
/// A finger held still this long right-clicks (or, after a tap, holds the button).
const HOLD_TIME: f64 = 0.5;

/// What two fingers do, decided when the second one lands.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Two {
    /// The cursor is over the canvas: pan, zoom and rotate it.
    Canvas,
    /// The cursor is over the interface: scroll what's under it.
    Scroll,
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
    /// The button is held (a tap-and-drag).
    pressed: bool,
    /// This touch came right after a tap: dragging holds the button, a lift double-clicks.
    armed: bool,
    /// A tap's click waiting for [`CLICK_WAIT`] (when the tap ended).
    pending_click: Option<f64>,
    /// The hold already right-clicked.
    held: bool,
    two: Option<Two>,
    /// Two fingers' centre, spread and angle as last applied.
    pose: Option<(Pos2, f32, f32)>,
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
                    self.moved = false;
                    self.most = 0;
                    self.held = false;
                    // Right after a tap: its click is dropped; this touch drags or double-clicks.
                    self.armed = self.pending_click.take().is_some();
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
                    self.two = Some(if on_canvas { Two::Canvas } else { Two::Scroll });
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
                if self.fingers.len() == 1 {
                    let cursor = self.cursor_or_centre(env);
                    if self.armed && self.moved && !self.pressed {
                        self.pressed = true;
                        press(out, cursor, PointerButton::Primary, true);
                    }
                    let to = (cursor + (pos - last) * env.config.trackpad_speed).clamp(env.screen.min, env.screen.max);
                    self.cursor = Some(to);
                    out.push(Routed::App(Event::PointerMoved(to)));
                } else {
                    self.two_fingers(env, out);
                }
            }
            TouchPhase::End | TouchPhase::Cancel => {
                let Some(i) = self.fingers.iter().position(|f| f.id == id) else { return };
                self.fingers.remove(i);
                self.pose = self.pose();
                if !self.fingers.is_empty() {
                    return;
                }
                let cursor = self.cursor_or_centre(env);
                let lifted = phase == TouchPhase::End;
                let quick = lifted && !self.moved && env.now - self.since <= TAP_TIME;
                if self.pressed {
                    self.pressed = false;
                    press(out, cursor, PointerButton::Primary, false);
                } else if self.most >= 2 {
                    if quick && self.two == Some(Two::Canvas) {
                        out.push(Routed::Tap(self.most));
                    }
                } else if quick && !self.held {
                    if self.armed {
                        // A second tap: a double click (the first tap's click was held back).
                        click(out, cursor, PointerButton::Primary);
                        click(out, cursor, PointerButton::Primary);
                    } else {
                        self.pending_click = Some(env.now);
                    }
                }
                self.armed = false;
                self.two = None;
                self.pose = None;
            }
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

    /// Timers: a tap's click once nothing followed it, a still finger's right-click, a tap and a
    /// held touch's press.
    pub fn tick(&mut self, env: &Env, out: &mut Vec<Routed>) {
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
    use egui::{Rect, pos2};

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
            Self { t: Trackpad::default(), config: TouchConfig { trackpad: true, ..Default::default() }, now: 10.0, covered: false }
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
    fn a_tap_clicks_at_the_cursor_after_a_moment_and_a_double_tap_double_clicks() {
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
    fn two_fingers_move_the_canvas_or_scroll_the_interface() {
        let mut g = Rig::new();
        g.t.pen_at(pos2(500.0, 400.0)); // over the canvas
        g.touch(1, TouchPhase::Start, pos2(300.0, 600.0));
        g.touch(2, TouchPhase::Start, pos2(400.0, 600.0));
        let out = g.touch(2, TouchPhase::Move, pos2(440.0, 600.0));
        assert!(out.iter().any(|r| matches!(r, Routed::View(m) if m.scale > 1.0)), "spreading zooms: {out:?}");
        assert!(presses(&out).is_empty());
        g.touch(1, TouchPhase::End, pos2(300.0, 600.0));
        g.touch(2, TouchPhase::End, pos2(440.0, 600.0));
        // A quick two-finger tap on the canvas: its shortcut (undo).
        g.wait(1.0);
        g.touch(1, TouchPhase::Start, pos2(300.0, 600.0));
        g.touch(2, TouchPhase::Start, pos2(400.0, 600.0));
        g.touch(1, TouchPhase::End, pos2(300.0, 600.0));
        let out = g.touch(2, TouchPhase::End, pos2(400.0, 600.0));
        assert!(out.iter().any(|r| matches!(r, Routed::Tap(2))));
        // Over a panel: two fingers scroll under the cursor.
        g.wait(1.0);
        g.covered = true;
        g.touch(1, TouchPhase::Start, pos2(300.0, 600.0));
        g.touch(2, TouchPhase::Start, pos2(400.0, 600.0));
        let out = g.touch(2, TouchPhase::Move, pos2(400.0, 560.0));
        assert!(out.iter().any(|r| matches!(r, Routed::App(Event::MouseWheel { delta, .. }) if delta.y < 0.0)), "{out:?}");
        assert_eq!(moved_to(&out), Some(pos2(500.0, 400.0)), "at the cursor");
    }
}
