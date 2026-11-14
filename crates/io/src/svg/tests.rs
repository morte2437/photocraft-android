use super::*;

#[cfg(not(target_arch = "wasm32"))]
#[test]
/// Require shallow, deep and nested private SVG probes to reuse the single native parser worker.
fn private_svg_parsing_uses_the_same_worker_at_every_depth() {
    for depth in [2, 200] {
        parse_at_depth(depth, false, |isolated| {
            assert!(isolated);
            assert!(SVG_WORKER.with(std::cell::Cell::get));
            assert_eq!(std::thread::current().name(), Some("photocraft-svg"));
            let worker = std::thread::current().id();
            parse_at_depth(200, isolated, |nested| {
                assert!(nested);
                assert_eq!(std::thread::current().id(), worker);
                Ok(())
            })
            .map_err(IoError::from)
        })
        .unwrap();
    }
}

#[cfg(not(target_arch = "wasm32"))]
#[test]
/// Keep invalid embedded images optional while recording failed isolation even for non-strict imports.
fn non_strict_embedded_svg_records_isolation_failure_but_omits_invalid_images() {
    /// Inject a worker failure before font resolution to exercise isolation-error propagation.
    fn panic_fonts() -> Arc<usvg::fontdb::Database> {
        panic!("injected worker failure")
    }
    let budget = Arc::new(Mutex::new(ImageBudget {
        pixels: MAX_RASTER_PIXELS,
        xml_bytes: MAX_BOUNDED_SVG_BYTES,
        svg_images: 0,
        svg_chain: 0,
        draw_nodes: 0,
        repeatable_raster: false,
        repeatable_svg: false,
        fontless_text: false,
        strict: false,
        error: None,
        resolver: Arc::new(craft_svg::tree::ImageResolver::new(tree_limits(MAX_XML_DEPTH, MAX_RASTER_PIXELS))),
    }));
    let invalid = image_svg(&embedded_image("image/svg+xml", b"<not-svg/>"));
    parse_image_bounded_with(invalid.as_bytes(), budget.clone(), MAX_XML_DEPTH, inter_only, false).unwrap();
    assert!(budget.lock().unwrap().error.is_none(), "ordinary invalid images are still omitted");
    let svg = image_svg("<text>x</text>");
    let error = parse_image_bounded_with(svg.as_bytes(), budget.clone(), 2, panic_fonts, false).unwrap_err();
    assert!(error.to_string().contains("bounded SVG worker failed"));
    assert_eq!(budget.lock().unwrap().error.as_deref(), Some(error.to_string().as_str()));
}

fn embedded_image(mime: &str, bytes: &[u8]) -> String {
    let data = bytes.iter().map(|byte| format!("%{byte:02X}")).collect::<String>();
    format!("<image width=\"1\" height=\"1\" href=\"data:{mime},{data}\"/>")
}

fn image_svg(images: &str) -> String {
    format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\">{images}</svg>")
}

fn png_dimensions(width: u32, height: u32) -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".to_vec();
    bytes.extend_from_slice(&width.to_be_bytes());
    bytes.extend_from_slice(&height.to_be_bytes());
    bytes.extend_from_slice(&[8, 6, 0, 0, 0]);
    bytes
}

/// Source-image decoding and canvas rendering are separate work components.
#[test]
fn preview_work_counts_large_compressed_rasters_on_tiny_canvases() {
    let image = photocraft_codecs::Image::from_u8(64, 64, photocraft_codecs::ChannelLayout::Rgb, vec![255; 64 * 64 * 3]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let svg = image_svg(&embedded_image("image/png", &png));
    let (imported, work) = import_svg_bounded_with_work("work", svg.as_bytes(), 8192).unwrap();
    assert_eq!(imported.document.size, Size::new(1, 1));
    assert!(work >= 4097, "a one-pixel canvas must pay for its 4096-pixel decoded raster: {work}");
    assert!(import_svg_bounded_with_work("work", svg.as_bytes(), 4096).is_err(), "decoder work cannot leave the canvas unpaid");
    let (same, exact_work) = import_svg_bounded_with_work("work", svg.as_bytes(), work).unwrap();
    assert_eq!(exact_work, work);
    assert_eq!(same.document.size, imported.document.size);
}

/// A full-size input and output may each use the existing 24 MP component allowance.
#[test]
fn preview_work_can_pay_for_both_a_24mp_image_and_a_24mp_canvas() {
    let image = photocraft_codecs::Image::from_u8(6000, 4000, photocraft_codecs::ChannelLayout::Rgb, vec![255; 6000 * 4000 * 3]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let data = png.iter().map(|byte| format!("%{byte:02X}")).collect::<String>();
    let svg =
        format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="6000" height="4000"><image width="6000" height="4000" href="data:image/png,{data}"/></svg>"#);
    assert!(import_svg_bounded_with_work("work", svg.as_bytes(), 48_000_000 - 1).is_err());
    let (imported, work) = import_svg_bounded_with_work("work", svg.as_bytes(), 48_000_000).unwrap();
    assert_eq!(work, 48_000_000);
    assert_eq!(imported.document.size, Size::new(6000, 4000));
    assert!(!imported.document.layers.is_empty(), "the legal component limits must still retain the display");
}

#[test]
fn preview_work_plain_paths_and_artboard_clip_do_not_repeat_photo_decode() {
    let image = photocraft_codecs::Image::from_u8(6000, 4000, photocraft_codecs::ChannelLayout::Rgb, vec![255; 6000 * 4000 * 3]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let data = png.iter().map(|byte| format!("%{byte:02X}")).collect::<String>();
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="6000" height="4000"><defs><clipPath id="page"><path d="M0 0H6000V4000H0Z"/></clipPath></defs><g clip-path="url(#page)"><image width="6000" height="4000" href="data:image/png,{data}"/><path d="M0 0H4V4Z"/><path d="M5 5H9V9Z"/><path d="M10 10H14V14Z"/><path d="M15 15H19V19Z"/></g></svg>"#
    );
    let parsed = parse_bounded_work_with(svg.as_bytes(), MAX_RASTER_PIXELS, true, fonts).unwrap();
    assert!(!parsed.repeatable_raster && !parsed.repeatable_svg);
    let (imported, work) = import_svg_bounded_with_work("photo and paths", svg.as_bytes(), 48_000_000).unwrap();
    assert!(work <= 48_000_000, "ordinary photo composition stays within two full-size surfaces: {work}");
    assert_eq!(imported.document.size, Size::new(6000, 4000));
}

#[test]
fn preview_work_repeatable_raster_patterns_pay_for_repainting() {
    let image = photocraft_codecs::Image::from_u8(64, 64, photocraft_codecs::ChannelLayout::Rgb, vec![255; 64 * 64 * 3]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let data = png.iter().map(|byte| format!("%{byte:02X}")).collect::<String>();
    let svg = format!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><defs><pattern id="paint" width="1" height="1" patternUnits="userSpaceOnUse"><image width="1" height="1" href="data:image/png,{data}"/></pattern></defs><path fill="url(#paint)" d="M0 0H4V4H0Z"/><path fill="url(#paint)" d="M4 4H8V8H4Z"/></svg>"#
    );
    let parsed = parse_bounded_work_with(svg.as_bytes(), MAX_RASTER_PIXELS, true, fonts).unwrap();
    assert!(parsed.repeatable_raster);
    assert!(
        import_svg_bounded_with_work("pattern", svg.as_bytes(), 8192).is_err(),
        "repeated pattern painting cannot be charged as one decode plus a tiny canvas"
    );
}

#[test]
fn preview_work_use_images_charge_each_resolved_occurrence() {
    let image = photocraft_codecs::Image::from_u8(64, 64, photocraft_codecs::ChannelLayout::Rgb, vec![255; 64 * 64 * 3]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let data = png.iter().map(|byte| format!("%{byte:02X}")).collect::<String>();
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="4" height="4"><defs><image id="photo" width="1" height="1" href="data:image/png,{data}"/></defs><use href="#photo"/><use href="#photo" x="1"/><use href="#photo" x="2"/></svg>"##
    );
    let parsed = parse_bounded_work_with(svg.as_bytes(), MAX_RASTER_PIXELS, true, fonts).unwrap();
    assert!(parsed.decoded >= 3 * 4096, "each resolved use must reserve its own raster decode: {}", parsed.decoded);
    assert!(!parsed.repeatable_raster);
    assert!(import_svg_bounded_with_work("uses", svg.as_bytes(), 3 * 4096).is_err());
    let blurred = svg
        .replace("<defs>", "<defs><filter id=\"blur\"><feGaussianBlur stdDeviation=\"0.1\"/></filter>")
        .replace("<use href=", "<use filter=\"url(#blur)\" href=");
    let parsed = parse_bounded_work_with(blurred.as_bytes(), MAX_RASTER_PIXELS, true, fonts).unwrap();
    assert!(!parsed.repeatable_raster && !parsed.repeatable_svg, "blur without image resources must not multiply source decoding");
}

#[test]
fn embedded_raster_intrinsic_sizes_are_rejected_before_pixel_decode() {
    let png = png_dimensions(32768, 32768);
    let jpeg = vec![0xff, 0xd8, 0xff, 0xc0, 0, 17, 8, 0x80, 0, 0x80, 0, 3, 1, 0x11, 0, 2, 0x11, 0, 3, 0x11, 0];
    let mut gif = b"GIF89a\x01\0\x01\0\0\0\0\x2c\0\0\0\0".to_vec();
    gif.extend_from_slice(&[0, 0x80, 0, 0x80, 0]);
    let mut webp = b"RIFF\x16\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0".to_vec();
    webp.extend_from_slice(&[0xff, 0x7f, 0, 0xff, 0x7f, 0]);
    for (mime, bytes) in [("image/png", png), ("image/jpeg", jpeg), ("image/gif", gif), ("image/webp", webp)] {
        // These are dimension headers, not valid compressed pixel streams. The budget
        // error proves no downstream image decoder was asked to allocate their pixels.
        let svg = image_svg(&embedded_image(mime, &bytes));
        let error = import_svg_bounded("clipboard", svg.as_bytes(), MAX_RASTER_PIXELS).unwrap_err();
        assert!(error.to_string().contains("decoded pixel budget"), "{mime}: {error}");
        assert!(rasterize_bounded(svg.as_bytes(), 1.0, MAX_RASTER_PIXELS).unwrap_err().to_string().contains("decoded pixel budget"));
    }
}

#[test]
fn embedded_images_share_budget_and_nested_svg_cannot_hide_raster_bombs() {
    let image = embedded_image("image/png", &png_dimensions(2, 2));
    let aggregate = image_svg(&image.repeat(2));
    assert!(parse_bounded(aggregate.as_bytes(), 7).unwrap_err().to_string().contains("decoded pixel budget"));
    let oversized = image_svg(&embedded_image("image/png", &png_dimensions(u32::MAX, u32::MAX)));
    let nested = image_svg(&embedded_image("image/svg+xml", oversized.as_bytes()));
    assert!(parse_bounded(nested.as_bytes(), MAX_RASTER_PIXELS).unwrap_err().to_string().contains("decoded pixel budget"));
    // A PNG without a header can't be charged, so it is left out like usvg's own resolver
    // leaves out an image it can't read; the drawing still opens.
    let malformed = image_svg(&embedded_image("image/png", b"\x89PNG\r\n\x1a\n"));
    assert!(!parse_bounded(malformed.as_bytes(), MAX_RASTER_PIXELS).unwrap().tree.root().has_children());
    assert!(photocraft_codecs::probe_allocation_size(b"GIF89a\x01\0\x01\0\0\0\0\x21\xfe\xff").is_err(), "truncated extension has no frame");
}

#[test]
fn webp_container_cannot_hide_a_larger_lossy_frame() {
    let mut webp = b"RIFF\x28\0\0\0WEBPVP8X\x0a\0\0\0\0\0\0\0\0\0\0\0\0\0".to_vec();
    webp.extend_from_slice(b"VP8 \x0a\0\0\0\0\0\0\x9d\x01\x2a");
    webp.extend_from_slice(&8000u16.to_le_bytes());
    webp.extend_from_slice(&8000u16.to_le_bytes());
    assert_eq!(photocraft_codecs::probe_allocation_size(&webp).unwrap(), (8000, 8000), "shared probe sees the coded frame beyond the canvas");
    let svg = image_svg(&embedded_image("image/webp", &webp));
    assert!(parse_bounded(svg.as_bytes(), MAX_RASTER_PIXELS).unwrap_err().to_string().contains("decoded pixel budget"));
}

#[test]
fn bounded_images_keep_small_pixels_and_external_references_stay_disabled() {
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgba, vec![255, 0, 0, 255]).unwrap();
    let bytes = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let inner = image_svg(&embedded_image("image/png", &bytes));
    let nested = image_svg(&embedded_image("image/svg+xml", inner.as_bytes()));
    for svg in [&inner, &nested] {
        let rendered = rasterize_bounded(svg.as_bytes(), 1.0, 16).unwrap();
        assert_eq!(rendered.px, vec![[1.0, 0.0, 0.0, 1.0]]);
    }
    assert!(rasterize_bounded(nested.as_bytes(), 32.0, 16).unwrap_err().to_string().contains("rendered pixel budget"));
    let external = image_svg("<rect width=\"1\" height=\"1\" fill=\"blue\"/><image href=\"file:///never-read.png\" width=\"1\" height=\"1\"/>");
    assert_eq!(rasterize_bounded(external.as_bytes(), 1.0, 16).unwrap().px, vec![[0.0, 0.0, 1.0, 1.0]]);
}

#[test]
fn sniffing_accepts_svg_and_rejects_other_xml() {
    assert!(is_svg(b"<svg xmlns=\"http://www.w3.org/2000/svg\"/>"));
    assert!(is_svg("\u{feff}<?xml version=\"1.0\"?>\n<!-- hi -->\n<svg/>".as_bytes()));
    assert!(is_svg(b"<!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"x\"><svg/>"));
    assert!(!is_svg(b"<?xml version=\"1.0\"?><x:xmpmeta xmlns:x=\"adobe:ns:meta/\"/>"));
    assert!(!is_svg(b"\x89PNG\r\n"));
    assert!(!is_svg(b""));
}

/// Ordinary filters, however many and on the largest canvas, never count against the budget.
#[test]
fn ordinary_filters_cost_nothing_on_any_canvas() {
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{MAX_SIDE}\" height=\"{MAX_SIDE}\"><filter id=\"f\">\
         <feGaussianBlur stdDeviation=\"20\"/><feOffset dx=\"5\" dy=\"5\"/><feFlood flood-color=\"black\"/>\
         <feComposite operator=\"in\" in2=\"SourceAlpha\"/><feMerge><feMergeNode/><feMergeNode in=\"SourceGraphic\"/></feMerge>\
         <feMorphology radius=\"8\"/><feConvolveMatrix order=\"5\" kernelMatrix=\"{}\"/><feTurbulence baseFrequency=\"0.02\" numOctaves=\"10\"/>\
         </filter><rect width=\"{MAX_SIDE}\" height=\"{MAX_SIDE}\" filter=\"url(#f)\"/></svg>",
        vec!["1"; 25].join(" ")
    );
    let tree = parse(svg.as_bytes()).unwrap().tree;
    assert_eq!(filter_cost(tree.root(), tiny_skia::Transform::identity(), 0, &mut { MAX_FILTER_VISITS }), 0);
    assert!(check_filter_cost(&tree, 1.0).is_ok());
}

