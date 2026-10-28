//! Dialogs rendered from `UiState::dialogs`. Field values live in the dialog data, so automation can
//! set them (`ui.dialog.set`) and confirm (`ui.dialog.confirm`) exactly like a user.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{Dialog, DialogKind};
use crate::widgets::{ButtonRole, DialogButton, dialog_buttons};

/// A dialog body sets this field to the key of the text field whose Browse… button was clicked;
/// [`show`] takes it out again and opens the matching file or folder dialog.
pub(crate) const BROWSE_FIELD: &str = "__browse";

/// A text field that names a folder: an export's `dir`, and the source (`input`) and destination
/// (`output`) folders of Image Processor, Batch, Lens Correction, Contact Sheet and Create
/// Droplet, which Photoshop picks with Select Folder… / Choose… (#9357).
pub(crate) fn is_folder_field(key: &str) -> bool {
    matches!(key, "dir" | "input" | "output")
}

/// Browse… beside a path field of dialog `id`: a folder picker for folder fields
/// ([`is_folder_field`]), a `.wasm` picker for Install Plug-in…'s `path`. The answer fills the field.
fn browse_field(app: &mut PhotocraftApp, id: u64, fields: &serde_json::Map<String, Value>, key: &str) {
    let key = key.to_string();
    let r = if is_folder_field(&key) {
        app.ask_folder(move |app, path| fill_field(app, id, &key, path))
    } else if fields.get("__command").and_then(Value::as_str) == Some("plugin.install") {
        let initial_dir = crate::file_dialog::last_used_dir(&app.ui.recent_files);
        let request = crate::file_dialog::FileDialogRequest::Open { multiple: false, initial_dir, extensions: Some(vec!["wasm".into()]) };
        app.ask_file(request, move |app, answer| match answer {
            crate::file_dialog::FileDialogAnswer::Paths(mut paths) if !paths.is_empty() => fill_field(app, id, &key, paths.remove(0)),
            _ => Err("Install Plug-in reads a .wasm file from disk".into()),
        })
    } else {
        return;
    };
    if let Err(e) = r.map(|_| ()).or_else(|e| if e == crate::file_dialog::CANCELLED { Ok(()) } else { Err(e) }) {
        app.ui.status = e;
    }
}

fn fill_field(app: &mut PhotocraftApp, id: u64, key: &str, path: String) -> Result<Value, String> {
    let d = app.ui.dialog_mut(id).ok_or("the dialog was closed")?;
    d.fields.insert(key.into(), json!(path));
    Ok(Value::Null)
}

/// Where this frame's dialogs are on screen (the canvas reads last frame's: it draws first).
const RECTS: &str = "pc-dialog-rects";

fn rects(ctx: &egui::Context) -> Vec<egui::Rect> {
    ctx.data(|m| m.get_temp(egui::Id::unique(RECTS))).unwrap_or_default()
}

/// The window drag target must not cover a dialog which has been moved into the title bar.
pub(crate) fn titlebar_drag_area_is_clear(ctx: &egui::Context, area: egui::Rect) -> bool {
    rects(ctx).iter().all(|dialog| !dialog.intersects(area))
}

/// Whether the press that is being held began on a dialog (#6810). The dialog follows the pointer
/// while its title is dragged, so the press origin ends up outside the (moved) dialog and a canvas
/// check against the dialog rects alone would take the drag for a canvas drag. Remembered from the
/// frame of the press, which is the only frame that carries the press event.
fn press_began_on_dialog(ctx: &egui::Context, rects: &[egui::Rect]) -> bool {
    let key = egui::Id::unique("pc-dialog-press-grabbed");
    let pressed = ctx.input(|i| {
        i.events.iter().rev().find_map(|e| match e {
            egui::Event::PointerButton { pos, pressed: true, .. } => Some(*pos),
            _ => None,
        })
    });
    if let Some(p) = pressed {
        let on_dialog = rects.iter().any(|r| r.contains(p));
        ctx.data_mut(|m| m.insert_temp(key, on_dialog));
        return on_dialog;
    }
    ctx.data(|m| m.get_temp::<bool>(key)).unwrap_or(false)
}

/// With a dialog open the rest of the window is inert (egui's modal layer), but like Photoshop the
/// image can still be panned and zoomed under it. The pointer position when it is over `canvas`
/// and not over a dialog or one of its popups.
pub fn free_pointer_over(ctx: &egui::Context, canvas: egui::Rect) -> Option<egui::Pos2> {
    if egui::Popup::is_any_open(ctx) {
        return None;
    }
    let p = ctx.pointer_hover_pos()?;
    let rects = rects(ctx);
    (canvas.contains(p) && !rects.iter().any(|r| r.contains(p))).then_some(p)
}

/// Pan drag under an open dialog: Space-drag, middle-drag, or a drag with the Hand tool, started on
/// the free canvas. Returns this frame's pointer movement.
pub fn pan_delta(ctx: &egui::Context, canvas: egui::Rect, hand: bool) -> Option<egui::Vec2> {
    let rects = rects(ctx);
    let grabbed = press_began_on_dialog(ctx, &rects);
    let (origin, panning, delta) = ctx.input(|i| {
        let p = &i.pointer;
        (p.press_origin(), p.middle_down() || (p.primary_down() && (hand || i.key_down(egui::Key::Space))), p.delta())
    });
    let origin = origin?;
    (panning && !grabbed && canvas.contains(origin) && !rects.iter().any(|r| r.contains(origin))).then_some(delta)
}

/// The primary button's drag and click on the free canvas under an open dialog, as the canvas
/// response would report them if it weren't inert: for the held Zoom (⌘Space / ⌥Space, #6460).
/// A drag counts from egui's drag threshold, like `drag_started`.
pub fn free_buttons(ctx: &egui::Context, canvas: egui::Rect) -> crate::paint_mouse::Buttons {
    let id = egui::Id::unique("pc-dialog-free-drag");
    let was = ctx.data(|d| d.get_temp::<bool>(id)).unwrap_or(false);
    let rects = rects(ctx);
    let open_popup = egui::Popup::is_any_open(ctx);
    let free = |p: egui::Pos2| !open_popup && canvas.contains(p) && !rects.iter().any(|r| r.contains(p));
    let (down, dragging, clicked) = ctx.input(|i| {
        let p = &i.pointer;
        let dragging = p.primary_down() && p.is_decidedly_dragging() && (was || p.press_origin().is_some_and(free));
        (p.primary_down(), dragging, p.primary_clicked() && p.interact_pos().is_some_and(free))
    });
    ctx.data_mut(|d| d.insert_temp(id, dragging));
    crate::paint_mouse::Buttons { started: dragging && !was, dragged: dragging, stopped: was && !down, clicked }
}

/// A click or drag with the primary button started on the free canvas under an open dialog, Space
/// not held: where the pointer is this frame. The press itself counts even when it is released in
/// the same frame (a quick click, `ui.click`); the drag only while it stays on the free canvas.
pub fn free_press(ctx: &egui::Context, canvas: egui::Rect) -> Option<egui::Pos2> {
    if egui::Popup::is_any_open(ctx) {
        return None;
    }
    let rects = rects(ctx);
    let free = |p: egui::Pos2| canvas.contains(p) && !rects.iter().any(|r| r.contains(p));
    let grabbed = press_began_on_dialog(ctx, &rects);
    ctx.input(|i| {
        if i.key_down(egui::Key::Space) {
            return None;
        }
        let pressed = i.events.iter().rev().find_map(|e| match e {
            egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => Some(*pos),
            _ => None,
        });
        if let Some(p) = pressed {
            return free(p).then_some(p);
        }
        let held = i.pointer.primary_down() && !grabbed && i.pointer.press_origin().is_some_and(free);
        i.pointer.latest_pos().filter(|p| held && free(*p))
    })
}

/// Height of a dialog's title, rule, button row and frame margins (points), around a body that
/// scrolls.
const DIALOG_CHROME: f32 = 150.0;

