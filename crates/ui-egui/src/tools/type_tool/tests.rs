use super::*;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.sync_views();
    app.ui.tool = crate::state::Tool::Type;
    app
}

/// Preferences ▸ Type ▸ Recent Fonts caps the recorded list; a re-pick moves to the front and
/// 0 keeps no list.
#[test]
fn note_recent_font_caps_and_de_duplicates() {
    let mut app = app();
    for f in ["A", "B", "C"] {
        app.note_recent_font(f);
    }
    assert_eq!(app.ui.recent_fonts, ["C", "B", "A"]);
    app.run("prefs.set", json!({"path": "type.recentFonts", "value": 2})).unwrap();
    app.note_recent_font("D");
    assert_eq!(app.ui.recent_fonts, ["D", "C"], "the cap trims the tail");
    app.note_recent_font("C");
    assert_eq!(app.ui.recent_fonts, ["C", "D"], "a re-pick moves to the front");
    app.run("prefs.set", json!({"path": "type.recentFonts", "value": 0})).unwrap();
    app.note_recent_font("E");
    assert!(app.ui.recent_fonts.is_empty(), "0 keeps no list");
}

/// A smaller Recent Fonts cap (or 0) applies to the existing list at once, without waiting
/// for the next pick (CodeRabbit review of #3559).
#[test]
fn lowering_the_recent_fonts_cap_applies_at_once() {
    let mut app = app();
    for f in ["A", "B", "C"] {
        app.note_recent_font(f);
    }
    app.run("prefs.set", json!({"path": "type.recentFonts", "value": 2})).unwrap();
    app.sync_recent_fonts();
    assert_eq!(app.ui.recent_fonts, ["C", "B"], "the smaller cap trims the tail without a pick");
    app.run("prefs.set", json!({"path": "type.recentFonts", "value": 0})).unwrap();
    app.sync_recent_fonts();
    assert!(app.ui.recent_fonts.is_empty(), "0 clears the list");
}

fn layer_text(app: &PhotocraftApp) -> String {
    let ed = app.ui.text_edit.as_ref().unwrap();
    current_text(app, LayerId(ed.layer)).unwrap()
}

/// A committed type layer reading `s` at the same spot as the others (stacked variants, #4320).
fn stacked_variant(app: &mut PhotocraftApp, s: &str) -> LayerId {
    app.ui.text_edit = None;
    pointer_up(app, [50.0, 100.0], [50.0, 100.0]);
    insert(app, s);
    commit(app);
    app.session.active().unwrap().active_layer.unwrap()
}

fn hide(app: &mut PhotocraftApp, id: LayerId) {
    app.run("layer.setProps", json!({"layer": id.0, "visible": false})).unwrap();
}

/// Wraps `id` in a new group and returns the group.
fn group_of(app: &mut PhotocraftApp, id: LayerId) -> LayerId {
    app.run("layer.select", json!({"layer": id.0, "mode": "replace"})).unwrap();
    app.run("layer.groupLayers", json!({})).unwrap();
    let doc = &app.session.active().unwrap().doc;
    let path = doc.path_of(id).unwrap();
    doc.layer_at(&path[..path.len() - 1]).unwrap().id
}

/// Three type variants at one spot, bottom to top: A, B, C.
fn variants() -> (PhotocraftApp, [LayerId; 3]) {
    let mut app = app();
    let ids = ["Variant A", "Variant B", "Variant C"].map(|s| stacked_variant(&mut app, s));
    (app, ids)
}

const ON_TEXT: (f64, f64) = (60.0, 95.0);

/// #4320: variants hidden through their groups above the shown one are not picked.
#[test]
fn type_click_skips_text_in_hidden_groups_above_the_shown_variant() {
    let (mut app, [a, b, c]) = variants();
    for id in [b, c] {
        let g = group_of(&mut app, id);
        hide(&mut app, g);
    }
    assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(a));
    assert!(pointer_down(&mut app, ON_TEXT.0, ON_TEXT.1, egui::Modifiers::NONE));
    assert_eq!(app.ui.text_edit.as_ref().map(|e| e.layer), Some(a.0), "the click edits the shown variant");
}

