//! Photoshop's page keys scroll the canvas (#8057): Page Up and Page Down a screen up or down,
//! ⌘/Ctrl turns them sideways (⌘ Page Up left, ⌘ Page Down right), and ⇧ makes the step ten
//! screen points. Home and End show the document's top-left and bottom-right corners. The zoom
//! stays as it is. A binding of the user's on the same keys has already run and taken them.

use egui::{Event, Key, Modifiers, Vec2, vec2};

use crate::PhotocraftApp;

/// Screen points a ⇧ page key scrolls.
pub const SMALL_STEP: f32 = 10.0;

/// What a page key does to the view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum PageKey {
    /// Scroll by this many screen points (positive: show what is below or to the right).
    Scroll(Vec2),
    /// Show the document's top-left corner (Home) or its bottom-right one (End).
    Corner { end: bool },
}

/// The page key `key` with `mods` held, in a canvas of `size` points. `None` for other keys, ⌥,
/// or ⇧ on Home and End.
pub fn page_key(key: Key, mods: Modifiers, size: Vec2) -> Option<PageKey> {
    if mods.alt {
        return None;
    }
    let sign = match key {
        Key::PageUp => -1.0,
        Key::PageDown => 1.0,
        Key::Home | Key::End if !mods.shift && !mods.command => return Some(PageKey::Corner { end: key == Key::End }),
        _ => return None,
    };
    let sideways = mods.command;
    let step = match (mods.shift, sideways) {
        (true, _) => SMALL_STEP,
        (false, true) => size.x,
        (false, false) => size.y,
    };
    Some(PageKey::Scroll(if sideways { vec2(sign * step, 0.0) } else { vec2(0.0, sign * step) }))
}

