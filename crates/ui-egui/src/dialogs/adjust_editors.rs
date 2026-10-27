//! One editor per adjustment kind, shared by the adjustment-layer Properties panel and the modal
//! Image › Adjustments dialogs (`adjust_dialog`).
//!
//! Every editor edits the kind's complete parameter set (`photocraft_engine::adjust_params`) as a
//! JSON object and reports an [`Edit`]: `changed` while values move (the host previews them live)
//! and `commit` when a gesture ends (the Properties host then runs one `layer.setAdjustment`; a
//! dialog commits on OK). Selective Color and Color Lookup keep their editors in `adjust_ui`.

use crate::point_curve as curve_edit;
use crate::state::CurvesEditorState as CurveUi;
use std::sync::Arc;

use egui::{Color32, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_doc::adjust::{HueRange, ToneSpace};
use photocraft_doc::{Adjustment, LayerId};
use photocraft_engine::adjust_params::{self, HUE_RANGES, PHOTO_FILTERS};
use photocraft_engine::presets::Group;
use photocraft_engine::presets::gradients::GradientPreset;
use serde_json::{Value, json};

use crate::PhotocraftApp;
use crate::theme::Tokens;
use crate::tone::{self, HistSource, Histograms};
use crate::widgets::{self, hue_color};

mod basic;
mod curves;
mod gradient_map;
mod hue_saturation;
mod levels;
mod session;

#[cfg(test)]
pub(crate) use basic::row_value;
#[cfg(test)]
use basic::track_stops;
use basic::{black_white, brightness_contrast, channel_mixer, color_balance, photo_filter, sliders, threshold};
pub use basic::{reset_adjustment_editor, reset_color_balance};
use curves::curves;
#[cfg(test)]
use curves::{CURVE_HANDLE_SIZE, curve_graph};
pub use curves::{ToneChannel, tone_channels};
pub(crate) use curves::{curve_side_max, read_curve};
use gradient_map::gradient_map;
pub use gradient_map::{gradient_presets, is_gray, swatches};
use hue_saturation::hue_saturation;
#[cfg(test)]
use hue_saturation::{lightness_track, move_hue_range_handle, saturation_track};
use levels::levels;
pub use levels::{auto_levels, gamma_from_pos, gamma_pos};
pub use session::{layer_editor, layer_mem, properties_coalesce, properties_reset, session_start};

/// Kinds edited here (Selective Color and Color Lookup have their own editors; Invert has none).
pub const KINDS: [&str; 13] = [
    "brightnessContrast",
    "levels",
    "curves",
    "exposure",
    "vibrance",
    "hueSaturation",
    "colorBalance",
    "blackWhite",
    "photoFilter",
    "channelMixer",
    "posterize",
    "threshold",
    "gradientMap",
];

pub fn has_editor(kind: &str) -> bool {
    KINDS.contains(&kind)
}

/// Kinds that draw a histogram behind their controls.
pub fn needs_histogram(kind: &str) -> bool {
    matches!(kind, "levels" | "curves" | "threshold")
}

/// What an editor did this frame.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Edit {
    /// Values changed (preview them).
    pub changed: bool,
    /// A gesture ended or a discrete control changed (commit them).
    pub commit: bool,
    /// Open the host's gradient-file dialog without changing adjustment parameters.
    pub import_gradients: bool,
}

impl Edit {
    fn add(&mut self, o: Edit) {
        self.changed |= o.changed;
        self.commit |= o.commit;
        self.import_gradients |= o.import_gradients;
    }
    fn discrete(changed: bool) -> Self {
        Edit { changed, commit: changed, ..Default::default() }
    }
    fn of(r: &egui::Response) -> Self {
        Edit { changed: r.changed(), commit: r.drag_stopped() || (r.changed() && !r.dragged()), ..Default::default() }
    }
}

