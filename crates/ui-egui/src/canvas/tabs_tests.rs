//! #1276, #4487: a document tab strip with more tabs than fit keeps the active document's tab on
//! the strip, keeps whole titles on the tabs it shows, and moves the rest into a » menu (whole
//! app, real tree).

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;
use crate::theme::ThemeKind;

use super::{TabStrip, pro_tab_title};

/// A real app window with `docs` documents named "ch1-pNN.png".
fn harness(docs: usize, theme: ThemeKind) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, theme);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        // The theme is an interface preference, applied on the first frame: set the preference.
        app.run("prefs.set", json!({"values": {"interface.theme": theme.id()}})).unwrap();
        for n in 1..=docs {
            app.run("file.new", json!({"width": 400, "height": 300, "name": format!("ch1-p{n:02}.png")})).unwrap();
        }
        app
    });
    h.run_steps(4);
    h
}

fn strip(h: &Harness<'_, PhotocraftApp>) -> TabStrip {
    h.state().tab_strip.clone().expect("the strip is drawn")
}

fn has_menu(h: &Harness<'_, PhotocraftApp>) -> bool {
    h.query_by_label("More documents").is_some()
}

#[test]
fn overflowing_tabs_stay_on_the_strip_and_the_rest_move_to_the_menu() {
    let h = harness(14, ThemeKind::Pro);
    assert_eq!(h.state().session.documents().len(), 14);
    let s = strip(&h);
    assert!(s.tabs.len() < 14, "some tabs must overflow into the menu: {} shown", s.tabs.len());
    for (_, r) in &s.tabs {
        assert!(s.rect.contains_rect(*r), "a tab runs past the strip: {r:?} in {:?}", s.rect);
    }
    assert!(has_menu(&h), "the strip carries a » menu while tabs are hidden");
    // The active document is the first thing the fit keeps: its tab is on the strip, not behind
    // the menu, however many tabs are open (#1276).
    let title = pro_tab_title(h.state(), h.state().session.active_index().unwrap());
    assert!(h.query_by_label(&title).is_some(), "the active tab is on the strip: {title}");
}

#[test]
fn the_active_document_brings_its_tab_onto_the_strip() {
    let mut h = harness(14, ThemeKind::Pro);
    for pick in [0, 13, 7] {
        h.state_mut().session.set_active(pick);
        h.run_steps(2);
        let title = pro_tab_title(h.state(), pick);
        assert!(h.query_by_label(&title).is_some(), "tab {pick} is focused on the strip: {title}");
    }
}

#[test]
fn tabs_that_fit_keep_their_natural_widths_and_no_menu() {
    let h = harness(3, ThemeKind::Pro);
    let s = strip(&h);
    assert_eq!(s.tabs.len(), 3, "every tab stays on the strip when they fit");
    assert!(h.query_by_label("More documents").is_none(), "no overflow menu when the tabs fit");
    for i in 0..3 {
        assert!(h.query_by_label(&pro_tab_title(h.state(), i)).is_some(), "tab {i} is on the strip");
    }
}

#[test]
fn studio_tabs_overflow_into_the_same_menu() {
    let h = harness(14, ThemeKind::Studio);
    let s = strip(&h);
    assert!(s.tabs.len() < 14, "{} shown", s.tabs.len());
    for (_, r) in &s.tabs {
        assert!(s.rect.contains_rect(*r), "a tab runs past the strip: {r:?} in {:?}", s.rect);
    }
    assert!(has_menu(&h), "the Studio strip carries a » menu too");
    let active = h.state().session.active_index().unwrap();
    let name = h.state().session.documents()[active].doc.name.clone();
    assert!(h.query_by_label(&name).is_some(), "the active tab is on the strip: {name}");
}

