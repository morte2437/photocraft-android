//! PDF export: structure, pixel oracle, colour modes and refusal paths.

use super::*;
use lopdf::{Object, StringFormat};
use photocraft_color::{PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Artboard, Group, Layer, LayerContent, Size};
use photocraft_geom::Rect;
use photocraft_raster::{Interrupt, Surface};

fn solid(fmt: PixelFormat, x0: i32, y0: i32, x1: i32, y1: i32, px: &[f32]) -> Surface {
    let mut s = Surface::new(fmt);
    let (w, h) = ((x1 - x0) as usize, (y1 - y0) as usize);
    let data: Vec<f32> = px.iter().copied().cycle().take(w * h * px.len()).collect();
    s.write_region(Rect::from_xywh(x0, y0, w as u32, h as u32), &data);
    s
}

/// The opaque paper colour of `mode`, with alpha.
fn paper(mode: ColorMode) -> Vec<f32> {
    match mode {
        ColorMode::Grayscale => vec![1.0, 1.0],
        ColorMode::Cmyk => vec![0.0, 0.0, 0.0, 0.0, 1.0],
        _ => vec![1.0, 1.0, 1.0, 1.0],
    }
}

fn doc(mode: ColorMode) -> Document {
    let fmt = PixelFormat::new(mode, SampleType::F32, true);
    let bg = paper(mode);
    let mut d = Document::new("t.pdf", Size { width: 4, height: 2 }, mode, SampleType::F32);
    d.layers = vec![Layer::new("Paper", LayerContent::Raster(solid(fmt, 0, 0, 4, 2, &bg)))];
    d
}

/// The bytes of the stream following `marker` (e.g. "/Subtype /Image"), inflated.
fn stream_after(pdf: &[u8], marker: &str) -> Vec<u8> {
    let at = pdf.windows(marker.len()).position(|w| w == marker.as_bytes()).expect("marker");
    let body = &pdf[at..];
    let len_at = body.windows(8).position(|w| w == b"/Length ").expect("length") + 8;
    let len_end = body[len_at..].iter().position(|b| !b.is_ascii_digit()).expect("length end");
    let len: usize = std::str::from_utf8(&body[len_at..len_at + len_end]).unwrap().parse().unwrap();
    let start = body.windows(7).position(|w| w == b"stream\n").expect("stream") + 7;
    let mut out = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&body[start..start + len]), &mut out).unwrap();
    out
}

fn has(pdf: &[u8], needle: &str) -> bool {
    pdf.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn check_structure(pdf: &[u8]) {
    assert!(pdf.starts_with(b"%PDF-1."), "PDF header");
    assert!(pdf.ends_with(b"%%EOF\n"), "EOF marker");
    let start = pdf.windows(9).rposition(|w| w == b"startxref").unwrap() + 9;
    let off: usize = std::str::from_utf8(&pdf[start..]).unwrap().split_whitespace().next().unwrap().parse().unwrap();
    assert_eq!(&pdf[off..off + 4], b"xref", "startxref points at the xref table");
    // Every xref offset really precedes its `n 0 obj` header. The xref/trailer region is pure
    // ASCII, so it is safe to decode just that slice.
    let table = std::str::from_utf8(&pdf[off..]).unwrap().lines().collect::<Vec<_>>();
    let count: usize = table[1].split_whitespace().nth(1).unwrap().parse().unwrap();
    for (i, line) in table.iter().skip(3).take(count - 1).enumerate() {
        let o: usize = line[..10].parse().unwrap();
        let want = format!("{} 0 obj", i + 1);
        assert_eq!(&pdf[o..o + want.len()], want.as_bytes(), "xref offset for object {}", i + 1);
    }
}

#[test]
fn a_flat_rgb_document_saves_as_one_page_pdf() {
    let d = doc(ColorMode::Rgb);
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    assert!(has(pdf, "/MediaBox [0 0 4.00 2.00]"), "page = image size at 72 pt/in");
    assert!(has(pdf, "/Width 4 /Height 2 /ColorSpace /DeviceRGB"), "RGB is DeviceRGB");
    assert_eq!(stream_after(pdf, "/Subtype /Image"), vec![255u8; 4 * 2 * 3], "the white paper, 8-bit");
    assert!(out.warnings.iter().any(|w| w.contains("flattened")), "layers are not preserved");
    // Through the general dispatch as well.
    let via = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
    assert_eq!(via.bytes, *pdf);
}

#[test]
fn layers_composite_over_white_before_the_page() {
    let mut d = doc(ColorMode::Rgb);
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    d.layers.push(Layer::new("Half red", LayerContent::Raster(solid(fmt, 1, 0, 3, 2, &[1.0, 0.0, 0.0, 0.5]))));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    let img = stream_after(&out.bytes, "/Subtype /Image");
    // Pixels 1-2 of row 0: 0.5 red over white = (1.0, 0.5, 0.5); elsewhere plain white.
    assert_eq!(&img[3..6], &[255, 128, 128], "the blend lands: {:?}", &img[3..6]);
    assert_eq!(&img[0..3], &[255, 255, 255]);
}

#[test]
fn topmost_solid_shape_is_exported_as_a_vector_path() {
    use photocraft_doc::{Fill, Path, ShapeLayer, Subpath};
    let mut d = doc(ColorMode::Rgb);
    let shape = ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (2.0, 0.0), (2.0, 1.0)])]),
        fill: Some(Fill::Solid(photocraft_color::Color::rgb(1.0, 0.0, 0.0))),
        ..Default::default()
    };
    d.layers.push(Layer::new("Shape", LayerContent::Shape(shape)));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    let content = stream_after(&out.bytes, "/Contents 4 0 R");
    let content = String::from_utf8(content).unwrap();
    assert!(content.contains("1 0 0 rg\n"));
    assert!(content.contains("0 0 m\n"));
    assert!(content.contains("h\nf\n"));
    assert_eq!(stream_after(&out.bytes, "/Subtype /Image"), vec![255; 4 * 2 * 3]);
}

