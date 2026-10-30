use super::settings::{WebFormat, WebMetadata, WebSettings};
use super::{Dither, Document, EngineError, Forced, PaletteKind, Rect, Result, Session, Value, json, other, quantize};

// ---------- the image ----------

/// The document as Save for Web sees it: RGB (converted to sRGB unless told otherwise), resized
/// by `"width"`/`"height"`/`"percent"`, flattened. Returns the document and the scale factors.
pub fn web_document(doc: &Document, p: &Value, st: &WebSettings) -> Result<(Document, f64, f64)> {
    let mut tmp = Session::new();
    tmp.add_document(doc.clone(), None);
    let mode = doc.mode;
    if mode != photocraft_color::ColorMode::Rgb {
        tmp.execute("image.mode.rgb", json!({}))?;
    }
    if st.convert_to_srgb && doc.icc_profile.is_some() {
        // Documents without a profile are treated as sRGB already.
        tmp.execute("edit.convertToProfile", json!({"profile": "srgb", "intent": "perceptual"}))?;
    }
    let (w0, h0) = (f64::from(doc.size.width), f64::from(doc.size.height));
    let pct = p.get("percent").and_then(Value::as_f64).filter(|v| *v > 0.0);
    let w = p.get("width").and_then(Value::as_f64).filter(|v| *v > 0.0);
    let h = p.get("height").and_then(Value::as_f64).filter(|v| *v > 0.0);
    let (nw, nh) = match (pct, w, h) {
        (Some(pc), _, _) => (w0 * pc / 100.0, h0 * pc / 100.0),
        (_, Some(w), Some(h)) => (w, h),
        (_, Some(w), None) => (w, h0 * w / w0),
        (_, None, Some(h)) => (w0 * h / h0, h),
        _ => (w0, h0),
    };
    let (nw, nh) = (nw.round().max(1.0), nh.round().max(1.0));
    if (nw - w0).abs() >= 1.0 || (nh - h0).abs() >= 1.0 {
        let resample = p.get("resample").and_then(Value::as_str).unwrap_or("bicubic");
        tmp.execute("image.imageSize", json!({"width": nw, "height": nh, "resample": resample}))?;
    }
    let out = (*tmp.active().ok_or(EngineError::NoDocument)?.doc).clone();
    Ok((out, nw / w0, nh / h0))
}

/// An optimised image.
#[derive(Clone, Debug)]
pub struct Optimized {
    pub bytes: Vec<u8>,
    pub ext: &'static str,
    pub width: u32,
    pub height: u32,
    /// Colours in the palette (indexed formats).
    pub colors: Option<usize>,
    /// Opaque source palette before replacements (empty for non-indexed formats).
    pub color_table: Vec<[u8; 3]>,
    pub transparent_index: Option<usize>,
    /// Indexed pixels for previews; empty for non-indexed formats or when preview is disabled.
    pub indices: Vec<u8>,
    /// What the optimised file looks like, straight RGBA8 (for the dialog's previews).
    pub preview: Vec<u8>,
}

pub(super) fn to8(v: f32) -> u8 {
    (v.clamp(0.0, 1.0) * 255.0).round() as u8
}

/// XMP to embed for the metadata choice.
pub fn web_xmp(doc: &Document, m: WebMetadata) -> Option<String> {
    let xmp = doc.metadata.xmp.as_deref()?;
    match m {
        WebMetadata::None => None,
        WebMetadata::All | WebMetadata::AllExceptCameraInfo => Some(xmp.to_string()),
        WebMetadata::Copyright | WebMetadata::CopyrightAndContact => {
            let info = crate::file_cmds::read_file_info(Some(xmp));
            let mut keep = serde_json::Map::new();
            let mut keys = vec!["copyright", "copyrightStatus", "copyrightUrl"];
            if m == WebMetadata::CopyrightAndContact {
                keys.extend(["author", "authorTitle"]);
            }
            for k in keys {
                if let Some(v) = info.get(k).filter(|v| v.as_str().is_some_and(|s| !s.is_empty()) || v.is_array()) {
                    keep.insert(k.into(), v.clone());
                }
            }
            (!keep.is_empty()).then(|| crate::file_cmds::write_file_info(None, &Value::Object(keep)))
        }
    }
}

