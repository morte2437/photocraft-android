//! The Shortcut Pad: a slim strip on the left or right edge of the window with 2 to 10 keys
//! (split above and below) and a wheel, like the ExpressKeys and Touch Ring of a pen tablet.
//!
//! - A key presses its shortcut while it is touched: `Ctrl+Z` undoes, a modifier on its own
//!   (`Alt`) is held, so the pen Alt-clicks while a finger rests on the key.
//! - Pressing the wheel opens a large ring (its size is a setting); sliding the finger around it
//!   zooms, rotates the view, sizes or hardens the brush (all smoothly), or steps through
//!   history, at the chosen sensitivity. Tapping the centre switches to the next of the chosen
//!   functions (without opening the ring).
//! - Each key can have a colour (Window › Shortcut Pad › Shortcut Pad Settings…), so keys are
//!   told apart by what they do at a glance.
//! - The key above the keyboard key (next), always there too, steps through full screen: first
//!   keeping the tool and options bars, then the canvas alone, then back (full screen has no
//!   menu bar, so without a keyboard there was no way back).
//! - The key at the very bottom, always there, opens the on-screen keyboard (and closes it
//!   again); its letters reach the app's shortcuts.
//! - Window › Shortcut Pad shows or hides it, moves it to either side and opens its settings
//!   (how many keys, each key's shortcut, the wheel's functions), which are saved in the app's
//!   files directory.
//!
//! Touch routing ([`router`]) keeps the pad's fingers away from the canvas, so a key can be held
//! while the pen draws.

pub mod config;
pub mod router;
pub mod trackpad;
pub mod wheel;

use config::{Chord, MAX_KEYS, MIN_KEYS, PadConfig, SENSITIVITY_RANGE, Side, WHEEL_SIZE_RANGE, WheelMode};
use egui::{Align2, Event, FontId, Key, Modifiers, Stroke};
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::menus::MenuItem;
use photocraft_ui_egui::platform_menu::PlatformMenu;
use photocraft_ui_egui::theme::Tokens;
use router::{Control, Env, Layout, PadTouch, Routed, Router, ViewMove};
use serde_json::Value;
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

/// The settings and whether their window is open: shared with the menu rows.
pub struct Shared {
    pub config: PadConfig,
    pub settings_open: bool,
    path: PathBuf,
}

impl Shared {
    fn save(&self) {
        if let Err(e) = self.config.save(&self.path) {
            log::error!("couldn't save the Shortcut Pad settings: {e}");
        }
    }
}

const MENU: [&str; 2] = ["Window", "Shortcut Pad"];
pub const SHOW: &str = "android.pad.show";
pub const LEFT: &str = "android.pad.left";
pub const RIGHT: &str = "android.pad.right";
pub const SETTINGS: &str = "android.pad.settings";

pub struct Pad {
    shared: Rc<RefCell<Shared>>,
    router: Router,
    /// Where the pad was drawn last frame (`None`: hidden).
    layout: Option<Layout>,
    /// The wheel function in use (an index into the configured list).
    mode: usize,
    turn: wheel::Turn,
    /// The ring is being turned.
    turning: bool,
    /// Keys being held, with the chord they pressed (released with the same one even if the
    /// settings change meanwhile).
    held: HashMap<usize, Chord>,
    /// Wheel changes waiting for the frame (they need the app and the egui context).
    pending: Vec<wheel::Action>,
    /// The large ring shown while the wheel is pressed.
    big: Option<BigRing>,
    /// Where the finger turning the wheel is.
    finger: Option<egui::Pos2>,
    /// What the wheel press started on, and where; whether the finger has slid since (a tap on
    /// the centre switches function, a slide turns).
    press: Option<(Control, egui::Pos2)>,
    slid: bool,
    /// The keyboard key was tapped (the shell opens or closes the on-screen keyboard).
    keyboard_tapped: bool,
    /// The shell has the on-screen keyboard open for the pad (the key is lit).
    pub keyboard_on: bool,
    /// The full screen key was tapped (the screen mode switches in `run_pending`).
    screen_tapped: bool,
    /// The screen mode last frame, for the full screen key's icon.
    screen_step: ScreenStep,
    /// The window's content area last frame (keeps the large ring on screen).
    screen: egui::Rect,
    /// The phone layout: a slimmer pad ([`router::COMPACT_WIDTH`]).
    pub compact: bool,
    /// Brush size and hardness as the turn has set them, unrounded, so slow turns add up.
    size_target: Option<f32>,
    hardness_target: Option<f32>,
    /// Canvas gestures to apply to the view (Touch Input).
    views: Vec<ViewMove>,
}

/// The large ring: where it is and how big.
#[derive(Clone, Copy, Debug, PartialEq)]
struct BigRing {
    center: egui::Pos2,
    radius: f32,
}

/// The large ring for a press on the wheel at `wheel`: as near the wheel as it fits on screen
/// (it slides in from the pad's edge onto the canvas).
fn big_ring(wheel: egui::Pos2, radius: f32, screen: egui::Rect, side: Side) -> BigRing {
    let m = radius + 8.0;
    let x = match side {
        Side::Right => wheel.x.min(screen.right() - m),
        Side::Left => wheel.x.max(screen.left() + m),
    };
    let y = if screen.height() > 2.0 * m { wheel.y.clamp(screen.top() + m, screen.bottom() - m) } else { screen.center().y };
    BigRing { center: egui::pos2(x, y), radius }
}

/// Brush sizes the engine accepts (`photocraft_paint::MAX_BRUSH_SIZE`).
const MAX_BRUSH_SIZE: f32 = 5000.0;