/// Draw a dialog body that opens on its first number, selected, as Photoshop's value dialogs do:
/// typing replaces it and Enter applies it (#1757). Once per dialog `id`, on the first laid-out
/// frame.
fn first_field_focused<R>(ui: &mut egui::Ui, id: egui::Id, body: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let ctx = ui.ctx().clone();
    let focused = id.with("first-field");
    let first = !ui.is_sizing_pass() && !ctx.data(|m| m.get_temp::<bool>(focused).unwrap_or(false));
    if first {
        ctx.data_mut(|m| m.insert_temp(focused, true));
    }
    crate::widgets::focus_first_field(&ctx, first);
    let r = body(ui);
    crate::widgets::focus_first_field(&ctx, false);
    r
}

/// Where a dialog's last position is remembered: per command for command dialogs (each filter has
/// its own), per kind for the rest.
fn place_id(d: &Dialog) -> egui::Id {
    match d.fields.get("__command").and_then(Value::as_str) {
        Some(cmd) => egui::Id::unique(("dialog-place", cmd)),
        None => egui::Id::unique(("dialog-place", format!("{:?}", d.kind))),
    }
}

/// Whether another dialog is drawn above `live` (egui's layer order, back to front).
fn older_dialog_above(ctx: &egui::Context, live: egui::LayerId, dialogs: &[Dialog]) -> bool {
    let others: Vec<egui::LayerId> =
        dialogs.iter().map(|d| egui::LayerId::new(egui::Order::Foreground, egui::Id::unique(("dialog", d.id)))).filter(|l| *l != live).collect();
    ctx.memory(|m| m.layer_ids().skip_while(|l| *l != live).skip(1).any(|l| others.contains(&l)))
}

/// The size a resizable dialog opens at, or `None` for a dialog sized by its contents.
fn resizable_dialog(fields: &serde_json::Map<String, Value>) -> Option<egui::Vec2> {
    if crate::prefs_ui::is_preferences(fields) {
        Some(crate::prefs_ui::PREFERENCES_SIZE)
    } else if fields.contains_key("__web") {
        Some(egui::vec2(960.0, 900.0))
    } else if is_curves(fields) {
        Some(CURVES_SIZE)
    } else {
        None
    }
}

/// Whether `fields` are the Image › Adjustments › Curves dialog's.
fn is_curves(fields: &serde_json::Map<String, Value>) -> bool {
    fields.get("__adjust").and_then(Value::as_str) == Some("curves")
}

/// The size the Curves dialog first opens at: the 300-point graph it had before it could be
/// resized (#5113); it grows to fit its controls below the graph.
const CURVES_SIZE: egui::Vec2 = egui::vec2(300.0, 420.0);
/// The smallest a resizable dialog is dragged to: Curves keeps a usable graph (#8984), the others
/// their two-column layouts.
fn resizable_min(fields: &serde_json::Map<String, Value>) -> egui::Vec2 {
    if is_curves(fields) { egui::vec2(300.0, 420.0) } else { egui::vec2(560.0, 400.0) }
}

/// Where the size the user last gave a resizable dialog is kept (#8984), beside its place.
fn size_id(place: egui::Id) -> egui::Id {
    place.with("size")
}

/// The largest a resizable dialog of `fields` with smallest size `min` is dragged to: the window
/// less a margin, and for Curves no wider than its graph can grow (#8984), so no blank column.
fn resizable_max(ctx: &egui::Context, fields: &serde_json::Map<String, Value>, min: egui::Vec2) -> egui::Vec2 {
    let mut max = (ctx.content_rect().size() - egui::vec2(40.0, 40.0)).max(min);
    if is_curves(fields) {
        max.x = max.x.min(crate::adjust_editors::curve_side_max(ctx)).max(min.x);
    }
    max
}

fn show_modal<T>(
    modal: egui::Modal,
    ctx: &egui::Context,
    id: egui::Id,
    resizable: Option<(egui::Vec2, egui::Vec2, egui::Vec2)>,
    interactive: bool,
    contents: impl FnOnce(&mut egui::Ui) -> T,
) -> egui::ModalResponse<T> {
    modal.show(ctx, |ui| {
        // Disable the entire older modal, including its resize grip, not just its body.
        if !interactive {
            ui.disable();
        }
        if let Some((size, min, max)) = resizable {
            // No stroke: Resize outlines its contents 2 points out, an inner box inside the
            // dialog's frame that left the title and buttons flush against it (#8199).
            egui::Resize::default().id(id.with("resize")).with_stroke(false).default_size(size.min(max)).min_size(min).max_size(max).show(ui, contents)
        } else {
            contents(ui)
        }
    })
}

/// Whether dialogs have a close box (×) in their title bar (#5496). Photoshop's modal dialogs have
/// one on Windows, where clicking it is Cancel, and none on macOS; Linux and the web follow Windows.
pub(crate) const CLOSE_BOX: bool = !cfg!(target_os = "macos");

/// The side of the close box's square, in points.
const CLOSE_BOX_SIZE: f32 = 20.0;

/// The close box's square: flush with the title band's right end `right`, centred on the title
/// row at height `centre_y`.
fn close_box_rect(right: f32, centre_y: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(right - CLOSE_BOX_SIZE, centre_y - CLOSE_BOX_SIZE / 2.0), egui::vec2(CLOSE_BOX_SIZE, CLOSE_BOX_SIZE))
}

/// Draws a title bar's close box in `rect`, the same × icon button the floating panels close with,
/// and says whether it was clicked. Its own child `Ui`, so the title row's layout is unchanged.
pub(crate) fn close_box(ui: &mut egui::Ui, rect: egui::Rect) -> bool {
    let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rect).layout(egui::Layout::right_to_left(egui::Align::Center)));
    crate::icons::button(&mut child, "x", CLOSE_BOX_SIZE, false, tl!("Close")).clicked()
}

/// The dialog's contents under the title rule, picked by its kind and the marker field that routes
/// it. Returns `Some(true)` / `Some(false)` when the body itself answered OK / Cancel.
/// The body of a command dialog: the first handler whose marker the fields carry. The prompts and
/// small editors come first; the rest are in `command_body_more`.
fn command_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, d: &Dialog, fields: &mut Fields, id: egui::Id, interactive: bool) -> Option<bool> {
    let mut outcome: Option<bool> = None;
    match d.kind {
        _ if crate::open_prompt::owns_damage(fields) => crate::open_prompt::damage_body(ui, fields),
        _ if crate::guide_layout_ui::owns(fields) => outcome = crate::guide_layout_ui::body(app, ui, fields, interactive),
        _ if crate::brush_preset_dialog::owns(fields) => outcome = crate::brush_preset_dialog::body(app, ui, fields, interactive),
        _ if crate::fill_ui::owns(fields) => crate::fill_ui::body(app, ui, fields),
        _ if crate::pattern_fill_ui::owns(fields) => crate::pattern_fill_ui::body(app, ui, fields),
        _ if crate::stroke_ui::owns(fields) => crate::stroke_ui::body(ui, fields),
        _ if crate::new_group_ui::owns(fields) => crate::new_group_ui::body(ui, fields),
        _ if crate::snapshot_dialog::owns(fields) => crate::snapshot_dialog::body(ui, fields),
        _ if crate::channel_options_ui::owns(fields) => crate::channel_options_ui::body(ui, fields),
        _ if crate::channel_options_ui::owns_new_spot(fields) => crate::channel_options_ui::new_spot_body(ui, fields),
        _ if crate::shape_dialog::owns(fields) => crate::shape_dialog::body(app, ui, fields),
        _ if crate::delete_layer_prompt::owns(fields) => crate::delete_layer_prompt::body(ui, fields),
        _ if crate::disk_change_prompt::owns(fields) => crate::disk_change_prompt::body(ui, fields),
        _ if crate::replace_prompt::owns(fields) => crate::replace_prompt::body(ui, fields),
        _ if crate::actions::owns_insert_stop(fields) => crate::actions::insert_stop_body(ui, fields),
        _ if crate::new_layer_ui::owns(fields) => crate::new_layer_ui::body(ui, fields),
        _ if crate::mask_trash_prompt::owns(fields) => crate::mask_trash_prompt::body(ui, fields),
        _ if crate::rasterize_prompt::owns(fields) => crate::rasterize_prompt::body(ui, fields),
        _ if crate::duplicate_layer_ui::owns(fields) => crate::duplicate_layer_ui::body(app, ui, fields),
        _ if crate::stack_dialog::owns(fields) => crate::stack_dialog::body(app, ui, fields, d.id),
        _ if crate::tool_reset::owns(fields) => crate::tool_reset::body(ui),
        _ if crate::crop_presets::owns(fields) => crate::crop_presets::body(app, ui, fields),
        _ if crate::variables_ui::owns(fields) => crate::variables_ui::body(app, ui, fields),
        _ => return command_body_more(app, ui, d, fields, id),
    }
    outcome
}

