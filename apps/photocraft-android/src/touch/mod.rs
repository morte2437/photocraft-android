//! Touch Input: multi-touch gestures on the canvas and fingers in the interface.
//!
//! - **On the canvas:** two fingers pan, pinch to zoom and twist to rotate the view, all at once
//!   (the image stays under the fingers); a quick two-finger tap undoes and a three-finger tap
//!   redoes (any shortcut can be chosen). One finger draws like the pen, moves the canvas, or
//!   does nothing.
//! - **Palm rejection:** fingers are ignored while the pen is down or near the screen.
//! - **Menus, panels and lists:** a flick keeps scrolling, and two fingers scroll. A finger held
//!   still right-clicks.
//! - **Pen buttons:** the lower, middle and top side buttons and the eraser end each erase,
//!   right-click (a side button pressed while hovering clicks under the pen), run a shortcut or
//!   do nothing. Each has a light in the settings that turns green while it's in use.
//! - **Window › Touch Input** switches what one finger does and palm rejection, and opens Touch
//!   Input Settings… for the rest. Settings are saved in the app's files directory.
//!
//! The gestures themselves are recognised by the touch router (`pad::router`), which already
//! owns every touch to keep the Shortcut Pad's fingers off the canvas; this module holds the
//! settings and applies view changes to the active document.

pub mod config;

use crate::pad::config::{self as pad_config, PRESETS};
use crate::pad::router::ViewMove;
use config::{FingerMode, PenAction, PenControl, TouchConfig};
use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::canvas::ViewXform;
use photocraft_ui_egui::menus::MenuItem;
use photocraft_ui_egui::platform_menu::PlatformMenu;
use photocraft_ui_egui::theme::Tokens;
use serde_json::Value;
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

const MENU: &[&str] = &["Window", "Touch Input"];
const PALM: &str = "android.touch.palmRejection";
const SETTINGS: &str = "android.touch.settings";
const FINGER: &str = "android.touch.finger.";
const TRACKPAD: &str = "android.touch.trackpad";

/// The zoom range of the canvas (as the Shortcut Pad's wheel allows).
const ZOOM_RANGE: std::ops::RangeInclusive<f32> = 0.01..=32.0;

pub struct Shared {
    pub config: TouchConfig,
    pub settings_open: bool,
    path: PathBuf,
}

impl Shared {
    fn save(&self) {
        if let Err(e) = self.config.save(&self.path) {
            log::warn!("couldn't save the touch input settings: {e}");
        }
    }
}

pub struct TouchInput {
    shared: Rc<RefCell<Shared>>,
}

impl TouchInput {
    pub fn new(files_dir: &std::path::Path) -> Self {
        let path = files_dir.join("touch-input.json");
        let config = TouchConfig::load(&path);
        Self { shared: Rc::new(RefCell::new(Shared { config, settings_open: false, path })) }
    }

    pub fn config(&self) -> TouchConfig {
        self.shared.borrow().config.clone()
    }

    /// The Window › Touch Input rows.
    pub fn menu(&self) -> PlatformMenu {
        let (read, write) = (self.shared.clone(), self.shared.clone());
        PlatformMenu {
            items: Box::new(move || {
                let s = read.borrow();
                let row = |id: String, label: &str, checked: Option<bool>| MenuItem {
                    id,
                    label: label.into(),
                    path: MENU.iter().map(|s| s.to_string()).collect(),
                    shortcut: None,
                    enabled: true,
                    checked,
                    color: None,
                };
                let mut rows: Vec<MenuItem> =
                    FingerMode::ALL.iter().map(|m| row(format!("{FINGER}{m:?}"), m.menu_label(), Some(s.config.finger == *m))).collect();
                rows.push(row(TRACKPAD.into(), "Trackpad Cursor", Some(s.config.trackpad)));
                rows.push(row(PALM.into(), "Palm Rejection", Some(s.config.palm_rejection)));
                rows.push(row(SETTINGS.into(), "Touch Input Settings…", None));
                rows
            }),
            run: Box::new(move |id| {
                let mut s = write.borrow_mut();
                if let Some(mode) = id.strip_prefix(FINGER) {
                    s.config.finger = FingerMode::ALL.into_iter().find(|m| format!("{m:?}") == mode).ok_or_else(|| format!("unknown finger mode {mode}"))?;
                } else if id == TRACKPAD {
                    s.config.trackpad = !s.config.trackpad;
                } else if id == PALM {
                    s.config.palm_rejection = !s.config.palm_rejection;
                } else if id == SETTINGS {
                    s.settings_open = true;
                    return Ok(Value::Null);
                } else {
                    return Err(format!("unknown Touch Input command {id}"));
                }
                s.save();
                Ok(Value::Null)
            }),
        }
    }

