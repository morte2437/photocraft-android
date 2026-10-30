//! Chrome around the canvas: title bar, options bar, toolbar, status bar, dock cards, Properties.

use egui::{Align2, Color32, CornerRadius, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, pos2, vec2};
use photocraft_color::BlendMode;
use photocraft_doc::{Layer, LayerContent, LayerId};
use photocraft_engine::history_source_cmds::HistorySource;
use serde_json::{Value, json};

use crate::mask_thumbs_ui::MaskKind;
use crate::state::Tool;
use crate::theme::{self, Tokens};
use crate::{PhotocraftApp, icons, widgets};

pub mod actions;
pub mod adjust_ui;
pub mod apply_color;
pub(crate) mod blend_preview;
pub(super) mod brand;
mod brush_controls;
pub mod brush_panel;
pub mod brush_preview;
pub mod brush_sections;
pub mod brushes_tab;
pub mod channels_panel;
pub mod chrome_ui;
pub mod clip_line_ui;
mod color;
pub mod color_panel_ui;
pub mod comps_ui;
pub mod doc_props_ui;
mod dock;
pub mod histogram_panel;
mod history;
pub mod history_menu_ui;
mod info;
mod layer_drag;
pub mod layer_filter_ui;
pub mod layer_menu_ui;
pub mod layer_props_ui;
pub(super) mod layer_reveal;
mod layer_row;
pub mod layer_row_ui;
pub(super) mod layer_transfer;
pub mod layer_tree_ui;
mod layers;
pub mod layers_panel_ui;
pub(super) mod mask_props_ui;
pub mod mask_thumbs_ui;
mod navigator;
mod options_bar;
pub(super) mod panel_docking;
mod panel_state;
pub mod preset_panels;
mod properties;
pub mod props_layout;
pub mod smart_ui;
mod status_bar;
pub mod swatches_ui;
pub(super) mod timeline_ui;
mod title_bar;
pub(super) mod titlebar;
mod toolbar;
pub mod type_panels_ui;
pub(super) mod variables_ui;

#[cfg(test)]
use color::color_field_height;
pub(crate) use color::{color_field, color_field_fill_id, field_chips, field_hsva, offer_color_field_fill, srgb_hsva};
#[cfg(test)]
pub(crate) use color::{hsva_color32, hsva_srgb, srgb_bytes};
#[cfg(test)]
pub(crate) use dock::DOCK_MIN_WIDTH;
pub(crate) use dock::{canvas_panel, dock_body, dock_width_id, dock_width_range};
pub use dock::{dock_area, request_dock_width};
#[cfg(test)]
use history::history;
#[cfg(test)]
pub(crate) use history::select_snapshot;
pub(crate) use history::selected_snapshot;
#[cfg(test)]
use info::info_panel;
pub(crate) use info::{SamplerReadouts, sampler_readouts};
#[cfg(test)]
use layer_drag::{background_drop_refused, layer_drop_payload};
pub(crate) use layer_drag::{dropped_on, layer_drag_edge_scroll};
#[cfg(test)]
pub(crate) use layer_row::thumb_load_badge;
#[cfg(test)]
use layer_row::{layer_thumb_fit, paint_clipping_indicator};
#[cfg(test)]
use layers::{layers, opacity_row_layout, simple_lock_toggle};
#[cfg(test)]
use navigator::{NAV_ROW_H, navigator, navigator_zoom_drag_id, navigator_zoom_to};
#[cfg(test)]
pub(crate) use navigator::{last_navigator_controls, last_navigator_preview};
#[cfg(test)]
use options_bar::LAYER_PCT_W;
pub use options_bar::options_bar;
#[cfg(test)]
use options_bar::options_bar_height;
pub(crate) use options_bar::{FIXED_SIZE_MAX_PX, opt_label};
pub use panel_state::{migrate_panel_state, panel_state_id, remember_panel_state};
pub use properties::properties_window;
#[cfg(test)]
use properties::{adjustments_grid, properties_body};
pub use status_bar::{status_bar, status_bar_heights};
#[cfg(test)]
use title_bar::title_x;
pub use title_bar::{sync_window_title, system_title_bar, title_bar, window_title};
#[cfg(test)]
use toolbar::{STUDIO_CHIP, TOOL_SECTIONS, chips_height, color_chips, tool_flyout_drag_open, tool_flyout_release, toolbar_icon_size, visible_sections};
pub use toolbar::{paint_toolbar_flyout_indicator, toolbar};

fn c32(c: [f32; 4]) -> Color32 {
    Color32::from_rgba_unmultiplied((c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8)
}

#[cfg(test)]
mod title_tests {
    use super::{system_title_bar, title_x, window_title};

    #[test]
    fn title_never_overlaps_menus_or_controls() {
        // Wide window: centred.
        assert_eq!(title_x(800.0, 420.0, 1300.0, 60.0), Some(770.0));
        // Centre would hit the menus: slid right into the gap.
        assert_eq!(title_x(400.0, 420.0, 1300.0, 60.0), Some(436.0));
        // Centre would hit the controls: slid left.
        assert_eq!(title_x(1280.0, 420.0, 1300.0, 60.0), Some(1224.0));
        // No room for the whole title: the caller elides or drops it.
        assert_eq!(title_x(400.0, 420.0, 480.0, 60.0), None);
        assert_eq!(title_x(f32::NAN, 420.0, 1300.0, 60.0), Some(436.0));
        assert_eq!(title_x(400.0, f32::INFINITY, 1300.0, 60.0), None);
    }

    fn app(custom_titlebar: bool) -> crate::PhotocraftApp {
        let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.custom_titlebar = custom_titlebar;
        // The preference that makes the shell launch without its own title bar.
        app.session.edit_prefs(|p| p.interface.system_title_bar = !custom_titlebar);
        app
    }

    #[test]
    fn without_the_preference_the_mark_stays() {
        // The default (e.g. the web build or the snapshot example): no custom bar, no preference.
        let app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        assert!(!system_title_bar(&app));
    }

    fn open_named(app: &mut crate::PhotocraftApp, name: &str) {
        app.session.add_document(
            photocraft_doc::Document::new(name, photocraft_doc::Size::new(4, 4), photocraft_doc::ColorMode::Rgb, photocraft_doc::SampleType::U8),
            None,
        );
    }

    #[test]
    fn window_title_with_no_document_is_the_app_name() {
        assert_eq!(window_title(&app(true)), "PhotoCraft");
    }

    #[test]
    fn window_title_follows_the_active_file() {
        let mut app = app(true);
        open_named(&mut app, "foo.psd");
        assert_eq!(window_title(&app), "foo.psd \u{2014} PhotoCraft");
        // Unsaved changes gain a `*` prefix, like the `•` in the app-drawn title.
        if let Some(st) = app.session.active_mut() {
            st.revision += 1;
        }
        assert_eq!(window_title(&app), "*foo.psd \u{2014} PhotoCraft");
    }

    #[test]
    fn hiding_the_file_name_keeps_the_app_name_and_the_dirty_mark() {
        let mut app = app(true);
        open_named(&mut app, "foo.psd");
        app.session.prefs.edit(|p| p.interface.show_file_name_in_title = false);
        assert_eq!(window_title(&app), "PhotoCraft");
        if let Some(st) = app.session.active_mut() {
            st.revision += 1;
        }
        assert_eq!(window_title(&app), "*PhotoCraft");
    }

    #[test]
    fn hidden_file_name_leaves_the_app_name_in_the_drawn_title() {
        let mut app = app(true);
        open_named(&mut app, "foo.psd");
        let texts = |app: &mut crate::PhotocraftApp| {
            let ctx = egui::Context::default();
            crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| super::title_bar(app, ui));
            out.textures_delta.clear();
            let mut texts = Vec::new();
            for shape in &out.shapes {
                collect_text(&shape.shape, &mut texts);
            }
            texts
        };
        assert!(texts(&mut app).iter().any(|t| t.contains("foo.psd")), "shown by default");
        app.session.prefs.edit(|p| p.interface.show_file_name_in_title = false);
        let hidden = texts(&mut app);
        assert!(!hidden.iter().any(|t| t.contains("foo.psd")), "file name hidden: {hidden:?}");
        assert!(hidden.iter().any(|t| t.contains("PhotoCraft")), "app name kept: {hidden:?}");
    }

    #[test]
    fn system_mode_hides_the_mark_and_the_centred_title() {
        for custom in [true, false] {
            let mut app = app(custom);
            open_named(&mut app, "foo.psd");
            let ctx = egui::Context::default();
            crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
            let out = ctx.run_ui(egui::RawInput::default(), |ui| super::title_bar(&mut app, ui));
            let mut out = out;
            out.textures_delta.clear();
            let mut texts = Vec::new();
            for shape in &out.shapes {
                collect_text(&shape.shape, &mut texts);
            }
            let system = system_title_bar(&app);
            assert_eq!(system, !custom && !cfg!(target_os = "macos"));
            assert_eq!(crate::brand::mark_rect(&ctx).is_some(), !system, "mark in system mode (custom={custom})");
            assert_eq!(texts.iter().any(|t| t.contains("foo.psd")), !system, "centred title in system mode (custom={custom}): {texts:?}");
        }
    }

    fn collect_text(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| collect_text(s, out)),
            _ => {}
        }
    }

    #[test]
    fn sync_window_title_sends_only_on_change() {
        let mut app = app(true);
        let ctx = egui::Context::default();
        crate::PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        super::sync_window_title(&mut app, &ctx);
        assert_eq!(app.last_window_title, "PhotoCraft");
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let cmds = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
        assert!(cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Title(t) if t == "PhotoCraft")), "{cmds:?}");
        // Same document state again: the cached title stops a repeat command.
        super::sync_window_title(&mut app, &ctx);
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let cmds = out.viewport_output.remove(&egui::ViewportId::ROOT).map(|v| v.commands).unwrap_or_default();
        assert!(!cmds.iter().any(|c| matches!(c, egui::ViewportCommand::Title(_))), "repeat Title: {cmds:?}");
        open_named(&mut app, "foo.psd");
        super::sync_window_title(&mut app, &ctx);
        assert_eq!(app.last_window_title, "foo.psd \u{2014} PhotoCraft");
    }
}

fn label(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.label(RichText::new(s).color(t.text_dim));
}

#[cfg(test)]
mod navigator_wheel_tests {
    use super::*;
    use egui::{Event, Modifiers, MouseWheelUnit, RawInput, TouchPhase};

