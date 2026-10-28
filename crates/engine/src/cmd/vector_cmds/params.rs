use super::{
    Color, EngineError, Fill, FillRule, GradientStyle, Knot, LayerId, LineCap, LineJoin, LiveShape, Path, PathOp, Point, Result, Session, ShapeStroke,
    StrokeAlign, Subpath, Value, bad, json,
};

pub(crate) fn layer_id(s: &Session, p: &Value) -> Result<LayerId> {
    match p.get("layer").and_then(Value::as_u64) {
        Some(id) => Ok(LayerId(id)),
        None => s.active().and_then(|d| d.active_layer).ok_or(EngineError::Other("no active layer".into())),
    }
}

pub(super) fn f64p(p: &Value, k: &str) -> Option<f64> {
    p.get(k).and_then(Value::as_f64)
}

pub(crate) fn pt(v: &Value) -> Option<Point> {
    match v {
        Value::Array(a) if a.len() >= 2 => Some(Point::new(a[0].as_f64()?, a[1].as_f64()?)),
        Value::Object(_) => Some(Point::new(v.get("x")?.as_f64()?, v.get("y")?.as_f64()?)),
        _ => None,
    }
}

pub(crate) fn nums<const N: usize>(p: &Value, k: &str) -> Option<[f64; N]> {
    let a = p.get(k)?.as_array()?;
    if a.len() < N {
        return None;
    }
    let mut out = [0.0; N];
    for (o, v) in out.iter_mut().zip(a) {
        *o = v.as_f64()?;
    }
    Some(out)
}

/// `[tl, tr, br, bl]` from an array or a single number.
pub(super) fn radii(p: &Value) -> Option<[f64; 4]> {
    match p.get("radii").or_else(|| p.get("radius"))? {
        Value::Number(n) => n.as_f64().map(|r| [r; 4]),
        _ => nums::<4>(p, "radii"),
    }
}

pub(crate) fn color(v: &Value) -> Option<Color> {
    match v {
        Value::Array(a) if a.len() >= 3 => {
            let c: Vec<f32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
            Some(Color::rgba(c[0], c[1], c[2], c.get(3).copied().unwrap_or(1.0)))
        }
        Value::String(s) => {
            let s = s.trim_start_matches('#');
            let h = |i: usize| s.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|v| f32::from(v) / 255.0);
            match s.len() {
                6 => Some(Color::rgb(h(0)?, h(2)?, h(4)?)),
                8 => Some(Color::rgba(h(0)?, h(2)?, h(4)?, h(6)?)),
                _ => None,
            }
        }
        // A stored `Color` (mode + components), so a CMYK or Lab gradient stop survives a
        // read-modify-write through the shape swatches.
        Value::Object(_) => serde_json::from_value::<Color>(v.clone())
            .ok()
            .filter(|c| c.c.iter().all(|x| x.is_finite()) && c.alpha.is_finite() && (0.0..=1.0).contains(&c.alpha)),
        _ => None,
    }
}

fn hex(c: &Color) -> String {
    let [r, g, b] = c.to_rgb();
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    if c.alpha < 1.0 { format!("#{:02x}{:02x}{:02x}{:02x}", q(r), q(g), q(b), q(c.alpha)) } else { format!("#{:02x}{:02x}{:02x}", q(r), q(g), q(b)) }
}

pub(super) fn op_from(s: &str) -> Option<PathOp> {
    Some(match s.to_ascii_lowercase().as_str() {
        "combine" | "add" | "union" => PathOp::Combine,
        "subtract" | "minus" => PathOp::Subtract,
        "intersect" => PathOp::Intersect,
        "exclude" | "xor" => PathOp::Exclude,
        "join" => PathOp::Join,
        _ => return None,
    })
}

fn op_name(op: PathOp) -> &'static str {
    match op {
        PathOp::Combine => "combine",
        PathOp::Subtract => "subtract",
        PathOp::Intersect => "intersect",
        PathOp::Exclude => "exclude",
        PathOp::Join => "join",
    }
}

