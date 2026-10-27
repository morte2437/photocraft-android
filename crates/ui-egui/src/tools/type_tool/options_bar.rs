use super::defaults::{apply, formatting_params, open_color_picker, shown_scale, target, target_doc, type_unit};
use super::editing::{cancel, commit_with};
use super::fonts::{font_picker, fonts_ready, selected_style, style_label, styles};
use super::keys::is_vertical;
use super::*;

/// Photoshop's Type options bar.
pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = crate::theme::Tokens::get(ui.ctx());
    // Show the target layer's (first-run) style, else the tool defaults.
    let shown = target(app).and_then(|(id, _)| {
        let st = app.session.active()?;
        let tl = text_layer(&st.doc, LayerId(id))?;
        let run = tl.char_runs().into_iter().next().map(|r| r.style);
        // The size at the selection (else the first run), as the layer shows it (#124).
        let size = styles_at(app).map_or(tl.size_pt, |(c, _)| c.size_pt) * shown_scale(app);
        Some((tl.font_family.clone(), run.as_ref().map(selected_style).unwrap_or_default(), size, styles_at(app).map_or(tl.color, |(c, _)| c.color)))
    });
    let o = app.ui.tool_options.clone();
    let (mut fam, mut style, size) = match &shown {
        Some((f, s, z, _)) => (f.clone(), if s.is_empty() { "Regular".into() } else { s.clone() }, *z),
        None => (o.type_font.clone(), o.type_style.clone(), o.type_size),
    };
    if crate::icons::button(ui, "text-cursor", 24.0, false, tl!("Toggle text orientation")).clicked()
        && let Some((layer, _)) = target(app)
        && let Some(params) = formatting_params(app)
    {
        let to = if is_vertical(app, LayerId(layer)) { "horizontal" } else { "vertical" };
        if let Err(e) = app.run(&format!("type.orientation.{to}"), params) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
    }
    // Font family and style menus (one placeholder in their place while the fonts load).
    if fonts_ready(ui, 170.0 + ui.spacing().item_spacing.x + 110.0) {
        if font_picker(ui, &mut fam, 170.0, app.session.prefs().type_.font_preview, &app.ui.recent_fonts) {
            app.note_recent_font(&fam);
            app.ui.tool_options.type_font = fam.clone();
            let st = styles(&fam);
            style = if st.contains(&style) { style } else { st.first().cloned().unwrap_or_else(|| "Regular".into()) };
            app.ui.tool_options.type_style = style.clone();
            apply(app, ui.ctx(), json!({"font": fam}));
        }
        let opts: Vec<(String, String)> = styles(&fam).into_iter().map(|s| (s.clone(), style_label(&s))).collect();
        let opts_ref: Vec<(String, &str)> = opts.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
        if crate::widgets::dropdown(ui, "type-style", tl!("Style"), &mut style, &opts_ref, 110.0) {
            app.ui.tool_options.type_style = style.clone();
            apply(app, ui.ctx(), json!({"fontStyle": style}));
        }
    }
    let (r, icon) = ui.allocate_exact_size(egui::vec2(18.0, 22.0), egui::Sense::hover());
    crate::icons::paint(ui, r, "type", 13.0, t.icon);
    icon.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, tl!("Font size")));
    // Photoshop's "T" scrubs the size field beside it, like the Character panel's icons (#6437).
    crate::widgets::scrub_option_icon(ui, &icon, tl!("Font size"));
    let (unit, label) = type_unit(app);
    let mut display = size * unit;
    if crate::widgets::value_field(ui, &mut display, unit..=1296.0 * unit, label, 66.0).changed() {
        let size = display / unit;
        app.ui.tool_options.type_size = size;
        apply(app, ui.ctx(), json!({"size": size, "metricsAsShown": true}));
    }
    let mut aa = target(app)
        .and_then(|(id, _)| app.session.active().and_then(|st| text_layer(&st.doc, LayerId(id))))
        .map(|t| photocraft_engine::type_extra_cmds::aa_name(t.antialias).to_string())
        .unwrap_or_else(|| o.type_aa.clone());
    let aa_opts = [
        ("none".to_string(), tl!("None")),
        ("sharp".to_string(), tl!("Sharp")),
        ("crisp".to_string(), tl!("Crisp")),
        ("strong".to_string(), tl!("Strong")),
        ("smooth".to_string(), tl!("Smooth")),
    ];
    if crate::widgets::dropdown(ui, "type-aa", tl!("Anti-aliasing method"), &mut aa, &aa_opts, 80.0) {
        app.ui.tool_options.type_aa = aa.clone();
        if let Some(params) = formatting_params(app) {
            let _ = app.run(&format!("type.antiAlias.{aa}"), params);
        }
    }
    crate::widgets::vline(ui, 22.0);
    ui.spacing_mut().item_spacing.x = 2.0;
    // Alignment and direction show the target paragraph's, as Photoshop's bar follows the text
    // being edited or the selected type layer; the tool defaults otherwise. A justified paragraph
    // lights none of the three.
    let para = styles_at(app).map(|(_, p)| p);
    let shown_align = match &para {
        Some(p) => match p.align {
            photocraft_doc::text::TextAlign::Left => "left",
            photocraft_doc::text::TextAlign::Center => "center",
            photocraft_doc::text::TextAlign::Right => "right",
            _ => "",
        },
        None => o.type_align.as_str(),
    };
    for (align, icon, tip) in
        [("left", "align-left", tl!("Left align text")), ("center", "align-center", tl!("Center text")), ("right", "align-right", tl!("Right align text"))]
    {
        if crate::icons::button(ui, icon, 24.0, shown_align == align, tip).clicked() {
            app.ui.tool_options.type_align = align.into();
            apply(app, ui.ctx(), json!({"align": align}));
        }
    }
    ui.spacing_mut().item_spacing.x = 8.0;
    crate::widgets::vline(ui, 22.0);
    ui.spacing_mut().item_spacing.x = 2.0;
    for (key, tip, d) in direction_items() {
        let on = para.as_ref().map_or(o.type_direction == key, |p| p.direction == d);
        if direction_button(ui, d, on, tip).clicked() {
            app.ui.tool_options.type_direction = key.into();
            apply(app, ui.ctx(), json!({"direction": key}));
        }
    }
    ui.spacing_mut().item_spacing.x = 8.0;
    crate::widgets::vline(ui, 22.0);
    // Text colour swatch with a picker popup.
    let c = shown
        .as_ref()
        .map(|s| s.3)
        .map(|c| {
            let v = c.to_rgb();
            [v[0], v[1], v[2], 1.0]
        })
        .unwrap_or(app.session.tools.foreground);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(28.0, 18.0), egui::Sense::click());
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    ui.painter().rect_filled(rect, 2.0, Color32::from_rgb(q(c[0]), q(c[1]), q(c[2])));
    ui.painter().rect_stroke(rect, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Outside);
    let resp = resp.on_hover_text(tl!("Set the text color"));
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Set the text color")));
    if resp.clicked() {
        open_color_picker(app, [c[0], c[1], c[2]]);
    }
    // Create warped text, after the swatch as in Photoshop (#5911): the Type › Warp Text… dialog.
    let can_warp = crate::menus::is_enabled(app, "type.warpText");
    if ui.add_enabled_ui(can_warp, |ui| crate::icons::button(ui, "text-warp", 24.0, false, tl!("Create warped text"))).inner.clicked()
        && let Err(e) = crate::menus::invoke(app, &ui.ctx().clone(), "type.warpText", json!({}))
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    if app.ui.text_edit.is_some() {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.add_space(8.0);
            if crate::icons::button(
                ui,
                "check",
                24.0,
                false,
                &crate::i18n::fmt(tl!("Commit any current edits ({key})"), &[("key", &crate::shortcuts::pretty("Cmd+Enter"))]),
            )
            .clicked()
            {
                commit_with(app, ui.input(|i| i.modifiers));
            }
            if crate::icons::button(ui, "ban", 24.0, false, tl!("Cancel any current edits (Esc)")).clicked() {
                cancel(app);
            }
        });
    }
}

