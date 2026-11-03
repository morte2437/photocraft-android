//! Smart Guides › measuring distances, as in Photoshop (#4946): with Smart Guides on (View › Show
//! › Smart Guides, Extras on) and the Move tool, holding ⌘ (Ctrl off the Mac) with no button down
//! and hovering another layer shows the distances between the selected layers' bounds and that
//! layer's, as magenta lines labelled with the value in the ruler unit. Over empty canvas (or the
//! Background, or the selection itself) the distances run to the canvas edges instead. While an
//! ⌥-drag duplicates the layers, the distance between the original and the copy shows.
//!
//! It's an overlay only: nothing changes in the document and there is no command or History
//! state. The hovered layer is the one a click there would Auto-Select
//! (`pick_cmds::auto_select_target`), cached per document revision, selection and pointer pixel,
//! so a pointer resting on the canvas costs nothing per frame.

use egui::{Color32, Pos2, Stroke, pos2, vec2};
use photocraft_doc::{DocId, Document, LayerId};
use photocraft_engine::snap::{layer_rect, union};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;
use crate::state::Tool;

/// One measured distance: a line along x (`horizontal`, at y = `at`) or along y (at x = `at`),
/// from `from` to `to` on that axis (`from < to`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Measure {
    pub horizontal: bool,
    pub at: f64,
    pub from: f64,
    pub to: f64,
}

impl Measure {
    pub fn len(&self) -> f64 {
        self.to - self.from
    }
}

/// What to draw between two boxes: the measured lines, and dashed extensions `[x0, y0, x1, y1]`
/// of the other box's edges where a line ends beside the box rather than on it.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct Overlay {
    pub measures: Vec<Measure>,
    pub extensions: Vec<[f64; 4]>,
}

/// The distances from box `a` (the selection, `[x0, y0, x1, y1]`) to box `b` (the hovered layer
/// or the canvas), per axis: the gap between facing edges when the boxes are apart on that axis,
/// else the distances between their corresponding edges (left to left, right to right). Zero
/// distances aren't shown. A line runs through the middle of where the boxes overlap on the other
/// axis, or through `a`'s centre when they don't.
pub(crate) fn measure(a: [f64; 4], b: [f64; 4]) -> Overlay {
    let mut out = Overlay::default();
    if !a.iter().chain(b.iter()).all(|v| v.is_finite()) {
        return out;
    }
    for horizontal in [true, false] {
        // Spans along the measured axis (`s`) and across it (`o`).
        let (k, o) = if horizontal { (0, 1) } else { (1, 0) };
        let (sa, sb) = ([a[k], a[k + 2]], [b[k], b[k + 2]]);
        let (oa, ob) = ([a[o], a[o + 2]], [b[o], b[o + 2]]);
        let (lo, hi) = (oa[0].max(ob[0]), oa[1].min(ob[1]));
        let at = if lo < hi { (lo + hi) / 2.0 } else { (oa[0] + oa[1]) / 2.0 };
        // Each pair: the two ends, and which of them lies on `b`'s edge.
        let pairs: Vec<(f64, f64, f64)> = if sa[1] <= sb[0] {
            vec![(sa[1], sb[0], sb[0])]
        } else if sb[1] <= sa[0] {
            vec![(sb[1], sa[0], sb[1])]
        } else {
            vec![(sa[0].min(sb[0]), sa[0].max(sb[0]), sb[0]), (sa[1].min(sb[1]), sa[1].max(sb[1]), sb[1])]
        };
        for (from, to, edge) in pairs {
            if to - from <= 0.0 {
                continue;
            }
            out.measures.push(Measure { horizontal, at, from, to });
            // The line ends beside `b`: extend `b`'s edge to it.
            if at < ob[0] || at > ob[1] {
                let near = if at < ob[0] { ob[0] } else { ob[1] };
                out.extensions.push(if horizontal { [edge, near, edge, at] } else { [near, edge, at, edge] });
            }
        }
    }
    out
}

