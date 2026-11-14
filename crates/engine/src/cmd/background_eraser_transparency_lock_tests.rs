//! The Background Eraser on a layer with Lock transparent pixels on (#11905): the reference
//! application erases to transparency exactly as on an unlocked layer, leaves the lock on and shows
//! no alert. Lock image pixels and Lock all still refuse; the Eraser and the Magic Eraser keep their
//! own lock behaviour (they paint the background colour).

use super::*;

/// White Background plus a normal layer "Target" with a solid red band across it (y 10..30).
pub(super) fn session(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 40, "depth": depth})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Target"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 10, "width": 64, "height": 20, "antiAlias": false})).unwrap();
    s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

pub(super) fn target(s: &Session) -> &Layer {
    let d = s.active().unwrap();
    d.doc.layer(d.active_layer.unwrap()).unwrap()
}

fn alphas(s: &Session) -> Vec<f32> {
    let surf = target(s).surface().unwrap();
    (0..40).flat_map(|y| (0..64).map(move |x| (x, y))).map(|(x, y)| surf.rgba(x, y)[3]).collect()
}

fn click() -> Value {
    json!({"points": [[32, 20]], "size": 20, "hardness": 1, "tolerance": 50})
}

#[test]
fn a_click_on_a_transparency_locked_layer_erases_as_on_an_unlocked_one() {
    for depth in [8, 16, 32] {
        let mut free = session(depth);
        free.execute("paint.backgroundEraser", click()).unwrap();
        let want = alphas(&free);

        let mut s = session(depth);
        s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
        let h = s.active().unwrap().history.past_len();
        s.execute("paint.backgroundEraser", click()).unwrap();
        assert_eq!(s.active().unwrap().history.past_len(), h + 1, "{depth}-bit: one History step");
        assert_eq!(target(&s).surface().unwrap().rgba(32, 20)[3], 0.0, "{depth}-bit: erased to transparency");
        assert_eq!(alphas(&s), want, "{depth}-bit: the same alpha as on an unlocked layer");
        assert!(target(&s).locks.transparency, "{depth}-bit: the lock stays on");
        assert_eq!(target(&s).name, "Target", "{depth}-bit: the same layer");
    }
}

#[test]
fn a_drag_and_the_live_preview_erase_a_transparency_locked_layer_too() {
    let mut s = session(8);
    s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    let opts = |pts: &[[f64; 2]]| json!({"points": pts, "size": 12, "hardness": 1});
    let all = [[8.0, 20.0], [32.0, 20.0], [56.0, 20.0]];
    let mut live = live_background_eraser(&s, &opts(&all[..1])).unwrap();
    for q in &all[1..] {
        live.push(&[photocraft_paint::StrokePoint::new(q[0], q[1], 1.0)]).unwrap();
    }
    let id = s.active().unwrap().active_layer.unwrap();
    let shown = live.doc.layer(id).unwrap().surface().unwrap().clone();
    s.execute("paint.backgroundEraser", opts(&all)).unwrap();
    let surf = target(&s).surface().unwrap();
    for x in [8, 32, 56] {
        assert_eq!(surf.rgba(x, 20)[3], 0.0, "({x}, 20) erased");
        assert_eq!(shown.rgba(x, 20)[3], 0.0, "the preview shows ({x}, 20) erased");
    }
    assert_eq!(surf.rgba(32, 5)[3], 0.0, "transparent pixels stay transparent");
    assert!(target(&s).locks.transparency);
}

#[test]
fn with_a_colour_channel_target_the_preview_matches_the_commit_on_a_locked_layer() {
    // A channel target erases colour channels only (towards the background colour) and never
    // changes alpha. The commit mixes back without the lock, so the preview must too: at 32 bits
    // a lock-keeping mix rounds partly transparent pixels differently. The stroke crosses the
    // band's edge and a half-transparent strip, at half opacity, so every mix case is under the tip.
    for depth in [8, 16, 32] {
        let mut s = session(depth);
        s.execute("select.rect", json!({"x": 0, "y": 4, "width": 64, "height": 6, "antiAlias": false})).unwrap();
        s.execute("edit.fill", json!({"color": "#3a7fd0", "opacity": 37})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
        let opts = |pts: &[[f64; 2]]| json!({"points": pts, "size": 16, "hardness": 0.5, "opacity": 0.53, "target": {"colors": [1, 2]}});
        let all = [[8.0, 9.0], [32.0, 10.0], [56.0, 11.0]];
        let mut live = live_background_eraser(&s, &opts(&all[..1])).unwrap();
        for q in &all[1..] {
            live.push(&[photocraft_paint::StrokePoint::new(q[0], q[1], 1.0)]).unwrap();
        }
        let id = s.active().unwrap().active_layer.unwrap();
        let shown = live.doc.layer(id).unwrap().surface().unwrap().clone();
        let before = alphas(&s);
        s.execute("paint.backgroundEraser", opts(&all)).unwrap();
        let surf = target(&s).surface().unwrap();
        assert!(surf.rgba(32, 12)[1] > 0.0, "{depth}-bit: the green channel moved towards the white background");
        for (x, y) in (0..40).flat_map(|y| (0..64).map(move |x| (x, y))) {
            assert_eq!(shown.rgba(x, y), surf.rgba(x, y), "{depth}-bit ({x}, {y}): preview = commit");
        }
        assert_eq!(alphas(&s), before, "{depth}-bit: alpha untouched");
    }
}

#[test]
fn locked_pixels_and_lock_all_still_refuse() {
    for locks in [json!({"pixels": true}), json!({"all": true})] {
        let mut s = session(8);
        s.execute("layer.setProps", json!({"locks": locks})).unwrap();
        let before = alphas(&s);
        let h = s.active().unwrap().history.past_len();
        let err = s.execute("paint.backgroundEraser", click()).unwrap_err().to_string();
        assert!(err.contains("Could not complete your request because the layer \"Target\" is locked"), "{locks}: {err}");
        assert!(live_background_eraser(&s, &click()).is_err(), "{locks}: the preview refuses too");
        assert_eq!(alphas(&s), before, "{locks}: nothing erased");
        assert_eq!(s.active().unwrap().history.past_len(), h, "{locks}: no History step");
    }
}

#[test]
fn the_eraser_and_the_magic_eraser_keep_painting_the_background_colour() {
    // Magic Eraser: the background colour (white by default), still opaque.
    let mut s = session(8);
    s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    s.execute("paint.magicEraser", json!({"x": 32, "y": 20})).unwrap();
    assert_eq!(target(&s).surface().unwrap().rgba(32, 20), [1.0, 1.0, 1.0, 1.0]);
    // Eraser: the same.
    let mut s = session(8);
    s.execute("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
    s.execute("paint.stroke", json!({"erase": true, "points": [[8, 20], [56, 20]], "size": 8, "hardness": 1.0})).unwrap();
    assert_eq!(target(&s).surface().unwrap().rgba(32, 20), [1.0, 1.0, 1.0, 1.0]);
}
