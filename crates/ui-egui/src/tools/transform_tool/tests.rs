use super::*;

fn session() -> TransformSession {
    TransformSession {
        session: 1,
        layer: 1,
        rect: [0.0, 0.0, 100.0, 50.0],
        quad: corners([0.0, 0.0, 100.0, 50.0]),
        pivot: [50.0, 25.0],
        interpolation: "bicubic".into(),
        warp: None,
        selection: false,
        target: None,
        path: None,
        made: None,
        mode: Default::default(),
        content_aware: None,
        skew_v: 0.0,
        layers: Vec::new(),
    }
}

#[test]
fn a_plain_click_far_outside_accepts_transform_but_a_rotation_drag_does_not() {
    let context = egui::Context::default();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    begin(&mut app, &context).unwrap();
    let before = app.session.active().unwrap().history.past_len();
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + 6.0, y]);
    let outside = [44.0, 44.0];
    let none = egui::Modifiers::NONE;
    assert!(pointer(&mut app, ToolEvent::Down { x: outside[0], y: outside[1], pressure: 1.0 }, none));
    assert!(app.ui.transform.is_some(), "press alone does not accept");
    assert!(pointer(&mut app, ToolEvent::Up { x: outside[0], y: outside[1] }, none));
    assert!(app.ui.transform.is_none(), "a release without a drag commits");
    assert_eq!(app.session.active().unwrap().history.past_len(), before + 1, "one undo step for the transform");
    let st = app.session.active().unwrap();
    let layer = st.doc.layer(st.active_layer.unwrap()).unwrap();
    assert_eq!(layer.surface().unwrap().content_bounds(), photocraft_geom::Rect::new(14, 8, 30, 24));

    // An outside drag remains a rotation gesture; releasing it does not commit.
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    begin(&mut app, &context).unwrap();
    let start = [44.0, 44.0];
    let end = [44.0, 54.0];
    let original = app.ui.transform.as_ref().unwrap().quad;
    pointer(&mut app, ToolEvent::Down { x: start[0], y: start[1], pressure: 1.0 }, none);
    pointer(&mut app, ToolEvent::Move { x: end[0], y: end[1], pressure: 1.0 }, none);
    pointer(&mut app, ToolEvent::Up { x: end[0], y: end[1] }, none);
    assert!(app.ui.transform.is_some(), "rotation stays active after an outside drag");
    assert_ne!(app.ui.transform.as_ref().unwrap().quad, original);
}

#[test]
fn a_click_near_rotation_handles_accepts_but_drags_and_option_pivot_do_not() {
    let t = session();
    let tol = 12.0;
    let near = Gesture { hit: Hit::Outside, start: [-18.0, -18.0], quad0: t.quad, pivot0: t.pivot };
    assert!(accepts_outside_click(near, near.start, tol), "a click in the rotation zone accepts; a drag still rotates");
    let far = Gesture { start: [500.0, 500.0], ..near };
    assert!(accepts_outside_click(far, far.start, tol));
    assert!(!accepts_outside_click(far, [512.0, 500.0], tol), "drag does not commit");
    let handle = Gesture { hit: Hit::Corner(0), ..near };
    assert!(!accepts_outside_click(handle, handle.start, tol), "resize handles stay interactive");

    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    begin(&mut app, &egui::Context::default()).unwrap();
    let p = [44.0, 44.0];
    pointer(&mut app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::ALT);
    pointer(&mut app, ToolEvent::Up { x: p[0], y: p[1] }, egui::Modifiers::ALT);
    assert!(app.ui.transform.is_some(), "Option-click moves the pivot, never accepts");
    assert_eq!(app.ui.transform.as_ref().unwrap().pivot, p);
}

#[test]
fn rotate_feedback_only_appears_just_outside_corners() {
    let mut t = session();
    assert!(near_rotate_corner(&t, [-18.0, -18.0], 12.0));
    assert!(!near_rotate_corner(&t, [50.0, 25.0], 12.0), "inside moves the box");
    assert!(!near_rotate_corner(&t, [500.0, 500.0], 12.0), "far outside stays generic");
    assert!(!near_rotate_corner(&t, [2.0, 2.0], 12.0), "corner handle resizes");
    t.mode = TransformMode::Distort;
    assert!(!near_rotate_corner(&t, [-18.0, -18.0], 12.0));
}

#[test]
fn angle_readout_tracks_rotated_quad_and_handles_degenerate_edges() {
    let mut t = session();
    assert_eq!(rotation_degrees(t.quad), 0.0);
    let pivot = t.pivot;
    t.quad = t.quad.map(|q| [pivot[0] - (q[1] - pivot[1]), pivot[1] + (q[0] - pivot[0])]);
    assert!((rotation_degrees(t.quad) - 90.0).abs() < 1e-9);
    t.quad[1] = t.quad[0];
    assert_eq!(rotation_degrees(t.quad), 0.0);
}

