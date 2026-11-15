//! Zoom tool › Zoom In / Zoom Out (#12066): a pair of toggles on the options bar. In Zoom Out mode
//! a click steps out and ⌥+click steps in, the cursor is the "-" magnifier ("+" with ⌥), and a
//! drag with Scrubby Zoom off steps out once at the press point, as ⌥+drag does in Zoom In mode.

use super::*;
use egui::{Event, Key, Modifiers, PointerButton};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::json;

use crate::state::{Tool, ToolOptions};

const DOC: [u32; 2] = [400, 300];

fn harness(tool: Tool, zoom_in: bool, scrubby: bool) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": DOC[0], "height": DOC[1]})).unwrap();
    // No animation, the clicked point stays under the pointer, and the view is never clamped:
    // every step lands on the first frame where the tests can compare it.
    app.run("prefs.set", json!({"values": {"general.animatedZoom": false, "tools.zoomClickedPointToCenter": false, "tools.overscroll": true}})).unwrap();
    app.sync_views();
    app.ui.extras.rulers = false;
    app.ui.tool = tool;
    app.ui.tool_options.zoom_in = zoom_in;
    app.ui.tool_options.zoom_scrubby = scrubby;
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
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
    v.fill_pending = false;
    h.run_steps(2);
    h
}

fn xf(h: &Harness<'static, PhotocraftApp>) -> ViewXform {
    ViewXform::active(h.state()).unwrap()
}

/// The platform's modifiers for `Cmd` (⌘ on the Mac, Ctrl elsewhere), as egui-winit reports them.
fn platform(m: Modifiers) -> Modifiers {
    let mut out = Modifiers { alt: m.alt, shift: m.shift, ctrl: m.ctrl, ..Default::default() };
    if m.command {
        out.command = true;
        if cfg!(target_os = "macos") {
            out.mac_cmd = true;
        } else {
            out.ctrl = true;
        }
    }
    out
}

fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, pressed: bool, m: Modifiers) {
    h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: platform(m) });
    h.step();
}

