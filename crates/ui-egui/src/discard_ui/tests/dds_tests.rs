//! DDS Options on Save As and Save a Copy: the compression and mipmaps it asks for, the scripted
//! `ddsCompression` and `ddsMipmaps` that skip it, and action recording and playback (#1327).
use super::*;
use crate::{FileDialogAnswer, dds_options_ui};
use egui_kittest::kittest::Queryable;
use photocraft_io::{DdsCompression, DdsOptions};
use std::{cell::RefCell, rc::Rc};

/// Each write: the path, and the DDS options the export was asked for.
type Writes = Rc<RefCell<Vec<(String, DdsOptions)>>>;

const PLAIN: DdsOptions = DdsOptions { compression: DdsCompression::None, mipmaps: false };

/// One 8×8 RGB document whose exports record their DDS options, in a harness showing DDS Options
/// and answering a save dialog with `to`.
fn dds_app(to: &str) -> (Prompted, Writes) {
    let mut app = app_with_docs(1);
    let writes: Writes = Rc::default();
    let (asked, written) = (Rc::new(RefCell::new(PLAIN)), writes.clone());
    let seen = asked.clone();
    app.services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(to.into()))]).0);
    app.services.export = Some(Box::new(move |_, _, settings| {
        *seen.borrow_mut() = settings.dds;
        Ok((Vec::new(), Vec::new()))
    }));
    app.services.write = Some(Box::new(move |path, _| {
        written.borrow_mut().push((path.to_string(), *asked.borrow()));
        Ok(())
    }));
    let h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).build_ui_state(
        |ui, app| {
            show(app, ui.ctx());
            dds_options_ui::show(app, ui.ctx());
            app.poll_file_dialog(ui.ctx(), None);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.ctx.set_os(egui::os::OperatingSystem::Windows);
    (h, writes)
}

/// Save As to a DDS texture asks first, starting uncompressed without mipmaps, and writes only
/// what is chosen once answered.
#[test]
fn save_as_to_a_dds_texture_asks_for_its_compression() {
    let (mut h, writes) = dds_app("unused.dds");
    h.state_mut().save_as(Some("t.dds".into())).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().dds_options.as_ref().map(|p| (p.options, p.copy)), Some((PLAIN, false)));
    assert!(writes.borrow().is_empty(), "nothing is written before the answer");
    assert!(h.state_mut().save_as(Some("other.dds".into())).is_err(), "one save at a time");
    h.get_by_label("BC3 / DXT5").click();
    h.run_steps(2);
    h.get_by_label("Generate Mipmaps").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    let chosen = DdsOptions { compression: DdsCompression::Bc3, mipmaps: true };
    assert_eq!(writes.borrow().as_slice(), &[("t.dds".to_string(), chosen)]);
    assert!(h.state().dds_options.is_none());
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("t.dds"));

    // Cancel writes nothing.
    h.state_mut().save_as(Some("c.dds".into())).unwrap();
    h.run_steps(2);
    h.key_press(Key::Escape);
    h.run_steps(3);
    assert!(h.state().dds_options.is_none());
    assert_eq!(writes.borrow().len(), 1, "cancelled: nothing more written");
    assert_eq!(h.state().ui.status, "Save cancelled");
}

/// Scripts choose with `ddsCompression` or `ddsMipmaps` and are never asked; bad values write
/// nothing, and other formats aren't asked.
#[test]
fn scripted_dds_options_and_other_formats_skip_the_prompt() {
    let (mut h, writes) = dds_app("unused.dds");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.dds", "ddsCompression": "bc1"})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "m.dds", "ddsMipmaps": true})).unwrap();
    assert!(h.state().dds_options.is_none());
    for bad in [json!({"ddsCompression": "bc7"}), json!({"ddsMipmaps": 1})] {
        let mut p = bad.clone();
        p["path"] = json!("bad.dds");
        assert!(crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", p).is_err(), "{bad}");
    }
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "p.png"})).unwrap();
    assert!(h.state().dds_options.is_none());
    assert_eq!(
        writes.borrow().as_slice(),
        &[
            ("s.dds".to_string(), DdsOptions { compression: DdsCompression::Bc1, mipmaps: false }),
            ("m.dds".to_string(), DdsOptions { compression: DdsCompression::None, mipmaps: true }),
            ("p.png".to_string(), PLAIN),
        ]
    );
}

/// Save a Copy to a DDS texture asks too, and the copy leaves the document's path and unsaved
/// state.
#[test]
fn save_a_copy_to_a_dds_texture_asks_and_keeps_the_document_unsaved() {
    let (mut h, writes) = dds_app("copy.dds");
    make_dirty(h.state_mut(), 0);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().dds_options.as_ref().map(|p| (p.options, p.copy)), Some((PLAIN, true)));
    assert!(dds_options_ui::park(h.state_mut(), "other.dds".into(), false, "file.saveAs").is_err(), "the waiting prompt is kept");
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert_eq!(writes.borrow().as_slice(), &[("copy.dds".to_string(), PLAIN)]);
    let st = h.state().session.active().unwrap();
    assert!(st.is_dirty());
    assert_eq!(st.path, None);
}

