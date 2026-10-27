//! Camera raw import: synthetic DNG / CR2 develop into a 16-bit ProPhoto
//! document; unsupported raw variants fall back to the embedded preview.

use photocraft_codecs::{ChannelLayout, EncodeOptions, Format, Image};
use photocraft_color::{ColorMode, SampleType};
use photocraft_io::{IoError, import};
use photocraft_raw::testgen::{Cr2Spec, DngSpec, TiffBuilder, Val, mosaic, scene};

#[test]
fn dng_opens_as_16_bit_prophoto() {
    let (w, h) = (40, 24);
    let mut spec = DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535));
    spec.as_shot_neutral = Some([0.6, 1.0, 0.8]);
    let r = import("shot.dng", &spec.build()).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height), (40, 24));
    assert_eq!(d.mode, ColorMode::Rgb);
    assert_eq!(d.depth, SampleType::U16);
    assert_eq!(d.layers.len(), 1);
    let icc = d.icc_profile.as_ref().expect("profile");
    assert_eq!(icc.as_slice(), &photocraft_cms::Builtin::ProPhotoCompat.profile().to_bytes()[..]);
    assert!(r.warnings.iter().any(|w| w.contains("DNG") && w.contains("ProPhoto")), "{:?}", r.warnings);
}

/// #8125: a lossy DNG's floating-point LinearRaw data, JPEG XL-compressed, is developed (not
/// replaced by a preview). The test codestream is an integer one the reader takes as floats.
#[test]
fn floating_point_jpeg_xl_dng_is_developed() {
    use photocraft_raw::testgen::DngStorage;
    use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace, options::EncoderOptions};
    let (w, h) = (24, 16);
    let data: Vec<u16> = scene(w, h).iter().flat_map(|p| p.map(|v| (v * 20000.0) as u16)).collect();
    let bytes: Vec<u8> = data.iter().flat_map(|v| v.to_ne_bytes()).collect();
    let mut segment = Vec::new();
    zune_jpegxl::JxlSimpleEncoder::new(&bytes, EncoderOptions::new(w, h, ColorSpace::RGB, BitDepth::Sixteen)).encode(&mut segment).unwrap();
    let mut spec = DngSpec::cfa(w, h, data);
    spec.samples = 3;
    spec.white = 0;
    spec.storage = DngStorage::JxlStrips { rows: h };
    spec.jxl_segments = vec![segment];
    spec.raw_extra = vec![(339, Val::Short(vec![3; 3]))];
    let r = import("lossy.dng", &spec.build()).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height, r.document.depth), (24, 16, SampleType::U16));
    assert!(r.warnings.iter().all(|w| !w.contains("preview")), "{:?}", r.warnings);
}

/// #3497: a monochrome (one-sample LinearRaw) DNG, as VueScan writes film scans, opens as a
/// 16-bit RGB document (neutral grey) instead of failing as unsupported.
#[test]
fn monochrome_dng_opens() {
    let (w, h) = (40, 24);
    let data: Vec<u16> = (0..w * h).map(|i| (i % w * 1600) as u16).collect();
    let r = import("S261009_90_67.dng", &DngSpec::monochrome(w, h, data).build()).unwrap();
    let d = &r.document;
    assert_eq!((d.size.width, d.size.height, d.mode, d.depth), (40, 24, ColorMode::Rgb, SampleType::U16));
    assert!(r.warnings.iter().all(|w| !w.contains("colour matrix") && !w.contains("preview")), "{:?}", r.warnings);
}

#[test]
fn cr2_opens() {
    let (w, h) = (48, 16);
    let data = mosaic(&scene(w, h), w, [0, 1, 1, 2], 256, 12000);
    let spec =
        Cr2Spec { width: w, height: h, data, precision: 14, components: 2, slices: vec![24, 24], borders: None, wb_rggb: None, orientation: 6, model_id: None };
    let r = import("IMG_0001.CR2", &spec.build()).unwrap();
    // Orientation 6 rotates the 48×16 sensor image to 16×48.
    assert_eq!((r.document.size.width, r.document.size.height), (16, 48));
    assert_eq!(r.document.depth, SampleType::U16);
}

