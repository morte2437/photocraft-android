use super::*;
use egui::{Event, Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
}

fn lay(app: &PhotocraftApp) -> &Layout<String> {
    &app.ui.docking.as_ref().expect("a customized workspace").layout
}

fn hidden(app: &PhotocraftApp, panel: &str) -> bool {
    app.ui.docking.as_ref().is_some_and(|workspace| workspace.hidden.iter().any(|(id, _)| id == panel))
}

fn hit_rect(h: &Harness<'static, PhotocraftApp>, hit: craft_ui::docking::Hit<String>) -> egui::Rect {
    let hits = craft_ui::docking::DockArea::<String>::hit_rects(&h.ctx, area_id());
    hits.iter().find(|(drawn, _)| *drawn == hit).map(|(_, rect)| *rect).unwrap_or_else(|| panic!("{hit:?} is not drawn"))
}

fn tab_rect(h: &Harness<'static, PhotocraftApp>, panel: &str) -> egui::Rect {
    hit_rect(h, craft_ui::docking::Hit::Tab(panel.to_owned()))
}

fn floating_rect(h: &Harness<'static, PhotocraftApp>, panel: &str) -> egui::Rect {
    let window = lay(h.state()).floating.iter().find(|window| window.contains(&panel.to_owned())).unwrap();
    let layer = craft_ui::docking::DockArea::<String>::floating_layer(area_id(), window).unwrap();
    h.ctx.memory(|memory| memory.area_rect(layer.id)).unwrap()
}

#[test]
fn panel_commands_group_split_reorder_float_and_restore_without_duplicate_ownership() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
    app.run("window.panel.move", json!({"panel":"properties", "anchor":"swatches", "before":"swatches"})).unwrap();
    let before_float = lay(&app).clone();
    app.run("window.panel.float", json!({"panel":"properties", "x":50,"y":70,"width":330,"height":410})).unwrap();
    assert!(lay(&app).floating.iter().any(|g| g.panels().map(String::as_str).eq(["properties"])));
    app.run("window.panel.dock", json!({"panel":"properties"})).unwrap();
    assert_eq!(lay(&app), &before_float);
    app.run("window.panel.close", json!({"panel":"properties"})).unwrap();
    assert!(!lay(&app).contains(&"properties".into()));
    app.run("window.panel.activate", json!({"panel":"properties"})).unwrap();
    assert_eq!(lay(&app), &before_float);
    let layout = lay(&app);
    assert!(valid(layout));
    let unique: std::collections::HashSet<_> = layout.panels().into_iter().collect();
    assert_eq!(unique.len(), layout.panels().len());
    let saved = serde_json::to_value(&app.ui).unwrap();
    let restored: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(restored.docking, app.ui.docking);
    assert_eq!(restored.docking_returns, app.ui.docking_returns);
}

#[test]
fn docking_a_floating_group_docks_all_of_its_panels() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"paths"})).unwrap();
    app.run("window.panel.move", json!({"panel":"history", "anchor":"paths"})).unwrap();
    assert!(lay(&app).floating.iter().any(|w| w.contains("paths") && w.contains("history")));
    app.run("window.panel.dock", json!({"panel":"paths", "group": true})).unwrap();
    assert!(lay(&app).floating.iter().all(|w| !w.contains("paths") && !w.contains("history")), "{:?}", lay(&app).floating);
    assert!(lay(&app).contains(&"paths".into()) && lay(&app).contains(&"history".into()));
}

#[test]
fn floating_panel_rail_toggle_closes_the_float_window() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"properties","x":50,"y":70,"width":330,"height":410})).unwrap();
    assert!(lay(&app).floating.iter().any(|g| g.contains("properties")));

    rail_click(&mut app, Group::Properties);

    assert!(lay(&app).floating.iter().all(|g| !g.contains("properties")));
    assert!(lay(&app).contains(&"adjustments".into()));
    assert!(app.ui.panels.properties, "the docked Adjustments tab keeps the group visible");
}

#[test]
fn brushes_menu_action_uses_the_shared_docking_layout() {
    let mut app = app();
    let before = app.ui.docking.clone();
    assert!(crate::menus::invoke(&mut app, &egui::Context::default(), "window.panel.brushes", json!({})).is_ok());
    let layout = app.ui.docking.as_ref().map(|w| &w.layout).expect("brushes panel must be registered in docking");
    assert!(layout.contains(&"brushes".to_string()));
    assert!(app.ui.panels.brushes);
    assert_ne!(app.ui.docking, before);

    app.run("window.panel.float", json!({"panel":"brushes"})).unwrap();
    assert!(lay(&app).floating.iter().any(|group| group.contains("brushes")));
    app.run("window.panel.dock", json!({"panel":"brushes"})).unwrap();
    assert!(!lay(&app).floating.iter().any(|group| group.contains("brushes")));
}

#[test]
fn invalid_panel_requests_are_atomic_and_do_not_enter_a_custom_workspace() {
    let mut app = app();
    for (id, params) in [
        ("window.panel.move", json!({"panel":"layers","anchor":"unknown"})),
        ("window.panel.move", json!({"panel":"layers","anchor":"properties","before":"unknown"})),
        ("window.panel.move", json!({"panel":"layers","anchor":"properties","zone":42})),
        ("window.panel.float", json!({"panel":"layers","width":-10})),
        ("window.panel.float", json!({"panel":"unknown"})),
        ("window.panel.float", json!({"panel":"layers","x":"invalid"})),
    ] {
        let old = app.ui.docking.clone();
        let hidden = app.ui.docking_returns.clone();
        assert!(app.run(id, params).is_err(), "{id}");
        assert_eq!(app.ui.docking, old);
        assert_eq!(app.ui.docking_returns, hidden);
    }
}

