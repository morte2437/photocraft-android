use super::content_aware::channel_mask;
use super::*;

// ------------------------------------------------------------------ Content-Aware Scale

/// Reach of Content-Aware Scale's `x`/`y`: the largest document side.
const CAS_POSITION_LIMIT: f64 = 300_000.0;

/// What Content-Aware Scale does, read from its params against the active document: the
/// command and the transform box's live preview ([`ContentAwareScalePreview`]) share it.
struct CasPlan {
    doc: std::sync::Arc<Document>,
    layer: LayerId,
    /// The area scaled: the selection's bounds, else the layer's content.
    region: Rect,
    /// Where the result goes: `x`, `y` (default: the region's corner), `width`, `height`.
    dst: Rect,
    amount: f32,
    protect: Option<Surface>,
    skin_tones: bool,
}

fn cas_plan(s: &Session, p: &Value) -> Result<CasPlan> {
    let cmd = "edit.contentAwareScale";
    let id = pixel_layer(s).map_err(EngineError::Other)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let surf = doc.layer(id).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(id))?;
    let region = match &doc.selection {
        Some(sel) => sel.content_bounds().intersect(&doc.bounds()),
        None => surf.content_bounds(),
    };
    if region.is_empty() {
        return Err(EngineError::Other("nothing to scale: the layer is empty".into()));
    }
    let (w, h) = (region.width() as usize, region.height() as usize);
    let dim = |k: &str, pct: &str, cur: usize| -> Result<u32> {
        let v = match (p.get(k).and_then(Value::as_f64), p.get(pct).and_then(Value::as_f64)) {
            (Some(px), _) => px,
            (None, Some(pc)) => cur as f64 * pc / 100.0,
            (None, None) => cur as f64,
        };
        if !(1.0..=60_000.0).contains(&v) {
            return Err(bad(cmd, format!("`{k}` must be 1..60000 px")));
        }
        Ok(v.round() as u32)
    };
    let (nw, nh) = (dim("width", "scaleX", w)?, dim("height", "scaleY", h)?);
    let at = |k: &str, cur: i32| -> Result<i32> {
        match p.get(k).filter(|v| !v.is_null()) {
            None => Ok(cur),
            Some(v) => match v.as_f64() {
                Some(x) if (-CAS_POSITION_LIMIT..=CAS_POSITION_LIMIT).contains(&x) => Ok(x.round() as i32),
                _ => Err(bad(cmd, format!("`{k}` must be a number of px within ±300000"))),
            },
        }
    };
    let dst = Rect::from_xywh(at("x", region.x0)?, at("y", region.y0)?, nw, nh);
    let amount = f32_or(p, "amount", 100.0).clamp(0.0, 100.0) / 100.0;
    // Protection: an alpha channel and/or skin tones.
    let protect = match p.get("protect").filter(|v| !v.is_null() && v.as_str() != Some("none")) {
        Some(v) => Some(channel_mask(&doc, v).cloned().ok_or_else(|| bad(cmd, "`protect` must name an alpha channel (index or name) or \"none\""))?),
        None => None,
    };
    let skin_tones = bool_or(p, "protectSkinTones", false);
    Ok(CasPlan { doc, layer: id, region, dst, amount: if amount.is_finite() { amount } else { 1.0 }, protect, skin_tones })
}

/// The scaled pixels: `img` (`w`×`h`, `fmt`) seam-carved to `nw`×`nh` where the protection
/// (`protect`, one value per pixel, and the skin tones when asked) steers the seams, blended with
/// a plain resample by `amount` (0..1). Checks `ctx` for cancellation before every seam.
#[allow(clippy::too_many_arguments)]
fn cas_pixels(
    ctx: &crate::jobs::JobCtx,
    label: &str,
    img: &[f32],
    (w, h): (usize, usize),
    fmt: PixelFormat,
    mut protect: Option<Vec<f32>>,
    skin_tones: bool,
    (nw, nh): (usize, usize),
    amount: f32,
) -> Result<Vec<f32>> {
    use photocraft_algo::seam::{carve_with, resize_bilinear, skin_mask};
    let n = fmt.channels();
    if skin_tones {
        let rgba: Vec<f32> = img.chunks_exact(n).flat_map(|px| to_rgba(&fmt, px)).collect();
        let skin = skin_mask(w, h, 4, &rgba);
        protect = Some(match protect {
            Some(pm) => pm.iter().zip(&skin).map(|(a, b)| a.max(*b)).collect(),
            None => skin,
        });
    }
    ctx.check()?;
    let carved = if amount > 0.0 {
        ctx.stage(0.05, 0.95, label, |ctl| carve_with(w, h, n, img, protect.as_deref(), nw, nh, ctl)).map_err(|_| EngineError::Cancelled)?
    } else {
        Vec::new()
    };
    let plain = if amount < 1.0 { resize_bilinear(w, h, n, img, nw, nh) } else { Vec::new() };
    Ok(match (carved.is_empty(), plain.is_empty()) {
        (false, true) => carved,
        (true, false) => plain,
        _ => carved.iter().zip(&plain).map(|(c, l)| l + (c - l) * amount).collect::<Vec<f32>>(),
    })
}

