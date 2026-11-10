use egui::Modifiers;
use photocraft_geom::Rect;
use serde_json::json;

use crate::canvas::{ToolEvent, tool_event};
use crate::state::Tool;
use crate::{PhotocraftApp, Services};

const RED: [f32; 4] = [1.0, 0.0, 0.0, 1.0];

/// Select the Eyedropper and target a fully revealed mask on a red pixel layer or an Invert adjustment above it.
fn app_with_mask(adjustment: bool) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Services::default());
    app.run("file.new", json!({"width": 8, "height": 8, "background": "transparent"})).unwrap();
    app.session
        .edit("Paint red", |doc, _| {
            let surface = doc.layers[0].surface_mut().unwrap();
            surface.fill_rect(Rect::new(0, 0, 8, 8), &photocraft_raster::from_rgba(&surface.format(), RED));
            Ok(())
        })
        .unwrap();
    let add = if adjustment { "layer.newAdjustmentLayer.invert" } else { "layer.layerMask.revealAll" };
    app.run(add, json!({})).unwrap();
    assert!(app.ui.mask_target);
    app.ui.tool = Tool::Eyedropper;
    app
}

fn paint_mask(app: &mut PhotocraftApp, rect: Rect, gray: f32) {
    let layer = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("Paint mask", |doc, _| {
            doc.layer_mut(layer).unwrap().mask.as_mut().unwrap().surface.fill_rect(rect, &[gray]);
            Ok(())
        })
        .unwrap();
}

fn click(app: &mut PhotocraftApp, x: f64, y: f64, mods: Modifiers) {
    tool_event(app, ToolEvent::Down { x, y, pressure: 1.0 }, mods);
    tool_event(app, ToolEvent::Up { x, y }, mods);
}

fn gray(value: f32) -> [f32; 4] {
    [value, value, value, 1.0]
}

/// Both measured modes pick white and mid-grey from the selected mask on either layer kind.
#[test]
fn targeted_pixel_and_adjustment_masks_supply_point_samples() {
    for adjustment in [false, true] {
        let mut app = app_with_mask(adjustment);
        paint_mask(&mut app, Rect::new(2, 2, 3, 3), 0.5);
        for mode in ["current", "currentAndBelow"] {
            app.ui.tool_options.eyedropper_sample = mode.into();
            app.run("tools.setColors", json!({"foreground": "#222222"})).unwrap();
            click(&mut app, 1.5, 1.5, Modifiers::NONE);
            assert_eq!(app.session.tools.foreground, gray(1.0), "adjustment={adjustment} {mode}");
            click(&mut app, 2.5, 2.5, Modifiers::NONE);
            assert_eq!(app.session.tools.foreground, gray(128.0 / 255.0), "adjustment={adjustment} {mode}");
        }
    }
}

/// The average at the corner counts only four in-canvas mask pixels and Alt writes background.
#[test]
fn mask_average_clips_at_canvas_edge_and_alt_sets_background() {
    let mut app = app_with_mask(false);
    paint_mask(&mut app, Rect::new(0, 0, 1, 2), 0.0);
    app.ui.tool_options.eyedropper_sample = "currentAndBelow".into();
    app.ui.tool_options.eyedropper_size = 3;
    app.run("tools.setColors", json!({"foreground": [0.2, 0.2, 0.2, 1.0], "background": [0.8, 0.8, 0.8, 1.0]})).unwrap();
    let foreground = app.session.tools.foreground;
    click(&mut app, 0.5, 0.5, Modifiers::ALT);
    assert_eq!(app.session.tools.background, gray(0.5));
    assert_eq!(app.session.tools.foreground, foreground);
}

/// Composite modes use the image, and selecting contents restores the same sampling as before.
#[test]
fn composite_modes_and_pixel_targets_still_sample_the_image() {
    let mut app = app_with_mask(true);
    for mode in ["all", "allNoAdjustments", "currentAndBelowNoAdjustments"] {
        app.ui.tool_options.eyedropper_sample = mode.into();
        let sample = app.run("document.sampleColor", json!({"x": 4.5, "y": 4.5, "sampleLayer": mode})).unwrap();
        let [r, g, b, _]: [f32; 4] = serde_json::from_value(sample).unwrap();
        click(&mut app, 4.5, 4.5, Modifiers::NONE);
        assert_eq!(app.session.tools.foreground, gray(photocraft_color::convert::rgb_to_gray([r, g, b])), "{mode}");
    }
    // An adjustment's content thumbnail can still route painting to its mask. That is not an
    // explicit Eyedropper target.
    app.run("channel.target.composite", json!({})).unwrap();
    assert!(!app.ui.mask_target);
    app.ui.tool_options.eyedropper_sample = "current".into();
    let before = app.session.tools.foreground;
    click(&mut app, 4.5, 4.5, Modifiers::NONE);
    assert_eq!(app.session.tools.foreground, before, "the adjustment alone has no image pixels");

    let mut app = app_with_mask(false);
    app.run("channel.target.composite", json!({})).unwrap();
    for mode in ["current", "currentAndBelow"] {
        app.ui.tool_options.eyedropper_sample = mode.into();
        click(&mut app, 4.5, 4.5, Modifiers::NONE);
        assert_eq!(app.session.tools.foreground, RED, "{mode}");
    }
}

