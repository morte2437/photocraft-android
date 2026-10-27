use super::*;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::{Color, ColorMode, SampleType, Size};
use photocraft_geom::Rect as GRect;

/// 40 × 30: red block (0..20, 0..15), blue block (20..40, 0..15), black and white below.
fn app_with_doc() -> PhotocraftApp {
    let mut doc = Document::with_background("cr", Size::new(40, 30), ColorMode::Rgb, SampleType::U8, Color::WHITE);
    let bg = doc.layers[0].surface_mut().unwrap();
    bg.fill_rect(GRect::new(0, 0, 20, 15), &[1.0, 0.0, 0.0, 1.0]);
    bg.fill_rect(GRect::new(20, 0, 40, 15), &[0.0, 0.0, 1.0, 1.0]);
    bg.fill_rect(GRect::new(0, 15, 20, 30), &[0.0, 0.0, 0.0, 1.0]);
    let mut s = photocraft_engine::Session::new();
    s.add_document(doc, None);
    PhotocraftApp::new(s, crate::Services::default())
}

fn coverage(app: &PhotocraftApp, x: i32, y: i32) -> f32 {
    app.session.active().unwrap().doc.selection.as_ref().map_or(0.0, |s| s.sample_channel(x, y, 0))
}

fn harness(app: PhotocraftApp) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1200.0, 900.0)).build_ui_state(
        |ui, app| {
            crate::menus::menu_bar(app, ui);
            crate::dialogs::show(app, ui.ctx());
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(2);
    h
}

fn dialog_id(app: &PhotocraftApp) -> u64 {
    app.ui.dialogs.iter().find(|d| owns(&d.fields)).map(|d| d.id).expect("Color Range dialog open")
}

fn set(h: &mut Harness<'static, PhotocraftApp>, key: &str, v: Value) {
    let id = dialog_id(h.state());
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert(key.into(), v);
    h.run_steps(3);
}

/// OK / Cancel are painted buttons without accessibility labels; the dialog's keys do the same.
fn ok(h: &mut Harness<'static, PhotocraftApp>) {
    h.key_press(egui::Key::Enter);
}

fn click(h: &mut Harness<'static, PhotocraftApp>, at: egui::Pos2) {
    h.hover_at(at);
    h.run_steps(1);
    h.drag_at(at);
    h.run_steps(1);
    h.drop_at(at);
}

fn disabled(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
    // A slider row names both its number and its slider with the caption; either reports it.
    h.query_all_by_label(label).next().unwrap_or_else(|| panic!("`{label}`")).accesskit_node().is_disabled()
}

#[test]
fn preview_and_eyedropper_keep_the_original_selection_boundary() {
    let mut app = app_with_doc();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    for params in [json!({"select": "sampledColors", "color": "#ff0000", "fuzziness": 0}), json!({"select": "reds"})] {
        let mask = proxy_mask(&app, &doc, 1, &params).unwrap();
        assert_eq!(mask[5 * 40 + 5], 1.0, "red inside the selection stays visible");
        assert_eq!(mask[5 * 40 + 25], 0.0, "the blue outside the selection stays excluded");
    }
    for params in [json!({"select": "sampledColors", "color": "#ffffff", "invert": true}), json!({"select": "outOfGamut", "invert": true})] {
        let mask = proxy_mask(&app, &doc, 1, &params).unwrap();
        assert_eq!(mask[5 * 40 + 25], 0.0, "even inverted previews stay within the selection");
    }

    // As in Photoshop, the eyedropper samples anywhere; only the result is scoped (#5244).
    open(&mut app);
    assert!(pick_top(&mut app, [25.0, 5.0], egui::Modifiers::NONE));
    assert_eq!(points(&app.ui.dialogs.last().unwrap().fields, "points"), vec![[25.0, 5.0]], "a colour outside the selection is sampled");
    let mask = proxy_mask(&app, &doc, 1, &json!({"select": "sampledColors", "points": [[25, 5]], "fuzziness": 0})).unwrap();
    assert_eq!(mask[5 * 40 + 25], 0.0, "but the preview stays within the selection");
    assert!(pick_top(&mut app, [5.0, 5.0], egui::Modifiers::NONE));
    assert_eq!(points(&app.ui.dialogs.last().unwrap().fields, "points"), vec![[5.0, 5.0]]);
}

#[test]
fn preview_cache_refreshes_after_foreground_and_proof_changes() {
    let mut app = app_with_doc();
    let p = json!({"select": "outOfGamut"});
    let mut previous = query_key(&app, &p);
    app.session.tools.foreground = [0.2, 0.3, 0.4, 1.0];
    let key = query_key(&app, &p);
    assert_ne!(key, previous);
    previous = key;
    for setup in [
        json!({"profile": "srgb"}),
        json!({"profile": "linear-srgb"}),
        json!({"profile": "linear-srgb", "intent": "perceptual"}),
        json!({"profile": "linear-srgb", "intent": "perceptual", "bpc": false}),
        json!({"profile": "linear-srgb", "intent": "perceptual", "bpc": false, "simulatePaper": true}),
    ] {
        app.run("view.proofSetup", setup).unwrap();
        let key = query_key(&app, &p);
        assert_ne!(key, previous, "changed proof settings must invalidate the preview");
        assert_eq!(key, query_key(&app, &p), "an unchanged ICC profile must keep its cache key");
        previous = key;
    }
    app.run("view.gamutWarning", json!({"on": true, "threshold": 8})).unwrap();
    assert_ne!(previous, query_key(&app, &p));
    previous = query_key(&app, &p);
    app.run("view.gamutWarning", json!({"on": true, "threshold": 16})).unwrap();
    assert_ne!(previous, query_key(&app, &p));
    assert_ne!(query_key(&app, &p), query_key(&app, &json!({"select": "outOfGamut", "invert": true})));
}

#[test]
fn eyedropper_hover_reuses_preview_textures_without_sampling() {
    let mut h = Harness::builder().with_size(vec2(1200.0, 900.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app_with_doc()
    });
    h.state_mut().ui.views[0].zoom = 20.0;
    h.state_mut().ui.views[0].center = [20.0, 15.0];
    h.state_mut().ui.views[0].fit_pending = false;
    h.run_steps(3);
    let id = open(h.state_mut());
    set(&mut h, "__selectionPreview", json!("quickMask"));
    let fields = h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields.clone();
    let cached = |app: &PhotocraftApp| {
        let p = app.color_range.as_ref().unwrap();
        (p.mask_key, p.overlay_key, p.mask.as_ref().unwrap().id(), p.overlay.as_ref().unwrap().id(), Arc::as_ptr(&p.proxy))
    };
    let before = cached(h.state());
    let st = h.state().session.active().unwrap();
    let document = (st.revision, st.history.past_len());
    for i in 0..12 {
        let at = crate::canvas::ViewXform::active(h.state()).unwrap().to_screen(4.0 + i as f32 * 0.5, 15.0);
        h.hover_at(at);
        h.run_steps(1);
        assert_eq!(cached(h.state()), before, "hover must reuse the mask, overlay and proxy");
    }
    assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields, fields, "hover must not pick a colour");
    let st = h.state().session.active().unwrap();
    assert_eq!((st.revision, st.history.past_len()), document);
}

