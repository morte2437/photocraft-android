use super::*;
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{Harness, kittest::Queryable};

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app
}

fn work_path(app: &mut PhotocraftApp) {
    app.run(
        "path.set",
        json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [
            {"anchor": [2, 2], "in": [2, 2], "out": [2, 2]},
            {"anchor": [20, 2], "in": [20, 2], "out": [20, 2]},
            {"anchor": [20, 20], "in": [20, 20], "out": [20, 20]}
        ]}]}}),
    )
    .unwrap();
}

fn choose_open(app: &mut PhotocraftApp, command: &str) {
    assert!(open(app, &Context::default(), Tool::Pen, [10.0, 10.0]));
    choose(app, &Context::default(), command);
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

fn right_click(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, modifiers: Modifiers) {
    h.event(egui::Event::ModifiersChanged(modifiers));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: true, modifiers });
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Secondary, pressed: false, modifiers });
    h.event(egui::Event::ModifiersChanged(Modifiers::NONE));
    h.run_steps(2);
}

#[test]
fn canvas_gesture_preserves_selection_and_command_override() {
    let mut app = app();
    app.ui.tool = Tool::Lasso;
    app.run("select.rect", json!({"x": 1, "y": 1, "width": 8, "height": 8})).unwrap();
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    let revision = h.state().session.active().unwrap().revision;
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_some());
    assert_eq!(h.state().session.active().unwrap().revision, revision, "right-click cannot alter selection");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none());
    right_click(&mut h, center, Modifiers::COMMAND);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "command right-click belongs to layer picker");
}

/// A type layer reading "Hello", being edited with every character selected.
fn editing_type(app: &mut PhotocraftApp) -> u64 {
    app.ui.tool = Tool::Type;
    let id = app.run("type.create", json!({"text": "Hello", "size": 12, "x": 4, "y": 20})).unwrap()["layer"].as_u64().unwrap();
    crate::menus::invoke(app, &Context::default(), "type.editText", json!({})).unwrap();
    id
}

fn text_of(app: &PhotocraftApp, id: u64) -> Option<photocraft_doc::TextLayer> {
    match &app.session.active().unwrap().doc.layer(photocraft_doc::LayerId(id)).unwrap().content {
        LayerContent::Text(t) => Some(t.clone()),
        _ => None,
    }
}

#[test]
fn right_click_while_typing_opens_the_type_edit_menu_and_esc_keeps_the_edit() {
    let mut app = app();
    editing_type(&mut app);
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            // The app's order: shortcuts (inline type's keys among them), then the canvas.
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.run_steps(4);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center, Modifiers::NONE);
    let menu = h.state().ui.canvas_tool_menu.clone().expect("right-click in a type edit opens a menu");
    assert!(menu.text && !menu.pasteboard && !menu.transform);
    assert_eq!(rows(&menu), TYPE_EDIT_MENU);
    let edit = h.state().ui.text_edit.clone().expect("the edit goes on under the menu");
    assert_eq!((edit.anchor, edit.caret), (0, 5), "the right-click keeps the selection for Cut and Copy");
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "Esc closes the menu");
    assert!(h.state().ui.text_edit.is_some(), "and only the menu: the edit isn't committed");
}

#[test]
fn type_edit_menu_rows_act_on_the_text_and_the_layer() {
    let mut app = app();
    let id = editing_type(&mut app);
    let ctx = Context::default();
    let reopen = |app: &mut PhotocraftApp| assert!(open(app, &Context::default(), Tool::Type, [10.0, 10.0]));
    // Faux Bold toggles the style of the selected characters, and shows checked.
    reopen(&mut app);
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(!mode_checked(&app, &menu, FAUX_BOLD) && mode_checked(&app, &menu, "type.antiAlias.smooth"));
    choose(&mut app, &ctx, FAUX_BOLD);
    assert!(text_of(&app, id).unwrap().char_runs().iter().all(|r| r.style.faux_bold));
    assert!(mode_checked(&app, &menu, FAUX_BOLD));
    // Cut takes the selection out of the text; with nothing selected Copy and Cut grey out.
    reopen(&mut app);
    choose(&mut app, &ctx, "edit.cut");
    assert_eq!(text_of(&app, id).unwrap().text, "");
    reopen(&mut app);
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(!entry_enabled(&app, &menu, "edit.copy") && !entry_enabled(&app, &menu, "edit.cut"));
    app.ui.canvas_tool_menu = None;
    crate::menus::invoke(&mut app, &ctx, "edit.undo", json!({})).unwrap();
    assert_eq!(text_of(&app, id).unwrap().text, "Hello");
    // Select All selects every character, not the canvas.
    reopen(&mut app);
    choose(&mut app, &ctx, "select.all");
    let edit = app.ui.text_edit.clone().unwrap();
    assert_eq!((edit.anchor, edit.caret), (0, 5));
    assert!(app.session.active().unwrap().doc.selection.is_none());
    // Warp Text… opens its dialog; Rasterize Type ends the edit and rasterizes the layer.
    reopen(&mut app);
    choose(&mut app, &ctx, "type.warpText");
    assert!(!app.ui.dialogs.is_empty(), "Warp Text… asks for its settings");
    app.ui.dialogs.clear();
    reopen(&mut app);
    choose(&mut app, &ctx, "type.rasterizeTypeLayer");
    assert!(app.ui.text_edit.is_none() && text_of(&app, id).is_none(), "the type layer is pixels now");
}

/// A committed type layer reading "Hello" over the middle of the canvas, no edit running.
fn committed_type(app: &mut PhotocraftApp) -> u64 {
    app.ui.tool = Tool::Type;
    let id = app.run("type.create", json!({"text": "Hello", "size": 14, "x": 1, "y": 22})).unwrap()["layer"].as_u64().unwrap();
    assert!(app.ui.text_edit.is_none());
    id
}

#[test]
fn right_click_on_committed_type_opens_the_type_layer_menu() {
    use photocraft_doc::text::Orientation;
    let mut app = app();
    let id = committed_type(&mut app);
    assert_eq!(crate::type_tool::hit_layer(&mut app, 16.0, 16.0).map(|l| l.0), Some(id), "the canvas centre is on the type");
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center, Modifiers::NONE);
    let menu = h.state().ui.canvas_tool_menu.clone().expect("right-click on committed type opens a menu (#7324)");
    assert!(menu.type_layer && !menu.text && !menu.pasteboard);
    assert_eq!(rows(&menu), TYPE_LAYER_MENU);
    assert!(h.state().ui.text_edit.is_none(), "the right-click doesn't start an edit");
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_some(), "the menu stays open without an edit");
    // An edit started some other way (here, as an agent's Type › Edit Text would) closes the
    // menu, which has no text edit rows.
    crate::menus::invoke(h.state_mut(), &Context::default(), "type.editText", json!({})).unwrap();
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "an edit closes the type layer menu");
    crate::type_tool::commit(h.state_mut());
    right_click(&mut h, center, Modifiers::NONE);
    let menu = h.state().ui.canvas_tool_menu.clone().expect("committed again, the menu opens again");
    assert!(menu.type_layer && mode_checked(h.state(), &menu, "type.orientation.horizontal"));
    choose(h.state_mut(), &Context::default(), "type.orientation.vertical");
    assert_eq!(text_of(h.state(), id).unwrap().orientation, Orientation::Vertical);
}

