//! Faithful vector and selectable text export for PDF.

use craft_pdf::write_raster::{ContentFillRule, ContentOp, ContentPath, DeviceColor, PathElement};

use photocraft_color::ColorMode;
use photocraft_doc::TextLayer;

/// Whether any glyph is thickened by faux bold or by a face lacking the weight: rendering
/// emboldens those glyphs, but [`photocraft_text::render::glyph_outlines`] gives the regular outline.
fn synthetic_bold(layout: &photocraft_text::TextLayout) -> bool {
    layout
        .glyphs
        .iter()
        .any(|g| layout.faces.get(g.face as usize).is_some_and(|f| f.embolden) || layout.styles.get(g.style as usize).is_some_and(|s| s.faux_bold))
}

/// Whether `layer` is laid out in straight horizontal lines with a finite transform and `dpi`.
/// Path text is bent along its path when rendered, and the outlines written here are not.
fn straight_horizontal(layer: &TextLayer, dpi: f32) -> bool {
    layer.warp.is_none()
        && !matches!(layer.shape, photocraft_doc::text::TextShape::Path { .. })
        && layer.orientation == photocraft_doc::text::Orientation::Horizontal
        && dpi.is_finite()
        && dpi > 0.0
        && layer.transform.m.iter().all(|v| v.is_finite())
}

/// Return PDF vector and invisible text operators for a directly representable text layer.
/// Coordinates are in document pixels with y increasing down; the caller applies the CTM.
#[cfg(test)]
fn text_ops(layer: &TextLayer, mode: ColorMode, dpi: f32) -> Option<String> {
    let prepared = prepare(layer, mode, dpi)?;
    let font = super::SelectionFont::new(std::iter::once(layer.text.as_str()));
    let commands = ops(&prepared, mode, &font)?;
    craft_pdf::write_raster::encode_vector_content(&craft_pdf::write_raster::VectorContent { operations: commands })
}

fn channels(mode: ColorMode) -> Option<usize> {
    match mode {
        ColorMode::Rgb => Some(3),
        ColorMode::Grayscale => Some(1),
        ColorMode::Cmyk => Some(4),
        _ => None,
    }
}

fn device_color(color: &photocraft_color::Color, mode: ColorMode) -> Option<DeviceColor> {
    Some(match mode {
        ColorMode::Grayscale => DeviceColor::Gray(f64::from(color.c[0])),
        ColorMode::Rgb => DeviceColor::Rgb([f64::from(color.c[0]), f64::from(color.c[1]), f64::from(color.c[2])]),
        ColorMode::Cmyk => DeviceColor::Cmyk([f64::from(color.c[0]), f64::from(color.c[1]), f64::from(color.c[2]), f64::from(color.c[3])]),
        _ => return None,
    })
}

fn outlines_permitted(bytes: &[u8], index: u32) -> bool {
    let Ok(face) = ttf_parser::Face::parse(bytes, index) else { return false };
    matches!(face.permissions(), Some(ttf_parser::Permissions::Installable | ttf_parser::Permissions::PreviewAndPrint | ttf_parser::Permissions::Editable))
        && face.is_outline_embedding_allowed()
        && face.is_subsetting_allowed()
}

/// A laid-out text layer that is directly representable, except for the selection font: its
/// codes depend on every candidate layer's characters, so the caller builds it afterwards.
pub(super) struct PreparedText<'a> {
    layer: &'a TextLayer,
    layout: photocraft_text::TextLayout,
}

impl PreparedText<'_> {
    pub(super) fn text(&self) -> &str {
        self.layer.text.as_str()
    }
}

/// Lay `layer` out and check it can be emitted as outlines plus selection text. The selection
/// font is deliberately not consulted, so a hidden or already-blocked layer cannot make a
/// supported top layer ineligible.
pub(super) fn prepare<'a>(layer: &'a TextLayer, mode: ColorMode, dpi: f32) -> Option<PreparedText<'a>> {
    if !straight_horizontal(layer, dpi) || channels(mode).is_none() {
        return None;
    }
    if layer.text.is_empty() {
        return Some(PreparedText { layer, layout: photocraft_text::TextLayout::default() });
    }
    let mut engine = photocraft_text::TextEngine::new();
    let layout = engine.layout(layer, dpi);
    layout_checks(layer, mode, &layout).then_some(PreparedText { layer, layout })
}

