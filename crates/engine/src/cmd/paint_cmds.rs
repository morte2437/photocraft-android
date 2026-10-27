//! Paint helpers: Paint Bucket and Gradient tool.

use photocraft_algo::paint::{FillMode, GradientShape, bucket_fill_inks, bucket_fill_region_inks, paint_gradient};
use serde_json::{Value, json};

use crate::commands::{CommandSpec, blend_from_str};
use crate::presets::gradients;
use crate::{EngineError, Result, Session};

fn f(p: &Value, k: &str, d: f32) -> f32 {
    p.get(k).and_then(Value::as_f64).map_or(d, |v| v as f32)
}
fn b(p: &Value, k: &str, d: bool) -> bool {
    p.get(k).and_then(Value::as_bool).unwrap_or(d)
}
fn color(v: Option<&Value>, d: [f32; 4]) -> [f32; 4] {
    match v {
        Some(Value::Array(a)) if a.len() >= 3 => {
            let c: Vec<f32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            [c[0], c[1], c[2], c.get(3).copied().unwrap_or(1.0)]
        }
        Some(Value::String(h)) => {
            let h = h.trim_start_matches('#');
            let c = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| f32::from(v) / 255.0);
            match (c(0), c(2), c(4)) {
                (Some(r), Some(g), Some(b)) => [r, g, b, c(6).unwrap_or(1.0)],
                _ => d,
            }
        }
        _ => d,
    }
}
fn point(p: &Value, k: &str) -> Option<(f32, f32)> {
    let a = p.get(k)?.as_array()?;
    Some((a.first()?.as_f64()? as f32, a.get(1)?.as_f64()? as f32))
}

/// `opacity` (1..100 %) as a coverage in 0.01..=1, clamped like the Magic Eraser's: a value
/// above 1 would overshoot the blend (#4167).
fn opacity(p: &Value, cmd: &str) -> Result<f32> {
    Ok(crate::eraser_cmds::number(p, cmd, "opacity", 100.0, 1.0..=100.0)? as f32 / 100.0)
}

/// The Paint Bucket's `mode`: a blend mode (not Pass Through, a group's) or `clear`.
fn fill_mode(p: &Value) -> Result<FillMode> {
    let bad = |msg: String| EngineError::BadParams { cmd: "paint.bucket".into(), msg };
    match p.get("mode") {
        None | Some(Value::Null) => Ok(FillMode::Blend(photocraft_color::BlendMode::Normal)),
        Some(Value::String(m)) if m.eq_ignore_ascii_case("clear") => Ok(FillMode::Clear),
        Some(Value::String(m)) => blend_from_str(m)
            .filter(|m| *m != photocraft_color::BlendMode::PassThrough)
            .map(FillMode::Blend)
            .ok_or_else(|| bad(format!("unknown blend mode `{m}`"))),
        Some(v) => Err(bad(format!("`mode` must be a blend mode name or `clear`, not {v}"))),
    }
}

fn bucket(s: &mut Session, p: &Value) -> Result<Value> {
    let fg = s.tools.foreground;
    let c = color(p.get("color"), fg);
    let (x, y) = (f(p, "x", 0.0).floor() as i32, f(p, "y", 0.0).floor() as i32);
    let (tol, contiguous, aa) = (f(p, "tolerance", 32.0), b(p, "contiguous", true), b(p, "antiAlias", true));
    let opacity = opacity(p, "paint.bucket")?;
    let mode = fill_mode(p)?;
    let all = b(p, "sampleAllLayers", false) && crate::channel_cmds::target_of(p) == crate::channel_cmds::Target::Pixels;
    // Other visible layers can define the region, but the fill still writes only to the target.
    let region = if all {
        let (area, img) = crate::selection_cmds::sample_rgba8(s, true)?;
        let region = photocraft_algo::selection::wand_region(&img, area, (x, y), tol, contiguous, aa);
        drop(img);
        region
    } else {
        None
    };
    // Fill source: foreground colour (default) or a pattern (the Paint Bucket "Fill" dropdown).
    let source = p.get("contents").or_else(|| p.get("source")).and_then(Value::as_str).unwrap_or("foreground");
    let pattern = if source == "pattern" {
        let pat = crate::pattern_cmds::resolve_param(s, "paint.bucket", p)?;
        let tile = photocraft_compose::pattern::Tile::new(&pat)
            .ok_or_else(|| EngineError::BadParams { cmd: "paint.bucket".into(), msg: "the pattern is empty or too large to paint".into() })?;
        let (scale, angle, _, phase) = crate::pattern_cmds::placement("paint.bucket", p)?;
        Some((tile, scale, angle, phase))
    } else {
        None
    };
    // A foreground picked as CMYK fills a CMYK document with the picked inks (#4196).
    let inks = s.tools.picked_cmyk([c[0], c[1], c[2]]).filter(|_| c[3] >= 1.0 && pattern.is_none());
    let filled = s.edit("Paint Bucket", |doc, active| {
        let area = doc.bounds();
        let sel = doc.selection.clone();
        let (surf, lock) = crate::channel_cmds::target_surface(doc, *active, p)?;
        let ok = if all && region.is_none() {
            false
        } else if let Some((tile, scale, angle, phase)) = &pattern {
            // Render the pattern over the canvas once, then sample it at each filled pixel.
            let place = photocraft_compose::pattern::Placement::new(photocraft_geom::Rect::EMPTY, false, *phase, *scale, *angle);
            let rendered = photocraft_compose::pattern::render(tile, &place, area);
            let w = area.width() as usize;
            let src = |px: i32, py: i32| rendered[(py - area.y0) as usize * w + (px - area.x0) as usize];
            if let Some(region) = &region {
                bucket_fill_region_inks(surf, region, opacity, sel.as_ref(), lock, src, None, mode);
                true
            } else {
                bucket_fill_inks(surf, area, (x, y), tol, contiguous, aa, opacity, sel.as_ref(), lock, src, None, mode)
            }
        } else if let Some(region) = &region {
            bucket_fill_region_inks(surf, region, opacity, sel.as_ref(), lock, |_, _| c, inks, mode);
            true
        } else {
            bucket_fill_inks(surf, area, (x, y), tol, contiguous, aa, opacity, sel.as_ref(), lock, |_, _| c, inks, mode)
        };
        surf.prune();
        Ok(ok)
    })?;
    Ok(json!({ "filled": filled }))
}