#[test]
fn topmost_text_layer_is_exported_with_selectable_text_and_vector_outlines() {
    use photocraft_doc::TextLayer;
    let mut d = doc(ColorMode::Rgb);
    let text = TextLayer {
        text: "Vector PDF".into(),
        size_pt: 24.0,
        color: photocraft_color::Color::rgb(0.0, 0.0, 1.0),
        transform: photocraft_geom::Affine::translate(1.0, 1.5),
        ..Default::default()
    };
    let layout = photocraft_text::TextEngine::new().layout(&text, 72.0);
    let cluster = layout.clusters.iter().filter(|cluster| !cluster.range.is_empty()).min_by_key(|cluster| cluster.range.start).expect("a shaped cluster");
    let baseline = layout.lines.get(cluster.line).expect("a line").baseline;
    let origin = text.transform.apply(photocraft_geom::Point::new(f64::from(cluster.x), f64::from(baseline)));
    d.layers.push(Layer::new("Text", LayerContent::Text(text)));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    let content = stream_after(&out.bytes, "/Contents 4 0 R");
    let content = String::from_utf8(content).unwrap();
    assert!(content.contains("/Span << /ActualText <FEFF"));
    assert!(content.contains("0 0 1 rg"));
    assert!(content.contains("BT"));
    assert!(content.contains("3 Tr"));
    assert!(content.contains("/FSelect"));
    assert!(has(&out.bytes, "/ToUnicode"));
    let tm = format!("{} {} Tm", origin.x, origin.y);
    assert!(content.lines().any(|line| line.ends_with(tm.as_str())), "the translated text starts at {tm}:\n{content}");
    let parsed = lopdf::Document::load_mem(&out.bytes).unwrap();
    assert_eq!(parsed.extract_text(&[1]).unwrap().trim(), "Vector PDF");
    assert!(content.contains("ET"));
    assert!(content.contains("EMC"));
    assert_eq!(stream_after(&out.bytes, "/Subtype /Image"), vec![255; 4 * 2 * 3]);
}

#[test]
fn topmost_shape_and_text_layers_are_both_exported_as_vectors() {
    use photocraft_doc::{Fill, Path, ShapeLayer, Subpath, TextLayer};
    let mut d = doc(ColorMode::Rgb);
    let shape = ShapeLayer {
        path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (2.0, 0.0), (2.0, 1.0)])]),
        fill: Some(Fill::Solid(photocraft_color::Color::rgb(1.0, 0.0, 0.0))),
        ..Default::default()
    };
    d.layers.push(Layer::new("Shape", LayerContent::Shape(shape)));
    let text = TextLayer { text: "Hi".into(), size_pt: 12.0, color: photocraft_color::Color::rgb(0.0, 1.0, 0.0), ..Default::default() };
    d.layers.push(Layer::new("Text", LayerContent::Text(text)));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    let content = stream_after(&out.bytes, "/Contents 4 0 R");
    let content = String::from_utf8(content).unwrap();
    let red = content.find("1 0 0 rg\n").expect("the shape fill");
    let green = content.find("0 1 0 rg\n").expect("the text fill");
    assert!(red < green, "the lower red shape is painted before the upper green text");
    assert!(content.contains("/Span << /ActualText <FEFF0048> >> BDC"));
    assert!(content.contains("> Tj"));
    assert_eq!(stream_after(&out.bytes, "/Subtype /Image"), vec![255; 4 * 2 * 3]);
}

#[test]
fn greyscale_and_cmyk_pages_keep_their_channels() {
    let g = export(&doc(ColorMode::Grayscale), true, &Interrupt::NONE).unwrap();
    assert!(has(&g.bytes, "/ColorSpace /DeviceGray"), "gray is DeviceGray");
    assert_eq!(stream_after(&g.bytes, "/Subtype /Image"), vec![255; 4 * 2], "one sample per pixel");
    let c = export(&doc(ColorMode::Cmyk), true, &Interrupt::NONE).unwrap();
    assert!(has(&c.bytes, "/ColorSpace /DeviceCMYK"), "cmyk is DeviceCMYK");
    assert_eq!(stream_after(&c.bytes, "/Subtype /Image"), vec![0; 4 * 2 * 4], "zero ink over the whole page");
}

#[test]
fn the_page_measures_at_the_document_resolution() {
    let mut d = doc(ColorMode::Rgb);
    d.size = Size { width: 300, height: 150 };
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    d.layers = vec![Layer::new("Paper", LayerContent::Raster(solid(fmt, 0, 0, 300, 150, &[1.0, 1.0, 1.0, 1.0])))];
    d.resolution_dpi = 300.0;
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    assert!(has(&out.bytes, "/MediaBox [0 0 72.00 36.00]"), "300 px at 300 dpi is one inch");
}

#[test]
fn an_embedded_profile_travels_as_iccbased() {
    let mut d = doc(ColorMode::Rgb);
    d.icc_profile = Some(std::sync::Arc::new(vec![1, 2, 3, 4, 5, 6, 7, 8]));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    assert!(has(&out.bytes, "[/ICCBased 7 0 R]"), "the image refers to the profile");
    assert_eq!(stream_after(&out.bytes, "/N 3 /Alternate"), vec![1, 2, 3, 4, 5, 6, 7, 8], "the profile bytes survive");
}

#[test]
fn other_modes_are_refused_with_an_actionable_error() {
    let d = doc(ColorMode::Lab);
    let e = export(&d, true, &Interrupt::NONE).unwrap_err().to_string();
    assert!(e.contains("RGB") && e.contains("Mode"), "{e}");
    assert!(crate::export(&d, "x.pdf", &crate::ExportOptions::default()).is_err());
}

// ---- Spot plates (FILE-215-17) ----

/// A grayscale channel surface over the 4×2 test canvas, one value per pixel.
fn channel(vals: &[f32]) -> Surface {
    let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    s.write_region(Rect::new(0, 0, 4, 2), vals);
    s
}

fn spot(name: &str, color: photocraft_color::Color, vals: &[f32]) -> AlphaChannel {
    AlphaChannel { spot: Some((color, 0.0)), ..AlphaChannel::new(name, channel(vals)) }
}

/// The content stream (object 4), inflated, as text.
fn content(pdf: &[u8]) -> String {
    String::from_utf8(stream_after(pdf, "4 0 obj")).unwrap()
}