#[test]
fn transform_bar_pins_warp_cancel_and_commit() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.transform = Some(session());
    let width = 720.0;
    let mut h = Harness::builder().with_size(vec2(width, 48.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            ui.horizontal_centered(|ui| options_bar(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(6);
    let commit = h.get_by_label("Commit transform").rect();
    let cancel = h.get_by_label("Cancel transform").rect();
    let mode = h.get_by_label("Switch between free transform and warp modes").rect();
    assert!(commit.right() > width - 36.0, "commit sits on the right: {commit:?}");
    assert!(cancel.right() < commit.left() + 1.0, "cancel is left of commit");
    assert!(mode.right() < cancel.left() + 1.0, "warp switch is left of cancel");
    h.get_by_label("Switch between free transform and warp modes").click();
    h.run_steps(2);
    assert!(h.state().ui.transform.as_ref().unwrap().warp.is_some());
    h.get_by_label("Switch between free transform and warp modes").click();
    h.run_steps(2);
    assert!(h.state().ui.transform.as_ref().unwrap().warp.is_none());
    h.get_by_label("Cancel transform").click();
    h.run_steps(2);
    assert!(h.state().ui.transform.is_none());
}

#[test]
fn interpolation_selection_is_reused_for_the_next_transform() {
    use egui::accesskit::Role;
    use egui_kittest::{Harness, kittest::Queryable};

    let app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let mut h = Harness::builder().with_size(vec2(2400.0, 360.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                ui.horizontal_centered(|ui| options_bar(app, ui));
            }
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    let ctx = h.ctx.clone();
    begin(h.state_mut(), &ctx).unwrap();
    h.run_steps(6);
    assert!(h.state().ui.transform.is_some(), "transform stays open while the bar renders");

    h.get_by_role(Role::ComboBox).click();
    h.run();
    assert!(h.state().ui.transform.is_some(), "opening interpolation dropdown keeps the transform open");
    h.key_press(egui::Key::ArrowDown);
    h.run();
    assert!(h.state().ui.transform.is_some(), "first interpolation step keeps the transform open");
    h.key_press(egui::Key::ArrowDown);
    h.run();
    assert!(h.state().ui.transform.is_some(), "second interpolation step keeps the transform open");
    assert_eq!(h.state().ui.transform.as_ref().unwrap().interpolation, "nearest");
    assert_eq!(h.state().ui.tool_options.transform_interpolation, Some(photocraft_engine::prefs::Interpolation::Nearest));
    assert_eq!(h.state().session.prefs().general.image_interpolation, photocraft_engine::prefs::Interpolation::BicubicAutomatic);

    let transform = h.state_mut().ui.transform.as_mut().unwrap();
    transform.quad = transform.quad.map(|[x, y]| [x + 1.0, y]);
    commit(h.state_mut());
    assert!(h.state().ui.transform.is_none());
    begin(h.state_mut(), &ctx).unwrap();
    assert_eq!(h.state().ui.transform.as_ref().unwrap().interpolation, "nearest");
}

#[test]
fn free_transform_bar_buttons_follow_the_fields_on_a_wide_bar() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.transform = Some(session());
    let width = 2400.0;
    let mut h = Harness::builder().with_size(vec2(width, 48.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            ui.horizontal_centered(|ui| options_bar(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(6);
    let commit = h.get_by_label("Commit transform").rect();
    assert!(commit.right() < width * 0.7, "commit sits next to the fields, not at the far right: {commit:?}");
}

#[test]
fn warp_bar_buttons_follow_the_fields_on_a_wide_bar() {
    use egui_kittest::Harness;
    use egui_kittest::kittest::Queryable;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.transform = Some(session());
    let width = 2000.0;
    let mut h = Harness::builder().with_size(vec2(width, 48.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            ui.horizontal_centered(|ui| options_bar(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(6);
    h.get_by_label("Switch between free transform and warp modes").click();
    h.run_steps(4);
    assert!(h.state().ui.transform.as_ref().unwrap().warp.is_some());
    let commit = h.get_by_label("Commit transform").rect();
    assert!(commit.right() < width / 2.0, "commit sits next to the fields, not at the far right: {commit:?}");
    let reset = h.get_by_label("Reset warp").rect();
    assert!(reset.right() < commit.left() + 1.0, "reset is left of commit");
    h.get_by_label("Reset warp").click();
    h.run_steps(2);
    assert!(h.state().ui.transform.as_ref().unwrap().warp.as_ref().unwrap().is_identity());
}

fn eq6(a: f64, b: f64) -> bool {
    (a - b).abs() < 1e-6
}

/// Scale, skew and rotation read back exactly from the box they make, flips included.
#[test]
fn parts_round_trip() {
    for (sx, sy, angle, h, v) in [(1.0, 1.0, 0.0, 0.0, 0.0), (1.5, 0.5, 30.0, 20.0, 0.0), (0.8, -1.2, -120.0, -35.0, 15.0), (2.0, 1.0, 179.0, 0.0, -60.0)] {
        let p = Parts { sx, sy, angle: f64::to_radians(angle), skew_h: f64::to_radians(h), skew_v: f64::to_radians(v) };
        let back = Parts::from_columns(p.columns_of(), p.skew_v);
        for (a, b) in [(back.sx, p.sx), (back.sy, p.sy), (back.angle, p.angle), (back.skew_h, p.skew_h)] {
            assert!(eq6(a, b), "{p:?} read back as {back:?}");
        }
    }
}

/// A box sheared sideways keeps W, H and the angle; only H skew moves (it used to read as a
/// taller box).
#[test]
fn a_sheared_box_reads_as_horizontal_skew() {
    let mut t = session();
    // The 100×50 frame with its bottom slid 50·tan 20° to the left: a positive H skew.
    let d = 50.0 * 20f64.to_radians().tan();
    t.quad = [[0.0, 0.0], [100.0, 0.0], [100.0 - d, 50.0], [-d, 50.0]];
    let (w, h, angle, skew) = readout(&t);
    assert!(eq6(w, 100.0) && eq6(h, 100.0) && eq6(angle, 0.0), "{w} {h} {angle}");
    assert!(eq6(skew, 20.0), "{skew}");
}

/// The test box flipped horizontally and vertically, as Edit › Transform › Flip leaves it.
const FLIPPED_H: [[f64; 2]; 4] = [[100.0, 0.0], [0.0, 0.0], [0.0, 50.0], [100.0, 50.0]];
const FLIPPED_V: [[f64; 2]; 4] = [[0.0, 50.0], [100.0, 50.0], [100.0, 0.0], [0.0, 0.0]];

/// Photoshop puts a flip's minus on the flipped axis and keeps the angle at 0: Flip Horizontal
/// reads W -100 %, Flip Vertical H -100 % (it read 180° and nothing at all).
#[test]
fn a_flipped_box_reads_as_a_negative_scale_on_that_axis() {
    let mut t = session();
    for (quad, want) in [(FLIPPED_H, (-100.0, 100.0)), (FLIPPED_V, (100.0, -100.0))] {
        t.quad = quad;
        let (w, h, angle, skew) = readout(&t);
        assert!(eq6(w, want.0) && eq6(h, want.1) && eq6(angle, 0.0) && eq6(skew, 0.0), "{quad:?}: {w} {h} {angle} {skew}");
    }
    // Flipped both ways is a half turn, not a mirror.
    t.quad = [[100.0, 50.0], [0.0, 50.0], [0.0, 0.0], [100.0, 0.0]];
    let (w, h, angle, _) = readout(&t);
    assert!(eq6(w, 100.0) && eq6(h, 100.0) && eq6(angle.abs(), 180.0), "{w} {h} {angle}");
}

/// The bar's fields act on a flipped box without undoing the flip: angle 0 keeps it as it is,
/// and a typed W scales it and keeps it mirrored.
#[test]
fn bar_fields_keep_a_flipped_box_flipped() {
    let flip_h = |app: &mut PhotocraftApp| app.ui.transform.as_mut().unwrap().quad = FLIPPED_H;
    let t = type_into_bar_field_with(flip_h, 4, "0");
    assert!(t.quad.iter().flatten().zip(FLIPPED_H.iter().flatten()).all(|(a, b)| eq6(*a, *b)), "{:?}", t.quad);
    let t = type_into_bar_field_with(flip_h, 2, "-50");
    let (w, h, angle, _) = readout(&t);
    assert!(eq6(w, -50.0) && eq6(h, 50.0) && eq6(angle, 0.0), "{w} {h} {angle}");
    assert!(eq6(t.quad[0][0] - t.quad[1][0], 50.0), "50 px wide, still mirrored: {:?}", t.quad);
    let flip_v = |app: &mut PhotocraftApp| app.ui.transform.as_mut().unwrap().quad = FLIPPED_V;
    let t = type_into_bar_field_with(flip_v, 3, "-200");
    let (w, h, _, _) = readout(&t);
    assert!(eq6(w, 200.0) && eq6(h, -200.0), "{w} {h}");
}

/// Typing H and V skews leans the box about the reference point; the angle, W and H fields keep
/// their values, and V stays as typed.
#[test]
fn skew_fields_lean_the_box_about_the_reference_point() {
    let t = type_into_bar_field(5, "20");
    let (w, h, angle, skew) = readout(&t);
    assert!(eq6(skew, 20.0) && eq6(angle, 0.0) && eq6(w, 100.0) && eq6(h, 100.0), "{w} {h} {angle} {skew}");
    // The pivot (50, 25) stays put: the top edge slid right, the bottom left, by the same amount.
    let d = 25.0 * 20f64.to_radians().tan();
    assert!(eq6(t.quad[0][0], d) && eq6(t.quad[0][1], 0.0) && eq6(t.quad[3][0], -d) && eq6(t.quad[3][1], 50.0), "{:?}", t.quad);

    let t = type_into_bar_field(6, "-10");
    assert!(eq6(t.skew_v, -10.0));
    let (w, h, angle, skew) = readout(&t);
    assert!(eq6(skew, 0.0) && eq6(angle, 0.0) && eq6(w, 100.0) && eq6(h, 100.0), "{w} {h} {angle} {skew}");
    // The right side rose: tan(−10°) per pixel across, about the pivot.
    let d = 50.0 * (-10f64).to_radians().tan();
    assert!(eq6(t.quad[1][1], d) && eq6(t.quad[0][1], -d), "{:?}", t.quad);
    // A skew past ±90° would flatten the box: the field stops at the limit.
    let t = type_into_bar_field(5, "120");
    assert!(eq6(readout(&t).3, f64::from(SKEW_LIMIT_DEG)), "{}", readout(&t).3);
}

/// Types `text` + Enter into the `n`th numeric field of the transform bar (X, Y, W, H, angle…).
fn type_into_bar_field(n: usize, text: &str) -> TransformSession {
    type_into_bar_field_with(|_| {}, n, text)
}

/// The transform bar over the test box, after `setup` (e.g. picking the W/H unit). Tall enough
/// for the fields' unit menu to open below the bar unscrolled.
fn bar_harness(setup: impl FnOnce(&mut PhotocraftApp)) -> egui_kittest::Harness<'static, PhotocraftApp> {
    use egui_kittest::Harness;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.ui.transform = Some(session());
    setup(&mut app);
    let mut h = Harness::builder().with_size(vec2(900.0, 360.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            if !ui.ctx().fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            ui.horizontal_centered(|ui| options_bar(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::Studio);
    h.run_steps(6);
    h
}

/// [`type_into_bar_field`] after `setup`.
fn type_into_bar_field_with(setup: impl FnOnce(&mut PhotocraftApp), n: usize, text: &str) -> TransformSession {
    use egui_kittest::kittest::Queryable;
    let mut h = bar_harness(setup);
    h.get_all_by_role(egui::accesskit::Role::SpinButton).nth(n).unwrap().click();
    h.run();
    for c in text.chars() {
        h.event(egui::Event::Text(c.to_string()));
        h.run();
    }
    h.key_press(egui::Key::Enter);
    h.run_steps(2);
    h.state().ui.transform.clone().unwrap()
}

/// The values the W and H fields show (the 3rd and 4th numeric fields).
fn shown_size(h: &egui_kittest::Harness<'static, PhotocraftApp>) -> Vec<String> {
    use egui_kittest::kittest::{NodeT, Queryable};
    h.get_all_by_role(egui::accesskit::Role::SpinButton).skip(2).take(2).map(|n| n.accesskit_node().value().unwrap_or_default().to_string()).collect()
}

#[test]
fn size_fields_convert_between_percent_and_lengths() {
    use crate::layer_props_ui::LenUnit;
    use photocraft_engine::prefs::Unit;
    // A 300 px side at 300 ppi is one inch at 100 %.
    let at = |unit| LenUnit { unit, dpi: 300.0, ppi: 72.0 };
    for (unit, at_100) in [
        (Unit::Percent, 100.0),
        (Unit::Pixels, 300.0),
        (Unit::Inches, 1.0),
        (Unit::Centimeters, 2.54),
        (Unit::Millimeters, 25.4),
        (Unit::Points, 72.0),
        (Unit::Picas, 6.0),
    ] {
        let u = at(unit);
        let shown = size_shown(&u, 100.0, 300.0);
        assert!((f64::from(shown) - at_100).abs() < 1e-3, "{unit:?}: {shown}");
        assert!((size_shown(&u, 150.0, 300.0) - shown * 1.5).abs() < 1e-3, "{unit:?} scales with the box");
        assert!((size_pct(&u, shown * 2.0, 300.0) - 200.0).abs() < 1e-3, "{unit:?}: twice the length is 200 %");
        assert!((size_pct(&u, -shown, 300.0) + 100.0).abs() < 1e-3, "{unit:?}: a negative length flips");
    }
    // A degenerate side never divides by zero.
    assert!(size_pct(&at(Unit::Inches), 1.0, 0.0).is_finite());
}

#[test]
fn w_and_h_take_lengths_in_the_unit_picked_for_them() {
    use photocraft_engine::prefs::Unit;
    // The test box is 100 × 50 px; with no document open, lengths convert at 72 ppi.
    let cm = |app: &mut PhotocraftApp| set_size_unit(app, Unit::Centimeters.name());
    let h = bar_harness(cm);
    assert_eq!(shown_size(&h), ["3.53", "1.76"], "the box's size in cm, not 100 %");
    // 7.06 cm is twice the width; linked, the height follows.
    let t = type_into_bar_field_with(cm, 2, "7.06");
    let (sx, sy, _, _) = readout(&t);
    assert!((sx - 200.0).abs() < 0.2 && (sy - 200.0).abs() < 0.2, "{sx} {sy}");
    // Pixels: an exact width.
    let t = type_into_bar_field_with(|app| set_size_unit(app, Unit::Pixels.name()), 2, "250");
    assert!((t.quad[1][0] - t.quad[0][0] - 250.0).abs() < 0.01, "{:?}", t.quad);
    // Percent is still the default.
    assert_eq!(shown_size(&bar_harness(|_| {})), ["100", "100"]);
}

/// Opens a document at `dpi` so the W/H fields convert typed lengths at its resolution.
fn at_dpi(app: &mut PhotocraftApp, dpi: f32) {
    app.session.execute("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.session
        .edit("resolution", |doc, _| {
            doc.resolution_dpi = dpi;
            Ok(())
        })
        .unwrap();
}

/// The box's width and height in px after a W/H edit.
fn box_size(t: &TransformSession) -> (f64, f64) {
    (t.quad[1][0] - t.quad[0][0], t.quad[3][1] - t.quad[0][1])
}

#[test]
fn w_and_h_in_percent_take_a_typed_length() {
    use photocraft_engine::prefs::Unit;
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    // The 100 × 50 px box with W/H showing percent (#6284): a typed length is the box's new side.
    for (text, px) in
        [("40px", 40.0), ("40 px", 40.0), ("2in", 600.0), ("2 in", 600.0), ("5cm", 5.0 / 2.54 * 300.0), ("25.4mm", 300.0), ("72pt", 300.0), ("1pica", 50.0)]
    {
        // Linked: the height follows the width.
        let t = type_into_bar_field_with(|app| at_dpi(app, 300.0), 2, text);
        let (w, h) = box_size(&t);
        assert!(near(w, px) && near(h, px / 2.0), "{text} linked: {w} × {h}");
        // Unlinked: only the width changes.
        let t = type_into_bar_field_with(
            |app| {
                at_dpi(app, 300.0);
                app.ui.transform_unlinked = true;
            },
            2,
            text,
        );
        let (w, h) = box_size(&t);
        assert!(near(w, px) && near(h, 50.0), "{text} unlinked: {w} × {h}");
    }
    // H takes a length too; linked, the width follows.
    let t = type_into_bar_field_with(|app| at_dpi(app, 300.0), 3, "1in");
    let (w, h) = box_size(&t);
    assert!(near(w, 600.0) && near(h, 300.0), "H 1in: {w} × {h}");
    // A plain number is still the percentage.
    let (w, _) = box_size(&type_into_bar_field_with(|app| at_dpi(app, 300.0), 2, "40"));
    assert!(near(w, 40.0), "40 is 40 % of 100 px: {w}");
    // A percentage typed into a length field is that share of the box's original side.
    let px = |app: &mut PhotocraftApp| {
        at_dpi(app, 300.0);
        set_size_unit(app, Unit::Pixels.name());
    };
    let (w, h) = box_size(&type_into_bar_field_with(px, 2, "50%"));
    assert!(near(w, 50.0) && near(h, 25.0), "50% in px: {w} × {h}");
    let inches = |app: &mut PhotocraftApp| {
        at_dpi(app, 300.0);
        set_size_unit(app, Unit::Inches.name());
    };
    let (w, h) = box_size(&type_into_bar_field_with(inches, 3, "300%"));
    assert!(near(w, 300.0) && near(h, 150.0), "300% in inches: {w} × {h}");
}

#[test]
fn w_and_h_reject_or_clamp_hostile_typed_lengths() {
    let near = |a: f64, b: f64| (a - b).abs() < 0.01;
    // Text that is no number leaves the box alone.
    for text in ["px", "in", "NaN", "%"] {
        let (w, h) = box_size(&type_into_bar_field_with(|app| at_dpi(app, 300.0), 2, text));
        assert!(near(w, 100.0) && near(h, 50.0), "{text}: {w} × {h}");
    }
    // Invalid text after a number keeps only what applied while typing (a plain number applies
    // per keystroke), never a NaN or a box past the largest document.
    for text in ["1in/0", "1e400px", "2 inches", "1e308in"] {
        let (w, h) = box_size(&type_into_bar_field_with(|app| at_dpi(app, 300.0), 2, text));
        assert!(w.is_finite() && h.is_finite() && w.abs() <= f64::from(POSITION_LIMIT_PX) + 0.01, "{text}: {w} × {h}");
    }
    // A huge length stops at the field's limit: the box stays within the largest document.
    let (w, h) = box_size(&type_into_bar_field_with(|app| at_dpi(app, 300.0), 2, "1e9in"));
    assert!(w.is_finite() && w <= f64::from(POSITION_LIMIT_PX) + 0.01 && h <= f64::from(POSITION_LIMIT_PX), "{w} × {h}");
    // A negative length flips, as a negative percentage does.
    let (w, _) = box_size(&type_into_bar_field_with(|app| at_dpi(app, 300.0), 2, "-40px"));
    assert!(near(w, -40.0), "{w}");
    // Zero collapses the side without dividing by zero.
    let (w, h) = box_size(&type_into_bar_field_with(|app| at_dpi(app, 300.0), 2, "0px"));
    assert!(w.is_finite() && h.is_finite(), "{w} × {h}");
}

#[test]
fn right_clicking_w_or_h_picks_their_unit_and_remembers_it() {
    use egui_kittest::kittest::Queryable;
    let mut h = bar_harness(|_| {});
    h.get_all_by_role(egui::accesskit::Role::SpinButton).nth(2).unwrap().click_secondary();
    h.run_steps(2);
    h.get_by_label("Inches").click();
    h.run_steps(3);
    assert_eq!(size_unit(h.state()).unit, photocraft_engine::prefs::Unit::Inches);
    assert_eq!(h.state().session.prefs().dialogs[SIZE_UNIT_KEY]["sizeUnit"], "inches");
    assert_eq!(shown_size(&h), ["1.389", "0.694"], "100 × 50 px at 72 ppi");
    // H opens the same menu, with the current unit checked.
    h.get_all_by_role(egui::accesskit::Role::SpinButton).nth(3).unwrap().click_secondary();
    h.run_steps(2);
    assert!(h.query_by_label("✓ Inches").is_some(), "the current unit is checked");
    h.get_by_label("Percent").click();
    h.run_steps(3);
    assert_eq!(shown_size(&h), ["100", "100"]);
    // A hand-edited or stale preference falls back to Percent instead of failing.
    h.state_mut().session.prefs.edit(|p| {
        p.dialogs.insert(SIZE_UNIT_KEY.into(), json!({"sizeUnit": "furlongs"}));
    });
    assert_eq!(size_unit(h.state()).unit, photocraft_engine::prefs::Unit::Percent);
    h.state_mut().session.prefs.edit(|p| {
        p.dialogs.insert(SIZE_UNIT_KEY.into(), json!("not an object"));
    });
    assert_eq!(size_unit(h.state()).unit, photocraft_engine::prefs::Unit::Percent);
    set_size_unit(h.state_mut(), "points");
    assert_eq!(size_unit(h.state()).unit, photocraft_engine::prefs::Unit::Points, "a bad entry is replaced, not kept");
}

#[test]
fn editing_the_contents_of_a_dropped_file_keeps_the_layer() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));

    app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.sync_views();

    app.session.execute("layer.duplicate", json!({"inPlace": true})).unwrap();
    app.sync_views();

    let parent_id = app.session.active().unwrap().doc.id;
    let placed_layer = app.session.active().unwrap().active_layer.unwrap();

    begin_placed(&mut app, &egui::Context::default()).unwrap();

    app.session.execute("layer.smartObjects.editContents", json!({})).unwrap();

    assert_ne!(app.session.active().unwrap().doc.id, parent_id, "Edit Contents should activate the child document");

    end_if_left(&mut app);

    let parent = app.session.documents().iter().find(|st| st.doc.id == parent_id);

    assert!(parent.is_some_and(|st| st.doc.layer(placed_layer).is_some()), "the placed Smart Object must remain in the parent document");
    assert!(app.ui.transform.is_none(), "the transform session should end");
    assert!(app.transform_preview.is_none(), "the transform preview should end");
}

/// Polls the app's background jobs until they have all landed.
fn land_jobs(app: &mut PhotocraftApp) {
    let ctx = egui::Context::default();
    for _ in 0..2000 {
        crate::jobs_ui::tick(app, &ctx);
        if !app.session.has_jobs() {
            return;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    panic!("the background jobs never landed");
}

/// A smart object placed the way a dropped file is (the last step made it), in Free Transform
/// and moved 8 px right, with commits running in the background as in the desktop app:
/// (app, the placed layer, its frame before the move, history steps before the place).
fn moved_place() -> (PhotocraftApp, LayerId, photocraft_geom::Rect, usize) {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    let steps = app.session.active().unwrap().history.past_len();
    app.session.execute("layer.duplicate", json!({"inPlace": true})).unwrap();
    app.sync_views();
    let st = app.session.active().unwrap();
    let placed = st.active_layer.unwrap();
    let frame = photocraft_engine::transform_cmds::transform_frame(&st.doc, st.doc.layer(placed).unwrap());
    begin_placed(&mut app, &egui::Context::default()).unwrap();
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + 8.0, y]);
    app.background_jobs = true;
    (app, placed, frame, steps)
}

fn frame_of(app: &PhotocraftApp, layer: LayerId) -> Option<photocraft_geom::Rect> {
    let st = app.session.active()?;
    Some(photocraft_engine::transform_cmds::transform_frame(&st.doc, st.doc.layer(layer)?))
}

/// #5665: picking another tool or selecting another layer while a dropped file is in Free
/// Transform applies the transform when the commit runs in the background too. Ending the
/// box used to cancel that commit, which took the dropped file back (or left a transformed
/// layer where it was).
#[test]
fn another_tool_or_layer_applies_a_transform_committed_in_the_background() {
    for by_layer in [false, true] {
        let (mut app, placed, frame, steps) = moved_place();
        if by_layer {
            let below = app.session.active().unwrap().doc.layers.iter().map(|l| l.id).find(|id| *id != placed).unwrap();
            app.run("layer.select", json!({"layer": below.0})).unwrap();
        } else {
            app.ui.tool = if app.ui.tool == Tool::Brush { Tool::Move } else { Tool::Brush };
        }
        end_if_left(&mut app);
        assert!(committing(&app).is_some(), "the commit runs in the background");
        land_jobs(&mut app);
        assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the box ended");
        assert_eq!(frame_of(&app, placed), Some(frame.translate(8, 0)), "the dropped file stays, moved (by layer: {by_layer})");
        let st = app.session.active().unwrap();
        assert_eq!(st.history.past_len(), steps + 1, "the place and its move are one step");
        assert_eq!(st.history.undo_label(), Some(photocraft_engine::file_cmds::PLACE_EMBEDDED));
    }
}

/// #5667: New Layer while a dropped file is in Free Transform applies the transform first. When
/// the commit runs in the background the command is refused until it has landed, then runs: the
/// place (which takes the move into its step) and the new layer each stay their own step, in
/// that order.
#[test]
fn new_layer_applies_the_open_transform() {
    let (mut app, placed, frame, steps) = moved_place();
    let layers = app.session.active().unwrap().doc.layers.len();
    let err = app.run("layer.new.layer", json!({})).unwrap_err();
    assert!(err.contains("Free Transform is still being applied"), "refused while the commit runs: {err}");
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "no layer yet");
    land_jobs(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the box ended");
    assert_eq!(frame_of(&app, placed), Some(frame.translate(8, 0)), "applied as it showed, not the original");
    app.run("layer.new.layer", json!({})).unwrap();
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.layers.len(), layers + 1);
    assert_eq!(st.history.past_len(), steps + 2, "the place with its move, then New Layer");
    assert_eq!(st.history.undo_label(), Some("New Layer"));
    app.session.undo();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "one undo takes back the new layer");
    assert_eq!(frame_of(&app, placed), Some(frame.translate(8, 0)), "the transform stays");
}

/// #5665, the report's second case through the UI with background jobs on, as in the desktop
/// app: Free Transform on a layer, move it, then click another layer's row in the Layers panel.
#[test]
fn clicking_another_layer_row_applies_the_transform() {
    use egui_kittest::kittest::{NodeT, Queryable};
    let app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let layer = app.session.active().unwrap().active_layer.unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1400.0, 900.0)).with_max_steps(64).with_step_dt(1.0 / 60.0).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let before = frame_of(h.state(), layer).unwrap();
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    let t = h.state_mut().ui.transform.as_mut().expect("in Free Transform");
    t.quad = t.quad.map(|[x, y]| [x + 8.0, y]);
    h.state_mut().background_jobs = true;
    h.run_steps(2);
    let row =
        h.query_all_by_label("Background").find(|n| n.accesskit_node().role() == egui::accesskit::Role::Button).expect("the Background row").rect().center();
    h.hover_at(row);
    h.run_steps(1);
    h.drag_at(row);
    h.run_steps(1);
    h.drop_at(row);
    for _ in 0..400 {
        h.run_steps(1);
        if h.state().ui.transform.is_none() && !h.state().session.has_jobs() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert_ne!(h.state().session.active().unwrap().active_layer, Some(layer), "the click selected the Background");
    assert!(h.state().ui.transform.is_none(), "the box ended");
    assert_eq!(frame_of(h.state(), layer), Some(before.translate(8, 0)), "the move was applied");
}

/// Large documents need scales past 10000% and positions past 30000 px (#1267).
#[test]
fn transform_fields_reach_large_documents() {
    let t = type_into_bar_field(2, "15000");
    let (sx, sy, _, _) = readout(&t);
    assert!((sx - 15000.0).abs() < 0.5 && (sy - 15000.0).abs() < 0.5, "{sx} {sy}");
    assert!((t.quad[1][0] - t.quad[0][0] - 15_000.0).abs() < 1.0, "{:?}", t.quad);
    let t = type_into_bar_field(0, "250000");
    assert!((t.pivot[0] - 250_000.0).abs() < 1.0, "{:?}", t.pivot);
}

/// A typed scale stops where the box reaches the largest document side, not past what can be
/// allocated: the 100 × 50 px test box tops out at 300000 % wide.
#[test]
fn typed_scales_stay_within_the_largest_document() {
    assert_eq!(scale_limit_pct(20.0), SCALE_LIMIT_PCT);
    assert_eq!(scale_limit_pct(3000.0), 10_000.0);
    assert_eq!(scale_limit_pct(1e9), 100.0);
    assert_eq!(scale_limit_pct(0.0), SCALE_LIMIT_PCT);
    assert_eq!(scale_limit_pct(f64::NAN), SCALE_LIMIT_PCT);
    let t = type_into_bar_field(2, "99999999");
    let (sx, sy, _, _) = readout(&t);
    let side = t.rect[2] - t.rect[0];
    assert!(sx <= f64::from(scale_limit_pct(side)) + 0.5 && sy <= f64::from(scale_limit_pct(t.rect[3] - t.rect[1])) + 0.5, "{sx} {sy}");
}

fn drag(s: &mut TransformSession, from: [f64; 2], to: [f64; 2], mods: egui::Modifiers) {
    let g = Gesture { hit: hit(s, from, 8.0), start: from, quad0: s.quad, pivot0: s.pivot };
    apply_drag(s, g, to, mods);
}

/// A handle drag in Edit › Transform `mode`, as the canvas applies it (on a pixel layer).
fn drag_in(mode: TransformMode, from: [f64; 2], to: [f64; 2], mods: egui::Modifiers) -> [[f64; 2]; 4] {
    drag_on(mode, false, from, to, mods)
}

fn drag_on(mode: TransformMode, type_layer: bool, from: [f64; 2], to: [f64; 2], mods: egui::Modifiers) -> [[f64; 2]; 4] {
    let mut s = session();
    let g = Gesture { hit: hit(&s, from, 8.0), start: from, quad0: s.quad, pivot0: s.pivot };
    apply_mode_drag(&mut s, g, to, mode, type_layer, mods);
    s.quad
}

/// Skew on editable type, checked in Photoshop: a corner can't move alone (type stays a
/// parallelogram), so it slides its whole edge: the top or bottom edge for a mostly
/// horizontal drag, the left or right one for a mostly vertical drag. ⌥ moves the opposite
/// edge the other way. Side handles are as on any layer.
#[test]
fn skew_mode_on_type_slides_the_corners_edge() {
    let (skew, none) = (TransformMode::Skew, egui::Modifiers::NONE);
    assert_eq!(drag_on(skew, true, [100.0, 0.0], [110.0, 3.0], none), [[10.0, 0.0], [110.0, 0.0], [100.0, 50.0], [0.0, 50.0]]);
    assert_eq!(drag_on(skew, true, [100.0, 0.0], [102.0, -9.0], none), [[0.0, 0.0], [100.0, -9.0], [100.0, 41.0], [0.0, 50.0]]);
    assert_eq!(drag_on(skew, true, [100.0, 0.0], [110.0, 3.0], egui::Modifiers::ALT), [[10.0, 0.0], [110.0, 0.0], [90.0, 50.0], [-10.0, 50.0]]);
    assert_eq!(drag_on(skew, true, [50.0, 0.0], [62.0, 7.0], none), [[12.0, 0.0], [112.0, 0.0], [100.0, 50.0], [0.0, 50.0]]);
}

/// Edit › Transform › Skew, checked in Photoshop (#3437): a corner moves alone, horizontally or
/// vertically (whichever way the drag mostly goes); the top and bottom handles slide only
/// horizontally, the left and right ones only vertically; ⌥ moves the opposite corner or edge
/// the other way. Nothing scales.
#[test]
fn skew_mode_moves_a_corner_or_an_edge_along_one_axis() {
    let skew = TransformMode::Skew;
    let none = egui::Modifiers::NONE;
    // Corner (top right), mostly horizontal, then mostly vertical: only that corner moves.
    assert_eq!(drag_in(skew, [100.0, 0.0], [110.0, 3.0], none), [[0.0, 0.0], [110.0, 0.0], [100.0, 50.0], [0.0, 50.0]]);
    assert_eq!(drag_in(skew, [100.0, 0.0], [102.0, -9.0], none), [[0.0, 0.0], [100.0, -9.0], [100.0, 50.0], [0.0, 50.0]]);
    // Top edge: horizontal only, both its corners.
    assert_eq!(drag_in(skew, [50.0, 0.0], [62.0, 7.0], none), [[12.0, 0.0], [112.0, 0.0], [100.0, 50.0], [0.0, 50.0]]);
    // Left edge: vertical only.
    assert_eq!(drag_in(skew, [0.0, 25.0], [6.0, 33.0], none), [[0.0, 8.0], [100.0, 0.0], [100.0, 50.0], [0.0, 58.0]]);
    // ⌥: the opposite corner, and the opposite edge, go the other way.
    let alt = egui::Modifiers::ALT;
    assert_eq!(drag_in(skew, [100.0, 0.0], [110.0, 3.0], alt), [[0.0, 0.0], [110.0, 0.0], [100.0, 50.0], [-10.0, 50.0]]);
    assert_eq!(drag_in(skew, [50.0, 0.0], [62.0, 7.0], alt), [[12.0, 0.0], [112.0, 0.0], [88.0, 50.0], [-12.0, 50.0]]);
    // ⇧ changes nothing in Skew mode.
    let shift = egui::Modifiers::SHIFT;
    assert_eq!(drag_in(skew, [100.0, 0.0], [110.0, 3.0], shift), drag_in(skew, [100.0, 0.0], [110.0, 3.0], none));
    assert_eq!(drag_in(skew, [50.0, 0.0], [62.0, 7.0], shift), drag_in(skew, [50.0, 0.0], [62.0, 7.0], none));
    // A drag outside the box doesn't rotate it in Skew mode.
    assert_eq!(drag_in(skew, [130.0, 25.0], [120.0, 60.0], none), corners([0.0, 0.0, 100.0, 50.0]));
    // Free Transform is unchanged: a corner scales.
    assert_ne!(drag_in(TransformMode::Free, [100.0, 0.0], [110.0, 3.0], none)[2], [100.0, 50.0]);
}

fn close(a: [[f64; 2]; 4], b: [[f64; 2]; 4]) -> bool {
    a.iter().flatten().zip(b.iter().flatten()).all(|(x, y)| (x - y).abs() < 1e-6)
}

/// On a box smaller than the handles' grab areas the nearest handle wins, so the reference
/// point at the centre doesn't take presses on the corners and edges.
#[test]
fn the_nearest_handle_wins_on_a_small_box() {
    let mut s = session();
    s.quad = corners([0.0, 0.0, 16.0, 16.0]);
    s.pivot = [8.0, 8.0];
    assert_eq!(hit(&s, [0.0, 0.0], 12.0), Hit::Corner(0));
    assert_eq!(hit(&s, [16.0, 17.0], 12.0), Hit::Corner(2));
    assert_eq!(hit(&s, [8.0, -1.0], 12.0), Hit::Edge(0));
    assert_eq!(hit(&s, [8.0, 8.0], 12.0), Hit::Pivot);
    assert_eq!(hit(&s, [40.0, 40.0], 12.0), Hit::Outside);
}

#[test]
fn interior_patch_points_are_hidden_and_grid_anchors_stay() {
    assert!(on_section_edge(0, 0) && on_section_edge(1, 0) && on_section_edge(0, 2));
    assert!(!on_section_edge(1, 1) && !on_section_edge(2, 2));
    let b = [0.0, 0.0, 30.0, 30.0];
    let w = Warp::custom(BezierMesh::identity(b, 1, 1), b);
    assert!(warp_hit(&w, [10.0, 10.0], 4.0).is_none(), "the point inside the patch is not shown");
    assert_eq!(warp_hit(&w, [0.0, 0.0], 4.0), Some(0));
    let grid = Warp::custom(BezierMesh::identity(b, 3, 3), b);
    assert_eq!(warp_hit(&grid, [10.0, 10.0], 4.0), Some(33), "a grid-corner anchor stays draggable");
    assert!(warp_hit(&grid, [30.0 / 9.0, 30.0 / 9.0], 2.0).is_none());
}

#[test]
fn grid_menu_reads_default_and_even_presets() {
    let b = [0.0, 0.0, 100.0, 80.0];
    let plain = BezierMesh::identity(b, 1, 1);
    assert_eq!(warp_grid_id(&plain), "default");
    assert_eq!(warp_grid_id(&BezierMesh::identity(b, 3, 3)), "3");
    assert_eq!(warp_grid_id(&BezierMesh::identity(b, 4, 4)), "4");
    assert_eq!(warp_grid_id(&BezierMesh::identity(b, 5, 5)), "5");
    assert_eq!(warp_grid_id(&BezierMesh::identity(b, 2, 2)), "custom");
    let mut split = plain.clone();
    assert!(split.split_u(0.5));
    assert_eq!(warp_grid_id(&split), "custom");
    assert!(single_patch(&plain));
    assert!(!single_patch(&BezierMesh::identity(b, 3, 3)));
}

#[test]
fn split_tool_places_the_line_where_the_pointer_releases() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(8, 8, 32, 32), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    begin_warp(&mut app, &egui::Context::default()).unwrap();
    app.transform_preview.as_mut().unwrap().split_tool = Some(SplitTool::Vertical);
    let knots = |app: &PhotocraftApp| app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().mesh.as_ref().unwrap().us.clone();
    assert_eq!(knots(&app).len(), 2);
    let mods = egui::Modifiers::NONE;
    warp_pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, 2.0, mods);
    warp_pointer(&mut app, ToolEvent::Move { x: 14.0, y: 20.0, pressure: 1.0 }, 2.0, mods);
    assert_eq!(knots(&app).len(), 2, "the line is only a guide until release");
    warp_pointer(&mut app, ToolEvent::Up { x: 14.0, y: 20.0 }, 2.0, mods);
    let us = knots(&app);
    let has = |ks: &[f64], u: f64| ks.iter().any(|k| (k - u).abs() < 0.02);
    // The default grid's thirds stay; the release cuts the section that contains it.
    assert!(has(&us, 0.25) && has(&us, 1.0 / 3.0) && has(&us, 2.0 / 3.0), "{us:?}");
    let vs = app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().mesh.as_ref().unwrap().vs.clone();
    assert!(has(&vs, 1.0 / 3.0) && has(&vs, 2.0 / 3.0), "horizontal sections stay: {vs:?}");
    assert!(app.transform_preview.as_ref().unwrap().split_tool.is_some(), "the tool stays armed");
    // Option-click in the open places both lines, without a button armed.
    app.transform_preview.as_mut().unwrap().split_tool = None;
    let alt = egui::Modifiers::ALT;
    warp_pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, 2.0, alt);
    warp_pointer(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, 2.0, alt);
    let m = app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().mesh.as_ref().unwrap();
    assert!(m.us.len() == us.len() + 1 && has(&m.vs, 0.5), "crosswise cuts the open section: us {:?} vs {:?}", m.us, m.vs);
}

#[test]
fn split_of_a_grid_cuts_one_section_and_keeps_the_rest() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(8, 8, 32, 32), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    begin_warp(&mut app, &egui::Context::default()).unwrap();
    let rect = app.ui.transform.as_ref().unwrap().rect;
    let before = BezierMesh::identity(rect, 3, 3);
    app.ui.transform.as_mut().unwrap().warp = Some(Warp::custom(before.clone(), rect));
    // Middle of the centre column (s = 0.5), inside the top row of cells.
    let x = rect[0] + 0.5 * (rect[2] - rect[0]);
    let y = rect[1] + (1.0 / 6.0) * (rect[3] - rect[1]);
    app.transform_preview.as_mut().unwrap().split_tool = Some(SplitTool::Vertical);
    let mods = egui::Modifiers::NONE;
    warp_pointer(&mut app, ToolEvent::Down { x, y, pressure: 1.0 }, 2.0, mods);
    warp_pointer(&mut app, ToolEvent::Up { x, y }, 2.0, mods);
    let m = app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().mesh.as_ref().unwrap();
    assert_eq!(m.vs.len(), before.vs.len(), "horizontal sections are not rebuilt: {:?}", m.vs);
    for (a, b) in m.vs.iter().zip(&before.vs) {
        assert!((a - b).abs() < 1e-6, "{:?} vs {:?}", m.vs, before.vs);
    }
    assert_eq!(m.us.len(), before.us.len() + 1, "one cut: {:?}", m.us);
    for u in [0.0, 1.0 / 3.0, 0.5, 2.0 / 3.0, 1.0] {
        assert!(m.us.iter().any(|k| (k - u).abs() < 1e-3), "missing {u} in {:?}", m.us);
    }
    let (nx0, nx1) = (before.nx(), m.nx());
    for j in 0..before.ny() {
        for i in 0..3 {
            let a = before.points[j * nx0 + i];
            let b = m.points[j * nx1 + i];
            assert!((a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6, "left section moved");
        }
    }
}

#[test]
fn corner_drag_scales_proportionally_about_opposite_corner() {
    let mut s = session();
    drag(&mut s, [100.0, 50.0], [200.0, 60.0], egui::Modifiers::NONE);
    assert!(close(s.quad, corners([0.0, 0.0, 200.0, 100.0])), "{:?}", s.quad);
    let mut s = session();
    drag(&mut s, [100.0, 50.0], [200.0, 60.0], egui::Modifiers::SHIFT);
    assert!(close(s.quad, corners([0.0, 0.0, 200.0, 60.0])), "shift = free: {:?}", s.quad);
}

/// Photoshop CC 2019 and later: an edge scales proportionally too, about the middle of the
/// opposite edge, and ⇧ stretches its one axis instead.
#[test]
fn edge_drag_scales_proportionally_about_the_opposite_edge() {
    // Right edge: 100 × 50 → 150 × 75; the left edge's middle (0, 25) stays, and so does the
    // reference point's place in the box (its centre).
    let mut s = session();
    drag(&mut s, [100.0, 25.0], [150.0, 25.0], egui::Modifiers::NONE);
    assert!(close(s.quad, corners([0.0, -12.5, 150.0, 62.5])), "{:?}", s.quad);
    assert!(near(s.pivot, [75.0, 25.0]), "{:?}", s.pivot);
    // Top edge to twice the height: twice the width, about x = 50.
    let mut s = session();
    drag(&mut s, [50.0, 0.0], [50.0, -50.0], egui::Modifiers::NONE);
    assert!(close(s.quad, corners([-50.0, -50.0, 150.0, 50.0])), "{:?}", s.quad);
    // Past the opposite edge: the dragged axis flips, the other only shrinks.
    let mut s = session();
    drag(&mut s, [100.0, 25.0], [-50.0, 25.0], egui::Modifiers::NONE);
    assert!(close(s.quad, corners([0.0, 12.5, -50.0, 37.5])), "{:?}", s.quad);
    // ⇧: one axis.
    let mut s = session();
    drag(&mut s, [50.0, 50.0], [50.0, 80.0], egui::Modifiers::SHIFT);
    assert!(close(s.quad, corners([0.0, 0.0, 100.0, 80.0])), "{:?}", s.quad);
}

#[test]
fn alt_scales_edges_about_reference_point() {
    let mut s = session();
    drag(&mut s, [100.0, 25.0], [150.0, 25.0], egui::Modifiers::ALT);
    assert!(close(s.quad, corners([-50.0, -25.0, 150.0, 75.0])), "{:?}", s.quad);
    // ⌥⇧: one axis, about the reference point.
    let mut s = session();
    drag(&mut s, [100.0, 25.0], [150.0, 25.0], egui::Modifiers { alt: true, shift: true, ..Default::default() });
    assert!(close(s.quad, corners([-50.0, 0.0, 150.0, 50.0])), "{:?}", s.quad);
}

/// A reference point moved off the centre (⌥-click) is the centre of ⌥-scaling too, not just
/// of rotation: it stays put and the box scales away from it.
#[test]
fn alt_scales_about_a_moved_reference_point() {
    let alt = egui::Modifiers::ALT;
    // Reference point on the top-left corner: the bottom-right corner to (200, 100) doubles
    // the box, top-left fixed (not centred on the reference point).
    let mut s = session();
    s.pivot = [0.0, 0.0];
    drag(&mut s, [100.0, 50.0], [200.0, 100.0], alt);
    assert!(close(s.quad, corners([0.0, 0.0, 200.0, 100.0])), "{:?}", s.quad);
    assert!(near(s.pivot, [0.0, 0.0]), "{:?}", s.pivot);
    // Reference point at (25, 25): the right edge to x = 175 is 2× about x = 25, and
    // proportionally 2× about y = 25 too.
    let mut s = session();
    s.pivot = [25.0, 25.0];
    drag(&mut s, [100.0, 25.0], [175.0, 25.0], alt);
    assert!(close(s.quad, corners([-25.0, -25.0, 175.0, 75.0])), "{:?}", s.quad);
    // ⌥⇧: that edge's axis only.
    let mut s = session();
    s.pivot = [25.0, 25.0];
    drag(&mut s, [100.0, 25.0], [175.0, 25.0], egui::Modifiers { alt: true, shift: true, ..Default::default() });
    assert!(close(s.quad, corners([-25.0, 0.0, 175.0, 50.0])), "{:?}", s.quad);
    // Reference point in line with the dragged corner horizontally: that axis has nothing to
    // scale against and stays, the other doubles about it.
    let mut s = session();
    s.pivot = [100.0, 50.0];
    drag(&mut s, [100.0, 0.0], [150.0, -50.0], egui::Modifiers { alt: true, shift: true, ..Default::default() });
    assert!(close(s.quad, corners([0.0, -50.0, 100.0, 50.0])), "{:?}", s.quad);
}

fn near(a: [f64; 2], b: [f64; 2]) -> bool {
    (a[0] - b[0]).abs() < 1e-6 && (a[1] - b[1]).abs() < 1e-6
}

/// The reference point is attached to the box (#1329): scaling, skewing, distorting and
/// perspective carry it along to the same place within the box; ⌥-scaling about it and
/// rotating about it leave it put.
#[test]
fn the_reference_point_moves_with_the_box() {
    // Corner scale down: the centre stays the centre.
    let mut s = session();
    drag(&mut s, [100.0, 50.0], [50.0, 25.0], egui::Modifiers::NONE);
    assert!(close(s.quad, corners([0.0, 0.0, 50.0, 25.0])), "{:?}", s.quad);
    assert!(near(s.pivot, [25.0, 12.5]), "{:?}", s.pivot);
    // A reference point set to a corner stays at that corner; an edge drag carries it too.
    let mut s = session();
    s.pivot = [100.0, 0.0];
    drag(&mut s, [50.0, 50.0], [50.0, 80.0], egui::Modifiers::SHIFT);
    assert!(near(s.pivot, [100.0, 0.0]), "{:?}", s.pivot);
    drag(&mut s, [100.0, 40.0], [150.0, 40.0], egui::Modifiers::SHIFT);
    assert!(near(s.pivot, s.quad[1]), "{:?} {:?}", s.pivot, s.quad);
    drag(&mut s, [150.0, 40.0], [180.0, 40.0], egui::Modifiers::NONE);
    assert!(near(s.pivot, s.quad[1]), "proportional edge drag: {:?} {:?}", s.pivot, s.quad);
    // Skew: the centre follows the box's centre.
    let mut s = session();
    drag(&mut s, [50.0, 0.0], [70.0, 0.0], egui::Modifiers::COMMAND);
    assert!(near(s.pivot, [60.0, 25.0]), "{:?}", s.pivot);
    // Distort: a corner reference point stays on its corner.
    let mut s = session();
    s.pivot = [0.0, 50.0];
    drag(&mut s, [0.0, 50.0], [-20.0, 70.0], egui::Modifiers::COMMAND);
    assert!(near(s.pivot, [-20.0, 70.0]), "{:?}", s.pivot);
    // Perspective: the centre stays inside the box, on the box's own centre lines.
    let mut s = session();
    drag(&mut s, [100.0, 0.0], [120.0, 0.0], egui::Modifiers { command: true, alt: true, shift: true, ..Default::default() });
    let h = Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], s.quad).unwrap();
    let (x, y) = h.apply(0.5, 0.5);
    assert!(near(s.pivot, [x, y]), "{:?} vs {:?}", s.pivot, [x, y]);
    // ⌥ scales about the reference point: it stays where it is.
    let mut s = session();
    drag(&mut s, [100.0, 50.0], [150.0, 75.0], egui::Modifiers::ALT);
    assert!(near(s.pivot, [50.0, 25.0]), "{:?}", s.pivot);
    // Rotation turns about it.
    let mut s = session();
    drag(&mut s, [150.0, 25.0], [50.0, 125.0], egui::Modifiers::NONE);
    assert!(near(s.pivot, [50.0, 25.0]), "{:?}", s.pivot);
    // From a box collapsed to a point there is no place within it to keep: it stays put.
    assert_eq!(carry_pivot([[0.0, 0.0]; 4], corners([0.0, 0.0, 10.0, 10.0]), [5.0, 5.0]), [5.0, 5.0]);
}

#[test]
fn move_rotate_and_distort() {
    let mut s = session();
    drag(&mut s, [40.0, 20.0], [50.0, 30.0], egui::Modifiers::NONE);
    assert!(close(s.quad, corners([10.0, 10.0, 110.0, 60.0])));
    assert_eq!(s.pivot, [60.0, 35.0]);
    // Rotate 90° about the centre (drag from right of the box to below it).
    let mut s = session();
    drag(&mut s, [150.0, 25.0], [50.0, 125.0], egui::Modifiers::NONE);
    let (_, _, angle, _) = readout(&s);
    assert!((angle - 90.0).abs() < 1e-6, "{angle}");
    // ⇧ snaps: a 50° drag lands on 45°.
    let mut s = session();
    let a = 50f64.to_radians();
    drag(&mut s, [150.0, 25.0], [50.0 + 100.0 * a.cos(), 25.0 + 100.0 * a.sin()], egui::Modifiers::SHIFT);
    assert!((readout(&s).2 - 45.0).abs() < 1e-6);
    // ⌘ corner = distort.
    let mut s = session();
    drag(&mut s, [100.0, 0.0], [120.0, -10.0], egui::Modifiers::COMMAND);
    assert_eq!(s.quad[1], [120.0, -10.0]);
    assert_eq!(s.quad[0], [0.0, 0.0]);
}

#[test]
fn begin_and_commit_through_the_engine() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(8, 8, 24, 24), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().rect, [8.0, 8.0, 24.0, 24.0]);
    scale_about_pivot(&mut app, 2.0, 2.0);
    commit(&mut app);
    let st = app.session.active().unwrap();
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert!(b.width().abs_diff(32) <= 2 && b.x0.abs_diff(0) <= 1, "{b:?}");
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
}

/// A commit the engine refuses (here a collapsed box; a result over the pixel budget, #3773, is
/// refused the same way) shows the reason and keeps the box open, unchanged; picking another
/// tool then ends it.
#[test]
fn a_refused_commit_keeps_the_box_open() {
    let ctx = egui::Context::default();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let (revision, steps) = (app.session.active().unwrap().revision, app.session.active().unwrap().history.past_len());
    begin(&mut app, &ctx).unwrap();
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = [[16.0, 16.0]; 4];
    }
    commit(&mut app);
    let t = app.ui.transform.as_ref().expect("the box stays open");
    assert_eq!(t.quad, [[16.0, 16.0]; 4], "as the user left it");
    assert!(app.transform_preview.is_some(), "with its preview");
    assert!(app.ui.status.contains("collapses"), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert_eq!((st.revision, st.history.past_len()), (revision, steps), "the document was not changed");
    app.ui.tool = if app.ui.tool == Tool::Brush { Tool::Move } else { Tool::Brush };
    end_if_left(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "another tool ends it");
}

/// #6737: Save for Web exports the committed document, so it is off while a box is pending.
#[test]
fn save_for_web_is_off_under_a_pending_transform() {
    let ctx = egui::Context::default();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    assert!(crate::menus::is_enabled(&app, SAVE_FOR_WEB), "enabled with no box");
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    assert!(!crate::menus::is_enabled(&app, SAVE_FOR_WEB), "off under the box");
    assert!(crate::menus::invoke(&mut app, &ctx, SAVE_FOR_WEB, json!({})).is_err(), "the dialog doesn't open");
    let path = std::env::temp_dir().join(format!("pc-web6737-{}.png", std::process::id())).to_string_lossy().into_owned();
    let r = crate::menus::invoke(&mut app, &ctx, SAVE_FOR_WEB, json!({"path": path, "format": "png24"}));
    assert!(r.is_err() && !std::path::Path::new(&path).exists(), "nor does the export run");
    assert!(app.ui.transform.is_some(), "the box stays open");
    cancel(&mut app);
    assert!(crate::menus::is_enabled(&app, SAVE_FOR_WEB), "back after the box closes");
}

/// #352: ⌥⌘T transforms a copy. OK leaves the original and a moved copy as one history step;
/// Cancel leaves no copy and nothing to redo; with a selection the copy holds the selected
/// pixels only.
#[test]
fn free_transform_a_copy() {
    let ctx = egui::Context::default();
    let bounds = |app: &PhotocraftApp, i: usize| app.session.active().unwrap().doc.layers[i].surface().unwrap().content_bounds();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let (layers, steps) = (app.session.active().unwrap().doc.layers.len(), app.session.active().unwrap().history.past_len());
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransformCopy", json!({})).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().made, Some(MadeLayer::Copy));
    assert!(!crate::menus::is_enabled(&app, "edit.freeTransformCopy"), "one transform at a time");
    // Esc: the copy goes too.
    cancel(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.layers.len(), st.history.past_len()), (layers, steps));
    assert!(!st.history.can_redo(), "nothing to redo");
    // OK: original in place, a moved copy, one step.
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransformCopy", json!({})).unwrap();
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = t.quad.map(|[x, y]| [x + 20.0, y]);
    }
    commit(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.layers.len(), st.history.past_len()), (layers + 1, steps + 1));
    assert_eq!(bounds(&app, layers - 1), photocraft_geom::Rect::new(8, 8, 24, 24), "the original stays");
    assert_eq!(bounds(&app, layers), photocraft_geom::Rect::new(28, 8, 44, 24), "the copy moved");
    app.session.undo();
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers, "one undo removes copy and move");
    // With a selection: Layer via Copy, so only the selected pixels are copied.
    app.run("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 16})).unwrap();
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransformCopy", json!({})).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().rect, [8.0, 8.0, 16.0, 24.0]);
    cancel(&mut app);
    assert_eq!(app.session.active().unwrap().doc.layers.len(), layers);
}

