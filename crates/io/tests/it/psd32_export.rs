//! 32-bit-per-channel PSD export (#291): the layout Photoshop requires to open the file.
//!
//! Photoshop 2026 refuses ("the open options are incorrect") any 32-bit PSD whose Color Mode
//! Data lacks the HDR toning records it writes itself. These tests pin the whole structure
//! of a 32-bit export against what Photoshop saves: header depth, toning records, required
//! resources, layer info in an `Lr32` global block with an empty classic layer info, channel
//! compression codes valid for 32-bit data, and a raw big-endian float merged image.

use crate::common;

use common::Features;
use craft_psd::{Compression, LayerInfoPlacement, PsdFile, hdr};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Size;
use photocraft_io::*;

fn be32(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().unwrap())
}

/// Walks the raw file and returns (classic layer info length, keys of the global blocks).
fn raw_layer_section(bytes: &[u8]) -> Option<(u32, Vec<[u8; 4]>)> {
    let mut p = 26;
    p += 4 + be32(bytes, p) as usize; // color mode data
    p += 4 + be32(bytes, p) as usize; // image resources
    let lm_len = be32(bytes, p) as usize;
    p += 4;
    if lm_len == 0 {
        return None;
    }
    let end = p + lm_len;
    let li_len = be32(bytes, p);
    p += 4 + li_len as usize;
    p += 4 + be32(bytes, p) as usize; // global layer mask
    let mut keys = Vec::new();
    while p + 12 <= end {
        assert!(matches!(&bytes[p..p + 4], b"8BIM" | b"8B64"), "global block signature at {p}");
        keys.push(bytes[p + 4..p + 8].try_into().unwrap());
        let n = be32(bytes, p + 8) as usize;
        // Global blocks are padded to 4 bytes.
        p += 12 + n.div_ceil(4) * 4;
    }
    Some((li_len, keys))
}