/// Everything that can refuse a laid-out layer independently of the selection font. The text
/// matrix products are checked here so a layer whose transform overflows is refused during
/// candidate selection, not halfway through emission.
fn layout_checks(layer: &TextLayer, mode: ColorMode, layout: &photocraft_text::TextLayout) -> bool {
    let Some(channels) = channels(mode) else { return false };
    if synthetic_bold(layout) || layout.faces.iter().any(|face| !outlines_permitted(face.font.data.as_ref(), face.font.index)) {
        return false;
    }
    for style in &layout.styles {
        let color = style.color.in_mode(mode);
        if !color.alpha.is_finite() || color.alpha != 1.0 || color.c[..channels].iter().any(|v| !v.is_finite() || !(0.0..=1.0).contains(v)) {
            return false;
        }
    }
    let [a, b, c, d, _, _] = layer.transform.m;
    for cluster in &layout.clusters {
        let Some(text) = layer.text.get(cluster.range.clone()) else { return false };
        if text.is_empty() {
            continue;
        }
        let Some(line) = layout.lines.get(cluster.line) else { return false };
        let height = (line.ascent + line.descent).max(1.0);
        let width = cluster.advance.abs() / text.chars().count().max(1) as f32;
        if !height.is_finite() || !width.is_finite() {
            return false;
        }
        let origin = layer.transform.apply(photocraft_geom::Point::new(f64::from(cluster.x), f64::from(line.baseline)));
        if !origin.x.is_finite() || !origin.y.is_finite() {
            return false;
        }
        for coefficient in [a * f64::from(width), b * f64::from(width), -c * f64::from(height), -d * f64::from(height)] {
            if !coefficient.is_finite() {
                return false;
            }
        }
    }
    true
}

pub(super) fn ops(prepared: &PreparedText<'_>, mode: ColorMode, selection_font: &super::SelectionFont) -> Option<Vec<ContentOp>> {
    channels(mode)?;
    layout_ops(prepared.layer, mode, &prepared.layout, selection_font)
}

/// Write one glyph outline as PDF path operators, refusing any coordinate or derived Bézier
/// control that is not finite. A quadratic's cubic controls come from finite differences, which
/// can overflow even when the endpoints are finite.
fn path_elements(elements: &[photocraft_text::render::PathEl]) -> Option<Vec<PathElement>> {
    let mut cur = [0.0f64, 0.0f64];
    let mut out = Vec::with_capacity(elements.len());
    for el in elements {
        match *el {
            photocraft_text::render::PathEl::MoveTo([x, y]) => {
                if !x.is_finite() || !y.is_finite() {
                    return None;
                }
                cur = [x, y];
                out.push(PathElement::MoveTo([x, y]));
            }
            photocraft_text::render::PathEl::LineTo([x, y]) => {
                if !x.is_finite() || !y.is_finite() {
                    return None;
                }
                cur = [x, y];
                out.push(PathElement::LineTo([x, y]));
            }
            photocraft_text::render::PathEl::QuadTo([cx, cy], [x, y]) => {
                if !cx.is_finite() || !cy.is_finite() || !x.is_finite() || !y.is_finite() {
                    return None;
                }
                let c1x = cur[0] + 2.0 / 3.0 * (cx - cur[0]);
                let c1y = cur[1] + 2.0 / 3.0 * (cy - cur[1]);
                let c2x = x + 2.0 / 3.0 * (cx - x);
                let c2y = y + 2.0 / 3.0 * (cy - y);
                if !c1x.is_finite() || !c1y.is_finite() || !c2x.is_finite() || !c2y.is_finite() {
                    return None;
                }
                cur = [x, y];
                out.push(PathElement::CubicTo([c1x, c1y], [c2x, c2y], [x, y]));
            }
            photocraft_text::render::PathEl::CurveTo([c1x, c1y], [c2x, c2y], [x, y]) => {
                if !c1x.is_finite() || !c1y.is_finite() || !c2x.is_finite() || !c2y.is_finite() || !x.is_finite() || !y.is_finite() {
                    return None;
                }
                cur = [x, y];
                out.push(PathElement::CubicTo([c1x, c1y], [c2x, c2y], [x, y]));
            }
            photocraft_text::render::PathEl::Close => {
                out.push(PathElement::Close);
            }
        }
    }
    Some(out)
}

