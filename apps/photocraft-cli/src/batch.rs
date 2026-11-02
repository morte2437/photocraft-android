use std::io::Write;
use std::path::{Path, PathBuf};

use photocraft_automation::Headless;
use photocraft_engine::actions_cmds::Action;
use photocraft_io::ExportOptions;
use serde_json::json;

use crate::{
    CliError, Result,
    actions::load_actions_file,
    args::{BatchArgs, DropletArgs},
    output::warn_all,
};

/// Whether `a` and `b` name the same existing folder, however they are spelt (relative, with a
/// trailing separator, through a symbolic link).
fn same_dir(a: &Path, b: &Path) -> bool {
    matches!((std::fs::canonicalize(a), std::fs::canonicalize(b)), (Ok(a), Ok(b)) if a == b)
}

pub(crate) fn is_input(p: &Path) -> bool {
    let known = |e: &str| {
        matches!(e, "pcraft" | "psd" | "psb" | "pdn" | "ora" | "kra" | "krz" | "af" | "afdesign" | "afphoto" | "afpub")
            || photocraft_codecs::from_extension(e).is_some_and(|f| photocraft_codecs::caps(f).read)
    };
    p.is_file() && p.extension().is_some_and(|e| known(&e.to_string_lossy().to_ascii_lowercase()))
}

/// Drops the Save and view steps the desktop records from `action` (Batch saves each result
/// itself, #3143), keeping its unchecked and dialog flags on the steps that remain (#6860).
fn strip_shell_steps(action: &mut Action) {
    use photocraft_engine::actions_cmds::{shell_save_command, shell_view_command};
    let (mut steps, mut disabled, mut dialogs) = (Vec::new(), std::collections::BTreeSet::new(), std::collections::BTreeSet::new());
    for (i, (id, p)) in std::mem::take(&mut action.steps).into_iter().enumerate() {
        if shell_save_command(&id) || shell_view_command(&id) {
            continue;
        }
        if action.disabled.contains(&i) {
            disabled.insert(steps.len());
        }
        if action.dialogs.contains(&i) {
            dialogs.insert(steps.len());
        }
        steps.push((id, p));
    }
    (action.steps, action.disabled, action.dialogs) = (steps, disabled, dialogs);
}

