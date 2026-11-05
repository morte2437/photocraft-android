use super::layers::footer_label;
use super::{Layer, LayerId, MaskKind, PhotocraftApp, Pos2, Rect, Sense, Stroke, StrokeKind, Tokens, Value, icons, json, pos2, vec2};

/// Scroll the Layers panel while holding a layer drag over its top/bottom edge or past them.
///
/// Returns the *content* displacement in points for this frame, so positive moves the
/// list downward (reveals rows above) and negative upward (reveals rows below).
/// The speed ramps with proximity to the edge and clamps to the maximum speed when
/// dragged outside, using elapsed time instead of assuming a particular refresh rate.
pub(crate) fn layer_drag_edge_scroll(pointer: Option<Pos2>, viewport: Rect, dragging: bool, dt: f32) -> f32 {
    if !dragging || viewport.width() <= 0.0 || viewport.height() <= 0.0 || dt.is_nan() || dt <= 0.0 {
        return 0.0;
    }
    let Some(pointer) = pointer.filter(|p| p.x.is_finite() && p.y.is_finite() && p.x >= viewport.left() && p.x <= viewport.right()) else {
        return 0.0;
    };
    let edge = 32.0_f32.min(viewport.height() * 0.25);
    let top = ((edge - (pointer.y - viewport.top())) / edge).clamp(0.0, 1.0);
    let bottom = ((edge - (viewport.bottom() - pointer.y)) / edge).clamp(0.0, 1.0);
    let direction = top - bottom;
    if direction == 0.0 {
        return 0.0;
    }
    let velocity = 80.0 + 520.0 * direction.abs();
    direction.signum() * velocity * dt.clamp(0.0, 0.05)
}

/// A layer row dragged onto a Layers panel footer button, as in Photoshop: onto Delete deletes it,
/// onto New Layer duplicates it, onto New Group groups it. A row that is part of the selection
/// carries the whole selection; any other row goes alone. Highlights the button while over it and
/// returns the command on release.
pub(super) fn footer_drop(ui: &egui::Ui, button: &egui::Response, drag: Option<(u64, bool)>, command: &str) -> Option<(String, Value)> {
    // Named for screen readers (and tests) after the command a drop runs.
    button.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, footer_label(command)));
    let (layer, in_selection) = drag?;
    dropped_on(ui, button).then(|| (command.into(), if in_selection { json!({}) } else { json!({"layer": layer}) }))
}

