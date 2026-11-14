//! #14042: a Web or Mobile preset in File › New turns Artboards on, as in Photoshop; a preset of
//! the Photo, Print, Art & Illustration or Film & Video tab turns that back off, and leaves a box
//! ticked by hand as it is. Recent and Saved cards keep restoring their own settings.

use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::new_doc_ui::{CATEGORIES, CLIPBOARD, Preset, apply_preset, apply_settings, command_params};
use crate::state::{DialogKind, UiState};

fn presets(tab: &str) -> &'static [Preset] {
    CATEGORIES.iter().find(|c| c.0 == tab).unwrap().1
}

/// The dialog's fields on `tab`, with Artboards ticked by hand or not.
fn on_tab(tab: &str, artboard: bool) -> Map<String, Value> {
    let mut f = UiState::new_document_fields();
    f.insert("__category".into(), json!(tab));
    if artboard {
        f.insert("artboard".into(), json!(true));
    }
    f
}

fn artboard(f: &Map<String, Value>) -> bool {
    f.get("artboard").and_then(Value::as_bool).unwrap_or(false)
}

#[test]
fn web_and_mobile_presets_turn_artboards_on() {
    for tab in ["Web", "Mobile"] {
        assert!(!presets(tab).is_empty(), "{tab}");
        for p in presets(tab) {
            for ticked in [false, true] {
                let mut f = on_tab(tab, ticked);
                apply_preset(&mut f, p, &Default::default());
                assert!(artboard(&f), "{tab} › {}", p.0);
                assert_eq!(command_params(&f)["artboard"], json!(true), "Create sends it: {tab} › {}", p.0);
            }
        }
    }
}

const OTHER_TABS: [&str; 4] = ["Photo", "Print", "Art & Illustration", "Film & Video"];

#[test]
fn presets_of_the_other_tabs_turn_a_web_or_mobile_presets_artboards_off() {
    for from in ["Web", "Mobile"] {
        for tab in OTHER_TABS {
            assert!(!presets(tab).is_empty(), "{tab}");
            for p in presets(tab) {
                let mut f = on_tab(from, false);
                apply_preset(&mut f, &presets(from)[0], &Default::default());
                assert!(artboard(&f));
                f.insert("__category".into(), json!(tab));
                apply_preset(&mut f, p, &Default::default());
                assert!(!artboard(&f), "{from} then {tab} › {}", p.0);
                assert!(command_params(&f).get("artboard").is_none(), "Create makes a plain document: {tab} › {}", p.0);
            }
        }
    }
}

/// Ticking Artboards by hand and then picking a preset of another tab wasn't measured: the box
/// keeps the user's choice, as before this change.
#[test]
fn presets_of_the_other_tabs_leave_a_box_ticked_by_hand() {
    for tab in OTHER_TABS {
        for p in presets(tab) {
            let mut f = on_tab(tab, true);
            apply_preset(&mut f, p, &Default::default());
            assert!(artboard(&f), "{tab} › {}", p.0);
            assert_eq!(command_params(&f)["artboard"], json!(true), "{tab} › {}", p.0);
            let mut f = on_tab(tab, false);
            apply_preset(&mut f, p, &Default::default());
            assert!(!artboard(&f), "{tab} › {}", p.0);
        }
        // Ticked by hand before a Web preset (which keeps it on): still the user's choice.
        let mut f = on_tab("Web", true);
        apply_preset(&mut f, &presets("Web")[0], &Default::default());
        f.insert("__category".into(), json!(tab));
        apply_preset(&mut f, &presets(tab)[0], &Default::default());
        assert!(artboard(&f), "Web then {tab}");
    }
}

/// Recent's built-in cards and the Clipboard preset weren't measured: they keep today's behaviour.
#[test]
fn recent_presets_and_the_clipboard_leave_artboards_as_it_is() {
    for ticked in [false, true] {
        for p in presets(CATEGORIES[0].0) {
            let mut f = on_tab(CATEGORIES[0].0, ticked);
            apply_preset(&mut f, p, &Default::default());
            assert_eq!(artboard(&f), ticked, "Recent › {}", p.0);
        }
        let mut f = on_tab(CATEGORIES[0].0, ticked);
        crate::new_doc_ui::set_clipboard(&mut f, 640, 360);
        assert_eq!(artboard(&f), ticked, "{CLIPBOARD}");
        // Applied without a tab (a script, or a preset no tab lists) changes nothing either.
        let mut f = on_tab("Web", ticked);
        apply_preset(&mut f, &("Not a Web preset", 640, 360, 72.0), &Default::default());
        assert_eq!(artboard(&f), ticked);
    }
}

