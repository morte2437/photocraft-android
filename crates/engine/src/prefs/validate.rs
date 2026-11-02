use super::shortcuts::normalize_shortcut;
use super::{
    AppearanceMode, Ask, CanvasBorder, CanvasColor, CheckerColors, CheckerSize, ColorPicker, DarkTheme, ExportLocation, ExportMetadata, FontPreview,
    GpuBackend, Interpolation, LightTheme, LineStyle, LinuxDisplayServer, LogDestination, LogDetail, OtherCursor, PaintingCursor, PointSize, PressureCurve,
    QuickExportFormat, RawColorSpace, RawDemosaic, RawDepth, RawSharpen, RenderingMode, RightClickPaint, TextEngine, Theme, TypeUnit, UiFontSize, UiScale,
    Unit, Value,
};

/// Preferences dialog sections in Photoshop's order: (id, title).
pub const SECTIONS: [(&str, &str); 18] = [
    ("general", "General"),
    ("interface", "Interface"),
    ("workspace", "Workspace"),
    ("tools", "Tools"),
    ("historyLog", "History Log"),
    ("fileHandling", "File Handling"),
    ("export", "Export"),
    ("performance", "Performance"),
    ("scratchDisks", "Scratch Disks"),
    ("cursors", "Cursors"),
    ("transparencyAndGamut", "Transparency & Gamut"),
    ("unitsAndRulers", "Units & Rulers"),
    ("guidesGridAndSlices", "Guides, Grid & Slices"),
    ("plugIns", "Plug-ins"),
    ("type", "Type"),
    ("enhancedControls", "Enhanced Controls"),
    ("rawDefaults", "Camera Raw Defaults"),
    ("integrations", "Integrations"),
];

/// Preferences that nothing reads yet, so Edit › Preferences doesn't show them: a control that
/// does nothing is worse than a missing one (https://github.com/storytold/photocraft/issues/204).
/// They still load, save and round-trip through `prefs.get` / `prefs.set` unchanged.
///
/// When you implement one, remove it here; the `prefs_usage` test fails while a listed
/// preference is read anywhere outside `prefs.rs`, and while an unlisted one is read nowhere.
pub const HIDDEN_UNTIL_IMPLEMENTED: &[&str] = &[
    "general.colorPicker",
    "workspace.autoShowHiddenPanels",
    "workspace.enableFloatingDocumentWindowDocking",
    "performance.effectCacheMb",
    "performance.legacyCompositing",
    "scratchDisks.disks",
    "unitsAndRulers.columnWidth",
    "unitsAndRulers.gutter",
    "plugIns.showExtensionPanels",
    "plugIns.allowScriptsToConnect",
    "type.textEngine",
    "enhancedControls.scrubbySliderAcceleration",
];

/// Is the preference at `path` (`"section.key"`) hidden from the Preferences dialog?
pub fn is_hidden(path: &str) -> bool {
    HIDDEN_UNTIL_IMPLEMENTED.contains(&path) || (!cfg!(target_os = "linux") && LINUX_ONLY.contains(&path))
}

/// Preferences that only do something on Linux; the dialog doesn't show them elsewhere. They
/// still load, save and round-trip on every platform.
pub const LINUX_ONLY: &[&str] = &["performance.linuxDisplayServer", "performance.gpuAdapter"];

/// Whether a graphics adapter called `name` is the one `performance.gpuAdapter` (`wanted`) asks
/// for: every whitespace-separated word of `wanted` appears in the name, ignoring case ("nvidia",
/// "arc a370m"). An empty `wanted` matches nothing. Startup and Preferences share this.
pub fn gpu_adapter_matches(name: &str, wanted: &str) -> bool {
    let name = name.to_lowercase();
    let mut words = wanted.split_whitespace().peekable();
    words.peek().is_some() && words.all(|w| name.contains(&w.to_lowercase()))
}

