//! A droplet skips a dropped file that is not an image (#13717).

use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-droplet-skip-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().into_owned()
}

fn small_png(path: &str) {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 4, "height": 3, "background": "#808080"})).unwrap();
    crate::file_cmds::save_doc(&s.active().unwrap().doc, path, None).unwrap();
}

/// Makes `dir/Rotate.pcdroplet`, writing its output to `dir/out`.
fn droplet(dir: &str) -> String {
    let r = Session::new()
        .execute(
            "file.automate.createDroplet",
            json!({"path": format!("{dir}/Rotate"), "steps": [["image.imageRotation.90cw", {}]], "format": "png", "output": format!("{dir}/out"), "shim": false}),
        )
        .unwrap();
    r["path"].as_str().unwrap().to_string()
}

#[test]
fn a_dropped_text_file_is_skipped_and_the_image_is_processed() {
    let dir = tmp("text");
    let (txt, png) = (format!("{dir}/notes.txt"), format!("{dir}/a.png"));
    std::fs::write(&txt, "just some notes").unwrap();
    small_png(&png);
    let dp = droplet(&dir);
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [txt, png]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(r["files"].as_array().unwrap().len(), 1, "{r}");
    assert!(std::path::Path::new(&format!("{dir}/out/a.png")).exists());
}

#[test]
fn a_dropped_broken_image_still_reports_its_error() {
    let dir = tmp("broken");
    let (bad, png) = (format!("{dir}/broken.png"), format!("{dir}/a.png"));
    std::fs::write(&bad, "not a png").unwrap();
    small_png(&png);
    let dp = droplet(&dir);
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [bad, png]})).unwrap();
    assert_eq!(r["errors"].as_array().unwrap().len(), 1, "{r}");
    assert!(r["errors"][0].to_string().contains("broken.png"), "{r}");
    assert_eq!(r["files"].as_array().unwrap().len(), 1, "{r}");
}

#[test]
fn a_folder_input_skips_non_images_as_before() {
    let dir = tmp("folder");
    let input = format!("{dir}/in");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::write(format!("{input}/notes.txt"), "just some notes").unwrap();
    small_png(&format!("{input}/a.png"));
    let dp = droplet(&dir);
    let r = Session::new().execute("file.automate.runDroplet", json!({"droplet": dp, "input": [input]})).unwrap();
    assert_eq!(r["errors"], json!([]), "{r}");
    assert_eq!(r["files"].as_array().unwrap().len(), 1, "{r}");
}
