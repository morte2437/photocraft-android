use super::*;
use photocraft_doc::StrokeFx;

/// #5701: with a shadow page selected, a drag on the image moves the shadow: its offset
/// follows the pointer, as whole degrees and pixels, the global light angle with it.
#[test]
fn dragging_on_the_image_moves_the_selected_shadow() {
    let shadow = |id: &str, kind: &str, global: bool| json!({"id": id, "kind": kind, "on": true, "params": {"angle": 120.0, "distance": 10.0, "useGlobalLight": global}});
    let mut f = Map::new();
    f.insert("selected".into(), json!("fx1"));
    f.insert("globalLight".into(), json!(120.0));
    f.insert("effects".into(), json!([shadow("fx1", "dropShadow", true), shadow("fx2", "innerShadow", true), shadow("fx3", "dropShadow", false)]));
    assert!(drags_shadow(&f));
    let params = |f: &Map<String, Value>, id: &str| {
        let p = &entry(f, id).unwrap()["params"];
        (p["angle"].as_f64().unwrap(), p["distance"].as_f64().unwrap())
    };
    // The press keeps the shadow where it is: (5, 8.66) px.
    drag_shadow(&mut f, true, [0.0, 0.0]);
    assert_eq!(params(&f, "fx1"), (120.0, 10.0));
    // Dragged to (15, 0): straight right of the layer, the light from the left (180°).
    drag_shadow(&mut f, false, [10.0, -8.660_254]);
    assert_eq!(params(&f, "fx1"), (180.0, 15.0));
    assert_eq!(params(&f, "fx2").0, 180.0, "the global light angle is shared");
    assert_eq!(params(&f, "fx3"), (120.0, 10.0), "an instance with its own angle is left alone");
    // Back onto the layer: no offset, and the angle the drag started with.
    drag_shadow(&mut f, false, [-5.0, -8.660_254]);
    assert_eq!(params(&f, "fx1"), (120.0, 0.0));
    // Not on a shadow page: nothing moves.
    f.insert("selected".into(), json!(BLENDING));
    assert!(!drags_shadow(&f));
    drag_shadow(&mut f, true, [50.0, 50.0]);
    assert_eq!(params(&f, "fx1"), (120.0, 0.0));
}

fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s
}

fn strokes_of(s: &photocraft_engine::Session) -> Vec<StrokeFx> {
    let d = s.active().unwrap();
    let id = d.active_layer.unwrap();
    d.doc
        .layer(id)
        .unwrap()
        .effects
        .items
        .iter()
        .filter_map(|e| match e {
            Effect::Stroke(st) => Some(st.clone()),
            _ => None,
        })
        .collect()
}

/// #1357: every effect row has its checkbox from the start, also for the effects not on the
/// layer yet (an empty box), not only after one was turned on and off again.
#[test]
fn every_effect_row_shows_a_checkbox() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    let l = Layer::raster("x", photocraft_doc::PixelFormat::RGBA8);
    let fields = initial_fields(&l, Some(BLENDING), 120.0);
    let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            let mut f = fields.clone();
            body(app, ui, &mut f);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
    h.run_steps(3);
    let boxes: Vec<egui::Rect> = h
        .output()
        .shapes
        .iter()
        .filter_map(|c| match &c.shape {
            egui::Shape::Rect(r) if r.rect.size() == vec2(14.0, 14.0) && r.stroke.width > 0.0 => Some(r.rect),
            _ => None,
        })
        .collect();
    // The effect list is the leftmost column; the selected pane draws checkboxes of its own.
    let column = boxes.iter().map(|r| r.min.x).fold(f32::INFINITY, f32::min);
    let rows = boxes.iter().filter(|r| (r.min.x - column).abs() < 0.5).count();
    assert_eq!(rows, KINDS.len(), "one empty checkbox per effect row");
}

// Regression test for #2209: Make Default and Reset to Default sat in a non-wrapping row, so
// translated labels wider than the page column (Russian) pushed one button over the other
// and out of the column instead of wrapping to a second row.
#[test]
fn default_buttons_wrap_inside_the_page_column_with_long_labels() {
    use egui_kittest::kittest::Queryable;
    let ru = crate::i18n::Lang::from_code("ru").unwrap();
    crate::i18n::with_language(ru, || {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
        let l = Layer::raster("x", photocraft_doc::PixelFormat::RGBA8);
        let fields = initial_fields(&l, Some("dropShadow"), 120.0);
        let mut h = egui_kittest::Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                let mut f = fields.clone();
                body(app, ui, &mut f);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.run_steps(3);
        let reset = h.get_by_role_and_label(egui::accesskit::Role::Button, crate::i18n::tr(ru, "Reset to Default")).rect();
        let make = h.get_by_role_and_label(egui::accesskit::Role::Button, crate::i18n::tr(ru, "Make Default")).rect();
        // Together they are wider than the page column: the second wraps to a row below
        // instead of sharing the row and running left over the effect list.
        assert!(make.top() >= reset.bottom(), "the buttons share one row: {make:?} {reset:?}");
        assert_eq!(make.right(), reset.right(), "the wrapped row stays right-aligned");
    });
}

/// Double-clicking an effect row of a layer that isn't active selects that layer first, so
/// the dialog holds its effects (and OK edits it), not the active layer's.
#[test]
fn effect_row_of_another_layer_opens_that_layers_style() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 32, "height": 32})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let with_fx = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.layerStyle.replace", json!({"layer": with_fx.0, "effects": [{"kind": "dropShadow", "params": defaults("dropShadow")}]})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let plain = app.session.active().unwrap().active_layer.unwrap();
    assert_ne!(plain, with_fx);
    assert!(app.session.active().unwrap().doc.layer(plain).unwrap().effects.items.is_empty());

    let id = open_for_layer(&mut app, with_fx, Some("dropShadow")).unwrap();
    assert_eq!(app.session.active().unwrap().active_layer, Some(with_fx), "the row's layer becomes active");
    let d = app.ui.dialogs.iter().find(|d| d.id == id).unwrap();
    assert_eq!(d.fields["layer"], json!(with_fx.0));
    let fx = effects_of(&d.fields);
    assert_eq!(fx.len(), 1);
    assert_eq!(fx[0]["kind"], json!("dropShadow"));
    assert!(fx[0]["fx"].is_object(), "the existing drop shadow, not a new instance");
}

