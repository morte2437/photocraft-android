//! The user contour library of the Layer Style dialog (`layer.layerStyle.contours.*`).
//!
//! Photoshop's Contour Editor "New…" saves the edited curve under a name, and the contour picker
//! lists those contours after its standard ones (picker menu: Load, Save and Replace Contours).
//! The library is [`PresetState::contours`](crate::presets::PresetState); like the other preset
//! sets it lives in the preferences document and changes the preferences, not a document's
//! history. A user contour is applied to an effect as a `contour` curve object (see
//! [`contour_param`]); `.shc` files go through `photocraft_psd::shc` (see [`contours_from_shc`]).

use photocraft_doc::Contour;
use photocraft_doc::adjust::CurvePoint;
use serde_json::{Value, json};

use crate::commands::CommandSpec;
use crate::layer_style::{builtin_contour, contour_from_param, contour_param, contour_to_shc_all, contours_from_shc};
use crate::presets::{always, bad, str_param};
use crate::{Result, Session};

/// Most user contours kept (further ones are refused / dropped on load).
pub const MAX_USER_CONTOURS: usize = 1000;
/// Longest contour name kept, in characters.
pub const MAX_NAME: usize = 255;

fn name_of(c: &Contour) -> &str {
    match c {
        Contour::Linear => "Linear",
        Contour::Custom { name, .. } => name,
    }
}

fn clip(s: &str) -> String {
    s.chars().take(MAX_NAME).collect()
}

/// `c` as a named custom contour (Linear becomes the identity curve), so it can be stored.
fn named(c: Contour, name: &str) -> Contour {
    match c {
        Contour::Linear => Contour::Custom {
            name: name.to_string(),
            points: vec![CurvePoint { input: 0.0, output: 0.0 }, CurvePoint { input: 1.0, output: 1.0 }],
            corners: Vec::new(),
        },
        Contour::Custom { points, corners, .. } => Contour::Custom { name: name.to_string(), points, corners },
    }
}

/// `base`, or `base 2`, `base 3`… so a name is neither a standard contour's nor a user contour's
/// (a name alone selects a standard contour, so a user contour may not shadow one).
fn unique(lib: &[Contour], base: &str) -> String {
    let taken = |n: &str| builtin_contour(n).is_some() || lib.iter().any(|c| name_of(c) == n);
    if !taken(base) {
        return base.to_string();
    }
    (2..).map(|i| format!("{base} {i}")).find(|n| !taken(n)).unwrap_or_else(|| base.to_string())
}

/// Adds `c` under `name` (made unique); the stored name.
fn add(lib: &mut Vec<Contour>, c: Contour, name: &str) -> String {
    let n = unique(lib, &clip(name));
    lib.push(named(c, &n));
    n
}

/// The library as saved with the preferences.
pub fn to_json(lib: &[Contour]) -> Value {
    Value::Array(lib.iter().map(contour_param).collect())
}

/// Restores a saved library, skipping entries that aren't usable contours; never refuses.
pub fn from_json(v: &Value) -> Vec<Contour> {
    let mut lib = Vec::new();
    for e in v.as_array().into_iter().flatten().take(MAX_USER_CONTOURS) {
        // Entries are curve objects; a bare name would only name a standard contour.
        if let Some(c) = e.is_object().then(|| contour_from_param(e)).flatten() {
            add(&mut lib, c.clone(), name_of(&c));
        }
    }
    lib
}

fn list(s: &mut Session, _: &Value) -> Result<Value> {
    let lib = &s.presets.contours;
    Ok(json!({"count": lib.len(), "contours": lib.iter().map(|c| json!({"name": name_of(c), "contour": contour_param(c)})).collect::<Vec<_>>()}))
}

fn new(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.layerStyle.contours.new";
    let c = contour_from_param(p.get("contour").ok_or_else(|| bad(CMD, "missing `contour` (a standard contour's name or a curve object)"))?)
        .ok_or_else(|| bad(CMD, "`contour` is not a usable contour (unknown name, or a curve with fewer than 2 or more than 64 points)"))?;
    if s.presets.contours.len() >= MAX_USER_CONTOURS {
        return Err(bad(CMD, format!("the contour library is full ({MAX_USER_CONTOURS} contours)")));
    }
    let name = match p.get("name") {
        None | Some(Value::Null) => format!("Contour {}", s.presets.contours.len() + 1),
        Some(Value::String(n)) if !n.trim().is_empty() => n.trim().to_string(),
        Some(_) => return Err(bad(CMD, "`name` must be a non-empty string")),
    };
    let name = add(&mut s.presets.contours, c, &name);
    s.presets_changed();
    let contour = s.presets.contours.last().map(contour_param).unwrap_or(Value::Null);
    Ok(json!({"name": name, "contour": contour, "count": s.presets.contours.len()}))
}

