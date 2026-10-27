//! The Channels panel (tab of the Layers card): composite, colour, alpha / spot and Quick Mask
//! rows with thumbnails and eye toggles. Every action is an engine command (`channel.*`,
//! `select.loadSelection`, `select.saveSelection`, …); the targeted channel and the eyes live in
//! the engine's `DocState::channel_view`, so agents see the same state through `inspect`.
//! Rows can be dragged onto the footer's buttons, as in Photoshop (#4689): New duplicates the
//! channel, Delete deletes it, and Load Channel as Selection loads it.

use egui::{Align2, Color32, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::ColorIndicates;
use photocraft_engine::channel_cmds::{self, ChannelTarget};
use serde_json::{Value, json};

use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons, widgets};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Composite,
    Color(usize),
    Alpha(usize),
    QuickMask,
    /// The selected layer's mask: a temporary channel, listed while the layer is selected.
    LayerMask,
}

impl Row {
    /// The channel reference the engine commands take.
    fn reference(self) -> Value {
        match self {
            Row::Composite => json!("composite"),
            Row::Color(k) => json!({ "color": k }),
            Row::Alpha(i) => json!(i),
            Row::QuickMask => json!("quickMask"),
            Row::LayerMask => json!("mask"),
        }
    }
}

/// ⌘ = new selection, ⌘⇧ add, ⌘⌥ subtract, ⌘⌥⇧ intersect (Photoshop's thumbnail ⌘-clicks).
pub fn load_operation(m: egui::Modifiers) -> &'static str {
    match (m.shift, m.alt) {
        (true, true) => "intersect",
        (true, false) => "add",
        (false, true) => "subtract",
        _ => "new",
    }
}

pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        ui.add_space(6.0);
        ui.label(RichText::new(tl!("No document")).color(t.text_faint));
        return;
    };
    let doc = st.doc.clone();
    let view = st.channel_view.clone();
    // ⌥-click mask view (#196): gray shows the mask alone (colour eyes off), overlay over the composite.
    let mask_view = photocraft_engine::mask_view_cmds::current(st).map(|v| v.mode);
    let gray_view = mask_view == Some(photocraft_engine::mask_view_cmds::MaskViewMode::Gray);
    let active_layer = st.active_layer;
    let mode = doc.pixel_format().mode;
    let colors = mode.color_channels();
    let quick = doc.quick_mask.is_some();
    let ctx = ui.ctx().clone();
    let thumbs = app.channel_thumbs(&ctx);
    // The active layer's mask (Photoshop lists it, in italics, below the colour channels).
    let masked = active_layer.and_then(|id| doc.layer(id)).filter(|l| l.mask.is_some());
    let mask_tex = masked.as_ref().and_then(|l| Some(app.mask_thumb(&ctx, &doc, l.id, l.mask.as_ref()?)));
    let mask_targeted = masked.is_some()
        && (app.ui.mask_target || view.target == ChannelTarget::LayerMask)
        && (view.target == ChannelTarget::Composite || view.target == ChannelTarget::LayerMask)
        && !quick;
    // Multichannel images are their ink channels only (no composite or colour rows).
    let multichannel = doc.mode == photocraft_doc::ColorMode::Multichannel;
    let mut rows = if multichannel { Vec::new() } else { vec![Row::Composite] };
    if colors > 1 && !multichannel {
        rows.extend((0..colors).map(Row::Color));
    }
    if masked.is_some() {
        rows.push(Row::LayerMask);
    }
    rows.extend((0..doc.channels.len()).map(Row::Alpha));
    if quick {
        rows.push(Row::QuickMask);
    }
    let shown_colors = if colors > 1 { colors } else { 0 };
    let mut actions: Vec<(String, Value)> = Vec::new();
    // The buttons sit in a footer at the panel's bottom, like Photoshop's.
    let footer = widgets::footer_height(ui) + ui.spacing().item_spacing.y;
    let fill = ui.available_height() > footer + 60.0;
    let rows_h = if fill { ui.available_height() - footer } else { f32::INFINITY };
    let mut mask_click = None;
    let mut drawn = Vec::new();
    // An alpha or spot channel dragged over the rows reorders like Photoshop's (#9163): a line marks
    // where it lands, above the colour channels it lands first, and colour channels don't move.
    let dragged_alpha = ctx.data(|d| d.get_temp::<(Row, String)>(drag_id())).and_then(|(row, _)| match row {
        Row::Alpha(i) => Some(i),
        _ => None,
    });
    let pointer = ctx.input(|i| i.pointer.interact_pos());
    let released = ctx.input(|i| i.pointer.any_released());
    let alpha_count = doc.channels.len();
    // The slot the dragged channel would take (0 = first alpha) and where its line goes.
    let mut insertion: Option<(usize, f32)> = None;
    let mut alphas_top: Option<f32> = None;
    egui::ScrollArea::vertical().max_height(rows_h).min_scrolled_height(if fill { rows_h } else { 0.0 }).auto_shrink([false, !fill]).show(ui, |ui| {
        for row in rows {
            let (name, thumb_idx, slot, visible, selected) = match row {
                Row::Composite => (
                    channel_cmds::composite_name(mode).to_string(),
                    0,
                    Some(2),
                    view.visible_colors(colors) == colors && !gray_view,
                    view.target == ChannelTarget::Composite && !quick && !mask_targeted,
                ),
                Row::Color(k) => (
                    channel_cmds::color_names(mode)[k].to_string(),
                    1 + k,
                    Some(3 + k),
                    view.color_visible(k) && !gray_view,
                    (view.target == ChannelTarget::Composite && !quick && !mask_targeted) || view.target == ChannelTarget::Color(k),
                ),
                Row::Alpha(i) => (
                    doc.channels[i].name.clone(),
                    1 + shown_colors + i,
                    Some(3 + shown_colors + i).filter(|s| *s <= 9),
                    view.alpha_shown(i),
                    view.target == ChannelTarget::Alpha(i),
                ),
                Row::QuickMask => {
                    ("Quick Mask".to_string(), 1 + shown_colors + doc.channels.len(), None, !view.quick_mask_hidden, view.target == ChannelTarget::Composite)
                }
                Row::LayerMask => {
                    (masked.as_ref().map(|l| format!("{} Mask", l.name)).unwrap_or_default(), usize::MAX, None, mask_view.is_some(), mask_targeted)
                }
            };
            let row_h = if t.pro { 36.0 } else { 40.0 };
            let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), row_h), Sense::click_and_drag());
            if let Row::Alpha(0) = row {
                alphas_top = Some(rect.top());
            }
            if dragged_alpha.is_some()
                && let Some(p) = pointer.filter(|p| rect.contains(*p))
            {
                insertion = match row {
                    Row::Alpha(j) if p.y < rect.center().y => Some((j, rect.top())),
                    Row::Alpha(j) => Some((j + 1, rect.bottom())),
                    Row::QuickMask => Some((alpha_count, rect.top())),
                    // Above the alpha channels: Photoshop puts it first among them.
                    Row::Composite | Row::Color(_) | Row::LayerMask => Some((0, f32::NAN)),
                };
            }
            if resp.drag_started() {
                ctx.data_mut(|d| d.insert_temp(drag_id(), (row, name.clone())));
            }
            let painter = ui.painter_at(rect.expand(1.0));
            if selected {
                painter.rect_filled(rect, if t.pro { 0.0 } else { t.radius }, if t.pro { t.row_selected } else { t.hover });
            } else if resp.hovered() {
                painter.rect_filled(rect, if t.pro { 0.0 } else { t.radius }, t.hover.gamma_multiply(0.5));
            }
            if t.pro {
                painter.line_segment([pos2(rect.left() + 30.0, rect.top()), pos2(rect.left() + 30.0, rect.bottom())], Stroke::new(1.0, t.separator));
                painter.line_segment([rect.left_bottom(), rect.right_bottom()], Stroke::new(1.0, t.separator));
            }
            let eye = Rect::from_min_size(pos2(rect.left() + 6.0, rect.center().y - 11.0), vec2(22.0, 22.0));
            let eye_resp = ui.interact(eye, ui.scope_id().with(("chan-eye", format!("{:?}", row.reference()))), Sense::click());
            // Like Photoshop, the temporary mask row's eye is empty until the mask is shown.
            if row != Row::LayerMask || visible || eye_resp.hovered() {
                icons::paint(ui, eye, if visible { "eye" } else { "eye-off" }, 14.0, if visible { t.icon } else { t.text_faint });
            }
            let eye_resp = eye_resp.on_hover_text(if row == Row::LayerMask { tl!("Show the layer mask as an overlay") } else { tl!("Toggle visibility") });
            eye_resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, format!("Visibility {name}")));
            let layer_id = masked.as_ref().map(|l| l.id.0);
            if eye_resp.clicked() {
                let view_cmd = photocraft_engine::mask_view_cmds::ID.to_string();
                match row {
                    // The mask's eye: the overlay on, or the mask view off.
                    Row::LayerMask => actions.push((view_cmd, json!({ "layer": layer_id, "mode": if visible { "off" } else { "overlay" } }))),
                    // Showing the composite again over a gray mask view keeps the mask as an overlay.
                    Row::Composite if gray_view => actions.push((view_cmd, json!({ "layer": layer_id, "mode": "overlay" }))),
                    _ => actions.push(("channel.setVisible".into(), json!({ "channel": row.reference(), "visible": !visible }))),
                }
            }
            let ts = if t.pro { 28.0 } else { 30.0 };
            let cell = Rect::from_min_size(pos2(rect.left() + 36.0, rect.center().y - ts / 2.0), vec2(ts, ts));
            // Thumbnails keep the document's aspect ratio (the textures are letterboxed squares).
            let (thumb, uv) = fit_thumb(cell, doc.size.width, doc.size.height);
            let tex = if row == Row::LayerMask { mask_tex } else { thumbs.get(thumb_idx).copied() };
            if let Some(tex) = tex {
                painter.image(tex, thumb, uv, Color32::WHITE);
            }
            drawn.push((name.clone(), thumb));
            // Rows are painted: name them for screen readers and UI tests.
            resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, &name));
            // Spot channels: a swatch of the ink on the thumbnail corner.
            if let Row::Alpha(i) = row
                && let Some((ink, _)) = doc.channels[i].spot
            {
                let c = ink.to_rgb().map(|v| (v.clamp(0.0, 1.0) * 255.0) as u8);
                painter.rect_filled(Rect::from_min_size(thumb.right_bottom() - vec2(9.0, 9.0), vec2(9.0, 9.0)), 0.0, Color32::from_rgb(c[0], c[1], c[2]));
            }
            painter.rect_stroke(thumb, 0.0, Stroke::new(1.0, if t.pro { Color32::from_gray(20) } else { t.field_border }), StrokeKind::Outside);
            let italic = matches!(row, Row::QuickMask | Row::LayerMask) || matches!(row, Row::Alpha(i) if doc.channels[i].spot.is_some());
            let mut job = egui::text::LayoutJob::default();
            let font = if selected && !t.pro { theme::medium(12.5) } else { egui::FontId::proportional(12.0) };
            // Built-in labels only: alpha/spot names and "{layer} Mask" stay verbatim.
            let label = match row {
                Row::Composite | Row::Color(_) | Row::QuickMask => tl!(&name).to_owned(),
                Row::Alpha(_) | Row::LayerMask => name.clone(),
            };
            job.append(&label, 0.0, egui::TextFormat { font_id: font, color: t.text, italics: italic, ..Default::default() });
            let galley = painter.layout_job(job);
            let text_pos = pos2(cell.right() + 10.0, rect.center().y - galley.size().y / 2.0);
            painter.galley(text_pos, galley, t.text);
            if let Some(slot) = slot {
                painter.text(
                    pos2(rect.right() - 8.0, rect.center().y),
                    Align2::RIGHT_CENTER,
                    crate::shortcuts::pretty(&format!("Cmd+{slot}")),
                    theme::mono(11.0),
                    t.text_faint,
                );
            }
            let rename_id = egui::Id::unique(("chan-rename", doc.id.0, format!("{:?}", row.reference())));
            if resp.clicked() && !eye_resp.clicked() {
                let m = ui.input(|i| i.modifiers);
                if m.command && row == Row::LayerMask {
                    let layer = masked.as_ref().map(|l| l.id.0);
                    actions.push(("select.loadSelection".into(), json!({ "channel": "mask", "layer": layer, "operation": load_operation(m) })));
                } else if m.command {
                    actions.push(("select.loadSelection".into(), json!({ "channel": row.reference(), "operation": load_operation(m) })));
                } else if row == Row::LayerMask {
                    // Targets the mask, as clicking its thumbnail in the Layers panel does.
                    mask_click = Some(true);
                    actions.push(("channel.target".into(), json!({ "channel": "mask" })));
                } else {
                    if matches!(row, Row::Composite | Row::Color(_)) {
                        mask_click = Some(false);
                    }
                    let target = if row == Row::QuickMask { json!("composite") } else { row.reference() };
                    actions.push(("channel.target".into(), json!({ "channel": target })));
                }
            }
            // Like Photoshop: double-clicking the thumbnail opens Channel Options (where an alpha
            // channel becomes a spot channel, #3761), double-clicking the name renames it.
            let on_thumb = resp.interact_pointer_pos().is_some_and(|p| p.x < cell.right() + 4.0);
            if row == Row::QuickMask && resp.double_clicked() {
                actions.push(("ui.channelOptions".into(), json!("quickMask")));
            }
            // The layer mask's row: Layer Mask Display Options (the rubylith's colour, opacity).
            if row == Row::LayerMask && resp.double_clicked() {
                actions.push(("ui.channelOptions".into(), json!("mask")));
            }
            if let Row::Alpha(i) = row {
                if resp.double_clicked() && on_thumb {
                    actions.push(("ui.channelOptions".into(), json!(i)));
                } else if resp.double_clicked() {
                    ctx.data_mut(|d| d.insert_temp(rename_id, name.clone()));
                }
                if let Some(mut text) = ctx.data(|d| d.get_temp::<String>(rename_id)) {
                    let edit_rect = Rect::from_min_max(pos2(text_pos.x - 3.0, rect.center().y - 11.0), pos2(rect.right() - 36.0, rect.center().y + 11.0));
                    let te = ui.put(edit_rect, egui::TextEdit::singleline(&mut text).font(egui::FontId::proportional(12.5))).accessible_name(tl!("Name"));
                    if !te.has_focus() && !te.lost_focus() {
                        te.request_focus();
                    }
                    let (enter, esc) = ui.input(|i| (i.key_pressed(egui::Key::Enter), i.key_pressed(egui::Key::Escape)));
                    if esc {
                        ctx.data_mut(|d| d.remove::<String>(rename_id));
                    } else if enter || te.lost_focus() {
                        ctx.data_mut(|d| d.remove::<String>(rename_id));
                        if !text.trim().is_empty() && text != name {
                            actions.push(("channel.rename".into(), json!({ "channel": i, "name": text.trim() })));
                        }
                    } else {
                        ctx.data_mut(|d| d.insert_temp(rename_id, text));
                    }
                }
            }
            resp.context_menu(|ui| {
                crate::widgets::menu_scroll(ui, |ui| {
                    ui.set_min_width(210.0);
                    let a = &mut actions;
                    match row {
                        Row::Alpha(i) => {
                            let ch = &doc.channels[i];
                            item(ui, a, "Duplicate Channel", "channel.duplicate", json!({ "channel": i }));
                            item(ui, a, "Delete Channel", "channel.delete", json!({ "channel": i }));
                            item(ui, a, "Rename Channel…", "ui.renameChannel", json!(i));
                            item(ui, a, "Channel Options…", "ui.channelOptions", json!(i));
                            ui.separator();
                            if ch.spot.is_some() {
                                item(ui, a, "Merge Spot Channel", "channel.mergeSpot", json!({ "channel": i }));
                                item(ui, a, "Convert to Alpha Channel", "channel.options", json!({ "channel": i, "indicates": "masked" }));
                            } else {
                                indicates_items(ui, a, json!(i), ch.indicates);
                                overlay_colors(ui, a, json!(i));
                            }
                        }
                        Row::QuickMask => {
                            item(ui, a, "Exit Quick Mask", "select.editInQuickMaskMode", json!({ "on": false }));
                            item(ui, a, "Quick Mask Options…", "ui.channelOptions", json!("quickMask"));
                            indicates_items(ui, a, json!("quickMask"), doc.quick_mask.as_ref().map(|q| q.indicates).unwrap_or_default());
                            overlay_colors(ui, a, json!("quickMask"));
                        }
                        Row::Composite | Row::Color(_) => {
                            item(ui, a, "Duplicate Channel", "channel.duplicate", json!({ "channel": row.reference() }));
                        }
                        Row::LayerMask => {
                            let enabled = masked.as_ref().and_then(|l| l.mask.as_ref()).is_none_or(|m| m.enabled);
                            item(ui, a, crate::layer_menu_ui::mask_toggle_label(enabled), "layer.layerMask.enabled", json!({}));
                            item(ui, a, "Delete Layer Mask", "layer.layerMask.delete", json!({}));
                            item(ui, a, "Layer Mask Display Options…", "ui.channelOptions", json!("mask"));
                        }
                    }
                    ui.separator();
                    item(ui, a, "New Channel…", "channel.new", json!({}));
                    item(ui, a, "New Spot Channel…", "ui.newSpotChannel", json!({}));
                    ui.separator();
                    item(ui, a, "Split Channels", "channel.split", json!({}));
                    item(ui, a, "Merge Channels…", "channel.merge", json!({}));
                });
            });
        }
        if let (Some(from), Some((slot, y))) = (dragged_alpha, insertion) {
            let y = if y.is_nan() { alphas_top.unwrap_or(y) } else { y };
            if y.is_finite() {
                let r = ui.min_rect();
                ui.painter().hline(r.x_range(), y, Stroke::new(2.0, t.accent));
            }
            // `channel.move` takes the channel's final index: a slot below it is one less.
            let to = if slot > from { slot - 1 } else { slot };
            if released && to != from {
                actions.push(("channel.move".into(), json!({ "channel": from, "to": to })));
            }
        }
    });
    // A row being dragged: its name follows the pointer until it is dropped.
    let dragged = ctx.data(|d| d.get_temp::<(Row, String)>(drag_id()));
    if let Some((_, name)) = &dragged
        && let Some(p) = ctx.input(|i| i.pointer.interact_pos())
    {
        crate::layer_transfer::ghost(&ctx, p, name);
    }
    let mask_layer = masked.as_ref().map(|l| l.id.0);
    let drop = |ui: &egui::Ui, button: &egui::Response, on: Footer| {
        let (row, name) = dragged.as_ref()?;
        let action = drop_action(*row, name, mask_layer, on)?;
        crate::panels::dropped_on(ui, button).then_some(action)
    };
    // Photoshop's order from the left: Load as selection, Save selection, New, Delete.
    widgets::panel_footer(ui, |ui| {
        let target_ref = match view.target {
            ChannelTarget::Alpha(i) => json!(i),
            ChannelTarget::Color(k) => json!({ "color": k }),
            ChannelTarget::Composite => json!("composite"),
            ChannelTarget::LayerMask => json!("mask"),
            ChannelTarget::VectorMask => json!("vectorMask"),
        };
        let trash = icons::button(ui, "trash", 26.0, false, tl!("Delete current channel"));
        if trash.clicked() {
            actions.push(("channel.delete".into(), json!({})));
        }
        actions.extend(drop(ui, &trash, Footer::Delete));
        let new = icons::button(ui, "plus", 26.0, false, tl!("Create new channel"));
        if new.clicked() {
            actions.push(("channel.new".into(), json!({})));
        }
        actions.extend(drop(ui, &new, Footer::New));
        if icons::button(ui, "square-dashed", 26.0, false, tl!("Save selection as channel")).clicked() {
            actions.push(("select.saveSelection".into(), json!({})));
        }
        let load = icons::button(ui, "circle-dashed", 26.0, false, tl!("Load channel as selection"));
        if load.clicked() {
            // The selected mask row can coexist with a Composite engine target while mask
            // painting is active. The footer follows the row the user sees as targeted.
            let params =
                if mask_targeted { json!({ "channel": "mask", "layer": masked.as_ref().map(|layer| layer.id.0) }) } else { json!({ "channel": target_ref }) };
            actions.push(("select.loadSelection".into(), params));
        }
        actions.extend(drop(ui, &load, Footer::Load));
    });
    // The drag ends on any release, after the buttons have had a chance to take the drop.
    if ctx.input(|i| i.pointer.any_released()) {
        ctx.data_mut(|d| d.remove::<(Row, String)>(drag_id()));
    }
    ctx.data_mut(|d| d.insert_temp(thumbs_id(), drawn));
    for (id, p) in actions {
        if id == "ui.renameChannel" {
            if let Some(i) = p.as_u64().and_then(|i| doc.channels.get(i as usize)) {
                let row = Row::Alpha(p.as_u64().unwrap_or(0) as usize);
                ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("chan-rename", doc.id.0, format!("{:?}", row.reference()))), i.name.clone()));
            }
            continue;
        }
        if id == "ui.newSpotChannel" {
            crate::channel_options_ui::open_new_spot(app);
            continue;
        }
        if id == "ui.channelOptions" {
            crate::channel_options_ui::open(app, &p);
            continue;
        }
        let _ = app.run(&id, p);
    }
    if let Some(on) = mask_click {
        app.ui.mask_target = on;
        app.sync_mask_targets();
    }
}

