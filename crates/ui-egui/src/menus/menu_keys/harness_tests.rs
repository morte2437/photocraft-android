//! The menu bar from the keyboard (#4988): Alt / F10 focus the bar, Alt+letter opens a menu,
//! letters run rows, and the shortcuts wait while the bar has the keyboard.
use crate::PhotocraftApp;
use crate::menu_keys::Keys;
use crate::menu_nav::{Nav, is_open};
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 64, "height": 48})).unwrap();
    app.sync_views();
    app
}

/// The menu bar alone, as `menu_nav_tests` drives it.
fn bar() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 720.0)).build_ui_state(
        |ui, app| {
            crate::menus::menu_bar(app, ui);
        },
        app(),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(3);
    h
}

fn key(h: &mut Harness<'static, PhotocraftApp>, k: egui::Key) {
    h.key_press(k);
    h.run_steps(3);
}

fn alt_key(h: &mut Harness<'static, PhotocraftApp>, k: egui::Key) {
    h.event_modifiers(egui::Event::Key { key: k, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::ALT }, egui::Modifiers::ALT);
    h.run_steps(3);
}

/// A tap on Alt: down, then up with nothing else pressed between.
fn alt_tap(h: &mut Harness<'static, PhotocraftApp>) {
    h.key_down(egui::Key::AltLeft);
    h.run_steps(2);
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
}

fn rows(h: &Harness<'static, PhotocraftApp>, level: usize) -> Vec<crate::menu_nav::Row> {
    Nav::current(&h.ctx).rows.get(level).cloned().unwrap_or_default()
}

#[test]
fn alt_focuses_the_bar_arrows_move_and_down_opens_the_lit_menu() {
    let mut h = bar();
    assert_eq!(Keys::current(&h.ctx).focus, None);
    alt_tap(&mut h);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0), "Alt puts the keyboard on File");
    assert!(crate::menu_keys::is_focused(&h.ctx), "the shortcuts wait for the bar");
    assert!(!is_open(&h.ctx), "no menu opens yet");
    key(&mut h, egui::Key::ArrowRight);
    assert_eq!(Keys::current(&h.ctx).focus, Some(1), "→ moves to Edit");
    key(&mut h, egui::Key::ArrowLeft);
    key(&mut h, egui::Key::ArrowLeft);
    assert_eq!(Keys::current(&h.ctx).focus, Some(9), "← wraps round to Help");
    key(&mut h, egui::Key::ArrowRight);
    key(&mut h, egui::Key::ArrowRight);
    assert_eq!(Keys::current(&h.ctx).focus, Some(1));
    key(&mut h, egui::Key::ArrowDown);
    h.run_steps(2);
    assert!(is_open(&h.ctx), "↓ opens the lit menu");
    assert!(h.query_by_label_contains("Undo").is_some(), "Edit is open");
    assert_eq!(Keys::current(&h.ctx).focus, None, "the focus moves into the menu");
    let first = rows(&h, 0).iter().position(|r| r.enabled).unwrap();
    assert_eq!(Nav::current(&h.ctx).highlighted(0), Some(first), "its first row is lit, as on Windows");
    assert!(Keys::current(&h.ctx).underline, "a menu opened from the keyboard shows its mnemonics");
    key(&mut h, egui::Key::Escape);
    assert!(!is_open(&h.ctx) && Keys::current(&h.ctx).focus.is_none(), "Esc leaves the menu and the bar");
}

/// Alt+wheel zooms; letting go of Alt after it is no tap, so Space (the hand tool) can't then
/// open File and run New….
#[test]
fn alt_with_a_wheel_scroll_or_a_lost_focus_is_no_tap() {
    let mut h = bar();
    h.key_down(egui::Key::AltLeft);
    h.run_steps(2);
    h.event_modifiers(
        egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Unknown,
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, -1.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::ALT,
        },
        egui::Modifiers::ALT,
    );
    h.run_steps(2);
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
    assert_eq!(Keys::current(&h.ctx).focus, None, "Alt+scroll leaves the bar alone");
    key(&mut h, egui::Key::Space);
    assert!(!is_open(&h.ctx), "Space opens no menu");
    h.key_down(egui::Key::AltLeft);
    h.run_steps(2);
    h.event(egui::Event::WindowFocused(false));
    h.event(egui::Event::WindowFocused(true));
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
    assert_eq!(Keys::current(&h.ctx).focus, None, "Alt that went to the desktop leaves the bar alone");
}

