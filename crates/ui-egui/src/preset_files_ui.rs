//! Preset files opened like documents (File › Open, drag-and-drop, the Brushes panel's
//! Import Brushes…, the Preset Manager's Load…): Photoshop brushes (`.abr`) go to the brush
//! library through `brush.presets.importAbr`, gradients (`.grd`) to the Gradients panel through
//! `gradient.presets.importGrd`, layer styles (`.asl`) to the Styles panel through
//! `style.presets.importAsl`, swatches (`.aco`, `.act`, `.ase`) to the Swatches panel through
//! `swatches.import`, patterns (`.pat`) to the Patterns panel through `pattern.import`, keyboard
//! shortcut sets (`.kys`) to Edit › Keyboard Shortcuts through
//! [`crate::kys_import`], instead of opening as documents. Import Brushes… also takes an image
//! file, which becomes a brush tip through `brush.presets.importImage` (TOOL-213-12).

use serde_json::{Value, json};

use crate::PhotocraftApp;

/// Extensions handled here rather than by the document importer (`.aco`/`.act`/`.ase` swatches go to
/// the Swatches panel through [`crate::swatches_ui::import_bytes`]).
/// `.psp` is Photoshop's live `Keyboard Shortcuts.psp` (the same XML as a `.kys`); other `.psp`
/// settings files are refused with a clear message.
pub const PRESET_EXTS: &[&str] = &["abr", "grd", "asl", "pat", "aco", "act", "ase", "kys", "psp"];

fn ext(name: &str) -> String {
    std::path::Path::new(name).extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Is `name` a preset file this module imports?
pub fn is_preset_file(name: &str) -> bool {
    PRESET_EXTS.contains(&ext(name).as_str())
}

/// Image files Import Brushes… offers besides `.abr`: each becomes a sampled brush tip.
pub const BRUSH_IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "tif", "tiff", "psd", "psb", "bmp", "gif", "webp", "tga"];

/// Brushes panel / brush preset picker › Import Brushes…: an `.abr` joins the brush library as a
/// group (as File › Open does), an image file becomes a new brush preset with that image as its tip.
pub fn import_brushes(app: &mut PhotocraftApp) -> Result<Value, String> {
    let exts: Vec<&str> = std::iter::once("abr").chain(BRUSH_IMAGE_EXTS.iter().copied()).collect();
    app.pick_file_bytes_filtered(&exts, |app, name, bytes| match open(app, &name, &bytes) {
        Some(r) => r.map(|()| Value::Null),
        None => import_brush_image(app, &name, &bytes),
    })
}

