use super::planes::{channel_format, color_count, edit_mode, next_alpha_name, plane_surface, read_plane};
use super::refs::{ChanRef, alpha_index, bad, doc_index, gray_plane, parse_ref};
use super::selection::{uniform, unselected_surface};
use super::targeting::target;
use super::view::{ChannelTarget, QuickMaskOptions, color_names, composite_name, fix_view, touch};
use super::{
    AlphaChannel, Color, ColorIndicates, ColorMode, DocState, Document, EngineError, Layer, PixelFormat, Result, Session, Surface, Value, json, list, sel,
};

// ---------------------------------------------------------------------------------------------
// Channel management

fn indicates_param(v: Option<&Value>) -> Option<ColorIndicates> {
    match v?.as_str()? {
        "masked" | "maskedAreas" => Some(ColorIndicates::MaskedAreas),
        "selected" | "selectedAreas" => Some(ColorIndicates::SelectedAreas),
        _ => None,
    }
}

fn color_value(cmd: &str, p: &Value, key: &str) -> Result<Option<Color>> {
    if p.get(key).is_none() {
        return Ok(None);
    }
    let c = crate::commands::color_param(cmd, p, key, [1.0, 0.0, 0.0, 1.0])?;
    Ok(Some(Color::rgb(c[0], c[1], c[2])))
}

pub(super) fn new_channel(s: &mut Session, p: &Value) -> Result<Value> {
    let fill = p.get("fill").and_then(Value::as_str).unwrap_or("black").to_string();
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let indicates = indicates_param(p.get("indicates"));
    let color = color_value("channel.new", p, "color")?;
    let opacity = p.get("opacity").and_then(Value::as_f64).map(|v| (v as f32 / 100.0).clamp(0.0, 1.0));
    let i = s.edit("New Channel", |doc, _| {
        let area = doc.bounds();
        let fmt = channel_format(doc);
        let surface = match fill.as_str() {
            "white" => plane_surface(&vec![1.0; area.width() as usize * area.height() as usize], area, fmt),
            "selection" => plane_surface(&sel::mask_from_surface(doc.selection.as_ref(), area), area, fmt),
            _ => Surface::new(fmt),
        };
        let mut ch = AlphaChannel::new(name.clone().unwrap_or_else(|| next_alpha_name(doc, "Alpha")), surface);
        if let Some(v) = indicates {
            ch.indicates = v;
        }
        if let Some(c) = color {
            ch.color = c;
        }
        if let Some(o) = opacity {
            ch.opacity = o;
        }
        doc.channels.push(ch);
        Ok(doc.channels.len() - 1)
    })?;
    // Photoshop targets and shows a new channel on its own.
    target(s, ChannelTarget::Alpha(i));
    Ok(json!({ "channel": i }))
}

pub(super) fn new_spot(s: &mut Session, p: &Value) -> Result<Value> {
    let color = color_value("channel.newSpot", p, "color")?.unwrap_or(Color::rgb(1.0, 0.0, 0.0));
    let solidity = p.get("solidity").and_then(Value::as_f64).unwrap_or(0.0).clamp(0.0, 100.0) as f32 / 100.0;
    let name = p.get("name").and_then(Value::as_str).map(str::to_string);
    let from_sel = p.get("fromSelection").and_then(Value::as_bool).unwrap_or(true);
    let i = s.edit("New Spot Channel", |doc, _| {
        let area = doc.bounds();
        let fmt = channel_format(doc);
        let surface = if from_sel && doc.selection.is_some() {
            plane_surface(&sel::mask_from_surface(doc.selection.as_ref(), area), area, fmt)
        } else {
            Surface::new(fmt)
        };
        let name = name.clone().unwrap_or_else(|| next_alpha_name(doc, "Spot Color"));
        doc.channels.push(AlphaChannel { spot: Some((color, solidity)), ..AlphaChannel::new(name, surface) });
        Ok(doc.channels.len() - 1)
    })?;
    if let Some(st) = s.active_mut() {
        st.channel_view.set_alpha_visible(i, true);
        touch(st);
    }
    Ok(json!({ "channel": i }))
}

