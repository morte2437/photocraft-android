//! Preferences › General › Zoom Resizes Windows, which is also the Zoom tool's Resize Windows to
//! Fit (#204).
//!
//! As in Photoshop the two are one setting, so turning it on in either place shows it on in the
//! other. With it on, zooming a document that floats in its own window resizes that window to wrap
//! the image at the new zoom, no bigger than the screen it is on; with it off (the default) the
//! image zooms inside a window that keeps its size. Tabbed documents never resize the main window.
//!
//! Only a change of zoom resizes. Fit on Screen derives the zoom from the window, so a fit leaves
//! the window alone and the two can never chase each other.
//!
//! A zoom about the pointer leaves the image off centre, so a window grown to wrap it also
//! re-centres the image in what the scrollbars leave free; a window the screen caps keeps the
//! zoomed spot where it is.

use egui::{Context, Id, Pos2, Rect, Vec2, ViewportClass, ViewportCommand, ViewportId, vec2};

use crate::PhotocraftApp;
use crate::state::{DocWindow, View};

/// The smallest window a zoom asks for (points), so zooming far out keeps the window usable.
pub const MIN_SIZE: Vec2 = vec2(240.0, 160.0);

/// Room left on the screen for the window's title bar and the taskbar or Dock (points).
pub const SCREEN_MARGIN: f32 = 48.0;

/// What wrapping a document window needs besides the view itself.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    /// The window's device pixels per egui point (`View::zoom` is in device pixels).
    pub ppp: f32,
    /// View › Pixel Aspect Ratio Correction: document pixels are shown this much wider.
    pub aspect: f32,
    /// The window's own chrome around the image, the rulers, in points.
    pub chrome: Vec2,
    /// The scrollbars along the right and bottom (points; 0 when hidden). They are drawn over the
    /// canvas, so the window makes room for them too.
    pub bars: f32,
    /// View › Flip Horizontal.
    pub flip: bool,
    /// The size of the screen the window is on, in points, when the platform reports it.
    pub monitor: Option<Vec2>,
}

/// The inner size (points) of a window that wraps `view`'s document at its zoom and rotation,
/// at least [`MIN_SIZE`] but never bigger than the screen, even a screen smaller than that
/// minimum. `None` until the view has a document size.
pub fn wrapped_size(view: &View, layout: &Layout) -> Option<Vec2> {
    wrap(view, layout).map(|(_, size)| size)
}

/// The window size the image and chrome need, and the size [`wrapped_size`] asks for.
fn wrap(view: &View, layout: &Layout) -> Option<(Vec2, Vec2)> {
    let usable = |x: f32| x.is_finite() && x > 0.0;
    if !usable(view.zoom) || !usable(layout.ppp) {
        return None;
    }
    let aspect = if usable(layout.aspect) { layout.aspect } else { 1.0 };
    let w = view.doc_size[0] as f32 * aspect * view.zoom / layout.ppp;
    let h = view.doc_size[1] as f32 * view.zoom / layout.ppp;
    if !usable(w) || !usable(h) {
        return None;
    }
    // A turned view (View › Rotate View) shows the image's bounding box.
    let (sin, cos) = view.rotation.to_radians().sin_cos();
    let (sin, cos) = (sin.abs(), cos.abs());
    let image = vec2(w * cos + h * sin, w * sin + h * cos);
    let bars = if usable(layout.bars) { layout.bars } else { 0.0 };
    let chrome = if layout.chrome.is_finite() { layout.chrome.max(Vec2::ZERO) } else { Vec2::ZERO } + Vec2::splat(bars);
    // The screen is the hard cap: applied last, it wins over the minimum on a tiny screen.
    let screen =
        layout.monitor.filter(|m| usable(m.x) && usable(m.y)).map_or(Vec2::splat(f32::INFINITY), |m| (m - Vec2::splat(SCREEN_MARGIN)).max(Vec2::splat(1.0)));
    Some((image + chrome, (image + chrome).max(MIN_SIZE).min(screen)))
}

