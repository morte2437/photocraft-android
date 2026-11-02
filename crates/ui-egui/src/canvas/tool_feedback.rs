//! On-canvas tool feedback: the selection tools' intent badge (#170) and outlines that stay
//! visible on any image (#172).
//!
//! Photoshop marks the selection-tool cursor with a small `+` (add), `−` (subtract) or `×`
//! (intersect) reflecting the effective mode: the options bar's mode, overridden by the held
//! modifiers. Selection outlines and drag previews are drawn as black-and-white dashes (marching
//! ants) so they read on light and dark pixels alike.

use egui::{Color32, Pos2, Shape, Stroke, pos2, vec2};

use crate::PhotocraftApp;
use crate::state::Tool;

/// The intent a selection tool's next gesture has, shown as a cursor badge (none for New).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    Add,
    Subtract,
    Intersect,
    /// Four arrows: a press drags the point under the pointer (the Curvature Pen over an anchor).
    Move,
}

impl Badge {
    /// The badge for an engine selection mode name (`replace` has none).
    pub fn from_mode(mode: &str) -> Option<Badge> {
        match mode {
            "add" => Some(Badge::Add),
            "subtract" => Some(Badge::Subtract),
            "intersect" => Some(Badge::Intersect),
            _ => None,
        }
    }
}

/// Tools whose cursor carries the selection-mode badge.
pub fn is_selection_tool(tool: Tool) -> bool {
    matches!(
        tool,
        Tool::RectMarquee
            | Tool::EllipseMarquee
            | Tool::SingleRowMarquee
            | Tool::SingleColumnMarquee
            | Tool::Lasso
            | Tool::PolygonLasso
            | Tool::MagneticLasso
            | Tool::MagicWand
            | Tool::QuickSelection
            | Tool::ObjectSelection
    )
}

/// The effective selection mode of `tool` for the options-bar mode `bar` (0 New, 1 Add,
/// 2 Subtract, 3 Intersect) and the held modifiers: exactly what the tool's gesture will use.
pub fn selection_mode(tool: Tool, bar: u8, m: egui::Modifiers) -> &'static str {
    match tool {
        // Quick Selection always adds (⌥ subtracts), like Photoshop's after the first stroke.
        Tool::QuickSelection => {
            if m.alt {
                "subtract"
            } else {
                "add"
            }
        }
        // Object Selection: ⌥ subtracts, ⇧ adds (see `retouch_ui::finish_object_selection`).
        Tool::ObjectSelection => {
            if m.alt {
                "subtract"
            } else if m.shift {
                "add"
            } else {
                "replace"
            }
        }
        _ => {
            if m.shift && m.alt {
                "intersect"
            } else if m.shift {
                "add"
            } else if m.alt {
                "subtract"
            } else {
                ["replace", "add", "subtract", "intersect"][bar.min(3) as usize]
            }
        }
    }
}

/// [`selection_mode`] for the active document: with nothing selected, ⌥ (or ⇧⌥) has nothing to
/// subtract from or intersect with, so it draws a new selection, as in Photoshop (where ⌥ then
/// only draws a marquee from its centre). An options-bar Subtract or Intersect is kept (#1106).
pub fn document_selection_mode(app: &PhotocraftApp, tool: Tool, m: egui::Modifiers) -> &'static str {
    let mode = selection_mode(tool, app.ui.selection_mode, m);
    let selected = app.session.active().and_then(|st| st.doc.selection.as_ref()).is_some_and(|s| !s.content_bounds().is_empty());
    if m.alt && !selected && matches!(mode, "subtract" | "intersect") && selection_mode(tool, app.ui.selection_mode, egui::Modifiers::NONE) == "replace" {
        "replace"
    } else {
        mode
    }
}

/// The badge the cursor of `tool` shows with modifiers `m` (None for non-selection tools or New;
/// the anchor point tools show theirs).
pub fn badge(app: &PhotocraftApp, tool: Tool, m: egui::Modifiers) -> Option<Badge> {
    // Add / Delete Anchor Point: + or − beside the crosshair, as on Photoshop's pen cursors (⌥
    // swaps them; ⌘ is Direct Selection, which shows none).
    if let Some(t) = crate::anchor_tools::effective(tool, m) {
        return (!m.command).then_some(if t == Tool::AddAnchorPoint { Badge::Add } else { Badge::Subtract });
    }
    // The Curvature Pen over a point: + on the active path's segments (a click adds an anchor
    // there), four arrows on an anchor (a press drags it).
    if let Some(p) = app.hover_doc.filter(|_| tool == Tool::CurvaturePen) {
        return crate::direct_select::curvature_hover(app, p, m)
            .map(|h| if matches!(h, photocraft_vector::edit::Hit::Segment(..)) { Badge::Add } else { Badge::Move });
    }
    // The Pen over the active path with Auto Add/Delete on: what a click would do (#4144).
    if let Some(p) = app.hover_doc.filter(|_| tool == Tool::Pen) {
        return crate::anchor_tools::pen_auto(app, tool, m, p).map(|(t, ..)| if t == Tool::AddAnchorPoint { Badge::Add } else { Badge::Subtract });
    }
    if !is_selection_tool(tool) {
        return None;
    }
    // ⌘ (⌘⌥) drags pixels or the layer instead of combining selections.
    if m.command && app.drag.is_none() {
        return None;
    }
    let mut m = app.drag.as_ref().filter(|d| d.tool == Tool::Lasso && tool == Tool::Lasso).map_or(m, |d| d.modifiers);
    // The Lasso's Alt with nothing selected draws straight segments; it subtracts nothing.
    if matches!(tool, Tool::Lasso | Tool::PolygonLasso) && app.drag.is_none() && app.session.active().is_none_or(|st| st.doc.selection.is_none()) {
        m.alt = false;
    }
    Badge::from_mode(selection_mode(tool, app.ui.selection_mode, m))
}

