use super::*;
use photocraft_doc::Layer;
use photocraft_geom::Rect;

/// A 400 × 300 document at 72 ppi with a 200 × 140 pixel layer at (100, 80) named `name`.
fn session(name: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("layer.new.layer", json!({"name": name})).unwrap();
    s.edit("paint", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(100, 80, 300, 220), &[0.16, 0.35, 0.78, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// Photoshop 2025 copies this for a 200 × 140 pixel layer at (100, 80), second from the bottom.
#[test]
fn a_pixel_layer_copies_as_photoshop_writes_it() {
    let mut s = session("Box");
    let r = s.execute(ID, json!({})).unwrap();
    assert_eq!(
        r["css"],
        json!(
            ".Box {\n  background-image: url(\"Box.png\");\n  position: absolute;\n  left: 100px;\n  top: 80px;\n  width: 200px;\n  height: 140px;\n  z-index: 2;\n}\n"
        )
    );
    // Photoshop: a name with spaces keeps them in the image name and turns them to `_` in the
    // class; an opacity under 100 % follows the background.
    let mut s = session("My Box 2");
    s.edit("opacity", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().opacity = 128.0 / 255.0;
        Ok(())
    })
    .unwrap();
    let css = s.execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.starts_with(".My_Box_2 {\n  background-image: url(\"My Box 2.png\");\n  opacity: 0.502;\n  position: absolute;\n"), "{css}");
}

#[test]
fn fill_and_type_layers_describe_their_colour_and_font() {
    let doc = session("Box").active().unwrap().doc.clone();
    // Photoshop: a solid colour gives `background-color`. A fill layer is a shape with a vector
    // mask, placed at its outline; without one it has no rule (Copy CSS is greyed out, measured).
    let mut fill = Layer::new("Card", LayerContent::Fill(Fill::Solid(photocraft_color::Color::rgb(1.0, 120.0 / 255.0, 30.0 / 255.0))));
    assert!(rule(&doc, 4, &fill).is_none());
    fill.vector_mask = Some(photocraft_doc::VectorMask::new(photocraft_vector::shapes::rect(250.0, 200.0, 120.0, 60.0)));
    assert_eq!(
        rule(&doc, 4, &fill).unwrap(),
        ".Card {\n  background-color: rgb(255, 120, 30);\n  position: absolute;\n  left: 250px;\n  top: 200px;\n  width: 120px;\n  height: 60px;\n  z-index: 4;\n}\n"
    );
    // A type layer: size, family, colour, auto leading and alignment, then only its position
    // (Photoshop writes no size for point text). Its position is where its pixels are.
    let mut cache = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    cache.fill_rect(Rect::new(49, 28, 120, 52), &[0.0, 0.0, 0.0, 1.0]);
    let text = photocraft_doc::TextLayer {
        font_family: "Inter".into(),
        size_pt: 24.0,
        color: photocraft_color::Color::rgb(0.0, 0.0, 0.0),
        cache: Some(cache),
        ..Default::default()
    };
    let title = Layer::new("Title", LayerContent::Text(text));
    let css = rule(&doc, 3, &title).unwrap_or_default();
    assert_eq!(
        css,
        ".Title {\n  font-size: 24px;\n  font-family: \"Inter\";\n  color: rgb(0, 0, 0);\n  line-height: 1.2;\n  text-align: left;\n  position: absolute;\n  left: 49px;\n  top: 28px;\n  z-index: 3;\n}\n"
    );
    // A group's own rule is its box, the canvas.
    let group = Layer::group("Group 1", Vec::new());
    assert_eq!(
        rule(&doc, 5, &group).unwrap(),
        ".Group_1 {\n  position: absolute;\n  left: 0px;\n  top: 0px;\n  width: 400px;\n  height: 300px;\n  z-index: 5;\n}\n"
    );
}

/// Measured: Layer › Copy CSS on a group holding a 50 × 30 layer "2nd-Box" at (10, 20) writes the
/// group's canvas box, then the layer, with no blank line; the group's end takes a place in the
/// stack, so the layer is 3 and the group 4 above the Background.
#[test]
fn a_group_writes_its_box_then_its_layers() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    s.execute("layer.new.layer", json!({"name": "2nd-Box"})).unwrap();
    s.edit("paint", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(10, 20, 60, 50), &[0.16, 0.35, 0.78, 1.0]);
        Ok(())
    })
    .unwrap();
    s.execute("layer.groupLayers", json!({})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.edit("paint", |doc, active| {
        doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(300, 200, 320, 220), &[0.16, 0.35, 0.78, 1.0]);
        Ok(())
    })
    .unwrap();
    let group = s.active().unwrap().doc.walk().into_iter().find(|(_, _, l)| l.children().is_some()).unwrap().2.id.0;
    let css = s.execute(ID, json!({"layer": group})).unwrap()["css"].as_str().unwrap().to_string();
    let rest = css.split_once(' ').unwrap().1;
    assert_eq!(
        rest,
        "{\n  position: absolute;\n  left: 0px;\n  top: 0px;\n  width: 400px;\n  height: 300px;\n  z-index: 4;\n}\n\
         ._2nd_Box {\n  background-image: url(\"2nd-Box.png\");\n  position: absolute;\n  left: 10px;\n  top: 20px;\n  width: 50px;\n  height: 30px;\n  z-index: 3;\n}\n"
    );
    let css = s.execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.starts_with(".Top {\n") && css.contains("  z-index: 5;\n"), "{css}");
}

