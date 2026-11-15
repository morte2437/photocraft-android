//! #11403: the document tab's right-click menu lists the reference application's eight entries in
//! order, with its two separators, and each entry runs the command it names on the clicked tab.

use std::cell::RefCell;
use std::rc::Rc;

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Value, json};

use crate::canvas::tabs::{TAB_MENU_SEPARATORS_AFTER, tab_context_items, tab_context_menu};
use crate::state::DialogKind;
use crate::{FileDialogRequest, PhotocraftApp, Services};

/// The app, what the menu is drawn for, and the command the last click returned.
struct Probe {
    app: PhotocraftApp,
    index: usize,
    has_path: bool,
    picked: Option<(&'static str, Value)>,
}

type Requests = Rc<RefCell<Vec<FileDialogRequest>>>;

fn probe(documents: usize, index: usize, has_path: bool) -> (Harness<'static, Probe>, Requests) {
    let requests: Requests = Default::default();
    let held = requests.clone();
    let services = Services { file_dialog: Some(Box::new(move |request, _parent, _reply| held.borrow_mut().push(request))), ..Default::default() };
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), services);
    for n in 0..documents {
        app.run("file.new", json!({"width": 10 + n, "height": 8})).unwrap();
    }
    let state = Probe { app, index, has_path, picked: None };
    let mut h = Harness::builder().with_size(vec2(400.0, 500.0)).build_ui_state(
        |ui, p: &mut Probe| {
            let count = p.app.session.documents().len();
            if let Some(hit) = tab_context_menu(ui, p.index, count, p.has_path) {
                p.picked = Some(hit);
            }
        },
        state,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    (h, requests)
}

/// The drawn rows top to bottom: label, top edge, enabled.
fn rows(h: &Harness<'static, Probe>) -> Vec<(String, f32, bool)> {
    let mut rows: Vec<(String, f32, bool)> = h
        .query_all_by_role(egui::accesskit::Role::Button)
        .map(|n| {
            let a = n.accesskit_node();
            (a.label().unwrap_or_default().to_string(), n.rect().top(), !a.is_disabled())
        })
        .collect();
    rows.sort_by(|a, b| a.1.total_cmp(&b.1));
    rows
}

/// Each document's width, which tells the documents apart (`probe` makes them 10, 11, 12 wide).
fn widths(h: &Harness<'static, Probe>) -> Vec<u32> {
    h.state().app.session.documents().iter().map(|d| d.doc.size.width).collect()
}

fn click(h: &mut Harness<'static, Probe>, label: &str) -> (&'static str, Value) {
    h.get_by_label(label).click();
    h.run_steps(3);
    h.state_mut().picked.take().unwrap_or_else(|| panic!("clicking {label} returned no command"))
}

#[test]
fn the_menu_lists_the_eight_reference_entries_in_order() {
    let (h, _) = probe(1, 0, false);
    let labels: Vec<String> = rows(&h).into_iter().map(|r| r.0).collect();
    assert_eq!(
        labels,
        [
            "Consolidate All to Here",
            "Close",
            "Close All",
            "Close Others",
            "Move to New Window",
            "New Document…",
            "Open Document…",
            crate::menus::reveal_label(),
        ]
    );
}

#[test]
fn separators_follow_move_to_new_window_and_open_document() {
    assert_eq!(TAB_MENU_SEPARATORS_AFTER, [4, 6]);
    let (h, _) = probe(1, 0, false);
    let tops: Vec<f32> = rows(&h).into_iter().map(|r| r.1).collect();
    let gaps: Vec<f32> = tops.windows(2).map(|w| w[1] - w[0]).collect();
    let plain = gaps[0];
    for (i, gap) in gaps.iter().enumerate() {
        let separated = TAB_MENU_SEPARATORS_AFTER.contains(&i);
        assert_eq!(*gap > plain + 1.0, separated, "gap after row {i} is {gap} against a plain {plain}");
    }
}

#[test]
fn each_row_names_its_command_and_the_clicked_tabs_index() {
    let items = tab_context_items(2, 3, true);
    let menu = crate::menus::menu_items(&PhotocraftApp::new(photocraft_engine::Session::new(), Services::default()));
    let want = [
        ("Consolidate All to Here", "window.arrange.consolidateAllToTabs", json!({})),
        ("Close", "file.close", json!({"document": 2})),
        ("Close All", "file.closeAll", json!({})),
        ("Close Others", "file.closeOthers", json!({"document": 2})),
        ("Move to New Window", "window.arrange.floatInWindow", json!({"document": 2})),
        ("New Document…", "file.new", json!({})),
        ("Open Document…", "file.open", json!({})),
        (crate::menus::reveal_label(), "file.revealInFinder", json!({"document": 2})),
    ];
    for ((label, id, params, _), (wl, wi, wp)) in items.iter().zip(want) {
        assert_eq!((*label, *id, params), (wl, wi, &wp));
        assert!(menu.iter().any(|i| i.id == *id) || photocraft_engine::commands::find(id).is_some(), "{id} is a command");
    }
}

