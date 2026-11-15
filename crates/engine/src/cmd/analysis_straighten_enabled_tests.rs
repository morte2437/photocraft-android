//! Straighten Layer's enabled states and its hidden-layer refusal, measured against the reference
//! application (#12033).
//!
//! With a ruler line drawn the reference greys the button out for the Background, two or more
//! selected layers, a group, a layer with Lock All and a layer with Lock Position. A hidden target
//! layer keeps the button enabled: the click answers an alert ("Could not straighten the layer
//! because the target layer is hidden.") and nothing changes. Pixel, text and smart object layers
//! stay enabled and are rotated.
//!
//! A layer with a mask, a shape or vector layer, an adjustment layer, artboards and the other lock
//! kinds are *not measured* there: they keep today's behaviour, so the tests below only pin that
//! the new check does not reach them.

use photocraft_doc::LayerId;

use super::*;

const CMD: &str = "image.analysis.straightenLayer";

/// A white document with a Background, as File > New makes one.
fn session(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "background": "white"})).unwrap();
    s
}

/// The same document in CMYK at 16 bits, which the reference also straightens.
fn cmyk_16(w: u32, h: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": w, "height": h, "mode": "cmyk", "depth": 16, "background": "white"})).unwrap();
    s
}

/// A ruler line to straighten by: without one the command is off for every target.
fn rule(s: &mut Session) {
    s.execute("image.analysis.rulerTool", json!({"start": [10, 30], "end": [70, 25]})).unwrap();
}

/// A block of opaque pixels on the layer: an empty layer has nothing to transform, which would
/// hide the state under test behind "nothing to transform". The colour has the layer's own channel
/// count, so it works for RGB, Grayscale and CMYK at 8 and 16 bits alike.
fn paint(s: &mut Session, l: LayerId) {
    s.edit("paint", |doc, _| {
        let surf = doc.layer_mut(l).unwrap().surface_mut().unwrap();
        let mut px = vec![0.0; surf.channels()];
        if let Some(first) = px.first_mut() {
            *first = 1.0;
        }
        if let Some(last) = px.last_mut() {
            *last = 1.0;
        }
        surf.fill_rect(Rect::new(10, 10, 50, 20), &px);
        Ok(())
    })
    .unwrap();
}

/// A new pixel layer with something on it, which `layer.new.layer` makes the active one.
fn pixel_layer(s: &mut Session, name: &str) -> LayerId {
    let l = LayerId(s.execute("layer.new.layer", json!({"name": name})).unwrap()["layer"].as_u64().unwrap());
    paint(s, l);
    l
}

/// A type layer over the canvas.
fn text_layer(s: &mut Session) -> LayerId {
    LayerId(s.execute("type.create", json!({"x": 40, "y": 50, "text": "Hi", "size": 20})).unwrap()["layer"].as_u64().unwrap())
}

/// A pixel layer turned into a smart object.
fn smart_layer(s: &mut Session) -> LayerId {
    let l = pixel_layer(s, "Smart");
    s.execute("layer.smartObjects.convertToSmartObject", json!({"layer": l.0})).unwrap();
    l
}

fn doc_of(s: &Session) -> Arc<Document> {
    s.active().unwrap().doc.clone()
}