#[test]
fn spot_channels_follow_the_image_as_overprinting_separation_plates() {
    let mut d = doc(ColorMode::Rgb);
    let under: Vec<f32> = vec![1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0];
    let red: Vec<f32> = vec![0.0, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0];
    d.channels.push(spot("Underbase White", photocraft_color::Color::WHITE, &under));
    d.channels.push(spot("PANTONE 185 C", photocraft_color::Color::rgb(0.89, 0.09, 0.22), &red));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    // The composite is still the first image, in its own colour space.
    assert!(has(pdf, "/Width 4 /Height 2 /ColorSpace /DeviceRGB"), "the flattened composite stays");
    assert_eq!(stream_after(pdf, "/Subtype /Image"), vec![255u8; 4 * 2 * 3]);
    // One Separation XObject per channel, named after it, with a DeviceCMYK tint transform.
    assert!(
        has(
            pdf,
            "/ColorSpace [/Separation /Underbase#20White /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [0.0000 0.0000 0.0000 0.0000] /N 1 >>]"
        ),
        "white ink previews as no ink"
    );
    assert!(has(pdf, "/Separation /PANTONE#20185#20C /DeviceCMYK"), "the second plate");
    // 1 = solid ink, as the channel stores it: the plate is the channel, 8-bit, not inverted.
    assert_eq!(stream_after(pdf, "/Separation /Underbase#20White"), vec![255, 255, 255, 255, 0, 0, 0, 0]);
    assert_eq!(stream_after(pdf, "/Separation /PANTONE#20185#20C"), vec![0, 128, 255, 0, 0, 0, 0, 0]);
    // Overprint is on for the plates, so each paints only its own colorant; the plates carry no
    // mask of any kind (plain viewers show the last plate opaque; a RIP separates them).
    assert!(has(pdf, "/Type /ExtGState /OP true /op true /OPM 1"), "overprint state");
    assert!(!has(pdf, "/Mask") && !has(pdf, "/ImageMask"), "no stencil or colour-key mask");
    assert!(!has(pdf, "/SMask"), "no transparency");
    assert!(!has(pdf, "/Width 4 /Height 2 /ColorSpace /DeviceRGB /BitsPerComponent 8 /Mask"), "the composite is not masked");
    let c = content(pdf);
    let (im, p0, p1) = (
        c.find("/Im0 Do").unwrap(),
        c.find("/GS0 gs 4.000 0 0 2.000 0.000 0.000 cm /Sp0 Do").unwrap(),
        c.find("/GS0 gs 4.000 0 0 2.000 0.000 0.000 cm /Sp1 Do").unwrap(),
    );
    assert!(im < p0 && p0 < p1, "image, then the plates in channel order (underbase first): {c}");
    assert!(out.warnings.iter().any(|w| w.contains("2 spot plates") && w.contains("Underbase White, PANTONE 185 C")), "{:?}", out.warnings);
    // An alpha channel of an RGB document is a mask, not a plate.
    d.channels.push(AlphaChannel::new("Mask", channel(&[1.0; 8])));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    assert!(!has(&out.bytes, "/Separation /Mask"), "alpha channels stay out of RGB plates");
    assert!(has(&out.bytes, "/Sp1 "), "the two spot plates remain");
    assert!(!has(&out.bytes, "/Sp2 "));
}