#[test]
fn opens_from_the_select_menu_without_touching_the_document() {
    let mut h = harness(app_with_doc());
    let rev = h.state().session.active().unwrap().revision;
    h.get_by_label("Select").click();
    h.run_steps(2);
    h.get_by_label("Color Range…").click();
    h.run_steps(3);
    assert!(h.state().ui.dialogs.iter().any(|d| owns(&d.fields)), "Select › Color Range… must open the dialog");
    for label in ["Color Range", "Fuzziness:", "Localized Color Clusters", "Invert", "Selection", "Color Range preview", "Add to Sample"] {
        assert!(h.query_all_by_label(label).next().is_some(), "missing {label}");
    }
    assert_eq!(h.query_all_by_label("Image").count(), 2, "the Image menu and the Image preview radio");
    assert_eq!(h.state().session.active().unwrap().revision, rev, "opening must not edit the document");
    // Automation and the generic command dialog get the same dialog.
    let ctx = egui::Context::default();
    let r = crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    assert!(r.get("dialog").is_some(), "{r}");
}

#[test]
fn select_mode_switches_the_enabled_controls() {
    let mut h = harness(app_with_doc());
    open(h.state_mut());
    h.run_steps(4);
    // Sampled Colors: Fuzziness, eyedroppers and Localized Color Clusters; Range only when localized.
    assert!(!disabled(&h, "Fuzziness:") && !disabled(&h, "Localized Color Clusters") && !disabled(&h, "Add to Sample"));
    assert!(h.query_all_by_label("Range:").next().is_none());
    h.get_by_label("Localized Color Clusters").click();
    h.run_steps(2);
    assert!(h.query_all_by_label("Range:").next().is_some(), "Range shows with Localized Color Clusters");
    // A hue family: no Fuzziness, eyedroppers or clusters (Photoshop disables them).
    set(&mut h, "select", json!("reds"));
    assert!(disabled(&h, "Fuzziness:") && disabled(&h, "Localized Color Clusters") && disabled(&h, "Add to Sample"));
    assert!(h.query_all_by_label("Range:").next().is_none());
    assert!(!disabled(&h, "Invert"));
    // Tones: Fuzziness and the tonal range.
    set(&mut h, "select", json!("shadows"));
    assert!(!disabled(&h, "Fuzziness:") && disabled(&h, "Localized Color Clusters"));
    assert!(h.query_all_by_label("Shadows up to:").next().is_some());
    set(&mut h, "select", json!("midtones"));
    assert!(h.query_all_by_label("Midtones from:").next().is_some() && h.query_all_by_label("Midtones to:").next().is_some());
    set(&mut h, "select", json!("outOfGamut"));
    assert!(disabled(&h, "Fuzziness:") && h.query_all_by_label("Midtones from:").next().is_none());
    // Skin Tones: Fuzziness (the colour one), no eyedroppers or clusters.
    set(&mut h, "select", json!("skinTones"));
    assert!(!disabled(&h, "Fuzziness:") && disabled(&h, "Localized Color Clusters") && disabled(&h, "Add to Sample"));
    let skin: Map<String, Value> = serde_json::from_value(json!({"select": "skinTones", "fuzziness": 25.0, "points": [[1, 1]]})).unwrap();
    assert_eq!(params(&skin), json!({"select": "skinTones", "invert": false, "fuzziness": 25.0}));
    assert_eq!(
        controls(&serde_json::from_value(json!({"select": "highlights"})).unwrap()),
        Controls { fuzziness: true, sampling: false, range: false, tonal: true }
    );
}

