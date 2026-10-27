use super::*;

/// Print's Printer menu: the default printer (value ""), then every printer `file.printers`
/// listed. A saved name the spooler no longer lists stays selectable rather than vanishing.
/// CUPS queue names stand in for spaces with `_`; the labels show the spaces.
pub(super) fn printer_options(list: Option<&Value>, current: &str) -> Vec<(String, String)> {
    let printers: Vec<(&str, bool)> = list
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(|p| Some((p.get("name")?.as_str()?, p.get("default").and_then(Value::as_bool).unwrap_or(false)))).collect())
        .unwrap_or_default();
    let pretty = |n: &str| n.replace('_', " ");
    let default = match printers.iter().find(|p| p.1) {
        Some((n, _)) => format!("{} ({})", tl!("Default printer"), pretty(n)),
        None => tl!("Default printer").to_string(),
    };
    let mut opts = vec![(String::new(), default)];
    opts.extend(printers.iter().map(|(n, _)| (n.to_string(), pretty(n))));
    if !current.is_empty() && !printers.iter().any(|p| p.0 == current) {
        opts.push((current.to_string(), pretty(current)));
    }
    opts
}

// ---------- Print ----------

/// Opens a Print dialog seeded from the last print, filling missing fields with defaults.
/// Refreshes the printer list on each opening; when native driver enumeration succeeds,
/// an unavailable saved printer is replaced and its stale driver settings are discarded.
/// Returns the dialog ID without submitting a print job.
pub fn open_print(app: &mut PhotocraftApp) -> u64 {
    let mut f = Map::new();
    f.insert("__print".into(), json!(true));
    f.insert("__label".into(), json!("PhotoCraft Print Settings"));
    // Asked once per opening: the Printer menu lists what the spooler knows now.
    let printers = app.session.execute("file.printers", json!({})).unwrap_or_default();
    f.insert("__systemPrinters".into(), printers["printers"].clone());
    // The settings of the last print, also from an earlier session (#7410).
    if let Some(Value::Object(m)) = photocraft_engine::print_cmds::last_print(&app.session) {
        f.extend(m);
    }
    for (k, v) in [
        ("printer", json!("")),
        ("copies", json!(1)),
        ("paper", json!("letter")),
        ("orientation", json!("portrait")),
        ("colorHandling", json!("printerManages")),
        ("printerProfile", json!("coated-cmyk")),
        ("intent", json!("relative")),
        ("bpc", json!(true)),
        ("center", json!(true)),
        ("top", json!(0.0)),
        ("left", json!(0.0)),
        ("scale", json!(100.0)),
        ("scaleToFit", json!(false)),
        ("cornerCropMarks", json!(false)),
        ("centerCropMarks", json!(false)),
        ("registrationMarks", json!(false)),
        ("description", json!(false)),
        ("labels", json!(false)),
        ("emulsionDown", json!(false)),
        ("units", json!("cm")),
        ("output", json!("")),
    ] {
        f.entry(k.to_string()).or_insert(v);
    }
    // Windows: the system's printers, and the paper from the chosen printer's driver settings.
    if let Ok(list) = craft_print::rgb::printers() {
        f.insert("__printers".into(), json!(list.iter().map(|p| p.name.clone()).collect::<Vec<_>>()));
        let known = |n: &str| list.iter().any(|p| p.name == n);
        if !known(&s(&f, "printer", "")) {
            let fallback = list.iter().find(|p| p.default).or(list.first()).map(|p| p.name.clone()).unwrap_or_default();
            f.insert("printer".into(), json!(fallback));
            f.remove("printerSettings");
        }
    }
    app.ui.open_dialog(DialogKind::Command, f)
}

/// Whether the driver's paper, as `driver_page` took it, is wider than tall: the Orientation
/// buttons show landscape then.
pub(super) fn driver_paper_is_wide(f: &Map<String, Value>) -> bool {
    let side = |i: usize| f.get("paper").and_then(Value::as_array).and_then(|a| a.get(i)).and_then(Value::as_f64);
    matches!((side(0), side(1)), (Some(w), Some(h)) if w > h)
}

