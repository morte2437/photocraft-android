use super::open::import_at_raw_ppi;
use super::paths::{extension, file_name, join, list_images, read_file, stem};
use super::processing::parse_steps;
use super::save::{SaveOpts, save_doc};
use super::{Arc, Document, EngineError, Result, Session, Value, batch_naming, json, str_param};

/// Inputs of a batch command: `"input"` is a folder or an array of files.
pub(crate) fn batch_inputs(p: &Value, cmd: &str) -> Result<Vec<String>> {
    match p.get("input").or_else(|| p.get("files")) {
        Some(Value::Array(a)) => Ok(a.iter().filter_map(Value::as_str).map(str::to_string).collect()),
        Some(Value::String(dir)) => list_images(dir),
        _ => Err(EngineError::BadParams { cmd: cmd.into(), msg: "missing \"input\" (folder or array of files)".into() }),
    }
}

/// One input of a batch run: the file, and the folder it sits in relative to the input folder
/// (`""` at the top level, `"a/b"` two levels down), which its result keeps under `output`.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct BatchInput {
    pub path: String,
    pub rel_dir: String,
}

/// Subfolders below a batch input folder are walked at most this deep ("Include All Subfolders").
pub(crate) const MAX_SUBFOLDER_DEPTH: usize = 32;

/// A batch over subfolders lists at most this many files; a larger tree is an error, not a run
/// that silently stops partway.
pub(crate) const MAX_SUBFOLDER_FILES: usize = 100_000;

/// [`batch_inputs`] for the commands whose Photoshop dialog has "Include All Subfolders" (Batch,
/// Image Processor): with `"includeSubfolders": true` a folder input also yields the images in its
/// subfolders, each tagged with its relative folder. `output` (when inside the input folder, as
/// the dialog's default `<input>/batch` is) is not walked, so a rerun doesn't process its results.
pub(crate) fn batch_inputs_tree(p: &Value, cmd: &str, output: &str) -> Result<Vec<BatchInput>> {
    let recurse = match p.get("includeSubfolders") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(_) => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "\"includeSubfolders\" must be true or false".into() }),
    };
    match p.get("input").or_else(|| p.get("files")) {
        Some(Value::String(dir)) if recurse => list_images_tree(dir, output, cmd),
        _ => Ok(batch_inputs(p, cmd)?.into_iter().map(|path| BatchInput { path, rel_dir: String::new() }).collect()),
    }
}

/// The images under `dir` and its subfolders: each folder's files sorted by name, then its
/// subfolders in name order (depth first). Bounded by [`MAX_SUBFOLDER_DEPTH`] and
/// [`MAX_SUBFOLDER_FILES`]; a folder reached twice (a symlink loop) is walked once, and `skip`
/// (the output folder) is left out.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn list_images_tree(dir: &str, skip: &str, cmd: &str) -> Result<Vec<BatchInput>> {
    use std::path::{Path, PathBuf};
    let canon = |p: &Path| std::fs::canonicalize(p).ok();
    let root = Path::new(dir);
    let root_canon = canon(root).ok_or_else(|| EngineError::Other(format!("{dir}: cannot open the folder")))?;
    let skip = if skip.is_empty() { None } else { canon(Path::new(skip)) };
    let mut seen = std::collections::HashSet::new();
    seen.insert(root_canon);
    let mut out = Vec::new();
    // Depth first with an explicit stack (no recursion): (folder, relative folder, depth).
    let mut stack: Vec<(PathBuf, String, usize)> = vec![(root.to_path_buf(), String::new(), 0)];
    while let Some((folder, rel, depth)) = stack.pop() {
        let shown = folder.to_string_lossy().into_owned();
        // The top folder must be readable; a subfolder that isn't is skipped.
        let files = match list_images(&shown) {
            Ok(f) => f,
            Err(e) if depth == 0 => return Err(e),
            Err(_) => continue,
        };
        if out.len().saturating_add(files.len()) > MAX_SUBFOLDER_FILES {
            return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("{dir} holds more than {MAX_SUBFOLDER_FILES} images with its subfolders") });
        }
        out.extend(files.into_iter().map(|path| BatchInput { path, rel_dir: rel.clone() }));
        if depth >= MAX_SUBFOLDER_DEPTH {
            continue;
        }
        let Ok(rd) = std::fs::read_dir(&folder) else { continue };
        let mut subs: Vec<(String, PathBuf)> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .filter_map(|p| Some((p.file_name()?.to_string_lossy().into_owned(), p)))
            .collect();
        subs.sort();
        // Pushed in reverse so the stack pops them in name order.
        for (name, sub) in subs.into_iter().rev() {
            let Some(c) = canon(&sub) else { continue };
            if skip.as_ref() == Some(&c) || !seen.insert(c) {
                continue;
            }
            let rel = if rel.is_empty() { name } else { format!("{rel}/{name}") };
            stack.push((sub, rel, depth + 1));
        }
    }
    Ok(out)
}

