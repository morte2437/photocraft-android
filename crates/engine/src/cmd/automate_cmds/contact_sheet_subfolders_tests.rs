//! #13687: Contact Sheet II's Include Subfolders (`includeSubfolders`, default on) and Group
//! Images by Folder (`groupByFolder`, default off), on the issue's measured test folder.
use super::*;

/// Removes a test folder when it goes out of scope, also when the test or its setup panics.
struct Cleanup(String);

impl Drop for Cleanup {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The issue's measured folder: four images at the top, one in `sub/` and one in `sub/deeper/`,
/// plus what holds no image (a text file, an empty folder). With `extras` it also has a folder
/// that sorts before the top folder's files (`0_first/`) and one after `sub/` (`sub2/`). The
/// returned guard holds the folder's path and removes it when dropped.
fn tree(name: &str, extras: bool) -> Cleanup {
    let root = std::env::temp_dir().join(format!("photocraft-contact-subfolders-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let guard = Cleanup(root.to_string_lossy().into_owned());
    let root = &guard.0;
    let mut files = vec!["a_red.png", "b_green.jpg", "c_blue.psd", "d gray.tif", "sub/e_yellow.png", "sub/deeper/f_cyan.png"];
    if extras {
        files.extend(["0_first/g_white.png", "sub2/h_black.png"]);
    }
    for (i, rel) in files.into_iter().enumerate() {
        let path = join(root, rel);
        std::fs::create_dir_all(std::path::Path::new(&path).parent().unwrap()).unwrap();
        let mut s = Session::new();
        let grey = 20 + 25 * i;
        s.execute("file.new", json!({"width": 12, "height": 8, "background": format!("#{grey:02x}{grey:02x}{grey:02x}")})).unwrap();
        crate::file_cmds::save_doc(&s.active().unwrap().doc, &path, None).unwrap();
    }
    std::fs::write(join(root, "notes.txt"), "not an image").unwrap();
    std::fs::create_dir_all(join(root, "sub/empty")).unwrap();
    guard
}

/// Runs Contact Sheet II on `input` with `extra` params (2 × 2 cells, no captions) and returns
/// each sheet's document name and the names of its image layers, in placing order.
fn sheets(input: Value, extra: Value) -> Result<Vec<(String, Vec<String>)>> {
    let mut p = json!({"input": input, "units": "pixels", "width": 200, "height": 200, "resolution": 72, "columns": 2, "rows": 2, "caption": false});
    p.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
    let mut s = Session::new();
    let r = s.execute("file.automate.contactSheetII", p)?;
    assert_eq!(r["errors"], json!([]), "{r}");
    let docs = r["documents"].as_array().unwrap();
    assert_eq!(r["pages"], docs.len());
    Ok(docs
        .iter()
        .map(|i| {
            let d = &s.documents()[i.as_u64().unwrap() as usize].doc;
            // The first layer is the sheet's white background.
            (d.name.clone(), d.layers.iter().skip(1).map(|l| l.name.clone()).collect())
        })
        .collect())
}

/// Every image layer of the run, in placing order across its sheets.
fn placed(sheets: &[(String, Vec<String>)]) -> Vec<&str> {
    sheets.iter().flat_map(|(_, layers)| layers.iter().map(String::as_str)).collect()
}

/// `(name, image layers)` pairs for the expected sheets.
fn expect(pages: &[(&str, &[&str])]) -> Vec<(String, Vec<String>)> {
    pages.iter().map(|(name, layers)| (name.to_string(), layers.iter().map(|l| l.to_string()).collect())).collect()
}

const MEASURED: [&str; 6] = ["a_red", "b_green", "c_blue", "d gray", "e_yellow", "f_cyan"];

#[test]
fn include_subfolders_is_on_by_default_and_reads_the_measured_order() {
    let fixture = tree("default", false);
    let dir = fixture.0.clone();
    let big = json!({"columns": 3, "rows": 3});
    let default = sheets(json!(dir), big.clone()).unwrap();
    assert_eq!(default, expect(&[("ContactSheet-001", &MEASURED)]));
    let mut on = big.clone();
    on["includeSubfolders"] = json!(true);
    assert_eq!(sheets(json!(dir), on).unwrap(), default);
    // A full sheet still continues on the next one, in the same order.
    assert_eq!(sheets(json!(dir), json!({})).unwrap(), expect(&[("ContactSheet-001", &MEASURED[..4]), ("ContactSheet-002", &MEASURED[4..])]));
}

#[test]
fn subfolders_come_depth_first_after_the_folders_own_files() {
    let fixture = tree("order", true);
    let dir = fixture.0.clone();
    let all = sheets(json!(dir), json!({"columns": 3, "rows": 3})).unwrap();
    // `0_first/` sorts before `a_red.png` but comes after the top folder's files; `sub/deeper/`
    // comes before `sub2/`.
    assert_eq!(placed(&all), ["a_red", "b_green", "c_blue", "d gray", "g_white", "e_yellow", "f_cyan", "h_black"]);
}

#[test]
fn include_subfolders_off_keeps_the_top_folder_only() {
    let fixture = tree("off", false);
    let dir = fixture.0.clone();
    let top = expect(&[("ContactSheet-001", &MEASURED[..4])]);
    assert_eq!(sheets(json!(dir), json!({"includeSubfolders": false})).unwrap(), top);
    // Group Images by Folder does nothing without Include Subfolders.
    assert_eq!(sheets(json!(dir), json!({"includeSubfolders": false, "groupByFolder": true})).unwrap(), top);
}

#[test]
fn group_by_folder_starts_a_sheet_for_each_folder_with_images() {
    let fixture = tree("group", false);
    let dir = fixture.0.clone();
    // Measured with a 2 × 2 grid: the top folder fills ContactSheet-001, `sub/` and
    // `sub/deeper/` get one sheet each, and the empty `sub/empty/` none.
    let grouped = sheets(json!(dir), json!({"groupByFolder": true})).unwrap();
    assert_eq!(grouped, expect(&[("ContactSheet-001", &MEASURED[..4]), ("ContactSheet-002", &["e_yellow"]), ("ContactSheet-003", &["f_cyan"])]));
    // Off by default: the same images fill the sheets one after another.
    assert_eq!(sheets(json!(dir), json!({"groupByFolder": false})).unwrap(), sheets(json!(dir), json!({})).unwrap());
    // A folder with more images than a sheet holds continues on further sheets before the next
    // folder starts.
    let small = sheets(json!(dir), json!({"groupByFolder": true, "columns": 3, "rows": 1})).unwrap();
    assert_eq!(
        small,
        expect(&[
            ("ContactSheet-001", &MEASURED[..3]),
            ("ContactSheet-002", &["d gray"]),
            ("ContactSheet-003", &["e_yellow"]),
            ("ContactSheet-004", &["f_cyan"]),
        ])
    );
    // A folder without subfolders is one group.
    let deeper = join(&dir, "sub/deeper");
    assert_eq!(sheets(json!(deeper), json!({"groupByFolder": true})).unwrap(), expect(&[("ContactSheet-001", &["f_cyan"])]));
}

#[test]
fn a_file_list_ignores_both_flags() {
    let fixture = tree("list", false);
    let dir = fixture.0.clone();
    let files = json!([join(&dir, "sub/deeper/f_cyan.png"), join(&dir, "a_red.png"), join(&dir, "sub/e_yellow.png")]);
    let listed = expect(&[("ContactSheet-001", &["f_cyan", "a_red", "e_yellow"])]);
    for extra in [json!({}), json!({"includeSubfolders": false}), json!({"includeSubfolders": true, "groupByFolder": true})] {
        assert_eq!(sheets(files.clone(), extra.clone()).unwrap(), listed, "{extra}");
    }
}

#[test]
fn a_flag_of_the_wrong_type_is_an_error() {
    let fixture = tree("types", false);
    let dir = fixture.0.clone();
    for (key, value) in [("includeSubfolders", json!("yes")), ("includeSubfolders", json!(1)), ("groupByFolder", json!("true")), ("groupByFolder", json!([]))] {
        for input in [json!(dir), json!([join(&dir, "a_red.png")])] {
            let mut p = json!({"input": input, "caption": false});
            p[key] = value.clone();
            let e = Session::new().execute("file.automate.contactSheetII", p).unwrap_err().to_string();
            assert!(e.contains(&format!("\"{key}\" must be true or false")), "{key}={value}: {e}");
        }
    }
    // `null` reads as the default.
    assert_eq!(placed(&sheets(json!(dir), json!({"includeSubfolders": null, "columns": 3, "rows": 3})).unwrap()), MEASURED);
}

/// A link from `sub/deeper/` back up to the top folder is walked once: no image comes twice.
/// Windows without developer mode can't make a directory symlink, so a junction stands in.
#[test]
fn a_link_back_up_the_tree_is_walked_once() {
    let fixture = tree("loop", false);
    let dir = fixture.0.clone();
    let link = join(&dir, "sub/deeper/up");
    #[cfg(unix)]
    let made = std::os::unix::fs::symlink(&dir, &link).is_ok();
    #[cfg(windows)]
    let made = std::os::windows::fs::symlink_dir(&dir, &link).is_ok()
        || std::process::Command::new("cmd").args(["/C", "mklink", "/J", &link.replace('/', "\\"), &dir]).output().is_ok_and(|o| o.status.success());
    #[cfg(not(any(unix, windows)))]
    let made = false;
    if !made {
        eprintln!("skipped: cannot create a directory link here");
        return;
    }
    assert!(std::path::Path::new(&link).join("a_red.png").is_file(), "the link leads back to the top folder");
    let all = sheets(json!(dir), json!({"columns": 3, "rows": 3})).unwrap();
    assert_eq!(all, expect(&[("ContactSheet-001", &MEASURED)]));
    let grouped = sheets(json!(dir), json!({"groupByFolder": true})).unwrap();
    assert_eq!(placed(&grouped), MEASURED);
    assert_eq!(grouped.len(), 3);
}
