use super::*;
use crate::file_dialog::{self, FileDialogAnswer, FileDialogRequest};
use crate::{Services, menus, notices};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Size;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;

/// Writes recorded by the fake writer: (path, bytes).
type Written = Rc<RefCell<Vec<(String, Vec<u8>)>>>;

/// An app whose importer names the document after the file (like `photocraft-io`) and reports a
/// warning for names containing "warn", and fails for bytes "bad"; the exporter warns for ".png"
/// and the writer records what it wrote. Its file dialogs answer with `answers`, in order.
fn app_with(answers: Vec<Option<FileDialogAnswer>>) -> (PhotocraftApp, Written) {
    let written: Written = Rc::default();
    let w = written.clone();
    let services = Services {
        import: Some(Box::new(|name: &str, bytes: &[u8], _opts: &photocraft_io::ImportOptions| {
            if bytes == b"bad" {
                return Err("not an image".into());
            }
            let warnings = if name.contains("warn") { vec!["Adjustment layer \"Curves 1\" was flattened".to_string()] } else { Vec::new() };
            Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), warnings))
        })),
        export: Some(Box::new(|_doc: &Document, path: &str, _s: &crate::ExportSettings| {
            let warnings = if path.ends_with(".png") { vec!["Layers were flattened".to_string()] } else { Vec::new() };
            Ok((b"out".to_vec(), warnings))
        })),
        file_dialog: Some(file_dialog::fake(answers).0),
        write: Some(Box::new(move |p: &str, b: &[u8]| {
            w.borrow_mut().push((p.to_string(), b.to_vec()));
            Ok(())
        })),
        ..Default::default()
    };
    (PhotocraftApp::new(photocraft_engine::Session::new(), services), written)
}

/// The end of a frame: the dialog asked for is shown, and its answer acted on.
fn answer(app: &mut PhotocraftApp) {
    app.poll_file_dialog(&egui::Context::default(), None);
}

/// Files named `names` (holding `bytes`) in a fresh directory for `tag`: (directory, paths).
fn files(tag: &str, names: &[&str], bytes: &[u8]) -> (std::path::PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("photocraft-file-open-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let paths = names
        .iter()
        .map(|name| {
            let path = dir.join(name);
            std::fs::write(&path, bytes).unwrap();
            path.to_string_lossy().into_owned()
        })
        .collect();
    (dir, paths)
}

#[test]
fn display_name_is_the_file_name() {
    assert_eq!(display_name("/a/b/photo.psd"), "photo.psd");
    assert_eq!(display_name("photo.psd"), "photo.psd");
    assert_eq!(display_name(""), "");
    assert_eq!(display_name("/"), "/");
}

#[test]
fn open_file_sets_name_path_and_recent() {
    let (mut app, _) = app_with(Vec::new());
    let w = app.open_file("/pics/cat.psd", b"x").unwrap();
    assert!(w.is_empty());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.name, "cat.psd");
    assert_eq!(st.path.as_deref(), Some("/pics/cat.psd"));
    assert_eq!(app.ui.recent_files, vec!["/pics/cat.psd".to_string()]);
    assert!(!app.ui.status_error);
    assert!(app.ui.notices.is_empty());
}

#[test]
fn affinity_preview_does_not_acquire_the_source_path_even_when_renamed() {
    for automation in [false, true] {
        for path in ["/pics/source.af", "/pics/renamed.psd"] {
            let (mut app, written) = app_with(vec![None]);
            // The fake importer supplies the preview; its source signature controls path policy.
            if automation {
                app.open_automation_bytes(&display_name(path), b"\x00\xffKA").unwrap();
                app.opened_from(path);
            } else {
                app.open_file(path, b"\x00\xffKA").unwrap();
            }
            assert!(app.session.active().unwrap().source_read_only);
            assert!(app.session.active().unwrap().path.is_none());
            assert_eq!(app.ui.recent_files.first().map(String::as_str), Some(path));
            // Save asks for a new file (cancelled here) instead of writing over the source.
            assert_eq!(menus::invoke(&mut app, &egui::Context::default(), "file.save", json!({})).unwrap(), json!({"fileDialog": "save"}));
            answer(&mut app);
            assert!(written.borrow().is_empty());
        }
    }
}

#[test]
fn file_open_dialog_sets_path_so_save_writes_in_place() {
    // The dialog returns a full path: the document is named after the file, not the path.
    let (dir, paths) = files("in-place", &["cat.psd"], b"x");
    let (mut app, written) = app_with(vec![Some(FileDialogAnswer::Paths(paths.clone()))]);
    let ctx = egui::Context::default();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap(), json!({"fileDialog": "open"}));
    answer(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.name, "cat.psd");
    assert_eq!(st.path.as_ref(), paths.first());
    assert_eq!(app.ui.recent_files.first(), paths.first());
    // File › Save goes straight back to the file, without a dialog.
    let r = menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(r["path"], paths[0]);
    assert!(!app.file_dialog_open());
    assert_eq!(written.borrow().last().map(|(p, _)| p), paths.first());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn file_open_dialog_opens_every_selected_path() {
    let (dir, paths) = files("issue-595", &["one.psd", "two.png"], b"x");
    let (mut app, _) = app_with(vec![Some(FileDialogAnswer::Paths(paths.clone()))]);

    menus::invoke(&mut app, &egui::Context::default(), "file.open", json!({})).unwrap();
    answer(&mut app);

    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(
        app.session.documents().iter().map(|doc| doc.path.as_deref()).collect::<Vec<_>>(),
        paths.iter().map(|path| Some(path.as_str())).collect::<Vec<_>>()
    );
    assert_eq!(app.ui.recent_files, vec![paths[1].clone(), paths[0].clone()]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn cancelling_file_open_opens_nothing() {
    let (mut app, _) = app_with(vec![None]);

    menus::invoke(&mut app, &egui::Context::default(), "file.open", json!({})).unwrap();
    answer(&mut app);

    assert!(app.session.documents().is_empty());
    assert!(!app.ui.status_error, "Cancel is not an error");
    assert!(!app.file_dialog_open());
}

#[test]
fn the_web_hands_over_contents_rather_than_paths() {
    let (mut app, _) = app_with(vec![Some(FileDialogAnswer::Contents("cat.png".into(), b"x".to_vec()))]);
    menus::invoke(&mut app, &egui::Context::default(), "file.open", json!({})).unwrap();
    answer(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.name.as_str(), st.path.as_deref()), ("cat.png", None));
}

#[test]
fn the_web_opens_every_selected_file() {
    let files = vec![("one.png".into(), b"x".to_vec()), ("broken.png".into(), b"bad".to_vec()), ("two.jpg".into(), b"x".to_vec())];
    let (mut app, _) = app_with(vec![Some(FileDialogAnswer::ContentsMany(files))]);
    menus::invoke(&mut app, &egui::Context::default(), "file.open", json!({})).unwrap();
    answer(&mut app);
    // Each good file is its own tab; the broken one is reported without stopping the rest.
    let names: Vec<_> = app.session.documents().iter().map(|st| st.doc.name.as_str()).collect();
    assert_eq!(names, ["one.png", "two.jpg"]);
    assert!(app.ui.notices.iter().any(|n| n.error && n.title.contains("broken.png")), "{:?}", app.ui.notices);
}

/// #5749: the web shell reads dropped files itself, so it asks where they landed: image files
/// dropped on the canvas are placed from their bytes, presets and drops elsewhere are opened.
#[test]
fn web_drops_onto_the_canvas_are_placed_from_their_bytes() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let names = vec!["photo.png".to_string(), "brushes.abr".to_string()];
    // No pointer known: nothing is placed.
    assert_eq!(app.dropped_on_canvas(&ctx, &names), [false, false]);
    let at = Rc::new(std::cell::Cell::new(egui::pos2(400.0, 300.0)));
    let reported = at.clone();
    app.services.cursor_pos = Some(Box::new(move |_: &egui::Context| Some(reported.get())));
    assert_eq!(app.dropped_on_canvas(&ctx, &names), [true, false]);
    at.set(egui::pos2(20.0, 20.0));
    assert_eq!(app.dropped_on_canvas(&ctx, &names), [false, false], "off the canvas: opened");
    let layers = layer_count(&app);
    let doc = app.session.active().unwrap().doc.id;
    app.drop_places.push_back((doc, crate::file_open::DropPlace::Bytes("photo.png".into(), png_bytes(4, 3))));
    app.drop_places.push_back((doc, crate::file_open::DropPlace::Bytes("bad.png".into(), b"bad".to_vec())));
    app.place_next_dropped(&ctx);
    assert_eq!(layer_count(&app), layers + 1);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    crate::transform_tool::commit(&mut app);
    // The undecodable one is reported, not placed.
    app.place_next_dropped(&ctx);
    assert_eq!(layer_count(&app), layers + 1);
    assert!(app.ui.notices.iter().any(|n| n.error && n.title.contains("bad.png")), "{:?}", app.ui.notices);
    assert!(app.drop_places.is_empty());
}

#[test]
fn pcraft_documents_save_in_place_but_flat_files_ask() {
    let (mut app, written) = app_with(vec![None]);
    let ctx = egui::Context::default();
    app.open_file("/pics/work.pcraft", b"x").unwrap();
    let r = menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(r["path"], "/pics/work.pcraft");
    assert_eq!(written.borrow().len(), 1);
    // A PNG goes through Save As (cancelled here), not silently flattened over the original.
    app.open_file("/pics/flat.png", b"x").unwrap();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap(), json!({"fileDialog": "save"}));
    answer(&mut app);
    assert_eq!(written.borrow().len(), 1);
    assert!(!app.ui.status_error);
}

/// #5733: like Photoshop's Save, a flat file still holding the whole document (one plain layer
/// it can store) is written back in place; once it would lose something (a second layer, or
/// transparency a JPEG can't keep) Save goes through Save As.
#[test]
fn a_flat_file_that_keeps_the_document_saves_in_place() {
    let (mut app, written) = app_with(vec![None, None]);
    let ctx = egui::Context::default();
    app.open_file("/pics/flat.png", b"x").unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let r = menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(r["path"], "/pics/flat.png", "{r}");
    assert_eq!(written.borrow().len(), 1);
    // A second layer: PNG can't keep it, so Save asks (cancelled).
    app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap(), json!({"fileDialog": "save"}));
    answer(&mut app);
    assert_eq!(written.borrow().len(), 1);
    // A JPEG with one transparent-capable layer that isn't the Background: JPEG has no alpha.
    app.open_file("/pics/photo.jpg", b"x").unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap(), json!({"fileDialog": "save"}));
    answer(&mut app);
    assert_eq!(written.borrow().len(), 1);
}

/// #5733: Save doesn't write one image back over a file that held more (here a two-size icon;
/// animations and multi-page TIFFs are recorded the same way), nor through an encoder that
/// quantises (GIF), nor a Grayscale document into a format that stores only colour.
#[test]
fn save_in_place_skips_files_it_would_write_back_with_less() {
    let (mut app, written) = app_with(vec![None, None, None]);
    app.services.import = Some(Box::new(|name: &str, bytes: &[u8], _opts: &photocraft_io::ImportOptions| {
        photocraft_io::import(name, bytes).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
    }));
    let ctx = egui::Context::default();
    let save = |app: &mut PhotocraftApp| menus::invoke(app, &ctx, "file.save", json!({})).unwrap();
    // One icon size: written back.
    app.open_file("/pics/one.ico", &ico(1)).unwrap();
    assert_eq!(save(&mut app)["path"], "/pics/one.ico");
    assert_eq!(written.borrow().len(), 1);
    // Two sizes: the document holds one, so Save asks (cancelled).
    app.open_file("/pics/two.ico", &ico(2)).unwrap();
    assert_eq!(save(&mut app), json!({"fileDialog": "save"}));
    answer(&mut app);
    // A recorded Save played on it doesn't write over the other sizes either.
    assert!(app.replay_save("file.save", &json!({})).is_err());
    // A GIF: its palette and on/off transparency would lose colours.
    app.open_file("/pics/still.gif", &gif_bytes()).unwrap();
    assert_eq!(save(&mut app), json!({"fileDialog": "save"}));
    answer(&mut app);
    // Grayscale in a BMP, which stores only colour.
    app.open_file("/pics/gray.bmp", &png_bytes(4, 3)).unwrap();
    app.run("image.mode.grayscale", json!({})).unwrap();
    assert_eq!(save(&mut app), json!({"fileDialog": "save"}));
    answer(&mut app);
    assert_eq!(written.borrow().len(), 1);
}

