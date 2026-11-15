use super::*;
use crate::state::{ContentAwareOptions, TransformMode, TransformSession};
use crate::transform_tool::{Hit, cursor, hit, rotate_cursor_dir};
use egui::{CursorIcon, Modifiers, pos2};
use egui_kittest::Harness;
use photocraft_doc::LayerId;
use serde_json::json;

const QUAD: [[f64; 2]; 4] = [[100.0, 100.0], [500.0, 100.0], [500.0, 300.0], [100.0, 300.0]];

fn session(quad: [[f64; 2]; 4]) -> TransformSession {
    TransformSession {
        session: 1,
        layer: 1,
        rect: [100.0, 100.0, 500.0, 300.0],
        quad,
        pivot: quad.iter().fold([0.0; 2], |c, p| [c[0] + p[0] / 4.0, c[1] + p[1] / 4.0]),
        interpolation: "bicubic".into(),
        warp: None,
        selection: false,
        target: None,
        path: None,
        made: None,
        mode: TransformMode::Free,
        content_aware: None,
        skew_v: 0.0,
        layers: Vec::new(),
    }
}

fn app(quad: [[f64; 2]; 4], zoom: f32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", serde_json::json!({"width": 32, "height": 32})).unwrap();
    app.sync_views();
    app.ui.views[0].zoom = zoom;
    app.ui.transform = Some(session(quad));
    app
}

fn view(zoom: f32) -> ViewXform {
    ViewXform { rect: Rect::from_min_size(Pos2::ZERO, vec2(1000.0, 800.0)), zoom, center: [300.0, 200.0], flip: false, rotation: 0.0, aspect: 1.0 }
}

fn near(actual: Option<[f64; 2]>, expected: [f64; 2]) {
    let actual = actual.expect("rotate direction");
    assert!((actual[0] - expected[0]).hypot(actual[1] - expected[1]) < 1e-6, "{actual:?} != {expected:?}");
}

/// Screen-sized zones must survive zoom changes; the interior control rejects an all-sided edge grab area.
#[test]
fn whole_edge_and_corner_priority_table_at_100_and_200_percent() {
    for zoom in [1.0, 2.0] {
        let q = QUAD.map(|p| p.map(|v| v / zoom));
        let t = session(q);
        // Coordinates are screen pixels, so the same table must hold at both document zooms.
        for (p, expected) in [
            ([100.0, 100.0], Hit::Corner(0)),
            ([105.0, 105.0], Hit::Corner(0)),
            ([95.0, 95.0], Hit::Corner(0)),
            ([500.0, 300.0], Hit::Corner(2)),
            ([180.0, 100.0], Hit::Edge(0)),
            ([180.0, 89.0], Hit::Edge(0)),
            ([500.0, 150.0], Hit::Edge(1)),
            ([511.0, 150.0], Hit::Edge(1)),
            ([420.0, 311.0], Hit::Edge(2)),
            ([89.0, 250.0], Hit::Edge(3)),
            ([180.0, 101.0], Hit::Inside),
            ([499.0, 150.0], Hit::Inside),
            ([180.0, 180.0], Hit::Inside),
            ([180.0, 88.0], Hit::Outside),
            ([620.0, 200.0], Hit::Outside),
        ] {
            assert_eq!(hit(&t, p.map(|v| v / zoom), 12.0 / zoom), expected, "zoom {zoom}, screen {p:?}");
        }
        let tiny = session([[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]]);
        assert_eq!(TransformSession::box_hit(&tiny.quad, [1.0, -1.0], 12.0), Hit::Corner(0), "corners beat overlapping edge bands");
    }
}

