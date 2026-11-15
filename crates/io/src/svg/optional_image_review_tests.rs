use super::*;
use std::io::Write;

fn url(mime: &str, bytes: &[u8]) -> String {
    let mut value = format!("data:{mime},");
    for byte in bytes {
        value.push_str(&format!("%{byte:02X}"));
    }
    value
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    let mut writer = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    writer.write_all(bytes).unwrap();
    writer.finish().unwrap()
}

fn drawing(url: &str) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#ff0000\"/><image width=\"2\" height=\"2\" href=\"{url}\"/></svg>"
    )
}

fn omission_warnings(imported: &ImportResult) -> Vec<&str> {
    imported.warnings.iter().filter(|warning| warning.contains("embedded") && warning.contains("omitted")).map(String::as_str).collect()
}

fn assert_omission_warning(imported: &ImportResult, count: usize) {
    let warnings = omission_warnings(imported);
    assert_eq!(warnings.len(), 1, "all omitted references share one bounded warning");
    assert!(warnings[0].starts_with(&format!("SVG: {count} embedded ")), "{}", warnings[0]);
    assert!(warnings[0].len() < 256);
    assert!(!warnings[0].contains("data:"));
}

fn oversized_png(seed: u8) -> Vec<u8> {
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgb, vec![0, seed, 0]).unwrap();
    let mut png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    png[16..20].copy_from_slice(&40_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&40_000u32.to_be_bytes());
    let checksum = crc32fast::hash(&png[12..29]);
    png[29..33].copy_from_slice(&checksum.to_be_bytes());
    png
}

#[test]
fn ordinary_open_preserves_outer_pixels_when_optional_images_exceed_limits_but_previews_refuse() {
    let deep = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\">{}<rect width=\"2\" height=\"2\" fill=\"#00ff00\"/>{}</svg>",
        "<g>".repeat(MAX_XML_DEPTH + 1),
        "</g>".repeat(MAX_XML_DEPTH + 1)
    );
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgb, vec![0, 255, 0]).unwrap();
    let mut png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    png[16..20].copy_from_slice(&40_000u32.to_be_bytes());
    png[20..24].copy_from_slice(&40_000u32.to_be_bytes());
    let checksum = crc32fast::hash(&png[12..29]);
    png[29..33].copy_from_slice(&checksum.to_be_bytes());
    for source in [url("image/png", &png), url("image/svg+xml", deep.as_bytes()), url("image/svg+xml", &gzip(deep.as_bytes()))] {
        let text = drawing(&source);
        let imported = crate::import("outer.svg", text.as_bytes()).expect("only the over-budget optional image is omitted");
        let pixel = photocraft_compose::flatten(&imported.document).get(1, 1);
        assert_eq!(pixel, [1., 0., 0., 1.], "the valid outer shape survives, and the rejected child is not decoded/drawn");
        assert_omission_warning(&imported, 1);
        assert!(import_svg_preview_with_work("strict.svg", text.as_bytes(), MAX_RASTER_PIXELS).is_err(), "stored previews must still draw completely");
    }
}

#[test]
fn valid_embedded_images_do_not_acquire_omission_warnings() {
    let green = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#00ff00\"/></svg>";
    let image = photocraft_codecs::Image::from_u8(1, 1, photocraft_codecs::ChannelLayout::Rgb, vec![0, 255, 0]).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    for source in [url("image/png", &png), url("image/svg+xml", green.as_bytes()), url("image/svg+xml", &gzip(green.as_bytes()))] {
        let text = drawing(&source);
        let imported = import_svg_with_group_depth("valid.svg", text.as_bytes(), MAX_GROUP_DEPTH).unwrap();
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [0., 1., 0., 1.]);
        assert!(omission_warnings(&imported).is_empty(), "a valid child must not acquire an omission warning");
        import_svg_preview_with_work("valid-preview.svg", text.as_bytes(), MAX_RASTER_PIXELS).unwrap();
    }
}

#[test]
fn ordinary_open_warns_once_for_embedded_svg_byte_and_chain_limits() {
    let green = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#00ff00\"/></svg>";
    let mut oversized = green.as_bytes().to_vec();
    oversized.resize(MAX_BOUNDED_SVG_BYTES as usize + 1, b' ');
    let bytes = drawing(&url("image/svg+xml", &gzip(&oversized)));
    drop(oversized);
    // Compress each level before embedding it; the chain remains tiny rather
    // than multiplying a base64 payload, and it reaches the actual chain cap.
    let mut chain = green.to_string();
    for _ in 0..=MAX_EMBEDDED_SVG_DEPTH {
        chain = drawing(&url("image/svg+xml", &gzip(chain.as_bytes())));
    }
    for text in [bytes, chain] {
        let imported = crate::import("omitted.svg", text.as_bytes()).unwrap();
        assert_omission_warning(&imported, 1); // The resource counter is a subset, not a second omission.
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
        assert!(import_svg_preview_with_work("strict.svg", text.as_bytes(), MAX_RASTER_PIXELS).is_err());
        let saved = photocraft_format::save_to_bytes(&imported.document, &Default::default()).unwrap();
        let reopened = photocraft_format::load_from_bytes(&saved).unwrap();
        assert_eq!(photocraft_compose::flatten(&reopened).get(1, 1), [1., 0., 0., 1.]);
    }
}

