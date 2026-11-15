use super::*;
use egui::{Event, Modifiers, PointerButton, Pos2, pos2, vec2};
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
            .filter(|widget| {
                widget.sense.senses_drag()
                    && widget.rect.height() == crate::widgets::TEXT_FIELD_H
                    && widget.rect.width() > floating.width() * 0.5
                    && floating.contains_rect(widget.rect)
            })
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

/// The Channels panel menu (☰) has the channel items, not only the rows' context menu (#11451):
/// New Spot Channel… and New Channel… open their dialogs, and OK in New Channel adds a channel.
#[test]
fn channels_panel_menu_opens_new_spot_and_adds_a_channel() {
    use egui_kittest::kittest::NodeT;
    let mut app = self::app();
    app.run("file.new", json!({"width": 32, "height": 24})).unwrap();
    let mut ready = false;
    let mut menu = Harness::builder().with_size(vec2(260.0, 420.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
            }
            panel_menu(app, ui, "channels");
        },
        app,
    );
    menu.run_steps(2);
    let disabled = |menu: &Harness<'static, PhotocraftApp>, label: &str| menu.get_by_label(label).accesskit_node().is_disabled();
    // The composite is targeted: nothing to duplicate, delete or give options to.
    for label in ["Duplicate Channel", "Delete Channel", "Merge Spot Channel", "Channel Options…"] {
        assert!(disabled(&menu, label), "{label} waits for a targeted channel");
    }
    menu.get_by_label("New Spot Channel…").click();
    menu.run_steps(2);
    let dialogs = &menu.state().ui.dialogs;
    assert!(dialogs.iter().any(|d| crate::channel_options_ui::owns_new_spot(&d.fields)), "New Spot Channel… opens its dialog");
    menu.state_mut().ui.dialogs.clear();
    menu.get_by_label("New Channel…").click();
    menu.run_steps(2);
    assert!(menu.state().session.active().unwrap().doc.channels.is_empty(), "New Channel… asks first");
    let dialog = menu.state().ui.dialogs.last().expect("the New Channel dialog");
    assert_eq!(dialog.fields["__label"], json!("New Channel…"));
    let id = dialog.id;
    crate::dialogs::confirm(menu.state_mut(), id).unwrap();
    assert_eq!(menu.state().session.active().unwrap().doc.channels.len(), 1, "OK adds an alpha channel");
    assert!(!menu.state().ui.status_error, "{}", menu.state().ui.status);
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

/// The dock-edge layout is only chosen for a horizontal root split whose canvas side is the lone
/// canvas and whose other side is the end dock itself.
#[test]
fn the_dock_edge_layout_only_applies_with_the_canvas_alone() {
    let tabs = |panel: &str| Node::Tabs { panels: vec![panel.to_owned()], active: 0 };
    let dock = |panels: Vec<&str>, width: f32| Node::Dock {
        side: Side::End,
        columns: vec![Column::new(vec![craft_ui::docking::Group::new(panels.into_iter().map(str::to_owned).collect(), 0)], width)],
    };
    let aside = |first: Node<String>| Node::Split {
        axis: SplitAxis::Horizontal,
        size: SplitSize::FixedSecond(280.0),
        first: Box::new(first),
        second: Box::new(dock(vec!["color"], 280.0)),
    };

    let alone = Layout { root: Some(aside(tabs(CANVAS))), floating: Vec::new() };
    assert!(canvas_alone_layout(&alone), "{alone:?}");

    // A panel docked beside the canvas splits the canvas side: not alone any more.
    let beside =
        Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedFirst(400.0), first: Box::new(tabs("layers")), second: Box::new(tabs(CANVAS)) };
    let layout = Layout { root: Some(aside(beside)), floating: Vec::new() };
    assert!(!canvas_alone_layout(&layout), "{layout:?}");

    // The second child must be the end dock itself: a trailing split or tabs cannot be resized,
    // so the handle would snap back on release.
    let not_a_dock = Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedSecond(280.0),
            first: Box::new(tabs(CANVAS)),
            second: Box::new(tabs("color")),
        }),
        floating: Vec::new(),
    };
    assert!(!canvas_alone_layout(&not_a_dock), "{not_a_dock:?}");

    // A vertical root split with the canvas above another panel is not the canvas-beside-dock shape.
    let vertical = Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Vertical,
            size: SplitSize::FixedSecond(280.0),
            first: Box::new(tabs(CANVAS)),
            second: Box::new(dock(vec!["color"], 280.0)),
        }),
        floating: Vec::new(),
    };
    assert!(!canvas_alone_layout(&vertical), "{vertical:?}");
}

/// The end dock is followed through horizontal splits only; a vertical or tabs sibling is not an
/// end dock, so resizing it returns false instead of half-applying.
#[test]
fn the_end_dock_is_only_the_horizontal_split_s_second_child() {
    let tabs = |panel: &str| Node::Tabs { panels: vec![panel.to_owned()], active: 0 };
    let dock = Node::Dock { side: Side::End, columns: vec![Column::new(vec![craft_ui::docking::Group::new(vec!["color".to_owned()], 0)], 260.0)] };
    let mut vertical =
        Node::Split { axis: SplitAxis::Vertical, size: SplitSize::FixedSecond(300.0), first: Box::new(tabs(CANVAS)), second: Box::new(dock.clone()) };
    assert!(end_dock_node(&vertical).is_none(), "a vertical split has no end dock");
    let mut actual = 0.0;
    assert!(!resize_end_dock(&mut vertical, 320.0, &mut actual), "resizing a vertical dock must not follow the split");

    let mut horizontal =
        Node::Split { axis: SplitAxis::Horizontal, size: SplitSize::FixedSecond(300.0), first: Box::new(tabs(CANVAS)), second: Box::new(dock) };
    assert!(end_dock_node(&horizontal).is_some());
    let mut actual = 0.0;
    assert!(resize_end_dock(&mut horizontal, 320.0, &mut actual));
    assert!((actual - 320.0).abs() < 0.5, "the end dock takes the requested width, got {actual}");
}

/// The dock-only mode is an argument of one call, so a dock-only frame never leaves a later
/// standard layout without its canvas.
#[test]
fn dock_only_frames_never_leak_into_the_standard_layout() {
    let mut ready = false;
    let dock_only = std::rc::Rc::new(std::cell::Cell::new(false));
    let mode = dock_only.clone();
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            if mode.get() { show_dock(app, ui) } else { show(app, ui) }
        },
        app(),
    );
    let canvas_drawn = |h: &Harness<'static, PhotocraftApp>| last_bodies(&h.ctx).iter().any(|(id, _)| id == CANVAS);
    h.run_steps(2);
    for _ in 0..3 {
        dock_only.set(true);
        h.step();
        assert!(!canvas_drawn(&h), "show_dock draws no canvas");
        dock_only.set(false);
        h.step();
        assert!(canvas_drawn(&h), "the standard layout after a dock-only frame keeps its canvas");
    }
}

/// `ui.set` still refuses an unknown toolbar side, though saved panels read one as `Start`.
#[test]
fn ui_set_refuses_an_unknown_toolbar_side() {
    let mut app = app();
    let ctx = egui::Context::default();
    for bad in [json!("diagonal"), json!(5), json!(null)] {
        let reply = control_set(&mut app, &ctx, json!({"panels": {"toolbar_side": bad, "layers": false}}));
        assert_ne!(reply["ok"], true, "{bad}: {reply}");
        assert_eq!(app.ui.panels.toolbar_side, crate::state::ToolbarSide::Start);
        assert!(app.ui.panels.layers, "{bad}: a refused request changes nothing");
    }
    let reply = control_set(&mut app, &ctx, json!({"panels": {"toolbar_side": "right"}}));
    assert_eq!(reply["ok"], true, "{reply}");
    assert_eq!(app.ui.panels.toolbar_side, crate::state::ToolbarSide::Right);
}

/// The toolbar lands on the requested physical side in every interface theme.
#[test]
fn the_toolbar_side_toggle_works_in_every_interface_theme() {
    use crate::state::ToolbarSide;

    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            crate::panels::toolbar(app, ui);
        },
        app(),
    );
    for kind in crate::theme::ThemeKind::ALL {
        crate::theme::apply(&h.ctx, kind);
        h.state_mut().ui.panels.toolbar_side = ToolbarSide::Left;
        h.run_steps(2);
        // The header's chevron carries the toolbar's label; its screen X is the panel's side.
        let left = h.get_by_label("Toolbar").rect().center().x;
        assert!(left < 640.0, "{kind:?}: left toolbar at {left}");

        // The dock button itself moves the toolbar, rather than the test setting the state.
        h.get_by_label("Move Toolbar to the Right").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.panels.toolbar_side, ToolbarSide::Right, "{kind:?}: the button moves the toolbar right");
        let right = h.get_by_label("Toolbar").rect().center().x;
        assert!(right > 640.0, "{kind:?}: right toolbar at {right}");
        // A grouped tool's flyout opens towards the canvas, to the left of its button.
        h.query_by_label_contains("Rectangular Marquee Tool").expect("the marquee slot").click_secondary();
        h.run_steps(2);
        let (key, button) = h.ctx.data(|d| d.get_temp::<(egui::Id, egui::Rect)>(egui::Id::unique("tool-flyout"))).expect("the flyout is open");
        let flyout = h.ctx.memory(|m| m.area_rect(key.with("flyout"))).expect("the flyout is drawn");
        assert!(flyout.right() <= button.left(), "{kind:?}: flyout {flyout:?} left of its button {button:?}");
        h.query_by_label_contains("Rectangular Marquee Tool").expect("the marquee slot").click_secondary();
        h.run_steps(2);

        h.get_by_label("Move Toolbar to the Left").click();
        h.run_steps(2);
        assert_eq!(h.state().ui.panels.toolbar_side, ToolbarSide::Left, "{kind:?}: the button moves the toolbar left");
        let back = h.get_by_label("Toolbar").rect().center().x;
        assert!(back < 640.0, "{kind:?}: left toolbar again at {back}");
    }
}