impl Pad {
    pub fn new(files_dir: &std::path::Path) -> Self {
        let path = files_dir.join("shortcut-pad.json");
        let config = PadConfig::load(&path);
        Self {
            shared: Rc::new(RefCell::new(Shared { config, settings_open: false, path })),
            router: Router::default(),
            layout: None,
            mode: 0,
            turn: wheel::Turn::default(),
            turning: false,
            held: HashMap::new(),
            pending: Vec::new(),
            big: None,
            finger: None,
            press: None,
            slid: false,
            keyboard_tapped: false,
            keyboard_on: false,
            screen_tapped: false,
            screen_step: ScreenStep::Normal,
            screen: egui::Rect::NOTHING,
            compact: false,
            size_target: None,
            hardness_target: None,
            views: Vec::new(),
        }
    }

    pub fn shared(&self) -> Rc<RefCell<Shared>> {
        self.shared.clone()
    }

    /// The Window › Shortcut Pad rows.
    pub fn menu(&self) -> PlatformMenu {
        let (read, write) = (self.shared.clone(), self.shared.clone());
        PlatformMenu {
            items: Box::new(move || {
                let s = read.borrow();
                let row = |id: &str, label: &str, checked: Option<bool>| MenuItem {
                    id: id.into(),
                    label: label.into(),
                    path: MENU.iter().map(|s| s.to_string()).collect(),
                    shortcut: None,
                    enabled: true,
                    checked,
                    color: None,
                };
                vec![
                    row(SHOW, "Show Shortcut Pad", Some(s.config.enabled)),
                    row(LEFT, "On the Left", Some(s.config.side == Side::Left)),
                    row(RIGHT, "On the Right", Some(s.config.side == Side::Right)),
                    row(SETTINGS, "Shortcut Pad Settings…", None),
                ]
            }),
            run: Box::new(move |id| {
                let mut s = write.borrow_mut();
                match id {
                    SHOW => s.config.enabled = !s.config.enabled,
                    LEFT | RIGHT => {
                        s.config.side = if id == LEFT { Side::Left } else { Side::Right };
                        s.config.enabled = true;
                    }
                    SETTINGS => {
                        s.settings_open = true;
                        return Ok(Value::Null);
                    }
                    _ => return Err(format!("unknown Shortcut Pad command {id}")),
                }
                s.save();
                Ok(Value::Null)
            }),
        }
    }

    fn mode(&self) -> WheelMode {
        let s = self.shared.borrow();
        let modes = &s.config.wheel_modes;
        modes.get(self.mode % modes.len().max(1)).copied().unwrap_or(WheelMode::Zoom)
    }

    /// Route a frame's input (from `raw_input_hook`, before modifiers are applied): pad touches
    /// become key presses and wheel commands, canvas gestures move the view and taps press their
    /// shortcut (Touch Input), the rest reaches the app.
    pub fn input(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput, env: &Env, covered: impl Fn(egui::Pos2) -> bool) {
        let shown = self.shared.borrow().config.enabled;
        let layout = if shown { self.layout.clone() } else { None };
        // A window or menu drawn over the pad last frame (the settings window, a menu dropping
        // down) keeps the touches there.
        let routed = self.router.route(std::mem::take(&mut raw.events), layout.as_ref(), covered, env);
        let mut out = Vec::with_capacity(routed.len());
        if layout.is_none() {
            self.release_all(&mut out);
        }
        for r in routed {
            match r {
                Routed::App(e) => out.push(e),
                Routed::Pad(t) => self.touch(t, &mut out),
                Routed::View(m) => self.views.push(m),
                Routed::StripScroll(dx) => photocraft_ui_egui::panels::scroll_options_bar(ctx, dx),
                Routed::Tap(fingers) => {
                    let shortcut = match fingers {
                        2 => &env.config.two_finger_tap,
                        3 => &env.config.three_finger_tap,
                        _ => continue,
                    };
                    match config::parse(shortcut) {
                        Ok(chord) if !chord.is_empty() => {
                            out.extend(press(chord));
                            out.extend(release(chord));
                        }
                        Ok(_) => {}
                        Err(e) => log::warn!("{fingers}-finger tap: {e}"),
                    }
                }
            }
        }
        if self.router.wants_frames() {
            ctx.request_repaint();
        }
        raw.events = out;
    }

    fn touch(&mut self, t: PadTouch, out: &mut Vec<Event>) {
        match t {
            PadTouch::Down(Control::Key(i), _) => {
                let shortcut = self.shared.borrow().config.keys.get(i).map(|k| k.shortcut.clone()).unwrap_or_default();
                match config::parse(&shortcut) {
                    Ok(chord) if !chord.is_empty() => {
                        out.extend(press(chord));
                        if let Some(old) = self.held.insert(i, chord) {
                            out.extend(release(old));
                        }
                    }
                    Ok(_) => {}
                    Err(e) => log::warn!("Shortcut Pad key {}: {e}", i + 1),
                }
            }
            PadTouch::Up(Control::Key(i)) => {
                if let Some(chord) = self.held.remove(&i) {
                    out.extend(release(chord));
                }
            }
            PadTouch::Down(Control::Keyboard, _) => self.keyboard_tapped = true,
            PadTouch::Down(Control::ScreenMode, _) => self.screen_tapped = true,
            // The centre is a button: a tap that doesn't slide switches function when lifted,
            // and it neither opens the large ring nor turns.
            PadTouch::Down(Control::Center, p) => {
                self.press = Some((Control::Center, p));
                self.slid = false;
            }
            PadTouch::Move(Control::Center, p) => {
                if self.press.is_some_and(|(_, start)| start.distance(p) > 12.0) {
                    self.slid = true;
                }
            }
            // The ring opens the large ring and turns.
            PadTouch::Down(Control::Ring, p) | PadTouch::Move(Control::Ring, p) => {
                let Some((center, small_r)) = self.layout.as_ref().map(|l| (l.center, l.ring_outer)) else { return };
                if let PadTouch::Down(control, _) = t {
                    self.press = Some((control, p));
                    self.slid = false;
                    self.turn = wheel::Turn::default();
                    self.turning = true;
                    self.size_target = None;
                    self.hardness_target = None;
                    let c = &self.shared.borrow().config;
                    self.big = c.enlarge.then(|| big_ring(center, c.wheel_size / 2.0, self.screen, c.side));
                }
                self.finger = Some(p);
                let (c, r) = self.big.map_or((center, small_r), |b| (b.center, b.radius));
                // Too near the centre for a steady angle: wait for the finger to come out.
                let angle = (p.distance(c) >= r * 0.2).then(|| wheel::angle(c, p));
                let (mode, sensitivity) = (self.mode(), self.shared.borrow().config.sensitivity);
                self.pending.extend(self.turn.moved(mode, angle, sensitivity));
            }
            PadTouch::Up(Control::Ring | Control::Center) => {
                if matches!(self.press, Some((Control::Center, _))) && !self.slid {
                    let n = self.shared.borrow().config.wheel_modes.len().max(1);
                    self.mode = (self.mode + 1) % n;
                }
                self.press = None;
                self.turning = false;
                self.big = None;
                self.finger = None;
            }
            _ => {}
        }
    }

