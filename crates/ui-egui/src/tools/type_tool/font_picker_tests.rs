//! The Type tool's searchable font menu (#1369): it keeps its height and focuses its search field.

use super::font_picker_in;
use egui::accesskit::Role;
use egui::{Key, Modifiers, Rect, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_engine::prefs::FontPreview;

struct State {
    current: String,
    families: Vec<String>,
    button: egui::Id,
    popup: egui::Id,
    preview: FontPreview,
    recents: Vec<String>,
}

/// Frames a harness may take to settle. Opening the font menu scrolls it to the current family on
/// the first laid-out pass, and the rows that scroll into view load their samples then: a family
/// with an Arabic sample asks for one more pass to widen its row. How many do depends on the
/// fonts installed (Linux CI settled after 6), so 4 frames, egui's default, is too few.
const SETTLE_STEPS: u64 = 16;

fn harness() -> Harness<'static, State> {
    harness_with(FontPreview::Medium)
}

fn harness_with(preview: FontPreview) -> Harness<'static, State> {
    harness_with_recents(preview, Vec::new())
}

fn harness_with_recents(preview: FontPreview, recents: Vec<String>) -> Harness<'static, State> {
    // Enough families that the unfiltered menu hits its maximum height and scrolls.
    let mut families: Vec<String> = (0..60).map(|i| format!("Family {i:02}")).collect();
    families.push("Zeta Sans".into());
    let state = State { current: "Family 00".into(), families, button: egui::Id::NULL, popup: egui::Id::NULL, preview, recents };
    let mut h = Harness::builder().with_size(vec2(600.0, 900.0)).with_max_steps(SETTLE_STEPS).build_ui_state(
        |ui, s: &mut State| {
            s.button = ui.make_persistent_id(egui::IdSalt::new("type-font"));
            s.popup = s.button.with("popup");
            let fams = s.families.clone();
            let recents = s.recents.clone();
            font_picker_in(ui, &mut s.current, 170.0, &fams, s.preview, &recents);
        },
        state,
    );
    h.run();
    h
}

fn popup_rect(h: &Harness<'static, State>) -> Option<Rect> {
    let id = h.state().popup;
    let open = egui::Popup::is_id_open(&h.ctx, id);
    if open { h.ctx.memory(|m| m.area_rect(id)) } else { None }
}

fn open(h: &mut Harness<'static, State>) {
    h.query_all_by_role(Role::ComboBox).next().unwrap().click();
    h.run();
}

