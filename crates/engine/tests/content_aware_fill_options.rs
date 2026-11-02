//! Content-Aware Fill exposes native scale and mirror adaptation without changing defaults.

use photocraft_algo::content_aware::{FillOptions, color_level, fill};
use photocraft_doc::{LayerId, Rect};
use photocraft_engine::Session;
use photocraft_raster::Surface;
use serde_json::{Value, json};

const WIDTH: usize = 64;
const HEIGHT: usize = 48;
const HOLE: Rect = Rect::new(28, 20, 36, 28);

fn fixture(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": WIDTH, "height": HEIGHT, "depth": depth, "background": "transparent"})).unwrap();
    let id = s.active().unwrap().doc.layers[0].id;
    s.edit("Asymmetric texture", |doc, _| {
        let surface = doc.layer_mut(id).unwrap().surface_mut().unwrap();
        let mut pixels = Vec::with_capacity(WIDTH * HEIGHT * 4);
        for y in 0..HEIGHT {
            for x in 0..WIDTH {
                pixels.extend([
                    ((37 * x + 11 * y + x * y % 17) % 251) as f32 / 250.0,
                    ((x * x + 13 * y + 3 * (x / 7)) % 241) as f32 / 240.0,
                    ((5 * x + 3 * y * y + 19 * (x / 9)) % 239) as f32 / 238.0,
                    1.0,
                ]);
            }
        }
        surface.write_region(Rect::from_xywh(0, 0, WIDTH as u32, HEIGHT as u32), &pixels);
        Ok(())
    })
    .unwrap();
    s.execute("select.rect", json!({"x": HOLE.x0, "y": HOLE.y0, "width": HOLE.width(), "height": HOLE.height()})).unwrap();
    (s, id)
}

fn surface(s: &Session, id: LayerId) -> &Surface {
    s.active().unwrap().doc.layer(id).unwrap().surface().unwrap()
}

fn pixels(s: &Session, id: LayerId) -> Vec<f32> {
    surface(s, id).read_region(s.active().unwrap().doc.bounds())
}

fn assert_pixels(actual: &[f32], expected: &[f32], label: &str) {
    assert_eq!(actual.len(), expected.len(), "{label}");
    let difference = actual.iter().zip(expected).enumerate().find(|(_, (a, b))| a != b).map(|(i, (a, b))| (i, *a, *b));
    assert_eq!(difference, None, "{label}");
}

/// The small canvas is wholly inside the command's sampling window.
fn oracle(s: &Session, id: LayerId, color: bool, scale: bool, mirror: bool) -> Vec<f32> {
    let input = surface(s, id);
    let mut hole = vec![false; WIDTH * HEIGHT];
    for y in HOLE.y0..HOLE.y1 {
        for x in HOLE.x0..HOLE.x1 {
            hole[y as usize * WIDTH + x as usize] = true;
        }
    }
    let source: Vec<_> = hole.iter().map(|v| !v).collect();
    let options = FillOptions { color_adaptation: color_level(if color { "default" } else { "none" }), scale, mirror, seed: 7, ..Default::default() };
    let filled = fill(WIDTH, HEIGHT, input.channels(), &pixels(s, id), &hole, &source, &options);
    // Match the destination's sample encoding at 8/16-bit depth.
    let mut expected = input.clone();
    expected.write_region(s.active().unwrap().doc.bounds(), &filled);
    expected.read_region(s.active().unwrap().doc.bounds())
}

#[test]
fn geometric_options_reach_native_fill_and_undo_as_one_edit() {
    for depth in [8, 16, 32] {
        for (key, scale, mirror) in [("scaleAdaptation", true, false), ("mirror", false, true)] {
            let (mut s, id) = fixture(depth);
            let before = pixels(&s, id);
            let expected = oracle(&s, id, false, scale, mirror);
            let default = oracle(&s, id, false, false, false);
            assert_ne!(expected, default, "fixture must distinguish {key} at {depth}-bit");
            let history = s.active().unwrap().history.past_len();
            let mut params = json!({"contents": "contentAware", "colorAdaptation": false, "seed": 7});
            params[key] = json!(true);
            s.execute("edit.fill", params).unwrap();
            assert_pixels(&pixels(&s, id), &expected, &format!("{key} at {depth}-bit"));
            assert_eq!(s.active().unwrap().history.past_len(), history + 1);
            for y in 0..HEIGHT {
                for x in 0..WIDTH {
                    if !HOLE.contains(x as i32, y as i32) {
                        let i = (y * WIDTH + x) * 4;
                        assert_eq!(&expected[i..i + 4], &before[i..i + 4]);
                    }
                }
            }
            s.execute("edit.undo", json!({})).unwrap();
            assert_pixels(&pixels(&s, id), &before, "undo");
            s.execute("edit.redo", json!({})).unwrap();
            assert_pixels(&pixels(&s, id), &expected, "redo");
        }
    }
}

#[test]
fn omitted_false_and_null_options_keep_existing_fill_defaults() {
    for depth in [8, 16, 32] {
        for value in [None, Some(json!(false)), Some(Value::Null)] {
            let (mut s, id) = fixture(depth);
            let expected = oracle(&s, id, true, false, false);
            let mut params = json!({"contents": "contentAware", "seed": 7});
            if let Some(value) = value {
                params["scaleAdaptation"] = value.clone();
                params["mirror"] = value;
            }
            s.execute("edit.fill", params).unwrap();
            assert_pixels(&pixels(&s, id), &expected, &format!("default at {depth}-bit"));
        }
    }
}

#[test]
fn invalid_geometric_options_fail_before_changing_pixels_or_history() {
    for key in ["scaleAdaptation", "mirror"] {
        for value in [json!(0), json!("false"), json!([]), json!({})] {
            let (mut s, id) = fixture(8);
            let before = pixels(&s, id);
            let history = s.active().unwrap().history.past_len();
            let mut params = json!({"contents": "contentAware", "seed": 7});
            params[key] = value;
            let error = s.execute("edit.fill", params).unwrap_err().to_string();
            assert!(error.contains(key), "{error}");
            assert_pixels(&pixels(&s, id), &before, "invalid option");
            assert_eq!(s.active().unwrap().history.past_len(), history);
        }
    }
    // Pattern's scale remains a numeric placement parameter and is not a CAF Boolean.
    let (mut s, _) = fixture(8);
    s.execute("edit.fill", json!({"contents": "pattern", "pattern": "Diagonal Lines", "scale": 75})).unwrap();
}
