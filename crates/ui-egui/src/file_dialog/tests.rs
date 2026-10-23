use super::*;
use crate::{Services, menus};
use std::cell::RefCell;
use std::rc::Rc;

/// The requests the fake dialog service was given, with their replies: dialogs the user is still in.
type Open = Rc<RefCell<Vec<(FileDialogRequest, FileDialogReply)>>>;

/// An app with an untitled document, whose file dialogs stay open until the test answers them, and
/// whose writer records the paths it wrote.
fn app() -> (PhotocraftApp, Open, Rc<RefCell<Vec<String>>>) {
    let (open, written): (Open, Rc<RefCell<Vec<String>>>) = Default::default();
    let (held, w) = (open.clone(), written.clone());
    let services = Services {
        export: Some(Box::new(|_: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| Ok((b"out".to_vec(), Vec::new())))),
        write: Some(Box::new(move |path: &str, _: &[u8]| {
            w.borrow_mut().push(path.to_string());
            Ok(())
        })),
        file_dialog: Some(Box::new(move |request, _parent, reply| held.borrow_mut().push((request, reply)))),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    (app, open, written)
}

#[test]
fn pdn_save_requests_a_native_copy_without_overwriting_the_source() {
    let (mut app, open, written) = app();
    let st = app.session.active_mut().unwrap();
    st.path = Some("/pics/layers.PDN".into());
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "/pics/layers.pcraft"));
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/layers.pcraft".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/layers.pcraft"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/layers.pcraft"));
}

/// A real flat JPEG import, optionally edited to contain another layer. A filename without a
/// path is the web/download case; desktop opens also retain their source location.
fn jpeg_app(path: Option<&str>, layered: bool) -> (PhotocraftApp, Open, Rc<RefCell<Vec<String>>>) {
    let (mut app, open, written) = app();
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "My image.JPEG", &Default::default()).unwrap().bytes;
    let doc = photocraft_io::import("My image.JPEG", &bytes).unwrap().document;
    app.session.add_document(doc, path.map(str::to_string));
    if layered {
        app.run("layer.new.layer", json!({})).unwrap();
    }
    (app, open, written)
}

#[test]
fn layered_flat_source_save_defaults_to_psd_without_overwriting_source() {
    let ctx = egui::Context::default();
    for command in ["file.save", "file.saveAs"] {
        let (mut app, open, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
        menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(
            matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "/pics/My image.psd"),
            "{command}: {:?}",
            open.borrow().first().map(|(r, _)| r.clone())
        );
        assert!(written.borrow().is_empty(), "asking where to save must not overwrite the JPEG");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/My image.JPEG"));
        answer(&open, Some(FileDialogAnswer::SaveTo("/pics/My image.psd".into())));
        app.poll_file_dialog(&ctx, None);
        assert_eq!(*written.borrow(), ["/pics/My image.psd"]);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/My image.psd"));
        assert_eq!(app.session.active().unwrap().doc.layers.len(), 2);
    }
}

#[test]
fn layered_filename_only_save_defaults_to_psd() {
    let (mut app, open, written) = jpeg_app(None, true);
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "My image.psd"));
    answer(&open, Some(FileDialogAnswer::SaveTo("My image.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["My image.psd"]);
}

#[test]
fn save_dialog_preserves_flat_and_existing_layered_format_defaults() {
    let ctx = egui::Context::default();
    for (path, layered) in [
        ("/pics/My image.JPEG", false),
        ("/pics/My image.PSD", true),
        ("/pics/My image.psb", true),
        ("/pics/My image.pcraft", true),
        ("/pics/My image.TIF", true),
        ("/pics/My image.tiff", true),
    ] {
        let (mut app, open, written) = jpeg_app(Some(path), layered);
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == path), "{path}");
        answer(&open, None);
        app.poll_file_dialog(&ctx, None);
        assert!(written.borrow().is_empty());
    }
}

#[test]
fn transparent_png_keeps_its_format_until_its_lone_layer_becomes_smart() {
    let (mut app, open, written) = app();
    std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).layers[0].surface_mut().unwrap().write_pixel(0, 0, &[1.0, 0.0, 0.0, 0.0]);
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "transparent.PNG", &Default::default()).unwrap().bytes;
    let doc = photocraft_io::import("transparent.PNG", &bytes).unwrap().document;
    assert_eq!(doc.layers.len(), 1);
    assert!(!doc.layers[0].locks.transparency, "transparent PNG imports as an unlocked raster layer");
    assert_eq!(doc.layers[0].surface().unwrap().pixel(0, 0)[3], 0.0);
    app.session.add_document(doc, Some("/pics/transparent.PNG".into()));
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "/pics/transparent.PNG"));
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1);
    assert!(matches!(app.session.active().unwrap().doc.layers[0].content, photocraft_doc::LayerContent::Smart(_)));
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "/pics/transparent.psd"));
    assert!(written.borrow().is_empty());
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
}

