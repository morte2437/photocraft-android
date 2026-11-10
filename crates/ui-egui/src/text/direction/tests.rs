//! Right-to-left shell layout: the direction rules, and where the mirrored chrome lands in a
//! running shell. The shell tests force the direction with `direction_scope`, so they don't
//! depend on a right-to-left language being selected.

use super::*;
use crate::PhotocraftApp;
use crate::i18n::{self, Lang};
use egui_kittest::{Harness, kittest::Queryable};
use photocraft_text::scripts;
use serde_json::json;

#[test]
fn right_to_left_scripts_by_language_code() {
    for code in ["ar", "AR", "ar-eg", "fa", "fa_IR", "he", "ur", "ckb", "ps", "yi"] {
        assert_eq!(i18n::dir_for_code(code), Dir::Rtl, "{code}");
    }
    // `ku` alone is Kurmanji, written in Latin script; Sorani is `ckb`.
    for code in ["en", "ja", "zh-hans", "pt-br", "ku", "arn", ""] {
        assert_eq!(i18n::dir_for_code(code), Dir::Ltr, "{code}");
    }
    // Arabic, Hebrew and Urdu are the right-to-left catalogs; every other registered language stays
    // left to right.
    for lang in Lang::all() {
        let want = if matches!(lang.code(), "ar" | "he" | "ur") { Dir::Rtl } else { Dir::Ltr };
        assert_eq!(lang.dir(), want, "{}", lang.code());
    }
}

#[test]
fn direction_follows_the_language_unless_forced() {
    let _lang = i18n::language_scope(Lang::EN);
    let forced = direction_scope(Dir::Rtl);
    assert!(is_rtl());
    {
        let _inner = direction_scope(Dir::Ltr);
        assert!(!is_rtl());
    }
    assert!(is_rtl(), "the inner scope restores the outer one");
    drop(forced);
    assert_eq!(FORCED.get(), None);
}

#[test]
fn sides_and_rows_mirror() {
    {
        let _ltr = direction_scope(Dir::Ltr);
        assert_eq!(row().main_dir(), egui::Direction::LeftToRight);
        assert_eq!(trailing().main_dir(), egui::Direction::RightToLeft);
        assert_eq!(below(), egui::RectAlign::BOTTOM_START);
    }
    let _rtl = direction_scope(Dir::Rtl);
    assert_eq!(row().main_dir(), egui::Direction::RightToLeft);
    assert_eq!(trailing().main_dir(), egui::Direction::LeftToRight);
    assert_eq!(below(), egui::RectAlign::BOTTOM_END);
}

#[test]
fn env_values_parse_strictly() {
    assert_eq!(parse_dir("rtl"), Some(Dir::Rtl));
    assert_eq!(parse_dir(" RTL "), Some(Dir::Rtl));
    assert_eq!(parse_dir("ltr"), Some(Dir::Ltr));
    assert_eq!(parse_dir(""), None);
    assert_eq!(parse_dir("ar"), None);
}

fn harness() -> Harness<'static, PhotocraftApp> {
    harness_with(Harness::builder())
}

fn harness_with(builder: egui_kittest::HarnessBuilder<PhotocraftApp>) -> Harness<'static, PhotocraftApp> {
    let mut h = builder.with_size(egui::vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, crate::theme::ThemeKind::ProMedium);
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).expect("new document");
        app
    });
    h.run_steps(4);
    h
}

/// Centre x of the widget labelled `label` (the first, when several are).
fn x_of(h: &Harness<'static, PhotocraftApp>, label: &str) -> f32 {
    h.query_all_by_label(label).next().unwrap_or_else(|| panic!("`{label}` drawn")).rect().center().x
}

/// Left x of the text `text` where it is painted (for widgets without an accessible label).
fn text_x(h: &Harness<'static, PhotocraftApp>, text: &str) -> f32 {
    fn find(shape: &egui::Shape, text: &str) -> Option<f32> {
        match shape {
            egui::Shape::Text(t) if t.galley.job.text == text => Some(t.pos.x),
            egui::Shape::Vec(shapes) => shapes.iter().find_map(|s| find(s, text)),
            _ => None,
        }
    }
    h.output().shapes.iter().find_map(|s| find(&s.shape, text)).unwrap_or_else(|| panic!("`{text}` painted"))
}

fn dock_x(h: &Harness<'static, PhotocraftApp>) -> f32 {
    crate::panel_docking::probe::panel_rect(&h.ctx, "layers").expect("Layers docked").center().x
}

