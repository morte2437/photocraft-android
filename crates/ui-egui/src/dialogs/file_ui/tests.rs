use super::*;

#[test]
fn busy_driver_page_requests_a_retry_without_input() {
    let (mut app, ctx) = app();
    let mut fields = json!({"printer": "Test printer", "__printers": ["Test printer"], "paper": "letter"}).as_object().unwrap().clone();
    let (held_tx, held) = std::sync::mpsc::channel();
    let (release, released_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        photocraft_engine::print_cmds::with_spooler(|| {
            held_tx.send(()).unwrap();
            let _ = released_rx.recv_timeout(std::time::Duration::from_secs(5));
        });
    });
    held.recv().unwrap();
    let mut delay = std::time::Duration::MAX;
    for _ in 0..6 {
        let mut output = ctx.run_ui(Default::default(), |ui| print_body(&mut app, ui, &mut fields, 1));
        delay = output.viewport_output[&egui::ViewportId::ROOT].repaint_delay;
        output.textures_delta.clear();
    }
    let busy = fields.get("__pageError").cloned();
    let initialized = fields.get("__driverPageInitialized").cloned();
    let _ = release.send(());
    holder.join().unwrap();
    assert_eq!(busy, Some(json!(photocraft_engine::print_cmds::SPOOLER_BUSY)));
    assert_ne!(initialized, Some(json!(true)));
    assert!(delay <= std::time::Duration::from_millis(100), "busy driver repaint delay: {delay:?}");
    photocraft_engine::print_cmds::with_spooler(|| {
        ctx.run_ui(Default::default(), |ui| print_body(&mut app, ui, &mut fields, 1)).textures_delta.clear();
        assert_eq!(fields.get("__driverPageInitialized"), Some(&json!(true)));
        assert_ne!(fields.get("__pageError"), Some(&json!(photocraft_engine::print_cmds::SPOOLER_BUSY)));
    });
}

#[cfg(windows)]
#[test]
fn print_dialog_keeps_saved_settings_when_the_cached_printer_list_is_empty() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (mut app, ctx) = app();
    let available = Arc::new(AtomicBool::new(false));
    let listed = Arc::clone(&available);
    app.session.printer_source = Some(Arc::new(move || {
        if listed.load(Ordering::SeqCst) {
            vec![craft_print::Printer { name: "Saved printer".into(), default: true, two_sided: None, description: String::new() }]
        } else {
            Vec::new()
        }
    }));
    app.session.prefs.edit(|p| {
        p.dialogs.insert(photocraft_engine::print_cmds::REMEMBERED.into(), json!({"printer": "Saved printer", "printerSettings": "0011"}));
    });
    let mut lister = photocraft_engine::Session::new();
    lister.printer_source = app.session.printer_source.clone();
    let (held_tx, held) = std::sync::mpsc::channel();
    let (release, released_rx) = std::sync::mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        photocraft_engine::print_cmds::with_spooler(|| {
            assert!(photocraft_engine::print_cmds::rgb_printers(&lister).unwrap().is_empty());
            available.store(true, Ordering::SeqCst);
            held_tx.send(()).unwrap();
            let _ = released_rx.recv_timeout(std::time::Duration::from_secs(5));
        });
    });
    held.recv().unwrap();
    let id = open_print(&mut app);
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    let before_retry = (fields.get("printer").cloned(), fields.get("printerSettings").cloned());
    let pending = fields.get("__printerListingPending").cloned();
    let _ = release.send(());
    holder.join().unwrap();
    assert_eq!(pending, Some(json!(true)));
    assert_eq!(before_retry, (Some(json!("Saved printer")), Some(json!("0011"))));
    fields.insert("dryRun".into(), json!(true));
    photocraft_engine::print_cmds::with_spooler(|| {
        ctx.run_ui(Default::default(), |ui| print_body(&mut app, ui, &mut fields, id)).textures_delta.clear();
        assert_eq!(fields["printer"], "Saved printer");
        assert_eq!(fields["printerSettings"], "0011");
        assert!(!fields.contains_key("__printerListingPending"));
    });
}

#[test]
fn print_dialog_retries_an_uncached_printer_list_after_the_spooler_is_released() {
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    let (mut app, ctx) = app();
    let asked = Arc::new(AtomicUsize::new(0));
    let counted = Arc::clone(&asked);
    app.session.printer_source = Some(Arc::new(move || {
        counted.fetch_add(1, Ordering::SeqCst);
        vec![craft_print::Printer { name: "Retry printer".into(), default: true, two_sided: None, description: String::new() }]
    }));
    let (held_tx, held) = std::sync::mpsc::channel();
    let (release, released_rx) = std::sync::mpsc::channel::<()>();
    let released = Arc::new(AtomicBool::new(false));
    let done = Arc::clone(&released);
    let holder = std::thread::spawn(move || {
        photocraft_engine::print_cmds::with_spooler(|| {
            held_tx.send(()).unwrap();
            let _ = released_rx.recv_timeout(std::time::Duration::from_secs(5));
            done.store(true, Ordering::SeqCst);
        });
    });
    held.recv().unwrap();
    let id = open_print(&mut app);
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    fields.insert("dryRun".into(), json!(true));
    ctx.run_ui(Default::default(), |ui| print_body(&mut app, ui, &mut fields, id)).textures_delta.clear();
    let waited = released.load(Ordering::SeqCst);
    let calls_while_busy = asked.load(Ordering::SeqCst);
    let _ = release.send(());
    holder.join().unwrap();
    assert!(!waited, "opening or drawing Print waited for the spooler");
    assert_eq!(calls_while_busy, 0);
    assert_eq!(fields["__systemPrinters"], json!([]));
    photocraft_engine::print_cmds::with_spooler(|| {
        ctx.run_ui(Default::default(), |ui| print_body(&mut app, ui, &mut fields, id)).textures_delta.clear();
        assert_eq!(fields["__systemPrinters"], json!([{"name": "Retry printer", "default": true}]));
        assert_eq!(fields.get("__printers"), cfg!(windows).then(|| json!(["Retry printer"])).as_ref());
        assert!(!fields.contains_key("__printerListingPending"));
        assert!(asked.load(Ordering::SeqCst) > 0);
    });
}