fn harness(app: PhotocraftApp, size: egui::Vec2) -> Harness<'static, PhotocraftApp> {
    let mut ready = false;
    let mut harness = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            crate::panels::dock_area(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    harness.run_steps(4);
    harness
}

fn drag(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2) {
    h.event(Event::PointerMoved(from));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for step in 1..=8 {
        h.event(Event::PointerMoved(from + (to - from) * (step as f32 / 8.0)));
        h.run_steps(1);
    }
    h.event(Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

/// The canvas's rect: from the area's start to the dock column's inner edge.
fn canvas_rect(h: &Harness<'static, PhotocraftApp>) -> egui::Rect {
    let hits = craft_ui::docking::DockArea::<String>::hit_rects(&h.ctx, area_id());
    let divider = hits
        .iter()
        .find(|(hit, _)| matches!(hit, craft_ui::docking::Hit::Divider(craft_ui::docking::Divider::Column(_))))
        .map(|(_, rect)| *rect)
        .expect("the dock column's edge is drawn");
    egui::Rect::from_min_max(egui::pos2(0.0, divider.top()), egui::pos2(divider.left(), divider.bottom()))
}

/// The panels of the root split's canvas side, which a drop beside the canvas splits.
fn canvas_side(layout: &Layout<String>) -> Layout<String> {
    let Some(Node::Split { first, .. }) = &layout.root else { panic!("the canvas is beside the dock: {layout:?}") };
    Layout { root: Some(first.as_ref().clone()), floating: Vec::new() }
}

#[test]
fn a_fresh_launch_draws_the_native_groups_beside_the_protected_canvas() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    assert!(h.state().ui.docking.is_none(), "drawing the default does not customize the workspace");
    let layout = legacy(h.state());
    assert!(valid(&layout));
    assert_eq!(
        root_groups(&layout),
        [vec!["color", "swatches", "gradients", "patterns"], vec!["properties", "adjustments"], vec!["layers", "channels", "paths"]]
    );
    assert!(in_dock_column(&layout, "layers"), "the groups are a dock column: {layout:?}");
    let canvas = canvas_rect(&h);
    let tops: Vec<f32> = ["color", "properties", "layers"].iter().map(|panel| tab_rect(&h, panel).top()).collect();
    assert!(tops.windows(2).all(|pair| pair[0] < pair[1]), "top to bottom: {tops:?}");
    assert!(tab_rect(&h, "color").left() > canvas.right(), "the dock is at the end edge");
    let hits = craft_ui::docking::DockArea::<String>::hit_rects(&h.ctx, area_id());
    assert!(!hits.iter().any(|(hit, _)| *hit == craft_ui::docking::Hit::Tab(CANVAS.into())), "the canvas has no tab");
    for command in ["window.panel.close", "window.panel.float"] {
        assert!(h.state_mut().run(command, json!({"panel": CANVAS})).is_err(), "{command}");
    }
    h.state_mut().run("window.panel.move", json!({"panel":"layers","anchor":CANVAS,"zone":"center"})).unwrap_err();
    assert!(h.state().ui.docking.is_none(), "a refused command leaves the default");

    let mut studio = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    studio.ui.theme = crate::theme::ThemeKind::Studio;
    let layout = legacy(&studio);
    let window = layout.floating.first().expect("Studio floats Properties");
    assert_eq!((window.id, window.panels().cloned().collect::<Vec<_>>()), (PROPERTIES_WINDOW, vec!["properties".to_string(), "adjustments".into()]));
    assert_eq!(legacy(&studio).floating, layout.floating, "the same window every frame");

    h.state_mut().ui.dock.set_collapsed(Group::Color, true);
    h.run_steps(2);
    assert!(collapsed_members(h.state(), &legacy(h.state())).contains("swatches"));
    let color = tab_rect(&h, "color");
    assert!(tab_rect(&h, "properties").top() - color.bottom() < 12.0, "a collapsed group shows only its tabs");
}

/// A panel no saved workspace has placed opens in its native group, or as that group in its
/// native place in the dock column, not as a tab of whichever group comes first.
#[test]
fn a_panel_never_laid_out_opens_in_its_default_group_and_side() {
    let mut app = app();
    app.ui.docking = Some(Workspace::new(legacy(&app)));
    app.run("window.panel.activate", json!({"panel":"navigator"})).unwrap();
    let layout = lay(&app);
    assert!(in_dock_column(layout, "navigator"), "{layout:?}");
    assert_eq!(
        root_groups(layout),
        [vec!["color", "swatches", "gradients", "patterns"], vec!["properties", "adjustments"], vec!["navigator"], vec!["layers", "channels", "paths"]]
    );
    app.run("window.panel.activate", json!({"panel":"info"})).unwrap();
    app.run("window.panel.activate", json!({"panel":"histogram"})).unwrap();
    assert_eq!(group_members(lay(&app), "info"), ["navigator", "histogram", "info"]);
    app.run("window.panel.activate", json!({"panel":"brushes"})).unwrap();
    assert_eq!(root_groups(lay(&app)).last().unwrap(), &["brushes"]);
}

#[test]
fn a_tab_dropped_on_the_canvas_floats_there_and_redocks() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    let canvas = canvas_rect(&h);
    let source = tab_rect(&h, "layers").center();
    let target = egui::pos2(canvas.left() + canvas.width() / 6.0, canvas.center().y);
    drag(&mut h, source, target);
    let layout = lay(h.state());
    assert!(valid(layout));
    assert!(!canvas_side(layout).contains(&"layers".to_string()) && !in_dock_column(layout, "layers"), "{layout:?}");
    assert!(layout.floating.iter().any(|window| window.panels().eq(["layers".to_string()].iter())), "{layout:?}");
    assert!(tab_rect(&h, "layers").expand(4.0).contains(target), "the tab stays under the pointer");

    let source = tab_rect(&h, "layers").center();
    let destination = tab_rect(&h, "properties").center();
    drag(&mut h, source, destination);
    let layout = lay(h.state());
    let mut members = group_members(layout, "layers");
    members.sort();
    assert_eq!(members, ["adjustments", "layers", "properties"]);
    assert!(in_dock_column(layout, "layers") && valid(layout));
}