/// Moving the toolbar persists at click time, even when the dock is not drawn.
#[test]
fn moving_the_toolbar_persists_without_the_dock() {
    use crate::state::ToolbarSide;
    let mut app = app();
    app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = true);
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            crate::panels::toolbar(app, ui);
        },
        app,
    );
    h.run_steps(2);
    h.get_by_label("Move Toolbar to the Right").click();
    h.run_steps(1);
    assert_eq!(h.state().ui.panels.toolbar_side, ToolbarSide::Right);
    let saved = &h.state().session.prefs().panel_layout;
    assert_eq!(saved["panels"]["toolbar_side"], "right", "the toolbar side is persisted without the dock: {saved}");
}

/// A dock-edge harness: draw only the dock column, as the dock-edge frame layout does.
fn dock_panel_harness(app: PhotocraftApp, size: egui::Vec2, theme: crate::theme::ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut ready = false;
    let mut harness = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), theme);
                ready = true;
                return;
            }
            crate::panels::dock_panel(app, ui);
        },
        app,
    );
    harness.run_steps(4);
    harness
}

/// [`dock_panel_harness`] in the Pro Medium theme.
fn dock_edge_harness(app: PhotocraftApp, size: egui::Vec2) -> Harness<'static, PhotocraftApp> {
    dock_panel_harness(app, size, crate::theme::ThemeKind::ProMedium)
}

/// The dock-edge handle resizes the dock and keeps the new width after release; a locked
/// workspace makes the same drag a no-op.
#[test]
fn the_dock_edge_handle_resizes_and_a_locked_workspace_refuses_it() {
    let drag_once = |locked: bool| -> (f32, f32, f32) {
        let mut app = app();
        app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers"})).unwrap();
        if locked {
            app.session.prefs.edit(|p| p.workspace_locked = true);
        }
        let mut h = dock_edge_harness(app, vec2(1280.0, 800.0));
        let stored = dock_width(h.state());
        let rail = if h.state().ui.panels.rail { if h.state().ui.theme.is_pro() { 36.0 } else { 44.0 } } else { 0.0 };
        let edge = 1280.0 - (stored + rail);
        drag(&mut h, pos2(edge, 400.0), pos2(edge + 60.0, 400.0));
        let resized = dock_width(h.state());
        h.run_steps(4);
        let settled = dock_width(h.state());
        (stored, resized, settled)
    };

    let (_, resized, settled) = drag_once(false);
    assert!(resized < 290.0, "dragging right should shrink the dock, got {resized}");
    assert!((settled - resized).abs() < 1.0, "the resized width must not snap back: {resized} then {settled}");

    let (locked_stored, locked_resized, locked_settled) = drag_once(true);
    assert!((locked_resized - locked_settled).abs() < 1.0);
    assert!((locked_resized - locked_stored).abs() < 1.0, "a locked workspace keeps its pre-drag width: {locked_stored} then {locked_resized}");
}

/// Asking for the width the dock already has is a no-op and must not create a customized
/// workspace: a later `ui.set {dockTabs}` then keeps its legacy collapse handling.
#[test]
fn setting_the_current_dock_width_keeps_the_legacy_layout() {
    let mut app = app();
    let current = dock_width(&app);
    assert!(app.ui.docking.is_none(), "the app starts on the native dock");
    set_dock_width(&mut app, current);
    assert!(app.ui.docking.is_none(), "a same-width request must not create a workspace");

    // The next legacy operation must still take the legacy path: selecting a tab in a collapsed
    // group selects it without expanding the group or customizing the workspace.
    app.ui.dock.set_collapsed(crate::dock::Group::Properties, true);
    let ctx = egui::Context::default();
    let (req, _rx) = crate::control::ControlRequest::new("ui.set", json!({"dockTabs": {"properties": 1}}));
    crate::control::handle(&mut app, &ctx, &req);
    assert_eq!(app.ui.dock_tabs.properties, 1, "the requested tab is selected");
    assert!(app.ui.dock.is_collapsed(crate::dock::Group::Properties), "the collapsed group stays collapsed");
    assert!(app.ui.docking.is_none(), "tab selection still did not customize the workspace");
}