pub(crate) fn batch(a: &BatchArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result {
    let (in_dir, out_dir) = (a.in_dir.as_path(), a.out.as_path());
    let (actions, mut set, warnings) = load_actions_file(&a.actions, a.action.as_deref())?;
    set.iter_mut().for_each(strip_shell_steps);
    warn_all(err, &warnings);
    let opts = ExportOptions::from(&a.export);
    // Results are saved as `<out>/<stem>.<ext>`, so an `--out` that is the `--in` folder would
    // replace the originals (#492).
    if !a.in_place && same_dir(in_dir, out_dir) {
        return Err(CliError::OutIsIn(out_dir.to_path_buf()));
    }
    std::fs::create_dir_all(out_dir)?;
    let mut inputs: Vec<PathBuf> = std::fs::read_dir(in_dir)
        .map_err(|source| CliError::ReadDir { path: in_dir.to_path_buf(), source })?
        .flatten()
        .map(|e| e.path())
        .filter(|p| is_input(p))
        .collect();
    inputs.sort();
    let (mut ok, mut failed) = (0, 0);
    let mut written = photocraft_engine::file_cmds::OutputClaims::default();
    // `--format .jpg` names outputs `<stem>.jpg`, as `--format jpg` does (#490).
    let format = a.export.format.as_deref().map(|f| f.strip_prefix('.').unwrap_or(f));
    for input in &inputs {
        let ext = format.map(str::to_owned).or_else(|| input.extension().map(|e| e.to_string_lossy().into_owned())).unwrap_or_else(|| "png".into());
        let stem = input.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let target = out_dir.join(format!("{stem}.{ext}"));
        let target_text = target.to_string_lossy();
        let r = (|| -> Result<Vec<String>> {
            written.check(&target_text).map_err(CliError::OutputClaimed)?;
            let mut h = Headless::trusted_local();
            // Steps that play another action find it in the set; the session's playback stack
            // stops an action that calls itself.
            h.session.actions.list = set.clone();
            h.open(input)?;
            for (id, p) in &actions {
                if photocraft_engine::actions_cmds::shell_view_command(id)
                    || photocraft_engine::actions_cmds::shell_save_command(id)
                    || photocraft_engine::actions_cmds::noop_step(&h.session, id, p)
                {
                    continue;
                }
                let r = h.command_run(id, p.clone()).map_err(|source| CliError::Command { id: id.clone(), source })?;
                // A failed step inside a called action makes this file an error, not a saved result.
                if let Some(message) = photocraft_engine::actions_cmds::nested_failure(id, &r) {
                    return Err(CliError::NestedFailure { id: id.clone(), message });
                }
            }
            let r = h.save(None, Some(&target), Some(&ext), &opts)?;
            Ok(serde_json::from_value(r["warnings"].clone()).unwrap_or_default())
        })();
        match r {
            Ok(ws) => {
                ok += 1;
                written.record(&target_text, &input.to_string_lossy());
                let _ = writeln!(out, "ok    {} -> {}", input.display(), target.display());
                warn_all(err, &ws);
            }
            Err(e) => {
                failed += 1;
                let _ = writeln!(err, "FAIL  {}: {e}", input.display());
            }
        }
    }
    let _ = writeln!(out, "{ok} succeeded, {failed} failed");
    if failed > 0 { Err(CliError::FilesFailed(failed)) } else { Ok(()) }
}

pub(crate) fn droplet(a: &DropletArgs, out: &mut dyn Write, err: &mut dyn Write) -> Result {
    let (file, inputs) = (&a.file, &a.inputs);
    let mut p = json!({"droplet": file, "input": inputs});
    if let Some(o) = &a.out {
        p["output"] = json!(o);
    }
    let mut s = photocraft_engine::Session::new();
    let r = s.execute("file.automate.runDroplet", p)?;
    for f in r["files"].as_array().into_iter().flatten() {
        let _ = writeln!(out, "ok    {}", f.as_str().unwrap_or_default());
    }
    let errors = r["errors"].as_array().cloned().unwrap_or_default();
    for e in &errors {
        let _ = writeln!(err, "FAIL  {}: {}", e["file"].as_str().unwrap_or_default(), e["error"].as_str().unwrap_or_default());
    }
    if errors.is_empty() { Ok(()) } else { Err(CliError::FilesFailed(errors.len())) }
}

#[cfg(test)]
mod batch_action_set_tests {
    use super::*;
    use crate::test_support::{Invoke, invoke};
    use serde_json::Value;

    /// A batched action that plays another action of its set finds it in every file's session; a
    /// failure inside the called action fails the file, and a self-call stops instead of looping.
    #[test]
    fn batch_plays_an_action_that_calls_another_action_of_its_set() {
        let dir = std::env::temp_dir().join(format!("photocraft-cli-action-set-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = |name: &str| dir.join(name).to_string_lossy().to_string();
        std::fs::create_dir_all(dir.join("in")).unwrap();
        for name in ["a", "b"] {
            let Invoke { code, stderr, .. } = invoke(&["run", "--new", r#"{"width":8,"height":6}"#, "--out", &path(&format!("in/{name}.png"))]);
            assert_eq!(code, 0, "{stderr}");
        }
        let set = json!({"version": 1, "actions": [
            {"name": "Outer", "steps": [["actions.play", {"action": "Rotate"}]]},
            {"name": "Rotate", "steps": [["image.imageRotation.90cw", {}]]},
            {"name": "Calls bad", "steps": [["actions.play", {"action": "Bad"}]]},
            {"name": "Bad", "steps": [["layer.delete", {"layer": 999_999}]]},
            {"name": "Loop", "steps": [["actions.play", {"action": "Loop"}]]},
        ]});
        std::fs::write(dir.join("actions.json"), set.to_string()).unwrap();
        let batch = |action: &str| invoke(&["batch", "--actions", &path("actions.json"), "--action", action, "--in", &path("in"), "--out", &path(action)]);

        let Invoke { code, stdout, stderr } = batch("Outer");
        assert_eq!(code, 0, "{stdout}{stderr}");
        for name in ["a", "b"] {
            let Invoke { code, stdout: info, stderr } = invoke(&["info", &path(&format!("Outer/{name}.png")), "--compact"]);
            assert_eq!(code, 0, "{stderr}");
            let info: Value = serde_json::from_str(&info).unwrap();
            assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(6), Some(8)), "rotated by the called action");
        }
        for action in ["Calls bad", "Loop"] {
            let Invoke { code, stdout, stderr } = batch(action);
            assert_ne!(code, 0, "{stdout}");
            assert_eq!(stderr.matches("FAIL").count(), 2, "{stderr}");
            assert!(!dir.join(action).join("a.png").exists(), "a file whose called action failed is not saved");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Save and view steps recorded inside a batched action set are skipped, also in a called
    /// action, and unchecked steps stay unchecked once they are gone (#6860).
    #[test]
    fn batch_skips_save_and_view_steps_inside_an_action_set() {
        let dir = std::env::temp_dir().join(format!("photocraft-cli-action-set-save-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = |name: &str| dir.join(name).to_string_lossy().to_string();
        std::fs::create_dir_all(dir.join("in")).unwrap();
        let Invoke { code, stderr, .. } = invoke(&["run", "--new", r#"{"width":8,"height":6}"#, "--out", &path("in/a.png")]);
        assert_eq!(code, 0, "{stderr}");
        let set = json!({"version": 1, "actions": [
            {"name": "Outer", "steps": [["actions.play", {"action": "Rotate"}], ["file.saveAs", {}]]},
            {"name": "Rotate", "steps": [
                ["file.save", {}],
                ["image.imageRotation.90cw", {}],
                ["view.fitOnScreen", {}],
                ["image.imageRotation.90cw", {}],
                ["file.saveAs", {}],
            ], "disabled": [3], "dialogs": [1]},
        ]});
        std::fs::write(dir.join("actions.json"), set.to_string()).unwrap();
        for action in ["Outer", "Rotate"] {
            let Invoke { code, stdout, stderr } =
                invoke(&["batch", "--actions", &path("actions.json"), "--action", action, "--in", &path("in"), "--out", &path(action)]);
            assert_eq!(code, 0, "{stdout}{stderr}");
            let Invoke { code, stdout: info, stderr } = invoke(&["info", &path(&format!("{action}/a.png")), "--compact"]);
            assert_eq!(code, 0, "{stderr}");
            let info: Value = serde_json::from_str(&info).unwrap();
            assert_eq!((info["width"].as_u64(), info["height"].as_u64()), (Some(6), Some(8)), "{action}: rotated once, the unchecked step skipped");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