#[test]
fn brushes_tab_drag_opens_above_the_canvas_and_can_redock_into_another_tab_group() {
    let mut app = app();
    crate::menus::invoke(&mut app, &egui::Context::default(), "window.panel.brushes", json!({})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let canvas = canvas_rect(&h);
    let source = tab_rect(&h, "brushes").center();
    // The outer edge band along the canvas column's top.
    drag(&mut h, source, egui::pos2(canvas.center().x, canvas.top() + 6.0));
    let layout = lay(h.state());
    assert!(canvas_side(layout).contains(&"brushes".to_string()), "{layout:?}");
    assert!(!layout.floating.iter().any(|window| window.contains("brushes")));
    let canvas_body = last_bodies(&h.ctx).into_iter().find(|(id, _)| id == CANVAS).expect("the canvas is drawn").1;
    assert!(tab_rect(&h, "brushes").bottom() <= canvas_body.top(), "brushes is above the canvas");

    let source = tab_rect(&h, "brushes").center();
    let destination = tab_rect(&h, "layers").center();
    drag(&mut h, source, destination);
    let layout = lay(h.state());
    assert!(in_dock_column(layout, "brushes"));
    assert!(valid(layout));
}

#[test]
fn saved_workspace_restores_the_custom_tree_and_hidden_panel_placement() {
    let mut app = app();
    let ctx = egui::Context::default();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    app.run("window.panel.float", json!({"panel":"properties","x":50,"y":80})).unwrap();
    app.run("window.panel.close", json!({"panel":"swatches"})).unwrap();
    let layout = app.ui.docking.clone();
    let hidden = app.ui.docking_returns.clone();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({"name":"Custom docking"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
    assert!(app.ui.docking.is_none());
    crate::menus::invoke(&mut app, &ctx, "window.workspace.select", json!({"name":"Custom docking"})).unwrap();
    assert_eq!(app.ui.docking, layout);
    assert_eq!(app.ui.docking_returns, hidden);
}

#[test]
fn shared_layout_keeps_panel_identity_across_themes_and_respects_workspace_lock() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"color","anchor":"layers"})).unwrap();
    let layout = app.ui.docking.clone();
    app.ui.theme = crate::theme::ThemeKind::Studio;
    assert_eq!(app.ui.docking, layout);
    app.session.prefs.edit(|prefs| prefs.workspace_locked = true);
    assert!(app.run("window.panel.float", json!({"panel":"color"})).is_err());
    assert_eq!(app.ui.docking, layout);
    assert!(app.run("window.panel.activate", json!({"panel":"color"})).is_ok());
    assert!(!crate::menus::is_enabled(&app, "window.panel.window"));
    assert!(crate::menus::is_enabled(&app, "window.panel.activate"));
}

/// Explicit offscreen evidence; this creates no native windows.
#[test]
fn capture_custom_panel_docking_visual_fixtures() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(directory) = std::env::var_os("CRAFT_UI_DOCKING_FIXTURES").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&directory).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        for width in [800.0, 1280.0] {
            for scale in [1.0, 1.5, 2.0] {
                let mut app = app();
                app.run("file.new", json!({})).unwrap();
                app.run("window.panel.move", json!({"panel":"swatches", "anchor":"layers", "zone":"top"})).unwrap();
                app.run("window.panel.float", json!({"panel":"properties","x":30,"y":90,"width":300,"height":440})).unwrap();
                let mut ready = false;
                let mut harness = Harness::builder().with_size(vec2(width, 800.0)).with_pixels_per_point(scale).wgpu().build_ui_state(
                    move |ui, app: &mut PhotocraftApp| {
                        if !ready {
                            PhotocraftApp::setup_context(ui.ctx(), theme);
                            app.ui.theme = theme;
                            ready = true;
                            return;
                        }
                        crate::panels::dock_area(app, ui);
                        egui::CentralPanel::default().show(ui, |ui| {
                            ui.label("Custom panel workspace");
                        });
                    },
                    app,
                );
                harness.input_mut().max_texture_side = Some(8192);
                harness.run_steps(5);
                assert!(valid(lay(harness.state())));
                harness.render().unwrap().save(directory.join(format!("photocraft-shared-docking-{}-{width}-{scale}x.png", theme.id()))).unwrap();
            }
        }
    }
}

