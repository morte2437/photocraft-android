//! The Ruler tool's own pointers (#12037). Away from the ruler, over its line or one of its end
//! points without ⌥, and with ⇧: a small arrow with a ruler below right of its tip. Over an end
//! point with ⌥ — where an ⌥-drag starts the protractor arm: the same arrow with a protractor's
//! angle mark, from the very hit test a press uses. ⌘ turns the Ruler into the temporary Move tool
//! (`hold_keys::cmd_moves`), so it keeps the Move pointer on and off the ruler. The hotspot is the
//! arrow's tip; the artwork is our own geometry, an OS cursor bitmap that is painted where a
//! backend takes no bitmap.

use egui::{CursorIcon, CustomCursorImage, Modifiers, Shape as EShape, Vec2, vec2};
use serde_json::json;

use crate::PhotocraftApp;
use crate::analysis_ui::Drag;
use crate::canvas::ToolEvent;
use crate::state::Tool;

use super::{CursorLifecycle, Shape, native_scale, rasterize, ruler_arrow, ruler_rings};

/// The document's ruler, and a point well clear of it.
const START: [f64; 2] = [80.0, 120.0];
const END: [f64; 2] = [260.0, 220.0];
const AWAY: [f64; 2] = [340.0, 60.0];
/// The point the pointer sits on in a bare frame.
const ON: egui::Pos2 = egui::Pos2::new(100.0, 80.0);

/// A document with the Ruler tool and one ruler drawn in it, at `zoom`.
fn ruler_app(zoom: f32) -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 400, "height": 400})).unwrap();
    app.run("image.analysis.rulerTool", json!({"start": START, "end": END, "protractor": null})).unwrap();
    app.ui.views[0].zoom = zoom;
    app.ui.tool = Tool::Ruler;
    app.sync_views();
    app
}

/// One cursor pass with no renderer: the pointer the app asks for, and what it drew.
fn cursor_frame(app: &PhotocraftApp, doc: [f64; 2], mods: Modifiers) -> egui::FullOutput {
    let ctx = egui::Context::default();
    ctx.add_plugin(CursorLifecycle);
    let mut output = ctx.run_ui(Default::default(), |ui| {
        let icon = super::ruler(app, ui.painter(), ON, doc, mods);
        ui.ctx().set_cursor_icon(icon);
    });
    // These tests read the cursor output, never a renderer.
    output.textures_delta.clear();
    output
}

fn alpha(image: &CustomCursorImage, x: u16, y: u16) -> u8 {
    image.rgba[(usize::from(y) * usize::from(image.size[0]) + usize::from(x)) * 4 + 3]
}

/// The Ruler tool's pointer as its bitmap: `protractor` is the angle mark over an end point.
fn bitmap(protractor: bool, scale: f32) -> CustomCursorImage {
    let shape = Shape::Outline { rings: ruler_rings(protractor).into(), centre: false };
    rasterize(&shape, scale).expect("the ruler cursor rasterizes")
}

/// The inked pixels of a bitmap as points round its hotspot, so a claim about the artwork does not
/// depend on the display scale.
fn ink(image: &CustomCursorImage, scale: f32) -> Vec<Vec2> {
    let side = usize::from(image.size[0]);
    let hot = f32::from(image.hotspot[0]);
    image
        .rgba
        .as_chunks::<4>()
        .0
        .iter()
        .enumerate()
        .filter(|(_, p)| p[3] > 0)
        .map(|(i, _)| vec2((i % side) as f32 - hot, (i / side) as f32 - hot) / scale)
        .collect()
}

/// The ink within 8 pt of the hotspot: the arrowhead alone, wherever the glyph is.
fn arrow_ink(image: &CustomCursorImage) -> Vec<(i32, i32)> {
    ink(image, 1.0).into_iter().filter(|p| p.length() <= 8.0).map(|p| (p.x.round() as i32, p.y.round() as i32)).collect()
}

/// Assert two bitmaps are the same picture, reporting the difference in words rather than in
/// thousands of bytes.
fn assert_same_bitmap(got: &CustomCursorImage, want: &CustomCursorImage, what: &str) {
    assert_eq!(got.size, want.size, "{what}: the bitmap size");
    assert_eq!(got.hotspot, want.hotspot, "{what}: the hotspot");
    let differs = got.rgba.iter().zip(want.rgba.iter()).filter(|(a, b)| a != b).count();
    assert_eq!(differs, 0, "{what}: {differs} of {} bytes differ from the pointer module's own picture", got.rgba.len());
}

