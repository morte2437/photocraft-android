use super::*;
#[cfg(not(target_arch = "wasm32"))]
use egui::{Event, Modifiers, PointerButton, pos2};
#[cfg(not(target_arch = "wasm32"))]
use egui_kittest::Harness;
#[cfg(not(target_arch = "wasm32"))]
use serde_json::json;

fn alpha(image: &CustomCursorImage, x: u16, y: u16) -> u8 {
    image.rgba[(usize::from(y) * usize::from(image.size[0]) + usize::from(x)) * 4 + 3]
}

fn rgba_at(image: &CustomCursorImage, x: u16, y: u16) -> [u8; 4] {
    let offset = (usize::from(y) * usize::from(image.size[0]) + usize::from(x)) * 4;
    image.rgba[offset..offset + 4].try_into().unwrap()
}

fn frame(ctx: &egui::Context, draw: impl FnMut(&mut egui::Ui)) -> egui::FullOutput {
    let mut output = ctx.run_ui(Default::default(), draw);
    // These tests inspect cursor output without a renderer.
    output.textures_delta.clear();
    output
}

#[cfg(target_os = "linux")]
fn collect_meshes(shape: &egui::Shape, meshes: &mut Vec<(egui::TextureId, egui::Rect)>) {
    match shape {
        egui::Shape::Mesh(mesh) => meshes.push((mesh.texture_id, mesh.calc_bounds())),
        egui::Shape::Vec(shapes) => {
            for shape in shapes {
                collect_meshes(shape, meshes);
            }
        }
        egui::Shape::Noop => {}
        other => panic!("unexpected non-image cursor fallback primitive: {other:?}"),
    }
}

#[test]
fn bitmap_has_a_transparent_hotspot_and_the_correct_radius_at_each_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let radius = 20.0;
        let image = rasterize(&Shape::Circle { radius, centre: false }, scale).unwrap();
        let [x, y] = image.hotspot;
        assert_eq!(image.size, [2 * x + 1, 2 * y + 1]);
        assert_eq!(image.rgba.len(), usize::from(image.size[0]).pow(2) * 4);
        assert_eq!(alpha(&image, x, y), 0, "the document is visible through the tip");
        assert_eq!(alpha(&image, 0, 0), 0);
        let edge = x + (radius * scale) as u16;
        assert!(alpha(&image, edge, y) > 230);
        assert_eq!(alpha(&image, x - (radius * scale) as u16, y), alpha(&image, edge, y));
        let edge_byte = (usize::from(y) * usize::from(image.size[0]) + usize::from(edge)) * 4;
        assert_eq!(image.rgba[edge_byte], 242, "straight RGBA: 235/255 white over 160/255 black (premultiplied would be 235)");
        assert!(image.rgba.as_chunks::<4>().0.iter().any(|p| p[3] > 0 && p[0] == 0), "dark outline stays visible on white");
    }
}

#[test]
fn precise_and_tip_centre_crosshairs_have_distinct_hotspots() {
    let ring = rasterize(&Shape::Circle { radius: 20.0, centre: true }, 1.0).unwrap();
    assert!(alpha(&ring, ring.hotspot[0], ring.hotspot[1]) > 230);
    let precise = rasterize(&Shape::Crosshair { length: 8.0, gap: 2.0 }, 1.0).unwrap();
    let [x, y] = precise.hotspot;
    assert_eq!(alpha(&precise, x, y), 0);
    assert!(alpha(&precise, x + 5, y) > 230);
    let solid = rasterize(&Shape::Crosshair { length: 6.0, gap: 0.0 }, 1.0).unwrap();
    assert!(alpha(&solid, solid.hotspot[0], solid.hotspot[1]) > 230);
}

#[test]
fn bucket_bitmap_keeps_the_svg_above_left_of_its_exact_crosshair_hotspot() {
    for scale in [1.0, 1.25, 2.0] {
        let image = super::bucket::rasterize(scale).unwrap();
        let expected_x = (26.0 / 24.0 * 22.0 * scale).round() + (scale + 1.0).ceil();
        let expected_y = (25.0 / 24.0 * 22.0 * scale).round() + (scale + 1.0).ceil();
        assert_eq!(image.hotspot, [expected_x as u16, expected_y as u16], "bucket box target at {scale}x");
        let gradient_cross = rasterize(&STANDARD_CROSSHAIR, scale).unwrap();
        assert_eq!(alpha(&image, image.hotspot[0], image.hotspot[1]), 0, "Gradient-style open center at {scale}x");

        let side = usize::from(image.size[0]);
        let has_bucket_art = image.rgba.as_chunks::<4>().0.iter().enumerate().any(|(i, p)| {
            let x = (i % side) as u16;
            let y = (i / side) as u16;
            p[3] > 200 && x + 2 < image.hotspot[0] && y + 2 < image.hotspot[1]
        });
        assert!(has_bucket_art, "opaque bucket art remains above-left of the hotspot at {scale}x");

        for arm in [1, 5, 8] {
            let arm = (arm as f32 * scale).round() as u16;
            assert_eq!(
                rgba_at(&image, image.hotspot[0] + arm, image.hotspot[1]),
                rgba_at(&gradient_cross, gradient_cross.hotspot[0] + arm, gradient_cross.hotspot[1]),
                "right arm matches the Gradient crosshair at {scale}x"
            );
            assert_eq!(
                rgba_at(&image, image.hotspot[0], image.hotspot[1] + arm),
                rgba_at(&gradient_cross, gradient_cross.hotspot[0], gradient_cross.hotspot[1] + arm),
                "lower arm matches the Gradient crosshair at {scale}x"
            );
        }
    }
}

#[test]
fn bucket_rasterization_reports_invalid_scales_and_missing_artwork() {
    for scale in [0.0, f32::NAN, f32::INFINITY] {
        let error = super::bucket::rasterize(scale).unwrap_err();
        assert!(error.contains("Invalid Paint Bucket cursor scale"), "{scale}: {error}");
    }
    let error = crate::icons::rasterize("missing-paint-bucket", 22).unwrap_err();
    assert!(error.contains("Cursor icon \"missing-paint-bucket\" is missing"), "{error}");
}