/// #6957: a developed raw keeps the raw's EXIF (camera, exposure), so File Info shows its Camera
/// Data. The raw's Orientation is left out: the developed image is already upright.
#[test]
fn a_developed_raw_keeps_its_exif() {
    let (w, h) = (48, 16);
    let data = mosaic(&scene(w, h), w, [0, 1, 1, 2], 256, 12000);
    let spec =
        Cr2Spec { width: w, height: h, data, precision: 14, components: 2, slices: vec![24, 24], borders: None, wb_rggb: None, orientation: 6, model_id: None };
    let r = import("IMG_0001.CR2", &spec.build()).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)), "developed, not a preview: {:?}", r.warnings);
    let exif = r.document.metadata.exif.as_deref().expect("the raw's EXIF");
    let camera = photocraft_algo::exif::read(exif);
    assert_eq!(camera.make.as_deref(), Some("Canon"));
    assert_eq!(camera.model.as_deref(), Some("Canon EOS Synthetic"));
    assert_eq!(camera.exposure_time, Some(0.01));
    assert_eq!(photocraft_codecs::exif_orientation(exif), 1, "no orientation to apply a second time");
    assert_eq!((r.document.size.width, r.document.size.height), (16, 48));
}

/// The tags of the Exif directory of an EXIF block (a little-endian TIFF stream).
fn exif_ifd_tags(b: &[u8]) -> Vec<u16> {
    assert_eq!(&b[..2], b"II");
    let u16_at = |i: usize| u16::from_le_bytes([b[i], b[i + 1]]);
    let u32_at = |i: usize| u32::from_le_bytes([b[i], b[i + 1], b[i + 2], b[i + 3]]) as usize;
    let entries = |at: usize| (0..usize::from(u16_at(at))).map(move |k| at + 2 + 12 * k);
    let ifd0 = u32_at(4);
    let exif = entries(ifd0).find(|&e| u16_at(e) == 34665).map(|e| u32_at(e + 8)).expect("an Exif directory");
    entries(exif).map(u16_at).collect()
}

/// A Panasonic RW2 (its TIFF header has 0x55 for 42) with an Exif directory: the developed
/// document keeps it, without the Exif pixel dimensions (the sensor's, not the image's).
#[test]
fn a_developed_rw2_keeps_its_exif() {
    let (w, h, bits) = (20usize, 12usize, 12u32);
    let data: Vec<u16> = (0..w * h).map(|i| (i * 13 % 4000) as u16 + 200).collect();
    let mut t = TiffBuilder::default();
    let raw = t.blob(photocraft_raw::testgen::rw2_pack(&data, w, bits));
    let exif = t.ifd(vec![(33434, Val::Rational(vec![(1, 60)])), (40962, Val::Long(vec![w as u32])), (40963, Val::Long(vec![h as u32]))]);
    let white = ((1u32 << bits) - 1) as u16;
    let ifd0 = t.ifd(vec![
        (0x0002, Val::Short(vec![w as u16])),
        (0x0003, Val::Short(vec![h as u16])),
        (0x0004, Val::Short(vec![2])),
        (0x0005, Val::Short(vec![2])),
        (0x0006, Val::Short(vec![h as u16 - 2])),
        (0x0007, Val::Short(vec![w as u16 - 2])),
        (0x0009, Val::Short(vec![1])),
        (0x000A, Val::Short(vec![bits as u16])),
        (0x000E, Val::Short(vec![white])),
        (0x000F, Val::Short(vec![white])),
        (0x0010, Val::Short(vec![white])),
        (0x002D, Val::Short(vec![5])),
        (0x010F, Val::Ascii("Panasonic".into())),
        (0x0110, Val::Ascii("DC-Synthetic".into())),
        (0x0112, Val::Short(vec![1])),
        (0x0118, Val::Blobs(vec![raw])),
        (34665, Val::Ifds(vec![exif])),
    ]);
    t.chain = vec![ifd0];
    let mut bytes = t.build();
    bytes[2..4].copy_from_slice(b"U\0");
    let r = import("P1000001.RW2", &bytes).unwrap();
    assert!(r.warnings.iter().any(|w| w.contains(photocraft_io::raw::DEVELOPED_NOTE)), "developed, not a preview: {:?}", r.warnings);
    let exif = r.document.metadata.exif.as_deref().expect("the raw's EXIF");
    let camera = photocraft_algo::exif::read(exif);
    assert_eq!((camera.make.as_deref(), camera.model.as_deref()), (Some("Panasonic"), Some("DC-Synthetic")));
    assert!(camera.exposure_time.is_some_and(|t| (t - 1.0 / 60.0).abs() < 1e-9));
    let tags = exif_ifd_tags(exif);
    assert!(tags.contains(&33434), "{tags:?}");
    assert!(!tags.contains(&40962) && !tags.contains(&40963), "no Exif pixel dimensions: {tags:?}");
}

