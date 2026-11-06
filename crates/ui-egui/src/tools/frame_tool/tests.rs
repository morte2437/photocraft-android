use super::*;
use crate::canvas::tool_event;
use photocraft_doc::LayerId;

fn drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2]) {
    let m = egui::Modifiers::NONE;
    tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 }, m);
    tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

/// K picks the Frame tool; a rectangular drag makes one frame (one undo step) clipped to the
/// drawn rect; with the elliptical button the frame's corners are hidden.
#[test]
fn frame_tool_draws_rectangular_and_elliptical_frames() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    let ctx = egui::Context::default();
    let raw = egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::K, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    };
    ctx.begin_pass(raw);
    crate::shortcuts::handle(&mut app, &ctx);
    ctx.end_pass().textures_delta.clear();
    assert_eq!(app.ui.tool, Tool::Frame);

    let frame = |app: &PhotocraftApp| {
        let st = app.session.active().unwrap();
        let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
        assert!(matches!(l.content, LayerContent::Group(_)) && l.name.starts_with("Frame"), "the new frame is selected");
        (l.id, TestMask(l.vector_mask.as_ref().unwrap().path.clone()))
    };
    let layers = app.session.active().unwrap().doc.layers.len();
    drag(&mut app, [20.0, 30.0], [120.0, 110.0]);
    let (_, m) = frame(&app);
    assert_eq!((m.value(20, 30), m.value(119, 109), m.value(19, 30), m.value(120, 109)), (1.0, 1.0, 0.0, 0.0));
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "one undo takes the frame back");

    app.ui.frame_tool.ellipse = true;
    drag(&mut app, [20.0, 30.0], [120.0, 110.0]);
    let (id, m): (LayerId, _) = frame(&app);
    assert_eq!((m.value(70, 70), m.value(20, 30)), (1.0, 0.0), "centre shown, corner hidden");
    // A click draws nothing.
    drag(&mut app, [300.0, 250.0], [300.0, 250.0]);
    assert_eq!(frame(&app).0, id);
}

struct TestMask(photocraft_doc::Path);
impl TestMask {
    fn value(&self, x: i32, y: i32) -> f32 {
        photocraft_vector::path_coverage(&self.0, Rect::new(x, y, x + 1, y + 1))[0]
    }
}

fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width":400,"height":300})).unwrap();
    app.sync_views();
    app.ui.extras.snap = false;
    app.ui.tool = Tool::Frame;
    app
}

#[test]
fn markers_survive_release_and_handles_resize_the_frame_without_creating_another() {
    let mut app = app();
    drag(&mut app, [20., 30.], [120., 110.]);
    let id = selected(&app).unwrap().id;
    let count = app.session.active().unwrap().doc.layer_count();
    let ctx = egui::Context::default();
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400., 300.)),
        zoom: 1.0,
        center: [200., 150.],
        flip: false,
        rotation: 0.,
        aspect: 1.,
    };
    let mut output = ctx.run_ui(egui::RawInput::default(), |ui| draw_overlay(&app, ui.painter(), &xf));
    output.textures_delta.clear();
    assert!(app.ui.frame_tool.drag.is_none());
    assert!(output.shapes.iter().filter(|s| matches!(s.shape, egui::Shape::Rect(_))).count() >= 8, "eight visible handles after release");
    drag(&mut app, [120., 110.], [145., 130.]);
    assert_eq!(app.session.active().unwrap().doc.layer_count(), count, "resizing does not create another frame");
    assert_eq!(selected(&app).unwrap().id, id);
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().transform.m, [125., 0., 0., 100., 20., 30.]);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().transform.m, [100., 0., 0., 80., 20., 30.]);
}