/// resvg draws a pattern's tile under the scale part of its transform only (`get_scale`): a
/// skewed patternTransform (determinant 1) whose full-affine bounds of a slim filter region are
/// 180 × 181 px still draws that region at 2400 × 180 px, with a morphology window as large,
/// about 2 × 10^11 reads. Never rendered here: only the estimate is checked.
#[test]
fn a_skewed_pattern_filter_counts_at_resvgs_tile_scale() {
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"100\" height=\"100\">\
        <filter id=\"f\"><feMorphology radius=\"1000\"/></filter>\
        <pattern id=\"p\" width=\"20\" height=\"20\" patternUnits=\"userSpaceOnUse\" patternTransform=\"matrix(0 -0.01 100 100 0 0)\">\
        <rect width=\"20\" height=\"1.5\" filter=\"url(#f)\"/></pattern>\
        <rect width=\"100\" height=\"100\" fill=\"url(#p)\"/></svg>";
    let tree = parse(svg.as_bytes()).unwrap().tree;
    // The tile resvg would allocate: 20 × 20 units at get_scale ≈ (100, 100).
    let (sx, sy) = tiny_skia::Transform::from_row(0.0, -0.01, 100.0, 100.0, 0.0, 0.0).get_scale();
    assert!((sx * 20.0).round() >= 2000.0 && (sy * 20.0).round() >= 1999.0, "{sx} {sy}");
    let cost = filter_cost(tree.root(), tiny_skia::Transform::identity(), 0, &mut { MAX_FILTER_VISITS });
    assert!(cost > 50 * MAX_FILTER_COST, "charged only {cost} reads");
    assert!(check_filter_cost(&tree, 1.0).is_err());
}

#[test]
fn quadratics_become_cubics_and_closing_merges_the_start_knot() {
    let mut pb = tiny_skia::PathBuilder::new();
    pb.move_to(0.0, 0.0);
    pb.quad_to(5.0, 10.0, 10.0, 0.0);
    pb.line_to(0.0, 0.0);
    pb.close();
    pb.move_to(20.0, 20.0);
    pb.line_to(30.0, 20.0);
    let path = pb.finish().unwrap();
    let sp = knots(&path);
    assert_eq!(sp.len(), 2);
    assert!(sp[0].closed && !sp[1].closed);
    // The explicit line back to the start was merged into the first knot.
    assert_eq!(sp[0].knots.len(), 2);
    let (a, b) = (&sp[0].knots[0], &sp[0].knots[1]);
    assert!((a.out_ctrl.x - 10.0 / 3.0).abs() < 1e-9 && (a.out_ctrl.y - 20.0 / 3.0).abs() < 1e-9);
    assert!((b.in_ctrl.x - 20.0 / 3.0).abs() < 1e-9 && (b.in_ctrl.y - 20.0 / 3.0).abs() < 1e-9);
    assert_eq!(b.anchor, Point::new(10.0, 0.0));
    assert_eq!(sp[1].knots.len(), 2);
}

#[test]
fn text_without_any_font_is_reported_not_silently_dropped() {
    // No font at all (a bare CI image): usvg drops the <text> while parsing, so nothing
    // reaches the conversion; the reader still has to say why the text is missing.
    fn no_fonts() -> Arc<usvg::fontdb::Database> {
        Arc::new(usvg::fontdb::Database::new())
    }
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"60\"><text x=\"5\" y=\"45\" font-size=\"48\">II</text></svg>";
    let parsed = parse_with(svg, no_fonts).unwrap();
    assert!(parsed.fontless_text);
    let r = import_parsed("t", parsed, MAX_GROUP_DEPTH, None, false).unwrap();
    assert!(r.document.layers.is_empty());
    assert!(r.warnings.iter().any(|w| w.contains("font")), "{:?}", r.warnings);
    // Clipboard/import paths use the bounded parser and retain the same diagnostic.
    let (parsed, _) = parse_bounded_inner_with(svg, MAX_RASTER_PIXELS, no_fonts).unwrap();
    assert!(parsed.fontless_text);
    let r = import_parsed("t", parsed, MAX_GROUP_DEPTH, Some(MAX_RASTER_PIXELS), false).unwrap();
    assert!(r.document.layers.is_empty());
    assert!(r.warnings.iter().any(|w| w.contains("font")), "{:?}", r.warnings);
    // A drawing without text never loads or misses fonts.
    let parsed =
        parse_with(b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><rect width=\"10\" height=\"10\"/></svg>", no_fonts).unwrap();
    assert!(!parsed.fontless_text);
    let r = import_parsed("t", parsed, MAX_GROUP_DEPTH, None, false).unwrap();
    assert!(r.warnings.iter().all(|w| !w.contains("font")), "{:?}", r.warnings);
}