/// Parses a path from the compact JSON form (or the serde form).
pub fn parse_path(v: &Value) -> std::result::Result<Path, String> {
    let subs = v.get("subpaths").and_then(Value::as_array).ok_or("path needs `subpaths`")?;
    let mut out = Vec::with_capacity(subs.len());
    for (i, s) in subs.iter().enumerate() {
        let knots_v = s.get("knots").and_then(Value::as_array).ok_or(format!("subpath {i} needs `knots`"))?;
        let mut knots = Vec::with_capacity(knots_v.len());
        for (j, k) in knots_v.iter().enumerate() {
            let knot = if k.is_array() {
                let a = pt(k).ok_or(format!("knot {i}.{j}: expected [x, y]"))?;
                Knot::corner(a.x, a.y)
            } else {
                let a = k.get("anchor").and_then(pt).ok_or(format!("knot {i}.{j}: missing anchor"))?;
                let get = |keys: &[&str]| keys.iter().find_map(|key| k.get(*key).and_then(pt));
                Knot {
                    anchor: a,
                    in_ctrl: get(&["in", "inCtrl", "in_ctrl"]).unwrap_or(a),
                    out_ctrl: get(&["out", "outCtrl", "out_ctrl"]).unwrap_or(a),
                    smooth: k.get("smooth").and_then(Value::as_bool).unwrap_or(false),
                }
            };
            knots.push(knot);
        }
        let op = match s.get("op").and_then(Value::as_str) {
            Some(o) => op_from(o).ok_or(format!("subpath {i}: unknown op `{o}`"))?,
            None => PathOp::Combine,
        };
        out.push(Subpath { closed: s.get("closed").and_then(Value::as_bool).unwrap_or(true), knots, op });
    }
    let fill_rule = match v.get("fillRule").or_else(|| v.get("fill_rule")).and_then(Value::as_str).map(str::to_ascii_lowercase).as_deref() {
        Some("evenodd" | "even-odd" | "even_odd") => FillRule::EvenOdd,
        _ => FillRule::NonZero,
    };
    Ok(Path { subpaths: out, fill_rule, inverted: v.get("inverted").and_then(Value::as_bool).unwrap_or(false) })
}

/// The compact JSON form of a path.
pub fn path_json(p: &Path) -> Value {
    let a = |q: Point| json!([q.x, q.y]);
    json!({
        "subpaths": p.subpaths.iter().map(|s| json!({
            "closed": s.closed,
            "op": op_name(s.op),
            "knots": s.knots.iter().map(|k| json!({ "anchor": a(k.anchor), "in": a(k.in_ctrl), "out": a(k.out_ctrl), "smooth": k.smooth })).collect::<Vec<_>>(),
        })).collect::<Vec<_>>(),
        "fillRule": if p.fill_rule == FillRule::EvenOdd { "evenodd" } else { "nonzero" },
        "inverted": p.inverted,
    })
}

pub(super) fn fill_json(f: &Fill) -> Value {
    match f {
        Fill::Solid(c) => json!(hex(c)),
        Fill::Gradient { stops, angle, scale, style, reverse, opacity_stops, midpoints, offset, dither, align } => json!({ "gradient": {
            "stops": stops.iter().map(|(t, c)| json!([t, hex(c)])).collect::<Vec<_>>(),
            "angle": angle, "scale": scale * 100.0, "style": format!("{style:?}").to_ascii_lowercase(), "reverse": reverse,
            "opacityStops": opacity_stops, "midpoints": midpoints, "offset": [offset.0, offset.1], "dither": dither, "align": align } }),
        Fill::Pattern { name, scale, .. } => json!({ "pattern": name, "scale": scale * 100.0 }),
    }
}

/// The most colour stops, opacity stops or midpoints a gradient fill takes, as the gradient stop
/// edits (`gradient.fill.stop`, `gradient.presets.stop`) cap them.
const MAX_GRADIENT_STOPS: usize = 1024;