/// An overlapping corner zone must not steal a closer painted midpoint on a small box.
#[test]
fn small_boxes_keep_their_painted_mid_edge_handles() {
    for side in [8.0, 16.0, 32.0] {
        for zoom in [1.0, 2.0] {
            let q = [[0.0, 0.0], [side, 0.0], [side, side], [0.0, side]].map(|p| p.map(|v| v / zoom));
            let t = session(q);
            for i in 0..4 {
                let (a, b) = (q[i], q[(i + 1) % 4]);
                let midpoint = [(a[0] + b[0]) / 2.0, (a[1] + b[1]) / 2.0];
                assert_eq!(hit(&t, midpoint, 12.0 / zoom), Hit::Edge(i), "side {side}, zoom {zoom}, edge {i}");
                assert_eq!(hit(&t, a, 12.0 / zoom), Hit::Corner(i));
            }
        }
    }
}

/// Match the measured #11097 boundaries and ties on the top edge and one point outside it.
#[test]
fn measured_edge_corner_and_midpoint_zones_at_100_and_200_percent() {
    use Hit::{Corner, Edge};
    let matrix: &[(f64, &[(f64, Hit)])] = &[
        (12.0, &[(3.0, Corner(0)), (4.0, Edge(0)), (5.0, Edge(0)), (6.0, Edge(0)), (9.0, Edge(0)), (10.0, Corner(1))]),
        (24.0, &[(6.0, Corner(0)), (7.0, Edge(0)), (18.0, Edge(0)), (19.0, Corner(1))]),
        (40.0, &[(10.0, Corner(0)), (11.0, Edge(0)), (30.0, Edge(0)), (31.0, Corner(1))]),
        (100.0, &[(11.0, Corner(0)), (12.0, Edge(0))]),
    ];
    for zoom in [1.0_f32, 2.0] {
        let scale = |p: [f64; 2]| p.map(|v| v / f64::from(zoom));
        let xf = view(zoom);
        for &(side, samples) in matrix {
            let quad = [[0.0, 0.0], [side, 0.0], [side, side], [0.0, side]].map(scale);
            let mut app = app(quad, zoom);
            app.ui.tool_options.transform_reference_point = false;
            let t = app.ui.transform.as_ref().unwrap();
            let rect = Rect::from_min_size(Pos2::ZERO, vec2(side as f32, side as f32));
            for y in [0.0, -1.0] {
                for &(x, expected) in samples {
                    let p = scale([x, y]);
                    let icon = match expected {
                        Corner(0) => CursorIcon::ResizeNwSe,
                        Corner(1) => CursorIcon::ResizeNeSw,
                        _ => CursorIcon::ResizeVertical,
                    };
                    assert_eq!(crate::transform_tool::box_hit(&app, t, p, 12.0 / f64::from(zoom)), expected, "{side}, {zoom}, {x}, {y}");
                    assert_eq!(cursor(&app, &xf, p, Modifiers::NONE), Some(icon));
                    assert_eq!(transform_controls_cursor(rect, pos2(x as f32, y as f32)), Some(icon));
                }
            }
        }
    }
}

/// Reproduce inward rounding through the view transform, with a one-pixel interior control.
#[test]
fn an_edge_survives_the_actual_f32_view_round_trip() {
    for zoom in [1.0_f32, 2.0] {
        let q = [[100.0, 100.00002], [500.0, 100.00002], [500.0, 300.0], [100.0, 300.0]].map(|p| p.map(|v| v / f64::from(zoom)));
        let t = session(q);
        let xf = view(zoom);
        let p = xf.to_doc(xf.to_screen((180.0 / f64::from(zoom)) as f32, q[0][1] as f32));
        let inward_rounding = (p[1] - q[0][1]) * f64::from(zoom);
        assert!(inward_rounding > 0.0 && inward_rounding < 1e-3, "fixture must expose inward f32 rounding: {inward_rounding}");
        assert_eq!(hit(&t, p, 12.0 / f64::from(zoom)), Hit::Edge(0));
        assert_eq!(hit(&t, [p[0], q[0][1] + 1.0 / f64::from(zoom)], 12.0 / f64::from(zoom)), Hit::Inside, "a whole pixel inside still moves");
    }
}

