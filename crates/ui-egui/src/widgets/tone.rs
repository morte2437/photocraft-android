//! Histograms for the tone editors (Levels, Curves, Threshold) and the Histogram panel.
//!
//! The adjustment editors themselves live in `adjust_editors` and the panel in `histogram_panel`;
//! this module computes and caches the histograms they draw, in the channels of the adjustment's
//! space (RGB, CMYK ink brightness, Lab), and draws them.

use std::sync::Arc;

use egui::{Color32, Rect};
use photocraft_doc::adjust::ToneSpace;
use photocraft_doc::{Document, Layer, LayerId};

use crate::PhotocraftApp;
use crate::theme::Tokens;

static EMPTY: [u32; 256] = [0; 256];

/// 256-bin histograms: the composite then one per channel of the space (RGB: the channel mean,
/// R, G, B; CMYK: composite, C, M, Y, K as channel brightness; Lab: L, a, b). `tag` identifies
/// what they were computed from (see [`HistSource`]).
#[derive(Clone, Debug)]
pub struct Histograms {
    pub tag: u64,
    pub ch: Vec<[u32; 256]>,
    /// Luminosity of the composite (0.30 R + 0.59 G + 0.11 B), whatever the space;
    /// empty for a gray plane's.
    pub lum: [u32; 256],
    /// Photoshop's histogram cache level of a composite histogram: 1 when every document pixel
    /// was counted, `n` when a proxy 2^(n-1) times smaller stood in; 0 for a gray plane's.
    pub level: u32,
}

impl Default for Histograms {
    fn default() -> Self {
        Self { tag: 0, ch: Vec::new(), lum: [0; 256], level: 0 }
    }
}

impl std::ops::Index<usize> for Histograms {
    type Output = [u32; 256];
    fn index(&self, i: usize) -> &[u32; 256] {
        self.ch.get(i).unwrap_or(&EMPTY)
    }
}

/// What a tone editor's histogram shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HistSource {
    /// The image below an adjustment layer (what the adjustment receives).
    BelowLayer(LayerId),
    /// One layer on its own (the target of a destructive Image › Adjustments command).
    Layer(LayerId),
    /// The gray plane a Curves target edits: a layer mask, a saved channel or the Quick Mask.
    /// The dialog shows this plane's own levels, not the colour layers beneath (#5335).
    Channel { layer: LayerId, surface: photocraft_engine::channel_cmds::GraySurface },
    /// A layer and everything beneath it, the layers above hidden (the Histogram panel's
    /// Adjustment Composite: what an adjustment layer outputs).
    UpTo(LayerId),
}

/// The whole composited image (the Histogram panel's Entire Image).
pub const IMAGE: HistSource = HistSource::BelowLayer(LayerId(u64::MAX));

/// The highest histogram cache level: a proxy 2^7 = 128 times smaller.
pub const MAX_CACHE_LEVEL: u32 = 8;

pub(crate) fn tag(source: HistSource, space: ToneSpace) -> u64 {
    let (k, id) = match source {
        HistSource::BelowLayer(l) => (1u64, l.0),
        HistSource::Layer(l) => (2, l.0),
        HistSource::Channel { layer, surface } => match surface {
            photocraft_engine::channel_cmds::GraySurface::LayerMask => (3, layer.0),
            photocraft_engine::channel_cmds::GraySurface::Alpha(i) => (4, i as u64),
            photocraft_engine::channel_cmds::GraySurface::QuickMask => (5, layer.0),
            photocraft_engine::channel_cmds::GraySurface::FilterMask => (6, layer.0),
        },
        HistSource::UpTo(l) => (7, l.0),
    };
    let s = match space {
        ToneSpace::Rgb => 0u64,
        ToneSpace::Cmyk => 1,
        ToneSpace::Lab => 2,
    };
    id.wrapping_mul(0x9e37_79b9_7f4a_7c15) ^ (k << 60) ^ (s << 56)
}

/// Shows only the branch of the layer tree holding `id`. Returns whether `id` is in `layers`.
fn isolate(layers: &mut [Layer], id: LayerId) -> bool {
    let mut found = false;
    for l in layers.iter_mut() {
        let here = l.id == id || l.children_mut().is_some_and(|c| isolate(c, id));
        l.visible = here;
        found |= here;
    }
    found
}

