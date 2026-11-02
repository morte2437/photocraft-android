//! Dock layout tests (#88): fixed group heights, splitters, collapse, reorder, persistence.

use egui::{Modifiers, PointerButton, Pos2, Rect, vec2};
use egui_kittest::Harness;
use serde_json::json;

use super::*;
use crate::panel_docking::probe;
use crate::theme::ThemeKind;

#[test]
fn bad_stored_values_are_sanitised() {
    let mut l = DockLayout { order: vec![Group::Layers, Group::Layers, Group::Color], ..Default::default() };
    assert_eq!(l.order(), vec![Group::Layers, Group::Brushes, Group::Color, Group::Properties, Group::Character, Group::Navigator, Group::History]);
    for bad in [f32::NAN, -10.0, f32::INFINITY, 1e12] {
        l.heights.insert(Group::Color, bad);
        let h = l.height(Group::Color);
        assert!(h.is_finite() && h >= Group::Color.min_height() && h <= MAX_HEIGHT, "{bad} -> {h}");
    }
    // Unknown fields, wrong types and old UI state all load.
    let back: DockLayout = serde_json::from_value(json!({"order": ["layers"], "bogus": 1})).unwrap();
    assert_eq!(back.order(), vec![Group::Layers, Group::Brushes, Group::Color, Group::Properties, Group::Character, Group::Navigator, Group::History]);
    let mut ui = serde_json::to_value(crate::state::UiState::default()).unwrap();
    ui.as_object_mut().unwrap().remove("dock");
    let ui: crate::state::UiState = serde_json::from_value(ui).unwrap();
    assert_eq!(ui.dock, DockLayout::default());
}

fn app_with_layers() -> (PhotocraftApp, photocraft_doc::LayerId, photocraft_doc::LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let pixel = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.newAdjustmentLayer.curves", json!({})).unwrap();
    let adj = app.session.active().unwrap().active_layer.unwrap();
    assert_ne!(pixel, adj);
    app.sync_views();
    (app, pixel, adj)
}

fn harness(app: PhotocraftApp, size: egui::Vec2, theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    // 60 fps steps, so two clicks a frame apart count as a double-click.
    let mut h = Harness::builder().with_size(size).with_step_dt(1.0 / 60.0).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::dock_area(app, ui);
            egui::CentralPanel::default().show(ui, |_| {});
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.state_mut().ui.theme = theme;
    h.run_steps(4);
    h
}

/// The panel id of the native `group`'s tab `tab`.
fn panel_id(g: Group, tab: usize, pro: bool) -> &'static str {
    crate::panel_docking::normalize(g.tabs(pro)[tab]).unwrap()
}

fn rect_of(h: &Harness<'static, PhotocraftApp>, g: Group) -> Rect {
    probe::native_group_rect(h.state(), &h.ctx, g).unwrap_or_else(|| panic!("{g:?} not drawn"))
}

type Drawn = (Vec<(String, Rect)>, Vec<(craft_ui::docking::Hit<String>, Rect)>);

/// Every panel body and docking control drawn last frame.
fn drawn(h: &Harness<'static, PhotocraftApp>) -> Drawn {
    (crate::panel_docking::last_bodies(&h.ctx), craft_ui::docking::DockArea::<String>::hit_rects(&h.ctx, egui::Id::unique("photocraft-panel-docking")))
}

