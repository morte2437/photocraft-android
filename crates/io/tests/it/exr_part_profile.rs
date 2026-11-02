//! Selecting an EXR part must interpret its linear samples like ordinary File > Open.

use photocraft_cms::{Builtin, ColorSpace, Intent, Profile, Transform};
use photocraft_codecs::{self as codecs, ChannelLayout, Format, Image, SampleType as CodecSample};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_io::{ExportOptions, ImportResult, export, exr_parts, import};

fn exr(layout: ChannelLayout, depth: CodecSample) -> Vec<u8> {
    // Exactly representable in half precision, including negative and HDR luminance.
    let samples: Vec<f32> = [-0.25, 0.25, 0.5, 2.0]
        .into_iter()
        .zip([1.0, 0.5, 0.25, 0.0])
        .flat_map(|(v, a)| match layout {
            ChannelLayout::Gray => vec![v],
            ChannelLayout::GrayA => vec![v, a],
            ChannelLayout::Rgb => vec![v, 0.125, 0.75],
            ChannelLayout::Rgba => vec![v, 0.125, 0.75, a],
            _ => panic!("not an EXR layout"),
        })
        .collect();
    let image = Image::from_f32(4, 1, layout, &samples).unwrap().convert(layout, depth);
    codecs::encode(&image, Format::OpenExr, &Default::default()).unwrap()
}

fn selected_part(bytes: &[u8]) -> ImportResult {
    let entries = exr_parts::entries(bytes).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].kind, exr_parts::EntryKind::Part);
    let selection = exr_parts::Selection { parts: vec![entries[0].name.clone()], groups: vec![], view: "both".into(), precomp: false };
    exr_parts::import("selected", bytes, &selection).unwrap()
}

fn samples(doc: &Document) -> Vec<f32> {
    assert_eq!(doc.layers.len(), 1);
    doc.layers[0].surface().unwrap().read_region(doc.bounds())
}

#[test]
fn selected_gray_exr_part_keeps_linear_profile_and_float_samples() {
    for depth in [CodecSample::F16, CodecSample::F32] {
        for layout in [ChannelLayout::Gray, ChannelLayout::GrayA] {
            let bytes = exr(layout, depth);
            let selected = selected_part(&bytes);
            let ordinary = import("ordinary.exr", &bytes).unwrap();
            let doc = &selected.document;
            assert_eq!((doc.mode, doc.depth), (ColorMode::Grayscale, SampleType::F32));
            assert_eq!(doc.size, ordinary.document.size);
            assert_eq!(selected.warnings, ordinary.warnings);
            assert_eq!(selected.warnings.len(), usize::from(depth == CodecSample::F16));
            let expected = codecs::decode(&bytes).unwrap().convert(ChannelLayout::GrayA, CodecSample::F32).to_f32_samples().unwrap();
            assert_eq!(samples(doc), expected, "{layout:?} {depth:?}: samples and alpha stay unchanged");
            assert_eq!(samples(doc), samples(&ordinary.document));
            assert_eq!(doc.icc_profile, Some(photocraft_cms::builtin::linear_gray().to_bytes()), "{layout:?} {depth:?}: linear luminance");
            assert_eq!(doc.icc_profile, ordinary.document.icc_profile);

            // The tagged samples convert to display gray, rather than being interpreted as sGray.
            let profile = Profile::parse(doc.icc_profile.as_deref().unwrap()).unwrap();
            assert_eq!(profile.color_space, ColorSpace::Gray);
            let transform = Transform::new(&profile, Builtin::SGray.profile(), Intent::RelativeColorimetric, false).unwrap();
            let mut display = samples(doc);
            transform.apply(&mut display, 2);
            for i in [1, 2] {
                let expected_gray = photocraft_color::convert::linear_to_srgb(expected[i * 2]);
                assert!((display[i * 2] - expected_gray).abs() < 1e-4, "{layout:?} {depth:?}: display gray");
                assert_eq!(display[i * 2 + 1], expected[i * 2 + 1], "alpha is not gamma encoded");
            }
        }
    }
}

#[test]
fn selected_gray_exr_part_exports_like_ordinary_import() {
    for depth in [CodecSample::F16, CodecSample::F32] {
        for layout in [ChannelLayout::Gray, ChannelLayout::GrayA] {
            let bytes = exr(layout, depth);
            let selected = selected_part(&bytes).document;
            let ordinary = import("ordinary.exr", &bytes).unwrap().document;
            for ext in ["exr", "tga"] {
                let output = export(&selected, ext, &ExportOptions::default()).unwrap();
                let control = export(&ordinary, ext, &ExportOptions::default()).unwrap();
                let output = codecs::decode(&output.bytes).unwrap();
                let control = codecs::decode(&control.bytes).unwrap();
                assert_eq!(output.layout(), control.layout());
                assert_eq!(output.to_normalized(), control.to_normalized(), "{layout:?} {depth:?} {ext}: same colours as File > Open");
                let output = output.convert(ChannelLayout::GrayA, CodecSample::F32).to_f32_samples().unwrap();
                let original = samples(&selected);
                if ext == "exr" {
                    assert_eq!(output, original, "linear EXR samples, including negative/HDR and alpha, survive re-export");
                } else {
                    for i in [1, 2] {
                        let expected = photocraft_color::convert::linear_to_srgb(original[i * 2]);
                        assert!((output[i * 2] - expected).abs() <= 1.0 / 255.0, "TGA uses display-encoded gray");
                        assert!((output[i * 2 + 1] - original[i * 2 + 1]).abs() <= 1.0 / 255.0, "TGA preserves alpha");
                    }
                }
            }
        }
    }
}

#[test]
fn selected_rgb_exr_part_keeps_linear_srgb() {
    for depth in [CodecSample::F16, CodecSample::F32] {
        for layout in [ChannelLayout::Rgb, ChannelLayout::Rgba] {
            let bytes = exr(layout, depth);
            let doc = selected_part(&bytes).document;
            let ordinary = import("ordinary.exr", &bytes).unwrap().document;
            assert_eq!((doc.mode, doc.depth), (ColorMode::Rgb, SampleType::F32));
            assert_eq!(doc.icc_profile, Some(Builtin::LinearSrgb.profile().to_bytes()));
            assert_eq!(doc.icc_profile, ordinary.icc_profile);
            let expected = codecs::decode(&bytes).unwrap().convert(ChannelLayout::Rgba, CodecSample::F32).to_f32_samples().unwrap();
            assert_eq!(samples(&doc), expected, "{layout:?} {depth:?}: RGB and alpha stay unchanged");
            assert_eq!(samples(&doc), samples(&ordinary));
        }
    }
}
