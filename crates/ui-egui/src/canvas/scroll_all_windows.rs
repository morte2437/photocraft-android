//! Hand tool › Scroll All Windows.
//!
//! With the option on, or with Shift held while dragging with the Hand, a drag in one document
//! view scrolls every other view that is shown: the main window's document, or the documents in its
//! Window › Arrange tiles, and the document windows of Float in Window and New Window for Document.
//! After each step of the drag, each of those views moves by the same distance in document pixels
//! as the dragged view, as the reference application does: a view at 100 % moves twice the screen
//! distance of one at 50 %. Each view keeps its own zoom and rotation. Documents in hidden tabs
//! keep their views.

use crate::PhotocraftApp;
use crate::state::View;

/// The view a Hand drag moves itself: a document's view in the main window, or a document window.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Main(usize),
    Window(u64),
}

/// The view that the canvas of document `idx` being drawn shows: the document window
/// `canvas::extra_windows` is drawing, or else the document's view in the main window.
pub fn source(app: &PhotocraftApp, idx: usize) -> Source {
    app.drawing_window.map_or(Source::Main(idx), Source::Window)
}

/// Whether a Hand drag scrolls every shown view: Scroll All Windows is on, or Shift is held.
pub fn active(app: &PhotocraftApp, shift: bool) -> bool {
    app.ui.tool_options.hand_scroll_all || shift
}

/// Store `scroll_all` under `id` while the Hand is dragging, and return the stored choice: a glide
/// after the drag keeps the choice the drag made.
pub fn glide_choice(ctx: &egui::Context, id: egui::Id, dragging: bool, scroll_all: bool) -> bool {
    if dragging {
        ctx.data_mut(|m| m.insert_temp(id, scroll_all));
    }
    ctx.data(|m| m.get_temp::<bool>(id)).unwrap_or(false)
}

/// Canvas geometry used to clamp a Hand drag when Overscroll is off.
pub struct ClampGeometry {
    pub size: egui::Vec2,
    pub ppp: f32,
    pub aspect: f32,
}

/// Move the other shown views after one step of a Hand drag or glide, which moved the dragged view's
/// center from `before` to `view.center`. With Overscroll off, both centers are taken where the
/// dragged view is clamped to, so a view held at its pan limit moves the others no further.
pub fn follow_step(app: &mut PhotocraftApp, enabled: bool, source: Source, before: [f32; 2], view: &View, geometry: ClampGeometry) {
    if !enabled {
        return;
    }
    let mut from = View { center: before, ..view.clone() };
    let mut to = view.clone();
    if !app.session.prefs().tools.overscroll {
        crate::scrollbars::clamp_view_with_aspect(&mut from, geometry.size, geometry.ppp, geometry.aspect);
        crate::scrollbars::clamp_view_with_aspect(&mut to, geometry.size, geometry.ppp, geometry.aspect);
    }
    follow(app, source, [to.center[0] - from.center[0], to.center[1] - from.center[1]]);
}

/// Whether the main window shows the Home screen or an open-file card instead of a document
/// canvas, as `tabs::documents` draws it.
fn main_canvas_hidden(app: &mut PhotocraftApp) -> bool {
    let n = app.session.documents().len();
    let opening = !app.jobs.opens.is_empty();
    (!opening && app.ui.chrome.shows_home(n, app.session.prefs().general.auto_show_home_screen)) || app.jobs.focus.is_some()
}

/// Move every shown view except `source` by `delta`, a distance in document pixels: each view's
/// center moves by the same number of its own document's pixels.
pub fn follow(app: &mut PhotocraftApp, source: Source, delta: [f32; 2]) {
    if !(delta[0].is_finite() && delta[1].is_finite()) || delta == [0.0, 0.0] {
        return;
    }
    let place = |v: &mut View| {
        let c = [v.center[0] + delta[0], v.center[1] + delta[1]];
        if c[0].is_finite() && c[1].is_finite() {
            v.zoom_animating = false;
            v.zoom_anchor = None;
            v.fit_pending = false;
            v.fill_pending = false;
            v.center = c;
        }
    };
    if !main_canvas_hidden(app) {
        // The main window shows the arranged tiles, or else a docked document alone.
        let shown: Vec<usize> = match crate::canvas::arranged_cells(app, app.last_canvas_rect) {
            Some(cells) => cells.into_iter().map(|(d, _)| d).collect(),
            None => super::tabs::main_canvas_document(app).into_iter().collect(),
        };
        for d in shown {
            if source != Source::Main(d)
                && let Some(v) = app.ui.views.get_mut(d)
            {
                place(v);
            }
        }
    }
    for w in app.ui.windows.iter_mut().filter(|w| w.open && source != Source::Window(w.id)) {
        place(&mut w.view);
    }
}