fn delete(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.layerStyle.contours.delete";
    let name = str_param(p, "name").ok_or_else(|| bad(CMD, "missing `name`"))?;
    let at = s
        .presets
        .contours
        .iter()
        .position(|c| name_of(c) == name)
        .ok_or_else(|| bad(CMD, format!("no user contour \"{name}\" (see layer.layerStyle.contours.list)")))?;
    s.presets.contours.remove(at);
    s.presets_changed();
    Ok(json!({"deleted": name, "count": s.presets.contours.len()}))
}

fn load(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.layerStyle.contours.load";
    let replace = match str_param(p, "mode") {
        None | Some("append") => false,
        Some("replace") => true,
        Some(m) => return Err(bad(CMD, format!("unknown mode `{m}` (append|replace)"))),
    };
    let (bytes, _) = crate::preset_import_cmds::file_bytes(p, CMD, ".shc file")?;
    let found = contours_from_shc(&bytes)?;
    let loaded: Vec<Contour> = found.iter().filter_map(contour_from_param).collect();
    if loaded.is_empty() {
        return Err(bad(CMD, "the file holds no usable contours"));
    }
    let mut lib = if replace { Vec::new() } else { s.presets.contours.clone() };
    let mut names = Vec::new();
    for c in loaded {
        if lib.len() >= MAX_USER_CONTOURS {
            break;
        }
        let n = name_of(&c).to_string();
        names.push(add(&mut lib, c, &n));
    }
    s.presets.contours = lib;
    s.presets_changed();
    Ok(json!({"loaded": names, "count": s.presets.contours.len(), "skipped": found.len() - names.len()}))
}

fn save(s: &mut Session, p: &Value) -> Result<Value> {
    const CMD: &str = "layer.layerStyle.contours.save";
    let lib: Vec<&Contour> = match p.get("names") {
        None | Some(Value::Null) => s.presets.contours.iter().collect(),
        Some(Value::Array(a)) => {
            let mut v = Vec::new();
            for n in a {
                let n = n.as_str().ok_or_else(|| bad(CMD, "`names` must be strings"))?;
                v.push(s.presets.contours.iter().find(|c| name_of(c) == n).ok_or_else(|| bad(CMD, format!("no user contour \"{n}\"")))?);
            }
            v
        }
        Some(_) => return Err(bad(CMD, "`names` must be a list of contour names")),
    };
    if lib.is_empty() {
        return Err(bad(CMD, "there are no user contours to save"));
    }
    let bytes = contour_to_shc_all(&lib)?;
    let mut out = json!({"count": lib.len(), "bytes": bytes.len()});
    match str_param(p, "path") {
        Some(path) => {
            crate::file_cmds::write_file(path, &bytes)?;
            out["path"] = json!(path);
        }
        None => out["data"] = json!(photocraft_paint::tile::b64_encode(&bytes)),
    }
    Ok(out)
}

