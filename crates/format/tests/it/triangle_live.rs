//! A live triangle (#13350) is saved in `triangle_live`, beside `live`, so builds from before the
//! kind still open the file and show the saved path.

use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Document, Fill, Layer, LayerContent, LiveShape, Path, ShapeLayer, Subpath};
use photocraft_format::*;
use serde_json::{Value, json};

fn triangle_json() -> Value {
    json!({"kind": "triangle", "rect": [400.0, 300.0, 200.0, 100.0], "corner_radius": 20.0})
}

fn path() -> Path {
    Path::new(vec![Subpath::polygon(&[(500.0, 300.0), (600.0, 400.0), (400.0, 400.0)])])
}

fn document(live: LiveShape) -> Document {
    let mut doc = Document::new("Triangle", photocraft_doc::Size::new(800, 600), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::new(
        "Triangle 1",
        LayerContent::Shape(ShapeLayer { path: path(), fill: Some(Fill::Solid(photocraft_doc::Color::BLACK)), live: Some(live), ..Default::default() }),
    ));
    doc
}

fn live(v: Value) -> LiveShape {
    serde_json::from_value(v).unwrap()
}

fn content(doc: &Document) -> Value {
    let bytes = save_to_bytes(doc, &SaveOptions::default()).unwrap();
    let json = serde_json::to_value(read_manifest(&bytes).unwrap()).unwrap();
    json["document"]["layers"][0]["content"].clone()
}

#[test]
fn a_live_triangle_round_trips_through_pcraft() {
    let doc = document(live(triangle_json()));
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

#[test]
fn a_live_triangle_is_written_beside_live_so_older_builds_open_the_file() {
    let saved = content(&document(live(triangle_json())));
    assert!(saved["live"].is_null());
    assert_eq!(saved["triangle_live"], triangle_json());
    assert_eq!(saved["path"], serde_json::to_value(path()).unwrap());

    // An old reader that knows only the earlier kinds.
    #[derive(serde::Deserialize, Debug, PartialEq)]
    struct LegacyShape {
        live: Option<Value>,
        path: Path,
    }
    let legacy: LegacyShape = serde_json::from_value(saved).unwrap();
    assert_eq!(legacy, LegacyShape { live: None, path: path() });
}

#[test]
fn a_three_sided_polygon_from_an_older_file_stays_a_polygon() {
    let doc = document(LiveShape::Polygon { rect: [400.0, 300.0, 200.0, 100.0], sides: 3, star_ratio: 1.0, corner_radius: 0.0 });
    let saved = content(&doc);
    assert_eq!(saved["live"]["kind"], "polygon");
    assert!(saved.get("triangle_live").is_none());
    let bytes = save_to_bytes(&doc, &SaveOptions::default()).unwrap();
    assert_eq!(load_from_bytes(&bytes).unwrap(), doc);
}

#[test]
fn the_triangle_kind_uses_these_json_names() {
    assert_eq!(serde_json::to_value(live(triangle_json())).unwrap(), triangle_json());
    let bare = live(json!({"kind": "triangle", "rect": [0.0, 0.0, 10.0, 10.0]}));
    assert_eq!(serde_json::to_value(bare).unwrap()["corner_radius"], json!(0.0));
}

/// `doc` saved, with `members` set on its first layer's content: what no save writes.
fn saved_with(doc: &Document, members: &Value) -> Vec<u8> {
    let bytes = save_to_bytes(doc, &SaveOptions::default()).unwrap();
    let bundle = zip::ZipReader::new(&bytes).unwrap();
    let mut out = zip::ZipWriter::new();
    for entry in &bundle.entries {
        let mut data = bundle.read(entry, usize::MAX).unwrap();
        if entry.name == "manifest.json" {
            let mut manifest: Value = serde_json::from_slice(&data).unwrap();
            for (key, value) in members.as_object().unwrap() {
                manifest["document"]["layers"][0]["content"][key] = value.clone();
            }
            data = serde_json::to_vec(&manifest).unwrap();
        }
        out.add(&entry.name, &data).unwrap();
    }
    out.finish().unwrap()
}

fn loaded_live(bytes: &[u8]) -> Option<LiveShape> {
    match &load_from_bytes(bytes).unwrap().layers[0].content {
        LayerContent::Shape(shape) => shape.live.clone(),
        _ => panic!("a shape layer"),
    }
}

/// No save writes these, but a hand-made or damaged file can hold them: the arrow field wins over
/// the triangle field, which wins over `live`; a field holding another kind is ignored; and a
/// value that is no live shape fails the load with an error.
#[test]
fn both_fields_a_wrong_kind_and_junk_never_panic() {
    let doc = document(live(triangle_json()));
    let arrow = json!({"kind": "arrowLine", "from": [0.0, 0.0], "to": [9.0, 0.0], "weight": 1.0, "start": false, "end": true, "width": 5.0, "length": 5.0, "concavity": 0.0});
    let rect = json!({"kind": "rect", "rect": [1.0, 2.0, 3.0, 4.0], "radii": [0.0, 0.0, 0.0, 0.0]});
    // Both new fields: the arrow wins. The triangle field wins over `live`.
    assert_eq!(loaded_live(&saved_with(&doc, &json!({"arrow_live": arrow}))), Some(live(arrow.clone())));
    assert_eq!(loaded_live(&saved_with(&doc, &json!({"live": rect}))), Some(live(triangle_json())));
    // A field holding another kind is ignored, and `live` is read instead.
    assert_eq!(loaded_live(&saved_with(&doc, &json!({"triangle_live": rect}))), None);
    assert_eq!(loaded_live(&saved_with(&doc, &json!({"triangle_live": arrow, "live": rect}))), Some(live(rect.clone())));
    assert_eq!(loaded_live(&saved_with(&doc, &json!({"arrow_live": triangle_json(), "triangle_live": null}))), None);
    // A value that is no live shape fails the load with an error.
    for junk in [
        json!(7),
        json!("x"),
        json!({}),
        json!({"kind": "hexagon"}),
        json!({"kind": "triangle"}),
        json!({"kind": "triangle", "rect": [1, 2, 3]}),
        json!({"kind": "triangle", "rect": [0, 0, "w", 10]}),
    ] {
        assert!(load_from_bytes(&saved_with(&doc, &json!({"triangle_live": junk}))).is_err(), "{junk}");
    }
}
