use super::{PhotocraftApp, RichText, Tokens, Value, empty, json, theme, widgets};

/// The active document's pixel under the pointer, if the pointer is over the image.
pub(crate) fn hover_pixel(app: &PhotocraftApp) -> Option<(i32, i32)> {
    let size = app.session.active()?.doc.size;
    app.hover_doc.map(|p| (p[0].floor() as i32, p[1].floor() as i32)).filter(|(x, y)| *x >= 0 && *y >= 0 && *x < size.width as i32 && *y < size.height as i32)
}

/// The colour the Info panel reads at document pixel `(x, y)`, straight RGBA 0..1: averaged over
/// the Eyedropper's Sample Size, as in Photoshop (#1649), and cached per pixel and revision.
pub(crate) fn pointer_sample(app: &mut PhotocraftApp, (x, y): (i32, i32)) -> Option<[f32; 4]> {
    let rev = app.session.active()?.revision;
    let sample_size = app.ui.tool_options.eyedropper_size;
    if let Some(((cx, cy, cr, cs), v)) = app.info_sample
        && (cx, cy, cr, cs) == (x, y, rev, sample_size)
    {
        return Some(v);
    }
    let params = json!({"x": f64::from(x) + 0.5, "y": f64::from(y) + 0.5, "size": sample_size});
    let v: Vec<f32> = serde_json::from_value(app.session.execute("document.sampleColor", params).ok()?).ok()?;
    let v = [*v.first()?, *v.get(1)?, *v.get(2)?, *v.get(3)?];
    app.info_sample = Some(((x, y, rev, sample_size), v));
    Some(v)
}

