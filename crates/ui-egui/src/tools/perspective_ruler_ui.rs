//! View › Perspective Ruler on the canvas: the eye level, lines fanning out from each vanishing
//! point, and the vanishing points as handles to drag. Dragging a point previews the new
//! perspective and sets it on release (`view.perspectiveRuler.set`); the snapping itself is in
//! `stroke_constraint::perspective`.

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use egui::{Color32, Pos2, Stroke};
use photocraft_engine::perspective_cmds::PerspectiveRuler;
use serde_json::json;

/// Screen points around a vanishing point that grab it.
const GRAB: f32 = 14.0;
/// Lines drawn from each vanishing point across the canvas.
const RAYS: usize = 24;

/// The ruler as drawn now: the document's, or the one being dragged.
fn shown(app: &PhotocraftApp) -> Option<PerspectiveRuler> {
    let mut r = app.session.active()?.perspective.clone()?;
    if let Some((doc, i, p)) = app.perspective_drag
        && Some(doc) == app.session.active_index()
        && let Some(slot) = r.points.get_mut(i)
    {
        *slot = p;
    }
    Some(r)
}

/// A press on a vanishing point drags it; returns true while the ruler takes the pointer (so the
/// press never paints).
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent) -> bool {
    let Some(idx) = app.session.active_index() else {
        app.perspective_drag = None;
        return false;
    };
    match ev {
        ToolEvent::Down { x, y, .. } => {
            app.perspective_drag = None;
            let Some(r) = shown(app).filter(|r| r.visible) else { return false };
            let Some(xf) = ViewXform::active(app) else { return false };
            let tol = f64::from(GRAB / xf.zoom.max(1e-3));
            let hit = r.points.iter().position(|p| (p[0] - x).hypot(p[1] - y) <= tol);
            match hit {
                Some(i) => {
                    app.perspective_drag = Some((idx, i, [x, y]));
                    true
                }
                None => false,
            }
        }
        ToolEvent::Move { x, y, .. } => match &mut app.perspective_drag {
            Some((doc, _, p)) if *doc == idx => {
                *p = [x, y];
                true
            }
            _ => false,
        },
        ToolEvent::Up { x, y } => {
            let Some((doc, i, _)) = app.perspective_drag.take() else { return false };
            if doc == idx
                && let Some(mut r) = app.session.active().and_then(|st| st.perspective.clone())
                && let Some(slot) = r.points.get_mut(i)
            {
                *slot = [x, y];
                if let Err(e) = app.run("view.perspectiveRuler.set", json!({"vanishingPoints": r.points})) {
                    app.ui.status = e;
                    app.ui.status_error = true;
                }
            }
            true
        }
    }
}