/// The Orientation buttons: the driver settings (or the printer's own) turned portrait or
/// landscape through `file.print.page`. An error stays on the dialog, with the settings left as
/// they were.
pub(super) fn turn_driver(app: &mut PhotocraftApp, f: &mut Map<String, Value>, landscape: bool) {
    let mut q = json!({"printer": s(f, "printer", ""), "orientation": if landscape { "landscape" } else { "portrait" }});
    if let (Some(h), Some(o)) = (f.get("printerSettings").cloned(), q.as_object_mut()) {
        o.insert("printerSettings".into(), h);
    }
    match app.session.execute("file.print.page", q) {
        Ok(r) => take_turned_page(f, &r),
        Err(e) => {
            f.insert("__pageError".into(), json!(e.to_string()));
        }
    }
}

/// The turned settings and the paper they give, both from the one `file.print.page` answer, as
/// `driver_page` takes a page: asking the driver again could fail and leave new settings with the
/// old paper. An answer without both leaves the dialog as it was, with the reason shown.
pub(super) fn take_turned_page(f: &mut Map<String, Value>, r: &Value) {
    let paper = r.get("paper").and_then(Value::as_array).and_then(|a| Some((a.first()?.as_f64()?, a.get(1)?.as_f64()?)));
    let (Some(settings), Some((w, h))) = (r.get("printerSettings").and_then(Value::as_str), paper) else {
        f.insert("__pageError".into(), json!("the printer did not report the turned page"));
        return;
    };
    f.insert("printerSettings".into(), json!(settings));
    f.insert("paper".into(), json!([w, h]));
    f.insert("orientation".into(), json!("portrait"));
    f.remove("__pageError");
}

/// Takes the paper from the printer's driver settings (Windows), as Photoshop does.
fn driver_page(f: &mut Map<String, Value>) {
    let printer = s(f, "printer", "");
    let settings = f.get("printerSettings").and_then(Value::as_str).and_then(|h| craft_print::rgb::DevMode::from_hex(h).ok());
    match craft_print::rgb::page(&printer, settings.as_ref()) {
        Ok(pg) => {
            f.insert("paper".into(), json!([pg.paper.0, pg.paper.1]));
            f.insert("printable".into(), json!([pg.printable.0, pg.printable.1, pg.printable.2, pg.printable.3]));
            f.insert("orientation".into(), json!("portrait"));
            f.remove("__pageError");
        }
        Err(e) => {
            f.remove("printable");
            f.insert("__pageError".into(), json!(e.to_string()));
        }
    }
}

/// Points per unit for the Position and Size fields.
fn unit_pt(units: &str) -> f64 {
    match units {
        "mm" => 72.0 / 25.4,
        "in" => 72.0,
        _ => 72.0 / 2.54,
    }
}

/// A number field in `units` for a value kept in points; true when edited.
fn length_field(ui: &mut egui::Ui, label: &str, pt: &mut f64, units: &str) -> bool {
    length_field_prec(ui, label, pt, units, 0)
}

/// Edits a point-valued length in the selected units, with the requested decimal precision.
/// Retains the nonnegative 0..=10000 display-unit range used by existing size fields.
fn length_field_prec(ui: &mut egui::Ui, label: &str, pt: &mut f64, units: &str, decimals: u32) -> bool {
    length_field_range(ui, label, pt, units, decimals, 0.0..=10000.0)
}

/// Edits a length stored in print points (72 per inch), returning true for a finite edit.
/// `range` is in the displayed `units`; a signed range permits negative Top/Left offsets.
fn length_field_range(ui: &mut egui::Ui, label: &str, pt: &mut f64, units: &str, decimals: u32, range: std::ops::RangeInclusive<f32>) -> bool {
    let t = Tokens::get(ui.ctx());
    let k = unit_pt(units);
    let mut v = (*pt / k) as f32;
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).color(t.text_dim));
        let before = v;
        crate::widgets::value_field_prec(ui, &mut v, range, units, 70.0, decimals).accessible_name(tl!(&label));
        if (v - before).abs() > 1e-4 && v.is_finite() {
            *pt = f64::from(v) * k;
            changed = true;
        }
    });
    changed
}