/// #5733: Save doesn't write in place where the format's writer would drop what the document has:
/// a gray layer's transparency in DPX, an embedded colour profile in BMP, or a clipped layer.
#[test]
fn save_in_place_skips_alpha_profiles_and_layer_state_the_file_would_drop() {
    let (mut app, written) = app_with(vec![None, None, None]);
    let ctx = egui::Context::default();
    let edit = |app: &mut PhotocraftApp, f: &dyn Fn(&mut Document)| f(std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc));
    // An RGB layer with transparency: DPX keeps its alpha, so it is written back.
    app.open_file("/pics/rgb.dpx", b"x").unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap()["path"], "/pics/rgb.dpx");
    assert_eq!(written.borrow().len(), 1);
    // A gray one: DPX writes gray without alpha.
    app.open_file("/pics/gray.dpx", b"x").unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    edit(&mut app, &|d| d.mode = ColorMode::Grayscale);
    assert_eq!(menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap(), json!({"fileDialog": "save"}));
    answer(&mut app);
    // A profile BMP can't embed.
    app.open_file("/pics/tagged.bmp", b"x").unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    edit(&mut app, &|d| d.icc_profile = Some(std::sync::Arc::new(vec![0; 128])));
    assert_eq!(menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap(), json!({"fileDialog": "save"}));
    answer(&mut app);
    // A clipped layer: the flat file keeps only its pixels.
    app.open_file("/pics/clipped.png", b"x").unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    edit(&mut app, &|d| d.layers[0].clipped = true);
    assert_eq!(menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap(), json!({"fileDialog": "save"}));
    answer(&mut app);
    assert_eq!(written.borrow().len(), 1);
}

/// An icon file whose `entries` directory entries all point at one 4×3 PNG image.
fn ico(entries: u16) -> Vec<u8> {
    let one = export_as(&png_bytes(4, 3), "i.ico");
    // ICONDIR (6 bytes) + one 16-byte ICONDIRENTRY whose last 4 bytes are the image offset.
    let (entry, image) = (&one[6..22], &one[22..]);
    let offset = 6 + 16 * u32::from(entries);
    let mut out = vec![0, 0, 1, 0];
    out.extend(entries.to_le_bytes());
    for _ in 0..entries {
        out.extend(&entry[..12]);
        out.extend(offset.to_le_bytes());
    }
    out.extend(image);
    out
}

/// A one-frame GIF.
fn gif_bytes() -> Vec<u8> {
    export_as(&png_bytes(4, 3), "g.gif")
}

/// `bytes` (a PNG) re-encoded for the file name `name`.
fn export_as(bytes: &[u8], name: &str) -> Vec<u8> {
    let doc = photocraft_io::import("p.png", bytes).unwrap().document;
    photocraft_io::export(&doc, name, &Default::default()).unwrap().bytes
}

#[test]
fn templates_open_as_new_untitled_documents() {
    let (mut app, written) = app_with(Vec::new());
    let ctx = egui::Context::default();
    app.open_file("/pics/card.PSDT", b"x").unwrap();
    app.open_file("/pics/card.psdt", b"x").unwrap();
    let names: Vec<_> = app.session.documents().iter().map(|d| d.doc.name.clone()).collect();
    assert_eq!(names, ["Untitled-1", "Untitled-2"]);
    assert!(app.session.documents().iter().all(|d| d.path.is_none()));
    assert_eq!(app.ui.recent_files.first().map(String::as_str), Some("/pics/card.psdt"));
    // File › Save asks for a new name (cancelled here) rather than suggesting the template.
    let (show, asked) = file_dialog::fake(vec![None]);
    app.services.file_dialog = Some(show);
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    answer(&mut app);
    assert_eq!(*asked.borrow(), [FileDialogRequest::Save { suggested: "Untitled-2.psd".into() }]);
    assert!(written.borrow().is_empty());
}

#[test]
fn import_warnings_reach_status_notice_and_control_response() {
    let (dir, paths) = files("warn", &["warn.psd"], b"x");
    let p = paths[0].clone();
    let (mut app, _) = app_with(Vec::new());
    let ctx = egui::Context::default();
    let r = menus::invoke(&mut app, &ctx, "file.open", json!({ "path": p })).unwrap();
    assert_eq!(r["warnings"][0], "Adjustment layer \"Curves 1\" was flattened");
    assert!(app.ui.status.contains("was flattened"), "{}", app.ui.status);
    assert!(app.ui.status_error);
    assert_eq!(app.ui.notices.len(), 1);
    assert!(!app.ui.notices[0].error);
    assert_eq!(app.ui.notices[0].lines.len(), 1);
    // The document still opened, with its path.
    assert_eq!(app.session.active().and_then(|s| s.path.clone()), Some(p));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn export_warnings_reach_status_notice_and_control_response() {
    let (mut app, _) = app_with(Vec::new());
    let ctx = egui::Context::default();
    app.open_file("/pics/cat.psd", b"x").unwrap();
    let r = menus::invoke(&mut app, &ctx, "file.saveAs", json!({ "path": "/pics/cat.png" })).unwrap();
    assert_eq!(r["path"], "/pics/cat.png");
    assert_eq!(r["warnings"][0], "Layers were flattened");
    assert!(app.ui.status.contains("Saved cat.png"), "{}", app.ui.status);
    assert_eq!(app.ui.notices.len(), 1);
    // No warnings: no notice, and the response says so.
    let r = menus::invoke(&mut app, &ctx, "file.saveAs", json!({ "path": "/pics/cat.psd" })).unwrap();
    assert_eq!(r["warnings"], json!([]));
    assert_eq!(app.ui.notices.len(), 1);
}

#[test]
fn open_failures_are_errors_and_leave_no_document() {
    let (dir, paths) = files("broken", &["broken.psd"], b"bad");
    let (mut app, _) = app_with(vec![Some(FileDialogAnswer::Paths(paths))]);
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap();
    answer(&mut app);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.recent_files.is_empty());
    assert!(app.ui.status_error);
    assert!(app.ui.status.starts_with("Couldn't open broken.psd"), "{}", app.ui.status);
    assert!(app.ui.notices.last().is_some_and(|n| n.error));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_picked_file_that_cannot_be_read_is_reported_like_any_open_failure() {
    let missing = std::env::temp_dir().join("photocraft-missing-dir").join("big.psb").to_string_lossy().into_owned();
    let picked = || Some(FileDialogAnswer::Paths(vec![missing.clone()]));
    let (mut app, _) = app_with(vec![picked(), picked(), None]);
    app.open_dialog_file().unwrap();
    answer(&mut app);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.status_error);
    let notice = app.ui.notices.last().unwrap();
    assert!(notice.error && notice.title.starts_with("Couldn't open big.psb: "), "{}", notice.title);
    // Commands that read a picked file (scripts, notes, Place) fail with the file's name.
    app.ui.status_error = false;
    app.pick_file_bytes(|_, _, _| Err("read".into())).unwrap();
    answer(&mut app);
    assert!(app.ui.status_error && app.ui.status.starts_with("big.psb: "), "{}", app.ui.status);
    app.ui.status_error = false;
    app.pick_file_bytes(|_, _, _| Err("read".into())).unwrap();
    answer(&mut app);
    assert!(!app.ui.status_error, "cancelled");
}

#[test]
fn open_paths_reports_each_failure_without_panicking() {
    let (mut app, _) = app_with(Vec::new());
    let missing = std::env::temp_dir().join("photocraft-definitely-missing-file.psd").to_string_lossy().to_string();
    let dir = std::env::temp_dir().to_string_lossy().to_string();
    let n = app.open_paths(&[missing, String::new(), dir, "\u{0}".into()]);
    assert_eq!(n, 0);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.status_error);
    assert_eq!(app.ui.notices.len(), notices::MAX_NOTICES);
    // Bad control params error instead of panicking.
    let ctx = egui::Context::default();
    assert!(menus::invoke(&mut app, &ctx, "file.open", json!({ "path": "" })).is_err());
}

