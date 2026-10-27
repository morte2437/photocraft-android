//! Histogram panel (Window › Histogram): Compact, Expanded and All Channels
//! views, a channel and a source selector, statistics for the level under the pointer or a
//! dragged tonal range, the cached-data warning and Uncached Refresh, and the adjusted histogram
//! over the original while an adjustment previews.
//!
//! Histograms are counted off the UI thread (`tone::compute_histograms_at`) for the document
//! revision, source and preview they describe, and kept until one of those changes; the panel
//! shows the last one with the cached-data warning until the new one lands.

use std::sync::Arc;

use egui::{Color32, Rect, Sense, pos2, vec2};
use photocraft_color::ColorMode;
use photocraft_doc::adjust::ToneSpace;
use photocraft_doc::{DocId, Document, LayerContent};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::tone::{self, HistSource, Histograms};

/// Where the panel options are kept between launches (`prefs.dialogs`).
const OPTIONS_PREF: &str = "histogramPanel";

/// The panel's view (its menu's Compact / Expanded / All Channels View).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistView {
    /// The histogram alone, of the entire image (Photoshop's default view).
    #[default]
    Compact,
    /// The histogram with the channel and source selectors, refresh and statistics.
    Expanded,
    /// Expanded, plus one histogram per colour channel (no alpha, spot or mask channels).
    AllChannels,
}

/// The Channel menu. Which entries a document offers depends on its colour mode
/// ([`channel_options`]); one it lacks shows as its composite.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistChannel {
    /// RGB, CMYK, Lab's Lightness or Gray.
    #[default]
    Composite,
    Red,
    Green,
    Blue,
    Cyan,
    Magenta,
    Yellow,
    Black,
    A,
    B,
    Luminosity,
    /// The colour channels overlaid in their colours.
    Colors,
}

/// What the histogram counts (the Source menu, shown for documents with several layers).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum HistSourceChoice {
    /// The composite of every layer.
    #[default]
    EntireImage,
    /// The selected layer on its own.
    SelectedLayer,
    /// The selected adjustment layer and every layer beneath it.
    AdjustmentComposite,
}

/// The panel options (its menu and selectors), kept between launches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HistogramPanelState {
    pub view: HistView,
    /// Kept when the view changes (Expanded ↔ All Channels); in All Channels View it picks only
    /// the top histogram.
    pub channel: HistChannel,
    /// Show Channels in Color: single channels in Expanded and All Channels View in their colour
    /// (Compact View always colours them; Colors is always in colour).
    pub show_channels_in_color: bool,
    pub show_statistics: bool,
    pub source: HistSourceChoice,
}

impl Default for HistogramPanelState {
    fn default() -> Self {
        Self {
            view: HistView::Compact,
            channel: HistChannel::Composite,
            show_channels_in_color: false,
            show_statistics: true,
            source: HistSourceChoice::EntireImage,
        }
    }
}

/// Saves the panel options with the preferences.
pub fn persist_options(app: &mut PhotocraftApp) {
    if let Ok(value) = serde_json::to_value(app.ui.histogram_panel) {
        app.session.prefs.edit(|p| {
            p.dialogs.insert(OPTIONS_PREF.into(), value);
        });
    }
}

/// Restores the saved panel options; unreadable ones keep the defaults.
pub fn restore_options(app: &mut PhotocraftApp) {
    if let Some(v) = app.session.prefs().dialogs.get(OPTIONS_PREF)
        && let Ok(state) = serde_json::from_value(v.clone())
    {
        app.ui.histogram_panel = state;
    }
}

// ---------------------------------------------------------------------------------------------
// Channels

/// The channels a document in `mode` offers, with their labels, in menu order.
pub fn channel_options(mode: ColorMode) -> Vec<(HistChannel, &'static str)> {
    use HistChannel::*;
    match tone_space(mode) {
        _ if gray(mode) => vec![(Composite, "Gray")],
        ToneSpace::Cmyk => vec![
            (Composite, "CMYK"),
            (Cyan, "Cyan"),
            (Magenta, "Magenta"),
            (Yellow, "Yellow"),
            (Black, "Black"),
            (Luminosity, "Luminosity"),
            (Colors, "Colors"),
        ],
        ToneSpace::Lab => vec![(Composite, "Lightness"), (A, "a"), (B, "b"), (Luminosity, "Luminosity")],
        ToneSpace::Rgb => vec![(Composite, "RGB"), (Red, "Red"), (Green, "Green"), (Blue, "Blue"), (Luminosity, "Luminosity"), (Colors, "Colors")],
    }
}

fn gray(mode: ColorMode) -> bool {
    matches!(mode, ColorMode::Grayscale | ColorMode::Bitmap | ColorMode::Duotone)
}

/// The space the panel counts a document of `mode` in.
pub fn tone_space(mode: ColorMode) -> ToneSpace {
    photocraft_engine::adjust_params::default_space(mode)
}

/// `want`, or the composite when a document of `mode` doesn't have it.
pub fn resolve_channel(mode: ColorMode, want: HistChannel) -> HistChannel {
    if channel_options(mode).iter().any(|(c, _)| *c == want) { want } else { HistChannel::Composite }
}