/// #4112: ⌥⌘T on a cylinder smart object, Warp mode, Enter with nothing touched: the copy
/// keeps the cylinder and stays its own step (no warp step to fold into), so one undo takes
/// back the copy and nothing before it.
#[test]
fn untouched_warp_of_a_cylinder_copy_keeps_its_own_step() {
    let ctx = egui::Context::default();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.run("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.run("layer.smartObjects.warp", json!({"style": "cylinder", "values": [8, 24, 24, 8, 0.3, 0.3, 0.5], "rect": [8, 8, 24, 24]})).unwrap();
    let warp = |app: &PhotocraftApp, i: usize| match &app.session.active().unwrap().doc.layers[i].content {
        photocraft_doc::LayerContent::Smart(sm) => sm.warp.as_ref().map(|w| w.style),
        _ => None,
    };
    let (layers, steps) = (app.session.active().unwrap().doc.layers.len(), app.session.active().unwrap().history.past_len());
    assert_eq!(warp(&app, layers - 1), Some(WarpStyle::Cylinder));
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransformCopy", json!({})).unwrap();
    enter_warp(&mut app, &ctx);
    assert!(app.ui.transform.as_ref().unwrap().warp.is_some(), "Warp mode shows the cylinder");
    commit(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.layers.len(), st.history.past_len()), (layers + 1, steps + 1));
    assert_eq!(warp(&app, layers), Some(WarpStyle::Cylinder), "the copy is still a cylinder");
    app.session.undo();
    let st = app.session.active().unwrap();
    assert_eq!((st.doc.layers.len(), st.history.past_len()), (layers, steps), "one undo takes back the copy only");
    assert_eq!(warp(&app, layers - 1), Some(WarpStyle::Cylinder));
}

/// #1099: cancel, frame cleanup and a commit before the next frame all belong to the
/// original document, including after tab reordering and with shared layer IDs.
#[test]
fn cancelling_a_copy_after_switching_documents_preserves_the_active_document() {
    for reorder in [false, true] {
        for finish in [cancel, end_if_left, commit] {
            let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
            let original = app.session.active().unwrap().doc.clone();
            let steps = app.session.active().unwrap().history.past_len();
            begin_copy(&mut app, &egui::Context::default()).unwrap();
            app.ui.transform.as_mut().unwrap().quad = corners([12.0, 8.0, 28.0, 24.0]);
            // Cloning a document deliberately shares layer IDs, but gets a fresh document ID.
            let other = app.session.active().unwrap().doc.as_ref().clone();
            app.session.add_document(other, None);
            app.run("layer.new.layer", json!({})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            assert!(app.session.undo());
            let st = app.session.active().unwrap();
            let (doc, history, redo, revision, layer) =
                (st.doc.clone(), st.history.entries(), st.history.redo_labels().map(str::to_owned).collect::<Vec<_>>(), st.revision, st.active_layer);
            if reorder {
                app.session.move_document(0, 1);
            }
            finish(&mut app);
            assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
            let st = app.session.active().unwrap();
            assert!(Arc::ptr_eq(&st.doc, &doc), "the selected document's pixels are unchanged");
            assert_eq!((st.history.entries(), st.revision, st.active_layer), (history, revision, layer));
            assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), redo);
            let origin = app.session.documents().iter().find(|st| st.doc.id == original.id).unwrap();
            assert!(Arc::ptr_eq(&origin.doc, &original), "only the originating copy is undone");
            assert_eq!(origin.history.past_len(), steps);
            assert!(!origin.history.can_redo());
        }
    }
}

#[test]
fn cancelling_a_copy_after_its_document_closes_does_not_undo_another_document() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    begin_copy(&mut app, &egui::Context::default()).unwrap();
    app.run("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let doc = app.session.active().unwrap().doc.clone();
    let steps = app.session.active().unwrap().history.past_len();
    app.session.close(0);
    end_if_left(&mut app);
    let st = app.session.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc));
    assert_eq!(st.history.past_len(), steps);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
}

#[test]
fn cancelling_an_already_undone_copy_preserves_history_and_redo() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    begin_copy(&mut app, &egui::Context::default()).unwrap();
    assert!(app.session.undo());
    let st = app.session.active().unwrap();
    let (doc, history, redo) = (st.doc.clone(), st.history.entries(), st.history.redo_labels().map(str::to_owned).collect::<Vec<_>>());
    end_if_left(&mut app);
    let st = app.session.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc));
    assert_eq!(st.history.entries(), history);
    assert_eq!(st.history.redo_labels().collect::<Vec<_>>(), redo);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
}