/// The photo's EXIF to embed for the metadata choice: only "All" carries it (#7436).
pub fn web_exif(doc: &Document, m: WebMetadata) -> Option<&[u8]> {
    doc.metadata.exif.as_deref().map(Vec::as_slice).filter(|_| m == WebMetadata::All)
}

/// The response's warnings: EXIF that a JPEG couldn't hold (one segment, about 64 KB).
pub(super) fn web_warnings(st: &WebSettings, exif: Option<&[u8]>, dpi: f32) -> Vec<String> {
    exif.filter(|x| st.format == WebFormat::Jpeg && !photocraft_codecs::web::jpeg_exif_fits(x, Some(dpi)))
        .map(|x| photocraft_codecs::FidelityWarning::MetadataTooLarge { what: "EXIF", bytes: x.len() }.to_string())
        .into_iter()
        .collect()
}

/// Optimises `rect` of `px` (straight RGBA, `bw` wide) with `st`. `preview` also returns what
/// the file looks like (the dialog's panes); exports skip it.
#[allow(clippy::too_many_arguments)]
pub fn optimize(
    px: &[[f32; 4]],
    bw: usize,
    rect: Rect,
    st: &WebSettings,
    icc: Option<&[u8]>,
    exif: Option<&[u8]>,
    xmp: Option<&str>,
    dpi: f32,
    preview: bool,
) -> Result<Optimized> {
    if bw == 0 || rect.x0 < 0 || rect.y0 < 0 || rect.is_empty() || rect.x1 as usize > bw || (rect.y1 as usize).checked_mul(bw).is_none_or(|end| end > px.len())
    {
        return Err(other("slice is outside the pixel buffer"));
    }
    let (w, h) = (rect.width() as usize, rect.height() as usize);
    let row = |y: i32| {
        let o = y as usize * bw;
        px.get(o + rect.x0 as usize..o + rect.x1 as usize).unwrap_or(&[])
    };
    let region = || (rect.y0..rect.y1).flat_map(row);
    let matte = st.matte.unwrap_or([1.0, 1.0, 1.0]);
    let over = |p: &[f32; 4]| -> [f32; 4] {
        [p[0] * p[3] + matte[0] * (1.0 - p[3]), p[1] * p[3] + matte[1] * (1.0 - p[3]), p[2] * p[3] + matte[2] * (1.0 - p[3]), 1.0]
    };
    let (ww, hh) = (w as u32, h as u32);
    let e = |e: photocraft_codecs::CodecError| other(e);
    let mut look: Vec<u8> = Vec::new();
    let mut color_table = Vec::new();
    let mut transparent_index = None;
    let mut indices = Vec::new();
    // True colour (PNG-24, WebP): straight RGBA when transparency is on and a pixel is
    // translucent, else RGB over the matte.
    let true_color = || {
        let alpha = st.transparency && region().any(|p| p[3] < 1.0);
        let k = if alpha { 4 } else { 3 };
        let mut data = Vec::with_capacity(w * h * k);
        for p in region() {
            let q = if alpha { *p } else { over(p) };
            data.extend_from_slice(&[to8(q[0]), to8(q[1]), to8(q[2]), to8(q[3])][..k]);
        }
        let layout = if alpha { photocraft_codecs::ChannelLayout::Rgba } else { photocraft_codecs::ChannelLayout::Rgb };
        (data, layout)
    };
    let rgba = |data: &[u8], layout: photocraft_codecs::ChannelLayout| -> Vec<u8> {
        if layout == photocraft_codecs::ChannelLayout::Rgba {
            data.to_vec()
        } else {
            data.as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect()
        }
    };
    let meta = || photocraft_codecs::Metadata { dpi: Some((dpi, dpi)), xmp: xmp.map(str::to_string), ..Default::default() };
    let (bytes, colors) = match st.format {
        WebFormat::Png24 => {
            let (data, layout) = true_color();
            if preview {
                look = rgba(&data, layout);
            }
            let img = photocraft_codecs::Image::from_u8(ww, hh, layout, data).map_err(e)?.with_icc(icc.map(<[u8]>::to_vec)).with_meta(meta());
            let opts = photocraft_codecs::EncodeOptions {
                png_interlaced: st.interlaced,
                embed_icc: icc.is_some(),
                embed_metadata: xmp.is_some(),
                ..Default::default()
            };
            (photocraft_codecs::encode(&img, photocraft_codecs::Format::Png, &opts).map_err(e)?, None)
        }
        WebFormat::Webp => {
            let (data, layout) = true_color();
            let icc = icc.filter(|_| st.embed_icc);
            // WebP holds EXIF (the metadata choice's "All" passes it in), so it rides along with the XMP.
            let webp_meta = photocraft_codecs::Metadata { exif: exif.map(<[u8]>::to_vec), ..meta() };
            let img = photocraft_codecs::Image::from_u8(ww, hh, layout, data).map_err(e)?.with_icc(icc.map(<[u8]>::to_vec)).with_meta(webp_meta);
            let opts = photocraft_codecs::EncodeOptions {
                webp_lossless: st.lossless,
                webp_quality: st.quality.min(100),
                embed_icc: icc.is_some(),
                embed_metadata: exif.is_some() || xmp.is_some(),
                ..Default::default()
            };
            let bytes = photocraft_codecs::encode(&img, photocraft_codecs::Format::WebP, &opts).map_err(e)?;
            if preview {
                // Lossless shows the source; lossy shows the compression artefacts.
                look = match (!st.lossless).then(|| photocraft_codecs::decode(&bytes).ok()).flatten() {
                    Some(img) => img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8).data().to_vec(),
                    None => rgba(img.data(), layout),
                };
            }
            (bytes, None)
        }
        WebFormat::Jpeg => {
            let mut rgb = Vec::with_capacity(w * h * 3);
            for p in region() {
                let q = over(p);
                rgb.extend_from_slice(&[to8(q[0]), to8(q[1]), to8(q[2])]);
            }
            // Save for Web's 0–100 quality → encoder 1–100.
            let q = st.quality.max(1);
            let bytes =
                photocraft_codecs::web::encode_jpeg_rgb8(ww, hh, &rgb, q, st.progressive, st.optimized, icc.filter(|_| st.embed_icc), Some(dpi), exif, xmp)
                    .map_err(e)?;
            if preview {
                // The preview shows the compression artefacts.
                look = match photocraft_codecs::decode(&bytes) {
                    Ok(img) => img.convert(photocraft_codecs::ChannelLayout::Rgba, photocraft_codecs::SampleType::U8).data().to_vec(),
                    Err(_) => rgb.as_chunks::<3>().0.iter().flat_map(|c| [c[0], c[1], c[2], 255]).collect(),
                };
            }
            (bytes, None)
        }
        WebFormat::Wbmp => {
            let mut gray: Vec<f32> = region()
                .map(|p| {
                    let q = over(p);
                    0.299 * q[0] + 0.587 * q[1] + 0.114 * q[2]
                })
                .collect();
            let method = match st.dither {
                Dither::None => quantize::BitmapMethod::Threshold,
                Dither::Pattern => quantize::BitmapMethod::Pattern,
                _ => quantize::BitmapMethod::Diffusion,
            };
            quantize::to_bitmap(&mut gray, w, method);
            let white: Vec<bool> = gray.iter().map(|v| *v > 0.5).collect();
            if preview {
                look = white.iter().flat_map(|b| if *b { [255, 255, 255, 255] } else { [0, 0, 0, 255] }).collect();
            }
            (photocraft_codecs::web::encode_wbmp(ww, hh, &white).map_err(e)?, Some(2))
        }
        WebFormat::Gif | WebFormat::Png8 => {
            let transparent = st.transparency && region().any(|p| p[3] < 0.5);
            // Partially transparent edges blend with the matte; with transparency off everything does.
            let mut work: Vec<[f32; 4]> = region()
                .map(|p| {
                    if st.transparency && p[3] < 0.5 {
                        [0.0, 0.0, 0.0, 0.0]
                    } else if st.transparency && st.matte.is_none() {
                        [p[0], p[1], p[2], 1.0]
                    } else {
                        over(p)
                    }
                })
                .collect();
            let room = if transparent { st.colors.saturating_sub(1).max(2) } else { st.colors };
            let mut pal = if let Some(table) = &st.color_table {
                if table.is_empty() || table.len() > if transparent { 255 } else { 256 } {
                    return Err(other("colorTable has too many entries for this image's transparency"));
                }
                table.clone()
            } else {
                // Large images: build adaptive palettes from an even ~1 MP sample (Exact needs all).
                let n_opaque = work.iter().filter(|p| p[3] >= 0.5).count();
                let step = if n_opaque > 1 << 20 && st.palette != PaletteKind::Exact { n_opaque / (1 << 20) + 1 } else { 1 };
                let mut src: Vec<[f32; 4]> = work.iter().filter(|p| p[3] >= 0.5).step_by(step).copied().collect();
                if src.is_empty() {
                    src = work.clone();
                }
                quantize::build_palette(&src, st.palette, room, Forced::None).map_err(other)?
            };
            if st.web_snap > 0.0 && st.color_table.is_none() {
                let tol = (st.web_snap * 255.0) as u8;
                for c in &mut pal {
                    let snap = c.map(|x| ((f32::from(x) / 51.0).round() * 51.0) as u8);
                    if c.iter().zip(snap).all(|(a, b)| a.abs_diff(b) <= tol) {
                        *c = snap;
                    }
                }
            }
            if transparent {
                pal.truncate(255);
            }
            let first = pal.first().copied().ok_or_else(|| other("empty colour table"))?;
            color_table = pal.clone();
            let t = transparent.then(|| {
                pal.push(first);
                pal.len() - 1
            });
            transparent_index = t;
            let idx = quantize_rows(&mut work, w, &pal, st.dither, st.dither_amount, t);
            for (index, color) in &st.color_table_edits {
                let slot = pal.get_mut(*index).filter(|_| Some(*index) != t).ok_or_else(|| other("colour edit is outside the opaque palette"))?;
                *slot = *color;
            }
            if preview {
                look = idx
                    .iter()
                    .flat_map(|k| {
                        let c = pal.get(*k as usize).copied().unwrap_or(first);
                        [c[0], c[1], c[2], if Some(*k as usize) == t { 0 } else { 255 }]
                    })
                    .collect();
            }
            let tt = t.map(|v| v as u8);
            let bytes = if st.format == WebFormat::Gif {
                photocraft_codecs::web::encode_gif_indexed(ww, hh, &idx, &pal, tt, st.interlaced).map_err(e)?
            } else {
                photocraft_codecs::encode_png_indexed(ww, hh, &idx, &pal, tt, None, None).map_err(e)?
            };
            if preview {
                indices = idx;
            }
            (bytes, Some(pal.len()))
        }
    };
    Ok(Optimized { bytes, ext: st.format.ext(), width: ww, height: hh, colors, color_table, transparent_index, indices, preview: look })
}

