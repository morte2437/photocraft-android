//! File-menu commands beyond New/Close: Close All / Others, Revert, Save a Copy, Open As, Place
//! Embedded / Linked, File Info, the Automate and Scripts items (Fit Image, Conditional Mode
//! Change, Batch, Image Processor, Load Files into Stack, Flatten All Layer Effects / Masks),
//! Export › Layers to Files and Color Lookup Tables, plus the document-level guide layouts of the
//! View menu (New Guide Layout, New Guides From Shape, Clear Canvas Guides).
//!
//! Commands that touch the file system take explicit paths (the UI asks for them with its own
//! pickers) and are unavailable on the web, where there is no file system; their byte-level
//! cores ([`place_bytes`], [`open_bytes_as`]) work everywhere.

use std::sync::Arc;

use photocraft_algo::resample::{Resample, resize_surface, translate_surface};
use photocraft_color::{ColorMode, PixelFormat};
use photocraft_doc::{Affine, Document, Layer, LayerContent, LayerId, SmartObject, SmartSource};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba_into};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, layer_param};
use crate::{EngineError, Result, Session};

mod batch_run;
mod export;
mod info;
mod open;
mod paths;
mod processing;
mod save;

pub use batch_run::OutputClaims;
pub(crate) use batch_run::{BatchInput, batch_inputs, batch_keeps_step, batch_save_point, process_files};
#[cfg(test)]
pub(crate) use batch_run::{MAX_SUBFOLDER_DEPTH, list_images_tree};
use batch_run::{batch, same_format};
pub use export::{LayerExportReport, SkippedExportLayer, bake_cube, export_layers_report_with, export_layers_with};
use export::{color_lookup_tables, layers_to_files, push_unique};
use info::file_info;
pub use info::{CAMERA_FIELDS, camera_data, read_file_info, write_file_info};
#[cfg(test)]
use info::{exposure_text, find_element};
pub use open::{PLACE_EMBEDDED, open_bytes_as, place_bytes, raw_defaults};
use open::{close_all, close_others, open_as, place, revert};
pub(crate) use open::{import, tag_exr_colorspace};
use paths::is_file;
#[cfg(test)]
use paths::openable_name;
pub use paths::{extension, holds, holds_at, is_template, layered_extension, list_images, losses_at, same_file, saves_in_place, template_name, untitled_name};
pub(crate) use paths::{file_name, forward_slashes, is_windows_device_name, join, portable_device_name, read_file, sanitize, stem, write_file};
use processing::{conditional_mode_change, fit_image, flatten_all_effects, flatten_all_masks, image_processor, load_files_into_stack};
use save::save_a_copy;
pub(crate) use save::{SaveOpts, encode, save_doc};
pub use save::{dds_param, dpx_bits_param, jpeg_format_param, jpeg_quality, tga_bits_param, tiff_compression_param, tiff_would_write_layers};

mod batch_naming;

// ---------- predicates ----------

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

/// The file system is reachable (not on the web).
pub(crate) fn native(_: &Session) -> std::result::Result<(), String> {
    if cfg!(target_arch = "wasm32") { Err("not available on the web (no file system)".into()) } else { Ok(()) }
}

pub(crate) fn native_doc(s: &Session) -> std::result::Result<(), String> {
    native(s)?;
    has_doc(s)
}

fn can_revert(s: &Session) -> std::result::Result<(), String> {
    native(s)?;
    let d = s.active().ok_or("no document open")?;
    let path = d.path.as_deref().ok_or("the document has never been saved")?;
    if !is_file(path) {
        return Err(format!("{path} is not a file on disk"));
    }
    Ok(())
}

fn has_adjustments(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if d.doc.layers.iter().any(|l| l.visible && matches!(l.content, LayerContent::Adjustment(_))) {
        Ok(())
    } else {
        Err("the document has no visible adjustment layers to export".into())
    }
}

