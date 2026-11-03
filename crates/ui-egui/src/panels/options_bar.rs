use super::brush_controls::{brush_preset_chip, mixer_load_swatch, paint_mode_dropdown, smoothing_field, smoothing_options, tool_mode_dropdown};
use super::properties::{select_and_mask_button, selection_mode_buttons};
use super::{BlendMode, Color32, PhotocraftApp, RichText, Sense, Tokens, Tool, Value, icons, json, vec2, widgets};
use photocraft_paint::BrushSettings;

// Custom controls also need to bring keyboard focus into a scroll viewport.
pub(super) fn reveal_focused_control(ui: &egui::Ui) {
    // read_response reprocesses pointer events. Reading a popup's newly focused field
    // during its opening click can undo the focus that the popup just requested.
    if ui.input(|input| input.pointer.any_pressed() || input.pointer.any_released()) {
        return;
    }
    let focused = ui.memory(|memory| memory.focused());
    let key = ui.scope_id().with("scroll-focused-control");
    let changed = ui.data_mut(|data| {
        let previous = data.get_temp::<Option<egui::Id>>(key).flatten();
        data.insert_temp(key, focused);
        previous != focused
    });
    if changed
        && let Some(response) = focused.and_then(|id| ui.ctx().read_response(id))
        && response.layer_id == ui.layer_id()
        && ui.min_rect().contains_rect(response.rect)
    {
        ui.scroll_to_rect(response.rect, Some(egui::Align::Center));
    }
}

// ----------------------------------------------------------------------------- options bar

/// Options bar height: Preferences ▸ Workspace ▸ Enable Narrow Options Bar trims it, as in
/// Photoshop, leaving the document more room.
pub(super) fn options_bar_height(pro: bool, narrow: bool) -> f32 {
    match (pro, narrow) {
        (true, false) => 36.0,
        (true, true) => 30.0,
        (false, false) => 42.0,
        (false, true) => 34.0,
    }
}

pub fn options_bar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    crate::vector_ui::track_arrow_default_selection(app, ui.ctx());
    let t = Tokens::get(ui.ctx());
    let narrow = app.session.prefs().workspace.narrow_options_bar();
    egui::Panel::top("options_bar")
        .exact_size(options_bar_height(t.pro, narrow))
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(10, 0)))
        .show(ui, |ui| {
            ui.style_mut().spacing.scroll = egui::style::ScrollStyle { bar_width: 4.0, ..egui::style::ScrollStyle::floating() };
            egui::ScrollArea::horizontal().id_salt("tool-options").auto_shrink([false, false]).show(ui, |ui| {
                // Font ink can extend beyond its advance width at fractional positions.
                // Include its gutter in the container layout, including right-aligned actions.
                egui::Frame::NONE.inner_margin(egui::Margin { right: 2, ..Default::default() }).show(ui, |ui| {
                    crate::direction::horizontal_centered(ui, |ui| {
                        if t.pro {
                            crate::chrome_ui::home_button(app, ui);
                            widgets::vline(ui, 22.0);
                        }
                        let tool_icon = icons::button(ui, icons::tool_icon(app.ui.tool), if t.pro { 26.0 } else { 28.0 }, !t.pro, tl!(app.ui.tool.label()));
                        // Photoshop: right-click the tool icon for Reset Tool / Reset All Tools (#2825).
                        let mut reset = None;
                        tool_icon.context_menu(|ui| {
                            if ui.button(tl!("Reset Tool")).clicked() {
                                reset = Some("tool.reset");
                                ui.close();
                            }
                            if ui.button(tl!("Reset All Tools")).clicked() {
                                reset = Some("tool.resetAll");
                                ui.close();
                            }
                        });
                        if let Some(id) = reset
                            && let Err(e) = crate::menus::invoke(app, ui.ctx(), id, json!({}))
                        {
                            app.ui.status = e;
                        }
                        if t.pro {
                            let (r, _) = ui.allocate_exact_size(vec2(10.0, 20.0), Sense::hover());
                            icons::paint(ui, r, "chevron-down", 10.0, t.text_faint);
                        }
                        widgets::vline(ui, 22.0);
                        if app.ui.transform.is_some() {
                            crate::transform_tool::options_bar(app, ui);
                            return;
                        }
                        if crate::symmetry_ui::options_bar(app, ui) || crate::distort_ui::options_bar(app, ui) {
                            return;
                        }
                        let tool = app.ui.tool;
                        // The options bar shows the active tool's brush: switch it in before drawing the
                        // chip, so a tool change swaps the brush with it (Photoshop keeps one per tool).
                        crate::paint_mouse::sync_tool_brush(app);
                        // Brush edits here go through `tools.setBrush`, one journal entry per gesture (Rule 1).
                        if (tool.is_brushlike() && !matches!(tool, Tool::Brush | Tool::Pencil | Tool::MixerBrush | Tool::Eraser))
                            || tool == Tool::QuickSelection
                        {
                            brush_preset_chip(ui, &app.session.tools.brush, &mut app.ui);
                            crate::brush_picker::settings_toggle(app, ui);
                            widgets::vline(ui, 22.0);
                        }
                        if matches!(tool, Tool::CloneStamp | Tool::PatternStamp | Tool::HistoryBrush | Tool::Healing) {
                            let before = app.session.tools.brush.clone();
                            let mut brush = before.clone();
                            // Mode is the tool's own brush's, kept per tool like its size.
                            if let Some(modes) = crate::blend_keys::tool_modes(tool) {
                                opt_label(ui, tl!("Mode"));
                                if let Some(Some(m)) =
                                    tool_mode_dropdown(ui, "retouch-mode", &modes, Some(crate::blend_keys::brush_tool_mode(tool, brush.mode)))
                                {
                                    brush.mode = m;
                                }
                            }
                            if tool == Tool::CloneStamp {
                                let width = if t.pro { 62.0 } else { 66.0 };
                                percent_field(ui, tl!("Opacity"), &mut brush.opacity, 1.0..=100.0, width);
                                percent_field(ui, tl!("Flow"), &mut brush.flow, 1.0..=100.0, width);
                            }
                            crate::brush_panel::commit_gesture(app, ui.ctx(), &before, &brush);
                            widgets::vline(ui, 22.0);
                        }
                        if crate::eraser_ui::options_bar(app, ui, tool)
                            || crate::retouch_ui::options_bar(app, ui, tool)
                            || crate::vector_ui::options_bar(app, ui, tool)
                            || crate::analysis_ui::options_bar(app, ui, tool)
                            || crate::slice_ui::options_bar(app, ui, tool)
                            || crate::frame_tool::options_bar(app, ui, tool)
                            || crate::artboard_tool::options_bar(app, ui, tool)
                        {
                            return;
                        }
                        let brush_before = app.session.tools.brush.clone();
                        let mut brush = brush_before.clone();
                        let b = &mut brush;
                        tool_specific_options(app, ui, t, tool, b);
                        // Painting symmetry is independent of visual theme. Keep the options-bar
                        // action available for Brush, Pencil and Eraser in every palette (#2659).
                        if matches!(tool, Tool::Brush | Tool::Pencil | Tool::Eraser) {
                            crate::symmetry_ui::menu(app, ui);
                        }
                        crate::brush_panel::commit_gesture(app, ui.ctx(), &brush_before, &brush);
                    });
                    reveal_focused_control(ui);
                });
            });
        });
}

