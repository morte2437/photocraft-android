use super::invoke::{panel_alias, proof_preset, workspace_name};
use super::state::{checked, is_enabled, reveal_label};
use super::{DOCUMENT_ITEM, PhotocraftApp, UI_COMMANDS};

/// Menu tree entry for rendering and for `ui.inspect`.
#[derive(Clone, Debug, serde::Serialize)]
pub struct MenuItem {
    pub id: String,
    pub label: String,
    pub path: Vec<String>,
    pub shortcut: Option<String>,
    pub enabled: bool,
    pub checked: Option<bool>,
    /// Edit › Menus colour (red, orange, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
}

/// Is `id` implemented by the engine or the shell (a live menu item)? Shared by the menus and
/// the parity report ([`crate::parity`]).
pub fn is_live(id: &str) -> bool {
    static UI_IDS: std::sync::OnceLock<std::collections::HashSet<&'static str>> = std::sync::OnceLock::new();
    photocraft_engine::commands::find(id).is_some()
        || crate::tool_shortcuts::tool(id).is_some()
        || UI_IDS.get_or_init(|| UI_COMMANDS.iter().map(|c| c.0).collect()).contains(id)
        || panel_alias(id).is_some()
        || workspace_name(id).is_some()
        || proof_preset(id).is_some()
        || id == "view.proofSetup.custom"
        || crate::view_cmds::handles(id)
        || crate::analysis_ui::handles(id)
        || crate::workspace_ui::handles(id)
        || crate::preset_panels::handles(id)
        || crate::type_panels_ui::handles(id)
        || crate::timeline_ui::handles(id)
}

/// Commands outside the catalogue that belong right after a catalogue item: `(id, after)`.
/// The command takes that item's submenu path, so it joins Photoshop's submenu even where the
/// catalogue spells it differently (Filter › "Other…").
const PLACE_AFTER: &[(&str, &str)] = &[
    ("file.newFromClipboard", "file.new"),
    ("filter.render.relight", "filter.render.lightingEffects"),
    ("view.resetView", "view.flipHorizontal"),
    ("filter.other.colorToAlpha", "filter.other.offset"),
];