/// Screen picking disables the parent UI mid-drag, so the handle never sees the release. The
/// stored drag must be dropped, saves must resume, and a later pointer must not apply the old
/// start.
#[test]
fn disabling_the_ui_cancels_a_dock_edge_drag() {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};

    let mut app = app();
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers"})).unwrap();
    app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = true);
    let disabled = Arc::new(AtomicBool::new(false));
    let flag = disabled.clone();
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            if flag.load(Ordering::Relaxed) {
                ui.disable();
            }
            crate::panels::dock_panel(app, ui);
        },
        app,
    );
    h.run_steps(4);
    let stored = dock_width(h.state());
    let before = h.state().session.prefs().panel_layout.clone();
    let rail = if h.state().ui.panels.rail { if h.state().ui.theme.is_pro() { 36.0 } else { 44.0 } } else { 0.0 };
    let edge = 1280.0 - (stored + rail);
    h.event(Event::PointerMoved(pos2(edge, 400.0)));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: pos2(edge, 400.0), button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerMoved(pos2(edge + 40.0, 400.0)));
    h.run_steps(2);
    assert!(crate::panels::dock_edge_drag_active(&h.ctx), "the resize drag started");
    assert_eq!(h.state().session.prefs().panel_layout, before, "a held drag is not saved");

    disabled.store(true, Ordering::Relaxed);
    h.run_steps(1);
    assert!(!crate::panels::dock_edge_drag_active(&h.ctx), "disabling cancels the held drag before any release");
    h.event(Event::PointerButton { pos: pos2(edge + 40.0, 400.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(!crate::panels::dock_edge_drag_active(&h.ctx), "a disabled UI clears the stored drag");
    // Saves resume once the drag is dropped, even while the UI stays disabled: the width the
    // held drag reached, which was not saved while held, is saved now.
    let saved = &h.state().session.prefs().panel_layout;
    assert_ne!(*saved, before, "the cancelled drag's width is saved");
    assert_eq!(*saved, crate::dock::snapshot(h.state()), "the saved layout is the live one after the cancel");
    let settled = dock_width(h.state());

    disabled.store(false, Ordering::Relaxed);
    h.run_steps(3);
    h.event(Event::PointerMoved(pos2(edge - 120.0, 400.0)));
    h.run_steps(2);
    assert!((dock_width(h.state()) - settled).abs() < 0.5, "a stale drag start must not resize the dock after re-enable");
    // Saves resume: a width change made after re-enabling reaches the saved layout.
    let resumed = h.state().session.prefs().panel_layout.clone();
    let ctx = h.ctx.clone();
    assert_eq!(control_set(h.state_mut(), &ctx, json!({"dockWidth": settled + 60.0}))["ok"], true);
    h.run_steps(3);
    assert!((dock_width(h.state()) - settled - 60.0).abs() < 0.5, "the new width is applied, got {}", dock_width(h.state()));
    let saved = &h.state().session.prefs().panel_layout;
    assert_ne!(*saved, resumed, "the width change after re-enable is saved");
    assert_eq!(*saved, crate::dock::snapshot(h.state()), "the saved layout is the new live one");
}

/// `ui.set {dockWidth}` sets the whole end dock, scaling every column rather than only the outer
/// one; the split holding the dock follows the new total.
#[test]
fn ui_set_dock_width_scales_every_column() {
    let mut app = app();
    let mut layout = legacy(&app);
    let used: std::collections::HashSet<String> = layout.panels().into_iter().map(|panel| panel.to_string()).collect();
    let extra = PANELS.iter().map(|(id, _, _)| *id).find(|id| !used.contains(*id)).expect("a free panel id");
    {
        let root = layout.root.as_mut().expect("the legacy layout has a root");
        let dock = match root {
            Node::Split { second, .. } => second.as_mut(),
            _ => root,
        };
        let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
        columns.push(Column::new(vec![craft_ui::docking::Group::new(vec![extra.to_owned()], 0)], 180.0));
    }
    app.ui.docking = Some(Workspace::new(layout));
    // The extra 180-point column first comes up to its 220-point minimum; the rest of the change
    // is shared equally between the two columns.
    let share = (520.0 - end_columns(&app)[0].width - 220.0) / 2.0;
    let split = [end_columns(&app)[0].width + share, 220.0 + share];
    let total = |app: &PhotocraftApp| -> f32 {
        let root = app.ui.docking.as_ref().expect("a customized workspace").layout.root.as_ref().expect("root");
        let dock = match root {
            Node::Split { second, .. } => second.as_ref(),
            _ => root,
        };
        match dock {
            Node::Dock { columns, .. } => columns.iter().map(|column| column.width).sum(),
            _ => 0.0,
        }
    };
    let before = total(&app);
    set_dock_width(&mut app, 520.0);
    let after = total(&app);
    assert!((after - 520.0).abs() < 1.0, "the whole dock should be 520 points, got {after} (was {before})");
    let widths: Vec<f32> = {
        let root = app.ui.docking.as_ref().unwrap().layout.root.as_ref().unwrap();
        let dock = match root {
            Node::Split { second, .. } => second.as_ref(),
            _ => root,
        };
        match dock {
            Node::Dock { columns, .. } => columns.iter().map(|column| column.width).collect(),
            _ => Vec::new(),
        }
    };
    assert_eq!(widths.len(), 2, "both columns are kept");
    assert!(widths.iter().all(|width| *width > 0.0), "every column keeps a positive width: {widths:?}");
    assert!(widths.iter().zip(split).all(|(width, expected)| (width - expected).abs() < 1.0), "both columns share the change: {widths:?}, expected {split:?}");

    // A target below the sum of the columns' 220-point panel minima must stop at that minimum,
    // not starve a column while leaving the other untouched.
    set_dock_width(&mut app, 250.0);
    let after = total(&app);
    assert!((after - 440.0).abs() < 1.0, "two columns keep the 440-point minimum, got {after}");
    let widths: Vec<f32> = {
        let root = app.ui.docking.as_ref().unwrap().layout.root.as_ref().unwrap();
        let dock = match root {
            Node::Split { second, .. } => second.as_ref(),
            _ => root,
        };
        match dock {
            Node::Dock { columns, .. } => columns.iter().map(|column| column.width).collect(),
            _ => Vec::new(),
        }
    };
    assert!(widths.iter().all(|width| *width >= 219.5), "every column keeps the panel minimum: {widths:?}");
    assert!(widths.iter().all(|width| *width < 221.0), "the minimum is shared between the columns: {widths:?}");
}

/// `ui.set {dockWidth}` travels through the queued request and the rendered dock pass, scaling a
/// multi-column dock to the requested total.
#[test]
fn ui_set_dock_width_queues_and_scales_every_column_when_drawn() {
    let mut app = app();
    let mut layout = legacy(&app);
    let used: std::collections::HashSet<String> = layout.panels().into_iter().map(|panel| panel.to_string()).collect();
    let extra = PANELS.iter().map(|(id, _, _)| *id).find(|id| !used.contains(*id)).expect("a free panel id");
    {
        let root = layout.root.as_mut().expect("the legacy layout has a root");
        let dock = match root {
            Node::Split { second, .. } => second.as_mut(),
            _ => root,
        };
        let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
        columns.push(Column::new(vec![craft_ui::docking::Group::new(vec![extra.to_owned()], 0)], 180.0));
    }
    app.ui.docking = Some(Workspace::new(layout));
    let mut h = dock_panel_harness(app, vec2(1280.0, 800.0), crate::theme::ThemeKind::ProMedium);
    // As in `ui_set_dock_width_scales_every_column`: the extra column comes up to its 220-point
    // minimum and the rest of the change is shared equally.
    let share = (520.0 - end_columns(h.state())[0].width - 220.0) / 2.0;
    let split = [end_columns(h.state())[0].width + share, 220.0 + share];
    crate::panels::request_dock_width(&h.ctx, 520.0);
    h.run_steps(2);
    let root = h.state().ui.docking.as_ref().expect("a workspace").layout.root.as_ref().expect("root");
    let dock = match root {
        Node::Split { second, .. } => second.as_ref(),
        _ => root,
    };
    let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
    let widths: Vec<f32> = columns.iter().map(|column| column.width).collect();
    assert_eq!(widths.len(), 2, "both columns are kept");
    assert!(widths.iter().all(|width| *width > 0.0), "every column keeps a positive width: {widths:?}");
    let total: f32 = widths.iter().sum();
    assert!((total - 520.0).abs() < 1.0, "the queued request sets the whole dock to 520 points, got {total} from {widths:?}");
    assert!(widths.iter().zip(split).all(|(width, expected)| (width - expected).abs() < 1.0), "both columns share the change: {widths:?}, expected {split:?}");
}

/// A narrow window must still give the canvas its 200-point minimum; the dock shrinks instead.
#[test]
fn a_narrow_window_keeps_the_canvas_minimum() {
    let mut app = app();
    let mut layout = legacy(&app);
    {
        let root = layout.root.as_mut().expect("the legacy layout has a root");
        let dock = match root {
            Node::Split { second, .. } => second.as_mut(),
            _ => root,
        };
        let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
        columns[0].width = 520.0;
    }
    app.ui.docking = Some(Workspace::new(layout));
    app.ui.panels.rail = true;
    let h = dock_panel_harness(app, vec2(700.0, 600.0), crate::theme::ThemeKind::ProMedium);
    let panel = crate::panels::dock_panel_width(&h.ctx);
    let toolbar = crate::panels::toolbar::toolbar_width(h.state().ui.theme.is_pro(), h.state().ui.panels.toolbar_double);
    let canvas = 700.0 - panel - toolbar;
    assert!(canvas >= 199.0, "the canvas keeps its 200-point minimum, got {canvas} (dock {panel}, toolbar {toolbar})");
}

/// The empty dock still renders its docking pass but must not consume the parent layout, so the
/// toolbar and canvas keep their space.
#[test]
fn an_empty_dock_keeps_the_parent_layout_for_toolbar_and_canvas() {
    let mut app = app();
    let layout = Layout {
        root: Some(Node::Split {
            axis: SplitAxis::Horizontal,
            size: SplitSize::FixedSecond(0.0),
            first: Box::new(Node::Tabs { panels: vec![CANVAS.to_owned()], active: 0 }),
            second: Box::new(Node::Dock { side: Side::End, columns: Vec::new() }),
        }),
        floating: Vec::new(),
    };
    app.ui.docking = Some(Workspace::new(layout));
    app.ui.panels.rail = false;
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(1280.0, 800.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            crate::panels::dock_panel(app, ui);
            let remaining = ui.available_width();
            ui.ctx().data_mut(|data| data.insert_temp(egui::Id::unique("empty-dock-remaining"), remaining));
            crate::panels::toolbar(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    h.run_steps(4);
    let remaining = h.ctx.data(|data| data.get_temp::<f32>(egui::Id::unique("empty-dock-remaining")).unwrap_or(0.0));
    assert!(remaining > 1200.0, "the empty dock must not consume the parent layout, left {remaining}");
    assert!(h.get_by_label("Toolbar").rect().is_positive(), "the toolbar still draws");
}

/// An empty dock must still run the docking pass, because Studio draws its floating Properties
/// group from it, and the zero-width dock must not lose it with or without the rail.
#[test]
fn an_empty_dock_still_draws_studio_floating_properties() {
    for rail in [true, false] {
        let mut app = app();
        let layout = Layout {
            root: Some(Node::Split {
                axis: SplitAxis::Horizontal,
                size: SplitSize::FixedSecond(0.0),
                first: Box::new(Node::Tabs { panels: vec![CANVAS.to_owned()], active: 0 }),
                second: Box::new(Node::Dock { side: Side::End, columns: Vec::new() }),
            }),
            floating: vec![Floating::single(vec!["properties".to_owned()], 0, [120.0, 160.0, 320.0, 440.0])],
        };
        app.ui.docking = Some(Workspace::new(layout));
        app.ui.panels.rail = rail;
        let h = dock_panel_harness(app, vec2(1280.0, 800.0), crate::theme::ThemeKind::Studio);
        assert!(dock_is_empty(h.state()), "rail={rail}: the dock is empty");
        assert!(show_dock_calls(&h.ctx) > 0, "rail={rail}: the docking pass must still run for floating windows");
        let rect = floating_rect(&h, "properties");
        assert!(rect.is_positive(), "rail={rail}: floating Properties must be drawn, got {rect:?}");
    }
}

/// A column collapsed to icons reserves its `iconic_width`, not the expanded width; resizing the
/// edge keeps that iconic width in step.
#[test]
fn an_iconic_dock_column_takes_its_iconic_width() {
    let mut app = app();
    let mut layout = legacy(&app);
    {
        let root = layout.root.as_mut().expect("the legacy layout has a root");
        let dock = match root {
            Node::Split { second, .. } => second.as_mut(),
            _ => root,
        };
        let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
        columns[0].iconic = true;
        columns[0].iconic_width = Some(36.0);
    }
    let expanded = {
        let root = layout.root.as_ref().expect("the legacy layout has a root");
        let dock = match root {
            Node::Split { second, .. } => second.as_ref(),
            _ => root,
        };
        let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
        columns[0].width
    };
    app.ui.docking = Some(Workspace::new(layout));
    assert!((dock_width(&app) - 36.0).abs() < 0.5, "the iconic column displays 36 points, got {}", dock_width(&app));
    assert!(set_end_dock_total(&mut app, 220.0).is_some(), "the dock-edge resize accepts an iconic dock");
    assert!((dock_width(&app) - 220.0).abs() < 1.0, "after the resize the dock displays 220 points, got {}", dock_width(&app));
    let root = app.ui.docking.as_ref().unwrap().layout.root.as_ref().unwrap();
    let dock = match root {
        Node::Split { second, .. } => second.as_ref(),
        _ => root,
    };
    let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
    assert!(columns[0].iconic_width.unwrap_or(0.0) > 200.0, "iconic_width follows the resize: {:?}", columns[0].iconic_width);
    assert!((columns[0].width - expanded).abs() < 0.5, "resizing while iconic keeps the expanded width: {} vs {expanded}", columns[0].width);

    let root = app.ui.docking.as_mut().unwrap().layout.root.as_mut().unwrap();
    let dock = match root {
        Node::Split { second, .. } => second.as_mut(),
        _ => root,
    };
    let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
    columns[0].iconic = false;
    assert!((dock_width(&app) - expanded).abs() < 0.5, "expanding restores the expanded width, got {} (was {expanded})", dock_width(&app));
}

/// Collapsing a column through craft-ui's action leaves `iconic_width` unset; the dock must use
/// the renderer's 36-point default rather than the expanded width.
#[test]
fn an_iconic_only_dock_keeps_its_narrow_panel() {
    let mut app = app();
    let layout = legacy(&app);
    let panel = {
        let root = layout.root.as_ref().expect("the legacy layout has a root");
        let dock = match root {
            Node::Split { second, .. } => second.as_ref(),
            _ => root,
        };
        let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
        columns[0].groups[0].panels[0].clone()
    };
    let mut workspace = Workspace::new(layout);
    workspace.apply(Action::SetIconic { panel, iconic: true }, permissions).expect("the column collapses");
    {
        let root = workspace.layout.root.as_ref().expect("root");
        let dock = match root {
            Node::Split { second, .. } => second.as_ref(),
            _ => root,
        };
        let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
        assert!(columns[0].iconic);
        assert!(columns[0].iconic_width.is_none(), "the collapse action leaves the stored width unset");
    }
    app.ui.docking = Some(workspace);
    assert!((dock_width(&app) - 36.0).abs() < 0.5, "an unset iconic width displays 36 points, got {}", dock_width(&app));
    let h = dock_panel_harness(app, vec2(1280.0, 800.0), crate::theme::ThemeKind::ProMedium);
    let rail = if h.state().ui.panels.rail { 36.0 } else { 0.0 };
    let drawn = crate::panels::dock_panel_width(&h.ctx);
    assert!((drawn - (36.0 + rail)).abs() < 2.0, "the iconic panel is its 36 points plus the rail ({rail}), got {drawn}");
    assert!(drawn < 120.0, "the iconic panel must not take the expanded minimum, got {drawn}");
}

/// Stored iconic widths are clamped to the renderer's 36..240 bounds.
#[test]
fn iconic_widths_follow_the_renderer_clamp() {
    let mut app = app();
    let mut layout = legacy(&app);
    {
        let root = layout.root.as_mut().expect("the legacy layout has a root");
        let dock = match root {
            Node::Split { second, .. } => second.as_mut(),
            _ => root,
        };
        let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
        columns[0].iconic = true;
        columns[0].iconic_width = Some(500.0);
    }
    app.ui.docking = Some(Workspace::new(layout));
    assert!((dock_width(&app) - 240.0).abs() < 0.5, "a stored 500 displays at most 240, got {}", dock_width(&app));
    let root = app.ui.docking.as_mut().unwrap().layout.root.as_mut().unwrap();
    let dock = match root {
        Node::Split { second, .. } => second.as_mut(),
        _ => root,
    };
    let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
    columns[0].iconic_width = Some(10.0);
    assert!((dock_width(&app) - 36.0).abs() < 0.5, "a stored 10 displays at least 36, got {}", dock_width(&app));
    let root = app.ui.docking.as_mut().unwrap().layout.root.as_mut().unwrap();
    let dock = match root {
        Node::Split { second, .. } => second.as_mut(),
        _ => root,
    };
    let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
    columns[0].iconic_width = None;
    assert!((dock_width(&app) - ICONIC_WIDTH).abs() < 0.5, "no saved width displays craft-ui's default 36, got {}", dock_width(&app));
    let style = craft_ui::docking::DockStyle::default();
    assert_eq!((style.iconic_width, style.iconic_max_width), (ICONIC_WIDTH, ICONIC_MAX_WIDTH), "the renderer's defaults");
}
/// Closing every panel leaves the dock with nothing in it, which the dock-edge layout uses to
/// take no width for the column.
#[test]
fn an_empty_dock_has_no_panels() {
    let mut app = app();
    assert!(!dock_is_empty(&app), "the default dock shows its panels");
    app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers"})).unwrap();
    {
        let root = app.ui.docking.as_mut().expect("a customized workspace").layout.root.as_mut().expect("root");
        let Node::Split { second, .. } = root else { panic!("the canvas is beside the dock") };
        **second = Node::Dock { side: Side::End, columns: Vec::new() };
    }
    assert!(dock_is_empty(&app), "a dock with no columns has nothing to show");
}

/// Width refused by the iconic 240-point cap reaches a column that can still grow.
#[test]
fn iconic_cap_surplus_goes_to_expandable_columns() {
    let mut app = app();
    let mut layout = legacy(&app);
    let used: std::collections::HashSet<String> = layout.panels().into_iter().map(|panel| panel.to_string()).collect();
    let extra = PANELS.iter().map(|(id, _, _)| *id).find(|id| !used.contains(*id)).expect("a free panel id");
    {
        let root = layout.root.as_mut().expect("the legacy layout has a root");
        let dock = match root {
            Node::Split { second, .. } => second.as_mut(),
            _ => root,
        };
        let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
        columns[0].iconic = true;
        columns[0].iconic_width = Some(240.0);
        columns.push(Column::new(vec![craft_ui::docking::Group::new(vec![extra.to_owned()], 0)], 220.0));
    }
    app.ui.docking = Some(Workspace::new(layout));
    set_dock_width(&mut app, 600.0);
    let root = app.ui.docking.as_ref().expect("a workspace").layout.root.as_ref().expect("root");
    let dock = match root {
        Node::Split { second, .. } => second.as_ref(),
        _ => root,
    };
    let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
    let displayed: Vec<f32> =
        columns.iter().map(|column| if column.iconic { column.iconic_width.unwrap_or(36.0).clamp(36.0, 240.0) } else { column.width }).collect();
    let total: f32 = displayed.iter().sum();
    assert!((total - 600.0).abs() < 1.0, "the requested total is reached: {displayed:?}");
    assert!((displayed[0] - 240.0).abs() < 0.5, "the iconic column stays at its cap: {displayed:?}");
    assert!((displayed[1] - 360.0).abs() < 1.0, "the expandable column takes the surplus: {displayed:?}");
}

/// The rendered panel uses the width the model actually stored when a drag asks for less than the
/// column minimum, during the drag and after release.
#[test]
fn dock_panel_draws_the_actual_minimum_during_and_after_a_drag() {
    let mut app = app();
    let mut layout = legacy(&app);
    let used: std::collections::HashSet<String> = layout.panels().into_iter().map(|panel| panel.to_string()).collect();
    let extra = PANELS.iter().map(|(id, _, _)| *id).find(|id| !used.contains(*id)).expect("a free panel id");
    {
        let root = layout.root.as_mut().expect("the legacy layout has a root");
        let dock = match root {
            Node::Split { second, .. } => second.as_mut(),
            _ => root,
        };
        let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
        columns.push(Column::new(vec![craft_ui::docking::Group::new(vec![extra.to_owned()], 0)], 180.0));
    }
    app.ui.docking = Some(Workspace::new(layout));
    let mut h = dock_edge_harness(app, vec2(1280.0, 800.0));
    let rail = if h.state().ui.panels.rail { 36.0 } else { 0.0 };
    let start = begin_outer_resize(&mut h);
    let edge = start.x;
    h.event(Event::PointerMoved(pos2(edge + 240.0, start.y)));
    h.run_steps(2);
    let during = dock_width(h.state());
    let drawn = crate::panels::dock_panel_width(&h.ctx);
    assert!((during - 440.0).abs() < 1.0, "the model stops at the 440-point minimum, got {during}");
    assert!((drawn - (during + rail)).abs() < 2.0, "the panel follows the stored width, got {drawn} for {during}+{rail}");
    h.event(Event::PointerButton { pos: pos2(edge + 240.0, start.y), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
    let after = dock_width(h.state());
    assert!((after - during).abs() < 1.0, "the release keeps the model width, got {after} then {during}");
    let drawn = crate::panels::dock_panel_width(&h.ctx);
    assert!((drawn - (after + rail)).abs() < 2.0, "the release frame follows the stored width, got {drawn} for {after}+{rail}");
}

/// A workspace replacement during a drag invalidates the stored gesture before it can write.
#[test]
fn replacing_the_workspace_cancels_the_dock_drag() {
    for release_first in [false, true] {
        let mut app = app();
        app.run("window.panel.move", json!({"panel":"swatches","anchor":"layers"})).unwrap();
        let mut h = dock_edge_harness(app, vec2(1280.0, 800.0));
        let start = begin_outer_resize(&mut h);
        h.event(Event::PointerMoved(start + vec2(20.0, 0.0)));
        h.run_steps(1);
        let mut replacement = legacy(h.state());
        dock_columns_mut(replacement.root.as_mut().expect("root")).expect("end dock")[0].width = 520.0;
        let generation = h.state().ui.docking_generation;
        crate::dock::apply(h.state_mut(), &json!({"docking": Workspace::new(replacement)}));
        assert_eq!(h.state().ui.docking_generation, generation.wrapping_add(1));
        assert!((dock_width(h.state()) - 520.0).abs() < 0.5, "the installed baseline is known before any frame");
        if release_first {
            release_outer_resize(&mut h, start + vec2(100.0, 0.0));
        } else {
            h.run_steps(1);
        }
        assert!(!crate::panels::dock_edge_drag_active(&h.ctx), "the first frame clears the stale gesture");
        assert!((dock_width(h.state()) - 520.0).abs() < 0.5, "the first frame must not overwrite the replacement");
        if !release_first {
            release_outer_resize(&mut h, start + vec2(100.0, 0.0));
        }
        h.run_steps(2);
        assert!((dock_width(h.state()) - 520.0).abs() < 0.5, "release and idle retain the installed width");
    }
}

/// A window too narrow for the column minima does not panic while resizing and keeps the panel
/// inside the available space, also when the window shrinks to it in the middle of a drag.
#[test]
fn constrained_width_does_not_panic_while_resizing() {
    let two_columns = || {
        let mut app = app();
        let mut layout = legacy(&app);
        let used: std::collections::HashSet<String> = layout.panels().into_iter().map(|panel| panel.to_string()).collect();
        let extra = PANELS.iter().map(|(id, _, _)| *id).find(|id| !used.contains(*id)).expect("a free panel id");
        {
            let root = layout.root.as_mut().expect("the legacy layout has a root");
            let dock = match root {
                Node::Split { second, .. } => second.as_mut(),
                _ => root,
            };
            let Node::Dock { columns, .. } = dock else { panic!("the end dock is missing") };
            columns.push(Column::new(vec![craft_ui::docking::Group::new(vec![extra.to_owned()], 0)], 180.0));
        }
        app.ui.docking = Some(Workspace::new(layout));
        app
    };
    // A drag started in a wide window is cancelled once the shrunk window cannot fit the columns.
    let mut h = dock_edge_harness(two_columns(), vec2(1280.0, 600.0));
    let start = begin_outer_resize(&mut h);
    h.event(Event::PointerMoved(start + vec2(-30.0, 0.0)));
    h.run_steps(1);
    assert!(crate::panels::dock_edge_drag_active(&h.ctx), "the drag is held in the wide window");
    let stored: Vec<f32> = end_columns(h.state()).iter().map(|column| column.width).collect();
    h.set_size(vec2(300.0, 600.0));
    h.event(Event::PointerMoved(pos2(200.0, 300.0)));
    h.run_steps(3);
    assert!(!crate::panels::dock_edge_drag_active(&h.ctx), "shrinking the window below the column minima cancels the held drag");
    let drawn = crate::panels::dock_panel_width(&h.ctx);
    assert!(drawn <= 80.0, "the dock stays within the shrunk window mid-drag, got {drawn}");
    let widths: Vec<f32> = end_columns(h.state()).iter().map(|column| column.width).collect();
    assert_eq!(widths, stored, "the cancel keeps the stored column widths");
    h.event(Event::PointerButton { pos: pos2(200.0, 300.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(crate::panels::dock_panel_width(&h.ctx) <= 80.0, "the release after the cancel keeps the dock in the window");
    let widths: Vec<f32> = end_columns(h.state()).iter().map(|column| column.width).collect();
    assert_eq!(widths, stored, "the release after the cancel keeps the stored column widths");

    let mut h = dock_edge_harness(two_columns(), vec2(300.0, 600.0));
    let drawn = crate::panels::dock_panel_width(&h.ctx);
    assert!(drawn <= 80.0, "the dock stays within the narrow window, got {drawn}");
    let start = outer_handle(&h).center();
    let edge = start.x;
    h.event(Event::PointerMoved(start));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    h.event(Event::PointerMoved(pos2(edge + 40.0, 300.0)));
    h.run_steps(2);
    h.event(Event::PointerButton { pos: pos2(edge + 40.0, 300.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
    assert!(crate::panels::dock_panel_width(&h.ctx) <= 80.0, "the panel stays within the narrow window after release");
}

/// The rendered dock-edge resize handle's interaction rectangle.
fn outer_handle(h: &Harness<'static, PhotocraftApp>) -> egui::Rect {
    h.ctx.read_response(egui::Id::unique("dock-edge-handle")).expect("the rendered resize handle").rect
}

/// The rectangle the dock-edge panel actually allocated on the last frame.
fn allocated_dock(h: &Harness<'static, PhotocraftApp>) -> egui::Rect {
    h.ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("photo-dock-panel-rect")).expect("the allocated dock rectangle"))
}

/// Presses on the dock-edge handle, asserts the drag started that frame and returns the press
/// position.
fn begin_outer_resize(h: &mut Harness<'static, PhotocraftApp>) -> Pos2 {
    let start = outer_handle(h).center();
    h.event(Event::PointerMoved(start));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert!(crate::panels::dock_edge_drag_active(&h.ctx), "Sense::drag starts on the press frame");
    start
}

/// Releases the pointer at `pos` with no preceding move, so the release alone carries the final
/// position.
fn release_outer_resize(h: &mut Harness<'static, PhotocraftApp>, pos: Pos2) {
    // No preceding PointerMoved: the release event alone supplies the final position.
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(1);
}

/// An app whose saved workspace has one end-dock column per width, each holding one panel; iconic
/// columns get distinct expanded widths so tests can check they are preserved.
fn app_with_columns(widths: &[f32], iconic: bool) -> PhotocraftApp {
    assert!(widths.len() <= PANELS.len());
    let columns = widths
        .iter()
        .zip(PANELS.iter())
        .enumerate()
        .map(|(index, (width, (id, _, _)))| {
            let mut column = Column::new(vec![craft_ui::docking::Group::new(vec![(*id).to_owned()], 0)], *width);
            if iconic {
                column.iconic = true;
                column.iconic_width = Some(*width);
                column.width = 290.0 + index as f32;
            }
            column
        })
        .collect();
    let root = with_canvas(Node::Dock { side: Side::End, columns }, widths.iter().sum::<f32>().min(520.0));
    let mut app = app();
    app.ui.docking = Some(Workspace::new(Layout { root: Some(root), floating: Vec::new() }));
    assert!(valid_workspace(app.ui.docking.as_ref().unwrap()), "fixtures must be accepted saved workspaces");
    app
}

/// The end dock's columns in the customized workspace.
fn end_columns(app: &PhotocraftApp) -> &[Column<String>] {
    let Some(Node::Dock { columns, .. }) = lay(app).root.as_ref().and_then(end_dock_node) else { panic!("the end dock is missing") };
    columns
}

/// A request equal to the current total still lifts an undersized column to its minimum.
#[test]
fn same_total_requests_normalize_each_populated_column() {
    let mut app = app_with_columns(&[290.0, 180.0], false);
    set_dock_width(&mut app, 470.0);
    assert!((dock_width(&app) - 470.0).abs() < 0.5);
    assert!(end_columns(&app).iter().all(|column| column.width >= MIN_COLUMN_WIDTH));
}

/// Redistribution reaches an attainable total even when more than eight columns saturate in turn.
#[test]
fn redistribution_converges_with_more_than_eight_saturating_columns() {
    let mut app = app_with_columns(&[221.0, 800221.0, 889109.75, 900220.75, 901807.9, 902072.25, 902124.94, 902137.9, 902141.9, 902242.4], false);
    set_dock_width(&mut app, 2300.5);
    assert!((dock_width(&app) - 2300.5).abs() < 0.5, "attainable target, got {}", dock_width(&app));
    assert!(end_columns(&app).iter().all(|column| column.width >= MIN_COLUMN_WIDTH));
}

/// Width an iconic column cannot take past its cap goes to the expanded column, whether the cap
/// was already reached or is reached by this request.
#[test]
fn mixed_iconic_caps_redistribute_both_existing_and_new_saturation() {
    for iconic_width in [100.0, ICONIC_MAX_WIDTH] {
        let mut app = app_with_columns(&[290.0, 220.0], false);
        let first = &mut dock_columns_mut(app.ui.docking.as_mut().unwrap().layout.root.as_mut().unwrap()).unwrap()[0];
        first.iconic = true;
        first.iconic_width = Some(iconic_width);
        assert!((set_end_dock_total(&mut app, 800.0).unwrap() - 800.0).abs() < 0.5);
        assert_eq!(end_columns(&app)[0].iconic_width, Some(ICONIC_MAX_WIDTH));
        assert_eq!(end_columns(&app)[0].width, 290.0, "iconic resizing preserves expanded width");
        assert!((end_columns(&app)[1].width - 560.0).abs() < 0.5);
    }
}

/// An all-iconic dock stops at the summed iconic minimum and cap, and keeps every expanded width.
#[test]
fn all_iconic_totals_stop_at_both_bounds_without_losing_expanded_widths() {
    let mut app = app_with_columns(&[100.0, 200.0], true);
    let expanded: Vec<_> = end_columns(&app).iter().map(|column| column.width).collect();
    for (requested, expected) in [(1000.0, 480.0), (1.0, 72.0), (300.0, 300.0)] {
        assert!((set_end_dock_total(&mut app, requested).unwrap() - expected).abs() < 0.5);
        let stored: f32 = end_columns(&app).iter().map(display_width).sum();
        assert!((stored - expected).abs() < 1.0, "the columns store the {expected}-point total, got {stored}");
        for (column, expanded) in end_columns(&app).iter().zip(&expanded) {
            assert!((ICONIC_WIDTH..=ICONIC_MAX_WIDTH).contains(&display_width(column)));
            assert_eq!(column.width, *expanded);
        }
    }
}

/// The end-dock setter reports a same-width legacy request as a no-op and rejects invalid,
/// locked and empty cases without changing the workspace.
#[test]
fn end_dock_setter_reports_legacy_noop_and_rejects_without_mutation() {
    let mut app = app();
    let current = dock_width(&app);
    assert_eq!(set_end_dock_total(&mut app, current), Some(current));
    assert!(app.ui.docking.is_none(), "same-width legacy requests must not customize");
    for rejected in [0.0, -1.0, f32::NAN, f32::INFINITY] {
        assert_eq!(set_end_dock_total(&mut app, rejected), None);
        assert!(app.ui.docking.is_none());
    }
    app.session.prefs.edit(|prefs| prefs.workspace_locked = true);
    assert_eq!(set_end_dock_total(&mut app, current + 100.0), None);
    assert!(app.ui.docking.is_none());
    app.session.prefs.edit(|prefs| prefs.workspace_locked = false);
    assert_eq!(set_end_dock_total(&mut app, 400.0), Some(400.0));
    assert!((dock_width(&app) - 400.0).abs() < 0.5);
    let mut empty = app_with_columns(&[], false);
    let before = empty.ui.docking.clone();
    assert_eq!(set_end_dock_total(&mut empty, 400.0), None);
    assert_eq!(empty.ui.docking, before);
}

/// Idle frames draw the stored 220-point width; a gesture, even without motion, applies the
/// 250-point resize minimum.
#[test]
fn an_idle_expanded_dock_uses_its_stored_width_not_the_gesture_minimum() {
    let mut h = dock_edge_harness(app_with_columns(&[220.0], false), vec2(1280.0, 800.0));
    let rail = if h.state().ui.panels.rail { 36.0 } else { 0.0 };
    assert!((allocated_dock(&h).width() - 220.0 - rail).abs() < 1.0);
    assert_eq!(dock_width(h.state()), 220.0);
    let start = begin_outer_resize(&mut h);
    assert_eq!(dock_width(h.state()), 220.0, "the press records the existing idle width");
    release_outer_resize(&mut h, start);
    assert_eq!(dock_width(h.state()), 250.0, "even an unmoved gesture applies the resize minimum");
    assert!((allocated_dock(&h).width() - 250.0 - rail).abs() < 1.0);
    h.run_steps(2);
    assert_eq!(dock_width(h.state()), 250.0, "idle keeps the normalized gesture result");
}

/// A release with no preceding move updates the model and the allocated panel on that frame,
/// counting the rail once, in both directions, both rail widths and with the rail hidden.
#[test]
fn release_only_position_updates_model_and_actual_geometry_on_that_frame() {
    for dir in [crate::direction::Dir::Ltr, crate::direction::Dir::Rtl] {
        let _direction = crate::direction::direction_scope(dir);
        for theme in [crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::Classic] {
            for rail in [false, true] {
                let mut app = app_with_columns(&[400.0], false);
                app.ui.panels.rail = rail;
                app.ui.theme = theme;
                let rw = if rail { if theme.is_pro() { 36.0 } else { 44.0 } } else { 0.0 };
                let mut h = dock_panel_harness(app, vec2(1280.0, 800.0), theme);
                let start = begin_outer_resize(&mut h);
                let sign = if crate::direction::is_rtl() { 1.0 } else { -1.0 };
                h.event(Event::PointerMoved(start + vec2(sign * 30.0, 0.0)));
                h.run_steps(1);
                assert!((dock_width(h.state()) - 430.0).abs() < 0.5);
                assert!((allocated_dock(&h).width() - 430.0 - rw).abs() < 1.0);
                release_outer_resize(&mut h, start + vec2(sign * 80.0, 0.0));
                assert!(!crate::panels::dock_edge_drag_active(&h.ctx));
                assert!((dock_width(h.state()) - 480.0).abs() < 0.5, "the release-only position is applied immediately");
                let released = allocated_dock(&h);
                assert!((released.width() - 480.0 - rw).abs() < 1.0, "the rail is counted once");
                h.run_steps(2);
                assert!((allocated_dock(&h).width() - released.width()).abs() < 0.5, "idle does not jump");
            }
        }
    }
}

/// A dock-edge harness `width` points wide with the Pro single-column toolbox on the right, the
/// rail and a stored 520-point dock column, after the frames that draw it.
fn narrow_dock_harness(width: f32) -> Harness<'static, PhotocraftApp> {
    let mut app = app_with_columns(&[400.0], false);
    app.ui.panels.rail = true;
    app.ui.panels.toolbar_double = false;
    app.ui.panels.toolbar_side = crate::state::ToolbarSide::Right;
    let mut ready = false;
    let mut h = Harness::builder().with_size(vec2(width, 600.0)).with_step_dt(1.0 / 60.0).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ready {
                PhotocraftApp::setup_context(ui.ctx(), crate::theme::ThemeKind::ProMedium);
                ready = true;
                return;
            }
            crate::panels::dock_panel(app, ui);
            crate::panels::toolbar(app, ui);
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                ui.ctx().data_mut(|data| data.insert_temp(egui::Id::unique("narrow-canvas-rect"), ui.max_rect()));
            });
        },
        app,
    );
    h.run_steps(4);
    assert_eq!(crate::panels::toolbar::toolbar_width(h.state().ui.theme.is_pro(), false), 40.0, "the Pro single-column toolbox");
    let ctx = h.ctx.clone();
    assert_eq!(control_set(h.state_mut(), &ctx, json!({"dockWidth":520.0}))["ok"], true);
    h.run_steps(2);
    h
}

/// The canvas rectangle [`narrow_dock_harness`] drew on its last frame.
fn narrow_canvas(h: &Harness<'static, PhotocraftApp>) -> egui::Rect {
    h.ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("narrow-canvas-rect")).expect("the canvas rect"))
}

/// The panel a dock column's control acts on; `None` for floating-window and split controls.
fn dock_hit_panel(hit: &craft_ui::docking::Hit<String>) -> Option<&String> {
    use craft_ui::docking::{Divider, Hit};
    match hit {
        Hit::Tab(p)
        | Hit::DragBar(p)
        | Hit::Border(p)
        | Hit::Overflow(p)
        | Hit::ColumnBar(p)
        | Hit::Collapse(p)
        | Hit::StackToggle(p)
        | Hit::Icon(p)
        | Hit::IconGrip(p) => Some(p),
        Hit::Divider(Divider::Column(p) | Divider::Group(p) | Divider::Stack(p)) => Some(p),
        _ => None,
    }
}

/// Asserts that every control of the end dock's panels keeps its clipped `interact_rect`, what is
/// drawn and clickable, inside `dock`, and returns how many have a full `rect` past it.
fn dock_controls_clipped(h: &Harness<'static, PhotocraftApp>, dock: egui::Rect) -> usize {
    use craft_ui::docking::DockArea;
    let panels: Vec<&String> = end_columns(h.state()).iter().flat_map(|c| c.groups.iter().flat_map(|g| g.panels.iter())).collect();
    let area = area_id();
    let hits = DockArea::<String>::hit_rects(&h.ctx, area);
    let (mut drawn, mut clipped) = (0, 0);
    for (hit, rect) in hits.iter().filter(|(hit, _)| dock_hit_panel(hit).is_some_and(|p| panels.contains(&p))) {
        let Some(response) = h.ctx.read_response(DockArea::<String>::hit_id(area, hit)) else {
            assert!(!rect.intersect(dock).is_positive(), "{hit:?} at {rect:?} has no response");
            continue;
        };
        let shown = response.interact_rect;
        assert!(!shown.is_positive() || dock.expand(0.5).contains_rect(shown), "{hit:?}: interactive {shown:?} (full {rect:?}) outside {dock:?}");
        drawn += usize::from(shown.is_positive());
        clipped += usize::from(!dock.expand(2.0).contains_rect(*rect));
    }
    assert!(drawn > 0, "the dock drew controls: {hits:?}");
    clipped
}

/// `dockWidth` stores the requested model width even when a narrow window draws the dock
/// narrower: in one frame the model keeps 520, the drawn dock column fits in 364 points and the
/// canvas keeps its 200, and a wider window draws the stored width again.
#[test]
fn a_narrow_window_draws_a_stored_dock_width_narrower_without_losing_it() {
    let mut h = narrow_dock_harness(640.0);
    let canvas = narrow_canvas(&h);
    let dock = allocated_dock(&h);
    let model = dock_width(h.state());
    assert!((model - 520.0).abs() < 0.5, "the model keeps the request, got {model}");
    assert!(dock.width() - 36.0 <= 364.5, "the drawn dock column fits beside the toolbox and canvas, got {dock:?}");
    assert!(canvas.width() >= 199.5, "the canvas keeps its 200-point minimum, got {canvas:?} beside {dock:?}");
    assert!(!canvas.intersect(dock).is_positive(), "the dock never overlaps the canvas: {canvas:?}, {dock:?}");
    // The fitted column lays its chrome out inside the dock, so here nothing needs the clip.
    dock_controls_clipped(&h, dock);
    h.set_size(vec2(1280.0, 600.0));
    h.run_steps(3);
    assert!((dock_width(h.state()) - 520.0).abs() < 0.5, "growing the window keeps the stored width");
    assert!((allocated_dock(&h).width() - 520.0 - 36.0).abs() < 1.0, "a wide window draws the stored width again, got {:?}", allocated_dock(&h));
}

/// With no room left for the dock column, its controls' full rects run past the panel edge and
/// the panel clip keeps what is drawn and clickable inside the dock.
#[test]
fn a_squeezed_dock_column_clips_its_controls_at_the_panel_edge() {
    // The toolbox, the Pro rail (36 points) and the canvas minimum fill the window, which leaves
    // the dock column only its one-point floor.
    let h = narrow_dock_harness(crate::panels::toolbar::toolbar_width(true, false) + 36.0 + CANVAS_MIN_WIDTH);
    let canvas = narrow_canvas(&h);
    let dock = allocated_dock(&h);
    assert!(!canvas.intersect(dock).is_positive(), "the dock never overlaps the canvas: {canvas:?}, {dock:?}");
    // Asserts that some control's full rect overflows the dock while every clipped one stays inside.
    let clipped = dock_controls_clipped(&h, dock);
    assert!(clipped > 0, "a squeezed column has chrome wider than the dock that the clip cuts off, in {dock:?}");
}

/// A queued external width cancels a held or just-released dock-edge drag and is drawn at its
/// actual width on the first frame that processes it.
#[test]
fn external_width_requests_supersede_held_and_release_first_gestures() {
    for release_first in [false, true] {
        let mut h = dock_edge_harness(app_with_columns(&[400.0], false), vec2(1280.0, 800.0));
        let start = begin_outer_resize(&mut h);
        h.event(Event::PointerMoved(start + vec2(30.0, 0.0)));
        h.run_steps(1);
        let ctx = h.ctx.clone();
        assert_eq!(control_set(h.state_mut(), &ctx, json!({"dockWidth":520.0}))["ok"], true);
        assert!(!crate::panels::dock_edge_drag_active(&h.ctx), "the external request invalidates the gesture before another frame");
        if release_first {
            release_outer_resize(&mut h, start + vec2(100.0, 0.0));
        } else {
            h.run_steps(1);
        }
        assert!((dock_width(h.state()) - 520.0).abs() < 0.5, "the request is installed on its first frame");
        let rail = if h.state().ui.panels.rail { 36.0 } else { 0.0 };
        assert!(
            (allocated_dock(&h).width() - 520.0 - rail).abs() < 1.0,
            "release_first={release_first}: the FIRST processing frame allocates the requested model width plus one rail, got {:?}",
            allocated_dock(&h),
        );
        if !release_first {
            h.event(Event::PointerMoved(start + vec2(100.0, 0.0)));
            h.run_steps(1);
            assert!((dock_width(h.state()) - 520.0).abs() < 0.5, "later held movement cannot restore the old width");
            release_outer_resize(&mut h, start + vec2(140.0, 0.0));
        }
        h.run_steps(2);
        assert!((dock_width(h.state()) - 520.0).abs() < 0.5);
    }
}

/// Narrowing the window below the iconic minimum cancels an active resize without committing the
/// stale pointer width, held or release-first, and widening restores the expanded width.
#[test]
fn active_iconic_resize_is_cancelled_when_the_window_narrows() {
    for release_first in [false, true] {
        let mut h = dock_edge_harness(app_with_columns(&[100.0], true), vec2(1280.0, 800.0));
        let start = begin_outer_resize(&mut h);
        h.event(Event::PointerMoved(start - vec2(40.0, 0.0)));
        h.run_steps(1);
        let before = dock_width(h.state());
        let expanded = end_columns(h.state())[0].width;
        assert!((before - 140.0).abs() < 0.5);
        h.set_size(vec2(300.0, 600.0));
        if release_first {
            release_outer_resize(&mut h, start + vec2(100.0, 0.0));
        } else {
            h.event(Event::PointerMoved(start + vec2(100.0, 0.0)));
            h.run_steps(1);
        }
        assert!(!crate::panels::dock_edge_drag_active(&h.ctx));
        assert_eq!(dock_width(h.state()), before, "narrowing cannot commit the stale pointer width");
        assert!(allocated_dock(&h).width() <= 44.5, "the allocated panel remains bounded");
        if !release_first {
            release_outer_resize(&mut h, start + vec2(140.0, 0.0));
        }
        h.set_size(vec2(1280.0, 800.0));
        h.run_steps(2);
        assert_eq!(dock_width(h.state()), before);
        assert_eq!(end_columns(h.state())[0].width, expanded);
    }
}

/// Several iconic columns disable resizing when the window cannot fit their combined minimum.
#[test]
fn multiple_iconic_columns_disable_resizing_below_their_combined_minimum() {
    let mut h = dock_edge_harness(app_with_columns(&[100.0, 100.0], true), vec2(342.0, 600.0));
    let original = dock_width(h.state());
    let start = outer_handle(&h).center();
    h.event(Event::PointerMoved(start));
    h.run_steps(1);
    h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert!(!crate::panels::dock_edge_drag_active(&h.ctx), "50 available points cannot fit two 36-point columns");
    h.event(Event::PointerMoved(start + vec2(100.0, 0.0)));
    h.run_steps(1);
    release_outer_resize(&mut h, start + vec2(120.0, 0.0));
    assert_eq!(dock_width(h.state()), original);
    h.set_size(vec2(1280.0, 800.0));
    h.run_steps(2);
    assert_eq!(dock_width(h.state()), original, "widening retains the pre-constraint iconic widths");
}

/// With the dock, toolbar and canvas composed in a narrow window, the canvas keeps its minimum
/// width and the model keeps its column minimums, in both directions and themes, with and without
/// the rail.
#[test]
fn constrained_composed_layout_reserves_the_canvas_with_or_without_a_rail() {
    for dir in [crate::direction::Dir::Ltr, crate::direction::Dir::Rtl] {
        let _direction = crate::direction::direction_scope(dir);
        for theme in [crate::theme::ThemeKind::ProMedium, crate::theme::ThemeKind::Classic] {
            for rail in [false, true] {
                let mut app = app_with_columns(&[290.0, 220.0], false);
                app.ui.panels.rail = rail;
                app.ui.theme = theme;
                app.ui.panels.toolbar_side = if crate::direction::is_rtl() { crate::state::ToolbarSide::Left } else { crate::state::ToolbarSide::Right };
                let mut ready = false;
                let mut h = Harness::builder().with_size(vec2(400.0, 600.0)).with_step_dt(1.0 / 60.0).build_ui_state(
                    move |ui, app: &mut PhotocraftApp| {
                        if !ready {
                            PhotocraftApp::setup_context(ui.ctx(), theme);
                            ready = true;
                            return;
                        }
                        crate::panels::dock_panel(app, ui);
                        crate::panels::toolbar(app, ui);
                        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ui, |ui| {
                            ui.ctx().data_mut(|data| data.insert_temp(egui::Id::unique("constrained-canvas-rect"), ui.max_rect()));
                        });
                    },
                    app,
                );
                h.run_steps(4);
                let assert_geometry = |h: &Harness<'static, PhotocraftApp>| {
                    let canvas = h.ctx.data(|data| data.get_temp::<egui::Rect>(egui::Id::unique("constrained-canvas-rect")).unwrap());
                    assert!(canvas.width() >= 199.5, "rail={rail}, dir={dir:?}: canvas {canvas:?}, dock {:?}", allocated_dock(h));
                    assert!((dock_width(h.state()) - 510.0).abs() < 0.5, "the constrained display does not corrupt model minima");
                    assert!(allocated_dock(h).width() <= 144.5, "actual allocation, not requested width");
                };
                assert_geometry(&h);
                let start = outer_handle(&h).center();
                h.event(Event::PointerMoved(start));
                h.run_steps(1);
                h.event(Event::PointerButton { pos: start, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
                h.run_steps(1);
                assert!(!crate::panels::dock_edge_drag_active(&h.ctx));
                release_outer_resize(&mut h, start + vec2(60.0, 0.0));
                assert_geometry(&h);
            }
        }
    }
}

/// A queued width is drawn on its first frame while native migration still waits for the
/// canvas; the native state is consumed once.
#[test]
fn queued_external_width_precedes_but_does_not_eagerly_consume_native_migration() {
    let mut app = app();
    app.last_canvas_rect = egui::Rect::NOTHING;
    let mut h = dock_edge_harness(app, vec2(1280.0, 800.0));
    assert!(!h.state().last_canvas_rect.is_positive(), "the dock-only harness has not laid out a canvas");
    assert!(h.state().ui.docking.is_none());
    let key = crate::panels::panel_state_id(&h.ctx, "dock");
    let state = egui::PanelState { outer_rect: egui::Rect::from_min_size(egui::pos2(900.0, 0.0), vec2(380.0, 700.0)) };
    h.ctx.data_mut(|data| data.insert_persisted(key, state));
    h.run_steps(1);
    assert!(h.state().ui.docking.is_none(), "native migration still waits for canvas readiness");
    assert!(h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).is_some());
    crate::panels::request_dock_width(&h.ctx, 520.0);
    h.run_steps(1);
    assert!((dock_width(h.state()) - 520.0).abs() < 0.5, "the explicit request takes precedence over the old native width");
    let rail = if h.state().ui.panels.rail { 36.0 } else { 0.0 };
    assert!((allocated_dock(&h).width() - 520.0 - rail).abs() < 1.0, "first-frame allocation uses that same width");
    assert!(h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).is_none(), "the migrated state is consumed once");
    h.run_steps(2);
    assert!((dock_width(h.state()) - 520.0).abs() < 0.5);
}

/// Stores a native dock `PanelState` `width` points wide, as earlier versions saved it, and
/// returns its key.
fn remember_native_width_for_request_test(ctx: &egui::Context, width: f32) -> egui::Id {
    let key = crate::panels::panel_state_id(ctx, "dock");
    let state = egui::PanelState { outer_rect: egui::Rect::from_min_size(pos2(900.0, 0.0), vec2(width, 700.0)) };
    ctx.data_mut(|data| data.insert_persisted(key, state));
    key
}

/// With the canvas already laid out, an explicit same-width request retires native migration
/// without customizing the legacy workspace, in both layouts and with the workspace locked.
#[test]
fn explicit_legacy_noop_width_supersedes_native_migration_with_ready_canvas() {
    for compact in [false, true] {
        for locked in [false, true] {
            let mut app = app();
            app.session.prefs.edit(|prefs| prefs.workspace_locked = locked);
            let mut h = if compact { dock_edge_harness(app, vec2(1280.0, 800.0)) } else { harness(app, vec2(1280.0, 800.0)) };
            assert!(h.state().last_canvas_rect.is_positive());
            assert!(h.state().ui.docking.is_none());
            let requested = dock_width(h.state());
            let key = remember_native_width_for_request_test(&h.ctx, 380.0);
            let ctx = h.ctx.clone();
            assert_eq!(control_set(h.state_mut(), &ctx, json!({"dockWidth":requested}))["ok"], true);
            h.run_steps(1);
            assert_eq!(dock_width(h.state()), requested, "compact={compact}, locked={locked}: migration cannot override an explicit legacy no-op");
            assert!(h.state().ui.docking.is_none(), "a same-width request must not customize the legacy workspace");
            assert!(h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).is_none());
            assert!(h.ctx.data(|data| data.get_temp::<f32>(crate::panels::dock_width_id())).is_none());
            h.run_steps(2);
            assert_eq!(dock_width(h.state()), requested);
            assert!(h.state().ui.docking.is_none());
        }
    }
}