/// A raw whose Exif directory holds only the pixel dimensions, which a developed image drops:
/// IFD0's camera make and model are kept on their own rather than lost with the empty directory.
#[test]
fn a_developed_raw_keeps_its_camera_when_the_exif_directory_empties() {
    let (w, h, bits) = (20usize, 12usize, 12u32);
    let data: Vec<u16> = (0..w * h).map(|i| (i * 13 % 4000) as u16 + 200).collect();
    let mut t = TiffBuilder::default();
    let raw = t.blob(photocraft_raw::testgen::rw2_pack(&data, w, bits));
    let exif = t.ifd(vec![(40962, Val::Long(vec![w as u32])), (40963, Val::Long(vec![h as u32]))]);
    let white = ((1u32 << bits) - 1) as u16;
    let ifd0 = t.ifd(vec![
        (0x0002, Val::Short(vec![w as u16])),
        (0x0003, Val::Short(vec![h as u16])),
        (0x0004, Val::Short(vec![2])),
        (0x0005, Val::Short(vec![2])),
        (0x0006, Val::Short(vec![h as u16 - 2])),
        (0x0007, Val::Short(vec![w as u16 - 2])),
        (0x0009, Val::Short(vec![1])),
        (0x000A, Val::Short(vec![bits as u16])),
        (0x000E, Val::Short(vec![white])),
        (0x000F, Val::Short(vec![white])),
        (0x0010, Val::Short(vec![white])),
        (0x002D, Val::Short(vec![5])),
        (0x010F, Val::Ascii("Panasonic".into())),
        (0x0110, Val::Ascii("DC-Synthetic".into())),
        (0x0112, Val::Short(vec![1])),
        (0x0118, Val::Blobs(vec![raw])),
        (34665, Val::Ifds(vec![exif])),
    ]);
    t.chain = vec![ifd0];
    let mut bytes = t.build();
    bytes[2..4].copy_from_slice(b"U\0");
    let r = import("P1000001.RW2", &bytes).unwrap();
    let exif = r.document.metadata.exif.as_deref().expect("the raw's camera, without an Exif directory");
    let camera = photocraft_algo::exif::read(exif);
    assert_eq!((camera.make.as_deref(), camera.model.as_deref()), (Some("Panasonic"), Some("DC-Synthetic")));
    assert_eq!(camera.exposure_time, None);
}

/// A NEF-like file with Nikon's (undocumented) compression and a full-size
/// baseline JPEG preview in IFD0.
fn nef_with_preview() -> Vec<u8> {
    let img = Image::from_u8(32, 20, ChannelLayout::Rgb, vec![180; 32 * 20 * 3]).unwrap();
    let jpeg = photocraft_codecs::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let mut t = TiffBuilder::default();
    let strip = t.blob(vec![0; 64]);
    let preview = t.blob(jpeg.clone());
    let raw = t.ifd(vec![
        (256, Val::Long(vec![8])),
        (257, Val::Long(vec![8])),
        (258, Val::Short(vec![12])),
        (259, Val::Short(vec![34713])),
        (262, Val::Short(vec![32803])),
        (273, Val::Blobs(vec![strip])),
        (279, Val::Long(vec![64])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(vec![0, 1, 1, 2])),
    ]);
    let ifd0 = t.ifd(vec![
        (271, Val::Ascii("NIKON CORPORATION".into())),
        (330, Val::Ifds(vec![raw])),
        (513, Val::Blobs(vec![preview])),
        (514, Val::Long(vec![jpeg.len() as u32])),
    ]);
    t.chain = vec![ifd0];
    t.build()
}

#[test]
fn unsupported_raw_falls_back_to_the_embedded_preview() {
    let r = import("DSC_0001.NEF", &nef_with_preview()).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (32, 20));
    assert!(r.warnings.first().is_some_and(|w| w.contains("Nikon compressed NEF") && w.contains("embedded")), "{:?}", r.warnings);
}