/// The per-tool options of the bar: the brush, mixer, marquee, pencil and view controls.
/// Brush family options share the same per-tool settings model.
fn brush_specific_options(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: Tokens, b: &mut BrushSettings) {
    match app.ui.tool {
        Tool::Brush | Tool::Eraser if t.pro => {
            // A Block-mode Eraser has a fixed size, opacity and flow (Photoshop, #2770).
            let block = crate::eraser_ui::block_mode(app, app.ui.tool);
            ui.add_enabled_ui(!block, |ui| {
                brush_preset_chip(ui, b, &mut app.ui);
                crate::brush_picker::settings_toggle(app, ui);
            });
            widgets::vline(ui, 22.0);
            // The Eraser has no blend mode: its Mode is Brush, Pencil or Block (#2662).
            if app.ui.tool == Tool::Eraser {
                crate::eraser_ui::mode_dropdown(ui, &mut app.ui.tool_options.eraser_mode);
            } else {
                opt_label(ui, tl!("Mode"));
                let held = app.held_erase;
                paint_mode_dropdown(ui, "brush-mode", b, &mut app.ui.tool_options.brush_clear, held);
            }
            ui.add_enabled_ui(!block, |ui| {
                percent_field(ui, tl!("Opacity"), &mut b.opacity, 0.0..=100.0, 62.0);
                if icons::button(ui, "circle-dot", 24.0, b.pressure_opacity, tl!("Always use pressure for opacity")).clicked() {
                    b.pressure_opacity = !b.pressure_opacity;
                }
            });
            // A Pencil- or Block-mode Eraser is always full flow, without build-up (Photoshop).
            ui.add_enabled_ui(!block && !crate::eraser_ui::pencil_mode(app, app.ui.tool), |ui| {
                percent_field(ui, tl!("Flow"), &mut b.flow, 1.0..=100.0, 62.0);
                let airbrush = icons::button(ui, "sparkles", 24.0, b.build_up, tl!("Enable airbrush-style build-up effects"));
                if crate::brush_picker::named(airbrush, "Enable airbrush-style build-up effects").clicked() {
                    b.build_up = !b.build_up;
                }
            });
            opt_label(ui, tl!("Smoothing"));
            smoothing_field(ui, b, 58.0);
            smoothing_options(ui, b);
            widgets::vline(ui, 22.0);
            if icons::button(ui, "circle-dot", 24.0, b.pressure_size, tl!("Always use pressure for size")).clicked() {
                b.pressure_size = !b.pressure_size;
            }
        }
        // Pencil: Photoshop's options (no hardness or flow: the pencil is always hard).
        Tool::Pencil => {
            brush_preset_chip(ui, b, &mut app.ui);
            crate::brush_picker::settings_toggle(app, ui);
            widgets::vline(ui, 22.0);
            if !t.pro {
                opt_label(ui, tl!("Size"));
                widgets::value_field(ui, &mut b.size, 1.0..=photocraft_engine::paint::MAX_BRUSH_SIZE, "px", 76.0);
                widgets::vline(ui, 22.0);
            }
            opt_label(ui, tl!("Mode"));
            let held = app.held_erase;
            paint_mode_dropdown(ui, "pencil-mode", b, &mut app.ui.tool_options.pencil_clear, held);
            percent_field(ui, tl!("Opacity"), &mut b.opacity, 0.0..=100.0, if t.pro { 62.0 } else { 66.0 });
            opt_label(ui, tl!("Smoothing"));
            smoothing_field(ui, b, if t.pro { 58.0 } else { 66.0 });
            widgets::vline(ui, 22.0);
            widgets::checkbox(ui, &mut app.ui.tool_options.pencil_auto_erase, tl!("Auto Erase"));
        }
        Tool::Brush | Tool::Eraser => {
            brush_preset_chip(ui, b, &mut app.ui);
            crate::brush_picker::settings_toggle(app, ui);
            widgets::vline(ui, 22.0);
            // A Block-mode Eraser has a fixed size, opacity and flow (Photoshop, #2770).
            let block = crate::eraser_ui::block_mode(app, app.ui.tool);
            ui.add_enabled_ui(!block, |ui| {
                opt_label(ui, tl!("Size"));
                widgets::value_field(ui, &mut b.size, 1.0..=photocraft_engine::paint::MAX_BRUSH_SIZE, "px", 76.0);
            });
            widgets::vline(ui, 22.0);
            if app.ui.tool == Tool::Eraser {
                crate::eraser_ui::mode_dropdown(ui, &mut app.ui.tool_options.eraser_mode);
            } else {
                opt_label(ui, tl!("Mode"));
                let held = app.held_erase;
                paint_mode_dropdown(ui, "brush-mode", b, &mut app.ui.tool_options.brush_clear, held);
            }
            // A Pencil-mode Eraser is always hard and at full flow (Photoshop).
            let fixed = block || crate::eraser_ui::pencil_mode(app, app.ui.tool);
            ui.add_enabled_ui(!fixed, |ui| percent_field(ui, tl!("Hardness"), &mut b.hardness, 0.0..=100.0, 66.0));
            ui.add_enabled_ui(!block, |ui| percent_field(ui, tl!("Opacity"), &mut b.opacity, 0.0..=100.0, 66.0));
            ui.add_enabled_ui(!fixed, |ui| percent_field(ui, tl!("Flow"), &mut b.flow, 1.0..=100.0, 66.0));
            opt_label(ui, tl!("Smoothing"));
            smoothing_field(ui, b, 66.0);
            widgets::vline(ui, 22.0);
            widgets::toggle(ui, &mut b.pressure_size, tl!("Pressure for Size"));
            widgets::toggle(ui, &mut b.pressure_opacity, tl!("Pressure for Opacity"));
        }
        Tool::MixerBrush => {
            brush_preset_chip(ui, b, &mut app.ui);
            crate::brush_picker::settings_toggle(app, ui);
            widgets::vline(ui, 22.0);
            mixer_load_swatch(app, ui);
            let m = &mut b.mixer;
            for (icon, on, tip) in [
                ("brush", &mut m.load_after_stroke, tl!("Load the brush after each stroke")),
                ("brush-cleaning", &mut m.clean_after_stroke, tl!("Clean the brush after each stroke")),
            ] {
                if icons::button(ui, icon, 24.0, *on, tip).clicked() {
                    *on = !*on;
                }
            }
            widgets::vline(ui, 22.0);
            for (label, value) in
                [(tl!("Wet"), &mut b.mixer.wet), (tl!("Load"), &mut b.mixer.load), (tl!("Mix"), &mut b.mixer.mix), (tl!("Flow"), &mut b.mixer.flow)]
            {
                percent_field(ui, label, value, 0.0..=100.0, 62.0);
            }
            widgets::vline(ui, 22.0);
            widgets::checkbox(ui, &mut b.mixer.sample_all_layers, tl!("Sample All Layers"));
        }
        _ => {}
    }
}

