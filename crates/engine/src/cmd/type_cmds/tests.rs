use super::*;

fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 100, "background": "transparent"})).unwrap();
    s
}

fn text_layer(s: &Session, id: u64) -> TextLayer {
    match &s.active().unwrap().doc.layer(LayerId(id)).unwrap().content {
        LayerContent::Text(t) => t.clone(),
        other => panic!("{}", other.kind_name()),
    }
}

#[test]
fn named_face_metadata_replaces_old_postscript_identity() {
    let mut style = CharStyle { font_family: "Inter".into(), postscript_name: Some("Inter-Regular".into()), ..Default::default() };
    apply_char_props(&mut style, &json!({"fontStyle": "SemiBold"}));
    assert_eq!(style.weight, 600);
    assert_eq!(style.postscript_name.as_deref(), Some("Inter-SemiBold"));
    apply_char_props(&mut style, &json!({"weight": 400}));
    // #3556: a style change re-resolves to the installed face instead of leaving it nameless.
    assert_eq!(style.postscript_name.as_deref(), Some("Inter-Regular"));
    assert!(style.font_style.is_empty());
    apply_char_props(&mut style, &json!({"fontStyle": "unknown-style"}));
    assert_eq!(style.postscript_name.as_deref(), Some("Inter-Regular"));
    apply_char_props(&mut style, &json!({"fontStyle": "Regular", "postscriptName": "ExplicitFace"}));
    assert_eq!(style.postscript_name.as_deref(), Some("ExplicitFace"));
}

/// #3556: a font named by family stores the installed face's PostScript name, so PSD export
/// names a real font instead of inventing "<Family>-Regular" that Photoshop calls missing.
#[test]
fn a_family_font_gets_the_installed_postscript_name() {
    let mut style = CharStyle { font_family: "Inter".into(), ..Default::default() };
    apply_char_props(&mut style, &json!({"font": "Inter"}));
    assert_eq!(style.font_family, "Inter");
    assert_eq!(style.postscript_name.as_deref(), Some("Inter-Regular"), "resolved from the font database");
    // A weight picks that face's name.
    apply_char_props(&mut style, &json!({"font": "Inter", "weight": 600}));
    assert_eq!(style.postscript_name.as_deref(), Some("Inter-SemiBold"));
    // A weight no face has (a variable font's axis, or between two faces) names no face, so
    // layout still matches the requested weight instead of a nearby face's.
    apply_char_props(&mut style, &json!({"weight": 650}));
    assert!(style.postscript_name.is_none(), "{:?}", style.postscript_name);
    // A family PhotoCraft does not have keeps no name; the export falls back as before.
    let mut absent = CharStyle { font_family: "No Such Family 123".into(), ..Default::default() };
    apply_char_props(&mut absent, &json!({"font": "No Such Family 123"}));
    assert!(absent.postscript_name.is_none());
}

/// Picking a style or weight resets the weight axis to it; other axes keep their values (#2985).
#[test]
fn a_style_or_weight_resets_the_weight_axis() {
    let mut style = CharStyle::default();
    apply_char_props(&mut style, &json!({"variations": {"wght": 650, "wdth": 80}}));
    apply_char_props(&mut style, &json!({"fontStyle": "Bold"}));
    let axes = |s: &CharStyle| s.variations.iter().map(|v| (v.axis.clone(), v.value)).collect::<Vec<_>>();
    assert_eq!(axes(&style), [("wdth".to_string(), 80.0)]);
    apply_char_props(&mut style, &json!({"variations": {"wght": 650}}));
    apply_char_props(&mut style, &json!({"weight": 300}));
    assert_eq!(axes(&style), [("wdth".to_string(), 80.0)]);
    apply_char_props(&mut style, &json!({"fontStyle": "Light", "variations": {"wght": 350}}));
    assert_eq!(axes(&style), [("wdth".to_string(), 80.0), ("wght".to_string(), 350.0)], "same-call values apply after the style");
}