/// A white raster mask beside another selected target cannot take over that target's sample.
#[test]
fn vector_alpha_quick_mask_and_temporary_samples_keep_their_source() {
    let red_gray = gray(photocraft_color::convert::rgb_to_gray([1.0, 0.0, 0.0]));
    let mut app = app_with_mask(false);
    app.ui.tool_options.eyedropper_sample = "current".into();
    app.ui.tool = Tool::Brush;
    click(&mut app, 4.5, 4.5, Modifiers::ALT);
    assert_eq!(app.session.tools.foreground, red_gray, "the brush's Alt-click still samples the image");

    app.ui.tool = Tool::Eyedropper;
    app.run("layer.vectorMask.revealAll", json!({})).unwrap();
    assert!(app.ui.vector_mask_target);
    click(&mut app, 4.5, 4.5, Modifiers::NONE);
    assert_eq!(app.session.tools.foreground, RED, "a vector target does not pick the raster mask");

    app.run("channel.new", json!({"name": "Alpha"})).unwrap();
    app.run("channel.target", json!({"channel": {"alpha": 0}})).unwrap();
    click(&mut app, 4.5, 4.5, Modifiers::NONE);
    assert_eq!(app.session.tools.foreground, red_gray, "an alpha target does not pick the raster mask");

    app.run("channel.target.layerMask", json!({})).unwrap();
    app.run("select.editInQuickMaskMode", json!({"on": true})).unwrap();
    click(&mut app, 4.5, 4.5, Modifiers::NONE);
    assert_eq!(app.session.tools.foreground, red_gray, "Quick Mask does not pick the raster mask");
}

/// A smart filter's mask keeps its image-based sample even when the same layer has a raster mask.
#[test]
fn filter_mask_target_keeps_the_image_based_sample() {
    let mut app = app_with_mask(false);
    app.background_jobs = false;
    app.run("channel.target.composite", json!({})).unwrap();
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("filter.blur.gaussianBlur", json!({"radius": 1})).unwrap();
    app.run("layer.layerMask.revealAll", json!({})).unwrap();
    let layer = app.session.active().unwrap().active_layer.unwrap();
    assert!(app.session.active().unwrap().doc.layer(layer).unwrap().mask.is_some());
    app.select_filter_mask_target(Some(layer.0));
    assert!(app.filter_mask_targeted());
    app.ui.tool_options.eyedropper_sample = "current".into();
    let sample = app.run("document.sampleColor", json!({"x": 4.5, "y": 4.5, "sampleLayer": "current"})).unwrap();
    let [r, g, b, a]: [f32; 4] = serde_json::from_value(sample).unwrap();
    let want = gray(photocraft_color::convert::rgb_to_gray([r, g, b]));
    assert!(a > 0.0 && want != gray(1.0), "the image and layer mask are distinct");
    click(&mut app, 4.5, 4.5, Modifiers::NONE);
    assert_eq!(app.session.tools.foreground, want);
}

/// Out-of-canvas points and invalid sample options leave both swatches intact.
#[test]
fn invalid_mask_samples_leave_both_swatches_unchanged() {
    let mut app = app_with_mask(false);
    app.ui.tool_options.eyedropper_sample = "current".into();
    app.run("tools.setColors", json!({"foreground": [0.2, 0.2, 0.2, 1.0], "background": [0.8, 0.8, 0.8, 1.0]})).unwrap();
    let before = (app.session.tools.foreground, app.session.tools.background);
    let assert_unchanged = |app: &PhotocraftApp| assert_eq!((app.session.tools.foreground, app.session.tools.background), before);
    for (x, y) in [(-0.1, 1.0), (1.0, 8.0), (f64::NAN, 1.0), (1.0, f64::INFINITY)] {
        for mods in [Modifiers::NONE, Modifiers::ALT] {
            click(&mut app, x, y, mods);
            assert_unchanged(&app);
        }
    }
    app.ui.tool_options.eyedropper_size = 2;
    click(&mut app, 1.0, 1.0, Modifiers::NONE);
    assert_unchanged(&app);
    app.ui.tool_options.eyedropper_size = 1;
    app.ui.tool_options.eyedropper_sample = "invalid".into();
    click(&mut app, 1.0, 1.0, Modifiers::ALT);
    assert_unchanged(&app);
}