#[test]
fn a_multichannel_document_is_its_plates_alone() {
    let mut d = Document::new("m.pdf", Size { width: 4, height: 2 }, ColorMode::Multichannel, SampleType::F32);
    d.resolution_dpi = 72.0;
    d.channels.push(spot("Cyan", photocraft_color::Color::rgb(0.0, 1.0, 1.0), &[1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
    d.channels.push(spot("Black Ink", photocraft_color::Color::BLACK, &[0.0, 1.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0]));
    // An alpha-type channel of a Multichannel document prints as black ink where it is
    // unselected (1 − value), as the composite shows it.
    d.channels.push(AlphaChannel::new("Alpha 1", channel(&[1.0, 1.0, 0.0, 0.5, 1.0, 1.0, 1.0, 1.0])));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    assert!(!has(pdf, "/Im0"), "no composite image");
    assert!(has(pdf, "/MediaBox [0 0 4.00 2.00]"));
    assert_eq!(stream_after(pdf, "/Separation /Cyan"), vec![255, 0, 0, 0, 0, 0, 0, 0]);
    // The alternate is the ink separated through the CMYK profile in use: a rich black here,
    // whatever its CMY, with the black ink near solid.
    let marker = "/Separation /Black#20Ink /DeviceCMYK << /FunctionType 2 /Domain [0 1] /C0 [0 0 0 0] /C1 [";
    let at = pdf.windows(marker.len()).position(|w| w == marker.as_bytes()).expect("the black plate's tint transform") + marker.len();
    let c1 = std::str::from_utf8(&pdf[at..at + 32]).unwrap().split(']').next().unwrap().to_string();
    let k: f32 = c1.split_whitespace().nth(3).unwrap().parse().unwrap();
    assert!(k > 0.85, "black ink's K alternate: {c1}");
    assert_eq!(stream_after(pdf, "/Separation /Alpha#201"), vec![0, 0, 255, 128, 0, 0, 0, 0], "alpha-type: ink = 1 − value");
    let c = content(pdf);
    assert!(c.find("/Sp0 Do").unwrap() < c.find("/Sp1 Do").unwrap() && c.find("/Sp1 Do").unwrap() < c.find("/Sp2 Do").unwrap());
    assert!(!out.warnings.iter().any(|w| w.contains("flattened")), "nothing was flattened: {:?}", out.warnings);
    // Without plates there is nothing to write.
    let e = export(&d, false, &Interrupt::NONE).unwrap_err().to_string();
    assert!(e.contains("Multichannel") && e.contains("spot plates"), "{e}");
    d.channels.clear();
    assert!(export(&d, true, &Interrupt::NONE).is_err(), "no channels, nothing to write");
}

#[test]
fn plate_names_are_escaped_as_pdf_names() {
    let mut d = doc(ColorMode::Grayscale);
    let ink = photocraft_color::Color::rgb(0.2, 0.3, 0.4);
    d.channels.push(spot("Black (Ink) #1/ü", ink, &[0.0; 8]));
    d.channels.push(spot("", ink, &[0.0; 8]));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    assert!(has(&out.bytes, "/Separation /Black#20#28Ink#29#20#231#2F#C3#BC /DeviceCMYK"), "delimiters, #, space and UTF-8 bytes are #xx");
    assert!(has(&out.bytes, "/Separation /Spot /DeviceCMYK"), "an empty name is not the bare /");
}

#[test]
fn spot_plates_can_be_left_out_of_a_scripted_save() {
    let mut d = doc(ColorMode::Cmyk);
    d.channels.push(spot("Varnish", photocraft_color::Color::rgb(0.9, 0.9, 0.6), &[1.0; 8]));
    let with = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
    assert!(has(&with.bytes, "/Separation /Varnish"), "plates by default");
    let without = crate::export(&d, "out.pdf", &crate::ExportOptions { pdf_spot_plates: false, ..Default::default() }).unwrap();
    assert!(!has(&without.bytes, "/Separation") && !has(&without.bytes, "/ExtGState"), "no plate, no overprint state, no mask");
    assert!(has(&without.bytes, "/ColorSpace /DeviceCMYK"), "the composite still saves");
    assert!(without.warnings.iter().any(|w| w.contains("not written")), "{:?}", without.warnings);
    check_structure(&without.bytes);
    // A plain document never gets the plate warning either way.
    let plain = export(&doc(ColorMode::Rgb), true, &Interrupt::NONE).unwrap();
    assert!(!plain.warnings.iter().any(|w| w.contains("plate")), "{:?}", plain.warnings);
}

#[test]
fn vector_layers_sit_between_the_image_and_the_plates() {
    use photocraft_doc::TextLayer;
    let mut d = doc(ColorMode::Rgb);
    d.layers.push(Layer::new("Text", LayerContent::Text(TextLayer { text: "Hi".into(), size_pt: 12.0, ..Default::default() })));
    d.channels.push(spot("Varnish", photocraft_color::Color::rgb(0.9, 0.9, 0.6), &[1.0; 8]));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    let c = content(pdf);
    let (im, tx, sp) = (c.find("/Im0 Do").unwrap(), c.find("/ActualText").unwrap(), c.find("/Sp0 Do").unwrap());
    assert!(im < tx && tx < sp, "the image, the vector text over it, then the plate overprinting both:\n{c}");
    assert!(has(pdf, "/FSelect") && has(pdf, "/Separation /Varnish"));
    // The text left the raster backdrop for the vector plane.
    assert_eq!(stream_after(pdf, "/Subtype /Image"), vec![255u8; 4 * 2 * 3]);
    // A Multichannel document has no composite for vectors to sit on: plates alone, no vector block.
    let mut m = Document::new("m.pdf", Size { width: 4, height: 2 }, ColorMode::Multichannel, SampleType::F32);
    m.resolution_dpi = 72.0;
    m.layers.push(Layer::new("Text", LayerContent::Text(TextLayer { text: "Hi".into(), size_pt: 12.0, ..Default::default() })));
    m.channels.push(spot("K", photocraft_color::Color::BLACK, &[1.0; 8]));
    let c = content(&export(&m, true, &Interrupt::NONE).unwrap().bytes);
    assert!(!c.contains("/ActualText") && c.contains("/Sp0 Do"), "{c}");
}

#[test]
fn a_16_bit_plate_is_written_at_8_bits() {
    let mut d = Document::new("m.pdf", Size { width: 4, height: 2 }, ColorMode::Multichannel, SampleType::U16);
    let mut s = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::U16, false));
    s.write_region(Rect::new(0, 0, 4, 2), &[0.0, 0.25, 0.5, 1.0, 0.0, 0.0, 0.0, 0.0]);
    d.channels.push(AlphaChannel { spot: Some((photocraft_color::Color::BLACK, 0.0)), ..AlphaChannel::new("K", s) });
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    assert!(has(&out.bytes, "/BitsPerComponent 8"));
    assert_eq!(stream_after(&out.bytes, "/Separation /K"), vec![0, 64, 128, 255, 0, 0, 0, 0]);
}

// ---- Spot-plate XMP description ----

/// The XMP packet (the catalog's /Metadata stream), as text.
fn xmp_of(pdf: &[u8]) -> String {
    let start = pdf.windows(9).position(|w| w == b"<?xpacket").expect("a packet");
    let tail = b"<?xpacket end=\"w\"?>";
    let end = pdf[start..].windows(tail.len()).position(|w| w == tail).expect("packet end") + start + tail.len();
    String::from_utf8(pdf[start..end].to_vec()).unwrap()
}

#[test]
fn the_plates_are_described_in_xmp_with_their_ink_colour_and_solidity() {
    let mut d = Document::new("m.pdf", Size { width: 4, height: 2 }, ColorMode::Multichannel, SampleType::F32);
    let ink = photocraft_color::Color::rgb(0.2, 0.3, 0.4);
    d.channels.push(AlphaChannel { spot: Some((ink, 0.35)), ..AlphaChannel::new("Black (Ink) #1/ü & <x>", channel(&[1.0; 8])) });
    d.channels.push(spot("Underbase White", photocraft_color::Color::WHITE, &[0.0; 8]));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    let pdf = &out.bytes;
    check_structure(pdf);
    assert!(has(pdf, "/Type /Catalog /Pages 2 0 R /Metadata "), "the catalog points at the metadata stream");
    assert!(has(pdf, "/Type /Metadata /Subtype /XML /Length "), "an unfiltered XMP stream");
    let xmp = xmp_of(pdf);
    assert!(xmp.contains("xmlns:pcsp=\"http://ns.getartcraft.com/photocraft/spot-plates/1.0/\""), "{xmp}");
    let lab = photocraft_color::convert::srgb_to_lab([0.2, 0.3, 0.4]);
    let want = format!(
        "<rdf:li rdf:parseType=\"Resource\">\n      <pcsp:index>0</pcsp:index>\n      <pcsp:name>Black (Ink) #1/ü &amp; &lt;x&gt;</pcsp:name>\n      <pcsp:labL>{:.2}</pcsp:labL>\n      <pcsp:labA>{:.2}</pcsp:labA>\n      <pcsp:labB>{:.2}</pcsp:labB>\n      <pcsp:srgb>#334D66</pcsp:srgb>\n      <pcsp:solidity>35</pcsp:solidity>\n     </rdf:li>",
        lab[0], lab[1], lab[2]
    );
    assert!(xmp.contains(&want), "first plate:\n{xmp}");
    assert!(xmp.contains("<pcsp:index>1</pcsp:index>\n      <pcsp:name>Underbase White</pcsp:name>\n      <pcsp:labL>100.00</pcsp:labL>\n      <pcsp:labA>0.00</pcsp:labA>\n      <pcsp:labB>0.00</pcsp:labB>\n      <pcsp:srgb>#FFFFFF</pcsp:srgb>\n      <pcsp:solidity>0</pcsp:solidity>"), "second plate:\n{xmp}");
    // The order is the plates' paint order, which is the Separation names' order in the page.
    let pos = |needle: &str| pdf.windows(needle.len()).position(|w| w == needle.as_bytes()).expect(needle);
    assert!(pos("/Separation /Black#20#28Ink") < pos("/Separation /Underbase#20White"));
    // One packet, well-formed enough for an XML parser.
    assert_eq!(xmp.matches("<?xpacket begin").count(), 1);
    assert!(craft_xml::parse(&xmp.as_bytes()[xmp.find("<x:xmpmeta").unwrap()..xmp.rfind("</x:xmpmeta>").unwrap() + 12]).is_ok(), "{xmp}");
}

#[test]
fn the_plate_description_joins_the_documents_own_packet() {
    let mut d = doc(ColorMode::Rgb);
    d.channels.push(spot("Varnish", photocraft_color::Color::rgb(0.9, 0.9, 0.6), &[1.0; 8]));
    d.metadata.xmp = Some(std::sync::Arc::from(
        "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <rdf:RDF xmlns:rdf=\"http://www.w3.org/1999/02/22-rdf-syntax-ns#\">\n  <rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Job 1</dc:title></rdf:Description>\n </rdf:RDF>\n</x:xmpmeta>\n<?xpacket end=\"w\"?>",
    ));
    let out = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
    let xmp = xmp_of(&out.bytes);
    assert_eq!(xmp.matches("<?xpacket begin").count(), 1, "one packet, not two");
    assert!(xmp.contains("<dc:title>Job 1</dc:title>") && xmp.contains("<pcsp:name>Varnish</pcsp:name>"), "{xmp}");
    assert!(xmp.find("<dc:title>").unwrap() < xmp.find("<pcsp:plates>").unwrap() && xmp.find("<pcsp:plates>").unwrap() < xmp.rfind("</rdf:RDF>").unwrap());
    // A save that embeds no metadata drops the document's packet but still describes the plates.
    let none = crate::export(&d, "out.pdf", &crate::ExportOptions { xmp: crate::XmpEmbed::None, ..Default::default() }).unwrap();
    let xmp = xmp_of(&none.bytes);
    assert!(!xmp.contains("dc:title") && xmp.contains("<pcsp:name>Varnish</pcsp:name>"), "{xmp}");
    // Without plates the document's packet travels as it is, and a document without one has no stream.
    d.channels.clear();
    let plain = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
    assert!(has(&plain.bytes, "<dc:title>Job 1</dc:title>") && !has(&plain.bytes, "pcsp:"), "the document packet alone");
    d.metadata.xmp = None;
    let bare = export(&d, true, &Interrupt::NONE).unwrap();
    assert!(!has(&bare.bytes, "/Metadata") && !has(&bare.bytes, "<?xpacket"), "no metadata stream");
    check_structure(&bare.bytes);
}

#[test]
fn the_plate_description_joins_the_rdf_element_however_the_packet_spells_it() {
    const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";
    let mut d = doc(ColorMode::Rgb);
    d.channels.push(spot("Varnish", photocraft_color::Color::rgb(0.9, 0.9, 0.6), &[1.0; 8]));
    // The packet must parse as namespaced XML with the plates a description inside its RDF element.
    let plates_in_rdf = |xmp: &str| {
        let canon = craft_xml::Canonical::new(&[("rdf", RDF), ("pcsp", crate::pdf::SPOT_PLATES_NS)]);
        let tree = craft_xml::Options::canonical(&canon).parse(xmp.trim_start_matches('\u{feff}').as_bytes()).unwrap_or_else(|e| panic!("{e}: {xmp}"));
        fn child_els(el: &craft_xml::El) -> impl Iterator<Item = &craft_xml::El> {
            el.children.iter().filter_map(|n| match n {
                craft_xml::Node::El(e) => Some(e),
                _ => None,
            })
        }
        let mut stack = vec![&tree];
        let mut found = false;
        while let Some(el) = stack.pop() {
            if el.name == "rdf:RDF" && child_els(el).any(|d| d.name == "rdf:Description" && child_els(d).any(|p| p.name == "pcsp:plates")) {
                found = true;
                break;
            }
            stack.extend(child_els(el));
        }
        assert!(found, "no rdf:RDF/rdf:Description/pcsp:plates: {xmp}");
        assert_eq!(xmp.matches("<?xpacket begin").count(), 1, "one packet, not two: {xmp}");
    };
    let head = "<?xpacket begin=\"\u{feff}\" id=\"W5M0MpCehiHzreSzNTczkc9d\"?>\n<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n";
    let tail = "\n</x:xmpmeta>\n<?xpacket end=\"w\"?>";
    let title = "<r:Description r:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\"><dc:title>Job 2</dc:title></r:Description>";
    for (packet, what) in [
        // Another prefix bound to RDF, whitespace before the `>` of the end tag, a leading byte order mark.
        (format!("\u{feff}{head} <r:RDF xmlns:r=\"{RDF}\">\n  {title}\n </r:RDF >{tail}"), "prefix, whitespace and BOM"),
        // An end tag spelled out inside a comment is not the element's end.
        (format!("{head} <rdf:RDF xmlns:rdf=\"{RDF}\">\n  <!-- </rdf:RDF> -->\n  {}\n </rdf:RDF>{tail}", title.replace("r:", "rdf:")), "comment"),
        // A self-closing RDF element is expanded around the description.
        (format!("{head} <r:RDF xmlns:r=\"{RDF}\"/>{tail}"), "self-closing"),
    ] {
        d.metadata.xmp = Some(std::sync::Arc::from(packet.as_str()));
        let out = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
        let xmp = xmp_of(&out.bytes);
        plates_in_rdf(&xmp);
        assert_eq!(xmp.contains("<dc:title>Job 2</dc:title>"), what != "self-closing", "{what}: {xmp}");
        assert!(xmp.contains("<pcsp:name>Varnish</pcsp:name>"), "{what}: {xmp}");
    }
    // A packet with no RDF element, or that is not XML, is kept as it is: the plates go undescribed
    // rather than the document's metadata lost.
    for kept in [format!("{head}<x:note>kept</x:note>{tail}"), format!("{head}<rdf:RDF xmlns:rdf=\"{RDF}\"><unclosed>{tail}")] {
        d.metadata.xmp = Some(std::sync::Arc::from(kept.as_str()));
        let out = crate::export(&d, "out.pdf", &crate::ExportOptions::default()).unwrap();
        assert_eq!(xmp_of(&out.bytes), kept);
    }
}

#[test]
fn unicode_text_over_a_photo_extracts_through_a_pdf_reader() {
    let mut d = doc(ColorMode::Rgb);
    d.size = Size { width: 128, height: 48 };
    let fmt = d.pixel_format();
    // Synthetic photo-like backdrop; no personal image or third-party asset.
    d.layers = vec![Layer::new("Photo", LayerContent::Raster(solid(fmt, 0, 0, 128, 48, &[0.2, 0.4, 0.6, 1.0])))];
    let text = "café Ω 😀";
    d.layers.push(Layer::new(
        "Text",
        LayerContent::Text(photocraft_doc::TextLayer {
            text: text.into(),
            size_pt: 12.0,
            transform: photocraft_geom::Affine::translate(2.0, 20.0),
            ..Default::default()
        }),
    ));
    let pdf = export(&d, true, &Interrupt::NONE).unwrap().bytes;
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    assert_eq!(parsed.extract_text(&[1]).unwrap().trim(), text);
    assert!(has(&pdf, "/Subtype /Image"));
    assert!(has(&pdf, "/Subtype /Type3"));
    assert!(has(&pdf, "/ToUnicode"));
    assert_eq!(&stream_after(&pdf, "/Subtype /Image")[..3], &[51, 102, 153]);
    let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
    assert!(!content.contains("/ActualText <FEFF00630061006600E9"), "selection spans are clusters, not the entire layer");
}

#[test]
fn text_with_compositing_or_a_profile_stays_in_the_raster_backdrop() {
    for case in 0..5 {
        let mut d = doc(ColorMode::Rgb);
        d.size = Size { width: 32, height: 32 };
        let mut layer = Layer::new(
            "Text",
            LayerContent::Text(photocraft_doc::TextLayer {
                text: "A".into(),
                size_pt: 16.0,
                transform: photocraft_geom::Affine::translate(2.0, 20.0),
                ..Default::default()
            }),
        );
        // The compositor consumes the text cache maintained by the engine/UI.
        if let LayerContent::Text(text) = &mut layer.content {
            photocraft_text::TextEngine::new().render_layer(text, d.resolution_dpi, d.pixel_format());
        }
        match case {
            0 => layer.opacity = 0.5,
            1 => layer.fill_opacity = 0.5,
            2 => layer.blend = photocraft_color::BlendMode::Multiply,
            3 => {
                layer.effects.enabled = true;
                layer.effects.items.push(photocraft_doc::effects::Effect::default_drop_shadow());
            }
            _ => d.icc_profile = Some(std::sync::Arc::new(vec![1, 2, 3, 4])),
        }
        d.layers.push(layer);
        let pdf = export(&d, true, &Interrupt::NONE).unwrap().bytes;
        let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
        assert!(!content.contains("/ActualText"), "case {case} must retain the raster composite");
        assert!(stream_after(&pdf, "/Subtype /Image").iter().any(|v| *v != 255), "case {case} must retain visible text");
        if case == 4 {
            assert!(has(&pdf, "[/ICCBased 7 0 R]"));
        }
    }
}

/// 300 distinct characters: more than the selection font's 255 one-byte codes.
fn many_distinct_chars() -> String {
    let text: String = (0x100u32..0x22C).filter_map(char::from_u32).collect();
    assert_eq!(text.chars().count(), 300);
    text
}

#[test]
fn text_outside_the_vector_prefix_does_not_consume_the_selection_font() {
    use photocraft_doc::TextLayer;
    let many = many_distinct_chars();
    for case in 0..2 {
        let mut d = doc(ColorMode::Rgb);
        // Hidden in the first case; in the second a raster layer with reduced opacity sits above
        // it, so scanning stops before the oversized repertoire.
        let mut lower = Layer::new("Lower text", LayerContent::Text(TextLayer { text: many.clone(), size_pt: 12.0, ..Default::default() }));
        lower.visible = case == 1;
        d.layers.push(lower);
        if case == 1 {
            let fmt = d.pixel_format();
            let mut blocker = Layer::new("Blocker", LayerContent::Raster(solid(fmt, 0, 0, 4, 2, &[0.0, 0.0, 0.0, 0.5])));
            blocker.opacity = 0.5;
            d.layers.push(blocker);
        }
        d.layers.push(Layer::new(
            "Top text",
            LayerContent::Text(TextLayer { text: "Hi".into(), size_pt: 12.0, color: photocraft_color::Color::rgb(0.0, 1.0, 0.0), ..Default::default() }),
        ));
        let pdf = export(&d, true, &Interrupt::NONE).unwrap().bytes;
        let parsed = lopdf::Document::load_mem(&pdf).unwrap();
        assert_eq!(parsed.extract_text(&[1]).unwrap().trim(), "Hi", "case {case}: only the top text is selectable");
        let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
        assert!(content.contains("/ActualText <FEFF0048"), "case {case}: the top text must stay vector:\n{content}");
        assert!(content.contains("/FSelect"), "case {case}");
    }
}

#[test]
fn a_text_layer_refused_at_emission_keeps_the_upper_vectors_and_stays_raster() {
    use photocraft_doc::{TextLayer, text::TextShape};
    let mut d = doc(ColorMode::Rgb);
    d.size = Size { width: 128, height: 48 };
    let fmt = d.pixel_format();
    d.layers = vec![Layer::new("Paper", LayerContent::Raster(solid(fmt, 0, 0, 128, 48, &[1.0, 1.0, 1.0, 1.0])))];
    // A box too short for its paragraph: the layout hides the overflow, so `prepare` accepts
    // the layer (its checks only see placed clusters) while emission refuses it, because the
    // hidden, unpainted source text would otherwise show up in the selection plane.
    let mut lower = Layer::new(
        "Lower text",
        LayerContent::Text(TextLayer {
            text: "The quick brown fox jumps over the lazy dog again and again".into(),
            font_family: "Inter".into(),
            size_pt: 12.0,
            color: photocraft_color::Color::rgb(1.0, 0.0, 0.0),
            shape: TextShape::Box { x: 0.0, y: 0.0, width: 100.0, height: 30.0 },
            transform: photocraft_geom::Affine::translate(2.0, 10.0),
            ..Default::default()
        }),
    );
    // The compositor consumes the text cache maintained by the engine/UI.
    if let LayerContent::Text(text) = &mut lower.content {
        photocraft_text::TextEngine::new().render_layer(text, d.resolution_dpi, fmt);
    }
    // Pin the fixture to the intended path: accepted by `prepare`, refused by `ops`.
    let LayerContent::Text(lower_text) = &lower.content else { panic!("the fixture is a text layer") };
    let prepared = super::text::prepare(lower_text, ColorMode::Rgb, d.resolution_dpi).expect("prepare accepts the truncated box");
    let lower_font = super::SelectionFont::new(std::iter::once(lower_text.text.as_str()));
    assert!(super::text::ops(&prepared, ColorMode::Rgb, &lower_font).is_none(), "emission must refuse the unrepresented source text");
    d.layers.push(lower);
    d.layers.push(Layer::new(
        "Top text",
        LayerContent::Text(TextLayer {
            text: "Hi".into(),
            font_family: "Inter".into(),
            size_pt: 12.0,
            color: photocraft_color::Color::rgb(0.0, 1.0, 0.0),
            ..Default::default()
        }),
    ));
    let pdf = export(&d, true, &Interrupt::NONE).unwrap().bytes;
    check_structure(&pdf);
    let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
    assert!(content.contains("/ActualText <FEFF0048"), "the upper text stays vector:\n{content}");
    assert!(content.contains("0 1 0 rg"), "the upper text keeps its fill:\n{content}");
    assert!(content.contains("/FSelect"), "the selection font survives the refused layer");
    assert!(!content.contains("1 0 0 rg"), "the refused layer must not be vector:\n{content}");
    assert!(!content.contains("/ActualText <FEFF0054"), "its text must stay out of the selection plane:\n{content}");
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    assert_eq!(parsed.extract_text(&[1]).unwrap().trim(), "Hi", "only the upper text is selectable");
    let img = stream_after(&pdf, "/Subtype /Image");
    assert!(
        img.chunks(3).any(|p| i16::from(p[0]) - i16::from(p[1]) > 8 && i16::from(p[0]) - i16::from(p[2]) > 8),
        "the refused red text stays visible in the raster backdrop"
    );
}

#[test]
fn multiline_text_keeps_its_paragraph_breaks_in_the_selection() {
    use photocraft_doc::TextLayer;
    let mut d = doc(ColorMode::Rgb);
    d.layers.push(Layer::new("Text", LayerContent::Text(TextLayer { text: "A\nB".into(), size_pt: 12.0, ..Default::default() })));
    let pdf = export(&d, true, &Interrupt::NONE).unwrap().bytes;
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    assert_eq!(parsed.extract_text(&[1]).unwrap().trim(), "A\nB", "the paragraph break survives selection");
    let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
    assert!(content.contains("/ActualText <FEFF0041000A"), "the first span carries the break:\n{content}");
}

#[test]
fn leading_newline_stays_in_the_first_selection_span() {
    use photocraft_doc::TextLayer;
    let mut d = doc(ColorMode::Rgb);
    d.layers.push(Layer::new("Text", LayerContent::Text(TextLayer { text: "\nA".into(), size_pt: 12.0, ..Default::default() })));
    let pdf = export(&d, true, &Interrupt::NONE).unwrap().bytes;
    let content = String::from_utf8(stream_after(&pdf, "/Contents 4 0 R")).unwrap();
    assert!(content.contains("/ActualText <FEFF000A0041>"), "the first span retains the leading newline:\n{content}");
}

#[test]
/// Each artboard becomes one labelled page, in document order.
fn artboards_export_as_one_page_each() {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let mut d = Document::new("pages.pdf", Size { width: 40, height: 30 }, ColorMode::Rgb, SampleType::F32);
    d.resolution_dpi = 72.0;
    for (rect, px) in [(Rect::new(0, 0, 20, 30), [1.0, 0.0, 0.0, 1.0]), (Rect::new(20, 0, 40, 30), [0.0, 0.0, 1.0, 1.0])] {
        let layer = Layer::new("Pixels", LayerContent::Raster(solid(fmt, rect.x0, rect.y0, rect.x1, rect.y1, &px)));
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        d.layers.push(Layer::new("Page", LayerContent::Group(group)));
    }
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    let parsed = lopdf::Document::load_mem(&out.bytes).unwrap();
    let pages = parsed.get_pages();
    assert_eq!(pages.len(), 2);
    for id in pages.values() {
        let page = parsed.get_dictionary(*id).unwrap();
        let media = page.get(b"MediaBox").unwrap().as_array().unwrap();
        let w = media[2].as_float().or_else(|_| media[2].as_i64().map(|v| v as f32)).unwrap();
        let h = media[3].as_float().or_else(|_| media[3].as_i64().map(|v| v as f32)).unwrap();
        assert!((w - 20.0).abs() < 0.01 && (h - 30.0).abs() < 0.01, "page {w}x{h}");
    }
    assert_eq!(out.bytes.windows(15).filter(|w| *w == b"/Subtype /Image").count(), 2);
}

/// Side-by-side 2x2 artboards at 72 dpi, named by `names` (empty names are kept).
fn artboard_doc(names: &[&str]) -> Document {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let width = (names.len() as u32).saturating_mul(2).max(2);
    let mut d = Document::new("labels.pdf", Size { width, height: 2 }, ColorMode::Rgb, SampleType::F32);
    d.resolution_dpi = 72.0;
    for (index, name) in names.iter().enumerate() {
        let x0 = (index as i32) * 2;
        let rect = Rect::new(x0, 0, x0 + 2, 2);
        let layer = Layer::new("Pixels", LayerContent::Raster(solid(fmt, rect.x0, rect.y0, rect.x1, rect.y1, &[0.5, 0.5, 0.5, 1.0])));
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        d.layers.push(Layer::new(*name, LayerContent::Group(group)));
    }
    d
}

/// The decompressed samples of page `id`'s image XObject `name` (`Im0` the composite, `Sp0`…
/// the plates).
fn page_xobject(pdf: &lopdf::Document, id: lopdf::ObjectId, name: &[u8]) -> Vec<u8> {
    let page = pdf.get_dictionary(id).expect("page dictionary");
    let resources = page.get(b"Resources").expect("page resources").as_dict().expect("resources dictionary");
    let xobjects = resources.get(b"XObject").expect("xobjects").as_dict().expect("xobject dictionary");
    let image = xobjects.get(name).expect("named XObject").as_reference().expect("image reference");
    pdf.get_object(image).expect("image object").as_stream().expect("image stream").decompressed_content().expect("image samples")
}

/// The decompressed samples of page `id`'s first image XObject (`/Im0`).
fn page_image(pdf: &lopdf::Document, id: lopdf::ObjectId) -> Vec<u8> {
    page_xobject(pdf, id, b"Im0")
}

/// An artboard page carries its own region of every spot plate, after its image and with
/// overprint on; the plates are the document's, described once in the catalog's XMP.
#[test]
fn artboard_pages_carry_their_region_of_each_plate() {
    let mut d = artboard_doc(&["Front", "Back"]);
    // The 4x2 canvas, row-major: ink on the second board (x 2..4) only.
    d.channels.push(spot("PANTONE 185 C", photocraft_color::Color::rgb(0.89, 0.09, 0.22), &[0.0, 0.0, 1.0, 0.5, 0.0, 0.0, 1.0, 0.5]));
    let out = export(&d, true, &Interrupt::NONE).unwrap();
    check_structure(&out.bytes);
    let parsed = lopdf::Document::load_mem(&out.bytes).unwrap();
    let ids: Vec<_> = parsed.get_pages().values().copied().collect();
    assert_eq!(ids.len(), 2);
    assert_eq!(page_xobject(&parsed, ids[0], b"Sp0"), vec![0, 0, 0, 0], "the first board's region has no ink");
    assert_eq!(page_xobject(&parsed, ids[1], b"Sp0"), vec![255, 128, 255, 128], "the second board's region of the plate");
    assert_eq!(page_image(&parsed, ids[1]).len(), 2 * 2 * 3, "the board's composite stays");
    let count = |needle: &str| out.bytes.windows(needle.len()).filter(|w| *w == needle.as_bytes()).count();
    assert_eq!(count("/Separation /PANTONE#20185#20C"), 2, "one plate XObject per page");
    assert_eq!(count("/Type /ExtGState /OP true"), 1, "one shared overprint state");
    let c = content(&out.bytes);
    assert!(c.find("/Im0 Do").unwrap() < c.find("/GS0 gs 2.000 0 0 2.000 0.000 0.000 cm /Sp0 Do").unwrap(), "image, then the plate: {c}");
    assert!(has(&out.bytes, "/PageLabels"), "the boards still name their pages");
    assert_eq!(xmp_of(&out.bytes).matches("<pcsp:name>PANTONE 185 C</pcsp:name>").count(), 1, "one description for the document");
    assert!(out.warnings.iter().any(|w| w.contains("1 spot plate written")), "{:?}", out.warnings);
}

/// A named artboard gives every page a `/PageLabels` entry, and an empty name gets an empty
/// label dictionary rather than inheriting the previous name.
#[test]
fn page_labels_name_every_page_and_an_empty_name_does_not_inherit() {
    let d = artboard_doc(&["A", "", "C"]);
    let pdf = export(&d, true, &Interrupt::NONE).unwrap().bytes;
    check_structure(&pdf);
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    let labels = parsed
        .catalog()
        .unwrap()
        .get(b"PageLabels")
        .expect("a named artboard gives the PDF page labels")
        .as_reference()
        .expect("the label tree is an indirect object");
    let nums = parsed.get_dictionary(labels).unwrap().get(b"Nums").unwrap().as_array().unwrap();
    assert_eq!(nums.len(), 6, "one key and one label dictionary per page");
    for page in 0..3 {
        assert_eq!(nums[page * 2].as_i64().unwrap(), page as i64, "the tree starts at page index 0 and covers every page");
    }
    let label = |page: usize| nums[page * 2 + 1].as_dict().unwrap();
    assert_eq!(label(0).get(b"P").unwrap(), &Object::String(vec![0xFE, 0xFF, 0x00, 0x41], StringFormat::Hexadecimal), "page 1 is A");
    assert!(label(1).get(b"P").is_err(), "the empty name gets an empty dictionary, not A");
    assert_eq!(label(2).get(b"P").unwrap(), &Object::String(vec![0xFE, 0xFF, 0x00, 0x43], StringFormat::Hexadecimal), "page 3 is C");
    assert_eq!(parsed.get_pages().len(), 3);
}

/// With no non-empty name there is no label tree at all: an empty `/Nums` would be invalid.
#[test]
fn an_all_empty_artboard_names_document_has_no_page_labels_tree() {
    let d = artboard_doc(&["", "", ""]);
    let pdf = export(&d, true, &Interrupt::NONE).unwrap().bytes;
    check_structure(&pdf);
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    assert!(parsed.catalog().unwrap().get(b"PageLabels").is_err(), "no names means no /PageLabels");
    assert!(!has(&pdf, "/PageLabels"));
    assert_eq!(parsed.get_pages().len(), 3, "the pages themselves are still written");
}

/// Two artboards at the same rectangle: each page must hold only its own board's pixels, not
/// the other board composited over it.
#[test]
fn overlapping_artboards_export_only_their_own_pixels() {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let mut d = Document::new("overlap.pdf", Size { width: 2, height: 2 }, ColorMode::Rgb, SampleType::F32);
    d.resolution_dpi = 72.0;
    for (name, color) in [("Red", [1.0, 0.0, 0.0, 1.0]), ("Blue", [0.0, 0.0, 1.0, 1.0])] {
        let layer = Layer::new("Pixels", LayerContent::Raster(solid(fmt, 0, 0, 2, 2, &color)));
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(Rect::new(0, 0, 2, 2))), frame_stroke: None };
        d.layers.push(Layer::new(name, LayerContent::Group(group)));
    }
    let pdf = export(&d, true, &Interrupt::NONE).unwrap().bytes;
    let parsed = lopdf::Document::load_mem(&pdf).unwrap();
    let pages = parsed.get_pages();
    let images = pages.values().map(|id| page_image(&parsed, *id)).collect::<Vec<_>>();
    assert_eq!(images.len(), 2);
    assert_eq!(images[0], [255u8, 0, 0].repeat(4), "the bottom board's page is red, not blue over red");
    assert_eq!(images[1], [0u8, 0, 255].repeat(4), "the top board's page is blue");
}

