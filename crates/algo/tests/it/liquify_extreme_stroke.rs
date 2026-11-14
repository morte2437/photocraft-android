//! A project file stores Liquify strokes as plain numbers, so a stroke may name any `f64` for its
//! brush size or its points. Replaying one must never overflow, however absurd the values.

use photocraft_algo::liquify::{LiquifyField, LiquifyStroke, LiquifyTool};
use photocraft_geom::Rect;

const BRUSH_TOOLS: [LiquifyTool; 10] = [
    LiquifyTool::ForwardWarp,
    LiquifyTool::Reconstruct,
    LiquifyTool::Smooth,
    LiquifyTool::TwirlCw,
    LiquifyTool::TwirlCcw,
    LiquifyTool::Pucker,
    LiquifyTool::Bloat,
    LiquifyTool::PushLeft,
    LiquifyTool::Freeze,
    LiquifyTool::Thaw,
];

const EXTREME: [f64; 9] = [f64::NAN, f64::INFINITY, f64::NEG_INFINITY, f64::MAX, f64::MIN, 1e300, 3.0e9, -1.0, 0.0];

/// A stroke of `tool` with brush `size` through `points`, at middling density, pressure and rate.
fn stroke(tool: LiquifyTool, size: f64, points: Vec<Vec<f64>>) -> LiquifyStroke {
    LiquifyStroke { tool, size, density: 50.0, pressure: 50.0, rate: 50.0, points, amount: None }
}

/// Every brush tool, with every absurd size and position, replays without a panic.
#[test]
fn a_brush_with_an_absurd_size_or_position_replays_without_overflow() {
    let bounds = Rect::new(0, 0, 64, 64);
    for tool in BRUSH_TOOLS {
        for size in EXTREME {
            for p in [0.0, 1e300, f64::NAN, 3.0e9, -3.0e9] {
                let points = vec![vec![p, 10.0], vec![20.0, p], vec![p, p, f64::MAX]];
                let _ = LiquifyField::from_strokes(bounds, 8.0, &[stroke(tool, size, points)]);
            }
        }
    }
}

/// Review on #11013: a dab whose centre and size are both huge gives infinity over infinity for its
/// distance; it must not write NaN displacements.
#[test]
fn a_huge_far_off_dab_leaves_the_field_finite() {
    let bounds = Rect::new(0, 0, 64, 64);
    for tool in BRUSH_TOOLS {
        let f = LiquifyField::from_strokes(bounds, 8.0, &[stroke(tool, 4e200, vec![vec![1e200, 10.0]])]);
        assert!(f.max_displacement().is_finite(), "{tool:?}");
        assert!(f.sample(10.0, 10.0).iter().all(|v| v.is_finite()), "{tool:?}");
    }
}

/// Review on #11013: a Freeze dab centred far to the left with a radius reaching the image changes
/// nodes inside it, so the area it reports must not be empty.
#[test]
fn a_far_off_dab_that_reaches_the_image_reports_what_it_changed() {
    let mut f = LiquifyField::new(Rect::new(0, 0, 64, 64), 8.0);
    let changed = f.stroke_begin(&stroke(LiquifyTool::Freeze, 1.2e10, vec![]), [-3e9, 10.0, 1.0]);
    assert!(f.freeze_at(10.0, 10.0) > 0.0, "the dab reaches the image, so it freezes nodes there");
    assert!(!changed.is_empty(), "the freeze changed nodes but reported {changed:?}");
}