#[test]
fn os_open_events_open_files_with_paths() {
    let dir = std::env::temp_dir().join(format!("photocraft-os-open-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let a = dir.join("a.psd");
    let b = dir.join("b.png");
    std::fs::write(&a, b"x").unwrap();
    std::fs::write(&b, b"x").unwrap();
    let paths: Vec<String> = [&a, &b].iter().map(|p| p.to_string_lossy().to_string()).collect();
    let (mut app, _) = app_with(Vec::new());
    let mut queue = vec![OsEvent::Open(paths.clone())];
    app.services.os_events = Some(Box::new(move || std::mem::take(&mut queue)));
    let ctx = egui::Context::default();
    app.drain_os_events(&ctx);
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.active().and_then(|s| s.path.clone()).as_deref(), Some(paths[1].as_str()));
    assert_eq!(app.ui.recent_files, vec![paths[1].clone(), paths[0].clone()]);
    // Drained: a second poll opens nothing more.
    app.drain_os_events(&ctx);
    assert_eq!(app.session.documents().len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The viewport commands an `Open` event sends: focus, or an attention request on native Wayland
/// where winit can't focus (#3708).
fn raise_commands(is_wayland: bool) -> Vec<egui::ViewportCommand> {
    let (mut app, _) = app_with(Vec::new());
    app.services.is_wayland = is_wayland;
    let mut queue = vec![OsEvent::Open(Vec::new())];
    app.services.os_events = Some(Box::new(move || std::mem::take(&mut queue)));
    let mut out = egui::Context::default().run_ui(egui::RawInput::default(), |ui| app.drain_os_events(ui.ctx()));
    out.textures_delta.clear();
    out.viewport_output.get(&egui::ViewportId::ROOT).map(|v| v.commands.clone()).unwrap_or_default()
}

#[test]
fn an_os_open_brings_the_window_forward_or_asks_for_attention_on_wayland() {
    use egui::{UserAttentionType, ViewportCommand};
    let attention = ViewportCommand::RequestUserAttention(UserAttentionType::Informational);
    let x11 = raise_commands(false);
    assert!(x11.contains(&ViewportCommand::Focus) && !x11.contains(&attention), "{x11:?}");
    let wayland = raise_commands(true);
    assert!(wayland.contains(&attention) && !wayland.contains(&ViewportCommand::Focus), "{wayland:?}");
}

#[test]
fn os_quit_and_empty_events_do_not_panic() {
    let (mut app, _) = app_with(Vec::new());
    let mut queue = vec![OsEvent::Open(Vec::new()), OsEvent::Quit];
    app.services.os_events = Some(Box::new(move || std::mem::take(&mut queue)));
    app.drain_os_events(&egui::Context::default());
    assert!(app.session.documents().is_empty());
}

#[derive(Debug)]
struct FakeDrop(std::path::PathBuf, Result<Vec<u8>, String>);

impl egui::DroppedFile for FakeDrop {
    fn path(&self) -> &std::path::Path {
        &self.0
    }
    fn bytes(&self) -> Result<Vec<u8>, String> {
        self.1.clone()
    }
}

/// A dropped file at `path` whose bytes (if read through egui) are `bytes`.
fn dropped(path: impl Into<std::path::PathBuf>, bytes: Result<Vec<u8>, String>) -> egui::DroppedFileHandle {
    std::sync::Arc::new(FakeDrop(path.into(), bytes))
}

fn doc_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.documents().iter().map(|d| d.doc.name.clone()).collect()
}

#[test]
fn dropped_files_open_with_path_and_recent() {
    let (mut app, _) = app_with(Vec::new());
    let dir = std::env::temp_dir().join(format!("photocraft-drop-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let abs = dir.join("dropped.psd");
    std::fs::write(&abs, b"x").unwrap();
    let abs_s = abs.to_string_lossy().to_string();
    // Dropped where the pointer can't be read: each file opens as a document.
    app.open_dropped(
        &egui::Context::default(),
        vec![
            // A desktop drop (absolute path) is read from disk like File › Open (#375), never
            // through egui's whole-file `std::fs::read`.
            dropped(abs, Err("egui's reader must not be used".into())),
            // Unreadable and undecodable drops are errors; no path means no recent entry.
            dropped("gone.psd", Err("permission denied".into())),
            dropped("rel.psd", Ok(b"x".to_vec())),
            dropped("", Ok(b"bad".to_vec())),
        ],
        None,
    );
    assert_eq!(app.session.documents().len(), 2);
    assert_eq!(app.session.documents()[0].path.as_deref(), Some(abs_s.as_str()));
    assert_eq!(app.session.documents()[0].doc.name, "dropped.psd");
    assert_eq!(app.session.documents()[1].path, None);
    assert_eq!(app.ui.recent_files, vec![abs_s]);
    assert!(app.ui.status_error);
    assert_eq!(app.ui.notices.iter().filter(|n| n.error).count(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

/// PNG bytes of a white `w`×`h` image.
fn png_bytes(w: u32, h: u32) -> Vec<u8> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    photocraft_io::export(&s.active().unwrap().doc, "p.png", &Default::default()).unwrap().bytes
}

/// An app with one open document whose canvas showed at (100, 50)–(700, 550) last frame.
fn app_with_canvas() -> PhotocraftApp {
    let (mut app, _) = app_with(Vec::new());
    app.open_bytes("canvas.psd", b"x").unwrap();
    app.drop_canvas_rect = Some(egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(700.0, 550.0)));
    app
}

fn layer_count(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().doc.layers.len()
}

fn history_steps(app: &PhotocraftApp) -> usize {
    app.session.active().unwrap().history.past_len()
}

/// As in the reference app, files dropped on the canvas are placed one at a time, each in Free
/// Transform; the next is placed once the previous transform is committed or cancelled.
#[test]
fn files_dropped_onto_the_canvas_are_placed_in_free_transform_one_by_one() {
    let dir = std::env::temp_dir().join(format!("photocraft-drop-place-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let abs = dir.join("photo.png");
    std::fs::write(&abs, png_bytes(200, 100)).unwrap();
    let svg = dir.join("logo.svg");
    std::fs::write(&svg, b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"20\" height=\"10\"><rect width=\"20\" height=\"10\" fill=\"red\"/></svg>")
        .unwrap();
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    app.open_dropped(
        &ctx,
        vec![
            // Read from its path, placed like File › Place Embedded (fitted to the canvas).
            dropped(abs, Err("egui's reader must not be used".into())),
            // Undecodable: an error, skipped.
            dropped("bad.png", Ok(b"bad".to_vec())),
            // No path: placed from the dropped bytes.
            dropped("small.png", Ok(png_bytes(2, 2))),
        ],
        Some(egui::pos2(400.0, 300.0)),
    );
    app.place_next_dropped(&ctx);
    assert_eq!(app.session.documents().len(), 1, "drops onto the canvas don't open documents");
    assert_eq!(layer_count(&app), layers + 1, "one at a time");
    let t = app.ui.transform.as_ref().expect("the placed layer is in Free Transform");
    assert_eq!(t.made, Some(crate::state::MadeLayer::Place));
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "photo");
    // ↩ without changes: the place stays as it is.
    crate::transform_tool::commit(&mut app);
    assert_eq!((layer_count(&app), history_steps(&app)), (layers + 1, steps + 1));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Place Embedded"));
    // Next frame: the undecodable file is reported, the next one placed and transformed.
    app.place_next_dropped(&ctx);
    assert_eq!(app.ui.notices.iter().filter(|n| n.error).count(), 1);
    assert_eq!(layer_count(&app), layers + 2);
    let st = app.session.active().unwrap();
    let top = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(top.name, "small");
    assert!(matches!(top.content, photocraft_doc::LayerContent::Smart(_)));
    // Esc takes the place back, leaving nothing to redo.
    crate::transform_tool::cancel(&mut app);
    assert_eq!((layer_count(&app), history_steps(&app)), (layers + 1, steps + 1));
    assert!(!app.session.active().unwrap().history.can_redo());
    app.place_next_dropped(&ctx);
    assert!(app.ui.transform.is_none() && app.drop_places.is_empty());
    assert!(app.ui.recent_files.is_empty(), "placing isn't opening");
    app.open_dropped(&ctx, vec![dropped(svg, Err("egui's reader must not be used".into()))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    let st = app.session.active().unwrap();
    let placed_svg = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(placed_svg.name, "logo");
    assert!(matches!(placed_svg.content, photocraft_doc::LayerContent::Smart(_)), "SVG drops use Place Embedded");
    assert!(app.ui.transform.is_some(), "SVG drop should enter Free Transform");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #4833: a file dropped on the canvas on its own while the previous one is still in Free
/// Transform applies that box and is placed at once, instead of waiting unseen for ↩.
#[test]
fn a_separate_drop_applies_the_open_transform_and_places_at_once() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    let at = Some(egui::pos2(400.0, 300.0));
    app.open_dropped(&ctx, vec![dropped("first.png", Ok(png_bytes(4, 4)))], at);
    app.place_next_dropped(&ctx);
    assert!(app.ui.transform.is_some(), "the first file is in Free Transform");
    app.open_dropped(&ctx, vec![dropped("second.png", Ok(png_bytes(2, 2)))], at);
    app.place_next_dropped(&ctx);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "second", "placed without waiting for ↩");
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    // The first place was applied, not taken back: both layers, one Place Embedded step each.
    assert_eq!(layer_count(&app), layers + 2);
    assert!(app.drop_places.is_empty());
    crate::transform_tool::commit(&mut app);
    assert_eq!(history_steps(&app), steps + 2);
}

/// Apply the jobs that have finished, frame by frame, until `done` holds (or 30 s pass).
fn tick_until(app: &mut PhotocraftApp, ctx: &egui::Context, done: impl Fn(&PhotocraftApp) -> bool) {
    let t = std::time::Instant::now();
    while !done(app) {
        assert!(t.elapsed() < std::time::Duration::from_secs(30), "not done after 30 s");
        crate::jobs_ui::tick(app, ctx);
        app.place_next_dropped(ctx);
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
}

/// #10031: with background jobs a dropped file is decoded on a worker. Free Transform starts when
/// the layer has landed, and the next file is started only after that transform is applied.
#[test]
fn background_drops_are_placed_by_a_job_one_after_another() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.background_jobs = true;
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    let at = Some(egui::pos2(400.0, 300.0));
    app.open_dropped(&ctx, vec![dropped("first.png", Ok(png_bytes(4, 4))), dropped("second.png", Ok(png_bytes(2, 2)))], at);
    app.place_next_dropped(&ctx);
    let place = app.jobs.place.clone().expect("the first file is being placed");
    assert_eq!(place.name, "first.png");
    assert!(app.session.job(place.job).is_some(), "as a background job");
    assert_eq!((layer_count(&app), history_steps(&app)), (layers, steps), "nothing lands before the job is applied");
    assert!(app.ui.transform.is_none(), "Free Transform waits for the layer");
    // Frames while it runs don't start the second file next to it.
    app.place_next_dropped(&ctx);
    assert_eq!(app.drop_places.len(), 1);
    assert_eq!(app.jobs.place.as_ref().map(|p| p.job), Some(place.job));

    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    assert_eq!(layer_count(&app), layers + 1);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "first");
    // The second file waits for the first transform.
    app.place_next_dropped(&ctx);
    assert!(app.jobs.place.is_none() && app.drop_places.len() == 1);
    crate::transform_tool::commit(&mut app);
    app.place_next_dropped(&ctx);
    assert_eq!(app.jobs.place.as_ref().map(|p| p.name.as_str()), Some("second.png"));
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "second");
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    crate::transform_tool::commit(&mut app);
    assert_eq!((layer_count(&app), history_steps(&app)), (layers + 2, steps + 2));
    assert!(app.drop_places.is_empty());
}

/// #10031: where there are no threads (the web build) a dropped file is placed in slices, a few
/// milliseconds per frame, with `background_jobs` still off. Free Transform starts when the
/// layer has landed, and the next file waits for it.
#[test]
fn a_dropped_file_is_placed_over_several_frames_without_background_jobs() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    assert!(!app.background_jobs);
    app.session.set_cooperative_jobs(true);
    // One unit of work per frame, so the number of frames does not depend on the machine.
    app.session.set_job_step_budget(std::time::Duration::ZERO);
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    app.open_dropped(&ctx, vec![dropped("first.png", Ok(png_bytes(40, 40))), dropped("second.png", Ok(png_bytes(8, 8)))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    let place = app.jobs.place.clone().expect("the first file is being placed");
    assert_eq!(place.name, "first.png");
    assert!(app.session.job(place.job).is_some() && app.session.has_jobs());
    assert!(crate::jobs_ui::keep_polling(&app, &ctx), "frames keep coming while it runs");
    assert!(app.ui.transform.is_none(), "Free Transform waits for the layer");
    assert_eq!((layer_count(&app), history_steps(&app)), (layers, steps));
    // The status bar shows it, and it can be cancelled like any job.
    assert_eq!(crate::jobs_ui::job_in_view(&app).map(|j| j.id), Some(place.job));

    let mut frames = 0;
    while app.jobs.place.is_some() {
        frames += 1;
        assert!(frames < 10_000, "the place never landed");
        assert_eq!(layer_count(&app), layers, "nothing lands before the last frame");
        crate::jobs_ui::tick(&mut app, &ctx);
        app.place_next_dropped(&ctx);
    }
    assert!(frames > 10, "placed in {frames} frames");
    assert!(!app.background_jobs);
    assert_eq!(layer_count(&app), layers + 1);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "first");
    // The second file waits for the first transform, then is placed the same way.
    app.place_next_dropped(&ctx);
    assert!(app.jobs.place.is_none() && app.drop_places.len() == 1);
    crate::transform_tool::commit(&mut app);
    app.place_next_dropped(&ctx);
    assert_eq!(app.jobs.place.as_ref().map(|p| p.name.as_str()), Some("second.png"));
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    crate::transform_tool::commit(&mut app);
    assert_eq!((layer_count(&app), history_steps(&app)), (layers + 2, steps + 2));
    assert!(app.drop_places.is_empty());
}

/// A file dropped from a path, a damaged file and a cancel all end as they do with worker threads.
#[test]
fn stepped_places_from_a_path_fail_and_cancel_like_background_ones() {
    let dir = std::env::temp_dir().join(format!("photocraft-drop-place-steps-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.png");
    std::fs::write(&good, png_bytes(30, 30)).unwrap();
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.session.set_cooperative_jobs(true);
    app.session.set_job_step_budget(std::time::Duration::ZERO);
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    app.open_dropped(
        &ctx,
        vec![dropped(&good, Ok(Vec::new())), dropped("bad.png", Ok(b"bad".to_vec())), dropped("last.png", Ok(png_bytes(4, 4)))],
        Some(egui::pos2(400.0, 300.0)),
    );
    app.place_next_dropped(&ctx);
    // The first is a path: the `file.placeEmbedded` job, tracked like the others.
    let first = app.jobs.place.clone().expect("being placed");
    assert_eq!(first.name, "good.png");
    assert!(crate::jobs_ui::cancel(&mut app, first.job));
    crate::jobs_ui::tick(&mut app, &ctx);
    assert_eq!((layer_count(&app), history_steps(&app)), (layers, steps), "a cancelled place leaves the document alone");
    assert!(!app.ui.status_error);
    // The queue went on to the damaged file, which fails by name.
    assert_eq!(app.jobs.place.as_ref().map(|p| p.name.as_str()), Some("bad.png"));
    tick_until(&mut app, &ctx, |app| app.ui.notices.iter().any(|n| n.error && n.title.contains("bad.png")));
    assert_eq!(app.jobs.place.as_ref().map(|p| p.name.as_str()), Some("last.png"));
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    assert_eq!(layer_count(&app), layers + 1);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A cooperative drop from a path completes in slices (the path read and the placement job, not
/// egui's reader), making its layer once.
#[test]
fn a_cooperative_drop_from_a_path_completes_in_slices() {
    let dir = std::env::temp_dir().join(format!("photocraft-drop-path-slices-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let good = dir.join("good.png");
    std::fs::write(&good, png_bytes(30, 30)).unwrap();
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.session.set_cooperative_jobs(true);
    app.session.set_job_step_budget(std::time::Duration::ZERO);
    let layers = layer_count(&app);
    app.open_dropped(&ctx, vec![dropped(&good, Ok(Vec::new()))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    assert_eq!(app.jobs.place.as_ref().map(|p| p.name.as_str()), Some("good.png"));
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    assert_eq!(layer_count(&app), layers + 1);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file that can't be decoded is reported by name and the files dropped after it go on.
#[test]
fn a_background_place_that_fails_reports_it_and_the_queue_continues() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.background_jobs = true;
    let layers = layer_count(&app);
    app.open_dropped(&ctx, vec![dropped("bad.png", Ok(b"bad".to_vec())), dropped("good.png", Ok(png_bytes(2, 2)))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    assert_eq!(app.jobs.place.as_ref().map(|p| p.name.as_str()), Some("bad.png"));
    tick_until(&mut app, &ctx, |app| app.ui.notices.iter().any(|n| n.error && n.title.contains("bad.png")));
    assert_eq!(layer_count(&app), layers, "nothing was placed");
    assert_eq!(app.jobs.place.as_ref().map(|p| p.name.as_str()), Some("good.png"), "the next file started at once");
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    assert_eq!(layer_count(&app), layers + 1);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    assert_eq!(app.ui.notices.iter().filter(|n| n.error).count(), 1);
}

/// Cancelling the place says so, places nothing and goes on with the queue.
#[test]
fn cancelling_a_background_place_continues_the_queue() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.background_jobs = true;
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    app.open_dropped(&ctx, vec![dropped("first.png", Ok(png_bytes(4, 4))), dropped("second.png", Ok(png_bytes(2, 2)))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    let first = app.jobs.place.clone().expect("being placed");
    assert!(crate::jobs_ui::cancel(&mut app, first.job));
    crate::jobs_ui::tick(&mut app, &ctx);
    assert_eq!((layer_count(&app), history_steps(&app)), (layers, steps));
    let second = app.jobs.place.clone().expect("the queue went on");
    assert_eq!(second.name, "second.png");
    assert_ne!(second.job, first.job);
    assert!(!app.ui.status_error);
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "second");
    assert_eq!(layer_count(&app), layers + 1);
}

/// #8492: with Skip Transform when Placing on, dropped files still go through the background place
/// job one at a time, but no Free Transform opens when a layer lands: the next file starts as soon
/// as the previous one has landed, without waiting for ↩.
#[test]
fn skip_transform_when_placing_places_background_drops_without_free_transform() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.background_jobs = true;
    app.session.execute("prefs.set", json!({"path": "general.skipTransformWhenPlacing", "value": true})).unwrap();
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    app.open_dropped(&ctx, vec![dropped("first.png", Ok(png_bytes(4, 4))), dropped("second.png", Ok(png_bytes(2, 2)))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    let first = app.jobs.place.clone().expect("the first file is being placed");
    assert_eq!(first.name, "first.png");
    assert!(app.session.job(first.job).is_some(), "as a background job");
    assert_eq!(app.drop_places.len(), 1, "the second file waits for the first job");
    tick_until(&mut app, &ctx, |app| app.jobs.place.as_ref().is_some_and(|p| p.name == "second.png"));
    assert!(app.ui.transform.is_none(), "no Free Transform on the first file");
    assert_eq!(layer_count(&app), layers + 1);
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    assert!(app.ui.transform.is_none() && app.drop_places.is_empty());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "second");
    assert_eq!((layer_count(&app), history_steps(&app)), (layers + 2, steps + 2), "one Place Embedded step each");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Place Embedded"));
}

/// #8492: inline (background jobs off, as on the web) the preference places every queued drop
/// without Free Transform, a file that can't be placed reported on the way.
#[test]
fn skip_transform_when_placing_places_inline_drops_without_free_transform() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.session.execute("prefs.set", json!({"path": "general.skipTransformWhenPlacing", "value": true})).unwrap();
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    app.open_dropped(
        &ctx,
        vec![dropped("first.png", Ok(png_bytes(4, 4))), dropped("bad.png", Ok(b"bad".to_vec())), dropped("second.png", Ok(png_bytes(2, 2)))],
        Some(egui::pos2(400.0, 300.0)),
    );
    app.place_next_dropped(&ctx);
    assert!(app.ui.transform.is_none() && app.drop_places.is_empty());
    assert_eq!((layer_count(&app), history_steps(&app)), (layers + 2, steps + 2));
    assert_eq!(app.ui.notices.iter().filter(|n| n.error && n.title.contains("bad.png")).count(), 1);
}

/// #8492: Skip Transform when Placing only concerns the interactive place. `file.placeEmbedded`
/// with a `path` (actions, scripts, the control channel, MCP) runs the engine command, which never
/// opens Free Transform, whatever the preference.
#[test]
fn a_scripted_place_never_opens_free_transform() {
    let dir = std::env::temp_dir().join(format!("pc-scripted-place-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("dot.png");
    std::fs::write(&path, png_bytes(4, 3)).unwrap();
    let mut app = app_with_canvas();
    for skip in [false, true] {
        app.session.execute("prefs.set", json!({"path": "general.skipTransformWhenPlacing", "value": skip})).unwrap();
        let layers = layer_count(&app);
        app.run("file.placeEmbedded", json!({"path": path.to_string_lossy()})).unwrap();
        assert_eq!(layer_count(&app), layers + 1, "skip={skip}");
        assert!(app.ui.transform.is_none(), "skip={skip}");
    }
    std::fs::remove_dir_all(dir).unwrap();
}

/// Frames with nothing to wait for: jobs applied and the drop queue served.
fn idle_frames(app: &mut PhotocraftApp, ctx: &egui::Context) {
    for _ in 0..3 {
        crate::jobs_ui::tick(app, ctx);
        app.place_next_dropped(ctx);
    }
}

/// Drops two files on the canvas of document A (the app's only one) with background jobs on, starts
/// placing the first, then opens document B, which becomes active. Returns A's id and layer count.
fn two_drops_then_switch(app: &mut PhotocraftApp, ctx: &egui::Context) -> (photocraft_doc::DocId, usize) {
    app.background_jobs = true;
    let (a, layers) = (app.session.active().unwrap().doc.id, layer_count(app));
    app.open_dropped(ctx, vec![dropped("first.png", Ok(png_bytes(4, 4))), dropped("second.png", Ok(png_bytes(2, 2)))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(ctx);
    assert_eq!(app.jobs.place.as_ref().map(|p| (p.name.as_str(), p.doc)), Some(("first.png", a)));
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    assert_ne!(app.session.active().unwrap().doc.id, a);
    (a, layers)
}

/// FredApps' review of #10147: files dropped on A stay A's when the tab changes while the first is
/// placed. It lands in A, no Free Transform opens over B, the second waits rather than going into B,
/// and goes on in A when A is back in view.
#[test]
fn background_drops_stay_with_their_document_when_the_tab_changes() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let (a, layers) = two_drops_then_switch(&mut app, &ctx);
    let (b_layers, b_steps) = (layer_count(&app), history_steps(&app));
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    idle_frames(&mut app, &ctx);
    assert!(app.jobs.place.is_none() && app.ui.transform.is_none());
    assert_eq!((layer_count(&app), history_steps(&app)), (b_layers, b_steps), "nothing placed in B");
    assert_eq!(app.session.documents()[0].doc.layers.len(), layers + 1, "the first landed in A");
    assert_eq!(app.drop_places.len(), 1, "the second waits for A");

    assert!(app.session.set_active(0));
    app.sync_views();
    app.place_next_dropped(&ctx);
    assert_eq!(app.jobs.place.as_ref().map(|p| (p.name.as_str(), p.doc)), Some(("second.png", a)));
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    assert_eq!(layer_count(&app), layers + 2);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    assert!(app.drop_places.is_empty());
    assert_eq!(app.session.documents()[1].doc.layers.len(), b_layers);
}

/// Cancelling A's place while B is in view doesn't hand A's next file to B.
#[test]
fn cancelling_a_background_place_from_another_tab_keeps_the_queue_in_its_document() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let (a, layers) = two_drops_then_switch(&mut app, &ctx);
    let (b_layers, b_steps) = (layer_count(&app), history_steps(&app));
    let first = app.jobs.place.clone().unwrap();
    assert!(crate::jobs_ui::cancel(&mut app, first.job));
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    idle_frames(&mut app, &ctx);
    assert!(app.jobs.place.is_none());
    assert_eq!((layer_count(&app), history_steps(&app)), (b_layers, b_steps), "nothing placed in B");
    assert_eq!(app.session.documents()[0].doc.layers.len(), layers, "the cancelled file wasn't placed");
    assert_eq!(app.drop_places.len(), 1);
    assert!(app.session.set_active(0));
    app.place_next_dropped(&ctx);
    assert_eq!(app.jobs.place.as_ref().map(|p| (p.name.as_str(), p.doc)), Some(("second.png", a)));
}

/// Closing A drops its waiting files with a message; none goes into B.
#[test]
fn closing_the_document_discards_its_dropped_files() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    two_drops_then_switch(&mut app, &ctx);
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    app.run("file.close", json!({"document": 0})).unwrap();
    let (b_layers, b_steps) = (layer_count(&app), history_steps(&app));
    app.place_next_dropped(&ctx);
    assert!(app.drop_places.is_empty());
    assert!(app.ui.status.contains("closed") && !app.ui.status_error, "{}", app.ui.status);
    idle_frames(&mut app, &ctx);
    assert!(app.jobs.place.is_none() && app.ui.transform.is_none());
    assert_eq!((layer_count(&app), history_steps(&app)), (b_layers, b_steps));
}

/// The closed-document notice survives the same call that would start the next queued file: the
/// placement waits one frame, so the message is drawn. That frame is asked for at once, so B's file
/// doesn't wait for the next input event.
#[test]
fn the_closed_document_notice_stays_when_the_next_queued_file_starts() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let a = app.session.active().unwrap().doc.id;
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let b = app.session.active().unwrap().doc.id;
    let b_layers = layer_count(&app);
    app.drop_places.push_back((a, crate::file_open::DropPlace::Bytes("a.png".into(), png_bytes(4, 3))));
    app.drop_places.push_back((b, crate::file_open::DropPlace::Bytes("b.png".into(), png_bytes(4, 3))));
    app.run("file.close", json!({"document": 0})).unwrap();
    for _ in 0..4 {
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    }
    assert!(!ctx.has_requested_repaint(), "idle");
    app.place_next_dropped(&ctx);
    assert!(app.ui.status.contains("closed"), "{}", app.ui.status);
    assert_eq!(app.drop_places.len(), 1, "B's file waits for the next frame");
    assert!(ctx.has_requested_repaint(), "and asks for it");
    assert!(app.jobs.place.is_none() && app.ui.transform.is_none());
    app.place_next_dropped(&ctx);
    assert_eq!(layer_count(&app), b_layers + 1);
    assert!(app.drop_places.is_empty());
}

/// The drop queue is bounded: files dropped on another tab wait, and a browser drop holds its whole
/// file, so neither a flood of drops nor a huge one grows it without limit.
#[test]
fn the_drop_queue_is_bounded() {
    let mut app = app_with_canvas();
    let doc = app.session.active().unwrap().doc.id;
    for _ in 0..crate::file_open::MAX_QUEUED_DROPS {
        app.queue_drop(doc, crate::file_open::DropPlace::Bytes("p.png".into(), vec![0]));
    }
    assert_eq!(app.drop_places.len(), crate::file_open::MAX_QUEUED_DROPS);
    app.queue_drop(doc, crate::file_open::DropPlace::Bytes("over.png".into(), vec![0]));
    assert_eq!(app.drop_places.len(), crate::file_open::MAX_QUEUED_DROPS, "the queue keeps its cap");
    assert!(app.ui.status.contains("too many") && app.ui.status_error, "{}", app.ui.status);
    assert!(crate::file_open::queue_takes_drop(0, crate::file_open::MAX_QUEUED_DROP_BYTES, 0));
    assert!(!crate::file_open::queue_takes_drop(0, crate::file_open::MAX_QUEUED_DROP_BYTES, 1));
    assert!(!crate::file_open::queue_takes_drop(crate::file_open::MAX_QUEUED_DROPS, 0, 0));
}

/// Path drops hold no file contents while they wait, so the cap on browser drops doesn't apply to
/// them: dropping 40 files on the canvas queues all of them, and a browser drop after them still fits.
#[test]
fn many_path_drops_all_queue() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let doc = app.session.active().unwrap().doc.id;
    // More than the browser drops' cap; none is read while it waits.
    let n = crate::file_open::MAX_QUEUED_DROPS + 8;
    app.open_dropped(&ctx, (0..n).map(|i| dropped(format!("p{i}.png"), Ok(Vec::new()))).collect(), Some(egui::pos2(400.0, 300.0)));
    assert_eq!(app.drop_places.len(), n);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    app.queue_drop(doc, crate::file_open::DropPlace::Bytes("web.png".into(), vec![0]));
    assert_eq!(app.drop_places.len(), n + 1, "path drops don't use up the browser drops' room");
}

/// The shell's place inbox, which the browser's reads fill between two frames, takes no more than
/// the drop queue it is drained into.
#[test]
fn the_place_inbox_has_the_drop_queues_limits() {
    use crate::file_open::{MAX_QUEUED_DROP_BYTES, MAX_QUEUED_DROPS, place_inbox_takes};
    assert!(place_inbox_takes(0, MAX_QUEUED_DROP_BYTES - 1, 1));
    assert!(!place_inbox_takes(0, MAX_QUEUED_DROP_BYTES, 1), "a read that would pass the byte cap is refused");
    assert!(!place_inbox_takes(MAX_QUEUED_DROPS, 0, 0), "so is one past the file cap");
}

/// Closing A while its file is being placed: nothing lands anywhere, and A's next file is dropped.
#[test]
fn closing_the_document_while_its_file_is_placed_places_nothing_elsewhere() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    two_drops_then_switch(&mut app, &ctx);
    let (b_layers, b_steps) = (layer_count(&app), history_steps(&app));
    app.run("file.close", json!({"document": 0})).unwrap();
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    idle_frames(&mut app, &ctx);
    assert!(app.drop_places.is_empty() && app.jobs.place.is_none() && app.ui.transform.is_none());
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!((layer_count(&app), history_steps(&app)), (b_layers, b_steps));
}

/// Without background jobs too: leaving A's Free Transform for B cancels that place (#1099) and A's
/// next file waits for A instead of going into B.
#[test]
fn inline_drops_stay_with_their_document_when_the_tab_changes() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let layers = layer_count(&app);
    app.open_dropped(&ctx, vec![dropped("first.png", Ok(png_bytes(4, 4))), dropped("second.png", Ok(png_bytes(2, 2)))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    assert!(app.ui.transform.is_some());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    crate::transform_tool::end_if_left(&mut app);
    let (b_layers, b_steps) = (layer_count(&app), history_steps(&app));
    app.place_next_dropped(&ctx);
    assert!(app.ui.transform.is_none());
    assert_eq!((layer_count(&app), history_steps(&app)), (b_layers, b_steps), "nothing placed in B");
    assert_eq!(app.drop_places.len(), 1);
    assert!(app.session.set_active(0));
    app.sync_views();
    app.place_next_dropped(&ctx);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    assert_eq!(layer_count(&app), layers + 1, "the first was cancelled by leaving it, the second placed");
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, "second");
}

/// A web drop goes to the document it was dropped on, even when the tab changes before the shell
/// has read the files.
#[test]
fn web_drops_go_to_the_document_they_were_dropped_on() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let layers = layer_count(&app);
    let inbox: crate::PlaceInbox = Default::default();
    app.services.place_inbox = Some(inbox.clone());
    app.services.cursor_pos = Some(Box::new(|_: &egui::Context| Some(egui::pos2(400.0, 300.0))));
    assert_eq!(app.dropped_on_canvas(&ctx, &["photo.png".to_string()]), [true]);
    let target = app.session.active().map(|st| st.doc.id).unwrap();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let (b_layers, b_steps) = (layer_count(&app), history_steps(&app));
    inbox.lock().unwrap().push((target, "photo.png".into(), png_bytes(4, 3)));
    app.queue_web_places();
    app.place_next_dropped(&ctx);
    assert_eq!((layer_count(&app), history_steps(&app)), (b_layers, b_steps), "nothing placed in B");
    assert!(app.session.set_active(0));
    app.sync_views();
    app.place_next_dropped(&ctx);
    assert_eq!(layer_count(&app), layers + 1);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
}

#[test]
fn overlapping_web_drop_deliveries_keep_each_target() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let a = app.session.active().unwrap().doc.id;
    let a_layers = layer_count(&app);
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    let b = app.session.active().unwrap().doc.id;
    let b_layers = layer_count(&app);
    let inbox: crate::PlaceInbox = Default::default();
    app.services.place_inbox = Some(inbox.clone());
    // B's read finishes before A's, although A was dropped first.
    inbox.lock().unwrap().extend([(b, "b.png".into(), png_bytes(3, 2)), (a, "a.png".into(), png_bytes(4, 3))]);
    app.queue_web_places();
    app.place_next_dropped(&ctx);
    assert_eq!(layer_count(&app), b_layers + 1);
    crate::transform_tool::commit(&mut app);
    assert!(app.session.set_active(0));
    app.sync_views();
    app.place_next_dropped(&ctx);
    assert_eq!(layer_count(&app), a_layers + 1);
    assert!(app.drop_places.is_empty());
}

/// A file dropped from a path is placed by the `file.placeEmbedded` job, read on its worker, and a
/// drop that arrives meanwhile queues behind it.
#[test]
fn a_background_drop_from_a_path_is_tracked_and_later_drops_queue() {
    let dir = std::env::temp_dir().join(format!("photocraft-drop-place-job-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let abs = dir.join("photo.png");
    std::fs::write(&abs, png_bytes(40, 20)).unwrap();
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.background_jobs = true;
    let layers = layer_count(&app);
    let at = Some(egui::pos2(400.0, 300.0));
    app.open_dropped(&ctx, vec![dropped(abs, Err("egui's reader must not be used".into()))], at);
    app.place_next_dropped(&ctx);
    let place = app.jobs.place.clone().expect("being placed");
    assert_eq!(place.name, "photo.png");
    assert_eq!(app.jobs.last_started, Some(place.job));
    // A drop while it runs is queued, not started beside it.
    app.open_dropped(&ctx, vec![dropped("later.png", Ok(png_bytes(2, 2)))], at);
    app.place_next_dropped(&ctx);
    assert_eq!(app.drop_places.len(), 1);
    assert_eq!(app.session.jobs().len(), 1);
    tick_until(&mut app, &ctx, |app| app.jobs.place.is_none());
    assert_eq!(layer_count(&app), layers + 1);
    assert_eq!(app.drop_places.len(), 1, "waiting for the transform");
    let _ = std::fs::remove_dir_all(&dir);
}

/// #1099: switching tabs after an actual canvas drop cancels the place in its source document.
#[test]
fn switching_documents_after_a_canvas_drop_undoes_only_the_place() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let original = app.session.active().unwrap().doc.clone();
    let steps = history_steps(&app);
    app.open_dropped(&ctx, vec![dropped("dot.png", Ok(png_bytes(2, 2)))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    assert_eq!(app.ui.transform.as_ref().unwrap().made, Some(crate::state::MadeLayer::Place));
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(app.session.undo());
    let st = app.session.active().unwrap();
    let (other, history, redo, revision) = (st.doc.clone(), st.history.entries(), st.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), st.revision);
    crate::transform_tool::end_if_left(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
    let st = app.session.active().unwrap();
    assert!(std::sync::Arc::ptr_eq(&st.doc, &other));
    assert_eq!((st.history.entries(), st.revision), (history, revision));
    assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), redo);
    let origin = &app.session.documents()[0];
    assert!(std::sync::Arc::ptr_eq(&origin.doc, &original));
    assert_eq!(origin.history.past_len(), steps);
    assert!(!origin.history.can_redo());
}

/// Moving a placed file in its Free Transform and committing makes one Place Embedded step.
#[test]
fn a_transformed_place_is_one_history_step() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    let (layers, steps) = (layer_count(&app), history_steps(&app));
    app.open_dropped(&ctx, vec![dropped("dot.png", Ok(png_bytes(2, 2)))], Some(egui::pos2(400.0, 300.0)));
    app.place_next_dropped(&ctx);
    let t = app.ui.transform.as_mut().expect("in Free Transform");
    t.quad = t.quad.map(|[x, y]| [x - 1.0, y - 1.0]);
    crate::transform_tool::commit(&mut app);
    assert_eq!((layer_count(&app), history_steps(&app)), (layers + 1, steps + 1));
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Place Embedded"));
    app.session.undo();
    assert_eq!(layer_count(&app), layers, "one undo removes the place and the move");
}

#[test]
fn files_dropped_on_the_tabs_open_at_that_slot() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.ui.views[0].zoom = 3.0;
    // One tab at (100, 20)–(200, 46): its left half is slot 0, its right half and beyond slot 1.
    let tab = egui::Rect::from_min_max(egui::pos2(100.0, 20.0), egui::pos2(200.0, 46.0));
    let strip = egui::Rect::from_min_max(egui::pos2(92.0, 14.0), egui::pos2(700.0, 50.0));
    app.tab_strip = Some(crate::canvas::TabStrip { rect: strip, tabs: vec![(0, tab)] });
    assert_eq!(app.drop_target(&ctx, Some(egui::pos2(120.0, 30.0))), crate::file_open::DropTarget::Tabs(0));
    assert_eq!(app.drop_target(&ctx, Some(egui::pos2(180.0, 30.0))), crate::file_open::DropTarget::Tabs(1));
    assert_eq!(app.drop_target(&ctx, Some(egui::pos2(600.0, 30.0))), crate::file_open::DropTarget::Tabs(1));
    let files = vec![dropped("first.png", Ok(b"x".to_vec())), dropped("bad.png", Ok(b"bad".to_vec())), dropped("second.png", Ok(b"x".to_vec()))];
    app.open_dropped(&ctx, files, Some(egui::pos2(120.0, 30.0)));
    assert_eq!(doc_names(&app), ["first.png", "second.png", "canvas.psd"], "in drop order from the slot");
    assert_eq!(app.session.active_index(), Some(1), "the last one opened is active");
    assert_eq!(app.ui.views.len(), 3);
    assert_eq!(app.ui.views[2].zoom, 3.0, "views move with their documents");
    // Past the last tab: opened at the end.
    let tabs = (0..3).map(|i| (i, tab.translate(egui::vec2(104.0 * i as f32, 0.0)))).collect();
    app.tab_strip = Some(crate::canvas::TabStrip { rect: strip, tabs });
    app.open_dropped(&ctx, vec![dropped("last.png", Ok(b"x".to_vec()))], Some(egui::pos2(600.0, 30.0)));
    assert_eq!(app.session.documents().last().map(|d| d.doc.name.as_str()), Some("last.png"));
}

#[test]
fn a_drop_slot_counts_documents_not_the_tabs_shown() {
    // #1276: when the tabs overflow into the » menu, the shown tabs aren't documents 0, 1, 2…
    // Documents 4, 5 and 6 shown: a drop before document 5's tab opens at position 5.
    let tab = |i: usize, x: f32| (i, egui::Rect::from_min_max(egui::pos2(x, 20.0), egui::pos2(x + 100.0, 46.0)));
    let strip = crate::canvas::TabStrip {
        rect: egui::Rect::from_min_max(egui::pos2(0.0, 14.0), egui::pos2(700.0, 50.0)),
        tabs: vec![tab(4, 0.0), tab(5, 104.0), tab(6, 208.0)],
    };
    assert_eq!(strip.slot(20.0), 4);
    assert_eq!(strip.slot(130.0), 5);
    assert_eq!(strip.slot(500.0), 7, "past the last shown tab: after it");
    assert_eq!(crate::canvas::TabStrip { rect: strip.rect, tabs: vec![] }.slot(10.0), 0);
}

/// #5687: files dropped from another app's window bring PhotoCraft to the front (a focus request
/// in the frame that takes the drop), so the source window ends up behind it.
#[test]
fn a_drop_brings_the_window_to_the_front() {
    let dir = std::env::temp_dir().join(format!("photocraft-drop-focus-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("photo.png");
    std::fs::write(&file, png_bytes(8, 8)).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(800.0, 600.0)).with_max_steps(8).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Services::default())
    });
    h.run_steps(2);
    let focus = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
        h.output().viewport_output.get(&egui::ViewportId::ROOT).is_some_and(|v| v.commands.contains(&egui::ViewportCommand::Focus))
    };
    assert!(!focus(&h), "no focus request without a drop");
    h.input_mut().dropped_files.push(dropped(file, Err("read from the path".into())));
    h.step();
    assert!(focus(&h), "the drop brings the window forward");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The whole app (eframe harness, real layout): over the tab strip, the pointer read from the OS
/// service picks a slot; files dropped there open (in the background) at it; dropped on the canvas they're placed in Free Transform, and Esc takes the place back.
#[test]
fn drag_and_drop_in_the_running_app() {
    let dir = std::env::temp_dir().join(format!("photocraft-drop-app-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("photo.png");
    std::fs::write(&file, png_bytes(40, 30)).unwrap();
    let pointer = Rc::new(std::cell::Cell::new(egui::Pos2::ZERO));
    let reported = pointer.clone();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(8).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut s = photocraft_engine::Session::new();
        for _ in 0..2 {
            s.execute("file.new", json!({"width": 320, "height": 240, "background": "white"})).unwrap();
        }
        let services = Services { cursor_pos: Some(Box::new(move |_: &egui::Context| Some(reported.get()))), ..Default::default() };
        let mut app = PhotocraftApp::new(s, services);
        app.background_jobs = true;
        app
    });
    h.run_steps(4);
    let strip = h.state().tab_strip.clone().expect("the tab strip is drawn");
    assert_eq!(strip.tabs.len(), 2);
    // Hovering over the first tab's left half: slot 0.
    let over_first = egui::pos2(strip.tabs[0].1.left() + 4.0, strip.tabs[0].1.center().y);
    pointer.set(over_first);
    h.input_mut().hovered_files.push(egui::HoveredFile { path: Some(file.clone()), ..Default::default() });
    h.step();
    assert_eq!(h.state().drop_target(&h.ctx, Some(over_first)), crate::file_open::DropTarget::Tabs(0));
    // Dropped there: opens in the background, then takes the first tab.
    h.input_mut().hovered_files.clear();
    h.input_mut().dropped_files.push(dropped(file.clone(), Err("read from the path".into())));
    h.step();
    let t = std::time::Instant::now();
    while !h.state().jobs.opens.is_empty() && t.elapsed() < std::time::Duration::from_secs(30) {
        h.step();
    }
    h.run_steps(2);
    let names = doc_names(h.state());
    assert_eq!(names.len(), 3);
    assert_eq!(names[0], "photo.png", "{names:?}");
    assert_eq!(h.state().session.active_index(), Some(0));
    // Dropped on the canvas: placed into the active document, in Free Transform.
    let layers = h.state().session.active().unwrap().doc.layers.len();
    pointer.set(h.state().drop_canvas_rect.expect("the canvas is shown").center());
    h.input_mut().dropped_files.push(dropped(file, Err("read from the path".into())));
    h.run_steps(2);
    // Placement runs on a worker; frame count does not imply completion.
    let started = std::time::Instant::now();
    while h.state().jobs.place.is_some() && started.elapsed() < std::time::Duration::from_secs(30) {
        h.step();
    }
    assert!(h.state().jobs.place.is_none(), "placement completed");
    assert_eq!(h.state().session.documents().len(), 3, "no new document");
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers + 1);
    assert_eq!(h.state().ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.transform.is_none());
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), layers, "Esc takes the place back");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn views_and_windows_stay_with_their_documents() {
    let (mut app, _) = app_with(Vec::new());
    for name in ["a.psd", "b.psd", "c.psd", "d.psd"] {
        app.open_bytes(name, b"x").unwrap();
    }
    for (i, v) in app.ui.views.iter_mut().enumerate() {
        v.zoom = i as f32 + 1.0;
    }
    app.ui.windows = (0..4)
        .map(|document| crate::state::DocWindow {
            id: document as u64,
            document,
            view: Default::default(),
            open: true,
            floating: false,
            focused: false,
            position: None,
        })
        .collect();
    let zooms = |app: &PhotocraftApp| app.ui.views.iter().map(|v| v.zoom).collect::<Vec<_>>();
    let windows = |app: &PhotocraftApp| app.ui.windows.iter().map(|w| (w.id, w.document)).collect::<Vec<_>>();
    assert_eq!(app.run("document.move", json!({"document": 0, "to": 3})).unwrap(), json!({"document": 3}));
    assert_eq!(doc_names(&app), ["b.psd", "c.psd", "d.psd", "a.psd"]);
    assert_eq!(zooms(&app), [2.0, 3.0, 4.0, 1.0]);
    assert_eq!(windows(&app), [(0, 3), (1, 0), (2, 1), (3, 2)]);
    // Out of range: an error, nothing moves.
    assert!(app.run("document.move", json!({"document": 7, "to": 0})).is_err());
    assert_eq!(zooms(&app), [2.0, 3.0, 4.0, 1.0]);
    // Closing a middle tab takes its view and windows with it; the others keep theirs.
    app.run("file.close", json!({"document": 1})).unwrap();
    assert_eq!(doc_names(&app), ["b.psd", "d.psd", "a.psd"]);
    assert_eq!(zooms(&app), [2.0, 4.0, 1.0]);
    assert_eq!(windows(&app), [(0, 2), (1, 0), (3, 1)]);
}

#[test]
fn files_dropped_off_the_canvas_open_as_documents() {
    let other = || vec![dropped("other.png", Ok(b"x".to_vec()))];
    let ctx = egui::Context::default();
    // On the tab bar or a panel (outside the canvas), or where the pointer can't be read.
    for at in [Some(egui::pos2(400.0, 30.0)), Some(egui::pos2(750.0, 300.0)), None] {
        let mut app = app_with_canvas();
        let before = layer_count(&app);
        app.open_dropped(&ctx, other(), at);
        assert_eq!(app.session.documents().len(), 2, "{at:?}");
        assert_eq!(app.session.documents()[0].doc.layers.len(), before, "{at:?}");
    }
    // Over a window floating above the canvas (shown from its second frame, once sized).
    for _ in 0..2 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            egui::Window::new("panel").fixed_pos(egui::pos2(300.0, 200.0)).show(ui.ctx(), |ui| ui.label("over the canvas"));
        });
        out.textures_delta.clear();
    }
    let mut app = app_with_canvas();
    app.open_dropped(&ctx, other(), Some(egui::pos2(320.0, 220.0)));
    assert_eq!(app.session.documents().len(), 2);
    // The start screen or an opening file's card (no canvas shown) opens too.
    let mut app = app_with_canvas();
    app.drop_canvas_rect = None;
    app.open_dropped(&ctx, other(), Some(egui::pos2(400.0, 300.0)));
    assert_eq!(app.session.documents().len(), 2);
}

#[test]
fn notices_are_capped_and_dismissable_state_round_trips() {
    let (mut app, _) = app_with(Vec::new());
    for i in 0..10 {
        notices::post(&mut app, format!("n{i}"), vec![], false, None);
    }
    assert_eq!(app.ui.notices.len(), notices::MAX_NOTICES);
    assert_eq!(app.ui.notices.last().map(|n| n.title.as_str()), Some("n9"));
    // Notices are part of the serialized UI state (ui.inspect / ui.set).
    let v = serde_json::to_value(&app.ui).unwrap();
    assert_eq!(v["notices"].as_array().map(Vec::len), Some(notices::MAX_NOTICES));
    let back: crate::UiState = serde_json::from_value(v).unwrap();
    assert_eq!(back.notices, app.ui.notices);
}

#[test]
fn notices_render_without_panicking() {
    let (mut app, _) = app_with(Vec::new());
    notices::post(&mut app, "Opened a.psd with 9 warnings", (0..9).map(|i| format!("warning {i}")).collect(), false, None);
    notices::error(&mut app, "Couldn't open b.psd: not an image".into());
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| notices::show(&mut app, ui.ctx()));
    out.textures_delta.clear();
    assert_eq!(app.ui.notices.len(), 2);
}

