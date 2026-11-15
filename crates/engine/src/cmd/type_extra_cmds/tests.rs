use super::*;

fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 300, "height": 120, "depth": depth, "background": "white"})).unwrap();
    s
}

fn text_layer(s: &mut Session, text: &str) -> LayerId {
    let r = s.execute("type.create", json!({"x": 20, "y": 70, "text": text, "size": 36, "color": "#000000"})).unwrap();
    LayerId(r["layer"].as_u64().unwrap())
}

fn text(s: &Session, id: LayerId) -> &TextLayer {
    match &s.active().unwrap().doc.layer(id).unwrap().content {
        LayerContent::Text(t) => t,
        c => panic!("not text: {}", c.kind_name()),
    }
}

fn ink(s: &Session, id: LayerId) -> photocraft_geom::Rect {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds()
}

#[test]
fn anti_alias_and_orientation_are_one_step_each() {
    let mut s = session(8);
    assert!(!s.is_enabled("type.antiAlias.crisp"));
    let id = text_layer(&mut s, "Aa");
    for (cmd, aa) in [
        ("type.antiAlias.crisp", AntiAlias::Crisp),
        ("type.antiAlias.windows", AntiAlias::Windows),
        ("type.antiAlias.windowsLcd", AntiAlias::WindowsLcd),
        ("type.antiAlias.none", AntiAlias::None),
    ] {
        s.execute(cmd, json!({})).unwrap();
        assert_eq!(text(&s, id).antialias, aa);
    }
    // No anti-aliasing: only fully covered or empty pixels.
    let surf = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone();
    let r = surf.content_bounds();
    let n = surf.channels();
    assert!(surf.read_region(r).chunks_exact(n).all(|p| p[n - 1] == 0.0 || p[n - 1] == 1.0));
    s.execute("type.orientation.vertical", json!({})).unwrap();
    assert_eq!(text(&s, id).orientation, Orientation::Vertical);
    assert!(s.undo());
    assert_eq!(text(&s, id).orientation, Orientation::Horizontal);
    assert!(s.undo());
    assert_eq!(text(&s, id).antialias, AntiAlias::WindowsLcd);
}

#[test]
fn point_and_paragraph_conversion_keep_the_text_in_place() {
    for align in ["left", "center", "right"] {
        let mut s = session(8);
        let r = s.execute("type.create", json!({"x": 150, "y": 70, "text": "Hello world", "size": 30, "align": align})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        let before = ink(&s, id);
        assert!(!s.is_enabled("type.convertToPointText"));
        s.execute("type.convertToParagraphText", json!({})).unwrap();
        assert!(matches!(text(&s, id).shape, TextShape::Box { .. }));
        let after = ink(&s, id);
        assert!((after.x0 - before.x0).abs() <= 2 && (after.y0 - before.y0).abs() <= 2, "{align}: {before:?} → {after:?}");
        let lines = |s: &Session| {
            let st = s.active().unwrap();
            layout(&st.doc, text(s, id)).lines.len()
        };
        assert_eq!(lines(&s), 1, "{align}: no new wraps");
        s.execute("type.convertToPointText", json!({})).unwrap();
        assert!(matches!(text(&s, id).shape, TextShape::Point));
        let back = ink(&s, id);
        assert!((back.x0 - before.x0).abs() <= 2 && (back.y0 - before.y0).abs() <= 2, "{align}: {before:?} → {back:?}");
    }
}

/// Issue #446: vertical type renders vertically (see the test below), so the command's description
/// must not claim it stays horizontal.
#[test]
fn vertical_orientation_params_match_horizontal() {
    let specs = specs();
    let params = |id: &str| specs.iter().find(|c| c.id == id).unwrap().params;
    assert_eq!(params("type.orientation.vertical"), params("type.orientation.horizontal"));
}

/// Issue #199: vertical type is laid out top to bottom, columns right to left, on the canvas.
#[test]
fn vertical_orientation_renders_columns_inside_the_canvas() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1000, "height": 600, "background": "#fff8e7", "name": "flyer"})).unwrap();
    // The issue's repro (its font may be missing here: fallback fonts then cover the text).
    let r = s.execute("type.create", json!({"x": 900, "y": 30, "text": "縦書きテスト", "size": 40, "font": "Noto Sans CJK JP", "name": "vertical"})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    let flat = ink(&s, id);
    assert!(flat.width() > flat.height(), "horizontal first: {flat:?}");
    let v = s.execute("type.orientation.vertical", json!({})).unwrap();
    assert_eq!(v, json!({"orientation": "vertical"}));
    let b = ink(&s, id);
    assert!(b.height() > b.width(), "{b:?}");
    assert!(b.x0 >= 0 && b.x1 <= 1000 && b.y0 >= 0 && b.y1 <= 600, "inside the canvas: {b:?}");
    // Centred on the anchor's x, running down from its y.
    assert!(b.x0 < 900 && b.x1 > 900 && b.y0 >= 25, "{b:?}");
    // Latin text: a second column of a two-line layer sits left of the first.
    let r = s.execute("type.create", json!({"x": 500, "y": 30, "text": "ABC\nDEF", "size": 40})).unwrap();
    let two = LayerId(r["layer"].as_u64().unwrap());
    s.execute("type.orientation.vertical", json!({"layer": two.0})).unwrap();
    let st = s.active().unwrap();
    let l = layout(&st.doc, text(&s, two));
    assert!(l.vertical && l.lines.len() == 2);
    let [x0, y0, x1, y1] = l.bounds().unwrap();
    assert!(x1 > 0.0 && x0 < -40.0 && y0 >= -1e-3 && y1 > 60.0, "{:?}", l.bounds());
    // Bad layer ids fail cleanly. Not a small number like 9999: ids come from one process-wide
    // counter, so in a full test run a layer made above can get it (#3343).
    assert!(s.execute("type.orientation.vertical", json!({"layer": u64::MAX})).is_err());
}