#[test]
fn enabled_states_for_one_saved_one_unsaved_and_two_documents() {
    let enabled = |documents: usize, has_path: bool| -> Vec<bool> {
        let (h, _) = probe(documents, 0, has_path);
        rows(&h).into_iter().map(|r| r.2).collect()
    };
    // Consolidate, Close, Close All, Close Others, Move, New, Open, Reveal.
    assert_eq!(enabled(1, true), [false, true, true, false, true, true, true, true], "one saved document");
    assert_eq!(enabled(1, false), [false, true, true, false, true, true, true, false], "one unsaved document");
    assert_eq!(enabled(2, true), [true, true, true, true, true, true, true, true], "two documents");
    assert_eq!(enabled(2, false), [true, true, true, true, true, true, true, false], "two documents, the clicked one unsaved");
}

#[test]
fn close_close_all_and_close_others_act_on_the_clicked_tab() {
    let (mut h, _) = probe(3, 1, true);
    let (id, params) = click(&mut h, "Close");
    assert_eq!((id, &params), ("file.close", &json!({"document": 1})));
    let ctx = h.ctx.clone();
    crate::menus::invoke(&mut h.state_mut().app, &ctx, id, params).unwrap();
    let names = widths(&h);
    assert_eq!(names, [10, 12]);

    let (mut h, _) = probe(3, 1, true);
    let (id, params) = click(&mut h, "Close Others");
    crate::menus::invoke(&mut h.state_mut().app, &ctx, id, params).unwrap();
    let names = widths(&h);
    assert_eq!(names, [11]);

    let (mut h, _) = probe(3, 1, true);
    let (id, params) = click(&mut h, "Close All");
    crate::menus::invoke(&mut h.state_mut().app, &ctx, id, params).unwrap();
    assert!(h.state().app.session.documents().is_empty());
}

#[test]
fn move_to_new_window_floats_the_clicked_tab_even_with_one_document() {
    let ctx = egui::Context::default();
    let (mut h, _) = probe(1, 0, false);
    let (id, params) = click(&mut h, "Move to New Window");
    assert_eq!((id, &params), ("window.arrange.floatInWindow", &json!({"document": 0})));
    crate::menus::invoke(&mut h.state_mut().app, &ctx, id, params).unwrap();
    assert!(h.state().app.ui.windows.iter().any(|w| w.floating && w.document == 0));

    // The clicked tab (2) is not the active document (0).
    let (mut h, _) = probe(3, 2, true);
    h.state_mut().app.session.set_active(0);
    assert_eq!(h.state().app.session.active_index(), Some(0));
    let (id, params) = click(&mut h, "Move to New Window");
    crate::menus::invoke(&mut h.state_mut().app, &ctx, id, params).unwrap();
    let windows = &h.state().app.ui.windows;
    assert!(windows.iter().any(|w| w.floating && w.document == 2), "the clicked tab floats");
    assert!(!windows.iter().any(|w| w.document == 0), "the active document does not float");
}

#[test]
fn consolidate_all_to_here_brings_floating_windows_back() {
    let ctx = egui::Context::default();
    let (mut h, _) = probe(2, 0, true);
    crate::menus::invoke(&mut h.state_mut().app, &ctx, "window.arrange.floatAllInWindows", json!({})).unwrap();
    assert_eq!(h.state().app.ui.windows.len(), 2);
    let (id, params) = click(&mut h, "Consolidate All to Here");
    assert_eq!((id, &params), ("window.arrange.consolidateAllToTabs", &json!({})));
    crate::menus::invoke(&mut h.state_mut().app, &ctx, id, params).unwrap();
    assert!(h.state().app.ui.windows.is_empty());
}

#[test]
fn new_document_opens_the_new_document_dialog() {
    let ctx = egui::Context::default();
    let (mut h, _) = probe(1, 0, false);
    let (id, params) = click(&mut h, "New Document…");
    assert_eq!((id, &params), ("file.new", &json!({})));
    crate::menus::invoke(&mut h.state_mut().app, &ctx, id, params).unwrap();
    assert!(h.state().app.ui.dialogs.iter().any(|d| d.kind == DialogKind::NewDocument));
}

#[test]
fn open_document_asks_for_files_like_file_open() {
    let ctx = egui::Context::default();
    let (mut h, requests) = probe(1, 0, false);
    let (id, params) = click(&mut h, "Open Document…");
    assert_eq!((id, &params), ("file.open", &json!({})));
    crate::menus::invoke(&mut h.state_mut().app, &ctx, id, params).unwrap();
    h.state_mut().app.poll_file_dialog(&ctx, None);
    let asked = requests.borrow();
    assert!(matches!(asked.as_slice(), [FileDialogRequest::Open { multiple: true, .. }]), "{asked:?}");
}

#[test]
fn reveal_keeps_its_platform_label_and_command() {
    let (mut h, _) = probe(1, 0, true);
    let (id, params) = click(&mut h, crate::menus::reveal_label());
    assert_eq!((id, &params), ("file.revealInFinder", &json!({"document": 0})));
}