#[test]
fn layered_save_respects_explicit_jpeg_choices() {
    let ctx = egui::Context::default();
    let (mut app, open, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/exports/chosen.JPEG".into())));
    app.poll_file_dialog(&ctx, None);
    // JPEG Options asks first (FILE-215-19, #5563). The JPEG can't hold the layers, so it is a
    // copy and the document keeps its file (#2550).
    assert_eq!(app.jpeg_options.as_ref().map(|p| (p.path.as_str(), p.copy)), Some(("/exports/chosen.jpeg", true)));
    assert!(written.borrow().is_empty());
    crate::jpeg_options_ui::confirm(&mut app, &ctx);
    assert_eq!(*written.borrow(), ["/exports/chosen.jpeg"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/My image.JPEG"));
    // Explicit command paths carry the same choice and bypass the picker: without a JPEG answer
    // JPEG Options still asks, and a `quality` answers it.
    for command in ["file.save", "file.saveAs"] {
        let (mut app, open, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
        menus::invoke(&mut app, &ctx, command, json!({"path": "/exports/chosen.jpg"})).unwrap();
        assert!(!app.file_dialog_open() && open.borrow().is_empty());
        assert!(app.jpeg_options.as_ref().is_some_and(|p| p.copy), "{command}");
        crate::jpeg_options_ui::confirm(&mut app, &ctx);
        assert_eq!(*written.borrow(), ["/exports/chosen.jpg"]);

        let (mut app, open, written) = jpeg_app(Some("/pics/My image.JPEG"), true);
        menus::invoke(&mut app, &ctx, command, json!({"path": "/exports/chosen.jpg", "quality": 8})).unwrap();
        assert!(!app.file_dialog_open() && open.borrow().is_empty() && app.jpeg_options.is_none(), "{command}");
        assert_eq!(*written.borrow(), ["/exports/chosen.jpg"]);
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/My image.JPEG"));
    }
}

/// File › Save on a flat JPEG asks for its name; confirming its own file asks JPEG Options and,
/// once answered, records a plain Save that writes the file an action is played on (#3531).
#[test]
fn plain_save_of_a_flat_jpeg_asks_jpeg_options_and_records_a_plain_save() {
    let ctx = egui::Context::default();
    let (mut app, open, written) = jpeg_app(Some("/pics/photo.jpg"), false);
    app.run("actions.record", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/photo.jpg".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(app.jpeg_options.as_ref().map(|p| (p.record, p.copy)), Some(("file.save", false)));
    assert!(written.borrow().is_empty());
    crate::jpeg_options_ui::confirm(&mut app, &ctx);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(*written.borrow(), ["/pics/photo.jpg"]);
    assert_eq!(app.session.actions.list[0].steps, [("file.save".to_string(), json!({}))]);
}

/// A plain Save confirmed in the dialog keeps a changed JPEG Options answer in its recorded step,
/// and playing it writes the active flat JPEG with that answer, without asking.
#[test]
fn plain_save_of_a_flat_jpeg_records_and_replays_a_changed_quality() {
    let ctx = egui::Context::default();
    let (mut app, open, written) = jpeg_app(Some("/pics/photo.jpg"), false);
    let qualities: Rc<RefCell<Vec<Option<u8>>>> = Rc::default();
    let q = qualities.clone();
    app.services.export = Some(Box::new(move |_: &photocraft_doc::Document, _: &str, s: &crate::ExportSettings| {
        q.borrow_mut().push(s.jpeg_quality);
        Ok((b"out".to_vec(), Vec::new()))
    }));
    app.run("actions.record", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/photo.jpg".into())));
    app.poll_file_dialog(&ctx, None);
    let prompt = app.jpeg_options.as_mut().expect("JPEG Options");
    prompt.quality = 3;
    prompt.format = photocraft_codecs::JpegFormat::Progressive;
    crate::jpeg_options_ui::confirm(&mut app, &ctx);
    app.run("actions.stop", json!({})).unwrap();
    assert_eq!(app.session.actions.list[0].steps, [("file.save".to_string(), json!({"quality": 3, "jpegFormat": "progressive"}))]);
    // Played on another flat JPEG: written over itself at quality 3, with no dialog or prompt.
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "other.jpg", &Default::default()).unwrap().bytes;
    let index = app.session.add_document(photocraft_io::import("other.jpg", &bytes).unwrap().document, Some("/pics/other.jpg".into()));
    app.session.set_active(index);
    written.borrow_mut().clear();
    let played = app.run("actions.play", json!({"action": 0})).unwrap();
    assert!(played.get("failed").is_none(), "{played}");
    assert!(app.jpeg_options.is_none(), "playback does not ask");
    assert!(open.borrow().is_empty(), "no Save dialog");
    assert_eq!(*written.borrow(), ["/pics/other.jpg"]);
    assert_eq!(qualities.borrow().last().copied().flatten(), Some(photocraft_engine::file_cmds::jpeg_quality(3.0)));
}

/// The user answers the open dialog, from the dialog's own thread.
fn answer(open: &Open, answer: Option<FileDialogAnswer>) {
    let (_, reply) = open.borrow_mut().pop().expect("a dialog is open");
    std::thread::spawn(move || reply.send(answer)).join().unwrap();
}

#[test]
fn menu_open_exr_parts_asks_for_a_file_then_shows_the_parts_dialog() {
    // Before the fix the menu ran the bare command: `missing "path"`, and no dialog (#7169).
    let (mut app, open, _) = app();
    let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "render.exr", &Default::default()).unwrap().bytes;
    let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
    let dir = std::env::temp_dir().join(format!("photocraft-menu-exr-parts-{}-{unique}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();
    let path = dir.join("render.exr");
    std::fs::write(&path, &bytes).unwrap();
    let path = path.to_string_lossy().into_owned();
    let ctx = egui::Context::default();
    let r = menus::invoke(&mut app, &ctx, "file.openExrParts", json!({})).unwrap();
    assert_eq!(r["fileDialog"], "open", "{r}");
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Open { multiple: false, extensions: Some(e), .. }, _)] if e == &["exr"]));
    answer(&open, Some(FileDialogAnswer::Paths(vec![path.clone()])));
    app.poll_file_dialog(&ctx, None);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let dialog = app.ui.dialogs.iter().find(|d| d.fields.get("__command") == Some(&json!("file.openExrParts"))).expect("the parts dialog is open");
    assert_eq!(dialog.fields["path"], json!(path));
    let parts: Vec<&String> = dialog.fields.keys().filter(|k| k.starts_with("part/")).collect();
    assert_eq!(parts.len(), 1, "{:?}", dialog.fields);
    assert_eq!(dialog.fields[parts[0].as_str()], json!(true), "the first colour part is preselected");
    let id = dialog.id;
    let documents = app.session.documents().len();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.documents().len(), documents + 1);
    std::fs::remove_dir_all(&dir).unwrap();
}

#[test]
fn menu_place_starts_transform_for_each_chosen_image() {
    for command in ["file.placeEmbedded", "file.placeLinked"] {
        let (mut app, open, _) = app();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "source.png", &Default::default()).unwrap().bytes;
        let ctx = egui::Context::default();
        let initial_layers = app.session.active().unwrap().doc.layer_count();
        let initial_history = app.session.active().unwrap().history.past_len();
        let linked_dir = (command == "file.placeLinked").then(|| {
            let unique = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos();
            let dir = std::env::temp_dir().join(format!("photocraft-menu-place-{}-{unique}", std::process::id()));
            std::fs::create_dir(&dir).unwrap();
            dir
        });
        for index in 0..2 {
            menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
            app.poll_file_dialog(&ctx, None);
            let picked = if let Some(dir) = &linked_dir {
                let path = dir.join(format!("drawing-{index}.png"));
                std::fs::write(&path, &bytes).unwrap();
                FileDialogAnswer::Paths(vec![path.to_string_lossy().into_owned()])
            } else {
                FileDialogAnswer::Contents(format!("drawing-{index}.png"), bytes.clone())
            };
            answer(&open, Some(picked));
            app.poll_file_dialog(&ctx, None);
            let st = app.session.active().unwrap();
            assert_eq!(st.doc.layer_count(), initial_layers + index + 1);
            assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().name, format!("drawing-{index}"));
            let transform = app.ui.transform.as_ref().expect("File > Place must offer transform handles after the picker completes");
            assert_eq!(transform.made, Some(crate::state::MadeLayer::Place));
            if index == 0 {
                let layer = st.active_layer.unwrap();
                let photocraft_doc::LayerContent::Smart(smart) = &st.doc.layer(layer).unwrap().content else { panic!("placed Smart Object") };
                let original_scale = smart.transform.m[0];
                let transform = app.ui.transform.as_mut().unwrap();
                let left = transform.quad[0][0];
                for corner in &mut transform.quad {
                    corner[0] = left + (corner[0] - left) * 2.0;
                }
                crate::transform_tool::commit(&mut app);
                assert!(app.ui.transform.is_none(), "resizing the placed image must commit");
                let st = app.session.active().unwrap();
                let photocraft_doc::LayerContent::Smart(smart) = &st.doc.layer(layer).unwrap().content else { panic!("placed Smart Object") };
                assert!((smart.transform.m[0] - original_scale * 2.0).abs() < 1e-8, "placement width must double");
                assert_eq!(app.session.active().unwrap().history.past_len(), initial_history + 1);
            } else {
                crate::transform_tool::cancel(&mut app);
                assert_eq!(app.session.active().unwrap().doc.layer_count(), initial_layers + 1);
                assert_eq!(app.session.active().unwrap().history.past_len(), initial_history + 1);
            }
        }
        if let Some(dir) = linked_dir {
            for index in 0..2 {
                std::fs::remove_file(dir.join(format!("drawing-{index}.png"))).unwrap();
            }
            std::fs::remove_dir(dir).unwrap();
        }
    }
}

#[test]
fn cancelled_or_invalid_menu_place_does_not_start_transform() {
    for command in ["file.placeEmbedded", "file.placeLinked"] {
        for reply in [None, Some(FileDialogAnswer::Contents("broken.png".into(), b"not an image".to_vec()))] {
            let (mut app, open, _) = app();
            let ctx = egui::Context::default();
            let before = app.session.active().unwrap().doc.clone();
            let history = app.session.active().unwrap().history.past_len();
            menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
            app.poll_file_dialog(&ctx, None);
            answer(&open, reply);
            app.poll_file_dialog(&ctx, None);
            assert!(app.ui.transform.is_none());
            assert_eq!(app.session.active().unwrap().doc, before);
            assert_eq!(app.session.active().unwrap().history.past_len(), history);
        }
    }
}

#[test]
fn menu_place_preserves_an_unfinished_transform() {
    for start_before_picker in [true, false] {
        let (mut app, open, _) = app();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("layer.new.layerFromBackground", json!({})).unwrap();
        let bytes = photocraft_io::export(&app.session.active().unwrap().doc, "source.png", &Default::default()).unwrap().bytes;
        let ctx = egui::Context::default();
        if !start_before_picker {
            menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).unwrap();
            app.poll_file_dialog(&ctx, None);
        }
        crate::transform_tool::begin(&mut app, &ctx).unwrap();
        let transform = app.ui.transform.clone();
        let doc = app.session.active().unwrap().doc.clone();
        let history = app.session.active().unwrap().history.past_len();
        if start_before_picker {
            assert!(menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).is_err());
            assert!(!app.file_dialog_open());
        } else {
            answer(&open, Some(FileDialogAnswer::Contents("source.png".into(), bytes)));
            app.poll_file_dialog(&ctx, None);
        }
        assert_eq!(app.ui.transform, transform);
        assert_eq!(app.session.active().unwrap().doc, doc);
        assert_eq!(app.session.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn open_starts_in_the_last_used_folder() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    app.ui.recent_files = vec!["/pics/cat.psd".into(), "/old/dog.png".into()];
    menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(
        matches!(open.borrow().as_slice(), [(FileDialogRequest::Open { multiple: true, initial_dir: Some(dir), extensions: None }, _)] if dir == "/pics"),
        "{:?}",
        open.borrow().first().map(|(r, _)| r.clone())
    );
}

