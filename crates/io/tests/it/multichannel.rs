//! Multichannel PSDs: alpha-type channels and the document's resources and global blocks survive
//! an import → export → import round trip.

use photocraft_doc::{ColorIndicates, Knot, NamedPath, Path, PathOp, Subpath};
use photocraft_io::*;
use photocraft_psd::resources::ImageResource;
use photocraft_psd::{ColorMode, Compression, PsdFile, TaggedBlock, Version, testgen};

/// DisplayInfo (1077) record: RGB colour, opacity (0..100) and kind (0/1 alpha, 2 spot).
fn display_record(out: &mut Vec<u8>, rgb: [u16; 3], opacity: u16, kind: u8) {
    out.extend_from_slice(&0u16.to_be_bytes());
    for v in [rgb[0], rgb[1], rgb[2], 0] {
        out.extend_from_slice(&v.to_be_bytes());
    }
    out.extend_from_slice(&opacity.to_be_bytes());
    out.push(kind);
}

/// A two-channel Multichannel file: a cyan spot ink and an alpha channel (masked areas, red 50 %).
fn file(depth: u16) -> PsdFile {
    let mut f = testgen::merged_only(Version::Psd, ColorMode::Multichannel, depth, Compression::Rle, 9, 5);
    let mut info = 1u32.to_be_bytes().to_vec();
    display_record(&mut info, [0, 65535, 65535], 0, 2);
    display_record(&mut info, [65535, 0, 0], 50, 1);
    f.resources.push(ImageResource::new(1077, info));
    f
}

fn roundtrip(d: &photocraft_doc::Document) -> (PsdFile, photocraft_doc::Document) {
    let out = export(d, "x.psd", &ExportOptions::default()).unwrap();
    let back = PsdFile::from_bytes(&out.bytes).unwrap();
    let (doc, _) = psd_to_document(&back);
    (back, doc)
}

#[test]
fn alpha_type_channels_import_as_alpha_and_round_trip() {
    for depth in [8, 16] {
        let f = file(depth);
        let (d, _) = psd_to_document(&f);
        assert_eq!(d.channels.len(), 2);
        assert_eq!(d.channels[0].spot.map(|(c, _)| c.to_rgb()), Some([0.0, 1.0, 1.0]));
        let a = &d.channels[1];
        assert_eq!(a.spot, None, "an alpha-type channel is not a spot ink ({depth}-bit)");
        assert_eq!(a.color.to_rgb(), [1.0, 0.0, 0.0]);
        assert!((a.opacity - 0.5).abs() < 1e-6);
        assert_eq!(a.indicates, ColorIndicates::MaskedAreas);

        // It still prints as black ink, so the composite is what a black spot channel gives. The
        // alpha is stored as-is (1 = selected), so that ink is its inverse, as on the PSD plane.
        let mut as_ink = d.clone();
        let canvas = d.bounds();
        let ink: Vec<f32> = d.channels[1].surface.read_region(canvas).iter().map(|v| 1.0 - v).collect();
        let mut s = photocraft_raster::Surface::new(d.channels[1].surface.format());
        s.write_region(canvas, &ink);
        as_ink.channels[1].surface = s;
        as_ink.channels[1].spot = Some((photocraft_color::Color::BLACK, 0.0));
        // Within f32 rounding: the inverted copy is re-quantised to the channel depth.
        let (shown, inked) = (photocraft_compose::flatten(&d).px, photocraft_compose::flatten(&as_ink).px);
        assert_eq!(shown.len(), inked.len());
        for (a, b) in shown.iter().zip(&inked) {
            assert!(a.iter().zip(b).all(|(x, y)| (x - y).abs() < 1e-5), "{a:?} vs {b:?} ({depth}-bit)");
        }

        // Saved back unchanged: same DisplayInfo, same planes, same channels on reopening.
        let (back, d2) = roundtrip(&d);
        assert_eq!(back.resource(1077).map(|r| r.data.clone()), f.resource(1077).map(|r| r.data.clone()));
        assert_eq!(back.decode_merged().unwrap(), f.decode_merged().unwrap());
        assert_eq!(d2.channels[1].spot, None);
        assert_eq!(d2.channels[1].indicates, ColorIndicates::MaskedAreas);
        assert!((d2.channels[1].opacity - 0.5).abs() < 1e-6);
        assert!(d2.channels[0].spot.is_some());
    }
}

/// Stored samples of one big-endian plane at `depth` (8 or 16), from 0..=1 tone values.
fn plane(vals: &[f32], depth: u16) -> Vec<u8> {
    let max = if depth == 16 { 65535.0 } else { 255.0 };
    vals.iter()
        .flat_map(|v| {
            let s = (v * max).round() as u16;
            if depth == 16 { s.to_be_bytes().to_vec() } else { vec![s as u8] }
        })
        .collect()
}

