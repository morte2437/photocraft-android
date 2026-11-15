//! Patterns ⇄ PSD: the global `Patt`/`Pat2`/`Pat3` blocks and `.pat` files.
//!
//! Import parses the blocks into [`Document::patterns`] and keeps the raw blocks. Export reuses
//! the raw blocks verbatim while they still decode to the document's patterns (byte-exact round
//! trips); otherwise they are replaced by one block written from the document.

use std::sync::Arc;

use craft_psd::patterns::{PatternBudget, PsdPattern, block_has_user_mask, block_key, mode_channels, parse_pattern_block_with, write_pattern_block};
use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Document, Pattern};
use photocraft_geom::Rect;
use photocraft_raster::Surface;

use crate::pixels::{max_sample, psd_depth, sample_for_depth, try_interleave, zero_sample};

/// Global block keys holding patterns.
pub const PATTERN_KEYS: [[u8; 4]; 3] = [*b"Patt", *b"Pat2", *b"Pat3"];

fn psd_mode(m: ColorMode) -> u32 {
    match m {
        ColorMode::Grayscale => 1,
        ColorMode::Cmyk => 4,
        ColorMode::Lab => 9,
        _ => 3,
    }
}

/// A PSD pattern as a document pattern (indexed patterns become RGB through their palette). A
/// malformed pattern, or one that can't be allocated, is `None`; [`try_from_psd`] tells them apart.
pub fn from_psd(p: &PsdPattern) -> Option<Pattern> {
    try_from_psd(p).ok().flatten()
}

/// [`from_psd`], with `Ok(None)` for a malformed pattern and `Err` for a failed allocation.
pub fn try_from_psd(p: &PsdPattern) -> Result<Option<Pattern>, String> {
    let mut oom = None;
    let pattern = pattern_from_psd(p, &mut oom);
    match oom {
        Some(error) => Err(error),
        None => Ok(pattern),
    }
}

fn pattern_from_psd(p: &PsdPattern, oom: &mut Option<String>) -> Option<Pattern> {
    let (w, h) = (p.width as usize, p.height as usize);
    if w == 0 || h == 0 || w > 30_000 || h > 30_000 || !matches!(p.depth, 1 | 8 | 16 | 32) {
        return None;
    }
    let n = w.checked_mul(h)?;
    let plane_len = w.checked_mul(usize::from(p.depth))?.div_ceil(8).checked_mul(h)?;
    if p.channels.iter().chain(p.alpha.iter()).any(|plane| plane.len() != plane_len) {
        return None;
    }
    fn copy_plane(plane: &[u8], oom: &mut Option<String>) -> Option<Vec<u8>> {
        let mut out = Vec::new();
        reserve(&mut out, plane.len(), oom)?;
        out.extend_from_slice(plane);
        Some(out)
    }
    fn reserve(out: &mut Vec<u8>, n: usize, oom: &mut Option<String>) -> Option<()> {
        out.try_reserve_exact(n).map_err(|e| *oom = Some(e.to_string())).ok()
    }
    // 1-bit patterns: expand to 8-bit grey.
    let (channels, depth): (Vec<Vec<u8>>, u16) = if p.depth == 1 {
        let row = w.div_ceil(8);
        let expand = |plane: &Vec<u8>, oom: &mut Option<String>| -> Option<Vec<u8>> {
            let mut out = Vec::new();
            reserve(&mut out, n, oom)?;
            out.extend((0..n).map(|i| if plane.get((i / w) * row + (i % w) / 8).is_some_and(|b| b & (0x80 >> (i % w % 8)) != 0) { 0 } else { 255 }));
            Some(out)
        };
        (p.channels.iter().map(|plane| expand(plane, oom)).collect::<Option<_>>()?, 8)
    } else {
        (p.channels.iter().map(|plane| copy_plane(plane, oom)).collect::<Option<_>>()?, p.depth)
    };
    let (mode, channels) = match p.mode {
        1 | 8 => (ColorMode::Grayscale, channels),
        2 => {
            let pal = p.palette.as_deref()?;
            let idx = channels.first()?;
            let ch: Vec<Vec<u8>> = (0..3)
                .map(|c| {
                    let mut out = Vec::new();
                    reserve(&mut out, idx.len(), oom)?;
                    out.extend(idx.iter().map(|i| pal.get(usize::from(*i) * 3 + c).copied().unwrap_or(0)));
                    Some(out)
                })
                .collect::<Option<_>>()?;
            (ColorMode::Rgb, ch)
        }
        4 => (ColorMode::Cmyk, channels),
        9 => (ColorMode::Lab, channels),
        3 => (ColorMode::Rgb, channels),
        _ => (ColorMode::Grayscale, channels.into_iter().take(1).collect()),
    };
    let s = sample_for_depth(depth);
    let fmt = PixelFormat::new(mode, s, p.alpha.is_some());
    let cc = mode_channels(psd_mode(mode));
    if channels.len() < cc {
        return None;
    }
    let mut refs: Vec<Option<&[u8]>> = channels.iter().take(cc).map(|c| Some(c.as_slice())).collect();
    let mut fill = vec![zero_sample(s); cc];
    let mut invert = vec![mode == ColorMode::Cmyk; cc];
    if let Some(a) = &p.alpha {
        refs.push(Some(a.as_slice()));
        fill.push(max_sample(s));
        invert.push(false);
    }
    let mut bytes = match try_interleave(&refs, &fill, n, s, &invert) {
        Ok(bytes) => bytes,
        Err(craft_psd::PsdError::OutOfMemory(error)) => {
            *oom = Some(error.to_string());
            return None;
        }
        Err(_) => return None,
    };
    if mode == ColorMode::Lab && s == SampleType::U16 {
        crate::pixels::lab16_chroma(&mut bytes, refs.len(), true);
    }
    let mut surface = Surface::new(fmt);
    let result = surface.try_write_interleaved(Rect::new(0, 0, w as i32, h as i32), &bytes);
    photocraft_raster::scratch::give(bytes);
    if let Err(error) = result {
        *oom = Some(error.to_string());
        return None;
    }
    Some(Pattern { id: p.id.clone(), name: p.name.clone(), width: p.width, height: p.height, surface })
}