/// Nearest palette entry through a lazily filled 6-bit-per-channel inverse colour map: each
/// 4×4×4 cell is resolved once (by its centre), so large images cost a table lookup per pixel
/// instead of a palette scan. `t` (the transparent index) is never returned.
struct InverseMap<'a> {
    pal: &'a [[u8; 3]],
    t: Option<usize>,
    lut: Vec<u16>,
}

impl<'a> InverseMap<'a> {
    fn new(pal: &'a [[u8; 3]], t: Option<usize>) -> Self {
        InverseMap { pal, t, lut: vec![u16::MAX; 1 << 18] }
    }
    #[inline]
    fn get(&mut self, c: [f32; 3]) -> usize {
        let q = c.map(|v| ((v.clamp(0.0, 1.0) * 255.0).round() as usize) >> 2);
        let key = q[0] << 12 | q[1] << 6 | q[2];
        let v = self.lut[key];
        if v != u16::MAX {
            return usize::from(v);
        }
        let center = q.map(|v| (v * 4 + 2) as f32);
        let mut best = (f32::MAX, 0usize);
        for (i, e) in self.pal.iter().enumerate() {
            if Some(i) == self.t {
                continue;
            }
            let d: f32 = (0..3).map(|k| (center[k] - f32::from(e[k])).powi(2)).sum();
            if d < best.0 {
                best = (d, i);
            }
        }
        self.lut[key] = best.1 as u16;
        best.1
    }
}