#[test]
fn bucket_cursor_errors_preserve_the_concise_rasterization_message_across_frames() {
    let ctx = egui::Context::default();
    ctx.set_zoom_factor(100.0);
    let mut errors = Vec::new();
    for _ in 0..2 {
        let output = frame(&ctx, |ui| {
            let scale = if native(ui.ctx()) { native_scale(ui.ctx()) } else { ui.ctx().pixels_per_point() };
            let expected = super::bucket::rasterize(scale).unwrap_err();
            let error = super::bucket(ui.painter(), egui::pos2(100.0, 100.0)).unwrap_err();
            assert_eq!(error, expected, "the dialog and status receive only the original rasterization error");
            assert_eq!(error.lines().count(), 1, "diagnostic backtraces belong in the log");
            errors.push(error);
        });
        assert!(output.platform_output.cursor_image.is_none(), "a failed rasterization never installs a cursor");
    }
    assert_eq!(errors[0], errors[1], "the cached failure stays stable across frames");
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn a_bucket_cursor_error_leaves_the_status_bar_when_the_cursor_recovers() {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    let ctx = egui::Context::default();
    ctx.set_zoom_factor(100.0);
    frame(&ctx, |ui| {
        crate::canvas::paint_bucket_cursor(&mut app, ui.painter(), pos2(100.0, 100.0));
    });
    assert!(app.ui.status_error, "the failure reaches the status bar");
    assert!(app.ui.status.contains("Paint Bucket"), "{}", app.ui.status);

    ctx.set_zoom_factor(1.0);
    for _ in 0..2 {
        frame(&ctx, |ui| {
            crate::canvas::paint_bucket_cursor(&mut app, ui.painter(), pos2(100.0, 100.0));
        });
    }
    assert!(!app.ui.status_error, "the cursor works again");
    assert!(app.ui.status.is_empty(), "{}", app.ui.status);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn bucket_cursor_reuses_the_native_bitmap_while_the_hotspot_moves() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    let cursor = |at| {
        let output = frame(&ctx, |ui| {
            let icon = super::bucket(ui.painter(), at).unwrap();
            ui.ctx().set_cursor_icon(icon);
        });
        output.platform_output.cursor_image.expect("root desktop viewport provides the bucket bitmap")
    };

    let first = cursor(pos2(100.0, 100.0));
    let moved = cursor(pos2(137.0, 163.0));
    assert!(Arc::ptr_eq(&first.rgba, &moved.rgba), "moving the pointer reuses the rasterized bitmap");
    assert_eq!(first.hotspot, moved.hotspot);
}

#[test]
#[cfg(target_os = "linux")]
fn scaled_wayland_bucket_uses_one_cached_image_area_without_a_split_crosshair() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    set_wayland(&ctx, true);
    let run = |native_ppp: f32, at: egui::Pos2| {
        let mut input = egui::RawInput::default();
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native_ppp);
        let mut output = ctx.run_ui(input, |ui| {
            let icon = super::bucket(ui.painter(), at).unwrap();
            ui.ctx().set_cursor_icon(icon);
        });
        output.textures_delta.clear();
        let mut images = Vec::new();
        for shape in &output.shapes {
            collect_meshes(&shape.shape, &mut images);
        }
        (output, images)
    };

    let first_at = pos2(100.0, 100.0);
    run(1.25, first_at); // egui's first Area pass measures content and emits no shapes.
    let (first, first_images) = run(1.25, first_at);
    assert!(first.platform_output.cursor_image.is_none(), "scaled Wayland does not send a bitmap to winit");
    assert_eq!(first.platform_output.cursor_icon, CursorIcon::None, "the image replaces the system arrow");
    assert_eq!(first_images.len(), 1, "bucket and fill crosshair are one Image primitive");
    let image = super::bucket::rasterize(1.25).unwrap();
    let image_bounds = first_images[0].1;
    let image_hotspot = image_bounds.min + egui::vec2((f32::from(image.hotspot[0]) + 0.5) / 1.25, (f32::from(image.hotspot[1]) + 0.5) / 1.25);
    assert_eq!(image_hotspot, first_at, "the center of the hotspot texel stays on the pointer at 1.25x");
    let moved_at = pos2(137.0, 163.0);
    let (moved, moved_images) = run(1.25, moved_at);
    assert_eq!(moved_images.len(), 1);
    assert_eq!(moved_images[0].0, first_images[0].0, "the single scaled texture is cached while it moves");
    let layer = egui::LayerId::new(egui::Order::Tooltip, egui::Id::unique("pc-tool-cursor"));
    assert!(ctx.memory(|memory| memory.areas().visible_layer_ids().contains(&layer)), "one cursor Area owns the image");
    assert!(
        !ctx.memory(|memory| memory.areas().visible_layer_ids().iter().any(|id| id.id == egui::Id::unique("pc-icon-cursor"))),
        "the old icon overlay is absent"
    );
    let mut moved_shapes = Vec::new();
    for shape in &moved.shapes {
        collect_meshes(&shape.shape, &mut moved_shapes);
    }
    assert_eq!(moved_shapes.len(), 1, "there are no independent crosshair line primitives");
    let moved_bounds = moved_images[0].1;
    let moved_hotspot = moved_bounds.min + egui::vec2((f32::from(image.hotspot[0]) + 0.5) / 1.25, (f32::from(image.hotspot[1]) + 0.5) / 1.25);
    assert_eq!(moved_hotspot, moved_at, "the cached image keeps its hotspot aligned after movement");
}

/// On Linux a tip stays within what X servers and hardware cursor planes take (256 px); a larger
/// one falls back to the painted outline. Windows and macOS take far larger cursors.
#[test]
fn cursor_bitmaps_stay_within_the_platform_limit() {
    let fits = |radius: f32| rasterize(&Shape::Circle { radius, centre: false }, 1.0).map(|i| i.size[0]);
    let large = cfg!(any(target_os = "windows", target_os = "macos"));
    assert!(fits(100.0).is_some_and(|side| side <= 255));
    assert_eq!(fits(200.0).is_some(), large);
    assert_eq!(fits(1000.0).is_some(), large, "a 2000 px brush at 100 % still has a native outline");
    assert!(fits(200.0).is_none_or(|side| side <= 2048));
    assert!(fits(1100.0).is_none(), "nothing past 2048 px");
}

#[test]
fn hostile_sizes_are_rejected_before_allocating() {
    for radius in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, 2048.0] {
        assert!(rasterize(&Shape::Circle { radius, centre: false }, 1.0).is_none());
    }
    for scale in [f32::NAN, f32::INFINITY, -1.0, 0.0, f32::MAX] {
        assert!(rasterize(&Shape::Circle { radius: 20.0, centre: false }, scale).is_none());
    }
    for (length, gap) in [(1.0, 2.0), (8.0, -1.0), (f32::NAN, 0.0), (8.0, f32::NAN)] {
        assert!(rasterize(&Shape::Crosshair { length, gap }, 1.0).is_none());
    }
}

#[test]
fn hand_bitmaps_are_outlined_hands_around_a_centred_hotspot_at_each_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let open = rasterize(&Shape::Hand { closed: false }, scale).unwrap();
        let fist = rasterize(&Shape::Hand { closed: true }, scale).unwrap();
        for image in [&open, &fist] {
            let [x, y] = image.hotspot;
            assert_eq!(image.size, [2 * x + 1, 2 * y + 1]);
            assert_eq!(image.rgba.len(), usize::from(image.size[0]).pow(2) * 4);
            assert!(image.size[0] <= (34.0 * scale) as u16, "no bigger than a stock cursor at {scale}: {:?}", image.size);
            assert!(alpha(image, x, y) > 230, "the hotspot is on the hand, not in a hole");
            assert_eq!(alpha(image, 0, 0), 0, "nothing at the corners");
            let pixels = image.rgba.as_chunks::<4>().0;
            assert!(pixels.iter().any(|p| p[3] > 230 && p[0] > 230), "a white hand");
            assert!(pixels.iter().any(|p| p[3] > 100 && p[0] < 25), "with a black outline, readable on white");
        }
        assert_ne!(open.rgba, fist.rgba, "the fist is a different picture");
    }
}

#[test]
fn hand_bitmap_is_a_straight_rgba_silhouette_outlined_outside_the_hand() {
    let image = rasterize(&Shape::Hand { closed: false }, 1.0).unwrap();
    let side = usize::from(image.size[0]);
    let px = |x: usize, y: usize| <[u8; 4]>::try_from(&image.rgba[(y * side + x) * 4..][..4]).unwrap();
    let mid = side / 2;
    // Walking left from the hotspot along the middle finger: white fill, then the dark outline,
    // then nothing, never the other way round.
    let mut seen = Vec::new();
    for x in (0..=mid).rev() {
        let [r, _, _, a] = px(x, mid - 4);
        let kind = if a < 20 {
            'n'
        } else if r > 200 {
            'w'
        } else {
            'd'
        };
        if seen.last() != Some(&kind) {
            seen.push(kind);
        }
    }
    assert_eq!(seen.first(), Some(&'w'));
    assert_eq!(seen.last(), Some(&'n'));
    let first_dark = seen.iter().position(|k| *k == 'd').expect("an outline");
    let first_none = seen.iter().position(|k| *k == 'n').unwrap();
    assert!(first_dark < first_none, "outline between fill and background: {seen:?}");
}

