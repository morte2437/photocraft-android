//! Faithful vector subset for PDF shape export; unsupported shapes use raster fallback.

use std::fmt::Write as _;

use photocraft_color::ColorMode;
use photocraft_doc::{Fill, FillRule, LineCap, LineJoin, PathOp, ShapeLayer, StrokeAlign};

/// Return a PDF path and fill operator for a directly representable shape.
/// Coordinates remain in document pixels with y increasing down; the caller applies transforms.
pub(super) fn shape_path(layer: &ShapeLayer, mode: ColorMode) -> Option<String> {
    if layer.has_soft_mask() || layer.path.inverted || layer.path.subpaths.len() != 1 {
        return None;
    }
    if layer.fill.is_none() && layer.stroke.is_none() {
        return None;
    }
    let channels = match mode {
        ColorMode::Rgb => 3,
        ColorMode::Grayscale => 1,
        ColorMode::Cmyk => 4,
        _ => return None,
    };
    let mut out = String::new();
    if let Some(fill) = &layer.fill {
        let Fill::Solid(color) = fill else { return None };
        if color.mode != mode || !color.alpha.is_finite() || color.alpha != 1.0 || color.c[..channels].iter().any(|v| !v.is_finite()) {
            return None;
        }
        for value in color.c.iter().take(channels) {
            if !(0.0..=1.0).contains(value) {
                return None;
            }
            write!(out, "{value} ").ok()?;
        }
        out.push_str(match mode {
            ColorMode::Rgb => "rg\n",
            ColorMode::Grayscale => "g\n",
            ColorMode::Cmyk => "k\n",
            _ => return None,
        });
    }
    if let Some(stroke) = &layer.stroke {
        if stroke.align != StrokeAlign::Center || stroke.opacity != 1.0 || stroke.validate().is_err() || stroke.width <= 0.0 {
            return None;
        }
        let Fill::Solid(stroke_color) = &stroke.paint else { return None };
        if stroke_color.mode != mode || stroke_color.alpha != 1.0 || !stroke_color.alpha.is_finite() {
            return None;
        }
        for value in stroke_color.c.iter().take(channels) {
            if !value.is_finite() || !(0.0..=1.0).contains(value) {
                return None;
            }
            write!(out, "{value} ").ok()?;
        }
        out.push_str(match mode {
            ColorMode::Rgb => "RG\n",
            ColorMode::Grayscale => "G\n",
            ColorMode::Cmyk => "K\n",
            _ => return None,
        });
        let cap = match stroke.cap {
            LineCap::Butt => 0,
            LineCap::Round => 1,
            LineCap::Square => 2,
        };
        let join = match stroke.join {
            LineJoin::Miter => 0,
            LineJoin::Round => 1,
            LineJoin::Bevel => 2,
        };
        writeln!(out, "{} w {cap} J {join} j {} M", stroke.width, stroke.miter_limit).ok()?;
        out.push('[');
        for dash in &stroke.dashes {
            write!(out, "{} ", dash * stroke.width).ok()?;
        }
        writeln!(out, "] {} d", stroke.dash_offset * stroke.width).ok()?;
    }
    let subpath = layer.path.subpaths.first()?;
    if !subpath.closed || subpath.knots.len() < 2 || subpath.op != PathOp::Combine {
        return None;
    }
    let first = subpath.knots.first()?;
    if !point_finite(first.anchor) {
        return None;
    }
    writeln!(out, "{} {} m", first.anchor.x, first.anchor.y).ok()?;
    for segment in subpath.segments() {
        if segment.iter().any(|p| !point_finite(*p)) {
            return None;
        }
        writeln!(out, "{} {} {} {} {} {} c", segment[1].x, segment[1].y, segment[2].x, segment[2].y, segment[3].x, segment[3].y).ok()?;
    }
    out.push_str("h\n");
    out.push_str(match (layer.fill.is_some(), layer.stroke.is_some(), layer.path.fill_rule) {
        (true, false, FillRule::NonZero) => "f\n",
        (true, false, FillRule::EvenOdd) => "f*\n",
        (false, true, _) => "S\n",
        (true, true, FillRule::NonZero) => "B\n",
        (true, true, FillRule::EvenOdd) => "B*\n",
        (false, false, _) => return None,
    });
    Some(out)
}

fn point_finite(p: photocraft_geom::Point) -> bool {
    p.x.is_finite() && p.y.is_finite()
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Knot, Path, Subpath};
    use photocraft_geom::Point;

    #[test]
    fn closed_cubic_emits_path_and_fill() {
        let layer = ShapeLayer {
            fill: Some(Fill::Solid(photocraft_color::Color::rgb(1.0, 0.0, 0.0))),
            path: Path::new(vec![Subpath {
                closed: true,
                knots: vec![Knot::smooth(Point::new(0.0, 0.0), Point::new(-1.0, 0.0), Point::new(1.0, 0.0)), Knot::corner(10.0, 0.0)],
                ..Subpath::default()
            }]),
            ..Default::default()
        };
        let pdf = shape_path(&layer, ColorMode::Rgb).unwrap();
        assert!(pdf.contains("1 0 10 0 10 0 c"));
        assert!(pdf.ends_with("h\nf\n"));
    }

    #[test]
    fn rejects_open_path_and_mismatched_mode() {
        let mut layer = ShapeLayer {
            fill: Some(Fill::Solid(photocraft_color::Color::rgb(1.0, 0.0, 0.0))),
            path: Path::new(vec![Subpath::polyline(&[(0.0, 0.0), (1.0, 1.0)])]),
            ..Default::default()
        };
        assert!(shape_path(&layer, ColorMode::Rgb).is_none());
        layer.path.subpaths[0].closed = true;
        assert!(shape_path(&layer, ColorMode::Cmyk).is_none());
    }

    #[test]
    fn centered_stroke_keeps_its_width_color_and_dash_pattern() {
        let layer = ShapeLayer {
            fill: Some(Fill::Solid(photocraft_color::Color::rgb(1.0, 0.0, 0.0))),
            path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])]),
            stroke: Some(photocraft_doc::ShapeStroke { width: 2.0, dashes: vec![2.0, 1.0], ..Default::default() }),
            ..Default::default()
        };
        let ops = shape_path(&layer, ColorMode::Rgb).unwrap();
        assert!(ops.contains("0 0 0 RG\n"));
        assert!(ops.contains("2 w 0 J 0 j 100 M\n[4 2 ] 0 d\n"));
        assert!(ops.ends_with("h\nB\n"));
        let inside = ShapeLayer { stroke: Some(photocraft_doc::ShapeStroke { align: StrokeAlign::Inside, ..Default::default() }), ..layer };
        assert!(shape_path(&inside, ColorMode::Rgb).is_none());
    }

    #[test]
    fn stroke_only_emits_stroke_operator() {
        let layer = ShapeLayer {
            path: Path::new(vec![Subpath::polygon(&[(0.0, 0.0), (10.0, 0.0), (10.0, 10.0)])]),
            stroke: Some(photocraft_doc::ShapeStroke { width: 1.0, ..Default::default() }),
            ..Default::default()
        };
        let ops = shape_path(&layer, ColorMode::Rgb).unwrap();
        assert!(!ops.contains("rg\n"));
        assert!(ops.contains("0 0 0 RG\n"));
        assert!(ops.ends_with("h\nS\n"));
    }
}