#[test]
fn cancelling_a_copy_does_not_undo_a_later_edit() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    begin_copy(&mut app, &egui::Context::default()).unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    let st = app.session.active().unwrap();
    let (doc, history) = (st.doc.clone(), st.history.entries());
    cancel(&mut app);
    let st = app.session.active().unwrap();
    assert!(Arc::ptr_eq(&st.doc, &doc));
    assert_eq!(st.history.entries(), history);
}

/// #1384: the Layers panel's opacity, fill, visibility and blend edits to the moving layer
/// and to the others show while the box is open, and survive the commit.
#[test]
fn edits_during_a_transform_show_in_the_preview() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.ui.tool = Tool::Move;
    let bg = app.session.active().unwrap().doc.layers[0].id;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    if let Some(t) = h.state_mut().ui.transform.as_mut() {
        t.quad = t.quad.map(|[x, y]| [x + 20.0, y]);
    }
    h.run_steps(2);
    let layer = h.state().ui.transform.as_ref().unwrap().layer;
    assert_eq!(h.state().transform_preview.as_ref().unwrap().opacity, 1.0);

    h.state_mut().run("layer.setProps", json!({"layer": layer, "opacity": 0.5, "fill": 0.6, "blend": "Multiply"})).unwrap();
    h.state_mut().run("layer.setProps", json!({"layer": bg.0, "visible": false})).unwrap();
    h.run_steps(2);
    let app = h.state();
    assert!(app.ui.transform.is_some(), "the edits leave the box open");
    let pv = app.transform_preview.as_ref().unwrap();
    assert!((pv.opacity - 0.3).abs() < 1e-6, "the moving pixels take the new opacity × fill: {}", pv.opacity);
    assert!(!pv.doc.layer(bg).unwrap().visible, "the document under the box follows");
    let moving = pv.doc.layer(LayerId(layer)).unwrap();
    assert!(!moving.visible, "the moving layer stays out of the document under the box");
    assert_eq!(moving.blend, photocraft_color::BlendMode::Multiply);

    h.state_mut().run("layer.setProps", json!({"layer": layer, "visible": false})).unwrap();
    h.run_steps(2);
    assert_eq!(h.state().transform_preview.as_ref().unwrap().opacity, 0.0, "hiding the layer hides the moving pixels");
    h.state_mut().run("layer.setProps", json!({"layer": layer, "visible": true})).unwrap();

    commit(h.state_mut());
    let st = h.state().session.active().unwrap();
    let l = st.doc.layer(LayerId(layer)).unwrap();
    assert_eq!((l.opacity, l.fill_opacity, l.blend), (0.5, 0.6, photocraft_color::BlendMode::Multiply));
    assert_eq!(l.surface().unwrap().content_bounds(), photocraft_geom::Rect::new(28, 8, 44, 24));
}

/// #1384 with a selection: the pixels it leaves behind stay in the rebuilt preview without
/// the lifted ones, and take the layer's new properties.
#[test]
fn edits_during_a_selection_transform_keep_the_lifted_pixels_out() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.ui.tool = Tool::Move;
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 8, "height": 16})).unwrap();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(2);
    let layer = LayerId(h.state().ui.transform.as_ref().unwrap().layer);
    // Alpha in the preview's copy of the layer: in the lifted half (x 8..16) and the rest (16..24).
    let left = |app: &PhotocraftApp| {
        let s = app.transform_preview.as_ref().unwrap().doc.layer(layer).unwrap().surface().unwrap().clone();
        (s.sample_channel(10, 12, 3), s.sample_channel(20, 12, 3))
    };
    assert_eq!(left(h.state()), (0.0, 1.0), "the selected half is lifted out");

    h.state_mut().run("layer.setProps", json!({"layer": layer.0, "opacity": 0.4})).unwrap();
    h.run_steps(2);
    let app = h.state();
    assert_eq!(left(app), (0.0, 1.0), "still without the lifted pixels");
    let moving = app.transform_preview.as_ref().unwrap().doc.layer(layer).unwrap();
    assert!(moving.visible && (moving.opacity - 0.4).abs() < 1e-6, "what stays behind is drawn with the new opacity");
    assert!((app.transform_preview.as_ref().unwrap().opacity - 0.4).abs() < 1e-6);
}

/// #670: picking another tool applies the open transform, as one history step.
#[test]
fn picking_another_tool_applies_the_transform() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.ui.tool = Tool::Move;
    let steps = app.session.active().unwrap().history.past_len();
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    if let Some(t) = h.state_mut().ui.transform.as_mut() {
        t.quad = t.quad.map(|[x, y]| [x + 20.0, y]);
    }
    h.run_steps(2);
    assert!(h.state().ui.transform.is_some(), "open while the tool stays");
    h.key_press(egui::Key::B);
    h.run_steps(2);
    let app = h.state();
    assert_eq!(app.ui.tool, Tool::Brush);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1);
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds(), photocraft_geom::Rect::new(28, 8, 44, 24));
}

/// #1400: selecting another layer while transforming applies the box to the layer it was on,
/// and the next transform starts on the newly selected layer.
#[test]
fn selecting_another_layer_applies_the_transform_and_the_next_one_follows() {
    let ctx = egui::Context::default();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let first = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(40, 40, 56, 60), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let second = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.select", json!({"layer": first.0})).unwrap();
    app.ui.tool = Tool::Move;
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = t.quad.map(|[x, y]| [x + 20.0, y]);
    }
    let steps = app.session.active().unwrap().history.past_len();
    app.run("layer.select", json!({"layer": second.0})).unwrap();
    end_if_left(&mut app);
    assert!(app.ui.transform.is_none() && app.transform_preview.is_none(), "the box left the old layer");
    let st = app.session.active().unwrap();
    assert_eq!(st.active_layer, Some(second));
    assert_eq!(st.history.past_len(), steps + 1, "applied as one step");
    let bounds = |id: LayerId| st.doc.layer(id).unwrap().surface().unwrap().content_bounds();
    assert_eq!(bounds(first), photocraft_geom::Rect::new(28, 8, 44, 24), "applied to the layer it was on");
    assert_eq!(bounds(second), photocraft_geom::Rect::new(40, 40, 56, 60), "the new layer is untouched");
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!((t.layer, t.rect), (second.0, [40.0, 40.0, 56.0, 60.0]), "the next box is on the selected layer");
}

/// The screen rectangle the marching ants cover as [`selection_ants`] places them over the
/// active document's selection; `None` when it leaves them to the canvas.
fn ants_bounds(app: &PhotocraftApp, xf: &ViewXform) -> Option<egui::Rect> {
    let st = app.session.active().unwrap();
    let sel = st.doc.selection.as_ref().unwrap();
    let segs = crate::outline::outline(sel, sel.content_bounds());
    let ctx = egui::Context::default();
    let mut drawn = false;
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let painter = ui.ctx().layer_painter(egui::LayerId::new(egui::Order::Foreground, egui::Id::unique("ants-test")));
        let ants = selection_ants(app, &painter, xf, st.doc.id, &segs, 0.0);
        drawn = ants.is_some();
        painter.extend(ants.unwrap_or_default());
    });
    output.textures_delta.clear();
    // The ants are a mesh of straight dashes and line shapes for the slanted ones.
    let ants = output.shapes.iter().filter(|s| matches!(s.shape, egui::Shape::Mesh(_) | egui::Shape::LineSegment { .. }));
    drawn.then(|| ants.map(|s| s.shape.visual_bounding_rect()).fold(egui::Rect::NOTHING, egui::Rect::union))
}

/// The screen rectangle around document points.
fn screen_bounds(xf: &ViewXform, points: &[[f64; 2]]) -> egui::Rect {
    egui::Rect::from_points(&points.iter().map(|p| xf.to_screen(p[0] as f32, p[1] as f32)).collect::<Vec<_>>())
}

fn rects_match(a: egui::Rect, b: egui::Rect) -> bool {
    (a.min - b.min).abs().max_elem() <= 2.0 && (a.max - b.max).abs().max_elem() <= 2.0
}

/// #5297: with a selection, Free Transform's box is the selection's own rectangle (wider here
/// than the pixels it holds), and the marching ants stay on the box through a move, a scale, a
/// rotation and a distort, before anything is applied. Applying leaves them there.
#[test]
fn the_box_is_the_selection_and_the_ants_follow_it() {
    let ctx = gpu_ctx();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0)),
        zoom: 2.0,
        center: [32.0, 32.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    app.run("select.rect", json!({"x": 4, "y": 4, "width": 30, "height": 26})).unwrap();
    assert_eq!(ants_bounds(&app, &xf), None, "no box open: the canvas draws them where they are");
    begin(&mut app, &ctx).unwrap();
    let rest = corners([4.0, 4.0, 34.0, 30.0]);
    assert_eq!(app.ui.transform.as_ref().unwrap().quad, rest, "the box is the selection, not the 16 px square in it");
    assert!(rects_match(ants_bounds(&app, &xf).unwrap(), screen_bounds(&xf, &rest)), "untouched, the ants are where they were");

    let on_the_box = |app: &PhotocraftApp, what: &str| {
        let quad = app.ui.transform.as_ref().unwrap().quad;
        let (ants, boxed) = (ants_bounds(app, &xf).unwrap(), screen_bounds(&xf, &quad));
        assert!(rects_match(ants, boxed), "{what}: ants {ants:?}, box {boxed:?}");
    };
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = t.quad.map(|[x, y]| [x + 20.0, y + 10.0]);
        t.pivot = [t.pivot[0] + 20.0, t.pivot[1] + 10.0];
    }
    on_the_box(&app, "moved");
    let moved = app.ui.transform.as_ref().unwrap().quad;
    scale_about_pivot(&mut app, 0.5, 1.5);
    on_the_box(&app, "scaled");
    rotate_about_pivot(&mut app, 0.6);
    on_the_box(&app, "rotated");
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad[2] = [t.quad[2][0] + 9.0, t.quad[2][1] + 13.0];
    }
    on_the_box(&app, "distorted");

    // Applied as moved only: the outline is where the ants showed it.
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = moved;
    }
    commit(&mut app);
    assert!(app.ui.transform.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.doc.selection.as_ref().unwrap().content_bounds(), photocraft_geom::Rect::new(24, 14, 54, 40));
    assert_eq!(st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds(), photocraft_geom::Rect::new(28, 18, 44, 34));
}

/// #5297: in Warp mode the ants bend with the mesh, and past a Distort's horizon they stop
/// rather than run off to infinity.
#[test]
fn the_ants_follow_a_warp_and_stop_at_a_distorts_horizon() {
    let ctx = gpu_ctx();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 56, 56));
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0)),
        zoom: 2.0,
        center: [32.0, 32.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    app.run("select.rect", json!({"x": 12, "y": 12, "width": 40, "height": 40})).unwrap();
    begin(&mut app, &ctx).unwrap();
    let rest = corners([12.0, 12.0, 52.0, 52.0]);

    // Two corners dragged across each other put half the box past the horizon.
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = [[12.0, 12.0], [52.0, 12.0], [20.0, 40.0], [44.0, 40.0]];
    }
    let ants = ants_bounds(&app, &xf).unwrap();
    assert!(ants == egui::Rect::NOTHING || (ants.is_finite() && ants.width() < 1e5 && ants.height() < 1e5), "{ants:?}");

    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = rest;
    }
    enter_warp(&mut app, &ctx);
    let arc = Warp::preset(WarpStyle::Arc, 50.0, [12.0, 12.0, 52.0, 52.0]);
    app.ui.transform.as_mut().unwrap().warp = Some(arc.clone());
    let ants = ants_bounds(&app, &xf).unwrap();
    assert!(!rects_match(ants, screen_bounds(&xf, &rest)), "the outline left its place: {ants:?}");
    let warped: Vec<[f64; 2]> = rest.iter().map(|p| arc.map(p[0], p[1])).map(|(x, y)| [x, y]).collect();
    assert!(ants.expand(2.0).contains_rect(screen_bounds(&xf, &warped)), "the warped corners are on it: {ants:?}");
}

/// Transform Selection moves the selection outline, not a layer: selecting a layer keeps it open.
#[test]
fn selecting_another_layer_keeps_a_transform_selection_open() {
    let ctx = egui::Context::default();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let first = app.session.active().unwrap().active_layer.unwrap();
    app.run("layer.new.layer", json!({})).unwrap();
    app.run("select.all", json!({})).unwrap();
    begin_selection(&mut app, &ctx).unwrap();
    app.run("layer.select", json!({"layer": first.0})).unwrap();
    end_if_left(&mut app);
    assert!(app.ui.transform.as_ref().is_some_and(|t| t.selection));
}

/// A control-channel stroke that picks another tool applies the box first, then paints.
#[test]
fn a_pointer_request_with_another_tool_applies_the_transform_first() {
    let ctx = egui::Context::default();
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.ui.tool = Tool::Move;
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    if let Some(t) = app.ui.transform.as_mut() {
        t.quad = t.quad.map(|[x, y]| [x + 20.0, y]);
    }
    let steps = app.session.active().unwrap().history.past_len();
    let ev = |kind: &str, x: f64| json!({"kind": kind, "x": x, "y": 50.0});
    let events = json!([ev("down", 4.0), ev("move", 32.0), ev("move", 60.0), ev("up", 60.0)]);
    let (req, _rx) = crate::control::ControlRequest::new("ui.pointer", json!({"tool": "brush", "events": events}));
    let _ = crate::control::handle(&mut app, &ctx, &req);
    assert!(app.ui.transform.is_none());
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 2, "the transform, then the stroke");
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert!(b.x0 < 8 && b.x1 >= 44 && b.y0 == 8 && b.y1 > 50, "moved square and stroke: {b:?}");
}

/// Drags with real pointer events (snapping off so the drops land exactly).
fn press_drag(app: &mut PhotocraftApp, from: [f64; 2], to: [f64; 2], m: egui::Modifiers) {
    crate::canvas::tool_event(app, ToolEvent::Down { x: from[0], y: from[1], pressure: 1.0 }, m);
    crate::canvas::tool_event(app, ToolEvent::Up { x: to[0], y: to[1] }, m);
}

#[test]
fn distort_mode_moves_one_corner_and_ignores_rotate_and_edges() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.ui.extras.snap = false;
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "edit.transform.distort", json!({})).unwrap();
    let q0 = app.ui.transform.as_ref().unwrap().quad;
    assert_eq!(app.ui.transform.as_ref().unwrap().mode, TransformMode::Distort);
    // Outside (rotate) and an edge handle do nothing in Distort.
    press_drag(&mut app, [44.0, 44.0], [54.0, 10.0], egui::Modifiers::NONE);
    press_drag(&mut app, [24.0, 16.0], [34.0, 16.0], egui::Modifiers::NONE);
    assert_eq!(app.ui.transform.as_ref().unwrap().quad, q0);
    // A corner moves alone, with no keys held.
    press_drag(&mut app, q0[1], [q0[1][0] + 7.0, q0[1][1] - 5.0], egui::Modifiers::NONE);
    let q = app.ui.transform.as_ref().unwrap().quad;
    assert_eq!(q[1], [q0[1][0] + 7.0, q0[1][1] - 5.0]);
    assert_eq!([q[0], q[2], q[3]], [q0[0], q0[2], q0[3]]);
    // Free Transform from the menu switches the live box back.
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().mode, TransformMode::Free);
}

/// #3941: Edit › Transform › Rotate must rotate from a corner handle, not scale.
#[test]
fn rotate_mode_turns_the_box_from_a_corner_handle() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.ui.extras.snap = false;
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "edit.transform.rotate", json!({})).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().mode, TransformMode::Rotate);
    let q0 = app.ui.transform.as_ref().unwrap().quad;
    press_drag(&mut app, q0[1], [q0[1][0], q0[1][1] + 16.0], egui::Modifiers::NONE);
    let q = app.ui.transform.as_ref().unwrap().quad;
    let w0 = (q0[1][0] - q0[0][0]).hypot(q0[1][1] - q0[0][1]);
    let w = (q[1][0] - q[0][0]).hypot(q[1][1] - q[0][1]);
    assert!((w - w0).abs() < 1e-6, "side length stays {w0}, got {w}: {q:?}");
    assert!((q[0][1] - q[1][1]).abs() > 1.0, "the box rotated, not scaled: {q:?}");
    crate::menus::invoke(&mut app, &ctx, "edit.transform.scale", json!({})).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().mode, TransformMode::Free);
}

fn near2(a: Option<[f64; 2]>, b: [f64; 2]) -> bool {
    a.is_some_and(|a| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9)
}

/// #1767: just outside a corner is the rotate zone: the OS pointer hides for the drawn curved
/// arrow, which bends round that corner.
#[test]
fn the_preview_leaves_out_cells_past_the_horizon() {
    // #4863: the bottom corners pulled in and one dragged far out: a concave quad whose
    // source straddles the horizon. Its cells there are left out; an ordinary quad keeps all.
    let past = Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], [[0.0, 0.0], [300.0, 0.0], [60.0, 60.0], [0.0, 300.0]]).unwrap();
    let ahead = front_vertices(&past, 24);
    assert!(ahead.iter().any(|a| *a) && ahead.iter().any(|a| !*a), "some of the grid is past the horizon");
    // Three corners are on one side (drawn); the top-left one is past the horizon.
    assert!(ahead[24 * 25 + 24] && ahead[24] && ahead[24 * 25], "the three corners in front are drawn");
    assert!(!ahead[0], "the corner past the horizon is left out");
    let plain = Homography::rect_to_quad([0.0, 0.0, 1.0, 1.0], [[0.0, 0.0], [300.0, 0.0], [280.0, 260.0], [0.0, 300.0]]).unwrap();
    assert!(front_vertices(&plain, 24).iter().all(|a| *a));
}

#[test]
fn hovering_just_outside_a_corner_asks_for_the_rotate_cursor() {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let tol = handle_tolerance(&app);
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let (tl, br) = ([32.0 - tol * 1.5, 32.0 - tol * 1.5], [160.0 + tol * 1.5, 160.0 + tol * 1.5]);
    let xf = ViewXform::active(&app).unwrap();
    assert_eq!(cursor(&app, &xf, tl, egui::Modifiers::NONE), Some(CursorIcon::None), "the arrow is drawn instead");
    assert!(near2(rotate_cursor_dir(&app, tl), [h, h]), "{:?}", rotate_cursor_dir(&app, tl));
    assert!(near2(rotate_cursor_dir(&app, br), [-h, -h]), "bends round the bottom-right corner");
    assert_eq!(cursor(&app, &xf, [60.0, 60.0], egui::Modifiers::NONE), Some(CursorIcon::Move));
    assert_eq!(rotate_cursor_dir(&app, [60.0, 60.0]), None, "inside moves");
    assert_eq!(rotate_cursor_dir(&app, [32.0, 32.0]), None, "the corner handle scales");
    assert_eq!(rotate_cursor_dir(&app, [f64::NAN, 32.0]), None);
}

// Regression test for #4139: the rotate zone hides the OS pointer (`CursorIcon::None`), so the
// canvas has to draw the curved turn arrow itself where the box has no preview to draw it
// (the preview's overlay draws it otherwise); the pointer vanished over the zone.
#[test]
fn the_rotate_zone_draws_the_turn_arrow_where_the_pointer_is_hidden() {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    app.ui.tool = Tool::Move;
    let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        app
    });
    h.run_steps(4);
    let ctx = h.ctx.clone();
    crate::menus::invoke(h.state_mut(), &ctx, "edit.freeTransform", json!({})).unwrap();
    h.run_steps(3);
    let tol = handle_tolerance(h.state());
    let xf = ViewXform::active(h.state()).unwrap();
    let accent = crate::theme::Tokens::get(&h.ctx).accent_text;
    // The arrow's bright stroke: a 17-point arc in the accent colour around the pointer.
    let arrows = |h: &egui_kittest::Harness<'_, PhotocraftApp>, at: egui::Pos2| {
        h.output()
            .shapes
            .iter()
            .filter(|c| match &c.shape {
                egui::Shape::Path(p) => {
                    p.points.len() == 17
                        && p.stroke.width == 1.5
                        && p.stroke.color == egui::epaint::ColorMode::Solid(accent)
                        && p.points.iter().all(|q| q.distance(at) < 40.0)
                }
                _ => false,
            })
            .count()
    };
    let outside = xf.to_screen((32.0 - tol * 1.5) as f32, (32.0 - tol * 1.5) as f32);

    // A box with no preview (its overlay is what draws the arrow otherwise).
    h.state_mut().transform_preview = None;
    h.hover_at(outside);
    h.run_steps(3);
    assert!(h.state().ui.transform.is_some() && h.state().transform_preview.is_none());
    assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::None, "the OS pointer is hidden in the rotate zone");
    assert_eq!(arrows(&h, outside), 1, "and the curved arrow is drawn in its place");

    let inside = xf.to_screen(96.0, 96.0);
    h.hover_at(inside);
    h.run_steps(3);
    assert_eq!(arrows(&h, inside), 0, "no arrow over the box");
}