#[test]
fn type_layer_menu_targets_the_type_under_the_pointer() {
    let mut app = app();
    let id = committed_type(&mut app);
    app.run("layer.new.layer", json!({})).unwrap();
    let ctx = Context::default();
    let active = |app: &PhotocraftApp| app.session.active().unwrap().active_layer.map(|l| l.0);
    assert_ne!(active(&app), Some(id));
    // Away from the type, or with a Type Mask tool, nothing opens and nothing is selected.
    assert!(!open_at(&mut app, &ctx, Tool::Type, [10.0, 10.0], Some([30.0, 2.0])));
    assert!(!open_at(&mut app, &ctx, Tool::HorizontalTypeMask, [10.0, 10.0], Some([16.0, 16.0])));
    assert!(!open_at(&mut app, &ctx, Tool::Type, [10.0, 10.0], Some([f64::NAN, 16.0])));
    assert_ne!(active(&app), Some(id));
    // Over the type: its layer becomes the target, and the menu has no text edit rows.
    assert!(open_at(&mut app, &ctx, Tool::Type, [10.0, 10.0], Some([16.0, 16.0])));
    assert_eq!(active(&app), Some(id));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    for edit_row in ["edit.undo", "edit.cut", "edit.copy", "edit.paste", "select.all", FAUX_BOLD, FAUX_ITALIC] {
        assert!(!available(&app, &menu, edit_row), "{edit_row} belongs to the edit menu only");
    }
    assert!(entry_enabled(&app, &menu, "type.convertToParagraphText") && !entry_enabled(&app, &menu, "type.convertToPointText"));
    choose(&mut app, &ctx, "type.rasterizeTypeLayer");
    assert!(text_of(&app, id).is_none(), "the type layer is pixels now");
    assert!(app.ui.text_edit.is_none());
}

/// Two selected type layers: the one right-clicked becomes active, and the selection stays.
#[test]
fn type_layer_menu_in_a_multiselection_acts_on_the_clicked_type() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.ui.tool = Tool::Type;
    let top = app.run("type.create", json!({"text": "Top", "size": 30, "x": 10, "y": 40})).unwrap()["layer"].as_u64().unwrap();
    let bottom = app.run("type.create", json!({"text": "Low", "size": 30, "x": 10, "y": 90})).unwrap()["layer"].as_u64().unwrap();
    app.run("layer.select", json!({"layer": bottom})).unwrap();
    app.run("layer.select", json!({"layer": top, "mode": "add"})).unwrap();
    let at = [25.0, 80.0];
    assert_eq!(crate::type_tool::hit_layer(&mut app, at[0], at[1]).map(|l| l.0), Some(bottom));
    assert_eq!(app.session.active().unwrap().active_layer.map(|l| l.0), Some(top));
    let ctx = Context::default();
    assert!(open_at(&mut app, &ctx, Tool::Type, [10.0, 10.0], Some(at)));
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer.map(|l| l.0), Some(bottom), "the clicked type is the target");
    assert_eq!(st.selected_layers().len(), 2, "the multiselection stays");
    choose(&mut app, &ctx, "type.rasterizeTypeLayer");
    assert!(text_of(&app, bottom).is_none(), "the clicked type is rasterized");
    assert!(text_of(&app, top).is_some(), "not the other selected type");
}

#[test]
fn type_edit_menu_lists_spelling_suggestions_for_the_word_under_the_pointer() {
    let mut app = app();
    app.ui.tool = Tool::Type;
    let id = app.run("type.create", json!({"text": "Teh cat", "size": 12, "x": 4, "y": 20})).unwrap()["layer"].as_u64().unwrap();
    let ctx = Context::default();
    crate::menus::invoke(&mut app, &ctx, "type.editText", json!({})).unwrap();
    // Away from the misspelling (past the end of the line): no spelling rows.
    assert!(open_at(&mut app, &ctx, Tool::Type, [10.0, 10.0], Some([600.0, 15.0])));
    assert!(spelling_rows(app.ui.canvas_tool_menu.as_ref().unwrap()).is_empty());
    // Over "Teh": its suggestions head the menu, then Add to Dictionary.
    assert!(open_at(&mut app, &ctx, Tool::Type, [10.0, 10.0], Some([5.0, 15.0])));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    let rows = spelling_rows(&menu);
    assert!(rows.iter().any(|(l, _)| l == "The"), "{rows:?}");
    assert_eq!(rows.last().map(|(_, id)| id.as_str()), Some(SPELLING_ADD));
    assert!(rows.len() <= MAX_SUGGESTIONS + 1, "at most {MAX_SUGGESTIONS} suggestions and Add to Dictionary: {rows:?}");
    assert!(available(&app, &menu, &rows[0].1));
    let pick = rows.iter().find(|(l, _)| l == "The").unwrap().1.clone();
    choose(&mut app, &ctx, &pick);
    assert_eq!(text_of(&app, id).unwrap().text, "The cat");
    assert_eq!(app.ui.text_edit.as_ref().map(|e| (e.caret, e.anchor)), Some((3, 3)), "the caret lands after the replacement");
    // Add to Dictionary stops flagging a word.
    app.run("type.insertText", json!({"layer": id, "text": " Zqx"})).unwrap();
    assert!(open_at(&mut app, &ctx, Tool::Type, [10.0, 10.0], Some([600.0, 15.0])));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(menu.spelling.as_ref().is_some_and(|s| s.word == "Zqx"));
    choose(&mut app, &ctx, SPELLING_ADD);
    assert!(app.session.prefs().user_dictionary.iter().any(|w| w == "Zqx"));
}

#[test]
fn right_click_outside_the_image_picks_the_pasteboard_color() {
    let mut app = app();
    app.ui.tool = Tool::RectMarquee;
    let mut h = harness(app);
    let rect = h.state().last_canvas_rect;
    let outside = rect.min + vec2(5.0, 5.0);
    right_click(&mut h, rect.center(), Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| !m.pasteboard), "inside: the tool's own menu");
    h.state_mut().ui.canvas_tool_menu = None;
    right_click(&mut h, outside, Modifiers::NONE);
    let menu = h.state().ui.canvas_tool_menu.clone().unwrap();
    assert!(menu.pasteboard && rows(&menu) == PASTEBOARD_MENU, "outside: the pasteboard menu");
    let ctx = h.ctx.clone();
    choose(h.state_mut(), &ctx, "pasteboard.black");
    assert_eq!(crate::prefs_ui::pasteboard_color(h.state()), Some(egui::Color32::BLACK));
    // Select Custom Color… opens the Color Picker; OK makes its colour the custom pasteboard.
    right_click(&mut h, outside, Modifiers::NONE);
    choose(h.state_mut(), &ctx, PASTEBOARD_PICK);
    let mut f = h.state().ui.dialogs.last().unwrap().fields.clone();
    f.insert("color".into(), json!("#123456"));
    crate::color_picker_ui::confirm(h.state_mut(), &f).unwrap();
    assert_eq!(crate::prefs_ui::pasteboard_color(h.state()), Some(egui::Color32::from_rgb(0x12, 0x34, 0x56)));
    // A painting tool keeps its Brush Preset picker there.
    h.state_mut().ui.tool = Tool::Brush;
    h.run_steps(2);
    right_click(&mut h, outside, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_none() && h.state().ui.brush_picker.is_some());
}