pub(super) fn duplicate_channel(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "channel.duplicate";
    let src = s.active().ok_or(EngineError::NoDocument)?;
    let area = src.doc.bounds();
    let r = p.get("channel").and_then(|v| parse_ref(v, &src.doc)).ok_or_else(|| bad(cmd, "missing or unknown `channel`"))?;
    let mut ch = match r {
        ChanRef::Alpha(i) => src.doc.channels.get(i).cloned().ok_or_else(|| bad(cmd, format!("no alpha channel {i}")))?,
        ChanRef::QuickMask => src.doc.quick_mask.clone().ok_or_else(|| bad(cmd, "not in Quick Mask mode"))?,
        other => {
            let plane = gray_plane(&src.doc, None, src.active_layer, other)?;
            AlphaChannel::new(String::new(), plane_surface(&plane, area, channel_format(&src.doc)))
        }
    };
    let base = match r {
        ChanRef::Alpha(_) | ChanRef::QuickMask => ch.name.clone(),
        ChanRef::Color(k) => color_names(edit_mode(&src.doc))[k].to_string(),
        _ => composite_name(edit_mode(&src.doc)).to_string(),
    };
    ch.name = p.get("name").and_then(Value::as_str).map_or_else(|| format!("{base} copy"), str::to_string);
    if p.get("invert").and_then(Value::as_bool).unwrap_or(false) {
        let v: Vec<f32> = read_plane(&ch.surface, area).into_iter().map(|v| 1.0 - v).collect();
        ch.surface = plane_surface(&v, area, ch.surface.format());
    }
    match p.get("document") {
        Some(Value::String(x)) if x == "new" => {
            // Photoshop makes a multichannel document; ours is grayscale with the channel as pixels.
            let doc = gray_document(&ch.name, &src.doc, &read_plane(&ch.surface, area));
            let i = s.add_document(doc, None);
            Ok(json!({ "document": i }))
        }
        _ => {
            let di = doc_index(s, p, "document")?;
            if s.documents()[di].doc.size != src.doc.size {
                return Err(bad(cmd, "the destination document must have the same size"));
            }
            let prev = s.active_index();
            s.set_active(di);
            let r = s.edit("Duplicate Channel", |doc, _| {
                let fmt = channel_format(doc);
                if ch.surface.format() != fmt {
                    ch.surface = ch.surface.convert(fmt);
                }
                doc.channels.push(ch);
                Ok(doc.channels.len() - 1)
            });
            if let Some(i) = prev {
                s.set_active(i);
            }
            Ok(json!({ "channel": r?, "document": di }))
        }
    }
}

/// A one-layer grayscale document whose pixels are `plane` (Split Channels, Duplicate Channel
/// to a new document).
pub(super) fn gray_document(name: &str, like: &Document, plane: &[f32]) -> Document {
    let mut d = Document::new(name, like.size, ColorMode::Grayscale, like.depth);
    d.resolution_dpi = like.resolution_dpi;
    let fmt = d.pixel_format();
    let mut bg = Layer::raster("Background", fmt);
    bg.locks.transparency = true;
    bg.locks.position = true;
    if let Some(s) = bg.surface_mut() {
        let data: Vec<f32> = plane.iter().flat_map(|v| [v.clamp(0.0, 1.0), 1.0]).collect();
        s.write_region(like.bounds(), &data);
        s.prune();
    }
    d.layers.push(bg);
    d
}