/// The resize arrows follow the box's turn and the view's rotation and flip: each handle
/// shows the arrow it resizes along on screen, not the one it had on the upright box.
#[test]
fn resize_cursors_turn_with_the_box_and_the_view() {
    use CursorIcon::{ResizeHorizontal as H, ResizeNeSw as NeSw, ResizeNwSe as NwSe, ResizeVertical as V};
    // Box turned (degrees), view rotation, flip → corner 0, corner 1, edge 0, edge 1 (corner
    // and edge i + 2 sit opposite, on the same axis).
    let cases = [
        (0.0, 0.0, false, [NwSe, NeSw, V, H]),
        (90.0, 0.0, false, [NeSw, NwSe, H, V]),
        (45.0, 0.0, false, [V, H, NeSw, NwSe]),
        (0.0, 0.0, true, [NeSw, NwSe, V, H]),
        (0.0, 90.0, false, [NeSw, NwSe, H, V]),
    ];
    for (deg, view, flip, want) in cases {
        let mut app = app_with_square(256, photocraft_geom::Rect::new(64, 96, 192, 160));
        crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
        let view_index = app.session.active_index().unwrap();
        app.ui.views[view_index].rotation = view;
        app.ui.view.flip_horizontal = flip;
        let t = app.ui.transform.as_mut().unwrap();
        let (s, c) = f64::to_radians(deg).sin_cos();
        let o = [128.0, 128.0];
        t.quad = t.quad.map(|[x, y]| [o[0] + (x - o[0]) * c - (y - o[1]) * s, o[1] + (x - o[0]) * s + (y - o[1]) * c]);
        let t = app.ui.transform.clone().unwrap();
        let xf = ViewXform::active(&app).unwrap();
        let mid = |i: usize| [(t.quad[i][0] + t.quad[(i + 1) % 4][0]) / 2.0, (t.quad[i][1] + t.quad[(i + 1) % 4][1]) / 2.0];
        let got: Vec<_> = (0..4)
            .map(|i| cursor(&app, &xf, t.quad[i], egui::Modifiers::NONE))
            .chain((0..4).map(|i| cursor(&app, &xf, mid(i), egui::Modifiers::NONE)))
            .collect();
        let want = [want[0], want[1], want[0], want[1], want[2], want[3], want[2], want[3]].map(Some);
        assert_eq!(got, want, "box {deg}°, view {view}°, flip {flip}");
    }
}

/// The corners of a wide or tall box show the diagonal arrows, as in Photoshop, not the
/// horizontal or vertical one its long diagonal is nearest to.
#[test]
fn corner_cursors_stay_diagonal_on_a_long_box() {
    use CursorIcon::{ResizeNeSw as NeSw, ResizeNwSe as NwSe};
    for rect in [photocraft_geom::Rect::new(8, 112, 248, 144), photocraft_geom::Rect::new(112, 8, 144, 248)] {
        let mut app = app_with_square(256, rect);
        crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
        let t = app.ui.transform.clone().unwrap();
        let xf = ViewXform::active(&app).unwrap();
        let got: Vec<_> = (0..4).map(|i| cursor(&app, &xf, t.quad[i], egui::Modifiers::NONE)).collect();
        assert_eq!(got, [NwSe, NeSw, NwSe, NeSw].map(Some), "{rect:?}");
    }
}

/// A warp split's arrow crosses the split line as it looks on screen: it turns with a turned
/// grid (Warp entered on a rotated box) and with a rotated view.
#[test]
fn warp_split_cursors_turn_with_the_grid_and_the_view() {
    use CursorIcon::{ResizeHorizontal as H, ResizeNeSw as NeSw, ResizeNwSe as NwSe, ResizeVertical as V};
    let rect = [0.0, 0.0, 200.0, 100.0];
    let screen = egui::Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0));
    let level = ViewXform { rect: screen, zoom: 1.0, center: [100.0, 50.0], flip: false, rotation: 0.0, aspect: 1.0 };
    // Grid turned (degrees, about its centre), view rotation → Split Vertically, Split Horizontally.
    for (grid, view, want) in [(0.0, 0.0, [H, V]), (0.0, 90.0, [V, H]), (90.0, 0.0, [V, H]), (45.0, 0.0, [NwSe, NeSw])] {
        let (s, c) = f64::to_radians(grid).sin_cos();
        let m = BezierMesh::identity(rect, 1, 1).map_points(|[x, y]| [100.0 + (x - 100.0) * c - (y - 50.0) * s, 50.0 + (x - 100.0) * s + (y - 50.0) * c]);
        let xf = ViewXform { rotation: view, ..level };
        let got = [SplitTool::Vertical, SplitTool::Horizontal].map(|tool| split_cursor(&m, tool, m.param_at([100.0, 50.0]), &xf));
        assert_eq!(got, want, "grid turned {grid}°, view {view}°");
    }
    let m = BezierMesh::identity(rect, 1, 1);
    assert_eq!(split_cursor(&m, SplitTool::Cross, (0.5, 0.5), &level), CursorIcon::Crosshair);
}

/// #1767: the readout beside the pointer during a drag (Photoshop's transformation values):
/// the angle while rotating, W/H while scaling, ΔX/ΔY while moving; nothing otherwise.
#[test]
fn the_live_readout_follows_the_drag() {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    app.ui.extras.snap = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    assert_eq!(live_readout(&app), None, "not dragging");
    let ev = |app: &mut PhotocraftApp, e| crate::canvas::tool_event(app, e, egui::Modifiers::NONE);
    // A quarter turn about the centre (96, 96): from right of the box to below it.
    ev(&mut app, ToolEvent::Down { x: 220.0, y: 96.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 96.0, y: 220.0, pressure: 1.0 });
    let Some(Readout::Angle(a)) = live_readout(&app) else { panic!("{:?}", live_readout(&app)) };
    assert!((a - 90.0).abs() < 1e-6, "{a}");
    ev(&mut app, ToolEvent::Up { x: 96.0, y: 220.0 });
    assert_eq!(live_readout(&app), None, "released");
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    app.ui.extras.snap = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    ev(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 65.0, y: 57.0, pressure: 1.0 });
    assert_eq!(live_readout(&app), Some(Readout::Offset([5.0, -3.0])));
    ev(&mut app, ToolEvent::Up { x: 65.0, y: 57.0 });
    // The right edge doubles the width (proportional by default: the height too).
    ev(&mut app, ToolEvent::Down { x: 165.0, y: 93.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 293.0, y: 93.0, pressure: 1.0 });
    let Some(Readout::Scale { percent: [w, h], px }) = live_readout(&app) else { panic!("{:?}", live_readout(&app)) };
    assert!((w - 200.0).abs() < 1e-6 && (h - 200.0).abs() < 1e-6, "{w} {h}");
    assert!((px[0] - 256.0).abs() < 1e-6 && (px[1] - 256.0).abs() < 1e-6, "{px:?}");
}

/// #9400: the W/H readout shows the box's size in the ruler unit, as Photoshop does, and
/// the scale of the original box when the rulers are in percent.
#[test]
fn the_scale_readout_follows_the_ruler_unit() {
    use photocraft_engine::prefs::Unit;
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    app.ui.extras.snap = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let ev = |app: &mut PhotocraftApp, e| crate::canvas::tool_event(app, e, egui::Modifiers::NONE);
    // The right edge from 160 to 288: the 128 px box doubles (proportionally).
    ev(&mut app, ToolEvent::Down { x: 160.0, y: 96.0, pressure: 1.0 });
    ev(&mut app, ToolEvent::Move { x: 288.0, y: 96.0, pressure: 1.0 });
    let Some(Readout::Scale { percent, px }) = live_readout(&app) else { panic!("{:?}", live_readout(&app)) };
    let shown = |app: &mut PhotocraftApp, unit: Unit| {
        app.session.prefs.edit(|p| p.units_and_rulers.rulers = unit);
        scale_values(app, percent, px)
    };
    let dpi = app.session.active().unwrap().doc.resolution_dpi;
    assert_eq!(dpi, 72.0, "the expected values below are at 72 ppi");
    assert_eq!(shown(&mut app, Unit::Pixels), ["256 px".to_string(), "256 px".to_string()]);
    assert_eq!(shown(&mut app, Unit::Inches), ["3.556 in".to_string(), "3.556 in".to_string()]);
    assert_eq!(shown(&mut app, Unit::Centimeters), ["9.03 cm".to_string(), "9.03 cm".to_string()]);
    assert_eq!(shown(&mut app, Unit::Millimeters), ["90.31 mm".to_string(), "90.31 mm".to_string()]);
    assert_eq!(shown(&mut app, Unit::Percent), ["200.0%".to_string(), "200.0%".to_string()]);
    // A flipped box reads as a negative width, as in the options bar.
    assert_eq!(scale_values(&app, [-50.0, 50.0], [-64.0, 64.0]), ["-50.0%".to_string(), "50.0%".to_string()]);
    app.session.prefs.edit(|p| p.units_and_rulers.rulers = Unit::Pixels);
    assert_eq!(scale_values(&app, [-50.0, 50.0], [-64.0, 64.0]), ["-64 px".to_string(), "64 px".to_string()]);
}

#[test]
fn a_distort_corner_stops_before_the_box_folds_over() {
    // #1323: dragging a corner across the opposite diagonal stops short of it, so the box
    // stays convex instead of folding into a bow tie.
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.ui.extras.snap = false;
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "edit.transform.distort", json!({})).unwrap();
    let q0 = app.ui.transform.as_ref().unwrap().quad;
    // Top-left far past the bottom-right corner.
    press_drag(&mut app, q0[0], [60.0, 60.0], egui::Modifiers::NONE);
    let q = app.ui.transform.as_ref().unwrap().quad;
    let sign = turns(&q0)[0].signum();
    assert!(turns(&q).iter().all(|t| t * sign > 0.0), "still convex: {q:?}");
    assert_eq!([q[1], q[2], q[3]], [q0[1], q0[2], q0[3]], "the other corners stay put");
    // It went as far as it could along the drag: up to the TR–BL diagonal (x + y = 32).
    assert!(q[0][0] > 15.0 && q[0][0] < 16.0 && (q[0][0] - q[0][1]).abs() < 1e-6, "{q:?}");
    // A drag that keeps the box convex is not held back.
    press_drag(&mut app, q[0], [2.0, 4.0], egui::Modifiers::NONE);
    assert_eq!(app.ui.transform.as_ref().unwrap().quad[0], [2.0, 4.0]);
}

#[test]
fn keep_convex_leaves_a_degenerate_start_alone() {
    let flat = [[0.0, 0.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];
    let to = [[5.0, 5.0], [1.0, 0.0], [2.0, 0.0], [3.0, 0.0]];
    assert_eq!(keep_convex(flat, to), to);
    let nan = [[f64::NAN, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
    let _ = keep_convex(nan, to);
}

#[test]
fn the_legacy_preference_swaps_corner_proportions_and_stretches_edges() {
    let none = egui::Modifiers::NONE;
    let corner = Hit::Corner(1);
    let edge = Hit::Edge(1);
    // Default: proportional (⇧ frees), so the drag keeps its keys.
    assert!(!legacy_mods(false, corner, none).shift);
    assert!(!legacy_mods(false, edge, none).shift);
    // Legacy: corners free by default, ⇧ keeps proportions; edges always stretch one axis;
    // ⌘ gestures are left alone.
    assert!(legacy_mods(true, corner, none).shift);
    assert!(!legacy_mods(true, corner, egui::Modifiers::SHIFT).shift);
    assert!(legacy_mods(true, edge, none).shift);
    assert!(legacy_mods(true, edge, egui::Modifiers::SHIFT).shift);
    assert!(!legacy_mods(true, corner, egui::Modifiers::COMMAND).shift);
    assert!(!legacy_mods(true, edge, egui::Modifiers::COMMAND).shift);
    // Through the box: a plain corner drag, then a plain right-edge drag, on a 16×16 square.
    for (legacy, proportional) in [(false, true), (true, false)] {
        for at_edge in [false, true] {
            let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
            app.ui.extras.snap = false;
            app.session.prefs.edit(|p| p.general.use_legacy_free_transform = legacy);
            begin(&mut app, &egui::Context::default()).unwrap();
            let q0 = app.ui.transform.as_ref().unwrap().quad;
            let from = if at_edge { [q0[1][0], (q0[1][1] + q0[2][1]) / 2.0] } else { q0[2] };
            press_drag(&mut app, from, [from[0] + 16.0, from[1]], none);
            let q = app.ui.transform.as_ref().unwrap().quad;
            let (w, h) = (q[2][0] - q[0][0], q[2][1] - q[0][1]);
            assert_eq!((w - h).abs() < 1e-6, proportional, "legacy {legacy}, edge {at_edge}: {w} × {h}");
        }
    }
}

/// #4174: turning off the options bar's Maintain Aspect Ratio link frees the handles too, as in
/// Photoshop: a plain corner or edge drag stretches one way, ⇧ keeps the proportions. Linked
/// (the default) is unchanged, and so is the legacy preference.
#[test]
fn the_aspect_link_decides_whether_handle_drags_keep_proportions() {
    let (none, shift) = (egui::Modifiers::NONE, egui::Modifiers::SHIFT);
    for (unlinked, legacy, mods, proportional) in
        [(false, false, none, true), (true, false, none, false), (true, false, shift, true), (true, true, none, false)]
    {
        for at_edge in [false, true] {
            let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
            app.ui.extras.snap = false;
            app.ui.transform_unlinked = unlinked;
            app.session.prefs.edit(|p| p.general.use_legacy_free_transform = legacy);
            begin(&mut app, &egui::Context::default()).unwrap();
            let q0 = app.ui.transform.as_ref().unwrap().quad;
            let from = if at_edge { [q0[1][0], (q0[1][1] + q0[2][1]) / 2.0] } else { q0[2] };
            press_drag(&mut app, from, [from[0] + 16.0, from[1]], mods);
            let q = app.ui.transform.as_ref().unwrap().quad;
            let (w, h) = (q[2][0] - q[0][0], q[2][1] - q[0][1]);
            let case = format!("unlinked {unlinked}, legacy {legacy}, shift {}, edge {at_edge}: {w} × {h}", mods.shift);
            if legacy && at_edge {
                // Legacy edges always stretch their one axis.
                assert!((w - 32.0).abs() < 1e-6 && (h - 16.0).abs() < 1e-6, "{case}");
            } else {
                assert_eq!((w - h).abs() < 1e-6, proportional, "{case}");
                assert!((w - 32.0).abs() < 1e-6, "{case}");
            }
        }
    }
    // ⌘ gestures (skew, distort) are left alone.
    assert!(!unlinked_mods(true, false, Hit::Corner(0), egui::Modifiers::COMMAND).shift);
    assert!(!unlinked_mods(true, false, Hit::Inside, none).shift);
}

#[test]
fn right_click_while_transforming_switches_the_mode() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    assert!(crate::canvas_tool_menu::open_transform(&mut app, &ctx, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    let ids: Vec<&str> = crate::canvas_tool_menu::rows(&menu).iter().flatten().map(|e| e.1).collect();
    assert!(ids.contains(&"edit.transform.distort") && ids.contains(&"edit.freeTransform"));
    crate::canvas_tool_menu::choose(&mut app, &ctx, "edit.transform.distort");
    assert_eq!(app.ui.transform.as_ref().unwrap().mode, TransformMode::Distort);
}

#[test]
fn right_click_menu_warp_row_enters_warp_mode() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    assert!(crate::canvas_tool_menu::open_transform(&mut app, &ctx, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(crate::canvas_tool_menu::rows(&menu).iter().flatten().any(|e| e.1 == "edit.transform.warp"));
    assert!(crate::canvas_tool_menu::available(&app, &menu, "edit.transform.warp"));
    // Only the current mode (Free Transform) is checked: not Scale, Rotate or Warp.
    let checked: Vec<&str> =
        crate::canvas_tool_menu::rows(&menu).iter().flatten().map(|e| e.1).filter(|id| crate::canvas_tool_menu::mode_checked(&app, &menu, id)).collect();
    assert_eq!(checked, ["edit.freeTransform"]);
    crate::canvas_tool_menu::choose(&mut app, &ctx, "edit.transform.warp");
    assert!(app.ui.transform.as_ref().unwrap().warp.is_some());
    // Already warping: the row is no longer offered.
    assert!(!crate::canvas_tool_menu::available(&app, &menu, "edit.transform.warp"));
}

#[test]
fn right_click_menu_offers_warp_for_a_path() {
    // Photoshop's Transform Path menu has Warp too (#3027).
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    app.ui.transform.as_mut().unwrap().path = Some(json!({}));
    assert!(crate::canvas_tool_menu::open_transform(&mut app, &ctx, [10.0, 10.0]));
    let menu = app.ui.canvas_tool_menu.clone().unwrap();
    assert!(crate::canvas_tool_menu::available(&app, &menu, "edit.transform.warp"));
}

fn app_with_square(size: u32, fill: photocraft_geom::Rect) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": size, "height": size})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(fill, &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app
}

/// #2439: a layer under others keeps its place in the stack while the box is open. The
/// canvas under the box shows only the layers below it; the ones over it are composited on
/// their own and drawn over the moving pixels.
#[test]
fn free_transform_keeps_the_layer_under_the_layers_over_it() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let red = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(16, 16, 40, 40), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let blue = app.session.active().unwrap().active_layer.unwrap();
    let bg = app.session.active().unwrap().doc.layers[0].id;
    app.session.execute("layer.select", json!({"layer": red.0})).unwrap();
    let ctx = gpu_ctx();
    begin(&mut app, &ctx).unwrap();
    let pv = app.transform_preview.as_ref().unwrap();
    assert!(pv.doc.layer(bg).unwrap().visible, "the layers below show under the box");
    assert!(!pv.doc.layer(blue).unwrap().visible, "the layer over it is drawn over the box instead");
    let (texture, rect) = pv.above.as_ref().and_then(|a| a.shown.as_ref()).expect("something covers the layer");
    assert_eq!(*rect, photocraft_geom::Rect::new(16, 16, 40, 40));
    assert_eq!(texture.size(), [24, 24]);
    cancel(&mut app);

    // The top layer: nothing to draw over it.
    app.session.execute("layer.select", json!({"layer": blue.0})).unwrap();
    begin(&mut app, &ctx).unwrap();
    let pv = app.transform_preview.as_ref().unwrap();
    assert!(pv.above.as_ref().is_some_and(|a| a.shown.is_none()));
    assert!(pv.doc.layer(red).unwrap().visible && pv.doc.layer(bg).unwrap().visible);
    cancel(&mut app);
}

#[test]
fn plain_layers_from_the_top_cover_the_moving_one() {
    use photocraft_color::BlendMode;
    use photocraft_doc::Layer;
    let px = |name: &str| Layer::raster(name, photocraft_color::PixelFormat::RGBA8);
    let covers = |layers: &[Layer], id: LayerId| {
        let mut out = Vec::new();
        covering(layers, id, &mut out);
        out
    };
    // In a group: the group's layers over it, then the ones over the group; never the layers
    // clipped to it.
    let (below, target, mut clip, inner_top, top) = (px("below"), px("target"), px("clip"), px("inner top"), px("top"));
    clip.clipped = true;
    let (t, c, inner, tp) = (target.id, clip.id, inner_top.id, top.id);
    let mut layers = vec![below, Layer::group("group", vec![target, clip, inner_top]), top];
    assert_eq!(covers(&layers, t), [tp, inner]);
    assert_eq!(covers(&layers, c), [tp, inner], "a clipped layer: covered by what is over its clipping group");

    // Under the box, the covering layers leave the document; over it, they alone show.
    let above = Above { ctx: egui::Context::default(), name: String::new(), max_side: 64, ids: vec![tp, inner], shown: None };
    let mut under = layers.clone();
    above.hide_in(&mut under);
    let vis = |ls: &[Layer]| [ls[0].visible, ls[1].visible, ls[2].visible];
    assert_eq!(vis(&under), [true, true, false]);
    let mut over = layers.clone();
    show_only(&mut over, &above.ids);
    assert_eq!(vis(&over), [false, true, true]);
    let inside: Vec<bool> = match &over[1].content {
        LayerContent::Group(g) => g.children.iter().map(|l| l.visible).collect(),
        _ => Vec::new(),
    };
    assert_eq!(inside, [false, false, true]);

    // A layer that blends with what is under it stops the search: it and everything under it
    // stay in the document under the box.
    layers[2].blend = BlendMode::Multiply;
    assert!(covers(&layers, t).is_empty());
    layers[2].blend = BlendMode::Normal;
    let mut multiply = px("multiply");
    multiply.blend = BlendMode::Multiply;
    layers.insert(2, multiply);
    assert_eq!(covers(&layers, t), [tp]);
    // A group that isn't Normal or Pass Through keeps what is inside it.
    layers[1].blend = BlendMode::Screen;
    layers.remove(2);
    assert_eq!(covers(&layers, t), [tp]);
    assert_eq!(covers(&layers, LayerId::fresh()), [tp], "not in the stack: down to the first layer that isn't plain");
}

/// A context that reports a real GPU's texture limit (the default reports 2048).
fn gpu_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    // Clear the font atlas delta: epaint panics on dropping unapplied deltas in debug builds.
    let mut out = ctx.run_ui(egui::RawInput { max_texture_side: Some(16384), ..Default::default() }, |_| {});
    out.textures_delta.clear();
    ctx
}

#[test]
fn preview_texture_is_full_resolution() {
    // #91: the preview used to be sampled down to 2048 px, so big layers looked blurry at 100%.
    let mut app = app_with_square(3000, photocraft_geom::Rect::new(10, 20, 2810, 2420));
    let ctx = gpu_ctx();
    begin(&mut app, &ctx).unwrap();
    let pv = app.transform_preview.as_ref().unwrap();
    assert_eq!(pv.texture.size(), [2800, 2400], "one texel per layer pixel");
    // At 100% the texture has at least as many texels as the box covers screen pixels.
    let t = app.ui.transform.as_ref().unwrap();
    assert!(pv.texture.size()[0] as f64 >= t.rect[2] - t.rect[0]);
}

#[test]
fn nested_group_preview_excludes_its_siblings() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::Layer;

    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    let target_pixels = doc.layers.pop().unwrap();
    let mut sibling = Layer::raster("sibling", PixelFormat::RGBA8);
    sibling.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(12, 12, 20, 20), &[0.0, 0.0, 1.0, 1.0]);
    let target = Layer::group("target", vec![target_pixels]);
    let target_id = target.id;
    doc.layers.push(Layer::group("parent", vec![target, sibling]));

    let (image, _) = preview_image(&st.doc, target_id, None, photocraft_geom::Rect::new(8, 8, 24, 24), 4096, false);
    assert_eq!(image.pixels[8 * image.size[0] + 8], Color32::from_rgb(255, 0, 0), "a sibling in the parent group must not be baked into the moving texture");
}

