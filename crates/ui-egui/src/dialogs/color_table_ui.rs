//! Image › Mode › Color Table…: Load… and Save… for `.act` color tables, as in Photoshop.
//!
//! The dialog itself is the schema one (`filter_dialog`); this module only adds the file row.
//! Load fills the pending table from a picked `.act` (OK then applies it); Save writes the
//! pending table — the loaded colors while the Table menu is on Custom, else its preset, else
//! the document's table — to a picked `.act`. Picking a preset after Load… drops the loaded
//! colors. Parsing lives in the engine (`swatch_cmds`); this module only draws and
//! dispatches.

use serde_json::{Map, Value, json};

use crate::PhotocraftApp;

pub fn owns(fields: &Map<String, Value>) -> bool {
    fields.get("__command").and_then(Value::as_str) == Some("image.mode.colorTable")
}

fn unhex(s: &str) -> Option<[u8; 3]> {
    let f = crate::color_picker_ui::parse_hex(s)?;
    Some(f.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8))
}

/// Whether the Table menu is on Custom (or unset), the only state where loaded colors apply.
fn custom(fields: &Map<String, Value>) -> bool {
    fields.get("table").and_then(Value::as_str).is_none_or(|t| t == "custom")
}

/// A preset picked after Load… replaces the loaded table: forget its colors and transparent entry.
fn drop_loaded_unless_custom(fields: &mut Map<String, Value>) {
    if !custom(fields) {
        fields.remove("colors");
        fields.remove("transparent");
    }
}

/// The pending table: the dialog's loaded colors (while the Table menu is on Custom), else its
/// preset, else the document's table.
fn pending(app: &PhotocraftApp, fields: &Map<String, Value>) -> Option<(Vec<[u8; 3]>, Option<u16>)> {
    if custom(fields)
        && let Some(a) = fields.get("colors").and_then(Value::as_array)
    {
        let colors: Option<Vec<[u8; 3]>> = a.iter().filter_map(Value::as_str).map(unhex).collect();
        let colors = colors.filter(|c| !c.is_empty() && c.len() <= photocraft_engine::swatch_cmds::ACT_COLORS)?;
        let transparent = fields.get("transparent").and_then(Value::as_u64).and_then(|i| u16::try_from(i).ok());
        return Some((colors, transparent));
    }
    if let Some(name) = fields.get("table").and_then(Value::as_str)
        && let Some(colors) = photocraft_engine::mode_cmds::preset_table(name)
    {
        return Some((colors, None));
    }
    let t = app.session.active()?.doc.color_table.clone()?;
    Some((t.colors, t.transparent.map(u16::from)))
}

