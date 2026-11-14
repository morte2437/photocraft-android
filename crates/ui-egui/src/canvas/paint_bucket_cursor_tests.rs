//! The Paint Bucket cursor (#12539): with Standard Other Cursors it is the bucket with a small
//! arrow at its top left, the hotspot on the arrow tip, and a click fills the pixel under that tip.
//! Precise Other Cursors and Caps Lock keep the crosshair; ⌥ and ⌘ keep the Eyedropper and Move
//! cursors. Tests drive the real canvas (`document_area`) and read the OS cursor bitmap it sets.

use egui::{CursorIcon, Modifiers};
use egui_kittest::Harness;
use serde_json::json;

use super::remove_cursor_tests::{Cursor, cursor, harness, set_pref};
use crate::PhotocraftApp;
use crate::state::Tool;

/// The cursor with `mods` held over the canvas centre.
fn cursor_with(h: &mut Harness<'_, PhotocraftApp>, mods: Modifiers) -> Cursor {
    let p = h.state().last_canvas_rect.center();
    h.event(egui::Event::ModifiersChanged(mods));
    h.event(egui::Event::PointerMoved(p));
    h.run_steps(3);
    cursor(h)
}

fn pixel(app: &PhotocraftApp, x: i32, y: i32) -> [u8; 3] {
    let st = app.session.active().unwrap();
    let c = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().rgba(x, y);
    [0, 1, 2].map(|i| (c[i] * 255.0).round() as u8)
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn the_standard_cursor_is_the_bucket_with_the_arrow_tip_on_the_hotspot() {
    let h = harness(Tool::PaintBucket);
    let (icon, image) = cursor(&h);
    assert_eq!(icon, CursorIcon::Crosshair, "the OS bitmap keeps its visible fallback");
    let (size, [x, y], rgba) = image.expect("the Paint Bucket's OS cursor bitmap");
    assert_eq!([x, y], [1, 1], "the hotspot is the arrow tip at the bitmap's top left");
    let at = |x: u16, y: u16| {
        let i = (usize::from(y) * usize::from(size[0]) + usize::from(x)) * 4;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    };
    // The tip sits on the hotspot texel's centre, so that pixel is antialiased, not solid.
    assert!(at(x, y)[3] > 128, "the tip itself is drawn: no open crosshair centre under the pointer");
    let body = at(x + 1, y + 4);
    assert!(body[3] > 230 && body[0] < 40, "a black arrow runs down from the tip: {body:?}");
    let bucket = (0..size[1]).flat_map(|row| (0..size[0]).map(move |col| (col, row))).any(|(col, row)| {
        let p = at(col, row);
        col > x + 10 && row > y + 10 && p[0] > 200 && p[3] > 200
    });
    assert!(bucket, "the white bucket art sits below right of the arrow");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_click_fills_the_pixel_under_the_arrow_tip() {
    let mut h = harness(Tool::PaintBucket);
    let p = h.state().last_canvas_rect.center();
    let (x, y) = crate::panels::info::hover_pixel(h.state()).expect("the pointer is over the image");
    // Black left of and above the pointer's pixel: a fill there stops at the edges, a fill one
    // pixel off to the left or up would turn the black red.
    let app = h.state_mut();
    for rect in [[0, 0, x, 300], [0, 0, 400, y]] {
        app.run("select.rect", json!({"x": rect[0], "y": rect[1], "width": rect[2], "height": rect[3]})).unwrap();
        app.run("edit.fill", json!({"contents": "color", "color": "#000000"})).unwrap();
    }
    app.run("select.deselect", json!({})).unwrap();
    app.run("tools.setColors", json!({"foreground": "#ff0000"})).unwrap();
    h.run_steps(2);
    for pressed in [true, false] {
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: Modifiers::NONE });
        h.run_steps(2);
    }
    assert_eq!(pixel(h.state(), x, y), [255, 0, 0], "the pixel under the tip is filled");
    assert_eq!(pixel(h.state(), x - 1, y), [0, 0, 0], "the pixel left of it is not");
    assert_eq!(pixel(h.state(), x, y - 1), [0, 0, 0], "the pixel above it is not");
}

#[test]
fn precise_other_cursors_and_caps_lock_keep_the_crosshair() {
    let mut h = harness(Tool::PaintBucket);
    let standard = cursor(&h);
    set_pref(&mut h, "cursors.other", json!("precise"));
    let precise = cursor(&h);
    assert_eq!(precise.0, CursorIcon::Crosshair, "Precise: the crosshair");
    assert_ne!(precise, standard, "Precise: not the bucket");
    set_pref(&mut h, "cursors.other", json!("standard"));
    h.state_mut().caps_lock = true;
    h.run_steps(2);
    assert_eq!(cursor(&h), precise, "Caps Lock: the same crosshair");
}

#[test]
fn alt_and_command_keep_the_eyedropper_and_move_cursors() {
    let mut bucket = harness(Tool::PaintBucket);
    let standard = cursor(&bucket);
    let eyedropper = cursor(&harness(Tool::Eyedropper));
    let alt = cursor_with(&mut bucket, Modifiers::ALT);
    assert_eq!(alt, eyedropper, "⌥: the Eyedropper cursor");
    assert_ne!(alt, standard);
    let mv = cursor_with(&mut harness(Tool::Move), Modifiers::NONE);
    let command = cursor_with(&mut bucket, Modifiers::COMMAND);
    assert_eq!(command, mv, "⌘: the Move cursor");
    assert_ne!(command, standard);
}