fn click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, m: Modifiers) {
    h.event(Event::ModifiersChanged(platform(m)));
    h.event(Event::PointerMoved(p));
    h.step();
    button(h, p, true, m);
    button(h, p, false, m);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

/// The view after one click at `p` with `m`, from zoom 1 centred on the document.
fn view_after_click(zoom_in: bool, m: Modifiers) -> (f32, [f32; 2]) {
    let mut h = harness(Tool::Zoom, zoom_in, true);
    let p = h.state().last_canvas_rect.center() + vec2(90.0, -45.0);
    click(&mut h, p, m);
    let v = &h.state().ui.views[0];
    (v.zoom, v.center)
}

#[test]
fn zoom_out_mode_click_steps_out_and_alt_click_steps_in() {
    let mut h = harness(Tool::Zoom, false, true);
    let p = h.state().last_canvas_rect.center() + vec2(90.0, -45.0);
    let doc = xf(&h).to_doc(p);

    click(&mut h, p, Modifiers::NONE);
    let out = crate::zoom_levels::step(1.0, -1, DOC);
    assert!(out < 1.0);
    assert_eq!(h.state().current_zoom(), out, "a click steps one stop out");
    assert!(xf(&h).to_screen(doc[0] as f32, doc[1] as f32).distance(p) < 0.5, "about the clicked point");

    click(&mut h, p, Modifiers::ALT);
    assert_eq!(h.state().current_zoom(), crate::zoom_levels::step(out, 1, DOC), "⌥+click steps one stop in");
    assert!(xf(&h).to_screen(doc[0] as f32, doc[1] as f32).distance(p) < 0.5);

    assert_eq!(h.state().session.documents()[0].history.past_len(), 0, "zooming adds no history state");
}

#[test]
fn zoom_out_mode_click_is_zoom_in_mode_alt_click_and_the_reverse() {
    assert_eq!(view_after_click(false, Modifiers::NONE), view_after_click(true, Modifiers::ALT));
    assert_eq!(view_after_click(false, Modifiers::ALT), view_after_click(true, Modifiers::NONE));
    // Zoom In mode keeps today's click: in, and out with ⌥.
    assert_eq!(view_after_click(true, Modifiers::NONE).0, crate::zoom_levels::step(1.0, 1, DOC));
    assert_eq!(view_after_click(true, Modifiers::ALT).0, crate::zoom_levels::step(1.0, -1, DOC));
}

/// A cursor as the platform gets it: the icon, and the bitmap with its size where one is drawn.
type Cursor = (egui::CursorIcon, Option<([u16; 2], std::sync::Arc<[u8]>)>);

/// The canvas cursor over the image with ⌥ held or not.
fn cursor(h: &mut Harness<'static, PhotocraftApp>, alt: bool) -> Cursor {
    let p = h.state().last_canvas_rect.center() + vec2(alt as u8 as f32, 0.0);
    h.event(Event::ModifiersChanged(if alt { Modifiers::ALT } else { Modifiers::NONE }));
    h.event(Event::PointerMoved(p));
    h.run_steps(2);
    let o = &h.output().platform_output;
    (o.cursor_icon, o.cursor_image.clone().map(|i| (i.size, i.rgba)))
}

#[test]
fn zoom_out_mode_cursor_is_the_minus_magnifier_and_plus_with_alt() {
    let mut zoom_in = harness(Tool::Zoom, true, true);
    let (plus, minus) = (cursor(&mut zoom_in, false), cursor(&mut zoom_in, true));
    assert_ne!(plus, minus, "the two magnifiers differ");
    let mut zoom_out = harness(Tool::Zoom, false, true);
    assert_eq!(cursor(&mut zoom_out, false), minus, "Zoom Out mode shows \"-\"");
    assert_eq!(cursor(&mut zoom_out, true), plus, "\"+\" while ⌥ is held");
}

/// Scrubby Zoom off: a drag from a point to 120 x 80 points away, with `m` held. Returns the
/// view at the release, and whether a zoom rectangle was on screen while dragging.
fn rect_drag(zoom_in: bool, m: Modifiers) -> ((f32, [f32; 2]), bool) {
    let mut h = harness(Tool::Zoom, zoom_in, false);
    let a = h.state().last_canvas_rect.center() + vec2(-80.0, -50.0);
    let doc = xf(&h).to_doc(a);
    h.event(Event::ModifiersChanged(platform(m)));
    h.event(Event::PointerMoved(a));
    h.step();
    button(&mut h, a, true, m);
    let mut shown = false;
    for t in [0.25, 0.5, 1.0] {
        h.event(Event::PointerMoved(a + vec2(120.0, 80.0) * t));
        h.step();
        shown |= shown_rect(&h.ctx).is_some();
        assert_eq!(h.state().current_zoom(), 1.0, "no zoom while dragging");
    }
    button(&mut h, a + vec2(120.0, 80.0), false, m);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    if !zoom_in || m.alt {
        assert!(xf(&h).to_screen(doc[0] as f32, doc[1] as f32).distance(a) < 0.5, "anchored at the press point");
    }
    let v = &h.state().ui.views[0];
    ((v.zoom, v.center), shown)
}

#[test]
fn zoom_out_mode_drag_without_scrubby_zoom_steps_out_once_like_alt_drag() {
    let (out_mode, out_shown) = rect_drag(false, Modifiers::NONE);
    let (alt_drag, alt_shown) = rect_drag(true, Modifiers::ALT);
    assert_eq!(out_mode.0, crate::zoom_levels::step(1.0, -1, DOC), "one stop out");
    assert_eq!(out_mode, alt_drag, "the same view as ⌥+drag in Zoom In mode");
    assert!(!out_shown && !alt_shown, "no zoom rectangle for a drag that steps out");
    // Zoom In mode keeps its zoom rectangle.
    let (rect, rect_shown) = rect_drag(true, Modifiers::NONE);
    assert!(rect_shown);
    assert!(rect.0 > 1.0, "the rectangle zooms in: {}", rect.0);
}

#[test]
fn temporary_zoom_keys_ignore_the_zoom_out_mode() {
    let mut h = harness(Tool::Brush, false, true);
    let p = h.state().last_canvas_rect.center();
    let key = |h: &mut Harness<'static, PhotocraftApp>, pressed: bool, m: Modifiers| {
        let m = platform(m);
        h.event(Event::ModifiersChanged(m));
        h.event(Event::Key { key: Key::Space, physical_key: None, pressed, repeat: false, modifiers: m });
        h.step();
    };
    key(&mut h, true, Modifiers::COMMAND);
    click(&mut h, p, Modifiers::COMMAND);
    key(&mut h, false, Modifiers::COMMAND);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    let z = h.state().current_zoom();
    assert_eq!(z, crate::zoom_levels::step(1.0, 1, DOC), "⌘Space still zooms in");
    key(&mut h, true, Modifiers::COMMAND | Modifiers::ALT);
    click(&mut h, p, Modifiers::COMMAND | Modifiers::ALT);
    key(&mut h, false, Modifiers::COMMAND | Modifiers::ALT);
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
    assert_eq!(h.state().current_zoom(), crate::zoom_levels::step(z, -1, DOC), "⌘⌥Space still zooms out");
    assert_eq!(h.state().ui.tool, Tool::Brush);
    assert!(!h.state().ui.tool_options.zoom_in);
}

