//! Layers-panel pieces for smart objects: the thumbnail badge and the Smart Filters sub-rows
//! (eye toggles; double-click a filter to re-open its dialog with the recorded parameters; click
//! to select one; drag onto another smart object to move it there, ⌥-drag to copy it, #4271;
//! drag it up or down its own stack to reorder it).
//! The Smart Filters row carries the filter mask thumbnail: click it to paint the filter mask,
//! ⌥-click to view it, ⇧-click to disable it (FILE-215-3).

use egui::{Color32, Rect, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::{Document, Layer, LayerContent, LayerId, SmartObject};
use serde_json::{Value, json};

use crate::state::LayerThumbnailSize;
use crate::theme::Tokens;
use crate::{PhotocraftApp, icons};

/// What the badge on a layer thumbnail marks the layer as.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ThumbBadge {
    Smart,
    Shape,
}

/// The badge `l`'s thumbnail carries. Smart objects and shape layers (#4584) show their pixels
/// like any other layer, so only the badge tells them apart.
fn thumb_badge_kind(l: &Layer) -> Option<ThumbBadge> {
    match l.content {
        LayerContent::Smart(_) => Some(ThumbBadge::Smart),
        LayerContent::Shape(_) => Some(ThumbBadge::Shape),
        _ => None,
    }
}

/// Where the badge sits: over the bottom-right corner of the thumbnail.
fn thumb_badge_rect(thumb: Rect) -> Rect {
    Rect::from_min_size(pos2(thumb.right() - 9.0, thumb.bottom() - 9.0), vec2(11.0, 11.0))
}

/// The shape badge's path, inset from the badge so its corner anchors stay inside the frame.
const SHAPE_PATH_INSET: f32 = 3.0;
/// Side of an anchor point on the shape badge's path.
const SHAPE_ANCHOR: f32 = 3.0;

