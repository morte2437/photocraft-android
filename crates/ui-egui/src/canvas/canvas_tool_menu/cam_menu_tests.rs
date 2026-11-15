use super::*;
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::Harness;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.ui.tool = Tool::ContentAwareMove;
    app
}

fn harness(mut app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.run_steps(4);
    h
}

fn right_click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

#[test]
fn content_aware_move_outside_and_without_selection_matches_lasso_menus() {
    let mut h = harness(app());
    assert!(applies(Tool::ContentAwareMove));
    let center = h.state().last_canvas_rect.center();

    // With no selection, a canvas right-click opens NO_SELECTION_MENU, matching Tool::Lasso.
    right_click(&mut h, center);
    let menu = h.state().ui.canvas_tool_menu.clone().expect("no-selection menu opened");
    assert!(!menu.has_selection && !inside_selection(&menu) && menu.sampler.is_none());
    assert_eq!(rows(&menu), NO_SELECTION_MENU);
    let ctx = h.ctx.clone();
    choose(h.state_mut(), &ctx, "select.all");
    assert!(h.state().session.active().unwrap().doc.selection.is_some());

    // Outside an active selection, a canvas right-click opens SELECTION_MENU, matching Tool::Lasso.
    h.state_mut().run("select.rect", json!({"x": 2, "y": 2, "width": 6, "height": 6})).unwrap();
    right_click(&mut h, center);
    let menu = h.state().ui.canvas_tool_menu.clone().expect("outside-selection menu opened");
    assert!(menu.has_selection && !inside_selection(&menu) && menu.sampler.is_none());
    assert_eq!(rows(&menu), SELECTION_MENU);
    choose(h.state_mut(), &ctx, "select.deselect");
    assert!(h.state().session.active().unwrap().doc.selection.is_none());
}

#[test]
fn content_aware_move_inside_selection_opens_color_range_and_load_selection_only() {
    let mut app = app();
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 24, "height": 24})).unwrap();
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center);

    let menu = h.state().ui.canvas_tool_menu.clone().expect("inside-selection menu opened");
    assert!(menu.has_selection && inside_selection(&menu) && menu.sampler.is_none() && menu.sampler_at.is_none());
    assert_eq!(rows(&menu), &[Some(("Color Range…", "select.colorRange")), Some(("Load Selection…", "select.loadSelection"))]);

    // Color Range… is enabled; Load Selection… is greyed out when no selection channel is saved.
    assert!(entry_enabled(h.state(), &menu, "select.colorRange"));
    assert!(!entry_enabled(h.state(), &menu, "select.loadSelection"));
    assert!(!available(h.state(), &menu, "select.deselect"), "full selection menu rows are not available inside selection");

    // A spot channel does not count as a saved selection channel.
    let ctx = h.ctx.clone();
    h.state_mut().run("channel.newSpot", json!({"name": "Spot 1"})).unwrap();
    assert!(!entry_enabled(h.state(), &menu, "select.loadSelection"));

    // Once a selection channel is saved, Load Selection… becomes enabled and renders cleanly with no samplers.
    h.state_mut().run("select.saveSelection", json!({"name": "Alpha 1"})).unwrap();
    assert!(open_at(h.state_mut(), &ctx, Tool::ContentAwareMove, [10.0, 10.0], Some([16.0, 16.0])));
    h.run_steps(1);
    let menu = h.state().ui.canvas_tool_menu.clone().unwrap();
    assert!(entry_enabled(h.state(), &menu, "select.loadSelection"));

    // Even with a saved channel, Load Selection… greys out when menus::is_enabled blocks it (e.g. a pending crop).
    h.state_mut().ui.tool = Tool::Crop;
    h.state_mut().ui.crop_rect = Some([0.0, 0.0, 16.0, 16.0]);
    h.state_mut().crop.default_frame = false;
    assert!(!entry_enabled(h.state(), &menu, "select.loadSelection"));
    h.state_mut().ui.crop_rect = None;
    h.state_mut().ui.tool = Tool::ContentAwareMove;
    assert!(entry_enabled(h.state(), &menu, "select.loadSelection"));

    // Choosing Color Range… opens the Color Range dialog.
    choose(h.state_mut(), &ctx, "select.colorRange");
    assert!(!h.state().ui.dialogs.is_empty());
    h.state_mut().ui.dialogs.clear();

    // Choosing Load Selection… opens the Load Selection dialog.
    assert!(open_at(h.state_mut(), &ctx, Tool::ContentAwareMove, [10.0, 10.0], Some([16.0, 16.0])));
    choose(h.state_mut(), &ctx, "select.loadSelection");
    assert!(!h.state().ui.dialogs.is_empty());
}

#[test]
fn content_aware_move_boundary_modes_and_overlay_guards() {
    let ctx = Context::default();
    let mut empty = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    empty.ui.tool = Tool::ContentAwareMove;
    crate::color_hud::open(&mut empty, [12.0, 12.0], false);
    assert!(!open_at(&mut empty, &ctx, Tool::ContentAwareMove, [10.0, 10.0], Some([8.0, 8.0])));
    assert!(empty.ui.color_hud.is_none());

    let mut app = app();
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 8, "height": 8})).unwrap();
    // Boundary, negative, and non-finite coordinates.
    for (pt, expected) in [([4.0, 4.0], true), ([11.9, 11.9], true), ([3.9, 4.0], false), ([12.0, 12.0], false), ([-1.0, 4.0], false), ([f64::NAN, 4.0], false)]
    {
        assert!(open_at(&mut app, &ctx, Tool::ContentAwareMove, [10.0, 10.0], Some(pt)));
        let m = app.ui.canvas_tool_menu.clone().unwrap();
        assert_eq!(inside_selection(&m), expected, "point {pt:?}");
    }
    // Transform/pasteboard/guide/crop/anchor menus are not replaced by CAM_INSIDE_SELECTION_MENU.
    let mut tmenu = app.ui.canvas_tool_menu.clone().unwrap();
    tmenu.transform = true;
    assert!(!inside_selection(&tmenu) && rows(&tmenu) == TRANSFORM_MENU);

    // Excluded color modes return false and release a non-held color HUD.
    for mode in [ColorMode::Bitmap, ColorMode::Duotone, ColorMode::Multichannel] {
        std::sync::Arc::make_mut(&mut app.session.active_mut().unwrap().doc).mode = mode;
        crate::color_hud::open(&mut app, [12.0, 12.0], false);
        assert!(!open_at(&mut app, &ctx, Tool::ContentAwareMove, [10.0, 10.0], Some([6.0, 6.0])));
        assert!(app.ui.color_hud.is_none());
    }
}