    fn release_all(&mut self, out: &mut Vec<Event>) {
        for (_, chord) in self.held.drain() {
            out.extend(release(chord));
        }
        self.turning = false;
        self.big = None;
        self.finger = None;
    }

    /// Apply the wheel's changes (from `logic`, before the app's). A frame's movements are
    /// merged: one zoom, one rotation, one brush change, however many touch events came in.
    pub fn run_pending(&mut self, app: &mut PhotocraftApp, ctx: &egui::Context) {
        if self.take_screen_tap() {
            // Normal → full screen keeping the tool and options bars → full screen → normal.
            let next = ScreenStep::of(app).next();
            let mode = if next == ScreenStep::Normal { "view.screenMode.standard" } else { "view.screenMode.fullScreen" };
            if let Err(e) = photocraft_ui_egui::menus::invoke(app, ctx, mode, serde_json::json!({})) {
                log::warn!("Shortcut Pad full screen key: {e}");
            }
            app.full_screen_toolbar = next == ScreenStep::WithToolbar;
            ctx.request_repaint();
        }
        let views = std::mem::take(&mut self.views);
        if !views.is_empty() {
            crate::touch::apply(app, &views);
            ctx.request_repaint();
        }
        let actions = std::mem::take(&mut self.pending);
        if actions.is_empty() {
            return;
        }
        let (mut zoom, mut rotate, mut size, mut hardness) = (1.0f32, 0.0f32, 1.0f32, 0.0f32);
        let run = |app: &mut PhotocraftApp, id: &str, params: Value| {
            if let Err(e) = photocraft_ui_egui::menus::invoke(app, ctx, id, params) {
                // No document to zoom, nothing to redo…: not worth an error for a wheel turn.
                log::debug!("Shortcut Pad wheel: {id}: {e}");
            }
        };
        for a in actions {
            match a {
                wheel::Action::Zoom(f) => zoom *= f,
                wheel::Action::Rotate(d) => rotate += d,
                wheel::Action::BrushSize(f) => size *= f,
                wheel::Action::Hardness(d) => hardness += d,
                wheel::Action::Command(id) => run(app, id, serde_json::json!({})),
            }
        }
        if zoom != 1.0 {
            app.sync_views();
            if let Some(v) = app.session.active_index().and_then(|i| app.ui.views.get_mut(i)) {
                v.fit_pending = false;
                v.fill_pending = false;
                v.zoom = (v.zoom * zoom).clamp(0.01, 32.0);
            }
        }
        if rotate != 0.0 {
            run(app, "view.rotateView", serde_json::json!({ "delta": rotate }));
        }
        if size != 1.0 {
            let target = (self.size_target.unwrap_or(app.session.tools.brush.size) * size).clamp(1.0, MAX_BRUSH_SIZE);
            self.size_target = Some(target);
            if target.round() != app.session.tools.brush.size {
                run(app, "tools.setBrush", serde_json::json!({ "brush": { "size": target.round() } }));
            }
        }
        if hardness != 0.0 {
            let target = (self.hardness_target.unwrap_or(app.session.tools.brush.hardness) + hardness).clamp(0.0, 1.0);
            self.hardness_target = Some(target);
            let rounded = (target * 100.0).round() / 100.0;
            if rounded != app.session.tools.brush.hardness {
                run(app, "tools.setBrush", serde_json::json!({ "brush": { "hardness": rounded } }));
            }
        }
        ctx.request_repaint();
    }

    /// What the large ring shows for the current function.
    fn readout(app: &PhotocraftApp, mode: WheelMode) -> String {
        let view = app.session.active_index().and_then(|i| app.ui.views.get(i));
        let brush = &app.session.tools.brush;
        match mode {
            WheelMode::Zoom => view.map(|v| format!("{:.0}%", v.zoom * 100.0)).unwrap_or_default(),
            WheelMode::Rotate => view.map(|v| format!("{:.0}°", v.rotation)).unwrap_or_default(),
            WheelMode::BrushSize => format!("{:.0} px", brush.size),
            WheelMode::BrushHardness => format!("{:.0}%", brush.hardness * 100.0),
            WheelMode::History => "Undo ◀ ▶ Redo".into(),
        }
    }

    fn paint_big(&self, p: &egui::Painter, b: BigRing, mode: WheelMode, readout: &str, tk: &Tokens) {
        let (c, r) = (b.center, b.radius);
        let band = r * 0.32;
        let mid = r - band / 2.0;
        p.circle_filled(c, r + 2.0, tk.shadow);
        p.circle_filled(c, r, tk.card);
        p.circle_stroke(c, mid, Stroke::new(band, tk.field));
        p.circle_stroke(c, r, Stroke::new(2.0, tk.accent));
        for k in 0..24 {
            let a = (k as f32 * wheel::TICK).to_radians();
            p.circle_filled(c + egui::vec2(a.cos(), a.sin()) * mid, 1.6, tk.text_faint);
        }
        if let Some(f) = self.finger
            && f.distance(c) > 1.0
        {
            let dir = (f - c).normalized();
            p.circle_filled(c + dir * mid, band * 0.38, tk.accent);
        }
        p.text(c - egui::vec2(0.0, 10.0), Align2::CENTER_CENTER, mode.short(), FontId::proportional(15.0), tk.text_dim);
        p.text(c + egui::vec2(0.0, 12.0), Align2::CENTER_CENTER, readout, FontId::proportional(18.0), tk.text);
    }