#[test]
fn absurd_type_size_and_tracking_are_rejected_as_bad_params() {
    let mut s = session();
    assert!(matches!(
        s.execute("type.create", json!({"text": "abc", "size": 1e30})),
        Err(EngineError::BadParams { cmd, .. }) if cmd == "type.create"
    ));

    let id = s.execute("type.create", json!({"text": "abc", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let before = text_layer(&s, id);
    for params in [json!({"layer": id, "tracking": 1e30}), json!({"layer": id, "runs": [{"start": 0, "end": 3, "tracking": 1e30}]})] {
        assert!(matches!(
            s.execute("type.edit", params),
            Err(EngineError::BadParams { cmd, .. }) if cmd == "type.edit"
        ));
        assert_eq!(text_layer(&s, id).runs, before.runs);
    }
    assert!(matches!(
        s.execute("type.setStyle", json!({"layer": id, "tracking": 1e30})),
        Err(EngineError::BadParams { cmd, .. }) if cmd == "type.setStyle"
    ));
    assert_eq!(text_layer(&s, id).runs, before.runs);
}

/// A font that arrives after the layer was drawn (served fonts on the web) re-renders it with
/// no history step, and a clean document stays clean.
#[test]
fn refreshing_type_layers_takes_no_history_step() {
    let mut s = session();
    let id = LayerId(s.execute("type.create", json!({"text": "Hello", "size": 24})).unwrap()["layer"].as_u64().unwrap());
    let doc_id = s.active().unwrap().doc.id;
    // Stale pixels, and a document saved in this state.
    {
        let st = s.active_mut().unwrap();
        let Some(Layer { content: LayerContent::Text(t), .. }) = Arc::make_mut(&mut st.doc).layer_mut(id) else { panic!("not a type layer") };
        t.cache = None;
        st.saved_revision = st.revision;
    }
    let rev = s.active().unwrap().revision;
    let unknown = [(doc_id, LayerId(u64::MAX)), (photocraft_doc::DocId(u64::MAX), id)];
    assert!(s.refresh_type_layers(&[&[(doc_id, id)][..], &unknown].concat()).is_empty());
    assert!(text_layer(&s, id.0).cache.is_some());
    let st = s.active().unwrap();
    assert!(st.revision > rev);
    assert_eq!(st.saved_revision, st.revision, "still clean");
    // The only history step is still the creation.
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(id).is_none());
}

#[test]
fn create_renders_and_is_undoable() {
    let mut s = session();
    let r = s.execute("type.create", json!({"x": 10, "y": 50, "text": "Hello", "size": 24, "color": "#ff0000"})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    let b = r["bounds"].as_array().unwrap();
    assert!(b[0].as_i64().unwrap() >= 9 && b[3].as_i64().unwrap() <= 56, "{b:?}");
    let t = text_layer(&s, id);
    assert_eq!(t.text, "Hello");
    assert_eq!(t.runs[0].style.size_pt, 24.0);
    assert!(t.cache.is_some() && t.psd_raw.is_some());
    let l = s.active().unwrap().doc.layer(LayerId(id)).unwrap();
    assert_eq!(l.name, "Hello");
    // Red pixels exist.
    let c = t.cache.as_ref().unwrap();
    let r = c.content_bounds();
    assert!(c.read_region(r).as_chunks::<4>().0.iter().any(|p| p[3] > 0.9 && p[0] > 0.9 && p[1] < 0.1));
    // The TySh we generated parses back to the same model.
    let back = photocraft_text::psd::text_layer_from_tysh(t.psd_raw.as_ref().unwrap(), 72.0).unwrap();
    assert_eq!(back.text, "Hello");
    let strip = |mut r: Vec<TextRun>| {
        for x in &mut r {
            x.style.postscript_name = None;
        }
        r
    };
    assert_eq!(strip(back.char_runs()), strip(t.char_runs()));
    assert_eq!(back.char_runs()[0].style.postscript_name.as_deref(), Some("Inter-Regular"));
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(LayerId(id)).is_none());
}

fn layer_name_of(s: &Session, id: u64) -> String {
    s.active().unwrap().doc.layer(LayerId(id)).unwrap().name.clone()
}

#[test]
fn auto_name_follows_the_text() {
    // The Type tool's flow (#483): a placeholder, selected, then replaced by typing.
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 0, "y": 40, "text": "Lorem Ipsum", "coalesce": "k"})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(layer_name_of(&s, id), "Lorem Ipsum");
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 0, "end": 11, "text": "H"}, "coalesce": "k"})).unwrap();
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 1, "end": 1, "text": "eading\nsecond line"}, "coalesce": "k"})).unwrap();
    assert_eq!(layer_name_of(&s, id), "Heading");
    // Emptied, the name is the default one and keeps following.
    s.execute("type.edit", json!({"layer": id, "text": ""})).unwrap();
    assert_eq!(layer_name_of(&s, id), "Type Layer");
    s.execute("type.edit", json!({"layer": id, "text": "Title"})).unwrap();
    assert_eq!(layer_name_of(&s, id), "Title");
    assert!(s.undo());
    assert_eq!(layer_name_of(&s, id), "Type Layer");
}

#[test]
fn size_keys_step_each_run_and_respect_layer_scale() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 0, "y": 40, "text": "Hello world", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.setStyle", json!({"layer": id, "range": [6, 11], "size": 20})).unwrap();
    s.execute("type.increaseSize", json!({})).unwrap();
    let sizes = |s: &Session| text_layer(s, id).char_runs().iter().map(|r| r.style.size_pt).collect::<Vec<_>>();
    assert_eq!(sizes(&s), vec![14.0, 22.0], "every run grows by 2 pt");
    s.execute("type.decreaseSizeMore", json!({"range": [0, 5]})).unwrap();
    assert_eq!(sizes(&s)[0], 4.0, "only the range, by 10 pt");
    for _ in 0..2 {
        s.execute("type.decreaseSize", json!({"range": [0, 5]})).unwrap();
    }
    assert_eq!(sizes(&s)[0], TYPE_SIZE_MIN_PT as f32, "never below the smallest size");
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(sizes(&s)[0], 2.0, "one undo step per press");

    let scaled = s.execute("type.create", json!({"x": 0, "y": 80, "text": "Big", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.edit", json!({"layer": scaled, "transform": [2.0, 0.0, 0.0, 2.0, 0.0, 80.0]})).unwrap();
    s.execute("type.increaseSize", json!({"layer": scaled})).unwrap();
    assert_eq!(text_layer(&s, scaled).char_runs()[0].style.size_pt, 13.0, "a 200% layer shows 24 pt and steps to 26 pt");
}

#[test]
fn size_keys_need_a_type_layer() {
    let mut s = session();
    assert!(s.execute("type.increaseSize", json!({})).is_err());
    let id = s.execute("type.create", json!({"x": 0, "y": 40, "text": "Hi", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.increaseSize", json!({"by": "big"})).unwrap();
    s.execute("type.increaseSize", json!({"by": -3})).unwrap();
    s.execute("type.decreaseSize", json!({"by": 1e30, "range": [9, 0]})).unwrap();
    assert_eq!(text_layer(&s, id).char_runs()[0].style.size_pt, TYPE_SIZE_MIN_PT as f32, "bad steps fall back or clamp, never panic");
    assert_eq!(crate::commands::find("type.increaseSize").and_then(|c| c.shortcut), Some("Cmd+Shift+."));
    assert_eq!(crate::commands::find("type.decreaseSize").and_then(|c| c.shortcut), Some("Cmd+Shift+,"));
}

#[test]
fn auto_name_skips_leading_blank_lines() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 0, "y": 40, "text": "\n  \nHello\nworld"})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(layer_name_of(&s, id), "Hello");
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 0, "end": 0, "text": "\n"}})).unwrap();
    s.execute("type.edit", json!({"layer": id, "text": "\n\nGoodbye"})).unwrap();
    assert_eq!(layer_name_of(&s, id), "Goodbye");
}

