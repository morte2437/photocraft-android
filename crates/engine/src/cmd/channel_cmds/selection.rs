use super::planes::{channel_format, next_alpha_name, plane_surface, read_plane};
use super::refs::{ChanRef, bad, doc_index, parse_ref, source};
use super::view::{ChannelTarget, QuickMaskOptions};
use super::*;

pub(super) fn save_selection(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "select.saveSelection";
    let op = p.get("operation").and_then(Value::as_str).unwrap_or("new").to_string();
    if !matches!(op.as_str(), "new" | "replace" | "add" | "subtract" | "intersect") {
        return Err(bad(cmd, format!("unknown operation `{op}`")));
    }
    let di = doc_index(s, p, "document")?;
    let src = s.active().ok_or(EngineError::NoDocument)?;
    let area = src.doc.bounds();
    let selection = sel::mask_from_surface(src.doc.selection.as_ref(), area);
    if s.documents()[di].doc.size != src.doc.size {
        return Err(bad(cmd, "the destination document must have the same size"));
    }
    let channel = p.get("channel").cloned().unwrap_or(Value::Null);
    let name = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    let prev = s.active_index();
    s.set_active(di);
    let r = s.edit("Save Selection", |doc, _| {
        let fmt = channel_format(doc);
        let existing = match &channel {
            Value::Null => None,
            Value::String(x) if x == "new" => None,
            v => match parse_ref(v, doc) {
                Some(ChanRef::Alpha(i)) if i < doc.channels.len() => Some(i),
                _ => return Err(bad(cmd, "`channel` must be \"new\" or an existing alpha channel")),
            },
        };
        match (existing, op.as_str()) {
            (None, _) | (_, "new") => {
                let name = name.clone().unwrap_or_else(|| next_alpha_name(doc, "Alpha"));
                doc.channels.push(AlphaChannel::new(name, plane_surface(&selection, area, fmt)));
                Ok(doc.channels.len() - 1)
            }
            (Some(i), op) => {
                let old = read_plane(&doc.channels[i].surface, area);
                let vals: Vec<f32> = match op {
                    "add" => old.iter().zip(&selection).map(|(a, b)| a.max(*b)).collect(),
                    "subtract" => old.iter().zip(&selection).map(|(a, b)| (a - b).max(0.0)).collect(),
                    "intersect" => old.iter().zip(&selection).map(|(a, b)| a.min(*b)).collect(),
                    _ => selection.clone(),
                };
                doc.channels[i].surface = plane_surface(&vals, area, fmt);
                if let Some(n) = &name {
                    doc.channels[i].name = n.clone();
                }
                Ok(i)
            }
        }
    });
    if let Some(i) = prev {
        s.set_active(i);
    }
    let i = r?;
    let name = s.documents()[di].doc.channels[i].name.clone();
    Ok(json!({ "channel": i, "name": name, "document": di }))
}

pub(super) fn load_selection(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "select.loadSelection";
    let mode = match operation(p, "operation") {
        "new" | "replace" => SelectionMode::Replace,
        "add" => SelectionMode::Add,
        "subtract" => SelectionMode::Subtract,
        "intersect" => SelectionMode::Intersect,
        other => return Err(bad(cmd, format!("unknown operation `{other}`"))),
    };
    if p.get("channel").is_none() {
        return Err(bad(cmd, "missing `channel`"));
    }
    // `"targeted"`: the Channels panel's target, as a Photoshop action's "load the target
    // channel" step records it.
    let mut targeted = p.clone();
    if p.get("channel").and_then(Value::as_str) == Some("targeted") {
        let st = s.active().ok_or(EngineError::NoDocument)?;
        targeted["channel"] = match st.channel_view.target {
            ChannelTarget::Composite => json!("composite"),
            ChannelTarget::Color(k) => json!({ "color": k }),
            ChannelTarget::Alpha(i) => json!(i),
            ChannelTarget::LayerMask => json!("mask"),
            ChannelTarget::VectorMask => json!("vectorMask"),
        };
    }
    let p = &targeted;
    // Composite and colour channels load their luminosity / values, as ⌘-clicking them does.
    let plane = source(s, p, cmd, true)?.into_iter().next().ok_or_else(|| bad(cmd, "the channel has no pixels"))?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let label = match p.get("channel").and_then(|v| parse_ref(v, doc)) {
        Some(ChanRef::Transparency) => "Load Transparency",
        Some(ChanRef::LayerMask) => "Load Layer Mask",
        _ => "Load Selection",
    };
    let selected = s.edit(label, |doc, _| {
        doc.selection = sel::combine(doc.selection.as_ref(), &plane, doc.bounds(), mode);
        Ok(doc.selection.is_some())
    })?;
    Ok(json!({ "selected": selected }))
}

