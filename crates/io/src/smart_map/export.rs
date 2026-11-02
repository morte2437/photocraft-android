use super::import::{
    CHANNELS, CUSTOM_CELL, CUSTOM_OFFSET, CUSTOM_SCALE, DIFFUSE_MODES, EXTRUDE_DEPTH, EXTRUDE_DEPTHS, EXTRUDE_SIZE, EXTRUDE_TYPES, GALLERY_COMMAND, Known,
    LENSES, MEZZOTINT_TYPES, NO_OPTIONS, OFFSET_FILL, POLAR_MODE, RADIAL_METHOD, RADIAL_QUALITY, RIPPLE_SIZE, SMART_BLUR_MODE, SMART_BLUR_QUALITY,
    SMART_SHARPEN, SPHERIZE_MODE, TILE_COUNT, TILE_FILLS, TILE_OFFSET, TRACE_EDGES, UNDEFINED_AREAS, WAVE_TYPE, WIND_DIRECTIONS, WIND_METHODS, ZIGZAG_STYLE,
    colour_rgbc, enum_param, from_hex, gallery_item, known_by_command, pf_range, pi_range, seed_param,
};
use super::placed::enumv;
use super::{
    Class, Descriptor, FilterStack, Id, J, ReferenceItem, SmartFilter, UNSUPPORTED_FILTER, UnicodeString, Value, camera_raw, displace, hue_sat,
    perspective_warp,
};

// ---------- writing ----------

pub(crate) fn unit(u: &[u8; 4], v: f64) -> Value {
    Value::UnitFloat { unit: *u, value: v }
}

pub(crate) fn rgbc(v: f64) -> Value {
    Value::Descriptor(Descriptor::new("RGBC").with("Rd  ", Value::Double(v)).with("Grn ", Value::Double(v)).with("Bl  ", Value::Double(v)))
}

pub(crate) fn blend_options(f: &SmartFilter) -> Value {
    let opacity = if f.opacity.is_finite() { f64::from(f.opacity.clamp(0.0, 1.0)) * 100.0 } else { 100.0 };
    Value::Descriptor(Descriptor::new("blendOptions").with("Opct", unit(b"#Prc", opacity)).with("Md  ", crate::effects_map::blend_long_value(f.blend)))
}

/// Parameter keys that are bookkeeping, not filter settings.
pub(crate) fn is_meta(k: &str) -> bool {
    k == "layer" || k.starts_with("__")
}

fn pf(p: &J, key: &str, default: f64) -> f64 {
    p.get(key).and_then(J::as_f64).filter(|v| v.is_finite()).unwrap_or(default)
}

pub(super) fn pi(p: &J, key: &str, default: f64) -> Value {
    Value::Integer(pf(p, key, default).round().clamp(f64::from(i32::MIN), f64::from(i32::MAX)) as i32)
}

/// A whole-number setting as a long, clamped to `range` like the engine clamps it; a fraction
/// has no PSD equivalent (Photoshop stores these as integers).
pub(super) fn whole(v: f64, (lo, hi): (i32, i32), what: &str) -> Result<Value, String> {
    if v.fract() != 0.0 {
        return Err(format!("{what} ({v}) is not a whole number"));
    }
    Ok(Value::Integer(v.clamp(f64::from(lo), f64::from(hi)) as i32))
}

fn channel_ref(code: &[u8; 4]) -> Value {
    Value::Reference(vec![ReferenceItem::Enumerated {
        class: Class { name: UnicodeString::new_nul(""), class_id: Id::new("Chnl") },
        type_id: Id::new("Chnl"),
        value: Id::Code(*code),
    }])
}

