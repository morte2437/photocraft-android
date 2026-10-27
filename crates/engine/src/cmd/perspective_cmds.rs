//! View › Perspective Ruler: one, two or three vanishing points that painting strokes snap to,
//! for drawing rooms, streets and buildings in perspective.
//!
//! - **1-point:** one vanishing point on the eye level (the horizon). Strokes snap towards it,
//!   or run level or upright.
//! - **2-point:** two vanishing points on the eye level (the line through them). Strokes snap
//!   towards either, or run upright (the vertical edges of a building).
//! - **3-point:** the 2-point ruler plus a third vanishing point above or below the eye level,
//!   which the upright edges converge on instead.
//!
//! A stroke decides where it goes from its first few points (whichever direction it starts
//! closest to), then stays on that line. The ruler is per document view state (like painting
//! symmetry): it is not part of the image and isn't saved with it.

use crate::{CommandSpec, EngineError, Result, Session};
use serde_json::{Value, json};

/// The farthest a vanishing point may be from the canvas, in canvas sizes. Points much farther
/// away draw parallel lines anyway; this keeps the arithmetic finite.
const MAX_DISTANCE: f64 = 1.0e6;

/// A document's perspective ruler.
#[derive(Clone, Debug, PartialEq)]
pub struct PerspectiveRuler {
    /// The vanishing points (1 to 3). The first two lie on the eye level; a third is the
    /// vertical vanishing point.
    pub points: Vec<[f64; 2]>,
    /// Painting strokes snap to the ruler.
    pub snap: bool,
    /// The ruler's lines are drawn over the canvas.
    pub visible: bool,
}

/// A direction a stroke can snap to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Snap {
    /// Towards (or away from) a vanishing point.
    Toward([f64; 2]),
    /// A fixed direction (unit vector): level or upright lines where no vanishing point governs.
    Fixed([f64; 2]),
}

impl PerspectiveRuler {
    /// The default ruler of `kind` (1, 2 or 3 points) for a `w` × `h` canvas: the eye level a
    /// little above the middle, two-point vanishing points near the sides, the third below.
    pub fn default_for(kind: usize, w: f64, h: f64) -> Option<Self> {
        let eye = h * 0.45;
        let points = match kind {
            1 => vec![[w * 0.5, eye]],
            2 => vec![[w * 0.1, eye], [w * 0.9, eye]],
            3 => vec![[w * 0.1, eye], [w * 0.9, eye], [w * 0.5, h * 1.6]],
            _ => return None,
        };
        Some(Self { points, snap: true, visible: true })
    }

    /// 1, 2 or 3.
    pub fn kind(&self) -> usize {
        self.points.len()
    }

    /// The eye level (horizon) as a point and a unit direction: through the first two vanishing
    /// points, or level through a single one.
    pub fn eye_level(&self) -> Option<([f64; 2], [f64; 2])> {
        let a = *self.points.first()?;
        let dir = match self.points.get(1) {
            Some(b) => unit([b[0] - a[0], b[1] - a[1]]).unwrap_or([1.0, 0.0]),
            None => [1.0, 0.0],
        };
        Some((a, dir))
    }

    /// The directions a stroke can snap to.
    pub fn snaps(&self) -> Vec<Snap> {
        let mut out: Vec<Snap> = self.points.iter().map(|p| Snap::Toward(*p)).collect();
        // Upright lines: towards the third point when there is one, else square to the eye
        // level (vertical edges stay parallel in one- and two-point perspective).
        if self.points.len() < 3
            && let Some((_, d)) = self.eye_level()
        {
            out.push(Snap::Fixed([-d[1], d[0]]));
        }
        // One-point perspective keeps widths level: lines along the eye level.
        if self.points.len() == 1 {
            out.push(Snap::Fixed([1.0, 0.0]));
        }
        out
    }

    /// The unit direction of the snap line through `origin` that best matches a stroke heading
    /// from `origin` to `p`; None when `p` hasn't left `origin` or nothing applies.
    pub fn direction(&self, origin: [f64; 2], p: [f64; 2]) -> Option<[f64; 2]> {
        let heading = unit([p[0] - origin[0], p[1] - origin[1]])?;
        self.snaps()
            .into_iter()
            .filter_map(|s| match s {
                Snap::Toward(vp) => unit([vp[0] - origin[0], vp[1] - origin[1]]),
                Snap::Fixed(d) => Some(d),
            })
            .max_by(|a, b| cos(heading, *a).abs().total_cmp(&cos(heading, *b).abs()))
    }

    fn validate(&self, w: f64, h: f64) -> std::result::Result<(), String> {
        if !(1..=3).contains(&self.points.len()) {
            return Err("a perspective ruler has 1, 2 or 3 vanishing points".into());
        }
        let limit = MAX_DISTANCE * w.max(h).max(1.0);
        if self.points.iter().flatten().any(|v| !v.is_finite() || v.abs() > limit) {
            return Err("vanishing points must be finite and near the canvas".into());
        }
        Ok(())
    }