/// An image file as a brush tip: a new preset named after the file, made the current brush.
pub fn import_brush_image(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Result<Value, String> {
    let file = std::path::Path::new(name).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_else(|| name.to_string());
    let mut p = source(name, bytes);
    if p.get("data").is_some() {
        p["fileName"] = json!(file);
    }
    let r = app.run("brush.presets.importImage", p)?;
    let preset = r.get("name").and_then(Value::as_str).unwrap_or_default();
    app.ui.status_error = false;
    app.ui.status = format!("Imported brush {preset} from {file}");
    // Show it: Brush Settings › Brushes tab.
    app.ui.panels.brush_settings = true;
    app.ui.brush_tab = 1;
    Ok(r)
}

/// Import from a gradient picker through the same preset command as File › Open.
pub(crate) fn import_gradients_dialog(app: &mut PhotocraftApp) {
    let result = app.pick_file_bytes_filtered(&["grd"], |app, name, bytes| {
        if ext(&name) != "grd" {
            return Err(tl!("Choose a gradient file (.grd).").into());
        }
        open(app, &name, &bytes).ok_or_else(|| tl!("Choose a gradient file (.grd).").to_string())??;
        Ok(Value::Null)
    });
    if let Err(error) = result
        && error != crate::file_dialog::CANCELLED
    {
        app.ui.status = error;
        app.ui.status_error = true;
    }
}

/// Command params for the file: a real path keeps the journal small; browser files arrive as bytes.
fn source(name: &str, bytes: &[u8]) -> Value {
    #[cfg(not(target_arch = "wasm32"))]
    if std::path::Path::new(name).is_absolute() && std::path::Path::new(name).is_file() {
        return json!({"path": name});
    }
    let _ = name;
    json!({"data": photocraft_engine::paint::tile::b64_encode(bytes)})
}

/// Import a preset file; `None` when `name` isn't one.
pub fn open(app: &mut PhotocraftApp, name: &str, bytes: &[u8]) -> Option<Result<(), String>> {
    let (cmd, what) = match ext(name).as_str() {
        "abr" => ("brush.presets.importAbr", "brushes"),
        "grd" => ("gradient.presets.importGrd", "gradients"),
        "asl" => ("style.presets.importAsl", "styles"),
        "pat" => ("pattern.import", "patterns"),
        "aco" | "act" | "ase" => return Some(crate::swatches_ui::import_bytes(app, name, bytes, false).map(|_| ())),
        "kys" | "psp" => return Some(crate::kys_import::open_bytes(app, name, bytes).map(|_| ())),
        _ => return None,
    };
    let stem = std::path::Path::new(name).file_stem().map(|s| s.to_string_lossy().to_string()).unwrap_or_else(|| tl!("Imported").into());
    let mut p = source(name, bytes);
    p["group"] = json!(stem);
    if what == "brushes" {
        p["select"] = json!(true);
    }
    Some(app.run(cmd, p).map(|r| {
        // A background import (#210): `jobs_ui` reports the count when it lands.
        if r.get("pending").and_then(Value::as_bool) == Some(true) {
            app.ui.status_error = false;
            let template = "Importing {what} from {file}…";
            app.ui.status = crate::i18n::fmt(tl!(template), &[("what", tl!(what)), ("file", &stem)]);
            return;
        }
        let n = r.get("count").and_then(Value::as_u64).unwrap_or(0);
        let warnings: Vec<String> = r.get("warnings").and_then(|w| serde_json::from_value(w.clone()).ok()).unwrap_or_default();
        app.ui.status_error = false;
        let (one, other) = match what {
            "brushes" => ("Imported {n} brush from {file}", "Imported {n} brushes from {file}"),
            "styles" => ("Imported {n} style from {file}", "Imported {n} styles from {file}"),
            "patterns" => ("Imported {n} pattern from {file}", "Imported {n} patterns from {file}"),
            _ => ("Imported {n} gradient from {file}", "Imported {n} gradients from {file}"),
        };
        let imported = crate::i18n::fmt(&crate::i18n::trn(crate::i18n::current(), n, one, other), &[("file", &stem)]);
        app.ui.status = match warnings.first() {
            Some(warning) => {
                let template = "{message} (notes: {n}; {warning})";
                crate::i18n::fmt(tl!(template), &[("message", &imported), ("n", &warnings.len().to_string()), ("warning", warning)])
            }
            None => imported,
        };
        if what == "brushes" {
            // Show them: Brush Settings › Brushes tab.
            app.ui.panels.brush_settings = true;
            app.ui.brush_tab = 1;
        }
        if what == "styles" {
            app.ui.presets_ui.styles = true;
        }
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_preset_files() {
        assert!(is_preset_file("/a/b/Set.ABR"));
        assert!(is_preset_file("sunsets.grd"));
        assert!(is_preset_file("Comic Styles.ASL"));
        assert!(is_preset_file("Brand.ACO"));
        assert!(is_preset_file("exchange.ase"));
        assert!(is_preset_file("Debaser-ColourLibrary-Part6.PAT"));
        assert!(!is_preset_file("photo.psd"));
        assert!(!is_preset_file("abr"));
    }

    /// Import Brushes… with a PNG: the image becomes the current brush's tip (TOOL-213-12).
    #[test]
    fn import_brushes_turns_an_image_into_a_brush_tip() {
        use photocraft_engine::paint::TipShape;
        let mut px = vec![255u8; 16 * 16];
        for y in 4..12 {
            for x in 6..9 {
                px[y * 16 + x] = 0;
            }
        }
        let img = photocraft_codecs::Image::from_u8(16, 16, photocraft_codecs::ChannelLayout::Gray, px).unwrap();
        let png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        let (show, asked) = crate::file_dialog::fake(vec![Some(crate::file_dialog::FileDialogAnswer::Contents("Leaf.png".into(), png))]);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() });
        import_brushes(&mut app).unwrap();
        app.poll_file_dialog(&egui::Context::default(), None);
        match asked.borrow().first() {
            Some(crate::file_dialog::FileDialogRequest::Open { extensions: Some(e), .. }) => {
                assert!(e.iter().any(|x| x == "abr") && e.iter().any(|x| x == "png"), "{e:?}")
            }
            other => panic!("expected a filtered open dialog, got {other:?}"),
        }
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(app.session.tools.current_preset.as_deref(), Some("Leaf"));
        let TipShape::Sampled(tip) = &app.session.tools.brush.tip else { panic!("sampled tip expected") };
        assert_eq!((tip.width, tip.height), (3, 8));
        assert!(app.ui.panels.brush_settings && app.ui.brush_tab == 1, "the Brushes tab shows the new brush");
    }
}
