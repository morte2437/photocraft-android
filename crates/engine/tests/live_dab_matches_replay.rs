use photocraft_engine::{Session, retouch_cmds::LiveDab};
use photocraft_paint::StrokePoint;
use serde_json::json;

fn setup(depth: u32, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width":200,"height":120,"depth":depth,"mode":mode})).unwrap();
    s.execute("layer.new.layer", json!({})).unwrap();
    s.edit("texture", |doc, active| {
        let bounds = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut px = Vec::new();
        for y in 0..120 {
            for x in 0..200 {
                px.extend(photocraft_raster::from_rgba(
                    &fmt,
                    [((x * 7 + y * 3) % 31) as f32 / 31.0, x as f32 / 200.0, y as f32 / 120.0, 0.4 + ((x + y) % 7) as f32 / 12.0],
                ));
            }
        }
        surf.write_region(bounds, &px);
        Ok(())
    })
    .unwrap();
    s
}

fn pixels(s: &Session) -> Vec<f32> {
    let st = s.active().unwrap();
    st.doc.layer(st.active_layer.unwrap()).unwrap().surface().unwrap().read_region(st.doc.bounds())
}

#[test]
fn pressure_shape_and_smoothing_match_replay() {
    let mut failures = Vec::new();
    for cmd in ["paint.smudge", "paint.blur", "paint.sharpen", "paint.dodge", "paint.burn", "paint.sponge"] {
        for case in ["pressure", "locked", "ellipse", "smoothing", "scattering"] {
            for depth in [8, 16, 32] {
                let mut s = setup(depth, "rgb");
                match case {
                    "locked" => {
                        s.edit("lock alpha", |doc, active| {
                            doc.layer_mut(active.unwrap()).unwrap().locks.transparency = true;
                            Ok(())
                        })
                        .unwrap();
                    }
                    "ellipse" => {
                        s.tools.brush.roundness = 0.2;
                        s.tools.brush.angle = 37.0;
                    }
                    "smoothing" => {
                        s.tools.brush.smoothing.amount = 0.6;
                    }
                    "scattering" => {
                        s.tools.brush.scattering.enabled = true;
                        s.tools.brush.scattering.scatter.jitter = 1.0;
                    }
                    _ => {}
                }
                let points = [[40.0, 40.0, 0.1], [65.0, 50.0, 0.4], [100.0, 60.0, 1.0], [140.0, 70.0, 0.7], [60.0, 40.0, 0.2]];
                let params = json!({"points":points,"size":40,"hardness":70,"strength":80,"opacity":55,"spacing":25});
                s.execute(cmd, params.clone()).unwrap();
                let want = pixels(&s);
                s.execute("edit.undo", json!({})).unwrap();
                let mut start = params.clone();
                start["points"] = json!([points[0]]);
                let mut live = LiveDab::begin(&s, cmd, &start).unwrap();
                for q in &points[1..] {
                    live.push(&[StrokePoint::new(q[0], q[1], q[2] as f32)]).unwrap();
                }
                s.offer_live_dab(live).unwrap();
                let result = s.execute(cmd, params).unwrap();
                assert_eq!(result["fromLive"], true);
                let got = pixels(&s);
                let count = got.iter().zip(&want).filter(|(a, b)| a != b).count();
                let max = got.iter().zip(&want).map(|(a, b)| (a - b).abs()).fold(0.0f32, f32::max);
                if count > 0 {
                    failures.push(format!("{cmd} {case} depth={depth}: {count} channels differ, max={max}"));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{}", failures.join("\n"));
}

#[test]
fn native_modes_selection_and_lock_match_replay() {
    for mode in ["rgb", "gray", "cmyk", "lab"] {
        for depth in [8, 16, 32] {
            let mut s = setup(depth, mode);
            s.edit("lock alpha", |doc, active| {
                doc.layer_mut(active.unwrap()).unwrap().locks.transparency = true;
                Ok(())
            })
            .unwrap();
            s.execute("select.rect", json!({"x":30,"y":20,"width":110,"height":60})).unwrap();
            let points = [[-10.0, 30.0], [80.0, 50.0], [160.0, 80.0], [60.0, 60.0]];
            for cmd in ["paint.blur", "paint.sharpen", "paint.dodge", "paint.burn", "paint.sponge", "paint.smudge"] {
                let params = json!({"points":points,"size":30,"hardness":40,"strength":70,"opacity":37});
                let expected_damage = s.execute(cmd, params.clone()).unwrap()["damage"].clone();
                let want = pixels(&s);
                s.execute("edit.undo", json!({})).unwrap();
                let mut start = params.clone();
                start["points"] = json!([points[0]]);
                let mut live = LiveDab::begin(&s, cmd, &start).unwrap();
                for q in &points[1..] {
                    live.push(&[StrokePoint::new(q[0], q[1], 1.0)]).unwrap();
                }
                s.offer_live_dab(live).unwrap();
                let result = s.execute(cmd, params).unwrap();
                assert_eq!(result["fromLive"], true);
                assert_eq!(result["damage"], expected_damage);
                assert_eq!(pixels(&s), want, "{mode} {depth} {cmd}");
                s.execute("edit.undo", json!({})).unwrap();
            }
        }
    }
}

#[test]
fn changed_document_refuses_live_pixels() {
    let mut s = setup(8, "rgb");
    let p = json!({"points":[[40,40],[80,60]],"size":30});
    let live = LiveDab::begin(&s, "paint.blur", &p).unwrap();
    s.execute("paint.stroke", json!({"points":[[40,40],[80,60]],"color":"#ffffff","size":20})).unwrap();
    let mut replay = Session::new();
    replay.add_document((*s.active().unwrap().doc).clone(), None);
    replay.tools.brush = s.tools.brush.clone();
    s.offer_live_dab(live).unwrap();
    assert!(s.execute("paint.blur", p.clone()).unwrap().get("fromLive").is_none());
    replay.execute("paint.blur", p).unwrap();
    assert_eq!(pixels(&s), pixels(&replay));
}

/// An independent paint-level oracle gives Smudge a generously padded working region. Pixels
/// picked up by the initial small dab must not depend on the final stroke's bounding rectangle.
#[test]
fn smudge_pickup_matches_full_neighborhood_when_pressure_expands() {
    use photocraft_paint::{
        Stroke,
        retouch::{Smudge, apply_dab_stroke},
    };

    let points = [[40.0, 40.0, 0.1], [65.0, 50.0, 0.4], [100.0, 60.0, 1.0], [140.0, 70.0, 0.7], [60.0, 40.0, 0.2]];
    for depth in [8, 16, 32] {
        let mut s = setup(depth, "rgb");
        let params = json!({"points":points,"size":40,"hardness":70,"strength":80,"opacity":55,"spacing":25});
        let (mut expected, lock) = {
            let st = s.active().unwrap();
            let id = st.active_layer.unwrap();
            (st.doc.layer(id).unwrap().surface().unwrap().clone(), st.doc.effective_locks(id).transparency)
        };
        let stroke = Stroke {
            brush: photocraft_engine::retouch_cmds::stroke_brush(&s.tools.brush, &params),
            points: points.iter().map(|q| StrokePoint::new(q[0], q[1], q[2] as f32)).collect(),
        };
        let mut effect = Smudge::new(0.8, None, 40.0).with_alpha(Some(3));
        apply_dab_stroke(&mut expected, &stroke, None, lock, 40, |work, fp| effect.dab(work, fp));
        let bounds = s.active().unwrap().doc.bounds();
        let want = expected.read_region(bounds);
        s.execute("paint.smudge", params.clone()).unwrap();
        assert!(pixels(&s) == want, "command must read the whole pickup square at depth {depth}");
        s.execute("edit.undo", json!({})).unwrap();
        let live = LiveDab::begin(&s, "paint.smudge", &params).unwrap();
        s.offer_live_dab(live).unwrap();
        assert_eq!(s.execute("paint.smudge", params).unwrap()["fromLive"], true);
        assert!(pixels(&s) == want, "a single batch uses the same pickup square at depth {depth}");
    }
}

#[test]
fn pressure_sized_live_dabs_commit_only_the_requested_mask_or_channel() {
    use photocraft_raster::Surface;

    let fill = |surface: &mut Surface| {
        let values: Vec<f32> = (0..120).flat_map(|y| (0..200).map(move |x| ((x * 7 + y * 3) % 31) as f32 / 31.0)).collect();
        surface.write_region(photocraft_geom::Rect::new(0, 0, 200, 120), &values);
    };
    for depth in [8, 16, 32] {
        let mut s = setup(depth, "rgb");
        s.execute("layer.new.layer", json!({})).unwrap();
        s.execute("layer.layerMask.revealAll", json!({})).unwrap();
        s.execute("channel.new", json!({"fill":"white"})).unwrap();
        s.execute("select.editInQuickMaskMode", json!({"on":true})).unwrap();
        s.edit("mask texture", |doc, active| {
            fill(&mut doc.layer_mut(active.unwrap()).unwrap().mask.as_mut().unwrap().surface);
            fill(&mut doc.channels[0].surface);
            fill(&mut doc.quick_mask.as_mut().unwrap().surface);
            Ok(())
        })
        .unwrap();
        let before = s.active().unwrap().doc.clone();
        for target in [json!("mask"), json!({"channel":0}), json!("quickMask")] {
            for cmd in ["paint.smudge", "paint.blur", "paint.sharpen", "paint.dodge", "paint.burn", "paint.sponge"] {
                let points = [[40.0, 40.0, 0.1], [65.0, 50.0, 0.4], [100.0, 60.0, 1.0], [140.0, 70.0, 0.7], [60.0, 40.0, 0.2]];
                let params = json!({"target":target,"points":points,"size":40,"hardness":70,"strength":80,"opacity":55,"spacing":25});
                s.execute(cmd, params.clone()).unwrap();
                let want = s.active().unwrap().doc.clone();
                s.execute("edit.undo", json!({})).unwrap();
                let mut start = params.clone();
                start["points"] = json!([points[0]]);
                let mut live = LiveDab::begin(&s, cmd, &start).unwrap();
                for q in &points[1..] {
                    live.push(&[StrokePoint::new(q[0], q[1], q[2] as f32)]).unwrap();
                }
                s.offer_live_dab(live).unwrap();
                assert_eq!(s.execute(cmd, params).unwrap()["fromLive"], true);
                assert_eq!(s.active().unwrap().doc, want, "depth={depth} {cmd} target={target}");
                s.execute("edit.undo", json!({})).unwrap();
                assert_eq!(s.active().unwrap().doc, before);
            }
        }
    }
}