#[test]
fn vertical_point_and_paragraph_conversion_keep_the_columns_in_place() {
    for align in ["left", "center", "right"] {
        let mut s = session(8);
        s.execute("image.canvasSize", json!({"width": 300, "height": 400})).ok();
        let r = s.execute("type.create", json!({"x": 150, "y": 100, "text": "Hello world", "size": 30, "align": align})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        s.execute("type.orientation.vertical", json!({})).unwrap();
        let st = s.active().unwrap();
        let before = layout(&st.doc, text(&s, id)).bounds().unwrap();
        let before_tf = text(&s, id).transform;
        let map = |t: &photocraft_geom::Affine, b: [f32; 4]| {
            let p = t.apply(photocraft_geom::Point::new(f64::from(b[0]), f64::from(b[1])));
            (p.x, p.y)
        };
        let doc_before = map(&before_tf, before);
        s.execute("type.convertToParagraphText", json!({})).unwrap();
        let TextShape::Box { height, .. } = text(&s, id).shape else { panic!("box expected") };
        let st = s.active().unwrap();
        let l = layout(&st.doc, text(&s, id));
        assert_eq!(l.lines.len(), 1, "{align}: no new wraps");
        assert!(height > 0.0);
        // The column's first glyph stays where it was (glyph positions in document space).
        let g_after = l.glyphs[0];
        let tf = text(&s, id).transform;
        let p_after = tf.apply(photocraft_geom::Point::new(f64::from(g_after.x), f64::from(g_after.y)));
        s.execute("type.convertToPointText", json!({})).unwrap();
        let st = s.active().unwrap();
        let back = layout(&st.doc, text(&s, id));
        let g_back = back.glyphs[0];
        let p_back = text(&s, id).transform.apply(photocraft_geom::Point::new(f64::from(g_back.x), f64::from(g_back.y)));
        assert!((p_after.x - p_back.x).abs() < 1.5 && (p_after.y - p_back.y).abs() < 1.5, "{align}: {p_after:?} → {p_back:?}");
        let back_doc = map(&text(&s, id).transform, back.bounds().unwrap());
        assert!((back_doc.0 - doc_before.0).abs() < 1.5 && (back_doc.1 - doc_before.1).abs() < 1.5, "{align}: {doc_before:?} → {back_doc:?}");
    }
}

#[test]
fn paragraph_to_point_hardens_soft_wraps() {
    let mut s = session(8);
    let r = s.execute("type.create", json!({"box": [10, 10, 120, 100], "text": "one two three four five six", "size": 18})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    let st = s.active().unwrap();
    let soft = layout(&st.doc, text(&s, id)).lines.len();
    assert!(soft >= 2);
    let r = s.execute("type.convertToPointText", json!({})).unwrap();
    assert_eq!(r["inserted"], soft - 1);
    let t = text(&s, id);
    assert_eq!(t.text.matches('\n').count(), soft - 1);
    assert_eq!(t.text.replace('\n', ""), "one two three four five six");
    let st = s.active().unwrap();
    assert_eq!(layout(&st.doc, text(&s, id)).lines.len(), soft);
}

#[test]
fn work_path_and_shape_follow_the_glyphs() {
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        let id = text_layer(&mut s, "oil");
        let before = ink(&s, id);
        s.execute("type.createWorkPath", json!({})).unwrap();
        let wp = s.active().unwrap().doc.work_path.clone().unwrap();
        // 'o' has an outer contour and a counter (subtracted), plus 'i' (stem + dot) and 'l'.
        assert!(wp.subpaths.len() >= 5, "{}", wp.subpaths.len());
        assert!(wp.subpaths.iter().any(|sp| sp.op == PathOp::Subtract));
        let (x0, y0, x1, y1) = wp.control_bounds().unwrap();
        assert!((x0 - before.x0 as f64).abs() < 3.0 && (x1 - before.x1 as f64).abs() < 3.0, "{x0},{x1} vs {before:?}");
        assert!((y0 - before.y0 as f64).abs() < 3.0 && (y1 - before.y1 as f64).abs() < 3.0);
        // Convert to Shape renders the same pixels (the counter of the 'o' stays open).
        let comp_before = photocraft_compose::flatten(&s.active().unwrap().doc);
        s.execute("type.convertToShape", json!({})).unwrap();
        let l = s.active().unwrap().doc.layer(id).unwrap();
        assert!(matches!(l.content, LayerContent::Shape(_)));
        assert!(!l.psd_blocks.iter().any(|(k, _)| k == b"TySh"));
        let comp_after = photocraft_compose::flatten(&s.active().unwrap().doc);
        let diff: f32 = comp_before.px.iter().zip(&comp_after.px).map(|(a, b)| (a[0] - b[0]).abs()).sum::<f32>() / comp_before.px.len() as f32;
        assert!(diff < 0.01, "depth {depth}: mean diff {diff}");
        // The middle of the 'o' counter is background (white).
        assert!(s.undo());
        assert!(matches!(s.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Text(_)));
    }
}

#[test]
fn warp_text_changes_pixels_and_round_trips() {
    let mut s = session(8);
    let id = text_layer(&mut s, "WARP ME");
    let flat = ink(&s, id);
    s.execute("type.warpText", json!({"style": "arc", "bend": 60})).unwrap();
    let w = text(&s, id).warp.clone().unwrap();
    assert_eq!(w.style, "warpArc");
    assert_eq!(w.value, 60.0);
    let arced = ink(&s, id);
    assert!(arced.height() > flat.height() + 8, "{flat:?} → {arced:?}");
    // The regenerated TySh carries the warp (PSD round trip).
    let raw = text(&s, id).psd_raw.clone().unwrap();
    let back = photocraft_text::psd::text_layer_from_tysh(&raw, 72.0).unwrap();
    assert_eq!(back.warp, Some(w));
    assert!(s.execute("type.warpText", json!({"style": "spiral"})).is_err());
    s.execute("type.warpText", json!({"style": "none"})).unwrap();
    assert!(text(&s, id).warp.is_none());
    assert_eq!(ink(&s, id), flat);
    for (_, style) in photocraft_text::warp::STYLES {
        s.execute("type.warpText", json!({"style": style, "bend": -40, "horizontalDistortion": 10})).unwrap();
        assert!(!ink(&s, id).is_empty(), "{style}");
    }
}

#[test]
fn opentype_toggles_store_features() {
    let mut s = session(8);
    let id = text_layer(&mut s, "office 1/2");
    let st = |s: &Session| text(s, id).char_runs()[0].style.clone();
    assert!(st(&s).ligatures);
    s.execute("type.openType.standardLigatures", json!({})).unwrap();
    assert!(!st(&s).ligatures);
    s.execute("type.openType.fractions", json!({})).unwrap();
    s.execute("type.openType.discretionaryLigatures", json!({"on": true})).unwrap();
    s.execute("type.openType.swash", json!({"range": [0, 3]})).unwrap();
    assert!(feature_on(&st(&s), "frac") && feature_on(&st(&s), "dlig") && feature_on(&st(&s), "swsh"));
    let runs = text(&s, id).char_runs();
    assert!(!feature_on(&runs.last().unwrap().style, "swsh"), "range respected");
    s.execute("type.openType.fractions", json!({})).unwrap();
    assert!(!feature_on(&st(&s), "frac"));
    // PSD round trip of the toggles.
    let raw = text(&s, id).psd_raw.clone().unwrap();
    let back = photocraft_text::psd::text_layer_from_tysh(&raw, 72.0).unwrap();
    assert!(feature_on(&back.char_runs()[0].style, "swsh") && !back.char_runs()[0].style.ligatures);
}

/// Is `tag` on at character `ci` of an ASCII text layer?
fn on_at(s: &Session, id: LayerId, ci: usize, tag: &str) -> bool {
    let mut end = 0;
    text(s, id)
        .char_runs()
        .iter()
        .find(|r| {
            end += r.len;
            ci < end
        })
        .is_some_and(|r| feature_on(&r.style, tag))
}

#[test]
fn numerals_are_registered_with_their_menu_path() {
    for style in NumeralsStyle::ALL {
        let spec = crate::commands::find(&format!("type.openType.numerals.{}", style.id())).unwrap();
        assert_eq!(spec.label, style.label());
        assert_eq!(spec.menu, ["Type", "OpenType", "Numerals"]);
    }
    assert!(crate::commands::find("type.openType.numerals.unknown").is_none());
}

#[test]
fn numerals_replace_figure_tags_and_keep_other_features() {
    let mut s = session(8);
    let id = text_layer(&mut s, "012345");
    let before = text(&s, id).char_runs()[0].style.clone();
    s.execute("type.openType.fractions", json!({"on": true})).unwrap();
    s.execute("type.openType.numerals.tabularLining", json!({})).unwrap();
    assert_eq!(numerals_style_of(&text(&s, id).char_runs()[0].style), Some(NumeralsStyle::TabularLining));
    assert!(on_at(&s, id, 0, "tnum") && on_at(&s, id, 0, "lnum") && !on_at(&s, id, 0, "onum") && !on_at(&s, id, 0, "pnum"));
    assert!(feature_on(&text(&s, id).char_runs()[0].style, "frac"), "non-figure features stay");
    s.execute("type.openType.numerals.proportionalOldstyle", json!({})).unwrap();
    assert_eq!(numerals_style_of(&text(&s, id).char_runs()[0].style), Some(NumeralsStyle::ProportionalOldstyle));
    assert!(on_at(&s, id, 0, "pnum") && on_at(&s, id, 0, "onum") && !on_at(&s, id, 0, "tnum") && !on_at(&s, id, 0, "lnum"));
    s.execute("type.openType.numerals.default", json!({})).unwrap();
    assert_eq!(numerals_style_of(&text(&s, id).char_runs()[0].style), Some(NumeralsStyle::Default));
    for tag in NUMERAL_FEATURE_TAGS {
        assert!(!on_at(&s, id, 0, tag), "{tag}");
    }
    let st = text(&s, id).char_runs()[0].style.clone();
    assert!(st.ligatures, "liga untouched");
    assert_eq!((st.size_pt, st.font_family.clone(), st.color), (before.size_pt, before.font_family, before.color));
}

#[test]
fn numerals_range_edits_only_those_characters() {
    let mut s = session(8);
    let id = text_layer(&mut s, "abcdef");
    s.execute("type.openType.numerals.lining", json!({"range": [2, 5]})).unwrap();
    let on: Vec<bool> = (0..6).map(|i| on_at(&s, id, i, "lnum")).collect();
    assert_eq!(on, [false, false, true, true, true, false]);
    assert_eq!(numerals_style_at(text(&s, id), 2), Some(NumeralsStyle::Lining));
    assert_eq!(numerals_style_at(text(&s, id), 0), Some(NumeralsStyle::Default));
    s.execute("type.openType.numerals.tabular", json!({"range": [2, 5]})).unwrap();
    assert!(on_at(&s, id, 3, "tnum") && !on_at(&s, id, 3, "lnum"));
    assert!(!on_at(&s, id, 0, "tnum"));
}

#[test]
fn numerals_apply_to_every_listed_layer() {
    let mut s = session(8);
    let a = text_layer(&mut s, "one");
    let b = text_layer(&mut s, "two");
    s.execute("type.openType.numerals.oldstyle", json!({"layers": [a.0, b.0]})).unwrap();
    assert!(on_at(&s, a, 0, "onum") && on_at(&s, b, 2, "onum"));
    s.execute("type.openType.numerals.default", json!({"layers": [a.0, b.0]})).unwrap();
    assert!(!on_at(&s, a, 0, "onum") && !on_at(&s, b, 0, "onum"));
}

#[test]
fn numerals_are_one_undo_step_each() {
    let mut s = session(8);
    let id = text_layer(&mut s, "123");
    s.execute("type.openType.numerals.lining", json!({})).unwrap();
    s.execute("type.openType.numerals.tabularOldstyle", json!({})).unwrap();
    assert!(s.undo());
    assert_eq!(numerals_style_of(&text(&s, id).char_runs()[0].style), Some(NumeralsStyle::Lining));
    assert!(s.undo());
    assert_eq!(numerals_style_of(&text(&s, id).char_runs()[0].style), Some(NumeralsStyle::Default));
    assert!(s.redo());
    assert_eq!(numerals_style_of(&text(&s, id).char_runs()[0].style), Some(NumeralsStyle::Lining));
}

#[test]
fn numerals_survive_a_pcraft_round_trip() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Il36");
    s.execute("type.openType.numerals.proportionalLining", json!({})).unwrap();
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let doc = photocraft_format::load_from_bytes(&bytes).unwrap();
    let Some(LayerContent::Text(t)) = doc.layer(id).map(|l| &l.content) else { panic!("text layer lost") };
    assert_eq!(numerals_style_of(&t.char_runs()[0].style), Some(NumeralsStyle::ProportionalLining));
}