/// While something is dragged over a footer `button`: highlights it, and is true on the release.
pub(crate) fn dropped_on(ui: &egui::Ui, button: &egui::Response) -> bool {
    let Some(p) = ui.ctx().input(|i| i.pointer.interact_pos()) else { return false };
    if !button.rect.contains(p) {
        return false;
    }
    let t = Tokens::get(ui.ctx());
    ui.painter().rect_stroke(button.rect, t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    ui.ctx().input(|i| i.pointer.any_released())
}

/// Effects dragged onto the footer's Delete button, as in Photoshop (#3102): the "Effects" row
/// clears the layer style, an effect's row deletes just that effect.
pub(super) fn fx_trash_drop(ui: &egui::Ui, button: &egui::Response, drag: Option<FxDrag>) -> Option<(String, Value)> {
    let (layer, effect) = drag?;
    if !dropped_on(ui, button) {
        return None;
    }
    let params = match effect {
        Some(i) => json!({"layer": layer, "effect": i}),
        None => json!({"layer": layer}),
    };
    Some(("layer.layerStyle.clear".into(), params))
}

/// A drag from within a multi-selection moves the whole selection, not only the grabbed
/// row. Dragging an unselected row keeps the existing single-layer behavior.
pub(super) fn layer_drop_payload(dragged: u64, target: LayerId, position: &str, selected: &[LayerId]) -> Value {
    if selected.len() > 1 && selected.contains(&LayerId(dragged)) {
        json!({"layers": selected.iter().map(|id| id.0).collect::<Vec<_>>(), "target": target.0, "position": position})
    } else {
        json!({"layer": dragged, "target": target.0, "position": position})
    }
}

/// Would this drop move the Background off the bottom, or put a layer below it? Photoshop
/// refuses both (no drop indicator); ⌥-dragging the Background still copies it above.
pub(super) fn background_drop_refused(doc: &photocraft_doc::Document, payload: &Value, target: LayerId, position: &str, copy: bool) -> bool {
    let Some(bg) = doc.layers.first().filter(|b| crate::doc_props_ui::is_background(doc, b)).map(|b| b.id) else { return false };
    let moves_bg =
        !copy && (payload["layer"].as_u64() == Some(bg.0) || payload["layers"].as_array().is_some_and(|a| a.iter().any(|v| v.as_u64() == Some(bg.0))));
    moves_bg || (target == bg && position == "below")
}

/// Drag a layer row to reorder: drop above, below, or inside an existing group. With ⌥ held on
/// release the layers stay put and copies land there instead (Photoshop).
/// Multi-layer moves are atomic (one undo step), using the engine's stable document order.
pub(super) fn layer_drag_and_drop(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    ui: &egui::Ui,
    l: &Layer,
    rect: Rect,
    resp: &egui::Response,
    actions: &mut Vec<(String, Value)>,
) {
    let t = Tokens::get(ctx);
    let key = egui::Id::unique("layer-drag");
    // A drag from the mask thumbnail (`starts_mask_drag`) moves the mask, not the layer.
    let mask_drag = ctx.data(|d| d.get_temp::<MaskDrag>(mask_drag_key())).is_some_and(|(id, _)| id == l.id.0);
    if resp.drag_started() && !mask_drag {
        ctx.data_mut(|d| d.insert_temp(key, l.id.0));
    }
    let Some(dragged) = ctx.data(|d| d.get_temp::<u64>(key)) else { return };
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    let released = ctx.input(|i| i.pointer.any_released());
    let copy = ctx.input(|i| i.modifiers.alt);
    if dragged == l.id.0 {
        // Ghost label following the pointer, and the copy cursor while ⌥ is held.
        if let Some(p) = pointer {
            crate::layer_transfer::ghost(ctx, p, &l.name);
        }
        if copy {
            ctx.set_cursor_icon(egui::CursorIcon::Copy);
        }
        // #1721: with ⌥ held the row also takes the drop onto itself — the only drop target a
        // single-layer document has. The copy lands above or below it (never into a group's own
        // centre). A multi-selection is left alone: the engine copies it beside each of its own
        // layers, not onto one of them.
        let selected = app.session.active().map(|st| st.selected_layers()).unwrap_or_default();
        let in_multi = selected.len() > 1 && selected.contains(&l.id);
        if copy
            && !in_multi
            && let Some(p) = pointer.filter(|p| rect.contains(*p))
        {
            let position = if (p.y - rect.top()) / rect.height() < 0.5 { "above" } else { "below" };
            let painter = ui.painter();
            if position == "above" {
                painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(2.0, t.accent));
            } else {
                painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.0, t.accent));
            }
            let mut payload = layer_drop_payload(dragged, l.id, position, &selected);
            if let Some(o) = payload.as_object_mut() {
                o.insert("copy".into(), json!(true));
            }
            if released {
                app.reorder_preview = None;
                actions.push(("layer.moveTo".into(), payload));
            } else if ctx.input(|i| i.pointer.primary_down()) {
                crate::layers_panel_ui::preview_reorder(app, &payload);
            }
        }
        return;
    }
    let Some(p) = pointer else { return };
    if !rect.contains(p) {
        return;
    }
    let f = (p.y - rect.top()) / rect.height();
    let position = if l.is_group() && (0.3..0.7).contains(&f) {
        "into"
    } else if f < 0.5 {
        "above"
    } else {
        "below"
    };
    let Some(st) = app.session.active() else { return };
    if position == "into" && l.frame.is_some() {
        let selected = st.selected_layers();
        if selected.len() > 1 && selected.iter().any(|id| id.0 == dragged) {
            return;
        }
        let source = LayerId(dragged);
        if st.doc.layer(source).is_none_or(|l| !matches!(l.content, photocraft_doc::LayerContent::Raster(_) | photocraft_doc::LayerContent::Smart(_))) {
            return;
        }
        ui.painter().rect_stroke(rect.shrink(1.0), t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
        if released {
            app.reorder_preview = None;
            actions.push(("layer.frame.setContent".into(), json!({"layer":l.id.0,"content":dragged,"copy":copy})));
        }
        return;
    }
    let mut payload = layer_drop_payload(dragged, l.id, position, &st.selected_layers());
    if background_drop_refused(&st.doc, &payload, l.id, position, copy) {
        return;
    }
    let painter = ui.painter();
    match position {
        "into" => {
            painter.rect_stroke(rect.shrink(1.0), t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
        }
        "above" => {
            painter.line_segment([rect.left_top(), rect.right_top()], Stroke::new(2.0, t.accent));
        }
        _ => {
            painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(2.0, t.accent));
        }
    }
    if copy && let Some(o) = payload.as_object_mut() {
        o.insert("copy".into(), json!(true));
    }
    if !released && ctx.input(|i| i.pointer.primary_down()) {
        crate::layers_panel_ui::preview_reorder(app, &payload);
    }
    if released {
        app.reorder_preview = None;
        actions.push(("layer.moveTo".into(), payload));
    }
}