// ---------------------------------------------------------------------------------------------
// Quick Mask

/// Quick Mask channel for the current selection: white where selected. No selection is an empty
/// mask that shows no colour: white (nothing masked, as in Photoshop) when the colour indicates
/// masked areas, black (nothing selected) when it indicates selected areas. White and "no
/// selection" are told apart by [`Document::quick_mask_unselected`].
fn quick_mask_from(doc: &Document, o: &QuickMaskOptions) -> AlphaChannel {
    let area = doc.bounds();
    let fmt = channel_format(doc);
    let surface = match &doc.selection {
        Some(s) => plane_surface(&sel::mask_from_surface(Some(s), area), area, fmt),
        None => unselected_surface(area, fmt, o.indicates),
    };
    AlphaChannel { color: o.color, opacity: o.opacity, indicates: o.indicates, ..AlphaChannel::new("Quick Mask", surface) }
}

/// The colourless, nothing-selected Quick Mask for `indicates`.
pub(super) fn unselected_surface(area: Rect, fmt: PixelFormat, indicates: ColorIndicates) -> Surface {
    let mut s = Surface::new(fmt);
    if indicates != ColorIndicates::SelectedAreas {
        s.fill_rect(area, &[1.0]);
    }
    s
}

/// Every pixel of `vals` reads as `target` on the 8-bit grid.
pub(super) fn uniform(vals: &[f32], target: f32) -> bool {
    vals.iter().all(|v| (v - target).abs() <= 0.5 / 255.0)
}

/// The selection a Quick Mask channel turns back into. All white is Select All, unless Quick
/// Mask was entered with no selection and the white is its untouched masked-areas start.
fn selection_from(doc: &Document, ch: &AlphaChannel) -> Option<Surface> {
    let area = doc.bounds();
    let vals = read_plane(&ch.surface, area);
    if doc.quick_mask_unselected && ch.indicates != ColorIndicates::SelectedAreas && uniform(&vals, 1.0) {
        return None;
    }
    sel::combine(None, &vals, area, SelectionMode::Replace)
}

pub(super) fn quick_mask(s: &mut Session, p: &Value) -> Result<Value> {
    let on = s.active().ok_or(EngineError::NoDocument)?.doc.quick_mask.is_some();
    let want = p.get("on").and_then(Value::as_bool).unwrap_or(!on);
    if want == on {
        return Ok(json!({ "quickMask": on }));
    }
    let opts = s.quick_mask_options;
    if want {
        s.edit("Edit in Quick Mask", |doc, _| {
            doc.quick_mask = Some(quick_mask_from(doc, &opts));
            doc.quick_mask_unselected = doc.selection.is_none();
            doc.selection = None;
            Ok(())
        })?;
        if let Some(st) = s.active_mut() {
            st.channel_view.target = ChannelTarget::Composite;
            st.channel_view.quick_mask_hidden = false;
        }
    } else {
        s.edit("Exit Quick Mask", |doc, _| {
            if let Some(q) = doc.quick_mask.take() {
                doc.selection = selection_from(doc, &q);
            }
            Ok(())
        })?;
    }
    Ok(json!({ "quickMask": want }))
}
