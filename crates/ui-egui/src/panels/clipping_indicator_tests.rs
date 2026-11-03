//! Clipping marker orientation and the Alt-click gesture that reveals it.

use egui::{Event, Modifiers, PointerButton, pos2, vec2};
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable;
use serde_json::json;

use crate::PhotocraftApp;

#[test]
fn clipping_indicator_points_down_inside_the_existing_row_slot() {
    let ctx = egui::Context::default();
    let at = pos2(100.0, 100.0);
    let mut output = ctx.run_ui(Default::default(), |ui| super::paint_clipping_indicator(ui.painter(), at, egui::Color32::LIGHT_GRAY));
    output.textures_delta.clear();
    let paths: Vec<_> = output.shapes.iter().filter_map(|shape| if let egui::Shape::Path(path) = &shape.shape { Some(path) } else { None }).collect();
    assert_eq!(paths.len(), 2, "the marker has a bent shaft and an arrowhead, independent of font glyphs");
    let head = paths.iter().find(|path| path.points.len() == 3 && path.points[0].y == path.points[2].y).unwrap();
    assert!(head.points[1].y > head.points[0].y && head.points[1].y > head.points[2].y, "the arrow points down toward the clipping base");
    let slot = egui::Rect::from_min_max(at + vec2(0.0, -5.0), at + vec2(12.0, 5.0));
    assert!(paths.iter().flat_map(|path| &path.points).all(|point| slot.contains(*point)), "the marker does not shift or overlap the thumbnail");
}

/// Build a themed Layers panel for clipping-marker and pointer-gesture checks.
fn harness(theme: crate::theme::ThemeKind, width: f32, scale: f32, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("prefs.set", json!({"path": "interface.language", "value": "en"})).unwrap();
    session.execute("file.new", json!({"width": 64, "height": 64, "background": "white"})).unwrap();
    let upper = session.execute("layer.newFillLayer.gradient", json!({"from": "#235b49", "to": "#a7d4ab", "angle": 90})).unwrap()["layer"].as_u64().unwrap();
    session.execute("layer.setProps", json!({"layer": upper, "name": "Gradient Fill 1"})).unwrap();
    let builder = Harness::builder().with_size(vec2(width, 250.0)).with_pixels_per_point(scale);
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::i18n::with_language(crate::i18n::Lang::EN, || {
                app.ppp = ui.ctx().pixels_per_point();
                ui.heading("Layers");
                super::layers(app, ui);
            });
        },
        PhotocraftApp::new(session, Default::default()),
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h
}

/// Toggle clipping through a real Alt-click at the boundary between the two layer rows.
fn alt_click_line(h: &mut Harness<'_, PhotocraftApp>) {
    let rows = crate::layer_row_ui::recorded(&h.ctx);
    assert_eq!(rows.len(), 2);
    let p = pos2(rows[1].row.left() + 100.0, rows[1].row.top());
    h.event(Event::PointerMoved(p));
    h.event(Event::ModifiersChanged(Modifiers::ALT));
    h.step();
    for pressed in [true, false] {
        h.event(Event::PointerButton { pos: p, button: PointerButton::Primary, pressed, modifiers: Modifiers::ALT });
        h.step();
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));
    h.event(Event::PointerMoved(pos2(0.0, 0.0)));
    h.run_steps(2);
}

#[test]
fn alt_click_clipping_still_creates_releases_and_undoes_without_moving_rows() {
    let mut h = harness(crate::theme::ThemeKind::default(), 420.0, 1.0, false);
    let active = h.state().session.active().unwrap().active_layer.unwrap();
    let before = crate::layer_row_ui::recorded(&h.ctx);
    alt_click_line(&mut h);
    assert!(h.state().session.active().unwrap().doc.layer(active).unwrap().clipped);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(active));
    let after = crate::layer_row_ui::recorded(&h.ctx);
    assert!(before.iter().zip(&after).all(|(before, after)| before.row == after.row));
    h.state_mut().run("edit.undo", json!({})).unwrap();
    assert!(!h.state().session.active().unwrap().doc.layer(active).unwrap().clipped);
    h.state_mut().run("edit.redo", json!({})).unwrap();
    assert!(h.state().session.active().unwrap().doc.layer(active).unwrap().clipped);
    alt_click_line(&mut h);
    assert!(!h.state().session.active().unwrap().doc.layer(active).unwrap().clipped);
}

