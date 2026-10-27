//! Filter › Vanishing Point: perspective planes (drawn, or torn off an edge of another plane),
//! pasting an image onto a plane and the perspective clone stamp, as one replayable command
//! ([`photocraft_algo::vanishing`]). The edit lands on the active pixel layer, or on a new layer
//! with `"newLayer": true` (Photoshop's advice for non-destructive Vanishing Point work).

use photocraft_algo::vanishing::{Edge, PlaneCheck, Scene, VpPlane, clone_dabs, paste};
use photocraft_doc::{Document, Layer, LayerContent, LayerId};
use photocraft_geom::Rect;
use photocraft_raster::Surface;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};

pub const VP: &str = "filter.vanishingPoint";
/// Most planes a scene holds.
pub const MAX_PLANES: usize = 64;
/// Most clone strokes, and dabs over all of them, one run paints.
pub const MAX_CLONE_STROKES: usize = 4096;
pub const MAX_CLONE_DABS: usize = 200_000;

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: VP.into(), msg: msg.into() }
}

fn pt(v: &Value) -> Option<[f64; 2]> {
    Some([v.get(0)?.as_f64()?, v.get(1)?.as_f64()?])
}

/// Builds the scene from `"planes"`.
pub fn scene(p: &Value, canvas: Rect) -> Result<Scene> {
    let planes = p
        .get("planes")
        .and_then(Value::as_array)
        .filter(|a| !a.is_empty())
        .ok_or_else(|| bad("pass `planes`: [{\"corners\":[[x,y]×4]}, {\"from\":0,\"edge\":\"top\",\"angle\":90}]"))?;
    if planes.len() > MAX_PLANES {
        return Err(bad(format!("at most {MAX_PLANES} planes")));
    }
    let center = [(canvas.x0 + canvas.x1) as f64 / 2.0, (canvas.y0 + canvas.y1) as f64 / 2.0];
    let corners = |v: &Value| -> Option<[[f64; 2]; 4]> {
        let a = v.get("corners")?.as_array()?;
        Some([pt(a.first()?)?, pt(a.get(1)?)?, pt(a.get(2)?)?, pt(a.get(3)?)?])
    };
    // A drawn quad must be convex (Photoshop's red plane): a bow-tie or dented quad has no
    // perspective to paste or clone along.
    let drawn = |i: usize, c: [[f64; 2]; 4]| -> Result<VpPlane> {
        let pl = VpPlane { corners: c };
        if pl.check(center) == PlaneCheck::Invalid {
            return Err(bad(format!("plane {i}: the four corners must form a convex quad")));
        }
        Ok(pl)
    };
    let first = planes.first().and_then(corners).ok_or_else(|| bad("the first plane needs four `corners`"))?;
    let first = drawn(0, first)?;
    let focal = p.get("focalLength").and_then(Value::as_f64).filter(|f| *f > 0.0);
    let mut sc = Scene::new(first, center, focal, canvas.width().max(canvas.height()) as f64);
    for (i, pl) in planes.iter().enumerate().skip(1) {
        if let Some(c) = corners(pl) {
            sc.planes.push(drawn(i, c)?);
            continue;
        }
        let from = pl
            .get("from")
            .and_then(Value::as_u64)
            .map(|v| v as usize)
            .filter(|f| *f < sc.planes.len())
            .ok_or_else(|| bad(format!("plane {i}: `corners` or `from` (an earlier plane)")))?;
        let edge = match pl.get("edge").and_then(Value::as_str).unwrap_or("top") {
            "top" => Edge::Top,
            "right" => Edge::Right,
            "bottom" => Edge::Bottom,
            "left" => Edge::Left,
            e => return Err(bad(format!("plane {i}: unknown edge `{e}`"))),
        };
        let angle = pl.get("angle").and_then(Value::as_f64).unwrap_or(90.0);
        let depth = pl.get("depth").and_then(Value::as_f64).unwrap_or(1.0);
        if !angle.is_finite() || !(0.0..=360.0).contains(&angle) {
            return Err(bad(format!("plane {i}: `angle` must be 0..360 degrees")));
        }
        if !depth.is_finite() || depth <= 0.0 || depth > 1000.0 {
            return Err(bad(format!("plane {i}: `depth` must be above 0 and at most 1000")));
        }
        sc.tear_off(from, edge, angle, depth).ok_or_else(|| bad(format!("plane {i} would lie behind the camera")))?;
    }
    if sc.planes.iter().any(|p| p.homography().is_none()) {
        return Err(bad("a plane is degenerate"));
    }
    Ok(sc)
}