/// The footer buttons a row can be dropped on.
#[derive(Clone, Copy)]
enum Footer {
    Delete,
    New,
    Load,
}

/// What dropping `row` (named `name`) on a footer button does, as in Photoshop; `None` where
/// Photoshop does nothing either (colour channels can't be deleted here). `mask_layer` is the
/// layer whose mask the temporary mask row shows.
fn drop_action(row: Row, name: &str, mask_layer: Option<u64>, on: Footer) -> Option<(String, Value)> {
    match (on, row) {
        (Footer::Delete, Row::Alpha(i)) => Some(("channel.delete".into(), json!({ "channel": i }))),
        // Asks "Apply mask to layer before removing?", like dropping the mask's Layers thumbnail.
        (Footer::Delete, Row::LayerMask) => Some(crate::mask_trash_prompt::drop_action(mask_layer?, crate::mask_thumbs_ui::MaskKind::Pixel)),
        (Footer::Delete, _) => None,
        // The engine would name a mask's copy after the composite; Photoshop says "Layer 1 Mask copy".
        (Footer::New, Row::LayerMask) => Some(("channel.duplicate".into(), json!({ "channel": "mask", "name": format!("{name} copy") }))),
        (Footer::New, _) => Some(("channel.duplicate".into(), json!({ "channel": row.reference() }))),
        (Footer::Load, Row::LayerMask) => Some(("select.loadSelection".into(), json!({ "channel": "mask", "layer": mask_layer? }))),
        (Footer::Load, _) => Some(("select.loadSelection".into(), json!({ "channel": row.reference() }))),
    }
}