/// A native click can leave the opening button held across several rendered frames.
fn hold_open<S>(h: &mut Harness<'_, S>, button: egui::Pos2) {
    h.hover_at(button);
    h.run();
    h.event(egui::Event::PointerButton { pos: button, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(4);
    h.event(egui::Event::PointerButton { pos: button, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
}

#[test]
fn hovering_a_font_row_sets_preview_and_leaving_clears_it() {
    let mut h = harness();
    open(&mut h);
    let row = h.get_by_label("Family 02").rect().center();
    let popup = popup_rect(&h).expect("font menu open");
    assert!(popup.contains(row), "row {row:?} is outside popup {popup:?}");
    h.hover_at(row);
    h.run_steps(2);
    let hover_id = egui::Id::unique("type-font-hover");
    assert_eq!(h.ctx.input(|i| i.pointer.interact_pos()), Some(row));
    assert!(popup_rect(&h).is_some(), "popup closes while moving pointer to the row");
    assert_eq!(
        h.ctx.data(|d| d.get_temp::<String>(hover_id)).as_deref(),
        Some("Family 02"),
        "row {row:?}, popup {popup:?}, top layer {:?}",
        h.ctx.layer_id_at(row)
    );

    let search = h.get_by_role(Role::TextInput).rect().center();
    h.hover_at(search);
    h.run();
    assert!(h.ctx.data(|d| d.get_temp::<String>(hover_id)).is_none());
}

/// #7406: hovering a font highlights its row without moving anything: the other rows keep
/// their place, the menu its width, and every row (the selected one too) takes one height.
#[test]
fn hovering_a_font_row_moves_no_other_row() {
    let mut h = harness();
    // The app's theme: its hovered and selected buttons draw a stroke (egui's default style
    // would hide the jump).
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run();
    open(&mut h);
    let rows = |h: &Harness<'static, State>| -> Vec<Rect> { (0..8).map(|i| h.get_by_label(&format!("Family {i:02}")).rect()).collect() };
    let before = rows(&h);
    let width = popup_rect(&h).expect("font menu open").width();
    let tops: Vec<f32> = before.windows(2).map(|w| w[1].top() - w[0].top()).collect();
    assert!(tops.windows(2).all(|t| (t[0] - t[1]).abs() < 0.01), "rows are evenly spaced, the selected Family 00 included: {tops:?}");
    for hovered in [2, 5] {
        h.hover_at(before[hovered].center());
        h.run_steps(2);
        let after = rows(&h);
        for (i, (b, a)) in before.iter().zip(&after).enumerate() {
            if i != hovered {
                assert_eq!(b, a, "hovering Family {hovered:02} moved Family {i:02}");
            }
        }
        assert_eq!(after[hovered].center(), before[hovered].center(), "the hovered row stays centred where it was");
        assert_eq!(popup_rect(&h).map(|r| r.width()), Some(width), "the menu keeps its width");
    }
}

/// #7792: the search field stays at the top of the menu while the list scrolls under it. The
/// menu's own scroll area used to carry the field away with the rows.
#[test]
fn the_search_field_stays_put_while_the_list_scrolls() {
    let mut h = harness();
    open(&mut h);
    let search = h.get_by_role(Role::TextInput).rect();
    let popup = popup_rect(&h).expect("font menu open");
    let first_row = h.get_by_label("Family 00").rect();

    // The wheel over a row scrolls the list.
    h.hover_at(h.get_by_label("Family 05").rect().center());
    for _ in 0..6 {
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, -3.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run();
    }
    let scrolled = h.get_by_label("Family 00").rect();
    assert!(scrolled.top() < first_row.top() - 1.0, "the list did not scroll: {first_row:?} -> {scrolled:?}");

    // The field did not move with the rows: it is still where it was, inside the menu.
    let after = h.get_by_role(Role::TextInput).rect();
    assert_eq!(search, after, "the search field scrolled away with the list");
    assert!(popup.contains(after.center()), "the search field left the menu");
}

/// #7792: the arrow keys scroll the list to the selected family without moving the search
/// field: they scroll the list, not the whole menu.
#[test]
fn the_search_field_stays_put_while_the_arrows_scroll_the_list() {
    let mut h = harness();
    open(&mut h);
    let search = h.get_by_role(Role::TextInput).rect();
    for _ in 0..40 {
        h.key_press(Key::ArrowDown);
        h.run();
    }
    assert_eq!(h.state().current, "Family 40");
    let row = h.get_by_label("Family 40").rect();
    let popup = popup_rect(&h).expect("font menu open");
    assert!(popup.contains(row.center()), "the selected row is in view: {row:?} in {popup:?}");
    assert_eq!(search, h.get_by_role(Role::TextInput).rect(), "the search field scrolled away with the list");
}

#[test]
fn clearing_the_query_restores_the_menu_height() {
    let mut h = harness();
    open(&mut h);
    let full = popup_rect(&h).expect("menu open").height();
    h.get_by_role(Role::TextInput).type_text("zeta");
    h.run();
    assert!(h.query_by_label("Family 01").is_none(), "the query filters the list");
    let searching = popup_rect(&h).expect("still open").height();
    for _ in 0.."zeta".len() {
        h.key_press(Key::Backspace);
    }
    h.run();
    assert!(h.query_by_label("Family 01").is_some(), "the cleared query lists every family");
    let cleared = popup_rect(&h).expect("still open").height();
    assert!((cleared - full).abs() < 0.5, "height after clearing {cleared} != original {full}");
    assert!((searching - full).abs() < 0.5, "the menu keeps its height while searching: {searching} vs {full}");
}

#[test]
fn reopening_focuses_the_search_field_and_clicking_it_keeps_the_menu_open() {
    let mut h = harness();
    let button = h.query_all_by_role(Role::ComboBox).next().unwrap().rect().center();
    hold_open(&mut h, button);
    assert!(h.get_by_role(Role::TextInput).is_focused(), "search is focused on first open");
    h.event(egui::Event::Text("zeta".into()));
    h.run();
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("zeta"), "typing goes straight to the search field");
    // Close and reopen on consecutive passes: the query stays and the field is focused again.
    h.input_mut().events.push(egui::Event::Key { key: Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE });
    h.step();
    assert!(popup_rect(&h).is_none(), "Escape closes the menu");
    h.query_all_by_role(Role::ComboBox).next().unwrap().click_accesskit();
    h.run();
    let search = h.get_by_role(Role::TextInput);
    assert_eq!(search.value().as_deref(), Some("zeta"));
    assert!(search.is_focused(), "search is focused on reopen with a query");
    // Clicking the search field (to edit the query) must not close the menu.
    h.get_by_role(Role::TextInput).click();
    h.run();
    assert!(popup_rect(&h).is_some(), "clicking the search field keeps the menu open");
    h.key_press_modifiers(Modifiers::COMMAND, Key::A);
    h.get_by_role(Role::TextInput).type_text("Zeta S");
    h.run();
    assert!(popup_rect(&h).is_some());
    // Picking a font still closes the menu.
    h.get_by_label("Zeta Sans").click();
    h.run();
    assert_eq!(h.state().current, "Zeta Sans");
    assert!(popup_rect(&h).is_none(), "choosing a font closes the menu");
}

#[test]
fn opening_click_release_returns_focus_to_search_before_typing() {
    let mut h = harness();
    let button = h.query_all_by_role(Role::ComboBox).next().unwrap().rect().center();
    h.hover_at(button);
    h.run();
    h.event(egui::Event::PointerButton { pos: button, button: egui::PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.run_steps(4);
    assert!(h.get_by_role(Role::TextInput).is_focused());
    // A native opening click can hand focus back to its button when it ends, after the
    // search has already been focused for several passes. Model that platform handoff.
    let id = h.state().button;
    h.ctx.memory_mut(|m| m.request_focus(id));
    h.event(egui::Event::PointerButton { pos: button, button: egui::PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run();
    assert!(h.get_by_role(Role::TextInput).is_focused(), "the opening release leaves the search focused");
    h.event(egui::Event::Text("zeta".into()));
    h.run();
    assert_eq!(h.get_by_role(Role::TextInput).value().as_deref(), Some("zeta"));
    assert!(h.query_by_label("Family 01").is_none());
}

#[test]
fn first_options_bar_open_searches_without_changing_the_tool_or_edited_text() {
    use crate::{PhotocraftApp, Services};
    use photocraft_doc::LayerContent;
    use serde_json::json;

    for editing in [false, true] {
        let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
            app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
            app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
            app.ui.tool = crate::state::Tool::Type;
            if editing {
                super::edit_active(&mut app).unwrap();
            }
            app.sync_views();
            app
        });
        h.run_steps(8);
        let button = h.query_all_by_role(Role::ComboBox).find(|n| n.value().as_deref() == Some("Inter")).unwrap().rect().center();
        hold_open(&mut h, button);
        // Native typing carries key events as well as text; an unfocused search would let
        // the first key switch tools, or replace the selected inline text instead.
        h.event(egui::Event::Key { key: Key::J, physical_key: Some(Key::J), pressed: true, repeat: false, modifiers: Modifiers::NONE });
        h.event(egui::Event::Text("JetBrains".into()));
        h.run();
        let search = h.query_all_by_role(Role::TextInput).find(|n| n.is_focused()).expect("font search focused on first open");
        assert_eq!(search.value().as_deref(), Some("JetBrains"));
        assert_eq!(h.state().ui.tool, crate::state::Tool::Type);
        let st = h.state().session.active().unwrap();
        let LayerContent::Text(text) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("type layer") };
        assert_eq!(text.text, "Sample");
    }
}

#[test]
fn arrows_apply_fonts_without_closing_and_enter_accepts() {
    let mut h = harness();
    open(&mut h);
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().current, "Family 01");
    assert!(popup_rect(&h).is_some());
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowUp);
    h.run();
    assert_eq!(h.state().current, "Family 01");
    h.key_press(Key::Enter);
    h.run();
    assert!(popup_rect(&h).is_none());
}

/// #7407: as in Photoshop, the field keeps the focus once a font is picked (click or Enter),
/// and ↑ / ↓ on it step through the families with the menu closed. Escape ends it.
#[test]
fn arrows_on_the_field_step_through_the_families_after_a_pick() {
    let mut h = harness();
    open(&mut h);
    h.get_by_label("Family 03").click();
    h.run();
    assert_eq!(h.state().current, "Family 03");
    assert!(popup_rect(&h).is_none(), "picking closes the menu");
    let button = h.state().button;
    assert!(h.ctx.memory(|m| m.has_focus(button)), "the field keeps the focus");
    let press = |h: &mut Harness<'static, State>, key: Key| {
        h.key_press(key);
        h.run();
        assert!(popup_rect(h).is_none(), "{key:?} leaves the menu closed");
        assert!(h.ctx.memory(|m| m.has_focus(button)), "{key:?} keeps the focus on the field");
        h.state().current.clone()
    };
    assert_eq!(press(&mut h, Key::ArrowDown), "Family 04");
    assert_eq!(press(&mut h, Key::ArrowDown), "Family 05");
    assert_eq!(press(&mut h, Key::ArrowUp), "Family 04");
    // Enter in the open menu is a pick too.
    open(&mut h);
    h.key_press(Key::ArrowDown);
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().current, "Family 05");
    assert_eq!(press(&mut h, Key::ArrowDown), "Family 06");
    // Escape takes the focus away; the arrows then leave the font alone.
    h.key_press(Key::Escape);
    h.run();
    assert!(!h.ctx.memory(|m| m.has_focus(button)));
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().current, "Family 06");
}