#[test]
fn dragging_border_moves_only_frame_and_escape_discards_live_preview() {
    let mut app = app();
    drag(&mut app, [20., 30.], [120., 110.]);
    let before = selected(&app).unwrap().frame.clone();
    let entries = app.session.active().unwrap().history.entries();
    pointer(&mut app, ToolEvent::Down { x: 50., y: 30., pressure: 1. }, egui::Modifiers::NONE);
    pointer(&mut app, ToolEvent::Move { x: 80., y: 50., pressure: 1. }, egui::Modifiers::NONE);
    assert_eq!(selected(&app).unwrap().frame, before, "preview never changes document");
    assert!(app.frame_preview.is_some());
    let ctx = egui::Context::default();
    ctx.begin_pass(egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: egui::Modifiers::NONE }],
        ..Default::default()
    });
    crate::shortcuts::handle(&mut app, &ctx);
    ctx.end_pass().textures_delta.clear();
    assert!(app.frame_preview.is_none());
    assert!(app.ui.frame_tool.drag.is_none());
    assert_eq!(app.session.active().unwrap().history.entries(), entries);
    drag(&mut app, [50., 30.], [80., 50.]);
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().transform.m, [100., 0., 0., 80., 50., 50.]);
}

#[test]
fn frame_content_and_both_selection_follow_clicks_and_double_clicks() {
    let mut app = app();
    let source = app
        .session
        .edit("Photo", |doc, active| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 200), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    drag(&mut app, [20., 30.], [120., 110.]);
    let frame = selected(&app).unwrap().id;
    let original = selected(&app).unwrap().frame.clone();
    double_click(&mut app, [65., 65.]);
    assert_eq!(app.ui.frame_tool.target, Target::Content);
    assert_eq!(app.session.active().unwrap().active_layer, Some(source));
    double_click(&mut app, [65., 65.]);
    assert_eq!(app.ui.frame_tool.target, Target::Both);
    assert_eq!(app.session.active().unwrap().active_layer, Some(frame));
    drag(&mut app, [50., 30.], [65., 40.]);
    assert_eq!(app.ui.frame_tool.target, Target::Frame);
    assert_ne!(selected(&app).unwrap().frame, original);
    let source_before = app.session.active().unwrap().doc.layer(source).unwrap().clone();
    double_click(&mut app, [70., 75.]);
    let f = selected(&app).unwrap().frame.clone();
    let ctx = egui::Context::default();
    crate::PhotocraftApp::setup_context(&ctx, Default::default());
    ctx.begin_pass(Default::default());
    app.frame_tool_ctx = Some(ctx.clone());
    drag(&mut app, [70., 75.], [85., 85.]);
    ctx.end_pass().textures_delta.clear();
    assert!(!app.ui.frame_tool.auto_transform);
    assert!(app.ui.transform.is_none());
    assert_eq!(selected(&app).unwrap().frame, f, "image-only transform leaves the mask");
    assert_ne!(app.session.active().unwrap().doc.layer(source).unwrap(), &source_before);
}

#[test]
fn modifiers_shape_selection_and_invalid_control_params_are_consistent() {
    let mut app = app();
    let ctx = egui::Context::default();
    let call = |app: &mut PhotocraftApp, p: Value| {
        let (req, _) = crate::control::ControlRequest::new("ui.set", p);
        let crate::control::Outcome::Done(v) = crate::control::handle(app, &ctx, &req) else { panic!("immediate state update") };
        v
    };
    assert_eq!(call(&mut app, json!({"frameShape":"triangle","frameSides":6}))["ok"], true);
    assert_eq!(call(&mut app, json!({"frameShape":"invalid","tool":"brush"}))["ok"], false);
    assert_eq!(app.ui.tool, Tool::Frame);
    assert_eq!(call(&mut app, json!({"frameSides":100000}))["ok"], false);
    let mods = egui::Modifiers { shift: true, alt: true, ..Default::default() };
    pointer(&mut app, ToolEvent::Down { x: 100., y: 100., pressure: 1. }, mods);
    pointer(&mut app, ToolEvent::Up { x: 130., y: 120. }, mods);
    let f = selected(&app).unwrap().frame.as_ref().unwrap();
    assert_eq!(f.bounds(), Rect::new(70, 70, 130, 130));
    assert_eq!(TestMask(f.outline()).value(70, 70), 0.0, "triangle rather than bounding rectangle");
    let frame = selected(&app).unwrap().id;
    let content = app
        .session
        .edit("Image", |doc, _| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(20, 20, 100, 100), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            Ok(id)
        })
        .unwrap();
    app.run("layer.frame.setContent", json!({"layer":frame.0,"content":content.0})).unwrap();
    assert_eq!(call(&mut app, json!({"frameTarget":"content"}))["ok"], true);
    assert_eq!(app.session.active().unwrap().active_layer, Some(content));
    assert_eq!(call(&mut app, json!({"frameTarget":"both"}))["ok"], true);
    assert_eq!(app.session.active().unwrap().active_layer, Some(frame));
}

