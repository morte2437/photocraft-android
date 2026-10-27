//! The phone layout's dock (`panels::phone_dock`): only the icon rail sits beside the canvas,
//! and a group opens as a sheet over it.

use egui::{Rect, vec2};
use egui_kittest::Harness;

use crate::PhotocraftApp;
use crate::dock::Group;

/// The dock area (rail, sheet and canvas) on a phone held upright.
fn harness() -> Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.compact_layout = true;
    let mut h = Harness::builder().with_size(vec2(400.0, 760.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::dock_area(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

/// The open group's sheet, when one was drawn last frame.
fn sheet(h: &Harness<'static, PhotocraftApp>) -> Option<Rect> {
    let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::unique("phone-panel-sheet"));
    h.ctx.memory(|m| m.areas().is_visible(&layer).then(|| m.area_rect(layer.id)).flatten())
}

#[test]
fn a_group_opens_as_a_sheet_over_the_canvas() {
    let mut h = harness();
    assert_eq!(sheet(&h), None, "no group drawn until one is opened");
    h.state_mut().phone_panel = Some(Group::Layers);
    h.run_steps(3);
    let screen = h.ctx.content_rect();
    let r = sheet(&h).expect("the Layers sheet");
    assert!(screen.contains_rect(r) && r.width() <= 364.0, "a sheet inside the window: {r:?}");
    // Closing it from the panel puts the sheet away.
    *Group::Layers.shown_mut(&mut h.state_mut().ui.panels) = false;
    h.run_steps(2);
    assert_eq!(h.state().phone_panel, None);
}

#[test]
fn window_menu_panels_open_as_the_sheet() {
    let mut h = harness();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.character", serde_json::json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(h.state().phone_panel, Some(Group::Character));
    assert!(sheet(&h).is_some(), "Character is drawn");
    // The same item again puts it away; another panel's item swaps to it.
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.character", serde_json::json!({})).unwrap();
    assert_eq!(h.state().phone_panel, None);
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.layers", serde_json::json!({})).unwrap();
    assert_eq!(h.state().phone_panel, Some(Group::Layers));
}
