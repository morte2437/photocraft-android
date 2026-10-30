//! Mixer Brush: a reservoir of loaded paint plus a pickup well that mixes in canvas colour.
//!
//! Model (behavioural, our own): each dab samples the canvas colour under its footprint. The pickup
//! colour `P` moves towards it by `wet`. The deposited colour mixes the reservoir `R` (weight
//! `amount × (1 − mix·wet)`) with the pickup (weight `mix·wet`); it lands at `flow` times the total
//! weight. The reservoir amount drains per dab unless `load` is 100 %, so low Load strokes dry out.
//! A dry brush (`wet = 0`) paints only reservoir colour; an empty, wet brush smears canvas colour.

use photocraft_raster::{Surface, from_rgba, from_rgba_into, to_rgba};
use serde::{Deserialize, Serialize};

use crate::Stroke;
use crate::retouch::{alpha_index, apply_dab_stroke, over_native};
use photocraft_geom::Rect;

/// Fraction of the way the pickup colour moves towards the canvas colour per dab at 100 % Wet.
pub const PICKUP_RATE: f32 = 0.25;

/// Mixer Brush options (all 0..1).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MixerSettings {
    pub wet: f32,
    pub load: f32,
    pub mix: f32,
    pub flow: f32,
    /// Pick up colour from all visible layers instead of the target layer only.
    pub sample_all_layers: bool,
    /// Load the brush with the stroke color after each stroke (the automatic Load option).
    #[serde(alias = "reloadAfterStroke")]
    pub load_after_stroke: bool,
    /// Empty the brush after each stroke (the automatic Clean option).
    pub clean_after_stroke: bool,
}

impl Default for MixerSettings {
    fn default() -> Self {
        Self { wet: 0.5, load: 0.5, mix: 0.5, flow: 1.0, sample_all_layers: false, clean_after_stroke: true, load_after_stroke: true }
    }
}

impl MixerSettings {
    /// Effective per-dab wetness, including Transfer's Wetness control.
    pub fn effective_wet(&self, dab: &crate::Dab) -> f32 {
        self.wet.clamp(0.0, 1.0) * dab.wet.clamp(0.0, 1.0)
    }

    /// Whether this dab can update the pickup well from the canvas.
    pub fn samples_canvas(&self, dab: &crate::Dab) -> bool {
        self.effective_wet(dab) > 0.0
    }
}

/// Paint carried between strokes (session tool state).
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct MixerState {
    /// Loaded paint (straight RGBA), None = clean/unloaded.
    pub reservoir: Option<[f32; 4]>,
    /// Remaining reservoir amount 0..1.
    pub amount: f32,
    /// Colour picked up from the canvas (straight RGBA), None = clean.
    pub pickup: Option<[f32; 4]>,
    /// The foreground colour in effect when the reservoir was loaded from the canvas
    /// (`load_sampled`), None when it holds the foreground itself.
    pub sampled_over: Option<[f32; 4]>,
    /// A bounded 64 x 64 spatial load in tip-local coordinates. Empty cells carry no paint.
    pub spatial: Option<Vec<[f32; 4]>>,
    /// The color recorded by the last stroke (its color) or the last Load Brush or Clean Brush
    /// (the foreground). None = no stroke, load or clean yet.
    pub color: Option<[f32; 4]>,
}

impl MixerState {
    /// Load Brush: fill the reservoir with a color (full).
    pub fn load(&mut self, color: [f32; 4]) {
        self.spatial = None;
        self.reservoir = Some(color);
        self.amount = 1.0;
        self.sampled_over = None;
    }

    /// Load the reservoir with a colour sampled from the canvas (⌥-click) while `foreground` is
    /// the foreground colour; `holds_sample` stays true until the foreground changes.
    pub fn load_sampled(&mut self, color: [f32; 4], foreground: [f32; 4]) {
        self.load(color);
        self.sampled_over = Some(foreground);
        self.color = Some(foreground);
    }
    /// Clean Brush: empty the reservoir and the pickup well.
    pub fn clean(&mut self) {
        self.spatial = None;
        self.reservoir = None;
        self.amount = 0.0;
        self.pickup = None;
        self.sampled_over = None;
    }

