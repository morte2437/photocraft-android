//! #189: while a Brush stroke is drawn, the canvas must show the stroke as it will be committed.
//! v0.1 drew a stand-in while dragging, a foreground-coloured egui polyline as wide as the brush,
//! which egui tessellates into hard pie wedges fanning out from the start once the brush is much
//! wider than the pointer's steps (zoomed in). The real stroke only appeared on release.
//!
//! These drive the real app offscreen and compare the screen at the last pointer move with the
//! screen after release, on the GPU canvas and on the CPU canvas (flipped views draw through it),
//! at several zooms, with smoothing, pressure, and 8- and 16-bit documents. Skips when no GPU
//! adapter exists (like `color_managed_canvas.rs`).

use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::control::{ControlRequest, handle};
use serde_json::{Value, json};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;
type Pixels = Vec<[u8; 4]>;

#[test]
fn raw_pointer_brush_and_mixer_pixels_appear_in_the_input_frame() {
    use photocraft_ui_egui::state::Tool;
    let _gpu = photocraft_testkit::gpu_lock();
    for flip in [false, true] {
        for tool in [Tool::Brush, Tool::MixerBrush] {
            let Some(mut h) = harness() else { return };
            let app = h.state_mut();
            app.run("file.new", json!({"width":300,"height":200,"background":"white"})).unwrap();
            app.run("edit.fill", json!({"color":"#2040df"})).unwrap();
            app.run("tools.setColors", json!({"foreground":"#ef2000"})).unwrap();
            app.run("tools.setBrush", json!({"brush":{"size":24,"hardness":1.0,"pressureSize":false,"smoothing":{"amount":0.0}}})).unwrap();
            app.ui.tool = tool;
            app.ui.view.flip_horizontal = flip;
            app.sync_views();
            h.run_steps(4);
            let at = h.state().last_canvas_rect.center();
            let before = h.render().unwrap().get_pixel(at.x as u32, at.y as u32).0;
            h.input_mut().events.push(egui::Event::PointerMoved(at));
            h.input_mut().events.push(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            let image = h.render().unwrap();
            assert_ne!(image.get_pixel(at.x as u32, at.y as u32).0, before, "{tool:?} flip={flip}: press visible in its own frame");
            let next = at + egui::vec2(40.0, 0.0);
            h.input_mut().events.push(egui::Event::PointerMoved(next));
            h.step();
            let image = h.render().unwrap();
            assert_ne!(image.get_pixel(next.x as u32, next.y as u32).0, before, "{tool:?} flip={flip}: move visible in its own frame");
            if tool == Tool::MixerBrush
                && !flip
                && let Ok(path) = std::env::var("PHOTOCRAFT_TEST_SNAPSHOT")
            {
                image.save(path).unwrap();
            }
            h.input_mut().events.push(egui::Event::PointerButton {
                pos: next,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.step();
            assert!(!h.state().ui.status_error, "{}", h.state().ui.status);
            assert_eq!(h.state().session.active().unwrap().history.undo_label(), Some(if tool == Tool::MixerBrush { "Mixer Brush" } else { "Brush Tool" }));
        }
    }
}

fn harness() -> Option<Harness> {
    let built = std::panic::catch_unwind(|| {
        egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 640.0)).with_pixels_per_point(1.0).with_max_steps(64).wgpu().build_eframe(|cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
            if let Some(rs) = cc.wgpu_render_state.as_ref() {
                app.set_wgpu(rs.clone());
            }
            app
        })
    });
    match built {
        Ok(h) => Some(h),
        Err(_) => {
            eprintln!("skipping: no GPU adapter");
            None
        }
    }
}

/// Runs a control call and the frame that shows it; returns that (call + frame) time in ms.
fn control(h: &mut Harness, method: &str, params: Value) -> f64 {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params);
    let t0 = std::time::Instant::now();
    handle(h.state_mut(), &ctx, &req);
    h.step();
    let ms = t0.elapsed().as_secs_f64() * 1000.0;
    h.run_steps(2);
    ms
}

