//! #12059: for vertical text the options bar's alignment buttons are Top / Center / Bottom, after
//! the targeted text layer's orientation, or with no text targeted the Vertical Type tool's. They
//! still set left / center / right, and the lit one follows the targeted paragraph. The Type Mask
//! tools keep the horizontal buttons (not measured).

use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use photocraft_doc::{LayerContent, LayerId, text::TextAlign};
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

const HORIZONTAL: [&str; 3] = ["Left align text", "Center text", "Right align text"];
const VERTICAL: [&str; 3] = ["Top align text", "Center text", "Bottom align text"];

fn new_app(tool: Tool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 800, "height": 400})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// A text layer with `orientation`, made the active layer (selected, not edited).
fn text_layer(app: &mut PhotocraftApp, orientation: &str) -> LayerId {
    let made = app.run("type.create", json!({"text": "AB CD", "x": 400, "y": 100, "orientation": orientation})).unwrap();
    LayerId(made["layer"].as_u64().unwrap())
}

fn bar(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 60.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            ui.horizontal(|ui| super::options_bar(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

/// Which of the two button sets the bar shows; panics on a mix or neither.
fn shown(h: &Harness<'static, PhotocraftApp>) -> [&'static str; 3] {
    let has = |label: &str| h.query_by_role_and_label(Role::Button, label).is_some();
    match (has("Top align text") && has("Bottom align text"), has("Left align text") && has("Right align text")) {
        (true, false) => VERTICAL,
        (false, true) => HORIZONTAL,
        (true, true) => panic!("both the vertical and the horizontal buttons"),
        (false, false) => panic!("no alignment buttons"),
    }
}

fn lit(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    h.get_by_role_and_label(Role::Button, label).accesskit_node().toggled() == Some(Toggled::True)
}

fn align_of(app: &PhotocraftApp, id: LayerId) -> TextAlign {
    match &app.session.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t.paragraphs[0].style.align,
        other => panic!("{}", other.kind_name()),
    }
}

#[test]
fn with_no_text_targeted_the_tool_decides() {
    for (tool, buttons) in [(Tool::VerticalType, VERTICAL), (Tool::Type, HORIZONTAL)] {
        let h = bar(new_app(tool));
        assert_eq!(shown(&h), buttons, "{tool:?}");
        assert!(lit(&h, buttons[0]), "{tool:?}: the default, left, lights the first button");
    }
}

#[test]
fn a_selected_text_layer_decides_whatever_the_tool() {
    for tool in [Tool::Type, Tool::VerticalType] {
        for (orientation, buttons) in [("vertical", VERTICAL), ("horizontal", HORIZONTAL)] {
            let mut app = new_app(tool);
            text_layer(&mut app, orientation);
            assert_eq!(shown(&bar(app)), buttons, "{tool:?}, {orientation} layer selected");
        }
    }
}

#[test]
fn text_being_edited_decides() {
    for (orientation, buttons) in [("vertical", VERTICAL), ("horizontal", HORIZONTAL)] {
        let mut app = new_app(Tool::Type);
        text_layer(&mut app, orientation);
        super::edit_active(&mut app).unwrap();
        assert!(app.ui.text_edit.is_some());
        assert_eq!(shown(&bar(app)), buttons, "editing {orientation} text");
    }
}

#[test]
fn the_vertical_buttons_set_left_center_and_right_and_follow_the_paragraph() {
    for (label, align, value) in
        [("Bottom align text", TextAlign::Right, "right"), ("Center text", TextAlign::Center, "center"), ("Top align text", TextAlign::Left, "left")]
    {
        let mut app = new_app(Tool::VerticalType);
        let id = text_layer(&mut app, "vertical");
        let mut h = bar(app);
        h.get_by_role_and_label(Role::Button, label).click();
        h.run_steps(2);
        assert_eq!(align_of(h.state(), id), align, "{label}");
        assert_eq!(h.state().ui.tool_options.type_align, value, "{label}: the tool default too");
        for other in VERTICAL {
            assert_eq!(lit(&h, other), other == label, "{label} clicked: {other}");
        }
    }
}

#[test]
fn the_type_mask_tools_keep_the_horizontal_buttons() {
    for tool in [Tool::VerticalTypeMask, Tool::HorizontalTypeMask] {
        assert_eq!(shown(&bar(new_app(tool))), HORIZONTAL, "{tool:?}, idle");
        let mut app = new_app(tool);
        super::pointer_up(&mut app, [400.0, 100.0], [400.0, 100.0]);
        assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.mask), "{tool:?}: typing a mask");
        assert_eq!(shown(&bar(app)), HORIZONTAL, "{tool:?}, editing");
    }
}

/// Clicking Toggle text orientation switches the buttons in the same frame (the toggle is drawn
/// before them).
#[test]
fn toggling_the_orientation_switches_the_buttons() {
    let mut app = new_app(Tool::Type);
    text_layer(&mut app, "horizontal");
    let mut h = bar(app);
    assert_eq!(shown(&h), HORIZONTAL);
    h.get_by_role_and_label(Role::Button, "Toggle text orientation").click();
    h.run_steps(1);
    assert_eq!(shown(&h), VERTICAL, "now vertical");
}

/// A text edit left in a background document decides by its own layer, not by a same-id layer of
/// the active document (layer ids repeat across documents; as `styles_at`, #9179).
#[test]
fn a_background_text_edit_decides_by_its_own_document() {
    let mut app = new_app(Tool::Type);
    let a = app.session.active().unwrap().doc.id.0;
    let id = text_layer(&mut app, "vertical");
    app.ui.text_edit = Some(crate::state::TextEdit {
        layer: id.0,
        document: Some(a),
        caret: 0,
        anchor: 0,
        session: "s".into(),
        created: false,
        mask: false,
        dragging: false,
        resize: None,
        preedit: None,
    });
    // The same file opened twice, its copy of the layer turned horizontal.
    let twin = app.session.active().unwrap().doc.as_ref().clone();
    app.session.open_document(twin, None);
    app.run("type.orientation.horizontal", json!({"layer": id.0})).unwrap();
    assert_ne!(app.session.active().unwrap().doc.id.0, a, "the other document is active");
    assert_eq!(shown(&bar(app)), VERTICAL, "the edited layer is vertical");
}

/// The lit button follows the targeted paragraph, not the tool default.
#[test]
fn the_lit_button_follows_the_paragraph_not_the_tool_default() {
    let mut app = new_app(Tool::VerticalType);
    app.ui.tool_options.type_align = "center".into();
    let made = app.run("type.create", json!({"text": "AB CD", "x": 400, "y": 100, "orientation": "vertical", "align": "right"})).unwrap();
    let id = LayerId(made["layer"].as_u64().unwrap());
    assert_eq!(align_of(&app, id), TextAlign::Right);
    let h = bar(app);
    for label in VERTICAL {
        assert_eq!(lit(&h, label), label == "Bottom align text", "{label}");
    }
}
