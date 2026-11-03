use super::fields::choice_label;
use super::{Map, PhotocraftApp, RichText, Tokens, Value, json, prefs};

/// Performance › Graphics: the compositor's GPU memory budget and whether the active document's
/// layers fit it (the GPU Memory preference sets it).
pub(super) fn gpu_memory_lines(app: &PhotocraftApp) -> Vec<String> {
    let mut v = Vec::new();
    if app.perf.gpu_budget_mb > 0 {
        let budget = crate::chrome_ui::fmt_bytes(app.perf.gpu_budget_mb.saturating_mul(1 << 20));
        v.push(crate::i18n::fmt(tl!("GPU memory budget: {budget}"), &[("budget", &budget)]));
    }
    let doc = app.session.active().map(|st| st.doc.id.0);
    if let Some(fit) = app.perf.gpu_layer_fit.filter(|f| Some(f.doc) == doc) {
        let args = [("needed", crate::chrome_ui::fmt_bytes(fit.needed)), ("budget", crate::chrome_ui::fmt_bytes(fit.budget))];
        let args: Vec<(&str, &str)> = args.iter().map(|(k, v)| (*k, v.as_str())).collect();
        v.push(if fit.fits() {
            crate::i18n::fmt(tl!("This document's layers: {needed} of {budget} for layers"), &args)
        } else {
            crate::i18n::fmt(tl!("This document's layers need {needed} but may hold {budget}: whole-document redraws use the CPU"), &args)
        });
    }
    v
}

/// Preferences › Performance: what the app renders with now, and a reset of the GPU backend
/// (a crashed start may have moved it to a safer choice).
pub(super) fn gpu_status_rows(ui: &mut egui::Ui, info: Option<&Value>, adapters: Option<&Value>, obj: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(10.0);
    ui.label(RichText::new(tl!("Graphics")).font(crate::theme::semibold(12.5)).color(t.text));
    for line in info.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str) {
        ui.label(RichText::new(line).color(t.text_dim));
    }
    ui.add_space(4.0);
    ui.label(RichText::new(tl!("Applies at next launch.")).color(t.text_faint));
    ui.collapsing(tl!("Advanced"), |ui| {
        ui.horizontal(|ui| {
            ui.label(tl!("GPU Backend"));
            let mut current = obj.get("gpuBackend").and_then(Value::as_str).unwrap_or("auto").to_string();
            let options = prefs::choices("performance.gpuBackend").unwrap_or(&[]);
            let labels: Vec<String> = options.iter().map(|o| choice_label(o)).collect();
            let pairs: Vec<(String, &str)> = options.iter().map(|o| o.to_string()).zip(labels.iter().map(String::as_str)).collect();
            crate::widgets::dropdown(ui, "graphics-backend", tl!("GPU Backend"), &mut current, &pairs, 220.0);
            obj.insert("gpuBackend".into(), json!(current));
        });
        if !prefs::is_hidden("performance.gpuAdapter") {
            graphics_adapter_row(ui, adapters, obj);
        }
    });
}

/// Preferences › Performance › Graphics Adapter: which GPU draws on a machine with several. The
/// value is the adapter's name; empty is automatic. A saved adapter that isn't present now (an
/// unplugged eGPU) stays listed, marked ⚠, so opening the dialog doesn't silently change the choice.
fn graphics_adapter_row(ui: &mut egui::Ui, adapters: Option<&Value>, obj: &mut Map<String, Value>) {
    let mut current = obj.get("gpuAdapter").and_then(Value::as_str).unwrap_or("").to_string();
    let mut names: Vec<String> = adapters.and_then(Value::as_array).into_iter().flatten().filter_map(Value::as_str).map(str::to_string).collect();
    // A saved name fragment ("nvidia") isn't an adapter's full name but still selects one; only a
    // value no listed adapter matches is marked.
    let kept = !current.is_empty() && !names.contains(&current);
    let missing = kept && !names.iter().any(|n| prefs::gpu_adapter_matches(n, &current));
    if kept {
        names.push(current.clone());
    }
    let automatic = tl!("Automatic");
    let labels: Vec<String> = names.iter().map(|n| if missing && *n == current { format!("⚠ {n}") } else { n.clone() }).collect();
    let mut pairs: Vec<(String, &str)> = vec![(String::new(), automatic)];
    pairs.extend(names.iter().cloned().zip(labels.iter().map(String::as_str)));
    ui.horizontal(|ui| {
        ui.label(tl!("GPU adapter"));
        crate::widgets::dropdown_literal(ui, "graphics-adapter", tl!("GPU adapter"), &mut current, &pairs, 320.0);
    });
    ui.label(RichText::new(tl!("Applies at next launch.")).color(Tokens::get(ui.ctx()).text_faint));
    obj.insert("gpuAdapter".into(), json!(current));
}