    /// The painted viewport follows the camera visible on the canvas, including preference changes.
    #[test]
    fn navigator_viewport_follows_the_displayed_camera_during_animation() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.sync_views();
        app.last_canvas_rect = Rect::from_min_size(Pos2::ZERO, vec2(80.0, 60.0));
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        for _ in 0..3 {
            frame(&mut app, &ctx, pos2(5.0, 5.0), 0.0);
        }
        for (animating, enabled, displayed) in [
            (true, true, Some((2.0, [70.0, 40.0]))),
            (true, true, Some((3.0, [110.0, 50.0]))),
            (false, true, Some((2.0, [70.0, 40.0]))),
            (true, false, Some((2.0, [70.0, 40.0]))),
            (true, true, None),
        ] {
            app.run("prefs.set", json!({"values": {"general.animatedZoom": enabled}})).unwrap();
            let v = &mut app.ui.views[0];
            v.zoom = 4.0;
            v.center = [140.0, 60.0];
            v.zoom_animating = animating;
            v.display_camera = displayed;
            let mut out = ctx.run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(430.0, 460.0))), ..Default::default() }, |ui| {
                navigator(&mut app, ui)
            });
            let painted = out
                .shapes
                .iter()
                .find_map(|s| match &s.shape {
                    egui::Shape::Rect(r) if r.stroke.color == Color32::from_rgb(255, 84, 84) => Some(r.rect),
                    _ => None,
                })
                .expect("Navigator viewport outline");
            let (zoom, center) = if animating && enabled { displayed.unwrap_or((4.0, [140.0, 60.0])) } else { (4.0, [140.0, 60.0]) };
            let preview = last_navigator_preview(&ctx).unwrap();
            let scale = preview.width() / 200.0;
            let expected_center = preview.min + vec2(center[0], center[1]) * scale;
            let expected_size = vec2(80.0 / app.ui.view.display_aspect(), 60.0) * (app.canvas_ppp() / zoom * scale);
            assert!((painted.center() - expected_center).length() < 0.01, "{animating}/{enabled}/{displayed:?}: {painted:?}");
            assert!((painted.size() - expected_size).length() < 0.01, "{animating}/{enabled}/{displayed:?}: {painted:?}");
            out.textures_delta.clear();
        }
    }

    fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, pointer: Pos2, wheel: f32) {
        let mut events = vec![Event::PointerMoved(pointer)];
        if wheel != 0.0 {
            events.push(Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: MouseWheelUnit::Line,
                delta: vec2(0.0, wheel),
                phase: TouchPhase::Move,
                modifiers: Modifiers::NONE,
            });
        }
        let mut out = ctx
            .run_ui(RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(430.0, 460.0))), events, ..Default::default() }, |ui| navigator(app, ui));
        out.textures_delta.clear();
    }

    #[test]
    fn navigator_wheel_changes_zoom_only_when_over_preview() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.sync_views();
        app.ui.views[0].zoom = 1.0;
        app.ui.views[0].fit_pending = false;
        // Animated Zoom off: a notch is one instant step, so one frame shows the whole ×1.1.
        app.session.edit_prefs(|p| p.general.animated_zoom = false);
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        crate::wheel_nav::configure(&ctx);
        frame(&mut app, &ctx, pos2(50.0, 55.0), 0.0);
        let center = app.ui.views[0].center;
        frame(&mut app, &ctx, pos2(50.0, 55.0), 1.0);
        assert!((app.ui.views[0].zoom - 1.1).abs() < 1e-4, "one wheel notch zooms once");
        assert_eq!(app.ui.views[0].center, center, "Navigator wheel must not pan");
        frame(&mut app, &ctx, pos2(50.0, 55.0), -1.0);
        assert!((app.ui.views[0].zoom - 1.0).abs() < 1e-4);
        // Below the preview (it now fills the group, #2726): over the zoom controls.
        let below = last_navigator_controls(&ctx).map_or(pos2(50.0, 455.0), |r| pos2(r.left() + 4.0, r.bottom() - 2.0));
        frame(&mut app, &ctx, below, 1.0);
        assert!((app.ui.views[0].zoom - 1.0).abs() < 1e-4, "wheel outside preview leaves zoom unchanged");
    }

    /// #6872: the zoom a Navigator ⌘/Ctrl-drag sets: the rectangle fills the canvas, centred.
    #[test]
    fn navigator_zoom_to_fits_the_rectangle_in_the_canvas() {
        let fresh = || crate::state::View { doc_size: [200, 100], zoom: 1.0, center: [100.0, 50.0], ..Default::default() };
        let canvas = vec2(800.0, 600.0);
        let r = Rect::from_min_max(pos2(50.0, 25.0), pos2(100.0, 50.0));
        // 50 × 25 px on 800 × 600 pt: the width limits it, 16 points per pixel.
        let mut v = fresh();
        navigator_zoom_to(&mut v, r, [200, 100], canvas, 1.0, 1.0);
        assert_eq!((v.zoom, v.center, v.fit_pending), (crate::zoom_levels::clamp(16.0, [200, 100]), [75.0, 37.5], false));
        // Device pixels per point and a 2:1 pixel aspect scale it as the canvas shows pixels.
        let mut v = fresh();
        navigator_zoom_to(&mut v, r, [200, 100], canvas, 2.0, 2.0);
        assert_eq!(v.zoom, crate::zoom_levels::clamp(16.0, [200, 100]), "8 points per pixel across, at 2 device pixels per point");
        // Off the document: only the part on it counts.
        let mut v = fresh();
        navigator_zoom_to(&mut v, Rect::from_min_max(pos2(150.0, 50.0), pos2(400.0, 300.0)), [200, 100], canvas, 1.0, 1.0);
        assert_eq!(v.center, [175.0, 75.0]);
        // Under 2 px across: one step in on its centre.
        let mut v = fresh();
        navigator_zoom_to(&mut v, Rect::from_min_max(pos2(10.0, 10.0), pos2(11.0, 40.0)), [200, 100], canvas, 1.0, 1.0);
        assert_eq!((v.zoom, v.center), (crate::zoom_levels::step(1.0, 1, [200, 100]), [10.5, 25.0]));
        // Not a number, or wholly off the document: nothing changes.
        for bad in [Rect::from_min_max(pos2(f32::NAN, 0.0), pos2(10.0, 10.0)), Rect::from_min_max(pos2(300.0, 300.0), pos2(400.0, 400.0))] {
            let mut v = fresh();
            navigator_zoom_to(&mut v, bad, [200, 100], canvas, 1.0, 1.0);
            assert_eq!((v.zoom, v.center), (1.0, [100.0, 50.0]), "{bad:?}");
        }
    }

    /// One frame of the Navigator with pointer `events` and `modifiers` held.
    fn frame_with(app: &mut PhotocraftApp, ctx: &egui::Context, modifiers: Modifiers, mut events: Vec<Event>) {
        events.insert(0, Event::ModifiersChanged(modifiers));
        let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(430.0, 460.0))), events, ..Default::default() };
        ctx.run_ui(input, |ui| navigator(app, ui)).textures_delta.clear();
    }

    /// A drag over the Navigator's preview between document points `from` and `to`.
    fn drag(app: &mut PhotocraftApp, ctx: &egui::Context, modifiers: Modifiers, from: [f32; 2], to: [f32; 2]) {
        let preview = last_navigator_preview(ctx).expect("the preview");
        let s = preview.width() / 200.0;
        let at = |p: [f32; 2]| preview.min + vec2(p[0], p[1]) * s;
        let (a, b) = (at(from), at(to));
        let button = |pos, pressed| Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers };
        frame_with(app, ctx, modifiers, vec![Event::PointerMoved(a), button(a, true)]);
        frame_with(app, ctx, modifiers, vec![Event::PointerMoved(a.lerp(b, 0.5))]);
        frame_with(app, ctx, modifiers, vec![Event::PointerMoved(b)]);
        frame_with(app, ctx, modifiers, vec![button(b, false)]);
        frame_with(app, ctx, Modifiers::NONE, vec![]);
    }

    /// #6872: ⌘/Ctrl-dragging a rectangle over the preview zooms the view to it, as in
    /// Photoshop; a plain drag still only moves the view.
    #[test]
    fn navigator_command_drag_zooms_to_the_rectangle() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.sync_views();
        app.ui.views[0].zoom = 1.0;
        app.ui.views[0].fit_pending = false;
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        // The preview settles once the zoom row under it has been measured.
        for _ in 0..3 {
            frame(&mut app, &ctx, pos2(5.0, 5.0), 0.0);
        }
        // A plain drag pans and keeps the zoom.
        drag(&mut app, &ctx, Modifiers::NONE, [50.0, 25.0], [100.0, 50.0]);
        assert_eq!(app.ui.views[0].zoom, 1.0);
        let c = app.ui.views[0].center;
        assert!((c[0] - 100.0).abs() < 1.0 && (c[1] - 50.0).abs() < 1.0, "the view follows the pointer: {c:?}");
        // Finishing direct navigation during an existing zoom must snap to the new target.
        let v = &mut app.ui.views[0];
        v.zoom = 2.0;
        v.request_zoom_animation(true);
        v.display_camera = Some((1.5, c));
        v.zoom_transition = Some(crate::state::ZoomTransition { from: (1.0, c), target: (v.zoom, c), started: 0.0 });
        v.zoom_anchor = Some(([50.0, 25.0], [200.0, 150.0]));
        // ⌘/Ctrl-drag: the rectangle fills the canvas (`last_canvas_rect`, 800 × 600 pt).
        drag(&mut app, &ctx, Modifiers::COMMAND, [50.0, 25.0], [100.0, 50.0]);
        let mut expected = app.ui.views[0].clone();
        navigator_zoom_to(
            &mut expected,
            Rect::from_min_max(pos2(50.0, 25.0), pos2(100.0, 50.0)),
            [200, 100],
            vec2(800.0, 600.0),
            app.canvas_ppp(),
            app.ui.view.display_aspect(),
        );
        let v = &app.ui.views[0];
        assert!((v.zoom - expected.zoom).abs() / expected.zoom < 0.05, "{} vs {}", v.zoom, expected.zoom);
        assert!(
            (v.center[0] - 75.0).abs() < 1.0 && (v.center[1] - 37.5).abs() < 1.0,
            "centred on the rectangle: {:?} (zoom {}, expected {:?})",
            v.center,
            v.zoom,
            (expected.zoom, expected.center)
        );
        assert!(v.zoom > 1.0 && !v.fit_pending);
        assert!(!v.zoom_animating && v.zoom_transition.is_none() && v.zoom_anchor.is_none(), "direct navigation cancels the old transition");
        assert_eq!(v.display_camera, Some((v.zoom, v.center)), "the selected rectangle is displayed immediately");
        assert!(ctx.data(|d| d.get_temp::<Pos2>(navigator_zoom_drag_id())).is_none(), "the drag is over");
    }

    /// #4840: under the preview, one row as in Photoshop: the zoom in percent, Zoom Out, the
    /// slider and Zoom In. The buttons step the zoom like View › Zoom In / Out.
    #[test]
    fn navigator_zoom_row_is_one_line_and_its_buttons_step_the_zoom() {
        use egui_kittest::kittest::Queryable;
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.sync_views();
        app.ui.views[0].zoom = 1.0;
        app.ui.views[0].fit_pending = false;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(360.0, 300.0)).build_ui_state(|ui, app: &mut PhotocraftApp| navigator(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
        h.run_steps(4);
        let row = last_navigator_controls(&h.ctx).expect("the zoom row");
        assert!(row.height() <= NAV_ROW_H + 14.0, "one row (with the gap above it): {row:?}");
        let zoom = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.state().ui.views[0].zoom;
        h.get_by_label("Zoom In").click();
        h.run_steps(2);
        assert!((zoom(&h) - crate::zoom_levels::step(1.0, 1, [200, 100])).abs() < 1e-4, "{}", zoom(&h));
        h.get_by_label("Zoom Out").click();
        h.run_steps(2);
        h.get_by_label("Zoom Out").click();
        h.run_steps(2);
        assert!((zoom(&h) - crate::zoom_levels::step(1.0, -1, [200, 100])).abs() < 1e-4, "{}", zoom(&h));
        assert!(!h.state().ui.views[0].fit_pending);
    }
}