#[test]
fn ruler_bitmaps_put_the_arrow_tip_on_the_hotspot_at_each_scale() {
    for scale in [1.0, 1.25, 2.0] {
        for protractor in [false, true] {
            let image = bitmap(protractor, scale);
            let [x, y] = image.hotspot;
            assert_eq!(image.size, [2 * x + 1, 2 * y + 1]);
            assert!(alpha(&image, x, y) > 100, "{protractor}: the arrow's tip is on the hotspot");
            let painted = ink(&image, scale);
            let reach = painted.iter().fold(0.0f32, |m, p| m.max(p.length()));
            assert!(reach <= 30.0, "{protractor} at {scale}: nothing reaches further than 30 pt from the tip, {reach} does");
            // Above and left of the tip there is nothing to show: that is where the pointer is.
            let stray: Vec<_> = painted.iter().filter(|p| p.x + p.y <= -3.0).copied().collect();
            assert!(stray.is_empty(), "{protractor} at {scale}: the glyph stays below right of the tip, {stray:?}");
            assert!(painted.iter().any(|p| p.x > 12.0 && p.y > 12.0), "{protractor}: a glyph below right of the arrow");
            let pixels = image.rgba.as_chunks::<4>().0;
            assert!(pixels.iter().any(|p| p[3] > 230 && p[0] > 230), "a light cursor");
            assert!(pixels.iter().any(|p| p[3] > 100 && p[0] < 25), "with a dark outline, readable on white");
        }
    }
}

#[test]
fn the_protractor_cursor_is_the_same_arrow_with_a_different_glyph() {
    let ruler = bitmap(false, 1.0);
    let protractor = bitmap(true, 1.0);
    assert_ne!(ruler.rgba, protractor.rgba, "the angle mark is not the ruler");
    let alone = rasterize(&Shape::Outline { rings: vec![ruler_arrow()].into(), centre: false }, 1.0).expect("the arrowhead alone rasterizes");
    let want = arrow_ink(&alone);
    assert!(!want.is_empty(), "the arrowhead is inked by its tip");
    for (image, what) in [(&ruler, "the ruler"), (&protractor, "the protractor")] {
        assert_eq!(arrow_ink(image), want, "{what}: the same arrowhead round the tip");
    }
}

/// Where a backend takes no cursor bitmap the shape is painted at the pointer, its tip on it.
#[test]
fn the_ruler_pointer_is_painted_at_the_pointer_without_a_bitmap() {
    let ctx = egui::Context::default();
    for protractor in [false, true] {
        let mut output = ctx.run_ui(Default::default(), |ui| {
            super::paint(ui.painter(), ON, &Shape::Outline { rings: ruler_rings(protractor).into(), centre: false });
        });
        output.textures_delta.clear();
        let paths: Vec<_> = output.shapes.iter().filter_map(|s| if let EShape::Path(p) = &s.shape { Some(p) } else { None }).collect();
        assert!(paths.len() >= 4, "{protractor}: the arrow and its glyph are drawn, got {}", paths.len());
        assert!(paths.iter().any(|p| p.closed && p.points.first().is_some_and(|q| *q == ON)), "{protractor}: an arrowhead with its tip on the pointer");
        assert!(paths.iter().any(|p| p.points.iter().all(|q| (*q - ON).length() > 11.0)), "{protractor}: and a glyph clear of it");
    }
}