fn drag(h: &mut Harness<'static, PhotocraftApp>, from: Pos2, to: Pos2) {
    h.event(egui::Event::PointerMoved(from));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: from, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    for i in 1..=6 {
        h.event(egui::Event::PointerMoved(from + (to - from) * (i as f32 / 6.0)));
        h.run_steps(1);
    }
    h.event(egui::Event::PointerButton { pos: to, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn switching_layer_kinds_keeps_the_layers_panel_still() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let (app, pixel, adj) = app_with_layers();
        let mut h = harness(app, vec2(1200.0, 800.0), theme);
        assert!(probe::panel_rect(&h.ctx, "layers").is_some(), "{theme:?}: Layers drawn");
        let with_adj = drawn(&h);
        h.state_mut().run("layer.select", json!({"layer": pixel.0})).unwrap();
        h.run_steps(4);
        assert_eq!(drawn(&h), with_adj, "{theme:?}: selecting a pixel layer moved the dock groups");
        h.state_mut().run("layer.select", json!({"layer": adj.0})).unwrap();
        h.run_steps(4);
        assert_eq!(drawn(&h), with_adj, "{theme:?}: selecting an adjustment layer moved the dock groups");
        // Properties ↔ Adjustments tabs don't move anything either.
        h.state_mut().ui.dock_tabs.properties = 1;
        h.run_steps(3);
        assert_eq!(drawn(&h).0.iter().map(|(_, r)| *r).collect::<Vec<_>>(), with_adj.0.iter().map(|(_, r)| *r).collect::<Vec<_>>());
    }
}

/// A group height dragged in the dock is part of the UI state and the remembered layout.
#[test]
fn dragging_the_splitter_resizes_and_survives_a_ui_state_round_trip() {
    let (mut app, _, _) = app_with_layers();
    // CodeRabbit on #9606: the Color panel's options ride the same frames and preferences.
    app.ui.color_panel.mode = crate::state::ColorPanelMode::HsbSliders;
    let mut h = harness(app, vec2(1200.0, 800.0), ThemeKind::ProMedium);
    let props = rect_of(&h, Group::Properties);
    let color = rect_of(&h, Group::Color);
    let split = probe::hit_rect(&h.ctx, &craft_ui::docking::Hit::Divider(craft_ui::docking::Divider::Group("properties".into())))
        .expect("divider below Properties")
        .center();
    // Less than Properties has above its minimum, so the groups above don't give way.
    drag(&mut h, split, split - vec2(0.0, 40.0));
    let props2 = rect_of(&h, Group::Properties);
    assert!((props2.height() - (props.height() - 40.0)).abs() < 2.0, "{props:?} -> {props2:?}");
    assert_eq!(rect_of(&h, Group::Color), color, "the group above keeps its place");

    // Save and restore the UI state (what `ui.inspect` / `ui.set` and workspaces carry).
    let saved = serde_json::to_value(&h.state().ui).unwrap();
    let (mut app2, _, _) = app_with_layers();
    app2.ui = serde_json::from_value(saved).unwrap();
    let h2 = harness(app2, vec2(1200.0, 800.0), ThemeKind::ProMedium);
    assert_eq!(drawn(&h2).0, drawn(&h).0);

    // Remembered in the preferences once the mouse is up, and restored at the next launch.
    let prefs = h.state().session.prefs_to_json();
    let mut s2 = photocraft_engine::Session::new();
    s2.load_prefs_json(&prefs).unwrap();
    let mut app3 = PhotocraftApp::new(s2, crate::Services::default());
    restore(&mut app3);
    assert!(app3.ui.docking.is_some());
    assert_eq!(app3.ui.docking.as_ref().map(|w| &w.layout), h.state().ui.docking.as_ref().map(|w| &w.layout));
    crate::color_panel_ui::restore(&mut app3);
    assert_eq!(app3.ui.color_panel.mode, crate::state::ColorPanelMode::HsbSliders, "the Color panel's mode too");
    // …unless Remember Workspace Changes is off.
    let mut s3 = photocraft_engine::Session::new();
    s3.load_prefs_json(&prefs).unwrap();
    s3.prefs.edit(|p| p.workspace.remember_workspace_changes = false);
    let mut app4 = PhotocraftApp::new(s3, crate::Services::default());
    restore(&mut app4);
    assert!(app4.ui.docking.is_none());
    assert_eq!(app4.ui.dock, DockLayout::default());
}

#[test]
fn reset_workspace_restores_the_default_layout_and_new_workspaces_keep_theirs() {
    let (app, _, _) = app_with_layers();
    let mut h = harness(app, vec2(1200.0, 800.0), ThemeKind::ProMedium);
    let ctx = h.ctx.clone();
    h.state_mut().ui.dock.heights.insert(Group::Properties, 120.0);
    h.state_mut().ui.dock.set_collapsed(Group::Color, true);
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.newWorkspace", json!({"name": "Tall Layers"})).unwrap();
    let mine = h.state().ui.dock.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.essentials", json!({})).unwrap();
    h.run_steps(3);
    let default = drawn(&h).0;
    assert_eq!(h.state().ui.dock, DockLayout::default());
    h.state_mut().ui.dock.heights.insert(Group::Color, 300.0);
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
    h.run_steps(3);
    assert_eq!(drawn(&h).0, default);
    crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.select", json!({"name": "Tall Layers"})).unwrap();
    assert_eq!(h.state().ui.dock, mine);
}

#[test]
fn a_single_tab_click_expands_a_collapsed_panel() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        for tab_index in [0, 1] {
            let (mut app, _, _) = app_with_layers();
            app.ui.dock_tabs.color = 0;
            app.ui.dock.set_collapsed(Group::Color, true);
            // Locking prevents rearrangement, not opening an existing panel.
            app.session.prefs.edit(|p| p.workspace_locked = true);
            let mut h = harness(app, vec2(1200.0, 800.0), theme);
            let collapsed_height = rect_of(&h, Group::Color).height();
            let tab = probe::tab_rect(&h.ctx, panel_id(Group::Color, tab_index, is_pro(theme))).unwrap().center();
            click(&mut h, tab);
            assert!(!h.state().ui.dock.is_collapsed(Group::Color), "{theme:?}: tab {tab_index}");
            assert_eq!(h.state().ui.dock_tabs.color, tab_index);
            assert!(rect_of(&h, Group::Color).height() > collapsed_height);
        }
    }
}

/// The strip shows reordered tabs in their order; dragging one onto another group moves the
/// panel that was dragged, and only that panel.
#[test]
fn dragging_a_reordered_tab_onto_another_group_moves_that_panel() {
    let (mut app, _, _) = app_with_layers();
    // Gradients (original tab 2) drawn first: Gradients | Color | Swatches | Patterns.
    app.ui.dock.tab_order.insert(Group::Color, vec!["Gradients".into()]);
    let mut h = harness(app, vec2(1200.0, 800.0), ThemeKind::ProMedium);
    let tab = |h: &Harness<'static, PhotocraftApp>, panel: &str| probe::tab_rect(&h.ctx, panel).unwrap_or_else(|| panic!("{panel} tab drawn"));
    assert!(tab(&h, "gradients").left() < tab(&h, "color").left());
    let (source, destination) = (tab(&h, "gradients").center(), tab(&h, "layers").center());

    drag(&mut h, source, destination);

    assert!(h.state().ui.docking.is_some(), "moving one tab of the default layout customizes the workspace");
    let moved = probe::group_of(h.state(), "gradients");
    assert!(moved.contains(&"layers".to_owned()), "the dragged Gradients tab joined Layers: {moved:?}");
    assert!(!moved.contains(&"color".to_owned()), "Color stayed in its group: {moved:?}");
    assert_eq!(probe::group_of(h.state(), "color"), ["color", "swatches", "patterns"]);
    assert!(crate::panel_docking::valid(&h.state().ui.docking.as_ref().unwrap().layout));
}

#[test]
fn tiny_windows_do_not_panic() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        for (w, ht) in [(40.0, 30.0), (300.0, 80.0), (600.0, 200.0), (2000.0, 120.0)] {
            let (mut app, _, _) = app_with_layers();
            app.ui.panels.history = true;
            app.ui.panels.navigator = true;
            let mut h = harness(app, vec2(w, ht), theme);
            let (bodies, hits) = drawn(&h);
            for (_, r) in bodies {
                assert!(r.height() >= 0.0 && r.is_finite());
            }
            // Drag a splitter way off-screen.
            let divider = hits.iter().find(|(hit, _)| matches!(hit, craft_ui::docking::Hit::Divider(craft_ui::docking::Divider::Group(_))));
            if let Some((_, r)) = divider {
                let p = r.center();
                drag(&mut h, p, p + vec2(0.0, 5000.0));
                drag(&mut h, p, p - vec2(0.0, 5000.0));
            }
        }
    }
}