#[test]
fn frame_creation_handles_render_at_native_scales() {
    let Some(out) = std::env::var_os("PHOTOCRAFT_FRAME_SNAPSHOTS") else { return };
    let _guard = photocraft_testkit::gpu_lock();
    let out = std::path::PathBuf::from(out);
    std::fs::create_dir_all(&out).unwrap();
    for theme in crate::theme::ThemeKind::ALL {
        for (width, height) in [(1280.0, 720.0), (800.0, 480.0)] {
            for scale in [1.0, 2.0] {
                let mut app = app();
                app.ui.theme = theme;
                app.run("prefs.set", json!({"path":"interface.theme","value":theme.id()})).unwrap();
                app.custom_titlebar = true;
                app.session.prefs.edit(|p| p.general.animated_zoom = false);
                drag(&mut app, [20.0, 30.0], [180.0, 130.0]);
                let empty = selected(&app).unwrap().id;
                let source = app
                    .session
                    .edit("Synthetic image", |doc, active| {
                        let mut layer = Layer::raster("Sample", doc.pixel_format());
                        for y in 0..100 {
                            for x in 0..100 {
                                layer.surface_mut().unwrap().write_pixel(x + 200, y + 60, &[x as f32 / 100.0, y as f32 / 100.0, 0.7, 1.0]);
                            }
                        }
                        let id = layer.id;
                        doc.layers.push(layer);
                        *active = Some(id);
                        Ok(id)
                    })
                    .unwrap();
                app.run("layer.new.frame",json!({"x":210,"y":70,"width":120,"height":160,"shape":"ellipse","layer":source.0,"stroke":{"color":"#50a0ef","width":4,"align":"outside"}})).unwrap();
                select(&mut app, empty, Target::Frame);
                let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(width, height)).with_pixels_per_point(scale).wgpu().build_eframe(move |cc| {
                    PhotocraftApp::setup_context(&cc.egui_ctx, theme);
                    if let Some(rs) = cc.wgpu_render_state.as_ref() {
                        app.set_wgpu(rs.clone());
                    }
                    app
                });
                h.run_steps(6);
                assert_eq!(h.state().ui.theme, theme);
                let img = h.render().unwrap();
                assert_eq!((img.width(), img.height()), ((width * scale) as u32, (height * scale) as u32));
                img.save(out.join(format!("frame-{theme:?}-{width}-{scale}x.png"))).unwrap();
            }
        }
    }
}

#[test]
fn free_transform_on_the_frame_thumbnail_previews_and_commits_only_the_clip() {
    let mut app = app();
    let source = app
        .session
        .edit("Photo", |doc, active| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 200), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    drag(&mut app, [20., 30.], [120., 110.]);
    let frame = selected(&app).unwrap().id;
    let original = app.session.active().unwrap().doc.layer(source).unwrap().clone();
    let ctx = egui::Context::default();
    PhotocraftApp::setup_context(&ctx, Default::default());
    ctx.begin_pass(Default::default());
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    assert!(frame_transform(app.ui.transform.as_ref().unwrap()));
    app.ui.transform.as_mut().unwrap().quad = [[40., 40.], [90., 40.], [90., 80.], [40., 80.]];
    let (shown, _) = crate::canvas::display_doc(&mut app, 0);
    assert_eq!(shown.layer(frame).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(40, 40, 90, 80));
    assert_eq!(shown.layer(source).unwrap(), &original);
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(20, 30, 120, 110));
    crate::transform_tool::commit(&mut app);
    assert!(app.ui.transform.is_none());
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(40, 40, 90, 80));
    assert_eq!(app.session.active().unwrap().doc.layer(source).unwrap(), &original);
    app.run("edit.undo", json!({})).unwrap();
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(20, 30, 120, 110));
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    app.ui.transform.as_mut().unwrap().quad = [[50., 50.], [100., 50.], [100., 90.], [50., 90.]];
    crate::transform_tool::cancel(&mut app);
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(20, 30, 120, 110));
    ctx.end_pass().textures_delta.clear();
}

