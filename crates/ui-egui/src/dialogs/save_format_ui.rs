//! Save As › Format, for platforms whose save picker has no list of file types (Android's
//! document picker: a name and one MIME type, #26 on the Android port). The format follows the
//! file's extension, so without this step every Save As there wrote the suggested PSD.
//!
//! The shell turns it on with [`crate::Services::save_format_prompt`]. File › Save As (and a
//! Save that has to ask for a name) parks here first; the picker then opens with the name in the
//! chosen format. Like Photoshop's Save As, the list starts on the format the document would be
//! saved in. Cancel drops the save.

use crate::{FormatChoices, PhotocraftApp};
use photocraft_doc::DocId;
use std::path::Path;

/// Save As formats: label and extension, in the desktop dialog's order. Each is offered only
/// where it can be written ([`writable`]).
pub const FORMATS: &[(&str, &str)] = &[
    ("Photoshop (PSD)", "psd"),
    ("Large Document Format (PSB)", "psb"),
    ("PhotoCraft (.pcraft)", "pcraft"),
    ("OpenRaster (ORA)", "ora"),
    ("TIFF", "tif"),
    ("PDF", "pdf"),
    ("PNG", "png"),
    ("JPEG", "jpg"),
    ("JPEG XL", "jxl"),
    ("WebP", "webp"),
    ("Targa", "tga"),
    ("BMP", "bmp"),
    ("GIF", "gif"),
    ("ICO", "ico"),
    ("OpenEXR", "exr"),
    ("Netpbm", "pnm"),
    ("Radiance HDR", "hdr"),
    ("FITS", "fits"),
    ("DPX", "dpx"),
    ("DDS", "dds"),
    ("Cineon", "cin"),
    ("Amiga IFF", "iff"),
];

/// Whether documents can be saved as `ext`: the layered formats, or a flat one with an encoder.
pub fn writable(ext: &str) -> bool {
    matches!(ext, "psd" | "psb" | "pcraft" | "ora" | "tif" | "tiff" | "pdf") || photocraft_codecs::from_extension(ext).is_some_and(|f| f.caps().write)
}

/// The formats offered, in order.
pub fn formats() -> impl Iterator<Item = (&'static str, &'static str)> {
    FORMATS.iter().copied().filter(|(_, ext)| writable(ext))
}

/// The entry of [`FORMATS`] for `ext` (`.jpeg` is JPEG's `.jpg`, `.tiff` TIFF's `.tif`).
fn entry(ext: &str) -> Option<usize> {
    let ext = ext.to_ascii_lowercase();
    let ext = match ext.as_str() {
        "jpeg" => "jpg",
        "tiff" => "tif",
        e => e,
    };
    FORMATS.iter().position(|(_, e)| *e == ext)
}

/// `suggested` with `ext` as its extension.
pub fn with_format(suggested: &str, ext: &str) -> String {
    Path::new(suggested).with_extension(ext).to_string_lossy().into_owned()
}

/// A Save As waiting for its format.
pub struct Prompt {
    /// The document that asked, even if tabs change while this is open.
    doc: DocId,
    /// The name (or path) the picker would have started from; its extension is replaced.
    suggested: String,
    /// The chosen entry of [`FORMATS`].
    pub selected: usize,
    choices: FormatChoices,
    plain_save: bool,
}

/// Whether a Save As starting from `suggested` asks for its format first: the shell asked for
/// it and `suggested` names a document format (not a swatch or preset file).
pub fn wants_prompt(app: &PhotocraftApp, suggested: &str) -> bool {
    app.services.save_format_prompt && Path::new(suggested).extension().and_then(|e| e.to_str()).and_then(entry).is_some()
}

/// Parks the Save As behind the prompt, starting on `suggested`'s format.
pub fn park(app: &mut PhotocraftApp, doc: DocId, suggested: String, choices: FormatChoices, plain_save: bool) {
    let selected = Path::new(&suggested).extension().and_then(|e| e.to_str()).and_then(entry).unwrap_or(0);
    app.save_format = Some(Prompt { doc, suggested, selected, choices, plain_save });
}