/// Contour library command specs.
pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec {
            id: "layer.layerStyle.contours.list",
            label: "List User Contours",
            menu: &[],
            shortcut: None,
            params: "{} → {count, contours:[{name, contour: curve object usable as any `contour` param}]}",
            enabled: always,
            run: list,
            journal: false,
        },
        CommandSpec {
            id: "layer.layerStyle.contours.new",
            label: "New Contour…",
            menu: &[],
            shortcut: None,
            params: r##"{"contour":name|{"name":str,"points":[[in 0..1,out 0..1],…],"corners":[bool,…]},"name":str?="Contour N"} → {name (made unique, never a standard contour's name), contour, count} (Contour Editor › New…)"##,
            enabled: always,
            run: new,
            journal: true,
        },
        CommandSpec {
            id: "layer.layerStyle.contours.delete",
            label: "Delete Contour",
            menu: &[],
            shortcut: None,
            params: r##"{"name":str} → {deleted, count}"##,
            enabled: always,
            run: delete,
            journal: true,
        },
        CommandSpec {
            id: "layer.layerStyle.contours.load",
            label: "Load Contours…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".shc file"?,"data":base64 bytes?,"mode":"append"|"replace"="append"} → {loaded, count, skipped} (picker menu: Load Contours… / Replace Contours…)"##,
            enabled: always,
            run: load,
            journal: true,
        },
        CommandSpec {
            id: "layer.layerStyle.contours.save",
            label: "Save Contours…",
            menu: &[],
            shortcut: None,
            params: r##"{"path":".shc file"?,"names":[str]?=all} → {count, bytes, path | data (base64 when no path)}"##,
            enabled: always,
            run: save,
            journal: false,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn curve(name: &str) -> Value {
        json!({"name": name, "points": [[0.0, 0.0], [0.3, 0.9], [1.0, 1.0]], "corners": [false, true, false]})
    }

    #[test]
    fn new_save_load_round_trips_and_names_stay_unique() {
        let mut s = Session::new();
        let r = new(&mut s, &json!({"contour": curve("x"), "name": "Ramp"})).unwrap();
        assert_eq!(r["name"], "Ramp");
        // A standard contour's name can't be shadowed; repeats get a suffix.
        assert_eq!(new(&mut s, &json!({"contour": curve("x"), "name": "Cone"})).unwrap()["name"], "Cone 2");
        assert_eq!(new(&mut s, &json!({"contour": "Ring", "name": "Ramp"})).unwrap()["name"], "Ramp 2");
        let saved = save(&mut s, &json!({})).unwrap();
        let data = saved["data"].clone();
        assert_eq!(saved["count"], 3);
        // Replace swaps the library for the file's contours; the curve and its corner survive.
        delete(&mut s, &json!({"name": "Ramp"})).unwrap();
        let r = load(&mut s, &json!({"data": data, "mode": "replace"})).unwrap();
        assert_eq!(r["count"], 3);
        let c = contour_from_param(&list(&mut s, &json!({})).unwrap()["contours"][0]["contour"]).unwrap();
        assert!(c.is_corner(1));
        // Appending the same file again keeps every name unique.
        let r = load(&mut s, &json!({"data": data})).unwrap();
        assert_eq!(r["count"], 6);
        let names: std::collections::HashSet<_> = s.presets.contours.iter().map(|c| name_of(c).to_string()).collect();
        assert_eq!(names.len(), 6);
    }

    #[test]
    fn library_persists_with_the_presets() {
        let mut s = Session::new();
        new(&mut s, &json!({"contour": curve("x"), "name": "Mine"})).unwrap();
        let saved = s.presets.to_json(&s);
        let mut t = Session::new();
        t.load_presets_json(saved);
        assert_eq!(t.presets.contours.len(), 1);
        assert_eq!(name_of(&t.presets.contours[0]), "Mine");
        assert_eq!(t.presets.contours, s.presets.contours);
    }

    #[test]
    fn bad_params_are_errors_not_panics() {
        let mut s = Session::new();
        for p in
            [json!({}), json!({"contour": 5}), json!({"contour": "Nope"}), json!({"contour": {"points": [[0, 0]]}}), json!({"contour": curve("x"), "name": 3})]
        {
            assert!(new(&mut s, &p).is_err(), "{p}");
        }
        assert!(delete(&mut s, &json!({"name": "ghost"})).is_err());
        assert!(delete(&mut s, &json!({})).is_err());
        assert!(save(&mut s, &json!({})).is_err(), "empty library");
        new(&mut s, &json!({"contour": curve("x"), "name": "Existing"})).unwrap();
        let before = s.presets.contours.clone();
        for p in [
            json!({}),
            json!({"data": 3}),
            json!({"data": "!!"}),
            json!({"data": "AAAA"}),
            json!({"data": "AAAA", "mode": "replace"}),
            json!({"data": "AAAA", "mode": "x"}),
            json!({"path": "/nonexistent/x.shc"}),
        ] {
            assert!(load(&mut s, &p).is_err(), "{p}");
            assert_eq!(s.presets.contours, before, "{p}");
        }
        assert_eq!(s.presets.contours, before);
        let big = from_json(&json!([{"points": [[0, 0]]}, "Ring", 7, {"name": "Cone", "points": [[0, 0], [1, 1]]}]));
        assert_eq!(big.len(), 1);
        assert_eq!(name_of(&big[0]), "Cone 2");
    }
    #[test]
    fn malformed_persisted_library_keeps_existing_contours() {
        let mut s = Session::new();
        new(&mut s, &json!({"contour": curve("x"), "name": "Existing"})).unwrap();
        let before = s.presets.contours.clone();
        for value in [json!(null), json!(3), json!("bad"), json!({})] {
            s.load_presets_json(json!({"contours": value}));
            assert_eq!(s.presets.contours, before);
        }
        s.load_presets_json(json!({"contours": []}));
        assert!(s.presets.contours.is_empty());
    }
}