/// A DNG 1.7 laid out as converters write one with JPEG XL raw data: an 8×6 JPEG thumbnail in
/// IFD0, raw data this reader refuses (here floating-point LinearRaw), and a 40×30 preview
/// compressed with JPEG XL, stored as the raw is (IFD0 Orientation 6).
fn dng_with_jxl_preview() -> Vec<u8> {
    use zune_core::{bit_depth::BitDepth, colorspace::ColorSpace, options::EncoderOptions};
    let thumb = Image::from_u8(8, 6, ChannelLayout::Rgb, vec![90; 8 * 6 * 3]).unwrap();
    let thumb = photocraft_codecs::encode(&thumb, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let (w, h) = (40usize, 30usize);
    let rgb: Vec<u8> = (0..w * h).flat_map(|i| [(i % w * 6) as u8, (i / w * 8) as u8, 120]).collect();
    let mut preview = Vec::new();
    zune_jpegxl::JxlSimpleEncoder::new(&rgb, EncoderOptions::new(w, h, ColorSpace::RGB, BitDepth::Eight)).encode(&mut preview).unwrap();
    let mut t = TiffBuilder::default();
    let raw_strip = t.blob(vec![0; 4 * 4 * 3 * 2]);
    let raw = t.ifd(vec![
        (254, Val::Long(vec![0])),
        (256, Val::Long(vec![4])),
        (257, Val::Long(vec![4])),
        (258, Val::Short(vec![16, 16, 16])),
        (259, Val::Short(vec![1])),
        (262, Val::Short(vec![34892])),
        (273, Val::Blobs(vec![raw_strip])),
        (277, Val::Short(vec![3])),
        (278, Val::Long(vec![4])),
        (279, Val::Long(vec![4 * 4 * 3 * 2])),
        (339, Val::Short(vec![3, 3, 3])),
    ]);
    let preview_len = preview.len() as u32;
    let preview_strip = t.blob(preview);
    let preview = t.ifd(vec![
        (254, Val::Long(vec![1])),
        (256, Val::Long(vec![w as u32])),
        (257, Val::Long(vec![h as u32])),
        (258, Val::Short(vec![8, 8, 8])),
        (259, Val::Short(vec![52546])),
        (262, Val::Short(vec![2])),
        (273, Val::Blobs(vec![preview_strip])),
        (277, Val::Short(vec![3])),
        (278, Val::Long(vec![h as u32])),
        (279, Val::Long(vec![preview_len])),
        (50970, Val::Long(vec![2])),
    ]);
    let thumb_len = thumb.len() as u32;
    let thumb_strip = t.blob(thumb);
    let ifd0 = t.ifd(vec![
        (254, Val::Long(vec![1])),
        (256, Val::Long(vec![8])),
        (257, Val::Long(vec![6])),
        (258, Val::Short(vec![8, 8, 8])),
        (259, Val::Short(vec![7])),
        (262, Val::Short(vec![6])),
        (271, Val::Ascii("Synthetic".into())),
        (273, Val::Blobs(vec![thumb_strip])),
        (274, Val::Short(vec![6])),
        (277, Val::Short(vec![3])),
        (278, Val::Long(vec![6])),
        (279, Val::Long(vec![thumb_len])),
        (330, Val::Ifds(vec![raw, preview])),
        (50706, Val::Byte(vec![1, 7, 0, 0])),
    ]);
    t.chain = vec![ifd0];
    t.build()
}

/// #8125: a DNG whose raw data isn't supported opens its full JPEG XL preview, upright, not
/// the small JPEG thumbnail.
#[test]
fn unsupported_dng_falls_back_to_its_jpeg_xl_preview() {
    let r = import("photo.dng", &dng_with_jxl_preview()).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (30, 40), "the 40×30 preview, turned upright: {:?}", r.warnings);
    let w = r.warnings.first().map_or("", String::as_str);
    assert!(w.contains("floating-point") && w.contains("40x30 JPEG XL preview"), "{:?}", r.warnings);
}

