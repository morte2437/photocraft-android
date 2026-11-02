//! Photoshop's HUD Color Picker (#6098): with a painting tool, Shift+Alt+right-click on the
//! canvas opens a floating saturation/value square with a hue strip to its right, placed so the
//! current colour sits under the pointer (as the reference application does).
//! While the right button stays down, moving the pointer over the HUD picks the foreground colour
//! live (`tools.setColors` calls sharing one `coalesce` key, so the gesture is one history step,
//! Rule 1); releasing the button or pressing Escape closes the HUD and keeps the colour picked so
//! far, as in the reference application (Undo takes it back). A left press closes it too, which is
//! how the control channel ends a gesture it opened with `ui.pointer`.
//!
//! The HUD is drawn as plain meshes and hit-tested from the raw pointer position rather than with
//! interactive widgets: the selection follows the pointer while the *right* button is held, which
//! egui's widgets (left-button only) cannot express.

use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Color32, Mesh, PointerButton, Rect, Stroke, Vec2, vec2};
use serde::{Deserialize, Serialize};
use serde_json::json;

use photocraft_color::convert::{hsb_to_rgb, rgb_to_hsb};

use crate::PhotocraftApp;
use crate::paint_mouse::has_brush_picker;
use crate::state::UiState;

/// Width of the hue strip.
const HUE_W: f32 = 16.0;
/// Side of the saturation/value square, and the strip's height.
const SQUARE: f32 = 128.0;
/// Gap between the square and the strip.
const GAP: f32 = 8.0;
/// The HUD's content size (the frame's chrome sits around it).
pub const SIZE: [f32; 2] = [SQUARE + GAP + HUE_W, SQUARE];

/// The square (left) and the hue strip (right) inside the HUD's content rect.
fn layout(content: Rect) -> (Rect, Rect) {
    let square = Rect::from_min_size(content.min, vec2(SQUARE, SQUARE));
    let strip = Rect::from_min_size(content.min + vec2(SQUARE + GAP, 0.0), vec2(HUE_W, SQUARE));
    (strip, square)
}

/// Where the current colour (saturation, value) sits inside the content rect, from its top left.
fn colour_offset(hsv: [f32; 3]) -> Vec2 {
    let f = |v: f32| if v.is_finite() { v.clamp(0.0, 1.0) } else { 0.0 };
    vec2(f(hsv[1]) * SQUARE, (1.0 - f(hsv[2])) * SQUARE)
}

/// The HUD Color Picker while it is open.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Hud {
    /// Where the gesture began, in screen points: the HUD opens with the starting colour here.
    pub anchor: [f32; 2],
    /// The colour being picked: hue 0..360, saturation and value 0..1.
    pub hsv: [f32; 3],
    /// The foreground colour when the gesture began: where the HUD opens, and the alpha it keeps.
    pub start: [f32; 4],
    /// The `coalesce` key of this gesture's `tools.setColors` calls.
    pub key: u64,
    /// Begun by a real right-press: it ends when the right button comes up. One opened through
    /// the control channel ends with its Up event instead.
    pub hold: bool,
    /// The gesture has just opened: near a screen edge the clamped HUD can sit under the still
    /// pointer, so hover picking waits until the pointer moves from the anchor.
    #[serde(default)]
    pub just_opened: bool,
}

pub(crate) fn hud_id() -> egui::Id {
    egui::Id::unique("canvas-color-hud")
}

/// Is the HUD open?
pub fn is_open(ui: &UiState) -> bool {
    ui.color_hud.is_some()
}

/// Open the HUD at `at`, seeded from the current foreground colour. `hold` marks a real-mouse
/// gesture (see [`Hud::hold`]).
pub fn open(app: &mut PhotocraftApp, at: [f32; 2], hold: bool) {
    static KEY: AtomicU64 = AtomicU64::new(0);
    let fg = app.session.tools.foreground;
    let hsv = rgb_to_hsb([fg[0], fg[1], fg[2]]);
    app.ui.color_hud = Some(Hud { anchor: at, hsv: [hsv[0], hsv[1], hsv[2]], start: fg, key: KEY.fetch_add(1, Ordering::Relaxed), hold, just_opened: true });
}

/// Close the HUD, keeping the picked colour.
pub fn close(ui: &mut UiState) {
    ui.color_hud = None;
}