#[test]
fn recent_cards_keep_restoring_their_own_settings() {
    let mut f = on_tab("Web", false);
    apply_preset(&mut f, &presets("Web")[0], &Default::default());
    let mut settings_from = f.clone();
    settings_from.remove("artboard");
    let settings = serde_json::from_value(crate::new_doc_ui::document_settings(&settings_from)).unwrap();
    let before = f.get("artboard").cloned();
    apply_settings(&mut f, &settings);
    assert_eq!(f.get("artboard").cloned(), before, "the stored settings hold no Artboards value");
}

mod dialog {
    use super::*;

    fn harness() -> Harness<'static, PhotocraftApp> {
        let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let mut h =
            Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app| crate::dialogs::show(app, ui.ctx()), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.state_mut().ui.open_dialog(DialogKind::NewDocument, UiState::new_document_fields());
        h.run_steps(3);
        h
    }

    fn fields(h: &Harness<'static, PhotocraftApp>) -> Map<String, Value> {
        h.state().ui.dialogs.first().map(|d| d.fields.clone()).expect("the dialog is open")
    }

    fn open_tab(h: &mut Harness<'static, PhotocraftApp>, tab: &str) {
        h.get_by_label(tab).click();
        h.run_steps(2);
    }

    /// Preset cards paint their titles directly, without an accessibility label.
    fn click_preset(h: &mut Harness<'static, PhotocraftApp>, title: &str) {
        fn find(shape: &egui::Shape, title: &str) -> Option<egui::Pos2> {
            match shape {
                egui::Shape::Text(text) if text.galley.job.text == title => Some(text.visual_bounding_rect().center()),
                egui::Shape::Vec(shapes) => shapes.iter().find_map(|shape| find(shape, title)),
                _ => None,
            }
        }
        let at = h.output().shapes.iter().find_map(|shape| find(&shape.shape, title)).expect("visible preset title");
        // Let a previous card click age past the double-click interval, so this one only selects.
        h.run_steps(60);
        h.hover_at(at);
        h.run_steps(1);
        h.drag_at(at);
        h.run_steps(1);
        h.drop_at(at);
        h.run_steps(2);
    }

    fn artboards_ticked(h: &Harness<'static, PhotocraftApp>) -> bool {
        h.get_by_label("Artboards").accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
    }

    #[test]
    fn a_web_preset_ticks_artboards_and_create_makes_an_artboard() {
        let mut h = harness();
        assert!(!artboards_ticked(&h));
        open_tab(&mut h, "Web");
        click_preset(&mut h, "Web Most Common");
        assert_eq!(fields(&h)["__preset"], "Web Most Common");
        assert!(artboards_ticked(&h), "the checkbox shows it");
        h.get_by_label("Create").click();
        h.run_steps(3);
        assert!(h.state().ui.dialogs.is_empty(), "the dialog closed");
        let doc = &h.state().session.active().expect("a new document").doc;
        assert_eq!((doc.size.width, doc.size.height), (1366, 768));
        assert_eq!(doc.artboards().len(), 1, "made as an artboard");
    }

    #[test]
    fn a_photo_preset_after_a_mobile_one_unticks_artboards() {
        let mut h = harness();
        open_tab(&mut h, "Mobile");
        click_preset(&mut h, presets("Mobile")[0].0);
        assert!(artboards_ticked(&h));
        open_tab(&mut h, "Photo");
        click_preset(&mut h, presets("Photo")[0].0);
        assert_eq!(h.state().ui.dialogs.len(), 1, "a single click only selects");
        assert!(!artboards_ticked(&h));
        h.get_by_label("Create").click();
        h.run_steps(3);
        let doc = &h.state().session.active().expect("a new document").doc;
        assert!(doc.artboards().is_empty(), "a plain document");
    }

    #[test]
    fn a_photo_preset_keeps_artboards_ticked_by_hand_after_a_web_one() {
        let mut h = harness();
        open_tab(&mut h, "Web");
        click_preset(&mut h, "Web Most Common");
        assert!(artboards_ticked(&h));
        // Untick and tick again by hand: the box now holds the user's choice.
        for _ in 0..2 {
            h.get_by_label("Artboards").click();
            h.run_steps(2);
        }
        assert!(artboards_ticked(&h));
        open_tab(&mut h, "Photo");
        click_preset(&mut h, presets("Photo")[0].0);
        assert!(artboards_ticked(&h), "a box ticked by hand stays ticked");
        h.get_by_label("Create").click();
        h.run_steps(3);
        let doc = &h.state().session.active().expect("a new document").doc;
        assert_eq!(doc.artboards().len(), 1, "made as an artboard");
    }
}