/// #2832: right-click over an open Free Transform box: Photoshop's transform menu. Skew
/// switches the box's mode; Rotate and Flip turn the box about its reference point without
/// committing it; Content-Aware Scale and Puppet Warp are greyed.
#[test]
fn right_click_during_free_transform_switches_modes_rotates_and_flips_the_box() {
    let mut app = app();
    app.ui.tool = Tool::RectMarquee;
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.rect", json!({"x": 4, "y": 6, "width": 10, "height": 8})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.deselect", json!({})).unwrap();
    let mut h = harness(app);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(2);
    let center = h.state().last_canvas_rect.center();
    let (revision, journal) = (h.state().session.active().unwrap().revision, h.state().session.journal.len());
    right_click(&mut h, center, Modifiers::NONE);
    let menu = h.state().ui.canvas_tool_menu.clone().expect("the transform menu");
    assert!(menu.transform && rows(&menu) == TRANSFORM_MENU);
    for (id, on) in [("edit.contentAwareScale", false), ("edit.puppetWarp", false), ("edit.transform.rotate90Cw", true), ("edit.transform.flipVertical", true)]
    {
        assert_eq!(entry_enabled(h.state(), &menu, id), on, "{id}");
    }
    choose(h.state_mut(), &ctx, "edit.transform.skew");
    assert_eq!(h.state().ui.transform.as_ref().unwrap().mode, crate::state::TransformMode::Skew);
    let t = h.state().ui.transform.clone().unwrap();
    let (q0, c) = (t.quad, t.pivot);
    right_click(&mut h, center, Modifiers::NONE);
    choose(h.state_mut(), &ctx, "edit.transform.flipHorizontal");
    let q = h.state().ui.transform.as_ref().expect("the box stays open").quad;
    for (a, b) in q.iter().zip(q0) {
        assert!((a[0] - (2.0 * c[0] - b[0])).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9, "mirrored about the reference point: {q0:?} -> {q:?}");
    }
    right_click(&mut h, center, Modifiers::NONE);
    choose(h.state_mut(), &ctx, "edit.transform.rotate180");
    let q = h.state().ui.transform.as_ref().unwrap().quad;
    for (a, b) in q.iter().zip(q0) {
        assert!((a[1] - (2.0 * c[1] - b[1])).abs() < 1e-9 && (a[0] - b[0]).abs() < 1e-9, "flipped then turned 180°: {q0:?} -> {q:?}");
    }
    let st = h.state().session.active().unwrap();
    assert_eq!((st.revision, h.state().session.journal.len()), (revision, journal), "nothing is committed");
}

#[test]
fn transform_menu_follows_photoshop_order() {
    assert_eq!(
        labels(TRANSFORM_MENU),
        vec![
            Some("Free Transform"),
            Some("Scale"),
            Some("Rotate"),
            Some("Skew"),
            Some("Distort"),
            Some("Perspective"),
            Some("Warp"),
            None,
            Some("Content-Aware Scale"),
            Some("Puppet Warp"),
            None,
            Some("Rotate 180°"),
            Some("Rotate 90° Clockwise"),
            Some("Rotate 90° Counter Clockwise"),
            None,
            Some("Flip Horizontal"),
            Some("Flip Vertical"),
        ]
    );
    // Over a box on editable type, Distort and Perspective are greyed (#2630).
    let mut app = app();
    app.run("type.create", json!({"x": 2, "y": 20, "text": "Hi", "size": 16})).unwrap();
    crate::transform_tool::begin(&mut app, &Context::default()).unwrap();
    assert!(open_transform(&mut app, &Context::default(), [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    for (id, on) in [("edit.transform.skew", true), ("edit.transform.distort", false), ("edit.transform.perspective", false)] {
        assert_eq!(entry_enabled(&app, &menu, id), on, "{id}");
    }
}

fn labels(rows: &[Row]) -> Vec<Option<&str>> {
    rows.iter().map(|row| row.map(|(label, _)| label)).collect()
}

#[test]
fn selection_menus_follow_photoshop_order() {
    assert_eq!(
        labels(SELECTION_MENU),
        vec![
            Some("Deselect"),
            Some("Select Inverse"),
            Some("Feather…"),
            Some("Select and Mask…"),
            None,
            Some("Save Selection…"),
            Some("Make Work Path…"),
            None,
            Some("Layer via Copy"),
            Some("Layer via Cut"),
            Some("New Layer…"),
            None,
            Some("Free Transform"),
            Some("Transform Selection"),
            Some("Distort"),
            Some("Perspective"),
            None,
            Some("Delete and Fill Selection"),
            Some("Fill…"),
            Some("Stroke…"),
            Some("Content-Aware Fill…"),
            None,
            Some("Last Filter"),
            None,
            Some("Fade…"),
        ]
    );
    assert_eq!(labels(NO_SELECTION_MENU), vec![Some("Select All"), Some("Reselect"), Some("Color Range…"), None, Some("Load Selection…")]);
    // Every row runs a real command: one the menu bar has, or an engine command.
    let app = app();
    let items = crate::menus::menu_items(&app);
    for (label, id) in [SELECTION_MENU, NO_SELECTION_MENU, PEN_MENU, TRANSFORM_MENU].into_iter().flatten().flatten() {
        assert!(items.iter().any(|i| i.id == *id) || photocraft_engine::commands::find(id).is_some(), "{label}: unknown command {id}");
    }
}

#[test]
fn selection_menu_routes_actions_through_commands() {
    let mut app = app();
    app.ui.tool = Tool::RectMarquee;
    app.run("select.rect", json!({"x": 1, "y": 1, "width": 8, "height": 8})).unwrap();
    assert!(open(&mut app, &Context::default(), Tool::RectMarquee, [10.0, 10.0]));
    assert_eq!(rows(app.ui.canvas_tool_menu.as_ref().unwrap()), SELECTION_MENU);
    let before = app.session.journal.len();
    choose(&mut app, &Context::default(), "select.inverse");
    assert!(app.ui.canvas_tool_menu.is_none());
    assert!(app.session.journal[before..].iter().any(|(id, _)| id == "select.inverse"));
    let before = app.session.journal.len();
    assert!(open(&mut app, &Context::default(), Tool::RectMarquee, [10.0, 10.0]));
    choose(&mut app, &Context::default(), "select.deselect");
    assert!(app.session.active().unwrap().doc.selection.is_none());
    assert!(app.session.journal[before..].iter().any(|(id, _)| id == "select.deselect"));
    // Nothing selected: the menu that makes a selection.
    assert!(open(&mut app, &Context::default(), Tool::RectMarquee, [10.0, 10.0]));
    assert_eq!(rows(app.ui.canvas_tool_menu.as_ref().unwrap()), NO_SELECTION_MENU);
    choose(&mut app, &Context::default(), "select.reselect");
    assert!(app.session.active().unwrap().doc.selection.is_some());
    assert!(app.session.journal.iter().any(|(id, _)| id == "select.reselect"));
    app.run("select.deselect", json!({})).unwrap();
    assert!(open(&mut app, &Context::default(), Tool::Lasso, [10.0, 10.0]));
    choose(&mut app, &Context::default(), "select.all");
    assert!(app.session.active().unwrap().doc.selection.is_some());
}

#[test]
fn selection_menu_layer_and_path_actions() {
    let mut app = app();
    app.ui.tool = Tool::EllipseMarquee;
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 10})).unwrap();
    let layers = app.session.active().unwrap().doc.layers.len();
    assert!(open(&mut app, &Context::default(), Tool::EllipseMarquee, [10.0, 10.0]));
    choose(&mut app, &Context::default(), "layer.new.layerViaCopy");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers + 1, "Layer via Copy adds a layer");
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("layer.new.layerViaCopy"));
    // Make Work Path… asks for the tolerance, then traces the selection.
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 10, "height": 10})).unwrap();
    assert!(open(&mut app, &Context::default(), Tool::EllipseMarquee, [10.0, 10.0]));
    choose(&mut app, &Context::default(), "select.toWorkPath");
    let dialog = app.ui.dialogs.last().map(|d| d.id).expect("tolerance dialog");
    assert!(app.session.active().unwrap().doc.work_path.is_none(), "nothing happens before OK");
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("select.toWorkPath"));
}