/// Unrotated paper dimensions in points, for either the current preset or a custom array.
fn print_paper_size(value: Option<&Value>) -> (f64, f64) {
    let default = (612.0, 792.0);
    match value {
        Some(Value::Array(a)) => (a.first().and_then(Value::as_f64).unwrap_or(default.0), a.get(1).and_then(Value::as_f64).unwrap_or(default.1)),
        Some(Value::String(name)) => photocraft_engine::print_cmds::PAPERS.iter().find(|p| p.0.eq_ignore_ascii_case(name)).map_or(default, |p| (p.1, p.2)),
        _ => default,
    }
}

/// Edits the paper preset, orientation and custom dimensions, remembering custom sizes
/// across preset switches. Dimensions are displayed in the selected units but stored as
/// unrotated print points; the engine applies orientation and validates the final size.
fn print_paper_controls(ui: &mut egui::Ui, f: &mut Map<String, Value>) {
    let previous = f.get("paper").cloned();
    let mut selected = if previous.as_ref().is_some_and(Value::is_array) { "custom".to_string() } else { s(f, "paper", "letter") };
    ui.horizontal(|ui| {
        let mut papers: Vec<_> = photocraft_engine::print_cmds::PAPERS.iter().map(|p| (p.0.to_string(), p.0)).collect();
        papers.push(("custom".to_string(), tl!("Custom")));
        if crate::widgets::dropdown(ui, "print-paper", tl!("Paper"), &mut selected, &papers, 110.0) {
            if selected == "custom" {
                let (w, h) = print_paper_size(f.get("__customPaper").or(previous.as_ref()));
                f.insert("paper".into(), json!([w, h]));
            } else {
                if let Some(paper @ Value::Array(_)) = previous {
                    f.insert("__customPaper".into(), paper);
                }
                f.insert("paper".into(), json!(selected));
            }
        }
        dropdown_str(ui, "print-orient", tl!("Orientation"), f, "orientation", &[("portrait", tl!("Portrait")), ("landscape", tl!("Landscape"))], 110.0);
    });
    if f.get("paper").is_some_and(Value::is_array) {
        let (mut w, mut h) = print_paper_size(f.get("paper"));
        let units = s(f, "units", "cm");
        let changed = ui
            .horizontal(|ui| {
                let wc = length_field_prec(ui, tl!("Paper Width"), &mut w, &units, 2);
                let hc = length_field_prec(ui, tl!("Paper Height"), &mut h, &units, 2);
                wc || hc
            })
            .inner;
        if changed {
            // Keep points in the command, and let the engine apply orientation and validation.
            f.insert("paper".into(), json!([w, h]));
            f.insert("__customPaper".into(), json!([w, h]));
        }
    }
}

/// Save as PDF › Browse…: where to write the PDF, into Print dialog `id` once the file dialog
/// answers. Without it the only way to save a PDF was to type a full path (#5328).
fn browse_pdf(app: &mut PhotocraftApp, id: u64, suggested: &str) {
    let r = app.pick_save(suggested, move |app, path| {
        let d = app.ui.dialog_mut(id).ok_or("the dialog was closed")?;
        d.fields.insert("output".into(), json!(path));
        Ok(Value::Null)
    });
    if let Err(e) = r
        && e != crate::file_dialog::CANCELLED
    {
        app.ui.status = e;
    }
}

/// Converts total displacement from a fixed drag-start layout into manual inch offsets.
/// Resolving Center before the drag avoids a jump; each update replaces the offsets rather
/// than accumulating movement. Emulsion Down reverses horizontal layout-space movement.
///
/// `delta` and the painter use egui points, including on Retina displays; the scale is egui
/// points per print point (1/72 inch). Applying pixels_per_point would count HiDPI twice.
/// Invalid scales or nonfinite displacement/offsets return false without changing fields.
/// Accepted offsets are clamped to the engine's safety bound and Center is disabled.
pub(super) fn drag_print_position(
    f: &mut Map<String, Value>,
    start: photocraft_engine::print_cmds::PrintLayout,
    delta: egui::Vec2,
    points_per_print_point: f32,
) -> bool {
    if !points_per_print_point.is_finite() || points_per_print_point <= 0.0 || !delta.is_finite() {
        return false;
    }
    let (x, y, _, h) = start.rect;
    let direction = if start.emulsion_down { -1.0 } else { 1.0 };
    let left = (x + direction * f64::from(delta.x) / f64::from(points_per_print_point)) / 72.0;
    let top = (start.paper.1 - y - h + f64::from(delta.y) / f64::from(points_per_print_point)) / 72.0;
    if !left.is_finite() || !top.is_finite() {
        return false;
    }
    let limit = photocraft_engine::print_cmds::MAX_POSITION_INCHES;
    f.insert("center".into(), json!(false));
    f.insert("left".into(), json!(left.clamp(-limit, limit)));
    f.insert("top".into(), json!(top.clamp(-limit, limit)));
    true
}