#[test]
fn every_kind_has_spec_and_defaults() {
    for &(kind, _) in KINDS {
        assert!(!spec(kind).is_empty(), "{kind}");
        let d = defaults(kind);
        for &(key, _, _) in spec(kind) {
            assert!(d.get(key).is_some(), "{kind}.{key} has no default");
        }
    }
}

#[test]
fn adding_an_effect_after_maximum_instance_id_uses_an_unused_id() {
    // Imported and scripted effects can provide arbitrary instance IDs.
    let layer = Layer::raster("x", photocraft_doc::PixelFormat::RGBA8);
    let mut fields = initial_fields(&layer, None, 120.0);
    fields.insert(
        "effects".into(),
        json!([
            {"id": "fx1", "kind": "stroke", "on": true, "params": {}},
            {"id": format!("fx{}", u64::MAX), "kind": "stroke", "on": false, "params": {}}
        ]),
    );
    add_instance(&mut fields, "stroke");

    let ids: Vec<&str> = effects_of(&fields).iter().filter_map(|e| e.get("id").and_then(Value::as_str)).collect();
    assert_eq!(ids, ["fx1", "fx18446744073709551615", "fx2"]);
    assert_eq!(fields["selected"], json!("fx2"));
}

#[test]
fn initial_fields_lists_every_instance_and_selects() {
    let l = Layer::raster("x", photocraft_doc::PixelFormat::RGBA8);
    let f = initial_fields(&l, Some("stroke"), 120.0);
    assert_eq!(f["globalLight"], json!(120.0));
    // The kind isn't on the layer yet: selecting it creates the instance.
    assert_eq!(f["effects"].as_array().unwrap().len(), 1);
    assert_eq!(f["effects"][0]["kind"], json!("stroke"));
    assert_eq!(f["effects"][0]["on"], json!(true));
    assert_eq!(f["selected"], json!("fx1"));
    // Opening the blending page selects no instance.
    let f = initial_fields(&l, Some(BLENDING), 120.0);
    assert_eq!(f["selected"], json!(BLENDING));
    assert!(effects_of(&f).is_empty());
}

