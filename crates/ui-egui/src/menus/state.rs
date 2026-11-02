use super::invoke::{panel_alias, proof_preset, workspace_name};
use super::items::MenuItem;
use super::{DOCUMENT_ITEM, PhotocraftApp, Value};

pub fn is_enabled(app: &PhotocraftApp, id: &str) -> bool {
    if id == "layer.selectUnderCursor" {
        return app.session.active().is_some() && app.drag.is_none() && app.brush_resize.is_none();
    }
    if crate::tool_shortcuts::tool(id).is_some() {
        return true;
    }
    if id == "ui.symmetryTransform" {
        return crate::symmetry_ui::can_transform(app);
    }
    // Draft Pen points can be undone even when the document has no History entries.
    if id == "edit.undo" && app.ui.pen.as_ref().is_some_and(|pen| !pen.knots.is_empty()) {
        return true;
    }
    // Photoshop greys these for the Background layer, other layer kinds or single-layer documents.
    if crate::enable_rules::disabled(app, id) {
        return false;
    }
    // A pending crop greys what Photoshop greys during one (#1918).
    if crate::crop_ui::blocks(app, id) {
        return false;
    }
    // Filters stay live on a type layer: choosing one asks to rasterize it (#4303).
    if crate::rasterize_prompt::filter_needs(app, id).is_some() {
        return true;
    }
    if let Some(e) = crate::workspace_ui::is_enabled(app, id) {
        return e;
    }
    if crate::analysis_ui::handles(id) {
        return true;
    }
    if crate::preset_panels::handles(id) || crate::type_panels_ui::handles(id) || crate::timeline_ui::handles(id) {
        return true;
    }
    if let Some(e) = crate::view_cmds::is_enabled(app, id) {
        return e;
    }
    if let Some(e) = crate::plugin_ui::is_enabled(app, id) {
        return e;
    }
    if let Some(e) = crate::transform_tool::is_enabled(app, id) {
        return e;
    }
    if let Some(e) = crate::crop_ui::is_enabled(app, id) {
        return e;
    }
    match id {
        "window.panel.activate" | "window.panel.layout" | "ui.dock" => true,
        "window.panel.group"
        | "window.panel.move"
        | "window.panel.float"
        | "window.panel.window"
        | "window.panel.dock"
        | "window.panel.close"
        | "window.panel.closeGroup"
        | "window.panel.closeOthers"
        | "window.panel.minimize"
        | "window.panel.iconic" => !app.session.prefs().workspace_locked,
        "file.open" | "file.exit" | "file.clearRecent" | "file.removeRecent" | "help.about" | "help.systemInfo" | "edit.search" => true,
        i if i.starts_with("file.openRecent.") => true,
        i if i.starts_with(DOCUMENT_ITEM) => true,
        i if crate::links::url_for(i).is_some() => true,
        i if i.starts_with("window.theme.") => true,
        // Interface brightness works with or without a document, as in Photoshop.
        "view.darkenInterface" | "view.lightenInterface" => true,
        "file.save" | "file.saveAs" | "file.export.exportAs" | "file.export.quickExportAsPng" => {
            app.session.active().is_some() && app.services.export.is_some()
        }
        i if i.starts_with("window.toggle.") => true,
        "window.togglePanels" => true,
        // With one document open the key stays quiet, as in Photoshop, rather than reporting why.
        "window.nextDocument" | "window.previousDocument" => app.session.active().is_some(),
        i if panel_alias(i).is_some() || workspace_name(i).is_some() => true,
        i if proof_preset(i).is_some() => app.session.active().is_some(),
        // "Custom…" is the full Proof Setup dialog.
        "view.proofSetup.custom" => app.session.active().is_some(),
        "view.rulers" | "view.show.grid" | "view.show.guides" | "view.snap" | "view.lockGuides" => true,
        "edit.cut" | "edit.copy" if app.ui.text_edit.is_some() => crate::type_tool::selected_range(app).is_some(),
        "edit.paste" | "select.all" if app.ui.text_edit.is_some() => true,
        // An image copied in another app can only be seen by reading the OS clipboard, which happens
        // on an explicit paste: with a clipboard service these stay enabled. Paste and New from
        // Clipboard need no document (with none open, Paste makes one); Paste in Place does.
        "edit.paste" | "file.newFromClipboard" => app.session.is_enabled(id) || app.services.clipboard_get_image.is_some(),
        "edit.pasteSpecial.pasteInPlace" => app.session.is_enabled(id) || (app.services.clipboard_get_image.is_some() && app.session.active().is_some()),
        "select.selectAndMask" => app.session.is_enabled("select.refineEdge"),
        "type.editText" => app
            .session
            .active()
            .and_then(|s| s.active_layer.and_then(|id| s.doc.layer(id)))
            .is_some_and(|l| matches!(l.content, photocraft_doc::LayerContent::Text(_))),
        "select.transformSelection" => app.ui.transform.is_none() && app.session.is_enabled("select.transformSelection"),
        // ⇧[ / ⇧] only step the hardness of a tool that paints with the brush tip, as in Photoshop.
        "tools.decreaseBrushHardness" | "tools.increaseBrushHardness" => app.ui.tool.is_brushlike(),
        i if (i.starts_with("view.zoom") || i == "view.fitOnScreen" || i == "view.actualPixels") || i == "window.newWindowForDocument" => {
            app.session.active().is_some()
        }
        "edit.freeTransformCopy" => app.ui.transform.is_none() && app.session.active().and_then(|s| s.active_layer).is_some(),
        "edit.freeTransform"
        | "edit.transform.scale"
        | "edit.transform.rotate"
        | "edit.transform.skew"
        | "edit.transform.distort"
        | "edit.transform.perspective" => match &app.ui.transform {
            Some(t) => t.warp.is_none(),
            None => app.session.active().and_then(|s| s.active_layer).is_some(),
        },
        // A targeted layer mask enables what edits it (Invert on an adjustment layer's mask).
        i => app.session.is_enabled_with(i, &app.with_mask_target(i, Value::Null)),
    }
}