#[test]
fn custom_name_survives_text_edits() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 0, "y": 40, "text": "Hello", "name": "Logo"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.edit", json!({"layer": id, "text": "Goodbye"})).unwrap();
    assert_eq!(layer_name_of(&s, id), "Logo");
    // Renamed after creation: kept too.
    let id = s.execute("type.create", json!({"x": 0, "y": 80, "text": "Hello"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.renameLayer", json!({"layer": id, "name": "Caption"})).unwrap();
    s.execute("type.edit", json!({"layer": id, "text": "Goodbye"})).unwrap();
    assert_eq!(layer_name_of(&s, id), "Caption");
    // A name given with the edit wins over the text.
    s.execute("type.edit", json!({"layer": id, "text": "Again", "name": "Footer"})).unwrap();
    assert_eq!(layer_name_of(&s, id), "Footer");
}

#[test]
fn edit_with_name_is_one_undo_step_for_text_name_and_bounds() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 10, "y": 50, "text": "Before", "size": 20, "name": "Original"})).unwrap()["layer"].as_u64().unwrap();
    let layer = s.active().unwrap().doc.layer(LayerId(id)).unwrap();
    let before_bounds = text_layer(&s, id).cache.unwrap().content_bounds();
    assert_eq!(layer.name, "Original");
    let steps = s.active().unwrap().history.entries().len();

    s.execute("type.edit", json!({"layer": id, "text": "A much longer edited title that wraps", "box": [10, 10, 50, 60], "name": "Renamed"})).unwrap();

    let edited = s.active().unwrap().doc.layer(LayerId(id)).unwrap();
    let edited_bounds = text_layer(&s, id).cache.unwrap().content_bounds();
    assert_eq!(edited.name, "Renamed");
    assert_eq!(text_layer(&s, id).text, "A much longer edited title that wraps");
    assert_ne!(edited_bounds, before_bounds);
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);

    assert!(s.undo());
    let restored = s.active().unwrap().doc.layer(LayerId(id)).unwrap();
    assert_eq!(restored.name, "Original");
    assert_eq!(text_layer(&s, id).text, "Before");
    assert_eq!(text_layer(&s, id).cache.unwrap().content_bounds(), before_bounds);
}

#[test]
fn edit_replace_keeps_styles() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 0, "y": 40, "text": "Hello world"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.setStyle", json!({"layer": id, "range": [6, 11], "fauxBold": true, "color": [0, 0, 1]})).unwrap();
    let t = text_layer(&s, id);
    assert_eq!(t.runs.len(), 2);
    assert_eq!(t.runs[0].len, 6);
    assert!(t.runs[1].style.faux_bold);
    // Insert inside the bold word: inherits bold.
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 8, "end": 8, "text": "XX"}})).unwrap();
    let t = text_layer(&s, id);
    assert_eq!(t.text, "Hello woXXrld");
    assert_eq!(t.runs[1].len, 7);
    // Delete across the style boundary.
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 3, "end": 8, "text": ""}})).unwrap();
    let t = text_layer(&s, id);
    assert_eq!(t.text, "HelXXrld");
    assert_eq!(t.runs.iter().map(|r| r.len).collect::<Vec<_>>(), vec![3, 5]);
    // Whole-text replace with a multi-byte string.
    let info = s.execute("type.edit", json!({"layer": id, "text": "Größe ✓\nzwei"})).unwrap();
    assert_eq!(info["lines"].as_array().unwrap().len(), 2);
    let t = text_layer(&s, id);
    assert_eq!(t.runs.iter().map(|r| r.len).sum::<usize>(), t.text.len());
}

#[test]
fn all_locked_type_edit_rejects_text_replacement_without_history() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 20, "y": 20, "text": "a", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": id, "locked": true})).unwrap();
    let history = s.active().unwrap().history.past_len();

    let result = s.execute("type.edit", json!({"layer": id, "text": "b"}));
    assert!(matches!(&result, Err(EngineError::Other(message)) if message.contains("locked")), "{result:?}");
    assert_eq!(text_layer(&s, id).text, "a");
    assert_eq!(s.active().unwrap().history.past_len(), history);

    s.execute("layer.setProps", json!({"layer": id, "locked": false})).unwrap();
    let history = s.active().unwrap().history.past_len();
    s.execute("type.edit", json!({"layer": id, "text": "b"})).unwrap();
    assert_eq!(text_layer(&s, id).text, "b");
    assert_eq!(s.active().unwrap().history.past_len(), history + 1);
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(text_layer(&s, id).text, "a");
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