/// Alt's own auto-repeat (holding it after Alt+wheel) doesn't make the release a tap again, and
/// holding Alt alone long enough to repeat is still a tap.
#[test]
fn a_held_alt_repeating_after_the_wheel_is_no_tap() {
    let repeat = |h: &mut Harness<'static, PhotocraftApp>| {
        h.event_modifiers(
            egui::Event::Key { key: egui::Key::AltLeft, physical_key: None, pressed: true, repeat: true, modifiers: egui::Modifiers::ALT },
            egui::Modifiers::ALT,
        );
        h.run_steps(1);
    };
    let mut h = bar();
    h.key_down(egui::Key::AltLeft);
    h.run_steps(2);
    h.event_modifiers(
        egui::Event::MouseWheel {
            source: egui::MouseWheelSource::Wheel,
            unit: egui::MouseWheelUnit::Line,
            delta: egui::vec2(0.0, 1.0),
            modifiers: egui::Modifiers::ALT,
            phase: egui::TouchPhase::Move,
        },
        egui::Modifiers::ALT,
    );
    h.run_steps(2);
    for _ in 0..5 {
        repeat(&mut h);
    }
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
    assert_eq!(Keys::current(&h.ctx).focus, None, "Alt held on after the wheel: still no tap");
    key(&mut h, egui::Key::Space);
    assert!(!is_open(&h.ctx), "Space opens no menu");
    // Alt alone, held long enough to repeat, is still a tap.
    h.key_down(egui::Key::AltLeft);
    h.run_steps(1);
    for _ in 0..5 {
        repeat(&mut h);
    }
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0), "a long press of Alt alone focuses the bar");
}

/// A stray Alt tap (a second, quick one after Alt+scroll) focuses the bar; Space for the hand
/// tool then takes the keyboard off it instead of opening File, where held Space ran New….
#[test]
fn space_on_the_focused_bar_leaves_it_and_opens_no_menu() {
    let mut h = bar();
    alt_tap(&mut h);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0), "the tap focuses the bar");
    h.key_down(egui::Key::Space);
    h.run_steps(2);
    // Held Space repeats, which ran New… from the open File menu.
    h.event(egui::Event::Key { key: egui::Key::Space, physical_key: None, pressed: true, repeat: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    assert!(!is_open(&h.ctx), "Space opens no menu");
    assert!(h.state().ui.dialogs.is_empty(), "Space does not run File › New");
    assert_eq!(Keys::current(&h.ctx).focus, None, "Space takes the keyboard off the bar");
    h.key_up(egui::Key::Space);
    h.run_steps(3);
}

#[test]
fn a_second_alt_or_esc_or_a_click_takes_the_keyboard_off_the_bar() {
    let mut h = bar();
    alt_tap(&mut h);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0));
    alt_tap(&mut h);
    assert_eq!(Keys::current(&h.ctx).focus, None, "a second tap leaves");
    key(&mut h, egui::Key::F10);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0), "F10 focuses the bar too");
    key(&mut h, egui::Key::Escape);
    assert_eq!(Keys::current(&h.ctx).focus, None);
    assert!(!crate::menu_keys::is_focused(&h.ctx));
    key(&mut h, egui::Key::F10);
    let at = egui::pos2(600.0, 300.0);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
    h.run_steps(2);
    h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
    h.run_steps(3);
    assert_eq!(Keys::current(&h.ctx).focus, None, "a click anywhere leaves");
}