/// Measured: Copy CSS is greyed out for a Solid Color fill layer without a shape.
#[test]
fn copy_css_is_off_for_a_fill_layer_without_a_shape() {
    let mut s = session("Box");
    assert!(s.is_enabled(ID));
    s.execute("layer.newFillLayer.solidColor", json!({})).unwrap();
    assert!(!s.is_enabled(ID), "a plain fill layer has no CSS");
    assert!(s.execute(ID, json!({})).is_err());
}

/// Point type as Photoshop placed it (measured: text bounds and pixel bounds from its scripting
/// interface, then the copied CSS).
#[test]
fn point_type_is_placed_as_photoshop_places_it() {
    let arial_cap = 1467.0 / 2048.0 * 24.0;
    let times_cap = 1356.0 / 2048.0 * 24.0;
    let css = |pix: Rect, ink: [f64; 4], cap: f64, size: f64, line_height: f64| {
        let at = place_point_text(pix, ink, 0.0, cap, size, line_height * size);
        (position(at.x), position(at.y))
    };
    let s = |a: &str, b: &str| (a.to_string(), b.to_string());
    // "Hello", Arial 24 at (50, 150).
    assert_eq!(css(Rect::new(52, 132, 104, 151), [1.875, -17.25, 53.71875, 0.1875], arial_cap, 24.0, 1.2), s("50.125", "127.1"));
    // The same at 48 px.
    assert_eq!(css(Rect::new(53, 65, 158, 101), [3.75, -34.5, 107.4375, 0.375], arial_cap * 2.0, 48.0, 1.2), s("49.25", "54.2"));
    // With a 40 pt leading: line-height 1.667.
    assert_eq!(css(Rect::new(202, 82, 254, 101), [1.875, -17.25, 53.71875, 0.1875], arial_cap, 24.0, 1.667), s("200.125", "71.496"));
    // "x": no descender.
    assert_eq!(css(Rect::new(300, 237, 312, 250), [0.1875, -12.5625, 11.8799, 0.0], arial_cap, 24.0, 1.2), s("299.812", "229.1"));
    // Descenders: "xyz", "xg", "Hg" in Arial, "Hg" and "xg" in Times New Roman.
    assert_eq!(css(Rect::new(50, 187, 86, 205), [0.1875, -12.5625, 35.71875, 4.875], arial_cap, 24.0, 1.2), s("49.812", "176.566"));
    assert_eq!(css(Rect::new(330, 237, 354, 255), [0.1875, -12.75, 23.6953, 4.875], arial_cap, 24.0, 1.2), s("329.812", "226.754"));
    assert_eq!(css(Rect::new(302, 262, 329, 285), [1.875, -17.25, 29.0273, 4.875], arial_cap, 24.0, 1.2), s("300.125", "256.254"));
    assert_eq!(css(Rect::new(50, 84, 79, 106), [0.375, -15.9375, 28.5, 5.0625], times_cap, 24.0, 1.2), s("49.625", "77.592"));
    assert_eq!(css(Rect::new(150, 89, 174, 106), [0.1875, -11.0625, 23.5, 5.0625], times_cap, 24.0, 1.2), s("149.812", "77.717"));
}

