//! #11381: the Move tool's options bar. The two Distribute buttons, the "..." icon panel (it
//! closes on a second click, not on Escape) and the gear menu with its three check items.

use egui::{Rect as ScreenRect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::LayerId;
use photocraft_geom::Rect;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

fn paint(app: &mut PhotocraftApp, r: Rect) -> LayerId {
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(r, &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.active().unwrap().active_layer.unwrap()
}

fn bounds(app: &PhotocraftApp, id: LayerId) -> Rect {
    let doc = &app.session.active().unwrap().doc;
    photocraft_compose::bounds::content_bounds(doc.layer(id).unwrap().surface().unwrap())
}

fn select_all(app: &mut PhotocraftApp, ids: &[LayerId]) {
    app.session.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
    for id in &ids[1..] {
        app.session.execute("layer.select", json!({"layer": id.0, "mode": "toggle"})).unwrap();
    }
}

/// A 120x120 document with three layers side by side and on top of each other unevenly, the
/// Move tool current. Returns the app and the layers, bottom to top.
fn app() -> (PhotocraftApp, [LayerId; 3]) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 120, "height": 120})).unwrap();
    let a = paint(&mut app, Rect::new(0, 0, 10, 10));
    let b = paint(&mut app, Rect::new(12, 50, 32, 54));
    let c = paint(&mut app, Rect::new(70, 60, 100, 80));
    app.sync_views();
    app.ui.tool = Tool::Move;
    (app, [a, b, c])
}

/// The Move tool's align controls and gear, as the options bar lays them out.
fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(700.0, 200.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| {
                ui.horizontal(|ui| {
                    crate::align_bar::options(app, ui);
                    crate::align_bar::gear(app, ui);
                });
            });
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h
}

fn rect_of(h: &Harness<'static, PhotocraftApp>, label: &str) -> ScreenRect {
    h.get_by_label(label).rect()
}

fn click(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(4);
}

const BAR: [&str; 10] = [
    "Align left edges",
    "Align horizontal centers",
    "Align right edges",
    "Distribute vertically",
    "Align top edges",
    "Align vertical centers",
    "Align bottom edges",
    "Distribute horizontally",
    "Align and distribute",
    "Options",
];

#[test]
fn bar_buttons_and_tooltips_are_in_the_measured_order() {
    let (app, _) = app();
    let h = harness(app);
    let xs: Vec<f32> = BAR.iter().map(|l| rect_of(&h, l).left()).collect();
    assert!(xs.windows(2).all(|w| w[0] < w[1]), "left to right: {BAR:?} at {xs:?}");
    let ys: Vec<f32> = BAR.iter().map(|l| rect_of(&h, l).center().y).collect();
    assert!(ys.iter().all(|y| (y - ys[0]).abs() < 1.0), "one row: {ys:?}");
}

#[test]
fn distribute_vertically_button_runs_its_command() {
    let (mut app, [a, b, c]) = app();
    select_all(&mut app, &[a, b, c]);
    let mut h = harness(app);
    assert!(!h.get_by_label("Distribute vertically").accesskit_node().is_disabled());
    click(&mut h, "Distribute vertically");
    // The same result as Layer > Distribute > Vertically: spans 0..80, heights 34, gaps 23.
    assert_eq!(bounds(h.state(), b), Rect::new(12, 33, 32, 37));
    assert_eq!(bounds(h.state(), a), Rect::new(0, 0, 10, 10));
}

#[test]
fn distribute_horizontally_button_runs_its_command() {
    let (mut app, [a, b, c]) = app();
    select_all(&mut app, &[a, b, c]);
    let mut h = harness(app);
    assert!(!h.get_by_label("Distribute horizontally").accesskit_node().is_disabled());
    click(&mut h, "Distribute horizontally");
    // Spans 0..100, widths 60, gaps 20: the middle layer starts at 30.
    assert_eq!(bounds(h.state(), b), Rect::new(30, 50, 50, 54));
    assert_eq!(bounds(h.state(), c), Rect::new(70, 60, 100, 80));
}