#[test]
fn type_click_skips_text_in_a_nested_hidden_group() {
    let (mut app, [a, b, c]) = variants();
    hide(&mut app, b);
    let inner = group_of(&mut app, c);
    let outer = group_of(&mut app, inner);
    hide(&mut app, outer);
    assert!(app.session.active().unwrap().doc.layer(c).unwrap().visible, "only the outer group is hidden");
    assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(a));
}

#[test]
fn type_click_skips_own_hidden_variants_above_and_below() {
    let (mut app, [a, b, c]) = variants();
    hide(&mut app, b);
    hide(&mut app, c);
    assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(a), "hidden variants above");
    let (mut app, [a, b, c]) = variants();
    hide(&mut app, a);
    hide(&mut app, b);
    assert_eq!(hit_layer(&mut app, ON_TEXT.0, ON_TEXT.1), Some(c), "hidden variants below");
}

/// The Move tool's double-click on type (#1132) shares the pick.
#[test]
fn move_double_click_edits_the_shown_variant_not_one_in_a_hidden_group() {
    let (mut app, [a, b, c]) = variants();
    for id in [b, c] {
        let g = group_of(&mut app, id);
        hide(&mut app, g);
    }
    app.ui.tool = crate::state::Tool::Move;
    assert!(edit_at(&mut app, ON_TEXT.0, ON_TEXT.1));
    assert_eq!(app.ui.text_edit.as_ref().map(|e| e.layer), Some(a.0));
}

/// Photoshop: a click on existing type edits it; a ⇧-click there starts a new type layer.
#[test]
fn shift_click_on_existing_type_starts_new_text() {
    let mut app = app();
    pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
    insert(&mut app, "Existing words");
    commit(&mut app);
    let existing = app.session.active().unwrap().doc.layers.last().unwrap().id;
    let (x, y) = (60.0, 95.0);
    assert_eq!(hit_layer(&mut app, x, y), Some(existing), "the click lands on the existing text");
    assert!(!pointer_down(&mut app, x, y, egui::Modifiers::SHIFT), "⇧: not consumed, so a new type layer starts");
    assert!(app.ui.text_edit.is_none());
    assert!(pointer_down(&mut app, x, y, egui::Modifiers::NONE), "a plain click edits the existing text");
    assert_eq!(app.ui.text_edit.as_ref().map(|e| e.layer), Some(existing.0));
}

#[test]
fn click_creates_placeholder_selected_and_typing_replaces_it() {
    let mut app = app();
    assert!(!pointer_down(&mut app, 50.0, 100.0, egui::Modifiers::NONE));
    pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
    let ed = app.ui.text_edit.clone().unwrap();
    assert_eq!((ed.anchor, ed.caret), (0, PLACEHOLDER.chars().count()));
    insert(&mut app, "Héllo");
    insert(&mut app, " world");
    assert_eq!(layer_text(&app), "Héllo world");
    // Creating and typing share the session key: one step after the initial snapshot.
    let steps = app.session.active().unwrap().history.entries();
    assert_eq!(steps, ["Open", "New Type Layer"]);
    commit(&mut app);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(doc.layers.last().unwrap().name, "Héllo world");
    assert!(app.ui.text_edit.is_none());
}

// Independent perimeter oracle for the rectangle used by the endpoint hit tests.
fn closed_path_endpoint_test_point(s: f64) -> (f64, f64) {
    if s < 200.0 {
        (20.0 + s, 20.0)
    } else if s < 300.0 {
        (220.0, 20.0 + s - 200.0)
    } else if s < 500.0 {
        (220.0 - (s - 300.0), 120.0)
    } else {
        (20.0, 120.0 - (s - 500.0))
    }
}