fn artboard_preview_app(background: photocraft_doc::ArtboardBackground) -> (PhotocraftApp, LayerId, LayerId) {
    use photocraft_doc::{Artboard, Layer};
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    let red = doc.layers.pop().unwrap();
    let red_id = red.id;
    let mut blue = Layer::raster("covering blue", photocraft_color::PixelFormat::RGBA8);
    blue.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(16, 16, 40, 40), &[0.0, 0.0, 1.0, 1.0]);
    let mut board = Layer::group("board", vec![red, blue]);
    let board_id = board.id;
    if let LayerContent::Group(g) = &mut board.content {
        let mut ab = Artboard::new(photocraft_geom::Rect::new(0, 0, 64, 64));
        ab.background = background;
        g.artboard = Some(ab);
    }
    doc.layers = vec![board];
    (app, board_id, red_id)
}

#[test]
fn artboard_transform_overlay_does_not_cover_moving_pixels_with_the_board_background() {
    use photocraft_doc::ArtboardBackground;
    for background in [ArtboardBackground::White, ArtboardBackground::Black] {
        let (mut app, board_id, red_id) = artboard_preview_app(background);
        let original = app.session.active().unwrap().doc.clone();
        begin(&mut app, &gpu_ctx()).unwrap();
        let pv = app.transform_preview.as_ref().unwrap();
        let (texture, rect) = pv.above.as_ref().unwrap().shown.as_ref().unwrap();
        let (image, _) = texture.level_image(0).unwrap();
        let at = |x: i32, y: i32| image.pixels[(y - rect.y0) as usize * image.size[0] + (x - rect.x0) as usize];
        assert_eq!(at(12, 12), Color32::TRANSPARENT, "the board background belongs under the moving layer");
        assert_eq!(at(20, 20), Color32::BLUE, "covering siblings must still cover the moving layer");
        let under = photocraft_compose::render(&pv.doc, photocraft_geom::Rect::new(0, 0, 64, 64));
        assert_eq!(under.px[12 * 64 + 12], background.rgba().unwrap(), "the board background remains on the base canvas");
        assert!(!pv.doc.layer(red_id).unwrap().visible);
        cancel(&mut app);
        let LayerContent::Group(g) = &app.session.active().unwrap().doc.layer(board_id).unwrap().content else { panic!("board") };
        assert_eq!(g.artboard.as_ref().unwrap().background, background);
        assert!(Arc::ptr_eq(&app.session.active().unwrap().doc, &original), "preview and cancellation do not edit the document");
    }
}

#[test]
fn artboard_solo_preview_keeps_holes_transparent_and_the_board_clip() {
    use photocraft_doc::ArtboardBackground;
    let (app, board_id, red_id) = artboard_preview_app(ArtboardBackground::White);
    let original = &app.session.active().unwrap().doc;
    let mut solo = (**original).clone();
    solo.layer_mut(red_id).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(-2, -2, 1, 1), &[1.0, 0.0, 0.0, 1.0]);
    solo_layer(&mut solo.layers, red_id);
    let b = photocraft_geom::Rect::new(-2, -2, 66, 66);
    let rendered = photocraft_compose::render(&solo, b);
    let at = |x: i32, y: i32| rendered.px[(y + 2) as usize * 68 + (x + 2) as usize];
    assert_eq!(at(12, 12), [1.0, 0.0, 0.0, 1.0]);
    assert_eq!(at(30, 30), [0.0; 4], "neither the board's white nor another child enters the moving texture");
    assert_eq!(at(-1, -1), [0.0; 4], "the artboard still clips its children");
    let LayerContent::Group(g) = &solo.layer(board_id).unwrap().content else { panic!("board") };
    assert!(g.artboard.is_some(), "keep the board boundary while omitting its background");
}

#[test]
fn artboard_moved_as_a_whole_keeps_its_background() {
    use photocraft_doc::ArtboardBackground;
    let (app, board_id, _) = artboard_preview_app(ArtboardBackground::Black);
    let mut doc = (*app.session.active().unwrap().doc).clone();
    solo_layer(&mut doc.layers, board_id);
    let rendered = photocraft_compose::render(&doc, photocraft_geom::Rect::new(0, 0, 64, 64));
    assert_eq!(rendered.px[50 * 64 + 50], [0.0, 0.0, 0.0, 1.0]);
    show_only(&mut doc.layers, &[board_id]);
    let rendered = photocraft_compose::render(&doc, photocraft_geom::Rect::new(0, 0, 64, 64));
    assert_eq!(rendered.px[50 * 64 + 50], [0.0, 0.0, 0.0, 1.0], "a whole covering artboard keeps its own background");
}

#[test]
fn nested_group_preview_detaches_every_clipped_ancestor() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::Layer;

    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    let target_pixels = doc.layers.pop().unwrap();
    let target = Layer::group("target", vec![target_pixels]);
    let target_id = target.id;
    let mut middle = Layer::group("middle clipped ancestor", vec![target]);
    middle.clipped = true;
    let mut inner_base = Layer::raster("hidden inner clipping base", PixelFormat::RGBA8);
    inner_base.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 64, 64), &[0.0, 0.0, 1.0, 1.0]);
    let mut outer = Layer::group("outer clipped ancestor", vec![inner_base, middle]);
    outer.clipped = true;
    doc.layers.push(outer);

    let (image, _) = preview_image(&st.doc, target_id, None, photocraft_geom::Rect::new(8, 8, 24, 24), 4096, false);
    assert_eq!(image.pixels[8 * image.size[0] + 8], Color32::from_rgb(255, 0, 0), "hidden clipping bases must not make a nested target disappear");
}

#[test]
fn several_preview_detaches_clipped_ancestors() {
    use photocraft_color::PixelFormat;
    use photocraft_doc::Layer;
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active_mut().unwrap();
    let doc = Arc::make_mut(&mut st.doc);
    let first = doc.layers.pop().unwrap();
    let first_id = first.id;
    let mut second = Layer::raster("second", PixelFormat::RGBA8);
    second.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(32, 8, 48, 24), &[0., 1., 0., 1.]);
    let second_id = second.id;
    let mut group = Layer::group("clipped", vec![first, second]);
    group.clipped = true;
    doc.layers.push(Layer::raster("hidden base", PixelFormat::RGBA8));
    doc.layers.push(group);
    let shown = Several::shown(doc, &[first_id, second_id], &[]);
    let (image, _) = composite_image(&shown, photocraft_geom::Rect::new(0, 0, 64, 64), 4096);
    assert_eq!(image.pixels[16 * image.size[0] + 16], Color32::from_rgb(255, 0, 0));
    assert_eq!(image.pixels[16 * image.size[0] + 40], Color32::from_rgb(0, 255, 0));
}

#[test]
fn unknown_preview_target_keeps_the_previous_document_fallback() {
    let app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active().unwrap();
    let (image, _) = preview_image(&st.doc, LayerId::fresh(), None, photocraft_geom::Rect::new(8, 8, 24, 24), 4096, false);
    assert_eq!(image.pixels[8 * image.size[0] + 8], Color32::from_rgb(255, 0, 0));
}

/// The textured meshes `draw_content` paints for the moving pixels (not the box or handles).
fn preview_meshes(app: &PhotocraftApp, xf: &ViewXform) -> Vec<egui::Mesh> {
    let ctx = gpu_ctx();
    let mut out = ctx.run_ui(egui::RawInput { max_texture_side: Some(16384), ..Default::default() }, |ui| {
        let painter = ui.ctx().layer_painter(egui::LayerId::background());
        draw_content(app, &painter, xf);
    });
    out.textures_delta.clear();
    out.shapes
        .into_iter()
        .filter_map(|s| match s.shape {
            egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => Some((*m).clone()),
            _ => None,
        })
        .collect()
}

#[test]
fn preview_pixels_past_the_canvas_are_hidden() {
    // #2027: a box scaled past the canvas drew its pixels over the pasteboard.
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 56, 56));
    begin(&mut app, &gpu_ctx()).unwrap();
    scale_about_pivot(&mut app, 2.0, 2.0);
    let q = app.ui.transform.as_ref().unwrap().quad;
    assert!(q[0][0] < -10.0 && q[2][1] > 74.0, "the box reaches past the canvas: {q:?}");
    let tol = 1e-3;
    for (rotation, flip) in [(0.0, false), (30.0, false), (-75.0, true)] {
        let xf =
            ViewXform { rect: egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(400.0, 400.0)), zoom: 2.0, center: [32.0, 32.0], flip, rotation, aspect: 1.0 };
        let meshes = preview_meshes(&app, &xf);
        let verts: Vec<[f64; 2]> = meshes.iter().flat_map(|m| m.vertices.iter().map(|v| xf.to_doc(v.pos))).collect();
        assert!(!verts.is_empty(), "the preview still draws (rotation {rotation})");
        for d in &verts {
            assert!(d[0] >= -tol && d[1] >= -tol && d[0] <= 64.0 + tol && d[1] <= 64.0 + tol, "vertex {d:?} off the canvas (rotation {rotation})");
        }
        // The clipped mesh still covers the whole canvas: the corners lie on it.
        for corner in [[0.0, 0.0], [64.0, 0.0], [64.0, 64.0], [0.0, 64.0]] {
            assert!(verts.iter().any(|d| (d[0] - corner[0]).abs() < 1e-2 && (d[1] - corner[1]).abs() < 1e-2), "corner {corner:?} (rotation {rotation})");
        }
    }
    // Inside the canvas the mesh is drawn unchanged (24×24 grid).
    cancel(&mut app);
    begin(&mut app, &gpu_ctx()).unwrap();
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(200.0, 200.0)),
        zoom: 1.0,
        center: [32.0, 32.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let meshes = preview_meshes(&app, &xf);
    assert_eq!(meshes.iter().map(|m| m.vertices.len()).sum::<usize>(), 25 * 25);
}

#[test]
fn shift_body_drag_locks_to_eight_directions_and_switches() {
    let mut s = session();
    let g = Gesture { hit: Hit::Inside, start: [40.0, 20.0], quad0: s.quad, pivot0: s.pivot };
    apply_drag(&mut s, g, [60.0, 23.0], egui::Modifiers::SHIFT);
    assert!(close(s.quad, corners([20.0, 0.0, 120.0, 50.0])), "horizontal: {:?}", s.quad);
    apply_drag(&mut s, g, [60.0, 39.0], egui::Modifiers::SHIFT);
    assert!((s.quad[0][0] - s.quad[0][1]).abs() < 1e-9 && s.quad[0][0] > 15.0, "diagonal: {:?}", s.quad[0]);
    apply_drag(&mut s, g, [41.0, 60.0], egui::Modifiers::SHIFT);
    assert!(close(s.quad, corners([0.0, 40.0, 100.0, 90.0])), "vertical: {:?}", s.quad);
}

#[test]
fn alt_click_moves_the_reference_point_and_edges_skew() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    assert!(pointer(&mut app, ToolEvent::Down { x: 40.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::ALT));
    pointer(&mut app, ToolEvent::Up { x: 40.0, y: 40.0 }, egui::Modifiers::ALT);
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!(t.pivot, [40.0, 40.0]);
    assert_eq!(t.quad, corners(t.rect), "the box didn't move");
    // ⌘ on an edge skews: both corners of the top edge move.
    let mut s = session();
    drag(&mut s, [50.0, 0.0], [70.0, 5.0], egui::Modifiers::COMMAND);
    assert_eq!((s.quad[0], s.quad[1], s.quad[2]), ([20.0, 5.0], [120.0, 5.0], [100.0, 50.0]));
    // ⌘⇧: only along the edge.
    let mut s = session();
    drag(&mut s, [50.0, 0.0], [70.0, 5.0], egui::Modifiers::COMMAND | egui::Modifiers::SHIFT);
    assert_eq!((s.quad[0], s.quad[1]), ([20.0, 0.0], [120.0, 0.0]));
    // ⌘⌥⇧ on a corner: perspective (the paired corner mirrors).
    let mut s = session();
    drag(&mut s, [100.0, 0.0], [101.0, -10.0], egui::Modifiers::COMMAND | egui::Modifiers::ALT | egui::Modifiers::SHIFT);
    assert_eq!((s.quad[1], s.quad[2]), ([100.0, -10.0], [100.0, 60.0]));
}

/// `cargo test --release -p photocraft-ui-egui transform_preview_bench -- --ignored --nocapture`
#[test]
#[ignore]
fn transform_preview_bench() {
    let (w, h) = (6000, 4000);
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": w, "height": h})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session.execute("filter.render.clouds", json!({})).unwrap();
    let ctx = gpu_ctx();
    let t0 = std::time::Instant::now();
    begin(&mut app, &ctx).unwrap();
    let begin_ms = t0.elapsed().as_secs_f64() * 1e3;
    // The same with a linked layer mask applied to the texels (#205).
    cancel(&mut app);
    app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
    app.session.execute("paint.gradient", json!({"from": [0, 0], "to": [w, 0], "colors": ["#000000", "#ffffff"], "target": "mask"})).unwrap();
    let t0 = std::time::Instant::now();
    begin(&mut app, &ctx).unwrap();
    let masked_ms = t0.elapsed().as_secs_f64() * 1e3;
    eprintln!("transform preview begin {w}x{h}: {begin_ms:.1} ms, with a linked mask {masked_ms:.1} ms");
    let size = app.transform_preview.as_ref().unwrap().texture.size();
    rotate_about_pivot(&mut app, 0.05);
    let frame = |zoom: f32| {
        let t0 = std::time::Instant::now();
        let _ = ctx.run_ui(egui::RawInput { max_texture_side: Some(16384), ..Default::default() }, |ui| {
            let painter = ui.ctx().layer_painter(egui::LayerId::background());
            let xf = ViewXform {
                rect: egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(1600.0, 1000.0)),
                zoom,
                center: [3000.0, 2000.0],
                flip: false,
                rotation: 0.0,
                aspect: 1.0,
            };
            draw_content(&app, &painter, &xf);
            draw_overlay(&app, &painter, &xf);
        });
        t0.elapsed().as_secs_f64() * 1e3
    };
    let first: Vec<f64> = [1.0, 0.25, 3.0].iter().map(|z| frame(*z)).collect();
    let n = 50;
    let steady = (0..n).map(|i| frame([1.0, 0.25, 3.0][i % 3])).sum::<f64>() / n as f64;
    eprintln!(
        "transform preview {w}x{h}: texture {size:?}, begin {begin_ms:.1} ms, first frame per level (1x, 0.25x, 3x) {first:.1?} ms, steady frame {steady:.3} ms"
    );
}

#[test]
fn transform_selection_moves_only_the_outline() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 16, "height": 16})).unwrap();
    let px0 = app.session.active().unwrap().doc.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 64, 64));
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "select.transformSelection", json!({})).unwrap();
    let t = app.ui.transform.as_ref().unwrap();
    assert!(t.selection && t.rect == [8.0, 8.0, 24.0, 24.0]);
    assert!(app.transform_preview.as_ref().unwrap().doc.selection.is_none(), "ants hidden while transforming");
    scale_about_pivot(&mut app, 2.0, 2.0);
    commit(&mut app);
    let st = app.session.active().unwrap();
    let b = st.doc.selection.as_ref().unwrap().content_bounds();
    assert!(b.width().abs_diff(32) <= 2 && b.x0.abs_diff(0) <= 1, "{b:?}");
    assert_eq!(st.doc.layers[0].surface().unwrap().read_region(photocraft_geom::Rect::new(0, 0, 64, 64)), px0);
}

#[test]
fn warp_mode_drags_anchor_with_handles_splits_and_commits() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(8, 8, 32, 32), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    let ctx = egui::Context::default();
    begin_warp(&mut app, &ctx).unwrap();
    let w = app.ui.transform.as_ref().unwrap().warp.clone().unwrap();
    assert!(w.is_identity());
    // Drag the bottom-right anchor (point 15) by (+10, +6): its two handles follow.
    warp_pointer(&mut app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    warp_pointer(&mut app, ToolEvent::Up { x: 42.0, y: 38.0 }, 2.0, egui::Modifiers::NONE);
    let m = app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().mesh.clone().unwrap();
    assert_eq!(m.points[15], [42.0, 38.0]);
    let near = |a: [f64; 2], b: [f64; 2]| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9;
    assert!(near(m.points[14], [24.0 + 10.0, 38.0]), "{:?}", m.points[14]);
    assert!(near(m.points[11], [42.0, 24.0 + 6.0]), "{:?}", m.points[11]);
    split(&mut app, "edit.transform.splitWarpCrosswise", None).unwrap();
    let split_m = app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().mesh.as_ref().unwrap();
    assert_eq!(split_m.points.last().copied(), Some([42.0, 38.0]), "the corner survives the cut");
    assert!(split_m.us.iter().any(|u| (u - 1.0 / 3.0).abs() < 1e-6) && split_m.us.iter().any(|u| (u - 0.5).abs() < 1e-6));
    commit(&mut app);
    let st = app.session.active().unwrap();
    let b = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().content_bounds();
    assert!(b.x1 >= 41 && b.y1 >= 37, "{b:?}");
    assert!(app.ui.transform.is_none());
    // Toggling back to the box drops the warp.
    begin_warp(&mut app, &ctx).unwrap();
    leave_warp(&mut app);
    assert!(app.ui.transform.as_ref().unwrap().warp.is_none());
}