/// Hides every layer composited above `id` (later siblings, and the later siblings of each group
/// holding it). Returns whether `id` is in `layers`.
fn hide_above(layers: &mut [Layer], id: LayerId) -> bool {
    let mut found = false;
    for l in layers.iter_mut() {
        if found {
            l.visible = false;
        } else if l.id == id || l.children_mut().is_some_and(|c| hide_above(c, id)) {
            found = true;
        }
    }
    found
}

/// Histograms of `doc` as seen by `source`, in the channels of `space`, at whatever cache level
/// the document's size calls for (up to [`MAX_CACHE_LEVEL`]).
pub fn compute_histograms(doc: &Document, source: HistSource, space: ToneSpace) -> Histograms {
    compute_histograms_at(doc, source, space, MAX_CACHE_LEVEL)
}

/// [`compute_histograms`] at cache level `max_level` or below (Preferences › Performance › Cache
/// Levels; 1 counts every pixel, the Histogram panel's Uncached Refresh).
pub fn compute_histograms_at(doc: &Document, source: HistSource, space: ToneSpace, max_level: u32) -> Histograms {
    let mut level = 0;
    let mut lum = [0u32; 256];
    let mut h = vec![
        [0u32; 256];
        match space {
            ToneSpace::Rgb => 4,
            ToneSpace::Cmyk => 5,
            ToneSpace::Lab => 3,
        }
    ];
    match source {
        // The gray plane itself: its levels are what the curve edits, whatever the document's
        // colour mode (#5335).
        HistSource::Channel { layer, surface } => {
            let surf = match surface {
                photocraft_engine::channel_cmds::GraySurface::LayerMask => doc.layer(layer).and_then(|l| l.mask.as_ref()).map(|m| &m.surface),
                photocraft_engine::channel_cmds::GraySurface::Alpha(i) => doc.channels.get(i).map(|c| &c.surface),
                photocraft_engine::channel_cmds::GraySurface::QuickMask => doc.quick_mask.as_ref().map(|c| &c.surface),
                photocraft_engine::channel_cmds::GraySurface::FilterMask => {
                    doc.layer(layer).and_then(|l| photocraft_engine::mask_view_cmds::viewed_mask(l, true)).map(|m| &m.surface)
                }
            };
            if let Some(s) = surf
                && let Some(bin) = h.first_mut()
            {
                *bin = gray_bins(s, doc.bounds());
            }
        }
        HistSource::BelowLayer(_) | HistSource::Layer(_) | HistSource::UpTo(_) => {
            // Luminosity rides along as one more histogram after the space's channels.
            h.push([0; 256]);
            level = composite_bins(doc, source, space, max_level, &mut h);
            if let Some(l) = h.pop() {
                lum = l;
            }
        }
    }
    Histograms { tag: tag(source, space), ch: h, lum, level }
}

/// The largest document counted pixel by pixel at the automatic cache level. The web build counts
/// on the UI thread (it has no worker threads), so it reduces documents a quarter that size.
fn exact_limit() -> u64 {
    if cfg!(target_arch = "wasm32") { photocraft_compose::PROXY_THUMBNAIL_PIXELS / 4 } else { photocraft_compose::PROXY_THUMBNAIL_PIXELS }
}

/// The cache level a histogram of `doc` is counted at: 1 (every pixel) up to the size a proxy
/// would stand in for, else the level whose 2^(n-1) reduction leaves about 768 pixels on the
/// longest side, at most `max_level` (clamped to 1..=[`MAX_CACHE_LEVEL`]).
pub fn cache_level(doc: &Document, max_level: u32) -> u32 {
    let longest = doc.size.width.max(doc.size.height);
    let k = longest / 768;
    let auto = if k >= 2 && doc.size.area() > exact_limit() && photocraft_compose::proxy::proxy_faithful(doc) { k.next_power_of_two().ilog2() + 1 } else { 1 };
    auto.min(max_level.clamp(1, MAX_CACHE_LEVEL))
}

