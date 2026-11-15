//! #10930: before the first change to a crop frame the reference application shows only a dashed
//! outline with grey handles, with no shield and no overlay in any show mode. The shield and the
//! overlay start with the first change and stay until the crop is committed or cancelled. These
//! test the decisions the canvas draws from (`current_guides`, `crop_shield::current`,
//! `frame_look`), driven by real Crop gestures through the control channel.

use photocraft_ui_egui::control::{ControlRequest, handle};
use photocraft_ui_egui::crop_overlay::{FrameLook, OverlayShow, current_guides, frame_look};
use photocraft_ui_egui::{PhotocraftApp, Tool};
use serde_json::{Value, json};

fn control(app: &mut PhotocraftApp, method: &str, params: Value) {
    let ctx = egui::Context::default();
    let (req, _rx) = ControlRequest::new(method, params);
    handle(app, &ctx, &req);
    app.sync_views();
}

/// A document with the Crop tool showing its untouched default frame, overlay mode `show`.
fn crop_app(show: OverlayShow) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 120})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::Crop;
    app.ui.tool_options.crop_overlay_show = show;
    photocraft_ui_egui::crop_ui::ensure_frame(&mut app);
    assert!(app.ui.crop_rect.is_some(), "the Crop tool shows a frame");
    app
}

/// What the canvas would draw now: (guides shown, shield shown, outline look).
fn drawn(app: &PhotocraftApp) -> (bool, bool, FrameLook) {
    let ctx = egui::Context::default();
    let guides = !current_guides(app, 300.0, 200.0).is_empty();
    let shield = photocraft_ui_egui::crop_shield::current(app, &ctx).is_some();
    (guides, shield, frame_look(app))
}

/// Press inside the untouched frame and drag: that draws a new frame, the first change.
fn press_and_drag(app: &mut PhotocraftApp) {
    control(app, "ui.pointer", json!({"tool": "Crop", "events": [{"kind": "down", "x": 50, "y": 40}, {"kind": "move", "x": 150, "y": 90}]}));
}

fn release(app: &mut PhotocraftApp) {
    control(app, "ui.pointer", json!({"tool": "Crop", "events": [{"kind": "up", "x": 150, "y": 90}]}));
}

const MODES: [OverlayShow; 3] = [OverlayShow::Always, OverlayShow::Auto, OverlayShow::Never];

#[test]
fn an_untouched_frame_shows_only_a_dashed_outline_in_every_show_mode() {
    for show in MODES {
        let app = crop_app(show);
        assert_eq!(drawn(&app), (false, false, FrameLook::Untouched), "{show:?}: no overlay, no shield, dashed outline");
    }
}

#[test]
fn the_shield_and_the_overlay_start_with_the_first_change() {
    // (mode, overlay while dragging, overlay after release with the crop still pending)
    for (show, dragging, released) in [(OverlayShow::Always, true, true), (OverlayShow::Auto, true, false), (OverlayShow::Never, false, false)] {
        let mut app = crop_app(show);
        press_and_drag(&mut app);
        assert_eq!(drawn(&app), (dragging, true, FrameLook::Started), "{show:?} mid-drag: shield on, solid outline");
        release(&mut app);
        assert!(app.ui.crop_rect.is_some(), "{show:?}: the crop is still pending after release");
        assert_eq!(drawn(&app), (released, true, FrameLook::Started), "{show:?} after release: the shield stays until commit or cancel");
    }
}

#[test]
fn committing_or_cancelling_brings_back_the_untouched_look() {
    for show in MODES {
        let mut app = crop_app(show);
        press_and_drag(&mut app);
        release(&mut app);
        photocraft_ui_egui::canvas::commit_crop(&mut app);
        photocraft_ui_egui::crop_ui::ensure_frame(&mut app);
        // A commit hides the frame until the next press, so there is nothing to draw.
        assert!(app.ui.crop_rect.is_none(), "{show:?}: the commit hides the frame");

        press_and_drag(&mut app);
        release(&mut app);
        assert!(drawn(&app).1, "{show:?}: a second crop starts again");
        photocraft_ui_egui::crop_ui::cancel(&mut app);
        photocraft_ui_egui::crop_ui::ensure_frame(&mut app);
        assert_eq!(drawn(&app), (false, false, FrameLook::Untouched), "{show:?} after cancel: a new untouched frame");
    }
}

#[test]
fn classic_mode_keeps_the_look_it_had() {
    // Classic Mode wasn't measured, so it keeps its solid white outline, shield and overlay rule.
    let mut app = crop_app(OverlayShow::Always);
    app.ui.tool_options.crop_shield.classic_mode = true;
    let (guides, shield, look) = drawn(&app);
    assert_eq!(look, FrameLook::Classic);
    assert!(guides && shield, "Classic Mode shows its overlay and shield as before, even untouched");
}

