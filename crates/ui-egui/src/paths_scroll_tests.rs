//! Scrolling the Paths panel moves only its list (#6238).

use egui::vec2;
use egui_kittest::{Harness, kittest::Queryable};
use serde_json::json;

use crate::PhotocraftApp;

fn harness(paths: usize, size: egui::Vec2, ppp: f32) -> Harness<'static, PhotocraftApp> {
    let mut h = Harness::builder().with_size(size).with_pixels_per_point(ppp).with_max_steps(64).build_eframe(move |cc| {
        PhotocraftApp::setup_context(&cc.egui_ctx, Default::default());
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), crate::Services::default());
        app.run("file.new", json!({"width": 400, "height": 300})).unwrap();
        for i in 0..paths {
            let x = (i % 8) as i32 * 40;
            app.run("select.rect", json!({"x": x, "y": 20, "width": 30, "height": 30})).unwrap();
            app.run("select.toWorkPath", json!({"tolerance": 2})).unwrap();
            app.run("path.rename", json!({"name": "work", "to": format!("Path {}", i + 1)})).unwrap();
        }
        // Layers | Channels | Paths: the Paths tab.
        app.ui.dock_tabs.layers = 2;
        app
    });
    h.run_steps(8);
    h
}

/// Scrolling the path list moves the rows and nothing else: the footer's buttons stay where they
/// are. The Paths panel lays out its own list and footer, but the dock also wrapped it in a
/// scroll area; at 125 % scaling and some window heights that outer area overflowed by one
/// physical pixel, took part of the wheel and moved the footer about a pixel down and back up.
#[test]
fn scrolling_the_paths_list_does_not_move_the_footer() {
    // Heights that moved it at 125 % (two bands of window heights), and ones that never did.
    for (ppp, height) in [(1.25, 520.0), (1.25, 900.0), (1.25, 960.0), (1.0, 900.0), (1.5, 900.0)] {
        let mut h = harness(30, vec2(1440.0, height), ppp);
        let footer = |h: &Harness<'_, PhotocraftApp>| h.query_by_label("Delete path").map(|r| r.rect().min.y);
        let rest = footer(&h).unwrap_or_else(|| panic!("the footer shows at {ppp}x, {height} high"));
        let first_row = h.query_by_label("Path 1").unwrap().rect();
        h.hover_at(first_row.center());
        h.run_steps(2);
        for i in 0..40 {
            // Down to the end of the list and past it, then back up.
            let dir = if i < 30 { -1.0 } else { 1.0 };
            h.event(egui::Event::MouseWheel {
                source: egui::MouseWheelSource::Unknown,
                unit: egui::MouseWheelUnit::Line,
                delta: vec2(0.0, dir),
                phase: egui::TouchPhase::Move,
                modifiers: Default::default(),
            });
            h.run_steps(1);
            assert_eq!(footer(&h), Some(rest), "{ppp}x, {height} high, notch {i}: the footer stayed put");
        }
        // The list itself did scroll (the first row is out of view or has moved).
        let moved = h.query_by_label("Path 1").map(|r| r.rect().min.y) != Some(first_row.min.y);
        assert!(moved || h.query_by_label("Path 1").is_none(), "{ppp}x, {height} high: the list scrolled");
    }
}

/// Channels, Paths and Layers are siblings with the same own-scroll layout, so none is wrapped in
/// the dock's scroll area.
#[test]
fn the_layers_group_tabs_all_scroll_themselves() {
    use crate::dock::Group;
    for tab in 0..=2 {
        assert!(Group::Layers.scrolls_itself(tab), "Layers group tab {tab}");
    }
    assert!(!Group::Layers.scrolls_itself(3), "a tab that does not exist is not special");
}