/// The single colour channels of a document of `mode` (All Channels View; Colors overlays them).
pub fn color_channels(mode: ColorMode) -> &'static [HistChannel] {
    use HistChannel::*;
    match tone_space(mode) {
        _ if gray(mode) => &[],
        ToneSpace::Cmyk => &[Cyan, Magenta, Yellow, Black],
        ToneSpace::Lab => &[A, B],
        ToneSpace::Rgb => &[Red, Green, Blue],
    }
}

/// The 256 bins of `channel` in `h` (counted in the space of a document of `mode`).
pub fn bins(h: &Histograms, mode: ColorMode, channel: HistChannel) -> &[u32; 256] {
    use HistChannel::*;
    let i = match (tone_space(mode), channel) {
        (_, Luminosity) => return &h.lum,
        (ToneSpace::Rgb, Red) | (ToneSpace::Cmyk, Cyan) | (ToneSpace::Lab, A) => 1,
        (ToneSpace::Rgb, Green) | (ToneSpace::Cmyk, Magenta) | (ToneSpace::Lab, B) => 2,
        (ToneSpace::Rgb, Blue) | (ToneSpace::Cmyk, Yellow) => 3,
        (ToneSpace::Cmyk, Black) => 4,
        _ => 0,
    };
    &h[i]
}

/// The label a channel is listed under in a document of `mode`.
pub fn channel_label(mode: ColorMode, channel: HistChannel) -> &'static str {
    channel_options(mode).into_iter().find(|(c, _)| *c == channel).map_or("", |(_, l)| l)
}

/// The id of a channel (`ui.inspect`, the serde name).
fn channel_id(channel: HistChannel) -> String {
    serde_json::to_value(channel).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default()
}

/// The colour a single channel draws in (saturated, as in All Channels View).
pub fn channel_paint(channel: HistChannel) -> Option<Color32> {
    use HistChannel::*;
    Some(match channel {
        Red => Color32::from_rgb(235, 30, 30),
        Green => Color32::from_rgb(30, 235, 30),
        Blue => Color32::from_rgb(40, 40, 240),
        Cyan => Color32::from_rgb(0, 200, 235),
        Magenta => Color32::from_rgb(230, 0, 160),
        Yellow => Color32::from_rgb(240, 220, 0),
        Black => Color32::from_gray(110),
        A => Color32::from_rgb(220, 90, 140),
        B => Color32::from_rgb(200, 180, 60),
        _ => return None,
    })
}

