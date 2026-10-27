use super::options_bar::reveal_focused_control;
use super::*;

// ----------------------------------------------------------------------------- toolbar

/// Tool groups separated by hairlines, Photoshop order.
/// Photoshop toolbar: divider-separated sections of slots; each slot is a flyout group whose button
/// shows the group's last-used tool.
pub(super) const TOOL_SECTIONS: &[&[&[Tool]]] = &[
    &[&[Tool::Move, Tool::Artboard]],
    &[
        &[Tool::RectMarquee, Tool::EllipseMarquee, Tool::SingleRowMarquee, Tool::SingleColumnMarquee],
        &[Tool::Lasso, Tool::PolygonLasso, Tool::MagneticLasso],
        &[Tool::ObjectSelection, Tool::QuickSelection, Tool::MagicWand],
        &[Tool::Crop, Tool::PerspectiveCrop, Tool::Slice, Tool::SliceSelect],
        &[Tool::Frame],
        &[Tool::Eyedropper, Tool::ColorSampler, Tool::Ruler, Tool::Note, Tool::Count],
    ],
    &[
        &[Tool::Remove, Tool::SpotHealing, Tool::Healing, Tool::Patch, Tool::ContentAwareMove, Tool::RedEye],
        &[Tool::Brush, Tool::Pencil, Tool::MixerBrush, Tool::ColorReplacement],
        &[Tool::CloneStamp, Tool::PatternStamp],
        &[Tool::HistoryBrush, Tool::ArtHistoryBrush],
        &[Tool::Eraser, Tool::BackgroundEraser, Tool::MagicEraser],
        &[Tool::Gradient, Tool::PaintBucket],
        &[Tool::Blur, Tool::Sharpen, Tool::Smudge],
        &[Tool::Dodge, Tool::Burn, Tool::Sponge],
    ],
    &[
        &[Tool::Pen, Tool::FreeformPen, Tool::CurvaturePen, Tool::AddAnchorPoint, Tool::DeleteAnchorPoint, Tool::ConvertPoint],
        &[Tool::Type, Tool::VerticalType, Tool::VerticalTypeMask, Tool::HorizontalTypeMask],
        &[Tool::PathSelection, Tool::DirectSelection],
        &[Tool::Rectangle, Tool::EllipseShape, Tool::Triangle, Tool::Polygon, Tool::Line, Tool::CustomShape],
    ],
    &[&[Tool::Hand, Tool::RotateView], &[Tool::Zoom]],
];

/// The tool a slot shows: the current tool if it belongs to the slot, else the last one used.
fn slot_tool(ui: &egui::Ui, current: Tool, slot: &[Tool], key: egui::Id) -> Tool {
    if slot.contains(&current) {
        ui.data_mut(|d| d.insert_temp(key, current));
        return current;
    }
    ui.data(|d| d.get_temp::<Tool>(key)).filter(|t| slot.contains(t)).unwrap_or(slot[0])
}

/// [`TOOL_SECTIONS`] without the tools hidden in Edit › Toolbar (`hidden` holds tool names: the
/// dialog writes `Tool` debug names, `edit.toolbar` takes any name `Tool::from_name` reads). A
/// slot keeps only its visible tools, so its flyout lists just those; a slot with none left is
/// dropped, and so is a section with no slots. Each slot carries its index in `TOOL_SECTIONS`,
/// so its remembered tool and flyout stay put when other slots are hidden.
pub(super) fn visible_sections(hidden: &[String]) -> Vec<Vec<(usize, Vec<Tool>)>> {
    let hidden: Vec<Tool> = hidden.iter().map(String::as_str).filter_map(Tool::from_name).collect();
    let mut index = 0usize;
    let mut sections = Vec::new();
    for section in TOOL_SECTIONS {
        let mut slots = Vec::new();
        for slot in section.iter() {
            let tools: Vec<Tool> = slot.iter().copied().filter(|tool| !hidden.contains(tool)).collect();
            if !tools.is_empty() {
                slots.push((index, tools));
            }
            index += 1;
        }
        if !slots.is_empty() {
            sections.push(slots);
        }
    }
    sections
}

