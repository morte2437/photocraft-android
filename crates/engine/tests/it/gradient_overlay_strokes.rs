//! A Gradient Overlay spans one gradient over the layer's current bounds, so a stroke that grows
//! the layer re-lays the gradient out over all of it (#4053). The damage a stroke reports must
//! cover that, or a canvas that redraws only the damage keeps the old gradient around the earlier
//! strokes: one gradient per stroke instead of one over the whole layer.

use photocraft_engine::Session;
use serde_json::json;

/// What a canvas that redraws only each edit's damage shows: the cold render before the edit,
/// with the edit's damage (grown by the effects' reach, as the canvas grows it) rendered again.
fn redraw_damage(shown: &mut photocraft_compose::Buffer, s: &Session) {
    let st = s.active().unwrap();
    let reach = st.doc.layers.iter().filter(|l| photocraft_compose::effects::has_effects(l)).map(photocraft_compose::effects::margin).max().unwrap_or(0);
    let damage = st.last_damage.expect("a stroke reports bounded damage").inflate(reach).intersect(&st.doc.bounds());
    let (w, h) = (st.doc.size.width, st.doc.size.height);
    let patch = photocraft_compose::render_reduced_damage(&st.doc, w, h, damage);
    for y in patch.rect.y0..patch.rect.y1 {
        for x in patch.rect.x0..patch.rect.x1 {
            let i = (y * w as i32 + x) as usize;
            shown.px[i] = patch.get(x, y);
        }
    }
}

fn session(effect: &str, params: serde_json::Value) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 160, "height": 80, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute(effect, params).unwrap();
    s
}

const GREY: &str = "#808080";

/// Each stroke redrawn over the last cold render equals a cold render after it.
fn strokes_redraw_like_a_cold_render(mut s: Session, strokes: &[serde_json::Value]) {
    s.execute("paint.stroke", json!({"points": [[20, 20], [40, 30]], "size": 12, "hardness": 1.0, "color": GREY})).unwrap();
    for (i, p) in strokes.iter().enumerate() {
        let st = s.active().unwrap();
        let mut shown = photocraft_compose::render(&st.doc, st.doc.bounds());
        s.execute("paint.stroke", p.clone()).unwrap();
        redraw_damage(&mut shown, &s);
        let st = s.active().unwrap();
        let cold = photocraft_compose::render(&st.doc, st.doc.bounds());
        let worst = shown.px.iter().zip(&cold.px).map(|(a, b)| (0..4).map(|c| (a[c] - b[c]).abs()).fold(0.0f32, f32::max)).fold(0.0f32, f32::max);
        assert!(worst < 1e-6, "after stroke {i} the redrawn canvas differs from a cold render by {worst}");
    }
}

#[test]
fn a_gradient_overlay_is_laid_out_again_after_each_stroke() {
    let s = session("layer.layerStyle.gradientOverlay", json!({"from": "#ff0000", "to": "#0000ff", "angle": 0}));
    strokes_redraw_like_a_cold_render(
        s,
        &[
            // Far from the first: the layer grows to the right and down.
            json!({"points": [[120, 50], [140, 64]], "size": 12, "hardness": 1.0, "color": GREY}),
            // Up and to the left of both.
            json!({"points": [[6, 6], [10, 8]], "size": 6, "hardness": 1.0, "color": GREY}),
            // Erasing the far stroke shrinks the layer again.
            json!({"points": [[110, 40], [150, 74]], "size": 40, "hardness": 1.0, "erase": true}),
        ],
    );
}

#[test]
fn a_stroke_inside_the_layer_keeps_its_own_damage() {
    let mut s = session("layer.layerStyle.gradientOverlay", json!({"from": "#ff0000", "to": "#0000ff", "angle": 0}));
    s.execute("paint.stroke", json!({"points": [[10, 10], [150, 70]], "size": 12, "hardness": 1.0, "color": GREY})).unwrap();
    // Within the layer's bounds: the gradient's frame stays, so only the stroke repaints.
    let r = s.execute("paint.stroke", json!({"points": [[60, 40], [70, 40]], "size": 6, "hardness": 1.0, "color": GREY})).unwrap();
    let d = &r["damage"];
    let reported = photocraft_geom::Rect::new(
        d[0].as_i64().unwrap() as i32,
        d[1].as_i64().unwrap() as i32,
        (d[0].as_i64().unwrap() + d[2].as_i64().unwrap()) as i32,
        (d[1].as_i64().unwrap() + d[3].as_i64().unwrap()) as i32,
    );
    assert_eq!(s.active().unwrap().last_damage, Some(reported));
}

#[test]
fn effects_not_laid_out_over_the_layer_keep_the_stroke_damage() {
    let mut s = session("layer.layerStyle.colorOverlay", json!({"color": "#00ff00"}));
    s.execute("paint.stroke", json!({"points": [[20, 20], [40, 30]], "size": 12, "hardness": 1.0, "color": GREY})).unwrap();
    s.execute("paint.stroke", json!({"points": [[120, 50], [140, 64]], "size": 12, "hardness": 1.0, "color": GREY})).unwrap();
    let d = s.active().unwrap().last_damage.unwrap();
    assert!(d.x0 >= 100, "a Color Overlay repaints only the stroke: {d:?}");
}
