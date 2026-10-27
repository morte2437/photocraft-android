//! Photoshop's brush keys (#626): `[` / `]` step the brush size and ⇧[ / ⇧] the hardness. Photoshop
//! has no menu item for them but lists them in Edit › Keyboard Shortcuts › Tools ("Decrease Brush
//! Size"…); here they are ordinary commands with default shortcuts, so they can be rebound,
//! recorded and driven by agents. Each one is a `tools.setBrush` on the session brush.
//!
//! `,` / `.` pick the previous or next brush preset and ⇧, / ⇧. the first or last, in the order
//! the Brushes panel lists them (Photoshop's "Previous Brush"… in Keyboard Shortcuts › Tools).
//!
//! `tools.rotateBrush` turns the tip: Photoshop's ← / → while painting (1°, ⇧ 15°). Arrows mean
//! other things with other tools, so the shell gives them to it only for the painting tools
//! (`move_mods::arrow_keys`).

use serde_json::{Value, json};

use crate::commands::{CommandSpec, always};
use crate::{EngineError, Result, Session};

/// Photoshop's largest brush diameter, in pixels.
pub const MAX_SIZE: f32 = 5000.0;

/// `[` / `]`: 25 % smaller or larger, and at least a pixel either way so small brushes still grow
/// and shrink (2 / 1.25 rounds back to 2, which kept `[` from reaching 1 px, #3229).
fn size_step(size: f32, up: bool) -> f32 {
    let next = if up { (size * 1.25).round().max(size + 1.0) } else { (size / 1.25).round().min(size - 1.0) };
    next.clamp(1.0, MAX_SIZE)
}

/// ⇧[ / ⇧]: to the previous or next quarter (0, 25, 50, 75, 100 %).
fn hardness_step(hardness: f32, up: bool) -> f32 {
    let quarter = (hardness * 4.0).round();
    let next = if up { quarter + 1.0 } else { quarter - 1.0 };
    next.clamp(0.0, 4.0) / 4.0
}

fn size(s: &mut Session, up: bool) -> Result<Value> {
    let size = size_step(s.tools.brush.size, up);
    crate::brush_cmds::set_brush(s, &json!({ "brush": { "size": size } }))
}

/// The Brushes panel's label for presets saved without a group: they share its group with presets
/// in a group of that name.
pub const UNGROUPED: &str = "My Brushes";

/// Brush preset indices in the Brushes panel's order: groups in order of first appearance (no
/// group is [`UNGROUPED`]), and in each group its folders where their first preset is, depth first
/// (as `brushes_tab` draws them). A preset its name can't select (an earlier one has the same name
/// up to case, and `tools.setBrush` picks that one) is left out afterwards, so stepping never
/// gets stuck.
fn panel_order(presets: &[photocraft_paint::BrushPreset]) -> Vec<usize> {
    fn path(p: &photocraft_paint::BrushPreset) -> &[String] {
        p.folder.get(..p.folder.len().min(crate::brush_preset_cmds::MAX_FOLDER_DEPTH)).unwrap_or_default()
    }
    fn flatten(presets: &[photocraft_paint::BrushPreset], items: &[usize], depth: usize, out: &mut Vec<usize>) {
        let mut seen: Vec<&str> = Vec::new();
        for &i in items {
            let Some(p) = presets.get(i) else { continue };
            match path(p).get(depth) {
                None => out.push(i),
                Some(name) if !seen.contains(&name.as_str()) => {
                    seen.push(name);
                    let inside: Vec<usize> = items.iter().copied().filter(|&j| presets.get(j).is_some_and(|q| path(q).get(depth) == Some(name))).collect();
                    flatten(presets, &inside, depth + 1, out);
                }
                Some(_) => {}
            }
        }
    }
    let mut groups: Vec<(&str, Vec<usize>)> = Vec::new();
    for (i, p) in presets.iter().enumerate() {
        let group = if p.group.is_empty() { UNGROUPED } else { p.group.as_str() };
        match groups.iter_mut().find(|(g, _)| *g == group) {
            Some((_, v)) => v.push(i),
            None => groups.push((group, vec![i])),
        }
    }
    let mut out = Vec::with_capacity(presets.len());
    for (_, items) in &groups {
        flatten(presets, items, 0, &mut out);
    }
    // Groups and folders are placed by every preset, as the panel places them; only then are the
    // presets their names can't select left out.
    out.retain(|&i| presets.get(i).is_some_and(|p| photocraft_paint::presets::find(presets, &p.name).is_some_and(|q| std::ptr::eq(q, p))));
    out
}

