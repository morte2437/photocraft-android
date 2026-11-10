//! Photoshop's Levels clipping display (#3328). While Alt is held and the black or white Input
//! slider of the Image › Adjustments › Levels dialog is dragged, the canvas shows what the input
//! levels clip instead of the preview:
//! - black slider: the image turns white; a channel the input levels clip to black drops to 0, so
//!   pixels clipped in every channel are black and pixels clipped in red alone are cyan;
//! - white slider: the image turns black; a channel clipped to white rises to 255, so red alone is
//!   red and all three are white.
//!
//! Releasing Alt or the mouse shows the preview again; the document is never touched. Grayscale
//! documents show black and white only. CMYK and Lab documents, and commands that edit a channel,
//! a layer mask or a non-pixel layer, keep the normal preview.
//!
//! The display is an opaque texture over the visible part of the image, at about the screen's
//! resolution: one texel per block of `factor`×`factor` document pixels. A block shows a channel
//! clipped when any pixel in it is, so a lone clipped highlight stays visible zoomed out. Clipping
//! is a threshold on each channel's sample (the input stretch is monotone), so the target's
//! per-block minimum and maximum are scanned once per view and a slider move only compares them
//! with the new thresholds.
//!
//! State is the dialog field `__clipping` (`"shadows"` / `"highlights"`): [`sync`] sets it when an
//! Alt-drag starts and clears it when the drag ends, and automation can set it with
//! `ui.dialog.set`. OK never sees it (`filter_dialog::params_of` drops `__` keys).

use egui::{Color32, Mesh, TextureOptions, pos2};
use photocraft_color::{ColorMode, SampleType};
use photocraft_compose::adjust::{levels_float, levels_q};
use photocraft_doc::adjust::{LevelsChannel, ToneSpace};
use photocraft_doc::{Adjustment, DocId, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Map, Value, json};

use crate::PhotocraftApp;
use crate::canvas::ViewXform;

/// The dialog field that turns the display on.
pub const FIELD: &str = "__clipping";

/// A pixel counts where the selection covers at least this much (the command blends the rest).
const SELECTED: f32 = 0.5;

/// Which input slider's clipping is shown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ClipEnd {
    /// The black slider: what the input levels clip to black.
    Shadows,
    /// The white slider: what they clip to white.
    Highlights,
}

impl ClipEnd {
    /// The [`FIELD`] value that asks for this end.
    fn name(self) -> &'static str {
        match self {
            ClipEnd::Shadows => "shadows",
            ClipEnd::Highlights => "highlights",
        }
    }

    /// The display a `__clipping` field value asks for (anything else: off).
    pub fn parse(v: Option<&Value>) -> Option<Self> {
        match v.and_then(Value::as_str) {
            Some("shadows") => Some(ClipEnd::Shadows),
            Some("highlights") => Some(ClipEnd::Highlights),
            _ => None,
        }
    }
}

/// Mirror an Alt-drag on the Levels dialog's black or white Input slider (editor state under
/// `mem`) into the dialog's [`FIELD`]. Only changes are written, so a display turned on through
/// automation stays until it is cleared or a drag ends.
pub fn sync(ctx: &egui::Context, mem: egui::Id, fields: &mut Map<String, Value>) {
    if fields.get("__adjust").and_then(Value::as_str) != Some("levels") {
        return;
    }
    let held = ctx.input(|i| i.modifiers.alt && i.pointer.primary_down());
    let live = match crate::adjust_editors::levels_drag(ctx, mem) {
        Some(0) if held => Some(ClipEnd::Shadows),
        Some(2) if held => Some(ClipEnd::Highlights),
        _ => None,
    };
    let last_id = mem.with("levels-clipping");
    let last: Option<ClipEnd> = ctx.data(|d| d.get_temp(last_id)).flatten();
    if live == last {
        return;
    }
    ctx.data_mut(|d| d.insert_temp(last_id, live));
    match live {
        Some(end) => {
            fields.insert(FIELD.into(), json!(end.name()));
        }
        None => {
            fields.remove(FIELD);
        }
    }
    // The canvas paints before the dialogs: show the change on the next frame.
    ctx.request_repaint();
}