/// An explicit same-width request made before the canvas is ready still prevents a later
/// migration from applying the saved native width.
#[test]
fn explicit_legacy_noop_width_supersedes_native_migration_after_canvas_becomes_ready() {
    for locked in [false, true] {
        let mut app = app();
        app.last_canvas_rect = egui::Rect::NOTHING;
        app.session.prefs.edit(|prefs| prefs.workspace_locked = locked);
        let mut h = dock_edge_harness(app, vec2(1280.0, 800.0));
        let requested = dock_width(h.state());
        let key = remember_native_width_for_request_test(&h.ctx, 380.0);
        let ctx = h.ctx.clone();
        assert_eq!(control_set(h.state_mut(), &ctx, json!({"dockWidth":requested}))["ok"], true);
        h.run_steps(1);
        assert!(!h.state().last_canvas_rect.is_positive());
        assert_eq!(dock_width(h.state()), requested);
        assert!(h.state().ui.docking.is_none());
        let native_retained_after_request = h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).is_some();
        assert!(h.ctx.data(|data| data.get_temp::<f32>(crate::panels::dock_width_id())).is_none());
        h.state_mut().last_canvas_rect = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 600.0));
        h.run_steps(1);
        assert_eq!(dock_width(h.state()), requested, "locked={locked}: later canvas readiness cannot revive the superseded native width");
        assert!(!native_retained_after_request, "consuming the explicit request retires native migration even before canvas readiness");
        assert!(h.state().ui.docking.is_none(), "the legacy no-op still must not customize the workspace");
        assert!(h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).is_none());
    }
}