#[test]
fn ask_folder_uses_the_async_dialog_and_returns_the_selected_path() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    app.ui.recent_files = vec!["/pics/cat.psd".into()];
    let selected = Rc::new(RefCell::new(None));
    let result = selected.clone();

    assert_eq!(
        app.ask_folder(move |_, path| {
            *result.borrow_mut() = Some(path);
            Ok(Value::Null)
        })
        .unwrap(),
        json!({"fileDialog": "folder"})
    );
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Folder { initial_dir: Some(dir) }, _)] if dir == "/pics"));

    answer(&open, Some(FileDialogAnswer::FolderTo("/pics/frames".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(selected.borrow().as_deref(), Some("/pics/frames"));
}

#[test]
fn filtered_picker_requests_only_the_given_extensions() {
    let (mut app, open, _) = app();
    app.pick_file_bytes_filtered(&["cube", "3dl", "look"], |_, _, _| Ok(Value::Null)).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert!(matches!(
        open.borrow().first().map(|(r, _)| r),
        Some(FileDialogRequest::Open { multiple: false, extensions: Some(exts), .. })
            if exts.iter().map(String::as_str).collect::<Vec<_>>() == ["cube", "3dl", "look"]
    ));
}

#[test]
fn scripts_browse_lists_only_script_files() {
    let (mut app, open, _) = app();
    menus::invoke(&mut app, &egui::Context::default(), "file.scripts.browse", json!({})).unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
    assert!(matches!(
        open.borrow().first().map(|(r, _)| r),
        Some(FileDialogRequest::Open { multiple: false, extensions: Some(exts), .. })
            if exts.iter().map(String::as_str).collect::<Vec<_>>() == ["psjs", "ccjs", "jsx", "js", "jsxbin", "json", "txt", "pcdroplet"]
    ));
}

#[test]
fn scripts_browse_extendscript_picks_report_the_error() {
    // Photoshop scripts stay in the picker (matching its filter) and choosing one shows why it won't run.
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.scripts.browse", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Contents("resize.jsx".into(), b"alert('hi');".to_vec())));
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status_error);
    assert!(app.ui.status.contains("not Photoshop ExtendScript"), "{}", app.ui.status);
}

#[test]
fn last_used_dir_takes_the_most_recent_parent() {
    assert_eq!(last_used_dir(&[]), None);
    assert_eq!(last_used_dir(&["/pics/cat.psd".into()]), Some("/pics".into()));
    assert_eq!(last_used_dir(&["/pics/cat.psd".into(), "/old/dog.png".into()]), Some("/pics".into()));
    assert_eq!(last_used_dir(&["cat.psd".into()]), None, "no folder to start in");
    assert_eq!(last_used_dir(&["/".into()]), None, "the root has no parent");
}

#[test]
fn the_app_keeps_running_while_a_dialog_is_open() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap(), json!({"fileDialog": "save"}));
    assert!(open.borrow().is_empty(), "shown at the end of the frame, which has the window to parent it to");
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested.ends_with(".psd")));
    // Frames go on while the user is in the dialog: nothing is written, commands still run, and
    // a second dialog is refused.
    for _ in 0..3 {
        app.poll_file_dialog(&ctx, None);
    }
    assert!(written.borrow().is_empty());
    assert!(app.file_dialog_open());
    app.run("layer.new.layer", json!({})).unwrap();
    assert_eq!(menus::invoke(&mut app, &ctx, "file.open", json!({})).unwrap_err(), "a file dialog is already open");
    app.poll_file_dialog(&ctx, None);
    assert_eq!(open.borrow().len(), 1);
    // The answer wakes the app, and the save goes on from where it asked.
    for _ in 0..4 {
        ctx.run_ui(egui::RawInput::default(), |_| {}).textures_delta.clear();
    }
    assert!(!ctx.has_requested_repaint(), "idle");
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/a.psd".into())));
    assert!(ctx.has_requested_repaint());
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/a.psd"]);
    let st = app.session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/pics/a.psd"));
    assert_eq!(st.doc.name, "a.psd", "a successful Save As adopts the file name");
    assert!(!st.is_dirty());
    assert!(!app.file_dialog_open());
}

/// Preferences ▸ File Handling ▸ Lowercase Extension (default on): the chosen path's extension
/// is lowercased on the way to the writer; unchecked, the user's spelling is kept.
#[test]
fn save_paths_get_a_lowercase_extension_when_the_preference_is_on() {
    let ctx = egui::Context::default();
    {
        let (mut app, open, written) = app();
        menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
        app.poll_file_dialog(&ctx, None);
        answer(&open, Some(FileDialogAnswer::SaveTo("/pics/CAT.PSD".into())));
        app.poll_file_dialog(&ctx, None);
        assert_eq!(*written.borrow(), ["/pics/CAT.psd"], "only the extension is lowercased");
        assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/CAT.psd"));
    }
    let (mut app, open, written) = app();
    app.run("prefs.set", json!({"path": "fileHandling.lowercaseExtension", "value": false})).unwrap();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/CAT.PSD".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/CAT.PSD"], "off keeps the user's spelling");
}

/// #5039: a name typed without an extension (where file extensions are hidden) gets the
/// suggested file's extension, so the save picks its format instead of failing.
#[test]
fn save_paths_without_an_extension_get_the_suggested_one() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/cat".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/cat.psd"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/cat.psd"));
    // A name that already has an extension is left alone.
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/dog.png".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/cat.psd", "/pics/dog.png"]);
}

/// The question whether to replace a file (`replace_prompt`), when one is open.
fn replace_prompt(app: &PhotocraftApp) -> Option<u64> {
    app.ui.dialogs.iter().find(|d| crate::replace_prompt::owns(&d.fields)).map(|d| d.id)
}

