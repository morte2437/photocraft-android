//! Edit › Auto-Align Layers and Auto-Blend Layers, on the selected pixel layers.
//!
//! Both share Photomerge's registration and blending ([`photocraft_algo::panorama`] via
//! `photo_cmds`). Auto-Align matches Harris / steered-BRIEF features between every pair of
//! layers, chains the verified pairs from the reference layer (maximum spanning tree) and
//! bundle-adjusts all of them, then warps each layer. Layers that nearly coincide (a focus or
//! exposure bracket) have nothing to match and register by intensity instead
//! ([`photocraft_algo::direct`], `registration`: auto falls back to it, or ask for it). Projections: Auto, Perspective
//! (homography), Cylindrical and Spherical (rigid motion on the cylinder / sphere, focal length
//! from the homographies or EXIF), Collage (similarity) and Reposition (translation). The
//! reference stays put in the planar projections.
//!
//! Auto-Blend gives each selected layer a mask so the composite shows, per pixel, the sharpest
//! layer (Stack Images, for focus stacking) or one layer per region with seams routed where the
//! layers agree (Panorama). With Seamless Tones and Colors it also adds a merged layer: the
//! panorama blended with Laplacian pyramids so transitions are invisible, the stack fused on the
//! Laplacian pyramid, each coefficient from the layer sharpest at that scale
//! ([`pyramid::fuse_stack`]; `haloControl` keeps a bright object's defocused glow out of the
//! coarse levels).
//! Stack Images can also add a hidden Depth layer, the layer each pixel is sharpest in by depth
//! from focus ([`photocraft_algo::dff`]): black = the bottom layer, white = the top, so that once
//! on a layer mask it drives Lens Blur's depth map.

use photocraft_algo::panorama::Layout;
use photocraft_algo::pyramid;
use photocraft_algo::transform::Interp;
use photocraft_algo::warp::warp_mesh_gray;
use photocraft_doc::{Document, Layer, LayerContent, LayerId, LayerMask};
use photocraft_geom::Rect;
use photocraft_raster::{Surface, to_rgba};
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::photo_cmds::{NO_DETAIL, plane_matrix, register, register_direct, seam_blend, seam_order, warp_placed};
use crate::{EngineError, Result, Session};

fn bad(cmd: &str, msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: cmd.into(), msg: msg.into() }
}

/// The selected raster layers, bottom to top.
fn selected_pixel_layers(s: &Session) -> Vec<LayerId> {
    let Some(st) = s.active() else { return Vec::new() };
    st.selected_layers().into_iter().filter(|id| st.doc.layer(*id).is_some_and(|l| matches!(l.content, LayerContent::Raster(_)))).collect()
}

/// Whether a lock keeps the layer `id` from being moved or warped: lock all, position or pixels.
/// The Background's own position lock doesn't count: like Free Transform, Auto-Align turns the
/// Background into a normal layer when it has to move it (Photoshop).
fn pinned(doc: &Document, id: LayerId) -> bool {
    let l = doc.effective_locks(id);
    l.all || l.pixels || (l.position && !doc.is_background(id))
}

fn two_layers(s: &Session) -> std::result::Result<(), String> {
    s.active().ok_or("no document open")?;
    if selected_pixel_layers(s).len() < 2 { Err("select two or more pixel layers in the Layers panel".into()) } else { Ok(()) }
}