/// Marquee tools: selection mode, feather, anti-alias, style and fixed size.
fn marquee_options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    ui.spacing_mut().item_spacing.x = 2.0;
    for (i, (icon, tip)) in [
        ("square", tl!("New selection").to_string()),
        ("plus", crate::i18n::fmt(tl!("Add to selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift"))])),
        ("minus", crate::i18n::fmt(tl!("Subtract from selection  ({key})"), &[("key", &crate::shortcuts::pretty("Alt"))])),
        ("squares-subtract", crate::i18n::fmt(tl!("Intersect with selection  ({key})"), &[("key", &crate::shortcuts::pretty("Shift+Alt"))])),
    ]
    .iter()
    .enumerate()
    {
        if icons::button(ui, icon, 24.0, app.ui.selection_mode == i as u8, tip).clicked() {
            app.ui.selection_mode = i as u8;
        }
    }
    ui.spacing_mut().item_spacing.x = 8.0;
    widgets::vline(ui, 22.0);
    let (units, size) = fixed_size_units(app);
    let o = &mut app.ui.tool_options;
    opt_label(ui, tl!("Feather"));
    widgets::value_field(ui, &mut o.feather, 0.0..=1000.0, "px", 62.0);
    // Photoshop greys Anti-alias for the Rectangular Marquee (its edges are always hard).
    ui.add_enabled_ui(app.ui.tool == Tool::EllipseMarquee, |ui| widgets::checkbox(ui, &mut o.anti_alias, tl!("Anti-alias")));
    widgets::vline(ui, 22.0);
    // A single row or column has no style or size: Photoshop greys them.
    let sized = matches!(app.ui.tool, Tool::RectMarquee | Tool::EllipseMarquee);
    ui.add_enabled_ui(sized, |ui| opt_label(ui, tl!("Style")));
    let styles = [("normal".to_string(), tl!("Normal")), ("fixedRatio".to_string(), tl!("Fixed Ratio")), ("fixedSize".to_string(), tl!("Fixed Size"))];
    if ui.add_enabled_ui(sized, |ui| widgets::dropdown(ui, "marquee-style", tl!("Style"), &mut o.marquee_style, &styles, 96.0)).inner {
        (o.marquee_width, o.marquee_height) = if o.marquee_style == "fixedSize" { (64.0, 64.0) } else { (1.0, 1.0) };
    }
    let fixed = sized && o.marquee_style != "normal";
    let fixed_size = o.marquee_style == "fixedSize";
    // Fixed Size shows its pixels in the unit picked by right-clicking Width or Height (#7910).
    let units = widgets::FieldUnits { unit: photocraft_engine::prefs::Unit::parse(&o.marquee_unit).unwrap_or_default(), ..units };
    ui.add_enabled_ui(fixed, |ui| {
        let mut picked = None;
        opt_label(ui, tl!("Width"));
        if fixed_size {
            picked = fixed_size_field(ui, &mut o.marquee_width, units, size[0]);
        } else {
            widgets::value_field(ui, &mut o.marquee_width, crate::tool_memory::style_range(&o.marquee_style), "", 62.0);
        }
        if icons::button(ui, "arrow-left-right", 22.0, false, tl!("Swaps height and width")).clicked() {
            std::mem::swap(&mut o.marquee_width, &mut o.marquee_height);
        }
        opt_label(ui, tl!("Height"));
        if fixed_size {
            picked = fixed_size_field(ui, &mut o.marquee_height, units, size[1]).or(picked);
        } else {
            widgets::value_field(ui, &mut o.marquee_height, crate::tool_memory::style_range(&o.marquee_style), "", 62.0);
        }
        if let Some(name) = picked {
            o.marquee_unit = name.into();
        }
    });
    widgets::vline(ui, 22.0);
    select_and_mask_button(app, ui);
}

