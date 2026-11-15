//! #11922: in a Lab document, Color mode keeps the pixel's L and takes the foreground's a and b.
//! The setup is the issue's: red, green and pink stripes and the darker block (200,40,40), a blue
//! foreground, a 40 px hard tip, one click on each. Hue, Saturation and Luminosity keep today's RGB
//! blend.

use photocraft_color::{ColorMode, PixelFormat, SampleType};

use super::*;
use crate::{BrushSettings, StrokePoint};

const LAB8: PixelFormat = PixelFormat { mode: ColorMode::Lab, sample: SampleType::U8, alpha: true };
const COLOURS: [[u8; 3]; 4] = [[220, 30, 30], [30, 180, 60], [240, 120, 120], [200, 40, 40]];
const BLUE: [f32; 4] = [0.0, 0.0, 1.0, 1.0];

/// A Lab/8 surface with the four colours in 100 px columns.
fn target() -> Surface {
    target_in(LAB8)
}

fn target_in(fmt: PixelFormat) -> Surface {
    let mut s = Surface::new(fmt);
    for (i, c) in COLOURS.iter().enumerate() {
        let rgba = [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0];
        let x0 = i as i32 * 100;
        s.fill_rect(Rect::new(x0, 0, x0 + 100, 100), &photocraft_raster::from_rgba(&fmt, rgba));
    }
    s
}

fn click(s: &mut Surface, mode: ReplaceMode, x: f64) {
    let stroke = Stroke {
        brush: BrushSettings { size: 40.0, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
        points: vec![StrokePoint::new(x, 50.0, 1.0)],
    };
    let rs = ReplaceSettings { mode, color: BLUE, ..Default::default() };
    apply_color_replacement(s, &stroke, &rs, None, false).unwrap();
}

/// The stored L (0..100) and a, b (-128..127) at a pixel.
fn lab(s: &Surface, x: i32, y: i32) -> [f32; 3] {
    let px = s.pixel(x, y);
    [px[0] * 100.0, px[1] * 255.0 - 128.0, px[2] * 255.0 - 128.0]
}

#[test]
fn color_keeps_l_and_takes_the_foregrounds_a_and_b() {
    let fg = photocraft_color::convert::srgb_to_lab([0.0, 0.0, 1.0]);
    for i in 0..COLOURS.len() {
        let mut s = target();
        let x = i as i32 * 100 + 50;
        let before = lab(&s, x, 50);
        click(&mut s, ReplaceMode::Color, f64::from(x) + 0.5);
        let after = lab(&s, x, 50);
        assert!((after[0] - before[0]).abs() <= 1.0, "colour {i}: L {} → {}", before[0], after[0]);
        assert!((after[1] - fg[1]).abs() <= 2.0 && (after[2] - fg[2]).abs() <= 2.0, "colour {i}: a, b {:?} vs the foreground's {:?}", &after[1..], &fg[1..]);
        // Outside the tip nothing changed.
        assert_eq!(lab(&s, x - 40, 50), before, "colour {i} outside the tip");
    }
}

#[test]
fn hue_saturation_and_luminosity_keep_todays_blend() {
    for mode in [ReplaceMode::Hue, ReplaceMode::Saturation, ReplaceMode::Luminosity] {
        let mut s = target();
        let pre = s.pixel(150, 50).to_vec();
        click(&mut s, mode, 150.5);
        // Today's path: through RGB, blended, and back.
        let blend = match mode {
            ReplaceMode::Hue => BlendMode::Hue,
            ReplaceMode::Saturation => BlendMode::Saturation,
            _ => BlendMode::Luminosity,
        };
        let c = to_rgba(&LAB8, &pre);
        let b = blend_rgb(blend, [c[0], c[1], c[2]], [0.0, 0.0, 1.0]);
        let want = photocraft_raster::from_rgba(&LAB8, [b[0], b[1], b[2], c[3]]);
        let got = s.pixel(150, 50);
        for (g, w) in got.iter().zip(&want) {
            assert!((g - w).abs() <= 1.0 / 255.0 + 1e-6, "{mode:?}: {got:?} vs {want:?}");
        }
    }
}

#[test]
fn lab_16_color_keeps_todays_blend() {
    // Lab/16 wasn't measured: its Color mode still blends through RGB.
    let lab16 = PixelFormat { sample: SampleType::U16, ..LAB8 };
    for i in 0..COLOURS.len() {
        let mut s = target_in(lab16);
        let x = i as i32 * 100 + 50;
        let pre = s.pixel(x, 50).to_vec();
        click(&mut s, ReplaceMode::Color, f64::from(x) + 0.5);
        let c = to_rgba(&lab16, &pre);
        let b = blend_rgb(BlendMode::Color, [c[0], c[1], c[2]], [0.0, 0.0, 1.0]);
        let want = photocraft_raster::from_rgba(&lab16, [b[0], b[1], b[2], c[3]]);
        let got = s.pixel(x, 50);
        for (g, w) in got.iter().zip(&want) {
            assert!((g - w).abs() <= 1.0 / 65535.0 + 1e-6, "colour {i}: {got:?} vs {want:?}");
        }
    }
}
