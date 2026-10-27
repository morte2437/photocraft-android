use super::*;

// ----------------------------------------------------------------------------- properties

/// Floating Properties card anchored to the canvas' top-right corner. It has the docked group's
/// tabs: Properties (`properties_body`: document, layer, adjustment and mask properties) and
/// Adjustments (`adjustments_grid`).
pub fn properties_window(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.docking.is_some() {
        return;
    }
    // Pro (Photoshop) docks Properties; Studio floats it over the canvas.
    if !app.ui.panels.properties || Tokens::get(ctx).pro {
        return;
    }
    if app.session.active().is_none() {
        return;
    }
    let t = Tokens::get(ctx);
    let canvas = app.last_canvas_rect;
    let width = 320.0;
    let pos = pos2(canvas.right() - width - 12.0, canvas.top() + 44.0);
    let frame = egui::Frame::NONE
        .fill(t.card)
        .stroke(Stroke::new(1.0, t.card_border))
        .corner_radius(CornerRadius::same(t.radius_lg as u8))
        .shadow(egui::Shadow { offset: [0, 12], blur: 36, spread: 0, color: t.shadow })
        .inner_margin(egui::Margin::same(14));
    // A floating panel (Order::Middle like other panels, so menus, popups and dialogs stay above
    // it), anchored to the canvas corner and dragged by its title off the part being worked on.
    let card_id = egui::Id::unique("properties-card");
    let offset: egui::Vec2 = ctx.data(|m| m.get_temp(card_id)).unwrap_or_default();
    let mut drag = egui::Vec2::ZERO;
    let shown = egui::Area::new(card_id).order(egui::Order::Middle).fixed_pos(pos + offset).show(ctx, |ui| {
        frame.show(ui, |ui| {
            ui.set_width(width - 28.0);
            // Properties | Adjustments, as on the docked group (#7265).
            let group = crate::dock::Group::Properties;
            let visible = app.ui.dock.visible_tabs(group, false);
            let indices: Vec<usize> = visible.iter().map(|(i, _)| *i).collect();
            let tabs: Vec<&str> = visible.iter().map(|(_, name)| *name).collect();
            let mut sel = indices.iter().position(|i| *i == app.ui.dock_tabs.properties).unwrap_or(0);
            let (row, _) = ui.allocate_exact_size(vec2(ui.available_width(), 24.0), Sense::hover());
            let hide = Rect::from_min_max(pos2(row.right() - 22.0, row.top() + 1.0), pos2(row.right(), row.bottom() - 1.0));
            if icons::button(&mut ui.new_child(egui::UiBuilder::new().max_rect(hide)), "minus", 22.0, false, tl!("Hide Properties")).clicked() {
                app.ui.panels.properties = false;
            }
            // The title bar drags the card; registered before the tabs so they keep their clicks.
            let bar = Rect::from_min_max(row.min, pos2((hide.left() - 6.0).max(row.left()), row.bottom()));
            let grip = ui.interact(bar, card_id.with("title"), Sense::drag());
            if grip.hovered() || grip.dragged() {
                ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
            }
            drag = grip.drag_delta();
            let context = crate::tab_strip::pill_tabs(ui, card_id.with("tabs"), bar, &tabs, &mut sel);
            if let Some(&picked) = indices.get(sel) {
                app.ui.dock_tabs.properties = picked;
            }
            match context {
                Some(crate::tab_strip::TabContextAction::Close(i)) => {
                    if let Some(&tab) = indices.get(i) {
                        crate::dock::close_tab(app, group, tab, false);
                    }
                }
                Some(crate::tab_strip::TabContextAction::CloseGroup) => app.ui.panels.properties = false,
                None => {}
            }
            ui.add_space(6.0);
            widgets::hairline(ui);
            ui.add_space(8.0);
            match indices.get(sel) {
                Some(0) | None => properties_body(app, ui),
                Some(_) => adjustments_grid(app, ui),
            }
        });
    });
    if drag != egui::Vec2::ZERO && canvas.is_positive() {
        // Keep the title bar inside the canvas.
        let r = shown.response.rect.translate(drag);
        let dx = (canvas.left() - r.left()).max(0.0) - (r.right() - canvas.right()).max(0.0);
        let dy = (canvas.top() - r.top()).max(0.0) - (r.top() + 40.0 - canvas.bottom()).max(0.0);
        ctx.data_mut(|m| m.insert_temp(card_id, offset + drag + egui::vec2(dx, dy)));
    }
}

