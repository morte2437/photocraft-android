//! TIFF Options (#8827): Save Transparency on and off, flat and layered, with and without spot
//! channels, and Save Image Pyramid. Unset Save Transparency is covered in `tiff_layers`.

use crate::common::*;
use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_doc::{AlphaChannel, Document, Layer, LayerContent};
use photocraft_geom::{Rect, Size};
use photocraft_io::*;
use photocraft_psd::tiff::ImageSourceData;
use photocraft_raster::Surface;

/// A 4×4 document whose one layer is opaque red in its middle and transparent around it, with a
/// "White" spot channel inked in its left half when `spot`.
fn art(depth: SampleType, spot: bool) -> Document {
    let mut doc = Document::new("t", Size::new(4, 4), ColorMode::Rgb, depth);
    let fmt = doc.pixel_format();
    let mut s = Surface::new(fmt);
    s.fill_rect(Rect::new(1, 1, 3, 3), &photocraft_raster::from_rgba(&fmt, [1.0, 0.0, 0.0, 1.0]));
    doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(s)));
    if spot {
        let gray = PixelFormat::new(ColorMode::Grayscale, depth, false);
        let mut plate = Surface::new(gray);
        plate.fill_rect(Rect::new(0, 0, 2, 4), &photocraft_raster::from_rgba(&gray, [1.0, 1.0, 1.0, 1.0]));
        let mut white = AlphaChannel::new("White", plate);
        white.spot = Some((Color::rgb(0.8, 0.2, 0.2), 1.0));
        doc.channels.push(white);
    }
    doc
}

fn save(doc: &Document, layers: bool, transparency: Option<bool>) -> ExportResult {
    export(doc, "t.tif", &ExportOptions { tiff_layers: layers, tiff_transparency: transparency, ..Default::default() }).expect("export")
}

fn extra_samples_tag(bytes: &[u8]) -> Vec<u16> {
    let mut dec = tiff::decoder::Decoder::new(std::io::Cursor::new(bytes)).unwrap();
    dec.get_tag_u16_vec(tiff::tags::Tag::ExtraSamples).unwrap_or_default()
}

/// A TIFF's channel resources: the names in 1006, and the byte lengths of DisplayInfo (1077)
/// and the alpha identifiers (1053).
fn channel_resources(bytes: &[u8]) -> (Vec<String>, usize, usize) {
    let tag = photocraft_codecs::tiff_photoshop_tags(bytes).0.expect("resources");
    let (resources, _) = photocraft_psd::tiff::resources_from_bytes(tag).expect("parse resources");
    let data = |id: u16| resources.iter().find(|r| r.id == id).map(|r| r.data.clone()).unwrap_or_default();
    let pascal = data(1006);
    let (mut at, mut names) = (0usize, Vec::new());
    while let Some(&len) = pascal.get(at) {
        names.push(String::from_utf8_lossy(&pascal[at + 1..at + 1 + usize::from(len)]).into_owned());
        at += 1 + usize::from(len);
    }
    (names, data(1077).len(), data(1053).len())
}

/// The first sample of pixel (0, 0) and its alpha, on the 0–1 scale.
fn corner(img: &photocraft_codecs::Image) -> (f32, Option<f32>) {
    let f = img.convert(img.layout(), photocraft_codecs::SampleType::F32);
    let s = f.to_f32_samples().unwrap();
    (s[0], img.layout().has_alpha().then(|| s[img.layout().channels() - 1]))
}

fn layer_alpha(doc: &Document, x: i32, y: i32) -> f32 {
    let LayerContent::Raster(s) = &doc.layers[0].content else { panic!("raster") };
    // Alpha is the last channel (index 4 in CMYK).
    s.pixel(x, y).last().copied().unwrap_or(1.0)
}

