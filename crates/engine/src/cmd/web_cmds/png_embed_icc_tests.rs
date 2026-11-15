//! #13798: a Quick Export PNG carries no ICC profile, with Convert to sRGB on or off, and Save for
//! Web's PNG-24 embeds one only with Embed Color Profile on. The pixels are as before.

use serde_json::json;

use crate::Session;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-png-icc-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

/// A red document with a wide-gamut profile (Adobe RGB) assigned.
fn tagged() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 20, "height": 10, "background": "#c81e1e"})).unwrap();
    s.execute("edit.assignProfile", json!({"profile": "adobe-rgb-compat"})).unwrap();
    assert!(s.active().unwrap().doc.icc_profile.is_some(), "the document has a profile");
    s
}

/// The PNG's chunk types, in order.
fn chunks(bytes: &[u8]) -> Vec<String> {
    assert_eq!(bytes.get(..8), Some(b"\x89PNG\r\n\x1a\n".as_slice()), "a PNG");
    let mut out = Vec::new();
    let mut at = 8;
    while let Some(head) = bytes.get(at..at + 8) {
        let len = u32::from_be_bytes([head[0], head[1], head[2], head[3]]) as usize;
        out.push(String::from_utf8_lossy(&head[4..8]).into_owned());
        at += 12 + len;
    }
    out
}

fn pixels(path: &str) -> Vec<u8> {
    let img = photocraft_codecs::decode(&std::fs::read(path).unwrap()).unwrap();
    img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8).data().to_vec()
}

#[test]
fn a_quick_export_png_embeds_no_profile_with_convert_to_srgb_on_or_off() {
    let dir = tmp("quick");
    let mut s = tagged();
    for convert in [true, false] {
        s.execute("file.export.exportPreferences", json!({"quickExportFormat": "png", "metadata": "none", "convertToSrgb": convert})).unwrap();
        let path = format!("{dir}/convert-{convert}.png");
        s.execute("file.export.quickExport", json!({"path": path})).unwrap();
        let found = chunks(&std::fs::read(&path).unwrap());
        assert!(!found.iter().any(|c| c == "iCCP"), "convert {convert}: {found:?}");
        assert!(photocraft_codecs::decode(&std::fs::read(&path).unwrap()).unwrap().icc.is_none(), "convert {convert}");
    }
    s.execute("prefs.reset", json!({"path": "export"})).unwrap();
}

#[test]
fn save_for_web_png24_embeds_the_profile_only_when_asked_and_keeps_its_pixels() {
    let dir = tmp("sfw");
    let mut s = tagged();
    let (on, off) = (format!("{dir}/on.png"), format!("{dir}/off.png"));
    s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "embedIcc": true, "path": on})).unwrap();
    s.execute("file.export.saveForWebLegacy", json!({"format": "png24", "embedIcc": false, "path": off})).unwrap();
    assert!(chunks(&std::fs::read(&on).unwrap()).iter().any(|c| c == "iCCP"), "Embed Color Profile on");
    assert!(!chunks(&std::fs::read(&off).unwrap()).iter().any(|c| c == "iCCP"), "Embed Color Profile off");
    assert_eq!(pixels(&on), pixels(&off), "only the profile differs");
}
