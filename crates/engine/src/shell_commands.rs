//! The commands the app's shell carries out itself (view, panels, windows: they change no
//! document), and the tools, as data: what they're called and which keys they have by default.
//! The shell implements them (`photocraft_ui_egui::menus::UI_COMMANDS`, its `Tool`); the engine
//! knows them so a headless session sees every bindable command: Edit › Keyboard Shortcuts'
//! list, conflicts, `.kys` import and export all work without the UI (the UI is a layer on
//! top). The app's tests keep these tables equal to its own.

/// Shell commands: id, label, menu path, default shortcut.
pub const SHELL_COMMANDS: &[(&str, &str, &[&str], Option<&str>)] = &[
    ("ui.symmetryTransform", "Transform Symmetry", &[], None),
    // Right-click the options bar's tool icon (#2825); `tool.resetAll` asks first.
    ("tool.reset", "Reset Tool", &[], None),
    ("tool.resetAll", "Reset All Tools", &[], None),
    ("window.panel.group", "Change Panel Group", &[], None),
    ("window.panel.layout", "Change Panel Layout", &[], None),
    ("window.panel.move", "Move Panel", &[], None),
    ("window.panel.float", "Float Panel", &[], None),
    ("window.panel.dock", "Dock Panel", &[], None),
    ("window.panel.activate", "Activate Panel", &[], None),
    ("window.panel.close", "Close Panel", &[], None),
    ("file.open", "Open…", &["File"], Some("Cmd+O")),
    ("file.save", "Save", &["File"], Some("Cmd+S")),
    ("file.saveAs", "Save As…", &["File"], Some("Cmd+Shift+S")),
    ("file.exit", "Exit", &["File"], Some("Cmd+Q")),
    ("edit.freeTransform", "Free Transform", &["Edit"], Some("Cmd+T")),
    ("file.export.exportAs", "Export As…", &["File", "Export"], Some("Cmd+Alt+Shift+W")),
    ("file.export.quickExportAsPng", "Quick Export as PNG", &["File", "Export"], None),
    ("edit.transform.scale", "Scale", &["Edit", "Transform"], None),
    ("edit.transform.rotate", "Rotate", &["Edit", "Transform"], None),
    ("edit.transform.skew", "Skew", &["Edit", "Transform"], None),
    ("edit.transform.distort", "Distort", &["Edit", "Transform"], None),
    ("edit.transform.perspective", "Perspective", &["Edit", "Transform"], None),
    ("select.selectAndMask", "Select and Mask…", &["Select"], Some("Cmd+Alt+R")),
    ("view.rulers", "Rulers", &["View"], Some("Cmd+R")),
    ("view.show.grid", "Grid", &["View", "Show"], Some("Cmd+'")),
    ("view.show.guides", "Guides", &["View", "Show"], Some("Cmd+;")),
    ("view.snap", "Snap", &["View"], Some("Cmd+Shift+;")),
    ("view.lockGuides", "Lock Guides", &["View"], Some("Cmd+Alt+;")),
    ("view.extras", "Extras", &["View"], Some("Cmd+H")),
    ("view.show.targetPath", "Target Path", &["View", "Show"], Some("Cmd+Shift+H")),
    ("view.screenMode.cycle", "Cycle Screen Mode", &[], Some("F")),
    // Photoshop's interface brightness shortcuts: step through the four Pro themes.
    ("view.darkenInterface", "Darken Interface", &[], Some("Cmd+Shift+F1")),
    ("view.lightenInterface", "Lighten Interface", &[], Some("Cmd+Shift+F2")),
    ("edit.freeTransformCopy", "Free Transform a Copy", &[], Some("Cmd+Alt+T")),
    ("type.editText", "Edit Type", &[], None),
    ("view.zoomIn", "Zoom In", &["View"], Some("Cmd+=")),
    ("view.zoomOut", "Zoom Out", &["View"], Some("Cmd+-")),
    ("view.fitOnScreen", "Fit on Screen", &["View"], Some("Cmd+0")),
    ("view.actualPixels", "100%", &["View"], Some("Cmd+1")),
    ("view.rotateView", "Rotate View", &[], None),
    ("view.resetView", "Reset View", &["View"], None),
    ("window.newWindowForDocument", "New Window for Document", &["Window", "Arrange"], None),
    ("window.toggle.layers", "Layers", &["Window"], Some("F7")),
    ("window.toggle.history", "History", &["Window"], None),
    ("window.toggle.properties", "Properties", &["Window"], None),
    ("window.toggle.color", "Color", &["Window"], Some("F6")),
    ("window.toggle.brushSettings", "Brush Settings", &["Window"], Some("F5")),
    ("window.toggle.brushes", "Brushes", &["Window"], None),
    ("window.toggle.navigator", "Navigator", &["Window"], None),
    ("window.toggle.toolbar", "Tools", &["Window"], None),
    // Photoshop's Tab and ⇧Tab: all panels (Tools, options bar and the dock), or only the dock.
    ("window.togglePanels", "Show/Hide All Panels", &[], Some("Tab")),
    ("window.toggle.dock", "Show/Hide Panels", &[], Some("Shift+Tab")),
    ("window.toggle.options", "Options", &["Window"], None),
    // Photoshop's Ctrl+Tab and Ctrl+Shift+Tab, on macOS too (⌘Tab belongs to the system, #2340).
    ("window.nextDocument", "Next Document", &[], Some("Ctrl+Tab")),
    ("window.previousDocument", "Previous Document", &[], Some("Ctrl+Shift+Tab")),
    ("window.theme.toggle", "Next Appearance Mode", &["Window"], None),
    ("window.theme.pro", "Pro", &["Window", "Theme", "Dark Theme"], None),
    ("window.theme.proMedium", "Pro Medium Gray", &["Window", "Theme", "Dark Theme"], None),
    ("window.theme.studio", "Studio", &["Window", "Theme", "Dark Theme"], None),
    ("window.theme.solarizedDark", "Solarized", &["Window", "Theme", "Dark Theme"], None),
    ("window.theme.adwaitaDark", "Adwaita", &["Window", "Theme", "Dark Theme"], None),
    ("window.theme.breezeDark", "Breeze", &["Window", "Theme", "Dark Theme"], None),
    ("window.theme.softDark", "Soft", &["Window", "Theme", "Dark Theme"], None),
    ("window.theme.proLightGray", "Pro Light Gray", &["Window", "Theme", "Light Theme"], None),
    ("window.theme.proLight", "Pro Light", &["Window", "Theme", "Light Theme"], None),
    ("window.theme.studioLight", "Studio", &["Window", "Theme", "Light Theme"], None),
    ("window.theme.classic", "Classic", &["Window", "Theme", "Light Theme"], None),
    ("window.theme.adwaita", "Adwaita", &["Window", "Theme", "Light Theme"], None),
    ("window.theme.breezeLight", "Breeze", &["Window", "Theme", "Light Theme"], None),
    ("window.theme.softLight", "Soft", &["Window", "Theme", "Light Theme"], None),
    ("window.theme.system", "Sync with system", &["Window", "Theme"], None),
    ("edit.search", "Search…", &["Edit"], Some("Cmd+F")),
    ("help.discord", "Join the ArtCraft Discord…", &["Help"], None),
    ("help.website", "PhotoCraft Website", &["Help"], None),
    ("help.artcraftWebsite", "ArtCraft Website", &["Help"], None),
    ("help.github", "PhotoCraft on GitHub", &["Help"], None),
    ("help.reportIssue", "Report an Issue…", &["Help"], None),
    ("help.systemInfo", "System Info…", &["Help"], None),
    ("help.about", "About PhotoCraft", &["Help"], None),
];

