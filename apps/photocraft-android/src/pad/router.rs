//! The pad's geometry, and touch routing for the whole window.
//!
//! egui turns only the first finger down into the pointer (egui-winit's touch emulation): a
//! finger resting on a pad key would leave the pen without a pointer, a second finger couldn't
//! pinch, and a palm would paint. [`Router`] takes over that emulation (on Android every pointer
//! event is emulated from a touch, winit reports even a mouse as one, so the router drops
//! egui-winit's pointer events and makes its own):
//!
//! - Touches that start on the pad belong to the pad: they reach the app neither as touches nor
//!   as the pointer.
//! - A pen or mouse (told apart from fingers by the Activity's pointer hook) drives the pointer
//!   at once, whatever fingers are down.
//! - A finger elsewhere is held for a moment ([`HOLD_TIME`], or until it moves) before it becomes
//!   the pointer, so a second finger landing with it starts a gesture instead of a stroke or a
//!   click.
//! - On the canvas, two or more fingers pan, pinch-zoom and twist-rotate the view
//!   ([`Routed::View`]), and a quick two- or three-finger tap is a [`Routed::Tap`]. One finger
//!   draws, moves the canvas or does nothing (Touch Input settings).
//! - Over menus, panels and lists, two fingers scroll (wheel events under the fingers). One
//!   finger swiping up or down on a draggable row or slider in a scrolling list (the Brushes,
//!   Layers and preset lists, whose rows reorder when dragged) scrolls the list too; holding
//!   still first ([`LONG_PRESS`]) then dragging reorders, and a sideways drag moves a slider.
//!   A scroll keeps gliding after the fingers lift. A sideways swipe on a strip that scrolls
//!   sideways (the options bar, when wider than the window) scrolls it the same way.
//! - A finger held still on the interface right-clicks (egui's press-and-hold, which needs the
//!   finger's touch events: the router passes on the pointer finger's, without its contact-size
//!   "force", which would read as a pen). A mouse's right button, and a pen whose side button is
//!   set to right-click, press the secondary button.
//! - Palm rejection: fingers that land while the pen is down or near are ignored, and a pen
//!   touching down drops the fingers already resting.
//!
//! A lifted touch leaves the pointer in place until the next frame, so egui still knows how fast
//! it was moving: a flicked menu or list keeps gliding.

use crate::touch::config::{FingerMode, TouchConfig};
use egui::{Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, TouchId, TouchPhase, Vec2, pos2, vec2};

/// The pad's width in points, and in the phone layout.
pub const WIDTH: f32 = 72.0;
pub const COMPACT_WIDTH: f32 = 56.0;
const MARGIN: f32 = 6.0;
/// A key's height when the pad has room; keys shrink evenly towards [`MIN_KEY_H`] to fit.
const KEY_H: f32 = 44.0;
/// The smallest key height, still a comfortable finger target.
pub const MIN_KEY_H: f32 = 26.0;
const GAP: f32 = 6.0;
const WHEEL_GAP: f32 = 12.0;
/// The on-screen keyboard key at the bottom of the pad.
const KEYBOARD_H: f32 = 36.0;

/// A part of the pad.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Key(usize),
    Ring,
    Center,
    /// The always-present key at the bottom that opens the on-screen keyboard.
    Keyboard,
    /// The always-present key above it that enters and leaves full screen.
    ScreenMode,
    /// The pad's background: touches there do nothing (and don't reach the canvas).
    Panel,
}

/// Where the pad's parts are, for a pad of a given size.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub panel: Rect,
    /// Top to bottom: the first half (rounded up) above the wheel, the rest below.
    pub keys: Vec<Rect>,
    pub center: Pos2,
    pub ring_outer: f32,
    pub ring_inner: f32,
    /// The on-screen keyboard key, at the very bottom.
    pub keyboard: Rect,
    /// The full screen key, just above it.
    pub screen_mode: Rect,
}

impl Layout {
    /// `count` keys split around the wheel (the extra one of an odd count above it), centred
    /// vertically in `panel` above the keyboard key at its bottom. Keys are [`KEY_H`] tall where
    /// they fit and shrink evenly, down to [`MIN_KEY_H`], on a shorter panel.
    pub fn new(full: Rect, count: usize) -> Self {
        let w = (full.width() - 2.0 * MARGIN).max(1.0);
        let keyboard = Rect::from_min_size(pos2(full.left() + MARGIN, full.bottom() - MARGIN - KEYBOARD_H), vec2(w, KEYBOARD_H));
        let screen_mode = Rect::from_min_size(pos2(full.left() + MARGIN, keyboard.top() - GAP - KEYBOARD_H), vec2(w, KEYBOARD_H));
        // The keys and wheel share what is left above them.
        let panel = Rect::from_min_max(full.min, pos2(full.right(), (screen_mode.top() - GAP).max(full.top())));
        let wheel = w;
        let above = count.div_ceil(2);
        let below = count / 2;
        let gaps = (above.saturating_sub(1) + below.saturating_sub(1)) as f32 * GAP;
        let fixed = wheel + 2.0 * WHEEL_GAP + gaps;
        let key_h = if count == 0 { KEY_H } else { ((panel.height() - 2.0 * MARGIN - fixed) / count as f32).clamp(MIN_KEY_H, KEY_H) };
        let total = count as f32 * key_h + fixed;
        // Centred; on a panel too short even for the smallest keys, from the top.
        let mut y = (panel.center().y - total / 2.0).max(panel.top() + MARGIN);
        let x = panel.left() + MARGIN;
        let mut keys = Vec::with_capacity(count);
        for i in 0..above {
            keys.push(Rect::from_min_size(pos2(x, y), vec2(w, key_h)));
            y += key_h + if i + 1 < above { GAP } else { 0.0 };
        }
        y += WHEEL_GAP;
        let center = pos2(x + w / 2.0, y + wheel / 2.0);
        y += wheel + WHEEL_GAP;
        for _ in 0..below {
            keys.push(Rect::from_min_size(pos2(x, y), vec2(w, key_h)));
            y += key_h + GAP;
        }
        Self { panel: full, keys, center, ring_outer: wheel / 2.0, ring_inner: wheel * 0.3, keyboard, screen_mode }
    }

    /// The keys' height (the same for all).
    pub fn key_height(&self) -> f32 {
        self.keys.first().map_or(KEY_H, |k| k.height())
    }

    /// What a touch at `p` (inside the pad) lands on. The wheel takes a little slop around its
    /// edge; keys take half the gap around them.
    pub fn hit(&self, p: Pos2) -> Control {
        if self.keyboard.expand(GAP / 2.0).contains(p) {
            return Control::Keyboard;
        }
        if self.screen_mode.expand(GAP / 2.0).contains(p) {
            return Control::ScreenMode;
        }
        let d = p.distance(self.center);
        if d <= self.ring_inner {
            return Control::Center;
        }
        // The whole width of the pad beside the wheel is the ring: easy to hit with a finger.
        if d <= self.ring_outer + WHEEL_GAP / 2.0 || (p.y - self.center.y).abs() <= self.ring_outer + WHEEL_GAP / 2.0 {
            return Control::Ring;
        }
        self.keys.iter().position(|k| k.expand(GAP / 2.0).contains(p)).map_or(Control::Panel, Control::Key)
    }
}

/// A pad touch, for the pad to act on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PadTouch {
    Down(Control, Pos2),
    Move(Control, Pos2),
    Up(Control),
}

/// A move of the canvas view by a gesture: the document point under `from` goes under `to`, the
/// zoom multiplies by `scale` and the view turns `rotate` degrees (clockwise on screen) around it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ViewMove {
    pub from: Pos2,
    pub to: Pos2,
    pub scale: f32,
    pub rotate: f32,
}