/// A document pattern as a PSD pattern (its own depth and colour model; other models as RGB).
/// Dense export is separately bounded: oversized native declarations remain valid stored data.
pub fn to_psd(p: &Pattern) -> Result<PsdPattern, String> {
    let mut remaining = photocraft_doc::pattern::MAX_WORKING_BYTES;
    to_psd_in(p, &mut remaining)
}

/// Convert a library with one shared dense-plane allowance, for PAT and ASL writers.
pub(crate) fn to_psd_patterns(patterns: &[Pattern]) -> Result<Vec<PsdPattern>, String> {
    let mut remaining = photocraft_doc::pattern::MAX_WORKING_BYTES;
    patterns.iter().map(|p| to_psd_in(p, &mut remaining)).collect()
}

/// Share the retained plane allowance across every pattern in one exported file.
fn to_psd_in(p: &Pattern, remaining: &mut u64) -> Result<PsdPattern, String> {
    let fail = |why: &str| format!("pattern \"{}\" cannot be exported: {why}", p.display_name());
    if p.width == 0 || p.height == 0 || p.width > photocraft_doc::pattern::MAX_EDGE || p.height > photocraft_doc::pattern::MAX_EDGE {
        return Err(fail("invalid dimensions"));
    }
    if p.pixel_count() > photocraft_doc::pattern::MAX_PAINT_PIXELS {
        return Err(fail("declared pixel count exceeds the pattern export limit; keep the native .pcraft file to preserve it"));
    }
    let f = p.surface.format();
    let mode = match f.mode {
        ColorMode::Grayscale | ColorMode::Rgb | ColorMode::Cmyk | ColorMode::Lab => f.mode,
        _ => ColorMode::Rgb,
    };
    let fmt = PixelFormat::new(mode, f.sample, f.alpha);
    let required = p.pixel_count().checked_mul(fmt.bytes_per_pixel() as u64).ok_or_else(|| fail("pixel byte count overflow"))?;
    if required > *remaining {
        return Err(fail("dense pixels exceed the file's remaining pattern memory budget; keep the native .pcraft file to preserve it"));
    }
    let surf = if fmt == f { p.surface.clone() } else { p.surface.try_convert(fmt).map_err(|error| fail(&error.to_string()))? };
    let mut bytes = surf.try_to_interleaved(p.rect()).map_err(|error| fail(&error.to_string()))?;
    let ch = fmt.channels();
    if mode == ColorMode::Lab && fmt.sample == SampleType::U16 {
        crate::pixels::lab16_chroma(&mut bytes, ch, false);
    }
    let mut invert = vec![mode == ColorMode::Cmyk; ch];
    if fmt.alpha {
        invert[ch - 1] = false;
    }
    // Allocate each output plane fallibly; the interleaved input and retained planes
    // are each bounded by MAX_WORKING_BYTES, irrespective of sparse source storage.
    let plane_len = usize::try_from(required / ch as u64).map_err(|_| fail("plane byte count overflow"))?;
    let mut planes: Vec<Vec<u8>> = Vec::with_capacity(ch);
    for _ in 0..ch {
        let mut plane = Vec::new();
        plane.try_reserve_exact(plane_len).map_err(|error| fail(&error.to_string()))?;
        planes.push(plane);
    }
    for pixel in bytes.chunks_exact(fmt.bytes_per_pixel()) {
        for (plane, sample) in planes.iter_mut().zip(pixel.chunks_exact(fmt.sample.bytes())) {
            plane.extend_from_slice(sample);
        }
    }
    for (plane, invert) in planes.iter_mut().zip(invert) {
        if invert {
            crate::pixels::invert_plane(plane, fmt.sample);
        }
        if cfg!(target_endian = "little") {
            for sample in plane.chunks_exact_mut(fmt.sample.bytes()) {
                sample.reverse();
            }
        }
    }
    let alpha = fmt.alpha.then(|| planes.pop()).flatten();
    *remaining -= required;
    Ok(PsdPattern {
        mode: psd_mode(mode),
        width: p.width,
        height: p.height,
        name: p.name.clone(),
        id: p.id.clone(),
        palette: None,
        depth: psd_depth(fmt.sample),
        channels: planes,
        alpha,
    })
}

