//! Original geometric CSH fixture, assembled from the public container/path layout.
//! No Adobe or third-party shape assets are embedded.

use photocraft_engine::Session;
use serde_json::json;

#[path = "../../psd/tests/common/csh_fixture.rs"]
mod fixture;
use fixture::{polygon_csh, triangle_csh};

#[test]
fn csh_import_stores_the_display_form_of_a_localization_key() {
    let bytes = fixture::named_polygon_csh("$$$/Presets/Shapes/Tiger=Tiger", &[&[(0.25, 0.25), (0.75, 0.25), (0.5, 0.75)]]);
    let blank = fixture::named_polygon_csh("$$$/Presets/Shapes/Blank=   ", &[&[(0.25, 0.25), (0.75, 0.25), (0.5, 0.75)]]);
    let mut session = Session::new();
    let result = session.execute("shape.presets.importCsh", json!({"data": photocraft_paint::tile::b64_encode(&bytes), "group": "Localized CSH"})).unwrap();
    assert_eq!(result["group"], "Localized CSH");
    assert_eq!(result["names"], json!(["Tiger"]));
    assert_eq!(session.presets.shapes.iter().find(|g| g.name == "Localized CSH").unwrap().items[0].name, "Tiger");
    let again = session.execute("shape.presets.importCsh", json!({"data": photocraft_paint::tile::b64_encode(&bytes), "group": "Localized CSH"})).unwrap();
    assert_eq!(again["names"], json!(["Tiger 2"]));
    let blanked = session.execute("shape.presets.importCsh", json!({"data": photocraft_paint::tile::b64_encode(&blank)})).unwrap();
    assert_eq!(blanked["names"], json!(["Shape"]));
}

#[test]
fn csh_import_makes_an_original_shape_available_without_a_document() {
    let bytes = triangle_csh();
    if let Some(path) = std::env::var_os("PHOTOCRAFT_CSH_FIXTURE") {
        std::fs::write(path, &bytes).unwrap();
    }
    let mut session = Session::new();
    session
        .execute("shape.presets.importCsh", json!({"data": photocraft_paint::tile::b64_encode(&bytes), "group": "Original CSH"}))
        .expect("CSH import must be available without an open document");
    let groups = photocraft_engine::presets::shapes::all_groups(&session);
    let imported = groups.iter().find(|g| g.name == "Original CSH").expect("imported group");
    assert_eq!(imported.items.len(), 1);
    assert_eq!(imported.items[0].name, "Triangle");
    let path = &imported.items[0].path;
    assert_eq!(path.subpaths.len(), 1);
    assert!(path.subpaths[0].closed);
    assert_eq!(path.subpaths[0].knots.len(), 3);
    // The reference bounds are twice as wide as tall; normalization must preserve that.
    assert_eq!(path.control_bounds().unwrap(), (0.0, 25.0, 100.0, 75.0));
}

#[test]
fn csh_failed_import_preserves_existing_presets_and_revision() {
    let mut session = Session::new();
    let before = session.presets.to_json(&session);
    let revision = session.presets.rev;
    let bytes = triangle_csh();
    for end in [0, 11, bytes.len() - 1] {
        assert!(session.execute("shape.presets.importCsh", json!({"data": photocraft_paint::tile::b64_encode(&bytes[..end])})).is_err());
        assert_eq!(session.presets.to_json(&session), before);
        assert_eq!(session.presets.rev, revision);
    }
    assert!(session.execute("shape.presets.importCsh", json!({"data": photocraft_paint::tile::b64_encode(&bytes), "group": 12})).is_err());
    assert_eq!(session.presets.to_json(&session), before);
}