#[test]
fn hostile_scales_get_no_hand_bitmap() {
    for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, 500.0] {
        assert!(rasterize(&Shape::Hand { closed: false }, scale).is_none(), "{scale}");
        assert!(rasterize(&Shape::Hand { closed: true }, scale).is_none(), "{scale}");
    }
}

#[test]
fn the_hand_is_a_bitmap_only_where_the_os_has_none() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    for closed in [false, true] {
        let native = frame(&ctx, |ui| {
            let icon = hand_with(ui.ctx(), closed, false);
            ui.ctx().set_cursor_icon(icon);
        });
        assert_eq!(native.platform_output.cursor_icon, if closed { CursorIcon::Grabbing } else { CursorIcon::Grab });
        assert!(native.platform_output.cursor_image.is_none(), "macOS and the Linux themes draw their own hands");

        let bitmap = frame(&ctx, |ui| {
            let icon = hand_with(ui.ctx(), closed, true);
            ui.ctx().set_cursor_icon(icon);
        });
        let image = bitmap.platform_output.cursor_image.expect("Windows has no stock hand");
        assert_eq!(image.rgba, rasterize(&Shape::Hand { closed }, 1.0).unwrap().rgba);
        assert_eq!(bitmap.platform_output.cursor_icon, CursorIcon::Crosshair, "visible fallback if the OS rejects a bitmap");
    }
    let open = hand_with(&ctx, false, true);
    let fist = hand_with(&ctx, true, true);
    assert_eq!((open, fist), (CursorIcon::None, CursorIcon::None));
}

#[test]
fn zoom_bitmaps_are_outlined_magnifiers_centred_on_the_lens_at_each_dpi() {
    for scale in [1.0, 1.25, 1.5, 2.0, 3.0] {
        let plus = rasterize(&Shape::Zoom { out: false }, scale).unwrap();
        let minus = rasterize(&Shape::Zoom { out: true }, scale).unwrap();
        for image in [&plus, &minus] {
            let [x, y] = image.hotspot;
            assert_eq!(image.size, [2 * x + 1, 2 * y + 1]);
            assert_eq!(image.rgba.len(), usize::from(image.size[0]).pow(2) * 4);
            assert!(image.size[0] <= (34.0 * scale) as u16, "no bigger than a stock cursor at {scale}: {:?}", image.size);
            assert_eq!(alpha(image, 0, 0), 0, "nothing at the corners");
            let handle = (9.0 * scale) as u16;
            assert!(alpha(image, x + handle, y + handle) > 230, "the handle reaches down and right");
            assert_eq!(alpha(image, x - handle, y - handle), 0, "and only that way");
            let pixels = image.rgba.as_chunks::<4>().0;
            assert!(pixels.iter().any(|p| p[3] > 230 && p[0] > 230), "a white lens");
            assert!(pixels.iter().any(|p| p[3] > 100 && p[0] < 25), "with a black outline, readable on white");
        }
        let side = usize::from(plus.size[0]);
        let red = |image: &CustomCursorImage, x: u16, y: u16| image.rgba[(usize::from(y) * side + usize::from(x)) * 4];
        let [x, y] = plus.hotspot;
        let up = y - (2.5 * scale) as u16;
        assert!(red(&plus, x, y) < 60 && red(&minus, x, y) < 60, "both glyphs cross the hotspot");
        assert!(red(&plus, x, up) < 60, "the plus has a vertical arm");
        assert!(red(&minus, x, up) > 200, "the minus does not");
    }
}

#[test]
fn hostile_scales_get_no_zoom_bitmap() {
    for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, 500.0] {
        assert!(rasterize(&Shape::Zoom { out: false }, scale).is_none(), "{scale}");
        assert!(rasterize(&Shape::Zoom { out: true }, scale).is_none(), "{scale}");
    }
}

/// winit shows the plain arrow for `ZoomIn` and `ZoomOut` on Windows (#3735).
#[test]
fn the_magnifier_is_a_bitmap_only_where_the_os_has_none() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    for out in [false, true] {
        let native = frame(&ctx, |ui| {
            let icon = zoom_with(ui.ctx(), out, false);
            ui.ctx().set_cursor_icon(icon);
        });
        assert_eq!(native.platform_output.cursor_icon, if out { CursorIcon::ZoomOut } else { CursorIcon::ZoomIn });
        assert!(native.platform_output.cursor_image.is_none(), "macOS and the Linux themes draw their own magnifiers");

        let bitmap = frame(&ctx, |ui| {
            let icon = zoom_with(ui.ctx(), out, true);
            ui.ctx().set_cursor_icon(icon);
        });
        let image = bitmap.platform_output.cursor_image.expect("Windows has no stock magnifier");
        assert_eq!(image.rgba, rasterize(&Shape::Zoom { out }, 1.0).unwrap().rgba);
        assert_eq!(bitmap.platform_output.cursor_icon, CursorIcon::Crosshair, "visible fallback if the OS rejects a bitmap");
    }
}

#[test]
fn unchanged_geometry_reuses_the_os_upload_and_dpi_invalidates_it() {
    let ctx = egui::Context::default();
    let shape = Shape::Circle { radius: 20.0, centre: false };
    let a = cached_image(&ctx, &shape, 1.0).unwrap();
    let b = cached_image(&ctx, &shape, 1.0).unwrap();
    assert!(Arc::ptr_eq(&a.rgba, &b.rgba));
    let scaled = cached_image(&ctx, &shape, 2.0).unwrap();
    assert!(!Arc::ptr_eq(&a.rgba, &scaled.rgba));
    assert!(scaled.size[0] > a.size[0]);
    let larger = cached_image(&ctx, &Shape::Circle { radius: 40.0, centre: false }, 2.0).unwrap();
    assert!(larger.size[0] > scaled.size[0]);
}

#[test]
fn lifecycle_clears_sticky_images_and_respects_later_widgets() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    let image = rasterize(&Shape::Circle { radius: 20.0, centre: false }, 1.0).unwrap();
    let output = frame(&ctx, |ui| {
        ui.ctx().set_cursor_image(Some(image.clone()));
        ui.ctx().set_cursor_icon(CursorIcon::None);
    });
    assert!(output.platform_output.cursor_image.is_some());
    assert_eq!(output.platform_output.cursor_icon, CursorIcon::Crosshair, "visible fallback if the OS rejects a bitmap");
    let output = frame(&ctx, |_| {});
    assert!(output.platform_output.cursor_image.is_none(), "leaving the canvas cannot keep its bitmap");
    let output = frame(&ctx, |ui| {
        ui.ctx().set_cursor_image(Some(image.clone()));
        ui.ctx().set_cursor_icon(CursorIcon::None);
        ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
    });
    assert!(output.platform_output.cursor_image.is_none());
    assert_eq!(output.platform_output.cursor_icon, CursorIcon::ResizeHorizontal);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn ordinary_brush_cursor_is_not_part_of_the_canvas_draw_list() {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    let output = frame(&ctx, |ui| {
        let icon = circle(ui.painter(), pos2(100.0, 100.0), 20.0, true);
        ui.ctx().set_cursor_icon(icon);
    });
    assert!(output.platform_output.cursor_image.is_some());
    assert!(output.shapes.iter().all(|s| matches!(s.shape, egui::Shape::Noop)));

    let output = frame(&ctx, |ui| {
        let icon = circle(ui.painter(), pos2(100.0, 100.0), 3000.0, false);
        ui.ctx().set_cursor_icon(icon);
    });
    let image = output.platform_output.cursor_image.unwrap();
    assert!(image.size[0] < 32, "oversized tips retain a small native hotspot");
    assert!(output.shapes.iter().any(|s| matches!(s.shape, egui::Shape::Circle(_))), "the full tip is not silently shrunk");
}

