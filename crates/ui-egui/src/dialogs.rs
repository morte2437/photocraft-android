//! Dialogs rendered from `UiState::dialogs`. Field values live in the dialog data, so automation can
//! set them (`ui.dialog.set`) and confirm (`ui.dialog.confirm`) exactly like a user.

use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{Dialog, DialogKind};
use crate::widgets::{ButtonRole, DialogButton, dialog_buttons};

/// A dialog body sets this field to the key of the text field whose Browse… button was clicked;
/// [`show`] takes it out again and opens the matching file or folder dialog.
pub(crate) const BROWSE_FIELD: &str = "__browse";

/// Browse… beside a path field of dialog `id`: a folder picker for `dir` fields, a `.wasm` picker
/// for Install Plug-in…'s `path`. The answer fills the field.
fn browse_field(app: &mut PhotocraftApp, id: u64, fields: &serde_json::Map<String, Value>, key: &str) {
    let key = key.to_string();
    let r = if key == "dir" {
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
    ctx.data(|m| m.get_temp(egui::Id::new(RECTS))).unwrap_or_default()
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
    let key = egui::Id::new("pc-dialog-press-grabbed");
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
    let id = egui::Id::new("pc-dialog-free-drag");
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
        Some(cmd) => egui::Id::new(("dialog-place", cmd)),
        None => egui::Id::new(("dialog-place", format!("{:?}", d.kind))),
    }
}

/// Whether another dialog is drawn above `live` (egui's layer order, back to front).
fn older_dialog_above(ctx: &egui::Context, live: egui::LayerId, dialogs: &[Dialog]) -> bool {
    let others: Vec<egui::LayerId> =
        dialogs.iter().map(|d| egui::LayerId::new(egui::Order::Foreground, egui::Id::new(("dialog", d.id)))).filter(|l| *l != live).collect();
    ctx.memory(|m| m.layer_ids().skip_while(|l| *l != live).skip(1).any(|l| others.contains(&l)))
}

/// The size a resizable dialog opens at, or `None` for a dialog sized by its contents.
fn resizable_dialog(fields: &serde_json::Map<String, Value>) -> Option<egui::Vec2> {
    if crate::prefs_ui::is_preferences(fields) {
        Some(crate::prefs_ui::PREFERENCES_SIZE)
    } else if fields.contains_key("__web") {
        Some(egui::vec2(960.0, 900.0))
    } else {
        None
    }
}

