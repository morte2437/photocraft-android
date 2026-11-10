//! #11962: the Pencil's options bar gets the two pressure toggles, editing the Pencil's own
//! brush; the Brush keeps its own. Pro themes show the icon toggles, the others the labelled
//! switches, both with the reference tooltips.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable, by};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;
use crate::theme::ThemeKind;

fn bar(theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100, "background": "white"})).unwrap();
    app.ui.tool = Tool::Pencil;
    let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| crate::panels::options_bar(app, ui), app);
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

fn description(h: &Harness<'static, PhotocraftApp>, label: &str) -> Option<String> {
    h.query_all_by_label(label).next().and_then(|n| n.accesskit_node().description().map(|d| d.to_string()))
}

const PRO: [(&str, &str); 2] = [
    ("Always use pressure for opacity", "Always use Pressure for Opacity. When off, Brush Preset controls pressure"),
    ("Always use pressure for size", "Always use Pressure for Size. When off, Brush Preset controls pressure"),
];

const CLASSIC: [(&str, &str); 2] = [
    ("Pressure for Size", "Always use Pressure for Size. When off, Brush Preset controls pressure"),
    ("Pressure for Opacity", "Always use Pressure for Opacity. When off, Brush Preset controls pressure"),
];

#[test]
fn the_pro_bar_shows_both_toggles_with_the_reference_tooltips() {
    let h = bar(ThemeKind::ProMedium);
    for (label, tip) in PRO {
        assert_eq!(h.query_all_by_label(label).count(), 1, "{label} exists once");
        assert_eq!(description(&h, label).as_deref(), Some(tip), "{label} tooltip");
    }
}

#[test]
fn the_pro_toggles_edit_the_pencils_own_flags_and_brush_get_sees_them() {
    let mut h = bar(ThemeKind::ProMedium);
    let flags = |h: &Harness<'static, PhotocraftApp>| (h.state().session.tools.brush.pressure_opacity, h.state().session.tools.brush.pressure_size);
    assert_eq!(flags(&h), (false, false), "both off on a fresh Pencil");
    h.get_by_label("Always use pressure for opacity").click();
    h.run_steps(2);
    h.get_by_label("Always use pressure for size").click();
    h.run_steps(2);
    assert_eq!(flags(&h), (true, true));
    let got = h.state_mut().run("brush.get", json!({})).unwrap();
    assert_eq!(got["pressureOpacity"], json!(true), "brush.get reads the Pencil's flag");
    assert_eq!(got["pressureSize"], json!(true));
}

#[test]
fn the_brush_keeps_its_own_pressure_flags() {
    let mut h = bar(ThemeKind::ProMedium);
    let flags = |h: &Harness<'static, PhotocraftApp>| (h.state().session.tools.brush.pressure_opacity, h.state().session.tools.brush.pressure_size);
    let toggle = |h: &mut Harness<'static, PhotocraftApp>, label: &str| {
        h.get_by_label(label).click();
        h.run_steps(2);
    };

    // Give the Brush its own brush first, so the Pencil's later toggles cannot be "inherited" on
    // the first switch and hide a leak.
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    h.state_mut().session.tools.brush.pressure_opacity = false;
    h.state_mut().session.tools.brush.pressure_size = false;
    h.run_steps(2);

    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(4);
    toggle(&mut h, "Always use pressure for opacity");
    toggle(&mut h, "Always use pressure for size");
    assert_eq!(flags(&h), (true, true), "the Pencil's flags are on");

    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    assert_eq!(flags(&h), (false, false), "the Pencil's toggles did not change the Brush");
    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(4);
    assert_eq!(flags(&h), (true, true), "the Pencil's flags came back");

    // The mirror: the Brush on, the Pencil off, so a reset to one value cannot pass.
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    h.state_mut().session.tools.brush.pressure_opacity = true;
    h.state_mut().session.tools.brush.pressure_size = true;
    h.run_steps(2);
    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(4);
    toggle(&mut h, "Always use pressure for opacity");
    toggle(&mut h, "Always use pressure for size");
    assert_eq!(flags(&h), (false, false), "the Pencil is off");
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    assert_eq!(flags(&h), (true, true), "the Brush stayed on");
    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(4);
    assert_eq!(flags(&h), (false, false), "the Pencil stayed off");

    // A mixed setting: opacity on, size off, across a switch and back.
    toggle(&mut h, "Always use pressure for opacity");
    assert_eq!(flags(&h), (true, false), "mixed: opacity on, size off");
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(4);
    assert_eq!(flags(&h), (true, true), "the Brush still has its own pair");
    h.state_mut().ui.tool = Tool::Pencil;
    h.run_steps(4);
    assert_eq!(flags(&h), (true, false), "the mixed pair came back unchanged");
    // And the bar draws that pair, not just stores it.
    let on = |h: &Harness<'static, PhotocraftApp>, label: &str| h.get_by_label(label).accesskit_node().toggled() == Some(egui::accesskit::Toggled::True);
    assert!(on(&h, "Always use pressure for opacity"), "the opacity toggle reads as on");
    assert!(!on(&h, "Always use pressure for size"), "the size toggle reads as off");
}

