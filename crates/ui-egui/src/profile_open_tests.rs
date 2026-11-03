//! Files opened together whose embedded profile doesn't match the working space (File › Open with
//! several selected, Ask When Opening on). Checked in Photoshop 27.11: one Embedded Profile
//! Mismatch prompt at a time, naming its file, before that file opens; OK opens it and asks about
//! the next; Cancel opens neither it nor the files still to come.

use crate::{PhotocraftApp, Services};
use egui::{Modifiers, PointerButton, Pos2, vec2};
use egui_kittest::{
    Harness,
    kittest::{NodeT, Queryable},
};
use photocraft_doc::{Color, ColorMode, Document, SampleType, Size};
use photocraft_engine::Session;
use serde_json::Value;

/// The embedded-profile choice uses the current built-in profile's displayed name.
fn use_embedded() -> String {
    format!("Use the embedded profile ({})", photocraft_cms::Builtin::AdobeRgbCompat.description())
}
const CONVERT: &str = "Convert to the working space (sRGB IEC61966-2.1)";
const DISCARD: &str = "Discard the embedded profile (don't color manage)";

/// A PNG tagged with Adobe RGB, which doesn't match the sRGB working space.
fn adobe_png() -> Vec<u8> {
    let mut d = Document::with_background("t", Size::new(8, 8), ColorMode::Rgb, SampleType::U8, Color::rgba(0.2, 0.6, 0.9, 1.0));
    d.icc_profile = Some(photocraft_cms::Builtin::from_id("adobe-rgb-compat").unwrap().profile().to_bytes());
    photocraft_io::export(&d, "t.png", &Default::default()).unwrap().bytes
}

fn harness(background: bool) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(vec2(1400.0, 900.0)).with_max_steps(64).build_eframe(|cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(
            Session::new(),
            Services {
                import: Some(Box::new(|name, bytes, opts| {
                    photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
                })),
                ..Default::default()
            },
        )
    });
    h.state_mut().background_jobs = background;
    h.run_steps(3);
    h.state_mut().run("edit.colorSettings", serde_json::json!({"askOnMismatch": true, "policyRgb": "preserve"})).unwrap();
    h
}

/// Open a.png, b.png and c.png together and let every read finish.
fn open_three(h: &mut Harness<'static, PhotocraftApp>) {
    let png = adobe_png();
    for name in ["a.png", "b.png", "c.png"] {
        h.state_mut().open_file(&format!("/pics/{name}"), &png).unwrap();
    }
    settle(h);
}

/// Run frames until no open is still reading.
fn settle(h: &mut Harness<'static, PhotocraftApp>) {
    for _ in 0..200 {
        h.run_steps(2);
        if h.state().jobs.opens.is_empty() && h.state().session.jobs().is_empty() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    h.run_steps(6);
}

fn docs(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
    h.state().session.documents().iter().map(|d| d.doc.name.clone()).collect()
}

/// The one prompt up: its message.
fn prompt(h: &Harness<'static, PhotocraftApp>) -> String {
    let d = &h.state().ui.dialogs;
    assert_eq!(d.len(), 1, "one prompt at a time");
    d[0].fields.get("message").and_then(Value::as_str).unwrap_or_default().to_string()
}

/// A real click (pointer events, hit-tested like a mouse).
fn click(h: &mut Harness<'static, PhotocraftApp>, label: &str) {
    let at: Pos2 = h.query_all_by_label(label).find(|n| !n.accesskit_node().is_disabled()).map(|n| n.rect().center()).unwrap();
    h.event(egui::Event::PointerMoved(at));
    h.run_steps(1);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: at, button: PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(1);
    }
    settle(h);
}

fn answer(h: &mut Harness<'static, PhotocraftApp>, choice: &str) {
    click(h, choice);
    click(h, "OK");
}

fn profile(h: &Harness<'static, PhotocraftApp>, name: &str) -> Option<String> {
    let st = h.state().session.documents().iter().find(|d| d.doc.name == name).unwrap();
    st.doc.icc_profile.as_ref().map(|b| photocraft_engine::color_cmds::profile_from_bytes(b).unwrap().description.clone())
}

