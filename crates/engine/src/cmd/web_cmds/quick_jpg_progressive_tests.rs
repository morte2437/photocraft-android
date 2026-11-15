//! Quick Export writes progressive JPGs (SOF2) at every quality, as the reference application
//! does, while Save for Web keeps its own Progressive checkbox, off by default (#13787).

use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-quick-jpg-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// A 64×48 8-bit document: a white background with a flat blue block over its left half.
fn session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48, "depth": 8, "name": "quick.psd", "background": "white"})).unwrap();
    s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 48})).unwrap();
    s.execute("edit.fill", json!({"color": "#3366cc"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// The marker of the file's frame header (0xC0 baseline, 0xC2 progressive), found by walking the
/// marker segments in front of the first scan.
fn sof_marker(bytes: &[u8]) -> Option<u8> {
    let mut i = 2;
    while i + 4 <= bytes.len() {
        if bytes[i] != 0xFF {
            return None;
        }
        let m = bytes[i + 1];
        if (0xC0..=0xCF).contains(&m) && !matches!(m, 0xC4 | 0xC8 | 0xCC) {
            return Some(m);
        }
        if m == 0xDA {
            return None;
        }
        let len = usize::from(u16::from_be_bytes([bytes[i + 2], bytes[i + 3]]));
        i += 2 + len;
    }
    None
}

fn rgba8(bytes: &[u8]) -> Vec<u8> {
    let img = photocraft_codecs::decode(bytes).unwrap();
    assert_eq!(img.dimensions(), (64, 48));
    img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8).data().to_vec()
}

fn max_channel_diff(a: &[u8], b: &[u8]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).map(|(x, y)| x.abs_diff(*y)).max().unwrap_or(0)
}

#[test]
fn quick_export_jpg_is_progressive_at_every_quality() {
    let dir = tmp("every_quality");
    let mut s = session();
    s.execute("file.export.exportPreferences", json!({"quickExportFormat": "jpg"})).unwrap();
    let convert = s.prefs().export.convert_to_srgb;
    for q in [1, 14, 29, 43, 57, 71, 85, 100] {
        s.execute("file.export.exportPreferences", json!({"jpegQuality": q})).unwrap();
        let quick = format!("{dir}/quick-{q}.jpg");
        s.execute("file.export.quickExport", json!({"path": quick})).unwrap();
        let quick = std::fs::read(&quick).unwrap();
        assert_eq!(sof_marker(&quick), Some(0xC2), "Quick Export at quality {q} is progressive");

        // Only the scan layout changed: Save for Web's progressive output at the same settings
        // decodes to the same pixels.
        let settings = json!({"format": "jpeg", "quality": q, "metadata": "copyright", "convertToSrgb": convert, "matte": "#ffffff"});
        let web = format!("{dir}/web-{q}.jpg");
        let mut p = settings.clone();
        p["progressive"] = json!(true);
        p["path"] = json!(web);
        s.execute("file.export.saveForWebLegacy", p).unwrap();
        let diff = max_channel_diff(&rgba8(&quick), &rgba8(&std::fs::read(&web).unwrap()));
        assert!(diff <= 2, "quality {q}: Quick Export and Save for Web pixels differ by {diff}");

        // Save for Web keeps its own checkbox, off by default: baseline.
        let baseline = format!("{dir}/baseline-{q}.jpg");
        let mut p = settings;
        p["path"] = json!(baseline);
        s.execute("file.export.saveForWebLegacy", p).unwrap();
        assert_eq!(sof_marker(&std::fs::read(&baseline).unwrap()), Some(0xC0), "Save for Web at quality {q} stays baseline by default");
    }
}

#[test]
fn quick_export_jpg_opens_again_with_the_same_pixels() {
    let dir = tmp("round_trip");
    let mut s = session();
    s.execute("file.export.exportPreferences", json!({"quickExportFormat": "jpg"})).unwrap();
    // The default quality, then the highest.
    for q in [None, Some(100)] {
        if let Some(q) = q {
            s.execute("file.export.exportPreferences", json!({"jpegQuality": q})).unwrap();
        }
        let path = format!("{dir}/round-{}.jpg", q.map_or_else(|| "default".to_string(), |q| q.to_string()));
        s.execute("file.export.quickExport", json!({"path": path})).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(sof_marker(&bytes), Some(0xC2));
        // The blue half ends on a block edge, so every pixel is flat colour.
        for (i, got) in rgba8(&bytes).as_chunks::<4>().0.iter().enumerate() {
            let want = if i % 64 < 32 { [0x33, 0x66, 0xcc, 255] } else { [255, 255, 255, 255] };
            assert!(max_channel_diff(got, &want) <= 2, "quality {q:?}, pixel ({}, {}): {got:?}, want {want:?}", i % 64, i / 64);
        }
    }
}
