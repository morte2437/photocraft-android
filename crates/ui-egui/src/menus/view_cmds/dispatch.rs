use super::arrange::{arrange, fit_layers};
use super::dialogs::front;
use super::options::{panel_state, panel_tab, pixel_aspect_ratio, screen_mode_commands, show_slot, snap_slot, was_maximized_id};
use super::{FONT_PREVIEW, LANGUAGE, LAYOUTS, PIXEL_ASPECTS, PhotocraftApp, SCREEN_MODES, Show, SnapTo, Value, json};

/// Ids handled here (live menu items).
pub fn handles(id: &str) -> bool {
    if let Some(k) = id.strip_prefix("view.show.") {
        return show_slot(&mut Show::default(), k).is_some() || k == "all";
    }
    if let Some(k) = id.strip_prefix("view.snapTo.") {
        return snap_slot(&mut SnapTo::default(), k).is_some() || k == "all" || k == "none";
    }
    if let Some(k) = id.strip_prefix("view.screenMode.") {
        return SCREEN_MODES.contains(&k) || k == "cycle";
    }
    if let Some(k) = id.strip_prefix("view.pixelAspectRatio.") {
        return PIXEL_ASPECTS.contains(&k);
    }
    if let Some(k) = id.strip_prefix("type.fontPreviewSize.") {
        return FONT_PREVIEW.contains(&k);
    }
    if let Some(k) = id.strip_prefix("type.languageOptions.") {
        return LANGUAGE.contains(&k) || k == "middleEasternAndSouthAsianComposer";
    }
    if let Some(k) = id.strip_prefix("window.arrange.") {
        return LAYOUTS.contains(&k)
            || matches!(
                k,
                "consolidateAllToTabs"
                    | "cascade"
                    | "floatInWindow"
                    | "floatAllInWindows"
                    | "newWindowForDocument"
                    | "matchZoom"
                    | "matchLocation"
                    | "matchRotation"
                    | "matchAll"
            );
    }
    matches!(
        id,
        "view.extras"
            | "view.twoHundredPercent"
            | "view.printSize"
            | "view.fitLayersOnScreen"
            | "view.fitArtboardOnScreen"
            | "window.panel.layerComps"
            | "view.flipHorizontal"
            | "view.pixelAspectRatioCorrection"
            | "view.patternPreview"
            | "view.pixelArtPreview"
            | "view.rotateView"
            | "view.resetView"
            | "window.panel.layers"
            | "window.panel.history"
            | "window.panel.navigator"
            | "window.panel.properties"
            | "window.panel.color"
            | "window.panel.actions"
            | "window.panel.channels"
            | "window.panel.paths"
            | "window.panel.character"
            | "window.panel.paragraph"
            | "window.panel.info"
            | "window.panel.histogram"
            | "window.panel.swatches"
            | "window.panel.adjustments"
            | "type.panels.character"
            | "type.panels.paragraph"
    )
}

pub fn is_enabled(app: &PhotocraftApp, id: &str) -> Option<bool> {
    is_enabled_with_engine(app, id, || app.session.is_enabled(id))
}

/// Keep picker enablement testable with the engine's native filesystem check disabled.
pub(super) fn is_enabled_with_engine(app: &PhotocraftApp, id: &str, engine_enabled: impl FnOnce() -> bool) -> Option<bool> {
    if !handles(id) && !wraps(id) {
        return None;
    }
    let doc = app.session.active().is_some();
    Some(match id {
        // Full Screen Mode shows a document, so without one it is unavailable, as in Photoshop
        // (#9349); with none open, Cycle can only bring a full screen mode back to Standard.
        "view.screenMode.cycle" => app.ui.text_edit.is_none() && (doc || app.ui.view.screen_mode != "standard"),
        "view.screenMode.fullScreen" | "view.screenMode.fullScreenWithMenuBar" => doc,
        "view.twoHundredPercent" | "view.printSize" | "view.fitLayersOnScreen" | "view.rotateView" | "view.resetView" => doc,
        "view.fitArtboardOnScreen" => app.session.active().is_some_and(|d| d.doc.has_artboards()),
        i if i.starts_with("window.arrange.") => match &i["window.arrange.".len()..] {
            "consolidateAllToTabs" => true,
            "floatInWindow" | "newWindowForDocument" => doc,
            "matchZoom" | "matchLocation" | "matchRotation" | "matchAll" => app.session.documents().len() > 1 || !app.ui.windows.is_empty(),
            _ => {
                app.session.documents().len() > 1
                    || (doc && matches!(&i["window.arrange.".len()..], "floatAllInWindows" | "cascade" | "tile" | "tileAllVertically" | "tileAllHorizontally"))
            }
        },
        // Embedded placement reads the picker's bytes on every platform. Linked placement keeps
        // the engine's native filesystem guard because its source must remain reachable.
        "file.placeEmbedded" => doc && app.services.file_dialog.is_some(),
        // Batch's dialog is built from an Actions-panel action. Keep it disabled until one
        // exists instead of opening a menu item that can only report "record an action".
        "file.automate.batch" => crate::actions::selected_action(app).is_some() && engine_enabled(),
        i if wraps(i) => engine_enabled(),
        _ => true,
    })
}