#[test]
fn dialog_shrinks_back_when_a_mode_needs_fewer_controls() {
    let mut h = harness(app_with_doc());
    open(h.state_mut());
    h.run_steps(4);
    let area = egui::Id::unique(("dialog", dialog_id(h.state())));
    let rect = |h: &Harness<'static, PhotocraftApp>| h.ctx.memory(|m| m.area_rect(area)).unwrap_or(egui::Rect::NOTHING);
    set(&mut h, "select", json!("outOfGamut"));
    let short = rect(&h);
    set(&mut h, "select", json!("midtones"));
    let tall = rect(&h);
    assert!(tall.height() > short.height() + 40.0, "Midtones adds two sliders: {short:?} → {tall:?}");
    assert_eq!(tall.min, short.min, "the dialog grows downward, it doesn't re-centre");
    set(&mut h, "select", json!("outOfGamut"));
    let back = rect(&h);
    assert!((back.height() - short.height()).abs() < 1.0, "the dialog must shrink back, not keep an empty band above OK: {short:?} → {tall:?} → {back:?}");
    assert_eq!(back.min, short.min);
}

#[test]
fn ok_applies_one_undo_step() {
    let mut h = harness(app_with_doc());
    // The prior selection covers the left red square: Color Range is scoped to it (#3313).
    h.state_mut().run("select.rect", json!({"x": 0, "y": 0, "width": 20, "height": 30})).unwrap();
    let past = h.state().session.active().unwrap().history.past_len();
    open(h.state_mut());
    h.run_steps(4);
    set(&mut h, "select", json!("reds"));
    ok(&mut h);
    h.run_steps(2);
    assert!(!h.state().ui.dialogs.iter().any(|d| owns(&d.fields)), "OK closes the dialog");
    let app = h.state();
    assert_eq!((coverage(app, 5, 5), coverage(app, 30, 5), coverage(app, 2, 22)), (1.0, 0.0, 0.0));
    assert_eq!(app.session.active().unwrap().history.past_len(), past + 1, "one history step");
    assert_eq!(app.session.active().unwrap().history.undo_label(), Some("Color Range"));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    let app = h.state();
    assert_eq!((coverage(app, 5, 5), coverage(app, 2, 22), coverage(app, 30, 5)), (1.0, 1.0, 0.0), "undo restores the previous selection");
}