fn auto_align(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.autoAlignLayers";
    let ids = selected_pixel_layers(s);
    if ids.len() < 2 {
        return Err(bad(cmd, "select two or more pixel layers"));
    }
    let projection = p.get("projection").and_then(Value::as_str).unwrap_or("auto");
    let layout = Layout::parse(projection)
        .ok_or_else(|| bad(cmd, format!("unknown projection `{projection}` (auto|perspective|cylindrical|spherical|collage|reposition)")))?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc = st.doc.clone();
    let reference = match p.get("reference").and_then(Value::as_u64) {
        Some(r) if ids.contains(&LayerId(r)) => LayerId(r),
        Some(r) => return Err(bad(cmd, format!("reference {r} is not a selected pixel layer"))),
        // As in Photoshop, a locked layer stays put: it becomes the reference (#7080).
        None => ids.iter().copied().find(|id| pinned(&doc, *id)).unwrap_or(ids[0]),
    };
    let ref_idx = ids.iter().position(|i| *i == reference).unwrap_or(0);
    let area = doc.bounds();
    let surfs: Vec<&Surface> = ids.iter().map(|id| doc.layer(*id).and_then(|l| l.surface()).ok_or(EngineError::NoLayer(*id))).collect::<Result<_>>()?;
    let images: Vec<(&Surface, Rect)> = surfs.iter().map(|s| (*s, area)).collect();
    let focal35 = doc.metadata.exif.as_ref().and_then(|e| photocraft_algo::exif::read(e).focal_length_35mm);
    let geometric = p.get("geometricCorrection").and_then(Value::as_bool).unwrap_or(false);
    let registration = p.get("registration").and_then(Value::as_str).unwrap_or("auto");
    if !matches!(registration, "auto" | "features" | "intensity") {
        return Err(bad(cmd, "`registration` must be auto|features|intensity"));
    }
    let no_detail = || EngineError::Other(NO_DETAIL.into());
    // Features first (a panorama); layers that nearly coincide (a focus or exposure bracket) have
    // nothing to match, and register by intensity instead.
    let by_features = (registration != "intensity").then(|| register(&images, layout, Some(ref_idx), geometric, focal35)).flatten();
    let (al, used) = match (by_features, registration) {
        (Some(al), _) => (al, "features"),
        (None, "features") => return Err(no_detail()),
        (None, _) => (
            register_direct(&images, layout, ref_idx).map_err(|m| if registration == "intensity" && m != NO_DETAIL { bad(cmd, m) } else { no_detail() })?,
            "intensity",
        ),
    };
    let interp = Interp::parse(p.get("interpolation").and_then(Value::as_str).unwrap_or("bicubic"));
    let planar = matches!(al.layout, Layout::Perspective | Layout::Collage | Layout::Reposition);
    let mut inliers = vec![0usize; ids.len()];
    for &(a, b, n) in &al.pairs {
        inliers[a] += n;
        inliers[b] += n;
    }
    // Everything but the reference moves in planar layouts; every layer is reprojected otherwise.
    let moves: Vec<usize> = (0..ids.len()).filter(|&i| al.placements[i].is_some() && (i != ref_idx || !planar)).collect();
    let failed: Vec<u64> = (0..ids.len()).filter(|&i| al.placements[i].is_none()).map(|i| ids[i].0).collect();
    if moves.is_empty() {
        return Err(EngineError::Other("Auto-Align couldn't find enough matching detail between the layers".into()));
    }
    if let Some(l) = moves.iter().find(|&&i| pinned(&doc, ids[i])).and_then(|&i| doc.layer(ids[i])) {
        return Err(EngineError::Other(format!("Auto-Align can't move the locked layer \"{}\"", l.name)));
    }
    let warped: Vec<(LayerId, Surface, Option<Surface>)> = moves
        .iter()
        .map(|&i| {
            let pl = al.placements[i].as_ref().ok_or_else(|| EngineError::Other("Auto-Align lost a layer's placement".into()))?;
            let l = doc.layer(ids[i]).ok_or(EngineError::NoLayer(ids[i]))?;
            let surf = l.surface().ok_or(EngineError::NoLayer(ids[i]))?;
            let b = surf.content_bounds();
            let px = if b.is_empty() { surf.clone() } else { warp_placed(surf, b, (0.0, 0.0), pl, (0.0, 0.0), interp)? };
            // Masks warp as masks: one grey channel whose outside keeps its default (reveal-all
            // stays revealed), not as colour, which would come back grey + alpha reading 0 outside.
            let mask = match l.mask.as_ref() {
                Some(m) => Some(match plane_matrix(pl, (0.0, 0.0)) {
                    Some(h) => crate::transform_cmds::warp_gray(&m.surface, &h, Interp::Bilinear)?,
                    None => warp_mesh_gray(&m.surface, &|x, y| pl.forward(x, y), Interp::Bilinear),
                }),
                None => None,
            };
            Ok((ids[i], px, mask))
        })
        .collect::<Result<_>>()?;
    s.edit("Auto-Align Layers", |doc, _| {
        for (id, px, mask) in &warped {
            // Photoshop turns the Background into a normal layer before moving it.
            crate::extra_cmds::background_to_layer(doc, *id);
            let l = doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?;
            if let Some(surf) = l.surface_mut() {
                *surf = px.clone();
            }
            if let (Some(m), Some(ms)) = (&mut l.mask, mask) {
                m.surface = ms.clone();
            }
        }
        Ok(())
    })?;
    let model = match al.layout {
        Layout::Reposition => "reposition",
        Layout::Collage => "collage",
        Layout::Perspective | Layout::Auto => "perspective",
        Layout::Cylindrical => "cylindrical",
        Layout::Spherical => "spherical",
    };
    let aligned: Vec<Value> = moves
        .iter()
        .filter_map(|&i| {
            let pl = al.placements[i].as_ref()?;
            Some(json!({"layer": ids[i].0, "model": model, "matches": inliers[i], "matrix": plane_matrix(pl, (0.0, 0.0)).map(|h| h.0.to_vec())}))
        })
        .collect();
    Ok(
        json!({"reference": reference.0, "layout": al.layout.name(), "registration": used, "focal": al.focal, "rms": al.rms, "aligned": aligned, "failed": failed}),
    )
}