#[test]
fn unsupported_raw_without_preview_is_a_clear_error() {
    let mut cr3 = vec![0, 0, 0, 24];
    cr3.extend_from_slice(b"ftypcrx ");
    cr3.extend_from_slice(&[0; 12]);
    match import("IMG_0001.CR3", &cr3) {
        Err(e @ IoError::Raw(_)) => assert!(e.to_string().contains("CR3"), "{e}"),
        Err(e) => panic!("expected a raw error, got {e}"),
        Ok(_) => panic!("CR3 must not decode yet"),
    }
}

#[test]
fn cr3_opens_its_upright_camera_jpeg_with_explicit_preview_warning() {
    let img = Image::from_u8(32, 20, ChannelLayout::Rgb, vec![180; 32 * 20 * 3]).unwrap();
    let jpeg = photocraft_codecs::encode(&img, Format::Jpeg, &EncodeOptions::default()).unwrap();
    let raw = photocraft_raw::testgen::cr3_preview(&jpeg, 6);
    let r = import("shot.cr3", &raw).unwrap();
    assert_eq!((r.document.size.width, r.document.size.height), (20, 32));
    assert_eq!(r.document.depth, SampleType::U8);
    assert!(r.warnings[0].contains("8-bit, not the raw sensor data"));
    assert!(r.warnings[0].contains("CR3"));
}

#[test]
fn truncated_raws_do_not_panic() {
    let b = nef_with_preview();
    for n in (0..b.len()).step_by(7) {
        let _ = import("x.nef", &b[..n]);
    }
}

/// Camera-space patches, each with a tonal ramp.
fn patches(w: usize, h: usize) -> Vec<[f32; 3]> {
    let mut seed = 99u32;
    let mut rnd = || {
        seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (seed >> 8) as f32 / (1u32 << 24) as f32
    };
    let colours: Vec<[f32; 3]> = (0..96).map(|_| [0.12 + 0.5 * rnd(), 0.12 + 0.5 * rnd(), 0.12 + 0.5 * rnd()]).collect();
    (0..w * h)
        .map(|i| {
            let (x, y) = (i % w, i / w);
            let ramp = 0.15 + 0.85 * ((x * 12) % w) as f32 / w as f32;
            colours[(y * 8 / h) * 12 + x * 12 / w].map(|v| v * ramp)
        })
        .collect()
}

/// A "camera JPEG" of `cam`: more saturated than sRGB primaries, with a brightening curve.
fn camera_rendering(cam: &[[f32; 3]], w: usize, h: usize) -> Vec<u8> {
    let k = [[1.3f32, -0.2, -0.1], [-0.15, 1.25, -0.1], [0.0, -0.25, 1.25]];
    let px: Vec<u8> = cam
        .iter()
        .flat_map(|c| {
            (0..3).map(move |i| {
                let v = (k[i][0] * c[0] + k[i][1] * c[1] + k[i][2] * c[2]).clamp(0.0, 1.0);
                let v = 1.6 * v / (1.0 + 0.6 * v);
                let e = if v <= 0.0031308 { 12.92 * v } else { 1.055 * v.powf(1.0 / 2.4) - 0.055 };
                (e * 255.0).round() as u8
            })
        })
        .collect();
    jpeg(Image::from_u8(w as u32, h as u32, ChannelLayout::Rgb, px).unwrap())
}

fn jpeg(img: Image) -> Vec<u8> {
    photocraft_codecs::encode(&img, Format::Jpeg, &EncodeOptions { jpeg_quality: 95, ..Default::default() }).unwrap()
}

/// An uncompressed NEF-like raw of `cam` with an optional JPEG preview in IFD0.
fn nef_uncompressed(cam: &[[f32; 3]], w: usize, h: usize, preview: Option<Vec<u8>>) -> Vec<u8> {
    nef_uncompressed_with_wb(cam, w, h, preview, None)
}