#[test]
fn numerals_fail_gracefully_on_bad_params() {
    let mut s = session(8);
    let bg = s.active().unwrap().active_layer.unwrap();
    let id = text_layer(&mut s, "abc");
    let cmd = "type.openType.numerals.tabularLining";
    assert!(s.execute(cmd, json!({"layer": 987_654})).is_err(), "unknown layer");
    assert!(s.execute(cmd, json!({"layer": bg.0})).is_err(), "not a type layer");
    assert!(s.execute(cmd, json!({"layers": ["x"]})).is_err());
    s.execute(cmd, json!({"range": [5, 2]})).unwrap();
    s.execute(cmd, json!({"range": [50, 90]})).unwrap();
    s.execute(cmd, json!({"range": [u64::MAX, 0]})).unwrap();
    s.execute(cmd, json!({"range": "all"})).unwrap();
    assert!(s.execute(cmd, json!({"layer": id.0, "range": [0, 1]})).is_ok());
    assert_eq!(numerals_style_of(&text(&s, id).char_runs()[0].style), Some(NumeralsStyle::TabularLining));
}

#[test]
fn numerals_tabular_changes_inter_digit_widths() {
    let mut s = session(8);
    let r = s.execute("type.create", json!({"x": 20, "y": 70, "text": "1111111111", "font": "Inter", "size": 36, "color": "#000000"})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    let width = |s: &Session| {
        let t = text(s, id).clone();
        let mut e = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
        let layout = e.layout(&t, 72.0);
        layout.lines.first().map_or(0.0, |line| line.x1 - line.x0)
    };
    let plain = width(&s);
    s.execute("type.openType.numerals.tabular", json!({})).unwrap();
    let tab = width(&s);
    assert!((tab - plain).abs() > 0.5, "tnum changes Inter's digit advances: {plain} → {tab}");
    s.execute("type.openType.numerals.default", json!({})).unwrap();
    assert!((width(&s) - plain).abs() < 0.5, "default restores the advances");
}

const SET_IDS: [&str; 3] = ["type.openType.stylisticSets.set1", "type.openType.stylisticSets.set3", "type.openType.stylisticSets.set20"];

#[test]
fn stylistic_sets_are_registered_with_their_menu_path() {
    for n in 1..=20 {
        let spec = crate::commands::find(&format!("type.openType.stylisticSets.set{n}")).unwrap();
        assert_eq!(spec.label, format!("Set {n}"));
        assert_eq!(spec.menu, ["Type", "OpenType", "Stylistic Sets"]);
    }
    assert!(crate::commands::find("type.openType.stylisticSets.set0").is_none());
    assert!(crate::commands::find("type.openType.stylisticSets.set21").is_none());
}

#[test]
fn stylistic_sets_toggle_independently_and_keep_other_features() {
    let mut s = session(8);
    let id = text_layer(&mut s, "office 1/2");
    let before = text(&s, id).char_runs()[0].style.clone();
    s.execute("type.openType.stylisticAlternates", json!({})).unwrap();
    s.execute(SET_IDS[0], json!({})).unwrap();
    s.execute(SET_IDS[1], json!({})).unwrap();
    s.execute(SET_IDS[2], json!({"on": true})).unwrap();
    for tag in ["ss01", "ss03", "ss20", "salt"] {
        assert!(on_at(&s, id, 0, tag), "{tag}");
    }
    assert!(!on_at(&s, id, 0, "ss02"));
    s.execute(SET_IDS[0], json!({})).unwrap();
    assert!(!on_at(&s, id, 0, "ss01"), "toggled off");
    for tag in ["ss03", "ss20", "salt"] {
        assert!(on_at(&s, id, 0, tag), "{tag} stays");
    }
    let st = text(&s, id).char_runs()[0].style.clone();
    assert!(st.ligatures, "liga untouched");
    assert_eq!((st.size_pt, st.font_family.clone(), st.color), (before.size_pt, before.font_family, before.color));
}

#[test]
fn stylistic_set_range_edits_only_those_characters_and_direction_follows_the_range_start() {
    let mut s = session(8);
    let id = text_layer(&mut s, "abcdef");
    s.execute("type.openType.stylisticSets.set2", json!({"range": [3, 6]})).unwrap();
    let on: Vec<bool> = (0..6).map(|i| on_at(&s, id, i, "ss02")).collect();
    assert_eq!(on, [false, false, false, true, true, true]);
    // The range starts on the set, so the default direction is off.
    let r = s.execute("type.openType.stylisticSets.set2", json!({"range": [3, 6]})).unwrap();
    assert_eq!(r["on"], false);
    assert!((0..6).all(|i| !on_at(&s, id, i, "ss02")));
    s.execute("type.openType.stylisticSets.set2", json!({"on": true, "range": [3, 6]})).unwrap();
    // A range starting before the styled characters turns it on (start is off), for all of them.
    let r = s.execute("type.openType.stylisticSets.set2", json!({"range": [1, 6]})).unwrap();
    assert_eq!(r["on"], true);
    let on: Vec<bool> = (0..6).map(|i| on_at(&s, id, i, "ss02")).collect();
    assert_eq!(on, [false, true, true, true, true, true]);
    // And a range starting on a set character toggles off even though it begins mid-text.
    let r = s.execute("type.openType.stylisticSets.set2", json!({"range": [4, 5]})).unwrap();
    assert_eq!(r["on"], false);
    let on: Vec<bool> = (0..6).map(|i| on_at(&s, id, i, "ss02")).collect();
    assert_eq!(on, [false, true, true, true, false, true]);
}

#[test]
fn stylistic_sets_apply_to_every_listed_layer() {
    let mut s = session(8);
    let a = text_layer(&mut s, "one");
    let b = text_layer(&mut s, "two");
    s.execute("type.openType.stylisticSets.set7", json!({"layers": [a.0, b.0]})).unwrap();
    assert!(on_at(&s, a, 0, "ss07") && on_at(&s, b, 2, "ss07"));
    s.execute("type.openType.stylisticSets.set7", json!({"layers": [a.0, b.0]})).unwrap();
    assert!(!on_at(&s, a, 0, "ss07") && !on_at(&s, b, 0, "ss07"));
}

#[test]
fn stylistic_set_toggles_are_one_undo_step_each() {
    let mut s = session(8);
    let id = text_layer(&mut s, "abc");
    s.execute(SET_IDS[0], json!({})).unwrap();
    s.execute(SET_IDS[1], json!({})).unwrap();
    assert!(s.undo());
    assert!(on_at(&s, id, 0, "ss01") && !on_at(&s, id, 0, "ss03"), "one step undone");
    assert!(s.undo());
    assert!(!on_at(&s, id, 0, "ss01"));
    assert!(s.redo());
    assert!(on_at(&s, id, 0, "ss01") && !on_at(&s, id, 0, "ss03"), "one step redone");
    assert!(s.redo());
    assert!(on_at(&s, id, 0, "ss01") && on_at(&s, id, 0, "ss03"));
}

/// The sets survive a `.pcraft` save. In PSD they live only in the file's `Txt2` block, which a
/// document made here doesn't get yet; the PSD round trip of an opened file is tested in
/// `photocraft-io` (`psd_kerning.rs`, `edited_text_layer_keeps_its_stylistic_sets`).
#[test]
fn stylistic_sets_survive_a_pcraft_round_trip() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Il36");
    s.execute("type.openType.stylisticSets.set1", json!({})).unwrap();
    s.execute("type.openType.stylisticSets.set5", json!({})).unwrap();
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let doc = photocraft_format::load_from_bytes(&bytes).unwrap();
    let Some(LayerContent::Text(t)) = doc.layer(id).map(|l| &l.content) else { panic!("text layer lost") };
    let st = &t.char_runs()[0].style;
    assert!(feature_on(st, "ss01") && feature_on(st, "ss05") && !feature_on(st, "ss02"));
}