fn drag_id() -> egui::Id {
    egui::Id::unique("channel-row-drag")
}

fn thumbs_id() -> egui::Id {
    egui::Id::unique("channel-thumb-rects")
}

/// The Channels rows drawn last frame: (name, thumbnail rect).
pub fn recorded(ctx: &egui::Context) -> Vec<(String, Rect)> {
    ctx.data(|d| d.get_temp(thumbs_id())).unwrap_or_default()
}

/// The part of a square `cell` a `w`×`h` document fills, and the matching UVs of a square,
/// letterboxed thumbnail texture: thumbnails keep the document's aspect ratio.
pub fn fit_thumb(cell: Rect, w: u32, h: u32) -> (Rect, Rect) {
    let (w, h) = (w.max(1) as f32, h.max(1) as f32);
    let (fw, fh) = (w / w.max(h), h / w.max(h));
    let size = vec2(cell.width() * fw, cell.height() * fh).max(vec2(1.0, 1.0));
    let uv = Rect::from_center_size(pos2(0.5, 0.5), vec2(fw, fh));
    (Rect::from_center_size(cell.center(), size), uv)
}

fn item(ui: &mut egui::Ui, actions: &mut Vec<(String, Value)>, label: &str, cmd: &str, p: Value) {
    if ui.button(tl!(&label)).clicked() {
        actions.push((cmd.into(), p));
        ui.close();
    }
}