#[test]
#[cfg(target_os = "linux")]
fn scaled_wayland_paints_the_brush_outline_instead_of_an_os_bitmap() {
    // Hyprland ignores the viewport winit sizes cursor bitmaps with: at 1.25 the outline was
    // 1.25× too large and its centre 0.2 × its half-width right of and below where the brush
    // painted. Off scale 1 on Wayland the outline is painted at the pointer, at its true size.
    let run = |ctx: &egui::Context, native_ppp: f32| {
        let mut input = egui::RawInput::default();
        input.viewports.entry(egui::ViewportId::ROOT).or_default().native_pixels_per_point = Some(native_ppp);
        let mut output = ctx.run_ui(input, |ui| {
            let icon = circle(ui.painter(), pos2(100.0, 100.0), 20.0, false);
            ui.ctx().set_cursor_icon(icon);
        });
        output.textures_delta.clear();
        let circles: Vec<_> =
            output.shapes.iter().filter_map(|s| if let egui::Shape::Circle(c) = &s.shape { Some((c.center, c.radius)) } else { None }).collect();
        (output.platform_output.cursor_image.is_some(), output.platform_output.cursor_icon, circles)
    };
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    set_wayland(&ctx, true);
    for ppp in [1.25, 1.5, 2.0] {
        let (bitmap, icon, circles) = run(&ctx, ppp);
        assert!(!bitmap, "{ppp}: no OS bitmap");
        assert_eq!(icon, CursorIcon::None, "{ppp}: the painted outline is the pointer");
        assert!(!circles.is_empty() && circles.iter().all(|&(c, r)| c == pos2(100.0, 100.0) && r == 20.0), "{ppp}: {circles:?}");
    }
    let (bitmap, _, circles) = run(&ctx, 1.0);
    assert!(bitmap && circles.is_empty(), "Wayland at scale 1 keeps the low-latency bitmap");
    set_wayland(&ctx, false);
    let (bitmap, _, circles) = run(&ctx, 1.25);
    assert!(bitmap && circles.is_empty(), "X11 at 1.25 keeps the bitmap");
}

/// Run explicitly for visual QA: native cursors are absent from GPU screenshots.
#[test]
#[ignore = "writes target/tool-cursor-preview.png for visual inspection"]
fn write_cursor_preview() {
    let cases = [
        (Shape::Circle { radius: 20.0, centre: false }, 1.0),
        (Shape::Circle { radius: 20.0, centre: false }, 1.5),
        (Shape::Circle { radius: 20.0, centre: true }, 2.0),
        (Shape::Crosshair { length: 8.0, gap: 2.0 }, 2.0),
        (Shape::Crosshair { length: 6.0, gap: 0.0 }, 2.0),
        (Shape::Hand { closed: false }, 1.0),
        (Shape::Hand { closed: true }, 1.0),
        (Shape::Hand { closed: false }, 1.5),
        (Shape::Hand { closed: true }, 2.0),
        (Shape::Zoom { out: false }, 1.0),
        (Shape::Zoom { out: true }, 1.0),
        (Shape::Zoom { out: false }, 2.0),
    ];
    let (width, height) = (600_usize, cases.len() * 100);
    let mut rgba = vec![255; width * height * 4];
    for (col, background) in [0_u8, 128, 255].into_iter().enumerate() {
        for y in 0..height {
            for x in col * 200..(col + 1) * 200 {
                rgba[(y * width + x) * 4..(y * width + x) * 4 + 3].fill(background);
            }
        }
        for (row, (shape, scale)) in cases.iter().enumerate() {
            let image = rasterize(shape, *scale).unwrap();
            let side = usize::from(image.size[0]);
            let x0 = col * 200 + 100 - usize::from(image.hotspot[0]);
            let y0 = row * 100 + 50 - usize::from(image.hotspot[1]);
            for (i, pixel) in image.rgba.as_chunks::<4>().0.iter().enumerate() {
                let at = ((y0 + i / side) * width + x0 + i % side) * 4;
                let a = f32::from(pixel[3]) / 255.0;
                let value = (f32::from(pixel[0]) * a + f32::from(background) * (1.0 - a)).round() as u8;
                rgba[at..at + 3].fill(value);
            }
        }
    }
    let image = photocraft_codecs::Image::from_u8(width as u32, height as u32, photocraft_codecs::ChannelLayout::Rgba, rgba).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tool-cursor-preview.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, png).unwrap();
}

/// Run explicitly to inspect the unified bucket bitmap at 1× and 2× against three backgrounds.
#[test]
#[ignore = "writes tmp/cursor-verification/unified-bucket-preview.png for visual inspection"]
fn write_bucket_cursor_preview() {
    let (width, height) = (600_usize, 300_usize);
    let mut rgba = vec![255; width * height * 4];
    let backgrounds = [32_u8, 128, 255];
    for (col, background) in backgrounds.into_iter().enumerate() {
        for y in 0..height {
            for x in col * 200..(col + 1) * 200 {
                rgba[(y * width + x) * 4..(y * width + x) * 4 + 3].fill(background);
            }
        }
        for (row, scale) in [1.0, 2.0].into_iter().enumerate() {
            let image = super::bucket::rasterize(scale).unwrap();
            let side = usize::from(image.size[0]);
            let pointer = (col * 200 + 100, row * 150 + 75);
            let x0 = pointer.0 - usize::from(image.hotspot[0]);
            let y0 = pointer.1 - usize::from(image.hotspot[1]);
            for (i, pixel) in image.rgba.as_chunks::<4>().0.iter().enumerate() {
                let at = ((y0 + i / side) * width + x0 + i % side) * 4;
                let a = f32::from(pixel[3]) / 255.0;
                for channel in 0..3 {
                    rgba[at + channel] = (f32::from(pixel[channel]) * a + f32::from(background) * (1.0 - a)).round() as u8;
                }
            }
        }
    }
    let image = photocraft_codecs::Image::from_u8(width as u32, height as u32, photocraft_codecs::ChannelLayout::Rgba, rgba).unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tmp/cursor-verification/unified-bucket-preview.png");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, png).unwrap();
}