fn steps(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

/// A state the reference greys the button out for: the engine says the command is off, so the menu
/// item and the options-bar button follow, and a call answered anyway (the CLI, the control channel,
/// an action step) is refused as a disabled command rather than run. Nothing is touched: the
/// document is the same `Arc`, no History step is added and the ruler line stays.
fn assert_greyed(s: &mut Session, what: &str) {
    let (before, history) = (doc_of(s), steps(s));
    assert!(!s.is_enabled(CMD), "{what}: the button is greyed out");
    let e = s.execute(CMD, json!({})).unwrap_err();
    let disabled = matches!(&e, EngineError::Disabled(id, _) if id == CMD);
    assert!(disabled, "{what}: refused as a disabled command, got {e}");
    assert!(Arc::ptr_eq(&before, &doc_of(s)), "{what}: the document was not touched");
    assert_eq!(steps(s), history, "{what}: no History step");
    assert!(doc_of(s).measurement.ruler.is_some(), "{what}: the ruler line is still there");
}

/// The angle the ruler line of [`rule`] straightens by.
const ROT: f64 = 4.7636;

#[test]
fn the_background_greys_straighten_layer_out() {
    // Today it rotates the canvas and crops it (800 x 600 becomes 500 x 375); the reference has no
    // button for it at all.
    let mut s = session(100, 80);
    assert!(doc_of(&s).is_background(s.active().unwrap().active_layer.unwrap()), "the Background is the target");
    rule(&mut s);
    assert_greyed(&mut s, "the Background");
}

#[test]
fn two_selected_layers_grey_straighten_layer_out() {
    let mut s = session(100, 80);
    let a = pixel_layer(&mut s, "One");
    let b = pixel_layer(&mut s, "Two");
    s.execute("layer.select", json!({"layer": a.0})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    assert_eq!(s.active().unwrap().selected_layers().len(), 2, "both selected");
    rule(&mut s);
    assert_greyed(&mut s, "two selected layers");
    // Back to one layer, the button is live again.
    s.execute("layer.select", json!({"layer": b.0})).unwrap();
    assert!(s.is_enabled(CMD), "one selected layer");
}

#[test]
fn a_group_greys_straighten_layer_out() {
    let mut s = session(100, 80);
    pixel_layer(&mut s, "Inner");
    s.execute("layer.groupLayers", json!({})).unwrap();
    let g = s.active().unwrap().active_layer.unwrap();
    assert!(doc_of(&s).layer(g).unwrap().is_group(), "the new group is the target");
    rule(&mut s);
    assert_greyed(&mut s, "a group");
    // A layer inside the group is a layer, so the button is on for it.
    let inner = doc_of(&s).layer(g).unwrap().children().unwrap()[0].id;
    s.execute("layer.select", json!({"layer": inner.0})).unwrap();
    assert!(s.is_enabled(CMD), "a layer in a group");
}

#[test]
fn lock_all_and_lock_position_grey_straighten_layer_out() {
    // Measured: both grey the button. Today it is enabled and the command fails with
    // `layer "Colors" is locked`, which is what a greyed button cannot do any more.
    for key in ["all", "position"] {
        let mut s = session(100, 80);
        let l = pixel_layer(&mut s, "Colors");
        s.execute("layer.setProps", json!({"layer": l.0, "locks": {key: true}})).unwrap();
        rule(&mut s);
        assert_greyed(&mut s, &format!("Lock {}", if key == "all" { "All" } else { "Position" }));
    }
}

#[test]
fn the_other_lock_kinds_are_not_reached_by_the_new_check() {
    // Not measured in the reference, so they keep today's behaviour: the button stays on and the
    // command answers as it does now.
    for key in ["transparency", "pixels", "artboard"] {
        let mut s = session(100, 80);
        let l = pixel_layer(&mut s, "Colors");
        s.execute("layer.setProps", json!({"layer": l.0, "locks": {key: true}})).unwrap();
        rule(&mut s);
        assert!(s.is_enabled(CMD), "Lock {key}: not measured, still enabled");
    }
}

#[test]
fn a_hidden_layer_stays_enabled_and_is_refused_with_the_alert() {
    // The one measured refusal: the button is on, the click shows the alert and nothing changes.
    let mut s = session(100, 80);
    let l = pixel_layer(&mut s, "Gone");
    s.execute("layer.setProps", json!({"layer": l.0, "visible": false})).unwrap();
    rule(&mut s);
    let (before, history) = (doc_of(&s), steps(&s));
    assert!(s.is_enabled(CMD), "a hidden layer keeps the button on");
    let e = s.execute(CMD, json!({})).unwrap_err();
    assert_eq!(e.to_string(), STRAIGHTEN_HIDDEN_ERROR, "the reference's alert text");
    assert!(matches!(e, EngineError::Other(_)), "an alert, not a greyed state: {e}");
    assert!(Arc::ptr_eq(&before, &doc_of(&s)), "nothing changed");
    assert_eq!(steps(&s), history, "no History step");
    assert!(doc_of(&s).measurement.ruler.is_some(), "a straighten that never ran leaves the ruler line");
}

#[test]
fn a_layer_hidden_with_its_group_is_refused_the_same_way() {
    // A layer inside a hidden group is hidden, as for the painting tools (#571).
    let mut s = session(100, 80);
    let l = pixel_layer(&mut s, "Inner");
    s.execute("layer.groupLayers", json!({})).unwrap();
    let g = s.active().unwrap().active_layer.unwrap();
    s.execute("layer.setProps", json!({"layer": g.0, "visible": false})).unwrap();
    s.execute("layer.select", json!({"layer": l.0})).unwrap();
    rule(&mut s);
    let (before, history) = (doc_of(&s), steps(&s));
    assert!(s.is_enabled(CMD), "the target is a layer, not the group");
    assert_eq!(s.execute(CMD, json!({})).unwrap_err().to_string(), STRAIGHTEN_HIDDEN_ERROR);
    assert!(Arc::ptr_eq(&before, &doc_of(&s)));
    assert_eq!(steps(&s), history);
}

#[test]
fn without_a_ruler_line_the_button_stays_greyed_for_every_layer() {
    let mut s = session(100, 80);
    pixel_layer(&mut s, "Paint");
    assert!(!s.is_enabled(CMD), "no ruler line");
    s.execute("layer.setProps", json!({"layer": s.active().unwrap().active_layer.unwrap().0, "visible": false})).unwrap();
    assert!(!s.is_enabled(CMD), "a hidden layer without a ruler line stays greyed, not enabled");
}

#[test]
fn an_active_layer_the_document_no_longer_holds_is_refused() {
    // `Session::edit` prunes the selected layers but not the active id, so an edit that drops the
    // active layer can leave one behind (`select_layer` and `restore_layer_selection` both refuse
    // such an id). The command must then say it is off, not rotate and crop the whole canvas for a
    // layer that is not there.
    let mut s = session(100, 80);
    let l = pixel_layer(&mut s, "Gone");
    s.edit("drop", |doc, _| {
        doc.layers.retain(|layer| layer.id != l);
        Ok(())
    })
    .unwrap();
    assert_eq!(s.active().unwrap().active_layer, Some(l), "the target still points at the dropped layer");
    assert!(doc_of(&s).layer(l).is_none(), "and the document does not hold it");
    rule(&mut s);
    assert_greyed(&mut s, "an active layer that is gone");
    // The destructive case this closes: `crop` asks for the canvas, which would turn and shrink for
    // a target that no longer exists.
    let size = (doc_of(&s).size.width, doc_of(&s).size.height);
    assert!(s.execute(CMD, json!({"crop": true})).is_err());
    assert_eq!((doc_of(&s).size.width, doc_of(&s).size.height), size, "the canvas kept its size");
}

#[test]
fn a_single_visible_layer_still_rotates_without_cropping() {
    // The pin for the state that stays: the layer turns about the canvas centre, the canvas does not.
    let mut s = session(100, 80);
    pixel_layer(&mut s, "Paint");
    rule(&mut s);
    let (history, size) = (steps(&s), (doc_of(&s).size.width, doc_of(&s).size.height));
    assert!(s.is_enabled(CMD), "a layer is what the button is for");
    let r = s.execute(CMD, json!({})).unwrap();
    assert_eq!(r["cropped"], false, "{r}");
    assert!((r["angle"].as_f64().unwrap() - ROT).abs() < 1e-3, "{r}");
    assert_eq!((doc_of(&s).size.width, doc_of(&s).size.height), size, "the canvas keeps its size");
    assert_eq!(steps(&s), history + 1, "one History step");
    assert!(doc_of(&s).measurement.ruler.is_none(), "the ruler line is cleared");
    assert!(s.undo());
    assert_eq!((doc_of(&s).size.width, doc_of(&s).size.height), size, "and one Undo takes it back");
}

#[test]
fn pixel_text_and_smart_object_layers_stay_enabled_and_rotate() {
    // Measured, also in a CMYK 16-bit document.
    type Maker = (&'static str, fn(&mut Session) -> LayerId);
    let make: [Maker; 3] = [("pixel", |s| pixel_layer(s, "Paint")), ("type", text_layer), ("smart object", smart_layer)];
    for (what, make_layer) in make {
        for new in [session as fn(u32, u32) -> Session, cmyk_16] {
            let mut s = new(100, 80);
            make_layer(&mut s);
            rule(&mut s);
            assert!(s.is_enabled(CMD), "{what}: enabled");
            let r = s.execute(CMD, json!({}));
            let r = r.unwrap_or_else(|e| panic!("{what}: {e}"));
            assert_eq!(r["cropped"], false, "{what}: rotated, not cropped");
            assert!((r["angle"].as_f64().unwrap() - ROT).abs() < 1e-3, "{what}: {r}");
        }
    }
}

#[test]
fn an_explicit_crop_still_crops_for_automation() {
    // The `crop` param stays available to automation where the command is enabled.
    let mut s = session(100, 80);
    pixel_layer(&mut s, "Paint");
    rule(&mut s);
    let r = s.execute(CMD, json!({"crop": true})).unwrap();
    assert_eq!(r["cropped"], true, "{r}");
    let d = doc_of(&s);
    assert!(d.size.width < 100 && d.size.height < 80, "the canvas is rotated and cropped to {:?}", [d.size.width, d.size.height]);
}