/// The rest of the command dialogs: files, colour, export, sizing, filters and forms.
fn command_body_more(app: &mut PhotocraftApp, ui: &mut egui::Ui, d: &Dialog, fields: &mut Fields, id: egui::Id) -> Option<bool> {
    let mut outcome: Option<bool> = None;
    match d.kind {
        _ if crate::file_ui::owns(fields) => crate::file_ui::body(app, ui, fields, d.id),
        _ if crate::color_picker_ui::owns(fields) => {
            outcome = crate::color_picker_ui::body(ui, fields);
            crate::color_picker_ui::take_add_swatch(app, fields);
            if let Some(web) = fields.get("__webOnly").and_then(Value::as_bool) {
                app.ui.color_panel.web_only = web;
            }
        }
        _ if crate::color_range_ui::owns(fields) => crate::color_range_ui::body(app, ui, fields),
        _ if crate::prefs_ui::owns(fields) => crate::prefs_ui::body(app, ui, fields),
        _ if fields.contains_key("__export") => crate::export_dialog::body(app, ui, fields),
        // Image Size and Canvas Size open on Width, selected (#2329).
        _ if fields.contains_key("__sizing") => first_field_focused(ui, id, |ui| crate::sizing::body(ui, fields)),
        _ if crate::smart_filter_dialog::owns(fields) => crate::smart_filter_dialog::body(app, ui, fields),
        _ if crate::adjust_dialog::owns(fields) => crate::adjust_dialog::body(app, ui, fields),
        _ if crate::blur_pins::owns(fields) => crate::blur_pins::body(ui, fields),
        _ if fields.contains_key("__filter") => filter_body(app, ui, d, fields, id),
        _ if crate::new_guide_ui::owns(fields) => crate::new_guide_ui::body(ui, fields),
        _ if fields.contains_key("__form") => crate::view_cmds::form_body(ui, fields),
        _ => {}
    }
    outcome
}

fn dialog_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, d: &Dialog, fields: &mut Fields, id: egui::Id, interactive: bool) -> Option<bool> {
    let mut outcome: Option<bool> = None;
    match d.kind {
        DialogKind::NewDocument => {
            if crate::new_doc_ui::body(app, ui, fields) {
                outcome = Some(true);
            }
        }
        DialogKind::About if fields.get("systemInfo").and_then(Value::as_bool) == Some(true) => {
            let lines = crate::gpu_status::system_info_ui(app);
            for l in &lines {
                ui.add(egui::Label::new(egui::RichText::new(l).font(crate::theme::mono(12.0))).selectable(true));
            }
            ui.add_space(8.0);
            if crate::widgets::secondary_button(ui, tl!("Copy"), 84.0).clicked() {
                ui.ctx().copy_text(crate::gpu_status::system_info(app).join("\n"));
            }
        }
        DialogKind::About => {
            // Tabs About · Contributors · Models (craftrules standards/contributors.md). The
            // tab is a dialog field, so automation can switch it with `ui.dialog.set`.
            let tab = about_tab(fields);
            let mut chosen = tab;
            ui.horizontal(|ui| {
                for (key, label) in [("about", tl!("About")), ("contributors", tl!("Contributors")), ("models", tl!("Models"))] {
                    if crate::widgets::pill_tab(ui, label, tab == key).clicked() {
                        chosen = key;
                    }
                }
            });
            if chosen != tab {
                fields.insert("tab".into(), json!(chosen));
            }
            ui.add_space(8.0);
            about_tab_body(app, ui, chosen);
        }
        DialogKind::Command => outcome = command_body(app, ui, d, fields, id, interactive),
        DialogKind::LayerStyle => crate::layer_style::body(app, ui, fields),
        DialogKind::Error => {
            ui.label(fields.get("message").and_then(Value::as_str).unwrap_or("Error"));
        }
    }
    outcome
}

/// A filter-style dialog: scrolling parameters with the in-dialog preview window under them.
fn filter_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, d: &Dialog, fields: &mut Fields, id: egui::Id) {
    // Color Settings: the monitor profile in use can change while it is open.
    if fields.get("__command").and_then(Value::as_str) == Some("edit.colorSettings") {
        fields.insert("__note".into(), Value::String(crate::monitor_status::note(app)));
    }
    // Long parameter lists (Flame, Lighting Effects) scroll, so the title and the
    // OK / Cancel buttons stay inside a small window. The preview window below the
    // parameters reserves its own space (#3769).
    let room = (ui.ctx().content_rect().height() - DIALOG_CHROME - crate::filter_preview_ui::reserved_height(fields)).max(120.0);
    let body = |ui: &mut egui::Ui, fields: &mut serde_json::Map<String, Value>| {
        egui::ScrollArea::vertical().id_salt(id.with("body")).max_height(room).show(ui, |ui| {
            if crate::bitmap_dialog::owns(fields) {
                crate::bitmap_dialog::body(app, ui, fields);
            } else {
                crate::filter_dialog::body(ui, fields);
            }
        });
        // The in-dialog preview window sits under the scrolling parameters (#3769),
        // so it never shifts them.
        crate::filter_preview_ui::window(app, d.id, ui, fields);
    };
    // Select and Mask leaves the keyboard to its view keys (F, X, O, V…).
    if crate::select_mask_view::owns(fields) {
        body(ui, fields);
    } else {
        first_field_focused(ui, id, |ui| body(ui, fields));
    }
}