fn has_shape(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    match d.active_layer.and_then(|id| d.doc.layer(id)).map(|l| &l.content) {
        Some(LayerContent::Shape(_)) => Ok(()),
        _ => Err("the active layer is not a shape layer".into()),
    }
}

impl crate::DocState {
    /// Complete a successful document save. File identity is not a history step; callers must
    /// finish the write first and must not call this for a copy or export.
    pub fn saved_to(&mut self, path: String) {
        let name = file_name(&path);
        if self.doc.name != name {
            Arc::make_mut(&mut self.doc).name = name;
        }
        self.path = Some(path);
        self.saved_revision = self.revision;
    }
}

pub(crate) fn str_param<'a>(p: &'a Value, key: &str, cmd: &str) -> Result<&'a str> {
    p.get(key).and_then(Value::as_str).filter(|v| !v.is_empty()).ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: format!("missing \"{key}\"") })
}

pub(crate) fn f64_param(p: &Value, key: &str) -> Option<f64> {
    p.get(key).and_then(Value::as_f64).filter(|v| v.is_finite())
}

/// A size parameter; zero or negative means "not set" (dialogs send 0 for empty fields).
fn size_param(p: &Value, key: &str) -> Option<f64> {
    f64_param(p, key).filter(|v| *v > 0.0)
}

// ---------- pixels ----------

/// A composite buffer as a surface in `fmt` (straight RGBA → the format's model and depth).
pub(crate) fn buffer_surface(buf: &photocraft_compose::Buffer, fmt: PixelFormat) -> Surface {
    let n = fmt.channels();
    let mut data = vec![0.0f32; buf.px.len() * n];
    for (p, out) in buf.px.iter().zip(data.chunks_exact_mut(n)) {
        from_rgba_into(&fmt, *p, out);
    }
    let mut s = Surface::new(fmt);
    if !buf.rect.is_empty() {
        s.write_region(buf.rect, &data);
    }
    s.prune();
    s
}

/// The flattened image of `doc` as a surface in `fmt`, placed at the origin.
pub(crate) fn flattened(doc: &Document, fmt: PixelFormat) -> Surface {
    photocraft_compose::flatten_to_surface(doc, fmt, None)
}

/// [`flattened`] for documents that came from a file (Save a Copy, Place, Load Files into Stack):
/// a sparse bundle declares its canvas, and one the size of the format allows would be rendered
/// in full, however little it holds. Refused past the compositor's own per-render limit.
pub(crate) fn flattened_checked(doc: &Document, fmt: PixelFormat, cmd: &str) -> Result<Surface> {
    let b = doc.bounds();
    if u64::from(b.width()) * u64::from(b.height()) > photocraft_compose::MAX_RENDER_PIXELS {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("the document's {}×{} canvas is too large to flatten", b.width(), b.height()) });
    }
    Ok(flattened(doc, fmt))
}

fn new_guide_layout(s: &mut Session, p: &Value) -> Result<Value> {
    crate::guide_layout::apply(s, p)
}

fn guides_from_shape(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let d = s.active().ok_or(EngineError::NoDocument)?;
    let l = d.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let LayerContent::Shape(sh) = &l.content else { return Err(EngineError::Other("the layer is not a shape layer".into())) };
    let (x0, y0, x1, y1) = sh.path.control_bounds().ok_or(EngineError::Other("the shape has no path".into()))?;
    s.edit("New Guides From Shape", |doc, _| {
        for x in [x0, (x0 + x1) / 2.0, x1] {
            push_unique(&mut doc.guides.vertical, x);
        }
        for y in [y0, (y0 + y1) / 2.0, y1] {
            push_unique(&mut doc.guides.horizontal, y);
        }
        Ok(())
    })?;
    Ok(json!({"bounds": [x0, y0, x1, y1]}))
}