#[test]
fn stylistic_sets_fail_gracefully_on_bad_params() {
    let mut s = session(8);
    let bg = s.active().unwrap().active_layer.unwrap();
    let id = text_layer(&mut s, "abc");
    let cmd = "type.openType.stylisticSets.set9";
    assert!(s.execute(cmd, json!({"layer": 987_654})).is_err(), "unknown layer");
    assert!(s.execute(cmd, json!({"layer": bg.0})).is_err(), "not a type layer");
    assert!(s.execute(cmd, json!({"layers": ["x"]})).is_err());
    // Same as the other toggles: an unusable `on` falls back to toggling, a reversed range is
    // read in order, and a range past the end clamps (nothing to style).
    s.execute(cmd, json!({"on": "x"})).unwrap();
    assert!(on_at(&s, id, 0, "ss09"));
    s.execute(cmd, json!({"range": [5, 2]})).unwrap();
    s.execute(cmd, json!({"range": [50, 90]})).unwrap();
    s.execute(cmd, json!({"range": [u64::MAX, 0]})).unwrap();
    s.execute(cmd, json!({"range": "all"})).unwrap();
    assert!(s.execute(cmd, json!({"layer": id.0, "range": [0, 1], "on": null})).is_ok());
}

#[test]
fn stylistic_sets_change_inter_glyphs() {
    let mut s = session(8);
    let r = s.execute("type.create", json!({"x": 20, "y": 70, "text": "Il36", "font": "Inter", "size": 36, "color": "#000000"})).unwrap();
    let id = LayerId(r["layer"].as_u64().unwrap());
    let glyphs = |s: &Session| {
        let t = text(s, id).clone();
        let mut e = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
        e.layout(&t, 72.0).glyphs.iter().map(|g| g.id).collect::<Vec<_>>()
    };
    let plain = glyphs(&s);
    s.execute("type.openType.stylisticSets.set1", json!({"range": [2, 4]})).unwrap();
    let one = glyphs(&s);
    assert_ne!(one, plain, "ss01 redraws the digits");
    s.execute("type.openType.stylisticSets.set2", json!({"range": [0, 2]})).unwrap();
    let two = glyphs(&s);
    assert_ne!(two, one, "ss02 redraws I and l");
    assert_eq!(two.len(), plain.len());
    s.execute("type.openType.stylisticSets.set1", json!({"range": [2, 4]})).unwrap();
    s.execute("type.openType.stylisticSets.set2", json!({"range": [0, 2]})).unwrap();
    assert_eq!(glyphs(&s), plain, "turning both off restores the glyphs");
}

