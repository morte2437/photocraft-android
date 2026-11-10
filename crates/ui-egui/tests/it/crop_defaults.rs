//! #10894: two Crop defaults as in the reference application. The overlay shows Always by
//! default, and Reset Tool brings back Always and Rule of Thirds. Fill goes back to its default
//! entry when another document that was already open becomes active, and keeps Content-Aware
//! Fill otherwise.

use photocraft_ui_egui::control::{ControlRequest, handle};
use photocraft_ui_egui::crop_overlay::{CropOverlay, OverlayShow};
use photocraft_ui_egui::{PhotocraftApp, Tool};
use serde_json::{Value, json};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.sync_views();
    app
}

/// A control request as the control channel sends it, then the frame's sync.
fn control(app: &mut PhotocraftApp, method: &str, params: Value) -> Value {
    let ctx = egui::Context::default();
    let (req, rx) = ControlRequest::new(method, params);
    handle(app, &ctx, &req);
    app.sync_views();
    rx.try_recv().unwrap_or(Value::Null)
}

/// Pick a document by its Window menu row, as the tab bar does.
fn activate(app: &mut PhotocraftApp, index: usize) {
    control(app, "ui.menu.invoke", json!({"id": format!("window.document.{index}")}));
    assert_eq!(app.session.active_index(), Some(index), "document {index} is active");
}

fn content_aware(app: &PhotocraftApp) -> bool {
    app.ui.tool_options.crop_content_aware
}

#[test]
fn the_overlay_shows_always_by_default_and_reset_tool_brings_it_back() {
    let mut app = app();
    let o = &app.ui.tool_options;
    assert_eq!((o.crop_overlay, o.crop_overlay_show), (CropOverlay::Thirds, OverlayShow::Always), "a fresh Crop tool");

    app.ui.tool = Tool::Crop;
    app.ui.tool_options.crop_overlay = CropOverlay::Diagonal;
    app.ui.tool_options.crop_overlay_show = OverlayShow::Auto;
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    let o = &app.ui.tool_options;
    assert_eq!((o.crop_overlay, o.crop_overlay_show), (CropOverlay::Thirds, OverlayShow::Always), "Reset Tool");

    app.ui.tool_options.crop_overlay_show = OverlayShow::Never;
    control(&mut app, "ui.menu.invoke", json!({"id": "tool.reset"}));
    assert_eq!(app.ui.tool_options.crop_overlay_show, OverlayShow::Always, "Reset Tool from Never too");
}

#[test]
fn a_show_mode_already_saved_keeps_its_value() {
    for (saved, want) in [("auto", OverlayShow::Auto), ("never", OverlayShow::Never), ("always", OverlayShow::Always)] {
        let o: photocraft_ui_egui::state::ToolOptions = serde_json::from_value(json!({"crop_overlay_show": saved})).unwrap();
        assert_eq!(o.crop_overlay_show, want, "a saved `{saved}` loads as it was saved");
    }
}

#[test]
fn fill_is_kept_by_tool_switches_and_new_documents() {
    let mut app = app();
    app.ui.tool_options.crop_content_aware = true;

    app.ui.tool = Tool::Brush;
    app.sync_views();
    app.ui.tool = Tool::Crop;
    app.sync_views();
    assert!(content_aware(&app), "switch to another tool and back: kept");

    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    assert_eq!(app.session.active_index(), Some(1), "the new document is active");
    assert!(content_aware(&app), "create a new document: kept");
}

#[test]
fn fill_is_kept_when_a_crop_is_committed_or_cancelled() {
    let mut app = app();
    app.ui.tool = Tool::Crop;
    app.ui.tool_options.crop_content_aware = true;

    photocraft_ui_egui::crop_ui::ensure_frame(&mut app);
    app.ui.crop_rect = Some([8.0, 8.0, 40.0, 32.0]);
    photocraft_ui_egui::canvas::commit_crop(&mut app);
    app.sync_views();
    let size = app.session.active().unwrap().doc.size;
    assert_eq!((size.width, size.height), (32, 24), "the crop was committed");
    assert!(content_aware(&app), "commit a crop: kept");

    photocraft_ui_egui::crop_ui::ensure_frame(&mut app);
    app.ui.crop_rect = Some([4.0, 4.0, 20.0, 16.0]);
    photocraft_ui_egui::crop_ui::cancel(&mut app);
    app.sync_views();
    assert_eq!(app.session.active().unwrap().doc.size.width, 32, "the cancelled crop changed nothing");
    assert!(content_aware(&app), "cancel a crop: kept");
}

#[test]
fn fill_goes_back_to_the_default_entry_when_another_open_document_becomes_active() {
    let mut app = app();
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();

    // A tab switch to a document that was already open.
    app.ui.tool_options.crop_content_aware = true;
    activate(&mut app, 0);
    assert!(!content_aware(&app), "make another already-open document active: back to the default");

    // Switching back to the first one doesn't bring it back.
    activate(&mut app, 1);
    assert!(!content_aware(&app), "switch back to the first document: stays at the default");

    // Closing the active document makes another open one active.
    app.ui.tool_options.crop_content_aware = true;
    control(&mut app, "ui.menu.invoke", json!({"id": "file.close"}));
    assert_eq!(app.session.documents().len(), 1, "one document closed");
    assert!(!content_aware(&app), "close the active document so another becomes active: back to the default");
}

#[test]
fn fill_is_kept_while_the_same_document_stays_active() {
    let mut app = app();
    app.ui.tool_options.crop_content_aware = true;
    // Repeated syncs of the same document, as every frame does, leave it alone.
    for _ in 0..3 {
        app.sync_views();
    }
    assert!(content_aware(&app));
    // An edit replaces the snapshot without changing the document.
    app.run("layer.new.layer", json!({})).unwrap();
    app.sync_views();
    assert!(content_aware(&app), "an edit in the active document keeps it");
}