/// Smart-object or shape-layer badge in the bottom-right corner of a layer thumbnail.
pub fn thumb_badge(ui: &egui::Ui, l: &Layer, thumb: Rect) {
    let Some(kind) = thumb_badge_kind(l) else { return };
    let t = Tokens::get(ui.ctx());
    let r = thumb_badge_rect(thumb);
    ui.painter().rect_filled(r, 2.0, t.field);
    ui.painter().rect_stroke(r, 2.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
    match kind {
        ThumbBadge::Smart => icons::paint(ui, r, "app-window", 9.0, t.icon),
        // Photoshop's shape mark: a path with an anchor point on each corner. Painted rather
        // than an icon, so the anchors stay solid squares at this size.
        ThumbBadge::Shape => {
            let path = r.shrink(SHAPE_PATH_INSET);
            ui.painter().rect_stroke(path, 0.0, Stroke::new(1.0, t.icon), egui::StrokeKind::Middle);
            for anchor in [path.left_top(), path.right_top(), path.right_bottom(), path.left_bottom()] {
                ui.painter().rect_filled(Rect::from_center_size(anchor, vec2(SHAPE_ANCHOR, SHAPE_ANCHOR)), 0.0, t.icon);
            }
        }
    }
}

/// The display name of a smart filter (its command's label without the ellipsis). A Photoshop
/// filter PhotoCraft doesn't implement shows its own name and is marked as kept as is.
pub(crate) fn filter_label(f: &photocraft_doc::SmartFilter) -> String {
    let command = f.command.as_str();
    if command == photocraft_engine::smart_cmds::UNSUPPORTED_FILTER {
        let name = f.params.get("name").and_then(Value::as_str).unwrap_or("Smart filter");
        return format!("{} (kept, not editable)", name.trim_end_matches("...").trim_end_matches('…'));
    }
    photocraft_engine::commands::find(command)
        .map_or_else(|| command.rsplit('.').next().unwrap_or(command).to_string(), |c| tl!(c.label).trim_end_matches('…').to_string())
}

/// Height of a Smart Filter or effect sub-row. As in Photoshop, these stay compact at every
/// Layers panel thumbnail size; only the layer row itself grows with its thumbnails.
pub(crate) const SUB_ROW_H: f32 = 22.0;

/// The Smart Filters header row: (row height, shared-mask thumbnail side). The mask follows the
/// Layers panel thumbnail size (hidden at None) and the row grows just enough to hold it.
pub(crate) fn smart_filters_header_geometry(size: LayerThumbnailSize, pro: bool) -> (f32, f32) {
    let (thumb, _) = crate::layers_panel_ui::thumbnail_geometry(size, pro);
    if thumb == 0.0 { (SUB_ROW_H, 0.0) } else { ((thumb + 6.0).max(SUB_ROW_H), thumb) }
}

/// Smart Filters header + one row per filter (top filter first, as in Photoshop) under a smart
/// layer. Edits are queued in `actions` as engine commands.
pub fn has_filters(l: &Layer) -> bool {
    matches!(&l.content, LayerContent::Smart(sm) if !sm.smart_filters.is_empty())
}

/// The two rings in the layer row indicate an attached Smart Filters stack.
pub fn paint_badge(painter: &egui::Painter, center: egui::Pos2, color: Color32) {
    for dx in [-2.5, 2.5] {
        painter.circle_stroke(center + vec2(dx, 0.0), 3.5, Stroke::new(1.0, color));
    }
}

/// Select smart object `layer` now, ahead of its filter editor. The editor captures the
/// document's revision as its target, so the click's queued `layer.select` must not run after it
/// opens (OK would refuse the edit): select first, as the Layers panel runs it. False when the
/// selection failed and the editor must stay closed.
fn select_before_editor(app: &mut PhotocraftApp, layer: u64, actions: &mut Vec<(String, Value)>) -> bool {
    actions.retain(|(id, p)| !(id == "layer.select" && p.get("layer").and_then(Value::as_u64) == Some(layer)));
    if app.ui.text_edit.is_some() {
        crate::type_tool::commit(app);
    }
    app.run("layer.select", json!({"layer": layer})).is_ok()
}

/// Paint the Smart Filters header and one row per filter of Smart Object `l` (top filter first), with their drag, drop and click handling; actions go to `actions`.
pub fn filter_rows(app: &mut PhotocraftApp, ui: &mut egui::Ui, doc: &Document, l: &Layer, depth: usize, actions: &mut Vec<(String, Value)>) {
    let LayerContent::Smart(sm) = &l.content else { return };
    if sm.smart_filters.is_empty() {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let (header_height, mask_size) = smart_filters_header_geometry(app.ui.layers_panel_options.thumbnail_size, t.pro);
    let indent = 30.0 + crate::layer_row_ui::indent(depth, ui.available_width(), 130.0);
    let Some(st) = app.session.active() else { return };
    let active = st.active_layer == Some(l.id);
    let picked = ui.ctx().data(|d| d.get_temp::<FilterDrag>(selected_key(doc.id)));
    let rows = std::iter::once(None).chain((0..sm.smart_filters.len()).rev().map(Some));
    for index in rows {
        let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), if index.is_none() { header_height } else { SUB_ROW_H }), Sense::click_and_drag());
        if let Some((to, action)) = reorder_drop(ui, l, sm.smart_filters.len(), index, rect) {
            // The moved filter ends up selected, with its smart object active, as after a click.
            ui.ctx().data_mut(|d| d.insert_temp::<FilterDrag>(selected_key(doc.id), (l.id.0, Some(to))));
            actions.push(action);
            if !active {
                actions.push(("layer.select".into(), json!({"layer": l.id.0})));
            }
        }
        // Selected as in Photoshop: the row stays highlighted while its smart object is active.
        let selected = active && picked == Some((l.id.0, index));
        if selected {
            ui.painter().rect_filled(rect, 0.0, if t.pro { t.row_selected } else { t.hover });
        } else if resp.contains_pointer() {
            ui.painter().rect_filled(rect, 0.0, t.hover.gamma_multiply(0.35));
        }
        if t.pro {
            ui.painter().line_segment([pos2(rect.left() + 30.0, rect.top()), pos2(rect.left() + 30.0, rect.bottom())], Stroke::new(1.0, t.separator));
        }
        let (name, on, shown) = filter_row_label(sm, index);
        let eye = Rect::from_center_size(pos2(rect.left() + indent + if index.is_none() { 9.0 } else { 41.0 }, rect.center().y), vec2(18.0, 18.0));
        let eye_resp = ui.interact(eye, ui.scope_id().with(("sf-eye", l.id.0, index)), Sense::click());
        if shown {
            icons::paint(ui, eye, "eye", 12.0, if on { t.icon } else { t.text_faint });
        }
        eye_resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, true, shown, format!("{} {name}", tl!("Visibility"))));
        if eye_resp.clicked() {
            actions.push(match index {
                None => ("layer.smartFilter.disableSmartFilters".into(), json!({"layer": l.id.0})),
                Some(i) => ("layer.smartFilter.setVisible".into(), json!({"layer": l.id.0, "index": i})),
            });
        }
        // The shared mask scales with the thumbnail setting (hidden at None); the header name
        // follows it, while filter names stay aligned past their eyes.
        let mask_left = rect.left() + indent + 28.0;
        let x = filter_label_x(rect.left(), indent, mask_size, index);
        if index.is_none() && mask_size > 0.0 {
            let sw = Rect::from_min_size(pos2(mask_left, rect.center().y - mask_size / 2.0), vec2(mask_size, mask_size));
            if let Some(mask) = &sm.filter_mask {
                let tex = app.mask_thumb_kind(ui.ctx(), doc, l.id, mask, crate::mask_thumbs_ui::THUMB_SMART_MASK);
                ui.painter().image(tex, sw, Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)), Color32::WHITE);
                if !mask.enabled {
                    ui.painter().line_segment([sw.left_top(), sw.right_bottom()], Stroke::new(1.0, t.danger));
                    ui.painter().line_segment([sw.right_top(), sw.left_bottom()], Stroke::new(1.0, t.danger));
                }
            } else {
                ui.painter().rect_filled(sw, 0.0, Color32::WHITE);
            }
            ui.painter().rect_stroke(sw, 1.0, Stroke::new(1.0, t.field_border), egui::StrokeKind::Inside);
            if app.ui.filter_mask_target == Some(l.id.0) && app.filter_mask_targeted() {
                crate::mask_thumbs_ui::paint_brackets(ui.painter(), sw, t.text);
            }
            let sw_resp = ui.interact(sw.expand(2.0), ui.scope_id().with(("sf-mask", l.id.0)), Sense::click());
            if sw_resp.clicked() {
                actions.extend(filter_mask_click(l, sm, ui.input(|i| i.modifiers)));
            }
        }
        let blend = index.map(|_| Rect::from_center_size(pos2(rect.right() - crate::layer_row_ui::RIGHT_PAD - 9.0, rect.center().y), vec2(18.0, 18.0)));
        let right = blend.map_or(rect.right() - crate::layer_row_ui::RIGHT_PAD, |r| r.left() - 4.0);
        crate::layer_row_ui::label(
            ui.painter(),
            x,
            rect.center().y,
            right,
            &name,
            egui::FontId::proportional(11.5),
            if on { t.text_dim } else { t.text_faint },
        );
        // A filter's name is its own target, apart from the Blending Options icon (#3283): it
        // selects, drags (#4271) and opens the filter's editor on a double-click. The row around
        // it does the same, but only the name carries the row's label and selected state.
        let name_resp = index.map(|i| {
            let name_rect = Rect::from_min_max(pos2(x.min(right), rect.top()), pos2(right, rect.bottom()));
            ui.interact(name_rect, ui.scope_id().with(("sf-name", l.id.0, i)), Sense::click_and_drag())
        });
        match &name_resp {
            Some(r) => r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name)),
            None => resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name)),
        }
        let hit = |f: fn(&egui::Response) -> bool| f(&resp) || name_resp.as_ref().is_some_and(f);
        if hit(egui::Response::drag_started) {
            ui.ctx().data_mut(|d| d.insert_temp::<FilterDrag>(drag_key(), (l.id.0, index)));
        }
        if hit(egui::Response::clicked) {
            ui.ctx().data_mut(|d| d.insert_temp::<FilterDrag>(selected_key(doc.id), (l.id.0, index)));
            if !active {
                actions.push(("layer.select".into(), json!({"layer": l.id.0})));
            }
        }
        if let Some(i) = index
            && hit(egui::Response::double_clicked)
            && (active || select_before_editor(app, l.id.0, actions))
        {
            open_editor(app, ui.ctx(), l.id.0, sm, i);
        }
        if let (Some(i), Some(r)) = (index, blend) {
            let blend_resp = ui.interact(r, ui.scope_id().with(("sf-blend", l.id.0, i)), Sense::click());
            icons::paint(ui, r, "sliders-horizontal", 13.0, if blend_resp.hovered() { t.text } else { t.icon });
            let label = crate::i18n::fmt(tl!("Blending Options ({filter})"), &[("filter", &name)]);
            blend_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &label));
            if blend_resp.double_clicked()
                && let Err(e) = crate::smart_filter_dialog::open(app, l.id, i)
            {
                crate::notices::error(app, e);
            }
            blend_resp.on_hover_text(label);
        }
    }
    // Every row of `l` is drawn, so a filter of it selected (now, after this pass's clicks) is on
    // screen: see [`forget_hidden`].
    if ui.ctx().data(|d| d.get_temp::<FilterDrag>(selected_key(doc.id))).is_some_and(|(layer, _)| layer == l.id.0) {
        let pass = ui.ctx().cumulative_pass_nr();
        ui.ctx().data_mut(|d| d.insert_temp(drawn_key(doc.id), pass));
    }
}

