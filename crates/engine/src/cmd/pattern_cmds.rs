//! Patterns: Edit › Define Pattern, the pattern library (`pattern.*`), Layer › New Fill Layer ›
//! Pattern, Edit › Fill with a pattern and brush textures from library patterns.
//!
//! Two places hold patterns, as in Photoshop:
//! - the **library** ([`PatternLibrary`], on the [`Session`]): built-in procedural patterns plus
//!   defined and imported ones. Library edits are app state, not document history.
//! - the **document** ([`photocraft_doc::Document::patterns`]): the patterns its layers use
//!   (saved in PSD `Patt` blocks and `.pcraft`). Commands that apply a pattern copy it into the
//!   document in the same history step, so a document always renders on its own.
//!
//! Patterns are referenced by `"pattern": id | name` (document first, then library).

use photocraft_color::{ColorMode, PixelFormat, SampleType};
use photocraft_doc::{Color, Document, Fill, Layer, LayerContent, Pattern, Rect};
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

/// The app-level pattern library (Window › Patterns).
#[derive(Clone, Debug)]
pub struct PatternLibrary {
    pub items: Vec<Pattern>,
    /// Bumped when a pattern's pixels are replaced under the same id (an imported `.pat` whose
    /// id is already in the library), which its id, name and size don't show.
    pub pixels_rev: u64,
    /// Saved patterns held as a small preview until used, by id: their full size. The preset
    /// store reads the pixels ([`full`]); everything that paints goes through [`resolve`].
    pub stored: std::collections::HashMap<String, (u32, u32)>,
}

impl Default for PatternLibrary {
    fn default() -> Self {
        PatternLibrary { items: builtin(), pixels_rev: 0, stored: Default::default() }
    }
}

impl PatternLibrary {
    /// Add `pat`, or replace the pattern with its id (marking the pixels changed).
    pub fn upsert(&mut self, pat: Pattern) {
        self.stored.remove(&pat.id);
        match self.items.iter_mut().find(|q| q.id == pat.id) {
            Some(q) => {
                *q = pat;
                self.pixels_rev += 1;
            }
            None => self.items.push(pat),
        }
    }
}

/// Whether `p` is a built-in pattern as shipped (same id, name, size and pixels): those are never
/// saved. A built-in replaced by an import, or renamed, is the user's and is.
pub(crate) fn is_pristine_builtin(p: &Pattern, builtins: &[Pattern]) -> bool {
    builtins.iter().any(|b| b.id == p.id && b.name == p.name && (b.width, b.height) == (p.width, p.height) && b.surface == p.surface)
}

fn hash01(x: u32, y: u32, seed: u32) -> f32 {
    let mut h = x.wrapping_mul(0x8da6_b343) ^ y.wrapping_mul(0xd816_3841) ^ seed.wrapping_mul(0xcb1a_b31f);
    h ^= h >> 13;
    h = h.wrapping_mul(0x5bd1_e995);
    h ^= h >> 15;
    (h & 0xffff) as f32 / 65535.0
}

fn procedural(name: &str, w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 3]) -> Pattern {
    let fmt = PixelFormat::new(ColorMode::Rgb, SampleType::U8, false);
    let mut s = Surface::new(fmt);
    let mut px = Vec::with_capacity((w * h * 3) as usize);
    for y in 0..h {
        for x in 0..w {
            px.extend_from_slice(&f(x, y));
        }
    }
    // Quantise through the 8-bit surface so ids are stable.
    s.write_region(Rect::new(0, 0, w as i32, h as i32), &px);
    Pattern::new(name, s, w, h)
}

/// Built-in patterns: generated procedurally here (our own, CC0), so the library is never empty.
pub fn builtin() -> Vec<Pattern> {
    let g = |v: f32| [v, v, v];
    vec![
        procedural("Checkerboard", 16, 16, |x, y| g(if (x / 8 + y / 8) % 2 == 0 { 0.8 } else { 1.0 })),
        procedural("Diagonal Lines", 8, 8, |x, y| g(if (x + y) % 8 < 2 { 0.2 } else { 1.0 })),
        procedural("Dots", 12, 12, |x, y| {
            let (dx, dy) = (x as f32 + 0.5 - 6.0, y as f32 + 0.5 - 6.0);
            g(((dx * dx + dy * dy).sqrt() - 3.0).clamp(0.0, 1.0))
        }),
        procedural("Grid", 16, 16, |x, y| g(if x == 0 || y == 0 { 0.55 } else { 1.0 })),
        procedural("Bricks", 32, 16, |x, y| {
            let row = y / 8;
            let xo = (x + if row % 2 == 1 { 8 } else { 0 }) % 16;
            let mortar = y % 8 == 7 || xo == 15;
            let n = hash01(x, y, 7) * 0.08;
            if mortar { [0.78 + n, 0.76 + n, 0.72 + n] } else { [0.62 + n, 0.27 + n * 0.5, 0.2] }
        }),
        procedural("Weave", 16, 16, |x, y| {
            let over = ((x / 4) + (y / 4)) % 2 == 0;
            let t = if over { (y % 4) as f32 / 3.0 } else { (x % 4) as f32 / 3.0 };
            let v = 0.55 + 0.35 * (1.0 - (2.0 * t - 1.0).abs());
            [v, v * 0.92, v * 0.8]
        }),
        procedural("Paper Noise", 64, 64, |x, y| {
            // Two octaves of smoothed value noise (wraps at 64 px).
            let oct = |cell: u32, seed: u32| {
                let (cx, cy) = (x / cell, y / cell);
                let (fx, fy) = ((x % cell) as f32 / cell as f32, (y % cell) as f32 / cell as f32);
                let n = 64 / cell;
                let v = |i: u32, j: u32| hash01(i % n, j % n, seed);
                let s = |t: f32| t * t * (3.0 - 2.0 * t);
                let (sx, sy) = (s(fx), s(fy));
                let a = v(cx, cy) + (v(cx + 1, cy) - v(cx, cy)) * sx;
                let b = v(cx, cy + 1) + (v(cx + 1, cy + 1) - v(cx, cy + 1)) * sx;
                a + (b - a) * sy
            };
            g(0.78 + 0.14 * oct(8, 1) + 0.08 * oct(2, 2))
        }),
    ]
}

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

