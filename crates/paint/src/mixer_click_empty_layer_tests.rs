//! A single wet Mixer Brush click on an empty layer leaves a full-strength dab, as a dry click
//! does: picking up from a transparent layer does not thin the paint (#11944). Measured with a
//! 35 px soft round tip, a black load, Wet 80 / Load 75 / Mix 90 and Flow 100, one click at
//! 182,47: a full black dab with layer bounds 153,18–210,75.

use crate::mixer::{MixerSettings, MixerState, apply_mixer_stroke};
use crate::{BrushSettings, Stroke, StrokePoint};
use photocraft_color::PixelFormat;
use photocraft_raster::Surface;

fn click(s: &mut Surface, wet: f32) {
    click_with(s, wet, 0.9, 1.0);
}

fn click_with(s: &mut Surface, wet: f32, mix: f32, flow: f32) {
    let stroke = Stroke {
        brush: BrushSettings { size: 35.0, hardness: 0.0, spacing: 0.25, pressure_size: false, ..Default::default() },
        points: vec![StrokePoint::new(182.0, 47.0, 1.0)],
    };
    let mut st = MixerState::default();
    st.load([0.0, 0.0, 0.0, 1.0]);
    let m = MixerSettings { wet, load: 0.75, mix, flow, ..Default::default() };
    apply_mixer_stroke(s, None, &stroke, &m, &mut st, None, false).unwrap();
}

fn alpha(s: &Surface, x: i32, y: i32) -> i32 {
    (s.rgba(x, y)[3] * 255.0).round() as i32
}

/// The bounds of the pixels with any alpha, as x0, y0, x1, y1 (exclusive).
fn bounds(s: &Surface) -> (i32, i32, i32, i32) {
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for y in 0..120 {
        for x in 120..260 {
            if alpha(s, x, y) > 0 {
                (x0, y0, x1, y1) = (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1));
            }
        }
    }
    (x0, y0, x1, y1)
}

#[test]
fn a_wet_click_on_an_empty_layer_is_a_full_strength_dab() {
    for wet in [0.8, 0.0] {
        let mut s = Surface::new(PixelFormat::RGBA8);
        click(&mut s, wet);
        assert!(alpha(&s, 182, 47) >= 250, "Wet {wet}: centre alpha {}", alpha(&s, 182, 47));
        let c = s.rgba(182, 47);
        assert!(c[..3].iter().all(|v| *v < 0.02), "Wet {wet}: the dab is black: {c:?}");
        let (x0, y0, x1, y1) = bounds(&s);
        for (got, want) in [(x0, 153), (y0, 18), (x1, 210), (y1, 75)] {
            assert!((got - want).abs() <= 2, "Wet {wet}: bounds {:?}, expected near 153,18-210,75", (x0, y0, x1, y1));
        }
    }
}

#[test]
fn a_wet_click_on_a_colour_still_mixes_it_in() {
    let mut s = Surface::new(PixelFormat::RGBA8);
    s.fill_rect(photocraft_geom::Rect::new(0, 0, 400, 120), &[1.0, 0.0, 0.0, 1.0]);
    click(&mut s, 0.8);
    let c = s.rgba(182, 47);
    assert!(c[0] > 0.3, "the red under the click is mixed in: {c:?}");
    assert!(c[0] < 0.99, "and the black load darkens it: {c:?}");
    assert!(c[1] < 0.02 && c[2] < 0.02 && c[3] > 0.99, "only red and black, opaque: {c:?}");
}

/// On an empty layer Wet and Mix change nothing: the dab is a dry dab at the same Flow, so its
/// centre is about 251 × Flow / 100. davidmann4 measured 126 at Flow 50 and 251 at Flow 100,
/// at Wet 80 with Mix 50 and with Mix 90 (#11944).
#[test]
fn at_flow_50_an_empty_layer_dab_is_half_the_flow_100_dab_at_any_mix() {
    for mix in [0.5, 0.9] {
        let centre = |flow| {
            let mut s = Surface::new(PixelFormat::RGBA8);
            click_with(&mut s, 0.8, mix, flow);
            alpha(&s, 182, 47)
        };
        let (full, half) = (centre(1.0), centre(0.5));
        assert!((half - 126).abs() <= 3, "Mix {mix}, Flow 50: centre alpha {half}, measured 126");
        assert!(full >= 250, "Mix {mix}, Flow 100: centre alpha {full}, measured 251");
        assert!((2 * half - full).abs() <= 6, "Mix {mix}: Flow 50 centre {half}, expected about half of {full}");
    }
}
