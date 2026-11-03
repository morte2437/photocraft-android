use super::open::import_at_raw_ppi;
use super::paths::{extension, file_name, join, list_images, read_file, stem, write_file};
use super::processing::parse_steps;
use super::save::{SaveOpts, encode, save_doc, save_doc_as};
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
pub struct BatchInput {
    pub path: String,
    pub rel_dir: String,
}

/// Subfolders below a batch input folder are walked at most this deep ("Include All Subfolders").
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const MAX_SUBFOLDER_DEPTH: usize = 32;

/// A batch over subfolders lists at most this many files; a larger tree is an error, not a run
/// that silently stops partway.
#[cfg(not(target_arch = "wasm32"))]
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
/// (the output folder) is left out. An unreadable reached folder is an error.
#[cfg(not(target_arch = "wasm32"))]
pub fn list_images_tree(dir: &str, skip: &str, cmd: &str) -> Result<Vec<BatchInput>> {
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
        // Fail before writing any output if a reached input folder cannot be read.
        let files = list_images(&shown)?;
        if out.len().saturating_add(files.len()) > MAX_SUBFOLDER_FILES {
            return Err(EngineError::BadParams { cmd: cmd.into(), msg: format!("{dir} holds more than {MAX_SUBFOLDER_FILES} images with its subfolders") });
        }
        out.extend(files.into_iter().map(|path| BatchInput { path, rel_dir: rel.clone() }));
        if depth >= MAX_SUBFOLDER_DEPTH {
            continue;
        }
        let rd = std::fs::read_dir(&folder).map_err(|e| EngineError::Other(format!("{shown}: {e}")))?;
        let mut subs: Vec<(String, PathBuf)> = Vec::new();
        for entry in rd {
            let path = entry.map_err(|e| EngineError::Other(format!("{shown}: {e}")))?.path();
            if path.is_dir()
                && let Some(name) = path.file_name()
            {
                subs.push((name.to_string_lossy().into_owned(), path.clone()));
            }
        }
        subs.sort();
        // Pushed in reverse so the stack pops them in name order.
        for (name, sub) in subs.into_iter().rev() {
            let c = canon(&sub).ok_or_else(|| EngineError::Other(format!("{}: cannot resolve the subfolder", sub.display())))?;
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
pub fn list_images_tree(dir: &str, _skip: &str, _cmd: &str) -> Result<Vec<BatchInput>> {
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
/// The format Batch's "Save in the same format" writes for `path`: its own, or PSD for a file
/// PhotoCraft only opens.
pub fn same_format(path: &str) -> String {
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
        let r = process_one(raw_ppi, path, &out, save, ProcessOptions { format: None, written: Some(&written), ctx: None }, f);
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

struct ProcessOptions<'a> {
    format: Option<&'a str>,
    written: Option<&'a OutputClaims>,
    ctx: Option<&'a crate::jobs::JobCtx>,
}

fn process_one(raw_ppi: f64, path: &str, out: &str, save: SaveOpts, options: ProcessOptions<'_>, f: &dyn Fn(&mut Session, &str) -> Result<bool>) -> Result<()> {
    if let Some(written) = options.written {
        written.check(out).map_err(EngineError::Other)?;
    }
    let bytes = read_file(path)?;
    let mut scratch = Session::new();
    let doc = import_at_raw_ppi(raw_ppi, &file_name(path), &bytes)?;
    scratch.add_document(doc, Some(path.to_string()));
    if let Some(ctx) = options.ctx {
        ctx.publish_batch_preview(path, scratch.active().map(|state| state.doc.clone()));
    }
    let saved_by_action = f(&mut scratch, out)?;
    if saved_by_action {
        // The action's saved snapshot is already written: a cancel or Esc arriving now stops the
        // inputs after this one, not this result.
        return Ok(());
    }
    if let Some(ctx) = options.ctx {
        ctx.check()?;
        if ctx.batch_interrupt_requested() {
            return Err(EngineError::Cancelled);
        }
    }
    let d = scratch.active().ok_or(EngineError::NoDocument)?;
    if let Some(format) = options.format {
        let ext = if format == "same" { same_format(path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
        if let Some(ctx) = options.ctx {
            save_batch_doc(&d.doc, out, &ext, save, ctx)?;
        } else {
            save_doc_as(&d.doc, out, &ext, save)?;
        }
    } else {
        save_doc(&d.doc, out, save)?;
    }
    Ok(())
}

/// Encode before the Batch write checkpoint. Esc accepted during an action or encode therefore
/// leaves the current destination untouched; the worker owns the write once the checkpoint passes.
fn save_batch_doc(doc: &Document, out: &str, format: &str, save: SaveOpts, ctx: &crate::jobs::JobCtx) -> Result<()> {
    let (bytes, _) = encode(doc, format, save)?;
    ctx.batch_before_write()?;
    write_file(out, &bytes)?;
    Ok(())
}

#[derive(Clone, Debug)]
pub(super) struct BatchPolicy {
    pub(super) stop_on_error: bool,
    pub(super) error_log: Option<String>,
}

impl BatchPolicy {
    fn parse(p: &Value, cmd: &str) -> Result<Self> {
        let stop_on_error = match p.get("stopOnError") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(value)) => *value,
            Some(_) => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "\"stopOnError\" must be true or false".into() }),
        };
        let error_log = match p.get("errorLog") {
            None | Some(Value::Null) => None,
            Some(Value::String(path)) if path.trim().is_empty() => None,
            Some(Value::String(path)) => Some(path.clone()),
            Some(_) => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "\"errorLog\" must be a file path string".into() }),
        };
        Ok(Self { stop_on_error, error_log })
    }
}