#[test]
fn lorem_update_fonts_and_default_styles() {
    let mut s = session(16);
    // No type layer: a new paragraph box.
    let r = s.execute("type.pasteLoremIpsum", json!({})).unwrap();
    let box_id = LayerId(r["layer"].as_u64().unwrap());
    assert!(matches!(text(&s, box_id).shape, TextShape::Box { .. }));
    let id = text_layer(&mut s, "Hi ");
    let r = s.execute("type.pasteLoremIpsum", json!({})).unwrap();
    assert!(text(&s, id).text.starts_with("Hi Lorem ipsum"));
    assert_eq!(r["caret"], text(&s, id).text.chars().count());
    assert_eq!(s.execute("type.updateAllTextLayers", json!({})).unwrap()["updated"], 2);
    // A missing font is reported and replaced.
    s.execute("type.setStyle", json!({"font": "No Such Font Family"})).unwrap();
    assert_eq!(s.execute("type.resolveMissingFonts", json!({})).unwrap()["missing"], json!(["No Such Font Family"]));
    s.execute("type.resolveMissingFonts", json!({"map": {"No Such Font Family": "JetBrains Mono"}})).unwrap();
    assert_eq!(text(&s, id).char_runs()[0].style.font_family, "JetBrains Mono");
    s.execute("type.setStyle", json!({"font": "Gone Sans"})).unwrap();
    assert_eq!(s.execute("type.replaceAllMissingFonts", json!({})).unwrap()["replaced"], 1);
    assert_eq!(text(&s, id).char_runs()[0].style.font_family, photocraft_text::fonts::DEFAULT_FAMILY);
    // Default type styles: save from one layer, new layers and Load use them.
    assert!(!s.is_enabled("type.loadDefaultTypeStyles"));
    s.execute("type.setStyle", json!({"size": 50, "font": "JetBrains Mono"})).unwrap();
    s.execute("type.saveDefaultTypeStyles", json!({})).unwrap();
    s.select_layer(box_id).unwrap();
    s.execute("type.loadDefaultTypeStyles", json!({})).unwrap();
    assert_eq!(text(&s, box_id).char_runs()[0].style.size_pt, 50.0);
    let r = s.execute("type.create", json!({"x": 5, "y": 5, "text": "new"})).unwrap();
    let st = text(&s, LayerId(r["layer"].as_u64().unwrap())).char_runs()[0].style.clone();
    assert_eq!((st.font_family.as_str(), st.size_pt), ("JetBrains Mono", 50.0));
}

#[test]
fn rasterize_type_layer() {
    let mut s = session(8);
    let id = text_layer(&mut s, "R");
    s.execute("type.rasterizeTypeLayer", json!({})).unwrap();
    assert!(matches!(s.active().unwrap().doc.layer(id).unwrap().content, LayerContent::Raster(_)));
    assert!(!s.is_enabled("type.rasterizeTypeLayer"));
}

#[test]
fn type_attaches_to_a_path_and_releases_it() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Hello path");
    assert!(s.is_enabled("type.attachToPath"));
    assert!(!s.is_enabled("type.detachFromPath"));
    // No work path yet, a bad subpath and bad params are errors, not panics.
    assert!(s.execute("type.attachToPath", json!({})).is_err());
    let flat = ink(&s, id);
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(150.0, 10.0), (150.0, 110.0)])]));
        Ok(())
    })
    .unwrap();
    assert!(s.execute("type.attachToPath", json!({"subpath": 5})).is_err());
    assert!(s.execute("type.attachToPath", json!({"start": "far"})).is_err());
    assert!(s.execute("type.attachToPath", json!({"shape": u64::MAX})).is_err());
    s.execute("type.attachToPath", json!({"start": 4})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start, .. } if start == 4.0));
    let on_path = ink(&s, id);
    assert!(on_path.height() > on_path.width(), "the type runs down the vertical path: {on_path:?}");
    assert!(on_path.height() > flat.height());
    assert!(!s.is_enabled("type.convertToPointText"));
    s.execute("type.detachFromPath", json!({})).unwrap();
    assert_eq!(text(&s, id).shape, TextShape::Point);
    assert!(ink(&s, id).width() > ink(&s, id).height());
    assert!(s.undo());
    assert!(matches!(text(&s, id).shape, TextShape::Path { .. }));
}

/// #9114: type on a path follows its own path when the path is edited (Direct Selection's
/// `path.*` commands with `"name": "layer"`), as Photoshop's Type Path does; the Paths list names
/// it, an edit is one undo step, and an edit that leaves no path is refused.
#[test]
fn type_follows_its_path_when_the_path_is_edited() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Hello path");
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(150.0, 10.0), (150.0, 110.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.attachToPath", json!({"start": 4})).unwrap();
    // In document space, the type path is where the work path was.
    let path = crate::type_extra_cmds::type_path(text(&s, id)).unwrap();
    let ends: Vec<(f64, f64)> = path.subpaths[0].knots.iter().map(|k| (k.anchor.x, k.anchor.y)).collect();
    assert!(ends.iter().zip([(150.0, 10.0), (150.0, 110.0)]).all(|(a, b)| (a.0 - b.0).abs() < 1e-6 && (a.1 - b.1).abs() < 1e-6), "{ends:?}");
    let listed = s.execute("path.list", json!({})).unwrap();
    assert_eq!(listed["typePath"]["kind"], "typePath", "{listed}");
    // Swing the path's end to the right: the type now runs down and to the right.
    let before = ink(&s, id);
    let r = s.execute("path.moveAnchors", json!({"name": "layer", "layer": id.0, "anchors": [[0, 1]], "move": [120, 0]})).unwrap();
    assert_eq!(r["name"], "layer");
    let after = ink(&s, id);
    assert!(after.width() > before.width() + 20, "{before:?} → {after:?}");
    assert!(matches!(text(&s, id).shape, TextShape::Path { start, .. } if start == 4.0), "where the type starts is kept");
    // The work path it came from is untouched; Undo puts the type path back.
    assert_eq!(s.active().unwrap().doc.work_path.as_ref().unwrap().subpaths[0].knots[1].anchor.x, 150.0);
    assert!(s.undo());
    assert_eq!(ink(&s, id), before);
    // An edit that would leave fewer than two points is refused and changes nothing.
    assert!(s.execute("path.deleteAnchor", json!({"name": "layer", "layer": id.0, "subpath": 0, "knot": 0})).is_err());
    assert_eq!(ink(&s, id), before);
}

/// #9114: a type-on-a-path layer's vector mask is still what `"name": "layer"` edits, as on main
/// and as `path.info` reads it; `"typePath": true` edits the Type Path; `path.list` reports both.
#[test]
fn a_vector_mask_on_type_on_a_path_is_edited_not_the_type_path() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Hello path");
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(150.0, 10.0), (150.0, 110.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.attachToPath", json!({})).unwrap();
    let rect = json!({"subpaths": [{"closed": true, "knots": [[10, 10], [200, 10], [200, 100], [10, 100]]}]});
    s.execute("layer.vectorMask.add", json!({"layer": id.0, "path": rect})).unwrap();
    let listed = s.execute("path.list", json!({})).unwrap();
    assert_eq!((listed["layerPath"]["kind"].as_str(), listed["typePath"]["kind"].as_str()), (Some("vectorMask"), Some("typePath")), "{listed}");
    let type_path = |s: &Session| crate::type_extra_cmds::type_path(text(s, id)).unwrap();
    let before = type_path(&s);
    let mask = |s: &Session| s.active().unwrap().doc.layer(id).unwrap().vector_mask.as_ref().unwrap().path.subpaths[0].knots[0].anchor;
    s.execute("path.moveAnchors", json!({"name": "layer", "layer": id.0, "anchors": [[0, 0]], "move": [10, 0]})).unwrap();
    assert_eq!(mask(&s), photocraft_geom::Point::new(20.0, 10.0), "the mask's corner moved");
    assert_eq!(type_path(&s), before, "the Type Path stays");
    s.execute("path.moveAnchors", json!({"name": "layer", "layer": id.0, "typePath": true, "anchors": [[0, 1]], "move": [20, 0]})).unwrap();
    assert!((type_path(&s).subpaths[0].knots[1].anchor.x - before.subpaths[0].knots[1].anchor.x - 20.0).abs() < 1e-6, "the Type Path moved");
    assert_eq!(mask(&s), photocraft_geom::Point::new(20.0, 10.0), "the mask stays");
}

