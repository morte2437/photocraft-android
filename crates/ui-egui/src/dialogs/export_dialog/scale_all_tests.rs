//! #13734: Export As's Scale All rows. + adds sizes in the measured order with `@<size>` suffixes,
//! up to 10 rows; each row's trash removes it (greyed with one row left). One row exports through
//! the save dialog; several ask for a folder and write `<name><suffix>.<ext>` per row at its size.
//! The rows are remembered after Export and not after Cancel.

use super::*;
use egui::accesskit::Role;
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::{Color, SampleType, Size};
use std::{cell::RefCell, rc::Rc};

type Writes = Rc<RefCell<Vec<(String, Vec<u8>)>>>;
type Asked = Rc<RefCell<Vec<crate::file_dialog::FileDialogRequest>>>;

fn rows(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter().map(|(a, b)| ((*a).to_string(), (*b).to_string())).collect()
}

#[test]
fn plus_adds_sizes_in_the_measured_order_up_to_ten_rows() {
    let mut list = rows(&[("1x", "")]);
    while let Some(row) = next_scale_row(&list) {
        list.push(row);
    }
    let sizes: Vec<&str> = list.iter().map(|(s, _)| s.as_str()).collect();
    assert_eq!(sizes, ["1x", "0.5x", "0.75x", "0.25x", "2x", "0.33x", "1.5x", "0.1x", "3x", "1.25x"]);
    assert_eq!(list[1].1, "@0.5x");
    assert_eq!(list[9].1, "@1.25x");
    assert!(!sizes.contains(&"0.3x"), "0.3x is never added");
    assert_eq!(SCALE_SIZES.len(), 11);
    // A changed first row: + still skips the sizes already in use.
    assert_eq!(next_scale_row(&rows(&[("0.5x", "")])), Some(("0.75x".into(), "@0.75x".into())));
}

#[test]
fn rows_validate_scale_the_size_and_name_files() {
    assert!(valid_scale_rows(&json!([{"size": "1x", "suffix": ""}, {"size": "0.3x", "suffix": "@small"}])));
    assert!(!valid_scale_rows(&json!([])), "at least one row");
    assert!(!valid_scale_rows(&json!([{"size": "4x", "suffix": ""}])), "a listed size");
    assert!(!valid_scale_rows(&Value::Array(vec![json!({"size": "1x", "suffix": ""}); 11])), "at most 10 rows");
    let f = json!({"scale": 100, "width": 200, "height": 150, "sizing": "pixels"}).as_object().unwrap().clone();
    let half = scaled_fields(&f, scale_factor("0.5x"));
    assert_eq!((n(&half, "scale", 0.0), n(&half, "width", 0.0), n(&half, "height", 0.0)), (50.0, 100.0, 75.0));
    assert_eq!(scaled_fields(&f, scale_factor("1x")), f);
    assert_eq!(file_suffix("@2x/../a:b"), "@2x_.._a_b");
}