#[test]
fn open_and_confirm_preserves_two_strokes() {
    // The manga/typesetting case: a 3 px black stroke under a 7 px white one.
    let mut s = session();
    s.execute("layer.layerStyle.stroke", json!({"size": 3, "color": "#000000"})).unwrap();
    s.execute("layer.layerStyle.stroke", json!({"size": 7, "color": "#ffffff", "add": true})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    assert_eq!(effects_of(&f).len(), 2, "both instances load");
    // Confirm without touching anything: both strokes, same order and params.
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let strokes = strokes_of(&s);
    assert_eq!(strokes.len(), 2);
    assert_eq!((strokes[0].size, strokes[1].size), (3.0, 7.0));
    assert_eq!(strokes[0].paint, FxPaint::Color(photocraft_color::Color::BLACK));
    assert_eq!(strokes[1].paint, FxPaint::Color(photocraft_color::Color::WHITE));
}

#[test]
fn editing_one_instance_leaves_its_sibling_alone() {
    let mut s = session();
    s.execute("layer.layerStyle.stroke", json!({"size": 3, "color": "#000000"})).unwrap();
    s.execute("layer.layerStyle.stroke", json!({"size": 7, "color": "#ffffff", "add": true})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    set_param(&mut f, "fx1", "size", json!(5));
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let strokes = strokes_of(&s);
    assert_eq!(strokes.len(), 2);
    assert_eq!(strokes[0].size, 5.0);
    assert_eq!(strokes[1].size, 7.0, "the second stroke is untouched");
    assert_eq!(strokes[1].paint, FxPaint::Color(photocraft_color::Color::WHITE));
}

#[test]
fn add_and_remove_instances() {
    let mut s = session();
    s.execute("layer.layerStyle.stroke", json!({"size": 3, "color": "#000000"})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    add_instance(&mut f, "stroke");
    add_instance(&mut f, "stroke");
    assert_eq!(effects_of(&f).len(), 3);
    assert_eq!(f["selected"], json!("fx3"));
    remove_instance(&mut f, "fx2");
    assert_eq!(effects_of(&f).len(), 2);
    assert_eq!(f["selected"], json!("fx3"), "selection moves to a sibling");
    remove_instance(&mut f, "fx3");
    assert_eq!(f["selected"], json!("fx1"), "the last sibling takes over");
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let strokes = strokes_of(&s);
    assert_eq!(strokes.len(), 1);
    assert_eq!(strokes[0].size, 3.0);
}

#[test]
fn disabled_instances_are_kept() {
    let mut s = session();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    if let Some(e) = entry_mut(&mut f, "fx1") {
        e["on"] = json!(false);
    }
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let d = s.active().unwrap();
    let items = &d.doc.layer(id).unwrap().effects.items;
    assert_eq!(items.len(), 1, "configured but switched off stays");
    assert!(!items[0].enabled());
}

#[test]
fn baked_gradient_overlay_keeps_endpoint_colours_on_reopen() {
    let mut s = session();
    s.execute("layer.layerStyle.gradientOverlay", json!({"from": "#ff0000", "to": "#0000ff", "method": "classic"})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let layer = st.doc.layer(id).unwrap();
    let Effect::GradientOverlay { gradient, .. } = &layer.effects.items[0] else { panic!() };
    assert!(gradient.stops.len() > 2, "classic bakes intermediate stops");
    let f = initial_fields(layer, Some("gradientOverlay"), st.doc.global_light.angle);
    let params = &effects_of(&f)[0]["params"];
    assert_eq!(params["from"], json!("#ff0000"));
    assert_eq!(params["to"], json!("#0000ff"));
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let Effect::GradientOverlay { gradient, .. } = &s.active().unwrap().doc.layer(id).unwrap().effects.items[0] else { panic!() };
    assert!(gradient.stops.len() > 2);
    assert_eq!(gradient.stops.first().unwrap().1, photocraft_color::Color::rgb(1.0, 0.0, 0.0));
    assert_eq!(gradient.stops.last().unwrap().1, photocraft_color::Color::rgb(0.0, 0.0, 1.0));
}

#[test]
fn gradient_strokes_survive_the_dialog() {
    let mut s = session();
    s.execute("layer.layerStyle.stroke", json!({"size": 4, "from": "#ff0000", "to": "#0000ff"})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    // The dialog doesn't model the gradient, so no colour key is stored for it.
    assert!(effects_of(&f)[0]["params"].get("color").is_none());
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let strokes = strokes_of(&s);
    assert_eq!(strokes.len(), 1);
    assert!(
        matches!(&strokes[0].paint, FxPaint::Gradient(g) if g.stops.len() == 2
            && g.stops[0].1 == photocraft_color::Color::rgb(1.0, 0.0, 0.0)
            && g.stops[1].1 == photocraft_color::Color::rgb(0.0, 0.0, 1.0)),
        "the gradient is carried, not reset",
    );
}

#[test]
fn bevel_gloss_anti_alias_survives_the_dialog() {
    let mut s = session();
    s.execute("layer.layerStyle.bevelEmboss", json!({"glossContour": "Ring - Double", "glossAntiAlias": true})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    assert_eq!(effects_of(&f)[0]["params"]["glossAntiAlias"], json!(true));
    // Editing another setting keeps the gloss contour's Anti-aliased.
    set_param(&mut f, "fx1", "size", json!(12));
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let st = s.active().unwrap();
    let items = &st.doc.layer(id).unwrap().effects.items;
    let Some(photocraft_doc::Effect::BevelEmboss(b)) = items.first() else { panic!("{items:?}") };
    assert_eq!(b.size, 12.0);
    assert!(b.gloss_anti_alias);
}

#[test]
fn stroke_overprint_survives_the_dialog() {
    let mut s = session();
    s.execute("layer.layerStyle.stroke", json!({"size": 3, "color": "#000000", "overprint": true})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    assert_eq!(effects_of(&f)[0]["params"]["overprint"], json!(true));
    // Editing another setting keeps Overprint.
    set_param(&mut f, "fx1", "size", json!(5));
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let strokes = strokes_of(&s);
    assert_eq!(strokes[0].size, 5.0);
    assert!(strokes[0].overprint);
}

#[test]
fn preview_applies_the_style_without_touching_the_document() {
    let s = session();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("colorOverlay"), st.doc.global_light.angle);
    add_instance(&mut f, "stroke");
    let h = preview_hash(&f);
    f.insert("selected".into(), json!("fx2"));
    assert_eq!(preview_hash(&f), h, "switching pages doesn't re-render");
    set_param(&mut f, "fx2", "size", json!(5));
    assert_ne!(preview_hash(&f), h);
    let shown = preview_document(&st.doc, &s.patterns, &f).unwrap();
    let fx = |doc: &photocraft_doc::Document| doc.layer(id).unwrap().effects.items.len();
    assert_eq!((fx(&shown), fx(&st.doc)), (2, 0));
}

#[test]
fn preview_rejects_non_object_effect_params() {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 16, "height": 16})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    let st = s.active().unwrap();
    let mut f = initial_fields(st.doc.layer(st.active_layer.unwrap()).unwrap(), Some("colorOverlay"), st.doc.global_light.angle);
    let id = f["selected"].as_str().unwrap().to_string();
    entry_mut(&mut f, &id).unwrap()["params"] = json!("invalid");

    assert!(preview_document(&st.doc, &s.patterns, &f).is_err());
}

#[test]
fn apply_rejects_non_object_params_before_running_commands() {
    let mut f = Map::new();
    f.insert("effects".into(), json!([{"id": "fx1", "kind": "stroke", "on": true, "params": [1, 2]}]));
    let mut calls = 0;

    let result = apply(&f, |_, _| {
        calls += 1;
        Ok(Value::Null)
    });

    assert!(result.is_err());
    assert_eq!(calls, 0, "invalid params must not partially apply the style");
}

#[test]
fn body_keeps_invalid_params_and_renders_without_panicking() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    for bad in [json!(7), json!(null), json!("x")] {
        let mut fields = Map::new();
        fields.insert("selected".into(), json!("fx1"));
        fields.insert("effects".into(), json!([{"id": "fx1", "kind": "dropShadow", "on": true, "params": bad.clone()}]));

        let mut out = ctx.run_ui(Default::default(), |ui| body(&mut app, ui, &mut fields));
        out.textures_delta.clear();

        assert_eq!(entry(&fields, "fx1").unwrap()["params"], bad);
    }
    for bad in [json!(7), json!([1])] {
        let mut fields = Map::new();
        fields.insert("selected".into(), json!(BLENDING));
        fields.insert("p:blendingOptions".into(), bad.clone());
        fields.insert("effects".into(), bad.clone());
        let mut out = ctx.run_ui(Default::default(), |ui| body(&mut app, ui, &mut fields));
        out.textures_delta.clear();
        assert_eq!(fields.get("p:blendingOptions"), Some(&bad));
    }
}

/// #2288: the selected effect row must dim again when its checkbox is cleared.
#[test]
fn disabled_effect_row_uses_muted_text_even_when_selected() {
    for theme in crate::theme::ThemeKind::ALL {
        let tokens = Tokens::for_kind(theme);
        assert_eq!(fx_label_color(true, &tokens), tokens.text, "{theme:?}: checked");
        assert_eq!(fx_label_color(false, &tokens), tokens.text_dim, "{theme:?}: unchecked");
    }
}

#[test]
fn blending_page_renders_for_every_mode_and_odd_fields() {
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let mut layer = photocraft_doc::Layer::new("L", photocraft_doc::LayerContent::Adjustment(photocraft_doc::Adjustment::Invert));
    layer.advanced.knockout = photocraft_doc::Knockout::Deep;
    for mode in [photocraft_color::ColorMode::Rgb, photocraft_color::ColorMode::Cmyk, photocraft_color::ColorMode::Lab, photocraft_color::ColorMode::Grayscale]
    {
        let mut f = initial_fields(&layer, Some(BLENDING), 120.0);
        set_blending_fields(&mut f, &layer, mode);
        let mut out = ctx.run_ui(Default::default(), |ui| body(&mut app, ui, &mut f));
        out.textures_delta.clear();
        assert_eq!(f["p:blendingOptions"]["knockout"], "deep", "{mode:?}");
        // Hostile dialog state (only reachable through `ui.dialog.set`) still renders.
        let bo = f.get_mut("p:blendingOptions").unwrap();
        bo["blendIfChannel"] = json!(99);
        bo["channels"] = json!("x");
        bo["blendIf"] = json!([{"thisLayer": [300, -5, "a"]}]);
        f.insert("channelNames".into(), json!([]));
        let mut out = ctx.run_ui(Default::default(), |ui| body(&mut app, ui, &mut f));
        out.textures_delta.clear();
    }
}

#[test]
fn confirm_rejects_invalid_params_and_applies_valid_effects() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let layer = app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().clone();
    let mut invalid = initial_fields(&layer, Some("colorOverlay"), 0.0);
    let id = invalid["selected"].as_str().unwrap().to_string();
    entry_mut(&mut invalid, &id).unwrap()["params"] = json!(null);
    assert!(confirm(&mut app, &invalid).is_err());
    assert!(app.session.active().unwrap().doc.layer(layer.id).unwrap().effects.items.is_empty());

    let valid = initial_fields(&layer, Some("colorOverlay"), 0.0);
    confirm(&mut app, &valid).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(layer.id).unwrap().effects.items.len(), 1);
}

#[test]
fn blending_page_round_trips_advanced_blending_and_blend_if() {
    use photocraft_doc::{BlendRange, Knockout};
    for mode in ["rgb", "cmyk", "grayscale"] {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 16, "height": 16, "mode": mode})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let mut l = st.doc.layer(id).unwrap().clone();
        l.advanced.knockout = Knockout::Shallow;
        l.advanced.blend_clipped = false;
        l.excluded_channels = 0b1;
        let gray = if st.doc.mode.color_channels() == 1 { 1 } else { 0 };
        l.blend_if.set(gray, [BlendRange { black: [10, 30], white: [255, 255] }, BlendRange::FULL]);
        let mut f = initial_fields(&l, None, 120.0);
        set_blending_fields(&mut f, &l, st.doc.mode);
        let names = channel_names(st.doc.mode);
        assert_eq!(f["channelNames"], json!(names), "{mode}");
        let p = &f[&format!("p:{BLENDING}")];
        assert_eq!(p["knockout"], "shallow");
        assert_eq!(p["blendClippedLayersAsGroup"], false);
        assert_eq!(p["transparencyShapesLayer"], true);
        assert_eq!(p["channels"].as_array().unwrap().len(), names.len());
        assert_eq!(p["channels"][0], false);
        assert_eq!(p["blendIf"][0]["thisLayer"], json!([10, 30, 255, 255]), "{mode}");
        // Applying the page sets every switch through the engine command.
        let mut f2 = f.clone();
        let bo = f2.get_mut(&format!("p:{BLENDING}")).unwrap();
        bo["knockout"] = json!("deep");
        bo["vectorMaskHidesEffects"] = json!(true);
        bo["blendIfChannel"] = json!(names.len().min(1));
        let shown = preview_document(&st.doc, &s.patterns, &f2).unwrap();
        let got = shown.layer(id).unwrap();
        assert_eq!(got.advanced.knockout, Knockout::Deep, "{mode}");
        assert!(got.advanced.vector_mask_hides_effects);
        assert!(!got.advanced.blend_clipped);
        assert_eq!(got.excluded_channels, 0b1);
        assert_eq!(got.blend_if.get(gray)[0], BlendRange { black: [10, 30], white: [255, 255] }, "{mode}");
    }
}

