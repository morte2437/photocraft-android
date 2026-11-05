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
    output::{warn_all, warn_dedup},
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
    let mut seen_warnings = std::collections::HashSet::<String>::new();
    warn_dedup(err, &warnings, &mut seen_warnings);
    let opts = ExportOptions::from(&a.export);
    // Results are saved as `<out>/<stem>.<ext>`, so an `--out` that is the `--in` folder would
    // replace the originals (#492).
    if !a.in_place && same_dir(in_dir, out_dir) {
        return Err(CliError::OutIsIn(out_dir.to_path_buf()));
    }
    let inputs: Vec<photocraft_engine::file_cmds::BatchInput> = if a.recursive {
        photocraft_engine::file_cmds::list_images_tree(&in_dir.to_string_lossy(), &out_dir.to_string_lossy(), "photocraft-cli batch")?
            .into_iter()
            .filter(|input| is_input(Path::new(&input.path)))
            .collect()
    } else {
        let mut images = Vec::new();
        let entries = std::fs::read_dir(in_dir).map_err(|source| CliError::ReadDir { path: in_dir.to_path_buf(), source })?;
        for entry in entries {
            let path = entry.map_err(|source| CliError::ReadDir { path: in_dir.to_path_buf(), source })?.path();
            if is_input(&path) {
                images.push(photocraft_engine::file_cmds::BatchInput { path: path.to_string_lossy().into_owned(), rel_dir: String::new() });
            }
        }
        // The tree listing already holds the engine's order (each folder's files, then its
        // subfolders), which `{serial}` numbers; only the flat listing needs sorting.
        images.sort_by(|a, b| a.path.cmp(&b.path));
        images
    };
    let (mut ok, mut failed) = (0, 0);
    // `--format .jpg` names outputs `<stem>.jpg`, as `--format jpg` does (#490).
    let format = a.export.format.as_deref().map(|f| f.strip_prefix('.').unwrap_or(f));
    let named = photocraft_engine::file_cmds::plan_batch_names(
        &inputs,
        &out_dir.to_string_lossy(),
        format.unwrap_or("same"),
        &json!({"naming": a.naming.as_deref().unwrap_or(""), "startingSerial": a.start_serial.unwrap_or(1)}),
    )
    .map_err(|error| CliError::Naming(error.to_string()))?;
    let targets: Vec<PathBuf> = match named {
        Some(paths) => paths.into_iter().map(PathBuf::from).collect(),
        None => inputs
            .iter()
            .map(|input| {
                let path = Path::new(&input.path);
                let ext = format.map_or_else(|| photocraft_engine::file_cmds::batch_same_format(&input.path), str::to_owned);
                let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
                out_dir.join(format!("{stem}.{ext}"))
            })
            .collect(),
    };
    let plan: Vec<_> = inputs.iter().zip(&targets).map(|(input, target)| (input.path.clone(), target.to_string_lossy().into_owned())).collect();
    photocraft_engine::file_cmds::validate_flat_outputs(&plan).map_err(|error| CliError::Naming(error.to_string()))?;
    if let Some(log) = &a.error_log {
        let same = |path: &Path| photocraft_engine::file_cmds::same_destination(log, path);
        if inputs.iter().any(|input| same(Path::new(&input.path))) || targets.iter().any(|target| same(target)) {
            return Err(CliError::BatchOption("--error-log must not replace an input or batch output".into()));
        }
    }
    std::fs::create_dir_all(out_dir)?;
    let mut failures = Vec::new();
    let policy = crate::gpu::GpuPolicy::from_args(&a.gpu).map_err(CliError::Other)?;
    let mut renderer = policy.create_renderer();
    for (input, target) in inputs.iter().zip(targets) {
        let input = Path::new(&input.path);
        // The selected format encodes the bytes, whatever extension `--naming` gives the file.
        // Without --format, a file PhotoCraft only opens (.kra, raw, …) is saved as PSD, the
        // same format `--naming`'s {ext} plans.
        let ext = format.map_or_else(|| photocraft_engine::file_cmds::batch_same_format(&input.to_string_lossy()), str::to_owned);
        let mut h = Headless::trusted_local();
        h.set_renderer(renderer.take());
        let r = (|| -> Result<Vec<String>> {
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            // Steps that play another action find it in the set; the session's playback stack
            // stops an action that calls itself.
            h.session.actions.list = set.clone();
            h.open_with_approval(input, a.approve_high_memory)?;
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
        let diags = h.drain_diagnostics();
        // Release the item's GPU caches before the renderer moves on to the next file.
        if h.session.active_index().is_some() {
            let _ = h.close(None);
        }
        if let Some(recovered) = h.take_renderer() {
            renderer = Some(recovered);
        }
        match r {
            Ok(ws) => {
                ok += 1;
                let _ = writeln!(out, "ok    {} -> {}", input.display(), target.display());
                // Each file's own export warnings (e.g. transparency composited over white) are
                // reported for that file, even when an earlier file had the same warning.
                warn_all(err, &ws);
                // A fallback diagnostic can also be in this file's export warnings; report it
                // once, through the per-file warnings above, not twice.
                let pending: Vec<String> = diags.into_iter().filter(|d| !ws.contains(d)).collect();
                warn_dedup(err, &pending, &mut seen_warnings);
            }
            Err(e) => {
                failed += 1;
                let _ = writeln!(err, "FAIL  {}: {e}", input.display());
                warn_dedup(err, &diags, &mut seen_warnings);
                failures.push((input.to_string_lossy().into_owned(), e.to_string()));
                if a.stop_on_error {
                    break;
                }
            }
        }
    }
    let skipped = inputs.len().saturating_sub(ok + failed);
    if skipped > 0 {
        let _ = writeln!(err, "--stop-on-error: {skipped} remaining input(s) skipped");
        let _ = writeln!(out, "{ok} succeeded, {failed} failed, {skipped} skipped");
    } else {
        let _ = writeln!(out, "{ok} succeeded, {failed} failed");
    }
    // After the summary, so a log that can't be written doesn't hide the counts.
    if let Some(path) = &a.error_log {
        let bytes = photocraft_engine::file_cmds::encode_batch_error_log(failures.iter().map(|(file, error)| (file.as_str(), error.as_str())));
        photocraft_format::atomic_write(path, &bytes)?;
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use photocraft_doc::{Color, ColorMode, Document, Rect, SampleType, Size};
    use photocraft_io::{BandRenderer, IoError, RenderBand, RenderError};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct RecordingRenderer {
        calls: Arc<AtomicUsize>,
    }

    impl BandRenderer for RecordingRenderer {
        fn render_bands(
            &mut self,
            doc: &Document,
            rect: Rect,
            band_rows: i32,
            ctl: &photocraft_raster::Interrupt<'_>,
            sink: &mut dyn FnMut(RenderBand) -> Result<(), IoError>,
        ) -> Result<(), RenderError> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            photocraft_io::CpuRenderer.render_bands(doc, rect, band_rows, ctl, sink)
        }
    }

    #[test]
    fn batch_loop_recovers_renderer_after_error_on_first_item() {
        let calls = Arc::new(AtomicUsize::new(0));
        let mut renderer: Option<Box<dyn BandRenderer>> = Some(Box::new(RecordingRenderer { calls: calls.clone() }));

        let tmp = std::env::temp_dir().join(format!("pc-batch-regress-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::fs::create_dir_all(&tmp).unwrap();

        let bad_file = tmp.join("01_bad.png");
        std::fs::write(&bad_file, b"not a valid png or document file").unwrap();

        let good_file = tmp.join("02_good.pcraft");
        let mut good_doc = Document::with_background("test", Size::new(32, 32), ColorMode::Rgb, SampleType::U8, Color::WHITE);
        let extra_layer = good_doc.layers[0].clone();
        good_doc.layers.push(extra_layer);
        photocraft_automation::files::save(&good_doc, &good_file, None, &Default::default(), None).unwrap();

        let target_dir = tmp.join("out");
        std::fs::create_dir_all(&target_dir).unwrap();

        let inputs = [bad_file, good_file];
        let opts = ExportOptions::default();

        for (idx, input) in inputs.iter().enumerate() {
            let mut h = Headless::trusted_local();
            h.set_renderer(renderer.take());
            let r = (|| -> Result<Vec<String>> {
                h.open(input)?;
                let target = target_dir.join(format!("{idx}.png"));
                let r = h.save(None, Some(&target), Some("png"), &opts)?;
                Ok(serde_json::from_value(r["warnings"].clone()).unwrap_or_default())
            })();
            if let Some(recovered) = h.take_renderer() {
                renderer = Some(recovered);
            }
            if idx == 0 {
                assert!(r.is_err(), "first corrupt item must fail");
            } else {
                assert!(r.is_ok(), "second valid item must succeed: {:?}", r);
            }
        }

        assert!(renderer.is_some(), "renderer must be preserved across batch items");
        assert_eq!(calls.load(Ordering::SeqCst), 1, "renderer must have been invoked on the second item");

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
