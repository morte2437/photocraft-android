use super::overlay::path_lines;
use super::shapes::hex;
use super::*;

// ---------------------------------------------------------------------------------------------
// Paths panel

fn thumb(ui: &egui::Ui, r: Rect, path: &Path, doc: &Document) {
    let t = Tokens::get(ui.ctx());
    // At the document's aspect ratio, like the Channels thumbnails.
    let r = crate::channels_panel::fit_thumb(r, doc.size.width, doc.size.height).0;
    ui.painter().rect_filled(r, 0.0, Color32::from_gray(if t.pro { 222 } else { 240 }));
    let (w, h) = (doc.size.width.max(1) as f64, doc.size.height.max(1) as f64);
    let s = (r.width() as f64 / w).min(r.height() as f64 / h);
    let off = r.center() - vec2((w * s) as f32 / 2.0, (h * s) as f32 / 2.0);
    let map = |q: [f64; 2]| pos2(off.x + (q[0] * s) as f32, off.y + (q[1] * s) as f32);
    for (pts, closed) in path_lines(path, &map) {
        if closed && pts.len() >= 3 {
            ui.painter().add(egui::Shape::closed_line(pts, Stroke::new(1.0, Color32::from_gray(40))));
        } else {
            ui.painter().add(egui::Shape::line(pts, Stroke::new(1.0, Color32::from_gray(40))));
        }
    }
}

/// What a Paths panel row is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathRow {
    Saved,
    Work,
    /// The selected layer's shape path or vector mask (temporary, italic).
    Layer,
}

/// A Paths panel row.
#[derive(Clone, Debug)]
pub struct PathEntry {
    pub name: String,
    pub path: Path,
    pub kind: PathRow,
}

/// The Paths panel's rows, top to bottom: saved paths, the work path, then the selected layer's
/// shape path ("<Layer> Shape Path") or vector mask ("<Layer> Vector Mask"), like Photoshop.
pub fn path_rows(doc: &Document, active: Option<photocraft_doc::LayerId>) -> Vec<PathEntry> {
    let mut rows: Vec<_> = doc.paths.iter().map(|p| PathEntry { name: p.name.clone(), path: p.path.clone(), kind: PathRow::Saved }).collect();
    if let Some(wp) = &doc.work_path {
        rows.push(PathEntry { name: "Work Path".into(), path: wp.clone(), kind: PathRow::Work });
    }
    if let Some(l) = active.and_then(|id| doc.layer(id)) {
        let layer_path = match &l.content {
            LayerContent::Shape(sh) => Some((format!("{} Shape Path", l.name), sh.path.clone())),
            _ => l.vector_mask.as_ref().map(|v| (format!("{} Vector Mask", l.name), v.path.clone())),
        };
        if let Some((name, path)) = layer_path {
            rows.push(PathEntry { name, path, kind: PathRow::Layer });
        }
    }
    rows
}

fn footer_id() -> egui::Id {
    egui::Id::unique("paths-footer")
}

/// A Paths panel row being dragged, for a drop on the trash (#6553): its name and the command
/// that deletes it.
#[derive(Clone, Debug)]
struct PathDrag {
    name: String,
    cmd: &'static str,
    params: Value,
}

fn path_drag_key() -> egui::Id {
    egui::Id::unique("paths-drag")
}

fn ctx_data_footer(ctx: &egui::Context, r: Rect) {
    ctx.data_mut(|d| d.insert_temp(footer_id(), r));
}

/// Where the Paths panel's footer buttons were drawn last frame (the rect around them all).
pub fn paths_footer(ctx: &egui::Context) -> Option<Rect> {
    ctx.data(|d| d.get_temp(footer_id()))
}

/// Photoshop's default name for a saved path: the first free "Path N", counting from the number of
/// saved paths.
fn free_path_name(doc: &Document) -> String {
    let mut n = doc.paths.len().saturating_add(1);
    while doc.paths.iter().any(|p| p.name == format!("Path {n}")) {
        n = n.saturating_add(1);
        if n == usize::MAX {
            break;
        }
    }
    format!("Path {n}")
}

/// A Paths panel name being typed in place: the row's key ("work" or the saved path's name) and
/// the text so far. It lives in egui's temp data, like the Channels panel's rename field.
#[derive(Clone)]
struct PathRename {
    key: String,
    text: String,
    focused: bool,
}

fn rename_id(doc: &Document) -> egui::Id {
    egui::Id::unique(("path-rename", doc.id.0))
}

