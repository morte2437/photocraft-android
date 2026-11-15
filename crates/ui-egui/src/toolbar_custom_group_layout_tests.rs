//! #14459: the toolbar's slots come from a stored group layout that `edit.toolbar` changes. Each
//! toolbar group is one slot listing its tools in order, the Extra Tools groups get no slot,
//! Shift+letter steps through a key's tools in layout order across slots, an empty toolbar is
//! allowed, `reset` brings back the default layout, and older settings still load.

use egui::{Key, Modifiers, Rect, vec2};
use photocraft_engine::prefs::{DEFAULT_TOOLBAR, Preferences, ToolbarCustomization};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::panels::toolbar::{TOOL_SECTIONS, layout_sections, toolbar};
use crate::state::Tool;
use crate::theme::Tokens;

fn app() -> PhotocraftApp {
    PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default())
}

fn names(groups: &Value) -> Vec<Vec<String>> {
    serde_json::from_value(groups.clone()).unwrap()
}

fn shift_press(app: &mut PhotocraftApp, key: Key) {
    let modifiers = Modifiers::SHIFT;
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::ModifiersChanged(modifiers), egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(app, &ctx);
    ctx.end_pass().textures_delta.clear();
}

/// The tool slots the toolbar drew in its last pass (clickable tool-sized buttons).
fn tool_buttons(app: &mut PhotocraftApp) -> usize {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    for time in [0.0, 0.1] {
        let mut out = ctx.run_ui(
            egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 1800.0))), ..Default::default() },
            |ui| toolbar(app, ui),
        );
        out.textures_delta.clear();
    }
    let size = egui::Vec2::splat(if Tokens::get(&ctx).pro { 30.0 } else { 36.0 });
    ctx.viewport(|v| v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == size && w.sense.senses_click()).count())
}

/// The slots `layout_sections` gives for the app's stored layout, as tool groups.
fn slots(app: &PhotocraftApp) -> Vec<Vec<Tool>> {
    layout_sections(&app.session.prefs().toolbar).into_iter().flatten().map(|(_, tools)| tools).collect()
}

#[test]
fn engine_tool_names_are_the_toolbar_tools() {
    for tool in Tool::ALL {
        let name = format!("{tool:?}");
        assert_eq!(ToolbarCustomization::tool_name(&name).as_deref(), Some(name.as_str()), "{tool:?}");
        assert_eq!(ToolbarCustomization::tool_name(tool.label()).as_deref(), Some(name.as_str()), "{tool:?} by its label");
        assert_eq!(Tool::from_name(&name), Some(tool));
    }
    let defaults: Vec<Vec<String>> = TOOL_SECTIONS.iter().flat_map(|s| s.iter()).map(|slot| slot.iter().map(|t| format!("{t:?}")).collect()).collect();
    let engine: Vec<Vec<String>> = DEFAULT_TOOLBAR.iter().map(|g| g.iter().map(|t| (*t).to_string()).collect()).collect();
    assert_eq!(engine, defaults, "the default layout is today's slot order");
    assert_eq!(ToolbarCustomization::tool_name("not a tool"), None);
}

#[test]
fn edit_toolbar_stores_and_returns_the_group_layout() {
    let mut app = app();
    let out = app
        .run(
            "edit.toolbar",
            json!({
                "toolbar": [["Move"], ["Lasso", "elliptical marquee", "Polygonal Lasso Tool"], ["Bogus Tool"], ["move", "Zoom"]],
                "extras": [["Hand", "Rotate View"], ["Zoom"]],
            }),
        )
        .unwrap();
    let toolbar = names(&out["toolbar"]);
    assert_eq!(
        toolbar,
        vec![vec!["Move"], vec!["Lasso", "EllipseMarquee", "PolygonLasso"], vec!["Zoom"]],
        "unknown names skipped, a repeat keeps its first place"
    );
    let extras = names(&out["extras"]);
    assert_eq!(extras[0], vec!["Hand", "RotateView"]);
    // Every tool listed nowhere is appended to the extras as its own group, once.
    let listed: Vec<&String> = toolbar.iter().chain(&extras).flatten().collect();
    assert_eq!(listed.len(), Tool::ALL.len());
    assert!(extras[1..].iter().all(|g| g.len() == 1));
    assert_eq!(extras[1], vec!["Artboard"], "the rest follow in toolbar order");
    // The stored layout is what a second call returns.
    assert_eq!(app.run("edit.toolbar", json!({"hidden": []})).unwrap(), out);
    assert!(app.run("edit.toolbar", json!({"toolbar": "Move"})).is_err(), "a layout is a list of groups");
}

#[test]
fn the_toolbar_draws_one_slot_per_toolbar_group() {
    let mut app = app();
    let all = tool_buttons(&mut app);
    let defaults: usize = TOOL_SECTIONS.iter().map(|s| s.len()).sum();
    app.run("edit.toolbar", json!({"toolbar": [["Move"], ["Lasso", "EllipseMarquee"], ["Brush"]]})).unwrap();
    assert_eq!(slots(&app), vec![vec![Tool::Move], vec![Tool::Lasso, Tool::EllipseMarquee], vec![Tool::Brush]]);
    assert_eq!(tool_buttons(&mut app), all - defaults + 3, "three slots");
    // A one-tool group has no flyout; the toolbar draws the triangle only for slots with more.
    assert_eq!(slots(&app).iter().filter(|s| s.len() > 1).count(), 1);
}