fn hash01(x: usize, y: usize) -> f32 {
    let mut h = (x as u32).wrapping_mul(0x8da6_b343) ^ (y as u32).wrapping_mul(0xd816_3841) ^ 0x2545_f491;
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

/// Palette mapping with dithering (Floyd–Steinberg diffusion, 8×8 Bayer pattern, noise), like
/// `algo::quantize::quantize` but through [`InverseMap`]. `y0` keeps patterns continuous across
/// bands. Pixels with alpha < ½ take index `t`.
fn quantize_band(px: &mut [[f32; 4]], w: usize, y0: usize, pal: &[[u8; 3]], dither: Dither, amount: f32, t: Option<usize>) -> Vec<u8> {
    let h = px.len() / w.max(1);
    let mut map = InverseMap::new(pal, t);
    let mut idx = vec![0u8; px.len()];
    let opaque_n = pal.len() - usize::from(t.is_some());
    let spread = (1.0 / (opaque_n as f32).cbrt().max(1.0)).min(0.5) * amount.clamp(0.0, 1.0);
    let a = amount.clamp(0.0, 1.0);
    let mut cur = vec![[0.0f32; 3]; if dither == Dither::Diffusion { w + 2 } else { 0 }];
    let mut next = cur.clone();
    for y in 0..h {
        if dither == Dither::Diffusion {
            std::mem::swap(&mut cur, &mut next);
            next.iter_mut().for_each(|e| *e = [0.0; 3]);
        }
        for x in 0..w {
            let i = y * w + x;
            let p = px[i];
            if p[3] < 0.5
                && let Some(t) = t
            {
                idx[i] = t as u8;
                continue;
            }
            let mut c = [p[0], p[1], p[2]];
            match dither {
                Dither::None => {}
                Dither::Diffusion => (0..3).for_each(|k| c[k] += cur[x + 1][k]),
                Dither::Pattern => {
                    let d = quantize::bayer8(x, y0 + y) * spread;
                    c = c.map(|v| v + d);
                }
                Dither::Noise => {
                    let d = (hash01(x, y0 + y) - 0.5) * spread;
                    c = c.map(|v| v + d);
                }
            }
            let c = c.map(|v| v.clamp(0.0, 1.0));
            let k = map.get(c);
            idx[i] = k as u8;
            if dither == Dither::Diffusion {
                let e = pal[k].map(|v| f32::from(v) / 255.0);
                let d: [f32; 3] = std::array::from_fn(|q| (c[q] - e[q]) * a);
                for q in 0..3 {
                    cur[x + 2][q] += d[q] * 7.0 / 16.0;
                    next[x][q] += d[q] * 3.0 / 16.0;
                    next[x + 1][q] += d[q] * 5.0 / 16.0;
                    next[x + 2][q] += d[q] / 16.0;
                }
            }
        }
    }
    idx
}

/// [`quantize_band`] over the whole image, in parallel bands of 64 rows when the dither is
/// position-independent. Error diffusion carries error down the image, so it stays sequential.
fn quantize_rows(px: &mut [[f32; 4]], w: usize, pal: &[[u8; 3]], dither: Dither, amount: f32, t: Option<usize>) -> Vec<u8> {
    let h = px.len() / w.max(1);
    if dither == Dither::Diffusion || h < 128 || cfg!(target_arch = "wasm32") {
        return quantize_band(px, w, 0, pal, dither, amount, t);
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        let band = w * 64;
        let mut idx = vec![0u8; px.len()];
        px.par_chunks_mut(band)
            .zip(idx.par_chunks_mut(band))
            .enumerate()
            .for_each(|(i, (p, out))| out.copy_from_slice(&quantize_band(p, w, i * 64, pal, dither, amount, t)));
        idx
    }
    // Single-threaded on wasm (the early return above always takes this path there).
    #[cfg(target_arch = "wasm32")]
    quantize_band(px, w, 0, pal, dither, amount, t)
}
