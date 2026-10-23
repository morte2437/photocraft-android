//! JPEG Options on Save As and Save a Copy (FILE-215-19): the quality and format it asks for, the
//! scripted `quality` / `jpegFormat` that skip it, recording and playback, and the close prompt
//! waiting behind it.
use super::*;
use crate::{FileDialogAnswer, jpeg_options_ui};
use egui_kittest::kittest::Queryable;
use photocraft_codecs::JpegFormat;
use std::{cell::RefCell, rc::Rc};

/// Each write: the path, and the JPEG quality (1–100) and format the export was asked for.
type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// The JSON a write records.
fn asked(quality: Option<u8>, format: JpegFormat) -> Vec<u8> {
    serde_json::to_vec(&(quality, format.name())).unwrap()
}

/// The encoder quality for Photoshop quality `q`.
fn pct(q: u8) -> Option<u8> {
    Some(photocraft_engine::file_cmds::jpeg_quality(f64::from(q)))
}

/// One 8×8 RGB document whose exports record their JPEG settings, in a harness showing JPEG
/// Options and answering a save dialog with `to`.
fn jpeg_app(to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs(1);
    let writes: Writes = Rc::default();
    let written = writes.clone();
    app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(to.into()))]).0);
    app.services.export = Some(Box::new(|_, _, settings| Ok((asked(settings.jpeg_quality, settings.jpeg_format), Vec::new()))));
    app.services.write = Some(Box::new(move |path, bytes| {
        written.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            show(app, ui.ctx());
            jpeg_options_ui::show(app, ui.ctx());
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    (h, writes)
}

/// Save As to a JPEG asks first, starting at the Export preferences' quality (85 → 10, Maximum)
/// and Baseline, and writes only once answered.
#[test]
fn save_as_to_a_jpeg_asks_for_jpeg_options() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().jpeg_options.as_ref().map(|p| (p.quality, p.format, p.copy)), Some((10, JpegFormat::Baseline, false)));
    assert!(h.query_by_label("JPEG Options").is_some());
    assert!(writes.borrow().is_empty(), "nothing is written before the answer");
    assert!(h.state_mut().save_as(Some("other.jpg".into())).is_err(), "one save at a time");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("p.jpg".to_string(), asked(pct(10), JpegFormat::Baseline))]);
    assert!(h.state().jpeg_options.is_none());
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("p.jpg"));
}

/// Re-saving an opened JPEG starts at the file's own quality (`file_encoding`, as File › Save
/// keeps it since #4032), not the Export preference's; left there, OK writes that quality exactly
/// rather than its nearest step on the 0–12 scale (95 would come back as 92), and the recorded step
/// leaves the quality out, so playback keeps it too. Save a Copy does the same; a quality changed
/// in the prompt is written and recorded on the scale.
#[test]
fn resaving_an_opened_jpeg_starts_at_its_own_quality() {
    let (mut h, writes) = jpeg_app("copy.jpg");
    std::sync::Arc::make_mut(&mut h.state_mut().session.active_mut().unwrap().doc).file_encoding.jpeg_quality = Some(95);
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().jpeg_options.as_ref().map(|p| p.quality), Some(11), "95 is 11 on Photoshop's scale");
    h.key_press(Key::Enter);
    h.run_steps(3);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().jpeg_options.as_ref().map(|p| (p.quality, p.copy)), Some((11, true)));
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.state_mut().save_as(Some("q.jpg".into())).unwrap();
    h.run_steps(2);
    h.state_mut().jpeg_options.as_mut().unwrap().quality = 12;
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let expected = [
        ("p.jpg".to_string(), asked(Some(95), JpegFormat::Baseline)),
        ("copy.jpg".to_string(), asked(Some(95), JpegFormat::Baseline)),
        ("q.jpg".to_string(), asked(pct(12), JpegFormat::Baseline)),
    ];
    assert_eq!(writes.borrow().as_slice(), &expected);
    assert_eq!(
        h.state().session.actions.list[0].steps,
        [
            ("file.saveAs".into(), json!({"path": "p.jpg", "jpegFormat": "baseline"})),
            ("file.saveACopy".into(), json!({"path": "copy.jpg", "jpegFormat": "baseline"})),
            ("file.saveAs".into(), json!({"path": "q.jpg", "quality": 12, "jpegFormat": "baseline"})),
        ]
    );
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(writes.borrow().as_slice(), &expected, "playback writes the file's own quality again");
}

/// A `file.saveACopy` step recorded from the engine command keeps its params as given, and the
/// engine clamps a quality past 12; playing it back writes the copy the same way instead of
/// failing on the quality.
#[test]
fn a_copy_recorded_from_the_engine_command_replays() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    let path = std::env::temp_dir().join(format!("photocraft-jpeg-copy-{}.jpg", std::process::id())).to_string_lossy().into_owned();
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().run("file.saveACopy", json!({"path": path, "quality": 90})).unwrap();
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let _ = std::fs::remove_file(&path);
    assert_eq!(h.state().session.actions.list[0].steps, [("file.saveACopy".into(), json!({"path": path, "quality": 90}))]);
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(writes.borrow().as_slice(), &[(path, asked(pct(12), JpegFormat::Baseline))]);
}

/// The quality and format chosen in the prompt are the ones written; Cancel writes nothing.
#[test]
fn jpeg_options_writes_the_choice_or_cancels() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("Progressive").click();
    h.run_steps(2);
    h.state_mut().jpeg_options.as_mut().unwrap().quality = 3;
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("p.jpg".to_string(), asked(pct(3), JpegFormat::Progressive))]);

    h.state_mut().save_as(Some("c.jpg".into())).unwrap();
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().jpeg_options.is_none());
    assert_eq!(writes.borrow().len(), 1, "cancelled: nothing more written");
    assert_eq!(h.state().ui.status, "Save cancelled");
}