/// Maps the engine's bottom-left print-point rectangle into top-left egui coordinates.
/// `k` is egui points per print point; Emulsion Down mirrors placement across the page,
/// matching the PDF page transform. Returns the full image bounds for the caller to clip.
pub(super) fn print_preview_image(l: photocraft_engine::print_cmds::PrintLayout, page: egui::Rect, k: f32) -> egui::Rect {
    let (x, y, w, h) = l.rect;
    let x = if l.emulsion_down { l.paper.0 - x - w } else { x };
    egui::Rect::from_min_size(egui::pos2(page.left() + x as f32 * k, page.bottom() - (y + h) as f32 * k), egui::vec2(w as f32 * k, h as f32 * k))
}

/// Draws the print controls and paper-clipped preview using the engine's shared layout.
/// Dragging the visible image updates dialog offsets live from a fixed gesture origin;
/// the position row stays present while centered so switching to manual cannot move the page.
pub(super) fn print_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, f: &mut Map<String, Value>, id: u64) {
    let t = Tokens::get(ui.ctx());
    let Some(doc) = app.session.active().map(|d| d.doc.clone()) else { return };
    // PDF exports and previews retain editable paper on Windows too. Only a real native
    // submission takes its media from the printer; entering that mode refreshes the driver
    // geometry after any export-only paper edits.
    let printers: Option<Vec<String>> = if !b(f, "dryRun", false) && s(f, "output", "").is_empty() {
        f.get("__printers").and_then(Value::as_array).map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
    } else {
        None
    };
    if printers.is_some() {
        if !b(f, "__driverPageInitialized", false) {
            driver_page(f);
            f.insert("__driverPageInitialized".into(), json!(true));
        }
    } else {
        // PDF output and previews have no driver page: Scale to Fit Media uses the paper.
        f.remove("__driverPageInitialized");
        f.remove("printable");
    }
    let p = params(f);
    let mut browse = false;
    ui.horizontal_top(|ui| {
        // Page preview.
        ui.vertical(|ui| {
            let (area, _) = ui.allocate_exact_size(egui::vec2(330.0, 400.0), egui::Sense::hover());
            ui.painter().rect_filled(area, 0.0, t.canvas);
            match photocraft_engine::print_cmds::layout(&doc, &p, "file.print") {
                Ok(mut l) => {
                    let (pw, ph) = l.paper;
                    let k = ((area.width() - 20.0) / pw as f32).min((area.height() - 20.0) / ph as f32);
                    let page = egui::Rect::from_center_size(area.center(), egui::vec2(pw as f32 * k, ph as f32 * k));
                    ui.painter().rect_filled(page, 0.0, egui::Color32::WHITE);
                    ui.painter().rect_stroke(page, 0.0, egui::Stroke::new(1.0, t.separator), egui::StrokeKind::Outside);
                    let mut img = print_preview_image(l, page, k);
                    let drag_id = ui.scope_id().with(("print-position", id));
                    let visible = img.intersect(page).intersect(ui.clip_rect());
                    let response = ui.interact(visible, drag_id, egui::Sense::drag()).on_hover_cursor(egui::CursorIcon::Grab);
                    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, tl!("Print preview image")));
                    // Keep a fixed layout and press origin for the whole gesture: rounding and
                    // the transition from Center must never accumulate or move the grab point.
                    if response.drag_started_by(egui::PointerButton::Primary)
                        && let Some(origin) = ui.input(|i| i.pointer.press_origin())
                    {
                        ui.data_mut(|d| d.insert_temp(drag_id, (l, origin, k)));
                    }
                    if (response.dragged_by(egui::PointerButton::Primary) || response.drag_stopped_by(egui::PointerButton::Primary))
                        && let Some((start, origin, scale)) = ui.data(|d| d.get_temp::<(photocraft_engine::print_cmds::PrintLayout, egui::Pos2, f32)>(drag_id))
                        && let Some(pointer) = response.interact_pointer_pos()
                        && drag_print_position(f, start, pointer - origin, scale)
                    {
                        if let Ok(updated) = photocraft_engine::print_cmds::layout(&doc, &params(f), "file.print") {
                            l = updated;
                            img = print_preview_image(l, page, k);
                        }
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
                    }
                    if !ui.input(|i| i.pointer.primary_down()) {
                        ui.data_mut(|d| d.remove::<(photocraft_engine::print_cmds::PrintLayout, egui::Pos2, f32)>(drag_id));
                    }
                    let (x, y, w, h) = l.rect;
                    let key = egui::Id::unique(("print-thumb", doc.id.0, app.session.active().map_or(0, |d| d.revision)));
                    let tex: Option<Arc<egui::TextureHandle>> = ui.data(|d| d.get_temp(key));
                    let tex = tex.unwrap_or_else(|| {
                        let th = photocraft_compose::thumbnail(&doc, 400);
                        let ci = egui::ColorImage::from_rgba_unmultiplied([th.width as usize, th.height as usize], &th.pixels);
                        let tx = Arc::new(ui.ctx().load_texture("print-thumb", ci, egui::TextureOptions::LINEAR));
                        ui.data_mut(|d| d.insert_temp(key, tx.clone()));
                        tx
                    });
                    let clip = ui.painter().with_clip_rect(page);
                    let uv = if l.emulsion_down {
                        egui::Rect::from_min_max(egui::pos2(1.0, 0.0), egui::pos2(0.0, 1.0))
                    } else {
                        egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0))
                    };
                    clip.image(tex.id(), img, uv, egui::Color32::WHITE);
                    let mark = egui::Stroke::new(1.0, egui::Color32::BLACK);
                    if l.marks.corner_crop {
                        // Bleed puts the marks on the trim line, inside the image edge.
                        let b = l.marks.bleed as f32 * k;
                        for (c, dx, dy) in
                            [(img.left_top(), -1.0, -1.0), (img.right_top(), 1.0, -1.0), (img.left_bottom(), -1.0, 1.0), (img.right_bottom(), 1.0, 1.0)]
                        {
                            clip.line_segment([c + egui::vec2(dx * 3.0, -dy * b), c + egui::vec2(dx * 12.0, -dy * b)], mark);
                            clip.line_segment([c + egui::vec2(-dx * b, dy * 3.0), c + egui::vec2(-dx * b, dy * 12.0)], mark);
                        }
                    }
                    if l.marks.registration {
                        for c in [
                            img.center_top() - egui::vec2(0.0, 9.0),
                            img.center_bottom() + egui::vec2(0.0, 9.0),
                            img.left_center() - egui::vec2(9.0, 0.0),
                            img.right_center() + egui::vec2(9.0, 0.0),
                        ] {
                            clip.circle_stroke(c, 3.5, mark);
                        }
                    }
                    let fits = x >= 0.0 && y >= 0.0 && x + w <= pw && y + h <= ph;
                    let note = if fits { String::new() } else { "  ·  larger than the paper".to_string() };
                    ui.label(
                        egui::RichText::new(format!("Scale {:.1}%  ·  {:.2} × {:.2} in{note}", l.scale * 100.0, w / 72.0, h / 72.0))
                            .color(if fits { t.text_dim } else { t.warning })
                            .size(11.0),
                    );
                }
                Err(e) => {
                    ui.painter().text(area.center(), egui::Align2::CENTER_CENTER, e.to_string(), egui::FontId::proportional(12.0), t.text_dim);
                }
            }
        });
        ui.add_space(12.0);
        ui.vertical(|ui| {
            ui.set_width(340.0);
            let head = |ui: &mut egui::Ui, s: &str| {
                ui.add_space(4.0);
                ui.label(egui::RichText::new(s).font(crate::theme::semibold(12.0)).color(t.text));
            };
            head(ui, tl!("Printer Setup"));
            if let Some(printers) = &printers {
                // Windows: Photoshop's Printer menu and Print Settings… (the driver's own dialog).
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Printer")).color(t.text_dim));
                    let before = s(f, "printer", "");
                    let opts: Vec<(&str, &str)> = printers.iter().map(|n| (n.as_str(), n.as_str())).collect();
                    dropdown_str(ui, "print-printer", tl!("Printer"), f, "printer", &opts, 220.0);
                    if s(f, "printer", "") != before {
                        f.remove("printerSettings");
                        driver_page(f);
                    }
                });
                ui.horizontal(|ui| {
                    number(ui, f, "copies", tl!("Copies"), 1.0..=999.0, "", 1.0);
                    if ui.button(tl!("Print Settings…")).clicked() {
                        let printer = s(f, "printer", "");
                        let current = f.get("printerSettings").and_then(Value::as_str).and_then(|h| craft_print::rgb::DevMode::from_hex(h).ok());
                        match craft_print::rgb::settings_dialog(&printer, current.as_ref(), None) {
                            Ok(Some(dm)) => {
                                f.insert("printerSettings".into(), json!(dm.to_hex()));
                                driver_page(f);
                            }
                            Ok(None) => {}
                            Err(e) => {
                                f.insert("__pageError".into(), json!(e.to_string()));
                            }
                        }
                    }
                });
                // Photoshop's Layout: Portrait / Landscape turn the driver settings, so the paper
                // and the preview turn with them without opening Print Settings….
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Orientation")).color(t.text_dim));
                    let wide = driver_paper_is_wide(f);
                    for (landscape, label) in [(false, "Portrait"), (true, "Landscape")] {
                        if crate::widgets::pill_tab(ui, label, wide == landscape).clicked() && wide != landscape {
                            turn_driver(app, f, landscape);
                        }
                    }
                });
                let paper = p.get("paper").and_then(Value::as_array).map(|a| (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64)));
                let text = match paper {
                    Some((Some(w), Some(h))) => format!("{:.0} × {:.0} mm  ·  {}", w / 72.0 * 25.4, h / 72.0 * 25.4, tl!("set in Print Settings…")),
                    _ => String::new(),
                };
                ui.label(egui::RichText::new(text).color(t.text_dim).size(11.0));
                if let Some(e) = f.get("__pageError").and_then(Value::as_str) {
                    ui.label(egui::RichText::new(e).color(t.warning).size(11.0));
                }
            } else {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(tl!("Printer")).color(t.text_dim));
                    let mut pr = s(f, "printer", "");
                    let opts = printer_options(f.get("__systemPrinters"), &pr);
                    let o: Vec<(String, &str)> = opts.iter().map(|(k, l)| (k.clone(), l.as_str())).collect();
                    if crate::widgets::dropdown(ui, "print-printer", tl!("Printer"), &mut pr, &o, 240.0) {
                        f.insert("printer".into(), json!(pr));
                    }
                });
                number(ui, f, "copies", tl!("Copies"), 1.0..=999.0, "", 1.0);
                print_paper_controls(ui, f);
            }
            head(ui, tl!("Color Management"));
            dropdown_str(
                ui,
                "print-color",
                tl!("Color Management"),
                f,
                "colorHandling",
                &[
                    ("printerManages", tl!("Printer Manages Colors")),
                    ("photocraftManages", tl!("PhotoCraft Manages Colors")),
                    ("noColorManagement", tl!("No Color Management")),
                ],
                240.0,
            );
            if s(f, "colorHandling", "") == "photocraftManages" {
                // Photoshop's Printer Profile menu: the built-ins, then every installed printer
                // (output class) profile by its description.
                let installed: Vec<_> = photocraft_engine::installed_profiles::installed()
                    .into_iter()
                    .filter(|p| p.is_destination() && p.class == photocraft_cms::ProfileClass::Output)
                    .collect();
                let mut profiles: Vec<(&str, &str)> = vec![
                    ("coated-cmyk", tl!("Coated CMYK")),
                    ("srgb", "sRGB IEC61966-2.1"),
                    ("adobe-rgb-compat", tl!("Adobe RGB (1998) compatible")),
                    ("display-p3", tl!("Display P3")),
                    ("gray-gamma-2.2", tl!("Gray Gamma 2.2")),
                ];
                profiles.extend(installed.iter().map(|p| (p.path.as_str(), p.description.as_str())));
                // A remembered profile that is no longer installed still shows, by its file name.
                let cur = s(f, "printerProfile", "coated-cmyk");
                let stem = std::path::Path::new(&cur).file_stem().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                if !profiles.iter().any(|(v, _)| *v == cur) {
                    profiles.push((cur.as_str(), stem.as_str()));
                }
                dropdown_str(ui, "print-profile", tl!("Profile"), f, "printerProfile", &profiles, 240.0);
                ui.horizontal(|ui| {
                    dropdown_str(
                        ui,
                        "print-intent",
                        tl!("Mode"),
                        f,
                        "intent",
                        &[
                            ("perceptual", tl!("Perceptual")),
                            ("relative", tl!("Relative Colorimetric")),
                            ("saturation", tl!("Saturation")),
                            ("absolute", tl!("Absolute Colorimetric")),
                        ],
                        170.0,
                    );
                    check(ui, f, "bpc", tl!("Black Point Compensation"), true);
                });
                if printers.is_some() {
                    ui.label(
                        egui::RichText::new(tl!("Remember to turn off the printer's own colour management in Print Settings…")).color(t.warning).size(11.0),
                    );
                }
            }
            head(ui, tl!("Position and Size"));
            dropdown_str(ui, "print-units", tl!("Units"), f, "units", &[("cm", tl!("Centimeters")), ("mm", tl!("Millimeters")), ("in", tl!("Inches"))], 130.0);
            let units = s(f, "units", "cm");
            check(ui, f, "center", tl!("Center"), true);
            let centered = b(f, "center", true);
            let centered_layout = centered.then(|| photocraft_engine::print_cmds::layout(&doc, &params(f), "file.print").ok()).flatten();
            // Revealing this row during a drag resizes the centered modal and moves the paper.
            // Keep it present, showing the resolved position while Center owns the offsets.
            ui.add_enabled_ui(!centered, |ui| {
                ui.horizontal(|ui| {
                    // Kept in inches (Photoshop's Position fields), shown in the chosen units.
                    for (key, label) in [("top", tl!("Top")), ("left", tl!("Left"))] {
                        let mut pt =
                            centered_layout.map_or_else(|| n(f, key, 0.0) * 72.0, |l| if key == "top" { l.paper.1 - l.rect.1 - l.rect.3 } else { l.rect.0 });
                        let limit = (photocraft_engine::print_cmds::MAX_POSITION_INCHES * 72.0 / unit_pt(&units)) as f32;
                        if length_field_range(ui, label, &mut pt, &units, 2, -limit..=limit) {
                            let limit = photocraft_engine::print_cmds::MAX_POSITION_INCHES;
                            f.insert(key.into(), json!((pt / 72.0).clamp(-limit, limit)));
                        }
                    }
                });
            });
            check(ui, f, "scaleToFit", tl!("Scale to Fit Media"), false);
            // Scaled Print Size: Scale, Height and Width move together; Print Resolution follows.
            if let Ok(l) = photocraft_engine::print_cmds::layout(&doc, &params(f), "file.print") {
                let (_, _, w, h) = l.rect;
                let (mut w2, mut h2) = (w, h);
                let fit = b(f, "scaleToFit", false);
                if !fit {
                    // Two decimals: Height and Width set fractional scales.
                    ui.horizontal(|ui| {
                        ui.label(egui::RichText::new(tl!("Scale")).color(t.text_dim));
                        let mut v = n(f, "scale", 100.0) as f32;
                        crate::widgets::value_field(ui, &mut v, 1.0..=1000.0, "%", 64.0).accessible_name(tl!("Scale"));
                        f.insert("scale".into(), json!((f64::from(v) * 100.0).round() / 100.0));
                    });
                }
                ui.horizontal(|ui| {
                    let hc = length_field(ui, tl!("Height"), &mut h2, &units);
                    let wc = length_field(ui, tl!("Width"), &mut w2, &units);
                    let k = if wc && w > 0.0 {
                        Some(w2 / w)
                    } else if hc && h > 0.0 {
                        Some(h2 / h)
                    } else {
                        None
                    };
                    if let Some(k) = k.filter(|k| k.is_finite() && *k > 0.0) {
                        f.insert("scale".into(), json!(((l.scale * k * 100.0) * 100.0).round() / 100.0));
                        f.insert("scaleToFit".into(), json!(false));
                    }
                });
                if w > 0.0 {
                    let ppi = f64::from(doc.size.width) / (w / 72.0);
                    ui.label(egui::RichText::new(format!("{} {ppi:.0} PPI", tl!("Print Resolution:"))).color(t.text_dim).size(11.0));
                }
            }
            head(ui, tl!("Printing Marks"));
            ui.horizontal(|ui| {
                check(ui, f, "cornerCropMarks", tl!("Corner Crop Marks"), false);
                check(ui, f, "centerCropMarks", tl!("Center Crop Marks"), false);
            });
            ui.horizontal(|ui| {
                check(ui, f, "registrationMarks", tl!("Registration Marks"), false);
                check(ui, f, "description", tl!("Description"), false);
                check(ui, f, "labels", tl!("Labels"), false);
            });
            head(ui, tl!("Functions"));
            check(ui, f, "emulsionDown", tl!("Emulsion Down"), false);
            if b(f, "emulsionDown", false) && printers.is_some() {
                ui.label(egui::RichText::new(tl!("Don't also tick Mirror Image in Print Settings…: the two flips cancel out.")).color(t.text_dim).size(11.0));
            }
            head(ui, tl!("Save as PDF"));
            ui.horizontal(|ui| {
                let mut out = s(f, "output", "");
                if ui.add(egui::TextEdit::singleline(&mut out).hint_text(tl!("(print to the printer)")).desired_width(240.0)).changed() {
                    f.insert("output".into(), json!(out));
                }
                browse = ui.button(tl!("Browse…")).clicked();
            });
        });
    });
    if browse {
        let current = s(f, "output", "");
        let suggested = if current.is_empty() { std::path::Path::new(&doc.name).with_extension("pdf").to_string_lossy().into_owned() } else { current };
        browse_pdf(app, id, &suggested);
    }
}