fn nef_uncompressed_with_wb(cam: &[[f32; 3]], w: usize, h: usize, preview: Option<Vec<u8>>, wb: Option<[u32; 2]>) -> Vec<u8> {
    let data = mosaic(cam, w, [0, 1, 1, 2], 0, 4095);
    let mut t = TiffBuilder::default();
    let strip = t.blob(data.iter().flat_map(|v| v.to_le_bytes()).collect());
    let raw = t.ifd(vec![
        (254, Val::Long(vec![0])),
        (256, Val::Long(vec![w as u32])),
        (257, Val::Long(vec![h as u32])),
        (258, Val::Short(vec![12])),
        (259, Val::Short(vec![1])),
        (262, Val::Short(vec![32803])),
        (273, Val::Blobs(vec![strip])),
        (277, Val::Short(vec![1])),
        (278, Val::Long(vec![h as u32])),
        (279, Val::Long(vec![(data.len() * 2) as u32])),
        (33421, Val::Short(vec![2, 2])),
        (33422, Val::Byte(vec![0, 1, 1, 2])),
    ]);
    let mut ifd0 = vec![(271, Val::Ascii("NIKON CORPORATION".into())), (330, Val::Ifds(vec![raw]))];
    if let Some([r, b]) = wb {
        let mut note = TiffBuilder::default();
        let note_ifd = note.ifd(vec![(0x000C, Val::Rational(vec![(r, 1024), (b, 1024), (1, 1), (1, 1)]))]);
        note.chain = vec![note_ifd];
        let mut maker = b"Nikon\0\x02\x10\0\0".to_vec();
        maker.extend(note.build());
        let exif = t.ifd(vec![(37500, Val::Undefined(maker))]);
        ifd0.push((34665, Val::Ifds(vec![exif])));
    }
    if let Some(j) = preview {
        let len = j.len() as u32;
        let b = t.blob(j);
        ifd0.extend([(513, Val::Blobs(vec![b])), (514, Val::Long(vec![len]))]);
    }
    let ifd0 = t.ifd(ifd0);
    t.chain = vec![ifd0];
    t.build()
}

fn fitted(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains("fitted to the camera's embedded JPEG"))
}

fn fallback(warnings: &[String]) -> bool {
    warnings.iter().any(|w| w.contains("sRGB primaries"))
}

#[test]
fn uncalibrated_raw_takes_its_colour_from_the_camera_jpeg() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let r = import("DSC_0002.NEF", &nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)))).unwrap();
    assert!(fitted(&r.warnings) && !fallback(&r.warnings), "{:?}", r.warnings);
    assert_eq!((r.document.size.width, r.document.size.height), (480, 320));
    assert_eq!(r.document.depth, SampleType::U16);
}

#[test]
fn camera_raw_tuning_keeps_the_default_look_and_applies_user_gains_after_fitting() {
    use photocraft_io::raw::{RawTuning, fit_preview_look, import_raw_tuned};
    use photocraft_raw::{DevelopOptions, WhiteBalance, decode, develop_sensor};

    let (w, h) = (480, 320);
    let cam: Vec<_> = patches(w, h).into_iter().map(|c| c.map(|v| v * 0.6)).collect();
    let balanced: Vec<_> = cam.iter().map(|c| [c[0] * 2.0, c[1], c[2] * 1.5]).collect();
    let bytes = nef_uncompressed_with_wb(&cam, w, h, Some(camera_rendering(&balanced, w, h)), Some([2048, 1536]));
    let normal = import("camera.nef", &bytes).unwrap();
    assert!(fitted(&normal.warnings));
    let sensor = decode(&bytes, &Default::default()).unwrap();
    let look = fit_preview_look(&bytes, &sensor, &DevelopOptions::default()).unwrap();
    for tuning in [
        RawTuning::default(),
        RawTuning { exposure: 0.5, ..Default::default() },
        RawTuning { exposure: -0.5, temperature: 20.0, tint: -15.0, ..Default::default() },
    ] {
        let (got, applied) = import_raw_tuned("camera.nef", &bytes, &tuning).unwrap();
        assert!(applied && fitted(&got.warnings) && !fallback(&got.warnings), "{:?}", got.warnings);
        assert_eq!(got.document.depth, SampleType::U16);
        assert_eq!((got.document.size.width, got.document.size.height), (w as u32, h as u32));
        let gains = photocraft_algo::camera_raw::white_balance_gains(tuning.temperature, tuning.tint).map(f64::from);
        let expected = develop_sensor(
            &sensor,
            &DevelopOptions {
                exposure: f64::from(tuning.exposure),
                white_balance: WhiteBalance::Multipliers([2.0 * gains[0], gains[1], 1.5 * gains[2]]),
                camera_look: Some(look.clone()),
                ..Default::default()
            },
        )
        .unwrap();
        let expected_bytes: Vec<_> = expected.rgb.iter().flat_map(|v| v.to_ne_bytes()).collect();
        let actual = photocraft_io::document_to_image(&got.document, &mut Vec::new()).unwrap();
        assert_eq!(actual.data(), expected_bytes, "tuning {tuning:?} must not refit away the user controls");
        if tuning == RawTuning::default() {
            let original = photocraft_io::document_to_image(&normal.document, &mut Vec::new()).unwrap();
            assert_eq!(actual.data(), original.data());
        }
    }
}