#[test]
fn all_locked_type_layer_rejects_formatting_without_history() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 20, "y": 20, "text": "a", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let other = s.execute("type.create", json!({"x": 20, "y": 60, "text": "b", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.setProps", json!({"layer": id, "locked": true})).unwrap();
    let before = text_layer(&s, id);
    let history = s.active().unwrap().history.past_len();

    for (cmd, p) in [
        ("type.setStyle", json!({"layer": id, "size": 40, "fauxBold": true})),
        ("type.setStyle", json!({"layers": [other, id], "size": 40})),
        ("type.warpText", json!({"layer": id, "style": "arc"})),
        ("type.antiAlias.none", json!({"layer": id})),
        ("type.orientation.vertical", json!({"layer": id})),
        ("type.openType.swash", json!({"layer": id})),
    ] {
        let result = s.execute(cmd, p);
        assert!(matches!(&result, Err(EngineError::Other(message)) if message.contains("locked")), "{cmd}: {result:?}");
    }
    assert_eq!(text_layer(&s, id), before);
    assert_eq!(text_layer(&s, other).size_pt, 12.0);
    assert_eq!(s.active().unwrap().history.past_len(), history);

    s.execute("layer.setProps", json!({"layer": id, "locked": false})).unwrap();
    s.execute("type.setStyle", json!({"layer": id, "size": 40})).unwrap();
    assert_eq!(text_layer(&s, id).size_pt, 40.0);
}

#[test]
fn trailing_empty_paragraph_styles_without_changing_previous_lines() {
    let mut s = session();
    let id = s.execute("type.create", json!({"text":"a\nb\n","x":20,"y":20,"size":12,"font":"Inter","align":"left"})).unwrap()["layer"].as_u64().unwrap();
    let before = s.execute("type.info", json!({"layer":id})).unwrap();
    let previous_line = before["lines"][1].clone();

    let styled = s.execute("type.setStyle", json!({"layer":id,"range":[4,4],"align":"right"})).unwrap();
    assert_eq!(styled["paragraphs"].as_array().unwrap().len(), 2);
    assert_eq!(styled["paragraphs"][0]["start"], 0);
    assert_eq!(styled["paragraphs"][0]["end"], 4);
    assert_eq!(styled["paragraphs"][0]["style"]["align"], "Left");
    assert_eq!(styled["paragraphs"][1]["start"], 4);
    assert_eq!(styled["paragraphs"][1]["end"], 4);
    assert_eq!(styled["paragraphs"][1]["style"]["align"], "Right");
    assert_eq!(styled["lines"][1], previous_line, "the preceding paragraph must not move");
    let raw = text_layer(&s, id).psd_raw.clone().unwrap();
    let round_trip = photocraft_text::psd::text_layer_from_tysh(&raw, 72.0).unwrap();
    assert_eq!(round_trip.paragraph_runs().last().unwrap().style.align, TextAlign::Right);

    // Typing directly into the previously empty final paragraph retains its style.
    let edited = s.execute("type.edit", json!({"layer":id,"replace":{"start":4,"end":4,"text":"c"}})).unwrap();
    assert_eq!(edited["text"], "a\nb\nc");
    assert_eq!(edited["paragraphs"][0]["end"], 4);
    assert_eq!(edited["paragraphs"][0]["style"]["align"], "Left");
    assert_eq!(edited["paragraphs"][1]["start"], 4);
    assert_eq!(edited["paragraphs"][1]["end"], 5);
    assert_eq!(edited["paragraphs"][1]["style"]["align"], "Right");
    assert_eq!(edited["lines"][1], previous_line, "typing in the final paragraph does not move b");
}

#[test]
fn set_style_paragraph_and_box() {
    let mut s = session();
    let id =
        s.execute("type.create", json!({"box": [10, 10, 80, 80], "text": "one two three four five six seven", "size": 12})).unwrap()["layer"].as_u64().unwrap();
    let info = s.execute("type.setStyle", json!({"layer": id, "align": "center", "leading": 20})).unwrap();
    let lines = info["lines"].as_array().unwrap();
    assert!(lines.len() >= 3);
    let b0 = lines[0]["baseline"].as_f64().unwrap();
    let b1 = lines[1]["baseline"].as_f64().unwrap();
    assert!((b1 - b0 - 20.0).abs() < 1e-3, "explicit leading {b0} {b1}");
    assert_eq!(info["paragraphs"][0]["style"]["align"], "Center");
    let t = text_layer(&s, id);
    assert!(matches!(t.shape, TextShape::Box { width, .. } if width == 80.0));
    // Pixels stay inside the box horizontally.
    let r = t.cache.as_ref().unwrap().content_bounds();
    assert!(r.x0 >= 9 && r.x1 <= 91, "{r:?}");
    // Convert to point text and move.
    s.execute("type.edit", json!({"layer": id, "point": [5, 30]})).unwrap();
    s.execute("type.edit", json!({"layer": id, "move": [10, 0]})).unwrap();
    let t = text_layer(&s, id);
    assert_eq!(t.shape, TextShape::Point);
    assert_eq!(t.transform.m[4], 15.0);
}

#[test]
fn rasterize_and_fonts() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 5, "y": 30, "text": "Raster"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.rasterize", json!({"layer": id})).unwrap();
    let l = s.active().unwrap().doc.layer(LayerId(id)).unwrap();
    assert!(matches!(l.content, LayerContent::Raster(_)));
    assert!(l.surface().unwrap().content_bounds().width() > 10);
    assert!(s.execute("type.edit", json!({"layer": id, "text": "x"})).is_err());
    let fonts = s.execute("type.fonts", json!({})).unwrap();
    assert!(fonts["families"].as_array().unwrap().iter().any(|f| f == "Inter"));
    let faces = s.execute("type.fonts", json!({"family": "Inter"})).unwrap();
    assert!(!faces["faces"].as_array().unwrap().is_empty());
}

/// #124: a type edit damages only the layer's old and new pixels (the canvas recomposites
/// that, not the document), and a drag's coalesced steps are one history step.
#[test]
fn type_edits_damage_only_the_layer_and_coalesce() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 3000, "height": 2000})).unwrap();
    let id = s.execute("type.create", json!({"x": 100, "y": 100, "text": "Hi", "size": 20})).unwrap()["layer"].as_u64().unwrap();
    let steps = s.active().unwrap().history.entries().len();
    for k in 0..5 {
        s.execute("type.setStyle", json!({"layer": id, "size": 30 + k * 10, "coalesce": "drag"})).unwrap();
        let d = s.active().unwrap().last_damage.expect("damage rect");
        let r = text_layer(&s, id).cache.unwrap().content_bounds();
        assert!(d.intersect(&r) == r && d.width() <= 512 && d.height() <= 512, "{d:?} {r:?}");
    }
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1);
    assert!(s.undo());
    assert_eq!(text_layer(&s, id).runs[0].style.size_pt, 20.0);
}