/// Lasso family: selection mode, feather, anti-alias, magnetic settings and the drawing hints.
fn lasso_options(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    selection_mode_buttons(app, ui);
    widgets::vline(ui, 22.0);
    opt_label(ui, tl!("Feather"));
    widgets::value_field(ui, &mut app.ui.tool_options.feather, 0.0..=1000.0, "px", 62.0);
    widgets::checkbox(ui, &mut app.ui.tool_options.anti_alias, tl!("Anti-alias"));
    widgets::vline(ui, 22.0);
    if app.ui.tool == Tool::MagneticLasso {
        let o = &mut app.ui.tool_options;
        opt_label(ui, tl!("Width"));
        widgets::value_field(ui, &mut o.magnetic_width, 1.0..=256.0, "px", 58.0)
            .on_hover_text(tl!("Follows only edges this close to the pointer ([ and ] change it)"));
        opt_label(ui, tl!("Contrast"));
        widgets::value_field(ui, &mut o.magnetic_contrast, 1.0..=100.0, "%", 58.0)
            .on_hover_text(tl!("Higher values follow only edges that contrast sharply with their surroundings"));
        opt_label(ui, tl!("Frequency"));
        widgets::value_field(ui, &mut o.magnetic_frequency, 0.0..=100.0, "", 50.0).on_hover_text(tl!("How often fastening points are placed by themselves"));
        if icons::button(ui, "circle-dot", 24.0, o.magnetic_pressure, tl!("Use tablet pressure to change pen width")).clicked() {
            o.magnetic_pressure = !o.magnetic_pressure;
        }
        widgets::vline(ui, 22.0);
    }
    select_and_mask_button(app, ui);
    if crate::lasso_ui::active(app) || (app.ui.tool == Tool::PolygonLasso && !app.ui.polygon.is_empty()) {
        hint(ui, &crate::i18n::fmt(tl!("Click the first point or press {key} to close · Esc cancels"), &[("key", &crate::shortcuts::pretty("Enter"))]));
    } else if app.ui.tool == Tool::Lasso {
        hint(ui, tl!("Hold Alt while drawing for straight segments"));
    }
    if app.ui.tool == Tool::MagneticLasso && app.ui.magnetic.active() {
        hint(
            ui,
            &crate::i18n::fmt(
                tl!("Click the first point or press {key} to close · {del} removes a point · Esc cancels"),
                &[("key", &crate::shortcuts::pretty("Enter")), ("del", &crate::shortcuts::pretty("Backspace"))],
            ),
        );
    }
}