    /// Whether the reservoir holds a canvas sample taken while `foreground` was the foreground.
    pub fn holds_sample(&self, foreground: [f32; 4]) -> bool {
        self.reservoir.is_some() && self.sampled_over == Some(foreground)
    }
    /// Before a stroke in `color`: choosing a color other than [`MixerState::color`] loads the
    /// reservoir with it.
    pub fn begin_stroke(&mut self, color: [f32; 4]) {
        if self.color != Some(color) {
            self.load(color);
            self.color = Some(color);
        }
    }
    /// After a stroke in `color`: the automatic Clean empties the brush, then the automatic Load
    /// fills the reservoir again.
    pub fn end_stroke(&mut self, color: [f32; 4], clean: bool, load: bool) {
        // Automatic Load replenishes a sampled tip as well as a solid-color load.
        // Move its bounded buffer through Clean instead of allocating a copy.
        let sampled = self.holds_sample(color).then_some(color);
        let spatial = if load { self.spatial.take() } else { None };
        let paint = if spatial.is_some() || sampled.is_some() { self.reservoir.unwrap_or(color) } else { color };
        if clean {
            self.clean();
        }
        if load {
            self.load(paint);
            self.spatial = spatial;
            self.sampled_over = sampled;
        }
        self.color = Some(color);
    }
    /// The reservoir color the next stroke in `color` starts with (the Current Brush Load
    /// swatch); None = a clean brush.
    pub fn next_load(&self, color: [f32; 4]) -> Option<[f32; 4]> {
        if self.color != Some(color) { Some(color) } else { self.reservoir }
    }
}

/// Sample a spatial load at a dab pixel using the dab's rotation, aspect and flips.
/// Keeping the reservoir in tip coordinates lets it follow subsequent brush poses.
pub fn load_at(load: &[[f32; 4]], dab: &crate::Dab, x: f32, y: f32) -> [f32; 4] {
    let mut dx = x - dab.center.x as f32;
    let mut dy = y - dab.center.y as f32;
    if dab.proj_scale < 0.999 {
        let (sin, cos) = dab.proj_angle.sin_cos();
        let along = (dx * cos - dy * sin) * (1.0 / dab.proj_scale.max(0.05) - 1.0);
        dx += along * cos;
        dy -= along * sin;
    }
    let (sin, cos) = dab.angle.sin_cos();
    let mut u = (dx * cos - dy * sin) / dab.radius.max(0.01);
    let mut v = (dx * sin + dy * cos) / (dab.radius * dab.roundness).max(0.5);
    if dab.flip_x {
        u = -u;
    }
    if dab.flip_y {
        v = -v;
    }
    if !(-1.0..1.0).contains(&u) || !(-1.0..1.0).contains(&v) {
        return [0.0; 4];
    }
    let col = ((u + 1.0) * 32.0) as usize;
    let row = ((v + 1.0) * 32.0) as usize;
    load.get(row * 64 + col).copied().unwrap_or([0.0; 4])
}

/// Paint a Mixer Brush stroke. `sample` is the surface colour is picked up from (the layer itself
/// when None; a flattened composite for Sample All Layers). Returns the damaged rectangle.
pub fn apply_mixer_stroke(
    target: &mut Surface,
    sample: Option<&Surface>,
    stroke: &Stroke,
    m: &MixerSettings,
    state: &mut MixerState,
    selection: Option<&Surface>,
    lock_transparency: bool,
) -> Rect {
    let fmt = target.format();
    let mut st = stroke.clone();
    st.brush.opacity = 1.0;
    apply_dab_stroke(target, &st, selection, lock_transparency, 0, |work, fp| {
        apply_mixer_dab(work, fp, fmt, sample, m, state, lock_transparency);
    })
}

/// One incremental wet-paint dab, shared by recorded strokes and the live document transaction.
pub fn apply_mixer_dab(
    work: &mut crate::retouch::Region,
    fp: &crate::retouch::Footprint,
    fmt: photocraft_color::PixelFormat,
    sample: Option<&Surface>,
    m: &MixerSettings,
    state: &mut MixerState,
    lock_transparency: bool,
) {
    let sample = sample.filter(|_| m.samples_canvas(&fp.dab)).map(|surface| {
        let source = crate::retouch::Region::read(surface, work.rect);
        if surface.format().mode == fmt.mode && surface.format().alpha == fmt.alpha {
            source
        } else {
            let mut native = crate::retouch::Region::new(work.rect, fmt.channels());
            for (src, dst) in source.data.chunks_exact(source.ch).zip(native.data.chunks_exact_mut(native.ch)) {
                from_rgba_into(&fmt, to_rgba(&surface.format(), src), dst);
            }
            native
        }
    });
    apply_mixer_dab_region(work, fp, fmt, sample.as_ref(), m, state, lock_transparency);
}