#[test]
fn distribute_buttons_are_greyed_with_too_few_layers_like_the_menu() {
    let (mut app, [a, b, c]) = app();
    select_all(&mut app, &[a, b]);
    let mut h = harness(app);
    for label in ["Distribute vertically", "Distribute horizontally"] {
        assert!(h.get_by_label(label).accesskit_node().is_disabled(), "{label} with two layers");
    }
    for cmd in ["layer.distribute.vertically", "layer.distribute.horizontally"] {
        assert!(!h.state().session.is_enabled(cmd), "{cmd} is off in the menu too");
    }
    // Align still works with two layers.
    assert!(!h.get_by_label("Align left edges").accesskit_node().is_disabled());
    select_all(h.state_mut(), &[a, b, c]);
    h.run_steps(2);
    assert!(!h.get_by_label("Distribute vertically").accesskit_node().is_disabled());
}

#[test]
fn ellipsis_panel_holds_align_and_distribute_and_closes_on_a_second_click_not_escape() {
    let (app, _) = app();
    let mut h = harness(app);
    assert!(h.query_by_label("Align: Left Edges").is_none(), "closed at first");
    click(&mut h, "Align and distribute");
    assert!(crate::align_bar::panel_is_open(&h.ctx));

    let align = ["Left Edges", "Horizontal Centers", "Right Edges", "Top Edges", "Vertical Centers", "Bottom Edges"];
    let distribute = ["Top Edges", "Vertical Centers", "Bottom Edges", "Left Edges", "Horizontal Centers", "Right Edges"];
    let tips: Vec<String> = align.iter().map(|l| format!("Align: {l}")).chain(distribute.iter().map(|l| format!("Distribute: {l}"))).collect();
    let rects: Vec<ScreenRect> = tips.iter().map(|t| rect_of(&h, t)).collect();
    // Align row above the Distribute row; each row runs left to right.
    assert!(rects[..6].windows(2).all(|w| w[0].left() < w[1].left()), "align row: {rects:?}");
    assert!(rects[6..].windows(2).all(|w| w[0].left() < w[1].left()), "distribute row: {rects:?}");
    assert!(rects[5].bottom() <= rects[6].top() + 0.5, "the Distribute group is below the Align group");
    // The panel hangs under the "..." button.
    assert!(rects[0].top() > rect_of(&h, "Align and distribute").bottom() - 0.5);

    // Escape leaves it open.
    h.key_press(egui::Key::Escape);
    h.run_steps(4);
    assert!(crate::align_bar::panel_is_open(&h.ctx));
    assert!(h.query_by_label("Align: Left Edges").is_some());

    // A second click on the button closes it.
    click(&mut h, "Align and distribute");
    assert!(!crate::align_bar::panel_is_open(&h.ctx));
    assert!(h.query_by_label("Align: Left Edges").is_none());
}

#[test]
fn panel_icons_run_their_commands() {
    let (mut app, [a, b, c]) = app();
    select_all(&mut app, &[a, b, c]);
    let mut h = harness(app);
    click(&mut h, "Align and distribute");
    click(&mut h, "Distribute: Left Edges");
    // Left edges 0, 12, 70 -> the middle one moves to 35.
    assert_eq!(bounds(h.state(), b), Rect::new(35, 50, 55, 54));
}

#[test]
fn gear_menu_has_three_check_items_on_by_default_that_switch_the_options() {
    let (app, _) = app();
    let mut h = harness(app);
    click(&mut h, "Options");
    for label in ["Show layer bounds on hover", "Expand layer group on click", "Show hover bounds from Layers"] {
        assert!(h.query_by_label(label).is_some(), "{label} is in the gear menu");
    }
    click(&mut h, "Show layer bounds on hover");
    assert!(!h.state().ui.tool_options.move_show_hover_bounds);
    click(&mut h, "Expand layer group on click");
    assert!(!h.state().ui.tool_options.move_expand_group_on_click);
    click(&mut h, "Show hover bounds from Layers");
    assert!(!h.state().ui.tool_options.move_show_layers_hover_bounds);
}