/// The first arrow right after a pick (in the very next frame) steps the font and keeps the
/// focus on the field, also between focusable widgets: the field's lock filter only holds from
/// the frame after it got the focus, and egui would read that arrow as a move to "Below".
#[test]
fn the_first_arrow_after_a_pick_keeps_the_focus_between_neighbours() {
    for (enter, picked, stepped) in [(false, "Family 05", "Family 06"), (true, "Family 04", "Family 05")] {
        let families: Vec<String> = (0..20).map(|i| format!("Family {i:02}")).collect();
        let state =
            State { current: "Family 03".into(), families, button: egui::Id::NULL, popup: egui::Id::NULL, preview: FontPreview::Off, recents: Vec::new() };
        let mut h = Harness::builder().with_size(vec2(600.0, 900.0)).with_max_steps(SETTLE_STEPS).build_ui_state(
            |ui, s: &mut State| {
                let _ = ui.button("Above");
                s.button = ui.make_persistent_id(egui::IdSalt::new("type-font"));
                s.popup = s.button.with("popup");
                let fams = s.families.clone();
                font_picker_in(ui, &mut s.current, 170.0, &fams, s.preview, &s.recents);
                let _ = ui.button("Below");
            },
            state,
        );
        h.run();
        open(&mut h);
        if enter {
            h.key_press(Key::ArrowDown);
            h.key_press(Key::Enter);
        } else {
            h.get_by_label("Family 05").click();
        }
        // Frame by frame up to the pick, then the arrow in the next frame.
        for _ in 0..8 {
            if h.state().current == picked && popup_rect(&h).is_none() {
                break;
            }
            h.step();
        }
        assert_eq!(h.state().current, picked, "enter: {enter}");
        h.key_press(Key::ArrowDown);
        h.step();
        h.run();
        assert_eq!(h.state().current, stepped, "enter: {enter}");
        let button = h.state().button;
        assert!(h.ctx.memory(|m| m.has_focus(button)), "enter: {enter}: the arrow moved the focus off the field");
    }
}

#[test]
fn arrows_follow_the_filtered_list_and_handle_no_matches() {
    let mut h = harness();
    open(&mut h);
    h.get_by_role(Role::TextInput).type_text("Family 5");
    h.run();
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().current, "Family 50");
    h.key_press(Key::ArrowUp);
    h.run();
    assert_eq!(h.state().current, "Family 50", "clamp at the first match");
    h.get_by_role(Role::TextInput).type_text("missing");
    h.run();
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowUp);
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().current, "Family 50");
    assert!(popup_rect(&h).is_some(), "no match cannot accept an unrelated font");
}

#[test]
fn arrows_scroll_the_selected_font_into_view() {
    let mut h = harness();
    open(&mut h);
    for _ in 0..59 {
        h.key_press(Key::ArrowDown);
        h.run();
    }
    assert_eq!(h.state().current, "Family 59");
    let row = h.get_by_label("Family 59");
    assert!(popup_rect(&h).unwrap().contains(row.rect().center()));
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().current, "Zeta Sans", "clamp at the last font");
}

#[test]
fn empty_font_list_ignores_navigation() {
    let mut h = harness();
    h.state_mut().families.clear();
    open(&mut h);
    h.key_press(Key::ArrowDown);
    h.key_press(Key::ArrowUp);
    h.key_press(Key::Enter);
    h.run();
    assert_eq!(h.state().current, "Family 00");
    assert!(popup_rect(&h).is_some());
}