/// Photoshop's Info panel: colour under the pointer (RGB and CMYK), position, selection size.
pub(super) fn info_panel(app: &mut PhotocraftApp, ui: &mut egui::Ui) {
    let t = Tokens::get(ui.ctx());
    let Some(st) = app.session.active() else {
        empty(ui, tl!("No document"));
        return;
    };
    let (size, origin) = (st.doc.size, st.doc.ruler_origin);
    let cmyk_space = photocraft_compose::cmyk_space(&st.doc);
    // Positions and sizes in the ruler unit (Preferences › Units & Rulers).
    let units = app.session.prefs().units_and_rulers.clone();
    let dpi = st.doc.resolution_dpi as f64;
    let fx = move |px: f64| units.format(px, dpi, size.width as f64);
    let units_y = app.session.prefs().units_and_rulers.clone();
    let fy = move |px: f64| units_y.format(px, dpi, size.height as f64);
    let sel = st.doc.selection.as_ref().map(photocraft_compose::bounds::content_bounds);
    let pos = hover_pixel(app);
    // While a guide is dragged (#5728), the Info panel shows its position in one axis only, even
    // when the pointer is outside the canvas (ruler drags do not update `hover_doc` there).
    let (guide_x, guide_y) = match app.guide_drag {
        Some(d) if d.vertical => (Some(d.pos), None),
        Some(d) => (None, Some(d.pos)),
        None => (None, None),
    };
    let rgba = pos.and_then(|p| pointer_sample(app, p));
    let mono = theme::mono(11.5);
    let row = |ui: &mut egui::Ui, k: &str, v: String| {
        ui.horizontal(|ui| {
            ui.add_sized([18.0, 16.0], egui::Label::new(RichText::new(k).color(t.text_dim).size(11.5)));
            ui.label(RichText::new(v).font(mono.clone()).color(t.text));
        });
    };
    ui.columns(2, |cols| {
        let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as i32;
        let dash = || "—".to_string();
        row(&mut cols[0], "R:", rgba.map_or_else(dash, |c| q(c[0]).to_string()));
        row(&mut cols[0], "G:", rgba.map_or_else(dash, |c| q(c[1]).to_string()));
        row(&mut cols[0], "B:", rgba.map_or_else(dash, |c| q(c[2]).to_string()));
        // Colour-managed CMYK in percent like Photoshop's readout, through the same conversion as
        // the Color Picker (#8101) and the Color Sampler readout (`sampler_cmds::device_cmyk`).
        let cmyk =
            rgba.map(|c| photocraft_color::convert::with_cmyk_space(cmyk_space.as_ref(), || photocraft_engine::sampler_cmds::device_cmyk(c[0], c[1], c[2])));
        for (i, k) in ["C:", "M:", "Y:", "K:"].iter().enumerate() {
            row(&mut cols[1], k, cmyk.map_or_else(dash, |c| format!("{}%", c[i])));
        }
    });
    widgets::hairline(ui);
    ui.columns(2, |cols| {
        let dash = || "—".to_string();
        // Measured from the rulers' zero point (#2955).
        row(&mut cols[0], "X:", guide_x.map(|p| fx(p - origin[0])).or_else(|| pos.map(|p| fx(p.0 as f64 - origin[0]))).unwrap_or_else(dash));
        row(&mut cols[0], "Y:", guide_y.map(|p| fy(p - origin[1])).or_else(|| pos.map(|p| fy(p.1 as f64 - origin[1]))).unwrap_or_else(dash));
        row(&mut cols[1], "W:", sel.map_or_else(dash, |r| fx(r.width() as f64)));
        row(&mut cols[1], "H:", sel.map_or_else(dash, |r| fy(r.height() as f64)));
    });
    widgets::hairline(ui);
    ui.label(
        RichText::new(crate::i18n::fmt(tl!("Doc: {w} × {h} px"), &[("w", &size.width.to_string()), ("h", &size.height.to_string())]))
            .color(t.text_dim)
            .size(11.5),
    );
    // Color Sampler points (#1046): each under its number, with its position in the current ruler
    // unit and the same RGB/CMYK readout as the pointer (one composite pixel each).
    let samplers = sampler_readouts(app);
    if !samplers.is_empty() {
        widgets::hairline(ui);
        let cell = |ui: &mut egui::Ui, s: String| {
            ui.label(RichText::new(s).font(mono.clone()).color(t.text));
        };
        for s in samplers.iter() {
            let n = s["number"].as_i64().unwrap_or(0);
            let at = s["position"]
                .as_array()
                .map_or((0.0, 0.0), |a| (a.first().and_then(Value::as_f64).unwrap_or(0.0), a.get(1).and_then(Value::as_f64).unwrap_or(0.0)));
            let num = |v: &Value, i: usize| v.get(i).and_then(Value::as_i64).map_or_else(|| "—".to_string(), |n| n.to_string());
            let (rgb, cmyk) = (&s["rgb"], &s["cmyk"]);
            ui.horizontal(|ui| {
                ui.add_sized([24.0, 16.0], egui::Label::new(RichText::new(format!("#{n}")).color(t.text_dim).size(11.5)));
                cell(ui, format!("X: {}  Y: {}", fx(at.0), fy(at.1)));
            });
            ui.horizontal(|ui| {
                ui.add_sized([24.0, 16.0], egui::Label::new(RichText::new("").size(11.5)));
                cell(ui, format!("R:{} G:{} B:{}", num(rgb, 0), num(rgb, 1), num(rgb, 2)));
            });
            ui.horizontal(|ui| {
                ui.add_sized([24.0, 16.0], egui::Label::new(RichText::new("").size(11.5)));
                cell(ui, format!("C:{}% M:{}% Y:{}% K:{}%", num(cmyk, 0), num(cmyk, 1), num(cmyk, 2), num(cmyk, 3)));
            });
        }
    }
}

/// The Info panel's Color Sampler readouts and the (document, revision) they were composited for.
pub(crate) type SamplerReadouts = ((photocraft_doc::DocId, u64), std::sync::Arc<Vec<Value>>);

/// The Color Sampler readouts of the active document, composited once per revision (#4528):
/// adding, moving or deleting a sampler and every pixel edit bump the revision, so between them
/// the Info panel reuses them instead of compositing each point through the stack every frame.
pub(crate) fn sampler_readouts(app: &mut PhotocraftApp) -> std::sync::Arc<Vec<Value>> {
    let Some(key) = app.session.active().map(|s| (s.doc.id, s.revision)) else { return std::sync::Arc::default() };
    if let Some((k, v)) = &app.info_samplers
        && *k == key
    {
        return v.clone();
    }
    let list = app.session.execute("view.colorSamplers.list", json!({})).ok();
    let v = std::sync::Arc::new(list.as_ref().and_then(|v| v["samplers"].as_array()).cloned().unwrap_or_default());
    app.info_samplers = Some((key, v.clone()));
    v
}
