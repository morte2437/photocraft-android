//! Slice tool and Slice Select tool (Crop group, C): drawing new user slices, selecting and
//! moving slices, the slice overlay (outlines and numbered badges in the Preferences › Guides,
//! Grid & Slices colour) and both tools' options bars. Every change is an engine command
//! (`slice.new`, `slice.set`, `slice.promote`, `slice.divide`, `slice.delete`); this module only
//! turns pointer gestures into them.

use egui::{Color32, Rect, Stroke, vec2};
use photocraft_doc::slices::{self, ResolvedSlice};
use photocraft_doc::{Document, SliceOrigin};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::canvas::{ToolEvent, ViewXform};
use crate::state::{DialogKind, Tool};

/// Slice tool state (serde: readable through the control channel).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct SliceUi {
    /// The selected slice's display number (Slice Select tool).
    pub selected: Option<usize>,
    /// Slice Select options bar › Hide Auto Slices.
    pub hide_auto: bool,
    #[serde(skip)]
    pub drag: Option<SliceDrag>,
}

/// A gesture in progress.
#[derive(Clone, Debug, PartialEq)]
pub enum SliceDrag {
    New {
        start: [f64; 2],
        cur: [f64; 2],
    },
    Move {
        id: u32,
        start: [f64; 2],
        cur: [f64; 2],
        rect: photocraft_geom::Rect,
    },
    /// A handle of the selected slice: `handle` is −1, 0 or 1 per axis (the left, untouched or
    /// right edge; the top, untouched or bottom edge).
    Resize {
        id: u32,
        handle: [i8; 2],
        start: [f64; 2],
        cur: [f64; 2],
        rect: photocraft_geom::Rect,
    },
}

/// How far from a handle (screen points) a press still grabs it.
const HANDLE_REACH: f64 = 5.0;

/// The handle of `r` at document point `p`, within `reach` document pixels: the corners first,
/// then the edge middles, as the overlay draws them.
fn handle_at(r: photocraft_geom::Rect, p: [f64; 2], reach: f64) -> Option<[i8; 2]> {
    let (x0, y0, x1, y1) = (f64::from(r.x0), f64::from(r.y0), f64::from(r.x1), f64::from(r.y1));
    let at = |h: [i8; 2]| {
        let x = match h[0] {
            -1 => x0,
            1 => x1,
            _ => (x0 + x1) / 2.0,
        };
        let y = match h[1] {
            -1 => y0,
            1 => y1,
            _ => (y0 + y1) / 2.0,
        };
        (p[0] - x).abs() <= reach && (p[1] - y).abs() <= reach
    };
    [[-1, -1], [1, -1], [-1, 1], [1, 1], [0, -1], [0, 1], [-1, 0], [1, 0]].into_iter().find(|h| at(*h))
}

/// `r` with the edges `handle` grabs moved by (`dx`, `dy`), turned the right way round when an
/// edge is dragged past the opposite one, and at least 1 × 1.
fn resized(r: photocraft_geom::Rect, handle: [i8; 2], dx: i32, dy: i32) -> photocraft_geom::Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (r.x0, r.y0, r.x1, r.y1);
    match handle[0] {
        -1 => x0 = x0.saturating_add(dx),
        1 => x1 = x1.saturating_add(dx),
        _ => {}
    }
    match handle[1] {
        -1 => y0 = y0.saturating_add(dy),
        1 => y1 = y1.saturating_add(dy),
        _ => {}
    }
    let (x0, x1) = (x0.min(x1), x0.max(x1).max(x0.min(x1).saturating_add(1)));
    let (y0, y1) = (y0.min(y1), y0.max(y1).max(y0.min(y1).saturating_add(1)));
    photocraft_geom::Rect::new(x0, y0, x1, y1)
}

fn is_slice_tool(t: Tool) -> bool {
    matches!(t, Tool::Slice | Tool::SliceSelect)
}