#[test]
fn cancel_keeps_the_previous_selection() {
    let mut h = harness(app_with_doc());
    h.state_mut().run("select.rect", json!({"x": 0, "y": 20, "width": 5, "height": 5})).unwrap();
    let (rev, past) = {
        let st = h.state().session.active().unwrap();
        (st.revision, st.history.past_len())
    };
    open(h.state_mut());
    h.run_steps(4);
    set(&mut h, "select", json!("blues"));
    h.get_by_label("Invert").click();
    h.run_steps(2);
    h.key_press(egui::Key::Escape);
    h.run_steps(2);
    assert!(h.state().ui.dialogs.is_empty());
    let app = h.state();
    let st = app.session.active().unwrap();
    assert_eq!((st.revision, st.history.past_len()), (rev, past));
    assert_eq!((coverage(app, 2, 22), coverage(app, 30, 5), coverage(app, 5, 5)), (1.0, 0.0, 0.0));
}

#[test]
fn invert_flips_the_selection() {
    let mut h = harness(app_with_doc());
    open(h.state_mut());
    h.run_steps(4);
    set(&mut h, "select", json!("reds"));
    h.get_by_label("Invert").click();
    h.run_steps(2);
    let id = dialog_id(h.state());
    assert_eq!(h.state_mut().ui.dialog_mut(id).unwrap().fields.get("invert"), Some(&json!(true)));
    ok(&mut h);
    h.run_steps(2);
    let app = h.state();
    assert_eq!((coverage(app, 5, 5), coverage(app, 30, 5), coverage(app, 5, 22), coverage(app, 30, 22)), (0.0, 1.0, 1.0, 1.0));
}

#[test]
fn eyedroppers_pick_on_the_preview() {
    let mut h = harness(app_with_doc());
    open(h.state_mut());
    h.run_steps(4);
    // 40 × 30 at 200 pt: 5 pt per pixel; the image is centred vertically (150 pt tall).
    let at = |h: &Harness<'static, PhotocraftApp>, x: f32, y: f32| {
        h.get_by_label("Color Range preview").rect().left_top() + egui::vec2(x * 5.0 + 2.5, 25.0 + y * 5.0 + 2.5)
    };
    let p = at(&h, 30.0, 5.0);
    click(&mut h, p);
    h.run_steps(2);
    let f = |h: &Harness<'static, PhotocraftApp>| h.state().ui.dialogs.iter().find(|d| owns(&d.fields)).unwrap().fields.clone();
    assert_eq!(points(&f(&h), "points"), vec![[30.0, 5.0]]);
    // Add to Sample: the red block too; the selection then covers both.
    h.get_by_label("Add to Sample").click();
    h.run_steps(1);
    let p = at(&h, 5.0, 5.0);
    click(&mut h, p);
    h.run_steps(2);
    assert_eq!(points(&f(&h), "points").len(), 2);
    // Subtract from Sample: take the blue back out.
    h.get_by_label("Subtract from Sample").click();
    h.run_steps(1);
    let p = at(&h, 35.0, 10.0);
    click(&mut h, p);
    h.run_steps(2);
    assert_eq!(points(&f(&h), "subtractPoints"), vec![[35.0, 10.0]]);
    let p = params(&f(&h));
    assert_eq!(p["select"], "sampledColors");
    assert_eq!(p["order"], json!(["+", "+", "-"]));
    ok(&mut h);
    h.run_steps(2);
    let app = h.state();
    // The sampled red stays fully selected.
    assert_eq!(coverage(app, 5, 5), 1.0);
    // Like Photoshop, subtracting a colour on the edge of the samples' range only trims that
    // edge by a fifth of the falloff: the blue stays mostly selected (Photoshop: 232 of 255
    // for the same case), black never was.
    let blue = coverage(app, 30, 5);
    assert!(blue > 0.85 && blue < 0.95, "{blue}");
    assert_eq!(coverage(app, 5, 22), 0.0);
}