#[cfg(not(target_arch = "wasm32"))]
fn harness() -> Harness<'static, crate::PhotocraftApp> {
    let mut app = crate::PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
    app.run("file.new", json!({"width": 200, "height": 200})).unwrap();
    app.run("tools.setBrush", json!({"brush": {"size": 40, "hardness": 1.0}})).unwrap();
    app.ui.tool = crate::Tool::Brush;
    app.sync_views();
    let mut h = Harness::builder().with_size(vec2(600.0, 500.0)).build_ui_state(
        |ui, app: &mut crate::PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::Panel::top("cursor-test-toolbar").show(ui, |ui| {
                ui.label("Toolbar");
            });
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            crate::dialogs::show(app, &ctx);
        },
        app,
    );
    crate::PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    h.event(Event::PointerMoved(h.state().last_canvas_rect.center()));
    h.run_steps(3);
    h
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn brush_cursor_follows_pen_orientation_6350() {
    for control in ["penTilt", "rotation"] {
        let mut h = harness();
        h.state_mut()
            .run(
                "tools.setBrush",
                json!({"brush": {
                    "size": 60, "hardness": 1.0, "roundness": 0.25,
                    "shapeDynamics": {"enabled": true, "angle": {"control": control}}
                }}),
            )
            .unwrap();
        h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample { tilt_x: 45.0, ..Default::default() }));
        h.run_steps(3);
        let initial = h.output().platform_output.cursor_image.clone().expect("initial brush cursor");
        h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample { tilt_y: -45.0, rotation: 90.0, ..Default::default() }));
        h.run_steps(3);
        let turned = h.output().platform_output.cursor_image.clone().expect("turned brush cursor");
        save_pen_cursor_image(&format!("{control}-initial"), &initial);
        save_pen_cursor_image(&format!("{control}-turned"), &turned);
        let pen = h.state().stylus.sample().expect("simulated pen reaches the canvas");
        assert_eq!((pen.tilt_y, pen.rotation), (-45.0, 90.0));
        let point = photocraft_paint::StrokePoint { tilt_y: pen.tilt_y, rotation: pen.rotation, ..Default::default() };
        let step = photocraft_paint::dynamics::StepInput { point, direction: 0.0, initial_direction: 0.0, step: 0 };
        let mut dabs = Vec::new();
        photocraft_paint::dynamics::DabBuilder::new(&h.state().session.tools.brush).build(&step, &mut dabs);
        assert_eq!(dabs.len(), 1);
        assert!((dabs[0].angle.to_degrees() - 90.0).abs() < 0.001, "painted dab turns: {control}");
        assert_ne!(initial.rgba, turned.rgba, "cursor must follow pen orientation: {control}");
        // The dynamically rotated cursor must match the same tip set to that angle directly.
        h.state_mut().session.tools.brush.shape_dynamics.enabled = false;
        h.state_mut().session.tools.brush.angle = 90.0;
        h.run_steps(3);
        let reference = h.output().platform_output.cursor_image.clone().expect("static rotated reference");
        assert_eq!(turned.size, reference.size);
        assert_eq!(turned.hotspot, reference.hotspot);
        assert_eq!(turned.rgba, reference.rgba, "dynamic cursor matches the dab angle: {control}");
    }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn round_cursor_shows_brush_projection_6350() {
    let mut h = harness();
    h.state_mut().session.tools.brush.shape_dynamics.enabled = true;
    h.state_mut().session.tools.brush.shape_dynamics.brush_projection = true;
    h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample::default()));
    h.run_steps(3);
    let upright = h.output().platform_output.cursor_image.clone().unwrap();
    h.state_mut().stylus.feed.set(Some(crate::stylus::PenSample { tilt_x: 60.0, ..Default::default() }));
    h.run_steps(3);
    let tilted = h.output().platform_output.cursor_image.clone().unwrap();
    save_pen_cursor_image("projection-upright", &upright);
    save_pen_cursor_image("projection-tilted", &tilted);
    assert_ne!(upright.rgba, tilted.rgba, "a projected round brush must not keep a circular cursor");
    h.state_mut().stylus.feed.set(None);
    h.run_steps(3);
    assert_eq!(upright.rgba, h.output().platform_output.cursor_image.clone().unwrap().rgba, "mouse restores the unprojected tip");
}

#[cfg(not(target_arch = "wasm32"))]
fn save_pen_cursor_image(name: &str, cursor: &egui::CustomCursorImage) {
    let Some(dir) = std::env::var_os("PHOTOCRAFT_CURSOR_OUTPUT") else { return };
    let dir = std::path::PathBuf::from(dir);
    std::fs::create_dir_all(&dir).unwrap();
    let image =
        photocraft_codecs::Image::from_u8(u32::from(cursor.size[0]), u32::from(cursor.size[1]), photocraft_codecs::ChannelLayout::Rgba, cursor.rgba.to_vec())
            .unwrap();
    let png = photocraft_codecs::encode(&image, photocraft_codecs::Format::Png, &Default::default()).unwrap();
    std::fs::write(dir.join(format!("{name}.png")), png).unwrap();
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn brush_hover_uses_an_os_bitmap_that_survives_motion_but_not_tool_or_panel_changes() {
    let mut h = harness();
    let initial = h.output().platform_output.cursor_image.clone().expect("brush hover must reach the OS, not the painter");
    let p = h.state().last_canvas_rect.center();
    h.event(Event::PointerMoved(p + vec2(25.0, 12.0)));
    h.run_steps(2);
    let moved = h.output().platform_output.cursor_image.clone().unwrap();
    assert!(Arc::ptr_eq(&initial.rgba, &moved.rgba), "moving does not rasterize or re-upload the cursor");

    h.event(Event::PointerMoved(pos2(20.0, 12.0)));
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_none(), "toolbar restores the normal pointer");
    h.event(Event::PointerMoved(p));
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_some());

    // Windows has no stock hand (winit maps `Grab` to the move cursor), so the Hand gets the open
    // hand as a bitmap too (#2196); macOS and the Linux themes draw their own.
    h.state_mut().ui.tool = crate::Tool::Hand;
    h.run_steps(2);
    let hand = h.output().platform_output.cursor_image.clone();
    if cfg!(target_os = "windows") {
        let hand = hand.expect("the Hand's open hand reaches the OS as a bitmap");
        assert_eq!(hand.rgba, rasterize(&Shape::Hand { closed: false }, h.ctx.pixels_per_point()).unwrap().rgba);
    } else {
        assert!(hand.is_none(), "the OS draws the hand");
        assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Grab);
    }

    // Likewise the Zoom tool's magnifier, a minus while ⌥ is held (#3735).
    h.state_mut().ui.tool = crate::Tool::Zoom;
    for out in [false, true] {
        let modifiers = if out { Modifiers::ALT } else { Modifiers::NONE };
        h.event(Event::ModifiersChanged(modifiers));
        h.event(Event::PointerMoved(p + vec2(1.0, 0.0)));
        h.run_steps(2);
        let image = h.output().platform_output.cursor_image.clone();
        if cfg!(target_os = "windows") {
            let image = image.expect("the magnifier reaches the OS as a bitmap");
            assert_eq!(image.rgba, rasterize(&Shape::Zoom { out }, h.ctx.pixels_per_point()).unwrap().rgba, "out: {out}");
        } else {
            assert!(image.is_none(), "the OS draws the magnifier");
            assert_eq!(h.output().platform_output.cursor_icon, if out { CursorIcon::ZoomOut } else { CursorIcon::ZoomIn });
        }
    }
    h.event(Event::ModifiersChanged(Modifiers::NONE));

    h.state_mut().ui.tool = crate::Tool::Brush;
    h.state_mut().run("prefs.set", json!({"path": "cursors.painting", "value": "standard"})).unwrap();
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.is_none());
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Default);
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn painting_preference_and_zoom_change_the_native_cursor_without_changing_pointer_input() {
    let mut h = harness();
    let image = h.output().platform_output.cursor_image.clone().unwrap();
    h.state_mut().ui.views[0].zoom *= 2.0;
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.as_ref().unwrap().size[0] > image.size[0]);
    h.state_mut().run("prefs.set", json!({"path": "cursors.showOnlyCrosshairWhilePainting", "value": true})).unwrap();
    let pos = h.state().last_canvas_rect.center();
    h.event(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Modifiers::NONE });
    h.step();
    h.event(Event::PointerMoved(pos + vec2(20.0, 0.0)));
    h.run_steps(2);
    let cross_size = h.output().platform_output.cursor_image.as_ref().unwrap().size[0];
    assert!(cross_size < image.size[0]);
    assert_eq!(h.ctx.pointer_latest_pos(), Some(pos + vec2(20.0, 0.0)));
    h.event(Event::PointerButton { pos: pos + vec2(20.0, 0.0), button: PointerButton::Primary, pressed: false, modifiers: Modifiers::NONE });
    h.run_steps(2);
    assert!(h.output().platform_output.cursor_image.as_ref().unwrap().size[0] > cross_size);
}