/// The routed input, in its original order.
#[derive(Clone, Debug, PartialEq)]
pub enum Routed {
    App(Event),
    Pad(PadTouch),
    /// A gesture moved the canvas view.
    View(ViewMove),
    /// A quick tap with this many fingers on the canvas.
    Tap(usize),
    /// A sideways swipe moved the scrolling strip (the options bar) this far, in points.
    StripScroll(f32),
}

/// What the router needs to know besides the events.
pub struct Env<'a> {
    /// The canvas, when a document is open (as laid out last frame).
    pub canvas: Option<Rect>,
    /// Whether a touch is a finger (not a pen, eraser or mouse).
    pub is_finger: &'a dyn Fn(TouchId) -> bool,
    /// A pen touched or hovered a moment ago.
    pub pen_near: bool,
    pub config: &'a TouchConfig,
    /// Whether a point is on a draggable row or slider inside a scrolling list (as laid out last
    /// frame): a swipe there scrolls rather than drags.
    pub list_item: &'a dyn Fn(Pos2) -> bool,
    /// Whether a pen or mouse touch presses the secondary button (a mouse's right button, a pen's
    /// side button set to right-click).
    pub secondary: &'a dyn Fn(TouchId) -> bool,
    /// A strip that scrolls sideways (the options bar running past the window's width), as laid
    /// out last frame: a sideways swipe there scrolls it.
    pub scroll_strip: Option<Rect>,
    /// The window (the trackpad cursor stays inside it).
    pub screen: Rect,
    /// The frame's time, in seconds.
    pub now: f64,
}

/// How long a lone finger waits for a second one before it becomes the pointer.
pub const HOLD_TIME: f64 = 0.1;
/// On a list row, how long a finger holds still before it grabs the row (to reorder it).
pub const LONG_PRESS: f64 = 0.35;
/// A scroll still moving this fast (points per second) when the fingers lift keeps gliding,
/// slowing by [`GLIDE_FRICTION`] (points per second, per second).
const GLIDE_MIN: f32 = 120.0;
const GLIDE_FRICTION: f32 = 1500.0;
/// A held finger moving this far (points) becomes the pointer at once.
const HOLD_SLOP: f32 = 10.0;
/// A tap lasts at most this long (first finger down to last finger up)...
const TAP_TIME: f64 = 0.35;
/// ...and no finger moves further than this (points).
const TAP_SLOP: f32 = 16.0;
/// A gesture starts once the fingers move together this far (points), change their spread by
/// this fraction, or turn this many degrees; small jitters during a tap don't move the view.
const LOCK_MOVE: f32 = 10.0;
const LOCK_SCALE: f32 = 0.06;
const LOCK_ANGLE: f32 = 8.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// A pen, eraser or mouse.
    Pen,
    /// A finger over the interface (menus, panels, dialogs).
    Ui,
    /// A finger on the canvas.
    Canvas,
    /// A palm, or a finger the settings ignore: nothing until it lifts.
    Ignored,
}

#[derive(Clone, Copy, Debug)]
struct Finger {
    id: TouchId,
    role: Role,
    start: Pos2,
    pos: Pos2,
}

/// A lone finger waiting to become the pointer.
#[derive(Clone, Copy, Debug)]
struct Hold {
    id: TouchId,
    start: Pos2,
    pos: Pos2,
    since: f64,
    /// On a list row: a vertical swipe scrolls, and grabbing needs [`LONG_PRESS`].
    list: bool,
    /// On a sideways-scrolling strip: a sideways swipe scrolls, and grabbing needs
    /// [`LONG_PRESS`].
    strip: bool,
}

/// A scroll gliding on after the fingers lifted (under the pointer, left where they lifted).
#[derive(Clone, Copy, Debug)]
struct Glide {
    velocity: Vec2,
    /// Glides the strip rather than wheeling what's under the pointer.
    strip: bool,
}

/// The canvas fingers from the first one down to the last one up (for taps).
#[derive(Clone, Copy, Debug)]
struct Sequence {
    since: f64,
    most: usize,
    moved: bool,
    gestured: bool,
}

/// Where the canvas fingers are: their centre, the spread and angle of the first two.
#[derive(Clone, Copy, Debug)]
struct Pose {
    center: Pos2,
    span: f32,
    angle: f32,
    count: usize,
}

#[derive(Default)]
pub struct Router {
    /// Touches that started on the pad, and what they started on (a few fingers at most).
    pad: Vec<(TouchId, Control)>,
    /// Touches elsewhere, in the order they went down.
    fingers: Vec<Finger>,
    /// The touch driving the pointer.
    primary: Option<TouchId>,
    hold: Option<Hold>,
    /// Fingers scrolling the interface (two anywhere, or one swiping a list): their last centre,
    /// and recent centres with their times (for the glide).
    scroll: Option<Pos2>,
    /// The axes the scroll moves (a strip scrolls only sideways).
    scroll_axes: Vec2,
    trail: Vec<(f64, Pos2)>,
    glide: Option<Glide>,
    /// The previous frame's time (the glide's step).
    last_now: Option<f64>,
    sequence: Option<Sequence>,
    /// The canvas gesture: where the fingers were last applied, and whether it has started.
    pose: Option<Pose>,
    anchor: Option<Pose>,
    locked: bool,
    /// The pointer was released last frame and is still to be taken away.
    gone_pending: bool,
    /// The pointer pressed the secondary button.
    secondary: bool,
    /// The interface finger whose touch events reach egui (for press-and-hold), as last sent.
    forwarded: Option<Event>,
    /// Fingers moving a cursor instead (Touch Input › Trackpad Cursor).
    trackpad: super::trackpad::Trackpad,
}

fn pointer_down(out: &mut Vec<Routed>, pos: Pos2, button: PointerButton) {
    out.push(Routed::App(Event::PointerMoved(pos)));
    out.push(Routed::App(Event::PointerButton { pos, button, pressed: true, modifiers: Modifiers::NONE }));
}

fn pointer_release(out: &mut Vec<Routed>, pos: Pos2, button: PointerButton) {
    out.push(Routed::App(Event::PointerButton { pos, button, pressed: false, modifiers: Modifiers::NONE }));
}

/// A touch event in another phase and without its force (a finger's contact size, which egui
/// would take for a pen's pressure and so turn press-and-hold off).
fn unforced(mut e: Event, phase: TouchPhase) -> Event {
    if let Event::Touch { force, phase: p, .. } = &mut e {
        *force = None;
        *p = phase;
    }
    e
}

fn wrap_degrees(d: f32) -> f32 {
    (d + 540.0).rem_euclid(360.0) - 180.0
}