pub fn menu_items(app: &PhotocraftApp) -> Vec<MenuItem> {
    // 1) Photoshop's full menu tree, in Photoshop order; live where we implement the command.
    let known = is_live;
    let mut items: Vec<MenuItem> = crate::menu_catalog::CATALOG
        .iter()
        .map(|&(path, label, sc, id)| MenuItem {
            id: id.to_string(),
            label: label.to_string(),
            path: path.iter().map(|s| s.to_string()).collect(),
            // A live item shows the shortcut that runs it ([`crate::shortcut_dispatch::bindings`]
            // prefers the command's own over the catalogue's): Undo ⌘Z, Copy ⌘C, Hide Layers ⌘,.
            shortcut: if known(id) { crate::shortcuts::default_shortcut(id) } else { sc.map(Into::into) },
            enabled: known(id) && is_enabled(app, id),
            checked: checked(app, id),
            color: None,
        })
        .collect();
    // 2) Our commands that Photoshop's tree doesn't list (or lists under another id).
    let mut extra: Vec<MenuItem> = Vec::new();
    for &(id, label, path, sc) in UI_COMMANDS {
        extra.push(MenuItem {
            id: id.into(),
            label: label.into(),
            path: path.iter().map(|s| s.to_string()).collect(),
            shortcut: sc.map(Into::into),
            enabled: is_enabled(app, id),
            checked: checked(app, id),
            color: None,
        });
    }
    for c in photocraft_engine::command_specs().iter().filter(|c| !c.menu.is_empty()) {
        extra.push(MenuItem {
            id: c.id.into(),
            label: c.label.into(),
            path: c.menu.iter().map(|s| s.to_string()).collect(),
            shortcut: c.shortcut.map(Into::into),
            enabled: is_enabled(app, c.id),
            checked: None,
            color: None,
        });
    }
    for mut e in extra {
        let dup = items.iter().any(|i| i.id == e.id || (i.path == e.path && i.label.trim_end_matches('…') == e.label.trim_end_matches('…')));
        if !dup {
            // Insert after the item it belongs next to, else after the last item of the same
            // top-level menu, keeping menus contiguous.
            let top = e.path.first().cloned();
            let after = PLACE_AFTER.iter().find(|(id, _)| *id == e.id).and_then(|(_, a)| items.iter().position(|i| i.id == *a));
            if let Some(path) = after.and_then(|a| items.get(a)).map(|i| i.path.clone()) {
                e.path = path;
            }
            let at = after.or_else(|| items.iter().rposition(|i| i.path.first() == top.as_ref())).map_or(items.len(), |p| p + 1);
            items.insert(at, e);
        }
    }
    crate::plugin_ui::insert_menu_items(app, &mut items);
    if let Some(mask) = app.session.active().and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id))).and_then(|l| l.mask.as_ref())
        && let Some(item) = items.iter_mut().find(|i| i.id == "layer.layerMask.enabled")
    {
        item.label = crate::layer_menu_ui::mask_toggle_label(mask.enabled).into();
    }
    // Layer › Merge Layers reads Merge Group with just a group selected, as in Photoshop (#3832).
    if let Some(st) = app.session.active()
        && st.selected_layers().len() <= 1
        && st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Group(_)))
        && let Some(item) = items.iter_mut().find(|i| i.id == "layer.mergeLayers")
    {
        item.label = "Merge Group".into();
    }
    // The File quick-export command uses the format selected in Export Preferences.
    // On the web it intentionally downloads PNG until the quick-export service supports
    // the other formats; the Layer quick-export command also always produces PNG.
    if let Some(item) = items.iter_mut().find(|i| i.id == "file.export.quickExportAsPng") {
        use photocraft_engine::prefs::QuickExportFormat;
        let format = if cfg!(target_arch = "wasm32") {
            "PNG"
        } else {
            match app.session.prefs().export.quick_export_format {
                QuickExportFormat::Png => "PNG",
                QuickExportFormat::Jpg => "JPG",
                QuickExportFormat::Gif => "GIF",
                QuickExportFormat::Webp => "WebP",
                QuickExportFormat::Exr => "EXR",
            }
        };
        item.label = format!("Quick Export as {format}");
    }
    // Smart Objects › Reveal in Finder names this platform's file manager (UI-217-19).
    if let Some(item) = items.iter_mut().find(|i| i.id == "layer.smartObjects.revealInFinder") {
        item.label = reveal_label().into();
    }
    // File › Open Recent: a dynamic submenu of recently opened files (inserted after "Open As…").
    if let Some(after) = items.iter().position(|i| i.id == "file.openAs") {
        let rp: Vec<String> = vec!["File".into(), "Open Recent".into()];
        let mut recent: Vec<MenuItem> = app
            .ui
            .recent_files
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let label = std::path::Path::new(p).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| p.clone());
                MenuItem { id: format!("file.openRecent.{i}"), label, path: rp.clone(), shortcut: None, enabled: true, checked: None, color: None }
            })
            .collect();
        if !recent.is_empty() {
            recent.push(MenuItem { id: "---".into(), label: "---".into(), path: rp.clone(), shortcut: None, enabled: false, checked: None, color: None });
        }
        recent.push(MenuItem {
            id: "file.clearRecent".into(),
            label: "Clear Recent Files".into(),
            path: rp.clone(),
            shortcut: None,
            enabled: !app.ui.recent_files.is_empty(),
            checked: None,
            color: None,
        });
        for (k, it) in recent.into_iter().enumerate() {
            items.insert(after + 1 + k, it);
        }
    }
    // Window › Workspace: saved workspaces follow the presets, checked while current.
    let saved = crate::workspace_ui::saved_names(app);
    if !saved.is_empty()
        && let Some(after) = items.iter().position(|i| i.id == "window.workspace.motion")
    {
        let wp: Vec<String> = vec!["Window".into(), "Workspace".into()];
        let separator = MenuItem { id: "---".into(), label: "---".into(), path: wp.clone(), shortcut: None, enabled: false, checked: None, color: None };
        let entries = saved.into_iter().enumerate().map(|(i, name)| {
            let id = format!("{}{i}", crate::workspace_ui::SAVED);
            let checked = checked(app, &id);
            MenuItem { id, label: name, path: wp.clone(), shortcut: None, enabled: true, checked, color: None }
        });
        for (k, it) in std::iter::once(separator).chain(entries).enumerate() {
            items.insert(after + 1 + k, it);
        }
    }
    // Keep the update command with the links, ahead of System Info and About.
    if let Some(at) = items.iter().position(|i| i.id == "help.checkForUpdates") {
        let update = items.remove(at);
        let before = items.iter().position(|i| i.id == "help.systemInfo" || i.id == "help.about").unwrap_or(items.len());
        items.insert(before, update);
    }
    // Help: the link items and update check, a separator, then System Info and About.
    if let Some(at) = items.iter().position(|i| i.id == "help.systemInfo" || i.id == "help.about") {
        items.insert(
            at,
            MenuItem { id: "---".into(), label: "---".into(), path: vec!["Help".into()], shortcut: None, enabled: false, checked: None, color: None },
        );
    }
    // Window: the open documents in tab order, after a separator, the active one checked.
    if let Some(last) = items.iter().rposition(|i| i.path.first().map(String::as_str) == Some("Window")) {
        let window = || vec!["Window".to_string()];
        let active = app.session.active_index();
        let docs = app.session.documents().iter().enumerate().map(|(i, st)| MenuItem {
            id: format!("{DOCUMENT_ITEM}{i}"),
            label: st.doc.name.clone(),
            path: window(),
            shortcut: None,
            enabled: true,
            checked: Some(active == Some(i)),
            color: None,
        });
        let separator = MenuItem { id: "---".into(), label: "---".into(), path: window(), shortcut: None, enabled: false, checked: None, color: None };
        let rows: Vec<MenuItem> = std::iter::once(separator).chain(docs).collect();
        if rows.len() > 1 {
            items.splice(last + 1..last + 1, rows);
        }
    }
    // Edit › Keyboard Shortcuts overrides, Edit › Menus hidden items and colours.
    let prefs = app.session.prefs();
    if !prefs.shortcuts.is_empty() {
        for it in &mut items {
            if prefs.shortcuts.contains_key(&it.id) {
                it.shortcut = prefs.shortcut(&it.id, None).map(str::to_string);
            }
        }
    }
    if !prefs.menus.hidden.is_empty() {
        items.retain(|i| !prefs.menus.is_hidden(&i.id));
    }
    if prefs.interface.show_menu_colors {
        for it in &mut items {
            it.color = prefs.menus.colors.get(&it.id).cloned();
        }
    }
    // Layer › Create Clipping Mask is Photoshop's one Create / Release item: when its shortcut
    // would release ([`crate::shortcut_dispatch::clip_key_releases`]) it reads Release Clipping
    // Mask and runs it, keeping that shortcut. The engine's own Release item lives in that slot.
    items.retain(|i| i.id != "layer.releaseClippingMask");
    if crate::shortcut_dispatch::clip_key_releases(app)
        && let Some(item) = items.iter_mut().find(|i| i.id == "layer.createClippingMask")
    {
        item.id = "layer.releaseClippingMask".into();
        item.label = "Release Clipping Mask".into();
        item.enabled = is_enabled(app, &item.id);
    }
    items
}