/// The view centre that shows the whole image in the middle of a window of [`wrapped_size`]:
/// the image centred in the canvas less its scrollbars. `None` when the screen capped the window
/// and the image doesn't fit, so the zoomed spot stays put.
pub fn wrapped_center(view: &View, layout: &Layout) -> Option<[f32; 2]> {
    let (needed, size) = wrap(view, layout)?;
    if needed.x > size.x + 0.5 || needed.y > size.y + 0.5 {
        return None;
    }
    let bars = if layout.bars.is_finite() && layout.bars > 0.0 { layout.bars } else { 0.0 };
    let middle = [view.doc_size[0] as f32 / 2.0, view.doc_size[1] as f32 / 2.0];
    // The canvas centre sits half a bar right of and below the middle of the free area.
    let xf = crate::canvas::ViewXform {
        rect: Rect::from_center_size(Pos2::ZERO, Vec2::ZERO),
        zoom: view.zoom / layout.ppp,
        center: middle,
        flip: layout.flip,
        rotation: view.rotation,
        aspect: layout.aspect,
    };
    let [x, y] = xf.to_doc(Pos2::new(bars / 2.0, bars / 2.0));
    let center = [x as f32, y as f32];
    center.iter().all(|c| c.is_finite()).then_some(center)
}

/// Where the zoom a window was last wrapped for (or first seen at) is kept between frames.
fn last_zoom_id(window: u64) -> Id {
    Id::new(("pc-zoom-resizes-windows", window))
}

/// The inner size floating window `window` should take this frame, if any: when `on` (the
/// preference) and its zoom moved from `before` (its view as the frame began) to `after` (the view
/// its canvas left), other than by a fit. Every call records the zoom, so the first frame of a new
/// window and the frames with the preference off never resize later.
pub fn resize_for(ctx: &Context, window: u64, on: bool, before: &View, after: &View, layout: &Layout) -> Option<Vec2> {
    let id = last_zoom_id(window);
    let last = ctx.data_mut(|d| {
        let last = d.get_temp::<f32>(id);
        d.insert_temp(id, after.zoom);
        last
    });
    let fitting = before.fit_pending || before.fill_pending || after.fit_pending || after.fill_pending;
    // Compare the bits: any change of zoom counts, and NaN never matches itself into a resize loop.
    if !on || fitting || last?.to_bits() == after.zoom.to_bits() {
        return None;
    }
    wrapped_size(after, layout)
}