/// The bounds of layer `id` and, for a group, of the layers in it (`snap::layer_rect`: what the
/// Move tool moves and snaps).
fn tree_rect(doc: &Document, id: LayerId) -> Option<[f64; 4]> {
    let rows = doc.walk();
    let (prefix, _, _) = rows.iter().find(|(_, _, l)| l.id == id)?;
    union(rows.iter().filter(|(p, _, _)| p.starts_with(prefix)).filter_map(|(_, _, l)| layer_rect(doc, l.id)))
}

/// The last answer: the boxes measured with the pointer at `at` on that revision and selection.
pub(crate) struct Cache {
    doc: DocId,
    revision: u64,
    at: [i32; 2],
    group: bool,
    selected: Vec<LayerId>,
    boxes: Option<([f64; 4], [f64; 4])>,
}

/// The selected layers' box and the box to measure it to with the pointer at `at` (document px):
/// the layer a click there would Auto-Select, or the canvas where that is no other layer.
fn boxes(app: &mut PhotocraftApp, at: [f64; 2]) -> Option<([f64; 4], [f64; 4])> {
    if !at[0].is_finite() || !at[1].is_finite() {
        return None;
    }
    let at = [at[0].floor() as i32, at[1].floor() as i32];
    let group = app.ui.tool_options.move_target == "group";
    let st = app.session.active()?;
    let selected = st.selected_layers();
    let (doc, revision) = (st.doc.id, st.revision);
    if let Some(c) = app.move_distance.as_ref().filter(|c| c.doc == doc && c.revision == revision && c.at == at && c.group == group && c.selected == selected) {
        return c.boxes;
    }
    let d = &st.doc;
    let a = union(selected.iter().filter(|id| !d.is_background(**id)).filter_map(|id| tree_rect(d, *id)));
    let canvas = [0.0, 0.0, d.size.width as f64, d.size.height as f64];
    let hovered = photocraft_engine::pick_cmds::auto_select_target(d, at[0], at[1], group).filter(|id| {
        // The selection itself (or a layer in a selected group) isn't another layer.
        !d.is_background(*id) && !selected.iter().any(|s| *s == *id || d.path_of(*id).zip(d.path_of(*s)).is_some_and(|(p, q)| p.starts_with(&q)))
    });
    let boxes = a.map(|a| (a, hovered.and_then(|id| tree_rect(d, id)).unwrap_or(canvas)));
    app.move_distance = Some(Cache { doc, revision, at, group, selected, boxes });
    boxes
}

/// What to measure now: the ⌥-drag's original and copy while it duplicates, else, with ⌘ held
/// (`command`) and no button down (`pressed`), the selection and what's under the pointer `at`.
/// `None` when Smart Guides are off or another tool is current.
pub(crate) fn current(app: &mut PhotocraftApp, at: Option<[f64; 2]>, command: bool, pressed: bool) -> Option<Overlay> {
    if app.ui.tool != Tool::Move || !app.ui.view.shows(app.ui.view.show.smart_guides) || app.ui.transform.is_some() {
        return None;
    }
    if let Some((original, copy)) = duplicate_drag(app) {
        return Some(measure(original, copy));
    }
    if !command || pressed || app.drag.is_some() || app.guide_drag.is_some() {
        return None;
    }
    let (a, b) = boxes(app, at?)?;
    Some(measure(a, b))
}

/// An ⌥-drag that has duplicated the layers: the original's box and the copy's where it is now.
fn duplicate_drag(app: &PhotocraftApp) -> Option<([f64; 4], [f64; 4])> {
    if !crate::move_mods::duplicated(app) {
        return None;
    }
    let snap = app.prefs_rt.snap.as_ref()?;
    let crate::snap_ui::Gesture::Move { rect } = &snap.gesture else { return None };
    let p = app.drag.as_ref()?.points.last()?;
    let d = [p[0] - snap.start[0], p[1] - snap.start[1]];
    Some((*rect, [rect[0] + d[0], rect[1] + d[1], rect[2] + d[0], rect[3] + d[1]]))
}