/// An edit can't grow the Type Path past the cap `type.attachToPath` keeps.
#[test]
fn an_edited_type_path_keeps_the_knot_cap() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Hi");
    let knots: Vec<(f64, f64)> = (0..=MAX_TYPE_PATH_KNOTS).map(|i| (i as f64 * 0.001, 50.0)).collect();
    let mut t = text(&s, id).clone();
    t.shape = TextShape::Path { path: Subpath::polyline(&knots[..2]), start: 0.0, end: None, flip: false };
    assert!(set_type_path(&mut t, &Path::new(vec![Subpath::polyline(&knots[..MAX_TYPE_PATH_KNOTS])])).is_ok());
    assert!(set_type_path(&mut t, &Path::new(vec![Subpath::polyline(&knots)])).is_err());
}

/// The Type tool's click on a path: `"at"` starts the type where the path passes nearest the
/// click, and the paragraph alignment is kept about that point, as in Photoshop.
#[test]
fn type_attaches_to_a_path_at_a_document_point() {
    let mut s = session(8);
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(10.0, 60.0), (290.0, 60.0)])]));
        Ok(())
    })
    .unwrap();
    for (align, centred) in [("left", false), ("center", true)] {
        let r = s.execute("type.create", json!({"x": 150, "y": 60, "text": "Path", "size": 24, "align": align, "color": "#000000"})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        assert!(s.execute("type.attachToPath", json!({"at": [150, "y"]})).is_err());
        assert!(s.execute("type.attachToPath", json!({"at": [150]})).is_err());
        assert!(s.execute("type.attachToPath", json!({"at": [150, 60], "start": 3})).is_err());
        let r = s.execute("type.attachToPath", json!({"at": [150, 64]})).unwrap();
        assert!((r["start"].as_f64().unwrap() - 140.0).abs() < 0.5, "{r}");
        let b = ink(&s, id);
        assert!(b.y1 <= 61, "the type sits on the path: {b:?}");
        if centred {
            assert!(b.x0 < 145 && b.x1 > 155, "centred on the click: {b:?}");
        } else {
            assert!((148..=156).contains(&b.x0), "starts at the click: {b:?}");
        }
    }
}

#[test]
fn all_locked_layer_rejects_paste_lorem_ipsum_without_history() {
    // #4188: Lock All stopped type.edit (#3132) but not Paste Lorem Ipsum into the layer.
    let mut s = session(8);
    let id = text_layer(&mut s, "Hi");
    s.execute("layer.setProps", json!({"layer": id.0, "locked": true})).unwrap();
    let history = s.active().unwrap().history.past_len();
    for params in [json!({}), json!({"layer": id.0})] {
        let result = s.execute("type.pasteLoremIpsum", params);
        assert!(matches!(&result, Err(EngineError::Other(message)) if message.contains("locked")), "{result:?}");
        assert_eq!(text(&s, id).text, "Hi");
        assert_eq!(s.active().unwrap().history.past_len(), history);
    }
}

#[test]
fn paste_lorem_ipsum_inserts_into_the_layer_param_not_the_active_layer() {
    // #5659: with a pixel layer active, an explicit type "layer" got a new text box instead.
    let mut s = session(8);
    let id = text_layer(&mut s, "Hi");
    s.execute("layer.new.layer", json!({})).unwrap();
    let pixel = s.active().unwrap().active_layer.unwrap();
    assert_ne!(pixel, id);
    let count = s.active().unwrap().doc.walk().len();
    let r = s.execute("type.pasteLoremIpsum", json!({"layer": id.0, "at": 0})).unwrap();
    assert!(text(&s, id).text.starts_with(LOREM_IPSUM), "{}", text(&s, id).text);
    assert!(text(&s, id).text.ends_with("Hi"));
    assert_eq!(r["caret"], LOREM_IPSUM.chars().count());
    assert_eq!(s.active().unwrap().doc.walk().len(), count);
    // An explicit layer that is not a type layer is an error, not a new layer.
    let history = s.active().unwrap().history.past_len();
    assert!(s.execute("type.pasteLoremIpsum", json!({"layer": pixel.0})).is_err());
    assert_eq!(s.active().unwrap().doc.walk().len(), count);
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

/// A type layer stored with a family label that is not installed, as a PSD keeps the family it
/// guessed from a PostScript name whose face was absent when the file was opened.
fn guessed_family_layer(s: &mut Session, text: &str, ps: Option<&str>) -> (LayerId, Value) {
    let mut p = json!({"x": 20, "y": 70, "text": text, "font": "Imported Wrong Family"});
    if let Some(ps) = ps {
        p["postscriptName"] = json!(ps);
    }
    let r = s.execute("type.create", p).unwrap();
    (LayerId(r["layer"].as_u64().unwrap()), r)
}

#[test]
fn installed_postscript_face_is_not_missing_or_replaced() {
    // Opening a PSD corrects the label when the face is installed; a run set by command, or one
    // reopened from .pcraft after its face was installed, keeps it.
    let mut s = session(8);
    let (id, created) = guessed_family_layer(&mut s, "test", Some("Inter-SemiBold"));
    let bytes = photocraft_format::save_to_bytes(&s.active().unwrap().doc, &Default::default()).unwrap();
    let reopened = photocraft_io::import("reopened.pcraft", &bytes).unwrap().document;
    let LayerContent::Text(t) = &reopened.layer(id).unwrap().content else { panic!("text") };
    assert_eq!(t.char_runs()[0].style.font_family, "Imported Wrong Family", ".pcraft open keeps the stored label");
    let on_open = missing_font_warnings(&reopened);
    s.add_document(reopened, None);
    let replaced = s.execute("type.replaceAllMissingFonts", json!({})).unwrap()["replaced"].clone();
    assert_eq!((created.get("warnings"), on_open, replaced), (None, Vec::<String>::new(), json!(0)));
}

#[test]
fn replacing_missing_family_preserves_a_resolved_run_with_the_same_label() {
    let mut s = session(8);
    let (good, _) = guessed_family_layer(&mut s, "good", Some("Inter-SemiBold"));
    let (bad, _) = guessed_family_layer(&mut s, "bad", None);
    assert_eq!(s.execute("type.replaceAllMissingFonts", json!({})).unwrap()["replaced"], 1);
    assert_eq!(text(&s, good).char_runs()[0].style.postscript_name.as_deref(), Some("Inter-SemiBold"));
    assert_eq!(text(&s, bad).char_runs()[0].style.font_family, "Inter");
}

/// `source` renamed in memory to `family` with the PostScript name `ps`; no font file is added.
fn renamed_face(source: &[u8], family: &str, ps: &str) -> Vec<u8> {
    let mut bytes = source.to_vec();
    let count = u16::from_be_bytes([bytes[4], bytes[5]]) as usize;
    let record = (0..count).map(|i| 12 + i * 16).find(|r| &bytes[*r..*r + 4] == b"name").unwrap();
    let names = [(1u16, family), (2, "Regular"), (6, ps), (16, family), (17, "Regular")];
    let mut table: Vec<u8> = [0u16, names.len() as u16, (6 + names.len() * 12) as u16].iter().flat_map(|v| v.to_be_bytes()).collect();
    let mut strings = Vec::new();
    for (id, value) in names {
        let encoded: Vec<u8> = value.encode_utf16().flat_map(u16::to_be_bytes).collect();
        for n in [3, 1, 0x409, id, encoded.len() as u16, strings.len() as u16] {
            table.extend(n.to_be_bytes());
        }
        strings.extend(encoded);
    }
    table.extend(strings);
    let offset = bytes.len() as u32;
    bytes[record + 8..record + 12].copy_from_slice(&offset.to_be_bytes());
    bytes[record + 12..record + 16].copy_from_slice(&(table.len() as u32).to_be_bytes());
    bytes.extend(table);
    bytes
}

#[test]
fn exact_postscript_face_outside_the_guessed_family_is_not_missing() {
    let mut fonts = photocraft_text::FontDb::new();
    fonts.register_font_data(renamed_face(&photocraft_text::fonts::INTER_REGULAR, "Exact Only Test", "UnrelatedPS30"));
    let style =
        |ps: &str| photocraft_doc::text::CharStyle { font_family: "Imported Wrong Family".into(), postscript_name: Some(ps.into()), ..Default::default() };
    // Neither the stored family nor the family guessed from the name is installed: only the exact face.
    assert!(!fonts.has_family(&photocraft_text::fonts::guess_from_postscript("UnrelatedPS30").family));
    assert_eq!((style_font_missing(&mut fonts, &style("UnrelatedPS30")), style_font_missing(&mut fonts, &style("UnrelatedPS31"))), (false, true));
}

/// #5044: PostScript names whose family has its space elsewhere (`BMDoHyeon`, family
/// "BM DoHyeon") or an underscore style suffix (`Daum_SemiBold`). Guessing splits them as
/// "BM Do Hyeon" and "Daum_Semi Bold", neither installed; the exact installed face resolves them.
#[test]
fn camel_case_and_underscore_postscript_names_resolve_to_their_installed_faces() {
    let mut fonts = photocraft_text::FontDb::new();
    fonts.register_font_data(renamed_face(&photocraft_text::fonts::INTER_REGULAR, "BM DoHyeon", "BMDoHyeon"));
    fonts.register_font_data(renamed_face(&photocraft_text::fonts::INTER_REGULAR, "Daum", "Daum_SemiBold"));
    for (ps, family) in [("BMDoHyeon", "BM DoHyeon"), ("Daum_SemiBold", "Daum")] {
        let guess = photocraft_text::fonts::guess_from_postscript(ps).family;
        assert!(!fonts.has_family(&guess), "{ps}: the guess {guess:?} is not an installed family");
        let r = fonts.resolve_postscript(ps);
        assert_eq!((r.family.as_str(), r.exact), (family, true), "{ps}");
        let style = photocraft_doc::text::CharStyle { font_family: guess.clone(), postscript_name: Some(ps.into()), ..Default::default() };
        assert!(!style_font_missing(&mut fonts, &style), "{ps} stored as {guess:?} is not missing");
    }
}

/// #7332: the shell asks for an opened file's missing fonts on the UI thread. A document without
/// type must answer without the shared text engine, whose first use waits for the installed-font
/// scan (minutes on some machines). The engine is held locked meanwhile, which used to block it.
#[test]
fn a_document_without_type_never_waits_for_the_text_engine() {
    let s = session(8);
    let doc = (*s.active().unwrap().doc).clone();
    assert!(!has_type_layers(&doc));
    let held = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(missing_font_warnings(&doc));
    });
    let got = rx.recv_timeout(std::time::Duration::from_secs(10));
    drop(held);
    assert_eq!(got, Ok(Vec::<String>::new()), "a document without type waited for the text engine");
    let mut s = session(8);
    text_layer(&mut s, "type");
    assert!(has_type_layers(&s.active().unwrap().doc));
}