/// Photoshop-compatible Batch log encoding, shared by the desktop command and CLI.
pub fn encode_batch_error_log<'a>(errors: impl IntoIterator<Item = (&'a str, &'a str)>) -> Vec<u8> {
    let mut body = String::from("Batch Start\r\n\r\n");
    for (input, reason) in errors {
        body.push_str(&format!("File: \"{input}\"\r\nError: {reason}\r\n\r\n"));
    }
    body.push_str("Batch End\r\n");
    let mut bytes = vec![0xff, 0xfe];
    for word in body.encode_utf16() {
        bytes.extend_from_slice(&word.to_le_bytes());
    }
    bytes
}

fn write_error_log(path: &str, errors: &[Value]) -> Result<()> {
    let bytes = encode_batch_error_log(errors.iter().map(|error| {
        (error.get("file").or_else(|| error.get("document")).and_then(Value::as_str).unwrap_or("?"), error.get("error").and_then(Value::as_str).unwrap_or("?"))
    }));
    super::paths::write_file(path, &bytes).map_err(|error| EngineError::Other(format!("error log: {error}")))
}

fn error_log_failure(path: Option<&str>, errors: &[Value]) -> Option<String> {
    path.and_then(|path| write_error_log(path, errors).err().map(|error| error.to_string()))
}

fn batch_outcome(files: Vec<String>, errors: Vec<Value>, stopped: Option<(String, Vec<String>)>, error_log: Option<&str>) -> Value {
    let mut result = json!({"files": files, "errors": errors});
    if let Some((at, skipped)) = stopped {
        result["stopped"] = json!(at);
        result["skipped"] = json!(skipped);
    }
    if let Some(path) = error_log {
        result["errorLog"] = json!(path);
    }
    result
}

fn validate_error_log(policy: &BatchPolicy, inputs: impl IntoIterator<Item = String>, outputs: impl IntoIterator<Item = String>) -> Result<()> {
    let Some(log) = &policy.error_log else { return Ok(()) };
    let conflicts = |path: String| super::paths::same_destination(log, &path);
    if inputs.into_iter().chain(outputs).any(conflicts) {
        return Err(EngineError::BadParams { cmd: "file.automate.batch".into(), msg: "\"errorLog\" must not replace an input or batch output".into() });
    }
    Ok(())
}

/// A flat Batch result must not replace another input before it is opened. Compare resolved
/// identities once for the whole plan, so aliases and large folder batches stay safe and linear.
#[cfg(not(target_arch = "wasm32"))]
pub fn validate_flat_outputs(outputs: &[(String, String)]) -> Result<()> {
    let sources: std::collections::HashMap<_, _> =
        outputs.iter().enumerate().filter_map(|(index, (input, _))| std::fs::canonicalize(input).ok().map(|identity| (identity, index))).collect();
    for (index, (_, output)) in outputs.iter().enumerate() {
        let identity = super::paths::destination_identity(std::path::Path::new(output))
            .ok_or_else(|| EngineError::BadParams { cmd: "file.automate.batch".into(), msg: format!("output path cannot be resolved safely: {output}") })?;
        if sources.get(&identity).is_some_and(|&source_index| source_index != index) {
            return Err(EngineError::BadParams {
                cmd: "file.automate.batch".into(),
                msg: format!("output would replace another input file: {output}; choose a different destination or naming template"),
            });
        }
    }
    Ok(())
}