/// One stroke of `"clone"`.
#[derive(Clone, Debug, PartialEq)]
pub struct CloneSpec {
    pub source: [f64; 2],
    pub points: Vec<[f64; 2]>,
    pub size: f64,
    pub hardness: f64,
    pub opacity: f64,
}

/// Reads `"clone"`: finite points, at most [`MAX_CLONE_STROKES`] strokes and
/// [`MAX_CLONE_DABS`] dabs in all.
pub fn clone_specs(p: &Value) -> Result<Vec<CloneSpec>> {
    let Some(v) = p.get("clone").filter(|v| !v.is_null()) else { return Ok(Vec::new()) };
    let strokes = v.as_array().ok_or_else(|| bad("`clone` is a list of strokes"))?;
    if strokes.len() > MAX_CLONE_STROKES {
        return Err(bad(format!("at most {MAX_CLONE_STROKES} clone strokes")));
    }
    let finite = |q: [f64; 2]| q.iter().all(|v| v.is_finite() && v.abs() < 1e7);
    let num = |c: &Value, k: &str, default: f64, lo: f64, hi: f64| -> Result<f64> {
        match c.get(k) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v.as_f64().filter(|x| x.is_finite()).map(|x| x.clamp(lo, hi)).ok_or_else(|| bad(format!("clone `{k}` must be a number"))),
        }
    };
    let mut dabs = 0usize;
    let mut out = Vec::with_capacity(strokes.len());
    for (k, c) in strokes.iter().enumerate() {
        let source = c.get("source").and_then(pt).filter(|q| finite(*q)).ok_or_else(|| bad(format!("clone {k}: `source` [x,y]")))?;
        let raw = c.get("points").and_then(Value::as_array).ok_or_else(|| bad(format!("clone {k}: `points` [[x,y], …]")))?;
        dabs = dabs.saturating_add(raw.len());
        if dabs > MAX_CLONE_DABS {
            return Err(bad(format!("at most {MAX_CLONE_DABS} clone points in all")));
        }
        let points =
            raw.iter().map(|q| pt(q).filter(|q| finite(*q))).collect::<Option<Vec<_>>>().ok_or_else(|| bad(format!("clone {k}: every point is [x,y]")))?;
        out.push(CloneSpec {
            source,
            points,
            size: num(c, "size", 40.0, 1.0, 2000.0)?,
            hardness: num(c, "hardness", 50.0, 0.0, 100.0)? / 100.0,
            opacity: num(c, "opacity", 100.0, 0.0, 100.0)? / 100.0,
        });
    }
    Ok(out)
}

/// Paints clone stroke `c` on `surf` inside `canvas`, or, with `work` (the image it samples), on
/// `work` and copies the pixels it changed to `surf`. Returns the dabs painted.
pub fn clone_into(sc: &Scene, surf: &mut Surface, work: Option<&mut Surface>, c: &CloneSpec, canvas: Rect) -> usize {
    let Some(first) = c.points.first() else { return 0 };
    let Some(work) = work else {
        return clone_dabs(sc, surf, c.source, *first, &c.points, c.size, c.hardness, c.opacity as f32, Some(canvas)).0;
    };
    let before = work.clone();
    // The dabs are cut to the canvas before they touch `work` or `surf`: points up to 1e7 px off
    // it would otherwise allocate tiles there (and reading their union back, without bound).
    let (dabs, area) = clone_dabs(sc, work, c.source, *first, &c.points, c.size, c.hardness, c.opacity as f32, Some(canvas));
    copy_changed(surf, &before, work, area.intersect(&canvas));
    dabs
}

/// Copies to `dst` the pixels of `area` where `after` differs from `before` (same format).
pub fn copy_changed(dst: &mut Surface, before: &Surface, after: &Surface, area: Rect) {
    if area.is_empty() {
        return;
    }
    let n = dst.format().channels();
    let (nw, ow, mut cur) = (after.read_region(area), before.read_region(area), dst.read_region(area));
    for ((c, a), b) in cur.chunks_exact_mut(n).zip(nw.chunks_exact(n)).zip(ow.chunks_exact(n)) {
        if a != b {
            c.copy_from_slice(a);
        }
    }
    dst.write_region(area, &cur);
}