/// File › Save As, answered with `path`.
fn save_as(app: &mut PhotocraftApp, open: &Open, path: &str) {
    let ctx = egui::Context::default();
    menus::invoke(app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(open, Some(FileDialogAnswer::SaveTo(path.into())));
    app.poll_file_dialog(&ctx, None);
}

/// The save dialog asked only about `cat`. The extension added to it names a file that exists,
/// so the save asks before replacing that file: Cancel writes nothing, Replace writes it.
#[test]
fn a_save_asks_before_replacing_the_file_its_added_extension_names() {
    let ctx = egui::Context::default();
    let (mut app, open, written) = app();
    let disk = on_disk(&mut app);
    changed_by_another_app(&disk, "/pics/cat.psd");
    save_as(&mut app, &open, "/pics/cat");
    let id = replace_prompt(&app).expect("the save asks first");
    let fields = &app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields;
    assert!(fields["message"].as_str().unwrap().contains("“cat.psd”"), "{fields:?}");
    assert!(fields["detail"].as_str().unwrap().contains("“cat”"), "{fields:?}");
    assert!(written.borrow().is_empty(), "nothing is written before the answer");
    app.poll_file_dialog(&ctx, None);
    assert!(app.file_dialog_open(), "the save waits for the answer");
    assert!(written.borrow().is_empty());

    crate::dialogs::cancel(&mut app, id).unwrap();
    assert!(written.borrow().is_empty(), "Cancel writes nothing");
    assert!(!app.file_dialog_open() && !app.ui.status_error);
    assert_eq!(app.session.active().unwrap().path, None);

    // A question closed without an answer counts as Cancel.
    save_as(&mut app, &open, "/pics/cat");
    app.ui.close_dialog(replace_prompt(&app).expect("asked again"));
    app.poll_file_dialog(&ctx, None);
    assert!(written.borrow().is_empty() && !app.file_dialog_open());

    // Replace answers with the save's failure.
    save_as(&mut app, &open, "/pics/cat");
    let write = app.services.write.replace(Box::new(|_: &str, _: &[u8]| Err("disk full".into())));
    let id = replace_prompt(&app).expect("asked again");
    let e = crate::dialogs::confirm(&mut app, id).unwrap_err();
    assert!(e.contains("disk full") && app.ui.status_error, "{e}");
    app.services.write = write;

    save_as(&mut app, &open, "/pics/cat");
    let id = replace_prompt(&app).expect("asked again");
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(*written.borrow(), ["/pics/cat.psd"]);
    assert_eq!(app.session.active().unwrap().path.as_deref(), Some("/pics/cat.psd"));
    assert!(!app.file_dialog_open() && replace_prompt(&app).is_none());
}

/// Lowercase Extension changes `CAT.PSD` to `CAT.psd`. Where letter case tells two files apart
/// (Linux), the dialog asked only about `CAT.PSD`; where it doesn't (Windows and macOS by
/// default), both names reach the same file, which the dialog asked about.
#[test]
fn a_save_asks_before_replacing_the_file_its_lowercased_extension_names() {
    {
        // Two files, even with the same size and modification time.
        let (mut app, open, written) = app();
        let disk = on_disk(&mut app);
        for name in ["/pics/CAT.psd", "/pics/CAT.PSD"] {
            changed_by_another_app(&disk, name);
        }
        app.services.same_file = Some(Box::new(|a: &str, b: &str| a == b));
        save_as(&mut app, &open, "/pics/CAT.PSD");
        let id = replace_prompt(&app).expect("the save asks first");
        assert!(written.borrow().is_empty());
        crate::dialogs::confirm(&mut app, id).unwrap();
        assert_eq!(*written.borrow(), ["/pics/CAT.psd"]);
    }
    let (mut app, open, written) = app();
    let disk = on_disk(&mut app);
    for name in ["/pics/CAT.psd", "/pics/CAT.PSD"] {
        changed_by_another_app(&disk, name);
    }
    app.services.same_file = Some(Box::new(|a: &str, b: &str| a.eq_ignore_ascii_case(b)));
    save_as(&mut app, &open, "/pics/CAT.PSD");
    assert!(replace_prompt(&app).is_none(), "the dialog asked about this file");
    assert_eq!(*written.borrow(), ["/pics/CAT.psd"]);
}

/// The save dialog asked about the file it answered: a bare name with no file under the added
/// extension, and a full name, are written at once.
#[test]
fn a_save_writes_at_once_when_the_dialog_asked_about_the_file() {
    let (mut app, open, written) = app();
    let disk = on_disk(&mut app);
    changed_by_another_app(&disk, "/pics/dog.png");
    save_as(&mut app, &open, "/pics/cat");
    save_as(&mut app, &open, "/pics/dog.png");
    assert!(replace_prompt(&app).is_none());
    assert_eq!(*written.borrow(), ["/pics/cat.psd", "/pics/dog.png"]);
}

#[test]
fn save_and_export_dialogs_start_beside_the_document() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    let dir = std::env::temp_dir().join("PhotoCraft projects").join("图像");
    let source = dir.join("original.psd");
    app.session.active_mut().unwrap().path = Some(source.to_string_lossy().into_owned());
    for command in ["file.saveAs", "file.export.exportAs", "file.export.saveForWebLegacy"] {
        let r = menus::invoke(&mut app, &ctx, command, json!({})).unwrap();
        if let Some(id) = r["dialog"].as_u64() {
            crate::dialogs::confirm(&mut app, id).unwrap();
        }
        app.poll_file_dialog(&ctx, None);
        {
            let held = open.borrow();
            let (FileDialogRequest::Save { suggested }, _) = &held[0] else { panic!("save dialog expected") };
            assert_eq!(std::path::Path::new(suggested).parent(), Some(dir.as_path()), "{command}");
            if command == "file.saveAs" {
                assert_eq!(std::path::Path::new(suggested), source);
            }
        }
        answer(&open, None);
        app.poll_file_dialog(&ctx, None);
    }
}

/// The path `pick_save` suggests for `name` in `dir` on this platform: it joins with `Path::join`,
/// which uses `\` on Windows, so the expected values must be built the same way (#2745).
fn joined(dir: &str, name: &str) -> String {
    std::path::Path::new(dir).join(name).to_string_lossy().into_owned()
}

