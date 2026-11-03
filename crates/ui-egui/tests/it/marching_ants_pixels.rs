//! #7489: the marching ants are drawn in device pixels, the same on every edge, at any display
//! scale, as Photoshop draws them.
//!
//! The ants were snapped to the middle of an egui point and drawn one point wide. At 125 % or
//! 150 % a point is not a whole number of pixels, so one edge of a rectangle came out 2 px wide
//! and the next 1 px, and the dashes 5 px long here and 4 px there. Photoshop draws them on the
//! screen's own pixels: black and white dashes, every edge as thick as every other, thicker on a
//! scaled display, the dash pattern running diagonally across the screen so a 2 px line's dashes
//! end on a slant.
//!
//! These tests render a committed rectangular selection with the real app offscreen on wgpu and
//! read the pixels along each edge. They skip when no GPU adapter exists.

use photocraft_ui_egui::PhotocraftApp;
use photocraft_ui_egui::canvas::ViewXform;
use photocraft_ui_egui::control::{ControlRequest, handle};
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

type Harness = egui_kittest::Harness<'static, PhotocraftApp>;

/// A rendered frame: RGBA8, physical pixels.
struct Image {
    w: u32,
    h: u32,
    px: Vec<u8>,
}

impl Image {
    /// Luma of pixel `(x, y)`, or `None` off the frame.
    fn luma(&self, x: i64, y: i64) -> Option<u8> {
        if x < 0 || y < 0 || x >= i64::from(self.w) || y >= i64::from(self.h) {
            return None;
        }
        let i = (y as usize * self.w as usize + x as usize) * 4;
        let p = self.px.get(i..i + 3)?;
        Some(((u16::from(p[0]) * 3 + u16::from(p[1]) * 6 + u16::from(p[2])) / 10) as u8)
    }
}