/// Consumes this frame's page keys and moves the active document's view. True when one ran.
pub fn handle(app: &mut PhotocraftApp, ctx: &egui::Context) -> bool {
    let Some(i) = app.session.active_index() else { return false };
    let rect = crate::rulers::content_rect(app, app.last_canvas_rect);
    let size = rect.size();
    let presses: Vec<PageKey> = ctx.input_mut(|input| {
        let mut out = Vec::new();
        input.events.retain(|e| match e {
            Event::Key { key, pressed: true, modifiers, .. } => match page_key(*key, *modifiers, size) {
                Some(p) => {
                    out.push(p);
                    false
                }
                None => true,
            },
            _ => true,
        });
        out
    });
    if presses.is_empty() {
        return false;
    }
    app.sync_views();
    let Some(doc_size) = app.session.active().map(|st| [st.doc.size.width as f32, st.doc.size.height as f32]) else { return false };
    let (ppp, aspect, flip, overscroll) = (app.ppp, app.ui.view.display_aspect(), app.ui.view.flip_horizontal, app.session.prefs().tools.overscroll);
    let ppp = if ppp.is_finite() && ppp > 0.0 { ppp } else { 1.0 };
    let Some(view) = app.ui.views.get_mut(i) else { return false };
    let zoom = view.zoom / ppp;
    if !zoom.is_finite() || zoom <= 0.0 {
        return false;
    }
    view.fit_pending = false;
    view.fill_pending = false;
    for p in presses {
        match p {
            PageKey::Scroll(d) => {
                let xf = crate::canvas::ViewXform { rect, zoom, center: view.center, flip, rotation: view.rotation, aspect };
                let d = xf.unmap_vec(d) / zoom;
                if d.is_finite() {
                    view.center[0] += d.x;
                    view.center[1] += d.y;
                }
            }
            PageKey::Corner { end } => {
                // The document corner goes to the canvas corner the document runs on from: the
                // top-left one in an upright view, another when it is rotated or flipped.
                let xf = crate::canvas::ViewXform { rect, zoom, center: view.center, flip, rotation: view.rotation, aspect };
                let [w, h] = doc_size;
                let (corner, inward) = if end { ([w, h], vec2(-1.0, -1.0)) } else { ([0.0, 0.0], vec2(1.0, 1.0)) };
                let dir = xf.map_vec(inward);
                let at = vec2(-dir.x.signum() * size.x / 2.0, -dir.y.signum() * size.y / 2.0);
                let off = xf.unmap_vec(at) / zoom;
                if off.is_finite() {
                    view.center = [corner[0] - off.x, corner[1] - off.y];
                }
            }
        }
    }
    // Overscroll off: the view stops at the edge, as the scrollbars do.
    if !overscroll {
        crate::scrollbars::clamp_view_with_aspect(view, size, ppp, aspect);
    }
    ctx.request_repaint();
    true
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn page_keys_map_to_photoshops_scrolls() {
        let size = vec2(800.0, 600.0);
        let (none, shift, cmd) = (Modifiers::NONE, Modifiers::SHIFT, Modifiers::COMMAND);
        assert_eq!(page_key(Key::PageDown, none, size), Some(PageKey::Scroll(vec2(0.0, 600.0))));
        assert_eq!(page_key(Key::PageUp, none, size), Some(PageKey::Scroll(vec2(0.0, -600.0))));
        assert_eq!(page_key(Key::PageDown, shift, size), Some(PageKey::Scroll(vec2(0.0, SMALL_STEP))));
        assert_eq!(page_key(Key::PageUp, cmd, size), Some(PageKey::Scroll(vec2(-800.0, 0.0))));
        assert_eq!(page_key(Key::PageDown, cmd | shift, size), Some(PageKey::Scroll(vec2(SMALL_STEP, 0.0))));
        assert_eq!(page_key(Key::Home, none, size), Some(PageKey::Corner { end: false }));
        assert_eq!(page_key(Key::End, none, size), Some(PageKey::Corner { end: true }));
        assert_eq!(page_key(Key::PageDown, Modifiers::ALT, size), None);
        assert_eq!(page_key(Key::Home, shift, size), None);
        assert_eq!(page_key(Key::ArrowDown, none, size), None);
    }

    /// #8057: through the app's key handling, Page Down scrolls a zoomed-in document a screen
    /// down at the same zoom, ⇧ a small step, ⌘ sideways; Home and End jump to the corners.
    #[test]
    fn page_keys_scroll_the_zoomed_canvas() {
        let ctx = egui::Context::default();
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 2000, "height": 2000})).unwrap();
        app.sync_views();
        app.ppp = 1.0;
        let v = &mut app.ui.views[0];
        (v.zoom, v.center, v.fit_pending, v.fill_pending) = (4.0, [1000.0, 1000.0], false, false);
        let size = crate::rulers::content_rect(&app, app.last_canvas_rect).size();
        let press = |app: &mut PhotocraftApp, key: Key, modifiers: Modifiers| {
            let raw = egui::RawInput {
                events: vec![Event::ModifiersChanged(modifiers), Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers }],
                ..Default::default()
            };
            ctx.begin_pass(raw);
            crate::shortcuts::handle(app, &ctx);
            ctx.end_pass().textures_delta.clear();
            app.ui.views[0].center
        };
        let near = |a: [f32; 2], b: [f32; 2]| (a[0] - b[0]).abs() < 1e-3 && (a[1] - b[1]).abs() < 1e-3;
        let c = press(&mut app, Key::PageDown, Modifiers::NONE);
        assert!(near(c, [1000.0, 1000.0 + size.y / 4.0]), "a screen down: {c:?}");
        assert_eq!(app.ui.views[0].zoom, 4.0, "the zoom stays");
        let c = press(&mut app, Key::PageUp, Modifiers::NONE);
        assert!(near(c, [1000.0, 1000.0]), "and back: {c:?}");
        let c = press(&mut app, Key::PageDown, Modifiers::SHIFT);
        assert!(near(c, [1000.0, 1000.0 + SMALL_STEP / 4.0]), "a small step: {c:?}");
        let c = press(&mut app, Key::PageUp, Modifiers::COMMAND);
        assert!(near(c, [1000.0 - size.x / 4.0, 1000.0 + SMALL_STEP / 4.0]), "a screen left: {c:?}");
        let c = press(&mut app, Key::Home, Modifiers::NONE);
        assert!(near(c, [size.x / 8.0, size.y / 8.0]), "the top-left corner: {c:?}");
        let c = press(&mut app, Key::End, Modifiers::NONE);
        assert!(near(c, [2000.0 - size.x / 8.0, 2000.0 - size.y / 8.0]), "the bottom-right corner: {c:?}");

        // Rotated a quarter turn or flipped, the corner still lands on a canvas corner with the
        // document running into the canvas from it.
        let rect = crate::rulers::content_rect(&app, app.last_canvas_rect);
        for (rotation, flip) in [(90.0, false), (0.0, true), (-90.0, true)] {
            app.ui.views[0].rotation = rotation;
            app.ui.view.flip_horizontal = flip;
            for (key, corner, inward) in [(Key::Home, [0.0, 0.0], 1.0), (Key::End, [2000.0, 2000.0], -1.0)] {
                let center = press(&mut app, key, Modifiers::NONE);
                let xf = crate::canvas::ViewXform { rect, zoom: 4.0, center, flip, rotation, aspect: 1.0 };
                let at = xf.to_screen(corner[0], corner[1]);
                let on_corner = [rect.left_top(), rect.right_top(), rect.left_bottom(), rect.right_bottom()].iter().any(|c| c.distance(at) < 1e-2);
                assert!(on_corner, "{key:?} at {rotation}° flip {flip}: {at:?} in {rect:?}");
                assert!(
                    rect.contains(xf.to_screen(corner[0] + inward, corner[1] + inward)),
                    "{key:?} at {rotation}° flip {flip}: the document runs into the canvas"
                );
            }
        }
    }
}