/// Commands offered by a Paths row's context menu. The row's path is passed explicitly so a
/// right-click acts on that row even when another path is selected in the panel. `layer` is the
/// layer a `PathRow::Layer` row belongs to (the active layer).
pub(super) fn path_context_actions(entry: &PathEntry, doc: &Document, layer: Option<photocraft_doc::LayerId>) -> Vec<(&'static str, &'static str, Value)> {
    let key = match entry.kind {
        PathRow::Work => "work".to_string(),
        PathRow::Layer => "layer".to_string(),
        PathRow::Saved => entry.name.clone(),
    };
    let mut actions = vec![
        ("Make Selection", "path.toSelection", json!({"name": key})),
        ("Fill Path", "path.fill", json!({"name": key})),
        ("Stroke Path…", "path.stroke", json!({"name": key})),
    ];
    if entry.kind == PathRow::Work {
        actions.push(("Save Path", "path.rename", json!({"name": "work", "to": free_path_name(doc)})));
    } else {
        let mut n = 1usize;
        while doc.paths.iter().any(|p| p.name == format!("{} copy {n}", entry.name)) {
            n = n.saturating_add(1);
            if n == usize::MAX {
                break;
            }
        }
        let copy_name = format!("{} copy {n}", entry.name);
        actions.push(("Duplicate Path", "path.set", json!({"name": copy_name, "path": photocraft_engine::vector_cmds::path_json(&entry.path)})));
    }
    if entry.kind == PathRow::Saved {
        // No `to`: the panel asks for the new name in place.
        actions.push(("Rename Path…", "path.rename", json!({"name": key})));
    }
    if entry.kind != PathRow::Layer {
        actions.push(("Delete Path", "path.delete", json!({"name": key})));
    } else if entry.name.ends_with(" Vector Mask") {
        actions.push(("Delete Vector Mask", "layer.vectorMask.delete", json!({})));
    } else if let Some(id) = layer {
        // A shape layer's path is the shape: Photoshop's Delete Path deletes the layer (#6552).
        actions.push(("Delete Path", "layer.delete", json!({"layer": id.0})));
    }
    actions
}