/// Load…/Save… row for the Color Table dialog (`id` finds its fields after the file picker
/// answers, as the picker is asynchronous).
pub fn buttons(app: &mut PhotocraftApp, ui: &mut egui::Ui, id: u64, fields: &mut Map<String, Value>) {
    // `fields` is the frame's copy that the dialog writes back and OK sends: drop the loaded
    // table there once a preset is picked, so OK applies what the Table menu shows.
    drop_loaded_unless_custom(fields);
    let fields = &*fields;
    ui.horizontal(|ui| {
        if ui.button(tl!("Load…")).clicked()
            && let Err(e) = app.pick_file_bytes_filtered(&["act"], move |app, name, bytes| {
                let (colors, transparent) =
                    photocraft_engine::swatch_cmds::act_table(&bytes).map_err(|e| format!("{}: {e}", crate::file_open::display_name(&name)))?;
                // The dialog may close while the picker is open; then there is nothing to fill.
                let Some(d) = app.ui.dialogs.iter_mut().find(|d| d.id == id) else { return Ok(Value::Null) };
                d.fields.insert("table".into(), json!("custom"));
                d.fields.insert("colors".into(), Value::Array(colors.into_iter().map(Value::String).collect()));
                d.fields.insert("transparent".into(), transparent.map_or(Value::Null, |i| json!(i)));
                Ok(Value::Null)
            })
            && e != crate::file_dialog::CANCELLED
        {
            app.ui.status = e;
            app.ui.status_error = true;
        }
        if ui.button(tl!("Save…")).clicked() {
            let Some((colors, transparent)) = pending(app, fields) else {
                app.ui.status = tl!("There is no color table to save.").into();
                app.ui.status_error = true;
                return;
            };
            let bytes = match photocraft_engine::swatch_cmds::act_bytes(&colors, transparent) {
                Ok(bytes) => bytes,
                Err(e) => {
                    app.ui.status = e;
                    app.ui.status_error = true;
                    return;
                }
            };
            let n = colors.len() as u64;
            if let Err(e) = app.pick_save("Table.act", move |app, path| {
                let write = app.services.write.as_mut().ok_or("no writer configured")?;
                write(&path, &bytes)?;
                app.ui.status_error = false;
                app.ui.status = crate::i18n::fmt(
                    &crate::i18n::trn(crate::i18n::current(), n, "Saved {n} color to {file}", "Saved {n} colors to {file}"),
                    &[("file", &crate::file_open::display_name(&path))],
                );
                Ok(json!({"path": path}))
            }) {
                app.ui.status = e;
                app.ui.status_error = true;
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn app() -> PhotocraftApp {
        PhotocraftApp::new(photocraft_engine::Session::new(), Default::default())
    }

    #[test]
    fn owns_only_the_color_table_dialog() {
        assert!(owns(&Map::from_iter([("__command".to_string(), json!("image.mode.colorTable"))])));
        assert!(!owns(&Map::from_iter([("__command".to_string(), json!("image.mode.indexedColor"))])));
        assert!(!owns(&Map::new()));
    }

    #[test]
    fn pending_prefers_dialog_colors_then_preset() {
        let app = app();
        // Loaded colors win while the Table menu is on Custom, with their transparent entry.
        let loaded = |table: &str| {
            Map::from_iter([("table".to_string(), json!(table)), ("colors".to_string(), json!(["#ff0000", "#00ff00"])), ("transparent".to_string(), json!(1))])
        };
        assert_eq!(pending(&app, &loaded("custom")), Some((vec![[255, 0, 0], [0, 255, 0]], Some(1))));
        // A preset picked after Load… is what Save… writes, not the loaded colors.
        let (colors, transparent) = pending(&app, &loaded("blackBody")).unwrap();
        assert_eq!((colors.len(), transparent), (256, None));
        // Else the preset (256 entries, no transparency).
        let fields = Map::from_iter([("table".to_string(), json!("blackBody"))]);
        let (colors, transparent) = pending(&app, &fields).unwrap();
        assert_eq!((colors.len(), transparent), (256, None));
        assert_eq!(colors[0], [0, 0, 0]);
        // Nothing pending without colors, preset or document table.
        assert_eq!(pending(&app, &Map::from_iter([("table".to_string(), json!("custom"))])), None);
    }

    /// What OK sends is the dialog's fields: after a preset is picked they hold no loaded colors
    /// (which `image.mode.colorTable` would apply over the preset), and Custom keeps them.
    #[test]
    fn a_preset_picked_after_load_drops_the_loaded_colors() {
        let loaded = |table: &str| {
            Map::from_iter([
                ("__command".to_string(), json!("image.mode.colorTable")),
                ("table".to_string(), json!(table)),
                ("colors".to_string(), json!(["#ff0000"])),
                ("transparent".to_string(), json!(0)),
            ])
        };
        let mut fields = loaded("grayscale");
        drop_loaded_unless_custom(&mut fields);
        assert!(!fields.contains_key("colors") && !fields.contains_key("transparent"), "{fields:?}");
        assert_eq!(fields.get("table"), Some(&json!("grayscale")));
        let mut fields = loaded("custom");
        drop_loaded_unless_custom(&mut fields);
        assert_eq!(fields, loaded("custom"));
    }

    #[test]
    fn loaded_act_bytes_fill_the_dialog_fields() {
        let table: Vec<[u8; 3]> = (0..256).map(|i| [i as u8, 0, 255]).collect();
        let bytes = photocraft_engine::swatch_cmds::act_bytes(&table, Some(3)).unwrap();
        let (colors, transparent) = photocraft_engine::swatch_cmds::act_table(&bytes).unwrap();
        assert_eq!((colors.len(), transparent), (256, Some(3)));
        // What OK would apply: the table's colors and transparent entry.
        let fields = Map::from_iter([
            ("table".to_string(), json!("custom")),
            ("colors".to_string(), Value::Array(colors.into_iter().map(Value::String).collect())),
            ("transparent".to_string(), json!(3)),
        ]);
        let app = app();
        assert_eq!(pending(&app, &fields), Some((table, Some(3))));
    }
}
