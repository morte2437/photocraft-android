use super::*;

/// Whether the window was maximized when a screen mode took it full screen (egui temp data).
pub(super) fn was_maximized_id() -> egui::Id {
    egui::Id::unique("pc-full-screen-was-maximized")
}

/// The window commands for switching to screen mode `next`, and whether the window should be
/// maximized again when it leaves full screen. With `keep_apart` (Windows), full screen and
/// maximized never overlap: a maximized window is un-maximized before it goes full screen and
/// maximized again after it leaves. Why: winit on Windows keeps the maximized style on a window
/// that goes full screen from maximized, and the shell then draws the taskbar black instead of
/// hiding it; on the way out the window stays maximized at the monitor's full size, over the
/// taskbar (#3550, #4018, #4230). [OBSERVED 2026-10-12, Windows 11, eframe 0.36.2, winit 0.30.13,
/// a minimal eframe window; un-maximizing first gave a clean full screen and a work-area maximize.]
///
/// WORKAROUND(winit#4756): remove `keep_apart` once the winit in Cargo.lock (0.30.13 today) has
/// the fix; check first with https://gist.github.com/aweussom/3f80ffb82df7f0102341858afbbffd9b
/// (`--max`, no `--fix`: full screen must have no WS_MAXIMIZE and hide the taskbar).
pub(super) fn screen_mode_commands(
    keep_apart: bool,
    full_screen: bool,
    maximized: bool,
    was_maximized: bool,
    next: &str,
) -> (Vec<egui::ViewportCommand>, bool) {
    let want = next != "standard";
    let mut commands = Vec::new();
    let mut remember = was_maximized;
    if keep_apart && want && !full_screen && maximized {
        commands.push(egui::ViewportCommand::Maximized(false));
        remember = true;
    }
    commands.push(egui::ViewportCommand::Fullscreen(want));
    if !want {
        if keep_apart && full_screen && remember {
            commands.push(egui::ViewportCommand::Maximized(true));
        }
        remember = false;
    }
    (commands, remember)
}

/// Pixel aspect ratio (width / height of a pixel) of a preset id.
pub fn pixel_aspect_ratio(id: &str) -> f32 {
    // View › Pixel Aspect Ratio › Custom: `custom:<ratio>:<name>` (see `workspace_ui`).
    if let Some(r) = id.strip_prefix("custom:").and_then(|r| r.split(':').next()?.parse::<f32>().ok()) {
        return r;
    }
    match id {
        "d1DvNtsc" => 0.91,
        "d1DvPal" => 1.09,
        "d1DvNtscWidescreen" => 1.21,
        "hdv1080" => 1.33,
        "d1DvPalWidescreen" => 1.46,
        "dvcproHd1080" => 1.5,
        "anamorphic2To1" => 2.0,
        _ => 1.0,
    }
}

pub(super) fn show_slot<'a>(s: &'a mut Show, key: &str) -> Option<&'a mut bool> {
    Some(match key {
        "layerEdges" => &mut s.layer_edges,
        "selectionEdges" => &mut s.selection_edges,
        "targetPath" => &mut s.target_path,
        "pixelGrid" => &mut s.pixel_grid,
        "smartGuides" => &mut s.smart_guides,
        "brushPreview" => &mut s.brush_preview,
        "slices" => &mut s.slices,
        "notes" => &mut s.notes,
        "mesh" => &mut s.mesh,
        "editPins" => &mut s.edit_pins,
        "count" => &mut s.count,
        "canvasGuides" => &mut s.canvas_guides,
        "artboardGuides" => &mut s.artboard_guides,
        _ => return None,
    })
}

pub(super) fn snap_slot<'a>(s: &'a mut SnapTo, key: &str) -> Option<&'a mut bool> {
    Some(match key {
        "guides" => &mut s.guides,
        "grid" => &mut s.grid,
        "layers" => &mut s.layers,
        "slices" => &mut s.slices,
        "documentBounds" => &mut s.document_bounds,
        _ => return None,
    })
}

/// Every [`show_slot`] key.
const SHOW_KEYS: [&str; 13] = [
    "layerEdges",
    "selectionEdges",
    "targetPath",
    "pixelGrid",
    "smartGuides",
    "brushPreview",
    "slices",
    "notes",
    "mesh",
    "editPins",
    "count",
    "canvasGuides",
    "artboardGuides",
];

/// Every [`snap_slot`] key.
const SNAP_KEYS: [&str; 5] = ["guides", "grid", "layers", "slices", "documentBounds"];

/// The flags of `now` that differ from `def`, by key (`None` when none do).
fn changed_flags<T: Copy>(now: T, def: T, keys: &[&str], slot: impl for<'a> Fn(&'a mut T, &str) -> Option<&'a mut bool>) -> Option<Value> {
    let (mut now, mut def) = (now, def);
    let diff: Map<String, Value> = keys
        .iter()
        .filter_map(|k| {
            let v = *slot(&mut now, k)?;
            (Some(v) != slot(&mut def, k).copied()).then(|| ((*k).to_string(), Value::Bool(v)))
        })
        .collect();
    (!diff.is_empty()).then_some(Value::Object(diff))
}