/// The file the prompt on screen asks about.
fn asked(h: &Harness<'static, PhotocraftApp>) -> &'static str {
    let msg = prompt(h);
    ["a.png", "b.png", "c.png", "d.png"]
        .into_iter()
        .find(|n| msg.starts_with(&format!("The document “{n}” has an embedded color profile that does not match the current RGB working space.")))
        .unwrap_or_else(|| panic!("the prompt names its file: {msg}"))
}

/// Files read on the spot ask in the order they were given. Read in the background, they ask in
/// the order their reads finish, which is the same order unless a later read overtakes an earlier
/// one: the tests hold either way.
fn in_order(background: bool, order: &[&str]) {
    if !background {
        assert_eq!(order, &["a.png", "b.png", "c.png"][..order.len()]);
    }
}

#[test]
fn files_opened_together_ask_one_at_a_time_before_each_opens() {
    for background in [false, true] {
        let mut h = harness(background);
        open_three(&mut h);
        let mut answered: Vec<&str> = Vec::new();
        for _ in 0..3 {
            assert_eq!(docs(&h), answered, "background {background}: a file opens only after its prompt");
            let name = asked(&h);
            assert!(!answered.contains(&name), "background {background}: {name} asked twice");
            answer(&mut h, &use_embedded());
            answered.push(name);
        }
        in_order(background, &answered);
        assert!(h.state().ui.dialogs.is_empty());
        assert_eq!(docs(&h), answered, "background {background}: each opened when it was answered");
        assert_eq!(h.state().session.active().map(|d| d.doc.name.as_str()), answered.last().copied());
    }
}

/// The prompt reads in the interface language: its sentence, the mode's name (French "RVB") and
/// the choices, which name the embedded and working profiles. The file's name goes in as it is,
/// braces and all.
#[test]
fn the_prompt_is_translated_and_still_names_its_file() {
    let png = adobe_png();
    let template = "The document “{name}” has an embedded color profile that does not match the current {mode} working space.";
    for code in ["en", "de", "fr", "ja"] {
        let services = Services {
            import: Some(Box::new(|name, bytes, opts| {
                photocraft_io::import_with_options(name, bytes, opts).map(|r| (r.document, r.warnings)).map_err(|e| e.to_string())
            })),
            ..Default::default()
        };
        let mut app = PhotocraftApp::new(Session::new(), services);
        app.run("edit.colorSettings", serde_json::json!({"askOnMismatch": true, "policyRgb": "preserve"})).unwrap();
        let lang = crate::i18n::Lang::from_code(code).unwrap();
        crate::i18n::with_language(lang, || app.open_file("/pics/shot {mode}.png", &png)).unwrap();
        assert!(app.session.documents().is_empty(), "{code}: held until answered");
        let fields = app.ui.dialogs[0].fields.clone();
        let message = fields.get("message").and_then(Value::as_str).unwrap().to_string();
        let expected = crate::i18n::fmt(crate::i18n::tr(lang, template), &[("mode", crate::i18n::tr(lang, "RGB")), ("name", "shot {mode}.png")]);
        assert_eq!(message, expected, "{code}: one line, the profiles are on the choices");
        if code == "en" {
            assert_eq!(message, "The document “shot {mode}.png” has an embedded color profile that does not match the current RGB working space.");
        }
        let name = |k: &str| fields.get(k).and_then(Value::as_str).unwrap().to_string();
        assert!(name("embedded").starts_with("Adobe RGB"), "{code}: {}", name("embedded"));
        assert!(name("working").starts_with("sRGB"), "{code}: {}", name("working"));
    }
}

/// #4559 with a held file: type in a font that isn't installed is reported with the import
/// warnings while the file still waits for its profile prompt, and shown once when it opens.
#[test]
fn a_held_file_still_reports_its_missing_fonts() {
    const MISSING: &str = "NoSuchFamily3471";
    let warning = format!("font '{MISSING}' is not installed; text may render using a fallback face");
    let mut s = Session::new();
    s.execute("file.new", serde_json::json!({"width": 40, "height": 20})).unwrap();
    s.execute("type.create", serde_json::json!({"text": "Hi", "x": 2, "y": 12, "size": 10, "font": MISSING})).unwrap();
    let mut doc = (*s.active().unwrap().doc).clone();
    doc.icc_profile = Some(photocraft_cms::Builtin::from_id("adobe-rgb-compat").unwrap().profile().to_bytes());
    let services =
        Services { import: Some(Box::new(move |_: &str, _: &[u8], _: &photocraft_io::ImportOptions| Ok((doc.clone(), Vec::new())))), ..Default::default() };
    let mut app = PhotocraftApp::new(Session::new(), services);
    app.run("edit.colorSettings", serde_json::json!({"askOnMismatch": true, "policyRgb": "preserve"})).unwrap();
    assert_eq!(app.open_bytes("poster.psd", b"x").unwrap(), vec![warning.clone()]);
    assert!(app.session.documents().is_empty() && app.ui.notices.is_empty(), "held: nothing shown yet");
    crate::open_prompt::answer(&mut app, Some("preserve")).unwrap();
    assert_eq!(app.session.documents().len(), 1);
    assert_eq!(app.ui.notices.len(), 1);
    assert_eq!(app.ui.notices[0].lines, vec![warning]);
}

#[test]
fn each_answer_applies_to_its_own_file() {
    let use_embedded = use_embedded();
    for background in [false, true] {
        let mut h = harness(background);
        open_three(&mut h);
        // Whichever file a prompt names gets that file's own answer.
        let mut answered: Vec<&str> = Vec::new();
        for _ in 0..3 {
            let name = asked(&h);
            answer(
                &mut h,
                match name {
                    "a.png" => CONVERT,
                    "b.png" => DISCARD,
                    _ => &use_embedded,
                },
            );
            answered.push(name);
        }
        in_order(background, &answered);
        assert_eq!(docs(&h), answered);
        assert_eq!(profile(&h, "a.png").as_deref(), Some("sRGB IEC61966-2.1"), "background {background}: a converted");
        assert_eq!(profile(&h, "b.png"), None, "background {background}: b discarded");
        assert!(profile(&h, "c.png").unwrap().starts_with("Adobe RGB"), "background {background}: c kept");
    }
}

#[test]
fn cancel_opens_neither_the_file_nor_the_ones_still_to_come() {
    for background in [false, true] {
        let mut h = harness(background);
        open_three(&mut h);
        let first = asked(&h);
        in_order(background, &[first]);
        answer(&mut h, &use_embedded());
        click(&mut h, "Cancel");
        settle(&mut h);
        assert!(h.state().ui.dialogs.is_empty(), "background {background}: no prompt left");
        assert_eq!(docs(&h), [first], "background {background}");
        // A later open asks again.
        h.state_mut().open_file("/pics/d.png", &adobe_png()).unwrap();
        settle(&mut h);
        assert_eq!(asked(&h), "d.png");
    }
}

/// "Remember this choice": one answer opens the files still waiting with it, saves it as the
/// Color Settings policy and stops asking, so the next mismatched file opens without a prompt.
#[test]
fn remember_answers_the_waiting_files_and_stops_asking() {
    for background in [false, true] {
        let mut h = harness(background);
        open_three(&mut h);
        click(&mut h, CONVERT);
        click(&mut h, "Remember this choice for future images");
        click(&mut h, "OK");
        assert!(h.state().ui.dialogs.is_empty(), "background {background}: no prompt is left");
        let mut opened = docs(&h);
        opened.sort();
        assert_eq!(opened, ["a.png", "b.png", "c.png"], "background {background}");
        for name in ["a.png", "b.png", "c.png"] {
            assert_eq!(profile(&h, name).as_deref(), Some("sRGB IEC61966-2.1"), "background {background}: {name} converted");
        }
        let settings = &h.state().session.color.settings;
        assert_eq!(settings.policy(photocraft_doc::ColorMode::Rgb), photocraft_engine::color_cmds::Policy::Convert);
        assert!(!settings.ask_on_mismatch, "background {background}");
        h.state_mut().open_file("/pics/d.png", &adobe_png()).unwrap();
        settle(&mut h);
        assert!(h.state().ui.dialogs.is_empty(), "background {background}: the next file doesn't ask");
        assert_eq!(profile(&h, "d.png").as_deref(), Some("sRGB IEC61966-2.1"), "background {background}");
    }
}

/// Without Remember, OK answers only the file asked about.
#[test]
fn without_remember_the_next_file_still_asks() {
    let mut h = harness(false);
    open_three(&mut h);
    answer(&mut h, CONVERT);
    assert_eq!(h.state().ui.dialogs.len(), 1);
    assert!(h.state().session.color.settings.ask_on_mismatch);
}