/// Selection and paint tools expose their controls as a family of options rows.
fn selection_specific_options(app: &mut PhotocraftApp, ui: &mut egui::Ui, tool: Tool) {
    match app.ui.tool {
        Tool::RectMarquee | Tool::EllipseMarquee | Tool::SingleRowMarquee | Tool::SingleColumnMarquee => {
            marquee_options(app, ui);
        }
        Tool::Lasso | Tool::PolygonLasso | Tool::MagneticLasso => {
            lasso_options(app, ui);
        }
        Tool::MagicWand => {
            selection_mode_buttons(app, ui);
            widgets::vline(ui, 22.0);
            opt_label(ui, tl!("Tolerance"));
            widgets::value_field(ui, &mut app.ui.tool_options.tolerance, 0.0..=255.0, "", 58.0);
            widgets::checkbox(ui, &mut app.ui.tool_options.anti_alias, tl!("Anti-alias"));
            widgets::checkbox(ui, &mut app.ui.tool_options.contiguous, tl!("Contiguous"));
            widgets::checkbox(ui, &mut app.ui.tool_options.sample_all.magic_wand, tl!("Sample All Layers"));
            widgets::vline(ui, 22.0);
            if widgets::secondary_button(ui, tl!("Select Subject"), 0.0).clicked() {
                let r = app.run("select.subject", json!({}));
                crate::retouch_ui::report_smart_result(app, r, tl!("No subject found — try Object Selection or the Lasso"));
            }
            select_and_mask_button(app, ui);
        }
        Tool::PaintBucket => {
            opt_label(ui, tl!("Fill"));
            let mut src = u8::from(app.ui.tool_options.bucket_fill_pattern);
            if widgets::dropdown(ui, "bucket-src", tl!("Source"), &mut src, &[(0u8, tl!("Foreground")), (1, tl!("Pattern"))], 100.0) {
                app.ui.tool_options.bucket_fill_pattern = src == 1;
            }
            opt_label(ui, tl!("Mode"));
            if let Some(modes) = crate::blend_keys::tool_modes(tool) {
                let o = &mut app.ui.tool_options;
                let current = if o.bucket_clear { None } else { Some(o.bucket_blend_mode) };
                if let Some(m) = tool_mode_dropdown(ui, "bucket-mode", &modes, current) {
                    o.bucket_clear = m.is_none();
                    if let Some(m) = m {
                        o.bucket_blend_mode = m;
                    }
                }
            }
            opt_label(ui, tl!("Opacity"));
            widgets::value_field(ui, &mut app.ui.tool_options.fill_opacity, 1.0..=100.0, "%", 62.0);
            opt_label(ui, tl!("Tolerance"));
            widgets::value_field(ui, &mut app.ui.tool_options.tolerance, 0.0..=255.0, "", 58.0);
            widgets::checkbox(ui, &mut app.ui.tool_options.anti_alias, tl!("Anti-alias"));
            widgets::checkbox(ui, &mut app.ui.tool_options.contiguous, tl!("Contiguous"));
            widgets::checkbox(ui, &mut app.ui.tool_options.sample_all.paint_bucket, tl!("All Layers"));
        }
        Tool::Gradient => {
            // Live "Gradient" (a Gradient Fill layer) or "Classic gradient" (pixels).
            let mut classic = app.ui.tool_options.gradient_classic;
            if widgets::dropdown(ui, "gradient-mode", tl!("Mode"), &mut classic, &[(false, "Gradient"), (true, "Classic gradient")], 118.0) {
                app.ui.tool_options.gradient_classic = classic;
            }
            // Live "Gradient" (a Gradient Fill layer) or "Classic gradient" (pixels):
            // both paint with the same engine-owned preset (#1651), and either swatch
            // click opens the Gradient Editor window, as in Photoshop.
            if crate::gradient_ui::preset_swatch(app, ui) {
                app.ui.panels.gradient_editor = true;
            }
            widgets::vline(ui, 22.0);
            ui.spacing_mut().item_spacing.x = 2.0;
            let before = app.ui.tool_options.clone();
            for (style, icon, tip) in [
                ("linear", "gradient", tl!("Linear Gradient")),
                ("radial", "circle", tl!("Radial Gradient")),
                ("angle", "rotate-cw", tl!("Angle Gradient")),
                ("reflected", "arrow-left-right", tl!("Reflected Gradient")),
                ("diamond", "diamond", tl!("Diamond Gradient")),
            ] {
                if icons::button(ui, icon, 24.0, app.ui.tool_options.gradient_style == style, tip).clicked() {
                    app.ui.tool_options.gradient_style = style.into();
                }
            }
            ui.spacing_mut().item_spacing.x = 8.0;
            widgets::vline(ui, 22.0);
            opt_label(ui, tl!("Mode"));
            let opts: Vec<(BlendMode, &str)> = BlendMode::LAYER_MODES.iter().map(|m| (*m, m.label())).collect();
            // The wheel over it steps the mode, as over the Layers panel's (#2061).
            widgets::dropdown_wheel_hovered(ui, "gradient-blend-mode", tl!("Blend Mode"), &mut app.ui.tool_options.gradient_blend_mode, &opts, 96.0);
            opt_label(ui, tl!("Opacity"));
            let r = widgets::value_field(ui, &mut app.ui.tool_options.fill_opacity, 1.0..=100.0, "%", 62.0);
            // Live while scrubbed, one undo step per scrub (#5012).
            let drag_key = r.id.with("gradient-opacity-drag");
            if r.drag_started() {
                let n = ui.ctx().cumulative_pass_nr();
                ui.data_mut(|d| d.insert_temp(drag_key, n));
            }
            if r.changed() {
                let drag = if r.dragged() { ui.data(|d| d.get_temp::<u64>(drag_key)) } else { None };
                crate::gradient_ui::opacity_changed(app, drag);
            }
            widgets::checkbox(ui, &mut app.ui.tool_options.gradient_reverse, tl!("Reverse"));
            widgets::checkbox(ui, &mut app.ui.tool_options.gradient_dither, tl!("Dither"));
            // Live mode: the options also change a selected gradient fill layer.
            crate::gradient_ui::options_changed(app, &before);
        }
        Tool::PerspectiveCrop => crate::perspective_crop_ui::options_bar(app, ui),
        Tool::Crop => {
            let before_ratio = crate::crop_ui::preset_ratio(app);
            let doc_size = app.session.active().map_or((0.0, 0.0), |s| (f64::from(s.doc.size.width), f64::from(s.doc.size.height)));
            // Ratios, sizes, saved presets, New / Delete Crop Preset… (#2443, #1919).
            crate::crop_presets::dropdown(app, ui);
            let o = &mut app.ui.tool_options;
            let mut ratio_base = None;
            if o.crop_ratio == crate::crop_size::WHR {
                crate::crop_size::fields(o, ui);
            } else {
                // Keep partial edits in ToolOptions so the first side survives a frame;
                // Original Ratio shows the document's numbers, as Photoshop does.
                let (mut rw, mut rh) = crate::crop_size::custom_ratio_fields(o, doc_size);
                let decimals = crate::crop_size::RATIO_DECIMALS;
                let fw = widgets::value_field_prec(ui, &mut rw, 0.0..=99_999.0, "", 54.0, decimals);
                let swap = icons::button(ui, "arrow-left-right", 22.0, false, tl!("Swaps height and width")).clicked();
                let fh = widgets::value_field_prec(ui, &mut rh, 0.0..=99_999.0, "", 54.0, decimals);
                let (cw, ch) = (fw.changed(), fh.changed());
                if swap {
                    std::mem::swap(&mut rw, &mut rh);
                }
                if cw || ch || swap {
                    crate::crop_size::set_custom_ratio_fields(o, rw, rh);
                }
                // Every ratio typed or scrubbed in one edit fits from the frame the edit
                // started with: 1:3 on the way to 16:3 mustn't shrink it first (#7752).
                let base_id = egui::Id::unique("crop-ratio-edit-base");
                if cw || ch {
                    ratio_base = ui.data(|d| d.get_temp::<[f64; 4]>(base_id)).or(app.ui.crop_rect);
                }
                if ![&fw, &fh].iter().any(|f| f.has_focus() || f.dragged()) {
                    ui.data_mut(|d| d.remove::<[f64; 4]>(base_id));
                } else if let Some(base) = ratio_base {
                    ui.data_mut(|d| d.insert_temp(base_id, base));
                }
            }
            widgets::vline(ui, 22.0);
            if widgets::secondary_button(ui, tl!("Clear"), 0.0).clicked() {
                crate::crop_size::clear_ratio(o);
            }
            crate::crop_straighten::options_button(&mut app.crop.straighten, ui);
            crate::crop_overlay::options_button(o, ui);
            let pick_shield_color = crate::crop_shield::options_button(&mut o.crop_shield, ui);
            widgets::checkbox(ui, &mut o.crop_delete, tl!("Delete Cropped Pixels"));
            // Fill: what a turned or extending crop puts where the image doesn't reach
            // (#3823). Generative Expand is not offered.
            ui.label(tl!("Fill:"));
            widgets::dropdown(
                ui,
                "crop-fill",
                tl!("Fill:"),
                &mut o.crop_content_aware,
                &[(false, tl!("Background (Default)")), (true, tl!("Content-Aware Fill"))],
                150.0,
            );
            if crate::crop_ui::preset_ratio(app) != before_ratio {
                match ratio_base {
                    Some(base) => crate::crop_ui::apply_ratio_from(app, base),
                    None => crate::crop_ui::apply_ratio(app),
                }
            }
            if pick_shield_color {
                crate::crop_shield::pick_custom_color(app);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icons::button(
                    ui,
                    "check",
                    26.0,
                    false,
                    &crate::i18n::fmt(tl!("Commit current crop operation  ({key})"), &[("key", &crate::shortcuts::pretty("Enter"))]),
                )
                .clicked()
                {
                    crate::canvas::commit_crop(app);
                }
                if icons::button(ui, "ban", 26.0, false, tl!("Cancel current crop operation  (Esc)")).clicked() {
                    crate::crop_ui::cancel(app);
                }
            });
        }
        // Every theme gets the font, style and size controls (#1387), as with Auto-Select (#1275).
        Tool::Type | Tool::VerticalType | Tool::HorizontalTypeMask | Tool::VerticalTypeMask => crate::type_tool::options_bar(app, ui),
        _ => {}
    }
}