/// Character and paragraph style at the target (selection start, else the layer's first run).
/// The paragraph at a collapsed caret is the caret's, as Photoshop's options bar and Paragraph
/// panel show it.
pub(crate) fn styles_at(app: &PhotocraftApp) -> Option<(photocraft_doc::text::CharStyle, photocraft_doc::text::ParagraphStyle)> {
    let (layer, range) = target(app)?;
    let t = text_layer(&target_doc(app)?.doc, LayerId(layer))?;
    let at = range.map_or(0, |r| byte_of(&t.text, r[0]));
    let para_at = match (range, &app.ui.text_edit) {
        (None, Some(ed)) => byte_of(&t.text, ed.caret),
        _ => at,
    };
    let pick_at = |at: usize, lens: Vec<usize>| -> usize {
        let mut acc = 0;
        for (i, len) in lens.iter().enumerate() {
            acc += len;
            if at < acc {
                return i;
            }
        }
        lens.len().saturating_sub(1)
    };
    let runs = t.char_runs();
    let paras = t.paragraph_runs();
    let c = runs.get(pick_at(at, runs.iter().map(|r| r.len).collect())).map(|r| r.style.clone())?;
    let p = paras.get(pick_at(para_at, paras.iter().map(|r| r.len).collect())).map(|r| r.style.clone()).unwrap_or_default();
    Some((c, p))
}

