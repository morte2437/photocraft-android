//! Animated Zoom: press and hold with the Zoom tool to zoom continuously (#6210).
//!
//! With Preferences › General › Animated Zoom on (the default, as in Photoshop) a press held
//! still past [`DELAY`] zooms in smoothly about the press point until the button comes up, or out
//! with ⌥. The release is then no click step. A quick click still steps the zoom (the canvas), a
//! drag still scrubs or draws a zoom rectangle (`zoom_tool.rs`), and with the preference off a
//! hold does nothing more than a click.
//!
//! The rate is per second of wall time, so the zoom grows the same at any frame rate.

use egui::Pos2;

use crate::canvas::ViewXform;
use crate::state::View;

/// Seconds a press must be held still before it zooms continuously (a click is shorter).
pub const DELAY: f64 = 0.3;
/// Doublings (or halvings, zooming out) per second while held.
pub const RATE: f32 = 2.0;
/// Screen points the pointer may wander before the press counts as a drag instead.
const SLOP: f32 = 6.0;
/// The longest frame gap a hold zooms over: a stalled frame never jumps the zoom far.
const MAX_DT: f64 = 0.25;

/// A Zoom-tool press being held.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Hold {
    /// Screen point where the button went down.
    anchor: Pos2,
    /// The document point under it (it stays under the anchor).
    doc: [f64; 2],
    /// When the button went down (egui seconds).
    t0: f64,
    /// The time the zoom last advanced to, once zooming.
    last: Option<f64>,
    /// The pointer moved before the hold began: this press is a drag, never a hold.
    dragged: bool,
}

/// Where a canvas keeps its hold: each canvas (document `idx` of the tiled windows, in the main
/// or a secondary window) has its own, so a hold on one never zooms another.
pub fn hold_id(idx: usize, primary: bool, viewport: egui::ViewportId) -> egui::Id {
    egui::Id::new(("pc-zoom-tool-hold", idx, primary, viewport))
}

/// The zoom after holding for `dt` seconds at `zoom` (out: shrinking), within the range for a
/// document of `size` px. A non-finite or negative `dt` changes nothing.
pub fn hold_zoom(zoom: f32, dt: f64, out: bool, size: [u32; 2]) -> f32 {
    let dt = if dt.is_finite() { dt.clamp(0.0, MAX_DT) as f32 } else { 0.0 };
    let e = if out { -RATE * dt } else { RATE * dt };
    crate::zoom_levels::clamp(zoom * e.exp2(), size)
}

/// Is a held press zooming right now? (It owns the gesture: no scrub, no click step.)
pub fn active(ctx: &egui::Context, id: egui::Id) -> bool {
    ctx.data(|d| d.get_temp::<Hold>(id)).is_some_and(|h| h.last.is_some())
}