pub(super) fn delete_channel(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "channel.delete";
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let i = match p.get("channel") {
        Some(_) => alpha_index(doc, p, "channel", cmd)?,
        None => match s.active().ok_or(EngineError::NoDocument)?.channel_view.target {
            ChannelTarget::Alpha(i) => i,
            _ => return Err(bad(cmd, "missing `channel`")),
        },
    };
    // `edit` validates the view against the shorter channel list before we can remap it.
    // Keep the original indices and eyes so a surviving final channel is not discarded.
    let view = s.active().ok_or(EngineError::NoDocument)?.channel_view.clone();
    s.edit("Delete Channel", |doc, _| {
        doc.channels.remove(i);
        Ok(())
    })?;
    if let Some(st) = s.active_mut() {
        st.channel_view = view;
        let v = &mut st.channel_view;
        if i < v.alpha_visible.len() {
            v.alpha_visible.remove(i);
        }
        v.target = match v.target {
            ChannelTarget::Alpha(t) if t == i => {
                v.color_hidden.clear();
                ChannelTarget::Composite
            }
            ChannelTarget::Alpha(t) if t > i => ChannelTarget::Alpha(t - 1),
            t => t,
        };
        fix_view_now(st);
    }
    Ok(Value::Null)
}

pub(super) fn fix_view_now(st: &mut DocState) {
    fix_view(st);
    touch(st);
}

pub(super) fn rename_channel(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "channel.rename";
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let i = alpha_index(doc, p, "channel", cmd)?;
    let name = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad(cmd, "missing `name`"))?.to_string();
    s.edit("Rename Channel", |doc, _| {
        doc.channels[i].name = name;
        Ok(())
    })?;
    Ok(Value::Null)
}

pub(super) fn move_channel(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "channel.move";
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let n = doc.channels.len();
    let i = alpha_index(doc, p, "channel", cmd)?;
    let to = p.get("to").and_then(Value::as_u64).map(|v| (v as usize).min(n - 1)).ok_or_else(|| bad(cmd, "missing `to`"))?;
    if to == i {
        return Ok(json!({ "channel": i }));
    }
    s.edit("Channel Order", |doc, _| {
        let c = doc.channels.remove(i);
        doc.channels.insert(to, c);
        Ok(())
    })?;
    if let Some(st) = s.active_mut() {
        let v = &mut st.channel_view;
        v.alpha_visible.resize(n, false);
        let vis = v.alpha_visible.remove(i);
        v.alpha_visible.insert(to, vis);
        let remap = |t: usize| {
            if t == i {
                to
            } else if i < t && t <= to {
                t - 1
            } else if to <= t && t < i {
                t + 1
            } else {
                t
            }
        };
        if let ChannelTarget::Alpha(t) = v.target {
            v.target = ChannelTarget::Alpha(remap(t));
        }
        touch(st);
    }
    Ok(json!({ "channel": to }))
}

pub(super) fn set_visible(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "channel.setVisible";
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let r = p.get("channel").and_then(|v| parse_ref(v, doc)).ok_or_else(|| bad(cmd, "missing or unknown `channel`"))?;
    let colors = color_count(doc);
    let n = doc.channels.len();
    let st = s.active_mut().ok_or(EngineError::NoDocument)?;
    let v = &mut st.channel_view;
    let visible = |cur: bool| p.get("visible").and_then(Value::as_bool).unwrap_or(!cur);
    match r {
        ChanRef::Composite => {
            let all = v.visible_colors(colors) == colors;
            let on = visible(all);
            v.color_hidden = vec![!on; colors];
        }
        ChanRef::Color(k) if k < colors => {
            let on = visible(v.color_visible(k));
            v.set_color_visible(k, on);
        }
        ChanRef::Alpha(i) if i < n => {
            let on = visible(v.alpha_shown(i));
            v.set_alpha_visible(i, on);
        }
        ChanRef::QuickMask => v.quick_mask_hidden = !visible(!v.quick_mask_hidden),
        _ => return Err(bad(cmd, "that channel has no visibility")),
    }
    fix_view_now(st);
    Ok(list(s))
}

