//! #12113: with Painting Cursors = Normal Brush Tip, holding ⌥ with the Healing Brush shows the
//! reference's fixed target crosshair (19 × 19, hotspot at its centre) whatever the tip size, and
//! releasing ⌥ brings the tip outline back. The Clone Stamp keeps its tip circle with a centre
//! mark, and the unmeasured Painting Cursors settings keep today's cursors.

use egui::{Modifiers, PointerButton};
use egui_kittest::Harness;
use serde_json::json;

use super::mixer_cursor_tests::{Bitmap, assert_centred, bitmap, centre, harness, has_fonts, hotspot_drawn, hover};
use crate::PhotocraftApp;
use crate::state::Tool;

/// The first column and row of the bitmap's visible (non-transparent) pixels, and their width and
/// height.
fn visible_box((size, _, alpha): &Bitmap) -> ([usize; 2], [usize; 2]) {
    let w = usize::from(size[0]);
    let (mut lo, mut hi) = ([usize::MAX; 2], [0; 2]);
    for (i, _) in alpha.iter().enumerate().filter(|(_, a)| **a > 0) {
        for (k, v) in [i % w, i / w].into_iter().enumerate() {
            lo[k] = lo[k].min(v);
            hi[k] = hi[k].max(v);
        }
    }
    (lo, [0, 1].map(|k| if lo[k] > hi[k] { 0 } else { hi[k] - lo[k] + 1 }))
}

/// The width and height of the bitmap's visible pixels.
fn visible(b: &Bitmap) -> [usize; 2] {
    visible_box(b).1
}

/// Asserts `b` is the target: 19 × 19 visible points, centred, its cross through the hotspot.
fn assert_target(h: &Harness<'static, PhotocraftApp>, b: &Bitmap, what: &str) {
    let side = 19.0 * crate::tool_cursor::native_scale(&h.ctx);
    assert_centred(b.0, b.1, what);
    let (lo, seen) = visible_box(b);
    let [w, hh] = seen;
    assert!((w as f32 - side).abs() <= 1.0 && (hh as f32 - side).abs() <= 1.0, "{what}: a 19 × 19 target, not {w} × {hh}");
    // The reference's hotspot 9,9 in its 19 × 19 bitmap: the centre of the visible target.
    for k in 0..2 {
        let at = usize::from(b.1[k]) - lo[k];
        assert!(at.abs_diff((seen[k] - 1) / 2) <= 1, "{what}: hotspot {at} px into the {} px target", seen[k]);
    }
    assert!(hotspot_drawn(b), "{what}: the cross runs through the hotspot");
}

fn tip(h: &mut Harness<'static, PhotocraftApp>, size: f32) {
    h.state_mut().session.tools.brush.size = size;
}

fn painting(h: &mut Harness<'static, PhotocraftApp>, value: &str) {
    h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": value})).unwrap();
}

#[test]
fn alt_shows_the_fixed_target_at_any_tip_size_and_releasing_it_brings_the_tip_back() {
    let mut h = harness(Tool::Healing);
    if !has_fonts(&h) {
        return;
    }
    let mut target: Option<Bitmap> = None;
    // 200 px is the largest of these that every platform still draws as an outline bitmap
    // (Linux caps cursor bitmaps at 255 px a side).
    for size in [10.0, 40.0, 200.0] {
        tip(&mut h, size);
        hover(&mut h, Modifiers::ALT);
        let alt = bitmap(&h).expect("the target is an OS cursor bitmap");
        assert_target(&h, &alt, &format!("{size} px tip"));
        match &target {
            Some(t) => assert_eq!(&alt, t, "{size} px tip: the same target as the smaller tips"),
            None => target = Some(alt.clone()),
        }
        hover(&mut h, Modifiers::NONE);
        let released = bitmap(&h).expect("the tip outline is an OS cursor bitmap");
        assert_ne!(released, alt, "{size} px tip: releasing ⌥ brings the tip outline back");
        assert!(!hotspot_drawn(&released), "{size} px tip: the plain outline has no centre mark");
    }
    tip(&mut h, 200.0);
    hover(&mut h, Modifiers::NONE);
    let outline = bitmap(&h).expect("the tip outline");
    assert!(visible(&outline)[0] > 4 * 19, "the 200 px tip's outline, not the target: {:?}", visible(&outline));
}

#[test]
fn sticky_alt_shows_the_target_too() {
    let mut h = harness(Tool::Healing);
    if !has_fonts(&h) {
        return;
    }
    tip(&mut h, 120.0);
    h.state_mut().ui.shell.sticky_alt = true;
    hover(&mut h, Modifiers::NONE);
    let target = bitmap(&h).expect("the target");
    assert_target(&h, &target, "sticky ⌥");
    h.state_mut().ui.shell.sticky_alt = false;
    hover(&mut h, Modifiers::NONE);
    assert_ne!(bitmap(&h), Some(target), "off again: the tip outline");
}

#[test]
fn the_target_stays_while_an_alt_click_is_held() {
    let mut h = harness(Tool::Healing);
    if !has_fonts(&h) {
        return;
    }
    tip(&mut h, 120.0);
    hover(&mut h, Modifiers::ALT);
    let target = bitmap(&h).expect("the target");
    assert_target(&h, &target, "⌥ hover");
    let p = centre(&h);
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::ALT });
    h.run_steps(2);
    assert_eq!(bitmap(&h), Some(target), "the target stays while the ⌥-click is held");
    h.event(egui::Event::PointerButton { pos: p, button: PointerButton::Primary, pressed: false, modifiers: Modifiers::ALT });
    h.run_steps(2);
}

#[test]
fn the_clone_stamp_keeps_its_tip_circle_with_a_centre_mark() {
    let mut h = harness(Tool::CloneStamp);
    if !has_fonts(&h) {
        return;
    }
    tip(&mut h, 100.0);
    hover(&mut h, Modifiers::NONE);
    let plain = bitmap(&h).expect("the tip circle");
    assert!(!hotspot_drawn(&plain), "no centre mark without ⌥");
    hover(&mut h, Modifiers::ALT);
    let alt = bitmap(&h).expect("the tip circle with ⌥");
    assert_eq!(visible(&alt), visible(&plain), "the tip circle keeps the tip size");
    assert!(hotspot_drawn(&alt), "⌥ adds the centre mark");
}

/// Standard, Precise and Full Size Brush Tip weren't measured with the Healing Brush's ⌥, so ⌥
/// shows what it does today: the standard pointer, the precise crosshair, or the full-size tip
/// with its centre mark.
#[test]
fn unmeasured_painting_cursors_keep_todays_alt_cursor() {
    let mut h = harness(Tool::Healing);
    if !has_fonts(&h) {
        return;
    }
    tip(&mut h, 100.0);
    for value in ["standard", "precise"] {
        painting(&mut h, value);
        hover(&mut h, Modifiers::NONE);
        let plain = (h.output().platform_output.cursor_icon, bitmap(&h));
        hover(&mut h, Modifiers::ALT);
        assert_eq!((h.output().platform_output.cursor_icon, bitmap(&h)), plain, "{value}: ⌥ changes nothing");
    }
    painting(&mut h, "fullSizeTip");
    hover(&mut h, Modifiers::ALT);
    let full = bitmap(&h).expect("the full-size tip");
    assert!(visible(&full)[0] > 4 * 19, "fullSizeTip: the tip circle, {:?}", visible(&full));
    assert!(hotspot_drawn(&full), "fullSizeTip: with its centre mark");
}