#[test]
fn real_canvas_clicks_select_both_then_double_click_selects_the_image() {
    let mut app = app();
    app.session
        .edit("Photo", |doc, active| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 200), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            *active = Some(id);
            Ok(())
        })
        .unwrap();
    drag(&mut app, [20., 30.], [120., 110.]);
    let frame = selected(&app).unwrap().id;
    let mut h = egui_kittest::Harness::builder().with_step_dt(1.0 / 60.0).with_size(egui::vec2(1000., 700.)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            app.frame_tool_ctx = Some(ui.ctx().clone());
            crate::canvas::document_area(app, ui);
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, Default::default());
    h.run_steps(4);
    let p = ViewXform::active(h.state()).unwrap().to_screen(65., 65.);
    for press in [true, false] {
        h.event(egui::Event::PointerMoved(p));
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: press, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    assert_eq!(h.state().ui.frame_tool.target, Target::Both);
    assert_eq!(h.state().session.active().unwrap().active_layer, Some(frame));
    for press in [true, false] {
        h.event(egui::Event::PointerMoved(p));
        h.event(egui::Event::PointerButton { pos: p, button: egui::PointerButton::Primary, pressed: press, modifiers: egui::Modifiers::NONE });
        h.step();
    }
    assert_eq!(h.state().ui.frame_tool.target, Target::Content);
    assert_ne!(h.state().session.active().unwrap().active_layer, Some(frame));
}

#[test]
fn hostile_frame_pointer_coordinates_never_publish_an_invalid_preview() {
    let mut app = app();
    app.ui.frame_tool.shape = "custom".into();
    let before = app.session.active().unwrap().doc.clone();
    for x in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::MIN, 1e30, -1e30] {
        assert!(pointer(&mut app, ToolEvent::Down { x, y: 30., pressure: 1. }, Default::default()));
        pointer(&mut app, ToolEvent::Move { x: -x, y: 40., pressure: 1. }, Default::default());
        pointer(&mut app, ToolEvent::Up { x, y: 50. }, Default::default());
        assert!(app.ui.frame_tool.drag.is_none());
        assert!(app.frame_preview.is_none());
        assert_eq!(app.session.active().unwrap().doc.layers, before.layers);
    }
}

#[test]
fn frame_keyboard_nudges_change_only_the_selected_thumbnail() {
    let mut app = app();
    app.session
        .edit("Photo", |doc, active| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 200), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            *active = Some(id);
            Ok(())
        })
        .unwrap();
    drag(&mut app, [20., 30.], [120., 110.]);
    let frame = selected(&app).unwrap().id;
    let child = selected(&app).unwrap().children().unwrap()[0].clone();
    let ctx = egui::Context::default();
    for (key, mods) in [(egui::Key::ArrowRight, egui::Modifiers::NONE), (egui::Key::ArrowDown, egui::Modifiers::SHIFT)] {
        ctx.begin_pass(egui::RawInput {
            events: vec![egui::Event::Key { key, physical_key: None, pressed: true, repeat: false, modifiers: mods }],
            ..Default::default()
        });
        assert!(keys(&mut app, &ctx));
        ctx.end_pass().textures_delta.clear();
    }
    assert_eq!(selected(&app).unwrap().frame.as_ref().unwrap().bounds(), Rect::new(21, 40, 121, 120));
    assert_eq!(app.session.active().unwrap().doc.layer(child.id).unwrap(), &child);
    select(&mut app, frame, Target::Content);
    let f = selected(&app).unwrap().frame.clone();
    ctx.begin_pass(egui::RawInput {
        events: vec![egui::Event::Key { key: egui::Key::ArrowRight, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() }],
        ..Default::default()
    });
    assert!(keys(&mut app, &ctx));
    ctx.end_pass().textures_delta.clear();
    assert_eq!(selected(&app).unwrap().frame, f);
    assert_ne!(app.session.active().unwrap().doc.layer(child.id).unwrap(), &child);
}