#[test]
fn font_cycle_in_options_bar_edits_the_layer_and_can_be_undone() {
    use crate::{PhotocraftApp, Services};
    use serde_json::json;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).with_max_steps(SETTLE_STEPS).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| crate::type_tool::options_bar(app, ui));
            }
        },
        app,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h.query_all_by_role(Role::ComboBox).next().unwrap().click();
    // The font search field's caret blinks every pass, so settle with bounded steps instead.
    h.run_steps(2);
    h.get_by_role(Role::TextInput).type_text("JetBrains");
    h.key_press(Key::ArrowDown);
    h.run();
    let font = |app: &PhotocraftApp| {
        let doc = &app.session.active().unwrap().doc;
        match &doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().content {
            photocraft_doc::LayerContent::Text(t) => t.font_family.clone(),
            _ => panic!("type layer"),
        }
    };
    assert_eq!(font(h.state()), "JetBrains Mono");
    h.key_press(Key::Enter);
    h.run();
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert_eq!(font(h.state()), "Inter");
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert_eq!(font(h.state()), "JetBrains Mono");
}

#[test]
fn hovering_a_font_does_not_pollute_history_and_clears_on_close() {
    use crate::{PhotocraftApp, Services};
    use serde_json::json;

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();

    let layer_id = app.session.active().unwrap().active_layer.unwrap();
    let history_before = app.session.active().unwrap().history.entries().len();

    // Simulate hovering a font directly via the preview module
    let ctx = egui::Context::default();
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique("type-font-hover"), "JetBrains Mono".to_string()));
    crate::type_font_preview::follow_hover(&mut app, &ctx);

    // Verify a preview document is generated
    let (_, preview_key) = crate::type_font_preview::display_doc(&mut app, 0).expect("preview document");

    // Verify the original document's history is NOT polluted
    let history_after = app.session.active().unwrap().history.entries().len();
    assert_eq!(history_before, history_after, "Hovering a font does not create an undo step");
    let doc = app.session.active().unwrap().doc.id;
    let revision = app.session.active().unwrap().revision;
    assert!(crate::type_font_preview::damage(&app, doc, revision, 0, preview_key).is_some(), "preview bounds survive dismissal for canvas invalidation");

    // Verify the actual document layer still has the original font
    let original_font = match &app.session.active().unwrap().doc.layer(layer_id).unwrap().content {
        photocraft_doc::LayerContent::Text(t) => t.font_family.clone(),
        _ => panic!("type layer"),
    };
    assert_eq!(original_font, "Inter", "Hovering does not commit the font to the document");

    // Simulate the font menu closing (hover cleared)
    crate::type_font_preview::follow_hover(&mut app, &ctx);
    let no_preview = crate::type_font_preview::display_doc(&mut app, 0);
    assert!(no_preview.is_none(), "No preview document is returned after clearing");

    crate::type_font_preview::set(&mut app, "Unavailable Font".into());
    assert!(crate::type_font_preview::display_doc(&mut app, 0).is_none(), "unavailable fonts do not render a misleading fallback preview");
}

#[test]
fn options_bar_hover_previews_on_canvas_without_committing() {
    use crate::{PhotocraftApp, Services};
    use photocraft_doc::LayerContent;
    use serde_json::json;

    struct PreviewApp {
        app: PhotocraftApp,
    }

    impl eframe::App for PreviewApp {
        fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            crate::type_font_preview::follow_hover(&mut self.app, ctx);
        }

        fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
            ui.horizontal(|ui| crate::type_tool::options_bar(&mut self.app, ui));
        }
    }

    // Opening the menu on the current family (#9197) paints rows that load their font samples
    // over a few frames, more than the default four steps.
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ALL[0]);
        PreviewApp { app: PhotocraftApp::new(photocraft_engine::Session::new(), Services::default()) }
    });
    h.state_mut().app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    h.state_mut().app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
    let layer = h.state().app.session.active().unwrap().active_layer.unwrap();
    h.state_mut().app.ui.tool = crate::state::Tool::Type;
    h.run_steps(3);
    h.query_all_by_role(Role::ComboBox).next().unwrap().click();
    h.run();
    h.get_by_role(Role::TextInput).type_text("JetBrains Mono");
    h.run();
    h.get_by_label("JetBrains Mono").hover();
    h.run();

    let (preview, _) = crate::canvas::display_doc(&mut h.state_mut().app, 0);
    let LayerContent::Text(preview_text) = &preview.layer(layer).unwrap().content else {
        panic!("type layer");
    };
    assert_eq!(preview_text.char_runs()[0].style.font_family, "JetBrains Mono");
    let LayerContent::Text(original_text) = &h.state().app.session.active().unwrap().doc.layer(layer).unwrap().content else {
        panic!("type layer");
    };
    assert_eq!(original_text.char_runs()[0].style.font_family, "Inter");
    assert_eq!(h.state().app.session.active().unwrap().history.entries().len(), 2, "hover adds no history entry");

    h.get_by_role(Role::TextInput).hover();
    h.run();
    let (restored, key) = crate::canvas::display_doc(&mut h.state_mut().app, 0);
    let LayerContent::Text(restored_text) = &restored.layer(layer).unwrap().content else {
        panic!("type layer");
    };
    assert_eq!(restored_text.char_runs()[0].style.font_family, "Inter");
    assert_eq!(key, 0, "leaving the font row restores committed canvas content");

    h.get_by_label("JetBrains Mono").hover();
    h.run();
    h.get_by_label("JetBrains Mono").click();
    h.run();
    let (committed, key) = crate::canvas::display_doc(&mut h.state_mut().app, 0);
    let LayerContent::Text(committed_text) = &committed.layer(layer).unwrap().content else {
        panic!("type layer");
    };
    assert_eq!(committed_text.char_runs()[0].style.font_family, "JetBrains Mono");
    assert_eq!(key, 0, "selection replaces the preview with committed content");
    assert_eq!(h.state().app.session.active().unwrap().history.entries().len(), 3, "selection adds one history step");
    let committed_font = |app: &PhotocraftApp| {
        let LayerContent::Text(text) = &app.session.active().unwrap().doc.layer(layer).unwrap().content else {
            panic!("type layer");
        };
        text.char_runs()[0].style.font_family.clone()
    };
    h.state_mut().app.run("edit.undo", json!({})).unwrap();
    assert_eq!(committed_font(&h.state().app), "Inter");
    h.state_mut().app.run("edit.redo", json!({})).unwrap();
    assert_eq!(committed_font(&h.state().app), "JetBrains Mono");
}