fn adjustment_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: LayerId, adj: &photocraft_doc::Adjustment) {
    // Keep the scope reachable before tall editors and within a compact dock (#5231).
    // Adjustment layers affect the stack by default; clipping limits them to their base.
    if let Some(mut clip) = app.session.active().and_then(|s| s.doc.layer(id).map(|l| l.clipped))
        && widgets::toggle(ui, &mut clip, tl!("Clip to layer below")).changed()
    {
        let _ = app.run("layer.setProps", json!({"layer": id.0, "clipped": clip}));
    }
    ui.add_space(6.0);
    crate::adjust_editors::layer_editor(app, ui, id, adj);
    ui.add_space(6.0);
    if let Some(mut vis) = app.session.active().and_then(|s| s.doc.layer(id).map(|l| l.visible))
        && widgets::toggle(ui, &mut vis, tl!("Adjustment visible")).changed()
    {
        let _ = app.run("layer.setProps", json!({"layer": id.0, "visible": vis}));
    }
    ui.add_space(6.0);
    if widgets::secondary_button(ui, tl!("Reset to defaults"), ui.available_width()).clicked() {
        let doc = app.session.active().map_or(0, |s| s.doc.id.0);
        // Photoshop's two-step Reset where measured (Color Balance), else the defaults.
        let params = crate::adjust_editors::properties_reset(ui.ctx(), doc, id, adj).unwrap_or_else(|| json!({"layer": id.0}));
        let _ = app.run("layer.setAdjustment", params);
        app.live_adjust = None;
    }
}

fn layer_controls(app: &mut PhotocraftApp, ui: &mut egui::Ui, layer: &Layer) {
    let t = Tokens::get(ui.ctx());
    if let Some(s) = layer.surface() {
        let b = app.layer_frame_bounds(layer, s);
        egui::Grid::new("props-grid").num_columns(2).spacing([12.0, 6.0]).show(ui, |ui| {
            for (k, v) in [("X", b.x0.to_string()), ("Y", b.y0.to_string()), ("W", b.width().to_string()), ("H", b.height().to_string())] {
                ui.label(RichText::new(k).color(t.text_dim));
                ui.label(RichText::new(format!("{v} px")).font(theme::mono(12.0)).color(t.text));
                ui.end_row();
            }
        });
        ui.add_space(6.0);
    }
    let mut o = layer.opacity * 100.0;
    let r = widgets::slider_row(ui, tl!("Opacity"), &mut o, 0.0..=100.0, "%", None);
    if r.drag_stopped() || (r.changed() && !r.dragged()) {
        let _ = app.run("layer.setProps", json!({"layer": layer.id.0, "opacity": o / 100.0}));
    }
    let mut f = layer.fill_opacity * 100.0;
    let r = widgets::slider_row(ui, crate::i18n::tr_ctx(crate::i18n::current(), "layers", "Fill"), &mut f, 0.0..=100.0, "%", None);
    if r.drag_stopped() || (r.changed() && !r.dragged()) {
        let _ = app.run("layer.setProps", json!({"layer": layer.id.0, "fill": f / 100.0}));
    }
}

/// Docked Properties body (Pro theme).
pub(super) fn properties_body(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let identity = app.session.active().map(|st| (st.doc.id, st.active_layer, app.ui.mask_target, app.ui.vector_mask_target, app.ui.properties_layer));
    ui.push_id(("properties-target", identity), |ui| properties_body_inner(app, ui));
}