/// The most memory Auto-Blend's working buffers may take past the canvas: a bound independent
/// of the layers' own bounding boxes, which a sparse layer (a dot at each far corner) can make
/// arbitrarily large, and of how many layers are blended.
const MAX_BLEND_BYTES: u64 = 4 << 30;

/// What Auto-Blend's buffers hold per work-area pixel, as f32: each layer's pixels, luminance,
/// alpha and mask weight, plus the merged picture and the depth map.
fn blend_bytes_per_pixel(layers: u64, channels: u64) -> u64 {
    layers.saturating_mul(channels.saturating_add(3)).saturating_add(channels.saturating_mul(2)).saturating_mul(4)
}

/// Auto-Blend's buffers, for allocation errors.
const BLEND_BUFFERS: &str = "Auto-Blend Layers";

fn auto_blend(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "edit.autoBlendLayers";
    let ids = selected_pixel_layers(s);
    if ids.len() < 2 {
        return Err(bad(cmd, "select two or more pixel layers"));
    }
    let method = p.get("method").and_then(Value::as_str).unwrap_or("panorama");
    if !matches!(method, "panorama" | "stack") {
        return Err(bad(cmd, "`method` must be panorama|stack"));
    }
    let seamless = p.get("seamlessTones").and_then(Value::as_bool).unwrap_or(true);
    let halo = match p.get("haloControl") {
        None | Some(Value::Null) => pyramid::StackFusion::default().halo,
        Some(v) => match v.as_f64() {
            Some(h) if (0.0..=f64::from(pyramid::HALO_MAX)).contains(&h) => h as f32,
            _ => return Err(bad(cmd, format!("`haloControl` must be a number from 0 to {}", pyramid::HALO_MAX))),
        },
    };
    let want_depth = p.get("depthMap").and_then(Value::as_bool).unwrap_or(false);
    if want_depth && method != "stack" {
        return Err(bad(cmd, "`depthMap` needs `method`: stack"));
    }
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let doc: std::sync::Arc<Document> = st.doc.clone();
    let mut union = Rect::EMPTY;
    let mut content_px = 0u64;
    let px_of = |r: &Rect| u64::from(r.width()) * u64::from(r.height());
    for id in &ids {
        if let Some(b) = doc.layer(*id).and_then(|l| l.surface()).map(|s| s.content_bounds()) {
            union = union.union(&b);
            content_px = content_px.saturating_add(px_of(&b));
        }
    }
    // The whole layers, past the canvas too (#5745): Auto-Align leaves a panorama's layers
    // partly off the canvas, and Photoshop masks and blends all of each layer, so Reveal All
    // shows the whole panorama. Layers so far apart that their bounds are mostly empty, or
    // whose buffers would exceed `MAX_BLEND_BYTES`, keep to the canvas.
    let canvas = doc.bounds();
    let channels = ids.iter().filter_map(|id| doc.layer(*id).and_then(|l| l.surface()).map(|s| s.channels() as u64)).max().unwrap_or(4);
    let per_px = blend_bytes_per_pixel(ids.len() as u64, channels);
    let fits = px_of(&union).saturating_mul(per_px) <= MAX_BLEND_BYTES && px_of(&union) <= content_px.max(px_of(&canvas)).saturating_mul(2);
    let area = if fits { union } else { union.intersect(&canvas) };
    if area.is_empty() {
        return Err(EngineError::Other("the selected layers are empty".into()));
    }
    let (w, h) = (area.width() as usize, area.height() as usize);
    let mut imgs: Vec<Vec<f32>> = Vec::new();
    let mut luma: Vec<Vec<f32>> = Vec::new();
    let mut alpha: Vec<Vec<f32>> = Vec::new();
    let mut fmt = None;
    for id in &ids {
        let l = doc.layer(*id).ok_or(EngineError::NoLayer(*id))?;
        let surf = l.surface().ok_or(EngineError::NoLayer(*id))?;
        let (px, f) = (crate::allocation::read_region(surf, area, BLEND_BUFFERS)?, surf.format());
        let n = f.channels();
        let (mut lv, mut av) = (crate::allocation::filled(w * h, 0.0f32, BLEND_BUFFERS)?, crate::allocation::filled(w * h, 0.0f32, BLEND_BUFFERS)?);
        for i in 0..w * h {
            let c = to_rgba(&f, &px[i * n..(i + 1) * n]);
            lv[i] = 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
            av[i] = c[3];
        }
        if fmt.is_some_and(|g: photocraft_color::PixelFormat| g != f) {
            return Err(EngineError::Other("the layers have different pixel formats".into()));
        }
        fmt = Some(f);
        imgs.push(px);
        luma.push(lv);
        alpha.push(av);
    }
    let fmt = fmt.ok_or(EngineError::NoDocument)?;
    let (weights, blended) = if method == "stack" {
        let weights = pyramid::stack_weights(w, h, &luma, &alpha);
        let blended = seamless.then(|| {
            let refs: Vec<&[f32]> = imgs.iter().map(Vec::as_slice).collect();
            let p = pyramid::StackFusion { halo, ..Default::default() };
            pyramid::fuse_stack(w, h, fmt.channels(), &refs, &luma, &alpha, p)
        });
        (weights, blended)
    } else {
        // Panorama: seams routed through agreement, multi-band blended (photo_cmds).
        let surfs: Vec<Surface> =
            ids.iter().map(|id| doc.layer(*id).and_then(|l| l.surface()).cloned().ok_or(EngineError::NoLayer(*id))).collect::<Result<_>>()?;
        let order = seam_order(&surfs, 0);
        // Seamless Tones and Colors also evens out exposure and colour between the layers (gain
        // compensation over their overlaps), as in Photomerge.
        let b = seam_blend(&surfs, area, &order, seamless, seamless, None);
        let weights: Vec<Vec<f32>> = b.layers.iter().map(|(_, m)| m.read_region(area)).collect();
        let blended = seamless.then(|| b.composite.read_region(area));
        (weights, blended)
    };
    // The depth map: the layer each pixel is sharpest in, from the focus volume guided by the
    // merged picture (or, without one, by the per-pixel pick), as a hidden gray layer, black =
    // the bottom layer, white = the top, ready for Lens Blur's depth map once on a mask.
    let depth_px = want_depth.then(|| {
        let n = fmt.channels();
        let guide: Vec<f32> = match &blended {
            Some(px) => (0..w * h)
                .map(|i| {
                    let c = to_rgba(&fmt, &px[i * n..(i + 1) * n]);
                    0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2]
                })
                .collect(),
            None => (0..w * h).map(|i| luma.iter().zip(&weights).map(|(l, wt)| l[i] * wt[i]).sum()).collect(),
        };
        let refs: Vec<&[f32]> = luma.iter().map(Vec::as_slice).collect();
        let dm = photocraft_algo::dff::depth_from_focus(&refs, &guide, w, h, &photocraft_algo::dff::DepthParams::default());
        let scale = 1.0 / (ids.len() - 1).max(1) as f32;
        let mut out = vec![0.0f32; w * h * n];
        for (i, d) in dm.depth.iter().enumerate() {
            let g = (d * scale).clamp(0.0, 1.0);
            photocraft_raster::from_rgba_into(&fmt, [g, g, g, 1.0], &mut out[i * n..(i + 1) * n]);
        }
        out
    });
    let top = *ids.last().ok_or_else(|| EngineError::Other("Auto-Blend needs two or more layers".into()))?;
    let label = "Auto-Blend Layers";
    let (new_layer, depth_layer) = s.edit(label, |doc, active| {
        for (id, wt) in ids.iter().zip(&weights) {
            // The Background can't have a mask: it becomes a normal layer first, as in Photoshop.
            crate::extra_cmds::background_to_layer_for_mask(doc, *id);
            let l = doc.layer_mut(*id).ok_or(EngineError::NoLayer(*id))?;
            let mut mask = LayerMask::hide_all();
            mask.surface.write_region(area, wt);
            mask.surface.prune();
            l.mask = Some(mask);
        }
        let mut below = top;
        let depth_id = depth_px.as_ref().map(|px| {
            let mut l = Layer::raster(doc.next_layer_name("Depth"), fmt);
            l.visible = false;
            if let Some(surf) = l.surface_mut() {
                surf.write_region(area, px);
                surf.prune();
            }
            below = crate::layer_place::insert_new(doc, Some(below), l, crate::layer_place::Place::Above);
            below
        });
        let Some(px) = &blended else {
            if depth_id.is_some() {
                *active = depth_id;
            }
            return Ok((None, depth_id));
        };
        let mut l = Layer::raster(doc.next_layer_name("Blended"), fmt);
        if let Some(surf) = l.surface_mut() {
            surf.write_region(area, px);
            surf.prune();
        }
        let nid = crate::layer_place::insert_new(doc, Some(below), l, crate::layer_place::Place::Above);
        *active = Some(nid);
        Ok((Some(nid), depth_id))
    })?;
    Ok(json!({"method": method, "layers": ids.iter().map(|i| i.0).collect::<Vec<_>>(), "blended": new_layer.map(|l| l.0), "depth": depth_layer.map(|l| l.0)}))
}

macro_rules! spec {
    ($id:literal, $label:literal, $params:literal, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &["Edit"], shortcut: None, params: $params, enabled: two_layers, run: $run, journal: true }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        spec!(
            "edit.autoAlignLayers",
            "Auto-Align Layers…",
            r##"{"projection":"auto|perspective|cylindrical|spherical|collage|reposition","reference":layer id?=the locked selected layer, else the bottom selected layer,"geometricCorrection":bool=false,"interpolation":"bicubic|bilinear|nearest","registration":"auto|features|intensity"}"##,
            auto_align
        ),
        spec!(
            "edit.autoBlendLayers",
            "Auto-Blend Layers…",
            r##"{"method":"panorama|stack","seamlessTones":bool=true,"haloControl":0..8=2,"depthMap":bool=false}"##,
            auto_blend
        ),
    ]
}

#[cfg(test)]
mod tests;