#[test]
fn disabled_selection_rows_do_nothing() {
    let mut app = app();
    app.ui.tool = Tool::MagicWand;
    app.run("select.rect", json!({"x": 1, "y": 1, "width": 8, "height": 8})).unwrap();
    assert!(open(&mut app, &Context::default(), Tool::MagicWand, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    // No filter has run and nothing can be faded yet.
    for id in ["filter.lastFilter", "edit.fade"] {
        assert!(!entry_enabled(&app, &menu, id), "{id} is greyed out");
    }
    let before = app.session.journal.len();
    choose(&mut app, &Context::default(), "edit.fade");
    assert_eq!(app.session.journal.len(), before);
    assert!(app.ui.dialogs.is_empty());
    // Rows of the other variant are not part of this menu.
    assert!(open(&mut app, &Context::default(), Tool::MagicWand, [10.0, 10.0]));
    choose(&mut app, &Context::default(), "select.all");
    assert_eq!(app.session.journal.len(), before);
}

/// Photoshop's Zoom tool menu is Fit on Screen, 100%, 200%, Print Size, then Zoom In and Zoom
/// Out after a separator (#6005); Fill Screen (the Hand tool's) keeps its place by Fit on Screen.
#[test]
fn view_menu_follows_photoshop_order() {
    assert_eq!(
        labels(VIEW_MENU),
        vec![Some("Fit on Screen"), Some("Fill Screen"), Some("100%"), Some("200%"), Some("Print Size"), None, Some("Zoom In"), Some("Zoom Out")]
    );
    // Every row but Fill Screen (the Hand tool's options-bar button) is a View menu command.
    let app = app();
    let items = crate::menus::menu_items(&app);
    for (label, id) in VIEW_MENU.iter().flatten().filter(|(_, id)| *id != FILL_SCREEN) {
        assert!(items.iter().any(|i| i.id == *id), "{label}: unknown command {id}");
    }
}

fn zoom_app(w: u32, h: u32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": w, "height": h})).unwrap();
    app
}

fn screen_of(app: &PhotocraftApp, doc: [f32; 2]) -> Pos2 {
    crate::canvas::ViewXform::active(app).unwrap().to_screen(doc[0], doc[1])
}

#[test]
fn zoom_and_hand_right_click_opens_the_view_menu() {
    for tool in [Tool::Zoom, Tool::Hand] {
        let mut app = zoom_app(400, 300);
        app.ui.tool = tool;
        let mut h = harness(app);
        let zoom = h.state().ui.views[0].zoom;
        let inside = screen_of(h.state(), [300.0, 100.0]);
        right_click(&mut h, inside, Modifiers::NONE);
        let menu = h.state().ui.canvas_tool_menu.clone().unwrap_or_else(|| panic!("{tool:?}: right-click opens no menu"));
        assert!(rows(&menu) == VIEW_MENU, "{tool:?}: the view menu");
        assert_eq!(h.state().ui.views[0].zoom, zoom, "{tool:?}: the right-click alone does not zoom");
        assert!(entry_enabled(h.state(), &menu, FILL_SCREEN) && entry_enabled(h.state(), &menu, "view.printSize"));
        // Outside the image the pasteboard colour menu wins, as for the other tools.
        h.state_mut().ui.canvas_tool_menu = None;
        let outside = h.state().last_canvas_rect.min + vec2(5.0, 5.0);
        right_click(&mut h, outside, Modifiers::NONE);
        assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.pasteboard), "{tool:?}: pasteboard menu outside");
    }
}

#[test]
fn view_menu_zoom_in_keeps_the_clicked_point_under_the_pointer() {
    let mut app = zoom_app(400, 300);
    app.ui.tool = Tool::Zoom;
    let mut h = harness(app);
    let zoom = h.state().ui.views[0].zoom;
    let doc = [300.0, 100.0];
    let p = screen_of(h.state(), doc);
    right_click(&mut h, p, Modifiers::NONE);
    // Chosen in the menu itself: the canvas frame that draws the menu must keep the new view.
    h.get_by_label("Zoom In").click();
    h.run_steps(3);
    assert!(h.state().ui.canvas_tool_menu.is_none());
    let after = h.state().ui.views[0].zoom;
    assert_eq!(after, crate::zoom_levels::step(zoom, 1, [400, 300]), "one Zoom In step");
    let s = screen_of(h.state(), doc);
    assert!((s - p).length() < 1.0, "the right-clicked point stays put: {s:?} vs {p:?}");
    assert_eq!(h.state().session.journal.last().map(|(id, _)| id.as_str()), Some("view.zoomIn"));
    // Zoom Out returns to where it was.
    right_click(&mut h, p, Modifiers::NONE);
    h.get_by_label("Zoom Out").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.views[0].zoom, zoom);
    assert!((screen_of(h.state(), doc) - p).length() < 1.0);
}

#[test]
fn animated_zoom_off_view_menu_keeps_the_anchor_without_a_transition() {
    for command in ["view.zoomIn", "view.zoomOut", FILL_SCREEN] {
        let mut app = zoom_app(400, 300);
        app.ui.tool = Tool::Zoom;
        app.run("prefs.set", json!({"values": {"general.animatedZoom": false, "tools.zoomClickedPointToCenter": false}})).unwrap();
        let mut h = harness(app);
        let doc = [300.0, 100.0];
        let p = screen_of(h.state(), doc);
        right_click(&mut h, p, Modifiers::NONE);
        let ctx = h.ctx.clone();
        choose(h.state_mut(), &ctx, command);
        assert!(!h.state().ui.views[0].zoom_animating, "{command}");
        h.step();
        let v = &h.state().ui.views[0];
        let xf = crate::canvas::ViewXform::active(h.state()).unwrap();
        assert_eq!((xf.zoom, xf.center), (v.zoom, v.center), "{command}");
        assert!(v.zoom_anchor.is_none() && v.zoom_transition.is_none(), "{command}");
        if command != FILL_SCREEN {
            assert!(screen_of(h.state(), doc).distance(p) < 1.0, "{command}: preserve the clicked point");
        }
    }
}

#[test]
fn view_menu_zoom_keeps_its_anchor_during_the_animation() {
    let mut app = zoom_app(400, 300);
    app.ui.tool = Tool::Zoom;
    let mut h = harness(app);
    let zoom = h.state().ui.views[0].zoom;
    let doc = [300.0, 100.0];
    let p = screen_of(h.state(), doc);
    right_click(&mut h, p, Modifiers::NONE);
    let ctx = h.ctx.clone();
    // Choose without the widget click helper, which runs repaint frames until settled.
    choose(h.state_mut(), &ctx, "view.zoomIn");
    h.step();
    h.input_mut().time = Some(ctx.input(|i| i.time) + 0.11);
    h.step();
    let displayed = crate::canvas::ViewXform::active(h.state()).unwrap();
    assert!(displayed.zoom > zoom && displayed.zoom < h.state().ui.views[0].zoom);
    assert!(displayed.to_screen(doc[0], doc[1]).distance(p) < 1.0);
    h.input_mut().time = Some(ctx.input(|i| i.time) + 0.3);
    h.step();
    assert!(!h.state().ui.views[0].zoom_animating);
}

