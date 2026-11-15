//! TIFF Options: what File › Save As and Save a Copy ask before writing a TIFF, as Photoshop's
//! TIFF Options dialog does (#8827): Image Compression (None, LZW, ZIP), Save Image Pyramid,
//! Save Transparency (offered when the document has no Background layer) and, for a document a
//! TIFF stores with its layers, "Discard Layers and Save a Copy". Photoshop's JPEG compression,
//! Pixel Order and Byte Order aren't offered: the TIFF encoder writes interleaved samples in the
//! machine's byte order without JPEG.
//!
//! With "Ask Before Saving Layered TIFF Files" on (Preferences › File Handling), a layered save
//! also notes that layers increase the file size; "Don't show again" clears the preference.
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered, journalled as the `record` command passed to [`park`] with its path,
//! `tiffLayers` (layered documents) and [`TiffChoice::record`]'s params. Cancel drops the save.
//! Scripts skip the prompt by passing any of `tiffCompression`, `tiffTransparency`,
//! `tiffPyramid` or `tiffLayers`. The answer is remembered for the next TIFF save of the session.

use photocraft_codecs::TiffCompression;
use serde_json::Value;

use crate::{ExportSettings, PhotocraftApp};

/// A TIFF Options answer, from the prompt or from a command's params.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TiffChoice {
    pub compression: TiffCompression,
    /// Save Transparency; `None` for the default (see [`ExportSettings::tiff_transparency`]).
    pub transparency: Option<bool>,
    /// Save Image Pyramid.
    pub pyramid: bool,
    /// `Some(false)`: "Discard Layers and Save a Copy". `None` keeps what the save would do.
    pub layers: Option<bool>,
}

impl TiffChoice {
    /// `settings` writing this choice.
    pub fn apply(self, settings: ExportSettings) -> ExportSettings {
        ExportSettings {
            tiff_compression: self.compression,
            tiff_transparency: self.transparency,
            tiff_pyramid: self.pyramid,
            tiff_layers: self.layers.unwrap_or(settings.tiff_layers),
            ..settings
        }
    }

    /// Adds `tiffCompression`, and `tiffTransparency` and `tiffPyramid` when set, to a recorded
    /// save step, so replaying it doesn't ask again. `tiffLayers` is recorded by the save itself.
    pub fn record(self, step: &mut Value) {
        step["tiffCompression"] = compression_name(self.compression).into();
        if let Some(t) = self.transparency {
            step["tiffTransparency"] = t.into();
        }
        if self.pyramid {
            step["tiffPyramid"] = true.into();
        }
    }
}

/// The `tiffCompression` name of `c` (PackBits, which the dialog doesn't offer, as none).
pub fn compression_name(c: TiffCompression) -> &'static str {
    match c {
        TiffCompression::Lzw => "lzw",
        TiffCompression::Deflate => "zip",
        TiffCompression::None | TiffCompression::PackBits => "none",
    }
}

/// The TIFF Options a command or control call gives (`tiffCompression`, `tiffTransparency`,
/// `tiffPyramid`, `tiffLayers`), which skip the prompt. None given: `None`. A bad value is an
/// error, as in the engine's saves.
pub fn choice_param(params: &Value, cmd: &str) -> Result<Option<TiffChoice>, String> {
    use photocraft_engine::file_cmds::{tiff_bool_param, tiff_compression_param};
    let compression = tiff_compression_param(params, cmd).map_err(|e| e.to_string())?;
    let transparency = tiff_bool_param(params, "tiffTransparency", cmd).map_err(|e| e.to_string())?;
    let pyramid = tiff_bool_param(params, "tiffPyramid", cmd).map_err(|e| e.to_string())?;
    let layers = match params.get("tiffLayers") {
        None | Some(Value::Null) => None,
        Some(Value::Bool(b)) => Some(*b),
        Some(v) => return Err(format!("{cmd}: `tiffLayers` must be true or false, got {v}")),
    };
    let any = compression.is_some() || transparency.is_some() || pyramid.is_some() || layers.is_some();
    Ok(any.then(|| TiffChoice { compression: compression.unwrap_or_default(), transparency, pyramid: pyramid.unwrap_or(false), layers }))
}

/// What the last TIFF Options answer of the session chose, where the next prompt starts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Remembered {
    pub compression: TiffCompression,
    pub pyramid: bool,
}