    /// The settings window, when open (after the app, so it sits on top).
    /// `held`: the pen controls in use now (by [`PenControl::index`]), for their lights;
    /// `buttons`: the pen's button state as Android reports it.
    pub fn settings(&mut self, ctx: &egui::Context, tk: &Tokens, held: [bool; 4], buttons: i32, contact: Option<Contact>) {
        let mut open = self.shared.borrow().settings_open;
        if !open {
            return;
        }
        // The lights follow the pen, which doesn't wake the UI by itself while it hovers.
        ctx.request_repaint_after(std::time::Duration::from_millis(50));
        let before = self.shared.borrow().config.clone();
        let mut c = before.clone();
        egui::Window::new("Touch Input")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .pivot(egui::Align2::CENTER_CENTER)
            .default_pos(ctx.content_rect().center())
            .show(ctx, |ui| {
                // Scrolls on a short screen (a phone on its side).
                egui::ScrollArea::vertical().max_height(ctx.content_rect().height() - 80.0).show(ui, |ui| {
                    ui.strong("One finger on the canvas");
                    for m in FingerMode::ALL {
                        ui.radio_value(&mut c.finger, m, m.label());
                    }
                    ui.separator();
                    ui.strong("Trackpad cursor");
                    ui.checkbox(&mut c.trackpad, "Fingers move a cursor, like a laptop trackpad, so they never cover what you point at");
                    if c.trackpad {
                        ui.label(
                            egui::RichText::new(
                                "Slide to move the cursor · tap to click · hold still to right-click · tap, then touch and drag, or rest one finger and move another, to paint or drag · two fingers together zoom the canvas or scroll a panel",
                            )
                            .color(tk.text_dim)
                            .small(),
                        );
                        ui.horizontal(|ui| {
                            ui.label("Speed");
                            ui.add(egui::Slider::new(&mut c.trackpad_speed, config::TRACKPAD_SPEED).fixed_decimals(1).suffix("×"));
                        });
                        ui.checkbox(&mut c.trackpad_acceleration, "Acceleration: quick swipes move the cursor further");
                        ui.checkbox(&mut c.trackpad_rest_to_drag, "Rest one finger and move another to hold the button (paint, drag)");
                        ui.checkbox(&mut c.trackpad_two_finger_right_click, "Two-finger tap right-clicks (instead of its shortcut)");
                    }
                    ui.separator();
                    finger_pressure(ui, tk, &mut c, contact);
                    ui.separator();
                    ui.strong("Two fingers on the canvas");
                    ui.checkbox(&mut c.pan, "Drag to move the canvas");
                    ui.checkbox(&mut c.zoom, "Pinch to zoom");
                    ui.checkbox(&mut c.rotate, "Twist to rotate the view");
                    ui.separator();
                    ui.strong("Taps on the canvas");
                    egui::Grid::new("touch_taps").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
                        tap_row(ui, tk, "Two-finger tap", "touch_two_tap", &mut c.two_finger_tap);
                        tap_row(ui, tk, "Three-finger tap", "touch_three_tap", &mut c.three_finger_tap);
                    });
                    ui.separator();
                    ui.checkbox(&mut c.palm_rejection, "Palm rejection: ignore fingers while the pen is on or near the screen");
                    ui.checkbox(&mut c.two_finger_scroll, "Two fingers scroll menus, panels and lists");
                    ui.checkbox(&mut c.long_press_right_click, "Hold a finger still to right-click (context menus, closing panels)");
                    ui.separator();
                    ui.strong("Pen");
                    ui.checkbox(&mut c.low_latency_ink, "Low latency ink (experimental): the line appears under the pen sooner");
                    ui.label(
                        egui::RichText::new("While you draw with the Brush or Pencil, a plain line in the brush's colour is drawn straight to the screen, then the real stroke replaces it.")
                            .color(tk.text_dim),
                    );
                    ui.separator();
                    ui.strong("Pen buttons");
                    ui.label(
                        egui::RichText::new("Button 1 is nearest the tip. Press a button or turn the pen over: its light turns green.").color(tk.text_dim),
                    );
                    ui.label(egui::RichText::new(format!("Android reports the buttons as {buttons:#04x}")).color(tk.text_faint).small());
                    egui::Grid::new("touch_pen_controls").num_columns(3).spacing([8.0, 6.0]).show(ui, |ui| {
                        for control in PenControl::ALL {
                            let setting = &mut c.pen[control.index()];
                            ui.horizontal(|ui| {
                                pen_light(ui, tk, held[control.index()]);
                                ui.label(config::pen_control_label(control));
                            });
                            let salt = format!("touch_pen_{control:?}");
                            egui::ComboBox::from_id_salt(&salt).selected_text(setting.action.label()).width(150.0).show_ui(ui, |ui| {
                                for a in PenAction::ALL {
                                    ui.selectable_value(&mut setting.action, a, a.label());
                                }
                            });
                            ui.label("");
                            ui.end_row();
                            if setting.action == PenAction::Shortcut {
                                tap_row(ui, tk, "    Shortcut", &format!("{salt}_shortcut"), &mut setting.shortcut);
                            }
                        }
                    });
                    ui.separator();
                    if ui.button("Restore Defaults").clicked() {
                        c = TouchConfig::default();
                    }
                });
            });
        let mut s = self.shared.borrow_mut();
        s.settings_open = open;
        if c != before {
            s.config = c.sanitized();
            s.save();
        }
    }
}