/// The level of `channel` at a straight RGBA 0..1 pixel, as the histogram bins it (the Color
/// Sampler marks).
pub fn channel_level(mode: ColorMode, channel: HistChannel, rgba: [f32; 4]) -> u8 {
    use HistChannel::*;
    let rgb = [rgba[0], rgba[1], rgba[2]].map(|v| if v.is_nan() { 0.0 } else { v.clamp(0.0, 1.0) });
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    let lum = || q(rgb.iter().zip(tone::LUMINOSITY).map(|(v, w)| v * w).sum());
    match (tone_space(mode), channel) {
        (_, Luminosity) => lum(),
        (ToneSpace::Rgb, Red) => q(rgb[0]),
        (ToneSpace::Rgb, Green) => q(rgb[1]),
        (ToneSpace::Rgb, Blue) => q(rgb[2]),
        (ToneSpace::Rgb, _) => ((u16::from(q(rgb[0])) + u16::from(q(rgb[1])) + u16::from(q(rgb[2]))) / 3) as u8,
        (ToneSpace::Cmyk, c) => {
            let ink = photocraft_color::convert::rgb_to_cmyk(rgb);
            let v = match c {
                Cyan => ink[0],
                Magenta => ink[1],
                Yellow => ink[2],
                Black => ink[3],
                _ => ink.iter().sum::<f32>() / 4.0,
            };
            q(1.0 - v)
        }
        (ToneSpace::Lab, c) => {
            let l = photocraft_color::convert::srgb_to_lab(rgb);
            match c {
                A => q((l[1] + 128.0) / 255.0),
                B => q((l[2] + 128.0) / 255.0),
                _ => q(l[0] / 100.0),
            }
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Statistics

/// The Histogram statistics. Mean, Std Dev, Median and Pixels describe the whole
/// histogram; Level, Count and Percentile the level under the pointer or the dragged range.
#[derive(Clone, Copy, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Stats {
    pub mean: f64,
    pub std_dev: f64,
    pub median: u8,
    pub pixels: u64,
    /// The inspected levels, `lo..=hi` (one level when they are equal), if any.
    pub level: Option<[u8; 2]>,
    /// Pixels at the inspected levels.
    pub count: Option<u64>,
    /// Percent of the pixels at or below the inspected level; for a range, in it.
    pub percentile: Option<f64>,
    /// Percent of the pixels at 0 and at 255: shadow and highlight clipping.
    pub clipped: [f64; 2],
}

/// The statistics of `h`, inspecting `levels` (a level or a range, in either order). None when
/// nothing was counted.
pub fn stats(h: &[u32; 256], levels: Option<[u8; 2]>) -> Option<Stats> {
    let n: u64 = h.iter().map(|v| u64::from(*v)).sum();
    if n == 0 {
        return None;
    }
    let nf = n as f64;
    let mean = h.iter().enumerate().map(|(i, v)| i as f64 * f64::from(*v)).sum::<f64>() / nf;
    let var = h.iter().enumerate().map(|(i, v)| (i as f64 - mean).powi(2) * f64::from(*v)).sum::<f64>() / nf;
    let mut acc = 0u64;
    let median = h
        .iter()
        .position(|v| {
            acc += u64::from(*v);
            acc * 2 >= n
        })
        .unwrap_or(0) as u8;
    let level = levels.map(|[a, b]| [a.min(b), a.max(b)]);
    let sum = |lo: u8, hi: u8| h.get(usize::from(lo)..=usize::from(hi)).map_or(0, |s| s.iter().map(|v| u64::from(*v)).sum::<u64>());
    let count = level.map(|[lo, hi]| sum(lo, hi));
    let percentile = level.map(|[lo, hi]| 100.0 * if lo == hi { sum(0, hi) } else { sum(lo, hi) } as f64 / nf);
    let clipped = [100.0 * f64::from(h[0]) / nf, 100.0 * f64::from(h[255]) / nf];
    Some(Stats { mean, std_dev: var.sqrt(), median, pixels: n, level, count, percentile, clipped })
}

// ---------------------------------------------------------------------------------------------
// Computing and caching

/// What one panel histogram was counted from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HistKey {
    pub doc: DocId,
    pub revision: u64,
    /// `tone::tag` of the source and space.
    pub source: u64,
    /// The adjustment preview's display key; 0 for the document itself.
    pub preview: u64,
    /// The highest cache level allowed (1: an Uncached Refresh).
    pub max_level: u32,
}

/// A counted panel histogram.
#[derive(Clone, Debug)]
pub struct PanelHist {
    pub key: HistKey,
    /// When it landed (ms), for throttling recounts while the document keeps changing.
    pub at: f64,
    pub hist: Arc<Histograms>,
}

/// A histogram being counted off the UI thread: its key, and the receiver of (key, ms, result).
pub(crate) type HistJob = (HistKey, std::sync::mpsc::Receiver<(HistKey, f64, Arc<Histograms>)>);

/// The source the panel counts for `choice` in `st`: Entire Image whenever the choice doesn't
/// apply (a single layer, no selected layer, or Adjustment Composite without an adjustment layer
/// selected).
pub fn resolve_source(st: &photocraft_engine::DocState, choice: HistSourceChoice) -> (HistSourceChoice, HistSource) {
    if st.doc.layer_count() < 2 {
        return (HistSourceChoice::EntireImage, tone::IMAGE);
    }
    let selected = st.active_layer.and_then(|id| st.doc.layer(id));
    match (choice, selected) {
        (HistSourceChoice::SelectedLayer, Some(l)) => (choice, HistSource::Layer(l.id)),
        (HistSourceChoice::AdjustmentComposite, Some(l)) if matches!(l.content, LayerContent::Adjustment(_)) => (choice, HistSource::UpTo(l.id)),
        _ => (HistSourceChoice::EntireImage, tone::IMAGE),
    }
}

/// Whether Adjustment Composite applies to `st` (an adjustment layer is selected).
fn adjustment_selected(st: &photocraft_engine::DocState) -> bool {
    st.active_layer.and_then(|id| st.doc.layer(id)).is_some_and(|l| matches!(l.content, LayerContent::Adjustment(_)))
}

/// Preferences › Performance › Cache Levels: the highest level the panel counts at by default.
fn max_cache_level(app: &PhotocraftApp) -> u32 {
    app.session.prefs().performance.cache_levels.clamp(1, tone::MAX_CACHE_LEVEL)
}

/// The source the panel counts: Compact View shows no Source menu and represents the entire image.
fn source_choice(opts: &HistogramPanelState) -> HistSourceChoice {
    if opts.view == HistView::Compact { HistSourceChoice::EntireImage } else { opts.source }
}

/// The key of the document's histogram the panel wants now, with the space and source it counts.
fn committed_key(app: &PhotocraftApp) -> Option<(HistKey, ToneSpace, HistSource)> {
    let st = app.session.active()?;
    let (doc, revision) = (st.doc.id, st.revision);
    let space = tone_space(st.doc.mode);
    let (_, source) = resolve_source(st, source_choice(&app.ui.histogram_panel));
    let max_level = if app.hist_uncached == Some((doc, revision)) { 1 } else { max_cache_level(app) };
    Some((HistKey { doc, revision, source: tone::tag(source, space), preview: 0, max_level }, space, source))
}

/// The document an open adjustment previews on the canvas (an Image › Adjustments dialog through
/// its preview layer, or an adjustment layer's Properties drag) and its display key.
fn adjustment_preview(app: &mut PhotocraftApp) -> Option<(Arc<Document>, u64)> {
    let idx = app.session.active_index()?;
    if let Some(shown) = crate::adjust_preview::display_doc(app, idx) {
        return Some(shown);
    }
    app.live_adjust.as_ref()?;
    let (doc, key) = crate::canvas::display_doc(app, idx);
    (key != 0).then_some((doc, key))
}

/// What the panel wants shown now: the document's histogram key, and the preview's document and
/// key while an adjustment previews.
struct Want {
    committed: HistKey,
    preview: Option<(Arc<Document>, HistKey)>,
    space: ToneSpace,
    source: HistSource,
}

fn want(app: &mut PhotocraftApp) -> Option<Want> {
    let preview = adjustment_preview(app);
    let (committed, space, source) = committed_key(app)?;
    let preview = preview.map(|(d, k)| (d, HistKey { preview: k | 1, ..committed }));
    Some(Want { committed, preview, space, source })
}

/// Brings the panel's histograms up to date as far as one frame should: lands a finished count,
/// keeps entries a view-only revision didn't invalidate, and starts the next count (at most one at
/// a time, at most every 250 ms per entry). `ctx` repaints when a worker delivers.
pub fn update(app: &mut PhotocraftApp, ctx: &egui::Context) {
    // A finished worker lands its histogram in the slot its key names. A worker that died
    // without sending clears the job so the next frame schedules a new one.
    if let Some((_, rx)) = &app.hist_job {
        match rx.try_recv() {
            Ok((key, ms, hist)) => {
                app.perf.span("histogram", ms);
                // egui's clock works on every target; `gpu_canvas::now_ms` is always 0 on wasm32.
                let entry = PanelHist { key, at: ctx.input(|i| i.time) * 1000.0, hist };
                if key.preview == 0 {
                    app.doc_hist = Some(entry);
                } else {
                    app.doc_hist_preview = Some(entry);
                }
                app.hist_job = None;
            }
            Err(std::sync::mpsc::TryRecvError::Disconnected) => app.hist_job = None,
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }
    }
    // A revision that changed no pixels (selecting a layer, say) keeps the histogram of the one
    // before (#125), and an Uncached Refresh with it.
    if let Some(st) = app.session.active()
        && tone::view_only(st)
    {
        let (doc, rev) = (st.doc.id, st.revision);
        if app.hist_uncached == Some((doc, rev.wrapping_sub(1))) {
            app.hist_uncached = Some((doc, rev));
        }
        if let Some(e) = app.doc_hist.as_mut()
            && e.key.doc == doc
            && e.key.revision.wrapping_add(1) == rev
        {
            e.key.revision = rev;
        }
    }
    let Some(want) = want(app) else { return };
    if want.preview.is_none() {
        app.doc_hist_preview = None;
    }
    if app.hist_job.is_some() {
        ctx.request_repaint_after(std::time::Duration::from_millis(30));
        return;
    }
    let now = ctx.input(|i| i.time) * 1000.0;
    let due = |e: &Option<PanelHist>| e.as_ref().is_none_or(|e| e.key.doc != want.committed.doc || now - e.at > 250.0);
    let stale = |e: &Option<PanelHist>, k: &HistKey| e.as_ref().is_none_or(|e| e.key != *k);
    // An Uncached Refresh is asked for, so it doesn't wait out the throttle (Cache Levels 1 alone
    // counts at level 1 too, and still waits).
    let forced = app.hist_uncached == Some((want.committed.doc, want.committed.revision));
    let job = if stale(&app.doc_hist, &want.committed) {
        (forced || due(&app.doc_hist)).then(|| (app.session.active().map(|st| st.doc.clone()), want.committed))
    } else if let Some((doc, key)) = &want.preview
        && stale(&app.doc_hist_preview, key)
    {
        due(&app.doc_hist_preview).then(|| (Some(doc.clone()), *key))
    } else {
        None
    };
    match job {
        Some((Some(doc), key)) => spawn(app, ctx, doc, key, want.source, want.space),
        Some((None, _)) => {}
        None if stale(&app.doc_hist, &want.committed) || want.preview.as_ref().is_some_and(|(_, k)| stale(&app.doc_hist_preview, k)) => {
            ctx.request_repaint_after(std::time::Duration::from_millis(260));
        }
        None => {}
    }
}

/// Counts `doc` for `key` on a worker thread (inline on the web, where there are none, or when a
/// spawn fails), delivered through `app.hist_job`.
fn spawn(app: &mut PhotocraftApp, ctx: &egui::Context, doc: Arc<Document>, key: HistKey, source: HistSource, space: ToneSpace) {
    let (tx, rx) = std::sync::mpsc::channel();
    #[cfg(not(target_arch = "wasm32"))]
    let spawned = {
        let doc = doc.clone();
        let tx = tx.clone();
        let ctx = ctx.clone();
        std::thread::Builder::new()
            .name("histogram".into())
            .spawn(move || {
                let t0 = crate::gpu_canvas::now_ms();
                let h = Arc::new(tone::compute_histograms_at(&doc, source, space, key.max_level));
                let _ = tx.send((key, crate::gpu_canvas::now_ms() - t0, h));
                ctx.request_repaint();
            })
            .is_ok()
    };
    #[cfg(target_arch = "wasm32")]
    let spawned = {
        let _ = ctx;
        false
    };
    if !spawned {
        let t0 = crate::gpu_canvas::now_ms();
        let h = Arc::new(tone::compute_histograms_at(&doc, source, space, key.max_level));
        let _ = tx.send((key, crate::gpu_canvas::now_ms() - t0, h));
    }
    app.hist_job = Some((key, rx));
}

/// Uncached Refresh: recount the active document's current state at cache level 1 (every pixel).
pub fn uncached_refresh(app: &mut PhotocraftApp) {
    if let Some(st) = app.session.active() {
        app.hist_uncached = Some((st.doc.id, st.revision));
    }
}

/// The histogram the panel shows now: the preview's while an adjustment previews (with the
/// document's as the original), else the document's. `stale` when it isn't the one wanted yet.
struct Shown {
    hist: Arc<Histograms>,
    original: Option<Arc<Histograms>>,
    stale: bool,
    mode: ColorMode,
    layers: usize,
    adjustment: bool,
}

fn shown(app: &mut PhotocraftApp) -> Option<Shown> {
    let want = want(app)?;
    let st = app.session.active()?;
    let (mode, layers, adjustment) = (st.doc.mode, st.doc.layer_count(), adjustment_selected(st));
    let same_doc = |e: &PanelHist| e.key.doc == want.committed.doc;
    let committed = app.doc_hist.as_ref().filter(|e| same_doc(e));
    let preview = app.doc_hist_preview.as_ref().filter(|e| same_doc(e) && want.preview.is_some());
    Some(match (preview, committed) {
        (Some(p), c) => Shown {
            hist: p.hist.clone(),
            original: c.map(|c| c.hist.clone()),
            stale: want.preview.as_ref().is_none_or(|(_, k)| *k != p.key),
            mode,
            layers,
            adjustment,
        },
        (None, Some(c)) => Shown { hist: c.hist.clone(), original: None, stale: c.key != want.committed || want.preview.is_some(), mode, layers, adjustment },
        (None, None) => return None,
    })
}

// ---------------------------------------------------------------------------------------------
// Drawing

/// One histogram and its colour, for [`draw_overlaid`].
type Layer<'a> = (&'a [u32; 256], Color32);