#[test]
fn the_rail_and_window_menu_never_lose_a_panel() {
    let (mut app, _, _) = app_with_layers();
    let ctx = egui::Context::default();
    // The rail collapses and expands a docked group; it never hides it (#129).
    rail_click(&mut app, Group::Layers, true);
    assert!(app.ui.panels.layers && app.ui.dock.is_collapsed(Group::Layers));
    rail_click(&mut app, Group::Layers, true);
    assert!(app.ui.panels.layers && !app.ui.dock.is_collapsed(Group::Layers));
    // Window › Layers on a collapsed group expands it instead of hiding it.
    app.ui.dock.set_collapsed(Group::Layers, true);
    crate::menus::invoke(&mut app, &ctx, "window.panel.layers", json!({})).unwrap();
    assert!(app.ui.panels.layers && !app.ui.dock.is_collapsed(Group::Layers));
    app.ui.dock.set_collapsed(Group::Color, true);
    crate::menus::invoke(&mut app, &ctx, "window.toggle.color", json!({})).unwrap();
    assert!(app.ui.panels.color && !app.ui.dock.is_collapsed(Group::Color));
    app.ui.dock.set_collapsed(Group::Color, true);
    crate::menus::invoke(&mut app, &ctx, "window.panel.gradients", json!({})).unwrap();
    assert!(app.ui.panels.color && !app.ui.dock.is_collapsed(Group::Color) && app.ui.dock_tabs.color == 2);
    // Hidden panels come back from the Window menu, expanded, on their tab.
    app.ui.panels.history = false;
    app.ui.dock.set_collapsed(Group::History, true);
    crate::menus::invoke(&mut app, &ctx, "window.panel.actions", json!({})).unwrap();
    assert!(app.ui.panels.history && !app.ui.dock.is_collapsed(Group::History) && app.ui.dock_tabs.history == 1);
    // Reset Workspace brings back everything the preset shows.
    app.ui.panels = crate::state::Panels { layers: false, properties: false, color: false, ..Default::default() };
    app.ui.dock_tabs.layers = 2;
    app.ui.dock.set_collapsed(Group::Properties, true);
    crate::menus::invoke(&mut app, &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
    assert!(app.ui.panels.layers && app.ui.panels.properties && app.ui.panels.color);
    assert_eq!(app.ui.dock, DockLayout::default());
    assert_eq!(app.ui.dock_tabs.layers, 0);
}

/// #129: clicking around the whole UI (layers, canvas, tools, options bar) never switches,
/// hides or moves a dock panel.
#[test]
fn clicking_around_the_ui_keeps_the_panels_put() {
    let mut h = Harness::builder().with_size(vec2(1440.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let (app, _, _) = app_with_layers();
        app
    });
    h.run_steps(8);
    let panels = h.state().ui.panels.clone();
    let tabs = h.state().ui.dock_tabs;
    let rects = drawn(&h);
    assert!(probe::panel_rect(&h.ctx, "layers").is_some(), "{rects:?}");
    let layers = rect_of(&h, Group::Layers);
    let canvas = h.state().last_canvas_rect;
    let mut points: Vec<Pos2> = Vec::new();
    // Layer rows (eyes, thumbnails, names) and the Layers footer.
    for dy in [118.0, 140.0, 150.0, 170.0, 182.0, 200.0] {
        for x in [layers.left() + 22.0, layers.left() + 50.0, layers.center().x] {
            points.push(Pos2::new(x, layers.top() + dy));
        }
    }
    // The canvas, the toolbar column and the options bar.
    points.extend([canvas.center(), canvas.left_top() + vec2(40.0, 40.0), canvas.right_bottom() - vec2(30.0, 30.0)]);
    for y in (110..460).step_by(33) {
        points.push(Pos2::new(20.0, y as f32));
    }
    for x in (40..1100).step_by(70) {
        points.push(Pos2::new(x as f32, 50.0));
    }
    for p in points {
        h.hover_at(p);
        h.run_steps(1);
        h.drag_at(p);
        h.run_steps(1);
        h.drop_at(p);
        h.run_steps(2);
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        // Popups and dialogs opened by a click are fine; panels changing aren't.
        let app = h.state();
        assert_eq!(app.ui.panels.layers, panels.layers, "click at {p:?} hid/showed Layers");
        assert_eq!(app.ui.panels.properties, panels.properties, "click at {p:?} hid/showed Properties");
        assert_eq!(app.ui.panels.color, panels.color, "click at {p:?} hid/showed Color");
        assert_eq!(app.ui.dock_tabs, tabs, "click at {p:?} switched a dock tab");
        assert!(app.ui.dock.collapsed.is_empty(), "click at {p:?} collapsed a group");
        if app.ui.dialogs.is_empty() && app.ui.shell.dialog.is_none() {
            assert_eq!(drawn(&h), rects, "click at {p:?} moved the dock groups");
        }
        h.state_mut().ui.dialogs.clear();
    }
}

#[test]
fn closing_tabs_keeps_stable_names_across_themes_and_workspace_round_trips() {
    let mut dock = DockLayout::default();
    // In Pro the first Color group tab is Color; in Studio it is Swatches.
    dock.hide_tab(Group::Color, 0, true);
    assert_eq!(dock.visible_tabs(Group::Color, true).iter().map(|(_, n)| *n).collect::<Vec<_>>(), vec!["Swatches", "Gradients", "Patterns"]);
    assert_eq!(dock.visible_tabs(Group::Color, false).iter().map(|(_, n)| *n).collect::<Vec<_>>(), vec!["Swatches", "Gradients", "Patterns"]);
    dock.hide_tab(Group::Color, 999, true);
    assert_eq!(dock.hidden_tabs[&Group::Color], vec!["Color"]);
    let saved = serde_json::to_value(&dock).unwrap();
    let mut restored: DockLayout = serde_json::from_value(saved).unwrap();
    assert_eq!(restored, dock);
    // Window › Color (Studio index 1) brings back precisely that tab.
    restored.show_tab(Group::Color, 1, false);
    assert!(restored.hidden_tabs.is_empty());
    assert_eq!(restored.visible_tabs(Group::Color, false).len(), 4);
}

fn right_click_tab(h: &mut Harness<'static, PhotocraftApp>, panel: &str) {
    let p = probe::tab_rect(&h.ctx, panel).unwrap_or_else(|| panic!("{panel} tab drawn")).center();
    h.hover_at(p);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
}

/// #1753: both themes offer Close for the clicked tab and Close Tab Group independently.
/// Closing the last tab hides its group; Window restores a hidden tab. (Studio floats
/// Properties outside the dock, so it closes Layers tabs instead.)
#[test]
fn panel_tab_context_menu_closes_one_tab_or_its_group() {
    use egui_kittest::kittest::Queryable;

    for (theme, g, window) in [(ThemeKind::ProMedium, Group::Properties, "window.panel.properties"), (ThemeKind::Studio, Group::Layers, "window.panel.layers")]
    {
        let pro = is_pro(theme);
        let names = g.tabs(pro);
        let ids: Vec<&str> = (0..names.len()).map(|i| panel_id(g, i, pro)).collect();
        let (app, _, _) = app_with_layers();
        let mut h = harness(app, vec2(1300.0, 850.0), theme);
        // Close the first tab: the group stays, showing (and selecting) the next one.
        right_click_tab(&mut h, ids[0]);
        h.get_by_label("Close Panel").click();
        h.run_steps(3);
        assert!(g.shown(&h.state().ui.panels), "{theme:?}: closing one tab keeps the group");
        assert_eq!(*g.tab_mut(&mut h.state_mut().ui.dock_tabs), 1);
        let rest: Vec<(usize, &str)> = names.iter().copied().enumerate().skip(1).collect();
        assert_eq!(h.state().ui.dock.visible_tabs(g, pro), rest);
        assert_eq!(probe::group_of(h.state(), ids[1]), ids[1..]);
        assert!(probe::tab_rect(&h.ctx, ids[0]).is_none());

        // Close the remaining tabs: the last one hides this group, not the other dock groups.
        for id in &ids[1..] {
            right_click_tab(&mut h, id);
            h.get_by_label("Close Panel").click();
            h.run_steps(3);
        }
        assert!(!g.shown(&h.state().ui.panels), "{theme:?}");
        assert!(h.state().ui.panels.color, "{theme:?}");

        // The Window menu reopens only the requested tab; hidden tabs stay hidden.
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, window, json!({})).unwrap();
        h.run_steps(3);
        assert!(g.shown(&h.state().ui.panels) && !h.state().ui.dock.is_collapsed(g));
        assert_eq!(h.state().ui.dock.visible_tabs(g, pro), vec![(0, names[0])]);
        assert_eq!(probe::group_of(h.state(), ids[0]), [ids[0]]);

        // A tab's Close Tab Group closes the entire group, which Window brings back with its one tab.
        right_click_tab(&mut h, ids[0]);
        h.get_by_label("Close Tab Group").click();
        h.run_steps(3);
        assert!(!g.shown(&h.state().ui.panels));
        crate::menus::invoke(h.state_mut(), &ctx, window, json!({})).unwrap();
        h.run_steps(3);
        assert_eq!(probe::group_of(h.state(), ids[0]), [ids[0]]);
        assert_eq!(h.state().ui.dock.visible_tabs(g, pro), vec![(0, names[0])]);
    }
}