#[test]
fn selection_preview_modes_cover_only_unselected_pixels() {
    assert_eq!(overlay_pixel("grayscale", 1.0), Color32::WHITE);
    assert_eq!(overlay_pixel("grayscale", 0.0), Color32::BLACK);
    assert_eq!(overlay_pixel("grayscale", 0.5), Color32::from_gray(128));
    for mode in ["blackMatte", "whiteMatte", "quickMask"] {
        assert_eq!(overlay_pixel(mode, 1.0).a(), 0, "{mode}: selected pixels stay visible");
    }
    assert_eq!(overlay_pixel("blackMatte", 0.0), Color32::BLACK);
    assert_eq!(overlay_pixel("whiteMatte", 0.0), Color32::WHITE);
    assert_eq!(overlay_pixel("quickMask", 0.0).a(), 128);
    assert_eq!(overlay_pixel("quickMask", 0.5).a(), 64);
    assert_eq!(overlay_pixel("none", 0.0), Color32::TRANSPARENT);
}

#[test]
fn preview_mode_is_view_state_and_cancel_preserves_selection() {
    let mut h = harness(app_with_doc());
    h.state_mut().run("select.rect", json!({"x": 0, "y": 20, "width": 5, "height": 5})).unwrap();
    let original = h.state().session.active().unwrap();
    let past = original.history.past_len();
    let revision = original.revision;
    let id = open(h.state_mut());
    h.run_steps(3);
    for (mode, _) in PREVIEWS {
        set(&mut h, "__selectionPreview", json!(mode));
        let f = &h.state().ui.dialogs.last().unwrap().fields;
        assert!(params(f).get("__selectionPreview").is_none());
        let painter = h.ctx.layer_painter(egui::LayerId::background());
        let doc = h.state().session.active().unwrap().doc.id;
        let ctx = h.ctx.clone();
        paint_selection_preview(h.state_mut(), &ctx, &painter, doc, Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0)), false);
        let st = h.state().session.active().unwrap();
        assert_eq!((st.revision, st.history.past_len()), (revision, past));
        assert_eq!(coverage(h.state(), 2, 22), 1.0);
    }
    h.state_mut().ui.close_dialog(id);
    let painter = h.ctx.layer_painter(egui::LayerId::background());
    let ctx = h.ctx.clone();
    let doc = h.state().session.active().unwrap().doc.id;
    paint_selection_preview(h.state_mut(), &ctx, &painter, doc, Rect::EVERYTHING, false);
    assert_eq!(coverage(h.state(), 2, 22), 1.0);
}

#[test]
fn repeated_subtraction_clicks_are_not_discarded() {
    let mut app = app_with_doc();
    let id = open(&mut app);
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    pick(&app, &mut f, [2.0, 2.0], egui::Modifiers::NONE);
    pick(&app, &mut f, [30.0, 2.0], egui::Modifiers::SHIFT);
    pick(&app, &mut f, [30.0, 2.0], egui::Modifiers::ALT);
    pick(&app, &mut f, [30.0, 2.0], egui::Modifiers::ALT);
    assert_eq!(params(&f)["order"], json!(["+", "+", "-", "-"]));
}

#[test]
fn control_pointer_drag_samples_and_never_paints_under_the_dialog() {
    let mut h = harness(app_with_doc());
    let app = h.state_mut();
    open(app);
    let doc = app.session.active().unwrap();
    let before = doc.revision;
    let history = doc.history.past_len();
    let ctx = h.ctx.clone();
    for (events, shift) in [
        (json!([{"kind":"down","x":2,"y":2},{"kind":"move","x":3,"y":2},{"kind":"up","x":3,"y":2}]), false),
        (json!([{"kind":"down","x":30,"y":2},{"kind":"move","x":31,"y":2},{"kind":"up","x":31,"y":2}]), true),
    ] {
        let (req, _) = crate::control::ControlRequest::new("ui.pointer", json!({"events":events,"shift":shift}));
        let _ = crate::control::handle(h.state_mut(), &ctx, &req);
    }
    let fields = &h.state().ui.dialogs.last().unwrap().fields;
    assert_eq!(points(fields, "points"), [[3.0, 2.0], [30.0, 2.0], [31.0, 2.0]]);
    let doc = h.state().session.active().unwrap();
    assert_eq!((doc.revision, doc.history.past_len()), (before, history));
    assert!(h.state().drag.is_none());
    assert!(!pick_top(h.state_mut(), [-1.0, 0.0], egui::Modifiers::NONE));
}

