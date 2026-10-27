//! DDS Options: the compression and mip chain File › Save As and Save a Copy ask for before
//! writing a DirectDraw Surface texture (#1327). Photoshop has no DDS writer of its own (a plug-in
//! adds one), so this follows the texture tools' usual choices: uncompressed, or BC1/BC2/BC3
//! (DXT1/DXT3/DXT5), and whether to generate mipmaps (see `photocraft_io::DdsOptions`).
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered. Cancel drops the save. Scripts skip the prompt by passing `ddsCompression` or
//! `ddsMipmaps`.

use photocraft_io::{DdsCompression, DdsOptions};

use crate::{ExportSettings, PhotocraftApp};

/// A save waiting on the DDS Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the texture goes.
    pub path: String,
    /// The options chosen, starting uncompressed without mipmaps.
    pub options: DdsOptions,
    /// Save a Copy: the document keeps its path and unsaved changes.
    pub copy: bool,
    /// The step the answered save is recorded as (`file.saveAs` or `file.saveACopy`), so an
    /// action being recorded keeps it; `file.save` when File › Save confirmed the document's own
    /// texture (#3531), which plays back on the file the action is played on.
    pub record: &'static str,
}

/// `true` when `path` names a DDS texture.
pub fn wants_prompt(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).and_then(|e| photocraft_codecs::from_extension(&e)) == Some(photocraft_codecs::Format::Dds)
}

/// The DDS options a command or control call asks for (`"ddsCompression"`, `"ddsMipmaps"`),
/// which skip the prompt.
pub fn options_param(params: &serde_json::Value, cmd: &str) -> Result<Option<DdsOptions>, String> {
    photocraft_engine::file_cmds::dds_param(params, cmd).map_err(|e| e.to_string())
}

/// The params a save with `options` is recorded with.
pub fn record(options: DdsOptions, step: &mut serde_json::Value) {
    step["ddsCompression"] = options.compression.name().into();
    step["ddsMipmaps"] = options.mipmaps.into();
}

/// Parks the save behind the prompt. A prompt already waiting is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    if app.save_options_open() {
        return Err(crate::SAVE_OPTIONS_OPEN.into());
    }
    let doc = app.active_doc_id()?;
    app.dds_options = Some(Prompt { doc, path, options: DdsOptions::default(), copy, record });
    Ok(())
}

/// The block compressions go by their format names, which stay untranslated like other
/// technology names.
fn label(c: DdsCompression) -> &'static str {
    match c {
        DdsCompression::None => tl!("None"),
        DdsCompression::Bc1 => "BC1 / DXT1",
        DdsCompression::Bc2 => "BC2 / DXT3",
        DdsCompression::Bc3 => "BC3 / DXT5",
    }
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.dds_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let mut options = p.options;
    let modal = egui::Modal::new(egui::Id::unique("dds-options")).show(ctx, |ui| {
        ui.set_max_width(340.0);
        ui.label(egui::RichText::new(tl!("DDS Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        crate::widgets::section_label(ui, tl!("Compression"));
        for c in DdsCompression::ALL {
            ui.radio_value(&mut options.compression, c, label(c));
        }
        ui.add_space(8.0);
        crate::widgets::checkbox(ui, &mut options.mipmaps, tl!("Generate Mipmaps"));
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.options = options;
    // A focused button takes Enter as its click without consuming the key: Enter on Cancel cancels.
    if ok || (!cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))) {
        let Some(p) = app.dds_options.take() else { return };
        let settings = ExportSettings { dds: p.options, ..app.save_settings() };
        let mut step = if p.record == "file.save" { serde_json::json!({}) } else { serde_json::json!({"path": p.path}) };
        record(p.options, &mut step);
        let written = app.with_document(p.doc, |app| app.write_document(p.path, &settings, p.copy));
        if written.is_ok() {
            // An action being recorded keeps the Save As or Save a Copy with this answer.
            app.session.journal.push((p.record.into(), step));
        }
        match written {
            Err(e) => {
                app.ui.status = crate::i18n::fmt(tl!("Save failed: {error}"), &[("error", &e)]);
                app.ui.status_error = true;
            }
            Ok(Some(_)) if !p.copy => crate::discard_ui::saved_document(app, ctx, p.doc),
            // A background save releases the close prompt when it finishes (`jobs_ui`).
            Ok(_) => {}
        }
    } else if cancel {
        app.dds_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dds_names() {
        assert!(wants_prompt("a.dds"));
        assert!(wants_prompt("dir/sub/a.DDS"));
        assert!(wants_prompt("C:\\x\\y.dds"));
        assert!(!wants_prompt("a.dds.psd"));
        assert!(!wants_prompt(".dds"));
        assert!(!wants_prompt("a.png"));
    }

    #[test]
    fn options_params() {
        use serde_json::json;
        assert_eq!(options_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(options_param(&json!({"ddsCompression": "bc3"}), "file.saveAs"), Ok(Some(DdsOptions { compression: DdsCompression::Bc3, mipmaps: false })));
        assert_eq!(options_param(&json!({"ddsMipmaps": true}), "file.saveAs"), Ok(Some(DdsOptions { compression: DdsCompression::None, mipmaps: true })));
        for bad in [json!({"ddsCompression": "bc7"}), json!({"ddsCompression": 3}), json!({"ddsMipmaps": "yes"})] {
            assert!(options_param(&bad, "file.saveAs").is_err(), "{bad}");
        }
        let mut step = json!({"path": "a.dds"});
        record(DdsOptions { compression: DdsCompression::Bc1, mipmaps: true }, &mut step);
        assert_eq!(step, json!({"path": "a.dds", "ddsCompression": "bc1", "ddsMipmaps": true}));
        assert_eq!(options_param(&step, "file.saveAs"), Ok(Some(DdsOptions { compression: DdsCompression::Bc1, mipmaps: true })));
    }
}
