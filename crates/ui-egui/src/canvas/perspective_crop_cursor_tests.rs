//! #11974: the Perspective Crop's hover cursor says what a press does. With no quad a small
//! crosshair with the tool's icon (hotspot on the crosshair's centre), our own solid black
//! arrow inside the quad, over a corner the hollow arrowhead; outside the quad (no press there
//! yet) the usual crosshair.

use egui::epaint::ColorMode;
use egui::{Color32, CursorIcon, Pos2, Shape, pos2};

use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::perspective_crop_ui::{Drag, Zone, cursor, pointer};
use crate::state::Tool;

const QUAD: [[f64; 2]; 4] = [[40.0, 30.0], [160.0, 40.0], [150.0, 120.0], [50.0, 110.0]];

/// A 200 × 150 document with the Perspective Crop tool.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", serde_json::json!({"width": 200, "height": 150})).unwrap();
    app.sync_views();
    app.ui.tool = Tool::PerspectiveCrop;
    app
}

/// The drag a press at `p` starts, with the quad left as it was.
fn press(app: &mut PhotocraftApp, p: [f64; 2]) -> Option<Drag> {
    let quad = app.ui.perspective_crop;
    assert!(pointer(app, ToolEvent::Down { x: p[0], y: p[1], pressure: 1.0 }, egui::Modifiers::NONE));
    let drag = app.pcrop.drag.take();
    app.ui.perspective_crop = quad;
    drag
}

/// The zone a drag acts on: the cursor has to show the same.
fn acts_on(drag: Option<Drag>) -> Zone {
    match drag {
        Some(Drag::New { .. }) => Zone::Idle,
        Some(Drag::Corner { index, .. }) => Zone::Corner(index),
        Some(Drag::Move { .. }) => Zone::Inside,
        None => Zone::Outside,
    }
}

#[test]
fn every_point_shows_what_a_press_there_does() {
    let mut app = app();
    let mut seen = Vec::new();
    for quad in [None, Some(QUAD)] {
        app.ui.perspective_crop = quad;
        for y in (0..=150).step_by(3) {
            for x in (0..=200).step_by(3) {
                let p = [f64::from(x), f64::from(y)];
                let shown = cursor(&app, p).expect("a cursor over the canvas");
                assert_eq!(shown, acts_on(press(&mut app, p)), "at {p:?} with {quad:?}");
                if !seen.contains(&shown) {
                    seen.push(shown);
                }
            }
        }
    }
    for z in [Zone::Idle, Zone::Inside, Zone::Outside, Zone::Corner(0), Zone::Corner(1), Zone::Corner(2), Zone::Corner(3)] {
        assert!(seen.contains(&z), "{z:?} was hovered");
    }
}

#[test]
fn the_corner_tolerance_is_the_press_tolerance() {
    let mut app = app();
    app.ui.perspective_crop = Some(QUAD);
    let tol = crate::distort_ui::tolerance(&app);
    // Just inside the tolerance, inside the quad: the corner wins, as for a press.
    let near = [QUAD[2][0] - tol * 0.6, QUAD[2][1] - tol * 0.6];
    assert_eq!(cursor(&app, near), Some(Zone::Corner(2)));
    assert_eq!(acts_on(press(&mut app, near)), Zone::Corner(2));
    // Just past it, inside the quad: the arrow, and a press moves the quad.
    let past = [QUAD[2][0] - tol * 1.2, QUAD[2][1] - tol * 1.2];
    assert_eq!(cursor(&app, past), Some(Zone::Inside));
    assert_eq!(acts_on(press(&mut app, past)), Zone::Inside);
}

#[test]
fn a_held_drag_keeps_its_cursor() {
    // Not measured: as the Crop tool, the cursor stays the drag's while the button is held.
    let mut app = app();
    app.ui.perspective_crop = Some(QUAD);
    assert!(pointer(&mut app, ToolEvent::Down { x: 100.0, y: 75.0, pressure: 1.0 }, egui::Modifiers::NONE));
    assert!(pointer(&mut app, ToolEvent::Move { x: 190.0, y: 140.0, pressure: 1.0 }, egui::Modifiers::NONE));
    assert_eq!(cursor(&app, [5.0, 5.0]), Some(Zone::Inside), "moving the quad");
    assert!(pointer(&mut app, ToolEvent::Up { x: 190.0, y: 140.0 }, egui::Modifiers::NONE));
    let corner = app.ui.perspective_crop.unwrap()[0];
    assert!(pointer(&mut app, ToolEvent::Down { x: corner[0], y: corner[1], pressure: 1.0 }, egui::Modifiers::NONE));
    assert_eq!(cursor(&app, [100.0, 75.0]), Some(Zone::Corner(0)), "dragging a corner");
}

#[test]
fn only_the_perspective_crop_tool_with_a_document() {
    let mut app = app();
    app.ui.tool = Tool::Crop;
    assert_eq!(cursor(&app, [100.0, 75.0]), None);
    app.ui.tool = Tool::PerspectiveCrop;
    assert_eq!(cursor(&app, [f64::NAN, 75.0]), None);
    app.run("file.close", serde_json::json!({})).unwrap();
    assert_eq!(cursor(&app, [100.0, 75.0]), None);
}