#[derive(Clone, Copy, Debug)]
struct Case {
    zoom: f32,
    smoothing: f32,
    hardness: f32,
    depth: u8,
    pressure: bool,
    /// Flipped view: drawn through the CPU canvas texture instead of the GPU canvas.
    flip: bool,
}

/// A dark blue document with a white line, then a soft black stroke across it. Returns the
/// canvas pixels before the stroke, at its last pointer move, and after release, and whether
/// the GPU canvas drew them.
fn frames(h: &mut Harness, c: Case) -> (Pixels, Pixels, Pixels, bool) {
    let mut times = Vec::new();
    {
        let app = h.state_mut();
        // Close what an earlier case left open.
        while app.session.active().is_some() {
            app.run("file.close", json!({"discard": true})).expect("close");
        }
        app.run("file.new", json!({"width": 300, "height": 200, "depth": c.depth, "background": "white"})).expect("new");
        app.run("edit.fill", json!({"color": "#1a2550"})).expect("fill");
        app.run("paint.stroke", json!({"points": [[60, 60, 1], [240, 140, 1]], "size": 12, "hardness": 1.0, "color": "#ffffff"})).expect("line");
        app.run("tools.setColors", json!({"foreground": [0.0, 0.0, 0.0, 1.0]})).expect("colours");
        let brush = json!({"size": 30, "hardness": c.hardness, "pressureSize": c.pressure, "smoothing": {"amount": c.smoothing}});
        app.run("tools.setBrush", json!({ "brush": brush })).expect("brush");
        app.sync_views();
        app.ui.view.flip_horizontal = c.flip;
    }
    control(h, "ui.set", json!({"tool": "brush", "zoom": c.zoom, "center": [150, 100]}));
    // The smoothing tool option syncs on the first frame with the Brush: set it after.
    h.state_mut().session.tools.brush.smoothing.amount = c.smoothing;
    let canvas = |h: &mut Harness| {
        h.run_steps(3);
        let r = h.state().last_canvas_rect.shrink(4.0);
        let img = h.render().expect("render");
        let mut px = Vec::new();
        for y in r.top() as u32..(r.bottom() as u32).min(img.height()) {
            for x in r.left() as u32..(r.right() as u32).min(img.width()) {
                px.push(img.get_pixel(x, y).0);
            }
        }
        px
    };
    let before = canvas(h);
    let pressure = |t: f64| if c.pressure { 0.3 + 0.7 * t } else { 1.0 };
    control(h, "ui.pointer", json!({"events": [{"kind": "down", "x": 100, "y": 100, "pressure": pressure(0.0)}]}));
    let n = 14;
    let mut last = (100.0, 100.0);
    for i in 1..=n {
        let t = f64::from(i) / f64::from(n);
        last = (100.0 + 100.0 * t, 100.0 + 40.0 * (t * 6.0).sin());
        times.push(control(h, "ui.pointer", json!({"events": [{"kind": "move", "x": last.0, "y": last.1, "pressure": pressure(t)}]})));
    }
    times.sort_by(f64::total_cmp);
    // Pointer move → stroke rendered → frame built (canvas refresh and upload included).
    eprintln!("{c:?}: move-to-frame median {:.2} ms, max {:.2} ms", times[times.len() / 2], times[times.len() - 1]);
    // The pointer stays held at its last point, as a mouse held still repeats its position.
    let held = |h: &mut Harness| {
        control(h, "ui.pointer", json!({"events": [{"kind": "move", "x": last.0, "y": last.1, "pressure": pressure(1.0)}]}));
        canvas(h)
    };
    let drawing = settled(h, held);
    let gpu = h.state().perf.gpu && !c.flip;
    control(h, "ui.pointer", json!({"events": [{"kind": "up", "x": last.0, "y": last.1}]}));
    let done = canvas(h);
    (before, drawing, done, gpu)
}