#[cfg(test)]
mod tests {
    use egui::{Pos2, Rect, vec2};
    use serde_json::json;

    use super::*;
    use crate::state::{DocWindow, Tool};

    fn view(zoom: f32, center: [f32; 2], rotation: f32) -> View {
        View { zoom, center, rotation, fit_pending: false, fill_pending: false, ..View::default() }
    }

    /// Three documents of different sizes, the first two tiled side by side and the third in a
    /// hidden tab.
    fn app_with_tiles() -> PhotocraftApp {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for (w, h) in [(400, 300), (800, 600), (200, 100)] {
            app.run("file.new", json!({"width": w, "height": h})).unwrap();
        }
        app.sync_views();
        app.session.set_active(0);
        app.ui.view.arrange = "twoUpVertical".into();
        app.arrange_start = 0;
        app.ui.views[0] = view(1.0, [200.0, 150.0], 0.0);
        app.ui.views[1] = view(2.0, [400.0, 300.0], 0.0);
        app.ui.views[2] = view(1.0, [100.0, 50.0], 0.0);
        app
    }

    fn near(a: [f32; 2], b: [f32; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3
    }

    #[test]
    fn a_drag_moves_the_other_shown_views_by_the_same_document_pixel_distance() {
        let mut app = app_with_tiles();
        app.ui.windows.push(DocWindow {
            id: 7,
            document: 2,
            view: view(0.5, [100.0, 50.0], 90.0),
            open: true,
            floating: false,
            focused: false,
            position: None,
        });
        // The dragged 400 × 300 view's center moved 50 document pixels left and 20 down.
        follow(&mut app, Source::Main(0), [-50.0, 20.0]);
        assert_eq!(app.ui.views[0].center, [200.0, 150.0], "the dragged view's canvas stores its own view");
        // The 800 × 600 tile at 200 % moves by the same document pixels, not to the same fraction.
        assert!(near(app.ui.views[1].center, [350.0, 320.0]), "{:?}", app.ui.views[1].center);
        assert_eq!(app.ui.views[1].zoom, 2.0, "each view keeps its zoom");
        // The document window shows the 200 × 100 document at 50 %, turned 90°.
        let w = &app.ui.windows[0].view;
        assert!(near(w.center, [50.0, 70.0]), "{:?}", w.center);
        assert_eq!((w.zoom, w.rotation), (0.5, 90.0), "and its rotation");
        // The third document's main view is in a tab hidden behind the tiles: it stays.
        assert_eq!(app.ui.views[2].center, [100.0, 50.0]);
    }

    #[test]
    fn a_drag_in_a_document_window_scrolls_the_main_window_and_skips_itself() {
        let mut app = app_with_tiles();
        app.ui.view.arrange = String::new();
        app.ui.windows.push(DocWindow {
            id: 3,
            document: 1,
            view: view(1.0, [400.0, 300.0], 0.0),
            open: true,
            floating: false,
            focused: false,
            position: None,
        });
        app.ui.windows.push(DocWindow {
            id: 4,
            document: 2,
            view: view(1.0, [100.0, 50.0], 0.0),
            open: false,
            floating: false,
            focused: false,
            position: None,
        });
        // The window's 800 × 600 view moved its center 100 document pixels right and 75 up.
        follow(&mut app, Source::Window(3), [100.0, -75.0]);
        assert_eq!(app.ui.windows[0].view.center, [400.0, 300.0], "the dragged window's canvas stores its own view");
        // Tabs: the main window shows the active 400 × 300 document only.
        assert!(near(app.ui.views[0].center, [300.0, 75.0]), "{:?}", app.ui.views[0].center);
        assert_eq!(app.ui.views[1].center, [400.0, 300.0], "a hidden tab stays");
        assert_eq!(app.ui.windows[1].view.center, [100.0, 50.0], "a closed window stays");
    }

    #[test]
    fn a_drag_in_a_floated_document_scrolls_the_visible_docked_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("file.new", json!({"width": 800, "height": 600})).unwrap();
        app.sync_views();
        app.session.set_active(0);
        app.ui.views[0] = view(1.0, [200.0, 150.0], 0.0);
        app.ui.views[1] = view(1.0, [400.0, 300.0], 0.0);
        crate::menus::invoke(&mut app, &egui::Context::default(), "window.arrange.floatInWindow", json!({"document": 0})).unwrap();
        assert!(app.ui.windows[0].floating);
        assert_eq!(app.session.active_index(), Some(0));
        assert_eq!(super::super::main_canvas_document(&app), Some(1));
        let window_id = app.ui.windows[0].id;

        // The floated 400 × 300 view moved 100 document pixels left and 90 up.
        follow(&mut app, Source::Window(window_id), [-100.0, -90.0]);
        assert!(near(app.ui.views[1].center, [300.0, 210.0]), "{:?}", app.ui.views[1].center);
        assert_eq!(app.ui.views[0].center, [200.0, 150.0], "the floated document's hidden main view stays");
    }