/// A 4×1 Multichannel file laid out like Photoshop writes one (measured with a file Photoshop saved
/// after RGB → Multichannel): a cyan spot plane stored as ink-inverted tone (0 = solid ink) and a
/// kind-0 alpha plane stored as-is (max = selected), selected in the left half.
fn ink_and_alpha_file(depth: u16) -> PsdFile {
    let mut f = testgen::merged_only(Version::Psd, ColorMode::Multichannel, depth, Compression::Rle, 4, 1);
    let mut planes = plane(&[0.0, 1.0, 0.0, 1.0], depth);
    planes.extend(plane(&[1.0, 1.0, 0.0, 0.0], depth));
    f.image_data = photocraft_psd::ImageData::encode(Compression::Rle, &planes, &f.header).unwrap();
    let mut info = 1u32.to_be_bytes().to_vec();
    display_record(&mut info, [0, 65535, 65535], 0, 2);
    display_record(&mut info, [65535, 0, 0], 50, 0);
    f.resources.push(ImageResource::new(1077, info));
    f
}

fn values(ch: &photocraft_doc::AlphaChannel) -> Vec<f32> {
    ch.surface.read_region(photocraft_geom::Rect::new(0, 0, 4, 1)).chunks_exact(ch.surface.channels()).map(|v| v[0]).collect()
}

#[test]
fn alpha_type_channels_are_stored_as_is_not_as_ink() {
    for depth in [8, 16] {
        let f = ink_and_alpha_file(depth);
        let (d, _) = psd_to_document(&f);
        // A white (selected) alpha plane imports as 1.0, like an alpha channel of any other mode;
        // the spot plane is still read as ink density (stored 0 = solid ink).
        assert_eq!(d.channels[1].spot, None);
        assert_eq!(values(&d.channels[1]), [1.0, 1.0, 0.0, 0.0], "alpha channel ({depth}-bit)");
        assert_eq!(values(&d.channels[0]), [1.0, 0.0, 1.0, 0.0], "spot channel ({depth}-bit)");

        // The composite is what Photoshop shows: the alpha prints as black ink where unselected.
        let px = photocraft_compose::flatten(&d).px;
        assert_eq!(px.iter().map(|p| [p[0], p[1], p[2]]).collect::<Vec<_>>(), [[0.0, 1.0, 1.0], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0], [0.0, 0.0, 0.0]]);

        // Saved back byte for byte.
        let (back, d2) = roundtrip(&d);
        assert_eq!(back.decode_merged().unwrap(), f.decode_merged().unwrap(), "{depth}-bit");
        assert_eq!(values(&d2.channels[1]), [1.0, 1.0, 0.0, 0.0]);
    }
}

#[test]
fn a_full_alpha_channel_exports_as_a_full_plane() {
    use photocraft_color::{Color, ColorMode as Mode, PixelFormat, SampleType};
    for (sample, depth) in [(SampleType::U8, 8u16), (SampleType::U16, 16)] {
        let mut doc = photocraft_doc::Document::new("m", photocraft_doc::Size::new(4, 1), Mode::Multichannel, sample);
        let fmt = PixelFormat::new(Mode::Grayscale, sample, false);
        let surface = |v: f32| {
            let mut s = photocraft_raster::Surface::new(fmt);
            s.write_region(photocraft_geom::Rect::new(0, 0, 4, 1), &[v; 4]);
            s
        };
        doc.channels.push(photocraft_doc::AlphaChannel { spot: Some((Color::BLACK, 0.0)), ..photocraft_doc::AlphaChannel::new("Black", surface(1.0)) });
        doc.channels.push(photocraft_doc::AlphaChannel::new("Alpha 1", surface(1.0)));
        let (back, _) = roundtrip(&doc);
        let mut want = plane(&[0.0; 4], depth);
        want.extend(plane(&[1.0; 4], depth));
        assert_eq!(back.decode_merged().unwrap(), want, "solid ink is stored 0, a full alpha max ({depth}-bit)");
    }
}