fn harness(answer: crate::FileDialogAnswer) -> (Harness<'static, PhotocraftApp>, Writes, Asked) {
    let writes = Writes::default();
    let captured = writes.clone();
    let (picker, asked) = crate::file_dialog::fake(vec![Some(answer)]);
    let mut picker = Some(picker);
    let h = Harness::builder().with_size(egui::vec2(1100.0, 900.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let services = crate::Services {
            file_dialog: picker.take(),
            export: Some(Box::new(|doc, path, settings| {
                photocraft_io::export(doc, path, &settings.io_options()).map(|r| (r.bytes, r.warnings)).map_err(|e| e.to_string())
            })),
            write: Some(Box::new(move |path, bytes| {
                captured.borrow_mut().push((path.to_string(), bytes.to_vec()));
                Ok(())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
        let mut doc = Document::with_background("harbor", Size::new(200, 150), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        doc.resolution_dpi = 144.0;
        app.session.add_document(doc, None);
        app
    });
    (h, writes, asked)
}

fn open_dialog(h: &mut Harness<'_, PhotocraftApp>) {
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, COMMAND, json!({})).unwrap();
    h.run_steps(6);
}

fn dialog_rows(h: &Harness<'_, PhotocraftApp>) -> Vec<(String, String)> {
    scale_rows(&h.state().ui.dialogs.last().expect("Export As is open").fields)
}

fn click(h: &mut Harness<'_, PhotocraftApp>, label: &str) {
    h.get_by_label(label).click();
    h.run_steps(4);
}

fn disabled(h: &Harness<'_, PhotocraftApp>, label: &str) -> bool {
    h.get_by_label(label).accesskit_node().is_disabled()
}

/// The decoded file's pixel size and resolution.
fn size_of(bytes: &[u8]) -> (u32, u32, f32) {
    let doc = photocraft_io::import("x.png", bytes).unwrap().document;
    (doc.size.width, doc.size.height, doc.resolution_dpi)
}

#[test]
fn rows_are_added_up_to_ten_and_trash_removes_one_but_never_the_last() {
    let (mut h, _, _) = harness(crate::FileDialogAnswer::SaveTo("x.png".into()));
    open_dialog(&mut h);
    assert_eq!(dialog_rows(&h), rows(&[("1x", "")]));
    assert!(disabled(&h, "Remove size 1"), "greyed with one row");
    for _ in 0..12 {
        click(&mut h, "Add a size");
    }
    let list = dialog_rows(&h);
    assert_eq!(list.len(), 10, "at most 10 rows");
    assert_eq!(list[1], ("0.5x".to_string(), "@0.5x".to_string()));
    assert!(!disabled(&h, "Remove size 1"));
    click(&mut h, "Remove size 2");
    let list = dialog_rows(&h);
    assert_eq!(list.len(), 9);
    assert_eq!(list[1].0, "0.75x", "the 0.5x row is gone");
}

#[test]
fn several_rows_ask_for_a_folder_and_write_one_file_per_size() {
    let dir = std::env::temp_dir().join(format!("pc-scale-all-{}", std::process::id()));
    let (mut h, writes, asked) = harness(crate::FileDialogAnswer::FolderTo(dir.to_string_lossy().into_owned()));
    open_dialog(&mut h);
    click(&mut h, "Add a size");
    h.get_by_role_and_label(Role::Button, "Export").click();
    h.run_steps(6);
    assert!(matches!(asked.borrow().first(), Some(crate::file_dialog::FileDialogRequest::Folder { .. })), "a folder choice");
    let writes = writes.borrow();
    let paths: Vec<&str> = writes.iter().map(|(p, _)| p.as_str()).collect();
    let want = [dir.join("harbor.png"), dir.join("harbor@0.5x.png")];
    assert_eq!(paths, want.iter().map(|p| p.to_str().unwrap()).collect::<Vec<_>>());
    let (full, half) = (size_of(&writes[0].1), size_of(&writes[1].1));
    assert_eq!((full.0, full.1), (200, 150));
    assert_eq!((half.0, half.1), (100, 75), "half the size");
    assert_eq!(half.2, full.2, "the same ppi");
}

#[test]
fn one_row_uses_the_save_dialog_at_its_size() {
    let (mut h, writes, asked) = harness(crate::FileDialogAnswer::SaveTo("out.png".into()));
    open_dialog(&mut h);
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("scaleAll".into(), json!([{"size": "2x", "suffix": "@2x"}]));
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Export").click();
    h.run_steps(6);
    assert!(matches!(asked.borrow().first(), Some(crate::file_dialog::FileDialogRequest::Save { .. })), "today's save dialog");
    let (w, h, _) = size_of(&writes.borrow()[0].1);
    assert_eq!((w, h), (400, 300));
}

#[test]
fn rows_are_remembered_after_export_not_after_cancel() {
    let dir = std::env::temp_dir().join(format!("pc-scale-all-memory-{}", std::process::id()));
    let (mut h, _, _) = harness(crate::FileDialogAnswer::FolderTo(dir.to_string_lossy().into_owned()));
    open_dialog(&mut h);
    click(&mut h, "Add a size");
    h.get_by_role_and_label(Role::Button, "Cancel").click();
    h.run_steps(4);
    open_dialog(&mut h);
    assert_eq!(dialog_rows(&h).len(), 1, "Cancel forgets the rows");
    click(&mut h, "Add a size");
    click(&mut h, "Add a size");
    h.get_by_role_and_label(Role::Button, "Export").click();
    h.run_steps(6);
    open_dialog(&mut h);
    assert_eq!(dialog_rows(&h), rows(&[("1x", ""), ("0.5x", "@0.5x"), ("0.75x", "@0.75x")]), "Export remembers them");
}

/// CodeRabbit on #15028: two rows that would write one file are refused before anything is written.
#[test]
fn rows_that_share_an_output_file_are_refused() {
    let same = rows(&[("1x", ""), ("0.5x", "")]);
    let err = batch_file_names("harbor", "png", &same).unwrap_err();
    assert!(err.contains("harbor.png"), "{err}");
    // Suffixes that sanitize alike, or differ only in case.
    assert!(batch_file_names("harbor", "png", &rows(&[("1x", "a/b"), ("2x", "a:b")])).is_err());
    assert!(batch_file_names("harbor", "png", &rows(&[("1x", "@Big"), ("2x", "@big")])).is_err());
    // Distinct suffixes pass, in row order.
    let ok = batch_file_names("harbor", "png", &rows(&[("1x", ""), ("0.5x", "@0.5x")])).unwrap();
    assert_eq!(ok, ["harbor.png", "harbor@0.5x.png"]);
}

#[test]
fn exporting_rows_with_one_name_writes_nothing_and_says_why() {
    let dir = std::env::temp_dir().join(format!("pc-scale-dup-{}", std::process::id()));
    let (mut h, writes, asked) = harness(crate::FileDialogAnswer::FolderTo(dir.to_string_lossy().into_owned()));
    open_dialog(&mut h);
    click(&mut h, "Add a size");
    h.state_mut().ui.dialogs.last_mut().unwrap().fields.insert("scaleAll".into(), scale_rows_value(&rows(&[("1x", ""), ("0.5x", "")])));
    h.run_steps(2);
    h.get_by_role_and_label(Role::Button, "Export").click();
    h.run_steps(6);
    assert!(writes.borrow().is_empty(), "nothing written");
    assert!(asked.borrow().is_empty(), "no folder asked for");
    assert!(h.state().ui.status.contains("would write the same file"), "{}", h.state().ui.status);
}

/// CodeRabbit on #15028: a suffix field takes at most the accepted 64 characters.
#[test]
fn a_suffix_longer_than_the_limit_cannot_be_typed() {
    let (mut h, _, _) = harness(crate::FileDialogAnswer::SaveTo("x.png".into()));
    open_dialog(&mut h);
    h.get_by_label("Suffix 1").click();
    h.run_steps(2);
    h.get_by_label("Suffix 1").type_text(&"s".repeat(80));
    h.run_steps(4);
    let list = dialog_rows(&h);
    assert_eq!(list, vec![("1x".to_string(), "s".repeat(64))], "capped, and the row list kept");
}

/// CodeRabbit on #15030: only a folder was chosen, so a file already there is never replaced. Any
/// existing target stops the batch before the first write.
#[test]
fn a_file_already_in_the_folder_stops_the_batch_before_any_write() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("harbor@0.5x.png"), b"keep me").unwrap();
    let (mut h, writes, _) = harness(crate::FileDialogAnswer::FolderTo(dir.path().to_string_lossy().into_owned()));
    open_dialog(&mut h);
    click(&mut h, "Add a size");
    h.get_by_role_and_label(Role::Button, "Export").click();
    h.run_steps(6);
    assert!(writes.borrow().is_empty(), "not even the first row (harbor.png) was written");
    assert!(h.state().ui.status.contains("already exists"), "{}", h.state().ui.status);
    assert_eq!(std::fs::read(dir.path().join("harbor@0.5x.png")).unwrap(), b"keep me");
}

/// CodeRabbit on #15028: the browser build cannot write a folder of files, so several rows are refused
/// with a message before any folder request; one row is fine there.
#[test]
fn several_rows_are_refused_in_the_browser_build_with_the_reason() {
    assert!(batch_unsupported(2, true).is_some_and(|m| m.contains("browser")));
    assert_eq!(batch_unsupported(1, true), None, "one size uses the save dialog");
    assert_eq!(batch_unsupported(5, false), None, "the desktop build writes the folder");
}

/// CodeRabbit on #15039, for the same names here: a final sigma and a sigma are one file on Windows.
#[test]
fn names_that_fold_together_by_uppercase_are_one_file() {
    assert!(batch_file_names("harbor", "png", &rows(&[("1x", "σ"), ("2x", "ς")])).is_err());
    assert!(batch_file_names("harbor", "png", &rows(&[("1x", "σ"), ("2x", "s")])).is_ok());
    // Lower-casing joins `K` and the Kelvin sign (APFS treats them as one name); upper-casing does not.
    assert!(batch_file_names("harbor", "png", &rows(&[("1x", "K"), ("2x", "\u{212A}")])).is_err());
}