#[test]
fn open_documents_as_tabs_preference_controls_floating_windows() {
    let (mut app, _) = app_with(Vec::new());
    app.session.edit_prefs(|p| p.workspace.open_documents_as_tabs = false);
    app.open_bytes("a.psd", b"x").unwrap();
    assert_eq!(app.ui.windows.len(), 1);
    assert_eq!(app.ui.windows[0].document, 0);

    app.run("file.new", json!({"width": 100, "height": 100})).unwrap();
    assert_eq!(app.ui.windows.len(), 2);
    assert_eq!(app.ui.windows[1].document, 1);

    // Opening with preference true docks into tabs, creating no new floating window
    app.session.edit_prefs(|p| p.workspace.open_documents_as_tabs = true);
    app.open_bytes("b.psd", b"x").unwrap();
    assert_eq!(app.ui.windows.len(), 2);
}

#[test]
fn natural_order_counts_numbers_by_value() {
    use std::cmp::Ordering;
    assert_eq!(natural_cmp("File_2.psd", "File_10.psd"), Ordering::Less);
    assert_eq!(natural_cmp("file_1.psd", "File_2.psd"), Ordering::Less, "case-insensitive");
    assert_eq!(natural_cmp("File_03.psd", "File_10.psd"), Ordering::Less, "leading zeros");
    assert_eq!(natural_cmp("a.psd", "a.psd"), Ordering::Equal);
    assert_eq!(natural_cmp("a", "ab"), Ordering::Less);
    assert_eq!(natural_cmp("File_02.psd", "File_2.psd").then(Ordering::Equal), "File_02.psd".cmp("File_2.psd"), "equal by value: by text");
    let mut names = vec!["File_10.psd", "File_9.psd", "file_1.psd", "File_100.psd", "b.psd", "A.psd", "File_03.psd"];
    names.sort_by(|a, b| natural_cmp(a, b));
    assert_eq!(names, ["A.psd", "b.psd", "file_1.psd", "File_03.psd", "File_9.psd", "File_10.psd", "File_100.psd"]);
}

