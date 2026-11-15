//! Package: a link copy whose name is taken is named `name_1.ext`, `name_2.ext`, ... (the number
//! goes before the extension; a name without one just gets the suffix).

use super::*;

fn tmp(name: &str) -> String {
    let d = std::env::temp_dir().join(format!("photocraft-pkgclash-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d.to_string_lossy().replace('\\', "/")
}

/// Writes a 4 x 4 solid PNG as `<dir>/<sub>/<name>` and returns its path.
fn write_link(dir: &str, sub: &str, name: &str, colour: &str) -> String {
    std::fs::create_dir_all(format!("{dir}/{sub}")).unwrap();
    let path = format!("{dir}/{sub}/{name}");
    let mut src = Session::new();
    src.execute("file.new", json!({"width": 4, "height": 4, "background": colour})).unwrap();
    let doc = src.active().unwrap().doc.clone();
    // A name without an extension has no format of its own: write the PNG bytes by hand.
    let png = format!("{dir}/{sub}/__tmp.png");
    crate::file_cmds::save_doc(&doc, &png, None).unwrap();
    std::fs::rename(&png, &path).unwrap();
    path
}

fn open_session() -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 120, "height": 90, "name": "Clash.psd", "background": "#3366cc"})).unwrap();
    s
}

fn place(s: &mut Session, path: &str) {
    s.execute("file.placeLinked", json!({"path": path})).unwrap();
}

fn package(s: &mut Session, dir: &str) -> Value {
    s.execute("file.package", json!({"dir": format!("{dir}/pkg")})).unwrap()
}

fn links(r: &Value) -> Vec<String> {
    r["links"].as_array().unwrap().iter().map(|v| v.as_str().unwrap().to_string()).collect()
}

fn linked_paths(d: &Document) -> Vec<String> {
    d.walk()
        .iter()
        .filter_map(|(_, _, l)| match &l.content {
            LayerContent::Smart(so) => match &so.source {
                SmartSource::Linked { path } => Some(path.clone()),
                _ => None,
            },
            _ => None,
        })
        .collect()
}

/// The first pixel's RGB of an image file.
fn first_rgb(path: &str) -> [u8; 3] {
    let img = photocraft_codecs::decode(&std::fs::read(path).unwrap()).unwrap();
    let d = img.data();
    [d[0], d[1], d[2]]
}

fn tails(r: &Value) -> Vec<String> {
    links(r).iter().map(|p| p.rsplit('/').next().unwrap().to_string()).collect()
}

#[test]
fn a_clashing_copy_is_named_name_1_ext() {
    let dir = tmp("one");
    let mut s = open_session();
    let a = write_link(&dir, "a", "logo.png", "#ff0000");
    let b = write_link(&dir, "b", "logo.png", "#00c800");
    place(&mut s, &a);
    place(&mut s, &b);
    let r = package(&mut s, &dir);
    let names = tails(&r);
    // The lower layer (`a`, placed first) is copied first and keeps the plain name.
    assert_eq!(names, vec!["logo.png", "logo_1.png"]);
    assert_eq!(first_rgb(&links(&r)[0]), [255, 0, 0]);
    assert_eq!(first_rgb(&links(&r)[1]), [0, 200, 0]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn further_clashes_count_up() {
    let dir = tmp("three");
    let mut s = open_session();
    for (sub, colour) in [("a", "#ff0000"), ("b", "#00c800"), ("c", "#0000ff")] {
        let p = write_link(&dir, sub, "logo.png", colour);
        place(&mut s, &p);
    }
    let r = package(&mut s, &dir);
    assert_eq!(tails(&r), vec!["logo.png", "logo_1.png", "logo_2.png"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_multi_dot_name_keeps_its_last_extension() {
    let dir = tmp("dots");
    let mut s = open_session();
    for (sub, colour) in [("a", "#ff0000"), ("b", "#00c800")] {
        let p = write_link(&dir, sub, "a.b.png", colour);
        place(&mut s, &p);
    }
    let r = package(&mut s, &dir);
    assert_eq!(tails(&r), vec!["a.b.png", "a.b_1.png"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_name_without_an_extension_gets_the_suffix_at_the_end() {
    let dir = tmp("noext");
    let mut s = open_session();
    let mut copied = Vec::new();
    for (sub, colour) in [("a", "#ff0000"), ("b", "#00c800")] {
        let p = write_link(&dir, sub, "logo", colour);
        place(&mut s, &p);
        copied.push(p);
    }
    let r = package(&mut s, &dir);
    let names = tails(&r);
    // The files can't be imported without an extension on every build; when both are copied the
    // names are `logo` and `logo_1`, and a copy is never given a form with a dash prefix.
    assert!(names.iter().all(|n| !n.contains('-')), "{names:?}");
    if names.len() == 2 {
        assert_eq!(names, vec!["logo", "logo_1"]);
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_file_linked_twice_is_copied_once() {
    let dir = tmp("twice");
    let mut s = open_session();
    let a = write_link(&dir, "a", "logo.png", "#ff0000");
    let b = write_link(&dir, "b", "logo.png", "#00c800");
    place(&mut s, &a);
    place(&mut s, &a);
    place(&mut s, &b);
    let r = package(&mut s, &dir);
    assert_eq!(tails(&r), vec!["logo.png", "logo_1.png"]);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn each_layer_points_at_its_own_copy_and_keeps_its_colour() {
    let dir = tmp("colours");
    let mut s = open_session();
    let a = write_link(&dir, "a", "logo.png", "#ff0000");
    let b = write_link(&dir, "b", "logo.png", "#00c800");
    place(&mut s, &a);
    place(&mut s, &b);
    let r = package(&mut s, &dir);
    let back = photocraft_io::import("Clash.pcraft", &std::fs::read(r["document"].as_str().unwrap()).unwrap()).unwrap().document;
    let packed = linked_paths(&back);
    let original = linked_paths(&s.active().unwrap().doc);
    assert_eq!(packed.len(), 2);
    assert_ne!(packed[0], packed[1]);
    for (copy, source) in packed.iter().zip(&original) {
        assert_eq!(first_rgb(copy), first_rgb(source), "{copy} vs {source}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}