#[test]
fn off_mattes_the_composite_on_white_and_layers_keep_theirs() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for spot in [false, true] {
            for layers in [false, true] {
                let case = format!("{depth:?} spot {spot} layers {layers}");
                let doc = art(depth, spot);
                let r = save(&doc, layers, Some(false));
                assert!(!r.warnings.iter().any(|w| w.contains("Save Transparency")), "{case}: the user chose it: {:?}", r.warnings);
                let img = photocraft_codecs::decode(&r.bytes).unwrap();
                assert!(!img.layout().has_alpha(), "{case}");
                assert_eq!(img.meta.extra_samples.len(), usize::from(spot), "{case}");
                assert_eq!(extra_samples_tag(&r.bytes), if spot { vec![0] } else { vec![] }, "{case}");
                assert!((corner(&img).0 - 1.0).abs() < 1e-4, "{case}: transparent corner is white");
                let back = import("t.tif", &r.bytes).unwrap().document;
                assert_eq!(back.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), if spot { vec!["White"] } else { vec![] }, "{case}");
                if layers {
                    assert_eq!(layer_alpha(&back, 0, 0), 0.0, "{case}: the layer keeps its transparency");
                    let (isd, _) = ImageSourceData::from_bytes(photocraft_codecs::tiff_photoshop_tags(&r.bytes).1.unwrap()).unwrap();
                    assert!(!isd.layer_info.unwrap().merged_alpha, "{case}: no merged transparency is claimed");
                    assert!(!isd.global_blocks.iter().any(|b| b.is_merged_transparency_marker()), "{case}");
                }
            }
        }
    }
}

#[test]
fn on_keeps_transparency_ahead_of_spot_channels() {
    for depth in [SampleType::U8, SampleType::U16, SampleType::F32] {
        for layers in [false, true] {
            let case = format!("{depth:?} layers {layers}");
            let doc = art(depth, true);
            let r = save(&doc, layers, Some(true));
            assert!(!r.warnings.iter().any(|w| w.contains("composited over white")), "{case}: {:?}", r.warnings);
            let img = photocraft_codecs::decode(&r.bytes).unwrap();
            assert!(img.layout().has_alpha(), "{case}");
            assert_eq!(img.meta.extra_samples.len(), 1, "{case}");
            // Transparency (unassociated alpha) first, then the spot.
            assert_eq!(extra_samples_tag(&r.bytes), vec![2, 0], "{case}");
            assert_eq!(corner(&img).1, Some(0.0), "{case}");
            let back = import("t.tif", &r.bytes).unwrap().document;
            let names: Vec<&str> = back.channels.iter().map(|c| c.name.as_str()).collect();
            assert_eq!(names, ["White"], "{case}: the transparency is not opened as a channel");
            assert!(back.channels[0].spot.is_some(), "{case}");
            assert_eq!(back.channels[0].surface.pixel(0, 0)[0], 1.0, "{case}: inked");
            assert_eq!(back.channels[0].surface.pixel(3, 0)[0], 0.0, "{case}: bare");
            assert_eq!(layer_alpha(&back, 0, 0), 0.0, "{case}");
            assert_eq!(layer_alpha(&back, 1, 1), 1.0, "{case}");
            // Photoshop pairs the channel resources with every sample past the colour channels,
            // so both paths list the transparency first: a name, a DisplayInfo record and an id.
            let (names, display, ids) = channel_resources(&r.bytes);
            assert_eq!(names, ["Transparency", "White"], "{case}");
            assert_eq!(display, 4 + 2 * 13, "{case}: DisplayInfo records");
            assert_eq!(ids, 2 * 4, "{case}: identifiers");
            if layers {
                // As Photoshop writes it: a positive layer count and `MTrn` first.
                let (isd, _) = ImageSourceData::from_bytes(photocraft_codecs::tiff_photoshop_tags(&r.bytes).1.unwrap()).unwrap();
                assert!(!isd.layer_info.as_ref().unwrap().merged_alpha, "{case}");
                assert_eq!(&isd.global_blocks[0].key, b"MTrn", "{case}");
                // The marker is kept for the next save, as from Photoshop's own file.
                let mut want = doc.clone();
                want.metadata.psd_global_blocks = vec![(*b"8BIM", *b"MTrn", std::sync::Arc::new(Vec::new()))];
                assert_docs_eq(&want, &back);
            }
        }
    }
}

/// Channel names outside ASCII survive a save with Save Transparency on, flat and layered: the
/// Unicode names (1045) carry them, after the transparency's entry.
#[test]
fn on_keeps_non_ascii_channel_names() {
    let mut doc = art(SampleType::U8, true);
    doc.channels[0].name = "Blå".into();
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    doc.channels.push(AlphaChannel::new("Máscara 2", Surface::new(gray)));
    for layers in [false, true] {
        let r = save(&doc, layers, Some(true));
        let back = import("t.tif", &r.bytes).unwrap().document;
        assert_eq!(back.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["Blå", "Máscara 2"], "layers {layers}");
        assert!(back.channels[0].spot.is_some(), "layers {layers}: the spot keeps its ink");
    }
}

