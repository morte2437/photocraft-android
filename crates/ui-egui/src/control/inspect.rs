use super::*;

/// The open canvas context menu's rows for `ui.state`: the spelling suggestions first (type edit
/// menu over a misspelled word), then the menu's own rows.
fn context_entries(app: &PhotocraftApp, menu: &crate::canvas_tool_menu::CanvasToolMenu) -> Vec<Value> {
    use crate::canvas_tool_menu::{crop_rows, entry_enabled, rows, spelling_rows};
    let spelling = spelling_rows(menu);
    let mut entries: Vec<Value> = spelling.iter().map(|(label, id)| json!({"label": label, "id": id, "enabled": entry_enabled(app, menu, id)})).collect();
    if !spelling.is_empty() {
        entries.push(json!({"separator": true}));
    }
    entries.extend(crop_rows(menu).iter().map(|row| match row {
        Some((label, id)) => json!({"label": label, "id": id, "enabled": entry_enabled(app, menu, id)}),
        None => json!({"separator": true}),
    }));
    entries.extend(rows(menu).iter().map(|row| match row {
        Some((label, id)) => json!({"label": label, "id": id, "enabled": entry_enabled(app, menu, id)}),
        None => json!({"separator": true}),
    }));
    entries
}

/// Snapshot of everything on screen, addressable by id.
pub fn inspect(app: &PhotocraftApp, ctx: &egui::Context) -> Value {
    let screen = ctx.content_rect();
    let mut dialogs: Vec<Value> =
        app.ui.dialogs.iter().map(|d| json!({"id": d.id, "kind": d.kind, "title": crate::dialogs::title(d), "fields": d.fields})).collect();
    if let Some((kind, fields)) = &app.ui.shell.dialog {
        dialogs.push(json!({"id": SHELL_DIALOG, "kind": kind, "title": crate::workspace_ui::title(kind), "fields": fields}));
    }
    dialogs.extend(crate::discard_ui::inspect(app));
    json!({
        "window": {"width": screen.width(), "height": screen.height(), "pixelsPerPoint": ctx.pixels_per_point()},
        "tool": app.ui.tool,
        "toolOptions": app.ui.tool_options,
        "perspectiveCrop": app.ui.perspective_crop,
        "strokeEditor": app.ui.stroke_editor,
        "magnetic": app.ui.magnetic,
        "textEdit": app.ui.text_edit,
        "typeTransform": app.ui.type_transform,
        "camTransform": app.ui.cam_transform,
        "layerMenu": app.ui.layer_menu,
        "layerPicker": app.ui.layer_picker,
        "brushModeMenu": app.ui.brush_mode_menu.map(|pos| json!({
            "pos": pos,
            "mode": if app.ui.tool_options.brush_clear { "Clear" } else { app.session.tools.brush.mode.label() },
            "choices": crate::brush_mode_menu::choices().iter().map(|(_, label)| *label).collect::<Vec<_>>(),
        })),
        "brushPicker": app.ui.brush_picker.map(|pos| json!({
            "pos": pos,
            // The picker's content size once its corner grip was dragged (null: the default).
            "size": app.ui.brush_picker_size,
            "list": {
                "collapsed": app.ui.brush_picker_list.collapsed,
                "filter": app.ui.brush_picker_list.filter,
                "renaming": app.ui.brush_picker_list.renaming,
                // The card's parts (`view` is the Brushes panel's listing and doesn't apply).
                "showName": app.ui.brush_picker_list.show_name,
                "showStroke": app.ui.brush_picker_list.show_stroke,
                "showTip": app.ui.brush_picker_list.show_tip,
                // The footer slider: the cards' width scale.
                "scale": app.ui.brush_picker_list.scale,
            },
        })),
        "canvasToolMenu": app.ui.canvas_tool_menu.as_ref().map(|menu| {
            json!({
                "pos": menu.pos,
                "tool": menu.tool,
                "pasteboard": menu.pasteboard,
                "guide": menu.guide,
                "crop": menu.crop.is_some(),
                "entries": context_entries(app, menu)
            })
        }),
        "panels": app.ui.panels,
        "dockTabs": app.ui.dock_tabs,
        "dock": app.ui.dock,
        "docking": app.ui.docking,
        "dockingReturns": app.ui.docking_returns,
        "dockingCollapsed": app.ui.docking_collapsed,
        "view": app.ui.view,
        "views": app.ui.views,
        "dialogs": dialogs,
        "windows": app.ui.windows,
        "theme": app.ui.theme,
        "status": app.ui.status,
        "statusError": app.ui.status_error,
        "notices": app.ui.notices,
        "updatesOpen": app.ui.updates_open,
        "updates": {"supported": app.session.updates.check.is_some(), "currentVersion": photocraft_engine::build_info::VERSION, "result": app.session.updates.status},
        "gpuFallbackNotice": app.ui.gpu_fallback_notice,
        "kysOffer": app.ui.kys_offer,
        "frame": app.frame,
        "session": photocraft_engine::inspect::session(&app.session),
        "document": app.session.active().map(photocraft_engine::inspect::document),
        "perf": {
            "fps": app.fps,
            "timings": app.perf,
            "strokeTrace": app.stroke_trace.last,
            "renderPath": crate::render_path::inspect(app),
            "slowFrames": app.slow_frames,
        },
        "brush": {"size": app.session.tools.brush.size, "hardness": app.session.tools.brush.hardness, "opacity": app.session.tools.brush.opacity},
        "distort": app.distort.describe(),
        "jobs": crate::jobs_ui::inspect(app),
        "histogram": crate::histogram_panel::inspect(app),
        "cameraRaw": app.camera_raw.as_ref().map(|d| d.describe(&app.ui.camera_raw_scope, &app.ui.camera_raw_preview)),
    })
}

pub fn save_screenshot(app: &mut PhotocraftApp, image: &egui::ColorImage, path: Option<&str>) -> Value {
    let [w, h] = image.size;
    let rgba: Vec<u8> = image.pixels.iter().flat_map(|c| c.to_array()).collect();
    save_screenshot_rgba(app, w, h, &rgba, path)
}

/// Encode already-flattened RGBA pixels. The capture lifecycle uses this after dropping the
/// retained [`egui::ColorImage`], so the encoder's input copy and the retained image do not
/// overlap in memory (see `PhotocraftApp::finish_captures`).
pub fn save_screenshot_rgba(app: &mut PhotocraftApp, w: usize, h: usize, rgba: &[u8], path: Option<&str>) -> Value {
    let png = match app.services.encode_png.as_ref() {
        Some(enc) => enc(w as u32, h as u32, rgba),
        None => Err("no PNG encoder configured".into()),
    };
    let Some(path) = path else {
        return match png {
            Ok(bytes) => json!({
                "ok": true,
                "result": {
                    "base64": base64::engine::general_purpose::STANDARD.encode(bytes),
                    "mime": "image/png",
                    "width": w,
                    "height": h,
                }
            }),
            Err(error) => json!({"ok": false, "error": error}),
        };
    };
    let result = png.and_then(|bytes| match app.services.automation_write.as_mut() {
        Some(write) => write(path, &bytes),
        None => Err("automation write authority is not configured".into()),
    });
    match result {
        Ok(()) => json!({"ok": true, "result": {"path": path, "width": w, "height": h}}),
        Err(e) => json!({"ok": false, "error": e}),
    }
}