/// Image › Mode items that carry a checkmark.
const MODE_CHECKS: [&str; 11] = ["rgb", "grayscale", "cmyk", "lab", "multichannel", "indexedColor", "bitmap", "duotone", "bits8", "bits16", "bits32"];

/// Is a UI-level panel toggle currently on (for checkmarks)?
pub(super) fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    if let Some(checked) = crate::panel_docking::checked(app, id) {
        return Some(checked);
    }
    use photocraft_doc::{ColorMode, SampleType};
    if let Some(name) = id.strip_prefix("window.theme.").filter(|n| *n != "toggle") {
        let auto = app.session.prefs().interface.appearance_mode == photocraft_engine::prefs::AppearanceMode::Auto;
        return Some(if name == "system" { auto } else { !auto && crate::theme::ThemeKind::from_name(name) == Some(app.ui.theme) });
    }
    if let Some(c) = crate::view_cmds::checked(app, id) {
        return Some(c);
    }
    if let Some(c) = crate::analysis_ui::checked(app, id).or_else(|| crate::workspace_ui::checked(app, id)).or_else(|| crate::file_ui::checked(app, id)) {
        return Some(c);
    }
    if let Some(c) = crate::preset_panels::checked(app, id) {
        return Some(c);
    }
    if let Some(c) = crate::timeline_ui::checked(app, id) {
        return Some(c);
    }
    if let Some(c) = crate::type_panels_ui::checked(app, id) {
        return Some(c);
    }
    if let Some(alias) = panel_alias(id) {
        return checked(app, alias);
    }
    // Check items stay check items with no document open (`Some(false)`, not `None`): a native
    // menu can't change an item's kind in place, so a change would rebuild the whole menu.
    if id == "select.isolateLayers" {
        return Some(app.session.active().is_some_and(|d| !d.isolated_layers.is_empty()));
    }
    if id == "view.proofColors" || id == "view.gamutWarning" {
        let Some(d) = app.session.active() else { return Some(false) };
        let pv = app.session.color.proof(d.doc.id);
        return Some(if id == "view.proofColors" { pv.enabled } else { pv.gamut_warning });
    }
    if id.starts_with("window.workspace.") && id != "window.workspace.resetWorkspace" {
        let want = workspace_name(id)?;
        return Some(app.ui.workspace == if want.is_empty() { "Essentials" } else { want });
    }
    if let Some(m) = id.strip_prefix("image.mode.") {
        let Some(d) = app.session.active().map(|s| &s.doc) else {
            return MODE_CHECKS.contains(&m).then_some(false);
        };
        return match m {
            "rgb" => Some(d.mode == ColorMode::Rgb),
            "grayscale" => Some(d.mode == ColorMode::Grayscale),
            "cmyk" => Some(d.mode == ColorMode::Cmyk),
            "lab" => Some(d.mode == ColorMode::Lab),
            "multichannel" => Some(d.mode == ColorMode::Multichannel),
            "indexedColor" => Some(d.mode == ColorMode::Indexed),
            "bitmap" => Some(d.mode == ColorMode::Bitmap),
            "duotone" => Some(d.mode == ColorMode::Duotone),
            "bits8" => Some(d.depth == SampleType::U8),
            "bits16" => Some(d.depth == SampleType::U16),
            "bits32" => Some(d.depth == SampleType::F32),
            _ => None,
        };
    }
    let e = &app.ui.extras;
    match id {
        "view.rulers" => return Some(e.rulers),
        "view.show.grid" => return Some(e.grid),
        "view.show.guides" => return Some(e.guides),
        "view.snap" => return Some(e.snap),
        "view.lockGuides" => return Some(e.lock_guides),
        _ => {}
    }
    let p = &app.ui.panels;
    Some(match id {
        "window.toggle.layers" => p.layers,
        "window.toggle.history" => p.history,
        "window.toggle.properties" => p.properties,
        "window.toggle.color" => p.color,
        "window.toggle.navigator" => p.navigator,
        "window.toggle.toolbar" => p.toolbar,
        "window.toggle.dock" => p.dock,
        "window.togglePanels" => p.toolbar || p.options_bar || p.dock,
        "window.toggle.options" => p.options_bar,
        "window.toggle.brushSettings" => p.brush_settings,
        "window.toggle.brushes" => p.brushes,
        _ => return None,
    })
}

/// "Reveal in Finder" named after the file manager the engine opens on this platform (UI-217-19):
/// Explorer on Windows, a plain folder elsewhere (xdg-open, the web). English action key; each
/// display translates it.
pub(crate) fn reveal_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Reveal in Finder"
    } else if cfg!(windows) {
        "Show in Explorer"
    } else {
        "Show in Folder"
    }
}

/// Translate the fixed command label and substitute the currently configured export format.
/// Reuse the existing translated PNG sentence, so dynamic formats work in every UI language.
pub(super) fn translated_menu_label(lang: crate::i18n::Lang, item: &MenuItem) -> String {
    // A document's name is shown as it is.
    if item.id.starts_with(DOCUMENT_ITEM) {
        return item.label.clone();
    }
    if item.id == "file.export.quickExportAsPng"
        && let Some(fmt) = item.label.strip_prefix("Quick Export as ")
    {
        return crate::i18n::tr_id(lang, &item.id, "Quick Export as PNG").replace("PNG", fmt);
    }
    crate::i18n::tr_id(lang, &item.id, &item.label).to_string()
}
