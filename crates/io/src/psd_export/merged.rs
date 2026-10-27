use super::*;

pub(super) fn cmyk_of(fmt: &PixelFormat) -> bool {
    fmt.mode == ColorMode::Cmyk
}

/// The merged image's planes (colour channels, then alpha), big-endian, matted against white
/// when `matte`; with whether any pixel needs the alpha channel and whether any is below 1.
/// `ctl` gets the fraction of rows done after each band; once it is cancelled the rest of the
/// image is left blank (the caller discards it).
pub(super) fn merged_planes(doc: &Document, fmt: &PixelFormat, cmyk: bool, matte: bool, ctl: &Interrupt) -> (Vec<u8>, bool, bool) {
    if cmyk && let Some(planes) = merged_ink_planes(doc, fmt.sample, matte, ctl) {
        return planes;
    }
    let sample = fmt.sample;
    let cc = fmt.mode.color_channels();
    let bps = sample.bytes();
    let n = doc.size.area() as usize;
    let plane = n * bps;
    let mut planes = vec![0u8; plane * (cc + 1)];
    let (mut has_alpha, mut translucent) = (false, false);
    let white = photocraft_raster::from_rgba(fmt, [1.0, 1.0, 1.0, 1.0]);
    let canvas = doc.bounds();
    let w = canvas.width() as usize;
    let space = photocraft_compose::cmyk_space(doc);
    let lab16 = fmt.mode == ColorMode::Lab && sample == SampleType::U16;
    let rows = canvas.height().max(1) as f32;
    let _ = photocraft_compose::render_bands(doc, canvas, 0, |band| -> Result<(), ()> {
        if ctl.cancelled() {
            return Err(());
        }
        // Retain alpha whenever it differs from opaque at the stored precision.
        has_alpha |= band.px.iter().any(|p| match sample {
            SampleType::U8 => q255(p[3]) < 255,
            SampleType::U16 => (p[3].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16 != u16::MAX,
            SampleType::F32 => p[3] != 1.0,
        });
        translucent |= band.px.iter().any(|p| p[3] < 1.0);
        let start = (band.rect.y0 - canvas.y0) as usize * w;
        // Converted and encoded on all cores, then copied into each plane.
        let parts = crate::pixels::par_map(crate::pixels::bands(band.px.len()), |range| {
            // The composite came through the document's CMYK profile: convert back through it too.
            photocraft_color::convert::with_cmyk_space(space.as_ref(), || {
                let mut out: Vec<Vec<u8>> = vec![Vec::with_capacity(range.len() * bps); cc + 1];
                let mut v = [0.0f32; 5];
                for p in &band.px[range.clone()] {
                    photocraft_raster::from_rgba_into(fmt, *p, &mut v);
                    for c in 0..=cc {
                        // Matte against white like Photoshop (see `pixels::matte`).
                        let m = if c < cc && matte { crate::pixels::matte(v[c], v[cc], white[c]) } else { v[c] };
                        let x = if cmyk && c < cc { 1.0 - m } else { m };
                        // 16-bit Lab a*/b* use Photoshop's 0..65280 scale (`pixels::lab16_chroma`).
                        let x = if lab16 && (c == 1 || c == 2) { x * (crate::pixels::LAB16_CHROMA_MAX / 65535.0) } else { x };
                        encode_be(x, sample, &mut out[c]);
                    }
                }
                (range.start, out)
            })
        });
        for (at, part) in parts {
            for (c, src) in part.iter().enumerate() {
                let o = c * plane + (start + at) * bps;
                planes[o..o + src.len()].copy_from_slice(src);
            }
        }
        ctl.progress((band.rect.y1 - canvas.y0) as f32 / rows);
        Ok(())
    });
    (planes, has_alpha, translucent)
}

/// [`merged_planes`] of a CMYK document whose layers are mixed on their ink values
/// (`photocraft_compose::cmyk`), so opaque pixels keep their exact inks; `None` for documents
/// that composite in RGB. Reports progress and stops on cancel like [`merged_planes`].
fn merged_ink_planes(doc: &Document, sample: SampleType, matte: bool, ctl: &Interrupt) -> Option<(Vec<u8>, bool, bool)> {
    let bps = sample.bytes();
    let n = doc.size.area() as usize;
    let plane = n * bps;
    let mut planes = vec![0u8; plane * 5];
    let (mut has_alpha, mut translucent) = (false, false);
    let canvas = doc.bounds();
    let w = canvas.width() as usize;
    let rows = canvas.height().max(1) as f32;
    // A cancelled export leaves the rest blank (the caller discards it), so the result stands either way.
    let _ = photocraft_compose::cmyk::render_cmyk_bands(doc, canvas, 0, |band| -> Result<(), ()> {
        if ctl.cancelled() {
            return Err(());
        }
        // Retain alpha whenever it differs from opaque at the stored precision.
        has_alpha |= band.px.iter().any(|p| match sample {
            SampleType::U8 => q255(p[4]) < 255,
            SampleType::U16 => (p[4].clamp(0.0, 1.0) * 65535.0 + 0.5) as u16 != u16::MAX,
            SampleType::F32 => p[4] != 1.0,
        });
        translucent |= band.px.iter().any(|p| p[4] < 1.0);
        let start = (band.rect.y0 - canvas.y0) as usize * w;
        let parts = crate::pixels::par_map(crate::pixels::bands(band.px.len()), |range| {
            let mut out: Vec<Vec<u8>> = (0..5).map(|_| Vec::with_capacity(range.len() * bps)).collect();
            for p in band.px.get(range.clone()).unwrap_or(&[]) {
                for (c, o) in out.iter_mut().enumerate() {
                    // Inks matted against white paper (no ink) like Photoshop (see
                    // `pixels::matte`), and stored inverted as PSD's CMYK is.
                    let v = if c < 4 { 1.0 - if matte { crate::pixels::matte(p[c], p[4], 0.0) } else { p[c] } } else { p[4] };
                    encode_be(v, sample, o);
                }
            }
            (range.start, out)
        });
        for (at, part) in parts {
            for (c, src) in part.iter().enumerate() {
                let o = c * plane + (start + at) * bps;
                if let Some(dst) = planes.get_mut(o..o + src.len()) {
                    dst.copy_from_slice(src);
                }
            }
        }
        ctl.progress((band.rect.y1 - canvas.y0) as f32 / rows);
        Ok(())
    })?;
    Some((planes, has_alpha, translucent))
}

/// The merged image of a file saved without Maximize Compatibility: Photoshop's white placeholder
/// in the document's colour channels, with no alpha. Encoded like [`merged_planes`], so CMYK white
/// is no ink and Lab white is L 100 with neutral a*/b*.
pub(super) fn placeholder_planes(doc: &Document, fmt: &PixelFormat) -> Vec<u8> {
    let cc = fmt.mode.color_channels();
    let cmyk = cmyk_of(fmt);
    let lab16 = fmt.mode == ColorMode::Lab && fmt.sample == SampleType::U16;
    let space = photocraft_compose::cmyk_space(doc);
    let white = photocraft_color::convert::with_cmyk_space(space.as_ref(), || photocraft_raster::from_rgba(fmt, [1.0, 1.0, 1.0, 1.0]));
    let n = doc.size.area() as usize;
    let mut planes = Vec::with_capacity(n * fmt.sample.bytes() * cc);
    for c in 0..cc {
        let v = white.get(c).copied().unwrap_or(1.0);
        let x = if cmyk { 1.0 - v } else { v };
        let x = if lab16 && (c == 1 || c == 2) { x * (crate::pixels::LAB16_CHROMA_MAX / 65535.0) } else { x };
        let mut sample = Vec::with_capacity(4);
        encode_be(x, fmt.sample, &mut sample);
        planes.extend(sample.iter().copied().cycle().take(n * sample.len()));
    }
    planes
}