/// A save waiting on the TIFF Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the document goes.
    pub path: String,
    pub compression: TiffCompression,
    pub pyramid: bool,
    /// Save Transparency, offered when [`Self::has_transparency`].
    pub transparency: bool,
    /// The document has no Background layer, so its composite can be transparent.
    pub has_transparency: bool,
    /// The TIFF would carry the document's layers: "Discard Layers and Save a Copy" is offered.
    pub layered: bool,
    /// "Discard Layers and Save a Copy".
    pub discard_layers: bool,
    /// Show the layered-file note ("Ask Before Saving Layered TIFF Files").
    pub layer_note: bool,
    /// Clears `ask_before_saving_layered_tiff` when the save goes ahead.
    pub dont_ask_again: bool,
    /// Save a Copy: the document keeps its path and unsaved changes.
    pub copy: bool,
    /// The step the answered save is recorded as: `file.saveAs` (#2032) or `file.saveACopy` (#4373).
    pub record: &'static str,
}

/// `true` when `path` is a TIFF name (`.tif` / `.tiff`).
pub fn is_tiff_path(path: &str) -> bool {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    name.rsplit_once('.').is_some_and(|(stem, ext)| !stem.is_empty() && (ext.eq_ignore_ascii_case("tif") || ext.eq_ignore_ascii_case("tiff")))
}

/// Whether saving the active document to `path` asks for TIFF Options: any TIFF.
pub fn wants_prompt(app: &PhotocraftApp, path: &str) -> bool {
    is_tiff_path(path) && app.session.active().is_some()
}