/// The badge's glyph as line segments around `c` (half-size `h`).
pub fn badge_glyph(b: Badge, c: Pos2, h: f32) -> Vec<[Pos2; 2]> {
    match b {
        Badge::Add => vec![[c - vec2(h, 0.0), c + vec2(h, 0.0)], [c - vec2(0.0, h), c + vec2(0.0, h)]],
        Badge::Subtract => vec![[c - vec2(h, 0.0), c + vec2(h, 0.0)]],
        Badge::Move => {
            let a = h * 0.45;
            let mut v = vec![[c - vec2(h, 0.0), c + vec2(h, 0.0)], [c - vec2(0.0, h), c + vec2(0.0, h)]];
            for d in [vec2(1.0, 0.0), vec2(-1.0, 0.0), vec2(0.0, 1.0), vec2(0.0, -1.0)] {
                let tip = c + d * h;
                let side = vec2(-d.y, d.x) * a;
                v.push([tip, tip - d * a + side]);
                v.push([tip, tip - d * a - side]);
            }
            v
        }
        Badge::Intersect => {
            let d = h * 0.8;
            vec![[c + vec2(-d, -d), c + vec2(d, d)], [c + vec2(-d, d), c + vec2(d, -d)]]
        }
    }
}

/// Draw the badge next to the cursor hotspot `p` (Photoshop puts it below-right of the
/// crosshair; Quick Selection shows it inside the brush circle, at the centre).
pub fn draw_badge(painter: &egui::Painter, p: Pos2, b: Badge, centred: bool) {
    // Right of the system arrow cursor: at 1× the arrow covers (11, 11), so the badge sat under it.
    let c = if centred { p } else { p + vec2(30.0, 11.0) };
    // White glyph with a dark halo: readable on any pixels.
    for (w, col) in [(3.0, Color32::from_black_alpha(200)), (1.2, Color32::WHITE)] {
        for seg in badge_glyph(b, c, 3.5) {
            painter.line_segment(seg, Stroke::new(w, col));
        }
    }
}

/// Draw animated marching ants along `points`: the same ants as a selection's (`crate::ants`),
/// visible on any image, like Photoshop's selection edges.
pub fn draw_ants(painter: &egui::Painter, points: &[Pos2], closed: bool) {
    crate::ants::draw_polyline(painter, points, closed);
}

/// A selection outline still being drawn (Lasso, Polygonal Lasso, Magnetic Lasso): a solid line,
/// as Photoshop draws it until the selection is made (#7495). Marching ants are for selections
/// that exist. A thin dark line on a soft light edge: Photoshop's look on light pixels, and still
/// visible on dark ones.
pub fn solid_path(points: &[Pos2], closed: bool) -> Vec<Shape> {
    let make = |s: Stroke| if closed { Shape::closed_line(points.to_vec(), s) } else { Shape::line(points.to_vec(), s) };
    vec![make(Stroke::new(3.0, Color32::from_white_alpha(140))), make(Stroke::new(1.0, Color32::BLACK))]
}

/// Draw [`solid_path`].
pub fn draw_solid_path(painter: &egui::Painter, points: &[Pos2], closed: bool) {
    painter.extend(solid_path(points, closed));
}

/// Polygon approximating the ellipse inscribed in `r` (enough vertices to look smooth).
pub fn ellipse_points(r: egui::Rect) -> Vec<Pos2> {
    let (c, a, b) = (r.center(), r.width() / 2.0, r.height() / 2.0);
    let n = ((a.abs() + b.abs()) * 0.5).clamp(16.0, 256.0) as usize;
    (0..n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            pos2(c.x + a * t.cos(), c.y + b * t.sin())
        })
        .collect()
}