/// The colour of a band covered by `covering`: additive for light channels (red over green is
/// yellow, all three grey), multiplied for inks (cyan over magenta is blue).
fn mix(covering: impl Iterator<Item = Color32>, multiply: bool) -> Color32 {
    if multiply {
        covering.fold(Color32::WHITE, |a, b| {
            Color32::from_rgb(
                (u16::from(a.r()) * u16::from(b.r()) / 255) as u8,
                (u16::from(a.g()) * u16::from(b.g()) / 255) as u8,
                (u16::from(a.b()) * u16::from(b.b()) / 255) as u8,
            )
        })
    } else {
        covering.fold(Color32::BLACK, |a, b| Color32::from_rgb(a.r().saturating_add(b.r()), a.g().saturating_add(b.g()), a.b().saturating_add(b.b())))
    }
}

/// Draws `layers` overlaid on one shared scale, each outlined like [`tone::draw_histogram`]
/// (straight lines between the bin centres, down to the baseline at both ends). Where several
/// cover a band it takes their [`mix`]ed colour. Each span between two centres is split where
/// outlines cross, so the channel order is constant within every piece and the bands are exact
/// trapezoids: no steps, and no draw-order-dependent blending.
fn draw_overlaid(p: &egui::Painter, r: Rect, layers: &[Layer<'_>], multiply: bool) {
    p.add(overlaid_mesh(r, layers, multiply));
}

fn overlaid_mesh(r: Rect, layers: &[Layer<'_>], multiply: bool) -> egui::Mesh {
    let top = layers.iter().map(|(h, _)| tone::histogram_scale_top(h)).fold(1.0f32, f32::max);
    // Outline points: the left baseline, the 256 bin centres, the right baseline (heights 0..1).
    let xs: Vec<f32> =
        std::iter::once(r.left()).chain((0..256).map(|i| r.left() + (i as f32 + 0.5) / 256.0 * r.width())).chain(std::iter::once(r.right())).collect();
    let height = |h: &[u32; 256], i: usize| if i == 0 || i > 256 { 0.0 } else { (h.get(i - 1).copied().unwrap_or(0) as f32 / top).min(1.0) };
    let mut mesh = egui::Mesh::default();
    let y = |v: f32| r.bottom() - v * r.height();
    for i in 0..xs.len().saturating_sub(1) {
        let (Some(&x0), Some(&x1)) = (xs.get(i), xs.get(i + 1)) else { continue };
        let a: Vec<f32> = layers.iter().map(|(h, _)| height(h, i)).collect();
        let b: Vec<f32> = layers.iter().map(|(h, _)| height(h, i + 1)).collect();
        // Where any two outlines cross within the span (as a fraction of it).
        let mut cuts = vec![0.0f32, 1.0];
        for j in 0..layers.len() {
            for k in j + 1..layers.len() {
                let (Some(aj), Some(bj), Some(ak), Some(bk)) = (a.get(j), b.get(j), a.get(k), b.get(k)) else { continue };
                let d = (bj - aj) - (bk - ak);
                if d.abs() > f32::EPSILON {
                    let t = (ak - aj) / d;
                    if t > 0.0 && t < 1.0 {
                        cuts.push(t);
                    }
                }
            }
        }
        cuts.sort_by(f32::total_cmp);
        for w in cuts.windows(2) {
            let (t0, t1) = (w[0], w[1]);
            if t1 - t0 <= f32::EPSILON {
                continue;
            }
            let at = |t: f32| -> Vec<f32> { a.iter().zip(&b).map(|(a, b)| a + (b - a) * t).collect() };
            let (left, right, mid) = (at(t0), at(t1), at((t0 + t1) / 2.0));
            let mut order: Vec<usize> = (0..layers.len()).collect();
            order.sort_by(|&m, &n| mid.get(m).copied().unwrap_or(0.0).total_cmp(&mid.get(n).copied().unwrap_or(0.0)));
            let (px0, px1) = (x0 + (x1 - x0) * t0, x0 + (x1 - x0) * t1);
            // Bottom up: each band is covered by the channels at least that tall.
            let (mut below_l, mut below_r) = (0.0f32, 0.0f32);
            for (n, &c) in order.iter().enumerate() {
                let (Some(&tl), Some(&tr)) = (left.get(c), right.get(c)) else { continue };
                if tl > below_l || tr > below_r {
                    let color = mix(order.iter().skip(n).filter_map(|&m| layers.get(m).map(|l| l.1)), multiply);
                    let v = mesh.vertices.len() as u32;
                    for q in [pos2(px0, y(below_l)), pos2(px0, y(tl)), pos2(px1, y(tr)), pos2(px1, y(below_r))] {
                        mesh.colored_vertex(q, color);
                    }
                    mesh.add_triangle(v, v + 1, v + 2);
                    mesh.add_triangle(v, v + 2, v + 3);
                }
                (below_l, below_r) = (tl.max(below_l), tr.max(below_r));
            }
        }
    }
    mesh
}

/// The colour a single-channel graph draws in: its channel colour when colourised, else the
/// composite grey.
fn graph_color(channel: HistChannel, colored: bool, t: &Tokens) -> Color32 {
    let grey = if t.pro { Color32::from_gray(215) } else { Color32::from_gray(70) };
    if colored { channel_paint(channel).unwrap_or(grey) } else { grey }
}

/// Draws `channel` of `h` in `r` (Colors overlays the colour channels).
fn draw_channel(p: &egui::Painter, r: Rect, h: &Histograms, mode: ColorMode, channel: HistChannel, colored: bool, t: &Tokens) {
    if channel == HistChannel::Colors {
        let layers: Vec<Layer<'_>> = color_channels(mode).iter().filter_map(|c| Some((bins(h, mode, *c), channel_paint(*c)?))).collect();
        draw_overlaid(p, r, &layers, tone_space(mode) == ToneSpace::Cmyk);
    } else {
        tone::draw_histogram(p, r, bins(h, mode, channel), graph_color(channel, colored, t));
    }
}

/// The histogram graph with its marks and pointer interaction. Returns the levels under the
/// pointer (hover) for the statistics.
fn graph(app: &mut PhotocraftApp, ui: &mut egui::Ui, s: &Shown, channel: HistChannel, colored: bool, height: f32) -> Option<u8> {
    let t = Tokens::get(ui.ctx());
    let w = ui.available_width();
    let (r, resp) = ui.allocate_exact_size(vec2(w, height), Sense::click_and_drag());
    let p = ui.painter_at(r);
    p.rect_filled(r, 0.0, t.field);
    let level_at = |x: f32| (((x - r.left()) / r.width().max(1.0)) * 256.0).floor().clamp(0.0, 255.0) as u8;
    let x_of = |l: u8| r.left() + (f32::from(l) + 0.5) / 256.0 * r.width();
    // The selected range behind the bars.
    if let Some([a, b]) = app.ui.histogram_range {
        let (lo, hi) = (a.min(b), a.max(b));
        let band = Rect::from_x_y_ranges(r.left() + f32::from(lo) / 256.0 * r.width()..=r.left() + (f32::from(hi) + 1.0) / 256.0 * r.width(), r.y_range());
        p.rect_filled(band, 0.0, t.accent.gamma_multiply(0.35));
    }
    draw_channel(&p, r, &s.hist, s.mode, channel, colored, &t);
    // The original histogram outlined over the adjusted one while an adjustment previews.
    if let Some(o) = &s.original {
        outline(&p, r, bins(o, s.mode, if channel == HistChannel::Colors { HistChannel::Composite } else { channel }), t.text);
    }
    // The Color Samplers' levels.
    let mark_channel = if channel == HistChannel::Colors { HistChannel::Composite } else { channel };
    for v in crate::panels::sampler_readouts(app).iter() {
        let rgb = v["rgb"].as_array().map(|a| a.iter().filter_map(Value::as_f64).map(|c| (c / 255.0) as f32).collect::<Vec<_>>()).unwrap_or_default();
        let (Some(&red), Some(&green), Some(&blue)) = (rgb.first(), rgb.get(1), rgb.get(2)) else { continue };
        let x = x_of(channel_level(s.mode, mark_channel, [red, green, blue, 1.0]));
        p.line_segment([pos2(x, r.bottom() - 9.0), pos2(x, r.bottom())], egui::Stroke::new(1.5, t.text));
        p.text(pos2(x + 2.0, r.bottom() - 9.0), egui::Align2::LEFT_BOTTOM, v["number"].to_string(), egui::FontId::proportional(9.0), t.text);
    }
    // The cached-data warning: click it (or double-click the graph) for an Uncached Refresh.
    if s.stale || s.hist.level > 1 {
        let icon = Rect::from_min_size(r.right_top() + vec2(-18.0, 2.0), vec2(16.0, 16.0));
        crate::icons::paint(ui, icon, "triangle-alert", 12.0, t.warning);
        let warn =
            ui.interact(icon, ui.scope_id().with("hist-cached-warning"), Sense::click()).on_hover_text(tl!("Click to refresh histogram with uncached data"));
        if warn.clicked() {
            uncached_refresh(app);
        }
    }
    if resp.double_clicked() {
        uncached_refresh(app);
    }
    let pointer = resp.interact_pointer_pos().or_else(|| resp.hover_pos());
    let id = ui.scope_id().with("hist-drag-from");
    if resp.drag_started()
        && let Some(pos) = resp.interact_pointer_pos()
    {
        ui.data_mut(|d| d.insert_temp(id, level_at(pos.x)));
    }
    if resp.dragged()
        && let (Some(from), Some(pos)) = (ui.data(|d| d.get_temp::<u8>(id)), pointer)
    {
        app.ui.histogram_range = Some([from, level_at(pos.x)]);
    } else if resp.clicked() {
        app.ui.histogram_range = None;
    }
    pointer.filter(|_| resp.hovered() || resp.dragged()).map(|pos| level_at(pos.x))
}

/// The outline of `h` (the original histogram over an adjusted one).
fn outline(p: &egui::Painter, r: Rect, h: &[u32; 256], color: Color32) {
    let top = tone::histogram_scale_top(h);
    let pts: Vec<_> = h
        .iter()
        .enumerate()
        .map(|(i, v)| pos2(r.left() + (i as f32 + 0.5) / 256.0 * r.width(), r.bottom() - (*v as f32 / top).min(1.0) * r.height()))
        .collect();
    p.add(egui::Shape::line(pts, egui::Stroke::new(1.0, color.gamma_multiply(0.8))));
}

/// The statistics grid: Mean, Std Dev, Median, Pixels | Level, Count, Percentile, Cache Level.
fn statistics(ui: &mut egui::Ui, h: &Histograms, mode: ColorMode, channel: HistChannel, levels: Option<[u8; 2]>, t: &Tokens) {
    let ch = if channel == HistChannel::Colors { HistChannel::Composite } else { channel };
    let Some(s) = stats(bins(h, mode, ch), levels) else { return };
    let level = match s.level {
        Some([a, b]) if a == b => a.to_string(),
        Some([a, b]) => format!("{a}..{b}"),
        None => String::new(),
    };
    let rows = [
        (tl!("Mean:"), format!("{:.2}", s.mean), tl!("Level:"), level),
        (tl!("Std Dev:"), format!("{:.2}", s.std_dev), tl!("Count:"), s.count.map(|c| c.to_string()).unwrap_or_default()),
        (tl!("Median:"), s.median.to_string(), tl!("Percentile:"), s.percentile.map(|p| format!("{p:.2}")).unwrap_or_default()),
        (tl!("Pixels:"), s.pixels.to_string(), tl!("Cache Level:"), h.level.max(1).to_string()),
    ];
    let mono = crate::theme::mono(11.5);
    egui::Grid::new("hist-stats").num_columns(4).spacing(vec2(6.0, 2.0)).show(ui, |ui| {
        for (k1, v1, k2, v2) in rows {
            for (k, v) in [(k1, v1), (k2, v2)] {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(k).color(t.text_dim).size(11.5));
                });
                ui.label(egui::RichText::new(v).font(mono.clone()).color(t.text));
            }
            ui.end_row();
        }
    });
}