/// What the screen last reported for a finger (from the pen hook): Android's pressure, size and
/// contact length, the size finger pressure goes by, and the pressure it gave.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Contact {
    pub pressure: f32,
    pub size: f32,
    pub major: f32,
    pub contact: f32,
    pub mapped: f32,
}

/// Finger pressure from the contact size (experimental): the switch, what the screen reports for
/// a finger right now, and a box to calibrate on (touch it lightly, then press firmly).
fn finger_pressure(ui: &mut egui::Ui, tk: &Tokens, c: &mut TouchConfig, contact: Option<Contact>) {
    ui.strong("Finger pressure (experimental)");
    ui.checkbox(&mut c.finger_pressure, "Press harder to paint heavier, judged from how much of your finger touches the screen");
    let reading = contact.map_or("Touch the box below to see what your screen reports.".to_string(), |k| {
        format!("Last touch: pressure {:.2} · size {:.3} · contact {:.0} px → {:.0}%", k.pressure, k.size, k.major, k.mapped * 100.0)
    });
    ui.label(egui::RichText::new(reading).color(tk.text_dim).small());
    let width = ui.available_width().min(420.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(width, 90.0), egui::Sense::click_and_drag());
    let pressing = resp.is_pointer_button_down_on();
    let p = ui.painter();
    p.rect(rect, 8.0, if pressing { tk.accent.gamma_multiply(0.25) } else { tk.field }, egui::Stroke::new(1.0, tk.separator), egui::StrokeKind::Inside);
    let calibrated = c.finger_firm > c.finger_light;
    let text = if calibrated {
        format!("Light {:.0} · firm {:.0}
Touch lightly, then press firmly, to calibrate again", c.finger_light, c.finger_firm)
    } else {
        "Calibrate: touch here lightly, then press firmly".to_string()
    };
    p.text(rect.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(13.0), tk.text);
    // While a finger is on the box: the smallest and largest contact are the light and firm ends.
    let id = resp.id.with("range");
    if pressing && let Some(k) = contact {
        let size = k.contact;
        let (lo, hi) = ui.ctx().data(|m| m.get_temp::<(f32, f32)>(id)).unwrap_or((f32::MAX, 0.0));
        let range = (lo.min(size), hi.max(size));
        ui.ctx().data_mut(|m| m.insert_temp(id, range));
        if range.1 > range.0 {
            (c.finger_light, c.finger_firm) = range;
        }
        ui.ctx().request_repaint();
    } else if resp.drag_started() || resp.clicked() {
        ui.ctx().data_mut(|m| m.remove_temp::<(f32, f32)>(id));
    }
    if resp.drag_stopped() || resp.clicked() {
        ui.ctx().data_mut(|m| m.remove_temp::<(f32, f32)>(id));
    }
    if contact.is_some_and(|k| k.major <= 0.0 && k.size <= 0.0) {
        ui.colored_label(tk.danger, "This screen doesn't report how big a touch is, so finger pressure can't work here.");
    }
}

/// A pen control's light: green while it's in use, so it's clear which button is which.
fn pen_light(ui: &mut egui::Ui, tk: &Tokens, on: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(12.0, 12.0), egui::Sense::hover());
    let fill = if on { egui::Color32::from_rgb(52, 199, 89) } else { tk.text_faint.gamma_multiply(0.35) };
    ui.painter().circle_filled(rect.center(), 5.0, fill);
    ui.painter().circle_stroke(rect.center(), 5.0, egui::Stroke::new(1.0, tk.text_faint));
}