/// How many times smaller the proxy counted at `level` (> 1) is: about 768 pixels on the longest
/// side (as at the automatic level), or 2^(level-1) when Cache Levels caps the level below that.
fn proxy_factor(doc: &Document, level: u32) -> u32 {
    let k = doc.size.width.max(doc.size.height) / 768;
    k.min(1 << (level.clamp(1, MAX_CACHE_LEVEL) - 1)).max(1)
}

/// Bins of the composited image below (`BelowLayer`), up to (`UpTo`) or of one layer alone
/// (`Layer`), in `space`, then luminosity in the last of `h`. Returns the cache level counted
/// (see [`cache_level`]): 1 when every document pixel was counted, higher when a downsampled proxy
/// stood in (like Photoshop's histogram cache levels).
///
/// The pixels are counted as they are, never blended: an area-averaged thumbnail invented greys
/// that aren't in the image wherever an edge fell inside one of its cells (#6538). Documents up to
/// [`photocraft_compose::PROXY_THUMBNAIL_PIXELS`] (and those a proxy can't reduce faithfully) are
/// composited in bands and counted exactly, which costs what the thumbnail did (it reduced the
/// same full composite). Larger ones count a nearest-neighbour proxy composite of about the old
/// thumbnail's work: it only holds values the layers have.
fn composite_bins(doc: &Document, source: HistSource, space: ToneSpace, max_level: u32, h: &mut [[u32; 256]]) -> u32 {
    let mut d = doc.clone();
    match source {
        HistSource::BelowLayer(hide) => {
            if let Some(l) = d.layer_mut(hide) {
                l.visible = false;
            }
        }
        HistSource::Layer(id) => {
            isolate(&mut d.layers, id);
        }
        HistSource::UpTo(id) => {
            hide_above(&mut d.layers, id);
        }
        HistSource::Channel { .. } => {}
    }
    let level = cache_level(&d, max_level);
    let d = if level > 1 { photocraft_compose::proxy::proxy_document(&d, proxy_factor(&d, level)) } else { d };
    let _ = photocraft_compose::render_bands(&d, d.bounds(), 0, |band| -> Result<(), ()> {
        #[cfg(not(target_arch = "wasm32"))]
        let part = {
            use rayon::prelude::*;
            band.px
                .par_chunks(64 * 1024)
                .map(|px| {
                    let mut part = vec![[0u32; 256]; h.len()];
                    count_pixels(px, space, &mut part);
                    part
                })
                .reduce(|| vec![[0u32; 256]; h.len()], add_bins)
        };
        #[cfg(target_arch = "wasm32")]
        let part = {
            let mut part = vec![[0u32; 256]; h.len()];
            count_pixels(&band.px, space, &mut part);
            part
        };
        for (dst, src) in h.iter_mut().zip(&part) {
            for (a, b) in dst.iter_mut().zip(src) {
                *a = a.saturating_add(*b);
            }
        }
        Ok(())
    });
    level
}

/// Bin-wise sum of two sets of histograms (saturating: a hostile canvas can exceed `u32`).
#[cfg(not(target_arch = "wasm32"))]
fn add_bins(mut a: Vec<[u32; 256]>, b: Vec<[u32; 256]>) -> Vec<[u32; 256]> {
    for (dst, src) in a.iter_mut().zip(&b) {
        for (x, y) in dst.iter_mut().zip(src) {
            *x = x.saturating_add(*y);
        }
    }
    a
}

/// Luminosity weights (the Histogram panel's Luminosity channel).
pub const LUMINOSITY: [f32; 3] = [0.30, 0.59, 0.11];