/// `file.print`'s params. The driver's printable area only goes with a print to the printer:
/// an output path or a preview set in the same frame as the confirm still drops it.
pub(super) fn print_params(f: &Map<String, Value>) -> Value {
    let mut p = params(f);
    if (b(f, "dryRun", false) || !s(f, "output", "").is_empty())
        && let Some(o) = p.as_object_mut()
    {
        o.remove("printable");
    }
    p
}

/// Submits the dialog parameters to `file.print`, propagating command errors unchanged.
/// Updates the status for completed results only; pending jobs report completion later.
pub(super) fn print_confirm(app: &mut PhotocraftApp, f: &Map<String, Value>) -> Result<Value, String> {
    let r = app.run("file.print", print_params(f))?;
    if r["pending"] != json!(true) {
        print_status(app, &r);
    }
    Ok(r)
}

pub(crate) fn print_status(app: &mut PhotocraftApp, r: &Value) {
    app.ui.status = if let Some(printer) = r["spooler"]["printer"].as_str() {
        crate::i18n::fmt(tl!("Sent to {printer}"), &[("printer", printer)])
    } else if r["sent"] == json!(true) {
        let template = "Sent to the printer ({spooler})";
        crate::i18n::fmt(tl!(template), &[("spooler", r["spooler"].as_str().unwrap_or(""))])
    } else {
        let template = "Printed to {path}";
        crate::i18n::fmt(tl!(template), &[("path", r["pdf"].as_str().unwrap_or(""))])
    };
    app.ui.status_error = false;
}