/// #4596: the close-path pointer is the centre cross with a ring below right.
#[test]
fn close_path_bitmap_has_the_centre_cross_and_a_ring_beside_it() {
    for scale in [1.0, 2.0] {
        let image = rasterize(&Shape::Outline { rings: vec![close_ring()].into(), centre: true }, scale).unwrap();
        let [x, y] = image.hotspot;
        let at = |dx: f32, dy: f32| alpha(&image, x + (dx * scale) as u16, y + (dy * scale) as u16);
        assert!(at(0.0, 0.0) > 230 && at(2.0, 0.0) > 0, "the centre cross");
        assert!(at(12.5, 9.0) > 0 && at(9.0, 12.5) > 0, "the ring around (9, 9)");
        assert_eq!(at(9.0, 9.0), 0, "open in the middle");
        assert_eq!(at(0.0, 9.0), 0, "and apart from the cross");
    }
}

/// A big tip's outline is a thin ring in a large square: only the ring (and the centre mark) is
/// evaluated, and the bitmap is exactly what evaluating every pixel gives.
#[test]
fn circle_bitmaps_evaluate_only_the_ring_and_match_every_pixel() {
    let [dark, light] = crate::theme::Tokens::cursor_outline();
    for (radius, scale, centre) in [(0.4, 1.0, true), (3.0, 1.0, false), (20.0, 1.25, true), (57.3, 2.0, false), (90.0, 1.0, true)] {
        let shape = Shape::Circle { radius, centre };
        let image = rasterize(&shape, scale).unwrap();
        let [hot, _] = image.hotspot;
        let side = usize::from(image.size[0]);
        for row in 0..side {
            for col in 0..side {
                let (dx, dy) = (col as f32 - f32::from(hot), row as f32 - f32::from(hot));
                let want = stroke_pixel(coverage(&shape, dx, dy, scale), dark, light);
                let i = (row * side + col) * 4;
                assert_eq!(image.rgba[i..i + 4], want, "radius {radius} at {scale}×, pixel ({col}, {row})");
            }
        }
    }
}

/// The arrow is the handle's direction as the view shows it: turned with a rotated view and
/// mirrored with a flipped one.
#[test]
fn resize_follows_the_view() {
    use CursorIcon::{ResizeHorizontal as H, ResizeNeSw as NeSw, ResizeNwSe as NwSe, ResizeVertical as V};
    let view = |rotation: f32, flip: bool| ViewXform {
        rect: egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)),
        zoom: 1.0,
        center: [0.0, 0.0],
        flip,
        rotation,
        aspect: 1.0,
    };
    // Right, down-right, down and down-left in the document.
    let directions = [[1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [-1.0, 1.0]];
    for (rotation, flip, want) in
        [(0.0, false, [H, NwSe, V, NeSw]), (90.0, false, [V, NeSw, H, NwSe]), (45.0, false, [NwSe, V, NeSw, H]), (0.0, true, [H, NeSw, V, NwSe])]
    {
        let got = directions.map(|d| resize(d, &view(rotation, flip)));
        assert_eq!(got, want, "view turned {rotation}°, flip {flip}");
    }
}

#[test]
fn type_path_cursors_are_small_and_centred() {
    for rings in [super::area_type_rings(), super::path_type_rings()] {
        assert!(rings.len() > 3);
        for v in rings.iter().flatten() {
            assert!(v.x.abs() <= 12.0 && v.y.abs() <= 12.0, "{v:?}");
        }
    }
}

#[test]
fn the_pointer_belongs_to_another_window_from_its_report_until_this_one_hears_it_again() {
    // The main window, then a floating panel's window over it, the desktop, and back.
    let mut state = false;
    let mut frame = |here, there| {
        state = elsewhere(state, here, there);
        state
    };
    assert!(!frame(true, false), "over the main window");
    assert!(frame(true, true), "crossing into the floating window");
    assert!(frame(false, true), "moving in the floating window");
    assert!(frame(false, false), "on the desktop: the main window still heard nothing");
    assert!(!frame(true, false), "back over the main window");
    let float = Some(egui::Rect::from_min_size(egui::pos2(100.0, 100.0), egui::vec2(200.0, 300.0)));
    assert!(covers(float, Some(egui::pos2(150.0, 150.0))) && !covers(float, Some(egui::pos2(98.0, 150.0))), "just outside its edge stays here");
}

/// Check glyph bounds, contrast and the fixed Convert Point apex across display scales.
#[test]
fn pen_bitmaps_render_all_six_variants_with_exact_outline_and_hotspots_at_each_scale() {
    let anchor = convert_point_anchor_centre();
    assert_eq!(super::PenVariant::Standard.hotspot_in_box(), vec2(2.3, 2.3), "the full-size pen nib is at (2.3, 2.3)");
    assert_eq!(super::PenVariant::ConvertPoint.hotspot_in_box(), vec2(12.0, 7.0), "match the painted Convert Point apex");
    assert_eq!(anchor, vec2(12.0, 7.0), "convert-point.svg anchor square centre");
    assert_eq!(super::PenVariant::ConvertPoint.hotspot_in_box(), anchor, "the Convert Point hotspot is the SVG anchor square centre");
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        for variant in super::PenVariant::all() {
            let image = super::pen::rasterize(variant, scale).expect("pen variant rasterizes");
            let [w, h] = image.size;
            assert!(w > 0 && h > 0, "{variant:?} side at {scale}");
            assert!(w <= (38.0 * scale) as u16, "{variant:?} side within cursor bounds at {scale}");
            assert_eq!(image.rgba.len(), (usize::from(w) * usize::from(h) * 4));

            let [hx, hy] = image.hotspot;
            assert!(hx < w && hy < h, "{variant:?} hotspot inside bitmap at {scale}");

            // Verify straight RGBA: white foreground has RGB=255, dark outline has RGB=0 and alpha>0
            let pixels = image.rgba.as_chunks::<4>().0;
            let has_white = pixels.iter().any(|p| p[0] == 255 && p[1] == 255 && p[2] == 255 && p[3] > 180);
            let has_dark_outline = pixels.iter().any(|p| p[0] == 0 && p[1] == 0 && p[2] == 0 && p[3] > 50);
            assert!(has_white, "{variant:?} at {scale} has solid white foreground");
            assert!(has_dark_outline, "{variant:?} at {scale} has dark outline");

            // Verify hotspot matches expected formula: pad + (hot_box / 24.0) * 22.0 * scale,
            // with Convert Point anchored to its SVG square rather than a literal.
            let offset_px = scale;
            let pad = (offset_px + 2.0).ceil();
            let hot_box = expected_hot_box(variant, anchor);
            let ideal_hx = pad + (hot_box.x / 24.0) * 22.0 * scale;
            let ideal_hy = pad + (hot_box.y / 24.0) * 22.0 * scale;
            let expected_hx = ideal_hx.round() as u16;
            let expected_hy = ideal_hy.round() as u16;
            assert_eq!(hx, expected_hx, "{variant:?} hotspot x at {scale}");
            assert_eq!(hy, expected_hy, "{variant:?} hotspot y at {scale}");
        }
    }
}