/// The `Adjs` entries of a Levels or Curves filter: the composite first, then red, green, blue.
fn adjustments(p: &J, curves: bool) -> Result<Value, String> {
    let obj = p.as_object().ok_or("parameters are not an object")?;
    let allowed: &[&str] =
        if curves { &["points", "red", "green", "blue", "pencil"] } else { &["inBlack", "inWhite", "gamma", "outBlack", "outWhite", "red", "green", "blue"] };
    if let Some(k) = obj.keys().find(|k| !is_meta(k) && !allowed.contains(&k.as_str())) {
        return Err(format!("setting `{k}` has no PSD equivalent"));
    }
    let mut list = Vec::new();
    for (code, key) in CHANNELS {
        let o = if key.is_empty() { Some(p) } else { p.get(key) };
        let Some(o) = o else { continue };
        let mut a = Descriptor::new(if curves { "CrvA" } else { "LvlA" }).with("Chnl", channel_ref(code));
        if curves {
            let src = if key.is_empty() { p.get("points") } else { Some(o) };
            let Some(pts) = src.and_then(J::as_array) else { continue };
            if p.get("pencil").and_then(J::as_bool) == Some(true) {
                let map: Option<Vec<Value>> =
                    pts.iter().map(|o| o.as_f64().filter(|o| o.is_finite()).map(|o| Value::Integer(o.round().clamp(0.0, 255.0) as i32))).collect();
                a = a.with("Mpng", Value::List(map.filter(|m| m.len() == 256).ok_or("a pencil-drawn curve is not 256 output levels")?));
                list.push(Value::Descriptor(a));
                continue;
            }
            let pts: Option<Vec<Value>> = pts
                .iter()
                .map(|q| {
                    let x = q.get(0)?.as_f64().filter(|v| v.is_finite())?;
                    let y = q.get(1)?.as_f64().filter(|v| v.is_finite())?;
                    Some(Value::Descriptor(Descriptor::new("Pnt ").with("Hrzn", Value::Double(x)).with("Vrtc", Value::Double(y))))
                })
                .collect();
            a = a.with("Crv ", Value::List(pts.ok_or("a curve point is not [input, output]")?));
        } else {
            if key.is_empty() && !["inBlack", "inWhite", "gamma", "outBlack", "outWhite"].iter().any(|k| o.get(k).is_some()) {
                continue;
            }
            a = a.with("Inpt", Value::List(vec![pi(o, "inBlack", 0.0), pi(o, "inWhite", 255.0)])).with("Gmm ", Value::Double(pf(o, "gamma", 1.0)));
            if o.get("outBlack").is_some() || o.get("outWhite").is_some() {
                a = a.with("Otpt", Value::List(vec![pi(o, "outBlack", 0.0), pi(o, "outWhite", 255.0)]));
            }
        }
        list.push(Value::Descriptor(a));
    }
    Ok(Value::List(list))
}

/// Crystallize's and Pointillize's cell size range (`ClSz`, a `long`).
pub(super) const CELL_SIZE: (i32, i32) = (3, 300);

/// Our `key` (a string) as Photoshop's enum value through `table` (Photoshop's value, ours).
fn enum_out(p: &J, key: &str, default: &str, type_id: &str, table: &[(&str, &str)]) -> Result<Value, String> {
    let ours = match p.get(key) {
        None => default,
        Some(v) => v.as_str().ok_or_else(|| format!("`{key}` is not a string"))?,
    };
    let (psd, _) = table.iter().find(|(_, o)| *o == ours).ok_or_else(|| format!("{key} `{ours}` has no PSD equivalent"))?;
    Ok(enumv(type_id, psd))
}

/// `key` rounded to a `long` in `lo..=hi` (clamped, and a non-number is the default, as the
/// engine reads Extrude, Tiles, Trace Contour, Crystallize and Pointillize).
fn pi_in(p: &J, key: &str, default: f64, lo: i32, hi: i32) -> Value {
    Value::Integer(pf(p, key, default).round().clamp(f64::from(lo), f64::from(hi)) as i32)
}

/// Param `key` as a `bool` (false when absent or not a bool, as the engine reads it).
fn bool_param(p: &J, key: &str) -> Value {
    Value::Boolean(p.get(key).and_then(J::as_bool).unwrap_or(false))
}