/// A cursor bitmap's hotspot and size.
type Bitmap = ([u16; 2], [u16; 2]);

/// What the cursor for a zone is made of.
struct Drawn {
    /// The OS cursor.
    icon: CursorIcon,
    /// The OS cursor bitmap's hotspot and size, if one is set.
    bitmap: Option<Bitmap>,
    /// Everything painted on top (the badge, the arrowhead, the arrow), flattened.
    shapes: Vec<Shape>,
}

impl Drawn {
    /// The positions of the fully opaque black vertices: a solid black fill (the arrowhead's dark
    /// outline is translucent, its fill white).
    fn black_fill(&self) -> Vec<Pos2> {
        let mut at = Vec::new();
        for shape in &self.shapes {
            if let Shape::Mesh(mesh) = shape {
                at.extend(mesh.vertices.iter().filter(|v| v.color == Color32::BLACK).map(|v| v.pos));
            }
        }
        at
    }

    /// The closed light outlines.
    fn light_outlines(&self) -> Vec<&egui::epaint::PathShape> {
        let light = |c: Color32| c.r() > 200 && c.g() > 200 && c.b() > 200 && c.a() > 128;
        self.shapes
            .iter()
            .filter_map(|s| match s {
                Shape::Path(p) if p.closed && p.stroke.width > 0.0 && matches!(p.stroke.color, ColorMode::Solid(c) if light(c)) => Some(p),
                _ => None,
            })
            .collect()
    }
}

fn flatten(shape: Shape, out: &mut Vec<Shape>) {
    match shape {
        Shape::Vec(inner) => inner.into_iter().for_each(|s| flatten(s, out)),
        s => out.push(s),
    }
}

/// The cursor drawn for `zone` at `at`.
fn drawn(zone: Zone, at: Pos2) -> Drawn {
    let ctx = egui::Context::default();
    ctx.add_plugin(crate::tool_cursor::CursorLifecycle);
    egui_extras::install_image_loaders(&ctx);
    let run = || {
        let mut out = ctx.run_ui(Default::default(), |ui| {
            let icon = crate::tool_cursor::perspective_crop(ui.painter(), zone, at);
            ui.ctx().set_cursor_icon(icon);
        });
        out.textures_delta.clear();
        out
    };
    // egui's first Area pass measures its content and paints nothing; the icon loads meanwhile.
    run();
    run();
    let out = run();
    let mut shapes = Vec::new();
    for clipped in out.shapes {
        flatten(clipped.shape, &mut shapes);
    }
    Drawn { icon: out.platform_output.cursor_icon, bitmap: out.platform_output.cursor_image.map(|i| (i.hotspot, i.size)), shapes }
}

#[test]
#[cfg(not(target_arch = "wasm32"))]
fn the_zones_draw_distinct_cursors_with_their_hotspots() {
    let at = pos2(120.0, 90.0);
    let idle = drawn(Zone::Idle, at);
    let inside = drawn(Zone::Inside, at);
    let corner = drawn(Zone::Corner(1), at);
    // No quad: the small crosshair as the OS bitmap, centred on the point, with the
    // tool's icon painted beside it. (`CursorLifecycle` leaves the OS crosshair under a bitmap,
    // for a backend that can't show one.)
    assert_eq!(idle.icon, CursorIcon::Crosshair);
    // The bitmap is padded for its outline: the hotspot is its centre, where the arms cross.
    let (hot, size) = idle.bitmap.expect("the crosshair bitmap");
    assert_eq!(hot[0], hot[1]);
    assert_eq!(hot[0] * 2 + 1, size[0], "the crosshair's centre is the hotspot: {hot:?} in {size:?}");
    assert!(!idle.shapes.is_empty(), "the tool badge");
    // Inside: our own solid black arrow, not the OS one (which is white on Windows and follows the
    // theme on Linux), with the OS cursor hidden under it.
    assert_eq!(inside.icon, CursorIcon::None, "the OS arrow is not used");
    assert_eq!(inside.bitmap, None, "no crosshair bitmap");
    let fill = inside.black_fill();
    assert!(!fill.is_empty(), "the arrow is filled solid black");
    let (min, max) = fill.iter().fold((pos2(f32::MAX, f32::MAX), pos2(f32::MIN, f32::MIN)), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    assert_eq!(min, at, "the arrow's tip is the hotspot, on the point");
    assert!(max.x - min.x >= 9.0 && max.y - min.y >= 15.0, "an arrow of the usual size, not a dot: {min:?} to {max:?}");
    let outlines = inside.light_outlines();
    assert!(!outlines.is_empty(), "a thin light outline so it reads on a dark image");
    assert!(outlines.iter().all(|o| o.stroke.width <= 2.0), "thin");
    assert!(outlines.iter().any(|o| o.points.contains(&at)), "the outline passes through the tip too");
    // Corner: the hollow arrowhead, painted with its tip on the point.
    assert_eq!(corner.icon, CursorIcon::None);
    assert_eq!(corner.bitmap, None, "no crosshair bitmap");
    assert!(!corner.shapes.is_empty(), "the arrowhead");
    assert!(corner.black_fill().is_empty(), "the corner arrowhead is hollow white, not the solid arrow");
    assert!(idle.black_fill().is_empty());
}