/// #4806: text placed by a `transform` (Illustrator writes `matrix(1 0 0 1 x y)` on every text
/// element), on the text or on a group around it, lands where resvg draws it. usvg leaves the
/// glyph outlines' absolute transforms at identity, so reading them dropped the transform: the
/// text went off the canvas or kept its untransformed place and size.
///
/// Shaped with the bundled Inter alone, never a system font: on a machine without fonts (a
/// bare CI image or VM) the test would otherwise have nothing to check, and the drawing and
/// its reference come from the same parse, so both use that face.
#[test]
fn transformed_text_lands_where_resvg_draws_it() {
    fn inter() -> Arc<usvg::fontdb::Database> {
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(photocraft_text::fonts::INTER_REGULAR.to_vec());
        Arc::new(db)
    }
    assert!(inter().faces().next().is_some(), "the bundled Inter loads");
    let font = r#"font-size="24" font-family="Inter" fill="black""#;
    let cases = [
        format!(r#"<text transform="translate(10 40)" {font}>IHI</text>"#),
        format!(r#"<text transform="matrix(1 0 0 1 10 40)" {font}>IHI</text>"#),
        format!(r#"<text transform="translate(10 0)" x="20" y="40" {font}>IHI</text>"#),
        format!(r#"<text transform="scale(2)" x="5" y="25" {font}>IHI</text>"#),
        format!(r#"<g transform="translate(10 40)"><text {font}>IHI</text></g>"#),
        format!(r#"<g transform="translate(60 0)"><g transform="rotate(90)"><text x="5" y="20" {font}>IHI</text></g></g>"#),
    ];
    for c in cases {
        let svg = format!(r#"<svg xmlns="http://www.w3.org/2000/svg" width="120" height="60">{c}</svg>"#);
        let parsed = parse_with(svg.as_bytes(), inter).unwrap();
        assert!(!parsed.fontless_text, "{c}");
        let reference = draw::render(&parsed.tree, usvg::Transform::default(), 120, 60, 0, MAX_PEAK_PIXEL_BYTES).unwrap();
        let r = import_parsed("t", parsed, MAX_GROUP_DEPTH, None, false).unwrap();
        let [layer] = r.document.layers.as_slice() else { panic!("one text layer, got {}: {c}", r.document.layers.len()) };
        assert!(matches!(layer.content, LayerContent::Group(_)), "text stays outline shapes: {c}");
        let imported = photocraft_compose::flatten(&r.document);
        let (mut both, mut either) = (0usize, 0usize);
        let (mut a_box, mut b_box) = ([i32::MAX, i32::MAX, i32::MIN, i32::MIN], [i32::MAX, i32::MAX, i32::MIN, i32::MIN]);
        let grow = |bx: &mut [i32; 4], x: i32, y: i32| *bx = [bx[0].min(x), bx[1].min(y), bx[2].max(x), bx[3].max(y)];
        for (i, px) in reference.pixels().iter().enumerate() {
            let (x, y) = ((i % 120) as i32, (i / 120) as i32);
            let (a, b) = (imported.get(x, y)[3] > 0.5, px.alpha() > 127);
            if a {
                grow(&mut a_box, x, y);
            }
            if b {
                grow(&mut b_box, x, y);
            }
            both += usize::from(a && b);
            either += usize::from(a || b);
        }
        assert!(either > 50, "resvg draws the text on the canvas: {c}");
        // Same place and size to the pixel; the outlines are filled by a different rasteriser,
        // so the edges of thin stems may differ by half a pixel.
        assert!(a_box.iter().zip(b_box).all(|(a, b)| (a - b).abs() <= 1), "imported {a_box:?}, resvg {b_box:?}: {c}");
        assert!(both * 10 >= either * 7, "imported glyphs cover what resvg draws ({both} of {either} px): {c}");
    }
}

/// #4266: a group clipped by one path (the PhotoCraft icon's rounded square) stays editable
/// shapes with the clip as its vector mask, and looks as resvg draws the drawing; a clip of
/// two shapes is still rasterised, with the warning.
#[test]
fn a_single_path_clip_becomes_the_groups_vector_mask() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">
            <defs><clipPath id="c"><rect x="4" y="4" width="40" height="40" rx="10"/></clipPath></defs>
            <g transform="translate(8 6)" clip-path="url(#c)">
              <rect width="64" height="64" fill="#2f7bf5"/>
              <circle cx="40" cy="40" r="20" fill="#efe9dc"/>
            </g></svg>"##;
    let r = import_svg("t", svg).unwrap();
    assert!(r.warnings.iter().all(|w| !w.contains("clip-path")), "{:?}", r.warnings);
    let [group] = r.document.layers.as_slice() else { panic!("one group: {:?}", r.document.layers.len()) };
    let LayerContent::Group(g) = &group.content else { panic!("a group") };
    assert_eq!(g.children.len(), 2);
    assert!(g.children.iter().all(|c| matches!(c.content, LayerContent::Shape(_))), "editable shapes");
    let mask = group.vector_mask.as_ref().expect("the clip as a vector mask");
    let xs: Vec<f64> = mask.path.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| k.anchor.x)).collect();
    let (lo, hi) = (xs.iter().copied().fold(f64::MAX, f64::min), xs.iter().copied().fold(f64::MIN, f64::max));
    assert!((lo - 12.0).abs() < 1e-3 && (hi - 52.0).abs() < 1e-3, "in the group's space: {lo}..{hi}");
    // Pixel for pixel what resvg draws, but for anti-aliasing along the edges.
    let ours = photocraft_compose::flatten(&r.document).to_rgba8().pixels;
    let tree = parse(svg).unwrap().tree;
    let pm = draw::render(&tree, usvg::Transform::default(), 64, 64, 0, MAX_PEAK_PIXEL_BYTES).unwrap();
    let theirs: Vec<u8> = pm
        .pixels()
        .iter()
        .flat_map(|p| {
            let c = p.demultiply();
            [c.red(), c.green(), c.blue(), c.alpha()]
        })
        .collect();
    let off = ours.chunks(4).zip(theirs.chunks(4)).filter(|(a, b)| a.iter().zip(*b).any(|(x, y)| x.abs_diff(*y) > 48)).count();
    assert!(off < 64, "{off} pixels differ");
    // The report's file: PhotoCraft's own icon opens without the clip-path warning.
    let icon = import_svg("photocraft.svg", include_bytes!("../../../../assets/app-icon/photocraft.svg")).unwrap();
    assert!(icon.warnings.iter().all(|w| !w.contains("clip-path")), "{:?}", icon.warnings);
    assert!(icon.document.layers.iter().any(|l| l.vector_mask.is_some()), "its rounded square is a vector mask");
    // Two shapes in the clip: rasterised, and said so.
    let two = br##"<svg xmlns="http://www.w3.org/2000/svg" width="64" height="64">
            <defs><clipPath id="c"><rect width="20" height="20"/><rect x="30" width="20" height="20"/></clipPath></defs>
            <g clip-path="url(#c)"><rect width="64" height="64"/><rect width="9" height="9" fill="red"/></g></svg>"##;
    let r = import_svg("t", two).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("clip-path")), "{:?}", r.warnings);
}

/// A database with the bundled Inter alone, generics resolved without Fontconfig: none of
/// fontdb's Windows defaults (Arial, Times New Roman, Courier New) is in it.
fn inter_only() -> Arc<usvg::fontdb::Database> {
    let mut db = usvg::fontdb::Database::new();
    db.load_font_data(include_bytes!("../../../../assets/fonts/Inter-Regular.ttf").to_vec());
    resolve_generic_families(&mut db, |_| None);
    Arc::new(db)
}

/// Text asking for fonts that aren't installed (`Arial, Helvetica, sans-serif` on a Linux
/// without them) is set in an installed font, as a browser does, instead of being dropped.
#[test]
fn text_in_missing_fonts_falls_back_to_an_installed_one() {
    for family in ["Arial, Helvetica, sans-serif", "Times New Roman", "Courier New, monospace", "cursive", "fantasy"] {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"60\"><text x=\"5\" y=\"45\" font-size=\"48\" font-family=\"{family}\">II</text></svg>"
        );
        let parsed = parse_with(svg.as_bytes(), inter_only).unwrap();
        assert!(!parsed.fontless_text);
        let r = import_parsed("t", parsed, MAX_GROUP_DEPTH, None, false).unwrap();
        assert_eq!(r.document.layers.first().map(|l| l.name.as_str()), Some("II"), "{family}: {:?}", r.warnings);
    }
}

/// #9406: a font installed on disk (how the system fonts load) is read once, not once per
/// glyph: fontdb re-reads a file source's whole font on every glyph it outlines, about a
/// millisecond a character. The face text is set in comes from memory, and the outlines
/// match the same font loaded from memory in the first place.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn text_in_a_font_on_disk_is_set_from_memory() {
    fn inter_on_disk() -> Arc<usvg::fontdb::Database> {
        let mut db = usvg::fontdb::Database::new();
        let _ = db.load_font_file(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/fonts/Inter-Regular.ttf"));
        Arc::new(db)
    }
    assert!(inter_on_disk().faces().all(|f| matches!(f.source, usvg::fontdb::Source::File(_))));
    let svg =
        b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"60\"><text x=\"5\" y=\"45\" font-size=\"48\" font-family=\"Inter\">IHI</text></svg>";
    let parsed = parse_with(svg, inter_on_disk).unwrap();
    let faces: Vec<_> = parsed.tree.fontdb().faces().map(|f| matches!(f.source, usvg::fontdb::Source::Binary(_))).collect();
    assert_eq!(faces, [true], "the face text was set in is held in memory");
    let on_disk = import_parsed("t", parsed, MAX_GROUP_DEPTH, None, false).unwrap();
    let in_memory = import_parsed("t", parse_with(svg, inter_only).unwrap(), MAX_GROUP_DEPTH, None, false).unwrap();
    assert_eq!(on_disk.document.layers.first().map(|l| l.name.as_str()), Some("IHI"));
    let outlines = |r: &ImportResult| {
        let groups = r.document.layers.iter().filter_map(|l| if let LayerContent::Group(g) = &l.content { Some(g) } else { None });
        groups.flat_map(|g| &g.children).filter_map(|l| if let LayerContent::Shape(s) = &l.content { Some(s.path.clone()) } else { None }).collect::<Vec<_>>()
    };
    assert!(outlines(&on_disk).iter().any(|p| !p.subpaths.is_empty()), "the text has outlines");
    assert_eq!(outlines(&on_disk), outlines(&in_memory));
}

/// A font file that no longer loads (replaced or cut short since fontdb indexed it) keeps its
/// faces in the database and isn't remembered: once the file is whole again, text is set
/// from it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_font_file_gone_bad_since_indexing_is_not_cached() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static BAD: AtomicBool = AtomicBool::new(true);
    fn indexed_then_overwritten() -> Arc<usvg::fontdb::Database> {
        let path = std::env::temp_dir().join(format!("photocraft-svg-test-bad-{}.ttf", std::process::id()));
        std::fs::write(&path, include_bytes!("../../../../assets/fonts/Inter-Regular.ttf")).unwrap();
        let mut db = usvg::fontdb::Database::new();
        db.load_font_file(&path).unwrap();
        if BAD.load(Ordering::SeqCst) {
            std::fs::write(&path, b"not a font any more").unwrap();
        }
        Arc::new(db)
    }
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"60\"><text x=\"5\" y=\"45\" font-family=\"Inter\">IHI</text></svg>";
    let parsed = parse_with(svg, indexed_then_overwritten).unwrap();
    let sources: Vec<_> = parsed.tree.fontdb().faces().map(|f| matches!(f.source, usvg::fontdb::Source::File(_))).collect();
    assert_eq!(sources, [true], "the face stays in the database, on disk");
    BAD.store(false, Ordering::SeqCst);
    let parsed = parse_with(svg, indexed_then_overwritten).unwrap();
    let sources: Vec<_> = parsed.tree.fontdb().faces().map(|f| matches!(f.source, usvg::fontdb::Source::Binary(_))).collect();
    assert_eq!(sources, [true], "the whole file is read into memory now");
    let layers = import_parsed("t", parsed, MAX_GROUP_DEPTH, None, false).unwrap().document.layers;
    assert_eq!(layers.first().map(|l| l.name.as_str()), Some("IHI"));
    assert!(
        layers.iter().any(|l| matches!(&l.content, LayerContent::Group(g) if g.children.iter().any(|c| matches!(&c.content, LayerContent::Shape(s) if !s.path.subpaths.is_empty())))),
        "the text has outlines"
    );
}

/// Imports that miss the cache for the same file at once read it once: every one sets its
/// text from the same bytes, not a copy of its own next to the kept one.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn imports_missing_the_same_font_at_once_share_one_read() {
    use std::sync::atomic::{AtomicUsize, Ordering};
    static ROUND: AtomicUsize = AtomicUsize::new(0);
    fn fresh_file() -> Arc<usvg::fontdb::Database> {
        let round = ROUND.load(Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("photocraft-svg-test-shared-{}-{round}.ttf", std::process::id()));
        let mut db = usvg::fontdb::Database::new();
        db.load_font_file(&path).unwrap();
        Arc::new(db)
    }
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"60\"><text x=\"5\" y=\"45\" font-family=\"Inter\">IHI</text></svg>";
    for round in 0..5 {
        ROUND.store(round, Ordering::SeqCst);
        let path = std::env::temp_dir().join(format!("photocraft-svg-test-shared-{}-{round}.ttf", std::process::id()));
        std::fs::write(&path, include_bytes!("../../../../assets/fonts/Inter-Regular.ttf")).unwrap();
        let start = Arc::new(std::sync::Barrier::new(8));
        let threads: Vec<_> = (0..8)
            .map(|_| {
                let start = start.clone();
                std::thread::spawn(move || {
                    start.wait();
                    let parsed = parse_with(svg, fresh_file).unwrap();
                    let bytes: Vec<_> = parsed
                        .tree
                        .fontdb()
                        .faces()
                        .filter_map(|f| if let usvg::fontdb::Source::Binary(b) = &f.source { Some(Arc::as_ptr(b).cast::<u8>() as usize) } else { None })
                        .collect();
                    bytes
                })
            })
            .collect();
        let bytes: Vec<_> = threads.into_iter().flat_map(|t| t.join().unwrap()).collect();
        assert_eq!(bytes.len(), 8, "every import set its text from memory");
        assert!(bytes.iter().all(|&b| b == bytes[0]), "round {round}: {bytes:x?}");
        let _ = std::fs::remove_file(&path);
    }
}

