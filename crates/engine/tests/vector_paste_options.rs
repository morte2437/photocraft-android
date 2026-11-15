//! Explicit Paste modes preserve source kind instead of silently using the ordinary raster route.

use photocraft_engine::Session;
use serde_json::json;

/// This uses only the original public command API, so the pre-fix failure is executable.
#[test]
fn raster_clip_cannot_silently_paste_as_path() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 6, "background": "white"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.past_len();
    assert!(s.execute("edit.paste", json!({"as": "path"})).is_err(), "a raster-only clipboard must refuse Paste As Path");
    assert_eq!(s.active().unwrap().doc, before);
    assert_eq!(s.active().unwrap().history.past_len(), history);
}

use std::sync::Arc;

use craft_psd::{
    atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem},
    descriptor::{Class, Descriptor, Id, UnicodeString, Value as D},
};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::{Fill, Layer, LayerContent, Path, Subpath};
use photocraft_engine::{actions_cmds::Action, atn_cmds, edit_cmds::Clip};
use photocraft_geom::{Point, Size};
use serde_json::Value;

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="8"><path d="M1 1 C2 0 8 0 9 1 L9 7 L1 7 Z" fill="#ff0000" stroke="#0000ff" stroke-width="1"/></svg>"##;

fn scene(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 16, "depth": depth, "background": "transparent"})).unwrap();
    s.clipboard = Some(Clip::from_svg(SVG).unwrap());
    s
}

fn output(s: &Session) -> &Layer {
    let state = s.active().unwrap();
    state.doc.layer(state.active_layer.unwrap()).unwrap()
}

fn source(s: &Session) -> &Arc<Vec<u8>> {
    s.clipboard.as_ref().unwrap().svg.as_ref().unwrap()
}

fn assert_curve(path: &Path) {
    assert!(
        path.subpaths.iter().flat_map(|p| &p.knots).any(|k| k.anchor == Point::new(6.0, 5.0) && k.out_ctrl == Point::new(7.0, 4.0)),
        "original cubic controls must survive canvas-centred placement"
    );
}

/// Refusal keeps document, selection, revision, redo, and the immutable clipboard source.
fn rejected(s: &mut Session, p: Value) -> String {
    let state = s.active().unwrap();
    let before = state.doc.clone();
    let revision = state.revision;
    let selection = state.selected_layers.clone();
    let active = state.active_layer;
    let history = (state.history.past_len(), state.history.redo_label().map(str::to_owned));
    let clip = s.clipboard.clone().unwrap();
    let error = s.execute("edit.paste", p).unwrap_err().to_string();
    let state = s.active().unwrap();
    assert_eq!(state.doc, before);
    assert_eq!(state.revision, revision);
    assert_eq!(state.selected_layers, selection);
    assert_eq!(state.active_layer, active);
    assert_eq!((state.history.past_len(), state.history.redo_label().map(str::to_owned)), history);
    let after = s.clipboard.as_ref().unwrap();
    assert_eq!(after.surface, clip.surface);
    assert_eq!(after.bounds, clip.bounds);
    match (&after.svg, &clip.svg) {
        (Some(a), Some(b)) => assert!(Arc::ptr_eq(a, b)),
        (None, None) => {}
        _ => panic!("refusal changed source availability"),
    }
    error
}