/// Files on the command line open before the first frame applies the interface language: their
/// status line follows the preference, not the English default.
#[test]
fn open_paths_reports_in_the_interface_language_before_the_first_frame() {
    if !crate::i18n::needs_catalogs() {
        return;
    }
    let pattern = |i: u8| {
        let mut surface = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
        surface.fill_rect(photocraft_geom::Rect::new(0, 0, 4, 4), &[f32::from(i) / 3.0, 0.5, 0.2, 1.0]);
        let mut p = photocraft_doc::Pattern::new("Launch", surface, 4, 4);
        p.id = format!("6f1c2c1e-0000-4000-8000-00000000000{i}");
        p
    };
    let bytes = photocraft_io::pattern_map::write_pat(&[pattern(1), pattern(2), pattern(3)]).unwrap();
    let (dir, paths) = files("launch-language", &["launch.pat"], &bytes);
    let (mut app, _) = app_with(Vec::new());
    app.session.edit_prefs(|p| p.interface.language = "ar".into());
    crate::i18n::set_current(crate::i18n::Lang::EN);
    assert_eq!(app.open_paths(&paths), 1);
    let ar = crate::i18n::Lang::from_code("ar").unwrap();
    let want =
        crate::i18n::fmt(&crate::i18n::trn(ar, 3, "Imported {n} pattern from {file}", "Imported {n} patterns from {file}"), &[("n", "3"), ("file", "launch")]);
    assert_eq!(app.ui.status, want);
    crate::i18n::set_current(crate::i18n::Lang::EN);
    let _ = std::fs::remove_dir_all(&dir);
}