#[test]
fn view_menu_zoom_in_centres_the_clicked_point_with_the_preference() {
    let mut app = zoom_app(400, 300);
    app.ui.tool = Tool::Hand;
    app.run("prefs.set", json!({"path": "tools.zoomClickedPointToCenter", "value": true})).unwrap();
    let mut h = harness(app);
    let doc = [300.0, 100.0];
    let p = screen_of(h.state(), doc);
    right_click(&mut h, p, Modifiers::NONE);
    h.get_by_label("Zoom In").click();
    h.run_steps(3);
    assert_eq!(h.state().ui.views[0].center, doc);
}

#[test]
fn view_menu_fits_fills_and_sets_fixed_zooms() {
    let mut app = zoom_app(400, 300);
    app.ui.tool = Tool::Zoom;
    let mut h = harness(app);
    let p = screen_of(h.state(), [200.0, 150.0]);
    let area = crate::rulers::content_rect(h.state(), h.state().last_canvas_rect).size();
    let ppp = h.state().canvas_ppp();
    let pick = |h: &mut Harness<'static, PhotocraftApp>, label: &str| {
        right_click(h, p, Modifiers::NONE);
        h.get_by_label(label).click();
        h.run_steps(3);
        h.state().ui.views[0].zoom
    };
    assert_eq!(pick(&mut h, "200%"), 2.0);
    // Fill Screen: the image covers the whole canvas (one axis can overflow).
    let fill = pick(&mut h, "Fill Screen") / ppp;
    assert!(400.0 * fill >= area.x - 1.0 && 300.0 * fill >= area.y - 1.0, "fill {fill} for {area:?}");
    assert!(400.0 * fill <= area.x + 1.0 || 300.0 * fill <= area.y + 1.0, "fill {fill} for {area:?}");
    assert_eq!(h.state().ui.views[0].center, [200.0, 150.0]);
    // Fit on Screen: back to the whole image (capped at 100%, as on open).
    let fit = pick(&mut h, "Fit on Screen") / ppp;
    assert!(400.0 * fit <= area.x && 300.0 * fit <= area.y && fit < fill, "fit {fit} fill {fill}");
}

#[test]
fn view_menu_needs_finite_points_and_a_view_tool() {
    let mut app = app();
    assert!(!open_view(&mut app, &Context::default(), Tool::Brush, [1.0, 1.0], [1.0, 1.0]));
    assert!(!open_view(&mut app, &Context::default(), Tool::Zoom, [f32::NAN, 1.0], [1.0, 1.0]));
    assert!(!open_view(&mut app, &Context::default(), Tool::Zoom, [1.0, 1.0], [f64::INFINITY, 1.0]));
    assert!(app.ui.canvas_tool_menu.is_none());
    assert!(open_view(&mut app, &Context::default(), Tool::Zoom, [1.0, 1.0], [1.0, 1.0]));
    let before = app.session.journal.len();
    choose(&mut app, &Context::default(), "select.all");
    assert_eq!(app.session.journal.len(), before, "the view menu cannot run another menu's row");
    // With no document every row is greyed out and choosing one does nothing.
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    assert!(open_view(&mut app, &Context::default(), Tool::Hand, [1.0, 1.0], [1.0, 1.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    for (_, id) in VIEW_MENU.iter().flatten() {
        assert!(!entry_enabled(&app, &menu, id), "{id} is greyed out with no document");
    }
    choose(&mut app, &Context::default(), FILL_SCREEN);
    choose(&mut app, &Context::default(), "view.zoomIn");
}

#[test]
fn unsupported_tools_and_nonfinite_positions_do_not_open_menu() {
    let mut app = app();
    assert!(!open(&mut app, &Context::default(), Tool::Brush, [1.0, 1.0]));
    assert!(!open(&mut app, &Context::default(), Tool::Lasso, [f32::NAN, 1.0]));
    assert!(app.ui.canvas_tool_menu.is_none());
    assert!(open(&mut app, &Context::default(), Tool::MagicWand, [1.0, 1.0]));
    let before = app.session.journal.len();
    choose(&mut app, &Context::default(), "file.new");
    assert_eq!(app.session.journal.len(), before, "context menu cannot invoke an unrelated command");
}

#[test]
fn completed_pen_path_right_click_makes_selection() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    for (x, y) in [(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)] {
        crate::vector_ui::pen_down(&mut app, x, y, false);
        crate::vector_ui::pen_up(&mut app);
    }
    crate::vector_ui::pen_commit(&mut app, true);
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.tool == Tool::Pen && m.has_path));
    let ctx = h.ctx.clone();
    choose(h.state_mut(), &ctx, "path.toSelection");
    let dialog = h.state().ui.dialogs.last().map(|d| d.id).unwrap();
    crate::dialogs::confirm(h.state_mut(), dialog).unwrap();
    assert!(h.state().session.active().unwrap().doc.selection.is_some());
    assert_eq!(h.state().session.journal.last().map(|(id, _)| id.as_str()), Some("path.toSelection"));
    // Like ⌘↩, it deselects and hides the work path, which stays in the document (#6079).
    assert_eq!(h.state().ui.selected_path, None);
    assert!(h.state().session.active().unwrap().doc.work_path.is_some());
    assert!(!crate::vector_ui::work_path_shown(h.state()));
    // The hidden work path is no longer the active path: a second right-click offers no path
    // commands, so Delete Path can't remove it unseen.
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.tool == Tool::Pen && !m.has_path && m.path_name.is_none()));
    choose(h.state_mut(), &ctx, "path.delete");
    assert!(h.state().session.active().unwrap().doc.work_path.is_some(), "the hidden work path survives Delete Path");
}