/// Catch swapped glyph mappings, including the two anchor badges.
#[test]
fn pen_variants_have_distinct_hotspots_and_geometry() {
    let standard = super::pen::rasterize(super::PenVariant::Standard, 1.0).unwrap();
    let convert = super::pen::rasterize(super::PenVariant::ConvertPoint, 1.0).unwrap();
    let freeform = super::pen::rasterize(super::PenVariant::Freeform, 1.0).unwrap();

    // Standard nib vs ConvertPoint apex vs Freeform nib
    assert_ne!(standard.hotspot, convert.hotspot);
    assert_ne!(standard.hotspot, freeform.hotspot);
    assert_ne!(convert.hotspot, freeform.hotspot);
    assert_ne!(standard.rgba, convert.rgba);
    assert_ne!(standard.rgba, freeform.rgba);
    let add = super::pen::rasterize(super::PenVariant::AddAnchor, 1.0).unwrap();
    let delete = super::pen::rasterize(super::PenVariant::DeleteAnchor, 1.0).unwrap();
    assert_ne!(standard.rgba, add.rgba, "Add Anchor keeps its plus badge");
    assert_ne!(standard.rgba, delete.rgba, "Delete Anchor keeps its minus badge");
    assert_ne!(add.rgba, delete.rgba, "the two anchor badges differ");

    // Pin every tool-to-variant and variant-to-icon mapping so a swapped glyph cannot pass.
    for (tool, variant, icon) in [
        (Tool::Pen, super::PenVariant::Standard, "pen-tool"),
        (Tool::FreeformPen, super::PenVariant::Freeform, "pen-freeform"),
        (Tool::CurvaturePen, super::PenVariant::Curvature, "pen-curvature"),
        (Tool::AddAnchorPoint, super::PenVariant::AddAnchor, "pen-add-anchor"),
        (Tool::DeleteAnchorPoint, super::PenVariant::DeleteAnchor, "pen-delete-anchor"),
        (Tool::ConvertPoint, super::PenVariant::ConvertPoint, "convert-point"),
    ] {
        assert_eq!(super::PenVariant::from_tool(tool), Some(variant), "{tool:?} maps to its Pen variant");
        assert_eq!(variant.icon_name(), icon, "{variant:?} uses its own icon");
    }
}

/// The Convert Point glyph's anchor-square centre, read from the SVG's `<rect>` so a moved square
/// fails the hotspot tests instead of passing against a stale literal.
fn convert_point_anchor_centre() -> Vec2 {
    const SVG: &str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/../../assets/icons/convert-point.svg"));
    let rect = SVG
        .split_once("<rect")
        .and_then(|(_, rest)| rest.split_once('>'))
        .map(|(attrs, _)| attrs)
        .expect("convert-point.svg contains one <rect> anchor square");
    let attr = |name: &str| -> f32 {
        rect.split_whitespace()
            .find_map(|a| a.strip_prefix(name)?.strip_prefix("=\"")?.strip_suffix('"')?.parse::<f32>().ok())
            .unwrap_or_else(|| panic!("convert-point.svg <rect> is missing numeric {name}"))
    };
    vec2(attr("x") + attr("width") / 2.0, attr("y") + attr("height") / 2.0)
}

/// The point each glyph's hotspot is built from: the SVG anchor-square centre for Convert Point,
/// the icon's own point for every other variant.
fn expected_hot_box(variant: super::PenVariant, anchor: Vec2) -> Vec2 {
    if variant == super::PenVariant::ConvertPoint { anchor } else { variant.hotspot_in_box() }
}

/// Reject invalid or unbounded rasterization scales without allocating a cursor.
#[test]
fn hostile_scales_get_no_pen_bitmap() {
    for scale in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -1.0, 0.0, f32::MAX, 500.0] {
        for variant in super::PenVariant::all() {
            assert!(super::pen::rasterize(variant, scale).is_none(), "{variant:?} at {scale}");
        }
    }
}

/// Tool switching reuses small Pen images without retaining additional brush images.
#[test]
fn pen_cursor_bounded_cache_reuses_uploads_and_bounds_memory() {
    let ctx = egui::Context::default();
    let shape_a = super::PenVariant::Standard;
    let a1 = super::cached_pen_image(&ctx, shape_a, 1.0).unwrap();
    let a2 = super::cached_pen_image(&ctx, shape_a, 1.0).unwrap();
    assert!(Arc::ptr_eq(&a1.rgba, &a2.rgba), "pointer movement reuses cached bitmap");

    // All six variants can be cached concurrently without thrashing
    let mut images = Vec::new();
    for v in super::PenVariant::all() {
        let img = super::cached_pen_image(&ctx, v, 1.0).unwrap();
        images.push(img);
    }
    for (i, v) in super::PenVariant::all().into_iter().enumerate() {
        let lookup = super::cached_pen_image(&ctx, v, 1.0).unwrap();
        assert!(Arc::ptr_eq(&lookup.rgba, &images[i].rgba), "switching tools retains cached image");
    }

    // Non-Pen shapes use single-entry cache (memory bounded to 1 bitmap)
    for i in 0..25 {
        let _ = super::cached_image(&ctx, &super::Shape::Circle { radius: (10 + i) as f32, centre: false }, 1.0);
    }
    let id = egui::Id::unique("photocraft-tool-cursor");
    let cached = ctx.data(|d| d.get_temp::<super::Cached>(id)).unwrap();
    assert_eq!(cached.shape, super::Shape::Circle { radius: 34.0, centre: false }, "non-Pen cache holds only single most recent shape");
}

/// Clear stale images and retain a visible fallback when the backend rejects a bitmap.
#[test]
fn pen_cursor_lifecycle_and_fallback_marks_visible_cursor() {
    let ctx = egui::Context::default();
    ctx.add_plugin(super::CursorLifecycle);

    // Simulated native pass
    let output = frame(&ctx, |ui| {
        let icon = super::pen(ui.ctx(), Tool::Pen);
        if let Some(i) = icon {
            ui.ctx().set_cursor_icon(i);
        }
    });
    assert!(output.platform_output.cursor_image.is_some(), "desktop root sets native cursor");
    assert_eq!(output.platform_output.cursor_icon, CursorIcon::Crosshair, "fallback if OS rejects upload");

    // Clear on pass end / leaving canvas
    let empty = frame(&ctx, |_| {});
    assert!(empty.platform_output.cursor_image.is_none(), "lifecycle clears cursor image");
}

/// Guard padding, integer hotspot rounding and unclipped cursor ink.
#[test]
fn pen_bitmaps_geometric_bounds_and_hotspot_quantization_limits() {
    let anchor = convert_point_anchor_centre();
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        let offset_px = scale;
        let pad = (offset_px + 2.0).ceil() as usize;
        let icon_px = (22.0 * scale).round() as usize;
        let expected_side = icon_px + pad * 2;

        for variant in super::PenVariant::all() {
            let image = super::pen::rasterize(variant, scale).expect("pen variant rasterizes");
            let [w, h] = image.size;
            let [hx, hy] = image.hotspot;

            // 1. Geometric bounds: exact bounding box accommodating 22pt icon + 1pt outline + padding
            assert_eq!(usize::from(w), expected_side, "{variant:?} width at {scale}");
            assert_eq!(usize::from(h), expected_side, "{variant:?} height at {scale}");

            // 2. Hotspot quantization: OS cursor requires integer hotspot coordinates.
            let hot_box = expected_hot_box(variant, anchor);
            let ideal_hx = pad as f32 + (hot_box.x / 24.0) * 22.0 * scale;
            let ideal_hy = pad as f32 + (hot_box.y / 24.0) * 22.0 * scale;

            // Quantization to integer physical pixel must not exceed 0.5 px (round)
            let diff_x = (f32::from(hx) - ideal_hx).abs();
            let diff_y = (f32::from(hy) - ideal_hy).abs();
            assert!(diff_x <= 0.5 + 1e-4, "{variant:?} hotspot x quantization error {diff_x} exceeds 0.5 px at scale {scale}");
            assert!(diff_y <= 0.5 + 1e-4, "{variant:?} hotspot y quantization error {diff_y} exceeds 0.5 px at scale {scale}");

            // Hotspot must lie strictly within bitmap bounds
            assert!(usize::from(hx) < expected_side);
            assert!(usize::from(hy) < expected_side);

            // 3. Measured alpha edge profile:
            // - Transparent margins: outermost 1px perimeter has zero alpha (no clipping)
            let pixels = image.rgba.as_chunks::<4>().0;
            for x in 0..usize::from(w) {
                let top_alpha = pixels[x][3];
                let bottom_alpha = pixels[(usize::from(h) - 1) * usize::from(w) + x][3];
                assert_eq!(top_alpha, 0, "{variant:?} top border pixel non-zero at scale {scale}");
                assert_eq!(bottom_alpha, 0, "{variant:?} bottom border pixel non-zero at scale {scale}");
            }
            for y in 0..usize::from(h) {
                let left_alpha = pixels[y * usize::from(w)][3];
                let right_alpha = pixels[y * usize::from(w) + usize::from(w) - 1][3];
                assert_eq!(left_alpha, 0, "{variant:?} left border pixel non-zero at scale {scale}");
                assert_eq!(right_alpha, 0, "{variant:?} right border pixel non-zero at scale {scale}");
            }

            // - Non-empty foreground with solid white pixels and dark halo outline
            let white_count = pixels.iter().filter(|p| p[0] == 255 && p[1] == 255 && p[2] == 255 && p[3] >= 200).count();
            let dark_count = pixels.iter().filter(|p| p[0] == 0 && p[1] == 0 && p[2] == 0 && p[3] >= 50).count();
            assert!(white_count > 10, "{variant:?} at {scale} has insufficient white foreground pixels: {white_count}");
            assert!(dark_count > 10, "{variant:?} at {scale} has insufficient dark outline pixels: {dark_count}");
        }
    }
}