/// Black or white, whichever reads better on `c`.
fn on(c: Color32) -> Color32 {
    let l = 0.299 * f32::from(c.r()) + 0.587 * f32::from(c.g()) + 0.114 * f32::from(c.b());
    if l > 150.0 { Color32::BLACK } else { Color32::WHITE }
}

/// Draws the measurements on the active document's canvas in the Smart Guides colour.
pub(crate) fn draw(app: &mut PhotocraftApp, painter: &egui::Painter, xf: &ViewXform, idx: usize, hover: Option<Pos2>) {
    if app.session.active_index() != Some(idx) {
        return;
    }
    let (mods, pressed) = painter.ctx().input(|i| (i.modifiers, i.pointer.any_down()));
    let command = crate::workspace_ui::sticky_mods(app, mods).command;
    let at = hover.map(|p| xf.to_doc(p));
    let Some(overlay) = current(app, at, command, pressed) else { return };
    let Some(st) = app.session.active() else { return };
    let (dpi, size) = (f64::from(st.doc.resolution_dpi), st.doc.size);
    let ur = &app.session.prefs().units_and_rulers;
    let color = crate::snap_ui::smart_color(app);
    let stroke = Stroke::new(1.0, color);
    let px = |p: Pos2| pos2(p.x.round() + 0.5, p.y.round() + 0.5);
    for e in &overlay.extensions {
        let (a, b) = (xf.to_screen(e[0] as f32, e[1] as f32), xf.to_screen(e[2] as f32, e[3] as f32));
        painter.add(egui::Shape::dashed_line(&[px(a), px(b)], stroke, 3.0, 3.0));
    }
    let t = crate::theme::Tokens::get(painter.ctx());
    let font = egui::FontId::proportional(11.0);
    for m in &overlay.measures {
        let (a, b) = if m.horizontal {
            (xf.to_screen(m.from as f32, m.at as f32), xf.to_screen(m.to as f32, m.at as f32))
        } else {
            (xf.to_screen(m.at as f32, m.from as f32), xf.to_screen(m.at as f32, m.to as f32))
        };
        let (a, b) = (px(a), px(b));
        painter.line_segment([a, b], stroke);
        // End caps across the line.
        let dir = (b - a).normalized();
        let cap = vec2(-dir.y, dir.x) * 4.0;
        for p in [a, b] {
            painter.line_segment([p - cap, p + cap], stroke);
        }
        let extent = if m.horizontal { size.width } else { size.height } as f64;
        let text = format!("{} {}", ur.format(m.len(), dpi, extent), ur.rulers.suffix());
        let galley = painter.layout_no_wrap(text, font.clone(), on(color));
        let mid = a + (b - a) / 2.0;
        let r = egui::Rect::from_center_size(mid, galley.size() + vec2(8.0, 4.0));
        painter.rect_filled(r, t.radius_sm, color);
        painter.galley(r.center() - galley.size() / 2.0, galley, on(color));
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn m(horizontal: bool, at: f64, from: f64, to: f64) -> Measure {
        Measure { horizontal, at, from, to }
    }

    #[test]
    fn boxes_apart_measure_the_gap_between_facing_edges() {
        // `b` 60 px right of `a`, overlapping it vertically from 20 to 40: the gap runs through
        // the middle of that overlap (y = 30).
        let o = measure([0.0, 0.0, 40.0, 40.0], [100.0, 20.0, 150.0, 80.0]);
        // Vertically they overlap: the corresponding edges (tops 0 → 20, bottoms 40 → 80), through
        // `a`'s centre (x = 20) as they don't overlap horizontally, with `b`'s top and bottom
        // edges extended to it.
        assert_eq!(o.measures, vec![m(true, 30.0, 40.0, 100.0), m(false, 20.0, 0.0, 20.0), m(false, 20.0, 40.0, 80.0)]);
        assert_eq!(o.extensions, vec![[100.0, 20.0, 20.0, 20.0], [100.0, 80.0, 20.0, 80.0]]);
    }

    #[test]
    fn diagonal_boxes_measure_both_gaps_from_the_selection_centre() {
        // `b` below-left of `a`: 30 px apart horizontally, 50 px vertically.
        let o = measure([100.0, 0.0, 140.0, 20.0], [20.0, 70.0, 70.0, 90.0]);
        assert_eq!(o.measures, vec![m(true, 10.0, 70.0, 100.0), m(false, 120.0, 20.0, 70.0)]);
        // Both lines end beside `b`: its right edge reaches up to y = 10, its top edge across to x = 120.
        assert_eq!(o.extensions, vec![[70.0, 70.0, 70.0, 10.0], [70.0, 70.0, 120.0, 70.0]]);
    }

    #[test]
    fn inside_the_canvas_measures_to_all_four_edges() {
        let o = measure([40.0, 40.0, 90.0, 80.0], [0.0, 0.0, 400.0, 300.0]);
        assert_eq!(o.measures, vec![m(true, 60.0, 0.0, 40.0), m(true, 60.0, 90.0, 400.0), m(false, 65.0, 0.0, 40.0), m(false, 65.0, 80.0, 300.0)]);
        assert!(o.extensions.is_empty());
        // Edges that touch measure nothing; nonsense boxes nothing at all.
        assert_eq!(measure([0.0, 0.0, 400.0, 300.0], [0.0, 0.0, 400.0, 300.0]), Overlay::default());
        assert_eq!(measure([f64::NAN, 0.0, 1.0, 1.0], [0.0, 0.0, 400.0, 300.0]), Overlay::default());
    }

    /// The `snap_ui` test document: a target box at (300..350, 200..260) and the selected mover
    /// at (40..90, 40..80) on a 400×300 canvas.
    fn app_with_boxes() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("layer.new.layer", json!({"name": "target"})).unwrap();
        app.run("select.rect", json!({"x": 300, "y": 200, "width": 50, "height": 60})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("layer.new.layer", json!({"name": "mover"})).unwrap();
        app.run("select.rect", json!({"x": 40, "y": 40, "width": 50, "height": 40})).unwrap();
        app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.view.extras = true;
        app.ui.view.show.smart_guides = true;
        app
    }

    #[test]
    fn command_hover_over_another_layer_measures_to_it() {
        let mut app = app_with_boxes();
        let revision = app.session.active().unwrap().revision;
        let o = current(&mut app, Some([320.5, 230.0]), true, false).unwrap();
        // 210 px across (90 → 300), 120 px down (80 → 200), from the mover's centre.
        assert_eq!(o.measures, vec![m(true, 60.0, 90.0, 300.0), m(false, 65.0, 80.0, 200.0)]);
        // Over empty canvas (the Background): to the canvas edges.
        let o = current(&mut app, Some([200.0, 150.0]), true, false).unwrap();
        assert_eq!(o.measures, vec![m(true, 60.0, 0.0, 40.0), m(true, 60.0, 90.0, 400.0), m(false, 65.0, 0.0, 40.0), m(false, 65.0, 80.0, 300.0)]);
        // Over the selected layer itself: also the canvas edges.
        assert_eq!(current(&mut app, Some([60.0, 60.0]), true, false).unwrap().measures.len(), 4);
        // Only an overlay: the document didn't change.
        assert_eq!(app.session.active().unwrap().revision, revision);
    }

    #[test]
    fn nothing_without_command_smart_guides_or_the_move_tool() {
        let mut app = app_with_boxes();
        let at = Some([320.5, 230.0]);
        assert!(current(&mut app, at, true, false).is_some());
        // ⌘ not held, or a button down (⌘-click auto-selects, unchanged).
        assert_eq!(current(&mut app, at, false, false), None);
        assert_eq!(current(&mut app, at, true, true), None);
        assert_eq!(current(&mut app, None, true, false), None);
        // Smart Guides off, or Extras off.
        app.ui.view.show.smart_guides = false;
        assert_eq!(current(&mut app, at, true, false), None);
        app.ui.view.show.smart_guides = true;
        app.ui.view.extras = false;
        assert_eq!(current(&mut app, at, true, false), None);
        app.ui.view.extras = true;
        // Another tool.
        app.ui.tool = Tool::Brush;
        assert_eq!(current(&mut app, at, true, false), None);
        // Nothing to measure from: the Background is selected (the Move tool doesn't move it).
        app.ui.tool = Tool::Move;
        let bg = app.session.active().unwrap().doc.walk().first().unwrap().2.id;
        app.run("layer.select", json!({"layer": bg.0})).unwrap();
        assert_eq!(current(&mut app, at, true, false), None);
    }

    #[test]
    fn alt_drag_duplicate_measures_from_the_original_to_the_copy() {
        let mut app = app_with_boxes();
        app.sync_views();
        app.ui.views[0].zoom = 1.0;
        app.ui.views[0].fit_pending = false;
        let alt = egui::Modifiers { alt: true, ..Default::default() };
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, alt);
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Move { x: 160.0, y: 60.0, pressure: 1.0 }, alt);
        // The copy's left edge is 50 px right of the original's right edge (90 → 140); they line
        // up vertically, so only the gap shows. A button is down and ⌘ isn't held.
        let o = current(&mut app, None, false, true).unwrap();
        assert_eq!(o.measures, vec![m(true, 60.0, 90.0, 140.0)]);
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 160.0, y: 60.0 }, alt);
        assert_eq!(current(&mut app, None, false, false), None);
    }

    /// Through the real canvas: ⌘ held over the other layer measures to it (the overlay is drawn
    /// from the cached boxes), and a ⌘-click there still Auto-Selects it as before.
    #[test]
    fn command_hover_on_the_canvas_draws_and_command_click_still_picks() {
        use egui::{Event, Modifiers, PointerButton, vec2};
        let mut app = app_with_boxes();
        app.ui.tool_options.move_auto_select = false;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 600.0)).build_ui_state(
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
        let v = &mut h.state_mut().ui.views[0];
        v.zoom = 1.0;
        v.center = [200.0, 150.0];
        v.fit_pending = false;
        h.run_steps(2);
        let screen = |h: &egui_kittest::Harness<'_, PhotocraftApp>, x: f32, y: f32| {
            let app = h.state();
            let v = &app.ui.views[0];
            let xf = ViewXform {
                rect: crate::rulers::content_rect(app, app.last_canvas_rect),
                zoom: v.zoom,
                center: v.center,
                flip: app.ui.view.flip_horizontal,
                rotation: v.rotation,
                aspect: app.ui.view.display_aspect(),
            };
            xf.to_screen(x, y)
        };
        let command = if cfg!(target_os = "macos") {
            Modifiers { command: true, mac_cmd: true, ..Default::default() }
        } else {
            Modifiers { command: true, ctrl: true, ..Default::default() }
        };
        let revision = h.state().session.active().unwrap().revision;
        let p = screen(&h, 320.5, 230.5);
        h.event(Event::PointerMoved(p));
        h.run_steps(2);
        // Not held: nothing measured.
        assert!(h.state().move_distance.is_none());
        h.event(Event::ModifiersChanged(command));
        h.run_steps(2);
        let boxes = h.state().move_distance.as_ref().and_then(|c| c.boxes);
        assert_eq!(boxes, Some(([40.0, 40.0, 90.0, 80.0], [300.0, 200.0, 350.0, 260.0])));
        if let Ok(path) = std::env::var("PHOTOCRAFT_MOVE_DISTANCE_SCREENSHOT") {
            h.render().unwrap().save(path).unwrap();
        }
        assert_eq!(h.state().session.active().unwrap().revision, revision, "hovering changed nothing");
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: command });
        h.run_steps(1);
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: command });
        h.run_steps(2);
        let st = h.state().session.active().unwrap();
        let picked = st.active_layer.and_then(|id| st.doc.layer(id)).map(|l| l.name.clone());
        assert_eq!(picked.as_deref(), Some("target"));
    }
}