fn filter_row_label(sm: &SmartObject, index: Option<usize>) -> (String, bool, bool) {
    match index {
        None => (tl!("Smart Filters").to_string(), sm.filters_enabled, sm.filters_enabled),
        Some(i) => {
            let filter = &sm.smart_filters[i];
            (filter_label(filter), filter.visible && sm.filters_enabled, filter.visible)
        }
    }
}

fn filter_label_x(left: f32, indent: f32, mask_size: f32, index: Option<usize>) -> f32 {
    match index {
        None if mask_size > 0.0 => left + indent + 28.0 + mask_size + 8.0,
        None => left + indent + 28.0,
        Some(_) => left + indent + 60.0,
    }
}

/// What a click on the filter mask thumbnail does: target it for painting (selecting the smart
/// object), ⌥ view it alone (⇧⌥ as an overlay), ⇧ disable or enable it.
fn filter_mask_click(l: &Layer, sm: &SmartObject, m: egui::Modifiers) -> Vec<(String, Value)> {
    let layer = l.id.0;
    if m.alt {
        let mode = if m.shift { "toggleOverlay" } else { "toggleGray" };
        return sm
            .filter_mask
            .is_some()
            .then(|| (photocraft_engine::mask_view_cmds::ID.to_string(), json!({"layer": layer, "mode": mode, "mask": "filter"})))
            .into_iter()
            .collect();
    }
    if m.shift {
        return sm
            .filter_mask
            .as_ref()
            .map(|f| ("layer.smartFilter.disableFilterMask".to_string(), json!({"layer": layer, "enabled": !f.enabled})))
            .into_iter()
            .collect();
    }
    vec![("layer.select".into(), json!({"layer": layer})), ("ui.filterMaskTarget".into(), json!(layer))]
}

/// Smart filter rows being dragged: the smart object and the filter index (`None` = the Smart
/// Filters row, all of them). Also what a click selects.
pub(crate) type FilterDrag = (u64, Option<usize>);

/// Memory key of the smart filter row being dragged.
pub(crate) fn drag_key() -> egui::Id {
    egui::Id::unique("smart-filter-drag")
}

/// The selected smart filter row of a document.
fn selected_key(doc: photocraft_doc::DocId) -> egui::Id {
    egui::Id::unique(("smart-filter-selected", doc))
}

/// The pass in which the selected smart filter row of a document was last drawn.
fn drawn_key(doc: photocraft_doc::DocId) -> egui::Id {
    egui::Id::unique(("smart-filter-selected-drawn", doc))
}

