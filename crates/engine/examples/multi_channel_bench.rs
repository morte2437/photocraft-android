//! Interleaved 24 MP command timings. The same harness runs on the pre-change base.
//! `cargo run --release -p photocraft-engine --example multi_channel_bench -- --subset`
use photocraft_engine::{Session, brush_cmds::LiveStroke};
use photocraft_geom::Rect;
use photocraft_paint::StrokePoint;
use serde_json::{Value, json};
use std::time::Instant;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let subset = std::env::args().any(|a| a == "--subset");
    let region = std::env::args().any(|a| a == "--region");
    let clipboard = std::env::args().any(|a| a == "--clipboard");
    if region && clipboard {
        return Err("choose --region or --clipboard".into());
    }
    let mut fixture = Session::new();
    fixture.execute("file.new", json!({"width":6000,"height":4000,"depth":16,"background":"transparent"}))?;
    fixture.execute("layer.new.layer", json!({}))?;
    fixture.edit("synthetic bands", |doc, id| {
        let surf = doc
            .layer_mut(id.ok_or(photocraft_engine::EngineError::NoDocument)?)
            .and_then(|l| l.surface_mut())
            .ok_or(photocraft_engine::EngineError::NoDocument)?;
        for x in (0..6000).step_by(64) {
            let value = 0.15 + (x % 640) as f32 / 1000.0;
            surf.fill_rect(Rect::new(x, 0, (x + 64).min(6000), 4000), &[value, 0.4, 1.0 - value, 0.8]);
        }
        Ok(())
    })?;
    let layer = fixture.active().and_then(|st| st.active_layer).ok_or(photocraft_engine::EngineError::NoDocument)?;
    let base = fixture.active().ok_or(photocraft_engine::EngineError::NoDocument)?.doc.clone();
    let commands: [(&str, Value); 9] = [
        ("image.adjustments.invert", json!({})),
        ("image.adjustments.threshold", json!({"level":128})),
        ("image.adjustments.levels", json!({"gamma":1.4})),
        ("image.adjustments.curves", json!({"points":[[0,0],[100,145],[255,255]]})),
        ("filter.blur.gaussianBlur", json!({"radius":8})),
        ("paint.stroke", json!({"points":[[2000,1500,1],[2500,1700,0.8],[3000,1600,1]],"size":120,"color":"#ffffff","seed":42})),
        ("paint.stroke", json!({"points":[[2000,1500,1],[2500,1700,0.8],[3000,1600,1]],"size":120,"erase":true,"seed":42})),
        ("live.brush.extend", json!({"points":[[2000,1500,1]],"size":120,"color":"#ffffff","seed":42})),
        ("live.eraser.extend", json!({"points":[[2000,1500,1]],"size":120,"erase":true,"seed":42})),
    ];
    let modes = if clipboard {
        if subset { vec!["single", "subset"] } else { vec!["single"] }
    } else if subset {
        vec!["composite", "single", "subset"]
    } else {
        vec!["composite", "single"]
    };
    let clipboard_cases = [("edit.paste", json!({}))];
    let cases = if clipboard { &clipboard_cases[..] } else { &commands[..if region { 5 } else { commands.len() }] };
    let mut rows = Vec::new();
    for (index, (command, params)) in cases.iter().enumerate() {
        let mut samples = vec![Vec::new(); modes.len()];
        for run in 0..7 {
            for offset in 0..modes.len() {
                let which = (run + offset) % modes.len();
                let mut s = Session::new();
                s.add_document((*base).clone(), None);
                s.select_layer(layer)?;
                match modes.get(which).copied().unwrap_or("composite") {
                    "single" => {
                        s.execute("channel.target", json!({"channel":{"color":0}}))?;
                    }
                    "subset" => {
                        s.execute("channel.target", json!({"colors":[0,2]}))?;
                    }
                    _ => {}
                }
                if clipboard {
                    s.execute("select.rect", json!({"x":2000,"y":1500,"width":1,"height":1}))?;
                    s.execute("edit.copy", json!({}))?;
                    s.execute("select.deselect", json!({}))?;
                }
                if region {
                    s.execute("select.rect", json!({"x":2000,"y":1500,"width":16,"height":16}))?;
                }
                let ms = if command.starts_with("live.") {
                    let mut stroke = LiveStroke::begin(&s, params)?;
                    let start = Instant::now();
                    stroke.push(&[StrokePoint::new(2050.0, 1510.0, 0.8)])?;
                    start.elapsed().as_secs_f64() * 1000.0
                } else {
                    let start = Instant::now();
                    s.execute(command, params.clone())?;
                    start.elapsed().as_secs_f64() * 1000.0
                };
                if run > 0
                    && let Some(sample) = samples.get_mut(which)
                {
                    sample.push(ms);
                }
            }
        }
        for (mode, mut samples) in modes.iter().zip(samples) {
            let raw = samples.clone();
            samples.sort_by(f64::total_cmp);
            let median = samples.get(samples.len() / 2).copied().unwrap_or(0.0);
            let p95 = samples.last().copied().unwrap_or(0.0);
            let row = json!({"command":command,"case":index,"target":mode,"p50_ms":median,"p95_ms":p95,"samples_ms":raw});
            eprintln!("{row}");
            rows.push(row);
        }
    }
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"size":[6000,4000],"depth":16,"clipboard_pixel":clipboard,"selection":if region {json!([2000,1500,16,16])}else{Value::Null},"warmups":1,"repeats":6,"rows":rows})
        )?
    );
    Ok(())
}