#[test]
fn pen_menu_finishes_in_progress_path_before_selection() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    for (x, y) in [(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)] {
        crate::vector_ui::pen_down(&mut app, x, y, false);
        crate::vector_ui::pen_up(&mut app);
    }
    assert!(app.session.active().unwrap().doc.work_path.is_none());
    assert!(open(&mut app, &Context::default(), Tool::Pen, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.as_ref().unwrap();
    assert!(entry_enabled(&app, menu, "path.toSelection"));
    choose(&mut app, &Context::default(), "path.toSelection");
    let dialog = app.ui.dialogs.last().map(|d| d.id).unwrap();
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert!(app.session.active().unwrap().doc.work_path.is_some());
    assert!(app.session.active().unwrap().doc.selection.is_some());
}

/// #5703: the Pen options bar's Make Selection (Path mode) opens the same dialog for the path
/// being drawn (finished into the Work Path first), and OK selects inside it; with no path at
/// all it reports that instead.
#[test]
fn pen_options_bar_makes_a_selection_from_the_path() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    assert!(make_selection(&mut app).is_err(), "no path yet");
    assert!(app.ui.dialogs.is_empty());
    for (x, y) in [(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)] {
        crate::vector_ui::pen_down(&mut app, x, y, false);
        crate::vector_ui::pen_up(&mut app);
    }
    make_selection(&mut app).unwrap();
    let dialog = app.ui.dialogs.last().map(|d| d.id).unwrap();
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    let st = app.session.active().unwrap();
    assert!(st.doc.work_path.is_some() && st.doc.selection.is_some());
    // That hid the Work Path (#6079): it is not the active path until it is selected in the
    // Paths panel again, and then it is, with no pen path in progress.
    app.run("select.deselect", json!({})).unwrap();
    assert!(make_selection(&mut app).is_err(), "the hidden Work Path is not the active path");
    assert!(app.ui.dialogs.is_empty());
    app.ui.selected_path = Some("work".into());
    make_selection(&mut app).unwrap();
    let dialog = app.ui.dialogs.last().map(|d| d.id).unwrap();
    crate::dialogs::confirm(&mut app, dialog).unwrap();
    assert!(app.session.active().unwrap().doc.selection.is_some());
    // Drawn with a saved path selected, the path is added to that one (#4721): the dialog is
    // for it, not for a Work Path.
    app.run("path.rename", json!({"name": "work", "to": "Cutout"})).unwrap();
    app.ui.selected_path = Some("Cutout".into());
    for (x, y) in [(4.0, 24.0), (12.0, 24.0), (12.0, 30.0)] {
        crate::vector_ui::pen_down(&mut app, x, y, false);
        crate::vector_ui::pen_up(&mut app);
    }
    make_selection(&mut app).unwrap();
    assert_eq!(app.ui.dialogs.last().and_then(|d| d.fields.get("name")), Some(&json!("Cutout")));
}

/// #1056: the Path Selection and Direct Selection tools' right-click opens the path menu for
/// the selected path, as the Pen's does, instead of nothing.
#[test]
fn path_selection_tools_open_the_path_menu() {
    for tool in [Tool::PathSelection, Tool::DirectSelection] {
        let mut app = app();
        app.ui.tool = Tool::Pen;
        for (x, y) in [(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)] {
            crate::vector_ui::pen_down(&mut app, x, y, false);
            crate::vector_ui::pen_up(&mut app);
        }
        crate::vector_ui::pen_commit(&mut app, false);
        assert!(app.session.active().unwrap().doc.work_path.is_some());
        app.ui.tool = tool;
        assert!(applies(tool));
        assert!(open(&mut app, &Context::default(), tool, [10.0, 10.0]), "{tool:?}");
        let menu = app.ui.canvas_tool_menu.clone().unwrap();
        assert_eq!(rows(&menu), PEN_MENU, "{tool:?}");
        assert!(menu.has_path && entry_enabled(&app, &menu, "path.toSelection"), "{tool:?}");
        choose(&mut app, &Context::default(), "path.toSelection");
        let dialog = app.ui.dialogs.last().map(|d| d.id).unwrap();
        crate::dialogs::confirm(&mut app, dialog).unwrap();
        assert!(app.session.active().unwrap().doc.selection.is_some(), "{tool:?}: Make Selection made one");
    }
}

#[test]
fn pen_menu_has_all_reference_rows_in_order_and_guards_unavailable_actions() {
    let labels: Vec<_> = PEN_MENU.iter().map(|row| row.map(|(label, _)| label)).collect();
    assert_eq!(
        labels,
        vec![
            Some("Create Vector Mask"),
            Some("Delete Path"),
            None,
            Some("Define Custom Shape…"),
            None,
            Some("Make Selection…"),
            Some("New Guides From Shape"),
            Some("Fill Path…"),
            Some("Stroke Path…"),
            None,
            Some("Clipping Path…"),
            None,
            Some("Free Transform Path"),
            None,
            Some("Unite Shapes"),
            Some("Subtract Front Shape"),
            Some("Unite Shapes at Overlap"),
            Some("Subtract Shapes at Overlap"),
            None,
            Some("Copy Fill"),
            Some("Copy Complete Stroke"),
            None,
            Some("Paste Fill"),
            Some("Paste Complete Stroke"),
            None,
            Some("Isolate Layers"),
            None,
            Some("Make Symmetry Path"),
            Some("Disable Symmetry Path"),
        ]
    );
    let mut app = app();
    app.ui.tool = Tool::Pen;
    assert!(open(&mut app, &Context::default(), Tool::Pen, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.as_ref().unwrap();
    assert!(!entry_enabled(&app, menu, "path.toSelection"));
    let before = app.session.journal.len();
    choose(&mut app, &Context::default(), "path.toSelection");
    assert_eq!(app.session.journal.len(), before);
    assert!(app.ui.dialogs.is_empty());
}

#[test]
fn pen_menu_creates_mask_deletes_path_and_opens_named_path_dialogs() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    work_path(&mut app);
    choose_open(&mut app, "layer.vectorMask.fromPath");
    let st = app.session.active().unwrap();
    assert!(st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| l.vector_mask.is_some()));
    // The new vector mask's path becomes the active one (#4728); pick the Work Path again.
    assert_eq!(app.ui.selected_path.as_deref(), Some("layer"));
    app.ui.selected_path = Some("work".into());
    choose_open(&mut app, "edit.defineCustomShape");
    let id = app.ui.dialogs.last().map(|d| d.id).unwrap();
    let fields = &app.ui.dialogs.last().unwrap().fields;
    assert_eq!(fields.get("path"), Some(&json!("work")));
    app.ui.dialog_mut(id).unwrap().fields.insert("name".into(), json!("Menu Shape"));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert!(app.session.edit_state.custom_shapes.iter().any(|s| s.name == "Menu Shape"));
    choose_open(&mut app, "path.fill");
    let id = app.ui.dialogs.last().map(|d| d.id).unwrap();
    assert_eq!(app.ui.dialogs.last().unwrap().fields.get("name"), Some(&json!("work")));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("path.fill"));
    choose_open(&mut app, "path.delete");
    assert!(app.session.active().unwrap().doc.work_path.is_none());
}

#[test]
fn pen_menu_saved_path_clipping_and_numeric_transform() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    work_path(&mut app);
    app.run("path.rename", json!({"name": "work", "to": "Cutout"})).unwrap();
    app.ui.selected_path = Some("Cutout".into());
    assert!(open(&mut app, &Context::default(), Tool::Pen, [10.0, 10.0]));
    assert!(entry_enabled(&app, app.ui.canvas_tool_menu.as_ref().unwrap(), "path.clippingPath.set"));
    choose(&mut app, &Context::default(), "path.clippingPath.set");
    let id = app.ui.dialogs.last().map(|d| d.id).unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("flatness".into(), json!(2.0));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.active().unwrap().doc.clipping_path.as_ref().map(|p| p.name.as_str()), Some("Cutout"));
    choose_open(&mut app, "path.transform");
    let id = app.ui.dialogs.last().map(|d| d.id).unwrap();
    app.ui.dialog_mut(id).unwrap().fields.insert("translateX".into(), json!(5.0));
    crate::dialogs::confirm(&mut app, id).unwrap();
    assert_eq!(app.session.journal.last().map(|(id, _)| id.as_str()), Some("path.transform"));
}