fn is_selection_options_tool(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::RectMarquee
            | Tool::EllipseMarquee
            | Tool::SingleRowMarquee
            | Tool::SingleColumnMarquee
            | Tool::Lasso
            | Tool::PolygonLasso
            | Tool::MagneticLasso
            | Tool::MagicWand
            | Tool::PaintBucket
            | Tool::Gradient
            | Tool::PerspectiveCrop
            | Tool::Crop
            | Tool::Type
            | Tool::VerticalType
            | Tool::HorizontalTypeMask
            | Tool::VerticalTypeMask
    )
}

fn tool_specific_options(app: &mut PhotocraftApp, ui: &mut egui::Ui, t: Tokens, tool: Tool, b: &mut BrushSettings) {
    if matches!(app.ui.tool, Tool::Brush | Tool::Eraser | Tool::Pencil | Tool::MixerBrush) {
        brush_specific_options(app, ui, t, b);
        return;
    }
    if is_selection_options_tool(app.ui.tool) {
        selection_specific_options(app, ui, tool);
        return;
    }
    match app.ui.tool {
        Tool::Move => {
            let o = &mut app.ui.tool_options;
            widgets::checkbox(ui, &mut o.move_auto_select, tl!("Auto-Select:"));
            widgets::dropdown(
                ui,
                "move-target",
                tl!("Auto-Select:"),
                &mut o.move_target,
                &[("layer".to_string(), tl!("Layer")), ("group".to_string(), tl!("Group"))],
                76.0,
            );
            widgets::vline(ui, 22.0);
            widgets::checkbox(ui, &mut o.move_show_transform, tl!("Show Transform Controls"));
            widgets::vline(ui, 22.0);
            crate::align_bar::options(app, ui);
        }
        Tool::Eyedropper => crate::eyedropper_ui::options(app, ui),
        Tool::Zoom => {
            // Resize Windows to Fit is Preferences › Zoom Resizes Windows itself, as in
            // Photoshop: one setting, shown in both places (zoom_window_fit.rs).
            let mut fit = app.session.prefs().general.zoom_resizes_windows;
            if widgets::checkbox(ui, &mut fit, tl!("Resize Windows to Fit")).changed() {
                app.session.prefs.edit(|p| p.general.zoom_resizes_windows = fit);
            }
            widgets::checkbox(ui, &mut app.ui.tool_options.zoom_scrubby, tl!("Scrubby Zoom"));
            hint(
                ui,
                &crate::i18n::fmt(
                    tl!("Click to zoom in  ·  {key}-click to zoom out  ·  drag right/left to zoom in/out"),
                    &[("key", &crate::shortcuts::pretty("Alt"))],
                ),
            );
            if widgets::secondary_button(ui, tl!("Fit Screen"), 0.0).clicked()
                && let Some(i) = app.session.active_index()
            {
                app.ui.views[i].fit_pending = true;
                app.ui.views[i].request_zoom_animation(app.session.prefs().general.animated_zoom);
                app.ui.views[i].zoom_anchor = None;
            }
            if widgets::secondary_button(ui, "100%", 0.0).clicked()
                && let Some(i) = app.session.active_index()
            {
                app.ui.views[i].zoom = 1.0;
                app.ui.views[i].request_zoom_animation(app.session.prefs().general.animated_zoom);
                app.ui.views[i].zoom_anchor = None;
                app.ui.views[i].fit_pending = false;
                app.ui.views[i].fill_pending = false;
            }
        }
        Tool::RotateView => {
            hint(ui, tl!("Drag around the centre to rotate the view  ·  Shift constrains to 15°"));
            if let Some(i) = app.session.active_index() {
                ui.label(tl!("Angle"));
                let mut angle = app.ui.views.get(i).map(|v| v.rotation).unwrap_or(0.0);
                if widgets::value_field(ui, &mut angle, -180.0..=180.0, "°", 56.0).changed()
                    && let Some(v) = app.ui.views.get_mut(i)
                {
                    v.rotation = crate::rotate_view::wrap_deg(angle);
                }
                if widgets::secondary_button(ui, tl!("Reset View"), 0.0).clicked()
                    && let Some(v) = app.ui.views.get_mut(i)
                {
                    v.rotation = 0.0;
                }
            }
        }
        Tool::Hand => {
            hint(ui, tl!("Drag to pan  ·  hold Space with any tool"));
            if widgets::secondary_button(ui, "100%", 0.0).clicked()
                && let Some(i) = app.session.active_index()
            {
                app.ui.views[i].zoom = 1.0;
                app.ui.views[i].request_zoom_animation(app.session.prefs().general.animated_zoom);
                app.ui.views[i].zoom_anchor = None;
                app.ui.views[i].fit_pending = false;
                app.ui.views[i].fill_pending = false;
            }
            if widgets::secondary_button(ui, tl!("Fit Screen"), 0.0).clicked()
                && let Some(i) = app.session.active_index()
            {
                app.ui.views[i].fit_pending = true;
                app.ui.views[i].request_zoom_animation(app.session.prefs().general.animated_zoom);
                app.ui.views[i].zoom_anchor = None;
                app.ui.views[i].fill_pending = false;
            }
            if widgets::secondary_button(ui, tl!("Fill Screen"), 0.0).clicked()
                && let Some(i) = app.session.active_index()
            {
                app.ui.views[i].fill_pending = true;
                app.ui.views[i].request_zoom_animation(app.session.prefs().general.animated_zoom);
                app.ui.views[i].zoom_anchor = None;
                app.ui.views[i].fit_pending = false;
            }
        }
        // Retouching and smart-selection tools draw their bar in `retouch_ui::options_bar`.
        _ => {}
    }
}

