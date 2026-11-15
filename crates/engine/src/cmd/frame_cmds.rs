//! Photoshop-style image frames: independently editable vector containers with smart content.
use crate::commands::CommandSpec;
use crate::{EngineError, Result, Session};
use photocraft_color::{BlendMode, Color};
use photocraft_doc::{Document, Frame, Layer, LayerContent, LayerId, Path, VectorMask};
use photocraft_geom::Affine;
use serde_json::{Value, json};

fn bad(msg: impl Into<String>) -> EngineError {
    EngineError::BadParams { cmd: "layer.frame".into(), msg: msg.into() }
}
fn has_doc(s: &Session) -> std::result::Result<(), String> {
    s.active().map(|_| ()).ok_or("no document open".into())
}
fn has_layer(s: &Session) -> std::result::Result<(), String> {
    s.active().and_then(|d| d.active_layer).map(|_| ()).ok_or("select a layer".into())
}
/// The enclosing frame, including a frame's selected content.
pub fn frame_of(doc: &Document, id: LayerId) -> Option<LayerId> {
    let path = doc.path_of(id)?;
    (1..=path.len()).rev().filter_map(|n| doc.layer_at(path.get(..n)?)).find(|l| l.frame.is_some()).map(|l| l.id)
}
fn frame_id(s: &Session, p: &Value) -> Result<LayerId> {
    let id = crate::vector_cmds::layer_id(s, p)?;
    frame_of(&s.active().ok_or(EngineError::NoDocument)?.doc, id).ok_or_else(|| bad("select a frame"))
}
fn check_lock(doc: &Document, id: LayerId) -> Result<()> {
    let locks = doc.effective_locks(id);
    if locks.all || locks.position { Err(bad("the frame or its containing group is locked")) } else { Ok(()) }
}
fn geometry(p: &Value) -> Result<Frame> {
    let get = |key: &str| crate::commands::int_i32("layer.new.frame", p, key)?.ok_or_else(|| bad(format!("missing `{key}`")));
    let (x, y, w, h) = (get("x")?, get("y")?, get("width")?, get("height")?);
    if !(1..=300_000).contains(&w) || !(1..=300_000).contains(&h) || x.checked_add(w).is_none() || y.checked_add(h).is_none() {
        return Err(bad("frame sides must be 1..=300000 pixels, within the coordinate range"));
    }
    let shape = p.get("shape").map_or(Some("rectangle"), Value::as_str).ok_or_else(|| bad("shape must be a string"))?;
    let path = match shape {
        "rectangle" | "ellipse" | "triangle" | "polygon" => {
            let sides = p.get("sides").map_or(Some(5), Value::as_u64).ok_or_else(|| bad("sides must be an integer"))?;
            if !(3..=100).contains(&sides) {
                return Err(bad("sides must be 3..=100"));
            }
            crate::vector_cmds::shape_path(&json!({"kind": match shape { "rectangle" => "rect", "ellipse" => "ellipse", _ => "polygon" },
                "rect": [0, 0, 1, 1], "sides": if shape == "triangle" { 3 } else { sides }}))?
        }
        "custom" => {
            let path = crate::vector_cmds::parse_path(p.get("path").ok_or_else(|| bad("a custom frame needs path"))?).map_err(bad)?;
            normalized(&path)?.0
        }
        _ => return Err(bad("shape must be rectangle, ellipse, triangle, polygon or custom")),
    };
    let stroke = p.get("stroke").map(|v| crate::vector_cmds::parse_stroke(v, None, Color::BLACK).map_err(bad)).transpose()?.flatten();
    let f = Frame { path, transform: Affine { m: [f64::from(w), 0.0, 0.0, f64::from(h), f64::from(x), f64::from(y)] }, stroke };
    if !f.valid() {
        return Err(bad("invalid or excessively complex frame geometry"));
    }
    Ok(f)
}
fn normalized(path: &Path) -> Result<(Path, Affine)> {
    let (x0, y0, x1, y1) = path.control_bounds().ok_or_else(|| bad("the outline is empty"))?;
    let (w, h) = (x1 - x0, y1 - y0);
    if ![x0, y0, x1, y1, w, h].iter().all(|v| v.is_finite()) || w <= 0.0 || h <= 0.0 {
        return Err(bad("the outline must have finite nonzero dimensions"));
    }
    let transform = Affine { m: [w, 0.0, 0.0, h, x0, y0] };
    let inverse = transform.inverse().ok_or_else(|| bad("the outline is too small"))?;
    Ok((path.transform(&inverse), transform))
}
/// Keep the clipping path in the ordinary document model, so every compositor and exporter masks it.
pub fn refresh(layer: &mut Layer) {
    if let Some(f) = &layer.frame {
        layer.vector_mask = Some(VectorMask { path: f.outline(), enabled: true, linked: true, density: 1.0, feather: 0.0 });
    }
}
fn info(layer: &Layer) -> Value {
    layer.frame.as_ref().map_or(
        Value::Null,
        |f| json!({"layer":layer.id.0,"name":layer.name,"transform":f.transform.m,"frame":[f.bounds().x0,f.bounds().y0,f.bounds().width(),f.bounds().height()],"stroke":f.stroke,"content":layer.children().and_then(|c| c.first()).map(|l|l.id.0)}),
    )
}
fn query(s: &mut Session, p: &Value) -> Result<Value> {
    let id = frame_id(s, p)?;
    Ok(info(s.active().and_then(|st| st.doc.layer(id)).ok_or(EngineError::NoLayer(id))?))
}
fn new_frame(s: &mut Session, p: &Value) -> Result<Value> {
    let f = geometry(p)?;
    let content = match p.get("layer") {
        None | Some(Value::Null) => None,
        Some(_) => Some(crate::commands::layer_param(s, p)?),
    };
    let name = p.get("name").map(|v| v.as_str().map(str::to_owned).ok_or_else(|| bad("name must be a string"))).transpose()?;
    let gid = s.edit("New Frame", |doc, active| {
        let child = match content {
            None => None,
            Some(id) => {
                check_lock(doc, id)?;
                let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
                if !matches!(l.content, LayerContent::Raster(_) | LayerContent::Text(_) | LayerContent::Shape(_) | LayerContent::Smart(_))
                    || doc.is_background(id)
                {
                    return Err(bad("a frame takes a pixel, type, shape or smart object layer other than the Background"));
                }
                if frame_of(doc, id).is_some() {
                    return Err(bad("that layer is already a frame's image"));
                }
                let mut child = if matches!(l.content, LayerContent::Smart(_)) { l.clone() } else { crate::smart_cmds::layer_to_smart(doc, l)? };
                child.id = id;
                Some(child)
            }
        };
        // A frame drawn around a layer is named after it ("Red Frame"); an empty one is "Frame N".
        let name = name.unwrap_or_else(|| child.as_ref().map_or_else(|| doc.next_layer_name("Frame"), |c| format!("{} Frame", c.name)));
        let mut g = Layer::group(name, vec![]);
        g.blend = BlendMode::Normal;
        g.frame = Some(Box::new(f));
        refresh(&mut g);
        // A frame is one row, as in Photoshop: closed, so the next new layer goes above it rather
        // than into it.
        if let LayerContent::Group(group) = &mut g.content {
            group.expanded = false;
        }
        let gid = crate::layer_place::insert_new(doc, content.or(*active), g, crate::layer_place::Place::AutoEditable);
        if let Some(child) = child {
            doc.remove(child.id).ok_or(EngineError::NoLayer(child.id))?;
            doc.layer_mut(gid).and_then(Layer::children_mut).ok_or(EngineError::NoLayer(gid))?.push(child);
        }
        crate::layer_multi_cmds::check_group_depth(doc, "New Frame")?;
        *active = Some(gid);
        Ok(gid)
    })?;
    crate::layer_multi_cmds::reselect(s, vec![gid], Some(gid));
    let mut result = info(s.active().and_then(|st| st.doc.layer(gid)).ok_or(EngineError::NoLayer(gid))?);
    result["shape"] = p.get("shape").cloned().unwrap_or(json!("rectangle"));
    let (x, y, w, h) = (p["x"].clone(), p["y"].clone(), p["width"].clone(), p["height"].clone());
    result["frame"] = json!([x, y, w, h]);
    Ok(result)
}
fn frame_from_layers(s: &mut Session, p: &Value) -> Result<Value> {
    // A `layer` of its own (a recorded step names its layer) is what the grouping below takes
    // in, so the frame fits that layer, not whatever is selected.
    let explicit = crate::commands::explicit_layer(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let ids = explicit.map_or_else(|| st.selected_layers(), |id| vec![id]);
    let bounds = ids
        .iter()
        .filter_map(|id| st.doc.layer(*id))
        .map(|l| crate::transform_cmds::transform_frame(&st.doc, l))
        .reduce(|a, b| a.union(&b))
        .filter(|b| !b.is_empty())
        .unwrap_or(st.doc.bounds());
    let f = geometry(&json!({"x":bounds.x0,"y":bounds.y0,"width":bounds.width(),"height":bounds.height()}))?;
    let journal = s.journal.len();
    let grouped = s.execute("layer.new.groupFromLayers", p.clone())?;
    let id = LayerId(grouped.get("layer").and_then(Value::as_u64).ok_or_else(|| bad("grouping returned no layer"))?);
    let r = s.edit("Frame from Layers", |doc, _| {
        let group = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let children = group.children().ok_or_else(|| bad("not a group"))?.to_vec();
        let image = crate::smart_cmds::layers_to_smart(doc, &group.name, children)?;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        if let Some(children) = l.children_mut() {
            *children = vec![image];
        }
        l.frame = Some(Box::new(f));
        l.blend = BlendMode::Normal;
        refresh(l);
        Ok(())
    });
    match r {
        Ok(()) => {
            if let Some(st) = s.active_mut() {
                st.history.purge_last();
            }
            Ok(json!({"layer":id.0,"frame":[bounds.x0,bounds.y0,bounds.width(),bounds.height()]}))
        }
        Err(e) => {
            // The group couldn't be framed: take the grouping back, leaving nothing to redo and
            // no grouping step to record.
            s.rollback();
            s.journal.truncate(journal);
            Err(e)
        }
    }
}
fn set(s: &mut Session, p: &Value) -> Result<Value> {
    if !["transform", "stroke", "name"].iter().any(|key| p.get(key).is_some()) {
        return Err(bad("supply transform, stroke or name"));
    }
    // The Frame tool moves a frame as a "Move" step (#13880); every other change is "Frame Properties".
    let step = match p.get("historyName") {
        None | Some(Value::Null) => "Frame Properties",
        Some(v) if v.as_str() == Some("Move") => "Move",
        Some(_) => return Err(bad("`historyName` must be \"Move\" or null")),
    };
    let id = frame_id(s, p)?;
    s.edit(step, |doc, _| {
        check_lock(doc, id)?;
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        let f = l.frame.as_mut().ok_or_else(|| bad("not a frame"))?;
        if let Some(v) = p.get("transform") {
            let m: [f64; 6] = serde_json::from_value(v.clone()).map_err(|_| bad("transform must be six finite numbers"))?;
            f.transform = Affine { m };
        }
        if let Some(v) = p.get("stroke") {
            f.stroke = crate::vector_cmds::parse_stroke(v, f.stroke.clone(), Color::BLACK).map_err(bad)?;
        }
        if let Some(v) = p.get("name") {
            l.name = v.as_str().ok_or_else(|| bad("name must be a string"))?.into();
        }
        if !f.valid() {
            return Err(bad("invalid frame geometry or stroke"));
        }
        refresh(l);
        Ok(())
    })?;
    Ok(info(s.active().and_then(|st| st.doc.layer(id)).ok_or(EngineError::NoLayer(id))?))
}
fn convert(s: &mut Session, p: &Value) -> Result<Value> {
    let id = crate::vector_cmds::layer_id(s, p)?;
    s.edit("Convert to Frame", |doc, active| {
        check_lock(doc, id)?;
        let l = doc.layer(id).ok_or(EngineError::NoLayer(id))?;
        let outline = match &l.content {
            LayerContent::Shape(sh) => sh.path.clone(),
            LayerContent::Text(t) => crate::type_extra_cmds::text_path(doc, t),
            _ => return Err(bad("select a shape or type layer")),
        };
        let (path, transform) = normalized(&outline)?;
        let mut f = Frame { path, transform, stroke: None };
        if p.get("width").is_some() || p.get("height").is_some() {
            let [a, b, c, d, x, y] = f.transform.m;
            let n = |k: &str, old: f64| -> Result<f64> {
                match p.get(k) {
                    None => Ok(old),
                    Some(v) => v.as_f64().filter(|v| v.is_finite() && (1.0..=300_000.0).contains(v)).ok_or_else(|| bad(format!("{k} must be 1..=300000"))),
                }
            };
            f.transform.m = [n("width", a)?, b, c, n("height", d)?, x, y];
        }
        if !f.valid() {
            return Err(bad("invalid converted frame"));
        }
        let name = p.get("name").map(|v| v.as_str().ok_or_else(|| bad("name must be a string"))).transpose()?.unwrap_or(&l.name).to_string();
        let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
        l.content = LayerContent::Group(photocraft_doc::Group { children: vec![], expanded: false, artboard: None, frame_stroke: None });
        l.name = name;
        l.frame = Some(Box::new(f));
        l.mask = None;
        l.effects = Default::default();
        // The group would otherwise still carry the type or shape layer's PSD content blocks.
        l.fill_cache = None;
        l.psd_blocks.retain(|(k, _)| !crate::vector_cmds::SHAPE_BLOCKS.contains(&k) && !matches!(k, b"TySh" | b"tySh" | b"lmfx"));
        l.blend = BlendMode::Normal;
        refresh(l);
        *active = Some(id);
        Ok(())
    })?;
    Ok(info(s.active().and_then(|st| st.doc.layer(id)).ok_or(EngineError::NoLayer(id))?))
}
/// Insert/replace the image, preserving the container's identity and placement.
pub fn put_content(doc: &mut Document, id: LayerId, layer: Layer) -> Result<()> {
    check_lock(doc, id)?;
    let l = doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?;
    if l.frame.is_none() {
        return Err(bad("not a frame"));
    }
    let children = l.children_mut().ok_or_else(|| bad("frame is not a container"))?;
    *children = vec![layer];
    Ok(())
}
fn set_content(s: &mut Session, p: &Value) -> Result<Value> {
    let id = frame_id(s, p)?;
    let source = LayerId(p.get("content").and_then(Value::as_u64).ok_or_else(|| bad("content must name an image layer"))?);
    let copy = p.get("copy").map_or(Some(false), Value::as_bool).ok_or_else(|| bad("copy must be a boolean"))?;
    let child_id = s.edit("Replace Frame Content", |doc, active| {
        check_lock(doc, source)?;
        let src = doc.layer(source).ok_or(EngineError::NoLayer(source))?;
        if !matches!(src.content, LayerContent::Raster(_) | LayerContent::Smart(_)) || frame_of(doc, source) == Some(source) {
            return Err(bad("content must be a pixel or smart object layer"));
        }
        let mut child = if matches!(src.content, LayerContent::Smart(_)) { src.clone() } else { crate::smart_cmds::layer_to_smart(doc, src)? };
        if copy {
            child = child.duplicate();
        } else {
            child.id = source;
            doc.remove(source).ok_or(EngineError::NoLayer(source))?;
        }
        let child_id = child.id;
        put_content(doc, id, child)?;
        *active = Some(id);
        Ok(child_id)
    })?;
    Ok(json!({"layer":id.0,"content":child_id.0}))
}
fn fit(s: &mut Session, p: &Value) -> Result<Value> {
    let id = frame_id(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let l = st.doc.layer(id).ok_or(EngineError::NoLayer(id))?;
    let f = l.frame.as_ref().ok_or_else(|| bad("not a frame"))?.clone();
    let child = l.children().and_then(|c| c.first()).ok_or_else(|| bad("the frame has no image"))?;
    check_lock(&st.doc, id)?;
    check_lock(&st.doc, child.id)?;
    // Transparent margins are part of a placed image. Fitting its painted-pixel bounds would
    // progressively zoom a PNG on every click; use the immutable source canvas instead.
    if let LayerContent::Smart(smart) = &child.content
        && smart.warp.is_none()
        && smart.perspective.is_none()
        && let Some([x0, y0, x1, y1]) = crate::smart_cmds::source_box(&st.doc, smart)
    {
        let (w, h) = (x1 - x0, y1 - y0);
        if w <= 0.0 || h <= 0.0 {
            return Err(bad("the frame image has no canvas"));
        }
        match p.get("mode").map_or(Some("contentToFrame"), Value::as_str) {
            Some("frameToContent") => {
                let transform = smart.transform.mul(&Affine { m: [w, 0.0, 0.0, h, x0, y0] });
                return set(s, &json!({"layer":id.0,"transform":transform.m}));
            }
            Some("contentToFrame") => {
                let [a, b, c, d, _, _] = f.transform.m;
                let (fw, fh) = (a.hypot(b), c.hypot(d));
                let k = (fw / w).max(fh / h);
                let transform =
                    f.transform.mul(&Affine { m: [k / fw, 0.0, 0.0, k / fh, (1.0 - w * k / fw) / 2.0 - x0 * k / fw, (1.0 - h * k / fh) / 2.0 - y0 * k / fh] });
                let child_id = child.id;
                let relative = transform.mul(&smart.transform.inverse().ok_or_else(|| bad("the frame image has a degenerate transform"))?);
                // The same path as `edit.transform`, so the image's linked masks and Smart Filter
                // mask move with it.
                return s.edit("Fit Content to Frame", |doc, _| {
                    crate::transform_cmds::transform_by(doc, child_id, relative, photocraft_algo::transform::Interp::Bicubic)?;
                    Ok(json!({"layer":id.0,"content":child_id.0}))
                });
            }
            _ => return Err(bad("mode must be contentToFrame or frameToContent")),
        }
    }
    let b = crate::transform_cmds::transform_frame(&st.doc, child);
    if b.is_empty() {
        return Err(bad("the frame content is empty"));
    }
    match p.get("mode").map_or(Some("contentToFrame"), Value::as_str) {
        Some("frameToContent") => set(s, &json!({"layer":id.0,"transform":[b.width(),0,0,b.height(),b.x0,b.y0]})),
        Some("contentToFrame") => {
            let [a, bv, c, d, _, _] = f.transform.m;
            let (w, h) = (a.hypot(bv), c.hypot(d));
            let k = (w / f64::from(b.width())).max(h / f64::from(b.height()));
            let (nw, nh) = (f64::from(b.width()) * k / w, f64::from(b.height()) * k / h);
            let quad = [
                (-0.5 * (nw - 1.0), -0.5 * (nh - 1.0)),
                (0.5 * (nw + 1.0), -0.5 * (nh - 1.0)),
                (0.5 * (nw + 1.0), 0.5 * (nh + 1.0)),
                (-0.5 * (nw - 1.0), 0.5 * (nh + 1.0)),
            ]
            .map(|(x, y)| {
                let q = f.transform.apply(photocraft_geom::Point::new(x, y));
                [q.x, q.y]
            });
            s.execute("edit.transform", json!({"layer":child.id.0,"rect":[b.x0,b.y0,b.x1,b.y1],"quad":quad}))
        }
        _ => Err(bad("mode must be contentToFrame or frameToContent")),
    }
}
pub fn specs() -> Vec<CommandSpec> {
    let spec = |id, label, params, enabled, run| CommandSpec { id, label, params, menu: &[], shortcut: None, journal: true, enabled, run };
    vec![
        spec(
            "layer.new.frame",
            "New Frame",
            r#"{"x":i32,"y":i32,"width":u32,"height":u32,"shape":"rectangle|ellipse|triangle|polygon|custom"="rectangle","sides":3..100=5,"path":object?,"stroke":object|null?,"layer":id?,"name":str?} → {layer,frame:[x,y,w,h],shape}"#,
            has_doc,
            new_frame,
        ),
        CommandSpec {
            menu: &["Layer", "New"],
            ..spec("layer.new.frameFromLayers", "Frame from Layers", r#"{"layer":id?,"name":str?}"#, has_layer, frame_from_layers)
        },
        spec(
            "layer.frame.set",
            "Frame Properties",
            r#"{"layer":id?,"transform":[a,b,c,d,x,y]?,"stroke":object|null?,"name":str?,"historyName":"Move"?}"#,
            has_doc,
            set,
        ),
        CommandSpec { journal: false, ..spec("layer.frame.info", "Frame Info", r#"{"layer":id?}"#, has_doc, query) },
        spec("layer.frame.convert", "Convert to Frame", r#"{"layer":id?,"name":str?,"width":px?,"height":px?}"#, has_doc, convert),
        spec(
            "layer.frame.setContent",
            "Replace Frame Content",
            r#"{"layer":id?,"content":id,"copy":bool=false}: a layer dragged into a frame keeps its original size"#,
            has_doc,
            set_content,
        ),
        spec("layer.frame.fit", "Fit Frame / Content", r#"{"layer":id?,"mode":"contentToFrame|frameToContent"="contentToFrame"}"#, has_doc, fit),
    ]
}
#[cfg(test)]
mod content_name_tests;
#[cfg(test)]
mod tests;