/// Histogram panel (Window › Histogram).
pub fn show(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    if app.session.active().is_none() {
        ui.label(egui::RichText::new(tl!("No document")).color(t.text_faint));
        return;
    }
    update(app, ui.ctx());
    let Some(s) = shown(app) else {
        ui.label(egui::RichText::new(tl!("Calculating…")).color(t.text_faint));
        return;
    };
    let opts = app.ui.histogram_panel;
    let channel = resolve_channel(s.mode, opts.channel);
    if opts.view == HistView::Compact {
        // Compact View: the graph alone; a single channel is always in its colour.
        graph(app, ui, &s, channel, true, 100.0);
        return;
    }
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(tl!("Channel:")).color(t.text_dim).size(12.0));
        let mut ch = channel;
        let options = channel_options(s.mode);
        if crate::widgets::dropdown(ui, "histogram-panel-ch", tl!("Channel"), &mut ch, &options, 110.0) {
            app.ui.histogram_panel.channel = ch;
            changed = true;
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if crate::icons::button(ui, "rotate-cw", 20.0, false, tl!("Uncached Refresh")).clicked() {
                uncached_refresh(app);
            }
        });
    });
    let colored = opts.show_channels_in_color;
    let hover = graph(app, ui, &s, channel, colored, 100.0);
    // Source: only for documents with several layers.
    if s.layers > 1 {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(tl!("Source:")).color(t.text_dim).size(12.0));
            let mut src = opts.source;
            let mut options = vec![(HistSourceChoice::EntireImage, "Entire Image"), (HistSourceChoice::SelectedLayer, "Selected Layer")];
            if s.adjustment {
                options.push((HistSourceChoice::AdjustmentComposite, "Adjustment Composite"));
            } else if src == HistSourceChoice::AdjustmentComposite {
                src = HistSourceChoice::EntireImage;
            }
            if crate::widgets::dropdown(ui, "histogram-panel-src", tl!("Source"), &mut src, &options, 150.0) {
                app.ui.histogram_panel.source = src;
                changed = true;
            }
        });
    }
    if opts.show_statistics {
        ui.add_space(4.0);
        let levels = app.ui.histogram_range.or_else(|| hover.map(|l| [l, l]));
        statistics(ui, &s.hist, s.mode, channel, levels, &t);
    }
    if opts.view == HistView::AllChannels {
        for c in color_channels(s.mode) {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(tl!(channel_label(s.mode, *c))).color(t.text).size(11.5));
            let w = ui.available_width();
            let (r, _) = ui.allocate_exact_size(vec2(w, 90.0), Sense::hover());
            let p = ui.painter_at(r);
            p.rect_filled(r, 0.0, t.field);
            tone::draw_histogram(&p, r, bins(&s.hist, s.mode, *c), graph_color(*c, colored, &t));
        }
    }
    if changed {
        persist_options(app);
    }
}