fn clear_canvas_guides(s: &mut Session) -> Result<Value> {
    let d = s.active().ok_or(EngineError::NoDocument)?;
    // Guides inside an artboard belong to it (View › Clear Selected Artboard Guides); the rest
    // are canvas guides. Without artboards every guide is a canvas guide.
    let boards: Vec<Rect> = d.doc.artboards().iter().map(|b| b.2.rect).collect();
    let in_x = move |x: f64| boards.iter().any(|r| x > f64::from(r.x0) && x < f64::from(r.x1));
    let boards_y: Vec<Rect> = d.doc.artboards().iter().map(|b| b.2.rect).collect();
    let in_y = move |y: f64| boards_y.iter().any(|r| y > f64::from(r.y0) && y < f64::from(r.y1));
    let n = d.doc.guides.vertical.iter().filter(|x| !in_x(**x)).count() + d.doc.guides.horizontal.iter().filter(|y| !in_y(**y)).count();
    if n == 0 {
        return Ok(json!({"cleared": 0}));
    }
    s.edit("Clear Canvas Guides", |doc, _| {
        doc.guides.vertical.retain(|x| in_x(*x));
        doc.guides.horizontal.retain(|y| in_y(*y));
        doc.guides.prune_colors();
        Ok(())
    })?;
    Ok(json!({"cleared": n}))
}

// ---------- registry ----------