/// A warning for each of `patterns` too large to paint: such a pattern fills with transparency,
/// so the file says so on opening.
pub fn unpaintable_warnings(patterns: &[Pattern]) -> Vec<String> {
    patterns.iter().filter_map(|p| Some(format!("pattern \"{}\" would paint nothing: {}", p.display_name(), p.paint_problem()?))).collect()
}

/// Patterns decoded from the document's raw pattern blocks (malformed blocks are skipped). All
/// blocks share one memory budget, so repeating a block can't multiply it.
pub fn from_global_blocks(doc: &Document) -> Vec<Pattern> {
    try_from_global_blocks(doc).unwrap_or_default()
}

/// [`from_global_blocks`], failing with the allocation error instead of dropping the pattern it
/// hit, so an import reports that the file could not be opened in full.
pub fn try_from_global_blocks(doc: &Document) -> Result<Vec<Pattern>, String> {
    let mut budget = PatternBudget::default();
    let mut out = Vec::new();
    for (_, _, d) in doc.metadata.psd_global_blocks.iter().filter(|(_, k, _)| PATTERN_KEYS.contains(k)) {
        let patterns = match parse_pattern_block_with(d, &mut budget) {
            Ok(patterns) => patterns,
            Err(craft_psd::PsdError::OutOfMemory(error)) => return Err(error.to_string()),
            Err(_) => continue,
        };
        for p in patterns {
            out.extend(try_from_psd(&p)?);
        }
    }
    Ok(out)
}

/// Global blocks to write: the raw ones while they still match [`Document::patterns`] (see
/// [`repair_legacy_blocks`]), else the raw pattern blocks replaced by one written from the
/// document (at the first one's position).
pub fn export_global_blocks(doc: &Document) -> Result<Vec<photocraft_doc::PsdGlobalBlock>, String> {
    export_global_blocks_with_limit(doc, photocraft_doc::pattern::MAX_WORKING_BYTES)
}

/// The strict exporter with an explicit dense-plane allowance, so tests can exercise shared
/// admission using tiny patterns instead of allocating the production-sized budget.
pub(crate) fn export_global_blocks_with_limit(doc: &Document, limit: u64) -> Result<Vec<photocraft_doc::PsdGlobalBlock>, String> {
    let mut warnings = Vec::new();
    let blocks = export_global_blocks_with_warnings_limit(doc, &mut warnings, limit);
    if warnings.is_empty() { Ok(blocks) } else { Err(warnings.join("; ")) }
}