/// The same small-edge press moves the Distort edge and stretches one axis in legacy unlinked scaling.
#[test]
fn a_small_edge_keeps_distort_and_legacy_aspect_behavior() {
    for mode in [TransformMode::Distort, TransformMode::Free] {
        let mut app = app(QUAD, 1.0);
        app.ui.transform = None;
        app.ui.extras.snap = false;
        app.ui.transform_unlinked = true;
        app.session.edit_prefs(|p| p.general.use_legacy_free_transform = true);
        app.session.execute("shape.create", serde_json::json!({"kind":"rect","rect":[8,8,16,16],"fill":"#ff0000"})).unwrap();
        crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
        app.ui.transform.as_mut().unwrap().mode = mode;
        let before = app.ui.transform.as_ref().unwrap().quad;
        for event in [
            crate::canvas::ToolEvent::Down { x: 24.0, y: 16.0, pressure: 1.0 },
            crate::canvas::ToolEvent::Move { x: 40.0, y: 20.0, pressure: 1.0 },
            crate::canvas::ToolEvent::Up { x: 40.0, y: 20.0 },
        ] {
            crate::canvas::tool_event(&mut app, event, Modifiers::NONE);
        }
        let q = app.ui.transform.as_ref().unwrap().quad;
        if mode == TransformMode::Distort {
            let mut expected = before;
            for corner in [1, 2] {
                expected[corner][0] += 16.0;
                expected[corner][1] += 4.0;
            }
            assert_eq!(q, expected, "Distort moves the pressed edge and holds the opposite edge");
        } else {
            assert!((q[2][0] - q[0][0] - 32.0).abs() < 1e-6 && (q[2][1] - q[0][1] - 16.0).abs() < 1e-6, "a legacy edge stretches one axis: {q:?}");
        }
    }
}

/// At 45 degrees, corners and edges exchange cursor axis orientation at both tested zooms.
#[test]
fn turned_box_zones_and_resize_cursors_follow_the_quad() {
    let turn = |p: [f64; 2]| {
        let (x, y) = (p[0] - 300.0, p[1] - 200.0);
        let k = std::f64::consts::FRAC_1_SQRT_2;
        [300.0 + (x - y) * k, 200.0 + (x + y) * k]
    };
    for zoom in [1.0_f32, 2.0] {
        let scale = |p: [f64; 2]| turn(p).map(|v| v / f64::from(zoom));
        let q = QUAD.map(scale);
        let app = app(q, zoom);
        let xf = view(zoom);
        let t = app.ui.transform.as_ref().unwrap();
        for (p, expected, icon) in [
            ([500.0, 100.0], Hit::Corner(1), CursorIcon::ResizeHorizontal),
            ([111.0, 99.0], Hit::Corner(0), CursorIcon::ResizeVertical),
            ([112.0, 99.0], Hit::Edge(0), CursorIcon::ResizeNeSw),
            ([180.0, 95.0], Hit::Edge(0), CursorIcon::ResizeNeSw),
            ([505.0, 150.0], Hit::Edge(1), CursorIcon::ResizeNwSe),
            ([180.0, 101.0], Hit::Inside, CursorIcon::Default),
            ([180.0, -20.0], Hit::Outside, CursorIcon::None),
        ] {
            assert_eq!(hit(t, scale(p), 12.0 / f64::from(zoom)), expected);
            assert_eq!(cursor(&app, &xf, scale(p), Modifiers::NONE), Some(icon), "zoom {zoom}, {p:?}");
        }
        assert_eq!(hit(t, scale([100.0, 100.0]), 12.0 / f64::from(zoom)), Hit::Corner(0));
    }
}