#[test]
fn hidden_frame_ancestors_hide_placeholders_and_handles() {
    let mut app = app();
    drag(&mut app, [20., 30.], [120., 110.]);
    let id = selected(&app).unwrap().id;
    app.session
        .edit("Hide group", |doc, _| {
            let frame = doc.remove(id).unwrap();
            let mut parent = Layer::group("Hidden", vec![frame]);
            parent.visible = false;
            doc.layers.push(parent);
            Ok(())
        })
        .unwrap();
    assert_eq!(frame_at(&app, [50., 50.]), None);
    let ctx = egui::Context::default();
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(Pos2::ZERO, egui::vec2(400., 300.)),
        zoom: 1.,
        center: [200., 150.],
        flip: false,
        rotation: 0.,
        aspect: 1.,
    };
    let mut output = ctx.run_ui(Default::default(), |ui| draw_overlay(&app, ui.painter(), &xf));
    output.textures_delta.clear();
    assert!(output.shapes.iter().all(|s| matches!(s.shape, egui::Shape::Noop)));
}

#[test]
fn drawing_a_frame_with_another_frames_image_selected_leaves_that_image_in_place() {
    let mut app = app();
    let source = app
        .session
        .edit("Photo", |doc, active| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 200), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    drag(&mut app, [20., 30.], [120., 110.]);
    let first = selected(&app).unwrap().id;
    double_click(&mut app, [65., 65.]);
    assert_eq!(app.session.active().unwrap().active_layer, Some(source));
    // Starts outside the first frame but overlaps the image's own bounds.
    drag(&mut app, [150., 150.], [250., 250.]);
    let doc = &app.session.active().unwrap().doc;
    assert_eq!(photocraft_engine::frame_cmds::frame_of(doc, source), Some(first), "the image stays in its frame");
    let second = selected(&app).unwrap();
    assert_ne!(second.id, first);
    assert!(second.children().unwrap().is_empty(), "the new frame is empty");
}

/// Esc during a frame's content drag ends its automatic transform: a Free Transform started
/// afterwards waits for Enter instead of committing on its first pointer release.
#[test]
fn cancelling_a_frame_content_drag_clears_its_auto_commit() {
    let mut app = app();
    app.session
        .edit("Photo", |doc, active| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 200), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    drag(&mut app, [20., 30.], [120., 110.]);
    double_click(&mut app, [65., 65.]);
    let ctx = egui::Context::default();
    crate::PhotocraftApp::setup_context(&ctx, Default::default());
    ctx.begin_pass(Default::default());
    app.frame_tool_ctx = Some(ctx.clone());
    tool_event(&mut app, ToolEvent::Down { x: 70., y: 75., pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(app.ui.frame_tool.auto_transform, "a content drag transforms automatically");
    crate::transform_tool::cancel(&mut app);
    ctx.end_pass().textures_delta.clear();
    assert!(app.ui.transform.is_none());
    assert!(!app.ui.frame_tool.auto_transform);
}

/// Double-clicking a frame's content thumbnail opens a Smart Object's contents; a pixel layer
/// inside a frame (as PSD files can hold) has no contents to open.
#[test]
fn double_clicking_frame_content_opens_only_smart_objects() {
    use egui::{Event, PointerButton, vec2};
    let mut s = photocraft_engine::Session::new();
    s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
    let photo = s.execute("layer.new.layer", json!({"name": "Photo"})).unwrap()["layer"].as_u64().unwrap();
    s.execute("edit.fill", json!({"contents": "black"})).unwrap();
    let smart = s.execute("layer.new.frame", json!({"x": 10, "y": 10, "width": 100, "height": 80, "layer": photo})).unwrap()["layer"].as_u64().unwrap();
    let pixels = s.execute("layer.new.frame", json!({"x": 200, "y": 10, "width": 100, "height": 80})).unwrap()["layer"].as_u64().unwrap();
    s.edit("Pixels", |doc, _| {
        let l = Layer::raster("Pixels", doc.pixel_format());
        doc.layer_mut(LayerId(pixels)).and_then(Layer::children_mut).unwrap().push(l);
        Ok(())
    })
    .unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1440.0, 900.0)).with_step_dt(1.0 / 60.0).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        PhotocraftApp::new(s, crate::Services::default())
    });
    let ctx = h.ctx.clone();
    let (r, _) = crate::control::ControlRequest::new("ui.set", json!({"dock": {"collapsed": ["color", "properties", "history", "navigator"]}}));
    crate::control::handle(h.state_mut(), &ctx, &r);
    h.run_steps(8);
    let double_click_content = |h: &mut egui_kittest::Harness<PhotocraftApp>, layer: u64| {
        // Past egui's double-click window, so the last pair's clicks don't run on into these.
        h.run_steps(30);
        let row = crate::layer_row_ui::recorded(&h.ctx).into_iter().find(|r| r.layer == layer).unwrap();
        let thumb = row.thumbnail.unwrap();
        let at = thumb.center() + vec2(thumb.width() + 6.0, 0.0);
        h.hover_at(at);
        h.run_steps(1);
        for _ in 0..2 {
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
            h.event(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
            h.run_steps(1);
        }
        h.run_steps(3);
    };
    double_click_content(&mut h, pixels);
    assert_eq!(h.state().session.documents().len(), 1, "a pixel layer has no contents to open");
    assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
    double_click_content(&mut h, smart);
    assert_eq!(h.state().session.documents().len(), 2, "the Smart Object's contents open");
}