#[test]
fn on_keeps_a_quick_mask_in_place() {
    let mut doc = art(SampleType::U8, true);
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mut q = Surface::new(gray);
    q.fill_rect(Rect::new(2, 2, 4, 4), &photocraft_raster::from_rgba(&gray, [1.0, 1.0, 1.0, 1.0]));
    doc.quick_mask = Some(AlphaChannel::new("Quick Mask", q));
    for layers in [false, true] {
        let r = save(&doc, layers, Some(true));
        assert_eq!(extra_samples_tag(&r.bytes), vec![2, 0, 0], "layers {layers}");
        let back = import("t.tif", &r.bytes).unwrap().document;
        assert_eq!(back.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["White"], "layers {layers}");
        let quick = back.quick_mask.as_ref().unwrap_or_else(|| panic!("layers {layers}: quick mask"));
        assert_eq!(quick.surface.pixel(3, 3)[0], 1.0, "layers {layers}");
        assert_eq!(quick.surface.pixel(0, 0)[0], 0.0, "layers {layers}");
    }
}

/// Past 16 channels a layered TIFF keeps the first 16. With Save Transparency on, the name and
/// DisplayInfo lists hold the transparency ahead of them, so trimming keeps 17 entries and
/// every channel still opens under its own name (they used to shift by one).
#[test]
fn on_past_sixteen_channels_keeps_each_channels_name() {
    let mut doc = art(SampleType::U8, false);
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    for i in 1..=18 {
        doc.channels.push(AlphaChannel::new(format!("Alpha {i}"), Surface::new(gray)));
    }
    let r = save(&doc, true, Some(true));
    assert!(r.warnings.iter().any(|w| w.contains("18 extra channels") && w.contains("first 16")), "{:?}", r.warnings);
    let img = photocraft_codecs::decode(&r.bytes).unwrap();
    assert!(img.layout().has_alpha());
    assert_eq!(img.meta.extra_samples.len(), 16);
    let back = import("t.tif", &r.bytes).unwrap().document;
    let want: Vec<String> = (1..=16).map(|i| format!("Alpha {i}")).collect();
    assert_eq!(back.channels.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), want.iter().map(String::as_str).collect::<Vec<_>>());
}

#[test]
fn on_without_transparency_writes_no_alpha() {
    // An opaque document has no transparency to save.
    let mut doc = art(SampleType::U8, false);
    doc.layers[0].locks.transparency = true;
    let fmt = doc.pixel_format();
    if let LayerContent::Raster(s) = &mut doc.layers[0].content {
        s.fill_rect(Rect::new(0, 0, 4, 4), &photocraft_raster::from_rgba(&fmt, [0.0, 0.0, 1.0, 1.0]));
    }
    for layers in [false, true] {
        let r = save(&doc, layers, Some(true));
        let img = photocraft_codecs::decode(&r.bytes).unwrap();
        assert!(!img.layout().has_alpha() || corner(&img).1 == Some(1.0), "layers {layers}");
    }
}

