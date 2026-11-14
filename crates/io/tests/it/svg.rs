//! SVG import: shapes, groups, transforms, curves, text, gradients, rasterised fallbacks, the
//! size limit, sniffing and malformed input.

use photocraft_color::blend::BlendMode;
use photocraft_doc::vector::{FillRule, LineCap, LineJoin};
use photocraft_doc::{Fill, LayerContent};
use photocraft_io::*;

const NS: &str = "xmlns=\"http://www.w3.org/2000/svg\"";

fn open(svg: &str) -> ImportResult {
    import("drawing.svg", svg.as_bytes()).unwrap_or_else(|e| panic!("{e}\n{svg}"))
}

fn px(doc: &photocraft_doc::Document, x: i32, y: i32) -> [f32; 4] {
    photocraft_compose::flatten(doc).get(x, y)
}

fn close(a: [f32; 4], b: [f32; 4]) -> bool {
    a.iter().zip(b).all(|(x, y)| (x - y).abs() < 0.03)
}

#[test]
fn basic_shapes_become_shape_layers_with_their_fills_and_strokes() {
    let r = open(&format!(
        "<svg {NS} width=\"100\" height=\"60\">\
           <rect id=\"box\" x=\"10\" y=\"10\" width=\"30\" height=\"20\" fill=\"#ff0000\"/>\
           <circle cx=\"70\" cy=\"30\" r=\"15\" fill=\"rgb(0,0,255)\" stroke=\"#000\" stroke-width=\"4\" stroke-linecap=\"round\" stroke-linejoin=\"bevel\"/>\
         </svg>"
    ));
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (100, 60));
    assert_eq!(d.layers.len(), 2, "{:?}", d.layers.iter().map(|l| &l.name).collect::<Vec<_>>());
    assert_eq!(d.layers[0].name, "box");
    assert_eq!(d.layers[1].name, "Shape 1", "unnamed shapes are numbered");
    let LayerContent::Shape(sh) = &d.layers[0].content else { panic!("not a shape") };
    assert_eq!(sh.fill, Some(Fill::Solid(photocraft_color::Color::rgba(1.0, 0.0, 0.0, 1.0))));
    assert!(sh.stroke.is_none());
    assert_eq!(sh.path.subpaths.len(), 1);
    assert!(sh.path.subpaths[0].closed);
    let LayerContent::Shape(c) = &d.layers[1].content else { panic!("not a shape") };
    let st = c.stroke.as_ref().expect("stroke");
    assert!((st.width - 4.0).abs() < 1e-3);
    assert_eq!((st.cap, st.join), (LineCap::Round, LineJoin::Bevel));
    assert_eq!(st.paint, Fill::Solid(photocraft_color::Color::rgba(0.0, 0.0, 0.0, 1.0)));
    // The composite: red inside the box, blue inside the circle, black on its edge, nothing else.
    assert!(close(px(d, 20, 20), [1.0, 0.0, 0.0, 1.0]), "{:?}", px(d, 20, 20));
    assert!(close(px(d, 70, 30), [0.0, 0.0, 1.0, 1.0]), "{:?}", px(d, 70, 30));
    assert!(close(px(d, 55, 30), [0.0, 0.0, 0.0, 1.0]), "{:?}", px(d, 55, 30));
    assert!(px(d, 3, 3)[3] < 0.01 && px(d, 95, 55)[3] < 0.01);
    assert!(r.warnings.is_empty(), "{:?}", r.warnings);
}

#[test]
fn groups_keep_opacity_blend_and_nesting_while_plain_groups_vanish() {
    let r = open(&format!(
        "<svg {NS} width=\"40\" height=\"40\">\
           <g transform=\"translate(5,5)\">\
             <g id=\"art\" opacity=\"0.5\" style=\"mix-blend-mode:multiply\">\
               <rect width=\"10\" height=\"10\" fill=\"#00ff00\"/>\
               <g id=\"inner\"><rect x=\"10\" width=\"10\" height=\"10\" fill=\"#0000ff\"/></g>\
             </g>\
           </g>\
         </svg>"
    ));
    let d = &r.document;
    assert_eq!(d.layers.len(), 1, "the transform-only group is not a layer");
    let art = &d.layers[0];
    assert_eq!(art.name, "art");
    assert!((art.opacity - 0.5).abs() < 1e-6);
    assert_eq!(art.blend, BlendMode::Multiply);
    let LayerContent::Group(g) = &art.content else { panic!("not a group") };
    assert_eq!(g.children.len(), 2);
    assert!(matches!(g.children[0].content, LayerContent::Shape(_)));
    assert_eq!(g.children[1].name, "inner");
    // The translate was baked into the paths: the green square now starts at (5, 5).
    assert!(px(d, 3, 3)[3] < 0.01);
    let p = px(d, 7, 7);
    assert!(p[1] > 0.9 && (p[3] - 0.5).abs() < 0.03, "{p:?}");
}

#[test]
fn svg_group_rasterization_threshold_is_configurable_and_clamped() {
    let source = format!(
        "<svg {NS} width=\"20\" height=\"20\"><g id=\"outer\"><rect width=\"2\" height=\"2\" fill=\"blue\"/><g id=\"inner\"><rect width=\"10\" height=\"10\" fill=\"red\"/><rect x=\"12\" width=\"5\" height=\"5\" fill=\"green\"/></g></g></svg>"
    );
    let shallow = import_with_svg_group_depth("nested.svg", source.as_bytes(), 1).unwrap();
    let LayerContent::Group(outer) = &shallow.document.layers[0].content else { panic!("outer group should remain editable") };
    assert!(outer.children.iter().any(|child| matches!(child.content, LayerContent::Raster(_))), "the subtree past the setting should be rasterized");
    assert!(shallow.warnings.iter().any(|warning| warning.contains("nested too deeply")));

    let unlimited = import_with_svg_group_depth("nested.svg", source.as_bytes(), 0).unwrap();
    let LayerContent::Group(outer) = &unlimited.document.layers[0].content else { panic!("outer group should remain editable") };
    assert!(outer.children.iter().any(|child| matches!(child.content, LayerContent::Group(_))), "zero disables the preference threshold");

    let clamped = import_with_svg_group_depth("nested.svg", source.as_bytes(), usize::MAX).unwrap();
    assert!(matches!(clamped.document.layers[0].content, LayerContent::Group(_)), "out-of-range settings clamp to the document's hard group limit");
}