/// Check the compositing identity independently from the rasterizer implementation.
#[test]
fn synthetic_straight_rgba_compositing_math_proves_equivalence() {
    // Mathematical equivalence of single-pass straight RGBA hardware blending
    // vs multi-pass painter fallback (dark outline + white foreground) over any background:
    // Hardware blend: C_hw = C_straight * a_out + C_bg * (1 - a_out)
    // Multi-pass blend: C_painter = 255 * a_fg + (C_bg * (1 - a_dark)) * (1 - a_fg)
    // Equivalence: C_straight * a_out = (a_fg / a_out) * a_out = a_fg.
    // (1 - a_out) = 1 - (a_fg + a_dark * (1 - a_fg)) = (1 - a_fg) * (1 - a_dark).
    // Therefore C_hw == C_painter for all C_bg, within integer rounding tolerance (<= 1 LSB).
    for &c_bg in &[0u8, 15, 64, 128, 200, 240, 255] {
        for fg_step in 0..=20 {
            let a_fg = fg_step as f32 / 20.0;
            for dark_step in 0..=20 {
                let a_dark = dark_step as f32 / 20.0;
                let a_out = a_fg + a_dark * (1.0 - a_fg);
                if a_out <= 0.0 {
                    continue;
                }
                let white = ((a_fg / a_out) * 255.0).round().clamp(0.0, 255.0);
                let alpha = (a_out * 255.0).round().clamp(0.0, 255.0);

                let c_hw = ((white * alpha + (c_bg as f32) * (255.0 - alpha) + 127.5) / 255.0).floor() as i32;
                let c_painter = ((255.0 * a_fg + ((c_bg as f32) * (1.0 - a_dark)) * (1.0 - a_fg)).round()) as i32;
                assert!((c_hw - c_painter).abs() <= 1, "background {c_bg}, a_fg {a_fg}, a_dark {a_dark}: hw {c_hw} vs painter {c_painter}");
            }
        }
    }
}

/// White icon coverage is the sole source of non-black RGB. Re-premultiplying
/// rasterizer output must recover the independently rasterized SVG coverage,
/// including translucent pixels where a premultiplied/straight mixup is visible.
#[test]
fn partial_alpha_pen_rgb_recovers_source_glyph_coverage() {
    let mut partial_alpha = Vec::new();
    for scale in [1.0f32, 1.25, 1.5, 2.0] {
        let pad = (scale + 2.0).ceil() as usize;
        let side = (22.0 * scale).round() as u32;
        for variant in super::PenVariant::all() {
            let source = crate::icons::rasterize_svg(variant.icon_name(), side, side).unwrap();
            let image = super::pen::rasterize(variant, scale).unwrap();
            let width = usize::from(image.size[0]);
            let mut checked = 0;
            for (i, pixel) in source.pixels.iter().enumerate() {
                let out_i = ((i / source.size[0] + pad) * width + i % source.size[0] + pad) * 4;
                let out = &image.rgba[out_i..out_i + 4];
                if pixel.a() == 0 || out[3] == 0 || out[3] == 255 {
                    continue;
                }
                for channel in &out[..3] {
                    let recovered = (u16::from(*channel) * u16::from(out[3]) + 127) / 255;
                    assert!(
                        recovered.abs_diff(u16::from(pixel.a())) <= 1,
                        "{variant:?} scale {scale}: partial-alpha RGB lost the source coverage at pixel {i}"
                    );
                }
                checked += 1;
            }
            println!("partial-alpha samples for {variant:?} at {scale}x: {checked}");
            partial_alpha.push((variant, scale, checked));
        }
    }
    // Some glyph/scale pairs composite every edge opaquely; each variant must still keep a
    // translucent edge at one of the scales.
    for variant in super::PenVariant::all() {
        let counts: Vec<_> = partial_alpha.iter().filter(|(v, _, _)| *v == variant).map(|&(_, scale, count)| (scale, count)).collect();
        let total: usize = counts.iter().map(|&(_, count)| count).sum();
        assert!(total > 0, "{variant:?} has no translucent edge sample at any scale: {counts:?}");
    }
}

// Exercise the benchmark's actual allocation-admission helper in normal library CI;
// example test binaries are not part of the workspace nextest gate.
mod benchmark_args {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/cursor_bench/args.rs"));
}

// Likewise the percentile helper: nearest rank must include the largest sample, so a single slow
// run cannot hide under the cost gate. Example test binaries are not in the workspace nextest gate.
mod benchmark_stats {
    include!(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/cursor_bench/stats.rs"));

    #[test]
    fn nearest_rank_percentiles_include_the_maximum_for_small_samples() {
        let samples: Vec<f64> = (1..=12).map(f64::from).collect();
        assert_eq!(percentile(&samples, 0.95), 12.0, "ceil(0.95 * 12) = 12, the largest sample");
        assert_eq!(percentile(&samples, 0.99), 12.0);
        assert_eq!(percentile(&samples, 0.5), 6.0, "lower median of an even count");
        assert_eq!(percentile(&samples, 0.1), 2.0, "ceil(0.1 * 12) = 2");
        assert_eq!(percentile(&samples, 0.0), 1.0, "the lowest sample");
        assert_eq!(percentile(&samples, 1.0), 12.0, "the highest sample");
        assert_eq!(percentile(&samples, f64::INFINITY), 12.0, "an infinite percentage stays bounded");
        assert_eq!(percentile(&samples, f64::NAN), 1.0, "a NaN percentage stays bounded");
        assert_eq!(percentile(&[], 0.95), 0.0, "no samples");
        assert_eq!(percentile(&[4.5], 0.95), 4.5, "one sample");
    }
}

/// #9356: the continue-path pointer is the centre cross with a slash below right.
#[test]
fn continue_path_bitmap_has_the_centre_cross_and_a_slash_beside_it() {
    for scale in [1.0, 2.0] {
        let image = rasterize(&Shape::Outline { rings: vec![continue_slash()].into(), centre: true }, scale).unwrap();
        let [x, y] = image.hotspot;
        let at = |dx: f32, dy: f32| alpha(&image, x + (dx * scale) as u16, y + (dy * scale) as u16);
        assert!(at(0.0, 0.0) > 230, "the centre cross");
        assert!(at(9.5, 9.5) > 0, "the slash");
        assert_eq!(at(0.0, 9.0), 0, "apart from the cross");
    }
}