/// Open one real adjustment editor inside a 250×160 scrollable Properties panel.
fn adjustment_harness(kind: &str, depth: u32, theme: crate::theme::ThemeKind, scale: f32, gpu: bool) -> Harness<'static, PhotocraftApp> {
    let mut session = photocraft_engine::Session::new();
    session.execute("file.new", json!({"width": 16, "height": 8, "depth": depth})).unwrap();
    session.execute("edit.fill", json!({"color": "#b04020"})).unwrap();
    session.execute("layer.new.layer", json!({})).unwrap();
    session.execute("select.rect", json!({"x": 8, "y": 0, "width": 8, "height": 8})).unwrap();
    session.execute("edit.fill", json!({"color": "#208040"})).unwrap();
    session.execute("select.deselect", json!({})).unwrap();
    let mut app = PhotocraftApp::new(session, Default::default());
    app.ui.dock_tabs.properties = 1;
    let builder = Harness::builder().with_size(vec2(250.0, 160.0)).with_pixels_per_point(scale);
    let builder = if gpu { builder.wgpu() } else { builder };
    let mut h = builder.build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|fonts| fonts.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            crate::i18n::with_language(crate::i18n::Lang::EN, || {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    if app.ui.dock_tabs.properties == 1 {
                        super::adjustments_grid(app, ui);
                    } else {
                        super::properties_body(app, ui);
                    }
                });
            });
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, theme);
    h.run_steps(4);
    h.get_by_label(kind).click();
    h.run_steps(4);
    h
}

/// Reach a control through real wheel input without enlarging the compact panel.
fn scroll_to_adjustment_control(h: &mut Harness<'_, PhotocraftApp>, label: &str) -> egui::Rect {
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(250.0, 160.0));
    for _ in 0..32 {
        if let Some(rect) = h.query_all_by_label(label).map(|node| node.rect()).find(|rect| viewport.contains_rect(*rect)) {
            return rect;
        }
        // The label column stays clear of the Color Lookup picker, which consumes wheel input.
        h.hover_at(pos2(16.0, 80.0));
        h.event(Event::MouseWheel {
            source: egui::MouseWheelSource::Trackpad,
            unit: egui::MouseWheelUnit::Point,
            delta: vec2(0.0, -80.0),
            phase: egui::TouchPhase::Move,
            modifiers: Modifiers::NONE,
        });
        h.run_steps(4);
    }
    panic!("adjustment control {label:?} was not reachable in the compact viewport");
}

/// Scope starts on-screen; the editor and visibility remain reachable by scrolling.
#[test]
fn adjustment_scope_control_fits_compact_properties_before_scrolling() {
    for theme in crate::theme::ThemeKind::ALL {
        for scale in [1.0, 1.5, 2.0] {
            for (kind, editor_control) in
                [("Hue/Saturation", "Colorize"), ("Curves", "Edit points to modify the curve"), ("Selective Color", "Relative"), ("Color Lookup", "Dither")]
            {
                let mut h = adjustment_harness(kind, 8, theme, scale, false);
                let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(250.0, 160.0));
                let rect = h.get_by_label("Clip to layer below").rect();
                assert!(viewport.contains_rect(rect), "{theme:?}, {scale}x, {kind}: {rect:?}");
                let editor = scroll_to_adjustment_control(&mut h, editor_control);
                assert!(viewport.contains_rect(editor), "{theme:?}, {scale}x, {kind}: editor {editor:?}");
                let visibility = scroll_to_adjustment_control(&mut h, "Adjustment visible");
                assert!(viewport.contains_rect(visibility), "{theme:?}, {scale}x, {kind}: visibility {visibility:?}");
            }
        }
    }
}

/// Read a document pixel on the shared scanline used by the clipping-output assertions.
fn sample(h: &mut Harness<'_, PhotocraftApp>, x: i32) -> Vec<f32> {
    serde_json::from_value(h.state_mut().run("document.pixel", json!({"x": x, "y": 4})).unwrap()).unwrap()
}

