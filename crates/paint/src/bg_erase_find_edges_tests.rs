//! Background Eraser Limits Find Edges (#11904): a blurred edge is erased graded, keeping at least
//! as much of the transition as Contiguous does, instead of cut hard at the Tolerance. Expected
//! values are the reference application's measurements from the issue (8-bit RGB, 40 px and 60 px
//! hard round tips, Tolerance 50, Sampling Continuous).

use photocraft_color::PixelFormat;
use photocraft_geom::Rect;
use photocraft_raster::{Surface, from_rgba};

use crate::bg_erase::{BgEraseSettings, apply_background_eraser};
use crate::replace::{Limits, Sampling};
use crate::{BrushSettings, Stroke, StrokePoint};

/// Measured red channel along y 70 from x 185 across the 4 px Gaussian red|green edge at x 200.
const RED: [i32; 30] = [220, 220, 220, 220, 219, 219, 217, 214, 210, 203, 194, 182, 168, 152, 134, 116, 98, 82, 68, 56, 47, 40, 36, 33, 31, 31, 30, 30, 30, 30];
/// Alpha along y 70 from x 185 after a click at 190,70.
const FIND_EDGES: [i32; 15] = [8, 8, 8, 4, 4, 0, 0, 0, 17, 42, 76, 121, 176, 238, 255];
const CONTIGUOUS: [i32; 15] = [0, 0, 0, 0, 0, 0, 0, 4, 21, 46, 79, 123, 178, 238, 255];

fn rgb(c: [i32; 3]) -> [f32; 4] {
    [c[0] as f32 / 255.0, c[1] as f32 / 255.0, c[2] as f32 / 255.0, 1.0]
}

/// The issue's test layer: red | green | blue | pink stripes (y 0-300), a white gap at y 140-160
/// across the red stripe, and the red|green edge blurred over y 0-130. A Gaussian blur mixes every
/// channel with the same weights, so green and blue follow red's mix fraction.
fn layer() -> Surface {
    let fmt = PixelFormat::RGBA8;
    let mut s = Surface::new(fmt);
    for (x0, c) in [(0, [220, 30, 30]), (200, [30, 180, 60]), (400, [40, 60, 220]), (600, [240, 120, 120])] {
        s.fill_rect(Rect::new(x0, 0, x0 + 200, 300), &from_rgba(&fmt, rgb(c)));
    }
    s.fill_rect(Rect::new(0, 140, 200, 160), &from_rgba(&fmt, rgb([255, 255, 255])));
    for (i, r) in RED.iter().enumerate() {
        let t = (220 - r) as f32 / 190.0;
        let c = [*r as f32 / 255.0, (30.0 + 150.0 * t) / 255.0, (30.0 + 30.0 * t) / 255.0, 1.0];
        let x = 185 + i as i32;
        s.fill_rect(Rect::new(x, 0, x + 1, 130), &from_rgba(&fmt, c));
    }
    s
}

fn click(at: (f64, f64), size: f32, limits: Limits) -> Surface {
    let mut s = layer();
    let stroke = Stroke {
        brush: BrushSettings { size, hardness: 1.0, spacing: 0.25, pressure_size: false, ..Default::default() },
        points: vec![StrokePoint::new(at.0, at.1, 1.0)],
    };
    let bs = BgEraseSettings { sampling: Sampling::Continuous, limits, tolerance: 0.5, ..Default::default() };
    apply_background_eraser(&mut s, &stroke, &bs, None);
    s
}

fn alpha(s: &Surface, x: i32, y: i32) -> i32 {
    (s.rgba(x, y)[3] * 255.0).round() as i32
}

#[test]
fn find_edges_grades_a_blurred_edge_instead_of_cutting_it() {
    let fe = click((190.0, 70.0), 40.0, Limits::FindEdges);
    let co = click((190.0, 70.0), 40.0, Limits::Contiguous);
    for (i, want) in FIND_EDGES.iter().enumerate() {
        let x = 185 + i as i32;
        let (f, c) = (alpha(&fe, x, 70), alpha(&co, x, 70));
        assert!((f - want).abs() <= 10, "x {x}: Find Edges alpha {f}, measured {want}");
        assert!(f >= c, "x {x}: Find Edges keeps {f}, less than Contiguous's {c}");
    }
    // Past the transition nothing is erased (before, Find Edges cut through x 201).
    for x in 200..230 {
        assert_eq!(alpha(&fe, x, 70), 255, "x {x}");
    }
}

#[test]
fn find_edges_on_a_hard_edge_gives_the_contiguous_result() {
    // 60 px tip at 100,138 on red just above the white gap: column x 100 erased through y 139, the
    // gap and the red below it kept.
    let fe = click((100.0, 138.0), 60.0, Limits::FindEdges);
    let co = click((100.0, 138.0), 60.0, Limits::Contiguous);
    for y in 100..176 {
        assert_eq!(alpha(&fe, 100, y), alpha(&co, 100, y), "y {y}");
    }
    for y in 110..140 {
        assert_eq!(alpha(&fe, 100, y), 0, "red above the gap, y {y}");
    }
    for y in 140..176 {
        assert_eq!(alpha(&fe, 100, y), 255, "the gap and the red below it, y {y}");
    }
}

#[test]
fn contiguous_and_discontiguous_keep_their_results() {
    // This item changes only Find Edges: the other two limits still give the measured row.
    for limits in [Limits::Contiguous, Limits::Discontiguous] {
        let s = click((190.0, 70.0), 40.0, limits);
        for (i, want) in CONTIGUOUS.iter().enumerate() {
            let x = 185 + i as i32;
            assert!((alpha(&s, x, 70) - want).abs() <= 8, "{limits:?} x {x}: {} against {want}", alpha(&s, x, 70));
        }
    }
}