/// Export patterns through the PSD model's warning contract: unsafe dense replacements
/// are omitted with their names and reasons; unchanged raw pattern blocks stay verbatim.
pub fn export_global_blocks_with_warnings(doc: &Document, warnings: &mut Vec<String>) -> Vec<photocraft_doc::PsdGlobalBlock> {
    export_global_blocks_with_warnings_limit(doc, warnings, photocraft_doc::pattern::MAX_WORKING_BYTES)
}

fn export_global_blocks_with_warnings_limit(doc: &Document, warnings: &mut Vec<String>, limit: u64) -> Vec<photocraft_doc::PsdGlobalBlock> {
    let raw = &doc.metadata.psd_global_blocks;
    if from_global_blocks(doc) == doc.patterns {
        return repair_legacy_blocks(raw);
    }
    let key = block_key(psd_depth(doc.depth));
    let mut remaining = limit;
    let patterns: Vec<PsdPattern> = doc
        .patterns
        .iter()
        .filter_map(|pattern| match to_psd_in(pattern, &mut remaining) {
            Ok(pattern) => Some(pattern),
            Err(error) => {
                warnings.push(format!("{error}; omitted from PSD/PSB"));
                None
            }
        })
        .collect();
    let fresh = match write_pattern_block(&patterns) {
        Ok(bytes) => bytes,
        Err(error) => {
            warnings.push(format!("patterns could not be written to PSD/PSB: {error}"));
            Vec::new()
        }
    };
    let mut out = Vec::with_capacity(raw.len() + 1);
    let mut placed = false;
    for b in raw {
        if PATTERN_KEYS.contains(&b.1) {
            if !placed {
                out.push((*b"8BIM", key, Arc::new(fresh.clone())));
                placed = true;
            }
        } else {
            out.push(b.clone());
        }
    }
    if !placed && !doc.patterns.is_empty() {
        out.push((*b"8BIM", key, Arc::new(fresh)));
    }
    out
}

/// `raw` with each pattern block that keeps transparency where older PhotoCraft builds put it
/// (which Photoshop refuses, #8054) rewritten in place from its own patterns. Every other block,
/// unreadable pattern blocks included, is kept verbatim. Blocks are decoded in order with one
/// shared budget, as [`from_global_blocks`] decodes them.
fn repair_legacy_blocks(raw: &[photocraft_doc::PsdGlobalBlock]) -> Vec<photocraft_doc::PsdGlobalBlock> {
    if !raw.iter().any(|(_, k, d)| PATTERN_KEYS.contains(k) && block_has_user_mask(d)) {
        return raw.to_vec();
    }
    let mut budget = PatternBudget::default();
    raw.iter()
        .map(|b @ (sig, key, data)| {
            if !PATTERN_KEYS.contains(key) {
                return b.clone();
            }
            match parse_pattern_block_with(data, &mut budget) {
                Ok(ps) if block_has_user_mask(data) => match write_pattern_block(&ps) {
                    Ok(fresh) => (*sig, *key, Arc::new(fresh)),
                    Err(_) => b.clone(),
                },
                _ => b.clone(),
            }
        })
        .collect()
}

/// Reads a `.pat` pattern file.
pub fn read_pat(bytes: &[u8]) -> Result<Vec<Pattern>, String> {
    let ps = craft_psd::patterns::parse_pat_file(bytes).map_err(|e| format!("not a readable .pat file: {e}"))?;
    Ok(ps.iter().filter_map(from_psd).collect())
}