#[test]
fn edit_rejects_a_reversed_runs_range() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 0, "y": 40, "text": "Hello world", "size": 20})).unwrap()["layer"].as_u64().unwrap();
    let before = text_layer(&s, id).char_runs();
    let steps = s.active().unwrap().history.entries().len();
    for runs in [
        json!([{"start": 5, "end": 2, "fauxBold": true}]),
        json!([{"start": 0, "end": 3, "fauxBold": true}, {"start": 8, "end": 7}]),
        json!([{"end": 0, "start": 1}]),
    ] {
        let e = s.execute("type.edit", json!({"layer": id, "runs": runs})).unwrap_err();
        assert!(matches!(e, EngineError::BadParams { .. }) && e.to_string().contains("is before `start`"), "{e}");
    }
    // Nothing changed and nothing was recorded, not even the valid run before the bad one.
    assert_eq!(text_layer(&s, id).char_runs(), before);
    assert_eq!(s.active().unwrap().history.entries().len(), steps);
    // The same range the right way round still styles it.
    s.execute("type.edit", json!({"layer": id, "runs": [{"start": 2, "end": 5, "fauxBold": true}]})).unwrap();
    let runs = text_layer(&s, id).char_runs();
    assert_eq!(runs.iter().map(|r| (r.len, r.style.faux_bold)).collect::<Vec<_>>(), [(2, false), (3, true), (6, false)]);
}

#[test]
fn style_range_reversed_is_a_no_op() {
    let mut t = TextLayer::default();
    replace_text(&mut t, 0, 0, "Hello world");
    let before = t.char_runs();
    style_range(&mut t, 5, 2, &|s| s.size_pt = 40.0);
    assert_eq!(t.char_runs(), before);
    assert_eq!(t.runs.iter().map(|r| r.len).sum::<usize>(), t.text.len());
}

#[test]
fn style_range_on_empty_text() {
    let mut t = TextLayer::default();
    style_range(&mut t, 0, 0, &|s| s.size_pt = 40.0);
    assert_eq!(t.runs[0].style.size_pt, 40.0);
    replace_text(&mut t, 0, 0, "abc");
    assert_eq!(t.runs.len(), 1);
    assert_eq!(t.runs[0].len, 3);
    assert_eq!(t.runs[0].style.size_pt, 40.0);
}

/// Replacing a selected smaller second line (by typing or pasting) keeps its size, not the
/// first line's, whose line break precedes the selection.
#[test]
fn replacing_a_selection_keeps_its_first_characters_style() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 5, "y": 50, "text": "BIG\nsmall", "size": 40})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.edit", json!({"layer": id, "runs": [{"start": 4, "end": 9, "size": 12}]})).unwrap();
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 4, "end": 9, "text": "AIR"}})).unwrap();
    let sizes = |s: &Session| text_layer(s, id).char_runs().iter().map(|r| (r.len, r.style.size_pt)).collect::<Vec<_>>();
    assert_eq!(text_layer(&s, id).text, "BIG\nAIR");
    assert_eq!(sizes(&s), [(4, 40.0), (3, 12.0)]);
    // A mixed selection takes its first character's style.
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 2, "end": 6, "text": "xy"}})).unwrap();
    assert_eq!(sizes(&s), [(4, 40.0), (1, 12.0)]);
    // Inserting at a caret still follows the character before it.
    s.execute("type.edit", json!({"layer": id, "replace": {"start": 4, "end": 4, "text": "!"}})).unwrap();
    assert_eq!(sizes(&s), [(5, 40.0), (1, 12.0)]);
}

/// A misspelt `baseline` is an error and keeps superscript, instead of resetting it to Normal.
#[test]
fn unknown_baseline_is_rejected_and_keeps_the_style() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 5, "y": 50, "text": "x2", "size": 30})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.setStyle", json!({"layer": id, "range": [1, 2], "baseline": "superscript"})).unwrap();
    let before = text_layer(&s, id).runs;
    for b in [json!("superscipt"), json!("Superscript"), json!(1)] {
        assert!(matches!(s.execute("type.setStyle", json!({"layer": id, "baseline": b})), Err(EngineError::BadParams { .. })), "{b}");
        assert!(s.execute("type.edit", json!({"layer": id, "baseline": b})).is_err(), "{b}");
        assert!(s.execute("type.create", json!({"x": 0, "y": 10, "text": "a", "baseline": b})).is_err(), "{b}");
    }
    assert_eq!(text_layer(&s, id).runs, before);
    s.execute("type.setStyle", json!({"layer": id, "baseline": "normal"})).unwrap();
    assert!(text_layer(&s, id).char_runs().iter().all(|r| r.style.baseline == FontBaseline::Normal));
}