/// The tools, in toolbar order: shortcut id (`tools.select.<tool>`), Photoshop's name for it
/// (ours too), and its default key. Tools that share a key cycle with Shift (Photoshop's
/// tool groups), so a shared default letter isn't a conflict.
pub const TOOLS: &[(&str, &str, Option<&str>)] = &[
    ("tools.select.move", "Move Tool", Some("V")),
    ("tools.select.artboard", "Artboard Tool", Some("V")),
    ("tools.select.rectMarquee", "Rectangular Marquee Tool", Some("M")),
    ("tools.select.ellipseMarquee", "Elliptical Marquee Tool", Some("M")),
    ("tools.select.singleRowMarquee", "Single Row Marquee Tool", None),
    ("tools.select.singleColumnMarquee", "Single Column Marquee Tool", None),
    ("tools.select.lasso", "Lasso Tool", Some("L")),
    ("tools.select.polygonLasso", "Polygonal Lasso Tool", Some("L")),
    ("tools.select.magneticLasso", "Magnetic Lasso Tool", Some("L")),
    ("tools.select.magicWand", "Magic Wand Tool", Some("W")),
    ("tools.select.crop", "Crop Tool", Some("C")),
    ("tools.select.perspectiveCrop", "Perspective Crop Tool", Some("C")),
    ("tools.select.eyedropper", "Eyedropper Tool", Some("I")),
    ("tools.select.colorSampler", "Color Sampler Tool", Some("I")),
    ("tools.select.ruler", "Ruler Tool", Some("I")),
    ("tools.select.note", "Note Tool", Some("I")),
    ("tools.select.count", "Count Tool", Some("I")),
    ("tools.select.brush", "Brush Tool", Some("B")),
    ("tools.select.pencil", "Pencil Tool", Some("B")),
    ("tools.select.mixerBrush", "Mixer Brush Tool", Some("B")),
    ("tools.select.colorReplacement", "Color Replacement Tool", Some("B")),
    ("tools.select.eraser", "Eraser Tool", Some("E")),
    ("tools.select.backgroundEraser", "Background Eraser Tool", Some("E")),
    ("tools.select.magicEraser", "Magic Eraser Tool", Some("E")),
    ("tools.select.gradient", "Gradient Tool", Some("G")),
    ("tools.select.paintBucket", "Paint Bucket Tool", Some("G")),
    ("tools.select.type", "Horizontal Type Tool", Some("T")),
    ("tools.select.verticalType", "Vertical Type Tool", Some("T")),
    ("tools.select.horizontalTypeMask", "Horizontal Type Mask Tool", Some("T")),
    ("tools.select.verticalTypeMask", "Vertical Type Mask Tool", Some("T")),
    ("tools.select.hand", "Hand Tool", Some("H")),
    ("tools.select.rotateView", "Rotate View Tool", Some("R")),
    ("tools.select.zoom", "Zoom Tool", Some("Z")),
    ("tools.select.remove", "Remove Tool", Some("J")),
    ("tools.select.spotHealing", "Spot Healing Brush Tool", Some("J")),
    ("tools.select.healing", "Healing Brush Tool", Some("J")),
    ("tools.select.patch", "Patch Tool", Some("J")),
    ("tools.select.contentAwareMove", "Content-Aware Move Tool", Some("J")),
    ("tools.select.redEye", "Red Eye Tool", Some("J")),
    ("tools.select.cloneStamp", "Clone Stamp Tool", Some("S")),
    ("tools.select.patternStamp", "Pattern Stamp Tool", Some("S")),
    ("tools.select.historyBrush", "History Brush Tool", Some("Y")),
    ("tools.select.blur", "Blur Tool", None),
    ("tools.select.sharpen", "Sharpen Tool", None),
    ("tools.select.smudge", "Smudge Tool", None),
    ("tools.select.dodge", "Dodge Tool", Some("O")),
    ("tools.select.burn", "Burn Tool", Some("O")),
    ("tools.select.sponge", "Sponge Tool", Some("O")),
    ("tools.select.quickSelection", "Quick Selection Tool", Some("W")),
    ("tools.select.objectSelection", "Object Selection Tool", Some("W")),
    ("tools.select.pen", "Pen Tool", Some("P")),
    ("tools.select.freeformPen", "Freeform Pen Tool", Some("P")),
    ("tools.select.curvaturePen", "Curvature Pen Tool", Some("P")),
    ("tools.select.addAnchorPoint", "Add Anchor Point Tool", Some("P")),
    ("tools.select.deleteAnchorPoint", "Delete Anchor Point Tool", Some("P")),
    ("tools.select.convertPoint", "Convert Point Tool", Some("P")),
    ("tools.select.pathSelection", "Path Selection Tool", Some("A")),
    ("tools.select.directSelection", "Direct Selection Tool", Some("A")),
    ("tools.select.rectangle", "Rectangle Tool", Some("U")),
    ("tools.select.ellipseShape", "Ellipse Tool", Some("U")),
    ("tools.select.triangle", "Triangle Tool", Some("U")),
    ("tools.select.polygon", "Polygon Tool", Some("U")),
    ("tools.select.line", "Line Tool", Some("U")),
    ("tools.select.customShape", "Custom Shape Tool", Some("U")),
    ("tools.select.slice", "Slice Tool", Some("C")),
    ("tools.select.sliceSelect", "Slice Select Tool", Some("C")),
    ("tools.select.frame", "Frame Tool", Some("K")),
];

/// The default key of tool shortcut `id`, if it's a tool.
pub fn tool_default(id: &str) -> Option<Option<&'static str>> {
    TOOLS.iter().find(|t| t.0 == id).map(|t| t.2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_keys_are_valid() {
        let mut ids: Vec<&str> = SHELL_COMMANDS.iter().map(|c| c.0).chain(TOOLS.iter().map(|t| t.0)).collect();
        let n = ids.len();
        ids.sort_unstable();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate ids");
        for (id, _, _, sc) in SHELL_COMMANDS {
            assert!(sc.is_none_or(|k| crate::prefs::normalize_shortcut(k).is_some()), "{id}: {sc:?}");
        }
        for (id, name, key) in TOOLS {
            assert!(id.starts_with("tools.select.") && name.ends_with("Tool"), "{id} {name}");
            assert!(key.is_none_or(|k| k.len() == 1), "{id}: {key:?}");
        }
        assert_eq!(tool_default("tools.select.move"), Some(Some("V")));
        assert_eq!(tool_default("edit.copy"), None);
    }
}