/// The OK / Cancel row (with Apply, Alternate or Reset where the dialog has them), drawn in the
/// right-to-left layout. Returns what the buttons (or Enter) answered and whether Apply was hit.
/// `popup_blocks_enter` is true when a web dialog has a popup open, which keeps Enter for the popup.
fn button_row(
    app: &mut PhotocraftApp,
    ui: &mut egui::Ui,
    d: &Dialog,
    fields: &mut Fields,
    id: egui::Id,
    interactive: bool,
    popup_blocks_enter: bool,
) -> (Option<bool>, bool) {
    let mut outcome: Option<bool> = None;
    let mut apply_requested = false;
    if matches!(d.kind, DialogKind::About | DialogKind::Error) {
        // ↩ is OK too, as on every other dialog (#4872), from the frame after the
        // alert appeared: the ↩ that raised it (committing a locked layer's Free
        // Transform, say) must not close it again at once.
        let seen = id.with("alert-seen");
        let shown_before = ui.ctx().data(|m| m.get_temp::<bool>(seen).unwrap_or(false));
        ui.ctx().data_mut(|m| m.insert_temp(seen, true));
        let enter = interactive && shown_before && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if dialog_buttons(ui, &[DialogButton::new(ButtonRole::Default, tl!("OK"), 84.0)]).is_some() || enter {
            outcome = Some(false);
        }
    } else {
        let ok_label = ok_label(d);
        // Dialogs that need an input first (a preset name, a selection) grey OK out.
        let can_ok = crate::crop_presets::can_confirm(app, fields) && crate::stack_dialog::can_confirm(fields);
        let ok = DialogButton::new(ButtonRole::Default, ok_label, 84.0).enabled(can_ok);
        // Photoshop: holding Alt turns Cancel into Reset (the dialog stays open).
        // Dialogs opt in via `dialog_alt_reset::supported` (per marker).
        let reset = ui.input(|i| i.modifiers.alt) && crate::dialog_alt_reset::supported(fields);
        let cancel_label = if reset {
            tl!("Reset")
        } else if d.kind == DialogKind::NewDocument {
            tl!("Close")
        } else {
            tl!("Cancel")
        };
        let cancel = DialogButton::new(ButtonRole::Cancel, cancel_label, 84.0);
        let clicked = if d.kind == DialogKind::Command && crate::prefs_ui::is_preferences(fields) {
            let changed = crate::prefs_ui::preferences_changed(app, fields);
            dialog_buttons(ui, &[ok, cancel, DialogButton::new(ButtonRole::Apply, tl!("Apply"), 84.0).enabled(changed)])
        } else if crate::mask_trash_prompt::owns(fields) && !crate::mask_trash_prompt::is_vector(fields) {
            // Photoshop: Apply / Cancel / Delete. Delete answers with the choice set.
            let apply = DialogButton::new(ButtonRole::Default, tl!("Apply"), 84.0).enabled(crate::mask_trash_prompt::can_apply(fields));
            let delete = DialogButton::new(ButtonRole::Alternate, tl!("Delete"), 84.0);
            let hit = dialog_buttons(ui, &[apply, cancel, delete]);
            if hit == Some(ButtonRole::Alternate) {
                fields.insert("choice".into(), json!("delete"));
            } else if hit == Some(ButtonRole::Default) {
                fields.insert("choice".into(), json!("apply"));
            }
            hit
        } else if crate::rasterize_prompt::is_filter(fields) {
            // Photoshop: Convert to Smart Object / Rasterize / Cancel (#4303).
            let rasterize = DialogButton::new(ButtonRole::Default, tl!("Rasterize"), 84.0);
            let hit = if crate::rasterize_prompt::offers_smart_object(fields) {
                let convert = DialogButton::new(ButtonRole::Alternate, tl!("Convert to Smart Object"), 160.0);
                dialog_buttons(ui, &[rasterize, cancel, convert])
            } else {
                dialog_buttons(ui, &[rasterize, cancel])
            };
            if hit == Some(ButtonRole::Alternate) {
                fields.insert("choice".into(), json!("smartObject"));
            } else if hit == Some(ButtonRole::Default) {
                fields.insert("choice".into(), json!("rasterize"));
            }
            hit
        } else {
            dialog_buttons(ui, &[ok, cancel])
        };
        match clicked {
            Some(ButtonRole::Cancel) if reset => crate::dialog_alt_reset::apply(app, ui.ctx(), fields),
            Some(ButtonRole::Cancel) => outcome = Some(false),
            Some(ButtonRole::Apply) => apply_requested = true,
            Some(_) => outcome = Some(true),
            None if interactive && can_ok && !popup_blocks_enter && ui.input(|i| i.key_pressed(egui::Key::Enter)) => outcome = Some(true),
            None => {}
        }
    }
    (outcome, apply_requested)
}

/// The default button's label: what OK means for this dialog.
fn ok_label(d: &Dialog) -> &'static str {
    if d.kind == DialogKind::NewDocument {
        tl!("Create")
    } else if crate::open_prompt::owns_damage(&d.fields) {
        tl!("Continue")
    } else if crate::delete_layer_prompt::owns(&d.fields) {
        tl!("Delete")
    } else if crate::disk_change_prompt::owns(&d.fields) || crate::replace_prompt::owns(&d.fields) {
        tl!("Replace")
    } else if d.fields.contains_key("__export") {
        tl!("Export")
    } else {
        crate::file_ui::ok_label(&d.fields).unwrap_or(tl!("OK"))
    }
}

