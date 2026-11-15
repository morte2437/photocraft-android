use super::*;
use photocraft_doc::{Layer, LayerContent};
use photocraft_geom::Rect;

fn session() -> (Session, LayerId, LayerId) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 80, "height": 60})).unwrap();
    let (a, b) = s
        .edit("setup", |doc, _| {
            let fmt = doc.pixel_format();
            let mut a = Layer::raster("A", fmt);
            a.surface_mut().unwrap().fill_rect(Rect::new(4, 4, 24, 24), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
            let mut b = Layer::raster("B", fmt);
            b.surface_mut().unwrap().fill_rect(Rect::new(30, 20, 50, 40), &photocraft_raster::from_rgba(&fmt, [0.0, 0.0, 1.0, 1.0]));
            let (ia, ib) = (a.id, b.id);
            doc.layers.push(a);
            doc.layers.push(b);
            Ok((ia, ib))
        })
        .unwrap();
    (s, a, b)
}

#[test]
fn frames_selected_layers_into_a_masked_group() {
    let (mut s, a, b) = session();
    s.execute("layer.select", json!({"layer": a.0, "mode": "replace"})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let r = s.execute("layer.new.frameFromLayers", json!({"name": "Frame 1"})).unwrap();
    let gid = LayerId(r["layer"].as_u64().unwrap());
    // frame rect = union of the two layers' bounds (4,4)..(50,40).
    let frame = r["frame"].as_array().unwrap();
    assert_eq!(frame[2].as_i64().unwrap(), 50 - 4); // width
    assert_eq!(frame[3].as_i64().unwrap(), 40 - 4); // height
    let st = s.active().unwrap();
    let g = st.doc.layer(gid).unwrap();
    assert!(matches!(g.content, LayerContent::Group(_)), "it's a group");
    assert!(g.frame.is_some() && g.vector_mask.is_some(), "the group has a clip mask");
    // Undoable (last step is the frame).
    assert_eq!(st.history.entries().last().map(|e| e.as_str()), Some("Frame from Layers"));
}

#[test]
fn frame_from_layers_is_one_history_step() {
    // #496: grouping and framing undo together.
    let (mut s, a, b) = session();
    let entries = s.active().unwrap().history.entries();
    let layers = |s: &Session| s.active().unwrap().doc.layers.iter().map(|l| l.id).collect::<Vec<_>>();
    let before = layers(&s);
    s.execute("layer.select", json!({"layer": a.0, "mode": "replace"})).unwrap();
    s.execute("layer.select", json!({"layer": b.0, "mode": "add"})).unwrap();
    let gid = LayerId(s.execute("layer.new.frameFromLayers", json!({})).unwrap()["layer"].as_u64().unwrap());
    let mut framed = entries.clone();
    framed.push("Frame from Layers".into());
    assert_eq!(s.active().unwrap().history.entries(), framed);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().history.entries(), entries);
    assert_eq!(layers(&s), before, "one undo leaves no group behind");
    assert!(s.redo());
    assert!(s.active().unwrap().doc.layer(gid).is_some_and(|g| g.frame.is_some() && g.vector_mask.is_some()), "redo brings back the framed group");
}

/// Frame tool: a rectangular frame is an empty group clipped to the drawn rect, undone in one
/// step; an elliptical one hides the corners; drawn over a pixel layer, it holds that layer.
#[test]
fn new_frame_clips_a_group_to_the_drawn_shape() {
    let (mut s, a, _) = session();
    let count = |s: &Session| s.active().unwrap().doc.layers.len();
    let before = count(&s);
    let r = s.execute("layer.new.frame", json!({"x": 10, "y": 10, "width": 40, "height": 30})).unwrap();
    let gid = LayerId(r["layer"].as_u64().unwrap());
    let st = s.active().unwrap();
    let g = st.doc.layer(gid).unwrap();
    assert!(matches!(&g.content, LayerContent::Group(c) if c.children.is_empty()), "an empty frame");
    let m = TestMask(g.vector_mask.as_ref().unwrap().path.clone());
    assert_eq!((m.value(10, 10), m.value(49, 39), m.value(9, 10), m.value(50, 39)), (1.0, 1.0, 0.0, 0.0));
    assert_eq!((g.name.as_str(), st.active_layer), ("Frame 1", Some(gid)));
    assert!(s.undo());
    assert_eq!(count(&s), before, "one undo removes the frame");

    let r = s.execute("layer.new.frame", json!({"x": 10, "y": 10, "width": 40, "height": 30, "shape": "ellipse", "layer": a.0})).unwrap();
    let gid = LayerId(r["layer"].as_u64().unwrap());
    let g = s.active().unwrap().doc.layer(gid).unwrap();
    assert!(matches!(&g.content, LayerContent::Group(c) if c.children.iter().map(|l| l.id).eq([a])), "the frame holds layer A");
    let m = TestMask(g.vector_mask.as_ref().unwrap().path.clone());
    assert_eq!((m.value(30, 25), m.value(10, 10)), (1.0, 0.0), "centre shown, corner hidden");
    for bad in [json!({"x": 0, "y": 0, "width": 0, "height": 5}), json!({"x": 0, "y": 0, "width": 5, "height": 5, "shape": "star"}), json!({"x": 0})] {
        assert!(s.execute("layer.new.frame", bad).is_err());
    }
}

