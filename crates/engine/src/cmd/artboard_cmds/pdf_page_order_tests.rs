//! Artboards to PDF writes its pages bottom artboard of the Layers panel first (#13643), on the
//! measured document: 800 x 400 with Red, Blue and Empty Board, bottom to top.

use super::*;

/// The measured document, and the ids of Red, Blue and Empty Board (bottom to top).
fn session() -> (Session, [u64; 3]) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 800, "height": 400, "depth": 8, "resolution": 72})).unwrap();
    let mut board = |rect: [i64; 4], name: &str| {
        let r = s.execute("layer.new.artboard", json!({"rect": rect, "name": name})).unwrap();
        r["layer"].as_u64().unwrap()
    };
    let ids = [board([0, 0, 200, 150], "Red Board"), board([120, 30, 200, 200], "Blue Board"), board([600, 0, 100, 100], "Empty Board")];
    (s, ids)
}

/// Each page's size in points, in page order, from the exported PDF.
fn page_sizes(s: &mut Session, params: Value) -> Vec<(u32, u32)> {
    let dir = std::env::temp_dir().join(format!("photocraft-pdf-order-{}-{}", std::process::id(), params["artboards"]));
    std::fs::create_dir_all(&dir).unwrap();
    let path = join(&dir.to_string_lossy(), "boards.pdf");
    let mut params = params;
    params["path"] = json!(path);
    s.execute("file.export.artboardsToPdf", params).unwrap();
    let text = String::from_utf8_lossy(&std::fs::read(&path).unwrap()).into_owned();
    let _ = std::fs::remove_dir_all(&dir);
    text.split("/MediaBox [0 0 ")
        .skip(1)
        .map(|rest| {
            let mut n = rest.split([' ', ']']).map(|v| v.parse::<f64>().unwrap().round() as u32);
            (n.next().unwrap(), n.next().unwrap())
        })
        .collect()
}

#[test]
fn the_bottom_artboard_is_the_first_page() {
    let (mut s, _) = session();
    assert_eq!(page_sizes(&mut s, json!({})), [(200, 150), (200, 200), (100, 100)]);
}

#[test]
fn chosen_artboards_keep_the_bottom_first_order_whatever_the_id_order() {
    let (mut s, [red, blue, _]) = session();
    assert_eq!(page_sizes(&mut s, json!({"artboards": [blue, red]})), [(200, 150), (200, 200)]);
}

#[test]
fn artboards_to_files_keeps_its_top_first_order() {
    let (mut s, _) = session();
    let dir = std::env::temp_dir().join(format!("photocraft-pdf-order-files-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let r = s.execute("file.export.artboardsToFiles", json!({"dir": dir.to_string_lossy(), "prefix": "p"})).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    let files: Vec<&str> = r["files"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
    assert!(files[0].ends_with("p_0000_Empty Board.png") && files[2].ends_with("p_0002_Red Board.png"), "{files:?}");
}