    pub fn inspect(&self) -> Value {
        json!({"type": self.kind(), "vanishingPoints": self.points, "snap": self.snap, "visible": self.visible})
    }
}

fn unit(v: [f64; 2]) -> Option<[f64; 2]> {
    let l = v[0].hypot(v[1]);
    (l.is_finite() && l > 1e-9).then(|| [v[0] / l, v[1] / l])
}

fn cos(a: [f64; 2], b: [f64; 2]) -> f64 {
    a[0] * b[0] + a[1] * b[1]
}

fn doc_size(s: &Session) -> Result<(f64, f64)> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    Ok((f64::from(st.doc.size.width), f64::from(st.doc.size.height)))
}

fn create(s: &mut Session, kind: usize) -> Result<Value> {
    let (w, h) = doc_size(s)?;
    let ruler = PerspectiveRuler::default_for(kind, w, h).ok_or_else(|| EngineError::Other("1, 2 or 3 points".into()))?;
    let out = ruler.inspect();
    s.active_mut().ok_or(EngineError::NoDocument)?.perspective = Some(ruler);
    Ok(out)
}

fn set(s: &mut Session, p: &Value) -> Result<Value> {
    let (w, h) = doc_size(s)?;
    let bad = |m: &str| EngineError::BadParams { cmd: "view.perspectiveRuler.set".into(), msg: m.into() };
    let mut ruler = s.active().ok_or(EngineError::NoDocument)?.perspective.clone().ok_or_else(|| bad("no perspective ruler (View › Perspective Ruler)"))?;
    if let Some(points) = p.get("vanishingPoints") {
        let list = points.as_array().ok_or_else(|| bad("vanishingPoints: [[x,y], …]"))?;
        ruler.points = list
            .iter()
            .map(|v| match v.as_array().map(|a| a.as_slice()) {
                Some([x, y]) => match (x.as_f64(), y.as_f64()) {
                    (Some(x), Some(y)) => Ok([x, y]),
                    _ => Err(bad("each vanishing point is [x, y]")),
                },
                _ => Err(bad("each vanishing point is [x, y]")),
            })
            .collect::<Result<_>>()?;
    }
    if let Some(v) = p.get("snap") {
        ruler.snap = v.as_bool().ok_or_else(|| bad("snap: true or false"))?;
    }
    if let Some(v) = p.get("visible") {
        ruler.visible = v.as_bool().ok_or_else(|| bad("visible: true or false"))?;
    }
    ruler.validate(w, h).map_err(|m| bad(&m))?;
    let out = ruler.inspect();
    s.active_mut().ok_or(EngineError::NoDocument)?.perspective = Some(ruler);
    Ok(out)
}

fn toggle(s: &mut Session, f: fn(&mut PerspectiveRuler) -> &mut bool) -> Result<Value> {
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let ruler = st.perspective.as_mut().ok_or_else(|| EngineError::Other("no perspective ruler".into()))?;
    let b = f(ruler);
    *b = !*b;
    Ok(ruler.inspect())
}

fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or_else(|| "no document open".into())
}

fn has_ruler(s: &Session) -> std::result::Result<(), String> {
    s.active().filter(|st| st.perspective.is_some()).map(|_| ()).ok_or_else(|| "no perspective ruler".into())
}