/// A full screen mode with no document left (the last one closed) goes back to Standard Screen
/// Mode, rather than showing the start screen on the full screen pasteboard (#9349).
pub fn leave_full_screen_without_document(app: &mut PhotocraftApp, ctx: &egui::Context) {
    if app.ui.view.screen_mode != "standard" && app.session.documents().is_empty() {
        let _ = crate::menus::invoke(app, ctx, "view.screenMode.standard", json!({}));
    }
}

pub fn checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let o = &app.ui.view;
    if let Some(k) = id.strip_prefix("view.show.") {
        let mut s = o.show;
        return show_slot(&mut s, k).map(|b| *b);
    }
    if let Some(k) = id.strip_prefix("view.snapTo.") {
        let mut s = o.snap_to;
        return snap_slot(&mut s, k).map(|b| *b);
    }
    if let Some(k) = id.strip_prefix("view.screenMode.") {
        return SCREEN_MODES.contains(&k).then(|| o.screen_mode == k);
    }
    if let Some(k) = id.strip_prefix("view.pixelAspectRatio.") {
        return PIXEL_ASPECTS.contains(&k).then(|| o.pixel_aspect == k);
    }
    if let Some(k) = id.strip_prefix("type.fontPreviewSize.") {
        return FONT_PREVIEW.contains(&k).then(|| o.font_preview_size == k);
    }
    if let Some(k) = id.strip_prefix("type.languageOptions.") {
        return if k == "middleEasternAndSouthAsianComposer" {
            Some(o.middle_eastern_composer)
        } else {
            LANGUAGE.contains(&k).then(|| o.language_features == k)
        };
    }
    if let Some(k) = id.strip_prefix("window.arrange.")
        && LAYOUTS.contains(&k)
    {
        return Some(o.arrange == k);
    }
    if let Some((panel, tab)) = panel_tab(app, id) {
        let (p, t) = (&app.ui.panels, &app.ui.dock_tabs);
        let (vis, cur) = match panel {
            "navigator" => (p.navigator, t.navigator),
            "history" => (p.history, t.history),
            "layers" => (p.layers, t.layers),
            "color" => (p.color, t.color),
            "character" => (p.character, t.character),
            _ => (p.properties, t.properties),
        };
        return Some(vis && cur == tab);
    }
    if let Some(c) = type_checked(app, id) {
        return Some(c);
    }
    Some(match id {
        "view.extras" => o.extras,
        "view.flipHorizontal" => o.flip_horizontal,
        "view.pixelAspectRatioCorrection" => o.pixel_aspect_correction,
        "view.patternPreview" => o.pattern_preview,
        "view.pixelArtPreview" => o.pixel_art_preview,
        _ => return None,
    })
}

/// Checkmarks of the Type menu's anti-aliasing, orientation and OpenType items, for the layer the
/// options bar and Properties show: the one being edited, else the active or a selected type layer.
/// Anti-aliasing and orientation stay check items (`Some(false)`) without a type layer: a native
/// menu can't change an item's kind in place.
fn type_checked(app: &PhotocraftApp, id: &str) -> Option<bool> {
    let t = crate::type_tool::target_text(app);
    if let Some(k) = id.strip_prefix("type.antiAlias.") {
        return Some(t.is_some_and(|t| photocraft_engine::type_extra_cmds::aa_name(t.antialias) == k));
    }
    if let Some(k) = id.strip_prefix("type.orientation.") {
        return Some(t.is_some_and(|t| (t.orientation == photocraft_doc::text::Orientation::Vertical) == (k == "vertical")));
    }
    let t = t?;
    let tag = match id.strip_prefix("type.openType.")? {
        "standardLigatures" => "liga",
        "contextualAlternates" => "calt",
        "discretionaryLigatures" => "dlig",
        "swash" => "swsh",
        "oldstyle" => "onum",
        "stylisticAlternates" => "salt",
        "titlingAlternates" => "titl",
        "ornaments" => "ornm",
        "ordinals" => "ordn",
        "fractions" => "frac",
        _ => return None,
    };
    Some(t.char_runs().first().is_some_and(|r| photocraft_engine::type_extra_cmds::feature_on(&r.style, tag)))
}