pub(super) fn content_aware_scale(s: &mut Session, p: &Value) -> Result<Value> {
    let CasPlan { doc, layer: id, region, dst, amount, protect: protect_mask, skin_tones } = cas_plan(s, p)?;
    let (w, h) = (region.width() as usize, region.height() as usize);
    let (nw, nh) = (dst.width() as usize, dst.height() as usize);
    let label = "Content-Aware Scale";
    // A background job when started with `Session::start` (#210): seam carving runs on a worker
    // against the document snapshot and checks for cancellation before every seam.
    crate::jobs::run(
        s,
        label,
        true,
        move |ctx| {
            ctx.progress(0.0, label);
            let surf = doc.layer(id).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(id))?;
            let img = surf.read_region(region);
            let protect: Option<Vec<f32>> =
                protect_mask.map(|m| (0..w * h).map(|i| m.sample_channel(region.x0 + (i % w) as i32, region.y0 + (i / w) as i32, 0)).collect());
            cas_pixels(ctx, label, &img, (w, h), surf.format(), protect, skin_tones, (nw, nh), amount)
        },
        move |s, out| {
            s.edit(label, |doc, _| {
                let surf = writable_surface(doc, id)?;
                crate::pixels::try_clear_surface(surf, region, None)?;
                surf.write_region(dst, &out);
                surf.prune();
                Ok(())
            })?;
            Ok(json!({"from": [w, h], "to": [nw, nh], "bounds": [dst.x0, dst.y0, nw, nh]}))
        },
    )
}

/// Content-Aware Scale on a reduced copy, for the transform box's live preview (#3096): the
/// command's own carving, on the area scaled down so the longer of its sides before and after
/// is at most `max_side` px. Prepared from the session ([`ContentAwareScalePreview::new`]) and
/// [`run`](ContentAwareScalePreview::run) on any thread.
pub struct ContentAwareScalePreview {
    plan: CasPlan,
    max_side: u32,
}

impl ContentAwareScalePreview {
    /// `p` takes `edit.contentAwareScale`'s params; the same ones fail the same way.
    pub fn new(s: &Session, p: &Value, max_side: u32) -> Result<Self> {
        Ok(Self { plan: cas_plan(s, p)?, max_side: max_side.max(1) })
    }

    /// The area scaled, in document px.
    pub fn region(&self) -> Rect {
        self.plan.region
    }

    /// The reduced sizes of the area before and after: `[w, h, new w, new h]`.
    fn reduced(&self) -> [usize; 4] {
        let (region, dst) = (self.plan.region, self.plan.dst);
        let (w, h, nw, nh) = (region.width(), region.height(), dst.width(), dst.height());
        let k = (f64::from(w.max(h).max(nw).max(nh)) / f64::from(self.max_side)).max(1.0);
        let side = |v: u32| ((f64::from(v) / k).round() as usize).max(1);
        [side(w), side(h), side(nw), side(nh)]
    }

    /// The size of [`run`](Self::run)'s result, px.
    pub fn size(&self) -> [u32; 2] {
        let [_, _, w, h] = self.reduced();
        [w as u32, h as u32]
    }

    /// The scaled pixels at the reduced size ([`size`](Self::size)), in the layer's format, as a
    /// surface with its corner at the origin. Checks `ctx` for cancellation before every seam.
    pub fn run(&self, ctx: &crate::jobs::JobCtx) -> Result<Surface> {
        let CasPlan { doc, layer, region, dst: _, amount, protect, skin_tones } = &self.plan;
        let surf = doc.layer(*layer).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(*layer))?;
        let fmt = surf.format();
        let n = fmt.channels();
        let (w, h) = (region.width(), region.height());
        let [pw, ph, pnw, pnh] = self.reduced();
        // Nearest samples of the area (and of the protecting channel) at the reduced size.
        let src_x: Vec<i32> = (0..pw).map(|i| region.x0 + (((i as f64 + 0.5) * f64::from(w) / pw as f64) as i32).min(w as i32 - 1)).collect();
        let src_y = |j: usize| region.y0 + (((j as f64 + 0.5) * f64::from(h) / ph as f64) as i32).min(h as i32 - 1);
        let mut img = Vec::with_capacity(pw * ph * n);
        for j in 0..ph {
            ctx.check()?;
            let row = surf.read_region(Rect::from_xywh(region.x0, src_y(j), w, 1));
            for &x in &src_x {
                let at = (x - region.x0) as usize * n;
                img.extend_from_slice(row.get(at..at + n).ok_or_else(|| EngineError::Other("preview read out of bounds".into()))?);
            }
        }
        let protect = protect.as_ref().map(|m| (0..pw * ph).map(|i| m.sample_channel(src_x[i % pw], src_y(i / pw), 0)).collect());
        let out = cas_pixels(ctx, "Content-Aware Scale", &img, (pw, ph), fmt, protect, *skin_tones, (pnw, pnh), *amount)?;
        let mut s = Surface::new(fmt);
        s.write_region(Rect::from_xywh(0, 0, pnw as u32, pnh as u32), &out);
        Ok(s)
    }
}