/// #3248: several files opened at once (File › Open, the command line, Finder) open in name order.
#[test]
fn open_paths_opens_a_set_in_name_order() {
    let (dir, paths) = files("sorted", &["File_10.psd", "File_2.psd", "file_1.psd", "File_03.psd"], b"x");
    let (mut app, _) = app_with(Vec::new());
    assert_eq!(app.open_paths(&paths), 4);
    assert_eq!(doc_names(&app), ["file_1.psd", "File_2.psd", "File_03.psd", "File_10.psd"]);
    assert_eq!(app.session.active().and_then(|s| s.path.clone()).as_deref(), Some(paths[0].as_str()), "the last in order is active");
    assert!(app.jobs.batches.is_empty(), "nothing left opening");
    // The next set goes after.
    let (dir2, more) = files("sorted-2", &["b.psd", "a.psd"], b"x");
    assert_eq!(app.open_paths(&more), 2);
    assert_eq!(doc_names(&app), ["file_1.psd", "File_2.psd", "File_03.psd", "File_10.psd", "a.psd", "b.psd"]);
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

/// #3248: a set opening in the background keeps its order however the opens finish (a small file
/// decodes before a big one named before it).
#[test]
fn a_set_of_background_opens_keeps_its_order_however_they_finish() {
    use crate::jobs_ui::{self, OpenOrder, OpenTab};
    use photocraft_engine::jobs::JobId;
    let (mut app, _) = app_with(Vec::new());
    app.open_bytes("existing.psd", b"x").unwrap();
    let batch = jobs_ui::begin_batch(&mut app);
    let tab = |seq: usize| OpenTab { job: JobId(100 + seq as u64), name: format!("f{seq}.psd"), path: None, slot: None, order: Some(OpenOrder { batch, seq }) };
    app.jobs.opens.extend((0..4).map(tab));
    // f1 opened at once (a preset file, say, or background jobs off for it).
    app.jobs.opens.remove(1);
    let index = app.session.add_document(Document::new("f1.psd", Size::new(4, 4), ColorMode::Rgb, SampleType::U8), None);
    assert_eq!(index, 1);
    jobs_ui::opened_in_order(&mut app.jobs, OpenOrder { batch, seq: 1 });
    jobs_ui::end_batch(&mut app);
    assert_eq!(app.jobs.batches.len(), 1, "kept while members are opening");
    // The rest finish out of order: last, first, third.
    for seq in [3usize, 0, 2] {
        let i = app.jobs.opens.iter().position(|o| o.order.is_some_and(|x| x.seq == seq)).unwrap();
        let tab = app.jobs.opens.remove(i);
        let index = app.session.add_document(Document::new(&tab.name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), None);
        let id = app.session.documents()[index].doc.id;
        jobs_ui::finish_open_tab(&mut app, &tab, &json!({"document": index, "docId": id.0})).unwrap();
    }
    assert_eq!(doc_names(&app), ["existing.psd", "f0.psd", "f1.psd", "f2.psd", "f3.psd"]);
    assert_eq!(app.session.active().map(|d| d.doc.name.as_str()), Some("f2.psd"), "the one that just opened is active");
    assert_eq!(app.ui.views.len(), 5, "views follow their documents");
    assert!(app.jobs.batches.is_empty(), "forgotten once every member opened");
    // A tab with a slot (dropped on the tabs) keeps it; one from a forgotten set goes to the end.
    let dropped = OpenTab { job: JobId(200), name: "dropped.psd".into(), path: None, slot: Some(0), order: None };
    let index = app.session.add_document(Document::new("dropped.psd", Size::new(4, 4), ColorMode::Rgb, SampleType::U8), None);
    let id = app.session.documents()[index].doc.id;
    jobs_ui::finish_open_tab(&mut app, &dropped, &json!({"document": index, "docId": id.0})).unwrap();
    let late = OpenTab { job: JobId(201), name: "late.psd".into(), path: None, slot: None, order: Some(OpenOrder { batch: 999, seq: 0 }) };
    let index = app.session.add_document(Document::new("late.psd", Size::new(4, 4), ColorMode::Rgb, SampleType::U8), None);
    let id = app.session.documents()[index].doc.id;
    jobs_ui::finish_open_tab(&mut app, &late, &json!({"document": index, "docId": id.0})).unwrap();
    assert_eq!(doc_names(&app), ["dropped.psd", "existing.psd", "f0.psd", "f1.psd", "f2.psd", "f3.psd", "late.psd"]);
}

/// #4373: Save a Copy is recorded in an action, like Save and Save As, and played back as a copy:
/// the document keeps its own file and unsaved changes.
#[test]
fn save_a_copy_is_recorded_and_replayed_as_a_copy() {
    let (mut app, written) = app_with(vec![Some(FileDialogAnswer::SaveTo("/pics/copy.png".into()))]);
    let ctx = egui::Context::default();
    app.open_file("/pics/work.pcraft", b"x").unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("actions.record", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    answer(&mut app);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[0].steps.last(), Some(&("file.saveACopy".to_string(), json!({"path": "/pics/copy.png"}))));
    assert_eq!(crate::actions::label_of("file.saveACopy"), "Save a Copy");
    written.borrow_mut().clear();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(written.borrow().iter().map(|(p, _)| p.as_str()).collect::<Vec<_>>(), ["/pics/copy.png"]);
    let st = app.session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/pics/work.pcraft"), "the document keeps its file");
    assert!(st.is_dirty(), "and its unsaved changes");
}

/// #7207: File › Export › Export As is recorded in an action with its destination and settings,
/// as Save a Copy is, and played back without the dialog: on another document it exports that
/// document, with the recorded format, quality and scale.
#[test]
fn export_as_is_recorded_and_replayed() {
    type Exported = Rc<RefCell<Vec<(String, u32, u32, Option<u8>)>>>;
    let exported: Exported = Rc::default();
    let e = exported.clone();
    let services = Services {
        export: Some(Box::new(move |doc: &Document, path: &str, s: &crate::ExportSettings| {
            e.borrow_mut().push((path.to_string(), doc.size.width, doc.size.height, s.jpeg_quality));
            Ok((b"out".to_vec(), Vec::new()))
        })),
        file_dialog: Some(file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo("/pics/out.jpg".into()))]).0),
        write: Some(Box::new(|_: &str, _: &[u8]| Ok(()))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 40, "height": 20})).unwrap();
    app.run("actions.record", json!({})).unwrap();
    let id = menus::invoke(&mut app, &ctx, "file.export.exportAs", json!({})).unwrap()["dialog"].as_u64().unwrap();
    let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
    fields.insert("format".into(), json!("jpg"));
    fields.insert("quality".into(), json!(70));
    fields.insert("scale".into(), json!(50));
    crate::dialogs::confirm(&mut app, id).unwrap();
    answer(&mut app);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(*exported.borrow(), [("/pics/out.jpg".to_string(), 20, 10, Some(70))]);
    let (step_id, step) = app.session.actions.list[0].steps.last().cloned().unwrap();
    assert_eq!(step_id, "file.export.exportAs");
    assert_eq!((&step["path"], &step["format"], &step["quality"], &step["scale"]), (&json!("/pics/out.jpg"), &json!("jpg"), &json!(70), &json!(50)));
    assert!(step.as_object().unwrap().keys().all(|k| !k.starts_with("__")), "only the destination and the dialog's settings: {step}");
    assert_eq!(crate::actions::label_of("file.export.exportAs"), "Export As");

    app.run("file.new", json!({"width": 100, "height": 60})).unwrap();
    exported.borrow_mut().clear();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert_eq!(*exported.borrow(), [("/pics/out.jpg".to_string(), 50, 30, Some(70))], "the played document, scaled as recorded");
    assert!(app.ui.dialogs.is_empty(), "no dialog on playback");
}