pub(super) fn process_batch_outputs(
    raw_ppi: f64,
    outputs: Vec<(String, String)>,
    format: &str,
    save: SaveOpts,
    policy: &BatchPolicy,
    ctx: &crate::jobs::JobCtx,
    f: &dyn Fn(&mut Session, &str) -> Result<bool>,
) -> Result<Value> {
    let total = outputs.len();
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut stopped = None;
    let mut done = 0;
    let mut interrupted = Vec::new();
    for (index, (path, out)) in outputs.iter().enumerate() {
        if ctx.cancelled() {
            break;
        }
        if ctx.batch_take_interrupt() && !ctx.batch_wait_for_decision() {
            ctx.publish_batch_preview(path, None);
            ctx.cancel();
            break;
        }
        ctx.progress(index as f32 / total.max(1) as f32, &format!("{} of {total}: {}", index + 1, file_name(path)));
        let result = process_one(raw_ppi, path, out, save, ProcessOptions { format: Some(format), written: None, ctx: Some(ctx) }, f);
        // An input that failed on its own (not because it was stopped) stays a failure with its
        // reason, even when a cancel or Esc lands while it runs.
        let failure =
            result.as_ref().err().filter(|error| !matches!(error, EngineError::Cancelled)).map(|error| json!({"file": path, "error": error.to_string()}));
        if ctx.cancelled() {
            ctx.publish_batch_preview(path, None);
            // A cancel that lands while the result is written still leaves that file written.
            if result.is_ok() {
                files.push(out.clone());
                done = index + 1;
            } else if let Some(failure) = failure {
                errors.push(failure);
                done = index + 1;
            } else {
                done = index;
            }
            break;
        }
        if ctx.batch_take_interrupt() {
            let continue_next = ctx.batch_wait_for_decision();
            ctx.publish_batch_preview(path, None);
            match failure {
                Some(failure) => {
                    errors.push(failure);
                    done = index + 1;
                }
                None if continue_next => {
                    interrupted.push(path.clone());
                    done = index + 1;
                }
                None => {}
            }
            if continue_next {
                continue;
            }
            ctx.cancel();
            break;
        }
        ctx.batch_after_write();
        ctx.publish_batch_preview(path, None);
        done = index + 1;
        match result {
            Ok(()) => {
                files.push(out.clone());
            }
            Err(error) => {
                errors.push(json!({"file": path, "error": error.to_string()}));
                if policy.stop_on_error {
                    stopped = Some(path.clone());
                    break;
                }
            }
        }
    }
    // No checkpoint follows the last input: a later Esc has nothing to interrupt.
    ctx.batch_close_interrupts();
    let log_failure = error_log_failure(policy.error_log.as_deref(), &errors);
    let cancelled = ctx.cancelled();
    if !cancelled {
        ctx.progress(1.0, "");
    }
    let mut skipped = interrupted;
    skipped.extend(outputs.iter().skip(done).map(|(path, _)| path.clone()));
    let mut result = batch_outcome(files, errors, stopped.map(|at| (at, skipped.clone())), policy.error_log.as_deref().filter(|_| log_failure.is_none()));
    if !skipped.is_empty() {
        result["skipped"] = json!(skipped);
    }
    if cancelled {
        result["cancelled"] = json!(true);
    }
    if let Some(error) = log_failure {
        result["errorLogError"] = json!(error);
    }
    Ok(result)
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
    let policy = BatchPolicy::parse(p, cmd)?;
    let source = match p.get("source") {
        None | Some(Value::Null) => "folder",
        Some(Value::String(source)) if matches!(source.as_str(), "folder" | "open") => source.as_str(),
        _ => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "\"source\" must be \"folder\" or \"open\"".into() }),
    };
    let output = if source == "folder" {
        str_param(p, "output", cmd)?.to_string()
    } else {
        match p.get("output") {
            None | Some(Value::Null) => String::new(),
            Some(Value::String(path)) => path.clone(),
            Some(_) => return Err(EngineError::BadParams { cmd: cmd.into(), msg: "\"output\" must be a folder path".into() }),
        }
    };
    let inputs = if source == "folder" { batch_inputs_tree(p, cmd, &output)? } else { Vec::new() };
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
    let input_count = if source == "open" { s.documents().len() } else { inputs.len() };
    let mut playback_steps = 0;
    if s.actions.playing > 0 {
        playback_steps = s.actions.playback_steps.saturating_add((steps.len().max(1) as u64).saturating_mul(input_count as u64));
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
    if source == "open" {
        let save = SaveOpts::from_params(s, p, cmd)?;
        if p.get("calledActions").is_some_and(|value| !value.is_null()) {
            let previous = std::mem::replace(&mut s.actions.list, actions.list.clone());
            let result = batch_open_documents(s, &steps, p, &output, &format, &policy, save);
            s.actions.list = previous;
            return result;
        }
        return batch_open_documents(s, &steps, p, &output, &format, &policy, save);
    }
    let names = batch_naming::plan(&inputs, &output, &format, p)?;
    let save = SaveOpts::from_params(s, p, cmd)?;
    let authorize = s.authorize;
    let raw_ppi = s.prefs().raw_defaults.resolution;
    let outputs: Vec<(String, String)> = match names {
        Some(names) => inputs.iter().map(|input| input.path.clone()).zip(names).collect(),
        None => inputs
            .iter()
            .map(|input| {
                let ext = if format == "same" { same_format(&input.path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
                let out = join(&output, &format!("{}.{ext}", stem(&input.path)));
                (input.path.clone(), out)
            })
            .collect(),
    };
    #[cfg(not(target_arch = "wasm32"))]
    validate_flat_outputs(&outputs)?;
    validate_error_log(&policy, outputs.iter().map(|(input, _)| input.clone()), outputs.iter().map(|(_, output)| output.clone()))?;
    // Each input runs in its own scratch session, so what the played steps spend is carried from
    // one input to the next (failed or not) and back to this session, not restarted per input.
    let was_playing = s.actions.playing > 0;
    crate::jobs::run(
        s,
        "Batch",
        false,
        move |ctx| {
            let carried = std::cell::Cell::new(playback_steps);
            let run = |scratch: &mut Session, out: &str| {
                scratch.authorize = authorize;
                scratch.actions = actions.clone();
                scratch.actions.playback_steps = carried.get();
                let result = (|| {
                    // A recorded save marks a result. If the action later closes or switches
                    // documents, write the saved snapshot instead of the new active document.
                    let source = scratch.active().and_then(|state| state.path.as_deref()).unwrap_or("").to_string();
                    let encode_format = if format == "same" { same_format(&source) } else { format.trim_start_matches('.').to_ascii_lowercase() };
                    let mut saved: Option<Arc<Document>> = None;
                    for (id, params) in &steps {
                        ctx.check()?;
                        if ctx.batch_interrupt_requested() {
                            return Err(EngineError::Cancelled);
                        }
                        if batch_save_point(id) {
                            let state = scratch.active_mut().ok_or(EngineError::NoDocument)?;
                            saved = Some(state.doc.clone());
                            // The destination save is performed by Batch; subsequent recorded
                            // Close steps see Save / Save As as saved, but Save a Copy as changed.
                            if id != "file.saveACopy" {
                                state.saved_revision = state.revision;
                            }
                            continue;
                        }
                        let result = scratch.execute(id, params.clone())?;
                        ctx.publish_batch_preview(&source, scratch.active().map(|state| state.doc.clone()));
                        if let Some(error) = crate::actions_cmds::nested_failure(id, &result) {
                            return Err(EngineError::Other(error));
                        }
                    }
                    match saved {
                        Some(doc) if scratch.active().is_none_or(|active| active.doc.id != doc.id) => {
                            ctx.check()?;
                            save_batch_doc(&doc, out, &encode_format, save, ctx)?;
                            Ok(true)
                        }
                        _ => Ok(false),
                    }
                })();
                carried.set(scratch.actions.playback_steps);
                result
            };
            let result = process_batch_outputs(raw_ppi, outputs, &format, save, &policy, ctx, &run);
            Ok((result, carried.get()))
        },
        move |s, (result, carried)| {
            if was_playing {
                s.actions.playback_steps = carried;
            }
            result
        },
    )
}

/// Source: Opened Files runs on the live documents, so edits remain in their histories.
/// Documents are identified before playback; a step that closes or reorders tabs cannot make
/// the next input silently become a different document.
fn batch_open_documents(
    s: &mut Session,
    steps: &[(String, Value)],
    p: &Value,
    output: &str,
    format: &str,
    policy: &BatchPolicy,
    save: SaveOpts,
) -> Result<Value> {
    let originals: Vec<(u64, String, String)> = s
        .documents()
        .iter()
        .map(|state| {
            let path = state.path.clone().unwrap_or_else(|| format!("{}.psd", state.doc.name));
            (state.doc.id.0, state.doc.name.clone(), path)
        })
        .collect();
    if originals.is_empty() {
        return Err(EngineError::NoDocument);
    }
    let previous = s.active().map(|state| state.doc.id.0);
    let virtual_inputs: Vec<BatchInput> = originals.iter().map(|(_, _, path)| BatchInput { path: path.clone(), rel_dir: String::new() }).collect();
    let planned = if output.is_empty() {
        None
    } else {
        let named = batch_naming::plan(&virtual_inputs, output, format, p)?;
        Some(match named {
            Some(paths) => paths,
            None => virtual_inputs
                .iter()
                .map(|input| {
                    let ext = if format == "same" { same_format(&input.path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
                    join(output, &format!("{}.{ext}", stem(&input.path)))
                })
                .collect(),
        })
    };
    // As for a folder: no result may replace another open document's file.
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(paths) = &planned {
        let plan: Vec<(String, String)> = originals.iter().map(|(_, _, path)| path.clone()).zip(paths.iter().cloned()).collect();
        validate_flat_outputs(&plan)?;
    }
    validate_error_log(policy, originals.iter().map(|(_, _, path)| path.clone()), planned.iter().flat_map(|paths| paths.iter().cloned()))?;
    let mut files = Vec::new();
    let mut errors = Vec::new();
    let mut stopped = None;
    let mut done = 0;
    for (index, (id, name, path)) in originals.iter().enumerate() {
        // The document a recorded Save / Save As marked as saved, with its earlier saved
        // revision: when Batch then writes nothing, it reads as changed again.
        let mut marked: Option<(u64, u64)> = None;
        let result = (|| -> Result<()> {
            let position = s.documents().iter().position(|state| state.doc.id.0 == *id).ok_or(EngineError::NoDocument)?;
            s.set_active(position);
            let mut saved: Option<Arc<Document>> = None;
            for (command, params) in steps {
                if batch_save_point(command) {
                    let state = s.active_mut().ok_or(EngineError::NoDocument)?;
                    saved = Some(state.doc.clone());
                    // With no destination nothing is written, so the document stays changed.
                    if command != "file.saveACopy" && planned.is_some() {
                        if marked.is_none() {
                            marked = Some((state.doc.id.0, state.saved_revision));
                        }
                        state.saved_revision = state.revision;
                    }
                    continue;
                }
                let response = s.execute(command, params.clone())?;
                if let Some(error) = crate::actions_cmds::nested_failure(command, &response) {
                    return Err(EngineError::Other(error));
                }
            }
            let Some(paths) = &planned else { return Ok(()) };
            let out = paths.get(index).ok_or_else(|| EngineError::Other("batch output plan is incomplete".into()))?;
            let doc = match saved {
                Some(saved) if s.active().is_some_and(|state| state.doc.id == saved.id) => s.active().ok_or(EngineError::NoDocument)?.doc.clone(),
                Some(saved) => saved,
                None => s.documents().iter().find(|state| state.doc.id.0 == *id).ok_or(EngineError::NoDocument)?.doc.clone(),
            };
            let ext = if format == "same" { same_format(path) } else { format.trim_start_matches('.').to_ascii_lowercase() };
            save_doc_as(&doc, out, &ext, save)?;
            files.push(out.clone());
            Ok(())
        })();
        done = index + 1;
        if result.is_err()
            && let Some((doc, revision)) = marked
            && let Some(position) = s.documents().iter().position(|state| state.doc.id.0 == doc)
        {
            s.set_active(position);
            if let Some(state) = s.active_mut() {
                state.saved_revision = revision;
            }
        }
        if let Err(error) = result {
            errors.push(json!({"document": name, "error": error.to_string()}));
            if policy.stop_on_error {
                stopped = Some(name.clone());
                break;
            }
        }
    }
    if let Some(id) = previous
        && let Some(position) = s.documents().iter().position(|state| state.doc.id.0 == id)
    {
        s.set_active(position);
    }
    let log_failure = error_log_failure(policy.error_log.as_deref(), &errors);
    let skipped = originals.iter().skip(done).map(|(_, name, _)| name.clone()).collect();
    let mut result = batch_outcome(files, errors, stopped.map(|at| (at, skipped)), policy.error_log.as_deref().filter(|_| log_failure.is_none()));
    if let Some(error) = log_failure {
        result["errorLogError"] = json!(error);
    }
    result["documents"] = json!(originals.len());
    Ok(result)
}