/// Width of the Layers panel's Opacity and Fill fields, with room for their pop-up slider's ▾.
pub(super) const LAYER_PCT_W: f32 = 66.0 + widgets::POPUP_ARROW_W;

/// The `layer.setProps` edit of a Layers panel Opacity or Fill (`key`) field at `pct` percent. A
/// drag of the pop-up slider (`drag`) coalesces into one history step, as in Photoshop.
pub(super) fn pct_action(target: &Value, key: &str, pct: f32, drag: Option<u64>) -> (String, Value) {
    let mut p = target.clone();
    p[key] = json!(pct / 100.0);
    if let Some(n) = drag {
        p["coalesce"] = json!(format!("layer-{key}-slider:{n}"));
    }
    ("layer.setProps".into(), p)
}

/// Photoshop's scrubby Opacity/Fill labels: drag horizontally to change the percentage without
/// hitting the small number field. The unique drag ID coalesces every frame into one undo step.
pub(super) fn scrub_pct_label(ui: &mut egui::Ui, text: &str, percent: &mut f32, max_w: f32) -> widgets::PopupFieldResponse {
    // Elide to the room left in the row (full text on hover): a longer translation shrinks here
    // instead of running over the fixed controls beside it (#3580).
    let resp = widgets::elided_label_sensed(ui, text, max_w, Sense::click_and_drag()).on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
    let key = resp.id.with("percent-label-drag");
    if resp.drag_started() {
        let stamp = ui.ctx().cumulative_pass_nr();
        ui.data_mut(|d| d.insert_temp(key, (*percent, stamp)));
    }
    let mut out = widgets::PopupFieldResponse::default();
    if resp.dragged()
        && let Some((start, stamp)) = ui.data(|d| d.get_temp::<(f32, u64)>(key))
        && let (Some(origin), Some(pos)) = (ui.input(|i| i.pointer.press_origin()), resp.interact_pointer_pos())
    {
        // Match DragValue's 0.5 percentage-point per screen point; do not jump on press.
        let next = (start + (pos.x - origin.x) * 0.5).clamp(0.0, 100.0);
        if next.is_finite() && next != *percent {
            *percent = next;
            out.changed = true;
            out.drag = Some(stamp);
        }
    }
    if resp.drag_stopped() {
        ui.data_mut(|d| d.remove::<(f32, u64)>(key));
    }
    out
}