fn empty(ui: &mut egui::Ui, s: &str) {
    let t = Tokens::get(ui.ctx());
    ui.add_space(6.0);
    ui.label(RichText::new(s).color(t.text_faint));
    ui.add_space(6.0);
}

#[cfg(test)]
mod symmetry_theme_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn brush_pencil_and_eraser_expose_symmetry_menu_in_every_theme() {
        for kind in crate::theme::ThemeKind::ALL {
            for tool in [Tool::Brush, Tool::Pencil, Tool::Eraser] {
                let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
                app.run("file.new", json!({"width": 80, "height": 80})).unwrap();
                app.ui.tool = tool;
                let mut h = Harness::builder().with_size(vec2(1600.0, 300.0)).build_ui_state(|ui, app| options_bar(app, ui), app);
                PhotocraftApp::setup_context(&h.ctx, kind);
                h.run_steps(4);
                assert!(h.query_by_label("Set painting symmetry options").is_some(), "missing symmetry menu in {:?} for {:?}", kind, tool);
            }
        }
    }
}

#[cfg(test)]
mod color_tests;

#[cfg(test)]
mod info_panel_sampler_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    /// #1046: the Info panel lists each Color Sampler point under its number, with its position and
    /// the composite RGB/CMYK readout, one composite pixel each.
    #[test]
    fn info_panel_lists_the_sample_points() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 80, "height": 60, "background": "white"})).unwrap();
        app.run("view.colorSamplers.add", json!({"x": 5, "y": 6})).unwrap();
        app.run("view.colorSamplers.add", json!({"x": 40, "y": 30})).unwrap();
        let mut h = Harness::builder().with_size(vec2(340.0, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| info_panel(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Pro);
        h.run_steps(3);
        // Both points are listed, numbered, and read as white (RGB 255 / CMYK 0%).
        assert!(h.query_by_label("#1").is_some(), "point 1 is listed");
        assert!(h.query_by_label("#2").is_some(), "point 2 is listed");
        assert_eq!(h.query_all_by_label_contains("R:255 G:255 B:255").count(), 2, "each point shows its RGB");
        assert_eq!(h.query_all_by_label_contains("C:0% M:0% Y:0% K:0%").count(), 2, "each point shows its CMYK");
        // Clearing them takes the list away.
        h.state_mut().run("view.colorSamplers.clear", json!({})).unwrap();
        h.run_steps(2);
        assert!(h.query_by_label("#1").is_none(), "no points listed once cleared");
    }
}

#[cfg(test)]
mod history_transform_tests {
    use super::*;
    use egui_kittest::Harness;

    fn click(h: &mut Harness<'static, PhotocraftApp>, p: egui::Pos2) {
        h.hover_at(p);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
    }

    /// While Free Transform is open, clicking a History state leaves the document alone: the
    /// transform owns Undo. Without a transform the same click steps back as usual.
    #[test]
    fn history_rows_wait_for_an_open_transform() {
        for transforming in [true, false] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("select.rect", json!({"x": 20, "y": 20, "width": 60, "height": 40})).unwrap();
            app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
            app.run("select.deselect", json!({})).unwrap();
            let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| history(app, ui), app);
            h.run_steps(2);
            if transforming {
                let ctx = h.ctx.clone();
                crate::transform_tool::begin(h.state_mut(), &ctx).unwrap();
            }
            let steps = h.state().session.active().unwrap().history.entries().len();
            // The first state's row, below the 38 pt snapshot row.
            let row = h.ctx.input(|i| i.viewport_rect()).min + vec2(60.0, 60.0);
            click(&mut h, row);
            let after = h.state().session.active().unwrap().history.entries().len();
            if transforming {
                assert_eq!(after, steps, "the document's history is untouched");
                assert!(h.state().ui.transform.is_some());
            } else {
                assert!(after < steps, "the click steps back: {steps} -> {after}");
            }
        }
    }

    /// The Photoshop-theme footer (#1117): trash deletes the current state, the camera takes a
    /// snapshot (#2966), and file-plus makes a new document from the current state.
    #[test]
    fn history_footer_buttons_run_their_commands() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| history(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.state_mut().ui.theme = crate::theme::ThemeKind::ProMedium;
        h.run_steps(3);
        // Footer buttons are the only 26 pt squares; the bar lays them out right to left.
        let buttons = |h: &Harness<'static, PhotocraftApp>| {
            let mut b: Vec<(Rect, bool)> = h.ctx.viewport(|v| {
                v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(26.0, 26.0)).map(|w| (w.rect, w.enabled)).collect()
            });
            b.sort_by(|a, b| b.0.center().x.total_cmp(&a.0.center().x));
            b
        };
        let b = buttons(&h);
        assert_eq!(b.len(), 3, "trash, snapshot, new document: {b:?}");
        assert!(b.iter().all(|b| b.1), "all enabled: {b:?}");
        let steps = h.state().session.active().unwrap().history.entries().len();
        click(&mut h, b[1].0.center());
        let st = h.state().session.active().unwrap();
        assert_eq!(st.snapshots.len(), 2, "the camera takes a snapshot, after the first one");
        assert_eq!(st.history.entries().len(), steps, "which is not a history state");
        let docs = h.state().session.documents().len();
        click(&mut h, b[2].0.center());
        assert_eq!(h.state().session.documents().len(), docs + 1, "file-plus makes a new document");
        h.state_mut().session.set_active(0);
        h.run_steps(2);
        let steps = h.state().session.active().unwrap().history.entries().len();
        let b = buttons(&h);
        click(&mut h, b[0].0.center());
        let st = h.state().session.active().unwrap();
        assert_eq!(st.history.entries().len(), steps - 1, "trash deletes the current state");
        assert!(!st.history.can_redo(), "and it can't be redone");
    }

    /// The source column at the left of each state (#2966): a click makes that state the History
    /// Brush's source through `history.setSource`, without stepping to it.
    #[test]
    fn history_source_column_sets_the_history_brush_source() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("edit.fill", json!({"color": "#00ff00"})).unwrap();
        let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| history(app, ui), app);
        h.run_steps(2);
        // The column cells are the only 24 pt squares, one per state, top to bottom.
        let mut cells: Vec<Rect> = h
            .ctx
            .viewport(|v| v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(24.0, 24.0)).map(|w| w.rect).collect());
        cells.sort_by(|a, b| a.center().y.total_cmp(&b.center().y));
        cells.dedup();
        assert_eq!(cells.len(), 3, "Open, Fill, Fill: {cells:?}");
        assert_eq!(h.state().session.active().unwrap().history.brush_source(), Some(0), "the default source");
        click(&mut h, cells[1].center());
        let st = h.state().session.active().unwrap();
        assert_eq!(st.history.brush_source(), Some(1), "the clicked state is the source");
        assert_eq!((st.history.entries().len(), st.history.can_redo()), (3, false), "and the document did not step back");
    }

    /// Snapshot rows have the source column too: a click makes the snapshot the source without
    /// returning to it.
    #[test]
    fn history_source_column_on_a_snapshot_row() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("history.newSnapshot", json!({})).unwrap();
        let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| history(app, ui), app);
        h.run_steps(2);
        // Snapshot rows' cells are the 24×38 ones: the opening snapshot (0) first, then the new one.
        let mut cells: Vec<Rect> = h
            .ctx
            .viewport(|v| v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(24.0, 38.0)).map(|w| w.rect).collect());
        cells.sort_by(|a, b| a.top().total_cmp(&b.top()));
        assert_eq!(cells.len(), 2, "a source cell on each snapshot row");
        let cell = cells[1];
        click(&mut h, cell.center());
        let st = h.state().session.active().unwrap();
        assert_eq!(photocraft_engine::history_source_cmds::source(st), Some(HistorySource::Snapshot(1)));
        assert_eq!(st.history.entries().len(), 2, "the document did not return to the snapshot");
    }
}

#[cfg(test)]
mod snapshot_row_tests {
    use egui::vec2;
    use egui_kittest::Harness;
    use egui_kittest::kittest::{NodeT, Queryable};
    use serde_json::json;

    use super::*;

    fn top_pixel(h: &Harness<'static, PhotocraftApp>) -> [f32; 4] {
        h.state().session.active().unwrap().doc.layers.last().unwrap().surface().unwrap().rgba(1, 1)
    }

    /// The names of the snapshots taken after the first one (the document as opened).
    fn snapshot_names(h: &Harness<'static, PhotocraftApp>) -> Vec<String> {
        h.state().session.active().unwrap().snapshots.iter().skip(1).map(|s| s.name.clone()).collect()
    }