#[test]
fn blend_if_values_stay_ordered() {
    assert_eq!(blend_quad(Some(&json!([200, 10, 5, 300]))), [200, 200, 200, 255]);
    assert_eq!(blend_quad(None), [0, 0, 255, 255]);
    assert_eq!(blend_quad(Some(&json!(["x", null]))), [0, 0, 255, 255]);
}

#[test]
fn contour_picker_and_editor_edit_the_effect_curve() {
    let mut s = session();
    s.execute("layer.layerStyle.dropShadow", json!({})).unwrap();
    let shadow = |s: &photocraft_engine::Session| {
        let d = s.active().unwrap();
        match &d.doc.layer(d.active_layer.unwrap()).unwrap().effects.items[0] {
            Effect::DropShadow(sh) => sh.contour.clone(),
            e => panic!("{e:?}"),
        }
    };
    let st = s.active().unwrap();
    let mut f = initial_fields(st.doc.layer(st.active_layer.unwrap()).unwrap(), None, 120.0);
    // The picker: Cone.
    contour::pick(&mut f, "fx1", "contour", "Cone");
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    assert_eq!(shadow(&s), photocraft_engine::layer_style::builtin_contour("Cone").unwrap());

    // The Contour Editor: drag the second point to 10 % / 30 % (between its neighbours on
    // the measured Cone), make the first point a corner.
    contour::open_editor(&mut f, "fx1", "contour");
    assert!(contour::set_point(&mut f, 1, 10.0, 30.0));
    assert!(contour::set_corner(&mut f, 0, true));
    assert!(!contour::set_point(&mut f, 99, 10.0, 10.0), "no such point");
    contour::close_editor(&mut f, true);
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let photocraft_doc::Contour::Custom { name, points, corners } = shadow(&s) else { panic!() };
    assert_eq!(name, "Custom", "an edited preset is no longer called Cone");
    assert!((points[1].input - 0.1).abs() < 3e-3 && (points[1].output - 0.3).abs() < 3e-3, "{points:?}");
    assert_eq!(corners.len(), points.len());
    assert!(corners[0] && corners[1] && !corners[corners.len() - 1], "{corners:?}");

    // Cancel restores the curve the editor opened with.
    let before = f.clone();
    contour::open_editor(&mut f, "fx1", "contour");
    contour::set_point(&mut f, 1, 10.0, 10.0);
    contour::close_editor(&mut f, false);
    assert_eq!(f, before);
}