/// A tap's row: its name, a preset menu and the shortcut (typed or chosen).
fn tap_row(ui: &mut egui::Ui, tk: &Tokens, name: &str, salt: &str, shortcut: &mut String) {
    ui.label(name);
    let current =
        PRESETS.iter().find(|(_, _, s)| s.eq_ignore_ascii_case(shortcut)).map_or(if shortcut.is_empty() { "Nothing" } else { "Custom" }, |(n, _, _)| *n);
    egui::ComboBox::from_id_salt(salt).selected_text(current).width(150.0).show_ui(ui, |ui| {
        if ui.selectable_label(shortcut.is_empty(), "Nothing").clicked() {
            shortcut.clear();
        }
        for (menu_name, _, s) in PRESETS {
            if ui.selectable_label(shortcut.eq_ignore_ascii_case(s), format!("{menu_name}  ({s})")).clicked() {
                *shortcut = (*s).into();
            }
        }
    });
    ui.vertical(|ui| {
        ui.add_sized([130.0, 24.0], egui::TextEdit::singleline(shortcut).hint_text("e.g. Ctrl+Z"));
        if let Err(e) = pad_config::parse(shortcut) {
            ui.colored_label(tk.danger, e);
        }
    });
    ui.end_row();
}

/// The tallest widget treated as a list row (taller ones, such as a curves graph, keep their
/// own vertical drags).
const LIST_ROW_MAX_HEIGHT: f32 = 120.0;

/// One widget as laid out in a frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Widget {
    pub rect: egui::Rect,
    pub interact: egui::Rect,
    pub click: bool,
    pub drag: bool,
    pub enabled: bool,
}

/// Where everything was drawn last frame, layer by layer (back to front), for the touch router
/// to decide what a new touch landed on. Taken at the end of the frame's UI ([`Self::take`]):
/// between frames, where the router runs, egui's own lookups (`layer_id_at`, the widget rects)
/// can miss popups such as the brush picker, which would let a touch on the picker count as one
/// on the canvas behind it.
#[derive(Clone, Debug, Default)]
pub struct Snapshot {
    layers: Vec<(egui::Order, Vec<Widget>)>,
}

