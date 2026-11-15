//! Type layers read from a file, made ready for the canvas and the panels.

use photocraft_doc::{Document, Layer, LayerContent, TextLayer};

/// Largest type raster the import draws on its own (pixels). The model comes from an untrusted
/// file: an absurd type size, scale or box must not make opening it allocate gigabytes (the
/// renderer's own guard is far larger). A layer over it keeps empty pixels, as before, until
/// it is edited.
pub(crate) const MAX_RENDER_PIXELS: u64 = 64 * 1024 * 1024;

/// Bytes a pixel cache covering `area` holds: surfaces store whole tiles, so a 2x2 px shape
/// still keeps a tile.
pub(crate) fn cache_bytes(area: photocraft_geom::Rect, bytes_per_pixel: usize) -> u64 {
    if area.is_empty() {
        return 0;
    }
    let tile = i64::from(photocraft_geom::TILE_SIZE);
    let tiles = |a: i32, b: i32| (i64::from(b) - 1).div_euclid(tile) - i64::from(a).div_euclid(tile) + 1;
    let n = tiles(area.x0, area.x1) as u64 * tiles(area.y0, area.y1) as u64;
    n.saturating_mul((tile * tile) as u64).saturating_mul(bytes_per_pixel as u64)
}

/// Renders `t` from its model unless its ink would exceed [`MAX_RENDER_PIXELS`] or the
/// document-wide `budget` (bytes of cache still allowed). Returns `false` when the layer went
/// without pixels because of the budget.
fn render_bounded(eng: &mut photocraft_text::TextEngine, t: &mut TextLayer, dpi: f32, format: photocraft_color::PixelFormat, budget: &mut u64) -> bool {
    let empty = || photocraft_raster::Surface::new(photocraft_color::PixelFormat { alpha: true, ..format });
    let tile_bytes = cache_bytes(photocraft_geom::Rect::new(0, 0, 1, 1), photocraft_color::PixelFormat { alpha: true, ..format }.bytes_per_pixel());
    if *budget < tile_bytes {
        // Nothing with ink fits any more: don't lay the text out just to find that out.
        t.cache = Some(empty());
        return false;
    }
    let layout = eng.layout(t, dpi);
    let warp = photocraft_text::render::layout_deform(&layout, t);
    let ink = photocraft_text::render::ink_rect_warped(&layout, &t.transform, warp.as_ref());
    if u64::from(ink.width()).saturating_mul(u64::from(ink.height())) > MAX_RENDER_PIXELS {
        t.cache = Some(empty());
        return true;
    }
    let cost = cache_bytes(ink, photocraft_color::PixelFormat { alpha: true, ..format }.bytes_per_pixel());
    let Some(rest) = budget.checked_sub(cost) else {
        t.cache = Some(empty());
        return false;
    };
    *budget = rest;
    let r = photocraft_text::render::rasterize_warped(&layout, &t.transform, format, t.antialias, warp.as_ref());
    t.cache = Some(r.surface);
    true
}

/// Prepares the type layers of a document that was just imported:
/// - a run whose font came as a PostScript name (PSD files) gets the installed family that name
///   resolves to, as the layout does, so Missing Fonts and the Character panel see the real
///   family (`CCWildWords-Regular` is the family "CCWildWords", not the guessed "CC Wild Words");
/// - a type layer without pixels (ag-psd, GIMP and other writers leave the image data out and
///   let Photoshop re-render on open) is rendered from its model, instead of staying empty
///   until it is edited.
pub(crate) fn prepare(doc: &mut Document, txt2: Option<&photocraft_text::engine_data::Value>) {
    let mut unlimited = u64::MAX;
    let _ = prepare_within(doc, txt2, &mut unlimited, &photocraft_raster::Interrupt::NONE);
}

/// [`prepare`] with the pixels it may draw limited to `budget` bytes for the whole document
/// (type layers past it keep empty pixels, as past [`MAX_RENDER_PIXELS`]) and stopping early
/// when `ctl` is cancelled. Returns how many layers the budget left without pixels.
pub(crate) fn prepare_within(
    doc: &mut Document,
    txt2: Option<&photocraft_text::engine_data::Value>,
    budget: &mut u64,
    ctl: &photocraft_raster::Interrupt,
) -> Result<usize, photocraft_raster::Cancelled> {
    #[allow(clippy::too_many_arguments)]
    fn visit(
        layers: &mut [Layer],
        eng: &mut photocraft_text::TextEngine,
        dpi: f32,
        format: photocraft_color::PixelFormat,
        txt2: Option<&photocraft_text::engine_data::Value>,
        budget: &mut u64,
        ctl: &photocraft_raster::Interrupt,
        skipped: &mut usize,
    ) -> Result<(), photocraft_raster::Cancelled> {
        for l in layers {
            ctl.check()?;
            match &mut l.content {
                LayerContent::Text(t) => {
                    let mut changed = false;
                    for r in &mut t.runs {
                        if let Some(ps) = r.style.postscript_name.clone()
                            && !eng.fonts.faces(&r.style.font_family).iter().any(|f| f.postscript_name.as_deref() == Some(&ps))
                        {
                            let f = eng.fonts.resolve_postscript(&ps);
                            // Keep installed families, including legacy names whose faces are
                            // indexed under a typographic family. An exact face may recover a
                            // truncated hyphenated family; a heuristic only fills a missing one.
                            let installed = eng.fonts.has_family(&r.style.font_family);
                            let exact_recovery =
                                f.exact && (!installed || f.family.strip_prefix(&r.style.font_family).is_some_and(|suffix| suffix.starts_with('-')));
                            if exact_recovery || (!installed && eng.fonts.has_family(&f.family)) {
                                r.style.font_family = f.family;
                                r.style.weight = f.weight;
                                r.style.italic = f.italic;
                                changed = true;
                            }
                        }
                    }
                    if changed {
                        t.sync_summary();
                    }
                    photocraft_text::psd::refine_legacy_point_origin(t, dpi, eng, txt2);
                    if t.cache.is_none() && !render_bounded(eng, t, dpi, format, budget) {
                        *skipped += 1;
                    }
                }
                LayerContent::Group(g) => visit(&mut g.children, eng, dpi, format, txt2, budget, ctl, skipped)?,
                _ => {}
            }
        }
        Ok(())
    }
    let dpi = doc.resolution_dpi;
    let format = doc.pixel_format();
    let mut eng = photocraft_text::shared().lock().unwrap_or_else(|e| e.into_inner());
    let mut skipped = 0;
    visit(&mut doc.layers, &mut eng, dpi, format, txt2, budget, ctl, &mut skipped)?;
    Ok(skipped)
}