#[test]
fn floating_group_titles_are_opaque_in_studio_themes() {
    let _gpu = photocraft_testkit::gpu_lock();
    // Studio's tab strip is transparent over the dock's card; a floating group must not show the canvas (#4509).
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"swatches","x":40,"y":60,"width":300,"height":300})).unwrap();
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).wgpu().build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::Studio);
                app.ui.theme = crate::theme::ThemeKind::Studio;
                ready = true;
                return;
            }
            crate::panels::dock_area(app, ui);
            egui::CentralPanel::default().frame(egui::Frame::NONE.fill(egui::Color32::from_rgb(255, 0, 255))).show(ui, |_| {});
        },
        app,
    );
    h.run_steps(5);
    let area = floating_rect(&h, "swatches");
    let image = h.render().unwrap();
    let ppp = image.width() as f32 / 1280.0;
    let strip = egui::Rect::from_min_size(area.min, vec2(area.width(), 40.0)).shrink(2.0);
    let magenta = (strip.left() as u32..strip.right() as u32)
        .flat_map(|x| (strip.top() as u32..strip.bottom() as u32).map(move |y| (x, y)))
        .filter(|&(x, y)| image.get_pixel((x as f32 * ppp) as u32, (y as f32 * ppp) as u32).0[..3] == [255, 0, 255])
        .count();
    assert_eq!(magenta, 0, "canvas shows through the floating group's title strip {strip:?}");
}

#[test]
fn escape_cancels_the_first_drag_without_replacing_the_legacy_workspace() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    let source = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerMoved(source - vec2(90.0, 0.0)));
    h.run_steps(2);
    assert!(h.state().ui.docking.is_none(), "a drag changes nothing before its drop");
    h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert!(h.state().ui.docking.is_none());
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.state().ui.docking.is_none());
}

#[test]
fn serialized_layout_actions_cover_geometry_reorder_accordion_and_atomic_rejection() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    app.run("window.panel.layout", json!({"action":{"ResizeSplit":{"path":[],"size":{"Ratio":0.35}}}})).unwrap();
    assert!(matches!(lay(&app).root, Some(Node::Split { size: craft_ui::layout::SplitSize::Ratio(value), .. }) if (value - 0.35).abs() < 0.001));
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    app.run("window.panel.layout", json!({"action":{"MoveFloating":{"panel":"properties","rect":[44,66,360,450]}}})).unwrap();
    assert_eq!(lay(&app).floating.iter().find(|group| group.contains(&"properties".to_string())).unwrap().rect, [44.0, 66.0, 360.0, 450.0]);
    app.run("window.panel.layout", json!({"action":{"Move":{"panel":"properties","anchor":"swatches","placement":{"Tab":{"before":"swatches"}}}}})).unwrap();
    assert_eq!(group_members(lay(&app), "swatches"), ["properties", "swatches"]);
    let before = app.ui.docking.clone();
    let hidden = app.ui.docking_returns.clone();
    for action in [
        json!({"MoveFloating":{"panel":"properties","rect":[0,0,-5,30]}}),
        json!({"ResizeSplit":{"path":[true,true,true],"size":{"Ratio":0.2}}}),
        json!({"Open":{"panel":"unknown","anchor":null}}),
        json!({"Move":{"panel":"properties","anchor":"swatches","placement":{"Tab":{"before":"unknown"}}}}),
    ] {
        assert!(app.run("window.panel.layout", json!({"action":action})).is_err());
        assert_eq!(app.ui.docking, before);
        assert_eq!(app.ui.docking_returns, hidden);
    }
    let stack = Node::Stack { entries: vec![craft_ui::docking::StackEntry { panel: "layers".into(), open: true, height: Some(100.0) }] };
    app.ui.docking = Some(Workspace::new(Layout { root: Some(with_canvas(stack, 300.0)), floating: Vec::new() }));
    app.run("window.panel.layout", json!({"action":{"SetStackOpen":{"panel":"layers","open":false}}})).unwrap();
    app.run("window.panel.layout", json!({"action":{"ResizeStack":{"panel":"layers","height":150}}})).unwrap();
    let Some(Node::Split { second, .. }) = &lay(&app).root else { panic!("expected the canvas split") };
    let Node::Stack { entries } = second.as_ref() else { panic!("expected stack") };
    assert!(!entries[0].open);
    assert_eq!(entries[0].height, Some(150.0));
}

#[test]
fn customization_retains_native_collapsed_groups_until_their_tab_is_activated() {
    let mut app = app();
    app.ui.dock.set_collapsed(Group::Color, true);
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let layout = lay(&app);
    assert!(collapsed_members(&app, layout).contains("swatches"));
    let saved = serde_json::to_value(&app.ui).unwrap();
    let restored: crate::state::UiState = serde_json::from_value(saved).unwrap();
    assert!(restored.dock.is_collapsed(Group::Color));
    app.run("window.panel.layout", json!({"action":{"Activate":{"panel":"swatches"}}})).unwrap();
    assert!(!app.ui.dock.is_collapsed(Group::Color));
}