#[test]
fn panel_adjustment_clips_to_selected_layer_and_scope_undoes_at_all_depths() {
    for depth in [8, 16, 32] {
        let mut h = adjustment_harness("Hue/Saturation", depth, crate::theme::ThemeKind::Pro, 1.0, false);
        let id = h.state().session.active().unwrap().active_layer.unwrap();
        h.state_mut().run("layer.setAdjustment", json!({"layer": id.0, "saturation": -100})).unwrap();
        h.run_steps(4);
        let all_layers = sample(&mut h, 2);
        assert!((all_layers[0] - all_layers[1]).abs() < 0.01, "unclipped adjustment desaturates the lower layer");
        let adjusted_target = sample(&mut h, 12);
        let steps = h.state().session.active().unwrap().history.past_len();
        h.get_by_label("Clip to layer below").click();
        h.run_steps(4);
        assert!(h.state().session.active().unwrap().doc.layer(id).unwrap().clipped);
        assert_eq!(h.state().session.active().unwrap().history.past_len(), steps + 1);
        let lower_layer = sample(&mut h, 2);
        assert!(lower_layer[0] - lower_layer[1] > 0.2, "clipping restores the lower layer's colour");
        assert!(sample(&mut h, 12).iter().zip(&adjusted_target).all(|(a, b)| (a - b).abs() <= 1.0 / 255.0), "the selected layer stays adjusted");
        h.state_mut().run("edit.undo", json!({})).unwrap();
        assert_eq!(sample(&mut h, 2), all_layers);
        h.state_mut().run("edit.redo", json!({})).unwrap();
        assert_eq!(sample(&mut h, 2), lower_layer);
        h.run_steps(4);
        h.get_by_label("Clip to layer below").click();
        h.run_steps(4);
        assert!(!h.state().session.active().unwrap().doc.layer(id).unwrap().clipped);
        assert_eq!(sample(&mut h, 2), all_layers, "scope can be released through the same control");
    }
}

#[test]
#[ignore = "writes synthetic compact adjustment-panel screenshots to log/adjustment-scope"]
fn write_adjustment_scope_preview() {
    let phase = std::env::var("PHOTOCRAFT_EVIDENCE_PHASE").unwrap_or_else(|_| "after".into());
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../log/adjustment-scope");
    std::fs::create_dir_all(&output).unwrap();
    for theme in [crate::theme::ThemeKind::Pro, crate::theme::ThemeKind::Studio] {
        let mut h = adjustment_harness("Hue/Saturation", 8, theme, 2.0, true);
        // egui 0.37 completes screenshots through a one-shot callback.
        let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
        let slot = captured.clone();
        h.ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
            if let Ok(mut slot) = slot.lock() {
                *slot = Some(image);
            }
        })));
        h.step();
        h.step();
        let image = captured.lock().ok().and_then(|mut c| c.take()).expect("the kittest renderer completes the screenshot callback");
        let rgba = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
        let image = photocraft_codecs::Image::from_u8(image.size[0] as u32, image.size[1] as u32, photocraft_codecs::ChannelLayout::Rgba, rgba).unwrap();
        let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
        std::fs::write(output.join(format!("{phase}-{}.png", theme.id())), png).unwrap();
    }
}

#[test]
#[ignore = "writes synthetic English Layers-panel screenshots for before/after inspection"]
fn write_clipping_indicator_preview() {
    let _gpu = photocraft_testkit::gpu_lock();
    let output = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../plan/evidence/clipping");
    std::fs::create_dir_all(&output).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        for width in [310.0, 420.0] {
            for scale in [1.0, 2.0] {
                let mut h = harness(theme, width, scale, true);
                alt_click_line(&mut h);
                // egui 0.37 completes screenshots through a one-shot callback.
                let captured = std::sync::Arc::new(std::sync::Mutex::new(None));
                let slot = captured.clone();
                h.ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::ScreenshotCallback::new(move |image| {
                    if let Ok(mut slot) = slot.lock() {
                        *slot = Some(image);
                    }
                })));
                h.step();
                h.step();
                let image = captured.lock().ok().and_then(|mut c| c.take()).expect("the kittest renderer completes the screenshot callback");
                let rgba = image.pixels.iter().flat_map(|pixel| pixel.to_array()).collect();
                let image =
                    photocraft_codecs::Image::from_u8(image.size[0] as u32, image.size[1] as u32, photocraft_codecs::ChannelLayout::Rgba, rgba).unwrap();
                let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
                std::fs::write(output.join(format!("{}-{width:.0}-{scale:.0}x.png", theme.id())), png).unwrap();
            }
        }
    }
}
