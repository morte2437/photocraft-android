//! 24 MP Liquify preview updates: all channels, Red, and Red+Blue.
//! RAYON_NUM_THREADS=8 cargo run --release -p photocraft-ui-egui --example multi_channel_preview_bench
use photocraft_engine::Session;
use photocraft_geom::Rect;
use photocraft_ui_egui::canvas::{ToolEvent, tool_event};
use photocraft_ui_egui::control::{ControlRequest, Outcome, handle};
use photocraft_ui_egui::{PhotocraftApp, Services};
use serde_json::{Value, json};
use std::time::Instant;

fn invoke(app: &mut PhotocraftApp, ctx: &egui::Context, params: Value) -> Result<(), Box<dyn std::error::Error>> {
    let (request, _) = ControlRequest::new("ui.menu.invoke", params);
    match handle(app, ctx, &request) {
        Outcome::Done(value) if value.get("ok") != Some(&Value::Bool(false)) => Ok(()),
        Outcome::Done(value) => Err(format!("control failed: {value}").into()),
        _ => Err("expected an inline workspace response".into()),
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut fixture = Session::new();
    fixture.execute("file.new", json!({"width":6000,"height":4000,"depth":16,"background":"transparent"}))?;
    fixture.execute("layer.new.layer", json!({}))?;
    fixture.edit("Bands", |doc, id| {
        let surface = doc
            .layer_mut(id.ok_or(photocraft_engine::EngineError::NoDocument)?)
            .and_then(|l| l.surface_mut())
            .ok_or(photocraft_engine::EngineError::NoDocument)?;
        for x in (0..6000).step_by(64) {
            let value = 0.15 + (x % 640) as f32 / 1000.0;
            surface.fill_rect(Rect::new(x, 0, (x + 64).min(6000), 4000), &[value, 0.4, 1.0 - value, 0.8]);
        }
        Ok(())
    })?;
    let st = fixture.active().ok_or(photocraft_engine::EngineError::NoDocument)?;
    let layer = st.active_layer.ok_or(photocraft_engine::EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let ctx = egui::Context::default();
    let mut apps = Vec::new();
    let modes = ["all", "red", "red_blue"];
    let mut opening = Vec::new();
    for mode in modes {
        let mut session = Session::new();
        session.add_document((*doc).clone(), None);
        session.select_layer(layer)?;
        if mode != "all" {
            session.execute("channel.target", json!({"colors":if mode=="red" {vec![0]} else {vec![0,2]}}))?;
        }
        let mut app = PhotocraftApp::new(session, Services::default());
        let start = Instant::now();
        invoke(&mut app, &ctx, json!({"id":"filter.liquify"}))?;
        opening.push(start.elapsed().as_secs_f64() * 1000.0);
        invoke(&mut app, &ctx, json!({"id":"filter.liquify","params":{"ui":{"size":120,"density":50}}}))?;
        tool_event(&mut app, ToolEvent::Down { x: 2000.0, y: 1500.0, pressure: 1.0 }, egui::Modifiers::NONE);
        apps.push(app);
    }
    let mut samples = vec![Vec::new(); 3];
    for run in 0..7 {
        for offset in 0..3 {
            let which = (run + offset) % 3;
            let app = apps.get_mut(which).ok_or("missing benchmark app")?;
            let x = 2050.0 + run as f64 * 10.0;
            let start = Instant::now();
            tool_event(app, ToolEvent::Move { x, y: 1510.0, pressure: 0.8 }, egui::Modifiers::NONE);
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            if run > 0
                && let Some(row) = samples.get_mut(which)
            {
                row.push(ms);
            }
        }
    }
    for app in &mut apps {
        let (request, _) = ControlRequest::new("ui.inspect", Value::Null);
        let Outcome::Done(state) = handle(app, &ctx, &request) else { return Err("expected inline state".into()) };
        if state.pointer("/result/distort/liquify/maxDisplacement").and_then(Value::as_f64).is_none_or(|value| value <= 0.0) {
            return Err("the benchmark did not deform the Liquify field".into());
        }
    }
    let rows: Vec<_> = modes
        .iter()
        .zip(samples)
        .zip(opening)
        .map(|((mode, samples), open_ms)| {
            let mut sorted = samples.clone();
            sorted.sort_by(f64::total_cmp);
            json!({"target":mode,"open_ms":open_ms,"samples_ms":samples,"p50_ms":sorted.get(3),"max_ms":sorted.last()})
        })
        .collect();
    println!(
        "{}",
        serde_json::to_string_pretty(
            &json!({"size":[6000,4000],"depth":16,"operation":"Liquify 120px live move; native proxy preview","warmups":1,"repeats":6,"rows":rows})
        )?
    );
    Ok(())
}