impl Snapshot {
    pub fn take(ctx: &egui::Context) -> Self {
        let order: Vec<egui::LayerId> = ctx.memory(|m| m.layer_ids().collect());
        ctx.viewport(|vp| {
            let w = &vp.this_pass.widgets;
            let widgets = |l: egui::LayerId| -> Vec<Widget> {
                w.get_layer(l)
                    .map(|w| Widget { rect: w.rect, interact: w.interact_rect, click: w.sense.senses_click(), drag: w.sense.senses_drag(), enabled: w.enabled })
                    .collect()
            };
            let mut layers: Vec<(egui::Order, Vec<Widget>)> = order.iter().map(|l| (l.order, widgets(*l))).collect();
            // Layers new this frame aren't in the area order yet: on top, by their order.
            let mut extra: Vec<egui::LayerId> = w.layer_ids().filter(|l| !order.contains(l)).collect();
            extra.sort_by_key(|l| l.order);
            layers.extend(extra.into_iter().map(|l| (l.order, widgets(l))));
            layers.retain(|(_, w)| !w.is_empty());
            Self { layers }
        })
    }

    #[cfg(test)]
    pub fn from_layers(layers: Vec<(egui::Order, Vec<Widget>)>) -> Self {
        Self { layers }
    }

    /// The topmost layer with something drawn at `p`.
    fn top(&self, p: egui::Pos2) -> Option<&(egui::Order, Vec<Widget>)> {
        self.layers.iter().rev().find(|(_, w)| w.iter().any(|w| w.rect.contains(p)))
    }

    /// A point on row `y`, from the right end of `xs` leftwards, where a wheel scrolls the list
    /// under it instead of changing a control: on something drawn (a label, a scroll area's
    /// background) but on nothing that can be clicked. For scrolling a field the keyboard covers
    /// back into view.
    pub fn quiet_point(&self, xs: std::ops::RangeInclusive<f32>, y: f32) -> Option<egui::Pos2> {
        let mut x = *xs.end();
        while x >= *xs.start() {
            let p = egui::pos2(x, y);
            if let Some((_, widgets)) = self.top(p) {
                // A window's or a modal's own background senses clicks too; a control is small.
                let small = |r: egui::Rect| r.height() <= LIST_ROW_MAX_HEIGHT && r.width() <= 600.0;
                let control = widgets.iter().any(|w| w.enabled && w.click && small(w.rect) && w.interact.contains(p));
                if !control {
                    return Some(p);
                }
            }
            x -= 8.0;
        }
        None
    }

    /// Whether a window, menu or popup was drawn over `p`: touches there belong to it, not to the
    /// canvas or the Shortcut Pad.
    /// What's drawn over the app's own layer (menus, windows, popups): touches there aren't on
    /// the canvas.
    pub fn covering(&self) -> Vec<egui::Rect> {
        self.layers.iter().filter(|(order, _)| *order != egui::Order::Background).flat_map(|(_, w)| w.iter().map(|w| w.rect)).collect()
    }

    pub fn covered(&self, p: egui::Pos2) -> bool {
        self.top(p).is_some_and(|(order, _)| *order != egui::Order::Background)
    }

    /// Whether what's under `p` can be dragged (a slider, a list row, a scroll area, a panel's
    /// edge) rather than only clicked (a button, a tool): the smallest control there, the one a
    /// fingertip is really on, senses drags.
    pub fn draggable(&self, p: egui::Pos2) -> bool {
        let Some((_, widgets)) = self.top(p) else { return false };
        widgets.iter().filter(|w| w.enabled && w.interact.contains(p) && (w.click || w.drag)).min_by(|a, b| a.interact.area().total_cmp(&b.interact.area())).is_some_and(|w| w.drag)
    }