#[test]
fn positions_are_cut_after_three_decimals() {
    assert_eq!(position(49.8125), "49.812");
    assert_eq!(position(141.5 - 14.4), "127.1");
    assert_eq!(position(100.0), "100");
    assert_eq!(position(f64::INFINITY), "0");
}

/// Photoshop 2025, paragraph text in a 200 × 120 box at (60, 40) that wraps to two lines whose
/// pixels span 156 × 51: `left: 59.937px; top: 36.948px; width: 156px; height: 51px;`. Unlike point
/// text it has a size, that of its text (not the box), and it's placed at its box, not where its
/// first pixel is; an explicit leading moves it down by that much (36 px: `top: 72.948px`).
#[test]
fn paragraph_text_is_placed_at_its_box_and_sized_by_its_text() {
    let doc = session("Box").active().unwrap().doc.clone();
    let mut cache = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    cache.fill_rect(Rect::new(62, 44, 218, 95), &[0.0, 0.0, 0.0, 1.0]);
    let mut text = photocraft_doc::TextLayer {
        text: "Hello world this wraps around".into(),
        font_family: "Inter".into(),
        size_pt: 24.0,
        cache: Some(cache),
        shape: photocraft_doc::text::TextShape::Box { x: 0.0, y: 0.0, width: 200.0, height: 120.0 },
        transform: photocraft_geom::Affine::translate(60.0, 40.0),
        ..Default::default()
    };
    let css = rule(&doc, 3, &Layer::new("Para", LayerContent::Text(text.clone()))).unwrap_or_default();
    assert_eq!(
        css,
        ".Para {\n  font-size: 24px;\n  font-family: \"Inter\";\n  color: rgb(0, 0, 0);\n  line-height: 1.2;\n  text-align: left;\n  position: absolute;\n  left: 60px;\n  top: 40px;\n  width: 156px;\n  height: 51px;\n  z-index: 3;\n}\n"
    );
    let style = photocraft_doc::text::CharStyle { font_family: "Inter".into(), size_pt: 24.0, leading_pt: Some(36.0), ..Default::default() };
    text.runs = vec![photocraft_doc::text::TextRun { len: text.text.len(), style }];
    let css = rule(&doc, 3, &Layer::new("Para", LayerContent::Text(text))).unwrap_or_default();
    assert!(
        css.contains("  line-height: 1.5;\n  text-align: left;\n  position: absolute;\n  left: 60px;\n  top: 76px;\n  width: 156px;\n  height: 51px;\n"),
        "{css}"
    );
}

#[test]
fn numbers_and_names_are_written_as_photoshop_does() {
    assert_eq!(number(48.9374), "48.937");
    assert_eq!(number(27.6), "27.6");
    assert_eq!(number(100.0), "100");
    assert_eq!(number(-0.0001), "0");
    assert_eq!(number(f64::NAN), "0");
    assert_eq!(class_name("My Box 2"), "My_Box_2");
    assert_eq!(class_name(""), "layer");
}

/// Photoshop 2025: a class can't start with a digit, so a layer named "2nd Box" gets a `_` in
/// front; a hyphen becomes `_` like the other punctuation ("-2x" is `._2x`, "my-box" `.my_box`).
#[test]
fn class_names_never_start_with_a_digit() {
    for (name, class) in [("2nd Box", "._2nd_Box {\n"), ("-2x", "._2x {\n"), ("my-box", ".my_box {\n")] {
        let css = session(name).execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
        assert!(css.starts_with(class), "{name}: {css}");
    }
}