/// Close a control-channel HUD on its Up event; a real-mouse HUD closes on the button release.
pub fn release(ui: &mut UiState) {
    if ui.color_hud.is_some_and(|h| !h.hold) {
        close(ui);
    }
}

/// The hue, saturation and value a pointer at `p` selects inside these rects (Photoshop's strip
/// runs red at the top down through the spectrum; the square is saturation left→right, value
/// top→bottom). `None` when the point is on neither.
pub fn pick_at(hsv: [f32; 3], strip: Rect, square: Rect, p: egui::Pos2) -> Option<[f32; 3]> {
    if square.contains(p) {
        let s = ((p.x - square.left()) / square.width()).clamp(0.0, 1.0);
        let v = 1.0 - ((p.y - square.top()) / square.height()).clamp(0.0, 1.0);
        Some([hsv[0], s, v])
    } else if strip.contains(p) {
        let h = ((p.y - strip.top()) / strip.height()).clamp(0.0, 1.0) * 360.0;
        Some([h, hsv[1], hsv[2]])
    } else {
        None
    }
}

fn rgb_color(c: [f32; 3]) -> Color32 {
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(q(c[0]), q(c[1]), q(c[2]))
}

/// The vertical hue strip: red at the top, a full spectrum cycle. One mesh of bands, so its cost
/// doesn't grow with the HUD. Row `k` (of `n + 1`) holds the left vertex at index `2k` and the
/// right one at `2k + 1`; band `k` joins rows `k` and `k + 1`.
fn hue_strip_mesh(r: Rect) -> Mesh {
    let n = 12;
    let mut mesh = Mesh::default();
    for k in 0..=n {
        let y = r.top() + r.height() * k as f32 / n as f32;
        let c = rgb_color(hsb_to_rgb([k as f32 / n as f32 * 360.0, 1.0, 1.0]));
        mesh.colored_vertex(egui::pos2(r.left(), y), c);
        mesh.colored_vertex(egui::pos2(r.right(), y), c);
    }
    for k in 0..n {
        let (a, b) = (2 * k, 2 * k + 1);
        mesh.add_triangle(a, b, b + 2);
        mesh.add_triangle(a, b + 2, a + 2);
    }
    mesh
}

fn paint_hue_strip(painter: &egui::Painter, r: Rect) {
    painter.add(hue_strip_mesh(r));
}