    /// Snapshot rows (#2966) sit under the opening row: clicking one returns to it as a new
    /// state, and its right-click menu renames or deletes it.
    #[test]
    fn snapshot_rows_select_rename_and_delete() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("history.newSnapshot", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        // 60 fps steps, so two clicks a frame apart count as a double-click.
        let mut h =
            Harness::builder().with_size(vec2(300.0, 400.0)).with_step_dt(1.0 / 60.0).build_ui_state(|ui, app: &mut PhotocraftApp| history(app, ui), app);
        h.run_steps(3);
        let steps = h.state().session.active().unwrap().history.entries().len();
        h.get_by_label("Snapshot 1").click();
        h.run_steps(2);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.history.entries().len(), steps + 1);
        assert_eq!(st.history.entries().last().map(String::as_str), Some("Snapshot 1"));
        assert_eq!(top_pixel(&h), [1.0, 0.0, 0.0, 1.0], "back to red");
        // Right-click › Rename…: type a name, Enter. The new "Snapshot 1" state is listed under the
        // snapshot rows, so the first match is the snapshot's row.
        h.get_all_by_label("Snapshot 1").next().unwrap().click_secondary();
        h.run_steps(2);
        h.get_by_label("Rename…").click();
        h.run_steps(2);
        h.get_by_role(egui::accesskit::Role::TextInput).type_text("Lesson start");
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        assert_eq!(snapshot_names(&h), ["Lesson start"]);
        // A double-click renames too; Escape keeps the old name.
        let steps_now = h.state().session.active().unwrap().history.entries().len();
        // A real double-click (accessibility clicks are single clicks).
        let p = h.get_by_label("Lesson start").rect().center();
        let p = egui::pos2(p.x as f32, p.y as f32);
        h.event(egui::Event::PointerMoved(p));
        // Wait out the double-click window of the earlier clicks.
        h.run_steps(30);
        for pressed in [true, false, true, false] {
            h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.step();
        }
        h.run_steps(3);
        h.get_by_role(egui::accesskit::Role::TextInput).type_text("Oops");
        h.key_press(egui::Key::Escape);
        h.run_steps(2);
        assert_eq!(snapshot_names(&h), ["Lesson start"], "Escape cancels");
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps_now + 1, "the click selected it once");
        // Right-click › Delete (the snapshot's row, above the state the click made).
        h.get_all_by_label("Lesson start").next().unwrap().click_secondary();
        h.run_steps(2);
        h.get_by_label("Delete").click();
        h.run_steps(2);
        assert!(snapshot_names(&h).is_empty());
        assert_eq!(h.state().session.active().unwrap().history.entries().len(), steps + 2, "deleting leaves the history alone");
    }

    /// A History panel in the Photoshop theme (the footer with the trash) on red, Snapshot 1, blue.
    fn pro_history() -> Harness<'static, PhotocraftApp> {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        app.run("history.newSnapshot", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#0000ff"})).unwrap();
        let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| history(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.state_mut().ui.theme = crate::theme::ThemeKind::ProMedium;
        h.run_steps(3);
        h
    }

    /// The footer's trash: the rightmost of its 26 pt buttons (the bar lays them out right to left).
    fn trash(h: &Harness<'static, PhotocraftApp>) -> egui::Pos2 {
        let buttons: Vec<Rect> = h
            .ctx
            .viewport(|v| v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(26.0, 26.0)).map(|w| w.rect).collect());
        buttons.into_iter().max_by(|a, b| a.center().x.total_cmp(&b.center().x)).unwrap().center()
    }

    fn click(h: &mut Harness<'static, PhotocraftApp>, p: egui::Pos2) {
        h.hover_at(p);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
    }

    fn selected(h: &Harness<'static, PhotocraftApp>, label: &str) -> bool {
        h.get_all_by_label(label).next().unwrap().accesskit_node().toggled() == Some(egui::accesskit::Toggled::True)
    }

    /// Press on `from`, drag in small steps to `to` and release there.
    fn drag(h: &mut Harness<'static, PhotocraftApp>, from: egui::Pos2, to: egui::Pos2) {
        h.event(egui::Event::PointerMoved(from));
        h.event(egui::Event::PointerButton { pos: from, button: egui::PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
        h.step();
        for k in 1..=10 {
            h.event(egui::Event::PointerMoved(from + (to - from) * (k as f32 / 10.0)));
            h.step();
        }
        h.event(egui::Event::PointerButton { pos: to, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
        h.run_steps(3);
    }

    /// Clicking a snapshot selects it, and the trash then deletes the snapshot instead of a state
    /// (#5324). Any other step deselects it, and the trash deletes the current state again.
    #[test]
    fn the_trash_deletes_the_selected_snapshot() {
        let mut h = pro_history();
        h.get_by_label("Snapshot 1").click();
        h.run_steps(2);
        let steps = h.state().session.active().unwrap().history.entries().len();
        assert!(selected(&h, "Snapshot 1"), "the clicked snapshot shows selected");
        let p = trash(&h);
        click(&mut h, p);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.snapshots.len(), 1, "the trash deleted the snapshot (the first snapshot stays)");
        assert_eq!(st.history.entries().len(), steps, "and left the states alone");

        // An edit after the click deselects the snapshot: the trash deletes the state.
        let mut h = pro_history();
        h.get_by_label("Snapshot 1").click();
        h.run_steps(2);
        h.state_mut().run("edit.fill", json!({"color": "#00ff00"})).unwrap();
        h.run_steps(2);
        assert!(!selected(&h, "Snapshot 1"), "an edit deselects the snapshot");
        let steps = h.state().session.active().unwrap().history.entries().len();
        let p = trash(&h);
        click(&mut h, p);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.snapshots.len(), 2, "the snapshot is kept");
        assert_eq!(st.history.entries().len(), steps - 1, "the current state is deleted");
    }

    /// A snapshot or a past state dragged onto the trash is deleted (#5324).
    #[test]
    fn rows_dragged_onto_the_trash_are_deleted() {
        let mut h = pro_history();
        let steps = h.state().session.active().unwrap().history.entries().len();
        let row = h.get_by_label("Snapshot 1").rect().center();
        let to = trash(&h);
        drag(&mut h, egui::pos2(row.x, row.y), to);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.snapshots.len(), 1, "the dragged snapshot is deleted (the first snapshot stays)");
        assert_eq!(st.history.entries().len(), steps, "no state is");

        // The second-to-last state: it and the state after it go, as with Delete.
        let mut h = pro_history();
        // The states are Open (or New), New Layer, Fill, Fill: drag New Layer.
        let entries = h.state().session.active().unwrap().history.entries();
        assert_eq!(entries.len(), 4, "{entries:?}");
        let row = h.get_by_label(&entries[1]).rect().center();
        let to = trash(&h);
        drag(&mut h, egui::pos2(row.x, row.y), to);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.history.entries().len(), 1, "the dragged state and the ones after it are deleted");
        assert_eq!(st.snapshots.len(), 2, "the snapshot is kept");
    }

    /// Snapshot rows scroll with the states: many snapshots in a short dock leave the footer and
    /// its camera button in view.
    #[test]
    fn many_snapshots_keep_the_footer_in_a_short_panel() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        for _ in 0..6 {
            app.run("history.newSnapshot", json!({})).unwrap();
        }
        let mut h = Harness::builder().with_size(vec2(300.0, 250.0)).build_ui_state(|ui, app: &mut PhotocraftApp| history(app, ui), app);
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.state_mut().ui.theme = crate::theme::ThemeKind::ProMedium;
        h.run_steps(3);
        let view = h.ctx.input(|i| i.viewport_rect());
        let buttons: Vec<Rect> = h
            .ctx
            .viewport(|v| v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(26.0, 26.0)).map(|w| w.rect).collect());
        assert_eq!(buttons.len(), 3, "{buttons:?}");
        assert!(buttons.iter().all(|b| view.contains_rect(*b)), "footer in view: {buttons:?} in {view:?}");
    }

    /// The top row is the first snapshot, the document as opened: clicking it brings that back
    /// (the Discord report on #2966: "History original file click does not work").
    #[test]
    fn the_top_row_returns_to_the_document_as_opened() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        let opened = app.session.active().unwrap().doc.layers[0].surface().unwrap().rgba(1, 1);
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let name = app.session.active().unwrap().doc.name.clone();
        let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| history(app, ui), app);
        h.run_steps(3);
        h.get_by_label(&name).click();
        h.run_steps(2);
        let st = h.state().session.active().unwrap();
        assert_eq!(st.doc.layers[0].surface().unwrap().rgba(1, 1), opened);
        assert_eq!(st.history.entries().last(), Some(&name));
    }

    /// ⌥-click on the camera opens New Snapshot (#2966): the name, selected so typing replaces
    /// it, and From (Full Document, Merged Layers, Current Layer); OK takes the snapshot.
    #[test]
    fn alt_click_on_the_camera_opens_new_snapshot() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
        app.run("layer.new.layer", json!({})).unwrap();
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                let ctx = ui.ctx().clone();
                if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    return;
                }
                history(app, ui);
                crate::dialogs::show(app, &ctx);
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ProMedium);
        h.state_mut().ui.theme = crate::theme::ThemeKind::ProMedium;
        h.run_steps(4);
        let mut buttons: Vec<Rect> = h
            .ctx
            .viewport(|v| v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == vec2(26.0, 26.0)).map(|w| w.rect).collect());
        buttons.sort_by(|a, b| b.center().x.total_cmp(&a.center().x));
        let camera = buttons[1].center();
        let alt = egui::Modifiers::ALT;
        h.event(egui::Event::ModifiersChanged(alt));
        h.event(egui::Event::PointerMoved(camera));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: camera, button: egui::PointerButton::Primary, pressed, modifiers: alt });
            h.run_steps(1);
        }
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run_steps(3);
        assert_eq!(h.state().session.active().unwrap().snapshots.len(), 1, "⌥-click opens the dialog instead");
        assert!(h.state().ui.dialogs.last().is_some_and(|d| crate::snapshot_dialog::owns(&d.fields)));
        h.get_by_role(egui::accesskit::Role::TextInput).type_text("Red layer only");
        h.run_steps(1);
        h.get_all_by_role(egui::accesskit::Role::ComboBox).find(|n| n.accesskit_node().value().as_deref() == Some("Full Document")).unwrap().click();
        h.run_steps(3);
        h.get_by_label("Current Layer").click();
        h.run_steps(3);
        h.get_by_label("OK").click();
        h.run_steps(3);
        let st = h.state().session.active().unwrap();
        let snap = st.snapshots.last().unwrap();
        assert_eq!(snap.name, "Red layer only");
        assert_eq!(snap.doc.layers.len(), 1, "only the current layer");
        assert!(h.state().ui.dialogs.is_empty());
    }

    /// Renaming belongs to one document: snapshot ids restart in each, so switching tabs must not
    /// carry the edit over to the other document's snapshot with the same id.
    #[test]
    fn a_rename_in_progress_stays_with_its_document() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        for _ in 0..2 {
            app.run("file.new", json!({"width": 200, "height": 150})).unwrap();
            app.run("history.newSnapshot", json!({})).unwrap();
        }
        let mut h = Harness::builder().with_size(vec2(300.0, 400.0)).build_ui_state(|ui, app: &mut PhotocraftApp| history(app, ui), app);
        h.run_steps(3);
        h.get_by_label("Snapshot 1").click_secondary();
        h.run_steps(2);
        h.get_by_label("Rename…").click();
        h.run_steps(2);
        assert!(h.query_by_role(egui::accesskit::Role::TextInput).is_some(), "renaming in document 2");
        h.state_mut().session.set_active(0);
        h.run_steps(2);
        assert!(h.query_by_role(egui::accesskit::Role::TextInput).is_none(), "not in document 1");
    }
}

#[cfg(test)]
mod layer_pct_slider_tests;

#[cfg(test)]
mod fx_drag_tests;

#[cfg(test)]
mod unsupported_marker_tests;

#[cfg(test)]
mod lock_tests {
    use super::*;
    use crate::canvas::{ToolEvent, tool_event};
    use egui::Modifiers;

    fn click_lock(app: &mut PhotocraftApp) {
        let st = app.session.active().unwrap();
        let l = st.active_layer.and_then(|id| st.doc.layer(id)).unwrap();
        let (cmd, p) = simple_lock_toggle(crate::doc_props_ui::is_background(&st.doc, l), l, &json!({"layer": l.id.0}));
        app.run(&cmd, p).unwrap();
    }

    fn erase_line(app: &mut PhotocraftApp) {
        app.ui.tool = Tool::Eraser;
        let m = Modifiers::NONE;
        tool_event(app, ToolEvent::Down { x: 10.0, y: 40.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Move { x: 100.0, y: 40.0, pressure: 1.0 }, m);
        tool_event(app, ToolEvent::Up { x: 100.0, y: 40.0 }, m);
    }

    #[test]
    fn simple_lock_button_never_locks_the_eraser_out_of_the_background() {
        // #76: the simple themes' lock button used to toggle "lock all" on the Background, which
        // kept showing as locked and then refused every paint tool.
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 120, "height": 80})).unwrap();
        app.run("tools.setColors", json!({"background": "#ff0000"})).unwrap();
        app.run("tools.setBrush", json!({"brush": {"size": 12, "hardness": 1.0}})).unwrap();
        // Clicking the Background's lock converts it to a normal layer (Photoshop).
        click_lock(&mut app);
        let l = &app.session.active().unwrap().doc.layers[0];
        assert_eq!(l.name, "Layer 0");
        assert!(!l.locks.any());
        erase_line(&mut app);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        assert_eq!(app.session.active().unwrap().doc.layers[0].surface().unwrap().rgba(50, 40)[3], 0.0, "erased to transparency");
        // On a normal layer the button locks all, then unlocks everything.
        click_lock(&mut app);
        assert!(app.session.active().unwrap().doc.layers[0].locks.all);
        click_lock(&mut app);
        assert!(!app.session.active().unwrap().doc.layers[0].locks.any());
        // A layer with only transparency locked unlocks in one click.
        app.run("layer.setProps", json!({"locks": {"transparency": true}})).unwrap();
        click_lock(&mut app);
        assert!(!app.session.active().unwrap().doc.layers[0].locks.transparency);
    }
}