/// One user-facing choice; legacy flags remain compatible with older settings.
pub(super) fn rendering_mode_row(ui: &mut egui::Ui, obj: &mut Map<String, Value>) {
    let mut current = rendering_mode_value(obj);
    ui.horizontal(|ui| {
        ui.label(tl!("Rendering Mode"));
        let pairs =
            vec![("auto".to_string(), tl!("Automatic (recommended)")), ("gpu".to_string(), tl!("GPU")), ("cpu".to_string(), tl!("CPU / Compatibility"))];
        let previous = current.clone();
        crate::widgets::dropdown(ui, "rendering-mode", tl!("Rendering Mode"), &mut current, &pairs, 240.0);
        if current != previous {
            obj.insert("renderingMode".into(), json!(current));
            obj.insert("useGpu".into(), json!(current != "cpu"));
        }
    });
    ui.label(tl!("Automatic uses GPU acceleration when available and falls back to CPU rendering on errors."));
    // Next to the setting, as for System Title Bar (#1677): the renderer is picked when the
    // window opens, so a change otherwise looks like it did nothing (#7939).
    ui.label(RichText::new(tl!("Applies at next launch.")).color(Tokens::get(ui.ctx()).text_dim));
    ui.add_space(8.0);
}

pub(super) fn rendering_mode_value(obj: &Map<String, Value>) -> String {
    obj.get("renderingMode").and_then(Value::as_str).map(str::to_string).unwrap_or_else(|| {
        if obj.get("useGpu").and_then(Value::as_bool) == Some(false) || obj.get("gpuBackend").and_then(Value::as_str) == Some("cpu") {
            "cpu".into()
        } else {
            "auto".into()
        }
    })
}

/// Explain the per-document RAM limit separately from the graphics-memory preference.
pub(super) fn memory_usage_row(ui: &mut egui::Ui, obj: &mut Map<String, Value>) {
    let t = Tokens::get(ui.ctx());
    ui.group(|ui| {
        ui.label(RichText::new(tl!("Memory Usage")).strong());
        let mut mb = obj.get("memoryUsageMb").and_then(Value::as_u64).unwrap_or(8192);
        ui.horizontal(|ui| {
            let limit_label = ui.label(tl!("Per-document RAM limit (MB)"));
            if ui.add(egui::DragValue::new(&mut mb).range(256..=1_048_576).suffix(" MB")).labelled_by(limit_label.id).changed() {
                obj.insert("memoryUsageMb".into(), json!(mb));
            }
        });
        if let Some(ram) = crate::gpu_canvas::physical_memory().filter(|n| *n > 0) {
            let installed = ram / (1 << 20);
            let percent = mb as f64 * 100.0 / (ram as f64 / (1 << 20) as f64);
            ui.label(crate::i18n::fmt(tl!("Installed RAM: {ram} MB; this limit: {percent}%"), &[("ram", &installed.to_string()), ("percent", &format!("{percent:.1}"))]));
        }
        ui.label(RichText::new(tl!("Limits each open document's pixels plus History. Oldest undo states are discarded when the limit is exceeded.")).color(t.text_dim));
        ui.label(RichText::new(tl!("Applies immediately when saved. Memory is not reserved in advance.")).color(t.text_dim));
        ui.label(RichText::new(tl!("With GPU Memory set to Automatic, this also limits the GPU layer budget. That budget is capped at one quarter of installed RAM and, when known, three eighths of the graphics card's memory, with a 512 MB minimum. Raising this limit beyond those caps does not increase the GPU budget.")).color(t.text_dim));
    });
    ui.add_space(8.0);
}

/// Settings read only when the app starts, which say "Applies at next launch." under their row.
pub(super) fn read_at_launch(path: &str) -> bool {
    match path {
        // Only Windows and Linux draw their own title bar.
        "interface.systemTitleBar" => cfg!(all(not(target_arch = "wasm32"), not(target_os = "macos"))),
        // The desktop app decides on the control server before its window opens.
        "integrations.allowAgentControl" | "integrations.controlPort" => cfg!(not(target_arch = "wasm32")),
        _ => false,
    }
}