/// #4435: a type layer (and the other rendered layer kinds) goes into a frame like a pixel layer.
#[test]
fn new_frame_takes_a_type_layer() {
    let (mut s, a, _) = session();
    let tid = s
        .edit("setup", |doc, _| {
            let cache = doc.layer(a).and_then(|l| l.surface()).cloned();
            let t = Layer::new("T", LayerContent::Text(photocraft_doc::TextLayer { cache, ..Default::default() }));
            let id = t.id;
            doc.layers.push(t);
            Ok(id)
        })
        .unwrap();
    let r = s.execute("layer.new.frame", json!({"x": 0, "y": 0, "width": 40, "height": 30, "layer": tid.0})).unwrap();
    let gid = LayerId(r["layer"].as_u64().unwrap());
    let g = s.active().unwrap().doc.layer(gid).unwrap();
    assert!(matches!(&g.content, LayerContent::Group(c) if c.children.iter().map(|l| l.id).eq([tid])), "the frame holds the type layer");
    assert!(s.undo());
    assert!(s.active().unwrap().doc.layer(tid).is_some(), "undo gives the type layer back");
}

#[test]
fn needs_a_selected_layer() {
    let mut s = Session::new();
    assert!(s.execute("layer.new.frameFromLayers", json!({})).is_err());
}

struct TestMask(photocraft_doc::Path);
impl TestMask {
    fn value(&self, x: i32, y: i32) -> f32 {
        photocraft_vector::path_coverage(&self.0, Rect::new(x, y, x + 1, y + 1))[0]
    }
}

#[test]
fn frame_and_smart_image_transform_independently_at_every_depth() {
    for depth in [8, 16, 32] {
        let mut s = Session::new();
        s.execute("file.new", json!({"width":80,"height":60,"depth":depth})).unwrap();
        let source = s
            .edit("Image", |doc, _| {
                let mut l = Layer::raster("Photo", doc.pixel_format());
                l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 80, 60), &photocraft_raster::from_rgba(&doc.pixel_format(), [1., 0., 0., 1.]));
                let id = l.id;
                doc.layers.push(l);
                Ok(id)
            })
            .unwrap();
        let id = LayerId(s.execute("layer.new.frame", json!({"x":10,"y":10,"width":40,"height":30,"layer":source.0})).unwrap()["layer"].as_u64().unwrap());
        let original = s.active().unwrap().doc.layer(source).unwrap().clone();
        assert!(matches!(original.content, LayerContent::Smart(_)));
        s.execute("layer.frame.set", json!({"layer":id.0,"transform":[20,0,0,25,30,12]})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(source).unwrap(), &original, "frame-only editing preserves the complete image");
        let frame = s.active().unwrap().doc.layer(id).unwrap().frame.clone();
        s.execute("layer.translate", json!({"layer":source.0,"dx":4,"dy":6})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().frame, frame, "content-only movement leaves the frame");
        s.execute("layer.translate", json!({"layer":id.0,"dx":5,"dy":7})).unwrap();
        let f = s.active().unwrap().doc.layer(id).unwrap().frame.as_ref().unwrap();
        assert_eq!(f.transform.m, [20., 0., 0., 25., 35., 19.]);
        let before = s.active().unwrap().doc.clone();
        s.execute("edit.transform", json!({"layer":id.0,"rect":[35,19,55,44],"quad":[[35,19],[75,19],[75,69],[35,69]]})).unwrap();
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().frame.as_ref().unwrap().transform.m, [40., 0., 0., 50., 35., 19.]);
        assert!(s.undo());
        assert_eq!(s.active().unwrap().doc.layer(id).unwrap().frame, before.layer(id).unwrap().frame);
    }
}