/// Kerning shown for the target: at a collapsed caret, the pair before it (Photoshop's
/// Character panel); else the style at the selection.
pub(super) fn kerning_at(app: &PhotocraftApp) -> Option<(photocraft_doc::text::Kerning, f32)> {
    let ed = app.ui.text_edit.as_ref().filter(|e| e.caret == e.anchor && e.caret > 0)?;
    let st = app.session.active()?;
    let t = text_layer(&st.doc, LayerId(ed.layer))?;
    let at = byte_of(&t.text, ed.caret - 1);
    let mut end = 0;
    t.char_runs()
        .into_iter()
        .find(|r| {
            end += r.len;
            at < end
        })
        .map(|r| (r.style.kerning, r.style.kern))
}

/// The Character panel's kerning text: "Metrics", "Optical" or the manual value.
pub(crate) fn kerning_label((mode, kern): (photocraft_doc::text::Kerning, f32)) -> String {
    use photocraft_doc::text::Kerning;
    match mode {
        _ if kern != 0.0 && kern.is_finite() => format!("{}", kern.round() as i64),
        Kerning::Metrics => "Metrics".into(),
        Kerning::Optical => "Optical".into(),
        Kerning::Off => "0".into(),
    }
}

/// Parses a typed kerning value: Metrics / Optical (any case) or a number in 1/1000 em
/// (Photoshop's range, -1000..10000). `None` for anything else.
pub(crate) fn parse_kerning(s: &str) -> Option<serde_json::Value> {
    let s = s.trim();
    match s.to_ascii_lowercase().as_str() {
        "metrics" => Some(json!("metrics")),
        "optical" => Some(json!("optical")),
        _ => crate::numeric_expression::parse(s).filter(|v| v.is_finite() && (-1000.0..=10000.0).contains(v)).map(|v| json!(v.round())),
    }
}