/// Called by a floating document window (`canvas::extra_windows`) after its canvas drew in `rect`:
/// with Zoom Resizes Windows on, a zoom resizes the window around the image. A window drawn
/// embedded in the main one (a backend without multiple windows, the web) is never resized, as the
/// command would resize the main window instead, and a maximized or full-screen one keeps the
/// size the user gave it. A window that grows around the whole image also centres it in `after`.
/// Returns the size asked for.
pub fn follow(
    app: &PhotocraftApp,
    ctx: &Context,
    class: ViewportClass,
    viewport: ViewportId,
    window: &DocWindow,
    after: &mut View,
    rect: Rect,
) -> Option<Vec2> {
    if class != ViewportClass::Immediate {
        return None;
    }
    // Still recorded while maximized, so leaving it doesn't resize for a zoom made meanwhile.
    let pinned = ctx.input(|i| i.viewport().maximized == Some(true) || i.viewport().fullscreen == Some(true));
    let layout = Layout {
        ppp: ctx.pixels_per_point(),
        aspect: app.ui.view.display_aspect(),
        chrome: rect.size() - crate::rulers::content_rect(app, rect).size(),
        bars: if app.ui.view.shows_scrollbars() { crate::scrollbars::THICKNESS } else { 0.0 },
        flip: app.ui.view.flip_horizontal,
        monitor: ctx.input(|i| i.viewport().monitor_size),
    };
    let on = app.session.prefs().general.zoom_resizes_windows && !pinned;
    let size = resize_for(ctx, window.id, on, &window.view, after, &layout)?;
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::InnerSize(size));
    if let Some(center) = wrapped_center(after, &layout) {
        after.center = center;
        // The canvas drew before this: show the centred view on the next frame.
        ctx.request_repaint_of(viewport);
    }
    Some(size)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn view(doc_size: [u32; 2], zoom: f32) -> View {
        View { zoom, doc_size, fit_pending: false, ..View::default() }
    }

    fn layout() -> Layout {
        Layout { ppp: 1.0, aspect: 1.0, chrome: Vec2::ZERO, bars: 0.0, flip: false, monitor: Some(vec2(1920.0, 1080.0)) }
    }

    #[test]
    fn the_window_wraps_the_image_at_its_zoom() {
        assert_eq!(wrapped_size(&view([400, 300], 1.0), &layout()), Some(vec2(400.0, 300.0)));
        assert_eq!(wrapped_size(&view([400, 300], 2.0), &layout()), Some(vec2(800.0, 600.0)));
        // `zoom` is device pixels: on a 2× display 100% is half as many points.
        assert_eq!(wrapped_size(&view([800, 600], 1.0), &Layout { ppp: 2.0, ..layout() }), Some(vec2(400.0, 300.0)));
        // The rulers sit inside the window, around the image.
        assert_eq!(wrapped_size(&view([400, 300], 1.0), &Layout { chrome: vec2(16.0, 16.0), ..layout() }), Some(vec2(416.0, 316.0)));
        // Pixel Aspect Ratio Correction shows the pixels wider.
        assert_eq!(wrapped_size(&view([400, 300], 1.0), &Layout { aspect: 2.0, ..layout() }), Some(vec2(800.0, 300.0)));
    }

    #[test]
    fn the_window_stays_between_the_minimum_and_the_screen() {
        assert_eq!(wrapped_size(&view([400, 300], 0.05), &layout()), Some(MIN_SIZE));
        let screen = vec2(1920.0, 1080.0) - Vec2::splat(SCREEN_MARGIN);
        assert_eq!(wrapped_size(&view([400, 300], 16.0), &layout()), Some(screen));
        // One axis can be clamped while the other still wraps.
        assert_eq!(wrapped_size(&view([4000, 300], 1.0), &layout()), Some(vec2(screen.x, 300.0)));
        // Without a reported screen only the minimum applies.
        assert_eq!(wrapped_size(&view([400, 300], 16.0), &Layout { monitor: None, ..layout() }), Some(vec2(6400.0, 4800.0)));
        // A nonsense screen size is ignored rather than shrinking the window to nothing.
        assert_eq!(wrapped_size(&view([400, 300], 1.0), &Layout { monitor: Some(vec2(0.0, f32::NAN)), ..layout() }), Some(vec2(400.0, 300.0)));
        // A screen smaller than the minimum still caps the window: it never outgrows the screen.
        let tiny = Layout { monitor: Some(vec2(260.0, 190.0)), ..layout() };
        assert_eq!(wrapped_size(&view([400, 300], 1.0), &tiny), Some(vec2(260.0, 190.0) - Vec2::splat(SCREEN_MARGIN)));
        assert_eq!(wrapped_size(&view([400, 300], 0.05), &tiny), Some(vec2(260.0, 190.0) - Vec2::splat(SCREEN_MARGIN)));
    }

    #[test]
    fn a_turned_view_wraps_the_bounding_box() {
        let turned = View { rotation: 90.0, ..view([400, 300], 1.0) };
        let size = wrapped_size(&turned, &layout()).unwrap();
        assert!((size - vec2(300.0, 400.0)).length() < 0.01, "{size:?}");
        let turned = View { rotation: 45.0, ..view([400, 400], 1.0) };
        let size = wrapped_size(&turned, &layout()).unwrap();
        assert!((size.x - 400.0 * std::f32::consts::SQRT_2).abs() < 0.01, "{size:?}");
    }

    #[test]
    fn nothing_to_wrap_asks_for_nothing() {
        assert_eq!(wrapped_size(&view([0, 0], 1.0), &layout()), None, "the view hasn't shown its document yet");
        assert_eq!(wrapped_size(&view([400, 300], 0.0), &layout()), None);
        assert_eq!(wrapped_size(&view([400, 300], f32::NAN), &layout()), None);
        assert_eq!(wrapped_size(&view([400, 300], 1.0), &Layout { ppp: 0.0, ..layout() }), None);
    }

    #[test]
    fn only_a_zoom_change_with_the_preference_on_resizes() {
        let ctx = Context::default();
        let at = |zoom| view([400, 300], zoom);
        // The first frame of a window only records its zoom.
        assert_eq!(resize_for(&ctx, 7, true, &at(1.0), &at(1.0), &layout()), None);
        // The same zoom again: nothing to do.
        assert_eq!(resize_for(&ctx, 7, true, &at(1.0), &at(1.0), &layout()), None);
        // Zoom in (the Zoom tool, the wheel, Window › Arrange › Match Zoom): the window follows.
        assert_eq!(resize_for(&ctx, 7, true, &at(1.0), &at(2.0), &layout()), Some(vec2(800.0, 600.0)));
        assert_eq!(resize_for(&ctx, 7, true, &at(2.0), &at(2.0), &layout()), None, "once per change");
        // A zoom changed outside the canvas (Match Zoom) still counts on the next frame.
        assert_eq!(resize_for(&ctx, 7, true, &at(0.5), &at(0.5), &layout()), Some(vec2(240.0, 160.0)));
        // Another window keeps its own record.
        assert_eq!(resize_for(&ctx, 8, true, &at(0.5), &at(0.5), &layout()), None);
    }

    #[test]
    fn with_the_preference_off_the_window_keeps_its_size() {
        let ctx = Context::default();
        let at = |zoom| view([400, 300], zoom);
        assert_eq!(resize_for(&ctx, 1, false, &at(1.0), &at(1.0), &layout()), None);
        assert_eq!(resize_for(&ctx, 1, false, &at(1.0), &at(3.0), &layout()), None);
        // Turning it on later doesn't catch up on zooms made while it was off.
        assert_eq!(resize_for(&ctx, 1, true, &at(3.0), &at(3.0), &layout()), None);
        assert_eq!(resize_for(&ctx, 1, true, &at(3.0), &at(1.0), &layout()), Some(vec2(400.0, 300.0)));
    }

    #[test]
    fn a_fit_never_resizes_the_window() {
        let ctx = Context::default();
        let at = |zoom| view([400, 300], zoom);
        assert_eq!(resize_for(&ctx, 2, true, &at(1.0), &at(1.0), &layout()), None);
        // The fit takes its zoom from the window; resizing for it would start a loop.
        let fitting = View { fit_pending: true, ..at(1.0) };
        assert_eq!(resize_for(&ctx, 2, true, &fitting, &at(0.4), &layout()), None);
        let filling = View { fill_pending: true, ..at(0.4) };
        assert_eq!(resize_for(&ctx, 2, true, &filling, &at(0.6), &layout()), None);
        // The zoom the fit chose is the new baseline.
        assert_eq!(resize_for(&ctx, 2, true, &at(0.6), &at(0.6), &layout()), None);
    }

    #[test]
    fn follow_resizes_only_real_floating_windows_with_the_preference_on() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        app.ui.extras.rulers = false;
        let ctx = Context::default();
        let viewport = ViewportId::from_hash_of(("docwin", 3_u64));
        let rect = Rect::from_min_size(egui::Pos2::ZERO, vec2(800.0, 600.0));
        let at = |zoom| view([400, 300], zoom);
        let window = DocWindow { id: 3, document: 0, view: at(1.0), open: true, position: None };
        let frame = |app: &PhotocraftApp, class, after: &View| follow(app, &ctx, class, viewport, &window, &mut after.clone(), rect);
        // Off by default, as in Photoshop.
        assert!(!app.session.prefs().general.zoom_resizes_windows);
        assert_eq!(frame(&app, ViewportClass::Immediate, &at(1.0)), None);
        assert_eq!(frame(&app, ViewportClass::Immediate, &at(2.0)), None);
        app.run("prefs.set", json!({"path": "general.zoomResizesWindows", "value": true})).unwrap();
        // The scrollbars are drawn over the canvas: the window makes room for them.
        assert!(app.ui.view.shows_scrollbars());
        let bars = Vec2::splat(crate::scrollbars::THICKNESS);
        assert_eq!(frame(&app, ViewportClass::Immediate, &at(1.0)), Some(vec2(400.0, 300.0) + bars));
        // Drawn inside the main window, a resize command would resize the main window: skipped.
        assert_eq!(frame(&app, ViewportClass::EmbeddedWindow, &at(2.0)), None);
        // With the rulers on, the window makes room for them.
        app.ui.extras.rulers = true;
        let ruled = frame(&app, ViewportClass::Immediate, &at(2.0)).unwrap();
        assert!(ruled.x > 800.0 && ruled.y > 600.0, "{ruled:?}");
    }

    /// Where the image lands in a window of the wrapped size, and the canvas area the scrollbars
    /// leave free, both in points from the window's top-left.
    fn landing(view: &View, layout: &Layout) -> (Rect, Rect) {
        let size = wrapped_size(view, layout).unwrap();
        let canvas = Rect::from_min_max(Pos2::ZERO + layout.chrome, Pos2::ZERO + size);
        let free = Rect::from_min_max(canvas.min, canvas.max - Vec2::splat(layout.bars));
        let xf = crate::canvas::ViewXform {
            rect: canvas,
            zoom: view.zoom / layout.ppp,
            center: view.center,
            flip: layout.flip,
            rotation: view.rotation,
            aspect: layout.aspect,
        };
        (xf.image_bounds(view.doc_size), free)
    }

    fn near(a: Rect, b: Rect) -> bool {
        (a.min - b.min).length() < 0.01 && (a.max - b.max).length() < 0.01
    }

    #[test]
    fn the_window_makes_room_for_the_scrollbars() {
        let barred = Layout { bars: 12.0, chrome: vec2(16.0, 16.0), ..layout() };
        assert_eq!(wrapped_size(&view([400, 300], 1.0), &barred), Some(vec2(428.0, 328.0)));
    }

    #[test]
    fn a_wrapped_window_shows_the_whole_image_centred() {
        // Zoomed in about the top-left corner: the view centre is far from the image's middle.
        let zoomed = View { center: [60.0, 40.0], ..view([400, 300], 2.0) };
        for layout in [
            layout(),
            Layout { bars: 12.0, chrome: vec2(16.0, 16.0), ..layout() },
            Layout { bars: 12.0, flip: true, ..layout() },
            Layout { bars: 12.0, ppp: 2.0, aspect: 1.5, ..layout() },
        ] {
            for rotation in [0.0, 90.0, -30.0] {
                let mut v = View { rotation, ..zoomed.clone() };
                let (before, free) = landing(&v, &layout);
                assert!(!near(before, free), "{layout:?} {rotation}: off centre before");
                v.center = wrapped_center(&v, &layout).unwrap();
                let (after, free) = landing(&v, &layout);
                assert!(near(after, free), "{layout:?} {rotation}: {after:?} fills {free:?}");
            }
        }
        // Zoomed far out the window keeps its minimum size, and the image sits in its middle.
        let small = View { center: [0.0, 0.0], ..view([400, 300], 0.1) };
        let c = wrapped_center(&small, &layout()).unwrap();
        assert!((c[0] - 200.0).abs() < 0.01 && (c[1] - 150.0).abs() < 0.01, "{c:?}");
    }

    #[test]
    fn a_window_the_screen_caps_keeps_the_zoomed_spot() {
        let zoomed = View { center: [60.0, 40.0], ..view([400, 300], 16.0) };
        assert_eq!(wrapped_center(&zoomed, &layout()), None);
    }

    #[test]
    fn follow_centres_the_image_in_the_grown_window() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        app.run("prefs.set", json!({"path": "general.zoomResizesWindows", "value": true})).unwrap();
        let ctx = Context::default();
        let viewport = ViewportId::from_hash_of(("docwin", 4_u64));
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let window = DocWindow { id: 4, document: 0, view: view([400, 300], 1.0), open: true, position: None };
        let mut first = view([400, 300], 1.0);
        assert_eq!(follow(&app, &ctx, ViewportClass::Immediate, viewport, &window, &mut first, rect), None);
        // A Zoom tool click near the top-left corner doubles the zoom about that spot.
        let mut zoomed = View { center: [60.0, 40.0], ..view([400, 300], 2.0) };
        assert!(follow(&app, &ctx, ViewportClass::Immediate, viewport, &window, &mut zoomed, rect).is_some());
        let half_bar = crate::scrollbars::THICKNESS / 2.0 / 2.0;
        assert_eq!(zoomed.center, [200.0 + half_bar, 150.0 + half_bar], "the whole image, centred");
        assert_eq!(zoomed.zoom, 2.0);
    }

    #[test]
    fn a_maximized_or_full_screen_window_keeps_its_size() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.sync_views();
        app.run("prefs.set", json!({"path": "general.zoomResizesWindows", "value": true})).unwrap();
        let rect = Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
        let window = DocWindow { id: 5, document: 0, view: view([400, 300], 1.0), open: true, position: None };
        for (maximized, fullscreen) in [(true, false), (false, true)] {
            let ctx = Context::default();
            let mut raw = egui::RawInput::default();
            let info = egui::ViewportInfo { maximized: Some(maximized), fullscreen: Some(fullscreen), ..Default::default() };
            raw.viewports.insert(ViewportId::ROOT, info);
            let viewport = ViewportId::ROOT;
            let mut sizes = Vec::new();
            let frame = |raw: egui::RawInput, zoom: f32, sizes: &mut Vec<Option<Vec2>>| {
                let mut out = ctx.run_ui(raw, |ui| {
                    let mut v = View { center: [60.0, 40.0], ..view([400, 300], zoom) };
                    sizes.push(follow(&app, ui.ctx(), ViewportClass::Immediate, viewport, &window, &mut v, rect));
                    assert_eq!(v.center, [60.0, 40.0]);
                });
                out.textures_delta.clear();
            };
            let mut normal = egui::RawInput::default();
            normal.viewports.insert(ViewportId::ROOT, egui::ViewportInfo { maximized: Some(false), fullscreen: Some(false), ..Default::default() });
            frame(normal.clone(), 1.0, &mut sizes);
            frame(raw, 2.0, &mut sizes);
            // Back to a normal window: the zoom made meanwhile is not caught up on.
            frame(normal, 2.0, &mut sizes);
            assert_eq!(sizes, [None, None, None], "maximized {maximized}, full screen {fullscreen}");
        }
    }

    #[test]
    fn the_zoom_tool_options_bar_shows_the_same_setting() {
        // Photoshop's Zoom tool option Resize Windows to Fit is Preferences › Zoom Resizes Windows.
        use egui_kittest::Harness;
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        app.ui.tool = crate::state::Tool::Zoom;
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
        assert!(!h.state().session.prefs().general.zoom_resizes_windows);
        h.get_by_label("Resize Windows to Fit").click();
        h.run_steps(2);
        assert!(h.state().session.prefs().general.zoom_resizes_windows, "the option turns the preference on");
        h.get_by_label("Resize Windows to Fit").click();
        h.run_steps(2);
        assert!(!h.state().session.prefs().general.zoom_resizes_windows, "and off again");
    }
}