/// Choices of an enumerated preference (dotted path, e.g. `"cursors.painting"`).
pub fn choices(path: &str) -> Option<&'static [&'static str]> {
    Some(match path {
        "general.colorPicker" => ColorPicker::NAMES,
        "general.imageInterpolation" => Interpolation::NAMES,
        "interface.theme" => Theme::NAMES,
        "interface.appearanceMode" => AppearanceMode::NAMES,
        "interface.darkTheme" => DarkTheme::NAMES,
        "interface.lightTheme" => LightTheme::NAMES,
        "interface.canvasColor" => CanvasColor::NAMES,
        "interface.canvasBorder" => CanvasBorder::NAMES,
        "interface.uiScale" => UiScale::NAMES,
        "interface.uiFontSize" => UiFontSize::NAMES,
        "historyLog.destination" => LogDestination::NAMES,
        "historyLog.detail" => LogDetail::NAMES,
        "fileHandling.imagePreviews" | "fileHandling.maximizePsdCompatibility" => Ask::NAMES,
        "export.quickExportFormat" => QuickExportFormat::NAMES,
        "export.quickExportLocation" => ExportLocation::NAMES,
        "export.metadata" => ExportMetadata::NAMES,
        "cursors.painting" => PaintingCursor::NAMES,
        "cursors.other" => OtherCursor::NAMES,
        "tools.rightClickWithPaintingTools" => RightClickPaint::NAMES,
        "transparencyAndGamut.gridSize" => CheckerSize::NAMES,
        "transparencyAndGamut.gridColors" => CheckerColors::NAMES,
        "unitsAndRulers.rulers" | "guidesGridAndSlices.gridUnit" => Unit::NAMES,
        "unitsAndRulers.typeUnits" => TypeUnit::NAMES,
        "unitsAndRulers.pointSize" => PointSize::NAMES,
        "guidesGridAndSlices.guideStyle" | "guidesGridAndSlices.gridStyle" => LineStyle::NAMES,
        "type.textEngine" => TextEngine::NAMES,
        "type.fontPreview" => FontPreview::NAMES,
        "rawDefaults.colorSpace" => RawColorSpace::NAMES,
        "rawDefaults.bitDepth" => RawDepth::NAMES,
        "rawDefaults.sharpenFor" => RawSharpen::NAMES,
        "rawDefaults.demosaic" => RawDemosaic::NAMES,
        "performance.gpuBackend" => GpuBackend::NAMES,
        "performance.linuxDisplayServer" => LinuxDisplayServer::NAMES,
        "performance.renderingMode" => RenderingMode::NAMES,
        _ => return None,
    })
}

/// Valid range of a numeric preference.
pub fn range(path: &str) -> Option<(f64, f64)> {
    Some(match path {
        "fileHandling.autosaveMinutes" => (1.0, 240.0),
        "fileHandling.recentFileCount" => (0.0, 100.0),
        "tools.nudgeBundlePauseMs" => (100.0, 10_000.0),
        "fileHandling.rasterizeSvgGroupsDeeperThan" => (0.0, photocraft_doc::MAX_GROUP_DEPTH as f64),
        "interface.notificationDurationSeconds" => (1.0, 120.0),
        "export.jpegQuality" | "export.webpQuality" => (1.0, 100.0),
        "performance.memoryUsageMb" => (256.0, 1_048_576.0),
        "performance.gpuMemoryMb" => (0.0, 1_048_576.0),
        "performance.historyStates" => (1.0, 1000.0),
        "performance.cacheLevels" => (1.0, 8.0),
        "performance.cacheTileSize" => (256.0, 16384.0),
        "performance.effectCacheMb" => (16.0, 65536.0),
        "transparencyAndGamut.gamutWarningOpacity" => (1.0, 100.0),
        "unitsAndRulers.columnWidth" | "unitsAndRulers.gutter" => (0.0, 10_000.0),
        "unitsAndRulers.printResolution" | "unitsAndRulers.screenResolution" | "rawDefaults.resolution" => (1.0, 10_000.0),
        "guidesGridAndSlices.gridlineEvery" => (0.001, 10_000.0),
        "guidesGridAndSlices.subdivisions" => (1.0, 100.0),
        "layerMaskOverlay.opacity" => (0.0, 100.0),
        "type.recentFonts" => (0.0, 50.0),
        "integrations.controlPort" => (0.0, 65535.0),
        _ => return None,
    })
}

/// Is this preference a `#rrggbb` colour?
pub fn is_color(path: &str) -> bool {
    if choices(path).is_some() || path.starts_with("shortcuts.") || path.starts_with("alternateShortcuts.") || path.starts_with("menus.") {
        return false;
    }
    if path == "layerMaskOverlay.color" {
        return true;
    }
    let leaf = path.rsplit('.').next().unwrap_or(path);
    matches!(
        leaf,
        "guideColor"
            | "smartGuideColor"
            | "gridColor"
            | "sliceColor"
            | "canvasCustomColor"
            | "brushPreviewColor"
            | "gamutWarningColor"
            | "customLight"
            | "customDark"
    )
}

pub fn parse_hex(s: &str) -> Option<[u8; 3]> {
    let s = s.trim().trim_start_matches('#');
    if s.len() != 6 {
        return None;
    }
    let b = |i: usize| s.get(i..i + 2).and_then(|s| u8::from_str_radix(s, 16).ok());
    Some([b(0)?, b(2)?, b(4)?])
}

// ------------------------------------------------------------------ JSON paths

/// Map keys that are command ids (which contain dots): `shortcuts.<id>`,
/// `alternateShortcuts.<id>`, `menus.colors.<id>`.
pub(super) fn keyed(path: &str) -> Option<(&str, &str)> {
    for prefix in ["shortcuts.", "alternateShortcuts.", "menus.colors."] {
        if let Some(id) = path.strip_prefix(prefix) {
            return Some((&prefix[..prefix.len() - 1], id));
        }
    }
    None
}