/// The Histogram items of the panel menu.
pub fn panel_menu(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    if ui.add_enabled(app.session.active().is_some(), egui::Button::new(tl!("Uncached Refresh"))).clicked() {
        uncached_refresh(app);
        ui.close();
    }
    ui.separator();
    let mut changed = false;
    for (view, label) in [(HistView::Compact, "Compact View"), (HistView::Expanded, "Expanded View"), (HistView::AllChannels, "All Channels View")] {
        if ui.radio(app.ui.histogram_panel.view == view, tl!(label)).clicked() {
            // The channel is one setting for every view, so it is kept.
            app.ui.histogram_panel.view = view;
            changed = true;
        }
    }
    ui.separator();
    let mut stats_on = app.ui.histogram_panel.show_statistics;
    if crate::widgets::checkbox(ui, &mut stats_on, tl!("Show Statistics")).changed() {
        app.ui.histogram_panel.show_statistics = stats_on;
        changed = true;
    }
    let mut color = app.ui.histogram_panel.show_channels_in_color;
    if crate::widgets::checkbox(ui, &mut color, tl!("Show Channels in Color")).changed() {
        app.ui.histogram_panel.show_channels_in_color = color;
        changed = true;
    }
    if changed {
        persist_options(app);
        ui.close();
    }
}