/// Writes patterns as a `.pat` file.
pub fn write_pat(patterns: &[Pattern]) -> Result<Vec<u8>, String> {
    let patterns = to_psd_patterns(patterns)?;
    craft_psd::patterns::write_pat_file(&patterns).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_color::SampleType;

    fn pattern(mode: ColorMode, s: SampleType, alpha: bool) -> Pattern {
        let fmt = PixelFormat::new(mode, s, alpha);
        let mut surf = Surface::new(fmt);
        let n = fmt.channels();
        for y in 0..3 {
            for x in 0..5 {
                let px: Vec<f32> = (0..n).map(|c| ((x * 3 + y * 7 + c as i32 * 5) % 11) as f32 / 10.0).collect();
                surf.write_pixel(x, y, &px);
            }
        }
        Pattern::new("P", surf, 5, 3)
    }

    #[test]
    fn patterns_round_trip_through_psd_at_every_depth_and_model() {
        for mode in [ColorMode::Rgb, ColorMode::Grayscale, ColorMode::Cmyk, ColorMode::Lab] {
            for s in [SampleType::U8, SampleType::U16, SampleType::F32] {
                for alpha in [false, true] {
                    let p = pattern(mode, s, alpha);
                    let back = from_psd(&to_psd(&p).unwrap()).unwrap();
                    assert_eq!((&back.id, &back.name, back.width, back.surface.format()), (&p.id, &p.name, p.width, p.surface.format()));
                    // Exact except float CMYK, whose stored inversion (1 − v) rounds.
                    let (a, b) = (back.surface.read_region(p.rect()), p.surface.read_region(p.rect()));
                    assert!(a.iter().zip(&b).all(|(x, y)| (x - y).abs() < 1e-6), "{mode:?} {s:?} {alpha}");
                    if !(mode == ColorMode::Cmyk && s == SampleType::F32) {
                        assert_eq!(back, p, "{mode:?} {s:?} {alpha}");
                    }
                }
            }
        }
    }

    #[test]
    fn export_reuses_raw_blocks_until_patterns_change() {
        let mut d = Document::new("x", photocraft_geom::Size { width: 4, height: 4 }, ColorMode::Rgb, SampleType::U8);
        let p = pattern(ColorMode::Rgb, SampleType::U8, false);
        let raw = write_pattern_block(&[to_psd(&p).unwrap()]).unwrap();
        d.metadata.psd_global_blocks = vec![(*b"8BIM", *b"Txt2", Arc::new(vec![1])), (*b"8BIM", *b"Patt", Arc::new(raw.clone()))];
        d.patterns = from_global_blocks(&d);
        assert_eq!(d.patterns.len(), 1);
        assert_eq!(export_global_blocks(&d).unwrap(), d.metadata.psd_global_blocks);
        d.patterns.push(pattern(ColorMode::Grayscale, SampleType::U16, true));
        let out = export_global_blocks(&d).unwrap();
        assert_eq!(out.len(), 2);
        assert_eq!(out[1].1, *b"Patt");
        let mut d2 = d.clone();
        d2.metadata.psd_global_blocks = out;
        assert_eq!(from_global_blocks(&d2), d.patterns);
        assert_eq!(read_pat(&write_pat(&d.patterns).unwrap()).unwrap(), d.patterns);
    }

    #[test]
    fn transparent_pattern_blocks_from_older_builds_are_rewritten_on_save() {
        // #8054: Define Pattern over transparency, then a Pattern Fill layer; Photoshop refused
        // the PSD because the pattern's transparency sat in the user mask array.
        let mut d = Document::new("x", photocraft_geom::Size { width: 4, height: 4 }, ColorMode::Rgb, SampleType::U16);
        d.patterns.push(pattern(ColorMode::Rgb, SampleType::U16, true));
        let out = export_global_blocks(&d).unwrap();
        assert_eq!(out.len(), 1);
        assert!(!block_has_user_mask(&out[0].2));
        // Move the transparency into the user mask array, as older builds wrote it: a 16-bit
        // pattern's array is raw, so it is the last 8 + 23 + 5 × 3 × 2 bytes of the pattern.
        let mut legacy = out[0].2.to_vec();
        let end = 4 + u32::from_be_bytes([legacy[0], legacy[1], legacy[2], legacy[3]]) as usize;
        legacy[end - (8 + 23 + 30) - 4..end].rotate_left(4);
        assert!(block_has_user_mask(&legacy));
        // A pattern block that doesn't decode (pattern version 2) sits beside it.
        let unreadable = (*b"8BIM", *b"Patt", Arc::new(vec![0, 0, 0, 8, 0, 0, 0, 2, 0, 0, 0, 0]));
        let other = (*b"8BIM", *b"Txt2", Arc::new(vec![1]));
        d.metadata.psd_global_blocks = vec![other.clone(), (*b"8BIM", *b"Pat2", Arc::new(legacy)), unreadable.clone()];
        assert_eq!(from_global_blocks(&d), d.patterns, "older files still open");
        // The patterns are unchanged: only the legacy block is rewritten, in place; the other
        // blocks are kept verbatim.
        let out = export_global_blocks(&d).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!((&out[0], &out[2]), (&other, &unreadable));
        assert_eq!(out[1].1, *b"Pat2");
        assert!(!block_has_user_mask(&out[1].2));
        let mut back = d.clone();
        back.metadata.psd_global_blocks = out;
        assert_eq!(from_global_blocks(&back), d.patterns);
        // Saving again keeps every block as it is.
        assert_eq!(export_global_blocks(&back).unwrap(), back.metadata.psd_global_blocks);
    }
}