/// Verifies `type.setPathPlacement` commits valid range updates with undo/redo and rejects inverted, infinite, or out-of-bounds inputs.
#[test]
fn path_placement_updates_and_rejects_invalid_inputs() {
    let mut empty = Session::new();
    assert!(empty.execute("type.setPathPlacement", json!({})).is_err());
    let mut s = session(8);
    let id = text_layer(&mut s, "Path");
    assert!(s.execute("type.setPathPlacement", json!({"start": 1})).is_err());
    s.edit("path", |doc, _| {
        doc.work_path = Some(Path::new(vec![Subpath::polyline(&[(10.0, 60.0), (200.0, 60.0)])]));
        Ok(())
    })
    .unwrap();
    s.execute("type.attachToPath", json!({"start": 4})).unwrap();
    let original = text(&s, id).shape.clone();
    let path = crate::type_extra_cmds::type_path(text(&s, id)).unwrap();
    s.edit("remove work path", |doc, _| {
        doc.work_path = None;
        Ok(())
    })
    .unwrap();
    s.execute("type.setPathPlacement", json!({"layer": id.0, "start": 20, "flip": true})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 20.0, flip: true, .. }));
    assert_eq!(crate::type_extra_cmds::type_path(text(&s, id)).unwrap(), path);
    assert!(s.undo());
    assert_eq!(text(&s, id).shape, original);
    assert!(s.redo());
    s.execute("type.setPathPlacement", json!({"start": 30})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 30.0, flip: true, .. }));
    s.execute("type.setPathPlacement", json!({"flip": false})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 30.0, flip: false, .. }));
    for p in [json!({"start": "bad"}), json!({"start": 1e100}), json!({"flip": 1}), json!({"layer": "bad"})] {
        assert!(s.execute("type.setPathPlacement", p).is_err());
    }
    s.execute("type.setPathPlacement", json!({"end": 50})).unwrap();
    s.execute("type.setPathPlacement", json!({"flip": true})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 30.0, end: Some(50.0), .. }));
    let before = text(&s, id).shape.clone();
    for params in [json!({"end": 29}), json!({"start": 51}), json!({"end": 1e100}), json!({"end": "bad"}), json!({"start": -1}), json!({"end": 191})] {
        assert!(s.execute("type.setPathPlacement", params).is_err());
        assert_eq!(text(&s, id).shape, before);
    }
    let path = crate::type_extra_cmds::type_path(text(&s, id)).unwrap();
    let mut layer = text(&s, id).clone();
    set_type_path(&mut layer, &path).unwrap();
    assert!(matches!(layer.shape, TextShape::Path { end: Some(50.0), .. }));
    s.execute("type.setPathPlacement", json!({"start": 0, "end": 190})).unwrap();
    s.execute("type.setPathPlacement", json!({"end": null})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { end: None, .. }));
    s.execute("layer.setProps", json!({"layer": id.0, "locked": true})).unwrap();
    assert!(s.execute("type.setPathPlacement", json!({"layer": id.0, "flip": true})).is_err());
}

