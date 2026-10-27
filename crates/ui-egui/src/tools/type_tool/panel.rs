use super::defaults::{apply, apply_defaults, default_styles, formatting_params, open_color_picker, shown_scale, target, type_unit};
use super::fonts::{axis_sliders, font_picker, fonts_ready, selected_style, style_label, styles};
use super::options_bar::{
    align_glyph, apply_kerning, direction_glyph, direction_items, icon_label, kerning_at, kerning_field, kerning_label, num_field, styles_at, toggle_cell,
};
use super::*;

/// The Character panel's OpenType buttons, in Photoshop's order: (feature tag, command id,
/// tooltip). The tooltips are the Type › OpenType menu labels.
pub(crate) const OPENTYPE_BUTTONS: [(&str, &str, &str); 8] = [
    ("liga", "type.openType.standardLigatures", "Standard Ligatures"),
    ("calt", "type.openType.contextualAlternates", "Contextual Alternates"),
    ("dlig", "type.openType.discretionaryLigatures", "Discretionary Ligatures"),
    ("swsh", "type.openType.swash", "Swash"),
    ("salt", "type.openType.stylisticAlternates", "Stylistic Alternates"),
    ("titl", "type.openType.titlingAlternates", "Titling Alternates"),
    ("ordn", "type.openType.ordinals", "Ordinals"),
    ("frac", "type.openType.fractions", "Fractions"),
];

/// Photoshop's OpenType button row: each shows pressed while its feature is on at the type
/// target (the selection start, else the layer) and toggles it through the Type › OpenType
/// command, on the same scope as the other Character controls.
fn opentype_row(app: &mut PhotocraftApp, ui: &mut egui::Ui, c: &photocraft_doc::text::CharStyle, full: f32) {
    let cw = cell_width(full, OPENTYPE_BUTTONS.len());
    let mut clicked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (tag, id, tip) in OPENTYPE_BUTTONS {
            let on = photocraft_engine::type_extra_cmds::feature_on(c, tag);
            if toggle_cell(ui, cw, on, tip, |ui, r, col| opentype_glyph(ui.painter(), r, tag, col)) {
                clicked = Some((id, on));
            }
        }
    });
    if let Some((id, on)) = clicked {
        toggle_opentype(app, id, on);
    }
}

/// Runs an OpenType toggle command on the Character panel's scope: the edited layer's selected
/// characters (or its whole text) while typing, else every selected type layer. The new state
/// is explicit, so it follows the button the user saw rather than the layer's first run.
pub(crate) fn toggle_opentype(app: &mut PhotocraftApp, id: &str, shown_on: bool) {
    let Some(mut p) = formatting_params(app) else {
        if let Some((tag, _, _)) = OPENTYPE_BUTTONS.iter().find(|(_, command, _)| *command == id) {
            let props = match *tag {
                "liga" => json!({"ligatures": !shown_on}),
                "dlig" => json!({"discretionaryLigatures": !shown_on}),
                _ => json!({"features": { (*tag): !shown_on }}),
            };
            apply_defaults(app, &props);
        }
        return;
    };
    p["on"] = json!(!shown_on);
    let _ = app.run(id, p);
}

/// Small glyph labels for the OpenType buttons (drawn here, no Adobe artwork): fi, st, ct with
/// a ligature arc, a swash A, aa, T, 1st and ½.
fn opentype_glyph(p: &egui::Painter, r: egui::Rect, tag: &str, col: Color32) {
    let font = egui::FontId::proportional(13.0);
    let centred = |s: &str, font: egui::FontId| {
        let g = p.layout_no_wrap(s.to_string(), font, col);
        let pos = r.center() - g.size() / 2.0;
        let gr = egui::Rect::from_min_size(pos, g.size());
        p.galley(pos, g, col);
        gr
    };
    match tag {
        "liga" => {
            centred("fi", font);
        }
        "calt" => {
            centred("st", font);
        }
        "dlig" => {
            let gr = centred("ct", font);
            // The ligature's connecting arc from the c over to the t.
            let a = egui::pos2(gr.left() + gr.width() * 0.3, gr.top() + gr.height() * 0.42);
            let b = egui::pos2(gr.right() - gr.width() * 0.2, gr.top() + 4.0);
            let mid = egui::pos2((a.x + b.x) / 2.0, gr.top() + 1.5);
            p.line_segment([a, mid], Stroke::new(1.0, col));
            p.line_segment([mid, b], Stroke::new(1.0, col));
        }
        "swsh" => {
            let gr = centred("A", crate::theme::semibold(13.0));
            // A swash tail sweeping out under the letter.
            let y = gr.bottom() - 3.0;
            p.line_segment([egui::pos2(gr.left() - 3.0, y + 1.5), egui::pos2(gr.right() + 2.0, y - 0.5)], Stroke::new(1.0, col));
        }
        "salt" => {
            centred("aa", font);
        }
        "titl" => {
            centred("T", crate::theme::semibold(13.0));
        }
        "ordn" => {
            let one = p.layout_no_wrap("1".into(), font, col);
            let st = p.layout_no_wrap("st".into(), egui::FontId::proportional(9.0), col);
            let left = r.center().x - (one.size().x + st.size().x) / 2.0;
            let top = r.center().y - one.size().y / 2.0;
            let st_x = left + one.size().x;
            p.galley(egui::pos2(left, top), one, col);
            p.galley(egui::pos2(st_x, top - 1.0), st, col);
        }
        _ => {
            centred("½", font);
        }
    }
}

