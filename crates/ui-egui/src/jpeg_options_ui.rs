//! JPEG Options: the quality and format File › Save, Save As and Save a Copy ask for before writing
//! a JPEG, as Photoshop's JPEG Options does. Quality is Photoshop's 0–12 scale (Low 0–4, Medium
//! 5–7, High 8–9, Maximum 10–12), mapped to the encoder's 1–100 by
//! `photocraft_engine::file_cmds::jpeg_quality`; Format Options are Baseline ("Standard"), Baseline
//! Optimized and Progressive. Matte, Scans and the size preview aren't offered yet. Re-saving an
//! opened JPEG starts at the file's own quality, as File › Save keeps it (#4032).
//!
//! The save is parked in [`Prompt`] while the modal is up (egui is immediate-mode) and written
//! once answered. Cancel drops the save. Scripts skip the prompt by passing `quality` or
//! `jpegFormat`.
//!
//! Any save can ask: check [`wants_prompt`], then [`park`] (which starts where
//! [`Start::for_active`] says) or [`park_at`] with a [`Start`] of its own, and return. The answer
//! is written by [`confirm`] and journalled as the `record` command passed to `park`: with its
//! path, `quality` (0–12, left out when the file's own quality was kept) and `jpegFormat`, or
//! without the path for a plain `file.save`, which writes whichever file an action is played on
//! ([`plain_save_step`]).

use photocraft_codecs::JpegFormat;
use serde_json::Value;

use crate::{ExportSettings, PhotocraftApp};

/// The highest quality on Photoshop's JPEG scale.
pub const MAX_QUALITY: u8 = 12;

/// A JPEG Options answer, from the prompt or from a command's `quality` and `jpegFormat`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct JpegChoice {
    /// Photoshop's 0–12 scale; `None` keeps the quality the save would use anyway (the opened
    /// file's own, else the encoder's default).
    pub quality: Option<u8>,
    pub format: JpegFormat,
}

impl JpegChoice {
    /// `settings` writing this choice.
    pub fn apply(self, settings: ExportSettings) -> ExportSettings {
        ExportSettings {
            jpeg_quality: self.quality.map(|q| photocraft_engine::file_cmds::jpeg_quality(f64::from(q))).or(settings.jpeg_quality),
            jpeg_format: self.format,
            ..settings
        }
    }

    /// Adds `quality` and `jpegFormat` to a recorded save step, so replaying it doesn't ask again.
    pub fn record(self, step: &mut Value) {
        if let Some(q) = self.quality {
            step["quality"] = q.into();
        }
        step["jpegFormat"] = self.format.name().into();
    }
}

/// The JPEG Options a command or control call gives (`"quality": 0..12`, `"jpegFormat"`), which
/// skip the prompt. Neither given: `None`. A quality that isn't a number from 0 to 12, or a
/// format other than the three names, is an error.
pub fn choice_param(params: &Value, cmd: &str) -> Result<Option<JpegChoice>, String> {
    let format = photocraft_engine::file_cmds::jpeg_format_param(params, cmd).map_err(|e| e.to_string())?;
    let quality = match params.get("quality") {
        None | Some(Value::Null) => None,
        Some(v) => Some(
            v.as_f64()
                .filter(|q| (0.0..=f64::from(MAX_QUALITY)).contains(q))
                .map(|q| q.round() as u8)
                .ok_or_else(|| format!("{cmd}: `quality` must be a number from 0 to 12 (Photoshop's JPEG scale), got {v}"))?,
        ),
    };
    Ok((quality.is_some() || format.is_some()).then(|| JpegChoice { quality, format: format.unwrap_or_default() }))
}

/// The params of a recorded plain `file.save` (which writes whichever file an action is played
/// on): none, unless JPEG Options changed something, so that a replay writes it too without
/// asking. `quality` is left out when the file's own quality was kept, and `jpegFormat` when it
/// is Baseline, the default.
pub fn plain_save_step(choice: Option<JpegChoice>) -> Value {
    let mut step = Value::Object(Default::default());
    if let Some(c) = choice {
        if let Some(q) = c.quality {
            step["quality"] = q.into();
        }
        if c.format != JpegFormat::Baseline {
            step["jpegFormat"] = c.format.name().into();
        }
    }
    step
}

/// The JPEG Options in a recorded save step, which replays without asking. A `file.saveACopy`
/// step from the engine command keeps its params as they were given, and the engine clamps a
/// `quality` to 0–12 and ignores one that isn't a number, so the replay reads it the same way. A
/// format other than the three names is still an error, as it is for the engine.
pub fn recorded_choice(params: &Value, cmd: &str) -> Result<Option<JpegChoice>, String> {
    let format = photocraft_engine::file_cmds::jpeg_format_param(params, cmd).map_err(|e| e.to_string())?;
    let quality = params.get("quality").and_then(Value::as_f64).filter(|q| q.is_finite()).map(|q| q.clamp(0.0, f64::from(MAX_QUALITY)).round() as u8);
    Ok((quality.is_some() || format.is_some()).then(|| JpegChoice { quality, format: format.unwrap_or_default() }))
}