fn properties_body_inner(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No properties"));
        return;
    };
    let doc = st.doc.clone();
    if let Some(layer) = st.active_layer.and_then(|id| doc.layer(id))
        && crate::mask_props_ui::targeted(app, layer)
    {
        crate::props_layout::header(ui, layer);
        crate::mask_props_ui::properties(app, ui, layer);
        return;
    }
    // The vector mask's own controls, when its thumbnail is targeted.
    if let Some(layer) = st.active_layer.and_then(|id| doc.layer(id))
        && crate::mask_props_ui::vector_targeted(app, layer)
    {
        crate::props_layout::header(ui, layer);
        crate::mask_props_ui::vector_properties(app, ui, layer);
        return;
    }
    // Photoshop shows the Document properties when nothing or the Background layer is selected,
    // with Align and Distribute above them when the Background is one of several (#4608).
    if crate::doc_props_ui::shows_document(&st.doc, st.active_layer) {
        if t.pro && st.selected_layers().len() > 1 {
            crate::layer_props_ui::selection_align(app, ui);
        }
        crate::doc_props_ui::properties(app, ui);
        return;
    }
    let Some(id) = st.active_layer else { return };
    // Borrowed from the document snapshot: cloning the layer every frame copied whole groups.
    let Some(layer) = doc.layer(id) else { return };
    // Header: kind icon, layer name and kind (#155); sections below draw their own separators.
    crate::props_layout::header(ui, layer);
    let is_adjustment = matches!(layer.content, LayerContent::Adjustment(_));
    if is_adjustment || layer.artboard().is_some() || !t.pro {
        ui.add_space(4.0);
        widgets::hairline(ui);
        ui.add_space(6.0);
    }
    if let LayerContent::Adjustment(adj) = &layer.content {
        adjustment_controls(app, ui, id, adj);
    } else if layer.artboard().is_some() {
        crate::artboard_ui::properties(app, ui, layer);
    } else if t.pro {
        // Transform, Align, the kind's sections and Quick Actions.
        crate::layer_props_ui::properties(app, ui, layer);
    } else {
        if matches!(layer.content, LayerContent::Fill(photocraft_doc::Fill::Gradient { .. })) {
            crate::gradient_ui::properties(app, ui, layer);
            ui.add_space(6.0);
        }
        layer_controls(app, ui, layer);
        if matches!(layer.content, LayerContent::Text(_)) {
            crate::type_tool::type_properties(app, ui);
        }
        if matches!(layer.content, LayerContent::Shape(_)) {
            crate::vector_ui::shape_properties(app, ui, id);
            crate::mask_props_ui::shape_properties(app, ui, layer);
        }
    }
}

/// Photoshop's Adjustments panel: a grid of one-click adjustment layers.
pub(super) fn adjustments_grid(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!("Add an adjustment")).color(t.text_dim));
    ui.add_space(4.0);
    let items: [(&str, &str, &str); 16] = [
        ("brightnessContrast", "sun", tl!("Brightness/Contrast")),
        ("levels", "gauge", tl!("Levels")),
        ("curves", "pen-tool", tl!("Curves")),
        ("exposure", "scan", tl!("Exposure")),
        ("vibrance", "sparkles", tl!("Vibrance")),
        ("hueSaturation", "droplet", tl!("Hue/Saturation")),
        ("colorBalance", "blend", tl!("Color Balance")),
        ("blackWhite", "contrast", tl!("Black & White")),
        ("photoFilter", "circle-dot", tl!("Photo Filter")),
        ("channelMixer", "sliders-horizontal", tl!("Channel Mixer")),
        ("invert", "squares-subtract", tl!("Invert")),
        ("posterize", "layers", tl!("Posterize")),
        ("threshold", "square", tl!("Threshold")),
        ("gradientMap", "palette", tl!("Gradient Map")),
        ("selectiveColor", "swatch-book", tl!("Selective Color")),
        ("colorLookup", "grid-3x3", tl!("Color Lookup")),
    ];
    let mut run = None;
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        for (kind, icon, tip) in items {
            if icons::button(ui, icon, 30.0, false, tip).clicked() {
                run = Some(format!("layer.newAdjustmentLayer.{kind}"));
            }
        }
    });
    if let Some(id) = run {
        let _ = app.run(&id, json!({}));
        app.ui.dock_tabs.properties = 0;
    }
}

/// The options bar's Select and Mask… button: greyed out, as the menu item is, until there is a
/// selection (or a layer mask) to refine (#7520).
pub(super) fn select_and_mask_button(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let enabled = crate::menus::is_enabled(app, "select.selectAndMask");
    let clicked = ui.add_enabled_ui(enabled, |ui| widgets::secondary_button(ui, tl!("Select and Mask…"), 0.0).clicked()).inner;
    if clicked {
        let _ = crate::menus::invoke(app, ui.ctx(), "select.selectAndMask", json!({}));
    }
}

/// New / add / subtract / intersect selection buttons (shared by selection tools).
pub(super) fn selection_mode_buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
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
}