#[test]
fn the_other_themes_show_the_labelled_toggles() {
    let mut h = bar(ThemeKind::Classic);
    // The switches are unnamed nodes: their tooltips are the descriptions in the bar.
    let tips: Vec<String> =
        h.query_all(by().predicate(|n| n.description().is_some())).filter_map(|n| n.accesskit_node().description().map(|d| d.to_string())).collect();
    for (label, tip) in CLASSIC {
        assert_eq!(h.query_all_by_label(label).count(), 1, "{label} exists once");
        assert!(tips.iter().any(|d| d == tip), "{label} tooltip: {tips:?}");
    }
    for (label, tip, expect) in [("Pressure for Size", CLASSIC[0].1, true), ("Pressure for Opacity", CLASSIC[1].1, true)] {
        let at = h
            .query_all(by().predicate(move |n| n.description().map(|d| d == tip).unwrap_or(false)))
            .next()
            .unwrap_or_else(|| panic!("no switch for {label}"))
            .rect()
            .center();
        h.hover_at(at);
        h.run();
        h.drag_at(at);
        h.run();
        h.drop_at(at);
        h.run_steps(2);
        let flag = if label.ends_with("Size") { h.state().session.tools.brush.pressure_size } else { h.state().session.tools.brush.pressure_opacity };
        assert_eq!(flag, expect, "{label} toggles the Pencil's flag");
    }
}

/// Painted width at column `x` on the active layer, through the stroke's row band.
fn width(app: &PhotocraftApp, x: i32, y: i32) -> usize {
    let st = app.session.active().expect("a document");
    let layer = st.active_layer.expect("an active layer");
    let s = st.doc.layer(layer).and_then(|l| l.surface()).expect("a raster layer");
    (y - 20..=y + 20).filter(|yy| s.rgba(x, *yy)[3] > 0.5).count()
}

#[test]
fn pressure_for_size_narrows_a_low_pressure_pencil_stroke() {
    let mut h = bar(ThemeKind::ProMedium);
    let app = h.state_mut();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 13.0, "pressureSize": true, "smoothing": {"amount": 0.0}}})).unwrap();
    app.run("paint.pencil", json!({"points": [[20.0, 50.0, 0.2], [30.0, 50.0, 0.2], [60.0, 50.0, 1.0]]})).unwrap();
    let (low, high) = (width(app, 22, 50), width(app, 58, 50));
    assert!(low < high, "pressure for size on: low-pressure {low} px, high-pressure {high} px");
    assert!((2..=4).contains(&low), "pressure for size on: the 0.2 stroke is about 3 px, got {low}");

    app.run("layer.new.layer", json!({})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"pressureSize": false}})).unwrap();
    app.run("paint.pencil", json!({"points": [[20.0, 50.0, 0.2], [30.0, 50.0, 0.2], [60.0, 50.0, 1.0]]})).unwrap();
    let (low, high) = (width(app, 22, 50), width(app, 58, 50));
    assert_eq!((low, high), (13, 13), "pressure for size off: every dab is 13 px");
}