/// #1826: after the user navigates a save or export dialog elsewhere, the document's next dialog
/// starts there. Save As of a saved document still suggests its own path; other documents and
/// closed documents are unaffected.
#[test]
fn save_dialogs_remember_where_each_document_was_last_saved() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    let first = app.session.active().unwrap().doc.id;
    app.session.active_mut().unwrap().path = Some("/proj/original.psd".into());
    let suggested = |open: &Open| match &open.borrow()[0].0 {
        FileDialogRequest::Save { suggested } => suggested.clone(),
        other => panic!("save dialog expected, got {other:?}"),
    };
    // An export starts beside the document; the user saves it under /renders instead.
    app.pick_save("result.png", |_, _| Ok(Value::Null)).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(suggested(&open), joined("/proj", "result.png"));
    answer(&open, Some(FileDialogAnswer::SaveTo("/renders/result.png".into())));
    app.poll_file_dialog(&ctx, None);
    // The next export starts in /renders; Save As still offers the document's own path.
    app.pick_save("again.png", |_, _| Ok(Value::Null)).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(suggested(&open), joined("/renders", "again.png"));
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(suggested(&open), "/proj/original.psd");
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    // A cancelled dialog and an explicit directory change nothing.
    app.pick_save("/elsewhere/x.png", |_, _| Ok(Value::Null)).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(suggested(&open), "/elsewhere/x.png");
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    app.pick_save("third.png", |_, _| Ok(Value::Null)).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(suggested(&open), joined("/renders", "third.png"));
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    // Another document has its own memory: untitled, it starts wherever the shell defaults.
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    let second = app.session.active().unwrap().doc.id;
    assert_ne!(first, second);
    app.pick_save("fresh.png", |_, _| Ok(Value::Null)).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(suggested(&open), "fresh.png");
    answer(&open, Some(FileDialogAnswer::SaveTo("/other/fresh.png".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(app.save_dirs.get(&second).map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/other"));
    assert_eq!(app.save_dirs.get(&first).map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/renders"));
    // The folder is remembered for the document the dialog was asked for, not the active one.
    app.session.set_active(0);
    app.pick_save("swap.png", |_, _| Ok(Value::Null)).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(suggested(&open), joined("/renders", "swap.png"));
    app.session.set_active(1);
    answer(&open, Some(FileDialogAnswer::SaveTo("/moved/swap.png".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(app.save_dirs.get(&first).map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/moved"));
    assert_eq!(app.save_dirs.get(&second).map(|p| p.to_string_lossy().into_owned()).as_deref(), Some("/other"));
    // Closing a document forgets its folder at the next dialog.
    app.run("file.close", json!({"document": 1})).unwrap();
    app.pick_save("last.png", |_, _| Ok(Value::Null)).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(suggested(&open), joined("/moved", "last.png"));
    assert!(!app.save_dirs.contains_key(&second));
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
}

#[test]
fn save_suggestions_preserve_explicit_directories_and_untitled_names() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    for (document_path, suggested) in
        [(None, "Untitled.png"), (Some("project/source.psd"), "exports/result.png"), (Some("project/source.psd"), "/other/result.png")]
    {
        app.session.active_mut().unwrap().path = document_path.map(str::to_string);
        app.pick_save(suggested, |_, _| Ok(Value::Null)).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(matches!(&open.borrow()[0].0, FileDialogRequest::Save { suggested: actual } if actual == suggested));
        answer(&open, None);
        app.poll_file_dialog(&ctx, None);
    }
}

#[test]
fn the_answer_goes_to_the_document_it_was_asked_for() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.session.set_active(0);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.session.set_active(1);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/first.psd".into())));
    app.poll_file_dialog(&ctx, None);
    let paths: Vec<_> = app.session.documents().iter().map(|d| d.path.clone()).collect();
    assert_eq!(paths, [Some("/pics/first.psd".to_string()), None]);
    assert_eq!(app.session.active_index(), Some(1), "a completed save preserves the selected tab");
    assert_eq!(app.session.documents()[0].doc.name, "first.psd");
    assert_ne!(app.session.documents()[1].doc.name, "first.psd");
    // Once that document is closed, the answer has nothing to save.
    app.session.set_active(0);
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.run("file.close", json!({"document": 0})).unwrap();
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/again.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/first.psd"]);
    assert!(app.ui.status_error && app.session.documents()[0].path.is_none());
}

#[test]
fn cancel_and_a_dialog_that_could_not_show_end_quietly() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    app.run("layer.new.layer", json!({})).unwrap();
    let before = app.session.active().unwrap().clone();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    assert!(!app.file_dialog_open() && !app.ui.status_error);
    // A reply dropped unanswered (the dialog failed to show) is a Cancel, not a dialog left open forever.
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    open.borrow_mut().clear();
    app.poll_file_dialog(&ctx, None);
    assert!(!app.file_dialog_open() && !app.ui.status_error);
    assert!(written.borrow().is_empty());
    let after = app.session.active().unwrap();
    assert_eq!(after.doc.name, before.doc.name);
    assert_eq!(after.path, before.path);
    assert_eq!((after.revision, after.saved_revision), (before.revision, before.saved_revision));
    // Without a dialog service, asking is a Cancel straight away.
    app.services.file_dialog = None;
    assert_eq!(app.open_dialog_file().unwrap_err(), CANCELLED);
    assert!(!app.file_dialog_open());
}

#[test]
fn opening_another_document_before_the_save_answer_keeps_its_identity() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    let first_id = app.active_doc_id().unwrap();
    app.save_as(None).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.run("file.new", json!({"width": 4, "height": 4, "name": "Untitled-2"})).unwrap();
    let second_id = app.active_doc_id().unwrap();
    app.run("document.move", json!({"document": 0, "to": 1})).unwrap();
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/保存 first.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/pics/保存 first.psd"]);
    let first = app.session.documents().iter().find(|s| s.doc.id == first_id).unwrap();
    assert_eq!(first.doc.name, "保存 first.psd");
    assert_eq!(first.path.as_deref(), Some("/pics/保存 first.psd"));
    assert_eq!(app.active_doc_id().unwrap(), second_id);
    assert_eq!(app.session.active().unwrap().doc.name, "Untitled-2");
}

#[test]
fn failed_deferred_save_preserves_both_identities_and_the_selected_tab() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    app.run("layer.new.layer", json!({})).unwrap();
    app.save_as(None).unwrap();
    app.poll_file_dialog(&ctx, None);
    app.run("file.new", json!({"width": 4, "height": 4, "name": "second"})).unwrap();
    let before: Vec<_> = app.session.documents().iter().map(|d| (d.doc.name.clone(), d.path.clone(), d.revision, d.saved_revision)).collect();
    app.services.write = Some(Box::new(|_, _| Err("disk full".into())));
    answer(&open, Some(FileDialogAnswer::SaveTo("failed.psd".into())));
    app.poll_file_dialog(&ctx, None);
    let after: Vec<_> = app.session.documents().iter().map(|d| (d.doc.name.clone(), d.path.clone(), d.revision, d.saved_revision)).collect();
    assert_eq!(after, before);
    assert_eq!(app.session.active_index(), Some(1));
    assert!(app.ui.status_error && app.ui.status == "disk full");
    assert!(written.borrow().is_empty());
}

#[test]
fn an_answer_of_the_wrong_kind_is_an_error_not_a_crash() {
    let (mut app, open, written) = app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Paths(vec!["/pics/a.psd".into()])));
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status_error && app.ui.status == UNEXPECTED);
    assert!(written.borrow().is_empty());
    app.open_dialog_file().unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/a.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(app.session.documents().len(), 1);
}

/// The app with a painted layer converted to a smart object and a duplicate instance of it.
fn smart_app() -> (PhotocraftApp, Open, Rc<RefCell<Vec<String>>>, [u64; 2]) {
    let (mut app, open, written) = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 4, 4), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let a = app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap()["layer"].as_u64().unwrap();
    let b = app.run("layer.duplicate", json!({"layer": a})).unwrap()["layer"].as_u64().unwrap();
    app.session.select_layer(photocraft_doc::LayerId(a)).unwrap();
    (app, open, written, [a, b])
}

fn smart_source(app: &PhotocraftApp, id: u64) -> photocraft_doc::SmartSource {
    match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        photocraft_doc::LayerContent::Smart(sm) => sm.source.clone(),
        other => panic!("not a smart object: {}", other.kind_name()),
    }
}

#[test]
fn place_embedded_uses_the_single_file_picker_and_accepts_contents() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    assert!(menus::is_enabled(&app, "file.placeEmbedded"));
    let source = photocraft_io::export(&app.session.active().unwrap().doc, "source.png", &Default::default()).unwrap().bytes;
    let layers_before = app.session.active().unwrap().doc.layers.len();

    // Cancel leaves the document alone and does not report an error.
    menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Open { multiple: false, .. }, _)]));
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers_before);
    assert!(!app.ui.status_error, "{}", app.ui.status);

    // The Web picker returns the selected file's name and bytes instead of a filesystem path.
    menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Contents("source.png".into(), source)));
    app.poll_file_dialog(&ctx, None);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers_before + 1);
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(layer.name, "source");
    assert_eq!(app.ui.transform.as_ref().and_then(|t| t.made), Some(crate::state::MadeLayer::Place));
    assert!(matches!(
        &layer.content,
        photocraft_doc::LayerContent::Smart(sm)
            if matches!(&sm.source, photocraft_doc::SmartSource::Embedded { file_name, .. } if file_name == "source.png")
    ));

    // Finish the successful placement before starting the next picker.
    crate::transform_tool::commit(&mut app);
    assert!(app.ui.transform.is_none());

    // Invalid contents are reported without adding a layer.
    menus::invoke(&mut app, &ctx, "file.placeEmbedded", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Contents("broken.png".into(), b"not an image".to_vec())));
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status_error);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers_before + 1);
}

#[test]
fn replace_contents_picks_a_file_for_every_instance() {
    let (mut app, open, _, ids) = smart_app();
    let ctx = egui::Context::default();
    let before = ids.map(|id| smart_source(&app, id));
    // Cancelling changes nothing.
    menus::invoke(&mut app, &ctx, "layer.smartObjects.replaceContents", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Open { multiple: false, .. }, _)]));
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    assert_eq!(ids.map(|id| smart_source(&app, id)), before);
    // A picked file (by contents, as on the web) replaces the contents of both instances.
    let png = photocraft_io::export(&app.session.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
    menus::invoke(&mut app, &ctx, "layer.smartObjects.replaceContents", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Contents("art.png".into(), png)));
    app.poll_file_dialog(&ctx, None);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    for id in ids {
        assert!(matches!(smart_source(&app, id), photocraft_doc::SmartSource::Embedded { ref file_name, .. } if file_name == "art.png"));
    }
    // A file that isn't an image is an error, and the contents stay.
    menus::invoke(&mut app, &ctx, "layer.smartObjects.replaceContents", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Contents("notes.png".into(), b"not an image".to_vec())));
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status_error);
    assert!(matches!(smart_source(&app, ids[1]), photocraft_doc::SmartSource::Embedded { ref file_name, .. } if file_name == "art.png"));
}