/// The widest a dialog grows when its body names no width of its own.
fn default_max_width(d: &Dialog, compact: bool, curves: bool, about_tabs: bool, available: f32) -> f32 {
    if d.kind == DialogKind::NewDocument {
        800.0
    } else if about_tabs {
        700.0
    } else if d.kind == DialogKind::LayerStyle || d.fields.contains_key("__export") || crate::color_picker_ui::owns(&d.fields) {
        600.0
    } else if compact {
        260.0
    } else if curves {
        available.max(300.0)
    } else {
        440.0
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    crate::layer_style::color_picker::prune(app);
    crate::type_panels_ui::color_picker::prune(app);
    crate::edit_history::track_dialogs(app, ctx);
    let dialogs = app.ui.dialogs.clone();
    let top = dialogs.last().map(|d| d.id);
    let mut shown = Vec::new();
    for d in dialogs {
        let lang = if crate::prefs_ui::is_preferences(&d.fields) {
            crate::i18n::Lang::from_pref(d.fields.get("values").and_then(|v| v.pointer("/interface/language")).and_then(Value::as_str).unwrap_or("auto"))
        } else {
            crate::i18n::current()
        };
        let _language = crate::i18n::language_scope(lang);
        let interactive = top == Some(d.id);
        let mut fields = d.fields.clone();
        let mut outcome: Option<bool> = None; // Some(true)=OK, Some(false)=Cancel
        let mut apply_requested = false;
        let title = display_title(&d);
        let id = egui::Id::unique(("dialog", d.id));
        // Opens where the same dialog was last left (centred the first time), then its top-left
        // stays put (offset from the window's top-left, moved by dragging the title bar; view
        // state only, so egui memory): a dialog whose body grows, like Layer Style switching
        // effects, extends down and right instead of re-centring.
        let pinned: Option<egui::Vec2> = ctx.data(|m| m.get_temp(id));
        let place = place_id(&d);
        let start = pinned.or_else(|| ctx.data_mut(|m| m.get_persisted::<egui::Vec2>(place))).or_else(|| crate::blur_pins::first_place(&d.fields, ctx));
        let mut drag = egui::Vec2::ZERO;
        let mut close_clicked = false;
        let mut sizing = false;
        let area = match start {
            Some(offset) => egui::Modal::default_area(id).anchor(egui::Align2::LEFT_TOP, offset),
            None => egui::Modal::default_area(id),
        };
        // Only the last dialog takes input, and egui only lets the modal drawn on top take input,
        // so the last one must be drawn above the others. egui can raise an older one (it raises an
        // area that is pressed or wasn't drawn the frame before), which left a greyed-out dialog
        // in front and the live one unreachable behind it. Raise the live one only then: every
        // frame would also cover its own popups.
        if interactive && older_dialog_above(ctx, area.layer(), &app.ui.dialogs) {
            ctx.move_to_top(area.layer());
        }
        let color_picker = crate::color_picker_ui::owns(&fields);
        let mut modal = egui::Modal::new(id).area(area).backdrop_color(egui::Color32::TRANSPARENT);
        if color_picker {
            modal = modal.frame(egui::Frame::popup(&ctx.global_style()).inner_margin(16));
        }
        // Tab / ⇧Tab walk the topmost dialog's text fields, not every widget (field_tab.rs).
        let tab = if top == Some(d.id) && !egui::Popup::is_any_open(ctx) { crate::field_tab::take_step(ctx) } else { 0 };
        // Photoshop doesn't dim the window behind dialogs: previews must be judged at true contrast.
        let web = fields.contains_key("__web");
        let web_popup_open = web && egui::Popup::is_any_open(ctx);
        let resizable = resizable_dialog(&fields);
        // Curves reopens at the size it was left at (#8984).
        let remembered = if is_curves(&fields) { ctx.data_mut(|m| m.get_persisted::<egui::Vec2>(size_id(place))) } else { None };
        let resize = resizable.map(|size| {
            let min = resizable_min(&fields);
            (remembered.unwrap_or(size), min, resizable_max(ctx, &fields, min))
        });
        let mut given = None;
        let modal = show_modal(modal, ctx, id, resize, interactive, |ui| {
            sizing = ui.is_sizing_pass();
            let available = ui.available_size();
            given = Some(available);
            // A dialog of a few short fields (`__compact`, Fit Image) is narrower (#4673): the title
            // rule and the button row fill the width they are given, so it needs its own maximum too.
            let compact = d.fields.contains_key("__compact");
            // The curve editor's graph is as wide as the dialog lets it be (#5113, #8984).
            let curves = is_curves(&fields);
            if !compact {
                ui.set_min_width(if curves { 300.0 } else { 380.0 });
            }
            // A resizable dialog's contents fill its resizable area (#7420), the others the window.
            let room = if resizable.is_some() { available.x } else { (ctx.content_rect().width() - 48.0).max(380.0) };
            let wide = crate::prefs_ui::width(&d.fields, room);
            if let Some(w) = wide {
                ui.set_min_width(w.min(460.0));
            }
            if d.kind == DialogKind::NewDocument {
                ui.set_min_width(800.0);
            }
            // The About window: room for the contributor table, the same width on every tab.
            let about_tabs = d.kind == DialogKind::About && d.fields.get("systemInfo").and_then(Value::as_bool) != Some(true);
            if about_tabs {
                ui.set_min_width(700.0);
            }
            ui.set_max_width(wide.unwrap_or_else(|| default_max_width(&d, compact, curves, about_tabs, available.x)));
            if web {
                ui.set_max_width(available.x);
                ui.set_min_size(available);
            } else if let Some(w) = crate::file_ui::dialog_width(&d.fields) {
                ui.set_min_width(w);
                ui.set_max_width(w);
            }
            // Duplicate Layer… is a fixed small grid (labels + 260 pt fields ≈ 352 pt):
            // the generic 380 pt minimum left a dead strip on its right.
            if crate::duplicate_layer_ui::owns(&d.fields) {
                ui.set_min_width(360.0);
                ui.set_max_width(360.0);
            }
            if color_picker {
                ui.set_width(crate::color_picker_ui::CONTENT_WIDTH);
            }
            let label = egui::Label::new(egui::RichText::new(&title).font(crate::theme::semibold(15.0))).selectable(false);
            let t = if crate::color_picker_ui::owns(&fields) { ui.add_sized(egui::vec2(ui.available_width(), 22.0), label).rect } else { ui.add(label).rect };
            // The whole title band drags (#1921): the full width plus a little of the popup's
            // padding around it and the gap above the hairline, but none of the controls below.
            // The close box (#5496) sits at the band's right end and is left out of the drag area:
            // egui would let a drag that starts on a button move the dialog.
            let close_rect = CLOSE_BOX.then(|| close_box_rect(ui.max_rect().right(), t.center().y));
            let bar_right = close_rect.map_or(ui.max_rect().right() + 7.0, |r| r.left());
            let bar = egui::Rect::from_min_max(t.min - egui::vec2(7.0, 7.0), egui::pos2(bar_right, t.bottom() + 4.0));
            drag = ui.interact(bar, id.with("title"), egui::Sense::drag()).drag_delta();
            if let Some(r) = close_rect {
                close_clicked = close_box(ui, r);
            }
            ui.add_space(4.0);
            crate::widgets::hairline(ui);
            ui.add_space(8.0);
            crate::field_tab::begin(ui.ctx());
            if let Some(done) = dialog_body(app, ui, &d, &mut fields, id, interactive) {
                outcome = Some(done);
            }
            crate::field_tab::end(ui.ctx(), tab);
            if let Some(Value::String(key)) = fields.remove(BROWSE_FIELD) {
                browse_field(app, d.id, &fields, &key);
            }
            if crate::guide_layout_ui::owns(&fields) || crate::brush_preset_dialog::owns(&fields) {
                // These dialogs own their right-side actions and validate Enter themselves.
            } else if crate::color_picker_ui::owns(&fields) {
                if interactive && outcome.is_none() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                    outcome = Some(true);
                }
            } else {
                // The Color Table dialog loads and saves `.act` files, as in Photoshop.
                if matches!(d.kind, DialogKind::Command) && crate::color_table_ui::owns(&fields) {
                    crate::color_table_ui::buttons(app, ui, d.id, &mut fields);
                }
                ui.add_space(8.0);
                // Align::Min, not Center: a centred row fills the height left over from last frame's
                // (larger) size, so a dialog whose body gets shorter would never shrink back.
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Min), |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    let (done, apply) = button_row(app, ui, &d, &mut fields, id, interactive, web && (web_popup_open || egui::Popup::is_any_open(ui.ctx())));
                    outcome = done.or(outcome);
                    apply_requested |= apply;
                });
            }
            // The frame around the content (Frame::popup's margin and stroke).
            ui.min_rect().expand(ui.spacing().menu_margin.sum().max_elem() + 2.0)
        });
        // The picker has a larger frame margin; include that whole frame in the canvas hit guard.
        shown.push(if color_picker { modal.response.rect } else { modal.inner });
        // The size Curves was given (dragged or grown to fit) is the one it reopens at (#8984).
        // A size at the window's limit is the window's, not the user's: a large remembered size
        // squeezed into a small window is kept for a larger one.
        if let (false, true, Some(size), Some((opened, _, max))) = (sizing, is_curves(&fields), given.filter(|g| g.is_finite()), resize)
            && size != opened.min(max)
            && size.x < max.x - 0.5
            && size.y < max.y - 0.5
            && remembered != Some(size)
        {
            ctx.data_mut(|m| m.insert_persisted(size_id(place), size));
        }
        // Pin once laid out at its real size (the first frame is an invisible sizing pass).
        if !sizing && (web || pinned.is_none() || drag != egui::Vec2::ZERO) {
            let screen = ctx.content_rect();
            // Keep the whole dialog (and so its title bar) on screen.
            let room = (screen.size() - modal.response.rect.size()).max(egui::Vec2::ZERO);
            let offset = (pinned.unwrap_or(modal.response.rect.min - screen.min) + drag).clamp(egui::Vec2::ZERO, room);
            ctx.data_mut(|m| m.insert_temp(id, offset));
            // A dialog the user moved reopens there (Photoshop remembers each dialog's place).
            if drag != egui::Vec2::ZERO {
                ctx.data_mut(|m| m.insert_persisted(place, offset));
            }
        }
        // Esc and the title bar's close box cancel (topmost dialog, no popup open for Esc). A click
        // outside does nothing: Photoshop keeps the dialog, and the pointer may be panning or
        // zooming the canvas under it.
        if interactive
            && outcome.is_none()
            && (close_clicked
                || modal.response.should_close()
                || (modal.is_top_modal && !modal.any_popup_open && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))))
        {
            outcome = Some(false);
        }
        if let Some(dm) = app.ui.dialog_mut(d.id) {
            dm.fields = fields;
        }
        // A colour swatch (Edit › Fill, Edit › Stroke) opens the Color Picker over the dialog.
        if outcome.is_none() {
            crate::color_picker_ui::open_requested(app, d.id);
        }
        if apply_requested && outcome.is_none() {
            let _ = crate::prefs_ui::apply(app, d.id);
        }
        match outcome {
            Some(true) => {
                if let Err(error) = confirm(app, d.id) {
                    app.ui.status = error;
                }
            }
            Some(false) => {
                let _ = cancel(app, d.id);
            }
            None => {
                if let Err(error) = crate::layer_style::color_picker::take_request(app, d.id) {
                    app.ui.status = error;
                }
            }
        }
    }
    ctx.data_mut(|m| m.insert_temp(egui::Id::unique(RECTS), shown));
}

/// The About window's tabs, as stored in its `tab` field.
pub const ABOUT_TABS: [&str; 3] = ["about", "contributors", "models"];