/// The canvas once the live stroke has stopped changing (#13477). The stroke paints on its own
/// thread and a frame waits only a few real ms for it, so on a slow machine its newest step
/// arrives late, and only a later pointer event shows it; release would then paint the rest.
/// `picture` holds the pointer still and takes a picture, with short real waits between them,
/// until the stroke is idle and three pictures in a row match (matching pixels alone don't prove
/// a slow step has finished): at most `TRIES` times, so a canvas that never settles fails
/// instead of hanging or comparing an unsettled picture.
fn settled(h: &mut Harness, picture: impl Fn(&mut Harness) -> Pixels) -> Pixels {
    const TRIES: usize = 80;
    let mut last = picture(h);
    let mut unchanged = 0;
    for _ in 0..TRIES {
        std::thread::sleep(std::time::Duration::from_millis(25));
        let next = picture(h);
        unchanged = if next == last { unchanged + 1 } else { 0 };
        last = next;
        if unchanged >= 2 && !h.state().live_stroke_busy() {
            return last;
        }
    }
    panic!("the live stroke did not settle before release");
}

fn max_diff(a: &[[u8; 4]], b: &[[u8; 4]]) -> u8 {
    assert_eq!(a.len(), b.len());
    a.iter().zip(b).flat_map(|(p, q)| (0..3).map(move |i| p[i].abs_diff(q[i]))).max().unwrap_or(0)
}

fn the_canvas_shows_the_committed_stroke_part(part: usize) {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness() else { return };
    h.run_steps(4);
    let mut cases = Vec::new();
    for zoom in [0.5, 1.0, 4.0, 12.0] {
        for smoothing in [0.0, 0.1, 0.5] {
            cases.push(Case { zoom, smoothing, hardness: 0.0, depth: 8, pressure: false, flip: false });
        }
    }
    cases.push(Case { zoom: 4.0, smoothing: 0.1, hardness: 1.0, depth: 8, pressure: false, flip: false });
    cases.push(Case { zoom: 4.0, smoothing: 0.1, hardness: 0.0, depth: 16, pressure: false, flip: false });
    cases.push(Case { zoom: 4.0, smoothing: 0.1, hardness: 0.0, depth: 8, pressure: true, flip: false });
    cases.push(Case { zoom: 4.0, smoothing: 0.1, hardness: 0.0, depth: 8, pressure: false, flip: true });
    cases.push(Case { zoom: 12.0, smoothing: 0.5, hardness: 0.0, depth: 16, pressure: true, flip: true });
    let mut gpu_seen = false;
    for c in cases.into_iter().skip(part).step_by(4) {
        let (before, drawing, done, gpu) = frames(&mut h, c);
        gpu_seen |= gpu;
        // The stroke is on screen while drawing (not a blank canvas compared with a blank one)…
        assert!(max_diff(&before, &drawing) > 100, "{c:?}: the stroke shows while drawing");
        // …and is exactly what release leaves there: nothing extra (no stand-in shape), nothing missing.
        let d = max_diff(&drawing, &done);
        assert!(d <= 1, "{c:?}: the canvas changed by {d}/255 on release");
    }
    if !gpu_seen {
        eprintln!("note: the GPU compositor fell back to the CPU for every case");
    }
}

#[test]
fn the_canvas_shows_the_committed_stroke_part_0() {
    the_canvas_shows_the_committed_stroke_part(0);
}

#[test]
fn the_canvas_shows_the_committed_stroke_part_1() {
    the_canvas_shows_the_committed_stroke_part(1);
}

#[test]
fn the_canvas_shows_the_committed_stroke_part_2() {
    the_canvas_shows_the_committed_stroke_part(2);
}

#[test]
fn the_canvas_shows_the_committed_stroke_part_3() {
    the_canvas_shows_the_committed_stroke_part(3);
}