#[test]
fn pyramid_levels_follow_the_full_image_and_open_the_same_document() {
    // Big enough to need a level: 100 px halves to 50.
    let mut doc = Document::new("t", Size::new(100, 100), ColorMode::Rgb, SampleType::U8);
    let fmt = doc.pixel_format();
    let mut s = Surface::new(fmt);
    s.fill_rect(Rect::new(10, 10, 60, 30), &photocraft_raster::from_rgba(&fmt, [1.0, 0.5, 0.0, 1.0]));
    doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(s)));
    let gray = PixelFormat::new(ColorMode::Grayscale, SampleType::U8, false);
    let mut plate = Surface::new(gray);
    plate.fill_rect(Rect::new(0, 0, 50, 100), &photocraft_raster::from_rgba(&gray, [1.0, 1.0, 1.0, 1.0]));
    doc.channels.push(AlphaChannel::new("Alpha 1", plate));
    for layers in [false, true] {
        let opts = |pyramid| ExportOptions {
            tiff_layers: layers,
            encode: photocraft_codecs::EncodeOptions { tiff_pyramid: pyramid, ..Default::default() },
            ..Default::default()
        };
        let with = export(&doc, "t.tif", &opts(true)).unwrap();
        let without = export(&doc, "t.tif", &opts(false)).unwrap();
        let info = photocraft_codecs::tiff_info(&with.bytes).unwrap();
        let kinds: Vec<_> = info.pages.iter().map(|p| (p.kind, p.width)).collect();
        assert_eq!(kinds, [(photocraft_codecs::TiffPageKind::Page, 100), (photocraft_codecs::TiffPageKind::ReducedResolution, 50)], "layers {layers}");
        // The level holds the image at half size: level (15, 10) covers full-size (30..32, 20..22),
        // inside the orange fill; (40, 40) is outside it, composited over white (the alpha
        // channel keeps the transparency out of the file).
        let level = photocraft_codecs::decode_tiff_page(&with.bytes, 1, &photocraft_codecs::DecodeOptions::default()).unwrap();
        assert_eq!(level.dimensions(), (50, 50), "layers {layers}");
        let level = level.convert(level.layout(), photocraft_codecs::SampleType::F32);
        let px = level.to_f32_samples().unwrap();
        for ((x, y), want) in [((15, 10), [1.0f32, 0.5, 0.0]), ((40, 40), [1.0, 1.0, 1.0])] {
            let i = (y * 50 + x) * level.layout().channels();
            for (c, want) in want.iter().enumerate() {
                assert!((px[i + c] - want).abs() < 0.01, "layers {layers} ({x}, {y}) channel {c}: {}", px[i + c]);
            }
        }
        let a = import("t.tif", &with.bytes).unwrap();
        let b = import("t.tif", &without.bytes).unwrap();
        assert!(!a.warnings.iter().any(|w| w.contains("page")), "layers {layers}: {:?}", a.warnings);
        assert_docs_eq(&b.document, &a.document);
    }
}