#[test]
fn alt_with_a_letter_opens_that_menu_and_enter_opens_the_lit_one() {
    let mut h = bar();
    alt_key(&mut h, egui::Key::E);
    h.run_steps(2);
    assert!(is_open(&h.ctx) && h.query_by_label_contains("Undo").is_some(), "Alt+E opens Edit");
    alt_key(&mut h, egui::Key::F);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Open…").is_some(), "Alt+F switches to File while a menu is open");
    key(&mut h, egui::Key::Escape);
    assert!(!is_open(&h.ctx));
    key(&mut h, egui::Key::F10);
    key(&mut h, egui::Key::Enter);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Open…").is_some(), "Enter opens the lit File menu");
    key(&mut h, egui::Key::Escape);
    key(&mut h, egui::Key::F10);
    key(&mut h, egui::Key::V);
    h.run_steps(2);
    assert!(is_open(&h.ctx) && h.query_by_label_contains("Zoom In").is_some(), "a letter on the focused bar opens that menu (V = View)");
}

#[test]
fn a_letter_in_an_open_menu_runs_the_row_with_that_mnemonic_or_opens_its_submenu() {
    let mut h = bar();
    alt_key(&mut h, egui::Key::V);
    h.run_steps(2);
    assert!(h.query_by_label_contains("Zoom In").is_some(), "View is open");
    let level = rows(&h, 0);
    let zoom_in = level.iter().find(|r| r.command.as_deref() == Some("view.zoomIn")).expect("Zoom In row");
    let letter = zoom_in.mnemonic.expect("Zoom In has a mnemonic");
    assert!(level.iter().filter(|r| r.mnemonic == Some(letter)).count() == 1, "mnemonics are unique within a level");
    let zoom = h.state().ui.views[0].zoom;
    key(&mut h, egui::Key::from_name(&letter.to_string()).unwrap());
    h.run_steps(3);
    assert!(h.state().ui.views[0].zoom > zoom, "the letter ran Zoom In ({letter})");
    assert!(!is_open(&h.ctx), "and closed the menu");
    // A submenu's letter opens it, with its first row lit.
    alt_key(&mut h, egui::Key::V);
    h.run_steps(2);
    let level = rows(&h, 0);
    let sub = level.iter().find(|r| r.command.is_none() && r.enabled && r.mnemonic.is_some()).expect("an enabled submenu with a mnemonic");
    let letter = sub.mnemonic.unwrap();
    key(&mut h, egui::Key::from_name(&letter.to_string()).unwrap());
    h.run_steps(3);
    assert!(!rows(&h, 1).is_empty(), "the submenu opened ({letter})");
    assert_eq!(Nav::current(&h.ctx).level, 1);
}

#[test]
fn mnemonics_are_underlined_only_with_alt_or_a_keyboard_opened_menu() {
    let mut h = bar();
    h.get_by_label("File").click();
    h.run_steps(4);
    assert!(is_open(&h.ctx));
    assert!(!Nav::current(&h.ctx).underline, "a menu opened with the mouse shows no underlines");
    h.key_down(egui::Key::AltLeft);
    h.run_steps(3);
    assert!(Nav::current(&h.ctx).underline, "while Alt is held they show");
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
    // The tap closed the menu (as on Windows); opening from the keyboard underlines.
    assert!(!is_open(&h.ctx), "an Alt tap closes an open menu");
    alt_key(&mut h, egui::Key::F);
    h.run_steps(2);
    assert!(is_open(&h.ctx) && Nav::current(&h.ctx).underline);
    assert!(rows(&h, 0).iter().any(|r| r.mnemonic.is_some()), "rows carry their letters");
}

#[test]
fn underlined_text_marks_the_mnemonic_once_and_keeps_the_prefix() {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let t = crate::menu_keys::mnemonic_text(ui, "✔ ", "Save As…", Some('A'), None);
        let egui::WidgetText::LayoutJob(job) = t else { panic!("a mnemonic makes a layout job") };
        assert_eq!(job.text, "✔ Save As…");
        let underlined: Vec<&str> =
            job.sections.iter().filter(|s| s.format.underline != egui::Stroke::NONE).map(|s| &job.text[s.byte_range.start.0..s.byte_range.end.0]).collect();
        assert_eq!(underlined, ["a"], "the first occurrence, case-insensitively");
        let t = crate::menu_keys::mnemonic_text(ui, "", "Edit", None, None);
        assert!(matches!(t, egui::WidgetText::RichText(_)), "without a mnemonic, plain text");
        let t = crate::menu_keys::mnemonic_text(ui, "", "Édition", Some('D'), None);
        let egui::WidgetText::LayoutJob(job) = t else { panic!() };
        assert_eq!(job.sections.len(), 3, "multi-byte text before the letter is kept whole");
    });
    out.textures_delta.clear();
}