const MENU: &[&str] = &["View", "Perspective Ruler"];

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "view.perspectiveRuler.onePoint",
            label: "1-Point Perspective",
            menu: MENU,
            shortcut: None,
            params: "{} — a ruler with one vanishing point, replacing the current one",
            enabled: has_doc,
            run: |s, _| create(s, 1),
            journal: false,
        },
        CommandSpec {
            id: "view.perspectiveRuler.twoPoint",
            label: "2-Point Perspective",
            menu: MENU,
            shortcut: None,
            params: "{} — two vanishing points on the eye level",
            enabled: has_doc,
            run: |s, _| create(s, 2),
            journal: false,
        },
        CommandSpec {
            id: "view.perspectiveRuler.threePoint",
            label: "3-Point Perspective",
            menu: MENU,
            shortcut: None,
            params: "{} — two on the eye level and a third for upright lines",
            enabled: has_doc,
            run: |s, _| create(s, 3),
            journal: false,
        },
        CommandSpec {
            id: "view.perspectiveRuler.snap",
            label: "Snap to Perspective",
            menu: MENU,
            shortcut: None,
            params: "{} — toggle: painting strokes snap to the ruler",
            enabled: has_ruler,
            run: |s, _| toggle(s, |r| &mut r.snap),
            journal: false,
        },
        CommandSpec {
            id: "view.perspectiveRuler.show",
            label: "Show Perspective Ruler",
            menu: MENU,
            shortcut: None,
            params: "{} — toggle: draw the ruler's lines and handles",
            enabled: has_ruler,
            run: |s, _| toggle(s, |r| &mut r.visible),
            journal: false,
        },
        CommandSpec {
            id: "view.perspectiveRuler.clear",
            label: "Clear Perspective Ruler",
            menu: MENU,
            shortcut: None,
            params: "{}",
            enabled: has_ruler,
            run: |s, _| {
                s.active_mut().ok_or(EngineError::NoDocument)?.perspective = None;
                Ok(json!({}))
            },
            journal: false,
        },
        CommandSpec {
            id: "view.perspectiveRuler.set",
            label: "Set Perspective Ruler",
            menu: &[],
            shortcut: None,
            params: r#"{"vanishingPoints":[[x,y],…]? (1–3; first two on the eye level, third for upright lines),"snap":bool?,"visible":bool?}"#,
            enabled: has_ruler,
            run: set,
            journal: false,
        },
        CommandSpec {
            id: "view.perspectiveRuler.info",
            label: "Perspective Ruler Info",
            menu: &[],
            shortcut: None,
            params: "{} → {type, vanishingPoints, snap, visible} or null",
            enabled: has_doc,
            run: |s, _| Ok(s.active().ok_or(EngineError::NoDocument)?.perspective.as_ref().map_or(Value::Null, PerspectiveRuler::inspect)),
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: [f64; 2], b: [f64; 2]) -> bool {
        (a[0] - b[0]).abs() < 1e-9 && (a[1] - b[1]).abs() < 1e-9
    }

    #[test]
    fn strokes_snap_to_the_closest_perspective_direction() {
        // Two-point: vanishing points left and right on the eye level at y = 100.
        let r = PerspectiveRuler { points: vec![[0.0, 100.0], [1000.0, 100.0]], snap: true, visible: true };
        let o = [500.0, 300.0];
        // Heading up and to the right: towards the right vanishing point.
        let d = r.direction(o, [560.0, 280.0]).unwrap();
        assert!(close(d, unit([500.0, -200.0]).unwrap()), "{d:?}");
        // Heading up and to the left: the left one.
        let d = r.direction(o, [440.0, 280.0]).unwrap();
        assert!(close(d, unit([-500.0, -200.0]).unwrap()));
        // Heading nearly straight down: upright (square to the eye level).
        let d = r.direction(o, [503.0, 400.0]).unwrap();
        assert!(d[0].abs() < 1e-9 && d[1].abs() > 0.999, "{d:?}");
        // Not moved yet: nothing.
        assert_eq!(r.direction(o, o), None);
    }

    #[test]
    fn one_point_keeps_level_and_upright_lines_and_three_point_converges_uprights() {
        let one = PerspectiveRuler { points: vec![[500.0, 200.0]], snap: true, visible: true };
        let d = one.direction([100.0, 600.0], [200.0, 602.0]).unwrap();
        assert!(close(d, [1.0, 0.0]), "level: {d:?}");
        let d = one.direction([100.0, 600.0], [150.0, 560.0]).unwrap();
        assert!(close(d, unit([400.0, -400.0]).unwrap()), "towards the point: {d:?}");
        let three = PerspectiveRuler { points: vec![[0.0, 100.0], [1000.0, 100.0], [500.0, 3000.0]], snap: true, visible: true };
        let d = three.direction([300.0, 500.0], [302.0, 600.0]).unwrap();
        assert!(close(d, unit([200.0, 2500.0]).unwrap()), "uprights lean to the third point: {d:?}");
        assert_eq!(three.snaps().len(), 3, "no fixed directions in three-point");
    }

    #[test]
    fn commands_create_edit_toggle_and_refuse_bad_input() {
        let mut s = Session::new();
        assert!(s.execute("view.perspectiveRuler.twoPoint", json!({})).is_err(), "needs a document");
        s.execute("file.new", json!({"width": 1000, "height": 800})).unwrap();
        let r = s.execute("view.perspectiveRuler.twoPoint", json!({})).unwrap();
        assert_eq!(r["type"], 2);
        assert_eq!(s.active().unwrap().perspective.as_ref().unwrap().points, vec![[100.0, 360.0], [900.0, 360.0]]);
        s.execute("view.perspectiveRuler.set", json!({"vanishingPoints": [[0, 50], [1000, 70], [500, 4000]]})).unwrap();
        assert_eq!(s.active().unwrap().perspective.as_ref().unwrap().kind(), 3);
        s.execute("view.perspectiveRuler.snap", json!({})).unwrap();
        assert!(!s.active().unwrap().perspective.as_ref().unwrap().snap);
        for bad in [json!({"vanishingPoints": []}), json!({"vanishingPoints": [[1, 2], [3, 4], [5, 6], [7, 8]]}), json!({"vanishingPoints": [[1e300, 0]]}), json!({"vanishingPoints": [[1]]}), json!({"snap": 3})] {
            assert!(s.execute("view.perspectiveRuler.set", bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(s.active().unwrap().perspective.as_ref().unwrap().kind(), 3, "unchanged by bad input");
        assert_eq!(s.execute("view.perspectiveRuler.info", json!({})).unwrap()["snap"], false);
        s.execute("view.perspectiveRuler.clear", json!({})).unwrap();
        assert!(s.active().unwrap().perspective.is_none());
        assert!(s.execute("view.perspectiveRuler.snap", json!({})).is_err());
        assert_eq!(s.execute("view.perspectiveRuler.info", json!({})).unwrap(), Value::Null);
    }
}
