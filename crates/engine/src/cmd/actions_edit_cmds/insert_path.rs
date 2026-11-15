//! Capture full native geometry without substituting another path for a stale selection.
use super::{Result, Session, Value, bad, json};
use photocraft_doc::{LayerContent, Path};

/// Borrow exactly the path the panel selected, without Work Path's ordinary layer fallback.
pub fn selected_path<'a>(s: &'a Session, name: &str) -> Result<&'a Path> {
    let st = s.active().ok_or(crate::EngineError::NoDocument)?;
    let path = match name {
        "work" => st.doc.work_path.as_ref(),
        "layer" => st.active_layer.and_then(|id| st.doc.layer(id)).and_then(|l| match &l.content {
            LayerContent::Shape(shape) => Some(&shape.path),
            _ => l.vector_mask.as_ref().map(|v| &v.path),
        }),
        name => st.doc.paths.iter().find(|p| p.name == name).map(|p| &p.path),
    };
    path.ok_or_else(|| bad("actions.insertPath", "the selected path no longer exists"))
}

/// The native path step is also the source for a future independently verified ATN wire mapping.
pub(super) fn run(s: &mut Session, p: &Value) -> Result<Value> {
    let name = p.get("name").and_then(Value::as_str).ok_or_else(|| bad("actions.insertPath", "select a path"))?;
    let path = selected_path(s, name)?;
    photocraft_vector::flatten::validate_path(path).map_err(|e| bad("actions.insertPath", e))?;
    let path = crate::vector_cmds::path_json(path);
    s.execute("actions.insert", json!({"action":p.get("action"),"at":p.get("at"),"id":"path.set","params":{"name":"work","path":path}}))
}
