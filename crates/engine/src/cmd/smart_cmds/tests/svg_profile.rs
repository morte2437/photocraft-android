use super::*;
use photocraft_cms::Builtin;

const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#808080"/></svg>"##;

#[test]
fn svg_builtin_srgb_keeps_direct_render_at_each_depth() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="4" height="8" fill="#336699"/><rect x="4" width="4" height="4" fill="#e65c1a" fill-opacity="0.5"/></svg>"##;
    for depth in DEPTHS {
        let s = session(depth);
        let mut doc = (*s.active().unwrap().doc).clone();
        for explicit_profile in [false, true] {
            doc.icc_profile = explicit_profile.then(|| Builtin::Srgb.profile().to_bytes());
            let expected = svg_source_image(svg, doc.pixel_format(), 1.0).unwrap();
            let actual = svg_source_image_in(svg, &doc, 1.0).unwrap();
            assert_eq!(actual.bounds, expected.bounds);
            assert_eq!(actual.surface, expected.surface, "sRGB at {depth}-bit, explicit={explicit_profile}");
        }
    }
}

#[test]
fn svg_smart_rerender_checks_embedded_raster_dimensions() {
    let mut header = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    header.extend_from_slice(&32768u32.to_be_bytes());
    header.extend_from_slice(&32768u32.to_be_bytes());
    header.extend_from_slice(&[8, 6, 0, 0, 0]);
    let data = header.iter().map(|byte| format!("%{byte:02X}")).collect::<String>();
    let svg =
        format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\"><image width=\"1\" height=\"1\" href=\"data:image/png,{data}\"/></svg>");
    assert!(svg_source_image(svg.as_bytes(), PixelFormat::RGBA32F, 1.0).unwrap_err().to_string().contains("decoded pixel budget"));
    let s = session(8);
    let smart = SmartObject::new(SmartSource::Embedded { file_name: "Clipboard.svg".into(), bytes: Arc::new(svg.into_bytes()) }, Affine::IDENTITY, None);
    assert!(render(&s.active().unwrap().doc, &smart).unwrap_err().to_string().contains("decoded pixel budget"));
}

/// An SVG with many shapes opens as editable layers for its contents and Stack Mode, as
/// File › Open does, under the SVG import's own retained-pixel budget.
#[test]
fn svg_smart_contents_and_stack_open_many_shapes_as_layers() {
    let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"256\" height=\"256\">{}</svg>", "<rect width=\"256\" height=\"256\"/>".repeat(400));
    assert_eq!(decode_source("Clipboard.svg", svg.as_bytes()).unwrap().layers.len(), 400);
    assert!(stack_image("Clipboard.svg", svg.as_bytes(), PixelFormat::RGBA32F, photocraft_doc::StackMode::Mean).is_ok());
}

/// Stack Mode renders a float frame per layer: an SVG whose shapes would need more than 1 GiB of
/// them (40 full-HD shapes, 1.3 GB) is refused before any is rendered, while its contents open.
#[test]
fn svg_stack_mode_refuses_frames_past_its_budget() {
    let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1920\" height=\"1080\">{}</svg>", "<rect width=\"10\" height=\"10\"/>".repeat(40));
    assert_eq!(decode_source("Many.svg", svg.as_bytes()).unwrap().layers.len(), 40);
    let error = stack_image("Many.svg", svg.as_bytes(), PixelFormat::RGBA32F, photocraft_doc::StackMode::Mean).unwrap_err();
    assert!(error.to_string().contains("Stack Mode"), "{error}");
}

