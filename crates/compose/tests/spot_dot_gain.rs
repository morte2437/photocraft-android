//! Edit › Color Settings › Spot drives the ink preview of Multichannel documents and the saved
//! thumbnail. The setting is process-wide (`multichannel::set_spot_dot_gain`), so this test has its
//! own binary, and it is one test: changing it here can't race other tests that render inks.

use photocraft_color::{Color, ColorMode, PixelFormat, SampleType};
use photocraft_compose::multichannel::{SPOT_DOT_GAIN, dot_gain, set_spot_dot_gain, spot_dot_gain};
use photocraft_doc::{AlphaChannel, Document, Size};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

fn half_tint_doc() -> Document {
    let mut doc = Document::new("m", Size::new(2, 1), ColorMode::Multichannel, SampleType::F32);
    let mut ink = Surface::new(PixelFormat::new(ColorMode::Grayscale, SampleType::F32, false));
    ink.write_region(Rect::new(0, 0, 2, 1), &[0.5, 1.0]);
    // A black, transparent ink: the printed value is 1 − coverage on every channel.
    doc.channels.push(AlphaChannel { spot: Some((Color::rgb(0.0, 0.0, 0.0), 0.0)), ..AlphaChannel::new("Black", ink) });
    doc
}

#[test]
fn spot_dot_gain_darkens_midtones_of_the_ink_preview() {
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN, "Photoshop's default Spot working space");
    let doc = half_tint_doc();
    // The same ink as a spot channel of an RGB document, printed into its saved thumbnail.
    let mut rgb = Document::new("r", Size::new(2, 1), ColorMode::Rgb, SampleType::F32);
    rgb.channels = doc.channels.clone();
    for gain in [0.0, 20.0, 38.0] {
        set_spot_dot_gain(gain);
        let r = photocraft_compose::flatten(&doc);
        let expect = 1.0 - (0.5 + gain / 100.0);
        assert!((r.px[0][0] - expect).abs() < 1e-4, "{gain} %: a 50 % dot prints at {} %, got {:?}", 50.0 + gain, r.px[0]);
        assert!(r.px[1][0].abs() < 1e-6, "a solid stays solid at {gain} %");
        assert!((r.px[0][0] - (1.0 - dot_gain(0.5, gain))).abs() < 1e-6);
        let t = photocraft_compose::multichannel::thumbnail_with_inks(&rgb, 160);
        assert!((t.px[0][0] - expect).abs() < 1e-4, "{gain} %: the thumbnail's 50 % dot prints at {} %, got {:?}", 50.0 + gain, t.px[0]);
        assert!(t.px[1][0].abs() < 1e-6, "the thumbnail's solid stays solid at {gain} %");
    }
    set_spot_dot_gain(f32::NAN);
    assert_eq!(spot_dot_gain(), SPOT_DOT_GAIN, "a non-finite value falls back to the default");
    set_spot_dot_gain(80.0);
    assert_eq!(spot_dot_gain(), photocraft_compose::multichannel::MAX_SPOT_DOT_GAIN);
    set_spot_dot_gain(SPOT_DOT_GAIN);
}