#[test]
fn gear_toggles_default_on_are_saved_and_reset() {
    let o = ToolOptions::default();
    assert!(o.move_show_hover_bounds && o.move_expand_group_on_click && o.move_show_layers_hover_bounds);

    // Saved with the Move tool options: they are in the serde form that ui.inspect and ui.set use,
    // and an older save without them reads as on.
    let mut v = serde_json::to_value(&o).unwrap();
    for key in ["move_show_hover_bounds", "move_expand_group_on_click", "move_show_layers_hover_bounds"] {
        assert_eq!(v[key], json!(true), "{key}");
        v[key] = json!(false);
    }
    let off: ToolOptions = serde_json::from_value(v.clone()).unwrap();
    assert!(!off.move_show_hover_bounds && !off.move_expand_group_on_click && !off.move_show_layers_hover_bounds);
    for key in ["move_show_hover_bounds", "move_expand_group_on_click", "move_show_layers_hover_bounds"] {
        v.as_object_mut().unwrap().remove(key);
    }
    let old: ToolOptions = serde_json::from_value(v).unwrap();
    assert!(old.move_show_hover_bounds && old.move_expand_group_on_click && old.move_show_layers_hover_bounds);

    // Tool > Reset Tool puts them back.
    let (mut app, _) = app();
    app.ui.tool_options = off;
    app.ui.tool_options.feather = 12.0;
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "tool.reset", json!({})).unwrap();
    let o = &app.ui.tool_options;
    assert!(o.move_show_hover_bounds && o.move_expand_group_on_click && o.move_show_layers_hover_bounds);
    assert_eq!(o.feather, 12.0, "another tool's option is left alone");
}

#[test]
fn show_layer_bounds_on_hover_switches_the_canvas_outline() {
    let (mut app, [_, b, _]) = app();
    let b_bounds = bounds(&app, b);
    assert_eq!(crate::move_hover::target(&mut app, Some([14.0, 52.0]), false, false), Some(b_bounds));
    app.ui.tool_options.move_show_hover_bounds = false;
    assert_eq!(crate::move_hover::target(&mut app, Some([14.0, 52.0]), false, false), None);
    assert_eq!(app.canvas_hover_layer, None);
    app.ui.tool_options.move_show_hover_bounds = true;
    assert_eq!(crate::move_hover::target(&mut app, Some([14.0, 52.0]), false, false), Some(b_bounds));
}

#[test]
fn show_hover_bounds_from_layers_switches_the_layers_panel_outline() {
    let (mut app, [a, _, _]) = app();
    app.hover_layer = Some(a);
    let a_bounds = bounds(&app, a);
    assert_eq!(crate::move_hover::active_outline(&mut app, None, false, false), Some(a_bounds));
    app.ui.tool_options.move_show_layers_hover_bounds = false;
    assert_eq!(crate::move_hover::active_outline(&mut app, None, false, false), None);
    // The canvas outline is a separate switch.
    assert!(crate::move_hover::active_outline(&mut app, Some([14.0, 52.0]), false, false).is_some());
    // Preferences > Show Highlight on Rollover stays the master switch.
    app.ui.tool_options.move_show_layers_hover_bounds = true;
    app.session.edit_prefs(|p| p.tools.show_highlight_on_rollover = false);
    assert_eq!(crate::move_hover::active_outline(&mut app, None, false, false), None);
}

fn expanded(app: &PhotocraftApp, group: LayerId) -> bool {
    match &app.session.active().unwrap().doc.layer(group).unwrap().content {
        photocraft_doc::LayerContent::Group(g) => g.expanded,
        _ => panic!("not a group"),
    }
}