/// An ordinary layout (a 1920 × 1080 SVG with 12 shapes) placed as a smart object: Replace
/// Contents with another like it and Convert to Layers both work through their commands.
#[test]
fn ordinary_layered_svg_contents_replace_and_convert_to_layers() {
    let rects: String = (0..12).map(|i| format!("<rect x=\"{}\" width=\"100\" height=\"100\" fill=\"#336699\"/>", i * 150)).collect();
    let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1920\" height=\"1080\">{rects}</svg>");
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 1920, "height": 1080})).unwrap();
    crate::file_cmds::place_bytes(&mut s, "Layout.svg", svg.clone().into_bytes(), None, &json!({"smartObject": true})).unwrap();
    let dir = std::env::temp_dir().join(format!("pcraft-svg-layout-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("Layout 2.svg");
    std::fs::write(&path, svg.replace("#336699", "#993366")).unwrap();
    s.execute("layer.smartObjects.replaceContents", json!({"path": path.to_str().unwrap()})).unwrap();
    std::fs::remove_dir_all(&dir).ok();
    let r = s.execute("layer.smartObjects.convertToLayers", json!({})).unwrap();
    let doc = &s.active().unwrap().doc;
    let group = doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap();
    assert_eq!(group.children().map(<[Layer]>::len), Some(12), "the 12 shapes become 12 layers");
}

/// Convert to Layers on an SVG scaled to fill a 4K document: four full-canvas shapes (33 MP of
/// shape caches, as many as four full-canvas layers anywhere) unpack.
#[test]
fn convert_to_layers_unpacks_full_canvas_shapes_in_a_4k_document() {
    let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"384\" height=\"216\">{}</svg>", "<rect width=\"384\" height=\"216\"/>".repeat(4));
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 3840, "height": 2160})).unwrap();
    let placed = crate::file_cmds::place_bytes(&mut s, "Full.svg", svg.into_bytes(), None, &json!({"smartObject": true})).unwrap();
    let id = LayerId(placed["layer"].as_u64().unwrap());
    let LayerContent::Smart(sm) = &mut Arc::make_mut(&mut s.active_mut().unwrap().doc).layer_mut(id).unwrap().content else { panic!("smart") };
    sm.transform = Affine { m: [10.0, 0.0, 0.0, 10.0, 0.0, 0.0] };
    s.execute("layer.smartObjects.convertToLayers", json!({"layer": id.0})).unwrap();
}

/// An SVG with an embedded image the bounded resolver can't read (an unknown MIME type) places
/// and renders the rest of the drawing, as it did before the bounded render.
#[test]
fn svg_with_an_unreadable_embedded_image_still_places() {
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"><image width=\"4\" height=\"4\" href=\"data:image/x-foo,abc\"/><rect x=\"4\" width=\"4\" height=\"8\" fill=\"#ff0000\"/></svg>";
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
    let placed = crate::file_cmds::place_bytes(&mut s, "Logo.svg", svg.as_bytes().to_vec(), None, &json!({"smartObject": true})).unwrap();
    let doc = &s.active().unwrap().doc;
    let LayerContent::Smart(sm) = &doc.layer(LayerId(placed["layer"].as_u64().unwrap())).unwrap().content else { panic!("smart") };
    assert_eq!(sm.cache.as_ref().unwrap().rgba(6, 4), [1.0, 0.0, 0.0, 1.0]);
}