/// The `Fltr` descriptor of a modelled filter (`None` = written without options).
fn fltr_for(k: &Known, p: &J) -> Result<Option<Descriptor>, String> {
    let d = Descriptor::new(k.class);
    Ok(Some(match k.class {
        "GsnB" | "boxblur" | "Mdn " | "HghP" | "Mxm " | "Mnm " => {
            if p.get("preserve").and_then(J::as_str).is_some_and(|s| s == "roundness") {
                return Err("“preserve roundness” has no PSD equivalent".into());
            }
            d.with("Rds ", unit(b"#Pxl", pf(p, "radius", 1.0)))
        }
        "MtnB" => d.with("Angl", pi(p, "angle", 0.0)).with("Dstn", unit(b"#Pxl", pf(p, "distance", 10.0))),
        "UnsM" => d.with("Amnt", unit(b"#Prc", pf(p, "amount", 50.0))).with("Rds ", unit(b"#Pxl", pf(p, "radius", 1.0))).with("Thsh", pi(p, "threshold", 0.0)),
        "AdNs" => {
            let gauss = p.get("distribution").and_then(J::as_str) == Some("gaussian");
            d.with("Dstr", Value::Enumerated { type_id: Id::new("Dstr"), value: Id::new(if gauss { "Gsn " } else { "Unfr" }) })
                .with("Nose", unit(b"#Prc", pf(p, "amount", 12.5)))
                .with("Mnch", Value::Boolean(p.get("monochromatic").and_then(J::as_bool).unwrap_or(false)))
                .with("FlRs", pi(p, "seed", 0.0))
        }
        "surfaceBlur" => d.with("Rds ", unit(b"#Pxl", pf_range(p, "radius", 5.0, 1.0, 100.0)?)).with("Thsh", pi_range(p, "threshold", 15.0, 2, 255)?),
        "SmrB" => d
            .with("Rds ", Value::Double(pf_range(p, "radius", 3.0, 0.1, 100.0)?))
            .with("Thsh", Value::Double(pf_range(p, "threshold", 25.0, 0.1, 100.0)?))
            .with("SmBQ", enum_param(p, "quality", "high", "SmBQ", &SMART_BLUR_QUALITY)?)
            .with("SmBM", enum_param(p, "mode", "normal", "SmBM", &SMART_BLUR_MODE)?),
        "RdlB" => {
            // Photoshop wrote no centre key for a centred blur; an off-centre one is unmeasured.
            for k in ["centerX", "centerY"] {
                if (pf_range(p, k, 0.5, 0.0, 1.0)? - 0.5).abs() > 1e-9 {
                    return Err("an off-centre Radial Blur centre is not written to PSD".into());
                }
            }
            d.with("Amnt", pi_range(p, "amount", 10.0, 1, 100)?)
                .with("BlrM", enum_param(p, "method", "spin", "BlrM", &RADIAL_METHOD)?)
                .with("BlrQ", enum_param(p, "quality", "good", "BlrQ", &RADIAL_QUALITY)?)
        }
        "Msc " => d.with("ClSz", unit(b"#Pxl", pf(p, "cellSize", 10.0))),
        // Photoshop's Mezzotint has no seed (it is random on every render), so `seed` is not saved.
        "Mztn" => {
            let t = p.get("type").and_then(J::as_str).unwrap_or("fineDots");
            let code = MEZZOTINT_TYPES.iter().find(|(_, k)| *k == t).map_or(b"FnDt", |(c, _)| *c);
            d.with("MztT", Value::Enumerated { type_id: Id::new("MztT"), value: Id::Code(*code) })
        }
        "ClrH" => {
            // Longs within the dialog's ranges (what the engine accepts: radius 4..127, angles ±360).
            let long = |key: &str, default: f64, lo: f64, hi: f64| Value::Integer(pf(p, key, default).round().clamp(lo, hi) as i32);
            d.with("Rds ", long("maxRadius", 8.0, 4.0, 127.0))
                .with("Ang1", long("channel1", 108.0, -360.0, 360.0))
                .with("Ang2", long("channel2", 162.0, -360.0, 360.0))
                .with("Ang3", long("channel3", 90.0, -360.0, 360.0))
                .with("Ang4", long("channel4", 45.0, -360.0, 360.0))
        }
        "Embs" => d.with("Angl", pi(p, "angle", 135.0)).with("Hght", pi(p, "height", 3.0)).with("Amnt", pi(p, "amount", 100.0)),
        SMART_SHARPEN => {
            // The engine's defaults and ranges (it clamps the same way when rendering).
            d.with("useLegacy", Value::Boolean(false))
                .with("Amnt", unit(b"#Prc", pf(p, "amount", 100.0).clamp(1.0, 500.0)))
                .with("Rds ", unit(b"#Pxl", pf(p, "radius", 1.0).clamp(0.1, 64.0)))
                .with("noiseReduction", unit(b"#Prc", pf(p, "reduceNoise", 10.0).clamp(0.0, 100.0)))
                .with("blur", enumv("blurType", "GsnB"))
        }
        // Photoshop stores whole pixels and levels.
        "DstS" => {
            let whole = |key: &str, default: f64, lo: f64, hi: f64| Value::Integer(pf(p, key, default).round().clamp(lo, hi) as i32);
            d.with("Rds ", whole("radius", 1.0, 1.0, 500.0)).with("Thsh", whole("threshold", 0.0, 0.0, 255.0))
        }
        // Photoshop stores no random seed for these: `seed` is ours only and is not written.
        "Dfs " => d.with("Md  ", enum_out(p, "mode", "normal", "DfsM", &DIFFUSE_MODES)?),
        "TrcC" => d.with("Lvl ", pi_in(p, "level", 128.0, 0, 255)).with("Edg ", enum_out(p, "edge", "lower", "CntE", &TRACE_EDGES)?),
        "Crst" => d.with("ClSz", pi_in(p, "cellSize", 10.0, CELL_SIZE.0, CELL_SIZE.1)),
        "Pntl" => d.with("ClSz", pi_in(p, "cellSize", 5.0, CELL_SIZE.0, CELL_SIZE.1)),
        // Photoshop stores no random seed for these: our `seed` is not written.
        "Wnd " => {
            d.with("WndM", enum_out(p, "method", "wind", "WndM", &WIND_METHODS)?).with("Drct", enum_out(p, "direction", "fromRight", "Drct", &WIND_DIRECTIONS)?)
        }
        "Extr" => d
            .with("ExtT", enum_out(p, "type", "blocks", "ExtT", &EXTRUDE_TYPES)?)
            .with("ExtS", pi_in(p, "size", 30.0, EXTRUDE_SIZE.0, EXTRUDE_SIZE.1))
            .with("ExtD", pi_in(p, "depth", 30.0, EXTRUDE_DEPTH.0, EXTRUDE_DEPTH.1))
            .with("ExtR", enum_out(p, "depthMode", "random", "ExtR", &EXTRUDE_DEPTHS)?)
            .with("ExtF", bool_param(p, "solidFrontFaces"))
            .with("ExtM", bool_param(p, "maskIncompleteBlocks")),
        "Tls " => d
            .with("TlNm", pi_in(p, "count", 10.0, TILE_COUNT.0, TILE_COUNT.1))
            .with("TlOf", pi_in(p, "maxOffset", 10.0, TILE_OFFSET.0, TILE_OFFSET.1))
            .with("FlCl", enum_out(p, "fill", "background", "FlCl", &TILE_FILLS)?),
        "Ofst" => {
            // As the engine reads it: anything but repeat / transparent wraps.
            let ours = p.get("undefinedAreas").and_then(J::as_str).unwrap_or("wrap");
            let fill = OFFSET_FILL.iter().find(|(_, o)| *o == ours).map_or("Wrp ", |(psd, _)| *psd);
            d.with("Hrzn", pi(p, "horizontal", 0.0)).with("Vrtc", pi(p, "vertical", 0.0)).with("Fl  ", enumv("FlMd", fill))
        }
        "Cstm" => {
            // The engine's identity kernel when none is given; missing cells are 0.
            let kernel = p.get("kernel").and_then(J::as_array);
            let cell = |i: usize| match kernel {
                Some(a) => a.get(i).and_then(J::as_f64).filter(|v| v.is_finite()).unwrap_or(0.0),
                None => f64::from(u8::from(i == 12)),
            };
            let m = (0..25).map(|i| whole(cell(i), CUSTOM_CELL, "a kernel value")).collect::<Result<Vec<_>, _>>()?;
            d.with("Scl ", whole(pf(p, "scale", 1.0), CUSTOM_SCALE, "Scale")?)
                .with("Ofst", whole(pf(p, "offset", 0.0), CUSTOM_OFFSET, "Offset")?)
                .with("Mtrx", Value::List(m))
        }
        "LnsF" => {
            let lens = p.get("lens").and_then(J::as_str).unwrap_or("zoom");
            let code = LENSES.iter().find(|(_, l)| *l == lens).ok_or_else(|| format!("lens `{lens}` has no PSD equivalent"))?.0;
            let mut centre = Descriptor::new("Pnt ").with("Hrzn", Value::Double(pf(p, "centerX", 0.5))).with("Vrtc", Value::Double(pf(p, "centerY", 0.5)));
            centre.name = UnicodeString::new_nul("");
            d.with("Brgh", pi(p, "brightness", 100.0))
                .with("FlrC", Value::Descriptor(centre))
                .with("Lns ", Value::Enumerated { type_id: Id::new("Lns "), value: Id::Code(*code) })
        }
        "Fbrs" => d.with("Vrnc", pi(p, "variance", 16.0)).with("Strg", pi(p, "strength", 4.0)).with("RndS", seed_param(p)),
        "Clds" | "DfrC" => d.with("FlRs", seed_param(p)),
        "Lvls" => d.with("Adjs", adjustments(p, false)?),
        "Crvs" => d.with("Adjs", adjustments(p, true)?),
        "adaptCorrect" => {
            if p.as_object().is_some_and(|o| o.keys().any(|k| !is_meta(k))) {
                return Err("only default Shadows/Highlights settings are written to PSD".into());
            }
            return Ok(None);
        }
        // No settings to write; any parameter keys an old record carries have no effect on the
        // filter either, so nothing is lost.
        c if NO_OPTIONS.contains(&c) => return Ok(None),
        // Keys and order as Photoshop writes them.
        "Pnch" => d.with("Amnt", pi(p, "amount", 50.0)),
        "Twrl" => d.with("Angl", pi(p, "angle", 50.0)),
        "Rple" => d.with("Amnt", pi(p, "amount", 100.0)).with("RplS", enum_param(p, "size", "medium", "RplS", &RIPPLE_SIZE)?),
        "Sphr" => d.with("Amnt", pi(p, "amount", 100.0)).with("SphM", enum_param(p, "mode", "normal", "SphM", &SPHERIZE_MODE)?),
        "Plr " => d.with("Cnvr", enum_param(p, "mode", "rectangularToPolar", "Cnvr", &POLAR_MODE)?),
        "ZgZg" => {
            d.with("Amnt", pi(p, "amount", 10.0)).with("NmbR", pi(p, "ridges", 5.0)).with("ZZTy", enum_param(p, "style", "pondRipples", "ZZTy", &ZIGZAG_STYLE)?)
        }
        "Wave" => {
            // The seed the renderer draws with (engine `filters::seed`): a u32, else a legacy signed
            // or fractional seed, rounded and saturated, as its bit pattern.
            let seed = match p.get("seed").and_then(J::as_u64).and_then(|s| u32::try_from(s).ok()) {
                Some(s) => Value::Integer(s as i32),
                None => pi(p, "seed", 0.0),
            };
            // The renderer draws at most MAX_WAVE_GENERATORS: write what PhotoCraft rendered, so
            // Photoshop doesn't draw more generators than the user saw.
            let generators = pf(p, "generators", 5.0).round().clamp(1.0, f64::from(photocraft_algo::MAX_WAVE_GENERATORS)) as i32;
            d.with("Wvtp", enum_param(p, "type", "sine", "Wvtp", &WAVE_TYPE)?)
                .with("NmbG", Value::Integer(generators))
                .with("WLMn", pi(p, "wavelengthMin", 10.0))
                .with("WLMx", pi(p, "wavelengthMax", 120.0))
                .with("AmMn", pi(p, "amplitudeMin", 5.0))
                .with("AmMx", pi(p, "amplitudeMax", 35.0))
                .with("SclH", Value::Integer(100))
                .with("SclV", Value::Integer(100))
                .with("UndA", enum_param(p, "undefinedAreas", "wrap", "UndA", &UNDEFINED_AREAS)?)
                .with("RndS", seed)
        }
        "Invr" => return Ok(None),
        _ => return Err("not a recognized filter".into()),
    }))
}