/// The panel's state for `ui.inspect` (`histogram`): its options as they apply to the active
/// document, and the shown histogram's statistics for the selected range.
pub fn inspect(app: &PhotocraftApp) -> Value {
    let opts = app.ui.histogram_panel;
    let Some(st) = app.session.active() else { return json!({"options": opts}) };
    let mode = st.doc.mode;
    let channel = resolve_channel(mode, opts.channel);
    let (source, _) = resolve_source(st, source_choice(&opts));
    let preview = app.doc_hist_preview.as_ref().filter(|e| e.key.doc == st.doc.id);
    let entry = preview.or(app.doc_hist.as_ref()).filter(|e| e.key.doc == st.doc.id);
    let committed = committed_key(app).map(|(k, _, _)| k);
    let ch = if channel == HistChannel::Colors { HistChannel::Composite } else { channel };
    json!({
        "options": opts,
        "channel": channel_id(channel),
        "channels": channel_options(mode).iter().map(|(c, _)| channel_id(*c)).collect::<Vec<_>>(),
        "source": source,
        "sourceShown": st.doc.layer_count() > 1 && opts.view != HistView::Compact,
        "range": app.ui.histogram_range,
        "preview": preview.is_some(),
        "revision": entry.map(|e| e.key.revision),
        "current": entry.zip(committed).is_some_and(|(e, k)| e.key == HistKey { preview: e.key.preview, ..k }),
        "cacheLevel": entry.map(|e| e.hist.level.max(1)),
        "maxCacheLevel": max_cache_level(app),
        "stats": entry.and_then(|e| stats(bins(&e.hist, mode, ch), app.ui.histogram_range)),
    })
}

#[cfg(test)]
mod tests;
