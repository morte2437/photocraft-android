use std::io::Write;
use std::path::Path;

use photocraft_automation::{Headless, files};
use serde_json::{Value, json};

use photocraft_engine::type_extra_cmds::missing_font_warnings;
use photocraft_io::ExportOptions;

use crate::{
    CliError, Result,
    args::{ConvertArgs, InfoArgs, RunArgs, Step},
    output::{JsonStyle, print_json, warn_all},
};

/// An output that is the input file itself, however it is spelt (relative, with `./`, through a
/// hard or symbolic link), would replace the original, and a lossy re-encode can't be undone: it
/// is refused unless `--in-place` asks for it, as `batch` refuses an `--out` that is its `--in`
/// folder (#7437). The paths are compared when the command starts and the save opens `output`
/// again later, so another process swapping in a link meanwhile goes unnoticed: this guards
/// against mistakes, it is not a security boundary.
fn refuse_overwriting_input(in_place: bool, input: &Path, output: &Path) -> Result {
    if !in_place && photocraft_engine::file_cmds::same_file(input, output) {
        return Err(CliError::OutIsInput(output.to_path_buf()));
    }
    Ok(())
}

pub(crate) fn convert(a: &ConvertArgs, _out: &mut dyn Write, err: &mut dyn Write) -> Result {
    let (input, output) = (&a.input, &a.output);
    let opts = ExportOptions::from(&a.export);
    refuse_overwriting_input(a.in_place, input, output)?;
    let mut o = files::open(input)?;
    // Preferences › Raw Defaults › Resolution at its default, as `run` and `info` open a raw.
    photocraft_engine::file_cmds::raw_defaults(&photocraft_engine::Session::new(), &o.warnings, &mut o.document);
    warn_all(err, &o.warnings);
    warn_all(err, &missing_font_warnings(&o.document));
    let ws = files::save(&o.document, output, a.export.format.as_deref(), &opts, None)?;
    warn_all(err, &ws);
    Ok(())
}

pub(crate) fn info(a: &InfoArgs, out: &mut dyn Write) -> Result {
    let file = &a.file;
    let mut h = Headless::trusted_local();
    let opened = h.open(file)?;
    let mut doc = h.inspect(None)?;
    let mut warnings = opened["warnings"].as_array().cloned().unwrap_or_default();
    if let Some(st) = h.session.active() {
        warnings.extend(missing_font_warnings(&st.doc).into_iter().map(Value::String));
    }
    doc["warnings"] = Value::Array(warnings);
    doc["file"] = json!(file.to_string_lossy());
    // Drop per-session noise.
    if let Value::Object(m) = &mut doc {
        for k in ["history", "canUndo", "canRedo", "revision"] {
            m.remove(k);
        }
    }
    print_json(out, &doc, if a.compact { JsonStyle::Compact } else { JsonStyle::Pretty })
}

/// `(id, params)` pairs from repeated `--cmd`/`--params` flags.
fn command_list(a: &RunArgs) -> Result<Vec<(String, Value)>> {
    let mut cmds: Vec<(String, Value)> = Vec::new();
    for step in &a.steps {
        match step {
            Step::Cmd(id) => cmds.push((id.clone(), json!({}))),
            Step::Params(p) => cmds.last_mut().ok_or(CliError::ParamsBeforeCmd)?.1 = p.clone(),
        }
    }
    Ok(cmds)
}