#[test]
fn driver_calls_do_not_wait_while_the_spooler_is_busy() {
    use std::sync::atomic::{AtomicBool, Ordering};
    let (mut app, _) = app();
    let (held_tx, held) = std::sync::mpsc::channel();
    let (release, released_rx) = std::sync::mpsc::channel::<()>();
    let released = Arc::new(AtomicBool::new(false));
    let holder = {
        let released = Arc::clone(&released);
        std::thread::spawn(move || {
            photocraft_engine::print_cmds::with_spooler(|| {
                held_tx.send(()).unwrap();
                let _ = released_rx.recv_timeout(std::time::Duration::from_secs(5));
                released.store(true, Ordering::SeqCst);
            });
        })
    };
    held.recv().unwrap();
    let mut page = Map::new();
    page.insert("printer".into(), json!("Test printer"));
    page.insert("__driverPageInitialized".into(), json!(true));
    let answered = print::driver_page(&mut page);
    let mut turn = Map::new();
    turn.insert("printer".into(), json!("Test printer"));
    turn.insert("paper".into(), json!([595.28, 841.89]));
    turn_driver(&mut app, &mut turn, true);
    let waited = released.load(Ordering::SeqCst);
    let _ = release.send(());
    holder.join().unwrap();
    assert!(!waited, "a driver call waited for the spooler");
    assert!(!answered);
    assert!(page.get("__driverPageInitialized").is_none());
    assert_eq!(page["__pageError"], photocraft_engine::print_cmds::SPOOLER_BUSY);
    assert_eq!(turn["__pageError"], photocraft_engine::print_cmds::SPOOLER_BUSY);
    assert_eq!((turn.get("paper"), turn.get("printerSettings")), (Some(&json!([595.28, 841.89])), None));
}

/// Checks no-jump centering and nonaccumulating screen displacement against shared layout
/// across fit, percentage scaling, orientation and mirroring combinations.
#[test]
fn print_drag_uses_the_resolved_layout_and_screen_points_without_accumulation() {
    let (app, _) = app();
    let doc = &app.session.active().unwrap().doc;
    for fit in [false, true] {
        for mirror in [false, true] {
            for orientation in ["portrait", "landscape"] {
                let mut f = json!({"center": true, "scale": 175, "scaleToFit": fit,
                    "emulsionDown": mirror, "orientation": orientation})
                .as_object()
                .unwrap()
                .clone();
                let start = photocraft_engine::print_cmds::layout(doc, &Value::Object(f.clone()), "file.print").unwrap();
                let page = egui::Rect::from_min_size(egui::pos2(20.0, 30.0), egui::vec2(start.paper.0 as f32, start.paper.1 as f32));
                assert!(drag_print_position(&mut f, start, egui::Vec2::ZERO, 1.0));
                let manual = photocraft_engine::print_cmds::layout(doc, &Value::Object(f.clone()), "file.print").unwrap();
                assert!((manual.rect.0 - start.rect.0).abs() < 1e-9);
                assert!((manual.rect.1 - start.rect.1).abs() < 1e-9);
                assert_eq!(manual.scale, start.scale);
                for k in [0.25, 0.5, 1.0, 2.0] {
                    let delta = egui::vec2(-90.0, 45.0) * k;
                    assert!(drag_print_position(&mut f, start, delta, k));
                    let moved = photocraft_engine::print_cmds::layout(doc, &Value::Object(f.clone()), "file.print").unwrap();
                    let old_img = print_preview_image(start, page, k);
                    let new_img = print_preview_image(moved, page, k);
                    assert!((new_img.min - old_img.min - delta).length() < 0.001);
                    assert_eq!((moved.rect.2, moved.rect.3, moved.scale), (start.rect.2, start.rect.3, start.scale));
                    let once = f.clone();
                    assert!(drag_print_position(&mut f, start, delta, k));
                    assert_eq!(f, once, "total drag displacement must not accumulate");
                }
            }
        }
    }
}