/// Skewed edges and reversed winding must use actual segment geometry for hits and turn direction.
#[test]
fn distorted_and_flipped_quads_use_their_edges_not_an_axis_aligned_box() {
    let q = [[100.0, 100.0], [500.0, 140.0], [460.0, 300.0], [80.0, 260.0]];
    for q in [q, [q[1], q[0], q[3], q[2]]] {
        let t = session(q);
        assert_eq!(hit(&t, [220.0, 112.0], 12.0), Hit::Edge(0));
        assert_eq!(hit(&t, [220.0, 107.0], 12.0), Hit::Edge(0));
        assert_eq!(hit(&t, [220.0, 113.0], 12.0), Hit::Inside);
        assert_eq!(hit(&t, [220.0, 90.0], 12.0), Hit::Outside);
        near(TransformSession::box_turn_direction(&q, [220.0, 90.0]), [-0.1 / 1.01_f64.sqrt(), 1.0 / 1.01_f64.sqrt()]);
    }
}

/// A concave quad whose corner mean is beyond an edge rejects center-based normal orientation.
#[test]
fn concave_quad_edge_direction_uses_winding_in_both_orientations() {
    let q = [[0.0, 0.0], [100.0, 0.0], [10.0, 10.0], [0.0, 100.0]];
    for q in [q, [q[1], q[0], q[3], q[2]]] {
        near(TransformSession::box_turn_direction(&q, [56.0, 14.0]), [-1.0 / 82.0_f64.sqrt(), -9.0 / 82.0_f64.sqrt()]);
    }
}

/// Each outside position selects its inward turn orientation and hides the system cursor.
#[test]
fn outside_rotation_has_eight_edge_and_corner_orientations() {
    let app = app(QUAD, 1.0);
    let h = std::f64::consts::FRAC_1_SQRT_2;
    for (p, direction) in [
        ([300.0, -20.0], [0.0, 1.0]),
        ([620.0, -20.0], [-h, h]),
        ([620.0, 200.0], [-1.0, 0.0]),
        ([620.0, 420.0], [-h, -h]),
        ([300.0, 420.0], [0.0, -1.0]),
        ([-20.0, 420.0], [h, -h]),
        ([-20.0, 200.0], [1.0, 0.0]),
        ([-20.0, -20.0], [h, h]),
    ] {
        near(rotate_cursor_dir(&app, p), direction);
        assert_eq!(cursor(&app, &view(1.0), p, Modifiers::NONE), Some(CursorIcon::None));
        assert_eq!(cursor(&app, &view(1.0), p, Modifiers::COMMAND), Some(CursorIcon::None));
    }
}

/// A hidden reference point outside the quad must leave both hover and the press free to rotate.
#[test]
fn outside_reference_point_owns_the_pointer_only_when_shown() {
    let p = [80.0, 40.0];
    let pivot = [76.0, 40.0];
    let to = [80.0, 48.0];
    for shown in [false, true] {
        let mut app = app(QUAD, 1.0);
        app.ui.transform = None;
        app.ui.extras.snap = false;
        app.session.execute("shape.create", serde_json::json!({"kind": "rect", "rect": [4, 4, 16, 16], "fill": "#ff0000"})).unwrap();
        crate::transform_tool::begin(&mut app, &egui::Context::default()).unwrap();
        let t = app.ui.transform.as_mut().unwrap();
        t.pivot = pivot;
        let quad = t.quad;
        app.ui.tool_options.transform_reference_point = shown;
        assert_eq!(rotate_cursor_dir(&app, p).is_some(), !shown);
        let icon = if shown { CursorIcon::Crosshair } else { CursorIcon::None };
        assert_eq!(cursor(&app, &view(1.0), p, Modifiers::NONE), Some(icon));

        let down = crate::canvas::ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 };
        crate::transform_tool::pointer(&mut app, down, Modifiers::NONE);
        let drag = crate::canvas::ToolEvent::Move { x: to[0], y: to[1], pressure: 1.0 };
        crate::transform_tool::pointer(&mut app, drag, Modifiers::NONE);
        let t = app.ui.transform.as_ref().unwrap();
        if shown {
            assert_eq!(t.quad, quad, "the reference-point drag leaves the box in place");
            assert_eq!(t.pivot, to);
        } else {
            assert_ne!(t.quad, quad, "the outside drag turns the box");
            assert_eq!(t.pivot, pivot);
        }
    }
}