/// #7265: Studio's floating Properties card has the docked group's Adjustments tab, so Window ›
/// Adjustments shows the grid of adjustment layers instead of Properties.
#[test]
fn studio_properties_card_shows_the_adjustments_tab() {
    use egui_kittest::kittest::Queryable;

    let (mut app, _, _) = app_with_layers();
    app.last_canvas_rect = Rect::from_min_size(Pos2::ZERO, vec2(900.0, 700.0));
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::panels::properties_window(app, &ctx);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, ThemeKind::Studio);
    h.state_mut().ui.theme = ThemeKind::Studio;
    h.run_steps(3);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "window.panel.adjustments", json!({})).unwrap();
    h.run_steps(3);
    assert!(h.state().ui.panels.properties && h.state().ui.dock_tabs.properties == 1);
    assert!(h.query_by_label("Add an adjustment").is_some(), "Window › Adjustments shows the Adjustments grid");
    // The Properties tab switches back to the layer's properties.
    h.get_by_label("Properties").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.dock_tabs.properties, 0);
    assert!(h.query_by_label("Add an adjustment").is_none());
    assert!(h.query_by_label("Reset to defaults").is_some(), "the Curves layer's properties");
    // Closing the Adjustments tab leaves Properties alone on the card.
    h.get_by_label("Adjustments").click();
    h.run_steps(3);
    let p = h.get_by_label("Adjustments").rect().center();
    h.hover_at(p);
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    h.get_by_label("Close").click();
    h.run_steps(3);
    assert!(h.state().ui.panels.properties && h.state().ui.dock_tabs.properties == 0);
    assert!(h.query_by_label("Adjustments").is_none() && h.query_by_label("Reset to defaults").is_some());
}

fn is_pro(theme: ThemeKind) -> bool {
    theme.is_pro()
}

#[test]
fn theme_switch_keeps_the_selected_color_group_panel() {
    use egui_kittest::kittest::Queryable;

    for theme in ThemeKind::ALL {
        for (tab, name) in Group::Color.tabs(true).iter().enumerate() {
            let (mut app, _, _) = app_with_layers();
            app.ui.dock_tabs.color = tab;
            let mut h = harness(app, vec2(1200.0, 900.0), ThemeKind::Pro);
            let ctx = h.ctx.clone();
            h.state_mut().set_theme(&ctx, theme);
            h.run_steps(4);
            assert_eq!(Group::Color.tabs(is_pro(theme)).get(h.state().ui.dock_tabs.color), Some(name), "{theme:?}");
            if *name == "Color" {
                // The spectrum field's hex readout, the same in every theme (#6000).
                assert!(h.query_by_label("#").is_some(), "{theme:?}: Color field missing");
                assert!(h.query_all_by_label("Hue").next().is_none(), "{theme:?}: no HSB sliders");
            }
            h.state_mut().set_theme(&ctx, ThemeKind::Pro);
            h.run_steps(4);
            assert_eq!(h.state().ui.dock_tabs.color, tab, "{theme:?}: switching back changed the panel");
        }
    }
}

