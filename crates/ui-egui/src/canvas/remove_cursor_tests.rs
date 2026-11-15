//! The Remove tool's cursor (#12550): with the default cursor preferences it is the brush circle
//! with a + at its centre. The Precise and Standard painting cursors stay as they were, and the
//! other brushes keep following Preferences › Cursors › Show Crosshair in Brush Tip. Tests drive
//! the real canvas (`document_area`) and read the OS cursor bitmap it hands the platform.

use egui::CursorIcon;
use egui_kittest::Harness;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

/// The canvas at 100 % with `tool` and a 40 px tip, the pointer over its centre.
pub(super) fn harness(tool: Tool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.ui.tool = tool;
    crate::paint_mouse::sync_tool_brush(&mut app);
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 1.0}})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    let mut h = Harness::builder().with_size(egui::vec2(900.0, 600.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    let v = &mut h.state_mut().ui.views[0];
    v.zoom = 1.0;
    v.center = [200.0, 150.0];
    v.fit_pending = false;
    h.run_steps(2);
    let p = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(3);
    h
}

pub(super) fn set_pref(h: &mut Harness<'_, PhotocraftApp>, path: &str, value: serde_json::Value) {
    h.state_mut().run("prefs.set", json!({"path": path, "value": value})).unwrap();
    h.run_steps(2);
}

/// Whether the brush-tip bitmap marks its hotspot: the bare circle leaves the pixel under the
/// pointer transparent, the centre + covers it.
fn centre_marked(h: &Harness<'_, PhotocraftApp>) -> bool {
    let out = h.output();
    let image = out.platform_output.cursor_image.as_ref().expect("the brush tip cursor bitmap");
    let [x, y] = image.hotspot.map(usize::from);
    image.rgba[(y * usize::from(image.size[0]) + x) * 4 + 3] > 230
}

/// Whether the brush-tip bitmap draws the 40 px tip's circle: an opaque pixel on the diagonal
/// below-right of the hotspot, about 20 px out, where no arm of the centre + reaches.
fn circle_drawn(h: &Harness<'_, PhotocraftApp>) -> bool {
    let out = h.output();
    let image = out.platform_output.cursor_image.as_ref().expect("the brush tip cursor bitmap");
    let [x, y] = image.hotspot.map(usize::from);
    let [w, rows] = image.size.map(usize::from);
    (12..=16).filter(|d| x + d < w && y + d < rows).any(|d| image.rgba[((y + d) * w + x + d) * 4 + 3] > 230)
}

/// A cursor icon, and its bitmap's size, hotspot and pixels.
pub(super) type Cursor = (CursorIcon, Option<([u16; 2], [u16; 2], Vec<u8>)>);

/// The cursor the canvas asks for this frame.
pub(super) fn cursor(h: &Harness<'_, PhotocraftApp>) -> Cursor {
    let out = h.output();
    let image = out.platform_output.cursor_image.as_ref().map(|i| (i.size, i.hotspot, i.rgba.to_vec()));
    (out.platform_output.cursor_icon, image)
}

#[test]
fn remove_tool_draws_the_centre_cross_with_default_cursor_preferences() {
    let h = harness(Tool::Remove);
    assert!(centre_marked(&h), "Normal Brush Tip, Show Crosshair off: circle with a + at the centre");
    assert!(circle_drawn(&h), "the brush circle around the +");
}

#[test]
fn remove_tool_precise_and_standard_cursors_match_the_brush() {
    for value in ["precise", "standard"] {
        let mut remove = harness(Tool::Remove);
        let mut brush = harness(Tool::Brush);
        set_pref(&mut remove, "cursors.painting", json!(value));
        set_pref(&mut brush, "cursors.painting", json!(value));
        assert_eq!(cursor(&remove), cursor(&brush), "{value}: the Remove tool's cursor is the Brush's");
    }
    let mut h = harness(Tool::Remove);
    set_pref(&mut h, "cursors.painting", json!("standard"));
    assert_eq!(cursor(&h), (CursorIcon::Default, None), "Standard: the plain pointer");
}

#[test]
fn other_brushes_follow_show_crosshair_in_brush_tip() {
    for tool in [Tool::Brush, Tool::SpotHealing] {
        let mut h = harness(tool);
        assert!(!centre_marked(&h), "{tool:?}: Show Crosshair off, a bare circle");
        set_pref(&mut h, "cursors.showCrosshairInBrushTip", json!(true));
        assert!(centre_marked(&h), "{tool:?}: Show Crosshair on, the centre +");
    }
}