#[test]
fn hovering_a_font_previews_only_the_selected_characters() {
    use crate::{PhotocraftApp, Services};
    use serde_json::json;

    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("type.create", json!({"text": "abcdef", "font": "Inter", "size": 18})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.run("type.setStyle", json!({"layer": layer.0, "range": [0, 2], "font": "JetBrains Mono"})).unwrap();
    app.ui.text_edit = Some(crate::state::TextEdit {
        layer: layer.0,
        document: None,
        caret: 5,
        anchor: 3,
        session: "font-preview-test".into(),
        created: false,
        mask: false,
        dragging: false,
        resize: None,
        preedit: None,
    });

    crate::type_font_preview::set(&mut app, "Inter".into());
    let (preview, _) = crate::type_font_preview::display_doc(&mut app, 0).expect("preview document");
    let fonts = |doc: &photocraft_doc::Document| {
        let photocraft_doc::LayerContent::Text(text) = &doc.layer(layer).unwrap().content else {
            panic!("type layer");
        };
        text.char_runs().into_iter().flat_map(|run| std::iter::repeat_n(run.style.font_family, run.len)).collect::<Vec<_>>()
    };

    assert_eq!(fonts(&preview), ["JetBrains Mono", "JetBrains Mono", "Inter", "Inter", "Inter", "Inter"]);
    assert_eq!(fonts(&app.session.active().unwrap().doc), ["JetBrains Mono", "JetBrains Mono", "Inter", "Inter", "Inter", "Inter"]);
}

/// Preferences ▸ Type ▸ Font Preview Size: Extra Large grows the sample rows, Off drops the
/// samples (rows shrink to the plain height).
#[test]
fn the_preview_size_preference_resizes_or_removes_the_samples() {
    let mut h = harness_with(FontPreview::Medium);
    open(&mut h);
    let medium = h.get_by_label("Family 00").rect().height();
    assert!((medium - 28.0).abs() < 0.5, "medium rows hold the 24 px sample: {medium}");
    let mut h = harness_with(FontPreview::ExtraLarge);
    open(&mut h);
    let extra = h.get_by_label("Family 00").rect().height();
    assert!(extra > medium, "extra large rows are taller: {extra} vs {medium}");
    let mut h = harness_with(FontPreview::Off);
    open(&mut h);
    let off = h.get_by_label("Family 00").rect().height();
    assert!((off - 28.0).abs() < 0.5, "off keeps the plain row height: {off}");
}

/// Preferences ▸ Type ▸ Show Font Names in English off: the menu lists a family by its localized
/// name, finds it by either name, and picking it still applies the English family.
#[test]
fn localized_font_names_label_the_menu_but_pick_the_family() {
    use std::collections::HashMap;
    use std::sync::Arc;
    let state = State {
        current: "Arial".into(),
        families: vec!["Arial".into(), "MS Gothic".into(), "Zeta Sans".into()],
        button: egui::Id::NULL,
        popup: egui::Id::NULL,
        preview: FontPreview::Medium,
        recents: Vec::new(),
    };
    let mut h = Harness::builder().with_size(vec2(600.0, 900.0)).with_max_steps(SETTLE_STEPS).build_ui_state(
        |ui, s: &mut State| {
            s.popup = ui.make_persistent_id(egui::IdSalt::new("type-font")).with("popup");
            let fams = s.families.clone();
            let recents = s.recents.clone();
            font_picker_in(ui, &mut s.current, 170.0, &fams, s.preview, &recents);
        },
        state,
    );
    // The localized menu lists Japanese names, so install the app's bundled theme fonts
    // (including the craft-fonts Japanese faces) before the first pass shapes text.
    crate::theme::install_fonts(&h.ctx);
    // Show Font Names in English on: no localized labels.
    crate::type_tool::sync_font_names(&h.ctx, true);
    let labels: HashMap<String, String> = [("MS Gothic".to_string(), "ＭＳ ゴシック".to_string())].into();
    h.ctx.data_mut(|d| d.insert_temp(super::font_labels_id(), Arc::new(labels.clone())));
    h.run();
    open(&mut h);
    assert!(h.query_by_label("ＭＳ ゴシック").is_some(), "the localized name labels the family");
    assert!(h.query_by_label("MS Gothic").is_none());
    assert!(h.query_by_label("Arial").is_some(), "fonts without a localized name keep theirs");
    h.get_by_role(Role::TextInput).type_text("gothic");
    h.run();
    assert!(h.query_by_label("ＭＳ ゴシック").is_some(), "the English name still finds it");
    assert!(h.query_by_label("Arial").is_none());
    h.get_by_label("ＭＳ ゴシック").click();
    h.run();
    assert_eq!(h.state().current, "MS Gothic", "the document keeps the English family");
    // Turning the preference back on drops the labels.
    crate::type_tool::sync_font_names(&h.ctx, true);
    h.run();
    open(&mut h);
    assert!(h.query_by_label("MS Gothic").is_some());
    assert!(h.query_by_label("ＭＳ ゴシック").is_none());
}

#[test]
fn font_menu_rows_search_both_names_and_sort_by_label() {
    use std::collections::HashMap;
    let families: Vec<String> = ["Arial", "Microsoft YaHei", "MS Gothic"].iter().map(|s| s.to_string()).collect();
    let labels: HashMap<String, String> =
        [("MS Gothic", "ＭＳ ゴシック"), ("Microsoft YaHei", "微软雅黑")].iter().map(|(a, b)| (a.to_string(), b.to_string())).collect();
    let rows = |labels: Option<&HashMap<String, String>>, q: &str| -> Vec<(String, String)> {
        super::font_menu_rows(&families, labels, q, &[]).0.into_iter().map(|(f, l)| (f.clone(), l.to_string())).collect()
    };
    let english = rows(None, "");
    assert!(english.iter().all(|(f, l)| f == l));
    let local = rows(Some(&labels), "");
    assert_eq!(local.iter().map(|(_, l)| l.as_str()).collect::<Vec<_>>(), ["Arial", "微软雅黑", "ＭＳ ゴシック"]);
    assert_eq!(rows(Some(&labels), "yahei"), [("Microsoft YaHei".to_string(), "微软雅黑".to_string())]);
    assert_eq!(rows(Some(&labels), "雅黑"), [("Microsoft YaHei".to_string(), "微软雅黑".to_string())]);
    assert!(rows(Some(&labels), "nothing").is_empty());
}

/// Preferences ▸ Type ▸ Recent Fonts: as in Photoshop, recently used families head the menu in
/// their own group (most recent first, once each) and keep their alphabetical place below it; a
/// query lists each match once, alphabetically.
#[test]
fn recent_fonts_head_the_menu_and_keep_their_place() {
    let families: Vec<String> = ["Arial", "Baskerville", "Courier", "Zeta Sans"].iter().map(|s| s.to_string()).collect();
    let recents: Vec<String> = vec!["Zeta Sans".into(), "Courier".into(), "Gone".into()];
    let rows = |q: &str| -> (Vec<String>, usize) {
        let (rows, n) = super::font_menu_rows(&families, None, q, &recents);
        (rows.into_iter().map(|(f, _)| f.clone()).collect(), n)
    };
    assert_eq!(
        rows(""),
        (vec!["Zeta Sans".into(), "Courier".into(), "Arial".into(), "Baskerville".into(), "Courier".into(), "Zeta Sans".into()], 2),
        "a recent group (no uninstalled family), then the whole alphabetical list"
    );
    assert_eq!(rows("a"), (vec!["Arial".into(), "Baskerville".into(), "Zeta Sans".into()], 0), "a query lists each match once");
}

/// The menu itself draws the recent group first, and the recent families again in their place.
#[test]
fn recent_fonts_appear_first_in_the_menu() {
    let mut h = harness_with_recents(FontPreview::Medium, vec!["Zeta Sans".into(), "Family 05".into()]);
    open(&mut h);
    let tops = |h: &Harness<'static, State>, label: &str| -> Vec<f32> {
        let mut t: Vec<f32> = h.query_all_by_label(label).map(|n| n.rect().top()).collect();
        t.sort_by(f32::total_cmp);
        t
    };
    let (zeta, five, first, six) = (tops(&h, "Zeta Sans"), tops(&h, "Family 05"), tops(&h, "Family 00"), tops(&h, "Family 06"));
    assert_eq!(five.len(), 2, "a recent family is listed twice: {five:?}");
    assert!(zeta[0] < five[0] && five[0] < first[0], "the recent group comes first, in order: {zeta:?} {five:?} {first:?}");
    assert!(first[0] < five[1] && five[1] < six[0], "and keeps its alphabetical place: {five:?} {six:?}");
    assert!(first[0] - five[0] > five[0] - zeta[0], "a divider ends the recent group: {zeta:?} {five:?} {first:?}");
}