/// Checks rejected pointer geometry leaves fields untouched and extreme finite drags
/// clamp both inch offsets to the same safety limits as numeric positioning.
#[test]
fn print_drag_rejects_nonfinite_input_and_bounds_offsets() {
    let (app, _) = app();
    let doc = &app.session.active().unwrap().doc;
    let start = photocraft_engine::print_cmds::layout(doc, &json!({}), "file.print").unwrap();
    let mut f = Map::new();
    for k in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert!(!drag_print_position(&mut f, start, egui::vec2(10.0, 10.0), k));
        assert!(f.is_empty());
    }
    assert!(!drag_print_position(&mut f, start, egui::vec2(f32::NAN, 0.0), 1.0));
    assert!(f.is_empty());
    assert!(drag_print_position(&mut f, start, egui::vec2(-f32::MAX, f32::MAX), 1.0));
    assert_eq!(f["left"], -photocraft_engine::print_cmds::MAX_POSITION_INCHES);
    assert_eq!(f["top"], photocraft_engine::print_cmds::MAX_POSITION_INCHES);
}

fn app() -> (PhotocraftApp, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.printer_source =
        Some(Arc::new(|| vec![craft_print::Printer { name: "Test printer".into(), default: true, two_sided: None, description: String::new() }]));
    app.run("file.new", json!({"width": 120, "height": 80, "name": "web"})).unwrap();
    (app, egui::Context::default())
}

#[test]
fn droplet_folder_follows_the_document_or_uses_a_safe_fallback() {
    let (mut app, _) = app();
    let fallback = droplet_default_dir(&app);
    #[cfg(target_os = "macos")]
    if std::env::var_os("HOME").is_some() {
        assert!(std::path::Path::new(&fallback).is_absolute());
    }
    #[cfg(not(target_os = "macos"))]
    assert_eq!(fallback, ".");
    app.session.active_mut().unwrap().path = Some("/tmp/photocraft/drawing.pcraft".into());
    assert_eq!(droplet_default_dir(&app), "/tmp/photocraft");
}

#[test]
fn save_for_web_dialog_runs_the_command() {
    let (mut app, ctx) = app();
    let r = crate::menus::invoke(&mut app, &ctx, "file.export.saveForWebLegacy", json!({})).unwrap();
    let id = r["dialog"].as_u64().unwrap();
    let dir = std::env::temp_dir().join(format!("pc-webui-{}", std::process::id()));
    let path = dir.join("web.gif").to_string_lossy().into_owned();
    let d = app.ui.dialog_mut(id).unwrap();
    d.fields.insert("format".into(), json!("gif"));
    d.fields.insert("path".into(), json!(path));
    let r = crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(r["files"][0], json!(path));
    assert!(std::fs::read(&path).unwrap().starts_with(b"GIF89a"));
    // The next dialog starts from those settings.
    let r = crate::menus::invoke(&mut app, &ctx, "file.export.saveForWebLegacy", json!({})).unwrap();
    let d = app.ui.dialog_mut(r["dialog"].as_u64().unwrap()).unwrap();
    assert_eq!(d.fields["format"], "gif");
    assert!(!d.fields.contains_key("path"));
    // WebP, with the dialog's own defaults (lossy at quality 60).
    let id = r["dialog"].as_u64().unwrap();
    let d = app.ui.dialog_mut(id).unwrap();
    let path = dir.join("web.webp").to_string_lossy().into_owned();
    d.fields.insert("format".into(), json!("webp"));
    d.fields.insert("path".into(), json!(path));
    crate::dialogs::confirm(&mut app, id).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(&bytes[8..12], b"WEBP");
    assert!(bytes.windows(4).any(|w| w == b"VP8 "), "lossy by default");
}

/// A fresh session's dialog seeds from the preferences: the last export before a restart is
/// remembered there (`web_cmds::remember_web_settings`), as `file_menu` is not saved.
#[test]
fn the_save_for_web_dialog_seeds_from_the_remembered_preferences() {
    let (mut app, _) = app();
    app.session.prefs.edit(|p| {
        p.dialogs.insert(WEB_COMMAND.into(), json!({"format": "png8", "colors": 64}));
    });
    let id = open_web(&mut app);
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(f["format"], "png8");
    assert_eq!(f["colors"], 64);
}

#[test]
fn save_for_web_starts_at_the_current_document_size() {
    let (mut app, _) = app();
    let path = std::env::temp_dir().join(format!("pc-webui-size-{}.png", std::process::id())).to_string_lossy().into_owned();
    app.run("file.export.saveForWebLegacy", json!({"path": path, "format": "png24", "width": 60, "height": 40, "percent": 50})).unwrap();
    app.run("file.new", json!({"width": 240, "height": 160, "name": "next"})).unwrap();
    let id = open_web(&mut app);
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(f["format"], "png24");
    assert_eq!(f["percent"], 100.0);
    assert!(!f.contains_key("width"));
    assert!(!f.contains_key("height"));
}