#[cfg(test)]
fn write_path_elements(out: &mut String, elements: &[photocraft_text::render::PathEl]) -> Option<()> {
    let commands = path_elements(elements)?;
    // A PDF path must have a paint operation. The test exercises geometry encoding, so use a
    // neutral fill while preserving the shared writer's production validation.
    let path = ContentPath { elements: commands, fill: Some((DeviceColor::Gray(0.0), ContentFillRule::NonZero)), ..ContentPath::default() };
    let encoded = craft_pdf::write_raster::encode_vector_content(&craft_pdf::write_raster::VectorContent { operations: vec![ContentOp::Path(path)] })?;
    out.push_str(&encoded);
    Some(())
}

fn layout_ops(layer: &TextLayer, mode: ColorMode, layout: &photocraft_text::TextLayout, selection_font: &super::SelectionFont) -> Option<Vec<ContentOp>> {
    if !layout_checks(layer, mode, layout) {
        return None;
    }

    let outlines = photocraft_text::render::glyph_outlines(layout, &layer.transform, None);

    let mut out = Vec::new();
    for glyph in &outlines {
        let style = layout.styles.get(glyph.style as usize)?;
        let color = style.color.in_mode(mode);
        let fill = device_color(&color, mode)?;
        out.push(ContentOp::Path(ContentPath { elements: path_elements(&glyph.elements)?, fill: Some((fill, ContentFillRule::EvenOdd)), stroke: None }));
    }

    for d in &layout.decorations {
        let style = layout.styles.get(d.style as usize)?;
        let color = style.color.in_mode(mode);
        let p0 = layer.transform.apply(photocraft_geom::Point::new(f64::from(d.x0), f64::from(d.y0)));
        let p1 = layer.transform.apply(photocraft_geom::Point::new(f64::from(d.x1), f64::from(d.y0)));
        let p2 = layer.transform.apply(photocraft_geom::Point::new(f64::from(d.x1), f64::from(d.y1)));
        let p3 = layer.transform.apply(photocraft_geom::Point::new(f64::from(d.x0), f64::from(d.y1)));
        if !p0.x.is_finite()
            || !p0.y.is_finite()
            || !p1.x.is_finite()
            || !p1.y.is_finite()
            || !p2.x.is_finite()
            || !p2.y.is_finite()
            || !p3.x.is_finite()
            || !p3.y.is_finite()
        {
            return None;
        }
        out.push(ContentOp::Path(ContentPath {
            elements: vec![
                PathElement::MoveTo([p0.x, p0.y]),
                PathElement::LineTo([p1.x, p1.y]),
                PathElement::LineTo([p2.x, p2.y]),
                PathElement::LineTo([p3.x, p3.y]),
                PathElement::Close,
            ],
            fill: Some((device_color(&color, mode)?, ContentFillRule::NonZero)),
            stroke: None,
        }));
    }

    selection_ops(&mut out, layer, layout, selection_font)?;
    Some(out)
}