/// Parks the save of the active document to `path` behind the prompt. A prompt already waiting
/// is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    if app.save_options_open() {
        return Err(crate::SAVE_OPTIONS_OPEN.into());
    }
    let doc = app.active_doc_id()?;
    let st = app.session.active().ok_or("no document")?;
    let layered = photocraft_engine::file_cmds::tiff_would_write_layers(&st.doc);
    let has_transparency = st.doc.background().is_none();
    // Starts where an unset Save Transparency writes: kept unless there are alpha or spot channels.
    let transparency = st.doc.channels.is_empty() && st.doc.quick_mask.is_none();
    let layer_note = layered && app.session.prefs().file_handling.ask_before_saving_layered_tiff;
    let Remembered { compression, pyramid } = app.tiff_remembered;
    app.tiff_options = Some(Prompt {
        doc,
        path,
        compression,
        pyramid,
        transparency,
        has_transparency,
        layered,
        discard_layers: false,
        layer_note,
        dont_ask_again: false,
        copy,
        record,
    });
    Ok(())
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.tiff_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let modal = egui::Modal::new(egui::Id::unique("tiff-options")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        ui.label(egui::RichText::new(tl!("TIFF Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        crate::widgets::section_label(ui, tl!("Image Compression"));
        ui.radio_value(&mut p.compression, TiffCompression::None, tl!("None"));
        ui.radio_value(&mut p.compression, TiffCompression::Lzw, tl!("LZW"));
        ui.radio_value(&mut p.compression, TiffCompression::Deflate, tl!("ZIP"));
        ui.add_space(8.0);
        crate::widgets::checkbox(ui, &mut p.pyramid, tl!("Save Image Pyramid"));
        if p.has_transparency {
            crate::widgets::checkbox(ui, &mut p.transparency, tl!("Save Transparency"));
        }
        if p.layered {
            ui.add_space(8.0);
            crate::widgets::section_label(ui, tl!("Layers"));
            crate::widgets::checkbox(ui, &mut p.discard_layers, tl!("Discard Layers and Save a Copy"));
            if p.layer_note && !p.discard_layers {
                ui.add_space(4.0);
                ui.label(tl!("Including layers will increase file size."));
                crate::widgets::checkbox(ui, &mut p.dont_ask_again, tl!("Don't show again"));
            }
        }
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    // A focused button takes Enter as its click without consuming the key: Enter on Cancel cancels.
    if ok || (!cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))) {
        confirm(app, ctx);
    } else if cancel {
        app.tiff_options = None;
        app.ui.status = tl!("Save canceled").into();
        app.ui.status_error = false;
    }
}

/// The answer the prompt gives as it stands.
pub fn choice(p: &Prompt) -> TiffChoice {
    TiffChoice {
        compression: p.compression,
        transparency: p.has_transparency.then_some(p.transparency),
        pyramid: p.pyramid,
        layers: p.layered.then_some(!p.discard_layers),
    }
}

/// OK: writes the waiting save with the prompt's answer, and journals it.
pub fn confirm(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.tiff_options.take() else { return };
    if p.layer_note && p.dont_ask_again && !p.discard_layers {
        app.session.edit_prefs(|prefs| prefs.file_handling.ask_before_saving_layered_tiff = false);
    }
    app.tiff_remembered = Remembered { compression: p.compression, pyramid: p.pyramid };
    let choice = choice(&p);
    // A plain Save writes whichever file the action is played on (#3531): no path.
    let mut step = if p.record == "file.save" { serde_json::json!({}) } else { serde_json::json!({"path": p.path}) };
    if let Some(layers) = choice.layers {
        step["tiffLayers"] = layers.into();
    }
    choice.record(&mut step);
    // The waiting document's own settings (its path resolves linked smart objects), even if
    // another tab is active now. Closed meanwhile, the write below reports it.
    let settings = app.with_document(p.doc, |app| Ok(app.save_settings())).unwrap_or_else(|_| app.save_settings());
    let settings = choice.apply(settings);
    app.write_answered_save(ctx, p.doc, p.path, &settings, p.copy || p.discard_layers, (p.record, step));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn tiff_names() {
        assert!(is_tiff_path("a.tif"));
        assert!(is_tiff_path("dir/sub/a.TIFF"));
        assert!(is_tiff_path("C:\\x\\y.tiff"));
        assert!(!is_tiff_path("a.tif.psd"));
        assert!(!is_tiff_path(".tif"));
        assert!(!is_tiff_path("tif"));
        assert!(!is_tiff_path("a.png"));
    }

    #[test]
    fn choice_params() {
        assert_eq!(choice_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(choice_param(&json!({"tiffCompression": null, "tiffPyramid": null}), "file.saveAs"), Ok(None));
        assert_eq!(
            choice_param(&json!({"tiffCompression": "lzw"}), "file.saveAs"),
            Ok(Some(TiffChoice { compression: TiffCompression::Lzw, ..Default::default() }))
        );
        assert_eq!(
            choice_param(&json!({"tiffTransparency": false, "tiffPyramid": true, "tiffLayers": false}), "file.saveAs"),
            Ok(Some(TiffChoice { compression: TiffCompression::None, transparency: Some(false), pyramid: true, layers: Some(false) }))
        );
        for bad in [
            json!({"tiffCompression": "jpeg"}),
            json!({"tiffCompression": 1}),
            json!({"tiffTransparency": "yes"}),
            json!({"tiffTransparency": 1}),
            json!({"tiffPyramid": [true]}),
            json!({"tiffLayers": "false"}),
        ] {
            assert!(choice_param(&bad, "file.saveAs").is_err(), "{bad}");
        }
    }

    #[test]
    fn recorded_steps_replay_the_same_answer() {
        for choice in [
            TiffChoice::default(),
            TiffChoice { compression: TiffCompression::Deflate, transparency: Some(true), pyramid: true, layers: None },
            TiffChoice { compression: TiffCompression::Lzw, transparency: Some(false), pyramid: false, layers: None },
        ] {
            let mut step = json!({"path": "a.tif"});
            choice.record(&mut step);
            assert_eq!(choice_param(&step, "file.saveAs"), Ok(Some(choice)), "{step}");
        }
    }

    #[test]
    fn apply_sets_the_export_settings() {
        let base = ExportSettings::default();
        let s = TiffChoice { compression: TiffCompression::Deflate, transparency: Some(false), pyramid: true, layers: Some(false) }.apply(base.clone());
        assert_eq!((s.tiff_compression, s.tiff_transparency, s.tiff_pyramid, s.tiff_layers), (TiffCompression::Deflate, Some(false), true, false));
        let io = s.io_options();
        assert_eq!(
            (io.encode.tiff_compression, io.tiff_transparency, io.encode.tiff_pyramid, io.tiff_layers),
            (TiffCompression::Deflate, Some(false), true, false)
        );
        // No layers answer keeps what the save would do.
        assert!(TiffChoice::default().apply(base).tiff_layers);
    }
}