/// The selected smart filter row, while its smart object is active, the filter still exists and
/// its row is on screen (drawn this pass or the last: the Layers panel may be closed).
pub(crate) fn selected(ctx: &egui::Context, doc: &Document, active: Option<LayerId>) -> Option<FilterDrag> {
    let (layer, filter) = ctx.data(|d| d.get_temp::<FilterDrag>(selected_key(doc.id)))?;
    if active != Some(LayerId(layer)) {
        return None;
    }
    let drawn = ctx.data(|d| d.get_temp::<u64>(drawn_key(doc.id)))?;
    if drawn + 1 < ctx.cumulative_pass_nr() {
        return None;
    }
    let LayerContent::Smart(sm) = &doc.layer(LayerId(layer))?.content else { return None };
    let exists = match filter {
        Some(i) => i < sm.smart_filters.len(),
        None => !sm.smart_filters.is_empty(),
    };
    exists.then_some((layer, filter))
}

/// Forgets the selected smart filter row: its smart object's own row was clicked, or the filter
/// was deleted (its index would now name the next one).
pub(crate) fn deselect(ctx: &egui::Context, doc: photocraft_doc::DocId) {
    ctx.data_mut(|d| d.remove::<FilterDrag>(selected_key(doc)));
}

/// Forgets the selected smart filter row when the Layers panel didn't draw it this pass (its smart
/// object is in a closed group, filtered out or isolated away): Delete must never remove a filter
/// nobody sees highlighted.
pub(crate) fn forget_hidden(ctx: &egui::Context, doc: photocraft_doc::DocId) {
    let picked = ctx.data(|d| d.get_temp::<FilterDrag>(selected_key(doc)));
    let drawn = ctx.data(|d| d.get_temp::<u64>(drawn_key(doc)));
    if picked.is_some() && drawn != Some(ctx.cumulative_pass_nr()) {
        deselect(ctx, doc);
    }
}

/// While smart filters are dragged: their name by the pointer, and the copy cursor while ⌥ is held.
pub(crate) fn drag_feedback(ctx: &egui::Context, doc: &Document) {
    let Some((from, filter)) = ctx.data(|d| d.get_temp::<FilterDrag>(drag_key())) else { return };
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    let label = match (filter, doc.layer(LayerId(from)).map(|l| &l.content)) {
        (Some(i), Some(LayerContent::Smart(sm))) => sm.smart_filters.get(i).map_or_else(String::new, filter_label),
        _ => "Smart Filters".to_string(),
    };
    crate::layer_transfer::ghost(ctx, p, &label);
    if ctx.input(|i| i.modifiers.alt) {
        ctx.set_cursor_icon(egui::CursorIcon::Copy);
    }
}

/// The command for dropping the smart filters of `from` on the smart object `target`: they move
/// there, or with ⌥ held on release are copied (Photoshop).
pub(crate) fn drop_action(from: u64, filter: Option<usize>, target: LayerId, copy: bool) -> (String, Value) {
    let mut payload = json!({"from": from, "to": target.0, "copy": copy});
    if let (Some(i), Some(o)) = (filter, payload.as_object_mut()) {
        o.insert("filter".into(), json!(i));
    }
    ("layer.smartFilter.transfer".into(), payload)
}

/// Another smart object's row accepts the smart filters being dragged: outlined while the pointer
/// is over it, dropped on release. Other layers don't take them, as in Photoshop.
pub(crate) fn drop_on_row(ctx: &egui::Context, ui: &egui::Ui, l: &Layer, rect: Rect, actions: &mut Vec<(String, Value)>) {
    let Some((from, filter)) = ctx.data(|d| d.get_temp::<FilterDrag>(drag_key())) else { return };
    let Some(p) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    if from == l.id.0 || !matches!(l.content, LayerContent::Smart(_)) || !rect.contains(p) {
        return;
    }
    let t = Tokens::get(ctx);
    ui.painter().rect_stroke(rect.shrink(1.0), t.radius_sm, Stroke::new(2.0, t.accent), StrokeKind::Inside);
    if ctx.input(|i| i.pointer.any_released()) {
        actions.push(drop_action(from, filter, l.id, ctx.input(|i| i.modifiers.alt)));
    }
}

/// Where `layer.smartFilter.move` puts filter `src` (stack index, 0 = bottom) dropped on the row
/// of filter `row` (`None`: the Smart Filters row, i.e. on top of the stack), `above` or below
/// it, in a stack of `n`. `None` when the filter would stay where it is.
fn reorder_target(src: usize, row: Option<usize>, above: bool, n: usize) -> Option<usize> {
    let to = match row {
        None => n.checked_sub(1)?,
        Some(i) if i == src => return None,
        // Above a row is just over it in the stack, below a row just under it. `move` removes
        // the filter first, so the filters over `src` shift down by one.
        Some(i) if above => {
            if src < i {
                i
            } else {
                i + 1
            }
        }
        Some(i) => {
            if src < i {
                i - 1
            } else {
                i
            }
        }
    };
    (to < n && to != src).then_some(to)
}

/// A filter dragged up or down its own smart object's stack (Photoshop): over another row of the
/// stack a line marks where it lands, above or below that row (the Smart Filters row: on top),
/// and the release moves it there. The bottom filter is applied first, so the order changes the
/// result (Unsharp Mask over a blur sharpens the blurred image; under it, the sharpening is
/// blurred). Returns the filter's new index and the command.
fn reorder_drop(ui: &egui::Ui, l: &Layer, n: usize, row: Option<usize>, rect: Rect) -> Option<(usize, (String, Value))> {
    let ctx = ui.ctx();
    let (from, src) = ctx.data(|d| d.get_temp::<FilterDrag>(drag_key()))?;
    let (src, p) = (src?, ctx.input(|i| i.pointer.interact_pos())?);
    if from != l.id.0 || !rect.contains(p) {
        return None;
    }
    let above = p.y < rect.center().y;
    let to = reorder_target(src, row, above, n)?;
    let t = Tokens::get(ctx);
    let y = if above && row.is_some() { rect.top() } else { rect.bottom() };
    ui.painter().line_segment([pos2(rect.left() + 30.0, y), pos2(rect.right(), y)], Stroke::new(2.0, t.accent));
    ctx.input(|i| i.pointer.any_released()).then(|| (to, ("layer.smartFilter.move".into(), json!({"layer": l.id.0, "index": src, "to": to}))))
}

