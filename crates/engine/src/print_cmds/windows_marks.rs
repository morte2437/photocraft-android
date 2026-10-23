//! App-owned print-mark composition in points from the sheet's top-left.
use super::{MARK_GAP, MARK_LEN, Marks, corner_crop_segments};
use craft_print::rgb::{Primitive, Shape};

pub(super) fn primitives(marks: Marks, (x, y, w, h): (f64, f64, f64, f64)) -> Vec<Primitive> {
    let mut out = Vec::with_capacity(24);
    let line = |out: &mut Vec<Primitive>, x0, y0, x1, y1| {
        out.push(Primitive { shape: Shape::Line { from: (x0, y0), to: (x1, y1) }, width: 0.3, color: [0; 3] });
    };
    let (gap, len) = (MARK_GAP, MARK_LEN);
    if marks.corner_crop {
        for ((x0, y0), (x1, y1)) in corner_crop_segments(marks.bleed, (x, y, w, h)) {
            line(&mut out, x0, y0, x1, y1);
        }
    }
    if marks.center_crop {
        let (mx, my) = (x + w / 2.0, y + h / 2.0);
        line(&mut out, mx, y - gap, mx, y - gap - len);
        line(&mut out, mx, y + h + gap, mx, y + h + gap + len);
        line(&mut out, x - gap, my, x - gap - len, my);
        line(&mut out, x + w + gap, my, x + w + gap + len, my);
    }
    if marks.registration {
        let (r, off) = (5.0, gap + len / 2.0);
        for (cx, cy) in [(x + w / 2.0, y - off), (x + w / 2.0, y + h + off), (x - off, y + h / 2.0), (x + w + off, y + h / 2.0)] {
            out.push(Primitive { shape: Shape::Ellipse { bounds: (cx - r, cy - r, 2.0 * r, 2.0 * r) }, width: 0.3, color: [0; 3] });
            line(&mut out, cx - r * 1.6, cy, cx + r * 1.6, cy);
            line(&mut out, cx, cy - r * 1.6, cx, cy + r * 1.6);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn marks_surround_the_final_image_box_in_physical_points() {
        let out = primitives(Marks { corner_crop: true, center_crop: true, registration: true, bleed: 0.0 }, (100.0, 200.0, 144.0, 72.0));
        assert_eq!(out.len(), 24);
        assert_eq!(out[0].shape, Shape::Line { from: (94.0, 200.0), to: (76.0, 200.0) });
        assert_eq!(out[12].shape, Shape::Ellipse { bounds: (167.0, 180.0, 10.0, 10.0) });
        assert!(out.iter().all(|p| p.width == 0.3 && p.color == [0; 3]));
        assert!(primitives(Marks::default(), (0.0, 0.0, 10.0, 10.0)).is_empty());
    }

    #[test]
    fn bleed_moves_corner_crop_marks_onto_the_trim_line_in_top_down_coordinates() {
        let rect = (100.0, 200.0, 144.0, 72.0);
        let out = primitives(Marks { corner_crop: true, bleed: 9.0, ..Marks::default() }, rect);
        assert_eq!(out.len(), 8);
        // Top-left corner: still outside the image, but on the trim line 9 pt in (y down here).
        assert_eq!(out[0].shape, Shape::Line { from: (94.0, 209.0), to: (76.0, 209.0) });
        assert_eq!(out[1].shape, Shape::Line { from: (109.0, 194.0), to: (109.0, 176.0) });
        // Bottom-right corner, mirrored.
        assert_eq!(out[6].shape, Shape::Line { from: (250.0, 263.0), to: (268.0, 263.0) });
        assert_eq!(out[7].shape, Shape::Line { from: (235.0, 278.0), to: (235.0, 296.0) });
    }
}