#[test]
fn resources_and_global_blocks_survive_a_save() {
    let mut f = file(8);
    let iptc = vec![0x1c, 2, 0, 0, 2, 0, 4];
    let print_flags = vec![0, 0, 0, 0, 0, 0, 0, 1, 0];
    let seed = 7u32.to_be_bytes().to_vec();
    let alpha_ids = [0u32, 1].iter().flat_map(|v| v.to_be_bytes()).collect::<Vec<u8>>();
    f.resources.push(ImageResource::new(1028, iptc.clone()));
    f.resources.push(ImageResource::new(1011, print_flags.clone()));
    f.resources.push(ImageResource::new(1044, seed.clone()));
    f.resources.push(ImageResource::new(1053, alpha_ids.clone()));
    // A target-layer index describes layers, which a Multichannel document doesn't have.
    f.resources.push(ImageResource::new(1024, vec![0, 0]));
    let fmsk = vec![0, 0, 0, 0, 0, 50];
    f.global_blocks.push(TaggedBlock::new(*b"FMsk", fmsk.clone()));
    let (mut d, _) = psd_to_document(&f);
    d.guides.horizontal.push(2.0);
    d.guides.vertical.push(3.0);
    d.global_light.angle = 60.0;
    let path =
        Path::new(vec![Subpath { closed: true, op: PathOp::Combine, knots: vec![Knot::corner(1.0, 1.0), Knot::corner(8.0, 1.0), Knot::corner(4.0, 4.0)] }]);
    d.paths.push(NamedPath { name: "Outline".into(), path, psd_raw: None });

    let (back, d2) = roundtrip(&d);
    let res = |id: u16| back.resource(id).map(|r| r.data.clone());
    assert_eq!(res(1028), Some(iptc));
    assert_eq!(res(1011), Some(print_flags));
    assert_eq!(res(1044), Some(seed));
    assert_eq!(res(1053), Some(alpha_ids));
    assert_eq!(res(1024), None);
    let blocks: Vec<_> = back.global_blocks.iter().filter(|b| &b.key == b"FMsk").collect();
    assert_eq!(blocks.len(), 1);
    assert_eq!(blocks[0].data, fmsk);
    assert_eq!(d2.guides.horizontal, vec![2.0]);
    assert_eq!(d2.guides.vertical, vec![3.0]);
    assert_eq!(d2.global_light.angle, 60.0);
    // The file's own saved path and the new one.
    let names: Vec<&str> = d2.paths.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Path 1", "Outline"]);
    let anchors = |p: &Path| p.subpaths.iter().flat_map(|s| s.knots.iter().map(|k| (k.anchor.x, k.anchor.y))).collect::<Vec<_>>();
    let (got, want) = (anchors(&d2.paths[1].path), anchors(&d.paths[1].path));
    assert_eq!(got.len(), want.len());
    assert!(got.iter().zip(&want).all(|(a, b)| (a.0 - b.0).abs() < 1e-3 && (a.1 - b.1).abs() < 1e-3), "{got:?}");
    assert_eq!(back.decode_merged().unwrap(), f.decode_merged().unwrap());

    // Alpha identifiers that no longer match the channels are not written.
    d.channels.pop();
    let (back, _) = roundtrip(&d);
    assert!(back.resource(1053).is_none());
}

/// A Photoshop Multichannel file: its alpha-type channel stays an alpha channel and a save keeps
/// every resource except the thumbnail (regenerated) and every document-level block.
#[cfg(feature = "corpus")]
#[test]
fn corpus_multichannel_round_trip_keeps_channels_and_resources() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../corpus/psd/psd-tools/colormodes/4x4_16bit_multichannel.psd");
    assert!(path.is_file(), "{} is missing: run `cargo xtask corpus --all`", path.display());
    let bytes = std::fs::read(&path).unwrap();
    let f = PsdFile::from_bytes(&bytes).unwrap();
    let (d, _) = psd_to_document(&f);
    let alpha: Vec<_> = d.channels.iter().filter(|c| c.spot.is_none()).collect();
    assert!(!alpha.is_empty(), "the file's alpha-type channel imports as an alpha channel");
    for a in &alpha {
        assert_eq!(a.indicates, ColorIndicates::MaskedAreas);
        assert_eq!(a.color.to_rgb(), [1.0, 0.0, 0.0]);
        assert!((a.opacity - 0.5).abs() < 1e-6);
    }
    let (back, d2) = roundtrip(&d);
    assert_eq!(back.resource(1077).map(|r| r.data.clone()), f.resource(1077).map(|r| r.data.clone()));
    assert_eq!(back.decode_merged().unwrap(), f.decode_merged().unwrap());
    assert_eq!(d2.channels.iter().filter(|c| c.spot.is_none()).count(), alpha.len());
    let mut lost: Vec<u16> = f.resources.iter().map(|r| r.id).filter(|id| back.resource(*id).is_none()).collect();
    lost.retain(|id| *id != 1036);
    assert!(lost.is_empty(), "resources lost on save: {lost:?}");
    let keys = |f: &PsdFile| f.global_blocks.iter().map(|b| b.key).collect::<Vec<_>>();
    assert_eq!(keys(&back), keys(&f));
}

#[test]
fn tile_row_conversion_preserves_spot_and_alpha_planes() {
    for depth in [8, 16, 32] {
        let mut f = testgen::merged_only(Version::Psb, ColorMode::Multichannel, depth, Compression::Rle, 5, 600);
        let mut info = 1u32.to_be_bytes().to_vec();
        display_record(&mut info, [0, 0, 0], 0, 2);
        display_record(&mut info, [65535, 0, 0], 50, 1);
        f.resources.push(ImageResource::new(1077, info));
        let raw = f.decode_merged().unwrap();
        let (doc, _) = psd_to_document(&f);
        let sample = usize::from(depth / 8);
        let plane = 5 * 600 * sample;
        for (k, channel) in doc.channels.iter().enumerate() {
            let values = channel.surface.read_region(doc.bounds());
            for (i, value) in values.iter().enumerate() {
                let bytes = &raw[k * plane + i * sample..k * plane + (i + 1) * sample];
                let original = match depth {
                    8 => f32::from(bytes[0]) / 255.0,
                    16 => f32::from(u16::from_be_bytes(bytes.try_into().unwrap())) / 65535.0,
                    _ => f32::from_be_bytes(bytes.try_into().unwrap()),
                };
                let expected = if channel.spot.is_some() { 1.0 - original } else { original };
                assert!((value - expected).abs() < 0.00001, "depth {depth}, channel {k}, pixel {i}");
            }
        }
    }
}