/// `PHOTOCRAFT_TIFF_INTEROP=<dir>` writes every TIFF Options combination there, to check with
/// other readers (libtiff `tiffinfo`, ImageMagick, vips):
///
/// ```sh
/// PHOTOCRAFT_TIFF_INTEROP=/tmp/tiff cargo test -p photocraft-io --test it -- tiff_save_options::write_interop_samples --ignored
/// ```
#[test]
#[ignore = "writes sample files for other readers; set PHOTOCRAFT_TIFF_INTEROP"]
fn write_interop_samples() {
    let Some(dir) = std::env::var_os("PHOTOCRAFT_TIFF_INTEROP") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (mode, mname) in [(ColorMode::Rgb, "rgb"), (ColorMode::Cmyk, "cmyk"), (ColorMode::Grayscale, "gray")] {
        for (depth, dname) in [(SampleType::U8, "8"), (SampleType::U16, "16"), (SampleType::F32, "32")] {
            if mode == ColorMode::Cmyk && depth == SampleType::F32 {
                continue;
            }
            let mut doc = Document::new("t", Size::new(300, 200), mode, depth);
            let fmt = doc.pixel_format();
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(40, 30, 260, 170), &photocraft_raster::from_rgba(&fmt, [0.9, 0.2, 0.1, 1.0]));
            doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(s)));
            for spot in [false, true] {
                if spot {
                    let gray = PixelFormat::new(ColorMode::Grayscale, depth, false);
                    let mut plate = Surface::new(gray);
                    plate.fill_rect(Rect::new(0, 0, 150, 200), &photocraft_raster::from_rgba(&gray, [1.0, 1.0, 1.0, 1.0]));
                    let mut white = AlphaChannel::new("White", plate);
                    white.spot = Some((Color::rgb(0.8, 0.2, 0.2), 1.0));
                    doc.channels = vec![white];
                }
                for layers in [false, true] {
                    for (t, tname) in [(None, "unset"), (Some(true), "on"), (Some(false), "off")] {
                        for (c, cname) in [
                            (photocraft_codecs::TiffCompression::None, "none"),
                            (photocraft_codecs::TiffCompression::Lzw, "lzw"),
                            (photocraft_codecs::TiffCompression::Deflate, "zip"),
                        ] {
                            for pyramid in [false, true] {
                                let opts = ExportOptions {
                                    tiff_layers: layers,
                                    tiff_transparency: t,
                                    encode: photocraft_codecs::EncodeOptions { tiff_compression: c, tiff_pyramid: pyramid, ..Default::default() },
                                    ..Default::default()
                                };
                                let r = export(&doc, "t.tif", &opts).unwrap();
                                let name = format!(
                                    "{mname}{dname}-{}-{}-t{tname}-{cname}{}.tif",
                                    if spot { "spot" } else { "plain" },
                                    if layers { "layered" } else { "flat" },
                                    if pyramid { "-pyramid" } else { "" }
                                );
                                std::fs::write(dir.join(name), r.bytes).unwrap();
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Save Transparency in every colour mode a TIFF holds: off mattes on the mode's white (paper
/// for CMYK, L* 100 for Lab), on keeps the transparency; flat and, where the mode keeps layers,
/// layered.
#[test]
fn every_colour_mode_saves_or_mattes_its_transparency() {
    for mode in [ColorMode::Rgb, ColorMode::Cmyk, ColorMode::Grayscale, ColorMode::Lab] {
        for depth in [SampleType::U8, SampleType::U16] {
            let mut doc = Document::new("t", Size::new(4, 4), mode, depth);
            let fmt = doc.pixel_format();
            let mut s = Surface::new(fmt);
            s.fill_rect(Rect::new(1, 1, 3, 3), &photocraft_raster::from_rgba(&fmt, [0.2, 0.4, 0.6, 1.0]));
            doc.layers.push(Layer::new("Layer 1", LayerContent::Raster(s)));
            for layers in [false, true] {
                for t in [Some(true), Some(false)] {
                    let case = format!("{mode:?} {depth:?} layers {layers} {t:?}");
                    let r = save(&doc, layers, t);
                    let img = photocraft_codecs::decode(&r.bytes).unwrap_or_else(|e| panic!("{case}: {e}"));
                    assert_eq!(img.layout().has_alpha(), t == Some(true), "{case}");
                    let back = import("t.tif", &r.bytes).unwrap().document;
                    assert_eq!(back.mode, mode, "{case}");
                    if t == Some(false) {
                        // The merged image (what other readers show) is matted on the mode's own
                        // white, flat or layered: CMYK paper is no ink, Lab white L* 100 with a*
                        // and b* at their midpoint.
                        let white: &[f32] = match mode {
                            ColorMode::Cmyk => &[0.0, 0.0, 0.0, 0.0],
                            ColorMode::Lab => &[1.0, 128.0 / 255.0, 128.0 / 255.0],
                            ColorMode::Grayscale => &[1.0],
                            _ => &[1.0, 1.0, 1.0],
                        };
                        let f = img.convert(img.layout(), photocraft_codecs::SampleType::F32);
                        let merged = f.to_f32_samples().unwrap();
                        assert_eq!(img.layout().channels(), white.len(), "{case}");
                        assert!(merged.iter().zip(white).all(|(a, b)| (a - b).abs() < 1e-2), "{case}: merged {:?} vs {white:?}", &merged[..white.len()]);
                        // Only the transparent corner is matted: the opaque centre at (1, 1)
                        // (sample 5 of the 4-px-wide image) keeps its colour.
                        let centre = &merged[5 * white.len()..6 * white.len()];
                        assert!(centre.iter().zip(white).any(|(a, b)| (a - b).abs() >= 1e-2), "{case}: centre {centre:?} is matte white");
                        // The corner was transparent: now the mode's white, read back opaque.
                        let LayerContent::Raster(s) = &back.layers[0].content else { panic!("raster") };
                        let want = photocraft_raster::from_rgba(&back.pixel_format(), [1.0, 1.0, 1.0, 1.0]);
                        if !layers {
                            let px = s.pixel(0, 0);
                            assert!(px.iter().zip(&want).all(|(a, b)| (a - b).abs() < 1e-2), "{case}: {px:?} vs {want:?}");
                        }
                    } else {
                        assert_eq!(layer_alpha(&back, 0, 0), 0.0, "{case}");
                    }
                    assert_eq!(layer_alpha(&back, 1, 1), 1.0, "{case}");
                }
            }
        }
    }
}