/// Ensures command availability is restricted to path text layers and correctly handles legacy unconstrained endpoints or degenerate subpaths.
#[test]
fn path_placement_availability_and_legacy_ranges() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Path");
    let line = Subpath::polyline(&[(0.0, 0.0), (100.0, 0.0)]);
    with_text(&mut s, &json!({}), "area", |t, _| {
        t.shape = TextShape::Area { path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (100.0, 0.0), (100.0, 100.0), (0.0, 100.0)])]) };
        Ok(())
    })
    .unwrap();
    assert!(!s.is_enabled("type.setPathPlacement"));
    assert!(s.is_enabled("type.detachFromPath"));
    assert!(s.execute("type.setPathPlacement", json!({"start": 0})).is_err());
    s.execute("type.detachFromPath", json!({})).unwrap();
    with_text(&mut s, &json!({}), "path", |t, _| {
        t.shape = TextShape::Path { path: line.clone(), start: -5.0, end: Some(10.0), flip: false };
        Ok(())
    })
    .unwrap();
    assert!(s.is_enabled("type.setPathPlacement"));
    s.execute("type.setPathPlacement", json!({"end": null})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: -5.0, end: None, .. }));
    with_text(&mut s, &json!({}), "degenerate", |t, _| {
        t.shape = TextShape::Path { path: Subpath::polyline(&[(0.0, 0.0), (0.0, 0.0)]), start: 0.0, end: None, flip: false };
        Ok(())
    })
    .unwrap();
    let before = text(&s, id).shape.clone();
    assert!(s.execute("type.setPathPlacement", json!({"end": 0, "flip": true})).is_err());
    assert_eq!(text(&s, id).shape, before);
    with_text(&mut s, &json!({}), "closed", |t, _| {
        let mut path = line.clone();
        path.closed = true;
        t.shape = TextShape::Path { path, start: 0.0, end: None, flip: false };
        Ok(())
    })
    .unwrap();
    s.execute("type.setPathPlacement", json!({"start": 190, "end": 210})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 190.0, end: Some(210.0), .. }));

    // Degenerate closed path is rejected on explicit range.
    with_text(&mut s, &json!({}), "degenerate_closed", |t, _| {
        t.shape = TextShape::Path { path: Subpath::polygon(&[(0.0, 0.0), (0.0, 0.0)]), start: 0.0, end: None, flip: false };
        Ok(())
    })
    .unwrap();
    assert!(s.execute("type.setPathPlacement", json!({"start": 0.0, "end": 10.0})).is_err());

    // Closed negative seam offsets, huge multi-lap ranges, and stationary ranges are stored gracefully.
    with_text(&mut s, &json!({}), "closed_ranges", |t, _| {
        let mut path = line.clone();
        path.closed = true;
        t.shape = TextShape::Path { path, start: 0.0, end: None, flip: false };
        Ok(())
    })
    .unwrap();
    s.execute("type.setPathPlacement", json!({"start": -25.0, "end": 1000.0})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: -25.0, end: Some(1000.0), .. }));
    s.execute("type.setPathPlacement", json!({"start": 40.0, "end": 40.0})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 40.0, end: Some(40.0), .. }));

    // Explicit start on open legacy end:null validated against bounds and canonical normalized.
    with_text(&mut s, &json!({}), "open_start_null_end", |t, _| {
        t.shape = TextShape::Path { path: line.clone(), start: 10.0, end: Some(50.0), flip: false };
        Ok(())
    })
    .unwrap();
    assert!(s.execute("type.setPathPlacement", json!({"start": 120.0, "end": null})).is_err());
    assert!(s.execute("type.setPathPlacement", json!({"start": -5.0, "end": null})).is_err());
    s.execute("type.setPathPlacement", json!({"start": 30.0, "end": null})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 30.0, end: None, .. }));

    // Position-locked and fully locked layers are rejected upfront.
    s.execute("layer.lockLayers", json!({"position": true})).unwrap();
    assert!(s.execute("type.setPathPlacement", json!({"start": 10.0})).is_err());
    s.execute("layer.lockLayers", json!({"position": false, "all": true})).unwrap();
    assert!(s.execute("type.setPathPlacement", json!({"start": 10.0})).is_err());
    s.execute("layer.lockLayers", json!({"all": false})).unwrap();
    assert!(s.execute("type.setPathPlacement", json!({"start": 10.0})).is_ok());
}

/// Proves path placement validation tolerates `f32` precision upward rounding at endpoints while rejecting true out-of-bounds spans.
#[test]
fn path_placement_noninteger_line_rounding_upward_and_zero_capacity() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Path");
    // Line from (0, 0) to (7, 7) has length 7 * sqrt(2) ≈ 9.899494936611665.
    // In f32, length rounds upward to 9.8994951248168945.
    let knots = [(0.0, 0.0), (7.0, 7.0)];
    let path = Subpath::polyline(&knots);
    let geometry = photocraft_text::warp::PathGeometry::new(&path, false).unwrap();
    let length = geometry.length();
    let length_f32 = length as f32;
    assert!(f64::from(length_f32) > length, "scale 7 * sqrt2 must round upward in f32");
    let canonical = geometry.canonical_length_f32();
    assert!(f64::from(canonical) <= length, "canonical f32 must never exceed geometry");

    with_text(&mut s, &json!({}), "attach", |t, _| {
        t.shape = TextShape::Path { path: path.clone(), start: 0.0, end: None, flip: false };
        Ok(())
    })
    .unwrap();

    // Genuine out-of-bounds rejected.
    assert!(s.execute("type.setPathPlacement", json!({"end": 9.95})).is_err());
    assert!(s.execute("type.setPathPlacement", json!({"end": length_f32 + 0.1})).is_err());

    // Upward-rounded endpoint is accepted and normalized to canonical maximum.
    s.execute("type.setPathPlacement", json!({"end": length_f32})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { end: Some(e), .. } if e == canonical));

    // Zero-capacity endpoint range at the path boundary is accepted.
    s.execute("type.setPathPlacement", json!({"start": length_f32, "end": length_f32})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start, end: Some(end), .. } if start == canonical && end == canonical));

    // Flipped case at the path boundary is accepted.
    s.execute("type.setPathPlacement", json!({"start": 0.0, "end": length_f32, "flip": true})).unwrap();
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 0.0, end: Some(end), flip: true, .. } if end == canonical));
}

/// Flipping preserves stale endpoints after shortening a path; endpoint edits still validate.
#[test]
fn path_placement_shortened_path_flip_only_and_endpoint_rejection() {
    let mut s = session(8);
    let id = text_layer(&mut s, "Path");
    let line100 = Subpath::polyline(&[(0.0, 0.0), (100.0, 0.0)]);
    with_text(&mut s, &json!({}), "attach", |t, _| {
        t.shape = TextShape::Path { path: line100, start: 10.0, end: Some(80.0), flip: false };
        Ok(())
    })
    .unwrap();

    // Shorten the path from length 100 to length 50.
    let shortened = Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (50.0, 0.0)])]);
    with_text(&mut s, &json!({}), "shorten", |t, _| set_type_path(t, &shortened)).unwrap();

    // Stored end (80.0) is now past the shortened path length (50.0).
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 10.0, end: Some(80.0), flip: false, .. }));

    // Flip-only call must succeed and preserve the stored range.
    let r = s.execute("type.setPathPlacement", json!({"flip": true})).unwrap();
    assert_eq!(r, json!({"start": 10.0, "end": 80.0, "flip": true}));
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 10.0, end: Some(80.0), flip: true, .. }));

    // Flip back without supplying endpoints also succeeds.
    let r = s.execute("type.setPathPlacement", json!({"flip": false})).unwrap();
    assert_eq!(r, json!({"start": 10.0, "end": 80.0, "flip": false}));
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 10.0, end: Some(80.0), flip: false, .. }));

    // Supplying out-of-bounds start or end on the shortened path must be rejected.
    let before = text(&s, id).shape.clone();
    assert!(s.execute("type.setPathPlacement", json!({"end": 80.0})).is_err());
    assert!(s.execute("type.setPathPlacement", json!({"start": 60.0})).is_err());
    assert!(s.execute("type.setPathPlacement", json!({"start": -5.0})).is_err());
    assert!(s.execute("type.setPathPlacement", json!({"start": 10.0, "flip": true})).is_err());
    assert!(s.execute("type.setPathPlacement", json!({"start": 40.0, "end": 30.0})).is_err());
    assert_eq!(text(&s, id).shape, before);

    // Supplying a valid range on the shortened path succeeds and updates the range.
    let r = s.execute("type.setPathPlacement", json!({"start": 5.0, "end": 45.0, "flip": true})).unwrap();
    assert_eq!(r, json!({"start": 5.0, "end": 45.0, "flip": true}));
    assert!(matches!(text(&s, id).shape, TextShape::Path { start: 5.0, end: Some(45.0), flip: true, .. }));
}