/// Contiguous native-channel sample variant. `sample` must have the same rectangle and
/// channel layout as `work`. The live compositor supplies it directly, avoiding a tiled
/// temporary surface and a heap allocation for every sampled/deposited pixel.
pub fn apply_mixer_dab_region(
    work: &mut crate::retouch::Region,
    fp: &crate::retouch::Footprint,
    fmt: photocraft_color::PixelFormat,
    sample: Option<&crate::retouch::Region>,
    m: &MixerSettings,
    state: &mut MixerState,
    lock_transparency: bool,
) {
    let n = fmt.channels();
    let a = alpha_index(&fmt);
    let nc = fmt.mode.color_channels();
    let native = |c: [f32; 4]| {
        let mut v = from_rgba(&fmt, c);
        if let Some(ai) = a {
            v[ai] = 1.0;
        }
        v
    };
    let res = state.reservoir.map(native);
    let mut amount = if res.is_some() { state.amount.clamp(0.0, 1.0) } else { 0.0 };
    let mut pickup: Option<Vec<f32>> = state.pickup.map(native);
    let (mix, flow, load) = (m.mix.clamp(0.0, 1.0), m.flow.clamp(0.0, 1.0), m.load.clamp(0.0, 1.0));
    let mut paint = vec![0.0f32; n];
    // Transfer › Wetness / Mix Jitter scale the settings per dab.
    let (wet, mix) = (m.effective_wet(&fp.dab), mix * fp.dab.mix.clamp(0.0, 1.0));
    // Sample the canvas under the dab.
    let r = fp.rect.intersect(&work.rect);
    if wet > 0.0 {
        let mut acc = vec![0.0f32; n];
        let (mut wsum, mut asum) = (0.0f32, 0.0f32);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let w = fp.at(x, y);
                if w <= 0.0 {
                    continue;
                }
                let px = sample.unwrap_or(work).px(x, y);
                let al = a.map_or(1.0, |ai| px[ai]);
                for c in 0..nc {
                    acc[c] += px[c] * w * al;
                }
                wsum += w;
                asum += w * al;
            }
        }
        if asum > 1e-6 {
            let ca = asum / wsum.max(1e-6);
            let canvas: Vec<f32> = (0..n).map(|c| if c < nc { acc[c] / asum } else { 1.0 }).collect();
            pickup = Some(match pickup.take() {
                None => canvas,
                Some(p) => p.iter().zip(&canvas).map(|(p, c)| p + (c - p) * PICKUP_RATE * wet * ca).collect(),
            });
        }
    }
    let wr = if res.is_some() { amount * (1.0 - mix * wet) } else { 0.0 };
    let wp = if pickup.is_some() { mix * wet } else { 0.0 };
    let total = wr + wp;
    if total > 1e-6 {
        for c in 0..n {
            let rv = res.as_ref().map_or(0.0, |v| v[c]);
            let pv = pickup.as_ref().map_or(0.0, |v| v[c]);
            paint[c] = (rv * wr + pv * wp) / total;
        }
        let dep = total.min(1.0) * flow;
        let mut local = vec![0.0; n];
        let mut mixed = vec![0.0; n];
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                let k = fp.at(x, y) * dep;
                if k > 0.0 {
                    if let Some(load) = &state.spatial {
                        let loaded = load_at(load, &fp.dab, x as f32 + 0.5, y as f32 + 0.5);
                        from_rgba_into(&fmt, loaded, &mut local);
                        let local_wr = wr * loaded[3];
                        let local_total = local_wr + wp;
                        if local_total > 1e-6 {
                            for (c, value) in mixed.iter_mut().enumerate() {
                                *value = (local.get(c).copied().unwrap_or(0.0) * local_wr
                                    + pickup.as_ref().and_then(|p| p.get(c)).copied().unwrap_or(0.0) * wp)
                                    / local_total;
                            }
                            if let Some(ai) = a
                                && let Some(alpha) = mixed.get_mut(ai)
                            {
                                *alpha = 1.0;
                            }
                            over_native(&fmt, work.px_mut(x, y), &mixed, fp.at(x, y) * local_total.min(1.0) * flow, lock_transparency);
                        }
                    } else {
                        over_native(&fmt, work.px_mut(x, y), &paint, k, lock_transparency);
                    }
                }
            }
        }
    }
    amount *= 1.0 - 0.08 * (1.0 - load);
    state.amount = amount;
    state.pickup = pickup.map(|p| to_rgba(&fmt, &p));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BrushSettings, StrokePoint};
    use photocraft_color::PixelFormat;

    fn stroke(y: f64) -> Stroke {
        Stroke {
            brush: BrushSettings { size: 8.0, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
            points: vec![StrokePoint::new(4.0, y, 1.0), StrokePoint::new(196.0, y, 1.0)],
        }
    }

    #[test]
    fn low_load_runs_dry_and_full_load_does_not() {
        for (load, expect_dry) in [(0.0, true), (1.0, false)] {
            let mut s = Surface::new(PixelFormat::RGBA8);
            let mut st = MixerState::default();
            st.load([1.0, 0.0, 0.0, 1.0]);
            apply_mixer_stroke(&mut s, None, &stroke(10.0), &MixerSettings { wet: 0.0, load, mix: 0.0, flow: 1.0, ..Default::default() }, &mut st, None, false);
            let (start, end) = (s.rgba(10, 10)[3], s.rgba(190, 10)[3]);
            assert!(start > 0.95, "{start}");
            if expect_dry {
                assert!(end < start * 0.6, "load {load}: {start} → {end}");
                assert!(st.amount < 0.2);
            } else {
                assert!(end > 0.95, "{end}");
                assert!((st.amount - 1.0).abs() < 1e-6);
            }
        }
    }

    #[test]
    fn wet_brush_picks_up_and_smears_canvas_colour() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        // Blue canvas on the left half, white on the right.
        s.fill_rect(Rect::new(0, 0, 100, 20), &[0.0, 0.0, 1.0, 1.0]);
        s.fill_rect(Rect::new(100, 0, 200, 20), &[1.0, 1.0, 1.0, 1.0]);
        let mut st = MixerState::default();
        st.load([1.0, 0.0, 0.0, 1.0]);
        apply_mixer_stroke(
            &mut s,
            None,
            &stroke(10.0),
            &MixerSettings { wet: 1.0, load: 0.3, mix: 0.9, flow: 1.0, ..Default::default() },
            &mut st,
            None,
            false,
        );
        // Blue is dragged into the white half.
        let c = s.rgba(115, 10);
        assert!(c[2] > c[1] + 0.05 && c[1] < 0.95, "{c:?}");
        // The pickup well now holds a canvas-contaminated colour.
        let p = st.pickup.unwrap();
        assert!(p[2] > 0.2, "{p:?}");
        st.clean();
        assert!(st.pickup.is_none());
    }

    #[test]
    fn a_clean_brush_paints_only_the_canvas_color_it_picks_up() {
        let mut st = MixerState::default();
        st.load([0.0, 0.0, 1.0, 1.0]);
        st.pickup = Some([0.0, 1.0, 0.0, 1.0]);
        st.clean();
        assert_eq!((st.reservoir, st.amount, st.pickup), (None, 0.0, None));
        // On red, the clean wet brush smears red and lays down none of the blue it was loaded with.
        let mut s = Surface::new(PixelFormat::RGBA8);
        s.fill_rect(Rect::new(0, 0, 200, 20), &[1.0, 0.0, 0.0, 1.0]);
        apply_mixer_stroke(&mut s, None, &stroke(10.0), &MixerSettings::default(), &mut st, None, false);
        let c = s.rgba(100, 10);
        assert!(c[0] > 0.99 && c[1] < 0.01 && c[2] < 0.01, "{c:?}");
    }

    #[test]
    fn a_new_color_loads_the_brush_and_the_automatic_options_run_after_the_stroke() {
        let (blue, green) = ([0.0, 0.0, 1.0, 1.0], [0.0, 1.0, 0.0, 1.0]);
        let mut st = MixerState::default();
        // A brush never loaded takes the stroke's color.
        assert_eq!(st.next_load(blue), Some(blue));
        st.begin_stroke(blue);
        assert_eq!((st.reservoir, st.amount), (Some(blue), 1.0));
        // Clean on, Load off: the next stroke in the same color starts empty.
        st.end_stroke(blue, true, false);
        assert_eq!(st.next_load(blue), None);
        st.begin_stroke(blue);
        assert_eq!((st.reservoir, st.pickup), (None, None));
        // Choosing another color loads it.
        assert_eq!(st.next_load(green), Some(green));
        st.begin_stroke(green);
        assert_eq!((st.reservoir, st.amount), (Some(green), 1.0));
        // Load on, Clean off: the reservoir is full again and the pickup stays.
        st.amount = 0.1;
        st.pickup = Some(blue);
        st.end_stroke(green, false, true);
        assert_eq!((st.reservoir, st.amount, st.pickup), (Some(green), 1.0, Some(blue)));
        // Both on: a fresh load with no pickup.
        st.end_stroke(green, true, true);
        assert_eq!((st.reservoir, st.amount, st.pickup), (Some(green), 1.0, None));
    }

    /// Spatial paint reservoirs honor independent automatic clean and reload choices.
    #[test]
    fn spatial_load_obeys_automatic_clean_and_load() {
        let foreground = [0.0, 0.0, 1.0, 1.0];
        let sampled = [1.0, 0.0, 0.0, 1.0];
        let tip = vec![sampled; 4096];
        let mut state = MixerState::default();
        state.load(sampled);
        state.spatial = Some(tip.clone());
        state.color = Some(foreground);
        state.amount = 0.1;
        state.end_stroke(foreground, true, true);
        assert_eq!(state.spatial.as_ref(), Some(&tip));
        assert_eq!((state.reservoir, state.amount, state.pickup), (Some(sampled), 1.0, None));
        state.begin_stroke(foreground);
        assert_eq!(state.spatial.as_ref(), Some(&tip));
        state.end_stroke(foreground, true, false);
        assert!(state.spatial.is_none() && state.reservoir.is_none());
        state.begin_stroke(foreground);
        assert!(state.reservoir.is_none(), "Clean stays empty until a new color or Load");
    }

    /// Older serialized reload settings remain compatible with the automatic load field.
    #[test]
    fn earlier_reload_setting_deserializes_as_automatic_load() {
        let settings: MixerSettings = serde_json::from_value(serde_json::json!({"reloadAfterStroke":false})).unwrap();
        assert!(!settings.load_after_stroke);
        let saved = serde_json::to_value(settings).unwrap();
        assert_eq!(saved["loadAfterStroke"], false);
        assert!(saved.get("reloadAfterStroke").is_none());
    }

    #[test]
    fn dry_empty_brush_paints_nothing() {
        let mut s = Surface::new(PixelFormat::RGBA8);
        let mut st = MixerState::default();
        let dmg = apply_mixer_stroke(&mut s, None, &stroke(10.0), &MixerSettings { wet: 0.0, ..Default::default() }, &mut st, None, false);
        assert!(!dmg.is_empty());
        assert_eq!(s.rgba(50, 10)[3], 0.0);
    }

    #[test]
    fn transfer_wetness_control_scales_pickup_per_dab() {
        // Wetness on Pen Pressure: no pressure means a dry brush (no pickup), full pressure picks up.
        let run = |pressure: f32| {
            let mut s = Surface::new(PixelFormat::RGBA8);
            s.fill_rect(Rect::new(0, 0, 200, 20), &[0.0, 0.0, 1.0, 1.0]);
            let mut st = MixerState::default();
            st.load([1.0, 0.0, 0.0, 1.0]);
            let mut k = stroke(10.0);
            k.brush.transfer = crate::Transfer { enabled: true, wetness: crate::Dynamic::controlled(crate::Control::PenPressure), ..Default::default() };
            for p in &mut k.points {
                p.pressure = pressure;
            }
            apply_mixer_stroke(&mut s, None, &k, &MixerSettings { wet: 1.0, load: 1.0, mix: 1.0, flow: 1.0, ..Default::default() }, &mut st, None, false);
            st.pickup
        };
        assert!(run(0.0).is_none());
        assert!(run(1.0).is_some_and(|p| p[2] > 0.2));
    }
}

#[cfg(test)]
mod reference_tests;