/// The image a `"newLayer"` run's clone strokes sample: the visible document.
pub fn visible_image(doc: &Document) -> Surface {
    crate::file_cmds::flattened(doc, doc.pixel_format())
}

fn has_layer(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    match d.active_layer.and_then(|id| d.doc.layer(id)).map(|l| &l.content) {
        Some(LayerContent::Raster(_)) => Ok(()),
        Some(other) => Err(format!("Vanishing Point needs a pixel layer (active layer is {} {} layer)", other.article(), other.kind_name())),
        None => Err("no active layer".into()),
    }
}

/// Pastes `src` (content in `rect`) onto plane `plane` of `sc` over `surf`, as the command's
/// `"paste"` does. Returns the area the paste covers.
pub fn paste_onto(sc: &Scene, surf: &mut Surface, plane: usize, src: &Surface, rect: Rect, at: [f64; 2], width: f64) -> Result<Rect> {
    let Some(out) = paste(sc, plane, src, rect, at, width)? else { return Ok(Rect::EMPTY) };
    over(surf, &out);
    Ok(out.content_bounds())
}

/// Composites `top` over `dst` (premultiplied source-over), same format.
pub fn over(dst: &mut Surface, top: &Surface) {
    let b = top.content_bounds();
    if b.is_empty() {
        return;
    }
    let top = if top.format() == dst.format() { top.clone() } else { top.convert(dst.format()) };
    let fmt = dst.format();
    let n = fmt.channels();
    let (t, mut d) = (top.read_region(b), dst.read_region(b));
    for (tp, dp) in t.chunks_exact(n).zip(d.chunks_exact_mut(n)) {
        if !fmt.alpha {
            dp.copy_from_slice(tp);
            continue;
        }
        let (ta, da) = (tp[n - 1], dp[n - 1]);
        let oa = ta + da * (1.0 - ta);
        for c in 0..n - 1 {
            dp[c] = if oa > 0.0 { (tp[c] * ta + dp[c] * da * (1.0 - ta)) / oa } else { 0.0 };
        }
        dp[n - 1] = oa;
    }
    dst.write_region(b, &d);
}

fn vanishing_point(s: &mut Session, p: &Value) -> Result<Value> {
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let canvas = st.doc.bounds();
    let sc = scene(p, canvas)?;
    // Paste sources resolved up front (layer pixels or the clipboard).
    let mut pastes: Vec<(usize, Surface, Rect, [f64; 2], f64)> = Vec::new();
    for (k, item) in p.get("paste").and_then(Value::as_array).cloned().unwrap_or_default().iter().enumerate() {
        let plane = item.get("plane").and_then(Value::as_u64).unwrap_or(0) as usize;
        if plane >= sc.planes.len() {
            return Err(bad(format!("paste {k}: no plane {plane}")));
        }
        let (surf, rect) = match item.get("layer").and_then(Value::as_u64) {
            Some(l) => {
                let sf = st.doc.layer(LayerId(l)).and_then(Layer::surface).ok_or(EngineError::NoLayer(LayerId(l)))?;
                (sf.clone(), sf.content_bounds())
            }
            None => {
                let clip = s.clipboard.as_ref().ok_or_else(|| bad(format!("paste {k}: nothing on the clipboard (or pass `layer`)")))?;
                (clip.surface.clone(), clip.surface.content_bounds())
            }
        };
        let at = item.get("at").and_then(pt).unwrap_or([0.25, 0.25]);
        let width = item.get("width").and_then(Value::as_f64).unwrap_or(0.5).clamp(0.01, 10.0);
        pastes.push((plane, surf, rect, at, width));
    }
    let clones = clone_specs(p)?;
    let new_layer = p.get("newLayer").and_then(Value::as_bool).unwrap_or(false);
    // The layer the mode was opened on (the UI passes it: other layers can be picked meanwhile).
    let layer = crate::commands::explicit_layer(s, p)?;
    let mut dabs = 0usize;
    let target = s.edit("Vanishing Point", |doc: &mut Document, active| {
        if let Some(l) = layer
            && doc.layer(l).is_none()
        {
            return Err(EngineError::NoLayer(l));
        }
        let on = layer.or(*active);
        let id = if new_layer {
            let l = Layer::raster(doc.next_layer_name("Vanishing Point"), doc.pixel_format());
            let nid = doc.insert_above(on, l);
            *active = Some(nid);
            nid
        } else {
            // The painted layer becomes active, so Fade and a colour-channel target follow it.
            let id = on.ok_or_else(|| EngineError::Other("no active layer".into()))?;
            *active = Some(id);
            id
        };
        // A new layer clones from the visible image; otherwise from the layer itself.
        let base: Option<Surface> = new_layer.then(|| visible_image(doc));
        let locks = doc.effective_locks(id);
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if locks.all || locks.pixels {
            return Err(EngineError::Other(format!("layer \"{}\" is locked", l.name)));
        }
        let surf = l.surface_mut().ok_or_else(|| EngineError::Other("Vanishing Point needs a pixel layer".into()))?;
        for (plane, src, rect, at, width) in &pastes {
            paste_onto(&sc, surf, *plane, src, *rect, *at, *width)?;
        }
        // A new layer samples the visible image (merged with what's been pasted) and takes only
        // the pixels the strokes change; otherwise the strokes paint the layer itself.
        let mut work = base.map(|mut w| {
            over(&mut w, surf);
            w
        });
        for c in &clones {
            dabs += clone_into(&sc, surf, work.as_mut(), c, canvas);
        }
        surf.prune();
        Ok(id)
    })?;
    let planes: Vec<Value> = sc
        .planes
        .iter()
        .enumerate()
        .map(|(i, pl)| {
            let (lu, lv) = sc.lifted(i).lengths();
            json!({"corners": pl.corners, "aspect": lu / lv})
        })
        .collect();
    Ok(json!({"layer": target.0, "focal": sc.focal, "planes": planes, "pasted": pastes.len(), "dabs": dabs}))
}