#[test]
fn modifier_temporarily_swaps_image_and_selection_thumbnail() {
    let mut h = harness(app_with_doc());
    let id = open(h.state_mut());
    h.run_steps(3);
    let f = h.state().ui.dialogs.last().unwrap().fields.clone();
    let ctx = h.ctx.clone();
    let selection = preview(h.state_mut(), &ctx, &f).unwrap().0;
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::COMMAND));
    h.run_steps(2);
    let image = preview(h.state_mut(), &ctx, &f).unwrap().0;
    assert_ne!(image, selection);
    assert_eq!(h.state().ui.dialogs.iter().find(|d| d.id == id).unwrap().fields["__view"], "selection");
    h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    h.run_steps(2);
    assert_eq!(preview(h.state_mut(), &ctx, &f).unwrap().0, selection);
}

/// A sample between thumbnail pixels must use the original pixel, not its neighbour.
#[test]
fn reduced_preview_matches_the_original_eyedropper_sample() {
    let mut doc = Document::with_background("fine detail", Size::new(400, 4), ColorMode::Rgb, SampleType::U8, Color::rgb(1.0, 0.0, 0.0));
    let surface = doc.layers[0].surface_mut().unwrap();
    surface.fill_rect(GRect::new(200, 0, 400, 4), &[0.0, 0.0, 1.0, 1.0]);
    surface.fill_rect(GRect::new(101, 0, 102, 4), &[0.0, 0.0, 1.0, 1.0]);
    let mut session = photocraft_engine::Session::new();
    session.add_document(doc, None);
    let mut app = PhotocraftApp::new(session, crate::Services::default());
    let id = open(&mut app);
    let mut fields = app.ui.dialog_mut(id).unwrap().fields.clone();
    pick(&app, &mut fields, [101.0, 1.0], egui::Modifiers::NONE);
    fields.insert("fuzziness".into(), json!(0.0));
    let original = app.session.active().unwrap().doc.clone();
    let proxy = crate::proxy::proxy_document(&original, 2);
    let mut old = photocraft_engine::Session::new();
    old.add_document(proxy.clone(), None);
    old.execute(COMMAND, json!({"points": [[50, 0]], "fuzziness": 0})).unwrap();
    assert_eq!(
        old.active().unwrap().doc.selection.as_ref().unwrap().sample_channel(150, 0, 0),
        0.0,
        "sampling the thumbnail instead selects the red neighbour"
    );
    let shown = proxy_mask(&app, &proxy, 2, &params(&fields)).unwrap();
    confirm(&mut app, &fields).unwrap();
    assert_eq!(coverage(&app, 300, 0), 1.0);
    assert_eq!(shown[150], coverage(&app, 300, 0), "preview must show the same blue pixels as OK");
    assert_eq!(shown[0], coverage(&app, 0, 0), "preview must leave the red background unselected");
}

/// `sampleAllLayers: false` judges the active layer in the preview, as OK does.
#[test]
fn preview_honours_sample_all_layers_like_ok() {
    let mut app = app_with_doc();
    let doc = app.session.active().unwrap().doc.clone();
    let mut top = photocraft_doc::Layer::new("blue cover", photocraft_doc::LayerContent::Raster(doc.layers[0].surface().unwrap().clone()));
    top.surface_mut().unwrap().fill_rect(GRect::new(0, 0, 40, 30), &[0.0, 0.0, 1.0, 1.0]);
    app.session
        .edit("layers", |doc, _| {
            doc.layers.push(top);
            Ok(())
        })
        .unwrap();
    for all_layers in [false, true] {
        let p = json!({"select": "sampledColors", "points": [[5, 5]], "fuzziness": 0, "sampleAllLayers": all_layers});
        let proxy = crate::proxy::proxy_document(&app.session.active().unwrap().doc, 1);
        let shown = proxy_mask(&app, &proxy, 1, &p).unwrap();
        app.run(COMMAND, p).unwrap();
        for (x, y) in [(5, 5), (30, 5), (5, 20), (30, 20)] {
            assert_eq!(shown[y * 40 + x], coverage(&app, x as i32, y as i32), "all_layers={all_layers} at ({x}, {y})");
        }
    }
}

