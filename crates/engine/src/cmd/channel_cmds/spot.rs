use super::manage::{fix_view_now, gray_document};
use super::planes::{channel_format, edit_mode, plane_surface, read_plane, tone};
use super::refs::{ChanRef, alpha_index, bad, gray_plane, native_pixels};
use super::view::{ChannelTarget, ChannelView, color_names};
use super::{AlphaChannel, ColorMode, DocState, Document, EngineError, Layer, Result, Session, Value, from_rgba, json};

/// Merge Spot Channel: flattens the image and prints the spot ink into the colour channels.
pub(super) fn merge_spot(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "channel.mergeSpot";
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let i = match p.get("channel") {
        Some(_) => alpha_index(doc, p, "channel", cmd)?,
        None => match s.active().ok_or(EngineError::NoDocument)?.channel_view.target {
            ChannelTarget::Alpha(i) => i,
            _ => doc.channels.iter().position(|c| c.spot.is_some()).ok_or_else(|| bad(cmd, "no spot channel"))?,
        },
    };
    let (ink, solidity) = doc.channels[i].spot.ok_or_else(|| bad(cmd, "not a spot channel"))?;
    s.edit("Merge Spot Channel", |doc, active| {
        let area = doc.bounds();
        let fmt = doc.pixel_format();
        let comp = photocraft_compose::render(doc, area);
        let k = read_plane(&doc.channels[i].surface, area);
        let ink = ink.to_rgb();
        // Ink over the image: opaque at 100% solidity, multiplied (transparent ink) at 0%.
        let data: Vec<f32> = comp
            .px
            .iter()
            .zip(&k)
            .flat_map(|(c, &v)| {
                let rgb: [f32; 3] = std::array::from_fn(|j| {
                    let over = solidity * ink[j] + (1.0 - solidity) * c[j] * ink[j];
                    c[j] + (over - c[j]) * v.clamp(0.0, 1.0)
                });
                from_rgba(&fmt, [rgb[0], rgb[1], rgb[2], 1.0])
            })
            .collect();
        let mut bg = Layer::raster("Background", fmt);
        bg.locks.transparency = true;
        bg.locks.position = true;
        if let Some(sf) = bg.surface_mut() {
            sf.write_region(area, &data);
            sf.prune();
        }
        let id = bg.id;
        doc.layers = vec![bg];
        doc.channels.remove(i);
        *active = Some(id);
        Ok(())
    })?;
    if let Some(st) = s.active_mut() {
        st.channel_view = ChannelView::default();
        fix_view_now(st);
    }
    Ok(Value::Null)
}

/// Split Channels: one grayscale document per colour and alpha channel; closes the original.
pub(super) fn split(s: &mut Session, p: &Value) -> Result<Value> {
    let idx = s.active_index().ok_or(EngineError::NoDocument)?;
    let doc = s.documents()[idx].doc.clone();
    let area = doc.bounds();
    let (px, n) = native_pixels(&doc, None)?;
    let mode = edit_mode(&doc);
    let names = color_names(mode);
    let short = |name: &str| name.chars().next().map(|c| c.to_ascii_uppercase().to_string()).unwrap_or_default();
    let mut docs: Vec<Document> = Vec::new();
    for (c, name) in names.iter().enumerate() {
        let plane: Vec<f32> = px.chunks_exact(n).map(|p| tone(mode, p[c])).collect();
        docs.push(gray_document(&format!("{}_{}", doc.name, short(name)), &doc, &plane));
    }
    for ch in &doc.channels {
        docs.push(gray_document(&format!("{}_{}", doc.name, ch.name), &doc, &read_plane(&ch.surface, area)));
    }
    if p.get("closeOriginal").and_then(Value::as_bool).unwrap_or(true) {
        s.close(idx);
    }
    let made: Vec<usize> = docs.into_iter().map(|d| s.add_document(d, None)).collect();
    Ok(json!({ "documents": made }))
}