/// #9671: the fallback search for a character the text's font lacks reads each font file's
/// character map once per process, not the whole file on every text element (430 ms an
/// element with 273 installed faces). A second import still finds the fallback face after
/// its file stopped being readable, so it didn't go back to the disk.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_fallback_search_reads_each_font_file_once() {
    use std::sync::atomic::{AtomicBool, Ordering};
    static GONE: AtomicBool = AtomicBool::new(false);
    fn path() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("photocraft-svg-test-fallback-{}.ttf", std::process::id()))
    }
    fn inter_and_mono_on_disk() -> Arc<usvg::fontdb::Database> {
        std::fs::write(path(), include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf")).unwrap();
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(include_bytes!("../../../../assets/fonts/Inter-Regular.ttf").to_vec());
        db.load_font_file(path()).unwrap();
        if GONE.load(Ordering::SeqCst) {
            std::fs::write(path(), b"not a font any more").unwrap();
        }
        Arc::new(db)
    }
    // Inter has no box-drawing characters; JetBrains Mono has.
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"60\"><text x=\"5\" y=\"45\" font-size=\"48\" font-family=\"Inter\">I\u{2500}I</text></svg>";
    let outlines = |parsed: Parsed| {
        let layers = import_parsed("t", parsed, MAX_GROUP_DEPTH, None, false).unwrap().document.layers;
        let groups = layers.into_iter().filter_map(|l| if let LayerContent::Group(g) = l.content { Some(g) } else { None });
        groups.flat_map(|g| g.children).filter_map(|l| if let LayerContent::Shape(s) = l.content { Some(s.path) } else { None }).collect::<Vec<_>>()
    };
    let first = parse_with(svg.as_bytes(), inter_and_mono_on_disk).unwrap();
    let mono =
        first.tree.fontdb().faces().any(|f| f.families.iter().any(|(n, _)| n == "JetBrains Mono") && matches!(f.source, usvg::fontdb::Source::Binary(_)));
    assert!(mono, "the box-drawing character falls back to JetBrains Mono");
    let first = outlines(first);
    GONE.store(true, Ordering::SeqCst);
    let second = outlines(parse_with(svg.as_bytes(), inter_and_mono_on_disk).unwrap());
    let _ = std::fs::remove_file(path());
    assert!(first.iter().any(|p| !p.subpaths.is_empty()), "the text has outlines");
    assert!(second == first, "the second import set the character in the same fallback face");
}

/// #9671: the kept `cmap` is the one usvg reads (the last such record of a linear walk), also
/// in a font whose table directory isn't sorted by tag, where a binary search misses it.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn the_fallback_search_reads_the_cmap_of_an_unsorted_font() {
    fn inter_and_unsorted_mono_on_disk() -> Arc<usvg::fontdb::Database> {
        let mut font = include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf").to_vec();
        let count = usize::from(u16::from_be_bytes([font[4], font[5]]));
        // The `cmap` record moves to the end of the directory.
        let mut records: Vec<Vec<u8>> = font[12..12 + count * 16].chunks(16).map(<[u8]>::to_vec).collect();
        let at = records.iter().position(|r| r.starts_with(b"cmap")).unwrap();
        let cmap = records.remove(at);
        records.push(cmap);
        font.splice(12..12 + count * 16, records.concat());
        let raw = ttf_parser::RawFace::parse(&font, 0).unwrap();
        assert!(raw.table(ttf_parser::Tag::from_bytes(b"cmap")).is_none(), "a binary search misses the cmap");
        assert!(raw.table(ttf_parser::Tag::from_bytes(b"name")).is_some(), "fontdb still finds the name");
        let path = std::env::temp_dir().join(format!("photocraft-svg-test-unsorted-{}.ttf", std::process::id()));
        std::fs::write(&path, font).unwrap();
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(include_bytes!("../../../../assets/fonts/Inter-Regular.ttf").to_vec());
        db.load_font_file(&path).unwrap();
        Arc::new(db)
    }
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"60\"><text x=\"5\" y=\"45\" font-family=\"Inter\">I\xE2\x94\x80I</text></svg>";
    let parsed = parse_with(svg, inter_and_unsorted_mono_on_disk).unwrap();
    let _ = std::fs::remove_file(std::env::temp_dir().join(format!("photocraft-svg-test-unsorted-{}.ttf", std::process::id())));
    let mono =
        parsed.tree.fontdb().faces().any(|f| f.families.iter().any(|(n, _)| n == "JetBrains Mono") && matches!(f.source, usvg::fontdb::Source::Binary(_)));
    assert!(mono, "the box-drawing character falls back to JetBrains Mono");
}

/// #9406: moving a font collection into memory keeps its faces in their order, so the
/// fallback search for a later text still finds the face it found on disk first.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn a_collection_in_memory_keeps_its_face_order() {
    fn collection_on_disk() -> Arc<usvg::fontdb::Database> {
        // A two-face .ttc of the bundled Inter Regular and SemiBold: the `ttcf` header, then
        // each font with its table offsets moved by where it starts.
        let fonts: [&[u8]; 2] = [include_bytes!("../../../../assets/fonts/Inter-Regular.ttf"), include_bytes!("../../../../assets/fonts/Inter-SemiBold.ttf")];
        let mut ttc = b"ttcf      ".to_vec();
        let mut start = 20;
        let mut body = Vec::new();
        for f in fonts {
            ttc.extend_from_slice(&u32::try_from(start).unwrap().to_be_bytes());
            let mut font = f.to_vec();
            for t in 0..usize::from(u16::from_be_bytes([font[4], font[5]])) {
                let at = 12 + t * 16 + 8;
                let off = u32::from_be_bytes(font[at..at + 4].try_into().unwrap()) + u32::try_from(start).unwrap();
                font[at..at + 4].copy_from_slice(&off.to_be_bytes());
            }
            font.resize(font.len().next_multiple_of(4), 0);
            start += font.len();
            body.extend(font);
        }
        ttc.extend(body);
        let path = std::env::temp_dir().join(format!("photocraft-svg-test-{}.ttc", std::process::id()));
        std::fs::write(&path, ttc).unwrap();
        let mut db = usvg::fontdb::Database::new();
        db.load_font_file(&path).unwrap();
        Arc::new(db)
    }
    let order = |db: &usvg::fontdb::Database| db.faces().map(|f| (f.index, f.weight.0)).collect::<Vec<_>>();
    assert_eq!(order(&collection_on_disk()), [(0, 400), (1, 600)]);
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"120\" height=\"60\"><text x=\"5\" y=\"45\" font-family=\"Inter\">IHI</text></svg>";
    let parsed = parse_with(svg, collection_on_disk).unwrap();
    let db = parsed.tree.fontdb();
    assert!(db.faces().all(|f| matches!(f.source, usvg::fontdb::Source::Binary(_))), "both faces of the file are in memory");
    assert_eq!(order(db), [(0, 400), (1, 600)]);
}

#[test]
fn generic_families_point_at_installed_fonts() {
    use usvg::fontdb::Family;
    let db = inter_only();
    for g in [Family::SansSerif, Family::Serif, Family::Monospace, Family::Cursive, Family::Fantasy] {
        assert_eq!(db.family_name(&g), "Inter", "{g:?}");
    }
    // The system's choice wins when installed; a generic whose font is installed stays.
    let mut db = usvg::fontdb::Database::new();
    db.load_font_data(include_bytes!("../../../../assets/fonts/Inter-Regular.ttf").to_vec());
    db.load_font_data(include_bytes!("../../../../assets/fonts/JetBrainsMono-Regular.ttf").to_vec());
    db.set_cursive_family("Inter");
    resolve_generic_families(&mut db, |g| Some(if g == "serif" { "JetBrains Mono" } else { "Not Installed" }.to_string()));
    assert_eq!(db.family_name(&Family::Serif), "JetBrains Mono");
    assert_eq!(db.family_name(&Family::Monospace), "JetBrains Mono", "the bundled mono stand-in");
    assert_eq!(db.family_name(&Family::SansSerif), "Inter");
    assert_eq!(db.family_name(&Family::Cursive), "Inter");
    // An empty database is left alone.
    let mut empty = usvg::fontdb::Database::new();
    resolve_generic_families(&mut empty, |_| Some("Inter".into()));
    assert_eq!(empty.family_name(&Family::SansSerif), "Arial");
}

#[test]
fn rasterized_groups_stop_at_the_aggregate_pixel_budget() {
    let mut svg = String::from("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"256\" height=\"256\">");
    svg.push_str("<defs><mask id=\"m\"><rect width=\"256\" height=\"256\" fill=\"white\"/></mask></defs>");
    for i in 0..8 {
        svg.push_str(&format!("<g id=\"g{i}\" mask=\"url(#m)\"><rect width=\"256\" height=\"256\" fill=\"#00ff00\"/></g>"));
    }
    svg.push_str("</svg>");
    // Enough for one retained 256×256 RGBA8 surface, not eight of them.
    let budget = 256u64 * 256 * 4;
    let result = import_parsed_with_budget("budget", parse(svg.as_bytes()).unwrap(), MAX_GROUP_DEPTH, budget).unwrap();
    assert_eq!(result.document.layers.len(), 1, "one U8 surface fills the retained-byte allowance");
    assert_eq!(result.document.layers[0].name, "g0");
    assert!(result.warnings.iter().any(|warning| warning.contains("pixel memory")), "{:?}", result.warnings);
}

#[test]
fn oversized_raster_does_not_consume_budget_for_later_layers() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="5100" height="4800">
            <defs>
              <mask id="big" maskUnits="userSpaceOnUse" x="0" y="0" width="5100" height="4800">
                <rect width="5100" height="4800" fill="white"/>
              </mask>
              <mask id="small-mask" maskUnits="userSpaceOnUse" x="0" y="0" width="32" height="32">
                <rect width="32" height="32" fill="white"/>
              </mask>
            </defs>
            <g id="oversized" mask="url(#big)"><rect width="5100" height="4800" fill="red"/></g>
            <g id="small" mask="url(#small-mask)"><rect width="32" height="32" fill="blue"/></g>
        </svg>"##;
    let result = import_parsed_with_budget("budget", parse(svg).unwrap(), MAX_GROUP_DEPTH, 256 * 256 * 4).unwrap();
    assert_eq!(result.document.layers.len(), 1, "the oversized group must not debit the budget");
    assert_eq!(result.document.layers[0].name, "small");
    assert!(result.warnings.iter().any(|w| w.contains("too large to rasterise")), "{:?}", result.warnings);
    assert!(!result.warnings.iter().any(|w| w.contains("pixel memory budget")), "{:?}", result.warnings);
}

#[test]
fn skipped_expensive_raster_does_not_hide_a_smaller_one() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="512" height="256">
            <defs>
              <mask id="big"><rect width="512" height="256" fill="white"/></mask>
              <mask id="little"><rect width="16" height="16" fill="white"/></mask>
            </defs>
            <g id="large" mask="url(#big)"><rect width="512" height="256" fill="red"/></g>
            <g id="small" mask="url(#little)"><rect width="16" height="16" fill="blue"/></g>
        </svg>"##;
    let result = import_parsed_with_budget("budget", parse(svg).unwrap(), MAX_GROUP_DEPTH, 256 * 256 * 4).unwrap();
    assert_eq!(result.document.layers.len(), 1);
    assert_eq!(result.document.layers[0].name, "small");
    assert!(result.warnings.iter().any(|w| w.contains("some rasterised elements were omitted")), "{:?}", result.warnings);
}