/// Background tint of an Edit › Menus colour name.
pub(super) fn menu_tint(name: &str) -> Option<egui::Color32> {
    Some(match name {
        "red" => egui::Color32::from_rgb(190, 60, 60),
        "orange" => egui::Color32::from_rgb(200, 120, 40),
        "yellow" => egui::Color32::from_rgb(190, 170, 40),
        "green" => egui::Color32::from_rgb(60, 150, 70),
        "blue" => egui::Color32::from_rgb(50, 110, 200),
        "violet" => egui::Color32::from_rgb(130, 80, 190),
        "gray" => egui::Color32::from_rgb(120, 120, 120),
        _ => return None,
    })
}

/// The open menus' items, kept between frames: building them asks every command whether it can
/// run, which on a document with thousands of layers costs far more than a frame.
pub(crate) struct ItemCache {
    /// [`crate::native_menu::state_hash`] of the state the items were built from.
    pub(super) state: u64,
    pub(super) items: Vec<MenuItem>,
}

impl ItemCache {
    /// The items, while they still describe `state`. A frame with a click or key press rebuilds
    /// them, which covers state the hash doesn't list.
    pub(super) fn reuse(self, state: u64, input: bool) -> Option<Vec<MenuItem>> {
        (!input && self.state == state).then_some(self.items)
    }
}