/// The slice under a document point: stored slices win (the latest on top), then auto slices.
pub fn slice_at(doc: &Document, x: f64, y: f64) -> Option<ResolvedSlice> {
    let all = slices::resolve(doc);
    let (xi, yi) = (x.floor() as i32, y.floor() as i32);
    let order = |r: &ResolvedSlice| r.id.and_then(|id| doc.slices.list.iter().position(|s| s.id == id));
    all.iter().filter(|r| r.rect.contains(xi, yi)).max_by_key(|r| order(r).map_or(0, |i| i + 1)).cloned()
}

fn rect_from(a: [f64; 2], b: [f64; 2]) -> photocraft_geom::Rect {
    let (x0, x1) = (a[0].min(b[0]).floor() as i32, a[0].max(b[0]).ceil() as i32);
    let (y0, y1) = (a[1].min(b[1]).floor() as i32, a[1].max(b[1]).ceil() as i32);
    photocraft_geom::Rect::new(x0, y0, x1, y1)
}

/// The new slice drawn from `start` to `cur` under the Slice tool's Style (Normal, Fixed Ratio,
/// Fixed Size), as the marquee's (#8360). A fixed style is placed on the pixel the press is in.
/// Fixed Size has exactly its pixels. Fixed Ratio's longer side follows the drag to within a pixel
/// and the shorter side takes the ratio from it, as close as whole pixels allow: rounding each edge of
/// a fractional drag apart would make 2 : 1 into 5 × 3, and keeping to whole multiples of the
/// ratio would grow a 1366 : 768 slice in steps of 683 px.
fn new_rect(app: &PhotocraftApp, start: [f64; 2], cur: [f64; 2]) -> photocraft_geom::Rect {
    let o = &app.ui.tool_options;
    let (w, h) = (f64::from(o.slice_width), f64::from(o.slice_height));
    let size = match o.slice_style.as_str() {
        "fixedSize" => [w.round().max(1.0), h.round().max(1.0)],
        "fixedRatio" if w > 0.0 && h > 0.0 => {
            let end = crate::chrome_ui::marquee_end("fixedRatio", w, h, false, start, cur);
            let k = (end[0] - start[0]).abs() / w;
            // A drag shorter than half a pixel is a click: no size.
            let follow = |long: f64, short: f64| {
                let l = (k * long).round();
                [l, if l > 0.0 { (l * short / long).round().max(1.0) } else { 0.0 }]
            };
            if w >= h {
                follow(w, h)
            } else {
                let [l, s] = follow(h, w);
                [s, l]
            }
        }
        _ => return rect_from(start, crate::chrome_ui::marquee_end(&o.slice_style, w, h, false, start, cur)),
    };
    // The drag's extent and direction are from the press itself; only the slice is placed on the
    // pixel the press is in.
    let sign = |d: f64| if d < 0.0 { -1.0 } else { 1.0 };
    // A slice drawn left or up grows from the far side of that pixel, so it holds the pixel: the
    // edges a Normal slice gets from the same press (`rect_from`), also on a pixel's edge.
    let dir = [sign(cur[0] - start[0]), sign(cur[1] - start[1])];
    let corner = [0, 1].map(|i| if dir[i] < 0.0 { start[i].ceil() } else { start[i].floor() });
    rect_from(corner, [corner[0] + dir[0] * size[0], corner[1] + dir[1] * size[1]])
}