/// A primary-button gesture started on this tool; open only after it leaves its hit area.
/// Using pointer origin rather than `Response::dragged` also works for click-only icon buttons.
pub(super) fn tool_flyout_drag_open(button: Rect, origin: Option<Pos2>, pointer: Option<Pos2>, primary_down: bool) -> bool {
    primary_down && origin.is_some_and(|p| button.contains(p)) && pointer.is_some_and(|p| !button.contains(p))
}

/// A toolbar press cannot click a flyout row in egui because the press began elsewhere.
pub(super) fn tool_flyout_release(row: Rect, pointer: Option<Pos2>, released: bool, from_button: bool) -> bool {
    released && from_button && pointer.is_some_and(|p| row.contains(p))
}

pub fn toolbar(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let (w1, bx, m) = if t.pro { (40.0, 30.0, 5i8) } else { (50.0, 36.0, 7i8) };
    let sections = visible_sections(&app.session.prefs().toolbar.hidden);
    // Two columns when the header chevron asks for them; a column that doesn't fit scrolls.
    let double = app.ui.panels.toolbar_double;
    let w = if double { w1 + bx + 2.0 } else { w1 };
    crate::direction::start_panel("toolbar")
        .resizable(false)
        .exact_size(w)
        .frame(egui::Frame::NONE.fill(t.chrome).inner_margin(egui::Margin::symmetric(m, 8)))
        .show(ui, |ui| {
            // Toolbar width is a user preference, not a Pro-only feature.
            // Keep the same hit target in every theme (including RTL layouts).
            let (cr, resp) = ui.allocate_exact_size(vec2(bx, 14.0), Sense::click());
            let (shrink, grow) = if crate::direction::is_rtl() { ("chevrons-right", "chevrons-left") } else { ("chevrons-left", "chevrons-right") };
            icons::paint(ui, cr, if double { shrink } else { grow }, 11.0, if resp.hovered() { t.text } else { t.text_faint });
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Toolbar")));
            if resp.clicked() {
                app.ui.panels.toolbar_double = !double;
            }
            ui.add_space(4.0);
            // Subtle violet wash at the bottom of the toolbar.
            let full = ui.max_rect();
            if !t.bevel && !t.pro && t.dark() {
                let mut mesh = egui::Mesh::default();
                let r = Rect::from_min_max(pos2(full.left() - 7.0, full.bottom() - 260.0), pos2(full.right() + 7.0, full.bottom() + 8.0));
                let top = Color32::TRANSPARENT;
                let bottom = Color32::from_rgba_unmultiplied(90, 70, 190, 34);
                mesh.colored_vertex(r.left_top(), top);
                mesh.colored_vertex(r.right_top(), top);
                mesh.colored_vertex(r.right_bottom(), bottom);
                mesh.colored_vertex(r.left_bottom(), bottom);
                mesh.add_triangle(0, 1, 2);
                mesh.add_triangle(0, 2, 3);
                ui.painter().add(mesh);
            }
            ui.spacing_mut().item_spacing = vec2(2.0, 3.0);
            let flyout_id = egui::Id::unique("tool-flyout");
            let held_id = flyout_id.with("held");
            if ui.input(|i| i.pointer.any_pressed()) {
                ui.data_mut(|d| d.remove::<egui::Id>(held_id));
            }
            // No tool tooltips over an open flyout, or while a button is held to open one (#4450):
            // Photoshop shows none there. (The flyout is our own Area, so egui's rule for popups
            // doesn't hide them.)
            let tool_tips = ui.data(|d| d.get_temp::<(egui::Id, Rect)>(flyout_id)).is_none() && !ui.input(|i| i.pointer.primary_down());
            let ctx = ui.ctx().clone();
            let footer = (if t.pro { 8.0 } else { 14.0 })
                + chips_height(t.pro, t.round_chips, t.round_chips && double)
                + 8.0
                + if double { bx } else { bx * 2.0 + 3.0 }
                + 6.0;
            // As tall as the tools, so the colour chips sit right under them (Photoshop); a column
            // taller than the window scrolls and keeps the chips at the bottom.
            egui::ScrollArea::vertical()
                .id_salt("toolbar-columns")
                .auto_shrink([false, true])
                .max_height((ui.available_height() - footer).max(0.0))
                .min_scrolled_height(0.0)
                .show(ui, |ui| {
                    // Pro has no group dividers, so its two columns fill every row across groups.
                    let sections: Vec<Vec<(usize, Vec<Tool>)>> = if t.pro { vec![sections.into_iter().flatten().collect()] } else { sections };
                    for (si, section) in sections.iter().enumerate() {
                        // Photoshop 2026 draws one uninterrupted column (no group dividers).
                        if si > 0 && !t.pro {
                            ui.add_space(4.0);
                            let (r, _) = ui.allocate_exact_size(vec2(if double { bx * 2.0 + 2.0 } else { bx }, 1.0), Sense::hover());
                            ui.painter().line_segment([r.left_center() + vec2(8.0, 0.0), r.right_center() - vec2(8.0, 0.0)], Stroke::new(1.0, t.separator));
                            ui.add_space(4.0);
                        }
                        let rows: Vec<&[(usize, Vec<Tool>)]> = if double { section.chunks(2).collect() } else { section.chunks(1).collect() };
                        for row in rows {
                            ui.horizontal(|ui| {
                                for (slot_index, slot) in row.iter() {
                                    let key = egui::Id::unique(("tool-slot", *slot_index));
                                    let tool = slot_tool(ui, app.ui.tool, slot, key);
                                    let sel = slot.contains(&app.ui.tool);
                                    let tip = match crate::tool_shortcuts::shortcut(app, tool) {
                                        Some(sc) => format!("{}  ({})", tl!(tool.label()), crate::shortcuts::pretty(&sc)),
                                        None => tl!(tool.label()).to_string(),
                                    };
                                    let resp =
                                        icons::button_with_icon_size_tip(ui, icons::tool_icon(tool), bx, toolbar_icon_size(bx, t.pro), sel, &tip, tool_tips);
                                    if slot.len() > 1 {
                                        paint_toolbar_flyout_indicator(ui, &t, &resp);
                                    }
                                    if resp.clicked() && ui.data(|d| d.get_temp::<egui::Id>(held_id)) != Some(key) {
                                        crate::tool_memory::select(app, tool);
                                    }
                                    // Double-clicking the Hand tool fits the image on screen (Photoshop).
                                    if resp.double_clicked() && tool == Tool::Hand {
                                        crate::tool_memory::select(app, tool);
                                        // With no document open there is nothing to fit.
                                        let _ = app.run("view.fitOnScreen", json!({}));
                                    }
                                    if resp.double_clicked() && tool == Tool::Zoom {
                                        crate::tool_memory::select(app, tool);
                                        let _ = app.run("view.actualPixels", json!({}));
                                    }
                                    // Right-click, long-press, or press-drag away opens the flyout (Photoshop).
                                    let held_for =
                                        resp.is_pointer_button_down_on().then(|| ui.input(|i| i.pointer.press_start_time().map(|t0| i.time - t0))).flatten();
                                    if slot.len() > 1
                                        && let Some(seconds) = held_for
                                        && seconds < 0.35
                                    {
                                        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(0.35 - seconds));
                                    }
                                    let long_press = held_for.is_some_and(|seconds| seconds >= 0.35);
                                    let drag_open = ui
                                        .input(|i| tool_flyout_drag_open(resp.rect, i.pointer.press_origin(), i.pointer.hover_pos(), i.pointer.primary_down()));
                                    // Clicking the button of its open flyout again, with either button, closes
                                    // it (#4932); the release that ends the long press that opened it doesn't.
                                    let open_here = ui.data(|d| d.get_temp::<(egui::Id, Rect)>(flyout_id)).is_some_and(|(k, _)| k == key);
                                    let reclicked = open_here
                                        && (resp.secondary_clicked() || (resp.clicked() && ui.data(|d| d.get_temp::<egui::Id>(held_id)) != Some(key)));
                                    if reclicked {
                                        ui.data_mut(|d| d.remove::<(egui::Id, Rect)>(flyout_id));
                                    } else if slot.len() > 1 && (resp.secondary_clicked() || long_press || drag_open) {
                                        ui.data_mut(|d| {
                                            d.insert_temp(flyout_id, (key, resp.rect));
                                            if long_press || drag_open {
                                                d.insert_temp(held_id, key);
                                            }
                                        });
                                    }
                                    if slot.len() > 1 && long_press {
                                        // Holding a tool button selects the tool it shows, as a click would (#4453).
                                        crate::tool_memory::select(app, tool);
                                        ui.ctx().request_repaint();
                                    }
                                    if let Some((open_key, anchor)) = ui.data(|d| d.get_temp::<(egui::Id, Rect)>(flyout_id))
                                        && open_key == key
                                    {
                                        // Beside the tool, towards the canvas.
                                        let (pivot, pos) = if crate::direction::is_rtl() {
                                            (egui::Align2::RIGHT_TOP, anchor.left_top() - vec2(6.0, 0.0))
                                        } else {
                                            (egui::Align2::LEFT_TOP, anchor.right_top() + vec2(6.0, 0.0))
                                        };
                                        let area = egui::Area::new(key.with("flyout")).order(egui::Order::Foreground).pivot(pivot).fixed_pos(pos).show(
                                            ui.ctx(),
                                            |ui| {
                                                egui::Frame::popup(ui.style()).show(ui, |ui| {
                                                    let label_w = slot
                                                        .iter()
                                                        .map(|it| {
                                                            ui.painter()
                                                                .layout_no_wrap(tl!(it.label()).to_string(), egui::FontId::proportional(12.5), t.text)
                                                                .size()
                                                                .x
                                                        })
                                                        .fold(0.0f32, f32::max);
                                                    let fw = (label_w + 42.0 + 40.0).max(180.0);
                                                    for &item in slot.iter() {
                                                        let (r, ir) = ui.allocate_exact_size(vec2(fw, 26.0), Sense::click());
                                                        if ir.hovered() {
                                                            ui.painter().rect_filled(r, 3.0, t.hover);
                                                        }
                                                        if item == tool {
                                                            ui.painter().rect_filled(
                                                                Rect::from_center_size(pos2(r.left() + 8.0, r.center().y), vec2(4.0, 4.0)),
                                                                0.0,
                                                                t.text,
                                                            );
                                                        }
                                                        icons::paint(
                                                            ui,
                                                            Rect::from_center_size(pos2(r.left() + 26.0, r.center().y), vec2(18.0, 18.0)),
                                                            icons::tool_icon(item),
                                                            14.0,
                                                            t.icon,
                                                        );
                                                        ui.painter().text(
                                                            pos2(r.left() + 42.0, r.center().y),
                                                            Align2::LEFT_CENTER,
                                                            tl!(item.label()),
                                                            egui::FontId::proportional(12.5),
                                                            t.text,
                                                        );
                                                        if let Some(sc) = crate::tool_shortcuts::shortcut(app, item) {
                                                            ui.painter().text(
                                                                pos2(r.right() - 8.0, r.center().y),
                                                                Align2::RIGHT_CENTER,
                                                                crate::shortcuts::pretty(&sc),
                                                                egui::FontId::proportional(12.0),
                                                                t.text_dim,
                                                            );
                                                        }
                                                        let drag_release = tool_flyout_release(
                                                            r,
                                                            ui.input(|i| i.pointer.hover_pos()),
                                                            ui.input(|i| i.pointer.primary_released()),
                                                            ui.data(|d| d.get_temp::<egui::Id>(held_id)) == Some(key),
                                                        );
                                                        if ir.clicked() || drag_release {
                                                            crate::tool_memory::select(app, item);
                                                            ui.data_mut(|d| d.remove::<(egui::Id, Rect)>(flyout_id));
                                                        }
                                                    }
                                                });
                                            },
                                        );
                                        // Any press outside the flyout and its button closes it, as in Photoshop: a
                                        // drag on the canvas (cropping, painting) isn't a click (#4460).
                                        let pressed_outside = ui.input(|i| {
                                            i.pointer.any_pressed()
                                                && i.pointer.press_origin().is_some_and(|p| !area.response.rect.contains(p) && !resp.rect.contains(p))
                                        });
                                        if pressed_outside || ui.input(|i| i.key_pressed(egui::Key::Escape)) {
                                            ui.data_mut(|d| d.remove::<(egui::Id, Rect)>(flyout_id));
                                        }
                                    }
                                }
                            });
                        }
                    }
                    if icons::button(ui, "ellipsis", bx, false, tl!("Edit Toolbar…")).clicked() {
                        let _ = crate::menus::invoke(app, &ctx, "edit.toolbar", json!({}));
                    }
                    reveal_focused_control(ui);
                });
            ui.add_space(if t.pro { 8.0 } else { 14.0 });
            // Centred in the tool column as it is before the tools scroll: a scroll bar widens the
            // UI it is in, and the chips moved over by its width (#7316).
            color_chips(app, ui, double, full.width());
            ui.add_space(8.0);

            let layout = if double { egui::Layout::left_to_right(egui::Align::Min) } else { egui::Layout::top_down(egui::Align::Min) };

            ui.with_layout(layout, |ui| {
                let quick_mask = app.session.active().is_some_and(|s| s.doc.quick_mask.is_some());
                if icons::button_with_icon_size_tip(
                    ui,
                    "square-dashed",
                    bx,
                    (bx * 0.52).round(),
                    quick_mask,
                    if quick_mask { tl!("Edit in Standard Mode  (Q)") } else { tl!("Edit in Quick Mask Mode  (Q)") },
                    tool_tips,
                )
                .clicked()
                {
                    let _ = crate::menus::invoke(app, &ctx, "select.editInQuickMaskMode", json!({}));
                }
                let sm = icons::button_with_icon_size_tip(ui, "app-window", bx, (bx * 0.52).round(), false, tl!("Change Screen Mode  (F)"), tool_tips);
                paint_toolbar_flyout_indicator(ui, &t, &sm);

                // Right-click, or click and hold like the tool flyouts (#4444), lists the modes,
                // like Photoshop's flyout. The release that ends a hold doesn't cycle the mode.
                let held_id = sm.id.with("held");
                let was_held = ui.data(|d| d.get_temp::<bool>(held_id)).unwrap_or(false);
                let held_for = sm.is_pointer_button_down_on().then(|| ui.input(|i| i.pointer.press_start_time().map(|t0| i.time - t0))).flatten();
                if let Some(seconds) = held_for
                    && seconds < 0.35
                {
                    ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64(0.35 - seconds));
                }
                let long_press = !was_held && held_for.is_some_and(|seconds| seconds >= 0.35);
                if long_press {
                    ui.data_mut(|d| d.insert_temp(held_id, true));
                } else if was_held && !sm.is_pointer_button_down_on() {
                    ui.data_mut(|d| d.remove::<bool>(held_id));
                }
                let cycle = sm.clicked() && !was_held;
                if cycle {
                    let _ = crate::menus::invoke(app, &ctx, "view.screenMode.cycle", json!({}));
                }
                let open = if sm.secondary_clicked() || long_press {
                    Some(egui::SetOpenCommand::Bool(true))
                } else if cycle {
                    Some(egui::SetOpenCommand::Bool(false))
                } else {
                    None
                };
                // The release ending the hold counts as a click; it mustn't close the list it opened.
                let close = if was_held || long_press { egui::PopupCloseBehavior::IgnoreClicks } else { egui::PopupCloseBehavior::CloseOnClick };
                egui::Popup::menu(&sm).open_memory(open).close_behavior(close).at_pointer_fixed().show(|ui| {
                    ui.set_min_width(220.0);
                    for (id, label) in [
                        ("view.screenMode.standard", tl!("Standard Screen Mode")),
                        ("view.screenMode.fullScreenWithMenuBar", tl!("Full Screen Mode With Menu Bar")),
                        ("view.screenMode.fullScreen", tl!("Full Screen Mode")),
                    ] {
                        let on = crate::view_cmds::checked(app, id).unwrap_or(false);
                        if ui.add(egui::Button::selectable(on, label)).clicked() {
                            let _ = crate::menus::invoke(app, &ctx, id, json!({}));
                            ui.close();
                        }
                    }
                });
            });
        });
}