#[test]
fn expand_layer_group_on_click_switches_the_opening_of_closed_parent_groups() {
    for on in [true, false] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        let inner = paint(&mut app, Rect::new(8, 8, 24, 24));
        app.session.execute("layer.groupLayers", json!({})).unwrap();
        let group = app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.is_group()).map(|(_, _, l)| l.id).unwrap();
        app.session.execute("layer.setExpanded", json!({"layer": group.0, "expanded": false})).unwrap();
        app.session.execute("layer.select", json!({"layer": group.0})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_expand_group_on_click = on;
        let ctx = egui::Context::default();
        // The first frame only records the active layer; then a canvas click picks the inner one.
        assert_eq!(crate::layer_reveal::track(&mut app, &ctx), None);
        // The Move tool picks the layer under the pointer (Auto-Select targets layers, not groups).
        app.ui.tool_options.move_target = "layer".into();
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, egui::Modifiers::default());
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 12.0, y: 12.0 }, egui::Modifiers::default());
        // The row is still revealed (scrolled to) either way.
        assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
        assert_eq!(expanded(&app, group), on, "Expand layer group on click = {on}");
    }
}

/// A 64x64 document: `inner` painted at (8..24)^2 inside a closed group, the Background-less
/// group selected, `tool` current and the gear option set to `on`. The reveal has seen the frame.
fn closed_group_doc(tool: Tool, on: bool) -> (PhotocraftApp, LayerId, LayerId, egui::Context) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    let inner = paint(&mut app, Rect::new(8, 8, 24, 24));
    app.session.execute("layer.groupLayers", json!({})).unwrap();
    let group = app.session.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.is_group()).map(|(_, _, l)| l.id).unwrap();
    app.session.execute("layer.setExpanded", json!({"layer": group.0, "expanded": false})).unwrap();
    app.session.execute("layer.select", json!({"layer": group.0})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app.ui.tool_options.move_expand_group_on_click = on;
    app.ui.tool_options.move_target = "layer".into();
    let ctx = egui::Context::default();
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), None);
    (app, inner, group, ctx)
}

/// A press and release at (12, 12) with Ctrl/Cmd held (a temporary Move from another tool).
fn ctrl_click(app: &mut PhotocraftApp) {
    use crate::canvas::{ToolEvent, tool_event};
    let mods = egui::Modifiers { command: true, ctrl: true, ..Default::default() };
    tool_event(app, ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x: 12.0, y: 12.0 }, mods);
}

#[test]
fn expand_layer_group_on_click_off_keeps_the_group_closed_for_a_ctrl_click_from_another_tool() {
    // Auto-Select off, so the Ctrl-click is the Move way of picking from the Brush.
    let (mut app, inner, group, ctx) = closed_group_doc(Tool::Brush, false);
    app.ui.tool_options.move_auto_select = false;
    ctrl_click(&mut app);
    assert_eq!(app.session.active().unwrap().active_layer, Some(inner), "the Ctrl-click picked the layer");
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner), "the row still scrolls into view");
    assert!(!expanded(&app, group), "the group stays closed");
}

#[test]
fn expand_layer_group_on_click_on_opens_the_group_for_a_ctrl_click_from_another_tool() {
    let (mut app, inner, group, ctx) = closed_group_doc(Tool::Brush, true);
    app.ui.tool_options.move_auto_select = false;
    ctrl_click(&mut app);
    assert_eq!(app.session.active().unwrap().active_layer, Some(inner));
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
    assert!(expanded(&app, group), "the group opens");
}

#[test]
fn expand_layer_group_on_click_off_keeps_the_group_closed_for_a_move_tool_click() {
    let (mut app, inner, group, ctx) = closed_group_doc(Tool::Move, false);
    use crate::canvas::{ToolEvent, tool_event};
    tool_event(&mut app, ToolEvent::Down { x: 12.0, y: 12.0, pressure: 1.0 }, egui::Modifiers::default());
    tool_event(&mut app, ToolEvent::Up { x: 12.0, y: 12.0 }, egui::Modifiers::default());
    assert_eq!(app.session.active().unwrap().active_layer, Some(inner));
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
    assert!(!expanded(&app, group));
}