    #[test]
    fn a_drag_in_a_document_window_skips_the_main_view_behind_home_or_an_open_card() {
        for home in [true, false] {
            let mut app = app_with_tiles();
            app.ui.view.arrange.clear();
            app.ui.windows.push(DocWindow {
                id: 3,
                document: 1,
                view: view(1.0, [400.0, 300.0], 0.0),
                open: true,
                floating: false,
                focused: false,
                position: None,
            });
            app.ui.windows.push(DocWindow {
                id: 4,
                document: 2,
                view: view(1.0, [100.0, 50.0], 0.0),
                open: true,
                floating: false,
                focused: false,
                position: None,
            });
            if home {
                app.ui.chrome.home = Some(app.session.documents().len());
            } else {
                let job = photocraft_engine::jobs::JobId(9);
                app.jobs.opens.push(crate::jobs_ui::OpenTab { job, name: "opening.psd".into(), path: None, slot: None, order: None });
                app.jobs.focus = Some(job);
            }

            follow(&mut app, Source::Window(3), [-30.0, -10.0]);
            assert_eq!(app.ui.views[0].center, [200.0, 150.0], "the main canvas is hidden when home={home}");
            assert!(near(app.ui.windows[1].view.center, [70.0, 40.0]), "another document window follows when home={home}");
        }
    }

    #[test]
    fn the_source_is_the_document_window_being_drawn() {
        let mut app = app_with_tiles();
        assert_eq!(source(&app, 1), Source::Main(1));
        app.drawing_window = Some(9);
        assert_eq!(source(&app, 1), Source::Window(9));
    }

    /// A Hand drag with `option` and the Shift key as given, on the main canvas of the 2000 × 1500
    /// document 0 or (`in_window`) on the canvas of a window showing the 1000 × 750 document 1, as
    /// `canvas::extra_windows` draws it. Both views start at their document's center, the main view
    /// at 100 % and the window at 50 %.
    /// Returns the dragged view's center and the stored centers of document 0's main view and of
    /// the window's view.
    fn drag_on_canvas(option: bool, shift: bool, in_window: bool) -> ([f32; 2], [f32; 2], [f32; 2]) {
        drag_on_canvas_with(option, shift, false, in_window, false)
    }