/// Width of `s` as [`label`] draws it (body text).
pub(super) fn body_text_width(ui: &egui::Ui, s: &str) -> f32 {
    let font = egui::TextStyle::Body.resolve(ui.style());
    ui.painter().layout_no_wrap(s.to_string(), font, Color32::WHITE).size().x
}

/// Options-bar label: Photoshop writes "Size:" with a colon.
/// An options-bar label; one right before a value field scrubs it when dragged (Photoshop).
pub(crate) fn opt_label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    let text = if t.pro && !s.ends_with(':') { format!("{s}:") } else { s.to_string() };
    let label = ui.add(egui::Label::new(RichText::new(tl!(&text)).color(t.text_dim)).selectable(false));
    widgets::scrub_option_label(ui, &label);
}

/// The largest Fixed Size side, in pixels.
pub(crate) const FIXED_SIZE_MAX_PX: f64 = 300_000.0;

/// The resolution Fixed Size's units convert at (the active document's, 72 ppi without one) and
/// the document's size in pixels, which a percentage is of. The unit is the caller's.
fn fixed_size_units(app: &PhotocraftApp) -> (widgets::FieldUnits, [f64; 2]) {
    let ppi = app.session.prefs().units_and_rulers.point_size.per_inch();
    let (dpi, size) = app
        .session
        .active()
        .map_or((72.0, [1.0, 1.0]), |d| (f64::from(d.doc.resolution_dpi.max(1.0)), [f64::from(d.doc.size.width), f64::from(d.doc.size.height)]));
    (widgets::FieldUnits { unit: photocraft_engine::prefs::Unit::Pixels, dpi, ppi }, size)
}

/// One of Fixed Size's Width and Height (#7910): `px` shown in `units.unit` (a typed unit converts),
/// still stored in pixels. A right-click lists the units, as on the rulers; returns the one picked.
fn fixed_size_field(ui: &mut egui::Ui, px: &mut f32, units: widgets::FieldUnits, extent: f64) -> Option<&'static str> {
    let widgets::FieldUnits { unit, dpi, ppi } = units;
    let to_unit = |px: f64| unit.from_px(px, dpi, extent, ppi) as f32;
    let before = to_unit(f64::from(*px));
    let mut v = before;
    let places = unit.decimals() as u32;
    let resp = widgets::value_field_extent(ui, &mut v, to_unit(1.0)..=to_unit(FIXED_SIZE_MAX_PX), unit.suffix(), 62.0, places, units, extent);
    // Leaving the field re-reads the rounded text it showed (100 px at 72 ppi reads back as
    // 1.389 in); that isn't an edit, so the pixels stay exact.
    let shown = |x: f32| (f64::from(x) * 10f64.powi(places as i32)).round();
    if resp.changed() && shown(v) != shown(before) {
        *px = unit.to_px(f64::from(v), dpi, extent, ppi).clamp(1.0, FIXED_SIZE_MAX_PX) as f32;
    }
    let mut picked = None;
    resp.context_menu(|ui| picked = crate::rulers::unit_menu(ui, unit));
    picked
}

fn percent_field(ui: &mut egui::Ui, label: &str, value: &mut f32, range: std::ops::RangeInclusive<f32>, width: f32) {
    opt_label(ui, label);
    let mut percent = *value * 100.0;
    if widgets::value_field_named(ui, label, &mut percent, range, "%", width).changed() {
        *value = percent / 100.0;
    }
}

fn hint(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(s).color(t.text_faint));
}
