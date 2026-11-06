//! Import a complete custom-shape set before changing the persistent library.

use std::collections::{HashMap, HashSet};

use serde_json::{Value, json};

use super::{CommandSpec, Group, Session, ShapePreset, always, bad, fit};
use crate::Result;

const CMD: &str = "shape.presets.importCsh";

pub(super) fn spec() -> CommandSpec {
    CommandSpec {
        id: CMD,
        label: "Import Shapes",
        menu: &[],
        shortcut: None,
        params: r#"{"path":str (.csh) | "data":str (base64),"group":str?} → {group,count,names}; appends a uniquely named group, never replaces existing presets"#,
        enabled: always,
        run: import,
        journal: true,
    }
}

/// Parse and convert the complete set before applying any library changes.
/// Group and shape names are resolved against the live library on apply, not the worker's
/// snapshot, so edits made while the job runs do not introduce duplicate addressable names.
/// Conversion fits each path into a 100-by-100 reference box while preserving its aspect ratio.
fn import(s: &mut Session, p: &Value) -> Result<Value> {
    let group = p
        .get("group")
        .map(|v| v.as_str().map(str::trim).filter(|n| !n.is_empty()).map(str::to_owned).ok_or_else(|| bad(CMD, "`group` must be a non-empty string")))
        .transpose()?;
    let params = p.clone();
    crate::jobs::run(
        s,
        "Import Shapes",
        false,
        move |ctx| {
            ctx.check()?;
            let (bytes, stem) = crate::preset_import_cmds::file_bytes_limited(&params, CMD, ".csh file", photocraft_psd::csh::MAX_FILE_BYTES as u64)?;
            let shapes = photocraft_psd::csh::parse(&bytes).map_err(|e| bad(CMD, e.to_string()))?;
            if shapes.is_empty() {
                return Err(bad(CMD, "the CSH set contains no shapes"));
            }
            let mut items = Vec::with_capacity(shapes.len());
            for shape in shapes {
                ctx.check()?;
                let [top, left, bottom, right] = shape.bounds;
                // The parser validates ordering; widen before subtraction for signed bounds.
                let width = u32::try_from(i64::from(right) - i64::from(left)).map_err(|_| bad(CMD, "shape width exceeds supported bounds"))?;
                let height = u32::try_from(i64::from(bottom) - i64::from(top)).map_err(|_| bad(CMD, "shape height exceeds supported bounds"))?;
                let path = photocraft_io::vector_map::path_from_records(&shape.path, width, height);
                if path.is_empty() {
                    return Err(bad(CMD, format!("shape {:?} has no path", shape.name)));
                }
                let path = fit(&path, [0.0, 0.0, 100.0, 100.0], true);
                // Photoshop stores some preset names as `$$$/…/Key=Shown`. The Shapes panel
                // renders the stored name, so keep the text after `=`.
                let shown = photocraft_doc::pattern::display_name(&shape.name);
                let name = if shown.trim().is_empty() { "Shape".to_owned() } else { shown.to_owned() };
                // Keep opaque IDs unchanged. An anonymous preset remains usable by name;
                // repeated IDs are checked against geometry when a filter resolves them.
                let id = (!shape.id.is_empty()).then_some(shape.id);
                items.push(ShapePreset { name, path, id });
            }
            let group = group.unwrap_or_else(|| if stem.is_empty() { "Imported Shapes".into() } else { stem });
            Ok((group, items))
        },
        |s, (group, mut items)| {
            let group = super::super::gradients::unique_group(&s.presets.shapes, &group);
            let mut taken: HashSet<String> = s.presets.shapes.iter().flat_map(|g| g.items.iter().map(|i| i.name.clone())).collect();
            taken.extend(s.edit_state.custom_shapes.iter().map(|c| c.name.clone()));
            let mut suffixes: HashMap<String, usize> = HashMap::new();
            for item in &mut items {
                let base = item.name.clone();
                let suffix = suffixes.entry(base.clone()).or_insert(2);
                while !taken.insert(item.name.clone()) {
                    item.name = format!("{base} {suffix}");
                    *suffix += 1;
                }
            }
            let names: Vec<String> = items.iter().map(|i| i.name.clone()).collect();
            let result = json!({"group": group, "count": items.len(), "names": names});
            s.presets.shapes.push(Group::new(&group, items));
            s.presets_changed();
            Ok(result)
        },
    )
}