    /// Draw the pad (before the app, which then lays out in the rest of the window).
    pub fn show(&mut self, ui: &mut egui::Ui, tk: &Tokens, app: &PhotocraftApp) {
        self.screen = ui.ctx().content_rect();
        self.screen_step = ScreenStep::of(app);
        let config = self.shared.borrow().config.clone();
        if !config.enabled {
            self.layout = None;
            return;
        }
        let panel = match config.side {
            Side::Left => egui::Panel::left("shortcut_pad"),
            Side::Right => egui::Panel::right("shortcut_pad"),
        };
        let frame = egui::Frame::new().fill(tk.chrome).stroke(Stroke::new(1.0, tk.separator));
        let mode = self.mode();
        let layout = panel
            .exact_size(if self.compact { router::COMPACT_WIDTH } else { router::WIDTH })
            .resizable(false)
            .frame(frame)
            .show(ui, |ui| {
                // The screen's rounded corners: the keys at the strip's ends move clear of them.
                let full = ui.max_rect();
                let span = photocraft_ui_egui::cutout::column_span(ui.ctx(), full.x_range());
                let top = full.top().max(span.min);
                let full = egui::Rect::from_x_y_ranges(full.x_range(), top..=full.bottom().min(span.max).max(top));
                let l = Layout::new(full, config.keys.len());
                self.paint(ui.painter(), &l, &config, mode, tk);
                l
            })
            .inner;
        self.layout = Some(layout);
        if let Some(b) = self.big {
            let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("shortcut_pad_ring")));
            self.paint_big(&painter, b, mode, &Self::readout(app, mode), tk);
        }
    }

    fn paint(&self, p: &egui::Painter, l: &Layout, config: &PadConfig, mode: WheelMode, tk: &Tokens) {
        // Labels a size smaller on keys shrunk to fit a short screen.
        let font = FontId::proportional(if l.key_height() < 36.0 { 11.0 } else { 12.0 });
        for (i, rect) in l.keys.iter().enumerate() {
            let down = self.held.contains_key(&i);
            let key_color = config.keys.get(i).and_then(|k| k.color).map(|[r, g, b]| egui::Color32::from_rgb(r, g, b));
            let (fill, color) = match key_color {
                // A coloured key lightens while held; its label is dark or light, whichever reads.
                Some(c) => {
                    let fill = if down { c.lerp_to_gamma(egui::Color32::WHITE, 0.35) } else { c };
                    (fill, contrasting_text(fill))
                }
                None if down => (tk.accent, tk.primary_text),
                None => (tk.field, tk.text),
            };
            p.rect(*rect, tk.radius, fill, Stroke::new(if down { 2.0 } else { 1.0 }, tk.field_border), egui::StrokeKind::Inside);
            let label = config.keys.get(i).map(|k| k.label.as_str()).unwrap_or("");
            let label = label.to_string();
            // The largest size that fits the key's width (typed labels can be wide), clipped
            // to the key at the smallest.
            let room = rect.width() - 6.0;
            let galley = [font.size, font.size - 1.0, font.size - 2.0, 9.0]
                .into_iter()
                .map(|size| p.layout_no_wrap(label.clone(), FontId::proportional(size), color))
                .find(|g| g.size().x <= room)
                .unwrap_or_else(|| p.layout_no_wrap(label.clone(), FontId::proportional(9.0), color));
            p.with_clip_rect(rect.shrink(2.0)).galley(rect.center() - galley.size() / 2.0, galley, color);
        }
        // The ring, with click-stop marks, and the centre button naming the function.
        let ring = if self.turning { tk.accent } else { tk.field_border };
        p.circle(l.center, l.ring_outer, tk.field, Stroke::new(1.5, ring));
        let mid = (l.ring_outer + l.ring_inner) / 2.0;
        for k in 0..24 {
            let a = (k as f32 * wheel::TICK).to_radians();
            p.circle_filled(l.center + egui::vec2(a.cos(), a.sin()) * mid, 1.2, tk.text_faint);
        }
        p.circle(l.center, l.ring_inner, tk.card, Stroke::new(1.0, tk.field_border));
        let n = config.wheel_modes.len();
        let current = self.mode % n.max(1);
        // One light per function, the current one lit (like a tablet's ring LEDs).
        for k in 0..n {
            let a = (-90.0 + (k as f32 - (n as f32 - 1.0) / 2.0) * 16.0f32).to_radians();
            let at = l.center + egui::vec2(a.cos(), a.sin()) * (l.ring_outer - 5.0);
            p.circle_filled(at, 2.2, if k == current { tk.accent } else { tk.text_faint });
        }
        p.text(l.center, Align2::CENTER_CENTER, mode.short(), FontId::proportional(9.0), tk.text_dim);
        self.paint_keyboard_key(p, l.keyboard, tk);
        self.paint_screen_key(p, l.screen_mode, tk);
    }

    /// The full screen key, drawn like a video player's, showing what a tap does: four corners
    /// pointing out to go (further) full screen, pointing in to come back. Lit in full screen;
    /// with the tool bar kept, a bar along its left edge.
    fn paint_screen_key(&self, p: &egui::Painter, rect: egui::Rect, tk: &Tokens) {
        let step = self.screen_step;
        let lit = step != ScreenStep::Normal;
        p.rect(rect, tk.radius, if lit { tk.accent } else { tk.field }, Stroke::new(1.0, tk.field_border), egui::StrokeKind::Inside);
        let ink = Stroke::new(2.0, if lit { tk.primary_text } else { tk.text });
        let on = step.next() == ScreenStep::Normal;
        if step == ScreenStep::WithToolbar {
            let x = rect.left() + 5.0;
            p.line_segment([egui::pos2(x, rect.top() + 6.0), egui::pos2(x, rect.bottom() - 6.0)], Stroke::new(3.0, ink.color));
        }
        let half = (rect.height() * 0.28).min(rect.width() * 0.3);
        let arm = half * 0.55;
        let c = rect.center();
        for (sx, sy) in [(-1.0f32, -1.0f32), (1.0, -1.0), (-1.0, 1.0), (1.0, 1.0)] {
            // The corner's point, and its arms going back along both edges.
            let (corner, back) = if on {
                // Inward: the corner sits nearer the middle, its arms reaching out.
                (c + egui::vec2(sx, sy) * (half - arm), egui::vec2(sx, sy) * arm)
            } else {
                (c + egui::vec2(sx, sy) * half, -egui::vec2(sx, sy) * arm)
            };
            p.line_segment([corner, corner + egui::vec2(back.x, 0.0)], ink);
            p.line_segment([corner, corner + egui::vec2(0.0, back.y)], ink);
        }
    }

    /// The keyboard key: a small keyboard drawn in it (keys and a space bar), lit while open.
    fn paint_keyboard_key(&self, p: &egui::Painter, rect: egui::Rect, tk: &Tokens) {
        let on = self.keyboard_on;
        p.rect(rect, tk.radius, if on { tk.accent } else { tk.field }, Stroke::new(1.0, tk.field_border), egui::StrokeKind::Inside);
        let ink = if on { tk.primary_text } else { tk.text };
        let body = egui::Rect::from_center_size(rect.center(), egui::vec2(30.0, 18.0).min(rect.size() - egui::vec2(6.0, 6.0)));
        p.rect_stroke(body, 2.0, Stroke::new(1.2, ink), egui::StrokeKind::Inside);
        let (cols, key) = (5, body.width() / 7.0);
        for row in 0..2 {
            for col in 0..cols {
                let x = body.left() + key * (1.0 + col as f32 * 1.0) + key / 2.0;
                let y = body.top() + body.height() * (0.3 + row as f32 * 0.22);
                p.circle_filled(egui::pos2(x, y), 1.2, ink);
            }
        }
        let bar = body.bottom() - body.height() * 0.24;
        p.line_segment([egui::pos2(body.center().x - key * 1.5, bar), egui::pos2(body.center().x + key * 1.5, bar)], Stroke::new(1.5, ink));
    }

    /// The settings window, when open (after the app, so it sits on top).
    pub fn settings(&mut self, ctx: &egui::Context, tk: &Tokens) {
        let mut open = self.shared.borrow().settings_open;
        if !open {
            return;
        }
        let before = self.shared.borrow().config.clone();
        let mut c = before.clone();
        egui::Window::new("Shortcut Pad")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                ui.checkbox(&mut c.enabled, "Show the Shortcut Pad");
                ui.horizontal(|ui| {
                    ui.label("Side:");
                    ui.radio_value(&mut c.side, Side::Left, "Left");
                    ui.radio_value(&mut c.side, Side::Right, "Right");
                });
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Number of keys:");
                    let mut n = c.keys.len();
                    if ui.add(egui::Slider::new(&mut n, MIN_KEYS..=MAX_KEYS)).changed() {
                        c.set_key_count(n);
                    }
                    ui.label(format!("{} above the wheel, {} below", n.div_ceil(2), n / 2));
                });
                ui.label("Keys (top to bottom). A modifier on its own, such as Alt, is held while the key is touched.");
                let above = c.keys.len().div_ceil(2);
                egui::ScrollArea::vertical().max_height(320.0).show(ui, |ui| {
                    egui::Grid::new("pad_keys").num_columns(5).spacing([8.0, 6.0]).show(ui, |ui| {
                        ui.label("");
                        ui.label("Label");
                        ui.label("Color");
                        ui.label("Preset");
                        ui.label("Shortcut");
                        ui.end_row();
                        for (i, key) in c.keys.iter_mut().enumerate() {
                            ui.label(format!("Key {}", i + 1)).on_hover_text(if i < above { "Above the wheel" } else { "Below the wheel" });
                            ui.add_sized([84.0, 24.0], egui::TextEdit::singleline(&mut key.label).char_limit(8));
                            color_choice(ui, tk, i, &mut key.color);
                            egui::ComboBox::from_id_salt(("pad_preset", i)).selected_text("Choose…").width(110.0).show_ui(ui, |ui| {
                                for (name, label, shortcut) in config::PRESETS {
                                    if ui.selectable_label(key.shortcut == *shortcut, format!("{name}  ({shortcut})")).clicked() {
                                        key.label = (*label).into();
                                        key.shortcut = (*shortcut).into();
                                    }
                                }
                                if ui.selectable_label(key.shortcut.is_empty(), "Nothing").clicked() {
                                    key.label.clear();
                                    key.shortcut.clear();
                                }
                            });
                            ui.vertical(|ui| {
                                ui.add_sized([130.0, 24.0], egui::TextEdit::singleline(&mut key.shortcut).hint_text("e.g. Ctrl+Z"));
                                if let Err(e) = config::parse(&key.shortcut) {
                                    ui.colored_label(tk.danger, e);
                                }
                            });
                            ui.end_row();
                        }
                    });
                });
                ui.separator();
                ui.label("Wheel: tapping its centre switches between");
                for m in WheelMode::ALL {
                    let mut on = c.wheel_modes.contains(&m);
                    let last = on && c.wheel_modes.len() == 1;
                    if ui.add_enabled(!last, egui::Checkbox::new(&mut on, m.label())).changed() {
                        if on {
                            // Keep the configured order: the order of ALL.
                            c.wheel_modes.push(m);
                            c.wheel_modes.sort_by_key(|x| WheelMode::ALL.iter().position(|a| a == x));
                        } else {
                            c.wheel_modes.retain(|x| *x != m);
                        }
                    }
                }
                ui.checkbox(&mut c.enlarge, "Open a large ring when the wheel is pressed");
                ui.add_enabled(c.enlarge, egui::Slider::new(&mut c.wheel_size, WHEEL_SIZE_RANGE).text("Ring size").suffix(" pt").integer());
                ui.add(egui::Slider::new(&mut c.sensitivity, SENSITIVITY_RANGE).logarithmic(true).text("Sensitivity").suffix("×"))
                    .on_hover_text("At 1×, a quarter turn doubles the zoom, a third of a turn doubles the brush size");
                ui.separator();
                if ui.button("Restore Defaults").clicked() {
                    c = PadConfig::default();
                }
            });
        let mut s = self.shared.borrow_mut();
        s.settings_open = open;
        if c != before {
            s.config = c.sanitized();
            s.save();
        }
    }

    /// Whether the full screen key was tapped since the last call.
    pub fn take_screen_tap(&mut self) -> bool {
        std::mem::take(&mut self.screen_tapped)
    }

    /// The trackpad cursor (Touch Input), over everything: an arrow pointer where it is.
    pub fn paint_trackpad_cursor(&self, ctx: &egui::Context, on: bool) {
        let Some(c) = self.router.trackpad_cursor().filter(|_| on) else { return };
        let p = ctx.layer_painter(egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("trackpad_cursor")));
        // A rounded arrow, its tip on the point: white with a dark outline and a soft shadow.
        let outline = arrow_outline();
        let at = |off: egui::Vec2| outline.iter().map(|q| c + q.to_vec2() + off).collect::<Vec<_>>();
        let shadow = egui::vec2(1.0, 1.6);
        for (width, alpha) in [(5.0, 18), (3.0, 30)] {
            p.add(egui::Shape::closed_line(at(shadow), Stroke::new(width, egui::Color32::from_black_alpha(alpha))));
        }
        p.add(arrow_fill(&at(shadow), c + shadow, egui::Color32::from_black_alpha(60)));
        p.add(arrow_fill(&at(egui::Vec2::ZERO), c, egui::Color32::WHITE));
        p.add(egui::Shape::closed_line(at(egui::Vec2::ZERO), Stroke::new(2.0, egui::Color32::from_gray(0x38))));
        // Holding the button (a drag): a ring at the tip, so a drag never comes as a surprise.
        if self.router.trackpad_pressed() {
            p.circle_stroke(c, 9.0, Stroke::new(3.0, egui::Color32::BLACK));
            p.circle_stroke(c, 9.0, Stroke::new(1.8, egui::Color32::from_rgb(0x3d, 0x9b, 0xff)));
        }
    }

    /// Whether the keyboard key was tapped since the last call.
    pub fn take_keyboard_tap(&mut self) -> bool {
        std::mem::take(&mut self.keyboard_tapped)
    }

    /// Whether the pad needs frames: a finger is on it.
    pub fn active(&self) -> bool {
        self.router.pad_active()
    }
}