/// Counts straight-alpha composite pixels into `h` (one histogram per channel of `space`, then
/// luminosity when `h` has room for it); transparent pixels are skipped.
fn count_pixels(px: &[[f32; 4]], space: ToneSpace, h: &mut [[u32; 256]]) {
    let bin = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as usize;
    let lum_at = match space {
        ToneSpace::Rgb => 4,
        ToneSpace::Cmyk => 5,
        ToneSpace::Lab => 3,
    };
    let mut add = |c: usize, b: usize| {
        if let Some(n) = h.get_mut(c).and_then(|ch| ch.get_mut(b)) {
            *n = n.saturating_add(1);
        }
    };
    for p in px {
        // Skips NaN alpha too.
        if p[3].is_nan() || p[3] <= 0.0 {
            continue;
        }
        let rgb = [p[0], p[1], p[2]].map(|v| v.clamp(0.0, 1.0));
        add(lum_at, bin(rgb.iter().zip(LUMINOSITY).map(|(v, w)| v * w).sum()));
        match space {
            ToneSpace::Rgb => {
                let q = [p[0], p[1], p[2]].map(bin);
                for (c, v) in q.iter().enumerate() {
                    add(c + 1, *v);
                }
                add(0, (q[0] + q[1] + q[2]) / 3);
            }
            ToneSpace::Cmyk => {
                let ink = photocraft_color::convert::rgb_to_cmyk([p[0], p[1], p[2]].map(|v| v.clamp(0.0, 1.0)));
                for (c, v) in ink.iter().enumerate() {
                    add(c + 1, bin(1.0 - v));
                }
                add(0, bin(1.0 - ink.iter().sum::<f32>() / 4.0));
            }
            ToneSpace::Lab => {
                let l = photocraft_color::convert::srgb_to_lab([p[0], p[1], p[2]].map(|v| v.clamp(0.0, 1.0)));
                add(0, bin(l[0] / 100.0));
                add(1, bin((l[1] + 128.0) / 255.0));
                add(2, bin((l[2] + 128.0) / 255.0));
            }
        }
    }
}

/// 256-bin histogram of a gray plane: its levels in 0–255, sampled like the engine's auto-tone
/// histograms so a big canvas costs no more than about a megapixel here (#5335).
fn gray_bins(surf: &photocraft_raster::Surface, area: photocraft_geom::Rect) -> [u32; 256] {
    let mut h = [0u32; 256];
    let step = ((u64::from(area.width()) * u64::from(area.height())) / 1_000_000).max(1).isqrt().max(1) as i32;
    let mut y = area.y0;
    while y < area.y1 {
        let mut x = area.x0;
        while x < area.x1 {
            let v = (surf.sample_channel(x, y, 0).clamp(0.0, 1.0) * 255.0).round() as usize;
            if let Some(bin) = h.get_mut(v) {
                *bin += 1;
            }
            x += step;
        }
        y += step;
    }
    h
}

/// Cached histograms for a tone editor; recomputed only when something other than the editor's
/// own commits changed the document (see [`keep_after_commit`]).
pub fn histograms(app: &mut PhotocraftApp, source: HistSource, space: ToneSpace) -> Arc<Histograms> {
    let Some(st) = app.session.active() else { return Arc::new(Histograms::default()) };
    let (doc_id, rev) = (st.doc.id, st.revision);
    let want = tag(source, space);
    let layer = match source {
        HistSource::BelowLayer(l) | HistSource::Layer(l) | HistSource::UpTo(l) => l,
        HistSource::Channel { layer, .. } => layer,
    };
    let pixels_kept = view_only(st);
    if let Some((d, l, r, h)) = app.tone_hist.as_mut()
        && *d == doc_id
        && *l == layer
        && (*r == rev || (*r + 1 == rev && pixels_kept))
        && h.tag == want
    {
        *r = rev;
        return h.clone();
    }
    let t0 = crate::gpu_canvas::now_ms();
    let h = Arc::new(compute_histograms(&st.doc, source, space));
    app.perf.span("histogram", crate::gpu_canvas::now_ms() - t0);
    app.tone_hist = Some((doc_id, layer, rev, h.clone()));
    h
}

/// Whether the latest revision changed no pixels (selecting a layer, say): cached histograms of
/// the previous revision still hold (#125).
pub(crate) fn view_only(st: &photocraft_engine::DocState) -> bool {
    st.last_damage.is_some_and(|r| r.is_empty())
}

/// After an editor commits its own adjustment layer, the image below it is unchanged: keep the
/// cached histogram valid for the new revision. Call with the revision seen before the commit.
pub fn keep_after_commit(app: &mut PhotocraftApp, layer: LayerId, before: Option<u64>) {
    let now = app.session.active().map(|s| s.revision);
    if let (Some((_, l, r, _)), Some(before), Some(now)) = (app.tone_hist.as_mut(), before, now)
        && *l == layer
        && *r == before
    {
        *r = now;
    }
}

