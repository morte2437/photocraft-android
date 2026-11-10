use super::{
    BlendMode, Document, EngineError, FadeTarget, Rect, Result, Session, Surface, Value, bad, blend_from_str, f32_or, from_rgba_into, json, str_or, surface_of,
    surface_of_mut, to_rgba,
};

// ------------------------------------------------------------------ Fade

pub(super) fn can_fade(s: &Session) -> std::result::Result<(), String> {
    let st = s.active().ok_or("no document open")?;
    let f = s.edit_state.fade.as_ref().ok_or("nothing to fade")?;
    if f.doc != st.doc.id || f.revision != st.revision || !st.history.can_undo() {
        return Err("Fade is only available right after a filter, adjustment or painting step".into());
    }
    if surface_of(&st.doc, f.target).is_none() {
        return Err(match f.target {
            FadeTarget::Pixels(_) | FadeTarget::Mask(_) => "the faded layer no longer exists".into(),
            FadeTarget::Channel(_) | FadeTarget::QuickMask => "the faded channel no longer exists".into(),
        });
    }
    Ok(())
}

/// Blend `after` back towards `before` over `area`: `before + opacity × (B(before, after) − before)`
/// with blend mode `mode` (premultiplied, so alpha changes fade too).
pub fn fade_surface(before: &Surface, after: &mut Surface, area: Rect, mode: BlendMode, opacity: f32) -> Result<()> {
    let fmt = after.format();
    let n = fmt.channels();
    if area.is_empty() || n == 0 {
        return Ok(());
    }
    let b = crate::allocation::read_region(before, area, "fading pixels")?;
    let mut a = crate::allocation::read_region(after, area, "fading pixels")?;
    let k = opacity.clamp(0.0, 1.0);
    let lerp_premul = |pb: &[f32], pa: &[f32], out: &mut [f32], alpha: bool| {
        let m = out.len();
        if !alpha {
            for c in 0..m {
                out[c] = pb[c] + (pa[c] - pb[c]) * k;
            }
            return;
        }
        let (ab, aa) = (pb[m - 1], pa[m - 1]);
        let ao = ab + (aa - ab) * k;
        for c in 0..m - 1 {
            let pm = pb[c] * ab + (pa[c] * aa - pb[c] * ab) * k;
            out[c] = if ao > 1e-9 { pm / ao } else { 0.0 };
        }
        out[m - 1] = ao;
    };
    let mut tmp = crate::allocation::filled(n, 0.0f32, "fading pixels")?;
    for (pa, pb) in a.chunks_exact_mut(n).zip(b.chunks_exact(n)) {
        if mode == BlendMode::Normal || n == 1 {
            lerp_premul(pb, pa, &mut tmp, fmt.alpha);
            pa.copy_from_slice(&tmp);
        } else {
            let rb = to_rgba(&fmt, pb);
            let ra = to_rgba(&fmt, pa);
            let blended = photocraft_color::blend::composite(mode, rb, ra, 1.0);
            let mut o = [0.0f32; 4];
            lerp_premul(&rb, &blended, &mut o, true);
            let mut enc = [0.0f32; 8];
            from_rgba_into(&fmt, o, &mut enc);
            pa.copy_from_slice(&enc[..n]);
        }
    }
    after.try_write_region(area, &a)?;
    Ok(())
}

/// What Edit › Fade with `p` blends: the layer, mask or channel of the last step, its pixels
/// before it, the area to blend, the mode and the opacity.
struct FadeJob {
    target: FadeTarget,
    label: String,
    before: Surface,
    area: Rect,
    mode: BlendMode,
    opacity: f32,
}

fn fade_job(s: &Session, p: &Value) -> Result<FadeJob> {
    let cmd = "edit.fade";
    let src = s.edit_state.fade.clone().ok_or_else(|| bad(cmd, "nothing to fade"))?;
    let opacity = f32_or(p, "opacity", 100.0).clamp(0.0, 100.0) / 100.0;
    let mode = blend_from_str(str_or(p, "mode", "normal")).ok_or_else(|| bad(cmd, "unknown blend mode"))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let prev = st.history.past_len().checked_sub(1).and_then(|i| st.history.state(i)).ok_or_else(|| bad(cmd, "no previous state"))?;
    let before = surface_of(&prev, src.target).cloned().ok_or_else(|| EngineError::Other("the layer or channel did not exist before this step".into()))?;
    let now = surface_of(&st.doc, src.target).ok_or_else(|| EngineError::Other("the faded layer or channel no longer exists".into()))?;
    if now.format() != before.format() {
        return Err(EngineError::Other("can't fade a step that changed the layer's pixel format".into()));
    }
    // A uniform mask can change every pixel without storing a single tile. In that case
    // the content bounds are empty, but Fade must still blend across the canvas.
    let area = if before.default_pixel() != now.default_pixel() { st.doc.bounds() } else { before.content_bounds().union(&now.content_bounds()) };
    Ok(FadeJob { target: src.target, label: src.label, before, area, mode, opacity })
}

fn apply_fade(doc: &mut Document, j: &FadeJob) -> Result<()> {
    let surf = surface_of_mut(doc, j.target).ok_or_else(|| EngineError::Other("the faded layer or channel no longer exists".into()))?;
    fade_surface(&j.before, surf, j.area, j.mode, j.opacity)?;
    surf.prune();
    Ok(())
}

pub(super) fn fade(s: &mut Session, p: &Value) -> Result<Value> {
    let j = fade_job(s, p)?;
    let label = format!("Fade {}", j.label);
    s.edit(&label, |doc, _| apply_fade(doc, &j))?;
    Ok(json!({"faded": j.label, "opacity": j.opacity * 100.0, "mode": j.mode.label()}))
}

/// Edit › Fade with given params, captured from the session without a history step: the Fade
/// dialog's live preview (#8230). [`FadePreview::render`] does the blend and can run off the
/// session (the preview worker). The document copy shares its tiles with the session's, and
/// only the tiles in the faded area are blended and rewritten.
pub struct FadePreview {
    doc: Document,
    job: FadeJob,
}

impl FadePreview {
    /// The active document as Edit › Fade would leave it.
    pub fn render(mut self) -> Result<Document> {
        apply_fade(&mut self.doc, &self.job)?;
        Ok(self.doc)
    }
}

/// What Edit › Fade with `p` would do to the active document. `Err` when Fade isn't available
/// or `p` is invalid.
pub fn fade_preview(s: &Session, p: &Value) -> Result<FadePreview> {
    can_fade(s).map_err(|e| bad("edit.fade", e))?;
    let job = fade_job(s, p)?;
    let doc = Document::clone(&s.active().ok_or(EngineError::NoDocument)?.doc);
    Ok(FadePreview { doc, job })
}