#[test]
fn splitting_a_short_column_keeps_the_layers_footer_visible_and_usable() {
    let mut app = app();
    app.run("file.new", json!({"width":480,"height":360})).unwrap();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    app.run("window.panel.float", json!({"panel":"properties","x":30,"y":90,"width":300,"height":440})).unwrap();
    let mut h = harness(app, vec2(800.0, 800.0));
    let button = h.get_by_label("Create a new layer").rect();
    assert!(button.bottom() <= 800.0, "Layers footer is clipped below the viewport: {button:?}");
    let before = h.state().session.active().unwrap().doc.layers.len();
    let at = button.center();
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    assert_eq!(h.state().session.active().unwrap().doc.layers.len(), before + 1);
}

#[test]
fn populated_canvas_keeps_a_real_opacity_edit_after_floating_and_redocking() {
    let _gpu = photocraft_testkit::gpu_lock();
    let mut app = app();
    app.ui.theme = crate::theme::ThemeKind::Studio;
    app.run("file.new", json!({"width":480,"height":360})).unwrap();
    app.run("layer.new.layer", json!({"name":"Docking sample"})).unwrap();
    app.run("select.rect", json!({"x":80,"y":70,"width":250,"height":160})).unwrap();
    app.run("edit.fill", json!({"color":"#5297bb"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("window.panel.float", json!({"panel":"properties","x":40,"y":140,"width":310,"height":490})).unwrap();
    let directory = std::env::var_os("CRAFT_UI_DOCKING_FIXTURES").map(std::path::PathBuf::from);
    let builder = Harness::builder().with_size(vec2(1280.0, 800.0));
    let builder = if directory.is_some() { builder.wgpu() } else { builder };
    let mut ready = false;
    let mut h = builder.build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::Studio);
                ready = true;
                return;
            }
            crate::panels::title_bar(app, ui);
            crate::panels::options_bar(app, ui);
            crate::panels::toolbar(app, ui);
            crate::panels::dock_area(app, ui);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    h.input_mut().max_texture_side = Some(8192);
    h.run_steps(5);
    let floating = floating_rect(&h, "properties");
    let mut sliders: Vec<_> = h.ctx.viewport(|vp| {
        vp.prev_pass
            .widgets
            .layers()
            .flat_map(|(_, widgets)| widgets.iter())
            .filter(|widget| widget.sense.senses_drag() && widget.rect.height() == 18.0 && floating.contains_rect(widget.rect))
            .map(|widget| widget.rect)
            .collect()
    });
    sliders.sort_by(|a, b| a.top().total_cmp(&b.top()));
    let at = sliders.first().expect("Properties opacity slider").center();
    h.hover_at(at);
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
    let opacity = h.state().session.active().unwrap().doc.layer(layer).unwrap().opacity;
    assert!((opacity - 0.5).abs() < 0.03, "actual opacity {opacity}");
    if let Some(directory) = &directory {
        std::fs::create_dir_all(directory).unwrap();
        h.render().unwrap().save(directory.join("photocraft-populated-opacity-floating.png")).unwrap();
    }
    let from = tab_rect(&h, "properties").center();
    let to = tab_rect(&h, "layers").center();
    drag(&mut h, from, to);
    assert!(lay(h.state()).floating.iter().all(|group| !group.contains(&"properties".to_string())));
    assert_eq!(h.state().session.active().unwrap().doc.layer(layer).unwrap().opacity, opacity);
    if let Some(directory) = &directory {
        h.render().unwrap().save(directory.join("photocraft-populated-opacity-redocked.png")).unwrap();
    }
}

#[test]
fn changing_workspace_during_a_drag_cancels_the_stale_gesture() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers","zone":"top"})).unwrap();
    let destination = app.ui.docking.clone();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({"name":"Destination"})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.essentials", json!({})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let source = h.query_all_by_label("Layers").find(|node| node.rect().height() <= 45.0).unwrap().rect().center();
    h.event(Event::PointerMoved(source));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerMoved(source - vec2(90.0, 0.0)));
    h.run_steps(2);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.select", json!({"name":"Destination"})).unwrap();
    h.event(Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert_eq!(h.state().ui.docking, destination);
    h.event(Event::PointerButton { pos: source, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert_eq!(h.state().ui.docking, destination);
}

fn control_set(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Value {
    let (request, _) = crate::control::ControlRequest::new("ui.set", params);
    match crate::control::handle(app, ctx, &request) {
        crate::control::Outcome::Done(value) => value,
        _ => panic!("ui.set must reply synchronously"),
    }
}

#[test]
fn current_main_hidden_tabs_migrate_by_name_and_selected_identity() {
    for theme in [crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::Studio] {
        let mut app = app();
        app.ui.theme = theme;
        let pro = matches!(theme, crate::theme::ThemeKind::Pro | crate::theme::ThemeKind::ProMedium);
        let color = Group::Color.tabs(pro).iter().position(|name| *name == "Color").unwrap();
        let swatches = Group::Color.tabs(pro).iter().position(|name| *name == "Swatches").unwrap();
        app.ui.dock.hide_tab(Group::Color, swatches, pro);
        app.ui.dock.hide_tab(Group::Layers, 2, pro);
        app.ui.dock_tabs.color = color;
        let saved = json!({"panels":app.ui.panels,"dock":app.ui.dock,"dockTabs":app.ui.dock_tabs});
        assert_eq!(saved["dock"]["hidden_tabs"]["layers"], json!(["Paths"]));
        let mut restored = self::app();
        restored.ui.theme = theme;
        crate::dock::apply(&mut restored, &saved);
        restored.run("window.panel.float", json!({"panel":"properties"})).unwrap();
        let layout = lay(&restored);
        assert!(!layout.contains(&"swatches".into()) && !layout.contains(&"paths".into()));
        assert!(panel_active(layout, "color"));
        assert!(hidden(&restored, "swatches") && hidden(&restored, "paths"));
        restored.run("window.panel.swatches", json!({"show":true})).unwrap();
        assert!(panel_active(lay(&restored), "swatches"));
    }
}

#[test]
fn legacy_control_updates_authoritative_custom_visibility_and_selection_atomically() {
    let mut app = app();
    app.ui.theme = crate::theme::ThemeKind::ProMedium;
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(control_set(&mut app, &ctx, json!({"dockTabs":{"layers":1}}))["ok"], true);
    assert!(panel_active(lay(&app), "channels"));
    assert_eq!(control_set(&mut app, &ctx, json!({"panels":{"layers":false}}))["ok"], true);
    assert!(PANELS.iter().filter(|p| p.2 == Group::Layers).all(|p| !lay(&app).contains(&p.0.into())));
    assert!(!app.ui.panels.layers);
    assert_eq!(control_set(&mut app, &ctx, json!({"panels":{"layers":true},"dockTabs":{"layers":2}}))["ok"], true);
    assert!(panel_active(lay(&app), "paths"));
    let before = serde_json::to_value(&app.ui).unwrap();
    for request in [
        json!({"panels":{"color":false},"dockTabs":{"layers":999}}),
        json!({"tool":"brush","dock":{"order":["layers","color"]}}),
        json!({"panels":{"layers":false},"dockTabs":{"layers":1}}),
    ] {
        assert_eq!(control_set(&mut app, &ctx, request)["ok"], false);
        assert_eq!(serde_json::to_value(&app.ui).unwrap(), before);
    }
}

#[test]
fn legacy_dock_width_is_consumed_by_custom_renderer_and_clamped_to_existing_range() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let ctx = h.ctx.clone();
    // A 1280-point window: the dock may take all but 400 points (#7896), so 700 is no longer
    // cut to 520.
    for (requested, expected) in [(410.0, 410.0), (700.0, 700.0), (900.0, 880.0), (20.0, 250.0)] {
        assert_eq!(control_set(h.state_mut(), &ctx, json!({"dockWidth":requested}))["ok"], true);
        h.run_steps(2);
        let Some(Node::Split { second, .. }) = &lay(h.state()).root else { panic!("expected the canvas split") };
        let Node::Dock { columns, .. } = second.as_ref() else { panic!("expected the dock") };
        assert!((columns[0].width - expected).abs() < 1.0, "{} / {expected}", columns[0].width);
        assert!(h.ctx.data(|data| data.get_temp::<f32>(crate::panels::dock_width_id())).is_none());
    }
}

#[test]
fn explicit_reveal_activates_existing_tab_and_expands_custom_group() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"properties","anchor":"layers"})).unwrap();
    app.run("window.panel.group", json!({"panel":"layers","operation":"collapse"})).unwrap();
    app.run("window.panel.layers", json!({"show":true})).unwrap();
    assert!(panel_active(lay(&app), "layers"));
    assert!(!collapsed_members(&app, lay(&app)).contains("layers"));
    app.run("window.panel.layers", json!({"show":true})).unwrap();
    assert!(lay(&app).contains(&"layers".into()));
    let before = app.ui.docking.clone();
    assert!(app.run("window.panel.layers", json!({"show":"yes"})).is_err());
    assert_eq!(app.ui.docking, before);
}

