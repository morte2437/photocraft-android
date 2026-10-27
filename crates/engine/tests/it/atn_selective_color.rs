//! #7672: strict ATN export must preserve both accepted Selective Color parameter forms.
use photocraft_engine::actions_cmds::Action;
use photocraft_engine::{Session, atn_cmds};
use serde_json::{Value, json};

const RANGES: [&str; 9] = ["reds", "yellows", "greens", "cyans", "blues", "magentas", "whites", "neutrals", "blacks"];
const COMMANDS: [&str; 2] = ["image.adjustments.selectiveColor", "layer.newAdjustmentLayer.selectiveColor"];

fn fixture(depth: u32) -> Session {
    let mut s = Session::new();
    s.execute("file.new", json!({"width": 10, "height": 2, "depth": depth})).unwrap();
    let palette = [
        [0.85, 0.10, 0.05, 1.0],
        [0.85, 0.80, 0.05, 1.0],
        [0.05, 0.85, 0.10, 1.0],
        [0.05, 0.80, 0.85, 1.0],
        [0.10, 0.05, 0.85, 1.0],
        [0.85, 0.05, 0.80, 1.0],
        [0.90, 0.93, 0.95, 1.0],
        [0.45, 0.50, 0.55, 1.0],
        [0.05, 0.08, 0.10, 1.0],
        [0.65, 0.20, 0.70, 0.4],
    ];
    s.edit("synthetic colours", |doc, active| {
        let surface = doc.layer_mut(active.unwrap()).unwrap().surface_mut().unwrap();
        let format = surface.format();
        let bytes: Vec<_> = palette.iter().cycle().take(20).flat_map(|rgba| photocraft_raster::from_rgba(&format, *rgba)).collect();
        surface.write_region(doc_bounds(), &bytes);
        Ok(())
    })
    .unwrap();
    s
}

fn doc_bounds() -> photocraft_doc::Rect {
    photocraft_doc::Rect::from_xywh(0, 0, 10, 2)
}

fn pixels(s: &Session) -> Vec<[f32; 4]> {
    photocraft_compose::flatten(&s.active().unwrap().doc).px
}

fn check_cases(id: &str, cases: &[Value], require_effect: bool) {
    let mut failures = Vec::new();
    for depth in [8, 16, 32] {
        for params in cases {
            let action = Action::new("Selective Color", vec![(id.into(), params.clone())]);
            let mut direct = fixture(depth);
            let before = pixels(&direct);
            direct.execute(id, params.clone()).unwrap();
            let expected = pixels(&direct);
            if require_effect {
                assert_ne!(expected, before, "ineffective fixture: {id}, depth {depth}, {params}");
            }

            let mut native = fixture(depth);
            native.actions.list.push(action.clone());
            native.execute("actions.play", json!({"action": action.name})).unwrap();
            assert_eq!(pixels(&native), expected, "native replay: {id}, depth {depth}, {params}");

            let (bytes, skipped) = atn_cmds::export("Regression", &[&action], &[], false).unwrap();
            assert!(skipped.is_empty());
            let imported = atn_cmds::import(&bytes).unwrap();
            assert!(imported.unsupported.is_empty());
            assert!(imported.warnings.is_empty());
            assert_eq!(imported.actions.len(), 1);
            assert_eq!(imported.actions[0].steps.len(), 1);
            assert_eq!(imported.actions[0].steps[0].0, id);

            let mut replay = fixture(depth);
            replay.actions.list = imported.actions;
            replay.execute("actions.play", json!({"action": action.name})).unwrap();
            if pixels(&replay) != expected {
                failures.push(format!("{id}, depth {depth}, {params}"));
            }
        }
    }
    assert!(failures.is_empty(), "ATN playback changed the result:\n{}", failures.join("\n"));
}

fn scalar_cases() -> Vec<Value> {
    RANGES
        .iter()
        .flat_map(|range| {
            ["relative", "absolute"].map(|method| json!({"method": method, "colors": range, "cyan": 35, "magenta": -12.5, "yellow": 17.75, "black": 9}))
        })
        .collect()
}

#[test]
fn scalar_selective_color_actions_preserve_pixels_at_every_depth() {
    let mut cases = scalar_cases();
    cases.push(json!({"method": "absolute", "colors": "reds", "cyan": 35}));
    check_cases(COMMANDS[0], &cases, true);
}

#[test]
fn scalar_selective_color_adjustment_layers_preserve_pixels_at_every_depth() {
    check_cases(COMMANDS[1], &scalar_cases(), true);
}

#[test]
fn mixed_forms_method_precedence_nulls_and_clamping_keep_their_effect() {
    let cases = [
        json!({"method": "absolute", "reds": [10, 20, 30, 40], "blues": [-10, 5, 0, 4], "colors": "reds", "cyan": 0, "yellow": -17.5}),
        json!({"method": "relative", "relative": false, "colors": "reds", "cyan": 35}),
        json!({"method": "absolute", "relative": true, "colors": "neutrals", "black": 15}),
        json!({"colors": "blues", "magenta": -25}),
        json!({"method": null, "relative": null, "reds": null, "colors": "reds", "cyan": 35, "black": null}),
        json!({"method": "absolute", "colors": "reds", "cyan": 175, "magenta": -150}),
        json!({"method": "absolute", "reds": [175, -150, 0, 0]}),
        json!({"method": "absolute", "reds": [10, 20, 30, 40], "colors": "reds", "cyan": null}),
        json!({}),
        json!({"colors": "reds"}),
    ];
    for id in COMMANDS {
        check_cases(id, &cases, false);
    }
}

#[test]
fn per_range_array_controls_still_preserve_pixels() {
    let cases: Vec<_> = RANGES.iter().map(|range| json!({"method": "absolute", *range: [35, -12.5, 17.75, 9]})).collect();
    for id in COMMANDS {
        check_cases(id, &cases, true);
    }
}

#[test]
fn malformed_selective_color_params_cannot_be_silently_exported() {
    for params in [
        json!({"colors": "purples", "cyan": 35}),
        json!({"colors": 3}),
        json!({"colors": "reds", "cyan": "35"}),
        json!({"method": "sideways"}),
        json!({"relative": "yes"}),
        json!({"reds": [10, 20]}),
        json!({"reds": [10, "20", 0, 0]}),
    ] {
        for id in COMMANDS {
            let action = Action::new("Invalid", vec![(id.into(), params.clone())]);
            assert!(atn_cmds::export("Regression", &[&action], &[], false).is_err(), "strict export accepted {id} {params}");
            let (_, skipped) = atn_cmds::export("Regression", &[&action], &[], true).unwrap();
            assert_eq!(skipped.len(), 1, "skipUnsupported must report {id} {params}");
        }
    }
}