#[test]
fn pen_shape_menu_enables_shape_ops_and_style_copy_only_when_applicable() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.run(
        "shape.create",
        json!({"kind":"path", "path":{"subpaths":[
        {"closed":true,"knots":[[2,2],[20,2],[20,20]]},
        {"closed":true,"knots":[[5,5],[10,5],[10,10]]}
    ]}, "fill":"#ff0000", "stroke":{"width":2,"color":"#000000"}}),
    )
    .unwrap();
    assert!(open(&mut app, &Context::default(), Tool::Pen, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.as_ref().unwrap();
    assert!(entry_enabled(&app, menu, "layer.combineShapes.unite"));
    assert!(entry_enabled(&app, menu, "path.style.copyFill"));
    assert!(!entry_enabled(&app, menu, "path.style.pasteFill"));
    assert!(!entry_enabled(&app, menu, "path.fill"));
    choose(&mut app, &Context::default(), "path.style.copyFill");
    assert!(app.session.path_fill_clipboard.is_some());
    assert!(open(&mut app, &Context::default(), Tool::Pen, [10.0, 10.0]));
    assert!(entry_enabled(&app, app.ui.canvas_tool_menu.as_ref().unwrap(), "path.style.pasteFill"));
}

#[test]
fn unfinished_shape_path_disables_actions_that_would_target_the_previous_layer() {
    let mut app = app();
    app.ui.tool = Tool::Pen;
    app.ui.tool_options.vector_mode = "shape".into();
    for (x, y) in [(2.0, 2.0), (20.0, 2.0), (20.0, 20.0)] {
        crate::vector_ui::pen_down(&mut app, x, y, false);
        crate::vector_ui::pen_up(&mut app);
    }
    assert!(open(&mut app, &Context::default(), Tool::Pen, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.as_ref().unwrap();
    for id in ["layer.vectorMask.fromPath", "path.delete", "path.fill", "path.stroke", "path.clippingPath.set", "view.newGuidesFromShape"] {
        assert!(!entry_enabled(&app, menu, id), "{id} must not act on the layer replaced by the pending shape");
    }
    assert!(entry_enabled(&app, menu, "path.toSelection"));
}

/// A 32 × 32 document with the Crop tool picked (Classic Mode, so the view stays put) and its
/// untouched default frame.
fn crop_app() -> PhotocraftApp {
    let mut app = app();
    app.ui.tool = Tool::Crop;
    app.ui.tool_options.crop_shield.classic_mode = true;
    app.ui.extras.snap = false;
    crate::crop_ui::ensure_frame(&mut app);
    app
}

/// Draws a crop frame from (4, 4) to (28, 28): a pending crop. Returns the frame.
fn draw_crop(app: &mut PhotocraftApp) -> [f64; 4] {
    use crate::canvas::{ToolEvent, tool_event};
    tool_event(app, ToolEvent::Down { x: 4.0, y: 4.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 16.0, y: 16.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Move { x: 28.0, y: 28.0, pressure: 1.0 }, Modifiers::NONE);
    tool_event(app, ToolEvent::Up { x: 28.0, y: 28.0 }, Modifiers::NONE);
    assert!(crate::crop_ui::pending(app), "the drag draws a pending frame");
    app.ui.crop_rect.unwrap()
}

fn aspect(r: [f64; 4]) -> f64 {
    (r[2] - r[0]) / (r[3] - r[1])
}

/// #6561: right-click on a pending crop opens its menu without touching the frame, and a ratio
/// row refits the frame as the options bar's dropdown does.
#[test]
fn right_click_on_a_pending_crop_opens_the_crop_menu_and_a_ratio_refits_the_frame() {
    let mut app = crop_app();
    let frame = draw_crop(&mut app);
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center, Modifiers::NONE);
    let menu = h.state().ui.canvas_tool_menu.clone().expect("right-click on a pending crop opens a menu");
    assert_eq!(h.state().ui.crop_rect, Some(frame), "the right-click leaves the frame alone");
    assert!(available(h.state(), &menu, "crop.ratio.16:9"), "the menu lists the options bar's ratios");
    let ctx = h.ctx.clone();
    choose(h.state_mut(), &ctx, "crop.ratio.16:9");
    assert_eq!(h.state().ui.tool_options.crop_ratio, "16:9");
    let r = h.state().ui.crop_rect.unwrap();
    assert!((aspect(r) - 16.0 / 9.0).abs() < 1e-9, "the frame is 16 : 9 now: {r:?}");
    assert!(r[0] >= frame[0] && r[1] >= frame[1] && r[2] <= frame[2] && r[3] <= frame[3], "fitted inside the old frame: {r:?}");
    assert!(h.state().ui.canvas_tool_menu.is_none());
}

#[test]
fn untouched_crop_frame_has_no_menu_and_a_pending_one_wins_over_the_pasteboard() {
    let mut h = harness(crop_app());
    let rect = h.state().last_canvas_rect;
    let outside = rect.min + vec2(5.0, 5.0);
    right_click(&mut h, rect.center(), Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "the tool's untouched frame is no crop in progress");
    right_click(&mut h, outside, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.pasteboard), "outside: the pasteboard menu, as before");
    h.state_mut().ui.canvas_tool_menu = None;
    draw_crop(h.state_mut());
    h.run_steps(2);
    right_click(&mut h, outside, Modifiers::NONE);
    let menu = h.state().ui.canvas_tool_menu.clone().unwrap();
    assert!(menu.crop.is_some() && !menu.pasteboard, "with a crop pending, its menu also outside the image");
}

/// A harness in the app's order: shortcuts (the crop's Esc and ↵ among them), then the canvas.
fn shortcut_harness(mut app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::shortcuts::handle(app, &ctx);
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
    h.run_steps(4);
    h
}

#[test]
fn esc_closes_only_the_crop_menu() {
    let mut app = crop_app();
    let frame = draw_crop(&mut app);
    let mut h = shortcut_harness(app);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.crop.is_some()));
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "Esc closes the menu");
    assert_eq!(h.state().ui.crop_rect, Some(frame), "and only the menu: the crop is not cancelled");
    assert!(crate::crop_ui::pending(h.state()));
}

/// ↵ under the open crop menu doesn't commit the crop beneath it.
#[test]
fn enter_under_the_crop_menu_does_not_crop() {
    let mut app = crop_app();
    let frame = draw_crop(&mut app);
    let mut h = shortcut_harness(app);
    let size = h.state().session.active().map(|s| s.doc.size);
    let center = h.state().last_canvas_rect.center();
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.crop.is_some()));
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    assert_eq!(h.state().session.active().map(|s| s.doc.size), size, "↵ under the menu doesn't crop");
    assert_eq!(h.state().ui.crop_rect, Some(frame));
    assert!(crate::crop_ui::pending(h.state()));
}

/// A right-click while the left button drags the crop frame opens no menu over the drag, on
/// the image or outside it (no pasteboard menu either), and the menu opens again once the left
/// button is up, though egui ended that drag without a release of its own.
#[test]
fn no_crop_menu_while_the_left_button_drags_the_frame() {
    let mut app = crop_app();
    draw_crop(&mut app);
    let mut h = harness(app);
    let rect = h.state().last_canvas_rect;
    let (center, outside) = (rect.center(), rect.min + vec2(5.0, 5.0));
    let left = |pos, pressed| egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.event(egui::Event::PointerMoved(center));
    h.run_steps(1);
    h.event(left(center, true));
    h.run_steps(1);
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(center + vec2(8.0 * k as f32, 0.0)));
        h.run_steps(1);
    }
    assert!(h.state().crop.drag.is_some(), "the left button drags the frame");
    let held = center + vec2(32.0, 0.0);
    right_click(&mut h, held, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "no crop menu over the drag");
    right_click(&mut h, outside, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "nor the pasteboard menu outside the image");
    h.event(egui::Event::PointerMoved(held));
    h.event(left(held, false));
    h.run_steps(2);
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.crop.is_some()), "with the button up the menu opens");
}