#[cfg(test)]
mod unpaintable_tests {
    use super::*;
    use photocraft_doc::PixelFormat;
    use photocraft_raster::Surface;

    #[test]
    fn a_pattern_too_large_to_paint_is_named_on_opening() {
        let sparse = |name: &str, format: PixelFormat, side: u32| Pattern {
            id: name.into(),
            name: name.into(),
            width: side,
            height: side,
            surface: Surface::new(format),
        };
        let patterns =
            [sparse("Small", PixelFormat::RGBA8, 64), sparse("Huge float", PixelFormat::RGBA32F, 5000), sparse("Largest 8-bit", PixelFormat::RGBA8, 8000)];
        let warnings = unpaintable_warnings(&patterns);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("Huge float") && warnings[0].contains("would paint nothing") && warnings[0].contains("32 bits"), "{warnings:?}");
    }
}

#[cfg(test)]
mod allocation_tests {
    use super::*;

    #[test]
    fn shared_small_export_budget_is_an_error_without_warnings_and_explicit_with_warnings() {
        let mut doc = Document::new("Patterns", photocraft_geom::Size::new(1, 1), ColorMode::Rgb, SampleType::U8);
        for name in ["First", "Second"] {
            let mut surface = Surface::new(PixelFormat::RGBA8);
            surface.fill_rect(Rect::new(0, 0, 2, 2), &[1., 0., 0., 1.]);
            doc.patterns.push(Pattern { id: name.into(), name: name.into(), width: 2, height: 2, surface });
        }
        let before = doc.clone();
        let error = export_global_blocks_with_limit(&doc, 16).unwrap_err();
        assert!(error.contains("Second") && error.contains("memory budget"), "{error}");
        let mut warnings = Vec::new();
        let blocks = export_global_blocks_with_warnings_limit(&doc, &mut warnings, 16);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Second") && warnings[0].contains("omitted from PSD/PSB"), "{warnings:?}");
        let mut restored = doc.clone();
        restored.metadata.psd_global_blocks = blocks;
        assert_eq!(from_global_blocks(&restored), vec![doc.patterns[0].clone()]);
        assert_eq!(doc, before);
    }

    #[test]
    /// Preserve oversized native pattern declarations while refusing dense exports or reporting named PSD omissions.
    fn oversized_native_pattern_declarations_are_preserved_but_exported_without_expansion() {
        for depth in SampleType::ALL {
            for mode in [ColorMode::Rgb, ColorMode::Multichannel] {
                let mut doc = Document::new("Stored pattern declaration", photocraft_geom::Size::new(2, 2), mode, depth);
                let mut surface = Surface::new(PixelFormat::new(ColorMode::Rgb, depth, true));
                surface.write_pixel(0, 0, &[0.25, 0.5, 0.75, 1.]);
                doc.patterns.push(Pattern { id: "oversized".into(), name: "Oversized retained pattern".into(), width: 30_000, height: 30_000, surface });
                let bytes = photocraft_format::save_to_bytes(&doc, &Default::default()).unwrap();
                let loaded = photocraft_format::load_from_bytes(&bytes).unwrap();
                let before = loaded.clone();
                let pattern = &loaded.patterns[0];
                assert_eq!((pattern.width, pattern.height, pattern.surface.tile_count()), (30_000, 30_000, 1));
                assert!(!pattern.is_paintable());
                assert!(to_psd(pattern).unwrap_err().contains("pixel count"));
                assert!(write_pat(&loaded.patterns).unwrap_err().contains("Oversized retained pattern"));
                assert!(crate::asl_map::write_asl(&[], &loaded.patterns).unwrap_err().contains("Oversized retained pattern"));
                assert!(export_global_blocks(&loaded).unwrap_err().contains("Oversized retained pattern"));
                for force_psb in [false, true] {
                    let opts = crate::PsdExportOptions { force_psb, ..Default::default() };
                    let (file, warnings) = crate::document_to_psd_with(&loaded, &opts);
                    assert!(
                        warnings.iter().any(|w| w.contains("Oversized retained pattern") && w.contains("omitted from PSD/PSB")),
                        "{mode:?} {depth:?}: {warnings:?}"
                    );
                    assert!(file.global_blocks.iter().all(|b| !PATTERN_KEYS.contains(&b.key) || b.data.is_empty()));
                }
                assert_eq!(loaded, before);
                let saved = photocraft_format::save_to_bytes(&loaded, &Default::default()).unwrap();
                assert_eq!(photocraft_format::load_from_bytes(&saved).unwrap().patterns, loaded.patterns);
            }
        }
    }