/// The About tab to show: the `tab` field when it names one, otherwise "about".
fn about_tab(fields: &serde_json::Map<String, Value>) -> &'static str {
    let want = fields.get("tab").and_then(Value::as_str).unwrap_or("");
    ABOUT_TABS.iter().copied().find(|t| *t == want).unwrap_or("about")
}

/// One About tab's contents: the credits lists, or the product blurb and links.
fn about_tab_body(app: &mut PhotocraftApp, ui: &mut egui::Ui, tab: &str) {
    match tab {
        "contributors" => crate::credits::contributors_ui(app, ui),
        "models" => crate::credits::models_ui(ui),
        _ => {
            ui.label(tl!("PhotoCraft — an open-source, native image editor written in Rust."));
            ui.label(crate::i18n::fmt(tl!("Version {version}"), &[("version", &photocraft_engine::build_info::long_version())]));
            ui.add_space(12.0);
            ui.vertical_centered(|ui| {
                crate::links::discord_button(app, ui, 220.0);
                ui.add_space(8.0);
                crate::links::link_row(app, ui);
            });
            ui.add_space(10.0);
            ui.weak("egui · wgpu · photocraft-engine");
        }
    }
}

/// The dialog title as shown: [`title`] in the UI language.
fn display_title(d: &Dialog) -> String {
    if crate::smart_filter_dialog::owns(&d.fields) {
        return crate::smart_filter_dialog::title(&d.fields);
    }
    match d.kind {
        DialogKind::Command => {
            let label = d.fields.get("__label").and_then(Value::as_str).unwrap_or("Command");
            // Dialog labels are catalogued with their "…" ("Export As…"); some commands omit it.
            let with_dots = format!("{}…", label.trim_end_matches('…'));
            let shown = if crate::i18n::has(crate::i18n::current(), label) { tl!(label) } else { tl!(&with_dots) };
            shown.trim_end_matches('…').to_string()
        }
        _ => tl!(&title(d)).to_string(),
    }
}

pub fn title(d: &Dialog) -> String {
    match d.kind {
        DialogKind::NewDocument => "New Document".into(),
        DialogKind::About if d.fields.get("systemInfo").and_then(Value::as_bool) == Some(true) => "System Info".into(),
        DialogKind::About => "About PhotoCraft".into(),
        DialogKind::LayerStyle => "Layer Style".into(),
        DialogKind::Command => d.fields.get("__label").and_then(Value::as_str).unwrap_or("Command").trim_end_matches('…').to_string(),
        DialogKind::Error => "Error".into(),
    }
}

type Fields = serde_json::Map<String, Value>;

/// The markers [`confirm`] and [`cancel`] choose a dialog's handler by: keys the code opening a
/// dialog puts in its fields. Keep in step with the arms of `confirm`.
const ROUTES: &[fn(&Fields) -> bool] = &[
    crate::guide_layout_ui::owns,
    crate::brush_preset_dialog::owns,
    crate::fill_ui::owns,
    crate::pattern_fill_ui::owns,
    crate::stroke_ui::owns,
    crate::channel_options_ui::owns,
    crate::channel_options_ui::owns_new_spot,
    crate::shape_dialog::owns,
    crate::delete_layer_prompt::owns,
    crate::disk_change_prompt::owns,
    crate::actions::owns_insert_stop,
    crate::new_layer_ui::owns,
    crate::new_group_ui::owns,
    crate::mask_trash_prompt::owns,
    crate::rasterize_prompt::owns,
    crate::duplicate_layer_ui::owns,
    crate::stack_dialog::owns,
    crate::tool_reset::owns,
    crate::crop_presets::owns,
    crate::variables_ui::owns,
    crate::file_ui::owns,
    crate::new_guide_ui::owns,
    crate::color_picker_ui::owns,
    crate::color_range_ui::owns,
    crate::prefs_ui::owns,
    crate::open_prompt::owns,
    crate::replace_prompt::owns,
    crate::layer_style::color_picker::owns,
    |f| f.contains_key("__export"),
    |f| f.contains_key("__smartFilter"),
    |f| f.contains_key(crate::actions::RESUME),
];

/// Whether `key` is a marker that picks a dialog's handler. Automation may change the values of
/// a dialog's fields, private ones included, but never add a marker: a dialog it opened would
/// otherwise confirm through another dialog's handler, one its command was never judged for.
pub fn is_route_marker(key: &str) -> bool {
    let probe = Fields::from_iter([(key.to_string(), Value::Bool(true))]);
    ROUTES.iter().any(|owns| owns(&probe))
}

/// The file command a dialog's handler stands for when it reads or writes files itself, outside
/// the engine (a disk-change save, Export As, Save for Web, Print), so automation is judged for
/// that command rather than for the dialog's `__command`.
fn file_command(fields: &Fields) -> Option<&'static str> {
    if crate::disk_change_prompt::owns(fields) {
        Some("file.save")
    } else if fields.contains_key("__export") {
        Some("file.export.exportAs")
    } else if fields.contains_key("__web") {
        Some("file.export.saveForWebLegacy")
    } else if fields.contains_key("__print") {
        Some("file.print")
    } else {
        None
    }
}

/// Confirm a command dialog no specific handler owns: run its `__command` with the dialog's
/// parameters, then settle the filter preview and the playing action.
fn confirm_generic_command(app: &mut PhotocraftApp, d: Dialog) -> Result<Value, String> {
    let Some(cmd) = d.fields.get("__command").and_then(Value::as_str).map(str::to_string) else {
        app.filter_preview = None;
        crate::actions::abandoned(app, &d.fields);
        return Err("dialog has no command".into());
    };
    if crate::filter_dialog::needs_displace_map(&cmd, &crate::filter_dialog::params_of(&d.fields)) {
        app.filter_preview = None;
        let result = crate::filter_dialog::ask_displace_map(app, cmd, d.fields.clone());
        if result.is_err() {
            crate::actions::abandoned(app, &d.fields);
        } else {
            // Cancel and an unavailable picker finish before the Displace callback
            // runs. Observe the whole file-dialog result, including those exits.
            app.after_file_dialog(move |app, result| {
                if result.is_err() {
                    crate::actions::abandoned(app, &d.fields);
                }
                result
            });
        }
        return result;
    }
    let (cmd, params) = match crate::smart_ui::confirm_command(app, &d.fields, cmd, crate::filter_dialog::params_of(&d.fields)) {
        Ok(edit) => edit,
        Err(e) => {
            app.filter_preview = None;
            return Err(e);
        }
    };
    let result = app.run(&cmd, params.clone());
    // A filter that runs as a background job keeps its preview on screen until the job
    // lands (`canvas::committed_filter_preview`) instead of flashing the unfiltered image.
    let running =
        app.session.active().is_some_and(|st| app.filter_preview.as_ref().is_some_and(|p| p.key.doc == st.doc.id) && app.session.job_on(st.doc.id).is_some());
    match app.filter_preview.as_mut() {
        Some(p) if result.is_ok() && running => p.committing = true,
        _ => app.filter_preview = None,
    }
    if result.is_ok()
        && cmd == "path.toSelection"
        && d.fields.contains_key("__deselectPath")
        && let Some(name) = params.get("name").and_then(Value::as_str)
    {
        crate::vector_ui::deselect_loaded_path(app, name);
    }
    if result.is_ok() && cmd == "view.newGuideLayout" {
        app.ui.view.guide_layout = params;
    }
    if result.is_ok() {
        if crate::adjust_dialog::owns(&d.fields) {
            crate::adjust_dialog::remember(app, &d.fields);
        } else {
            crate::filter_dialog::remember(app, &cmd, &d.fields);
        }
        // A step dialog of a playing action: the action continues after it.
        crate::actions::resume(app, &d.fields)?;
    } else {
        crate::actions::abandoned(app, &d.fields);
    }
    result
}