/// Open grayscale documents with the active document's size, active first.
pub(super) fn mergeable(s: &Session) -> Vec<usize> {
    let Some(a) = s.active() else { return Vec::new() };
    let ok = |d: &DocState| d.doc.mode == ColorMode::Grayscale && d.doc.size == a.doc.size;
    let mut v: Vec<usize> = s.active_index().into_iter().filter(|_| ok(a)).collect();
    v.extend(s.documents().iter().enumerate().filter(|(i, d)| Some(*i) != s.active_index() && ok(d)).map(|(i, _)| i));
    v
}

/// Merge Channels: grayscale documents of one size become the channels of an RGB, CMYK or Lab
/// document (extra documents become alpha channels); the sources close.
pub(super) fn merge(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "channel.merge";
    let mode = match p.get("mode").and_then(Value::as_str).unwrap_or("rgb") {
        "rgb" => ColorMode::Rgb,
        "cmyk" => ColorMode::Cmyk,
        "lab" => ColorMode::Lab,
        other => return Err(bad(cmd, format!("unknown mode `{other}`"))),
    };
    let srcs: Vec<usize> = match p.get("documents").and_then(Value::as_array) {
        Some(a) => a
            .iter()
            .map(|v| v.as_u64().map(|i| i as usize).filter(|i| *i < s.documents().len()).ok_or_else(|| bad(cmd, "bad document index")))
            .collect::<Result<_>>()?,
        None => mergeable(s).into_iter().take(mode.color_channels()).collect(),
    };
    if srcs.len() < mode.color_channels() {
        return Err(bad(cmd, format!("{mode:?} needs {} grayscale documents, got {}", mode.color_channels(), srcs.len())));
    }
    // Each source closes by its *current* index, so a repeated index would shift the later
    // removals onto a document that was never named (#934). Refuse it rather than guess.
    let mut seen = std::collections::HashSet::new();
    if let Some(dup) = srcs.iter().find(|i| !seen.insert(**i)) {
        return Err(bad(cmd, format!("document {dup} is listed more than once")));
    }
    let Some(first) = srcs.first().and_then(|&i| s.documents().get(i)).map(|d| d.doc.clone()) else {
        return Err(bad(cmd, "bad document index"));
    };
    let mut planes = Vec::new();
    for &i in &srcs {
        let d = &s.documents()[i].doc;
        if d.size != first.size || d.mode != ColorMode::Grayscale {
            return Err(bad(cmd, format!("\"{}\" is not a grayscale document of the same size", d.name)));
        }
        planes.push(gray_plane(d, None, None, ChanRef::Composite)?);
    }
    let mut out = Document::new(p.get("name").and_then(Value::as_str).unwrap_or("Untitled"), first.size, mode, first.depth);
    out.resolution_dpi = first.resolution_dpi;
    let fmt = out.pixel_format();
    let cc = mode.color_channels();
    let npx = planes[0].len();
    let mut data = Vec::with_capacity(npx * fmt.channels());
    for i in 0..npx {
        for pl in &planes[..cc] {
            data.push(tone(mode, pl[i]).clamp(0.0, 1.0));
        }
        data.push(1.0);
    }
    let mut bg = Layer::raster("Background", fmt);
    bg.locks.transparency = true;
    bg.locks.position = true;
    if let Some(sf) = bg.surface_mut() {
        sf.write_region(out.bounds(), &data);
        sf.prune();
    }
    out.layers.push(bg);
    let area = out.bounds();
    let cf = channel_format(&out);
    for (k, pl) in planes[cc..].iter().enumerate() {
        out.channels.push(AlphaChannel::new(format!("Alpha {}", k + 1), plane_surface(pl, area, cf)));
    }
    let mut close: Vec<usize> = srcs.clone();
    close.sort_unstable();
    for i in close.into_iter().rev() {
        s.close(i);
    }
    let i = s.add_document(out, None);
    Ok(json!({ "document": i }))
}
