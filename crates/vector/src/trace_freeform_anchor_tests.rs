//! Freeform Pen anchors against the reference application's sample strokes (#12838): mouse moves
//! in document coordinates, fitted at Curve Fit 2 px unless said otherwise.

use std::f64::consts::TAU;

use super::*;

/// (100,400) to (500,400) in 4 px steps.
fn line() -> Vec<[f64; 2]> {
    (0..=100).map(|i| [100.0 + 4.0 * i as f64, 400.0]).collect()
}

/// x from 100 to 500 in 4 px steps, y = 400 + 60·sin(2π(x − 100)/200): two periods.
fn sine() -> Vec<[f64; 2]> {
    (0..=100).map(|i| 100.0 + 4.0 * i as f64).map(|x| [x, 400.0 + 60.0 * ((x - 100.0) / 200.0 * TAU).sin()]).collect()
}

/// Centre (600,300), radius 100, `steps` of 60 around from (700,300): 60 is the whole circle
/// back to the start, 54 the open one.
fn circle(steps: usize) -> Vec<[f64; 2]> {
    (0..=steps).map(|i| i as f64 * TAU / 60.0).map(|a| [600.0 + 100.0 * a.cos(), 300.0 + 100.0 * a.sin()]).collect()
}

/// (100,450) to (250,550) to (400,450), 45 steps a leg.
fn vee() -> Vec<[f64; 2]> {
    let legs = [[100.0, 450.0], [250.0, 550.0], [400.0, 450.0]];
    legs.windows(2)
        .flat_map(|w| (0..45).map(move |k| k as f64 / 45.0).map(move |t| [w[0][0] + (w[1][0] - w[0][0]) * t, w[0][1] + (w[1][1] - w[0][1]) * t]))
        .chain([legs[2]])
        .collect()
}

