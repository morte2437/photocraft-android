//! Scripted `.pcraft` saves write the full-size composite when they ask for it
//! (`pcraftFullComposite`), and leave it out by default, as the app's Save does.

use photocraft_automation::{Headless, files};
use photocraft_format::zip::ZipReader;
use serde_json::json;

fn full_composite(bytes: &[u8]) -> ((u32, u32), Vec<u8>) {
    let png = photocraft_format::read_full_composite(bytes).unwrap().expect("the manifest names the full-size composite");
    let image = photocraft_codecs::decode(&png).unwrap();
    (image.dimensions(), image.to_rgba8())
}

fn has_full_composite(bytes: &[u8]) -> bool {
    photocraft_format::read_full_composite(bytes).unwrap().is_some() || ZipReader::new(bytes).unwrap().find("composite/full.png").is_some()
}

#[test]
fn scripted_native_saves_write_the_full_size_composite_when_asked() {
    let dir = std::env::temp_dir().join(format!(
        "pc-native-full-composite-{}-{}",
        std::process::id(),
        std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_nanos()
    ));
    std::fs::create_dir_all(&dir).unwrap();
    let mut headless = Headless::trusted_local();
    headless.handle("doc.new", json!({"width": 40, "height": 30, "name": "Full"})).unwrap();
    headless.handle("engine.execute", json!({"command":"image.adjustments.invert", "params":{}})).unwrap();
    let doc = headless.session.active().unwrap().doc.clone();
    let flattened = photocraft_compose::render(&doc, doc.bounds()).to_rgba8().pixels;

    let (bytes, _) = files::save_bytes(&doc, "Full.pcraft", None, &Default::default()).unwrap();
    assert!(!has_full_composite(&bytes), "off by default");
    let on = photocraft_io::ExportOptions { pcraft_full_composite: true, ..Default::default() };
    let (bytes, _) = files::save_bytes(&doc, "Full.pcraft", None, &on).unwrap();
    assert_eq!(full_composite(&bytes), ((40, 30), flattened.clone()));

    let path = dir.join("Full.pcraft");
    headless.handle("doc.save", json!({"path": path.to_str().unwrap()})).unwrap();
    assert!(!has_full_composite(&std::fs::read(&path).unwrap()), "off by default");
    headless.handle("doc.save", json!({"path": path.to_str().unwrap(), "pcraftFullComposite": true})).unwrap();
    assert_eq!(full_composite(&std::fs::read(&path).unwrap()), ((40, 30), flattened));
    for bad in [json!("true"), json!(1)] {
        let err = headless.handle("doc.save", json!({"path": dir.join("Bad.pcraft").to_str().unwrap(), "pcraftFullComposite": bad})).unwrap_err();
        assert!(format!("{err:?}").contains("`pcraftFullComposite` must be true or false"), "{bad}: {err:?}");
        assert!(!dir.join("Bad.pcraft").exists(), "{bad}: nothing is written");
    }
    std::fs::remove_dir_all(dir).unwrap();
}