/// Names go into quoted CSS strings: a `"`, a backslash or a line break in one is escaped, so the
/// string doesn't end early and keeps the name (Photoshop writes them unescaped, which breaks
/// the rule). Ordinary names are written as before.
#[test]
fn names_in_quoted_values_are_escaped() {
    let css = session("Say \"hi\"").execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains("  background-image: url(\"Say \\\"hi\\\".png\");\n"), "{css}");
    let css = session("a\\b\nc").execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains("  background-image: url(\"a\\\\b\\a c.png\");\n"), "{css}");
    let doc = session("Box").active().unwrap().doc.clone();
    let mut cache = photocraft_raster::Surface::new(photocraft_color::PixelFormat::RGBA8);
    cache.fill_rect(Rect::new(49, 28, 120, 52), &[0.0, 0.0, 0.0, 1.0]);
    let text = photocraft_doc::TextLayer { font_family: "My \"Font\"".into(), size_pt: 24.0, cache: Some(cache), ..Default::default() };
    let css = rule(&doc, 3, &Layer::new("Title", LayerContent::Text(text))).unwrap_or_default();
    assert!(css.contains("  font-family: \"My \\\"Font\\\"\";\n"), "{css}");
}

#[test]
fn bad_params_and_layers_without_css_are_errors() {
    let mut s = session("Box");
    assert!(s.execute(ID, json!({"layer": "x"})).is_err());
    assert!(s.execute(ID, json!({"layer": 999_999})).is_err());
    assert!(s.execute(ID, json!([])).is_err());
    s.execute("layer.new.group", json!({})).unwrap();
    assert!(s.execute(ID, json!({})).unwrap()["css"].as_str().unwrap().contains("  width: 400px;\n"), "an empty group writes its box");
    let mut empty = Session::new();
    assert!(empty.execute(ID, json!({})).is_err(), "no document");
}