// #2455 in a custom workspace: the pro Color field fills a tall floating group, not 120 pt.
#[test]
fn pro_color_field_fills_a_tall_custom_group() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"color","x":40,"y":60,"width":320,"height":560})).unwrap();
    assert!(app.ui.docking.is_some());
    let h = harness(app, vec2(1280.0, 800.0));
    // The SV field and the 14 pt hue strip beside it are draggable, equally tall and 8 pt apart.
    let drags: Vec<egui::Rect> =
        h.ctx.viewport(|v| v.this_pass.widgets.layers().flat_map(|(_, w)| w).filter(|w| w.sense.senses_drag()).map(|w| w.rect).collect());
    let field = drags
        .iter()
        .find(|f| drags.iter().any(|s| s.width() == 14.0 && s.top() == f.top() && s.height() == f.height() && (s.left() - f.right() - 8.0).abs() < 0.5))
        .expect("Color field with its hue strip");
    assert!(field.height() > 300.0, "field stays short in a 560 pt group: {field:?}");
}

// #4028: in a custom workspace too, a new adjustment layer brings Properties to the front.
#[test]
fn a_new_adjustment_layer_activates_properties_in_a_custom_workspace() {
    let mut app = app();
    app.run("window.panel.activate", json!({"panel":"adjustments"})).unwrap();
    assert!(panel_active(lay(&app), "adjustments"));
    app.run("file.new", json!({"width":64,"height":64})).unwrap();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    assert!(panel_active(lay(&app), "properties"));
}