/// Glyph size of a toolbar tool button. Pro fills two thirds of the button, close to Photoshop's
/// toolbar glyphs (#2665); the other themes keep the general icon-button proportion.
pub(super) fn toolbar_icon_size(box_size: f32, pro: bool) -> f32 {
    (box_size * if pro { 2.0 / 3.0 } else { 0.52 }).round()
}

/// Paints a triangle on the bottom corner of an icon button that faces the canvas (bottom right,
/// bottom left in a right-to-left UI) to indicate that a toolbar item has a submenu.
pub fn paint_toolbar_flyout_indicator(ui: &mut egui::Ui, tokens: &Tokens, resp: &egui::Response) {
    let r = resp.rect;
    let tri = if crate::direction::is_rtl() {
        vec![r.left_bottom() + vec2(2.0, -2.0), r.left_bottom() + vec2(6.0, -2.0), r.left_bottom() + vec2(2.0, -6.0)]
    } else {
        vec![r.right_bottom() + vec2(-2.0, -2.0), r.right_bottom() + vec2(-6.0, -2.0), r.right_bottom() + vec2(-2.0, -6.0)]
    };
    ui.painter().add(egui::Shape::convex_polygon(tri, tokens.text_faint, Stroke::NONE));
}

/// Colour chips: chip size and the background chip's offset (points).
fn chip_metrics(pro: bool) -> (f32, f32) {
    if pro { (18.0, 10.0) } else { (21.0, 12.0) }
}