/// `"#rrggbb"` / `[r,g,b,a]` / `{"gradient":{"stops":[[t,"#hex"|[r,g,b,a]],…],"angle","scale","style","reverse",
/// "opacityStops":[[t,0..1],…],"midpoints":[0..1,…],"offset":[x,y],"dither","align"}}` / `{"pattern":name,"scale"}`.
/// `null` → `Ok(None)` (no fill).
pub(super) fn parse_fill(v: &Value) -> std::result::Result<Option<Fill>, String> {
    if v.is_null() {
        return Ok(None);
    }
    if let Some(c) = color(v) {
        return Ok(Some(Fill::Solid(c)));
    }
    if let Some(g) = v.get("gradient") {
        // Stop lists are capped as `gradient.fill.stop` caps them: more is an error, not a cut.
        let list = |key: &str, what: &str| match g.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Array(a)) if a.len() > MAX_GRADIENT_STOPS => Err(format!("too many {what} (max {MAX_GRADIENT_STOPS})")),
            Some(Value::Array(a)) => Ok(Some(a)),
            Some(_) => Err(format!("gradient `{key}` must be an array")),
        };
        let stops: Vec<(f32, Color)> = list("stops", "colour stops")?
            .map(|a| a.iter().filter_map(|s| Some((s.get(0)?.as_f64()? as f32, color(s.get(1)?)?))).collect())
            .unwrap_or_else(|| vec![(0.0, Color::BLACK), (1.0, Color::WHITE)]);
        let style = match g.get("style").and_then(Value::as_str).unwrap_or("linear") {
            "radial" => GradientStyle::Radial,
            "angle" => GradientStyle::Angle,
            "reflected" => GradientStyle::Reflected,
            "diamond" => GradientStyle::Diamond,
            _ => GradientStyle::Linear,
        };
        let mut fill = Fill::gradient(
            stops,
            f64p(g, "angle").unwrap_or(90.0) as f32,
            f64p(g, "scale").unwrap_or(100.0) as f32 / 100.0,
            style,
            g.get("reverse").and_then(Value::as_bool).unwrap_or(false),
        );
        // The rest of a gradient, so a read-modify-write (the shape swatches) loses nothing.
        if let Fill::Gradient { opacity_stops, midpoints, offset, dither, align, .. } = &mut fill {
            let unit = |v: &Value| v.as_f64().map(|n| n.clamp(0.0, 1.0) as f32);
            if let Some(a) = list("opacityStops", "opacity stops")? {
                *opacity_stops = a.iter().filter_map(|s| Some((unit(s.get(0)?)?, unit(s.get(1)?)?))).collect();
                opacity_stops.sort_by(|a, b| a.0.total_cmp(&b.0));
            }
            if let Some(a) = list("midpoints", "midpoints")? {
                *midpoints = a.iter().filter_map(unit).collect();
            }
            if let Some(o) = g.get("offset") {
                let at = |i: usize| o.get(i).and_then(Value::as_f64).map(|n| n.clamp(-1000.0, 1000.0) as f32);
                *offset = (at(0).ok_or("gradient offset must be [x, y]")?, at(1).ok_or("gradient offset must be [x, y]")?);
            }
            if let Some(d) = g.get("dither").and_then(Value::as_bool) {
                *dither = d;
            }
            if let Some(a) = g.get("align").and_then(Value::as_bool) {
                *align = a;
            }
        }
        return Ok(Some(fill));
    }
    if let Some(name) = v.get("pattern").and_then(Value::as_str) {
        return Ok(Some(Fill::Pattern {
            name: name.into(),
            scale: f64p(v, "scale").unwrap_or(100.0) as f32 / 100.0,
            id: String::new(),
            angle: 0.0,
            link: true,
            phase: (0.0, 0.0),
        }));
    }
    Err(format!("unrecognised fill {v}"))
}

pub(super) fn stroke_json(s: &ShapeStroke) -> Value {
    json!({
        "width": s.width, "fill": fill_json(&s.paint), "opacity": s.opacity * 100.0,
        "align": format!("{:?}", s.align).to_ascii_lowercase(), "cap": format!("{:?}", s.cap).to_ascii_lowercase(),
        "join": format!("{:?}", s.join).to_ascii_lowercase(), "miterLimit": s.miter_limit, "dashes": s.dashes, "dashOffset": s.dash_offset,
    })
}

/// Applies stroke keys onto `base` (`null` → no stroke).
pub fn parse_stroke(v: &Value, base: Option<ShapeStroke>, fg: Color) -> std::result::Result<Option<ShapeStroke>, String> {
    if v.is_null() || v.as_bool() == Some(false) {
        return Ok(None);
    }
    if !v.is_object() {
        return Err("stroke must be an object or null".into());
    }
    for (key, lo, hi) in [("width", 0.0, 1_000_000.0), ("opacity", 0.0, 100.0), ("miterLimit", 1.0, 500.0), ("dashOffset", -1_000_000.0, 1_000_000.0)] {
        if let Some(value) = v.get(key)
            && !value.as_f64().is_some_and(|n| n.is_finite() && (lo..=hi).contains(&n))
        {
            return Err(format!("`{key}` must be a finite number between {lo} and {hi}"));
        }
    }
    for key in ["align", "cap", "join"] {
        if v.get(key).is_some_and(|value| !value.is_string()) {
            return Err(format!("`{key}` must be a string"));
        }
    }
    let mut s = base.unwrap_or(ShapeStroke { paint: Fill::Solid(fg), ..Default::default() });
    if let Some(w) = f64p(v, "width") {
        s.width = w as f32;
    }
    if let Some(c) = v.get("color").or_else(|| v.get("fill")) {
        s.paint = parse_fill(c)?.ok_or("stroke colour cannot be null")?;
    }
    if let Some(o) = f64p(v, "opacity") {
        s.opacity = (o / 100.0) as f32;
    }
    if let Some(a) = v.get("align").and_then(Value::as_str) {
        s.align = match a {
            "inside" => StrokeAlign::Inside,
            "outside" => StrokeAlign::Outside,
            "center" | "centre" => StrokeAlign::Center,
            o => return Err(format!("unknown align `{o}`")),
        };
    }
    if let Some(c) = v.get("cap").and_then(Value::as_str) {
        s.cap = match c {
            "butt" => LineCap::Butt,
            "round" => LineCap::Round,
            "square" => LineCap::Square,
            o => return Err(format!("unknown cap `{o}`")),
        };
    }
    if let Some(j) = v.get("join").and_then(Value::as_str) {
        s.join = match j {
            "miter" => LineJoin::Miter,
            "round" => LineJoin::Round,
            "bevel" => LineJoin::Bevel,
            o => return Err(format!("unknown join `{o}`")),
        };
    }
    if let Some(m) = f64p(v, "miterLimit") {
        s.miter_limit = m as f32;
    }
    if let Some(d) = v.get("dashes") {
        s.dashes = match d {
            Value::Null => Vec::new(),
            Value::Array(a) if a.len() <= 32 => a
                .iter()
                .map(|x| {
                    x.as_f64()
                        .filter(|n| n.is_finite() && (0.0..=10000.0).contains(n))
                        .map(|x| x as f32)
                        .ok_or_else(|| "dash lengths must be finite numbers between 0 and 10000 widths".to_string())
                })
                .collect::<std::result::Result<Vec<_>, _>>()?,
            _ => return Err("`dashes` must be an array (multiples of the width)".into()),
        };
    }
    if let Some(o) = f64p(v, "dashOffset") {
        s.dash_offset = o as f32;
    }
    s.validate()?;
    Ok(Some(s))
}

