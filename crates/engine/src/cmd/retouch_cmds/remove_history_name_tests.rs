//! History step name of the Remove Tool (#12541): every applied removal adds exactly one step
//! named "Remove Tool", whatever the strokes, options, colour mode or bit depth.

use super::*;
use crate::jobs::Started;

/// A flat grey layer with a dark square at 40..52 × 24..36.
fn square_session(depth: u64, mode: &str) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 96, "height": 64, "depth": depth, "mode": mode})).unwrap();
    s.edit("setup", |doc, active| {
        let b = doc.bounds();
        let surf = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let fmt = surf.format();
        let mut data = Vec::new();
        for y in b.y0..b.y1 {
            for x in b.x0..b.x1 {
                let v = if (40..52).contains(&x) && (24..36).contains(&y) { 0.05 } else { 0.5 };
                data.extend(photocraft_raster::from_rgba(&fmt, [v, v, v, 1.0]));
            }
        }
        surf.write_region(b, &data);
        Ok(())
    })
    .unwrap();
    s
}

fn past(s: &Session) -> usize {
    s.active().unwrap().history.past_len()
}

fn last_step(s: &Session) -> Option<String> {
    s.active().unwrap().history.undo_label().map(str::to_string)
}

const STROKE: [[i32; 2]; 2] = [[38, 30], [54, 30]];

#[test]
fn one_stroke_is_one_step_named_remove_tool_in_every_mode_and_depth() {
    for (depth, mode) in [(8, "rgb"), (16, "rgb"), (32, "rgb"), (8, "cmyk"), (16, "gray"), (16, "lab")] {
        let mut s = square_session(depth, mode);
        let steps = past(&s);
        s.execute("paint.remove", json!({"points": STROKE, "size": 16})).unwrap();
        assert_eq!(past(&s), steps + 1, "{mode}/{depth}: one step");
        assert_eq!(last_step(&s).as_deref(), Some("Remove Tool"), "{mode}/{depth}");
    }
}

#[test]
fn waiting_strokes_applied_together_are_one_step_named_remove_tool() {
    let mut s = square_session(8, "rgb");
    let steps = past(&s);
    s.execute("paint.remove", json!({"strokes": [[[38, 30], [45, 30]], [[47, 30], [54, 30]]], "size": 16})).unwrap();
    assert_eq!(past(&s), steps + 1);
    assert_eq!(last_step(&s).as_deref(), Some("Remove Tool"));
}

#[test]
fn sample_all_layers_and_create_new_layer_are_one_step_named_remove_tool() {
    let mut s = square_session(8, "rgb");
    s.execute("layer.new.layer", json!({})).unwrap();
    let steps = past(&s);
    s.execute("paint.remove", json!({"points": STROKE, "size": 16, "sampleAllLayers": true})).unwrap();
    assert_eq!(past(&s), steps + 1, "sampleAllLayers");
    assert_eq!(last_step(&s).as_deref(), Some("Remove Tool"), "sampleAllLayers");

    let mut s = square_session(8, "rgb");
    let layers = s.active().unwrap().doc.layer_count();
    let steps = past(&s);
    let v = s.execute("paint.remove", json!({"points": STROKE, "size": 16, "newLayer": true})).unwrap();
    assert!(v["layer"].is_u64(), "{v}");
    assert_eq!(s.active().unwrap().doc.layer_count(), layers + 1, "newLayer adds the result layer");
    assert_eq!(past(&s), steps + 1, "newLayer");
    assert_eq!(last_step(&s).as_deref(), Some("Remove Tool"), "newLayer");
}

#[test]
fn the_background_job_records_remove_tool_and_undo_redo_keep_the_name() {
    let mut s = square_session(8, "rgb");
    let steps = past(&s);
    let Started::Job(job) = s.start("paint.remove", json!({"points": STROKE, "size": 16})).unwrap() else {
        panic!("paint.remove runs as a background job");
    };
    s.wait_job(job).unwrap();
    assert_eq!(past(&s), steps + 1);
    assert_eq!(last_step(&s).as_deref(), Some("Remove Tool"));
    s.execute("edit.undo", json!({})).unwrap();
    assert_eq!(s.active().unwrap().history.redo_label(), Some("Remove Tool"));
    s.execute("edit.redo", json!({})).unwrap();
    assert_eq!(last_step(&s).as_deref(), Some("Remove Tool"));
}