/// The saturation/value square for `hue`: white→hue horizontally, then clear→black vertically.
fn paint_sv_square(painter: &egui::Painter, r: Rect, hue: f32) {
    let hue_c = rgb_color(hsb_to_rgb([hue, 1.0, 1.0]));
    let mut mesh = Mesh::default();
    mesh.colored_vertex(r.left_top(), Color32::WHITE);
    mesh.colored_vertex(r.right_top(), hue_c);
    mesh.colored_vertex(r.left_bottom(), Color32::WHITE);
    mesh.colored_vertex(r.right_bottom(), hue_c);
    mesh.add_triangle(0, 1, 3);
    mesh.add_triangle(0, 3, 2);
    painter.add(mesh);
    let mut mesh = Mesh::default();
    mesh.colored_vertex(r.left_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(r.right_top(), Color32::TRANSPARENT);
    mesh.colored_vertex(r.left_bottom(), Color32::BLACK);
    mesh.colored_vertex(r.right_bottom(), Color32::BLACK);
    mesh.add_triangle(0, 1, 3);
    mesh.add_triangle(0, 3, 2);
    painter.add(mesh);
}

/// Draw the HUD and track the pointer: called once per frame while a gesture is open.
pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(mut hud) = app.ui.color_hud else { return };
    if !has_brush_picker(app.ui.tool) {
        close(&mut app.ui);
        return;
    }
    let (released, esc, pressed_left, hover) = ctx.input(|i| {
        (
            i.pointer.button_released(PointerButton::Secondary),
            i.key_pressed(egui::Key::Escape),
            i.pointer.button_pressed(PointerButton::Primary),
            i.pointer.hover_pos(),
        )
    });
    if esc || (hud.hold && released) || pressed_left {
        close(&mut app.ui);
        return;
    }
    // The starting colour's spot in the square sits under the press point, as in the reference
    // application; near a screen edge the HUD is pulled in so it stays whole.
    let screen = ctx.content_rect();
    let style = ctx.global_style();
    let m = style.spacing.menu_margin;
    let stroke = style.visuals.window_stroke().width;
    let size = Vec2::from(SIZE) + m.sum() + egui::Vec2::splat(2.0 * stroke);
    let start = rgb_to_hsb([hud.start[0], hud.start[1], hud.start[2]]);
    let at = egui::pos2(hud.anchor[0], hud.anchor[1]) - colour_offset(start) - vec2(f32::from(m.left) + stroke, f32::from(m.top) + stroke);
    let pos = egui::pos2(at.x.min(screen.right() - size.x).max(screen.left()), at.y.min(screen.bottom() - size.y).max(screen.top()));
    let area = egui::Area::new(hud_id()).order(egui::Order::Foreground).fixed_pos(pos).show(ctx, |ui| {
        egui::Frame::popup(ui.style())
            .show(ui, |ui| {
                let (rect, _) = ui.allocate_exact_size(vec2(SIZE[0], SIZE[1]), egui::Sense::hover());
                let (strip, square) = layout(rect);
                let p = ui.painter();
                paint_hue_strip(p, strip);
                paint_sv_square(p, square, hud.hsv[0]);
                p.rect_stroke(strip, 0.0, Stroke::new(1.0, Color32::from_black_alpha(120)), egui::StrokeKind::Inside);
                p.rect_stroke(square, 0.0, Stroke::new(1.0, Color32::from_black_alpha(120)), egui::StrokeKind::Inside);
                // Markers: a ring in the square at the picked s/v, a bar on the strip at the hue.
                let sv = egui::pos2(square.left() + hud.hsv[1] * square.width(), square.top() + (1.0 - hud.hsv[2]) * square.height());
                p.circle(sv, 4.0, Color32::TRANSPARENT, Stroke::new(1.5, Color32::WHITE));
                p.circle(sv, 5.5, Color32::TRANSPARENT, Stroke::new(1.0, Color32::from_black_alpha(160)));
                let hy = strip.top() + hud.hsv[0] / 360.0 * strip.height();
                p.line_segment([egui::pos2(strip.left() + 1.0, hy), egui::pos2(strip.right(), hy)], Stroke::new(2.0, Color32::WHITE));
                (strip, square)
            })
            .inner
    });
    let (strip, square) = area.inner;
    // The HUD opens under the stationary pointer: picking starts only once the pointer has moved
    // from where the gesture began, so the opening press alone never changes the colour.
    if hud.just_opened && hover.is_some_and(|p| p != egui::pos2(hud.anchor[0], hud.anchor[1])) {
        hud.just_opened = false;
    }
    if !hud.just_opened
        && let Some(p) = hover
        && let Some(new) = pick_at(hud.hsv, strip, square, p)
        && new != hud.hsv
    {
        hud.hsv = new;
        let [r, g, b] = hsb_to_rgb(new);
        let fg = app.session.tools.foreground;
        if (r - fg[0]).abs() > f32::EPSILON || (g - fg[1]).abs() > f32::EPSILON || (b - fg[2]).abs() > f32::EPSILON {
            set_foreground(app, [r, g, b, hud.start[3]], hud.key);
        }
    }
    app.ui.color_hud = Some(hud);
    // The selection follows the pointer even when it rests between frames.
    ctx.request_repaint();
}

/// One foreground-colour edit of the gesture: coalesced under `key`, so the whole pick is a single
/// history step (the colour is tool state, so it never touches the document).
fn set_foreground(app: &mut PhotocraftApp, rgba: [f32; 4], key: u64) {
    let p = json!({ "foreground": rgba, "coalesce": format!("color-hud:{key}") });
    if let Err(e) = app.run("tools.setColors", p) {
        app.ui.status = e;
    }
}

#[cfg(test)]
mod tests {
    use egui::{Modifiers, PointerButton, Pos2, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    use super::*;
    use crate::state::Tool;

    const SHIFT_ALT: Modifiers = Modifiers { alt: true, ctrl: false, shift: true, mac_cmd: false, command: false };

    fn harness() -> Harness<'static, PhotocraftApp> {
        harness_size(400, 300)
    }

