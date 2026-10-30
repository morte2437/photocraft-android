use super::METHODS;
use super::inspect::inspect;
use super::params::{
    BUTTONS, KEY_MODIFIERS, MAX_CLICKS, POINTER_MODIFIERS, UI_SET_FIELDS, bool_field, err, f32_field, f32_range, input_choice, input_modifiers, input_number,
    input_object, merged_object, modifiers_from, num_field, ok, screen_point, uint_field, whole_object,
};
use super::{ControlRequest, DialogKind, Outcome, PhotocraftApp, SHELL_DIALOG, Tool, ToolEvent, Value, json, run_waiting, tool_event, wrap};

/// A domain handler: `Some` when it owns the request's method, `None` to let the next one try.
type Handler = fn(&mut PhotocraftApp, &egui::Context, &ControlRequest) -> Option<Outcome>;

pub(super) fn dispatch(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    const HANDLERS: [Handler; 5] = [engine_methods, shell_methods, dialog_methods, input_methods, app_methods];
    HANDLERS.iter().find_map(|handler| handler(app, ctx, req)).unwrap_or_else(|| err(photocraft_engine::command_hints::unknown_method(&req.method, METHODS)))
}

fn str_param<'a>(p: &'a Value, key: &str) -> Option<&'a str> {
    p.get(key).and_then(Value::as_str)
}

fn u64_param(p: &Value, key: &str) -> Option<u64> {
    p.get(key).and_then(Value::as_u64)
}

fn wait_param(p: &Value) -> bool {
    p.get("wait").and_then(Value::as_bool).unwrap_or(true)
}

// ---- engine commands, menus and jobs ----

fn engine_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Outcome> {
    Some(match req.method.as_str() {
        "ui.context.choose" => context_choose(app, ctx, &req.params),
        "engine.execute" | "ui.menu.invoke" => execute_command(app, ctx, req),
        "engine.commands" => wrap(app.run("command.list", json!({}))),
        // Background jobs (#210): running ones with progress, then the last few that ended.
        "jobs.list" => wrap(app.session.execute("jobs.list", json!({})).map_err(|e| e.to_string())),
        "jobs.cancel" => jobs_cancel(app, &req.params),
        "ui.menu.list" => {
            // Report the same enablement the native menu shows, so a listed item is invokable.
            let mut items = crate::menus::menu_items(app);
            for item in &mut items {
                item.enabled &= crate::menus::modal_allows(app, &item.id);
            }
            ok(serde_json::to_value(items).unwrap_or_default())
        }
        "ui.inspect" => ok(inspect(app, ctx)),
        _ => return None,
    })
}