#[test]
fn a_thin_layer_scaled_then_warped_commits_the_curve() {
    // #5793: a 3 px column stretched wide, switched to Warp and pulled down in the middle.
    // The commit came out straight: the warp's grid was one cell across the 3 source pixels.
    let mut app = app_with_square(128, photocraft_geom::Rect::new(40, 20, 43, 60));
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    scale_about_pivot(&mut app, 30.0, 1.0);
    enter_warp(&mut app, &ctx);
    warp_pointer(&mut app, ToolEvent::Down { x: 41.5, y: 40.0, pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    warp_pointer(&mut app, ToolEvent::Move { x: 41.5, y: 70.0, pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    warp_pointer(&mut app, ToolEvent::Up { x: 41.5, y: 70.0 }, 2.0, egui::Modifiers::NONE);
    commit(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let alpha = |x: i32, y: i32| {
        let mut p = [0.0f32; 4];
        s.read_pixel(x, y, &mut p);
        p[3]
    };
    // The middle sags below the band's old bottom edge (y 60); the ends stay put.
    assert!(alpha(41, 70) > 0.99, "middle {}", alpha(41, 70));
    assert!(alpha(15, 70) < 0.01 && alpha(15, 40) > 0.99, "end {} {}", alpha(15, 70), alpha(15, 40));
}

fn warping_square() -> PhotocraftApp {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 32, 32));
    begin_warp(&mut app, &egui::Context::default()).unwrap();
    app
}

fn warp_mesh(app: &PhotocraftApp) -> BezierMesh {
    app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().mesh.clone().unwrap()
}

#[test]
fn warp_drag_from_inside_the_grid_pulls_the_surface_under_the_pointer() {
    let mut app = warping_square();
    let before = warp_mesh(&app);
    // (20, 20) is mid-cell, far from every anchor and handle.
    warp_pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    assert!(matches!(app.transform_preview.as_ref().unwrap().warp_drag, Some(WarpDrag::Surface { .. })));
    warp_pointer(&mut app, ToolEvent::Move { x: 30.0, y: 14.0, pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    warp_pointer(&mut app, ToolEvent::Up { x: 30.0, y: 14.0 }, 2.0, egui::Modifiers::NONE);
    let after = warp_mesh(&app);
    let (s, t) = surface_at(&before, [20.0, 20.0]).unwrap();
    let moved = after.eval(s, t);
    assert!((moved[0] - 30.0).abs() < 1e-6 && (moved[1] - 14.0).abs() < 1e-6, "{moved:?}");
    assert!(app.transform_preview.as_ref().unwrap().warp_drag.is_none());
    // The picture keeps filling its frame: a pull in the middle leaves the box corners put.
    assert!([0, 3, 12, 15].iter().all(|k| after.points[*k] == before.points[*k]));
    assert!(after.points[5] != before.points[5], "the inside of the grid moved");
}

/// #7885: a surface drag (a press inside a cell, on no anchor or handle) commits exactly as
/// the preview draws it. The layer is a gradient whose colour encodes where each pixel came
/// from, so every committed pixel says which source point landed there; the preview puts
/// source point `p` at `warp.map(p)`, and the commit must agree.
#[test]
fn warp_surface_drag_commits_as_previewed() {
    const N: i32 = 96;
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": N, "height": N})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let code = |x: f64, y: f64| [x / (N - 1) as f64, y / (N - 1) as f64];
    app.session
        .edit("paint", |doc, a| {
            let s = doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap();
            for y in 0..N {
                for x in 0..N {
                    let [r, g] = code(x as f64, y as f64);
                    s.write_pixel(x, y, &[r as f32, g as f32, 0.5, 1.0]);
                }
            }
            Ok(())
        })
        .unwrap();
    begin_warp(&mut app, &egui::Context::default()).unwrap();
    let (start, end) = ([30.0, 30.0], [39.0, 25.0]);
    // A press well inside the top-left cell: a surface drag, not a point drag.
    warp_pointer(&mut app, ToolEvent::Down { x: start[0], y: start[1], pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    assert!(matches!(app.transform_preview.as_ref().unwrap().warp_drag, Some(WarpDrag::Surface { .. })));
    warp_pointer(&mut app, ToolEvent::Move { x: end[0], y: end[1], pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    warp_pointer(&mut app, ToolEvent::Up { x: end[0], y: end[1] }, 2.0, egui::Modifiers::NONE);
    // What the preview draws: the mapping of the warp the session holds.
    let previewed = app.ui.transform.as_ref().unwrap().warp.clone().unwrap();
    let landed = previewed.map(start[0], start[1]);
    assert!((landed.0 - end[0]).abs() < 1e-6 && (landed.1 - end[1]).abs() < 1e-6, "the grabbed point follows the pointer: {landed:?}");
    commit(&mut app);
    assert!(app.ui.transform.is_none(), "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let s = st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap();
    let from = |x: f64, y: f64| {
        let mut p = [0.0f32; 4];
        s.read_pixel(x.round() as i32, y.round() as i32, &mut p);
        [p[0] as f64 * (N - 1) as f64, p[1] as f64 * (N - 1) as f64]
    };
    // The grabbed point's pixel moved with the pointer: the drag did not get lost.
    let got = from(end[0], end[1]);
    assert!((got[0] - start[0]).abs() < 2.0 && (got[1] - start[1]).abs() < 2.0, "pixel at the drop point came from {got:?}, not {start:?}");
    // And everywhere else the committed pixels sit where the preview put them.
    for j in 1..12 {
        for i in 1..12 {
            let src = [i as f64 * 8.0, j as f64 * 8.0];
            let (dx, dy) = previewed.map(src[0], src[1]);
            if !(1.0..(N - 2) as f64).contains(&dx) || !(1.0..(N - 2) as f64).contains(&dy) {
                continue;
            }
            let got = from(dx, dy);
            assert!((got[0] - src[0]).abs() < 2.0 && (got[1] - src[1]).abs() < 2.0, "{src:?} previewed at {:?} but committed {got:?} there", (dx, dy));
        }
    }
}

#[test]
fn a_handle_belongs_to_the_anchor_it_hangs_off() {
    assert_eq!(handle_anchor(0, 1), (0, 0));
    assert_eq!(handle_anchor(0, 2), (0, 3));
    assert_eq!(handle_anchor(1, 0), (0, 0));
    assert_eq!(handle_anchor(5, 3), (6, 3));
    assert_eq!(handle_anchor(3, 3), (3, 3));
}

#[test]
fn reset_warp_returns_to_the_untouched_grid_and_keeps_the_session() {
    let mut app = warping_square();
    warp_pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    warp_pointer(&mut app, ToolEvent::Up { x: 30.0, y: 14.0 }, 2.0, egui::Modifiers::NONE);
    assert!(!app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().is_identity());
    reset_warp(&mut app);
    let w = app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap();
    assert!(w.is_identity(), "back to the untouched box");
    assert_eq!(w.mesh.as_ref().map(|m| (m.us.len(), m.vs.len())), Some((2, 2)));
    assert!(app.transform_preview.as_ref().unwrap().warp_drag.is_none());
    // Not warping: nothing to reset.
    leave_warp(&mut app);
    reset_warp(&mut app);
    assert!(app.ui.transform.as_ref().unwrap().warp.is_none());
}

#[test]
fn warp_press_outside_the_grid_grabs_nothing() {
    let mut app = warping_square();
    let before = warp_mesh(&app);
    warp_pointer(&mut app, ToolEvent::Down { x: 55.0, y: 55.0, pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    assert!(app.transform_preview.as_ref().unwrap().warp_drag.is_none());
    warp_pointer(&mut app, ToolEvent::Up { x: 60.0, y: 60.0 }, 2.0, egui::Modifiers::NONE);
    assert_eq!(warp_mesh(&app), before);
}

#[test]
fn ctrl_click_cuts_the_grid_both_ways() {
    let mut app = warping_square();
    let ctrl = egui::Modifiers::COMMAND;
    warp_pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, 2.0, ctrl);
    warp_pointer(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, 2.0, ctrl);
    let m = warp_mesh(&app);
    // A cut each way at the pointer (the default grid's rule-of-thirds guides become real
    // lines too, as with the Cross split tool).
    assert!(m.us.iter().any(|u| (u - 0.5).abs() < 1e-6), "vertical cut: {:?}", m.us);
    assert!(m.vs.iter().any(|v| (v - 0.5).abs() < 1e-6), "horizontal cut: {:?}", m.vs);
}

#[test]
fn replacing_the_warp_mesh_cancels_a_pending_point_drag() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 32, 32));
    let ctx = egui::Context::default();
    begin_warp(&mut app, &ctx).unwrap();
    for _ in 0..3 {
        split(&mut app, "edit.transform.splitWarpCrosswise", None).unwrap();
    }

    pointer(&mut app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, egui::Modifiers::NONE);
    assert!(matches!(app.transform_preview.as_ref().unwrap().warp_drag, Some(WarpDrag::Point { .. })), "the anchor drag is armed");

    split(&mut app, "edit.transform.removeWarpSplit", None).unwrap();
    assert!(app.transform_preview.as_ref().unwrap().warp_drag.is_none(), "the old mesh index is discarded");

    // A later control-channel move belongs to the old gesture and must be harmless.
    pointer(&mut app, ToolEvent::Move { x: 40.0, y: 40.0, pressure: 1.0 }, egui::Modifiers::NONE);
}

/// A press on a preset warp that misses its points leaves the preset alone (no invisible undo
/// step); grabbing a point turns it into a custom mesh.
#[test]
fn a_press_on_a_preset_warp_converts_it_only_when_it_grabs_the_grid() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(8, 8, 32, 32), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
    begin_warp(&mut app, &egui::Context::default()).unwrap();
    let t = app.ui.transform.as_mut().unwrap();
    let preset = Warp::preset(WarpStyle::Arc, 50.0, t.rect);
    t.warp = Some(preset.clone());
    // Off the grid altogether: a miss.
    warp_pointer(&mut app, ToolEvent::Down { x: 60.0, y: 60.0, pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    warp_pointer(&mut app, ToolEvent::Up { x: 60.0, y: 60.0 }, 2.0, egui::Modifiers::NONE);
    assert_eq!(app.ui.transform.as_ref().unwrap().warp.as_ref(), Some(&preset), "a miss changes nothing");
    // Inside the grid, away from every point: the surface is grabbed, so the preset converts.
    warp_pointer(&mut app, ToolEvent::Down { x: 20.0, y: 20.0, pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    assert_eq!(app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().style, WarpStyle::Custom);
    warp_pointer(&mut app, ToolEvent::Up { x: 20.0, y: 20.0 }, 2.0, egui::Modifiers::NONE);
    app.ui.transform.as_mut().unwrap().warp = Some(preset.clone());
    let corner = preset.to_mesh(1, 1).points[15];
    warp_pointer(&mut app, ToolEvent::Down { x: corner[0], y: corner[1], pressure: 1.0 }, 2.0, egui::Modifiers::NONE);
    assert_eq!(app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().style, WarpStyle::Custom);
}

// ---- Layer masks (#205) ----

/// `app_with_square` (red over 8..24) plus a reveal-all mask hiding x < 16.
fn masked_app(linked: bool) -> PhotocraftApp {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.session.execute("layer.layerMask.revealAll", json!({})).unwrap();
    app.session
        .edit("mask", |doc, a| {
            let m = doc.layer_mut(a.unwrap()).unwrap().mask.as_mut().unwrap();
            m.surface.fill_rect(photocraft_geom::Rect::new(0, 0, 16, 64), &[0.0]);
            m.linked = linked;
            Ok(())
        })
        .unwrap();
    app
}

fn texel_alpha(img: &egui::ColorImage, x: usize, y: usize) -> u8 {
    img.pixels[y * img.size[0] + x].a()
}

#[test]
fn transform_preview_preserves_displayed_profile_colors() {
    let bounds = photocraft_geom::Rect::new(0, 0, 8, 8);
    for (wide_gamut, monitor, warp) in [(false, "srgb", false), (true, "srgb", false), (false, "display-p3", false), (true, "srgb", true)] {
        let mut app = app_with_square(8, bounds);
        app.session
            .edit("original color sample", |doc, active| {
                if wide_gamut {
                    doc.icc_profile = Some(photocraft_cms::Builtin::AdobeRgbCompat.profile().to_bytes());
                }
                doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.55, 0.35, 0.2, 1.0]);
                Ok(())
            })
            .unwrap();
        app.session.color.settings.monitor_profile = monitor.into();
        let st = app.session.active().unwrap();
        let display = app.session.color.canvas_display(&st.doc).unwrap();
        assert_eq!(!display.is_identity(), wide_gamut || monitor != "srgb");
        let composite = photocraft_compose::render(&st.doc, bounds);
        let expected = crate::canvas::display_image(Some(&display), &composite);
        let ctx = gpu_ctx();
        if warp {
            begin_warp(&mut app, &ctx).unwrap();
        } else {
            begin(&mut app, &ctx).unwrap();
        }
        let xf = ViewXform {
            rect: egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(80.0, 80.0)),
            zoom: 1.0,
            center: [4.0, 4.0],
            flip: false,
            rotation: 0.0,
            aspect: 1.0,
        };
        let mut out = ctx.run_ui(egui::RawInput { max_texture_side: Some(16384), ..Default::default() }, |ui| {
            draw_content(&app, &ui.ctx().layer_painter(egui::LayerId::background()), &xf);
        });
        let texture = out
            .shapes
            .iter()
            .find_map(|s| match &s.shape {
                egui::Shape::Mesh(mesh) if mesh.texture_id != egui::TextureId::default() => Some(mesh.texture_id),
                _ => None,
            })
            .expect("transform draws a textured mesh");
        let upload = out.textures_delta.set.get(&texture).and_then(|updates| updates.last()).expect("drawn texture was uploaded");
        let egui::ImageData::Color(image) = &upload.image;
        let shown = image.pixels[0];
        out.textures_delta.clear();
        assert_eq!(shown, expected.pixels[0], "transform changed displayed color; wide_gamut={wide_gamut}, monitor={monitor}, warp={warp}");
    }
}

#[test]
fn preview_applies_the_linked_mask() {
    for linked in [true, false] {
        let app = masked_app(linked);
        let st = app.session.active().unwrap();
        let id = st.active_layer.unwrap();
        let b = photocraft_geom::Rect::new(8, 8, 24, 24);
        let (img, _) = preview_image(&st.doc, id, None, b, 4096, false);
        // Texel x = doc x − 8: x 10 is hidden by the mask, x 20 shows.
        assert_eq!(texel_alpha(&img, 12, 4), 255);
        assert_eq!(texel_alpha(&img, 2, 4), if linked { 0 } else { 255 }, "linked={linked}");
    }
    // With a selection the lifted pixels carry the mask too.
    let mut app = masked_app(true);
    app.session.execute("select.rect", json!({"x": 8, "y": 8, "width": 12, "height": 16})).unwrap();
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    let st = app.session.active().unwrap();
    let sel = st.doc.selection.clone().unwrap();
    let id = st.active_layer.unwrap();
    let (lifted, _) = photocraft_engine::transform_cmds::split_selected(st.doc.layer(id).unwrap().surface().unwrap(), &sel).unwrap();
    let b = photocraft_geom::Rect::new(8, 8, 20, 24);
    let (img, _) = preview_image(&st.doc, id, Some(&lifted), b, 4096, false);
    assert_eq!(texel_alpha(&img, 2, 4), 0, "masked");
    assert_eq!(texel_alpha(&img, 10, 4), 255, "revealed");
}

#[test]
fn transform_selection_tint_is_not_document_color() {
    let mut app = app_with_square(8, photocraft_geom::Rect::new(0, 0, 8, 8));
    app.session.color.settings.monitor_profile = "display-p3".into();
    app.session.execute("select.rect", json!({"x": 0, "y": 0, "width": 8, "height": 8})).unwrap();
    let ctx = gpu_ctx();
    begin_selection(&mut app, &ctx).unwrap();
    let raw = app.transform_preview.as_ref().unwrap().texture.level_image(0).unwrap().0;
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(80.0, 80.0)),
        zoom: 1.0,
        center: [4.0, 4.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        draw_content(&app, &ui.ctx().layer_painter(egui::LayerId::background()), &xf);
    });
    let id = out
        .shapes
        .iter()
        .find_map(|s| match &s.shape {
            egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => Some(m.texture_id),
            _ => None,
        })
        .unwrap();
    let egui::ImageData::Color(shown) = &out.textures_delta.set.get(&id).unwrap().last().unwrap().image;
    assert_eq!(shown.pixels, raw.pixels);
    out.textures_delta.clear();
}

#[test]
fn transform_covering_layer_uses_the_same_monitor_profile() {
    let bounds = photocraft_geom::Rect::new(0, 0, 8, 8);
    let mut app = app_with_square(8, bounds);
    let lower = app.session.active().unwrap().active_layer.unwrap();
    app.session.execute("layer.new.layer", json!({})).unwrap();
    let upper = app.session.active().unwrap().active_layer.unwrap();
    app.session
        .edit("original color samples", |doc, _| {
            doc.layer_mut(lower).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.55, 0.35, 0.2, 1.0]);
            doc.layer_mut(upper).unwrap().surface_mut().unwrap().fill_rect(bounds, &[0.2, 0.5, 0.7, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.color.settings.monitor_profile = "display-p3".into();
    let doc = app.session.active().unwrap().doc.clone();
    let display = app.session.color.canvas_display(&doc).unwrap();
    let expected = [lower, upper].map(|id| {
        let mut solo = (*doc).clone();
        for layer in &mut solo.layers {
            layer.visible = layer.id == id;
        }
        crate::canvas::display_image(Some(&display), &photocraft_compose::render(&solo, bounds)).pixels[0]
    });
    app.session.execute("layer.select", json!({"layer": lower.0})).unwrap();
    let ctx = gpu_ctx();
    begin(&mut app, &ctx).unwrap();
    let xf = ViewXform {
        rect: egui::Rect::from_min_size(egui::Pos2::ZERO, vec2(80.0, 80.0)),
        zoom: 1.0,
        center: [4.0, 4.0],
        flip: false,
        rotation: 0.0,
        aspect: 1.0,
    };
    let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
        draw_content(&app, &ui.ctx().layer_painter(egui::LayerId::background()), &xf);
    });
    let shown: Vec<_> = out
        .shapes
        .iter()
        .filter_map(|s| match &s.shape {
            egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default() => {
                let egui::ImageData::Color(image) = &out.textures_delta.set.get(&m.texture_id).unwrap().last().unwrap().image;
                Some(image.pixels[0])
            }
            _ => None,
        })
        .collect();
    out.textures_delta.clear();
    assert_eq!(shown, expected, "moving pixels and covering pixels use the same display conversion");
}

#[test]
#[ignore = "requires a GPU adapter; renders profile-managed transform evidence"]
fn transform_profile_visual_evidence() {
    let _guard = photocraft_testkit::gpu_lock();
    let destination = std::env::var("PHOTOCRAFT_TRANSFORM_EVIDENCE").unwrap_or_else(|_| "transform-profile-evidence".into());
    std::fs::create_dir_all(&destination).unwrap();
    for monitor in ["srgb", "display-p3"] {
        let mut h = egui_kittest::Harness::builder().with_size(vec2(1200.0, 800.0)).with_max_steps(64).wgpu().build_eframe(move |cc| {
            PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
            let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
            app.set_wgpu(cc.wgpu_render_state.as_ref().unwrap().clone());
            app.run("file.new", json!({"width": 400, "height": 300, "background": "transparent"})).unwrap();
            app.run("layer.new.layer", json!({})).unwrap();
            app.session
                .edit("original color swatches", |doc, active| {
                    let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
                    for (x, color) in [(60, [0.55, 0.35, 0.2, 1.0]), (160, [0.2, 0.6, 0.35, 1.0]), (260, [0.3, 0.4, 0.7, 1.0])] {
                        surface.fill_rect(photocraft_geom::Rect::new(x, 60, x + 80, 220), &color);
                    }
                    Ok(())
                })
                .unwrap();
            app.session.color.settings.monitor_profile = monitor.into();
            app.ui.extras.rulers = false;
            app.sync_views();
            app.ui.views[0].zoom = 1.0;
            app.ui.views[0].center = [200.0, 150.0];
            app.ui.views[0].fit_pending = false;
            app
        });
        h.run_steps(8);
        let app = h.state();
        let view = &app.ui.views[0];
        let xf = ViewXform { rect: app.last_canvas_rect, zoom: view.zoom, center: view.center, flip: false, rotation: 0.0, aspect: 1.0 };
        let points = [100.0, 200.0, 300.0].map(|x| xf.to_screen(x, 100.0) * h.ctx.pixels_per_point());
        let before = h.render().unwrap();
        before.save(std::path::Path::new(&destination).join(format!("{monitor}-before.png"))).unwrap();
        let ctx = h.ctx.clone();
        begin(h.state_mut(), &ctx).unwrap();
        h.run_steps(8);
        assert!(h.state().ui.transform.is_some());
        let during = h.render().unwrap();
        during.save(std::path::Path::new(&destination).join(format!("{monitor}-during.png"))).unwrap();
        commit(h.state_mut());
        h.run_steps(8);
        assert!(h.state().ui.transform.is_none());
        let after = h.render().unwrap();
        after.save(std::path::Path::new(&destination).join(format!("{monitor}-after.png"))).unwrap();
        for point in points {
            let (x, y) = (point.x as u32, point.y as u32);
            let expected = before.get_pixel(x, y).0;
            assert_ne!(expected[0], expected[1], "sample must hit a color swatch, not the checkerboard");
            for actual in [during.get_pixel(x, y).0, after.get_pixel(x, y).0] {
                assert!(actual.iter().zip(expected).all(|(a, b)| a.abs_diff(b) <= 2), "{monitor}: before={expected:?}, shown={actual:?}");
            }
        }
    }
}

#[test]
fn preview_feathers_the_linked_mask() {
    // A feathered pixel mask without a vector mask: the compositor softens the edge at x 16,
    // so the moving texels must be soft there too, not hard until the commit (#4560).
    let mut app = masked_app(true);
    app.session
        .edit("feather", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().mask.as_mut().unwrap().feather = 4.0;
            Ok(())
        })
        .unwrap();
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    let b = photocraft_geom::Rect::new(8, 8, 24, 24);
    let (img, _) = preview_image(&st.doc, id, None, b, 4096, false);
    // Texel x 7 = doc x 15, just inside the hidden half; texel x 8 = doc x 16, just revealed.
    assert!(texel_alpha(&img, 7, 4) > 0, "the feather reaches into the hidden side");
    assert!(texel_alpha(&img, 8, 4) < 255, "the feather reaches into the revealed side");
    // The same values the compositor masks the committed layer with.
    let m = photocraft_compose::masks::combined_mask(st.doc.layer(id).unwrap(), b).unwrap();
    for x in [6usize, 7, 8, 9, 15] {
        let want = (m.sample_channel(8 + x as i32, 12, 0) * 255.0).round() as i32;
        assert!((i32::from(texel_alpha(&img, x, 4)) - want).abs() <= 1, "texel {x}");
    }
}

/// #5239: a smart object clipped to the layer under it previewed as nothing while the box
/// was open (its flattened render hid the clipping base too).
#[test]
fn preview_of_a_clipped_smart_object_is_not_empty() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    app.session.execute("layer.new.layer", json!({})).unwrap();
    app.session
        .edit("paint", |doc, a| {
            doc.layer_mut(a.unwrap()).unwrap().surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(0, 0, 64, 64), &[0.0, 0.0, 1.0, 1.0]);
            Ok(())
        })
        .unwrap();
    app.session.execute("layer.smartObjects.convertToSmartObject", json!({})).unwrap();
    app.session.execute("layer.createClippingMask", json!({})).unwrap();
    let st = app.session.active().unwrap();
    let id = st.active_layer.unwrap();
    assert!(st.doc.layer(id).unwrap().clipped);
    let (img, _) = preview_image(&st.doc, id, None, photocraft_geom::Rect::new(0, 0, 64, 64), 4096, false);
    assert_eq!(texel_alpha(&img, 32, 32), 255, "the moving object shows");
}

#[test]
fn targeted_unlinked_mask_transforms_alone_through_the_box() {
    let mut app = masked_app(false);
    app.ui.mask_target = true;
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    let t = app.ui.transform.as_ref().unwrap();
    assert_eq!(t.target, Some(json!("mask")));
    assert_eq!(t.rect, [0.0, 0.0, 16.0, 64.0], "the box frames the mask's content");
    // The preview document shows the mask vacated (revealed).
    let pd = &app.transform_preview.as_ref().unwrap().doc;
    assert!(pd.layer(app.session.active().unwrap().active_layer.unwrap()).unwrap().mask.as_ref().unwrap().surface.sample_channel(4, 4, 0) > 0.99);
    // Move the box 20 px right and commit.
    let t = app.ui.transform.as_mut().unwrap();
    t.quad = t.quad.map(|[x, y]| [x + 20.0, y]);
    commit(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let l = st.doc.layer(st.active_layer.unwrap()).unwrap();
    let m = &l.mask.as_ref().unwrap().surface;
    assert!(m.sample_channel(4, 30, 0) > 0.99 && m.sample_channel(30, 30, 0) < 0.01, "mask moved 20 px right");
    assert_eq!(l.surface().unwrap().content_bounds(), photocraft_geom::Rect::new(8, 8, 24, 24), "pixels untouched");
    // Warp mode doesn't apply to a lone mask.
    app.ui.mask_target = true;
    begin(&mut app, &ctx).unwrap();
    enter_warp(&mut app, &egui::Context::default());
    assert!(app.ui.transform.as_ref().unwrap().warp.is_none());
}

#[test]
fn mask_view_keeps_move_and_transform_available_behind_a_colour_set() {
    for view in ["gray", "overlay"] {
        let mut app = masked_app(false);
        app.run("channel.target", json!({"colors":[0,2]})).unwrap();
        app.run("view.layerMask", json!({"mode":view})).unwrap();
        assert!(crate::menus::is_enabled(&app, "edit.freeTransform"));
        let st = app.session.active().unwrap();
        let layer = st.active_layer.unwrap();
        let original = st.doc.layer(layer).unwrap().surface().unwrap().clone();
        begin(&mut app, &egui::Context::default()).unwrap();
        let transform = app.ui.transform.as_mut().unwrap();
        assert_eq!(transform.target, Some(json!("mask")));
        transform.quad = transform.quad.map(|[x, y]| [x + 20.0, y]);
        commit(&mut app);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        app.ui.tool = crate::state::Tool::Move;
        app.ui.tool_options.move_auto_select = false;
        app.ui.extras.snap = false;
        app.ui.view.show.smart_guides = false;
        let mods = egui::Modifiers::NONE;
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Down { x: 28.0, y: 30.0, pressure: 1.0 }, mods);
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Move { x: 38.0, y: 30.0, pressure: 1.0 }, mods);
        crate::canvas::tool_event(&mut app, crate::canvas::ToolEvent::Up { x: 38.0, y: 30.0 }, mods);
        assert!(!app.ui.status_error, "{}", app.ui.status);
        let actual = app.session.active().unwrap().doc.layer(layer).unwrap();
        assert_eq!(actual.surface().unwrap(), &original);
        let mask = &actual.mask.as_ref().unwrap().surface;
        assert!(mask.sample_channel(24, 30, 0) > 0.99 && mask.sample_channel(40, 30, 0) < 0.01, "only the mask moved");
    }
}

#[test]
fn linked_mask_target_transforms_the_layer() {
    let mut app = masked_app(true);
    app.ui.mask_target = true;
    let ctx = egui::Context::default();
    begin(&mut app, &ctx).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().target, None);
    assert_eq!(app.ui.transform.as_ref().unwrap().rect, [8.0, 8.0, 24.0, 24.0]);
}

/// ⌘T with Path Selection and a path transforms the path, not the layer's pixels, as one
/// undoable step; it works on the Background too.
#[test]
fn free_transform_moves_the_selected_path_not_pixels() {
    let ctx = egui::Context::default();
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 64, "height": 64})).unwrap();
    app.sync_views();
    app.run("path.set", json!({"name": "work", "path": {"subpaths": [{"closed": true, "knots": [[10, 10], [30, 10], [30, 20], [10, 20]]}]}})).unwrap();
    app.ui.tool = Tool::PathSelection;
    assert!(crate::menus::is_enabled(&app, "edit.freeTransform"), "a path is transformable on the Background");
    let pixels = app.session.active().unwrap().doc.layers[0].surface().cloned();
    let steps = app.session.active().unwrap().history.past_len();
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    let t = app.ui.transform.as_mut().unwrap();
    assert_eq!(t.rect, [10.0, 10.0, 30.0, 20.0], "the box frames the path");
    assert_eq!(t.path, Some(json!({"name": "work"})));
    // Twice as wide and 5 px down.
    t.quad = [[10.0, 15.0], [50.0, 15.0], [50.0, 25.0], [10.0, 25.0]];
    commit(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 1, "one step");
    let wp = st.doc.work_path.as_ref().unwrap();
    let anchors: Vec<[f64; 2]> = wp.subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    assert_eq!(anchors, vec![[10.0, 15.0], [50.0, 15.0], [50.0, 25.0], [10.0, 25.0]]);
    assert_eq!(st.doc.layers[0].surface().cloned(), pixels, "the pixels stay put");

    // ⌘-drag of a corner (Distort) maps the path onto the box (#3027): one step.
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();
    app.ui.transform.as_mut().unwrap().quad[2] = [70.0, 40.0];
    commit(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 2, "one step: {}", app.ui.status);
    let anchors: Vec<[f64; 2]> = st.doc.work_path.as_ref().unwrap().subpaths[0].knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect();
    let want = [[10.0, 15.0], [50.0, 15.0], [70.0, 40.0], [10.0, 25.0]];
    assert!(anchors.iter().zip(want).all(|(a, b)| (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9), "{anchors:?}");

    // Edit › Transform Path › Warp bends it: one more step.
    crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap();
    let t = app.ui.transform.as_mut().unwrap();
    assert!(t.warp.is_some(), "Warp mode on a path");
    t.warp = Some(Warp { style: WarpStyle::Arc, bend: 50.0, ..Warp::none(t.rect) });
    commit(&mut app);
    let st = app.session.active().unwrap();
    assert_eq!(st.history.past_len(), steps + 3, "one step: {}", app.ui.status);
    assert!(st.doc.work_path.as_ref().unwrap().subpaths[0].knots.len() > 4, "the warp bends the edges");

    // Without a path tool, ⌘T is the pixel transform again.
    app.ui.tool = Tool::Move;
    app.run("layer.new.layer", json!({})).unwrap();
    assert!(crate::vector_ui::free_transform_path(&app).is_none());
}

