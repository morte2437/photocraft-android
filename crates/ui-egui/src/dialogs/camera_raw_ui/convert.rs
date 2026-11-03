use super::open::{Coverage, Target};
use super::{CameraRaw, CameraRawDialog, LayerId, MAX_SAMPLERS, PROXY_SIDE, PhotocraftApp, Rect, RgbHistogram, Value, json};

/// Camera Raw Defaults › Sharpen For: output sharpening, folded into the Camera Raw step `rest`
/// as the workflow's Sharpen For is. Returns what it applied.
pub(super) fn output_sharpening(d: &photocraft_engine::prefs::RawDefaults, rest: &mut CameraRaw) -> Vec<&'static str> {
    match d.sharpen_for.preset() {
        Some((amount, radius)) if rest.sharpen_amount == 0.0 => {
            rest.sharpen_amount = amount;
            rest.sharpen_radius = radius;
            vec!["sharpenFor"]
        }
        _ => Vec::new(),
    }
}

/// The Camera Raw Defaults [`apply_raw_conversions`] applies. Resolution is not among them: the
/// open itself sets it (`file_cmds::raw_defaults`), and a re-develop keeps it.
pub(super) fn raw_conversions(d: &photocraft_engine::prefs::RawDefaults) -> Vec<&'static str> {
    use photocraft_engine::prefs::{RawColorSpace, RawDepth};
    let mut out = Vec::new();
    // Raws develop into ProPhoto-compatible RGB (`photocraft_io::raw`); the other spaces are a
    // conversion.
    if d.color_space != RawColorSpace::ProPhoto {
        out.push("colorSpace");
    }
    if d.bit_depth == RawDepth::Eight {
        out.push("bitDepth");
    }
    if d.open_as_smart_object {
        out.push("openAsSmartObject");
    }
    out
}

/// Camera Raw Defaults (Preferences) after the Camera Raw step: the colour space and bit depth
/// the developed document opens in, and Open as Smart Object.
pub(super) fn apply_raw_conversions(app: &mut PhotocraftApp, index: usize, d: &photocraft_engine::prefs::RawDefaults) -> Result<(), String> {
    app.session.set_active(index);
    for step in raw_conversions(d) {
        let (id, params) = match step {
            "colorSpace" => ("edit.convertToProfile", json!({"profile": d.color_space.profile_id(), "intent": "relative"})),
            "bitDepth" => ("image.mode.bits8", json!({})),
            _ => ("layer.smartObjects.convertToSmartObject", json!({})),
        };
        app.session.execute(id, params).map_err(|e| e.to_string())?;
    }
    Ok(())
}