fn gradient(s: &mut Session, p: &Value) -> Result<Value> {
    let from = point(p, "from").ok_or_else(|| EngineError::BadParams { cmd: "paint.gradient".into(), msg: "missing from".into() })?;
    let to = point(p, "to").ok_or_else(|| EngineError::BadParams { cmd: "paint.gradient".into(), msg: "missing to".into() })?;
    let shape = match p.get("style").and_then(Value::as_str).unwrap_or("linear") {
        "radial" => GradientShape::Radial,
        "angle" => GradientShape::Angle,
        "reflected" => GradientShape::Reflected,
        "diamond" => GradientShape::Diamond,
        _ => GradientShape::Linear,
    };
    let (fg, bg) = (s.tools.foreground, s.tools.background);
    // A preset (`gradient`), explicit `stops`, or the current gradient (Gradients panel); the
    // legacy `colors` list spaces its colours evenly. Explicit `transparency` applies to all.
    let stops: Vec<(f32, [f32; 4])> = match gradients::tool_stops(s, p)? {
        Some(st) => st,
        None => {
            let colors: Vec<[f32; 4]> =
                p.get("colors").and_then(Value::as_array).map(|a| a.iter().map(|v| color(Some(v), fg)).collect()).unwrap_or_else(|| vec![fg, bg]);
            let n = colors.len();
            let cs = colors.into_iter().enumerate().map(|(i, c)| (if n > 1 { i as f32 / (n - 1) as f32 } else { 0.0 }, c)).collect();
            gradients::apply_opacity(cs, &gradients::transparency_param(p, "paint.gradient")?.unwrap_or_default())
        }
    };
    // Photoshop's interpolation (`method`, with `smoothness` 0..=100, default 100) baked into dense
    // stops, as for Gradient Overlay: Photoshop 27.11's Classic black → white is 0.037 at t = 1/16
    // where straight stops give 0.0625.
    let stops = match p.get("method").and_then(Value::as_str).and_then(photocraft_io::GradientMethod::from_name) {
        Some(m) if stops.len() >= 2 => {
            let colors = stops.iter().map(|(t, c)| (*t, photocraft_color::Color::rgba(c[0], c[1], c[2], c[3]))).collect();
            let mids = vec![0.5; stops.len() - 1];
            photocraft_io::bake_gradient(colors, &mids, (f(p, "smoothness", 100.0) / 100.0).clamp(0.0, 1.0), m)
                .into_iter()
                .map(|(t, c)| {
                    let rgb = c.to_rgb();
                    (t, [rgb[0], rgb[1], rgb[2], c.alpha])
                })
                .collect()
        }
        _ => stops,
    };
    let reverse = b(p, "reverse", false);
    let opacity = opacity(p, "paint.gradient")?;
    let blend = p.get("mode").and_then(Value::as_str).and_then(blend_from_str).unwrap_or(photocraft_color::BlendMode::Normal);
    let dither = b(p, "dither", true); // Photoshop dithers gradients by default (reduces banding).
    s.edit("Gradient", |doc, active| {
        let sel = doc.selection.clone();
        let area = sel.as_ref().map(|m| m.content_bounds()).filter(|r| !r.is_empty()).unwrap_or_else(|| doc.bounds()).intersect(&doc.bounds());
        let (surf, lock) = crate::channel_cmds::target_surface(doc, *active, p)?;
        paint_gradient(surf, area, from, to, shape, &stops, reverse, opacity, blend, dither, sel.as_ref(), lock);
        Ok(())
    })?;
    Ok(Value::Null)
}

/// Paint helper command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "paint.bucket",
            label: "Paint Bucket",
            menu: &[],
            shortcut: None,
            params: r##"{"x":px,"y":px,"tolerance":0..255=32,"contiguous":bool=true,"antiAlias":bool=true,"sampleAllLayers":bool=false,"contents":"foreground|pattern"="foreground","color":"#rrggbb"=foreground,"pattern":id|name (contents=pattern),"scale":%=100,"angle":deg,"opacity":1..100=100,"mode":"normal|multiply|…|clear (erases the filled area)"="normal","target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}=Channels panel target}"##,
            enabled: crate::commands::has_paintable,
            run: bucket,
            journal: true,
        },
        CommandSpec {
            id: "paint.gradient",
            label: "Gradient",
            menu: &[],
            shortcut: None,
            params: r##"{"from":[x,y],"to":[x,y],"style":"linear|radial|angle|reflected|diamond"="linear","colors":["#rrggbb",…]? (evenly spaced),"gradient":preset name?,"stops":[[t,"#rrggbb"|"foreground"|"background"],…]?,"transparency":[[t,0..100],…]? (default: the current gradient, see gradient.presets.select),"method":"classic|perceptual|linear|smooth"? (Photoshop's interpolation; omitted: straight stops),"smoothness":0..100=100,"reverse":bool=false,"dither":bool=true,"opacity":1..100=100,"mode":"normal|multiply|…"="normal","target":"pixels"|"mask"|"filterMask"|"quickMask"|{"channel":i}=Channels panel target}"##,
            enabled: crate::commands::has_paintable,
            run: gradient,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests;