/// The trackpad cursor's corners (points from its tip) and how round each one is.
const ARROW: [(f32, f32, f32); 4] = [(0.0, 0.0, 1.6), (0.0, 20.5, 1.6), (7.6, 14.2, 2.4), (16.0, 14.2, 1.4)];
/// A point every edge of the arrow can be seen from, to fill it as a fan (it isn't convex).
const ARROW_CENTRE: egui::Pos2 = egui::pos2(3.0, 12.0);

/// The trackpad cursor's outline, its corners rounded, relative to its tip.
fn arrow_outline() -> Vec<egui::Pos2> {
    let n = ARROW.len();
    let corner = |i: usize| egui::pos2(ARROW[i % n].0, ARROW[i % n].1);
    let mut out = Vec::new();
    for (i, &(_, _, r)) in ARROW.iter().enumerate() {
        let (prev, here, next) = (corner(i + n - 1), corner(i), corner(i + 1));
        let a = here + (prev - here).normalized() * r;
        let b = here + (next - here).normalized() * r;
        for s in 0..=5 {
            let t = s as f32 / 5.0;
            // A quadratic curve from a to b, bending towards the corner.
            let q = a.to_vec2() * (1.0 - t) * (1.0 - t) + here.to_vec2() * 2.0 * t * (1.0 - t) + b.to_vec2() * t * t;
            out.push(q.to_pos2());
        }
    }
    out
}