/// Save for Web's Image Size on a 120 x 80 document, as a PNG-24 (no other number fields).
fn web_size_dialog(app: PhotocraftApp) -> (egui_kittest::Harness<'static, PhotocraftApp>, u64) {
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1100.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                crate::dialogs::show(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    let id = open_web(h.state_mut());
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("format".into(), json!("png24"));
    h.run_steps(5);
    (h, id)
}

/// Types `text` into the number field now showing `value`, and leaves it with Tab.
fn type_into_field(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, value: f64, text: &str) {
    use egui_kittest::kittest::{NodeT, Queryable};
    let field = h
        .query_all_by_role(egui::accesskit::Role::SpinButton)
        .find(|n| n.accesskit_node().numeric_value() == Some(value))
        .map(|n| n.rect())
        .expect("a field showing the value");
    h.hover_at(field.center());
    h.run_steps(1);
    h.drag_at(field.center());
    h.run_steps(1);
    h.drop_at(field.center());
    h.run_steps(2);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

/// #6388: Save for Web's Image Size has Width and Height linked in proportion, as in
/// Photoshop. A typed width sets the height, the file comes out at that size, and the
/// document itself is never resized.
#[test]
fn save_for_web_width_keeps_proportions_and_sizes_the_file() {
    let (app, _) = app();
    let (mut h, id) = web_size_dialog(app);
    let revision = h.state().session.active().unwrap().revision;
    type_into_field(&mut h, 120.0, "60");
    let f = h.state().ui.dialogs[0].fields.clone();
    assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(60.0), Some(40.0)), "{f:?}");
    // Unlinked, the height changes alone; linked again, it follows the width.
    {
        use egui_kittest::kittest::Queryable;
        h.get_by_label("Constrain proportions").click();
        h.run_steps(3);
        type_into_field(&mut h, 40.0, "50");
        let f = h.state().ui.dialogs[0].fields.clone();
        assert_eq!((f["width"].as_f64(), f["height"].as_f64()), (Some(60.0), Some(50.0)), "{f:?}");
        h.get_by_label("Constrain proportions").click();
        h.run_steps(3);
    }
    let f = h.state().ui.dialogs[0].fields.clone();
    assert_eq!((f["width"].as_f64(), f["height"].as_f64(), f.get("__constrain")), (Some(60.0), Some(40.0), Some(&json!(true))), "{f:?}");
    let path = std::env::temp_dir().join(format!("pc-webui-wh-{}.png", std::process::id())).to_string_lossy().into_owned();
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("path".into(), json!(path));
    let r = crate::dialogs::confirm(h.state_mut(), id).unwrap();
    assert_eq!((r["width"].as_u64(), r["height"].as_u64()), (Some(60), Some(40)), "{r}");
    let st = h.state().session.active().unwrap();
    assert_eq!((st.revision, st.doc.size), (revision, photocraft_doc::Size::new(120, 80)));
}

/// Reopened on another document, Save for Web starts at that document's full size again
/// (#1857) but keeps the resampling method, like the other encoding choices.
#[test]
fn save_for_web_reopens_at_full_size_with_the_resample_method() {
    let (app, _) = app();
    let (mut h, id) = web_size_dialog(app);
    type_into_field(&mut h, 120.0, "60");
    let path = std::env::temp_dir().join(format!("pc-webui-reopen-{}.png", std::process::id())).to_string_lossy().into_owned();
    let d = h.state_mut().ui.dialog_mut(id).unwrap();
    d.fields.insert("resample".into(), json!("nearest"));
    d.fields.insert("path".into(), json!(path));
    crate::dialogs::confirm(h.state_mut(), id).unwrap();
    h.state_mut().run("file.new", json!({"width": 240, "height": 160, "name": "next"})).unwrap();
    let id = open_web(h.state_mut());
    h.run_steps(3);
    let f = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    assert_eq!(f["resample"], "nearest");
    assert!(!f.contains_key("width") && !f.contains_key("height"), "{f:?}");
    let p = web_preview::dimensions(&h.state().session.active().unwrap().doc, &params(&f)).unwrap();
    assert_eq!(p, (240.0, 160.0));
}

#[test]
fn save_for_web_downloads_images_and_html_through_the_platform_writer() {
    let (mut app, _) = app();
    let downloads = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let captured = downloads.clone();
    app.services.write = Some(Box::new(move |path, bytes| {
        captured.borrow_mut().push((path.to_string(), bytes.to_vec()));
        Ok(())
    }));
    download_for_web(&mut app, json!({"path": "web.png", "format": "png24"})).unwrap();
    assert_eq!(downloads.borrow()[0].0, "web.png");
    assert!(downloads.borrow()[0].1.starts_with(b"\x89PNG\r\n\x1a\n"));
    downloads.borrow_mut().clear();

    app.run("slice.new", json!({"rect": [0, 0, 60, 80], "name": "left"})).unwrap();
    download_for_web(&mut app, json!({"dir": ".", "format": "png24", "html": true, "imagesFolder": "images"})).unwrap();
    let files = downloads.borrow();
    let html = std::str::from_utf8(&files.iter().find(|(path, _)| path.ends_with(".html")).unwrap().1).unwrap();
    assert!(!html.contains("images/"), "downloaded files are siblings, not an images directory");
    for (path, bytes) in files.iter().filter(|(path, _)| !path.ends_with(".html")) {
        let name = std::path::Path::new(path).file_name().unwrap().to_str().unwrap();
        assert!(html.contains(&format!("src=\"{name}\"")), "missing downloaded image {name}");
        assert!(!bytes.is_empty());
    }
    assert!(files.iter().any(|(path, _)| path.ends_with("spacer.gif")));
    drop(files);

    app.services.write = Some(Box::new(|_, _| Err("download failed".into())));
    assert_eq!(download_for_web(&mut app, json!({"path": "fail.png", "format": "png24"})).unwrap_err(), "download failed");
    app.services.write = None;
    assert_eq!(download_for_web(&mut app, json!({"path": "missing.png"})).unwrap_err(), "no download service");
}

