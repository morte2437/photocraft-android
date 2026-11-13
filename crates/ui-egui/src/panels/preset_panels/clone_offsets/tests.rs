use super::*;
use crate::canvas::{Drag, ToolEvent, tool_event};
use photocraft_engine::Session;

/// Create two distinguishable source colours and an empty layer for real mouse strokes.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("paint.pencil", json!({"points": [[16,16]], "size": 4, "color": "#ff0000"})).unwrap();
    app.run("paint.pencil", json!({"points": [[20,16]], "size": 4, "color": "#0000ff"})).unwrap();
    app.ui.tool = Tool::CloneStamp;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"size": 8, "hardness": 1, "opacity": 1, "flow": 1})).unwrap();
    crate::retouch_ui::set_source(&mut app, 16.0, 16.0);
    app.run("layer.new.layer", json!({})).unwrap();
    app
}

/// Exercise the same Down/Up path used by canvas interaction.
fn stamp(app: &mut PhotocraftApp, at: [f64; 2]) {
    remember_clone_cursor(app, at);
    tool_event(app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, egui::Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: at[0], y: at[1] }, egui::Modifiers::NONE);
    assert!(!app.ui.status_error, "{}", app.ui.status);
}

/// Hover may change a displayed pairing but cannot alter source state or document revision.
#[test]
fn clone_offset_hover_readout_does_not_establish_a_painting_anchor() {
    for aligned in [false, true] {
        let mut app = app();
        app.ui.tool_options.clone_aligned = aligned;
        let before = app.session.presets.clone.clone();
        let revision = app.session.active().unwrap().revision;
        for at in [[40.0, 30.0], [50.0, 40.0]] {
            remember_clone_cursor(&mut app, at);
            assert_eq!(clone_offset_readout(&app), Some([16.0 - at[0], 16.0 - at[1]]));
            assert_eq!(app.session.presets.clone, before);
            assert_eq!(app.session.active().unwrap().revision, revision);
        }
    }
}

/// Entering the panel must retain the last canvas reference for a numeric offset edit.
#[test]
fn clone_offset_edit_before_painting_uses_the_preview_reference_and_matches_pixels() {
    for aligned in [false, true] {
        let mut app = app();
        app.ui.tool_options.clone_aligned = aligned;
        remember_clone_cursor(&mut app, [40.0, 30.0]);
        app.hover_doc = None; // The pointer left the canvas to edit the panel.
        let edit = clone_offset_edit(&app, [-20.0, -14.0]).unwrap();
        assert_eq!(edit["anchor"], json!([40.0, 30.0]));
        app.run("cloneSource.set", edit).unwrap();
        assert_eq!(clone_offset_readout(&app), Some([-20.0, -14.0]));
        stamp(&mut app, [40.0, 30.0]);
        let st = app.session.active().unwrap();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(40, 30), [0.0, 0.0, 1.0, 1.0]);
        assert_eq!(clone_sample_point(&app, Some([40.0, 30.0])), Some([20.0, 16.0]));
    }
}

/// Reporting an unaligned stroke must not discard the established aligned sampling origin.
#[test]
fn clone_offset_readout_tracks_aligned_mode_without_replacing_its_saved_anchor() {
    let mut app = app();
    stamp(&mut app, [40.0, 30.0]);
    remember_clone_cursor(&mut app, [50.0, 40.0]);
    assert_eq!(clone_offset_readout(&app), Some([-24.0, -14.0]));
    let original_anchor = app.session.presets.clone.active().anchor;
    app.ui.tool_options.clone_aligned = false;
    assert_eq!(clone_offset_readout(&app), Some([-34.0, -24.0]));
    stamp(&mut app, [50.0, 40.0]);
    assert_eq!(app.session.presets.clone.active().anchor, original_anchor);
    assert_eq!(clone_offset_readout(&app), Some([-34.0, -24.0]));
    app.ui.tool_options.clone_aligned = true;
    assert_eq!(clone_offset_readout(&app), Some([-24.0, -14.0]));
    assert_eq!(app.session.presets.clone.active().anchor, original_anchor);
}

