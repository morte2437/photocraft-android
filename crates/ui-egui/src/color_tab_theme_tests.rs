//! The Color panel tab is Photoshop's spectrum field in every theme (#4367, #6000): the light
//! themes used to show HSB sliders only, then a short field with the sliders under it.

use egui::{Rect, pos2};
use egui_kittest::kittest::Queryable;

use crate::PhotocraftApp;
use crate::dock::Group;
use crate::theme::{ThemeKind, Tokens};

fn left_top() -> egui::Id {
    egui::Id::unique("color-tab-test-origin")
}

/// The Color tab of `kind`, alone in a 300 pt wide panel, starting from a red foreground. `fill`
/// is the group body height the dock offers, if any.
fn color_tab(kind: ThemeKind, fill: Option<f32>) -> egui_kittest::Harness<'static, PhotocraftApp> {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.tools.foreground = [1.0, 0.0, 0.0, 1.0];
    let tab = Group::Color.tabs(Tokens::for_kind(kind).pro).iter().position(|name| *name == "Color").unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(300.0, 400.0)).build_ui_state(
        move |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                let origin = ui.cursor().min;
                ui.data_mut(|d| d.insert_temp(left_top(), Rect::from_min_size(origin, egui::Vec2::ZERO)));
                if let Some(fill) = fill {
                    ui.data_mut(|d| d.insert_temp(crate::panels::color_field_fill_id(), fill));
                }
                crate::panels::dock_body(app, ui, Group::Color, tab);
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, kind);
    h.run_steps(3);
    h
}

/// Drag on the saturation/brightness field at `y` below its top, on its left edge (no saturation).
fn pick_at(h: &mut egui_kittest::Harness<'static, PhotocraftApp>, y: f32) -> [f32; 4] {
    let origin = h.ctx.data(|d| d.get_temp::<Rect>(left_top())).unwrap().min;
    // After the 38 pt chips and the 8 pt gap.
    let p = pos2(origin.x + 38.0 + 8.0 + 1.0, origin.y + y);
    h.hover_at(p);
    h.run_steps(1);
    h.drag_at(p);
    h.run_steps(1);
    h.drop_at(p);
    h.run_steps(2);
    h.state().session.tools.foreground
}

#[test]
fn every_theme_shows_the_same_spectrum_field() {
    for kind in ThemeKind::ALL {
        let mut h = color_tab(kind, None);
        // Halfway down its 120 pt: no saturation, half brightness.
        let [r, g, b, _] = pick_at(&mut h, 60.0);
        assert!((r - g).abs() < 0.02 && (g - b).abs() < 0.02 && (0.3..0.7).contains(&r), "{kind:?}: the field picks a mid grey, got {r} {g} {b}");
        // No theme swaps the field for sliders or adds them under it (#6000).
        for slider in ["Hue", "Saturation", "Brightness"] {
            assert!(h.query_all_by_label(slider).next().is_none(), "{kind:?}: no {slider} slider");
        }
    }
}

/// Every theme stretches the field over the dock group, as Photoshop's themes always did (#6000).
#[test]
fn every_theme_stretches_the_field_over_the_group() {
    for kind in ThemeKind::ALL {
        let mut h = color_tab(kind, Some(380.0));
        // Far below a 120 pt field: only a stretched one picks here, and a dark grey.
        let [r, g, b, _] = pick_at(&mut h, 300.0);
        assert!((r - g).abs() < 0.02 && (g - b).abs() < 0.02 && r < 0.25, "{kind:?}: the stretched field picks a dark grey, got {r} {g} {b}");
    }
}