/// Photoshop's name for a quality, in the UI language: Low, Medium, High or Maximum.
pub fn quality_label(q: u8) -> &'static str {
    match q {
        0..=4 => tl!("Low"),
        5..=7 => tl!("Medium"),
        8..=9 => tl!("High"),
        _ => tl!("Maximum"),
    }
}

/// The 0–12 quality nearest an encoder quality (1–100), the inverse of the engine's mapping.
pub fn quality_from_percent(q: u32) -> u8 {
    let q = f64::from(q.clamp(1, 100));
    ((q - 1.0) / 99.0 * f64::from(MAX_QUALITY)).round() as u8
}

/// Where the prompt starts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Start {
    /// Quality on the 0–12 scale.
    pub quality: u8,
    /// The opened file's own encoder quality (1–100) that `quality` was rounded from. Left on
    /// that step, OK writes it exactly and records no `quality`, so a replay keeps it too.
    pub own_quality: Option<u8>,
    pub format: JpegFormat,
}

impl Start {
    /// Starts at an opened JPEG's own encoder quality (1–100; out of range is clamped).
    pub fn own(quality: u8) -> Self {
        let q = quality.clamp(1, 100);
        Start { quality: quality_from_percent(u32::from(q)), own_quality: Some(q), format: JpegFormat::Baseline }
    }

    /// Starts at an encoder quality (1–100) that isn't the file's own, such as a preference.
    pub fn percent(quality: u32) -> Self {
        Start { quality: quality_from_percent(quality), own_quality: None, format: JpegFormat::Baseline }
    }

    /// For the active document: its opened JPEG's own quality (`Document::file_encoding`), or
    /// else the Export preferences' JPEG quality (the one Quick Export uses).
    pub fn for_active(app: &PhotocraftApp) -> Self {
        match app.session.active().and_then(|st| st.doc.file_encoding.jpeg_quality) {
            Some(q) => Start::own(q),
            None => Start::percent(app.session.prefs().export.jpeg_quality),
        }
    }
}

/// A save waiting on the JPEG Options answer.
pub struct Prompt {
    /// The document that requested the save, even if tabs change while this is open.
    doc: photocraft_doc::DocId,
    /// Where the document goes.
    pub path: String,
    /// Quality, 0–12.
    pub quality: u8,
    pub format: JpegFormat,
    /// See [`Start::own_quality`].
    pub own_quality: Option<u8>,
    /// Save a Copy: the document keeps its path and unsaved changes.
    pub copy: bool,
    /// The step the answered save is recorded as, so an action being recorded keeps it:
    /// `file.saveAs` for Save As (#2032), `file.saveACopy` for Save a Copy (#4373), and
    /// `file.save` (with no params) for File › Save writing the document's own file (#3531).
    pub record: &'static str,
}

/// `true` when `path` names a JPEG (`.jpg`, `.jpeg`, `.jpe`, …).
pub fn is_jpeg_path(path: &str) -> bool {
    photocraft_engine::file_cmds::extension(path).and_then(|e| photocraft_codecs::from_extension(&e)) == Some(photocraft_codecs::Format::Jpeg)
}

/// Whether saving the active document to `path` asks for JPEG Options.
pub fn wants_prompt(app: &PhotocraftApp, path: &str) -> bool {
    is_jpeg_path(path) && app.session.active().is_some()
}

/// Parks the save of the active document to `path` behind the prompt, starting at
/// [`Start::for_active`]. A prompt already waiting is never replaced.
pub fn park(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str) -> Result<(), String> {
    let start = Start::for_active(app);
    park_at(app, path, copy, record, start)
}

/// [`park`], starting at `start`.
pub fn park_at(app: &mut PhotocraftApp, path: String, copy: bool, record: &'static str, start: Start) -> Result<(), String> {
    if app.save_options_open() {
        return Err(crate::SAVE_OPTIONS_OPEN.into());
    }
    let doc = app.active_doc_id()?;
    let Start { quality, own_quality, format } = start;
    app.jpeg_options = Some(Prompt { doc, path, quality: quality.min(MAX_QUALITY), format, own_quality, copy, record });
    Ok(())
}