/// Layer › Smart Objects › Relink to File… (and the layer context menu item) asks for a file and
/// links every instance to its path; it ran without a path before, so it always failed.
#[test]
fn relink_to_file_picks_a_file_for_every_instance() {
    let (mut app, open, _, ids) = smart_app();
    let ctx = egui::Context::default();
    let before = ids.map(|id| smart_source(&app, id));
    // Cancelling changes nothing.
    menus::invoke(&mut app, &ctx, "layer.smartObjects.relinkToFile", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Open { multiple: false, .. }, _)]));
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    assert_eq!(ids.map(|id| smart_source(&app, id)), before);
    // The web picker answers only a name and bytes: nothing to link to, so the contents stay.
    let png = photocraft_io::export(&app.session.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
    menus::invoke(&mut app, &ctx, "layer.smartObjects.relinkToFile", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Contents("art.png".into(), png.clone())));
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status_error);
    assert_eq!(ids.map(|id| smart_source(&app, id)), before);
    // A file picked on disk links both instances to its path.
    let dir = std::env::temp_dir().join(format!("pcraft-relink-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("art.png");
    std::fs::write(&path, &png).unwrap();
    let path = path.to_string_lossy().into_owned();
    menus::invoke(&mut app, &ctx, "layer.smartObjects.relinkToFile", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Paths(vec![path.clone()])));
    app.poll_file_dialog(&ctx, None);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    std::fs::remove_dir_all(&dir).ok();
    for id in ids {
        assert_eq!(smart_source(&app, id), photocraft_doc::SmartSource::Linked { path: path.clone() });
    }
}