fn kerning_of(s: &Session, id: u64) -> Vec<(Kerning, f32)> {
    let t = text_layer(s, id);
    let mut out = Vec::new();
    let mut at = 0;
    for r in t.char_runs() {
        out.extend(std::iter::repeat_n((r.style.kerning, r.style.kern), t.text[at..at + r.len].chars().count()));
        at += r.len;
    }
    out
}

/// #8489: `type.edit {"text"}` changes only what differs, so the rest keeps its styles: an
/// added character takes the style beside it, and a change inside one run stays in it.
#[test]
fn edit_text_keeps_the_styles_of_unchanged_text() {
    // (characters, colour) per run.
    fn runs(s: &Session, id: u64) -> Vec<(usize, [u8; 4])> {
        let t = text_layer(s, id);
        let mut at = 0;
        t.char_runs()
            .into_iter()
            .map(|r| {
                let n = t.text[at..at + r.len].chars().count();
                at += r.len;
                (n, r.style.color.to_rgba8())
            })
            .collect()
    }
    let (dark, purple) = ([0x1b, 0x1f, 0x4b, 255], [0x5a, 0x3d, 0xf0, 255]);
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 20, "y": 50, "text": "Craft Suite in Workshop", "size": 40, "color": "#1b1f4b"})).unwrap()["layer"]
        .as_u64()
        .unwrap();
    s.execute("type.setStyle", json!({"layer": id, "range": [15, 23], "color": "#5a3df0"})).unwrap();
    assert_eq!(runs(&s, id), [(15, dark), (8, purple)]);
    s.execute("type.edit", json!({"layer": id, "text": "Craft Suite in Workshop!"})).unwrap();
    assert_eq!(runs(&s, id), [(15, dark), (9, purple)], "the added character joins the last run");
    s.execute("type.edit", json!({"layer": id, "text": "Craft Suite in Worksh\u{f6}p!"})).unwrap();
    assert_eq!(runs(&s, id), [(15, dark), (9, purple)], "a change inside a run stays in it");
    s.execute("type.edit", json!({"layer": id, "text": "Kraft Suite in Worksh\u{f6}p!"})).unwrap();
    assert_eq!(runs(&s, id), [(15, dark), (9, purple)]);
    // Replacing everything leaves one style, as before.
    s.execute("type.edit", json!({"layer": id, "text": "Hello"})).unwrap();
    assert_eq!(text_layer(&s, id).text, "Hello");
    assert_eq!(runs(&s, id).iter().map(|r| r.0).sum::<usize>(), 5);
    s.execute("type.edit", json!({"layer": id, "text": ""})).unwrap();
    assert_eq!(text_layer(&s, id).text, "");

    // Text inserted at the start of a paragraph joins that paragraph, which keeps its style.
    let id = s
        .execute(
            "type.create",
            json!({"x": 20, "y": 50, "text": "a
b", "size": 20}),
        )
        .unwrap()["layer"]
        .as_u64()
        .unwrap();
    s.execute("type.setStyle", json!({"layer": id, "range": [2, 3], "align": "right"})).unwrap();
    s.execute(
        "type.edit",
        json!({"layer": id, "text": "a
Xb"}),
    )
    .unwrap();
    let paragraphs: Vec<(usize, photocraft_doc::text::TextAlign)> = text_layer(&s, id).paragraph_runs().into_iter().map(|r| (r.len, r.style.align)).collect();
    assert_eq!(paragraphs.first().map(|p| p.0), Some(2), "{paragraphs:?}");
    assert_eq!(paragraphs.get(1).copied(), Some((2, photocraft_doc::text::TextAlign::Right)), "{paragraphs:?}");
}

/// #206: `type.edit {"kerning"}` over a range takes a number (manual, 1/1000 em) or a mode.
#[test]
fn edit_sets_kerning_over_a_range() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 5, "y": 50, "text": "AVAT", "size": 30})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.edit", json!({"layer": id, "range": [0, 1], "kerning": 100})).unwrap();
    s.execute("type.edit", json!({"layer": id, "range": [2, 4], "kerning": "optical"})).unwrap();
    assert_eq!(kerning_of(&s, id), vec![(Kerning::Off, 100.0), (Kerning::Metrics, 0.0), (Kerning::Optical, 0.0), (Kerning::Optical, 0.0)]);
    s.execute("type.edit", json!({"layer": id, "kerning": "metrics"})).unwrap();
    assert!(kerning_of(&s, id).iter().all(|k| *k == (Kerning::Metrics, 0.0)));
    // Runs and setStyle take the same values; type.info reports them.
    s.execute("type.edit", json!({"layer": id, "runs": [{"start": 1, "end": 2, "kerning": -50}]})).unwrap();
    s.execute("type.setStyle", json!({"layer": id, "range": [3, 4], "kerning": "off"})).unwrap();
    assert_eq!(kerning_of(&s, id)[1], (Kerning::Off, -50.0));
    assert_eq!(kerning_of(&s, id)[3], (Kerning::Off, 0.0));
    let info = s.execute("type.info", json!({"layer": id})).unwrap();
    assert_eq!(info["runs"][1]["style"]["kern"], json!(-50.0));
}