#[test]
fn a_press_that_has_not_moved_yet_is_not_a_change() {
    // The first change is a drag; a press held still leaves the untouched look in every mode.
    for show in MODES {
        let mut app = crop_app(show);
        control(&mut app, "ui.pointer", json!({"tool": "Crop", "events": [{"kind": "down", "x": 50, "y": 40}]}));
        assert_eq!(drawn(&app), (false, false, FrameLook::Untouched), "{show:?}: pressed, not moved");
        control(&mut app, "ui.pointer", json!({"tool": "Crop", "events": [{"kind": "move", "x": 150, "y": 90}]}));
        assert_eq!(drawn(&app).2, FrameLook::Started, "{show:?}: the move is the first change");
    }
}

/// A click inside the untouched frame that doesn't move is no change either: released, the frame
/// keeps its untouched look, with no shield and no overlay (noted in review).
#[test]
fn a_click_inside_the_untouched_frame_is_not_a_change() {
    for show in MODES {
        let mut app = crop_app(show);
        let before = app.ui.crop_rect;
        control(&mut app, "ui.pointer", json!({"tool": "Crop", "events": [{"kind": "down", "x": 50, "y": 40}, {"kind": "up", "x": 50, "y": 40}]}));
        assert_eq!(app.ui.crop_rect, before, "{show:?}: the frame is where it was");
        assert_eq!(drawn(&app), (false, false, FrameLook::Untouched), "{show:?}: clicked, not changed");
    }
}

/// A click on a handle of the untouched frame that doesn't move resizes nothing, so the frame
/// keeps its untouched look after the release, as a click inside it does (noted in review).
#[test]
fn a_click_on_a_handle_of_the_untouched_frame_is_not_a_change() {
    for show in MODES {
        let mut app = crop_app(show);
        let before = app.ui.crop_rect.unwrap();
        let (x, y) = (before[2], before[3]);
        control(&mut app, "ui.pointer", json!({"tool": "Crop", "events": [{"kind": "down", "x": x, "y": y}, {"kind": "up", "x": x, "y": y}]}));
        assert_eq!(app.ui.crop_rect, Some(before), "{show:?}: the frame is where it was");
        assert_eq!(drawn(&app), (false, false, FrameLook::Untouched), "{show:?}: a handle clicked, not dragged");
    }
}

/// An Alt-drag from the centre can draw the very frame the drag started from. The drag changed
/// the frame on its way there, so after the release the crop is started, not untouched again
/// (noted in review): what counts is that the gesture changed the frame, not how it ended.
#[test]
fn a_drag_that_draws_the_same_frame_again_has_started_the_crop() {
    for show in MODES {
        let mut app = crop_app(show);
        let before = app.ui.crop_rect.unwrap();
        let c = [(before[0] + before[2]) / 2.0, (before[1] + before[3]) / 2.0];
        control(
            &mut app,
            "ui.pointer",
            json!({"tool": "Crop", "alt": true, "events": [{"kind": "down", "x": c[0], "y": c[1]}, {"kind": "move", "x": c[0] + 20.0, "y": c[1] + 10.0}, {"kind": "move", "x": before[2], "y": before[3]}, {"kind": "up", "x": before[2], "y": before[3]}]}),
        );
        assert_eq!(app.ui.crop_rect, Some(before), "{show:?}: drawn from the centre back to the same frame");
        assert_eq!(drawn(&app).2, FrameLook::Started, "{show:?}: the drag changed it, so the crop has started");
    }
}

#[test]
fn show_cropped_area_off_still_hides_what_is_outside_an_untouched_frame() {
    let mut app = crop_app(OverlayShow::Always);
    app.ui.tool_options.crop_shield.show_cropped_area = false;
    let ctx = egui::Context::default();
    assert!(photocraft_ui_egui::crop_shield::current(&app, &ctx).is_some(), "the opaque outside fill stays");
    assert!(current_guides(&app, 300.0, 200.0).is_empty(), "but no overlay before the first change");
}

/// Within a drag the first change counts: a drag that changes the frame and comes back to where it
/// started keeps the started look, rather than flickering back to the untouched one mid-drag
/// (noted in review). Released, the crop is pending as after any drawn frame.
#[test]
fn a_drag_brought_back_to_its_start_keeps_the_started_look_until_it_ends() {
    for show in MODES {
        let mut app = crop_app(show);
        control(&mut app, "ui.pointer", json!({"tool": "Crop", "events": [{"kind": "down", "x": 50, "y": 40}, {"kind": "move", "x": 150, "y": 90}]}));
        assert_eq!(drawn(&app).2, FrameLook::Started, "{show:?}: the first change");
        control(&mut app, "ui.pointer", json!({"tool": "Crop", "events": [{"kind": "move", "x": 50, "y": 40}]}));
        assert_eq!(drawn(&app).2, FrameLook::Started, "{show:?}: back where it started, still the started look");
        assert!(drawn(&app).1, "{show:?}: and the shield");
        control(&mut app, "ui.pointer", json!({"tool": "Crop", "events": [{"kind": "up", "x": 50, "y": 40}]}));
        assert_eq!(drawn(&app).2, FrameLook::Started, "{show:?}: released, the crop is pending");
    }
}