#[test]
fn csh_invalid_second_shape_does_not_partially_import_a_set() {
    let triangle = triangle_csh();
    let mut set = triangle.clone();
    set[8..12].copy_from_slice(&2u32.to_be_bytes());
    set.extend_from_slice(&triangle[12..]);
    // The first shape is complete; truncate the second shape's path/padding.
    set.pop();
    let mut session = Session::new();
    session.execute("shape.presets.importCsh", json!({"data": photocraft_paint::tile::b64_encode(&triangle)})).unwrap();
    let before = session.presets.to_json(&session);
    let revision = session.presets.rev;
    assert!(session.execute("shape.presets.importCsh", json!({"data": photocraft_paint::tile::b64_encode(&set)})).is_err());
    assert_eq!(session.presets.to_json(&session), before);
    assert_eq!(session.presets.rev, revision);
}

#[test]
fn csh_multiple_shapes_with_matching_names_remain_individually_available() {
    let triangle = triangle_csh();
    let mut set = triangle.clone();
    set[8..12].copy_from_slice(&2u32.to_be_bytes());
    set.extend_from_slice(&triangle[12..]);
    let mut s = Session::new();
    let result = s.execute("shape.presets.importCsh", json!({"data": photocraft_paint::tile::b64_encode(&set), "group": "Pair"})).unwrap();
    assert_eq!(result["count"], 2);
    assert_eq!(result["names"], json!(["Triangle", "Triangle 2"]));
    let group = s.presets.shapes.iter().find(|g| g.name == "Pair").unwrap();
    assert_eq!(group.items.len(), 2);
    assert_eq!(group.items[0].path, group.items[1].path);
    s.execute("file.new", json!({"width": 32, "height": 32})).unwrap();
    for name in ["Triangle", "Triangle 2"] {
        s.execute("shape.presets.place", json!({"preset": name, "group": "Pair"})).unwrap();
    }
}

#[test]
fn csh_repeat_import_names_are_unique_and_survive_preferences_reload() {
    let mut session = Session::new();
    let params = json!({"data": photocraft_paint::tile::b64_encode(&triangle_csh()), "group": "Original CSH"});
    let first = session.execute("shape.presets.importCsh", params.clone()).unwrap();
    let second = session.execute("shape.presets.importCsh", params).unwrap();
    assert_eq!(first["count"], 1);
    assert_eq!(second["count"], 1);
    assert_ne!(first["group"], second["group"]);
    assert_ne!(first["names"], second["names"]);
    let persisted = session.presets.to_json(&session);
    let mut restored = Session::new();
    restored.load_presets_json(persisted);
    assert_eq!(restored.presets.shapes, session.presets.shapes);
}

#[test]
fn csh_same_winding_inner_subpath_stays_a_hole_after_place_and_reload() {
    // Same winding intentionally: a nonzero union would incorrectly fill the hole.
    let bytes =
        polygon_csh(&[&[(0.125, 0.125), (0.875, 0.125), (0.875, 0.875), (0.125, 0.875)], &[(0.375, 0.375), (0.625, 0.375), (0.625, 0.625), (0.375, 0.625)]]);
    let mut imported = Session::new();
    let result = imported.execute("shape.presets.importCsh", json!({"data": photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
    let mut s = Session::new();
    s.load_presets_json(imported.presets.to_json(&imported));
    s.execute("file.new", json!({"width": 96, "height": 96, "background": "transparent"})).unwrap();
    s.execute("shape.presets.place", json!({"preset": result["names"][0], "rect": [8, 8, 80, 80], "keepAspect": false, "fill": "#ff0000"})).unwrap();
    let image = photocraft_compose::flatten(&s.active().unwrap().doc);
    assert!(image.px[48 * 96 + 16][0] > 0.99, "ring must remain red");
    assert!(image.px[48 * 96 + 16][3] > 0.99, "ring must remain opaque");
    assert!(image.px[48 * 96 + 48][3] < 0.01, "inner subpath must remain transparent");
    assert!(image.px[4 * 96 + 4][3] < 0.01, "outside shape must remain transparent");
    s.execute("edit.undo", json!({})).unwrap();
    assert!(photocraft_compose::flatten(&s.active().unwrap().doc).px.iter().all(|pixel| pixel[3] < 0.01));
}