/// Smart filters dropped on the footer's Delete button: a filter's row deletes that filter, the
/// Smart Filters row clears them all.
pub(crate) fn trash_action((layer, filter): FilterDrag) -> (String, Value) {
    match filter {
        Some(i) => ("layer.smartFilter.delete".into(), json!({"layer": layer, "index": i})),
        None => ("layer.smartFilter.clearSmartFilters".into(), json!({"layer": layer})),
    }
}

/// Delete or Backspace with no modifiers while a filter's row is selected removes that filter (the
/// Smart Filters row: all of them), as the panel's Delete button does (#4751), instead of
/// deleting the smart object or reaching Edit › Clear.
pub(crate) fn keys(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    use crate::shortcut_dispatch::Focus;
    // Text fields and widgets that delete with these keys themselves keep them (#1534).
    if app.ui.text_edit.is_some() || matches!(Focus::of(ctx), Focus::Text | Focus::DeletingWidget) {
        return false;
    }
    let Some(st) = app.session.active() else { return false };
    let Some(picked) = selected(ctx, &st.doc, st.active_layer) else { return false };
    let doc = st.doc.id;
    // Only where the key would delete the smart object: Quick Mask, a single channel, a targeted
    // mask or a selection keep their own Delete.
    if crate::shortcut_dispatch::delete_key_command(app, "edit.clear".into()) != "layer.delete" {
        return false;
    }
    let mods = ctx.input(|i| i.modifiers);
    if !mods.is_none() || !ctx.input_mut(|i| i.consume_key(mods, egui::Key::Delete) || i.consume_key(mods, egui::Key::Backspace)) {
        return false;
    }
    let (command, params) = trash_action(picked);
    deselect(ctx, doc);
    if let Err(e) = app.run(&command, params) {
        app.ui.status = e;
        app.ui.status_error = true;
    }
    true
}