#[cfg(test)]
mod swatch_type_tests {
    use super::*;
    use egui_kittest::Harness;

    /// The Color field only acts on an edit. Its h/s/v round trip isn't exact for every colour
    /// (#D8452E isn't), and writing it back every frame would recolour selected type the moment it
    /// was selected.
    #[test]
    fn idle_color_field_leaves_the_foreground_and_selected_type_alone() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 200})).unwrap();
        let id = app.run("type.create", json!({"text": "Hello world", "size": 40, "x": 20, "y": 100, "color": "#ffffff"})).unwrap()["layer"].as_u64().unwrap();
        app.run("tools.setColors", json!({"foreground": "#d8452e"})).unwrap();
        let fg = app.session.tools.foreground;
        app.ui.tool = Tool::Type;
        app.ui.text_edit = Some(crate::state::TextEdit {
            layer: id,
            document: None,
            caret: 0,
            anchor: 5,
            session: "s".into(),
            created: false,
            mask: false,
            dragging: false,
            resize: None,
            preedit: None,
        });
        let mut h = Harness::builder().with_size(vec2(300.0, 300.0)).build_ui_state(|ui, app: &mut PhotocraftApp| color_field(app, ui), app);
        h.run_steps(4);
        assert_eq!(h.state().session.tools.foreground, fg);
        let st = h.state().session.active().unwrap();
        let Some(photocraft_doc::LayerContent::Text(t)) = st.doc.layer(photocraft_doc::LayerId(id)).map(|l| &l.content) else { panic!("type layer") };
        assert_eq!(t.char_runs().len(), 1, "still one white run");
    }
}

#[cfg(test)]
mod tool_flyout_drag_tests {
    use super::*;

    fn frame(app: &mut PhotocraftApp, ctx: &egui::Context, time: f64, events: Vec<egui::Event>) {
        let mut out = ctx.run_ui(
            egui::RawInput { time: Some(time), events, screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 1800.0))), ..Default::default() },
            |ui| toolbar(app, ui),
        );
        out.textures_delta.clear();
    }

    fn primary(pos: Pos2, pressed: bool) -> egui::Event {
        egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, modifiers: Default::default() }
    }

    #[test]
    fn flyout_drag_only_opens_after_a_primary_press_leaves_its_button() {
        let button = Rect::from_min_size(pos2(20.0, 20.0), vec2(36.0, 36.0));
        let inside = button.center();
        let outside = pos2(120.0, 34.0);
        assert!(!tool_flyout_drag_open(button, Some(inside), Some(inside), true));
        assert!(!tool_flyout_drag_open(button, Some(inside), Some(outside), false));
        assert!(!tool_flyout_drag_open(button, Some(outside), Some(outside), true));
        assert!(!tool_flyout_drag_open(button, None, Some(outside), true));
        assert!(tool_flyout_drag_open(button, Some(inside), Some(outside), true));
        let row = Rect::from_min_size(pos2(90.0, 20.0), vec2(160.0, 26.0));
        assert!(tool_flyout_release(row, Some(row.center()), true, true));
        assert!(!tool_flyout_release(row, Some(row.center()), false, true));
        assert!(!tool_flyout_release(row, Some(row.center()), true, false));
        assert!(!tool_flyout_release(row, Some(outside + vec2(0.0, 200.0)), true, true));
        assert!(!tool_flyout_release(row, None, true, true));
    }

    /// #2301: the press begins on the toolbar button, not on a flyout row. No long-press
    /// delay is needed; releasing over the second row picks it and closes the flyout.
    #[test]
    fn flyout_press_drag_release_selects_the_row_without_long_press() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        frame(&mut app, &ctx, 0.0, vec![]);
        frame(&mut app, &ctx, 0.1, vec![]);
        let slot_index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::Brush)).unwrap();
        let bx = if Tokens::get(&ctx).pro { 30.0 } else { 36.0 };
        let mut buttons: Vec<Rect> = ctx.viewport(|v| {
            v.prev_pass
                .widgets
                .layers()
                .flat_map(|(_, w)| w.iter())
                .filter(|w| w.rect.size() == egui::Vec2::splat(bx) && w.sense.senses_click())
                .map(|w| w.rect)
                .collect()
        });
        buttons.sort_by(|a, b| a.top().total_cmp(&b.top()));
        let origin = buttons[slot_index].center();
        let key = egui::Id::unique(("tool-slot", slot_index));
        let flyout_id = egui::Id::unique("tool-flyout");
        let initial = app.ui.tool;
        frame(&mut app, &ctx, 1.0, vec![egui::Event::PointerMoved(origin), primary(origin, true)]);
        assert!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(flyout_id)).is_none());
        // Move right of the icon, still held. The flyout must appear before 350 ms.
        frame(&mut app, &ctx, 1.08, vec![egui::Event::PointerMoved(origin + vec2(bx + 70.0, 0.0))]);
        assert_eq!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(flyout_id)).map(|(id, _)| id), Some(key));
        assert_eq!(app.ui.tool, initial, "drag-opening doesn't activate the starting tool");
        let menu = ctx.memory(|m| m.area_rect(key.with("flyout"))).expect("flyout area");
        let row = pos2(menu.left() + 65.0, menu.top() + 47.0);
        frame(&mut app, &ctx, 1.12, vec![egui::Event::PointerMoved(row)]);
        frame(&mut app, &ctx, 1.16, vec![primary(row, false)]);
        assert_eq!(app.ui.tool, Tool::Pencil);
        assert!(ctx.data(|d| d.get_temp::<(egui::Id, Rect)>(flyout_id)).is_none());
        frame(&mut app, &ctx, 1.2, vec![]);
        assert_eq!(app.ui.tool, Tool::Pencil, "the original button cannot activate on release");
    }
}

#[cfg(test)]
mod type_flyout_tests;

#[cfg(test)]
mod toolbar_hidden_tests {
    use super::*;

    /// Clickable tool-sized buttons the toolbar drew in its last pass.
    fn tool_buttons(app: &mut PhotocraftApp) -> usize {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::ALL[0]);
        for time in [0.0, 0.1] {
            let mut out = ctx.run_ui(
                egui::RawInput { time: Some(time), screen_rect: Some(Rect::from_min_size(egui::Pos2::ZERO, vec2(1200.0, 1800.0))), ..Default::default() },
                |ui| toolbar(app, ui),
            );
            out.textures_delta.clear();
        }
        let size = egui::Vec2::splat(if Tokens::get(&ctx).pro { 30.0 } else { 36.0 });
        ctx.viewport(|v| v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == size && w.sense.senses_click()).count())
    }

    fn hide(app: &mut PhotocraftApp, tools: &[Tool]) {
        let names: Vec<String> = tools.iter().map(|tool| format!("{tool:?}")).collect();
        app.session.edit_prefs(|p| p.toolbar.hidden = names);
    }

    #[test]
    fn hidden_tools_leave_the_toolbar() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        let all = tool_buttons(&mut app);
        let slots: usize = TOOL_SECTIONS.iter().map(|section| section.len()).sum();
        hide(&mut app, &Tool::ALL);
        assert_eq!(tool_buttons(&mut app), all - slots, "every tool slot is gone when every tool is hidden");
        hide(&mut app, &[Tool::Brush, Tool::Pencil, Tool::MixerBrush, Tool::ColorReplacement]);
        assert_eq!(tool_buttons(&mut app), all - 1, "a slot whose tools are all hidden is gone");
        hide(&mut app, &[Tool::Sponge]);
        assert_eq!(tool_buttons(&mut app), all, "a slot with a visible tool stays");
    }

    #[test]
    fn a_slot_keeps_only_its_visible_tools_and_its_index() {
        let sections = visible_sections(&["Sponge".to_string(), "Move".to_string(), "Artboard".to_string()]);
        assert_eq!(sections.len(), TOOL_SECTIONS.len() - 1, "the Move section has no tools left");
        let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&Tool::Sponge)).unwrap();
        let slot = sections.iter().flatten().find(|(i, _)| *i == index).map(|(_, tools)| tools.clone());
        assert_eq!(slot, Some(vec![Tool::Dodge, Tool::Burn]));
        assert!(sections.iter().flatten().all(|(_, tools)| !tools.contains(&Tool::Move) && !tools.contains(&Tool::Sponge)));
        assert_eq!(visible_sections(&[]).iter().map(Vec::len).sum::<usize>(), TOOL_SECTIONS.iter().map(|section| section.len()).sum::<usize>());
        let by_command = visible_sections(&["sponge".to_string(), "move".to_string(), "artboard".to_string()]);
        assert_eq!(by_command, sections, "edit.toolbar names are read like `ui.set {{tool}}` names");
    }
}

#[cfg(test)]
mod properties_card_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    /// Arithmetic left uncommitted in one fill layer's card never lands on the layer selected next.
    #[test]
    fn uncommitted_arithmetic_stays_with_its_layer() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
        let mut ids = Vec::new();
        for color in ["#00ff00", "#0000ff"] {
            s.execute("layer.newFillLayer.solidColor", json!({"color": color})).unwrap();
            ids.push(s.active().unwrap().active_layer.unwrap());
        }
        // The layer selected next already shows 25, the value the edited one has when `25*` stops it.
        s.execute("layer.setProps", json!({"layer": ids[0].0, "opacity": 0.25})).unwrap();
        let mut app = PhotocraftApp::new(s, crate::Services::default());
        app.ui.panels.properties = true;
        let mut h = Harness::builder().with_size(vec2(800.0, 600.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("semibold".into()))) {
                    properties_window(app, ui.ctx());
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        h.query_all_by_role(egui::accesskit::Role::SpinButton).next().unwrap().click();
        h.run_steps(1);
        for ch in "25*2".chars() {
            h.event(egui::Event::Text(ch.to_string()));
            h.run_steps(1);
        }
        // The Layers panel selects the other layer in the frame the click leaves the field.
        h.state_mut().session.execute("layer.select", json!({"layer": ids[0].0})).unwrap();
        h.ctx.memory_mut(|m| m.stop_text_input());
        h.run_steps(3);
        let doc = &h.state().session.active().unwrap().doc;
        assert_eq!(ids.iter().map(|&id| doc.layer(id).unwrap().opacity).collect::<Vec<_>>(), [0.25, 0.25]);
    }
}