#[test]
fn custom_group_commands_preserve_members_and_workspace_timeline() {
    let mut app = app();
    app.run("window.panel.move", json!({"panel":"properties","anchor":"layers"})).unwrap();
    let members = group_members(lay(&app), "layers");
    app.run("window.panel.group", json!({"panel":"layers","operation":"up"})).unwrap();
    assert_eq!(group_members(lay(&app), "layers"), members);
    app.ui.timeline.open = true;
    app.run("window.panel.group", json!({"panel":"layers","operation":"collapse"})).unwrap();
    let saved = json!({"panels":app.ui.panels,"dock":app.ui.dock,"dockTabs":app.ui.dock_tabs,"docking":app.ui.docking,"dockingReturns":app.ui.docking_returns,"dockingCollapsed":app.ui.docking_collapsed,"timelineOpen":true});
    let mut restored = self::app();
    crate::dock::apply(&mut restored, &saved);
    assert_eq!(restored.ui.docking, app.ui.docking);
    assert!(restored.ui.timeline.open);
    assert!(collapsed_members(&restored, lay(&restored)).contains("layers"));
    app.run("window.panel.group", json!({"panel":"layers","operation":"close"})).unwrap();
    assert!(members.iter().all(|id| !lay(&app).contains(id)));
    assert!(members.iter().all(|id| hidden(&app, id)));
    app.session.prefs.edit(|p| p.workspace_locked = true);
    let before = serde_json::to_value(&app.ui).unwrap();
    assert!(app.run("window.panel.group", json!({"panel":"color","operation":"close"})).is_err());
    assert_eq!(serde_json::to_value(&app.ui).unwrap(), before);
    restored.ui.timeline.playing = true;
    crate::dock::apply(&mut restored, &json!({"dock":{}}));
    assert!(!restored.ui.timeline.open && !restored.ui.timeline.playing);
}

#[test]
fn customized_panel_menus_and_rail_remain_reachable_after_float_and_restore() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"swatches","x":20,"y":40})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    assert!(h.query_all_by_label("Panel menu").count() > 0);
    assert!(h.query_all_by_label("Color & Swatches").count() > 0);
    let floating = floating_rect(&h, "swatches");
    let first = h.query_all_by_label("Panel menu").find(|node| floating.contains_rect(node.rect())).unwrap().rect().center();
    h.hover_at(first);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: first, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
    assert!(h.query_all_by_label("Close Tab Group").count() > 0);
    assert!(h.query_all_by_label("New Swatch…").count() > 0);
    // Panel-specific menu body is shared by both its context and group menu seams.
    let mut ready = false;
    let mut menu = Harness::builder().build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
            }
            panel_menu(app, ui, "swatches");
        },
        self::app(),
    );
    menu.run_steps(2);
    assert!(menu.query_all_by_label("New Swatch…").count() > 0);
}

/// A floated Color panel's menu still picks its mode (#6000): the shared docking renderer has
/// its own panel menu, separate from the legacy dock's.
#[test]
fn floated_color_panel_menu_picks_the_mode() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"color","x":20,"y":40})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let floating = floating_rect(&h, "color");
    let button = h.query_all_by_label("Panel menu").find(|node| floating.contains_rect(node.rect())).unwrap().rect().center();
    h.hover_at(button);
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: button, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    h.run_steps(2);
    h.get_by_label("   RGB Sliders").click();
    h.run_steps(2);
    assert_eq!(h.state().ui.color_panel.mode, crate::state::ColorPanelMode::RgbSliders);
}

#[test]
fn actions_panel_menu_includes_load_and_save_actions() {
    let mut ready = false;
    let mut menu = Harness::builder().build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
            }
            panel_menu(app, ui, "actions");
        },
        self::app(),
    );
    menu.run_steps(2);
    assert!(menu.query_all_by_label("Load Actions…").count() > 0);
    assert!(menu.query_all_by_label("Save Actions…").count() > 0);
    assert!(menu.query_all_by_label("New Action…").count() > 0);
}

#[test]
fn custom_workspace_saves_shared_layout_and_timeline_without_embedding_flags() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    app.ui.timeline.open = true;
    app.ui.panels.rail = false;
    app.ui.panels.menu_bar = false;
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "window.workspace.newWorkspace", json!({"name":"Docked Motion"})).unwrap();
    let saved = app.session.prefs().workspaces.get("Docked Motion").unwrap().clone();
    assert!(saved["docking"].is_object() && saved["timelineOpen"] == true);
    assert!(saved["panels"].get("rail").is_none() && saved["panels"].get("menu_bar").is_none());
    let layout = app.ui.docking.clone();
    app.ui.workspace = "Motion".into();
    crate::menus::apply_workspace(&mut app);
    assert!(app.ui.docking.is_none() && app.ui.timeline.open);
    crate::dock::apply(&mut app, &saved);
    assert_eq!(app.ui.docking, layout);
    assert!(app.ui.timeline.open);
}

#[test]
fn closing_and_reopening_a_whole_group_retains_its_return_position_and_tab_order() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let before = lay(&app).clone();
    let members = group_members(&before, "layers");
    app.run("window.panel.group", json!({"panel":"layers","operation":"close"})).unwrap();
    for panel in &members {
        app.run("window.panel.activate", json!({"panel":panel})).unwrap();
    }
    let after = lay(&app);
    assert_eq!(group_members(after, "layers"), members);
    assert_eq!(root_groups(after), root_groups(&before));
    assert!(valid(after));
}