#[test]
fn huge_drawings_scale_to_the_side_limit() {
    let tree = parse(b"<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 100000 50000\"/>").unwrap().tree;
    let mut w = Vec::new();
    let cv = canvas(&tree, &mut w);
    assert_eq!((cv.size.width, cv.size.height), (MAX_SIDE, MAX_SIDE / 2));
    assert_eq!(w.len(), 1);
}

/// A pasted drawing is one pixel per SVG unit, and a drawing past the pixel limit is scaled
/// down to fit it, keeping its proportions, instead of failing.
#[test]
fn rgba8_renders_at_its_own_size_within_the_pixel_limit() {
    let svg = br##"<svg xmlns="http://www.w3.org/2000/svg" width="40" height="20"><rect width="20" height="20" fill="#ff0000"/></svg>"##;
    let (w, h, px) = rasterize_rgba8(svg).unwrap();
    assert_eq!((w, h, px.len()), (40, 20, 40 * 20 * 4));
    assert_eq!(px[..4], [255, 0, 0, 255], "the rectangle");
    assert_eq!(px[(10 * 40 + 30) * 4..][..4], [0, 0, 0, 0], "transparent beside it");
    for (side_w, side_h) in [(10_000u32, 5_000u32), (20_000, 20_000), (20_000, 2_000)] {
        let svg = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{side_w}\" height=\"{side_h}\"/>");
        let (w, h, px) = rasterize_rgba8(svg.as_bytes()).unwrap();
        let pixels = u64::from(w) * u64::from(h);
        assert!(pixels <= MAX_RASTER_PIXELS && pixels > MAX_RASTER_PIXELS / 2, "{side_w} × {side_h} became {w} × {h}");
        assert!(w <= MAX_SIDE && h <= MAX_SIDE, "{w} × {h}");
        let (aspect, want) = (f64::from(w) / f64::from(h), f64::from(side_w) / f64::from(side_h));
        assert!((aspect / want - 1.0).abs() < 0.01, "{w} × {h} keeps {side_w} × {side_h}");
        assert_eq!(px.len() as u64, pixels * 4);
    }
    assert!(rasterize_rgba8(b"<svg xmlns=\"http://www.w3.org/2000/svg\"><rect").is_err(), "malformed markup is an error");
}

#[test]
fn bounded_import_preserves_shapes_at_budget_and_rasterises_aggregate_overflow() {
    let bytes = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="red"/><rect x="4" width="4" height="8" fill="blue"/></svg>"#;
    let at_limit = import_svg_bounded("bounded", bytes, 128).unwrap();
    assert_eq!(at_limit.document.layers.len(), 2);
    assert!(at_limit.document.layers.iter().all(|l| matches!(l.content, LayerContent::Shape(_))));
    let over = import_svg_bounded("bounded", bytes, 127).unwrap();
    assert!(import_svg_bounded_layers("bounded", bytes, 127).unwrap_err().to_string().contains("cache budget"));
    assert_eq!(over.document.layers.len(), 1);
    assert!(matches!(over.document.layers[0].content, LayerContent::Raster(_)));
    assert!(over.warnings.iter().any(|w| w.contains("cache budget") && w.contains("editable shapes")));
    let surface = over.document.layers[0].surface().unwrap();
    assert_eq!(surface.read_region(Rect::new(1, 1, 2, 2)), vec![1.0, 0.0, 0.0, 1.0]);
    assert_eq!(surface.read_region(Rect::new(6, 1, 7, 2)), vec![0.0, 0.0, 1.0, 1.0]);
    // Ordinary file imports retain their existing editable-layer behavior.
    assert_eq!(import_svg("unbounded", bytes).unwrap().document.layers.len(), 2);
}

#[test]
fn bounded_import_rejects_canvas_before_rendering_and_respects_raster_limit() {
    let bytes = br#"<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8"/></svg>"#;
    assert!(import_svg_bounded("bounded", bytes, 0).is_err());
    assert!(import_svg_bounded("bounded", bytes, 63).is_err());
    assert!(import_svg_bounded("bounded", bytes, 64).is_ok());
    let oversized = br#"<svg xmlns="http://www.w3.org/2000/svg" width="5000" height="5000"><rect width="5000" height="5000"/></svg>"#;
    assert!(import_svg_bounded("bounded", oversized, u64::MAX).unwrap_err().to_string().contains("cache budget"));
}

/// The smart-object budget is 24,000,000 pixels: a total exactly equal to it fits, one more
/// node of caches does not. Bounds-only callers keep a frame past the layer-cache budget, and
/// the canvas itself is limited at the same exact boundary.
#[test]
fn bounded_canvas_bounds_match_the_import_at_the_exact_24_mp_boundary() {
    const BUDGET: u64 = 24_000_000;
    let rects = |n: usize| {
        format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2000\" height=\"2000\">{}</svg>", "<rect width=\"2000\" height=\"2000\"/>".repeat(n))
    };
    // 2000 × 2000 × 6 = 24,000,000: exactly at the budget, still editable layers.
    let at_budget = rects(6);
    let strict = import_svg_bounded_layers("at", at_budget.as_bytes(), BUDGET).unwrap();
    assert_eq!(strict.document.layers.len(), 6);
    assert_eq!(bounded_canvas_bounds(at_budget.as_bytes(), BUDGET).unwrap(), strict.document.bounds());
    // A seventh node is 28,000,000 pixels of caches: strict layers refuse it, bounds do not.
    let over = rects(7);
    assert!(import_svg_bounded_layers("over", over.as_bytes(), BUDGET).unwrap_err().to_string().contains("cache budget"));
    assert_eq!(bounded_canvas_bounds(over.as_bytes(), BUDGET).unwrap(), Rect::new(0, 0, 2000, 2000));
    // The canvas: 4000 × 6000 is exactly 24 MP and fits; one more row does not.
    let canvas = |h: u32| format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4000\" height=\"{h}\"><rect width=\"8\" height=\"8\"/></svg>");
    assert_eq!(bounded_canvas_bounds(canvas(6000).as_bytes(), BUDGET).unwrap(), Rect::new(0, 0, 4000, 6000));
    assert!(bounded_canvas_bounds(canvas(6001).as_bytes(), BUDGET).unwrap_err().to_string().contains("cache budget"));
    // A larger request never lifts the 24 MP rasterisation limit, and parse budgets still apply.
    assert!(bounded_canvas_bounds(canvas(6001).as_bytes(), u64::MAX).is_err());
    assert!(bounded_canvas_bounds(b"<svg", BUDGET).is_err());
    let huge_image = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1\" height=\"1\">{}</svg>",
        embedded_image(
            "image/png",
            &[b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR".as_slice(), &32768u32.to_be_bytes(), &32768u32.to_be_bytes(), &[8, 6, 0, 0, 0]].concat()
        )
    );
    assert!(bounded_canvas_bounds(huge_image.as_bytes(), BUDGET).is_err(), "decoded images keep their budget");
}
/// A one-font database reproduces Linux hosts without Times New Roman. A missing authored
/// family must still produce text geometry; it must not silently disappear on save-back.
#[test]
fn missing_svg_font_uses_an_available_generic_instead_of_dropping_text() {
    fn inter() -> Arc<usvg::fontdb::Database> {
        let mut db = usvg::fontdb::Database::new();
        db.load_font_data(include_bytes!("../../../../assets/fonts/Inter-Regular.ttf").to_vec());
        resolve_generic_families(&mut db, |_| None);
        Arc::new(db)
    }
    let svg=br#"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="30"><text x="2" y="24" font-size="20" font-family="Unavailable Authored Font">LOGO</text></svg>"#;
    let parsed = parse_with(svg, inter).unwrap();
    assert!(!parsed.fontless_text);
    assert!(!parsed.tree.root().children().is_empty(), "missing-family fallback must preserve text geometry");
    let mut db = (*inter()).clone();
    let before = db.query(&usvg::fontdb::Query { families: &[usvg::fontdb::Family::Serif], ..Default::default() });
    resolve_generic_families(&mut db, |_| None);
    assert_eq!(db.query(&usvg::fontdb::Query { families: &[usvg::fontdb::Family::Serif], ..Default::default() }), before);
}

#[test]
fn resolve_data_rejects_oversized_and_malformed_embedded_png_and_webp() {
    let opt = options(Arc::new(usvg::fontdb::Database::new()), MAX_XML_DEPTH);
    let resolve = &opt.image_href_resolver.resolve_data;
    let img = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, vec![255; 12]).unwrap();
    let valid_png = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    assert!(resolve("image/png", Arc::new(valid_png.clone()), &opt).is_some());
    assert!(resolve("text/plain", Arc::new(valid_png.clone()), &opt).is_some());

    let mut huge_png = valid_png;
    huge_png[16..20].copy_from_slice(&40_000u32.to_be_bytes());
    huge_png[20..24].copy_from_slice(&40_000u32.to_be_bytes());
    assert!(resolve("image/png", Arc::new(huge_png.clone()), &opt).is_none(), "oversized PNG rejected before decode");
    assert!(resolve("text/plain", Arc::new(huge_png), &opt).is_none(), "oversized mime-less PNG rejected before decode");

    // Malformed WebP header under image/webp or text/plain must be rejected at the probe step.
    let bad_webp = b"RIFF\x10\x00\x00\x00WEBPVP8 ".to_vec();
    assert!(resolve("image/webp", Arc::new(bad_webp.clone()), &opt).is_none());
    assert!(resolve("text/plain", Arc::new(bad_webp), &opt).is_none());
}

#[test]
fn jpeg_allocation_probe_uses_the_frame_header_and_oversized_jpegs_are_refused() {
    // SOI, an APP0 segment of length 4, then SOF0 (length 11): 8 bits, height 40000, width 30000.
    let jpeg = |h: u16, w: u16| {
        let mut v = vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x04, 0, 0, 0xFF, 0xC0, 0x00, 0x0B, 8];
        v.extend(h.to_be_bytes());
        v.extend(w.to_be_bytes());
        v.extend([1, 1, 0x11, 0]);
        v
    };
    assert_eq!(photocraft_codecs::probe_allocation_size(&jpeg(400, 300)).unwrap(), (300, 400));
    assert!(photocraft_codecs::probe_allocation_size(&[0xFF, 0xD8, 0xFF, 0xDA, 0, 2]).is_err());
    assert!(photocraft_codecs::probe_allocation_size(&[0xFF, 0xD8]).is_err());
    let opt = options(Arc::new(usvg::fontdb::Database::new()), MAX_XML_DEPTH);
    let resolve = &opt.image_href_resolver.resolve_data;
    // 1.2 billion pixels would be 4.8 GB of RGBA before resvg scaled anything down.
    assert!(resolve("image/jpeg", Arc::new(jpeg(40_000, 30_000)), &opt).is_none());
    assert!(resolve("text/plain", Arc::new(jpeg(40_000, 30_000)), &opt).is_none());
}