/// Compare unopened controls with an active session at identical points, including a free interior.
#[test]
fn unopened_move_controls_share_hover_but_keep_the_move_press_band() {
    let app = app(QUAD, 1.0);
    let r = Rect::from_min_max(pos2(100.0, 100.0), pos2(500.0, 300.0));
    for (p, starts_transform) in [
        (pos2(100.0, 100.0), true),
        (pos2(180.0, 95.0), true),
        (pos2(180.0, 89.0), true),
        (pos2(505.0, 150.0), true),
        (pos2(518.0, 150.0), true),
        (pos2(519.0, 150.0), false),
        (pos2(300.0, 70.0), false),
        (pos2(180.0, 101.0), false),
        (pos2(620.0, 200.0), false),
        (pos2(-20.0, -20.0), false),
    ] {
        let d = [f64::from(p.x), f64::from(p.y)];
        assert_eq!(transform_controls_cursor(r, p), cursor(&app, &view(1.0), d, Modifiers::NONE), "{p:?}");
        assert_eq!(transform_controls_hit(r, p), starts_transform, "{p:?}");
        match (transform_controls_turn(r, p), rotate_cursor_dir(&app, d)) {
            (None, None) => {}
            (Some(a), Some(b)) => near(Some([f64::from(a.x) / 16.0, f64::from(a.y) / 16.0]), b),
            pair => panic!("unopened/open rotation disagreement: {pair:?}"),
        }
    }
    assert_eq!(transform_controls_cursor(r, r.center()), Some(CursorIcon::Default));
    assert!(!transform_controls_hit(r, r.center()), "inside remains an ordinary Move drag");
}