/// Pointer input for the two slice tools. Returns true when the event was theirs.
pub fn pointer(app: &mut PhotocraftApp, ev: ToolEvent, _mods: egui::Modifiers) -> bool {
    let tool = app.ui.tool;
    if !is_slice_tool(tool) {
        return false;
    }
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return true };
    let locked = app.session.file_menu.slices_locked;
    match ev {
        // View › Lock Slices: neither tool draws, selects or moves a slice, and the press says
        // why, as Photoshop's error does (#7865).
        ToolEvent::Down { .. } if locked => {
            app.ui.slices.selected = None;
            app.ui.status = tl!("Slices are locked (View › Lock Slices)").to_string();
            app.ui.status_error = true;
        }
        ToolEvent::Down { x, y, .. } => {
            if tool == Tool::Slice {
                app.ui.slices.drag = Some(SliceDrag::New { start: [x, y], cur: [x, y] });
            } else {
                // A handle of the selected user slice resizes it (#7863).
                let reach = HANDLE_REACH / f64::from(app.point_zoom().max(0.01));
                let grabbed = app.ui.slices.selected.filter(|_| !locked).and_then(|n| {
                    let r = slices::resolve(&doc).into_iter().find(|r| r.number == n && r.origin == SliceOrigin::User)?;
                    Some((r.id?, r.rect, handle_at(r.rect, [x, y], reach)?))
                });
                if let Some((id, rect, handle)) = grabbed {
                    app.ui.slices.drag = Some(SliceDrag::Resize { id, handle, start: [x, y], cur: [x, y], rect });
                    return true;
                }
                let hit = slice_at(&doc, x, y);
                app.ui.slices.selected = hit.as_ref().map(|r| r.number);
                // The whole slice moves, also when the press hits one piece of a covered slice.
                if let Some(r) = hit
                    && let Some(sl) = r.id.and_then(|id| doc.slices.get(id)).filter(|_| r.origin == SliceOrigin::User)
                {
                    app.ui.slices.drag = Some(SliceDrag::Move { id: sl.id, start: [x, y], cur: [x, y], rect: sl.rect });
                }
            }
        }
        ToolEvent::Move { x, y, .. } => match &mut app.ui.slices.drag {
            Some(SliceDrag::New { cur, .. }) | Some(SliceDrag::Move { cur, .. }) | Some(SliceDrag::Resize { cur, .. }) => *cur = [x, y],
            None => {}
        },
        ToolEvent::Up { x, y } => match app.ui.slices.drag.take() {
            Some(SliceDrag::New { start, .. }) => {
                let r = new_rect(app, start, [x, y]);
                // A drag of a pixel or two is a click, which selects; a Fixed Size slice is made
                // by a click, at any size.
                let fixed_size = app.ui.tool_options.slice_style == "fixedSize";
                if r.width() >= 1 && r.height() >= 1 && (fixed_size || r.width() > 2 || r.height() > 2) {
                    if let Ok(v) = app.run("slice.new", json!({"rect": [r.x0, r.y0, r.width(), r.height()]})) {
                        app.ui.slices.selected = v.get("number").and_then(Value::as_u64).map(|n| n as usize);
                    }
                } else {
                    app.ui.slices.selected = slice_at(&doc, x, y).map(|r| r.number);
                }
            }
            Some(SliceDrag::Move { id, start, rect, .. }) => {
                let (dx, dy) = ((x - start[0]).round() as i32, (y - start[1]).round() as i32);
                if dx != 0 || dy != 0 {
                    let r = rect.translate(dx, dy);
                    if app.run("slice.set", json!({"slice": id, "rect": [r.x0, r.y0, r.width(), r.height()]})).is_ok() {
                        let d = app.session.active().map(|d| d.doc.clone());
                        app.ui.slices.selected = d.and_then(|d| slices::resolve(&d).into_iter().find(|s| s.id == Some(id)).map(|s| s.number));
                    }
                }
            }
            Some(SliceDrag::Resize { id, handle, start, rect, .. }) => {
                let r = resized(rect, handle, (x - start[0]).round() as i32, (y - start[1]).round() as i32);
                if r != rect && app.run("slice.set", json!({"slice": id, "rect": [r.x0, r.y0, r.width(), r.height()]})).is_ok() {
                    let d = app.session.active().map(|d| d.doc.clone());
                    app.ui.slices.selected = d.and_then(|d| slices::resolve(&d).into_iter().find(|s| s.id == Some(id)).map(|s| s.number));
                }
            }
            None => {}
        },
    }
    true
}

fn hex(s: &str, fallback: Color32) -> Color32 {
    let s = s.trim_start_matches('#');
    let b = |i: usize| s.get(i..i + 2).and_then(|h| u8::from_str_radix(h, 16).ok());
    match (s.len(), b(0), b(2), b(4)) {
        (6, Some(r), Some(g), Some(bl)) => Color32::from_rgb(r, g, bl),
        _ => fallback,
    }
}