#[test]
fn resolve_data_rejects_nested_gzip_and_handles_text_plain_svgz() {
    use std::io::Write;
    fn gz(bytes: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(bytes).unwrap();
        e.finish().unwrap()
    }
    let opt = options(Arc::new(usvg::fontdb::Database::new()), MAX_XML_DEPTH);
    let resolve = &opt.image_href_resolver.resolve_data;
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"><rect width=\"4\" height=\"4\"/></svg>";
    let once = gz(svg);
    assert!(resolve("image/svg+xml", Arc::new(once.clone()), &opt).is_some());
    assert!(resolve("text/plain", Arc::new(once.clone()), &opt).is_some());

    // Doubly gzip-compressed payload must be rejected rather than decompressed a second time by usvg.
    let twice = gz(&once);
    assert!(resolve("image/svg+xml", Arc::new(twice.clone()), &opt).is_none());
    assert!(resolve("text/plain", Arc::new(twice), &opt).is_none());
}

/// The outer gzip is size-limited by PhotoCraft. Its result must be XML, never another
/// compressed stream which usvg could inflate independently without that byte limit.
#[test]
fn bounded_svg_rejects_nested_gzip_at_top_level_and_inside_data_urls() {
    use std::io::Write;
    let gz = |bytes: &[u8]| {
        let mut encoder = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        encoder.write_all(bytes).unwrap();
        encoder.finish().unwrap()
    };
    let svg = b"<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\"><rect width=\"4\" height=\"4\"/></svg>";
    let once = gz(svg);
    let twice = gz(&once);
    assert!(parse_bounded(&once, 64).is_ok(), "ordinary SVGZ remains accepted");
    // craft-svg refuses gzip inside gzip when it unpacks the file, before any text is read.
    assert!(parse_bounded(&twice, 64).unwrap_err().to_string().contains("another compressed file"));
    assert!(rasterize_bounded(&twice, 1.0, 64).is_err());
    assert!(bounded_canvas_bounds(&twice, 64).is_err());
    for mime in ["image/svg+xml", "text/plain"] {
        let single = image_svg(&embedded_image(mime, &once));
        assert!(parse_bounded(single.as_bytes(), 64).is_ok());
        // Inflated once, the doubly compressed image is not SVG text: it is left out, never
        // inflated a second time.
        let nested = image_svg(&embedded_image(mime, &twice));
        assert!(!parse_bounded(nested.as_bytes(), 64).unwrap().tree.root().has_children());
    }
    assert!(parse_bounded(b"\xff\xfe", 64).is_err(), "invalid XML encoding is refused");
}

/// An embedded image the bounded resolver can't charge (an unknown MIME type, a format resvg
/// doesn't draw, a PNG under another MIME type, a corrupt PNG) is left out as usvg's own
/// resolver leaves it out, and the rest of the drawing still renders.
#[test]
fn unreadable_embedded_images_are_left_out_and_the_drawing_still_renders() {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, vec![255; 12]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    for (mime, bytes) in [
        ("image/x-foo", png.clone()),
        ("application/octet-stream", png.clone()),
        ("application/octet-stream", png_dimensions(6000, 5000)),
        ("image/png", png_dimensions(0, 4)),
        ("image/bmp", b"BM\0\0\0\0".to_vec()),
        ("image/png", png[..20].to_vec()),
    ] {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"4\" height=\"4\">{}<rect x=\"2\" width=\"2\" height=\"4\" fill=\"#ff0000\"/></svg>",
            embedded_image(mime, &bytes)
        );
        let buf = rasterize_bounded(svg.as_bytes(), 1.0, MAX_RASTER_PIXELS).unwrap_or_else(|e| panic!("{mime}: {e}"));
        let at = |x: usize, y: usize| buf.px[y * 4 + x];
        assert!(at(3, 1)[0] > 0.99 && at(3, 1)[3] > 0.99, "{mime}: the rect still draws");
        assert!(import_svg_bounded("clipboard", svg.as_bytes(), MAX_RASTER_PIXELS).is_ok(), "{mime}");
    }
}

/// resvg reuses one scratch surface per level of nested SVG images, so side-by-side SVG
/// images (icons in a layout) cost one drawing-sized surface, not one each, and a drawing
/// rendered at up to the 24 MP output limit with one level of them still renders.
#[test]
fn side_by_side_svg_images_need_one_scratch_surface() {
    let icon = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"10\" height=\"10\"><rect width=\"10\" height=\"10\" fill=\"#0000ff\"/></svg>";
    let images: String = (0..12)
        .map(|i| {
            let data = icon.bytes().map(|byte| format!("%{byte:02X}")).collect::<String>();
            format!("<image x=\"{}\" width=\"100\" height=\"100\" href=\"data:image/svg+xml,{data}\"/>", i * 150)
        })
        .collect();
    let layout = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"1920\" height=\"1080\">{images}</svg>");
    assert!(parse_bounded(layout.as_bytes(), 24_000_000).is_ok(), "12 icons on a 2 MP canvas");
    let buf = rasterize_bounded(layout.as_bytes(), 2.0, 24_000_000).unwrap();
    assert_eq!((buf.rect.width(), buf.rect.height()), (3840, 2160));
    assert!(buf.px[100 * 3840 + 100][2] > 0.99, "the first icon draws");
    let one = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"400\" height=\"300.1\">{}</svg>", &images[..images.find("/>").unwrap() + 2]);
    let big = rasterize_bounded(one.as_bytes(), 10.0, 24_000_000).unwrap();
    assert_eq!((big.rect.width(), big.rect.height()), (4000, 3001), "a 12 MP render with one SVG image");
}

/// usvg parses an embedded SVG on top of the drawing's own recursion: 511 + 511 levels, each fine
/// alone, overflowed a 1 MB stack. The bounded path shares [`MAX_XML_DEPTH`] between them too.
#[test]
fn bounded_embedded_svg_gets_only_the_nesting_depth_the_drawing_left() {
    let nested = |depth: usize| {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\">{}<rect width=\"8\" height=\"8\"/>{}</svg>",
            "<g>".repeat(depth),
            "</g>".repeat(depth)
        )
    };
    let embed = move |outer: usize, inner: usize| {
        let data = nested(inner).bytes().map(|byte| format!("%{byte:02X}")).collect::<String>();
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\">{}<image width=\"8\" height=\"8\" href=\"data:image/svg+xml,{data}\"/>{}</svg>",
            "<g>".repeat(outer),
            "</g>".repeat(outer)
        )
    };
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let deep = embed(MAX_XML_DEPTH - 2, MAX_XML_DEPTH - 2);
            let buf = rasterize_bounded(deep.as_bytes(), 1.0, MAX_RASTER_PIXELS).unwrap();
            assert!(buf.px[4 * 8 + 4][3] < 0.01, "the too-deep image is left out");
            let fine = embed(200, 200);
            let buf = rasterize_bounded(fine.as_bytes(), 1.0, MAX_RASTER_PIXELS).unwrap();
            assert!(buf.px[4 * 8 + 4][3] > 0.99, "within the shared budget the image draws");
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Each level of embedded SVG image adds the parser's and renderer's frames on top of the one it
/// sits in. Every chain the bounded path takes (base64 grows each level, so the XML byte budget
/// ends it before [`MAX_EMBEDDED_SVG_DEPTH`]) parses and renders on a 1 MB thread stack (Windows'
/// main-thread size); deeper ones are refused.
#[test]
fn the_deepest_chain_of_embedded_svg_images_fits_a_small_stack() {
    fn base64(bytes: &[u8]) -> String {
        const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let mut out = String::new();
        for chunk in bytes.chunks(3) {
            let n = chunk.iter().enumerate().fold(0u32, |n, (i, b)| n | u32::from(*b) << (16 - 8 * i));
            for i in 0..4 {
                out.push(if i <= chunk.len() { T[(n >> (18 - 6 * i) & 63) as usize] as char } else { '=' });
            }
        }
        out
    }
    let chain = |levels: usize| {
        let mut svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"><rect width=\"8\" height=\"8\"/></svg>".to_string();
        for _ in 0..levels {
            svg = format!(
                "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"8\" height=\"8\"><image width=\"8\" height=\"8\" href=\"data:image/svg+xml;base64,{}\"/></svg>",
                base64(svg.as_bytes())
            );
        }
        svg
    };
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let mut accepted = 0;
            for levels in 1..=MAX_EMBEDDED_SVG_DEPTH + 1 {
                match rasterize_bounded(chain(levels).as_bytes(), 1.0, MAX_RASTER_PIXELS) {
                    Ok(buf) => {
                        assert!(buf.px[4 * 8 + 4][3] > 0.99, "{levels} levels: the innermost rect draws through every level");
                        accepted = levels;
                    }
                    Err(_) => assert!(accepted > 0, "a one-level chain renders"),
                }
            }
            assert!((20..=MAX_EMBEDDED_SVG_DEPTH).contains(&accepted), "deepest accepted chain: {accepted}");
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Only a stored display preview is strict about an embedded image it can't read; an edited SVG
/// saved back from an external editor leaves it out, as placement does.
#[test]
fn only_stored_previews_refuse_unreadable_embedded_images() {
    let svg = image_svg(&embedded_image("image/bmp", b"BM\0\0\0\0"));
    assert!(import_svg_bounded_with_work("edit", svg.as_bytes(), MAX_RASTER_PIXELS).is_ok());
    assert!(import_svg_preview_with_work("preview", svg.as_bytes(), MAX_RASTER_PIXELS).is_err());
}

fn drawing(w: u32, h: u32, body: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\"><defs><filter id=\"f\"><feOffset dx=\"1\"/></filter><pattern id=\"p\" patternUnits=\"userSpaceOnUse\" width=\"TILE\" height=\"TILE\"><rect width=\"4\" height=\"4\"/></pattern></defs>{body}</svg>"
    )
}

fn pixel_layers(r: &ImportResult) -> usize {
    r.document.layers.iter().filter(|l| matches!(l.content, LayerContent::Raster(_))).count()
}

/// resvg gives every filtered group a layer up to 5 x 5 times the pixmap, whatever the pixmap's
/// own 24 MP limit: a 4000 x 4000 canvas with one huge filtered rect asked for about 1.6 GB.
#[test]
fn a_huge_filtered_group_is_dropped_not_allocated() {
    let svg = drawing(4000, 4000, "<g filter=\"url(#f)\"><rect x=\"-100000\" y=\"-100000\" width=\"200000\" height=\"200000\"/></g>");
    let r = import_svg("hostile", svg.as_bytes()).unwrap();
    assert_eq!(pixel_layers(&r), 0);
    assert!(r.warnings.iter().any(|w| w.contains("too much memory")), "{:?}", r.warnings);
    assert!(rasterize(svg.as_bytes(), 1.0).is_err());
}

/// A pattern tile is sized from the document (`width × height × scale`); 100,000 px a side was a
/// 40 GB allocation, which aborts the process when it fails.
#[test]
fn a_huge_pattern_tile_is_refused_not_allocated() {
    let svg = drawing(100, 100, "<rect width=\"100\" height=\"100\" fill=\"url(#p)\"/>").replace("TILE", "100000");
    let err = rasterize(svg.as_bytes(), 1.0).unwrap_err().to_string();
    assert!(err.contains("can't rasterise"), "{err}");
    let r = import_svg("hostile", svg.as_bytes()).unwrap();
    assert_eq!(pixel_layers(&r), 0);
    assert!(r.warnings.iter().any(|w| w.contains("too much memory")), "{:?}", r.warnings);
}