pub(super) fn get_path<'a>(root: &'a Value, path: &str) -> Option<&'a Value> {
    if path.is_empty() {
        return Some(root);
    }
    if let Some((map, id)) = keyed(path) {
        return get_path(root, map)?.get(id);
    }
    path.split('.').try_fold(root, |v, k| v.get(k))
}

/// Set `value` at `path` (creating keys only inside free-form maps such as `shortcuts`).
pub(super) fn set_path(root: &mut Value, path: &str, value: Value) -> std::result::Result<(), String> {
    if let Some((map, id)) = keyed(path) {
        let mut cur = &mut *root;
        for k in map.split('.') {
            cur = cur.get_mut(k).ok_or_else(|| format!("unknown preference `{path}`"))?;
        }
        cur.as_object_mut().ok_or_else(|| format!("`{map}` is not a map"))?.insert(id.to_string(), value);
        return Ok(());
    }
    let keys: Vec<&str> = path.split('.').filter(|k| !k.is_empty()).collect();
    let Some((last, parents)) = keys.split_last() else { return Err("empty path".into()) };
    let mut cur = root;
    for k in parents {
        cur = cur.get_mut(*k).ok_or_else(|| format!("unknown preference `{path}`"))?;
    }
    let free_map = matches!(parents.first(), Some(&"shortcuts")) || (parents.len() == 2 && parents[0] == "menus" && parents[1] == "colors");
    let obj = cur.as_object_mut().ok_or_else(|| format!("`{path}` is not inside a section"))?;
    if !obj.contains_key(*last) && !free_map {
        return Err(format!("unknown preference `{path}`"));
    }
    obj.insert(last.to_string(), value);
    Ok(())
}

/// Validate one value for `path` before it is stored (choices, ranges, colours).
pub(super) fn check_value(path: &str, v: &Value) -> std::result::Result<(), String> {
    if path == "interface.favoriteThemes" {
        let items = v.as_array().ok_or("`interface.favoriteThemes` must be a list of theme ids")?;
        if items.len() > Theme::NAMES.len() {
            return Err("`interface.favoriteThemes` contains too many themes".into());
        }
        let mut seen = Vec::with_capacity(items.len());
        for item in items {
            let theme = item.as_str().and_then(Theme::parse).ok_or_else(|| format!("unknown favorite theme `{item}`"))?;
            if seen.contains(&theme) {
                return Err(format!("duplicate favorite theme `{}`", theme.name()));
            }
            seen.push(theme);
        }
    }
    if path == "performance.renderingMode" && v.is_null() {
        return Ok(()); // Legacy policy, resolved from useGpu and gpuBackend.
    }
    if let Some(c) = choices(path) {
        let s = v.as_str().ok_or_else(|| format!("`{path}` must be one of {}", c.join("|")))?;
        if !c.contains(&s) {
            return Err(format!("`{path}` must be one of {} (got `{s}`)", c.join("|")));
        }
    }
    if path == "interface.language" {
        let ok = v.as_str().is_some_and(|s| !s.is_empty() && s.len() <= 16 && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'));
        if !ok {
            return Err("`interface.language` must be `auto` or a language code such as `en` or `ja`".into());
        }
    }
    if let Some((lo, hi)) = range(path) {
        let x = v.as_f64().ok_or_else(|| format!("`{path}` must be a number"))?;
        if !(lo..=hi).contains(&x) {
            return Err(format!("`{path}` must be within {lo}..{hi} (got {x})"));
        }
    }
    if is_color(path) && v.as_str().and_then(parse_hex).is_none() {
        return Err(format!("`{path}` must be a #rrggbb colour"));
    }
    if path == "tools.pressureCurve" {
        let pts = v.as_array().ok_or("`tools.pressureCurve` must be a list of [input, output] points")?;
        if !(2..=PressureCurve::MAX_POINTS).contains(&pts.len()) {
            return Err(format!("`tools.pressureCurve` needs 2..={} points (got {})", PressureCurve::MAX_POINTS, pts.len()));
        }
        for p in pts {
            let ok = p.as_array().is_some_and(|a| a.len() == 2 && a.iter().all(|x| x.as_f64().is_some_and(|x| (0.0..=1.0).contains(&x))));
            if !ok {
                return Err(format!("`tools.pressureCurve` points are [input, output] pairs within 0..1 (got {p})"));
            }
        }
    }
    if let Some(sc) = path.strip_prefix("shortcuts.") {
        let s = v.as_str().ok_or_else(|| format!("shortcut for `{sc}` must be a string"))?;
        if !s.is_empty() && normalize_shortcut(s).is_none() {
            return Err(format!("`{s}` is not a valid shortcut (e.g. Cmd+Shift+N, F7)"));
        }
    }
    Ok(())
}