/// The `filterFXList` item for one smart filter, or why it can't be written.
pub fn item_for_filter(f: &SmartFilter) -> Result<Descriptor, String> {
    if f.command == camera_raw::COMMAND {
        let mut d = camera_raw::export_item(&f.params)?;
        set(&mut d, "blendOptions", blend_options(f));
        set(&mut d, "enab", Value::Boolean(f.visible));
        return Ok(d);
    }
    if f.command == displace::COMMAND {
        let mut d = displace::export_item(&f.params)?;
        set(&mut d, "blendOptions", blend_options(f));
        set(&mut d, "enab", Value::Boolean(f.visible));
        return Ok(d);
    }
    if f.command == GALLERY_COMMAND {
        return gallery_item(f);
    }
    if f.command == hue_sat::COMMAND {
        return hue_sat::export_item(f);
    }
    if perspective_warp::handles(&f.command) {
        return perspective_warp::export_item(f);
    }
    if f.command == UNSUPPORTED_FILTER {
        let raw = f.params.get("psd").and_then(J::as_str).and_then(from_hex).ok_or("its filter data is missing")?;
        let mut d = Descriptor::from_bytes(&raw).map_err(|e| format!("its filter data is unreadable ({e})"))?;
        set(&mut d, "blendOptions", blend_options(f));
        set(&mut d, "enab", Value::Boolean(f.visible));
        return Ok(d);
    }
    let k = known_by_command(&f.command).ok_or("this filter has no PSD equivalent")?;
    let fltr = fltr_for(k, &f.params)?;
    // Pointillize's background, Tiles' colours and the render filters' colours are the entry's;
    // the others store Photoshop's defaults.
    let (fg, bg) = match k.class {
        "Pntl" => (rgbc(0.0), colour_rgbc(f.params.get("background"), 255.0)?),
        "Tls " | "Fbrs" | "Clds" | "DfrC" => (colour_rgbc(f.params.get("foreground"), 0.0)?, colour_rgbc(f.params.get("background"), 255.0)?),
        _ => (rgbc(0.0), rgbc(255.0)),
    };
    // The render filters are written with the descriptor names Photoshop gives them: the filter's
    // own name on `Fltr` and empty ones on the colours.
    let render = matches!(k.class, "Fbrs" | "Clds" | "DfrC" | "LnsF");
    let (fltr, fg, bg) = if render { (fltr.map(|f| named(f, k.name.trim_end_matches("..."))), named_value(fg), named_value(bg)) } else { (fltr, fg, bg) };
    let mut d = Descriptor::new("filterFX")
        .with("Nm  ", Value::Text(UnicodeString::new_nul(k.name)))
        .with("blendOptions", blend_options(f))
        .with("enab", Value::Boolean(f.visible))
        .with("hasoptions", Value::Boolean(fltr.is_some()))
        .with("FrgC", fg)
        .with("BckC", bg);
    if let Some(x) = fltr {
        d = d.with("Fltr", Value::Descriptor(x));
    }
    Ok(d.with("filterID", Value::Integer(k.filter_id)))
}