impl Router {
    /// Route one frame's events. `pad` is where the pad was drawn last frame (`None`: hidden).
    /// `covered` says whether a window or menu was drawn over a point last frame: touches there
    /// belong to it, not to the pad or the canvas.
    pub fn route(&mut self, events: Vec<Event>, pad: Option<&Layout>, covered: impl Fn(Pos2) -> bool, env: &Env) -> Vec<Routed> {
        let mut out = Vec::with_capacity(events.len() + 4);
        if pad.is_none() {
            self.pad.clear();
        }
        if self.glide.is_none() && std::mem::take(&mut self.gone_pending) && self.primary.is_none() {
            out.push(Routed::App(Event::PointerGone));
        }
        let dt = self.last_now.map_or(0.0, |t| (env.now - t).clamp(0.0, 0.1)) as f32;
        self.last_now = Some(env.now);
        if events.iter().any(|e| matches!(e, Event::Touch { phase: TouchPhase::Start, .. })) {
            // A new touch stops a gliding list where it is.
            self.glide = None;
        }
        if let Some(g) = &mut self.glide {
            if g.strip {
                out.push(Routed::StripScroll(g.velocity.x * dt));
            } else {
                out.push(Routed::App(Event::MouseWheel {
                    unit: MouseWheelUnit::Point,
                    delta: g.velocity * dt,
                    phase: TouchPhase::Move,
                    modifiers: Modifiers::NONE,
                }));
            }
            let speed = g.velocity.length();
            let slower = (speed - GLIDE_FRICTION * dt).max(0.0);
            g.velocity = if speed > 0.0 { g.velocity * (slower / speed) } else { Vec2::ZERO };
            if slower < GLIDE_MIN / 3.0 {
                self.glide = None;
                self.gone_pending = true;
            }
        }
        for e in events {
            match e {
                // egui-winit's emulated pointer: replaced by the router's own.
                Event::PointerMoved(_) | Event::PointerButton { .. } | Event::PointerGone => {}
                Event::Touch { id, phase, pos, .. } => {
                    if let Some(control) = self.pad.iter().find(|(t, _)| *t == id).map(|(_, c)| *c) {
                        match phase {
                            TouchPhase::Move => out.push(Routed::Pad(PadTouch::Move(control, pos))),
                            TouchPhase::End | TouchPhase::Cancel => {
                                self.pad.retain(|(t, _)| *t != id);
                                out.push(Routed::Pad(PadTouch::Up(control)));
                            }
                            TouchPhase::Start => {}
                        }
                        continue;
                    }
                    if phase == TouchPhase::Start
                        && let Some(layout) = pad
                        && layout.panel.contains(pos)
                        && !covered(pos)
                    {
                        let control = layout.hit(pos);
                        self.pad.retain(|(t, _)| *t != id);
                        self.pad.push((id, control));
                        out.push(Routed::Pad(PadTouch::Down(control, pos)));
                        continue;
                    }
                    if env.config.trackpad && (env.is_finger)(id) {
                        // A palm beside the pen does nothing.
                        let palm = env.config.palm_rejection && (env.pen_near || self.count(Role::Pen) > 0);
                        self.trackpad.touch((id, phase, pos), palm, &covered, env, &mut out);
                        continue;
                    }
                    if env.config.trackpad && phase != TouchPhase::End && phase != TouchPhase::Cancel {
                        self.trackpad.pen_at(pos);
                    }
                    match phase {
                        TouchPhase::Start => self.start(e, id, pos, &covered, env, &mut out),
                        TouchPhase::Move => self.moved(e, id, pos, env, &mut out),
                        TouchPhase::End | TouchPhase::Cancel => self.ended(e, id, pos, phase == TouchPhase::End, env, &mut out),
                    }
                }
                other => out.push(Routed::App(other)),
            }
        }
        if env.config.trackpad {
            self.trackpad.tick(env, &mut out);
        }
        // A finger held long enough without a second one becomes the pointer.
        if let Some(h) = self.hold
            && env.now - h.since >= if h.list || h.strip { LONG_PRESS } else { HOLD_TIME }
        {
            self.commit(&mut out);
        }
        out
    }

    /// Whether the router needs frames: a pad touch is down, a finger is waiting or the pointer
    /// is still to be taken away.
    pub fn wants_frames(&self) -> bool {
        !self.pad.is_empty() || self.hold.is_some() || self.gone_pending || self.glide.is_some() || self.trackpad.wants_frames()
    }

    /// Where the trackpad cursor is (Touch Input › Trackpad Cursor), to draw it.
    pub fn trackpad_cursor(&self) -> Option<Pos2> {
        self.trackpad.cursor()
    }

    /// Whether a pad touch is down.
    pub fn pad_active(&self) -> bool {
        !self.pad.is_empty()
    }

    fn count(&self, role: Role) -> usize {
        self.fingers.iter().filter(|f| f.role == role).count()
    }

    fn start(&mut self, e: Event, id: TouchId, pos: Pos2, covered: &impl Fn(Pos2) -> bool, env: &Env, out: &mut Vec<Routed>) {
        self.fingers.retain(|f| f.id != id);
        if !(env.is_finger)(id) {
            if env.config.palm_rejection {
                self.reject_fingers(out);
            }
            self.fingers.push(Finger { id, role: Role::Pen, start: pos, pos });
            out.push(Routed::App(e));
            if self.primary.is_none() {
                self.drop_hold();
                self.primary = Some(id);
                self.secondary = (env.secondary)(id);
                pointer_down(out, pos, self.button());
            }
            return;
        }
        let pen_busy = env.config.palm_rejection && (env.pen_near || self.count(Role::Pen) > 0);
        let on_canvas = env.canvas.is_some_and(|r| r.contains(pos)) && !covered(pos);
        let role = if pen_busy {
            Role::Ignored
        } else if on_canvas {
            Role::Canvas
        } else {
            Role::Ui
        };
        self.fingers.push(Finger { id, role, start: pos, pos });
        match role {
            Role::Ui => {
                let others: Vec<TouchId> = self.fingers.iter().filter(|f| f.role == Role::Ui && f.id != id).map(|f| f.id).collect();
                if others.is_empty() && self.primary.is_none() && self.hold.is_none() {
                    let strip = env.scroll_strip.is_some_and(|r| r.contains(pos));
                    self.hold = Some(Hold { id, start: pos, pos, since: env.now, list: !strip && (env.list_item)(pos), strip });
                    if env.config.long_press_right_click {
                        let t = unforced(e, TouchPhase::Start);
                        out.push(Routed::App(t.clone()));
                        self.forwarded = Some(t);
                    }
                } else if env.config.two_finger_scroll && others.len() == 1 && self.hold.is_some_and(|h| h.id == others[0]) {
                    // Two fingers together over the interface: they scroll, nothing is clicked.
                    self.hold = None;
                    self.unforward(out);
                    let c = self.center(Role::Ui);
                    self.scroll = Some(c);
                    self.scroll_axes = vec2(1.0, 1.0);
                    self.trail = vec![(env.now, c)];
                    out.push(Routed::App(Event::PointerMoved(c)));
                }
            }
            Role::Canvas => {
                let n = self.count(Role::Canvas);
                if n == 1 {
                    self.sequence = Some(Sequence { since: env.now, most: 1, moved: false, gestured: false });
                    self.locked = false;
                    if env.config.finger == FingerMode::Draw && self.primary.is_none() && self.hold.is_none() {
                        self.hold = Some(Hold { id, start: pos, pos, since: env.now, list: false, strip: false });
                    }
                } else {
                    // A second finger: a gesture, not a stroke. A finger that had already become
                    // the pointer lets go.
                    if self.hold.is_some_and(|h| self.is_canvas(h.id)) {
                        self.hold = None;
                    }
                    if let Some(p) = self.primary
                        && let Some(f) = self.fingers.iter().find(|f| f.id == p && f.role == Role::Canvas).copied()
                    {
                        self.primary = None;
                        pointer_release(out, f.pos, self.button());
                        self.gone_pending = true;
                    }
                }
                if let Some(s) = &mut self.sequence {
                    s.most = s.most.max(n);
                }
                self.rebase(env);
            }
            Role::Pen | Role::Ignored => {}
        }
    }

