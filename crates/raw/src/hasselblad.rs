//! Hasselblad 3FR / FFF: a TIFF whose IFD0 holds the camera's JPEG preview and
//! whose CFA image sits in a SubIFD with DNG-style tags (BlackLevel, WhiteLevel,
//! DefaultCropOrigin / DefaultCropSize, AsShotNeutral, ColorMatrix1) but no
//! DNGVersion and no CFA-pattern tag — neither the TIFF/EP 33421/33422 pair nor
//! the EXIF one — so the Bayer phase comes from measurements of each body.
//!
//! Decoded today: the X2D II 100C's 16-bit uncompressed samples (Compression 1,
//! one strip covering the whole sensor, masked borders included; the crop to the
//! usable area comes from the file's DefaultCrop tags). Bodies that store
//! lossless-JPEG strips (Compression 7, the earlier X and H generations) are
//! recognised but not decoded yet.
//!
//! Clean-room: derived from the TIFF 6.0 and DNG specifications, observation of
//! sample files, and the camera's own embedded JPEG — the oracle the
//! camera-profile measurements in [`crate::cameras`] use. No code or tables
//! from dcraw, LibRaw, rawspeed, rawler or darktable.

use crate::cr2::clip_level;
use crate::error::{RawError, Result};
use crate::sensor::{BlackLevels, Cfa, JpegLayout, Rect, Sensor, read_plane};
use crate::tiff::{Ifd, Tiff, tag};
use crate::tiffep;
use crate::{Limits, RawFormat};

/// The Bayer colours at the data origin, row-major (0 red, 1 green, 2 blue).
/// Hasselblad records no CFA tag, so a body's phase is measured against its own
/// embedded JPEG: on an X2D II 100C dusk landscape the blue sky reads red at the
/// origin site at about half the green rate while the blue site holds three
/// quarters of it, the reverse of the warm ground.
fn cfa_for(model: Option<&str>) -> (Cfa, bool) {
    let measured = model.is_some_and(|m| m.eq_ignore_ascii_case("X2D II 100C"));
    (Cfa { width: 2, height: 2, colors: vec![0, 1, 1, 2], origin_x: 0, origin_y: 0 }, measured)
}

pub(crate) fn decode(t: &Tiff, limits: &Limits) -> Result<Sensor> {
    let ifds = t.all_ifds();
    let Some(raw) = tiffep::raw_ifd(t, &ifds) else {
        return Err(RawError::unsupported("Hasselblad 3FR/FFF: no CFA image found"));
    };
    match t.tag_uint(&raw, tag::COMPRESSION).unwrap_or(1) {
        1 => {}
        7 => return Err(RawError::unsupported("lossless-JPEG (compressed 3FR/FFF)")),
        c => return Err(RawError::unsupported(format!("raw compression scheme {c}"))),
    }
    let plane = read_plane(t, &raw, limits, JpegLayout::Flat)?;
    if plane.samples != 1 {
        return Err(RawError::unsupported(format!("CFA data with {} samples per pixel", plane.samples)));
    }
    let make = ifds.iter().find_map(|i| t.tag_ascii(i, tag::MAKE));
    let model = ifds.iter().find_map(|i| t.tag_ascii(i, tag::MODEL));
    let (cfa, measured) = cfa_for(model.as_deref());
    let mut warnings = Vec::new();
    if !measured {
        warnings.push(format!(
            "{}: this body's Bayer phase is not measured yet; assumed the X2D II 100C's (red at the data origin)",
            model.as_deref().unwrap_or("Hasselblad")
        ));
    }
    let full = Rect::new(0, 0, plane.width, plane.height);
    // DNG-style ActiveArea (top, left, bottom, right) in the raw IFD; an absent,
    // inverted or out-of-plane one leaves the whole plane active.
    let active = match t.tag_uints(&raw, tag::ACTIVE_AREA).as_slice() {
        [top, left, bottom, right] if bottom > top && right > left => {
            Rect::new(*left as usize, *top as usize, (*right - *left) as usize, (*bottom - *top) as usize).intersect(&full)
        }
        _ => full,
    };
    let active = if active.is_empty() { full } else { active };
    // DNG-style DefaultCropOrigin / DefaultCropSize in the raw IFD, relative to
    // the active area as in DNG.
    let mut crop = active;
    if let ([ox, oy], [cw, ch]) = (t.tag_floats(&raw, tag::DEFAULT_CROP_ORIGIN).as_slice(), t.tag_floats(&raw, tag::DEFAULT_CROP_SIZE).as_slice()) {
        let f = |v: f64| if v.is_finite() && v >= 0.0 { v.round() as usize } else { 0 };
        let r = Rect::new(active.x.saturating_add(f(*ox)), active.y.saturating_add(f(*oy)), f(*cw), f(*ch)).intersect(&active);
        if !r.is_empty() {
            crop = r;
        }
    }
    let bl = t.tag_floats(&raw, tag::BLACK_LEVEL);
    let black = match bl.as_slice() {
        [b] if b.is_finite() && *b >= 0.0 => BlackLevels::uniform(*b as f32),
        // A 2×2 repeat is in the CFA's own row-major order.
        [r, g1, g2, b] if [*r, *g1, *g2, *b].iter().all(|v| v.is_finite() && *v >= 0.0) => {
            BlackLevels { rows: 2, cols: 2, values: [r, g1, g2, b].map(|v| *v as f32).to_vec(), delta_h: Vec::new(), delta_v: Vec::new() }
        }
        _ => {
            warnings.push("Hasselblad 3FR/FFF: black level not recorded; assumed 0".into());
            BlackLevels::uniform(0.0)
        }
    };
    let white =
        t.tag_floats(&raw, tag::WHITE_LEVEL).first().copied().filter(|w| *w > 0.0).map(|w| w as f32).unwrap_or_else(|| clip_level(&plane.data, plane.bits));
    // AsShotNeutral (IFD0) is the camera-space colour of a neutral patch: the
    // green-normalised multipliers are the other channels over green.
    let camera_wb = ifds
        .iter()
        .find_map(|i: &Ifd| t.tag_floats(i, tag::AS_SHOT_NEUTRAL).try_into().ok())
        .and_then(|n: [f64; 3]| match n {
            [r, g, b] if g > 0.0 && r > 0.0 && b > 0.0 => Some([g / r, 1.0, g / b]),
            _ => None,
        })
        .filter(|m| m.iter().all(|v| (0.25..8.0).contains(v)));
    let ifd0 = t.ifd_at(t.first_ifd, 0);
    let baseline_exposure = ifd0.as_ref().map(|i| t.tag_floats(i, tag::BASELINE_EXPOSURE).first().copied().unwrap_or(0.0)).unwrap_or(0.0);
    let mut sensor = Sensor {
        format: RawFormat::Fff,
        make,
        model,
        width: plane.width,
        height: plane.height,
        samples: 1,
        data: plane.data,
        cfa: Some(cfa),
        linearization: None,
        black,
        white: [white; 3],
        active,
        crop,
        color: Default::default(),
        camera_wb,
        orientation: crate::tiff::orientation(ifd0.as_ref().and_then(|i| t.tag_uint(i, tag::ORIENTATION))),
        baseline_exposure: if baseline_exposure.is_finite() { baseline_exposure.clamp(-10.0, 10.0) } else { 0.0 },
        gain_maps: Vec::new(),
        tone_curve: Vec::new(),
        black_point: 0.0,
        warnings,
    };
    crate::cameras::apply(&mut sensor, plane.bits);
    Ok(sensor)
}
