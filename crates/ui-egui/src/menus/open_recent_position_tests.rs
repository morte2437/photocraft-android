//! File menu: Open Recent comes directly after Open as Smart Object (#14053).

use super::*;

fn app_with(recent: &[&str]) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.recent_files = recent.iter().map(|s| (*s).to_string()).collect();
    app
}

#[test]
fn open_recent_follows_open_as_smart_object() {
    let app = app_with(&["/tmp/a.png"]);
    let items = menu_items(&app);
    let mut flat: Vec<String> = Vec::new();
    let mut seen_recent = false;
    for item in items.iter().filter(|i| i.path.first().map(String::as_str) == Some("File")) {
        if item.path.len() == 2 && item.path[1] == "Open Recent" {
            if !seen_recent {
                flat.push("Open Recent".to_string());
                seen_recent = true;
            }
        } else if item.path.len() == 1 {
            flat.push(item.id.clone());
        }
        if flat.len() >= 8 {
            break;
        }
    }
    assert_eq!(
        flat,
        vec!["file.new", "file.newFromClipboard", "file.open", "file.openAs", "file.openAsSmartObject", "Open Recent", "file.openExrParts", "---"]
    );
}

#[test]
fn open_recent_entries_are_unchanged() {
    let rp = vec!["File".to_string(), "Open Recent".to_string()];
    for (recent, expect) in [
        (vec![], vec![("file.clearRecent", "Clear Recent File List", false)]),
        (
            vec!["/tmp/a.png", "/dir/b.psd"],
            vec![
                ("file.openRecent.0", "a.png", true),
                ("file.openRecent.1", "b.psd", true),
                ("---", "---", false),
                ("file.clearRecent", "Clear Recent File List", true),
            ],
        ),
    ] {
        let app = app_with(&recent);
        let got: Vec<(String, String, bool)> = menu_items(&app).into_iter().filter(|i| i.path == rp).map(|i| (i.id, i.label, i.enabled)).collect();
        let want: Vec<(String, String, bool)> = expect.into_iter().map(|(a, b, c)| (a.to_string(), b.to_string(), c)).collect();
        assert_eq!(got, want);
    }
}
