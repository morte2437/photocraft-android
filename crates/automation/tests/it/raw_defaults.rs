//! #7789: the headless (MCP) `doc.open` opens a developed raw at Preferences › Raw Defaults ›
//! Resolution, as the app's File › Open does.

use photocraft_automation::Headless;
use photocraft_raw::testgen::{DngSpec, mosaic, scene};
use serde_json::json;

#[test]
fn doc_open_gives_a_raw_the_raw_defaults_resolution() {
    let dir = std::env::temp_dir().join(format!("pc-raw-defaults-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let (w, h) = (24, 16);
    let path = dir.join("IMG_0001.dng");
    std::fs::write(&path, DngSpec::cfa(w, h, mosaic(&scene(w, h), w, [0, 1, 1, 2], 0, 65535)).build()).unwrap();
    let mut headless = Headless::trusted_local();
    headless.handle("engine.execute", json!({"command": "prefs.set", "params": {"path": "rawDefaults.resolution", "value": 240}})).unwrap();
    headless.handle("doc.open", json!({"path": path.to_str().unwrap()})).unwrap();
    assert_eq!(headless.session.active().unwrap().doc.resolution_dpi, 240.0);
    std::fs::remove_dir_all(dir).unwrap();
}