#[test]
fn expand_layer_group_on_click_off_still_opens_the_group_for_other_active_layer_changes() {
    let (mut app, inner, group, ctx) = closed_group_doc(Tool::Brush, false);
    app.run("layer.select", json!({"layer": inner.0})).unwrap();
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
    assert!(expanded(&app, group), "a command that changes the active layer reveals it");
}

#[test]
fn expand_layer_group_on_click_governs_a_ctrl_click_pick_of_a_position_locked_layer() {
    for on in [false, true] {
        let (mut app, inner, group, ctx) = closed_group_doc(Tool::Brush, on);
        app.ui.tool_options.move_auto_select = false;
        app.session.execute("layer.setProps", json!({"layer": inner.0, "locks": {"position": true}})).unwrap();
        ctrl_click(&mut app);
        assert_eq!(app.session.active().unwrap().active_layer, Some(inner), "the Ctrl-click picked the locked layer (on = {on})");
        assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
        assert_eq!(expanded(&app, group), on, "Expand layer group on click = {on}");
    }
}

#[test]
fn a_pick_marker_the_hidden_layers_panel_never_read_does_not_hide_a_later_command_change() {
    let (mut app, inner, group, ctx) = closed_group_doc(Tool::Brush, false);
    app.ui.tool_options.move_auto_select = false;
    // A Move-way pick of the inner layer while the Layers panel draws nothing (no `track`).
    ctrl_click(&mut app);
    assert_eq!(app.session.active().unwrap().active_layer, Some(inner));
    // Frames pass without the panel, then commands change the layer twice and land on it again.
    app.frame += 5;
    app.run("layer.select", json!({"layer": group.0})).unwrap();
    app.run("layer.select", json!({"layer": inner.0})).unwrap();
    // The panel is shown again: the change was a command's, so the group opens.
    app.frame += 1;
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
    assert!(expanded(&app, group), "a command change after a stale pick expands the group");
}

#[test]
fn a_pick_marker_survives_any_number_of_frames_while_the_layers_panel_is_hidden() {
    let (mut app, inner, group, ctx) = closed_group_doc(Tool::Brush, false);
    app.ui.tool_options.move_auto_select = false;
    ctrl_click(&mut app);
    app.frame += 5;
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
    assert!(!expanded(&app, group), "the pick is still the latest change of the active layer");
}

#[test]
fn commands_after_a_pick_in_the_same_frame_or_the_next_make_it_a_command_change() {
    for gap in [0, 1] {
        let (mut app, inner, group, ctx) = closed_group_doc(Tool::Brush, false);
        app.ui.tool_options.move_auto_select = false;
        ctrl_click(&mut app);
        app.frame += gap;
        app.run("layer.select", json!({"layer": group.0})).unwrap();
        app.run("layer.select", json!({"layer": inner.0})).unwrap();
        assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
        assert!(expanded(&app, group), "selects {gap} frame(s) after the pick reveal the layer");
    }
}

#[test]
fn a_pick_marker_is_read_once_and_governs_the_next_change_only() {
    let (mut app, inner, group, ctx) = closed_group_doc(Tool::Brush, false);
    app.ui.tool_options.move_auto_select = false;
    ctrl_click(&mut app);
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
    assert!(!expanded(&app, group));
    app.run("layer.select", json!({"layer": group.0})).unwrap();
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(group));
    app.run("layer.select", json!({"layer": inner.0})).unwrap();
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
    assert!(expanded(&app, group), "a later command change opens the group");
}

#[test]
fn direct_selection_calls_after_a_pick_make_it_a_command_change() {
    let (mut app, inner, group, ctx) = closed_group_doc(Tool::Brush, false);
    app.ui.tool_options.move_auto_select = false;
    ctrl_click(&mut app);
    app.session.select_layer(group).unwrap();
    app.session.select_layer(inner).unwrap();
    assert_eq!(crate::layer_reveal::track(&mut app, &ctx), Some(inner));
    assert!(expanded(&app, group), "a direct selection away and back is not a pick");
}