/// Full app at `size` (1× scale) on a document with `n` layers named "Row 00", "Row 01", …
fn app_harness_rows(size: egui::Vec2, theme: ThemeKind, n: usize) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(size).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.theme = theme;
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        for i in 0..n {
            app.run("layer.new.layer", json!({"name": format!("Row {i:02}")})).unwrap();
        }
        app.sync_views();
        app
    });
    h.run_steps(8);
    h
}

/// Layer rows fully inside the Layers rows viewport.
fn rows_in_view(h: &Harness<'static, PhotocraftApp>, n: usize) -> usize {
    use egui_kittest::kittest::Queryable;
    let layers = rect_of(h, Group::Layers);
    (0..n)
        .filter(|i| {
            h.query_all_by_label(&format!("Row {i:02}"))
                .map(|q| q.rect())
                .any(|r| r.left() >= layers.left() - 1.0 && r.top() >= layers.top() + 26.0 && r.bottom() <= layers.bottom() - 36.0)
        })
        .count()
}

/// #147: the default workspace gives Layers the column's spare height: at least ten rows on a
/// 900 pt window, and a usable list on a 720 pt one.
#[test]
fn default_layout_shows_ten_layer_rows_at_900pt() {
    for (size, want) in [(vec2(1440.0, 900.0), 10), (vec2(1280.0, 720.0), 4)] {
        let h = app_harness_rows(size, ThemeKind::ProMedium, 40);
        let rows = rows_in_view(&h, 40);
        let groups = crate::panel_docking::last_bodies(&h.ctx);
        assert!(rows >= want, "{size:?}: {rows} rows visible, want ≥ {want}: {groups:?}");
        // Color and Properties stay open, just not at the expense of Layers.
        for g in [Group::Color, Group::Properties] {
            assert!(rect_of(&h, g).height() >= g.compact_height() - 0.5, "{g:?} at {size:?}: {groups:?}");
        }
    }
}

/// #147: in a tall column the groups above Layers keep their default heights, and heights the
/// user set stay as they are in a shorter one.
#[test]
fn defaults_give_way_to_layers_but_user_sizes_stay() {
    let (app, _, _) = app_with_layers();
    let h = harness(app, vec2(1440.0, 1400.0), ThemeKind::ProMedium);
    for g in [Group::Color, Group::Properties] {
        let height = rect_of(&h, g).height();
        assert!((height - g.default_height()).abs() < 2.0, "{g:?}: {height}");
    }
    let (mut app, _, _) = app_with_layers();
    app.ui.dock.heights.insert(Group::Properties, 340.0);
    app.ui.dock.heights.insert(Group::Color, 200.0);
    let h = harness(app, vec2(1440.0, 800.0), ThemeKind::ProMedium);
    for (g, want) in [(Group::Color, 200.0), (Group::Properties, 340.0)] {
        let height = rect_of(&h, g).height();
        assert!((height - want).abs() < 2.0, "{g:?}: {height}");
    }
}

/// #150: Window › Character opens a Character | Paragraph group next to Properties (which
/// stays), and toggles closed again; old saved layouts place it above Layers.
#[test]
fn window_character_opens_its_own_group_and_keeps_properties() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let (app, _, _) = app_with_layers();
        let mut h = harness(app, vec2(1440.0, 900.0), theme);
        let ctx = h.ctx.clone();
        crate::menus::invoke(h.state_mut(), &ctx, "window.panel.character", json!({})).unwrap();
        h.run_steps(3);
        let app = h.state();
        assert!(app.ui.panels.character && app.ui.panels.properties && app.ui.dock_tabs.character == 0, "{theme:?}");
        let ch = rect_of(&h, Group::Character);
        assert!(ch.height() > 60.0, "{theme:?}: Character expanded: {ch:?}");
        if is_pro(theme) {
            let props = rect_of(&h, Group::Properties);
            assert!(props.height() > 60.0 && props.bottom() <= ch.top(), "{theme:?}: Properties visible above Character");
        }
        assert!(rect_of(&h, Group::Layers).top() >= ch.bottom(), "{theme:?}: Layers stays the filler at the bottom");
        assert_eq!(crate::view_cmds::checked(h.state(), "window.panel.character"), Some(true));
        // Paragraph is the group's second tab; choosing it again on its tab closes the group.
        crate::menus::invoke(h.state_mut(), &ctx, "window.panel.paragraph", json!({})).unwrap();
        assert!(h.state().ui.panels.character && h.state().ui.dock_tabs.character == 1);
        crate::menus::invoke(h.state_mut(), &ctx, "window.panel.paragraph", json!({})).unwrap();
        h.run_steps(3);
        assert!(!h.state().ui.panels.character && h.state().ui.panels.properties, "{theme:?}");
        assert!(probe::native_group_rect(h.state(), &h.ctx, Group::Character).is_none());
        // Type › Panels › Character Panel always shows it; Reset Workspace closes it.
        crate::menus::invoke(h.state_mut(), &ctx, "type.panels.character", json!({})).unwrap();
        crate::menus::invoke(h.state_mut(), &ctx, "type.panels.character", json!({})).unwrap();
        assert!(h.state().ui.panels.character);
        crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.resetWorkspace", json!({})).unwrap();
        assert!(!h.state().ui.panels.character && h.state().ui.panels.properties);
    }
    // A layout saved before the group existed keeps Layers last.
    let old = DockLayout { order: vec![Group::Color, Group::Properties, Group::Navigator, Group::History, Group::Layers], ..Default::default() };
    assert_eq!(old.order(), Group::ALL.to_vec());
    let moved = DockLayout { order: vec![Group::Layers, Group::Color], ..Default::default() };
    assert_eq!(moved.order(), vec![Group::Layers, Group::Brushes, Group::Color, Group::Properties, Group::Character, Group::Navigator, Group::History]);
}