/// A slot switch changes the source; a cursor from another destination cannot set its reference.
#[test]
fn clone_offset_readout_uses_the_selected_slot_and_destination_document() {
    let mut app = app();
    let source_layer = app.session.presets.clone.active().layer.unwrap();
    remember_clone_cursor(&mut app, [40.0, 30.0]);
    app.run("cloneSource.set", json!({"index": 2, "source": [20,16], "layer": source_layer})).unwrap();
    assert_eq!(clone_offset_readout(&app), Some([-20.0, -14.0]));
    app.run("cloneSource.select", json!({"index": 0})).unwrap();
    assert_eq!(clone_offset_readout(&app), Some([-24.0, -14.0]));
    app.run("cloneSource.select", json!({"index": 1})).unwrap();
    assert!(clone_offset_readout(&app).is_none());
    assert!(clone_offset_edit(&app, [0.0, 0.0]).is_none());
    app.run("cloneSource.select", json!({"index": 0})).unwrap();
    app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
    assert_eq!(clone_offset_readout(&app), Some([0.0, 0.0]), "another destination has no preview position yet");
    remember_clone_cursor(&mut app, [50.0, 40.0]);
    assert_eq!(clone_offset_readout(&app), Some([-34.0, -24.0]));
    assert!(app.session.set_active(0));
    assert_eq!(clone_offset_readout(&app), Some([0.0, 0.0]), "a cursor from another document cannot become this one's reference");
}

/// The marker and readout use the same scaled, rotated or flipped mapping as sampling.
#[test]
fn clone_offsets_share_transformed_sampling_geometry_during_a_stroke() {
    for (transform, want) in [
        (json!({"width": 200}), [18.0, 18.0]),
        (json!({"rotation": 90}), [14.0, 20.0]),
        (json!({"flipH": true}), [12.0, 18.0]),
        (json!({"flipV": true}), [20.0, 14.0]),
    ] {
        let mut app = app();
        app.ui.tool_options.clone_aligned = false;
        app.run("cloneSource.set", transform).unwrap();
        app.drag = Some(Drag::new(Tool::CloneStamp, [40.0, 30.0], vec![[44.0, 32.0, 1.0]], egui::Modifiers::NONE, false));
        remember_clone_cursor(&mut app, [44.0, 32.0]);
        assert_eq!(clone_offset_readout(&app), Some([-24.0, -14.0]), "offsets describe the transform's origin, not a moving screen-space distance");
        let sample = clone_sample_point(&app, Some([44.0, 32.0])).unwrap();
        for (got, want) in sample.into_iter().zip(want) {
            assert!((got - want).abs() < 1e-8);
        }
        assert!(app.session.presets.clone.active().anchor.is_none());
    }
}

/// Invalid cursor positions and restored UI state must not invent a preview reference.
#[test]
fn clone_offset_cursor_is_transient_and_rejects_nonfinite_positions() {
    let mut app = app();
    remember_clone_cursor(&mut app, [40.0, 30.0]);
    let before = app.ui.presets_ui.clone_cursor;
    for at in [[f64::NAN, 0.0], [0.0, f64::INFINITY], [1e300, 0.0]] {
        remember_clone_cursor(&mut app, at);
        assert_eq!(app.ui.presets_ui.clone_cursor, before);
    }
    let saved = serde_json::to_value(&app.ui.presets_ui).unwrap();
    assert!(saved.get("clone_cursor").is_none());
    let restored: crate::preset_panels::PresetUi = serde_json::from_value(saved).unwrap();
    assert!(restored.clone_cursor.is_none());
}

/// View zoom changes screen geometry while document-space displacement remains constant.
#[test]
fn clone_offset_readout_is_in_document_pixels_at_different_canvas_zooms() {
    let mut app = app();
    app.last_canvas_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(800.0, 600.0));
    for zoom in [1.0, 2.0, 4.0] {
        let view = app.ui.views.first_mut().unwrap();
        view.zoom = zoom;
        view.center = [32.0, 32.0];
        view.fit_pending = false;
        view.doc_size = [64, 64];
        let screen = egui::pos2(400.0 + 8.0 * zoom, 300.0 - 2.0 * zoom);
        let at = crate::canvas::doc_point_at(&app, screen).unwrap();
        assert!((at[0] - 40.0).abs() < 1e-6 && (at[1] - 30.0).abs() < 1e-6);
        remember_clone_cursor(&mut app, at);
        assert_eq!(clone_offset_readout(&app), Some([-24.0, -14.0]));
    }
}
