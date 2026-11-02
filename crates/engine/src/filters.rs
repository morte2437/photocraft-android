//! `filter.*` commands (Photoshop's Filter menu) backed by `photocraft-algo`.
//!
//! Every command acts on the active pixel layer (or the cached pixels of a
//! smart object, recording a smart filter), respects the selection, and is
//! undoable. `filter.lastFilter` re-runs the most recent filter command.

use photocraft_algo::{
    self as algo, Distribution, FilterParams, PolarMode, Preserve, RadialMethod, RadialQuality, RippleSize, SpherizeMode, UndefinedAreas, WaveType,
};
use photocraft_doc::{LayerContent, SmartFilter};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

mod validation;
pub(crate) use validation::validate_params;

fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn i(p: &Value, k: &str, d: i32) -> i32 {
    // Clamped like the neighbouring filter params: an out-of-range value saturates instead of
    // wrapping through `as i32` (`3e9` used to come back as a negative offset).
    p.get(k).and_then(|v| v.as_i64().or_else(|| v.as_f64().map(|f| f.round() as i64))).map_or(d, |v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn s<'a>(p: &'a Value, k: &str, d: &'a str) -> &'a str {
    p.get(k).and_then(Value::as_str).unwrap_or(d)
}
fn seed(p: &Value) -> u32 {
    // New calls accept the entire documented u32 range; keep legacy record decoding otherwise.
    p.get("seed").and_then(Value::as_u64).and_then(|n| u32::try_from(n).ok()).unwrap_or_else(|| i(p, "seed", 0) as u32)
}
fn undefined(p: &Value) -> UndefinedAreas {
    match s(p, "undefinedAreas", "wrap") {
        "repeat" => UndefinedAreas::Repeat,
        "transparent" => UndefinedAreas::Transparent,
        _ => UndefinedAreas::Wrap,
    }
}

/// Minimum / Maximum › Preserve (Photoshop's default is Squareness).
fn preserve(p: &Value) -> Preserve {
    if s(p, "preserve", "squareness") == "roundness" { Preserve::Roundness } else { Preserve::Squareness }
}

/// One-step presets (no dialog): the history step they record (their own name, not the generic
/// algorithm's) and the fixed parameters, tuned to match the reference app's fixed-strength filters.
fn preset(id: &str) -> Option<(&'static str, FilterParams)> {
    Some(match id {
        "filter.blur.blur" => ("Blur", FilterParams::GaussianBlur { radius: 0.6 }),
        // Photoshop 27.11's Blur More is the 3×3 kernel [1 2 1; 2 2 2; 1 2 1] / 14 (its response
        // to a single white pixel, exactly).
        "filter.blur.blurMore" => (
            "Blur More",
            FilterParams::Custom {
                kernel: vec![
                    0.0, 0.0, 0.0, 0.0, 0.0, //
                    0.0, 1.0, 2.0, 1.0, 0.0, //
                    0.0, 2.0, 2.0, 2.0, 0.0, //
                    0.0, 1.0, 2.0, 1.0, 0.0, //
                    0.0, 0.0, 0.0, 0.0, 0.0,
                ],
                scale: 14.0,
                offset: 0.0,
            },
        ),
        "filter.sharpen.sharpen" => ("Sharpen", FilterParams::UnsharpMask { amount: 60.0, radius: 0.5, threshold: 0.0 }),
        "filter.sharpen.sharpenMore" => ("Sharpen More", FilterParams::UnsharpMask { amount: 150.0, radius: 0.6, threshold: 0.0 }),
        "filter.sharpen.sharpenEdges" => ("Sharpen Edges", FilterParams::UnsharpMask { amount: 100.0, radius: 0.8, threshold: 6.0 }),
        "filter.noise.despeckle" => ("Despeckle", FilterParams::SurfaceBlur { radius: 2.0, threshold: 12.0 }),
        _ => return None,
    })
}

/// Builds the algorithm parameters for a filter command id from JSON params
/// (Photoshop dialog units). Stored smart filters and previews retain their tolerant decoding;
/// new command invocations pass through `validate_params` before any document changes.
pub fn params_for(id: &str, p: &Value) -> Option<FilterParams> {
    if let Some((_, fp)) = preset(id) {
        return Some(fp);
    }
    Some(match id {
        "filter.blur.gaussianBlur" => FilterParams::GaussianBlur { radius: f(p, "radius", 1.0).clamp(0.1, 1000.0) },
        "filter.blur.boxBlur" => FilterParams::BoxBlur { radius: f(p, "radius", 1.0).clamp(1.0, 2000.0) },
        "filter.blur.motionBlur" => FilterParams::MotionBlur { angle: f(p, "angle", 0.0), distance: f(p, "distance", 10.0).clamp(1.0, 2000.0) },
        "filter.blur.radialBlur" => FilterParams::RadialBlur {
            amount: f(p, "amount", 10.0).clamp(1.0, 100.0),
            method: if s(p, "method", "spin") == "zoom" { RadialMethod::Zoom } else { RadialMethod::Spin },
            quality: match s(p, "quality", "good") {
                "draft" => RadialQuality::Draft,
                "best" => RadialQuality::Best,
                "good" => RadialQuality::Good,
                _ => RadialQuality::Good,
            },
            center_x: f(p, "centerX", 0.5),
            center_y: f(p, "centerY", 0.5),
        },
        "filter.blur.surfaceBlur" => {
            FilterParams::SurfaceBlur { radius: f(p, "radius", 5.0).clamp(1.0, 100.0), threshold: f(p, "threshold", 15.0).clamp(2.0, 255.0) }
        }
        "filter.sharpen.unsharpMask" => FilterParams::UnsharpMask {
            amount: f(p, "amount", 50.0).clamp(1.0, 500.0),
            radius: f(p, "radius", 1.0).clamp(0.1, 1000.0),
            threshold: f(p, "threshold", 0.0).clamp(0.0, 255.0),
        },
        "filter.sharpen.smartSharpen" => FilterParams::SmartSharpen {
            amount: f(p, "amount", 100.0).clamp(1.0, 500.0),
            radius: f(p, "radius", 1.0).clamp(0.1, 64.0),
            reduce_noise: f(p, "reduceNoise", 10.0).clamp(0.0, 100.0),
        },
        "filter.other.highPass" => FilterParams::HighPass { radius: f(p, "radius", 10.0).clamp(0.1, 1000.0) },
        "filter.noise.addNoise" => FilterParams::AddNoise {
            amount: f(p, "amount", 12.5).clamp(0.1, 400.0),
            distribution: if s(p, "distribution", "uniform") == "gaussian" { Distribution::Gaussian } else { Distribution::Uniform },
            monochromatic: b(p, "monochromatic", false),
            seed: seed(p),
        },
        "filter.noise.median" => FilterParams::Median { radius: f(p, "radius", 1.0).clamp(1.0, 500.0) },
        "filter.noise.dustAndScratches" => {
            FilterParams::DustAndScratches { radius: f(p, "radius", 1.0).clamp(1.0, 500.0), threshold: f(p, "threshold", 0.0).clamp(0.0, 255.0) }
        }
        "filter.other.minimum" => FilterParams::Minimum { radius: f(p, "radius", 1.0).clamp(0.2, 500.0), preserve: preserve(p) },
        "filter.other.maximum" => FilterParams::Maximum { radius: f(p, "radius", 1.0).clamp(0.2, 500.0), preserve: preserve(p) },
        "filter.other.offset" => FilterParams::Offset { horizontal: i(p, "horizontal", 0), vertical: i(p, "vertical", 0), undefined: undefined(p) },
        "filter.pixelate.mosaic" => FilterParams::Mosaic { cell_size: f(p, "cellSize", 10.0).clamp(2.0, 200.0) },
        "filter.stylize.emboss" => {
            FilterParams::Emboss { angle: f(p, "angle", 135.0), height: f(p, "height", 3.0).clamp(1.0, 100.0), amount: f(p, "amount", 100.0).clamp(1.0, 500.0) }
        }
        "filter.stylize.findEdges" => FilterParams::FindEdges,
        "filter.stylize.solarize" => FilterParams::Solarize,
        "filter.distort.twirl" => FilterParams::Twirl { angle: f(p, "angle", 50.0).clamp(-999.0, 999.0) },
        "filter.distort.pinch" => FilterParams::Pinch { amount: f(p, "amount", 50.0).clamp(-100.0, 100.0) },
        "filter.distort.spherize" => FilterParams::Spherize {
            amount: f(p, "amount", 100.0).clamp(-100.0, 100.0),
            mode: match s(p, "mode", "normal") {
                "horizontalOnly" => SpherizeMode::HorizontalOnly,
                "verticalOnly" => SpherizeMode::VerticalOnly,
                _ => SpherizeMode::Normal,
            },
        },
        "filter.distort.wave" => FilterParams::Wave {
            generators: i(p, "generators", 5).clamp(1, 999) as u32,
            wavelength_min: f(p, "wavelengthMin", 10.0).clamp(1.0, 998.0),
            wavelength_max: f(p, "wavelengthMax", 120.0).clamp(2.0, 999.0),
            amplitude_min: f(p, "amplitudeMin", 5.0).clamp(1.0, 998.0),
            amplitude_max: f(p, "amplitudeMax", 35.0).clamp(1.0, 999.0),
            wave_type: match s(p, "type", "sine") {
                "triangle" => WaveType::Triangle,
                "square" => WaveType::Square,
                _ => WaveType::Sine,
            },
            undefined: undefined(p),
            seed: seed(p),
        },
        "filter.distort.ripple" => FilterParams::Ripple {
            amount: f(p, "amount", 100.0).clamp(-999.0, 999.0),
            size: match s(p, "size", "medium") {
                "small" => RippleSize::Small,
                "large" => RippleSize::Large,
                _ => RippleSize::Medium,
            },
        },
        "filter.distort.polarCoordinates" => FilterParams::PolarCoordinates {
            mode: if s(p, "mode", "rectangularToPolar") == "polarToRectangular" { PolarMode::PolarToRectangular } else { PolarMode::RectangularToPolar },
        },
        _ => return crate::filters_ext::params_for(id, p),
    })
}

/// Runs filter command `id` with JSON `params` on `surf` (the pixel function behind every
/// `filter.*` command; smart filters re-run it on re-render). `bounds` frames distortions;
/// `selection` limits the result. Like layer filters, neighbourhood filters repeat the edge pixels
/// `None` for ids that aren't pixel filters. `Err` when the requesting session's authority denies
/// the filter's params (an ambient path the session may not read): the caller must surface the
/// error rather than render unfiltered pixels.
pub fn apply_filter_to_surface(
    id: &str,
    params: &Value,
    surf: &photocraft_raster::Surface,
    bounds: photocraft_geom::Rect,
    selection: Option<&photocraft_raster::Surface>,
    canvas: photocraft_geom::Rect,
) -> Result<Option<photocraft_raster::Surface>> {
    // Replays bypass command dispatch; nested paths must still obey the requesting session's
    // authority, including adjustment LUTs and plug-ins, before any resolver can read them.
    crate::jobs::authorize_render_command(id, params)?;
    // Liquify / Puppet Warp / Perspective Warp smart filters (distort_cmds).
    if let Some(out) = crate::distort_cmds::apply_to_surface(id, params, surf, canvas) {
        return Ok(Some(out));
    }
    // Lens Correction / Adaptive Wide Angle / Camera Raw smart filters (lens_cmds).
    if let Some(out) = crate::lens_cmds::apply_to_surface(id, params, surf, canvas) {
        return Ok(Some(out));
    }
    // Image › Adjustments recorded as smart filters (Levels, Curves, Shadows/Highlights… from PSD).
    if let Some(kind) = id.strip_prefix("image.adjustments.") {
        return Ok(crate::adjust_cmds::adjust_as_filter(kind, params, surf));
    }
    // WebAssembly plug-in smart filters (plugin_cmds).
    if id == crate::plugin_cmds::RUN {
        return Ok(crate::plugin_cmds::apply_to_surface(id, params, surf, selection, canvas));
    }
    let Some(mut fp) = params_for(id, params) else { return Ok(None) };
    // A map that can no longer be read (moved or deleted since) leaves the pixels alone, like an
    // unknown filter; the command itself already refused an unreadable map when it was added.
    if crate::filters_ext::resolve_map_path(&mut fp, params).is_err() {
        return Ok(None);
    }
    let sel_bounds = selection.map(photocraft_raster::Surface::content_bounds);
    let content = surf.content_bounds();
    let area = algo::output_area(&fp, content, bounds, sel_bounds);
    Ok(Some(algo::apply_in(surf, &fp, area, bounds, selection, canvas.union(&content))))
}

pub(crate) fn has_filterable_layer(s: &Session) -> std::result::Result<(), String> {
    // A targeted alpha channel or Quick Mask is filtered instead of the layer (channel_cmds).
    if crate::channel_cmds::edits_channel(s) {
        return Ok(());
    }
    let d = s.active().ok_or("no document open")?;
    let id = d.active_layer.ok_or("no active layer")?;
    let l = d.doc.layer(id).ok_or("no active layer")?;
    match &l.content {
        LayerContent::Raster(_) => Ok(()),
        LayerContent::Smart(sm) if sm.cache.is_some() => Ok(()),
        other => Err(format!("filters need a pixel layer (active layer is {} {} layer)", other.article(), other.kind_name())),
    }
}

pub(crate) fn run_filter(s: &mut Session, id: &str, p: &Value) -> Result<Value> {
    validate_params(id, p)?;
    // Session state some filters read (colours) becomes explicit params; external inputs resolve here.
    let mut prepared = crate::filters_ext::prepare(s, id, p);
    if id == "filter.distort.displace" {
        prepared = crate::filters_ext::displace_request(&prepared);
    }
    let p = &prepared;
    let mut fp = params_for(id, p).ok_or_else(|| EngineError::Other(format!("unknown filter {id}")))?;
    crate::filters_ext::resolve(s, &mut fp, p)?;
    let layer = crate::commands::layer_param(s, p)?;
    let label = preset(id).map_or(fp.label(), |(name, _)| name).to_string();
    let msg = label.clone();
    let mut params = p.clone();
    if let Value::Object(m) = &mut params {
        m.remove("__kind");
    }
    let colors = crate::color_channels::target(s, p)?;
    let id = id.to_string();
    let p = p.clone();
    let layer_id = layer.0;
    // A background job when started with `Session::start` (#210): the filter runs on a worker,
    // checking for cancellation before every tile.
    crate::jobs::edit_job(
        s,
        &label,
        move |doc, _, ctx| {
            let filter = |surf: &photocraft_raster::Surface, fp: &FilterParams, area, bounds, sel: Option<&photocraft_raster::Surface>, extent| {
                ctx.stage(0.0, 1.0, &msg, |ctl| algo::apply_in_with(surf, fp, area, bounds, sel, extent, ctl)).ok_or(EngineError::Cancelled)
            };
            let selection = doc.selection.clone();
            let sel_bounds = selection.as_ref().map(photocraft_raster::Surface::content_bounds);
            // Distortions centre on the selection when there is one, else the canvas. Clouds and
            // Fibers place and size their pattern by the canvas either way; the selection masks them.
            let doc_bounds = doc.bounds();
            let bounds = if fp.sizes_by_canvas() { doc_bounds } else { sel_bounds.filter(|b| !b.is_empty()).unwrap_or(doc_bounds) };
            // Neighbourhood filters repeat edge pixels at the canvas edge (or past it, where the
            // layer has off-canvas pixels) instead of fading in transparency, as Photoshop does.
            if let Some(surf) = crate::channel_cmds::channel_surface_for_filter(doc, Some(layer), &p)? {
                let content = doc_bounds.union(&surf.content_bounds());
                let area = algo::output_area(&fp, content, bounds, sel_bounds);
                *surf = filter(surf, &fp, area, bounds, selection.as_ref(), algo::separable::edge_extent(doc_bounds, content))?;
                return Ok(fp.clone());
            }
            // Filters that make pixels transparent (Color to Alpha) turn the Background into a
            // normal layer first, as the erasers do: it can't hold transparency.
            if fp.makes_transparency() {
                crate::extra_cmds::background_to_layer_for_mask(doc, layer);
            }
            let locks = doc.effective_locks(layer);
            let l = doc.layer_mut(layer).ok_or(EngineError::NoLayer(layer))?;
            if locks.pixels || locks.all {
                return Err(EngineError::Other(format!("Could not complete your request because the layer \"{}\" is locked", l.name)));
            }
            // Putting the old alpha back (below) would leave unmixed colours fully opaque.
            if locks.transparency && fp.makes_transparency() && matches!(l.content, LayerContent::Raster(_)) {
                return Err(EngineError::Other(format!("Could not complete your request because the transparency of layer \"{}\" is locked", l.name)));
            }
            let mut fp = fp.clone();
            crate::filters_ext::resolve_in_layer(&mut fp, l, bounds);
            let surf = match &mut l.content {
                LayerContent::Raster(surf) => surf,
                LayerContent::Smart(_) => {
                    // A smart filter re-renders from its params alone, without the document's
                    // channels, so Lens Blur's alpha-channel depth would be lost (#7400).
                    if matches!(fp, FilterParams::LensBlur { depth: algo::DepthSource::Channel, .. }) {
                        return Err(EngineError::Other("Lens Blur can't read an alpha channel as depth on a Smart Object: rasterize the layer first".into()));
                    }
                    // A smart filter re-renders from its params alone: a Displace map read from an
                    // open document, which the re-render can't read again, is kept in them, as
                    // Photoshop embeds it (#7423). A `mapPath` stays a path, read next to the
                    // document on each re-render.
                    let params = match &fp {
                        FilterParams::Displace { map: Some(map), .. }
                            if photocraft_io::smart_map::displace_embedded_map(&params).is_none() && crate::filters_ext::map_path(&params).is_none() =>
                        {
                            photocraft_io::smart_map::displace_embed_map(&params, map).ok_or_else(|| {
                                EngineError::Other(
                                    "Displace: the displacement map is too large to keep in a Smart Object's filter: rasterize the layer first".into(),
                                )
                            })?
                        }
                        _ => params.clone(),
                    };
                    // Non-destructive: record the filter and re-render the smart object from its source.
                    let sf = SmartFilter { command: id.clone(), params, blend: photocraft_color::BlendMode::Normal, opacity: 1.0, visible: true };
                    return crate::smart_cmds::add_smart_filter(doc, layer, sf, selection.as_ref()).map(|()| fp);
                }
                _ => return Err(EngineError::Other("not a pixel layer".into())),
            };
            if let Some(colors) = colors {
                if fp.makes_transparency() {
                    return Err(EngineError::Other("this filter requires composite channels".into()));
                }
                // A grayscale plane cannot distinguish an opaque black border from its zero
                // default. Keep the native surface's extent when finding the filter support.
                let content = surf.content_bounds();
                let extent = doc_bounds.union(&content);
                let opaque_default = !surf.format().alpha || surf.default_pixel().last().is_some_and(|alpha| *alpha > 0.0);
                let support = if opaque_default { extent } else { content };
                let mut area = algo::output_area(&fp, support, bounds, sel_bounds).intersect(&extent);
                // Colour-only writes cannot create opacity, so generators need not materialize
                // invisible samples across empty parts of a transparent layer.
                if !opaque_default && fp.renders_from_nothing() {
                    area = area.intersect(&content);
                }
                // Keep the full native input context: filters can read transparency as a depth
                // or texture map, and colour-valued settings belong to the source colour model.
                let before = surf.clone();
                *surf = filter(surf, &fp, area, bounds, selection.as_ref(), extent)?;
                crate::color_channels::restrict_surface_in(&before, surf, colors, extent)?;
                return Ok(fp);
            }
            let before = locks.transparency.then(|| surf.clone());
            let content = surf.content_bounds();
            let area = algo::output_area(&fp, content, bounds, sel_bounds);
            *surf = filter(surf, &fp, area, bounds, selection.as_ref(), algo::separable::edge_extent(doc_bounds, content))?;
            if let Some(before) = &before {
                keep_alpha(before, surf, &|| ctx.cancelled())?;
            }
            Ok(fp)
        },
        move |fp| json!({ "layer": layer_id, "filter": serde_json::to_value(&fp).unwrap_or(Value::Null) }),
    )
}

/// Lock transparency: put the layer's alpha back after a filter, in the tiles it changed.
///
/// The tiles are independent, so they are fixed in parallel, each in a scratch surface of its
/// own, and `cancelled` is asked before each one: on a 24 MP layer (a Background, whose
/// transparency is locked) a serial pass took about 400 ms that a cancelled job had to sit out
/// (#6205). `Err(Cancelled)` leaves `new` holding all its tiles, some of them not yet fixed.
pub(crate) fn keep_alpha(old: &photocraft_raster::Surface, new: &mut photocraft_raster::Surface, cancelled: &(dyn Fn() -> bool + Sync)) -> Result<()> {
    use photocraft_raster::Surface;
    use rayon::prelude::*;
    if old.format() != new.format() || !new.format().alpha {
        return Ok(());
    }
    let mut coords: Vec<_> = new.tiles().filter(|(c, t)| old.tile(**c).is_none_or(|o| !std::sync::Arc::ptr_eq(o, t))).map(|(c, _)| *c).collect();
    coords.extend(old.tiles().filter(|(c, _)| new.tile(**c).is_none()).map(|(c, _)| *c));
    let (fmt, n) = (new.format(), new.channels());
    let default = new.default_pixel();
    // Each changed tile alone in a surface (a tile `new` lacks is taken as its default pixel, as
    // `write_region` would create it).
    let mut tiles: Vec<(photocraft_geom::Rect, Surface)> = coords
        .iter()
        .map(|c| {
            let mut s = Surface::with_default(fmt, &default);
            s.put_tiles(new.take_tiles(c.rect()));
            (c.rect(), s)
        })
        .collect();
    let fixed = tiles
        .par_iter_mut()
        .map(|(r, s)| {
            if cancelled() {
                return false;
            }
            let o = old.read_region(*r);
            let mut v = s.read_region(*r);
            for (pn, po) in v.chunks_exact_mut(n).zip(o.chunks_exact(n)) {
                if let (Some(a), Some(b)) = (pn.last_mut(), po.last()) {
                    *a = *b;
                }
            }
            s.write_region(*r, &v);
            true
        })
        .reduce(|| true, |a, b| a && b);
    for (_, s) in tiles {
        new.put_tiles(s.tiles().map(|(c, t)| (*c, t.clone())));
    }
    if !fixed {
        return Err(EngineError::Cancelled);
    }
    new.prune();
    Ok(())
}

macro_rules! filter_cmd {
    ($id:literal, $label:literal, [$($m:literal),*], $params:literal) => {
        CommandSpec {
            id: $id,
            label: $label,
            menu: &[$($m),*],
            shortcut: None,
            params: $params,
            enabled: has_filterable_layer,
            run: |s, p| run_filter(s, $id, p),
            journal: true,
        }
    };
}

/// The filter command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        filter_cmd!("filter.blur.gaussianBlur", "Gaussian Blur…", ["Filter", "Blur"], r##"{"radius":0.1..1000=1}"##),
        filter_cmd!("filter.blur.blur", "Blur", ["Filter", "Blur"], "{}"),
        filter_cmd!("filter.blur.blurMore", "Blur More", ["Filter", "Blur"], "{}"),
        filter_cmd!("filter.sharpen.sharpen", "Sharpen", ["Filter", "Sharpen"], "{}"),
        filter_cmd!("filter.sharpen.sharpenMore", "Sharpen More", ["Filter", "Sharpen"], "{}"),
        filter_cmd!("filter.sharpen.sharpenEdges", "Sharpen Edges", ["Filter", "Sharpen"], "{}"),
        filter_cmd!("filter.noise.despeckle", "Despeckle", ["Filter", "Noise"], "{}"),
        filter_cmd!("filter.blur.boxBlur", "Box Blur…", ["Filter", "Blur"], r##"{"radius":1..2000=1}"##),
        filter_cmd!("filter.blur.motionBlur", "Motion Blur…", ["Filter", "Blur"], r##"{"angle":-360..360=0,"distance":1..2000=10}"##),
        filter_cmd!(
            "filter.blur.radialBlur",
            "Radial Blur…",
            ["Filter", "Blur"],
            r##"{"amount":1..100=10,"method":"spin|zoom","quality":"draft|good|best"="good","centerX":0..1=0.5,"centerY":0..1=0.5}"##
        ),
        filter_cmd!("filter.blur.surfaceBlur", "Surface Blur…", ["Filter", "Blur"], r##"{"radius":1..100=5,"threshold":2..255=15}"##),
        filter_cmd!(
            "filter.sharpen.unsharpMask",
            "Unsharp Mask…",
            ["Filter", "Sharpen"],
            r##"{"amount":1..500=50,"radius":0.1..1000=1,"threshold":0..255=0}"##
        ),
        filter_cmd!(
            "filter.sharpen.smartSharpen",
            "Smart Sharpen…",
            ["Filter", "Sharpen"],
            r##"{"amount":1..500=100,"radius":0.1..64=1,"reduceNoise":0..100=10}"##
        ),
        filter_cmd!(
            "filter.noise.addNoise",
            "Add Noise…",
            ["Filter", "Noise"],
            r##"{"amount":0.1..400=12.5,"distribution":"uniform|gaussian","monochromatic":bool,"seed":u32=0}"##
        ),
        filter_cmd!("filter.noise.median", "Median…", ["Filter", "Noise"], r##"{"radius":1..500=1}"##),
        filter_cmd!("filter.noise.dustAndScratches", "Dust & Scratches…", ["Filter", "Noise"], r##"{"radius":1..500=1,"threshold":0..255=0}"##),
        filter_cmd!("filter.pixelate.mosaic", "Mosaic…", ["Filter", "Pixelate"], r##"{"cellSize":2..200=10}"##),
        filter_cmd!("filter.stylize.emboss", "Emboss…", ["Filter", "Stylize"], r##"{"angle":-180..180=135,"height":1..100=3,"amount":1..500=100}"##),
        filter_cmd!("filter.stylize.findEdges", "Find Edges", ["Filter", "Stylize"], "{}"),
        filter_cmd!("filter.stylize.solarize", "Solarize", ["Filter", "Stylize"], "{}"),
        filter_cmd!("filter.distort.twirl", "Twirl…", ["Filter", "Distort"], r##"{"angle":-999..999=50}"##),
        filter_cmd!("filter.distort.pinch", "Pinch…", ["Filter", "Distort"], r##"{"amount":-100..100=50}"##),
        filter_cmd!("filter.distort.spherize", "Spherize…", ["Filter", "Distort"], r##"{"amount":-100..100=100,"mode":"normal|horizontalOnly|verticalOnly"}"##),
        filter_cmd!(
            "filter.distort.wave",
            "Wave…",
            ["Filter", "Distort"],
            r##"{"generators":1..999=5,"wavelengthMin":1..998=10,"wavelengthMax":2..999=120,"amplitudeMin":1..998=5,"amplitudeMax":1..999=35,"type":"sine|triangle|square","undefinedAreas":"wrap|repeat","seed":u32=0}"##
        ),
        filter_cmd!("filter.distort.ripple", "Ripple…", ["Filter", "Distort"], r##"{"amount":-999..999=100,"size":"small|medium|large"}"##),
        filter_cmd!("filter.distort.polarCoordinates", "Polar Coordinates…", ["Filter", "Distort"], r##"{"mode":"rectangularToPolar|polarToRectangular"}"##),
        filter_cmd!("filter.other.highPass", "High Pass…", ["Filter", "Other"], r##"{"radius":0.1..1000=10}"##),
        filter_cmd!("filter.other.minimum", "Minimum…", ["Filter", "Other"], r##"{"radius":0.2..500=1,"preserve":"squareness|roundness"}"##),
        filter_cmd!("filter.other.maximum", "Maximum…", ["Filter", "Other"], r##"{"radius":0.2..500=1,"preserve":"squareness|roundness"}"##),
        filter_cmd!(
            "filter.other.offset",
            "Offset…",
            ["Filter", "Other"],
            r##"{"horizontal":px=0,"vertical":px=0,"undefinedAreas":"wrap|repeat|transparent"}"##
        ),
        CommandSpec {
            id: "filter.lastFilter",
            label: "Last Filter",
            menu: &["Filter"],
            shortcut: Some("Cmd+Alt+F"),
            params: "{}",
            enabled: |s| {
                has_filterable_layer(s)?;
                last_filter(s).map(|_| ()).ok_or_else(|| "no filter applied yet".into())
            },
            run: |s, p| {
                let (id, mut params) = last_filter(s).ok_or_else(|| EngineError::Other("no filter applied yet".into()))?;
                if let Some(target) = p.get("target") {
                    if params.is_null() {
                        params = json!({});
                    }
                    if let Some(object) = params.as_object_mut() {
                        object.insert("target".into(), target.clone());
                    }
                }
                crate::param_check::check(crate::commands::find(&id).ok_or_else(|| EngineError::Other("unknown saved filter".into()))?, &params)?;
                run_filter(s, &id, &params)
            },
            journal: true,
        },
    ]
}

/// The most recent filter command in the session journal, retargeted at the active layer: dialogs
/// such as the Filter Gallery record the layer they ran on, and Last Filter applies to the
/// current layer, as in Photoshop (#1270).
pub fn last_filter(s: &Session) -> Option<(String, Value)> {
    let (id, mut params) = s.journal.iter().rev().find(|(id, _)| params_for(id, &Value::Null).is_some() && id.starts_with("filter.")).cloned()?;
    if let Value::Object(m) = &mut params {
        m.remove("layer");
        m.remove("target");
    }
    Some((id, params))
}

#[cfg(test)]
mod tests;