/// An action being recorded keeps a DDS Save As or Save a Copy with its options, answered in DDS
/// Options or scripted, and plays them back without asking.
#[test]
fn recorded_dds_saves_keep_their_options_and_replay_without_asking() {
    let (mut h, writes) = dds_app("copy.dds");
    let ctx = h.ctx.clone();
    h.state_mut().run("actions.record", json!({})).unwrap();
    h.state_mut().save_as(Some("t.dds".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("BC1 / DXT1").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveACopy", json!({})).unwrap();
    h.run_steps(3);
    h.get_by_label("Generate Mipmaps").click();
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "s.dds", "ddsCompression": "bc2"})).unwrap();
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let want = [
        ("t.dds".to_string(), DdsOptions { compression: DdsCompression::Bc1, mipmaps: false }),
        ("copy.dds".to_string(), DdsOptions { compression: DdsCompression::None, mipmaps: true }),
        ("s.dds".to_string(), DdsOptions { compression: DdsCompression::Bc2, mipmaps: false }),
    ];
    assert_eq!(writes.borrow().as_slice(), &want);
    let steps = &h.state().session.actions.list[0].steps;
    assert_eq!(
        steps,
        &[
            ("file.saveAs".into(), json!({"path": "t.dds", "ddsCompression": "bc1", "ddsMipmaps": false})),
            ("file.saveACopy".into(), json!({"path": "copy.dds", "ddsCompression": "none", "ddsMipmaps": true})),
            ("file.saveAs".into(), json!({"path": "s.dds", "ddsCompression": "bc2", "ddsMipmaps": false}))
        ]
    );
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(h.state().dds_options.is_none(), "playback doesn't ask");
    assert_eq!(writes.borrow().as_slice(), &want);
}

/// File › Save confirming the document's own texture asks DDS Options, and an action being
/// recorded keeps it as a plain Save with the answer (scripted options too): played back, it
/// writes the texture the action is played on as recorded, without asking (#3531).
#[test]
fn recorded_plain_save_of_a_dds_texture_keeps_its_options() {
    let (mut h, writes) = dds_app("own.dds");
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "own.dds", "ddsCompression": "none"})).unwrap();
    assert_eq!(h.state().session.active().unwrap().path.as_deref(), Some("own.dds"));
    h.state_mut().run("actions.record", json!({})).unwrap();
    crate::menus::invoke(h.state_mut(), &ctx, "file.save", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().dds_options.is_some(), "confirming its own texture asks DDS Options");
    h.get_by_label("BC1 / DXT1").click();
    h.run_steps(2);
    h.get_by_label("OK").click();
    h.run_steps(3);
    h.state_mut().services.file_dialog = Some(crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo("own.dds".into()))]).0);
    crate::menus::invoke(h.state_mut(), &ctx, "file.save", json!({"ddsCompression": "bc3", "ddsMipmaps": true})).unwrap();
    h.run_steps(3);
    assert!(h.state().dds_options.is_none(), "scripted options don't ask");
    h.state_mut().run("actions.stop", json!({})).unwrap();
    let bc1 = DdsOptions { compression: DdsCompression::Bc1, mipmaps: false };
    let bc3 = DdsOptions { compression: DdsCompression::Bc3, mipmaps: true };
    assert_eq!(&writes.borrow()[1..], &[("own.dds".to_string(), bc1), ("own.dds".to_string(), bc3)]);
    let steps = &h.state().session.actions.list[0].steps;
    assert_eq!(
        steps,
        &[
            ("file.save".into(), json!({"ddsCompression": "bc1", "ddsMipmaps": false})),
            ("file.save".into(), json!({"ddsCompression": "bc3", "ddsMipmaps": true}))
        ]
    );
    crate::menus::invoke(h.state_mut(), &ctx, "file.saveAs", json!({"path": "other.dds", "ddsCompression": "none"})).unwrap();
    writes.borrow_mut().clear();
    let played = h.state_mut().run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(h.state().dds_options.is_none(), "playback doesn't ask");
    assert_eq!(writes.borrow().as_slice(), &[("other.dds".to_string(), bc1), ("other.dds".to_string(), bc3)]);
}

/// Enter on a focused Cancel cancels: egui clicks the focused button on Enter without consuming
/// the key, so the dialog's Enter-for-OK must not save as well.
#[test]
fn enter_on_a_focused_cancel_writes_nothing() {
    let (mut h, writes) = dds_app("unused.dds");
    h.state_mut().save_as(Some("t.dds".into())).unwrap();
    h.run_steps(2);
    h.get_by_label("Cancel").focus();
    h.run_steps(2);
    h.key_press(Key::Enter);
    h.run_steps(3);
    assert!(h.state().dds_options.is_none());
    assert!(writes.borrow().is_empty(), "nothing written");
    assert_eq!(h.state().ui.status, "Save cancelled");
}
