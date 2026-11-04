//! The Layers panel's Effect filter (#10880): lists the layers with an enabled effect of one kind.
//! Overlay covers Color, Gradient and Pattern Overlay, as in Photoshop.

use photocraft_doc::{Effect, Layer};
use serde::{Deserialize, Serialize};

use crate::{PhotocraftApp, widgets};

/// The effect kinds the Effect filter picks from, in Photoshop's order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EffectKind {
    #[default]
    BevelEmboss,
    Stroke,
    InnerShadow,
    InnerGlow,
    Satin,
    Overlay,
    OuterGlow,
    DropShadow,
}

impl EffectKind {
    pub const ALL: [EffectKind; 8] = [
        EffectKind::BevelEmboss,
        EffectKind::Stroke,
        EffectKind::InnerShadow,
        EffectKind::InnerGlow,
        EffectKind::Satin,
        EffectKind::Overlay,
        EffectKind::OuterGlow,
        EffectKind::DropShadow,
    ];

    /// The id `ui.set {layerFilter: {effect}}` takes.
    pub fn id(self) -> &'static str {
        match self {
            EffectKind::BevelEmboss => "bevelEmboss",
            EffectKind::Stroke => "stroke",
            EffectKind::InnerShadow => "innerShadow",
            EffectKind::InnerGlow => "innerGlow",
            EffectKind::Satin => "satin",
            EffectKind::Overlay => "overlay",
            EffectKind::OuterGlow => "outerGlow",
            EffectKind::DropShadow => "dropShadow",
        }
    }

    fn label(self) -> &'static str {
        match self {
            EffectKind::BevelEmboss => tl!("Bevel & Emboss"),
            EffectKind::Stroke => tl!("Stroke"),
            EffectKind::InnerShadow => tl!("Inner Shadow"),
            EffectKind::InnerGlow => tl!("Inner Glow"),
            EffectKind::Satin => tl!("Satin"),
            EffectKind::Overlay => tl!("Overlay"),
            EffectKind::OuterGlow => tl!("Outer Glow"),
            EffectKind::DropShadow => tl!("Drop Shadow"),
        }
    }

    fn matches(self, e: &Effect) -> bool {
        matches!(
            (self, e),
            (EffectKind::BevelEmboss, Effect::BevelEmboss(_))
                | (EffectKind::Stroke, Effect::Stroke(_))
                | (EffectKind::InnerShadow, Effect::InnerShadow(_))
                | (EffectKind::InnerGlow, Effect::InnerGlow(_))
                | (EffectKind::Satin, Effect::Satin(_))
                | (EffectKind::Overlay, Effect::ColorOverlay { .. } | Effect::GradientOverlay { .. } | Effect::PatternOverlay { .. })
                | (EffectKind::OuterGlow, Effect::OuterGlow(_))
                | (EffectKind::DropShadow, Effect::DropShadow(_))
        )
    }

    /// Whether `l` has an enabled effect of this kind; a turned-off one doesn't count.
    pub fn shows(self, l: &Layer) -> bool {
        l.effects.items.iter().any(|e| e.enabled() && self.matches(e))
    }
}

/// `layerFilter.effect`: the kind named by `v`, or an error listing the ids.
pub fn parse(v: &serde_json::Value) -> Result<EffectKind, String> {
    let ids = || EffectKind::ALL.map(EffectKind::id).join(", ");
    let name = v.as_str().ok_or_else(|| format!("layerFilter.effect must be one of {}", ids()))?;
    EffectKind::ALL.into_iter().find(|k| k.id() == name).ok_or_else(|| format!("layerFilter.effect: unknown effect `{name}` (effects: {})", ids()))
}

/// The Effect filter's controls: the effect kind dropdown.
pub fn controls(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let kinds: Vec<(EffectKind, &str)> = EffectKind::ALL.into_iter().map(|k| (k, k.label())).collect();
    let width = ui.available_width().max(96.0);
    widgets::dropdown(ui, "layer-filter-effect", tl!("Effect"), &mut app.ui.layer_filter_effect, &kinds, width);
}