/// The open menu's arrows walk on through the list although each step records its family as
/// the most recent one (CodeRabbit on #3559): the recent group keeps its order while the menu
/// is open, and a step from a recent family's lower copy goes on from there.
#[test]
fn open_menu_arrows_walk_on_through_recent_fonts() {
    let mut h = harness_with_recents(FontPreview::Off, vec!["Family 03".into()]);
    open(&mut h);
    let mut seen = vec![h.state().current.clone()];
    for _ in 0..5 {
        h.key_press(Key::ArrowDown);
        h.run();
        let now = h.state().current.clone();
        // The options bar records each applied family as the most recent one.
        h.state_mut().recents.retain(|f| *f != now);
        h.state_mut().recents.insert(0, now.clone());
        h.run();
        seen.push(now);
    }
    assert_eq!(seen, ["Family 00", "Family 01", "Family 02", "Family 03", "Family 04", "Family 05"], "{seen:?}");
    h.key_press(Key::ArrowUp);
    h.run();
    assert_eq!(h.state().current, "Family 04", "↑ steps back");
}

/// Opened on a family that is also a recent one (the usual case: every pick records it), the
/// menu marks its copy in the recent group, as Photoshop's does (#9160), and only that row; the
/// arrows walk on from there through the list as shown (Photoshop: ↓ goes to the next recent
/// family).
#[test]
fn open_menu_on_a_recent_font_marks_its_recent_row() {
    use egui_kittest::kittest::NodeT;
    let mut h = harness_with_recents(FontPreview::Off, vec!["Family 05".into(), "Family 09".into()]);
    h.state_mut().current = "Family 05".into();
    h.run();
    open(&mut h);
    let marked = |h: &Harness<'static, State>, label: &str| -> Vec<f32> {
        h.query_all_by_label(label).filter(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)).map(|n| n.rect().top()).collect()
    };
    let tops = |h: &Harness<'static, State>, label: &str| -> Vec<f32> {
        let mut t: Vec<f32> = h.query_all_by_label(label).map(|n| n.rect().top()).collect();
        t.sort_by(f32::total_cmp);
        t
    };
    assert_eq!(marked(&h, "Family 05"), [tops(&h, "Family 05")[0]], "the upper (recent) copy alone is marked");
    h.key_press(Key::ArrowDown);
    h.run();
    assert_eq!(h.state().current, "Family 09", "↓ walks on to the next recent family");
    h.key_press(Key::ArrowUp);
    h.run();
    assert_eq!(h.state().current, "Family 05", "and ↑ back");
}