/// Scripts choose with `quality` or `jpegFormat` and are never asked; a bad value writes nothing;
/// other formats aren't asked.
#[test]
fn scripted_jpeg_options_and_other_formats_skip_the_prompt() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.jpg", "quality": 12, "jpegFormat": "optimized"})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "f.jpeg", "jpegFormat": "progressive"})).unwrap();
    assert!(h.state().jpeg_options.is_none());
    for bad in [json!({"quality": 13}), json!({"quality": "8"}), json!({"jpegFormat": "fast"})] {
        let mut p = bad.clone();
        p["path"] = json!("s.jpg");
        assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", p).is_err(), "{bad}");
    }
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "p.png"})).unwrap();
    assert!(h.state().jpeg_options.is_none());
    assert_eq!(
        writes.borrow().as_slice(),
        &[
            ("s.jpg".to_string(), asked(Some(100), JpegFormat::Optimized)),
            ("f.jpeg".to_string(), asked(None, JpegFormat::Progressive)),
            ("p.png".to_string(), asked(None, JpegFormat::Baseline)),
        ]
    );
}

/// Save a Copy to a JPEG asks too, and the copy leaves the document's path and unsaved state.
#[test]
fn save_a_copy_to_a_jpeg_asks_and_keeps_the_document_unsaved() {
    let (mut h, writes) = jpeg_app("copy.jpg");
    make_dirty(h.state_mut(), 0);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().jpeg_options.as_ref().map(|p| p.copy), Some(true));
    assert!(jpeg_options_ui::park(h.state_mut(), "other.jpg".into(), false, "file.saveAs").is_err(), "the waiting prompt is kept");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("copy.jpg".to_string(), asked(pct(10), JpegFormat::Baseline))]);
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty());
    assert_eq!(st.path, None);
}

/// An action being recorded keeps a JPEG Save As or Save a Copy with its answer (#2032, #4373)
/// and plays them back without asking.
#[test]
fn recorded_jpeg_saves_keep_their_options_and_replay_without_asking() {
    let (mut h, writes) = jpeg_app("copy.jpg");
    let ctx = h.ctx.clone();
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some("p.jpg".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("Baseline Optimized").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    h.state_mut().jpeg_options.as_mut().unwrap().quality = 6;
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let steps = &h.state().session.actions.list[0].steps;
    assert_eq!(
        steps,
        &[
            ("file.saveAs".into(), json!({"path": "p.jpg", "quality": 10, "jpegFormat": "optimized"})),
            ("file.saveACopy".into(), json!({"path": "copy.jpg", "quality": 6, "jpegFormat": "baseline"})),
        ]
    );
    let expected = [("p.jpg".to_string(), asked(pct(10), JpegFormat::Optimized)), ("copy.jpg".to_string(), asked(pct(6), JpegFormat::Baseline))];
    assert_eq!(writes.borrow().as_slice(), &expected);
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(h.state().jpeg_options.is_none(), "playback doesn't ask");
    assert_eq!(writes.borrow().as_slice(), &expected);
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("p.jpg"), "the replayed copy leaves the document's file");
}

/// Closing an unsaved document whose save goes to a JPEG waits behind JPEG Options, then closes.
#[test]
fn closing_after_a_jpeg_save_waits_for_jpeg_options() {
    let (mut h, writes) = jpeg_app("kept.jpg");
    // An edit that keeps the lone Background, so the JPEG holds the document and isn't a copy.
    h.state_mut().run("edit.fill", json!({"color": "#000000"})).unwrap();
    assert!(intercept(h.state_mut(), "file.close", &Value::Null));
    h.run_steps(2);
    h.key_press(Key::Y);
    h.run_steps(3);
    assert!(h.state().jpeg_options.is_some());
    assert_eq!(h.state().session.documents().len(), 1, "nothing closes before JPEG Options saves");
    assert!(h.query_by_label("(Y)es").is_none(), "the close prompt waits behind JPEG Options");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("kept.jpg".to_string(), asked(pct(10), JpegFormat::Baseline))]);
    assert!(h.state().discard.is_none());
    assert!(h.state().session.documents().is_empty());
}

/// Another save can park the prompt with a starting point of its own (`park_at`): an own quality
/// left alone is written exactly and recorded without `quality`; the starting format is kept.
#[test]
fn a_save_can_choose_where_the_prompt_starts() {
    let (mut h, writes) = jpeg_app("unused.jpg");
    h.state_mut().run("actions.record", json!({})).unwrap();
    let start = jpeg_options_ui::Start { format: JpegFormat::Progressive, ..jpeg_options_ui::Start::own(70) };
    jpeg_options_ui::park_at(h.state_mut(), "s.jpg".into(), false, "file.saveAs", start).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().jpeg_options.as_ref().map(|p| (p.quality, p.format, p.own_quality)), Some((8, JpegFormat::Progressive, Some(70))));
    assert!(jpeg_options_ui::park_at(h.state_mut(), "t.jpg".into(), false, "file.saveAs", start).is_err(), "one prompt at a time");
    h.key_press(Key::Enter);
    h.run_steps(3);
    h.state_mut().run("actions.stop", json!({})).unwrap();
    assert_eq!(writes.borrow().as_slice(), &[("s.jpg".to_string(), asked(Some(70), JpegFormat::Progressive))]);
    assert_eq!(h.state().session.actions.list[0].steps, [("file.saveAs".into(), json!({"path": "s.jpg", "jpegFormat": "progressive"}))]);
}