#[test]
fn frame_placement_covers_and_replaces_with_one_undo() {
    let (mut s, _, _) = session();
    let id = LayerId(s.execute("layer.new.frame", json!({"x":10,"y":10,"width":40,"height":30})).unwrap()["layer"].as_u64().unwrap());
    let mut image = Document::new("Image", photocraft_geom::Size::new(20, 10), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut l = Layer::raster("Photo", image.pixel_format());
    l.surface_mut().unwrap().fill_rect(image.bounds(), &[0., 0., 1., 1.]);
    image.layers.push(l);
    let png = photocraft_io::export(&image, "png", &Default::default()).unwrap().bytes;
    let result = crate::file_cmds::place_bytes(&mut s, "blue.png", png.clone(), None, &json!({"smartObject":false})).unwrap();
    assert_eq!(result["frame"], id.0);
    assert_eq!(result["scale"], 300.0);
    let frame = s.active().unwrap().doc.layer(id).unwrap();
    let child = frame.children().unwrap().first().unwrap();
    let first = child.id;
    let LayerContent::Smart(sm) = &child.content else { panic!("a placed frame image must remain smart") };
    assert_eq!(sm.transform.m, [3., 0., 0., 3., 0., 10.]);
    crate::file_cmds::place_bytes(&mut s, "again.png", png, None, &json!({})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().children().unwrap().len(), 1);
    assert_ne!(s.active().unwrap().doc.layer(id).unwrap().children().unwrap()[0].id, first);
    assert!(s.undo());
    assert_eq!(s.active().unwrap().doc.layer(id).unwrap().children().unwrap()[0].id, first);
}

#[test]
fn frame_shapes_strokes_and_editability_survive_native_and_psd() {
    for depth in [8, 16, 32] {
        for shape in ["rectangle", "ellipse", "triangle", "polygon", "custom"] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width":64,"height":48,"depth":depth,"background":"transparent"})).unwrap();
            let p = json!({"x":12,"y":10,"width":32,"height":24,"shape":shape,"sides":6,
            "path":{"subpaths":[{"closed":true,"knots":[[0,0],[100,0],[50,100]]}]},"stroke":{"color":"#ff0000","width":3,"align":"outside"}});
            let id = LayerId(s.execute("layer.new.frame", p).unwrap()["layer"].as_u64().unwrap());
            let original = &s.active().unwrap().doc;
            let rendered = photocraft_compose::render(original, original.bounds());
            assert!(rendered.px.iter().any(|p| p[0] > 0.9 && p[3] > 0.5), "empty {shape} stroke at {depth}");
            for name in ["pcraft", "psd"] {
                let bytes = photocraft_io::export(original, name, &Default::default()).unwrap().bytes;
                let restored = photocraft_io::import(name, &bytes).unwrap().document;
                let l = restored
                    .walk()
                    .into_iter()
                    .find(|(_, _, l)| l.frame.is_some())
                    .unwrap_or_else(|| {
                        panic!(
                            "no frame in {name} {shape} {depth}; {:?}",
                            restored.walk().iter().map(|(_, _, l)| (&l.name, l.psd_blocks.iter().map(|(k, _)| k).collect::<Vec<_>>())).collect::<Vec<_>>()
                        )
                    })
                    .2;
                assert_eq!(l.frame, original.layer(id).unwrap().frame, "{name} {shape} {depth}");
                assert!(l.children().unwrap().is_empty(), "export-only vector border is not frame content");
                let after = photocraft_compose::render(&restored, restored.bounds());
                let error = rendered.px.iter().zip(after.px.iter()).flat_map(|(a, b)| a.iter().zip(b).map(|(a, b)| (a - b).abs())).fold(0.0f32, f32::max);
                assert!(error < 0.025, "{name} {shape} {depth}: {error}");
            }
        }
    }
}