/// An invalid workspace defers both the queued request and the native state until recovery,
/// after which the explicit request wins.
#[test]
fn explicit_width_request_preserves_queue_and_native_state_until_invalid_workspace_recovery() {
    // The current width (a legacy no-op) and a changed one.
    for changed in [false, true] {
        let mut h = dock_edge_harness(app(), vec2(1280.0, 800.0));
        let requested = if changed { 520.0 } else { dock_width(h.state()) };
        let key = remember_native_width_for_request_test(&h.ctx, 380.0);
        let invalid = Workspace::new(Layout { root: None, floating: Vec::new() });
        assert!(!valid_workspace(&invalid));
        h.state_mut().ui.docking = Some(invalid.clone());
        let ctx = h.ctx.clone();
        assert_eq!(control_set(h.state_mut(), &ctx, json!({"dockWidth":requested}))["ok"], true);
        apply_pending_dock_width(h.state_mut(), &ctx);
        assert_eq!(h.state().ui.docking, Some(invalid));
        assert_eq!(h.ctx.data(|data| data.get_temp::<f32>(crate::panels::dock_width_id())), Some(requested));
        // The rectangle `remember_native_width_for_request_test` saved, in both cases.
        assert_eq!(
            h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).map(|state| state.outer_rect),
            Some(egui::Rect::from_min_size(pos2(900.0, 0.0), vec2(380.0, 700.0))),
            "changed={changed}: neither queued request nor native state is consumed or changed before recovery"
        );
        h.run_steps(1);
        assert!(
            (dock_width(h.state()) - requested).abs() < 0.5,
            "changed={changed}: the restored workspace honors the explicit request instead of the saved native width, got {}",
            dock_width(h.state())
        );
        if !changed {
            assert!(h.state().ui.docking.is_none(), "the legacy no-op does not customize the workspace");
        }
        assert!(h.ctx.data(|data| data.get_temp::<f32>(crate::panels::dock_width_id())).is_none(), "changed={changed}: the queued request is consumed");
        assert!(h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).is_none(), "changed={changed}: the native state is retired");
    }
}