/// `outline` (the arrow with its tip at `tip`) filled with `color`.
fn arrow_fill(outline: &[egui::Pos2], tip: egui::Pos2, color: egui::Color32) -> egui::Shape {
    let mut mesh = egui::Mesh::default();
    mesh.colored_vertex(tip + ARROW_CENTRE.to_vec2(), color);
    for &q in outline {
        mesh.colored_vertex(q, color);
    }
    let n = outline.len() as u32;
    for i in 0..n {
        mesh.add_triangle(0, 1 + i, 1 + (i + 1) % n);
    }
    egui::Shape::mesh(mesh)
}

fn key_event(key: Key, pressed: bool) -> Event {
    Event::Key { key, physical_key: Some(key), pressed, repeat: false, modifiers: Modifiers::NONE }
}

/// Where the full screen key is in its cycle.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ScreenStep {
    Normal,
    /// Full screen with the tool and options bars kept (the first step).
    WithToolbar,
    Full,
}

impl ScreenStep {
    fn of(app: &PhotocraftApp) -> Self {
        match app.ui.view.screen_mode.as_str() {
            "standard" => ScreenStep::Normal,
            "fullScreen" if app.full_screen_toolbar => ScreenStep::WithToolbar,
            _ => ScreenStep::Full,
        }
    }

    /// What a tap goes to.
    fn next(self) -> Self {
        match self {
            ScreenStep::Normal => ScreenStep::WithToolbar,
            ScreenStep::WithToolbar => ScreenStep::Full,
            ScreenStep::Full => ScreenStep::Normal,
        }
    }
}

