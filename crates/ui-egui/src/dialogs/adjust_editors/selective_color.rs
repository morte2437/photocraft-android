use photocraft_engine::adjust_cmds::RANGES;

use super::{Edit, EditorCx, RichText, Tokens, Value, adjust_params, json, nums, widgets};

// ---------------------------------------------------------------------------------------------
// Selective Color

/// The nine colours, in [`RANGES`] order.
const RANGE_LABELS: [&str; 9] = ["Reds", "Yellows", "Greens", "Cyans", "Blues", "Magentas", "Whites", "Neutrals", "Blacks"];

/// The single-colour scalar form of the params (`"colors":"blues","cyan":30`), which this editor
/// folds into the per-colour rows.
const SCALARS: [&str; 4] = ["cyan", "magenta", "yellow", "black"];

/// The colour the editor shows. The dialog keeps it in the `"colors"` field (so automation can
/// drive it with `ui.dialog.set`); the Properties panel keeps it in egui memory, because its
/// params go to `layer.setAdjustment`.
fn selected_range(ui: &egui::Ui, v: &Value, cx: &EditorCx) -> usize {
    let range = if cx.dialog {
        v.get("colors").and_then(Value::as_str).and_then(|c| RANGES.iter().position(|r| *r == c)).unwrap_or(0)
    } else {
        ui.data(|d| d.get_temp::<usize>(cx.mem.with("selc-range"))).unwrap_or(0)
    };
    range.min(RANGES.len() - 1)
}

/// Selective Color: a colour dropdown, the Cyan/Magenta/Yellow/Black sliders of that colour's row,
/// and Relative/Absolute. Changing the colour is no edit of the values.
pub(super) fn selective_color(ui: &mut egui::Ui, v: &mut Value, cx: &EditorCx) -> Edit {
    let t = Tokens::get(ui.ctx());
    let mut e = Edit::default();
    // Params in the older single-colour form move into the rows, and `relative`, which the engine
    // reads over `method`, into `method`, so the controls show what OK applies. Unparsable ones
    // stay as they are, for the command to report on OK.
    if (SCALARS.iter().any(|k| v.get(*k).is_some()) || v.get("relative").is_some())
        && let Ok(adj) = adjust_params::from_params("selectiveColor", &*v, None, photocraft_doc::ColorMode::Rgb)
    {
        let mut folded = adjust_params::to_params(&adj);
        if cx.dialog
            && let Some(colors) = v.get("colors").cloned()
            && let Some(o) = folded.as_object_mut()
        {
            o.insert("colors".into(), colors);
        }
        *v = folded;
        e.changed = true;
    }
    let range = selected_range(ui, v, cx);
    let mut picked = range;
    ui.horizontal(|ui| {
        ui.label(RichText::new(tl!("Colors")).color(t.text_dim));
        let opts: Vec<(usize, &str)> = RANGE_LABELS.iter().copied().enumerate().collect();
        widgets::dropdown(ui, &format!("selc-colors-{:x}", cx.mem.value()), tl!("Color"), &mut picked, &opts, 150.0);
    });
    if picked != range {
        let picked = picked.min(RANGES.len() - 1);
        if cx.dialog {
            if let (Some(o), Some(name)) = (v.as_object_mut(), RANGES.get(picked)) {
                o.insert("colors".into(), json!(name));
                e.changed = true;
            }
        } else {
            ui.data_mut(|d| d.insert_temp(cx.mem.with("selc-range"), picked));
        }
    }
    let range = picked.min(RANGES.len() - 1);
    let key = RANGES.get(range).copied().unwrap_or("reds");
    ui.add_space(4.0);
    let mut row = nums(v, key, [0.0; 4]);
    let mut moved = false;
    for (k, text) in [tl!("Cyan"), tl!("Magenta"), tl!("Yellow"), tl!("Black")].into_iter().enumerate() {
        let mut x = row.get(k).copied().unwrap_or(0.0).clamp(-100.0, 100.0);
        let r = widgets::slider_row(ui, text, &mut x, -100.0..=100.0, "%", None);
        if r.changed()
            && let Some(slot) = row.get_mut(k)
        {
            *slot = x.round();
            moved = true;
        }
        e.add(Edit::of(&r));
    }
    if moved && let Some(o) = v.as_object_mut() {
        // A whole new row, so a missing or malformed one becomes four numbers.
        o.insert(key.into(), json!(row));
    }
    ui.add_space(4.0);
    let method = v.get("method").and_then(Value::as_str).unwrap_or("relative").to_string();
    ui.horizontal(|ui| {
        for (m, text) in [("relative", tl!("Relative")), ("absolute", tl!("Absolute"))] {
            if ui.radio(method == m, text).clicked()
                && method != m
                && let Some(o) = v.as_object_mut()
            {
                o.insert("method".into(), json!(m));
                e.add(Edit::discrete(true));
            }
        }
    });
    e
}