/// What an editor needs from its host.
pub struct EditorCx {
    /// Root of the editor's view state in egui memory (selected channel, point, range…).
    pub mem: egui::Id,
    /// Histograms in the adjustment's space ([`needs_histogram`] kinds).
    pub hist: Option<Arc<Histograms>>,
    /// Grayscale (or Duotone/Bitmap) document: one Gray channel.
    pub gray: bool,
    /// Current foreground and background colours (Gradient Map preset).
    pub swatches: [[f32; 3]; 2],
    /// The session's gradient preset library (Gradient Map's picker; empty for other kinds).
    pub gradients: Vec<Group<GradientPreset>>,
    /// Hosted by an Image › Adjustments dialog (false: the Properties panel). Photoshop's dialog
    /// sliders follow the mouse wheel; the panel's don't.
    pub dialog: bool,
    /// Preferences ▸ Interface ▸ Dynamic Color Sliders: colour tracks on the colour sliders
    /// (off: the plain track).
    pub dynamic_color_sliders: bool,
    /// Image › Curves with a single Channels-panel target: that channel only (#5335).
    pub isolated_curve: Option<photocraft_engine::channel_cmds::IsolatedCurveChannel>,
}

/// The Levels/Curves channel space the values address.
pub fn space_of(v: &Value) -> ToneSpace {
    if v.get("cyan").is_some() || v.get("black").is_some() {
        ToneSpace::Cmyk
    } else if v.get("a").is_some() {
        ToneSpace::Lab
    } else {
        ToneSpace::Rgb
    }
}

/// The editor for `kind` over its parameter set `v`.
pub fn editor(ui: &mut egui::Ui, kind: &str, v: &mut Value, cx: &EditorCx) -> Edit {
    if !v.is_object() {
        *v = json!({});
    }
    match kind {
        "brightnessContrast" => brightness_contrast(ui, v),
        "levels" => levels(ui, v, cx),
        "curves" => curves(ui, v, cx),
        "exposure" => sliders(
            ui,
            v,
            &[
                ("exposure", tl!("Exposure"), -20.0, 20.0, 0.0, "", Some(2)),
                ("offset", tl!("Offset"), -0.5, 0.5, 0.0, "", Some(4)),
                ("gamma", tl!("Gamma Correction"), 0.01, 9.99, 1.0, "", None),
            ],
        ),
        "vibrance" => sliders(ui, v, &[("vibrance", "Vibrance", -100.0, 100.0, 0.0, "%", None), ("saturation", "Saturation", -100.0, 100.0, 0.0, "%", None)]),
        "hueSaturation" => hue_saturation(ui, v, cx),
        "colorBalance" => color_balance(ui, v, cx),
        "blackWhite" => black_white(ui, v, cx),
        "photoFilter" => photo_filter(ui, v, cx),
        "channelMixer" => channel_mixer(ui, v, cx),
        "posterize" => sliders(ui, v, &[("levels", "Levels", 2.0, 255.0, 4.0, "", None)]),
        "threshold" => threshold(ui, v, cx),
        "gradientMap" => gradient_map(ui, v, cx),
        _ => Edit::default(),
    }
}

// ---------------------------------------------------------------------------------------------
// JSON helpers

fn num(v: &Value, key: &str, default: f32) -> f32 {
    v.get(key).and_then(Value::as_f64).map_or(default, |x| x as f32)
}

fn flag(v: &Value, key: &str, default: bool) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(default)
}

fn nums<const N: usize>(v: &Value, key: &str, default: [f32; N]) -> [f32; N] {
    let a = v.get(key).and_then(Value::as_array);
    std::array::from_fn(|i| a.and_then(|a| a.get(i)).and_then(Value::as_f64).map_or(default[i], |x| x as f32))
}

fn round(x: f32, step: f32) -> f32 {
    (x / step).round() * step
}

fn rgb_of(v: &Value, key: &str, default: [f32; 3]) -> [f32; 3] {
    match v.get(key) {
        Some(Value::String(s)) => {
            let h = s.trim_start_matches('#');
            let b = |i: usize| h.get(i..i + 2).and_then(|x| u8::from_str_radix(x, 16).ok()).map(|x| f32::from(x) / 255.0);
            match (b(0), b(2), b(4)) {
                (Some(r), Some(g), Some(bb)) => [r, g, bb],
                _ => default,
            }
        }
        Some(Value::Array(a)) => std::array::from_fn(|i| a.get(i).and_then(Value::as_f64).map_or(default[i], |x| x as f32)),
        _ => default,
    }
}

fn color32(c: [f32; 3]) -> Color32 {
    let b = |x: f32| (x.clamp(0.0, 1.0) * 255.0).round() as u8;
    Color32::from_rgb(b(c[0]), b(c[1]), b(c[2]))
}

