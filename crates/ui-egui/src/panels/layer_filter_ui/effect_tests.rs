//! #10880: the Layers panel's Effect filter, against the cases measured in Photoshop.

use egui::vec2;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use photocraft_doc::Document;
use serde_json::json;

use super::effect::EffectKind;
use super::{Filter, FilterBy};
use crate::PhotocraftApp;
use crate::state::UiState;

/// One pixel layer per effect kind (the three overlays each on their own), "shadow off" whose
/// Drop Shadow is turned off, and "plain" without effects, over the Background.
fn session() -> photocraft_engine::Session {
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 64, "height": 48})).unwrap();
    for (name, kind, params) in [
        ("plain", "", json!({})),
        ("bevel", "bevelEmboss", json!({})),
        ("stroke", "stroke", json!({})),
        ("inner shadow", "innerShadow", json!({})),
        ("inner glow", "innerGlow", json!({})),
        ("satin", "satin", json!({})),
        ("color", "colorOverlay", json!({"color": "#ff8000"})),
        ("gradient", "gradientOverlay", json!({})),
        ("pattern", "patternOverlay", json!({})),
        ("outer glow", "outerGlow", json!({})),
        ("shadow", "dropShadow", json!({})),
        ("shadow off", "dropShadow", json!({"enabled": false})),
    ] {
        s.execute("layer.new.layer", json!({"name": name})).unwrap();
        if !kind.is_empty() {
            s.execute("layer.layerStyle.replace", json!({"effects": [{"kind": kind, "params": params}]})).unwrap();
        }
    }
    s
}

/// The names `filter` lists, top to bottom.
fn listed(doc: &Document, filter: &Filter) -> Vec<String> {
    crate::layer_tree_ui::display_rows(doc, filter.active()).into_iter().filter(|(_, l)| filter.shows(l)).map(|(_, l)| l.name.clone()).collect()
}

fn by_effect(effect: EffectKind) -> Filter {
    Filter::of(&UiState { layer_filter_by: FilterBy::Effect, layer_filter_effect: effect, ..UiState::default() })
}

#[test]
fn each_effect_lists_the_layers_with_it_enabled() {
    let s = session();
    let doc = &s.active().unwrap().doc;
    for (effect, want) in [
        (EffectKind::BevelEmboss, &["bevel"][..]),
        (EffectKind::Stroke, &["stroke"]),
        (EffectKind::InnerShadow, &["inner shadow"]),
        (EffectKind::InnerGlow, &["inner glow"]),
        (EffectKind::Satin, &["satin"]),
        // Overlay covers Color, Gradient and Pattern Overlay.
        (EffectKind::Overlay, &["pattern", "gradient", "color"]),
        (EffectKind::OuterGlow, &["outer glow"]),
        // A turned-off Drop Shadow doesn't count.
        (EffectKind::DropShadow, &["shadow"]),
    ] {
        assert_eq!(listed(doc, &by_effect(effect)), want, "{effect:?}");
    }
}

#[test]
fn effect_filters_at_once_with_bevel_and_emboss() {
    let f = Filter::of(&UiState { layer_filter_by: FilterBy::Effect, ..UiState::default() });
    assert_eq!(f.effect, EffectKind::BevelEmboss);
    assert!(f.active(), "Effect filters as soon as it's picked");
}

#[test]
fn ui_set_picks_the_effect_by_id() {
    let mut ui = UiState::default();
    for effect in EffectKind::ALL {
        let f = super::patched(&ui, &json!({"by": "effect", "effect": effect.id()})).unwrap();
        super::apply(&mut ui, f);
        assert_eq!((ui.layer_filter_by, ui.layer_filter_effect), (FilterBy::Effect, effect));
    }
    let err = super::patched(&ui, &json!({"effect": "colorOverlay"})).unwrap_err();
    assert!(err.contains("colorOverlay") && err.contains("bevelEmboss") && err.contains("dropShadow"), "{err}");
    assert!(super::patched(&ui, &json!({"effect": 3})).is_err());
    assert!(super::patched(&ui, &json!({"effect": null})).is_err());
}

#[test]
fn saved_settings_without_the_effect_still_load() {
    let mut saved = serde_json::to_value(UiState { layer_filter_effect: EffectKind::Satin, ..UiState::default() }).unwrap();
    assert_eq!(saved["layer_filter_effect"], "satin");
    saved.as_object_mut().unwrap().remove("layer_filter_effect");
    let ui: UiState = serde_json::from_value(saved).unwrap();
    assert_eq!(ui.layer_filter_effect, EffectKind::BevelEmboss);
}

/// The names of the rows the Layers panel drew, top to bottom.
fn drawn(h: &Harness<'_, PhotocraftApp>) -> Vec<String> {
    let doc = &h.state().session.active().unwrap().doc;
    let mut rows = crate::layer_row_ui::recorded(&h.ctx);
    rows.sort_by(|a, b| a.row.top().total_cmp(&b.row.top()));
    rows.iter().filter_map(|r| doc.walk().into_iter().find(|(_, _, l)| l.id.0 == r.layer).map(|(_, _, l)| l.name.clone())).collect()
}

#[test]
fn the_panel_filters_by_the_effect_picked_in_the_dropdown() {
    let session = session();
    let mut h = Harness::builder().with_size(vec2(1440.0, 1200.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(session, crate::Services::default())
    });
    h.run_steps(8);
    h.get_by_value("Kind").click();
    h.run_steps(4);
    h.query_all_by_label("Effect").last().expect("Effect in the open list").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_by, FilterBy::Effect);
    assert_eq!(drawn(&h), ["bevel"], "Bevel & Emboss at first");
    h.get_by_value("Bevel & Emboss").click();
    h.run_steps(4);
    h.query_all_by_label("Drop Shadow").last().expect("Drop Shadow in the open list").click();
    h.run_steps(4);
    assert_eq!(h.state().ui.layer_filter_effect, EffectKind::DropShadow);
    assert_eq!(drawn(&h), ["shadow"]);
}