/// Engine commands that get a dialog or file picker here when invoked without parameters.
fn wraps(id: &str) -> bool {
    matches!(
        id,
        "file.openAs"
            | "file.openExrParts"
            | "file.saveACopy"
            | "file.placeEmbedded"
            | "file.placeLinked"
            | "layer.smartObjects.replaceContents"
            | "layer.smartObjects.exportContents"
            | "layer.smartObjects.relinkToFile"
            | "layer.smartObjects.convertToLinked"
            | "file.closeAll"
            | "file.closeOthers"
            | "file.fileInfo"
            | "file.automate.fitImage"
            | "file.automate.conditionalModeChange"
            | "file.automate.batch"
            | "file.scripts.imageProcessor"
            | "file.scripts.loadFilesIntoStack"
            | "file.automate.photomerge"
            | "file.automate.mergeToHdrPro"
            | "file.automate.lensCorrection"
            | "file.export.layersToFiles"
            | "file.export.layerCompsToFiles"
            | "file.export.artboardsToFiles"
            | "file.export.artboardsToPdf"
            | "layer.new.artboard"
            | "file.export.colorLookupTables"
            | "view.newGuideLayout"
            | "type.warpText"
            | "type.pasteLoremIpsum"
            | "image.applyImage"
            | "image.calculations"
            | "select.saveSelection"
            | "select.loadSelection"
            | "layer.videoLayers.newVideoLayerFromFile"
            | "layer.videoLayers.replaceFootage"
            | "layer.videoLayers.interpretFootage"
    )
}

pub(super) fn no_params(p: &Value) -> bool {
    p.as_object().is_none_or(|o| o.is_empty())
}

/// Run a View/Window/Type shell command, or front an engine command with its dialog. `None` when
/// `id` isn't ours.
pub fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, params: &Value) -> Option<Result<Value, String>> {
    if wraps(id) {
        return front(app, ctx, id, params);
    }
    if !handles(id) {
        return None;
    }
    Some(run(app, ctx, id, params))
}

fn flag_param(p: &Value, cur: bool) -> bool {
    p.get("on").and_then(Value::as_bool).unwrap_or(!cur)
}