    fn moved(&mut self, e: Event, id: TouchId, pos: Pos2, env: &Env, out: &mut Vec<Routed>) {
        let Some(f) = self.fingers.iter_mut().find(|f| f.id == id) else { return };
        f.pos = pos;
        let (role, start) = (f.role, f.start);
        if role == Role::Pen {
            out.push(Routed::App(e));
        } else if self.forwarded_id() == Some(id) {
            let t = unforced(e, TouchPhase::Move);
            out.push(Routed::App(t.clone()));
            self.forwarded = Some(t);
        }
        if let Some(h) = &mut self.hold
            && h.id == id
        {
            h.pos = pos;
            if h.start.distance(pos) <= HOLD_SLOP {
                return;
            }
            let d = pos - h.start;
            let along_list = h.list && d.y.abs() > d.x.abs();
            let along_strip = h.strip && d.x.abs() > d.y.abs();
            if !(along_list || along_strip) {
                self.commit(out);
                return;
            }
            // A swipe along a list or strip scrolls it; nothing is grabbed.
            self.scroll_axes = if along_strip { vec2(1.0, 0.0) } else { vec2(1.0, 1.0) };
            let start = h.start;
            self.hold = None;
            self.unforward(out);
            self.scroll = Some(start);
            self.trail = vec![(env.now, start)];
            out.push(Routed::App(Event::PointerMoved(start)));
        }
        if self.primary == Some(id) {
            out.push(Routed::App(Event::PointerMoved(pos)));
            return;
        }
        match role {
            Role::Ui => {
                if let Some(last) = self.scroll {
                    let c = self.center(Role::Ui);
                    if self.scroll_axes.y == 0.0 {
                        // The strip scrolls itself: as the wheel, the dropdown under the finger
                        // would step through its options instead.
                        out.push(Routed::StripScroll(c.x - last.x));
                    } else {
                        out.push(Routed::App(Event::PointerMoved(c)));
                        out.push(Routed::App(Event::MouseWheel {
                            unit: MouseWheelUnit::Point,
                            delta: c - last,
                            phase: TouchPhase::Move,
                            modifiers: Modifiers::NONE,
                        }));
                    }
                    self.scroll = Some(c);
                    self.trail.push((env.now, c));
                    self.trail.retain(|(t, _)| env.now - t <= 0.12);
                }
            }
            Role::Canvas => {
                if start.distance(pos) > TAP_SLOP
                    && let Some(s) = &mut self.sequence
                {
                    s.moved = true;
                }
                self.gesture(env, out);
            }
            Role::Pen | Role::Ignored => {}
        }
    }

    fn ended(&mut self, e: Event, id: TouchId, pos: Pos2, lifted: bool, env: &Env, out: &mut Vec<Routed>) {
        let Some(i) = self.fingers.iter().position(|f| f.id == id) else { return };
        let f = self.fingers.remove(i);
        if f.role == Role::Pen {
            out.push(Routed::App(e));
        } else if self.forwarded_id() == Some(id) {
            self.forwarded = None;
            out.push(Routed::App(unforced(e, if lifted { TouchPhase::End } else { TouchPhase::Cancel })));
        }
        if self.hold.is_some_and(|h| h.id == id) {
            // Lifted before it became the pointer: a quick tap (a click, or a dot of paint).
            if lifted {
                self.commit(out);
            } else {
                self.hold = None;
            }
        }
        if self.primary == Some(id) {
            self.primary = None;
            pointer_release(out, pos, self.button());
            if lifted {
                // Kept for a frame, so egui sees the flick's speed.
                self.gone_pending = true;
            } else {
                out.push(Routed::App(Event::PointerGone));
            }
        }
        match f.role {
            Role::Ui => {
                if self.scroll.is_some() && self.count(Role::Ui) < 2 {
                    // Lifted mid-scroll: keep gliding at the fingers' speed.
                    self.scroll = None;
                    let velocity = match (self.trail.first(), self.trail.last()) {
                        (Some((t0, p0)), Some((t1, p1))) if t1 - t0 > 0.01 => (*p1 - *p0) / (t1 - t0) as f32 * self.scroll_axes,
                        _ => Vec2::ZERO,
                    };
                    self.trail.clear();
                    if lifted && velocity.length() >= GLIDE_MIN {
                        self.glide = Some(Glide { velocity, strip: self.scroll_axes.y == 0.0 });
                    } else {
                        self.gone_pending = true;
                    }
                }
            }
            Role::Canvas => {
                if f.start.distance(pos) > TAP_SLOP
                    && let Some(s) = &mut self.sequence
                {
                    s.moved = true;
                }
                if self.count(Role::Canvas) == 0 {
                    if let Some(s) = self.sequence.take()
                        && lifted
                        && !s.moved
                        && !s.gestured
                        && s.most >= 2
                        && env.now - s.since <= TAP_TIME
                    {
                        out.push(Routed::Tap(s.most));
                    }
                    self.pose = None;
                    self.anchor = None;
                    self.locked = false;
                } else {
                    self.rebase(env);
                }
            }
            Role::Pen | Role::Ignored => {}
        }
    }

    fn is_canvas(&self, id: TouchId) -> bool {
        self.fingers.iter().any(|f| f.id == id && f.role == Role::Canvas)
    }

    /// The held finger becomes the pointer, pressed where it went down.
    fn commit(&mut self, out: &mut Vec<Routed>) {
        let Some(h) = self.hold.take() else { return };
        if self.primary.is_some() {
            return;
        }
        self.primary = Some(h.id);
        self.secondary = false;
        self.gone_pending = false;
        pointer_down(out, h.start, self.button());
        if h.pos != h.start {
            out.push(Routed::App(Event::PointerMoved(h.pos)));
        }
    }

    fn drop_hold(&mut self) {
        self.hold = None;
    }

    fn button(&self) -> PointerButton {
        if self.secondary { PointerButton::Secondary } else { PointerButton::Primary }
    }

    fn forwarded_id(&self) -> Option<TouchId> {
        match self.forwarded {
            Some(Event::Touch { id, .. }) => Some(id),
            _ => None,
        }
    }

    /// The forwarded finger stops being a press: egui forgets its touch (no press-and-hold).
    fn unforward(&mut self, out: &mut Vec<Routed>) {
        if let Some(t) = self.forwarded.take() {
            out.push(Routed::App(unforced(t, TouchPhase::Cancel)));
        }
    }

    /// A pen came down: fingers already resting are palms. They let go of the pointer and do
    /// nothing until lifted.
    fn reject_fingers(&mut self, out: &mut Vec<Routed>) {
        self.hold = None;
        self.scroll = None;
        self.glide = None;
        self.unforward(out);
        if let Some(p) = self.primary
            && let Some(f) = self.fingers.iter().find(|f| f.id == p && f.role != Role::Pen).copied()
        {
            self.primary = None;
            pointer_release(out, f.pos, self.button());
            out.push(Routed::App(Event::PointerGone));
        }
        for f in &mut self.fingers {
            if matches!(f.role, Role::Ui | Role::Canvas) {
                f.role = Role::Ignored;
            }
        }
        self.sequence = None;
        self.pose = None;
        self.anchor = None;
        self.locked = false;
    }

    fn center(&self, role: Role) -> Pos2 {
        let pts: Vec<Pos2> = self.fingers.iter().filter(|f| f.role == role).map(|f| f.pos).collect();
        if pts.is_empty() {
            return Pos2::ZERO;
        }
        let sum = pts.iter().fold(vec2(0.0, 0.0), |a, p| a + p.to_vec2());
        (sum / pts.len() as f32).to_pos2()
    }