fn always(_: &Session) -> std::result::Result<(), String> {
    Ok(())
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_pattern(s: &Session) -> std::result::Result<(), String> {
    if s.patterns.items.is_empty() && s.active().is_none_or(|d| d.doc.patterns.is_empty()) { Err("no patterns".into()) } else { Ok(()) }
}

/// `p` with its full pixels: a saved pattern held as a preview is read from the preset store.
pub fn full(s: &Session, p: &Pattern) -> std::result::Result<Pattern, String> {
    if !s.patterns.stored.contains_key(&p.id) {
        return Ok(p.clone());
    }
    let st = s.preset_store.as_ref().ok_or_else(|| format!("pattern \"{}\" is not loaded: no preset store", p.display_name()))?;
    let mut f = st.load_pattern(&p.id).map_err(|e| format!("pattern \"{}\" could not be loaded: {e}", p.display_name()))?;
    f.name = p.name.clone();
    Ok(f)
}

/// The library pattern `key` names (the panel's selection or the first for an empty key).
fn library_pattern<'a>(s: &'a Session, key: &str) -> Option<&'a Pattern> {
    if key.is_empty() {
        return crate::presets::patterns::current(s).or(s.patterns.items.first());
    }
    photocraft_doc::pattern::find(&s.patterns.items, key, key)
}

/// The id of the pattern [`resolve`] finds, without reading a saved pattern's pixels: for checks
/// and lookups that don't paint.
pub fn resolve_id(s: &Session, key: &str) -> Option<String> {
    if !key.is_empty()
        && let Some(d) = s.active()
        && let Some(p) = photocraft_doc::pattern::find(&d.doc.patterns, key, key)
    {
        return Some(p.id.clone());
    }
    library_pattern(s, key).map(|p| p.id.clone())
}

/// Finds a pattern by id or name: the active document's first, then the library. An empty key
/// picks the first library pattern (Photoshop's default). `None` also when a saved pattern's
/// pixels can't be read (see [`resolve_param`] for the reason).
pub fn resolve(s: &Session, key: &str) -> Option<Pattern> {
    if !key.is_empty()
        && let Some(d) = s.active()
        && let Some(p) = photocraft_doc::pattern::find(&d.doc.patterns, key, key)
    {
        return Some(p.clone());
    }
    full(s, library_pattern(s, key)?).ok()
}

pub(crate) fn resolve_param(s: &Session, cmd: &str, p: &Value) -> Result<Pattern> {
    let key = p.get("pattern").and_then(Value::as_str).unwrap_or("");
    if let Some(pat) = resolve(s, key) {
        return paintable(cmd, pat);
    }
    match library_pattern(s, key).map(|lp| full(s, lp)) {
        Some(Err(e)) => Err(bad(cmd, e)),
        _ => Err(bad(cmd, format!("no pattern \"{key}\" (see pattern.list)"))),
    }
}

/// `pat`, or the reason it would paint nothing: a pattern too large for the compositor renders as
/// transparent, so a command that would use one says so up front.
fn paintable(cmd: &str, pat: Pattern) -> Result<Pattern> {
    match pat.paint_problem() {
        Some(why) => Err(bad(cmd, format!("pattern \"{}\" would paint nothing: {why}", pat.display_name()))),
        None => Ok(pat),
    }
}

/// Copies `pat` into the document's patterns unless it is already there (by id).
pub fn ensure_in_doc(doc: &mut Document, pat: &Pattern) {
    if !doc.patterns.iter().any(|p| p.id == pat.id) {
        doc.patterns.push(pat.clone());
    }
}

/// Resolves the pattern of a Pattern Overlay / pattern stroke built from params (its `name`
/// holds the requested key) into a real pattern: returns the effect with name and id filled in,
/// plus the pattern to copy into the document.
pub fn resolve_effect(s: &Session, fx: photocraft_doc::Effect) -> Result<(photocraft_doc::Effect, Option<Pattern>)> {
    use photocraft_doc::Effect;
    match fx {
        Effect::PatternOverlay { common, name, id, scale, angle, link, phase } => {
            let key = if id.is_empty() { name } else { id };
            let pat = resolve(s, &key).ok_or_else(|| bad("layer.layerStyle.patternOverlay", format!("no pattern \"{key}\" (see pattern.list)")))?;
            let pat = paintable("layer.layerStyle.patternOverlay", pat)?;
            Ok((Effect::PatternOverlay { common, name: pat.name.clone(), id: pat.id.clone(), scale, angle, link, phase }, Some(pat)))
        }
        other => Ok((other, None)),
    }
}

