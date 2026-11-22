//! #14056: files picked together in File > Open open in the order picked, the last one active;
//! the command line and drops keep name order.

use crate::file_dialog::{self, FileDialogAnswer};
use crate::{PhotocraftApp, Services};
use photocraft_color::{ColorMode, SampleType};
use photocraft_doc::Document;
use photocraft_geom::Size;

fn app_with(answers: Vec<Option<FileDialogAnswer>>) -> PhotocraftApp {
    let services = Services {
        import: Some(Box::new(|name: &str, _bytes: &[u8], _opts: &photocraft_io::ImportOptions| {
            Ok((Document::new(name, Size::new(4, 4), ColorMode::Rgb, SampleType::U8), Vec::new()))
        })),
        file_dialog: Some(file_dialog::fake(answers).0),
        ..Default::default()
    };
    PhotocraftApp::new(photocraft_engine::Session::new(), services)
}

fn files(tag: &str, names: &[&str]) -> (std::path::PathBuf, Vec<String>) {
    let dir = std::env::temp_dir().join(format!("photocraft-dialog-order-{tag}-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let paths = names
        .iter()
        .map(|name| {
            let path = dir.join(name);
            std::fs::write(&path, b"x").unwrap();
            path.to_string_lossy().into_owned()
        })
        .collect();
    (dir, paths)
}

fn doc_names(app: &PhotocraftApp) -> Vec<String> {
    app.session.documents().iter().map(|d| d.doc.name.clone()).collect()
}

fn active_name(app: &PhotocraftApp) -> Option<String> {
    app.session.active().map(|s| s.doc.name.clone())
}

fn pick(app: &mut PhotocraftApp) {
    app.open_dialog_file().unwrap();
    app.poll_file_dialog(&egui::Context::default(), None);
}

#[test]
fn dialog_picks_open_in_the_order_picked_and_the_last_is_active() {
    let (dir, paths) = files("order", &["flat.png", "layers.psd", "flat.jpg"]);
    let mut app = app_with(vec![Some(FileDialogAnswer::Paths(paths))]);
    pick(&mut app);
    assert_eq!(doc_names(&app), ["flat.png", "layers.psd", "flat.jpg"]);
    assert_eq!(active_name(&app).as_deref(), Some("flat.jpg"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_picked_file_that_is_already_open_is_activated_not_opened_twice() {
    let (dir, paths) = files("already", &["flat.png", "layers.psd", "flat.jpg"]);
    let mut app = app_with(vec![Some(FileDialogAnswer::Paths(vec![paths[1].clone(), paths[0].clone()]))]);
    assert_eq!(app.open_paths_in_order(&[paths[2].clone()]), 1);
    pick(&mut app);
    assert_eq!(doc_names(&app), ["flat.jpg", "layers.psd", "flat.png"], "each file once");
    assert_eq!(active_name(&app).as_deref(), Some("flat.png"));
    // The last pick being the already open file activates it.
    let mut app = app_with(vec![Some(FileDialogAnswer::Paths(vec![paths[0].clone(), paths[2].clone()]))]);
    assert_eq!(app.open_paths_in_order(&[paths[2].clone()]), 1);
    pick(&mut app);
    assert_eq!(doc_names(&app), ["flat.jpg", "flat.png"]);
    assert_eq!(active_name(&app).as_deref(), Some("flat.jpg"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn the_command_line_and_drop_path_still_opens_in_name_order() {
    let (dir, paths) = files("sorted", &["flat.png", "layers.psd", "flat.jpg"]);
    let mut app = app_with(Vec::new());
    assert_eq!(app.open_paths(&paths), 3);
    assert_eq!(doc_names(&app), ["flat.jpg", "flat.png", "layers.psd"]);
    assert_eq!(active_name(&app).as_deref(), Some("layers.psd"));
    let _ = std::fs::remove_dir_all(&dir);
}