/// Re-opens a smart filter's dialog with its recorded parameters; OK runs
/// `layer.smartFilter.setParams` instead of adding another filter (see `dialogs::confirm`).
pub fn open_editor(app: &mut PhotocraftApp, ctx: &egui::Context, layer: u64, sm: &SmartObject, index: usize) {
    let Some(f) = sm.smart_filters.get(index) else { return };
    // Camera Raw has its own full-window dialog rather than a generic parameter form.
    if f.command == photocraft_engine::lens_cmds::RAW {
        if let Err(e) = crate::camera_raw_ui::open_smart_filter(app, ctx, photocraft_doc::LayerId(layer), index) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    // So has the Filter Gallery.
    if f.command == "filter.filterGallery" {
        if let Err(e) = crate::gallery_ui::open_smart_filter(app, photocraft_doc::LayerId(layer), index) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    // And Liquify, which re-opens with the filter's recorded strokes.
    if f.command == photocraft_engine::distort_cmds::LIQUIFY {
        if let Err(e) = crate::liquify_ui::open_smart_filter(app, ctx, photocraft_doc::LayerId(layer), index) {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        return;
    }
    let target = match crate::smart_filter_dialog::target(app, photocraft_doc::LayerId(layer), index) {
        Ok(target) => target,
        Err(e) => {
            crate::notices::error(app, e);
            return;
        }
    };
    let Some(id) = crate::adjust_dialog::open(app, &f.command).or_else(|| {
        let id = crate::filter_dialog::has_dialog(&f.command).then(|| crate::filter_dialog::open(app, &f.command)).flatten()?;
        // The filter dialog opens on the user's last-used values; a recorded filter that lacks a
        // key must show (and save) the schema default instead.
        let mut fields = app.ui.dialog_mut(id)?.fields.clone();
        crate::filter_dialog::reset_fields(app, &mut fields);
        app.ui.dialog_mut(id)?.fields = fields;
        Some(id)
    }) else {
        return;
    };
    if let Some(d) = app.ui.dialog_mut(id) {
        d.fields.insert("__smartFilter".into(), target);
        if let Value::Object(p) = &f.params {
            for (k, v) in p {
                if !k.starts_with("__") {
                    d.fields.insert(k.clone(), v.clone());
                }
            }
        }
    }
}

/// The command a filter dialog's live preview runs ([`confirm_command`]'s): the Smart Filter blending
/// command for its Blending Options dialog, `layer.smartFilter.setParams` while it edits a smart filter
/// (#6324), else `command`.
pub fn preview_command<'a>(fields: &serde_json::Map<String, Value>, command: &'a str) -> &'a str {
    if crate::smart_filter_dialog::owns(fields) {
        crate::smart_filter_dialog::COMMAND
    } else if fields.contains_key("__smartFilter") {
        "layer.smartFilter.setParams"
    } else {
        command
    }
}

/// The command a confirmed filter dialog runs: smart-filter edits update the recorded filter.
pub fn confirm_command(app: &PhotocraftApp, fields: &serde_json::Map<String, Value>, command: String, params: Value) -> Result<(String, Value), String> {
    // One builder for the preview and for OK, so the two cannot differ.
    if fields.contains_key("__smartFilter") {
        return crate::smart_filter_dialog::edit_command(app, fields);
    }
    Ok((command, params))
}

#[cfg(test)]
mod editor_tests;

#[cfg(test)]
mod layer_thumbnail_scaling_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn sub_rows_stay_compact_while_the_smart_filter_mask_follows_thumbnail_setting() {
        for setting in [LayerThumbnailSize::None, LayerThumbnailSize::Small, LayerThumbnailSize::Medium, LayerThumbnailSize::Large] {
            let mut session = photocraft_engine::Session::new();
            session.execute("file.new", json!({"width": 50, "height": 38})).unwrap();
            session.execute("edit.fill", json!({"color": "#8A42B3"})).unwrap();
            session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
            session.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
            session.execute("filter.sharpen.unsharpMask", json!({"amount": 80, "radius": 1, "threshold": 0})).unwrap();
            session.execute("layer.layerStyle.dropShadow", json!({"size": 2})).unwrap();
            assert!(matches!(session.active().unwrap().doc.layers.last().unwrap().content, LayerContent::Smart(_)));
            let mut h = Harness::builder().with_size(vec2(1440.0, 1400.0)).with_max_steps(64).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
                PhotocraftApp::new(session, crate::Services::default())
            });
            h.state_mut().ui.layers_panel_options.thumbnail_size = setting;
            h.run_steps(8);
            // Filter and effect rows keep Photoshop's compact height at every thumbnail size.
            for label in ["Gaussian Blur", "Unsharp Mask", "Effects", "Drop Shadow"] {
                let rect = h.get_by_label(label).rect();
                assert!((rect.height() - SUB_ROW_H).abs() < 1.0, "{setting:?}: {label} was {rect:?}, expected {SUB_ROW_H}");
            }
            // Only the Smart Filters header grows, just enough for its shared-mask thumbnail.
            let (header, mask) = smart_filters_header_geometry(setting, true);
            let rect = h.get_by_label("Smart Filters").rect();
            assert!((rect.height() - header).abs() < 1.0, "{setting:?}: Smart Filters was {rect:?}, expected {header}");
            assert!(mask <= header - 6.0 || mask == 0.0, "{setting:?}: a {mask} mask must fit a {header} row");
            assert_eq!(mask == 0.0, setting == LayerThumbnailSize::None, "{setting:?}: the mask hides only at None");
            if let Some(dir) = std::env::var_os("PHOTOCRAFT_LAYERS_SNAPSHOTS") {
                let dir = std::path::PathBuf::from(dir);
                std::fs::create_dir_all(&dir).unwrap();
                h.render().unwrap().save(dir.join(format!("layers-sub-rows-{setting:?}.png"))).unwrap();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use photocraft_engine::Session;

    use super::*;
    use crate::Services;

    fn smart(app: &PhotocraftApp) -> (photocraft_doc::Layer, photocraft_doc::SmartObject) {
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap().clone();
        let LayerContent::Smart(sm) = l.content.clone() else { panic!("not a smart object") };
        (l, sm)
    }

    /// FILE-215-3: clicking the filter mask thumbnail targets it, the Brush paints into it (not
    /// the layer mask, no rasterize prompt), and clicking the layer thumbnail goes back.
    #[test]
    fn the_filter_mask_thumbnail_targets_the_filter_mask() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
        let (l, sm) = smart(&app);
        let actions = super::filter_mask_click(&l, &sm, egui::Modifiers::NONE);
        assert_eq!(actions.last().unwrap().0, "ui.filterMaskTarget");
        assert!(super::filter_mask_click(&l, &sm, egui::Modifiers::ALT).is_empty(), "nothing to view yet");
        app.ui.filter_mask_target = Some(l.id.0);
        app.sync_views();
        assert!(app.filter_mask_targeted());
        assert_eq!(crate::canvas::paint_target(&app), json!("filterMask"));
        assert_eq!(app.with_mask_target("filter.blur.gaussianBlur", json!({}))["target"], "filterMask");
        assert!(crate::rasterize_prompt::needed(&app).is_none(), "painting the filter mask needs no rasterizing");
        app.run(
            "paint.stroke",
            json!({"points": [[4, 0], [4, 31]], "size": 4, "hardness": 1, "color": "#000000", "target": crate::canvas::paint_target(&app)}),
        )
        .unwrap();
        let (l, sm) = smart(&app);
        assert_eq!(sm.filter_mask.as_ref().unwrap().value(4, 16), 0.0);
        assert!(l.mask.is_none(), "no layer mask");
        assert_eq!(super::filter_mask_click(&l, &sm, egui::Modifiers::ALT)[0].1["mask"], "filter");
        // Targeting a layer mask (or the pixels, `ui.maskTarget`) ends it.
        app.ui.mask_target = true;
        app.sync_views();
        assert_eq!(app.ui.filter_mask_target, None, "a layer mask target replaces it");
    }

    #[test]
    fn selecting_the_filter_mask_replaces_a_layer_mask_view() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
        app.run("layer.layerMask.revealAll", json!({})).unwrap();
        app.run("view.layerMask", json!({"mode": "gray"})).unwrap();
        app.sync_mask_targets();
        assert!(app.layer_mask_targeted());
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.select_filter_mask_target(Some(id.0));
        app.sync_views();
        assert!(app.filter_mask_targeted());
        assert!(!app.layer_mask_targeted());
        assert!(photocraft_engine::mask_view_cmds::current(app.session.active().unwrap()).is_none());
        app.run("paint.stroke", json!({"points": [[8, 8]], "size": 6, "hardness": 1, "color": "#000000"})).unwrap();
        let st = app.session.active().unwrap();
        let layer = st.doc.layer(id).unwrap();
        assert_eq!(layer.mask.as_ref().unwrap().value(8, 8), 1.0);
        let LayerContent::Smart(sm) = &layer.content else { panic!("smart object") };
        assert_eq!(sm.filter_mask.as_ref().unwrap().value(8, 8), 0.0);
        app.run("view.layerMask", json!({"mode": "gray", "mask": "filter"})).unwrap();
        app.select_filter_mask_target(Some(id.0));
        assert!(photocraft_engine::mask_view_cmds::current(app.session.active().unwrap()).unwrap().filter, "a matching filter-mask view stays open");
    }

    /// A pixel layer, a shape layer and a smart object, in that order.
    fn layers() -> [Layer; 3] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        // An empty layer can't become a smart object: fill it first.
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "color", "color": "#336699"})).unwrap();
        let smart = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        let shape = s.execute("shape.create", json!({"kind": "rect", "rect": [8, 8, 40, 24], "fill": "#cc3333"})).unwrap()["layer"].as_u64().unwrap();
        let pixel = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
        let doc = &s.active().unwrap().doc;
        [pixel, shape, smart].map(|id| doc.layer(photocraft_doc::LayerId(id)).unwrap().clone())
    }

    /// Stack [A, B, C] (A at the bottom, drawn last): where a dragged filter lands.
    #[test]
    fn a_filter_dropped_above_or_below_a_row_lands_next_to_it() {
        // A over C: [B, C, A]. A under C: [B, A, C].
        assert_eq!(reorder_target(0, Some(2), true, 3), Some(2));
        assert_eq!(reorder_target(0, Some(2), false, 3), Some(1));
        // C under A: [C, A, B]. C over A: [A, C, B].
        assert_eq!(reorder_target(2, Some(0), false, 3), Some(0));
        assert_eq!(reorder_target(2, Some(0), true, 3), Some(1));
        // The Smart Filters row: on top.
        assert_eq!(reorder_target(0, None, false, 3), Some(2));
        // Next to where it already is, or on itself: nothing to do.
        assert_eq!(reorder_target(1, Some(2), false, 3), None);
        assert_eq!(reorder_target(1, Some(0), true, 3), None);
        assert_eq!(reorder_target(1, Some(1), true, 3), None);
        assert_eq!(reorder_target(2, None, true, 3), None);
        assert_eq!(reorder_target(0, None, true, 0), None);
    }

    #[test]
    fn shape_layers_and_smart_objects_carry_a_thumbnail_badge() {
        let [pixel, shape, smart] = layers();
        assert_eq!(thumb_badge_kind(&pixel), None);
        assert_eq!(thumb_badge_kind(&shape), Some(ThumbBadge::Shape), "#4584");
        assert_eq!(thumb_badge_kind(&smart), Some(ThumbBadge::Smart));
    }

    /// The rects `thumb_badge` fills for `l` over `thumb`, with their colours.
    fn filled(l: &Layer, thumb: Rect) -> Vec<(Rect, Color32)> {
        let l = l.clone();
        let mut h = egui_kittest::Harness::new_ui(move |ui| thumb_badge(ui, &l, thumb));
        h.run();
        let badge = thumb_badge_rect(thumb).expand(0.5);
        let rects = h.output().shapes.iter().filter_map(|s| match &s.shape {
            egui::Shape::Rect(r) if badge.contains_rect(r.rect) && r.fill != Color32::TRANSPARENT => Some((r.rect, r.fill)),
            _ => None,
        });
        rects.collect()
    }

    /// #4584: a shape layer's thumbnail shows a path with four anchor points in its corner.
    #[test]
    fn shape_badge_paints_a_path_with_an_anchor_on_each_corner() {
        let [pixel, shape, smart] = layers();
        let thumb = Rect::from_min_size(pos2(20.0, 20.0), vec2(34.0, 34.0));
        let t = Tokens::get(&egui::Context::default());
        let frame = thumb_badge_rect(thumb);
        assert!(frame.right() > thumb.right() && frame.bottom() > thumb.bottom(), "over the bottom-right corner");
        assert!(filled(&pixel, thumb).is_empty(), "a pixel layer has no badge");

        let drawn = filled(&shape, thumb);
        assert!(drawn.contains(&(frame, t.field)), "the badge's frame");
        let anchors: Vec<Rect> = drawn.iter().filter(|(_, c)| *c == t.icon).map(|(r, _)| *r).collect();
        let path = frame.shrink(SHAPE_PATH_INSET);
        let corners = [path.left_top(), path.right_top(), path.right_bottom(), path.left_bottom()];
        assert_eq!(anchors.iter().map(Rect::center).collect::<Vec<_>>(), corners);
        for a in &anchors {
            assert_eq!(a.size(), vec2(SHAPE_ANCHOR, SHAPE_ANCHOR));
            assert!(frame.shrink(1.0).contains_rect(*a), "inside the frame's border");
        }

        // The smart-object badge keeps its frame and draws no anchors.
        let drawn = filled(&smart, thumb);
        assert!(drawn.contains(&(frame, t.field)));
        assert!(drawn.iter().all(|(_, c)| *c != t.icon));
    }

    /// #5689: an Image › Adjustments recorded on a smart object (Brightness/Contrast…) opens its
    /// own editor from the Smart Filters row, with the recorded settings, and OK updates that
    /// filter instead of stacking another.
    #[test]
    fn an_adjustment_smart_filter_opens_its_editor_and_updates_it() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"contents": "black"})).unwrap();
        let id = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("image.adjustments.brightnessContrast", json!({"brightness": 40, "contrast": -10})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        let sm = match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
            LayerContent::Smart(sm) => sm.clone(),
            other => panic!("expected a smart object, got {other:?}"),
        };
        assert_eq!(sm.smart_filters.len(), 1);
        open_editor(&mut app, &egui::Context::default(), id, &sm, 0);
        let d = app.ui.dialogs.last().expect("the adjustment editor opened");
        assert_eq!(d.fields.get("__adjust").and_then(Value::as_str), Some("brightnessContrast"));
        assert_eq!(d.fields.get("brightness").and_then(Value::as_f64), Some(40.0), "the recorded settings");
        assert_eq!(d.fields.get("contrast").and_then(Value::as_f64), Some(-10.0));
        let dialog = d.id;
        // OK updates the recorded filter (one history step), it does not add a second filter.
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        let LayerContent::Smart(sm) = &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content else {
            panic!("the layer stays a smart object");
        };
        assert_eq!(sm.smart_filters.len(), 1, "no filter was stacked");
        assert_eq!(sm.smart_filters[0].params.get("brightness").and_then(Value::as_f64), Some(40.0));
        assert_eq!(sm.smart_filters[0].params.get("contrast").and_then(Value::as_f64), Some(-10.0));
    }

    /// #6324: a smart filter opened for editing previews, and its preview replaces the filter in
    /// the stack (as OK does) instead of stacking a second one.
    #[test]
    fn an_edited_smart_filter_previews_what_ok_gives() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 48, "background": "#ffffff"})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.all", json!({})).unwrap();
        s.execute("edit.stroke", json!({"width": 4, "color": "#000000", "location": "inside"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        let id = s.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
        s.execute("filter.blur.gaussianBlur", json!({"radius": 1.0})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.background_jobs = false;
        let smart = |app: &PhotocraftApp, doc: &Document| match &doc.layer(LayerId(id)).unwrap().content {
            LayerContent::Smart(sm) => sm.clone(),
            other => panic!("expected a smart object, got {other:?} ({})", app.ui.status),
        };
        let sm = smart(&app, &app.session.active().unwrap().doc);
        open_editor(&mut app, &egui::Context::default(), id, &sm, 0);
        let d = app.ui.dialogs.last_mut().expect("the filter's dialog opened");
        assert_eq!(d.fields.get("__preview"), Some(&json!(true)), "the dialog previews");
        d.fields.insert("radius".into(), json!(6.0));
        let dialog = d.id;

        crate::canvas::ensure_filter_preview(&mut app, 0, &egui::Context::default()).expect("a preview");
        let preview = app.filter_preview.as_ref().unwrap();
        assert_eq!(preview.key.command, "layer.smartFilter.setParams");
        // The dialog's own preview window matches it to the dialog by that command.
        let fields = &app.ui.dialogs.last().unwrap().fields;
        assert_eq!(preview_command(fields, "filter.blur.gaussianBlur"), preview.key.command);
        assert_eq!(preview_command(&serde_json::Map::new(), "filter.blur.gaussianBlur"), "filter.blur.gaussianBlur");
        let shown = preview.result.clone().expect("the preview's document");
        let filters = smart(&app, &shown).smart_filters;
        assert_eq!(filters.len(), 1, "the edited filter is replaced, not stacked");
        assert_eq!(filters[0].params.get("radius").and_then(Value::as_f64), Some(6.0));
        assert_eq!(smart(&app, &app.session.active().unwrap().doc).smart_filters[0].params["radius"], json!(1.0), "the document is untouched");

        crate::dialogs::confirm(&mut app, dialog).unwrap();
        let doc = &app.session.active().unwrap().doc;
        assert_eq!(smart(&app, doc).smart_filters.len(), 1);
        assert!(photocraft_compose::flatten(&shown).px == photocraft_compose::flatten(doc).px, "OK gives what the preview showed");
    }

    /// FILE-215-3: with the filter mask targeted, Move and Edit › Transform act on the smart object
    /// (no filter-mask target, so a move doesn't re-render the filters), and painting and filters
    /// still edit the filter mask.
    #[test]
    fn only_filter_mask_edits_carry_the_filter_mask_target() {
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
        app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
        app.run("filter.blur.gaussianBlur", json!({"radius": 2})).unwrap();
        let id = app.session.active().unwrap().active_layer.unwrap();
        app.select_filter_mask_target(Some(id.0));
        assert!(app.filter_mask_targeted());
        for cmd in [
            "layer.translate",
            "select.float",
            "edit.transform.rotate180",
            "filter.convertForSmartFilters",
            "image.applyImage",
            "filter.liquify",
            "filter.lastFilter",
            "image.adjustments.desaturate",
            "filter.render.flame",
        ] {
            assert!(app.with_mask_target(cmd, json!({"dx": 1})).get("target").is_none(), "{cmd}");
        }
        for cmd in [
            "paint.stroke",
            "filter.blur.gaussianBlur",
            "edit.fill",
            "edit.paste",
            "image.adjustments.invert",
            "image.adjustments.equalize",
            "filter.blur.average",
        ] {
            assert_eq!(app.with_mask_target(cmd, json!({}))["target"], "filterMask", "{cmd}");
        }
    }
}