/// Bad kerning params are errors, never panics, and leave the layer alone.
#[test]
fn kerning_params_fail_gracefully() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 5, "y": 50, "text": "AV\nT", "size": 30})).unwrap()["layer"].as_u64().unwrap();
    let before = text_layer(&s, id);
    for p in [
        json!({"kerning": "tight"}),
        json!({"kerning": 1e9}),
        json!({"kerning": -5000}),
        json!({"kerning": true}),
        json!({"kerning": [1]}),
        json!({"runs": [{"start": 0, "end": 1, "kerning": "x"}]}),
        json!({"kernPair": 3}),
        json!({"kernPair": {"by": 20}}),
        json!({"kernPair": {"at": 1}}),
        json!({"kernPair": {"at": 1, "by": "20"}}),
        json!({"kernPair": {"at": 1, "by": 1e12}}),
        json!({"kernPair": {"at": 0, "by": 20}}),
        json!({"kernPair": {"at": 4, "by": 20}}),
        json!({"kernPair": {"at": 99, "by": 20}}),
        json!({"kernPair": {"at": 2, "by": 20}}),
        json!({"kernPair": {"at": 3, "by": 20}}),
        json!({"kernPair": {"at": -1, "by": 20}}),
    ] {
        let mut p = p;
        p["layer"] = json!(id);
        assert!(s.execute("type.edit", p.clone()).is_err(), "{p}");
    }
    assert!(s.execute("type.setStyle", json!({"layer": id, "kerning": "tight"})).is_err());
    assert_eq!(text_layer(&s, id).runs, before.runs);
    // type.create rejects the same values and creates no layer (#994).
    let layers = s.active().unwrap().doc.layer_count();
    for k in [json!("tight"), json!(1e9), json!(-5000), json!(true), json!([1])] {
        let p = json!({"x": 0, "y": 10, "text": "AV", "kerning": k});
        assert!(s.execute("type.create", p.clone()).is_err(), "{p}");
    }
    assert_eq!(s.active().unwrap().doc.layer_count(), layers);
    let id = s.execute("type.create", json!({"x": 0, "y": 10, "text": "AV", "kerning": "optical"})).unwrap()["layer"].as_u64().unwrap();
    assert!(kerning_of(&s, id).iter().all(|k| *k == (Kerning::Optical, 0.0)));
    let id = s.execute("type.create", json!({"x": 0, "y": 10, "text": "AV", "kerning": -50})).unwrap()["layer"].as_u64().unwrap();
    assert!(kerning_of(&s, id).iter().all(|k| *k == (Kerning::Off, -50.0)));
}

/// Cost of one Alt+←/→ press (`kernPair`: two measuring layouts + the edit's re-render) on
/// a headline and on a 2000-character paragraph. Release timings for the dev log:
/// `cargo test --release -p photocraft-engine kern_pair_cost -- --ignored --nocapture`.
#[test]
#[ignore]
fn kern_pair_cost() {
    let para = "The quick brown fox jumps over the lazy dog. ".repeat(45);
    for (name, text, size, bx, kerning) in [
        ("headline", "AVATAR Wave", 72, false, "metrics"),
        ("paragraph", para.as_str(), 12, true, "metrics"),
        ("optical paragraph", para.as_str(), 12, true, "optical"),
    ] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 2000, "height": 1500})).unwrap();
        let mut p = json!({"x": 20, "y": 100, "text": text, "size": size, "font": "Inter", "kerning": kerning});
        if bx {
            p["box"] = json!([20, 20, 900, 1400]);
        }
        let id = s.execute("type.create", p).unwrap()["layer"].as_u64().unwrap();
        let n = 20;
        let t0 = std::time::Instant::now();
        for i in 0..n {
            s.execute("type.edit", json!({"layer": id, "kernPair": {"at": 1 + i % 3, "by": 20}})).unwrap();
        }
        let per = t0.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
        let t1 = std::time::Instant::now();
        for i in 0..n {
            s.execute("type.setStyle", json!({"layer": id, "range": [1 + i % 3, 2 + i % 3], "tracking": i})).unwrap();
        }
        let base = t1.elapsed().as_secs_f64() * 1000.0 / f64::from(n);
        eprintln!("{name} ({} chars): kernPair {per:.2} ms/press; a plain style edit {base:.2} ms", text.chars().count());
    }
}

/// Photoshop's Alt+←/→: the pair before the caret becomes manual, starting from the kerning
/// it shows (the font's pair kerning for "AV"), ±20 per press, one history step each.
#[test]
fn kern_pair_steps_from_the_shown_kerning() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 5, "y": 50, "text": "AVA", "size": 40, "font": "Inter"})).unwrap()["layer"].as_u64().unwrap();
    let metric = {
        let t = text_layer(&s, id);
        photocraft_text::shared().lock().unwrap().pair_kerning(&t, 72.0, 0).unwrap()
    };
    assert!(metric < -10.0, "Inter kerns AV: {metric}");
    let steps = s.active().unwrap().history.entries().len();
    s.execute("type.edit", json!({"layer": id, "kernPair": {"at": 1, "by": 20}})).unwrap();
    let k0 = kerning_of(&s, id)[0];
    assert_eq!(k0, (Kerning::Off, metric.round() + 20.0));
    s.execute("type.edit", json!({"layer": id, "kernPair": {"at": 1, "by": 100}})).unwrap();
    assert_eq!(kerning_of(&s, id)[0].1, metric.round() + 120.0);
    s.execute("type.edit", json!({"layer": id, "kernPair": {"at": 1, "by": -20}})).unwrap();
    assert_eq!(kerning_of(&s, id)[0].1, metric.round() + 100.0);
    // Other characters are untouched; each press is its own undoable step.
    assert_eq!(&kerning_of(&s, id)[1..], &[(Kerning::Metrics, 0.0), (Kerning::Metrics, 0.0)]);
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 3);
    assert!(s.undo());
    assert_eq!(kerning_of(&s, id)[0].1, metric.round() + 120.0);
    assert!(s.undo() && s.undo());
    assert_eq!(kerning_of(&s, id)[0], (Kerning::Metrics, 0.0));
}
#[test]
fn create_vertical_type_is_atomic_and_invalid_orientation_is_rejected() {
    for boxed in [false, true] {
        let mut s = session();
        let before = s.active().unwrap().history.entries().len();
        for orientation in [json!("diagonal"), json!(null), json!(true)] {
            assert!(s.execute("type.create", json!({"text": "test", "orientation": orientation})).is_err());
            assert_eq!(s.active().unwrap().history.entries().len(), before);
        }
        let mut p = json!({"text": "test", "orientation": "vertical", "x": 20, "y": 20});
        if boxed {
            p["box"] = json!([20, 20, 80, 60]);
        }
        let id = s.execute("type.create", p).unwrap()["layer"].as_u64().unwrap();
        let t = text_layer(&s, id);
        assert_eq!(t.orientation, Orientation::Vertical);
        assert_eq!(matches!(t.shape, TextShape::Box { .. }), boxed);
        s.execute("edit.undo", json!({})).unwrap();
        assert!(s.active().unwrap().doc.layer(LayerId(id)).is_none());
    }
}

