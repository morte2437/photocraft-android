//! Image-placeholder geometry. The outline is normalized to the unit square; its affine
//! placement is independent of the image's placement, so resizing a frame never resamples it.
use crate::{Path, ShapeLayer, ShapeStroke};
use photocraft_geom::{Affine, Point, Rect};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Frame {
    pub path: Path,
    pub transform: Affine,
    #[serde(default)]
    pub stroke: Option<ShapeStroke>,
}

impl Frame {
    pub fn outline(&self) -> Path {
        self.path.transform(&self.transform)
    }
    pub fn corners(&self) -> [Point; 4] {
        [Point::new(0.0, 0.0), Point::new(1.0, 0.0), Point::new(1.0, 1.0), Point::new(0.0, 1.0)].map(|p| self.transform.apply(p))
    }
    pub fn bounds(&self) -> Rect {
        let [a, b, c, d] = self.corners();
        let x0 = a.x.min(b.x).min(c.x).min(d.x);
        let y0 = a.y.min(b.y).min(c.y).min(d.y);
        let x1 = a.x.max(b.x).max(c.x).max(d.x);
        let y1 = a.y.max(b.y).max(c.y).max(d.y);
        Rect::new(x0.floor() as i32, y0.floor() as i32, x1.ceil() as i32, y1.ceil() as i32)
    }
    /// Reject invalid metadata before it can enter geometry or rasterization.
    pub fn valid(&self) -> bool {
        self.transform.m.iter().all(|n| n.is_finite() && n.abs() <= 600_000.0)
            && self.transform.inverse().is_some()
            && {
                let [a, b, c, d, _, _] = self.transform.m;
                (0.01..=300_000.0).contains(&a.hypot(b)) && (0.01..=300_000.0).contains(&c.hypot(d))
            }
            && !self.path.is_empty()
            && self.path.subpaths.len() <= 4096
            && self.path.subpaths.iter().fold(0usize, |n, s| n.saturating_add(s.knots.len())) <= 100_000
            && self
                .path
                .subpaths
                .iter()
                .flat_map(|s| &s.knots)
                .flat_map(|k| [k.anchor, k.in_ctrl, k.out_ctrl])
                .all(|p| p.x.is_finite() && p.y.is_finite() && p.x.abs() <= 10.0 && p.y.abs() <= 10.0)
            && self.corners().iter().all(|p| p.x.abs() <= 600_000.0 && p.y.abs() <= 600_000.0)
            && self.stroke.as_ref().is_none_or(|s| s.validate().is_ok() && (0.0..=1000.0).contains(&s.width) && s.opacity.is_finite())
    }
    pub fn stroke_shape(&self) -> Option<ShapeLayer> {
        let stroke = self.stroke.clone()?;
        Some(ShapeLayer { path: self.outline(), fill: None, stroke: Some(stroke), cache: None, psd_raw: None, live: None, density: 1.0, feather: 0.0 })
    }
}