#[test]
fn an_empty_toolbar_list_leaves_no_tool_slots() {
    let mut app = app();
    let all = tool_buttons(&mut app);
    let defaults: usize = TOOL_SECTIONS.iter().map(|s| s.len()).sum();
    let out = app.run("edit.toolbar", json!({"toolbar": []})).unwrap();
    assert!(names(&out["toolbar"]).is_empty());
    assert_eq!(names(&out["extras"]).len(), Tool::ALL.len());
    assert!(slots(&app).is_empty());
    assert_eq!(tool_buttons(&mut app), all - defaults);
}

#[test]
fn shift_letter_cycles_in_layout_order_across_slots() {
    let mut app = app();
    // Elliptical Marquee moved into the Lasso group keeps M.
    app.run("edit.toolbar", json!({"toolbar": [["RectMarquee", "SingleRowMarquee"], ["Lasso", "EllipseMarquee"]]})).unwrap();
    app.ui.tool = Tool::RectMarquee;
    shift_press(&mut app, Key::M);
    assert_eq!(app.ui.tool, Tool::EllipseMarquee);
    shift_press(&mut app, Key::M);
    assert_eq!(app.ui.tool, Tool::RectMarquee);
    // A reordered group steps in its new order: Magic Wand, Quick Selection, Object Selection.
    app.run("edit.toolbar", json!({"toolbar": [["MagicWand", "QuickSelection", "ObjectSelection"]]})).unwrap();
    app.ui.tool = Tool::MagicWand;
    for expected in [Tool::QuickSelection, Tool::ObjectSelection, Tool::MagicWand] {
        shift_press(&mut app, Key::W);
        assert_eq!(app.ui.tool, expected);
    }
}

#[test]
fn reset_restores_the_default_layout() {
    let mut app = app();
    app.run("edit.toolbar", json!({"toolbar": [["Zoom", "Move"]]})).unwrap();
    let out = app.run("edit.toolbar", json!({"reset": true})).unwrap();
    let toolbar = names(&out["toolbar"]);
    let expected: Vec<Vec<String>> = DEFAULT_TOOLBAR.iter().map(|g| g.iter().map(|t| (*t).to_string()).collect()).collect();
    assert_eq!(toolbar, expected);
    assert!(names(&out["extras"]).is_empty());
    assert_eq!(slots(&app).len(), TOOL_SECTIONS.iter().map(|s| s.len()).sum::<usize>());
    assert_eq!(layout_sections(&app.session.prefs().toolbar).len(), TOOL_SECTIONS.len(), "the default keeps its dividers");
}

#[test]
fn older_settings_still_load() {
    // Only the flat hidden list: its tools become extras groups, in toolbar order.
    let prefs: Preferences = serde_json::from_value(json!({"toolbar": {"hidden": ["Sponge", "Move"]}})).unwrap();
    let layout = prefs.toolbar.layout();
    assert_eq!(layout.extras, vec![vec!["Move".to_string()], vec!["Sponge".to_string()]]);
    assert!(layout.toolbar.iter().any(|g| g == &vec!["Dodge".to_string(), "Burn".to_string()]));
    assert!(layout.toolbar.iter().flatten().all(|t| t != "Sponge" && t != "Move"));
    // No toolbar fields at all: the default layout.
    let prefs: Preferences = serde_json::from_value(json!({})).unwrap();
    assert_eq!(prefs.toolbar.layout().toolbar.len(), DEFAULT_TOOLBAR.len());
    assert!(prefs.toolbar.layout().extras.is_empty());
    // A stored layout survives a round trip through the settings file.
    let mut app = app();
    app.run("edit.toolbar", json!({"toolbar": [["Brush"]]})).unwrap();
    let back: Preferences = serde_json::from_str(&app.session.prefs_to_json()).unwrap();
    assert_eq!(back.toolbar.layout().toolbar, vec![vec!["Brush".to_string()]]);
}

/// CodeRabbit on #14811: `prefs.reset` of one layout path works once `edit.toolbar` stored a custom
/// layout. The default layout used to be left out of the stored JSON, so reset found no default
/// for `toolbar.toolbar` or `toolbar.extras` and reported an unknown preference.
#[test]
fn prefs_reset_brings_back_one_part_of_a_custom_layout() {
    for path in ["toolbar.toolbar", "toolbar.extras"] {
        let mut app = app();
        app.run("edit.toolbar", json!({"toolbar": [["Zoom"], ["Hand"]], "extras": [["Move", "Artboard"]]})).unwrap();
        app.run("prefs.reset", json!({"path": path})).unwrap_or_else(|e| panic!("{path}: {e}"));
        let custom = ToolbarCustomization::default();
        let layout = app.session.prefs().toolbar.layout();
        if path == "toolbar.toolbar" {
            assert_ne!(layout.toolbar, vec![vec!["Zoom".to_string()], vec!["Hand".to_string()]], "{path}: back to the default groups");
            assert_eq!(app.session.prefs().toolbar.toolbar, custom.toolbar, "{path}: the stored layout is the default again");
        } else {
            assert_eq!(app.session.prefs().toolbar.extras, custom.extras, "{path}: the stored Extra Tools are the default again");
        }
    }
}