#[test]
fn tab_hides_all_panels_and_shift_tab_only_the_dock() {
    // #1313: Photoshop's Tab hides the Tools panel, the options bar and the panel dock (Tab again
    // brings back what it hid); ⇧Tab hides and shows the dock alone.
    let (mut app, _, _) = app_with_layers();
    let ctx = egui::Context::default();
    let run = |app: &mut PhotocraftApp, id: &str| crate::menus::invoke(app, &ctx, id, json!({})).unwrap();
    app.ui.panels.options_bar = false;
    run(&mut app, "window.togglePanels");
    let p = &app.ui.panels;
    assert!(!p.toolbar && !p.options_bar && !p.dock);
    run(&mut app, "window.togglePanels");
    let p = &app.ui.panels;
    assert!(p.toolbar && !p.options_bar && p.dock, "Tab brings back only what it hid");
    run(&mut app, "window.toggle.dock");
    assert!(!app.ui.panels.dock && app.ui.panels.toolbar, "⇧Tab hides only the dock");
    run(&mut app, "window.toggle.dock");
    assert!(app.ui.panels.dock);
    // Everything hidden by hand: Tab shows it all.
    (app.ui.panels.toolbar, app.ui.panels.options_bar, app.ui.panels.dock) = (false, false, false);
    run(&mut app, "window.togglePanels");
    assert!(app.ui.panels.toolbar && app.ui.panels.options_bar && app.ui.panels.dock);
    // The keys are Photoshop's.
    let bound = |key: &str| crate::shortcut_dispatch::bindings(&app).into_iter().find(|(_, sc)| Some(*sc) == crate::shortcuts::parse(key)).map(|(id, _)| id);
    assert_eq!(bound("Tab").as_deref(), Some("window.togglePanels"));
    assert_eq!(bound("Shift+Tab").as_deref(), Some("window.toggle.dock"));
    // An older saved UI state without the field shows the dock.
    let mut v = serde_json::to_value(crate::state::Panels::default()).unwrap();
    v.as_object_mut().unwrap().remove("dock");
    assert!(serde_json::from_value::<crate::state::Panels>(v).unwrap().dock);
}

#[test]
fn tab_in_full_screen_shows_and_hides_the_chrome_and_leaving_restores_the_panels() {
    // #6760: Full Screen Mode starts with the chrome hidden; Tab shows the menu bar and panels over
    // the canvas, Tab again hides them. Tab there never touches the panels' own state, so leaving
    // Full Screen brings them back exactly as they were, however many Tabs were pressed.
    let (mut app, _, _) = app_with_layers();
    let ctx = egui::Context::default();
    let run = |app: &mut PhotocraftApp, id: &str| crate::menus::invoke(app, &ctx, id, json!({})).unwrap();
    let state = |app: &PhotocraftApp| {
        let p = &app.ui.panels;
        (p.toolbar, p.options_bar, p.dock, p.hidden_by_tab)
    };
    // Before entering: the options bar hidden by hand, then everything hidden by Tab (outside Full Screen).
    for (hide_by_hand, tab_first) in [(false, false), (true, false), (false, true)] {
        app.ui.panels = crate::state::Panels::default();
        app.ui.panels.options_bar = !hide_by_hand;
        if tab_first {
            run(&mut app, "window.togglePanels");
        }
        let before = state(&app);
        run(&mut app, "view.screenMode.fullScreen");
        assert!(!app.ui.panels.full_screen_chrome, "hidden on entry");
        run(&mut app, "window.togglePanels");
        assert!(app.ui.panels.full_screen_chrome, "Tab shows the chrome");
        assert_eq!(state(&app), before, "Tab in Full Screen leaves the panels alone");
        run(&mut app, "view.screenMode.standard");
        assert!(!app.ui.panels.full_screen_chrome);
        assert_eq!(state(&app), before, "one Tab: restored as before entering");
        run(&mut app, "view.screenMode.fullScreen");
        run(&mut app, "window.togglePanels");
        run(&mut app, "window.togglePanels");
        assert!(!app.ui.panels.full_screen_chrome, "Tab twice hides the chrome again");
        assert_eq!(state(&app), before);
        run(&mut app, "view.screenMode.standard");
        assert_eq!(state(&app), before, "two Tabs: restored as before entering");
    }
    // Entering again starts hidden, even when the last visit left the chrome shown.
    run(&mut app, "view.screenMode.fullScreen");
    run(&mut app, "window.togglePanels");
    run(&mut app, "view.screenMode.standard");
    run(&mut app, "view.screenMode.fullScreen");
    assert!(!app.ui.panels.full_screen_chrome);
    // The flag is not saved.
    let v = serde_json::to_value(&app.ui.panels).unwrap();
    assert!(v.get("full_screen_chrome").is_none());
}

