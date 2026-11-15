//! A Home screen recent-file row is File › Open Recent's item, so synthetic input that clicks it
//! is judged as that command: a client confined to its automation roots can't plant a recent
//! file outside them and click it open. A user's own click still opens it.

use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::{PhotocraftApp, Services};

/// The desktop policy's verdict on file commands: only the in-memory ones run.
fn deny_file(id: &str, _: &Value) -> photocraft_engine::Result<()> {
    if id.starts_with("file.") && !matches!(id, "file.new" | "file.close" | "file.closeAll" | "file.closeOthers") {
        Err(photocraft_engine::EngineError::Other(format!("automation command `{id}` is disabled")))
    } else {
        Ok(())
    }
}

/// The Home screen with `path` as its one recent file, under the desktop's automation policy.
fn home(path: &str) -> Harness<'static, (PhotocraftApp, bool)> {
    let services = Services {
        automation_authorize: Some(deny_file),
        automation_command: Some(Box::new(|id, p| deny_file(id, p).map_err(|e| e.to_string()))),
        import: Some(Box::new(|name, bytes, opts| {
            photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
        })),
        ..Default::default()
    };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    app.background_jobs = false;
    app.push_recent(path);
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 760.0)).build_ui_state(
        |ui, (app, ready): &mut (PhotocraftApp, bool)| {
            if *ready {
                egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| crate::canvas::document_area(app, ui));
            }
        },
        (app, false),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.state_mut().1 = true;
    h.run_steps(3);
    h
}

/// Click the row, as synthetic input when `automation` is set.
fn click(h: &mut Harness<'static, (PhotocraftApp, bool)>, automation: bool) {
    h.get_by_label("outsideroot.png").click();
    h.state_mut().0.automation_input = automation;
    h.step();
    h.state_mut().0.automation_input = false;
    h.run_steps(2);
}

/// A synthetic click on a Home recent row outside the automation roots is refused and reported
/// as `file.openRecent.0`, like the menu item; the user's own click opens the file.
#[test]
fn synthetic_click_on_a_home_recent_row_is_judged_as_open_recent() {
    let dir = std::env::temp_dir().join(format!("pc-home-recent-gate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("outsideroot.png");
    let image = photocraft_codecs::Image::from_u8(4, 4, photocraft_codecs::ChannelLayout::Rgba, [10u8, 200, 30, 255].repeat(16)).unwrap();
    std::fs::write(&file, photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
    let path = file.to_string_lossy().into_owned();
    let paths = |h: &Harness<'static, (PhotocraftApp, bool)>| h.state().0.session.documents().iter().map(|d| d.path.clone()).collect::<Vec<_>>();

    // The menu route to the same file is refused for synthetic input.
    let mut h = home(&path);
    let ctx = h.ctx.clone();
    h.state_mut().0.automation_input = true;
    assert!(crate::menus::invoke(&mut h.state_mut().0, &ctx, "file.openRecent.0", json!({})).is_err());
    h.state_mut().0.automation_input = false;

    click(&mut h, true);
    let status = h.state().0.ui.status.clone();
    assert!(paths(&h).is_empty(), "a synthetic click on the Home row opened {:?}", paths(&h));
    assert!(status.contains("file.openRecent.0"), "the refusal is reported: {status:?}");

    // The user's own click opens it.
    let mut h = home(&path);
    click(&mut h, false);
    let status = h.state().0.ui.status.clone();
    assert_eq!(paths(&h), [Some(path.clone())], "status: {status:?}");
    drop(h);
    let _ = std::fs::remove_dir_all(&dir);
}