/// Which ends the Levels input levels clip a sample `v` (0..1) of one channel to, as
/// (black, white): the channel's own input range or, after it, the composite's. `quantum` is the
/// document's (whole levels, as the compositor rounds them; None: 32-bit float, unclamped). Gamma
/// and the output levels don't matter: what reaches an end of an input range is flattened there,
/// wherever the output levels then put it.
pub fn classify(master: &LevelsChannel, channel: &LevelsChannel, v: f32, quantum: Option<f32>) -> (bool, bool) {
    let eval = |ch: &LevelsChannel, v: f32| if quantum.is_some() { levels_q(ch, v, quantum) } else { levels_float(ch, v) };
    let full = |ch: &LevelsChannel| LevelsChannel { out_black: 0.0, out_white: 1.0, ..ch.clone() };
    let stretch = |ch: &LevelsChannel| LevelsChannel { gamma: 1.0, ..full(ch) };
    let own = eval(&stretch(channel), v);
    let composite = eval(&stretch(master), eval(&full(channel), v));
    (own <= 0.0 || composite <= 0.0, own >= 1.0 || composite >= 1.0)
}

/// A sample as a 16-bit key: exact for 8- and 16-bit documents (their samples are on this grid),
/// 32-bit samples clamped to 0..1 (beyond is clipped either way). Non-finite: None.
fn key(v: f32) -> Option<u16> {
    v.is_finite().then(|| (v.clamp(0.0, 1.0) * 65535.0 + 0.5) as u16)
}

/// Per channel (R, G, B): the highest key clipped to black and the lowest clipped to white (None:
/// no key clips that way).
pub type Thresholds = [(Option<u16>, Option<u16>); 3];

/// The [`Thresholds`] of the Levels `adj` in a document of `depth`. None for anything but Levels
/// in the RGB channel space (which Grayscale documents use too).
pub fn thresholds(adj: &Adjustment, depth: SampleType) -> Option<Thresholds> {
    let Adjustment::Levels { master, per_channel, space: ToneSpace::Rgb, .. } = adj else { return None };
    let quantum = photocraft_compose::adjustment_quantum(depth);
    let at = |ch: &LevelsChannel, k: u16| classify(master, ch, f32::from(k) / 65535.0, quantum);
    Some(std::array::from_fn(|c| {
        let ch = &per_channel[c];
        // Black clipping holds below a key and white clipping above one (the stretch is monotone).
        let black = (at(ch, 0).0).then(|| last_true(|k| at(ch, k).0));
        let white = (at(ch, u16::MAX).1).then(|| first_true(|k| at(ch, k).1));
        (black, white)
    }))
}