/// Channel Options › Color Indicates.
fn indicates_items(ui: &mut egui::Ui, actions: &mut Vec<(String, Value)>, channel: Value, current: ColorIndicates) {
    let masked = current == ColorIndicates::MaskedAreas;
    let check = |on: bool, s: &str| if on { format!("✓ {s}") } else { s.to_string() };
    item(ui, actions, &check(masked, "Color Indicates Masked Areas"), "channel.options", json!({ "channel": channel.clone(), "indicates": "masked" }));
    item(ui, actions, &check(!masked, "Color Indicates Selected Areas"), "channel.options", json!({ "channel": channel, "indicates": "selected" }));
}

/// Overlay colour presets (Channel Options › Color) and opacity.
fn overlay_colors(ui: &mut egui::Ui, actions: &mut Vec<(String, Value)>, channel: Value) {
    ui.menu_button(tl!("Overlay Color"), |ui| {
        for (label, hex) in [
            (tl!("Red"), "#ff0000"),
            (tl!("Green"), "#00c000"),
            (tl!("Blue"), "#0050ff"),
            (tl!("Cyan"), "#00d0e0"),
            (tl!("Magenta"), "#e000c0"),
            (tl!("Yellow"), "#f0d000"),
        ] {
            if ui.button(tl!(&label)).clicked() {
                actions.push(("channel.options".into(), json!({ "channel": channel.clone(), "color": hex })));
                ui.close();
            }
        }
    });
    ui.menu_button(tl!("Overlay Opacity"), |ui| {
        for o in [25, 50, 75, 100] {
            if ui.button(format!("{o}%")).clicked() {
                actions.push(("channel.options".into(), json!({ "channel": channel.clone(), "opacity": o })));
                ui.close();
            }
        }
    });
}

#[cfg(test)]
mod tests;