/// #1721: the drop zones no row owns — the space between two rows and the empty space below the
/// bottom-most one. The drop lands on `target` at `position` (`above` or `below` its row): a copy
/// with ⌥ held on release (Photoshop), a move without. A multi-selection dropped on the row it
/// was grabbed from is left alone: the engine can only copy it beside each of its own layers.
pub(super) fn layer_drop_zone(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    ui: &egui::Ui,
    zone: Rect,
    target: (LayerId, Rect),
    position: &'static str,
    actions: &mut Vec<(String, Value)>,
) {
    let Some(dragged) = ctx.data(|d| d.get_temp::<u64>(egui::Id::unique("layer-drag"))) else { return };
    let selected = app.session.active().map(|st| st.selected_layers()).unwrap_or_default();
    if target.0 == LayerId(dragged) && selected.len() > 1 && selected.contains(&target.0) {
        return;
    }
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    if !zone.contains(p) {
        return;
    }
    let t = Tokens::get(ctx);
    let line = if position == "above" { [target.1.left_top(), target.1.right_top()] } else { [target.1.left_bottom(), target.1.right_bottom()] };
    ui.painter().line_segment(line, Stroke::new(2.0, t.accent));
    let mut payload = layer_drop_payload(dragged, target.0, position, &selected);
    if ctx.input(|i| i.modifiers.alt)
        && let Some(o) = payload.as_object_mut()
    {
        o.insert("copy".into(), json!(true));
    }
    if ctx.input(|i| i.pointer.any_released()) {
        app.reorder_preview = None;
        actions.push(("layer.moveTo".into(), payload));
    } else if ctx.input(|i| i.pointer.primary_down()) {
        crate::layers_panel_ui::preview_reorder(app, &payload);
    }
}

/// An fx row being dragged: the layer it belongs to and the effect index (`None` = the "Effects"
/// row, all of them).
pub(super) type FxDrag = (u64, Option<usize>);

/// A mask thumbnail being dragged: the layer and which of its masks (#3113, #3119).
pub(super) type MaskDrag = (u64, MaskKind);

pub(super) fn mask_drag_key() -> egui::Id {
    egui::Id::unique("mask-drag")
}

/// Which mask's thumbnail (where it was drawn last frame) the row's drag starts on, if any.
pub(super) fn starts_mask_drag(ctx: &egui::Context, l: &Layer, resp: &egui::Response) -> Option<MaskKind> {
    if !resp.drag_started() {
        return None;
    }
    let p = ctx.input(|i| i.pointer.press_origin())?;
    let (thumbs, _) = crate::mask_thumbs_ui::recorded(ctx, l.id.0)?;
    let kind = thumbs.iter().find(|(_, r)| r.contains(p))?.0;
    let present = match kind {
        MaskKind::Pixel => l.mask.is_some(),
        MaskKind::Vector => l.vector_mask.is_some(),
    };
    present.then_some(kind)
}

pub(super) fn fx_drag_key() -> egui::Id {
    egui::Id::unique("fx-drag")
}

/// Start dragging the effects of `layer`: one (`effect`) or all of them.
pub(super) fn start_fx_drag(ctx: &egui::Context, layer: LayerId, effect: Option<usize>) {
    ctx.data_mut(|d| d.insert_temp::<FxDrag>(fx_drag_key(), (layer.0, effect)));
}

/// While effects are dragged: their name by the pointer, and the copy cursor while ⌥ is held.
pub(super) fn fx_drag_feedback(ctx: &egui::Context, doc: &photocraft_doc::Document) {
    let Some((from, effect)) = ctx.data(|d| d.get_temp::<FxDrag>(fx_drag_key())) else { return };
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    // `Effect::label` is the English key; translate for the drag ghost like the Layers panel rows.
    let label = match effect {
        Some(i) => doc.layer(LayerId(from)).and_then(|l| l.effects.items.get(i)).map(|e| tl!(e.label())).unwrap_or_default(),
        None => tl!("Effects"),
    };
    crate::layer_transfer::ghost(ctx, p, label);
    if ctx.input(|i| i.modifiers.alt) {
        ctx.set_cursor_icon(egui::CursorIcon::Copy);
    }
}

/// The command for dropping effects of layer `from` on `target`: they move there, or with ⌥ held
/// on release are copied (Photoshop). `effect` is one effect row, or all effects when `None`.
fn fx_drop_action(from: u64, effect: Option<usize>, target: LayerId, copy: bool) -> (String, Value) {
    let mut payload = json!({"from": from, "to": target.0, "copy": copy});
    if let Some(o) = payload.as_object_mut()
        && let Some(i) = effect
    {
        o.insert("effect".into(), json!(i));
    }
    ("layer.layerStyle.transferEffects".into(), payload)
}