/// Continue: the save picker opens with the name in the chosen format.
pub(crate) fn accept(app: &mut PhotocraftApp) -> Result<serde_json::Value, String> {
    let Some(p) = app.save_format.take() else { return Err("no Save As is waiting for a format".into()) };
    let ext = FORMATS.get(p.selected).map_or("psd", |(_, ext)| ext);
    let suggested = with_format(&p.suggested, ext);
    app.pick_document_save(&suggested, p.doc, p.choices, p.plain_save)
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.save_format else { return };
    let (mut ok, mut cancel) = (false, false);
    let mut selected = p.selected;
    let modal = egui::Modal::new(egui::Id::unique("save-format")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(egui::RichText::new(tl!("Save As")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            ui.label(tl!("Format:"));
            let current = FORMATS.get(selected).map_or("", |(label, _)| label);
            craft_ui::ComboBox::from_id_salt("save-format-list").selected_text(current).width(240.0).show_ui(ui, |ui| {
                for (i, (label, ext)) in FORMATS.iter().enumerate() {
                    if writable(ext) && ui.selectable_label(i == selected, *label).clicked() {
                        selected = i;
                    }
                }
            });
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new(tl!("Choose the name and folder next.")).small());
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("Continue"), 96.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.selected = selected;
    if ok || (!cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))) {
        if let Err(e) = accept(app) {
            app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
            app.ui.status_error = true;
        }
    } else if cancel {
        app.save_format = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_layered_formats_and_the_common_flat_ones_are_offered() {
        let offered: Vec<_> = formats().map(|(_, ext)| ext).collect();
        for ext in ["psd", "psb", "pcraft", "ora", "tif", "pdf", "png", "jpg", "webp"] {
            assert!(offered.contains(&ext), ".{ext} is offered: {offered:?}");
        }
    }

    #[test]
    fn a_format_replaces_the_extension() {
        assert_eq!(with_format("Untitled-1.psd", "pcraft"), "Untitled-1.pcraft");
        assert_eq!(with_format("/data/x/Cat.psd", "png"), "/data/x/Cat.png");
        assert_eq!(with_format("Scan.2024.psd", "jpg"), "Scan.2024.jpg");
    }

    #[test]
    fn the_list_starts_on_the_suggested_format() {
        assert_eq!(entry("PSD"), Some(0));
        assert_eq!(entry("jpeg"), entry("jpg"));
        assert_eq!(entry("tiff"), entry("tif"));
        assert_eq!(entry("pcraft").map(|i| FORMATS[i].1), Some("pcraft"));
        assert_eq!(entry("aco"), None, "a swatch file isn't a document format");
    }

    /// A new document, with a save dialog that records what it was asked and cancels.
    fn app(prompt: bool) -> (PhotocraftApp, std::rc::Rc<std::cell::RefCell<Vec<crate::FileDialogRequest>>>) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", serde_json::json!({"width": 8, "height": 8})).unwrap();
        app.sync_views();
        let (show, asked) = crate::file_dialog::fake(vec![None]);
        app.services.file_dialog = Some(show);
        app.services.save_format_prompt = prompt;
        (app, asked)
    }

    fn suggested(asked: &[crate::FileDialogRequest]) -> Vec<String> {
        asked
            .iter()
            .filter_map(|r| match r {
                crate::FileDialogRequest::Save { suggested } => Some(suggested.clone()),
                _ => None,
            })
            .collect()
    }

    /// #26 (Android port): Save As asks for the format, then the picker gets the name in it.
    #[test]
    fn save_as_asks_for_the_format_before_the_picker() {
        let (mut app, asked) = app(true);
        let ctx = egui::Context::default();
        app.save_as(None).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(asked.borrow().is_empty(), "no picker before the format is chosen");
        let p = app.save_format.as_ref().expect("the format is asked for");
        assert_eq!(FORMATS[p.selected].1, "psd", "starting on the format the document would be saved in");
        assert!(app.save_as(None).is_err(), "one Save As at a time");
        app.save_format.as_mut().unwrap().selected = entry("pcraft").unwrap();
        accept(&mut app).unwrap();
        app.poll_file_dialog(&ctx, None);
        let names = suggested(&asked.borrow());
        assert_eq!(names.len(), 1, "{names:?}");
        assert!(names[0].ends_with(".pcraft"), "{names:?}");
        assert!(app.save_format.is_none());
    }

    #[test]
    fn without_the_prompt_save_as_goes_straight_to_the_picker() {
        let (mut app, asked) = app(false);
        app.save_as(None).unwrap();
        app.poll_file_dialog(&egui::Context::default(), None);
        assert!(app.save_format.is_none());
        assert!(suggested(&asked.borrow())[0].ends_with(".psd"));
    }
}
