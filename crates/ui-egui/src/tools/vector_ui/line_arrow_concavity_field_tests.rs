//! #13258: arrowhead Concavity takes whole numbers from -50 % to 50 % in the Properties panel and
//! the Line tool's gear menu; a value typed past the range becomes the closest end and, once the
//! field is left, the reference's "Closest value inserted" alert with OK says so.

use egui::accesskit::Role;
use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId, LiveShape};
use serde_json::json;

use super::{CONCAVITY_RANGE_ALERT, shape_properties};
use crate::PhotocraftApp;
use crate::state::{DialogKind, Tool};

/// An 800 x 600 document with a weight 4 End arrow (5 x 10 px head) from 400,300 to 600,300.
fn app_with_arrow() -> (PhotocraftApp, LayerId) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
    let made = app
        .run("shape.create", json!({"kind": "line", "from": [400, 300], "to": [600, 300], "weight": 4, "arrowEnd": true, "arrowWidth": 5, "arrowLength": 10}))
        .unwrap();
    let id = LayerId(made["layer"].as_u64().unwrap());
    (app, id)
}

/// True once `setup_context` installed the app's fonts (the first frame runs before it).
fn fonts_ready(ui: &egui::Ui) -> bool {
    ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into())))
}

fn properties() -> (Harness<'static, PhotocraftApp>, LayerId) {
    let (app, id) = app_with_arrow();
    let mut h = Harness::builder().with_size(vec2(400.0, 900.0)).build_ui_state(
        move |ui, app| {
            if fonts_ready(ui) {
                shape_properties(app, ui, id);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    (h, id)
}

/// The options bar with the Line tool active, its arrowhead gear menu open.
fn gear() -> Harness<'static, PhotocraftApp> {
    let (mut app, _) = app_with_arrow();
    app.ui.tool = Tool::Line;
    let mut h = Harness::builder().with_size(vec2(1800.0, 400.0)).build_ui_state(
        |ui, app| {
            if fonts_ready(ui) {
                crate::panels::options_bar(app, ui);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h.query_by_label("Arrowhead options").expect("the arrowhead gear").click();
    h.run_steps(3);
    h
}

/// Types `text` into the Concavity field, then Tabs out of it.
fn type_concavity(h: &mut Harness<'static, PhotocraftApp>, text: &str) {
    h.query_all_by_role(Role::SpinButton).find(|n| n.accesskit_node().label().as_deref() == Some("Concavity")).expect("the Concavity field").click();
    h.run_steps(1);
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::A);
    h.event(egui::Event::Text(text.into()));
    h.run_steps(1);
    h.key_press(egui::Key::Tab);
    h.run_steps(3);
}

fn alerts(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().ui.dialogs.iter().filter(|d| d.kind == DialogKind::Error).filter_map(|d| d.fields.get("message")?.as_str().map(str::to_owned)).collect()
}

fn concavity(h: &Harness<'static, PhotocraftApp>, id: LayerId) -> f64 {
    match h.state().session.active().and_then(|st| st.doc.layer(id)).map(|l| &l.content) {
        Some(LayerContent::Shape(sh)) => match sh.live {
            Some(LiveShape::ArrowLine { concavity, .. }) => concavity,
            _ => panic!("expected a live arrow"),
        },
        _ => panic!("expected a shape layer"),
    }
}

#[test]
fn properties_concavity_in_range_is_applied_without_an_alert() {
    for (typed, want) in [("-50", -50.0), ("-12", -12.0), ("25", 25.0), ("50", 50.0)] {
        let (mut h, id) = properties();
        type_concavity(&mut h, typed);
        assert_eq!(concavity(&h, id), want, "{typed}");
        assert!(alerts(&h).is_empty(), "{typed} is in range");
    }
}

#[test]
fn properties_concavity_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("-60", -50.0), ("60", 50.0)] {
        let (mut h, id) = properties();
        type_concavity(&mut h, typed);
        assert_eq!(concavity(&h, id), want, "{typed}");
        assert_eq!(alerts(&h), vec![CONCAVITY_RANGE_ALERT.to_string()], "{typed}: one alert");
    }
}

#[test]
fn gear_concavity_past_the_range_takes_the_closest_value_and_alerts() {
    for (typed, want) in [("-60", -50.0), ("60", 50.0), ("-20", -20.0)] {
        let mut h = gear();
        assert!(h.query_all_by_role(Role::SpinButton).any(|n| n.accesskit_node().label().as_deref() == Some("Concavity")), "the gear menu is open");
        type_concavity(&mut h, typed);
        assert_eq!(h.state().ui.tool_options.arrow_concavity, want, "{typed}");
        let expected: Vec<String> = if typed == "-20" { vec![] } else { vec![CONCAVITY_RANGE_ALERT.to_string()] };
        assert_eq!(alerts(&h), expected, "{typed}");
    }
}