#[test]
fn frame_commands_reject_invalid_geometry_and_locks_without_history_changes() {
    let (mut s, _, _) = session();
    let id = s.execute("layer.new.frame", json!({"x":10,"y":10,"width":20,"height":20})).unwrap()["layer"].as_u64().unwrap();
    let before = s.active().unwrap().doc.clone();
    let history = s.active().unwrap().history.entries();
    for p in [
        json!({"layer":id,"transform":[0,0,0,0,1,1]}),
        json!({"layer":id,"transform":[1e200,0,0,1,0,0]}),
        json!({"layer":id,"stroke":{"width":1e8}}),
        json!({"layer":id,"transform":"bad"}),
    ] {
        assert!(s.execute("layer.frame.set", p).is_err());
        assert_eq!(s.active().unwrap().doc.layer(LayerId(id)).unwrap(), before.layer(LayerId(id)).unwrap());
        assert_eq!(s.active().unwrap().history.entries(), history);
    }
    s.execute("layer.setProps", json!({"layer":id,"locks":{"position":true}})).unwrap();
    assert!(s.execute("layer.frame.set", json!({"layer":id,"transform":[30,0,0,30,10,10]})).is_err());
    assert!(s.execute("layer.frame.fit", json!({"layer":id})).is_err());
    assert!(s.execute("layer.frame.setContent", json!({"layer":id,"content":id})).is_err());
}

#[test]
fn converting_a_shape_uses_its_outline_and_leaves_an_empty_frame() {
    let (mut s, _, _) = session();
    let r = s.execute("shape.create", json!({"kind":"ellipse","rect":[10,15,40,30],"fill":"#123456"})).unwrap();
    let id = r["layer"].as_u64().unwrap();
    s.execute("layer.frame.convert", json!({"layer":id,"name":"Portrait","width":20,"height":25})).unwrap();
    let l = s.active().unwrap().doc.layer(LayerId(id)).unwrap();
    assert_eq!(l.name, "Portrait");
    assert!(l.children().unwrap().is_empty());
    assert_eq!(l.frame.as_ref().unwrap().transform.m, [20., 0., 0., 25., 10., 15.]);
    assert_eq!(TestMask(l.vector_mask.as_ref().unwrap().path.clone()).value(10, 15), 0.0);
    assert!(s.undo());
    assert!(matches!(s.active().unwrap().doc.layer(LayerId(id)).unwrap().content, LayerContent::Shape(_)));
}

#[test]
fn fitting_transparent_images_uses_their_canvas_and_is_idempotent() {
    let (mut s, _, _) = session();
    let frame = s.execute("layer.new.frame", json!({"x":10,"y":10,"width":40,"height":30})).unwrap()["layer"].as_u64().unwrap();
    let mut image = Document::new("Transparent", photocraft_geom::Size::new(20, 10), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut l = Layer::raster("Mark", image.pixel_format());
    l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 4, 4), &[0., 0., 1., 1.]);
    image.layers.push(l);
    let bytes = photocraft_io::export(&image, "png", &Default::default()).unwrap().bytes;
    crate::file_cmds::place_bytes(&mut s, "transparent.png", bytes, None, &json!({})).unwrap();
    let child = s.active().unwrap().doc.layer(LayerId(frame)).unwrap().children().unwrap()[0].id;
    let pose = |s: &Session| {
        let LayerContent::Smart(sm) = &s.active().unwrap().doc.layer(child).unwrap().content else { panic!("smart") };
        sm.transform
    };
    let before = pose(&s);
    assert_eq!(before.m, [3., 0., 0., 3., 0., 10.]);
    for _ in 0..3 {
        s.execute("layer.frame.fit", json!({"layer":frame})).unwrap();
        assert_eq!(pose(&s), before);
    }
    s.execute("layer.frame.fit", json!({"layer":frame,"mode":"frameToContent"})).unwrap();
    assert_eq!(s.active().unwrap().doc.layer(LayerId(frame)).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(0, 10, 60, 40));
    assert_eq!(pose(&s), before);
}