/// Width of each of `n` equal cells filling `avail` with 2 pt gaps.
fn cell_width(avail: f32, n: usize) -> f32 {
    let n = n.max(1) as f32;
    ((avail - (n - 1.0) * 2.0) / n).floor().max(20.0)
}

/// Properties panel sections for a type layer: Character, Paragraph and Type Options (#155).
pub fn type_properties(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    type_sections(app, ui, true, true, false);
    type_options(app, ui);
}

/// Window › Character / Paragraph (#150): the same controls as Properties, in their own dock
/// group. Without a type target they edit the defaults for the next Type-tool layer.
pub fn character_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui, paragraph: bool) {
    type_sections(app, ui, !paragraph, paragraph, true);
}

/// The Character and/or Paragraph sections, with the shared collapsible headers and fields that
/// fill the panel width (#155).
fn type_sections(app: &mut PhotocraftApp, ui: &mut egui::Ui, character: bool, paragraph: bool, show_defaults: bool) {
    use crate::props_layout::{COL_GAP, LABEL_GAP, LABEL_W, field_width, section};
    use crate::theme::ROW_GAP;
    let Some((c, para)) = styles_at(app).or_else(|| show_defaults.then(|| default_styles(app))) else { return };
    // Preferences ▸ Units & Rulers ▸ Type, as in the options bar: the model keeps points.
    let (u, unit) = type_unit(app);
    let t = crate::theme::Tokens::get(ui.ctx());
    // Row helper: `ui.horizontal` with the column gap, then the shared row gap.
    let row = |ui: &mut egui::Ui, add: &mut dyn FnMut(&mut egui::Ui)| {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = COL_GAP;
            add(ui)
        });
        ui.add_space(ROW_GAP);
    };
    if character && section(ui, "character", tl!("Character")) {
        let full = ui.available_width();
        let w = field_width(full, 2, LABEL_W);
        let mut fam = c.font_family.clone();
        let mut ready = true;
        row(ui, &mut |ui| {
            ready = fonts_ready(ui, full);
            if ready && font_picker(ui, &mut fam, full, app.session.prefs().type_.font_preview, &app.ui.recent_fonts) {
                app.note_recent_font(&fam);
                app.ui.tool_options.type_font = fam.clone();
                apply(app, ui.ctx(), json!({"font": fam}));
            }
        });
        row(ui, &mut |ui| {
            if !ready {
                return;
            }
            let mut style = selected_style(&c);
            let opts: Vec<(String, String)> = styles(&fam).into_iter().map(|s| (s.clone(), style_label(&s))).collect();
            let opts_ref: Vec<(String, &str)> = opts.iter().map(|(a, b)| (a.clone(), b.as_str())).collect();
            if crate::widgets::dropdown(ui, "props-type-style", tl!("Style"), &mut style, &opts_ref, full) {
                apply(app, ui.ctx(), json!({"fontStyle": style}));
            }
        });
        axis_sliders(app, ui, &c);
        row(ui, &mut |ui| {
            let k = shown_scale(app);
            // Hover tips stay English until wired through `tl!` in a multi-locale change
            // (`every_tl_literal_is_translated` requires every complete catalog).
            if let Some(v) = num_field(ui, "tT", "Font size", c.size_pt * k * u, 0.1 * u..=1296.0 * u, unit, w) {
                app.ui.tool_options.type_size = v / u;
                apply(app, ui.ctx(), json!({"size": v / u, "metricsAsShown": true}));
            }
            let lead = c.leading_pt.unwrap_or(c.size_pt * para.auto_leading.max(0.01)) * k;
            if let Some(v) = num_field(ui, "A↕", "Leading (set to the font size × auto-leading when Auto)", lead * u, 0.1 * u..=5000.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"leading": v / u, "metricsAsShown": true}));
            }
        });
        row(ui, &mut |ui| {
            let shown = kerning_label(kerning_at(app).unwrap_or((c.kerning, c.kern)));
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = LABEL_GAP;
                icon_label(ui, "text-cursor", "Kerning: Metrics, Optical or a value in 1/1000 em (Alt+←/→ at the caret)");
                if let Some(v) = kerning_field(ui, &shown, w) {
                    apply_kerning(app, ui.ctx(), v);
                }
            });
            if let Some(v) = num_field(ui, "VA", "Tracking (1/1000 em)", c.tracking, -1000.0..=10000.0, "", w) {
                apply(app, ui.ctx(), json!({"tracking": v}));
            }
        });
        row(ui, &mut |ui| {
            if let Some(v) = num_field(ui, "↕T", "Vertical scale", c.vertical_scale * 100.0, 0.0..=1000.0, "%", w) {
                apply(app, ui.ctx(), json!({"verticalScale": v}));
            }
            if let Some(v) = num_field(ui, "↔T", "Horizontal scale", c.horizontal_scale * 100.0, 0.0..=1000.0, "%", w) {
                apply(app, ui.ctx(), json!({"horizontalScale": v}));
            }
        });
        row(ui, &mut |ui| {
            if let Some(v) = num_field(ui, "Aª", "Baseline shift", c.baseline_shift_pt * u, -1296.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"baselineShift": v / u}));
            }
            // Colour: label + a swatch filling the rest of the second column.
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = LABEL_GAP;
                let g = ui.painter().layout_no_wrap(tl!("Color").into(), egui::FontId::proportional(12.0), t.text_dim);
                let lw = g.size().x.max(LABEL_W);
                let (lr, _) = ui.allocate_exact_size(egui::vec2(lw, 22.0), egui::Sense::hover());
                ui.painter().galley(egui::pos2(lr.left(), lr.center().y - g.size().y / 2.0), g, t.text_dim);
                let rgb = c.color.to_rgb();
                let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
                let sw = (LABEL_W + LABEL_GAP + w - lw - LABEL_GAP).max(24.0);
                let (rect, resp) = ui.allocate_exact_size(egui::vec2(sw, 22.0), egui::Sense::click());
                ui.painter().rect_filled(rect, t.radius_sm, Color32::from_rgb(q(rgb[0]), q(rgb[1]), q(rgb[2])));
                ui.painter().rect_stroke(rect, t.radius_sm, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
                let resp = resp.on_hover_text(tl!("Text color"));
                resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Text color")));
                if resp.clicked() {
                    open_color_picker(app, rgb);
                }
            });
        });
        // Faux styles: T (bold)  T (italic)  TT  Tᴛ  T̲  T̶, sharing the row evenly.
        let caps = c.caps;
        // Tip strings are English keys; `toggle_cell` runs them through `tl!`.
        let toggles: [(&str, &str, bool, serde_json::Value); 6] = [
            ("T", "Faux Bold", c.faux_bold, json!({"fauxBold": !c.faux_bold})),
            ("T", "Faux Italic", c.faux_italic, json!({"fauxItalic": !c.faux_italic})),
            (
                "TT",
                "All Caps",
                caps == photocraft_doc::text::Caps::AllCaps,
                json!({"caps": if caps == photocraft_doc::text::Caps::AllCaps { "normal" } else { "allCaps" }}),
            ),
            (
                "Tᴛ",
                "Small Caps",
                caps == photocraft_doc::text::Caps::SmallCaps,
                json!({"caps": if caps == photocraft_doc::text::Caps::SmallCaps { "normal" } else { "smallCaps" }}),
            ),
            ("T", "Underline", c.underline, json!({"underline": !c.underline})),
            ("T", "Strikethrough", c.strikethrough, json!({"strikethrough": !c.strikethrough})),
        ];
        let cw = cell_width(full, toggles.len());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (i, (glyph, tip, on, props)) in toggles.into_iter().enumerate() {
                let clicked = toggle_cell(ui, cw, on, tip, |ui, r, col| {
                    let font = if i == 0 { crate::theme::semibold(13.0) } else { egui::FontId::proportional(13.0) };
                    let g = ui.painter().layout_no_wrap(glyph.to_string(), font, col);
                    let pos = r.center() - g.size() / 2.0;
                    let gr = egui::Rect::from_min_size(pos, g.size());
                    if i == 1 {
                        // Faux italic: a slanted T drawn as strokes (no italic face is bundled).
                        let (top, bot, cx) = (gr.top() + 3.0, gr.bottom() - 3.0, gr.center().x);
                        let slant = (bot - top) * 0.25;
                        ui.painter().line_segment([egui::pos2(cx - 4.0 + slant / 2.0, top), egui::pos2(cx + 4.0 + slant / 2.0, top)], Stroke::new(1.3, col));
                        ui.painter().line_segment([egui::pos2(cx + slant / 2.0, top), egui::pos2(cx - slant / 2.0, bot)], Stroke::new(1.3, col));
                    } else {
                        ui.painter().galley(pos, g, col);
                    }
                    if i == 4 {
                        ui.painter().line_segment([egui::pos2(gr.left(), gr.bottom() - 2.0), egui::pos2(gr.right(), gr.bottom() - 2.0)], Stroke::new(1.0, col));
                    }
                    if i == 5 {
                        ui.painter()
                            .line_segment([egui::pos2(gr.left() - 1.0, gr.center().y), egui::pos2(gr.right() + 1.0, gr.center().y)], Stroke::new(1.0, col));
                    }
                });
                if clicked {
                    apply(app, ui.ctx(), props);
                }
            }
        });
        ui.add_space(ROW_GAP);
        opentype_row(app, ui, &c, full);
        ui.add_space(ROW_GAP);
    }
    if paragraph && section(ui, "paragraph", tl!("Paragraph")) {
        use photocraft_doc::text::TextAlign as A;
        let full = ui.available_width();
        let w = field_width(full, 2, LABEL_W);
        let items = [
            ("left", tl!("Left align text"), A::Left),
            ("center", tl!("Center text"), A::Center),
            ("right", tl!("Right align text"), A::Right),
            ("justifyLeft", tl!("Justify last left"), A::JustifyLeft),
            ("justifyCenter", tl!("Justify last centered"), A::JustifyCenter),
            ("justifyRight", tl!("Justify last right"), A::JustifyRight),
            ("justifyAll", tl!("Justify all"), A::JustifyAll),
        ];
        let cw = cell_width(full, items.len());
        // Justification needs a paragraph box or area type: on point text Photoshop greys the four
        // justify buttons (#7713).
        // The panel formats the layer being typed in, else every selected type layer.
        let point = match formatting_params(app) {
            Some(p) => p.get("layers").and_then(serde_json::Value::as_array).map_or_else(
                || p.get("layer").and_then(serde_json::Value::as_u64).into_iter().collect::<Vec<_>>(),
                |ids| ids.iter().filter_map(serde_json::Value::as_u64).collect(),
            ),
            None => Vec::new(),
        }
        .into_iter()
        .any(|layer| {
            let shape = app.session.active().and_then(|s| text_layer(&s.doc, LayerId(layer)).map(|t| t.shape.clone()));
            !matches!(shape, Some(photocraft_doc::text::TextShape::Box { .. } | photocraft_doc::text::TextShape::Area { .. }))
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (key, tip, a) in items {
                let justify = !matches!(a, A::Left | A::Center | A::Right);
                let clicked = ui
                    .add_enabled_ui(!(point && justify), |ui| {
                        toggle_cell(ui, cw, para.align == a, tip, |ui, r, col| {
                            // The glyph keeps its size; the cell grows around it.
                            let g = egui::Rect::from_center_size(r.center(), egui::vec2(12.0, 10.0));
                            let col = if !ui.is_enabled() || para.align == a { col } else { t.icon };
                            align_glyph(ui.painter(), g, a, col);
                        })
                    })
                    .inner;
                if clicked {
                    apply(app, ui.ctx(), json!({"align": key}));
                }
            }
        });
        ui.add_space(ROW_GAP);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (key, tip, d) in direction_items() {
                let clicked = toggle_cell(ui, cw, para.direction == d, tip, |ui, r, col| {
                    let g = egui::Rect::from_center_size(r.center(), egui::vec2(14.0, 16.0));
                    direction_glyph(ui.painter(), g, d, col);
                });
                if clicked {
                    apply(app, ui.ctx(), json!({"direction": key}));
                }
            }
        });
        ui.add_space(ROW_GAP);
        row(ui, &mut |ui| {
            if let Some(v) = num_field(ui, "→|", "Indent left margin", para.start_indent_pt * u, -1296.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"startIndent": v / u}));
            }
            if let Some(v) = num_field(ui, "|←", "Indent right margin", para.end_indent_pt * u, -1296.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"endIndent": v / u}));
            }
        });
        row(ui, &mut |ui| {
            if let Some(v) = num_field(ui, "¶→", "Indent first line", para.first_line_indent_pt * u, -1296.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"firstLineIndent": v / u}));
            }
        });
        row(ui, &mut |ui| {
            if let Some(v) = num_field(ui, "↑¶", "Add space before paragraph", para.space_before_pt * u, 0.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"spaceBefore": v / u}));
            }
            if let Some(v) = num_field(ui, "¶↓", "Add space after paragraph", para.space_after_pt * u, 0.0 * u..=1296.0 * u, unit, w) {
                apply(app, ui.ctx(), json!({"spaceAfter": v / u}));
            }
        });
        // Hyphenate is stored (⌥⇧⌘H / PSD AutoHyphenate) but layout never breaks words (#6710).
        ui.add_space(ROW_GAP);
    }
}