#[test]
fn save_for_web_authorizes_before_opening_a_picker() {
    fn deny(_: &str, _: &Value) -> photocraft_engine::Result<()> {
        Err(photocraft_engine::EngineError::Other("export denied".into()))
    }
    for explicit_path in [false, true] {
        for synthetic_input in [false, true] {
            let (mut app, ctx) = app();
            let id = open_web(&mut app);
            if explicit_path {
                app.ui.dialog_mut(id).unwrap().fields.insert("path".into(), json!("denied.png"));
            }
            app.services.automation_authorize = Some(deny);
            app.services.file_dialog = Some(Box::new(|_, _, _| panic!("denied export must not show a picker")));
            app.services.write = Some(Box::new(|_, _| panic!("denied export must not write")));
            let error = if synthetic_input {
                app.automation_input = true;
                crate::dialogs::confirm(&mut app, id).unwrap_err()
            } else {
                let (request, _) = crate::control::ControlRequest::new("ui.dialog.confirm", json!({"dialog": id}));
                let crate::control::Outcome::Done(reply) = crate::control::handle(&mut app, &ctx, &request) else { panic!("expected immediate denial") };
                assert_eq!(reply["ok"], false);
                reply["error"].as_str().unwrap().to_string()
            };
            assert_eq!(error, "export denied");
            assert!(!app.file_dialog_open());
        }
    }
}

#[test]
fn save_for_web_authorizes_generated_palette_before_opening_a_picker() {
    fn deny_palette(_: &str, p: &Value) -> photocraft_engine::Result<()> {
        if p.get("colorTable").is_some() { Err(photocraft_engine::EngineError::Other("palette denied".into())) } else { Ok(()) }
    }
    for synthetic_input in [false, true] {
        let (mut app, _) = app();
        let id = open_web(&mut app);
        let fields = &mut app.ui.dialog_mut(id).unwrap().fields;
        fields.insert("format".into(), json!("gif"));
        assert!(!fields.contains_key("colorTable"));
        if synthetic_input {
            app.automation_input = true;
            app.services.automation_command = Some(Box::new(|cmd, p| deny_palette(cmd, p).map_err(|e| e.to_string())));
        } else {
            app.session.authorize = Some(deny_palette);
        }
        app.services.file_dialog = Some(Box::new(|_, _, _| panic!("denied palette must not show a picker")));
        app.services.write = Some(Box::new(|_, _| panic!("denied palette must not write")));
        assert_eq!(crate::dialogs::confirm(&mut app, id).unwrap_err(), "palette denied");
        assert!(!app.file_dialog_open());
    }
}

#[test]
fn web_preview_grows_with_resize_and_accepts_zoom_and_pan() {
    let (mut app, _) = app();
    let id = open_web(&mut app);
    let revision = app.session.active().unwrap().revision;
    app.ui.dialog_mut(id).unwrap().fields.insert("__view".into(), json!("optimized"));
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1600.0, 1100.0)).build_ui_state(
        |ui, app| {
            crate::color_picker_ui::widget_tick(app, ui.ctx());
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                crate::dialogs::show(app, ui.ctx());
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(5);
    let pane = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
        let r = &h.state().ui.dialogs[0].fields["__previewRects"][0];
        egui::Rect::from_min_max(
            egui::pos2(r[0].as_f64().unwrap() as f32, r[1].as_f64().unwrap() as f32),
            egui::pos2(r[2].as_f64().unwrap() as f32, r[3].as_f64().unwrap() as f32),
        )
    };
    let before = pane(&h);
    let rects: Vec<egui::Rect> = h.ctx.data(|m| m.get_temp(egui::Id::unique("pc-dialog-rects"))).unwrap();
    let corner = rects[0].max - egui::vec2(14.0, 14.0);
    h.event(egui::Event::PointerMoved(corner));
    h.step();
    h.event(egui::Event::PointerButton { pos: corner, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    let end = corner + egui::vec2(180.0, 100.0);
    h.event(egui::Event::PointerMoved(end));
    h.step();
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(4);
    let after = pane(&h);
    assert!(after.width() > before.width() + 100.0, "before {before:?}, after {after:?}, corner {corner:?}");
    assert!(after.height() > before.height() + 50.0);
    // Zoom in enough that the small fixture can pan, then drag the real pane.
    h.state_mut().ui.dialogs[0].fields.insert("__zoom".into(), json!(16.0));
    h.run_steps(3);
    let at = pane(&h).center();
    h.event(egui::Event::PointerMoved(at));
    h.step();
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.step();
    let end = at + egui::vec2(35.0, 20.0);
    h.event(egui::Event::PointerMoved(end));
    h.step();
    h.event(egui::Event::PointerButton { pos: end, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert_ne!(h.state().ui.dialogs[0].fields["__center"], json!([0.5, 0.5]));
    assert_eq!(h.state().session.active().unwrap().revision, revision);
    // A colour table swatch opens the Color Picker over the dialog; Enter confirms the picker
    // only: it must not export and close the parent dialog.
    h.state_mut().ui.dialogs[0].fields.insert("format".into(), json!("gif"));
    h.run_steps(4);
    use egui_kittest::kittest::Queryable;
    h.get_by_role(egui::accesskit::Role::ColorWell).click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.len(), 2);
    assert!(crate::color_picker_ui::owns(&h.state().ui.dialogs[1].fields));
    h.key_press(egui::Key::Enter);
    h.run_steps(3);
    assert_eq!(h.state().ui.dialogs.len(), 1);
}

/// #10675: moving the pointer over Save for Web repaints it, and each repaint must lay it out the
/// same way. At a fractional display scale the previews came out a fraction of a pixel wider than
/// the room they were given, so the dialog grew a little on every repaint and the fitted zoom
/// with it.
#[test]
fn web_preview_keeps_its_size_and_zoom_while_the_pointer_moves() {
    let mut drifted = Vec::new();
    for ppp in [1.0, 1.25, 1.5, 1.75, 2.25] {
        for screen in [egui::vec2(1600.0, 1100.0), egui::vec2(1097.0, 617.0), egui::vec2(1280.0, 720.0)] {
            for view in ["2up", "4up", "original"] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                app.run("file.new", json!({"width": 1980, "height": 1200})).unwrap();
                let id = open_web(&mut app);
                app.ui.dialog_mut(id).unwrap().fields.insert("__view".into(), json!(view));
                let mut h = egui_kittest::Harness::builder().with_size(screen).with_pixels_per_point(ppp).build_ui_state(
                    |ui, app: &mut PhotocraftApp| {
                        if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                            crate::dialogs::show(app, ui.ctx());
                        }
                    },
                    app,
                );
                PhotocraftApp::setup_context(&h.ctx, Default::default());
                h.run_steps(8);
                let layout = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
                    let f = &h.state().ui.dialogs[0].fields;
                    let rects: Vec<egui::Rect> = h.ctx.data(|m| m.get_temp(egui::Id::unique("pc-dialog-rects"))).unwrap();
                    (rects[0], f["__previewRects"].clone(), f["__effectiveZoom"].clone())
                };
                let before = layout(&h);
                let pane = &before.1[0];
                let at = egui::pos2(pane[0].as_f64().unwrap() as f32 + 20.0, pane[1].as_f64().unwrap() as f32 + 20.0);
                for i in 0..30 {
                    h.event(egui::Event::PointerMoved(at + egui::vec2(i as f32 * 3.0, i as f32)));
                    h.step();
                }
                let after = layout(&h);
                if after != before {
                    drifted.push(format!("{view} at {ppp}x on {screen:?}: {before:?} -> {after:?}"));
                }
            }
        }
    }
    assert!(drifted.is_empty(), "the dialog or its zoom changed while the pointer moved: {drifted:#?}");
}