/// A live rounded rectangle gives its corners as Photoshop writes them, before the fill: one
/// value when all four are equal, else top-left, top-right, bottom-right, bottom-left; a sharp
/// one writes none.
#[test]
fn a_rounded_rectangle_writes_its_border_radius() {
    let mut s = session("Box");
    let id = s.execute("shape.create", json!({"kind": "roundedRect", "rect": [40, 30, 120, 80], "radius": 12, "fill": "#ff0000"})).unwrap()["layer"]
        .as_u64()
        .unwrap();
    let css = s.execute(ID, json!({"layer": id})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains(" {\n  border-radius: 12px;\n  background-color: rgb(255, 0, 0);\n  position: absolute;\n"), "{css}");
    s.execute("shape.edit", json!({"layer": id, "radii": [20, 0, 10.5, 5]})).unwrap();
    let css = s.execute(ID, json!({"layer": id})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains("  border-radius: 20px 0px 10.5px 5px;\n  background-color: rgb(255, 0, 0);\n"), "{css}");
    s.execute("shape.edit", json!({"layer": id, "radii": [0, 0, 0, 0]})).unwrap();
    let css = s.execute(ID, json!({"layer": id})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(!css.contains("border-radius") && css.contains("  background-color: rgb(255, 0, 0);\n"), "{css}");
    // Scaled 2× on both axes, its corners are twice as round.
    s.execute("shape.edit", json!({"layer": id, "radii": [12, 12, 12, 12]})).unwrap();
    s.execute("shape.edit", json!({"layer": id, "transform": [2, 0, 0, 2, 0, 0]})).unwrap();
    let css = s.execute(ID, json!({"layer": id})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains("  border-radius: 24px;\n  background-color: rgb(255, 0, 0);\n"), "{css}");
}

/// Scaled 200 % across and 100 % down, a rounded rectangle's corners are 24 × 12 px ellipses, but
/// it keeps one radius per corner (12): no `border-radius` rather than a circular one. A flipped
/// one, whose outline runs the other way, keeps its circular corners and writes them.
#[test]
fn an_unequally_scaled_rounded_rectangle_writes_no_border_radius() {
    let mut s = session("Box");
    let create = json!({"kind": "roundedRect", "rect": [40, 30, 120, 80], "radius": 12, "fill": "#ff0000"});
    let id = s.execute("shape.create", create.clone()).unwrap()["layer"].as_u64().unwrap();
    s.execute("shape.edit", json!({"layer": id, "transform": [2, 0, 0, 1, 0, 0]})).unwrap();
    let css = s.execute(ID, json!({"layer": id})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(!css.contains("border-radius") && css.contains(" {\n  background-color: rgb(255, 0, 0);\n"), "{css}");
    let flipped = s.execute("shape.create", create).unwrap()["layer"].as_u64().unwrap();
    s.execute("shape.edit", json!({"layer": flipped, "transform": [-1, 0, 0, 1, 400, 0]})).unwrap();
    let css = s.execute(ID, json!({"layer": flipped})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains(" {\n  border-radius: 12px;\n  background-color: rgb(255, 0, 0);\n"), "{css}");
}

/// A live ellipse writes `border-radius: 50%;` where a rounded rectangle writes its radii, so the
/// rule draws an ellipse, not a rectangle; other live shapes (a star here) write no geometry.
#[test]
fn an_ellipse_writes_a_half_border_radius() {
    let mut s = session("Box");
    let id = s.execute("shape.create", json!({"kind": "ellipse", "rect": [40, 30, 120, 80], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    let css = s.execute(ID, json!({"layer": id})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(css.contains(" {\n  border-radius: 50%;\n  background-color: rgb(255, 0, 0);\n  position: absolute;\n"), "{css}");
    let star = s.execute("shape.create", json!({"kind": "star", "rect": [40, 30, 120, 80], "fill": "#ff0000"})).unwrap()["layer"].as_u64().unwrap();
    let css = s.execute(ID, json!({"layer": star})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(!css.contains("border-radius") && css.contains(" {\n  background-color: rgb(255, 0, 0);\n"), "{css}");
}

/// `css` without its double-quoted strings (escapes inside them skipped).
fn outside_strings(css: &str) -> String {
    let (mut out, mut quoted, mut escaped) = (String::new(), false, false);
    for c in css.chars() {
        match (quoted, escaped, c) {
            (true, true, _) => escaped = false,
            (true, false, '\\') => escaped = true,
            (_, false, '"') => quoted = !quoted,
            (false, _, _) => out.push(c),
            _ => {}
        }
    }
    out
}

/// A name with a comment closer can't end a comment or break the rule: outside the quoted image
/// name there is no `*/`, and the class is sanitised.
#[test]
fn a_comment_closer_in_a_name_stays_inside_the_string() {
    let css = session("a */ b").execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
    assert!(!outside_strings(&css).contains("*/"), "{css}");
    assert!(css.starts_with(".a____b {\n  background-image: url(\"a */ b.png\");\n"), "{css}");
}

/// Layer effects aren't written: a layer with a drop shadow gives the same rule as without it,
/// placed and sized by its pixels, not the shadow.
#[test]
fn a_layer_with_a_drop_shadow_writes_its_rule_without_the_effect() {
    let mut s = session("Box");
    let plain = s.execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
    s.edit("fx", |doc, active| {
        let l = doc.layer_mut(active.unwrap()).unwrap();
        l.effects.enabled = true;
        l.effects.items.push(photocraft_doc::effects::Effect::default_drop_shadow());
        Ok(())
    })
    .unwrap();
    let css = s.execute(ID, json!({})).unwrap()["css"].as_str().unwrap().to_string();
    assert_eq!(css, plain);
    assert!(!css.contains("shadow"), "{css}");
}

/// Real point type goes through its layout: "Hello" at a baseline of 150 is placed by its line
/// box, which starts above its first pixel (Photoshop: 127.1 for pixels from 132), not at it.
#[test]
fn point_type_is_placed_by_its_line_box() {
    let mut s = session("Box");
    let id = s.execute("type.create", json!({"x": 50, "y": 150, "text": "Hello", "size": 24})).unwrap()["layer"].as_u64().unwrap();
    let doc = &s.active().unwrap().doc;
    let pix = photocraft_doc::comps::position_bounds(doc.walk().into_iter().find(|(_, _, l)| l.id.0 == id).unwrap().2);
    let css = s.execute(ID, json!({"layer": id})).unwrap()["css"].as_str().unwrap().to_string();
    let value = |key: &str| css.split(&format!("  {key}: ")).nth(1).and_then(|v| v.split("px;").next()).and_then(|v| v.parse::<f64>().ok()).unwrap();
    let (left, top) = (value("left"), value("top"));
    assert!((left - 50.0).abs() < 1.0, "left at the origin, within the pixel rounding: {css}");
    assert!(top < f64::from(pix.y0) - 2.0 && top > 150.0 - 28.8, "top {top} above the pixels at {}: {css}", pix.y0);
}