    #[test]
    /// Charge retained pattern planes to one shared budget before expanding any sparse pattern pixels.
    fn pattern_export_plane_budget_is_shared_and_refused_before_sparse_expansion() {
        let p = Pattern {
            id: "float".into(),
            name: "Large float declaration".into(),
            width: 5000,
            height: 5000,
            surface: Surface::new(PixelFormat::new(ColorMode::Rgb, SampleType::F32, true)),
        };
        assert!(p.pixel_count() <= photocraft_doc::pattern::MAX_PAINT_PIXELS);
        assert!(to_psd(&p).unwrap_err().contains("memory budget"));
        // A tiny injected allowance proves that later patterns cannot reuse retained planes'
        // reservation, without creating a large buffer in the test.
        let small = Pattern { width: 2, height: 2, ..p };
        let mut remaining = 64;
        assert!(to_psd_in(&small, &mut remaining).is_ok());
        assert_eq!(remaining, 0);
        assert!(to_psd_in(&small, &mut remaining).unwrap_err().contains("memory budget"));
        assert_eq!(remaining, 0);
    }

    #[test]
    /// Keep safe patterns and unchanged raw blocks when an oversized pattern must be omitted from export.
    fn unsafe_pattern_omission_preserves_other_patterns_and_raw_blocks() {
        let mut doc = Document::new("Patterns", photocraft_geom::Size::new(2, 2), ColorMode::Rgb, SampleType::U8);
        let mut surface = Surface::new(PixelFormat::RGBA8);
        surface.write_pixel(0, 0, &[1., 0., 0., 1.]);
        let small = Pattern { id: "small".into(), name: "Small".into(), width: 2, height: 2, surface };
        doc.patterns.push(small.clone());
        let raw = export_global_blocks(&doc).unwrap();
        doc.metadata.psd_global_blocks = raw.clone();
        assert_eq!(export_global_blocks(&doc).unwrap(), raw, "unchanged raw patterns pass through");
        doc.patterns.push(Pattern { id: "large".into(), name: "Large".into(), width: 30_000, height: 30_000, surface: Surface::new(PixelFormat::RGBA8) });
        let mut warnings = Vec::new();
        let blocks = export_global_blocks_with_warnings(&doc, &mut warnings);
        assert_eq!(warnings.len(), 1);
        assert!(warnings[0].contains("Large") && warnings[0].contains("omitted"));
        doc.metadata.psd_global_blocks = blocks;
        assert_eq!(from_global_blocks(&doc), vec![small]);
    }

    #[test]
    fn malformed_dimensions_and_planes_fail_before_conversion() {
        let mut p = PsdPattern {
            mode: 1,
            width: 9,
            height: 2,
            name: "Bits".into(),
            id: "bits".into(),
            palette: None,
            depth: 1,
            channels: vec![vec![0x80, 0, 0, 0x80]],
            alpha: None,
        };
        let converted = from_psd(&p).unwrap();
        assert_eq!(converted.surface.to_interleaved(converted.rect()), [0, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 255, 0]);
        p.channels[0].pop();
        assert!(from_psd(&p).is_none());
        p.width = u32::MAX;
        assert!(from_psd(&p).is_none());
        p.width = 9;
        p.depth = 7;
        assert!(from_psd(&p).is_none());
    }
}
