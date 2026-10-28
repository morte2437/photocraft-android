use super::filters::{smart, smart_mut};
use super::placing::detach_psd;
use super::rendering::{refresh, refresh_or_fail, unapplied_filters};
use super::source::{authorized_source_bytes, source_bytes};
use super::{
    Arc, DocId, Document, EngineError, LayerContent, LayerId, Metadata, Result, Session, SmartContentsId, SmartLink, SmartSource, Value, bad, base_name,
    decode_source, encode_source, json, layer_param, other,
};

pub(super) fn path_param<'a>(cmd: &str, p: &'a Value) -> Result<&'a str> {
    p.get("path").and_then(Value::as_str).filter(|s| !s.is_empty()).ok_or_else(|| bad(cmd, "pass `path`"))
}

/// Instances sharing contents in this document, including those inside groups.
fn instances(doc: &Document, contents_id: SmartContentsId) -> Vec<LayerId> {
    doc.walk().into_iter().filter_map(|(_, _, l)| matches!(&l.content, LayerContent::Smart(sm) if sm.contents_id == contents_id).then_some(l.id)).collect()
}

/// Swaps shared contents, keeping each instance's transform and filters, and re-renders.
pub(super) fn set_source(
    s: &mut Session,
    p: &Value,
    label: &str,
    keep_psd: bool,
    make: impl FnOnce(&Metadata, &SmartSource) -> Result<SmartSource>,
) -> Result<Value> {
    let id = layer_param(s, p)?;
    let doc = &s.active().ok_or(EngineError::NoDocument)?.doc;
    let new = make(&doc.metadata, &smart(doc, id)?.source)?;
    set_shared_source(s, id, label, keep_psd, new)
}

// Keep the shared-instance transaction out of the per-command source-builder instantiations.
fn set_shared_source(s: &mut Session, id: LayerId, label: &str, keep_psd: bool, new: SmartSource) -> Result<Value> {
    let (parent, contents_id, changed, warnings) = s.edit(label, |doc, _| {
        let contents_id = smart(doc, id)?.contents_id;
        let changed = instances(doc, contents_id);
        let mut warnings = std::collections::BTreeSet::new();
        for &instance in &changed {
            smart_mut(doc, instance)?.source = new.clone();
            if !keep_psd {
                detach_psd(doc.layer_mut(instance).ok_or(EngineError::NoLayer(instance))?);
            }
            warnings
                .extend(unapplied_filters(smart(doc, instance)?).into_iter().map(|n| format!("the {n} smart filter is not supported, so it was not applied")));
            refresh_or_fail(doc, instance)?;
        }
        Ok((doc.id, contents_id, changed, warnings))
    })?;
    // An old editor must not overwrite newly replaced/relinked contents on close. Keep its
    // document open, but the next Edit Contents opens the replacement source.
    s.smart_links.retain(|l| l.parent != parent || l.contents_id != contents_id);
    // Every instance changed, not only `layer`: list them, so a caller doesn't repeat the
    // command once per copy (#4075).
    Ok(json!({"layer": id.0, "instances": changed.iter().map(|l| l.0).collect::<Vec<_>>(), "warnings": warnings}))
}

pub(super) fn replace_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_param("layer.smartObjects.replaceContents", p)?.to_string();
    let bytes = photocraft_format::read_file(std::path::Path::new(&path)).map_err(|e| other(format!("can't read {path}: {e}")))?;
    let id = layer_param(s, p)?;
    replace_contents_bytes(s, id, &base_name(&path), bytes)
}

/// Replace Contents with a file the caller already read, for the smart object `p` names (`layer`,
/// else the active layer): the body of the command minus its ambient path read (#8061).
pub fn replace_contents_from_bytes(s: &mut Session, p: &Value, file_name: &str, bytes: Vec<u8>) -> Result<Value> {
    let id = layer_param(s, p)?;
    replace_contents_bytes(s, id, file_name, bytes)
}

/// Replace Contents from a file's name and bytes: what the desktop and web file pickers hand over.
pub fn replace_contents_bytes(s: &mut Session, id: LayerId, file_name: &str, bytes: Vec<u8>) -> Result<Value> {
    decode_source(file_name, &bytes)?; // fail before touching the document
    set_shared_source(s, id, "Replace Contents", false, SmartSource::Embedded { file_name: file_name.to_string(), bytes: Arc::new(bytes) })
}

/// The file name and bytes of smart object `id`'s contents.
pub fn contents_of(doc: &Document, id: LayerId) -> Result<(String, Arc<Vec<u8>>)> {
    source_bytes(&doc.metadata, &smart(doc, id)?.source).ok_or_else(|| other("the smart object's contents are unavailable"))
}