#[test]
fn transforms_curves_and_fill_rules_are_in_the_knots() {
    let r = open(&format!(
        "<svg {NS} width=\"60\" height=\"60\">\
           <g transform=\"translate(10,20) scale(2)\"><rect width=\"5\" height=\"5\" fill=\"black\"/></g>\
           <path d=\"M30 10 Q 40 0 50 10 L 50 20 Z\" fill=\"black\"/>\
           <path d=\"M0 40 h20 v20 h-20 z M5 45 h10 v10 h-10 z\" fill=\"black\" fill-rule=\"evenodd\"/>\
         </svg>"
    ));
    let d = &r.document;
    let LayerContent::Shape(rect) = &d.layers[0].content else { panic!() };
    let xs: Vec<f64> = rect.path.subpaths[0].knots.iter().map(|k| k.anchor.x).collect();
    let ys: Vec<f64> = rect.path.subpaths[0].knots.iter().map(|k| k.anchor.y).collect();
    assert_eq!((xs.iter().cloned().fold(f64::MAX, f64::min), xs.iter().cloned().fold(f64::MIN, f64::max)), (10.0, 20.0));
    assert_eq!((ys.iter().cloned().fold(f64::MAX, f64::min), ys.iter().cloned().fold(f64::MIN, f64::max)), (20.0, 30.0));
    let LayerContent::Shape(curve) = &d.layers[1].content else { panic!() };
    let k = &curve.path.subpaths[0].knots;
    assert_eq!(k.len(), 3);
    // The quadratic became a cubic: the first knot's leaving handle moved off its anchor.
    assert!(k[0].out_ctrl != k[0].anchor && k[1].in_ctrl != k[1].anchor);
    assert!(k[1].out_ctrl == k[1].anchor, "the straight segment keeps retracted handles");
    let LayerContent::Shape(ring) = &d.layers[2].content else { panic!() };
    assert_eq!(ring.path.fill_rule, FillRule::EvenOdd);
    assert_eq!(ring.path.subpaths.len(), 2);
    assert!(px(d, 2, 42)[3] > 0.99 && px(d, 10, 50)[3] < 0.01, "even-odd leaves the hole empty");
}

#[test]
fn gradients_are_approximated_with_a_warning() {
    let r = open(&format!(
        "<svg {NS} width=\"100\" height=\"20\">\
           <defs><linearGradient id=\"g\" x1=\"0\" y1=\"0\" x2=\"1\" y2=\"0\">\
             <stop offset=\"0\" stop-color=\"#ff0000\"/><stop offset=\"1\" stop-color=\"#0000ff\" stop-opacity=\"0.5\"/>\
           </linearGradient></defs>\
           <rect width=\"100\" height=\"20\" fill=\"url(#g)\"/>\
         </svg>"
    ));
    let LayerContent::Shape(sh) = &r.document.layers[0].content else { panic!() };
    let Some(Fill::Gradient { stops, angle, .. }) = &sh.fill else { panic!("{:?}", sh.fill) };
    assert_eq!(stops.len(), 2);
    assert!((stops[1].1.alpha - 0.5).abs() < 1e-6);
    assert!(angle.abs() < 1e-3, "left to right is 0°, got {angle}");
    assert!(r.warnings.iter().any(|w| w.contains("gradient")), "{:?}", r.warnings);
    // Red on the left, bluish on the right.
    let (l, rr) = (px(&r.document, 2, 10), px(&r.document, 97, 10));
    assert!(l[0] > 0.8 && l[2] < 0.2, "{l:?}");
    assert!(rr[2] > rr[0], "{rr:?}");
}

#[test]
fn clip_paths_masks_filters_patterns_and_images_are_rasterised() {
    // A clip of one path becomes a vector mask (#4266); one of two shapes is still rasterised.
    let r = open(&format!(
        "<svg {NS} width=\"60\" height=\"30\">\
           <defs>\
             <clipPath id=\"c\"><rect width=\"10\" height=\"30\"/><rect x=\"26\" width=\"2\" height=\"30\"/></clipPath>\
             <pattern id=\"p\" width=\"4\" height=\"4\" patternUnits=\"userSpaceOnUse\"><rect width=\"2\" height=\"4\" fill=\"black\"/></pattern>\
           </defs>\
           <g id=\"clipped\" clip-path=\"url(#c)\"><rect width=\"30\" height=\"30\" fill=\"#ff0000\"/></g>\
           <rect id=\"striped\" x=\"30\" width=\"30\" height=\"30\" fill=\"url(#p)\"/>\
         </svg>"
    ));
    let d = &r.document;
    assert_eq!(d.layers.len(), 2);
    assert!(matches!(d.layers[0].content, LayerContent::Raster(_)), "clipped group is pixels");
    assert!(matches!(d.layers[1].content, LayerContent::Raster(_)), "pattern paint is pixels");
    assert!(close(px(d, 5, 15), [1.0, 0.0, 0.0, 1.0]), "{:?}", px(d, 5, 15));
    assert!(px(d, 20, 15)[3] < 0.01, "clipped away");
    // 4 px tiles with a 2 px bar from the pattern origin: x = 32 is a bar, x = 34 a gap.
    assert!(px(d, 32, 15)[3] > 0.99 && px(d, 34, 15)[3] < 0.01, "stripes: {:?} {:?}", px(d, 32, 15), px(d, 34, 15));
    assert!(r.warnings.iter().any(|w| w.contains("clip-path")) && r.warnings.iter().any(|w| w.contains("pattern")), "{:?}", r.warnings);
}