/// A large, mostly empty canvas must not be charged as if its whole area were rendered.
#[test]
fn sparse_canvas_artboards_export_without_a_false_memory_limit() {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::F32, true);
    let mut d = Document::new("sparse.pdf", Size { width: 9000, height: 9000 }, ColorMode::Rgb, SampleType::F32);
    d.resolution_dpi = 72.0;
    for index in 0..2 {
        let x0 = index * 10;
        let rect = Rect::new(x0, 0, x0 + 2, 2);
        let layer = Layer::new("Pixels", LayerContent::Raster(solid(fmt, rect.x0, 0, rect.x1, 2, &[1.0, 0.0, 0.0, 1.0])));
        let group = Group { children: vec![layer], expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        d.layers.push(Layer::new("Page", LayerContent::Group(group)));
    }
    let out = export(&d, true, &Interrupt::NONE).expect("two 2x2 boards on a 9000x9000 canvas must fit the export budget");
    let parsed = lopdf::Document::load_mem(&out.bytes).unwrap();
    assert_eq!(parsed.get_pages().len(), 2);
}

/// Seven US Letter boards at 300 dpi with the importer's gaps: the pages are retained as encoded
/// data, but no whole-canvas buffer exists, so the old whole-canvas charge rejected this.
#[test]
fn seven_us_letter_artboards_export_without_a_false_memory_limit() {
    const PAGE_W: u32 = 2550;
    const PAGE_H: u32 = 3300;
    const GAP: u32 = 80;
    const PAGES: u32 = 7;
    let mut d = Document::new("letters.pdf", Size { width: PAGES * PAGE_W + (PAGES - 1) * GAP, height: PAGE_H }, ColorMode::Rgb, SampleType::F32);
    d.resolution_dpi = 300.0;
    for index in 0..PAGES {
        let x0 = (index * (PAGE_W + GAP)) as i32;
        let rect = Rect::new(x0, 0, x0 + PAGE_W as i32, PAGE_H as i32);
        let group = Group { children: Vec::new(), expanded: true, artboard: Some(Artboard::new(rect)), frame_stroke: None };
        d.layers.push(Layer::new(format!("Letter {}", index + 1), LayerContent::Group(group)));
    }
    let out = export(&d, true, &Interrupt::NONE).expect("seven US Letter pages at 300 dpi fit the export budget");
    let parsed = lopdf::Document::load_mem(&out.bytes).unwrap();
    assert_eq!(parsed.get_pages().len(), 7);
}