/// Asserts the structure of a 32-bit PSD written by our exporter.
fn assert_32bit_layout(name: &str, bytes: &[u8]) {
    let f = PsdFile::from_bytes(bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
    assert_eq!(f.header.depth, 32, "{name}: depth");
    assert_eq!(f.color_mode_data, hdr::default_toning_data(), "{name}: HDR toning color mode data");
    for id in [1005u16, 1057] {
        assert!(f.resource(id).is_some(), "{name}: resource {id} missing");
    }
    assert_eq!(f.has_real_merged_data(), Some(true), "{name}: 1057 real merged data");
    // Merged image: raw big-endian IEEE floats, finite.
    assert_eq!(f.image_data.compression, Compression::Raw, "{name}: merged compression");
    let n = f.header.width as usize * f.header.height as usize * usize::from(f.header.channels);
    assert_eq!(f.image_data.data.len(), n * 4, "{name}: merged size");
    assert!(f.image_data.data.as_chunks::<4>().0.iter().all(|c| f32::from_be_bytes(*c).is_finite()), "{name}: non-finite merged sample");
    if f.layers().is_empty() {
        return;
    }
    // Layers live in an `Lr32` global block; the classic layer info is empty.
    assert!(
        matches!(&f.layer_info_placement, LayerInfoPlacement::GlobalBlock { key, signature, .. } if key == b"Lr32" && signature == b"8BIM"),
        "{name}: placement {:?}",
        f.layer_info_placement
    );
    let (li_len, keys) = raw_layer_section(bytes).unwrap_or_else(|| panic!("{name}: no layer section"));
    assert_eq!(li_len, 0, "{name}: classic layer info length");
    assert_eq!(keys.first(), Some(b"Lr32"), "{name}: global blocks {keys:?}");
    // Channel data: raw or ZIP (RLE is not valid for 32-bit samples), decodable.
    for (i, rec) in f.layers().iter().enumerate() {
        for ch in &rec.channels {
            let c = ch.compression.unwrap_or(Compression::Raw);
            assert!(matches!(c, Compression::Raw | Compression::Zip | Compression::ZipPrediction), "{name}: layer {i} channel {} uses {c:?}", ch.id);
            rec.decode_channel(ch.id, 32, f.header.version).unwrap_or_else(|e| panic!("{name}: layer {i} channel {}: {e}", ch.id));
        }
    }
    let errs = common::strict_block_errors(bytes);
    assert!(errs.is_empty(), "{name}: {errs:?}");
}

fn export_psd(doc: &Document) -> Vec<u8> {
    export(doc, "t.psd", &ExportOptions::default()).unwrap().bytes
}

#[test]
/// Verify raw unmasked HDR planes and independently authored masked Smart caches across PSD and PSB round trips.
fn masked_hdr_smart_filter_cache_survives_psd_and_psb_export() {
    use photocraft_color::{BlendMode, PixelFormat};
    use photocraft_doc::{Layer, LayerContent, LayerMask, SmartFilter, SmartObject, SmartSource};
    use photocraft_geom::Affine;
    use photocraft_raster::Surface;
    use std::sync::Arc;

    // Public document export reconstructs embedded sources in document.pixel_format()
    // (RGBA even for opaque content). Exercise both opaque and varying partial alpha;
    // true non-alpha Surface layouts are covered by compose::filter_mask's unit tests.
    for opaque in [true, false] {
        let mut source = Document::new("varying HDR source", Size::new(5, 3), ColorMode::Rgb, SampleType::F32);
        let mut pixels = Surface::new(source.pixel_format());
        let mut mask = LayerMask::reveal_all();
        mask.surface = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
        for y in 0..3 {
            for x in 0..5 {
                let alpha = if opaque { 1. } else { 0.25 + 0.125 * x as f32 + 0.0625 * y as f32 };
                pixels.write_pixel(x, y, &[-4. + 3. * x as f32, 2. + 4. * (x % 2) as f32, 0.25 + 2. * y as f32, alpha]);
                mask.surface.write_pixel(x, y, &[0.25 * (1 + (x + y) % 3) as f32]);
            }
        }
        let bounds = source.bounds();
        let filtered = photocraft_algo::apply_in(&pixels, &photocraft_algo::FilterParams::GaussianBlur { radius: 1. }, bounds, bounds, None, bounds);
        let base_samples = pixels.read_region(bounds);
        let filtered_samples = filtered.read_region(bounds);
        assert!(base_samples.iter().zip(&filtered_samples).any(|(base, filtered)| (base - filtered).abs() > 0.1), "the actual blur must change the fixture");
        assert!(filtered_samples.as_chunks::<4>().0.iter().any(|p| p[..3].iter().any(|&v| v < 0.)), "retain signed filtered colors");
        assert!(filtered_samples.as_chunks::<4>().0.iter().any(|p| p[..3].iter().any(|&v| v > 1.)), "retain HDR filtered colors");

        // Author the mixed cache independently of mix/unmix. This is a premultiplied
        // weighted sum, so the import and export inverses cannot mask each other's bugs.
        let mut mixed = Surface::new(source.pixel_format());
        for y in 0..3 {
            for x in 0..5 {
                let (base, after) = (pixels.pixel(x, y), filtered.pixel(x, y));
                let coverage = f64::from(mask.value(x, y));
                let before = f64::from(base[3]) * (1. - coverage);
                let after_alpha = f64::from(after[3]) * coverage;
                let alpha = before + after_alpha;
                let mut sample = [0.; 4];
                for c in 0..3 {
                    sample[c] = ((f64::from(base[c]) * before + f64::from(after[c]) * after_alpha) / alpha) as f32;
                }
                sample[3] = alpha as f32;
                mixed.write_pixel(x, y, &sample);
            }
        }
        let mixed_samples = mixed.read_region(bounds);
        assert!(
            mixed_samples.iter().zip(&filtered_samples).any(|(mixed, filtered)| (mixed - filtered).abs() > 0.1),
            "skipping export unmix must fail the raw cache oracle"
        );
        source.layers.push(Layer::new("HDR pixels", LayerContent::Raster(pixels)));
        let bytes = export(&source, "psd", &ExportOptions::default()).unwrap().bytes;
        let mut smart = SmartObject::new(SmartSource::Embedded { file_name: "source.psd".into(), bytes: Arc::new(bytes) }, Affine::IDENTITY, Some(mixed));
        smart.smart_filters.push(SmartFilter {
            command: "filter.blur.gaussianBlur".into(),
            params: serde_json::json!({"radius": 1.}),
            blend: BlendMode::Normal,
            opacity: 1.,
            visible: true,
        });
        smart.filter_mask = Some(mask);
        let mut doc = Document::new("masked HDR Smart object", source.size, source.mode, source.depth);
        doc.layers.push(Layer::new("HDR Smart", LayerContent::Smart(smart)));
        for extension in ["psd", "psb"] {
            let out = export(&doc, extension, &ExportOptions::default()).unwrap();
            assert!(!out.warnings.iter().any(|w| w.contains("smart object written as pixels")), "{:?}", out.warnings);
            assert!(common::strict_block_errors(&out.bytes).is_empty());
            let file = PsdFile::from_bytes(&out.bytes).unwrap();
            assert_eq!(file.header.depth, 32);
            let record = file.layers().iter().find(|record| record.name() == "HDR Smart").unwrap();
            // Inspect the stored planes directly, before the importer can apply the mask.
            // PSD/PSB records must contain filtered, unmasked pixels, not our mixed cache.
            for (channel, id) in [0, 1, 2, -1].into_iter().enumerate() {
                let rect = record.channel_rect(id);
                assert_eq!((rect.left, rect.top, rect.right, rect.bottom), (0, 0, 5, 3));
                let plane = record.decode_channel(id, file.header.depth, file.header.version).unwrap();
                assert_eq!(plane.len(), 15 * 4);
                for (index, bytes) in plane.as_chunks::<4>().0.iter().enumerate() {
                    let actual = f32::from_be_bytes(*bytes);
                    let expected = filtered_samples[index * 4 + channel];
                    assert!(
                        actual.is_finite() && (actual - expected).abs() < 1e-4,
                        "{extension} opaque={opaque} raw pixel {index} channel {id}: {actual} != {expected}"
                    );
                }
            }
            let back = import(&format!("masked.{extension}"), &out.bytes).unwrap().document;
            let LayerContent::Smart(smart) = &back.layers[0].content else { panic!("Smart object was rasterized") };
            assert_eq!(smart.smart_filters.len(), 1);
            assert!(smart.filter_mask.is_some());
            let actual = smart.cache.as_ref().unwrap().read_region(bounds);
            assert_eq!(actual.len(), mixed_samples.len());
            for (index, (&actual, &expected)) in actual.iter().zip(&mixed_samples).enumerate() {
                assert!(actual.is_finite() && (actual - expected).abs() < 1e-4, "{extension} opaque={opaque} mixed sample {index}: {actual} != {expected}");
            }
        }
    }
}

#[test]
fn flat_32bit_export_layout() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let doc = Document::new("flat", Size::new(9, 7), mode, SampleType::F32);
        assert_32bit_layout(&format!("flat {mode:?}"), &export_psd(&doc));
    }
}