/// View › Extras, Show and Snap To as remembered across restarts (`Preferences::view_extras`):
/// only what differs from the defaults, so a default changed in a later release still reaches
/// users who never touched that item. `Null` when nothing differs.
fn remembered(app: &PhotocraftApp) -> Value {
    let (v, e, def) = (&app.ui.view, &app.ui.extras, crate::state::Extras::default());
    let mut m = Map::new();
    if v.extras != ViewOptions::default().extras {
        m.insert("extras".into(), Value::Bool(v.extras));
    }
    // View › Show › Grid and Guides live in `UiState::extras` beside the rulers and snapping.
    if e.grid != def.grid {
        m.insert("grid".into(), Value::Bool(e.grid));
    }
    if e.guides != def.guides {
        m.insert("guides".into(), Value::Bool(e.guides));
    }
    if e.snap != def.snap {
        m.insert("snap".into(), Value::Bool(e.snap));
    }
    if let Some(show) = changed_flags(v.show, Show::default(), &SHOW_KEYS, show_slot) {
        m.insert("show".into(), show);
    }
    if let Some(snap) = changed_flags(v.snap_to, SnapTo::default(), &SNAP_KEYS, snap_slot) {
        m.insert("snapTo".into(), snap);
    }
    if m.is_empty() { Value::Null } else { Value::Object(m) }
}

/// Remember View › Extras, Show and Snap To in the preferences once the user lets go of the
/// pointer (#3762). Cheap: a small JSON compare per frame. Run every frame, so items set by the
/// control channel or the Show Extras Options dialog are remembered too.
pub fn persist(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if ctx.input(|i| i.pointer.any_down()) {
        return;
    }
    let now = remembered(app);
    if app.session.prefs().view_extras != now {
        app.session.prefs.edit(|p| p.view_extras = now);
    }
}

/// Restore the remembered View › Extras, Show and Snap To items at launch. A hand-edited file
/// can't break them: an unknown key or a value that isn't a boolean keeps its default.
pub fn restore(app: &mut PhotocraftApp) {
    let saved = app.session.prefs().view_extras.clone();
    if !saved.is_null() && !saved.is_object() {
        log::warn!("Preferences: the remembered View › Show items are not an object; using the defaults");
        return;
    }
    let flag = |k: &str| saved.get(k).and_then(Value::as_bool);
    if let Some(on) = flag("extras") {
        app.ui.view.extras = on;
    }
    if let Some(on) = flag("grid") {
        app.ui.extras.grid = on;
    }
    if let Some(on) = flag("guides") {
        app.ui.extras.guides = on;
    }
    if let Some(on) = flag("snap") {
        app.ui.extras.snap = on;
    }
    for (k, v) in saved.get("show").and_then(Value::as_object).into_iter().flatten() {
        if let (Some(slot), Some(on)) = (show_slot(&mut app.ui.view.show, k), v.as_bool()) {
            *slot = on;
        }
    }
    for (k, v) in saved.get("snapTo").and_then(Value::as_object).into_iter().flatten() {
        if let (Some(slot), Some(on)) = (snap_slot(&mut app.ui.view.snap_to, k), v.as_bool()) {
            *slot = on;
        }
    }
}

/// Window › <panel>: (panel, dock tab) for panels that live as tabs of a dock card.
pub(super) fn panel_tab(app: &PhotocraftApp, id: &str) -> Option<(&'static str, usize)> {
    let pro = app.ui.theme.is_pro();
    Some(match id {
        "window.panel.info" => ("navigator", 2),
        "window.panel.histogram" => ("navigator", 1),
        "window.panel.navigator" => ("navigator", 0),
        "window.panel.actions" => ("history", 1),
        "window.panel.layerComps" => ("history", 2),
        "window.panel.history" => ("history", 0),
        "window.panel.channels" => ("layers", 1),
        "window.panel.paths" => ("layers", 2),
        "window.panel.layers" => ("layers", 0),
        "window.panel.adjustments" => ("properties", 1),
        "window.panel.properties" => ("properties", 0),
        // Their own Character | Paragraph group, so Properties stays open (#150).
        "window.panel.character" | "type.panels.character" => ("character", 0),
        "window.panel.paragraph" | "type.panels.paragraph" => ("character", 1),
        "window.panel.swatches" => ("color", usize::from(pro)),
        "window.panel.color" => ("color", usize::from(!pro)),
        _ => return None,
    })
}

pub(super) fn panel_state<'a>(app: &'a mut PhotocraftApp, panel: &str) -> (&'a mut bool, &'a mut usize) {
    let (p, t) = (&mut app.ui.panels, &mut app.ui.dock_tabs);
    match panel {
        "navigator" => (&mut p.navigator, &mut t.navigator),
        "history" => (&mut p.history, &mut t.history),
        "layers" => (&mut p.layers, &mut t.layers),
        "color" => (&mut p.color, &mut t.color),
        "character" => (&mut p.character, &mut t.character),
        _ => (&mut p.properties, &mut t.properties),
    }
}
