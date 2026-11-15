//! #9635: the dialogs that use the shared label column (Create Rectangle/Polygon, Fill with its
//! Blending section, Stroke, Channel Options, Layer Mask Display Options, New Spot Channel, New
//! Snapshot) draw every label inside the dialog in every language.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::state::Tool;

/// The widest these dialogs are laid out (`default_max_width` in dialogs.rs).
const ROOM_W: f32 = 440.0;

type Body = fn(&PhotocraftApp, &mut egui::Ui, &mut Map<String, Value>);

/// An app with a small document and a saved selection, so every dialog in [`cases`] can open.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 64, "height": 48, "background": "white"})).unwrap();
    app.run("select.rect", json!({"x": 2, "y": 2, "width": 8, "height": 8})).unwrap();
    app.run("select.saveSelection", json!({})).unwrap();
    app
}

/// The fields of the dialog `id` that an `open` just returned.
fn dialog_fields(app: &mut PhotocraftApp, id: Option<u64>) -> Map<String, Value> {
    app.ui.dialog_mut(id.unwrap()).unwrap().fields.clone()
}

/// Each dialog body with the fields it opens with, and the variants that show other labels.
fn cases(app: &mut PhotocraftApp) -> Vec<(String, Map<String, Value>, Body)> {
    let mut cases: Vec<(String, Map<String, Value>, Body)> = Vec::new();
    for tool in [Tool::Rectangle, Tool::Polygon] {
        let id = crate::shape_dialog::open(app, tool, [10.0, 20.0]);
        cases.push((format!("shape {tool:?}"), dialog_fields(app, id), crate::shape_dialog::body));
    }
    for contents in ["foreground", "color", "pattern", "contentAware"] {
        let mut f = crate::fill_ui::fields(app);
        f.insert("contents".into(), json!(contents));
        cases.push((format!("fill {contents}"), f, crate::fill_ui::body));
    }
    cases.push(("stroke".into(), crate::stroke_ui::fields(app), |_, ui, f| crate::stroke_ui::body(ui, f)));
    for channel in [json!(0), json!("quickMask"), json!("mask")] {
        let f = crate::channel_options_ui::fields(app, &channel).unwrap();
        cases.push((format!("channel options {channel}"), f, |_, ui, f| crate::channel_options_ui::body(ui, f)));
    }
    let f = crate::channel_options_ui::new_spot_fields(app).unwrap();
    cases.push(("new spot channel".into(), f, |_, ui, f| crate::channel_options_ui::new_spot_body(ui, f)));
    let id = crate::snapshot_dialog::open(app);
    cases.push(("new snapshot".into(), dialog_fields(app, id), |_, ui, f| crate::snapshot_dialog::body(ui, f)));
    cases
}

/// In every language (Arabic included: dialogs keep their left-to-right layout), at 100% and 200%
/// UI scale, no text a dialog body draws starts left of the dialog's edge, and the longest
/// translation doesn't push the fields out past its right edge.
#[test]
fn translated_labels_stay_inside_the_dialog() {
    let mut app = app();
    let cases = cases(&mut app);
    let ctx = egui::Context::default();
    crate::theme::install_fonts(&ctx);
    crate::theme::apply(&ctx, crate::theme::ThemeKind::ALL[0]);
    let mut failures = Vec::new();
    for ppp in [1.0, 2.0] {
        ctx.set_pixels_per_point(ppp);
        for lang in crate::i18n::Lang::all() {
            let _language = crate::i18n::language_scope(lang);
            for (name, fields, body) in &cases {
                let mut f = fields.clone();
                let mut room = egui::Rect::NOTHING;
                let mut used = egui::Rect::NOTHING;
                let mut out = ctx.run_ui(Default::default(), |ui| {
                    assert_eq!(ui.ctx().pixels_per_point(), ppp);
                    ui.set_max_width(ROOM_W);
                    room = ui.max_rect();
                    body(&app, ui, &mut f);
                    used = ui.min_rect();
                });
                out.textures_delta.clear();
                let texts: Vec<(String, egui::Rect)> = out
                    .shapes
                    .iter()
                    .filter_map(|s| match &s.shape {
                        egui::Shape::Text(t) => Some((t.galley.text().to_string(), s.shape.visual_bounding_rect())),
                        _ => None,
                    })
                    .collect();
                assert!(!texts.is_empty(), "{name} {} @{ppp}x: draws its labels", lang.code());
                for (text, r) in texts.iter().filter(|(_, r)| r.left() < room.left() - 0.5) {
                    failures.push(format!("{name} {} @{ppp}x: {text:?} at {r:?} starts left of the dialog's {room:?}", lang.code()));
                }
                if used.right() > room.left() + ROOM_W + 0.5 {
                    failures.push(format!("{name} {} @{ppp}x: the fields reach {used:?}, past the dialog's {ROOM_W} pt", lang.code()));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