/// #4400: a new document saved under a name is listed in Open Recent (and on the Home screen),
/// as opening it is; a Save a Copy isn't, since the document doesn't live there.
#[test]
fn saving_a_new_document_lists_it_in_recent_files() {
    let (mut app, _written) = app_with(vec![Some(FileDialogAnswer::SaveTo("/pics/new.psd".into())), Some(FileDialogAnswer::SaveTo("/pics/copy.png".into()))]);
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    assert!(app.ui.recent_files.is_empty());
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    answer(&mut app);
    assert_eq!(app.ui.recent_files, ["/pics/new.psd"]);
    assert_eq!(app.session.prefs().file_handling.recent_files, ["/pics/new.psd"], "kept for the next launch");
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    answer(&mut app);
    assert_eq!(app.ui.recent_files, ["/pics/new.psd"], "a copy isn't the document's file");
}

/// #4559: a file whose type uses a font that isn't installed opens with a notice naming it, as
/// Photoshop warns on open (the CLI and automation already did); the file still opens. Type in
/// an installed face adds nothing, and a font the importer already warned about isn't repeated.
#[test]
fn opening_type_in_a_missing_font_warns() {
    fn doc_with_type(font: Option<&str>) -> Document {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 40, "height": 20})).unwrap();
        let mut p = json!({"text": "Hi", "x": 2, "y": 12, "size": 10});
        if let Some(f) = font {
            p["font"] = json!(f);
        }
        s.execute("type.create", p).unwrap();
        (*s.active().unwrap().doc).clone()
    }
    const MISSING: &str = "NoSuchFamily4559";
    let warning = format!("font '{MISSING}' is not installed; text may render using a fallback face");
    let (with_missing, installed) = (doc_with_type(Some(MISSING)), doc_with_type(None));
    let already = warning.clone();
    let services = Services {
        import: Some(Box::new(move |name: &str, _bytes: &[u8], _opts: &photocraft_io::ImportOptions| {
            Ok(match name {
                "installed.psd" => (installed.clone(), Vec::new()),
                "warned.psd" => (with_missing.clone(), vec![already.clone()]),
                _ => (with_missing.clone(), Vec::new()),
            })
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    let warnings = app.open_bytes("poster.psd", b"x").unwrap();
    assert_eq!(warnings, vec![warning.clone()]);
    assert!(app.ui.status.contains(MISSING), "{}", app.ui.status);
    assert_eq!(app.ui.notices.len(), 1);
    assert_eq!(app.ui.notices[0].lines, vec![warning.clone()]);
    assert_eq!(app.session.documents().len(), 1, "the file still opens");
    // Type in an installed face: no notice.
    assert!(app.open_bytes("installed.psd", b"x").unwrap().is_empty());
    assert_eq!(app.ui.notices.len(), 1);
    // The importer's own warning for the same font isn't doubled.
    assert_eq!(app.open_bytes("warned.psd", b"x").unwrap(), vec![warning]);
}

/// A file name that isn't valid Unicode on this platform: non-UTF-8 bytes on Unix, an unpaired
/// surrogate on Windows.
fn non_unicode_path() -> std::path::PathBuf {
    #[cfg(unix)]
    let name = {
        use std::os::unix::ffi::OsStrExt;
        std::ffi::OsStr::from_bytes(b"img\xFF.png").to_owned()
    };
    #[cfg(windows)]
    let name = {
        use std::os::windows::ffi::OsStringExt;
        let mut wide: Vec<u16> = "img".encode_utf16().collect();
        wide.push(0xD800);
        wide.extend(".png".encode_utf16());
        std::ffi::OsString::from_wide(&wide)
    };
    std::env::temp_dir().join(name)
}

/// #6601: a lossy copy of such a name is another file's name, so it is refused instead.
#[test]
fn a_path_that_is_not_valid_unicode_is_refused_not_rewritten() {
    let bad = non_unicode_path();
    let open = path_text(&bad, false).unwrap_err();
    assert!(open.contains("not valid Unicode") && open.contains("open it again"), "{open}");
    let save = path_text(&bad, true).unwrap_err();
    assert!(save.contains("not valid Unicode") && save.contains("another name"), "{save}");
    let good = std::env::temp_dir().join("img é.png");
    assert_eq!(path_text(&good, true).as_deref(), Ok(good.to_str().unwrap()));
}

/// #6598: opening a file that is already open shows its document, as Photoshop does, rather than
/// a second copy bound to the same file (saving one would overwrite the other). The path is
/// compared as the file it names: another spelling of it, a symlink to it or, with background
/// opens, a file still opening finds the same tab.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn opening_an_open_file_again_shows_its_tab() {
    let (dir, paths) = files("already-open", &["a.psd", "b.psd"], b"x");
    let (a, b) = (&paths[0], &paths[1]);
    let (mut app, _) = app_with(Vec::new());
    app.open_path(a).unwrap();
    app.open_path(b).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.ui.chrome.home = Some(2);
    // Open Recent, File › Open, a drop: all go through `open_path`.
    #[allow(unused_mut)]
    let mut spellings = vec![a.clone(), dir.join(".").join("a.psd").to_string_lossy().into_owned()];
    #[cfg(unix)]
    {
        let link = dir.join("link.psd");
        std::os::unix::fs::symlink(a, &link).unwrap();
        spellings.push(link.to_string_lossy().into_owned());
    }
    for again in spellings {
        app.session.set_active(1);
        app.ui.chrome.home = Some(2);
        app.open_path(&again).unwrap();
        assert_eq!(doc_names(&app), ["a.psd", "b.psd"], "{again}");
        assert_eq!(app.session.active_index(), Some(0), "{again}");
        assert_eq!(app.ui.chrome.home, None, "the Home screen gives way to the document");
    }
    assert!(app.session.documents()[1].is_dirty(), "b keeps its unsaved changes");
    // A set holding an open file opens only the others.
    let (dir2, more) = files("already-open-set", &["c.psd"], b"x");
    assert_eq!(app.open_paths(&[b.clone(), more[0].clone()]), 1);
    assert_eq!(doc_names(&app), ["a.psd", "b.psd", "c.psd"]);
    // In the background: an open document, then a file still opening, show their tabs.
    app.background_jobs = true;
    app.open_path(a).unwrap();
    assert!(app.jobs.opens.is_empty());
    assert_eq!(app.session.active_index(), Some(0));
    let (dir3, late) = files("already-open-bg", &["d.psd"], b"x");
    app.open_path(&late[0]).unwrap();
    let job = app.jobs.opens[0].job;
    app.jobs.focus = None;
    app.open_path(&late[0]).unwrap();
    assert_eq!(app.jobs.opens.len(), 1, "one tab for the file still opening");
    assert_eq!(app.jobs.focus, Some(job));
    // A template still loading opens again as another Untitled, as it does once loaded.
    let (dir4, template) = files("already-open-template", &["e.psdt"], b"x");
    app.open_path(&template[0]).unwrap();
    app.open_path(&template[0]).unwrap();
    assert_eq!(app.jobs.opens.len(), 3, "two Untitleds from the template");
    for d in [dir, dir2, dir3, dir4] {
        let _ = std::fs::remove_dir_all(&d);
    }
}

// ---- Where a drop goes (#10492) ----

/// A tab strip of three 100-wide tabs at y 14-50 over x 0-700, and a canvas below it.
fn strip() -> crate::canvas::TabStrip {
    let tab = |i: usize| (i, egui::Rect::from_min_max(egui::pos2(104.0 * i as f32, 20.0), egui::pos2(104.0 * i as f32 + 100.0, 46.0)));
    crate::canvas::TabStrip { rect: egui::Rect::from_min_max(egui::pos2(0.0, 14.0), egui::pos2(700.0, 50.0)), tabs: vec![tab(0), tab(1), tab(2)] }
}

fn canvas() -> Option<egui::Rect> {
    Some(egui::Rect::from_min_max(egui::pos2(100.0, 50.0), egui::pos2(700.0, 550.0)))
}

#[test]
fn a_drop_lands_on_the_region_under_the_pointer() {
    let strip = strip();
    let on = |at, uncovered, tabs, canvas, doc| target_at(at, uncovered, tabs, canvas, doc);
    // Over the canvas of an open document: placed.
    assert_eq!(on(Some(egui::pos2(300.0, 300.0)), true, Some(&strip), canvas(), true), DropTarget::Canvas);
    // Over the tab strip: that tab position (the strip wins where it overlaps the canvas).
    assert_eq!(on(Some(egui::pos2(4.0, 30.0)), true, Some(&strip), canvas(), true), DropTarget::Tabs(0));
    assert_eq!(on(Some(egui::pos2(150.0, 30.0)), true, Some(&strip), canvas(), true), DropTarget::Tabs(1));
    assert_eq!(on(Some(egui::pos2(690.0, 30.0)), true, Some(&strip), canvas(), true), DropTarget::Tabs(3));
    // Over a panel, outside every region.
    assert_eq!(on(Some(egui::pos2(750.0, 300.0)), true, Some(&strip), canvas(), true), DropTarget::Elsewhere);
    // The pointer can't be read, or a window or dialog covers it: open, never place.
    assert_eq!(on(None, true, Some(&strip), canvas(), true), DropTarget::Elsewhere);
    assert_eq!(on(Some(egui::pos2(300.0, 300.0)), false, Some(&strip), canvas(), true), DropTarget::Elsewhere);
    assert_eq!(on(Some(egui::pos2(4.0, 30.0)), false, Some(&strip), canvas(), true), DropTarget::Elsewhere);
    // No document: the Home screen or an empty workspace. A canvas rectangle left from a document
    // closed since doesn't make the drop a placement with nothing to place into.
    assert_eq!(on(Some(egui::pos2(300.0, 300.0)), true, None, None, false), DropTarget::Elsewhere);
    assert_eq!(on(Some(egui::pos2(300.0, 300.0)), true, None, canvas(), false), DropTarget::Elsewhere);
}

#[test]
fn only_a_canvas_drop_of_a_non_preset_file_is_placed() {
    let files =
        ["photo.png", "photo.JPG", "layers.psd", "big.psb", "scan.tif", "shot.heic", "notes.txt", "no_extension", "", "C:\\pics\\a.webp", "/pics/b.svg"];
    let presets = ["brushes.abr", "Sunsets.GRD", "/lib/set.abr", "C:\\lib\\x.grd"];
    for target in [DropTarget::Canvas, DropTarget::Tabs(0), DropTarget::Tabs(7), DropTarget::Elsewhere] {
        for name in files {
            let want = if target == DropTarget::Canvas { DropRoute::Place } else { DropRoute::Open };
            assert_eq!(target.route(name), want, "{target:?} {name:?}");
        }
        for name in presets {
            assert_eq!(target.route(name), DropRoute::Open, "presets go to their libraries: {target:?} {name:?}");
        }
    }
}

/// A scratch folder, removed by the test.
fn scratch(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("photocraft-drop-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn error_titles(app: &PhotocraftApp) -> Vec<String> {
    app.ui.notices.iter().filter(|n| n.error).map(|n| n.title.clone()).collect()
}

/// The Home screen (no document open) is where a first drop lands: every file opens as its own
/// document, in drop order, whatever the type, and nothing is placed.
#[test]
fn files_dropped_with_no_document_open_all_open() {
    let dir = scratch("home");
    let names = ["a.png", "b.jpg", "c.psd", "d.tif"];
    let files: Vec<_> = names
        .iter()
        .map(|n| {
            let p = dir.join(n);
            std::fs::write(&p, b"x").unwrap();
            dropped(p, Err("read from the path".into()))
        })
        .collect();
    let (mut app, _) = app_with(Vec::new());
    // A canvas rectangle left from an earlier document must not turn the drop into a placement.
    app.drop_canvas_rect = canvas();
    assert_eq!(app.drop_target(&egui::Context::default(), Some(egui::pos2(300.0, 300.0))), DropTarget::Elsewhere);
    app.open_dropped(&egui::Context::default(), files, Some(egui::pos2(300.0, 300.0)));
    assert_eq!(doc_names(&app), names);
    assert!(app.drop_places.is_empty());
    assert!(error_titles(&app).is_empty(), "{:?}", app.ui.notices);
    let _ = std::fs::remove_dir_all(&dir);
}

/// A folder dragged in says it is a folder (Windows' own error for reading one is "Access is
/// denied"), opens nothing and places nothing, and the files dropped with it still go through.
#[test]
fn a_dropped_folder_is_refused_with_a_clear_message() {
    let dir = scratch("folder");
    let folder = dir.join("Holiday photos");
    std::fs::create_dir_all(&folder).unwrap();
    let file = dir.join("ok.psd");
    std::fs::write(&file, b"x").unwrap();
    let ctx = egui::Context::default();
    // Opened (no document, or off the canvas).
    let (mut app, _) = app_with(Vec::new());
    app.open_dropped(&ctx, vec![dropped(folder.clone(), Err("read from the path".into())), dropped(file, Err("read from the path".into()))], None);
    assert_eq!(doc_names(&app), ["ok.psd"]);
    let errors = error_titles(&app);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("Couldn't open Holiday photos: ") && errors[0].contains("folder"), "{errors:?}");
    // On the canvas: not queued for placing, and the layers stay as they were.
    let mut app = app_with_canvas();
    let layers = layer_count(&app);
    app.open_dropped(&ctx, vec![dropped(folder, Err("read from the path".into()))], Some(egui::pos2(300.0, 300.0)));
    assert!(app.drop_places.is_empty());
    app.place_next_dropped(&ctx);
    assert_eq!(layer_count(&app), layers);
    assert_eq!(app.session.documents().len(), 1);
    let errors = error_titles(&app);
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(errors[0].starts_with("Couldn't place Holiday photos: ") && errors[0].contains("folder"), "{errors:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A file that isn't an image is an error the user can read, on either route, not a silent no-op.
#[test]
fn a_dropped_file_that_is_not_an_image_reports_why() {
    let dir = scratch("text");
    let file = dir.join("notes.txt");
    std::fs::write(&file, b"bad").unwrap();
    let ctx = egui::Context::default();
    let (mut app, _) = app_with(Vec::new());
    app.open_dropped(&ctx, vec![dropped(file.clone(), Err("read from the path".into()))], None);
    assert!(app.session.documents().is_empty());
    assert!(app.ui.status_error);
    assert!(error_titles(&app).iter().any(|t| t.starts_with("Couldn't open notes.txt: ")), "{:?}", app.ui.notices);
    let mut app = app_with_canvas();
    let layers = layer_count(&app);
    app.open_dropped(&ctx, vec![dropped(file, Err("read from the path".into()))], Some(egui::pos2(300.0, 300.0)));
    app.place_next_dropped(&ctx);
    assert_eq!(layer_count(&app), layers);
    assert!(app.ui.status_error);
    assert!(error_titles(&app).iter().any(|t| t.starts_with("Couldn't place notes.txt: ")), "{:?}", app.ui.notices);
    assert!(app.ui.transform.is_none(), "a failed place starts no Free Transform");
    let _ = std::fs::remove_dir_all(&dir);
}

/// Several files dropped on the canvas together are all queued, in order; a preset file among
/// them goes to its library instead.
#[test]
fn several_files_dropped_on_the_canvas_are_queued_in_order() {
    let ctx = egui::Context::default();
    let mut app = app_with_canvas();
    app.open_dropped(
        &ctx,
        vec![dropped("/pics/one.png", Err("unused".into())), dropped("/pics/set.abr", Err("unused".into())), dropped("/pics/two.jpg", Err("unused".into()))],
        Some(egui::pos2(300.0, 300.0)),
    );
    let queued: Vec<String> = app
        .drop_places
        .iter()
        .map(|(_, p)| match p {
            DropPlace::At { source, .. } => match source.as_ref() {
                DropPlace::File(f) => dropped_name(f),
                _ => panic!("expected a native file"),
            },
            _ => panic!("expected a drop with a captured destination"),
        })
        .collect();
    assert_eq!(queued, ["one.png", "two.jpg"]);
    assert!(error_titles(&app).iter().any(|t| t.starts_with("Couldn't open set.abr: ")), "{:?}", app.ui.notices);
}

#[test]
fn queued_drop_waits_for_an_external_document_job() {
    let mut app = app_with_canvas();
    let doc = app.session.active().unwrap().doc.id;
    app.session.start_job("test.external", json!({}), "External job", true, |_| Ok(()), |_, ()| Ok(json!(null))).unwrap();
    let job = app.session.active_job().unwrap().id;
    app.drop_places.push_back((doc, crate::file_open::DropPlace::Bytes("queued.png".into(), png_bytes(4, 3))));
    app.place_next_dropped(&egui::Context::default());
    assert_eq!(app.drop_places.len(), 1, "busy document retains its drop");
    assert!(app.jobs.place.is_none());
    let layers = layer_count(&app);
    app.session.wait_job(job).unwrap();
    app.place_next_dropped(&egui::Context::default());
    assert_eq!(layer_count(&app), layers + 1);
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    assert!(app.drop_places.is_empty());
}

/// File › Open as Smart Object… (#11678) asks with File › Open's dialog and opens each chosen
/// file as its own untitled document holding one embedded smart object.
#[test]
fn open_as_smart_object_dialog_opens_one_document_per_selected_file() {
    let (dir, paths) = files("open-as-so", &["one.png", "two.png", "bad.png"], &png_bytes(8, 6));
    std::fs::write(&paths[2], b"not an image").unwrap();
    let (mut app, _) = app_with(vec![Some(FileDialogAnswer::Paths(paths.clone()))]);

    menus::invoke(&mut app, &egui::Context::default(), "file.openAsSmartObject", json!({})).unwrap();
    assert!(app.file_dialog_open(), "the menu item asks for files first");
    answer(&mut app);

    assert_eq!(app.session.documents().iter().map(|d| d.doc.name.as_str()).collect::<Vec<_>>(), ["one-1", "two-1"]);
    assert!(app.session.documents().iter().all(|d| d.path.is_none() && d.doc.layers.len() == 1));
    assert!(app.ui.notices.iter().any(|n| n.error && n.title.starts_with("Couldn't open bad.png")), "the unreadable file is reported");
    let _ = std::fs::remove_dir_all(&dir);
}

/// File > Open as Smart Object... runs each file as a tracked background job like File > Open
/// does: a tab per file, the documents landing in name order, untitled, failures reported.
#[test]
fn open_as_smart_object_opens_in_the_background_in_name_order() {
    let ctx = egui::Context::default();
    let (dir, paths) = files("open-as-so-bg", &["two.png", "one.png", "bad.png"], &png_bytes(8, 6));
    std::fs::write(&paths[2], b"not an image").unwrap();
    let (mut app, _) = app_with(vec![Some(FileDialogAnswer::Paths(paths.clone()))]);
    app.background_jobs = true;

    menus::invoke(&mut app, &ctx, "file.openAsSmartObject", json!({})).unwrap();
    answer(&mut app);
    assert_eq!(app.jobs.opens.len(), 3, "a tab for each file while it opens");
    assert!(app.session.documents().is_empty(), "nothing is on the UI thread's critical path");

    tick_until(&mut app, &ctx, |app| app.jobs.opens.is_empty());
    assert_eq!(app.session.documents().iter().map(|d| d.doc.name.as_str()).collect::<Vec<_>>(), ["one-1", "two-1"]);
    assert!(app.session.documents().iter().all(|d| d.path.is_none() && d.doc.layers.len() == 1));
    assert!(app.ui.notices.iter().any(|n| n.error && n.title.starts_with("Couldn't open bad.png")), "the unreadable file is reported");
    let _ = std::fs::remove_dir_all(&dir);
}