#[test]
fn params_send_only_what_the_mode_uses() {
    let mut app = app_with_doc();
    let id = open(&mut app);
    let mut f = app.ui.dialog_mut(id).unwrap().fields.clone();
    assert_eq!(params(&f), json!({"select": "sampledColors", "invert": false, "fuzziness": 40.0}));
    // Localized needs a picked point.
    f.insert("localized".into(), json!(true));
    assert!(params(&f).get("localized").is_none());
    pick(&app, &mut f, [3.0, 4.0], egui::Modifiers::NONE);
    pick(&app, &mut f, [99.0, -4.0], egui::Modifiers::SHIFT);
    pick(&app, &mut f, [1.0, 1.0], egui::Modifiers::ALT);
    assert_eq!(
        params(&f),
        json!({"select": "sampledColors", "invert": false, "fuzziness": 40.0, "points": [[3.0, 4.0], [39.0, 0.0]], "subtractPoints": [[1.0, 1.0]], "order": ["+", "+", "-"], "localized": true, "range": 100.0})
    );
    f.insert("select".into(), json!("midtones"));
    f.insert("midtonesLow".into(), json!(160.0));
    assert_eq!(params(&f), json!({"select": "midtones", "invert": false, "fuzziness": 20.0, "tonalRange": [150.0, 160.0]}));
    f.insert("select".into(), json!("highlights"));
    assert_eq!(params(&f)["tonalRange"], json!(190.0));
    f.insert("select".into(), json!("cyans"));
    assert_eq!(params(&f), json!({"select": "cyans", "invert": false}));
    // Every Select entry is accepted by the engine.
    for (v, _) in SELECTS {
        f.insert("select".into(), json!(v));
        app.run(COMMAND, params(&f)).unwrap();
    }
}

/// #145: the whole app without a GPU (no wgpu render state: the CPU canvas, as on a Linux
/// machine whose adapter can't run the GPU canvas), on a small window. Select › Color Range…
/// from the menu opens the dialog on screen and draws both previews.
#[test]
fn opens_and_previews_in_the_full_app_without_a_gpu() {
    let mut h = Harness::builder().with_size(egui::vec2(1024.0, 600.0)).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        assert!(cc.wgpu_render_state.is_none(), "this test covers the no-GPU path");
        app_with_doc()
    });
    h.run_steps(6);
    assert!(h.state().gpu.is_none(), "the CPU canvas");
    h.get_all_by_label("Select").next().expect("the Select menu").click();
    h.run_steps(3);
    h.get_by_label("Color Range…").click();
    h.run_steps(6);
    let id = dialog_id(h.state());
    let area = h.ctx.memory(|m| m.area_rect(egui::Id::unique(("dialog", id)))).expect("the dialog is drawn");
    assert!(h.ctx.content_rect().contains_rect(area), "the dialog is on screen: {area:?}");
    assert!(h.query_by_label("Color Range preview").is_some());
    let p = h.state().color_range.as_ref().expect("the preview is cached");
    assert!(p.mask.is_some() && p.error.is_none(), "the selection preview is drawn: {:?}", p.error);
    set(&mut h, "__view", json!("image"));
    let p = h.state().color_range.as_ref().unwrap();
    assert!(p.image.is_some() && p.error.is_none(), "the image preview is drawn: {:?}", p.error);
    set(&mut h, "select", json!("reds"));
    ok(&mut h);
    h.run_steps(3);
    assert_eq!(coverage(h.state(), 5, 5), 1.0);
}

#[test]
fn a_preview_failure_is_shown_not_silent() {
    let mut h = harness(app_with_doc());
    open(h.state_mut());
    h.run_steps(3);
    // The engine refuses these params: the preview says so instead of staying blank.
    let id = dialog_id(h.state());
    h.state_mut().ui.dialog_mut(id).unwrap().fields.insert("select".into(), json!("noSuchMode"));
    h.run_steps(3);
    let err = h.state().color_range.as_ref().and_then(|p| p.error.clone());
    assert!(err.is_some(), "the failure is recorded");
    assert!(h.query_by_label_contains("Color Range preview unavailable").is_some(), "and shown");
    set(&mut h, "select", json!("reds"));
    assert!(h.state().color_range.as_ref().unwrap().error.is_none(), "and cleared once the preview works again");
}