/// Advance the Zoom-tool hold of the canvas whose [`hold_id`] is `id` by one frame. `enabled` is
/// the Animated Zoom preference, `out` the zoom direction (⌥ or a temporary zoom-out key, read
/// live), `on_canvas` whether the canvas owns the press (not a dialog, a scrollbar or the compass
/// over it); only such a press can become a hold. Returns true when the press is a hold zoom (on
/// its release frame too), so the canvas skips the scrub and the click step.
#[allow(clippy::too_many_arguments)]
pub fn update(ctx: &egui::Context, view: &mut View, xf: &ViewXform, enabled: bool, out: bool, ppp: f32, on_canvas: bool, id: egui::Id) -> bool {
    let (down, origin, start, pos, now) =
        ctx.input(|i| (i.pointer.primary_down(), i.pointer.press_origin(), i.pointer.press_start_time(), i.pointer.latest_pos(), i.time));
    let held = ctx.data(|d| d.get_temp::<Hold>(id));
    if !enabled || !down {
        if held.is_some() {
            ctx.data_mut(|d| d.remove::<Hold>(id));
        }
        return enabled && held.is_some_and(|h| h.last.is_some());
    }
    let start = start.filter(|t| t.is_finite());
    // A hold left from an earlier press (the tool changed before its release, so no frame here
    // saw the button come up) is not this press's.
    let mine = |h: &Hold| origin == Some(h.anchor) && start.is_none_or(|t| t == h.t0);
    let mut h = match held.filter(mine) {
        Some(h) => h,
        None => {
            // A press that began on the canvas (not one dragged in from a panel) and that the
            // canvas owns.
            let Some(anchor) = origin.filter(|p| on_canvas && xf.rect.contains(*p)) else {
                ctx.data_mut(|d| d.remove::<Hold>(id));
                return false;
            };
            let t0 = start.unwrap_or(now);
            let h = Hold { anchor, doc: xf.to_doc(anchor), t0, last: None, dragged: false };
            ctx.data_mut(|d| d.insert_temp(id, h));
            h
        }
    };
    if h.dragged {
        return false;
    }
    if h.last.is_none() {
        // Moved before the hold began: a drag (scrubby zoom or rectangle), not a hold.
        if pos.is_some_and(|p| p.distance(h.anchor) > SLOP) {
            h.dragged = true;
            ctx.data_mut(|d| d.insert_temp(id, h));
            return false;
        }
        let begin = h.t0 + DELAY;
        if now < begin {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((begin - now).clamp(0.0, DELAY)));
            return false;
        }
        h.last = Some(begin);
    }
    let dt = now - h.last.unwrap_or(now);
    let zoom = hold_zoom(view.zoom, dt, out, view.doc_size);
    // At the zoom limit the view stays as it is (a window wrapped around the image centred it).
    if zoom.to_bits() != view.zoom.to_bits() {
        crate::zoom_tool::anchor_view(view, xf, h.anchor, h.doc, zoom, ppp);
    }
    h.last = Some(now);
    ctx.data_mut(|d| d.insert_temp(id, h));
    ctx.request_repaint();
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::PhotocraftApp;
    use egui::{Event, PointerButton, vec2};
    use egui_kittest::Harness;
    use serde_json::json;

    const DOC: [u32; 2] = [400, 300];

    /// The hold of the harness's one canvas.
    fn hid() -> egui::Id {
        hold_id(0, true, egui::ViewportId::ROOT)
    }

    #[test]
    fn hold_zoom_is_frame_rate_independent_and_safe() {
        // One second in one step or in 60 steps: two doublings either way.
        let mut z = 1.0;
        for _ in 0..60 {
            z = hold_zoom(z, 1.0 / 60.0, false, DOC);
        }
        let big = hold_zoom(hold_zoom(hold_zoom(hold_zoom(1.0, 0.25, false, DOC), 0.25, false, DOC), 0.25, false, DOC), 0.25, false, DOC);
        assert!((z - 4.0).abs() < 1e-3 && (big - 4.0).abs() < 1e-3, "{z} {big}");
        assert!((hold_zoom(1.0, 0.25, true, DOC) - 2f32.powf(-0.5)).abs() < 1e-5);
        // Nothing for no time, a negative or broken dt, or a non-finite zoom (100 %).
        assert_eq!(hold_zoom(1.5, 0.0, false, DOC), 1.5);
        assert_eq!(hold_zoom(1.5, -3.0, false, DOC), 1.5);
        assert_eq!(hold_zoom(1.5, f64::NAN, true, DOC), 1.5);
        assert_eq!(hold_zoom(1.5, f64::INFINITY, true, DOC), 1.5);
        assert_eq!(hold_zoom(f32::NAN, 0.1, false, DOC), 1.0);
        // A stall is capped; the range is kept.
        assert!((hold_zoom(1.0, 10.0, false, DOC) - 2f32.powf(0.5)).abs() < 1e-5);
        assert_eq!(hold_zoom(crate::zoom_levels::MAX, 0.2, false, DOC), crate::zoom_levels::MAX);
        let min = crate::zoom_levels::min(DOC);
        assert_eq!(hold_zoom(min, 0.2, true, DOC), min);
        assert_eq!(hold_zoom(1.0, 0.1, false, [0, 0]), hold_zoom(1.0, 0.1, false, [0, 0]));
    }

    fn harness(animated: bool) -> Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        app.run("prefs.set", json!({"values": {"general.animatedZoom": animated, "tools.zoomClickedPointToCenter": false}})).unwrap();
        app.sync_views();
        app.ui.extras.rulers = false;
        app.ui.tool = crate::state::Tool::Zoom;
        app.ui.tool_options.zoom_scrubby = true;
        // Each step is 0.1 s of egui time.
        let mut h = Harness::builder().with_size(vec2(1000.0, 700.0)).with_step_dt(0.1).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        let view = &mut h.state_mut().ui.views[0];
        view.zoom = 1.0;
        view.center = [200.0, 150.0];
        view.fit_pending = false;
        view.fill_pending = false;
        h.run_steps(2);
        h
    }

    fn xf(h: &Harness<'static, PhotocraftApp>) -> ViewXform {
        ViewXform::active(h.state()).unwrap()
    }

    fn button(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, pressed: bool, alt: bool) {
        let modifiers = egui::Modifiers { alt, ..Default::default() };
        h.event(Event::ModifiersChanged(modifiers));
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers });
        h.step();
    }

    /// Press at `p`, hold still for `frames` steps, release; returns the zoom after every step,
    /// then after the release step and two more frames.
    fn hold(h: &mut Harness<'static, PhotocraftApp>, p: Pos2, frames: usize, alt: bool) -> Vec<f32> {
        h.event(Event::PointerMoved(p));
        h.step();
        button(h, p, true, alt);
        let mut zs = vec![h.state().current_zoom()];
        for _ in 0..frames {
            h.step();
            zs.push(h.state().current_zoom());
        }
        button(h, p, false, alt);
        zs.push(h.state().current_zoom());
        h.run_steps(2);
        zs.push(h.state().current_zoom());
        zs
    }

    #[test]
    fn holding_zooms_in_continuously_about_the_press_point() {
        let mut h = harness(true);
        let p = h.state().last_canvas_rect.center() + vec2(-120.0, 70.0);
        let doc = xf(&h).to_doc(p);
        let zs = hold(&mut h, p, 8, false);
        // Nothing during the delay, then more every frame; the release adds no click step.
        assert_eq!(zs[0], 1.0, "{zs:?}");
        let rising = &zs[4..=8];
        assert!(rising.windows(2).all(|w| w[1] > w[0]), "{zs:?}");
        // The release frame ends it: no click step, nothing more afterwards.
        assert_eq!(zs[10], zs[9], "{zs:?}");
        assert!(zs[9] < zs[8] * 1.2, "{zs:?}");
        // About 0.5 s held past the delay at RATE doublings per second.
        let expected = (RATE * 0.5).exp2();
        assert!((zs[8] - expected).abs() < 0.2, "{zs:?} vs {expected}");
        assert!(xf(&h).to_screen(doc[0] as f32, doc[1] as f32).distance(p) < 0.5);
        assert_eq!(h.state().session.documents()[0].history.past_len(), 0);
    }

    #[test]
    fn alt_hold_zooms_out() {
        let mut h = harness(true);
        let p = h.state().last_canvas_rect.center() + vec2(50.0, -30.0);
        let doc = xf(&h).to_doc(p);
        let zs = hold(&mut h, p, 8, true);
        assert!(zs[4..=8].windows(2).all(|w| w[1] < w[0]), "{zs:?}");
        assert_eq!(zs[10], zs[9], "{zs:?}");
        assert!(zs[9] > zs[8] / 1.2, "{zs:?}");
        assert!(xf(&h).to_screen(doc[0] as f32, doc[1] as f32).distance(p) < 0.5);
    }

    #[test]
    fn a_hold_at_the_zoom_limit_leaves_the_view_alone() {
        // Zoom Resizes Windows centres the image in a window wrapped around it
        // (zoom_window_fit.rs); a hold that can't zoom further must not move it back.
        let mut h = harness(true);
        let p = h.state().last_canvas_rect.center() + vec2(50.0, -30.0);
        h.event(Event::PointerMoved(p));
        h.step();
        button(&mut h, p, true, true);
        let min = crate::zoom_levels::min(DOC);
        for _ in 0..200 {
            if h.state().current_zoom() == min {
                break;
            }
            h.step();
        }
        assert_eq!(h.state().current_zoom(), min);
        h.state_mut().ui.views[0].center = [200.0, 150.0];
        h.run_steps(3);
        assert_eq!(h.state().ui.views[0].center, [200.0, 150.0]);
        button(&mut h, p, false, true);
    }

    #[test]
    fn with_animated_zoom_off_a_hold_is_one_click_step() {
        let mut h = harness(false);
        let p = h.state().last_canvas_rect.center();
        // Shorter than egui's longest click, so the release is a click.
        let zs = hold(&mut h, p, 5, false);
        assert!(zs[..=5].iter().all(|&z| z == 1.0), "{zs:?}");
        assert_eq!(*zs.last().unwrap(), crate::zoom_levels::step(1.0, 1, DOC));
    }

    #[test]
    fn a_quick_click_steps_and_a_drag_still_scrubs() {
        let mut h = harness(true);
        let p = h.state().last_canvas_rect.center();
        let zs = hold(&mut h, p, 0, false);
        assert_eq!(*zs.last().unwrap(), crate::zoom_levels::step(1.0, 1, DOC), "{zs:?}");
        // Scrubby Zoom: a drag zooms by its distance however long it is held, never by time.
        let z0 = h.state().current_zoom();
        h.event(Event::PointerMoved(p));
        h.step();
        button(&mut h, p, true, false);
        h.event(Event::PointerMoved(p + vec2(50.0, 0.0)));
        h.step();
        for _ in 0..6 {
            h.step();
        }
        let expected = crate::zoom_tool::scrub_zoom(z0, 50.0, DOC);
        assert!((h.state().current_zoom() - expected).abs() < 1e-4, "{} vs {expected}", h.state().current_zoom());
        button(&mut h, p + vec2(50.0, 0.0), false, false);
        h.run_steps(2);
        assert!((h.state().current_zoom() - expected).abs() < 1e-4);
        assert!(!active(&h.ctx, hid()));
    }

    #[test]
    fn a_hold_is_only_reused_by_its_own_press_on_its_own_canvas() {
        // The Hold only goes away on a Zoom-tool frame with the button up. When the tool changed
        // before the release, the next Zoom press can be the first such frame: it is a new press.
        let h = harness(true);
        let (xf, view) = (xf(&h), h.state().ui.views[0].clone());
        let p = xf.rect.center();
        let press = |t: f64| egui::RawInput {
            time: Some(t),
            events: vec![Event::PointerMoved(p), Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Default::default() }],
            ..Default::default()
        };
        let elsewhere = p + vec2(-100.0, 30.0);
        let outside = xf.rect.right_bottom() + vec2(50.0, 50.0);
        for stale in [
            Hold { anchor: elsewhere, doc: xf.to_doc(elsewhere), t0: 1.0, last: Some(1.6), dragged: false },
            Hold { anchor: elsewhere, doc: xf.to_doc(elsewhere), t0: 1.0, last: None, dragged: true },
            // Same point, earlier press.
            Hold { anchor: p, doc: xf.to_doc(p), t0: 1.0, last: Some(1.6), dragged: false },
            // A hold on another (tiled) canvas.
            Hold { anchor: outside, doc: [0.0, 0.0], t0: 10.0, last: Some(10.0), dragged: false },
        ] {
            let ctx = egui::Context::default();
            ctx.data_mut(|d| d.insert_temp(hid(), stale));
            let mut v = view.clone();
            ctx.begin_pass(press(10.0));
            let zooming = update(&ctx, &mut v, &xf, true, false, 1.0, true, hid());
            ctx.end_pass().textures_delta.clear();
            assert!(!zooming, "{stale:?}");
            assert_eq!(v.zoom, view.zoom, "{stale:?}");
            let fresh = ctx.data(|d| d.get_temp::<Hold>(hid()));
            assert_eq!(fresh.map(|h| (h.anchor, h.last, h.dragged)), Some((p, None, false)), "{stale:?}");
        }
        // Tiled windows: canvas 0's hold zooms canvas 0 only. Canvas 1 (which doesn't own the
        // press) neither picks it up nor ends it.
        let ctx = egui::Context::default();
        let ours = Hold { anchor: p, doc: xf.to_doc(p), t0: 10.0, last: Some(10.0), dragged: false };
        ctx.data_mut(|d| d.insert_temp(hid(), ours));
        let mut v = view.clone();
        ctx.begin_pass(press(10.0));
        let other = hold_id(1, true, egui::ViewportId::ROOT);
        assert!(!update(&ctx, &mut v, &xf, true, false, 1.0, false, other));
        ctx.end_pass().textures_delta.clear();
        assert_eq!(v.zoom, view.zoom);
        assert_eq!(ctx.data(|d| d.get_temp::<Hold>(hid())), Some(ours));
    }

    #[test]
    fn a_press_the_canvas_does_not_own_never_becomes_a_hold() {
        let mut h = harness(true);
        crate::dialogs::open_command_dialog(h.state_mut(), "image.adjustments.brightnessContrast", "Brightness/Contrast…");
        h.run_steps(3);
        assert_eq!(h.state().ui.dialogs.len(), 1);
        let p = h.state().last_canvas_rect.center() + vec2(-120.0, 70.0);
        let zs = hold(&mut h, p, 8, false);
        assert!(zs.iter().all(|&z| z == 1.0), "{zs:?}");
        assert!(!active(&h.ctx, hid()));
    }
}
