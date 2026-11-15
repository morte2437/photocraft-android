//! #13805: hiding or showing another layer after a fadeable step keeps Edit › Fade available, as in
//! Photoshop; other steps in between still disable it.

use super::*;
use photocraft_doc::LayerId;

/// A command and its params, given the Background and the faded layer.
type Step = (&'static str, fn(LayerId, LayerId) -> Value);

/// A white Background and a layer `Paint` with a black square, made active and blurred:
/// (session, Background, Paint, Paint's edge pixel before the blur).
fn blurred() -> (Session, LayerId, LayerId, Vec<f32>) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": 8, "background": "white"})).unwrap();
    let bg = s.active().unwrap().active_layer.unwrap();
    let paint = LayerId(s.execute("layer.new.layer", json!({"name": "Paint"})).unwrap()["layer"].as_u64().unwrap());
    s.execute("select.rect", json!({"x": 16, "y": 16, "width": 16, "height": 16})).unwrap();
    s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    let sharp = pixel(&s, paint);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 3})).unwrap();
    assert_ne!(pixel(&s, paint), sharp, "the blur changed the edge pixel");
    assert!(s.is_enabled("edit.fade"), "Fade follows the blur");
    (s, bg, paint, sharp)
}

fn pixel(s: &Session, layer: LayerId) -> Vec<f32> {
    s.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().pixel(16, 20)
}

fn visible(s: &Session, layer: LayerId) -> bool {
    s.active().unwrap().doc.layer(layer).unwrap().visible
}

/// Hiding the Background after the blur, by each way of hiding it, with visibility changes
/// recorded in History or not: Fade stays available and fades the blur towards the square as it
/// was before the blur, and the Background stays hidden.
#[test]
fn hiding_another_layer_keeps_fade() {
    let hides: [Step; 3] = [
        ("layer.hideLayers", |bg, _| json!({"layer": bg.0})),
        ("layer.setProps", |bg, _| json!({"layer": bg.0, "visible": false})),
        ("layer.showOnly", |_, _| json!({})),
    ];
    for undoable in [true, false] {
        for (hide, params) in hides {
            let (mut s, bg, paint, sharp) = blurred();
            s.execute("history.options", json!({"makeLayerVisibilityChangesUndoable": undoable})).unwrap();
            assert!(s.is_enabled("edit.fade"), "History Options don't touch the document");
            let steps = s.active().unwrap().history.past_len();

            s.execute(hide, params(bg, paint)).unwrap();
            assert!(!visible(&s, bg), "{hide} undoable={undoable}: the Background is hidden");
            assert_eq!(s.active().unwrap().history.past_len(), steps + usize::from(undoable), "{hide} undoable={undoable}");
            assert!(s.is_enabled("edit.fade"), "{hide} undoable={undoable}: Fade stays available");
            let preview = fade_preview(&s, &json!({"opacity": 0})).and_then(FadePreview::render).unwrap();
            assert_eq!(preview.layer(paint).unwrap().surface().unwrap().pixel(16, 20), sharp, "{hide} undoable={undoable}: the preview fades the blur");

            s.execute("edit.fade", json!({"opacity": 0})).unwrap();
            assert_eq!(pixel(&s, paint), sharp, "{hide} undoable={undoable}: 0 % fades the blur away");
            assert!(!visible(&s, bg), "{hide} undoable={undoable}: Fade leaves the Background hidden");
            assert_eq!(s.active().unwrap().history.undo_label(), Some("Fade Gaussian Blur"));
            assert!(!s.is_enabled("edit.fade"), "{hide} undoable={undoable}: Fade can't repeat");
        }
    }
}

/// Showing the hidden layer again keeps Fade available too, and Fade at 50 % lands halfway
/// between the square before the blur and after it.
#[test]
fn hiding_and_showing_another_layer_keeps_fade() {
    let (mut s, bg, paint, sharp) = blurred();
    let blurred_px = pixel(&s, paint);
    s.execute("layer.hideLayers", json!({"layer": bg.0})).unwrap();
    s.execute("layer.showLayers", json!({"layer": bg.0})).unwrap();
    assert!(s.is_enabled("edit.fade"));
    s.execute("edit.fade", json!({"opacity": 50})).unwrap();
    let half = pixel(&s, paint);
    for c in 0..half.len() {
        assert!((half[c] - (sharp[c] + blurred_px[c]) / 2.0).abs() < 0.01, "channel {c}: {half:?} between {sharp:?} and {blurred_px:?}");
    }
}

/// Selecting another layer, or a step that is neither fadeable nor a visibility change, disables
/// Fade as before.
#[test]
fn other_steps_still_disable_fade() {
    let steps: [Step; 4] = [
        ("layer.select", |bg, _| json!({"layer": bg.0})),
        ("select.all", |_, _| json!({})),
        ("view.colorSamplers.add", |_, _| json!({"x": 15, "y": 15})),
        ("layer.setProps", |_, paint| json!({"layer": paint.0, "opacity": 0.5})),
    ];
    for (step, params) in steps {
        let (mut s, bg, paint, _) = blurred();
        s.execute(step, params(bg, paint)).unwrap();
        assert!(!s.is_enabled("edit.fade"), "{step}: Fade is disabled");
        assert!(s.execute("edit.fade", json!({})).is_err(), "{step}");
        // A visibility change afterwards doesn't bring it back.
        s.execute("layer.hideLayers", json!({"layer": bg.0})).unwrap();
        assert!(!s.is_enabled("edit.fade"), "{step}, then a hide: Fade stays disabled");
    }
    // A hide together with another property is not a visibility change only.
    let (mut s, bg, _, _) = blurred();
    s.execute("layer.setProps", json!({"layer": bg.0, "visible": false, "name": "Back"})).unwrap();
    assert!(!s.is_enabled("edit.fade"), "a hide with a rename disables Fade");
}

/// Hiding the faded layer itself was not measured in the reference: Fade stays disabled, by itself
/// and when a later step shows the layer again.
#[test]
fn hiding_the_faded_layer_disables_fade() {
    let (mut s, _, paint, _) = blurred();
    s.execute("layer.hideLayers", json!({})).unwrap();
    assert!(!visible(&s, paint), "Hide Layers hid the active, faded layer");
    assert!(!s.is_enabled("edit.fade"));
    s.execute("layer.showLayers", json!({"layer": paint.0})).unwrap();
    assert!(!s.is_enabled("edit.fade"), "showing it again doesn't bring Fade back");
}
