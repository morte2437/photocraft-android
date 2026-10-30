use super::*;
use eframe::App as _;

/// The shell's side strip (the Android Shortcut Pad) starts below the menu and options bars,
/// which run the window's full width, and the rest of the app lays out beside it.
#[test]
fn the_edge_strip_sits_below_the_top_bars() {
    for right in [false, true] {
        let ctx = egui::Context::default();
        PhotocraftApp::setup_context(&ctx, crate::theme::ThemeKind::Pro);
        let mut app = PhotocraftApp::new(Session::new(), Services::default());
        app.session.execute("file.new", serde_json::json!({"width": 64, "height": 64})).unwrap();
        app.sync_views();
        app.edge_strip = Some((right, 60.0));
        let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 700.0));
        let run = |app: &mut PhotocraftApp| {
            let input = egui::RawInput { screen_rect: Some(screen), ..Default::default() };
            let mut frame = eframe::Frame::_new_kittest();
            let mut out = ctx.run_ui(input, |ui| {
                app.logic(ui.ctx(), &mut frame);
                app.ui(ui, &mut frame);
            });
            out.textures_delta.clear();
        };
        for _ in 0..3 {
            run(&mut app);
        }
        let strip = app.edge_strip_rect.expect("the strip is laid out");
        assert_eq!(strip.width(), 60.0, "{strip:?}");
        if right {
            assert_eq!(strip.right(), screen.right());
        } else {
            assert_eq!(strip.left(), screen.left());
        }
        // Below the menu bar and the options bar.
        assert!(strip.top() > 50.0, "{strip:?}");
        // Nothing of the app is laid out over it: the canvas is beside it.
        assert!(!app.last_canvas_rect.intersects(strip.shrink(1.0)), "{:?} {strip:?}", app.last_canvas_rect);
        // Without one, nothing is kept.
        app.edge_strip = None;
        run(&mut app);
        assert_eq!(app.edge_strip_rect, None);
    }
}
