//! File › New › Advanced Options › Pixel Aspect Ratio (#14034): the dialog's entries, and the
//! ratio Create gives the new document.

use serde_json::{Map, Value};

use crate::PhotocraftApp;
use crate::workspace_ui::CustomPar;

/// The dialog field holding the picked entry: a View › Pixel Aspect Ratio id (`square`,
/// `d1DvNtsc`…, or `custom:<ratio>:<name>`). Shell-only, so `file.new` never sees it.
pub const FIELD: &str = "__pixelAspect";

/// The Pixel Aspect Ratio menu as (id, label): the presets in the order Photoshop's New Document
/// lists them (Anamorphic 2:1 before DVCPRO HD 1080, unlike View › Pixel Aspect Ratio), then the
/// saved custom ratios, as View › Pixel Aspect Ratio orders them. The ids are View › Pixel Aspect
/// Ratio's; custom names are the user's, so they are not translated.
pub fn options(custom: &[CustomPar]) -> Vec<(String, String)> {
    let presets = [
        ("square", tl!("Square Pixels")),
        ("d1DvNtsc", tl!("D1/DV NTSC (0.91)")),
        ("d1DvPal", tl!("D1/DV PAL (1.09)")),
        ("d1DvNtscWidescreen", tl!("D1/DV NTSC Widescreen (1.21)")),
        ("hdv1080", tl!("HDV 1080/DVCPRO HD 720 (1.33)")),
        ("d1DvPalWidescreen", tl!("D1/DV PAL Widescreen (1.46)")),
        ("anamorphic2To1", tl!("Anamorphic 2:1 (2)")),
        ("dvcproHd1080", tl!("DVCPRO HD 1080 (1.5)")),
    ];
    let mut v: Vec<(String, String)> = presets.iter().map(|(id, label)| (id.to_string(), label.to_string())).collect();
    v.extend(custom.iter().map(|c| (format!("custom:{}:{}", c.ratio, c.name), c.name.clone())));
    v
}

/// The entry the dialog shows: the picked one while it is still listed, else Square Pixels.
pub fn picked(f: &Map<String, Value>, custom: &[CustomPar]) -> String {
    f.get(FIELD).and_then(Value::as_str).filter(|id| options(custom).iter().any(|(v, _)| v == id)).unwrap_or("square").to_string()
}

/// Create: the new document gets the picked ratio, Square Pixels included, whatever
/// View › Pixel Aspect Ratio held before. The ratio is app-wide today, so this sets the view's.
pub fn apply(app: &mut PhotocraftApp, f: &Map<String, Value>) {
    app.ui.view.pixel_aspect = picked(f, &app.ui.shell.custom_pars);
}