/// #9160: the menu opens showing the current family, as Photoshop's does: a family deep in the
/// list is scrolled to the top of the view, not left wherever the list happens to start.
#[test]
fn open_menu_shows_the_current_family() {
    let mut h = harness_with_recents(FontPreview::Off, vec!["Zeta Sans".into()]);
    h.state_mut().current = "Family 40".into();
    h.run();
    open(&mut h);
    h.run();
    let popup = popup_rect(&h).expect("the menu is open");
    let row = h.query_all_by_label("Family 40").next().expect("its row").rect();
    assert!(popup.contains_rect(row), "Family 40 is in view: {row:?} in {popup:?}");
}

/// The current family is in view the frame the menu opens, with the app's scroll animation on:
/// the reveal jumps, where an animated scroll left the row outside the popup for a while.
/// The harness turns animations off, so this turns a slow one back on.
#[test]
fn open_menu_shows_the_current_family_without_animating() {
    let mut h = harness_with_recents(FontPreview::Off, Vec::new());
    h.ctx.all_styles_mut(|s| s.scroll_animation = egui::style::ScrollAnimation::duration(10.0));
    h.state_mut().current = "Family 40".into();
    h.run();
    h.query_all_by_role(Role::ComboBox).next().unwrap().click();
    h.run_steps(4);
    let popup = popup_rect(&h).expect("the menu is open");
    let row = h.query_all_by_label("Family 40").next().expect("its row").rect();
    assert!(popup.contains_rect(row), "Family 40 is in view: {row:?} in {popup:?}");
}

/// Photoshop: ↑ on the top recent row stays there, without wrapping or changing the font.
#[test]
fn open_menu_arrow_up_on_the_top_recent_row_stays_there() {
    use egui_kittest::kittest::NodeT;
    let mut h = harness_with_recents(FontPreview::Off, vec!["Family 05".into(), "Family 09".into()]);
    h.state_mut().current = "Family 05".into();
    h.run();
    open(&mut h);
    let marked_top = |h: &Harness<'static, State>| -> Vec<f32> {
        h.query_all_by_label("Family 05").filter(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)).map(|n| n.rect().top()).collect()
    };
    let before = marked_top(&h);
    assert_eq!(before.len(), 1, "one row marked");
    for _ in 0..2 {
        h.key_press(Key::ArrowUp);
        h.run();
        assert_eq!(h.state().current, "Family 05", "↑ on the top row keeps the font");
        assert_eq!(marked_top(&h), before, "and the mark stays on the top row");
    }
}

/// #9160: when the marked recent row fits below the top of the recent group, the menu opens
/// with the whole group in view, its first row just below the search field.
#[test]
fn open_menu_shows_the_recent_group_when_the_marked_row_fits_below_it() {
    use egui_kittest::kittest::NodeT;
    let recents: Vec<String> = (10..15).map(|i| format!("Family {i}")).collect();
    let mut h = harness_with_recents(FontPreview::Off, recents);
    h.state_mut().current = "Family 12".into();
    h.run();
    open(&mut h);
    h.run();
    let popup = popup_rect(&h).expect("the menu is open");
    let search = h.get_by_role(Role::TextInput).rect();
    let first = h.query_all_by_label("Family 10").map(|n| n.rect()).min_by(|a, b| a.top().total_cmp(&b.top())).expect("the recent group's first row");
    let row = h.query_all_by_label("Family 12").find(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)).expect("its marked row").rect();
    assert!(popup.contains_rect(row) && row.top() >= search.bottom(), "recent #3 is in view: {row:?} in {popup:?}");
    assert!(
        popup.contains_rect(first) && (0.0..=12.0).contains(&(first.top() - search.bottom())),
        "the group's first row sits just below the search field: {first:?} under {search:?}"
    );
}

/// #9160 with large previews: a recent family further down the default ten recents opens in view,
/// not below the fold because the menu scrolled to the top of the recent group.
#[test]
fn open_menu_shows_a_lower_recent_family_with_large_previews() {
    use egui_kittest::kittest::NodeT;
    for preview in [FontPreview::Huge, FontPreview::ExtraLarge] {
        let recents: Vec<String> = (10..20).map(|i| format!("Family {i}")).collect();
        let mut h = harness_with_recents(preview, recents);
        h.state_mut().current = "Family 17".into();
        h.run();
        open(&mut h);
        h.run();
        let popup = popup_rect(&h).expect("the menu is open");
        let search = h.get_by_role(Role::TextInput).rect();
        let row =
            h.query_all_by_label("Family 17").find(|n| n.accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)).expect("its marked row").rect();
        assert!(popup.contains_rect(row) && row.top() >= search.bottom(), "{preview:?}: recent #8 is in view: {row:?} in {popup:?} below {search:?}");
    }
}

/// The open-scroll happens once, when the menu opens. With no row for the current family then
/// (a missing font, a mixed selection), the list the user scrolled does not jump later when one
/// turns up: the font finishes loading, or the selection settles on one family.
#[test]
fn open_menu_without_the_current_family_never_jumps_later() {
    for missing in [true, false] {
        let mut h = harness_with_recents(FontPreview::Off, Vec::new());
        if missing {
            h.state_mut().families.retain(|f| f != "Family 40");
            h.state_mut().current = "Family 40".into();
        } else {
            h.state_mut().current = String::new();
        }
        h.run();
        open(&mut h);
        h.run();
        h.hover_at(h.get_by_label("Family 05").rect().center());
        for _ in 0..3 {
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, -3.0),
                phase: egui::TouchPhase::Move,
                modifiers: Modifiers::NONE,
            });
            h.run();
        }
        let scrolled = h.get_by_label("Family 00").rect();
        if missing {
            h.state_mut().families.push("Family 40".into());
        } else {
            h.state_mut().current = "Family 40".into();
        }
        h.run_steps(3);
        assert!(popup_rect(&h).is_some(), "the menu stays open");
        assert_eq!(h.get_by_label("Family 00").rect(), scrolled, "missing = {missing}: the list jumped");
    }
}