fn run(app: &mut PhotocraftApp, ctx: &egui::Context, id: &str, p: &Value) -> Result<Value, String> {
    if let Some((panel, tab)) = panel_tab(app, id) {
        let group = crate::dock::Group::from_key(panel);
        let collapsed = group.is_some_and(|g| app.ui.dock.is_collapsed(g));
        let (vis, cur) = panel_state(app, panel);
        let requested = match p.get("show").or_else(|| p.get("on")) {
            None => None,
            Some(value) => Some(value.as_bool().ok_or("show must be a boolean")?),
        };
        if let Some(show) = requested {
            *vis = show;
            if show {
                *cur = tab;
            }
        } else {
            // Like Photoshop: choosing a visible panel's menu item again hides it. A collapsed
            // group is expanded instead, so the menu item always brings the panel back (#129).
            if *vis && *cur == tab && !collapsed && !id.starts_with("type.panels.") {
                *vis = false;
            } else {
                *vis = true;
                *cur = tab;
            }
        }
        let visible = *vis;
        if let Some(g) = group.filter(|_| visible) {
            crate::dock::reveal(app, g);
        }
        return Ok(json!({"panel": panel, "tab": tab, "visible": visible}));
    }
    if id == "view.rotateView" {
        return crate::rotate_view::command(app, p);
    }
    if id == "view.resetView" {
        return crate::rotate_view::reset(app);
    }
    let o = &mut app.ui.view;
    if let Some(k) = id.strip_prefix("view.show.") {
        if k == "all" {
            o.show = Show { layer_edges: true, mesh: true, ..Show::default() };
            o.extras = true;
            app.ui.extras.grid = true;
            app.ui.extras.guides = true;
            return Ok(json!(true));
        }
        let slot = show_slot(&mut o.show, k).ok_or("unknown Show item")?;
        *slot = flag_param(p, *slot);
        let v = *slot;
        if v {
            // Showing an item turns Extras back on, as in Photoshop.
            o.extras = true;
        }
        return Ok(json!(v));
    }
    if let Some(k) = id.strip_prefix("view.snapTo.") {
        if k == "all" || k == "none" {
            let on = k == "all";
            o.snap_to = SnapTo { guides: on, grid: on, layers: on, slices: on, document_bounds: on };
            return Ok(json!(on));
        }
        let slot = snap_slot(&mut o.snap_to, k).ok_or("unknown Snap To item")?;
        *slot = flag_param(p, *slot);
        return Ok(json!(*slot));
    }
    if let Some(k) = id.strip_prefix("view.screenMode.") {
        // Full Screen Mode shows a document (#9349): refused without one, from any caller (the
        // toolbar's list, automation), before a viewport command can go out.
        if matches!(k, "fullScreen" | "fullScreenWithMenuBar") && app.session.documents().is_empty() {
            return Err("Full Screen Mode needs an open document".into());
        }
        let next = if k == "cycle" && app.session.documents().is_empty() {
            // Without a document Cycle can only leave a full screen mode (#9349): straight back
            // to Standard, not on to the next full screen mode.
            "standard"
        } else if k == "cycle" {
            let i = SCREEN_MODES.iter().position(|m| *m == o.screen_mode).unwrap_or(0);
            SCREEN_MODES[(i + 1) % SCREEN_MODES.len()]
        } else {
            k
        };
        o.screen_mode = next.to_string();
        // Tab's view of the chrome (#6760) lasts only while in Full Screen Mode.
        app.ui.panels.full_screen_chrome = false;
        // Both full-screen modes take the whole display; the menu bar stays in the first.
        let (full_screen, maximized) = ctx.input(|i| (i.viewport().fullscreen.unwrap_or(false), i.viewport().maximized.unwrap_or(false)));
        let was_maximized = ctx.data(|d| d.get_temp::<bool>(was_maximized_id())).unwrap_or(false);
        let (commands, was_maximized) = screen_mode_commands(cfg!(target_os = "windows"), full_screen, maximized, was_maximized, next);
        ctx.data_mut(|d| d.insert_temp(was_maximized_id(), was_maximized));
        for command in commands {
            ctx.send_viewport_cmd(command);
        }
        return Ok(json!({"screenMode": next}));
    }
    if let Some(k) = id.strip_prefix("view.pixelAspectRatio.") {
        o.pixel_aspect = k.to_string();
        return Ok(json!({"pixelAspectRatio": pixel_aspect_ratio(k)}));
    }
    if let Some(k) = id.strip_prefix("type.fontPreviewSize.") {
        o.font_preview_size = k.to_string();
        return Ok(json!(k));
    }
    if let Some(k) = id.strip_prefix("type.languageOptions.") {
        if k == "middleEasternAndSouthAsianComposer" {
            o.middle_eastern_composer = flag_param(p, o.middle_eastern_composer);
            return Ok(json!(o.middle_eastern_composer));
        }
        o.language_features = k.to_string();
        return Ok(json!(k));
    }
    if let Some(k) = id.strip_prefix("window.arrange.") {
        return arrange(app, k, p);
    }
    match id {
        "view.extras" => {
            o.extras = flag_param(p, o.extras);
            Ok(json!(o.extras))
        }
        "view.flipHorizontal" => {
            o.flip_horizontal = flag_param(p, o.flip_horizontal);
            Ok(json!(o.flip_horizontal))
        }
        "view.pixelAspectRatioCorrection" => {
            o.pixel_aspect_correction = flag_param(p, o.pixel_aspect_correction);
            Ok(json!(o.pixel_aspect_correction))
        }
        "view.patternPreview" => {
            o.pattern_preview = flag_param(p, o.pattern_preview);
            Ok(json!(o.pattern_preview))
        }
        "view.pixelArtPreview" => {
            o.pixel_art_preview = flag_param(p, o.pixel_art_preview);
            Ok(json!(o.pixel_art_preview))
        }
        "view.twoHundredPercent" | "view.printSize" => {
            let i = app.session.active_index().ok_or("no document")?;
            // Print Size shows the document at its print size for the screen resolution
            // (Preferences ▸ Units & Rulers: Screen Resolution, 72 ppi by default).
            let dpi = app.session.active().map_or(72.0, |d| d.doc.resolution_dpi.max(1.0));
            let size = app.session.active().map_or([0, 0], |d| [d.doc.size.width, d.doc.size.height]);
            let screen = app.session.prefs().units_and_rulers.screen_resolution.max(1.0) as f32;
            let z = if id == "view.twoHundredPercent" { 2.0 } else { screen / dpi };
            app.ui.views[i].zoom = crate::zoom_levels::clamp(z, size);
            app.ui.views[i].request_zoom_animation(app.session.prefs().general.animated_zoom);
            app.ui.views[i].zoom_anchor = None;
            Ok(json!({"zoom": app.ui.views[i].zoom}))
        }
        "view.fitLayersOnScreen" => fit_layers(app),
        "view.fitArtboardOnScreen" => crate::artboard_ui::fit_artboard(app),
        _ => Err(format!("unhandled {id}")),
    }
}