fn info(p: &Pattern, size: (u32, u32), in_doc: bool, in_library: bool) -> Value {
    let f = p.surface.format();
    json!({
        "id": p.id, "name": p.display_name(), "storedName": p.name, "width": size.0, "height": size.1,
        "mode": format!("{:?}", f.mode), "depth": format!("{:?}", f.sample), "alpha": f.alpha,
        "inDocument": in_doc, "inLibrary": in_library,
        "paintable": p.is_paintable(), "paintProblem": p.paint_problem(),
    })
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let doc_pats: Vec<Pattern> = s.active().map(|d| d.doc.patterns.clone()).unwrap_or_default();
    let size = |p: &Pattern| s.patterns.stored.get(&p.id).copied().unwrap_or((p.width, p.height));
    let mut out: Vec<Value> = s.patterns.items.iter().map(|p| info(p, size(p), doc_pats.iter().any(|d| d.id == p.id), true)).collect();
    out.extend(doc_pats.iter().filter(|d| !s.patterns.items.iter().any(|p| p.id == d.id)).map(|p| info(p, (p.width, p.height), true, false)));
    Ok(Value::Array(out))
}

/// Pixels composited at a time by [`composite_surface`]: a band's render and conversion buffers
/// stay a few tens of MB however large the pattern.
const BAND_PIXELS: u32 = 1 << 20;

/// The visible composite over `r` as a surface in the document's pixel format (with alpha), and
/// whether every pixel of it is opaque.
fn composite_surface(doc: &Document, r: Rect) -> (Surface, bool) {
    composite_surface_in_bands(doc, r, (BAND_PIXELS / r.width().max(1)).max(1))
}

/// [`composite_surface`], `band_rows` rows at a time. Rendering and converting one band at a
/// time, and noting opacity as it goes, keeps an 8000 × 8000 pattern (64 MP, about 1 GB per full
/// `f32` copy) from holding several full-size copies at once. Opacity is judged on the stored
/// samples, as the surface quantizes them: an alpha of 0.999 in an 8-bit document is stored as 255.
fn composite_surface_in_bands(doc: &Document, r: Rect, band_rows: u32) -> (Surface, bool) {
    let df = doc.pixel_format();
    let fmt = PixelFormat::new(df.mode, df.sample, true);
    let n = fmt.channels();
    let (width, height) = (r.width(), r.height());
    let mut s = Surface::new(fmt);
    let mut opaque = true;
    let mut px = Vec::new();
    let mut top = 0;
    while top < height {
        let rows = band_rows.min(height - top);
        let buf = photocraft_compose::render(doc, Rect::new(r.x0, r.y0 + top as i32, r.x1, r.y0 + (top + rows) as i32));
        px.clear();
        px.resize(buf.px.len() * n, 0.0f32);
        for (o, p) in px.chunks_exact_mut(n).zip(&buf.px) {
            photocraft_raster::from_rgba_into(&fmt, *p, o);
        }
        let band = Rect::new(0, top as i32, width as i32, (top + rows) as i32);
        s.write_region(band, &px);
        if opaque {
            opaque = s.read_region(band).chunks_exact(n).all(|q| q[n - 1] >= 1.0);
        }
        top += rows;
    }
    (s, opaque)
}

/// The name Define Pattern gives a pattern without one: the active document's name without its
/// extension ("Pattern" without a document), numbered past the library's existing names. The
/// Define Pattern dialog starts from it.
pub fn default_name(s: &Session) -> String {
    let doc_name = s.active().map(|st| st.doc.name.clone()).unwrap_or_default();
    let base =
        std::path::Path::new(&doc_name).file_stem().map(|x| x.to_string_lossy().to_string()).filter(|b| !b.is_empty()).unwrap_or_else(|| "Pattern".into());
    let taken = |n: &str| s.patterns.items.iter().any(|q| q.name == n);
    if !taken(&base) {
        return base;
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or(base)
}

/// Longest edge Define Pattern accepts. Its area is limited too, to what the compositor holds
/// (`Pattern::working_bytes`): 8000 x 8000 at 8 bits, and fewer pixels at 16 bits and float,
/// where the working copy is larger per pixel. What is accepted always paints.
const MAX_DEFINE_EDGE: u32 = 8000;

fn define(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.definePattern";
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = &st.doc;
    let r = match p.get("rect").and_then(Value::as_array) {
        Some(a) if a.len() == 4 => {
            let v: Vec<i32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0).round() as i32).collect();
            Rect::new(v[0], v[1], v[2], v[3])
        }
        _ => doc.selection.as_ref().map_or(doc.bounds(), |m| m.content_bounds()),
    }
    .intersect(&doc.bounds());
    if r.is_empty() {
        return Err(bad(cmd, "the area is empty"));
    }
    if r.width() > MAX_DEFINE_EDGE || r.height() > MAX_DEFINE_EDGE {
        return Err(bad(cmd, "patterns are limited to 8000 × 8000 px"));
    }
    // A pattern the compositor could not hold would define but paint nothing: 8 bits go to the
    // edge limit, 16-bit documents to 33,554,432 px in all, float to 16,777,216 (4096 × 4096).
    let sample = doc.pixel_format().sample;
    let max_px = photocraft_doc::pattern::max_paint_pixels(sample);
    if u64::from(r.width()) * u64::from(r.height()) > max_px {
        return Err(bad(cmd, format!("patterns in a {}-bit document are limited to {max_px} px in all", sample.bytes() * 8)));
    }
    let (mut surf, opaque) = composite_surface(doc, r);
    // Opaque patterns drop their alpha channel (as Photoshop does).
    if opaque {
        let fmt = surf.format();
        surf = surf.convert(PixelFormat::new(fmt.mode, fmt.sample, false));
    }
    let name = match p.get("name").and_then(Value::as_str) {
        Some(n) if !n.trim().is_empty() => n.to_string(),
        _ => default_name(s),
    };
    let pat = Pattern::new(name, surf, r.width(), r.height());
    let id = pat.id.clone();
    let out = info(&pat, (pat.width, pat.height), false, true);
    if !s.patterns.items.iter().any(|q| q.id == id) {
        s.patterns.items.push(pat);
    }
    Ok(json!({"pattern": id, "info": out}))
}