pub fn paths_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        ui.label(egui::RichText::new(tl!("No document")).color(t.text_faint));
        return;
    };
    let doc = st.doc.clone();
    let st_layer = st.active_layer;
    let rows = path_rows(&doc, st_layer);
    let has_layer_path = rows.iter().any(|r| r.kind == PathRow::Layer);
    let mut action: Option<(&str, Value)> = None;
    let mut stroke_dialog: Option<String> = None;
    // A row dragged onto the trash is deleted (#6553); a drag that never saw its release must not
    // outlive it.
    let ctx = ui.ctx().clone();
    if !ctx.input(|i| i.pointer.primary_down() || i.pointer.any_released()) {
        ctx.data_mut(|d| d.remove::<PathDrag>(path_drag_key()));
    }
    let drag = ctx.data(|d| d.get_temp::<PathDrag>(path_drag_key()));
    let rename_id = rename_id(&doc);
    // The buttons sit in a footer at the panel's bottom, like Photoshop's.
    let footer = crate::widgets::footer_height(ui) + ui.spacing().item_spacing.y;
    let fill = ui.available_height() > footer + 60.0;
    let rows_h = if fill { ui.available_height() - footer } else { f32::INFINITY };
    egui::ScrollArea::vertical().id_salt("path-rows").max_height(rows_h).min_scrolled_height(if fill { rows_h } else { 0.0 }).auto_shrink([false, !fill]).show(
        ui,
        |ui| {
            if rows.is_empty() {
                ui.label(egui::RichText::new(tl!("Draw with the Pen tool (P) or make a work path from a selection.")).color(t.text_faint).size(11.5));
            }
            for PathEntry { name, path, kind } in &rows {
                // What dropping the row on the trash runs; a shape layer's path is not dragged.
                let delete = match kind {
                    PathRow::Work => Some(("path.delete", json!({"name": "work"}))),
                    PathRow::Saved => Some(("path.delete", json!({"name": name}))),
                    PathRow::Layer if name.ends_with(" Vector Mask") => Some(("layer.vectorMask.delete", json!({}))),
                    PathRow::Layer => None,
                };
                let sense = if delete.is_some() { Sense::click_and_drag() } else { Sense::click() };
                let (r, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 36.0), sense);
                let key = match kind {
                    PathRow::Work => "work".to_string(),
                    PathRow::Layer => "layer".to_string(),
                    PathRow::Saved => name.clone(),
                };
                if resp.drag_started()
                    && let Some((cmd, params)) = delete
                {
                    let name = if *kind == PathRow::Work { tl!(name).to_string() } else { name.clone() };
                    ui.ctx().data_mut(|d| d.insert_temp(path_drag_key(), PathDrag { name, cmd, params }));
                }
                // Rows are painted: name them for screen readers and UI tests.
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name));
                let sel = app.ui.selected_path.as_deref() == Some(key.as_str());
                if sel {
                    ui.painter().rect_filled(r, 0.0, t.row_selected);
                } else if resp.hovered() {
                    ui.painter().rect_filled(r, 0.0, t.hover.gamma_multiply(0.5));
                }
                let thumbnail = Rect::from_min_size(pos2(r.left() + 8.0, r.top() + 4.0), vec2(28.0, 28.0));
                thumb(ui, thumbnail, path, &doc);
                let mut job = egui::text::LayoutJob::default();
                // Temporary paths (the work path, the selected layer's shape path or vector mask) are italic.
                let italics = *kind != PathRow::Saved;
                job.append(
                    if *kind == PathRow::Work { tl!(name) } else { name },
                    0.0,
                    egui::TextFormat { font_id: egui::FontId::proportional(12.0), color: t.text, italics, ..Default::default() },
                );
                let renaming = ui.ctx().data(|d| d.get_temp::<PathRename>(rename_id)).filter(|e| e.key == key);
                if let Some(mut edit) = renaming {
                    let field = Rect::from_min_max(pos2(r.left() + 43.0, r.center().y - 11.0), pos2(r.right() - 8.0, r.center().y + 11.0));
                    let id = rename_id.with("field");
                    let te =
                        ui.put(field, egui::TextEdit::singleline(&mut edit.text).id(id).font(egui::FontId::proportional(12.0))).accessible_name(tl!("Name"));
                    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                    if !edit.focused {
                        te.request_focus();
                        // Like Photoshop, the whole name starts selected so typing replaces it.
                        if let Some(mut state) = egui::TextEdit::load_state(ui.ctx(), id) {
                            let all = egui::text::CCursorRange::two(egui::text::CCursor::new(0), egui::text::CCursor::new(edit.text.chars().count()));
                            state.cursor.set_char_range(Some(all));
                            state.store(ui.ctx(), id);
                        }
                        edit.focused = true;
                        ui.ctx().data_mut(|d| d.insert_temp(rename_id, edit));
                    } else if esc {
                        ui.ctx().data_mut(|d| d.remove::<PathRename>(rename_id));
                    } else if enter || te.lost_focus() || !te.has_focus() {
                        ui.ctx().data_mut(|d| d.remove::<PathRename>(rename_id));
                        let to = edit.text.trim();
                        // The Work Path is saved even under its proposed name; an unchanged saved name is left alone.
                        if !to.is_empty() && (*kind == PathRow::Work || to != name) {
                            action = Some(("path.rename", json!({"name": key, "to": to})));
                        }
                    } else {
                        ui.ctx().data_mut(|d| d.insert_temp(rename_id, edit));
                    }
                } else {
                    let g = ui.painter().layout_job(job);
                    ui.painter().galley(pos2(r.left() + 46.0, r.center().y - g.size().y / 2.0), g, t.text);
                }
                // Ctrl/Cmd-click loads only the thumbnail's path, without changing the panel target.
                let modifiers = ui.input(|i| i.modifiers);
                let load = resp.clicked() && modifiers.command && resp.interact_pointer_pos().is_some_and(|p| thumbnail.contains(p));
                if load {
                    let mode = match crate::channels_panel::load_operation(modifiers) {
                        "new" => "replace",
                        mode => mode,
                    };
                    action = Some(("path.toSelection", json!({"name": key, "mode": mode})));
                } else if resp.clicked() {
                    app.ui.selected_path = Some(key.clone());
                }
                // As in Photoshop, double-clicking a saved path's name renames it, and double-clicking
                // the Work Path asks for the name to save it under (#6238).
                let proposed = match kind {
                    PathRow::Work => Some(free_path_name(&doc)),
                    PathRow::Saved => Some(name.clone()),
                    PathRow::Layer => None,
                };
                if !load
                    && resp.double_clicked()
                    && let Some(text) = proposed
                {
                    ui.ctx().data_mut(|d| d.insert_temp(rename_id, PathRename { key: key.clone(), text, focused: false }));
                }
                resp.context_menu(|ui| {
                    crate::widgets::menu_scroll(ui, |ui| {
                        ui.set_min_width(190.0);
                        let entry = PathEntry { name: name.clone(), path: path.clone(), kind: *kind };
                        let can_paint = app.session.active().and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id))).is_some_and(|l| l.surface().is_some());
                        for (label, cmd, params) in path_context_actions(&entry, &doc, st_layer) {
                            let enabled = !matches!(cmd, "path.fill" | "path.stroke") || can_paint;
                            if ui.add_enabled(enabled, egui::Button::new(tl!(label))).clicked() {
                                if cmd == "path.stroke" {
                                    // Stroke Path… asks for the tool first, as in Photoshop (#2978).
                                    stroke_dialog = Some(params.get("name").and_then(Value::as_str).unwrap_or("work").to_string());
                                } else if cmd == "path.rename" && params.get("to").is_none() {
                                    let edit = PathRename { key: key.clone(), text: name.clone(), focused: false };
                                    ui.ctx().data_mut(|d| d.insert_temp(rename_id, edit));
                                } else {
                                    action = Some((cmd, params));
                                }
                                ui.close();
                            }
                        }
                    });
                });
                ui.painter().line_segment([r.left_bottom(), r.right_bottom()], Stroke::new(1.0, t.separator));
            }
            // Clicking the empty area deselects the path, as in Photoshop: new fill and
            // adjustment layers then take the selection as their mask instead (#1419).
            let rest = ui.available_rect_before_wrap();
            let rest = rest.intersect(Rect::from_min_size(rest.min, vec2(rest.width(), rest.height().min(4000.0))));
            if rest.is_positive() && ui.interact(rest, ui.scope_id().with("paths-empty"), Sense::click()).clicked() {
                app.ui.selected_path = None;
            }
        },
    );
    // The dragged row follows the cursor.
    if let Some(drag) = &drag
        && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
    {
        crate::layer_transfer::ghost(&ctx, p, &drag.name);
    }
    let sel = app.ui.selected_path.clone().filter(|s| s != "layer" || has_layer_path).unwrap_or_else(|| "work".into());
    let buttons = crate::widgets::panel_footer(ui, |ui| {
        let fg = hex(app.session.tools.foreground);
        let mut n = doc.paths.len() + 1;
        while doc.paths.iter().any(|p| p.name == format!("Path {n}")) {
            n += 1;
        }
        let items: [(&str, &str, &str, Value); 7] = [
            ("paint-bucket", "Fill path with foreground color", "path.fill", json!({"name": sel, "color": fg})),
            ("circle", "Stroke path with brush", "path.stroke", json!({"name": sel, "tool": "brush"})),
            ("square-dashed", "Load path as a selection", "path.toSelection", json!({"name": sel})),
            ("spline", "Make work path from selection", "select.toWorkPath", json!({"tolerance": 2.0})),
            ("square", "Add vector mask", "layer.vectorMask.fromPath", json!({"name": sel})),
            ("square-plus", "Create new path", "path.set", json!({"name": format!("Path {n}"), "path": {"subpaths": []}})),
            ("trash", "Delete path", "path.delete", json!({"name": sel})),
        ];
        // The layer's own path: deleting it deletes the vector mask (a shape keeps its path).
        let shape = rows.iter().any(|r| r.kind == PathRow::Layer && r.name.ends_with(" Shape Path"));
        let delete_layer_path = sel == "layer" && !shape;
        let items = items.map(|(icon, tip, cmd, p)| {
            if cmd == "path.delete" && delete_layer_path { (icon, "Delete vector mask", "layer.vectorMask.delete", json!({})) } else { (icon, tip, cmd, p) }
        });
        // Laid out from the right, so the last button goes first.
        let mut buttons = Rect::NOTHING;
        for (icon, tip, cmd, p) in items.into_iter().rev() {
            let icon = if crate::icons::exists(icon) { icon } else { "square" };
            let b = crate::icons::button(ui, icon, 26.0, false, tip);
            if b.clicked() {
                if cmd == "path.stroke" && ui.input(|i| i.modifiers.alt) {
                    // Alt-click opens the Stroke Path dialog, as in Photoshop (#2978).
                    stroke_dialog = Some(sel.clone());
                } else {
                    action = Some((cmd, p));
                }
            }
            // A row dropped on the trash deletes that row's path, selected or not (#6553).
            if matches!(cmd, "path.delete" | "layer.vectorMask.delete")
                && let Some(drag) = &drag
                && crate::panels::dropped_on(ui, &b)
            {
                action = Some((drag.cmd, drag.params.clone()));
            }
            buttons = buttons.union(b.rect);
        }
        buttons
    });
    ctx_data_footer(ui.ctx(), buttons);
    if ctx.input(|i| i.pointer.any_released()) {
        ctx.data_mut(|d| d.remove::<PathDrag>(path_drag_key()));
    }
    if let Some(name) = stroke_dialog {
        crate::canvas_tool_menu::stroke_path_dialog(app, &name);
    }
    if let Some((cmd, p)) = action {
        let renamed = if cmd == "path.rename" { p.get("to").and_then(Value::as_str).map(str::to_string) } else { None };
        match app.run(cmd, p) {
            // The renamed (or just saved) path stays the panel's target.
            Ok(_) => {
                if renamed.is_some() {
                    app.ui.selected_path = renamed;
                }
            }
            Err(e) => {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
    }
}
