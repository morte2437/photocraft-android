//! #12628: the Art History Brush starts with Style Tight Short (`tightShort`), Reset Tool restores
//! it, `paint.artHistoryBrush` without `style` (or with `style: null`) paints Tight Short, and
//! saved settings with another style keep their stored value.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::state::{Tool, ToolOptions};

/// Build an app with a document and the Art History Brush selected.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 96, "height": 96, "background": "white"})).unwrap();
    app.ui.tool = Tool::ArtHistoryBrush;
    app
}

/// Render the options bar inside a kittest harness once fonts are loaded.
fn bar(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::panels::options_bar(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h
}

/// Whether a combo box in the options bar currently shows `value`.
fn has_combo_value(h: &Harness<'_, PhotocraftApp>, value: &str) -> bool {
    h.query_all_by_role(egui::accesskit::Role::ComboBox).any(|n| n.value().as_deref() == Some(value))
}

/// Paint one deterministic `paint.artHistoryBrush` stroke with `extra` merged into its parameters.
fn stroke_pixels(extra: Value) -> Vec<f32> {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    s.execute("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).unwrap();
    s.execute("edit.fill", json!({"contents": "foreground"})).unwrap();
    let mut p = json!({"points": [[8, 32], [56, 32]], "size": 12, "hardness": 100, "area": 50, "tolerance": 0});
    if let (Some(dst), Some(src)) = (p.as_object_mut(), extra.as_object()) {
        for (k, v) in src {
            dst.insert(k.clone(), v.clone());
        }
    }
    s.execute("paint.artHistoryBrush", p).unwrap();
    let surf = s.active().unwrap().doc.layers[0].surface().unwrap();
    let mut out = Vec::with_capacity(64 * 64 * 4);
    for y in 0..64 {
        for x in 0..64 {
            out.extend_from_slice(&surf.rgba(x, y));
        }
    }
    out
}

/// Fresh settings and `tool.reset` give `tightShort`, and the options bar Style field reads "Tight Short".
#[test]
fn fresh_settings_and_reset_tool_give_tight_short() {
    assert_eq!(ToolOptions::default().art_history_style, "tightShort");
    let a = app();
    assert_eq!(a.ui.tool_options.art_history_style, "tightShort");
    let mut h = bar(a);
    assert!(has_combo_value(&h, "Tight Short"), "options bar Style starts at Tight Short");

    h.state_mut().ui.tool_options.art_history_style = "looseCurl".into();
    h.run_steps(4);
    assert!(has_combo_value(&h, "Loose Curl"));

    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "tool.reset", json!({})).unwrap();
    h.run_steps(4);
    assert_eq!(h.state().ui.tool_options.art_history_style, "tightShort");
    assert!(has_combo_value(&h, "Tight Short"), "Reset Tool restores Style to Tight Short");
}

/// `paint.artHistoryBrush` without `style` (or with `style: null`) paints Tight Short, matching its params doc.
#[test]
fn paint_art_history_brush_defaults_to_tight_short() {
    let spec = photocraft_engine::command_specs().iter().find(|c| c.id == "paint.artHistoryBrush").unwrap();
    assert!(spec.params.contains(r#"="tightShort""#), "{}", spec.params);
    assert!(!spec.params.contains(r#"="tightMedium""#), "{}", spec.params);

    let omitted = stroke_pixels(json!({}));
    let null_style = stroke_pixels(json!({"style": null}));
    let tight_short = stroke_pixels(json!({"style": "tightShort"}));
    let tight_medium = stroke_pixels(json!({"style": "tightMedium"}));
    assert!(omitted == tight_short, "missing style should paint Tight Short");
    assert!(null_style == tight_short, "null style should paint Tight Short");
    assert!(omitted != tight_medium, "Tight Short and Tight Medium strokes should differ");
}

/// Loading saved settings that hold another style preserves it, while missing `art_history_style` defaults to `tightShort`.
#[test]
fn saved_settings_keep_stored_art_history_style() {
    let saved: ToolOptions = serde_json::from_value(json!({"art_history_style": "tightMedium"})).unwrap();
    assert_eq!(saved.art_history_style, "tightMedium");

    let missing: ToolOptions = serde_json::from_value(json!({})).unwrap();
    assert_eq!(missing.art_history_style, "tightShort");
}