    /// Where the canvas fingers are now, when they make a gesture (one finger only when it moves
    /// the canvas).
    fn pose_now(&self, env: &Env) -> Option<Pose> {
        let fingers: Vec<Pos2> = self.fingers.iter().filter(|f| f.role == Role::Canvas).map(|f| f.pos).collect();
        let count = fingers.len();
        if count == 0 || (count == 1 && env.config.finger != FingerMode::Navigate) {
            return None;
        }
        let center = self.center(Role::Canvas);
        let (span, angle) = match fingers.as_slice() {
            [a, b, ..] => (a.distance(*b), (b.y - a.y).atan2(b.x - a.x).to_degrees()),
            _ => (0.0, 0.0),
        };
        Some(Pose { center, span, angle, count })
    }

    /// The fingers changed: measure from where they are now (no jump), keeping a started gesture
    /// going.
    fn rebase(&mut self, env: &Env) {
        self.pose = self.pose_now(env);
        self.anchor = self.pose;
    }

    fn gesture(&mut self, env: &Env, out: &mut Vec<Routed>) {
        let (Some(now), Some(anchor)) = (self.pose_now(env), self.anchor) else { return };
        let Some(last) = self.pose else { return };
        if now.count != last.count {
            self.rebase(env);
            return;
        }
        let two = now.count >= 2 && last.span > 1.0 && now.span > 1.0;
        if !self.locked {
            let turned = two && wrap_degrees(now.angle - anchor.angle).abs() > LOCK_ANGLE;
            let spread = two && anchor.span > 1.0 && (now.span / anchor.span).ln().abs() > LOCK_SCALE;
            if now.center.distance(anchor.center) <= LOCK_MOVE && !turned && !spread {
                return;
            }
            self.locked = true;
            if let Some(s) = &mut self.sequence {
                s.gestured = true;
            }
        }
        let c = env.config;
        let pan = if now.count == 1 { true } else { c.pan };
        let m = ViewMove {
            from: last.center,
            to: if pan { now.center } else { last.center },
            scale: if two && c.zoom { now.span / last.span } else { 1.0 },
            rotate: if two && c.rotate { wrap_degrees(now.angle - last.angle) } else { 0.0 },
        };
        self.pose = Some(now);
        if m.from != m.to || m.scale != 1.0 || m.rotate != 0.0 {
            out.push(Routed::View(m));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{TouchDeviceId, Vec2};

    fn layout() -> Layout {
        Layout::new(Rect::from_min_size(pos2(1000.0, 0.0), vec2(WIDTH, 800.0)), 4)
    }

    fn touch(id: u64, phase: TouchPhase, pos: Pos2) -> Event {
        Event::Touch { device_id: TouchDeviceId(1), id: TouchId(id), phase, pos, force: Some(1.0) }
    }

    /// Touch ids from 100 up are pens (from 200 up, a mouse with its right button down).
    fn finger(id: TouchId) -> bool {
        id.0 < 100
    }

    const CANVAS: Rect = Rect { min: pos2(100.0, 50.0), max: pos2(900.0, 750.0) };
    /// A sideways-scrolling strip above the canvas (an options bar wider than the window).
    const STRIP: Rect = Rect { min: pos2(100.0, 10.0), max: pos2(900.0, 40.0) };
    /// A scrolling list of draggable rows (outside the canvas and the pad).
    const LIST: Rect = Rect { min: pos2(920.0, 100.0), max: pos2(990.0, 600.0) };

    struct Rig {
        r: Router,
        config: TouchConfig,
        now: f64,
        pen_near: bool,
    }

    impl Rig {
        fn new() -> Self {
            Self { r: Router::default(), config: TouchConfig::default(), now: 10.0, pen_near: false }
        }
        fn with(config: TouchConfig) -> Self {
            Self { config, ..Self::new() }
        }
        fn route(&mut self, events: Vec<Event>, pad: Option<&Layout>, covered: impl Fn(Pos2) -> bool) -> Vec<Routed> {
            let list = |p: Pos2| LIST.contains(p);
            let env = Env {
                canvas: Some(CANVAS),
                is_finger: &finger,
                pen_near: self.pen_near,
                config: &self.config,
                list_item: &list,
                secondary: &|id: TouchId| id.0 >= 200,
                scroll_strip: Some(STRIP),
                screen: Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 1000.0)),
                now: self.now,
            };
            self.r.route(events, pad, covered, &env)
        }
        fn send(&mut self, events: Vec<Event>) -> Vec<Routed> {
            let l = layout();
            self.route(events, Some(&l), |_| false)
        }
        fn wait(&mut self, s: f64) -> Vec<Routed> {
            self.now += s;
            self.send(vec![])
        }
    }