pub fn specs() -> Vec<CommandSpec> {
    vec![CommandSpec {
        id: VP,
        label: "Vanishing Point…",
        menu: &["Filter"],
        shortcut: Some("Cmd+Alt+V"),
        params: r##"{"planes":[{"corners":[[x,y]×4]} | {"from":plane,"edge":"top|right|bottom|left","angle":deg=90,"depth":1}],"focalLength":px=0,"paste":[{"plane":0,"layer":id? (else the clipboard),"at":[u,v]=[0.25,0.25],"width":0..1=0.5}],"clone":[{"source":[x,y],"points":[[x,y]],"size":px=40,"hardness":0..100=50,"opacity":0..100=100}],"newLayer":bool=false,"layer":id|name? (else the active layer)}"##,
        enabled: has_layer,
        run: vanishing_point,
        journal: true,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn planes_paste_and_clone() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 400, "height": 300})).unwrap();
        s.execute("layer.new.layer", json!({"name": "floor"})).unwrap();
        s.execute("edit.fill", json!({"color": "#404040"})).unwrap();
        let floor = s.active().unwrap().active_layer.unwrap();
        // A trapezoid "floor" receding to the top.
        let planes = json!([{"corners": [[120, 120], [280, 120], [360, 280], [40, 280]]}, {"from": 0, "edge": "top", "angle": 90}]);
        s.execute("layer.new.layer", json!({"name": "sticker"})).unwrap();
        s.edit("sticker", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(0, 0, 40, 20), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let sticker = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.select", json!({"layer": floor.0})).unwrap();
        let r = s.execute(VP, json!({"planes": planes, "paste": [{"plane": 0, "layer": sticker.0, "at": [0.3, 0.4], "width": 0.4}]})).unwrap();
        assert_eq!(r["planes"].as_array().unwrap().len(), 2);
        assert!(r["focal"].as_f64().unwrap() > 50.0, "{r}");
        let l = s.active().unwrap().doc.layer(floor).unwrap().surface().unwrap().clone();
        // The paste covers the plane's middle and not its far corner.
        let h = VpPlane { corners: [[120.0, 120.0], [280.0, 120.0], [360.0, 280.0], [40.0, 280.0]] }.homography().unwrap();
        let (x, y) = h.apply(0.5, 0.41);
        assert!(l.rgba(x as i32, y as i32)[0] > 0.9, "{:?}", l.rgba(x as i32, y as i32));
        assert!(l.rgba(130, 125)[0] < 0.5);
        s.undo();
        // Clone: copy the bright stripe painted near the bottom to the top of the floor.
        s.edit("stripe", |doc, _| {
            doc.layer_mut(floor).unwrap().surface_mut().unwrap().fill_rect(Rect::new(180, 250, 220, 265), &[0.0, 1.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        s.execute("layer.select", json!({"layer": floor.0})).unwrap();
        let r = s
            .execute(VP, json!({"planes": planes, "clone": [{"source": [200, 257], "points": [[200, 160], [205, 160]], "size": 12, "hardness": 80}]}))
            .unwrap();
        assert!(r["dabs"].as_u64().unwrap() >= 1, "{r}");
        let l = s.active().unwrap().doc.layer(floor).unwrap().surface().unwrap();
        assert!(l.rgba(200, 160)[1] > 0.8, "{:?}", l.rgba(200, 160));
        assert!(s.execute(VP, json!({})).is_err());
        assert!(s.execute(VP, json!({"planes": [{"corners": [[0, 0], [1, 1]]}]})).is_err());
        let r = s.execute(VP, json!({"planes": planes, "newLayer": true})).unwrap();
        assert_ne!(r["layer"].as_u64().unwrap(), floor.0);
    }

    #[test]
    fn rejects_planes_that_are_not_convex() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 200, "depth": 16})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let bowtie = json!([[20, 20], [180, 180], [180, 20], [20, 180]]);
        let dent = json!([[20, 20], [180, 20], [60, 60], [20, 180]]);
        let square = json!([[20, 20], [180, 20], [180, 180], [20, 180]]);
        for planes in [json!([{"corners": bowtie}]), json!([{"corners": square}, {"corners": dent}])] {
            let e = s.execute(VP, json!({"planes": planes})).unwrap_err().to_string();
            assert!(e.contains("convex"), "{e}");
        }
        assert!(s.execute(VP, json!({"planes": [{"corners": square}]})).is_ok());
    }

    #[test]
    fn rejects_hostile_clone_and_tear_off_params() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 200, "depth": 32})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        let floor = json!({"corners": [[60, 60], [140, 60], [190, 190], [10, 190]]});
        let before = s.active().unwrap().doc.clone();
        let steps = s.active().unwrap().history.past_len();
        for torn in [json!({"from": 0, "angle": 1e9}), json!({"from": 0, "depth": -1}), json!({"from": 0, "depth": 1e300})] {
            let e = s.execute(VP, json!({"planes": [floor, torn]})).unwrap_err().to_string();
            assert!(e.contains("plane 1"), "{e}");
            assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
            assert_eq!(s.active().unwrap().history.past_len(), steps);
        }
        let many: Vec<[f64; 2]> = vec![[100.0, 150.0]; MAX_CLONE_DABS + 1];
        for clone in [
            json!([{"points": [[100, 150]]}]),
            json!([{"source": [100, 180], "points": [[100, 150], "x"]}]),
            json!([{"source": [100, 180], "points": [[100, 150]], "size": "big"}]),
            json!([{"source": [100, 180], "points": many}]),
            json!({"source": [100, 180]}),
        ] {
            let label: String = clone.to_string().chars().take(80).collect();
            assert!(s.execute(VP, json!({"planes": [floor], "clone": clone, "newLayer": true})).is_err(), "{label}");
            assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
            assert_eq!(s.active().unwrap().history.past_len(), steps);
        }
        let planes: Vec<Value> = std::iter::repeat_n(floor.clone(), MAX_PLANES + 1).collect();
        assert!(s.execute(VP, json!({"planes": planes})).is_err());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
        assert_eq!(s.active().unwrap().history.past_len(), steps);
        // A clone on a torn-off wall, at 32 bits, sampling the floor.
        let r = s
            .execute(VP, json!({"planes": [floor, {"from": 0, "edge": "top", "angle": 90, "depth": 0.5}], "clone": [{"source": [100, 150], "points": [[100, 50], [104, 50]], "size": 8}]}))
            .unwrap();
        assert_eq!(r["planes"].as_array().unwrap().len(), 2);
        assert!(r["dabs"].as_u64().unwrap() >= 1, "{r}");
    }

    #[test]
    fn new_layer_clone_stays_on_the_canvas() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#40c040"})).unwrap();
        // A red stripe where the first dab samples, so cloning it changes pixels.
        s.edit("stripe", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(140, 80, 160, 120), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let planes = json!([{"corners": [[0, 0], [200, 0], [200, 200], [0, 200]]}]);
        // The second dab lies left of the canvas and samples the canvas 50 px to its right; the
        // third is far off it (its box joined with the first's once asked for petabytes).
        for points in [json!([[100, 100], [-30, 100]]), json!([[100, 100], [-30, 100], [9_000_000, 9_000_000]])] {
            let r = s.execute(VP, json!({"planes": planes, "newLayer": true, "clone": [{"source": [150, 100], "points": points, "size": 20}]})).unwrap();
            let l = s.active().unwrap().doc.layer(LayerId(r["layer"].as_u64().unwrap())).unwrap().surface().unwrap();
            let b = l.content_bounds();
            assert!(!b.is_empty() && Rect::new(0, 0, 200, 200).contains_rect(&b), "{b:?}");
            assert!(l.rgba(100, 100)[0] > 0.9, "{:?}", l.rgba(100, 100));
            s.undo();
        }
    }

    /// Without a new layer the strokes paint the layer itself, and only inside the canvas, as in
    /// Photoshop: a dab off it painted pixels there (and allocated tiles for every dab, up to
    /// 1e7 px away).
    #[test]
    fn layer_clone_stays_on_the_canvas() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("edit.fill", json!({"color": "#40c040"})).unwrap();
        let id = s.active().unwrap().active_layer.unwrap();
        let planes = json!([{"corners": [[0, 0], [200, 0], [200, 200], [0, 200]]}]);
        // The second dab lies left of the canvas and samples the canvas 50 px to its right; the
        // third is far off it.
        let points = json!([[100, 100], [-30, 100], [9_000_000, 9_000_000]]);
        let r = s.execute(VP, json!({"planes": planes, "clone": [{"source": [150, 100], "points": points, "size": 20}]})).unwrap();
        assert_eq!(r["dabs"].as_u64(), Some(1), "{r}");
        let b = s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().content_bounds();
        assert!(Rect::new(0, 0, 200, 200).contains_rect(&b), "{b:?}");
    }

    #[test]
    fn layer_param_paints_that_layer_not_the_active_one() {
        let mut s = Session::new();
        s.execute("file.new", json!({"width": 200, "height": 200})).unwrap();
        s.execute("layer.new.layer", json!({"name": "floor"})).unwrap();
        s.execute("edit.fill", json!({"color": "#40c040"})).unwrap();
        s.edit("stripe", |doc, active| {
            doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap().fill_rect(Rect::new(140, 80, 160, 120), &[1.0, 0.0, 0.0, 1.0]);
            Ok(())
        })
        .unwrap();
        let floor = s.active().unwrap().active_layer.unwrap();
        s.execute("layer.new.layer", json!({"name": "other"})).unwrap();
        let other = s.active().unwrap().active_layer.unwrap();
        let planes = json!([{"corners": [[0, 0], [200, 0], [200, 200], [0, 200]]}]);
        let clone = json!([{"source": [150, 100], "points": [[100, 100]], "size": 20}]);
        let surf = |s: &Session, id| s.active().unwrap().doc.layer(id).unwrap().surface().unwrap().clone();
        let r = s.execute(VP, json!({"planes": planes, "clone": clone, "layer": floor.0})).unwrap();
        assert_eq!(r["layer"].as_u64(), Some(floor.0));
        assert!(surf(&s, floor).rgba(100, 100)[0] > 0.9, "{:?}", surf(&s, floor).rgba(100, 100));
        assert!(surf(&s, other).content_bounds().is_empty(), "the active layer stays untouched");
        s.undo();
        // A new layer goes right above the named layer.
        let r = s.execute(VP, json!({"planes": planes, "clone": clone, "layer": floor.0, "newLayer": true})).unwrap();
        let d = &s.active().unwrap().doc;
        let ids: Vec<u64> = d.layers.iter().map(|l| l.id.0).collect();
        let at = |id: u64| ids.iter().position(|&i| i == id).unwrap();
        assert_eq!(at(r["layer"].as_u64().unwrap()), at(floor.0) + 1, "{ids:?}");
        s.undo();
        let before = s.active().unwrap().doc.clone();
        assert!(s.execute(VP, json!({"planes": planes, "clone": clone, "layer": 999_999})).is_err());
        assert!(s.execute(VP, json!({"planes": planes, "clone": clone, "layer": 999_999, "newLayer": true})).is_err());
        assert!(std::sync::Arc::ptr_eq(&s.active().unwrap().doc, &before));
    }
}