/// A dialog-owned brush must remain above the image even when a live painting tool is selected.
#[test]
fn refine_edge_brush_stays_visible_above_the_canvas_with_live_tools() {
    use photocraft_ui_egui::state::Tool;
    let _gpu = photocraft_testkit::gpu_lock();
    for flip in [false, true] {
        for tool in [Tool::Brush, Tool::MixerBrush] {
            let Some(mut h) = harness() else { return };
            h.state_mut().run("file.new", json!({"width":1200,"height":800,"background":"black"})).unwrap();
            h.state_mut().run("select.all", json!({})).unwrap();
            h.state_mut().ui.tool = tool;
            h.state_mut().ui.view.flip_horizontal = flip;
            h.run_steps(4);
            control(&mut h, "ui.set", json!({"zoom":1.0,"center":[600,400]}));
            control(&mut h, "ui.menu.invoke", json!({"command":"select.refineEdge"}));
            let fields = &mut h.state_mut().ui.dialogs.last_mut().expect("Select and Mask opens").fields;
            fields.insert("__brush".into(), json!(true));
            fields.insert("__preview".into(), json!(false));
            h.run_steps(3);
            let canvas = h.state().last_canvas_rect;
            let at = egui::pos2(canvas.left() + 32.0, canvas.top() + 80.0);
            h.input_mut().events.push(egui::Event::PointerMoved(at));
            h.step();
            let image = h.render().unwrap();
            let cross = image.get_pixel(at.x as u32 + 2, at.y as u32).0;
            assert!(cross[0] > 100 && cross[1] > 100 && cross[2] > 100, "{tool:?} flip={flip}: Refine Edge cross is hidden: {cross:?}");
            if tool == Tool::MixerBrush
                && !flip
                && let Ok(path) = std::env::var("PHOTOCRAFT_DIALOG_SNAPSHOT")
            {
                image.save(path).unwrap();
            }
        }
    }
}

/// Deferred live-tool painting uses the camera changed by this frame's navigation input.
#[test]
fn live_tool_camera_navigation_is_visible_in_the_input_frame() {
    use photocraft_ui_egui::state::Tool;
    let _gpu = photocraft_testkit::gpu_lock();
    for flip in [false, true] {
        for tool in [Tool::Brush, Tool::MixerBrush] {
            for zoom in [false, true] {
                let Some(mut h) = harness() else { return };
                let app = h.state_mut();
                app.run("file.new", json!({"width":300,"height":200,"background":"white"})).unwrap();
                app.run("paint.stroke", json!({"points":[[40,40],[240,160]],"size":30,"hardness":1.0,"color":"#2050df"})).unwrap();
                app.run("prefs.set", json!({"values":{"tools.overscroll":true,"general.animatedZoom":false,"general.zoomWithScrollWheel":false}})).unwrap();
                app.ui.tool = tool;
                app.ui.view.flip_horizontal = flip;
                app.sync_views();
                h.run_steps(4);
                control(&mut h, "ui.set", json!({"zoom":1.0,"center":[150,100]}));
                let at = h.state().last_canvas_rect.center() + egui::vec2(30.0, 10.0);
                h.hover_at(at);
                h.run_steps(2);
                let before = h.state().ui.views[0].clone();
                h.input_mut().events.push(if zoom {
                    egui::Event::Zoom(1.25)
                } else {
                    egui::Event::MouseWheel {
                        unit: egui::MouseWheelUnit::Point,
                        delta: egui::vec2(30.0, 20.0),
                        modifiers: egui::Modifiers::NONE,
                        phase: egui::TouchPhase::Move,
                        source: egui::MouseWheelSource::Unknown,
                    }
                });
                h.step();
                let moved = h.state().ui.views[0].clone();
                assert!(if zoom { moved.zoom != before.zoom } else { moved.center != before.center }, "navigation was applied");
                let immediate = h.render().unwrap();
                let rect = h.state().last_canvas_rect.shrink(8.0);
                h.step();
                let settled = h.render().unwrap();
                if tool == Tool::MixerBrush
                    && flip
                    && zoom
                    && let Ok(path) = std::env::var("PHOTOCRAFT_NAV_SNAPSHOT")
                {
                    settled.save(path).unwrap();
                }
                assert_eq!(h.state().ui.views[0].zoom, moved.zoom);
                assert_eq!(h.state().ui.views[0].center, moved.center);
                let mut diff = 0;
                for y in rect.top() as u32..rect.bottom() as u32 {
                    for x in rect.left() as u32..rect.right() as u32 {
                        for (a, b) in immediate.get_pixel(x, y).0.into_iter().zip(settled.get_pixel(x, y).0) {
                            diff = diff.max(a.abs_diff(b));
                        }
                    }
                }
                assert!(diff <= 1, "{tool:?} flip={flip} zoom={zoom}: canvas changes {diff}/255 one frame after navigation");
            }
        }
    }
}