/// All modes use original data, even when the supplied raster mirror is deliberately different.
#[test]
fn original_svg_drives_pixels_curves_and_shape_paint_with_one_undo() {
    let mut s = scene(8);
    s.edit("Existing work path", |doc, _| {
        doc.work_path = Some(Path { subpaths: vec![Subpath::polygon(&[(0.0, 0.0), (2.0, 0.0), (1.0, 2.0)])], ..Default::default() });
        Ok(())
    })
    .unwrap();
    let clip = s.clipboard.as_mut().unwrap();
    clip.surface.fill_rect(clip.bounds, &[0.0, 1.0, 0.0, 1.0]);
    let raw = source(&s).clone();
    for mode in ["pixels", "path", "shape"] {
        let before = s.active().unwrap().doc.clone();
        let history = s.active().unwrap().history.past_len();
        let result = s.execute("edit.paste", json!({"as":mode})).unwrap();
        assert_eq!(result["offset"], json!([5, 4]));
        match mode {
            "pixels" => {
                assert!(matches!(output(&s).content, LayerContent::Raster(_)));
                let px = output(&s).surface().unwrap().pixel(10, 8);
                assert!(px[0] > 0.9 && px[1] < 0.1 && px[3] > 0.9, "Pixels must render red source, not green mirror: {px:?}");
            }
            "path" => {
                assert_eq!(s.active().unwrap().doc.layers.len(), before.layers.len());
                assert_curve(&s.active().unwrap().doc.paths.last().unwrap().path);
                assert_eq!(s.active().unwrap().doc.work_path, before.work_path);
            }
            _ => {
                let LayerContent::Shape(shape) = &output(&s).content else {
                    panic!("Shape must remain editable");
                };
                assert_curve(&shape.path);
                assert!(matches!(&shape.fill,Some(Fill::Solid(c)) if c.c[0]>0.9 && c.c[1]<0.1));
                let stroke = shape.stroke.as_ref().unwrap();
                assert_eq!(stroke.width, 1.0);
                assert!(matches!(&stroke.paint,Fill::Solid(c) if c.c[2]>0.9));
            }
        }
        assert_eq!(s.active().unwrap().history.past_len(), history + 1);
        assert!(Arc::ptr_eq(source(&s), &raw));
        s.execute("edit.undo", json!({})).unwrap();
        assert_eq!(s.active().unwrap().doc, before);
        assert!(Arc::ptr_eq(source(&s), &raw));
    }
    s.execute("edit.paste", json!({})).unwrap();
    let legacy = output(&s).surface().unwrap().pixel(10, 8);
    assert!(legacy[1] > 0.9 && legacy[0] < 0.1, "omission keeps the ordinary raster mirror route");
    s.execute("edit.undo", json!({})).unwrap();
    let replaced = String::from_utf8(SVG.to_vec()).unwrap().replace("C2 0", "C3 0");
    s.clipboard = Some(Clip::from_svg(replaced.as_bytes()).unwrap());
    s.execute("edit.paste", json!({"as":"path"})).unwrap();
    assert!(s.active().unwrap().doc.paths.last().unwrap().path.subpaths.iter().flat_map(|p| &p.knots).any(|k| k.out_ctrl == Point::new(8.0, 4.0)));
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.paste", json!({"as":"path","center":[7,6]})).unwrap();
    assert!(s.active().unwrap().doc.paths.last().unwrap().path.subpaths.iter().flat_map(|p| &p.knots).any(|k| k.anchor == Point::new(3.0, 3.0)));
}