/// Dark or light text, whichever reads on `fill`.
fn contrasting_text(fill: egui::Color32) -> egui::Color32 {
    let luma = 0.299 * fill.r() as f32 + 0.587 * fill.g() as f32 + 0.114 * fill.b() as f32;
    if luma > 150.0 { egui::Color32::from_rgb(20, 20, 20) } else { egui::Color32::WHITE }
}

/// A swatch for painting a colour choice (the theme's key colour when `None`).
fn swatch(ui: &mut egui::Ui, tk: &Tokens, color: Option<[u8; 3]>) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
    let fill = color.map_or(tk.field, |[r, g, b]| egui::Color32::from_rgb(r, g, b));
    ui.painter().rect(rect, 3.0, fill, Stroke::new(1.0, tk.field_border), egui::StrokeKind::Inside);
}

/// A key's colour: the theme's, one of [`config::KEY_COLORS`], or any other picked.
fn color_choice(ui: &mut egui::Ui, tk: &Tokens, i: usize, color: &mut Option<[u8; 3]>) {
    ui.horizontal(|ui| {
        let name = match color {
            None => "Default",
            Some(c) => config::KEY_COLORS.iter().find(|(_, k)| k == c).map_or("Custom", |(n, _)| *n),
        };
        egui::ComboBox::from_id_salt(("pad_color", i)).selected_text(name).width(84.0).show_ui(ui, |ui| {
            let mut row = |ui: &mut egui::Ui, label: &str, value: Option<[u8; 3]>| {
                let r = ui.horizontal(|ui| {
                    swatch(ui, tk, value);
                    ui.selectable_label(*color == value, label)
                });
                if r.inner.clicked() {
                    *color = value;
                }
            };
            row(ui, "Default", None);
            for (n, c) in config::KEY_COLORS {
                row(ui, n, Some(*c));
            }
        });
        // Any other colour: the swatch opens a picker.
        let mut rgb = color.unwrap_or([128, 128, 128]);
        if egui::widgets::color_picker::color_edit_button_srgb(ui, &mut rgb).changed() {
            *color = Some(rgb);
        }
    });
}

fn modifier_keys(c: Chord) -> Vec<Key> {
    [(c.ctrl, Key::ControlLeft), (c.alt, Key::AltLeft), (c.shift, Key::ShiftLeft)].into_iter().filter(|(on, _)| *on).map(|(_, k)| k).collect()
}

/// The key events of pressing a chord: its modifiers, then its key.
pub fn press(c: Chord) -> Vec<Event> {
    let mut v: Vec<Event> = modifier_keys(c).into_iter().map(|k| key_event(k, true)).collect();
    v.extend(c.key.map(|k| key_event(k, true)));
    v
}

