//! #11773: the Count tool's cursors. An arrow with "1+" off a marker, "1" on one and "1−" with ⌥
//! on one; Shift changes nothing, Ctrl is the Move tool's cursor. "On a marker" is the click's own
//! hit test, so the cursor never promises what the click won't do.

use serde_json::json;

use super::{CountCursor, count};
use crate::PhotocraftApp;
use crate::canvas::ToolEvent;
use crate::state::Tool;

/// A 200 × 100 document with the Count tool and one marker at 50,40.
fn app() -> PhotocraftApp {
    let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
    app.run("file.new", json!({"width": 200, "height": 100})).unwrap();
    app.run("count.add", json!({"x": 50, "y": 40})).unwrap();
    app.ui.tool = Tool::Count;
    app
}

fn markers(app: &PhotocraftApp) -> Vec<[f64; 2]> {
    app.session.active().unwrap().doc.measurement.count_groups.iter().flat_map(|g| g.points.clone()).collect()
}

#[test]
fn the_cursor_kind_follows_the_marker_and_alt() {
    assert_eq!(CountCursor::pick(false, false), CountCursor::Add, "empty canvas or pasteboard");
    assert_eq!(CountCursor::pick(true, false), CountCursor::Move, "over a marker");
    assert_eq!(CountCursor::pick(true, true), CountCursor::Delete, "⌥ over a marker");
    assert_eq!(CountCursor::pick(false, true), CountCursor::Add, "⌥ off a marker: still 1+");
    assert_eq!([CountCursor::Add, CountCursor::Move, CountCursor::Delete].map(CountCursor::label), ["1+", "1", "1\u{2212}"], "the badges (a real minus sign)");
}

#[test]
fn over_a_marker_is_the_clicks_own_hit_test() {
    // Walk a click away from the marker: wherever the cursor says "1" the click grabs and moves the
    // marker, and wherever it says "1+" the click adds a new one.
    for dx in [0.0, 2.0, 4.0, 6.0, 6.9, 7.1, 8.0, 15.0] {
        let mut app = app();
        let at = [50.0 + dx, 40.0];
        let kind = CountCursor::pick(crate::analysis_ui::count_marker_at(&app, at).is_some(), false);
        crate::canvas::tool_event(&mut app, ToolEvent::Down { x: at[0], y: at[1], pressure: 1.0 }, egui::Modifiers::NONE);
        crate::canvas::tool_event(&mut app, ToolEvent::Move { x: 150.0, y: 80.0, pressure: 1.0 }, egui::Modifiers::NONE);
        crate::canvas::tool_event(&mut app, ToolEvent::Up { x: 150.0, y: 80.0 }, egui::Modifiers::NONE);
        let m = markers(&app);
        match kind {
            CountCursor::Move => assert_eq!(m, vec![[150.0, 80.0]], "{dx} px off: \"1\" and the press moved the marker"),
            CountCursor::Add => assert_eq!(m.len(), 2, "{dx} px off: \"1+\" and the click added a marker: {m:?}"),
            CountCursor::Delete => assert_ne!(kind, CountCursor::Delete, "{dx} px off: no ⌥ is held"),
        }
    }
}

#[test]
fn ctrl_makes_the_count_tool_the_move_tool() {
    // Holding Ctrl turns the Count tool into the temporary Move tool, so the canvas shows the Move
    // tool's cursor instead of these.
    assert!(crate::hold_keys::cmd_moves(Tool::Count));
}

#[test]
fn the_cursor_is_the_os_arrow_with_its_badge() {
    // The OS arrow keeps its hotspot at the tip; the badge is drawn beside it.
    let ctx = egui::Context::default();
    let mut got = Vec::new();
    let mut o = ctx.run_ui(egui::RawInput::default(), |ui| {
        for kind in [CountCursor::Add, CountCursor::Move, CountCursor::Delete] {
            got.push(count(ui.ctx(), egui::pos2(100.0, 60.0), kind));
        }
    });
    o.textures_delta.clear();
    assert_eq!(got, [egui::CursorIcon::Default; 3]);
    let texts: Vec<String> = o
        .shapes
        .into_iter()
        .filter_map(|c| match c.shape {
            egui::Shape::Text(t) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect();
    for label in ["1+", "1", "1\u{2212}"] {
        assert!(texts.iter().any(|t| t == label), "badge {label} drawn: {texts:?}");
    }
}

#[test]
fn the_badge_halo_surrounds_the_digits_on_all_eight_sides() {
    // White copies one point off in every direction, diagonals too, so the dark digits read on
    // dark and busy pixels.
    let ctx = egui::Context::default();
    let mut o = ctx.run_ui(egui::RawInput::default(), |ui| {
        count(ui.ctx(), egui::pos2(100.0, 60.0), CountCursor::Add);
    });
    o.textures_delta.clear();
    let texts: Vec<(egui::Pos2, egui::Color32)> = o
        .shapes
        .into_iter()
        .filter_map(|c| match c.shape {
            egui::Shape::Text(t) => Some((t.pos, t.fallback_color)),
            _ => None,
        })
        .collect();
    let digits = texts.iter().find(|(_, c)| *c == egui::Color32::BLACK).expect("the dark digits").0;
    for dx in [-1.0, 0.0, 1.0] {
        for dy in [-1.0, 0.0, 1.0] {
            if (dx, dy) == (0.0, 0.0) {
                continue;
            }
            let want = digits + egui::vec2(dx, dy);
            assert!(texts.iter().any(|&(p, c)| c == egui::Color32::WHITE && p == want), "a halo copy at {dx},{dy}: {texts:?}");
        }
    }
}