#[test]
fn palette_draft_survives_navigation_and_resets_when_colors_change() {
    let (app, _) = app();
    let mut f = serde_json::from_value::<Map<String, Value>>(json!({"format": "gif", "colors": 16})).unwrap();
    web_preview::prepare_table(&app, &mut f).unwrap();
    let table = f["colorTable"].clone();
    f.insert("colorTableEdits".into(), json!({"0": "#12ab34"}));
    f.insert("__zoom".into(), json!(2.0));
    f.insert("__center".into(), json!([0.25, 0.75]));
    web_preview::prepare_table(&app, &mut f).unwrap();
    assert_eq!(f["colorTable"], table);
    assert_eq!(f["colorTableEdits"]["0"], "#12ab34");
    f.insert("metadata".into(), json!("none"));
    f.insert("interlaced".into(), json!(true));
    f.insert("dither".into(), json!("pattern"));
    web_preview::prepare_table(&app, &mut f).unwrap();
    assert_eq!(f["colorTableEdits"]["0"], "#12ab34", "non-palette settings must retain edits");
    f.insert("colors".into(), json!(8));
    web_preview::prepare_table(&app, &mut f).unwrap();
    assert!(!f.contains_key("colorTableEdits"));
    assert!(f["colorTable"].as_array().unwrap().len() <= 8);
}

#[test]
fn print_dialog_and_toggles() {
    let (mut app, ctx) = app();
    let r = crate::menus::invoke(&mut app, &ctx, "file.print", json!({})).unwrap();
    let id = r["dialog"].as_u64().unwrap();
    let out = std::env::temp_dir().join(format!("pc-printui-{}.pdf", std::process::id())).to_string_lossy().into_owned();
    app.ui.dialog_mut(id).unwrap().fields.insert("output".into(), json!(out));
    let r = crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(r["sent"], false);
    assert!(std::fs::read(&out).unwrap().starts_with(b"%PDF"));
    assert_eq!(checked(&app, "view.lockSlices"), Some(false));
    crate::menus::invoke(&mut app, &ctx, "view.lockSlices", json!({})).unwrap();
    assert_eq!(checked(&app, "view.lockSlices"), Some(true));
    // Generator is opt-in (Preferences ▸ Plug-ins), as in Photoshop: the menu item is
    // unavailable until it is enabled.
    assert!(crate::menus::invoke(&mut app, &ctx, "file.generate.imageAssets", json!({})).is_err());
    app.run("prefs.set", json!({"path": "plugIns.generatorEnabled", "value": true})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "file.generate.imageAssets", json!({})).unwrap();
    assert_eq!(checked(&app, "file.generate.imageAssets"), Some(true));
    // Form dialogs open for the rest.
    for id in ["file.automate.contactSheetII", "file.scripts.statistics", "file.scripts.scriptEventsManager", "file.package"] {
        let r = crate::menus::invoke(&mut app, &ctx, id, json!({})).unwrap();
        assert!(r["dialog"].is_u64(), "{id}");
    }
}