#[test]
fn missing_font_emits_warning_in_create_set_style_and_edit() {
    let mut s = session();
    let r = s.execute("type.create", json!({"x": 10, "y": 20, "text": "Hello", "font": "NoSuchFamily123"})).unwrap();
    assert_eq!(r["warnings"], json!(["font 'NoSuchFamily123' is not installed; text may render using a fallback face"]));
    let id = r["layer"].as_u64().unwrap();
    let styled = s.execute("type.setStyle", json!({"layer": id, "font": "StillNoSuchFamily456"})).unwrap();
    assert_eq!(styled["warnings"], json!(["font 'StillNoSuchFamily456' is not installed; text may render using a fallback face"]));
    let edited = s.execute("type.edit", json!({"layer": id, "runs": [{"start": 0, "end": 2, "font": "AnotherMissingFont789"}]})).unwrap();
    assert_eq!(
        edited["warnings"],
        json!([
            "font 'AnotherMissingFont789' is not installed; text may render using a fallback face",
            "font 'StillNoSuchFamily456' is not installed; text may render using a fallback face"
        ])
    );
}

/// `leadingBy`, `baselineShiftBy` and `trackingBy` step every character from its own value
/// (auto leading from its laid-out value), clamp to Photoshop's ranges, and reject hostile
/// steps without changing anything.
#[test]
fn relative_steps_move_each_character_from_its_own_value() {
    let mut s = session();
    let id = s.execute("type.create", json!({"x": 0, "y": 40, "text": "Hello\nworld", "size": 10})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.setStyle", json!({"layer": id, "range": [8, 11], "leading": 30, "tracking": 50})).unwrap();
    s.execute("type.setStyle", json!({"layer": id, "range": [6, 11], "autoLeading": 150})).unwrap();
    let steps = s.active().unwrap().history.entries().len();
    s.execute("type.setStyle", json!({"layer": id, "range": [0, 11], "leadingBy": 2, "baselineShiftBy": -2, "trackingBy": 20})).unwrap();
    assert_eq!(s.active().unwrap().history.entries().len(), steps + 1, "one history step");
    let t = text_layer(&s, id);
    let at = |c: usize| {
        let byte = byte_at(&t.text, c);
        let mut off = 0;
        t.char_runs()
            .into_iter()
            .find(|r| {
                off += r.len;
                off > byte
            })
            .unwrap()
            .style
    };
    // Auto leading 120% × 10 pt (first paragraph), 150% × 10 pt (second), and a set 30 pt.
    assert_eq!(at(0).leading_pt, Some(14.0));
    assert_eq!(at(6).leading_pt, Some(17.0));
    assert_eq!(at(9).leading_pt, Some(32.0));
    assert_eq!((at(0).baseline_shift_pt, at(9).baseline_shift_pt), (-2.0, -2.0));
    assert_eq!((at(0).tracking, at(9).tracking), (20.0, 70.0));
    // Clamped to the ranges.
    s.execute("type.setStyle", json!({"layer": id, "leadingBy": -5000, "baselineShiftBy": 10000, "trackingBy": -11000})).unwrap();
    let t = text_layer(&s, id);
    assert!(
        t.char_runs().iter().all(|r| r.style.leading_pt == Some(LEADING_MIN_PT)
            && r.style.baseline_shift_pt == BASELINE_SHIFT_MAX_PT
            && r.style.tracking == TRACKING_MIN as f32)
    );
    // A collapsed range in non-empty text changes no character.
    let before = text_layer(&s, id).char_runs();
    s.execute("type.setStyle", json!({"layer": id, "range": [3, 3], "trackingBy": 100})).unwrap();
    assert_eq!(text_layer(&s, id).char_runs(), before);
    // Hostile steps are refused.
    let steps = s.active().unwrap().history.entries().len();
    for bad_step in [json!({"leadingBy": "up"}), json!({"trackingBy": 1e30}), json!({"baselineShiftBy": null}), json!({"leadingBy": [1]})] {
        let mut p = bad_step.clone();
        p["layer"] = json!(id);
        assert!(matches!(s.execute("type.setStyle", p), Err(EngineError::BadParams { .. })), "{bad_step}");
    }
    assert_eq!(s.active().unwrap().history.entries().len(), steps);
    // Empty text: the step sets the style the next typed characters get.
    let empty = s.execute("type.create", json!({"x": 0, "y": 80, "text": "", "size": 10})).unwrap()["layer"].as_u64().unwrap();
    s.execute("type.setStyle", json!({"layer": empty, "range": [0, 0], "leadingBy": -2})).unwrap();
    assert_eq!(text_layer(&s, empty).char_runs()[0].style.leading_pt, Some(10.0));
}
