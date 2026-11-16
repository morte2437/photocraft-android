use photocraft_doc::Size;
use photocraft_geom::Rect;

use super::*;

/// One action with a single recorded Canvas Size step.
fn canvas_file(descriptor: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Canvas"),
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Step"),
            items: vec![AtnItem { event: AtnEvent::Code(*b"CnvS"), dictionary: b"CnvS".to_vec(), descriptor: Some(descriptor), ..Default::default() }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

/// The 400 x 300 test document: a white Background with a red square at the top left and a blue
/// one at the bottom right, plus a layer "Top" with a green 50 x 50 square in the middle.
fn test_document() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300, "background": "white"})).unwrap();
    s.execute("layer.new.layer", json!({"name": "Top"})).unwrap();
    s.edit("paint", |doc, _| {
        doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(0, 0, 100, 100), &[1.0, 0.0, 0.0, 1.0]);
        doc.layers[0].surface_mut().unwrap().fill_rect(Rect::new(300, 200, 400, 300), &[0.0, 0.0, 1.0, 1.0]);
        doc.layers[1].surface_mut().unwrap().fill_rect(Rect::new(175, 125, 225, 175), &[0.0, 1.0, 0.0, 1.0]);
        Ok(())
    })
    .unwrap();
    s
}

/// What a Canvas Size step leaves: the document size, the bounds of "Top" and pixel (10, 10).
fn outcome(s: &Session) -> (Size, Rect, Vec<f32>) {
    let doc = &s.active().unwrap().doc;
    (doc.size, doc.layers[1].surface().unwrap().content_bounds(), doc.layers[0].surface().unwrap().pixel(10, 10))
}

/// The document after playing a recorded Relative +100 / +100 Canvas Size step with these anchor keys.
fn played(keys: &[(&str, &str, &str)]) -> Session {
    let mut descriptor = desc("null").with("Wdth", px(100.0)).with("Hght", px(100.0)).with("Rltv", D::Boolean(true));
    for (key, ty, value) in keys {
        descriptor = descriptor.with(key, enm(ty, value));
    }
    let mut s = test_document();
    let loaded = s.execute("actions.load", json!({"data":photocraft_paint::tile::b64_encode(&canvas_file(descriptor))})).unwrap();
    assert_eq!(loaded["unsupported"], json!([]));
    let result = s.execute("actions.play", json!({"action":loaded["actions"][0]})).unwrap();
    assert!(result.get("failed").is_none(), "{result}");
    s
}

const WHITE: [f32; 4] = [1.0, 1.0, 1.0, 1.0];
const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// A step with neither anchor key anchors at the top left.
#[test]
fn canvas_size_step_without_anchor_keys_anchors_top_left() {
    let s = played(&[]);
    assert_eq!(outcome(&s), (Size::new(500, 400), Rect::new(175, 125, 225, 175), RED.to_vec()));
}

/// The imported command names the top-left anchor.
#[test]
fn canvas_size_step_without_anchor_keys_imports_top_left() {
    let descriptor = desc("null").with("Wdth", px(100.0)).with("Hght", px(100.0)).with("Rltv", D::Boolean(true));
    let loaded = import(&canvas_file(descriptor)).unwrap();
    assert_eq!(loaded.actions[0].steps[0].1["anchor"], json!("topLeft"));
}

/// A step with both keys keeps today's mapping: Cntr / Cntr centres.
#[test]
fn canvas_size_step_with_both_keys_keeps_the_mapping() {
    let s = played(&[("Hrzn", "HrzL", "Cntr"), ("Vrtc", "VrtL", "Cntr")]);
    assert_eq!(outcome(&s), (Size::new(500, 400), Rect::new(225, 175, 275, 225), WHITE.to_vec()));
    let s = played(&[("Hrzn", "HrzL", "Left"), ("Vrtc", "VrtL", "Top")]);
    assert_eq!(outcome(&s), (Size::new(500, 400), Rect::new(175, 125, 225, 175), RED.to_vec()));
    let s = played(&[("Hrzn", "HrzL", "Rght"), ("Vrtc", "VrtL", "Bttm")]);
    assert_eq!(outcome(&s), (Size::new(500, 400), Rect::new(275, 225, 325, 275), WHITE.to_vec()));
}

/// A step with only one of the two keys keeps today's behaviour: the missing one is centred.
#[test]
fn canvas_size_step_with_one_key_centres_the_other_axis() {
    let s = played(&[("Hrzn", "HrzL", "Left")]);
    assert_eq!(outcome(&s), (Size::new(500, 400), Rect::new(175, 175, 225, 225), WHITE.to_vec()));
    let s = played(&[("Vrtc", "VrtL", "Top")]);
    assert_eq!(outcome(&s), (Size::new(500, 400), Rect::new(225, 125, 275, 175), WHITE.to_vec()));
}

/// `image.canvasSize` run without an `anchor` still centres.
#[test]
fn canvas_size_command_without_anchor_still_centres() {
    let mut s = test_document();
    s.execute("image.canvasSize", json!({"width": 100, "height": 100, "relative": true})).unwrap();
    assert_eq!(outcome(&s), (Size::new(500, 400), Rect::new(225, 175, 275, 225), WHITE.to_vec()));
}