/// Photoshop's editable kerning combo: type a value or Metrics/Optical, or pick a preset.
pub(super) fn kerning_field(ui: &mut egui::Ui, shown: &str, width: f32) -> Option<serde_json::Value> {
    let id = ui.make_persistent_id("props-kern-text");
    let mut buf: String = ui.data_mut(|d| d.get_temp::<String>(id)).unwrap_or_else(|| shown.to_string());
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let resp = ui.add(egui::TextEdit::singleline(&mut buf).desired_width((width - 40.0).max(24.0)).id(id.with("edit"))).accessible_name(tl!("Kerning"));
        if resp.has_focus() {
            ui.data_mut(|d| d.insert_temp(id, buf.clone()));
        } else {
            ui.data_mut(|d| d.remove::<String>(id));
        }
        if resp.lost_focus() && buf.trim() != shown {
            out = parse_kerning(&buf);
        }
        let presets = ["Metrics", "Optical", "0", "-100", "-75", "-50", "-25", "-10", "-5", "5", "10", "25", "50", "75", "100", "200"];
        let (press, close) = crate::press_menu::PressCombo::before(ui, "props-kern-presets");
        let combo = egui::ComboBox::from_id_salt("props-kern-presets")
            .selected_text("")
            .width(16.0)
            .height(420.0)
            .icon(crate::widgets::chevron_icon)
            .close_behavior(close)
            .show_ui(ui, |ui| {
                for p in presets {
                    let item = ui.selectable_label(p == shown, p);
                    if press.chosen(ui, &item) {
                        out = parse_kerning(p);
                    }
                }
            });
        press.after(&combo.response);
        let _ = combo.response.accessible_name(tl!("Kerning"));
    });
    out
}

/// Applies a kerning value: at a collapsed caret to the pair before it, else to the selection
/// (or the whole layer).
pub(super) fn apply_kerning(app: &mut PhotocraftApp, ctx: &egui::Context, v: serde_json::Value) {
    match app.ui.text_edit.clone().filter(|e| e.caret == e.anchor) {
        Some(ed) if ed.caret > 0 => {
            let _ = app.run("type.edit", json!({"layer": ed.layer, "range": [ed.caret - 1, ed.caret], "kerning": v}));
        }
        Some(_) => {}
        None => apply(app, ctx, json!({ "kerning": v })),
    }
}

pub(super) fn icon_label(ui: &mut egui::Ui, icon: &str, tip: &str) {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(egui::vec2(crate::props_layout::LABEL_W, 22.0), egui::Sense::hover());
    crate::icons::paint(ui, r, icon, 12.0, t.text_dim);
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), tl!(tip)));
    // Photoshop's Character panel icons scrub the field beside them.
    crate::widgets::scrub_label(ui, &resp.on_hover_text(tl!(tip)));
}

/// Labelled numeric field (Photoshop's icon + value pairs). Returns the new value when edited.
pub(super) fn num_field(ui: &mut egui::Ui, label: &str, tip: &str, v: f32, range: std::ops::RangeInclusive<f32>, unit: &str, width: f32) -> Option<f32> {
    let t = crate::theme::Tokens::get(ui.ctx());
    let mut v = v;
    let mut out = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        let (r, resp) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), egui::Sense::hover());
        ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, label, crate::theme::semibold(10.0), t.text_dim);
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, ui.is_enabled(), tl!(tip)));
        crate::widgets::scrub_label(ui, &resp.on_hover_text(tl!(tip)));
        let response = crate::widgets::value_field(ui, &mut v, range, unit, width);
        response.widget_info(|| {
            let mut info = egui::WidgetInfo::drag_value(ui.is_enabled(), f64::from(v));
            info.label = Some(tl!(tip).to_owned());
            info
        });
        if response.changed() {
            out = Some(v);
        }
    });
    out
}

/// Photoshop's paragraph alignment glyphs: four text lines whose widths/offsets show the mode
/// (justify variants run full width, with the last line placed left/centre/right).
pub(super) fn align_glyph(p: &egui::Painter, r: egui::Rect, a: photocraft_doc::text::TextAlign, c: Color32) {
    use photocraft_doc::text::TextAlign as A;
    let widths = [1.0, 0.62, 1.0, 0.62];
    for (i, w) in widths.iter().enumerate() {
        let y = r.top() + i as f32 * r.height() / 3.0;
        let full = r.width();
        let (len, x0) = match a {
            A::Left => (full * w, r.left()),
            A::Center => (full * w, r.center().x - full * w / 2.0),
            A::Right => (full * w, r.right() - full * w),
            A::JustifyAll => (full, r.left()),
            _ if i < 3 => (full, r.left()),
            A::JustifyLeft => (full * 0.55, r.left()),
            A::JustifyCenter => (full * 0.55, r.center().x - full * 0.275),
            _ => (full * 0.55, r.right() - full * 0.55),
        };
        p.line_segment([egui::pos2(x0, y), egui::pos2(x0 + len, y)], Stroke::new(1.5, c));
    }
}