/// Which way [`step_preset`] moves through the presets.
#[derive(Clone, Copy)]
enum Step {
    Previous,
    Next,
    First,
    Last,
}

/// `,` / `.` / ⇧, / ⇧.: pick the previous, next, first or last brush preset (wrapping around), as
/// a click in the Brushes panel would (`tools.setBrush` with the preset). With no preset picked
/// yet, Next starts at the first and Previous at the last.
fn step_preset(s: &mut Session, step: Step) -> Result<Value> {
    let order = panel_order(&s.tools.presets);
    let n = order.len();
    if n == 0 {
        return Err(EngineError::Other("there are no brush presets".into()));
    }
    let at = s.tools.current_preset.as_deref().and_then(|c| order.iter().position(|&i| s.tools.presets.get(i).is_some_and(|p| p.name.eq_ignore_ascii_case(c))));
    let k = match (step, at) {
        (Step::First, _) | (Step::Next, None) => 0,
        (Step::Last, _) | (Step::Previous, None) => n - 1,
        (Step::Next, Some(k)) => (k + 1) % n,
        (Step::Previous, Some(k)) => (k + n - 1) % n,
    };
    let name = order.get(k).and_then(|&i| s.tools.presets.get(i)).map(|p| p.name.clone()).ok_or_else(|| EngineError::Other("no such brush preset".into()))?;
    crate::brush_cmds::set_brush(s, &json!({ "preset": name }))?;
    Ok(json!({ "preset": name }))
}

fn hardness(s: &mut Session, up: bool) -> Result<Value> {
    let hardness = hardness_step(s.tools.brush.hardness, up);
    crate::brush_cmds::set_brush(s, &json!({ "brush": { "hardness": hardness } }))
}

/// The brush angle turned by `degrees` (positive is anticlockwise, as the angle field counts),
/// wrapped into Photoshop's -180..=180.
fn turned(angle: f32, degrees: f32) -> f32 {
    let a = (angle + degrees).rem_euclid(360.0);
    if a > 180.0 { a - 360.0 } else { a }
}

fn rotate(s: &mut Session, p: &Value) -> Result<Value> {
    let by = p
        .get("degrees")
        .and_then(Value::as_f64)
        .filter(|d| d.is_finite() && d.abs() <= 360.0)
        .ok_or_else(|| EngineError::BadParams { cmd: "tools.rotateBrush".into(), msg: "`degrees` must be a number from -360 to 360".into() })?;
    let angle = turned(s.tools.brush.angle, by as f32);
    crate::brush_cmds::set_brush(s, &json!({ "brush": { "angle": angle } }))
}

macro_rules! key_cmd {
    ($id:literal, $label:literal, $sc:literal, $run:expr) => {
        CommandSpec { id: $id, label: $label, menu: &[], shortcut: Some($sc), params: "{}", enabled: always, run: $run, journal: true }
    };
}