/// Type Options: anti-aliasing and orientation of the whole layer (the Type menu's commands).
fn type_options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    use crate::props_layout::{COL_GAP, LABEL_GAP};
    let Some((layer, _)) = target(app) else { return };
    let Some((aa, vertical)) = app
        .session
        .active()
        .and_then(|st| text_layer(&st.doc, LayerId(layer)))
        .map(|t| (photocraft_engine::type_extra_cmds::aa_name(t.antialias).to_string(), t.orientation == photocraft_doc::text::Orientation::Vertical))
    else {
        return;
    };
    if !crate::props_layout::section(ui, "typeOptions", "Type Options") {
        return;
    }
    let full = ui.available_width();
    let mut run: Option<String> = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = COL_GAP;
        // Anti-aliasing dropdown (aₐ glyph label), then the two orientation toggles.
        let orient_w = 2.0 * 28.0 + 2.0;
        let dd_w = (full - crate::props_layout::LABEL_W - LABEL_GAP - COL_GAP - orient_w).max(60.0);
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = LABEL_GAP;
            let t = crate::theme::Tokens::get(ui.ctx());
            let (r, resp) = ui.allocate_exact_size(egui::vec2(crate::props_layout::LABEL_W, 22.0), egui::Sense::hover());
            ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, "aa", crate::theme::semibold(11.0), t.text_dim);
            resp.on_hover_text(tl!("Anti-aliasing method"));
            let mut cur = aa.clone();
            let opts: Vec<(String, &str)> = ["none", "sharp", "crisp", "strong", "smooth", "windowsLcd", "windows"]
                .into_iter()
                .filter_map(|k| photocraft_engine::commands::find(&format!("type.antiAlias.{k}")).map(|c| (k.to_string(), c.label)))
                .collect();
            if crate::widgets::dropdown(ui, "props-type-aa", tl!("Anti-aliasing method"), &mut cur, &opts, dd_w) {
                run = Some(format!("type.antiAlias.{cur}"));
            }
        });
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            for (key, tip, on) in [("horizontal", "Horizontal type", !vertical), ("vertical", "Vertical type", vertical)] {
                let clicked = toggle_cell(ui, 28.0, on, tip, |ui, r, col| {
                    let g = egui::Rect::from_center_size(r.center(), egui::vec2(12.0, 12.0));
                    let s = Stroke::new(1.3, col);
                    if key == "horizontal" {
                        ui.painter().line_segment([g.left_top(), g.right_top()], s);
                        ui.painter().line_segment([g.center_top(), g.center_bottom()], s);
                    } else {
                        ui.painter().line_segment([g.left_top(), g.left_bottom()], s);
                        ui.painter().line_segment([g.left_center(), g.right_center()], s);
                    }
                });
                if clicked && !on {
                    run = Some(format!("type.orientation.{key}"));
                }
            }
        });
    });
    ui.add_space(crate::theme::ROW_GAP);
    if let Some(id) = run
        && let Some(params) = formatting_params(app)
        && let Err(e) = app.run(&id, params)
    {
        app.ui.status = e;
        app.ui.status_error = true;
    }
}