    /// Whether `p` is on a list row inside a scrolling list: a widget that can be both clicked
    /// and dragged (a Brushes or Layers row that reorders, a preset card, a slider) on top of a
    /// larger drag-only area containing it (a scroll area's content). A finger swiping up or down
    /// there scrolls the list instead of dragging the row.
    pub fn list_item(&self, p: egui::Pos2) -> bool {
        let Some((_, widgets)) = self.top(p) else { return false };
        let hits: Vec<&Widget> = widgets.iter().filter(|w| w.enabled && w.interact.contains(p)).collect();
        let Some(row) = hits.iter().rev().find(|w| w.click && w.drag) else { return false };
        if row.rect.height() > LIST_ROW_MAX_HEIGHT {
            return false;
        }
        // A scroll area's drag-only content area under the row, or (a scroll area that registers
        // none, like the brush picker's) content overflowing a box that shows part of it.
        let all: Vec<&Widget> = widgets.iter().filter(|w| w.rect.contains(p)).collect();
        let dragged_area = hits.iter().any(|w| w.drag && !w.click && w.rect.height() > row.rect.height() * 1.5);
        // Scrolled content overlaps the box showing it and runs past it: neither holds the other
        // (a parent box holds its children; it does not scroll them).
        let overflowing = all.iter().any(|content| {
            content.rect.height() > row.rect.height() * 1.5
                && all.iter().any(|view| {
                    view.rect.height() > row.rect.height()
                        && view.rect.height() < content.rect.height()
                        && !view.rect.contains_rect(content.rect)
                        && !content.rect.contains_rect(view.rect)
                })
        });
        dragged_area || overflowing
    }
}

/// The view after a gesture: `m` applied to `xf` (the active canvas mapping). Returns the new
/// zoom factor relative to now, rotation and centre, keeping the document point under `m.from`
/// under `m.to`.
pub fn moved_view(xf: &ViewXform, zoom: f32, m: &ViewMove) -> (f32, f32, [f32; 2]) {
    let doc = xf.to_doc(m.from);
    let scale = if m.scale.is_finite() && m.scale > 0.0 { m.scale } else { 1.0 };
    let new_zoom = (zoom * scale).clamp(*ZOOM_RANGE.start(), *ZOOM_RANGE.end());
    // A flipped view mirrors the screen: a clockwise twist turns the camera the other way.
    let turn = if m.rotate.is_finite() { m.rotate } else { 0.0 };
    let turn = if xf.flip { -turn } else { turn };
    let rotation = (xf.rotation + turn + 540.0).rem_euclid(360.0) - 180.0;
    let moved = ViewXform { zoom: xf.zoom * (new_zoom / zoom), rotation, center: [0.0, 0.0], ..*xf };
    let off = moved.to_doc(m.to);
    let center = [(doc[0] - off[0]) as f32, (doc[1] - off[1]) as f32];
    (new_zoom, rotation, center)
}

