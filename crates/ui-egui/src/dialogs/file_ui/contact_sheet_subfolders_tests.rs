//! #13687: Contact Sheet II's Include Subfolders and Group Images by Folder checkboxes.
use crate::PhotocraftApp;
use egui::accesskit::Toggled;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

const INCLUDE: &str = "Include Subfolders";
const GROUP: &str = "Group Images by Folder";
const TIP: &str = "Images in subfolders will start on a new Contact Sheet";

/// The app with the Contact Sheet II dialog open from File › Automate.
fn dialog() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1440.0, 1100.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    });
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "file.automate.contactSheetII", json!({})).unwrap();
    h.run_steps(4);
    h
}

/// The open dialog's `key` field.
fn field(h: &Harness<'_, PhotocraftApp>, key: &str) -> Value {
    h.state().ui.dialogs.last().and_then(|d| d.fields.get(key).cloned()).unwrap_or(Value::Null)
}

/// Whether the checkbox labelled `label` is ticked, and whether it can be clicked.
fn checkbox(h: &Harness<'_, PhotocraftApp>, label: &str) -> (bool, bool) {
    let node = h.get_by_label(label);
    let node = node.accesskit_node();
    (node.toggled() == Some(Toggled::True), !node.is_disabled())
}

#[test]
fn the_checkboxes_sit_under_the_folder_with_the_reference_defaults() {
    let h = dialog();
    assert_eq!(checkbox(&h, INCLUDE), (true, true), "Include Subfolders is on");
    assert_eq!(checkbox(&h, GROUP), (false, true), "Group Images by Folder is off and usable");
    assert_eq!((field(&h, "includeSubfolders"), field(&h, "groupByFolder")), (json!(true), json!(false)));
    // Source Images comes first: the folder, Include Subfolders, then Group Images by Folder
    // right below it, before every other control of the form.
    // A field's label and the field itself share a name; the row's top is the higher one.
    let top = |label: &str| h.query_all_by_label(label).map(|n| n.rect().top()).reduce(f32::min).unwrap_or_else(|| panic!("no {label}"));
    let (input, include, group) = (top("Input"), top(INCLUDE), top(GROUP));
    assert!(input < include && include < group, "Input {input}, {INCLUDE} {include}, {GROUP} {group}");
    for other in ["Auto spacing", "Caption", "Columns", "Flatten", "Width"] {
        assert!(group < top(other), "{GROUP} {group} is above {other} {}", top(other));
    }
}

#[test]
fn group_images_by_folder_is_greyed_while_include_subfolders_is_off() {
    let mut h = dialog();
    h.get_by_label(GROUP).click();
    h.run_steps(4);
    assert_eq!(checkbox(&h, GROUP), (true, true));
    assert_eq!(field(&h, "groupByFolder"), json!(true));
    h.get_by_label(INCLUDE).click();
    h.run_steps(4);
    assert_eq!(checkbox(&h, INCLUDE), (false, true));
    assert_eq!(checkbox(&h, GROUP), (true, false), "greyed, keeping its tick");
    assert_eq!(field(&h, "includeSubfolders"), json!(false));
    // A click on the greyed checkbox changes nothing.
    h.get_by_label(GROUP).click();
    h.run_steps(4);
    assert_eq!(field(&h, "groupByFolder"), json!(true));
    // Its tooltip still explains it.
    h.get_by_label(GROUP).hover();
    h.run_steps(60);
    assert!(h.query_by_label(TIP).is_some(), "the tooltip of the greyed checkbox");
    h.get_by_label(INCLUDE).click();
    h.run_steps(4);
    assert_eq!(checkbox(&h, INCLUDE), (true, true));
    assert_eq!(checkbox(&h, GROUP), (true, true), "usable again");
}

#[test]
fn ok_runs_the_sheet_with_both_settings() {
    /// Removes the test folder when it goes out of scope, also when the test panics.
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    let dir = std::env::temp_dir().join(format!("pc-contact-subfolders-ui-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let _cleanup = Cleanup(dir.clone());
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    for (i, rel) in ["top.png", "sub/inner.png"].into_iter().enumerate() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 8 + i, "height": 6})).unwrap();
        s.execute("file.saveACopy", json!({"path": dir.join(rel).to_string_lossy()})).unwrap();
    }
    let sheets = |group: bool| {
        let mut h = dialog();
        let id = h.state().ui.dialogs.last().unwrap().id;
        let fields = &mut h.state_mut().ui.dialog_mut(id).unwrap().fields;
        fields.insert("input".into(), json!(dir.to_string_lossy()));
        // No captions: they wait on the app's fonts, which load in the background.
        fields.insert("caption".into(), json!(false));
        if group {
            h.get_by_label(GROUP).click();
            h.run_steps(4);
        }
        h.get_by_label("OK").click();
        h.run_steps(8);
        assert!(h.state().ui.dialogs.is_empty(), "OK closes the dialog");
        // Each sheet's image layers: all but its background.
        let docs = h.state().session.documents().iter().map(|d| (d.doc.name.clone(), d.doc.layers.len() - 1)).collect::<Vec<_>>();
        (docs, h.state().ui.status.clone())
    };
    // Include Subfolders (on) takes `sub/inner.png` too; Group puts it on its own sheet.
    let ((all, all_status), (grouped, grouped_status)) = (sheets(false), sheets(true));
    assert_eq!(all, [("ContactSheet-001".to_string(), 2)], "status: {all_status}");
    assert_eq!(grouped, [("ContactSheet-001".to_string(), 1), ("ContactSheet-002".to_string(), 1)], "status: {grouped_status}");
}