/// Display scale for a 256-bin histogram: bar heights are `v / top` clamped to 1, linear
/// to the tallest bin like Photoshop. A percentile of the sorted bins degenerates on
/// sparse histograms (few occupied levels sort 0 into every low index), drawing tiny
/// counts as tall as the real peaks (#6574).
/// Extracted from [`draw_histogram`] so the scaling is unit-testable.
pub fn histogram_scale_top(h: &[u32; 256]) -> f32 {
    h.iter().copied().max().unwrap_or(1).max(1) as f32 * 1.1
}

pub fn draw_histogram(p: &egui::Painter, r: Rect, h: &[u32; 256], color: Color32) {
    let top = histogram_scale_top(h);
    let mut pts = vec![egui::pos2(r.left(), r.bottom())];
    for (i, v) in h.iter().enumerate() {
        let x = r.left() + (i as f32 + 0.5) / 256.0 * r.width();
        let y = r.bottom() - (*v as f32 / top).min(1.0) * r.height();
        pts.push(egui::pos2(x, y));
    }
    pts.push(egui::pos2(r.right(), r.bottom()));
    // One mesh of vertical strips: no anti-aliasing feather, so no seams between bins.
    let mut mesh = egui::Mesh::default();
    for w in pts.windows(2).skip(1).take(256) {
        let (a, b) = (w[0], w[1]);
        let i = mesh.vertices.len() as u32;
        for q in [egui::pos2(a.x, r.bottom()), a, b, egui::pos2(b.x, r.bottom())] {
            mesh.colored_vertex(q, color);
        }
        mesh.add_triangle(i, i + 1, i + 2);
        mesh.add_triangle(i, i + 2, i + 3);
    }
    p.add(mesh);
}