/// Paragraph direction choices for the Paragraph panel and the options bar (TYPE-218-2): the
/// `direction` value `type.setStyle` takes, the tooltip, and the direction it sets.
pub(super) fn direction_items() -> [(&'static str, &'static str, photocraft_doc::text::TextDirection); 3] {
    use photocraft_doc::text::TextDirection as D;
    [
        ("auto", tl!("Automatic paragraph direction"), D::Auto),
        ("ltr", tl!("Left-to-right paragraph direction"), D::Ltr),
        ("rtl", tl!("Right-to-left paragraph direction"), D::Rtl),
    ]
}

/// A pilcrow over an arrow pointing the way the paragraph runs; Auto points both ways.
pub(super) fn direction_glyph(p: &egui::Painter, r: egui::Rect, d: photocraft_doc::text::TextDirection, c: Color32) {
    use photocraft_doc::text::TextDirection as D;
    let g = p.layout_no_wrap("¶".to_string(), egui::FontId::proportional(11.0), c);
    p.galley(egui::pos2(r.center().x - g.size().x / 2.0, r.top() - 2.0), g, c);
    let (y, s) = (r.bottom() - 1.5, Stroke::new(1.3, c));
    p.line_segment([egui::pos2(r.left(), y), egui::pos2(r.right(), y)], s);
    let head = |x: f32, towards: f32| {
        for dy in [-3.0, 3.0] {
            p.line_segment([egui::pos2(x, y), egui::pos2(x - towards * 3.0, y + dy)], s);
        }
    };
    if d != D::Rtl {
        head(r.right(), 1.0);
    }
    if d != D::Ltr {
        head(r.left(), -1.0);
    }
}

/// The options bar's 24 px square button for one paragraph direction, styled like its icon buttons.
fn direction_button(ui: &mut egui::Ui, d: photocraft_doc::text::TextDirection, selected: bool, tip: &str) -> egui::Response {
    let (r, resp) = ui.allocate_exact_size(egui::vec2(24.0, 24.0), egui::Sense::click());
    let tint = crate::icons::button_chrome(ui, r, selected, resp.hovered());
    direction_glyph(ui.painter(), egui::Rect::from_center_size(r.center(), egui::vec2(14.0, 16.0)), d, tint);
    let tip = tip.to_string();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &tip));
    resp.on_hover_text(tip)
}

/// A toggle button (faux styles, paragraph alignment) of width `w`, painted by `paint`.
pub(super) fn toggle_cell(ui: &mut egui::Ui, w: f32, on: bool, tip: &str, paint: impl FnOnce(&egui::Ui, egui::Rect, Color32)) -> bool {
    let t = crate::theme::Tokens::get(ui.ctx());
    let (r, resp) = ui.allocate_exact_size(egui::vec2(w, 24.0), egui::Sense::click());
    let bg = if on {
        t.accent_soft
    } else if resp.hovered() {
        t.hover
    } else {
        Color32::TRANSPARENT
    };
    ui.painter().rect_filled(r, t.radius_sm, bg);
    if on {
        ui.painter().rect_stroke(r, t.radius_sm, Stroke::new(1.0, t.accent_border), egui::StrokeKind::Inside);
    }
    let enabled = ui.is_enabled();
    paint(
        ui,
        r,
        if !enabled {
            t.text_faint
        } else if on {
            t.text
        } else {
            t.text_dim
        },
    );
    let tip = tip.to_string();
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, enabled, on, &tip));
    resp.on_hover_text(tl!(&tip)).clicked()
}