/// The state table, as the canvas match asks for it: the arrow with its ruler everywhere but over
/// an end point with ⌥, where the angle mark takes the ruler's place. The hotspot of both is the
/// arrow's tip, which is what the pointer sits on.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn every_ruler_state_gets_its_pointer() {
    let app = ruler_app(1.0);
    let scale = native_scale(&egui::Context::default());
    let mid = [(START[0] + END[0]) / 2.0, (START[1] + END[1]) / 2.0];
    for (doc, mods, protractor, what) in [
        (AWAY, Modifiers::NONE, false, "away from the ruler"),
        (mid, Modifiers::NONE, false, "over the line"),
        (END, Modifiers::NONE, false, "over an end"),
        (START, Modifiers::NONE, false, "over the other end"),
        (AWAY, Modifiers::SHIFT, false, "with ⇧"),
        (mid, Modifiers::SHIFT, false, "over the line with ⇧"),
        (mid, Modifiers::ALT, false, "over the line with ⌥"),
        (AWAY, Modifiers::ALT, false, "away from the ruler with ⌥"),
        (END, Modifiers::ALT, true, "over an end with ⌥"),
        (START, Modifiers::ALT, true, "over the other end with ⌥"),
    ] {
        let out = cursor_frame(&app, doc, mods);
        let image = out.platform_output.cursor_image.unwrap_or_else(|| panic!("{what}: the Ruler's own pointer is a bitmap cursor"));
        assert_same_bitmap(&image, &bitmap(protractor, scale), what);
        assert_eq!(out.platform_output.cursor_icon, CursorIcon::Crosshair, "{what}: a bitmap marks itself None, with a visible fallback");
    }
}

/// ⌘ makes the Ruler the temporary Move tool, so the canvas' cursor match runs with `Tool::Move`
/// and shows the Move pointer — held on the ruler and off it (#12037; #9933 draws the four-way
/// cross Photoshop shows for it).
#[test]
fn ctrl_handed_to_the_ruler_is_the_move_tool() {
    let app = ruler_app(1.0);
    assert!(crate::hold_keys::cmd_moves(Tool::Ruler), "⌘ is the Move tool with the Ruler");
    let ctx = egui::Context::default();
    let mut input = egui::RawInput::default();
    input.events.push(egui::Event::ModifiersChanged(Modifiers::COMMAND));
    let mut out = ctx.run_ui(input, |_| {});
    out.textures_delta.clear();
    assert_eq!(crate::hold_keys::held_tool(&app, &ctx), Some(crate::hold_keys::Temporary::Move), "⌘ held over the canvas");
    assert_eq!(crate::hold_keys::Temporary::Move.tool(), Tool::Move, "and the cursor match runs with the Move tool's own arm");
}

/// The ⌥ pointer shows exactly where an ⌥-drag starts the protractor arm: the cursor's hit test and
/// a press agree at every zoom, because they are the same `dist` with the same grab radius, in
/// screen points.
#[test]
fn the_protractor_pointer_marks_exactly_where_an_alt_press_takes_the_end() {
    for zoom in [0.5f32, 1.0, 2.0] {
        // Past the end along its own line, in screen points: the grab radius is 7 of them.
        for (reach, takes_the_end) in [(3.0, true), (12.0, false)] {
            let app = ruler_app(zoom);
            let mut pressed = ruler_app(zoom);
            let p = [END[0] + reach / f64::from(app.point_zoom().max(0.01)), END[1]];
            crate::analysis_ui::pointer(&mut pressed, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, Modifiers::ALT);
            assert_eq!(crate::analysis_ui::ruler_end_at(&app, p), takes_the_end, "{reach} pt past the end at {zoom}: the cursor");
            assert_eq!(pressed.ui.analysis.drag == Some(Drag::RulerHandle(2)), takes_the_end, "{reach} pt past the end at {zoom}: an ⌥-press");
        }
    }
}

/// Without a ruler there is no end point to mark, and an ⌥-press starts a new ruler.
#[test]
fn an_empty_document_has_no_end_to_mark() {
    let mut app = ruler_app(1.0);
    app.run("image.analysis.rulerTool", json!({"clear": true})).unwrap();
    assert!(app.session.active().is_some_and(|d| d.doc.measurement.ruler.is_none()));
    assert!(!crate::analysis_ui::ruler_end_at(&app, START), "no ruler, no end point");
    crate::analysis_ui::pointer(&mut app, ToolEvent::Down { x: START[0], y: START[1], pressure: 1.0 }, Modifiers::ALT);
    assert!(matches!(app.ui.analysis.drag, Some(Drag::RulerNew(_))), "and a press there starts a new ruler");
}

/// Over an end point without ⌥ the press grabs that end, so the plain ruler pointer stays.
#[test]
fn over_an_end_without_alt_the_press_grabs_the_end() {
    let mut app = ruler_app(1.0);
    assert!(crate::analysis_ui::ruler_end_at(&app, END), "the end is a grab target");
    crate::analysis_ui::pointer(&mut app, ToolEvent::Down { x: END[0], y: END[1], pressure: 1.0 }, Modifiers::NONE);
    assert_eq!(app.ui.analysis.drag, Some(Drag::RulerHandle(1)), "and a press there drags it");
}