    /// As `drag_on_canvas`, with Shift held in Window › Modifier Keys when `sticky_shift` is set,
    /// and with a dialog open over the canvas when `dialog` is set.
    fn drag_on_canvas_with(option: bool, shift: bool, sticky_shift: bool, in_window: bool, dialog: bool) -> ([f32; 2], [f32; 2], [f32; 2]) {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 2000, "height": 1500})).unwrap();
        app.run("file.new", json!({"width": 1000, "height": 750})).unwrap();
        app.run("prefs.set", json!({"path": "tools.enableFlickPanning", "value": false})).unwrap();
        app.sync_views();
        app.session.set_active(0);
        app.ui.tool = Tool::Hand;
        app.ui.tool_options.hand_scroll_all = option;
        app.ui.shell.sticky_shift = sticky_shift;
        if dialog {
            app.ui.open_dialog(crate::state::DialogKind::About, Default::default());
        }
        app.ui.views[0] = view(1.0, [1000.0, 750.0], 0.0);
        app.ui.windows.push(DocWindow {
            id: 5,
            document: 1,
            view: view(0.5, [500.0, 375.0], 0.0),
            open: true,
            floating: false,
            focused: false,
            position: None,
        });
        let mut v = if in_window { view(0.5, [500.0, 375.0], 0.0) } else { view(1.0, [1000.0, 750.0], 0.0) };
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let modifiers = if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE };
        let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                // The open dialog's modal layer leaves the canvas widget inert (`dialogs::pan_delta`).
                if dialog {
                    ui.disable();
                }
                if in_window {
                    app.drawing_window = Some(5);
                    v = crate::canvas::canvas_view(app, ui, 1, rect, v.clone(), false);
                    app.drawing_window = None;
                } else {
                    v = crate::canvas::canvas_view(app, ui, 0, rect, v.clone(), true);
                }
            });
            out.textures_delta.clear();
        };
        let start = Pos2::new(200.0, 150.0);
        let press = |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        frame(&mut app, vec![egui::Event::ModifiersChanged(modifiers), egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![press(start, true)]);
        let end = start + vec2(-60.0, 30.0);
        for i in 1..=6 {
            frame(&mut app, vec![egui::Event::PointerMoved(start + (end - start) * (i as f32 / 6.0))]);
        }
        frame(&mut app, vec![press(end, false)]);
        (v.center, app.ui.views[0].center, app.ui.windows[0].view.center)
    }

    #[test]
    fn the_option_or_shift_makes_a_hand_drag_scroll_the_document_windows_too() {
        let (dragged, _, window) = drag_on_canvas(false, false, false);
        assert!(near(dragged, [1060.0, 720.0]), "the dragged view follows the pointer: {dragged:?}");
        assert_eq!(window, [500.0, 375.0], "off: the window stays");
        // The same 60 and -30 document pixels in the window at 50 %: half the screen distance.
        let moved = [560.0, 345.0];
        let (_, _, window) = drag_on_canvas(true, false, false);
        assert!(near(window, moved), "on: it moves by the same document pixels: {window:?}");
        let (_, _, window) = drag_on_canvas(false, true, false);
        assert!(near(window, moved), "Shift turns it on for the drag: {window:?}");
    }

    #[test]
    fn shift_held_in_the_modifier_keys_panel_makes_a_hand_drag_scroll_the_document_windows_too() {
        let (dragged, _, window) = drag_on_canvas_with(false, false, true, false, false);
        assert!(near(dragged, [1060.0, 720.0]), "the dragged view follows the pointer: {dragged:?}");
        assert!(near(window, [560.0, 345.0]), "sticky Shift turns it on for the drag: {window:?}");
    }

    #[test]
    fn a_hand_drag_under_an_open_dialog_scrolls_the_document_windows_too() {
        let (dragged, _, window) = drag_on_canvas_with(false, false, false, false, true);
        assert!(near(dragged, [1060.0, 720.0]), "the dragged view pans under the dialog: {dragged:?}");
        assert_eq!(window, [500.0, 375.0], "off: the window stays");
        let (_, _, window) = drag_on_canvas_with(true, false, false, false, true);
        assert!(near(window, [560.0, 345.0]), "on: it moves by the same document pixels: {window:?}");
        let (_, _, window) = drag_on_canvas_with(false, true, false, false, true);
        assert!(near(window, [560.0, 345.0]), "Shift turns it on for the drag: {window:?}");
    }

    #[test]
    fn a_hand_drag_in_a_document_window_scrolls_the_main_window_once() {
        let (dragged, main, window) = drag_on_canvas(false, false, true);
        // At 50 % the window's center moves twice the screen distance in document pixels.
        assert!(near(dragged, [620.0, 315.0]), "the dragged window follows the pointer: {dragged:?}");
        assert_eq!((main, window), ([1000.0, 750.0], [500.0, 375.0]), "off: nothing else moves");
        // The window's own canvas moved its view (`dragged`), and `follow` leaves its stored view
        // alone: `extra_windows` stores the canvas's view, so the window moves once.
        let (dragged, main, window) = drag_on_canvas(true, false, true);
        assert!(near(dragged, [620.0, 315.0]), "{dragged:?}");
        // The main view at 100 % moves the same 120 and -60 document pixels: twice the screen distance.
        assert!(near(main, [1120.0, 690.0]), "on: the main window moves by the same document pixels: {main:?}");
        assert_eq!(window, [500.0, 375.0]);
    }

    #[test]
    fn with_overscroll_off_the_other_windows_follow_the_clamped_center() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 2000, "height": 1500})).unwrap();
        app.run("file.new", json!({"width": 1000, "height": 750})).unwrap();
        app.run("prefs.set", json!({"values": {"tools.enableFlickPanning": false, "tools.overscroll": false}})).unwrap();
        app.sync_views();
        app.session.set_active(0);
        app.ui.tool = Tool::Hand;
        app.ui.tool_options.hand_scroll_all = true;
        app.ui.windows.push(DocWindow {
            id: 5,
            document: 1,
            view: view(4.0, [500.0, 375.0], 0.0),
            open: true,
            floating: false,
            focused: false,
            position: None,
        });
        // A 400 × 300 canvas at 100 % keeps the center 200 px or more inside the left edge.
        let mut v = view(1.0, [220.0, 750.0], 0.0);
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                v = crate::canvas::canvas_view(app, ui, 0, rect, v.clone(), true);
            });
            out.textures_delta.clear();
        };
        let start = Pos2::new(200.0, 150.0);
        let press =
            |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        frame(&mut app, vec![egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![press(start, true)]);
        let end = start + vec2(60.0, 0.0);
        for i in 1..=6 {
            frame(&mut app, vec![egui::Event::PointerMoved(start + (end - start) * (i as f32 / 6.0))]);
        }
        frame(&mut app, vec![press(end, false)]);
        frame(&mut app, vec![]);
        assert!(near(v.center, [200.0, 750.0]), "the dragged view stops at its limit: {:?}", v.center);
        // The dragged view moved 20 document pixels before its limit held it; the window moves the
        // same 20, not the 60 of the pointer, though at 400 % it has room to go further left.
        let window = app.ui.windows[0].view.center;
        assert!(near(window, [480.0, 375.0]), "{window:?}");
    }

    #[test]
    fn a_flick_panning_glide_moves_the_other_windows_too() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 2000, "height": 1500})).unwrap();
        app.run("file.new", json!({"width": 1000, "height": 750})).unwrap();
        app.run("prefs.set", json!({"path": "tools.enableFlickPanning", "value": true})).unwrap();
        app.sync_views();
        app.session.set_active(0);
        app.ui.tool = Tool::Hand;
        app.ui.windows.push(DocWindow {
            id: 5,
            document: 1,
            view: view(1.0, [500.0, 375.0], 0.0),
            open: true,
            floating: false,
            focused: false,
            position: None,
        });
        let mut v = view(1.0, [1000.0, 750.0], 0.0);
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(400.0, 300.0));
        let mut frame = |app: &mut PhotocraftApp, events: Vec<egui::Event>| -> [f32; 2] {
            let input = egui::RawInput { screen_rect: Some(rect), events, ..Default::default() };
            let mut out = ctx.run_ui(input, |ui| {
                v = crate::canvas::canvas_view(app, ui, 0, rect, v.clone(), true);
            });
            out.textures_delta.clear();
            v.center
        };
        let start = Pos2::new(200.0, 150.0);
        let press =
            |pos: Pos2, pressed: bool| egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        // Shift turns Scroll All Windows on for the drag, and is let go with the button.
        frame(&mut app, vec![egui::Event::ModifiersChanged(egui::Modifiers::SHIFT), egui::Event::PointerMoved(start)]);
        frame(&mut app, vec![press(start, true)]);
        let mut end = start;
        for i in 1..=8 {
            end = start + vec2(-20.0 * i as f32, 0.0);
            frame(&mut app, vec![egui::Event::PointerMoved(end)]);
        }
        let released = frame(&mut app, vec![press(end, false), egui::Event::ModifiersChanged(egui::Modifiers::NONE)]);
        // On each frame of the glide the window has moved by the same document pixels as the
        // dragged view, though its document is half the size.
        let mut glided = released;
        for _ in 0..5 {
            glided = frame(&mut app, vec![]);
            let same = [glided[0] - 500.0, glided[1] - 375.0];
            assert!(near(app.ui.windows[0].view.center, same), "{:?} {same:?}", app.ui.windows[0].view.center);
        }
        assert!(glided[0] > released[0], "the dragged view glides on: {released:?} {glided:?}");
    }

    #[test]
    fn the_hand_tool_options_bar_shows_scroll_all_windows() {
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        app.ui.tool = Tool::Hand;
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 60.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                crate::panels::options_bar(app, ui);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.run_steps(4);
        assert!(!h.state().ui.tool_options.hand_scroll_all, "off by default");
        h.get_by_label("Scroll All Windows").click();
        h.run_steps(2);
        assert!(h.state().ui.tool_options.hand_scroll_all, "the checkbox turns it on");
        h.get_by_label("Scroll All Windows").click();
        h.run_steps(2);
        assert!(!h.state().ui.tool_options.hand_scroll_all, "and off again");
    }

    /// The Hand tool's options bar with Scroll All Windows on, two tiled documents and a document
    /// window, as `the_hand_tool_options_bar_shows_scroll_all_windows` draws it.
    fn options_bar_with_scroll_all() -> egui_kittest::Harness<'static, PhotocraftApp> {
        use egui_kittest::Harness;
        let mut app = app_with_tiles();
        app.ui.tool = Tool::Hand;
        app.ui.tool_options.hand_scroll_all = true;
        app.ui.views[0].zoom = 0.25;
        app.ui.windows.push(DocWindow {
            id: 7,
            document: 1,
            view: view(0.5, [400.0, 300.0], 0.0),
            open: true,
            floating: false,
            focused: false,
            position: None,
        });
        let mut h = Harness::builder().with_size(egui::vec2(1400.0, 60.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                crate::panels::options_bar(app, ui);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::default());
        h.run_steps(4);
        h
    }

    #[test]
    fn with_scroll_all_windows_on_the_zoom_buttons_change_only_the_active_view() {
        use egui_kittest::kittest::Queryable;
        for button in ["100%", "Fit Screen", "Fill Screen"] {
            let mut h = options_bar_with_scroll_all();
            let others = (h.state().ui.views[1].clone(), h.state().ui.windows[0].view.clone());
            h.get_by_label(button).click();
            h.run_steps(2);
            let active = &h.state().ui.views[0];
            assert!(active.zoom == 1.0 || active.fit_pending || active.fill_pending, "{button} changes the active view");
            let (tile, window) = (&h.state().ui.views[1], &h.state().ui.windows[0].view);
            assert_eq!(
                (tile.zoom, tile.center, tile.fit_pending, tile.fill_pending),
                (others.0.zoom, others.0.center, false, false),
                "{button}: the other tile stays"
            );
            assert_eq!(
                (window.zoom, window.center, window.fit_pending, window.fill_pending),
                (others.1.zoom, others.1.center, false, false),
                "{button}: the window stays"
            );
        }
    }

    #[test]
    fn the_scroll_all_windows_checkbox_has_the_reference_tooltip() {
        use egui_kittest::kittest::Queryable;
        let mut h = options_bar_with_scroll_all();
        h.ctx.global_style_mut(|s| s.interaction.tooltip_delay = 0.0);
        h.get_by_label("Scroll All Windows").hover();
        h.run_steps(4);
        assert!(h.query_by_label("Scroll all open document windows when using the hand tool").is_some());
    }
}