/// The five strokes, with whether the drag ends on its start (closes).
fn strokes() -> [(&'static str, Vec<[f64; 2]>, bool); 5] {
    [("line", line(), false), ("sine", sine(), false), ("circle", circle(60), true), ("circle-open", circle(54), false), ("vee", vee(), false)]
}

/// The stroke as given and on the whole pixels a mouse delivers at 100 % zoom.
fn as_given_and_in_pixels(points: Vec<[f64; 2]>) -> [Vec<[f64; 2]>; 2] {
    let pixels = points.iter().map(|p| [p[0].round(), p[1].round()]).collect();
    [points, pixels]
}

fn fit(points: &[[f64; 2]], curve_fit: f64, closed: bool) -> Subpath {
    let sp = fit_trace(points, curve_fit, closed);
    assert!(sp.is_some(), "the stroke fits");
    sp.unwrap_or_else(|| Subpath { closed, knots: Vec::new(), op: PathOp::Combine })
}

fn anchors(sp: &Subpath) -> Vec<[f64; 2]> {
    sp.knots.iter().map(|k| [k.anchor.x, k.anchor.y]).collect()
}

/// Largest distance from a sample to the fitted subpath (each segment densely sampled).
fn max_deviation(sp: &Subpath, samples: &[[f64; 2]]) -> f64 {
    let curve: Vec<P> = sp
        .segments()
        .iter()
        .flat_map(|s| {
            let b = [(s[0].x, s[0].y), (s[1].x, s[1].y), (s[2].x, s[2].y), (s[3].x, s[3].y)];
            (0..=1000).map(move |k| bezier(&b, k as f64 / 1000.0))
        })
        .collect();
    samples.iter().map(|q| curve.iter().map(|c| len(sub(*c, (q[0], q[1])))).fold(f64::MAX, f64::min)).fold(0.0, f64::max)
}

#[test]
fn sine_anchors_sit_on_the_four_peaks() {
    for samples in as_given_and_in_pixels(sine()) {
        let sp = fit(&samples, 2.0, false);
        let got = anchors(&sp);
        assert_eq!(got.len(), 6, "{got:?}");
        assert_eq!((got.first(), got.last()), (samples.first(), samples.last()), "press and release are anchors");
        for (a, peak) in got[1..5].iter().zip([[150.0, 460.0], [250.0, 340.0], [350.0, 460.0], [450.0, 340.0]]) {
            assert!((a[0] - peak[0]).hypot(a[1] - peak[1]) <= 3.0, "{a:?} is not on the peak {peak:?}: {got:?}");
        }
        assert!(sp.knots[1..5].iter().all(|k| k.smooth));
    }
}

#[test]
fn closed_circle_has_five_anchors_and_the_open_one_four() {
    for samples in as_given_and_in_pixels(circle(60)) {
        let sp = fit(&samples, 2.0, true);
        assert!(sp.closed);
        assert_eq!(sp.knots.len(), 5, "{:?}", anchors(&sp));
        assert!(sp.knots.iter().all(|k| k.smooth), "a circle has no corners");
    }
    for samples in as_given_and_in_pixels(circle(54)) {
        let sp = fit(&samples, 2.0, false);
        assert!(!sp.closed);
        assert_eq!(sp.knots.len(), 4, "{:?}", anchors(&sp));
        assert!(sp.knots[1..3].iter().all(|k| k.smooth));
    }
}

/// On whole pixels the first segment past the turn fits worse over a few samples than over many;
/// the fit still reaches past them to the reference application's four anchors.
#[test]
fn vee_turns_with_a_rounded_join_not_a_cusp() {
    for (i, samples) in as_given_and_in_pixels(vee()).into_iter().enumerate() {
        let sp = fit(&samples, 2.0, false);
        let got = anchors(&sp);
        assert_eq!(got.len(), 4, "{got:?}");
        assert_eq!((got[0], got[3]), ([100.0, 450.0], [400.0, 450.0]));
        assert!(sp.knots[1..3].iter().all(|k| k.smooth), "no corner anchor: {got:?}");
        if i == 0 {
            // As measured, both inner anchors are near the turn, not along the legs.
            assert!(got[1..3].iter().all(|a| (a[0] - 250.0).hypot(a[1] - 550.0) < 60.0), "{got:?}");
        }
    }
}

#[test]
fn line_is_unchanged_one_segment_with_handles_at_a_third() {
    let sp = fit(&line(), 2.0, false);
    assert_eq!(anchors(&sp), [[100.0, 400.0], [500.0, 400.0]]);
    let (a, b) = (&sp.knots[0], &sp.knots[1]);
    assert!((a.out_ctrl.x - (100.0 + 400.0 / 3.0)).abs() < 1e-9 && a.out_ctrl.y == 400.0);
    assert!((b.in_ctrl.x - (500.0 - 400.0 / 3.0)).abs() < 1e-9 && b.in_ctrl.y == 400.0);
}

#[test]
fn a_lower_curve_fit_gives_more_anchors() {
    for (name, points, closed) in [("sine", sine(), false), ("circle-open", circle(54), false)] {
        let at_2 = fit(&points, 2.0, closed).knots.len();
        for curve_fit in [0.5, 1.0] {
            let n = fit(&points, curve_fit, closed).knots.len();
            assert!(n > at_2, "{name}: {n} anchors at Curve Fit {curve_fit}, {at_2} at 2");
        }
    }
}

/// Today's guarantee: the path stays within Curve Fit of every sample, on the strokes as given
/// and on the whole pixels a mouse delivers.
#[test]
fn every_sample_stays_within_curve_fit() {
    for (name, points, closed) in strokes() {
        for samples in as_given_and_in_pixels(points) {
            for curve_fit in [0.5, 1.0, 2.0, 5.0, 10.0] {
                let sp = fit(&samples, curve_fit, closed);
                let d = max_deviation(&sp, &samples);
                assert!(d <= curve_fit + 0.05, "{name} at Curve Fit {curve_fit}: a sample {d:.3} px away");
            }
        }
    }
}