/// A scale factor multiplies the tile too.
#[test]
fn a_pattern_tile_scaled_up_past_the_limit_is_refused() {
    // 20 px at 64x is a 1280 px canvas, well inside the 24 MP limit; the 1000 px tile becomes 64,000 px.
    let svg = drawing(20, 20, "<rect width=\"20\" height=\"20\" fill=\"url(#p)\"/>").replace("TILE", "1000");
    assert!(rasterize(svg.as_bytes(), 1.0).is_ok());
    let err = rasterize(svg.as_bytes(), 64.0).unwrap_err().to_string();
    assert!(err.contains("can't rasterise"), "{err}");
}

/// Ordinary filtered groups and patterns, including a large canvas, still rasterise.
#[test]
fn ordinary_filters_and_patterns_still_rasterise() {
    let svg = drawing(4000, 3000, "<g filter=\"url(#f)\"><rect x=\"100\" y=\"100\" width=\"3000\" height=\"2000\"/></g>");
    let r = import_svg("big", svg.as_bytes()).unwrap();
    assert_eq!(pixel_layers(&r), 1, "{:?}", r.warnings);
    assert!(!r.warnings.iter().any(|w| w.contains("too much memory")));
    let svg = drawing(512, 512, "<rect width=\"512\" height=\"512\" fill=\"url(#p)\"/>").replace("TILE", "64");
    let buf = rasterize(svg.as_bytes(), 1.0).unwrap();
    assert!(buf.px.iter().any(|p| p[3] > 0.99));
}

fn tile_svg(tile: &str, body: &str) -> String {
    drawing(100, 100, body).replace("TILE", tile)
}

fn too_big(svg: &str) {
    let err = rasterize(svg.as_bytes(), 1.0).unwrap_err().to_string();
    assert!(err.contains("can't rasterise"), "{err}");
}

/// The filter region is not clamped by resvg: a flood fills all of it.
#[test]
fn a_huge_filter_region_is_refused() {
    let svg = tile_svg(
        "4",
        "<filter id=\"r\" x=\"0\" y=\"0\" width=\"100000\" height=\"100000\" filterUnits=\"userSpaceOnUse\"><feFlood/></filter><rect width=\"10\" height=\"10\" filter=\"url(#r)\"/>",
    );
    too_big(&svg);
}

/// A default region follows the element, so a huge element makes a huge region for turbulence.
#[test]
fn a_huge_element_with_a_region_sized_primitive_is_refused() {
    let svg = tile_svg("4", "<filter id=\"t\"><feTurbulence baseFrequency=\"0.1\"/></filter><rect width=\"200000\" height=\"200000\" filter=\"url(#t)\"/>");
    too_big(&svg);
}

/// What an feImage draws is walked too.
#[test]
fn a_hostile_pattern_inside_fe_image_is_refused() {
    let svg = tile_svg(
        "100000",
        "<defs><g id=\"g\"><rect width=\"100\" height=\"100\" fill=\"url(#p)\"/></g></defs><filter id=\"i\" x=\"0\" y=\"0\" width=\"200\" height=\"200\" filterUnits=\"userSpaceOnUse\"><feImage href=\"#g\"/></filter><rect width=\"10\" height=\"10\" filter=\"url(#i)\"/>",
    );
    too_big(&svg);
}

/// An embedded SVG image is drawn by resvg on its own pixmap, with its own drawing inside.
#[test]
fn a_hostile_pattern_inside_an_embedded_svg_image_is_refused() {
    let inner = tile_svg("100000", "<rect width=\"100\" height=\"100\" fill=\"url(#p)\"/>");
    let svg = image_svg(&embedded_image("image/svg+xml", inner.as_bytes())).replacen("width=\"1\" height=\"1\"", "width=\"100\" height=\"100\"", 2);
    too_big(&svg);
}

/// Each link of a clip-path chain, and of a mask-on-mask chain, needs a layer while the next is applied.
#[test]
fn long_clip_and_mask_chains_are_refused() {
    let n = 150;
    let mut clips = String::new();
    let mut masks = String::new();
    for i in 0..n {
        let next = if i + 1 < n { format!(" clip-path=\"url(#c{})\"", i + 1) } else { String::new() };
        clips.push_str(&format!("<clipPath id=\"c{i}\"{next}><rect width=\"2000\" height=\"2000\"/></clipPath>"));
        let next = if i + 1 < n { format!(" mask=\"url(#m{})\"", i + 1) } else { String::new() };
        masks.push_str(&format!("<mask id=\"m{i}\" maskUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"40\" height=\"40\"{next}><rect width=\"2000\" height=\"2000\" fill=\"#fff\"/></mask>"));
    }
    for (defs, attr) in [(clips, "clip-path=\"url(#c0)\""), (masks, "mask=\"url(#m0)\"")] {
        let svg = format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2000\" height=\"2000\"><defs>{defs}</defs><rect width=\"2000\" height=\"2000\" {attr}/></svg>"
        );
        too_big(&svg);
    }
    // A short chain is fine.
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"200\"><defs><clipPath id=\"a\" clip-path=\"url(#b)\"><rect width=\"100\" height=\"100\"/></clipPath><clipPath id=\"b\"><rect width=\"80\" height=\"80\"/></clipPath></defs><rect width=\"200\" height=\"200\" clip-path=\"url(#a)\"/></svg>";
    assert!(rasterize(svg.as_bytes(), 1.0).is_ok());
}

/// A mask chain that ends in a chain the walk already measured is still one long chain: the cached
/// length must be checked against the cap too, not only the links walked on the way in.
#[test]
fn a_mask_chain_that_joins_a_cached_chain_is_still_capped() {
    let mut defs = String::new();
    // `t0..t14` is 15 links long, under the cap of 16.
    for i in 0..15 {
        let next = if i + 1 < 15 { format!(" mask=\"url(#t{})\"", i + 1) } else { String::new() };
        defs.push_str(&format!("<mask id=\"t{i}\" maskUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"40\" height=\"40\"{next}><rect width=\"20\" height=\"20\" fill=\"#fff\"/></mask>"));
    }
    // `m0..m9` is 10 links that end in `t0`: 25 links in all.
    for i in 0..10 {
        let next = if i + 1 < 10 { format!(" mask=\"url(#m{})\"", i + 1) } else { " mask=\"url(#t0)\"".to_string() };
        defs.push_str(&format!("<mask id=\"m{i}\" maskUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"40\" height=\"40\"{next}><rect width=\"20\" height=\"20\" fill=\"#fff\"/></mask>"));
    }
    let uses = |first: &str, second: &str| {
        format!(
            "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"40\" height=\"40\"><defs>{defs}</defs><rect width=\"20\" height=\"20\" mask=\"url(#{first})\"/><rect x=\"20\" width=\"20\" height=\"20\" mask=\"url(#{second})\"/></svg>"
        )
    };
    // The short chain alone is fine, and so is the cached chain used twice.
    assert!(rasterize(uses("t0", "t0").as_bytes(), 1.0).is_ok());
    // The long chain, measured after the cached one, is refused.
    too_big(&uses("t0", "m0"));
    // And refused on its own too.
    too_big(&uses("m0", "m0"));
}

/// The estimate itself has a depth limit, so a deep drawing cannot overflow its own walk.
#[test]
fn a_very_deep_drawing_is_refused_by_the_estimate() {
    let svg = tile_svg("4", &format!("{}<rect width=\"10\" height=\"10\"/>{}", "<g opacity=\"0.9\">".repeat(300), "</g>".repeat(300)));
    std::thread::Builder::new().stack_size(64 << 20).spawn(move || too_big(&svg)).unwrap().join().unwrap();
}

#[test]
fn an_unknown_size_counts_as_unbounded() {
    assert_eq!(limits::px(f64::NAN), f64::INFINITY);
    assert_eq!(limits::px(2.0), 2.0);
}

/// Clips, masks, filters and patterns of ordinary size still draw.
#[test]
fn ordinary_clips_masks_and_filters_still_rasterise() {
    let svg = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"200\" height=\"200\"><defs><clipPath id=\"c\"><rect width=\"100\" height=\"100\"/></clipPath><mask id=\"m\"><rect width=\"200\" height=\"200\" fill=\"#fff\"/></mask><filter id=\"f\"><feFlood flood-color=\"red\"/><feGaussianBlur stdDeviation=\"2\"/></filter></defs><rect width=\"200\" height=\"200\" clip-path=\"url(#c)\" mask=\"url(#m)\"/><rect x=\"20\" y=\"20\" width=\"50\" height=\"50\" filter=\"url(#f)\"/></svg>";
    let buf = rasterize(svg.as_bytes(), 1.0).unwrap();
    assert!(buf.px[10 * 200 + 10][3] > 0.99);
    assert!(buf.px[150 * 200 + 150][3] < 0.01);
}

fn parsed(svg: &str) -> usvg::Tree {
    parse(svg.as_bytes()).unwrap().tree
}

/// The limits check alone, for drawings too big to render in a test.
fn check_only(svg: &str, w: u32, h: u32) -> Result<(), limits::Refusal> {
    limits::check(parsed(svg).root().children(), tiny_skia::Transform::identity(), w, h, 0)
}

/// resvg keeps every primitive's result: a flood and 20 offsets over a 15,000 px region is about 19 GB.
#[test]
fn many_primitives_over_a_huge_region_are_refused() {
    let offsets = "<feOffset dx=\"1\"/>".repeat(20);
    let svg = tile_svg(
        "4",
        &format!(
            "<filter id=\"r\" x=\"0\" y=\"0\" width=\"15000\" height=\"15000\" filterUnits=\"userSpaceOnUse\"><feFlood/>{offsets}</filter><rect width=\"10\" height=\"10\" filter=\"url(#r)\"/>"
        ),
    );
    too_big(&svg);
}

/// A 100-link chain of `use` elements each carrying a clip-path, inside a clipPath.
#[test]
fn a_chain_of_uses_with_clip_paths_inside_a_clip_path_is_refused() {
    let mut defs = String::from("<clipPath id=\"k\"><rect width=\"6000\" height=\"4000\"/></clipPath><rect id=\"u0\" width=\"6000\" height=\"4000\"/>");
    for i in 1..100 {
        defs.push_str(&format!("<use id=\"u{i}\" href=\"#u{}\" clip-path=\"url(#k)\"/>", i - 1));
    }
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"6000\" height=\"4000\"><defs>{defs}<clipPath id=\"c\"><use href=\"#u99\"/></clipPath></defs><rect width=\"6000\" height=\"4000\" clip-path=\"url(#c)\"/></svg>"
    );
    let err = check_only(&svg, 6000, 4000).unwrap_err();
    assert!(!err.complex && err.message.contains("deeply"), "{err:?}");
}

/// Seven thousand paths sharing one pattern and one clip are walked once, not once per use.
#[test]
fn many_paths_sharing_a_pattern_are_accepted() {
    let tile: String = (0..300).map(|i| format!("<rect x=\"{i}\" width=\"1\" height=\"1\"/>")).collect();
    let paths: String = (0..7000).map(|i| format!("<rect x=\"{}\" width=\"2\" height=\"2\" fill=\"url(#q)\" clip-path=\"url(#c)\"/>", i % 400)).collect();
    let svg = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"500\" height=\"500\"><defs><pattern id=\"q\" patternUnits=\"userSpaceOnUse\" width=\"300\" height=\"2\">{tile}</pattern><clipPath id=\"c\"><rect width=\"500\" height=\"500\"/></clipPath></defs>{paths}</svg>"
    );
    check_only(&svg, 500, 500).unwrap();
}