#[test]
fn unusable_previews_keep_the_neutral_fallback() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let mut seed = 5u32;
    let noise: Vec<u8> = (0..w * h * 3)
        .map(|_| {
            seed = seed.wrapping_mul(1_103_515_245).wrapping_add(12345);
            (seed >> 16) as u8
        })
        .collect();
    let mut garbage = jpeg(Image::from_u8(64, 48, ChannelLayout::Rgb, vec![90; 64 * 48 * 3]).unwrap());
    let sos = garbage.windows(2).rposition(|p| p == [0xFF, 0xDA]).unwrap();
    garbage.truncate(sos + 8);
    garbage.extend(std::iter::repeat_n(0xAB, 200));
    let previews = [
        None,
        Some(jpeg(Image::from_u8(w as u32, h as u32, ChannelLayout::Rgb, noise).unwrap())),
        Some(jpeg(Image::from_u8(240, 240, ChannelLayout::Rgb, vec![128; 240 * 240 * 3]).unwrap())),
        Some(jpeg(Image::from_u8(w as u32, h as u32, ChannelLayout::Gray, vec![128; w * h]).unwrap())),
        Some(garbage),
    ];
    for (i, p) in previews.into_iter().enumerate() {
        let bytes = nef_uncompressed(&cam, w, h, p);
        let r = import("DSC_0003.NEF", &bytes).unwrap();
        assert!(fallback(&r.warnings) && !fitted(&r.warnings), "preview {i}: {:?}", r.warnings);
        let (tuned, wb) =
            photocraft_io::raw::import_raw_tuned("DSC_0003.NEF", &bytes, &photocraft_io::raw::RawTuning { exposure: 0.5, ..Default::default() }).unwrap();
        assert!(!wb && fallback(&tuned.warnings) && !fitted(&tuned.warnings), "preview {i}: {:?}", tuned.warnings);
        assert_eq!(tuned.document.depth, SampleType::U16);
    }
}

#[test]
fn truncated_raws_with_a_fittable_preview_do_not_panic() {
    let (w, h) = (96, 64);
    let cam = patches(w, h);
    let b = nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)));
    for n in (0..b.len()).step_by(211) {
        let _ = import("x.nef", &b[..n]);
        let _ = photocraft_io::raw::import_raw_tuned("x.nef", &b[..n], &Default::default());
    }
}

/// The open-time Camera Raw re-develop keeps the colour rendering the default open used: a raw
/// without calibration gets the same look fitted to its JPEG, so neutral settings give the same
/// pixels, and the previews' tone curve is the look's.
#[test]
fn a_re_develop_keeps_the_fitted_look() {
    let (w, h) = (480, 320);
    let cam = patches(w, h);
    let nef = nef_uncompressed(&cam, w, h, Some(camera_rendering(&cam, w, h)));
    let opened = import("DSC_0003.NEF", &nef).unwrap();
    let (again, _) = photocraft_io::raw::import_raw_tuned("DSC_0003.NEF", &nef, &Default::default()).unwrap();
    assert!(fitted(&again.warnings), "{:?}", again.warnings);
    let pixels = |d: &photocraft_doc::Document| {
        let s = d.layers[0].surface().unwrap();
        let mut px = vec![[0.0f32; 4]; w * h];
        s.read_rgba_into(photocraft_geom::Rect::new(0, 0, w as i32, h as i32), &mut px);
        px
    };
    assert_eq!(pixels(&opened.document), pixels(&again.document));
    let curve = photocraft_io::raw::tone_curve(&nef);
    assert!(curve.len() > 2 && curve.windows(2).all(|k| k[1][0] > k[0][0] && k[1][1] >= k[0][1]), "{curve:?}");
    // The camera's brightening curve: mid-grey comes out lighter.
    let mid = curve.iter().find(|k| k[0] >= 0.18).unwrap();
    assert!(mid[1] > mid[0], "{mid:?}");
}
