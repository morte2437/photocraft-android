//! #11528: Gaussian Blur's edge profile on an opaque interior edge, pinned at RGB8 and RGB16 and
//! across the direct command and an action (ATN) replay.
//!
//! The issue measures Photoshop 27.11 against PhotoCraft on a black/white step with no selection,
//! mask, transparency or smart object, so nothing but the kernel can explain the difference:
//! at radius 2.3 the reported red levels for x = 10..22 are
//!
//! ```text
//! Photoshop   0, 2,10,27,59,103,152,196,228,245,253,255,255
//! PhotoCraft  2, 6,16,35,65,105,150,190,220,239,249,253,255
//! ```
//!
//! PhotoCraft's row is what a Gaussian sampled at whole pixels with sigma = radius and a 3-sigma
//! window gives (its variance is 0.6 % short of 2.3^2 from that truncation), so the pipeline adds
//! no error of its own: the gap is the two applications' radius-to-blur mapping, 8 levels at
//! radius 2.3, 5 at radius 1 and 1 at radius 5 (where PhotoCraft already runs box passes). No
//! single factor maps one to the other, as the issue itself notes, so the kernel is deliberately
//! left alone here; what this file does is pin the profile that PhotoCraft produces today, at both
//! depths and on both routes, and ratchet the distance to Photoshop's measured row so a future
//! kernel change is a decision with a number attached to it instead of a silent drift.

use craft_psd::atn::{self, AtnAction, AtnEvent, AtnFile, AtnItem};
use craft_psd::descriptor::{Descriptor, UnicodeString, Value as D};
use photocraft_engine::{Session, atn_cmds};
use serde_json::json;

/// The issue's fixture: 33 px wide, black at x = 0..15, white from x = 16.
const WIDTH: i32 = 33;
/// The step is at x = 16; seven pixels of kernel reach keep x = 10..22 clear of both canvas ends.
const FROM: i32 = 10;
const TO: i32 = 22;
/// PhotoCraft 0.20.2 (`7c12f6471`) as published in #11528, and Photoshop 27.11 beside it.
const PHOTOCRAFT: [u32; 13] = [2, 6, 16, 35, 65, 105, 150, 190, 220, 239, 249, 253, 255];
const PHOTOSHOP: [u32; 13] = [0, 2, 10, 27, 59, 103, 152, 196, 228, 245, 253, 255, 255];
/// The measured gap at radius 2.3. May only shrink, never grow: raising it needs the new number
/// and the reason in #11528.
const GAP: i64 = 8;
/// The same blur on the same fixture at 16 bits; the issue's (13, 0) is 8929 against Photoshop's
/// 7012, a gap of 1917 of 65535 - the same eight-level distance in the other scale.
const PHOTOCRAFT_16: [u32; 13] = [490, 1561, 4069, 8929, 16726, 27078, 38457, 48809, 56606, 61466, 63974, 65045, 65424];
const PHOTOSHOP_16_AT_13: u32 = 7012;
const GAP_16: i64 = 1917;

fn edge_doc(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": WIDTH, "height": 1, "depth": depth, "background": "#000000"})).unwrap();
    s.execute("select.rect", json!({"x": 16, "y": 0, "width": WIDTH - 16, "height": 1})).unwrap();
    s.execute("edit.fill", json!({"contents": "color", "color": "#ffffff"})).unwrap();
    s.execute("select.deselect", json!({})).unwrap();
    s
}

/// The red levels across the edge, in the document's own sample values.
fn profile(s: &mut Session, scale: f32) -> Vec<u32> {
    (FROM..=TO)
        .map(|x| {
            let px: Vec<f32> = serde_json::from_value(s.execute("document.pixel", json!({"x": x, "y": 0})).unwrap()).unwrap();
            (px[0] * scale + 0.5).floor() as u32
        })
        .collect()
}

fn blurred(depth: u32) -> (Vec<u32>, Vec<u32>) {
    let mut unblurred = edge_doc(depth);
    let before = profile(&mut unblurred, 255.0);
    let mut s = edge_doc(depth);
    s.execute("filter.blur.gaussianBlur", json!({"radius": 2.3})).unwrap();
    (before, profile(&mut s, if depth == 8 { 255.0 } else { 65535.0 }))
}

/// The action the issue plays: Photoshop's `GsnB` event with its `#Pxl` radius descriptor.
fn gaussian_recording() -> Vec<u8> {
    atn::write(&AtnFile {
        actions: vec![AtnAction {
            name: UnicodeString::new_nul("Blur2_3"),
            items: vec![AtnItem {
                event: AtnEvent::Code(*b"GsnB"),
                descriptor: Some(Descriptor::new("null").with("Rds ", D::UnitFloat { unit: *b"#Pxl", value: 2.3 })),
                ..Default::default()
            }],
            ..Default::default()
        }],
        ..Default::default()
    })
    .unwrap()
}

#[test]
fn edge_profile_at_radius_2_3_is_the_sampled_sigma_equals_radius_kernel_at_eight_bits() {
    let (before, got) = blurred(8);
    assert_eq!(before, [vec![0; 6], vec![255; 7]].concat(), "the unblurred fixture is the step itself");
    assert_eq!(got, PHOTOCRAFT, "the 8-bit profile across the edge changed");
    let gap = got.iter().zip(PHOTOSHOP).map(|(a, b)| (*a as i64 - b as i64).abs()).max().unwrap();
    assert!(gap <= GAP, "{gap} levels from Photoshop's row in #11528, was {GAP}");
}

/// 16 bits must be the same curve rather than a second rounding decision, and it carries the
/// issue's own 16-bit measurement (8929 at (13, 0) against Photoshop's 7012).
#[test]
fn edge_profile_at_radius_2_3_at_sixteen_bits_is_the_same_curve_in_another_scale() {
    let (_, got) = blurred(16);
    assert_eq!(got, PHOTOCRAFT_16, "the 16-bit profile across the edge changed");
    let gap = (got[3] as i64 - PHOTOSHOP_16_AT_13 as i64).abs();
    assert!(gap <= GAP_16, "{gap} levels from Photoshop's 16-bit row in #11528, was {GAP_16}");
    for (a, b) in PHOTOCRAFT.iter().zip(&got) {
        assert!((*a as i64 * 257 - *b as i64).abs() <= 257, "{a} at 8 bits is {b} at 16 bits: more than one level apart");
    }
}

/// #11528 also checks Photoshop's direct blur against its own action replay; PhotoCraft's two
/// routes must stay identical, at both depths, or the action path hides a second implementation.
#[test]
fn gaussian_recording_replays_the_direct_blur_pixels_and_radius() {
    let imported = atn_cmds::import(&gaussian_recording()).unwrap();
    assert!(imported.unsupported.is_empty());
    assert_eq!(imported.actions[0].steps[0].0, "filter.blur.gaussianBlur");
    assert_eq!(imported.actions[0].steps[0].1["radius"], 2.3, "the recorded radius must survive import");
    for depth in [8u32, 16] {
        let mut direct = edge_doc(depth);
        direct.execute("filter.blur.gaussianBlur", json!({"radius": 2.3})).unwrap();
        let mut s = edge_doc(depth);
        s.actions.list = imported.actions.clone();
        assert_eq!(s.execute("actions.play", json!({"action": "Blur2_3"})).unwrap()["ran"], 1);
        assert_eq!(
            photocraft_compose::flatten(&s.active().unwrap().doc).px,
            photocraft_compose::flatten(&direct.active().unwrap().doc).px,
            "{depth}-bit: the recording blurs differently from the command"
        );
    }
}