#[test]
fn printer_menu_lists_the_spooler_printers_after_the_default() {
    let list = json!([{"name": "HP_OfficeJet_3830_series", "default": true}, {"name": "DNP_DP_DS620", "default": false}]);
    let o = printer_options(Some(&list), "");
    let keys: Vec<&str> = o.iter().map(|p| p.0.as_str()).collect();
    assert_eq!(keys, ["", "HP_OfficeJet_3830_series", "DNP_DP_DS620"]);
    assert!(o[0].1.ends_with("(HP OfficeJet 3830 series)"), "{}", o[0].1);
    assert_eq!(o[2].1, "DNP DP DS620");
    // A remembered printer that's gone stays selectable; no list still offers the default.
    let o = printer_options(Some(&list), "Old_Queue");
    assert_eq!(o.last().unwrap().0, "Old_Queue");
    assert_eq!(printer_options(None, "").len(), 1);
    // The dialog asks the engine, and the list never reaches file.print's params.
    let (mut app, ctx) = app();
    let id = photocraft_engine::print_cmds::with_spooler(|| crate::menus::invoke(&mut app, &ctx, "file.print", json!({})).unwrap()["dialog"].as_u64().unwrap());
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!(f["__systemPrinters"], json!([{"name": "Test printer", "default": true}]));
    assert_eq!(f.get("__printers"), cfg!(windows).then(|| json!(["Test printer"])).as_ref());
    if cfg!(windows) {
        assert_eq!(f["printer"], "Test printer");
    }
    assert!(params(f).get("__systemPrinters").is_none());
    assert!(params(f).get("__printers").is_none());
    assert_eq!(photocraft_engine::print_cmds::system_printer_listings(), 0, "a test asked the system spooler for its printers");
}

/// An output path or a preview typed and confirmed in one frame doesn't take the driver's area.
#[test]
fn the_printable_area_goes_only_with_a_print_to_the_printer() {
    let area = json!([0.0, 0.0, 842.29, 1191.14]);
    let mut f = Map::new();
    f.insert("printable".into(), area.clone());
    assert_eq!(print_params(&f)["printable"], area);
    f.insert("output".into(), json!("/tmp/out.pdf"));
    assert!(print_params(&f).get("printable").is_none());
    f.insert("output".into(), json!(""));
    f.insert("dryRun".into(), json!(true));
    assert!(print_params(&f).get("printable").is_none());
}

/// #7410: File › Print opens with the settings of the last print, also those remembered from
/// an earlier session (the preferences), before anything was printed in this one.
#[test]
fn print_dialog_opens_with_the_settings_remembered_from_an_earlier_session() {
    let (mut app, _) = app();
    app.session.prefs.edit(|p| {
        p.dialogs
            .insert(photocraft_engine::print_cmds::REMEMBERED.into(), json!({"paper": "a5", "scale": 50, "orientation": "landscape", "intent": "perceptual"}));
    });
    let id = open_print(&mut app);
    let f = &app.ui.dialog_mut(id).unwrap().fields;
    assert_eq!((&f["paper"], &f["scale"], &f["orientation"], &f["intent"]), (&json!("a5"), &json!(50), &json!("landscape"), &json!("perceptual")));
    assert_eq!(f["copies"], 1, "what wasn't remembered keeps its default");
}

/// #5328: Save as PDF has Browse…, which fills the field from a Save dialog (suggesting the
/// document's name as a PDF) rather than needing a typed path.
#[test]
fn save_as_pdf_browse_fills_the_output_from_a_save_dialog() {
    use crate::file_dialog::{FileDialogAnswer, FileDialogRequest};
    use egui_kittest::{Harness, kittest::Queryable};
    let out = std::env::temp_dir().join(format!("pc-printui-browse-{}", std::process::id())).to_string_lossy().into_owned();
    let (show, asked) = crate::file_dialog::fake(vec![Some(FileDialogAnswer::SaveTo(out.clone()))]);
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services { file_dialog: Some(show), ..Default::default() });
        app.session.printer_source =
            Some(Arc::new(|| vec![craft_print::Printer { name: "Test printer".into(), default: true, two_sided: None, description: String::new() }]));
        app
    });
    h.state_mut().run("file.new", json!({"width": 120, "height": 80, "name": "poster"})).unwrap();
    let id = open_print(h.state_mut());
    h.run_steps(3);
    h.get_by_label("Browse…").click();
    h.run_steps(4);
    assert!(matches!(asked.borrow().as_slice(), [FileDialogRequest::Save { suggested }] if suggested.ends_with("poster.pdf")), "{:?}", asked.borrow());
    // A name chosen without an extension gets the suggested one.
    assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields["output"], json!(format!("{out}.pdf")));
}

#[test]
fn print_dialog_does_not_inherit_scripted_send_override() {
    for send in [false, true] {
        let (mut app, ctx) = app();
        let out = std::env::temp_dir().join(format!("pc-printui-state-{}-{send}.pdf", std::process::id())).to_string_lossy().into_owned();
        app.session.execute("file.print", json!({"output": out, "send": send, "dryRun": true})).unwrap();
        assert!(app.session.file_menu.last_print.as_ref().unwrap().get("send").is_none());
        let r = crate::menus::invoke(&mut app, &ctx, "file.print", json!({})).unwrap();
        let id = r["dialog"].as_u64().unwrap();
        assert!(app.ui.dialog_mut(id).unwrap().fields.get("send").is_none());
        std::fs::remove_file(out).unwrap();
    }
}