#[cfg(test)]
mod toolbar_tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    /// #1197 / #3469: every theme can toggle the toolbar without changing its selection.
    #[test]
    fn header_chevron_toggles_two_columns_in_every_layout() {
        for theme in [
            crate::theme::ThemeKind::Pro,
            crate::theme::ThemeKind::Studio,
            crate::theme::ThemeKind::StudioLight,
            crate::theme::ThemeKind::Classic,
            crate::theme::ThemeKind::AdwaitaDark,
            crate::theme::ThemeKind::SolarizedDark,
            crate::theme::ThemeKind::BreezeDark,
        ] {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h = egui_kittest::Harness::builder().with_size(vec2(800.0, 1400.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        toolbar(app, ui);
                        let left = ui.available_rect_before_wrap().left();
                        ui.data_mut(|d| d.insert_temp(egui::Id::unique("toolbar-test-left"), left));
                    }
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, theme);
            h.run_steps(2);
            let left = |h: &egui_kittest::Harness<'_, PhotocraftApp>| h.ctx.data(|d| d.get_temp::<f32>(egui::Id::unique("toolbar-test-left"))).unwrap_or(0.0);
            let single = left(&h);
            assert!(!h.state().ui.panels.toolbar_double, "{theme:?}");

            h.get_by_label("Toolbar").click();
            h.run_steps(2);
            assert!(h.state().ui.panels.toolbar_double, "{theme:?}");
            assert!(left(&h) > single + 20.0, "{theme:?}: the toolbar did not widen: {single} -> {}", left(&h));

            h.get_by_label("Toolbar").click();
            h.run_steps(2);
            assert!(!h.state().ui.panels.toolbar_double, "{theme:?}");
            assert_eq!(left(&h), single, "{theme:?}");
        }
    }

    /// #4653: every theme draws Edit Toolbar, Quick Mask and Screen Mode, not only Pro.
    #[test]
    fn every_theme_has_the_toolbar_controls() {
        for kind in crate::theme::ThemeKind::ALL {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h = egui_kittest::Harness::builder().with_size(vec2(800.0, 1400.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        toolbar(app, ui);
                    }
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, kind);
            h.run_steps(2);
            for label in ["Edit Toolbar…", "Edit in Quick Mask Mode  (Q)", "Change Screen Mode  (F)"] {
                assert!(h.query_by_label(label).is_some(), "{kind:?}: no {label:?} button");
            }
        }
    }

    // Regression test for #2241: Quick Mask and Screen Mode ignored the two-column toolbar and
    // stayed stacked in one column; in two columns they sit side by side.
    #[test]
    fn quick_mask_and_screen_mode_follow_the_toolbar_columns() {
        for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::Studio, crate::theme::ThemeKind::Classic] {
            let app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            let mut h = egui_kittest::Harness::builder().with_size(vec2(800.0, 1400.0)).build_ui_state(
                |ui, app: &mut PhotocraftApp| {
                    if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                        toolbar(app, ui);
                    }
                },
                app,
            );
            PhotocraftApp::setup_context(&h.ctx, theme);
            h.run_steps(2);
            let modes = |h: &egui_kittest::Harness<'_, PhotocraftApp>| {
                (h.get_by_label("Edit in Quick Mask Mode  (Q)").rect(), h.get_by_label("Change Screen Mode  (F)").rect())
            };
            let (quick, screen) = modes(&h);
            assert_eq!(quick.left(), screen.left(), "{theme:?}: one column stacks them");
            assert!(screen.top() >= quick.bottom(), "{theme:?}: Screen Mode below Quick Mask: {quick:?} {screen:?}");

            h.get_by_label("Toolbar").click();
            h.run_steps(2);
            assert!(h.state().ui.panels.toolbar_double, "{theme:?}");
            let (quick, screen) = modes(&h);
            assert_eq!(quick.top(), screen.top(), "{theme:?}: two columns put them on one row: {quick:?} {screen:?}");
            assert!(screen.left() >= quick.right(), "{theme:?}: Screen Mode beside Quick Mask: {quick:?} {screen:?}");
        }
    }

    fn click(h: &mut egui_kittest::Harness<'_, PhotocraftApp>, p: egui::Pos2) {
        h.hover_at(p);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
    }

    /// A tool's toolbar button: tool buttons have no label, so find it by slot order.
    fn tool_button(h: &egui_kittest::Harness<'_, PhotocraftApp>, tool: Tool) -> egui::Pos2 {
        let size = egui::Vec2::splat(if Tokens::get(&h.ctx).pro { 30.0 } else { 36.0 });
        let buttons: Vec<Rect> = h.ctx.viewport(|v| {
            v.prev_pass.widgets.layers().flat_map(|(_, w)| w.iter()).filter(|w| w.rect.size() == size && w.sense.senses_click()).map(|w| w.rect).collect()
        });
        let index = TOOL_SECTIONS.iter().flat_map(|section| section.iter()).position(|slot| slot.contains(&tool)).unwrap();
        buttons[index].center()
    }

    fn hand_button(h: &egui_kittest::Harness<'_, PhotocraftApp>) -> egui::Pos2 {
        tool_button(h, Tool::Hand)
    }

    fn toolbar_harness(app: PhotocraftApp) -> egui_kittest::Harness<'static, PhotocraftApp> {
        // 60 fps steps, so two clicks a few frames apart are a double-click.
        let mut h = egui_kittest::Harness::builder().with_size(vec2(800.0, 1400.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    toolbar(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(2);
        h
    }

    /// Double-clicking the Hand tool fits the image on screen (Photoshop); a single click only
    /// picks the tool.
    #[test]
    fn double_clicking_the_hand_tool_fits_on_screen() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.views[0].fit_pending = false;
        let mut h = toolbar_harness(app);
        let p = hand_button(&h);
        click(&mut h, p);
        assert_eq!(h.state().ui.tool, Tool::Hand);
        assert!(!h.state().ui.views[0].fit_pending, "a single click doesn't fit");
        // Past the double-click window, so the next two clicks are a fresh double-click.
        h.run_steps(40);
        click(&mut h, p);
        click(&mut h, p);
        assert_eq!(h.state().ui.tool, Tool::Hand);
        assert!(h.state().ui.views[0].fit_pending, "a double-click fits on screen");
    }

    /// With no document open the double-click still picks the tool and doesn't panic.
    #[test]
    fn double_clicking_the_hand_tool_without_a_document_is_harmless() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.tool = Tool::Move;
        let mut h = toolbar_harness(app);
        let p = hand_button(&h);
        click(&mut h, p);
        click(&mut h, p);
        assert_eq!(h.state().ui.tool, Tool::Hand);
        assert!(h.state().session.active().is_none());
    }

    /// Double-clicking the Zoom tool zooms to 100% (Photoshop); a single click only picks the tool.
    #[test]
    fn double_clicking_the_zoom_tool_zooms_to_actual_pixels() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
        app.ui.tool = Tool::Move;
        app.ui.views[0].fit_pending = true;
        app.ui.views[0].zoom = 0.25;
        let mut h = toolbar_harness(app);
        let p = tool_button(&h, Tool::Zoom);
        click(&mut h, p);
        assert_eq!(h.state().ui.tool, Tool::Zoom);
        assert_eq!(h.state().ui.views[0].zoom, 0.25, "a single click doesn't zoom");
        // Past the double-click window, so the next two clicks are a fresh double-click.
        h.run_steps(40);
        click(&mut h, p);
        click(&mut h, p);
        assert_eq!(h.state().ui.tool, Tool::Zoom);
        assert_eq!(h.state().ui.views[0].zoom, 1.0, "a double-click zooms to 100%");
        assert!(!h.state().ui.views[0].fit_pending, "100% replaces a pending fit");
    }

    /// With no document open the double-click still picks the tool and doesn't panic.
    #[test]
    fn double_clicking_the_zoom_tool_without_a_document_is_harmless() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.ui.tool = Tool::Move;
        let mut h = toolbar_harness(app);
        let p = tool_button(&h, Tool::Zoom);
        click(&mut h, p);
        click(&mut h, p);
        assert_eq!(h.state().ui.tool, Tool::Zoom);
        assert!(h.state().session.active().is_none());
    }

    /// A press and release with no pointer move before it (first click after focus, a pen or
    /// touch tap) picks the tool: the button doesn't need to have been hovered (#2603).
    #[test]
    fn a_click_without_a_prior_hover_picks_the_tool() {
        for tool in [Tool::Brush, Tool::Type] {
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.ui.tool = Tool::Move;
            let mut h = toolbar_harness(app);
            let p = tool_button(&h, tool);
            assert!(h.ctx.input(|i| i.pointer.latest_pos()).is_none(), "the pointer has not entered the window");
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE });
                h.run_steps(1);
            }
            assert_eq!(h.state().ui.tool, tool);
        }
    }
}

#[cfg(test)]
mod embedding_tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    fn whole_app(rail: bool, menu_bar: bool) -> egui_kittest::Harness<'static, PhotocraftApp> {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1280.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.ui.panels.rail = rail;
            app.ui.panels.menu_bar = menu_bar;
            app
        });
        h.run_steps(4);
        h
    }

    fn drawn(h: &egui_kittest::Harness<'_, PhotocraftApp>, panel: &str) -> bool {
        egui::containers::panel::PanelState::load(&h.ctx, panel_state_id(&h.ctx, panel)).is_some()
    }

    /// The rail is drawn inside the canvas panel, so its panel state is keyed under that panel's
    /// scope: find it by a button only the rail has.
    fn rail_drawn(h: &egui_kittest::Harness<'_, PhotocraftApp>) -> bool {
        h.query_by_label("Color & Swatches").is_some()
    }

    /// An app that embeds the UI can hide the icon rail and the menu bar; both are on by default.
    #[test]
    fn rail_and_menu_bar_follow_their_panel_flags() {
        let panels = crate::state::Panels::default();
        assert!(panels.rail && panels.menu_bar);

        let h = whole_app(true, true);
        assert!(rail_drawn(&h) && drawn(&h, "title_bar"));
        assert!(h.query_by_label("File").is_some());

        let h = whole_app(false, false);
        assert!(!rail_drawn(&h), "the icon rail is drawn");
        assert!(!drawn(&h, "title_bar"), "the title bar is drawn");
        assert!(h.query_by_label("File").is_none(), "the menus are drawn");
        assert!(crate::panel_docking::last_bodies(&h.ctx).iter().any(|(id, _)| id == "layers"), "the dock itself stays");
    }

    /// Settings saved before the flags existed load with both on.
    #[test]
    fn panels_without_the_flags_deserialize_with_both_on() {
        let mut v = serde_json::to_value(crate::state::Panels::default()).unwrap();
        let m = v.as_object_mut().unwrap();
        m.remove("rail");
        m.remove("menu_bar");
        let p: crate::state::Panels = serde_json::from_value(v).unwrap();
        assert!(p.rail && p.menu_bar);
    }
}

#[cfg(test)]
mod layer_drag_edge_scroll_tests {
    use super::*;

    #[test]
    fn inside_edge_scrolls_proportional_to_proximity() {
        let viewport = Rect::from_min_max(pos2(10.0, 30.0), pos2(310.0, 330.0));
        let point = |y| Some(pos2(100.0, y));
        let near_top = layer_drag_edge_scroll(point(33.0), viewport, true, 1.0 / 60.0);
        let far_top = layer_drag_edge_scroll(point(53.0), viewport, true, 1.0 / 60.0);
        let near_bottom = layer_drag_edge_scroll(point(327.0), viewport, true, 1.0 / 60.0);
        let far_bottom = layer_drag_edge_scroll(point(307.0), viewport, true, 1.0 / 60.0);
        assert!(near_top > far_top && far_top > 0.0, "approaching the top reveals earlier rows");
        assert!(near_bottom < far_bottom && far_bottom < 0.0, "approaching the bottom reveals later rows");
        assert!((near_top + near_bottom).abs() < 1e-5, "symmetric edge behavior");
        assert_eq!(layer_drag_edge_scroll(point(160.0), viewport, true, 1.0 / 60.0), 0.0);
    }