/// Apply gesture moves to the active document's view, or to the Liquify preview while it's
/// open (it pans and zooms; it doesn't rotate).
pub fn apply(app: &mut PhotocraftApp, moves: &[ViewMove]) {
    if moves.is_empty() {
        return;
    }
    if photocraft_ui_egui::liquify_ui::preview_rect(app).is_some() {
        for m in moves {
            photocraft_ui_egui::liquify_ui::move_preview(app, m.from, m.to, m.scale);
        }
        return;
    }
    app.sync_views();
    for m in moves {
        let Some(i) = app.session.active_index() else { return };
        let Some(xf) = ViewXform::active(app) else { return };
        let Some(v) = app.ui.views.get_mut(i) else { return };
        let (zoom, rotation, center) = moved_view(&xf, v.zoom, m);
        if !(zoom.is_finite() && rotation.is_finite() && center.iter().all(|c| c.is_finite())) {
            return;
        }
        v.fit_pending = false;
        v.fill_pending = false;
        v.zoom = zoom;
        v.rotation = rotation;
        v.center = center;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Rect, pos2, vec2};

    fn xf(rotation: f32, flip: bool) -> ViewXform {
        ViewXform { rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0)), zoom: 1.0, center: [400.0, 300.0], flip, rotation, aspect: 1.0 }
    }

    /// The document point under the fingers stays under them, whatever the gesture.
    #[test]
    fn the_image_stays_under_the_fingers() {
        for (rot, flip) in [(0.0, false), (30.0, false), (-120.0, true)] {
            let x = xf(rot, flip);
            let m = ViewMove { from: pos2(200.0, 150.0), to: pos2(260.0, 190.0), scale: 1.7, rotate: 25.0 };
            let doc = x.to_doc(m.from);
            let (zoom, rotation, center) = moved_view(&x, 1.0, &m);
            let after = ViewXform { zoom: x.zoom * zoom, rotation, center, ..x };
            let p = after.to_screen(doc[0] as f32, doc[1] as f32);
            assert!(p.distance(m.to) < 0.01, "{rot} {flip}: {p:?}");
            assert!((zoom - 1.7).abs() < 1e-4);
        }
    }

    #[test]
    fn twist_turns_the_view_the_way_the_fingers_turn() {
        let m = ViewMove { from: pos2(400.0, 300.0), to: pos2(400.0, 300.0), scale: 1.0, rotate: 30.0 };
        assert_eq!(moved_view(&xf(0.0, false), 1.0, &m).1, 30.0);
        assert_eq!(moved_view(&xf(0.0, true), 1.0, &m).1, -30.0, "mirrored by View › Flip Horizontal");
        assert!((moved_view(&xf(170.0, false), 1.0, &m).1 + 160.0).abs() < 1e-3, "wrapped to (−180, 180]");
    }

    #[test]
    fn hostile_values_stay_in_range() {
        let m = ViewMove { from: pos2(1.0, 1.0), to: pos2(1.0, 1.0), scale: f32::NAN, rotate: f32::INFINITY };
        let (zoom, rotation, _) = moved_view(&xf(0.0, false), 1.0, &m);
        assert_eq!((zoom, rotation), (1.0, 0.0));
        let m = ViewMove { scale: 1e9, ..m };
        assert_eq!(moved_view(&xf(0.0, false), 1.0, &m).0, 32.0);
    }

    /// #20: while Liquify is open, two fingers move its preview, not the canvas behind it.
    #[test]
    fn gestures_move_the_liquify_preview_while_it_is_open() {
        use photocraft_ui_egui::liquify_ui;
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", serde_json::json!({"width": 120, "height": 80})).unwrap();
        app.run("layer.new.layer", serde_json::json!({})).unwrap();
        app.sync_views();
        let canvas = app.ui.views.first().map(|v| (v.zoom, v.center));
        liquify_ui::open(&mut app, &ctx).unwrap();
        let raw = egui::RawInput { screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1000.0, 700.0))), ..Default::default() };
        let mut out = ctx.run_ui(raw, |ui| liquify_ui::show(&mut app, ui.ctx()));
        out.textures_delta.clear();
        let area = liquify_ui::preview_rect(&app).expect("the preview was drawn");
        let zoom = |app: &PhotocraftApp| liquify_ui::describe(app).and_then(|d| d["zoom"].as_f64()).unwrap_or_default();
        let before = zoom(&app);
        let at = area.center();
        apply(&mut app, &[ViewMove { from: at, to: at + vec2(30.0, 10.0), scale: 2.0, rotate: 15.0 }]);
        assert!((zoom(&app) - before * 2.0).abs() < 1e-4, "{before} → {}", zoom(&app));
        assert_eq!(app.ui.views.first().map(|v| (v.zoom, v.center)), canvas, "the canvas behind keeps its view");
        liquify_ui::cancel(&mut app);
        assert!(liquify_ui::preview_rect(&app).is_none());
    }

    fn widget(rect: Rect, click: bool, drag: bool) -> Widget {
        Widget { rect, interact: rect, click, drag, enabled: true }
    }

    /// The brush picker, a popup over the canvas: touches on it are the picker's, and its preset
    /// cards (draggable, inside the scrolling list) are list rows; the canvas itself is neither.
    #[test]
    fn popups_over_the_canvas_and_their_list_rows_are_found() {
        let canvas = Rect::from_min_max(pos2(40.0, 68.0), pos2(883.0, 776.0));
        let list = Rect::from_min_max(pos2(140.0, 250.0), pos2(430.0, 470.0));
        let card = Rect::from_min_max(pos2(140.0, 400.0), pos2(430.0, 470.0));
        let slider = Rect::from_min_max(pos2(140.0, 100.0), pos2(430.0, 118.0));
        let s = Snapshot::from_layers(vec![
            (egui::Order::Background, vec![widget(Rect::from_min_size(pos2(0.0, 0.0), vec2(1280.0, 800.0)), false, false), widget(canvas, true, true)]),
            (egui::Order::Foreground, vec![widget(slider, true, true), widget(list, false, true), widget(card, true, true)]),
        ]);
        assert!(s.covered(card.center()) && s.list_item(card.center()));
        assert!(s.covered(slider.center()) && !s.list_item(slider.center()), "a slider outside a list drags");
        let open_canvas = pos2(700.0, 600.0);
        assert!(!s.covered(open_canvas) && !s.list_item(open_canvas), "the canvas is too big to be a row");
        assert!(!Snapshot::default().covered(open_canvas));
        // A list whose scroll area registers no drag area: its content overflows the popup.
        let frame = Rect::from_min_max(pos2(130.0, 66.0), pos2(444.0, 506.0));
        let content = Rect::from_min_max(pos2(137.0, 244.0), pos2(431.0, 2972.0));
        let s =
            Snapshot::from_layers(vec![(egui::Order::Foreground, vec![widget(frame, true, false), widget(content, false, false), widget(card, true, true)])]);
        assert!(s.list_item(card.center()));
        // The same row in a box that doesn't overflow (an ordinary panel) drags as usual.
        let panel = Rect::from_min_max(pos2(130.0, 66.0), pos2(444.0, 506.0));
        let group = Rect::from_min_max(pos2(135.0, 300.0), pos2(440.0, 500.0));
        let s =
            Snapshot::from_layers(vec![(egui::Order::Background, vec![widget(panel, false, false), widget(group, false, false), widget(card, true, true)])]);
        assert!(!s.list_item(card.center()));
    }

    /// After a tap, the trackpad cursor drags only from something that drags: a button in a
    /// scrolling panel is the button's, though the panel's scroll area drags.
    #[test]
    fn buttons_dont_drag_but_sliders_and_rows_do() {
        let panel = Rect::from_min_max(pos2(0.0, 0.0), pos2(300.0, 800.0));
        let tool = Rect::from_min_max(pos2(10.0, 10.0), pos2(40.0, 40.0));
        let slider = Rect::from_min_max(pos2(10.0, 100.0), pos2(280.0, 118.0));
        let s = Snapshot::from_layers(vec![(egui::Order::Background, vec![widget(panel, false, true), widget(tool, true, false), widget(slider, true, true)])]);
        assert!(!s.draggable(tool.center()), "a tool button");
        assert!(s.draggable(slider.center()));
        assert!(s.draggable(pos2(150.0, 500.0)), "the panel's empty space scrolls");
        assert!(!s.draggable(pos2(900.0, 500.0)), "nothing there");
    }

    #[test]
    fn the_menu_switches_modes_and_palm_rejection() {
        let dir = std::env::temp_dir().join(format!("pc-touch-menu-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = TouchInput::new(&dir);
        let mut m = t.menu();
        let rows = (m.items)();
        assert!(rows.iter().all(|r| r.path == ["Window", "Touch Input"]));
        assert_eq!(rows.iter().filter(|r| r.checked == Some(true)).count(), 2, "Finger Draws and Palm Rejection");
        (m.run)("android.touch.finger.Navigate").unwrap();
        (m.run)(PALM).unwrap();
        assert_eq!(t.config().finger, FingerMode::Navigate);
        assert!(!t.config().palm_rejection);
        assert_eq!(TouchConfig::load(&dir.join("touch-input.json")).finger, FingerMode::Navigate, "saved");
        (m.run)(SETTINGS).unwrap();
        assert!(t.shared.borrow().settings_open);
        assert!((m.run)("android.touch.finger.Nope").is_err());
        std::fs::remove_dir_all(dir).ok();
    }
}