fn show_modal<T>(
    modal: egui::Modal,
    ctx: &egui::Context,
    id: egui::Id,
    resizable: Option<egui::Vec2>,
    interactive: bool,
    compact: Option<f32>,
    contents: impl FnOnce(&mut egui::Ui) -> T,
) -> egui::ModalResponse<T> {
    modal.show(ctx, |ui| {
        // Disable the entire older modal, including its resize grip, not just its body.
        if !interactive {
            ui.disable();
        }
        if let Some(size) = resizable.filter(|_| compact.is_none()) {
            // Never a minimum bigger than the window.
            let room = (ctx.content_rect().size() - egui::vec2(40.0, 40.0)).max(egui::vec2(240.0, 200.0));
            let min = egui::vec2(560.0, 400.0).min(room);
            let max = room.max(min);
            // No stroke: Resize outlines its contents 2 points out, an inner box inside the
            // dialog's frame that left the title and buttons flush against it (#8199).
            egui::Resize::default()
                .id(id.with("resize"))
                .with_stroke(false)
                .default_size(size.min(max))
                .min_size(min)
                .max_size(max)
                .show(ui, contents)
        } else if let Some(h) = compact {
            // The phone layout: a dialog taller than `h` scrolls, OK and Cancel included.
            egui::ScrollArea::vertical().id_salt(id.with("compact-scroll")).max_height(h).min_scrolled_height(h).show(ui, contents).inner
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
        let id = egui::Id::new(("dialog", d.id));
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
        // The phone layout keeps dialogs centred: a place remembered from a bigger window could
        // be off this one.
        let phone = app.compact_layout;
        // In the phone layout a dialog whose work is on the canvas (Blur Gallery's pins) sits at
        // the bottom, at most 40% of the window tall, so the image stays in view above it.
        let on_canvas = phone && crate::blur_pins::owns(&d.fields);
        let screen_h = ctx.content_rect().height();
        let phone_height = phone.then(|| if on_canvas { (screen_h * 0.4).max(160.0) } else { (screen_h - 40.0).max(120.0) });
        let area = match start {
            _ if on_canvas => egui::Modal::default_area(id).anchor(egui::Align2::CENTER_BOTTOM, egui::vec2(0.0, -8.0)),
            Some(offset) if !phone => egui::Modal::default_area(id).anchor(egui::Align2::LEFT_TOP, offset),
            _ => egui::Modal::default_area(id),
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
        // The phone layout has no resize grip: the dialog fits the window and scrolls.
        let resizable = resizable_dialog(&fields).filter(|_| !phone);
        let modal = show_modal(modal, ctx, id, resizable, interactive, phone_height, |ui| {
            sizing = ui.is_sizing_pass();
            let available = ui.available_size();
            // A dialog of a few short fields (`__compact`, Fit Image) is narrower (#4673): the title
            // rule and the button row fill the width they are given, so it needs its own maximum too.
            let compact = d.fields.contains_key("__compact");
            // The curve editor caps its graph allocation at 300 points (#5113).
            let curves = fields.get("__adjust").and_then(Value::as_str) == Some("curves");
            // No minimum is wider than the window (a phone held upright).
            let fit = (ctx.content_rect().width() - 24.0).max(240.0);
            if !compact {
                ui.set_min_width((if curves { 300.0 } else { 380.0_f32 }).min(fit));
            }
            // A resizable dialog's contents fill its resizable area (#7420), the others the window.
            let room = if resizable.is_some() { available.x } else { (ctx.content_rect().width() - 48.0).max(240.0) };
            let wide = crate::prefs_ui::width(&d.fields, room);
            if let Some(w) = wide {
                ui.set_min_width(w.min(460.0).min(fit));
            }
            if d.kind == DialogKind::NewDocument {
                ui.set_min_width(800.0_f32.min(fit));
            }
            // The About window: room for the contributor table, the same width on every tab.
            let about_tabs = d.kind == DialogKind::About && d.fields.get("systemInfo").and_then(Value::as_bool) != Some(true);
            if about_tabs {
                ui.set_min_width(700.0_f32.min(fit));
            }
            ui.set_max_width(wide.unwrap_or(if d.kind == DialogKind::NewDocument {
                800.0
            } else if about_tabs {
                700.0
            } else if d.kind == DialogKind::LayerStyle || d.fields.contains_key("__export") || crate::color_picker_ui::owns(&d.fields) {
                600.0
            } else if compact {
                260.0
            } else if curves {
                300.0
            } else {
                440.0
            }));
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
            } else if phone && !web {
                // The phone layout: every dialog takes the window's width (less its frame).
                let full = (ctx.content_rect().width() - 48.0).max(240.0);
                ui.set_min_width(full);
                ui.set_max_width(full);
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
            match d.kind {
                DialogKind::NewDocument => {
                    if crate::new_doc_ui::body(app, ui, &mut fields) {
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
                    let tab = about_tab(&fields);
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
                DialogKind::Command if crate::guide_layout_ui::owns(&fields) => outcome = crate::guide_layout_ui::body(app, ui, &mut fields, interactive),
                DialogKind::Command if crate::brush_preset_dialog::owns(&fields) => {
                    outcome = crate::brush_preset_dialog::body(app, ui, &mut fields, interactive)
                }
                DialogKind::Command if crate::fill_ui::owns(&fields) => crate::fill_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::pattern_fill_ui::owns(&fields) => crate::pattern_fill_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::stroke_ui::owns(&fields) => crate::stroke_ui::body(ui, &mut fields),
                DialogKind::Command if crate::channel_options_ui::owns(&fields) => crate::channel_options_ui::body(ui, &mut fields),
                DialogKind::Command if crate::channel_options_ui::owns_new_spot(&fields) => crate::channel_options_ui::new_spot_body(ui, &mut fields),
                DialogKind::Command if crate::shape_dialog::owns(&fields) => crate::shape_dialog::body(app, ui, &mut fields),
                DialogKind::Command if crate::delete_layer_prompt::owns(&fields) => crate::delete_layer_prompt::body(ui, &mut fields),
                DialogKind::Command if crate::disk_change_prompt::owns(&fields) => crate::disk_change_prompt::body(ui, &fields),
                DialogKind::Command if crate::replace_prompt::owns(&fields) => crate::replace_prompt::body(ui, &fields),
                DialogKind::Command if crate::actions::owns_insert_stop(&fields) => crate::actions::insert_stop_body(ui, &mut fields),
                DialogKind::Command if crate::new_layer_ui::owns(&fields) => crate::new_layer_ui::body(ui, &mut fields),
                DialogKind::Command if crate::mask_trash_prompt::owns(&fields) => crate::mask_trash_prompt::body(ui, &fields),
                DialogKind::Command if crate::rasterize_prompt::owns(&fields) => crate::rasterize_prompt::body(ui, &fields),
                DialogKind::Command if crate::duplicate_layer_ui::owns(&fields) => crate::duplicate_layer_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::stack_dialog::owns(&fields) => crate::stack_dialog::body(app, ui, &mut fields, d.id),
                DialogKind::Command if crate::tool_reset::owns(&fields) => crate::tool_reset::body(ui),
                DialogKind::Command if crate::crop_presets::owns(&fields) => crate::crop_presets::body(app, ui, &mut fields),
                DialogKind::Command if crate::variables_ui::owns(&fields) => crate::variables_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::file_ui::owns(&fields) => crate::file_ui::body(app, ui, &mut fields, d.id),
                DialogKind::Command if crate::color_picker_ui::owns(&fields) => {
                    outcome = crate::color_picker_ui::body(ui, &mut fields);
                    crate::color_picker_ui::take_add_swatch(app, &mut fields);
                    if let Some(web) = fields.get("__webOnly").and_then(Value::as_bool) {
                        app.ui.color_panel.web_only = web;
                    }
                }
                DialogKind::Command if crate::color_range_ui::owns(&fields) => crate::color_range_ui::body(app, ui, &mut fields),
                DialogKind::Command if crate::prefs_ui::owns(&fields) => crate::prefs_ui::body(app, ui, &mut fields),
                DialogKind::Command if fields.contains_key("__export") => crate::export_dialog::body(app, ui, &mut fields),
                // Image Size and Canvas Size open on Width, selected (#2329).
                DialogKind::Command if fields.contains_key("__sizing") => first_field_focused(ui, id, |ui| crate::sizing::body(ui, &mut fields)),
                DialogKind::Command if crate::smart_filter_dialog::owns(&fields) => crate::smart_filter_dialog::body(app, ui, &mut fields),
                DialogKind::Command if crate::adjust_dialog::owns(&fields) => crate::adjust_dialog::body(app, ui, &mut fields),
                DialogKind::Command if crate::blur_pins::owns(&fields) => crate::blur_pins::body(ui, &mut fields),
                DialogKind::Command if fields.contains_key("__filter") => {
                    // Color Settings: the monitor profile in use can change while it is open.
                    if fields.get("__command").and_then(Value::as_str) == Some("edit.colorSettings") {
                        fields.insert("__note".into(), Value::String(crate::monitor_status::note(app)));
                    }
                    // Long parameter lists (Flame, Lighting Effects) scroll, so the title and the
                    // OK / Cancel buttons stay inside a small window. The preview window below the
                    // parameters reserves its own space (#3769).
                    let room = (ctx.content_rect().height() - DIALOG_CHROME - crate::filter_preview_ui::reserved_height(&fields)).max(120.0);
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
                    if crate::select_mask_view::owns(&fields) {
                        body(ui, &mut fields);
                    } else {
                        first_field_focused(ui, id, |ui| body(ui, &mut fields));
                    }
                }
                DialogKind::Command if crate::new_guide_ui::owns(&fields) => crate::new_guide_ui::body(ui, &mut fields),
                DialogKind::Command if fields.contains_key("__form") => crate::view_cmds::form_body(ui, &mut fields),
                DialogKind::Command => {}
                DialogKind::LayerStyle => crate::layer_style::body(app, ui, &mut fields),
                DialogKind::Error => {
                    ui.label(fields.get("message").and_then(Value::as_str).unwrap_or("Error"));
                }
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
                        let ok_label = if d.kind == DialogKind::NewDocument {
                            tl!("Create")
                        } else if crate::delete_layer_prompt::owns(&d.fields) {
                            tl!("Delete")
                        } else if crate::disk_change_prompt::owns(&d.fields) || crate::replace_prompt::owns(&d.fields) {
                            tl!("Replace")
                        } else if d.fields.contains_key("__export") {
                            tl!("Export")
                        } else {
                            crate::file_ui::ok_label(&d.fields).unwrap_or(tl!("OK"))
                        };
                        // Dialogs that need an input first (a preset name, a selection) grey OK out.
                        let can_ok = crate::crop_presets::can_confirm(app, &fields) && crate::stack_dialog::can_confirm(&fields);
                        let ok = DialogButton::new(ButtonRole::Default, ok_label, 84.0).enabled(can_ok);
                        // Photoshop: holding Alt turns Cancel into Reset (the dialog stays open).
                        // Dialogs opt in via `dialog_alt_reset::supported` (per marker).
                        let reset = ui.input(|i| i.modifiers.alt) && crate::dialog_alt_reset::supported(&fields);
                        let cancel_label = if reset {
                            tl!("Reset")
                        } else if d.kind == DialogKind::NewDocument {
                            tl!("Close")
                        } else {
                            tl!("Cancel")
                        };
                        let cancel = DialogButton::new(ButtonRole::Cancel, cancel_label, 84.0);
                        let clicked = if d.kind == DialogKind::Command && crate::prefs_ui::is_preferences(&fields) {
                            let changed = crate::prefs_ui::preferences_changed(app, &fields);
                            dialog_buttons(ui, &[ok, cancel, DialogButton::new(ButtonRole::Apply, tl!("Apply"), 84.0).enabled(changed)])
                        } else if crate::mask_trash_prompt::owns(&fields) && !crate::mask_trash_prompt::is_vector(&fields) {
                            // Photoshop: Apply / Cancel / Delete. Delete answers with the choice set.
                            let apply = DialogButton::new(ButtonRole::Default, tl!("Apply"), 84.0).enabled(crate::mask_trash_prompt::can_apply(&fields));
                            let delete = DialogButton::new(ButtonRole::Alternate, tl!("Delete"), 84.0);
                            let hit = dialog_buttons(ui, &[apply, cancel, delete]);
                            if hit == Some(ButtonRole::Alternate) {
                                fields.insert("choice".into(), json!("delete"));
                            } else if hit == Some(ButtonRole::Default) {
                                fields.insert("choice".into(), json!("apply"));
                            }
                            hit
                        } else if crate::rasterize_prompt::is_filter(&fields) {
                            // Photoshop: Convert to Smart Object / Rasterize / Cancel (#4303).
                            let rasterize = DialogButton::new(ButtonRole::Default, tl!("Rasterize"), 84.0);
                            let hit = if crate::rasterize_prompt::offers_smart_object(&fields) {
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
                            Some(ButtonRole::Cancel) if reset => crate::dialog_alt_reset::apply(app, ui.ctx(), &mut fields),
                            Some(ButtonRole::Cancel) => outcome = Some(false),
                            Some(ButtonRole::Apply) => apply_requested = true,
                            Some(_) => outcome = Some(true),
                            None if interactive
                                && can_ok
                                && (!web || (!web_popup_open && !egui::Popup::is_any_open(ui.ctx())))
                                && ui.input(|i| i.key_pressed(egui::Key::Enter)) =>
                            {
                                outcome = Some(true)
                            }
                            None => {}
                        }
                    }
                });
            }
            // The frame around the content (Frame::popup's margin and stroke).
            ui.min_rect().expand(ui.spacing().menu_margin.sum().max_elem() + 2.0)
        });
        // The picker has a larger frame margin; include that whole frame in the canvas hit guard.
        shown.push(if color_picker { modal.response.rect } else { modal.inner });
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
    ctx.data_mut(|m| m.insert_temp(egui::Id::new(RECTS), shown));
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

/// Confirm a dialog: run its action and close it. Used by the OK button and by automation.
pub fn confirm(app: &mut PhotocraftApp, id: u64) -> Result<Value, String> {
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
        DialogKind::Command if crate::guide_layout_ui::owns(&d.fields) => crate::guide_layout_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::brush_preset_dialog::owns(&d.fields) => crate::brush_preset_dialog::confirm(app, &d.fields),
        DialogKind::Command if crate::fill_ui::owns(&d.fields) => crate::fill_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::pattern_fill_ui::owns(&d.fields) => crate::pattern_fill_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::stroke_ui::owns(&d.fields) => crate::stroke_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::channel_options_ui::owns(&d.fields) => crate::channel_options_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::channel_options_ui::owns_new_spot(&d.fields) => crate::channel_options_ui::confirm_new_spot(app, &d.fields),
        DialogKind::Command if crate::shape_dialog::owns(&d.fields) => crate::shape_dialog::confirm(app, &d.fields),
        DialogKind::Command if crate::delete_layer_prompt::owns(&d.fields) => crate::delete_layer_prompt::confirm(app, &d.fields),
        DialogKind::Command if crate::disk_change_prompt::owns(&d.fields) => crate::disk_change_prompt::confirm(app, &d.fields),
        DialogKind::Command if crate::replace_prompt::owns(&d.fields) => crate::replace_prompt::confirm(app),
        DialogKind::Command if crate::actions::owns_insert_stop(&d.fields) => crate::actions::insert_stop_confirm(app, &d.fields),
        DialogKind::Command if crate::new_layer_ui::owns(&d.fields) => crate::new_layer_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::mask_trash_prompt::owns(&d.fields) => crate::mask_trash_prompt::confirm(app, &d.fields),
        DialogKind::Command if crate::rasterize_prompt::owns(&d.fields) => crate::rasterize_prompt::confirm(app, &d.fields),
        DialogKind::Command if crate::duplicate_layer_ui::owns(&d.fields) => crate::duplicate_layer_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::stack_dialog::owns(&d.fields) => crate::stack_dialog::confirm(app, &d.fields),
        DialogKind::Command if crate::tool_reset::owns(&d.fields) => crate::tool_reset::confirm(app),
        DialogKind::Command if crate::crop_presets::owns(&d.fields) => crate::crop_presets::confirm(app, &d.fields),
        DialogKind::Command if crate::variables_ui::owns(&d.fields) => crate::variables_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::file_ui::owns(&d.fields) => crate::file_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::new_guide_ui::owns(&d.fields) => crate::new_guide_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::color_picker_ui::owns(&d.fields) => crate::color_picker_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::color_range_ui::owns(&d.fields) => crate::color_range_ui::confirm(app, &d.fields),
        DialogKind::Command if crate::prefs_ui::owns(&d.fields) => crate::prefs_ui::confirm(app, &d.fields),
        DialogKind::Command if d.fields.contains_key("__export") => crate::export_dialog::confirm(app, &d.fields),
        DialogKind::Command => {
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
            let running = app
                .session
                .active()
                .is_some_and(|st| app.filter_preview.as_ref().is_some_and(|p| p.key.doc == st.doc.id) && app.session.job_on(st.doc.id).is_some());
            match app.filter_preview.as_mut() {
                Some(p) if result.is_ok() && running => p.committing = true,
                _ => app.filter_preview = None,
            }
            if result.is_ok() && cmd == "view.newGuideLayout" {
                app.ui.view.guide_layout = params;
            }
            if result.is_ok() {
                crate::filter_dialog::remember(app, &cmd, &d.fields);
                // A step dialog of a playing action: the action continues after it.
                crate::actions::resume(app, &d.fields)?;
            } else {
                crate::actions::abandoned(app, &d.fields);
            }
            result
        }
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
    if crate::open_prompt::owns(&closed.fields) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// A filter dialog over a 256×256 layer, with the preview the canvas computed for it on screen.
    fn filter_dialog_with_preview(background: bool) -> (PhotocraftApp, u64) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.background_jobs = background;
        app.run("file.new", serde_json::json!({"width": 256, "height": 256, "background": "#3366cc"})).unwrap();
        let id = open_command_dialog(&mut app, "filter.blur.gaussianBlur", "Gaussian Blur");
        let st = app.session.active().unwrap();
        let result = crate::filter_dialog::preview_document(&st.doc, st.active_layer, "filter.blur.gaussianBlur", &serde_json::json!({"radius": 4.0}), 1);
        let key = crate::filter_dialog::FilterPreviewKey {
            doc: st.doc.id,
            revision: st.revision,
            dialog: id,
            active: st.active_layer,
            target: st.channel_view.target,
            command: "filter.blur.gaussianBlur".into(),
            params: serde_json::json!({"radius": 4.0}),
            k: 1,
        };
        app.filter_preview = Some(crate::filter_dialog::FilterPreview { key, result: result.map(std::sync::Arc::new), committing: false });
        (app, id)
    }

    #[test]
    fn a_filters_preview_stays_on_screen_until_its_job_lands() {
        // OK dropped the preview and the filter ran as a background job, so the canvas flashed
        // the unfiltered image until the job finished.
        let (mut app, id) = filter_dialog_with_preview(true);
        let revision = app.session.active().unwrap().revision;
        confirm(&mut app, id).unwrap();
        let doc = app.session.active().unwrap().doc.id;
        let job = app.session.job_on(doc).expect("the blur runs as a background job").id;
        assert!(matches!(crate::canvas::committed_filter_preview(&mut app, 0), Some(Some((1, _, [256, 256])))), "the preview is shown");
        // The job lands: the document changes and the preview goes.
        app.session.wait_job(job).unwrap();
        assert_ne!(app.session.active().unwrap().revision, revision);
        assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), Some(None));
        assert!(app.filter_preview.is_none());

        // A cancelled job leaves the document as it was, so its preview goes too.
        let (mut app, id) = filter_dialog_with_preview(true);
        confirm(&mut app, id).unwrap();
        let doc = app.session.active().unwrap().doc.id;
        let job = app.session.job_on(doc).unwrap().id;
        assert!(matches!(crate::canvas::committed_filter_preview(&mut app, 0), Some(Some(_))));
        assert!(app.session.cancel_job(job));
        assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), Some(None));
        assert!(app.filter_preview.is_none());

        // Inline (no background jobs): the filter has already landed, nothing is held.
        let (mut app, id) = filter_dialog_with_preview(false);
        confirm(&mut app, id).unwrap();
        assert!(app.filter_preview.is_none());
        assert_eq!(crate::canvas::committed_filter_preview(&mut app, 0), None);
    }

    /// #5496: a pointer click on the title bar's close box does what Cancel does: the dialog
    /// closes, the live preview goes and nothing reaches the document or the journal. On macOS
    /// dialogs have no close box.
    #[test]
    fn the_title_bar_close_box_cancels_like_cancel() {
        use egui_kittest::kittest::Queryable;
        let close = |button: &str| {
            let (mut app, _) = filter_dialog_with_preview(false);
            let revision = app.session.active().unwrap().revision;
            let journal = app.session.journal.len();
            // The fonts are set up after the harness's first frame: the dialog appears after it.
            let dialogs = std::mem::take(&mut app.ui.dialogs);
            let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.state_mut().ui.dialogs = dialogs;
            h.run_steps(4);
            assert!(h.state().filter_preview.is_some(), "the preview is on screen");
            if !CLOSE_BOX {
                assert_eq!(h.query_all_by_label("Close").count(), 0, "no close box on macOS");
            }
            let target = h.query_by_label(button).map(|n| n.rect().center())?;
            h.hover_at(target);
            h.drag_at(target);
            h.drop_at(target);
            h.run_steps(3);
            let app = h.state();
            Some((
                app.ui.dialogs.is_empty(),
                app.filter_preview.is_none(),
                app.session.active().unwrap().revision == revision,
                app.session.journal.len() == journal,
            ))
        };
        let cancel = close("Cancel");
        assert_eq!(cancel, Some((true, true, true, true)), "Cancel closes, drops the preview and changes nothing");
        if CLOSE_BOX {
            assert_eq!(close("Close"), cancel, "the × is Cancel");
        }
    }

    /// #5496: the close box sits at the title band's right end, and the band stays a drag handle
    /// around it: a drag that starts on the × neither moves nor closes the dialog, and the band
    /// just left of the × still moves it.
    #[test]
    fn the_close_box_and_the_title_drag_keep_apart() {
        use egui_kittest::kittest::Queryable;
        if !CLOSE_BOX {
            return;
        }
        let mut h = dialog_harness(egui::vec2(1280.0, 800.0), "filter.blur.gaussianBlur");
        let title = h.get_by_label("Gaussian Blur").rect();
        let x = h.get_by_label("Close").rect();
        let frame = rects(&h.ctx)[0];
        assert!((x.center().y - title.center().y).abs() < 2.0 && x.left() > title.right(), "the × is on the title row: {x:?}, {title:?}");
        assert!(frame.right() - x.right() < 16.0, "at its right end: {x:?} in {frame:?}");
        let drag = |h: &mut egui_kittest::Harness<'static, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2| {
            h.hover_at(from);
            h.drag_at(from);
            h.run_steps(2);
            for i in 1..=8 {
                h.hover_at(from + (to - from) * (i as f32 / 8.0));
                h.run_steps(1);
            }
            h.drop_at(to);
            h.run_steps(3);
        };
        drag(&mut h, x.center(), x.center() + egui::vec2(-80.0, 60.0));
        assert_eq!(h.state().ui.dialogs.len(), 1, "a drag from the × doesn't close the dialog");
        assert!((h.get_by_label("Gaussian Blur").rect().min - title.min).length() < 1.0, "nor move it");
        let x = h.get_by_label("Close").rect();
        let beside = x.left_center() - egui::vec2(4.0, 0.0);
        drag(&mut h, beside, beside + egui::vec2(-120.0, 40.0));
        assert_eq!(h.state().ui.dialogs.len(), 1, "a title drag doesn't close the dialog");
        let moved = h.get_by_label("Gaussian Blur").rect().min - title.min;
        assert!((moved - egui::vec2(-120.0, 40.0)).length() < 2.0, "the band beside the × moved the dialog: {moved:?}");
    }

    /// #6810: dragging a dialog's title moves the dialog under the pointer, so the press origin is
    /// no longer inside it; that drag must not pan the canvas or reach a canvas tool.
    #[test]
    fn dragging_a_dialog_title_does_not_pan_the_canvas() {
        let ctx = egui::Context::default();
        let canvas = egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1000.0, 800.0));
        let at_press = egui::Rect::from_min_max(egui::pos2(100.0, 100.0), egui::pos2(300.0, 200.0));
        let moved = egui::Rect::from_min_max(egui::pos2(550.0, 480.0), egui::pos2(750.0, 580.0));
        let run = |rect: egui::Rect, events: Vec<egui::Event>| {
            ctx.data_mut(|m| m.insert_temp(egui::Id::new(RECTS), vec![rect]));
            ctx.begin_pass(egui::RawInput { events, ..Default::default() });
            let out = (pan_delta(&ctx, canvas, true), free_press(&ctx, canvas));
            ctx.end_pass().textures_delta.clear();
            out
        };
        let press =
            egui::Event::PointerButton { pos: egui::pos2(150.0, 120.0), button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE };
        let (pan, click) = run(at_press, vec![egui::Event::PointerMoved(egui::pos2(150.0, 120.0)), press]);
        assert!(pan.is_none() && click.is_none(), "the press is on the dialog");
        let (pan, click) = run(moved, vec![egui::Event::PointerMoved(egui::pos2(600.0, 500.0))]);
        assert!(pan.is_none(), "the dialog moved away from the press origin: {pan:?}");
        assert!(click.is_none(), "and the drag is not a canvas tool drag: {click:?}");
    }

    #[test]
    fn color_picker_has_inset_content_and_actions_at_the_right_edge() {
        use egui_kittest::{Harness, kittest::Queryable};

        for size in [egui::vec2(760.0, 480.0), egui::vec2(960.0, 640.0)] {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h = Harness::builder().with_size(size).build_ui_state(|ui, app| show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            crate::color_picker_ui::open(h.state_mut(), "foreground");
            h.run_steps(4);
            let frame = rects(&h.ctx)[0];
            let title = h.get_by_label("Color Picker (Foreground Color)").rect();
            let ok = h.get_by_label("OK").rect();
            let cancel = h.get_by_label("Cancel").rect();
            let web = h.get_by_label("Only Web Colors").rect();
            assert!((ok.right() - title.right()).abs() < 1.0, "OK aligns with the right content edge: {ok:?} vs {title:?}");
            assert!((cancel.right() - ok.right()).abs() < 1.0);
            assert!(frame.right() - ok.right() >= 16.0, "buttons retain the window's padding");
            assert!(web.left() - frame.left() >= 16.0, "checkbox has comfortable left padding");
            assert!(frame.bottom() - web.bottom() >= 16.0, "checkbox has comfortable bottom padding");
            assert!(title.top() - frame.top() >= 16.0, "title has top padding");
            assert!(egui::Rect::from_min_size(egui::Pos2::ZERO, size).contains_rect(frame), "padded picker fits {size:?}: {frame:?}");
            // A click anywhere inside the added margin must stay out of the canvas eyedropper.
            let margin = egui::pos2(frame.left() + 8.0, web.center().y);
            h.hover_at(margin);
            h.run_steps(1);
            assert!(free_pointer_over(&h.ctx, egui::Rect::from_min_size(egui::Pos2::ZERO, size)).is_none());
        }
    }

    #[test]
    fn layer_style_title_gutters_can_start_drags() {
        use egui_kittest::{Harness, kittest::Queryable};

        for point in 0..5 {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
            PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
            h.state_mut().ui.open_dialog(DialogKind::LayerStyle, serde_json::Map::new());
            h.run_steps(3);
            let before = h.get_by_label("Layer Style").rect();
            let from = match point {
                0 => egui::pos2(before.left() - 5.0, before.bottom() - 4.0),
                1 => egui::pos2(before.right() + 5.0, before.bottom() - 4.0),
                2 => egui::pos2(before.center().x, before.bottom() + 3.0),
                3 => egui::pos2(before.center().x, before.top() - 5.0),
                _ => egui::pos2(before.center().x, before.bottom() - 3.0),
            };
            h.hover_at(from);
            h.drag_at(from);
            h.run_steps(2);
            for i in 1..=8 {
                h.hover_at(from + egui::vec2(64.0, -32.0) * (i as f32 / 8.0));
                h.run_steps(1);
            }
            h.drop_at(from + egui::vec2(64.0, -32.0));
            h.run_steps(3);
            let delta = h.get_by_label("Layer Style").rect().min - before.min;
            assert!((delta - egui::vec2(64.0, -32.0)).length() < 2.0, "point {point}: moved {delta:?}");
            assert_eq!(h.state().ui.dialogs.len(), 1);
        }
    }

    #[test]
    fn dragging_the_title_bar_moves_the_dialog() {
        use egui_kittest::{Harness, kittest::Queryable};

        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut harness = Harness::builder().with_size(egui::vec2(1400.0, 900.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&harness.ctx, crate::theme::ThemeKind::ALL[0]);
        harness.state_mut().ui.open_dialog(DialogKind::LayerStyle, serde_json::Map::new());
        harness.run_steps(3);
        let before = harness.get_by_label("Layer Style").rect();

        // Grab the title text itself: it must move the dialog, not select the text.
        let from = before.center();
        harness.hover_at(from);
        harness.drag_at(from);
        harness.run_steps(2);
        for i in 1..=10 {
            harness.hover_at(from + egui::vec2(-12.0, 8.0) * i as f32);
            harness.run_steps(1);
        }
        harness.drop_at(from + egui::vec2(-120.0, 80.0));
        harness.run_steps(3);

        let moved = harness.get_by_label("Layer Style").rect().min - before.min;
        assert!((moved - egui::vec2(-120.0, 80.0)).length() < 1.0, "dialog moved by {moved:?}");
        assert_eq!(harness.state().ui.dialogs.len(), 1, "dragging must not close the dialog");
    }

    /// #4872: an alert (Error, About) closes with ↩ as with OK, but not on the ↩ that raised it.
    #[test]
    fn enter_closes_an_alert_but_not_the_enter_that_opened_it() {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(2);
        let alert = |h: &mut egui_kittest::Harness<'static, PhotocraftApp>| {
            h.state_mut().ui.open_dialog(DialogKind::Error, json!({"message": crate::move_lock::MESSAGE}).as_object().cloned().unwrap_or_default())
        };
        // Raised in the frame its own ↩ arrives: that ↩ leaves it open.
        alert(&mut h);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert_eq!(h.state().ui.dialogs.len(), 1, "the ↩ that raised the alert doesn't close it");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert!(h.state().ui.dialogs.is_empty(), "↩ is OK");
        // A new alert starts over, and Esc still closes it.
        alert(&mut h);
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert!(h.state().ui.dialogs.is_empty(), "Esc too");
    }

    fn dialog_harness(size: egui::Vec2, command: &str) -> egui_kittest::Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(size).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        crate::filter_dialog::open(h.state_mut(), command).unwrap();
        h.run_steps(4);
        h
    }

    /// #3152: Motion Blur's angle is a dial, as in Photoshop. Clicking or dragging it points the
    /// blur; the field still takes the whole -360..360 range.
    #[test]
    fn motion_blur_angle_dial_sets_the_angle_by_click_and_drag() {
        use egui_kittest::kittest::Queryable;
        let mut h = dialog_harness(egui::vec2(1280.0, 800.0), "filter.blur.motionBlur");
        let angle = |h: &egui_kittest::Harness<'static, PhotocraftApp>| h.state().ui.dialogs.last().and_then(|d| d.fields["angle"].as_f64());
        let centre = |h: &egui_kittest::Harness<'static, PhotocraftApp>| h.get_by_role_and_label(egui::accesskit::Role::Slider, "Angle").rect().center();
        let press = |h: &mut egui_kittest::Harness<'static, PhotocraftApp>, pos: egui::Pos2, pressed: bool| {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() });
        };
        assert_eq!(angle(&h), Some(0.0));
        // The dial's field is the dialog's first number, so it opens focused (#1757).
        let first = h.get_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap();
        assert!(first.is_focused() && first.rect().center().y < h.get_by_label("Distance").rect().center().y, "the Angle field has focus");
        // Click above the centre: straight up is 90°.
        let up = centre(&h) + egui::vec2(0.0, -15.0);
        h.event(egui::Event::PointerMoved(up));
        press(&mut h, up, true);
        h.step();
        press(&mut h, up, false);
        h.run_steps(3);
        assert_eq!(angle(&h), Some(90.0));
        // Drag from the right (0°) round to the lower left (-135°).
        let c = centre(&h);
        let from = c + egui::vec2(15.0, 0.0);
        h.event(egui::Event::PointerMoved(from));
        press(&mut h, from, true);
        h.step();
        for (k, to) in [egui::vec2(12.0, 12.0), egui::vec2(0.0, 15.0), egui::vec2(-12.0, 12.0)].into_iter().enumerate() {
            h.event(egui::Event::PointerMoved(c + to));
            h.step();
            assert_eq!(angle(&h), Some([-45.0, -90.0, -135.0][k]), "dragged to {to:?}");
        }
        press(&mut h, c + egui::vec2(-12.0, 12.0), false);
        h.run_steps(3);
        assert_eq!(angle(&h), Some(-135.0));
        // A remembered or typed angle past half a turn is kept, not clamped to the dial's ±180°.
        if let Some(d) = h.state_mut().ui.dialogs.last_mut() {
            d.fields.insert("angle".into(), json!(270.0));
        }
        h.run_steps(3);
        assert_eq!(angle(&h), Some(270.0));
    }

    #[test]
    fn a_tall_filter_dialog_keeps_its_title_and_buttons_in_a_small_window() {
        // Filter › Render › Flame was taller than a 1000×560 window: OK and Cancel were cut off.
        use egui_kittest::kittest::Queryable;
        let size = egui::vec2(1000.0, 560.0);
        let h = dialog_harness(size, "filter.render.flame");
        for label in ["Flame", "OK", "Cancel"] {
            let r = h.get_by_label(label).rect();
            assert!(r.top() >= 0.0 && r.bottom() <= size.y, "{label} at {r:?} is outside the {size:?} window");
        }
    }

    #[test]
    fn a_moved_dialog_reopens_where_it_was_left() {
        use egui_kittest::kittest::Queryable;
        let size = egui::vec2(1280.0, 800.0);
        let mut h = dialog_harness(size, "filter.blur.gaussianBlur");
        let centred = h.get_by_label("Gaussian Blur").rect();
        // Drag the title bar up and to the left.
        let (from, by) = (centred.center(), egui::vec2(-240.0, -150.0));
        h.event(egui::Event::PointerMoved(from));
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: Default::default() });
        for k in 1..=6 {
            h.event(egui::Event::PointerMoved(from + by * (k as f32 / 6.0)));
            h.step();
        }
        h.event(egui::Event::PointerButton { pos: from + by, button: egui::PointerButton::Primary, pressed: false, modifiers: Default::default() });
        h.run_steps(3);
        let moved = h.get_by_label("Gaussian Blur").rect();
        assert!((moved.min - (centred.min + by)).length() < 2.0, "dragged: {centred:?} → {moved:?}");
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty());
        // Reopened: where it was left.
        crate::filter_dialog::open(h.state_mut(), "filter.blur.gaussianBlur").unwrap();
        h.run_steps(4);
        let reopened = h.get_by_label("Gaussian Blur").rect();
        assert!((reopened.min - moved.min).length() < 2.0, "reopened at {reopened:?}, left at {moved:?}");
        h.get_by_label("Cancel").click();
        h.run_steps(3);
        // Another dialog keeps its own place: never moved, it opens centred.
        crate::filter_dialog::open(h.state_mut(), "filter.blur.motionBlur").unwrap();
        h.run_steps(4);
        let other = h.get_by_label("Motion Blur").rect();
        assert!(other.min.x > moved.min.x + 100.0, "Motion Blur opened centred: {other:?}");
    }

    /// #1757: like Photoshop, a value dialog opens on its first number with the text selected, so
    /// typing replaces it and Enter applies it; it reopens with the value applied last time, and a
    /// cancelled edit is not remembered.
    #[test]
    fn a_value_dialog_opens_on_its_first_number_selected_and_remembers_it() {
        use egui_kittest::kittest::Queryable;
        const BLUR: &str = "filter.blur.gaussianBlur";
        let mut h = dialog_harness(egui::vec2(1280.0, 800.0), BLUR);
        let radius = |h: &egui_kittest::Harness<'static, PhotocraftApp>| h.state().ui.dialogs.last().and_then(|d| d.fields["radius"].as_f64());
        assert_eq!(radius(&h), Some(1.0), "the default the first time");
        assert!(h.get_by_role(egui::accesskit::Role::SpinButton).is_focused(), "the Radius field has focus");
        h.event(egui::Event::Text("5".into()));
        h.run_steps(1);
        assert_eq!(radius(&h), Some(5.0), "typing replaced the selected 1");
        h.key_press(egui::Key::Enter);
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "Enter is OK");
        assert_eq!(h.state().session.journal.last(), Some(&(BLUR.to_string(), json!({"radius": 5.0}))));

        crate::filter_dialog::open(h.state_mut(), BLUR).unwrap();
        h.run_steps(4);
        assert_eq!(radius(&h), Some(5.0), "reopens with the last value");
        assert!(h.get_by_role(egui::accesskit::Role::SpinButton).is_focused(), "focused again");
        h.event(egui::Event::Text("9".into()));
        h.run_steps(1);
        h.key_press(egui::Key::Escape);
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "Esc is Cancel");
        crate::filter_dialog::open(h.state_mut(), BLUR).unwrap();
        h.run_steps(2);
        assert_eq!(radius(&h), Some(5.0), "a cancelled 9 is not remembered");
    }

    /// The phone layout: Preferences, the widest dialog, fits a phone held upright and its OK
    /// stays reachable.
    #[test]
    fn phone_layout_dialogs_fit_the_window() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.compact_layout = true;
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 760.0));
        let mut h = egui_kittest::Harness::builder().with_size(screen.size()).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let mut wide = Vec::new();
        for (section, _) in photocraft_engine::prefs::SECTIONS {
            h.state_mut().ui.dialogs.clear();
            h.run_steps(1);
            let d = crate::prefs_ui::open_preferences(h.state_mut(), section);
            h.run_steps(4);
            let window = h.ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", d)))).expect("the Preferences window");
            if !screen.contains_rect(window) {
                wide.push(format!("{section}: {:.0}", window.width()));
            } else {
                assert!(window.contains_rect(h.get_by_label("OK").rect()), "{section}: OK {:?} inside the window {window:?}", h.get_by_label("OK").rect());
            }
        }
        assert!(wide.is_empty(), "sections wider than the phone's {:.0}: {wide:?}", screen.width());
    }

    /// #4673: File › Automate › Fit Image holds two numbers and a check box, so its window is as
    /// wide as that content and its buttons, not the 380-point floor of the larger dialogs.
    #[test]
    fn fit_image_is_as_narrow_as_its_content() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).build_ui_state(|ui, app| show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        let ctx = h.ctx.clone();
        let d = crate::menus::invoke(h.state_mut(), &ctx, "file.automate.fitImage", json!({})).unwrap()["dialog"].as_u64().unwrap();
        h.run_steps(4);
        let window = h.ctx.memory(|m| m.area_rect(egui::Id::new(("dialog", d)))).expect("the Fit Image window");
        assert!(window.width() < 300.0, "Fit Image is {} points wide", window.width());
        for label in ["Fit Image", "Width", "Height", "OK", "Cancel"] {
            assert!(window.contains_rect(h.get_by_label(label).rect()), "{label} inside the window");
        }
    }
}

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
        let rects: Vec<egui::Rect> = h.ctx.data(|m| m.get_temp(egui::Id::new(RECTS))).unwrap();
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
