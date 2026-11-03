use super::{Document, Footprint, LayerId, PixelFormat, Rect, Region, Result, Surface, Value, bad, flag, from_rgba_into, string, targets_pixels};

// ---------------------------------------------------------------------------------------------
// Sampling sources
// ---------------------------------------------------------------------------------------------

/// Which pixels Clone Stamp / Healing Brush sample.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SampleLayers {
    Current,
    CurrentAndBelow,
    All,
}

pub(super) fn sample_layers(p: &Value, cmd: &str) -> Result<SampleLayers> {
    match string(p, "sampleLayer", "current") {
        "current" => Ok(SampleLayers::Current),
        "currentAndBelow" => Ok(SampleLayers::CurrentAndBelow),
        "all" => Ok(SampleLayers::All),
        o => Err(bad(cmd, format!("unknown sampleLayer `{o}` (current|currentAndBelow|all)"))),
    }
}

/// Composite pixels of `rect` in `fmt` (native channels): the whole document, or the target layer
/// and everything below it.
pub(crate) fn composite_region(pre: &Document, id: Option<LayerId>, which: SampleLayers, rect: Rect, fmt: PixelFormat) -> Region {
    let mut doc = pre.clone();
    if which == SampleLayers::CurrentAndBelow
        && let Some(id) = id
    {
        // Hide everything composited after (above) the target in the bottom-to-top walk.
        let walk: Vec<(Vec<usize>, LayerId)> = pre.walk().into_iter().map(|(p, _, l)| (p, l.id)).collect();
        if let Some(pos) = walk.iter().position(|(_, l)| *l == id) {
            for (path, _) in &walk[pos + 1..] {
                if let Some(l) = doc.layer_at_mut(path) {
                    l.visible = false;
                }
            }
        }
    }
    let buf = photocraft_compose::render(&doc, rect);
    let n = fmt.channels();
    let mut out = Region::new(rect, n);
    let mut enc = [0.0f32; 8];
    for (i, px) in buf.px.iter().enumerate() {
        from_rgba_into(&fmt, *px, &mut enc);
        out.data[i * n..(i + 1) * n].copy_from_slice(&enc[..n]);
    }
    out
}

/// Pixels to sample at `rect` (document coordinates, already offset to the source position).
pub(super) fn sample(pre: &Document, id: Option<LayerId>, surf: &Surface, which: SampleLayers, rect: Rect) -> Region {
    match which {
        SampleLayers::Current => Region::read(surf, rect),
        _ => composite_region(pre, id, which, rect, surf.format()),
    }
}

/// "Sample All Layers" for Spot Healing, Blur, Sharpen and Smudge (layer pixels only: a mask or
/// channel has nothing to sample from other layers).
pub(super) fn sample_all_layers(p: &Value) -> bool {
    flag(p, "sampleAllLayers", false) && targets_pixels(p)
}

/// Sample All Layers for the sequential-dab tools: each dab's effect runs on a copy of the visible
/// composite (kept across dabs, so a dab sees the earlier ones), and the result is laid onto the
/// layer under the dab. An empty layer above the image so picks up the blurred or smudged picture,
/// and pixels the effect didn't change look as before (they are what was already visible).
pub(super) struct AllLayers<'a> {
    pre: &'a Document,
    fmt: PixelFormat,
    visible: Option<Region>,
}

impl<'a> AllLayers<'a> {
    pub(super) fn new(pre: &'a Document, fmt: PixelFormat) -> Self {
        Self { pre, fmt, visible: None }
    }

    pub(super) fn dab(&mut self, work: &mut Region, fp: &Footprint, effect: impl FnOnce(&mut Region, &Footprint)) {
        let (pre, fmt) = (self.pre, self.fmt);
        let visible = self.visible.get_or_insert_with(|| composite_region(pre, None, SampleLayers::All, work.rect, fmt));
        if visible.rect != work.rect || visible.ch != work.ch {
            return effect(work, fp);
        }
        effect(visible, fp);
        let r = fp.rect.intersect(&work.rect);
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                if fp.at(x, y) > 0.0 {
                    work.px_mut(x, y).copy_from_slice(visible.px(x, y));
                }
            }
        }
    }
}