#[test]
fn layered_32bit_export_layout() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for (label, features) in [("pixels", Features::PIXELS), ("all", Features::ALL)] {
            let doc = common::gen_doc(mode, SampleType::F32, features);
            assert_32bit_layout(&format!("{mode:?} {label}"), &export_psd(&doc));
        }
    }
}

#[test]
fn only_32bit_exports_carry_toning_data() {
    for depth in [SampleType::U8, SampleType::U16] {
        let doc = common::gen_doc(ColorMode::Rgb, depth, Features::PIXELS);
        let f = PsdFile::from_bytes(&export_psd(&doc)).unwrap();
        assert!(f.color_mode_data.is_empty(), "{depth:?}");
    }
}

#[test]
fn layered_32bit_export_reimports_render_equal() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let doc = common::gen_doc(mode, SampleType::F32, Features::ALL);
        let back = import("t.psd", &export_psd(&doc)).unwrap().document;
        let (a, b) = (photocraft_compose::flatten(&doc).into_px(), photocraft_compose::flatten(&back).into_px());
        assert_eq!(a.len(), b.len());
        let d = common::max_diff(&a, &b);
        assert!(d <= 1e-4, "{mode:?}: render differs by {d}");
    }
}

/// Every 32-bit Photoshop oracle: its Color Mode Data is the record we write, and our export
/// of it has the 32-bit layout, re-imports and renders the same.
#[cfg(feature = "corpus")]
#[test]
fn photoshop_32bit_corpus_round_trip() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/photoshop/adjustments");
    assert!(root.is_dir(), "{} is missing: run `cargo xtask corpus --all`", root.display());
    let mut files = Vec::new();
    for dir in ["rgb32", "gray32"] {
        for e in std::fs::read_dir(root.join(dir)).unwrap() {
            let p = e.unwrap().path();
            if p.extension().is_some_and(|x| x == "psd") {
                files.push(p);
            }
        }
    }
    files.sort();
    assert!(files.len() >= 20, "only {} 32-bit oracles", files.len());
    for p in &files {
        let name = p.strip_prefix(&root).unwrap().display().to_string();
        let bytes = std::fs::read(p).unwrap();
        let ps = PsdFile::from_bytes(&bytes).unwrap();
        assert_eq!(ps.header.depth, 32, "{name}");
        assert_eq!(ps.color_mode_data, hdr::default_toning_data(), "{name}: Photoshop's toning record differs from ours");
        let doc = import(&name, &bytes).unwrap().document;
        let out = export_psd(&doc);
        assert_32bit_layout(&name, &out);
        let back = import(&name, &out).unwrap().document;
        assert_eq!(doc.layer_count(), back.layer_count(), "{name}: layer count");
        let (a, b) = (photocraft_compose::flatten(&doc).into_px(), photocraft_compose::flatten(&back).into_px());
        assert_eq!(a.len(), b.len(), "{name}");
        let d = common::max_diff(&a, &b);
        assert!(d <= 1e-4, "{name}: render differs by {d}");
    }
}