#[test]
fn export_contents_suggests_the_contents_file_name() {
    let (mut app, open, written, _) = smart_app();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "layer.smartObjects.exportContents", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(
        matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "Layer 1.pcraft"),
        "{:?}",
        open.borrow().first().map(|(r, _)| r.clone())
    );
    answer(&open, Some(FileDialogAnswer::SaveTo("/out/contents.pcraft".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(*written.borrow(), ["/out/contents.pcraft"]);
}

#[test]
fn convert_to_linked_asks_where_to_write_the_contents() {
    let (mut app, open, _, ids) = smart_app();
    let ctx = egui::Context::default();
    let dir = std::env::temp_dir().join(format!("pcraft-convert-linked-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("linked.pcraft").to_string_lossy().into_owned();
    menus::invoke(&mut app, &ctx, "layer.smartObjects.convertToLinked", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { .. }, _)]));
    answer(&open, Some(FileDialogAnswer::SaveTo(path.clone())));
    app.poll_file_dialog(&ctx, None);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert!(std::fs::metadata(&path).is_ok_and(|m| m.len() > 0));
    std::fs::remove_dir_all(&dir).ok();
    for id in ids {
        assert_eq!(smart_source(&app, id), photocraft_doc::SmartSource::Linked { path: path.clone() });
    }
}

#[test]
fn quick_export_selected_layers_writes_separate_pngs_with_one_prompt() {
    let (mut app, open, written) = app();
    app.run("layer.new.layer", json!({"name": "Same/name"})).unwrap();
    app.run("paint.bucket", json!({"x": 0, "y": 0, "color": "#ff0000"})).unwrap();
    app.run("layer.new.layer", json!({"name": "Same/name"})).unwrap();
    app.run("paint.bucket", json!({"x": 0, "y": 0, "color": "#00ff00"})).unwrap();
    app.run("select.allLayers", json!({})).unwrap();
    let selected = app.session.active().unwrap().selected_layers();
    let before = app.session.active().unwrap().doc.clone();
    let sources = Rc::new(RefCell::new(Vec::new()));
    let capture = sources.clone();
    app.services.export = Some(Box::new(move |doc, path, _| {
        assert_eq!(doc.layers.len(), 1, "each PNG must contain just its source layer");
        let png = photocraft_io::export(doc, path, &Default::default()).unwrap();
        assert!(png.bytes.starts_with(b"\x89PNG\r\n\x1a\n"));
        capture.borrow_mut().push(doc.name.clone());
        Ok((png.bytes, Vec::new()))
    }));
    let ctx = egui::Context::default();
    assert!(!crate::layer_menu_ui::single_layer_only("layer.quickExportAsPng"));
    menus::invoke(&mut app, &ctx, "layer.quickExportAsPng", Value::Null).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(open.borrow().len(), 1, "one destination for the entire selection");
    // Selection changes while choosing a destination must not change the captured sources.
    app.run("layer.select", json!({"layer": selected[0].0})).unwrap();
    answer(&open, Some(FileDialogAnswer::SaveTo("selected.png".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(written.borrow().len(), selected.len());
    assert_eq!(sources.borrow().len(), selected.len());
    assert_eq!(written.borrow()[0], "selected.png");
    assert_eq!(written.borrow()[1], "selected-2-Same_name.png");
    assert_eq!(selected.len(), 2, "Select All Layers leaves the locked Background out");
    assert_eq!(app.session.active().unwrap().doc, before);
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// #6599: Quick Export as PNG with Location = Same Folder asks before replacing the document's
/// own file (a document opened from photo.png), and keeps writing beside a PSD without a prompt.
#[test]
fn same_folder_quick_export_asks_before_replacing_the_documents_own_file() {
    let dir = std::env::temp_dir().join(format!("photocraft-quick-own-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let own = dir.join("photo.png").to_string_lossy().into_owned();
    std::fs::write(&own, b"original").unwrap();
    let (mut app, open, _) = app();
    app.run("prefs.set", json!({"path": "export.quickExportLocation", "value": "sameFolder"})).unwrap();
    app.session.active_mut().unwrap().path = Some(own.clone());
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.export.quickExportAsPng", Value::Null).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(open.borrow().len(), 1, "the save dialog asks");
    assert_eq!(std::fs::read(&own).unwrap(), b"original", "nothing written before the answer");
    answer(&open, Some(FileDialogAnswer::SaveTo(own.clone())));
    app.poll_file_dialog(&ctx, None);
    assert!(std::fs::read(&own).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"), "replaced once confirmed: {}", app.ui.status);

    // Beside a PSD, Same Folder still writes without a prompt.
    app.session.active_mut().unwrap().path = Some(dir.join("photo.psd").to_string_lossy().into_owned());
    std::fs::write(&own, b"earlier export").unwrap();
    menus::invoke(&mut app, &ctx, "file.export.quickExportAsPng", Value::Null).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert!(open.borrow().is_empty(), "no dialog");
    assert!(std::fs::read(&own).unwrap().starts_with(b"\x89PNG\r\n\x1a\n"));
    std::fs::remove_dir_all(&dir).ok();
}

/// #6597: once Save As or Save a Copy writes over the file another open document is bound to,
/// that document no longer has the file and reads as unsaved (so closing it asks, and its Save
/// asks for a name), and a notice says so. Documents bound to other files are left alone.
#[test]
fn saving_over_the_file_of_another_open_document_detaches_it() {
    let dir = std::env::temp_dir().join(format!("photocraft-save-over-tab-{}", std::process::id()));
    let (a, d) = (dir.join("a.psd").to_string_lossy().into_owned(), dir.join("d.psd").to_string_lossy().into_owned());
    let (mut app, open, written) = app();
    let saved_as = |app: &mut PhotocraftApp, path: &str| {
        let st = app.session.active_mut().unwrap();
        st.path = Some(path.to_string());
        st.saved_revision = st.revision;
    };
    // A lives in a.psd and D in d.psd, both saved.
    saved_as(&mut app, &a);
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    saved_as(&mut app, &d);
    // B, in another tab, is saved as a.psd.
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    app.save_as(Some(a.clone())).unwrap();
    let docs = app.session.documents();
    assert_eq!((docs[0].path.as_deref(), docs[0].is_dirty()), (None, true), "A lost a.psd and is unsaved");
    assert_eq!((docs[1].path.as_deref(), docs[1].is_dirty()), (Some(d.as_str()), false), "D is untouched");
    assert_eq!(docs[2].path.as_deref(), Some(a.as_str()), "B lives in a.psd");
    let notice = app.ui.notices.last().expect("a notice");
    assert!(notice.title.contains("a.psd was saved over") && notice.lines.len() == 1, "{} {:?}", notice.title, notice.lines);
    // C's Save a Copy onto a.psd detaches B the same way, and C doesn't take the file.
    app.run("file.new", json!({"width": 4, "height": 4})).unwrap();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo(a.clone())));
    app.poll_file_dialog(&ctx, None);
    let docs = app.session.documents();
    assert_eq!((docs[2].path.as_deref(), docs[2].is_dirty()), (None, true), "B lost a.psd and is unsaved");
    assert_eq!(docs[3].path, None, "a copy doesn't bind C to a.psd");
    assert_eq!(*written.borrow(), [a.clone(), a]);
}

#[test]
fn cancelling_multi_layer_quick_export_writes_nothing() {
    let (mut app, open, written) = app();
    for color in ["#ff0000", "#00ff00"] {
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("paint.bucket", json!({"x": 0, "y": 0, "color": color})).unwrap();
    }
    app.run("select.allLayers", json!({})).unwrap();
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "layer.quickExportAsPng", Value::Null).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    assert!(written.borrow().is_empty());
}

/// #3354: a system with no dialog backend answers `Unavailable`; the message reaches the status
/// bar as an error, where a cancelled dialog stays silent.
#[test]
fn an_unavailable_dialog_is_reported_and_a_cancelled_one_is_not() {
    let ctx = egui::Context::default();
    let (mut app, open, _) = app();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    assert!(!app.ui.status_error, "Cancel is the user's choice");
    assert!(!app.file_dialog_open());

    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::Unavailable("No file dialog can open".into())));
    app.poll_file_dialog(&ctx, None);
    assert!(app.ui.status_error);
    assert_eq!(app.ui.status, "No file dialog can open");
    assert!(!app.file_dialog_open(), "the pending dialog is over");
}

/// #3629: Layer › Export As with several artboards selected exports one file per board, at each
/// board's size and with the dialog's settings, after one destination prompt.
#[test]
fn export_as_with_several_artboards_selected_writes_one_file_each() {
    let (mut app, open, written) = app();
    let a = app.run("layer.new.artboard", json!({"rect": [0, 0, 40, 30], "name": "Header"})).unwrap()["layer"].as_u64().unwrap();
    let b = app.run("layer.new.artboard", json!({"rect": [60, 0, 20, 50], "name": "Capsule"})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": a})).unwrap();
    app.run("layer.select", json!({"layer": b, "mode": "add"})).unwrap();
    assert_eq!(app.session.active().unwrap().selected_layers().len(), 2);
    let sizes = Rc::new(RefCell::new(Vec::new()));
    let capture = sizes.clone();
    app.services.export = Some(Box::new(move |doc: &photocraft_doc::Document, _: &str, _: &crate::ExportSettings| {
        capture.borrow_mut().push((doc.name.clone(), doc.size.width, doc.size.height));
        Ok((b"out".to_vec(), Vec::new()))
    }));
    assert!(!crate::layer_menu_ui::single_layer_only("layer.exportAs"), "the menu offers it for a selection");
    let ctx = egui::Context::default();
    let r = menus::invoke(&mut app, &ctx, "layer.exportAs", Value::Null).unwrap();
    let id = r["dialog"].as_u64().unwrap();
    let mut fields = app.ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    assert_eq!(fields["__label"], "Export As: 2 layers");
    // The dialog's settings apply to every file: JPG at 50 %.
    fields.insert("format".into(), json!("jpg"));
    fields.insert("scale".into(), json!(50));
    crate::export_dialog::confirm(&mut app, &fields).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(open.borrow().len(), 1, "one destination for every board");
    answer(&open, Some(FileDialogAnswer::SaveTo("/out/boards.jpg".into())));
    app.poll_file_dialog(&ctx, None);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    assert_eq!(written.borrow().len(), 2);
    assert_eq!(written.borrow()[0], "/out/boards.jpg");
    // The sibling path is built with `Path::join`, so Windows turns the separator into `\`.
    let second = written.borrow()[1].replace('\\', "/");
    assert!(second.starts_with("/out/boards-2-") && second.ends_with(".jpg"), "{:?}", written.borrow());
    let written = written.borrow();
    assert_eq!(written.len(), 2);
    // First file uses the exact path from the dialog (forward slashes in test)
    assert_eq!(std::path::Path::new(&written[0]).file_name().and_then(|n| n.to_str()), Some("boards.jpg"));
    // Second file is generated by batch_paths which uses platform path separators
    let sibling = std::path::Path::new(&written[1]);
    assert_eq!(sibling.parent(), std::path::Path::new(&written[0]).parent());
    assert_eq!(sibling.file_name().and_then(|name| name.to_str()), Some("boards-2-Capsule.jpg"));
    let mut got = sizes.borrow().iter().map(|(_, w, h)| (*w, *h)).collect::<Vec<_>>();
    got.sort_unstable();
    assert_eq!(got, [(10, 25), (20, 15)], "each board at its own size, scaled to 50 %");
}

/// #3806: File › Save on an OpenRaster file that can't hold the document (here an adjustment
/// layer) doesn't overwrite it: Save As opens on a PSD with a notice saying what the .ora would
/// drop, and the PSD becomes the document's file. A plain document still saves back to its .ora.
#[test]
fn openraster_save_never_overwrites_what_it_cannot_hold() {
    let (mut app, open, written) = app();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session.active_mut().unwrap().path = Some("/pics/art.ora".into());
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(*written.borrow(), ["/pics/art.ora"], "a plain document saves in place");
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    let notices = app.ui.notices.len();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    assert_eq!(written.borrow().len(), 1, "the .ora is not overwritten");
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if suggested == "/pics/art.psd"), "Save As offers PSD");
    assert_eq!(app.ui.notices.len(), notices + 1);
    assert!(app.ui.notices.last().unwrap().lines.iter().any(|l| l.contains("adjustment layers")), "{:?}", app.ui.notices.last().unwrap().lines);
    assert!(app.session.active().unwrap().is_dirty(), "the edits are still unsaved");
    answer(&open, Some(FileDialogAnswer::SaveTo("/pics/art.psd".into())));
    app.poll_file_dialog(&ctx, None);
    assert_eq!(written.borrow().last().map(String::as_str), Some("/pics/art.psd"));
    let st = app.session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/pics/art.psd"));
    assert!(!st.is_dirty());
}

/// Save As to OpenRaster of a document it can't hold writes a copy, as Photoshop does for formats
/// that can't keep everything: the document keeps its file and its unsaved state. An automation
/// save without a path won't overwrite such an .ora either.
#[test]
fn save_as_openraster_of_what_it_cannot_hold_is_a_copy() {
    let (mut app, _open, written) = app();
    app.run("layer.newAdjustmentLayer.invert", json!({})).unwrap();
    app.session.active_mut().unwrap().path = Some("/pics/art.psd".into());
    let ctx = egui::Context::default();
    menus::invoke(&mut app, &ctx, "file.saveAs", json!({"path": "/pics/copy.ora"})).unwrap();
    assert_eq!(*written.borrow(), ["/pics/copy.ora"]);
    let st = app.session.active().unwrap();
    assert_eq!(st.path.as_deref(), Some("/pics/art.psd"), "the document keeps its file");
    assert!(st.is_dirty());
    app.session.active_mut().unwrap().path = Some("/pics/art.ora".into());
    let err = app.save_automation(None).unwrap_err();
    assert!(err.contains("OpenRaster"), "{err}");
    assert_eq!(written.borrow().len(), 1);
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
fn render_video_menu_asks_for_directory_and_writes_frames_only_after_confirmation() {
    let (mut app, open, _) = app();
    let ctx = egui::Context::default();
    let dir = std::env::temp_dir().join(format!(
        "pc-render-video-ui-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    let path = dir.to_string_lossy().into_owned();

    let result = menus::invoke(&mut app, &ctx, "file.export.renderVideo", json!({})).unwrap();
    assert_eq!(result["fileDialog"], "folder");
    app.poll_file_dialog(&ctx, None);
    assert!(matches!(open.borrow().as_slice(), [(FileDialogRequest::Folder { .. }, _)]));
    assert!(!dir.exists(), "a menu click must not create an output directory");

    answer(&open, None);
    app.poll_file_dialog(&ctx, None);
    assert!(!dir.exists(), "cancelling the picker must not render video");

    menus::invoke(&mut app, &ctx, "file.export.renderVideo", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::FolderTo(path.clone())));
    app.poll_file_dialog(&ctx, None);
    let rendered: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|entry| entry.unwrap().path()).collect();
    assert_eq!(rendered.len(), 1, "a document without a timeline exports one frame");
    assert_eq!(rendered[0].extension().and_then(|ext| ext.to_str()), Some("png"));
    std::fs::remove_dir_all(&dir).unwrap();

    // Commands that already supply a directory must not open a file picker.
    let result = menus::invoke(&mut app, &ctx, "file.export.renderVideo", json!({"dir": path})).unwrap();
    assert!(result["frames"].as_u64().is_some());
    assert!(!app.file_dialog_open());
    std::fs::remove_dir_all(&dir).unwrap();
}

/// How many times each path was written, standing in for its size and modification time.
type Disk = Rc<RefCell<std::collections::HashMap<String, u64>>>;

/// Gives `app` file stamps from a fake disk that its own writes update too.
fn on_disk(app: &mut PhotocraftApp) -> Disk {
    let disk = Disk::default();
    let (read, wrote) = (disk.clone(), disk.clone());
    app.services.file_stamp = Some(Box::new(move |path| read.borrow().get(path).map(|&n| crate::FileStamp { len: n, modified: None })));
    let mut write = app.services.write.take().unwrap();
    app.services.write = Some(Box::new(move |path, bytes| {
        write(path, bytes)?;
        *wrote.borrow_mut().entry(path.to_string()).or_default() += 1;
        Ok(())
    }));
    disk
}

fn changed_by_another_app(disk: &Disk, path: &str) {
    *disk.borrow_mut().entry(path.to_string()).or_default() += 1;
}

fn disk_prompt(app: &PhotocraftApp) -> Option<&crate::state::Dialog> {
    app.ui.dialogs.iter().find(|d| crate::disk_change_prompt::owns(&d.fields))
}

#[test]
fn save_asks_before_replacing_a_file_another_app_changed() {
    let ctx = egui::Context::default();
    let (mut app, _open, written) = app();
    let disk = on_disk(&mut app);
    changed_by_another_app(&disk, "/pics/a.psd");
    app.opened_from("/pics/a.psd");
    app.run("layer.new.layer", json!({})).unwrap();
    // Unchanged on disk: Save writes at once.
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(*written.borrow(), ["/pics/a.psd"]);
    assert!(disk_prompt(&app).is_none());

    // Our own save isn't a change; another app's write after it is.
    changed_by_another_app(&disk, "/pics/a.psd");
    app.run("layer.new.layer", json!({})).unwrap();
    let r = menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(r["confirmationPending"], true);
    assert_eq!(written.borrow().len(), 1, "nothing is written before the answer");
    let prompt = disk_prompt(&app).expect("Save asks first");
    assert!(prompt.fields["message"].as_str().unwrap().contains("a.psd"));

    // Cancel leaves the file and the unsaved document alone.
    let id = prompt.id;
    app.ui.close_dialog(id);
    assert_eq!(written.borrow().len(), 1);
    assert!(app.session.active().unwrap().is_dirty());

    // Replace writes it, and the next Save doesn't ask again.
    let id = menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap()["dialog"].as_u64().unwrap();
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(written.borrow().len(), 2);
    assert!(!app.session.active().unwrap().is_dirty());
    app.run("layer.new.layer", json!({})).unwrap();
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    assert_eq!(written.borrow().len(), 3);
    assert!(disk_prompt(&app).is_none());
}

#[test]
fn save_of_a_flat_file_asks_after_the_dialog_when_another_app_changed_it() {
    let ctx = egui::Context::default();
    // Lowercase: the Save dialog lowercases a typed extension by default.
    let path = "/pics/My image.jpg";
    let (mut app, open, written) = jpeg_app(None, false);
    let disk = on_disk(&mut app);
    changed_by_another_app(&disk, path);
    app.opened_from(path);
    app.run("image.adjustments.invert", json!({})).unwrap();
    changed_by_another_app(&disk, path);
    menus::invoke(&mut app, &ctx, "file.save", json!({})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo(path.into())));
    app.poll_file_dialog(&ctx, None);
    assert!(written.borrow().is_empty(), "the changed file is not replaced before the answer");
    let id = disk_prompt(&app).expect("Save asks first").id;
    crate::dialogs::confirm(&mut app, id).unwrap();
    // Then JPEG Options (FILE-215-19), whose answer is still recorded as a plain Save.
    assert!(written.borrow().is_empty());
    assert_eq!(app.jpeg_options.as_ref().map(|p| p.record), Some("file.save"));
    crate::jpeg_options_ui::confirm(&mut app, &ctx);
    assert_eq!(*written.borrow(), [path]);
    assert!(!app.session.active().unwrap().is_dirty());
    assert_eq!(app.session.journal.last(), Some(&("file.save".to_string(), json!({}))), "still recorded as a plain Save");
}

/// A replayed plain Save keeps its Targa answer in the step it records, as well as its JPEG one.
#[test]
fn a_replayed_plain_save_records_its_targa_answer() {
    let (mut app, _, written) = jpeg_app(Some("/pics/photo.tga"), false);
    app.replay_save("file.save", &json!({"tgaBits": 24})).unwrap();
    assert_eq!(*written.borrow(), ["/pics/photo.tga"]);
    assert_eq!(app.session.journal.last(), Some(&("file.save".to_string(), json!({"tgaBits": 24}))));
}

/// A script's JPEG Options answer survives the changed-on-disk prompt: Replace writes with it,
/// without asking JPEG Options.
#[test]
fn a_scripted_quality_survives_the_changed_on_disk_prompt() {
    let ctx = egui::Context::default();
    let path = "/pics/My image.jpg";
    let (mut app, open, written) = jpeg_app(None, false);
    let qualities: Rc<RefCell<Vec<Option<u8>>>> = Rc::default();
    let q = qualities.clone();
    app.services.export = Some(Box::new(move |_: &photocraft_doc::Document, _: &str, s: &crate::ExportSettings| {
        q.borrow_mut().push(s.jpeg_quality);
        Ok((b"out".to_vec(), Vec::new()))
    }));
    let disk = on_disk(&mut app);
    changed_by_another_app(&disk, path);
    app.opened_from(path);
    app.run("image.adjustments.invert", json!({})).unwrap();
    changed_by_another_app(&disk, path);
    menus::invoke(&mut app, &ctx, "file.save", json!({"quality": 5})).unwrap();
    app.poll_file_dialog(&ctx, None);
    answer(&open, Some(FileDialogAnswer::SaveTo(path.into())));
    app.poll_file_dialog(&ctx, None);
    assert!(written.borrow().is_empty(), "the changed file is not replaced before the answer");
    let id = disk_prompt(&app).expect("Save asks first").id;
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.jpeg_options.is_none(), "the script answered JPEG Options");
    assert_eq!(*written.borrow(), [path]);
    assert_eq!(qualities.borrow().last().copied().flatten(), Some(photocraft_engine::file_cmds::jpeg_quality(5.0)));
    // Recorded as a plain Save keeping the answer, so a replay writes the file it's played on.
    assert_eq!(app.session.journal.last(), Some(&("file.save".to_string(), json!({"quality": 5, "jpegFormat": "baseline"}))));
}

/// File › Save a Copy suggests "<name> copy", and with Preferences › File Handling › Do not
/// append "copy" to filename when saving a copy, the document's own name, as in Photoshop
/// (#8080). The preference goes through `prefs.set` like the Preferences dialog's checkbox.
#[test]
fn save_a_copy_appends_copy_unless_the_preference_says_not_to() {
    let ctx = egui::Context::default();
    for (off, name) in [(false, "My image copy.psd"), (true, "My image.psd")] {
        let want = std::path::Path::new("/pics").join(name).to_string_lossy().into_owned();
        let (mut app, open, _) = jpeg_app(Some("/pics/My image.JPEG"), false);
        app.run("prefs.set", json!({"path": "fileHandling.doNotAppendCopy", "value": off})).unwrap();
        menus::invoke(&mut app, &ctx, "file.saveACopy", json!({})).unwrap();
        app.poll_file_dialog(&ctx, None);
        assert!(
            matches!(open.borrow().as_slice(), [(FileDialogRequest::Save { suggested }, _)] if *suggested == want),
            "{off}: {:?}",
            open.borrow().first().map(|(r, _)| r.clone())
        );
    }
}