/// The checks before a dialog is confirmed: no open child picker, the automation gate for a handler
/// that touches files itself, and a valid guide layout.
fn confirm_preflight(app: &mut PhotocraftApp, id: u64) -> Result<(), String> {
    if crate::layer_style::color_picker::has_child(app, id) {
        return Err("confirm or cancel the Layer Style color picker first".into());
    }
    // A control request installs the session gate; synthetic input sets `automation_input`.
    // Either way the handler's own file access is judged before it runs, as its command would be.
    if (app.automation_input || app.session.authorize.is_some())
        && let Some(d) = app.ui.dialogs.iter().find(|d| d.id == id)
        && let Some(command) = file_command(&d.fields)
        && let Some(authorize) = app.services.automation_command.as_ref()
    {
        authorize(command, &Value::Object(d.fields.clone()))?;
    }
    if let Some(d) = app.ui.dialogs.iter().find(|d| d.id == id).filter(|d| crate::guide_layout_ui::owns(&d.fields)) {
        if app.ui.dialogs.iter().any(|child| crate::color_picker_ui::owns(&child.fields) && child.fields.get("__dialog").and_then(Value::as_u64) == Some(id)) {
            return Err("confirm or cancel the guide color picker first".into());
        }
        crate::guide_layout_ui::validate(app, &d.fields)?;
    }
    Ok(())
}

/// Confirm a dialog: run its action and close it. Used by the OK button and by automation.
/// Confirm a command dialog through the first handler whose marker its fields carry; the prompts
/// and small editors are here, the rest in `confirm_command_more`.
fn confirm_command(app: &mut PhotocraftApp, d: Dialog) -> Result<Value, String> {
    match d.kind {
        _ if crate::open_prompt::owns_damage(&d.fields) => crate::open_prompt::answer_damage(app, true),
        _ if crate::guide_layout_ui::owns(&d.fields) => crate::guide_layout_ui::confirm(app, &d.fields),
        _ if crate::brush_preset_dialog::owns(&d.fields) => crate::brush_preset_dialog::confirm(app, &d.fields),
        _ if crate::fill_ui::owns(&d.fields) => crate::fill_ui::confirm(app, &d.fields),
        _ if crate::pattern_fill_ui::owns(&d.fields) => crate::pattern_fill_ui::confirm(app, &d.fields),
        _ if crate::stroke_ui::owns(&d.fields) => crate::stroke_ui::confirm(app, &d.fields),
        _ if crate::new_group_ui::owns(&d.fields) => crate::new_group_ui::confirm(app, &d.fields),
        _ if crate::snapshot_dialog::owns(&d.fields) => crate::snapshot_dialog::confirm(app, &d.fields),
        _ if crate::channel_options_ui::owns(&d.fields) => crate::channel_options_ui::confirm(app, &d.fields),
        _ if crate::channel_options_ui::owns_new_spot(&d.fields) => crate::channel_options_ui::confirm_new_spot(app, &d.fields),
        _ if crate::shape_dialog::owns(&d.fields) => crate::shape_dialog::confirm(app, &d.fields),
        _ if crate::delete_layer_prompt::owns(&d.fields) => crate::delete_layer_prompt::confirm(app, &d.fields),
        _ if crate::disk_change_prompt::owns(&d.fields) => crate::disk_change_prompt::confirm(app, &d.fields),
        _ if crate::replace_prompt::owns(&d.fields) => crate::replace_prompt::confirm(app),
        _ if crate::actions::owns_insert_stop(&d.fields) => crate::actions::insert_stop_confirm(app, &d.fields),
        _ if crate::new_layer_ui::owns(&d.fields) => crate::new_layer_ui::confirm(app, &d.fields),
        _ if crate::mask_trash_prompt::owns(&d.fields) => crate::mask_trash_prompt::confirm(app, &d.fields),
        _ if crate::rasterize_prompt::owns(&d.fields) => crate::rasterize_prompt::confirm(app, &d.fields),
        _ if crate::duplicate_layer_ui::owns(&d.fields) => crate::duplicate_layer_ui::confirm(app, &d.fields),
        _ if crate::stack_dialog::owns(&d.fields) => crate::stack_dialog::confirm(app, &d.fields),
        _ if crate::tool_reset::owns(&d.fields) => crate::tool_reset::confirm(app),
        _ if crate::crop_presets::owns(&d.fields) => crate::crop_presets::confirm(app, &d.fields),
        _ if crate::variables_ui::owns(&d.fields) => crate::variables_ui::confirm(app, &d.fields),
        _ => confirm_command_more(app, d),
    }
}

/// The rest of the command dialogs: files, colour, preferences and export, then any other command.
fn confirm_command_more(app: &mut PhotocraftApp, d: Dialog) -> Result<Value, String> {
    match d.kind {
        _ if crate::file_ui::owns(&d.fields) => crate::file_ui::confirm(app, &d.fields),
        _ if crate::new_guide_ui::owns(&d.fields) => crate::new_guide_ui::confirm(app, &d.fields),
        _ if crate::color_picker_ui::owns(&d.fields) => crate::color_picker_ui::confirm(app, &d.fields),
        _ if crate::color_range_ui::owns(&d.fields) => crate::color_range_ui::confirm(app, &d.fields),
        _ if crate::prefs_ui::owns(&d.fields) => crate::prefs_ui::confirm(app, &d.fields),
        _ if d.fields.contains_key("__export") => crate::export_dialog::confirm(app, &d.fields),
        _ => confirm_generic_command(app, d),
    }
}

pub fn confirm(app: &mut PhotocraftApp, id: u64) -> Result<Value, String> {
    confirm_preflight(app, id)?;
    let d = app.ui.close_dialog(id).ok_or_else(|| format!("no dialog {id}"))?;
    match d.kind {
        DialogKind::NewDocument => {
            let r = app.run("file.new", crate::new_doc_ui::command_params(&d.fields));
            if let Some(i) = app.session.active_index() {
                app.ui.views[i].fit_pending = true;
            }
            if r.is_ok() {
                crate::remember_new_document(app, &d.fields);
                // File › New › Recent. Best effort: the document exists either way.
                let _ = app.run("document.recent.add", json!({"settings": crate::new_doc_ui::document_settings(&d.fields)}));
            }
            r
        }
        DialogKind::Command => confirm_command(app, d),
        DialogKind::LayerStyle => crate::layer_style::confirm(app, &d.fields),
        DialogKind::About | DialogKind::Error => Ok(Value::Null),
    }
}

/// Cancel a dialog and its dependent Layer Style picker, without applying edits.
pub fn cancel(app: &mut PhotocraftApp, id: u64) -> Result<Value, String> {
    let closed = app.ui.close_dialog(id).ok_or_else(|| format!("no dialog {id}"))?;
    crate::actions::stopped(app, &closed.fields);
    if crate::guide_layout_ui::owns(&closed.fields) {
        app.ui.dialogs.retain(|child| child.fields.get("__dialog").and_then(Value::as_u64) != Some(id));
    }
    // Cancel on a profile prompt opens neither its file nor the ones still to come.
    if crate::open_prompt::owns_damage(&closed.fields) {
        crate::open_prompt::answer_damage(app, false)?;
    } else if crate::open_prompt::owns(&closed.fields) {
        crate::open_prompt::answer(app, None)?;
    }
    // Cancel on the question whether to replace a file writes nothing.
    if crate::replace_prompt::owns(&closed.fields) {
        crate::replace_prompt::cancel(app)?;
    }
    app.ui.dialogs.retain(|d| !crate::layer_style::color_picker::child_of(&d.fields, id));
    app.filter_preview = None;
    app.color_range = None;
    crate::type_panels_ui::color_picker::prune(app);
    Ok(Value::Null)
}

