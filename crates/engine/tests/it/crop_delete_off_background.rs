//! Crop with Delete Cropped Pixels off turns the Background into a normal layer, "Layer 0", as
//! Photoshop does: it keeps the pixels past the canvas, and an extension is transparent on every
//! layer. Delete on keeps the Background, and Image › Reveal All is unchanged (#10867).

use photocraft_engine::actions_cmds::Action;
use photocraft_engine::{EngineError, Session, atn_cmds};
use photocraft_geom::Rect;
use photocraft_psd::{atn, descriptor::Value as D};
use serde_json::{Value, json};

const MAGENTA: [f32; 4] = [1.0, 0.0, 1.0, 1.0];
const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// The issue's test document at a tenth of the size: 80 × 50 px, a white Background and a pixel
/// layer "Quads" covering the canvas in red, with magenta as the background colour.
fn doc() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 50, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Quads"})).unwrap();
    s.execute("tools.setColors", json!({"foreground": "#ff0000", "background": "#ff00ff"})).unwrap();
    s.execute("select.all", json!({})).unwrap();
    s.execute("edit.fill", json!({"contents": "foreground"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

fn d(s: &Session) -> &photocraft_doc::Document {
    &s.active().unwrap().doc
}

fn size(s: &Session) -> (u32, u32) {
    (d(s).size.width, d(s).size.height)
}

fn past(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn bottom(s: &Session) -> &photocraft_doc::Layer {
    &d(s).layers[0]
}

fn content(s: &Session, i: usize) -> Rect {
    d(s).layers[i].surface().unwrap().content_bounds()
}

/// The bottom layer is no longer a Background: a normal, unlocked layer named "Layer 0".
fn assert_layer_0(s: &Session, what: &str) {
    assert!(d(s).background().is_none(), "{what}: no Background left");
    let l = bottom(s);
    assert_eq!(l.name, "Layer 0", "{what}");
    assert!(!l.locks.transparency && !l.locks.position, "{what}: unlocked");
}

fn crop(s: &mut Session, mut p: Value, delete: bool) {
    p["deleteCroppedPixels"] = json!(delete);
    let before = past(s);
    s.execute("image.crop", p).unwrap();
    assert_eq!(past(s), before + 1, "one undo step");
}

#[test]
fn shrinking_turns_the_background_into_layer_0_keeping_the_hidden_pixels() {
    let mut s = doc();
    crop(&mut s, json!({"x": 0, "y": 0, "width": 70, "height": 50}), false);
    assert_eq!(size(&s), (70, 50));
    assert_layer_0(&s, "shrink");
    // Both layers keep their 80 px, hidden past the canvas.
    assert_eq!(content(&s, 0), Rect::new(0, 0, 80, 50), "Layer 0");
    assert_eq!(content(&s, 1), Rect::new(0, 0, 80, 50), "Quads");
    assert_eq!(bottom(&s).surface().unwrap().rgba(75, 25), WHITE);
}

#[test]
fn extending_leaves_the_new_area_transparent_on_every_layer() {
    let mut s = doc();
    crop(&mut s, json!({"x": 0, "y": 0, "width": 90, "height": 50}), false);
    assert_eq!(size(&s), (90, 50));
    assert_layer_0(&s, "extend");
    for i in 0..2 {
        let surf = d(&s).layers[i].surface().unwrap();
        assert_eq!(surf.rgba(85, 25)[3], 0.0, "layer {i}: the added strip is transparent");
    }
    assert_eq!(bottom(&s).surface().unwrap().rgba(40, 25), WHITE);
    assert_eq!(d(&s).layers[1].surface().unwrap().rgba(40, 25), RED);
}

#[test]
fn a_turned_crop_turns_layer_0_with_the_others() {
    // The issue's 6.3° frame (687 × 429 of 800 × 500), centred.
    let mut s = doc();
    crop(&mut s, json!({"x": 6, "y": 4, "width": 68, "height": 42, "angle": 6.3}), false);
    assert_eq!(size(&s), (68, 42));
    assert_layer_0(&s, "turned");
    // Every layer turned, keeping its overhang past the canvas.
    let canvas = Rect::new(0, 0, 68, 42);
    for i in 0..2 {
        let c = content(&s, i);
        assert!(c.x0 < canvas.x0 && c.y0 < canvas.y0 && c.x1 > canvas.x1 && c.y1 > canvas.y1, "layer {i} keeps its overhang: {c:?}");
    }
    // The corners the turn uncovered are transparent, not the background colour.
    let surf = bottom(&s).surface().unwrap();
    let c = content(&s, 0);
    assert_eq!(surf.rgba(c.x0, c.y0)[3], 0.0, "an uncovered corner of Layer 0");
    assert_eq!(surf.rgba(34, 21), WHITE);
}

#[test]
fn the_action_steps_turned_crop_unlocks_the_background() {
    // The action step `Crop` with `Dlt` false, rect (100,100)–(700,400) and angle 10°, scaled down,
    // through a binary .atn: its descriptor carries `Dlt` false, the importer decodes it and
    // `actions.play` runs the step.
    let params = json!({"x": 10, "y": 10, "width": 60, "height": 30, "angle": 10, "deleteCroppedPixels": false});
    let action = Action::new("Crop", vec![("image.crop".into(), params)]);
    let (bytes, skipped) = atn_cmds::export("Crop", &[&action], &[], false).unwrap();
    assert!(skipped.is_empty());
    let file = atn::parse(&bytes).unwrap();
    let descriptor = file.actions[0].items[0].descriptor.as_ref().unwrap();
    assert_eq!(descriptor.get("Dlt "), Some(&D::Boolean(false)));
    let imported = atn_cmds::import(&bytes).unwrap();
    assert!(imported.unsupported.is_empty() && imported.warnings.is_empty());
    let step = &imported.actions[0].steps[0];
    assert_eq!((step.0.as_str(), &step.1["deleteCroppedPixels"]), ("image.crop", &json!(false)));

    let mut s = doc();
    s.actions.list = imported.actions;
    let before = past(&s);
    s.execute("actions.play", json!({"action": "Crop"})).unwrap();
    assert_eq!(past(&s), before + 1, "one undo step");
    assert_eq!(size(&s), (60, 30));
    assert_layer_0(&s, "action step");
}

#[test]
fn content_aware_with_delete_off_keeps_the_background() {
    // Not measured in the reference: the Background stays, and the fill goes on it, as before.
    let mut s = doc();
    let mut p = json!({"x": 0, "y": 0, "width": 90, "height": 50, "contentAware": true});
    p["deleteCroppedPixels"] = json!(false);
    s.execute("image.crop", p).unwrap();
    assert_eq!(size(&s), (90, 50));
    assert_eq!(d(&s).background().map(|l| l.name.as_str()), Some("Background"));
}

#[test]
fn a_target_size_crop_unlocks_the_background() {
    let mut s = doc();
    crop(&mut s, json!({"x": 0, "y": 0, "width": 40, "height": 50, "targetWidth": 20, "targetHeight": 25, "resolution": 150}), false);
    assert_eq!(size(&s), (20, 25));
    assert_layer_0(&s, "W x H x Resolution");
}

#[test]
fn delete_on_keeps_the_background_and_fills_an_extension() {
    for (frame, want) in [
        (json!({"x": 0, "y": 0, "width": 70, "height": 50}), (70, 50)),
        (json!({"x": 0, "y": 0, "width": 90, "height": 50}), (90, 50)),
        (json!({"x": 6, "y": 4, "width": 68, "height": 42, "angle": 6.3}), (68, 42)),
    ] {
        let mut s = doc();
        crop(&mut s, frame.clone(), true);
        assert_eq!(size(&s), want, "{frame}");
        let bg = d(&s).background().unwrap_or_else(|| panic!("{frame}: the Background stays"));
        assert_eq!(bg.name, "Background", "{frame}");
    }
    let mut s = doc();
    crop(&mut s, json!({"x": 0, "y": 0, "width": 90, "height": 50}), true);
    assert_eq!(bottom(&s).surface().unwrap().rgba(85, 25), MAGENTA, "the extension takes the background colour");
}

#[test]
fn undo_restores_the_background() {
    for frame in [json!({"x": 0, "y": 0, "width": 90, "height": 50}), json!({"x": 6, "y": 4, "width": 68, "height": 42, "angle": 6.3})] {
        let mut s = doc();
        let before = bottom(&s).clone();
        crop(&mut s, frame.clone(), false);
        assert_layer_0(&s, "cropped");
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(size(&s), (80, 50), "{frame}");
        let bg = d(&s).background().unwrap_or_else(|| panic!("{frame}: undo restores the Background"));
        assert_eq!(bg.id, before.id, "{frame}");
        assert_eq!(bg.name, "Background", "{frame}");
        assert!(bg.locks.transparency && bg.locks.position, "{frame}");
        let (a, b) = (bg.surface().unwrap(), before.surface().unwrap());
        let r = Rect::new(0, 0, 80, 50);
        assert_eq!(a.read_region(r), b.read_region(r), "{frame}: the pixels");
        s.execute("edit.redo", json!({})).unwrap();
        assert_layer_0(&s, "redo");
    }
}

#[test]
fn reveal_all_keeps_the_background() {
    let mut s = revealable();
    let before = past(&s);
    s.execute("image.revealAll", json!({})).unwrap();
    assert_eq!(past(&s), before + 1);
    assert_eq!(size(&s), (90, 50));
    let bg = d(&s).background().expect("Reveal All keeps the Background");
    assert_eq!(bg.name, "Background");
    assert_eq!(bg.surface().unwrap().rgba(85, 25), MAGENTA, "the extension takes the background colour, as before");
}

/// `doc()` with Quads reaching 10 px past the right edge, so Reveal All has something to reveal.
fn revealable() -> Session {
    let mut s = doc();
    s.edit("paint past the canvas", |doc, _| {
        let quads = doc.layers.last_mut().unwrap();
        quads.surface_mut().unwrap().fill_rect(Rect::new(80, 0, 90, 50), &RED);
        Ok(())
    })
    .unwrap();
    s
}

#[test]
fn reveal_alls_crop_passes_the_session_gate() {
    // A gate that refuses `image.crop` refuses Reveal All's crop too, before anything changes.
    let mut s = revealable();
    s.authorize = Some(|id, _| if id == "image.crop" { Err(EngineError::Other("image.crop is not allowed".into())) } else { Ok(()) });
    let before = past(&s);
    let err = s.execute("image.revealAll", json!({})).unwrap_err();
    assert!(err.to_string().contains("image.crop is not allowed"), "{err}");
    assert_eq!(size(&s), (80, 50));
    assert_eq!(past(&s), before);
    assert!(d(&s).background().is_some());

    // A gate that allows it sees the crop Reveal All runs.
    let mut s = revealable();
    // (An `Err`, not an assert: the gate runs inside the command's panic guard.)
    s.authorize = Some(|id, p| {
        let expected = json!({"x": 0, "y": 0, "width": 90, "height": 50, "deleteCroppedPixels": false});
        if id == "image.crop" && *p != expected { Err(EngineError::Other(format!("unexpected crop {p}"))) } else { Ok(()) }
    });
    s.execute("image.revealAll", json!({})).unwrap();
    assert_eq!(size(&s), (90, 50));
}

#[test]
fn a_document_without_a_background_has_nothing_to_convert() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 50, "background": "transparent"})).unwrap();
    let names: Vec<String> = d(&s).layers.iter().map(|l| l.name.clone()).collect();
    crop(&mut s, json!({"x": 0, "y": 0, "width": 90, "height": 50}), false);
    assert_eq!(size(&s), (90, 50));
    assert!(d(&s).background().is_none());
    assert_eq!(d(&s).layers.iter().map(|l| l.name.clone()).collect::<Vec<_>>(), names, "no layer renamed");
}