/// The largest key where `f` holds, given it holds at 0 and stops holding at most once.
fn last_true(f: impl Fn(u16) -> bool) -> u16 {
    let (mut lo, mut hi) = (0u32, u32::from(u16::MAX) + 1);
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if f(mid as u16) {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    lo as u16
}

/// The smallest key where `f` holds, given it holds at `u16::MAX` and starts holding at most once.
fn first_true(f: impl Fn(u16) -> bool) -> u16 {
    let (mut lo, mut hi) = (-1i32, i32::from(u16::MAX));
    while hi - lo > 1 {
        let mid = (lo + hi) / 2;
        if f(mid as u16) {
            hi = mid;
        } else {
            lo = mid;
        }
    }
    hi as u16
}

/// Per block of the scanned area: each channel's minimum (0..3) and maximum (3..6) key over the
/// pixels that count. A block with none has min > max.
type Extremes = [u16; 6];
const NONE: Extremes = [u16::MAX, u16::MAX, u16::MAX, 0, 0, 0];

/// The target's per-block extremes over `rect` (document pixels, `factor`×`factor` blocks).
pub struct Stats {
    rect: Rect,
    factor: u32,
    size: [usize; 2],
    blocks: Vec<Extremes>,
}

/// Scan `surface` (a layer's pixels) over `rect` in `factor`×`factor` blocks. Pixels count when
/// visible (alpha > 0, finite) and, with a `selection`, at least half selected.
pub fn scan(surface: &Surface, selection: Option<&Surface>, rect: Rect, factor: u32) -> Stats {
    let factor = factor.max(1);
    let f = factor as usize;
    let size = [rect.width().div_ceil(factor) as usize, rect.height().div_ceil(factor) as usize];
    let mut blocks = vec![NONE; size[0] * size[1]];
    let row = |by: usize, out: &mut [Extremes]| {
        let y0 = rect.y0.saturating_add((by * f) as i32);
        let band = Rect::new(rect.x0, y0, rect.x1, y0.saturating_add(factor as i32).min(rect.y1));
        let width = band.width() as usize;
        let mut px = vec![[0.0f32; 4]; width * band.height() as usize];
        surface.read_rgba_into(band, &mut px);
        let mut coverage = Vec::new();
        let stride = selection.map_or(1, |s| s.channels().max(1));
        if let Some(s) = selection {
            s.read_region_into(band, &mut coverage);
        }
        for (i, p) in px.iter().enumerate() {
            let visible = p[3] > 0.0;
            let selected = selection.is_none() || coverage.get(i * stride).is_some_and(|k| *k >= SELECTED);
            if !visible || !selected {
                continue;
            }
            let (Some(r), Some(g), Some(b)) = (key(p[0]), key(p[1]), key(p[2])) else { continue };
            let Some(e) = out.get_mut((i % width.max(1)) / f) else { continue };
            for (c, k) in [r, g, b].into_iter().enumerate() {
                e[c] = e[c].min(k);
                e[c + 3] = e[c + 3].max(k);
            }
        }
    };
    if size[0] > 0 {
        #[cfg(not(target_arch = "wasm32"))]
        {
            use rayon::prelude::*;
            blocks.par_chunks_mut(size[0]).enumerate().for_each(|(by, out)| row(by, out));
        }
        #[cfg(target_arch = "wasm32")]
        for (by, out) in blocks.chunks_mut(size[0]).enumerate() {
            row(by, out);
        }
    }
    Stats { rect, factor, size, blocks }
}

/// The display colour of a pixel clipped to black in the channels of `black` and to white in
/// those of `white` (bit 0 red, 1 green, 2 blue).
pub fn colour(end: ClipEnd, black: u8, white: u8) -> Color32 {
    let on = |mask: u8, c: u8| if mask & (1 << c) != 0 { 255 } else { 0 };
    match end {
        ClipEnd::Shadows => {
            let m = 7 ^ black;
            Color32::from_rgb(on(m, 0), on(m, 1), on(m, 2))
        }
        ClipEnd::Highlights => Color32::from_rgb(on(white, 0), on(white, 1), on(white, 2)),
    }
}

/// The display of `stats` for `end` under the per-channel `thresholds`, one colour per block (in
/// parallel off the web: this runs on every slider move).
pub fn image(stats: &Stats, thresholds: &Thresholds, end: ClipEnd) -> Vec<Color32> {
    // Integer bounds (none clipped: below every key / above every key) and the 8 possible colours.
    let lo = thresholds.map(|(lo, _)| lo.map_or(-1, i32::from));
    let hi = thresholds.map(|(_, hi)| hi.map_or(i32::from(u16::MAX) + 1, i32::from));
    let palette: [Color32; 8] = std::array::from_fn(|m| match end {
        ClipEnd::Shadows => colour(end, m as u8, 0),
        ClipEnd::Highlights => colour(end, 0, m as u8),
    });
    let block = |e: &Extremes| {
        let mut mask = 0usize;
        for c in 0..3 {
            let (min, max) = (i32::from(e[c]), i32::from(e[c + 3]));
            let clipped = match end {
                ClipEnd::Shadows => min <= max && min <= lo[c],
                ClipEnd::Highlights => min <= max && max >= hi[c],
            };
            mask |= usize::from(clipped) << c;
        }
        palette[mask]
    };
    #[cfg(not(target_arch = "wasm32"))]
    {
        use rayon::prelude::*;
        stats.blocks.par_iter().map(block).collect()
    }
    #[cfg(target_arch = "wasm32")]
    {
        stats.blocks.iter().map(block).collect()
    }
}

/// What the display needs from the app: the end shown and the Levels adjustment, for the active
/// document's target layer.
struct Request {
    end: ClipEnd,
    adj: Adjustment,
    doc: std::sync::Arc<photocraft_doc::Document>,
    revision: u64,
    target: LayerId,
}

/// The open Levels dialog's fields, if any. A Smart Filter's Levels (`__smartFilter`) edits the
/// filter, not the layer's pixels, so it has no display (`adjust_preview` keeps it off the layer
/// path too).
fn dialog(app: &PhotocraftApp) -> Option<&Map<String, Value>> {
    app.ui.dialogs.iter().map(|d| &d.fields).find(|f| f.get("__adjust").and_then(Value::as_str) == Some("levels") && !f.contains_key("__smartFilter"))
}

/// The display the open Levels dialog asks for on document `idx`. None when no Levels dialog
/// shows clipping, `idx` isn't the active document, or the document can't show it: CMYK and Lab,
/// a channel, mask or Quick Mask target, or a target that isn't a pixel layer.
fn request(app: &PhotocraftApp, idx: usize) -> Option<Request> {
    let fields = dialog(app)?;
    let end = ClipEnd::parse(fields.get(FIELD))?;
    let params = crate::filter_dialog::params_of(fields);
    if app.session.active_index() != Some(idx) || crate::adjust_preview::targets_channel(app, idx, "levels", &params) {
        return None;
    }
    let st = app.session.documents().get(idx)?;
    if !matches!(st.doc.mode, ColorMode::Rgb | ColorMode::Grayscale) {
        return None;
    }
    let target = st.active_layer?;
    if !matches!(st.doc.layer(target).map(|l| &l.content), Some(LayerContent::Raster(_))) {
        return None;
    }
    let adj = photocraft_engine::adjust_params::from_params("levels", &params, None, st.doc.mode).ok()?;
    Some(Request { end, adj, doc: st.doc.clone(), revision: st.revision, target })
}

/// The display's per-view state on the app.
pub struct Cache {
    /// What `stats` scanned: document, revision, target, area and block size.
    key: (DocId, u64, LayerId, Rect, u32),
    stats: Stats,
    /// The end and thresholds the texture shows.
    drawn: Option<(ClipEnd, Thresholds)>,
    tex: Option<egui::TextureHandle>,
}

/// Most blocks the display builds (2048 × 2048). A 2560×1440 canvas still gets one block per
/// device pixel, a bigger one bigger blocks: the block stats stay under 50 MB, the texture at
/// 16 MB, and a slider move near the 2 ms `levels_clipping_bench` measures for 3.7M blocks.
const MAX_BLOCKS: u64 = 2048 * 2048;

/// The visible part of the document in `xf` and the block size giving about one texel per screen
/// pixel (`ppp` physical pixels per point), within `max_side` texels a side and [`MAX_BLOCKS`].
fn view_area(xf: &ViewXform, bounds: Rect, ppp: f32, max_side: u32) -> Option<(Rect, u32)> {
    let device_zoom = xf.zoom * ppp;
    if !(device_zoom.is_finite() && device_zoom > 0.0) {
        return None;
    }
    let visible = crate::canvas::visible_doc_rect(xf).intersect(&bounds);
    if visible.is_empty() {
        return None;
    }
    let max_side = max_side.max(1);
    let (w, h) = (visible.width(), visible.height());
    let budget = ((u64::from(w) * u64::from(h)) as f64 / MAX_BLOCKS as f64).sqrt().ceil() as u32;
    let mut factor = ((1.0 / device_zoom).floor().clamp(1.0, 65536.0) as u32).max(w.div_ceil(max_side)).max(h.div_ceil(max_side)).max(budget);
    // Snap to the block grid so panning keeps the blocks where they were. That can widen the area
    // by up to a block, so grow the blocks until the snapped area fits.
    loop {
        let f = i32::try_from(factor).ok()?;
        let r = Rect::new(visible.x0.div_euclid(f) * f, visible.y0.div_euclid(f) * f, visible.x1, visible.y1).intersect(&bounds);
        let (bw, bh) = (r.width().div_ceil(factor), r.height().div_ceil(factor));
        if bw <= max_side && bh <= max_side && u64::from(bw) * u64::from(bh) <= MAX_BLOCKS {
            return Some((r, factor));
        }
        factor = factor.checked_add(1)?;
    }
}

/// Draw the Levels clipping display over the image in canvas view `xf` of document `idx`, if one
/// is asked for. Drops the cache once no Levels dialog is open. Only the main canvas calls it:
/// the one cache follows that view.
pub fn paint(app: &mut PhotocraftApp, ctx: &egui::Context, painter: &egui::Painter, xf: &ViewXform, idx: usize) {
    if dialog(app).is_none() {
        app.levels_clipping = None;
        return;
    }
    // A background job holds the workers and may replace the document: no thresholds, scan or
    // image until it ends.
    if app.session.has_jobs() {
        return;
    }
    let Some(req) = request(app, idx) else { return };
    let Some(thr) = thresholds(&req.adj, req.doc.pixel_format().sample) else { return };
    let max_side = (ctx.input(|i| i.max_texture_side) as u32).clamp(256, 8192);
    let Some((rect, factor)) = view_area(xf, req.doc.bounds(), ctx.pixels_per_point(), max_side) else { return };
    let key = (req.doc.id, req.revision, req.target, rect, factor);
    if app.levels_clipping.as_ref().is_none_or(|c| c.key != key) {
        let Some(LayerContent::Raster(surface)) = req.doc.layer(req.target).map(|l| &l.content) else { return };
        let stats = scan(surface, req.doc.selection.as_ref(), rect, factor);
        let tex = app.levels_clipping.take().and_then(|c| c.tex);
        app.levels_clipping = Some(Cache { key, stats, drawn: None, tex });
    }
    let Some(cache) = app.levels_clipping.as_mut() else { return };
    if cache.drawn != Some((req.end, thr)) || cache.tex.is_none() {
        let img = egui::ColorImage::new(cache.stats.size, image(&cache.stats, &thr, req.end));
        match &mut cache.tex {
            Some(t) => t.set(img, TextureOptions::NEAREST),
            None => cache.tex = Some(ctx.load_texture("levels-clipping", img, TextureOptions::NEAREST)),
        }
        cache.drawn = Some((req.end, thr));
    }
    let Some(tex) = cache.tex.as_ref().map(egui::TextureHandle::id) else { return };
    // The texture covers whole blocks; the last row and column may reach past the area.
    let s = &cache.stats;
    let f = s.factor as f32;
    let uv = [s.rect.width() as f32 / (s.size[0] as f32 * f).max(1.0), s.rect.height() as f32 / (s.size[1] as f32 * f).max(1.0)];
    let (x0, y0, x1, y1) = (s.rect.x0 as f32, s.rect.y0 as f32, s.rect.x1 as f32, s.rect.y1 as f32);
    let mut mesh = Mesh::with_texture(tex);
    for (p, t) in [((x0, y0), (0.0, 0.0)), ((x1, y0), (uv[0], 0.0)), ((x1, y1), (uv[0], uv[1])), ((x0, y1), (0.0, uv[1]))] {
        mesh.vertices.push(egui::epaint::Vertex { pos: xf.to_screen(p.0, p.1), uv: pos2(t.0, t.1), color: Color32::WHITE });
    }
    mesh.add_triangle(0, 1, 2);
    mesh.add_triangle(0, 2, 3);
    painter.add(egui::Shape::mesh(mesh));
}

#[cfg(test)]
#[path = "levels_clipping_tests.rs"]
mod tests;