/// Open the parameter dialog of `command`: the adjustment editor for `image.adjustments.*`, the
/// schema dialog for filters, the Color Range dialog for `select.colorRange`, otherwise a bare
/// confirm dialog.
pub fn open_command_dialog(app: &mut PhotocraftApp, command: &str, label: &str) -> u64 {
    if command == "view.newGuideLayout" {
        return crate::guide_layout_ui::open(app);
    }
    if command == crate::stroke_ui::COMMAND {
        return crate::stroke_ui::open(app);
    }
    if crate::new_group_ui::fronts(command) {
        return crate::new_group_ui::open(app, command);
    }
    if command == crate::color_range_ui::COMMAND {
        return crate::color_range_ui::open(app);
    }
    if let Some(id) = crate::adjust_dialog::open(app, command) {
        return id;
    }
    if crate::filter_dialog::has_dialog(command)
        && let Some(id) = crate::filter_dialog::open(app, command)
    {
        return id;
    }
    let mut fields = serde_json::Map::new();
    fields.insert("__command".into(), json!(command));
    fields.insert("__label".into(), json!(label));
    app.ui.open_dialog(DialogKind::Command, fields)
}

pub mod adjust_dialog;
pub mod adjust_editors;
pub mod adjust_preview;
pub(crate) mod bitmap_dialog;
pub mod blur_pins;
pub mod brush_preset_dialog;
pub mod caf_ui;
pub(super) mod camera_raw_detail_ui;
pub(super) mod camera_raw_gpu;
pub(super) mod camera_raw_preview_ui;
pub(super) mod camera_raw_scope_ui;
pub mod camera_raw_ui;
pub mod channel_options_ui;
pub mod color_picker_ui;
pub mod color_range_ui;
pub mod color_table_ui;
pub mod credits;
pub mod dds_options_ui;
pub mod delete_layer_prompt;
pub mod dialog_alt_reset;
pub mod dialog_blend_ui;
pub mod discard_ui;
pub mod disk_change_prompt;
pub mod distort_ui;
pub mod duplicate_layer_ui;
pub mod edit_history;
pub mod export_dialog;
pub mod file_ui;
pub mod fill_ui;
pub mod filter_dialog;
pub(super) mod filter_preview_ui;
#[cfg(not(target_arch = "wasm32"))]
pub(super) mod filter_preview_worker;
pub mod gallery_ui;
pub mod guide_layout_ui;
pub mod history_options_ui;
pub mod jpeg_options_ui;
pub mod layer_picker;
pub mod layer_style;
pub mod liquify_ui;
pub mod mask_trash_prompt;
pub mod new_doc_ui;
pub mod new_group_ui;
pub mod new_guide_ui;
pub mod new_layer_ui;
pub mod open_prompt;
pub mod palette;
pub(crate) mod pattern_fill_ui;
pub mod rasterize_prompt;
pub mod replace_prompt;
pub(super) mod rgb_histogram;
pub mod screen_picker;
pub mod shape_dialog;
pub(crate) mod shape_fill_ui;
pub(super) mod sizing;
pub mod smart_filter_dialog;
pub mod snapshot_dialog;
pub(crate) mod solid_fill_ui;
pub mod stack_dialog;
pub mod stroke_ui;
pub mod targa_options_ui;
#[cfg(test)]
mod tests;
pub mod tiff_options_ui;
pub mod vp_ui;
pub mod wide_angle_ui;

#[cfg(test)]
mod preference_resize_tests {
    use super::*;

    #[test]
    fn preferences_can_resize_while_standard_native_dialogs_keep_their_layout() {
        let prefs = json!({"__prefsui": "prefs"});
        let shortcuts = json!({"__prefsui": "shortcuts"});
        let ordinary = json!({});
        let web = json!({"__web": true});

        assert_eq!(resizable_dialog(prefs.as_object().unwrap()), Some(crate::prefs_ui::PREFERENCES_SIZE));
        assert_eq!(resizable_dialog(shortcuts.as_object().unwrap()), None);
        assert_eq!(resizable_dialog(ordinary.as_object().unwrap()), None);
        assert_eq!(resizable_dialog(web.as_object().unwrap()), Some(egui::vec2(960.0, 900.0)));
    }

    /// Drags the dialog's resize grip (its frame's bottom-right corner) by `by`.
    fn drag_corner(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, by: egui::Vec2) {
        let corner = frame(h).max - egui::vec2(14.0, 14.0);
        h.event(egui::Event::PointerMoved(corner));
        h.step();
        h.event(egui::Event::PointerButton { pos: corner, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerMoved(corner + by));
        h.step();
        h.event(egui::Event::PointerButton { pos: corner + by, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(4);
    }

    fn frame(h: &egui_kittest::Harness<'_, PhotocraftApp>) -> egui::Rect {
        let rects: Vec<egui::Rect> = h.ctx.data(|m| m.get_temp(egui::Id::unique(RECTS))).unwrap();
        rects[0]
    }

    /// The button row sits in the frame's bottom-right corner (its order follows the platform):
    /// the contents fill the dialog, no empty space.
    fn assert_buttons_in_corner(h: &egui_kittest::Harness<'_, PhotocraftApp>, when: &str) {
        use egui_kittest::kittest::Queryable;
        let f = frame(h);
        let row = ["OK", "Cancel", "Apply"].map(|l| h.get_by_label(l).rect()).into_iter().reduce(|a, b| a.union(b)).unwrap();
        // Inside the frame (not past its edge) and near it.
        let near = |gap: f32| (0.0..24.0).contains(&gap);
        assert!(near(f.bottom() - row.bottom()) && near(f.right() - row.right()), "{when}: frame {f:?}, buttons {row:?}");
    }

    #[test]
    fn preferences_contents_follow_the_dialog_when_it_is_resized() {
        // #7420: the dialog resized but its contents kept a size from the window, leaving the
        // buttons part-way up a dialog that opened, or was dragged, larger than them.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::prefs_ui::open_preferences(&mut app, "interface");
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_ui_state(
            |ui, app| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                    show(app, ui.ctx());
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(5);
        assert_buttons_in_corner(&h, "on open");
        let opened = frame(&h);
        drag_corner(&mut h, egui::vec2(220.0, 80.0));
        assert!(frame(&h).width() > opened.width() + 150.0 && frame(&h).height() > opened.height() + 50.0, "wider: {:?}", frame(&h));
        assert_buttons_in_corner(&h, "wider");
        drag_corner(&mut h, egui::vec2(-500.0, -300.0));
        let smaller = frame(&h);
        assert!(smaller.width() < opened.width() - 200.0 && smaller.height() < opened.height() - 150.0, "smaller: {smaller:?}");
        assert_buttons_in_corner(&h, "smaller");
        // The contents never ask for more than the dialog gives them, so it keeps its size.
        h.run_steps(10);
        assert_eq!(frame(&h), smaller);
    }

    /// #8199: no outline is drawn just around the contents, which left the title flush against a
    /// second, inner box instead of the frame's margin like the other dialogs.
    #[test]
    fn preferences_has_no_inner_outline_hugging_its_title() {
        fn find<'a>(shape: &'a egui::Shape, out: &mut Vec<&'a egui::Shape>) {
            match shape {
                egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| find(s, out)),
                s => out.push(s),
            }
        }
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        crate::prefs_ui::open_preferences(&mut app, "general");
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1000.0)).build_ui_state(
            |ui, app| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                    show(app, ui.ctx());
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, Default::default());
        h.run_steps(5);
        let mut shapes = Vec::new();
        h.output().shapes.iter().for_each(|s| find(&s.shape, &mut shapes));
        let title = shapes
            .iter()
            .find_map(|s| match s {
                egui::Shape::Text(t) if t.galley.job.text == "Preferences" => Some(t.pos),
                _ => None,
            })
            .expect("the title is painted");
        let hugging: Vec<egui::Rect> = shapes
            .iter()
            .filter_map(|s| match s {
                egui::Shape::Rect(r) if r.stroke.width > 0.0 && r.rect.contains(title) && (0.0..4.0).contains(&(title.x - r.rect.left())) => Some(r.rect),
                _ => None,
            })
            .collect();
        assert!(hugging.is_empty(), "an outline within 4 points of the title at {title:?}: {hugging:?}");
    }
}