#[cfg(target_arch = "wasm32")]
pub(crate) fn list_images_tree(dir: &str, _skip: &str, _cmd: &str) -> Result<Vec<BatchInput>> {
    Err(EngineError::Other(format!("cannot list {dir}: no file system on the web")))
}

/// Opens each input in a scratch session, runs `f` on it and saves it to `output` as `format`
/// (`"same"` keeps the input's extension). Errors per file are collected, not fatal.
pub(crate) fn process_files(
    s: &Session,
    inputs: &[String],
    output: &str,
    format: &str,
    save: SaveOpts,
    suffix: &str,
    f: &dyn Fn(&mut Session) -> Result<()>,
) -> Value {
    let inputs: Vec<BatchInput> = inputs.iter().map(|path| BatchInput { path: path.clone(), rel_dir: String::new() }).collect();
    process_tree(s, &inputs, output, format, save, suffix, &|scratch, _| f(scratch).map(|()| false))
}

/// [`process_files`] for inputs from subfolders: each result goes to its input's relative folder
/// under `output` (created as needed), so `a/x.png` and `b/x.png` don't collide. `f` gets the
/// output path and returns `true` when it wrote the result there itself (Batch writes what a
/// recorded save marked); otherwise the active document is saved after it.
pub(crate) fn process_tree(
    s: &Session,
    inputs: &[BatchInput],
    output: &str,
    format: &str,
    save: SaveOpts,
    suffix: &str,
    f: &dyn Fn(&mut Session, &str) -> Result<bool>,
) -> Value {
    let outputs = inputs.iter().map(|BatchInput { path, rel_dir }| {
        let ext = if format == "same" { same_format(path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
        (path.as_str(), join(&join(output, rel_dir), &format!("{}{suffix}.{ext}", stem(path))))
    });
    process_outputs(s, outputs, save, f)
}

/// The extension a batch output takes with Format "Same": the input's own, except for formats
/// PhotoCraft opens but can't write, camera raws above all (#5799: a folder of ARWs failed on
/// every file), GIMP files (`gz` is the last extension of `.xcf.gz`), Photoshop templates and the
/// codec formats without a writer in this build. Those are saved as PSD, the format Photoshop's
/// Save As offers for a raw. An input whose file name has no extension (a dot in a folder name
/// doesn't count) is saved as PNG.
pub(super) fn same_format(path: &str) -> String {
    const IMPORT_ONLY: &[&str] =
        &["pdn", "kra", "krz", "af", "afdesign", "afphoto", "afpub", "heic", "heif", "hif", "svg", "svgz", "cin", "pcx", "xpm", "xcf", "xcfgz", "gz", "psdt"];
    let ext = extension(path).unwrap_or_else(|| "png".into());
    let no_writer = photocraft_codecs::from_extension(&ext).is_some_and(|f| !f.caps().write);
    if photocraft_io::raw::EXTENSIONS.contains(&ext.as_str()) || IMPORT_ONLY.contains(&ext.as_str()) || no_writer { "psd".into() } else { ext }
}

#[cfg(test)]
#[test]
fn same_format_takes_the_file_names_extension() {
    assert_eq!(same_format("/in/a.PNG"), "png");
    assert_eq!(same_format("/in/IMG_0001.ARW"), "psd");
    assert_eq!(same_format("/in/a.kra"), "psd");
    assert_eq!(same_format("/in/IMG_0001.HEIC"), "psd");
    assert_eq!(same_format("/in/logo.svg"), "psd");
    assert_eq!(same_format("/in/scan.cin"), "psd");
    // Formats PhotoCraft opens and does not write are saved as PSD.
    for name in ["/in/scan.dcm", "/in/scan.DICOM", "/in/scan.dc3", "/in/page.xcf", "/in/page.xcf.gz", "/in/page.xcfgz", "/in/card.psdt"] {
        assert_eq!(same_format(name), "psd", "{name}");
    }
    for f in photocraft_codecs::Format::ALL.into_iter().filter(|f| f.caps().read && !f.caps().write) {
        for ext in f.extensions() {
            assert_eq!(same_format(&format!("/in/a.{ext}")), "psd", "{f:?} .{ext}");
        }
    }
    assert_eq!(same_format("/in/a.pam"), "pam");
    assert_eq!(same_format("/in/a.dds"), "dds");
    // No extension in the file name: PNG, not the path after a dot in a folder name.
    assert_eq!(same_format("/in/v1.2/scan"), "png");
    assert_eq!(same_format("C:\\in.d\\scan"), "png");
    assert_eq!(same_format("/in/.hidden"), "png");
}

fn process_outputs<'a>(
    s: &Session,
    outputs: impl IntoIterator<Item = (&'a str, String)>,
    save: SaveOpts,
    f: &dyn Fn(&mut Session, &str) -> Result<bool>,
) -> Value {
    // The scratch sessions have default preferences; a raw opens at the user's Raw Defaults.
    let raw_ppi = s.prefs().raw_defaults.resolution;
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut written = OutputClaims::default();
    for (path, out) in outputs {
        let r = (|| -> Result<()> {
            written.check(&out).map_err(EngineError::Other)?;
            let bytes = read_file(path)?;
            let mut scratch = Session::new();
            let doc = import_at_raw_ppi(raw_ppi, &file_name(path), &bytes)?;
            scratch.add_document(doc, Some(path.to_string()));
            if !f(&mut scratch, &out)? {
                let d = scratch.active().ok_or(EngineError::NoDocument)?;
                save_doc(&d.doc, &out, save)?;
            }
            Ok(())
        })();
        match r {
            Ok(()) => {
                written.record(&out, path);
                files.push(out);
            }
            Err(e) => errors.push(json!({"file": path, "error": e.to_string()})),
        }
    }
    json!({"files": files, "errors": errors})
}

/// The output paths a batch run has written, so a later input whose output name matches an
/// earlier one's (`a.png` and `a.jpg` saved as JPEG, or the same name in two input folders) is
/// reported instead of silently replacing that result (#420, #422). Names are compared ignoring
/// case, because macOS and Windows file systems do.
#[derive(Default)]
pub struct OutputClaims(pub(super) std::collections::HashMap<String, String>);

impl OutputClaims {
    /// `Err` (naming the earlier input) when `out` was already written in this run.
    pub fn check(&self, out: &str) -> std::result::Result<(), String> {
        match self.0.get(&out.to_lowercase()) {
            Some(first) => Err(format!("not written: {out} already holds the result of {first} from this run (same output name)")),
            None => Ok(()),
        }
    }
    /// Remember that `input`'s result was written to `out`.
    pub fn record(&mut self, out: &str, input: &str) {
        self.0.insert(out.to_lowercase(), input.to_string());
    }
}

/// A recorded Save, Save As or Save a Copy, which marks the result Batch writes to its destination
/// (Photoshop's "Override Action 'Save As' Commands").
pub(crate) fn batch_save_point(id: &str) -> bool {
    matches!(id, "file.save" | "file.saveAs" | "file.saveACopy")
}

/// Whether Batch and droplets keep a recorded step: save points mark the result, while other
/// shell-only saves and view steps (zoom, fit) don't change the output (#2752).
pub(crate) fn batch_keeps_step(id: &str) -> bool {
    (batch_save_point(id) || !crate::actions_cmds::shell_save_command(id)) && !crate::actions_cmds::shell_view_command(id)
}

pub(super) fn batch(s: &mut Session, p: &Value) -> Result<Value> {
    let cmd = "file.automate.batch";
    let mut steps =
        parse_steps(p.get("steps").or_else(|| p.get("action")).ok_or_else(|| EngineError::BadParams { cmd: cmd.into(), msg: "missing \"steps\"".into() })?)?;
    // Each result goes to the destination folder, which overrides the action's recorded Save /
    // Save As steps: they only mark the result.
    steps.retain(|(id, _)| batch_keeps_step(id) && id != "file.export.quickExport");
    if let Some((id, _)) = steps.iter().find(|(id, _)| !batch_save_point(id) && crate::commands::find(id).is_none()) {
        return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("unknown command `{id}` in the action") });
    }
    let output = str_param(p, "output", cmd)?.to_string();
    let inputs = batch_inputs_tree(p, cmd, &output)?;
    let format = p.get("format").and_then(Value::as_str).unwrap_or("same").to_string();
    // Steps that call another action play it from the caller's actions; the playback stack comes
    // along so an action that batches itself is caught as a recursive call.
    let list = match p.get("calledActions") {
        None | Some(Value::Null) => s.actions.list.clone(),
        // A droplet brings the actions it calls (#2794) and runs with those alone.
        Some(v) => {
            let list: Vec<crate::actions_cmds::Action> =
                serde_json::from_value(v.clone()).map_err(|e| EngineError::BadParams { cmd: cmd.into(), msg: format!("\"calledActions\": {e}") })?;
            if list.len() > 4096 {
                return Err(EngineError::BadParams { cmd: cmd.into(), msg: "\"calledActions\" holds more than 4096 actions".into() });
            }
            list
        }
    };
    // A batch inside a played action charges its inputs × steps (at least one each: an input is read,
    // decoded and saved even with no steps) to the playback tree it is in and
    // hands the count on, so a batch of batches can't multiply past the limit. A top-level batch
    // is not limited by it.
    let mut playback_steps = 0;
    if s.actions.playing > 0 {
        playback_steps = s.actions.playback_steps.saturating_add((steps.len().max(1) as u64).saturating_mul(inputs.len() as u64));
        if playback_steps > crate::actions_cmds::MAX_PLAYBACK_STEPS {
            return Err(EngineError::BadParams { cmd: cmd.into(), msg: "this batch would exceed the playback step limit of the action that runs it".into() });
        }
        s.actions.playback_steps = playback_steps;
    }
    let actions = crate::actions_cmds::ActionState {
        list,
        playing: s.actions.playing,
        playback_stack: s.actions.playback_stack.clone(),
        playback_steps,
        ..Default::default()
    };
    let names = batch_naming::plan(&inputs, &output, &format, p)?;
    let save = SaveOpts::from_params(s, p, cmd)?;
    let authorize = s.authorize;
    // Each input runs in its own scratch session, so what the played steps spend is carried from
    // one input to the next (failed or not) and back to this session, not restarted per input.
    let carried = std::cell::Cell::new(playback_steps);
    let carried = &carried;
    let run = move |scratch: &mut Session, out: &str| {
        scratch.authorize = authorize;
        scratch.actions = actions.clone();
        scratch.actions.playback_steps = carried.get();
        let result = (|| {
            // The document as the last recorded save found it, written once the action has run so a
            // failing step still writes nothing.
            let mut saved: Option<Arc<Document>> = None;
            for (id, params) in &steps {
                if batch_save_point(id) {
                    saved = Some(scratch.active().ok_or(EngineError::NoDocument)?.doc.clone());
                    continue;
                }
                let r = scratch.execute(id, params.clone())?;
                // A failed step inside a called action makes this file an error, not a saved result.
                if let Some(error) = crate::actions_cmds::nested_failure(id, &r) {
                    return Err(EngineError::Other(error));
                }
            }
            // While the saved document is still open the steps after the save reach the result too
            // (#3058), so it is saved after the action as usual. Closed after its save, or with another
            // document left active, it is written as the save found it.
            match saved {
                Some(doc) if scratch.active().is_none_or(|d| d.doc.id != doc.id) => {
                    save_doc(&doc, out, save)?;
                    Ok(true)
                }
                _ => Ok(false),
            }
        })();
        carried.set(scratch.actions.playback_steps);
        result
    };
    let r = match names {
        Some(names) => process_outputs(s, inputs.iter().map(|i| i.path.as_str()).zip(names), save, &run),
        None => process_tree(s, &inputs, &output, &format, save, "", &run),
    };
    if s.actions.playing > 0 {
        s.actions.playback_steps = carried.get();
    }
    Ok(r)
}