/// A layer row accepts the fx row being dragged: outlined while the pointer is over it, dropped
/// on release.
pub(super) fn fx_drop(ctx: &egui::Context, ui: &egui::Ui, l: &Layer, rect: Rect, actions: &mut Vec<(String, Value)>) {
    let Some((from, effect)) = ctx.data(|d| d.get_temp::<FxDrag>(fx_drag_key())) else { return };
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    if from == l.id.0 || !rect.contains(p) {
        return;
    }
    let t = Tokens::get(ctx);
    ui.painter().rect_stroke(rect.shrink(1.0), t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    if ctx.input(|i| i.pointer.any_released()) {
        actions.push(fx_drop_action(from, effect, l.id, ctx.input(|i| i.modifiers.alt)));
    }
}

/// Photoshop shows a layer's effects as indented sub-rows ("Effects", then each effect). The eye
/// on "Effects" shows or hides them all, the eye on an effect's row just that one (#1622).
pub(super) fn effect_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, l: &Layer, depth: usize, actions: &mut Vec<(String, Value)>) {
    let t = Tokens::get(ui.ctx());
    let indent = 30.0 + depth as f32 * 14.0 + 34.0;
    // English `Effect::label` keys stay for kind matching; display goes through `tl!` (catalogs
    // already have Drop Shadow → 投影, Effects → 效果, …).
    let mut rows: Vec<(String, bool, Option<&'static str>)> = vec![(tl!("Effects").into(), l.effects.enabled, None)];
    for e in &l.effects.items {
        let kind = crate::layer_style::KINDS.iter().find(|k| k.1 == e.label()).map(|k| k.0);
        rows.push((tl!(e.label()).into(), e.enabled(), kind));
    }
    for (i, (name, on, kind)) in rows.into_iter().enumerate() {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 22.0), Sense::click_and_drag());
        // Drag the row onto another layer to move its effects there, ⌥-drag to copy them.
        if resp.drag_started() {
            start_fx_drag(ui.ctx(), l.id, i.checked_sub(1));
        }
        if !ui.is_rect_visible(rect) {
            continue;
        }
        if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.35));
        }
        if t.pro {
            ui.painter().line_segment([pos2(rect.left() + 30.0, rect.top()), pos2(rect.left() + 30.0, rect.bottom())], Stroke::new(1.0, t.separator));
        }
        // Nested under the layer's eye column so an effect does not read as its own layer (#4572).
        let eye = Rect::from_min_size(pos2(rect.left() + 36.0, rect.center().y - 9.0), vec2(18.0, 18.0));
        // A hidden effect's eye box is left empty (still clickable), like a hidden layer's.
        if on {
            icons::paint(ui, eye, "eye", 12.0, t.icon);
        }
        // The whole eye column of the row takes the click, so it never opens the Layer Style dialog.
        let eye_cell = Rect::from_min_max(pos2(rect.left() + 30.0, rect.top()), pos2((rect.left() + 58.0).min(rect.right()), rect.bottom()));
        if ui.interact(eye_cell, ui.scope_id().with(("fx-eye", l.id.0, i)), Sense::click()).clicked() {
            let mut params = json!({"layer": l.id.0, "visible": !on});
            if let (Some(index), Some(o)) = (i.checked_sub(1), params.as_object_mut()) {
                o.insert("index".into(), json!(index));
            }
            actions.push(("layer.setEffectsVisible".into(), params));
        }
        let x = rect.left() + indent + if i == 0 { 8.0 } else { 24.0 };
        if i == 0 {
            icons::paint(ui, Rect::from_center_size(pos2(x - 12.0, rect.center().y), vec2(14.0, 14.0)), "sparkles", 11.0, t.text_dim);
        }
        let right = rect.right() - crate::layer_row_ui::RIGHT_PAD;
        crate::layer_row_ui::label(
            ui.painter(),
            x,
            rect.center().y,
            right,
            &name,
            egui::FontId::proportional(11.5),
            if on { t.text_dim } else { t.text_faint },
        );
        resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, name.clone()));
        if resp.double_clicked() {
            crate::layer_style::open_for_layer(app, l.id, kind);
        }
        // #2846: right-click opens the layer style menu (Create Layer, Copy Layer Style…), as in Photoshop.
        resp.context_menu(|ui| {
            crate::widgets::style_spectrum_popup_menu(ui);
            crate::layer_menu_ui::show_effects(app, ui, l, actions);
        });
    }
}