/// A path preview in `color` with a contrasting dark halo, so a blue shape outline still shows
/// on blue (or any) pixels: the shape tools' drag preview.
pub fn contrast_path(points: Vec<Pos2>, closed: bool, color: Color32) -> Vec<Shape> {
    let make = |pts: Vec<Pos2>, s: Stroke| if closed { Shape::closed_line(pts, s) } else { Shape::line(pts, s) };
    vec![make(points.clone(), Stroke::new(3.0, Color32::from_black_alpha(150))), make(points, Stroke::new(1.0, color))]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strokes(shapes: &[Shape]) -> Vec<Color32> {
        shapes
            .iter()
            .filter_map(|s| match s {
                Shape::LineSegment { stroke, .. } => Some(stroke.color),
                Shape::Path(p) => match p.stroke.color {
                    egui::epaint::ColorMode::Solid(c) => Some(c),
                    _ => None,
                },
                _ => None,
            })
            .collect()
    }

    #[test]
    fn badge_follows_modifiers_and_the_options_bar() {
        let mut app = PhotocraftApp::new(photocraft_engine::Session::new(), Default::default());
        let none = egui::Modifiers::NONE;
        let (shift, alt) = (egui::Modifiers::SHIFT, egui::Modifiers::ALT);
        let both = egui::Modifiers { shift: true, alt: true, ..Default::default() };
        app.run("file.new", serde_json::json!({"width": 40, "height": 30})).unwrap();
        app.run("select.rect", serde_json::json!({"x": 5, "y": 5, "width": 10, "height": 10})).unwrap();
        for t in [Tool::RectMarquee, Tool::EllipseMarquee, Tool::Lasso, Tool::PolygonLasso, Tool::MagicWand] {
            assert_eq!(badge(&app, t, none), None, "{t:?}: New selection has no badge");
            assert_eq!(badge(&app, t, shift), Some(Badge::Add));
            assert_eq!(badge(&app, t, alt), Some(Badge::Subtract));
            assert_eq!(badge(&app, t, both), Some(Badge::Intersect));
        }
        // With nothing selected the Lasso's Alt draws straight segments: no − badge.
        app.run("select.deselect", serde_json::json!({})).unwrap();
        assert_eq!(badge(&app, Tool::Lasso, alt), None);
        assert_eq!(badge(&app, Tool::RectMarquee, alt), Some(Badge::Subtract));
        // The options bar's mode shows without any key held; keys still override it.
        app.ui.selection_mode = 2;
        assert_eq!(badge(&app, Tool::Lasso, none), Some(Badge::Subtract));
        assert_eq!(badge(&app, Tool::Lasso, shift), Some(Badge::Add));
        app.ui.selection_mode = 3;
        assert_eq!(badge(&app, Tool::RectMarquee, none), Some(Badge::Intersect));
        // Smart selection tools.
        assert_eq!(badge(&app, Tool::QuickSelection, none), Some(Badge::Add));
        assert_eq!(badge(&app, Tool::QuickSelection, alt), Some(Badge::Subtract));
        assert_eq!(badge(&app, Tool::ObjectSelection, none), None);
        assert_eq!(badge(&app, Tool::ObjectSelection, shift), Some(Badge::Add));
        // Other tools never show one.
        for t in [Tool::Brush, Tool::Move, Tool::Zoom, Tool::Crop] {
            assert_eq!(badge(&app, t, shift), None);
        }
        // Distinct glyphs: + has two strokes, − one, × two diagonals.
        let c = pos2(10.0, 10.0);
        assert_eq!(badge_glyph(Badge::Add, c, 3.0).len(), 2);
        assert_eq!(badge_glyph(Badge::Subtract, c, 3.0).len(), 1);
        assert!(badge_glyph(Badge::Intersect, c, 3.0).iter().all(|[a, b]| a.x != b.x && a.y != b.y));
    }

    /// #7495: an outline still being drawn is one unbroken line (no dashes, nothing marching),
    /// black over a light edge.
    #[test]
    fn solid_path_is_one_unbroken_dark_line_on_a_light_edge() {
        let pts = [pos2(0.0, 0.0), pos2(40.0, 0.0), pos2(40.0, 30.0)];
        for closed in [false, true] {
            let shapes = solid_path(&pts, closed);
            assert!(shapes.iter().all(|s| matches!(s, Shape::Path(p) if p.closed == closed && p.points.len() == 3)), "{shapes:?}");
            assert_eq!(strokes(&shapes), vec![Color32::from_white_alpha(140), Color32::BLACK], "the dark line on top");
        }
    }

    #[test]
    fn shape_previews_contrast_and_ellipses_close() {
        let pts = [pos2(0.0, 0.0), pos2(40.0, 0.0), pos2(40.0, 30.0)];
        // Shape previews: the accent path over a dark halo.
        let s = contrast_path(pts.to_vec(), true, Color32::from_rgb(0, 120, 255));
        let cols = strokes(&s);
        assert_eq!(cols.len(), 2);
        assert!(cols[0].r() < 10 && cols[0].a() > 100 && cols[1] == Color32::from_rgb(0, 120, 255));
        // Ellipses close smoothly.
        let e = ellipse_points(egui::Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 50.0)));
        assert!(e.len() >= 16 && e.iter().all(|p| p.x >= -0.01 && p.x <= 100.01 && p.y >= -0.01 && p.y <= 50.01));
    }
}
