//! Faithful vector subset for PDF shape export; unsupported shapes use raster fallback.

use craft_pdf::write_raster::{ContentFillRule, ContentPath, ContentStroke, DeviceColor, PathElement};
use photocraft_color::ColorMode;
use photocraft_doc::{Fill, FillRule, LineCap, LineJoin, PathOp, ShapeLayer, StrokeAlign};

/// Convert a directly representable PhotoCraft shape to neutral vector geometry.
pub(super) fn shape_content(layer: &ShapeLayer, mode: ColorMode) -> Option<ContentPath> {
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
    let mut fill_color = None;
    if let Some(fill) = &layer.fill {
        let Fill::Solid(color) = fill else { return None };
        if color.mode != mode || !color.alpha.is_finite() || color.alpha != 1.0 || color.c[..channels].iter().any(|v| !v.is_finite()) {
            return None;
        }
        for value in color.c.iter().take(channels) {
            if !(0.0..=1.0).contains(value) {
                return None;
            }
        }
        fill_color = Some((device_color(color.c, mode)?, content_fill_rule(layer.path.fill_rule)));
    }
    let mut stroke_command = None;
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
        }
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
        stroke_command = Some(ContentStroke {
            color: device_color(stroke_color.c, mode)?,
            width: f64::from(stroke.width),
            cap,
            join,
            miter_limit: f64::from(stroke.miter_limit),
            dashes: stroke.dashes.iter().map(|dash| f64::from(dash * stroke.width)).collect(),
            dash_offset: f64::from(stroke.dash_offset * stroke.width),
        });
    }
    let subpath = layer.path.subpaths.first()?;
    if !subpath.closed || subpath.knots.len() < 2 || subpath.op != PathOp::Combine {
        return None;
    }
    let first = subpath.knots.first()?;
    if !point_finite(first.anchor) {
        return None;
    }
    let mut elements = vec![PathElement::MoveTo([first.anchor.x, first.anchor.y])];
    for segment in subpath.segments() {
        if segment.iter().any(|p| !point_finite(*p)) {
            return None;
        }
        elements.push(PathElement::CubicTo([segment[1].x, segment[1].y], [segment[2].x, segment[2].y], [segment[3].x, segment[3].y]));
    }
    elements.push(PathElement::Close);
    Some(ContentPath { elements, fill: fill_color, stroke: stroke_command })
}

fn device_color(values: [f32; 4], mode: ColorMode) -> Option<DeviceColor> {
    Some(match mode {
        ColorMode::Grayscale => DeviceColor::Gray(f64::from(values[0])),
        ColorMode::Rgb => DeviceColor::Rgb([f64::from(values[0]), f64::from(values[1]), f64::from(values[2])]),
        ColorMode::Cmyk => DeviceColor::Cmyk([f64::from(values[0]), f64::from(values[1]), f64::from(values[2]), f64::from(values[3])]),
        _ => return None,
    })
}

fn content_fill_rule(rule: FillRule) -> ContentFillRule {
    match rule {
        FillRule::NonZero => ContentFillRule::NonZero,
        FillRule::EvenOdd => ContentFillRule::EvenOdd,
    }
}

#[cfg(test)]
fn shape_path(layer: &ShapeLayer, mode: ColorMode) -> Option<String> {
    let path = shape_content(layer, mode)?;
    craft_pdf::write_raster::encode_vector_content(&craft_pdf::write_raster::VectorContent { operations: vec![craft_pdf::write_raster::ContentOp::Path(path)] })
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