fn named(mut d: Descriptor, name: &str) -> Descriptor {
    d.name = UnicodeString::new_nul(name);
    d
}

fn named_value(v: Value) -> Value {
    match v {
        Value::Descriptor(d) => Value::Descriptor(named(d, "")),
        other => other,
    }
}

/// Builds a `filterFX` object; filters that can't be written are left out, with a message each.
pub fn filter_fx(stack: &FilterStack, layer: &str, warnings: &mut Vec<String>) -> Descriptor {
    let mut list = Vec::new();
    for f in &stack.filters {
        match item_for_filter(f) {
            Ok(d) => list.push(Value::Descriptor(d)),
            Err(e) => warnings.push(format!("layer \"{layer}\": smart filter {} was not written to PSD: {e}", f.command)),
        }
    }
    Descriptor::new("filterFXStyle")
        .with("enab", Value::Boolean(stack.enabled))
        .with("validAtPosition", Value::Boolean(true))
        .with("filterMaskEnable", Value::Boolean(stack.mask_enabled))
        .with("filterMaskLinked", Value::Boolean(stack.mask_linked))
        .with("filterMaskExtendWithWhite", Value::Boolean(true))
        .with("filterFXList", Value::List(list))
}

/// Replaces item `key` in place, or appends it.
pub(crate) fn set(d: &mut Descriptor, key: &str, v: Value) {
    match d.items.iter_mut().find(|(k, _)| k.is(key)) {
        Some(e) => e.1 = v,
        None => d.items.push((Id::new(key), v)),
    }
}

/// Replaces item `key` in place, or inserts it before item `before` (appends it without one).
pub(super) fn set_before(d: &mut Descriptor, key: &str, v: Value, before: &str) {
    if let Some(e) = d.items.iter_mut().find(|(k, _)| k.is(key)) {
        e.1 = v;
        return;
    }
    let at = d.items.iter().position(|(k, _)| k.is(before)).unwrap_or(d.items.len());
    d.items.insert(at, (Id::new(key), v));
}
