//! #12959: `paint.smudge` takes the options bar's Mode (Normal, Darken, Lighten, Hue, Saturation,
//! Color, Luminosity) and blends the smudged colour with the pixel from before the stroke the way
//! a paint colour would be. The reference drags are 100 px along a row across a colour boundary,
//! Strength 50 %, a 20 px hard tip; here the boundary is at x 100 instead of x 400.

use serde_json::json;

use super::*;

const MODES: [&str; 7] = ["normal", "darken", "lighten", "hue", "saturation", "color", "luminosity"];
const RED: [i32; 3] = [200, 40, 40];
const BLUE: [i32; 3] = [40, 40, 200];
const BLACK: [i32; 3] = [0, 0, 0];
const WHITE: [i32; 3] = [255, 255, 255];
/// The measured positions (x 400 · 410 · 420 · 430 · 440 · 455 in the reference document).
const XS: [i32; 6] = [100, 110, 120, 130, 140, 155];

/// A 200 × 60 document of `depth` and `mode` whose left half is `left` and right half `right`.
fn two_tone(depth: u64, mode: &str, left: [i32; 3], right: [i32; 3]) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 200, "height": 60, "depth": depth, "mode": mode})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for _y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let c = if x < 100 { left } else { right };
                data.extend(photocraft_raster::from_rgba(&fmt, [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

fn stroke(mode: Option<&str>) -> Value {
    let mut p = json!({"points": [[50, 30], [150, 30]], "size": 20, "hardness": 100, "strength": 50});
    if let Some(mode) = mode {
        p["mode"] = json!(mode);
    }
    p
}

/// The 8-bit RGB of the active layer at a pixel.
fn rgb(s: &Session, x: i32, y: i32) -> [i32; 3] {
    let d = s.active().unwrap();
    let c = d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as i32)
}

/// Every pixel of the active layer.
fn image(s: &Session) -> Vec<[i32; 3]> {
    (0..60).flat_map(|y| (0..200).map(move |x| (x, y))).map(|(x, y)| rgb(s, x, y)).collect()
}

/// The document after the reference drag in `mode`.
fn dragged(left: [i32; 3], right: [i32; 3], mode: &str) -> Session {
    let mut s = two_tone(8, "rgb", left, right);
    s.execute("paint.smudge", stroke(Some(mode))).unwrap_or_else(|e| panic!("{mode}: {e}"));
    s
}

fn near(got: [i32; 3], want: [i32; 3], tol: i32) -> bool {
    got.iter().zip(want).all(|(g, w)| (g - w).abs() <= tol)
}

#[test]
fn mode_takes_the_seven_names_and_refuses_anything_else() {
    for mode in MODES {
        let s = dragged(RED, BLUE, mode);
        // The options bar sends the capitalised label.
        let label = format!("{}{}", mode[..1].to_uppercase(), &mode[1..]);
        assert_eq!(image(&s), image(&dragged(RED, BLUE, &label)), "{mode} and {label} are the same mode");
    }
    for bad in [json!("multiply"), json!("screen"), json!("dissolve"), json!("clear"), json!("bogus"), json!(""), json!(3), json!(null)] {
        let mut s = two_tone(8, "rgb", RED, BLUE);
        let (before, steps) = (image(&s), s.active().unwrap().history.past_len());
        let mut p = stroke(None);
        p["mode"] = bad.clone();
        let err = s.execute("paint.smudge", p.clone()).unwrap_err().to_string();
        assert!(err.contains("unknown mode") && err.contains("normal|darken|lighten|hue|saturation|color|luminosity"), "{bad}: {err}");
        assert_eq!(image(&s), before, "{bad}: nothing smudged");
        assert_eq!(s.active().unwrap().history.past_len(), steps, "{bad}: no history");
        assert!(LiveDab::begin(&s, "paint.smudge", &p).is_err(), "{bad}: the live stroke is refused too");
    }
}

#[test]
fn no_mode_and_normal_are_todays_smudge() {
    let mut plain = two_tone(8, "rgb", RED, BLUE);
    plain.execute("paint.smudge", stroke(None)).unwrap();
    assert_eq!(image(&plain), image(&dragged(RED, BLUE, "normal")));
    assert_eq!(rgb(&plain, 100, 30), RED, "red is dragged onto the boundary");
}

#[test]
fn darken_keeps_the_minimum_and_lighten_the_maximum() {
    let normal = dragged(RED, BLUE, "normal");
    let darken = dragged(RED, BLUE, "darken");
    let lighten = dragged(RED, BLUE, "lighten");
    assert_eq!(rgb(&darken, 100, 30), [40, 40, 40]);
    assert_eq!(rgb(&lighten, 100, 30), [200, 40, 200]);
    let original = image(&two_tone(8, "rgb", RED, BLUE));
    for ((d, l), o) in image(&darken).iter().zip(image(&lighten)).zip(&original) {
        assert!((0..3).all(|c| d[c] <= o[c]), "Darken never lightens a channel: {d:?} from {o:?}");
        assert!((0..3).all(|c| l[c] >= o[c]), "Lighten never darkens a channel: {l:?} from {o:?}");
    }
    // Along the trail each is the smudged colour's minimum / maximum with the original blue.
    for x in XS {
        let n = rgb(&normal, x, 30);
        assert!(near(rgb(&darken, x, 30), [n[0].min(BLUE[0]), n[1].min(BLUE[1]), n[2].min(BLUE[2])], 2), "darken at {x}");
        assert!(near(rgb(&lighten, x, 30), [n[0].max(BLUE[0]), n[1].max(BLUE[1]), n[2].max(BLUE[2])], 2), "lighten at {x}");
    }
    assert_ne!(image(&darken), image(&normal));
    assert_ne!(image(&lighten), image(&normal));
}

#[test]
fn component_modes_match_the_reference_at_the_boundary() {
    // x 400 of the reference's drag A.
    for (mode, want) in [("hue", [170, 10, 10]), ("saturation", [49, 49, 130]), ("color", [170, 10, 10]), ("luminosity", [70, 70, 230])] {
        let s = dragged(RED, BLUE, mode);
        let got = rgb(&s, 100, 30);
        assert!(near(got, want, 4), "{mode}: {got:?}, the reference has {want:?}");
        // The later columns follow the decay: the end of the trail is the untouched blue.
        assert!(near(rgb(&s, 155, 30), BLUE, 3), "{mode}: the trail has faded at its end");
        assert_eq!(rgb(&s, 100, 5), BLUE, "{mode}: off the stroke nothing changes");
    }
}

#[test]
fn component_modes_take_one_component_from_the_smudged_colour() {
    let lum = |c: [i32; 3]| 0.3 * c[0] as f32 + 0.59 * c[1] as f32 + 0.11 * c[2] as f32;
    let sat = |c: [i32; 3]| c.iter().max().unwrap() - c.iter().min().unwrap();
    for x in XS {
        // Hue, Saturation and Color keep the original blue's luminosity.
        for mode in ["hue", "saturation", "color"] {
            let got = rgb(&dragged(RED, BLUE, mode), x, 30);
            assert!((lum(got) - lum(BLUE)).abs() <= 3.0, "{mode} at {x}: luminosity of {got:?}");
        }
        // Luminosity keeps its hue and saturation: still a blue with equal red and green.
        let got = rgb(&dragged(RED, BLUE, "luminosity"), x, 30);
        assert!(got[0] == got[1] && got[2] > got[0] && (sat(got) - sat(BLUE)).abs() <= 2, "luminosity at {x}: {got:?}");
    }
}

#[test]
fn black_into_white_changes_only_in_normal_darken_and_luminosity() {
    let original = image(&two_tone(8, "rgb", BLACK, WHITE));
    for mode in ["lighten", "hue", "saturation", "color"] {
        assert_eq!(image(&dragged(BLACK, WHITE, mode)), original, "{mode} leaves the drag unchanged");
    }
    let normal = dragged(BLACK, WHITE, "normal");
    assert_ne!(image(&normal), original);
    for mode in ["darken", "luminosity"] {
        let s = dragged(BLACK, WHITE, mode);
        for x in XS {
            let (got, want) = (rgb(&s, x, 30), rgb(&normal, x, 30));
            assert!(got[0] == got[1] && got[1] == got[2], "{mode} at {x}: grey ({got:?})");
            assert!(near(got, want, 2), "{mode} at {x}: {got:?}, Normal has {want:?}");
        }
        assert_eq!(rgb(&s, 100, 30), BLACK, "{mode}: black reaches the boundary");
        assert_eq!(rgb(&s, 155, 30), WHITE, "{mode}: the end of the trail is white");
    }
}

#[test]
fn the_live_stroke_previews_the_mode() {
    // The canvas draws the stroke with the tool it commits, parsed from the same params.
    let s = two_tone(8, "rgb", RED, BLUE);
    let parsed = |p: &Value| DabTool::parse(&s, "paint.smudge", p, &parse_brush(&s, p, "paint.smudge").unwrap().0).unwrap().0;
    for mode in ["darken", "hue"] {
        let p = stroke(Some(mode));
        assert_ne!(parsed(&p), parsed(&stroke(None)), "{mode} is part of the tool");
        assert!(LiveDab::begin(&s, "paint.smudge", &p).is_ok(), "{mode}: live");
    }
    assert_eq!(parsed(&stroke(Some("normal"))), parsed(&stroke(None)));
}

#[test]
fn unmeasured_colour_modes_and_depths_smudge_as_normal() {
    for (depth, colour) in [(16, "rgb"), (32, "rgb"), (8, "gray"), (8, "cmyk")] {
        let mut normal = two_tone(depth, colour, RED, BLUE);
        normal.execute("paint.smudge", stroke(None)).unwrap();
        assert_ne!(image(&normal), image(&two_tone(depth, colour, RED, BLUE)), "{depth}-bit {colour}: smudged");
        for mode in ["darken", "hue", "luminosity"] {
            let mut s = two_tone(depth, colour, RED, BLUE);
            s.execute("paint.smudge", stroke(Some(mode))).unwrap_or_else(|e| panic!("{depth}/{colour}/{mode}: {e}"));
            assert_eq!(image(&s), image(&normal), "{depth}-bit {colour} {mode}: today's smudge");
        }
    }
}

#[test]
fn a_transparent_pixel_takes_the_smudged_colour() {
    // Nothing to blend with where the layer was empty: the smudge lands there as in Normal.
    let alpha = |s: &Session, x: i32| {
        let d = s.active().unwrap();
        d.doc.layer(d.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, 30)[3]
    };
    let run = |mode: &str| {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 60, "background": "transparent"})).unwrap();
        s.execute("paint.pencil", json!({"points": [[0, 30], [99, 30]], "size": 40, "color": "#c82828"})).unwrap();
        s.execute("paint.smudge", stroke(Some(mode))).unwrap();
        s
    };
    let normal = run("normal");
    assert!(alpha(&normal, 125) > 0.1 && alpha(&normal, 190) == 0.0, "the fixture drags red into the empty half");
    for mode in ["darken", "lighten", "hue", "saturation", "color", "luminosity"] {
        let s = run(mode);
        for x in [125, 130, 135] {
            assert!((alpha(&s, x) - alpha(&normal, x)).abs() < 0.02, "{mode} at {x}: alpha as Normal");
            assert!(near(rgb(&s, x, 30), rgb(&normal, x, 30), 3), "{mode} at {x}: {:?} vs {:?}", rgb(&s, x, 30), rgb(&normal, x, 30));
        }
    }
}

#[test]
fn blur_and_sharpen_keep_ignoring_a_mode() {
    for cmd in ["paint.blur", "paint.sharpen"] {
        let mut plain = two_tone(8, "rgb", RED, BLUE);
        plain.execute(cmd, stroke(None)).unwrap();
        let mut with = two_tone(8, "rgb", RED, BLUE);
        with.execute(cmd, stroke(Some("darken"))).unwrap();
        assert_eq!(image(&with), image(&plain), "{cmd}");
    }
}