pub(super) fn open_pixels(
    app: &mut PhotocraftApp,
    ctx: &egui::Context,
    layer: LayerId,
    surf: photocraft_raster::Surface,
    params: CameraRaw,
    coverage: Coverage,
    target: Target,
) -> Result<(), String> {
    let st = app.session.active().ok_or("no document")?;
    let canvas = st.doc.bounds();
    let name = st.doc.layer(layer).map(|l| l.name.clone()).unwrap_or_default();
    let mut params = params;
    if params.encoding_gamma == 0.0 {
        params.encoding_gamma = photocraft_engine::lens_cmds::encoding_gamma(&st.doc);
    }
    // Use the engine filter domain, including pixels outside the canvas: spatial Camera Raw
    // stages (vignette, grain, local contrast) must agree with the committed result.
    let area = canvas.union(&surf.content_bounds());
    let width = area.x1.checked_sub(area.x0).filter(|n| (1..=1_048_576).contains(n));
    let height = area.y1.checked_sub(area.y0).filter(|n| (1..=1_048_576).contains(n));
    let (Some(w), Some(h)) = (width, height) else {
        return Err("Camera Raw preview bounds are too large".into());
    };
    let (w, h) = (w as usize, h as usize);
    if w.checked_mul(h).is_none_or(|pixels| pixels > 512_000_000) {
        return Err("Camera Raw preview bounds are too large".into());
    }
    let k = w.max(h).div_ceil(PROXY_SIDE).max(1);
    let (pw, ph) = (w.div_ceil(k), h.div_ceil(k));
    let mut proxy = vec![[0.0f32; 4]; pw * ph];
    let mut row = vec![[0.0f32; 4]; w];
    for py in 0..ph {
        // Box-average k×k blocks.
        let mut acc = vec![[0.0f32; 5]; pw];
        for dy in 0..k {
            let y = area.y0 + (py * k + dy).min(h - 1) as i32;
            surf.read_rgba_into(Rect::new(area.x0, y, area.x1, y + 1), &mut row);
            for (x, q) in row.iter().enumerate() {
                let a = &mut acc[x / k];
                for c in 0..4 {
                    a[c] += q[c];
                }
                a[4] += 1.0;
            }
        }
        for (x, a) in acc.iter().enumerate() {
            proxy[py * pw + x] = [a[0] / a[4], a[1] / a[4], a[2] / a[4], a[3] / a[4]];
        }
    }
    // Coverage at the centre of each proxy block.
    let sample = |value: &dyn Fn(i32, i32) -> f32| {
        (0..ph)
            .flat_map(|y| (0..pw).map(move |x| (area.x0 + (x * k + k / 2).min(w - 1) as i32, area.y0 + (y * k + k / 2).min(h - 1) as i32)))
            .map(|(x, y)| value(x, y).clamp(0.0, 1.0))
            .collect::<Vec<_>>()
    };
    let selection = match &coverage {
        Coverage::Selection => st.doc.selection.as_ref().map(|mask| sample(&|x, y| mask.sample_channel(x, y, 0))),
        Coverage::Mask(mask) => Some(sample(&|x, y| mask.value(x, y))),
        Coverage::None => None,
    };
    let source = photocraft_engine::color_cmds::composite_profile(&st.doc);
    let display = app.session.color.canvas_display(&st.doc).map_err(|e| e.to_string())?;
    let lab_transform = photocraft_cms::cached(&source, photocraft_cms::Builtin::LabD50.profile(), Default::default()).map_err(|e| e.to_string())?;
    let scope_transform = photocraft_cms::cached(&source, photocraft_cms::Builtin::Srgb.profile(), Default::default()).map_err(|e| e.to_string())?;
    if let Some(saved) = app.session.prefs().dialogs.get("filter.cameraRaw.scope")
        && saved.get("samplers").and_then(Value::as_array).is_none_or(|a| a.len() <= MAX_SAMPLERS)
        && let Ok(view) = serde_json::from_value(saved.clone())
    {
        app.ui.camera_raw_scope = view;
    }
    app.ui.camera_raw_preview = Default::default();
    app.ui.camera_raw_scope.samplers.clear();
    // Keep Camera Raw compact on open; vectorscope is an opt-in context-menu view.
    app.ui.camera_raw_scope.vectorscope = false;
    if selection.is_none() {
        app.ui.camera_raw_scope.selected_region = false;
    }
    let float = surf.format().sample == photocraft_color::SampleType::F32;
    // Builds the GPU pass the first time any Camera Raw dialog opens.
    let gpu = crate::camera_raw_gpu::for_app(app);
    let mut d = CameraRawDialog {
        layer,
        layer_name: name,
        original: params.clone(),
        params,
        target,
        before_histogram: RgbHistogram::from_rgba(&proxy),
        processed: Vec::new(),
        lab_transform,
        scope_transform,
        scope: Default::default(),
        after_histogram: RgbHistogram::default(),
        preview_revision: 0,
        histogram_ms: 0.0,
        proxy: std::sync::Arc::new(proxy),
        selection: selection.map(std::sync::Arc::new),
        pw,
        ph,
        full_w: w,
        full_h: h,
        preview_ppp: ctx.pixels_per_point(),
        detail: crate::camera_raw_detail_ui::DetailPreview::new(
            surf,
            area,
            match coverage {
                Coverage::Selection => {
                    st.doc.selection.clone().map(crate::camera_raw_detail_ui::Coverage::Selection).unwrap_or(crate::camera_raw_detail_ui::Coverage::None)
                }
                Coverage::Mask(mask) => crate::camera_raw_detail_ui::Coverage::Mask(mask),
                Coverage::None => crate::camera_raw_detail_ui::Coverage::None,
            },
        ),
        display,
        viewport: None,
        float,
        tex: None,
        before_tex: None,
        before_display_key: None,
        dirty: true,
        show_before: false,
        mixer_tab: 1,
        curve_state: Default::default(),
        curve_rect: None,
        render_ms: 0.0,
        raw_open: None,
        raw_curve: None,
        pending: None,
        preview_error: None,
        gpu,
        source_key: crate::camera_raw_gpu::source_key(),
        develop_ms: 0.0,
        develop_backend: "cpu",
        undo: Vec::new(),
        redo: Vec::new(),
        edit_origin: None,
    };
    d.render(ctx);
    app.camera_raw = Some(d);
    Ok(())
}