    #[test]
    fn outside_edges_scroll_in_drag_direction() {
        let viewport = Rect::from_min_max(pos2(10.0, 30.0), pos2(310.0, 330.0));
        let above = layer_drag_edge_scroll(Some(pos2(100.0, 20.0)), viewport, true, 1.0 / 60.0);
        let below = layer_drag_edge_scroll(Some(pos2(100.0, 335.0)), viewport, true, 1.0 / 60.0);
        assert!(above > 0.0, "dragging past the top continues scrolling upward");
        assert!(below < 0.0, "dragging past the bottom continues scrolling downward");
    }

    #[test]
    fn clamp_bounds_speed_outside_edges_and_across_frame_times() {
        let viewport = Rect::from_min_max(pos2(10.0, 30.0), pos2(310.0, 330.0));
        let at_top = layer_drag_edge_scroll(Some(pos2(100.0, 30.0)), viewport, true, 1.0 / 60.0);
        let past_top = layer_drag_edge_scroll(Some(pos2(100.0, 15.0)), viewport, true, 1.0 / 60.0);
        let far_past_top = layer_drag_edge_scroll(Some(pos2(100.0, -50.0)), viewport, true, 1.0 / 60.0);
        assert_eq!(past_top, at_top, "speed past top edge is clamped to maximum edge speed");
        assert_eq!(far_past_top, at_top, "speed far past top edge remains clamped");

        let at_bottom = layer_drag_edge_scroll(Some(pos2(100.0, 330.0)), viewport, true, 1.0 / 60.0);
        let past_bottom = layer_drag_edge_scroll(Some(pos2(100.0, 345.0)), viewport, true, 1.0 / 60.0);
        let far_past_bottom = layer_drag_edge_scroll(Some(pos2(100.0, 500.0)), viewport, true, 1.0 / 60.0);
        assert_eq!(past_bottom, at_bottom, "speed past bottom edge is clamped to maximum edge speed");
        assert_eq!(far_past_bottom, at_bottom, "speed far past bottom edge remains clamped");

        let active = Some(pos2(100.0, 325.0));
        let step = layer_drag_edge_scroll(active, viewport, true, 1.0 / 60.0);
        let twice = layer_drag_edge_scroll(active, viewport, true, 2.0 / 60.0);
        assert!((twice - step * 2.0).abs() < 1e-4, "time-based scrolling scales across refresh rates");
        assert!(layer_drag_edge_scroll(active, viewport, true, 0.5).abs() <= 30.0, "long frames have a bounded step");
    }

    #[test]
    fn zero_when_no_drag_or_outside_horizontal_bounds() {
        let viewport = Rect::from_min_max(pos2(10.0, 30.0), pos2(310.0, 330.0));
        let active = Some(pos2(100.0, 325.0));
        assert_eq!(layer_drag_edge_scroll(active, viewport, false, 1.0 / 60.0), 0.0);
        assert_eq!(layer_drag_edge_scroll(None, viewport, true, 1.0 / 60.0), 0.0);
        assert_eq!(layer_drag_edge_scroll(Some(pos2(9.0, 325.0)), viewport, true, 1.0 / 60.0), 0.0);
        assert_eq!(layer_drag_edge_scroll(Some(pos2(311.0, 325.0)), viewport, true, 1.0 / 60.0), 0.0);
        assert_eq!(layer_drag_edge_scroll(active, viewport, true, 0.0), 0.0);
    }

    #[test]
    fn short_viewports_keep_the_edge_zones_disjoint() {
        let viewport = Rect::from_min_max(pos2(0.0, 0.0), pos2(150.0, 40.0));
        assert!(layer_drag_edge_scroll(Some(pos2(10.0, 2.0)), viewport, true, 0.016) > 0.0);
        assert!(layer_drag_edge_scroll(Some(pos2(10.0, 38.0)), viewport, true, 0.016) < 0.0);
        assert_eq!(layer_drag_edge_scroll(Some(pos2(10.0, 20.0)), viewport, true, 0.016), 0.0);
    }
}

#[cfg(test)]
mod group_drag_selection_tests {
    use super::*;

    #[test]
    fn dragging_a_selected_layer_moves_the_complete_selection_into_a_group() {
        let a = LayerId(10);
        let b = LayerId(11);
        let group = LayerId(20);
        let payload = layer_drop_payload(a.0, group, "into", &[a, b]);
        assert_eq!(payload, json!({"layers": [10, 11], "target": 20, "position": "into"}));
        assert_eq!(payload.get("layer"), None, "batch drops must not also send a single layer");

        // Above/below use the same batch route; engine preserves the document stack order.
        assert_eq!(layer_drop_payload(b.0, group, "above", &[a, b])["position"], "above");
        assert_eq!(layer_drop_payload(b.0, group, "below", &[a, b])["position"], "below");
    }

    #[test]
    fn the_background_row_neither_moves_nor_takes_a_layer_below_it() {
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        let bg = s.active().unwrap().doc.layers[0].id;
        let a = LayerId(s.execute("layer.new.layer", json!({})).unwrap()["layer"].as_u64().unwrap());
        let doc = &s.active().unwrap().doc;
        let refused = |dragged: LayerId, target: LayerId, pos: &str, sel: &[LayerId], copy: bool| {
            background_drop_refused(doc, &layer_drop_payload(dragged.0, target, pos, sel), target, pos, copy)
        };
        assert!(refused(bg, a, "above", &[bg], false));
        assert!(refused(a, bg, "below", &[a], false));
        assert!(refused(a, bg, "below", &[a], true));
        assert!(refused(a, a, "above", &[bg, a], false), "the Background is among the dragged layers");
        assert!(!refused(a, bg, "above", &[a], false));
        assert!(!refused(bg, a, "above", &[bg], true), "⌥-dragging copies the Background above");
        // Without a Background every drop is fine.
        let mut s = photocraft_engine::Session::new();
        s.execute("file.new", json!({"width": 8, "height": 8})).unwrap();
        s.execute("layer.new.layerFromBackground", json!({})).unwrap();
        let l0 = s.active().unwrap().doc.layers[0].id;
        let doc = &s.active().unwrap().doc;
        assert!(!background_drop_refused(doc, &layer_drop_payload(l0.0, a, "above", &[l0]), a, "above", false));
    }

    #[test]
    fn dragging_unselected_or_singular_row_remains_a_single_layer_move() {
        let a = LayerId(10);
        let b = LayerId(11);
        let group = LayerId(20);
        assert_eq!(layer_drop_payload(9, group, "into", &[a, b]), json!({"layer": 9, "target": 20, "position": "into"}));
        assert_eq!(layer_drop_payload(a.0, group, "above", &[a]), json!({"layer": 10, "target": 20, "position": "above"}));
        assert_eq!(layer_drop_payload(a.0, group, "below", &[]), json!({"layer": 10, "target": 20, "position": "below"}));
    }

    #[test]
    fn layer_thumb_of_a_tall_document_fills_the_height() {
        let cell = Rect::from_min_size(pos2(10.0, 20.0), vec2(30.0, 30.0));
        let (r, uv) = layer_thumb_fit(cell, 100, 400);
        assert_eq!((r.top(), r.bottom()), (cell.top(), cell.bottom()));
        assert!((r.width() - 7.5).abs() < 1e-3, "{r:?}");
        assert!((uv.width() - 0.25).abs() < 1e-3 && (uv.height() - 1.0).abs() < 1e-3, "{uv:?}");
    }
}

#[cfg(test)]
mod opacity_row_layout_tests {
    use super::*;

    #[test]
    fn opacity_label_yields_to_controls_in_narrow_layers_panel() {
        let gap = 2.0;
        let label = 62.0;
        let (wide, visible) = opacity_row_layout(360.0, label, gap);
        assert!(visible);
        assert!(wide >= 100.0);
        // The default dock width keeps the English label (the Layers dock at its 250 px minimum).
        assert!(opacity_row_layout(255.0, 46.0, 8.0).1);
        let (medium, visible) = opacity_row_layout(180.0, label, gap);
        assert!(!visible, "the long label must not overlap the blend field");
        assert!(medium + LAYER_PCT_W + gap <= 180.0);
        let (narrow, visible) = opacity_row_layout(135.0, label, gap);
        assert!(!visible);
        assert!(narrow >= 40.0);
        assert!(narrow + LAYER_PCT_W + gap <= 135.0);
    }
}

#[cfg(test)]
mod clipping_indicator_tests;

#[cfg(test)]
mod mask_properties_tests;

#[cfg(test)]
mod compact_visual_regressions;

#[cfg(test)]
mod properties_target_regressions {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    #[test]
    fn pending_transform_arithmetic_does_not_apply_to_another_document_with_the_same_layer_id() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width":200,"height":200,"background":"transparent"})).unwrap();
        app.run("select.rect", json!({"x":10,"y":20,"width":100,"height":50})).unwrap();
        app.run("edit.fill", json!({"color":"#ff0000"})).unwrap();
        app.run("select.deselect", json!({})).unwrap();
        let layer = app.session.active().unwrap().active_layer.unwrap();
        let duplicate = (*app.session.active().unwrap().doc).clone();
        let second = app.session.add_document(duplicate, None);
        app.session.active_mut().unwrap().active_layer = Some(layer);
        assert!(app.session.set_active(0));
        let mut ready = false;
        let mut h = Harness::builder().with_size(vec2(360.0, 650.0)).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if !ready {
                    PhotocraftApp::setup_context(ui.ctx(), theme::ThemeKind::ProMedium);
                    ready = true;
                    return;
                }
                properties_body(app, ui);
            },
            app,
        );
        h.run_steps(3);
        h.get_all_by_role(egui::accesskit::Role::SpinButton).next().expect("transform width").click();
        h.run_steps(1);
        for c in "50/2".chars() {
            h.event(egui::Event::Text(c.to_string()));
            h.run_steps(1);
        }
        let before = h.state().session.documents()[second].doc.layer(layer).unwrap().surface().unwrap().content_bounds();
        assert!(h.state_mut().session.set_active(second));
        h.run_steps(2);
        h.key_press(egui::Key::Enter);
        h.run_steps(2);
        let after = h.state().session.active().unwrap().doc.layer(layer).unwrap().surface().unwrap().content_bounds();
        assert_eq!(after, before, "pending width belongs to the old document/layer");
    }
}