/// The same through the real options bar, where the pick itself records the recent family.
#[test]
fn options_bar_menu_arrows_walk_on_while_recording_recent_fonts() {
    use crate::{PhotocraftApp, Services};
    use serde_json::json;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).with_max_steps(SETTLE_STEPS).build_ui_state(
        |ui, app| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal(|ui| crate::type_tool::options_bar(app, ui));
            }
        },
        app,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    h.query_all_by_role(Role::ComboBox).next().unwrap().click();
    h.run();
    let mut seen = vec![h.state().ui.tool_options.type_font.clone()];
    for _ in 0..4 {
        h.key_press(Key::ArrowDown);
        // The open font menu repaints for its caret, so step a bounded number of times.
        h.run_steps(2);
        seen.push(h.state().ui.tool_options.type_font.clone());
    }
    let mut once = seen.clone();
    once.sort();
    once.dedup();
    assert_eq!(once.len(), seen.len(), "each ↓ reaches a new family: {seen:?}");
    assert_eq!(h.state().ui.recent_fonts.first(), seen.last(), "the picks are recorded");
}

#[test]
fn wheel_over_the_closed_font_button_steps_through_the_families() {
    let mut h = harness();
    let button = h.query_all_by_role(Role::ComboBox).next().unwrap().rect().center();
    h.hover_at(button);
    h.run();
    let wheel = |h: &mut Harness<'static, State>, y: f32| {
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run();
    };
    wheel(&mut h, -2.0);
    assert_eq!(h.state().current, "Family 02", "two notches down pick the second next family");
    assert!(popup_rect(&h).is_none(), "the list stays closed");
    wheel(&mut h, 1.0);
    assert_eq!(h.state().current, "Family 01");
    wheel(&mut h, 5.0);
    assert_eq!(h.state().current, "Family 00", "stops at the first family");
}

#[test]
fn wheel_over_the_closed_button_keeps_a_stable_order_with_recents() {
    let mut h = harness_with_recents(FontPreview::Medium, vec![]);
    let button = h.query_all_by_role(Role::ComboBox).next().unwrap().rect().center();
    h.hover_at(button);
    h.run();
    let wheel = |h: &mut Harness<'static, State>, y: f32| {
        h.event(egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: vec2(0.0, y),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run();
    };
    wheel(&mut h, -1.0);
    assert_eq!(h.state().current, "Family 01");
    // The caller (the options bar) records the picked family as the most recent one.
    h.state_mut().recents.insert(0, "Family 01".into());
    h.run();
    wheel(&mut h, -1.0);
    assert_eq!(h.state().current, "Family 02", "the next notch keeps going, not back to the recent family");
    h.state_mut().recents.insert(0, "Family 02".into());
    h.run();
    wheel(&mut h, -1.0);
    assert_eq!(h.state().current, "Family 03");
    assert!(popup_rect(&h).is_none(), "the list stays closed");
}

#[test]
fn wheel_family_index_clamps_to_the_ends() {
    assert_eq!(super::wheel_family_index(0, 3, -5.0), Some(2));
    assert_eq!(super::wheel_family_index(1, 3, 1.0), Some(0));
    assert_eq!(super::wheel_family_index(1, 3, 0.0), None);
    assert_eq!(super::wheel_family_index(0, 0, -1.0), None);
}

/// #5152: in the full app the Properties panel's Character section draws a second font picker
/// after the options bar; hovering a font in the options bar still previews it on the canvas.
#[test]
fn options_bar_hover_previews_with_the_character_picker_also_shown() {
    use crate::{PhotocraftApp, Services};
    use photocraft_doc::LayerContent;
    use serde_json::json;

    let theme = crate::theme::ThemeKind::Pro;
    let mut h = Harness::builder().with_size(vec2(1600.0, 1000.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
        app.ui.theme = theme;
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("type.create", json!({"text": "Sample", "font": "Inter", "size": 18})).unwrap();
        app.ui.tool = crate::state::Tool::Type;
        app.sync_views();
        app
    });
    h.run_steps(8);
    let layer = h.state().session.active().unwrap().active_layer.unwrap();
    let font = |h: &mut Harness<'static, PhotocraftApp>| {
        let (shown, _) = crate::canvas::display_doc(h.state_mut(), 0);
        let LayerContent::Text(t) = &shown.layer(layer).unwrap().content else { panic!("type layer") };
        t.char_runs()[0].style.font_family.clone()
    };
    let pickers: Vec<_> = h.query_all_by_role(Role::ComboBox).filter(|n| n.value().as_deref() == Some("Inter")).collect();
    assert_eq!(pickers.len(), 2, "options bar and Character section both show the font picker");
    let button = pickers[0].rect().center();
    hold_open(&mut h, button);
    h.query_all_by_role(Role::TextInput).find(|n| n.is_focused()).expect("search field focused").type_text("JetBrains Mono");
    h.run();
    h.get_by_label("JetBrains Mono").hover();
    h.run();
    assert_eq!(font(&mut h), "JetBrains Mono");
    h.query_all_by_role(Role::TextInput).find(|n| n.is_focused()).expect("search field focused").hover();
    h.run();
    assert_eq!(font(&mut h), "Inter", "leaving the row restores the committed font");
}
