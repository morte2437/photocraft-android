//! Native Fill rotation enables quarter-turn candidates without changing ordinary Fill or Crop.

use photocraft_algo::content_aware::{FillOptions, color_level, fill, rotation_level};
use photocraft_doc::{LayerId, Rect};
use photocraft_engine::Session;
use photocraft_raster::Surface;
use serde_json::{Value, json};

const W: usize = 64;
const H: usize = 48;
const HOLE: Rect = Rect::new(28, 20, 36, 28);

fn fixture(depth: u32) -> (Session, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": W, "height": H, "depth": depth, "background": "transparent"})).unwrap();
    let id = s.active().unwrap().doc.layers[0].id;
    s.edit("Asymmetric texture", |doc, _| {
        let mut pixels = Vec::with_capacity(W * H * 4);
        for y in 0..H {
            for x in 0..W {
                pixels.extend([
                    ((37 * x + 11 * y + x * y % 17) % 251) as f32 / 250.0,
                    ((x * x + 13 * y + 3 * (x / 7)) % 241) as f32 / 240.0,
                    ((5 * x + 3 * y * y + 19 * (x / 9)) % 239) as f32 / 238.0,
                    1.0,
                ]);
            }
        }
        doc.layer_mut(id).unwrap().surface_mut().unwrap().write_region(Rect::from_xywh(0, 0, W as u32, H as u32), &pixels);
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

fn assert_pixels(actual: &[f32], expected: &[f32], message: &str) {
    assert_eq!(actual.len(), expected.len());
    assert_eq!(actual.iter().zip(expected).position(|(a, b)| a != b), None, "{message}");
}

/// Qualify rotation against the same bounded algorithm before checking the command wiring.
fn oracle(s: &Session, id: LayerId, rotate: bool) -> Vec<f32> {
    let input = surface(s, id);
    let mut hole = vec![false; W * H];
    for y in HOLE.y0..HOLE.y1 {
        for x in HOLE.x0..HOLE.x1 {
            hole[y as usize * W + x as usize] = true;
        }
    }
    let source: Vec<_> = hole.iter().map(|v| !v).collect();
    let options =
        FillOptions { color_adaptation: color_level("none"), rotations: if rotate { rotation_level("full") } else { vec![] }, seed: 7, ..Default::default() };
    let output = fill(W, H, input.channels(), &pixels(s, id), &hole, &source, &options);
    let mut encoded = input.clone();
    encoded.write_region(s.active().unwrap().doc.bounds(), &output);
    encoded.read_region(s.active().unwrap().doc.bounds())
}

/// A nonzero rotated/unrotated difference makes an ignored option fail at every native depth.
#[test]
fn rotated_candidates_reach_fill_at_all_depths_and_undo_once() {
    assert_eq!(rotation_level("full"), vec![90.0, 180.0, 270.0]);
    for depth in [8, 16, 32] {
        let (mut s, id) = fixture(depth);
        let before = s.active().unwrap().doc.clone();
        let on = oracle(&s, id, true);
        let off = oracle(&s, id, false);
        assert!(on != off, "the rotation fixture must discriminate at {depth}-bit");
        let history = s.active().unwrap().history.past_len();
        s.execute("edit.fill", json!({"contents": "contentAware", "colorAdaptation": false, "rotationAdaptation": true, "seed": 7})).unwrap();
        assert_pixels(&pixels(&s, id), &on, "rotationAdaptation must reach the native sampler");
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        let after = s.active().unwrap().doc.clone();
        let old = before.layer(id).unwrap().surface().unwrap().read_region(before.bounds());
        for y in 0..H {
            for x in 0..W {
                if !HOLE.contains(x as i32, y as i32) {
                    let i = (y * W + x) * 4;
                    assert_eq!(&on[i..i + 4], &old[i..i + 4]);
                }
            }
        }
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, before);
        s.execute("edit.redo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, after);
    }
}

/// Omission, false and null preserve old sampling; malformed flags preserve document and redo.
#[test]
fn defaults_and_invalid_rotation_flags_preserve_existing_behavior() {
    for value in [None, Some(Value::Null), Some(json!(false))] {
        let (mut s, id) = fixture(8);
        let expected = oracle(&s, id, false);
        let mut p = json!({"contents": "contentAware", "colorAdaptation": false, "seed": 7});
        if let Some(value) = value {
            p["rotationAdaptation"] = value;
        }
        s.execute("edit.fill", p).unwrap();
        assert_pixels(&pixels(&s, id), &expected, "ordinary Fill default");
    }
    for value in [json!(1), json!("full"), json!([]), json!({})] {
        let (mut s, _) = fixture(8);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.undo", json!({})).unwrap();
        let before = s.active().unwrap().doc.clone();
        let revision = s.active().unwrap().revision;
        let history = s.active().unwrap().history.past_len();
        let redo = s.active().unwrap().history.redo_label().map(str::to_owned);
        assert!(s.execute("edit.fill", json!({"contents": "contentAware", "rotationAdaptation": value})).is_err());
        assert_eq!(s.active().unwrap().doc, before);
        assert_eq!(s.active().unwrap().revision, revision);
        assert_eq!(s.active().unwrap().history.past_len(), history);
        assert_eq!(s.active().unwrap().history.redo_label(), redo.as_deref());
    }
}