/// Height of the default-colours and swap icons over the chips.
const CHIP_ICON: f32 = 13.0;

/// Studio themes: round chip diameter and the background chip's vertical offset (points).
pub(super) const STUDIO_CHIP: (f32, f32) = (30.0, 19.0);

/// Room under the round chips for the Default Colors and Switch Colors buttons.
const STUDIO_ROW: f32 = 15.0;

/// Room above the stacked round chips so Default Colors, at their top-right, clears them.
const STUDIO_TOP: f32 = 6.0;

/// Height of the colour chips block.
pub(super) fn chips_height(pro: bool, studio: bool, wide: bool) -> f32 {
    if studio && wide {
        return STUDIO_CHIP.0 + STUDIO_ROW;
    }
    if studio {
        return STUDIO_TOP + STUDIO_CHIP.0 + STUDIO_CHIP.1 + STUDIO_ROW;
    }
    let (chip, step) = chip_metrics(pro);
    CHIP_ICON + 3.0 + chip + step
}

/// Photoshop's foreground / background colour chips: the Default Colors (D) and Switch Colors (X)
/// icons above them, the foreground chip over the background one, all centred in the toolbar
/// column and kept inside it.
pub(super) fn color_chips(app: &mut PhotocraftApp, ui: &mut egui::Ui, two_columns: bool, column: f32) {
    let t = Tokens::get(ui.ctx());
    // Studio themes: round chips stacked vertically, the controls below them.
    let studio = t.round_chips;
    let (chip, step) = if studio { STUDIO_CHIP } else { chip_metrics(t.pro) };
    // With two tool columns (a short window) the round chips lie side by side, which saves height.
    let wide = studio && two_columns;
    let group = if studio && !wide { chip } else { chip + step };
    let w = column.max(group);
    let (rect, _) = ui.allocate_exact_size(vec2(w, chips_height(t.pro, studio, wide)), Sense::hover());
    let left = (rect.left() + (w - group) / 2.0).round();
    let (fg, bg) = if wide {
        let fg = Rect::from_min_size(pos2(left, rect.top()), vec2(chip, chip));
        (fg, fg.translate(vec2(step, 0.0)))
    } else if studio {
        let fg = Rect::from_min_size(pos2(left, rect.top() + STUDIO_TOP), vec2(chip, chip));
        (fg, fg.translate(vec2(0.0, step)))
    } else {
        let fg = Rect::from_min_size(pos2(left, rect.top() + CHIP_ICON + 3.0), vec2(chip, chip));
        (fg, fg.translate(vec2(step, step)))
    };
    let radius = t.radius_sm.min(3.0);
    let p = ui.painter();
    if studio {
        let dot = |r: Rect, color: Color32| {
            p.circle_filled(r.center(), r.width() / 2.0, color);
            // A light line inside a dark one, so any colour reads against the toolbar.
            p.circle_stroke(r.center(), r.width() / 2.0 - 0.5, Stroke::new(1.0, t.text.gamma_multiply(0.9)));
            p.circle_stroke(r.center(), r.width() / 2.0 + 0.5, Stroke::new(1.0, t.field_border));
        };
        dot(bg, c32(app.session.tools.background));
        // The foreground chip sits on the background one, a toolbar-coloured gap between them.
        p.circle_filled(fg.center(), fg.width() / 2.0 + 2.0, t.chrome);
        dot(fg, c32(app.session.tools.foreground));
    } else {
        let frame = |r: Rect| {
            // A light line inside a dark one, so any colour reads against the toolbar.
            p.rect_stroke(r, radius, Stroke::new(1.0, t.text.gamma_multiply(0.9)), StrokeKind::Inside);
            p.rect_stroke(r, radius, Stroke::new(1.0, t.field_border), StrokeKind::Outside);
        };
        p.rect_filled(bg, radius, c32(app.session.tools.background));
        frame(bg);
        p.rect_filled(fg.expand(2.0), radius + 2.0, t.chrome);
        p.rect_filled(fg, radius, c32(app.session.tools.foreground));
        frame(fg);
    }
    // Photoshop: clicking a chip opens the Color Picker for that colour.
    let bg_resp = ui.interact(bg, ui.scope_id().with("bgchip"), Sense::click());
    let fg_resp = ui.interact(fg, ui.scope_id().with("fgchip"), Sense::click());
    if fg_resp.on_hover_text(tl!("Set foreground color")).clicked() {
        crate::color_picker_ui::open(app, "foreground");
    } else if bg_resp.on_hover_text(tl!("Set background color")).clicked() {
        crate::color_picker_ui::open(app, "background");
    }
    // Default Colors at the left, Switch Colors at the right, over the chips.
    let (dr, sr) = if studio {
        // Switch Colors is a curve right under the chips, centred on them so its heads touch
        // them. Default Colors never moves it: at the left edge beside it when the chips lie side
        // by side, at the block's top-right corner when they are stacked.
        let (cx, row) = ((fg.center().x + bg.center().x) / 2.0, fg.bottom().max(bg.bottom()) + 3.5);
        let default = if wide {
            Rect::from_center_size(pos2(rect.left() + CHIP_ICON / 2.0, row), vec2(CHIP_ICON, CHIP_ICON))
        } else {
            // Just high enough that its bottom-left corner clears the foreground chip's outline.
            let x = rect.right() - CHIP_ICON;
            let (dx, r) = ((x - fg.center().x).max(0.0), fg.width() / 2.0 + 1.0);
            let clear = if dx < r { (r * r - dx * dx).sqrt() } else { 0.0 };
            Rect::from_min_size(pos2(x, fg.center().y - clear - CHIP_ICON), vec2(CHIP_ICON, CHIP_ICON))
        };
        (Some(default), Rect::from_center_size(pos2(cx, row), vec2(20.0, 14.0)))
    } else {
        let icon = |x: f32| Rect::from_min_size(pos2(x, rect.top()), vec2(CHIP_ICON, CHIP_ICON));
        (Some(icon(left)), icon(left + group - CHIP_ICON))
    };
    let d = dr.map(|dr| ui.interact(dr, ui.scope_id().with("default-colors"), Sense::click()));
    let s = ui.interact(sr, ui.scope_id().with("swap-colors"), Sense::click());
    let p = ui.painter();
    for (r, resp) in dr.into_iter().zip(d.iter()).chain([(sr, &s)]) {
        if resp.hovered() && !studio {
            p.rect_filled(r.expand(2.0), radius, t.hover);
        }
    }
    let ink = |resp: &egui::Response| if resp.hovered() { t.text } else { t.icon };
    if studio {
        // Switch Colors: a shallow curve hugging the bottom chip, with a filled head at each end.
        // It glows on hover instead of getting a button box.
        let hot = s.hovered();
        let stroke = Stroke::new(1.5, ink(&s));
        let (c, rad) = (pos2(sr.center().x, sr.center().y - 5.0), 9.0);
        let (from, to) = (0.45, std::f32::consts::PI - 0.45);
        let arc: Vec<_> = (0..=12)
            .map(|i| {
                let a = from + (to - from) * i as f32 / 12.0;
                c + vec2(rad * a.cos(), rad * a.sin())
            })
            .collect();
        if hot {
            p.add(egui::Shape::line(arc.clone(), Stroke::new(5.0, t.text.gamma_multiply(0.18))));
        }
        let ends = [(arc[0], from, -1.0_f32), (arc[arc.len() - 1], to, 1.0)];
        p.add(egui::Shape::line(arc, stroke));
        // Each head points along the curve, away from its middle.
        for (end, a, sign) in ends {
            let dir = vec2(-a.sin(), a.cos()) * sign;
            let side = vec2(-dir.y, dir.x);
            let tip = end + dir * 2.5;
            let base = tip - dir * 5.0;
            p.add(egui::Shape::convex_polygon(vec![tip, base + side * 3.0, base - side * 3.0], ink(&s), Stroke::NONE));
        }
    } else {
        // Switch Colors: the compact clockwise bend has left- and down-pointing heads.
        // swap_arrow.rs owns its geometry and the pixel-level render regression test.
        crate::swap_arrow::paint(p, sr.min, ink(&s));
    }
    // Default Colors: a small black chip over a small white one.
    if let (Some(dr), Some(d)) = (dr, d.as_ref()) {
        let small = 7.0;
        let (b, w) = (Rect::from_min_size(dr.min + vec2(1.0, 1.0), vec2(small, small)), Rect::from_min_size(dr.min + vec2(5.0, 5.0), vec2(small, small)));
        p.rect_filled(w, 1.0, Color32::WHITE);
        p.rect_stroke(w, 1.0, Stroke::new(1.0, ink(d)), StrokeKind::Inside);
        p.rect_filled(b.expand(1.0), 1.5, t.chrome);
        p.rect_filled(b, 1.0, Color32::BLACK);
        p.rect_stroke(b, 1.0, Stroke::new(1.0, ink(d)), StrokeKind::Inside);
    }
    if let Some(d) = &d {
        d.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Default colours (D)")));
    }
    s.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, tl!("Swap colours (X)")));
    if s.on_hover_text(tl!("Swap colours (X)")).clicked() {
        chip_command(app, "tools.swapColors");
    }
    if d.is_some_and(|d| d.on_hover_text(tl!("Default colours (D)")).clicked()) {
        chip_command(app, "tools.defaultColors");
    }
}

/// Run Switch Colors or Default Colors from the chips. As in Photoshop, characters highlighted
/// in the type layer being edited take the new foreground colour, like picking one (#5690).
fn chip_command(app: &mut PhotocraftApp, id: &str) {
    if app.run(id, json!({})).is_ok() {
        crate::type_tool::foreground_changed(app);
    }
}