/// The left button down with no crop gesture under way (a press egui doesn't take for a drag
/// yet, or one whose release was lost to an Alt+Tab or a cancelled touch) doesn't keep the
/// crop menu shut.
#[test]
fn a_left_button_down_without_a_crop_drag_keeps_the_crop_menu() {
    let mut app = crop_app();
    draw_crop(&mut app);
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    h.event(egui::Event::PointerMoved(center));
    h.run_steps(1);
    h.event(egui::Event::PointerButton { pos: center, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(1);
    assert!(h.ctx.input(|i| i.pointer.primary_down()) && h.state().crop.drag.is_none(), "down, dragging nothing");
    right_click(&mut h, center, Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.crop.is_some()), "the crop menu opens");
}

/// Nor does a right-click open a menu over a Straighten line the left button still draws.
#[test]
fn no_crop_menu_while_the_left_button_draws_a_straighten_line() {
    let mut app = crop_app();
    draw_crop(&mut app);
    crate::crop_straighten::toggle(&mut app);
    let mut h = harness(app);
    let center = h.state().last_canvas_rect.center();
    let left = |pos, pressed| egui::Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.event(egui::Event::PointerMoved(center));
    h.run_steps(1);
    h.event(left(center, true));
    h.run_steps(1);
    for k in 1..=4 {
        h.event(egui::Event::PointerMoved(center + vec2(8.0 * k as f32, 4.0)));
        h.run_steps(1);
    }
    assert!(h.state().crop.straighten.line.is_some() && h.state().crop.drag.is_none(), "the left button draws a Straighten line");
    right_click(&mut h, center + vec2(32.0, 4.0), Modifiers::NONE);
    assert!(h.state().ui.canvas_tool_menu.is_none(), "no menu over the line");
}

/// A finger held still on a pending crop (a touch screen's press-and-hold right-click) opens
/// the crop menu, though the touch counts as the primary button down.
#[test]
fn a_finger_held_on_a_pending_crop_opens_the_crop_menu() {
    let mut app = crop_app();
    let frame = draw_crop(&mut app);
    let mut h = harness(app);
    let at = h.state().last_canvas_rect.center();
    let touch = |phase| egui::Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(1), phase, pos: at, force: None };
    let button = |pressed| egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE };
    h.input_mut().events.extend([egui::Event::PointerMoved(at), touch(egui::TouchPhase::Start), button(true)]);
    h.step();
    // Held past egui's max_click_duration (0.8 s) without moving.
    for _ in 0..120 {
        if h.state().ui.canvas_tool_menu.is_some() {
            break;
        }
        h.step();
    }
    assert!(h.state().ui.canvas_tool_menu.as_ref().is_some_and(|m| m.crop.is_some()), "the held finger opens the crop menu");
    h.input_mut().events.extend([touch(egui::TouchPhase::End), button(false)]);
    h.run_steps(2);
    assert_eq!(h.state().ui.crop_rect, Some(frame), "the hold leaves the frame alone");
}

#[test]
fn crop_menu_rows_act_like_the_options_bar() {
    use crate::crop_overlay::{CropOverlay, OverlayShow};
    let ctx = Context::default();
    let mut app = crop_app();
    draw_crop(&mut app);
    // Space's Hand held over the Crop tool keeps its own right-click.
    assert!(!open_crop(&mut app, &ctx, Tool::Hand, [10.0, 10.0]));
    let reopen = |app: &mut PhotocraftApp| {
        assert!(open_crop(app, &Context::default(), Tool::Crop, [10.0, 10.0]));
        app.ui.canvas_tool_menu.clone().unwrap()
    };
    // Nothing to save with a free ratio, nothing to delete without saved presets.
    let menu = reopen(&mut app);
    assert!(!entry_enabled(&app, &menu, CROP_NEW) && !entry_enabled(&app, &menu, CROP_DELETE));
    assert!(mode_checked(&app, &menu, "crop.ratio."), "free Ratio is checked, as in the dropdown");
    // A size preset fills W x H x Resolution, which the menu checks, as the dropdown does.
    choose(&mut app, &ctx, "crop.ratio.preset:4x5in");
    let menu = reopen(&mut app);
    assert!(mode_checked(&app, &menu, "crop.ratio.whr") && !mode_checked(&app, &menu, "crop.ratio.preset:4x5in"));
    assert!((aspect(app.ui.crop_rect.unwrap()) - 0.8).abs() < 1e-9);
    assert!(entry_enabled(&app, &menu, CROP_NEW));
    // Clear empties W x H x Resolution, as the bar's Clear.
    choose(&mut app, &ctx, CROP_CLEAR);
    assert_eq!(app.ui.tool_options.crop_width, "");
    // Overlays: another overlay starts at its first orientation; when it shows.
    app.ui.tool_options.crop_overlay = CropOverlay::GoldenSpiral;
    app.ui.tool_options.crop_overlay_orientation = 2;
    reopen(&mut app);
    choose(&mut app, &ctx, "crop.overlay.triangle");
    assert_eq!((app.ui.tool_options.crop_overlay, app.ui.tool_options.crop_overlay_orientation), (CropOverlay::Triangle, 0));
    reopen(&mut app);
    choose(&mut app, &ctx, "crop.overlayShow.never");
    assert_eq!(app.ui.tool_options.crop_overlay_show, OverlayShow::Never);
    let menu = reopen(&mut app);
    assert!(mode_checked(&app, &menu, "crop.overlay.triangle") && mode_checked(&app, &menu, "crop.overlayShow.never"));
    // A saved preset is listed by name; picking it refits the frame to its W x H, though the
    // ratio key is "whr" before and after.
    app.ui.canvas_tool_menu = None;
    app.ui.tool_options.crop_ratio = crate::crop_size::WHR.into();
    let saved = json!({"crop_ratio": "whr", "crop_width": "300 px", "crop_height": "100 px", "crop_resolution": "72", "crop_resolution_unit": "px/in"});
    app.run("tool.presets.new", json!({"name": "Banner", "tool": "crop", "options": {"toolOptions": saved}})).unwrap();
    let menu = reopen(&mut app);
    let banner = crop_rows(&menu).into_iter().flatten().find(|(l, _)| l == "Banner").expect("the saved preset is a row").1;
    assert!(entry_enabled(&app, &menu, CROP_DELETE));
    choose(&mut app, &ctx, &banner);
    assert_eq!(app.ui.tool_options.crop_width, "300 px");
    assert!((aspect(app.ui.crop_rect.unwrap()) - 3.0).abs() < 1e-9, "the frame follows the preset: {:?}", app.ui.crop_rect);
    let menu = reopen(&mut app);
    assert!(mode_checked(&app, &menu, &banner));
    // Crop commits as ↵ does: the preset's size.
    choose(&mut app, &ctx, CROP_COMMIT);
    let size = app.session.active().unwrap().doc.size;
    assert_eq!((size.width, size.height), (300, 100));
    // Cancel drops a pending crop.
    draw_crop(&mut app);
    reopen(&mut app);
    choose(&mut app, &ctx, CROP_CANCEL);
    assert!(!crate::crop_ui::pending(&app));
}