pub(crate) fn run_cmds(a: &RunArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result {
    let opts = ExportOptions::from(&a.export);
    let mut h = Headless::trusted_local();
    // Opening, running commands and exporting can all notice the same missing font:
    // one `run` invocation reports each warning line at most once.
    let mut reported_fonts = std::collections::HashSet::<String>::new();
    // clap's `source` group gives exactly one of the two, and its `work` group a --cmd or --out.
    if let Some(new) = &a.new {
        h.command_run("file.new", new.clone()).map_err(CliError::New)?;
    } else if let Some(file) = &a.file {
        // Refused before the file is read or any command runs.
        if let Some(out) = &a.out {
            refuse_overwriting_input(a.in_place, file, out)?;
        }
        let o = h.open(file)?;
        let ws: Vec<String> = serde_json::from_value(o["warnings"].clone()).unwrap_or_default();
        warn_all(err, &ws);
        reported_fonts.extend(ws);
    }
    let cmds = command_list(a)?;
    for (id, params) in cmds {
        // A step that is a silent no-op in action playback (Deselect with nothing selected, an Arrange that cannot move the layer) is one in a script too (#8802).
        if photocraft_engine::actions_cmds::noop_step(&h.session, &id, &params) {
            print_json(out, &json!({"command": id, "result": null}), JsonStyle::Compact)?;
            continue;
        }
        let r = h.command_run(&id, params).map_err(|source| CliError::Command { id: id.clone(), source })?;
        if let Some(message) = photocraft_engine::actions_cmds::nested_failure(&id, &r) {
            return Err(CliError::NestedFailure { id, message });
        }
        let new_warnings: Vec<String> =
            h.session.active().map(|st| missing_font_warnings(&st.doc)).unwrap_or_default().into_iter().filter(|w| reported_fonts.insert(w.clone())).collect();
        warn_all(err, &new_warnings);
        let mut result = json!({"command": id, "result": r});
        if !new_warnings.is_empty() {
            result["warnings"] = json!(new_warnings);
        }
        print_json(out, &result, JsonStyle::Compact)?;
    }
    // A `run` with only `--out` still reports missing fonts before exporting.
    let remaining: Vec<String> =
        h.session.active().map(|st| missing_font_warnings(&st.doc)).unwrap_or_default().into_iter().filter(|w| reported_fonts.insert(w.clone())).collect();
    warn_all(err, &remaining);
    if let Some(o) = a.out.as_deref() {
        let r = h.save(None, Some(o), a.export.format.as_deref(), &opts)?;
        let ws: Vec<String> = serde_json::from_value(r["warnings"].clone()).unwrap_or_default();
        let ws: Vec<String> = ws.into_iter().filter(|w| reported_fonts.insert(w.clone())).collect();
        warn_all(err, &ws);
    }
    Ok(())
}

#[cfg(test)]
mod missing_font_warning_tests {
    use super::*;
    use crate::{
        batch::is_input,
        test_support::{Invoke, invoke},
    };

    #[test]
    fn missing_typeface_is_reported_by_run_info_and_convert() {
        let family = "DefinitelyMissingPhotoCraftTypeface1251";
        let folder = std::env::temp_dir().join(format!("photocraft-cli-font-warnings-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let native = folder.join("missing.pcraft");
        let png = folder.join("missing.png");
        let native_str = native.to_string_lossy().to_string();
        let png_str = png.to_string_lossy().to_string();
        let params = json!({"x": 6, "y": 38, "text": "Missing font warning", "font": family, "size": 20}).to_string();

        let Invoke { code, stdout, stderr } = invoke(&[
            "run",
            "--new",
            r#"{"width":256,"height":96,"background":"white"}"#,
            "--cmd",
            "type.create",
            "--params",
            &params,
            "--cmd",
            "type.resolveMissingFonts",
            "--out",
            &native_str,
        ]);
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(stderr.matches(family).count(), 1, "multi-command runs should not repeat the same missing font warning");
        let commands: Vec<Value> = stdout.lines().map(|s| serde_json::from_str(s).unwrap()).collect();
        assert_eq!(commands.len(), 2, "{stdout}");
        assert!(commands[0]["warnings"].as_array().unwrap().iter().any(|w| w.as_str().is_some_and(|s| s.contains(family))));
        assert!(commands[1].get("warnings").is_none(), "only newly detected missing fonts are reported");

        let Invoke { code, stdout, stderr } = invoke(&["info", &native_str, "--compact"]);
        assert_eq!(code, 0, "{stderr}");
        let info: Value = serde_json::from_str(&stdout).unwrap();
        assert!(info["warnings"].as_array().unwrap().iter().any(|w| w.as_str().is_some_and(|s| s.contains(family))));
        assert!(stderr.is_empty(), "info puts warnings in JSON, not stderr");

        let Invoke { code, stderr, .. } = invoke(&["convert", &native_str, &png_str]);
        assert_eq!(code, 0, "{stderr}");
        assert!(stderr.contains(&format!("warning: font '{family}'")), "{stderr}");
        assert!(png.is_file());

        // Even with no commands, run --out checks the opened document before export.
        let Invoke { code, stdout, stderr } = invoke(&["run", &native_str, "--out", &png_str]);
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.is_empty());
        assert_eq!(stderr.matches(family).count(), 1);

        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn convert_reads_and_writes_openraster() {
        let folder = std::env::temp_dir().join(format!("photocraft-cli-ora-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let path = |name: &str| folder.join(name).to_string_lossy().to_string();
        let (native, ora, png) = (path("layers.pcraft"), path("layers.ora"), path("back.png"));
        let Invoke { code, stderr, .. } = invoke(&["run", "--new", r#"{"width":16,"height":12}"#, "--cmd", "layer.new.layer", "--out", &native]);
        assert_eq!(code, 0, "{stderr}");
        let Invoke { code, stderr, .. } = invoke(&["convert", &native, &ora]);
        assert_eq!(code, 0, "{stderr}");
        let Invoke { code, stdout, stderr } = invoke(&["info", &ora, "--compact"]);
        assert_eq!(code, 0, "{stderr}");
        let info: Value = serde_json::from_str(&stdout).unwrap();
        assert_eq!(info["layers"].as_array().map(Vec::len), Some(2), "{info}");
        let Invoke { code, stderr, .. } = invoke(&["convert", &ora, &png]);
        assert_eq!(code, 0, "{stderr}");
        assert!(is_input(std::path::Path::new(&ora)));
        // Krita documents are batch inputs too (import-only).
        for name in ["sketch.kra", "old.KRZ"] {
            std::fs::write(folder.join(name), b"").unwrap();
            assert!(is_input(&folder.join(name)), "{name}");
        }
        let _ = std::fs::remove_dir_all(&folder);
    }

    #[test]
    fn no_type_layers_do_not_produce_missing_font_warnings() {
        let folder = std::env::temp_dir().join(format!("photocraft-cli-no-font-warning-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let native = folder.join("blank.pcraft");
        let png = folder.join("blank.png");
        let native_str = native.to_string_lossy().to_string();
        let png_str = png.to_string_lossy().to_string();
        let Invoke { code, stderr, .. } = invoke(&["run", "--new", r#"{"width":16,"height":16}"#, "--out", &native_str]);
        assert_eq!(code, 0, "{stderr}");
        assert!(!stderr.contains("font '"), "{stderr}");
        let Invoke { code, stdout, stderr } = invoke(&["info", &native_str, "--compact"]);
        assert_eq!(code, 0, "{stderr}");
        let info: Value = serde_json::from_str(&stdout).unwrap();
        assert!(info["warnings"].as_array().unwrap().iter().all(|w| !w.as_str().unwrap_or_default().contains("font '")));
        let Invoke { code, stderr, .. } = invoke(&["convert", &native_str, &png_str]);
        assert_eq!(code, 0, "{stderr}");
        assert!(!stderr.contains("font '"), "{stderr}");
        let _ = std::fs::remove_dir_all(&folder);
    }
}