pub fn specs() -> Vec<CommandSpec> {
    macro_rules! spec {
        ($id:expr, $label:expr, $menu:expr, $sc:expr, $params:expr, $enabled:expr, $run:expr) => {
            CommandSpec { id: $id, label: $label, menu: $menu, shortcut: $sc, params: $params, enabled: $enabled, journal: true, run: $run }
        };
    }
    vec![
        spec!("file.closeAll", "Close All", &["File"], Some("Cmd+Alt+W"), "{}", has_doc, |s, _| close_all(s)),
        spec!(
            "file.closeOthers",
            "Close Others",
            &["File"],
            Some("Cmd+Alt+P"),
            r##"{"document":index? (the one to keep; default active)}"##,
            has_doc,
            close_others
        ),
        // The document tab's menu (UI-217-6): the tab's own file in the platform file manager.
        // Not a Photoshop menu-bar item; enabled when the target document has a saved path.
        spec!("file.revealInFinder", "Reveal in Finder", &[], None, r##"{"document":index? (default active),"dryRun":bool=false}"##, native_doc, |s, p| {
            let i = match p.get("document") {
                Some(v) => v
                    .as_u64()
                    .and_then(|v| usize::try_from(v).ok())
                    .ok_or_else(|| EngineError::BadParams { cmd: "file.revealInFinder".into(), msg: "`document` must be an index".into() })?,
                None => s.active_index().ok_or(EngineError::NoDocument)?,
            };
            let path = s
                .documents()
                .get(i)
                .and_then(|d| d.path.clone())
                .ok_or_else(|| EngineError::BadParams { cmd: "file.revealInFinder".into(), msg: format!("document {i} has no saved file") })?;
            crate::layer_menu_cmds::reveal(&path, p.get("dryRun").and_then(Value::as_bool).unwrap_or(false))
        }),
        spec!("file.revert", "Revert", &["File"], Some("F12"), "{} (reloads the saved file as one undoable step)", can_revert, |s, _| revert(s)),
        spec!(
            "file.saveACopy",
            "Save a Copy…",
            &["File"],
            Some("Cmd+Alt+S"),
            r##"{"path":str (format from the extension),"quality":0..12? (JPEG),"jpegFormat":"baseline|optimized|progressive"? (JPEG; default baseline),"layers":bool=true,"overwrite":bool=false (replace the document's own file),"tiffLayers":bool=false (TIFF: keep the layers; flat by default),"tiffCompression":"none|lzw|zip"? (TIFF: image compression; unset, none),"tgaBits":24|32? (TGA: 32 writes the alpha channel, or else the transparency, as the alpha; unset, 32 with an alpha channel or transparency, else 24),"dpxBits":8|10|12|16? (DPX: bits per sample; unset, 8 for an 8-bit document, else 16),"ddsCompression":"none|bc1|bc2|bc3"="none" (DDS: uncompressed, or block compression; BC1 keeps alpha on or off),"ddsMipmaps":bool=false (DDS: also write the full mip chain)}"##,
            native_doc,
            save_a_copy
        ),
        spec!(
            "file.openAs",
            "Open As…",
            &["File"],
            Some("Cmd+Alt+Shift+O"),
            r##"{"path":str,"as":"psd|png|jpg|tiff|…"? (decode as this format)}"##,
            native,
            open_as
        ),
        spec!(
            "file.placeEmbedded",
            "Place Embedded…",
            &["File"],
            None,
            r##"{"path":str,"scale":%? (default: fit when larger than the canvas),"fit":bool? (default: Preferences > General > Resize Image During Place),"smartObject":bool? (default: Preferences > General > Always Create Smart Objects When Placing),"center":[x,y]?}"##,
            native_doc,
            |s, p| place(s, p, false)
        ),
        spec!(
            "file.placeLinked",
            "Place Linked…",
            &["File"],
            None,
            r##"{"path":str,"scale":%?,"fit":bool? (default: Preferences > General > Resize Image During Place),"center":[x,y]?}"##,
            native_doc,
            |s, p| place(s, p, true)
        ),
        spec!(
            "file.fileInfo",
            "File Info…",
            &["File"],
            Some("Cmd+Alt+Shift+I"),
            r##"{"title":str?,"author":str?,"authorTitle":str?,"description":str?,"keywords":[str]|"a; b"?,"copyright":str?,"copyrightStatus":"unknown|copyrighted|publicDomain"?,"copyrightUrl":str?} (no keys: read; the result also has read-only "camera":{"make","model","dateTaken","exposure","fNumber","iso","focalLength","focalLength35mm","lens"}, present keys only)"##,
            has_doc,
            file_info
        ),
        spec!(
            "file.automate.fitImage",
            "Fit Image…",
            &["File", "Automate"],
            None,
            r##"{"width":px,"height":px,"dontEnlarge":bool=false,"resample":"automatic|preserveDetails|bicubicSmoother|bicubicSharper|bicubicAutomatic|bicubic|nearest|bilinear|lanczos"="bicubic"}"##,
            has_doc,
            fit_image
        ),
        spec!(
            "file.automate.conditionalModeChange",
            "Conditional Mode Change…",
            &["File", "Automate"],
            None,
            r##"{"from":["rgb","grayscale","cmyk","lab","indexed","bitmap",…]|"any"="any","to":"rgb|grayscale|cmyk|lab"}"##,
            has_doc,
            conditional_mode_change
        ),
        spec!(
            "file.automate.batch",
            "Batch…",
            &["File", "Automate"],
            None,
            r##"{"steps":[[commandId,params]|{"command":id,"params":{}}…] (a recorded action; a Save, Save As or Save a Copy step marks the document written to output, in this command's format and options: as the save found it if it was closed or another document is active at the end, else as the action leaves it; with no such step, the active document at the end),"input":folder|[paths],"includeSubfolders":bool=false (a folder input also takes the images in its subfolders, up to 32 levels and 100000 files; each result keeps its subfolder under output),"output":folder,"format":"same|png|jpg|psd|tiff|…"="same","quality":0..12?,"jpegFormat":"baseline|optimized|progressive"?,"naming":string? (blank keeps source names; {name}, {serial} padded to 3 digits, {date} as YYYY-MM-DD UTC; extension added automatically),"startingSerial":1..4294967295=1 (input position, including failed inputs),"tiffLayers":bool=false,"tiffCompression":"none|lzw|zip"?,"tgaBits":24|32?,"dpxBits":8|10|12|16? (DPX: bits per sample; unset, 8 for an 8-bit document, else 16),"ddsCompression":"none|bc1|bc2|bc3"="none" (DDS: uncompressed, or block compression; BC1 keeps alpha on or off),"ddsMipmaps":bool=false (DDS: also write the full mip chain),"calledActions":[{"name","steps"}…]? (the actions Play Action steps call; default: the session's)} → {files, errors} (an input whose output name was already written in the run goes to errors)"##,
            native,
            batch
        ),
        spec!(
            "file.scripts.imageProcessor",
            "Image Processor…",
            &["File", "Scripts"],
            None,
            r##"{"input":folder|[paths],"includeSubfolders":bool=false (a folder input also takes the images in its subfolders, up to 32 levels and 100000 files; each result keeps its subfolder under output),"output":folder,"format":"jpg|png|psd|tiff|…"="jpg","quality":0..12=8,"jpegFormat":"baseline|optimized|progressive"?,"tiffLayers":bool=false,"tiffCompression":"none|lzw|zip"?,"tgaBits":24|32?,"dpxBits":8|10|12|16? (DPX: bits per sample; unset, 8 for an 8-bit document, else 16),"ddsCompression":"none|bc1|bc2|bc3"="none" (DDS: uncompressed, or block compression; BC1 keeps alpha on or off),"ddsMipmaps":bool=false (DDS: also write the full mip chain),"width":px?,"height":px? (fit, never enlarge),"convertToSrgb":bool=false} → {files, errors} (an input whose output name was already written in the run goes to errors)"##,
            native,
            image_processor
        ),
        spec!(
            "file.scripts.loadFilesIntoStack",
            "Load Files into Stack…",
            &["File", "Scripts"],
            None,
            r##"{"paths":[str]|folder,"align":bool=false (Auto-Align Layers on them first),"createSmartObject":bool=false} → new document with one layer per file (inside one smart object with createSmartObject)"##,
            native,
            load_files_into_stack
        ),
        spec!("file.scripts.flattenAllLayerEffects", "Flatten All Layer Effects", &["File", "Scripts"], None, "{}", has_doc, |s, _| flatten_all_effects(s)),
        spec!(
            "file.scripts.flattenAllMasks",
            "Flatten All Masks",
            &["File", "Scripts"],
            None,
            "{} (pixel layers; masks on other layer kinds are left)",
            has_doc,
            |s, _| flatten_all_masks(s)
        ),
        spec!(
            "file.export.layersToFiles",
            "Layers to Files…",
            &["File", "Export"],
            None,
            r##"{"dir":folder,"format":"png|jpg|psd|tiff|…"="png","prefix":str=document name,"visibleOnly":bool=true,"trim":bool=false,"quality":0..12?,"jpegFormat":"baseline|optimized|progressive"?,"tiffLayers":bool=false,"tiffCompression":"none|lzw|zip"?,"tgaBits":24|32?,"dpxBits":8|10|12|16? (DPX: bits per sample; unset, 8 for an 8-bit document, else 16),"ddsCompression":"none|bc1|bc2|bc3"="none" (DDS: uncompressed, or block compression; BC1 keeps alpha on or off),"ddsMipmaps":bool=false (DDS: also write the full mip chain)} → {files}"##,
            native_doc,
            layers_to_files
        ),
        spec!(
            "file.export.colorLookupTables",
            "Color Lookup Tables…",
            &["File", "Export"],
            None,
            r##"{"path":str? (.cube; omit to return the text),"size":2..129=33,"title":str?,"scope":"all|selected"="all","layers":[id,…]?} (selected layer IDs use original document stacking; masks are spatial and omitted)"##,
            has_adjustments,
            color_lookup_tables
        ),
        spec!(
            "view.newGuideLayout",
            "New Guide Layout…",
            &["View"],
            None,
            r##"{"columns":n=0,"width":len?,"gutter":len=0,"rows":n=0,"height":len?,"rowGutter":len=gutter,"margin":len|[top,left,bottom,right]=0,"centerColumns":bool=false,"clearExisting":bool=false,"target":"document|selectedArtboards"="document"} (len: pixels, or text with px, in, cm, mm, pt, pc or % as typed in Photoshop, e.g. "1 cm"; % is of the target's width for columns, gutters and left/right margins, its height for rows and top/bottom margins)"##,
            has_doc,
            new_guide_layout
        ),
        spec!("view.newGuidesFromShape", "New Guides From Shape", &["View"], None, r##"{"layer":id?}"##, has_shape, guides_from_shape),
        spec!("view.clearCanvasGuides", "Clear Canvas Guides", &["View"], None, "{}", has_doc, |s, _| clear_canvas_guides(s)),
    ]
}