pub fn show(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = &mut app.jpeg_options else { return };
    let (mut ok, mut cancel) = (false, false);
    let (mut quality, mut format) = (p.quality, p.format);
    let modal = egui::Modal::new(egui::Id::new("jpeg-options")).show(ctx, |ui| {
        ui.set_max_width(360.0);
        ui.label(egui::RichText::new(tl!("JPEG Options")).font(crate::theme::semibold(15.0)));
        ui.add_space(4.0);
        crate::widgets::hairline(ui);
        ui.add_space(8.0);
        crate::widgets::section_label(ui, tl!("Image Options"));
        ui.horizontal(|ui| {
            ui.label(tl!("Quality"));
            ui.add(egui::DragValue::new(&mut quality).range(0..=MAX_QUALITY).speed(0.1));
            // Photoshop's presets: picking one sets the quality it starts at.
            egui::ComboBox::from_id_salt("jpeg-quality-preset").selected_text(quality_label(quality)).show_ui(ui, |ui| {
                for q in [3, 5, 8, 10] {
                    if ui.selectable_label(quality_label(quality) == quality_label(q), quality_label(q)).clicked() {
                        quality = q;
                    }
                }
            });
        });
        ui.add(egui::Slider::new(&mut quality, 0..=MAX_QUALITY).show_value(false));
        ui.add_space(8.0);
        crate::widgets::section_label(ui, tl!("Format Options"));
        ui.radio_value(&mut format, JpegFormat::Baseline, tl!("Baseline (\"Standard\")"));
        ui.radio_value(&mut format, JpegFormat::Optimized, tl!("Baseline Optimized"));
        ui.radio_value(&mut format, JpegFormat::Progressive, tl!("Progressive"));
        ui.add_space(12.0);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ok = crate::widgets::primary_button(ui, tl!("OK"), 84.0).clicked();
            cancel = crate::widgets::secondary_button(ui, tl!("Cancel"), 84.0).clicked();
        });
    });
    cancel |= modal.should_close();
    p.quality = quality.min(MAX_QUALITY);
    p.format = format;
    // A focused button takes Enter as its click without consuming the key: Enter on Cancel cancels.
    if ok || (!cancel && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Enter))) {
        confirm(app, ctx);
    } else if cancel {
        app.jpeg_options = None;
        app.ui.status = tl!("Save cancelled").into();
        app.ui.status_error = false;
    }
}