pub fn specs() -> Vec<CommandSpec> {
    vec![
        key_cmd!("tools.decreaseBrushSize", "Decrease Brush Size", "[", |s, _| size(s, false)),
        key_cmd!("tools.increaseBrushSize", "Increase Brush Size", "]", |s, _| size(s, true)),
        key_cmd!("tools.decreaseBrushHardness", "Decrease Brush Hardness", "Shift+[", |s, _| hardness(s, false)),
        key_cmd!("tools.increaseBrushHardness", "Increase Brush Hardness", "Shift+]", |s, _| hardness(s, true)),
        key_cmd!("tools.previousBrush", "Previous Brush", ",", |s, _| step_preset(s, Step::Previous)),
        key_cmd!("tools.nextBrush", "Next Brush", ".", |s, _| step_preset(s, Step::Next)),
        key_cmd!("tools.firstBrush", "First Brush", "Shift+,", |s, _| step_preset(s, Step::First)),
        key_cmd!("tools.lastBrush", "Last Brush", "Shift+.", |s, _| step_preset(s, Step::Last)),
        CommandSpec {
            id: "tools.rotateBrush",
            label: "Rotate Brush Tip",
            menu: &[],
            shortcut: None,
            params: r##"{"degrees":-360..360 (positive anticlockwise; ← / → while painting send 1, ⇧ 15)}"##,
            enabled: always,
            run: rotate,
            journal: true,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steps_like_photoshop() {
        assert_eq!(size_step(1.0, false), 1.0, "never below 1 px");
        assert_eq!(size_step(1.0, true), 2.0, "small brushes still grow");
        assert_eq!(size_step(2.0, false), 1.0, "small brushes still shrink (#3229)");
        assert_eq!(size_step(3.0, false), 2.0);
        assert_eq!(size_step(4.0, false), 3.0);
        for size in [2.0, 3.0, 5.0, 8.0, 13.0, 100.0] {
            assert!(size_step(size, false) < size, "`[` always shrinks {size}");
            assert!(size_step(size, true) > size, "`]` always grows {size}");
        }
        assert_eq!(size_step(100.0, true), 125.0);
        assert_eq!(size_step(125.0, false), 100.0);
        assert_eq!(size_step(MAX_SIZE, true), MAX_SIZE, "never past the largest brush");
        assert_eq!(hardness_step(0.5, true), 0.75);
        assert_eq!(hardness_step(0.6, false), 0.25, "snaps to the quarter below the nearest");
        assert_eq!(hardness_step(1.0, true), 1.0);
        assert_eq!(hardness_step(0.0, false), 0.0);
    }

    #[test]
    fn rotate_brush_turns_the_tip_and_wraps_at_180() {
        assert_eq!(turned(0.0, 1.0), 1.0);
        assert_eq!(turned(179.0, 15.0), -166.0);
        assert_eq!(turned(-175.0, -15.0), 170.0);
        assert_eq!(turned(180.0, 0.0), 180.0);
        let mut s = Session::new();
        s.execute("tools.rotateBrush", json!({"degrees": -15})).unwrap();
        assert_eq!(s.tools.brush.angle, -15.0);
        for bad in [json!({}), json!({"degrees": "x"}), json!({"degrees": 1e9}), json!(null)] {
            assert!(s.execute("tools.rotateBrush", bad.clone()).is_err(), "{bad}");
        }
        assert_eq!(s.tools.brush.angle, -15.0);
    }

    fn preset(name: &str, group: &str, folder: &[&str], size: f32) -> photocraft_paint::BrushPreset {
        photocraft_paint::BrushPreset {
            name: name.into(),
            brush: photocraft_paint::BrushSettings { size, ..Default::default() },
            builtin: false,
            group: group.into(),
            folder: folder.iter().map(|f| f.to_string()).collect(),
            captures: Default::default(),
        }
    }

    #[test]
    fn panel_order_groups_then_folders_depth_first() {
        let presets = vec![
            preset("a", "G1", &[], 1.0),
            preset("b", "G2", &[], 2.0),
            preset("c", "G1", &["F"], 3.0),
            preset("d", "G1", &[], 4.0),
            preset("e", "G1", &["F"], 5.0),
            preset("f", "G1", &["F", "Sub"], 6.0),
            preset("g", "G1", &["F"], 7.0),
        ];
        let names: Vec<&str> = panel_order(&presets).into_iter().map(|i| presets[i].name.as_str()).collect();
        // G1: a, folder F (c, e, then Sub's f where its first preset is, g), d; then G2: b.
        assert_eq!(names, ["a", "c", "e", "f", "g", "d", "b"]);
        // No group is the panel's "My Brushes": one group with presets named into it.
        let presets = vec![preset("x", "", &[], 1.0), preset("y", UNGROUPED, &[], 1.0), preset("z", "", &[], 1.0), preset("w", "G", &[], 1.0)];
        let names: Vec<&str> = panel_order(&presets).into_iter().map(|i| presets[i].name.as_str()).collect();
        assert_eq!(names, ["x", "y", "z", "w"]);
        // A name only a case apart from an earlier one can't be picked by name: skipped.
        let presets = vec![preset("Foo", "", &[], 1.0), preset("foo", "", &[], 1.0), preset("Bar", "", &[], 1.0)];
        let names: Vec<&str> = panel_order(&presets).into_iter().map(|i| presets[i].name.as_str()).collect();
        assert_eq!(names, ["Foo", "Bar"]);
        // A skipped duplicate still places its group where the panel does: G2 (from `foo`) comes
        // before G3, so the panel's Foo, foo, Baz, Bar steps as Foo, Baz, Bar.
        let presets = vec![preset("Foo", "G1", &[], 1.0), preset("foo", "G2", &[], 1.0), preset("Bar", "G3", &[], 1.0), preset("Baz", "G2", &[], 1.0)];
        let names: Vec<&str> = panel_order(&presets).into_iter().map(|i| presets[i].name.as_str()).collect();
        assert_eq!(names, ["Foo", "Baz", "Bar"]);
    }

    #[test]
    fn previous_and_next_brush_walk_the_presets_and_wrap() {
        let mut s = Session::new();
        s.tools.presets = vec![preset("One", "", &[], 11.0), preset("Two", "", &[], 22.0), preset("Three", "", &[], 33.0)];
        s.tools.current_preset = None;
        let picked = |s: &Session| (s.tools.current_preset.clone().unwrap_or_default(), s.tools.brush.size);
        s.execute("tools.nextBrush", Value::Null).unwrap();
        assert_eq!(picked(&s), ("One".into(), 11.0), "Next with no preset picked starts at the first");
        s.execute("tools.nextBrush", Value::Null).unwrap();
        assert_eq!(picked(&s), ("Two".into(), 22.0));
        s.execute("tools.lastBrush", Value::Null).unwrap();
        assert_eq!(picked(&s), ("Three".into(), 33.0));
        s.execute("tools.nextBrush", Value::Null).unwrap();
        assert_eq!(picked(&s).0, "One", "wraps past the last");
        s.execute("tools.previousBrush", Value::Null).unwrap();
        assert_eq!(picked(&s).0, "Three", "wraps before the first");
        s.execute("tools.firstBrush", Value::Null).unwrap();
        assert_eq!(picked(&s).0, "One");
        let sc = |id: &str| crate::commands::find(id).and_then(|c| c.shortcut);
        assert_eq!(
            [sc("tools.previousBrush"), sc("tools.nextBrush"), sc("tools.firstBrush"), sc("tools.lastBrush")],
            [Some(","), Some("."), Some("Shift+,"), Some("Shift+.")]
        );
        // No presets: an error, and the brush and the current preset stay as they were.
        s.tools.presets.clear();
        let (brush, current) = (s.tools.brush.clone(), s.tools.current_preset.clone());
        for id in ["tools.previousBrush", "tools.nextBrush", "tools.firstBrush", "tools.lastBrush"] {
            assert!(s.execute(id, Value::Null).is_err(), "{id}");
        }
        assert_eq!((s.tools.brush.clone(), s.tools.current_preset.clone()), (brush, current));
    }

    #[test]
    fn commands_step_the_session_brush_and_journal() {
        let mut s = Session::new();
        s.execute("tools.setBrush", json!({"brush": {"size": 40, "hardness": 0.5}})).unwrap();
        s.execute("tools.increaseBrushSize", json!({})).unwrap();
        assert_eq!(s.tools.brush.size, 50.0);
        s.execute("tools.decreaseBrushSize", Value::Null).unwrap();
        assert_eq!(s.tools.brush.size, 40.0);
        // Pixel artists: `[` held down reaches 1 px (#3229).
        s.execute("tools.setBrush", json!({"brush": {"size": 3}})).unwrap();
        for expected in [2.0, 1.0, 1.0] {
            s.execute("tools.decreaseBrushSize", Value::Null).unwrap();
            assert_eq!(s.tools.brush.size, expected);
        }
        s.execute("tools.increaseBrushHardness", json!({})).unwrap();
        assert_eq!(s.tools.brush.hardness, 0.75);
        s.execute("tools.decreaseBrushHardness", json!({"junk": [1, 2]})).unwrap();
        assert_eq!(s.tools.brush.hardness, 0.5);
        assert_eq!(s.journal.last().map(|(id, _)| id.as_str()), Some("tools.decreaseBrushHardness"));
        // They need no document and are bindable under Tools.
        let sc = |id| crate::commands::find(id).and_then(|c| c.shortcut);
        assert_eq!(sc("tools.decreaseBrushSize"), Some("["));
        assert_eq!(sc("tools.increaseBrushHardness"), Some("Shift+]"));
    }
}