/// The Orientation buttons show the driver paper's shape: landscape when it is wider than tall.
#[test]
fn orientation_follows_the_driver_paper() {
    let with = |paper: Value| {
        let mut f = Map::new();
        f.insert("paper".into(), paper);
        f
    };
    assert!(driver_paper_is_wide(&with(json!([1191.14, 842.29]))));
    assert!(!driver_paper_is_wide(&with(json!([842.29, 1191.14]))));
    assert!(!driver_paper_is_wide(&with(json!("a3"))), "a named paper is not a driver page");
    assert!(!driver_paper_is_wide(&Map::new()));
}

/// Turning a printer whose driver can't be asked keeps its settings and says why on the dialog.
#[test]
fn turning_without_a_driver_keeps_the_settings_and_shows_the_error() {
    // Valid driver settings (a minimal DEVMODEW: 220 bytes, no private part), so the command
    // gets past reading them and fails where it asks the (missing) printer for its page.
    let mut dm = vec![0u8; 220];
    dm[68..70].copy_from_slice(&220u16.to_le_bytes());
    let hex: String = dm.iter().map(|b| format!("{b:02x}")).collect();
    assert!(craft_print::rgb::DevMode::from_hex(&hex).is_ok());
    let (mut app, _) = app();
    let mut f = Map::new();
    f.insert("printer".into(), json!("photocraft-no-such-printer"));
    f.insert("printerSettings".into(), json!(hex));
    f.insert("paper".into(), json!([595.28, 841.89]));
    turn_driver(&mut app, &mut f, true);
    assert_eq!(f.get("printerSettings"), Some(&json!(hex)));
    assert_eq!(f.get("paper"), Some(&json!([595.28, 841.89])));
    let error = f.get("__pageError").and_then(Value::as_str).unwrap_or_default();
    // Reading bad settings fails with "printer settings are too short / damaged"; this must not.
    assert!(!error.is_empty() && !error.contains("printer settings are"), "failed at the printer, not at reading the settings: {error}");
}

/// A turn takes the settings and the paper from the same answer, never one without the other.
#[test]
fn a_turn_takes_settings_and_paper_together() {
    let fields = || {
        let mut f = Map::new();
        f.insert("printerSettings".into(), json!("old"));
        f.insert("paper".into(), json!([595.28, 841.89]));
        f.insert("__pageError".into(), json!("earlier"));
        f
    };
    let mut f = fields();
    take_turned_page(&mut f, &json!({"printerSettings": "new", "paper": [841.89, 595.28]}));
    assert_eq!((f.get("printerSettings"), f.get("paper")), (Some(&json!("new")), Some(&json!([841.89, 595.28]))));
    assert_eq!(f.get("orientation"), Some(&json!("portrait")), "the driver paper is already turned");
    assert!(f.get("__pageError").is_none());
    for partial in [json!({"printerSettings": "new"}), json!({"paper": [841.89, 595.28]}), json!({"printerSettings": "new", "paper": [841.89]})] {
        let mut f = fields();
        take_turned_page(&mut f, &partial);
        assert_eq!((f.get("printerSettings"), f.get("paper")), (Some(&json!("old")), Some(&json!([595.28, 841.89]))), "{partial}");
        assert!(f.get("__pageError").is_some(), "{partial}");
    }
}

/// #7782: the driver's printable area turns with the paper. Left portrait on a landscape
/// sheet, Scale to Fit Media refused the print instead of fitting it.
#[test]
fn a_turn_takes_the_printable_area_with_the_paper() {
    let (mut app, ctx) = app();
    crate::menus::invoke(&mut app, &ctx, "file.new", json!({"width": 500, "height": 1000})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let mut f = Map::new();
    f.insert("scaleToFit".into(), json!(true));
    f.insert("dryRun".into(), json!(false));
    f.insert("printerSettings".into(), json!("old"));
    f.insert("paper".into(), json!([842.29, 1191.14]));
    f.insert("printable".into(), json!([12.0, 12.0, 818.29, 1167.14]));
    f.insert("orientation".into(), json!("portrait"));
    take_turned_page(&mut f, &json!({"printerSettings": "new", "paper": [1191.14, 842.29], "printable": [12.0, 12.0, 1167.14, 818.29]}));
    let l = photocraft_engine::print_cmds::layout(&doc, &print_params(&f), "file.print");
    assert_eq!(l.map(|l| l.paper).map_err(|e| e.to_string()), Ok((1191.14, 842.29)));
    assert_eq!(f.get("printable"), Some(&json!([12.0, 12.0, 1167.14, 818.29])));
    // An answer without a usable area drops the old one: the fixed margin, not a stale area.
    for area in [Value::Null, json!([12.0, 12.0, 818.29]), json!("x")] {
        f.insert("printable".into(), json!([12.0, 12.0, 1167.14, 818.29]));
        take_turned_page(&mut f, &json!({"printerSettings": "newer", "paper": [842.29, 1191.14], "printable": area}));
        assert!(f.get("printable").is_none(), "{area}");
    }
}