/// What realistic documents on a 24 MP canvas cost, against the 1.5 GiB limit: all accepted.
#[test]
fn realistic_effects_on_a_24_megapixel_canvas_are_accepted() {
    let drawing = |defs: &str, body: &str| format!("<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"6000\" height=\"4000\"><defs>{defs}</defs>{body}</svg>");
    let shadow = "<filter id=\"s\" x=\"-0.1\" y=\"-0.1\" width=\"1.2\" height=\"1.2\"><feFlood flood-color=\"#000\" result=\"a\"/><feComposite in=\"a\" in2=\"SourceGraphic\" operator=\"in\" result=\"b\"/><feGaussianBlur in=\"b\" stdDeviation=\"6\" result=\"c\"/><feOffset dx=\"5\" dy=\"5\" result=\"d\"/><feComposite in=\"SourceGraphic\" in2=\"d\" operator=\"over\"/></filter>";
    let full = "<rect width=\"6000\" height=\"4000\"/>";
    // An Inkscape-style drop shadow on a group covering the whole canvas.
    check_only(&drawing(shadow, &format!("<g filter=\"url(#s)\">{full}</g>")), 6000, 4000).unwrap();
    // Two nested shadow groups.
    check_only(&drawing(shadow, &format!("<g filter=\"url(#s)\"><g filter=\"url(#s)\">{full}</g></g>")), 6000, 4000).unwrap();
    // A CSS blur on the whole canvas.
    check_only(&drawing("", &format!("<g filter=\"blur(8)\">{full}</g>")), 6000, 4000).unwrap();
    // A shadow plus a clip and a mask on the same group.
    let defs = format!(
        "{shadow}<clipPath id=\"c\"><rect width=\"5000\" height=\"3000\"/></clipPath><mask id=\"m\"><rect width=\"6000\" height=\"4000\" fill=\"#fff\"/></mask>"
    );
    check_only(&drawing(&defs, &format!("<g filter=\"url(#s)\" clip-path=\"url(#c)\" mask=\"url(#m)\">{full}</g>")), 6000, 4000).unwrap();
}

const NS: &str = "xmlns=\"http://www.w3.org/2000/svg\"";

/// A shared userSpaceOnUse pattern is one definition: the same filter inside it costs far more
/// when it is used at scale 300 than at scale 1, and the second use must not be skipped.
#[test]
fn a_shared_pattern_used_again_at_a_larger_scale_is_refused() {
    let floods = "<feFlood/>".repeat(64);
    let svg = format!(
        "<svg {NS} width=\"6000\" height=\"4000\"><defs><filter id=\"f\" x=\"-10\" y=\"-10\" width=\"20\" height=\"20\" filterUnits=\"userSpaceOnUse\">{floods}</filter><pattern id=\"P\" patternUnits=\"userSpaceOnUse\" width=\"1\" height=\"1\"><rect width=\"1\" height=\"1\" filter=\"url(#f)\"/></pattern></defs><rect width=\"10\" height=\"10\" fill=\"url(#P)\"/><g transform=\"scale(300)\"><rect width=\"0.01\" height=\"0.01\" fill=\"url(#P)\"/></g></svg>"
    );
    let err = check_only(&svg, 6000, 4000).unwrap_err();
    assert!(err.message.contains("a filter with 64 primitives"), "{err:?}");
}

/// Patterns inside patterns, each with 190 nested layers, referenced deepest first: every
/// definition is walked once, but the depth adds up where they are used.
#[test]
fn nested_shared_patterns_add_up_their_depth() {
    let k = 50;
    let mut defs = String::new();
    for i in 0..k {
        let fill = if i + 1 < k { format!("url(#D{})", i + 1) } else { "#000".to_string() };
        defs.push_str(&format!(
            "<pattern id=\"D{i}\" patternUnits=\"userSpaceOnUse\" width=\"4\" height=\"4\">{}<rect width=\"4\" height=\"4\" fill=\"{fill}\"/>{}</pattern>",
            "<g opacity=\"0.5\">".repeat(190),
            "</g>".repeat(190)
        ));
    }
    let body: String = (0..k).rev().map(|i| format!("<rect width=\"10\" height=\"10\" fill=\"url(#D{i})\"/>")).collect();
    let svg = format!("<svg {NS} width=\"100\" height=\"100\"><defs>{defs}</defs>{body}</svg>");
    // 9,500 nested groups: usvg's conversion overflows the 2 MiB test-thread stack on Windows.
    let msg = std::thread::Builder::new()
        .stack_size(64 << 20)
        .spawn(move || match parse(svg.as_bytes()) {
            Err(e) => e.to_string(),
            Ok(p) => limits::check(p.tree.root().children(), tiny_skia::Transform::identity(), 100, 100, 0).unwrap_err().message,
        })
        .unwrap()
        .join()
        .unwrap();
    assert!(msg.contains("layers"), "{msg}");
}

/// usvg makes the layer the union of a group's filter regions and every unary primitive on it keeps
/// a layer-sized result, however small its own region is.
#[test]
fn a_small_filter_after_a_big_one_is_charged_at_the_layer_size() {
    let matrices = "<feColorMatrix in=\"SourceGraphic\" type=\"saturate\" values=\"0.5\"/>".repeat(64);
    let svg = format!(
        "<svg {NS} width=\"6000\" height=\"4000\"><defs><filter id=\"big\" x=\"0\" y=\"0\" width=\"6000\" height=\"4000\" filterUnits=\"userSpaceOnUse\"><feOffset dx=\"1\"/></filter><filter id=\"small\" x=\"0\" y=\"0\" width=\"1\" height=\"1\" filterUnits=\"userSpaceOnUse\">{matrices}</filter></defs><rect width=\"6000\" height=\"4000\" filter=\"url(#big) url(#small)\"/></svg>"
    );
    let err = check_only(&svg, 6000, 4000).unwrap_err();
    assert!(err.message.contains("filter"), "{err:?}");
}

/// What an feImage draws sees the results the filter kept before it: 20 floods and an feImage
/// whose pattern tile fits on its own.
#[test]
fn fe_image_content_sees_the_results_kept_before_it() {
    let floods = "<feFlood/>".repeat(20);
    let svg = format!(
        "<svg {NS} width=\"4000\" height=\"4000\"><defs><pattern id=\"p\" patternUnits=\"userSpaceOnUse\" width=\"7000\" height=\"7000\"><rect width=\"4\" height=\"4\"/></pattern><g id=\"g\"><rect width=\"100\" height=\"100\" fill=\"url(#p)\"/></g><filter id=\"i\" x=\"0\" y=\"0\" width=\"4000\" height=\"4000\" filterUnits=\"userSpaceOnUse\">{floods}<feImage href=\"#g\"/></filter></defs><rect width=\"10\" height=\"10\" filter=\"url(#i)\"/></svg>"
    );
    let err = check_only(&svg, 4000, 4000).unwrap_err();
    assert!(err.message.contains("pattern tile"), "{err:?}");
}

/// Mask content is drawn while the group's layer, the mask pixmap and its 1 byte per pixel mask are alive.
#[test]
fn mask_content_is_charged_with_the_mask_buffers() {
    let svg = format!(
        "<svg {NS} width=\"2000\" height=\"2000\"><defs><filter id=\"b\" x=\"0\" y=\"0\" width=\"9740\" height=\"9740\" filterUnits=\"userSpaceOnUse\"><feGaussianBlur stdDeviation=\"8\"/></filter><mask id=\"m\" maskUnits=\"userSpaceOnUse\" x=\"0\" y=\"0\" width=\"2000\" height=\"2000\"><g filter=\"url(#b)\"><rect width=\"100\" height=\"100\"/></g></mask></defs><rect width=\"100000\" height=\"100000\" mask=\"url(#m)\"/></svg>"
    );
    let err = check_only(&svg, 2000, 2000).unwrap_err();
    assert!(err.message.contains("a filter with 1 primitives over a region of 94867600 px"), "{err:?}");
}

/// A blur under 2 px sigma is an IIR blur with f64 scratch, charged one buffer more than a box blur.
#[test]
fn an_iir_blur_is_charged_more_than_a_box_blur() {
    let blur = |sigma: &str| {
        format!(
            "<svg {NS} width=\"4000\" height=\"4000\"><defs><filter id=\"b\" x=\"0\" y=\"0\" width=\"10480\" height=\"10480\" filterUnits=\"userSpaceOnUse\"><feGaussianBlur stdDeviation=\"{sigma}\"/></filter></defs><rect width=\"10\" height=\"10\" filter=\"url(#b)\"/></svg>"
        )
    };
    check_only(&blur("8"), 4000, 4000).unwrap();
    assert!(check_only(&blur("1"), 4000, 4000).is_err());
}

/// usvg expands a chain of `use` elements into nested groups (it refuses 1,000 of them: "nodes limit
/// reached"); walking 300 must not need more stack than a test thread has.
#[test]
fn a_deep_use_chain_does_not_overflow_the_walk() {
    let mut body = String::from("<g id=\"u0\"><rect width=\"10\" height=\"10\"/></g>");
    for i in 1..300 {
        body.push_str(&format!("<use id=\"u{i}\" href=\"#u{}\"/>", i - 1));
    }
    let svg = format!("<svg {NS} width=\"100\" height=\"100\"><defs>{body}</defs><use href=\"#u299\"/></svg>");
    std::thread::Builder::new().spawn(move || check_only(&svg, 100, 100).unwrap()).unwrap().join().unwrap();
}

/// resvg draws an feImage at the scale of the transform, whatever its skew: skewX(89.9) stretches
/// a 3000 px pattern tile in the image to 3000 x 1.7 million px.
#[test]
fn a_skewed_fe_image_scales_its_pattern_tile() {
    let svg = format!(
        "<svg {NS} width=\"100\" height=\"100\"><defs><pattern id=\"p\" patternUnits=\"userSpaceOnUse\" width=\"3000\" height=\"3000\"><rect width=\"4\" height=\"4\"/></pattern><g id=\"g\"><rect width=\"100\" height=\"100\" fill=\"url(#p)\"/></g><filter id=\"i\" x=\"0\" y=\"0\" width=\"100\" height=\"100\" filterUnits=\"userSpaceOnUse\"><feImage href=\"#g\"/></filter></defs><g transform=\"skewX(89.9)\"><rect width=\"10\" height=\"10\" filter=\"url(#i)\"/></g></svg>"
    );
    let err = check_only(&svg, 100, 100).unwrap_err();
    assert!(err.message.contains("pattern tile"), "{err:?}");
}

#[test]
fn every_raster_refusal_reason_is_reported_once() {
    // One element over a filter-primitive limit, then one over the memory limit: the second must
    // not be dropped silently because the first already reported a refusal.
    let many = "<feOffset/>".repeat(65);
    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="4096" height="4096">
            <filter id="many" x="0" y="0" width="1" height="1">{many}</filter>
            <filter id="big" x="0" y="0" width="1" height="1">{}</filter>
            <rect width="16" height="16" fill="red" filter="url(#many)"/>
            <rect width="4096" height="4096" fill="blue" filter="url(#big)"/>
        </svg>"##,
        "<feGaussianBlur stdDeviation=\"3\"/>".repeat(60)
    );
    let result = import_svg("refusals", svg.as_bytes()).unwrap();
    let dropped = |what: &str| result.warnings.iter().filter(|w| w.contains("was dropped because") && w.contains(what)).count();
    assert_eq!(dropped("rasterising limit"), 1, "{:?}", result.warnings);
    assert_eq!(dropped("too much memory"), 1, "{:?}", result.warnings);
}