pub(super) fn live_json(l: &LiveShape) -> Value {
    serde_json::to_value(l).unwrap_or(Value::Null)
}

/// Builds a live shape from `kind` + geometry params.
pub(super) fn live_from_params(cmd: &str, kind: &str, p: &Value, prev: Option<&LiveShape>) -> Result<LiveShape> {
    let prev_rect = match prev {
        Some(LiveShape::Rect { rect, .. } | LiveShape::Ellipse { rect } | LiveShape::Polygon { rect, .. }) => Some(*rect),
        _ => None,
    };
    let rect = || nums::<4>(p, "rect").or(prev_rect).ok_or_else(|| bad(cmd, "`rect`: [x, y, w, h] is required"));
    Ok(match kind {
        "rect" | "rectangle" => {
            let r = match prev {
                Some(LiveShape::Rect { radii, .. }) => *radii,
                _ => [0.0; 4],
            };
            LiveShape::Rect { rect: rect()?, radii: radii(p).unwrap_or(r) }
        }
        "roundedRect" | "roundRect" => {
            let r = match prev {
                Some(LiveShape::Rect { radii, .. }) if radii.iter().any(|v| *v > 0.0) => *radii,
                _ => [10.0; 4],
            };
            LiveShape::Rect { rect: rect()?, radii: radii(p).unwrap_or(r) }
        }
        "ellipse" => LiveShape::Ellipse { rect: rect()? },
        "polygon" | "star" => {
            let (ps, pr, pc) = match prev {
                Some(LiveShape::Polygon { sides, star_ratio, corner_radius, .. }) => (*sides, *star_ratio, *corner_radius),
                _ => (5, if kind == "star" { 0.5 } else { 1.0 }, 0.0),
            };
            let corner_radius = f64p(p, "cornerRadius").map_or(pc, |v| if v.is_finite() { v.clamp(0.0, 1.0e6) } else { pc });
            let sides = f64p(p, "sides").map_or(ps, |v| v.round().clamp(3.0, 100.0) as u32);
            let ratio = f64p(p, "starRatio").map_or(if kind == "star" && pr >= 1.0 { 0.5 } else { pr }, |v| v.clamp(0.01, 1.0));
            LiveShape::Polygon {
                rect: rect()?,
                sides,
                star_ratio: if kind == "polygon" && f64p(p, "starRatio").is_none() { 1.0 } else { ratio },
                corner_radius,
            }
        }
        "line" => {
            let (pf, pto, pw) = match prev {
                Some(LiveShape::Line { from, to, weight }) => (Some(*from), Some(*to), *weight),
                _ => (None, None, 1.0),
            };
            LiveShape::Line {
                from: nums::<2>(p, "from").or(pf).ok_or_else(|| bad(cmd, "`from`: [x, y] is required"))?,
                to: nums::<2>(p, "to").or(pto).ok_or_else(|| bad(cmd, "`to`: [x, y] is required"))?,
                weight: f64p(p, "weight").unwrap_or(pw).max(0.0),
            }
        }
        o => return Err(bad(cmd, format!("unknown kind `{o}`"))),
    })
}