#[test]
fn percent_fields_round_trip_through_the_engine() {
    let mut s = session();
    s.execute("layer.layerStyle.outerGlow", json!({"spread": 6, "range": 40})).unwrap();
    s.execute("layer.layerStyle.dropShadow", json!({"spread": 12, "add": true})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    let fx = effects_of(&f);
    assert_eq!(fx.len(), 2);
    assert_eq!(fx[0]["params"]["spread"], json!(6.0));
    assert_eq!(fx[0]["params"]["range"], json!(40.0));
    assert_eq!(fx[1]["params"]["spread"], json!(12.0));
}

/// OK with no edits used to turn a Center-source Inner Glow into an Edge one: the dialog never
/// loaded the source, so it showed the default and applied it.
#[test]
fn inner_glow_center_source_survives_ok() {
    let mut s = session();
    s.execute("layer.layerStyle.innerGlow", json!({"source": "center"})).unwrap();
    let source = |s: &photocraft_engine::Session| {
        let d = s.active().unwrap();
        let id = d.active_layer.unwrap();
        d.doc.layer(id).unwrap().effects.items.iter().find_map(|e| match e {
            Effect::InnerGlow(g) => Some(g.source),
            _ => None,
        })
    };
    assert_eq!(source(&s), Some(GlowSource::Center));
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    let glow = effects_of(&f).iter().find(|e| e["kind"] == json!("innerGlow")).unwrap();
    assert_eq!(glow["params"]["source"], json!("center"), "the dialog shows the effect's source");
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    assert_eq!(source(&s), Some(GlowSource::Center), "OK keeps the source");
}

/// #6698: Inner Glow lacked Photoshop's Technique and Range, Outer Glow its Technique. They
/// show in Photoshop's order, the dialog loads them, and OK keeps them.
#[test]
fn glows_show_and_keep_technique_and_range() {
    let keys = |kind: &str| spec(kind).iter().map(|(k, _, _)| *k).collect::<Vec<_>>();
    assert_eq!(keys("innerGlow"), ["blend", "opacity", "color", "technique", "source", "choke", "size", "contour", "range", "noise"]);
    assert_eq!(keys("outerGlow"), ["blend", "opacity", "color", "technique", "spread", "size", "contour", "range", "noise"]);
    let mut s = session();
    s.execute("layer.layerStyle.innerGlow", json!({"technique": "precise", "range": 30})).unwrap();
    s.execute("layer.layerStyle.outerGlow", json!({"technique": "precise"})).unwrap();
    let glows = |s: &photocraft_engine::Session| {
        let d = s.active().unwrap();
        let id = d.active_layer.unwrap();
        d.doc
            .layer(id)
            .unwrap()
            .effects
            .items
            .iter()
            .filter_map(|e| match e {
                Effect::InnerGlow(g) | Effect::OuterGlow(g) => Some((kind_of(e), g.technique, (g.range * 100.0).round())),
                _ => None,
            })
            .collect::<Vec<_>>()
    };
    let expected = [("outerGlow", GlowTechnique::Precise, 50.0), ("innerGlow", GlowTechnique::Precise, 30.0)];
    let mut have = glows(&s);
    have.sort_by_key(|g| g.0);
    let mut want = expected.to_vec();
    want.sort_by_key(|g| g.0);
    assert_eq!(have, want);
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    let inner = effects_of(&f).iter().find(|e| e["kind"] == json!("innerGlow")).unwrap().clone();
    assert_eq!((&inner["params"]["technique"], &inner["params"]["range"]), (&json!("precise"), &json!(30.0)), "the dialog shows them");
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let mut have = glows(&s);
    have.sort_by_key(|g| g.0);
    assert_eq!(have, want, "OK keeps them");
}

#[test]
fn drop_shadow_angle_reaches_the_effect() {
    // #350: the Angle slider was dead — the dialog never sent `useGlobalLight`, so the engine
    // defaulted it to true and the compositor used the fixed global light angle, ignoring the slider.
    let mut s = session();
    let shadow = |s: &photocraft_engine::Session| {
        let d = s.active().unwrap();
        let id = d.active_layer.unwrap();
        d.doc.layer(id).unwrap().effects.items.iter().find_map(|e| match e {
            Effect::DropShadow(sh) => Some(sh.clone()),
            _ => None,
        })
    };

    // Use Global Light off: the per-effect angle is stored and used.
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("dropShadow"), st.doc.global_light.angle);
    set_param(&mut f, "fx1", "angle", json!(45.0));
    set_param(&mut f, "fx1", "useGlobalLight", json!(false));
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let eff = shadow(&s).unwrap();
    assert!(!eff.use_global_light, "Use Global Light off keeps the per-effect angle");
    assert_eq!(eff.angle, 45.0);

    // Use Global Light on: the Angle slider drives the document's shared light angle.
    let st = s.active().unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("dropShadow"), st.doc.global_light.angle);
    set_param(&mut f, "fx1", "useGlobalLight", json!(true));
    set_param(&mut f, "fx1", "angle", json!(30.0));
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    assert_eq!(s.active().unwrap().doc.global_light.angle, 30.0, "the Angle slider moves the shared light");
    assert!(shadow(&s).unwrap().use_global_light);

    // Reopening the dialog shows the effective angle and the checkbox state.
    let st = s.active().unwrap();
    let f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    let fx = effects_of(&f);
    let shadow_entry = fx.iter().find(|e| e["kind"] == json!("dropShadow")).unwrap();
    assert_eq!(shadow_entry["params"]["useGlobalLight"], json!(true));
    assert_eq!(shadow_entry["params"]["angle"], json!(30.0));
}