#[test]
fn omitted_image_flood_has_one_bounded_warning_even_in_whole_drawing_fallback() {
    let mut text = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\">".to_string();
    for _ in 0..=MAX_NODES {
        text.push_str("<rect width=\"2\" height=\"2\" fill=\"#ff0000\"/>");
    }
    for seed in 0..64 {
        text.push_str(&format!("<image width=\"2\" height=\"2\" href=\"{}\"/>", url("image/png", &oversized_png(seed))));
    }
    text.push_str("</svg>");
    let imported = crate::import("flood.svg", text.as_bytes()).unwrap();
    assert_omission_warning(&imported, 64);
    assert_eq!(imported.document.layers.len(), 1);
    assert!(matches!(imported.document.layers[0].content, LayerContent::Raster(_)));
    assert!(imported.warnings.iter().any(|warning| warning.contains("one pixel layer")), "exercise the whole-drawing fallback");
    assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
}

#[test]
fn bounded_non_strict_xml_omission_warns_while_strict_preview_still_refuses() {
    let child = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\">{}<rect width=\"2\" height=\"2\" fill=\"#00ff00\"/>{}</svg>",
        "<g>".repeat(MAX_XML_DEPTH - 2),
        "</g>".repeat(MAX_XML_DEPTH - 2)
    );
    let text = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#ff0000\"/>{}<image width=\"2\" height=\"2\" href=\"{}\"/>{}</svg>",
        "<g>".repeat(MAX_XML_DEPTH - 2),
        url("image/svg+xml", child.as_bytes()),
        "</g>".repeat(MAX_XML_DEPTH - 2)
    );
    let imported = import_svg_bounded_images("soft.svg", text.as_bytes(), MAX_RASTER_PIXELS).unwrap();
    assert_omission_warning(&imported, 1);
    assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
    assert!(import_svg_preview_with_work("strict.svg", text.as_bytes(), MAX_RASTER_PIXELS).is_err());
}

#[test]
fn raster_only_callers_reject_small_budget_omissions_while_import_keeps_its_warning() {
    fn no_fonts() -> Arc<usvg::fontdb::Database> {
        Arc::new(usvg::fontdb::Database::new())
    }
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let text = drawing(&url("image/png", &png));
    let parsed = || parse_with_image_limit(text.as_bytes(), no_fonts, 3).unwrap();
    assert_eq!(parsed().image_usage.raster_limit_images, 1, "a valid four-pixel PNG exceeds the three-pixel allowance before decode");
    assert!(matches!(complete_raster_tree(parsed()), Err(IoError::Svg(message)) if message.contains("incomplete drawing")));
    let imported = import_parsed("limited.svg", parsed(), MAX_GROUP_DEPTH, None, false).unwrap();
    assert_omission_warning(&imported, 1);
    assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
    let complete = parse_with_image_limit(text.as_bytes(), no_fonts, 4).unwrap();
    assert_eq!(omitted_image_count(complete.image_usage), 0);
    let pixels = rasterize_tree(&complete_raster_tree(complete).unwrap(), 1.).unwrap();
    assert_eq!(pixels.get(1, 1), [0., 1., 0., 1.]);
}

#[test]
fn raster_only_public_apis_reject_omitted_images_instead_of_returning_partial_pixels() {
    let deep = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\">{}<rect width=\"2\" height=\"2\" fill=\"#00ff00\"/>{}</svg>",
        "<g>".repeat(MAX_XML_DEPTH + 1),
        "</g>".repeat(MAX_XML_DEPTH + 1)
    );
    for source in [url("image/png", &oversized_png(0)), url("image/svg+xml", deep.as_bytes()), url("image/png", b"not a PNG")] {
        let text = drawing(&source);
        assert!(matches!(rasterize(text.as_bytes(), 1.), Err(IoError::Svg(message)) if message.contains("incomplete drawing")));
        assert!(matches!(rasterize_rgba8(text.as_bytes()), Err(IoError::Svg(message)) if message.contains("incomplete drawing")));
        let imported = crate::import("outer.svg", text.as_bytes()).unwrap();
        assert_omission_warning(&imported, 1);
        assert_eq!(photocraft_compose::flatten(&imported.document).get(1, 1), [1., 0., 0., 1.]);
    }
}

#[test]
fn raster_only_public_apis_preserve_complete_embedded_image_pixels() {
    let image = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, [0, 255, 0].repeat(4)).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let green = "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"2\" height=\"2\"><rect width=\"2\" height=\"2\" fill=\"#00ff00\"/></svg>";
    for source in [url("image/png", &png), url("image/svg+xml", green.as_bytes())] {
        let text = drawing(&source);
        assert_eq!(rasterize(text.as_bytes(), 1.).unwrap().get(1, 1), [0., 1., 0., 1.]);
        let (width, height, pixels) = rasterize_rgba8(text.as_bytes()).unwrap();
        assert_eq!((width, height), (2, 2));
        assert_eq!(pixels, [0, 255, 0, 255].repeat(4));
    }
}