fn move_controls_fixture() -> (Harness<'static, PhotocraftApp>, [LayerId; 2]) {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 300, "background": "transparent"})).unwrap();
    let layers = [[60, 60, 80, 60], [230, 150, 40, 40]].map(|rect| {
        app.run("shape.create", json!({"kind": "rect", "rect": rect, "fill": "#ff0000"})).unwrap();
        app.session.active().unwrap().active_layer.unwrap()
    });
    app.run("layer.select", json!({"layer": layers[0].0, "mode": "replace"})).unwrap();
    app.ui.tool = Tool::Move;
    app.ui.tool_options.move_show_transform = true;
    app.ui.tool_options.move_auto_select = true;
    app.ui.tool_options.move_target = "layer".into();
    app.ui.extras.snap = false;
    let mut h = Harness::builder().with_size(vec2(1000.0, 800.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    (h, layers)
}

fn move_pointer(h: &mut Harness<'static, PhotocraftApp>, control: bool, kind: &str, [x, y]: [f32; 2]) {
    if control {
        use crate::control::{ControlRequest, Outcome};
        let (request, _) = ControlRequest::new("ui.pointer", json!({"events": [{"kind": kind, "x": x, "y": y}]}));
        let ctx = h.ctx.clone();
        let Outcome::Done(reply) = crate::control::handle(h.state_mut(), &ctx, &request) else { panic!("expected pointer reply") };
        assert_eq!(reply["ok"], true, "{reply}");
    } else {
        let xf = ViewXform::active(h.state()).unwrap();
        let pos = xf.to_screen(x, y);
        assert!(xf.rect.contains(pos));
        h.hover_at(pos);
        if kind != "move" {
            h.event(egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: kind == "down", modifiers: Modifiers::NONE });
        }
        h.run_steps(2);
    }
}

#[test]
fn move_controls_allow_auto_select_on_another_layer_outside_the_press_band() {
    for control in [false, true] {
        let (mut h, layers) = move_controls_fixture();
        let at = [250.0, 170.0];
        let xf = ViewXform::active(h.state()).unwrap();
        let frame = transform_controls_rect(h.state(), &xf).expect("the first layer has visible transform controls");
        assert!(!frame.expand(18.0).contains(xf.to_screen(at[0], at[1])));
        assert_eq!(h.state().session.active().unwrap().selected_layers(), vec![layers[0]]);
        let before = h.state().session.active().unwrap().doc.clone();
        move_pointer(&mut h, control, "down", at);
        move_pointer(&mut h, control, "up", at);
        let state = h.state().session.active().unwrap();
        assert!(h.state().ui.transform.is_none(), "control={control}: Auto-Select does not start Free Transform");
        assert_eq!(state.selected_layers(), vec![layers[1]], "control={control}");
        assert_eq!(state.doc, before);
    }
}

#[test]
fn move_controls_allow_a_layer_marquee_from_empty_canvas_outside_the_press_band() {
    for control in [false, true] {
        let (mut h, layers) = move_controls_fixture();
        let start = [190.0, 125.0];
        let end = [285.0, 210.0];
        let xf = ViewXform::active(h.state()).unwrap();
        let frame = transform_controls_rect(h.state(), &xf).expect("the first layer has visible transform controls");
        assert!(!frame.expand(18.0).contains(xf.to_screen(start[0], start[1])));
        assert_eq!(h.state().session.active().unwrap().selected_layers(), vec![layers[0]]);
        let before = h.state().session.active().unwrap().doc.clone();
        move_pointer(&mut h, control, "down", start);
        move_pointer(&mut h, control, "move", end);
        assert!(h.state().layer_box.is_some(), "control={control}: the empty-canvas drag starts a layer marquee");
        move_pointer(&mut h, control, "up", end);
        let state = h.state().session.active().unwrap();
        assert!(h.state().ui.transform.is_none(), "control={control}: the marquee does not start Free Transform");
        assert!(h.state().layer_box.is_none(), "the marquee ends on release");
        assert_eq!(state.selected_layers(), vec![layers[1]], "control={control}");
        assert_eq!(state.doc, before);
    }
}

/// A press away from midpoint handles must scale an anchored preview without creating a history edit.
#[test]
fn move_whole_edge_press_enters_scaling_at_both_zooms() {
    for zoom in [1.0, 2.0] {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.session.execute("file.new", serde_json::json!({"width": 640, "height": 480})).unwrap();
        app.session.execute("shape.create", serde_json::json!({"kind": "rect", "rect": [100, 100, 400, 200], "fill": "#ff0000"})).unwrap();
        app.sync_views();
        app.ui.views[0].zoom = zoom;
        app.ui.tool = Tool::Move;
        app.ui.tool_options.move_show_transform = true;
        app.ui.extras.snap = false;
        let xf = view(zoom);
        let ctx = egui::Context::default();
        let before = app.session.active().unwrap().doc.clone();
        let steps = app.session.active().unwrap().history.past_len();
        // Away from the top edge's midpoint and corners: the native canvas first opens the
        // controls here, then forwards the same document-space press to the transform tool.
        assert_eq!(begin_transform_controls_at(&mut app, &ctx, &xf, xf.to_screen(180.0, 100.0)), ControlsPress::Started);
        for event in [
            crate::canvas::ToolEvent::Down { x: 180.0, y: 100.0, pressure: 1.0 },
            crate::canvas::ToolEvent::Move { x: 180.0, y: 90.0, pressure: 1.0 },
            crate::canvas::ToolEvent::Up { x: 180.0, y: 90.0 },
        ] {
            crate::canvas::tool_event(&mut app, event, Modifiers::SHIFT);
        }
        let t = app.ui.transform.as_ref().expect("a handle release retains the transform session");
        assert!(t.quad[0][1] < 100.0, "the pressed edge scales");
        assert!((t.quad[2][1] - 300.0).abs() < 1e-6, "the opposite edge anchors scaling instead of moving the whole box");
        assert_eq!(app.session.active().unwrap().doc, before, "a preview does not commit pixels");
        assert_eq!(app.session.active().unwrap().history.past_len(), steps);
    }
}

/// Excluded modes retain cursor ownership, and non-finite coordinates cannot activate controls.
#[test]
fn modes_that_do_not_rotate_and_invalid_points_do_not_hide_the_pointer() {
    let mut app = app(QUAD, 1.0);
    for mode in [TransformMode::Distort, TransformMode::Skew] {
        app.ui.transform.as_mut().unwrap().mode = mode;
        assert_eq!(rotate_cursor_dir(&app, [620.0, 200.0]), None);
        assert_eq!(cursor(&app, &view(1.0), [620.0, 200.0], Modifiers::NONE), Some(CursorIcon::Default));
    }
    app.ui.transform.as_mut().unwrap().mode = TransformMode::Free;
    app.ui.transform.as_mut().unwrap().content_aware = Some(ContentAwareOptions::default());
    assert_eq!(rotate_cursor_dir(&app, [620.0, 200.0]), None);
    assert_eq!(cursor(&app, &view(1.0), [620.0, 200.0], Modifiers::NONE), Some(CursorIcon::Default));
    app.ui.transform.as_mut().unwrap().content_aware = None;
    let t = app.ui.transform.as_mut().unwrap();
    t.warp = Some(photocraft_geom::warp::Warp::none(t.rect));
    assert_eq!(rotate_cursor_dir(&app, [620.0, 200.0]), None);
    assert_eq!(cursor(&app, &view(1.0), [620.0, 200.0], Modifiers::NONE), Some(CursorIcon::Crosshair), "Warp retains its own cursor");
    app.ui.transform.as_mut().unwrap().warp = None;
    assert_eq!(rotate_cursor_dir(&app, [f64::NAN, 200.0]), None);
    assert_eq!(TransformSession::box_turn_direction(&[[0.0, 0.0]; 4], [20.0, 20.0]), None);
    let r = Rect::from_min_max(pos2(100.0, 100.0), pos2(500.0, 300.0));
    for p in [pos2(f32::NAN, 200.0), pos2(f32::INFINITY, 200.0)] {
        assert_eq!(transform_controls_cursor(r, p), None, "invalid points cannot hide the pointer");
        assert_eq!(transform_controls_turn(r, p), None);
        assert!(!transform_controls_hit(r, p), "invalid points cannot activate the overlay");
    }
}

/// Captured rotation must retain feedback after crossing into the box, independent of hover zones.
#[test]
fn rotate_drag_keeps_the_arrow_when_the_pointer_enters_the_box() {
    let mut app = app(QUAD, 1.0);
    app.ui.transform = None;
    app.session.execute("shape.create", serde_json::json!({"kind": "rect", "rect": [4, 4, 16, 16], "fill": "#ff0000"})).unwrap();
    let ctx = egui::Context::default();
    crate::transform_tool::begin(&mut app, &ctx).unwrap();
    crate::transform_tool::pointer(&mut app, crate::canvas::ToolEvent::Down { x: 80.0, y: 40.0, pressure: 1.0 }, Modifiers::NONE);
    crate::transform_tool::pointer(&mut app, crate::canvas::ToolEvent::Move { x: 10.0, y: 10.0, pressure: 1.0 }, Modifiers::NONE);
    assert!(rotate_cursor_dir(&app, [10.0, 10.0]).is_some(), "the held rotation gesture owns the arrow even inside");
    assert_eq!(cursor(&app, &view(1.0), [10.0, 10.0], Modifiers::NONE), Some(CursorIcon::None));
}