/// OK: writes the waiting save with the prompt's quality and format, and journals it.
pub fn confirm(app: &mut PhotocraftApp, ctx: &egui::Context) {
    let Some(p) = app.jpeg_options.take() else { return };
    // Left where it started, an opened JPEG keeps its own quality exactly rather than the nearest
    // step on the 0–12 scale (95 would be written as 92). The recorded step then leaves the
    // quality out, so a replay keeps the file's own quality too, as File › Save does (#4032).
    let own = p.own_quality.filter(|&q| quality_from_percent(u32::from(q)) == p.quality);
    let choice = JpegChoice { quality: own.is_none().then_some(p.quality), format: p.format };
    let step = if p.record == "file.save" {
        // A plain Save writes whichever file the action is played on (#3531), with the answer.
        plain_save_step(Some(choice))
    } else {
        let mut step = serde_json::json!({"path": p.path});
        choice.record(&mut step);
        step
    };
    let mut settings = choice.apply(app.save_settings());
    if own.is_some() {
        settings.jpeg_quality = own;
    }
    app.write_answered_save(ctx, p.doc, p.path, &settings, p.copy, (p.record, step));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn jpeg_names() {
        for p in ["a.jpg", "dir/sub/a.JPEG", "C:\\x\\y.jpe", "a.jpg.jpg"] {
            assert!(is_jpeg_path(p), "{p}");
        }
        for p in ["a.jpg.psd", ".jpg", "jpg", "a.png", "a.jp2", ""] {
            assert!(!is_jpeg_path(p), "{p}");
        }
    }

    #[test]
    fn choice_params() {
        let base = JpegFormat::Baseline;
        assert_eq!(choice_param(&json!({}), "file.saveAs"), Ok(None));
        assert_eq!(choice_param(&json!({"quality": null, "jpegFormat": null}), "file.saveAs"), Ok(None));
        assert_eq!(choice_param(&json!({"quality": 12}), "file.saveAs"), Ok(Some(JpegChoice { quality: Some(12), format: base })));
        assert_eq!(choice_param(&json!({"quality": 7.6}), "file.saveAs"), Ok(Some(JpegChoice { quality: Some(8), format: base })));
        assert_eq!(choice_param(&json!({"jpegFormat": "progressive"}), "file.saveAs"), Ok(Some(JpegChoice { quality: None, format: JpegFormat::Progressive })));
        for bad in [
            json!({"quality": 13}),
            json!({"quality": -1}),
            json!({"quality": 1e300}),
            json!({"quality": "8"}),
            json!({"quality": true}),
            json!({"quality": [8]}),
            json!({"jpegFormat": "Optimized"}),
            json!({"jpegFormat": 1}),
            json!({"quality": 8, "jpegFormat": ""}),
        ] {
            assert!(choice_param(&bad, "file.saveAs").is_err(), "{bad}");
        }
    }

    #[test]
    fn recorded_steps_read_like_the_engine() {
        let base = JpegFormat::Baseline;
        assert_eq!(recorded_choice(&json!({"path": "a.jpg"}), "file.saveACopy"), Ok(None));
        assert_eq!(recorded_choice(&json!({"quality": 90}), "file.saveACopy"), Ok(Some(JpegChoice { quality: Some(12), format: base })));
        assert_eq!(recorded_choice(&json!({"quality": -3}), "file.saveACopy"), Ok(Some(JpegChoice { quality: Some(0), format: base })));
        assert_eq!(recorded_choice(&json!({"quality": "8"}), "file.saveACopy"), Ok(None), "a quality that isn't a number is ignored");
        assert_eq!(
            recorded_choice(&json!({"quality": 7.6, "jpegFormat": "optimized"}), "file.saveAs"),
            Ok(Some(JpegChoice { quality: Some(8), format: JpegFormat::Optimized }))
        );
        assert!(recorded_choice(&json!({"jpegFormat": "fast"}), "file.saveACopy").is_err());
    }

    #[test]
    fn quality_scale() {
        assert_eq!([0, 4, 5, 7, 8, 9, 10, 12].map(quality_label), ["Low", "Low", "Medium", "Medium", "High", "High", "Maximum", "Maximum"]);
        // The prompt's starting quality maps back to the encoder quality it came from, near enough.
        for q in 0..=MAX_QUALITY {
            assert_eq!(quality_from_percent(u32::from(photocraft_engine::file_cmds::jpeg_quality(f64::from(q)))), q);
        }
        assert_eq!((quality_from_percent(0), quality_from_percent(85), quality_from_percent(1000)), (0, 10, 12));
    }

    #[test]
    fn starts() {
        assert_eq!(Start::own(95), Start { quality: 11, own_quality: Some(95), format: JpegFormat::Baseline });
        assert_eq!(Start::own(0), Start { quality: 0, own_quality: Some(1), format: JpegFormat::Baseline });
        assert_eq!(Start::own(255).own_quality, Some(100));
        assert_eq!(Start::percent(85), Start { quality: 10, own_quality: None, format: JpegFormat::Baseline });
        assert_eq!(Start::percent(u32::MAX).quality, MAX_QUALITY);
    }

    /// The desktop and web exporters write through `ExportSettings::io_options`: the format picked
    /// in JPEG Options reaches the encoder there, not just the settings.
    #[test]
    fn the_answer_reaches_the_encoder() {
        let doc = photocraft_doc::Document::with_background(
            "x",
            photocraft_doc::Size::new(16, 16),
            photocraft_doc::ColorMode::Rgb,
            photocraft_doc::SampleType::U8,
            photocraft_doc::Color::WHITE,
        );
        let write = |format| {
            let settings = JpegChoice { quality: Some(10), format }.apply(ExportSettings::default());
            photocraft_io::export(&doc, "a.jpg", &settings.io_options()).unwrap().bytes
        };
        // Start of frame: 0xFFC0 baseline, 0xFFC2 progressive. Scan data stuffs its 0xFF bytes, and
        // this small white image's tables and headers hold neither pair, so only the frame marker matches.
        let sof = |b: &[u8], m: u8| b.windows(2).any(|w| w == [0xFF, m]);
        let (base, prog) = (write(JpegFormat::Baseline), write(JpegFormat::Progressive));
        assert!(sof(&base, 0xC0) && !sof(&base, 0xC2), "baseline");
        assert!(sof(&prog, 0xC2) && !sof(&prog, 0xC0), "progressive");
    }

    #[test]
    fn choices_apply_and_record() {
        let c = JpegChoice { quality: Some(10), format: JpegFormat::Optimized };
        let s = c.apply(ExportSettings::default());
        assert_eq!((s.jpeg_quality, s.jpeg_format), (Some(84), JpegFormat::Optimized));
        let mut step = json!({"path": "a.jpg"});
        c.record(&mut step);
        assert_eq!(step, json!({"path": "a.jpg", "quality": 10, "jpegFormat": "optimized"}));
        assert_eq!(choice_param(&step, "file.saveAs"), Ok(Some(c)), "a recorded step replays the same answer");
        let unset = JpegChoice { quality: None, format: JpegFormat::Progressive }.apply(ExportSettings { jpeg_quality: Some(95), ..ExportSettings::default() });
        assert_eq!(unset.jpeg_quality, Some(95), "no quality keeps the quality the save had");
    }
}