fn context_choose(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Outcome {
    let Some(id) = str_param(p, "id") else { return err("missing `id`") };
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

fn execute_command(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Outcome {
    let p = &req.params;
    let Some(id) = str_param(p, "command").or(str_param(p, "id")) else { return err("missing `command`") };
    if req.method == "engine.execute" && METHODS.contains(&id) && photocraft_engine::commands::find(id).is_none() {
        return err(photocraft_engine::command_hints::protocol_method_as_command(id));
    }
    let params = p.get("params").cloned().unwrap_or(json!({}));
    let wait = wait_param(p);
    // A control-channel menu click must respect the same modal gate as the native menu.
    // The documented `ui` requests of Camera Raw and the full-window workspaces operate
    // the already-open one rather than click a menu. Only that continuation may pass its
    // own modal gate. Both command entry points respect document ownership.
    let open_modal = if app.camera_raw.is_some() { Some("filter.cameraRaw") } else { crate::distort_ui::modal_workspace(app) };
    let continuation = open_modal == Some(id)
        && params.as_object().is_some_and(|fields| fields.len() == 1 && fields.get("ui").is_some_and(Value::is_object))
        && app.ui.dialogs.is_empty()
        && app.discard.is_none()
        && !crate::crop_ui::blocks(app, id);
    if !continuation && !crate::menus::modal_allows(app, id) {
        return err("command is unavailable while a modal dialog is open");
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

fn jobs_cancel(app: &mut PhotocraftApp, p: &Value) -> Outcome {
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

// ---- the shell: view state, windows, GPU, focus, screenshots ----

fn shell_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Outcome> {
    let p = &req.params;
    Some(match req.method.as_str() {
        "ui.set" => ui_set(app, ctx, p),
        "ui.window.open" => {
            if let Some(d) = u64_param(p, "document")
                && !app.session.set_active(d as usize)
            {
                return Some(err(format!("no document {d}")));
            }
            wrap(crate::menus::invoke(app, ctx, "window.newWindowForDocument", json!({})))
        }
        "ui.window.close" => {
            let Some(id) = u64_param(p, "window") else { return Some(err("missing `window`")) };
            let before = app.ui.windows.len();
            app.ui.windows.retain(|w| w.id != id);
            if app.ui.windows.len() < before { ok(Value::Null) } else { err(format!("no window {id}")) }
        }
        "ui.resize" => {
            let (w, h) = (p.get("width").and_then(Value::as_f64).unwrap_or(1280.0), p.get("height").and_then(Value::as_f64).unwrap_or(800.0));
            ctx.send_viewport_cmd(egui::ViewportCommand::InnerSize(egui::vec2(w as f32, h as f32)));
            ok(Value::Null)
        }
        "ui.gpu.simulateLoss" => simulate_gpu_loss(app, ctx, p),
        "ui.focus" => {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            ctx.request_repaint();
            ok(Value::Null)
        }
        "ui.screenshot" => screenshot(app, ctx, p),
        _ => return None,
    })
}

fn simulate_gpu_loss(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Outcome {
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

fn screenshot(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Outcome {
    let parsed =
        input_object(p, None, &["path", "focus"]).and_then(|p| Ok((input_choice(&p, "path", "path", &[])?.map(str::to_string), bool_field(&p, "focus")?)));
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

// ---- ui.set: every field is validated and materialized into a `UiSet` before the first one is applied ----

fn ui_set(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Outcome {
    if let Some(field) = p.as_object().and_then(|o| o.keys().find(|k| !UI_SET_FIELDS.contains(&k.as_str()))) {
        return err(format!("unknown field `{field}` (fields: {})", UI_SET_FIELDS.join(", ")));
    }
    // Everything is validated and materialized before the first field is applied, so a
    // rejected call applies none of its fields (#412's guarantee, down to the values
    // and nested keys).
    wrap(UiSet::parse(app, p).and_then(|set| set.apply(app, ctx, p)))
}

struct UiSet {
    tools: ToolSet,
    crop: CropSet,
    panels: PanelSet,
    view: ViewSet,
    theme: Option<crate::theme::ThemeKind>,
    brush: BrushSet,
    legacy_update: Option<crate::panel_docking::LegacyUpdate>,
}

impl UiSet {
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        let tools = ToolSet::parse(app, p)?;
        let crop = CropSet::parse(app, p)?;
        let panels = PanelSet::parse(app, p)?;
        let view = ViewSet::parse(app, p)?;
        let theme = theme_param(p)?;
        let brush = BrushSet::parse(app, p)?;
        let bridge_pro = matches!(theme.unwrap_or(app.ui.theme), crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
        let legacy_update =
            crate::panel_docking::prepare_legacy_update(app, p, panels.panels.as_ref(), panels.dock_tabs.as_ref(), panels.dock.as_ref(), bridge_pro)?;
        Ok(Self { tools, crop, panels, view, theme, brush, legacy_update })
    }

    /// Apply (nothing but the final `brushSize` command can fail).
    fn apply(mut self, app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> std::result::Result<Value, String> {
        self.tools.apply(app);
        self.crop.apply(app);
        self.panels.apply_options(app);
        if let Some(k) = self.theme {
            app.set_theme(ctx, k);
        }
        self.panels.apply_dock(app);
        if let Some(update) = self.legacy_update {
            crate::panel_docking::commit_legacy_update(app, update, p);
        }
        self.panels.apply_color_and_width(app, ctx);
        self.view.apply(app);
        self.brush.apply(app)?;
        Ok(Value::Null)
    }
}

fn theme_param(p: &Value) -> std::result::Result<Option<crate::theme::ThemeKind>, String> {
    let Some(name) = str_param(p, "theme") else { return Ok(None) };
    crate::theme::ThemeKind::from_name(name).map(Some).ok_or_else(|| {
        let names: Vec<_> = crate::theme::ThemeKind::ALL.iter().map(|k| k.id()).collect();
        format!("unknown theme `{name}` ({})", names.join(", "))
    })
}

// -- tools: stroke, tool, gradient, eyedropper, Content-Aware Move, tile grid, layer filter --

struct ToolSet {
    gradient_blend: Option<photocraft_color::BlendMode>,
    shape_stroke: Option<Option<photocraft_doc::ShapeStroke>>,
    tool: Option<Tool>,
    gradient_classic: Option<bool>,
    stroke_trace: Option<&'static str>,
    eyedropper_size: Option<u32>,
    eyedropper_sample: Option<String>,
    eyedropper_ring: Option<bool>,
    cam_transform_on_drop: Option<bool>,
    tile_grid: Option<bool>,
    layer_filter: Option<crate::layer_filter_ui::Filter>,
    pen_mode: Option<String>,
    pen_path_op: Option<String>,
}

impl ToolSet {
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        Ok(Self {
            gradient_blend: gradient_blend_param(p)?,
            shape_stroke: shape_stroke_param(app, p)?,
            tool: tool_param(p)?,
            gradient_classic: bool_field(p, "gradientClassic")?,
            stroke_trace: stroke_trace_param(p)?,
            // The Eyedropper's options bar (#1649): Sample Size, Sample, Show Sampling Ring.
            eyedropper_size: eyedropper_size_param(p)?,
            eyedropper_sample: eyedropper_sample_param(p)?,
            eyedropper_ring: bool_field(p, "eyedropperRing")?,
            // Content-Aware Move's Transform On Drop checkbox.
            cam_transform_on_drop: bool_field(p, "camTransformOnDrop")?,
            // Developer overlay of the active layer's storage tiles (tile_overlay.rs).
            tile_grid: bool_field(p, "tileGrid")?,
            layer_filter: p.get("layerFilter").map(|v| crate::layer_filter_ui::patched(&app.ui, v)).transpose()?,
            pen_mode: pen_mode_param(p)?,
            pen_path_op: pen_path_op_param(p)?,
        })
    }

    fn apply(self, app: &mut PhotocraftApp) {
        if let Some(stroke) = &self.shape_stroke {
            if let Some(stroke) = stroke {
                app.ui.tool_options.stroke_width = stroke.width;
                app.ui.tool_options.shape_stroke = crate::shape_stroke_ui::StrokeOptions::from(stroke);
            } else {
                app.ui.tool_options.stroke_width = 0.0;
            }
        }
        if let Some(t) = self.tool {
            crate::tool_memory::select(app, t);
            // Each tool keeps its own brush (#218), so switch it in before `brushSize`
            // sets the new tool's size.
            crate::paint_mouse::sync_tool_brush(app);
        }
        self.apply_gradient_and_trace(app);
        if let Some(size) = self.eyedropper_size {
            app.ui.tool_options.eyedropper_size = size;
        }
        if let Some(id) = self.eyedropper_sample {
            app.ui.tool_options.eyedropper_sample = id;
        }
        if let Some(ring) = self.eyedropper_ring {
            app.ui.tool_options.eyedropper_ring = ring;
        }
        if let Some(on) = self.cam_transform_on_drop {
            app.ui.tool_options.cam_transform_on_drop = on;
        }
        if let Some(on) = self.tile_grid {
            app.ui.view.tile_grid = on;
        }
        if let Some(f) = self.layer_filter {
            crate::layer_filter_ui::apply(&mut app.ui, f);
        }
        if let Some(m) = self.pen_mode {
            app.ui.tool_options.vector_mode = m;
        }
        if let Some(o) = self.pen_path_op {
            app.ui.tool_options.pen_path_op = o;
        }
    }

    fn apply_gradient_and_trace(&self, app: &mut PhotocraftApp) {
        let gradient_before = app.ui.tool_options.clone();
        if let Some(mode) = self.gradient_blend {
            app.ui.tool_options.gradient_blend_mode = mode;
        }
        if let Some(v) = &self.stroke_trace {
            app.stroke_trace.set(v);
        }
        if let Some(classic) = self.gradient_classic {
            app.ui.tool_options.gradient_classic = classic;
        }
        if self.gradient_blend.is_some() {
            crate::gradient_ui::options_changed(app, &gradient_before);
        }
    }
}

/// The Pen tools' options bar (#1397): Path or Shape mode.
fn pen_mode_param(p: &Value) -> std::result::Result<Option<String>, String> {
    let Some(v) = p.get("penMode") else { return Ok(None) };
    v.as_str().filter(|m| matches!(*m, "path" | "shape")).map(|m| Some(m.to_string())).ok_or_else(|| "penMode must be \"path\" or \"shape\"".to_string())
}

/// The Pen tools' Path Operation.
fn pen_path_op_param(p: &Value) -> std::result::Result<Option<String>, String> {
    let Some(v) = p.get("penPathOp") else { return Ok(None) };
    v.as_str()
        .filter(|o| crate::vector_ui::PATH_OPS.iter().any(|(id, ..)| id == o))
        .map(|o| Some(o.to_string()))
        .ok_or_else(|| "penPathOp must be one of newLayer, combine, subtract, intersect, exclude".to_string())
}

fn gradient_blend_param(p: &Value) -> std::result::Result<Option<photocraft_color::BlendMode>, String> {
    let Some(value) = p.get("gradientBlendMode") else { return Ok(None) };
    let Some(name) = value.as_str() else { return Err("gradientBlendMode must be a blend mode name".into()) };
    photocraft_engine::commands::blend_from_str(name)
        .filter(|m| photocraft_color::BlendMode::LAYER_MODES.contains(m))
        .map(Some)
        .ok_or_else(|| format!("unknown gradient blend mode `{name}`"))
}

fn shape_stroke_param(app: &PhotocraftApp, p: &Value) -> std::result::Result<Option<Option<photocraft_doc::ShapeStroke>>, String> {
    let Some(v) = p.get("shapeStroke") else { return Ok(None) };
    if let Some(object) = v.as_object() {
        let keys = ["width", "opacity", "align", "cap", "join", "miterLimit", "dashes", "dashOffset"];
        if let Some(key) = object.keys().find(|key| !keys.contains(&key.as_str())) {
            return Err(format!("unknown shapeStroke field `{key}`"));
        }
    }
    let o = &app.ui.tool_options;
    let base = o.shape_stroke.stroke(o.stroke_width, photocraft_doc::Fill::Solid(photocraft_doc::Color::BLACK));
    Ok(Some(photocraft_engine::vector_cmds::parse_stroke(v, Some(base), photocraft_doc::Color::BLACK)?))
}

fn tool_param(p: &Value) -> std::result::Result<Option<Tool>, String> {
    match str_param(p, "tool") {
        Some(t) => Tool::from_name(t).map(Some).ok_or_else(|| format!("unknown tool `{t}`")),
        None => Ok(None),
    }
}

/// The stroke latency trace (`stroke_trace`), at runtime: on (stderr) or off. A CSV
/// file is only for the user's own launch (`--stroke-trace <path>`): a path over the
/// control channel would write outside the automation write root.
fn stroke_trace_param(p: &Value) -> std::result::Result<Option<&'static str>, String> {
    match p.get("strokeTrace") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(b)) => Ok(Some(if *b { "stderr" } else { "off" })),
        Some(Value::String(v)) => match v.trim() {
            "stderr" | "on" | "true" => Ok(Some("stderr")),
            "off" | "false" => Ok(Some("off")),
            _ => Err("strokeTrace must be true, false, \"stderr\" or \"off\"; a CSV file is set at launch (--stroke-trace <path>)".into()),
        },
        Some(_) => Err("strokeTrace must be true, false, \"stderr\" or \"off\"".into()),
    }
}

fn eyedropper_size_param(p: &Value) -> std::result::Result<Option<u32>, String> {
    let Some(v) = p.get("eyedropperSampleSize") else { return Ok(None) };
    photocraft_engine::sample_cmds::size_param("ui.set", &json!({"size": v}))
        .map(Some)
        .map_err(|_| format!("eyedropperSampleSize must be \"point\" or one of {:?}", photocraft_engine::sample_cmds::SAMPLE_SIZES))
}

fn eyedropper_sample_param(p: &Value) -> std::result::Result<Option<String>, String> {
    let Some(v) = p.get("eyedropperSample") else { return Ok(None) };
    let ids: Vec<&str> = photocraft_engine::sample_cmds::SampleLayers::ALL.iter().map(|(_, n)| *n).collect();
    let id = v.as_str().filter(|id| ids.contains(id)).ok_or_else(|| format!("eyedropperSample must be one of {}", ids.join(", ")))?;
    Ok(Some(id.to_string()))
}

// -- the Crop tool's overlay, Fill and gear menu --

struct CropSet {
    overlay: Option<crate::crop_overlay::CropOverlay>,
    overlay_show: Option<crate::crop_overlay::OverlayShow>,
    orientation: Option<u8>,
    content_aware: Option<bool>,
    shield: Option<crate::crop_shield::CropShield>,
}

impl CropSet {
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        // The Crop tool's overlay menu (#1919).
        let overlay = crop_overlay_param(p)?;
        let overlay_show = crop_overlay_show_param(p)?;
        let orientation = crop_orientation_param(p)?;
        // The Crop tool's Fill (#3823): "background" (Background (Default)) or "contentAware".
        let content_aware = crop_fill_param(p)?;
        // The Crop tool's gear menu: Show Cropped Area and the shield (#1919).
        let shield = merged_object(&app.ui.tool_options.crop_shield, p.get("cropShield"), "cropShield")?;
        if shield.as_ref().is_some_and(|s| !(0.0..=100.0).contains(&s.opacity)) {
            return Err("cropShield.opacity must be 0..100".into());
        }
        Ok(Self { overlay, overlay_show, orientation, content_aware, shield })
    }

    fn apply(self, app: &mut PhotocraftApp) {
        let o = &mut app.ui.tool_options;
        if let Some(k) = self.overlay {
            o.crop_overlay = k;
        }
        if let Some(on) = self.content_aware {
            o.crop_content_aware = on;
        }
        if let Some(v) = self.overlay_show {
            o.crop_overlay_show = v;
        }
        if let Some(v) = self.orientation {
            o.crop_overlay_orientation = v;
        }
        if let Some(s) = self.shield {
            o.crop_shield = s;
        }
    }
}

fn crop_overlay_param(p: &Value) -> std::result::Result<Option<crate::crop_overlay::CropOverlay>, String> {
    let Some(v) = p.get("cropOverlay") else { return Ok(None) };
    v.as_str().and_then(crate::crop_overlay::CropOverlay::from_id).map(Some).ok_or_else(|| {
        let ids: Vec<&str> = crate::crop_overlay::CropOverlay::ALL.iter().map(|k| k.id()).collect();
        format!("cropOverlay must be one of {}", ids.join(", "))
    })
}

fn crop_overlay_show_param(p: &Value) -> std::result::Result<Option<crate::crop_overlay::OverlayShow>, String> {
    let Some(v) = p.get("cropOverlayShow") else { return Ok(None) };
    v.as_str().and_then(crate::crop_overlay::OverlayShow::from_id).map(Some).ok_or_else(|| "cropOverlayShow must be one of auto, always, never".to_string())
}

fn crop_orientation_param(p: &Value) -> std::result::Result<Option<u8>, String> {
    match uint_field(p, "cropOverlayOrientation")? {
        Some(o) if o < 4 => Ok(Some(o as u8)),
        Some(_) => Err("cropOverlayOrientation must be 0, 1, 2 or 3".into()),
        None => Ok(None),
    }
}

fn crop_fill_param(p: &Value) -> std::result::Result<Option<bool>, String> {
    match p.get("cropFill") {
        Some(v) => match v.as_str() {
            Some("background") => Ok(Some(false)),
            Some("contentAware") => Ok(Some(true)),
            _ => Err("cropFill must be \"background\" or \"contentAware\"".into()),
        },
        None => Ok(None),
    }
}

// -- panels: Histogram, masks targets, docking, Color panel --

struct PanelSet {
    histogram_panel: Option<crate::histogram_panel::HistogramPanelState>,
    histogram_range: Option<Option<[u8; 2]>>,
    histogram_refresh: Option<bool>,
    panels: Option<crate::state::Panels>,
    mask_target: Option<bool>,
    updates_open: Option<bool>,
    vector_mask_target: Option<bool>,
    filter_mask_target: Option<bool>,
    selection_mode: Option<u64>,
    dock_tabs: Option<crate::state::DockTabs>,
    dock: Option<crate::dock::DockLayout>,
    color_panel: Option<crate::state::ColorPanelState>,
    dock_width: Option<f64>,
}

impl PanelSet {
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        Ok(Self {
            // The Histogram panel: its options, the dragged tonal range ([from, to] levels or
            // null) and Uncached Refresh.
            histogram_panel: merged_object(&app.ui.histogram_panel, p.get("histogramPanel"), "histogramPanel")?,
            histogram_range: histogram_range_param(p)?,
            histogram_refresh: bool_field(p, "histogramRefresh")?,
            panels: merged_object(&app.ui.panels, p.get("panels"), "panels")?,
            mask_target: bool_field(p, "maskTarget")?,
            updates_open: bool_field(p, "updatesOpen")?,
            vector_mask_target: bool_field(p, "vectorMaskTarget")?,
            filter_mask_target: filter_mask_target_param(app, p)?,
            selection_mode: uint_field(p, "selectionMode")?,
            dock_tabs: merged_object(&app.ui.dock_tabs, p.get("dockTabs"), "dockTabs")?,
            dock: whole_object(&app.ui.dock, p.get("dock"), "dock")?,
            // Which chip the Color panel edits.
            color_panel: merged_object(&app.ui.color_panel, p.get("colorPanel"), "colorPanel")?,
            dock_width: f32_field(p, "dockWidth")?,
        })
    }

    /// Everything that precedes the theme.
    fn apply_options(&mut self, app: &mut PhotocraftApp) {
        self.apply_histogram(app);
        if let Some(v) = self.panels.take() {
            app.ui.panels = v;
        }
        self.apply_mask_targets(app);
        // Selection tools' options-bar mode: 0 New, 1 Add, 2 Subtract, 3 Intersect.
        if let Some(m) = self.selection_mode {
            app.ui.selection_mode = m.min(3) as u8;
        }
    }

    fn apply_histogram(&mut self, app: &mut PhotocraftApp) {
        if let Some(h) = self.histogram_panel.take() {
            app.ui.histogram_panel = h;
            crate::histogram_panel::persist_options(app);
        }
        if let Some(r) = self.histogram_range {
            app.ui.histogram_range = r;
        }
        if self.histogram_refresh == Some(true) {
            crate::histogram_panel::uncached_refresh(app);
        }
    }

    fn apply_mask_targets(&self, app: &mut PhotocraftApp) {
        if let Some(m) = self.mask_target {
            app.end_filter_mask_view();
            app.ui.filter_mask_target = None;
            app.ui.mask_target = m;
            app.ui.properties_layer = None;
            app.ui.vector_mask_target &= !m;
            app.sync_mask_targets();
        }
        if let Some(open) = self.updates_open {
            app.ui.updates_open = open;
        }
        if let Some(m) = self.vector_mask_target {
            app.end_filter_mask_view();
            app.ui.filter_mask_target = None;
            app.ui.vector_mask_target = m;
            app.ui.mask_target &= !m;
            app.sync_mask_targets();
        }
        if let Some(m) = self.filter_mask_target {
            let layer = if m { app.session.active().and_then(|st| st.active_layer).map(|id| id.0) } else { None };
            app.select_filter_mask_target(layer);
        }
    }

    fn apply_dock(&mut self, app: &mut PhotocraftApp) {
        if let Some(v) = self.dock_tabs.take() {
            app.ui.dock_tabs = v;
        }
        // Dock group order, heights and collapsed groups (see `dock::DockLayout`).
        if let Some(v) = self.dock.take() {
            app.ui.dock = v;
        }
    }

    fn apply_color_and_width(&mut self, app: &mut PhotocraftApp, ctx: &egui::Context) {
        if let Some(v) = self.color_panel.take() {
            app.ui.color_panel = v;
        }
        // Right dock width in points (clamped to the dock's 250..=520 range), applied next frame.
        if let Some(w) = self.dock_width {
            crate::panels::request_dock_width(ctx, w as f32);
        }
    }
}

fn histogram_range_param(p: &Value) -> std::result::Result<Option<Option<[u8; 2]>>, String> {
    let Some(v) = p.get("histogramRange") else { return Ok(None) };
    serde_json::from_value::<Option<[u8; 2]>>(v.clone()).map(Some).map_err(|_| "histogramRange must be [from, to] levels 0..255, or null".to_string())
}

/// Target the active smart object's filter mask (FILE-215-3): it needs smart filters.
fn filter_mask_target_param(app: &PhotocraftApp, p: &Value) -> std::result::Result<Option<bool>, String> {
    let target = bool_field(p, "filterMaskTarget")?;
    if target == Some(true) {
        check_filter_mask_target(app)?;
    }
    Ok(target)
}

fn check_filter_mask_target(app: &PhotocraftApp) -> std::result::Result<(), String> {
    use photocraft_engine::channel_cmds::ChannelTarget;
    let st = app.session.active().ok_or("filterMaskTarget needs an open document")?;
    let l = st.active_layer.and_then(|id| st.doc.layer(id)).ok_or("filterMaskTarget needs an active layer")?;
    if !matches!(&l.content, photocraft_doc::LayerContent::Smart(sm) if !sm.smart_filters.is_empty()) {
        return Err("filterMaskTarget needs an active smart object with smart filters".into());
    }
    let composite = matches!(st.channel_view.target, ChannelTarget::Composite | ChannelTarget::LayerMask | ChannelTarget::VectorMask);
    if !composite || st.doc.quick_mask.is_some() {
        return Err("filterMaskTarget can't be set while a single channel or Quick Mask is targeted".into());
    }
    Ok(())
}

// -- the document view: zoom, centre, rotation, fit --

struct ViewSet {
    zoom: Option<f64>,
    rotation: Option<f64>,
    center: Option<[f32; 2]>,
    fit: bool,
}

impl ViewSet {
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        let zoom = f32_field(p, "zoom")?;
        let rotation = f32_field(p, "rotation")?;
        if rotation.is_some() && app.session.active_index().is_none() {
            return Err("no document open".into());
        }
        let center = center_param(p)?;
        let fit = bool_field(p, "fit")? == Some(true);
        Ok(Self { zoom, rotation, center, fit })
    }

    fn apply(&self, app: &mut PhotocraftApp) {
        let Some(i) = app.session.active_index() else { return };
        if self.zoom.is_some() || self.center.is_some() || self.rotation.is_some() || self.fit {
            app.ui.views[i].zoom_animating = false;
            app.ui.views[i].zoom_anchor = None;
        }
        if let Some(z) = self.zoom {
            let size = app.session.documents().get(i).map_or([0, 0], |st| [st.doc.size.width, st.doc.size.height]);
            app.ui.views[i].zoom = crate::zoom_levels::clamp(z as f32, size);
            app.ui.views[i].fit_pending = false;
            app.ui.views[i].fill_pending = false;
        }
        if let Some(c) = self.center {
            app.ui.views[i].center = c;
            app.ui.views[i].fit_pending = false;
            app.ui.views[i].fill_pending = false;
        }
        if self.fit {
            app.ui.views[i].fit_pending = true;
            app.ui.views[i].fill_pending = false;
        }
        if let Some(r) = self.rotation {
            app.ui.views[i].rotation = crate::rotate_view::wrap_deg(r as f32);
        }
    }
}

fn center_param(p: &Value) -> std::result::Result<Option<[f32; 2]>, String> {
    let Some(v) = p.get("center") else { return Ok(None) };
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
    Ok(Some([x as f32, y as f32]))
}

// -- brushes: sections, picker, size --

struct BrushSet {
    section: Option<u64>,
    tab: Option<u64>,
    brushes_view: Option<crate::brush_panel::BrushesView>,
    picker_name: Option<bool>,
    picker_stroke: Option<bool>,
    picker_tip: Option<bool>,
    picker_scale: Option<f32>,
    picker: Option<Option<[f32; 2]>>,
    size: Option<f64>,
}

impl BrushSet {
    fn parse(app: &PhotocraftApp, p: &Value) -> std::result::Result<Self, String> {
        let section = uint_field(p, "brushSection")?;
        let tab = uint_field(p, "brushTab")?;
        let brushes_view = brushes_view_param(p)?;
        // The three parts of the picker's cards. At least one stays on (a rejected call
        // leaves them all alone).
        let picker_name = bool_field(p, "brushPickerName")?;
        let picker_stroke = bool_field(p, "brushPickerStroke")?;
        let picker_tip = bool_field(p, "brushPickerTip")?;
        check_picker_parts(app, [picker_name, picker_stroke, picker_tip])?;
        Ok(Self {
            section,
            tab,
            brushes_view,
            picker_name,
            picker_stroke,
            picker_tip,
            picker_scale: picker_scale_param(p)?,
            picker: picker_param(p)?,
            size: brush_size_param(p)?,
        })
    }

    fn apply(&self, app: &mut PhotocraftApp) -> std::result::Result<(), String> {
        if let Some(i) = self.section {
            app.ui.brush_section = (i as usize).min(crate::brush_panel::SECTIONS.len() - 1);
        }
        if let Some(i) = self.tab {
            app.ui.brush_tab = (i as usize).min(1);
        }
        if let Some(v) = self.brushes_view {
            app.ui.brushes_panel.view = v;
        }
        self.apply_picker(app);
        if let Some(size) = self.size {
            app.run("tools.setBrush", json!({"brush": {"size": size}})).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    fn apply_picker(&self, app: &mut PhotocraftApp) {
        let list = &mut app.ui.brush_picker_list;
        if let Some(v) = self.picker_name {
            list.show_name = v;
        }
        if let Some(v) = self.picker_stroke {
            list.show_stroke = v;
        }
        if let Some(v) = self.picker_tip {
            list.show_tip = v;
        }
        if let Some(v) = self.picker_scale {
            list.scale = v;
        }
        if let Some(at) = self.picker {
            app.ui.brush_picker = at;
        }
    }
}

fn brushes_view_param(p: &Value) -> std::result::Result<Option<crate::brush_panel::BrushesView>, String> {
    let Some(v) = p.get("brushesView") else { return Ok(None) };
    serde_json::from_value(v.clone()).map(Some).map_err(|e| format!("brushesView: {e} (list, grid)"))
}

fn check_picker_parts(app: &PhotocraftApp, parts: [Option<bool>; 3]) -> std::result::Result<(), String> {
    if parts.iter().all(Option::is_none) {
        return Ok(());
    }
    let list = &app.ui.brush_picker_list;
    let on = [parts[0].unwrap_or(list.show_name), parts[1].unwrap_or(list.show_stroke), parts[2].unwrap_or(list.show_tip)];
    if on.iter().any(|b| *b) { Ok(()) } else { Err("brushPickerName, brushPickerStroke and brushPickerTip: at least one must stay on".into()) }
}

/// The picker footer slider: the preset cards' size scale (1 standard; 0.30 and
/// below the tips drop their size numbers).
fn picker_scale_param(p: &Value) -> std::result::Result<Option<f32>, String> {
    match num_field(p, "brushPickerScale")? {
        Some(s) if !(0.15..=2.0).contains(&s) => Err("brushPickerScale must be between 0.15 and 2".into()),
        Some(s) => Ok(Some(s as f32)),
        None => Ok(None),
    }
}

/// `Some(None)` is an explicit null, which closes the picker (a missing field leaves it alone).
fn picker_param(p: &Value) -> std::result::Result<Option<Option<[f32; 2]>>, String> {
    let Some(v) = p.get("brushPicker") else { return Ok(None) };
    let at =
        serde_json::from_value::<Option<[f32; 2]>>(v.clone()).map_err(|_| "brushPicker must be [x, y] in screen points, or null to close it".to_string())?;
    if at.is_some_and(|[x, y]| !x.is_finite() || !y.is_finite()) {
        return Err("brushPicker must be [x, y] in screen points, or null to close it".into());
    }
    Ok(Some(at))
}

/// brushSize rides the journaled `tools.setBrush` command (#744). Numeric values
/// are validated here so the command cannot fail after other fields were
/// applied; non-numeric values keep the API's historical silent no-op.
fn brush_size_param(p: &Value) -> std::result::Result<Option<f64>, String> {
    let Some(size) = p.get("brushSize").and_then(Value::as_f64) else { return Ok(None) };
    if !size.is_finite() || size > f64::from(photocraft_paint::MAX_BRUSH_SIZE) {
        return Err(format!("brushSize must be finite and at most {} px", photocraft_paint::MAX_BRUSH_SIZE));
    }
    Ok(Some(size))
}

// ---- dialogs ----

fn dialog_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Outcome> {
    let p = &req.params;
    let s = |k: &str| str_param(p, k);
    let u = |k: &str| u64_param(p, k);
    Some(match req.method.as_str() {
        "ui.dialog.open" => dialog_open(app, p),
        // The shell's own dialog (Window › Workspace, View › Pixel Aspect Ratio › Custom, Show
        // Extras Options, 32-bit Preview Options), listed by ui.inspect with the id "shell".
        "ui.dialog.set" if s("dialog") == Some(SHELL_DIALOG) => shell_dialog_set(app, p),
        "ui.dialog.confirm" if s("dialog") == Some(SHELL_DIALOG) => shell_dialog_confirm(app, ctx),
        "ui.dialog.apply" if s("dialog") == Some(SHELL_DIALOG) => err("`ui.dialog.apply` is for Preferences; use `ui.dialog.confirm`"),
        // The unsaved-changes prompt (closing, Revert, quitting), listed by ui.inspect with the
        // id "discard" (#4920): confirm takes a `choice` (save, dontSave, cancel; default save).
        "ui.dialog.confirm" if s("dialog") == Some(crate::discard_ui::CONTROL_ID) => discard_dialog_confirm(app, ctx, s("choice").unwrap_or("save")),
        "ui.dialog.cancel" if s("dialog") == Some(crate::discard_ui::CONTROL_ID) => wrap(crate::discard_ui::answer_control(app, ctx, "cancel")),
        "ui.dialog.set" | "ui.dialog.apply" if s("dialog") == Some(crate::discard_ui::CONTROL_ID) => {
            err("the unsaved-changes prompt has no fields; use `ui.dialog.confirm` with a `choice`")
        }
        "ui.dialog.cancel" if s("dialog") == Some(SHELL_DIALOG) => match app.ui.shell.dialog.take() {
            Some(_) => ok(Value::Null),
            None => err("no such dialog"),
        },
        "ui.dialog.set" => dialog_set(app, p),
        "ui.dialog.confirm" | "ui.dialog.apply" => match u("dialog") {
            Some(id) => dialog_confirm_or_apply(app, req, id),
            None => err("missing `dialog`"),
        },
        "ui.dialog.cancel" => match u("dialog") {
            Some(id) => match crate::dialogs::cancel(app, id) {
                Ok(value) => ok(value),
                Err(error) => err(error),
            },
            None => err("missing `dialog`"),
        },
        _ => return None,
    })
}

fn dialog_open(app: &mut PhotocraftApp, p: &Value) -> Outcome {
    let kind = match str_param(p, "kind").unwrap_or("") {
        "newDocument" | "NewDocument" => DialogKind::NewDocument,
        "about" | "About" => DialogKind::About,
        "colorPicker" | "ColorPicker" => return color_picker_dialog(app, p),
        other => return open_named_dialog(app, p, other),
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

/// The dialog kinds that open through their own module rather than `open_dialog`.
fn open_named_dialog(app: &mut PhotocraftApp, p: &Value, kind: &str) -> Outcome {
    let opened = |id: Option<u64>, missing: &str| match id {
        Some(id) => ok(json!({"dialog": id})),
        None => err(missing),
    };
    match kind {
        "layerStyle" | "LayerStyle" => opened(crate::layer_style::open(app, str_param(p, "effect")), "no active layer"),
        // The Crop tool's New / Delete Crop Preset… (#1919).
        "newCropPreset" => opened(crate::crop_presets::open_new(app), "nothing to save: set W and H (W x H x Resolution) or a ratio first"),
        "deleteCropPreset" => opened(crate::crop_presets::open_delete(app), "no crop presets are saved"),
        "command" | "Command" => {
            let Some(cmd) = str_param(p, "command") else { return err("command dialogs need `command`") };
            let label = photocraft_engine::commands::find(cmd).map(|c| c.label).unwrap_or(cmd);
            ok(json!({"dialog": crate::dialogs::open_command_dialog(app, cmd, label)}))
        }
        other => err(format!("unknown dialog kind `{other}`")),
    }
}

fn color_picker_dialog(app: &mut PhotocraftApp, p: &Value) -> Outcome {
    if let Some(style) = p.get("textStyle") {
        return text_style_color_picker(app, p, style);
    }
    if p.get("parent").is_some() {
        let (Some(parent), Some(effect), Some(field)) = (u64_param(p, "parent"), str_param(p, "effect"), str_param(p, "field")) else {
            return err("need integer `parent` and string `effect` and `field`");
        };
        let stop = match p.get("stop") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_u64().and_then(|i| usize::try_from(i).ok()) {
                Some(i) => Some(i),
                None => return err("`stop` must be a non-negative integer"),
            },
        };
        return match crate::layer_style::color_picker::open(app, parent, effect, field, stop) {
            Ok(id) => ok(json!({"dialog": id})),
            Err(error) => err(error),
        };
    }
    if p.get("effect").is_some() || p.get("field").is_some() {
        return err("Layer Style color pickers need `parent`");
    }
    let target = if str_param(p, "target") == Some("background") { "background" } else { "foreground" };
    ok(json!({"dialog": crate::color_picker_ui::open(app, target)}))
}

fn text_style_color_picker(app: &mut PhotocraftApp, p: &Value, style: &Value) -> Outcome {
    if ["parent", "effect", "field", "target"].iter().any(|key| p.get(*key).is_some()) {
        return err("textStyle cannot be combined with another picker target");
    }
    let Some(style) = style.as_object().filter(|s| s.keys().all(|k| k == "kind" || k == "id")) else {
        return err("textStyle needs kind and id only");
    };
    let Some(kind) = style.get("kind").cloned().and_then(|v| serde_json::from_value::<crate::type_panels_ui::color_picker::Kind>(v).ok()) else {
        return err("textStyle kind must be character or paragraph");
    };
    let Some(id) = style.get("id").and_then(Value::as_u64).and_then(|id| u32::try_from(id).ok()) else {
        return err("textStyle id must be a u32 integer");
    };
    match crate::type_panels_ui::color_picker::open(app, kind, id) {
        Ok(id) => ok(json!({"dialog": id})),
        Err(error) => err(error),
    }
}

fn shell_dialog_set(app: &mut PhotocraftApp, p: &Value) -> Outcome {
    let Some(field) = str_param(p, "field") else { return err("need `dialog` and `field`") };
    let value = p.get("value").cloned().unwrap_or(Value::Null);
    match app.ui.shell.dialog.as_mut() {
        Some((_, fields)) => {
            fields.insert(field.to_string(), value);
            ok(Value::Null)
        }
        None => err("no shell dialog is open"),
    }
}

fn shell_dialog_confirm(app: &mut PhotocraftApp, ctx: &egui::Context) -> Outcome {
    let Some((kind, fields)) = app.ui.shell.dialog.clone() else { return err("no shell dialog is open") };
    if let Some((command, params)) = crate::workspace_ui::dialog_command(&kind, &fields)
        && let Some(authorize) = app.services.automation_command.as_ref()
        && let Err(error) = authorize(command, &params)
    {
        return err(error);
    }
    wrap(crate::workspace_ui::confirm(app, ctx))
}

fn discard_dialog_confirm(app: &mut PhotocraftApp, ctx: &egui::Context, choice: &str) -> Outcome {
    if choice != "cancel"
        && let Some((command, params)) = crate::discard_ui::pending_command(app)
        && let Some(authorize) = app.services.automation_command.as_ref()
        && let Err(error) = authorize(&command, &params)
    {
        return err(error);
    }
    wrap(crate::discard_ui::answer_control(app, ctx, choice))
}

fn dialog_set(app: &mut PhotocraftApp, p: &Value) -> Outcome {
    let (Some(id), Some(field)) = (u64_param(p, "dialog"), str_param(p, "field")) else { return err("need `dialog` and `field`") };
    let value = p.get("value").cloned().unwrap_or(Value::Null);
    match app.ui.dialog_mut(id) {
        // A continuation belongs to the dialog that paused it, even when another
        // action has a valid token. Replacing it would resume or cancel that action.
        Some(_) if field == crate::actions::RESUME => err("the paused action token is read-only"),
        // A marker picks the handler OK runs: a dialog keeps the ones it was opened with.
        Some(d) if !d.fields.contains_key(field) && crate::dialogs::is_route_marker(field) => err(format!("`{field}` is set only by the dialog it belongs to")),
        Some(d) => {
            d.fields.insert(field.to_string(), value);
            ok(Value::Null)
        }
        None => err(format!("no dialog {id}")),
    }
}

fn dialog_confirm_or_apply(app: &mut PhotocraftApp, req: &ControlRequest, id: u64) -> Outcome {
    let apply = req.method == "ui.dialog.apply";
    let command = app.ui.dialogs.iter().find(|dialog| dialog.id == id).and_then(|dialog| {
        if apply {
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
    run_waiting(app, wait_param(&req.params), |app| if apply { crate::prefs_ui::apply(app, id) } else { crate::dialogs::confirm(app, id) })
}

// ---- synthetic input: pointer events, clicks, keys, text ----

fn input_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Outcome> {
    let p = &req.params;
    Some(match req.method.as_str() {
        "ui.pointer" => pointer(app, ctx, p),
        "ui.click" | "ui.move" => click(app, ctx, p, req.method == "ui.click"),
        "ui.key" => key(app, ctx, p),
        "ui.type" => {
            let Some(text) = str_param(p, "text") else { return Some(err("missing `text`")) };
            app.synthetic.push(egui::Event::Text(text.to_string()));
            ctx.request_repaint();
            Outcome::AfterInput
        }
        _ => return None,
    })
}

fn click(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value, click: bool) -> Outcome {
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

fn key(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Outcome {
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

/// Simulated pen tilt/rotation of one `ui.pointer` event.
type Pen = (Option<f32>, Option<f32>, Option<f32>);

/// One validated `ui.pointer` event.
struct PointerSample {
    ev: ToolEvent,
    x: f64,
    y: f64,
    pressure: f32,
    pen: Pen,
}

struct ParsedPointer {
    tool: Option<Tool>,
    button: String,
    modifiers: Value,
    events: Vec<PointerSample>,
}

/// The gesture shared by every event of one `ui.pointer` call.
struct PointerGesture {
    secondary: bool,
    middle: bool,
    space: bool,
    mods: egui::Modifiers,
}

fn pointer(app: &mut PhotocraftApp, ctx: &egui::Context, p: &Value) -> Outcome {
    // Every field and event is validated before the first event runs, so a rejected call
    // changes nothing (#2449).
    let parsed = match parse_pointer(p) {
        Ok(parsed) => parsed,
        Err(e) => return err(e),
    };
    if let Some(t) = parsed.tool {
        crate::tool_memory::select(app, t);
    }
    // Space held: the Crop tool moves the frame being drawn, a marquee, lasso or shape
    // being drawn moves instead of growing (hold_keys.rs). Like a modifier flag, it may be
    // top-level or grouped under "modifiers".
    let space = p.get("modifiers").unwrap_or(p).get("space").and_then(Value::as_bool).unwrap_or(false);
    crate::crop_ui::set_space(app, space);
    let gesture = PointerGesture {
        secondary: matches!(parsed.button.as_str(), "secondary" | "right"),
        middle: parsed.button == "middle",
        space,
        mods: modifiers_from(&parsed.modifiers),
    };
    for sample in parsed.events {
        if crate::brush_mode_menu::blocks_control(app, ctx, sample.ev) {
            continue;
        }
        if !pointer_event_intercepted(app, ctx, &gesture, &sample) {
            forward_pointer_event(app, &gesture, sample);
        }
    }
    app.stylus.feed.set(None);
    ok(json!({"status": app.ui.status}))
}

fn parse_pointer(p: &Value) -> std::result::Result<ParsedPointer, String> {
    let fields = ["events", "tool", "modifiers", "button", "shift", "alt", "command", "ctrl", "space"];
    let p = input_object(p, None, &fields)?;
    let tool = match input_choice(&p, "tool", "tool", &[])? {
        Some(t) => Some(Tool::from_name(t).ok_or_else(|| format!("unknown tool `{t}`"))?),
        None => None,
    };
    let button = input_choice(&p, "button", "button", &BUTTONS)?.unwrap_or("left").to_string();
    // Modifier flags may be top-level or grouped under "modifiers".
    let modifiers = input_modifiers(&p, &POINTER_MODIFIERS)?;
    let Some(events) = p.get("events") else { return Err("missing `events`".into()) };
    let Some(events) = events.as_array() else { return Err("events must be an array".into()) };
    let events = events.iter().enumerate().map(|(i, e)| parse_pointer_event(i, e)).collect::<std::result::Result<Vec<_>, _>>()?;
    Ok(ParsedPointer { tool, button, modifiers, events })
}

fn parse_pointer_event(i: usize, e: &Value) -> std::result::Result<PointerSample, String> {
    let name = format!("events[{i}]");
    let e = input_object(e, Some(&name), &["kind", "x", "y", "pressure", "tiltX", "tiltY", "rotation"])?;
    let num = |k: &str, range| input_number(&e, k, &format!("{name}.{k}"), range);
    let (x, y) = (num("x", None)?.unwrap_or(0.0), num("y", None)?.unwrap_or(0.0));
    let pressure = num("pressure", Some((0.0, 1.0)))?.unwrap_or(1.0) as f32;
    let ev = match input_choice(&e, "kind", &format!("{name}.kind"), &["down", "move", "up"])?.unwrap_or("move") {
        "down" => ToolEvent::Down { x, y, pressure },
        "up" => ToolEvent::Up { x, y },
        _ => ToolEvent::Move { x, y, pressure },
    };
    let tilt = |k: &str, range| num(k, Some(range)).map(|v| v.map(|v| v as f32));
    let pen = (tilt("tiltX", (-90.0, 90.0))?, tilt("tiltY", (-90.0, 90.0))?, tilt("rotation", (0.0, 360.0))?);
    Ok(PointerSample { ev, x, y, pressure, pen })
}

/// Whether a modal tool, picker or context menu took the event instead of the canvas.
fn pointer_event_intercepted(app: &mut PhotocraftApp, ctx: &egui::Context, g: &PointerGesture, s: &PointerSample) -> bool {
    pointer_eyedropper_took(app, ctx, g, s) || color_range_took(app, g, s) || (g.secondary && secondary_menu_took(app, ctx, g, s))
}

fn pointer_eyedropper_took(app: &mut PhotocraftApp, ctx: &egui::Context, g: &PointerGesture, s: &PointerSample) -> bool {
    let (x, y) = (s.x, s.y);
    let down = matches!(s.ev, ToolEvent::Down { .. });
    // With the Color Picker on top the image is its eyedropper, as for the mouse.
    if crate::color_picker_ui::top(app).is_some() {
        if !matches!(s.ev, ToolEvent::Up { .. }) {
            crate::color_picker_ui::sample_at(app, x, y);
        }
        return true;
    }
    // Curves' modal eyedroppers use document coordinates here, matching the native
    // canvas path after its ViewXform conversion. Sample on press only.
    if !g.secondary && crate::adjust_dialog::picker_armed(app) {
        if down {
            crate::adjust_dialog::sample_at(app, x, y);
        }
        return true;
    }
    // ⌘/Ctrl-click with Curves open adds a point at the pixel's tone, on press only (#7325).
    if !g.secondary && !g.space && !g.middle && g.mods.command && crate::adjust_dialog::curves_on_top(app) {
        if down {
            crate::adjust_dialog::add_point_at(app, ctx, x, y, g.mods.shift);
        }
        return true;
    }
    // A Properties Levels/Curves eyedropper owns the primary button the same way.
    if app.ui.dialogs.is_empty() && !g.secondary && crate::layer_picker::armed(app).is_some() {
        if down {
            crate::layer_picker::sample_at(app, x, y);
        }
        return true;
    }
    false
}

/// The dialog owns all pointer events, including Up: no underlying brush stroke
/// or context menu may leak through while inspecting a Color Range preview.
fn color_range_took(app: &mut PhotocraftApp, g: &PointerGesture, s: &PointerSample) -> bool {
    if !app.ui.dialogs.last().is_some_and(|d| crate::color_range_ui::owns(&d.fields)) {
        return false;
    }
    let down = matches!(s.ev, ToolEvent::Down { .. });
    let up = matches!(s.ev, ToolEvent::Up { .. });
    let held = app.ui.dialogs.last().and_then(|d| d.fields.get("__pointerDown")).and_then(Value::as_bool).unwrap_or(false);
    if let Some(d) = app.ui.dialogs.last_mut() {
        d.fields.insert("__pointerDown".into(), json!(!up && (down || held)));
    }
    if !up && (down || held) && !g.space && !g.secondary && !g.middle {
        crate::color_range_ui::pick_top(app, [s.x, s.y], g.mods);
    }
    true
}

/// Right-click: the transform box's menu, the layer list, or a canvas menu. Whether it opens
/// only on the press, the event is consumed either way.
fn secondary_menu_took(app: &mut PhotocraftApp, ctx: &egui::Context, g: &PointerGesture, s: &PointerSample) -> bool {
    secondary_transform_or_crop_took(app, ctx, g, s) || secondary_canvas_menu_took(app, ctx, g, s)
}

fn secondary_transform_or_crop_took(app: &mut PhotocraftApp, ctx: &egui::Context, g: &PointerGesture, s: &PointerSample) -> bool {
    let (x, y) = (s.x, s.y);
    let down = matches!(s.ev, ToolEvent::Down { .. });
    // Right-click while transforming: the box's menu, as on the canvas.
    if app.ui.transform.as_ref().is_some_and(|t| t.warp.is_none()) {
        if down {
            crate::canvas_tool_menu::open_transform(app, ctx, screen_point(app, x, y));
        }
        return true;
    }
    // Right-click with the Move tool, or ⌘/Ctrl+right-click: list the layers there.
    if crate::layer_pick_ui::is_gesture(app.ui.tool, g.mods) {
        if down {
            app.ui.canvas_tool_menu = None;
            crate::layer_pick_ui::open(app, ctx, screen_point(app, x, y), x, y);
        }
        return true;
    }
    if app.ui.tool != crate::state::Tool::Crop {
        return false;
    }
    // No menu over the agent's own crop frame or Straighten line still being
    // dragged, also the first one on the default frame (nothing pending yet).
    if app.crop.drag.is_some() || app.crop.straighten.line.is_some() {
        return true;
    }
    if crate::crop_ui::pending(app) {
        if down {
            crate::canvas_tool_menu::open_crop(app, ctx, app.ui.tool, screen_point(app, x, y));
        }
        return true;
    }
    false
}

fn secondary_canvas_menu_took(app: &mut PhotocraftApp, ctx: &egui::Context, g: &PointerGesture, s: &PointerSample) -> bool {
    let (x, y) = (s.x, s.y);
    let down = matches!(s.ev, ToolEvent::Down { .. });
    if crate::canvas_tool_menu::pasteboard_at(app, app.ui.tool, [x, y]) {
        if down {
            crate::canvas_tool_menu::open_pasteboard(app, ctx, app.ui.tool, screen_point(app, x, y));
        }
        return true;
    }
    if crate::canvas_tool_menu::is_view_tool(app.ui.tool) {
        if down {
            crate::canvas_tool_menu::open_view(app, ctx, app.ui.tool, screen_point(app, x, y), [x, y]);
        }
        return true;
    }
    if crate::canvas_tool_menu::applies(app.ui.tool)
        || crate::canvas_tool_menu::editing_text(app, app.ui.tool)
        || crate::canvas_tool_menu::type_layer_at(app, app.ui.tool, [x, y]).is_some()
    {
        if down {
            crate::canvas_tool_menu::open_at(app, ctx, app.ui.tool, screen_point(app, x, y), Some([x, y]));
        }
        return true;
    }
    !crate::paint_mouse::pointer_secondary(app, ctx, down, g.mods, screen_point(app, x, y))
}

/// Hand an event nothing intercepted to the active tool.
fn forward_pointer_event(app: &mut PhotocraftApp, g: &PointerGesture, s: PointerSample) {
    // A simulated pen: tilt/rotation reach the stroke like a real stylus's (see `stylus`).
    if s.pen != (None, None, None) {
        let (tilt_x, tilt_y, rotation) = (s.pen.0.unwrap_or(0.0), s.pen.1.unwrap_or(0.0), s.pen.2.unwrap_or(0.0));
        app.stylus.feed.set(Some(crate::stylus::PenSample { pressure: s.pressure, tilt_x, tilt_y, rotation, eraser: false }));
    }
    if let Some(d) = app.drag.as_mut().filter(|d| crate::hold_keys::repositions(d.tool)) {
        d.reposition = g.space;
    }
    // A Crop drag in the default mode turns and pans the view with the image: its
    // points are read as the view showed them at the press (`crop_mode`).
    let ev = match s.ev {
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
    tool_event(app, ev, g.mods);
}

// ---- the application: open, save, quit ----

fn app_methods(app: &mut PhotocraftApp, ctx: &egui::Context, req: &ControlRequest) -> Option<Outcome> {
    let p = &req.params;
    Some(match req.method.as_str() {
        "app.open" => match str_param(p, "path") {
            Some(path) => app_open(app, path),
            None => err("missing `path`"),
        },
        "app.save" => match crate::FormatChoices::from_params(p, "app.save") {
            Ok(choices) => {
                let path = str_param(p, "path").map(str::to_string);
                run_waiting(app, wait_param(p), move |app| app.save_automation_with(path, choices))
            }
            Err(e) => err(e),
        },
        "app.quit" => {
            app.allow_close = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            ok(Value::Null)
        }
        _ => return None,
    })
}

fn app_open(app: &mut PhotocraftApp, path: &str) -> Outcome {
    if let Some(outcome) = open_pcraft_directory(app, path) {
        return outcome;
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

/// Directory .pcraft (the native on-disk form) loads through a capability-scoped
/// source; the single-file (zip) form falls back to the bytes path, as do all
/// other formats. `None` means "not a directory .pcraft".
fn open_pcraft_directory(app: &mut PhotocraftApp, path: &str) -> Option<Outcome> {
    if !path.rsplit_once('.').is_some_and(|(_, ext)| ext.eq_ignore_ascii_case(photocraft_format::EXTENSION)) {
        return None;
    }
    let open = app.services.automation_open_pcraft.as_mut()?;
    match open(path) {
        Ok(Some((name, doc))) => {
            app.session.add_document(doc, None);
            // Recent files, but no save-back target: the capability writer
            // replaces files, never directories.
            app.push_recent(path);
            app.sync_views();
            app.ui.status = crate::i18n::fmt(tl!("Opened {name}"), &[("name", &name)]);
            app.ui.status_error = false;
            Some(ok(json!({"path": path, "name": name, "warnings": []})))
        }
        Ok(None) => None, // a regular file: the bytes path reads the zip form
        Err(e) => Some(err(e)),
    }
}