fn library_index(s: &Session, cmd: &str, p: &Value) -> Result<usize> {
    let key = p.get("pattern").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `pattern` (id or name)"))?;
    let found =
        photocraft_doc::pattern::find(&s.patterns.items, key, key).map(|f| f.id.clone()).ok_or_else(|| bad(cmd, format!("no library pattern \"{key}\"")))?;
    Ok(s.patterns.items.iter().position(|q| q.id == found).unwrap_or(0))
}

fn rename(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "pattern.rename";
    let name = p.get("name").and_then(Value::as_str).filter(|n| !n.trim().is_empty()).ok_or_else(|| bad(cmd, "missing `name`"))?.to_string();
    let i = library_index(s, cmd, p)?;
    let id = s.patterns.items[i].id.clone();
    s.patterns.items[i].name = name.clone();
    // A copy stored in the document is renamed too (a document edit, so undoable).
    if s.active().is_some_and(|d| d.doc.patterns.iter().any(|q| q.id == id)) {
        s.edit("Rename Pattern", |doc, _| {
            for q in doc.patterns.iter_mut().filter(|q| q.id == id) {
                q.name = name.clone();
            }
            Ok(())
        })?;
    }
    Ok(json!({"pattern": id, "name": name}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    let i = library_index(s, "pattern.delete", p)?;
    let pat = s.patterns.items.remove(i);
    s.patterns.stored.remove(&pat.id);
    Ok(json!({"deleted": pat.id}))
}

fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "pattern.import";
    // A file on disk, or its bytes (a dropped file, the web build).
    let bytes = match (p.get("path").and_then(Value::as_str), p.get("data").and_then(Value::as_str)) {
        (Some(path), _) => std::fs::read(path).map_err(|e| EngineError::Other(format!("{path}: {e}")))?,
        (None, Some(b64)) => photocraft_paint::tile::b64_decode(b64).ok_or_else(|| bad(cmd, "`data` is not base64"))?,
        (None, None) => return Err(bad(cmd, "missing `path` (.pat file) or `data`")),
    };
    let pats = photocraft_io::pattern_map::read_pat(&bytes).map_err(EngineError::Other)?;
    let mut ids = Vec::new();
    for pat in pats {
        ids.push(pat.id.clone());
        s.patterns.stored.remove(&pat.id);
        s.patterns.upsert(pat);
    }
    Ok(json!({"imported": ids, "count": ids.len()}))
}

fn export(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "pattern.export";
    let path = p.get("path").and_then(Value::as_str).ok_or_else(|| bad(cmd, "missing `path`"))?;
    let pats: Vec<Pattern> = match p.get("patterns").and_then(Value::as_array) {
        Some(keys) => {
            keys.iter().filter_map(Value::as_str).map(|k| resolve(s, k).ok_or_else(|| bad(cmd, format!("no pattern \"{k}\"")))).collect::<Result<_>>()?
        }
        None => s.patterns.items.iter().map(|p| full(s, p).map_err(|e| bad(cmd, e))).collect::<Result<_>>()?,
    };
    let bytes = photocraft_io::pattern_map::write_pat(&pats).map_err(EngineError::Other)?;
    crate::file_cmds::write_file(path, &bytes)?;
    Ok(json!({"path": path, "count": pats.len(), "bytes": bytes.len()}))
}

/// A pattern's scale, angle, link and phase from `p`. The angle is normalised to (-180, 180];
/// a non-finite angle or a phase beyond ±`MAX_PHASE` is an error (it would render NaN).
pub(crate) fn placement(cmd: &str, p: &Value) -> Result<(f32, f32, bool, (f32, f32))> {
    let scale = p.get("scale").and_then(Value::as_f64).unwrap_or(100.0) as f32;
    let max = crate::pattern_fill_cmds::MAX_PHASE;
    let a = p.get("phase").and_then(Value::as_array);
    let c = |i: usize| {
        let v = a.and_then(|a| a.get(i)).and_then(Value::as_f64).unwrap_or(0.0);
        (v.is_finite() && v.abs() <= max).then_some(v as f32).ok_or_else(|| bad(cmd, format!("`phase` needs finite coordinates within ±{max}")))
    };
    let phase = (c(0)?, c(1)?);
    let angle = crate::gradient_fill_cmds::angle_param(cmd, p, 0.0)?;
    Ok(((scale / 100.0).clamp(0.01, 10.0), angle, p.get("link").and_then(Value::as_bool).unwrap_or(true), phase))
}

/// The edit of `layer.newFillLayer.pattern`: a Pattern Fill layer of `pat` placed as `p` asks,
/// above `active`. The Pattern Fill dialog previews it on a copy of the document.
pub(crate) fn insert_fill_layer(
    doc: &mut Document,
    active: &mut Option<photocraft_doc::LayerId>,
    pat: &Pattern,
    p: &Value,
    default_mask: bool,
) -> Result<photocraft_doc::LayerId> {
    let (scale, angle, link, phase) = placement("layer.newFillLayer.pattern", p)?;
    ensure_in_doc(doc, pat);
    let fill = Fill::Pattern { name: pat.name.clone(), scale, id: pat.id.clone(), angle, link, phase };
    let mut l = Layer::new(doc.next_layer_name("Pattern Fill"), LayerContent::Fill(fill));
    crate::commands::mask_new_layer(doc, &mut l, p, "layer.newFillLayer.pattern", default_mask)?;
    let id = crate::layer_place::insert_new(doc, *active, l, crate::layer_place::Place::Auto);
    *active = Some(id);
    Ok(id)
}

/// `layer.newFillLayer.pattern`: a Pattern Fill layer above the active one, masked as the Layers panel's Use Default Masks on Fill Layers option says.
fn new_fill_layer(s: &mut Session, p: &Value) -> Result<Value> {
    let pat = resolve_param(s, "layer.newFillLayer.pattern", p)?;
    let default_mask = crate::commands::layer_panel_option(s, "useDefaultMasksOnFillLayers", true);
    let id = s.edit("New Pattern Fill Layer", |doc, active| insert_fill_layer(doc, active, &pat, p, default_mask))?;
    Ok(json!({"layer": id.0, "pattern": pat.id}))
}

/// `brush.texturePattern`: use a pattern as the brush texture, converted to grey as the luminance of its composite over white.
fn brush_texture(s: &mut Session, p: &Value) -> Result<Value> {
    let pat = resolve_param(s, "brush.texturePattern", p)?;
    let n = pat.is_paintable().then(|| usize::try_from(pat.pixel_count()).ok()).flatten();
    let Some(n) = n else {
        return Err(bad("brush.texturePattern", "the pattern is empty or too large to paint"));
    };
    // Brush textures are grey: luminance of the composite over white.
    // Read a band of rows at a time: a whole-tile RGBA copy would cost 16 bytes a pixel.
    let mut lum: Vec<f32> = Vec::new();
    lum.try_reserve_exact(n).map_err(|_| bad("brush.texturePattern", "not enough memory for the pattern"))?;
    for_each_band(&pat, pat.width, pat.height, |band| {
        lum.extend(band.iter().map(|q| {
            let l = 0.299 * q[0] + 0.587 * q[1] + 0.114 * q[2];
            l * q[3] + (1.0 - q[3])
        }));
    });
    let tile = photocraft_paint::GrayTile::from_f32(pat.width, pat.height, &lum);
    let t = &mut s.tools.brush.texture;
    t.pattern = photocraft_paint::Pattern::Tile(tile);
    t.enabled = p.get("enabled").and_then(Value::as_bool).unwrap_or(true);
    if let Some(sc) = p.get("scale").and_then(Value::as_f64) {
        t.scale = (sc as f32 / 100.0).clamp(0.01, 10.0);
    }
    Ok(json!({"pattern": pat.id}))
}

/// A swatch colour for UIs (average of the tile), straight RGB.
pub fn average_color(p: &Pattern) -> Color {
    // A pattern too large to paint is averaged over its paintable top-left corner.
    let side = photocraft_doc::pattern::MAX_PAINT_PIXELS.isqrt() as u32;
    let (w, h) = (p.width.min(side), p.height.min(side));
    let mut s = [0.0f32; 3];
    for_each_band(p, w, h, |band| s = band.iter().fold(s, |a, q| [a[0] + q[0], a[1] + q[1], a[2] + q[2]]));
    let n = (u64::from(w) * u64::from(h)).max(1) as f32;
    Color::rgb(s[0] / n, s[1] / n, s[2] / n)
}

/// Calls `f` with the straight RGBA pixels of the top-left `w × h` corner of `p`, a band of rows
/// at a time in row-major order.
fn for_each_band(p: &Pattern, w: u32, h: u32, mut f: impl FnMut(&[[f32; 4]])) {
    let mut band = Vec::new();
    for top in (0..h).step_by(photocraft_doc::pattern::BAND_ROWS as usize) {
        let bottom = (top + photocraft_doc::pattern::BAND_ROWS).min(h);
        band.clear();
        band.resize(w as usize * (bottom - top) as usize, [0.0f32; 4]);
        p.surface.read_rgba_into(Rect::new(0, top as i32, w as i32, bottom as i32), &mut band);
        f(&band);
    }
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "edit.definePattern",
            label: "Define Pattern…",
            menu: &["Edit"],
            shortcut: None,
            params: r##"{"name":text,"rect":[x0,y0,x1,y1]?} → {"pattern":id} (name: default the document's name numbered past the library's names; rect: default the selection bounds or else the canvas; added to the library from the visible composite)"##,
            enabled: has_doc,
            journal: true,
            run: define,
        },
        CommandSpec {
            id: "pattern.list",
            label: "Patterns",
            menu: &[],
            shortcut: None,
            params: "{} → [{id,name,width,height,mode,depth,inDocument,inLibrary}]",
            enabled: always,
            journal: false,
            run: list,
        },
        CommandSpec {
            id: "pattern.rename",
            label: "Rename Pattern",
            menu: &[],
            shortcut: None,
            params: r##"{"pattern":id|name,"name":str}"##,
            enabled: has_pattern,
            journal: true,
            run: rename,
        },
        CommandSpec {
            id: "pattern.delete",
            label: "Delete Pattern",
            menu: &[],
            shortcut: None,
            params: r##"{"pattern":id|name} (library only; documents keep their copy)"##,
            enabled: has_pattern,
            journal: true,
            run: delete,
        },
        CommandSpec {
            id: "pattern.import",
            label: "Import Patterns…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".pat file"?,"data":base64 bytes? (instead of a path)} → {imported:[ids],count}"##,
            enabled: always,
            journal: true,
            run: import,
        },
        CommandSpec {
            id: "pattern.export",
            label: "Export Patterns…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".pat file","patterns":[id|name]? (default: the whole library)}"##,
            enabled: has_pattern,
            journal: true,
            run: export,
        },
        CommandSpec {
            id: "layer.newFillLayer.pattern",
            label: "Pattern…",
            menu: &["Layer", "New Fill Layer"],
            shortcut: None,
            params: r##"{"pattern":id|name?=first library pattern,"scale":1..1000=100,"angle":deg=0,"link":bool=true,"phase":[x,y]?,"path":"work"|saved path name|{…path}? (the active path: becomes the vector mask; else the selection is the layer mask)}"##,
            enabled: has_doc,
            journal: true,
            run: new_fill_layer,
        },
        CommandSpec {
            id: "brush.texturePattern",
            label: "Brush Texture Pattern",
            menu: &[],
            shortcut: None,
            params: r##"{"pattern":id|name,"scale":1..1000?,"enabled":bool=true} (sets the session brush's Texture to that pattern)"##,
            enabled: has_pattern,
            journal: true,
            run: brush_texture,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::LayerId;

    fn session(depth: u64) -> Session {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
        s
    }

    fn pixel(s: &Session, x: i32, y: i32) -> [f32; 4] {
        let b = photocraft_compose::render(&s.active().unwrap().doc, Rect::new(x, y, x + 1, y + 1));
        b.px[0]
    }

    #[test]
    fn library_has_builtins_and_lists() {
        let mut s = Session::new();
        let v = s.execute("pattern.list", json!({})).unwrap();
        assert!(v.as_array().unwrap().len() >= 6);
        assert!(v[0]["id"].as_str().unwrap().len() == 36);
        // Built-in ids are stable across sessions.
        assert_eq!(builtin()[0].id, Session::new().patterns.items[0].id);
    }

    #[test]
    fn define_rename_delete_and_pat_round_trip() {
        let mut s = session(8);
        s.execute("edit.fill", json!({"color": "#ff0000"})).unwrap();
        s.execute("select.rect", json!({"x": 2, "y": 3, "width": 5, "height": 4})).unwrap();
        let r = s.execute("edit.definePattern", json!({"name": "Red"})).unwrap();
        let id = r["pattern"].as_str().unwrap().to_string();
        let pat = resolve(&s, &id).unwrap();
        assert_eq!((pat.width, pat.height), (5, 4));
        assert!(!pat.surface.format().alpha, "opaque pattern drops alpha");
        // Same content → same id (replayable); not duplicated.
        let n = s.patterns.items.len();
        s.execute("edit.definePattern", json!({"name": "Red"})).unwrap();
        assert_eq!(s.patterns.items.len(), n);
        s.execute("pattern.rename", json!({"pattern": id, "name": "Crimson"})).unwrap();
        assert_eq!(resolve(&s, "Crimson").unwrap().id, id);
        let dir = std::env::temp_dir().join(format!("pc-pat-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("lib.pat");
        let path = path.to_str().unwrap();
        s.execute("pattern.export", json!({"path": path})).unwrap();
        s.execute("pattern.delete", json!({"pattern": "Crimson"})).unwrap();
        assert!(resolve(&s, "Crimson").is_none());
        assert!(s.execute("pattern.delete", json!({"pattern": "Crimson"})).is_err());
        let v = s.execute("pattern.import", json!({"path": path})).unwrap();
        assert!(v["imported"].as_array().unwrap().iter().any(|x| x == id.as_str()));
        assert_eq!(resolve(&s, &id).unwrap().surface.read_region(Rect::new(0, 0, 5, 4)), pat.surface.read_region(Rect::new(0, 0, 5, 4)));
        // The same file as bytes (a dropped file, the web build).
        let bytes = std::fs::read(path).unwrap();
        s.execute("pattern.delete", json!({"pattern": "Crimson"})).unwrap();
        let v = s.execute("pattern.import", json!({"data": photocraft_paint::tile::b64_encode(&bytes)})).unwrap();
        assert_eq!(v["count"], v["imported"].as_array().unwrap().len());
        assert_eq!(resolve(&s, "Crimson").unwrap().id, id);
        std::fs::remove_dir_all(dir).ok();
        assert!(s.execute("pattern.import", json!({"path": "/nonexistent.pat"})).is_err());
        for bad in [json!({}), json!({"data": "not base64!"}), json!({"data": "aGVsbG8="})] {
            assert!(s.execute("pattern.import", bad.clone()).is_err(), "{bad}");
        }
    }

    /// Review of the 8000 px cap: compositing a band of rows at a time gives the pixels and the
    /// opacity of one pass, at every depth, whatever the band height.
    #[test]
    fn banded_compositing_matches_a_single_pass() {
        for depth in [8u64, 16, 32] {
            for transparent in [false, true] {
                let mut s = Session::new();
                let background = if transparent { "transparent" } else { "white" };
                s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth, "background": background})).unwrap();
                s.execute("select.rect", json!({"x": 3, "y": 4, "width": 20, "height": 13})).unwrap();
                s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
                let doc = &s.active().unwrap().doc;
                let area = Rect::new(2, 1, 38, 29);
                let all = Rect::new(0, 0, 36, 28);
                let (whole, whole_opaque) = composite_surface_in_bands(doc, area, 28);
                for band_rows in [1, 5, 27] {
                    let (banded, opaque) = composite_surface_in_bands(doc, area, band_rows);
                    assert_eq!(opaque, whole_opaque, "depth {depth}, transparent {transparent}, bands of {band_rows}");
                    assert_eq!(banded.read_region(all), whole.read_region(all), "depth {depth}, transparent {transparent}, bands of {band_rows}");
                }
                assert_eq!(whole_opaque, !transparent);
            }
        }
    }

    /// Review of the banded composite: opacity is the stored samples', so an alpha an 8-bit
    /// document stores as 255 is opaque and a float document's 0.999 is not.
    #[test]
    fn alpha_that_stores_as_opaque_counts_as_opaque() {
        for (depth, opaque) in [(8u64, true), (32, false)] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 8, "height": 8, "depth": depth, "background": "transparent"})).unwrap();
            s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
            s.execute("layer.setProps", json!({"opacity": 0.999})).unwrap();
            let (_, all_opaque) = composite_surface_in_bands(&s.active().unwrap().doc, Rect::new(0, 0, 8, 8), 3);
            assert_eq!(all_opaque, opaque, "depth {depth}");
        }
    }

    /// Review of #8902: what Define Pattern accepts, the compositor paints. Areas over
    /// 4096 x 4096, past the compositor's whole-copy cap, define and fill at 8 and 16 bits (held
    /// at the pattern's own depth); a depth that has no smaller exact form is refused up front
    /// with its limit, instead of defining a pattern that paints nothing.
    #[test]
    fn defined_patterns_over_4096_squared_paint_or_are_refused() {
        for depth in [8, 16] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": 4100, "height": 4100, "depth": depth})).unwrap();
            s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
            s.execute("edit.definePattern", json!({"name": "Big"})).unwrap();
            assert!(s.patterns.items.iter().any(|p| p.name == "Big" && p.is_paintable()), "depth {depth}");
            s.execute("file.new", json!({"width": 40, "height": 30, "depth": depth})).unwrap();
            s.execute("layer.newFillLayer.pattern", json!({"pattern": "Big"})).unwrap();
            let [r, g, b, a] = pixel(&s, 5, 5);
            assert!(a == 1.0 && (r - 0.2).abs() < 0.01 && (g - 0.4).abs() < 0.01 && (b - 0.6).abs() < 0.01, "depth {depth}: {:?}", [r, g, b, a]);
        }
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 4100, "height": 4100, "depth": 32})).unwrap();
        let e = s.execute("edit.definePattern", json!({})).unwrap_err().to_string();
        assert!(e.contains("32-bit") && e.contains("16777216"), "{e}");
        assert!(s.patterns.items.iter().all(|p| p.name != "Pattern"), "nothing was defined");
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 6000, "height": 5600, "depth": 16})).unwrap();
        let e = s.execute("edit.definePattern", json!({})).unwrap_err().to_string();
        assert!(e.contains("16-bit") && e.contains("33554432"), "{e}");
    }

    /// A pattern of the largest area Define Pattern accepts at 8 bits is paintable: held at 4
    /// bytes a pixel, 256 MB, as the compositor's cap allows, and it renders.
    #[test]
    fn an_8000_by_8000_pattern_is_defined_and_paints() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 8000, "height": 8000, "depth": 8})).unwrap();
        s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
        s.execute("edit.definePattern", json!({"name": "Largest"})).unwrap();
        let p = s.patterns.items.iter().find(|p| p.name == "Largest").unwrap();
        assert_eq!((p.width, p.height), (8000, 8000));
        assert!(p.is_paintable() && p.working_bytes() == 256_000_000);
        s.execute("file.new", json!({"width": 40, "height": 30, "depth": 8})).unwrap();
        s.execute("layer.newFillLayer.pattern", json!({"pattern": "Largest"})).unwrap();
        let [r, g, b, a] = pixel(&s, 5, 5);
        assert!(a == 1.0 && (r - 0.2).abs() < 0.01 && (g - 0.4).abs() < 0.01 && (b - 0.6).abs() < 0.01, "{:?}", [r, g, b, a]);
    }

    /// Review of #8902: a pattern too large to paint is named by every command that would use it
    /// and by pattern.list, instead of rendering as transparency with no message.
    #[test]
    fn a_pattern_that_would_paint_nothing_is_reported() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 40, "height": 30, "depth": 8})).unwrap();
        // A 32-bit 5000 x 5000 pattern (declared only: 400 MB as a working copy, past the cap).
        let surface = photocraft_raster::Surface::new(photocraft_doc::PixelFormat::RGBA32F);
        s.patterns.items.push(Pattern { id: "huge".into(), name: "Huge".into(), width: 5000, height: 5000, surface });
        let list = s.execute("pattern.list", json!({})).unwrap();
        let huge = list.as_array().unwrap().iter().find(|p| p["name"] == "Huge").unwrap().clone();
        assert_eq!(huge["paintable"], json!(false));
        assert!(huge["paintProblem"].as_str().unwrap().contains("32 bits"), "{huge}");
        for (id, params) in [
            ("layer.newFillLayer.pattern", json!({"pattern": "Huge"})),
            ("layer.layerStyle.patternOverlay", json!({"pattern": "Huge"})),
            ("edit.fill", json!({"contents": "pattern", "pattern": "Huge"})),
            ("brush.texturePattern", json!({"pattern": "Huge"})),
        ] {
            let e = s.execute(id, params).expect_err(id).to_string();
            assert!(e.contains("Huge") && e.contains("would paint nothing"), "{id}: {e}");
        }
        let e = s.execute("layer.newFillLayer.pattern", json!({"pattern": "Huge"})).unwrap_err().to_string();
        assert!(e.contains("would paint nothing") && e.contains("5000 × 5000"), "{e}");
        assert!(list.as_array().unwrap().iter().any(|p| p["name"] != "Huge" && p["paintable"] == json!(true)), "built-in patterns paint");
    }

    #[test]
    fn define_pattern_accepts_edges_over_4000_up_to_8000() {
        for (w, ok) in [(4100, true), (8000, true), (8001, false)] {
            let mut s = Session::new();
            s.execute("file.new", json!({"width": w, "height": 2, "depth": 8})).unwrap();
            s.execute("edit.fill", json!({"color": "#336699"})).unwrap();
            let r = s.execute("edit.definePattern", json!({}));
            assert_eq!(r.is_ok(), ok, "width {w}: {r:?}");
        }
    }

    #[test]
    fn pattern_fill_layer_renders_at_every_depth_and_undoes() {
        for depth in [8u64, 16, 32] {
            let mut s = session(depth);
            let r = s.execute("layer.newFillLayer.pattern", json!({"pattern": "Checkerboard"})).unwrap();
            let id = LayerId(r["layer"].as_u64().unwrap());
            let st = s.active().unwrap();
            assert_eq!(st.doc.patterns.len(), 1, "copied into the document");
            assert!(matches!(&st.doc.layer(id).unwrap().content, LayerContent::Fill(Fill::Pattern { .. })));
            // Checkerboard: 8 px squares of 0.8 and 1.0 grey from the canvas origin.
            let (a, b) = (pixel(&s, 1, 1), pixel(&s, 9, 1));
            assert!((a[0] - 0.8).abs() < 0.01 && (b[0] - 1.0).abs() < 0.01, "{depth}: {a:?} {b:?}");
            s.undo();
            assert!(s.active().unwrap().doc.patterns.is_empty());
            assert!(s.execute("layer.newFillLayer.pattern", json!({"pattern": "nope"})).is_err());
        }
    }

    #[test]
    fn pattern_placement_rejects_non_finite_angle_and_phase() {
        // 1e39 overflowed f32 to infinity: a NaN render, and a fill layer .pcraft refused to save.
        let mut s = session(8);
        let n = s.active().unwrap().doc.layers.len();
        for cmd in ["layer.newFillLayer.pattern", "edit.fill", "paint.bucket"] {
            for bad in [json!({"angle": 1e39}), json!({"angle": "x"}), json!({"phase": [1e39, 0]}), json!({"phase": [0, -1e300]})] {
                let mut p = json!({"pattern": "Checkerboard", "contents": "pattern", "x": 1, "y": 1});
                p.as_object_mut().unwrap().extend(bad.as_object().unwrap().clone());
                assert!(s.execute(cmd, p).is_err(), "{cmd} {bad}");
            }
        }
        assert_eq!(s.active().unwrap().doc.layers.len(), n);
        // The angle is normalised to (-180, 180] like the Pattern Fill dialog's.
        let r = s.execute("layer.newFillLayer.pattern", json!({"pattern": "Checkerboard", "angle": 270, "phase": [3, -4]})).unwrap();
        let id = LayerId(r["layer"].as_u64().unwrap());
        let Some(LayerContent::Fill(Fill::Pattern { angle, phase, .. })) = s.active().unwrap().doc.layer(id).map(|l| &l.content) else { panic!() };
        assert_eq!((*angle, *phase), (-90.0, (3.0, -4.0)));
    }

    #[test]
    fn fill_with_pattern_respects_selection() {
        for depth in [8u64, 16, 32] {
            let mut s = session(depth);
            s.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
            s.execute("edit.fill", json!({"contents": "pattern", "pattern": "Diagonal Lines"})).unwrap();
            let st = s.active().unwrap();
            let surf = st.doc.layers[0].surface().unwrap();
            assert!(surf.pixel(0, 0)[0] < 0.3, "{depth}: line pixel");
            assert!(surf.pixel(4, 0)[0] > 0.9);
            assert!(surf.pixel(20, 20)[0] > 0.99, "outside the selection untouched");
        }
    }

    #[test]
    fn pattern_overlay_composes_with_an_outside_stroke() {
        let mut s = session(8);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 10, "y": 10, "width": 10, "height": 10})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s.execute("layer.layerStyle.patternOverlay", json!({"pattern": "Diagonal Lines"})).unwrap();
        s.execute("layer.layerStyle.stroke", json!({"size": 3, "position": "outside", "color": "#00ff00", "add": true})).unwrap();
        let out = pixel(&s, 8, 15);
        assert!(out[1] > 0.9 && out[0] < 0.1, "stroke outside: {out:?}");
    }

    #[test]
    fn pattern_overlay_renders_and_brush_texture_uses_library() {
        let mut s = session(16);
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#0000ff"})).unwrap();
        s.execute("layer.layerStyle.patternOverlay", json!({"pattern": "Checkerboard", "opacity": 100})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.patterns.len(), 1);
        let p = pixel(&s, 1, 1);
        assert!((p[0] - 0.8).abs() < 0.01 && (p[2] - 0.8).abs() < 0.01, "{p:?}");
        assert!(s.execute("layer.layerStyle.patternOverlay", json!({"pattern": "missing"})).is_err());
        s.execute("layer.layerStyle.stroke", json!({"size": 4, "color": "#ffffff", "add": true})).unwrap();
        let st = s.active().unwrap();
        assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().effects.items.len(), 2);
        s.execute("brush.texturePattern", json!({"pattern": "Dots", "scale": 50})).unwrap();
        assert!(s.tools.brush.texture.enabled);
        assert!(matches!(s.tools.brush.texture.pattern, photocraft_paint::Pattern::Tile(_)));
    }

    #[test]
    fn a_pattern_too_large_to_paint_is_refused_not_allocated() {
        // width × height overflows u32: this panicked (debug) or wrapped (release).
        let mut s = session(8);
        let huge = Pattern { id: "huge".into(), name: "Huge".into(), width: 65_536, height: 65_536, surface: Surface::new(PixelFormat::RGBA8) };
        s.patterns.items.push(huge);
        let e = s.execute("brush.texturePattern", json!({"pattern": "huge"})).unwrap_err();
        assert!(e.to_string().contains("would paint nothing") && e.to_string().contains("65536 × 65536"), "{e}");
    }
}
