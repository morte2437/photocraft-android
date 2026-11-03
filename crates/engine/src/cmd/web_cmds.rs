//! File › Export › Save for Web (Legacy), File › Export › Export Preferences, the Quick Export
//! pipeline that honours them, and File › Generate › Image Assets (Photoshop's Generator).
//!
//! Save for Web optimises the flattened (optionally resized, sRGB-converted) image as GIF,
//! PNG-8, PNG-24, JPEG or WBMP, per slice, into a folder with an optional HTML table; without an
//! output it only reports the optimised size (the dialog's 2-Up/4-Up annotations). Colour
//! reduction and dithering reuse `algo::quantize`; GIF/WBMP/progressive JPEG writers live in
//! `codecs::web`.
//!
//! Image Assets implements the naming grammar of Adobe's public Generator documentation: a
//! layer or group named `foo.png`, `200% foo@2x.png, 48x48 icons/foo.png8`, `photo.jpg80%`,
//! `?x100 thumb.gif` is exported into `<document>-assets/` whenever the document is saved while
//! the feature is on; a `default` layer (`default 50% low/ + 200% @2x`) adds variants.

use photocraft_algo::quantize::{self, Dither, Forced, PaletteKind};
use photocraft_doc::slices::{self, ResolvedSlice, SliceKind};
use photocraft_doc::{DocId, Document, LayerContent, LayerId};
use photocraft_geom::Rect;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::file_cmds::{is_windows_device_name, join, native_doc, portable_device_name, stem, write_file};
use crate::{EngineError, Result, Session};

mod assets;
mod optimizer;
mod quick;
mod save_web;
mod settings;

use assets::image_assets;
pub use assets::{AssetDefault, AssetSpec, assets_dir, generate_assets, parse_asset_name, parse_defaults};
pub use optimizer::{Optimized, optimize, web_document, web_exif, web_xmp};
use quick::{export_preferences, quick_export};
pub use quick::{quick_export_target, quick_export_target_is_own_file};
use save_web::save_for_web;
pub use save_web::save_for_web_with_writer;
pub use settings::{PRESETS, WebFormat, WebMetadata, WebSettings, preset_settings, rgb_hex};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}
fn other(e: impl std::fmt::Display) -> EngineError {
    EngineError::Other(e.to_string())
}

/// After a save: regenerate the document's image assets when Generate › Image Assets is on.
pub fn on_saved(s: &Session, index: usize) -> Option<Value> {
    let d = s.documents().get(index)?;
    if !s.file_menu.image_assets.contains(&d.doc.id) || !s.prefs().plug_ins.generator_enabled || cfg!(target_arch = "wasm32") {
        return None;
    }
    let dir = assets_dir(d.path.as_deref()?);
    let (files, errors) = generate_assets(&d.doc, &dir);
    Some(json!({"dir": dir, "files": files, "errors": errors}))
}

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    fn has_doc(s: &Session) -> std::result::Result<(), String> {
        s.active().map(|_| ()).ok_or_else(|| "no document open".into())
    }

    /// Generate ▸ Image Assets needs Preferences ▸ Plug-ins ▸ Enable Generator, as in
    /// Photoshop (where "Enable Generator" is unchecked until turned on).
    fn generator_enabled(s: &Session) -> std::result::Result<(), String> {
        has_doc(s)?;
        if !s.prefs().plug_ins.generator_enabled {
            return Err("Enable Generator in Preferences ▸ Plug-ins".into());
        }
        Ok(())
    }
    vec![
        spec!(
            "file.export.saveForWebLegacy",
            "Save for Web (Legacy)…",
            &["File", "Export"],
            Some("Cmd+Alt+Shift+S"),
            r##"{"preset":"GIF 128 Dithered|JPEG High|PNG-24|…"?,"format":"gif|png8|png24|jpeg|wbmp|webp"="png24","palette":"perceptual|selective|adaptive|restrictive|exact|systemMac|systemWindows|uniform"="selective","colors":2..256=256,"colorTable":["#rrggbb",…]? (fixed opaque source palette, 1–256 entries; 255 with transparency),"colorTableEdits":{"0":"#rrggbb",…}? (replace entries after indexing; requires colorTable),"dither":"none|diffusion|pattern|noise"="diffusion","ditherAmount":0..100=88,"transparency":bool=true,"matte":"#rrggbb|none"="none" (a preset's own: white; formats without transparency go over white),"interlaced":bool=false,"webSnap":0..100=0,"quality":0..100=60 (JPEG, lossy WebP),"lossless":bool=false (WebP),"progressive":bool=false,"optimized":bool=true,"embedIcc":bool=false,"metadata":"none|copyright|copyrightAndContact|allExceptCameraInfo|all"="copyright" (EXIF only with all),"convertToSrgb":bool=true,"width"|"height"|"percent"? (image size),"resample":"bicubic|bilinear|nearest"?,"path":file? (whole image),"dir":folder? (one file per slice in images/),"imagesFolder":relative-folder?="images","html":bool=false,"trustedSliceHtml":bool=false (only for trusted documents; emit raw no-image cell HTML),"slices":"all|user"="all","numbers":[n]?} → no path/dir: {bytes,width,height,colors,colorTable,transparentIndex} estimate; else {files, html, bytes, warnings}"##,
            has_doc,
            save_for_web
        ),
        spec!(
            "file.export.exportPreferences",
            "Export Preferences…",
            &["File", "Export"],
            None,
            r##"{"quickExportFormat":"png|jpg|gif|webp|exr"?,"quickExportLocation":"ask|sameFolder"?,"jpegQuality":1..100?,"webpLossless":bool?,"webpQuality":1..100?,"metadata":"none|copyright|all"?,"convertToSrgb":bool?} → {values}"##,
            |_| Ok(()),
            export_preferences
        ),
        spec!(
            "file.export.quickExport",
            "Quick Export",
            &[],
            None,
            r##"{"path":str? (required unless Export Preferences › Location is "sameFolder" and the document is saved)} → {path, format, bytes, warnings}"##,
            native_doc,
            quick_export
        ),
        spec!(
            "file.generate.imageAssets",
            "Image Assets",
            &["File", "Generate"],
            None,
            r##"{"on":bool? (default: toggle),"dir":folder? (default <document>-assets next to the file)} → {enabled, files, errors}; layers named like "foo.png", "200% foo@2x.png", "48x48 icons/a.png8", "photo.jpg80%" are exported, now and after each save"##,
            generator_enabled,
            image_assets
        ),
    ]
}

#[cfg(test)]
#[path = "web_cmds/tests.rs"]
mod tests;
