use super::inspect::inspect;
use super::params::{
    BUTTONS, KEY_MODIFIERS, MAX_CLICKS, POINTER_MODIFIERS, UI_SET_FIELDS, bool_field, err, f32_field, f32_range, input_choice, input_modifiers, input_number,
    input_object, merged_object, modifiers_from, num_field, ok, screen_point, uint_field, whole_object,
};
use super::*;

pub(super) fn dispatch(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let s = |k: &str| p.get(k).and_then(Value::as_str);
    let u = |k: &str| p.get(k).and_then(Value::as_u64);
    let wait = p.get("wait").and_then(Value::as_bool).unwrap_or(true);
    match req.method.as_str() {
        "ui.context.choose" => {
            let Some(id) = s("id") else { return err("missing `id`") };
            let Some(menu) = app.ui.canvas_tool_menu.as_ref() else { return err("no canvas context menu is open") };
            if !crate::canvas_tool_menu::available(app, menu, id) {
                return err("context action is unavailable");
            }
            // A pasteboard row changes a preference: the policy sees the `prefs.set` it runs.
            let (command, params) = crate::canvas_tool_menu::engine_call(app, menu, id);
            if let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(&command, &params)
            {
                return err(error);
            }
            crate::canvas_tool_menu::choose(app, ctx, id);
            ok(json!({"command": id, "dialog": app.ui.dialogs.last().map(|d| d.id)}))
        }
        "engine.execute" | "ui.menu.invoke" => {
            let Some(id) = s("command").or(s("id")) else { return err("missing `command`") };
            let params = p.get("params").cloned().unwrap_or(json!({}));
            // A control-channel menu click must respect the same modal gate as the native menu.
            // The documented `ui` requests of Camera Raw and the full-window workspaces operate
            // the already-open one rather than click a menu. Only that continuation may pass its
            // own modal gate. engine.execute is deliberately not subject to the UI's menu-click semantics.
            let open_modal = if app.camera_raw.is_some() { Some("filter.cameraRaw") } else { crate::distort_ui::modal_workspace(app) };
            let continuation = open_modal == Some(id)
                && params.as_object().is_some_and(|fields| fields.len() == 1 && fields.get("ui").is_some_and(Value::is_object))
                && app.ui.dialogs.is_empty()
                && app.discard.is_none()
                && !crate::crop_ui::blocks(app, id);
            if req.method == "ui.menu.invoke" && !continuation && !crate::menus::modal_allows(app, id) {
                return err("menu command is unavailable while a modal dialog is open");
            }
            if let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(id, &params)
            {
                return err(error);
            }
            // `engine.execute` is programmatic: engine commands run directly with their default
            // params and never open a dialog (an agent would otherwise get a modal instead of a
            // result). `ui.menu.invoke` behaves like a menu click, so it may open the dialog.
            if req.method == "engine.execute" && photocraft_engine::commands::find(id).is_some() {
                return run_waiting(app, wait, |app| app.run(id, params));
            }
            run_waiting(app, wait, |app| crate::menus::invoke(app, ctx, id, params))
        }
        "engine.commands" => wrap(app.run("command.list", json!({}))),
        // Background jobs (#210): running ones with progress, then the last few that ended.
        "jobs.list" => wrap(app.session.execute("jobs.list", json!({})).map_err(|e| e.to_string())),
        "jobs.cancel" => {
            let all = p.get("job").is_none_or(Value::is_null);
            match p.get("job").and_then(Value::as_u64) {
                Some(j) if app.session.job(photocraft_engine::jobs::JobId(j)).is_some() => {
                    let cancelled = crate::jobs_ui::cancel(app, photocraft_engine::jobs::JobId(j));
                    ok(json!({"cancelled": usize::from(cancelled)}))
                }
                Some(j) => err(format!("no running job {j}")),
                None if all => {
                    let ids: Vec<_> = app.session.jobs().into_iter().map(|j| j.id).collect();
                    let cancelled = ids.into_iter().filter(|j| crate::jobs_ui::cancel(app, *j)).count();
                    ok(json!({"cancelled": cancelled}))
                }
                None => err("`job` must be a job id"),
            }
        }
        "ui.menu.list" => {
            // Report the same enablement the native menu shows, so a listed item is invokable.
            let mut items = crate::menus::menu_items(app);
            for item in &mut items {
                item.enabled &= crate::menus::modal_allows(app, &item.id);
            }
            ok(serde_json::to_value(items).unwrap_or_default())
        }
        "ui.inspect" => ok(inspect(app, ctx)),
        "ui.set" => {
            if let Some(field) = p.as_object().and_then(|o| o.keys().find(|k| !UI_SET_FIELDS.contains(&k.as_str()))) {
                return err(format!("unknown field `{field}` (fields: {})", UI_SET_FIELDS.join(", ")));
            }
            let gradient_blend = if let Some(value) = p.get("gradientBlendMode") {
                let Some(name) = value.as_str() else { return err("gradientBlendMode must be a blend mode name") };
                let Some(mode) = photocraft_engine::commands::blend_from_str(name).filter(|m| photocraft_color::BlendMode::LAYER_MODES.contains(m)) else {
                    return err(format!("unknown gradient blend mode `{name}`"));
                };
                Some(mode)
            } else {
                None
            };
            // Everything is validated and materialized before the first field is applied, so a
            // rejected call applies none of its fields (#412's guarantee, down to the values
            // and nested keys).
            let applied = (|| -> std::result::Result<Value, String> {
                let shape_stroke = match p.get("shapeStroke") {
                    Some(v) => {
                        if let Some(object) = v.as_object() {
                            let keys = ["width", "opacity", "align", "cap", "join", "miterLimit", "dashes", "dashOffset"];
                            if let Some(key) = object.keys().find(|key| !keys.contains(&key.as_str())) {
                                return Err(format!("unknown shapeStroke field `{key}`"));
                            }
                        }
                        let o = &app.ui.tool_options;
                        let base = o.shape_stroke.stroke(o.stroke_width, photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK));
                        Some(photocraft_engine::vector_cmds::parse_stroke(v, Some(base), photocraft_doc::Color::BLACK)?)
                    }
                    None => None,
                };
                let tool = match s("tool") {
                    Some(t) => Tool::from_name(t).map(Some).ok_or_else(|| format!("unknown tool `{t}`"))?,
                    None => None,
                };
                let gradient_classic = bool_field(p, "gradientClassic")?;
                // The stroke latency trace (`stroke_trace`), at runtime: on (stderr) or off. A CSV
                // file is only for the user's own launch (`--stroke-trace <path>`): a path over the
                // control channel would write outside the automation write root.
                let stroke_trace = match p.get("strokeTrace") {
                    None | Some(Value::Null) => None,
                    Some(Value::Bool(b)) => Some(if *b { "stderr" } else { "off" }),
                    Some(Value::String(v)) => match v.trim() {
                        "stderr" | "on" | "true" => Some("stderr"),
                        "off" | "false" => Some("off"),
                        _ => return Err("strokeTrace must be true, false, \"stderr\" or \"off\"; a CSV file is set at launch (--stroke-trace <path>)".into()),
                    },
                    Some(_) => return Err("strokeTrace must be true, false, \"stderr\" or \"off\"".into()),
                };
                // The Eyedropper's options bar (#1649): Sample Size, Sample, Show Sampling Ring.
                let eyedropper_size = match p.get("eyedropperSampleSize") {
                    Some(v) => Some(
                        photocraft_engine::sample_cmds::size_param("ui.set", &json!({"size": v}))
                            .map_err(|_| format!("eyedropperSampleSize must be \"point\" or one of {:?}", photocraft_engine::sample_cmds::SAMPLE_SIZES))?,
                    ),
                    None => None,
                };
                let eyedropper_sample = match p.get("eyedropperSample") {
                    Some(v) => {
                        let ids: Vec<&str> = photocraft_engine::sample_cmds::SampleLayers::ALL.iter().map(|(_, n)| *n).collect();
                        let id = v.as_str().filter(|id| ids.contains(id)).ok_or_else(|| format!("eyedropperSample must be one of {}", ids.join(", ")))?;
                        Some(id.to_string())
                    }
                    None => None,
                };
                let eyedropper_ring = bool_field(p, "eyedropperRing")?;
                // Content-Aware Move's Transform On Drop checkbox.
                let cam_transform_on_drop = bool_field(p, "camTransformOnDrop")?;
                // Developer overlay of the active layer's storage tiles (tile_overlay.rs).
                let tile_grid = bool_field(p, "tileGrid")?;
                let layer_filter = p.get("layerFilter").map(|v| crate::layer_filter_ui::patched(&app.ui, v)).transpose()?;
                // The Crop tool's overlay menu (#1919).
                let crop_overlay = match p.get("cropOverlay") {
                    Some(v) => Some(v.as_str().and_then(crate::crop_overlay::CropOverlay::from_id).ok_or_else(|| {
                        let ids: Vec<&str> = crate::crop_overlay::CropOverlay::ALL.iter().map(|k| k.id()).collect();
                        format!("cropOverlay must be one of {}", ids.join(", "))
                    })?),
                    None => None,
                };
                let crop_overlay_show = match p.get("cropOverlayShow") {
                    Some(v) => Some(
                        v.as_str()
                            .and_then(crate::crop_overlay::OverlayShow::from_id)
                            .ok_or_else(|| "cropOverlayShow must be one of auto, always, never".to_string())?,
                    ),
                    None => None,
                };
                let crop_overlay_orientation = match uint_field(p, "cropOverlayOrientation")? {
                    Some(o) if o < 4 => Some(o as u8),
                    Some(_) => return Err("cropOverlayOrientation must be 0, 1, 2 or 3".into()),
                    None => None,
                };
                // The Crop tool's Fill (#3823): "background" (Background (Default)) or "contentAware".
                let crop_content_aware = match p.get("cropFill") {
                    Some(v) => Some(match v.as_str() {
                        Some("background") => false,
                        Some("contentAware") => true,
                        _ => return Err("cropFill must be \"background\" or \"contentAware\"".into()),
                    }),
                    None => None,
                };
                // The Crop tool's gear menu: Show Cropped Area and the shield (#1919).
                let crop_shield = merged_object(&app.ui.tool_options.crop_shield, p.get("cropShield"), "cropShield")?;
                if crop_shield.as_ref().is_some_and(|s| !(0.0..=100.0).contains(&s.opacity)) {
                    return Err("cropShield.opacity must be 0..100".into());
                }
                // The Histogram panel: its options, the dragged tonal range ([from, to] levels or
                // null) and Uncached Refresh.
                let histogram_panel = merged_object(&app.ui.histogram_panel, p.get("histogramPanel"), "histogramPanel")?;
                let histogram_range = match p.get("histogramRange") {
                    Some(v) => Some(
                        serde_json::from_value::<Option<[u8; 2]>>(v.clone())
                            .map_err(|_| "histogramRange must be [from, to] levels 0..255, or null".to_string())?,
                    ),
                    None => None,
                };
                let histogram_refresh = bool_field(p, "histogramRefresh")?;
                let panels = merged_object(&app.ui.panels, p.get("panels"), "panels")?;
                let mask_target = bool_field(p, "maskTarget")?;
                let updates_open = bool_field(p, "updatesOpen")?;
                let vector_mask_target = bool_field(p, "vectorMaskTarget")?;
                // Target the active smart object's filter mask (FILE-215-3): it needs smart filters.
                let filter_mask_target = bool_field(p, "filterMaskTarget")?;
                if filter_mask_target == Some(true) {
                    let st = app.session.active().ok_or("filterMaskTarget needs an open document")?;
                    let l = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or("filterMaskTarget needs an active layer")?;
                    if !matches!(&l.content, photocraft_doc::LayerContent::Smart(sm) if !sm.smart_filters.is_empty()) {
                        return Err("filterMaskTarget needs an active smart object with smart filters".into());
                    }
                    use photocraft_engine::channel_cmds::ChannelTarget;
                    let composite = matches!(st.channel_view.target, ChannelTarget::Composite | ChannelTarget::LayerMask | ChannelTarget::VectorMask);
                    if !composite || st.doc.quick_mask.is_some() {
                        return Err("filterMaskTarget can't be set while a single channel or Quick Mask is targeted".into());
                    }
                }
                let selection_mode = uint_field(p, "selectionMode")?;
                let dock_tabs = merged_object(&app.ui.dock_tabs, p.get("dockTabs"), "dockTabs")?;
                let dock = whole_object(&app.ui.dock, p.get("dock"), "dock")?;
                // Which chip the Color panel edits.
                let color_panel = merged_object(&app.ui.color_panel, p.get("colorPanel"), "colorPanel")?;
                let dock_width = f32_field(p, "dockWidth")?;
                let zoom = f32_field(p, "zoom")?;
                let rotation = f32_field(p, "rotation")?;
                if rotation.is_some() && app.session.active_index().is_none() {
                    return Err("no document open".into());
                }
                let center = match p.get("center") {
                    Some(v) => {
                        let a = v.as_array().ok_or_else(|| "center must be [x, y]".to_string())?;
                        if a.len() != 2 {
                            return Err("center must be exactly [x, y]".into());
                        }
                        let (Some(x), Some(y)) = (a.first().and_then(Value::as_f64), a.get(1).and_then(Value::as_f64)) else {
                            return Err("center must be [x, y] of two numbers".into());
                        };
                        if !f32_range(&x) || !f32_range(&y) {
                            return Err("center must be finite numbers within f32 range".into());
                        }
                        Some([x as f32, y as f32])
                    }
                    None => None,
                };
                let fit = bool_field(p, "fit")? == Some(true);
                let theme = match s("theme") {
                    Some(name) => {
                        let kind = crate::theme::ThemeKind::from_name(name).ok_or_else(|| {
                            let names: Vec<_> = crate::theme::ThemeKind::ALL.iter().map(|k| k.id()).collect();
                            format!("unknown theme `{name}` ({})", names.join(", "))
                        })?;
                        Some(kind)
                    }
                    None => None,
                };
                let brush_section = uint_field(p, "brushSection")?;
                let brush_tab = uint_field(p, "brushTab")?;
                let brushes_view = match p.get("brushesView") {
                    Some(v) => serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("brushesView: {e} (list, grid)"))?,
                    None => None,
                };
                // The three parts of the picker's cards. At least one stays on (a rejected call
                // leaves them all alone).
                let brush_picker_name = bool_field(p, "brushPickerName")?;
                let brush_picker_stroke = bool_field(p, "brushPickerStroke")?;
                let brush_picker_tip = bool_field(p, "brushPickerTip")?;
                if [brush_picker_name, brush_picker_stroke, brush_picker_tip].iter().any(Option::is_some) {
                    let on = [
                        brush_picker_name.unwrap_or(app.ui.brush_picker_list.show_name),
                        brush_picker_stroke.unwrap_or(app.ui.brush_picker_list.show_stroke),
                        brush_picker_tip.unwrap_or(app.ui.brush_picker_list.show_tip),
                    ];
                    if !on.iter().any(|b| *b) {
                        return Err("brushPickerName, brushPickerStroke and brushPickerTip: at least one must stay on".into());
                    }
                }
                // The picker footer slider: the preset cards' size scale (1 standard; 0.30 and
                // below the tips drop their size numbers).
                let brush_picker_scale = match num_field(p, "brushPickerScale")? {
                    Some(s) if !(0.15..=2.0).contains(&s) => return Err("brushPickerScale must be between 0.15 and 2".into()),
                    Some(s) => Some(s as f32),
                    None => None,
                };
                // `Some(None)` is an explicit null, which closes the picker (a missing field
                // leaves it alone).
                let brush_picker = match p.get("brushPicker") {
                    Some(v) => {
                        let at = serde_json::from_value::<Option<[f32; 2]>>(v.clone())
                            .map_err(|_| "brushPicker must be [x, y] in screen points, or null to close it".to_string())?;
                        if at.is_some_and(|[x, y]| !x.is_finite() || !y.is_finite()) {
                            return Err("brushPicker must be [x, y] in screen points, or null to close it".into());
                        }
                        Some(at)
                    }
                    None => None,
                };
                // brushSize rides the journaled `tools.setBrush` command (#744). Numeric values
                // are validated here so the command cannot fail after other fields were
                // applied; non-numeric values keep the API's historical silent no-op.
                let brush_size: Option<f64> = match p.get("brushSize").and_then(Value::as_f64) {
                    Some(size) => {
                        if !size.is_finite() || size > f64::from(photocraft_paint::MAX_BRUSH_SIZE) {
                            return Err(format!("brushSize must be finite and at most {} px", photocraft_paint::MAX_BRUSH_SIZE));
                        }
                        Some(size)
                    }
                    None => None,
                };

                let bridge_pro = matches!(theme.unwrap_or(app.ui.theme), crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
                let legacy_update = crate::panel_docking::prepare_legacy_update(app, p, panels.as_ref(), dock_tabs.as_ref(), dock.as_ref(), bridge_pro)?;

                // Apply (nothing below can fail).
                if let Some(stroke) = shape_stroke {
                    if let Some(stroke) = stroke {
                        app.ui.tool_options.stroke_width = stroke.width;
                        app.ui.tool_options.shape_stroke = crate::shape_stroke_ui::StrokeOptions::from(&stroke);
                    } else {
                        app.ui.tool_options.stroke_width = 0.0;
                    }
                }
                if let Some(t) = tool {
                    crate::tool_memory::select(app, t);
                    // Each tool keeps its own brush (#218), so switch it in before `brushSize`
                    // below sets the new tool's size.
                    crate::paint_mouse::sync_tool_brush(app);
                }
                let gradient_before = app.ui.tool_options.clone();
                if let Some(mode) = gradient_blend {
                    app.ui.tool_options.gradient_blend_mode = mode;
                }
                if let Some(v) = &stroke_trace {
                    app.stroke_trace.set(v);
                }
                if let Some(classic) = gradient_classic {
                    app.ui.tool_options.gradient_classic = classic;
                }
                if gradient_blend.is_some() {
                    crate::gradient_ui::options_changed(app, &gradient_before);
                }
                if let Some(size) = eyedropper_size {
                    app.ui.tool_options.eyedropper_size = size;
                }
                if let Some(id) = eyedropper_sample {
                    app.ui.tool_options.eyedropper_sample = id;
                }
                if let Some(ring) = eyedropper_ring {
                    app.ui.tool_options.eyedropper_ring = ring;
                }
                if let Some(on) = cam_transform_on_drop {
                    app.ui.tool_options.cam_transform_on_drop = on;
                }
                if let Some(on) = tile_grid {
                    app.ui.view.tile_grid = on;
                }
                if let Some(f) = layer_filter {
                    crate::layer_filter_ui::apply(&mut app.ui, f);
                }
                if let Some(h) = histogram_panel {
                    app.ui.histogram_panel = h;
                    crate::histogram_panel::persist_options(app);
                }
                if let Some(r) = histogram_range {
                    app.ui.histogram_range = r;
                }
                if histogram_refresh == Some(true) {
                    crate::histogram_panel::uncached_refresh(app);
                }
                if let Some(k) = crop_overlay {
                    app.ui.tool_options.crop_overlay = k;
                }
                if let Some(on) = crop_content_aware {
                    app.ui.tool_options.crop_content_aware = on;
                }
                if let Some(v) = crop_overlay_show {
                    app.ui.tool_options.crop_overlay_show = v;
                }
                if let Some(o) = crop_overlay_orientation {
                    app.ui.tool_options.crop_overlay_orientation = o;
                }
                if let Some(s) = crop_shield {
                    app.ui.tool_options.crop_shield = s;
                }
                if let Some(v) = panels {
                    app.ui.panels = v;
                }
                if let Some(m) = mask_target {
                    app.end_filter_mask_view();
                    app.ui.filter_mask_target = None;
                    app.ui.mask_target = m;
                    app.ui.properties_layer = None;
                    app.ui.vector_mask_target &= !m;
                    app.sync_mask_targets();
                }
                if let Some(open) = updates_open {
                    app.ui.updates_open = open;
                }
                if let Some(m) = vector_mask_target {
                    app.end_filter_mask_view();
                    app.ui.filter_mask_target = None;
                    app.ui.vector_mask_target = m;
                    app.ui.mask_target &= !m;
                    app.sync_mask_targets();
                }
                if let Some(m) = filter_mask_target {
                    let layer = if m { app.session.active().and_then(|st| st.active_layer).map(|id| id.0) } else { None };
                    app.select_filter_mask_target(layer);
                }
                // Selection tools' options-bar mode: 0 New, 1 Add, 2 Subtract, 3 Intersect.
                if let Some(m) = selection_mode {
                    app.ui.selection_mode = m.min(3) as u8;
                }
                if let Some(k) = theme {
                    app.set_theme(ctx, k);
                }
                if let Some(v) = dock_tabs {
                    app.ui.dock_tabs = v;
                }
                // Dock group order, heights and collapsed groups (see `dock::DockLayout`).
                if let Some(v) = dock {
                    app.ui.dock = v;
                }
                if let Some(update) = legacy_update {
                    crate::panel_docking::commit_legacy_update(app, update, p);
                }
                if let Some(v) = color_panel {
                    app.ui.color_panel = v;
                }
                // Right dock width in points (clamped to the dock's 250..=520 range), applied next frame.
                if let Some(w) = dock_width {
                    crate::panels::request_dock_width(ctx, w as f32);
                }
                if let Some(i) = app.session.active_index() {
                    if zoom.is_some() || center.is_some() || rotation.is_some() || fit {
                        app.ui.views[i].zoom_animating = false;
                        app.ui.views[i].zoom_anchor = None;
                    }
                    if let Some(z) = zoom {
                        let size = app.session.documents().get(i).map_or([0, 0], |st| [st.doc.size.width, st.doc.size.height]);
                        app.ui.views[i].zoom = crate::zoom_levels::clamp(z as f32, size);
                        app.ui.views[i].fit_pending = false;
                        app.ui.views[i].fill_pending = false;
                    }
                    if let Some(c) = center {
                        app.ui.views[i].center = c;
                        app.ui.views[i].fit_pending = false;
                        app.ui.views[i].fill_pending = false;
                    }
                    if fit {
                        app.ui.views[i].fit_pending = true;
                        app.ui.views[i].fill_pending = false;
                    }
                    if let Some(r) = rotation {
                        app.ui.views[i].rotation = crate::rotate_view::wrap_deg(r as f32);
                    }
                }
                if let Some(i) = brush_section {
                    app.ui.brush_section = (i as usize).min(crate::brush_panel::SECTIONS.len() - 1);
                }
                if let Some(i) = brush_tab {
                    app.ui.brush_tab = (i as usize).min(1);
                }
                if let Some(v) = brushes_view {
                    app.ui.brushes_panel.view = v;
                }
                if let Some(v) = brush_picker_name {
                    app.ui.brush_picker_list.show_name = v;
                }
                if let Some(v) = brush_picker_stroke {
                    app.ui.brush_picker_list.show_stroke = v;
                }
                if let Some(v) = brush_picker_tip {
                    app.ui.brush_picker_list.show_tip = v;
                }
                if let Some(v) = brush_picker_scale {
                    app.ui.brush_picker_list.scale = v;
                }
                if let Some(at) = brush_picker {
                    app.ui.brush_picker = at;
                }
                // Which chip the Color panel edits.
                if let Some(c) = color_panel {
                    app.ui.color_panel = c;
                }
                if let Some(size) = brush_size {
                    app.run("tools.setBrush", json!({"brush": {"size": size}})).map_err(|e| e.to_string())?;
                }
                Ok(Value::Null)
            })();
            wrap(applied)
        }
        "ui.dialog.open" => {
            let kind = match s("kind").unwrap_or("") {
                "newDocument" | "NewDocument" => DialogKind::NewDocument,
                "about" | "About" => DialogKind::About,
                "layerStyle" | "LayerStyle" => {
                    return match crate::layer_style::open(app, s("effect")) {
                        Some(id) => ok(json!({"dialog": id})),
                        None => err("no active layer"),
                    };
                }
                "colorPicker" | "ColorPicker" => {
                    if let Some(style) = p.get("textStyle") {
                        if ["parent", "effect", "field", "target"].iter().any(|key| p.get(*key).is_some()) {
                            return err("textStyle cannot be combined with another picker target");
                        }
                        let Some(style) = style.as_object().filter(|s| s.keys().all(|k| k == "kind" || k == "id")) else {
                            return err("textStyle needs kind and id only");
                        };
                        let Some(kind) = style.get("kind").cloned().and_then(|v| serde_json::from_value::<crate::type_panels_ui::color_picker::Kind>(v).ok())
                        else {
                            return err("textStyle kind must be character or paragraph");
                        };
                        let Some(id) = style.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()) else {
                            return err("textStyle id must be a u32 integer");
                        };
                        return match crate::type_panels_ui::color_picker::open(app, kind, id) {
                            Ok(id) => ok(json!({"dialog": id})),
                            Err(error) => err(error),
                        };
                    }
                    if p.get("parent").is_some() {
                        let (Some(parent), Some(effect), Some(field)) = (u("parent"), s("effect"), s("field")) else {
                            return err("need integer `parent` and string `effect` and `field`");
                        };
                        return match crate::layer_style::color_picker::open(app, parent, effect, field) {
                            Ok(id) => ok(json!({"dialog": id})),
                            Err(error) => err(error),
                        };
                    }
                    if p.get("effect").is_some() || p.get("field").is_some() {
                        return err("Layer Style color pickers need `parent`");
                    }
                    let target = if s("target") == Some("background") { "background" } else { "foreground" };
                    return ok(json!({"dialog": crate::color_picker_ui::open(app, target)}));
                }
                // The Crop tool's New / Delete Crop Preset… (#1919).
                "newCropPreset" => {
                    return match crate::crop_presets::open_new(app) {
                        Some(id) => ok(json!({"dialog": id})),
                        None => err("nothing to save: set W and H (W x H x Resolution) or a ratio first"),
                    };
                }
                "deleteCropPreset" => {
                    return match crate::crop_presets::open_delete(app) {
                        Some(id) => ok(json!({"dialog": id})),
                        None => err("no crop presets are saved"),
                    };
                }
                "command" | "Command" => {
                    let Some(cmd) = s("command") else { return err("command dialogs need `command`") };
                    let label = photocraft_engine::commands::find(cmd).map(|c| c.label).unwrap_or(cmd);
                    return ok(json!({"dialog": crate::dialogs::open_command_dialog(app, cmd, label)}));
                }
                other => return err(format!("unknown dialog kind `{other}`")),
            };
            let mut fields = if kind == DialogKind::NewDocument { app.new_document_fields() } else { Default::default() };
            if let Some(f) = p.get("fields").and_then(Value::as_object) {
                if let Some(marker) = f.keys().find(|k| crate::dialogs::is_route_marker(k)) {
                    return err(format!("`{marker}` is set only by the dialog it belongs to"));
                }
                fields.extend(f.clone());
            }
            ok(json!({"dialog": app.ui.open_dialog(kind, fields)}))
        }
        // The shell's own dialog (Window › Workspace, View › Pixel Aspect Ratio › Custom, Show
        // Extras Options, 32-bit Preview Options), listed by ui.inspect with the id "shell".
        "ui.dialog.set" if s("dialog") == Some(SHELL_DIALOG) => {
            let Some(field) = s("field") else { return err("need `dialog` and `field`") };
            let value = p.get("value").cloned().unwrap_or(Value::Null);
            match app.ui.shell.dialog.as_mut() {
                Some((_, fields)) => {
                    fields.insert(field.to_string(), value);
                    ok(Value::Null)
                }
                None => err("no shell dialog is open"),
            }
        }
        "ui.dialog.confirm" if s("dialog") == Some(SHELL_DIALOG) => {
            let Some((kind, fields)) = app.ui.shell.dialog.clone() else { return err("no shell dialog is open") };
            if let Some((command, params)) = crate::workspace_ui::dialog_command(&kind, &fields)
                && let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(command, &params)
            {
                return err(error);
            }
            wrap(crate::workspace_ui::confirm(app, ctx))
        }
        "ui.dialog.apply" if s("dialog") == Some(SHELL_DIALOG) => err("`ui.dialog.apply` is for Preferences; use `ui.dialog.confirm`"),
        // The unsaved-changes prompt (closing, Revert, quitting), listed by ui.inspect with the
        // id "discard" (#4920): confirm takes a `choice` (save, dontSave, cancel; default save).
        "ui.dialog.confirm" if s("dialog") == Some(crate::discard_ui::CONTROL_ID) => {
            let choice = s("choice").unwrap_or("save");
            if choice != "cancel"
                && let Some((command, params)) = crate::discard_ui::pending_command(app)
                && let Some(authorize) = app.services.automation_command.as_ref()
                && let Err(error) = authorize(&command, &params)
            {
                return err(error);
            }
            wrap(crate::discard_ui::answer_control(app, ctx, choice))
        }
        "ui.dialog.cancel" if s("dialog") == Some(crate::discard_ui::CONTROL_ID) => wrap(crate::discard_ui::answer_control(app, ctx, "cancel")),
        "ui.dialog.set" | "ui.dialog.apply" if s("dialog") == Some(crate::discard_ui::CONTROL_ID) => {
            err("the unsaved-changes prompt has no fields; use `ui.dialog.confirm` with a `choice`")
        }
        "ui.dialog.cancel" if s("dialog") == Some(SHELL_DIALOG) => match app.ui.shell.dialog.take() {
            Some(_) => ok(Value::Null),
            None => err("no such dialog"),
        },
        "ui.dialog.set" => {
            let (Some(id), Some(field)) = (u("dialog"), s("field")) else { return err("need `dialog` and `field`") };
            let value = p.get("value").cloned().unwrap_or(Value::Null);
            match app.ui.dialog_mut(id) {
                // A continuation belongs to the dialog that paused it, even when another
                // action has a valid token. Replacing it would resume or cancel that action.
                Some(_) if field == crate::actions::RESUME => err("the paused action token is read-only"),
                // A marker picks the handler OK runs: a dialog keeps the ones it was opened with.
                Some(d) if !d.fields.contains_key(field) && crate::dialogs::is_route_marker(field) => {
                    err(format!("`{field}` is set only by the dialog it belongs to"))
                }
                Some(d) => {
                    d.fields.insert(field.to_string(), value);
                    ok(Value::Null)
                }
                None => err(format!("no dialog {id}")),
            }
        }
        "ui.dialog.confirm" | "ui.dialog.apply" => match u("dialog") {
            Some(id) => {
                let command = app.ui.dialogs.iter().find(|dialog| dialog.id == id).and_then(|dialog| {
                    if req.method == "ui.dialog.apply" {
                        dialog.fields.get("values").map(|values| ("prefs.set".to_string(), json!({"path": "", "value": values})))
                    } else {
                        dialog.fields.get("__command").and_then(Value::as_str).map(|command| (command.to_string(), Value::Object(dialog.fields.clone())))
                    }
                });
                if let Some((command, params)) = command
                    && let Some(authorize) = app.services.automation_command.as_ref()
                    && let Err(error) = authorize(&command, &params)
                {
                    return err(error);
                }
                let apply = req.method == "ui.dialog.apply";
                run_waiting(app, wait, |app| if apply { crate::prefs_ui::apply(app, id) } else { crate::dialogs::confirm(app, id) })
            }
            None => err("missing `dialog`"),
        },
        "ui.dialog.cancel" => match u("dialog") {
            Some(id) => match crate::dialogs::cancel(app, id) {
                Ok(value) => ok(value),
                Err(error) => err(error),
            },
            None => err("missing `dialog`"),
        },
        "ui.window.open" => {
            if let Some(d) = u("document")
                && !app.session.set_active(d as usize)
            {
                return err(format!("no document {d}"));
            }
            wrap(crate::menus::invoke(app, ctx, "window.newWindowForDocument", json!({})))
        }
        "ui.window.close" => {
            let Some(id) = u("window") else { return err("missing `window`") };
            let before = app.ui.windows.len();
            app.ui.windows.retain(|w| w.id != id);
            if app.ui.windows.len() < before { ok(Value::Null) } else { err(format!("no window {id}")) }
        }
        "ui.pointer" => {
            // Every field and event is validated before the first event runs, so a rejected call
            // changes nothing (#2449).
            let parsed = (|| -> std::result::Result<_, String> {
                let fields = ["events", "tool", "modifiers", "button", "shift", "alt", "command", "ctrl", "space"];
                let p = input_object(p, None, &fields)?;
                let tool = match input_choice(&p, "tool", "tool", &[])? {
                    Some(t) => Some(Tool::from_name(t).ok_or_else(|| format!("unknown tool `{t}`"))?),
                    None => None,
                };
                let button = input_choice(&p, "button", "button", &BUTTONS)?.unwrap_or("left").to_string();
                // Modifier flags may be top-level or grouped under "modifiers".
                let m = input_modifiers(&p, &POINTER_MODIFIERS)?;
                let Some(events) = p.get("events") else { return Err("missing `events`".into()) };
                let Some(events) = events.as_array() else { return Err("events must be an array".into()) };
                let mut parsed = Vec::with_capacity(events.len());
                for (i, e) in events.iter().enumerate() {
                    let name = format!("events[{i}]");
                    let e = input_object(e, Some(&name), &["kind", "x", "y", "pressure", "tiltX", "tiltY", "rotation"])?;
                    let num = |k: &str, range| input_number(&e, k, &format!("{name}.{k}"), range);
                    let (x, y) = (num("x", None)?.unwrap_or(0.0), num("y", None)?.unwrap_or(0.0));
                    let pr = num("pressure", Some((0.0, 1.0)))?.unwrap_or(1.0) as f32;
                    let ev = match input_choice(&e, "kind", &format!("{name}.kind"), &["down", "move", "up"])?.unwrap_or("move") {
                        "down" => ToolEvent::Down { x, y, pressure: pr },
                        "up" => ToolEvent::Up { x, y },
                        _ => ToolEvent::Move { x, y, pressure: pr },
                    };
                    let tilt = |k: &str, range| num(k, Some(range)).map(|v| v.map(|v| v as f32));
                    let pen = (tilt("tiltX", (-90.0, 90.0))?, tilt("tiltY", (-90.0, 90.0))?, tilt("rotation", (0.0, 360.0))?);
                    parsed.push((ev, x, y, pr, pen));
                }
                Ok((tool, button, m, parsed))
            })();
            let (tool, button, m, events) = match parsed {
                Ok(parsed) => parsed,
                Err(e) => return err(e),
            };
            let secondary = matches!(button.as_str(), "secondary" | "right");
            let mods = modifiers_from(&m);
            if let Some(t) = tool {
                crate::tool_memory::select(app, t);
            }
            // Space held: the Crop tool moves the frame being drawn, a marquee, lasso or shape
            // being drawn moves instead of growing (hold_keys.rs). Like a modifier flag, it may be
            // top-level or grouped under "modifiers".
            let space = p.get("modifiers").unwrap_or(p).get("space").and_then(Value::as_bool).unwrap_or(false);
            crate::crop_ui::set_space(app, space);
            for (ev, x, y, pr, pen) in events {
                // With the Color Picker on top the image is its eyedropper, as for the mouse.
                if crate::color_picker_ui::top(app).is_some() {
                    if !matches!(ev, ToolEvent::Up { .. }) {
                        crate::color_picker_ui::sample_at(app, x, y);
                    }
                    continue;
                }
                // Curves' modal eyedroppers use document coordinates here, matching the native
                // canvas path after its ViewXform conversion. Sample on press only.
                if !secondary && crate::adjust_dialog::picker_armed(app) {
                    if matches!(ev, ToolEvent::Down { .. }) {
                        crate::adjust_dialog::sample_at(app, x, y);
                    }
                    continue;
                }
                // ⌘/Ctrl-click with Curves open adds a point at the pixel's tone, on press only (#7325).
                if !secondary && !space && button != "middle" && mods.command && crate::adjust_dialog::curves_on_top(app) {
                    if matches!(ev, ToolEvent::Down { .. }) {
                        crate::adjust_dialog::add_point_at(app, ctx, x, y, mods.shift);
                    }
                    continue;
                }
                // A Properties Levels/Curves eyedropper owns the primary button the same way.
                if app.ui.dialogs.is_empty() && !matches!(s("button"), Some("secondary" | "right")) && crate::layer_picker::armed(app).is_some() {
                    if matches!(ev, ToolEvent::Down { .. }) {
                        crate::layer_picker::sample_at(app, x, y);
                    }
                    continue;
                }
                // The dialog owns all pointer events, including Up: no underlying brush stroke
                // or context menu may leak through while inspecting a Color Range preview.
                if app.ui.dialogs.last().is_some_and(|d| crate::color_range_ui::owns(&d.fields)) {
                    let down = matches!(ev, ToolEvent::Down { .. });
                    let up = matches!(ev, ToolEvent::Up { .. });
                    let held = app.ui.dialogs.last().and_then(|d| d.fields.get("__pointerDown")).and_then(Value::as_bool).unwrap_or(false);
                    if let Some(d) = app.ui.dialogs.last_mut() {
                        d.fields.insert("__pointerDown".into(), json!(!up && (down || held)));
                    }
                    if !up && (down || held) && !space && !secondary && button != "middle" {
                        crate::color_range_ui::pick_top(app, [x, y], mods);
                    }
                    continue;
                }
                if secondary {
                    let down = matches!(ev, ToolEvent::Down { .. });
                    // Right-click while transforming: the box's menu, as on the canvas.
                    if app.ui.transform.as_ref().is_some_and(|t| t.warp.is_none()) {
                        if down {
                            crate::canvas_tool_menu::open_transform(app, ctx, screen_point(app, x, y));
                        }
                        continue;
                    }
                    // Right-click with the Move tool, or ⌘/Ctrl+right-click: list the layers there.
                    if crate::layer_pick_ui::is_gesture(app.ui.tool, mods) {
                        if down {
                            app.ui.canvas_tool_menu = None;
                            crate::layer_pick_ui::open(app, ctx, screen_point(app, x, y), x, y);
                        }
                        continue;
                    }
                    // No menu over the agent's own crop frame or Straighten line still being
                    // dragged, also the first one on the default frame (nothing pending yet).
                    if app.ui.tool == crate::state::Tool::Crop && (app.crop.drag.is_some() || app.crop.straighten.line.is_some()) {
                        continue;
                    }
                    if app.ui.tool == crate::state::Tool::Crop && crate::crop_ui::pending(app) {
                        if down {
                            crate::canvas_tool_menu::open_crop(app, ctx, app.ui.tool, screen_point(app, x, y));
                        }
                        continue;
                    }
                    if crate::canvas_tool_menu::pasteboard_at(app, app.ui.tool, [x, y]) {
                        if down {
                            crate::canvas_tool_menu::open_pasteboard(app, ctx, app.ui.tool, screen_point(app, x, y));
                        }
                        continue;
                    }
                    if crate::canvas_tool_menu::is_view_tool(app.ui.tool) {
                        if down {
                            crate::canvas_tool_menu::open_view(app, ctx, app.ui.tool, screen_point(app, x, y), [x, y]);
                        }
                        continue;
                    }
                    if crate::canvas_tool_menu::applies(app.ui.tool)
                        || crate::canvas_tool_menu::editing_text(app, app.ui.tool)
                        || crate::canvas_tool_menu::type_layer_at(app, app.ui.tool, [x, y]).is_some()
                    {
                        if down {
                            crate::canvas_tool_menu::open_at(app, ctx, app.ui.tool, screen_point(app, x, y), Some([x, y]));
                        }
                        continue;
                    }
                    if !crate::paint_mouse::pointer_secondary(app, ctx, down, mods, screen_point(app, x, y)) {
                        continue;
                    }
                }
                // A simulated pen: tilt/rotation reach the stroke like a real stylus's (see `stylus`).
                if pen != (None, None, None) {
                    let (tilt_x, tilt_y, rotation) = (pen.0.unwrap_or(0.0), pen.1.unwrap_or(0.0), pen.2.unwrap_or(0.0));
                    app.stylus.feed.set(Some(crate::stylus::PenSample { pressure: pr, tilt_x, tilt_y, rotation, eraser: false }));
                }
                if let Some(d) = app.drag.as_mut().filter(|d| crate::hold_keys::repositions(d.tool)) {
                    d.reposition = space;
                }
                // A Crop drag in the default mode turns and pans the view with the image: its
                // points are read as the view showed them at the press (`crop_mode`).
                let ev = match ev {
                    ToolEvent::Move { x, y, pressure } => {
                        let [x, y] = crate::crop_mode::control_point(app, [x, y]);
                        ToolEvent::Move { x, y, pressure }
                    }
                    ToolEvent::Up { x, y } => {
                        let [x, y] = crate::crop_mode::control_point(app, [x, y]);
                        ToolEvent::Up { x, y }
                    }
                    down => down,
                };
                tool_event(app, ev, mods);
            }
            app.stylus.feed.set(None);
            ok(json!({"status": app.ui.status}))
        }
        "ui.click" | "ui.move" => {
            let click = req.method == "ui.click";
            let fields: &[&str] = if click { &["x", "y", "button", "count"] } else { &["x", "y"] };
            let parsed = (|| -> std::result::Result<_, String> {
                let p = input_object(p, None, fields)?;
                // Screen coordinates in points (as reported by ui.inspect window size).
                let x = input_number(&p, "x", "x", None)?.unwrap_or(0.0) as f32;
                let y = input_number(&p, "y", "y", None)?.unwrap_or(0.0) as f32;
                let button = match input_choice(&p, "button", "button", &BUTTONS)?.unwrap_or("left") {
                    "right" | "secondary" => egui::PointerButton::Secondary,
                    "middle" => egui::PointerButton::Middle,
                    _ => egui::PointerButton::Primary,
                };
                let clicks = if click { uint_field(&p, "count")?.unwrap_or(1) } else { 0 };
                if clicks > MAX_CLICKS {
                    return Err(format!("`count` must be at most {MAX_CLICKS} (got {clicks})"));
                }
                Ok((egui::pos2(x, y), button, clicks))
            })();
            let (pos, button, clicks) = match parsed {
                Ok(parsed) => parsed,
                Err(e) => return err(e),
            };
            app.synthetic.push(egui::Event::PointerMoved(pos));
            for _ in 0..clicks {
                app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: true, modifiers: Default::default() });
                app.synthetic.push(egui::Event::PointerButton { pos, button, pressed: false, modifiers: Default::default() });
            }
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.key" => {
            let parsed = (|| -> std::result::Result<_, String> {
                let p = input_object(p, None, &["key", "modifiers", "shift", "alt", "command", "ctrl"])?;
                let Some(name) = input_choice(&p, "key", "key", &[])? else { return Err("missing `key`".into()) };
                let key = egui::Key::from_name(name).ok_or_else(|| format!("unknown key `{name}`"))?;
                // Modifier flags may be top-level or grouped under "modifiers".
                Ok((key, input_modifiers(&p, &KEY_MODIFIERS)?))
            })();
            let (key, m) = match parsed {
                Ok(parsed) => parsed,
                Err(e) => return err(e),
            };
            let modifiers = modifiers_from(&m);
            app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers });
            app.synthetic.push(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers });
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.type" => {
            let Some(text) = s("text") else { return err("missing `text`") };
            app.synthetic.push(egui::Event::Text(text.to_string()));
            ctx.request_repaint();
            Outcome::AfterInput
        }
        "ui.resize" => {
            let (w, h) = (p.get("width").and_then(Value::as_f64).unwrap_or(1280.0), p.get("height").and_then(Value::as_f64).unwrap_or(800.0));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w as f32, h as f32)));
            ok(Value::Null)
        }
        "ui.gpu.simulateLoss" => {
            if p.get("destroy").and_then(Value::as_bool).unwrap_or(false) {
                let destroyed = app.destroy_window_device();
                ctx.request_repaint();
                return ok(json!({"destroyed": destroyed}));
            }
            let error = p.get("error").and_then(Value::as_bool).unwrap_or(false);
            let fault = if error {
                photocraft_gpu::Fault::Error("simulated error (ui.gpu.simulateLoss)".into())
            } else {
                photocraft_gpu::Fault::Lost("simulated (ui.gpu.simulateLoss)".into())
            };
            let active = match app.gpu_health() {
                Some(h) => {
                    h.mark(fault);
                    true
                }
                None => false,
            };
            app.check_gpu(ctx);
            ctx.request_repaint();
            ok(json!({"wasActive": active, "gpuInfo": app.perf.gpu_info}))
        }
        "ui.focus" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.screenshot" => {
            let parsed = input_object(p, None, &["path", "focus"])
                .and_then(|p| Ok((input_choice(&p, "path", "path", &[])?.map(str::to_string), bool_field(&p, "focus")?)));
            let (path, focus) = match parsed {
                Ok(parsed) => parsed,
                Err(e) => return err(e),
            };
            // Occluded windows don't render on macOS, so the screenshot would never arrive: raise first.
            if focus.unwrap_or(true) {
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            }
            // The capture itself is admitted by `PhotocraftApp::drain_control` (budget and
            // geometry) and issued from a later `App::ui` pass once open/close animations
            // (modals, popups) have settled (see `screenshot.rs`).
            let token = app.ui.alloc_id();
            ctx.request_repaint();
            Outcome::Screenshot { token, path }
        }
        "app.open" => match s("path") {
            Some(path) => {
                // Directory .pcraft (the native on-disk form) loads through a capability-scoped
                // source; the single-file (zip) form falls back to the bytes path, as do all
                // other formats.
                if path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case(photocraft_format::EXTENSION))
                    && let Some(open) = app.services.automation_open_pcraft.as_mut()
                {
                    match open(path) {
                        Ok(Some((name, doc))) => {
                            app.session.add_document(doc, None);
                            // Recent files, but no save-back target: the capability writer
                            // replaces files, never directories.
                            app.push_recent(path);
                            app.sync_views();
                            app.ui.status = crate::i18n::fmt(tl!("Opened {name}"), &[("name", &name)]);
                            app.ui.status_error = false;
                            return ok(json!({"path": path, "name": name, "warnings": []}));
                        }
                        Ok(None) => {} // a regular file: the bytes path reads the zip form
                        Err(e) => return err(e),
                    }
                }
                let opened = match app.services.automation_read.as_mut() {
                    Some(read) => read(path),
                    None => Err("automation read authority is not configured".into()),
                };
                wrap(opened.and_then(|(name, bytes)| {
                    let name = app.open_name(&name);
                    let warnings = app.open_automation_bytes(&name, &bytes)?;
                    // Brushes/gradients go to the preset libraries: no document, no Open Recent entry.
                    if !crate::preset_files_ui::is_preset_file(&name) {
                        app.opened_from(path);
                    }
                    Ok(json!({"path": path, "name": name, "warnings": warnings}))
                }))
            }
            None => err("missing `path`"),
        },
        "app.save" => match crate::FormatChoices::from_params(p, "app.save") {
            Ok(choices) => {
                let path = s("path").map(str::to_string);
                run_waiting(app, p.get("wait").and_then(Value::as_bool).unwrap_or(true), move |app| app.save_automation_with(path, choices))
            }
            Err(e) => err(e),
        },
        "app.quit" => {
            app.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ok(Value::Null)
        }
        other => err(format!("unknown method `{other}`")),
    }
}