/// The whole app: with the bar focused a letter opens a menu instead of picking a tool, and the
/// arrows don't nudge the layer.
#[test]
fn the_shortcuts_wait_while_the_keyboard_is_on_the_bar() {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = app();
        app.services.native_menu = None;
        app
    });
    h.run_steps(4);
    let tool = h.state().ui.tool;
    key(&mut h, egui::Key::F10);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0), "F10 focuses the bar");
    key(&mut h, egui::Key::E);
    h.run_steps(2);
    assert_eq!(h.state().ui.tool, tool, "E did not pick the Eraser");
    assert!(is_open(&h.ctx) && h.query_by_label_contains("Undo").is_some(), "E opened Edit");
    key(&mut h, egui::Key::Escape);
    h.run_steps(2);
    assert!(!is_open(&h.ctx));
    h.key_press(egui::Key::E);
    h.run_steps(3);
    assert_ne!(h.state().ui.tool, tool, "off the bar, E picks the Eraser again");
}

/// #5863: a label with no Latin letter (Chinese, Japanese…) shows its English label's letter in
/// parentheses, before a trailing ellipsis, and takes it as its mnemonic; labels with letters and
/// the top menus' Photoshop letters are as before, and the letters stay unique.
#[test]
fn labels_without_latin_letters_show_their_english_key() {
    let labels: Vec<String> = ["文件", "编辑", "Image", "图层"].map(String::from).to_vec();
    let english = ["File", "Edit", "Image", "Layer"];
    let preferred = [Some('F'), Some('E'), Some('I'), Some('L')];
    let (shown, keys) = crate::menu_keys::localized(&labels, &english, &preferred);
    assert_eq!(shown, ["文件(F)", "编辑(E)", "Image", "图层(L)"]);
    assert_eq!(keys, [Some('F'), Some('E'), Some('I'), Some('L')]);
    // Rows: the key goes before the ellipsis; a clash takes the next letter of the shown label.
    let labels: Vec<String> = ["新建…", "打开...", "关闭"].map(String::from).to_vec();
    let (shown, keys) = crate::menu_keys::localized(&labels, &["New…", "Open...", "Close"], &[]);
    assert_eq!(shown, ["新建(N)…", "打开(O)...", "关闭(C)"]);
    assert_eq!(keys, [Some('N'), Some('O'), Some('C')]);
    let (shown, keys) = crate::menu_keys::localized(&["关闭".to_string(), "复制".to_string()], &["Close", "Copy"], &[]);
    assert_eq!(shown, ["关闭(C)", "复制(O)"]);
    assert_eq!(keys, [Some('C'), Some('O')]);
    // A shown "(C)" keeps its key even when a label with letters of its own comes first.
    let (shown, keys) = crate::menu_keys::localized(&["Copy".to_string(), "关闭".to_string()], &["Open", "Close"], &[]);
    assert_eq!(shown, ["Copy", "关闭(C)"]);
    assert_eq!(keys, [Some('O'), Some('C')]);
    // An English UI is unchanged.
    let labels: Vec<String> = ["File", "Edit"].map(String::from).to_vec();
    assert_eq!(crate::menu_keys::localized(&labels, &["File", "Edit"], &[Some('F'), Some('E')]).0, labels);
}

/// The whole app: with the bar focused by an Alt tap, a command shortcut leaves the bar and runs
/// in the same press (#6573); the bar's own keys still drive it.
#[test]
fn a_command_shortcut_leaves_the_bar_and_runs() {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = app();
        app.services.native_menu = None;
        app
    });
    h.run_steps(4);
    h.state_mut().run("select.all", json!({})).unwrap();
    h.run_steps(2);
    alt_tap(&mut h);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0), "Alt puts the keyboard on File");
    key(&mut h, egui::Key::ArrowRight);
    assert_eq!(Keys::current(&h.ctx).focus, Some(1), "→ still moves along the bar");
    assert!(h.state().session.active().unwrap().doc.selection.is_some());
    h.key_press_modifiers(egui::Modifiers::COMMAND, egui::Key::D);
    h.run_steps(3);
    assert!(h.state().session.active().unwrap().doc.selection.is_none(), "⌘D deselects");
    assert_eq!(Keys::current(&h.ctx).focus, None, "and takes the keyboard off the bar");
    assert!(!crate::menu_keys::is_focused(&h.ctx));
    assert!(!is_open(&h.ctx), "no menu opened");
}