#[cfg(test)]
mod navigator_animated_zoom_tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn animated_zoom_off_navigator_buttons_jump_without_a_transition() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
        app.run("prefs.set", json!({"values": {"general.animatedZoom": false}})).unwrap();
        app.sync_views();
        app.ui.views[0].zoom = 1.0;
        app.ui.views[0].fit_pending = false;
        let mut h = egui_kittest::Harness::builder().with_size(vec2(430.0, 460.0)).build_ui_state(
            |ui, app: &mut PhotocraftApp| {
                if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                    navigator(app, ui);
                }
            },
            app,
        );
        PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
        h.run_steps(4);
        for (label, direction) in [("Zoom In", 1), ("Zoom Out", -1), ("Zoom Out", -1)] {
            let before = &h.state().ui.views[0];
            let zoom = crate::zoom_levels::step(before.zoom, direction, before.doc_size);
            h.get_by_label(label).click();
            h.run_steps(2);
            let view = &h.state().ui.views[0];
            assert_eq!(view.zoom, zoom, "{label}");
            assert_eq!(view.display_camera, Some((view.zoom, view.center)), "{label}");
            assert!(!view.zoom_animating && view.zoom_anchor.is_none() && view.zoom_transition.is_none(), "{label}");
            assert!(!view.fit_pending, "{label}");
        }
    }
}

#[cfg(test)]
mod history_panel_select_tests;
#[cfg(test)]
mod layer_click_type_commit_tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};

    fn harness(kind: theme::ThemeKind) -> (Harness<'static, PhotocraftApp>, LayerId, LayerId) {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
        let text = LayerId(app.run("type.create", json!({"text": "Before", "size": 24, "x": 20, "y": 80})).unwrap()["layer"].as_u64().unwrap());
        app.run("layer.setProps", json!({"layer": text.0, "name": "Edited type"})).unwrap();
        let other = LayerId(app.run("layer.new.layer", json!({"name": "Other pixels"})).unwrap()["layer"].as_u64().unwrap());
        app.run("layer.select", json!({"layer": text.0})).unwrap();
        crate::type_tool::edit_active(&mut app).unwrap();
        let session = app.ui.text_edit.as_ref().unwrap().session.clone();
        app.run("type.edit", json!({"layer": text.0, "text": "Preserved edit", "coalesce": session})).unwrap();
        let mut ready = false;
        let mut h = Harness::builder().with_size(vec2(360.0, 650.0)).with_step_dt(1.0 / 60.0).build_ui_state(
            move |ui, app: &mut PhotocraftApp| {
                if !ready {
                    PhotocraftApp::setup_context(ui.ctx(), kind);
                    ready = true;
                    return;
                }
                layers(app, ui);
            },
            app,
        );
        h.run_steps(4);
        (h, text, other)
    }

    fn click(h: &mut Harness<'_, PhotocraftApp>, at: Pos2, modifiers: egui::Modifiers) {
        h.event(egui::Event::ModifiersChanged(modifiers));
        h.hover_at(at);
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers });
            h.run_steps(1);
        }
    }

    fn preserved(app: &PhotocraftApp, id: LayerId) {
        let layer = app.session.active().unwrap().doc.layer(id).unwrap();
        let LayerContent::Text(text) = &layer.content else { panic!("type must remain editable") };
        assert_eq!(text.text, "Preserved edit");
    }

    #[test]
    fn layer_name_and_thumbnail_clicks_commit_same_or_other_type_selection() {
        for kind in theme::ThemeKind::ALL {
            for same in [true, false] {
                for thumbnail in [true, false] {
                    let (mut h, text, other) = harness(kind);
                    let label = if same { "Edited type" } else { "Other pixels" };
                    let row = h.get_by_label(label).rect();
                    let at = pos2(row.left() + if thumbnail { 43.0 } else { 120.0 }, row.center().y);
                    click(&mut h, at, egui::Modifiers::NONE);
                    assert!(h.state().ui.text_edit.is_none(), "{kind:?}, same={same}, thumbnail={thumbnail}");
                    assert_eq!(h.state().session.active().unwrap().active_layer, Some(if same { text } else { other }));
                    preserved(h.state(), text);
                }
            }
        }
    }

    #[test]
    fn layer_click_commits_without_breaking_modifier_selection_or_double_click_editing() {
        for kind in theme::ThemeKind::ALL {
            let (mut h, text, other) = harness(kind);
            let row = h.get_by_label("Other pixels").rect();
            click(&mut h, pos2(row.left() + 120.0, row.center().y), egui::Modifiers { command: true, ..Default::default() });
            assert!(h.state().ui.text_edit.is_none());
            let selected = h.state().session.active().unwrap().selected_layers();
            assert!(selected.contains(&text) && selected.contains(&other));
            preserved(h.state(), text);

            let (mut h, text, _) = harness(kind);
            let row = h.get_by_label("Edited type").rect();
            let thumb = pos2(row.left() + 43.0, row.center().y);
            click(&mut h, thumb, egui::Modifiers::NONE);
            assert!(h.state().ui.text_edit.is_none());
            click(&mut h, thumb, egui::Modifiers::NONE);
            assert_eq!(h.state().ui.text_edit.as_ref().map(|ed| ed.layer), Some(text.0));
            preserved(h.state(), text);
        }
    }

    #[test]
    #[ignore = "offscreen visual evidence for #5584"]
    fn layer_click_synthetic_visual_evidence() {
        let (h, text, _) = harness(theme::ThemeKind::ProMedium);
        let mut app = h.into_state();
        app.ui.tool = Tool::Type;
        let mut h = Harness::builder().with_size(vec2(1280.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, theme::ThemeKind::ProMedium);
            app
        });
        h.run_steps(4);
        assert!(h.state().ui.text_edit.is_some());
        h.render().unwrap().save("5584-before.png").unwrap();
        let row = h.get_by_label("Other pixels").rect();
        click(&mut h, pos2(row.left() + 120.0, row.center().y), egui::Modifiers::NONE);
        assert!(h.state().ui.text_edit.is_none());
        preserved(h.state(), text);
        h.run_steps(4);
        h.render().unwrap().save("5584-after.png").unwrap();
    }
}

#[cfg(test)]
mod sampler_readout_tests {
    use super::*;

    /// Composited once per revision: repeated frames share the readouts; adding a sampler or an
    /// edit recomputes them (#4528).
    #[test]
    fn sampler_readouts_are_reused_until_the_document_changes() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 32, "height": 32, "background": "white"})).unwrap();
        app.run("view.colorSamplers.add", json!({"x": 4, "y": 4})).unwrap();
        let a = sampler_readouts(&mut app);
        assert_eq!(a.len(), 1);
        assert_eq!(a[0]["rgb"], json!([255, 255, 255]));
        assert!(std::sync::Arc::ptr_eq(&a, &sampler_readouts(&mut app)), "the next frame reuses them");
        app.run("edit.fill", json!({"color": "#ff0000"})).unwrap();
        let b = sampler_readouts(&mut app);
        assert!(!std::sync::Arc::ptr_eq(&a, &b));
        assert_eq!(b[0]["rgb"], json!([255, 0, 0]), "an edit shows");
        app.run("view.colorSamplers.add", json!({"x": 8, "y": 8})).unwrap();
        assert_eq!(sampler_readouts(&mut app).len(), 2, "so does a new sampler");
    }
}

#[cfg(test)]
mod panel_state_migration_tests {
    use super::{migrate_panel_state, panel_state_id};
    use egui::containers::panel::PanelState;
    use egui::{Context, Id, Rect, pos2, vec2};

    fn state(width: f32) -> PanelState {
        PanelState { outer_rect: Rect::from_min_size(pos2(0.0, 0.0), vec2(width, 600.0)) }
    }

    /// 0.36 saved `dock` under `Id::new("dock")`; 0.37 reads a parent-scoped key.
    #[test]
    fn the_parent_scoped_key_matches_the_panel_egui_shows() {
        let ctx = Context::default();
        let mut output = ctx.run_ui(Default::default(), |ui| {
            egui::Panel::right("dock").exact_size(410.0).show(ui, |ui| ui.set_min_width(ui.available_width()));
        });
        output.textures_delta.clear();
        let key = panel_state_id(&ctx, "dock");
        assert_eq!(key, ctx.viewport_id().root_ui_id().with("dock"));
        let panel = PanelState::load(&ctx, key).expect("the panel persisted under its parent-scoped id");
        assert_eq!(panel.size().x, 410.0);
        assert_ne!(key, Id::unique("dock"), "the old persisted key is a different id");
    }

    #[test]
    fn old_panel_widths_move_to_the_new_key_once() {
        let ctx = Context::default();
        let old = Id::unique("dock");
        ctx.data_mut(|d| d.insert_persisted(old, state(410.0)));
        migrate_panel_state(&ctx);
        let new = panel_state_id(&ctx, "dock");
        assert_eq!(PanelState::load(&ctx, new).map(|p| p.size().x), Some(410.0));
        assert_eq!(PanelState::load(&ctx, old).map(|p| p.size().x), Some(410.0), "0.36 rollback still reads it");

        // An existing 0.37 value wins, in a partial migration or on a later run.
        ctx.data_mut(|d| d.insert_persisted(new, state(290.0)));
        ctx.data_mut(|d| d.insert_temp(Id::unique("photocraft-panel-state-migrated"), false));
        migrate_panel_state(&ctx);
        assert_eq!(PanelState::load(&ctx, new).map(|p| p.size().x), Some(290.0));
        assert_eq!(PanelState::load(&ctx, old).map(|p| p.size().x), Some(410.0));
    }

    /// 0.37 resize → 0.36 rollback resize → upgrade must keep the newer 0.36 width, not the
    /// stale 0.37 one (the old key stays the only one a 0.36 build writes).
    #[test]
    fn a_0_36_rollback_resize_after_0_37_is_picked_up() {
        use super::remember_panel_state;
        let ctx = Context::default();
        let old = Id::unique("dock");
        let new = panel_state_id(&ctx, "dock");
        ctx.data_mut(|d| d.insert_persisted(old, state(410.0)));
        migrate_panel_state(&ctx);
        // This 0.37 session resized the dock; the pass-end stamp records it.
        ctx.data_mut(|d| d.insert_persisted(new, state(290.0)));
        remember_panel_state(&ctx);
        // A 0.36 build then resized the same dock (it writes only the old key).
        ctx.data_mut(|d| d.insert_persisted(old, state(330.0)));
        ctx.data_mut(|d| d.insert_temp(Id::unique("photocraft-panel-state-migrated"), false));
        migrate_panel_state(&ctx);
        assert_eq!(PanelState::load(&ctx, new).map(|p| p.size().x), Some(330.0), "the newer 0.36 width wins");
        assert_eq!(PanelState::load(&ctx, old).map(|p| p.size().x), Some(330.0), "and stays for a rollback");
    }

    /// A 0.37 resize after the migration is not overwritten by the unchanged old key.
    #[test]
    fn a_0_37_resize_after_the_migration_is_kept() {
        use super::remember_panel_state;
        let ctx = Context::default();
        let old = Id::unique("dock");
        let new = panel_state_id(&ctx, "dock");
        ctx.data_mut(|d| d.insert_persisted(old, state(410.0)));
        migrate_panel_state(&ctx);
        ctx.data_mut(|d| d.insert_persisted(new, state(290.0)));
        remember_panel_state(&ctx);
        ctx.data_mut(|d| d.insert_temp(Id::unique("photocraft-panel-state-migrated"), false));
        migrate_panel_state(&ctx);
        assert_eq!(PanelState::load(&ctx, new).map(|p| p.size().x), Some(290.0));
    }
}
