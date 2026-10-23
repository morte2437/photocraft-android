//! The Nikon D4 profile (`cameras.rs`) was measured on 14-bit NEFs. The camera also writes 12-bit
//! files (lossless, lossy and uncompressed) whose samples are the 14-bit values shifted right by
//! two, so the profile's saturation level must follow the sample depth: a 12-bit D4 frame has to
//! develop as bright as the same scene at 14 bits.

use photocraft_raw::testgen::{Val, tiff_ep};
use photocraft_raw::{Demosaic, DevelopOptions, Limits, Rect, WhiteBalance, decode, develop_sensor};

const WIDTH: usize = 4992;
const HEIGHT: usize = 3292;

/// A synthetic D4 NEF: a smooth ramp, 14-bit values (3000..7000, well below the saturation
/// level) stored at `bits`.
fn d4_nef(bits: u16) -> Vec<u8> {
    let shift = 14 - u32::from(bits);
    let data: Vec<u16> = (0..WIDTH * HEIGHT).map(|i| (3000 + ((i % WIDTH) + 2 * (i / WIDTH)) % 4000) as u16 >> shift).collect();
    tiff_ep("NIKON CORPORATION", WIDTH, HEIGHT, &data, [0, 1, 1, 2], bits, vec![(272, Val::Ascii("NIKON D4".into()))])
}

/// Mean linear luminance of the developed ProPhoto frame over a small window of the image.
fn mean_luminance(bits: u16) -> (f32, f64) {
    let mut s = decode(&d4_nef(bits), &Limits::default()).expect("a synthetic D4 NEF decodes");
    let white = s.white[1];
    // Only a window is developed: the profile is keyed on the full sensor size, the test is not.
    s.crop = Rect::new(2000, 1000, 64, 64);
    let opts = DevelopOptions { demosaic: Demosaic::Bilinear, white_balance: WhiteBalance::Multipliers([2.0, 1.0, 1.5]), ..DevelopOptions::default() };
    let d = develop_sensor(&s, &opts).expect("a synthetic D4 NEF develops");
    let lin = |v: u16| (f64::from(v) / 65535.0).powf(1.8);
    let pixels = d.rgb.as_chunks::<3>().0;
    let y: f64 = pixels.iter().map(|p| 0.2880402 * lin(p[0]) + 0.7118741 * lin(p[1]) + 0.0000857 * lin(p[2])).sum();
    (white, y / pixels.len() as f64)
}

#[test]
fn a_12_bit_d4_nef_develops_as_bright_as_the_same_scene_at_14_bit() {
    let (white14, y14) = mean_luminance(14);
    let (white12, y12) = mean_luminance(12);
    let stops = (y12 / y14).log2();
    assert!(stops.abs() < 0.2, "12-bit differs from 14-bit by {stops:.2} stops (mean Y {y12:.4} vs {y14:.4})");
    // The 14-bit level is the measured one, untouched.
    assert_eq!(white14, 15520.0);
    // The 12-bit level is the same fraction of the (4095-range) data.
    assert_eq!(white12, 3880.0);
}