#[test]
fn floating_group_grip_redocks_all_tabs_as_one_atomic_move() {
    let mut app = app();
    app.run("window.panel.float", json!({"panel":"swatches","x":30,"y":60})).unwrap();
    app.run("window.panel.move", json!({"panel":"patterns","anchor":"swatches"})).unwrap();
    let mut h = harness(app, vec2(1280.0, 800.0));
    let from = hit_rect(&h, craft_ui::docking::Hit::DragBar("swatches".into())).center();
    let to = tab_rect(&h, "layers").center();
    drag(&mut h, from, to);
    let layout = lay(h.state());
    assert!(layout.floating.iter().all(|group| !group.contains(&"swatches".to_string()) && !group.contains(&"patterns".to_string())));
    let members = group_members(layout, "layers");
    let swatches = members.iter().position(|id| id == "swatches").unwrap();
    assert_eq!(members.get(swatches + 1).map(String::as_str), Some("patterns"));
    assert!(valid(layout));
    let before = h.state().ui.docking.clone();
    assert!(h.state_mut().run("window.panel.group", json!({"panel":"swatches","operation":"move","anchor":"patterns"})).is_err());
    assert_eq!(h.state().ui.docking, before);
}

#[test]
fn mixed_theme_and_legacy_selection_projects_the_selected_panel_in_the_new_theme() {
    let mut app = app();
    app.ui.theme = crate::theme::ThemeKind::ProMedium;
    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let ctx = egui::Context::default();
    assert_eq!(control_set(&mut app, &ctx, json!({"theme":"studio","dockTabs":{"color":0}}))["ok"], true);
    assert!(panel_active(lay(&app), "swatches"));
    assert_eq!(app.ui.dock_tabs.color, 0);
    assert_eq!(control_set(&mut app, &ctx, json!({"theme":"proMedium","dockTabs":{"color":0}}))["ok"], true);
    assert!(panel_active(lay(&app), "color"));
    assert_eq!(app.ui.dock_tabs.color, 0);
}

/// #7896: the dock's widest is the window less what the toolbar and the canvas keep, never
/// below the old 520 points, so a wide window allows a two-column dock.
#[test]
fn the_dock_can_be_as_wide_as_the_window_allows() {
    for (window, max) in [(2560.0, 2160.0), (1280.0, 880.0), (800.0, 520.0), (f32::NAN, 520.0)] {
        let range = crate::panels::dock_width_range(window);
        assert_eq!((*range.start(), *range.end()), (crate::panels::DOCK_MIN_WIDTH, max), "{window}");
    }
}

/// A workspace file written by main: a bare `docking` layout with `dockingHidden` and
/// `dockingCollapsed` beside the native keys. It loads as the default built from the native
/// keys, and the first change saves a versioned workspace.
#[test]
fn main_workspace_file_loads_as_the_default_and_saves_as_a_workspace() {
    let saved: Value = serde_json::from_str(include_str!("../../../tests/fixtures/main-docking-workspace.json")).unwrap();
    let mut app = app();
    app.ui.theme = crate::theme::ThemeKind::ProMedium;
    crate::dock::apply(&mut app, &saved);
    assert!(app.ui.docking.is_none() && app.ui.docking_returns.is_empty() && app.ui.docking_collapsed.is_empty());
    let workspace = legacy_workspace(&app);
    let layout = &workspace.layout;
    assert_eq!(
        root_groups(layout),
        [vec!["color", "swatches", "gradients", "patterns"], vec!["properties", "adjustments"], vec!["history", "actions"], vec!["layers", "channels"]]
    );
    assert!(in_dock_column(layout, "layers") && panel_active(layout, "actions"));
    for (panel, _, _) in PANELS {
        let panel = panel.to_string();
        assert!(layout.contains(&panel) || workspace.hidden.iter().any(|(id, _)| *id == panel), "{panel} is shown or can be reopened");
    }

    app.run("window.panel.float", json!({"panel":"properties"})).unwrap();
    let written = crate::dock::snapshot(&app);
    assert!(written.get("dockingHidden").is_none());
    assert_eq!(written["docking"]["version"], 2);
    let mut reloaded = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    crate::dock::apply(&mut reloaded, &written);
    let again = reloaded.ui.docking.as_ref().expect("the workspace loads as saved");
    let now = app.ui.docking.as_ref().unwrap();
    assert_eq!((&again.layout, &again.hidden), (&now.layout, &now.hidden));
    assert_eq!(reloaded.ui.docking_returns, app.ui.docking_returns);
}

/// The width a user gave the native dock, kept in egui's memory, sizes the default column once.
#[test]
fn the_native_dock_width_sizes_the_default_column() {
    let mut h = harness(app(), vec2(1280.0, 800.0));
    let state = egui::PanelState { outer_rect: egui::Rect::from_min_size(egui::pos2(900.0, 0.0), vec2(380.0, 700.0)) };
    let key = crate::panels::panel_state_id(&h.ctx, "dock");
    h.ctx.data_mut(|data| data.insert_persisted(key, state));
    h.run_steps(2);
    let Some(Node::Split { second, .. }) = &lay(h.state()).root else { panic!("expected the canvas split") };
    let Node::Dock { columns, .. } = second.as_ref() else { panic!("expected the dock") };
    assert_eq!(columns[0].width, 380.0);
    assert!(h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).is_none(), "taken once");
}