/// The app on an offscreen wgpu harness, or `None` when no GPU adapter exists. Only a panic
/// before the app callback runs (creating the adapter) is a skip; one from the app's own setup
/// fails the test rather than passing it untested.
fn harness(ppp: f32) -> Option<Harness> {
    let started = Arc::new(AtomicBool::new(false));
    let started_in = Arc::clone(&started);
    let built = std::panic::catch_unwind(move || {
        egui_kittest::Harness::builder().with_size(egui::vec2(900.0, 640.0)).with_pixels_per_point(ppp).with_max_steps(64).wgpu().build_eframe(move |cc| {
            started_in.store(true, Ordering::SeqCst);
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
        Err(payload) if started.load(Ordering::SeqCst) => std::panic::resume_unwind(payload),
        Err(_) => {
            eprintln!("skipping: no GPU adapter");
            None
        }
    }
}

fn control(h: &mut Harness, method: &str, params: Value) {
    let ctx = h.ctx.clone();
    let (req, _rx) = ControlRequest::new(method, params);
    handle(h.state_mut(), &ctx, &req);
    h.run_steps(3);
}

/// A fresh 400 × 300 document in `background` at 100 % with the rectangle `r` selected.
fn setup(h: &mut Harness, background: &str, r: [i32; 4]) {
    {
        let app = h.state_mut();
        while app.session.active().is_some() {
            app.run("file.close", json!({"discard": true})).expect("close");
        }
        app.run("file.new", json!({"width": 400, "height": 300, "background": background})).expect("new");
        app.sync_views();
        app.ui.extras.rulers = false;
        app.run("select.rect", json!({"x": r[0], "y": r[1], "width": r[2] - r[0], "height": r[3] - r[1]})).expect("select");
    }
    control(h, "ui.set", json!({"tool": "rectMarquee", "zoom": 1.0, "center": [200, 150]}));
    h.run_steps(3);
}

fn render(h: &mut Harness) -> Image {
    let img = h.render().expect("render");
    Image { w: img.width(), h: img.height(), px: img.into_raw() }
}

/// Device pixel of document point `(x, y)`.
fn device(h: &Harness, ppp: f32, x: f32, y: f32) -> (i64, i64) {
    let p = ViewXform::active(h.state()).expect("an active document view").to_screen(x, y) * ppp;
    (p.x.round() as i64, p.y.round() as i64)
}

/// One edge of the outline as seen on screen: for every device pixel along its middle, the
/// pixels across it (±6) that are dark.
struct Edge {
    /// Dark pixels across the edge at each step along it.
    across: Vec<Vec<i64>>,
    /// Pixels in the band that are neither black nor white (antialiasing grey).
    greys: usize,
}

/// Scan the edge from device pixel `a` to `b` (one of x or y shared), skipping 12 px at each end.
fn scan(img: &Image, a: (i64, i64), b: (i64, i64)) -> Edge {
    let vertical = a.0 == b.0;
    let (lo, hi) = if vertical { (a.1.min(b.1), a.1.max(b.1)) } else { (a.0.min(b.0), a.0.max(b.0)) };
    let mut across = Vec::new();
    let mut greys = 0;
    for t in lo + 12..hi - 12 {
        let mut dark = Vec::new();
        for o in -6..=6 {
            let (x, y) = if vertical { (a.0 + o, t) } else { (t, a.1 + o) };
            let l = img.luma(x, y).unwrap_or(255);
            if l < 64 {
                dark.push(o);
            }
            if (24..232).contains(&l) {
                greys += 1;
            }
        }
        across.push(dark);
    }
    Edge { across, greys }
}

/// The line is `w` px thick on every edge, crisp, with dashes of 4·w px.
#[test]
fn every_edge_is_equally_thick_at_any_display_scale() {
    let _gpu = photocraft_testkit::gpu_lock();
    for ppp in [1.0f32, 1.25, 1.5, 2.0] {
        let Some(mut h) = harness(ppp) else { return };
        h.run_steps(4);
        // Edges at odd and even document columns, so a fractional scale lands them at different
        // fractions of a pixel.
        let r = [101, 60, 203, 241];
        setup(&mut h, "white", r);
        let img = render(&mut h);
        let w = ppp.ceil() as usize;
        let [x0, y0, x1, y1] = r.map(|v| v as f32);
        let (tl, tr, br, bl) = (device(&h, ppp, x0, y0), device(&h, ppp, x1, y0), device(&h, ppp, x1, y1), device(&h, ppp, x0, y1));
        let sides = [("top", (tl, tr)), ("right", (tr, br)), ("bottom", (bl, br)), ("left", (tl, bl))];
        let edges: Vec<(&str, Edge)> = sides.iter().map(|&(name, (a, b))| (name, scan(&img, a, b))).collect();
        // The bug: at 125 % one edge came out 2 px thick and the next 1 px.
        let thickest: Vec<(&str, usize)> = edges.iter().map(|(name, e)| (*name, e.across.iter().map(Vec::len).max().unwrap_or(0))).collect();
        assert!(thickest.iter().all(|t| t.1 == thickest[0].1), "@{ppp}x: every edge as thick as the others: {thickest:?}");
        for (name, e) in edges {
            assert_eq!(e.greys, 0, "@{ppp}x {name}: the ants are crisp black and white");
            let thick: Vec<usize> = e.across.iter().map(Vec::len).filter(|&n| n > 0).collect();
            assert!(!thick.is_empty(), "@{ppp}x {name}: the edge shows");
            // Where a dash covers every row across the line it is exactly `w` thick; at a dash's
            // slanted end only some of them are dark.
            let max = thick.iter().copied().max().unwrap_or(0);
            assert_eq!(max, w, "@{ppp}x {name}: {w} px thick, got {thick:?}");
            // Half the line is black: dashes and gaps of 4·w px each.
            let dark: usize = e.across.iter().map(Vec::len).sum();
            let frac = dark as f32 / (e.across.len() * w) as f32;
            assert!((0.45..=0.55).contains(&frac), "@{ppp}x {name}: {frac:.2} of the line is black");
            // Along any one row of the line, black runs are 4·w px long.
            let rows: std::collections::BTreeSet<i64> = e.across.iter().flatten().copied().collect();
            for row in rows {
                let on: Vec<bool> = e.across.iter().map(|d| d.contains(&row)).collect();
                let mut runs = Vec::new();
                let mut n = 0;
                for (i, &b) in on.iter().enumerate() {
                    if b {
                        n += 1;
                    }
                    if (!b || i + 1 == on.len()) && n > 0 {
                        runs.push(n);
                        n = 0;
                    }
                }
                // The first and last run may be cut by the scan's ends.
                let inner = runs.get(1..runs.len().saturating_sub(1)).unwrap_or(&[]);
                assert!(inner.iter().all(|&n| n == 4 * w), "@{ppp}x {name} row {row}: dashes {runs:?}, want {} px", 4 * w);
            }
        }
    }
}

/// On black the white dashes show, between black ones.
#[test]
fn the_ants_alternate_black_and_white() {
    let _gpu = photocraft_testkit::gpu_lock();
    let Some(mut h) = harness(1.0) else { return };
    h.run_steps(4);
    let r = [100, 60, 200, 240];
    setup(&mut h, "black", r);
    let img = render(&mut h);
    let (a, b) = (device(&h, 1.0, 100.0, 60.0), device(&h, 1.0, 200.0, 60.0));
    let mut bright = 0;
    let mut n = 0;
    for x in a.0 + 12..b.0 - 12 {
        n += 1;
        bright += usize::from((-2..=2).any(|o| img.luma(x, a.1 + o).is_some_and(|l| l > 230)));
    }
    let frac = bright as f32 / n as f32;
    assert!((0.4..=0.6).contains(&frac), "{frac:.2} of the top edge is white on black");
}