#[test]
fn saved_document_tabs_track_file_identity() {
    for theme in ThemeKind::ALL {
        let mut h = harness(2, theme);
        h.state_mut().services.export = Some(Box::new(|_, _, _| Ok((vec![1], Vec::new()))));
        h.state_mut().services.write = Some(Box::new(|_, _| Ok(())));
        h.state_mut().save_as(Some("folder/My saved image.psd".into())).unwrap();
        h.run_steps(2);
        let pro = crate::theme::Tokens::get(&h.ctx).pro;
        let app = h.state();
        let label = if pro { pro_tab_title(app, 1) } else { "My saved image.psd".into() };
        assert!(h.query_by_label(&label).is_some(), "saved tab in {theme:?}: {label}");
        assert_eq!(app.session.active().unwrap().doc.name, "My saved image.psd");
        h.state_mut().session.set_active(0);
        h.run_steps(2);
        assert_eq!(h.state().session.active().unwrap().doc.name, "ch1-p01.png");
        h.state_mut().session.set_active(1);
        h.state_mut().run("layer.new.layer", json!({})).unwrap();
        h.run_steps(2);
        let label = if pro { pro_tab_title(h.state(), 1) } else { "My saved image.psd".into() };
        assert!(h.query_by_label(&label).is_some());
        assert_eq!(h.state().session.active().unwrap().doc.name, "My saved image.psd");
        assert!(h.state().session.active().unwrap().is_dirty());
    }
}

/// What a Ctrl press reports: on Windows and Linux Ctrl is also the command key, on macOS it isn't.
fn ctrl() -> egui::Modifiers {
    egui::Modifiers { ctrl: true, command: !cfg!(target_os = "macos"), ..egui::Modifiers::NONE }
}

/// #2340: Ctrl+Tab shows the next document's tab and Ctrl+Shift+Tab the previous one, wrapping
/// around at either end, as in Photoshop. Plain Tab still hides the panels instead.
#[test]
fn ctrl_tab_cycles_through_the_document_tabs() {
    let mut h = harness(3, ThemeKind::Pro);
    let active = |h: &Harness<'_, PhotocraftApp>| h.state().session.active_index();
    assert_eq!(active(&h), Some(2), "the last new document is active");
    let press = |h: &mut Harness<'_, PhotocraftApp>, shift: bool| {
        h.key_press_modifiers(if shift { ctrl() | egui::Modifiers::SHIFT } else { ctrl() }, egui::Key::Tab);
        h.run_steps(2);
    };
    press(&mut h, false);
    assert_eq!(active(&h), Some(0), "Ctrl+Tab on the last tab wraps to the first");
    press(&mut h, false);
    assert_eq!(active(&h), Some(1));
    press(&mut h, true);
    assert_eq!(active(&h), Some(0), "Ctrl+Shift+Tab goes back");
    press(&mut h, true);
    assert_eq!(active(&h), Some(2), "and wraps to the last tab");
    assert!(h.state().ui.panels.dock, "Ctrl+Tab doesn't hide the panels");
    // Tab order is document order, so a dragged tab is cycled through at its new place.
    h.state_mut().session.move_document(0, 2);
    h.state_mut().session.set_active(1);
    press(&mut h, false);
    assert_eq!(h.state().session.active().unwrap().doc.name, "ch1-p01.png");
    h.key_press(egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(active(&h), Some(2), "plain Tab doesn't switch documents");
    assert!(!h.state().ui.panels.dock, "plain Tab hides the panels");
}

/// One document: Ctrl+Tab stays on it quietly; a focused widget or text field doesn't keep the key.
#[test]
fn ctrl_tab_with_one_document_or_a_focused_field() {
    let mut h = harness(1, ThemeKind::Pro);
    h.key_press_modifiers(ctrl(), egui::Key::Tab);
    h.run_steps(2);
    assert_eq!(h.state().session.active_index(), Some(0));
    assert!(crate::shortcut_dispatch::take_log(&h.ctx).iter().all(|(_, o)| *o == crate::shortcut_dispatch::Outcome::Ran), "no disabled-shortcut report");
    use crate::shortcut_dispatch::Focus;
    for (key, sc) in [("Ctrl+Tab", "window.nextDocument"), ("Ctrl+Shift+Tab", "window.previousDocument")] {
        let parsed = crate::shortcuts::parse(key).unwrap();
        assert!(Focus::Widget.allows(&parsed) && Focus::Text.allows(&parsed), "{key} reaches {sc} from a focused field");
        let bound = crate::shortcut_dispatch::bindings(h.state()).into_iter().find(|(_, s)| *s == parsed).map(|(id, _)| id);
        assert_eq!(bound.as_deref(), Some(sc));
    }
    assert!(!Focus::Widget.allows(&crate::shortcuts::parse("Tab").unwrap()), "a focused widget keeps plain Tab");
}