/// #2726: the Navigator sizes its own preview to the group (no outer scroll area, so no scroll
/// fade over the thumbnail), keeping the document's aspect, with the zoom controls below it.
#[test]
fn navigator_preview_fills_its_group_and_keeps_aspect() {
    assert!(Group::Navigator.scrolls_itself(0), "the Navigator tab lays itself out");
    assert!(!Group::Navigator.scrolls_itself(1) && !Group::Navigator.scrolls_itself(2), "Histogram and Info still scroll");
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let mut sizes = Vec::new();
        for tall in [150.0, 260.0, 420.0] {
            let mut h = Harness::builder().with_size(vec2(1200.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
                PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                app.ui.theme = theme;
                app.ui.panels.navigator = true;
                app.ui.dock.heights.insert(Group::Navigator, tall);
                app.run("file.new", json!({"width": 200, "height": 300})).unwrap();
                app.sync_views();
                app
            });
            h.run_steps(8);
            let group = rect_of(&h, Group::Navigator);
            let preview = crate::panels::last_navigator_preview(&h.ctx).unwrap_or_else(|| panic!("{theme:?} {tall}: no preview drawn"));
            assert!((group.height() - tall).abs() < 1.0, "{theme:?}: group {group:?}");
            assert!(group.contains_rect(preview), "{theme:?} {tall}: preview {preview:?} outside {group:?}");
            assert!((preview.width() / preview.height() - 2.0 / 3.0).abs() < 0.02, "{theme:?} {tall}: aspect {preview:?}");
            // The zoom controls stay inside the group, below the preview.
            let controls = crate::panels::last_navigator_controls(&h.ctx).unwrap_or_else(|| panic!("{theme:?} {tall}: no controls"));
            assert!(controls.top() >= preview.bottom() && controls.bottom() <= group.bottom() + 0.5, "{theme:?} {tall}: {controls:?} {preview:?} {group:?}");
            sizes.push(preview);
        }
        // A portrait document grows with every extra point of height the group gets.
        for pair in sizes.windows(2) {
            assert!(pair[1].height() > pair[0].height() + 80.0, "{theme:?}: {sizes:?}");
        }
    }
}

fn click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2) {
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(3);
}

#[test]
fn tab_order_is_by_name_and_bad_saved_orders_fall_back() {
    let mut l = DockLayout::default();
    let names = |l: &DockLayout, g, pro| l.visible_tabs(g, pro).into_iter().map(|(_, n)| n).collect::<Vec<_>>();
    l.tab_order.insert(Group::Layers, vec!["Paths".into()]);
    assert_eq!(l.visible_tabs(Group::Layers, true), vec![(2, "Paths"), (0, "Layers"), (1, "Channels")]);
    // Names, not positions: the order survives the Pro/Studio swap of Color and Swatches.
    l.tab_order.insert(Group::Color, vec!["Patterns".into(), "Color".into()]);
    assert_eq!(names(&l, Group::Color, true), ["Patterns", "Color", "Swatches", "Gradients"]);
    assert_eq!(names(&l, Group::Color, false), ["Patterns", "Color", "Swatches", "Gradients"]);
    // Hidden tabs keep their place for when they come back.
    l.hide_tab(Group::Color, 1, true);
    assert_eq!(names(&l, Group::Color, true), ["Patterns", "Color", "Gradients"]);
    // Garbage in a saved layout falls back per group, and never loses the rest of the layout.
    for bad in [
        json!(5),
        json!("x"),
        json!(null),
        json!({"layers": "paths"}),
        json!({"nope": ["Paths"]}),
        json!({"layers": [1, null, {"a": 1}]}),
        json!({"layers": ["Nope", "Paths", "Paths"]}),
    ] {
        let back: DockLayout = serde_json::from_value(json!({"order": ["layers"], "tabOrder": bad})).unwrap();
        assert_eq!(back.order().first(), Some(&Group::Layers), "{bad}");
        let layers = names(&back, Group::Layers, true);
        assert!(layers == ["Layers", "Channels", "Paths"] || layers == ["Paths", "Layers", "Channels"], "{bad}: {layers:?}");
        assert_eq!(back.visible_tabs(Group::Color, true).len(), 4);
    }
    let back: DockLayout = serde_json::from_value(json!({"tabOrder": {"layers": ["Channels"]}})).unwrap();
    assert_eq!(names(&back, Group::Layers, true), ["Channels", "Layers", "Paths"], "missing tabs follow in their default order");
}

/// #2272: a tab dragged along its strip reorders the group's tabs; the order persists in the
/// remembered layout and saved workspaces, and clicking still selects.
#[test]
fn dragging_a_tab_reorders_the_group_and_the_order_persists() {
    for theme in [ThemeKind::ProMedium, ThemeKind::Studio] {
        let (app, _, _) = app_with_layers();
        let mut h = harness(app, vec2(1200.0, 800.0), theme);
        let tab = |h: &Harness<'static, PhotocraftApp>, panel: &str| probe::tab_rect(&h.ctx, panel).unwrap_or_else(|| panic!("{panel} tab drawn"));
        let others = |h: &Harness<'static, PhotocraftApp>| probe::group_of(h.state(), "color");
        let color = others(&h);
        let (paths, layers) = (tab(&h, "paths"), tab(&h, "layers"));
        drag(&mut h, paths.center(), layers.left_center() + vec2(layers.width() / 4.0, 0.0));
        assert_eq!(probe::group_of(h.state(), "layers"), ["paths", "layers", "channels"], "{theme:?}: Paths moved before Layers");
        assert_eq!(others(&h), color, "{theme:?}: a tab drag along the strip leaves the other groups put");
        assert_eq!(h.state().ui.dock_tabs.layers, 2, "{theme:?}: the dragged tab is selected");

        // A plain click still selects, and selects the tab under the pointer.
        let layers = tab(&h, "layers").center();
        click(&mut h, layers);
        assert_eq!(h.state().ui.dock_tabs.layers, 0, "{theme:?}");
        assert_eq!(probe::group_of(h.state(), "layers"), ["paths", "layers", "channels"]);

        let ctx = h.ctx.clone();
        // Remembered across launches (prefs panelLayout) ...
        h.state_mut().session.prefs.edit(|p| p.workspace.remember_workspace_changes = true);
        persist(h.state_mut(), &ctx);
        let saved = h.state().session.prefs().panel_layout.clone();
        let (mut fresh, _, _) = app_with_layers();
        fresh.session.prefs.edit(|p| {
            p.workspace.remember_workspace_changes = true;
            p.panel_layout = saved.clone();
        });
        restore(&mut fresh);
        assert_eq!(probe::group_of(&fresh, "layers"), ["paths", "layers", "channels"], "{theme:?}");

        // ... and in a saved workspace, which Essentials resets and selecting brings back.
        crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.newWorkspace", json!({"name": "Paths First"})).unwrap();
        crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.essentials", json!({})).unwrap();
        assert_eq!(probe::group_of(h.state(), "layers"), ["layers", "channels", "paths"]);
        crate::menus::invoke(h.state_mut(), &ctx, "window.workspace.select", json!({"name": "Paths First"})).unwrap();
        h.run_steps(3);
        assert_eq!(probe::group_of(h.state(), "layers"), ["paths", "layers", "channels"], "{theme:?}");
        assert!(tab(&h, "paths").left() < tab(&h, "layers").left());
    }
}