/// Warp setup needs only the source frame, found with the bounded parse and the exact canvas
/// limit; Stack Mode and Edit Contents build the SVG's layers.
#[test]
fn svg_warp_frame_uses_bounded_bounds() {
    // 256 × 256 × 400 shapes = 26,214,400 pixels of layer caches, over the 24 MP budget.
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"256\" height=\"256\">{}</svg>",
        "<rect width=\"256\" height=\"256\" fill=\"#336699\"/>".repeat(400)
    );
    let s = session(8);
    let doc = s.active().unwrap().doc.clone();
    let mut smart =
        SmartObject::new(SmartSource::Embedded { file_name: "Clipboard.svg".into(), bytes: Arc::new(svg.clone().into_bytes()) }, Affine::IDENTITY, None);
    let bounds = [0.0, 0.0, 256.0, 256.0];
    assert_eq!(source_box(&doc, &smart), Some(bounds));
    smart.warp = Some(photocraft_geom::warp::Warp::custom(photocraft_geom::warp::BezierMesh::identity(bounds, 1, 1), bounds));
    assert_eq!(source_box(&doc, &smart), Some(bounds), "a warped SVG keeps its frame");
    let warped = render(&doc, &smart).unwrap().unwrap();
    assert!((warped.pixel(10, 10)[2] - 0x99 as f32 / 255.0).abs() < 0.01, "the bounded warp render draws the SVG");
    assert_eq!(decode_source("Clipboard.svg", svg.as_bytes()).unwrap().layers.len(), 400);
    smart.warp = None;
    smart.stack_mode = Some(photocraft_doc::StackMode::Mean);
    assert_eq!(source_box(&doc, &smart), Some(bounds), "Stack Mode gets the layered import's frame");
    // The canvas limit is exact: 24,000,000 pixels gets a frame, one more row does not.
    let canvas = |h: u32| format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4000\" height=\"{h}\"><rect width=\"8\" height=\"8\"/></svg>");
    let frame = |svg: String| {
        source_box(&doc, &SmartObject::new(SmartSource::Embedded { file_name: "Big.svg".into(), bytes: Arc::new(svg.into_bytes()) }, Affine::IDENTITY, None))
    };
    assert_eq!(frame(canvas(6000)), Some([0.0, 0.0, 4000.0, 6000.0]));
    assert_eq!(frame(canvas(6001)), None);
}

#[test]
fn svg_smart_render_cache_tracks_destination_profile_and_transform() {
    for depth in DEPTHS {
        let s = session(depth);
        let srgb = (*s.active().unwrap().doc).clone();
        let mut linear = srgb.clone();
        linear.icc_profile = Some(Builtin::LinearSrgb.profile().to_bytes());
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "clipboard.svg".into(), bytes: Arc::new(SVG.to_vec()) }, Affine::IDENTITY, None);
        let original = render(&srgb, &smart).unwrap().unwrap();
        let corrected = render(&linear, &smart).unwrap().unwrap();
        assert!((original.pixel(3, 3)[0] - 128.0 / 255.0).abs() < 0.01);
        assert!((corrected.pixel(3, 3)[0] - 0.216).abs() < 0.01, "linear profile at {depth}-bit");
        assert_eq!(render(&srgb, &smart).unwrap().unwrap(), original, "profile-specific placed cache");
        smart.transform = Affine { m: [2.0, 0.0, 0.0, 2.0, 4.0, 2.0] };
        let scaled = render(&linear, &smart).unwrap().unwrap();
        assert!((scaled.pixel(10, 8)[0] - corrected.pixel(3, 3)[0]).abs() < 0.01, "scale must keep corrected colours");
        assert_eq!(scaled.format(), linear.pixel_format());
        let bounds = [0.0, 0.0, 8.0, 8.0];
        smart.warp = Some(photocraft_geom::warp::Warp::custom(photocraft_geom::warp::BezierMesh::identity(bounds, 1, 1), bounds));
        let warped = render(&linear, &smart).unwrap().unwrap();
        assert!((warped.pixel(10, 8)[0] - corrected.pixel(3, 3)[0]).abs() < 0.01, "warp must keep corrected colours");
        linear.layers = vec![Layer::new("Saved SVG", LayerContent::Smart(smart.clone()))];
        let saved = encode_source(&linear).unwrap();
        let reopened = decode_source("saved.pcraft", &saved).unwrap();
        let LayerContent::Smart(restored) = &reopened.layers[0].content else { panic!("smart object lost on save") };
        assert_eq!(restored.source, smart.source);
        assert_eq!(restored.transform, smart.transform);
        let rerendered = render(&reopened, restored).unwrap().unwrap();
        assert!((rerendered.pixel(10, 8)[0] - corrected.pixel(3, 3)[0]).abs() < 0.01, "saved source keeps ICC and warp");
    }
}