    fn harness_size(width: u32, height: u32) -> Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": width, "height": height})).unwrap();
        app.run("tools.setColors", json!({"foreground": "#0000ff"})).unwrap();
        app.ui.tool = Tool::Brush;
        app.sync_views();
        let mut h = Harness::builder().with_size(vec2(1200.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        h
    }

    fn button(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2, pressed: bool, modifiers: Modifiers) {
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.event(egui::Event::PointerButton { pos, button: PointerButton::Secondary, pressed, modifiers });
        h.run_steps(2);
    }

    /// The HUD's strip and square rects as drawn this frame.
    fn rects(h: &Harness<'static, PhotocraftApp>) -> (Rect, Rect) {
        let area = h.ctx.memory(|m| m.area_rect(hud_id())).expect("the HUD is shown");
        let style = h.ctx.global_style();
        let m = style.spacing.menu_margin;
        let w = style.visuals.window_stroke().width;
        let inner =
            Rect::from_min_max(area.min + vec2(f32::from(m.left) + w, f32::from(m.top) + w), area.max - vec2(f32::from(m.right) + w, f32::from(m.bottom) + w));
        layout(inner)
    }

    fn foreground(h: &Harness<'static, PhotocraftApp>) -> [f32; 4] {
        h.state().session.tools.foreground
    }

    #[test]
    fn shift_alt_right_press_opens_the_hud_not_the_picker() {
        let mut h = harness();
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        let hud = h.state().ui.color_hud.expect("the HUD opened");
        assert!(hud.hold);
        assert_eq!(hud.start, [0.0, 0.0, 1.0, 1.0], "seeded from the foreground");
        assert_eq!(h.state().ui.brush_picker, None, "the Brush Preset picker stays closed");
        // The square is left of the strip, and the pointer sits on the current colour: blue is
        // full saturation and value, the square's top-right corner.
        let (strip, square) = rects(&h);
        assert!(strip.height() > 100.0 && square.width() > 100.0);
        assert!(square.right() <= strip.left(), "square {square:?} left of strip {strip:?}");
        assert!(strip.right() < h.ctx.content_rect().right());
        assert!((square.right_top() - c).length() < 1.0, "the current colour {:?} is under the pointer {c:?}", square.right_top());
        // Nothing painted.
        assert!(!h.state().session.journal.iter().any(|(id, _)| id == "paint.stroke"));
    }

    /// Quick Selection's canvas right-click is the selection menu (#7268), so Shift+Alt+right
    /// must not open the HUD from the same gesture (#6098).
    #[test]
    fn quick_selections_shift_alt_right_opens_only_the_selection_menu() {
        let mut h = harness();
        h.state_mut().ui.tool = Tool::QuickSelection;
        h.run_steps(1);
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        assert_eq!(h.state().ui.color_hud, None, "no HUD for Quick Selection");
        assert_eq!(h.state().ui.brush_picker, None, "and no Brush Preset picker");
        button(&mut h, c, false, SHIFT_ALT);
        assert_eq!(h.state().ui.color_hud, None, "the release does not open it either");
        assert!(h.state().ui.canvas_tool_menu.is_some(), "the selection menu opens, once");
    }

    /// The HUD opens with the current colour under the still pointer: the opening press alone
    /// must not pick, the colour changes only once the pointer moves (#6098). Near the
    /// bottom-right edge the HUD is pulled in to stay on screen, and still picks nothing.
    #[test]
    fn the_opening_press_picks_nothing_until_the_pointer_moves() {
        let mut h = harness_size(1200, 800);
        h.state_mut().run("tools.setColors", json!({"foreground": [0.4, 0.3, 0.6, 1.0]})).unwrap();
        let start = foreground(&h);
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        assert!(h.state().ui.color_hud.is_some_and(|hud| hud.just_opened), "the HUD opened");
        let (_strip, square) = rects(&h);
        let hsv = rgb_to_hsb([start[0], start[1], start[2]]);
        let spot = square.min + colour_offset([hsv[0], hsv[1], hsv[2]]);
        assert!(square.contains(c) && (spot - c).length() < 1.0, "the current colour {spot:?} is under the pointer {c:?}");
        h.run_steps(4);
        assert_eq!(foreground(&h), start, "the stationary opening pointer picks nothing");
        // Moving inside the HUD now picks, as during any other gesture.
        let to = square.min + vec2(1.0, square.height() - 1.0);
        h.event(egui::Event::PointerMoved(to));
        h.run_steps(2);
        assert_ne!(foreground(&h), start, "the move picks");
        button(&mut h, to, false, SHIFT_ALT);
        // At the bottom-right corner the HUD stays whole on screen, and the press alone picks nothing.
        let near = egui::pos2(h.state().last_canvas_rect.max.x - 10.0, h.state().last_canvas_rect.max.y - 10.0);
        let start = foreground(&h);
        h.event(egui::Event::PointerMoved(near));
        h.run_steps(1);
        button(&mut h, near, true, SHIFT_ALT);
        let (strip, square) = rects(&h);
        let screen = h.ctx.content_rect();
        assert!(screen.contains_rect(square.union(strip)), "on screen: {square:?} {strip:?} in {screen:?}");
        h.run_steps(4);
        assert_eq!(foreground(&h), start, "the press near the edge picks nothing");
        button(&mut h, near, false, SHIFT_ALT);
    }

    #[test]
    fn dragging_over_the_hud_picks_the_foreground_live_and_release_keeps_it() {
        let mut h = harness();
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        let j0 = h.state().session.journal.len();
        let (_strip, square) = rects(&h);
        // Top-right of the square: full saturation, full value at the current hue (blue).
        let pick = egui::pos2(square.max.x - 1.0, square.min.y + 1.0);
        h.event(egui::Event::PointerMoved(pick));
        h.run_steps(2);
        let fg = foreground(&h);
        assert!(fg[2] > 0.9 && fg[0] < 0.1 && fg[1] < 0.1, "still blue at the top of the square: {fg:?}");
        // Bottom-left: value 0 → black.
        h.event(egui::Event::PointerMoved(square.min + vec2(1.0, square.height() - 1.0)));
        h.run_steps(2);
        let fg = foreground(&h);
        assert!(fg[..3].iter().all(|v| *v < 0.02), "black at the bottom: {fg:?}");
        // The hue strip: its middle is cyan-ish (180°), at the square's current s/v.
        let (strip, _) = rects(&h);
        h.event(egui::Event::PointerMoved(egui::pos2(square.max.x - 1.0, square.min.y + 1.0)));
        h.run_steps(2);
        h.event(egui::Event::PointerMoved(strip.center()));
        h.run_steps(2);
        let fg = foreground(&h);
        assert!(fg[2] > 0.5 && fg[0] < 0.5, "hue moved off red: {fg:?}");
        // The gesture's edits share one coalesce key (one history step, like the brush HUD).
        let sets: Vec<&serde_json::Value> = h.state().session.journal[j0..].iter().filter(|(id, _)| id == "tools.setColors").map(|(_, p)| p).collect();
        assert!(!sets.is_empty(), "the picks are journaled");
        let keys: Vec<&str> = sets.iter().map(|p| p["coalesce"].as_str().unwrap()).collect();
        assert!(keys.windows(2).all(|w| w[0] == w[1]), "one gesture, one key: {keys:?}");
        // Releasing the right button closes the HUD and keeps the colour.
        let kept = foreground(&h);
        button(&mut h, strip.center(), false, SHIFT_ALT);
        assert_eq!(h.state().ui.color_hud, None);
        assert_eq!(foreground(&h), kept);
    }

    /// The reference application's Escape ends the pick where it is: the HUD closes, the colour
    /// picked so far stays, and moving afterwards changes nothing.
    #[test]
    fn escape_closes_the_hud_and_keeps_the_picked_colour() {
        let mut h = harness();
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        let (_strip, square) = rects(&h);
        h.event(egui::Event::PointerMoved(square.center()));
        h.run_steps(2);
        let picked = foreground(&h);
        assert_ne!(picked, [0.0, 0.0, 1.0, 1.0], "the pick is live");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert_eq!(h.state().ui.color_hud, None);
        assert_eq!(foreground(&h), picked, "Escape keeps the picked colour");
        h.event(egui::Event::PointerMoved(square.min + vec2(2.0, 2.0)));
        h.run_steps(2);
        assert_eq!(foreground(&h), picked, "moving after Escape picks nothing");
        button(&mut h, square.min + vec2(2.0, 2.0), false, SHIFT_ALT);
        assert_eq!(h.state().ui.color_hud, None);
        assert_eq!(h.state().ui.brush_picker, None, "releasing the button afterwards opens nothing");
        assert_eq!(foreground(&h), picked);
    }

    #[test]
    fn plain_right_click_still_opens_the_picker_and_alt_drag_still_resizes() {
        let mut h = harness();
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, Modifiers::NONE);
        button(&mut h, c, false, Modifiers::NONE);
        assert!(h.state().ui.brush_picker.is_some() && h.state().ui.color_hud.is_none());
        // Alt+right-drag (no Shift) resizes the brush, as before.
        h.state_mut().ui.brush_picker = None;
        h.run_steps(1);
        h.event(egui::Event::ModifiersChanged(Modifiers::ALT));
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, Modifiers::ALT);
        h.event(egui::Event::PointerMoved(c + vec2(40.0, 0.0)));
        h.run_steps(2);
        assert!(h.state().brush_resize.is_some_and(|r| r.secondary), "Alt+right still resizes");
        assert!(h.state().ui.color_hud.is_none());
        button(&mut h, c + vec2(40.0, 0.0), false, Modifiers::ALT);
    }

    #[test]
    fn shift_alt_right_drag_does_not_resize_the_brush() {
        let mut h = harness();
        let size0 = h.state().session.tools.brush.size;
        let c = h.state().last_canvas_rect.center();
        h.event(egui::Event::PointerMoved(c));
        h.run_steps(1);
        button(&mut h, c, true, SHIFT_ALT);
        for k in 1..=6 {
            h.event(egui::Event::PointerMoved(c + vec2(20.0 * k as f32, 0.0)));
            h.run_steps(1);
        }
        assert!(h.state().brush_resize.is_none(), "the HUD gesture is not a resize");
        assert_eq!(h.state().session.tools.brush.size, size0);
        button(&mut h, c + vec2(120.0, 0.0), false, SHIFT_ALT);
    }

    #[test]
    fn the_control_channel_opens_and_closes_the_hud() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
        app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
        app.ui.tool = Tool::Brush;
        let ctx = egui::Context::default();
        let (req, _rx) = crate::control::ControlRequest::new(
            "ui.pointer",
            json!({"button": "right", "shift": true, "alt": true, "events": [{"kind": "down", "x": 50, "y": 50}, {"kind": "up", "x": 50, "y": 50}]}),
        );
        let _ = crate::control::handle(&mut app, &ctx, &req);
        assert!(app.ui.color_hud.is_none(), "the Up event closed it");
        assert!(app.ui.brush_picker.is_none(), "the picker stayed closed");
        // A down without its up leaves the HUD open for the next frame's edits.
        let (req, _rx) = crate::control::ControlRequest::new(
            "ui.pointer",
            json!({"button": "right", "shift": true, "alt": true, "events": [{"kind": "down", "x": 50, "y": 50}]}),
        );
        let _ = crate::control::handle(&mut app, &ctx, &req);
        assert!(app.ui.color_hud.is_some_and(|h| !h.hold));
    }

    #[test]
    fn hue_strip_mesh_indices_stay_within_its_rows() {
        let mesh = hue_strip_mesh(Rect::from_min_size(Pos2::ZERO, vec2(HUE_W, SQUARE)));
        let rows = 13; // n + 1 rows, two vertices each
        assert_eq!(mesh.vertices.len(), 2 * rows);
        assert_eq!(mesh.indices.len(), 3 * 2 * (rows - 1), "two triangles per band");
        for (j, tri) in mesh.indices.chunks(3).enumerate() {
            // Band k = j/2 joins rows k and k+1: vertices 2k..=2k+3, nothing further ahead.
            let k = j / 2;
            for &i in tri {
                assert!((2 * k) as u32 <= i && i <= (2 * k + 3) as u32, "band {k} uses index {i}: {tri:?}");
                assert!((i as usize) < mesh.vertices.len(), "index {i} out of range");
            }
        }
    }

    #[test]
    fn pick_at_maps_the_rects_and_ignores_the_gaps() {
        let (strip, square) = layout(Rect::from_min_size(Pos2::ZERO, Vec2::from(SIZE)));
        assert_eq!(pick_at([120.0, 0.5, 0.5], strip, square, square.center()), Some([120.0, 0.5, 0.5]));
        let p = pick_at([120.0, 0.5, 0.5], strip, square, square.left_top() + vec2(1.0, 1.0)).expect("in the square");
        assert!(p[0] == 120.0 && p[1] < 0.02 && p[2] > 0.98, "top-left is saturated and bright: {p:?}");
        assert_eq!(pick_at([0.0, 0.5, 0.5], strip, square, strip.center()), Some([180.0, 0.5, 0.5]));
        assert_eq!(pick_at([0.0, 0.5, 0.5], strip, square, Pos2::new(SQUARE + GAP / 2.0, 10.0)), None, "the gap picks nothing");
        assert_eq!(pick_at([0.0, 0.5, 0.5], strip, square, Pos2::new(1e9, -1e9)), None);
    }
}