#[cfg(test)]
#[path = "file_cmds/tests.rs"]
mod tests;

#[cfg(test)]
mod aces_exr_tests {
    use super::*;

    /// File Handling › "Open color-less EXR as ACEScg": off, an EXR opens on io's linear-sRGB
    /// default; on, it opens as ACEScg — the working space ACES render pipelines write.
    #[test]
    fn exr_aces_cg_preference_retags_the_opened_document() {
        let image = photocraft_codecs::Image::from_f32(
            4,
            4,
            photocraft_codecs::ChannelLayout::Rgb,
            &[
                0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5,
                0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0, 0.25, 0.5, 0.75, 1.0,
            ],
        )
        .unwrap();
        let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::OpenExr, &photocraft_codecs::EncodeOptions::default()).unwrap();

        let linear = photocraft_cms::Builtin::LinearSrgb.profile().to_bytes();
        let aces = photocraft_cms::Builtin::AcesCg.profile().to_bytes();

        // Off (the default): io's linear-sRGB tagging stays.
        let mut s = Session::new();
        open_bytes_as(&mut s, "shot.exr", &bytes, None, None).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.icc_profile.as_deref().map(|v| v.as_slice()), Some(linear.as_slice()));

        // On: the same file opens as ACEScg.
        let mut s = Session::new();
        s.prefs.edit(|p| p.file_handling.exr_aces_cg = true);
        open_bytes_as(&mut s, "shot.exr", &bytes, None, None).unwrap();
        let d = &s.active().unwrap().doc;
        assert_eq!(d.icc_profile.as_deref().map(|v| v.as_slice()), Some(aces.as_slice()));
    }

    /// The desktop app opens files as background jobs (File > Open, drag-drop, Open Recent):
    /// the ACEScg preference applies there too, not only to `file.openAs`.
    #[test]
    fn exr_aces_cg_preference_applies_to_background_opens() {
        use crate::jobs::{OpenSource, Started};
        let image = photocraft_codecs::Image::from_f32(4, 4, photocraft_codecs::ChannelLayout::Rgb, &[0.5; 48]).unwrap();
        let bytes =
            std::sync::Arc::new(photocraft_codecs::encode(&image, photocraft_codecs::Format::OpenExr, &photocraft_codecs::EncodeOptions::default()).unwrap());
        let linear = photocraft_cms::Builtin::LinearSrgb.profile().to_bytes();
        let aces = photocraft_cms::Builtin::AcesCg.profile().to_bytes();
        for (on, want) in [(false, &linear), (true, &aces)] {
            let mut s = Session::new();
            s.prefs.edit(|p| p.file_handling.exr_aces_cg = on);
            let v = match s.start_open("shot.exr", OpenSource::Bytes(std::sync::Arc::clone(&bytes))).unwrap() {
                Started::Job(id) => s.wait_job(id).unwrap(),
                Started::Done(v) => v,
            };
            // ACEScg isn't the working space, so Ask When Opening holds it for the prompt, which
            // names the ACEScg profile; linear sRGB (io's HDR default) opens without asking.
            let profile = match v.get("held").and_then(Value::as_u64) {
                Some(held) => {
                    assert!(on, "{v}");
                    assert!(v["color"]["embedded"].as_str().unwrap().starts_with("ACEScg"), "{v}");
                    s.take_held_open(held).unwrap().0.icc_profile
                }
                None => {
                    assert!(v.get("document").is_some(), "{v}");
                    s.active().unwrap().doc.icc_profile.clone()
                }
            };
            assert_eq!(profile.as_deref().map(|v| v.as_slice()), Some(want.as_slice()), "exr_aces_cg = {on}");
        }
    }
}