/// Native depth/profile adaptation applies to the real source and editable shape caches.
#[test]
fn source_outputs_follow_destination_depth_and_pixels_forces_raster_from_layer_copy() {
    for depth in [8, 16, 32] {
        for mode in ["pixels", "shape"] {
            let mut s = scene(depth);
            s.execute("edit.paste", json!({"as":mode})).unwrap();
            let expected = match depth {
                8 => SampleType::U8,
                16 => SampleType::U16,
                _ => SampleType::F32,
            };
            assert_eq!(output(&s).surface().unwrap().format().sample, expected);
            assert_eq!(output(&s).surface().unwrap().format().mode, ColorMode::Rgb);
        }
    }
    let mut s = scene(8);
    s.execute("edit.paste", json!({"as":"shape"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    assert!(s.clipboard.as_ref().unwrap().svg.is_none(), "ordinary Copy replaces vector source");
    assert!(s.clipboard.as_ref().unwrap().layers.is_some());
    rejected(&mut s, json!({"as":"path"}));
    s.execute("edit.paste", json!({})).unwrap();
    assert!(matches!(output(&s).content, LayerContent::Shape(_)), "omission keeps whole-layer recreation");
    s.execute("edit.undo", json!({})).unwrap();
    s.execute("edit.paste", json!({"as":"pixels"})).unwrap();
    assert!(matches!(output(&s).content, LayerContent::Raster(_)));
    s.execute("edit.purge.clipboard", json!({})).unwrap();
    assert!(s.clipboard.is_none());
}

/// Malformed modes, unsupported placement/targets, and unsupported source kinds fail atomically.
#[test]
fn invalid_modes_targets_and_capabilities_preserve_source_and_history() {
    let mut s = scene(8);
    s.execute("layer.new.layer", json!({})).unwrap();
    s.execute("edit.undo", json!({})).unwrap();
    for p in [
        json!({"as":"unknown"}),
        json!({"as":true}),
        json!({"as":null}),
        json!({"as":"shape","center":[0]}),
        json!({"as":"shape","center":[0,null]}),
        json!({"as":"shape","center":[1e20,0]}),
        json!({"as":"shape","target":"mask"}),
        json!({"as":"shape","clampToCanvas":false}),
        json!({"as":"shape","inPlace":false}),
    ] {
        rejected(&mut s, p);
    }
    let multi = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="8"><rect width="3" height="3"/><rect x="5" width="3" height="3"/></svg>"##;
    s.clipboard = Some(Clip::from_svg(multi).unwrap());
    rejected(&mut s, json!({"as":"shape"}));
    s.execute("edit.paste", json!({"as":"path"})).unwrap();
    assert_eq!(s.active().unwrap().doc.paths.last().unwrap().path.subpaths.len(), 2);
    let opacity = br##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="8"><g opacity=".5"><rect width="3" height="3"/></g></svg>"##;
    s.clipboard = Some(Clip::from_svg(opacity).unwrap());
    for mode in ["path", "shape"] {
        rejected(&mut s, json!({"as":mode}));
    }
    let mut empty = Session::new();
    empty.clipboard = Some(Clip::from_svg(SVG).unwrap());
    assert!(empty.execute("edit.paste", json!({"as":"pixels"})).is_err());
    assert!(empty.active().is_none());
    let mut locked = scene(8);
    locked
        .edit("Locked ancestor", |doc, active| {
            let child = Layer::raster("Child", doc.pixel_format());
            *active = Some(child.id);
            let mut group = Layer::group("Locked", vec![child]);
            group.locks.all = true;
            doc.layers.push(group);
            Ok(())
        })
        .unwrap();
    for mode in ["pixels", "shape"] {
        rejected(&mut locked, json!({"as":mode}));
    }
    locked.clipboard.as_mut().unwrap().svg = None;
    rejected(&mut locked, json!({"as":"pixels"}));
}

/// Existing geometry and cache limits apply before publication, including intermediate caches.
#[test]
fn source_and_destination_work_limits_refuse_before_editing() {
    assert!(Clip::from_svg(b"not svg").is_err());
    assert!(Clip::from_svg(&vec![b' '; 16 * 1024 * 1024 + 1]).is_err());
    assert!(Clip::from_svg(br##"<svg xmlns="http://www.w3.org/2000/svg" width="6000" height="6000"><rect width="6000" height="6000"/></svg>"##).is_err());
    let mut s = scene(8);
    let mut many = String::from("<svg xmlns='http://www.w3.org/2000/svg' width='10' height='8'>");
    for _ in 0..1025 {
        many.push_str("<rect width='1' height='1'/>");
    }
    many.push_str("</svg>");
    s.clipboard = Some(Clip::from_svg(many.as_bytes()).unwrap());
    assert!(rejected(&mut s, json!({"as":"path"})).contains("4096"));
    // The original position covers 36MP in this destination; the final position covers zero.
    let oversized = br##"<svg xmlns="http://www.w3.org/2000/svg" width="1" height="1"><path d="M0 0H6000V6000H0Z"/></svg>"##;
    let prepared = photocraft_io::svg::import_svg_bounded("control.svg", oversized, 24_000_000).unwrap();
    assert!(prepared.document.walk().iter().any(|(_, _, l)| matches!(l.content, LayerContent::Shape(_))), "the cache fixture must retain actual geometry");
    let mut doc = photocraft_doc::Document::new("Sparse destination", Size::new(7000, 7000), ColorMode::Rgb, SampleType::U8);
    doc.layers.push(Layer::raster("Pixels", doc.pixel_format()));
    let mut large = Session::new();
    large.add_document(doc, None);
    large.clipboard = Some(Clip::from_svg(oversized).unwrap());
    assert!(rejected(&mut large, json!({"as":"shape","center":[-10000,-10000]})).contains("cache exceeds 24 megapixels"));
}

fn descriptor(class_id: &str) -> Descriptor {
    Descriptor::new("null").with("As  ", D::Class(Class { name: UnicodeString::new_nul(""), class_id: Id::new(class_id) }))
}

fn recorded(d: Descriptor) -> Vec<u8> {
    atn::write(&AtnFile {
        name: UnicodeString::new_nul("Vector"),
        expanded: false,
        warnings: vec![],
        encoding: None,
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Paste"),
            items: vec![AtnItem { event: AtnEvent::Code(*b"past"), dictionary: b"Paste".to_vec(), descriptor: Some(d), ..Default::default() }],
            ..Default::default()
        }],
    })
    .unwrap()
}

fn load(s: &mut Session, bytes: &[u8]) -> Value {
    s.execute("actions.load", json!({"data":photocraft_engine::paint::tile::b64_encode(bytes)})).unwrap()
}
fn save(s: &mut Session) -> Vec<u8> {
    let v = s.execute("actions.save", json!({"set":"Vector"})).unwrap();
    assert_eq!(v["skipped"], json!([]));
    photocraft_engine::paint::tile::b64_decode(v["data"].as_str().unwrap()).unwrap()
}

/// Raw classes survive save/reimport and drive actual source-backed document output.
#[test]
fn atn_classes_roundtrip_and_play_real_modes() {
    for (class_id, mode) in [("Pxel", "pixels"), ("Path", "path"), ("shapeClass", "shape")] {
        let mut s = scene(8);
        assert_eq!(load(&mut s, &recorded(descriptor(class_id)))["unsupported"], json!([]));
        // `As Pxel` is what an ordinary Paste records: it keeps the ordinary route.
        let params = if mode == "pixels" { json!({}) } else { json!({"as":mode}) };
        assert_eq!(s.actions.list[0].steps, vec![("edit.paste".into(), params)]);
        let saved = save(&mut s);
        let file = atn::parse(&saved).unwrap();
        if mode != "pixels" {
            assert_eq!(file.actions[0].items[0].descriptor.as_ref().unwrap().get("As  "), descriptor(class_id).get("As  "));
        }
        let imported = atn_cmds::import(&saved).unwrap();
        assert_eq!(imported.actions[0].steps, s.actions.list[0].steps);
        s.actions.list = imported.actions;
        let result = s.execute("actions.play", json!({"action":"Paste"})).unwrap();
        assert_eq!(result["ran"], 1);
        assert!(result.get("failed").is_none(), "{result}");
        match mode {
            "path" => assert_curve(&s.active().unwrap().doc.paths.last().unwrap().path),
            "shape" => assert!(matches!(output(&s).content, LayerContent::Shape(_))),
            _ => assert!(matches!(output(&s).content, LayerContent::Raster(_))),
        }
    }
    let mut raster = scene(8);
    raster.clipboard.as_mut().unwrap().svg = None;
    assert_eq!(load(&mut raster, &recorded(descriptor("Pxel")))["unsupported"], json!([]));
    assert_eq!(raster.execute("actions.play", json!({"action":"Paste"})).unwrap()["ran"], 1);
    assert!(matches!(output(&raster).content, LayerContent::Raster(_)));
}

/// Unknown classes/types/placement remain complete kept steps; invalid native options refuse export.
#[test]
fn atn_unknown_forms_are_kept_and_legacy_omission_stays_legacy() {
    let mut s = scene(8);
    assert_eq!(load(&mut s, &recorded(Descriptor::new("null")))["unsupported"], json!([]));
    assert_eq!(s.actions.list[0].steps, vec![("edit.paste".into(), json!({}))]);
    for d in [
        descriptor("unknownClass"),
        Descriptor::new("null").with("As  ", D::Text(UnicodeString::new_nul("Path"))),
        descriptor("Path").with("inPlace", D::Boolean(true)),
        descriptor("Path").with("inPlace", D::Integer(0)),
        descriptor("Path").with("AntA", D::Enumerated { type_id: Id::new("Wrong"), value: Id::new("Anno") }),
    ] {
        let original = recorded(d);
        let file = atn::parse(&original).unwrap();
        let mut s = Session::new();
        assert_eq!(load(&mut s, &original)["unsupported"].as_array().unwrap().len(), 1);
        assert_eq!(s.actions.list[0].steps[0].0, atn_cmds::PHOTOSHOP_STEP);
        assert_eq!(atn::parse(&save(&mut s)).unwrap().actions[0].items[0].descriptor, file.actions[0].items[0].descriptor);
    }
    for p in [
        json!({"as":null}),
        json!({"as":"smartObject"}),
        json!({"as":"shape","center":[1,2]}),
        json!({"as":"path","target":"mask"}),
        json!({"as":"pixels","inPlace":false}),
    ] {
        let action = Action::new("Bad", vec![("edit.paste".into(), p)]);
        assert!(atn_cmds::export("Vector", &[&action], &[], false).is_err());
    }
}

fn recorded_pixels_paste(in_place: bool) -> Descriptor {
    let d = Descriptor::new("null").with("AntA", D::Enumerated { type_id: Id::new("Annt"), value: Id::new("Anno") });
    let d = if in_place { d.with("inPlace", D::Boolean(true)) } else { d };
    d.with("As  ", descriptor("Pxel").get("As  ").unwrap().clone())
}

/// A recorded ordinary Paste (`AntA Anno`, `As Pxel`) keeps whole-layer recreation of a copied layer.
#[test]
fn recorded_pixels_paste_keeps_the_ordinary_route() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 16, "background": "transparent"})).unwrap();
    s.clipboard = Some(Clip::from_svg(SVG).unwrap());
    s.execute("edit.paste", json!({"as":"shape"})).unwrap();
    s.execute("edit.copy", json!({})).unwrap();
    assert_eq!(load(&mut s, &recorded(recorded_pixels_paste(false)))["unsupported"], json!([]));
    assert_eq!(s.actions.list[0].steps, vec![("edit.paste".into(), json!({}))]);
    let layers = s.active().unwrap().doc.layers.len();
    let result = s.execute("actions.play", json!({"action":"Paste"})).unwrap();
    assert_eq!(result["ran"], 1);
    assert!(result.get("failed").is_none(), "{result}");
    assert!(s.active().unwrap().doc.layers.len() > layers);
    assert!(matches!(output(&s).content, LayerContent::Shape(_)), "a copied shape layer pastes as a shape layer");
}

/// A recorded Paste in Place (`inPlace true`, `AntA Anno`, `As Pxel`) plays as Paste in Place.
#[test]
fn recorded_pixels_paste_in_place_maps() {
    let mut s = Session::new();
    assert_eq!(load(&mut s, &recorded(recorded_pixels_paste(true)))["unsupported"], json!([]));
    assert_eq!(s.actions.list[0].steps, vec![("edit.pasteSpecial.pasteInPlace".into(), json!({}))]);
}