/// Display colour of channel `name` (curve, histogram and gradient bars).
pub fn channel_color(name: &str, t: &Tokens) -> Color32 {
    match name {
        "red" => Color32::from_rgb(235, 80, 80),
        "green" => Color32::from_rgb(80, 205, 95),
        "blue" => Color32::from_rgb(90, 140, 255),
        "cyan" => Color32::from_rgb(40, 200, 230),
        "magenta" => Color32::from_rgb(225, 70, 200),
        "yellow" => Color32::from_rgb(235, 210, 40),
        "a" => Color32::from_rgb(220, 90, 140),
        "b" => Color32::from_rgb(200, 180, 60),
        _ => t.text,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn histogram_ignores_the_edited_adjustment() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap(); // white
        s.execute("layer.newAdjustmentLayer.invert", json!({})).unwrap();
        let st = s.active().unwrap();
        let h = compute_histograms(&st.doc, HistSource::BelowLayer(st.active_layer.unwrap()), ToneSpace::Rgb);
        assert!(h[0][255] > 0 && h[0][0] == 0, "sees the white image, not the inverted result");
        assert_eq!(h[9], [0; 256], "out-of-range channels are empty, not a panic");
    }

    #[test]
    fn layer_histogram_sees_only_that_layer_and_spaces_have_their_channels() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap(); // white background
        let bg = s.active().unwrap().doc.layers[0].id;
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 32})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        let st = s.active().unwrap();
        let h = compute_histograms(&st.doc, HistSource::Layer(bg), ToneSpace::Rgb);
        assert!(h[0][255] > 0 && h[0][0] == 0, "the black layer above is hidden");
        assert_eq!(compute_histograms(&st.doc, HistSource::Layer(bg), ToneSpace::Cmyk).ch.len(), 5);
        assert_eq!(compute_histograms(&st.doc, HistSource::Layer(bg), ToneSpace::Lab).ch.len(), 3);
    }

    /// #5335: the Curves histogram of a targeted gray plane describes that plane, not the
    /// colour layers beneath it: a channel filled with 25% grey over a black layer shows a
    /// spike at 25% grey, not at black.
    #[test]
    fn channel_histogram_describes_the_gray_plane() {
        use photocraft_engine::channel_cmds::GraySurface;
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 32, "height": 32})).unwrap(); // white background
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": 32, "height": 32})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("channel.new", json!({"name": "Sky sel", "fill": "white"})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        let b = photocraft_geom::Rect::from_xywh(0, 0, 32, 32);
        s.edit("fill-planes", |doc, active| {
            doc.channels[0].surface.fill_rect(b, &[0.25]);
            doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap().surface.fill_rect(b, &[0.0]);
            Ok(())
        })
        .unwrap();
        let st = s.active().unwrap();
        let layer = st.active_layer.unwrap();
        let h = compute_histograms(&st.doc, HistSource::Channel { layer, surface: GraySurface::Alpha(0) }, ToneSpace::Rgb);
        assert!(h[0][64] > 0, "a spike at 25% grey, got {:?}", &h[0][0..80]);
        assert_eq!(h[0][0], 0, "not the black layer beneath");
        let m = compute_histograms(&st.doc, HistSource::Channel { layer, surface: GraySurface::LayerMask }, ToneSpace::Rgb);
        assert!(m[0][0] > 0, "the hidden mask is black");
        // Each plane keeps its own cache entry: switching targets recomputes.
        assert_ne!(
            tag(HistSource::Channel { layer, surface: GraySurface::Alpha(0) }, ToneSpace::Rgb),
            tag(HistSource::Channel { layer, surface: GraySurface::LayerMask }, ToneSpace::Rgb)
        );
        assert_ne!(tag(HistSource::Channel { layer, surface: GraySurface::LayerMask }, ToneSpace::Rgb), tag(HistSource::Layer(layer), ToneSpace::Rgb));
        // Every source of one layer has its own tag (the filter mask and the Adjustment
        // Composite once shared one).
        let sources = [
            HistSource::BelowLayer(layer),
            HistSource::Layer(layer),
            HistSource::UpTo(layer),
            HistSource::Channel { layer, surface: GraySurface::LayerMask },
            HistSource::Channel { layer, surface: GraySurface::QuickMask },
            HistSource::Channel { layer, surface: GraySurface::FilterMask },
        ];
        for (i, a) in sources.iter().enumerate() {
            for b in &sources[i + 1..] {
                assert_ne!(tag(*a, ToneSpace::Rgb), tag(*b, ToneSpace::Rgb), "{a:?} and {b:?}");
            }
        }
    }

    /// #6574: bar heights scale linearly to the tallest bin. A 90/10 black/white
    /// histogram draws the small bar at a ninth of the big one, not full height.
    #[test]
    fn histogram_scale_is_proportional_for_two_level_images() {
        let mut h = [0u32; 256];
        h[0] = 100_000;
        h[255] = 900_000;
        let top = histogram_scale_top(&h);
        let small = (h[0] as f32 / top).min(1.0);
        let big = (h[255] as f32 / top).min(1.0);
        assert!((small / big - 1.0 / 9.0).abs() < 0.02, "small/big = {}, top = {top}", small / big);
    }

    /// #6574: a sparse histogram (few occupied levels) must not scale to a bin
    /// index that is 0: the old `sorted.get(250)` degenerated to `top = 1.1`,
    /// drawing a few hundred samples as tall as hundreds of thousands.
    #[test]
    fn histogram_scale_does_not_degenerate_on_sparse_histograms() {
        let mut h = [0u32; 256];
        h[0] = 500_000;
        h[128] = 300;
        h[255] = 500_000;
        let top = histogram_scale_top(&h);
        assert!(top > h[128] as f32, "scale {top} must exceed the tiny middle bin");
        let tiny = (h[128] as f32 / top).min(1.0);
        assert!(tiny < 0.05, "tiny bin draws near-flat, got {tiny}");
    }

    /// A white document with black filled over columns `0..split` (a layer above the background).
    fn split_doc(w: u32, h: u32, split: u32, depth: Option<u32>) -> photocraft_engine::Session {
        let mut s = photocraft_engine::Session::new();
        let mut p = json!({"width": w, "height": h});
        if let Some(bits) = depth {
            p["depth"] = json!(bits);
        }
        s.execute("file.new", p).unwrap(); // white
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("select.rect", json!({"x": 0, "y": 0, "width": split, "height": h})).unwrap();
        s.execute("edit.fill", json!({"color": "#000000"})).unwrap();
        s.execute("select.deselect", json!({})).unwrap();
        s
    }

    fn panel_histograms(s: &photocraft_engine::Session) -> Histograms {
        compute_histograms(&s.active().unwrap().doc, HistSource::BelowLayer(LayerId(u64::MAX)), ToneSpace::Rgb)
    }

    fn mean(h: &[u32; 256]) -> f64 {
        let n: u64 = h.iter().map(|v| u64::from(*v)).sum();
        h.iter().enumerate().map(|(i, v)| i as f64 * f64::from(*v)).sum::<f64>() / n as f64
    }

    /// #6538: a black/white split counts only black and white, wherever the edge falls (the old
    /// 384 px area-averaged thumbnail turned the cell holding the edge mid-grey), and counts
    /// every pixel: cache level 1.
    #[test]
    fn split_image_histogram_counts_only_black_and_white() {
        for (w, h, split) in [(1000, 1000, 334), (1000, 1000, 500), (1000, 600, 1), (777, 333, 389), (2000, 50, 1999), (64, 64, 32)] {
            let s = split_doc(w, h, split, None);
            let hist = panel_histograms(&s);
            assert_eq!(hist.level, 1, "{w}x{h}: every pixel is counted");
            for c in 0..4 {
                let stray: Vec<(usize, u32)> = hist[c].iter().copied().enumerate().filter(|&(i, v)| v > 0 && i != 0 && i != 255).collect();
                assert!(stray.is_empty(), "{w}x{h} split {split}, channel {c}: invented values {stray:?}");
                assert_eq!(u64::from(hist[c][0]), u64::from(split) * u64::from(h), "{w}x{h} split {split}: black pixels");
                assert_eq!(u64::from(hist[c][255]), u64::from(w - split) * u64::from(h), "{w}x{h} split {split}: white pixels");
            }
        }
        let half = panel_histograms(&split_doc(1000, 1000, 500, None));
        assert!((mean(&half[0]) - 127.5).abs() < 1e-9, "a 50/50 split has mean 127.5, got {}", mean(&half[0]));
    }

    /// The same at 16 and 32 bits per channel: the count does not go through an 8-bit thumbnail.
    #[test]
    fn split_image_histogram_is_exact_at_high_bit_depths() {
        for bits in [16, 32] {
            let s = split_doc(1000, 400, 334, Some(bits));
            assert_eq!(s.active().unwrap().doc.depth.bits(), bits, "the document is {bits}-bit");
            let hist = panel_histograms(&s);
            for c in 0..4 {
                let stray: Vec<(usize, u32)> = hist[c].iter().copied().enumerate().filter(|&(i, v)| v > 0 && i != 0 && i != 255).collect();
                assert!(stray.is_empty(), "{bits}-bit channel {c}: invented values {stray:?}");
                assert_eq!(hist[c][0], 334 * 400);
                assert_eq!(hist[c][255], 666 * 400);
            }
        }
    }

    /// Above the exact-count size a nearest-neighbour proxy stands in (cache level > 1): it is
    /// smaller, yet still only holds black and white.
    #[test]
    fn large_split_image_counts_a_nearest_proxy_without_inventing_greys() {
        let s = split_doc(6000, 3000, 2001, None);
        let hist = panel_histograms(&s);
        assert!(hist.level > 1, "a proxy stood in, level {}", hist.level);
        let n: u64 = hist[0].iter().map(|v| u64::from(*v)).sum();
        assert!(n > 0 && n < 6000 * 3000, "counted {n} proxy pixels");
        for c in 0..4 {
            let stray: Vec<(usize, u32)> = hist[c].iter().copied().enumerate().filter(|&(i, v)| v > 0 && i != 0 && i != 255).collect();
            assert!(stray.is_empty(), "channel {c}: invented values {stray:?}");
        }
        assert!((mean(&hist[0]) - 255.0 * 3999.0 / 6000.0).abs() < 1.0, "mean {}", mean(&hist[0]));
    }
}