pub(super) fn options(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "channel.options";
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let r = p.get("channel").and_then(|v| parse_ref(v, doc)).ok_or_else(|| bad(cmd, "missing or unknown `channel`"))?;
    let name = p.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty()).map(str::to_string);
    let color = color_value(cmd, p, "color")?;
    let opacity = p.get("opacity").and_then(Value::as_f64).map(|v| (v as f32 / 100.0).clamp(0.0, 1.0));
    let solidity = p.get("solidity").and_then(Value::as_f64).map(|v| (v as f32 / 100.0).clamp(0.0, 1.0));
    let kind = p.get("indicates").and_then(Value::as_str).map(str::to_string);
    let display_only = name.is_none() && kind.is_none() && solidity.is_none();
    let apply = move |ch: &mut AlphaChannel| -> Result<()> {
        if let Some(n) = &name {
            ch.name = n.clone();
        }
        match kind.as_deref() {
            Some("spot") => {
                let (c, sol) = ch.spot.unwrap_or((ch.color, 0.0));
                ch.spot = Some((color.unwrap_or(c), solidity.unwrap_or(sol)));
                return Ok(());
            }
            Some(k) => {
                ch.indicates = indicates_param(Some(&json!(k))).ok_or_else(|| bad(cmd, format!("unknown `indicates` {k}")))?;
                ch.spot = None;
            }
            None => {}
        }
        match &mut ch.spot {
            Some((c, sol)) => {
                *c = color.unwrap_or(*c);
                *sol = solidity.or(opacity).unwrap_or(*sol);
            }
            None => {
                ch.color = color.unwrap_or(ch.color);
                ch.opacity = opacity.unwrap_or(ch.opacity);
            }
        }
        Ok(())
    };
    match r {
        ChanRef::Alpha(i) if i < doc.channels.len() => {
            s.edit("Channel Options", |doc, _| apply(&mut doc.channels[i]))?;
        }
        ChanRef::QuickMask => {
            // Quick Mask Options persist for the session; the live Quick Mask follows them.
            let mut probe = AlphaChannel {
                color: s.quick_mask_options.color,
                opacity: s.quick_mask_options.opacity,
                indicates: s.quick_mask_options.indicates,
                ..AlphaChannel::new("Quick Mask", Surface::new(PixelFormat::GRAY8))
            };
            apply(&mut probe)?;
            if probe.spot.is_some() {
                return Err(bad(cmd, "Quick Mask can't be a spot channel"));
            }
            s.quick_mask_options = QuickMaskOptions { color: probe.color, opacity: probe.opacity, indicates: probe.indicates };
            if s.active().is_some_and(|d| d.doc.quick_mask.is_some()) {
                let o = s.quick_mask_options;
                s.edit("Quick Mask Options", |doc, _| {
                    let area = doc.bounds();
                    let fmt = channel_format(doc);
                    let unselected = doc.quick_mask_unselected;
                    if let Some(q) = &mut doc.quick_mask {
                        // An untouched no-selection mask stays colourless (nothing selected)
                        // when the colour switches between masked and selected areas.
                        if unselected && q.indicates != o.indicates {
                            let empty = if q.indicates == ColorIndicates::SelectedAreas { 0.0 } else { 1.0 };
                            if uniform(&read_plane(&q.surface, area), empty) {
                                q.surface = unselected_surface(area, fmt, o.indicates);
                            }
                        }
                        q.color = o.color;
                        q.opacity = o.opacity;
                        q.indicates = o.indicates;
                    }
                    Ok(())
                })?;
            }
        }
        ChanRef::LayerMask => {
            // Layer Mask Display Options: only the rubylith's colour and opacity, for every mask.
            if !display_only {
                return Err(bad(cmd, "a layer mask's options are only `color` and `opacity`"));
            }
            // A preference, kept across restarts; not an undo step.
            if color.is_some() || opacity.is_some() {
                s.edit_prefs(|prefs| {
                    let o = &mut prefs.layer_mask_overlay;
                    if let Some(c) = color {
                        o.color = crate::adjust_params::hex(c.to_rgb());
                    }
                    if let Some(a) = opacity {
                        o.opacity = (a * 100.0).round();
                    }
                });
            }
            let o = serde_json::to_value(&s.prefs().layer_mask_overlay).unwrap_or(Value::Null);
            let mut out = list(s);
            if let Some(m) = out.as_object_mut() {
                m.insert("layerMaskOverlay".into(), o);
            }
            return Ok(out);
        }
        _ => return Err(bad(cmd, "`channel` must be an alpha/spot channel, \"quickMask\" or \"mask\"")),
    }
    Ok(list(s))
}
