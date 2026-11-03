use photocraft_codecs::{ChannelLayout, EncodeOptions, ExrCompression, Format, Image, SampleType};
use photocraft_doc::LayerContent;
use photocraft_raster::Interrupt;
use std::sync::atomic::{AtomicBool, Ordering};

fn fixture(layout: ChannelLayout, sample: SampleType, compression: ExrCompression) -> Vec<u8> {
    let values: Vec<f32> = (0..31 * 35 * layout.channels()).map(|i| [-0.25, 4.0, 0.0, 0.0001, 0.5][i % 5]).collect();
    let image = Image::from_f32(31, 35, layout, &values).unwrap().convert(layout, sample);
    photocraft_codecs::encode(&image, Format::OpenExr, &EncodeOptions { exr_compression: compression, ..Default::default() }).unwrap()
}

fn open(path: &std::path::Path, ctl: &Interrupt) -> Result<photocraft_io::ImportResult, photocraft_io::IoError> {
    photocraft_io::import_file("HDR", path, &photocraft_io::ImportOptions::default(), ctl)
}

// One test keeps these imports from competing for the global working allowance.
#[test]
fn native_scanline_import_preserves_hdr_document_and_cancels_without_publication() {
    let path = std::env::temp_dir().join(format!("photocraft-exr-stream-{}.exr", std::process::id()));
    for layout in [ChannelLayout::Gray, ChannelLayout::GrayA, ChannelLayout::Rgb, ChannelLayout::Rgba] {
        for sample in [SampleType::F16, SampleType::F32] {
            for compression in [ExrCompression::None, ExrCompression::Rle, ExrCompression::Zip1, ExrCompression::Zip16, ExrCompression::Piz] {
                let bytes = fixture(layout, sample, compression);
                std::fs::write(&path, &bytes).unwrap();
                let reference = photocraft_io::import("HDR", &bytes).unwrap();
                let actual = open(&path, &Interrupt::NONE).unwrap();
                {
                    let a = &actual.document;
                    let r = &reference.document;
                    assert_eq!(actual.warnings, reference.warnings);
                    assert_eq!(a.name, r.name);
                    assert_eq!(a.size, r.size);
                    assert_eq!(a.mode, r.mode);
                    assert_eq!(a.depth, photocraft_color::SampleType::F32);
                    assert_eq!(a.icc_profile, r.icc_profile);
                    assert_eq!(a.layers[0].locks, r.layers[0].locks);
                    let (LayerContent::Raster(a), LayerContent::Raster(r)) = (&a.layers[0].content, &r.layers[0].content) else {
                        panic!("raster background required")
                    };
                    assert_eq!(
                        a.to_interleaved(reference.document.bounds()),
                        r.to_interleaved(reference.document.bounds()),
                        "{layout:?}/{sample:?}/{compression:?}"
                    );
                }
            }
        }
    }
    let bytes = fixture(ChannelLayout::Rgb, SampleType::F16, ExrCompression::Zip16);
    std::fs::write(&path, &bytes).unwrap();
    let cancelled = AtomicBool::new(false);
    let cancel = || cancelled.load(Ordering::Relaxed);
    let progress = |p| {
        if p > 0.2 {
            cancelled.store(true, Ordering::Relaxed);
        }
    };
    assert!(matches!(open(&path, &Interrupt::new(&cancel, &progress)), Err(photocraft_io::IoError::Cancelled)));
    std::fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    assert!(open(&path, &Interrupt::NONE).is_err());
    // A syntactically complete huge header/table must be refused before reading
    // its pixel blocks when scratch is unavailable. No giant fixture is needed.
    assert!(!photocraft_raster::spill::enabled());
    let mut header_end = 8;
    loop {
        if bytes[header_end] == 0 {
            header_end += 1;
            break;
        }
        for _ in 0..2 {
            while bytes[header_end] != 0 {
                header_end += 1;
            }
            header_end += 1;
        }
        let length = i32::from_le_bytes(bytes[header_end..header_end + 4].try_into().unwrap()) as usize;
        header_end += 4 + length;
    }
    let mut huge = bytes[..header_end].to_vec();
    let needle = b"dataWindow\0box2i\0\x10\0\0\0";
    let at = huge.windows(needle.len()).position(|v| v == needle).unwrap() + needle.len();
    huge[at + 8..at + 12].copy_from_slice(&65535_i32.to_le_bytes());
    huge[at + 12..at + 16].copy_from_slice(&32767_i32.to_le_bytes());
    let offset = (header_end + 2048 * 8) as u64;
    for _ in 0..2048 {
        huge.extend_from_slice(&offset.to_le_bytes());
    }
    huge.extend_from_slice(&[0; 8]);
    std::fs::write(&path, &huge).unwrap();
    assert!(matches!(open(&path, &Interrupt::NONE), Err(photocraft_io::IoError::OutOfMemory(ref message)) if message.contains("decoded image tiles")));
    std::fs::remove_file(path).unwrap();
}