/// The live canvas: with the Ruler tool it hands the OS the arrow-and-ruler bitmap, the angle mark
/// over an end with ⌥, and the Move tool's pointer under ⌘.
#[test]
#[cfg(not(target_arch = "wasm32"))]
fn the_canvas_gives_the_ruler_tool_its_own_pointer() {
    use egui::{Event, vec2};
    use egui_kittest::Harness;

    let mut h = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            // Nothing to lay out before the font stack is in: an early pass draws no label at all.
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        ruler_app(1.0),
    );
    PhotocraftApp::setup_context(&h.ctx, crate::theme::ThemeKind::ALL[0]);
    h.run_steps(4);
    if !h.ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
        eprintln!("skipped: no font stack, so the canvas cannot be laid out here");
        return;
    }
    let scale = native_scale(&h.ctx);
    let at = |h: &Harness<'static, PhotocraftApp>, doc: [f64; 2]| {
        crate::canvas::ViewXform::active(h.state()).expect("a document with a view").to_screen(doc[0] as f32, doc[1] as f32)
    };
    let hover = |h: &mut Harness<'static, PhotocraftApp>, doc: [f64; 2], mods: Modifiers| {
        h.event(Event::ModifiersChanged(mods));
        h.event(Event::PointerMoved(at(h, doc)));
        h.run_steps(2);
        h.output().platform_output.cursor_image.clone()
    };
    for (doc, mods, protractor, what) in [
        (AWAY, Modifiers::NONE, false, "away from the ruler"),
        (END, Modifiers::ALT, true, "over an end with ⌥"),
        (AWAY, Modifiers::NONE, false, "back away from it"),
    ] {
        let image = hover(&mut h, doc, mods).unwrap_or_else(|| panic!("{what}: the canvas shows the Ruler's own pointer"));
        assert_same_bitmap(&image, &bitmap(protractor, scale), what);
    }
    for doc in [AWAY, END] {
        h.event(Event::ModifiersChanged(Modifiers::COMMAND));
        h.event(Event::PointerMoved(at(&h, doc)));
        h.run_steps(2);
        assert!(h.output().platform_output.cursor_image.is_none(), "{doc:?}: ⌘ shows no Ruler bitmap");
        assert_eq!(h.output().platform_output.cursor_icon, CursorIcon::Default, "{doc:?}: the Move tool's pointer");
    }
    // Preferences › Cursors › Other Cursors = Precise: the Ruler keeps the plain crosshair, as
    // the Move and Type tools do; its own arrow is a nice-to-have, not a precise pointer (#12037 review).
    let mut app = ruler_app(1.0);
    app.run("prefs.set", json!({"values": {"cursors.other": "precise"}})).unwrap();
    let mut hp = Harness::builder().with_size(vec2(900.0, 700.0)).build_ui_state(
        |ui, app: &mut PhotocraftApp| {
            let ctx = ui.ctx().clone();
            if !ctx.fonts(|f| f.families().contains(&egui::FontFamily::Name("medium".into()))) {
                return;
            }
            egui::CentralPanel::default().show(ui, |ui| crate::canvas::document_area(app, ui));
        },
        app,
    );
    PhotocraftApp::setup_context(&hp.ctx, crate::theme::ThemeKind::ALL[0]);
    hp.run_steps(4);
    for doc in [AWAY, END] {
        hp.event(Event::ModifiersChanged(Modifiers::NONE));
        hp.event(Event::PointerMoved(crate::canvas::ViewXform::active(hp.state()).expect("a view").to_screen(doc[0] as f32, doc[1] as f32)));
        hp.run_steps(2);
        // The icon is the point: Precise asks for the plain crosshair, exactly as the Move and
        // Type arms do; the small bitmap that rides with it on native is the cursor pipeline's
        // visible crosshair fallback, not the Ruler's arrow-and-ruler art of the phases above.
        assert_eq!(hp.output().platform_output.cursor_icon, CursorIcon::Crosshair, "{doc:?}: Precise keeps the crosshair");
    }
}