#[test]
fn bevel_angle_change_is_not_overridden_by_unchanged_drop_shadow_angle() {
    let mut s = session();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    add_instance(&mut f, "bevelEmboss");
    set_param(&mut f, "fx1", "angle", json!(45.0));
    add_instance(&mut f, "dropShadow");
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    assert_eq!(s.active().unwrap().doc.global_light.angle, 45.0);
    let d = s.active().unwrap();
    assert_eq!(d.doc.layer(id).unwrap().effects.items.len(), 2);
}

#[test]
fn bevel_technique_is_selectable_and_round_trips() {
    let Some((_, _, P::Choice(options))) = spec("bevelEmboss").iter().find(|(k, _, _)| *k == "technique") else {
        panic!("Bevel & Emboss has no Technique dropdown");
    };
    let values: Vec<&str> = options.iter().map(|(v, _)| *v).collect();
    assert_eq!(values, ["smooth", "chiselHard", "chiselSoft"]);

    let mut s = session();
    s.execute("layer.layerStyle.bevelEmboss", json!({"technique": "chiselSoft"})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("bevelEmboss"), st.doc.global_light.angle);
    assert_eq!(entry(&f, "fx1").unwrap()["params"]["technique"], json!("chiselSoft"));
    set_param(&mut f, "fx1", "technique", json!("chiselHard"));
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let Effect::BevelEmboss(b) = &s.active().unwrap().doc.layer(id).unwrap().effects.items[0] else { panic!("expected bevel") };
    assert_eq!(b.technique, photocraft_doc::effects::BevelTechnique::ChiselHard);
}

#[test]
fn unchanged_global_light_angle_is_not_applied() {
    let layer = Layer::raster("x", photocraft_doc::PixelFormat::RGBA8);
    let mut f = initial_fields(&layer, None, 47.5);
    add_instance(&mut f, "bevelEmboss");
    assert_eq!(effects_of(&f)[0]["params"]["angle"].as_f64(), Some(47.5), "a fresh effect starts from the shared light",);
    let mut commands = Vec::new();
    apply(&f, |cmd, _| {
        commands.push(cmd.to_string());
        Ok(Value::Null)
    })
    .unwrap();
    assert!(!commands.iter().any(|cmd| cmd == "layer.layerStyle.globalLight"));
}

#[test]
fn last_angle_edit_wins_across_global_light_instances_and_preview() {
    let mut s = session();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let original = st.doc.clone();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("dropShadow"), st.doc.global_light.angle);
    add_instance(&mut f, "innerShadow");
    add_instance(&mut f, "bevelEmboss");
    set_param(&mut f, "fx2", "angle", json!(30.0));
    set_param(&mut f, "fx1", "angle", json!(-90.0));
    let preview = preview_document(&original, &s.patterns, &f).unwrap();
    assert_eq!(preview.global_light.angle, -90.0, "a later instance must not override the edited shadow");
    for e in effects_of(&f) {
        assert_eq!(e["params"]["angle"], json!(-90.0));
    }
    assert_eq!(s.active().unwrap().doc.global_light.angle, original.global_light.angle, "preview does not mutate the document");
    // Adding an effect and disabling the edited effect must preserve the pending light.
    add_instance(&mut f, "dropShadow");
    assert_eq!(effects_of(&f)[3]["params"]["angle"], json!(-90.0));
    entry_mut(&mut f, "fx1").unwrap()["on"] = json!(false);
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    assert_eq!(s.active().unwrap().doc.global_light.angle, -90.0);
    // Moving back to the opening angle also refreshes the preview.
    let before = preview_hash(&f);
    set_param(&mut f, "fx2", "angle", json!(original.global_light.angle));
    assert_ne!(preview_hash(&f), before);
    assert_eq!(preview_document(&original, &s.patterns, &f).unwrap().global_light.angle, original.global_light.angle);
}

#[test]
fn local_angles_stay_independent_and_enabling_global_light_uses_shared_angle() {
    let layer = Layer::raster("x", photocraft_doc::PixelFormat::RGBA8);
    let mut f = initial_fields(&layer, Some("dropShadow"), 120.0);
    add_instance(&mut f, "innerShadow");
    set_param(&mut f, "fx2", "useGlobalLight", json!(false));
    set_param(&mut f, "fx2", "angle", json!(45.0));
    set_param(&mut f, "fx1", "angle", json!(60.0));
    assert_eq!(entry(&f, "fx2").unwrap()["params"]["angle"], json!(45.0));
    set_param(&mut f, "fx2", "useGlobalLight", json!(true));
    assert_eq!(entry(&f, "fx2").unwrap()["params"]["angle"], json!(60.0));
}