/// Edit › Transform › Warp on editable type (#3004), as in Photoshop: no rasterize prompt, no
/// mesh to drag, only the warp styles; the commit is the type's own Warp Text, one step, and
/// the layer stays type.
#[test]
fn warp_on_type_takes_a_style_and_keeps_the_type_editable() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    app.run("type.create", json!({"x": 10, "y": 60, "text": "Hello"})).unwrap();
    let ctx = egui::Context::default();
    let past = |app: &PhotocraftApp| app.session.active().unwrap().history.past_len();
    let before = past(&app);
    crate::menus::invoke(&mut app, &ctx, "edit.transform.warp", json!({})).unwrap();
    assert!(app.ui.dialogs.is_empty(), "no prompt");
    let w0 = app.ui.transform.as_ref().and_then(|t| t.warp.clone()).expect("Warp mode");
    assert_eq!(w0.style, WarpStyle::None);
    // A drag on the grid changes nothing: type takes only the styles.
    let c = [(w0.bounds[0] + w0.bounds[2]) / 2.0, (w0.bounds[1] + w0.bounds[3]) / 2.0];
    warp_pointer(&mut app, ToolEvent::Down { x: w0.bounds[2], y: w0.bounds[3], pressure: 1.0 }, 4.0, egui::Modifiers::NONE);
    warp_pointer(&mut app, ToolEvent::Up { x: w0.bounds[2] + 20.0, y: w0.bounds[3] + 15.0 }, 4.0, egui::Modifiers::NONE);
    warp_pointer(&mut app, ToolEvent::Down { x: c[0], y: c[1], pressure: 1.0 }, 4.0, egui::Modifiers::NONE);
    warp_pointer(&mut app, ToolEvent::Up { x: c[0], y: c[1] + 20.0 }, 4.0, egui::Modifiers::NONE);
    assert_eq!(app.ui.transform.as_ref().unwrap().warp.as_ref(), Some(&w0));
    assert!(split(&mut app, "edit.transform.splitWarpCrosswise", None).is_err());
    // Pick Flag in the options bar, then Enter.
    if let Some(w) = app.ui.transform.as_mut().and_then(|t| t.warp.as_mut()) {
        *w = Warp { style: WarpStyle::Flag, bend: 40.0, ..Warp::none(w.bounds) };
    }
    commit(&mut app);
    assert!(!app.ui.status_error, "{}", app.ui.status);
    let st = app.session.active().unwrap();
    let LayerContent::Text(t) = &st.doc.layer(st.active_layer.unwrap()).unwrap().content else { panic!("still type") };
    let tw = t.warp.as_ref().expect("Warp Text set");
    assert_eq!((tw.style.as_str(), tw.value, tw.horizontal), ("warpFlag", 40.0, true));
    assert_eq!(past(&app), before + 1, "one history step");
    // Warp again shows Flag; None takes it off in one more step.
    begin_warp(&mut app, &ctx).unwrap();
    assert_eq!(app.ui.transform.as_ref().unwrap().warp.as_ref().unwrap().style, WarpStyle::Flag);
    if let Some(t) = app.ui.transform.as_mut() {
        t.warp = Some(Warp::none(t.rect));
    }
    commit(&mut app);
    let st = app.session.active().unwrap();
    assert!(matches!(&st.doc.layer(st.active_layer.unwrap()).unwrap().content, LayerContent::Text(t) if t.warp.is_none()));
    assert_eq!(past(&app), before + 2);
}

#[test]
fn anchor_grid_point_on_quad_and_carry_pivot() {
    let quad = [[0.0, 0.0], [100.0, 0.0], [100.0, 50.0], [0.0, 50.0]];
    assert_eq!(point_on_quad(quad, 0), [0.0, 0.0]);
    assert_eq!(point_on_quad(quad, 1), [50.0, 0.0]);
    assert_eq!(point_on_quad(quad, 2), [100.0, 0.0]);
    assert_eq!(point_on_quad(quad, 3), [0.0, 25.0]);
    assert_eq!(point_on_quad(quad, 4), [50.0, 25.0]);
    assert_eq!(point_on_quad(quad, 5), [100.0, 25.0]);
    assert_eq!(point_on_quad(quad, 6), [0.0, 50.0]);
    assert_eq!(point_on_quad(quad, 7), [50.0, 50.0]);
    assert_eq!(point_on_quad(quad, 8), [100.0, 50.0]);

    let quad_scaled = [[0.0, 0.0], [200.0, 0.0], [200.0, 100.0], [0.0, 100.0]];
    let p_top_right = carry_pivot(quad, quad_scaled, [100.0, 0.0]);
    assert_eq!(p_top_right, [200.0, 0.0]);
    let p_center = carry_pivot(quad, quad_scaled, [50.0, 25.0]);
    assert_eq!(p_center, [100.0, 50.0]);

    for anchor in 0..9 {
        assert_eq!(matching_anchor(quad, point_on_quad(quad, anchor)), Some(anchor));
    }
    assert_eq!(matching_anchor(quad, [12.3, 45.6]), None);
    assert_eq!(matching_anchor(quad, [50.0, 30.0]), None);
}

#[test]
fn hovering_corner_with_ctrl_shows_distort_cursor() {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let tol = handle_tolerance(&app);
    let corner = [32.0, 32.0];
    let mut command = egui::Modifiers::NONE;
    command.command = true;
    let xf = ViewXform::active(&app).unwrap();

    // Without Ctrl: corner handle gives standard diagonal scale cursor.
    assert_eq!(cursor(&app, &xf, corner, egui::Modifiers::NONE), Some(CursorIcon::ResizeNwSe));

    // With Ctrl: corner handle arms free distort, hiding OS cursor so custom pointer is drawn.
    assert_eq!(cursor(&app, &xf, corner, command), Some(CursorIcon::None));

    // A Cmd-drag just outside a corner still rotates, so the curved arrow stays.
    let near_corner = [32.0 - tol * 1.5, 32.0 - tol * 1.5];
    assert_eq!(cursor(&app, &xf, near_corner, command), Some(CursorIcon::None));
    assert!(rotate_cursor_dir(&app, near_corner).is_some());
    assert_eq!(cursor(&app, &xf, near_corner, egui::Modifiers::NONE), Some(CursorIcon::None));
}

#[test]
fn hovering_edge_with_ctrl_shows_skew_cursor() {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let edge = [96.0, 32.0]; // top edge midpoint
    let mut command = egui::Modifiers::NONE;
    command.command = true;
    let xf = ViewXform::active(&app).unwrap();

    // Without Ctrl: edge handle gives axis resize cursor.
    assert_eq!(cursor(&app, &xf, edge, egui::Modifiers::NONE), Some(CursorIcon::ResizeVertical));

    // With Ctrl: edge handle arms skew, hiding OS cursor so custom skew indicator is drawn.
    assert_eq!(cursor(&app, &xf, edge, command), Some(CursorIcon::None));
}

#[test]
fn hovering_corner_on_text_layer_with_ctrl_retains_resize_cursor() {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.session.execute("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.sync_views();
    app.run("type.create", json!({"x": 10, "y": 60, "text": "Hello"})).unwrap();
    let ctx = egui::Context::default();
    crate::menus::invoke(&mut app, &ctx, "edit.freeTransform", json!({})).unwrap();

    let t = app.ui.transform.as_ref().unwrap();
    let corner = t.quad[0];
    let mut command = egui::Modifiers::NONE;
    command.command = true;
    let xf = ViewXform::active(&app).unwrap();

    // Editable type refuses distort, so holding Ctrl retains standard resize cursor on corners.
    let without_ctrl = cursor(&app, &xf, corner, egui::Modifiers::NONE);
    let with_ctrl = cursor(&app, &xf, corner, command);
    assert_ne!(with_ctrl, Some(CursorIcon::None));
    assert_eq!(with_ctrl, without_ctrl, "text layer must retain resize cursor");
}

#[test]
fn cursor_during_drag_stays_stable() {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    app.ui.extras.snap = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let mut command = egui::Modifiers::NONE;
    command.command = true;
    let xf = ViewXform::active(&app).unwrap();

    // Start dragging corner [32.0, 32.0] with Command held.
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, command);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 50.0, y: 60.0, pressure: 1.0 }, command);

    // While dragging, cursor anywhere stays CursorIcon::None (distort custom cursor).
    assert_eq!(cursor(&app, &xf, [50.0, 60.0], command), Some(CursorIcon::None));
    assert_eq!(cursor(&app, &xf, [200.0, 200.0], command), Some(CursorIcon::None));

    // Release drag
    crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 50.0, y: 60.0 }, command);

    // Once released, dragging has ended: far outside returns the commit-on-click cursor,
    // and hovering the corner without Command restores normal resize cursor.
    assert_eq!(cursor(&app, &xf, [200.0, 200.0], command), Some(CursorIcon::Alias));
    assert_ne!(cursor(&app, &xf, [50.0, 60.0], egui::Modifiers::NONE), Some(CursorIcon::None));
}

fn cmd() -> egui::Modifiers {
    egui::Modifiers { command: true, ..Default::default() }
}

/// The indicator each handle shows in `mode` with `mods`: corner 0 and edge 0 (top) of a square.
fn indicators(mode: &str, mods: egui::Modifiers) -> (Option<HandleIndicator>, Option<HandleIndicator>) {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    crate::menus::invoke(&mut app, &egui::Context::default(), mode, json!({})).unwrap();
    let t = app.ui.transform.clone().unwrap();
    (handle_indicator(&app, &t, Hit::Corner(0), mods), handle_indicator(&app, &t, Hit::Edge(0), mods))
}

#[test]
fn handle_pointers_follow_what_the_drag_does() {
    use HandleIndicator::{Distort, Skew};
    let none = egui::Modifiers::NONE;
    let persp = egui::Modifiers { command: true, alt: true, shift: true, ..Default::default() };
    assert_eq!(indicators("edit.freeTransform", none), (None, None), "plain drags scale");
    assert_eq!(indicators("edit.freeTransform", cmd()), (Some(Distort), Some(Skew)));
    assert_eq!(indicators("edit.freeTransform", persp), (Some(Distort), Some(Skew)));
    // Rotate mode turns the box from every handle, whatever the keys.
    assert_eq!(indicators("edit.transform.rotate", cmd()), (None, None));
    // A Skew-mode corner slides along one edge, with or without Cmd; its edges skew.
    assert_eq!(indicators("edit.transform.skew", cmd()), (None, Some(Skew)));
    assert_eq!(indicators("edit.transform.skew", none), (None, Some(Skew)));
    // Distort mode leaves the edges alone, Cmd or not.
    assert_eq!(indicators("edit.transform.distort", none), (Some(Distort), None));
    assert_eq!(indicators("edit.transform.distort", cmd()), (Some(Distort), None));
    // Perspective mode: corners move in pairs; Cmd on an edge skews (`apply_drag`).
    assert_eq!(indicators("edit.transform.perspective", none), (Some(Distort), None));
    assert_eq!(indicators("edit.transform.perspective", cmd()), (Some(Distort), Some(Skew)));
    assert_eq!(indicators("edit.transform.scale", cmd()), (Some(Distort), Some(Skew)));
}

#[test]
fn ctrl_in_rotate_mode_keeps_the_rotate_pointer() {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.transform.rotate", json!({})).unwrap();
    let xf = ViewXform::active(&app).unwrap();
    let corner = [32.0, 32.0];
    assert_eq!(cursor(&app, &xf, corner, cmd()), Some(CursorIcon::None));
    assert!(rotate_cursor_dir(&app, corner).is_some(), "a Rotate-mode corner drag rotates: the turn arrow stays");
    // An edge drag rotates too: Cmd draws no skew pointer there.
    let edge = [96.0, 32.0];
    assert_eq!(cursor(&app, &xf, edge, cmd()), cursor(&app, &xf, edge, egui::Modifiers::NONE));
}

#[test]
fn rotate_mode_handle_drag_keeps_the_arrow_away_from_the_corners() {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    app.ui.extras.snap = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.transform.rotate", json!({})).unwrap();
    let xf = ViewXform::active(&app).unwrap();
    let none = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 32.0, y: 32.0, pressure: 1.0 }, none);
    // Far from every corner of the turned box, past the hover zone.
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 250.0, y: 96.0, pressure: 1.0 }, none);
    let t = app.ui.transform.clone().unwrap();
    let tol = handle_tolerance(&app);
    assert!(t.quad.iter().all(|q| (250.0 - q[0]).hypot(96.0 - q[1]) > tol * 4.0), "{:?}", t.quad);
    assert!(rotate_cursor_dir(&app, [250.0, 96.0]).is_some(), "the corner drag rotates");
    assert_eq!(cursor(&app, &xf, [250.0, 96.0], none), Some(CursorIcon::None));
}

#[test]
fn scale_drag_past_a_corner_shows_no_rotate_arrow() {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    app.ui.extras.snap = false;
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let none = egui::Modifiers::NONE;
    crate::canvas::tool_event(&mut app, ToolEvent::Down { x: 160.0, y: 160.0, pressure: 1.0 }, none);
    crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 150.0, pressure: 1.0 }, none);
    let t = app.ui.transform.clone().unwrap();
    let tol = handle_tolerance(&app);
    // Just outside another corner, where a hover would show the turn arrow.
    let past = [t.quad[0][0] - tol * 1.5, t.quad[0][1] - tol * 1.5];
    assert_eq!(rotate_cursor_dir(&app, past), None, "the drag scales");
}

#[test]
fn applying_the_box_hides_no_pointer() {
    let mut app = app_with_square(256, photocraft_geom::Rect::new(32, 32, 160, 160));
    crate::menus::invoke(&mut app, &egui::Context::default(), "edit.freeTransform", json!({})).unwrap();
    let xf = ViewXform::active(&app).unwrap();
    app.transform_preview.as_mut().unwrap().committing = Some(JobId(1));
    // The overlay draws no handles while the box is applied, so no drawn pointer either.
    assert_eq!(cursor(&app, &xf, [32.0, 32.0], cmd()), Some(CursorIcon::ResizeNwSe));
    assert_eq!(cursor(&app, &xf, [96.0, 32.0], cmd()), Some(CursorIcon::ResizeVertical));
}

#[test]
fn several_layers_reuse_the_transform_interpolation_preference() {
    let mut app = app_with_square(64, photocraft_geom::Rect::new(8, 8, 24, 24));
    let st = app.session.active_mut().unwrap();
    let first = st.active_layer.unwrap();
    let mut second = photocraft_doc::Layer::raster("second", photocraft_color::PixelFormat::RGBA8);
    second.surface_mut().unwrap().fill_rect(photocraft_geom::Rect::new(32, 8, 48, 24), &[0., 1., 0., 1.]);
    let second_id = second.id;
    Arc::make_mut(&mut st.doc).layers.push(second);
    st.selected_layers = vec![first, second_id];
    app.ui.tool_options.transform_interpolation = Some(photocraft_engine::prefs::Interpolation::Nearest);
    begin(&mut app, &egui::Context::default()).unwrap();
    let transform = app.ui.transform.as_ref().unwrap();
    assert_eq!(transform.layers.len(), 2);
    assert_eq!(transform.interpolation, "nearest");
}