#[test]
fn the_shell_mirrors_in_a_right_to_left_ui() {
    let _lang = i18n::language_scope(Lang::EN);
    for dir in [Dir::Ltr, Dir::Rtl] {
        let _dir = direction_scope(dir);
        let h = harness();
        let mid = 600.0;
        let (file, help, toolbar, dock, workspace) = (x_of(&h, "File"), x_of(&h, "Help"), x_of(&h, "Toolbar"), dock_x(&h), text_x(&h, "Essentials"));
        if dir == Dir::Rtl {
            assert!(file > help, "File starts the menu bar at the right: {file} vs {help}");
            assert!(toolbar > mid && dock < mid, "toolbox right, dock left: {toolbar}, {dock}");
            assert!(workspace < help, "the workspace switcher is at the far (left) end: {workspace}");
        } else {
            assert!(file < help, "{file} vs {help}");
            assert!(toolbar < mid && dock > mid, "{toolbar}, {dock}");
            assert!(workspace > help, "{workspace}");
        }
    }
}

/// Choosing a right-to-left catalog (not just forcing the direction) mirrors the shell, and the
/// shell's fonts draw its letters; English mirrors back. Panel interiors and dialogs stay
/// left-to-right for now (module docs), so only the dock side is checked.
#[test]
fn hebrew_and_urdu_mirror_the_shell_and_draw_their_letters() {
    let _lang = i18n::language_scope(Lang::EN);
    for (code, letter) in [("he", 'ש'), ("ur", 'ر')] {
        let lang = Lang::from_code(code).expect("registered language");
        // Builds with craft-fonts (CI: `CRAFT_FONTS_REQUIRED`) draw these letters from the first
        // frame and check them. Without it the host's fonts decide, so the harness allows tofu
        // and only the mirroring is checked.
        let fonts = craft_font_covers(letter);
        if !fonts {
            eprintln!("{code}: no craft-fonts face for {letter}; glyphs not checked (build with CRAFT_FONTS_DIR)");
        }
        let mut h = if fonts { harness() } else { harness_with(Harness::builder().allow_missing_glyphs()) };
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": code})).expect("choose language");
        h.run_steps(4);
        assert_eq!(i18n::current(), lang);
        assert!(is_rtl(), "{code} draws right-to-left");
        assert!(dock_x(&h) < 600.0, "{code}: the dock moves to the left");
        for label in ["File", "Help", "Layers"] {
            let text = i18n::tr(lang, label);
            assert!(!text.is_ascii(), "{code}: {label} is translated");
            assert!(!fonts || h.ctx.fonts_mut(|f| f.has_glyphs(&egui::FontId::proportional(12.0), text)), "{code}: {text} has glyphs");
        }
        h.state_mut().run("prefs.set", json!({"path": "interface.language", "value": "en"})).expect("back to English");
        h.run_steps(4);
        assert!(!is_rtl() && dock_x(&h) > 600.0, "{code} -> en: the shell mirrors back");
    }
}

/// Whether craft-fonts has a face for `letter`. Only those are served in the first frame (the
/// eager provider); installed fonts arrive a frame later through the lazy loader, which the
/// harness's strict missing-glyph policy doesn't wait for (`cjk_fonts` tests).
fn craft_font_covers(letter: char) -> bool {
    photocraft_text::craft_fonts::CRAFT_FONTS.iter().any(|f| !f.bytes.is_empty() && scripts::covering_face(f.bytes, letter).is_some())
}

#[test]
fn arrow_keys_step_through_the_menu_bar_in_screen_order() {
    let _lang = i18n::language_scope(Lang::EN);
    for (dir, towards_edit) in [(Dir::Ltr, egui::Key::ArrowRight), (Dir::Rtl, egui::Key::ArrowLeft)] {
        let _dir = direction_scope(dir);
        let mut h = harness();
        h.get_by_label("File").click();
        h.run_steps(4);
        assert!(h.query_by_label_contains("Undo").is_none(), "File is open, not Edit");
        // Edit is File's neighbour on screen: to its right, or to its left in a right-to-left UI.
        h.key_press(towards_edit);
        h.run_steps(4);
        assert!(h.query_by_label_contains("Undo").is_some(), "{dir:?}: {towards_edit:?} opens Edit");
    }
}

#[test]
fn menus_open_under_the_title_s_reading_start_edge() {
    let _lang = i18n::language_scope(Lang::EN);
    let _dir = direction_scope(Dir::Rtl);
    let mut h = harness();
    let edit = h.get_by_label("Edit").rect();
    h.get_by_label("Edit").click();
    h.run_steps(4);
    let undo = h.get_by_label_contains("Undo").rect();
    assert!(undo.right() <= edit.right() + 1.0 && undo.right() > edit.right() - 40.0, "the menu hangs left from Edit's right edge: {undo:?} {edit:?}");
}