    /// The app's events, less the interface finger's forwarded touches (see [`forwarded`]).
    fn app_events(r: &[Routed]) -> Vec<&Event> {
        r.iter()
            .filter_map(|r| match r {
                Routed::App(Event::Touch { force: None, .. }) => None,
                Routed::App(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    /// The forwarded touches' phases, for egui's press-and-hold.
    fn forwarded(r: &[Routed]) -> Vec<TouchPhase> {
        r.iter()
            .filter_map(|r| match r {
                Routed::App(Event::Touch { force: None, phase, .. }) => Some(*phase),
                _ => None,
            })
            .collect()
    }

    fn presses(r: &[Routed]) -> Vec<bool> {
        app_events(r)
            .into_iter()
            .filter_map(|e| match e {
                Event::PointerButton { pressed, .. } => Some(*pressed),
                _ => None,
            })
            .collect()
    }

    fn views(r: &[Routed]) -> Vec<ViewMove> {
        r.iter()
            .filter_map(|r| match r {
                Routed::View(m) => Some(*m),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn layout_fits_the_pad_and_parts_are_hit() {
        let l = layout();
        for k in &l.keys {
            assert!(l.panel.contains_rect(*k), "{k:?}");
        }
        assert!(l.keys[1].bottom() < l.center.y - l.ring_outer && l.keys[2].top() > l.center.y + l.ring_outer, "keys above and below the wheel");
        assert_eq!(l.hit(l.center), Control::Center);
        assert_eq!(l.hit(l.center + vec2(l.ring_outer - 2.0, 0.0)), Control::Ring);
        for (i, k) in l.keys.iter().enumerate() {
            assert_eq!(l.hit(k.center()), Control::Key(i));
        }
        assert_eq!(l.hit(pos2(l.panel.center().x, l.panel.top() + 2.0)), Control::Panel);
        assert_eq!(l.hit(l.keyboard.center()), Control::Keyboard);
        assert_eq!(l.hit(l.screen_mode.center()), Control::ScreenMode);
        assert!(l.screen_mode.bottom() < l.keyboard.top(), "the full screen key sits above the keyboard key");
        assert!(l.panel.contains_rect(l.keyboard) && l.keyboard.bottom() > l.panel.bottom() - 10.0, "at the very bottom");
    }

    #[test]
    fn any_key_count_splits_around_the_wheel_and_fits() {
        for count in 2..=10 {
            let l = Layout::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, 800.0)), count);
            assert_eq!(l.keys.len(), count);
            let above = l.keys.iter().filter(|k| k.bottom() < l.center.y - l.ring_outer).count();
            let below = l.keys.iter().filter(|k| k.top() > l.center.y + l.ring_outer).count();
            assert_eq!((above, below), (count.div_ceil(2), count / 2), "{count} keys");
            for k in &l.keys {
                assert!(l.panel.contains_rect(*k), "{count} keys: {k:?}");
            }
            assert!(l.keys.iter().all(|k| k.bottom() < l.screen_mode.top()), "{count} keys: above the full screen and keyboard keys");
            for pair in l.keys.windows(2) {
                assert!(pair[0].bottom() <= pair[1].top(), "keys don't overlap");
            }
            for (i, k) in l.keys.iter().enumerate() {
                assert_eq!(l.hit(k.center()), Control::Key(i), "{count} keys: key {i}");
            }
            assert_eq!(l.hit(l.center), Control::Center);
        }
    }

    #[test]
    fn keys_shrink_to_fit_a_short_screen_and_no_further() {
        let tall = Layout::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, 800.0)), 10);
        assert_eq!(tall.key_height(), 44.0, "10 full-size keys fit 800 points");
        let short = Layout::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, 500.0)), 10);
        assert!(short.key_height() < 44.0 && short.key_height() >= MIN_KEY_H, "{}", short.key_height());
        assert!(short.keys.iter().all(|k| short.panel.contains_rect(*k)), "everything fits 500 points");
        let tiny = Layout::new(Rect::from_min_size(pos2(0.0, 0.0), vec2(WIDTH, 200.0)), 10);
        assert_eq!(tiny.key_height(), MIN_KEY_H, "never smaller than a finger target");
        assert!(tiny.keys[0].top() >= tiny.panel.top(), "starts at the top rather than above it");
    }

    /// A finger holds a pad key, then the pen draws: the pen drives the pointer at once, and the
    /// finger never reaches the app.
    #[test]
    fn the_pen_draws_while_a_finger_holds_a_key() {
        let l = layout();
        let mut g = Rig::new();
        let key = l.keys[2].center();
        // egui-winit made the finger the pointer: those events are dropped.
        let out = g.send(vec![
            touch(1, TouchPhase::Start, key),
            Event::PointerMoved(key),
            Event::PointerButton { pos: key, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() },
        ]);
        assert_eq!(out, vec![Routed::Pad(PadTouch::Down(Control::Key(2), key))]);
        assert!(g.r.pad_active());

        let (a, b) = (pos2(300.0, 300.0), pos2(350.0, 320.0));
        let out = g.send(vec![touch(100, TouchPhase::Start, a)]);
        assert!(matches!(app_events(&out).as_slice(), [Event::Touch { .. }, Event::PointerMoved(p), Event::PointerButton { pressed: true, .. }] if *p == a));
        let out = g.send(vec![touch(100, TouchPhase::Move, b), touch(1, TouchPhase::Move, key)]);
        assert!(matches!(app_events(&out).as_slice(), [Event::Touch { .. }, Event::PointerMoved(p)] if *p == b));
        assert!(out.contains(&Routed::Pad(PadTouch::Move(Control::Key(2), key))));
        let out = g.send(vec![touch(100, TouchPhase::End, b), touch(1, TouchPhase::End, key)]);
        assert_eq!(presses(&out), [false]);
        assert_eq!(out.last(), Some(&Routed::Pad(PadTouch::Up(Control::Key(2)))));
        assert!(!g.r.pad_active());
        // The pointer leaves a frame later (egui keeps the flick's speed meanwhile).
        assert!(matches!(app_events(&g.wait(0.016)).as_slice(), [Event::PointerGone]));
    }

    /// A window or menu over the pad (the settings window, a menu dropping down) gets the
    /// touches there; the rest of the pad still works.
    #[test]
    fn a_window_over_the_pad_takes_its_touches() {
        let l = layout();
        let mut g = Rig::new();
        let covered_key = l.keys[0].center();
        let out = g.route(vec![touch(1, TouchPhase::Start, covered_key)], Some(&l), |p| p.y < l.center.y);
        assert!(!out.iter().any(|r| matches!(r, Routed::Pad(_))), "{out:?}");
        let out = g.route(vec![touch(1, TouchPhase::End, covered_key)], Some(&l), |_| false);
        assert_eq!(presses(&out), [true, false], "a quick tap clicks the window");
        let free_key = l.keys[3].center();
        let out = g.route(vec![touch(2, TouchPhase::Start, free_key)], Some(&l), |p| p.y < l.center.y);
        // (The tap's pointer leaves first, a frame after its release.)
        assert_eq!(out, vec![Routed::App(Event::PointerGone), Routed::Pad(PadTouch::Down(Control::Key(3), free_key))]);
    }

    #[test]
    fn a_lone_finger_becomes_the_pointer_after_a_moment_or_a_move() {
        let mut g = Rig::new();
        let p = pos2(300.0, 300.0);
        assert!(presses(&g.send(vec![touch(1, TouchPhase::Start, p)])).is_empty(), "held");
        assert!(g.r.wants_frames());
        assert_eq!(presses(&g.wait(HOLD_TIME + 0.01)), [true], "then pressed where it went down");
        g.send(vec![touch(1, TouchPhase::End, p)]);
        // Moving commits at once.
        g.wait(0.5);
        g.send(vec![touch(2, TouchPhase::Start, p)]);
        let out = g.send(vec![touch(2, TouchPhase::Move, p + vec2(30.0, 0.0))]);
        assert!(
            matches!(app_events(&out).as_slice(), [Event::PointerMoved(a), Event::PointerButton { pressed: true, .. }, Event::PointerMoved(b)] if *a == p && b.x == 330.0)
        );
    }

    #[test]
    fn two_fingers_on_the_canvas_pinch_and_twist_instead_of_painting() {
        let mut g = Rig::new();
        let (a, b) = (pos2(400.0, 400.0), pos2(500.0, 400.0));
        let out = g.send(vec![touch(1, TouchPhase::Start, a), touch(2, TouchPhase::Start, b)]);
        assert!(presses(&out).is_empty(), "no stroke");
        // Small jitters don't move the view.
        assert!(views(&g.send(vec![touch(2, TouchPhase::Move, b + vec2(2.0, 0.0))])).is_empty());
        // Spread to twice the distance, around the same centre: zoom ×2.
        let out = g.send(vec![touch(1, TouchPhase::Move, pos2(350.0, 400.0)), touch(2, TouchPhase::Move, pos2(550.0, 400.0))]);
        let zoom: f32 = views(&out).iter().map(|m| m.scale).product();
        assert!((zoom - 2.0).abs() < 0.05, "{zoom}");
        // Turn a quarter turn clockwise about the centre.
        let out = g.send(vec![touch(1, TouchPhase::Move, pos2(450.0, 300.0)), touch(2, TouchPhase::Move, pos2(450.0, 500.0))]);
        let turned: f32 = views(&out).iter().map(|m| m.rotate).sum();
        assert!((turned - 90.0).abs() < 0.5, "{turned}");
        // Drag both: pan.
        let out = g.send(vec![touch(1, TouchPhase::Move, pos2(500.0, 300.0)), touch(2, TouchPhase::Move, pos2(500.0, 500.0))]);
        let panned: f32 = views(&out).iter().map(|m| (m.to - m.from).x).sum();
        assert!((panned - 50.0).abs() < 0.01, "{out:?}");
        let out = g.send(vec![touch(1, TouchPhase::End, pos2(500.0, 300.0)), touch(2, TouchPhase::End, pos2(500.0, 500.0))]);
        assert!(!out.iter().any(|r| matches!(r, Routed::Tap(_))), "a gesture isn't a tap");
        assert!(g.r.fingers.is_empty() && !g.r.wants_frames());
    }

    #[test]
    fn switched_off_gestures_do_nothing() {
        let mut g = Rig::with(TouchConfig { zoom: false, rotate: false, pan: false, ..Default::default() });
        g.send(vec![touch(1, TouchPhase::Start, pos2(400.0, 400.0)), touch(2, TouchPhase::Start, pos2(500.0, 400.0))]);
        let out = g.send(vec![touch(1, TouchPhase::Move, pos2(300.0, 300.0)), touch(2, TouchPhase::Move, pos2(600.0, 500.0))]);
        assert!(views(&out).is_empty(), "{out:?}");
    }

    #[test]
    fn quick_two_and_three_finger_taps() {
        let mut g = Rig::new();
        let (a, b, c) = (pos2(400.0, 400.0), pos2(450.0, 400.0), pos2(500.0, 400.0));
        g.send(vec![touch(1, TouchPhase::Start, a), touch(2, TouchPhase::Start, b)]);
        g.now += 0.1;
        let out = g.send(vec![touch(1, TouchPhase::End, a), touch(2, TouchPhase::End, b)]);
        assert!(out.contains(&Routed::Tap(2)) && presses(&out).is_empty(), "{out:?}");
        g.wait(1.0);
        g.send(vec![touch(3, TouchPhase::Start, a), touch(4, TouchPhase::Start, b)]);
        g.send(vec![touch(5, TouchPhase::Start, c)]);
        g.now += 0.1;
        let out = g.send(vec![touch(3, TouchPhase::End, a), touch(4, TouchPhase::End, b), touch(5, TouchPhase::End, c)]);
        assert!(out.contains(&Routed::Tap(3)), "{out:?}");
        // Too slow: not a tap.
        g.wait(1.0);
        g.send(vec![touch(6, TouchPhase::Start, a), touch(7, TouchPhase::Start, b)]);
        g.now += 0.8;
        let out = g.send(vec![touch(6, TouchPhase::End, a), touch(7, TouchPhase::End, b)]);
        assert!(!out.iter().any(|r| matches!(r, Routed::Tap(_))));
    }

    #[test]
    fn a_second_finger_after_the_stroke_began_ends_it() {
        let mut g = Rig::new();
        let a = pos2(400.0, 400.0);
        g.send(vec![touch(1, TouchPhase::Start, a)]);
        assert_eq!(presses(&g.send(vec![touch(1, TouchPhase::Move, a + vec2(40.0, 0.0))])), [true]);
        let out = g.send(vec![touch(2, TouchPhase::Start, pos2(600.0, 400.0))]);
        assert_eq!(presses(&out), [false], "the stroke ends; the fingers pinch from here");
    }

    #[test]
    fn finger_modes() {
        // Move the canvas: one finger pans, never paints.
        let mut g = Rig::with(TouchConfig { finger: FingerMode::Navigate, ..Default::default() });
        let a = pos2(400.0, 400.0);
        g.send(vec![touch(1, TouchPhase::Start, a)]);
        let out = g.send(vec![touch(1, TouchPhase::Move, a + vec2(60.0, 0.0))]);
        assert!(presses(&out).is_empty() && views(&out).iter().any(|m| m.to.x - m.from.x > 50.0), "{out:?}");
        assert!(presses(&g.wait(1.0)).is_empty());
        // Gestures only: one finger does nothing on the canvas...
        let mut g = Rig::with(TouchConfig { finger: FingerMode::Gestures, ..Default::default() });
        g.send(vec![touch(1, TouchPhase::Start, a)]);
        let out = g.send(vec![touch(1, TouchPhase::Move, a + vec2(60.0, 0.0))]);
        assert!(presses(&out).is_empty() && views(&out).is_empty());
        g.send(vec![touch(1, TouchPhase::End, a + vec2(60.0, 0.0))]);
        // ...but still clicks the interface.
        let ui = pos2(950.0, 900.0);
        g.send(vec![touch(2, TouchPhase::Start, ui)]);
        assert_eq!(presses(&g.send(vec![touch(2, TouchPhase::End, ui)])), [true, false]);
    }

    #[test]
    fn palms_are_ignored_while_the_pen_is_near_or_down() {
        let mut g = Rig::new();
        g.pen_near = true;
        let palm = pos2(400.0, 600.0);
        g.send(vec![touch(1, TouchPhase::Start, palm)]);
        let out = g.send(vec![touch(1, TouchPhase::Move, palm + vec2(50.0, 0.0))]);
        assert!(out.is_empty(), "{out:?}");
        assert!(presses(&g.wait(1.0)).is_empty());
        g.send(vec![touch(1, TouchPhase::End, palm)]);
        // A palm resting before the pen arrives lets go when the pen touches down.
        g.pen_near = false;
        g.wait(1.0);
        g.send(vec![touch(2, TouchPhase::Start, palm)]);
        assert_eq!(presses(&g.wait(HOLD_TIME + 0.01)), [true]);
        let out = g.send(vec![touch(100, TouchPhase::Start, pos2(300.0, 300.0))]);
        assert_eq!(presses(&out), [false, true], "the palm's press ends, the pen's begins: {out:?}");
        // Without palm rejection the finger keeps going (the pen waits its turn).
        let mut g = Rig::with(TouchConfig { palm_rejection: false, ..Default::default() });
        g.pen_near = true;
        g.send(vec![touch(1, TouchPhase::Start, palm)]);
        assert_eq!(presses(&g.wait(HOLD_TIME + 0.01)), [true]);
    }

    #[test]
    fn two_fingers_scroll_menus_and_panels() {
        let mut g = Rig::new();
        let (a, b) = (pos2(950.0, 860.0), pos2(990.0, 860.0));
        let out = g.send(vec![touch(1, TouchPhase::Start, a), touch(2, TouchPhase::Start, b)]);
        assert!(presses(&out).is_empty());
        let out = g.send(vec![touch(1, TouchPhase::Move, a + vec2(0.0, -40.0)), touch(2, TouchPhase::Move, b + vec2(0.0, -40.0))]);
        let wheel: Vec2 = app_events(&out)
            .into_iter()
            .filter_map(|e| match e {
                Event::MouseWheel { delta, unit: MouseWheelUnit::Point, .. } => Some(*delta),
                _ => None,
            })
            .fold(Vec2::ZERO, |s, d| s + d);
        assert!((wheel.y + 40.0).abs() < 0.01, "content follows the fingers: {wheel:?}");
        let out = g.send(vec![touch(1, TouchPhase::End, a), touch(2, TouchPhase::End, b)]);
        assert!(presses(&out).is_empty(), "nothing clicked");
    }

    #[test]
    fn a_hidden_pad_still_routes_and_cancel_lets_go() {
        let mut g = Rig::new();
        let p = pos2(950.0, 900.0);
        let out = g.route(vec![touch(1, TouchPhase::Start, p)], None, |_| false);
        assert!(app_events(&out).is_empty());
        g.now += HOLD_TIME + 0.01;
        g.route(vec![], None, |_| false);
        let out = g.route(vec![touch(1, TouchPhase::Cancel, p)], None, |_| false);
        assert!(matches!(app_events(&out).as_slice(), [Event::PointerButton { pressed: false, .. }, Event::PointerGone]));
        // Other events pass through in order.
        let out = g.route(vec![Event::Text("x".into())], None, |_| false);
        assert_eq!(out, vec![Routed::App(Event::Text("x".into()))]);
    }

    fn wheel(r: &[Routed]) -> Vec2 {
        app_events(r)
            .into_iter()
            .filter_map(|e| match e {
                Event::MouseWheel { delta, .. } => Some(*delta),
                _ => None,
            })
            .fold(Vec2::ZERO, |s, d| s + d)
    }

    /// One finger on a list row: a vertical swipe scrolls (and glides on), it never grabs.
    #[test]
    fn swiping_a_list_scrolls_instead_of_dragging_a_row() {
        let mut g = Rig::new();
        let p = pos2(950.0, 400.0);
        g.send(vec![touch(1, TouchPhase::Start, p)]);
        g.now += 0.05;
        let out = g.send(vec![touch(1, TouchPhase::Move, p + vec2(0.0, -60.0))]);
        assert!(presses(&out).is_empty(), "no press: {out:?}");
        assert!((wheel(&out).y + 60.0).abs() < 0.01, "the list follows the finger: {out:?}");
        g.now += 0.05;
        g.send(vec![touch(1, TouchPhase::Move, p + vec2(0.0, -120.0))]);
        let out = g.send(vec![touch(1, TouchPhase::End, p + vec2(0.0, -120.0))]);
        assert!(presses(&out).is_empty());
        // Lifted moving fast: it glides on, slowing, then stops and the pointer leaves.
        assert!(g.r.wants_frames());
        let first = wheel(&g.wait(0.016)).y;
        assert!(first < -5.0, "{first}");
        let mut later = 0.0;
        for _ in 0..200 {
            let out = g.wait(0.016);
            if app_events(&out).iter().any(|e| matches!(e, Event::PointerGone)) {
                break;
            }
            later = wheel(&out).y;
        }
        assert!(later > first && !g.r.wants_frames(), "slowed and stopped");
    }

    #[test]
    fn holding_a_list_row_grabs_it_and_sideways_drags_slide() {
        let mut g = Rig::new();
        let p = pos2(950.0, 400.0);
        g.send(vec![touch(1, TouchPhase::Start, p)]);
        assert!(presses(&g.wait(HOLD_TIME + 0.01)).is_empty(), "a list row waits longer");
        assert_eq!(presses(&g.wait(LONG_PRESS)), [true], "then it is grabbed");
        let out = g.send(vec![touch(1, TouchPhase::Move, p + vec2(0.0, 80.0))]);
        assert!(matches!(app_events(&out).as_slice(), [Event::PointerMoved(_)]), "and dragged to reorder: {out:?}");
        g.send(vec![touch(1, TouchPhase::End, p + vec2(0.0, 80.0))]);
        // A sideways drag (a slider) is pressed at once.
        g.wait(1.0);
        g.send(vec![touch(2, TouchPhase::Start, p)]);
        let out = g.send(vec![touch(2, TouchPhase::Move, p + vec2(30.0, 3.0))]);
        assert_eq!(presses(&out), [true]);
        g.send(vec![touch(2, TouchPhase::End, p + vec2(30.0, 3.0))]);
        // A tap still clicks the row.
        g.wait(1.0);
        g.send(vec![touch(3, TouchPhase::Start, p)]);
        assert_eq!(presses(&g.send(vec![touch(3, TouchPhase::End, p)])), [true, false]);
    }

    #[test]
    fn an_interface_finger_reaches_egui_as_a_touch_for_press_and_hold() {
        let mut g = Rig::new();
        let p = pos2(950.0, 700.0);
        assert_eq!(forwarded(&g.send(vec![touch(1, TouchPhase::Start, p)])), [TouchPhase::Start]);
        assert_eq!(forwarded(&g.send(vec![touch(1, TouchPhase::Move, p + vec2(2.0, 0.0))])), [TouchPhase::Move]);
        assert_eq!(forwarded(&g.send(vec![touch(1, TouchPhase::End, p)])), [TouchPhase::End]);
        // A second finger makes it a scroll: egui forgets the touch.
        g.wait(1.0);
        g.send(vec![touch(2, TouchPhase::Start, p)]);
        let out = g.send(vec![touch(3, TouchPhase::Start, p + vec2(30.0, 0.0))]);
        assert_eq!(forwarded(&out), [TouchPhase::Cancel]);
        g.send(vec![touch(2, TouchPhase::End, p), touch(3, TouchPhase::End, p)]);
        // Canvas fingers never do, nor any finger with the setting off.
        g.wait(1.0);
        assert!(forwarded(&g.send(vec![touch(4, TouchPhase::Start, CANVAS.center())])).is_empty());
        g.send(vec![touch(4, TouchPhase::End, CANVAS.center())]);
        let mut g = Rig::with(TouchConfig { long_press_right_click: false, ..Default::default() });
        assert!(forwarded(&g.send(vec![touch(1, TouchPhase::Start, p)])).is_empty());
    }

    #[test]
    fn a_held_finger_right_clicks_in_egui() {
        let ctx = egui::Context::default();
        let mut g = Rig::new();
        let p = pos2(950.0, 700.0);
        let mut right = false;
        let mut left = false;
        let frames: Vec<(f64, Vec<Event>)> = vec![
            (0.0, vec![]),
            (0.1, vec![touch(1, TouchPhase::Start, p)]),
            (0.4, vec![]),
            (0.8, vec![]),
            (1.2, vec![]),
            (1.3, vec![]),
            (1.4, vec![touch(1, TouchPhase::End, p)]),
            (1.5, vec![]),
        ];
        for (t, events) in frames {
            g.now = 10.0 + t;
            let events = app_events_owned(g.send(events));
            let raw = egui::RawInput { time: Some(t), screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1100.0, 800.0))), events, ..Default::default() };
            let mut out = ctx.run_ui(raw, |ui| {
                let r = ui.allocate_rect(Rect::from_center_size(p, vec2(60.0, 60.0)), egui::Sense::click());
                right |= r.secondary_clicked();
                left |= r.clicked();
            });
            out.textures_delta.clear();
        }
        assert!(right && !left, "held: a right-click, not a click");
    }

    fn app_events_owned(r: Vec<Routed>) -> Vec<Event> {
        r.into_iter()
            .filter_map(|r| match r {
                Routed::App(e) => Some(e),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_mouse_right_button_presses_the_secondary_button() {
        let mut g = Rig::new();
        let p = CANVAS.center();
        let out = g.send(vec![touch(200, TouchPhase::Start, p)]);
        assert!(app_events(&out).iter().any(|e| matches!(e, Event::PointerButton { button: PointerButton::Secondary, pressed: true, .. })));
        let out = g.send(vec![touch(200, TouchPhase::End, p)]);
        assert!(app_events(&out).iter().any(|e| matches!(e, Event::PointerButton { button: PointerButton::Secondary, pressed: false, .. })));
        // A pen without it is the primary button.
        let out = g.send(vec![touch(100, TouchPhase::Start, p)]);
        assert_eq!(presses(&out), [true]);
        assert!(app_events(&out).iter().any(|e| matches!(e, Event::PointerButton { button: PointerButton::Primary, .. })));
    }

    #[test]
    fn a_sideways_swipe_scrolls_a_strip_and_a_hold_still_grabs() {
        let mut g = Rig::new();
        let p = pos2(500.0, 25.0);
        g.send(vec![touch(1, TouchPhase::Start, p)]);
        assert!(presses(&g.wait(HOLD_TIME + 0.01)).is_empty(), "a strip waits as long as a list row");
        let out = g.send(vec![touch(1, TouchPhase::Move, p + vec2(-60.0, 4.0))]);
        assert!(presses(&out).is_empty(), "nothing pressed");
        let dx: f32 = out.iter().filter_map(|r| if let Routed::StripScroll(d) = r { Some(*d) } else { None }).sum();
        assert!(dx < -40.0 && wheel(&out) == Vec2::ZERO, "the strip itself, not the wheel: {dx} {out:?}");
        g.send(vec![touch(1, TouchPhase::End, p + vec2(-60.0, 4.0))]);
        // Held still first, a drag works the control under the finger.
        g.wait(2.0);
        g.send(vec![touch(2, TouchPhase::Start, p)]);
        assert_eq!(presses(&g.wait(LONG_PRESS + 0.01)), [true]);
        g.send(vec![touch(2, TouchPhase::End, p)]);
        // A tap still clicks.
        g.wait(1.0);
        g.send(vec![touch(3, TouchPhase::Start, p)]);
        assert_eq!(presses(&g.send(vec![touch(3, TouchPhase::End, p)])), [true, false]);
    }
}