#[test]
fn closed_path_forward_hit_after_last_caret() {
    for align in ["left", "center", "right"] {
        let mut app = app();
        app.ui.tool_options.type_align = align.into();
        let path = photocraft_vector::shapes::rect(20.0, 20.0, 200.0, 100.0);
        app.run("path.set", json!({"path": photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
        pointer_up(&mut app, [20.0, 45.0], [20.0, 45.0]);
        insert(&mut app, "ssssssssssss");
        let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        let (l, _, _) = layout(&mut app, id).unwrap();
        let (last, _, _) = l.caret(12);
        let s = (575.0 + f64::from(last) + 2.0).rem_euclid(600.0);
        let (x, y) = closed_path_endpoint_test_point(s);
        assert_eq!(offset_at(&mut app, id, x, y), 12, "{align}");
    }
}

#[test]
fn open_path_hits_keep_endpoint_clamping() {
    let mut app = app();
    let path = photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polyline(&[(20.0, 100.0), (380.0, 100.0)])]);
    app.run("path.set", json!({"path": photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
    pointer_up(&mut app, [200.0, 100.0], [200.0, 100.0]);
    insert(&mut app, "ssss");
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    assert_eq!(offset_at(&mut app, id, 198.0, 100.0), 0);
    assert_eq!(offset_at(&mut app, id, 20.0, 100.0), 0);
    assert_eq!(offset_at(&mut app, id, 380.0, 100.0), 4);
}

#[test]
fn closed_path_backward_hit_before_first_caret() {
    for align in ["left", "center", "right"] {
        let mut app = app();
        app.ui.tool_options.type_align = align.into();
        let path = photocraft_vector::shapes::rect(20.0, 20.0, 200.0, 100.0);
        app.run("path.set", json!({"path": photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
        pointer_up(&mut app, [20.0, 45.0], [20.0, 45.0]);
        insert(&mut app, "ssssssssssss");
        let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        let (l, _, _) = layout(&mut app, id).unwrap();
        // The rectangle runs clockwise; the text origin is perimeter distance 575.
        // Project 2 px backward from the first caret, using an independent oracle.
        let (first, _, _) = l.caret(0);
        let s = (575.0 + f64::from(first) - 2.0).rem_euclid(600.0);
        let (x, y) = closed_path_endpoint_test_point(s);
        assert_eq!(offset_at(&mut app, id, x, y), 0, "{align}");
    }
}

#[test]
fn closed_path_upstream_caret_rectangle_left_edge_roundtrip() {
    let mut app = app();
    let path = photocraft_vector::shapes::rect(20.0, 20.0, 200.0, 100.0);
    app.run("path.set", json!({"path": photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
    pointer_up(&mut app, [20.0, 45.0], [20.0, 45.0]);
    insert(&mut app, "ssssssssssss");
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    let (layout, _, _) = layout(&mut app, id).unwrap();
    let shape = text_layer(&app.session.active().unwrap().doc, id).unwrap().shape.clone();
    let photocraft_doc::text::TextShape::Path { start, .. } = shape else {
        assert!(matches!(shape, photocraft_doc::text::TextShape::Path { .. }), "outline click must attach path text");
        return;
    };
    // Independent rectangle perimeter oracle: never use the warp under test to
    // generate expected caret coordinates. Start is 25 px before the top-left seam.
    assert!((f64::from(start) - 575.0).abs() < 0.5);
    let mut tested_after_seam = false;
    for index in [1, 4, 7, 10] {
        let (x, _, _) = layout.caret(index);
        let distance = f64::from(start) + f64::from(x);
        tested_after_seam |= distance > 600.0;
        let s = distance.rem_euclid(600.0);
        let point = if s < 200.0 {
            Point::new(20.0 + s, 20.0)
        } else if s < 300.0 {
            Point::new(220.0, 20.0 + s - 200.0)
        } else if s < 500.0 {
            Point::new(220.0 - (s - 300.0), 120.0)
        } else {
            Point::new(20.0, 120.0 - (s - 500.0))
        };
        // The rectangle oracle already returns document coordinates.
        let doc = point;
        assert_eq!(offset_at(&mut app, id, doc.x, doc.y), index, "caret round trip at {index}");
    }
    assert!(tested_after_seam, "fixture must cross the seam");
}

/// TYPE-218-1: a Type tool click on the work path starts type on the path from the point
/// clicked, in one history step with the placeholder selected; a click off it is point text.
#[test]
fn click_on_a_path_starts_type_on_the_path() {
    use photocraft_doc::text::TextShape;
    let mut app = app();
    app.session
        .edit("path", |doc, _| {
            doc.work_path = Some(photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polyline(&[(20.0, 100.0), (380.0, 100.0)])]));
            Ok(())
        })
        .unwrap();
    assert!(matches!(path_target(&app, 200.0, 101.0), Some(PathClick::OnPath { .. })));
    assert!(path_target(&app, 200.0, 160.0).is_none(), "off the path");
    let before = app.session.active().unwrap().history.entries().len();
    assert!(!pointer_down(&mut app, 200.0, 101.0, egui::Modifiers::NONE));
    pointer_up(&mut app, [200.0, 101.0], [200.0, 101.0]);
    let ed = app.ui.text_edit.clone().unwrap();
    assert_eq!((ed.anchor, ed.caret), (0, PLACEHOLDER.chars().count()), "the placeholder is selected");
    let shape = text_layer(&app.session.active().unwrap().doc, LayerId(ed.layer)).unwrap().shape.clone();
    assert!(matches!(shape, TextShape::Path { start, flip: false, .. } if (start - 180.0).abs() < 0.5), "{shape:?}");
    assert_eq!(app.session.active().unwrap().history.entries().len(), before + 1, "one step for the click");
    insert(&mut app, "On the line");
    assert_eq!(layer_text(&app), "On the line");
    commit(&mut app);
    // With the vertical tool a click does not start path type.
    app.ui.tool = crate::state::Tool::VerticalType;
    assert!(!matches!(path_target(&app, 200.0, 101.0), Some(PathClick::OnPath { .. })));
    app.ui.tool = crate::state::Tool::Type;
    pointer_up(&mut app, [200.0, 160.0], [200.0, 160.0]);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    assert_eq!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, TextShape::Point);
}

/// A click in type on a path puts the caret at the glyph there along the curve, not where the
/// straight line would have it.
#[test]
fn caret_hits_type_on_a_path_along_the_curve() {
    let mut app = app();
    app.session
        .edit("path", |doc, _| {
            doc.work_path = Some(photocraft_doc::vector::Path::new(vec![photocraft_doc::vector::Subpath::polyline(&[(300.0, 10.0), (300.0, 195.0)])]));
            Ok(())
        })
        .unwrap();
    pointer_up(&mut app, [300.0, 20.0], [300.0, 20.0]);
    insert(&mut app, "ABCDEFGH");
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    assert!(matches!(text_layer(&app.session.active().unwrap().doc, id).unwrap().shape, photocraft_doc::text::TextShape::Path { .. }));
    // The type runs down the path: near its start the caret is at the beginning, far down at the end.
    assert_eq!(offset_at(&mut app, id, 300.0, 18.0), 0);
    // (The straight line would put both of these at the start: they are left of its first glyph.)
    let (mid, far) = (offset_at(&mut app, id, 296.0, 60.0), offset_at(&mut app, id, 296.0, 190.0));
    assert!(0 < mid && mid < far, "{mid} {far}");
}

/// Preferences ▸ Units & Rulers ▸ Type switches the display unit of type metrics: points by
/// default, pixels by the document's resolution, or millimetres.
#[test]
fn type_unit_follows_the_units_and_rulers_preference() {
    let mut app = app();
    assert_eq!(type_unit(&app), (1.0, "pt"));
    app.run("prefs.set", json!({"path": "unitsAndRulers.typeUnits", "value": "pixels"})).unwrap();
    assert_eq!(type_unit(&app), (1.0, "px"), "a 72 ppi document: 1 pt is 1 px");
    app.run("image.imageSize", json!({"resolution": 144, "resample": "none"})).unwrap();
    assert_eq!(type_unit(&app), (2.0, "px"), "144 ppi: a point is two pixels");
    app.run("prefs.set", json!({"path": "unitsAndRulers.typeUnits", "value": "mm"})).unwrap();
    assert!((type_unit(&app).0 - 25.4 / 72.0).abs() < 1e-6, "millimetres ignore the resolution");
}

/// Preferences ▸ Type ▸ Fill new type layers with placeholder text: off, a click creates an
/// empty layer (nothing selected) and committing without typing removes it again.
#[test]
fn placeholder_preference_controls_new_type_layer_text() {
    let mut app = app();
    app.run("prefs.set", json!({"path": "type.fillNewTypeLayersWithPlaceholder", "value": false})).unwrap();
    pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
    let ed = app.ui.text_edit.clone().unwrap();
    assert_eq!((ed.anchor, ed.caret), (0, 0), "nothing to select");
    assert_eq!(layer_text(&app), "");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 2, "the layer was created");
    commit(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), 1, "an empty type layer is removed on commit");
    assert!(app.ui.text_edit.is_none());
}

/// Pure smart-quote mapping: opening after whitespace, a bracket or a dash, closing next to a
/// letter or another quote.
#[test]
fn smart_quotes_are_typographic() {
    assert_eq!(smart_quotes("\"Hello\"", None), "\u{201c}Hello\u{201d}");
    assert_eq!(smart_quotes("don't", None), "don\u{2019}t");
    assert_eq!(smart_quotes("(\"x\")", None), "(\u{201c}x\u{201d})");
    assert_eq!(smart_quotes("--'x'", None), "--\u{2018}x\u{2019}", "a dash stays a dash, the quote opens after it");
    assert_eq!(smart_quotes("\"\"", None), "\u{201c}\u{201d}");
}

/// Preferences ▸ Type ▸ Use Smart Quotes switches typed quotes (typed text only, not paste).
#[test]
fn smart_quotes_preference_controls_typed_quotes() {
    for (smart, expected) in [(true, "hi \u{201c}you\u{201d}"), (false, "hi \"you\"")] {
        let mut app = app();
        app.run("prefs.set", json!({"path": "type.smartQuotes", "value": smart})).unwrap();
        pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
        insert_typed(&mut app, "hi \"you\"");
        assert_eq!(layer_text(&app), expected, "smartQuotes={smart}");
    }
}

#[test]
fn a_name_from_text_after_blank_lines_keeps_following_the_text() {
    let mut app = app();
    pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
    insert(&mut app, "\n\nTitle");
    let id = app.ui.text_edit.as_ref().unwrap().layer;
    commit(&mut app);
    let name = |app: &PhotocraftApp| app.session.active().unwrap().doc.layers.last().unwrap().name.clone();
    assert_eq!(name(&app), "Title");
    // Edited later, outside the session that created it (#483).
    app.run("type.edit", json!({"layer": id, "text": "\nSubtitle"})).unwrap();
    assert_eq!(name(&app), "Subtitle");
}

#[test]
fn dragging_a_box_handle_resizes_the_paragraph_box() {
    let mut app = app();
    pointer_up(&mut app, [10.0, 10.0], [110.0, 60.0]);
    let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
    let steps = app.session.active().unwrap().history.entries().len();
    // Top-left corner: the box keeps its bottom-right corner.
    assert!(pointer_down(&mut app, 10.0, 10.0, egui::Modifiers::NONE));
    assert_eq!(app.ui.text_edit.as_ref().unwrap().resize.map(|r| r.handle), Some(0));
    pointer_move(&mut app, 20.0, 25.0, egui::Modifiers::NONE);
    pointer_move(&mut app, 30.0, 30.0, egui::Modifiers::NONE);
    pointer_up(&mut app, [10.0, 10.0], [30.0, 30.0]);
    assert_eq!(box_shape(&app, id), Some((0.0, 0.0, 80.0, 30.0)));
    let aff = layout(&mut app, id).unwrap().1;
    assert_eq!((aff.m[4], aff.m[5]), (30.0, 30.0));
    // The whole drag is one history step, and the edit session survives it.
    assert_eq!(app.session.active().unwrap().history.entries().len(), steps);
    assert!(app.ui.text_edit.is_some());
    // Right edge: only the width changes, and never below the minimum.
    assert!(pointer_down(&mut app, 110.0, 45.0, egui::Modifiers::NONE));
    pointer_move(&mut app, 0.0, 99.0, egui::Modifiers::NONE);
    pointer_up(&mut app, [110.0, 45.0], [0.0, 99.0]);
    assert_eq!(box_shape(&app, id), Some((0.0, 0.0, MIN_BOX, 30.0)));
    // ⌥ mirrors the drag about the centre: the left edge follows the right one out.
    assert!(pointer_down(&mut app, 34.0, 45.0, egui::Modifiers::ALT));
    pointer_move(&mut app, 70.0, 45.0, egui::Modifiers::ALT);
    pointer_up(&mut app, [34.0, 45.0], [70.0, 45.0]);
    assert_eq!(box_shape(&app, id), Some((0.0, 0.0, MIN_BOX + 72.0, 30.0)));
    let aff = layout(&mut app, id).unwrap().1;
    // Text space −36..40 (centre 2) through the translation: the doc centre stays at 32.
    assert_eq!((aff.m[4], aff.m[5]), (-6.0, 30.0));
}

#[test]
fn ime_preedit_is_replaced_by_each_update_and_commit_finalises() {
    let mut app = app();
    pointer_up(&mut app, [50.0, 100.0], [50.0, 100.0]);
    // Each preedit update replaces the previous one (and the placeholder selection first).
    ime_update(&mut app, "に", false);
    assert_eq!(layer_text(&app), "に");
    ime_update(&mut app, "にほん", false);
    ime_update(&mut app, "にほんご", false);
    assert_eq!(layer_text(&app), "にほんご");
    assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, Some((0, 4)));
    ime_update(&mut app, "日本語", true);
    assert_eq!(layer_text(&app), "日本語");
    let ed = app.ui.text_edit.clone().unwrap();
    assert_eq!((ed.caret, ed.anchor, ed.preedit), (3, 3, None));
    // The next composition starts after the committed text; cancelling removes it again.
    ime_update(&mut app, "あ", false);
    assert_eq!(layer_text(&app), "日本語あ");
    ime_update(&mut app, "", false);
    assert_eq!(layer_text(&app), "日本語");
    assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, None);
    // A stale preedit range (text shortened by undo) must not replace committed text; the
    // composition is invalidated and its pending commit is dropped.
    app.ui.text_edit.as_mut().unwrap().preedit = Some((10, 5));
    ime_update(&mut app, "x", true);
    assert_eq!(layer_text(&app), "日本語");
    assert_eq!(app.ui.text_edit.as_ref().unwrap().preedit, None);
}

#[test]
fn empty_new_layer_is_deleted_on_commit() {
    let mut app = app();
    pointer_up(&mut app, [10.0, 50.0], [10.0, 50.0]);
    let n = app.session.active().unwrap().doc.layers.len();
    insert(&mut app, "");
    if let Some(e) = app.ui.text_edit.as_mut() {
        e.anchor = 0;
    }
    insert(&mut app, "");
    commit(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), n - 1);
}

#[test]
fn word_boundaries() {
    assert_eq!(word_boundary("hello big world", 0, true), 5);
    assert_eq!(word_boundary("hello big world", 7, false), 6);
    assert_eq!(word_boundary("hello big world", 15, false), 10);
}
#[test]
fn arrow_keys_reset_caret_blink_phase() {
    let mut app = app();
    pointer_up(&mut app, [10.0, 50.0], [10.0, 50.0]);
    insert(&mut app, "Hello");
    app.ui.caret_blink_time = 0.0;
    let ctx = egui::Context::default();

    let has_caret = |app: &mut PhotocraftApp, ctx: &egui::Context, time: f64| -> bool {
        let mut out = ctx.run_ui(egui::RawInput { time: Some(time), ..Default::default() }, |ui| {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(400.0, 400.0));
            let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(rect);
            let xf = ViewXform { rect, zoom: 1.0, center: [100.0, 100.0], flip: false, rotation: 0.0, aspect: 1.0 };
            draw_overlay(app, &painter, &xf);
        });
        out.textures_delta.clear();
        out.shapes.iter().any(|s| matches!(&s.shape, egui::Shape::LineSegment { stroke, .. } if CARET_STROKES.contains(stroke)))
    };

    // Before resetting, the caret is in the off phase at t = 10.5 s (10500 / 530 = 19, odd phase)
    assert!(!has_caret(&mut app, &ctx, 10.5), "caret should be off at t = 10.5 with phase 0.0");

    ctx.begin_pass(egui::RawInput {
        time: Some(10.5),
        events: vec![egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    });
    handle_keys(&mut app, &ctx);
    assert_eq!(app.ui.caret_blink_time, 10.5);

    // draw_overlay paints the solid caret in the new phase
    assert!(has_caret(&mut app, &ctx, 10.5), "caret should be painted right after arrow key");
}

#[test]
fn pointer_up_click_resets_caret_blink_phase() {
    let mut app = app();
    pointer_up(&mut app, [10.0, 50.0], [10.0, 50.0]);
    insert(&mut app, "Hello");
    app.ui.caret_blink_time = 0.0;
    app.last_frame_time = 12.3;
    pointer_up(&mut app, [10.0, 50.0], [10.0, 50.0]);
    assert_eq!(app.ui.caret_blink_time, 12.3);
}

#[test]
fn double_click_from_move_tool_restores_move_tool_on_commit_and_cancel() {
    let (mut app, _) = variants();
    app.ui.tool = crate::state::Tool::Move;
    assert!(edit_at(&mut app, ON_TEXT.0, ON_TEXT.1));
    assert_eq!(app.ui.tool, crate::state::Tool::Type);
    assert_eq!(app.ui.text_edit_return_tool, Some(crate::state::Tool::Move));
    commit(&mut app);
    assert_eq!(app.ui.tool, crate::state::Tool::Move, "commit restores the Move tool");
    assert_eq!(app.ui.text_edit_return_tool, None);

    // Also test cancel
    app.ui.tool = crate::state::Tool::Move;
    assert!(edit_at(&mut app, ON_TEXT.0, ON_TEXT.1));
    assert_eq!(app.ui.tool, crate::state::Tool::Type);
    cancel(&mut app);
    assert_eq!(app.ui.tool, crate::state::Tool::Move, "cancel restores the Move tool");
    assert_eq!(app.ui.text_edit_return_tool, None);

    // Switching tools mid-edit commits without overriding the newly chosen tool
    app.ui.tool = crate::state::Tool::Move;
    assert!(edit_at(&mut app, ON_TEXT.0, ON_TEXT.1));
    assert_eq!(app.ui.tool, crate::state::Tool::Type);
    assert_eq!(app.ui.text_edit_return_tool, Some(crate::state::Tool::Move));
    app.ui.tool = crate::state::Tool::Brush;
    commit(&mut app);
    assert_eq!(app.ui.tool, crate::state::Tool::Brush, "switching tools keeps the chosen tool");
    assert_eq!(app.ui.text_edit_return_tool, None);

    // Layers-thumbnail edit stays on Type tool (matches Photoshop)
    app.ui.tool = crate::state::Tool::Move;
    edit_active(&mut app).unwrap();
    assert_eq!(app.ui.tool, crate::state::Tool::Type);
    assert_eq!(app.ui.text_edit_return_tool, None);
    commit(&mut app);
    assert_eq!(app.ui.tool, crate::state::Tool::Type, "Layers-thumbnail edit stays on Type tool");
}

#[test]
fn closed_path_aligned_selection_highlights_every_visible_cluster() {
    for align in ["center", "right"] {
        let mut app = app();
        app.ui.tool_options.type_align = align.into();
        let path = photocraft_vector::shapes::rect(20.0, 20.0, 200.0, 100.0);
        app.run("path.set", json!({"path":photocraft_engine::vector_cmds::path_json(&path)})).unwrap();
        pointer_up(&mut app, [20.0, 45.0], [20.0, 45.0]);
        insert(&mut app, "ssssssssssss");
        let id = LayerId(app.ui.text_edit.as_ref().unwrap().layer);
        let (l, _, _) = layout(&mut app, id).unwrap();
        assert!(l.bounds().unwrap()[0] < 0.0, "{align} must exercise negative text coordinates");
        let text_layer = text_layer(&app.session.active().unwrap().doc, id).unwrap();
        let rendered = photocraft_text::render::layout_deform(&l, text_layer).unwrap();
        let visible = l.clusters.iter().filter(|c| rendered.place_glyph(f64::from(c.x + c.advance / 2.0)) != photocraft_text::warp::Placement::Hidden).count();
        assert_eq!(visible, l.clusters.len());
        let ed = app.ui.text_edit.as_mut().unwrap();
        ed.anchor = 0;
        ed.caret = 12;
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
            let painter = ui.ctx().layer_painter(egui::LayerId::background()).with_clip_rect(rect);
            let xf = ViewXform { rect, zoom: 1.0, center: [120.0, 70.0], flip: false, rotation: 0.0, aspect: 1.0 };
            draw_overlay(&mut app, &painter, &xf);
        });
        output.textures_delta.clear();
        let highlights = output.shapes.iter().filter(|s| matches!(&s.shape, egui::Shape::Path(p) if p.closed && p.fill.a() == 110)).count();
        assert_eq!(highlights, visible, "{align}: every visible selected cluster needs its highlight");
    }
}