#[test]
fn preview_moves_shadow_pixels_when_shared_angle_changes() {
    let mut s = session();
    s.execute("select.rect", json!({"x": 20, "y": 20, "width": 16, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    let st = s.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let mut f = initial_fields(layer, Some("dropShadow"), st.doc.global_light.angle);
    set_param(&mut f, "fx1", "distance", json!(8.0));
    set_param(&mut f, "fx1", "size", json!(0.0));
    set_param(&mut f, "fx1", "opacity", json!(100.0));
    let pixel = |f: &Map<String, Value>, x, y| {
        let preview = preview_document(&st.doc, &s.patterns, f).unwrap();
        photocraft_compose::flatten(&preview).get(x, y)
    };
    set_param(&mut f, "fx1", "angle", json!(0.0));
    assert!(pixel(&f, 14, 28)[0] < 0.01, "light from right casts shadow left");
    set_param(&mut f, "fx1", "angle", json!(90.0));
    assert!(pixel(&f, 14, 28)[0] > 0.99, "the old shadow disappears");
    assert!(pixel(&f, 28, 40)[0] < 0.01, "light from above casts shadow below");
}

#[test]
fn angle_dial_clicks_follow_light_direction() {
    use egui_kittest::kittest::Queryable;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(300.0, 100.0)).build_ui_state(
        |ui, angle| {
            widgets::angle_dial_row(ui, "Angle", angle, -180.0..=180.0);
        },
        120.0_f32,
    );
    h.run();
    let dial = h.get_by_role_and_label(egui::accesskit::Role::Slider, "Angle");
    let rect = dial.rect();
    for (delta, expected) in [(vec2(15.0, 0.0), 0.0), (vec2(0.0, -15.0), 90.0), (vec2(0.0, 15.0), -90.0)] {
        let pos = rect.center() + delta;
        h.event(egui::Event::PointerMoved(pos));
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run();
        assert_eq!(*h.state(), expected);
    }
}

#[test]
fn unedited_bevel_shading_keeps_imported_precision() {
    for size in [None, Some(12.0)] {
        let mut s = session();
        s.execute("layer.layerStyle.bevelEmboss", json!({})).unwrap();
        s.edit("imported shading", |doc, active| {
            let Effect::BevelEmboss(b) = &mut doc.layer_mut(active.unwrap()).unwrap().effects.items[0] else { panic!("bevel") };
            b.highlight_color = photocraft_color::Color::rgb(0.12345, 0.45678, 0.78901);
            b.shadow_color = photocraft_color::Color::rgb(0.98765, 0.54321, 0.13579);
            b.highlight.opacity = 0.3525;
            b.shadow.opacity = 0.6175;
            Ok(())
        })
        .unwrap();
        let st = s.active().unwrap();
        let id = st.active_layer.unwrap();
        let original = st.doc.layer(id).unwrap().effects.items[0].clone();
        let mut fields = initial_fields(st.doc.layer(id).unwrap(), Some("bevelEmboss"), st.doc.global_light.angle);
        if let Some(size) = size {
            set_param(&mut fields, "fx1", "size", json!(size));
        }
        apply(&fields, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
        let Effect::BevelEmboss(before) = original else { panic!("bevel") };
        let Effect::BevelEmboss(after) = &s.active().unwrap().doc.layer(id).unwrap().effects.items[0] else { panic!("bevel") };
        assert_eq!(after.highlight, before.highlight);
        assert_eq!(after.shadow, before.shadow);
        assert_eq!(after.highlight_color, before.highlight_color);
        assert_eq!(after.shadow_color, before.shadow_color);
        if let Some(size) = size {
            assert_eq!(after.size, size, "the unrelated edit still applies");
        }
    }
}

#[test]
fn bevel_shading_round_trips_through_dialog_preview_and_confirm() {
    let mut s = session();
    s.execute(
        "layer.layerStyle.bevelEmboss",
        json!({
            "highlightBlend": "Normal", "highlightColor": "#e08020", "highlightOpacity": 25,
            "shadowBlend": "Multiply", "shadowColor": "#204080", "shadowOpacity": 40
        }),
    )
    .unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut fields = initial_fields(st.doc.layer(id).unwrap(), Some("bevelEmboss"), st.doc.global_light.angle);
    let values = &entry(&fields, "fx1").unwrap()["params"];
    assert_eq!(values["highlightColor"], "#e08020");
    assert_eq!(values["highlightOpacity"], 25.0);
    assert_eq!(values["shadowColor"], "#204080");
    assert_eq!(values["shadowOpacity"], 40.0);

    set_param(&mut fields, "fx1", "highlightBlend", json!("Screen"));
    set_param(&mut fields, "fx1", "highlightOpacity", json!(80.0));
    set_param(&mut fields, "fx1", "shadowColor", json!("#ff0000"));
    assert!(preview_document(&st.doc, &s.patterns, &fields).is_ok());
    apply(&fields, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let effect = &s.active().unwrap().doc.layer(id).unwrap().effects.items[0];
    let Effect::BevelEmboss(bevel) = effect else { panic!("expected bevel") };
    assert_eq!(bevel.highlight.blend, photocraft_doc::BlendMode::Screen);
    assert!((bevel.highlight.opacity - 0.8).abs() < 0.001);
    assert_eq!(hex(&bevel.shadow_color), "#ff0000");
    assert_eq!(hex(&bevel.highlight_color), "#e08020");
}

#[test]
fn preview_off_shows_the_original_until_reenabled() {
    let mut s = session();
    s.execute("layer.layerStyle.colorOverlay", json!({"color": "#ff0000"})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), Some("colorOverlay"), st.doc.global_light.angle);
    // Preview off is the dialog default's complement; edits still change the state.
    f.insert("preview".into(), json!(false));
    let h_off = preview_hash(&f);
    set_param(&mut f, "fx1", "color", json!("#00ff00"));
    assert_ne!(preview_hash(&f), h_off, "editing while preview is off still updates the dialog state");
    // Re-enabling preview applies the pending state (the canvas reads `preview`; the
    // scratch-session apply is the same one the checkbox-on path uses).
    f.insert("preview".into(), json!(true));
    let shown = preview_document(&st.doc, &s.patterns, &f).unwrap();
    let d = s.active().unwrap();
    let color = |doc: &photocraft_doc::Document| match &doc.layer(id).unwrap().effects.items[0] {
        Effect::ColorOverlay { color, .. } => *color,
        other => panic!("{other:?}"),
    };
    assert_eq!(color(&shown), photocraft_color::Color::rgb(0.0, 1.0, 0.0));
    assert_ne!(color(&d.doc), color(&shown), "the document itself is untouched until OK");
    // OK commits regardless of the checkbox.
    f.insert("preview".into(), json!(false));
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let d = s.active().unwrap();
    assert_eq!(color(&d.doc), photocraft_color::Color::rgb(0.0, 1.0, 0.0), "OK commits with Preview off");
}

/// #3648's steps in one dialog session: disable the stroke row, add a Color Overlay,
/// untick Preview. Preview off shows the document as saved — the stroke was never
/// disabled in it, so it keeps rendering until OK, exactly like Photoshop's preview
/// semantics; nothing disabled resurrects, and OK applies the dialog's state.
#[test]
fn issue_steps_disable_stroke_add_overlay_untick_preview() {
    let mut s = session();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ff0000"})).unwrap();
    // Steps 1-3: a stroke is enabled and OK'd; reopening the dialog loads it enabled.
    s.execute("layer.layerStyle.stroke", json!({"size": 40, "position": "inside", "color": "#00ff00"})).unwrap();
    let st = s.active().unwrap();
    let id = st.active_layer.unwrap();
    let mut f = initial_fields(st.doc.layer(id).unwrap(), None, st.doc.global_light.angle);
    let rows = f["effects"].as_array().unwrap();
    assert_eq!(rows.len(), 1, "the saved stroke is the only row");
    assert_eq!((rows[0]["kind"].as_str(), rows[0]["on"].as_bool()), (Some("stroke"), Some(true)));
    // Step 4: disable the stroke row — a pending edit, the document is untouched.
    f["effects"][0]["on"] = json!(false);
    // Step 5: add a Color Overlay (a new instance: no fx snapshot, factory defaults).
    f["effects"].as_array_mut().unwrap().push(json!({
        "id": "fx2",
        "kind": "colorOverlay",
        "on": true,
        "params": {"color": "#0000ff"},
    }));
    // While Preview is on, the live preview honours the in-dialog disable: the stroke
    // is disabled in the scratch document, the overlay renders.
    let shown = preview_document(&st.doc, &s.patterns, &f).unwrap();
    let mut shown_effects = shown.layer(id).unwrap().effects.items.iter();
    assert!(!shown_effects.find(|e| kind_of(e) == "stroke").is_none_or(|e| e.enabled()), "the preview leaves the stroke off");
    assert!(shown_effects.find(|e| kind_of(e) == "colorOverlay").is_some_and(|e| e.enabled()), "the preview shows the overlay");
    // Step 6: untick Preview — the canvas shows the real document, whose stroke is
    // still enabled (the disable is an unapplied edit). This is the state the issue
    // reports; it is the saved state, not a resurrected effect.
    f.insert("preview".into(), json!(false));
    let green: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": 32, "y": 32})).unwrap()).unwrap();
    assert!(green[1] > 0.9 && green[0] < 0.1, "preview off shows the saved stroke: {green:?}");
    // OK (with Preview off) applies the dialog state: the stroke is now disabled in the
    // document too, the overlay shows, and the dialog — which never models the list's
    // master switch — leaves it at its default.
    apply(&f, |cmd, p| s.execute(cmd, p).map_err(|e| e.to_string())).unwrap();
    let d = s.active().unwrap();
    let effects = &d.doc.layer(id).unwrap().effects;
    assert!(!effects.items.iter().find(|e| kind_of(e) == "stroke").is_none_or(|e| e.enabled()), "OK disables the stroke");
    assert!(effects.items.iter().find(|e| kind_of(e) == "colorOverlay").is_some_and(|e| e.enabled()), "OK keeps the overlay on");
    assert!(effects.enabled, "the list's master switch keeps its value");
    let blue: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": 32, "y": 32})).unwrap()).unwrap();
    assert!(blue[2] > 0.9 && blue[1] < 0.1, "after OK only the overlay renders: {blue:?}");
}

#[test]
fn new_style_saves_pending_state_and_a_swatch_apply_restores_it() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let mut f = initial_fields(app.session.active().unwrap().doc.layer(id).unwrap(), Some("stroke"), 120.0);
    set_param(&mut f, "fx1", "size", json!(9));
    set_param(&mut f, BLENDING, "opacity", json!(40));
    save_new_style(&mut app, &f);
    let preset = app.session.presets.styles.iter().flat_map(|g| g.items.iter()).find(|st| st.name == "Style").expect("the preset is created");
    assert_eq!(preset.effects.len(), 1, "only the enabled instance is saved");
    assert_eq!(preset.opacity, Some(0.4), "the pending opacity is saved, not the layer's");
    // A second layer gets the pending style back through the swatch path.
    app.run("layer.new.layer", json!({})).unwrap();
    let mut f2 = initial_fields(app.session.active().unwrap().doc.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap(), None, 120.0);
    f2.insert("layer".into(), json!(app.session.active().unwrap().active_layer.unwrap().0));
    apply_style_preset(&app, &mut f2, "Style");
    // Swatch clicks update the dialog fields only; document is untouched until confirm.
    assert_eq!(effects_of(&f2).len(), 1);
    assert_eq!(effects_of(&f2)[0]["params"]["size"], json!(9.0));
    assert_eq!(f2["p:blendingOptions"]["opacity"], json!(40.0), "the swatch brings the style's opacity");
    // Confirming applies the style to the layer.
    confirm(&mut app, &f2).unwrap();
    let st = app.session.active().unwrap();
    let strokes: Vec<f32> = st
        .doc
        .layer(app.session.active().unwrap().active_layer.unwrap())
        .unwrap()
        .effects
        .items
        .iter()
        .filter_map(|e| match e {
            Effect::Stroke(s) => Some(s.size),
            _ => None,
        })
        .collect();
    assert_eq!(strokes, vec![9.0], "confirming commits the style preset to the layer");
}

#[test]
fn style_click_does_not_mutate_document_until_confirm() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 16, "height": 16})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let id = app.session.active().unwrap().active_layer.unwrap();
    let mut f = initial_fields(app.session.active().unwrap().doc.layer(id).unwrap(), Some(STYLES_PAGE), 120.0);
    apply_style_preset(&app, &mut f, "Black Stroke");
    // Document itself still has 0 effects.
    assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().effects.items.len(), 0);
    // Dialog fields now have the preset loaded.
    assert_eq!(effects_of(&f).len(), 1);
    // Cancel discards dialog; document remains with 0 effects.
    // Confirm commits the style.
    confirm(&mut app, &f).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layer(id).unwrap().effects.items.len(), 1);
}