/// The options bar's snap switch, shown while the document has a perspective ruler: on, strokes
/// snap to the ruler; off, they draw freely.
pub fn snap_button(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let Some(snap) = app.session.active().and_then(|st| st.perspective.as_ref()).map(|r| r.snap) else { return };
    let tip = if snap { tl!("Snapping to the perspective ruler (click to draw freely)") } else { tl!("Drawing freely (click to snap to the perspective ruler)") };
    let resp = crate::icons::button_with_icon_size(ui, "ruler", 24.0, 14.0, snap, tip);
    let resp = crate::brush_picker::named(resp, "Snap to perspective ruler");
    if resp.clicked()
        && let Err(e) = app.run("view.perspectiveRuler.snap", json!({}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}

/// Points spread evenly around the document's edges, for the fan of lines from a vanishing point.
fn edge_points(w: f64, h: f64, n: usize) -> Vec<[f64; 2]> {
    let perimeter = 2.0 * (w + h);
    (0..n)
        .map(|k| {
            let mut t = perimeter * (k as f64 + 0.5) / n as f64;
            if t < w {
                return [t, 0.0];
            }
            t -= w;
            if t < h {
                return [w, t];
            }
            t -= h;
            if t < w {
                return [w - t, h];
            }
            [0.0, h - (t - w)]
        })
        .collect()
}

/// The ruler's lines and handles over the canvas.
pub fn draw(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let Some(r) = shown(app).filter(|r| r.visible) else { return };
    let Some(st) = app.session.active() else { return };
    let (w, h) = (f64::from(st.doc.size.width), f64::from(st.doc.size.height));
    let painter = painter.with_clip_rect(painter.clip_rect().intersect(xf.rect));
    let screen = |p: [f64; 2]| xf.to_screen(p[0] as f32, p[1] as f32);
    let ok = |p: Pos2| p.x.is_finite() && p.y.is_finite();
    let colors = [Color32::from_rgb(64, 160, 255), Color32::from_rgb(255, 120, 80), Color32::from_rgb(120, 210, 90)];
    let shade = Stroke::new(2.5, Color32::from_black_alpha(70));
    // The fan of lines from each vanishing point, through the canvas and beyond it.
    let far = (w + h) * 4.0;
    for (i, vp) in r.points.iter().enumerate() {
        let c = colors[i % colors.len()];
        let line = Stroke::new(1.0, c.gamma_multiply(if r.snap { 0.55 } else { 0.3 }));
        for e in edge_points(w, h, RAYS) {
            let (dx, dy) = (e[0] - vp[0], e[1] - vp[1]);
            let l = dx.hypot(dy);
            if l < 1e-6 {
                continue;
            }
            let end = [vp[0] + dx / l * far, vp[1] + dy / l * far];
            let (a, b) = (screen(*vp), screen(end));
            if ok(a) && ok(b) {
                painter.line_segment([a, b], line);
            }
        }
    }
    // The eye level, across the whole view.
    if let Some((o, d)) = r.eye_level() {
        let reach = far * 2.0;
        let (a, b) = (screen([o[0] - d[0] * reach, o[1] - d[1] * reach]), screen([o[0] + d[0] * reach, o[1] + d[1] * reach]));
        if ok(a) && ok(b) {
            painter.line_segment([a, b], shade);
            painter.line_segment([a, b], Stroke::new(1.5, Color32::from_rgb(255, 210, 60)));
        }
    }
    // The vanishing points: handles to drag.
    for (i, vp) in r.points.iter().enumerate() {
        let p = screen(*vp);
        if !ok(p) {
            continue;
        }
        let c = colors[i % colors.len()];
        painter.circle(p, 7.0, Color32::from_black_alpha(90), Stroke::new(2.0, c));
        painter.circle_filled(p, 2.5, c);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app
    }

    #[test]
    fn the_options_bar_button_switches_snapping() {
        use egui_kittest::kittest::Queryable;
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(200.0, 60.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                ui.horizontal(|ui| snap_button(app, ui));
            },
            app(),
        );
        h.run();
        assert!(h.query_by_label("Snap to perspective ruler").is_none(), "no ruler, no button");
        h.state_mut().run("view.perspectiveRuler.onePoint", json!({})).unwrap();
        h.run();
        let snap = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().session.active().unwrap().perspective.as_ref().unwrap().snap;
        assert!(snap(&h));
        h.get_by_label("Snap to perspective ruler").click();
        h.run();
        assert!(!snap(&h), "off: draw freely");
        h.get_by_label("Snap to perspective ruler").click();
        h.run();
        assert!(snap(&h), "back on");
    }

    #[test]
    fn painting_snaps_to_the_ruler_and_a_vanishing_point_drags() {
        let mut app = app();
        app.ui.tool = crate::state::Tool::Brush;
        app.run("view.perspectiveRuler.twoPoint", json!({})).unwrap();
        let r = app.session.active().unwrap().perspective.clone().unwrap();
        assert_eq!(r.points, vec![[40.0, 135.0], [360.0, 135.0]]);
        // A stroke heading towards the right vanishing point, wobbling: every point lands on the
        // line from its start to that point.
        let mods = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 100.0, y: 250.0, pressure: 1.0 }, mods);
        for (x, y) in [(120.0, 240.0), (150.0, 225.0), (180.0, 214.0), (210.0, 200.0)] {
            crate::canvas::tool_event(&mut app, ToolEvent::Move { x, y, pressure: 1.0 }, mods);
        }
        let pts = app.drag.as_ref().unwrap().points.clone();
        let dir = [260.0_f64, -115.0];
        for p in pts.iter().skip(1) {
            let cross = (p[0] - 100.0) * dir[1] - (p[1] - 250.0) * dir[0];
            assert!(cross.abs() < 1e-6, "off the perspective line: {p:?}");
        }
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 210.0, y: 200.0 }, mods);
        // Dragging the left vanishing point moves it, and doesn't paint.
        let revision = app.session.active().unwrap().revision;
        assert!(pointer(&mut app, ToolEvent::Down { x: 41.0, y: 136.0, pressure: 1.0 }));
        assert!(pointer(&mut app, ToolEvent::Move { x: 10.0, y: 120.0, pressure: 1.0 }));
        assert!(pointer(&mut app, ToolEvent::Up { x: 0.0, y: 110.0 }));
        assert_eq!(app.session.active().unwrap().perspective.as_ref().unwrap().points[0], [0.0, 110.0]);
        assert_eq!(app.session.active().unwrap().revision, revision, "no paint, no history step");
        // Away from the points, a press paints as usual.
        assert!(!pointer(&mut app, ToolEvent::Down { x: 200.0, y: 200.0, pressure: 1.0 }));
    }

    #[test]
    fn snapping_off_or_shift_draws_freely() {
        let mut app = app();
        app.ui.tool = crate::state::Tool::Brush;
        app.run("view.perspectiveRuler.onePoint", json!({})).unwrap();
        app.run("view.perspectiveRuler.snap", json!({})).unwrap();
        let mods = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 50.0, y: 50.0, pressure: 1.0 }, mods);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 90.0, y: 77.0, pressure: 1.0 }, mods);
        let last = *app.drag.as_ref().unwrap().points.last().unwrap();
        assert_eq!([last[0], last[1]], [90.0, 77.0], "snap off: as drawn");
    }

    #[test]
    fn edge_points_go_round_the_canvas() {
        let pts = edge_points(100.0, 50.0, 6);
        assert_eq!(pts.len(), 6);
        assert!(pts.iter().all(|p| p[0] >= 0.0 && p[0] <= 100.0 && p[1] >= 0.0 && p[1] <= 50.0));
        assert!(pts.iter().all(|p| p[0] == 0.0 || p[0] == 100.0 || p[1] == 0.0 || p[1] == 50.0), "on the edges: {pts:?}");
    }
}