/// Checked, unchecked and plain rows (and submenu headers) start their label at the same x, in
/// any script: the checkmark column has a fixed width instead of padding characters (#5705).
#[test]
fn every_menu_row_starts_its_label_after_one_checkmark_column() {
    let ctx = egui::Context::default();
    let mut out = ctx.run_ui(Default::default(), |ui| {
        let start = |checked: bool, label: &str, mnemonic: Option<char>| {
            let galley = crate::menu_keys::menu_row_text(ui, checked, label, mnemonic).into_galley(
                ui,
                Some(egui::TextWrapMode::Extend),
                f32::INFINITY,
                egui::TextStyle::Button,
            );
            let first = label.chars().next().unwrap_or(' ');
            let row = galley.rows.first().expect("one row");
            let glyph = row.glyphs.iter().find(|g| g.chr == first).expect("the label's first glyph");
            assert!(galley.text().ends_with(label), "the label is shown whole: {:?}", galley.text());
            glyph.pos.x
        };
        let gutter = crate::menu_keys::check_gutter(ui);
        assert!(gutter > 0.0, "the column has room for the mark");
        for label in ["Lock Guides", "锁定参考线", "清除参考线"] {
            let checked = start(true, label, None);
            let unchecked = start(false, label, None);
            assert!((checked - unchecked).abs() < 0.01, "{label}: checked {checked} vs unchecked {unchecked}");
            assert!((unchecked - gutter).abs() < 1.0, "{label}: the label starts right after the column ({unchecked} vs {gutter})");
        }
        let underlined = start(false, "Clear Guides", Some('C'));
        let plain = start(false, "Clear Guides", None);
        assert!((underlined - plain).abs() < 0.01, "an underlined mnemonic doesn't move the label");
        let underlined_mid = start(true, "Lock Guides", Some('G'));
        assert!((underlined_mid - plain).abs() < 0.01, "nor a mnemonic after the first letter");
        let t = crate::menu_keys::menu_row_text(ui, true, "Rulers", None);
        assert_eq!(t.text(), "✔ Rulers", "a checked row keeps its mark (and its accessible label)");
    });
    out.textures_delta.clear();
}

/// The whole app (shortcuts, dialogs and the menu bar), for keys other handlers see first.
fn whole_app() -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(egui::vec2(1280.0, 800.0)).with_max_steps(16).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app()
    });
    h.run_steps(4);
    h
}

/// `key` pressed and released with `mods` held, as the platform reports a chord.
fn chord(h: &mut Harness<'static, PhotocraftApp>, key: egui::Key, mods: egui::Modifiers) {
    h.event_modifiers(egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: mods }, mods);
    h.event_modifiers(egui::Event::Key { key, physical_key: None, pressed: false, repeat: false, modifiers: mods }, mods);
    h.run_steps(2);
}

fn alt_down(h: &mut Harness<'static, PhotocraftApp>, mods: egui::Modifiers) {
    h.event_modifiers(egui::Event::Key { key: egui::Key::AltLeft, physical_key: None, pressed: true, repeat: false, modifiers: mods }, mods);
    h.run_steps(2);
}

fn alt_up(h: &mut Harness<'static, PhotocraftApp>) {
    h.event_modifiers(
        egui::Event::Key { key: egui::Key::AltLeft, physical_key: None, pressed: false, repeat: false, modifiers: egui::Modifiers::NONE },
        egui::Modifiers::NONE,
    );
    h.run_steps(3);
}