/// Slice outlines and numbered badges (View › Show › Slices; always while a slice tool is on).
pub fn draw_overlay(app: &PhotocraftApp, painter: &egui::Painter, xf: &ViewXform) {
    let tool_on = is_slice_tool(app.ui.tool);
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    if !(tool_on || (app.ui.view.shows(app.ui.view.show.slices) && !doc.slices.is_empty())) {
        return;
    }
    let prefs = &app.session.prefs().guides_grid_and_slices;
    let color = hex(&prefs.slice_color, Color32::from_rgb(0x38, 0xb5, 0xff));
    let numbers = prefs.show_slice_numbers;
    let selected_color = Color32::from_rgb(0xff, 0xb0, 0x00);
    let to_screen = |r: photocraft_geom::Rect| Rect::from_two_pos(xf.to_screen(r.x0 as f32, r.y0 as f32), xf.to_screen(r.x1 as f32, r.y1 as f32));
    let all = slices::resolve(&doc);
    // Auto slices first (dotted, underneath), then user and layer slices.
    for pass in [true, false] {
        for r in all.iter().filter(|r| (r.origin == SliceOrigin::Auto) == pass) {
            if pass && (!tool_on || app.ui.slices.hide_auto) {
                continue;
            }
            let sr = to_screen(r.rect);
            let selected = app.ui.slices.selected == Some(r.number);
            if pass {
                let pts = [sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom(), sr.left_top()];
                painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, color.gamma_multiply(0.7)), 3.0, 3.0));
            } else {
                painter.rect_stroke(sr, 0.0, Stroke::new(1.0, color), egui::StrokeKind::Inside);
            }
            if selected && tool_on {
                painter.rect_stroke(sr, 0.0, Stroke::new(2.0, selected_color), egui::StrokeKind::Inside);
                if r.origin == SliceOrigin::User {
                    for c in [
                        sr.left_top(),
                        sr.right_top(),
                        sr.left_bottom(),
                        sr.right_bottom(),
                        sr.center_top(),
                        sr.center_bottom(),
                        sr.left_center(),
                        sr.right_center(),
                    ] {
                        painter.rect_filled(Rect::from_center_size(c, vec2(5.0, 5.0)), 0.0, selected_color);
                    }
                }
            }
            if numbers && sr.width() > 18.0 && sr.height() > 12.0 {
                let label = match r.origin {
                    SliceOrigin::Layer => format!("{:02} ▪", r.number),
                    _ => format!("{:02}", r.number),
                };
                let font = egui::FontId::proportional(10.0);
                let galley = painter.layout_no_wrap(label, font, Color32::WHITE);
                let badge = Rect::from_min_size(sr.left_top() + vec2(2.0, 2.0), galley.size() + vec2(6.0, 2.0));
                let bg = if pass {
                    Color32::from_gray(110)
                } else if selected && tool_on {
                    selected_color
                } else {
                    color
                };
                painter.rect_filled(badge, 2.0, bg);
                painter.galley(badge.min + vec2(3.0, 1.0), galley, Color32::WHITE);
            }
        }
    }
    match &app.ui.slices.drag {
        Some(SliceDrag::New { start, cur }) => {
            let sr = to_screen(new_rect(app, *start, *cur));
            painter.rect_stroke(sr, 0.0, Stroke::new(1.0, Color32::WHITE), egui::StrokeKind::Middle);
            let pts = [sr.left_top(), sr.right_top(), sr.right_bottom(), sr.left_bottom(), sr.left_top()];
            painter.add(egui::Shape::dashed_line(&pts, Stroke::new(1.0, color), 4.0, 4.0));
        }
        Some(SliceDrag::Move { start, cur, rect, .. }) => {
            let r = rect.translate((cur[0] - start[0]).round() as i32, (cur[1] - start[1]).round() as i32);
            painter.rect_stroke(to_screen(r), 0.0, Stroke::new(2.0, selected_color), egui::StrokeKind::Inside);
        }
        Some(SliceDrag::Resize { handle, start, cur, rect, .. }) => {
            let r = resized(*rect, *handle, (cur[0] - start[0]).round() as i32, (cur[1] - start[1]).round() as i32);
            painter.rect_stroke(to_screen(r), 0.0, Stroke::new(2.0, selected_color), egui::StrokeKind::Inside);
        }
        None => {}
    }
}