// Supply independent float merged planes: using the RGBA8 preview as the oracle
// silently clips these samples, even though Photoshop stores the full range.
#[test]
fn merged_oracle_preserves_signed_hdr_samples() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        for alpha in [1.0, 0.5] {
            let doc = Document::new("float merged", Size::new(3, 1), mode, SampleType::F32);
            let mut file = PsdFile::from_bytes(&export_psd(&doc)).unwrap();
            let color_channels = if mode == ColorMode::Rgb { 3 } else { 1 };
            file.layer_info = None;
            file.header.channels = color_channels + u16::from(alpha != 1.0);
            file.image_data.compression = Compression::Raw;
            // Photoshop's merged pixels are matted against white when alpha is present.
            file.image_data.data =
                (0..color_channels).flat_map(|_| [-0.125_f32, 0.123456, 4.0].into_iter().flat_map(|v| (v * alpha + 1.0 - alpha).to_be_bytes())).collect();
            if alpha != 1.0 {
                file.image_data.data.extend((0..3).flat_map(|_| alpha.to_be_bytes()));
            }
            let pixels = merged_composite(&file).unwrap();
            for (p, value) in pixels.iter().zip([-0.125, 0.123456, 4.0]) {
                for (got, want) in p.iter().zip([value, value, value, alpha]) {
                    assert!((got - want).abs() < 1e-6, "{mode:?}, alpha {alpha}: {p:?}");
                }
            }
        }
    }
}

#[test]
fn merged_oracle_does_not_treat_extra_channels_as_transparency() {
    for mode in [ColorMode::Rgb, ColorMode::Grayscale] {
        let doc = Document::new("extra channel", Size::new(1, 1), mode, SampleType::F32);
        let mut file = PsdFile::from_bytes(&export_psd(&doc)).unwrap();
        let color_channels = if mode == ColorMode::Rgb { 3 } else { 1 };
        file.header.channels = color_channels + 1;
        file.layer_info = Some(craft_psd::LayerInfo { merged_alpha: false, layers: Vec::new(), padding: None });
        file.image_data.compression = Compression::Raw;
        file.image_data.data = (0..color_channels).flat_map(|_| 2.0_f32.to_be_bytes()).chain(0.0_f32.to_be_bytes()).collect();
        assert!(!file.merged_has_alpha());
        assert_eq!(merged_composite(&file).unwrap(), vec![[2.0, 2.0, 2.0, 1.0]], "{mode:?}");
    }
}

// Photoshop saves a transparent 32-bit document with a positive layer count and an `Mt32`
// ("Saving Merged Transparency") global block: the merged image's first extra channel, named
// "Transparency", is its transparency (psd-tools transparentbg.psd). The oracle reads it as the
// composite's alpha, and opening the file doesn't turn it into an alpha channel.
#[test]
fn mt32_marks_the_merged_transparency() {
    let mut doc = Document::new("transparent", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    let fmt = doc.pixel_format();
    let mut layer = photocraft_doc::Layer::raster("shape", fmt);
    if let Some(s) = layer.surface_mut() {
        s.fill_rect(photocraft_geom::Rect::new(0, 0, 1, 1), &[0.0, 0.0, 0.0, 1.0]);
    }
    doc.layers.push(layer);
    let mut file = PsdFile::from_bytes(&export_psd(&doc)).unwrap();
    file.header.channels = 4;
    if let Some(li) = file.layer_info.as_mut() {
        li.merged_alpha = false;
    }
    file.image_data.compression = Compression::Raw;
    let plane = |v: [f32; 2]| v.into_iter().flat_map(f32::to_be_bytes).collect::<Vec<u8>>();
    file.image_data.data = [plane([0.0, 1.0]), plane([0.0, 1.0]), plane([0.0, 1.0]), plane([1.0, 0.0])].concat();
    let name: Vec<u8> = [0u8, 0, 0, 13].into_iter().chain("Transparency\0".encode_utf16().flat_map(u16::to_be_bytes)).collect();
    file.resources.retain(|r| !matches!(r.id, 1006 | 1045));
    assert!(!file.merged_has_alpha(), "a positive layer count alone");
    file.global_blocks.push(craft_psd::TaggedBlock::new(*b"Mt32", Vec::new()));
    assert!(file.merged_has_alpha());
    file.resources.push(craft_psd::ImageResource::new(1045, name));
    let merged = merged_composite(&file).unwrap();
    assert_eq!(merged.iter().map(|p| p[3]).collect::<Vec<_>>(), vec![1.0, 0.0]);
    let opened = import("transparent.psd", &file.to_bytes().unwrap()).unwrap();
    assert!(opened.document.channels.is_empty(), "{:?}", opened.document.channels.iter().map(|c| &c.name).collect::<Vec<_>>());
}