#[test]
fn held_z_ignores_the_zoom_out_mode() {
    let mut h = harness(Tool::Brush, false, true);
    let p = h.state().last_canvas_rect.center();
    let z_key = |h: &mut Harness<'static, PhotocraftApp>, pressed: bool| {
        h.event(Event::Key { key: Key::Z, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE });
        h.run_steps(3);
    };
    z_key(&mut h, true);
    assert_eq!(h.state().ui.tool, Tool::Zoom, "Z held springs to the Zoom tool");
    click(&mut h, p, Modifiers::NONE);
    z_key(&mut h, false);
    let z = h.state().current_zoom();
    assert_eq!(z, crate::zoom_levels::step(1.0, 1, DOC), "a click with Z held still zooms in");
    assert_eq!(h.state().ui.tool, Tool::Brush);
    z_key(&mut h, true);
    click(&mut h, p, Modifiers::ALT);
    z_key(&mut h, false);
    assert_eq!(h.state().current_zoom(), crate::zoom_levels::step(z, -1, DOC), "⌥+click with Z held still zooms out");
    assert_eq!(h.state().ui.tool, Tool::Brush);
    assert!(!h.state().ui.tool_options.zoom_in);
}

fn options_bar() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.tool = Tool::Zoom;
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1400.0, 60.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                crate::theme::install_fonts(ui.ctx());
                crate::theme::apply(ui.ctx(), crate::theme::ThemeKind::ALL[0]);
                ready = true;
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    h.run_steps(4);
    h
}

fn left(h: &Harness<'static, PhotocraftApp>, label: &str) -> f64 {
    h.get_by_label(label).accesskit_node().bounding_box().unwrap_or_else(|| panic!("{label} is laid out")).x0
}

#[test]
fn options_bar_has_the_zoom_in_and_zoom_out_pair_before_the_checkboxes() {
    let mut h = options_bar();
    assert!(h.state().ui.tool_options.zoom_in, "Zoom In is pressed by default");
    let (zin, zout, fit) = (left(&h, "Zoom In"), left(&h, "Zoom Out"), left(&h, "Resize Windows to Fit"));
    assert!(zin < zout && zout < fit, "Zoom In {zin}, Zoom Out {zout}, Resize Windows to Fit {fit}");

    // Exactly one is pressed: clicking either selects it, clicking it again keeps it.
    for (label, zoom_in) in [("Zoom Out", false), ("Zoom Out", false), ("Zoom In", true), ("Zoom In", true), ("Zoom Out", false)] {
        h.get_by_label(label).click();
        h.run_steps(2);
        assert_eq!(h.state().ui.tool_options.zoom_in, zoom_in, "after clicking {label}");
    }
}

#[test]
fn zoom_mode_survives_reset_tool_and_defaults_to_zoom_in() {
    let mut o = ToolOptions { zoom_in: false, zoom_scrubby: false, ..ToolOptions::default() };
    crate::tool_reset::reset_options(&mut o, Tool::Zoom);
    assert!(o.zoom_scrubby, "Reset Tool resets Scrubby Zoom");
    assert!(!o.zoom_in, "but leaves Zoom Out selected");

    assert!(ToolOptions::default().zoom_in);
    let mut saved = serde_json::to_value(ToolOptions { zoom_in: false, ..ToolOptions::default() }).unwrap();
    assert!(!serde_json::from_value::<ToolOptions>(saved.clone()).unwrap().zoom_in, "the mode is saved");
    // Options saved before the mode existed load as Zoom In; a bad value is an error, not a panic.
    saved.as_object_mut().unwrap().remove("zoom_in");
    assert!(serde_json::from_value::<ToolOptions>(saved.clone()).unwrap().zoom_in);
    saved["zoom_in"] = json!("out");
    assert!(serde_json::from_value::<ToolOptions>(saved).is_err());
}