/// A consumed request retires the native state even when it is clamped or the dock is empty.
#[test]
fn explicit_width_requests_retire_native_state_even_when_clamped_or_empty() {
    for empty in [false, true] {
        let mut h = dock_edge_harness(if empty { app_with_columns(&[], false) } else { app() }, vec2(1280.0, 800.0));
        let before = h.state().ui.docking.clone();
        let key = remember_native_width_for_request_test(&h.ctx, 380.0);
        let ctx = h.ctx.clone();
        assert_eq!(control_set(h.state_mut(), &ctx, json!({"dockWidth":20}))["ok"], true);
        apply_pending_dock_width(h.state_mut(), &ctx);
        assert!(h.ctx.data(|data| data.get_temp::<f32>(crate::panels::dock_width_id())).is_none());
        assert!(
            h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).is_none(),
            "an accepted explicit request supersedes migration even when empty={empty}"
        );
        if empty {
            assert_eq!(h.state().ui.docking, before, "an empty dock still has no width to change");
        } else {
            assert_eq!(dock_width(h.state()), 250.0, "the existing request clamp remains unchanged");
        }
    }
}

/// Without an accepted request, native migration still waits for the canvas and then applies
/// the saved width once.
#[test]
fn native_migration_still_waits_and_runs_without_an_accepted_explicit_request() {
    for rejected in [false, true] {
        let mut app = app();
        app.last_canvas_rect = egui::Rect::NOTHING;
        let mut h = dock_edge_harness(app, vec2(1280.0, 800.0));
        let key = remember_native_width_for_request_test(&h.ctx, 380.0);
        if rejected {
            let ctx = h.ctx.clone();
            assert_eq!(control_set(h.state_mut(), &ctx, json!({"dockWidth":"not a width"}))["ok"], false);
        }
        assert!(h.ctx.data(|data| data.get_temp::<f32>(crate::panels::dock_width_id())).is_none());
        h.run_steps(1);
        assert!(h.state().ui.docking.is_none());
        assert!(h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).is_some(), "absent/rejected requests do not discard a pending migration");
        h.state_mut().last_canvas_rect = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 600.0));
        h.run_steps(1);
        assert_eq!(dock_width(h.state()), 380.0);
        assert!(h.state().ui.docking.is_some());
        assert!(h.ctx.data_mut(|data| data.get_persisted::<egui::PanelState>(key)).is_none(), "the unchanged native migration consumes its state once");
    }
}