/// Writes the smart object's file as it is. To a path with another extension (a `.png` for
/// `.psb` or `.pcraft` contents) the contents are opened and exported in that format instead.
pub(super) fn export_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let path = path_param("layer.smartObjects.exportContents", p)?;
    let id = layer_param(s, p)?;
    let (name, bytes) = contents_of(&s.active().ok_or(EngineError::NoDocument)?.doc, id)?;
    let ext = |f: &str| {
        let e = std::path::Path::new(f).extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase);
        e.map(|e| match e.as_str() {
            "jpeg" => "jpg".to_string(),
            "tiff" => "tif".to_string(),
            _ => e,
        })
    };
    if ext(path).is_none() || ext(path) == ext(&name) {
        crate::file_cmds::write_file(path, &bytes)?;
        return Ok(json!({"path": path, "fileName": name, "bytes": bytes.len(), "converted": false}));
    }
    let contents = decode_source(&name, &bytes)?;
    let r = photocraft_io::export(&contents, path, &Default::default()).map_err(|e| other(format!("can't export the contents as {}: {e}", base_name(path))))?;
    crate::file_cmds::write_file(path, &r.bytes)?;
    Ok(json!({"path": path, "fileName": name, "bytes": r.bytes.len(), "converted": true, "warnings": r.warnings}))
}

pub(super) fn edit_contents(s: &mut Session, p: &Value) -> Result<Value> {
    let id = layer_param(s, p)?;
    let st = s.active().ok_or(EngineError::NoDocument)?;
    let parent = st.doc.id;
    let contents_id = smart(&st.doc, id)?.contents_id;
    // Already open for editing: switch to that document rather than opening another copy.
    if let Some(index) =
        s.smart_links.iter().find(|l| l.parent == parent && l.contents_id == contents_id).and_then(|l| s.documents().iter().position(|d| d.doc.id == l.child))
    {
        s.set_active(index);
        return Ok(json!({"document": index, "parentLayer": id.0}));
    }
    let source = &smart(&st.doc, id)?.source;
    let (name, bytes) = authorized_source_bytes(s, "layer.smartObjects.editContents", &st.doc.metadata, source)?
        .ok_or_else(|| other("the smart object's contents are unavailable"))?;
    let mut child = decode_source(&name, &bytes)?;
    // Bundles keep their document id; each open copy needs its own.
    child.id = DocId::fresh();
    child.name = name;
    let index = s.add_document(child, None);
    // Admission may replace an ID already owned by another open document.
    let child_id = s.documents().get(index).ok_or(EngineError::NoDocument)?.doc.id;
    s.smart_links.retain(|l| l.child != child_id);
    s.smart_links.push(SmartLink { child: child_id, parent, layer: id, contents_id });
    Ok(json!({"document": index, "parentLayer": id.0}))
}

/// Writes an Edit Contents document back into all its parent instances (one undoable step in the
/// parent) and marks it saved. Returns false if `index` isn't an Edit Contents document.
pub fn commit_child(s: &mut Session, index: usize) -> Result<bool> {
    let Some(st) = s.docs.get(index) else { return Ok(false) };
    let child = st.doc.clone();
    let Some(link) = s.smart_links.iter().find(|l| l.child == child.id).copied() else { return Ok(false) };
    let parent = s.docs.iter().position(|d| d.doc.id == link.parent).ok_or_else(|| other("the smart object's document was closed"))?;
    let bytes = Arc::new(encode_source(&child)?);
    let prev = s.active;
    s.active = Some(parent);
    s.note_active_change();
    let r = s.edit("Edit Contents", |doc, _| {
        let ids = instances(doc, link.contents_id);
        if ids.is_empty() {
            return Err(other("the smart object's instances were removed"));
        }
        for id in ids {
            let sm = smart_mut(doc, id)?;
            let name = match &sm.source {
                SmartSource::Embedded { file_name, .. } => file_name.as_str(),
                SmartSource::Linked { .. } => child.name.as_str(),
            };
            let stem = name.rsplit_once('.').map_or(name, |(stem, _)| stem);
            sm.source = SmartSource::Embedded { file_name: format!("{stem}.pcraft"), bytes: bytes.clone() };
            detach_psd(doc.layer_mut(id).ok_or(EngineError::NoLayer(id))?);
            refresh_or_fail(doc, id)?;
        }
        Ok(())
    });
    s.active = prev;
    s.note_active_change();
    r?;
    if let Some(st) = s.docs.get_mut(index) {
        st.saved_revision = st.revision;
    }
    Ok(true)
}

/// Called before a document closes: an edited Edit Contents document updates its parent.
pub(crate) fn on_close(s: &mut Session, index: usize) {
    let Some(st) = s.docs.get(index) else { return };
    let id = st.doc.id;
    if st.is_dirty() && s.smart_links.iter().any(|l| l.child == id) {
        let _ = commit_child(s, index);
    }
    s.smart_links.retain(|l| l.child != id && l.parent != id);
}

pub(super) fn is_smart_child(s: &Session) -> std::result::Result<(), String> {
    let d = s.active().ok_or("no document open")?;
    if s.smart_links.iter().any(|l| l.child == d.doc.id) { Ok(()) } else { Err("this document isn't smart object contents".into()) }
}

pub(super) fn update_all(s: &mut Session) -> Result<Value> {
    let ids: Vec<LayerId> = s
        .active()
        .ok_or(EngineError::NoDocument)?
        .doc
        .walk()
        .into_iter()
        .filter(|(_, _, l)| matches!(&l.content, LayerContent::Smart(sm) if matches!(sm.source, SmartSource::Linked { .. })))
        .map(|(_, _, l)| l.id)
        .collect();
    let mut updated = Vec::new();
    s.edit("Update All Modified Content", |doc, _| {
        for id in &ids {
            if refresh(doc, *id)? {
                updated.push(id.0);
            }
        }
        Ok(())
    })?;
    Ok(json!({"updated": updated}))
}