/// #7481: Alt held through a shortcut is no tap, even when the shortcut consumed its key before
/// the bar saw it. Ctrl+Alt+I opens Image Size; releasing Alt leaves the keyboard where it was,
/// so Enter confirms the dialog instead of opening File. A lone tap still puts it on the bar.
#[test]
fn alt_held_through_a_shortcut_does_not_put_the_keyboard_on_the_bar() {
    let mut h = whole_app();
    alt_down(&mut h, egui::Modifiers::ALT);
    chord(&mut h, egui::Key::I, egui::Modifiers::CTRL | egui::Modifiers::ALT | egui::Modifiers::COMMAND);
    alt_up(&mut h);
    assert_eq!(h.state().ui.dialogs.len(), 1, "Image Size opened");
    assert_eq!(Keys::current(&h.ctx).focus, None, "the bar did not take the keyboard");
    h.key_press(egui::Key::Enter);
    h.run_steps(4);
    assert!(!is_open(&h.ctx), "Enter opened no menu");
    assert!(h.state().ui.dialogs.is_empty(), "Enter confirmed Image Size");

    alt_down(&mut h, egui::Modifiers::ALT);
    alt_up(&mut h);
    assert_eq!(Keys::current(&h.ctx).focus, Some(0), "a lone tap still focuses the bar");
}

/// #8113: Alt pressed during a drag (Free Transform's scale from the centre) is a modifier for the
/// drag, not a tap: with the mouse button let go before or after Alt, the keyboard stays where
/// it was, so Enter commits the transform instead of opening File. A lone tap still works.
#[test]
fn alt_held_during_a_drag_is_no_tap() {
    let mut h = bar();
    let at = egui::pos2(640.0, 400.0);
    let button = |h: &mut Harness<'static, PhotocraftApp>, pressed: bool, mods: egui::Modifiers| {
        h.event_modifiers(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: mods }, mods);
        h.run_steps(2);
    };
    for mouse_up_first in [true, false] {
        h.hover_at(at);
        button(&mut h, true, egui::Modifiers::NONE);
        h.key_down(egui::Key::AltLeft);
        h.run_steps(2);
        if mouse_up_first {
            button(&mut h, false, egui::Modifiers::ALT);
            h.key_up(egui::Key::AltLeft);
        } else {
            h.key_up(egui::Key::AltLeft);
            h.run_steps(2);
            button(&mut h, false, egui::Modifiers::NONE);
        }
        h.run_steps(3);
        assert_eq!(Keys::current(&h.ctx).focus, None, "mouse up first: {mouse_up_first}");
        key(&mut h, egui::Key::Enter);
        assert!(!is_open(&h.ctx), "Enter opens no menu (mouse up first: {mouse_up_first})");
    }
    // The whole gesture within one frame: press, Alt down, Alt up, release.
    let (alt, none) = (egui::Modifiers::ALT, egui::Modifiers::NONE);
    h.event_modifiers(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: true, modifiers: none }, none);
    h.event_modifiers(egui::Event::Key { key: egui::Key::AltLeft, physical_key: None, pressed: true, repeat: false, modifiers: alt }, alt);
    h.event_modifiers(egui::Event::Key { key: egui::Key::AltLeft, physical_key: None, pressed: false, repeat: false, modifiers: none }, none);
    h.event_modifiers(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed: false, modifiers: none }, none);
    h.run_steps(3);
    assert_eq!(Keys::current(&h.ctx).focus, None, "one frame");
    // Two buttons: the right one let go while the left is still down. Alt pressed then is still
    // pressed during the left-button drag.
    let btn = |h: &mut Harness<'static, PhotocraftApp>, button: egui::PointerButton, pressed: bool| {
        h.event_modifiers(egui::Event::PointerButton { pos: at, button, pressed, modifiers: none }, none);
        h.run_steps(2);
    };
    btn(&mut h, egui::PointerButton::Primary, true);
    btn(&mut h, egui::PointerButton::Secondary, true);
    btn(&mut h, egui::PointerButton::Secondary, false);
    h.key_down(egui::Key::AltLeft);
    h.run_steps(2);
    h.key_up(egui::Key::AltLeft);
    h.run_steps(3);
    assert_eq!(Keys::current(&h.ctx).focus, None, "the left button is still down");
    btn(&mut h, egui::PointerButton::Primary, false);
    alt_tap(&mut h);
    assert!(Keys::current(&h.ctx).focus.is_some(), "a lone tap still focuses the bar");
}