#[test]
fn explicit_frames_accept_layer_drops_when_the_dragged_image_is_active() {
    let (mut s, a, _) = session();
    let frame = s.execute("layer.new.frame", json!({"x":10,"y":10,"width":40,"height":30})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.select", json!({"layer":a.0})).unwrap();
    let before = crate::transform_cmds::transform_frame(&s.active().unwrap().doc, s.active().unwrap().doc.layer(a).unwrap());
    s.execute("layer.frame.setContent", json!({"layer":frame,"content":a.0})).unwrap();
    let child = s.active().unwrap().doc.layer(LayerId(frame)).unwrap().children().unwrap().first().unwrap();
    assert_eq!(child.id, a);
    assert!(matches!(child.content, LayerContent::Smart(_)));
    assert_eq!(crate::transform_cmds::transform_frame(&s.active().unwrap().doc, child), before, "a layer drop does not resize or recenter the image");
    s.execute("layer.select", json!({"layer":s.active().unwrap().doc.layers.first().unwrap().id.0})).unwrap();
    assert!(s.execute("layer.frame.set", json!({"layer":frame,"name":"Portrait"})).is_ok());
    let (history, journal) = (s.active().unwrap().history.entries(), s.journal.len());
    let info = s.execute("layer.frame.info", json!({"layer":frame})).unwrap();
    assert_eq!(info["name"], "Portrait");
    assert_eq!(info["frame"], json!([10, 10, 40, 30]));
    assert_eq!(info["content"], a.0);
    assert_eq!(s.active().unwrap().history.entries(), history);
    assert_eq!(s.journal.len(), journal);
}

#[test]
fn copying_an_image_into_a_frame_preserves_the_source_and_undoes_together() {
    let (mut s, a, _) = session();
    let frame = s.execute("layer.new.frame", json!({"x":10,"y":10,"width":40,"height":30})).unwrap()["layer"].as_u64().unwrap();
    let source = s.active().unwrap().doc.layer(a).unwrap().clone();
    let entries = s.active().unwrap().history.entries();
    let placed = s.execute("layer.frame.setContent", json!({"layer":frame,"content":a.0,"copy":true})).unwrap();
    assert_ne!(placed["content"], a.0);
    assert_eq!(s.active().unwrap().doc.layer(a).unwrap(), &source);
    let child = s.active().unwrap().doc.layer(LayerId(frame)).unwrap().children().unwrap().first().unwrap();
    assert_eq!(
        crate::transform_cmds::transform_frame(&s.active().unwrap().doc, child),
        crate::transform_cmds::transform_frame(&s.active().unwrap().doc, &source)
    );
    s.undo();
    assert_eq!(s.active().unwrap().history.entries(), entries);
    assert!(s.active().unwrap().doc.layer(LayerId(frame)).unwrap().children().unwrap().is_empty());
    assert_eq!(s.active().unwrap().doc.layer(a).unwrap(), &source);
}

#[test]
fn empty_frame_placeholders_do_not_schedule_canvas_sized_composites() {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":6000,"height":4000})).unwrap();
    let frame = LayerId(s.execute("layer.new.frame", json!({"x":100,"y":100,"width":5800,"height":3800})).unwrap()["layer"].as_u64().unwrap());
    let st = s.active().unwrap();
    let layer = st.doc.layer(frame).unwrap();
    assert_eq!(crate::transform_cmds::transform_frame(&st.doc, layer), Rect::new(100, 100, 5900, 3900), "the editable container still has its full geometry");
    assert_eq!(photocraft_compose::layer_bounds(layer, st.doc.bounds()), Rect::EMPTY);
    assert_eq!(photocraft_compose::composite_bounds(layer, st.doc.bounds()), Some(Rect::EMPTY));
    s.execute("layer.frame.set", json!({"layer":frame.0,"stroke":{"color":"#ff0000","width":2}})).unwrap();
    let st = s.active().unwrap();
    assert!(!photocraft_compose::layer_bounds(st.doc.layer(frame).unwrap(), st.doc.bounds()).is_empty());
}

#[test]
fn new_layers_and_outside_placement_do_not_enter_a_frames_image_slot() {
    let (mut s, image, _) = session();
    let frame = LayerId(s.execute("layer.new.frame", json!({"x":10,"y":10,"width":40,"height":30,"layer":image.0})).unwrap()["layer"].as_u64().unwrap());
    s.execute("layer.select", json!({"layer":image.0})).unwrap();
    let new = s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap();
    assert_eq!(s.active().unwrap().doc.path_of(LayerId(new)).unwrap().len(), 1);
    s.execute("layer.select", json!({"layer":image.0})).unwrap();
    let bytes = photocraft_io::export(&s.active().unwrap().doc, "png", &Default::default()).unwrap().bytes;
    let placed = crate::file_cmds::place_bytes(&mut s, "outside.png", bytes, None, &json!({"frame":false})).unwrap();
    assert_eq!(placed["frame"], Value::Null);
    let st = s.active().unwrap();
    assert_eq!(st.doc.path_of(LayerId(placed["layer"].as_u64().unwrap())).unwrap().len(), 1);
    assert_eq!(st.doc.layer(frame).unwrap().children().unwrap().len(), 1);
    assert_eq!(st.doc.layer(frame).unwrap().children().unwrap()[0].id, image);
    let mut doc = (*st.doc).clone();
    let below = Layer::raster("Below", doc.pixel_format());
    let id = doc.insert_below(Some(image), below);
    assert_eq!(doc.path_of(id).unwrap().len(), 1);
    assert_eq!(doc.layer(frame).unwrap().children().unwrap().len(), 1);
}

fn blue_png(w: u32, h: u32) -> Vec<u8> {
    let mut image = Document::new("Image", photocraft_geom::Size::new(w, h), photocraft_color::ColorMode::Rgb, photocraft_color::SampleType::U8);
    let mut l = Layer::raster("Photo", image.pixel_format());
    l.surface_mut().unwrap().fill_rect(image.bounds(), &[0., 0., 1., 1.]);
    image.layers.push(l);
    photocraft_io::export(&image, "png", &Default::default()).unwrap().bytes
}

#[test]
fn frame_placement_reports_document_bounds() {
    let (mut s, _, _) = session();
    s.execute("layer.new.frame", json!({"x":10,"y":10,"width":40,"height":30})).unwrap();
    // 20×10 covers 40×30 at 3×: 60×30, centred on the frame, so 10 px overhangs each side.
    let r = crate::file_cmds::place_bytes(&mut s, "blue.png", blue_png(20, 10), None, &json!({})).unwrap();
    assert_eq!(r["bounds"], json!([0.0, 10.0, 60.0, 40.0]), "{r}");
}

#[test]
fn placing_into_a_huge_frame_does_not_resample_to_the_frame_size() {
    // Filling this frame scales the image to 300000×150000 px (45 Gpx). Placement used to
    // bicubic-resize it to that size, and the smart object render warped it unclipped: either
    // filled memory until the OS killed the process. The render now refuses it up front.
    let (mut s, _, _) = session();
    let id = LayerId(s.execute("layer.new.frame", json!({"x":0,"y":0,"width":200_000,"height":150_000})).unwrap()["layer"].as_u64().unwrap());
    let before = s.active().unwrap().history.entries();
    let err = crate::file_cmds::place_bytes(&mut s, "blue.png", blue_png(20, 10), None, &json!({})).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
    let st = s.active().unwrap();
    assert_eq!(st.history.entries(), before, "a failed place leaves no history");
    assert!(st.doc.layer(id).unwrap().children().unwrap().is_empty());
}

#[test]
fn fitting_content_to_a_huge_frame_is_refused_before_rendering() {
    let (mut s, image, _) = session();
    let id = s.execute("layer.new.frame", json!({"x":0,"y":0,"width":40,"height":30,"layer":image.0})).unwrap()["layer"].as_u64().unwrap();
    s.execute("layer.frame.set", json!({"layer":id,"transform":[200_000,0,0,150_000,0,0]})).unwrap();
    let before = s.active().unwrap().history.entries();
    let err = s.execute("layer.frame.fit", json!({"layer":id,"mode":"contentToFrame"})).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
    assert_eq!(s.active().unwrap().history.entries(), before);
}

#[test]
fn converting_drops_the_layers_own_psd_content_blocks() {
    let (mut s, _, _) = session();
    let id = s.execute("shape.create", json!({"kind":"rectangle","rect":[10,15,40,30],"fill":"#123456"})).unwrap()["layer"].as_u64().unwrap();
    s.edit("blocks", |doc, _| {
        let l = doc.layer_mut(LayerId(id)).unwrap();
        for k in [b"TySh", b"tySh", b"SoCo", b"vscg", b"vstk", b"lmfx", b"luni"] {
            l.psd_blocks.push((*k, std::sync::Arc::new(vec![0; 4])));
        }
        Ok(())
    })
    .unwrap();
    s.execute("layer.frame.convert", json!({"layer":id})).unwrap();
    let l = s.active().unwrap().doc.layer(LayerId(id)).unwrap();
    let keys: Vec<_> = l.psd_blocks.iter().map(|(k, _)| k).collect();
    assert_eq!(keys, [b"luni"], "only blocks unrelated to the shape survive");
    assert!(l.fill_cache.is_none());
}

#[test]
fn a_frames_image_cannot_be_framed_again() {
    let (mut s, image, _) = session();
    let first = LayerId(s.execute("layer.new.frame", json!({"x":0,"y":0,"width":40,"height":30,"layer":image.0})).unwrap()["layer"].as_u64().unwrap());
    let before = s.active().unwrap().history.entries();
    assert!(s.execute("layer.new.frame", json!({"x":20,"y":20,"width":40,"height":30,"layer":image.0})).is_err());
    let st = s.active().unwrap();
    assert_eq!(st.doc.layer(first).unwrap().children().unwrap()[0].id, image, "the image stays in its frame");
    assert_eq!(st.history.entries(), before);
}

#[test]
fn moving_a_frame_past_its_limits_keeps_a_plain_masked_group() {
    // An invalid frame would make the saved file fail to load; the clip survives as the mask.
    let (mut s, _, _) = session();
    let id = LayerId(s.execute("layer.new.frame", json!({"x":10,"y":10,"width":40,"height":30})).unwrap()["layer"].as_u64().unwrap());
    let doc = (*s.active().unwrap().doc).clone();
    let mut l = doc.layer(id).unwrap().clone();
    crate::vector_cmds::translate_vectors(&doc, &mut l, 700_000.0, 0.0);
    assert!(l.frame.is_none());
    assert!(l.vector_mask.is_some());
    let mut l = doc.layer(id).unwrap().clone();
    crate::canvas_geom::transform_layer_geometry(&mut l, &photocraft_geom::Affine::scale(10_000.0), true, 72.0);
    assert!(l.frame.is_none());
    assert!(l.vector_mask.is_some());
}

#[test]
fn linked_layers_keep_their_links_beside_a_frame_in_psd() {
    // The frame's extra PSD records ("Frame Image" group, divider, stroke) used to shift every
    // later Link Layers entry onto the wrong record.
    let (mut s, a, b) = session();
    s.execute("layer.new.frame", json!({"x":0,"y":0,"width":20,"height":20,"layer":a.0,"stroke":{"color":"#ff0000","width":2}})).unwrap();
    let doc = &mut (*s.active().unwrap().doc).clone();
    let top = Layer::raster("C", doc.pixel_format());
    let c = top.id;
    doc.layers.push(top);
    for id in [b, c] {
        doc.layer_mut(id).unwrap().link_group = Some(5);
    }
    let bytes = photocraft_io::export(doc, "psd", &Default::default()).unwrap().bytes;
    let restored = photocraft_io::import("psd", &bytes).unwrap().document;
    let links: Vec<_> = restored.walk().iter().map(|(_, _, l)| (l.name.clone(), l.link_group)).filter(|(_, g)| g.is_some()).collect();
    let names: Vec<_> = links.iter().map(|(n, _)| n.as_str()).collect();
    assert_eq!(names, ["B", "C"], "{links:?}");
    assert_eq!(links[0].1, links[1].1);
}

#[test]
fn a_pcraft_with_unusable_frame_metadata_still_opens() {
    let (mut s, _, _) = session();
    let id = LayerId(s.execute("layer.new.frame", json!({"x":10,"y":10,"width":40,"height":30})).unwrap()["layer"].as_u64().unwrap());
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.layer_mut(id).unwrap().frame.as_mut().unwrap().transform.m[4] = 900_000.0;
    let bytes = photocraft_io::export(&doc, "pcraft", &Default::default()).unwrap().bytes;
    let restored = photocraft_io::import("pcraft", &bytes).unwrap().document;
    let l = restored.layer(id).unwrap();
    assert!(l.frame.is_none());
    assert!(l.vector_mask.is_some(), "the clip survives as the group's vector mask");
}

/// A frame at (40,28) 32×32 holding session layer `A` (a 20×20 patch at (4,4)), with `setup` run on
/// the image before it is fitted. Returns the session, the frame and the image ids.
fn fitted(setup: impl FnOnce(&mut Layer)) -> (Session, LayerId, LayerId) {
    let (mut s, a, _) = session();
    let frame = LayerId(s.execute("layer.new.frame", json!({"x":40,"y":28,"width":32,"height":32,"layer":a.0})).unwrap()["layer"].as_u64().unwrap());
    let child = s.active().unwrap().doc.layer(frame).unwrap().children().unwrap()[0].id;
    s.edit("setup", |doc, _| {
        setup(doc.layer_mut(child).unwrap());
        Ok(())
    })
    .unwrap();
    s.execute("layer.frame.fit", json!({"layer":frame.0,"mode":"contentToFrame"})).unwrap();
    (s, frame, child)
}

fn near(a: Rect, b: Rect) -> bool {
    (a.x0 - b.x0).abs() <= 1 && (a.y0 - b.y0).abs() <= 1 && (a.x1 - b.x1).abs() <= 1 && (a.y1 - b.y1).abs() <= 1
}

fn image_bounds(s: &Session, child: LayerId) -> Rect {
    let LayerContent::Smart(sm) = &s.active().unwrap().doc.layer(child).unwrap().content else { panic!("smart") };
    sm.cache.as_ref().unwrap().content_bounds()
}

fn patch_mask(bounds: Rect) -> photocraft_doc::LayerMask {
    let mut m = photocraft_doc::LayerMask::hide_all();
    m.surface.fill_rect(bounds, &[1.0]);
    m
}

#[test]
fn fitting_content_moves_a_linked_raster_mask_with_the_image() {
    let (mut s, frame, child) = fitted(|l| l.mask = Some(patch_mask(Rect::new(4, 4, 24, 24))));
    let image = image_bounds(&s, child);
    assert!(image.x0 >= 30, "the image moved into the frame: {image:?}");
    let mask = s.active().unwrap().doc.layer(child).unwrap().mask.as_ref().unwrap().surface.content_bounds();
    assert!(near(mask, image), "the mask {mask:?} follows the image {image:?}");
    assert!(s.undo());
    let l = s.active().unwrap().doc.layer(child).unwrap();
    assert_eq!(l.mask.as_ref().unwrap().surface.content_bounds(), Rect::new(4, 4, 24, 24));
    assert!(s.active().unwrap().doc.layer(frame).is_some());
}

#[test]
fn fitting_content_moves_a_linked_vector_mask_with_the_image() {
    let outline = photocraft_doc::Path::new(vec![photocraft_doc::Subpath::polygon(&[(4., 4.), (24., 4.), (24., 24.), (4., 24.)])]);
    let (mut s, _, child) = fitted(|l| l.vector_mask = Some(photocraft_doc::VectorMask::new(outline.clone())));
    let image = image_bounds(&s, child);
    let (x0, y0, x1, y1) = s.active().unwrap().doc.layer(child).unwrap().vector_mask.as_ref().unwrap().path.control_bounds().unwrap();
    assert!(near(Rect::new(x0.round() as i32, y0.round() as i32, x1.round() as i32, y1.round() as i32), image), "{x0} {y0} {x1} {y1} vs {image:?}");
    assert!(s.undo());
    let vm = s.active().unwrap().doc.layer(child).unwrap().vector_mask.clone().unwrap();
    assert_eq!(vm.path, outline);
}

#[test]
fn fitting_content_moves_the_smart_filter_mask_with_the_image() {
    let (mut s, _, child) = fitted(|l| {
        if let LayerContent::Smart(sm) = &mut l.content {
            sm.filter_mask = Some(patch_mask(Rect::new(4, 4, 24, 24)));
        }
    });
    let image = image_bounds(&s, child);
    let LayerContent::Smart(sm) = &s.active().unwrap().doc.layer(child).unwrap().content else { panic!("smart") };
    let mask = sm.filter_mask.as_ref().unwrap().surface.content_bounds();
    assert!(near(mask, image), "the filter mask {mask:?} follows the image {image:?}");
    assert!(s.undo());
    let LayerContent::Smart(sm) = &s.active().unwrap().doc.layer(child).unwrap().content else { panic!("smart") };
    assert_eq!(sm.filter_mask.as_ref().unwrap().surface.content_bounds(), Rect::new(4, 4, 24, 24));
}