#[test]
fn filled_frame_closing_border_selects_frame() {
    let mut app = app();
    app.session
        .edit("Photo", |doc, active| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 200), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    drag(&mut app, [20., 30.], [120., 110.]);
    double_click(&mut app, [65., 65.]);
    pointer(&mut app, ToolEvent::Down { x: 20., y: 50., pressure: 1. }, egui::Modifiers::NONE);
    assert_eq!(app.ui.frame_tool.target, Target::Frame);
    assert!(matches!(app.ui.frame_tool.drag, Some(FrameDrag::Edit { .. })));
}

#[test]
fn overhanging_content_handles_start_transform_and_cancel() {
    let mut app = app();
    app.session
        .edit("Photo", |doc, active| {
            let mut l = Layer::raster("Photo", doc.pixel_format());
            l.surface_mut().unwrap().fill_rect(Rect::new(0, 0, 200, 100), &[1., 0., 0., 1.]);
            let id = l.id;
            doc.layers.push(l);
            *active = Some(id);
            Ok(id)
        })
        .unwrap();
    drag(&mut app, [20., 30.], [120., 110.]);
    double_click(&mut app, [65., 65.]);
    let ctx = egui::Context::default();
    crate::PhotocraftApp::setup_context(&ctx, Default::default());
    ctx.begin_pass(Default::default());
    app.frame_tool_ctx = Some(ctx.clone());
    let st = app.session.active().unwrap();
    let frame = selected(&app).unwrap();
    let child = frame.children().unwrap().first().unwrap();
    let b = photocraft_engine::transform_cmds::transform_frame(&st.doc, child);
    assert!(b.x0 < 20, "image must overhang the frame: {b:?}");
    let entries = st.history.entries();
    for (x, y) in [(f64::from(b.x0), f64::from(b.y0)), (f64::from(b.x0), f64::from(b.y0 + b.y1) / 2.)] {
        tool_event(&mut app, ToolEvent::Down { x, y, pressure: 1. }, egui::Modifiers::NONE);
        assert!(app.ui.transform.is_some());
        assert!(app.ui.frame_tool.auto_transform);
        crate::transform_tool::cancel(&mut app);
        assert!(app.ui.transform.is_none());
    }
    assert_eq!(app.session.active().unwrap().history.entries(), entries);
    ctx.end_pass().textures_delta.clear();
}