#[test]
fn brush_settings_window_is_closed_on_restart_but_can_be_reopened() {
    let mut first = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    first.ui.panels.brush_settings = true;
    first.ui.panels.history = true;
    let saved = snapshot(&first);
    first.session.prefs.edit(|p| {
        p.workspace.remember_workspace_changes = true;
        p.panel_layout = saved.clone();
    });
    let persisted = first.session.prefs_to_json();

    let mut session = photocraft_engine::Session::new();
    session.load_prefs_json(&persisted).unwrap();
    let mut next = PhotocraftApp::new(session, crate::Services::default());
    restore(&mut next);
    assert!(!next.ui.panels.brush_settings, "a floating Brush Settings window should not reopen on startup");
    assert!(next.ui.panels.history, "remembered dock panel visibility is preserved");

    let ctx = egui::Context::default();
    crate::menus::invoke(&mut next, &ctx, "window.toggle.brushSettings", json!({})).unwrap();
    assert!(next.ui.panels.brush_settings, "Window > Brush Settings can still open it");

    let mut workspace = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    apply(&mut workspace, &saved);
    assert!(workspace.ui.panels.brush_settings, "switching workspaces still restores their saved layout");
}

/// The saved `docking` value is the app's own serialization of `craft_ui::docking::Layout`. A
/// value from an older/foreign docking schema that no longer deserializes is dropped (the
/// legacy layout is rebuilt); the rest of the workspace still applies, and nothing panics.
/// Panel widths still migrate separately through `panels::migrate_panel_state`, but a split/tab
/// arrangement from such a schema may reset. No 0.36 sample was available to prove the actual
/// transition.
#[test]
fn an_unreadable_docking_layout_is_dropped_and_the_rest_applies() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    let mut saved = snapshot(&app);
    saved["docking"] = json!({"root": "not-a-dock-node"});
    saved["timelineOpen"] = json!(true);
    apply(&mut app, &saved);
    assert!(app.ui.docking.is_none(), "the unreadable docking value is dropped");
    assert!(app.ui.timeline.open, "the rest of the layout still applies");
}

/// Layers panel options live in the preferences on their own (like Photoshop's Panel Options),
/// so a workspace layout neither saves nor restores them.
#[test]
fn workspace_layouts_leave_the_layers_panel_options_alone() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.layers_panel_options.show_filters = false;
    crate::layers_panel_ui::persist_options(&mut app);
    let stored = app.session.prefs().dialogs.get("ui.layersPanelOptions").cloned();
    assert!(stored.is_some(), "the options are saved in the preferences");

    let mut saved = snapshot(&app);
    assert!(saved.get("layersPanelOptions").is_none(), "a layout does not carry the options");

    // A layout saved by an earlier build still has the key; it must not win over the preferences.
    saved["layersPanelOptions"] = json!({"showFilters": true, "thumbnailSize": "large"});
    apply(&mut app, &saved);
    assert!(!app.ui.layers_panel_options.show_filters);
    assert_ne!(app.ui.layers_panel_options.thumbnail_size, crate::state::LayerThumbnailSize::Large);
    assert_eq!(app.session.prefs().dialogs.get("ui.layersPanelOptions").cloned(), stored);
}

#[test]
fn a_panel_window_the_os_moves_is_saved_once_it_keeps_still() {
    let (mut app, _, _) = app_with_layers();
    app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = true);
    app.run("window.panel.float", json!({"panel": "info"})).unwrap();
    let ctx = egui::Context::default();
    let frame = |app: &mut PhotocraftApp| ctx.run_ui(Default::default(), |ui| persist(app, ui.ctx())).textures_delta.clear();
    let place = |app: &mut PhotocraftApp, x: f32| {
        let window = app.ui.docking.as_mut().unwrap().layout.floating.iter_mut().find(|w| w.contains("info")).unwrap();
        window.native = Some([x, 100.0, 300.0, 200.0]);
    };
    frame(&mut app);
    let saved = app.session.prefs().panel_layout.clone();
    for x in 500..520 {
        place(&mut app, x as f32);
        frame(&mut app);
        assert_eq!(app.session.prefs().panel_layout, saved, "not saved while the window moves");
    }
    frame(&mut app);
    let native = app.session.prefs().panel_layout["docking"]["layout"]["floating"][0]["native"].clone();
    assert_eq!(native, json!([519.0, 100.0, 300.0, 200.0]), "saved once it keeps still");
}

#[test]
fn a_workspace_from_a_newer_version_shows_the_default_and_stays_saved() {
    let (mut app, _, _) = app_with_layers();
    app.session.prefs.edit(|p| p.workspace.remember_workspace_changes = true);
    let mut saved = snapshot(&app);
    saved["docking"] = json!({"version": 99, "layout": {"root": null, "floating": []}, "future": true});
    app.session.prefs.edit(|p| p.panel_layout = saved.clone());
    restore(&mut app);
    assert!(app.ui.docking.is_none(), "the default layout shows");
    let ctx = egui::Context::default();
    let frame = |app: &mut PhotocraftApp| ctx.run_ui(Default::default(), |ui| persist(app, ui.ctx())).textures_delta.clear();
    frame(&mut app);
    assert_eq!(app.session.prefs().panel_layout, saved, "an idle frame leaves it saved");

    app.run("window.panel.close", json!({"panel": "layers"})).unwrap();
    frame(&mut app);
    let docking = &app.session.prefs().panel_layout["docking"];
    assert_eq!(docking["version"], json!(craft_ui::docking::Workspace::<String>::VERSION), "a changed layout replaces it: {docking}");
}