/// The selected slice's id, promoting an auto slice first when `promote`.
fn selected_id(app: &mut PhotocraftApp, promote: bool) -> Option<u32> {
    let n = app.ui.slices.selected?;
    let doc = app.session.active()?.doc.clone();
    let r = slices::resolve(&doc).into_iter().find(|r| r.number == n)?;
    match r.id {
        Some(id) => Some(id),
        None if promote => app.run("slice.promote", json!({"number": n})).ok()?.get("slice")?.as_u64().map(|v| v as u32),
        None => None,
    }
}

/// Delete or Backspace with no modifiers, with the Slice Select tool and a slice selected,
/// deletes that slice, as in Photoshop (#7862). It never reaches Layer › Delete or Edit › Clear.
/// An auto slice has nothing to delete, and View › Lock Slices refuses with a message.
pub fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    use crate::shortcut_dispatch::Focus;
    // Text fields and widgets that delete with these keys themselves keep them.
    if app.ui.tool != Tool::SliceSelect
        || app.ui.slices.selected.is_none()
        || app.ui.text_edit.is_some()
        || matches!(Focus::of(ctx), Focus::Text | Focus::DeletingWidget)
    {
        return false;
    }
    let mods = ctx.input(|i| i.modifiers);
    if !mods.is_none() || !ctx.input_mut(|i| i.consume_key(mods, egui::Key::Delete) || i.consume_key(mods, egui::Key::Backspace)) {
        return false;
    }
    let result = if app.session.file_menu.slices_locked {
        Err(tl!("Slices are locked (View › Lock Slices)").to_string())
    } else {
        match selected_id(app, false) {
            Some(id) => app.run("slice.delete", json!({"slice": id})).map(|_| app.ui.slices.selected = None),
            // An auto slice is made from the user slices around it: it goes when they do.
            None => Ok(()),
        }
    };
    if let Err(e) = result {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

/// Slice Options dialog for the selected slice (auto slices are promoted on OK).
pub fn open_options(app: &mut PhotocraftApp) -> Result<Value, String> {
    let n = app.ui.slices.selected.ok_or("select a slice with the Slice Select tool")?;
    let st = app.session.active().ok_or("no document")?;
    let r = slices::resolve(&st.doc).into_iter().find(|r| r.number == n).ok_or("no such slice")?;
    let v = photocraft_engine::slice_cmds::resolved_json(&st.doc, &r);
    let mut f = serde_json::Map::new();
    f.insert("__command".into(), json!("slice.set"));
    f.insert("__label".into(), json!("Slice Options"));
    f.insert("__form".into(), json!(true));
    f.insert("__choices".into(), json!({"kind": ["image", "noImage"]}));
    f.insert("number".into(), json!(n));
    f.insert("kind".into(), v["kind"].clone());
    for k in ["name", "url", "target", "alt", "message", "cellText"] {
        f.insert(k.into(), v[k].clone());
    }
    f.insert("background".into(), json!(v["background"].as_str().unwrap_or("none")));
    Ok(json!({"dialog": app.ui.open_dialog(DialogKind::Command, f)}))
}

/// Divide Slice dialog.
fn open_divide(app: &mut PhotocraftApp) -> Result<Value, String> {
    let n = app.ui.slices.selected.ok_or("select a slice first")?;
    let mut f = serde_json::Map::new();
    f.insert("__command".into(), json!("slice.divide"));
    f.insert("__label".into(), json!("Divide Slice"));
    f.insert("__form".into(), json!(true));
    f.insert("__choices".into(), json!({}));
    f.insert("number".into(), json!(n));
    f.insert("horizontal".into(), json!(2));
    f.insert("vertical".into(), json!(1));
    Ok(json!({"dialog": app.ui.open_dialog(DialogKind::Command, f)}))
}

/// Options bars of the Slice and Slice Select tools.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) -> bool {
    if !is_slice_tool(tool) {
        return false;
    }
    let has_doc = app.session.active().is_some();
    let locked = app.session.file_menu.slices_locked;
    let t = crate::theme::Tokens::get(ui.ctx());
    if tool == Tool::Slice {
        // Style, Width and Height, as the marquee's (#8360).
        let o = &mut app.ui.tool_options;
        crate::panels::opt_label(ui, tl!("Style"));
        let styles = [("normal".to_string(), tl!("Normal")), ("fixedRatio".to_string(), tl!("Fixed Ratio")), ("fixedSize".to_string(), tl!("Fixed Size"))];
        if crate::widgets::dropdown(ui, "slice-style", tl!("Style"), &mut o.slice_style, &styles, 96.0) {
            (o.slice_width, o.slice_height) = if o.slice_style == "fixedSize" { (64.0, 64.0) } else { (1.0, 1.0) };
        }
        let fixed_size = o.slice_style == "fixedSize";
        let range = crate::tool_memory::style_range(&o.slice_style);
        // Fixed Size shows whole pixels, the size the slice gets (`new_rect` rounds the same way),
        // however the value was typed, stepped or scrubbed.
        let field = |ui: &mut egui::Ui, v: &mut f32| {
            if fixed_size {
                crate::widgets::value_field_prec(ui, v, range.clone(), "px", 62.0, 0)
            } else {
                crate::widgets::value_field(ui, v, range.clone(), "", 62.0)
            }
        };
        ui.add_enabled_ui(o.slice_style != "normal", |ui| {
            crate::panels::opt_label(ui, tl!("Width"));
            field(ui, &mut o.slice_width);
            if crate::icons::button(ui, "arrow-left-right", 22.0, false, tl!("Swaps height and width")).clicked() {
                std::mem::swap(&mut o.slice_width, &mut o.slice_height);
            }
            crate::panels::opt_label(ui, tl!("Height"));
            field(ui, &mut o.slice_height);
        });
        crate::widgets::vline(ui, 22.0);
        if ui.add_enabled_ui(has_doc && !locked, |ui| crate::widgets::secondary_button(ui, tl!("Slices From Guides"), 0.0)).inner.clicked() {
            let _ = app.run("slice.fromGuides", json!({}));
        }
    } else {
        let sel = app.ui.slices.selected.is_some();
        let stored =
            sel && app.session.active().is_some_and(|d| slices::resolve(&d.doc).iter().any(|r| Some(r.number) == app.ui.slices.selected && r.id.is_some()));
        if ui.add_enabled_ui(sel && !locked, |ui| crate::widgets::secondary_button(ui, tl!("Promote"), 0.0)).inner.clicked() {
            let _ = selected_id(app, true).map(|id| app.run("slice.promote", json!({"slice": id})));
        }
        if ui.add_enabled_ui(sel && !locked, |ui| crate::widgets::secondary_button(ui, tl!("Divide…"), 0.0)).inner.clicked() {
            let _ = open_divide(app);
        }
        if ui.add_enabled_ui(stored && !locked, |ui| crate::widgets::secondary_button(ui, tl!("Delete"), 0.0)).inner.clicked()
            && let Some(id) = selected_id(app, false)
            && app.run("slice.delete", json!({"slice": id})).is_ok()
        {
            app.ui.slices.selected = None;
        }
        crate::widgets::vline(ui, 22.0);
        crate::widgets::checkbox(ui, &mut app.ui.slices.hide_auto, tl!("Hide Auto Slices"));
        crate::widgets::vline(ui, 22.0);
        if ui.add_enabled_ui(sel && !locked, |ui| crate::widgets::secondary_button(ui, tl!("Slice Options…"), 0.0)).inner.clicked() {
            let _ = open_options(app);
        }
    }
    if locked {
        ui.label(egui::RichText::new(tl!("Slices are locked (View › Lock Slices)")).color(t.text_dim).size(11.0));
    }
    true
}

#[cfg(test)]
mod tests;