/// A 2×2 PNG (red, green / blue, white) as a data URL, for `<image>`.
fn png_data_url() -> String {
    let img = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]).unwrap();
    let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::from("data:image/png;base64,");
    for chunk in bytes.chunks(3) {
        let n = chunk.len();
        let v = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= n {
                out.push(T[((v >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// A small valid PNG whose IHDR claims `w`x`h`, far larger than its actual (tiny) image data.
/// The header alone must be enough to reject it before any decode is attempted.
fn png_data_url_with_claimed_size(w: u32, h: u32) -> String {
    let img = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, vec![255, 0, 0, 0, 255, 0, 0, 0, 255, 255, 255, 255]).unwrap();
    let mut bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    // IHDR starts right after the 8-byte signature and 8-byte chunk length/type: width at 16,
    // height at 20 (big-endian u32 each).
    bytes[16..20].copy_from_slice(&w.to_be_bytes());
    bytes[20..24].copy_from_slice(&h.to_be_bytes());
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::from("data:image/png;base64,");
    for chunk in bytes.chunks(3) {
        let n = chunk.len();
        let v = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= n {
                out.push(T[((v >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[test]
fn embedded_raster_images_become_pixel_layers() {
    let r = open(&format!(
        "<svg {NS} width=\"20\" height=\"20\"><image id=\"photo\" x=\"0\" y=\"0\" width=\"20\" height=\"20\" href=\"{}\" style=\"image-rendering:pixelated\"/></svg>",
        png_data_url()
    ));
    let d = &r.document;
    assert_eq!(d.layers.len(), 1);
    assert_eq!(d.layers[0].name, "photo");
    assert!(matches!(d.layers[0].content, LayerContent::Raster(_)));
    assert!(close(px(d, 3, 3), [1.0, 0.0, 0.0, 1.0]), "{:?}", px(d, 3, 3));
    assert!(close(px(d, 16, 16), [1.0, 1.0, 1.0, 1.0]), "{:?}", px(d, 16, 16));
}

#[test]
fn embedded_png_with_a_huge_claimed_size_is_not_decoded() {
    // The IHDR claims 40000x40000 (6.4 GB RGBA decoded) but the display size is tiny; the
    // header-only check must reject it before resvg ever decodes the actual (2x2) image data.
    let r = open(&format!(
        "<svg {NS} width=\"20\" height=\"20\"><image id=\"photo\" x=\"0\" y=\"0\" width=\"20\" height=\"20\" href=\"{}\"/></svg>",
        png_data_url_with_claimed_size(40_000, 40_000)
    ));
    // Rejected at the resolver: no image layer is produced (resvg treats it as a missing image).
    assert!(r.document.layers.is_empty() || !matches!(r.document.layers[0].content, LayerContent::Raster(_)), "{:?}", r.document.layers);
}

#[test]
fn text_becomes_outline_shapes_or_warns_without_a_font() {
    let r = open(&format!(
        "<svg {NS} width=\"120\" height=\"60\"><text x=\"5\" y=\"45\" font-size=\"48\" font-family=\"Arial, DejaVu Sans, Helvetica, sans-serif\" fill=\"black\">II</text></svg>"
    ));
    let d = &r.document;
    if d.layers.is_empty() {
        assert!(r.warnings.iter().any(|w| w.contains("font")), "{:?}", r.warnings);
        return;
    }
    assert_eq!(d.layers[0].name, "II", "the text names its group");
    let LayerContent::Group(g) = &d.layers[0].content else { panic!("{:?}", d.layers[0].content.kind_name()) };
    assert!(g.children.iter().all(|l| matches!(l.content, LayerContent::Shape(_))));
    let dark = (0..120).filter(|&x| px(d, x, 30)[3] > 0.5).count();
    assert!(dark > 4, "glyph strokes cross the middle row: {dark}");
}

#[test]
fn size_comes_from_the_viewbox_and_huge_drawings_shrink_to_fit() {
    let r = open(&format!("<svg {NS} viewBox=\"0 0 300 150\"><rect width=\"300\" height=\"150\"/></svg>"));
    assert_eq!((r.document.size.width, r.document.size.height), (300, 150));
    let r = open(&format!("<svg {NS} viewBox=\"0 0 100000 50000\"><rect x=\"50000\" width=\"50000\" height=\"50000\" fill=\"black\"/></svg>"));
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (svg::MAX_SIDE, svg::MAX_SIDE / 2));
    assert!(r.warnings.iter().any(|w| w.contains("scaled")), "{:?}", r.warnings);
    // The right half is still the right half.
    let (w, h) = (d.size.width as i32, d.size.height as i32);
    assert!(px(d, w / 4, h / 2)[3] < 0.01 && px(d, 3 * w / 4, h / 2)[3] > 0.99);
}

fn nested(depth: usize) -> String {
    format!("<svg {NS} width=\"8\" height=\"8\">{}<rect width=\"8\" height=\"8\"/>{}</svg>", "<g>".repeat(depth), "</g>".repeat(depth))
}

fn gzip(bytes: &[u8]) -> Vec<u8> {
    use std::io::Write;
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(bytes).unwrap();
    e.finish().unwrap()
}

/// A `data:` URL of `bytes` under `mime`, base64-encoded.
fn base64_data_url(mime: &str, bytes: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = format!("data:{mime};base64,");
    for chunk in bytes.chunks(3) {
        let n = chunk.len();
        let v = (u32::from(chunk[0]) << 16) | (u32::from(*chunk.get(1).unwrap_or(&0)) << 8) | u32::from(*chunk.get(2).unwrap_or(&0));
        for i in 0..4 {
            if i <= n {
                out.push(T[((v >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

#[test]
fn gzip_compressed_embedded_svg_decompresses_once_with_a_size_cap() {
    // A small gzip-compressed SVG embedded via data URL (SVGZ-in-data-URL) must still import.
    let inner = format!("<svg {NS} width=\"4\" height=\"4\"><rect width=\"4\" height=\"4\" fill=\"#00ff00\"/></svg>");
    let url = base64_data_url("image/svg+xml", &gzip(inner.as_bytes()));
    let r = open(&format!("<svg {NS} width=\"10\" height=\"10\"><image x=\"0\" y=\"0\" width=\"10\" height=\"10\" href=\"{url}\"/></svg>"));
    assert!(
        r.document.layers.iter().any(|l| matches!(l.content, LayerContent::Raster(_))),
        "valid embedded gzip SVG must decode into a raster layer: {:?}",
        r.document.layers
    );

    // A gzip-compressed embedded SVG whose decompressed size exceeds the cap must be rejected
    // by our bounded inflate, not silently decompressed again without a limit by usvg.
    let mut bomb = format!("<svg {NS} width=\"4\" height=\"4\">").into_bytes();
    bomb.resize(usize::try_from(svg::MAX_INFLATED_BYTES).unwrap() + 1024, b' ');
    bomb.extend_from_slice(b"</svg>");
    let gz = gzip(&bomb);
    assert!(gz.len() < 2 << 20, "{} bytes compressed", gz.len());
    let url = base64_data_url("image/svg+xml", &gz);
    let start = std::time::Instant::now();
    let r = open(&format!("<svg {NS} width=\"10\" height=\"10\"><image x=\"0\" y=\"0\" width=\"10\" height=\"10\" href=\"{url}\"/></svg>"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "took {:?}", start.elapsed());
    // The embedded bomb is rejected; the outer drawing may still open (e.g. with a missing
    // image), but must not have decoded the bomb into a raster layer.
    assert!(r.document.layers.iter().all(|l| !matches!(l.content, LayerContent::Raster(_))), "{:?}", r.document.layers);
}

/// usvg parses an SVG embedded as a `data:` image on its own, past the drawing's depth check:
/// 20 000 nested groups in it overflowed a 2 MB thread stack, an abort nothing can catch. The
/// embedded SVG gets the same check now, so the image is dropped and the drawing opens.
#[test]
fn deeply_nested_embedded_svg_is_dropped_not_a_stack_overflow() {
    let deep = nested(20_000);
    let percent: String = deep.bytes().map(|b| if b.is_ascii_alphanumeric() { char::from(b).to_string() } else { format!("%{b:02X}") }).collect();
    let urls = [
        format!("data:image/svg+xml,{percent}"),
        base64_data_url("image/svg+xml", deep.as_bytes()),
        base64_data_url("text/plain", deep.as_bytes()),
        base64_data_url("image/svg+xml", &gzip(deep.as_bytes())),
        base64_data_url("text/plain", &gzip(deep.as_bytes())),
    ];
    let embed = |url: &str| format!("<svg {NS} width=\"8\" height=\"8\"><image width=\"8\" height=\"8\" href=\"{url}\"/></svg>");
    std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || {
            for url in &urls {
                let r = open(&embed(url));
                assert!(r.document.layers.iter().all(|l| !matches!(l.content, LayerContent::Raster(_))), "{}: {:?}", &url[..20], r.document.layers);
            }
            // Just inside the limit (the drawing's own `<svg>` takes one level) the embedded
            // drawing still renders (its rect is black).
            let r = open(&embed(&base64_data_url("image/svg+xml", nested(svg::MAX_XML_DEPTH - 2).as_bytes())));
            assert!(close(px(&r.document, 4, 4), [0.0, 0.0, 0.0, 1.0]), "{:?}", px(&r.document, 4, 4));
        })
        .unwrap()
        .join()
        .unwrap();
    // A PNG sent as `text/plain` is still an image, not markup to count.
    let r = open(&format!(
        "<svg {NS} width=\"8\" height=\"8\"><image width=\"8\" height=\"8\" image-rendering=\"pixelated\" href=\"{}\"/></svg>",
        png_data_url().replace("image/png", "text/plain")
    ));
    assert!(close(px(&r.document, 1, 1), [1.0, 0.0, 0.0, 1.0]), "{:?}", px(&r.document, 1, 1));
}

/// The embedded SVG is parsed on top of the drawing's own recursion, so the two depths share
/// [`svg::MAX_XML_DEPTH`]: 511 + 511 groups, each fine alone, overflowed a 1 MB thread stack.
#[test]
fn an_embedded_svg_gets_only_the_nesting_depth_the_drawing_left() {
    let embed = |outer: usize, inner: usize| {
        let url = base64_data_url("image/svg+xml", nested(inner).as_bytes());
        format!("<svg {NS} width=\"8\" height=\"8\">{}<image width=\"8\" height=\"8\" href=\"{url}\"/>{}</svg>", "<g>".repeat(outer), "</g>".repeat(outer))
    };
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(move || {
            let r = open(&embed(svg::MAX_XML_DEPTH - 1, svg::MAX_XML_DEPTH - 1));
            assert!(px(&r.document, 4, 4)[3] < 0.01, "{:?}", px(&r.document, 4, 4));
            // Within the shared budget the embedded drawing renders (its rect is black).
            let r = open(&embed(250, 250));
            assert!(close(px(&r.document, 4, 4), [0.0, 0.0, 0.0, 1.0]), "{:?}", px(&r.document, 4, 4));
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Accepted maximum-depth markup and deep embedded drawings must render correctly on the caller's unchanged small stack.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn accepted_deep_svg_and_embedded_children_do_not_exhaust_a_small_caller_stack() {
    std::thread::Builder::new()
        .stack_size(1 << 20)
        .spawn(|| {
            let top = nested(svg::MAX_XML_DEPTH - 1);
            let imported = open(&top);
            assert!(close(px(&imported.document, 4, 4), [0., 0., 0., 1.]), "near-limit top-level shape must remain visible");
            let (_, _, pixels) = svg::rasterize_rgba8(top.as_bytes()).unwrap();
            assert_eq!(&pixels[(4 * 8 + 4) * 4..(4 * 8 + 4) * 4 + 4], &[0, 0, 0, 255]);
            let (preview, _) = svg::import_svg_preview_with_work("drawing.svg", top.as_bytes(), u64::MAX).unwrap();
            assert!(close(px(&preview.document, 4, 4), [0., 0., 0., 1.]), "near-limit preview shape must remain visible");
            // The outer drawing alone is shallow. Its decoded child must receive the same
            // isolation as a deep top-level drawing, without increasing the shared depth cap.
            for inner in [126, 500] {
                let url = base64_data_url("image/svg+xml", nested(inner).as_bytes());
                let embedded = format!("<svg {NS} width=\"8\" height=\"8\"><g><image width=\"8\" height=\"8\" href=\"{url}\"/></g></svg>");
                let imported = open(&embedded);
                assert!(close(px(&imported.document, 4, 4), [0., 0., 0., 1.]), "shallow outer/deep child {inner} must remain visible");
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Multiple children and opacity retain the groups: parsing alone on a worker is insufficient.
#[cfg(not(target_arch = "wasm32"))]
#[test]
fn retained_deep_svg_groups_lower_render_and_drop_on_a_bounded_worker() {
    // Native document compositing has its own recursive stack requirements. Keep only this
    // independent pixel oracle off the small caller; SVG APIs and native document destruction
    // below must still complete there, with all retained groups intact.
    let pixel = |doc: &photocraft_doc::Document| {
        std::thread::scope(|scope| {
            std::thread::Builder::new()
                .name("svg-native-pixel-oracle".into())
                .stack_size(8 << 20)
                .spawn_scoped(scope, || px(doc, 0, 0))
                .unwrap()
                .join()
                .unwrap()
        })
    };
    std::thread::Builder::new()
        .stack_size(512 << 10)
        .spawn(move || {
            let source = format!(
                "<svg {NS} width=\"8\" height=\"8\">{}<rect width=\"8\" height=\"8\"/>{}</svg>",
                "<g opacity=\"0.99\"><rect width=\"1\" height=\"1\"/>".repeat(500),
                "</g>".repeat(500)
            );
            let imported = svg::import_svg("retained.svg", source.as_bytes()).unwrap();
            assert_eq!(imported.document.max_group_depth(), 100);
            assert!(pixel(&imported.document)[3] > 0.5);
            let (preview, _) = svg::import_svg_preview_with_work("retained.svg", source.as_bytes(), u64::MAX).unwrap();
            assert_eq!(preview.document.max_group_depth(), 100);
            assert!(pixel(&preview.document)[3] > 0.5);
            // Whole-drawing rasterisation refuses more than 200 nested layers (#10953) with an
            // error, not a crash; a drawing within that limit still renders on this small stack.
            let refused = svg::rasterize_rgba8(source.as_bytes()).unwrap_err().to_string();
            assert!(refused.contains("layers are nested too deeply"), "{refused}");
            let within = format!(
                "<svg {NS} width=\"8\" height=\"8\">{}<rect width=\"8\" height=\"8\"/>{}</svg>",
                "<g opacity=\"0.99\"><rect width=\"1\" height=\"1\"/>".repeat(150),
                "</g>".repeat(150)
            );
            let (w, h, pixels) = svg::rasterize_rgba8(within.as_bytes()).unwrap();
            assert_eq!((w, h), (8, 8));
            assert!(pixels[3] > 127);
        })
        .unwrap()
        .join()
        .unwrap();
}

/// 8 000 nested groups (70 KB) overflowed the XML parser's recursion on a 2 MB thread stack, an
/// abort nothing can catch. It is an error now, found before the parser runs; the thread here
/// has a small stack so a regression aborts the test instead of passing by luck.
#[test]
fn deeply_nested_markup_is_an_error_not_a_stack_overflow() {
    let deep = nested(8_000);
    let gz = gzip(deep.as_bytes());
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            for (name, bytes) in [("deep.svg", deep.as_bytes()), ("deep.svgz", gz.as_slice())] {
                let e = import(name, bytes).err().unwrap_or_else(|| panic!("{name} opened"));
                assert!(matches!(&e, IoError::Svg(m) if m.contains("nested") && m.contains(&svg::MAX_XML_DEPTH.to_string())), "{name}: {e}");
            }
        })
        .unwrap()
        .join()
        .unwrap();
    // Just inside the limit still opens, the empty groups folded away.
    let r = open(&nested(svg::MAX_XML_DEPTH - 1));
    assert_eq!(r.document.layers.len(), 1);
}

/// The parser expands a DTD's internal entities after the depth check: an entity holding
/// markup, or a quote in a DTD comment or declaration that made the check skip ahead, hid 20 000 nested groups
/// from it and overflowed the stack again. Both are errors now; same small stack as above.
#[test]
fn a_dtd_cannot_hide_nesting_from_the_depth_check() {
    let deep = |open: &str, close: &str| format!("{}{}", open.repeat(20_000), close.repeat(20_000));
    let with_dtd = |subset: &str, body: &str| format!("<!DOCTYPE svg [{subset}]><svg {NS} width=\"8\" height=\"8\">{body}</svg>");
    let cases = [
        ("entity", with_dtd(&format!("<!ENTITY a \"{}\">", deep("<g>", "</g>")), "&a;")),
        ("single-quoted entity", with_dtd(&format!("<!ENTITY a '{}'>", deep("<g>", "</g>")), "&a;")),
        ("quote in a comment", with_dtd("<!-- ' -->", &format!("{}<desc>'</desc>", deep("<g>", "</g>")))),
        ("quote in an element declaration", with_dtd("<!ELEMENT x '>", &format!("{}<desc>'</desc>", deep("<g>", "</g>")))),
        (
            "entity between quoted attribute lists",
            with_dtd(&format!("<!ATTLIST svg x CDATA '><!ENTITY a \"{}\"><!ATTLIST svg y CDATA '>", deep("<g>", "</g>")), "&a;"),
        ),
        ("entity between quoted notations", with_dtd(&format!("<!NOTATION n SYSTEM '><!ENTITY a \"{}\"><!NOTATION m SYSTEM '>", deep("<g>", "</g>")), "&a;")),
    ];
    std::thread::Builder::new()
        .stack_size(512 * 1024)
        .spawn(move || {
            for (name, svg) in &cases {
                let gz = gzip(svg.as_bytes());
                for bytes in [svg.as_bytes(), gz.as_slice()] {
                    let e = import("deep.svg", bytes).err().unwrap_or_else(|| panic!("{name} opened"));
                    assert!(matches!(&e, IoError::Svg(_)), "{name}: {e}");
                }
            }
        })
        .unwrap()
        .join()
        .unwrap();
}

/// Text-only entities, as Illustrator declares its namespaces, still open, `&#60;` included:
/// the parser keeps a character reference in an entity as text.
#[test]
fn text_entities_in_a_dtd_still_open() {
    let r = open(
        "<?xml version=\"1.0\"?><!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\" [\
           <!ENTITY ns_svg \"http://www.w3.org/2000/svg\"> <!-- a ' and a <g> --> <?pi > ?> <!ATTLIST svg data-x CDATA '<'> <!ENTITY red '#ff0000'> <!ENTITY c \"&#169; &#60;g&#62;\"> ]>\
         <svg xmlns=\"&ns_svg;\" width=\"8\" height=\"8\"><title>&c;</title><rect width=\"8\" height=\"8\" fill=\"&red;\"/></svg>",
    );
    assert_eq!(r.document.layers.len(), 1);
    assert!(close(px(&r.document, 4, 4), [1.0, 0.0, 0.0, 1.0]));
}

/// The depth check reads the markup, not the parser's tree: it must not count `<` in comments,
/// CDATA, processing instructions or the DOCTYPE, nor end a tag at a `>` inside quotes.
#[test]
fn the_depth_check_skips_comments_cdata_doctype_and_quoted_markup() {
    let svg = format!(
        "<?xml version=\"1.0\"?><!-- {g} --><!DOCTYPE svg PUBLIC \"-//W3C//DTD SVG 1.1//EN\" \"http://www.w3.org/Graphics/SVG/1.1/DTD/svg11.dtd\">\
         <svg {NS} width=\"8\" height=\"8\"><desc><![CDATA[ {g} ]]></desc><?pi {g} ?>\
         <g id='a>b' data-x=\"c/>d\"><rect width=\"8\" height=\"8\" fill=\"#ff0000\"/></g><!-- {g} --></svg>",
        g = "<g>".repeat(svg::MAX_XML_DEPTH + 10)
    );
    let r = open(&svg);
    assert_eq!(r.document.layers.len(), 1);
    assert!(close(px(&r.document, 4, 4), [1.0, 0.0, 0.0, 1.0]));
}

#[test]
fn svgz_is_inflated_with_a_size_cap() {
    let plain = format!("<svg {NS} width=\"8\" height=\"8\"><rect width=\"8\" height=\"8\" fill=\"#0000ff\"/></svg>");
    let r = import("drawing.svgz", &gzip(plain.as_bytes())).unwrap();
    assert_eq!(r.document.layers.len(), 1);
    assert!(close(px(&r.document, 4, 4), [0.0, 0.0, 1.0, 1.0]));
    let split = plain.len() / 2;
    let members = [gzip(&plain.as_bytes()[..split]), gzip(&plain.as_bytes()[split..])].concat();
    assert!(close(px(&import("members.svgz", &members).unwrap().document, 4, 4), [0.0, 0.0, 1.0, 1.0]));
    // A bomb: a few hundred KB of gzip holding more than the cap of whitespace.
    let mut bomb = format!("<svg {NS} width=\"8\" height=\"8\">").into_bytes();
    bomb.resize(usize::try_from(svg::MAX_INFLATED_BYTES).unwrap() + 1024, b' ');
    bomb.extend_from_slice(b"</svg>");
    let gz = gzip(&bomb);
    assert!(gz.len() < 2 << 20, "{} bytes compressed", gz.len());
    let e = import("bomb.svgz", &gz).err().unwrap_or_else(|| panic!("the bomb opened"));
    assert!(matches!(&e, IoError::Svg(m) if m.contains("inflates past")), "{e}");
    // Corrupt gzip is an error too.
    let e = import("bad.svgz", &[0x1f, 0x8b, 0x08, 0x00, 0xff, 0xff]).err().unwrap_or_else(|| panic!("corrupt gzip opened"));
    assert!(matches!(e, IoError::Svg(_)), "{e}");
}

#[test]
fn sniffed_without_an_extension_and_refused_when_malformed() {
    let svg = format!("<?xml version=\"1.0\"?>\n<svg {NS} width=\"8\" height=\"8\"><rect width=\"8\" height=\"8\"/></svg>");
    assert!(svg::is_svg(svg.as_bytes()));
    let r = import("mystery.bin", svg.as_bytes()).unwrap();
    assert_eq!(r.document.layers.len(), 1);
    for bad in ["<svg", "<svg xmlns=\"http://www.w3.org/2000/svg\"><rect", "not xml at all", ""] {
        let e = import("x.svg", bad.as_bytes()).err().unwrap_or_else(|| panic!("{bad:?} opened"));
        assert!(matches!(e, IoError::Svg(_)), "{bad:?}: {e}");
    }
    let empty = open(&format!("<svg {NS} width=\"8\" height=\"8\"/>"));
    assert!(empty.document.layers.is_empty());
    assert!(empty.warnings.iter().any(|w| w.contains("nothing")), "{:?}", empty.warnings);
}

#[test]
fn rasterize_renders_the_whole_drawing_at_a_scale() {
    let svg = format!("<svg {NS} width=\"20\" height=\"10\"><rect x=\"0\" y=\"4\" width=\"20\" height=\"0.5\" fill=\"black\"/></svg>");
    let b = svg::rasterize(svg.as_bytes(), 10.0).unwrap();
    assert_eq!((b.rect.width(), b.rect.height()), (200, 100));
    // The half-unit line is a crisp 5 px band at 10×.
    assert!(b.get(100, 39)[3] < 0.01 && b.get(100, 42)[3] > 0.99 && b.get(100, 45)[3] < 0.01, "{:?}", (b.get(100, 39), b.get(100, 42), b.get(100, 45)));
    let one = svg::rasterize(svg.as_bytes(), 1.0).unwrap();
    assert_eq!((one.rect.width(), one.rect.height()), (20, 10));
    let huge = format!("<svg {NS} width=\"10000\" height=\"10000\"><rect width=\"10\" height=\"10\"/></svg>");
    assert!(svg::rasterize(huge.as_bytes(), 64.0).is_err(), "a render past the pixel cap is an error, not an allocation");
}

/// resvg's morphology, convolution and turbulence cost pixels × radius², × order² and × octaves:
/// such a drawing is refused up front instead of rendering for minutes.
#[test]
fn filters_too_costly_to_render_are_refused_not_run() {
    let drawing = |side: u32, fe: &str| {
        format!(
            "<svg {NS} width=\"{side}\" height=\"{side}\"><filter id=\"f\">{fe}</filter><rect width=\"{side}\" height=\"{side}\" fill=\"red\" filter=\"url(#f)\"/></svg>"
        )
    };
    let kernel = vec!["1"; 100 * 100].join(" ");
    // Masks, patterns and embedded drawings filter at the scale of the element using them: a
    // 20-unit radius-10 morphology drawn 100× larger.
    let f = "<filter id=\"f\"><feMorphology radius=\"10\"/></filter>";
    let inner = format!("<svg {NS} width=\"20\" height=\"20\">{f}<rect width=\"20\" height=\"20\" filter=\"url(#f)\"/></svg>");
    let embedded = base64_data_url("image/svg+xml", inner.as_bytes());
    for svg in [
        drawing(2000, "<feMorphology operator=\"dilate\" radius=\"1000\"/>"),
        drawing(2000, &format!("<feConvolveMatrix order=\"100\" kernelMatrix=\"{kernel}\"/>")),
        drawing(2000, "<feTurbulence baseFrequency=\"0.01\" numOctaves=\"1000000\"/>"),
        format!(
            "<svg {NS} width=\"2000\" height=\"2000\" viewBox=\"0 0 20 20\">{f}<mask id=\"m\"><rect width=\"20\" height=\"20\" fill=\"white\" filter=\"url(#f)\"/></mask><rect width=\"20\" height=\"20\" mask=\"url(#m)\"/></svg>"
        ),
        format!(
            "<svg {NS} width=\"2000\" height=\"2000\">{f}<pattern id=\"p\" width=\"20\" height=\"20\" patternUnits=\"userSpaceOnUse\" patternTransform=\"scale(100)\"><rect width=\"20\" height=\"20\" filter=\"url(#f)\"/></pattern><rect width=\"2000\" height=\"2000\" fill=\"url(#p)\"/></svg>"
        ),
        format!("<svg {NS} width=\"2000\" height=\"2000\"><image width=\"2000\" height=\"2000\" href=\"{embedded}\"/></svg>"),
    ] {
        let head: String = svg.chars().take(300).collect();
        let e = import("x.svg", svg.as_bytes()).err().unwrap_or_else(|| panic!("opened: {head}"));
        assert!(matches!(&e, IoError::Svg(m) if m.contains("filter")), "{e}");
        assert!(svg::rasterize(svg.as_bytes(), 1.0).is_err(), "rendered: {head}");
        // The budgeted paths (smart-object contents and their renders) refuse it too.
        let e = svg::import_svg_bounded_images("x.svg", svg.as_bytes(), 1 << 24).err().unwrap_or_else(|| panic!("opened bounded: {head}"));
        assert!(matches!(&e, IoError::Svg(m) if m.contains("filter")), "{e}");
        let e = svg::rasterize_bounded(svg.as_bytes(), 1.0, 1 << 24).err().unwrap_or_else(|| panic!("rendered bounded: {head}"));
        assert!(matches!(&e, IoError::Svg(m) if m.contains("filter")), "{e}");
    }
    // Ordinary filters still open and render.
    let ordinary = drawing(500, "<feGaussianBlur stdDeviation=\"8\"/><feMorphology radius=\"3\"/><feTurbulence baseFrequency=\"0.05\" numOctaves=\"4\"/>");
    assert!(!open(&ordinary).document.layers.is_empty());
    assert!(svg::rasterize(ordinary.as_bytes(), 1.0).is_ok());
    // The smart-object render counts its scale: cheap at 1×, refused at 40×.
    let small = drawing(100, "<feMorphology radius=\"20\"/>");
    assert!(svg::rasterize(small.as_bytes(), 1.0).is_ok());
    assert!(svg::rasterize(small.as_bytes(), 40.0).is_err());
}

/// Patterns that fill and stroke with the next one double resvg's work at every level: past the
/// check's element budget the drawing is refused, not walked (or rendered) for hours.
#[test]
fn patterns_nested_too_deep_are_refused_not_walked() {
    let chain = |depth: usize| {
        let defs: String = (0..depth)
            .map(|i| {
                let paint = if i + 1 < depth { format!("url(#p{})", i + 1) } else { "red".into() };
                format!("<pattern id=\"p{i}\" width=\"4\" height=\"4\" patternUnits=\"userSpaceOnUse\"><rect width=\"4\" height=\"4\" fill=\"{paint}\" stroke=\"{paint}\"/></pattern>")
            })
            .collect();
        format!("<svg {NS} width=\"8\" height=\"8\">{defs}<rect width=\"8\" height=\"8\" fill=\"url(#p0)\"/></svg>")
    };
    let deep = chain(24);
    let e = import("x.svg", deep.as_bytes()).err().unwrap_or_else(|| panic!("opened a 2^24-use pattern chain"));
    assert!(matches!(&e, IoError::Svg(m) if m.contains("too costly")), "{e}");
    assert!(svg::rasterize(deep.as_bytes(), 1.0).is_err());
    // A few levels still open.
    assert!(!open(&chain(4)).document.layers.is_empty());
}

/// usvg keeps a chain of single-use patterns past the check's recursion cutoff and resvg draws
/// every one of them: a costly filter at the end of the chain is refused, not rendered unseen.
#[test]
fn a_costly_filter_past_the_pattern_recursion_cutoff_is_refused() {
    let chain = |depth: usize, fe: &str| {
        let defs: String = (0..depth)
            .map(|i| {
                let body = if i + 1 < depth {
                    format!("<rect width=\"500\" height=\"500\" fill=\"url(#p{})\"/>", i + 1)
                } else {
                    "<rect width=\"500\" height=\"500\" fill=\"red\" filter=\"url(#f)\"/>".into()
                };
                format!("<pattern id=\"p{i}\" width=\"500\" height=\"500\" patternUnits=\"userSpaceOnUse\">{body}</pattern>")
            })
            .collect();
        format!("<svg {NS} width=\"500\" height=\"500\"><filter id=\"f\">{fe}</filter>{defs}<rect width=\"500\" height=\"500\" fill=\"url(#p0)\"/></svg>")
    };
    // Just over the budget (about 3.5 billion reads), so a regression fails in seconds, not hours.
    let fe = "<feMorphology operator=\"dilate\" radius=\"50\"/>";
    assert!(import("x.svg", chain(4, fe).as_bytes()).is_err(), "the filter alone is over the budget");
    let deep = chain(210, fe);
    let e = import("x.svg", deep.as_bytes()).err().unwrap_or_else(|| panic!("opened a costly filter 210 patterns deep"));
    assert!(matches!(&e, IoError::Svg(m) if m.contains("too costly")), "{e}");
    assert!(svg::rasterize(deep.as_bytes(), 1.0).is_err());
    // A short chain with an ordinary filter still opens, and so do groups nested past the cutoff.
    assert!(!open(&chain(4, "<feGaussianBlur stdDeviation=\"2\"/>")).document.layers.is_empty());
    let groups =
        format!("<svg {NS} width=\"10\" height=\"10\">{}<rect width=\"10\" height=\"10\"/>{}</svg>", "<g opacity=\"0.99\">".repeat(250), "</g>".repeat(250));
    assert!(!open(&groups).document.layers.is_empty());
}

#[test]
fn drawings_the_svg_renderer_panics_on_are_errors_not_crashes() {
    // usvg panics laying out an embedded SVG image of a zero-ish width.
    let nested = "data:image/svg+xml;utf8,%3Csvg xmlns='http://www.w3.org/2000/svg' width='10' height='10'%3E%3Crect width='10' height='10'/%3E%3C/svg%3E";
    let bad_image = format!("<svg {NS} width=\"100\" height=\"100\"><image width=\"1e-30\" height=\"50\" href=\"{nested}\"/></svg>");
    let e = import("image.svg", bad_image.as_bytes()).err().unwrap_or_else(|| panic!("opened"));
    assert!(matches!(e, IoError::Svg(_)), "{e}");
    // The budgeted parse (placed and embedded drawings) is guarded too.
    let e = svg::import_svg_bounded_images("image.svg", bad_image.as_bytes(), 1 << 24).err().unwrap_or_else(|| panic!("opened"));
    assert!(matches!(e, IoError::Svg(_)), "{e}");
    // resvg panics rendering an offset past the pixmap's size: that element is dropped with a
    // warning, the rest of the drawing still opens.
    let offset = "<filter id=\"f\"><feOffset dx=\"3e9\" dy=\"-3e9\"/></filter>\
                  <rect width=\"50\" height=\"50\" filter=\"url(#f)\"/><rect id=\"plain\" x=\"60\" y=\"60\" width=\"20\" height=\"20\"/>";
    let svg = format!("<svg {NS} width=\"100\" height=\"100\">{offset}</svg>");
    let r = open(&svg);
    assert_eq!(r.document.layers.len(), 1, "{:?}", r.document.layers.iter().map(|l| &l.name).collect::<Vec<_>>());
    assert!(matches!(r.document.layers[0].content, LayerContent::Shape(_)));
    assert!(r.warnings.iter().any(|w| w.contains("could not be rendered")), "{:?}", r.warnings);
    // The same drawing as a smart object, and past the element limit (opened as one pixel layer).
    let e = svg::rasterize(svg.as_bytes(), 1.0).err().unwrap_or_else(|| panic!("rendered"));
    assert!(matches!(e, IoError::Svg(_)), "{e}");
    let e = svg::rasterize_bounded(svg.as_bytes(), 1.0, 1 << 24).err().unwrap_or_else(|| panic!("rendered"));
    assert!(matches!(e, IoError::Svg(_)), "{e}");
    let many = format!("<svg {NS} width=\"100\" height=\"100\">{offset}{}</svg>", "<rect width=\"1\" height=\"1\"/>".repeat(2_000));
    let r = open(&many);
    assert!(r.document.layers.is_empty());
    assert!(r.warnings.iter().any(|w| w.contains("could not be rendered")), "{:?}", r.warnings);
    assert!(!r.warnings.iter().any(|w| w.contains("nothing to draw")), "not an empty drawing: {:?}", r.warnings);
}

#[test]
fn an_svg_round_trips_through_pcraft_as_shapes() {
    let r = open(&format!(
        "<svg {NS} width=\"50\" height=\"50\"><rect id=\"a\" width=\"25\" height=\"25\" fill=\"#123456\"/><circle cx=\"37\" cy=\"37\" r=\"10\" fill=\"#abcdef\"/></svg>"
    ));
    let saved = export(&r.document, "x.pcraft", &ExportOptions::default()).unwrap();
    let back = import("x.pcraft", &saved.bytes).unwrap().document;
    assert_eq!(back.layers.len(), 2);
    assert!(back.layers.iter().all(|l| matches!(l.content, LayerContent::Shape(_))));
    assert!(close(px(&back, 10, 10), px(&r.document, 10, 10)));
}

#[test]
fn image_hrefs_to_local_files_are_never_read() {
    // A real PNG on disk, referenced by absolute path and as a file URL: opening an untrusted
    // drawing must not pull it in (only `data:` URLs resolve).
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("svg-local-image");
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("secret.png");
    let img = photocraft_codecs::Image::from_u8(2, 2, photocraft_codecs::ChannelLayout::Rgb, vec![255; 12]).unwrap();
    std::fs::write(&file, photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &Default::default()).unwrap()).unwrap();
    let path = file.to_string_lossy().into_owned();
    for href in [path.clone(), format!("file://{path}"), "secret.png".to_string()] {
        let r = open(&format!("<svg {NS} width=\"20\" height=\"20\"><image x=\"0\" y=\"0\" width=\"20\" height=\"20\" href=\"{href}\"/></svg>"));
        let d = &r.document;
        assert!(d.layers.iter().all(|l| !matches!(l.content, LayerContent::Raster(_))), "{href}: {:?}", d.layers.len());
        assert!(px(d, 10, 10)[3] < 0.01, "{href}: {:?}", px(d, 10, 10));
    }
    // The same picture as a data URL still loads.
    assert!(matches!(
        open(&format!("<svg {NS} width=\"20\" height=\"20\"><image width=\"20\" height=\"20\" href=\"{}\"/></svg>", png_data_url())).document.layers[0].content,
        LayerContent::Raster(_)
    ));
}