/// A colour swatch button (egui's picker); returns the edit.
fn color_button(ui: &mut egui::Ui, c: &mut [f32; 3]) -> Edit {
    let mut srgb = [c[0], c[1], c[2]].map(|x| (x.clamp(0.0, 1.0) * 255.0).round() as u8);
    let r = crate::widgets::color_edit_button_srgb(ui, &mut srgb);
    if r.changed() {
        *c = srgb.map(|x| f32::from(x) / 255.0);
    }
    // The popup edits continuously; each change is a complete value.
    Edit::discrete(r.changed())
}

fn label(ui: &mut egui::Ui, text: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(tl!(&text)).color(t.text_dim).size(12.0));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saturation_track_follows_the_hue() {
        // The Saturation slider track recolours with the hue (UI-217-16).
        assert_eq!(saturation_track(0.0), [Color32::from_gray(128), hue_color(0.0)]);
        assert_ne!(saturation_track(0.0)[1], saturation_track(120.0)[1]);
        assert_eq!(saturation_track(720.0)[1], saturation_track(0.0)[1], "hue wraps");
    }

    #[test]
    fn lightness_track_is_black_grey_white_whatever_the_hue() {
        // Photoshop's Lightness track does not take the hue's colour (#6539).
        assert_eq!(lightness_track(), [Color32::BLACK, Color32::from_gray(128), Color32::WHITE]);
        let [_, mid, _] = lightness_track();
        assert!(mid.r() == mid.g() && mid.g() == mid.b(), "the middle stop is neutral grey");
    }

    /// Preferences ▸ Interface ▸ Dynamic Color Sliders: off, the colour sliders get plain tracks.
    #[test]
    fn dynamic_color_sliders_switch_the_tracks_off() {
        let cx = |dynamic: bool| EditorCx {
            mem: egui::Id::NULL,
            hist: None,
            gray: false,
            swatches: [[0.0; 3]; 2],
            gradients: Vec::new(),
            dialog: false,
            dynamic_color_sliders: dynamic,
            isolated_curve: None,
        };
        let stops = [Color32::BLACK, Color32::RED];
        assert_eq!(track_stops(&cx(true), &stops), Some(&stops[..]));
        assert_eq!(track_stops(&cx(false), &stops), None);
    }

    #[test]
    fn hue_range_drag_canonicalizes_bounds_before_clamping() {
        let moved = move_hue_range_handle([0.0, 350.0, 100.0, 200.0], 2, 354.0);
        assert_eq!(moved, [-360.0, -10.0, -6.0, -1.0]);

        let canonical = move_hue_range_handle([0.0, 30.0, 60.0, 90.0], 2, 65.0);
        assert_eq!(canonical, [0.0, 30.0, 65.0, 90.0]);
    }

    #[test]
    fn curve_endpoint_handles_fit_inside_the_allocated_area() {
        for side in [120.0, 200.0, 300.0] {
            let full = Rect::from_min_size(pos2(100.0, 50.0), vec2(side, side + 14.0));
            let graph = curve_graph(full, side);
            let to_scr = |q: [f32; 2]| pos2(graph.left() + q[0] / 255.0 * graph.width(), graph.bottom() - q[1] / 255.0 * graph.height());
            for q in [[0.0, 0.0], [255.0, 255.0]] {
                let r = Rect::from_center_size(to_scr(q), vec2(CURVE_HANDLE_SIZE, CURVE_HANDLE_SIZE)).expand(1.0);
                assert!(r.left() >= full.left() && r.right() <= full.right());
                assert!(r.top() >= full.top() && r.bottom() <= full.bottom());
            }
        }
    }

    #[test]
    fn levels_helpers() {
        for g in [0.3f32, 1.0, 1.8, 4.0] {
            let x = gamma_pos(20.0, 230.0, g);
            assert!((gamma_from_pos(20.0, 230.0, x) - g).abs() < 1e-3, "{g}");
        }
        // Moving the grey slider left brightens (gamma > 1), as in Photoshop.
        assert!(gamma_from_pos(0.0, 255.0, 80.0) > 1.0);
        let mut h = [0u32; 256];
        h[10] = 1000;
        h[200] = 1000;
        assert_eq!(auto_levels(&h), (10.0, 200.0));
        assert_eq!(auto_levels(&[0; 256]), (253.0, 255.0), "an empty histogram is no panic");
    }
}

#[cfg(test)]
mod ui_tests;