/// Emit the invisible selection text for each shaped cluster. A span reaches the next shaped
/// cluster, so paragraph breaks and other whitespace omitted by shaping survive
/// in the selection; the first span also takes leading characters. Without this a multi-line
/// layer extracts as one run of text.
fn selection_ops(out: &mut Vec<ContentOp>, layer: &TextLayer, layout: &photocraft_text::TextLayout, selection_font: &super::SelectionFont) -> Option<()> {
    let [a, b, c, d, _, _] = layer.transform.m;
    let mut current_line = None;
    let mut clusters: Vec<_> = layout.clusters.iter().collect();
    clusters.sort_by_key(|cluster| cluster.range.start);
    let mut first = true;
    for (i, cluster) in clusters.iter().enumerate() {
        let text = layer.text.get(cluster.range.clone())?;
        if text.is_empty() {
            continue;
        }
        let span_start = if first { 0 } else { cluster.range.start };
        first = false;
        let next_start = clusters.iter().skip(i + 1).find(|next| !next.range.is_empty()).map_or(layer.text.len(), |next| next.range.start);
        let span_end = next_start.max(cluster.range.end).min(layer.text.len());
        // Only layout whitespace may extend a cluster. Truncated box text must stay raster,
        // rather than exposing unpainted source text in the selection plane.
        if !layer.text.get(span_start..cluster.range.start)?.chars().all(char::is_whitespace)
            || !layer.text.get(cluster.range.end..span_end)?.chars().all(char::is_whitespace)
        {
            return None;
        }
        let span = layer.text.get(span_start..span_end)?;
        let line = layout.lines.get(cluster.line)?;
        if current_line != Some(cluster.line) {
            if current_line.is_some() {
                out.push(ContentOp::EndSelectionLine);
            }
            out.push(ContentOp::BeginSelectionLine);
            current_line = Some(cluster.line);
        }
        let height = (line.ascent + line.descent).max(1.0);
        let width = cluster.advance.abs() / text.chars().count().max(1) as f32;
        if !height.is_finite() || !width.is_finite() {
            return None;
        }
        let x = cluster.x;
        let origin = layer.transform.apply(photocraft_geom::Point::new(f64::from(x), f64::from(line.baseline)));
        if !origin.x.is_finite() || !origin.y.is_finite() {
            return None;
        }
        let [ta, tb, tc, td] = [a * f64::from(width), b * f64::from(width), -c * f64::from(height), -d * f64::from(height)];
        if !ta.is_finite() || !tb.is_finite() || !tc.is_finite() || !td.is_finite() {
            return None;
        }
        let encoded = selection_font.encode(span)?;
        out.push(ContentOp::SelectionText { actual_text: span.to_owned(), encoded, matrix: [ta, tb, tc, td, origin.x, origin.y] });
    }

    if current_line.is_some() {
        out.push(ContentOp::EndSelectionLine);
    }
    Some(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_geom::Affine;

    #[test]
    fn embedding_restrictions_refuse_outlines() {
        let original = photocraft_text::fonts::INTER_REGULAR.as_slice();
        assert!(outlines_permitted(original, 0));
        let count = u16::from_be_bytes(original[4..6].try_into().unwrap()) as usize;
        let record = (0..count).map(|i| 12 + 16 * i).find(|at| &original[*at..*at + 4] == b"OS/2").unwrap();
        let offset = u32::from_be_bytes(original[record + 8..record + 12].try_into().unwrap()) as usize;
        for flags in [0x0002u16, 0x0100, 0x0200] {
            let mut restricted = original.to_vec();
            restricted[offset + 8..offset + 10].copy_from_slice(&flags.to_be_bytes());
            assert!(!outlines_permitted(&restricted, 0), "fsType {flags:#x}");
            let layer = TextLayer { text: "Restricted".into(), ..Default::default() };
            let mut layout = photocraft_text::TextEngine::new().layout(&layer, 72.0);
            for face in &mut layout.faces {
                face.font.data = restricted.clone().into();
                face.font.index = 0;
            }
            let font = super::super::SelectionFont::new(std::iter::once(layer.text.as_str()));
            assert!(layout_ops(&layer, ColorMode::Rgb, &layout, &font).is_none(), "restricted text must remain raster");
        }
        assert!(!outlines_permitted(&[], 0));
    }

    #[test]
    fn text_layer_emits_actualtext_outlines_and_invisible_text() {
        let layer = TextLayer {
            text: "Hello".into(),
            size_pt: 12.0,
            color: photocraft_color::Color::rgb(0.0, 0.5, 1.0),
            transform: Affine::translate(10.0, 20.0),
            ..Default::default()
        };
        let ops = text_ops(&layer, ColorMode::Rgb, 72.0).unwrap();
        assert!(ops.contains("/Span << /ActualText <FEFF0048> >> BDC"));
        assert!(ops.contains("0 0.5 1 rg"));
        assert!(ops.contains(" m\n"));
        assert!(ops.contains(" c\n"));
        assert!(ops.contains("h\nf*\n"));
        assert!(ops.contains("BT\n3 Tr\n"));
        assert!(ops.contains("/FSelect"));
        assert!(ops.contains("> Tj\nEMC\n"));
        assert!(ops.ends_with("ET\n"));
    }

    #[test]
    fn rejects_warp_and_invalid_alpha() {
        let warped = TextLayer { text: "Warped".into(), warp: Some(photocraft_doc::text::TextWarp::default()), ..Default::default() };
        assert!(text_ops(&warped, ColorMode::Rgb, 72.0).is_none());

        let transparent = TextLayer {
            text: "Alpha".into(),
            color: photocraft_color::Color { mode: ColorMode::Rgb, c: [1.0, 0.0, 0.0, 0.0], alpha: 0.5 },
            ..Default::default()
        };
        assert!(text_ops(&transparent, ColorMode::Rgb, 72.0).is_none());
    }

    /// Ensures PDF text export rejects path text shapes and synthetic bold styles that cannot be represented natively.
    #[test]
    fn rejects_path_text_and_synthetic_bold() {
        let on_path = TextLayer {
            text: "Path".into(),
            shape: photocraft_doc::text::TextShape::Path { path: Default::default(), start: 0.0, end: None, flip: false },
            ..Default::default()
        };
        assert!(text_ops(&on_path, ColorMode::Rgb, 72.0).is_none());

        let faux_bold = TextLayer {
            text: "Bold".into(),
            runs: vec![photocraft_doc::text::TextRun { len: 4, style: photocraft_doc::text::CharStyle { faux_bold: true, ..Default::default() } }],
            ..Default::default()
        };
        assert!(text_ops(&faux_bold, ColorMode::Rgb, 72.0).is_none());

        // A face without the requested weight is emboldened by font matching.
        let plain = TextLayer { text: "Bold".into(), ..Default::default() };
        let mut layout = photocraft_text::TextEngine::new().layout(&plain, 72.0);
        assert!(!synthetic_bold(&layout));
        for face in &mut layout.faces {
            face.embolden = true;
        }
        assert!(synthetic_bold(&layout));
    }

    #[test]
    fn decorations_are_emitted_as_rectangles() {
        let layer = TextLayer {
            text: "Underline".into(),
            runs: vec![photocraft_doc::text::TextRun {
                len: 9,
                style: photocraft_doc::text::CharStyle { underline: true, color: photocraft_color::Color::rgb(1.0, 0.0, 0.0), ..Default::default() },
            }],
            ..Default::default()
        };
        let ops = text_ops(&layer, ColorMode::Rgb, 72.0).unwrap();
        assert!(ops.contains("h\nf\n"));
    }

    #[test]
    fn quadratic_cubic_controls_that_overflow_are_refused() {
        use photocraft_text::render::PathEl;
        // Finite endpoints whose difference is not finite: 1e308 - -1e308 overflows.
        let mut out = String::new();
        let overflowing = [PathEl::MoveTo([1e308, 0.0]), PathEl::QuadTo([-1e308, 0.0], [1e308, 0.0])];
        assert!(write_path_elements(&mut out, &overflowing).is_none(), "a non-finite derived control must be refused");
        assert!(!out.contains(" c\n"), "no cubic with a non-finite control is written:\n{out}");

        // A representable quadratic still becomes the equivalent cubic.
        let mut out = String::new();
        let normal = [PathEl::MoveTo([0.0, 0.0]), PathEl::QuadTo([30.0, 60.0], [60.0, 0.0])];
        assert!(write_path_elements(&mut out, &normal).is_some());
        assert!(out.contains("20 40 40 40 60 0 c\n"), "{out}");
    }

    #[test]
    fn overflowing_text_matrix_products_are_refused() {
        // Whitespace shapes no outlines, so the text matrix is the only place the transform
        // reaches the output. Zero the positions: the products overflow, while the cluster
        // origin they are added to stays finite.
        let layer =
            TextLayer { text: " ".into(), size_pt: 100.0, transform: photocraft_geom::Affine { m: [1e308, 1e308, 0.0, 0.0, 0.0, 0.0] }, ..Default::default() };
        let mut layout = photocraft_text::TextEngine::new().layout(&layer, 72.0);
        for cluster in &mut layout.clusters {
            cluster.x = 0.0;
            cluster.advance = cluster.advance.abs().max(2.0);
        }
        for line in &mut layout.lines {
            line.baseline = 0.0;
        }
        let cluster = layout.clusters.first().expect("a whitespace cluster");
        let line = layout.lines.get(cluster.line).expect("a line");
        let origin = layer.transform.apply(photocraft_geom::Point::new(f64::from(cluster.x), f64::from(line.baseline)));
        assert!(origin.x.is_finite() && origin.y.is_finite(), "the fixture's origin stays finite");
        let font = super::super::SelectionFont::new(std::iter::once(layer.text.as_str()));
        assert!(layout_ops(&layer, ColorMode::Rgb, &layout, &font).is_none(), "overflowing Tm coefficients must not be emitted");
        assert!(text_ops(&layer, ColorMode::Rgb, 72.0).is_none());
    }
}
