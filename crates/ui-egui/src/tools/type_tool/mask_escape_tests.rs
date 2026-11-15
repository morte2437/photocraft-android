//! #12070: Escape commits a Type Mask session the way ⌘↩ does, whatever "Use Escape to Commit"
//! says: the text outline becomes the selection in one history state and no layer is added. An
//! empty mask text and a composing input method still cancel, and the Type tool's Escape still
//! follows the preference. (A ⌘ transform drag's Escape arm stays first; masks have no transform
//! box yet, #12082.) Keys go through the app's shortcut handling, as on the canvas.

use egui_kittest::Harness;
use photocraft_doc::LayerId;
use photocraft_raster::Surface;
use serde_json::json;

use crate::PhotocraftApp;
use crate::state::Tool;

fn new_app(tool: Tool, use_esc_to_commit: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
    app.run("prefs.set", json!({"path": "type.useEscToCommit", "value": use_esc_to_commit})).unwrap();
    // Away from the text, so New, Add, Subtract and Intersect each leave a different selection.
    app.run("select.rect", json!({"x": 300, "y": 150, "width": 60, "height": 30})).unwrap();
    app.sync_views();
    app.ui.tool = tool;
    app
}

/// Start new text with a click and type `s`, as the canvas does.
fn type_new(app: &mut PhotocraftApp, s: &str) -> LayerId {
    super::pointer_up(app, [50.0, 100.0], [50.0, 100.0]);
    super::insert(app, s);
    LayerId(app.ui.text_edit.as_ref().unwrap().layer)
}

/// One frame of the app's shortcut handling for each of `events`.
fn send(app: PhotocraftApp, events: Vec<egui::Event>) -> PhotocraftApp {
    let mut h = Harness::builder().build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            // As `canvas_tests`' harness: the shortcuts wait for the app's fonts.
            if ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                crate::shortcuts::handle(app, &ctx);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    for event in events {
        h.event(event);
        h.run_steps(1);
    }
    h.into_state()
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }
}

pub(super) fn escape(app: PhotocraftApp) -> PhotocraftApp {
    send(app, vec![key(egui::Key::Escape, egui::Modifiers::NONE)])
}

/// The selection (compared pixel for pixel, shown by its bounds), the history (oldest first) and
/// the layer count.
#[derive(PartialEq)]
struct Snapshot {
    selection: Option<Surface>,
    history: Vec<String>,
    layers: usize,
}

impl std::fmt::Debug for Snapshot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let bounds = self.selection.as_ref().map(Surface::content_bounds);
        write!(f, "selection {bounds:?}, history {:?}, {} layers", self.history, self.layers)
    }
}

fn snapshot(app: &PhotocraftApp) -> Snapshot {
    let st = app.session.active().unwrap();
    Snapshot { selection: st.doc.selection.clone(), history: st.history.entries(), layers: st.doc.layers.len() }
}

#[test]
fn escape_commits_a_type_mask_as_command_return_does() {
    for tool in [Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        for use_esc_to_commit in [true, false] {
            let what = format!("{tool:?}, useEscToCommit={use_esc_to_commit}");
            let session = || {
                let mut app = new_app(tool, use_esc_to_commit);
                let before = snapshot(&app);
                type_new(&mut app, "AB");
                (app, before)
            };
            let (app, before) = session();
            let escaped = escape(app);
            let (app, _) = session();
            let returned = send(app, vec![key(egui::Key::Enter, egui::Modifiers::COMMAND)]);
            assert!(escaped.ui.text_edit.is_none(), "{what}: the session ends");
            let after = snapshot(&escaped);
            let bounds = after.selection.as_ref().map(Surface::content_bounds);
            assert!(bounds.is_some_and(|r| r.x1 < 300), "{what}: the text outline alone is the selection: {bounds:?}");
            assert_eq!(after.history.len(), before.history.len() + 1, "{what}: one history state");
            assert_eq!(after.layers, before.layers, "{what}: no layer is added");
            assert_eq!(after, snapshot(&returned), "{what}: the same as ⌘↩");
        }
    }
}

#[test]
fn escape_with_an_empty_mask_text_changes_nothing() {
    for tool in [Tool::HorizontalTypeMask, Tool::VerticalTypeMask] {
        for use_esc_to_commit in [true, false] {
            let what = format!("{tool:?}, useEscToCommit={use_esc_to_commit}");
            let mut app = new_app(tool, use_esc_to_commit);
            let before = snapshot(&app);
            type_new(&mut app, "");
            let app = escape(app);
            assert!(app.ui.text_edit.is_none(), "{what}");
            assert_eq!(snapshot(&app), before, "{what}");
        }
    }
}

#[test]
fn escape_while_an_input_method_composes_still_cancels_a_type_mask() {
    let preedit = || egui::Event::Ime(egui::ImeEvent::Preedit { text: "にほ".into(), active_range_chars: None });
    let session = || {
        let mut app = new_app(Tool::HorizontalTypeMask, true);
        let before = snapshot(&app);
        type_new(&mut app, "AB");
        (app, before)
    };
    let (app, _) = session();
    let app = send(app, vec![preedit()]);
    assert!(app.ui.text_edit.as_ref().is_some_and(|ed| ed.preedit.is_some()), "composing");
    let (app, before) = session();
    let app = send(app, vec![preedit(), key(egui::Key::Escape, egui::Modifiers::NONE)]);
    assert!(app.ui.text_edit.is_none());
    assert_eq!(snapshot(&app), before);
}

#[test]
fn the_type_tools_escape_still_follows_the_preference() {
    for (use_esc_to_commit, kept) in [(true, true), (false, false)] {
        let mut app = new_app(Tool::Type, use_esc_to_commit);
        let id = type_new(&mut app, "AB");
        let app = escape(app);
        assert!(app.ui.text_edit.is_none());
        let layer = app.session.active().unwrap().doc.layer(id).is_some();
        assert_eq!(layer, kept, "useEscToCommit={use_esc_to_commit}: committed, or cancelled and the new layer gone");
    }
}