/// The key events of releasing a chord: its key, then its modifiers.
pub fn release(c: Chord) -> Vec<Event> {
    let mut v: Vec<Event> = c.key.map(|k| key_event(k, false)).into_iter().collect();
    v.extend(modifier_keys(c).into_iter().rev().map(|k| key_event(k, false)));
    v
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, TouchDeviceId, TouchId, TouchPhase, pos2, vec2};

    /// The arrow fills as a fan from its centre: every triangle must turn the same way, or the
    /// fill would spill outside the notch.
    #[test]
    fn the_trackpad_cursor_fills_inside_its_outline() {
        let outline = arrow_outline();
        assert!(outline.iter().all(|q| q.x >= 0.0 && q.y >= 0.0 && q.x <= 16.0 && q.y <= 20.5));
        let o = ARROW_CENTRE;
        for (a, b) in outline.iter().zip(outline.iter().cycle().skip(1)) {
            let cross = (*a - o).x * (*b - o).y - (*a - o).y * (*b - o).x;
            assert!(cross <= 1e-4, "{a:?} → {b:?} turns the wrong way ({cross})");
        }
    }

    fn pad() -> (Pad, PathBuf) {
        let dir = std::env::temp_dir().join(format!("photocraft-padmod-{}-{}", std::process::id(), rand()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut p = Pad::new(&dir);
        p.layout = Some(Layout::new(Rect::from_min_size(pos2(1000.0, 0.0), vec2(router::WIDTH, 800.0)), 4));
        (p, dir)
    }

    fn rand() -> u64 {
        use std::sync::atomic::{AtomicU64, Ordering};
        static N: AtomicU64 = AtomicU64::new(0);
        N.fetch_add(1, Ordering::Relaxed)
    }

    fn touch(id: u64, phase: TouchPhase, pos: egui::Pos2) -> Event {
        Event::Touch { device_id: TouchDeviceId(1), id: TouchId(id), phase, pos, force: None }
    }

    fn run(p: &mut Pad, events: Vec<Event>) -> Vec<Event> {
        let mut raw = egui::RawInput { events, ..Default::default() };
        let config = crate::touch::config::TouchConfig::default();
        let env = Env {
            canvas: None,
            is_finger: &|_| true,
            pen_near: false,
            config: &config,
            list_item: &|_| false,
            draggable: &|_| false,
            secondary: &|_| false,
            scroll_strip: None,
            screen: egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1280.0, 800.0)),
            now: 0.0,
        };
        p.input(&egui::Context::default(), &mut raw, &env, |_| false);
        raw.events
    }

    fn keys(events: &[Event]) -> Vec<(Key, bool)> {
        events
            .iter()
            .filter_map(|e| match e {
                Event::Key { key, pressed, .. } => Some((*key, *pressed)),
                _ => None,
            })
            .collect()
    }

    #[test]
    fn a_key_presses_its_shortcut_and_releases_it() {
        let (mut p, dir) = pad();
        let k0 = p.layout.clone().unwrap().keys[0].center();
        // Default key 1: Undo (Ctrl+Z).
        let down = run(&mut p, vec![touch(1, TouchPhase::Start, k0)]);
        assert_eq!(keys(&down), [(Key::ControlLeft, true), (Key::Z, true)]);
        assert!(!down.iter().any(|e| matches!(e, Event::Touch { .. } | Event::PointerButton { .. })), "the finger doesn't reach the canvas");
        let up = run(&mut p, vec![touch(1, TouchPhase::End, k0)]);
        assert_eq!(keys(&up), [(Key::Z, false), (Key::ControlLeft, false)]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn a_modifier_key_is_held_while_touched() {
        let (mut p, dir) = pad();
        let k2 = p.layout.clone().unwrap().keys[2].center(); // default: Alt
        assert_eq!(keys(&run(&mut p, vec![touch(1, TouchPhase::Start, k2)])), [(Key::AltLeft, true)]);
        assert!(keys(&run(&mut p, vec![touch(1, TouchPhase::Move, k2)])).is_empty(), "held, not repeated");
        // Hiding the pad releases it (no stuck Alt).
        p.shared.borrow_mut().config.enabled = false;
        assert_eq!(keys(&run(&mut p, vec![])), [(Key::AltLeft, false)]);
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_wheel_queues_changes_and_the_centre_switches_function() {
        let (mut p, dir) = pad();
        p.screen = Rect::from_min_size(pos2(0.0, 0.0), vec2(1072.0, 800.0));
        let l = p.layout.clone().unwrap();
        let at = |deg: f32| l.center + vec2(deg.to_radians().cos(), deg.to_radians().sin()) * (l.ring_outer - 4.0);
        run(&mut p, vec![touch(1, TouchPhase::Start, at(90.0))]);
        let big = p.big.expect("pressing the wheel opens the large ring");
        assert!(big.center.x + big.radius <= p.screen.right(), "on screen: {big:?}");
        let on_big = |deg: f32| big.center + vec2(deg.to_radians().cos(), deg.to_radians().sin()) * big.radius * 0.8;
        run(&mut p, vec![touch(1, TouchPhase::Move, on_big(0.0)), touch(1, TouchPhase::Move, on_big(45.0))]);
        assert!(p.pending.iter().any(|a| matches!(a, wheel::Action::Zoom(f) if *f > 1.0)), "clockwise zooms in: {:?}", p.pending);
        run(&mut p, vec![touch(1, TouchPhase::End, on_big(45.0))]);
        assert!(p.big.is_none(), "lifting closes it");
        p.pending.clear();
        // Tap the centre: Rotate View, and no large ring.
        run(&mut p, vec![touch(2, TouchPhase::Start, l.center)]);
        assert!(p.big.is_none() && !p.turning, "the centre button doesn't open the large ring");
        run(&mut p, vec![touch(2, TouchPhase::End, l.center)]);
        assert_eq!(p.mode(), WheelMode::Rotate);
        assert!(p.pending.is_empty());
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn the_full_screen_key_keeps_the_bars_first_then_hides_everything() {
        use ScreenStep::*;
        assert_eq!((Normal.next(), WithToolbar.next(), Full.next()), (WithToolbar, Full, Normal));
    }

    #[test]
    fn the_keyboard_key_asks_for_the_keyboard() {
        let (mut p, dir) = pad();
        let at = p.layout.clone().unwrap().keyboard.center();
        assert!(!p.take_keyboard_tap());
        let out = run(&mut p, vec![touch(1, TouchPhase::Start, at), touch(1, TouchPhase::End, at)]);
        assert!(keys(&out).is_empty() && !out.iter().any(|e| matches!(e, Event::Touch { .. })));
        assert!(p.take_keyboard_tap());
        assert!(!p.take_keyboard_tap(), "once per tap");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn menu_rows_toggle_move_and_persist() {
        let (p, dir) = pad();
        let mut menu = p.menu();
        let checked = |m: &PlatformMenu, id: &str| (m.items)().into_iter().find(|i| i.id == id).and_then(|i| i.checked);
        assert_eq!(checked(&menu, SHOW), Some(true));
        assert_eq!(checked(&menu, RIGHT), Some(true));
        (menu.run)(SHOW).unwrap();
        assert_eq!(checked(&menu, SHOW), Some(false));
        (menu.run)(LEFT).unwrap();
        assert_eq!((checked(&menu, LEFT), checked(&menu, SHOW)), (Some(true), Some(true)), "choosing a side shows it");
        (menu.run)(SETTINGS).unwrap();
        assert!(p.shared.borrow().settings_open);
        // Saved: a new pad (next launch) has them.
        let again = Pad::new(&dir);
        assert_eq!(again.shared.borrow().config.side, Side::Left);
        assert!((menu.run)("android.pad.bogus").is_err());
        std::fs::remove_dir_all(dir).ok();
    }

    /// Ten keys: the sixth (the first below the wheel) presses its own shortcut.
    #[test]
    fn a_ten_key_pad_routes_every_key() {
        let (mut p, dir) = pad();
        p.shared.borrow_mut().config.set_key_count(10);
        let count = p.shared.borrow().config.keys.len();
        p.layout = Some(Layout::new(Rect::from_min_size(pos2(1000.0, 0.0), vec2(router::WIDTH, 800.0)), count));
        let l = p.layout.clone().unwrap();
        assert!(l.keys[5].top() > l.center.y, "key 6 is the first below the wheel");
        let shortcut = p.shared.borrow().config.keys[5].shortcut.clone();
        let chord = config::parse(&shortcut).unwrap();
        let down = run(&mut p, vec![touch(1, TouchPhase::Start, l.keys[5].center())]);
        assert_eq!(keys(&down), keys(&press(chord)), "{shortcut}");
        std::fs::remove_dir_all(dir).ok();
    }

    #[test]
    fn press_and_release_mirror_each_other() {
        let c = config::parse("Ctrl+Shift+Alt+S").unwrap();
        let down: Vec<(Key, bool)> = keys(&press(c));
        let up: Vec<(Key, bool)> = keys(&release(c));
        assert_eq!(down.iter().map(|k| k.0).collect::<Vec<_>>(), up.iter().rev().map(|k| k.0).collect::<Vec<_>>());
    }
}