#[test]
fn quick_mask_mode_names_the_quick_mask_in_the_tab() {
    let mut h = harness(1, ThemeKind::Pro);
    h.state_mut().run("layer.new.layer", json!({})).unwrap();
    assert_eq!(pro_tab_title(h.state(), 0), "ch1-p01.png @ 100% (Layer 1, RGB/8)*");
    h.state_mut().run("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    assert_eq!(pro_tab_title(h.state(), 0), "ch1-p01.png @ 100% (Quick Mask/8)*");
    h.state_mut().run("select.editInQuickMaskMode", json!({"on": false})).unwrap();
    assert_eq!(pro_tab_title(h.state(), 0), "ch1-p01.png @ 100% (Layer 1, RGB/8)*");
}

/// Whether any title painted on the strip was cut with '…'.
fn elided_on_strip(h: &Harness<'_, PhotocraftApp>) -> bool {
    fn elided(shape: &egui::Shape, strip: egui::Rect) -> bool {
        match shape {
            egui::Shape::Text(text) => text.galley.elided && strip.intersects(text.visual_bounding_rect()),
            egui::Shape::Vec(shapes) => shapes.iter().any(|shape| elided(shape, strip)),
            _ => false,
        }
    }
    let strip = strip(h).rect;
    h.output().shapes.iter().any(|shape| elided(&shape.shape, strip))
}

/// #4487: a crowded strip keeps whole names on the tabs it shows and moves the rest into the »
/// menu instead of shrinking them all, and the menu lists every open document with the active one
/// checked, as Photoshop's does.
#[test]
fn crowded_tabs_keep_their_whole_names_and_the_menu_lists_every_document() {
    for theme in [ThemeKind::Pro, ThemeKind::Studio] {
        let mut h = harness(14, theme);
        let s = strip(&h);
        assert!(s.tabs.len() > 1 && s.tabs.len() < 14, "{theme:?}: {} shown", s.tabs.len());
        assert!(!elided_on_strip(&h), "{theme:?}: no tab title is cut");
        // The last new document is active; then one that was behind the menu, then the first.
        for active in [13, 5, 0] {
            h.state_mut().session.set_active(active);
            h.run_steps(2);
            let pro = crate::theme::Tokens::get(&h.ctx).pro;
            let labels: Vec<String> =
                (0..14).map(|i| if pro { pro_tab_title(h.state(), i) } else { h.state().session.documents()[i].doc.name.clone() }).collect();
            h.get_by_label("More documents").click();
            h.run_steps(2);
            for (i, label) in labels.iter().enumerate() {
                // The menu bar's padding: an unchecked row keeps the check mark's room, so names line up.
                let (on, off) = (format!("✔ {label}"), format!("   {label}"));
                let (want, other) = if i == active { (&on, &off) } else { (&off, &on) };
                assert_eq!(h.query_all_by_label(want).count(), 1, "{theme:?}, active {active}: document {i} is in the menu as {want:?}");
                assert_eq!(h.query_all_by_label(other).count(), 0, "{theme:?}, active {active}: document {i} is not {other:?}");
            }
            h.key_press(egui::Key::Escape);
            h.run_steps(2);
            assert!(h.query_by_label(&format!("✔ {}", labels[active])).is_none(), "{theme:?}: the menu closed");
        }
    }
}

/// #8290: the » button sits at the strip's right end, as Photoshop's does, not right after the
/// last tab that fits (which left a gap of up to a tab's width after it), in both layouts: it is
/// in the same place however many tabs fit before it.
#[test]
fn the_overflow_button_is_pinned_to_the_right_end() {
    for theme in [ThemeKind::Pro, ThemeKind::Studio] {
        let at = |docs: usize| {
            let h = harness(docs, theme);
            let s = strip(&h);
            let button = h.get_by_label("More documents").rect();
            let last = s.tabs.iter().map(|(_, r)| r.right()).fold(f32::MIN, f32::max);
            assert!(button.left() >= last - 0.5, "{theme:?}: » {button:?} overlaps the last tab (ends at {last})");
            // At the end of the strip (the Studio row keeps a small inset).
            assert!(button.right() <= s.rect.right() + 0.5 && button.right() >= s.rect.right() - 10.0, "{theme:?}: » at {button:?} in {:?}", s.rect);
            (button, last)
        };
        let ((a, last_a), (b, last_b)) = (at(14), at(23));
        assert!((last_a - last_b).abs() > 1.0, "{theme:?}: the fits must end at different places to tell");
        assert_eq!(a, b, "{theme:?}: the » button doesn't follow the last tab");
    }
}
